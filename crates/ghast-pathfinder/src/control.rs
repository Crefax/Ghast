//! Path following: per-tick aim and keys for a planned [`Path`].
//!
//! The controller is pure (no I/O, no clock): it reads an [`Observation`] of the player and
//! the world, and answers with an aim target and keys. Where a decision depends on the
//! physics (when to jump off an edge, when to let go of the keys to land on a block, whether
//! a step would leave the floor) it runs `ghast-physics` a few ticks ahead from the
//! observation, so a simulated run ([`crate::simulate`]) and a live run make the same
//! decisions.
//!
//! Each tick is two calls, because the client turns toward the aim in whole mouse counts and
//! the keys must be chosen for the look the movement step will really use:
//!
//! 1. [`Controller::aim`]: advances along the path and returns the look to aim at;
//! 2. the caller sets that aim and asks for the look this tick's movement will use
//!    (`TickContext::planned_look` in ghast-client);
//! 3. [`Controller::keys`]: the keys (and a dig / place job) for that look.

use ghast_physics::Keys;

use crate::Version;
use crate::class::{Class, Table, flag, table};
use crate::jump::{self, Jumps, Mode};
use crate::moves::{MoveKind, NodeKind};
#[cfg(feature = "v1_12_2")]
use crate::physics::P12;
#[cfg(feature = "v1_16_5")]
use crate::physics::P16;
#[cfg(feature = "v1_8")]
use crate::physics::P18;
use crate::physics::{Observation, Physics, SourceView};
use crate::plan::{Path, Pathfinder, Place, Step};
use crate::world::BlockSource;

/// Eye height of a standing player.
const EYES: f64 = 1.62;

/// Options of the [`Controller`]. Build with [`ControllerConfig::default`] and the `with_*`
/// methods.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ControllerConfig {
    /// Sprint on straight runs (needs food > 6). Default on.
    pub sprint: bool,
    /// The last step counts as reached within this horizontal distance of its centre.
    /// Default 0.35.
    pub arrive_radius: f64,
    /// Ticks without getting closer to the next step before [`Progress::Stuck`]. Default 60.
    pub stuck_ticks: u32,
    /// Ticks away from the path before [`Progress::OffPath`]. Default 10.
    pub off_path_ticks: u32,
    /// Ticks a dig or place job may take before [`Progress::Stuck`]. Default 400.
    pub work_ticks: u32,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        ControllerConfig {
            sprint: true,
            arrive_radius: 0.35,
            stuck_ticks: 60,
            off_path_ticks: 10,
            work_ticks: 400,
        }
    }
}

impl ControllerConfig {
    /// Sprinting on or off.
    #[must_use]
    pub fn with_sprint(mut self, on: bool) -> Self {
        self.sprint = on;
        self
    }

    /// Arrival radius around the last step's centre.
    #[must_use]
    pub fn with_arrive_radius(mut self, blocks: f64) -> Self {
        self.arrive_radius = blocks.max(0.05);
        self
    }

    /// Ticks without progress before giving up.
    #[must_use]
    pub fn with_stuck_ticks(mut self, ticks: u32) -> Self {
        self.stuck_ticks = ticks;
        self
    }

    /// Ticks off the path before giving up.
    #[must_use]
    pub fn with_off_path_ticks(mut self, ticks: u32) -> Self {
        self.off_path_ticks = ticks;
        self
    }

    /// Time limit of one dig / place job.
    #[must_use]
    pub fn with_work_ticks(mut self, ticks: u32) -> Self {
        self.work_ticks = ticks;
        self
    }
}

/// What the controller wants this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Command {
    /// Movement keys.
    pub keys: Keys,
    /// Aim target yaw (degrees), as returned by [`Controller::aim`].
    pub yaw: f32,
    /// Aim target pitch (degrees).
    pub pitch: f32,
    /// Break this block (hold the left button while the crosshair is on it). The player
    /// stands still meanwhile.
    pub dig: Option<[i32; 3]>,
    /// Place the held block: right-click the top face of [`Place::against`].
    pub place: Option<Place>,
}

impl Command {
    fn idle(yaw: f32, pitch: f32) -> Self {
        Command {
            keys: Keys::NONE,
            yaw,
            pitch,
            dig: None,
            place: None,
        }
    }
}

/// Where the controller is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Progress {
    /// Moving toward the next step.
    Moving,
    /// Digging or placing before the next move.
    Working,
    /// The last step is reached (keys released).
    Arrived,
    /// No progress toward the next step for [`ControllerConfig::stuck_ticks`] (or a job ran
    /// out of time).
    Stuck,
    /// The player left the path (fell, was pushed) for [`ControllerConfig::off_path_ticks`].
    OffPath,
}

/// A dig / place job before a move.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Job {
    Dig([i32; 3]),
    Place(Place),
}

/// A gap jump in progress (see [`crate::jump`]).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Leap {
    /// Lining up and coming to rest on the take-off block; ticks since the last key.
    Settle(Mode, u32),
    /// Running with the yaw fixed, replaying the checked run: ground ticks before the jump
    /// tick and air ticks with the keys held after it, ground ticks done so far.
    Run(Mode, f32, (u32, u32), u32),
    /// The jump was pressed: in the air; (air ticks with the keys held, air ticks so far),
    /// or `None` (the run went differently than checked) to steer the landing instead.
    Air(Mode, f32, Option<(u32, u32)>),
}

/// Result of running the physics ahead with fixed keys.
#[derive(Clone, Copy, Debug)]
struct Roll {
    pos: [f64; 3],
    on_ground: bool,
    /// A horizontal collision happened.
    collided: bool,
}

/// Follows a [`Path`] tick by tick.
///
/// Pure (no I/O, no clock): it reads an [`Observation`] of the player and the world, and
/// answers with an aim target and keys. Where a decision depends on the physics (when to
/// jump off an edge, when to let go of the keys to land on a block, whether a step would
/// leave the floor) it runs `ghast-physics` a few ticks ahead from the observation, so a
/// simulated run ([`crate::simulate`]) and a live run make the same decisions.
///
/// Each tick is two calls, because the client turns toward the aim in whole mouse counts
/// and the keys must be chosen for the look the movement step will really use:
///
/// 1. [`Controller::aim`]: advances along the path and returns the look to aim at;
/// 2. the caller sets that aim and asks for the look this tick's movement will use
///    (`TickContext::planned_look` in ghast-client);
/// 3. [`Controller::keys`]: the keys (and a dig / place job) for that look.
///
/// It never gives up silently: [`Controller::progress`] turns [`Progress::Stuck`] or
/// [`Progress::OffPath`] and [`Controller::current_move`] names the move, to plan around.
///
/// ```
/// use ghast_pathfinder::{Config, Controller, ControllerConfig, Goal, Pathfinder, Progress, Version};
///
/// let mut world = ghast_physics::v1_8::memory_world();
/// world.load_chunks(-1, -1, 1, 1);
/// world.fill_raw([-10, 63, -10], [10, 63, 10], 1 << 4);
/// let path = Pathfinder::new(Config::default())
///     .plan(Version::V1_8, &world, [0, 64, 0], &Goal::Block([5, 64, 0]))
///     .unwrap();
/// let mut c = Controller::new(path, ControllerConfig::default());
/// // Each tick, with `obs` the player's observation:
/// # let obs = ghast_pathfinder::Observation { pos: [0.5, 64.0, 0.5], vel: [0.0; 3],
/// #     on_ground: true, yaw: 0.0, pitch: 0.0, sprinting: false, food: 20 };
/// let (yaw, pitch) = c.aim(&obs, &world);
/// // ... the client turns toward (yaw, pitch); `look` is what this tick's movement uses.
/// let look = (yaw, pitch);
/// let cmd = c.keys(&obs, look, &world);
/// assert_eq!(c.progress(), Progress::Moving);
/// assert!(cmd.keys.forward || cmd.keys == ghast_physics::Keys::NONE);
/// ```
#[derive(Clone, Debug)]
pub struct Controller {
    path: Path,
    cfg: ControllerConfig,
    /// Index of the last step reached.
    reached: usize,
    progress: Progress,
    /// Best distance to the next step so far, and ticks since it improved.
    best: f64,
    idle: u32,
    off: u32,
    work: u32,
    /// This tick's aim target and the distance to the point it looks at.
    want: (f32, f32),
    aim_dist: f64,
    /// The gap jump of the current move.
    leap: Option<Leap>,
    /// The move's keys would have left the floor: back to the centre of the last step first.
    recover: bool,
    /// This tick walks toward the last step's centre to line up for the move.
    aligning: bool,
}

impl Controller {
    /// A controller at the start of `path`.
    #[must_use]
    pub fn new(path: Path, cfg: ControllerConfig) -> Self {
        let progress = if path.steps.len() <= 1 {
            Progress::Arrived
        } else {
            Progress::Moving
        };
        Controller {
            path,
            cfg,
            reached: 0,
            progress,
            best: f64::INFINITY,
            idle: 0,
            off: 0,
            work: 0,
            want: (0.0, 0.0),
            aim_dist: 0.0,
            leap: None,
            recover: false,
            aligning: false,
        }
    }

    /// The path being followed.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Index (into [`Path::steps`]) of the last step reached.
    #[must_use]
    pub fn reached(&self) -> usize {
        self.reached
    }

    /// The current state.
    #[must_use]
    pub fn progress(&self) -> Progress {
        self.progress
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &ControllerConfig {
        &self.cfg
    }

    /// The move (`from`, `to` feet blocks) the controller is on: after [`Progress::Stuck`] /
    /// [`Progress::OffPath`] the move that failed, for [`Pathfinder::plan_avoiding`].
    #[must_use]
    pub fn current_move(&self) -> Option<([i32; 3], [i32; 3])> {
        let s = &self.path.steps;
        (self.reached + 1 < s.len()).then(|| (s[self.reached].pos, s[self.reached + 1].pos))
    }

    fn class<S: BlockSource + ?Sized>(&self, w: &S, p: [i32; 3]) -> Class {
        let t: &Table = table(self.path.version);
        match w.raw_block(p[0], p[1], p[2]) {
            Some(raw) => t.classes[t.index(raw) as usize],
            None => Class::UNKNOWN,
        }
    }

    /// The pending dig / place job of the next step.
    fn job<S: BlockSource + ?Sized>(&self, w: &S) -> Option<Job> {
        let t = self.path.steps.get(self.reached + 1)?;
        for &d in &t.dig {
            if self.class(w, d).has(flag::SOLID) {
                return Some(Job::Dig(d));
            }
        }
        if let Some(p) = t.place
            && !self.class(w, p.pos).has(flag::SUPPORT)
        {
            return Some(Job::Place(p));
        }
        None
    }

    /// Advances along the path from `obs` and returns the look (yaw, pitch) to aim at this
    /// tick. Call once per tick, before [`Controller::keys`].
    pub fn aim<S: BlockSource + ?Sized>(&mut self, obs: &Observation, world: &S) -> (f32, f32) {
        self.update(obs, world);
        if matches!(
            self.progress,
            Progress::Arrived | Progress::Stuck | Progress::OffPath
        ) {
            self.want = (obs.yaw, obs.pitch);
            return self.want;
        }
        if let Some(job) = self.job(world) {
            if self.progress != Progress::Working {
                self.work = 0;
            }
            self.progress = Progress::Working;
            let p = match job {
                Job::Dig(d) => [
                    f64::from(d[0]) + 0.5,
                    f64::from(d[1]) + 0.5,
                    f64::from(d[2]) + 0.5,
                ],
                Job::Place(pl) => [
                    f64::from(pl.against[0]) + 0.5,
                    f64::from(pl.against[1]) + 1.0,
                    f64::from(pl.against[2]) + 0.5,
                ],
            };
            let eyes = [obs.pos[0], obs.pos[1] + EYES, obs.pos[2]];
            self.want = look_at(eyes, p);
            return self.want;
        }
        if self.progress == Progress::Working {
            // The job is done: measure progress afresh.
            self.best = f64::INFINITY;
            self.idle = 0;
        }
        self.progress = Progress::Moving;
        let yaw = self.move_yaw(obs, world);
        self.want = (yaw, obs.pitch.clamp(-25.0, 25.0));
        self.want
    }

    /// The keys (and dig / place job) for this tick, given the look the movement step will
    /// use (`look`, after the client's turn toward [`Controller::aim`]'s target).
    pub fn keys<S: BlockSource + ?Sized>(
        &mut self,
        obs: &Observation,
        look: (f32, f32),
        world: &S,
    ) -> Command {
        let (yaw, pitch) = self.want;
        match self.progress {
            Progress::Arrived | Progress::Stuck | Progress::OffPath => {
                return Command::idle(yaw, pitch);
            }
            Progress::Working => {
                self.work += 1;
                if self.work > self.cfg.work_ticks {
                    self.progress = Progress::Stuck;
                    return Command::idle(yaw, pitch);
                }
                let mut c = Command::idle(yaw, pitch);
                match self.job(world) {
                    Some(Job::Dig(d)) => c.dig = Some(d),
                    Some(Job::Place(p)) => c.place = Some(p),
                    None => {}
                }
                return c;
            }
            Progress::Moving => {}
        }
        let keys = match self.path.version {
            #[cfg(feature = "v1_8")]
            Version::V1_8 => self.move_keys::<P18, S>(obs, look, world),
            #[cfg(feature = "v1_12_2")]
            Version::V1_12 => self.move_keys::<P12, S>(obs, look, world),
            #[cfg(feature = "v1_16_5")]
            Version::V1_16_5 => self.move_keys::<P16, S>(obs, look, world),
        };
        Command {
            keys,
            yaw,
            pitch,
            dig: None,
            place: None,
        }
    }

    /// `true` when the player stands at step `k` the way the planner expects.
    fn at(&self, k: usize, obs: &Observation) -> bool {
        let steps = &self.path.steps;
        let s = &steps[k];
        let fb = Pathfinder::start_block(obs.pos);
        let in_col = fb[0] == s.pos[0] && fb[2] == s.pos[2];
        match s.node {
            NodeKind::Ground => {
                if !obs.on_ground || (obs.pos[1] - s.feet_y).abs() >= 0.02 {
                    return false;
                }
                if in_col {
                    return true;
                }
                // Standing on this step's floor with the centre still over the previous step
                // (at another height) or already over the next one: the box is 0.6 wide.
                let col = |n: &Step| n.pos[0] == fb[0] && n.pos[2] == fb[2];
                over(obs.pos, s.pos, 0.3)
                    && (steps.get(k + 1).is_some_and(col)
                        || (k > 0
                            && col(&steps[k - 1])
                            && (steps[k - 1].feet_y - s.feet_y).abs() > 0.02))
            }
            // In the water column: from two blocks under the cell (sinking after a fall) up
            // to floating with the feet just under its top.
            NodeKind::Swim => {
                in_col
                    && obs.pos[1] < f64::from(s.pos[1]) + 0.9
                    && obs.pos[1] > f64::from(s.pos[1]) - 2.5
            }
            NodeKind::Climb => in_col && (fb[1] == s.pos[1] || (obs.pos[1] - s.feet_y).abs() < 0.5),
        }
    }

    fn update<S: BlockSource + ?Sized>(&mut self, obs: &Observation, world: &S) {
        if matches!(
            self.progress,
            Progress::Arrived | Progress::Stuck | Progress::OffPath
        ) {
            return;
        }
        let last = self.path.steps.len() - 1;
        let hi = (self.reached + 3).min(last);
        for k in (self.reached + 1..=hi).rev() {
            if self.at(k, obs) && (k < last || self.arrived_at_end(obs)) {
                // Never skip a step whose dig / place is still pending.
                let skipped_job = (self.reached + 1..=k).any(|j| {
                    let s = &self.path.steps[j];
                    s.dig.iter().any(|&d| self.class(world, d).has(flag::SOLID))
                        || s.place
                            .is_some_and(|p| !self.class(world, p.pos).has(flag::SUPPORT))
                });
                if skipped_job {
                    continue;
                }
                self.reached = k;
                self.best = f64::INFINITY;
                self.idle = 0;
                self.off = 0;
                self.leap = None;
                self.recover = false;
                break;
            }
        }
        if self.reached == last {
            self.progress = Progress::Arrived;
            return;
        }
        if self.progress == Progress::Working {
            return;
        }
        let t = self.path.steps[self.reached + 1].target();
        let d = hdist(obs.pos, t) + (obs.pos[1] - t[1]).abs();
        if d < self.best - 0.05 {
            self.best = d;
            self.idle = 0;
        } else {
            self.idle += 1;
            if self.idle > self.cfg.stuck_ticks {
                self.progress = Progress::Stuck;
                return;
            }
        }
        if self.off_path(obs) {
            self.off += 1;
            if self.off > self.cfg.off_path_ticks {
                self.progress = Progress::OffPath;
            }
        } else {
            self.off = 0;
        }
    }

    fn arrived_at_end(&self, obs: &Observation) -> bool {
        let s = self.path.steps.last().expect("non-empty path");
        hdist(obs.pos, s.target()) <= self.cfg.arrive_radius
    }

    fn off_path(&self, obs: &Observation) -> bool {
        let a = self.path.steps[self.reached].target();
        let b = self.path.steps[self.reached + 1].target();
        let swim = self.path.steps[self.reached].node == NodeKind::Swim
            || self.path.steps[self.reached + 1].node == NodeKind::Swim;
        if obs.pos[1] < a[1].min(b[1]) - if swim { 3.0 } else { 1.0 } {
            return true;
        }
        segment_dist(obs.pos, a, b) > 1.0
    }

    /// The yaw to move with toward the next step.
    fn move_yaw<S: BlockSource + ?Sized>(&mut self, obs: &Observation, world: &S) -> f32 {
        let steps = &self.path.steps;
        let last = steps.len() - 1;
        let f = &steps[self.reached];
        let t = &steps[self.reached + 1];
        let (dx, dz) = (t.pos[0] - f.pos[0], t.pos[2] - f.pos[2]);
        self.aim_dist = hdist(obs.pos, t.target());
        self.aligning = false;
        if self.recover {
            let fc = f.target();
            let d = hdist(obs.pos, fc);
            if d < 0.2 || !obs.on_ground {
                self.recover = false;
            } else {
                self.aim_dist = d;
                return yaw_to(obs.pos, fc);
            }
        }
        match t.kind {
            MoveKind::ClimbUp => {
                if let Some((wx, wz)) = self.climb_wall(world, f.pos) {
                    return dir_yaw(wx, wz);
                }
            }
            MoveKind::Descend if dx == 0 || dz == 0 => {
                // Walk off straight: a box sticking out sideways can catch a neighbouring
                // block (or step up onto it).
                let fc = f.target();
                if obs.on_ground
                    && over(obs.pos, f.pos, 0.3)
                    && lateral(obs.pos, fc, dx.signum(), dz.signum()) > 0.15
                    && hdist(obs.pos, fc) > 0.05
                {
                    self.aligning = true;
                    self.aim_dist = hdist(obs.pos, fc);
                    return yaw_to(obs.pos, fc);
                }
            }
            MoveKind::ClimbDown => {
                // A fixed yaw: the keys keep the player over the ladder.
                if dx != 0 || dz != 0 {
                    return dir_yaw(dx.signum(), dz.signum());
                }
                return obs.yaw;
            }
            MoveKind::Parkour { .. } => {
                // Facing along the axis; once running, the yaw stays where the run began.
                return match self.leap {
                    Some(Leap::Run(_, yaw, ..) | Leap::Air(_, yaw, _)) => yaw,
                    _ => dir_yaw(dx.signum(), dz.signum()),
                };
            }
            _ => {}
        }
        // Look ahead along a straight run of plain walks on one level.
        let mut k = self.reached + 1;
        if matches!(t.kind, MoveKind::Walk | MoveKind::Diagonal) && plain(t) {
            for j in self.reached + 2..=(self.reached + 4).min(last) {
                let s = &steps[j];
                let p = &steps[j - 1];
                if matches!(s.kind, MoveKind::Walk | MoveKind::Diagonal)
                    && plain(s)
                    && s.pos[1] == t.pos[1]
                    && (s.pos[0] - p.pos[0], s.pos[2] - p.pos[2]) == (dx, dz)
                {
                    k = j;
                } else {
                    break;
                }
            }
        }
        let point = steps[k].target();
        self.aim_dist = hdist(obs.pos, point);
        if self.aim_dist < 0.05 {
            return obs.yaw;
        }
        yaw_to(obs.pos, point)
    }

    /// The direction a climber at `p` pushes toward (the wall the ladder hangs on).
    fn climb_wall<S: BlockSource + ?Sized>(&self, w: &S, p: [i32; 3]) -> Option<(i32, i32)> {
        let c = self.class(w, p);
        match c.wall {
            1 => return Some((0, -1)),
            2 => return Some((0, 1)),
            3 => return Some((-1, 0)),
            4 => return Some((1, 0)),
            _ => {}
        }
        if !c.has(flag::CLIMB) {
            return None;
        }
        [(0, 1), (0, -1), (1, 0), (-1, 0)]
            .into_iter()
            .find(|&(dx, dz)| {
                let n = self.class(w, [p[0] + dx, p[1], p[2] + dz]);
                n.has(flag::SUPPORT) && n.top >= 1.0 && n.bottom <= 0.0
            })
    }

    fn move_keys<P: Physics, S: BlockSource + ?Sized>(
        &mut self,
        obs: &Observation,
        look: (f32, f32),
        world: &S,
    ) -> Keys {
        let view = SourceView::<P, S>::new(world);
        let steps = &self.path.steps;
        let last = steps.len() - 1;
        let f = &steps[self.reached];
        let t = &steps[self.reached + 1];
        let err = wrap(self.want.0 - look.0).abs();
        let fwd = Keys {
            forward: true,
            ..Keys::NONE
        };
        let (dx, dz) = (t.pos[0] - f.pos[0], t.pos[2] - f.pos[2]);
        match t.kind {
            MoveKind::Start | MoveKind::Walk | MoveKind::Diagonal => {
                let mut k = Keys {
                    forward: err < 35.0,
                    ..Keys::NONE
                };
                let in_water = f.node == NodeKind::Swim || t.node == NodeKind::Swim;
                k.sprint = k.forward
                    && self.cfg.sprint
                    && obs.food > 6
                    && err < 10.0
                    && self.aim_dist > 2.5
                    && !in_water;
                if self.reached + 1 == last {
                    let level = |p: [f64; 3]| (p[1] - t.feet_y).abs() < 0.02;
                    if hdist(obs.pos, t.target()) < 0.15 && level(obs.pos) {
                        return Keys::NONE;
                    }
                    let rest = rollout::<P, S>(&view, obs, Keys::NONE, look, 40);
                    let fb = Pathfinder::start_block(rest.pos);
                    if fb[0] == t.pos[0]
                        && fb[2] == t.pos[2]
                        && rest.on_ground
                        && level(rest.pos)
                        && hdist(rest.pos, t.target()) <= self.cfg.arrive_radius * 0.8
                    {
                        return Keys::NONE;
                    }
                    k.sprint = k.sprint && hdist(obs.pos, t.target()) > 3.0;
                }
                if k.forward && obs.on_ground {
                    // Never step off the floor where the path does not go: this tick's keys,
                    // then braking, must keep the player on a floor or over the path. And
                    // brake before a wall rather than slide along it (corners of narrow
                    // corridors) when braking avoids it.
                    let back = Keys {
                        back: true,
                        ..Keys::NONE
                    };
                    let cands = [k, Keys { sprint: false, ..k }, Keys::NONE, back];
                    let checks = cands.map(|c| self.safe::<P, S>(&view, obs, c, look));
                    let first_on_floor = checks.iter().position(|c| c.0);
                    // Braking only helps when moving: slowly, a touch is unavoidable (a box
                    // grazing a wall), and standing still would never get anywhere.
                    let moving = obs.vel[0].hypot(obs.vel[2]) > 0.03;
                    let pick = checks
                        .iter()
                        .position(|&(floor, clear)| floor && (clear || !moving))
                        .or(first_on_floor);
                    let Some(i) = pick else {
                        return back;
                    };
                    if first_on_floor.is_some_and(|j| j >= 2)
                        && !self.recover
                        && hdist(obs.pos, f.target()) > 0.3
                    {
                        // Only braking keeps to the floor here: go back to the last step's
                        // centre and try the move again from there.
                        self.recover = true;
                    }
                    return cands[i];
                }
                k
            }
            MoveKind::Ascend => {
                let (ux, uz) = (dx.signum(), dz.signum());
                let boundary = boundary_dist(obs.pos, f.pos, ux, uz);
                // Jump only when lined up: sideways drift during the jump can catch a
                // neighbouring column's block.
                let side_speed = if ux != 0 { obs.vel[2] } else { obs.vel[0] }.abs();
                let lined_up = (ux != 0 && uz != 0)
                    || (lateral(obs.pos, f.target(), ux, uz) < 0.2 && side_speed < 0.05);
                if !obs.on_ground && obs.pos[1] >= t.feet_y {
                    // Over the edge: land on the block, not beyond it.
                    return self.steer::<P, S>(&view, obs, look, t, fwd);
                }
                Keys {
                    forward: err < 35.0,
                    jump: obs.on_ground
                        && err < 20.0
                        && boundary < 0.9
                        && lined_up
                        && obs.pos[1] < t.feet_y - 0.3,
                    ..Keys::NONE
                }
            }
            MoveKind::Descend => {
                if err > 35.0 && obs.on_ground {
                    return Keys::NONE;
                }
                if self.aligning {
                    let back = Keys {
                        back: true,
                        ..Keys::NONE
                    };
                    for c in [fwd, Keys::NONE, back] {
                        if self.safe::<P, S>(&view, obs, c, look).0 {
                            return c;
                        }
                    }
                    return back;
                }
                self.steer::<P, S>(&view, obs, look, t, fwd)
            }
            MoveKind::Parkour { gap } => self.leap_keys::<P, S>(&view, obs, look, world, gap),
            MoveKind::Swim => {
                let horiz = dx != 0 || dz != 0;
                let down = t.pos[1] < f.pos[1] && t.node == NodeKind::Swim;
                Keys {
                    forward: horiz && err < 45.0,
                    jump: !down,
                    ..Keys::NONE
                }
            }
            MoveKind::ClimbUp | MoveKind::ClimbExit => Keys {
                forward: err < 35.0,
                jump: true,
                ..Keys::NONE
            },
            MoveKind::ClimbDown => {
                if obs.on_ground && (dx != 0 || dz != 0) && f.node == NodeKind::Ground {
                    // Onto a ladder from above: inch over the edge.
                    let slow = obs.vel[0].hypot(obs.vel[2]) < 0.08;
                    return Keys {
                        forward: slow && err < 35.0,
                        ..Keys::NONE
                    };
                }
                // On the ladder: stay over its column while sliding down, without pushing
                // into the wall (that would climb).
                let back = Keys {
                    back: true,
                    ..Keys::NONE
                };
                let goal = t.target();
                let mut best = (f64::INFINITY, Keys::NONE);
                for c in [Keys::NONE, fwd, back] {
                    let r = rollout::<P, S>(&view, obs, c, look, 1);
                    let score = hdist(r.pos, goal) + if r.collided { 10.0 } else { 0.0 };
                    if score < best.0 {
                        best = (score, c);
                    }
                }
                best.1
            }
        }
    }

    /// The keys of a gap jump (see [`crate::jump`] for the policy): settle on the take-off
    /// block, check the jump in the physics from there, run, jump at the edge (or at once),
    /// steer the landing.
    fn leap_keys<P: Physics, S: BlockSource + ?Sized>(
        &mut self,
        view: &SourceView<'_, P, S>,
        obs: &Observation,
        look: (f32, f32),
        world: &S,
        gap: u8,
    ) -> Keys {
        let f = self.path.steps[self.reached].clone();
        let t = self.path.steps[self.reached + 1].clone();
        let dir = (
            (t.pos[0] - f.pos[0]).signum(),
            (t.pos[2] - f.pos[2]).signum(),
        );
        let leap = match self.leap {
            Some(l) => l,
            None => {
                let dy = (t.feet_y - f.feet_y).round() as i8;
                let jumps = Jumps::of(self.path.version);
                let spec = jump::select(&jumps, f.pos, dir, gap, dy, self.cfg.sprint, |q| {
                    self.class(world, q)
                });
                match spec {
                    Some(spec) => Leap::Settle(spec.mode, 0),
                    None => {
                        // The world changed since planning: plan again.
                        self.progress = Progress::Stuck;
                        return Keys::NONE;
                    }
                }
            }
        };
        self.leap = Some(leap);
        match leap {
            Leap::Settle(mode, quiet) => {
                if mode.sprint() && obs.food <= 6 {
                    self.progress = Progress::Stuck;
                    return Keys::NONE;
                }
                if !obs.on_ground {
                    return Keys::NONE;
                }
                let fc = f.target();
                let (along, side) = axis_offsets(obs.pos, fc, dir);
                let axis = dir_yaw(dir.0, dir.1);
                let ready = wrap(look.0 - axis).abs() <= jump::YAW_ERR - 0.01
                    && obs.vel[0].hypot(obs.vel[2]) < jump::REST
                    && along.abs() <= jump::ALONG - 0.01
                    && side.abs() <= jump::LATERAL - 0.01
                    && (obs.pos[1] - f.feet_y).abs() < 1e-6
                    && quiet >= 2;
                if ready {
                    // The last word: the jump from exactly here, in the physics.
                    let start = P::from_observation(obs);
                    let Some(run) = jump::attempt::<P, S>(view, start, mode, look.0, t.pos) else {
                        self.progress = Progress::Stuck;
                        return Keys::NONE;
                    };
                    // Replay exactly the keys of the run that landed.
                    let plan = (run.run_ticks, run.release);
                    return self.leap_run(mode, look.0, plan, 0);
                }
                let k = self.inch::<P, S>(view, obs, look, &f, dir);
                self.leap = Some(Leap::Settle(
                    mode,
                    if k == Keys::NONE { quiet + 1 } else { 0 },
                ));
                k
            }
            Leap::Run(mode, yaw, plan, done) => {
                if !obs.on_ground {
                    // Left the floor early: steer the landing.
                    self.leap = Some(Leap::Air(mode, yaw, None));
                    return self.steer::<P, S>(view, obs, look, &t, mode.keys());
                }
                self.leap_run(mode, yaw, plan, done)
            }
            Leap::Air(mode, yaw, replay) => {
                if obs.on_ground {
                    // Landed short or elsewhere: off-path detection takes over.
                    return Keys::NONE;
                }
                match replay {
                    Some((hold, done)) => {
                        self.leap = Some(Leap::Air(mode, yaw, Some((hold, done + 1))));
                        if done < hold { mode.keys() } else { Keys::NONE }
                    }
                    None => self.steer::<P, S>(view, obs, look, &t, mode.keys()),
                }
            }
        }
    }

    /// One ground tick of the replayed run: the run keys, plus jump on its last ground tick
    /// (the one the check found would leave the floor, or the first one).
    fn leap_run(&mut self, mode: Mode, yaw: f32, (ground, hold): (u32, u32), done: u32) -> Keys {
        let keys = mode.keys();
        if done + 1 >= ground {
            self.leap = Some(Leap::Air(mode, yaw, Some((hold, 0))));
            return Keys { jump: true, ..keys };
        }
        self.leap = Some(Leap::Run(mode, yaw, (ground, hold), done + 1));
        keys
    }

    /// Lining up on the take-off block: the key tap (one tick, then nothing until at rest)
    /// whose resting point is closest to the block's centre in the jump's start envelope, never
    /// leaving the floor. `Keys::NONE` when no tap does better than coasting.
    fn inch<P: Physics, S: BlockSource + ?Sized>(
        &self,
        view: &SourceView<'_, P, S>,
        obs: &Observation,
        look: (f32, f32),
        f: &Step,
        dir: (i32, i32),
    ) -> Keys {
        let fc = f.target();
        let rest = |keys: Keys| -> Option<f64> {
            let mut s = P::from_observation(obs);
            let mut o = *obs;
            for i in 0..20 {
                P::tick(
                    &mut s,
                    if i == 0 { keys } else { Keys::NONE },
                    look.0,
                    look.1,
                    view,
                );
                o = P::observe(&s);
                if !o.on_ground || (o.pos[1] - f.feet_y).abs() > 1e-6 {
                    return None;
                }
                if i > 0 && o.vel[0].hypot(o.vel[2]) < jump::REST {
                    break;
                }
            }
            let (a, l) = axis_offsets(o.pos, fc, dir);
            Some((a / jump::ALONG).powi(2) + (l / jump::LATERAL).powi(2))
        };
        let mut best = (rest(Keys::NONE).unwrap_or(f64::INFINITY), Keys::NONE);
        for sneak in [true, false] {
            for (fw, bk, lf, rt) in [
                (true, false, false, false),
                (false, true, false, false),
                (false, false, true, false),
                (false, false, false, true),
                (true, false, true, false),
                (true, false, false, true),
                (false, true, true, false),
                (false, true, false, true),
            ] {
                let k = Keys {
                    forward: fw,
                    back: bk,
                    left: lf,
                    right: rt,
                    sneak,
                    ..Keys::NONE
                };
                if let Some(sc) = rest(k)
                    && sc < best.0 - 1e-9
                {
                    best = (sc, k);
                }
            }
        }
        best.1
    }

    /// Bang-bang landing control: tries "`primary` for n ticks, then nothing" (and, in the
    /// air, the same with the back key) in the physics and returns this tick's keys of the
    /// sequence that lands closest to `t`'s centre. Landing anywhere else is worse than not
    /// having left the floor yet, which is worse than landing on `t`.
    fn steer<P: Physics, S: BlockSource + ?Sized>(
        &self,
        view: &SourceView<'_, P, S>,
        obs: &Observation,
        look: (f32, f32),
        t: &Step,
        primary: Keys,
    ) -> Keys {
        let goal = t.target();
        let from_y = self.path.steps[self.reached].feet_y;
        let back = Keys {
            back: true,
            ..Keys::NONE
        };
        let mut best = (f64::INFINITY, Keys::NONE);
        let mut try_seq = |first: Keys, n: u32| {
            let mut s = P::from_observation(obs);
            let mut left = !obs.on_ground;
            let mut o = *obs;
            let mut rested = false;
            for i in 0..48 {
                let k = if i < n { first } else { Keys::NONE };
                P::tick(&mut s, k, look.0, look.1, view);
                o = P::observe(&s);
                let swim_in = t.node == NodeKind::Swim && o.pos[1] < f64::from(t.pos[1]) + 0.9;
                let on_target = (o.pos[1] - t.feet_y).abs() < 0.02;
                if !o.on_ground && !swim_in {
                    left = true;
                } else if left && !swim_in && !on_target && (o.pos[1] - from_y).abs() < 0.02 {
                    // Back on the take-off floor (stepped down onto it): keep going.
                    left = false;
                } else if left || swim_in {
                    left = true;
                    break;
                } else if i >= n && o.vel[0].hypot(o.vel[2]) < 0.003 {
                    rested = true;
                    break;
                }
            }
            let fb = Pathfinder::start_block(o.pos);
            let d = hdist(o.pos, goal);
            let in_col = fb[0] == t.pos[0] && fb[2] == t.pos[2];
            let score = if !left || rested {
                100.0 + d
            } else if in_col && (t.node != NodeKind::Ground || (o.pos[1] - t.feet_y).abs() < 0.3) {
                d
            } else {
                1000.0 + d
            };
            if score < best.0 {
                best = (score, if n == 0 { Keys::NONE } else { first });
            }
        };
        for n in 0..=10 {
            try_seq(primary, n);
        }
        if !obs.on_ground {
            for n in 1..=6 {
                try_seq(back, n);
            }
        }
        best.1
    }

    /// Whether a feet position is in a column of the current move (or the next one).
    fn allowed_column(&self, pos: [f64; 3]) -> bool {
        let fb = Pathfinder::start_block(pos);
        let steps = &self.path.steps;
        let f = &steps[self.reached];
        let t = &steps[self.reached + 1];
        let col = |p: [i32; 3]| p[0] == fb[0] && p[2] == fb[2];
        if col(f.pos) || col(t.pos) {
            return true;
        }
        steps.get(self.reached + 2).is_some_and(|n| col(n.pos))
    }

    /// `keys` this tick, then braking until at rest (at most 12 ticks): (never leaves the
    /// floor outside the move's columns, never collides horizontally).
    fn safe<P: Physics, S: BlockSource + ?Sized>(
        &self,
        view: &SourceView<'_, P, S>,
        obs: &Observation,
        keys: Keys,
        look: (f32, f32),
    ) -> (bool, bool) {
        let mut s = P::from_observation(obs);
        let mut clear = true;
        for i in 0..12 {
            let k = if i == 0 { keys } else { Keys::NONE };
            P::tick(&mut s, k, look.0, look.1, view);
            let o = P::observe(&s);
            clear &= !P::horizontal_collision(&s);
            if !o.on_ground && !self.allowed_column(o.pos) {
                return (false, clear);
            }
            if i > 0 && o.on_ground && o.vel[0].hypot(o.vel[2]) < 0.003 {
                break;
            }
        }
        (true, clear)
    }
}

/// A step without digs or places.
fn plain(s: &Step) -> bool {
    s.dig.is_empty() && s.place.is_none() && s.node == NodeKind::Ground
}

/// Runs the physics from `obs` with `keys` held for up to `max` ticks; stops on landing (after
/// having been in the air) or once at rest on the ground.
fn rollout<P: Physics, S: BlockSource + ?Sized>(
    view: &SourceView<'_, P, S>,
    obs: &Observation,
    keys: Keys,
    look: (f32, f32),
    max: u32,
) -> Roll {
    let mut s = P::from_observation(obs);
    let mut left = !obs.on_ground;
    let mut collided = false;
    let mut o = *obs;
    for i in 0..max {
        P::tick(&mut s, keys, look.0, look.1, view);
        o = P::observe(&s);
        collided |= P::horizontal_collision(&s);
        if !o.on_ground {
            left = true;
        } else if left || (i > 0 && o.vel[0].hypot(o.vel[2]) < 0.003) {
            break;
        }
    }
    Roll {
        pos: o.pos,
        on_ground: o.on_ground,
        collided,
    }
}

/// `MathHelper.wrapDegrees` for `f32`.
pub(crate) fn wrap(a: f32) -> f32 {
    let mut v = a % 360.0;
    if v >= 180.0 {
        v -= 360.0;
    }
    if v < -180.0 {
        v += 360.0;
    }
    v
}

/// Horizontal distance.
fn hdist(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}

/// Yaw (vanilla convention: +Z is 0, -X is 90) from `from` toward `to`.
fn yaw_to(from: [f64; 3], to: [f64; 3]) -> f32 {
    (-(to[0] - from[0])).atan2(to[2] - from[2]).to_degrees() as f32
}

/// The yaw of a horizontal direction.
fn dir_yaw(dx: i32, dz: i32) -> f32 {
    (-f64::from(dx)).atan2(f64::from(dz)).to_degrees() as f32
}

/// Yaw / pitch that look from `eyes` at `p`.
fn look_at(eyes: [f64; 3], p: [f64; 3]) -> (f32, f32) {
    let (dx, dy, dz) = (p[0] - eyes[0], p[1] - eyes[1], p[2] - eyes[2]);
    let yaw = (-dx).atan2(dz).to_degrees();
    let pitch = -(dy.atan2(dx.hypot(dz))).to_degrees();
    (yaw as f32, pitch as f32)
}

/// Horizontal distance of `p` from the segment `a`–`b`.
fn segment_dist(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let (ax, az, bx, bz) = (a[0], a[2], b[0], b[2]);
    let (vx, vz) = (bx - ax, bz - az);
    let len2 = vx * vx + vz * vz;
    let t = if len2 < 1e-12 {
        0.0
    } else {
        (((p[0] - ax) * vx + (p[2] - az) * vz) / len2).clamp(0.0, 1.0)
    };
    (p[0] - (ax + t * vx)).hypot(p[2] - (az + t * vz))
}

/// Offsets of `p` from `c` along the direction (`dx`, `dz`) and to its right.
fn axis_offsets(p: [f64; 3], c: [f64; 3], (dx, dz): (i32, i32)) -> (f64, f64) {
    let (ex, ez) = (p[0] - c[0], p[2] - c[2]);
    let (fx, fz) = (f64::from(dx), f64::from(dz));
    // Right-hand side of (dx, dz) is (-dz, dx).
    (ex * fx + ez * fz, -ex * fz + ez * fx)
}

/// Whether a box `r` either side of `p` overlaps block column `b`.
fn over(p: [f64; 3], b: [i32; 3], r: f64) -> bool {
    let (x0, z0) = (f64::from(b[0]), f64::from(b[2]));
    p[0] + r > x0 && p[0] - r < x0 + 1.0 && p[2] + r > z0 && p[2] - r < z0 + 1.0
}

/// Distance of `p` from the axis through `c` along (`dx`, `dz`).
fn lateral(p: [f64; 3], c: [f64; 3], dx: i32, dz: i32) -> f64 {
    if dx != 0 {
        (p[2] - c[2]).abs()
    } else if dz != 0 {
        (p[0] - c[0]).abs()
    } else {
        0.0
    }
}

/// Distance of `p` from the face of block column `from` toward (`dx`, `dz`).
fn boundary_dist(p: [f64; 3], from: [i32; 3], dx: i32, dz: i32) -> f64 {
    let mut d = f64::INFINITY;
    if dx > 0 {
        d = d.min(f64::from(from[0] + 1) - p[0]);
    } else if dx < 0 {
        d = d.min(p[0] - f64::from(from[0]));
    }
    if dz > 0 {
        d = d.min(f64::from(from[2] + 1) - p[2]);
    } else if dz < 0 {
        d = d.min(p[2] - f64::from(from[2]));
    }
    if d.is_infinite() { 0.0 } else { d }
}

#[cfg(all(test, feature = "v1_8", feature = "v1_12_2", feature = "v1_16_5"))]
mod tests {
    use super::*;

    #[test]
    fn angles() {
        assert_eq!(dir_yaw(0, 1), 0.0);
        assert_eq!(dir_yaw(-1, 0), 90.0);
        assert_eq!(dir_yaw(1, 0), -90.0);
        assert_eq!(dir_yaw(0, -1).abs(), 180.0);
        assert_eq!(wrap(370.0), 10.0);
        assert_eq!(wrap(-190.0), 170.0);
        let (y, p) = look_at([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
        assert_eq!(y, 0.0);
        assert!((p - 45.0).abs() < 1e-4);
    }

    #[test]
    fn geometry() {
        let a = [0.5, 64.0, 0.5];
        let b = [4.5, 64.0, 0.5];
        assert!((segment_dist([2.0, 64.0, 1.5], a, b) - 1.0).abs() < 1e-9);
        assert!((segment_dist([6.5, 64.0, 0.5], a, b) - 2.0).abs() < 1e-9);
        assert!((boundary_dist([0.5, 64.0, 0.5], [0, 64, 0], 1, 0) - 0.5).abs() < 1e-9);
        assert!((boundary_dist([0.2, 64.0, 0.5], [0, 64, 0], -1, 0) - 0.2).abs() < 1e-9);
        assert!((lateral([0.7, 64.0, 3.0], a, 0, 1) - 0.2).abs() < 1e-9);
    }
}
