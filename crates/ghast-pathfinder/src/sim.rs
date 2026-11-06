//! Running a path offline: the [`Controller`] drives the vanilla-exact `ghast-physics` player
//! through the path, turning in whole mouse counts like ghast-client does.

use ghast_physics::Keys;

use crate::Version;
use crate::class::{flag, table};
use crate::control::{Controller, ControllerConfig, Progress};
use crate::moves::MoveKind;
use crate::physics::{Observation, P16, P18, Physics, SourceView};
use crate::plan::Path;
use crate::world::BlockSource;

/// How a simulated run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SimOutcome {
    /// The last step was reached.
    Arrived,
    /// The controller made no progress ([`Progress::Stuck`]).
    Stuck,
    /// The player left the path ([`Progress::OffPath`]).
    OffPath,
    /// The body touched lava or a block the planner avoids (fire, cobweb, ...).
    Hazard,
    /// Out of ticks.
    Timeout,
}

/// The result of [`simulate`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SimReport {
    /// How it ended.
    pub outcome: SimOutcome,
    /// Ticks run.
    pub ticks: u32,
    /// Index of the last step reached.
    pub reached: usize,
    /// Feet position at the end.
    pub end: [f64; 3],
    /// Ticks with a horizontal collision during moves that should not touch anything
    /// (walks, diagonals, drops, gap jumps). Jumps up, climbs and swims onto land are
    /// expected to touch the block they go onto and are not counted.
    pub collisions: u32,
    /// The first tick of such a collision.
    pub first_collision: Option<u32>,
    /// Largest fall distance at a landing (vanilla fall damage starts above 3).
    pub max_fall: f32,
    /// Feet position after every tick.
    pub trace: Vec<[f64; 3]>,
}

/// The client's rotation (ghast-client `rotation::Rotation` at sensitivity 0.5): whole mouse
/// counts, at most 14° yaw / 10.5° pitch per tick with a carried budget.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Turner {
    yaw_budget: f64,
    pitch_budget: f64,
}

impl Turner {
    const MAX_YAW: f64 = 14.0;
    const MAX_PITCH: f64 = 10.5;
    /// `mouse_multiplier(0.5)`: `(0.5 * 0.6 + 0.2)^3 * 8` in `f32`.
    const MULTIPLIER: f32 = 1.0;

    fn counts(budget: &mut f64, delta: f32, per_tick_deg: f64) -> i64 {
        let step = f64::from(Self::MULTIPLIER) * 0.15;
        let per_tick = per_tick_deg / step;
        *budget = (*budget + per_tick).min(per_tick + 1.0);
        let wanted = (f64::from(delta) / step + 0.5).floor() as i64;
        let lim = budget.trunc() as i64;
        let count = wanted.clamp(-lim, lim);
        *budget -= count.unsigned_abs() as f64;
        if count == wanted {
            *budget = 0.0;
        }
        count
    }

    /// One tick from (`yaw`, `pitch`) toward the target (pitch target clamped to ±89.5).
    pub(crate) fn step(&mut self, yaw: f32, pitch: f32, target: (f32, f32)) -> (f32, f32) {
        let m = Self::MULTIPLIER;
        let ty = target.0;
        let tp = target.1.clamp(-89.5, 89.5);
        let dx = Self::counts(
            &mut self.yaw_budget,
            crate::control::wrap(ty - yaw),
            Self::MAX_YAW,
        );
        let dy = Self::counts(&mut self.pitch_budget, tp - pitch, Self::MAX_PITCH);
        let fx = dx as f32 * m;
        let fy = -(dy as f32) * m;
        let new_yaw = (f64::from(yaw) + f64::from(fx) * 0.15) as f32;
        let new_pitch = ((f64::from(pitch) - f64::from(fy) * 0.15) as f32).clamp(-90.0, 90.0);
        (new_yaw, new_pitch)
    }
}

/// Runs `path` in `world` with a [`Controller`]: the player starts standing at the first
/// step's centre, facing the second, and the physics is `ghast-physics` (bit-exact vanilla)
/// for the path's version. Digs complete after `5 + 30 × hardness` ticks (a bare hand's
/// order of magnitude) and places after 4 ticks (a stone block appears); the world itself is
/// not changed.
///
/// ```
/// use ghast_pathfinder::{Config, ControllerConfig, Goal, Pathfinder, SimOutcome, Version, simulate};
///
/// let mut world = ghast_physics::v1_8::memory_world();
/// world.load_chunks(-2, -2, 2, 2);
/// world.fill_raw([-20, 63, -20], [20, 63, 20], 1 << 4);
/// world.fill_raw([4, 64, -3], [4, 64, 3], 1 << 4); // a step
/// let path = Pathfinder::new(Config::default())
///     .plan(Version::V1_8, &world, [0, 64, 0], &Goal::Block([9, 65, 0]))
///     .unwrap();
/// let report = simulate(&path, &world, &ControllerConfig::default());
/// assert_eq!(report.outcome, SimOutcome::Arrived);
/// ```
#[must_use]
pub fn simulate<S: BlockSource + ?Sized>(
    path: &Path,
    world: &S,
    cfg: &ControllerConfig,
) -> SimReport {
    match path.version {
        Version::V1_8 => run::<P18, S>(path, world, cfg),
        Version::V1_16_5 => run::<P16, S>(path, world, cfg),
    }
}

/// The raw stone state of a version (what a simulated place puts down).
fn stone(version: Version) -> u32 {
    match version {
        Version::V1_8 => 1 << 4,
        Version::V1_16_5 => u32::from(ghast_data::v1_16_5::Block::STONE.default_state().id()),
    }
}

fn run<P: Physics, S: BlockSource + ?Sized>(
    path: &Path,
    world: &S,
    cfg: &ControllerConfig,
) -> SimReport {
    let tbl = table(path.version);
    let mut view = SourceView::<P, S>::new(world);
    let start = path.steps[0].target();
    let yaw0 = path.steps.get(1).map_or(0.0, |s| {
        let t = s.target();
        (-(t[0] - start[0])).atan2(t[2] - start[2]).to_degrees() as f32
    });
    let mut state = P::new_state(start, yaw0, 0.0);
    let mut turner = Turner::default();
    let mut ctrl = Controller::new(path.clone(), cfg.clone());
    let max_ticks = (path.cost * 4.0) as u32 + 20 * path.steps.len() as u32 + 200;
    let mut report = SimReport {
        outcome: SimOutcome::Timeout,
        ticks: 0,
        reached: 0,
        end: start,
        collisions: 0,
        first_collision: None,
        max_fall: 0.0,
        trace: Vec::with_capacity(max_ticks.min(4096) as usize),
    };
    let mut job: Option<([i32; 3], u32)> = None;
    let class = |v: &SourceView<'_, P, S>, x: i32, y: i32, z: i32| {
        v.raw(x, y, z).map_or(crate::class::Class::UNKNOWN, |r| {
            tbl.classes[tbl.index(r) as usize]
        })
    };
    for tick in 0..max_ticks {
        let mut obs: Observation = P::observe(&state);
        obs.food = 20;
        let want = ctrl.aim(&obs, &view);
        let look = turner.step(obs.yaw, obs.pitch, want);
        let cmd = ctrl.keys(&obs, look, &view);
        report.reached = ctrl.reached();
        match ctrl.progress() {
            Progress::Arrived => {
                report.outcome = SimOutcome::Arrived;
                break;
            }
            Progress::Stuck => {
                report.outcome = SimOutcome::Stuck;
                break;
            }
            Progress::OffPath => {
                report.outcome = SimOutcome::OffPath;
                break;
            }
            Progress::Moving | Progress::Working => {}
        }
        // Jobs: a dig or place takes a while, then the block changes.
        let target = cmd.dig.or(cmd.place.map(|p| p.pos));
        match (target, job) {
            (Some(p), Some((q, n))) if p == q => {
                let need = match cmd.dig {
                    Some(d) => {
                        5 + (f64::from(class(&view, d[0], d[1], d[2]).hardness.max(0.0)) * 30.0)
                            as u32
                    }
                    None => 4,
                };
                if n + 1 >= need {
                    view.set(
                        p,
                        if cmd.dig.is_some() {
                            0
                        } else {
                            stone(path.version)
                        },
                    );
                    job = None;
                } else {
                    job = Some((q, n + 1));
                }
            }
            (Some(p), _) => job = Some((p, 0)),
            (None, _) => job = None,
        }
        let keys = if target.is_some() {
            Keys::NONE
        } else {
            cmd.keys
        };
        let kind = path
            .steps
            .get(ctrl.reached() + 1)
            .map_or(MoveKind::Start, |s| s.kind);
        let fall_before = P::fall_distance(&state);
        P::tick(&mut state, keys, look.0, look.1, &view);
        let o = P::observe(&state);
        report.ticks = tick + 1;
        report.end = o.pos;
        report.trace.push(o.pos);
        if o.on_ground {
            report.max_fall = report.max_fall.max(fall_before);
        }
        if P::horizontal_collision(&state)
            && matches!(
                kind,
                MoveKind::Walk | MoveKind::Diagonal | MoveKind::Descend | MoveKind::Parkour { .. }
            )
        {
            report.collisions += 1;
            report.first_collision.get_or_insert(tick);
        }
        // Hazards in the body's column.
        let (x, z) = (o.pos[0].floor() as i32, o.pos[2].floor() as i32);
        let y0 = o.pos[1].floor() as i32;
        let y1 = (o.pos[1] + 1.79).floor() as i32;
        if (y0..=y1).any(|y| class(&view, x, y, z).has(flag::LAVA | flag::AVOID)) {
            report.outcome = SimOutcome::Hazard;
            break;
        }
    }
    report
}

#[cfg(all(test, feature = "client"))]
mod tests {
    use super::Turner;

    /// The simulator turns exactly like ghast-client.
    #[test]
    fn turner_matches_the_client_rotation() {
        let mut r = ghast_client::rotation::Rotation::new(0.5);
        let mut t = Turner::default();
        let (mut yaw, mut pitch) = (12.5_f32, 3.0_f32);
        let targets = [
            (100.0, 40.0),
            (-170.0, -80.0),
            (365.0, 0.0),
            (12.3, 1.1),
            (-45.0, 89.9),
        ];
        let (mut y2, mut p2) = (yaw, pitch);
        for (i, &(ty, tp)) in targets.iter().cycle().take(200).enumerate() {
            if i % 7 == 0 {
                r.set_target(ty, tp);
            }
            let target = r.target();
            let a = r.step(yaw, pitch);
            let b = t.step(y2, p2, target);
            assert_eq!(a, b, "tick {i}");
            (yaw, pitch) = a;
            (y2, p2) = b;
        }
    }
}
