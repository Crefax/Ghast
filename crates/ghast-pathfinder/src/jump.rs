//! Gap jumps (parkour), validated in the vanilla-exact physics.
//!
//! A gap jump is described relative to its take-off block: `gap` columns along one cardinal
//! axis, then a landing block `dy` higher or lower. Whether it lands is not guessed, at two
//! levels:
//!
//! 1. **Planning** uses a table ([`Jumps::of`], committed in `jump_table.rs`) measured with
//!    `ghast-physics` on canonical terrain: only the take-off and landing blocks exist (the
//!    landing is a 1×1 pillar in the void). Each run follows the exact key [policy](#policy)
//!    of the [`crate::Controller`], from a dense sample of the start envelope it guarantees
//!    (position along and across the axis, yaw error). A mode is kept only if **every** start
//!    lands on the landing block and comes to rest on it, without touching anything else. The
//!    cells the body sweeps in any of those runs are recorded: the planner offers the jump only
//!    where all of them are free (no collision, fluid, ladder or hazard), so along the jump the
//!    real world is, to the physics, the canonical terrain.
//! 2. **Before jumping**, standing still at the take-off, the controller runs the same policy
//!    in the physics from its actual state in the actual world ([`attempt`]) and jumps only if
//!    that run lands, replaying exactly its keys; otherwise it reports `Stuck` and the
//!    follower plans around the jump.
//!
//! # Policy
//! 1. stand still on the take-off block within [`ALONG`] / [`LATERAL`] of its centre, facing
//!    the axis within [`YAW_ERR`];
//! 2. hold forward (and sprint, except in the walking modes) along the axis, the yaw fixed;
//! 3. press jump on the last tick on the ground (when one more tick would leave it), or in
//!    the [`Mode::JumpWalk`] / [`Mode::JumpSprint`] modes on the first tick;
//! 4. in the air keep the keys for `n` more ticks, then release them, with the `n` that lands
//!    closest to the landing block's centre;
//! 5. after landing, keys released: the player stops on the landing block.

use ghast_physics::Keys;

use crate::Version;
use crate::class::{Class, flag};
use crate::physics::{Physics, SourceView};
use crate::world::BlockSource;

#[path = "jump_table.rs"]
#[rustfmt::skip]
mod jump_table;

/// Start envelope: largest distance from the take-off centre along the axis.
pub(crate) const ALONG: f64 = 0.2;
/// Start envelope: largest distance from the axis.
pub(crate) const LATERAL: f64 = 0.15;
/// Start envelope: largest yaw error (degrees).
pub(crate) const YAW_ERR: f32 = 0.5;
/// Horizontal speed (blocks per tick) below which the player counts as standing still.
pub(crate) const REST: f64 = 0.003;
/// Widest gap measured.
#[cfg(all(
    test,
    feature = "v1_8",
    feature = "v1_12_2",
    feature = "v1_16_5",
    feature = "v26_1"
))]
pub(crate) const MAX_GAP: u8 = 4;
/// Height changes measured (landing floor minus take-off floor).
pub(crate) const DYS: [i8; 4] = [1, 0, -1, -2];

/// How a jump is run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Mode {
    /// Walk (no sprint), jump at the edge.
    Walk,
    /// Sprint, jump at the edge.
    Sprint,
    /// Walk and jump at once (from the take-off centre).
    JumpWalk,
    /// Sprint and jump at once.
    JumpSprint,
}

impl Mode {
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) const ALL: [Mode; 4] = [Mode::Walk, Mode::Sprint, Mode::JumpWalk, Mode::JumpSprint];

    /// Sprints (needs food > 6).
    pub(crate) fn sprint(self) -> bool {
        matches!(self, Mode::Sprint | Mode::JumpSprint)
    }

    /// Jumps on the first tick instead of at the edge.
    pub(crate) fn at_once(self) -> bool {
        matches!(self, Mode::JumpWalk | Mode::JumpSprint)
    }

    /// The run keys.
    pub(crate) fn keys(self) -> Keys {
        Keys {
            forward: true,
            sprint: self.sprint(),
            ..Keys::NONE
        }
    }
}

/// A cell the body passes through, relative to the take-off feet block: `along` the jump,
/// `side` to the right of it, `up`; with the cell-local height range the body occupies. A
/// `speed` cell is instead the block half a block under the feet (1.16 takes the velocity
/// multiplier of soul sand or honey from there, even in the air): it must not slow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cell {
    pub(crate) along: i8,
    pub(crate) side: i8,
    pub(crate) up: i8,
    pub(crate) lo: f32,
    pub(crate) hi: f32,
    pub(crate) speed: bool,
}

/// A validated jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spec {
    pub(crate) gap: u8,
    pub(crate) dy: i8,
    pub(crate) mode: Mode,
    /// Ticks from the start at rest to the landing, plus settling (planner cost).
    pub(crate) cost: f32,
    /// Largest fall distance at the landing over all starts.
    pub(crate) fall: f32,
    /// Cells that must be free.
    pub(crate) cells: &'static [Cell],
}

/// The validated jumps of a version.
#[derive(Debug)]
pub(crate) struct Jumps {
    /// Sorted by (gap, dy), then cost.
    specs: &'static [Spec],
}

impl Jumps {
    /// The validated jumps of a version.
    pub(crate) fn of(version: Version) -> Jumps {
        Jumps {
            specs: match version {
                #[cfg(feature = "v1_8")]
                Version::V1_8 => jump_table::V1_8,
                #[cfg(feature = "v1_12_2")]
                Version::V1_12 => jump_table::V1_12,
                #[cfg(feature = "v1_16_5")]
                Version::V1_16_5 => jump_table::V1_16_5,
                #[cfg(feature = "v26_1")]
                Version::V26_1 => jump_table::V26_1,
            },
        }
    }

    /// The modes that land a jump over `gap` columns onto a block `dy` higher, cheapest first.
    pub(crate) fn get(&self, gap: u8, dy: i8) -> impl Iterator<Item = &'static Spec> + use<> {
        self.specs
            .iter()
            .filter(move |s| s.gap == gap && s.dy == dy)
    }

    /// The widest gap with any jump.
    pub(crate) fn widest(&self) -> u8 {
        self.specs.iter().map(|s| s.gap).max().unwrap_or(0)
    }
}

/// Whether a swept cell is free for the body: known, no collision in the swept height range,
/// no fluid, climbable or hazard.
#[inline]
pub(crate) fn cell_free(c: Class, lo: f32, hi: f32) -> bool {
    !c.has(flag::WATER | flag::LAVA | flag::AVOID | flag::CLIMB) && !c.blocks_between(lo, hi)
}

/// A floor a jump starts from or lands on: a full-height top with ordinary friction.
#[inline]
pub(crate) fn plain_floor(c: Class) -> bool {
    c.has(flag::SUPPORT)
        && !c.has(
            flag::IRREGULAR
                | flag::AVOID_FLOOR
                | flag::SLOW
                | flag::SLIPPERY
                | flag::BOUNCY
                | flag::FRAGILE
                | flag::WATER,
        )
        && (c.top - 1.0).abs() < 1e-6
        && c.top_any <= 1.0 + 1e-6
}

/// The world position of a jump-relative cell.
#[inline]
pub(crate) fn at(p: [i32; 3], (dx, dz): (i32, i32), along: i8, side: i8, up: i8) -> [i32; 3] {
    // The right-hand side of a run toward (dx, dz) is (-dz, dx).
    let (a, s) = (i32::from(along), i32::from(side));
    [
        p[0] + dx * a - dz * s,
        p[1] + i32::from(up),
        p[2] + dz * a + dx * s,
    ]
}

/// How to run a jump from take-off feet block `p` toward (`dx`, `dz`) over `gap` columns onto
/// a block `dy` higher: the cheapest validated mode (sprinting ones only if `sprint`) whose
/// cells are all free. `class` reads the world. The planner and the controller call this with
/// the same world, so they agree on the mode.
pub(crate) fn select(
    jumps: &Jumps,
    p: [i32; 3],
    dir: (i32, i32),
    gap: u8,
    dy: i8,
    sprint: bool,
    mut class: impl FnMut([i32; 3]) -> Class,
) -> Option<&'static Spec> {
    jumps.get(gap, dy).find(|spec| {
        (sprint || !spec.mode.sprint())
            && spec.cells.iter().all(|c| {
                let k = class(at(p, dir, c.along, c.side, c.up));
                if c.speed {
                    !k.has(flag::SLOW)
                } else {
                    cell_free(k, c.lo, c.hi)
                }
            })
    })
}

/// A run of the policy that landed.
#[derive(Clone, Debug)]
#[cfg_attr(
    not(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    )),
    allow(dead_code)
)]
pub(crate) struct Landed {
    /// Feet positions after every tick (until at rest on the landing block).
    pub(crate) trace: Vec<[f64; 3]>,
    /// Fall distance at the landing.
    pub(crate) fall: f32,
    /// Ticks from the start to the landing.
    pub(crate) ticks: u32,
    /// Ticks on the ground, the last of them with the jump pressed.
    pub(crate) run_ticks: u32,
    /// Air ticks the run keys stay held after the jump tick.
    pub(crate) release: u32,
}

/// Runs the [policy](self#policy) in the physics: from `start` (standing on the take-off
/// block), toward (`dx`, `dz`) with the yaw fixed at `yaw`, onto the landing block whose feet
/// block is `land` (floor at its bottom). `Some` only when the run lands on that block
/// without a horizontal collision and comes to rest on it.
pub(crate) fn attempt<P: Physics, S: BlockSource + ?Sized>(
    view: &SourceView<'_, P, S>,
    start: P::State,
    mode: Mode,
    yaw: f32,
    land: [i32; 3],
) -> Option<Landed> {
    let land_y = f64::from(land[1]);
    let goal = [f64::from(land[0]) + 0.5, f64::from(land[2]) + 0.5];
    let in_col = |p: &[f64; 3]| p[0].floor() as i32 == land[0] && p[2].floor() as i32 == land[2];
    let keys = mode.keys();
    let mut s = start;
    let mut trace = Vec::with_capacity(48);
    // Run until the jump tick.
    let mut ticks = 0u32;
    loop {
        if ticks > 40 || !P::observe(&s).on_ground {
            return None;
        }
        let leave = mode.at_once() || {
            let mut probe = s;
            P::tick(&mut probe, keys, yaw, 0.0, view);
            !P::observe(&probe).on_ground
        };
        let k = if leave {
            Keys { jump: true, ..keys }
        } else {
            keys
        };
        P::tick(&mut s, k, yaw, 0.0, view);
        ticks += 1;
        if P::horizontal_collision(&s) {
            return None;
        }
        trace.push(P::observe(&s).pos);
        if leave {
            break;
        }
    }
    // In the air: the keys for `n` more ticks, then none.
    struct Air<St> {
        trace: Vec<[f64; 3]>,
        state: St,
        landed: bool,
        fall: f32,
        ticks: u32,
    }
    let air = |n: u32| {
        let mut st = s;
        let mut tr = Vec::with_capacity(24);
        for i in 0..60u32 {
            let k = if i < n { keys } else { Keys::NONE };
            let before = P::fall_distance(&st);
            P::tick(&mut st, k, yaw, 0.0, view);
            let o = P::observe(&st);
            tr.push(o.pos);
            if P::horizontal_collision(&st) || o.pos[1] < land_y - 3.0 {
                break;
            }
            if o.on_ground {
                return Air {
                    trace: tr,
                    state: st,
                    landed: true,
                    fall: before,
                    ticks: i + 1,
                };
            }
        }
        let ticks = tr.len() as u32;
        Air {
            trace: tr,
            state: st,
            landed: false,
            fall: 0.0,
            ticks,
        }
    };
    // Keys held for the whole flight first: no later release changes anything.
    let full = air(60);
    let mut best: Option<(f64, u32)> = None;
    for n in (0..=full.ticks).rev() {
        let r = if n == full.ticks { None } else { Some(air(n)) };
        let r = r.as_ref().unwrap_or(&full);
        let Some(end) = r.trace.last() else { continue };
        let d = (end[0] - goal[0]).hypot(end[2] - goal[1]);
        let score = if r.landed && in_col(end) && (end[1] - land_y).abs() < 0.3 {
            d
        } else {
            1000.0 + d
        };
        if best.is_none_or(|(b, _)| score < b) {
            best = Some((score, n));
        }
    }
    let (score, n) = best?;
    if score >= 1000.0 {
        return None;
    }
    let run_ticks = ticks;
    let r = if n == full.ticks { full } else { air(n) };
    let mut st = r.state;
    if (P::observe(&st).pos[1] - land_y).abs() > 1e-6 {
        return None;
    }
    trace.extend(r.trace);
    ticks += r.ticks;
    // Come to rest on the landing block, keys released.
    for _ in 0..30 {
        P::tick(&mut st, Keys::NONE, yaw, 0.0, view);
        let o = P::observe(&st);
        trace.push(o.pos);
        if !o.on_ground || (o.pos[1] - land_y).abs() > 1e-6 || P::horizontal_collision(&st) {
            return None;
        }
        if o.vel[0].hypot(o.vel[2]) < REST {
            break;
        }
    }
    if !in_col(&P::observe(&st).pos) {
        return None;
    }
    Some(Landed {
        trace,
        fall: r.fall,
        ticks,
        run_ticks,
        release: n,
    })
}

/// Measuring the table (tests only: `jump_table.rs` is generated from it).
#[cfg(all(
    test,
    feature = "v1_8",
    feature = "v1_12_2",
    feature = "v1_16_5",
    feature = "v26_1"
))]
pub(crate) mod measure {
    use std::collections::HashMap;
    use std::fmt::Write as _;

    use super::*;
    use crate::physics::{P12, P16, P18, P26};

    /// Stone blocks at the given positions, air elsewhere.
    pub(crate) struct Pillars {
        pub(crate) blocks: Vec<[i32; 3]>,
        pub(crate) stone: u32,
    }

    impl BlockSource for Pillars {
        fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
            if !(0..256).contains(&y) {
                return None;
            }
            Some(if self.blocks.contains(&[x, y, z]) {
                self.stone
            } else {
                0
            })
        }

        fn is_column_loaded(&self, _: i32, _: i32) -> bool {
            true
        }
    }

    pub(crate) fn stone(version: Version) -> u32 {
        match version {
            Version::V1_8 | Version::V1_12 => 1 << 4,
            Version::V1_16_5 => u32::from(ghast_data::v1_16_5::Block::STONE.default_state().id()),
            Version::V26_1 => u32::from(ghast_data::v26_1::Block::STONE.default_state().id()),
        }
    }

    /// How far below the body nothing may be until the landing (blocks).
    pub(crate) const BELOW: f64 = 0.3;

    /// A measured mode (owned cells).
    #[derive(Clone, Debug, PartialEq)]
    pub(crate) struct Measured {
        pub(crate) gap: u8,
        pub(crate) dy: i8,
        pub(crate) mode: Mode,
        pub(crate) cost: f32,
        pub(crate) fall: f32,
        pub(crate) cells: Vec<Cell>,
    }

    /// Starts: (along, side, yaw error). `dense`: the along axis every 0.025.
    pub(crate) fn starts(dense: bool) -> Vec<(f64, f64, f32)> {
        let mut v = vec![(0.0, 0.0, 0.0)];
        let alongs: Vec<f64> = if dense {
            (-8..=8).map(|i| f64::from(i) * ALONG / 8.0).collect()
        } else {
            vec![-ALONG, 0.0, ALONG]
        };
        for &a in &alongs {
            for s in [-LATERAL, 0.0, LATERAL] {
                for y in [-YAW_ERR, 0.0, YAW_ERR] {
                    if (a, s, y) != (0.0, 0.0, 0.0) {
                        v.push((a, s, y));
                    }
                }
            }
        }
        v
    }

    /// The canonical run toward +z (yaw 0) from feet block (0, 64, 0) onto (0, 64 + dy,
    /// gap + 1), started at rest at offset (along, side) from the centre with a yaw error.
    pub(crate) fn canonical<P: Physics>(
        gap: u8,
        dy: i8,
        mode: Mode,
        (along, side, dyaw): (f64, f64, f32),
    ) -> Option<Landed> {
        let land = [0, 64 + i32::from(dy), i32::from(gap) + 1];
        let world = Pillars {
            blocks: vec![[0, 63, 0], [land[0], land[1] - 1, land[2]]],
            stone: stone(P::VERSION),
        };
        let view = SourceView::<P, Pillars>::new(&world);
        // Yaw 0 runs toward +z; its right-hand side is -x.
        let mut s = P::new_state([0.5 - side, 64.0, 0.5 + along], dyaw, 0.0);
        for _ in 0..3 {
            P::tick(&mut s, Keys::NONE, dyaw, 0.0, &view);
        }
        attempt::<P, Pillars>(&view, s, mode, dyaw, land)
    }

    fn measure<P: Physics>(dense: bool) -> Vec<Measured> {
        let starts = starts(dense);
        let mut out = Vec::new();
        for gap in 1..=MAX_GAP {
            for dy in DYS {
                let mut specs = Vec::new();
                for mode in Mode::ALL {
                    let mut traces = Vec::new();
                    let mut fall = 0.0f32;
                    let mut nominal = 0;
                    let ok = starts
                        .iter()
                        .all(|&st| match canonical::<P>(gap, dy, mode, st) {
                            Some(r) => {
                                fall = fall.max(r.fall);
                                if st == (0.0, 0.0, 0.0) {
                                    nominal = r.ticks;
                                }
                                traces.push((r.trace, r.ticks));
                                true
                            }
                            None => false,
                        });
                    if !ok {
                        continue;
                    }
                    specs.push(Measured {
                        gap,
                        dy,
                        mode,
                        cost: nominal as f32 + 4.0,
                        fall,
                        cells: sweep(&traces, gap, dy),
                    });
                }
                specs.sort_by(|a, b| a.cost.total_cmp(&b.cost).then(a.mode.cmp(&b.mode)));
                out.extend(specs);
            }
        }
        out
    }

    /// Measures a version's table.
    pub(crate) fn table(version: Version, dense: bool) -> Vec<Measured> {
        match version {
            Version::V1_8 => measure::<P18>(dense),
            Version::V1_12 => measure::<P12>(dense),
            Version::V1_16_5 => measure::<P16>(dense),
            Version::V26_1 => measure::<P26>(dense),
        }
    }

    /// The cells the body overlaps in any trace (canonical coordinates), without the floors.
    /// Up to the landing, the body is taken [`BELOW`] deeper than it is: nothing may be
    /// close enough under it to catch it before the landing block does (the canonical terrain
    /// has nothing there).
    fn sweep(traces: &[(Vec<[f64; 3]>, u32)], gap: u8, dy: i8) -> Vec<Cell> {
        const E: f64 = 1e-6;
        let floors = [(0, 0, -1), (i32::from(gap) + 1, 0, i32::from(dy) - 1)];
        let mut cells: HashMap<(i32, i32, i32), (f32, f32)> = HashMap::new();
        let mut speed: Vec<(i32, i32, i32)> = Vec::new();
        let positions = traces.iter().flat_map(|(t, landing)| {
            t.iter()
                .enumerate()
                .map(move |(i, p)| (p, (i as u32) < *landing))
        });
        for (p, flying) in positions {
            // The block whose velocity multiplier applies (1.16: half a block under the feet).
            let key = (
                p[2].floor() as i32,
                -(p[0].floor() as i32),
                (p[1] - 0.500_000_1).floor() as i32 - 64,
            );
            if !floors.contains(&key) && !speed.contains(&key) {
                speed.push(key);
            }
            let (x0, x1) = (p[0] - 0.3 + E, p[0] + 0.3 - E);
            let (z0, z1) = (p[2] - 0.3 + E, p[2] + 0.3 - E);
            let below = if flying { BELOW } else { -E };
            let (y0, y1) = (p[1] - below, p[1] + 1.8 - E);
            for cx in x0.floor() as i32..=x1.floor() as i32 {
                for cz in z0.floor() as i32..=z1.floor() as i32 {
                    for cy in y0.floor() as i32..=y1.floor() as i32 {
                        // along = z, side = -x, up = y - 64.
                        let key = (cz, -cx, cy - 64);
                        if floors.contains(&key) {
                            continue;
                        }
                        // Rounded outward to 1/64 so the table is stable and conservative.
                        let lo = ((y0 - f64::from(cy)).max(0.0) * 64.0).floor() / 64.0;
                        let hi = ((y1 - f64::from(cy)).min(1.0) * 64.0).ceil() / 64.0;
                        let e = cells.entry(key).or_insert((lo as f32, hi as f32));
                        e.0 = e.0.min(lo as f32);
                        e.1 = e.1.max(hi as f32);
                    }
                }
            }
        }
        let mut v: Vec<Cell> = cells
            .into_iter()
            .map(|((a, s, u), (lo, hi))| Cell {
                along: a as i8,
                side: s as i8,
                up: u as i8,
                lo,
                hi,
                speed: false,
            })
            .chain(speed.into_iter().map(|(a, s, u)| Cell {
                along: a as i8,
                side: s as i8,
                up: u as i8,
                lo: 0.0,
                hi: 0.0,
                speed: true,
            }))
            .collect();
        v.sort_by_key(|c| (c.speed, c.along, c.side, c.up));
        v
    }

    /// The source of `jump_table.rs`.
    pub(crate) fn source(
        t18: &[Measured],
        t12: &[Measured],
        t16: &[Measured],
        t26: &[Measured],
    ) -> String {
        let mut s = String::from(
            "//! Generated by `GHAST_UPDATE_JUMPS=1 cargo test --release -p ghast-pathfinder --lib \
             -- --ignored jump_table`\n//! from the physics (see `jump.rs`); do not edit.\n\n\
             use super::{Cell, Mode, Spec};\n\n\
             const fn c(along: i8, side: i8, up: i8, lo: f32, hi: f32) -> Cell {\n    \
             Cell { along, side, up, lo, hi, speed: false }\n}\n\n\
             const fn v(along: i8, side: i8, up: i8) -> Cell {\n    \
             Cell { along, side, up, lo: 0.0, hi: 0.0, speed: true }\n}\n",
        );
        for (name, feature, t) in [
            ("V1_8", "v1_8", t18),
            ("V1_12", "v1_12_2", t12),
            ("V1_16_5", "v1_16_5", t16),
            ("V26_1", "v26_1", t26),
        ] {
            let _ = write!(
                s,
                "\n#[cfg(feature = \"{feature}\")]\npub(super) static {name}: &[Spec] = &[\n"
            );
            for m in t {
                let _ = write!(
                    s,
                    "    Spec {{\n        gap: {},\n        dy: {},\n        mode: Mode::{:?},\n        \
                     cost: {:?},\n        fall: {:?},\n        cells: &[\n",
                    m.gap, m.dy, m.mode, m.cost, m.fall
                );
                for c in &m.cells {
                    let _ = if c.speed {
                        writeln!(s, "            v({}, {}, {}),", c.along, c.side, c.up)
                    } else {
                        writeln!(
                            s,
                            "            c({}, {}, {}, {:?}, {:?}),",
                            c.along, c.side, c.up, c.lo, c.hi
                        )
                    };
                }
                s.push_str("        ],\n    },\n");
            }
            s.push_str("];\n");
        }
        s
    }

    /// The committed table as measured rows.
    pub(crate) fn committed(version: Version) -> Vec<Measured> {
        Jumps::of(version)
            .specs
            .iter()
            .map(|s| Measured {
                gap: s.gap,
                dy: s.dy,
                mode: s.mode,
                cost: s.cost,
                fall: s.fall,
                cells: s.cells.to_vec(),
            })
            .collect()
    }
}

#[cfg(all(
    test,
    feature = "v1_8",
    feature = "v1_12_2",
    feature = "v1_16_5",
    feature = "v26_1"
))]
mod tests {
    use super::measure::*;
    use super::*;
    use crate::physics::{P12, P16, P18, P26};

    /// The committed table is the dense measurement of the current physics and policy.
    /// Regenerate with `GHAST_UPDATE_JUMPS=1 cargo test --release -p ghast-pathfinder --lib
    /// -- --ignored jump_table`.
    #[test]
    #[ignore = "measures ~150 starts per jump; run in release on demand"]
    fn jump_table_is_current() {
        let t18 = table(Version::V1_8, true);
        let t12 = table(Version::V1_12, true);
        let t16 = table(Version::V1_16_5, true);
        let t26 = table(Version::V26_1, true);
        if std::env::var_os("GHAST_UPDATE_JUMPS").is_some() {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/jump_table.rs");
            std::fs::write(path, source(&t18, &t12, &t16, &t26)).expect("write jump_table.rs");
            return;
        }
        assert_eq!(t18, committed(Version::V1_8), "1.8 table out of date");
        assert_eq!(t12, committed(Version::V1_12), "1.12 table out of date");
        assert_eq!(t16, committed(Version::V1_16_5), "1.16.5 table out of date");
        assert_eq!(t26, committed(Version::V26_1), "26.1 table out of date");
    }

    /// Every committed jump still lands from the centre and the corners of the envelope, at
    /// the committed cost (cheap enough for every test run).
    #[test]
    fn committed_jumps_land() {
        for v in [
            Version::V1_8,
            Version::V1_12,
            Version::V1_16_5,
            Version::V26_1,
        ] {
            let rows = committed(v);
            assert!(!rows.is_empty());
            for m in rows {
                for st in [
                    (0.0, 0.0, 0.0),
                    (-ALONG, -LATERAL, -YAW_ERR),
                    (ALONG, LATERAL, YAW_ERR),
                ] {
                    let r = match v {
                        Version::V1_8 => canonical::<P18>(m.gap, m.dy, m.mode, st),
                        Version::V1_12 => canonical::<P12>(m.gap, m.dy, m.mode, st),
                        Version::V1_16_5 => canonical::<P16>(m.gap, m.dy, m.mode, st),
                        Version::V26_1 => canonical::<P26>(m.gap, m.dy, m.mode, st),
                    };
                    let r = r.unwrap_or_else(|| panic!("{v:?} {m:?} from {st:?} does not land"));
                    if st == (0.0, 0.0, 0.0) {
                        assert_eq!(r.ticks as f32 + 4.0, m.cost, "{v:?} {m:?}");
                    }
                    assert!(r.fall <= m.fall + 1e-6);
                }
            }
        }
    }

    #[test]
    fn coverage() {
        // What the tests and the README promise.
        for v in [
            Version::V1_8,
            Version::V1_12,
            Version::V1_16_5,
            Version::V26_1,
        ] {
            let j = Jumps::of(v);
            for (gap, dy) in [
                (1, 0),
                (2, 0),
                (3, 0),
                (1, 1),
                (2, 1),
                (1, -1),
                (2, -1),
                (3, -1),
            ] {
                assert!(
                    j.get(gap, dy).next().is_some(),
                    "{v:?}: no jump over {gap} dy {dy}"
                );
            }
            for (gap, dy) in [(4, 0), (4, 1), (5, 0)] {
                assert!(
                    j.get(gap, dy).next().is_none(),
                    "{v:?}: jump over {gap} dy {dy}"
                );
            }
            assert!(j.widest() <= MAX_GAP);
        }
    }

    #[test]
    fn relative_cells() {
        // Toward +x the right-hand side is +z; toward +z it is -x.
        assert_eq!(at([0, 64, 0], (1, 0), 2, 1, 1), [2, 65, 1]);
        assert_eq!(at([0, 64, 0], (0, 1), 2, 1, 0), [-1, 64, 2]);
        assert_eq!(at([0, 64, 0], (-1, 0), 1, -1, 0), [-1, 64, 1]);
    }
}
