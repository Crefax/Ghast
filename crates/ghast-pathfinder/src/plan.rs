//! The planner: A* over feet blocks with physics-measured move costs.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::Version;
use crate::class::flag;
use crate::control::ControllerConfig;
use crate::cost::Costs;
use crate::goal::{Goal, Heuristic};
use crate::moves::{
    HEIGHT, JUMP_APEX, JUMP_MARGIN, MAX_JUMP_UP, MoveKind, NodeKind, STEP, Spot, ceiling, clear,
    climb_wall, spot,
};
use crate::sim::{SimOutcome, SimReport, simulate};
use crate::world::{BlockSource, BuildKeyHasher, Grid};

/// Digging through blocks (off by default): cost per block in ticks
/// `base + ticks_per_hardness * hardness`, blocks harder than `max_hardness` are never dug.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DigCost {
    /// Fixed ticks per block (aiming, the server's confirmation).
    pub base: f64,
    /// Ticks per unit of vanilla hardness (hand on dirt ≈ 30, a good pickaxe on stone ≈ 4).
    pub ticks_per_hardness: f64,
    /// Hardest block that may be dug.
    pub max_hardness: f64,
}

impl Default for DigCost {
    fn default() -> Self {
        DigCost {
            base: 6.0,
            ticks_per_hardness: 30.0,
            max_hardness: 2.0,
        }
    }
}

/// Planner options. Build with [`Config::default`] and the `with_*` methods.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Config {
    /// Sprint on straight runs (costs assume sprinting; needs food > 6). Default on.
    pub sprint: bool,
    /// Jump over 1-block gaps (and 2-block gaps when sprinting). Default on.
    pub parkour: bool,
    /// Highest drop onto a floor, in blocks (3 = no fall damage). Default 3.
    pub max_fall: f64,
    /// Highest drop into water at least 2 blocks deep. Default 16.
    pub max_water_fall: f64,
    /// Swim. Default on.
    pub swim: bool,
    /// Climb ladders and vines. Default on.
    pub climb: bool,
    /// Dig through blocks in the way (default off). The follower digs with the held item.
    pub dig: Option<DigCost>,
    /// Place a block to fill a one-deep hole, at this cost in ticks (default off). The
    /// follower places the held item: the caller keeps a block in hand.
    pub place: Option<f64>,
    /// Node budget per plan; when exhausted the best partial path is returned.
    pub max_nodes: usize,
    /// Heuristic weight (1 = optimal; larger plans faster, slightly longer paths).
    pub heuristic_weight: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            sprint: true,
            parkour: true,
            max_fall: 3.0,
            max_water_fall: 16.0,
            swim: true,
            climb: true,
            dig: None,
            place: None,
            max_nodes: 100_000,
            heuristic_weight: 1.0,
        }
    }
}

impl Config {
    /// Sprinting on or off.
    #[must_use]
    pub fn with_sprint(mut self, on: bool) -> Self {
        self.sprint = on;
        self
    }
    /// Gap jumps on or off.
    #[must_use]
    pub fn with_parkour(mut self, on: bool) -> Self {
        self.parkour = on;
        self
    }
    /// Highest drop onto a floor.
    #[must_use]
    pub fn with_max_fall(mut self, blocks: f64) -> Self {
        self.max_fall = blocks;
        self
    }
    /// Swimming on or off.
    #[must_use]
    pub fn with_swim(mut self, on: bool) -> Self {
        self.swim = on;
        self
    }
    /// Climbing on or off.
    #[must_use]
    pub fn with_climb(mut self, on: bool) -> Self {
        self.climb = on;
        self
    }
    /// Digging (None = off).
    #[must_use]
    pub fn with_dig(mut self, dig: Option<DigCost>) -> Self {
        self.dig = dig;
        self
    }
    /// Placing blocks into one-deep holes (cost in ticks; None = off).
    #[must_use]
    pub fn with_place(mut self, cost: Option<f64>) -> Self {
        self.place = cost;
        self
    }
    /// Node budget.
    #[must_use]
    pub fn with_max_nodes(mut self, n: usize) -> Self {
        self.max_nodes = n;
        self
    }
    /// Heuristic weight (>= 1).
    #[must_use]
    pub fn with_heuristic_weight(mut self, w: f64) -> Self {
        self.heuristic_weight = w.max(1.0);
        self
    }
}

/// A block to place before a step: the new block at `pos`, clicked on the top face of
/// `against` (the block below it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    /// Where the new block goes.
    pub pos: [i32; 3],
    /// The block whose top face is clicked.
    pub against: [i32; 3],
}

/// One step of a path: the feet block to reach and how.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    /// Feet block.
    pub pos: [i32; 3],
    /// Feet height when standing there (swim / climb steps: the block's y).
    pub feet_y: f64,
    /// The move that reaches this step from the previous one.
    pub kind: MoveKind,
    /// How the player is held up here.
    pub node: NodeKind,
    /// Blocks to break before the move (in order).
    pub dig: Vec<[i32; 3]>,
    /// Block to place before the move.
    pub place: Option<Place>,
}

impl Step {
    /// Centre of the step at feet height.
    #[must_use]
    pub fn target(&self) -> [f64; 3] {
        [
            f64::from(self.pos[0]) + 0.5,
            self.feet_y,
            f64::from(self.pos[2]) + 0.5,
        ]
    }
}

/// A planned path.
#[derive(Clone, Debug, PartialEq)]
pub struct Path {
    /// `steps[0]` is the start; the last step reaches the goal (or the best node found).
    pub steps: Vec<Step>,
    /// Estimated cost in ticks.
    pub cost: f64,
    /// The goal was not reached (node budget, unknown terrain, or unreachable): the path ends
    /// at the node closest to the goal. Replan from there.
    pub partial: bool,
    /// Nodes expanded.
    pub expanded: usize,
    /// The version the path was planned for.
    pub version: Version,
}

/// Why planning failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PlanError {
    /// The start block is not loaded.
    StartUnknown,
    /// The player cannot stand in the start block (inside a block, in lava, ...).
    StartBlocked,
    /// No node closer to the goal is reachable.
    NoPath,
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PlanError::StartUnknown => "start block not loaded",
            PlanError::StartBlocked => "cannot stand at the start",
            PlanError::NoPath => "no path",
        })
    }
}

impl std::error::Error for PlanError {}

/// The path planner. Cheap to clone; holds only its configuration.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pathfinder {
    cfg: Config,
}

#[derive(Clone, Copy)]
pub(crate) struct Edge {
    pub(crate) to: [i32; 3],
    pub(crate) spot: Spot,
    pub(crate) kind: MoveKind,
    pub(crate) cost: f64,
    pub(crate) dig: [Option<[i32; 3]>; 3],
    pub(crate) place: Option<Place>,
}

struct Rec {
    pos: [i32; 3],
    spot: Spot,
    g: f64,
    h: f64,
    parent: u32,
    kind: MoveKind,
    dig: [Option<[i32; 3]>; 3],
    place: Option<Place>,
    closed: bool,
}

#[derive(PartialEq)]
struct Open {
    f: f64,
    h: f64,
    idx: u32,
}

impl Eq for Open {}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap on f, then on h (deeper nodes first).
        other
            .f
            .total_cmp(&self.f)
            .then_with(|| other.h.total_cmp(&self.h))
    }
}

#[inline]
pub(crate) fn pack(p: [i32; 3]) -> u64 {
    (p[0] as u32 as u64 & 0x3ff_ffff) << 34
        | (p[2] as u32 as u64 & 0x3ff_ffff) << 8
        | (p[1] as u8 as u64)
}

impl Pathfinder {
    /// A planner with this configuration.
    #[must_use]
    pub fn new(cfg: Config) -> Self {
        Pathfinder { cfg }
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// The start block of a player at feet position `pos` (the block under the feet when it
    /// stands on a partial block, else the block containing the feet).
    #[must_use]
    pub fn start_block(pos: [f64; 3]) -> [i32; 3] {
        [
            pos[0].floor() as i32,
            (pos[1] + 1e-4).floor() as i32,
            pos[2].floor() as i32,
        ]
    }

    /// Plans a path from the feet block `from` to `goal`.
    ///
    /// # Errors
    /// [`PlanError`] when the start is unknown or blocked, or no node closer to the goal can
    /// be reached.
    pub fn plan<S: BlockSource + ?Sized>(
        &self,
        version: Version,
        world: &S,
        from: [i32; 3],
        goal: &Goal,
    ) -> Result<Path, PlanError> {
        self.plan_avoiding(version, world, from, goal, &HashSet::new())
    }

    /// Like [`Pathfinder::plan`], never using the moves in `banned` (`(from, to)` feet blocks).
    ///
    /// # Errors
    /// See [`Pathfinder::plan`].
    pub fn plan_avoiding<S: BlockSource + ?Sized>(
        &self,
        version: Version,
        world: &S,
        from: [i32; 3],
        goal: &Goal,
        banned: &HashSet<([i32; 3], [i32; 3])>,
    ) -> Result<Path, PlanError> {
        let mut g = Grid::new(world, version);
        let costs = Costs::of(version);
        let start_spot = start_spot(&mut g, from)?;
        let heur = self.heuristic(costs);
        let w = self.cfg.heuristic_weight;
        let mut recs: Vec<Rec> = Vec::with_capacity(4096);
        let mut index: HashMap<u64, u32, BuildKeyHasher> =
            HashMap::with_capacity_and_hasher(4096, BuildKeyHasher);
        let mut open = BinaryHeap::with_capacity(4096);
        let h0 = goal.estimate(from, &heur);
        recs.push(Rec {
            pos: from,
            spot: start_spot,
            g: 0.0,
            h: h0,
            parent: u32::MAX,
            kind: MoveKind::Start,
            dig: [None; 3],
            place: None,
            closed: false,
        });
        index.insert(pack(from), 0);
        open.push(Open {
            f: h0 * w,
            h: h0,
            idx: 0,
        });
        let mut best = 0u32;
        let mut expanded = 0usize;
        let mut edges = Vec::with_capacity(32);
        let mut found = None;
        while let Some(Open { idx, .. }) = open.pop() {
            let r = &mut recs[idx as usize];
            if r.closed {
                continue;
            }
            r.closed = true;
            let (pos, sp, gcost) = (r.pos, r.spot, r.g);
            if goal.is_reached(pos) {
                found = Some(idx);
                break;
            }
            expanded += 1;
            if expanded > self.cfg.max_nodes {
                break;
            }
            edges.clear();
            successors(&mut g, &self.cfg, version, costs, pos, &sp, &mut edges);
            for e in &edges {
                if !banned.is_empty() && banned.contains(&(pos, e.to)) {
                    continue;
                }
                let ng = gcost + e.cost;
                let key = pack(e.to);
                match index.get(&key) {
                    Some(&i) => {
                        let rr = &mut recs[i as usize];
                        if rr.closed || ng >= rr.g - 1e-9 {
                            continue;
                        }
                        rr.g = ng;
                        rr.parent = idx;
                        rr.spot = e.spot;
                        rr.kind = e.kind;
                        rr.dig = e.dig;
                        rr.place = e.place;
                        let h = rr.h;
                        open.push(Open {
                            f: ng + h * w,
                            h,
                            idx: i,
                        });
                    }
                    None => {
                        let h = goal.estimate(e.to, &heur);
                        let i = recs.len() as u32;
                        recs.push(Rec {
                            pos: e.to,
                            spot: e.spot,
                            g: ng,
                            h,
                            parent: idx,
                            kind: e.kind,
                            dig: e.dig,
                            place: e.place,
                            closed: false,
                        });
                        index.insert(key, i);
                        let b = &recs[best as usize];
                        if h < b.h || (h == b.h && ng < b.g) {
                            best = i;
                        }
                        open.push(Open {
                            f: ng + h * w,
                            h,
                            idx: i,
                        });
                    }
                }
            }
        }
        let (end, partial) = match found {
            Some(i) => (i, false),
            None if best != 0 => (best, true),
            None => return Err(PlanError::NoPath),
        };
        let mut steps = Vec::new();
        let mut i = end;
        while i != u32::MAX {
            let r = &recs[i as usize];
            steps.push(Step {
                pos: r.pos,
                feet_y: r.spot.floor,
                kind: r.kind,
                node: r.spot.kind,
                dig: r.dig.iter().flatten().copied().collect(),
                place: r.place,
            });
            i = r.parent;
        }
        steps.reverse();
        Ok(Path {
            cost: recs[end as usize].g,
            steps,
            partial,
            expanded,
            version,
        })
    }

    /// Plans, then runs the path through the vanilla-exact physics ([`crate::simulate`]) with
    /// a [`crate::Controller`]; a move the simulation fails is avoided and the path replanned,
    /// up to `attempts` plans. Returns the last path and its simulation: check
    /// [`SimReport::outcome`] (anything but [`crate::SimOutcome::Arrived`] means even the
    /// last plan failed in the simulation, at [`SimReport::reached`]).
    ///
    /// ```
    /// use ghast_pathfinder::{Config, Goal, Pathfinder, SimOutcome, Version};
    ///
    /// let mut world = ghast_physics::v1_8::memory_world();
    /// world.load_chunks(-1, -1, 1, 1);
    /// world.fill_raw([-10, 63, -10], [10, 63, 10], 1 << 4);
    /// let pf = Pathfinder::new(Config::default());
    /// let (path, sim) = pf.plan_verified(Version::V1_8, &world, [0, 64, 0], &Goal::Block([6, 64, 3]), 3).unwrap();
    /// assert_eq!(sim.outcome, SimOutcome::Arrived);
    /// assert!(!path.partial);
    /// ```
    ///
    /// # Errors
    /// See [`Pathfinder::plan`].
    pub fn plan_verified<S: BlockSource + ?Sized>(
        &self,
        version: Version,
        world: &S,
        from: [i32; 3],
        goal: &Goal,
        attempts: usize,
    ) -> Result<(Path, SimReport), PlanError> {
        let ctrl = ControllerConfig::default().with_sprint(self.cfg.sprint);
        self.plan_verified_avoiding(
            version,
            world,
            from,
            goal,
            &mut HashSet::new(),
            attempts,
            &ctrl,
        )
    }

    /// [`Pathfinder::plan_verified`] with moves to avoid (failed moves are added to `banned`)
    /// and the controller options the simulation uses.
    ///
    /// # Errors
    /// See [`Pathfinder::plan`].
    #[allow(clippy::too_many_arguments)]
    pub fn plan_verified_avoiding<S: BlockSource + ?Sized>(
        &self,
        version: Version,
        world: &S,
        from: [i32; 3],
        goal: &Goal,
        banned: &mut HashSet<([i32; 3], [i32; 3])>,
        attempts: usize,
        controller: &ControllerConfig,
    ) -> Result<(Path, SimReport), PlanError> {
        let mut last = None;
        for _ in 0..attempts.max(1) {
            let path = self.plan_avoiding(version, world, from, goal, banned)?;
            let sim = simulate(&path, world, controller);
            if sim.outcome == SimOutcome::Arrived {
                return Ok((path, sim));
            }
            let k = (sim.reached + 1).min(path.steps.len() - 1);
            if k == 0 || !banned.insert((path.steps[k - 1].pos, path.steps[k].pos)) {
                return Ok((path, sim));
            }
            last = Some((path, sim));
        }
        Ok(last.expect("at least one attempt"))
    }

    fn heuristic(&self, c: &Costs) -> Heuristic {
        let per_block = 1.0
            / if self.cfg.sprint {
                c.sprint_speed
            } else {
                c.walk_speed
            };
        let per_up = (c.jump_up)
            .min(1.0 / c.climb_up_speed)
            .min(1.0 / c.swim_up_speed)
            .max(1.0);
        let fall = c.fall_ticks(24.0) / 24.0;
        let per_down = fall
            .min(1.0 / c.climb_down_speed)
            .min(1.0 / c.swim_down_speed);
        Heuristic {
            per_block,
            per_up,
            per_down,
        }
    }
}

fn start_spot<S: BlockSource + ?Sized>(
    g: &mut Grid<'_, S>,
    p: [i32; 3],
) -> Result<Spot, PlanError> {
    if g.unknown(p[0], p[1], p[2]) || g.unknown(p[0], p[1] - 1, p[2]) {
        return Err(PlanError::StartUnknown);
    }
    if let Some(s) = spot(g, p[0], p[1], p[2]) {
        return Ok(s);
    }
    // Mid-air or in a place the model does not stand in: start as if standing at the block's
    // bottom if the body fits there at all.
    let base = f64::from(p[1]);
    if clear(g, p[0], p[2], base, base + HEIGHT) {
        return Ok(Spot {
            kind: NodeKind::Ground,
            floor: base,
            floor_flags: 0,
            water: false,
            submerged: false,
            climb: false,
        });
    }
    Err(PlanError::StartBlocked)
}

const CARDINALS: [(i32, i32); 4] = [(0, 1), (1, 0), (0, -1), (-1, 0)];
const DIAGONALS: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

/// Extra ticks for the floor a step ends on.
fn floor_penalty(s: &Spot, per_block: f64) -> f64 {
    let mut p = 0.0;
    if s.floor_flags & flag::SLOW != 0 {
        p += per_block * 1.5;
    }
    if s.floor_flags & flag::SLIPPERY != 0 {
        p += 2.0;
    }
    if s.floor_flags & flag::BOUNCY != 0 {
        p += 12.0;
    }
    if s.submerged {
        p += 4.0;
    }
    p
}

fn dig_cost<S: BlockSource + ?Sized>(g: &mut Grid<'_, S>, d: &DigCost, p: [i32; 3]) -> Option<f64> {
    let c = g.class(p[0], p[1], p[2]);
    if !c.has(flag::SOLID) {
        return Some(0.0);
    }
    if !c.has(flag::DIGGABLE) || f64::from(c.hardness) > d.max_hardness {
        return None;
    }
    // Never open a liquid or bring down a falling block.
    if g.class(p[0], p[1] + 1, p[2])
        .has(flag::FALLING | flag::WATER | flag::LAVA)
    {
        return None;
    }
    for (dx, dz) in CARDINALS {
        if g.class(p[0] + dx, p[1], p[2] + dz)
            .has(flag::WATER | flag::LAVA)
        {
            return None;
        }
    }
    Some(d.base + d.ticks_per_hardness * f64::from(c.hardness))
}

fn push(out: &mut Vec<Edge>, to: [i32; 3], spot: Spot, kind: MoveKind, cost: f64) {
    out.push(Edge {
        to,
        spot,
        kind,
        cost,
        dig: [None; 3],
        place: None,
    });
}

/// All moves out of `p` (standing as `s`).
pub(crate) fn successors<S: BlockSource + ?Sized>(
    g: &mut Grid<'_, S>,
    cfg: &Config,
    version: Version,
    c: &Costs,
    p: [i32; 3],
    s: &Spot,
    out: &mut Vec<Edge>,
) {
    let per_block = 1.0
        / if cfg.sprint {
            c.sprint_speed
        } else {
            c.walk_speed
        };
    let swim_block = 1.0 / c.swim_speed;
    let [x, y, z] = p;
    let f0 = s.floor;

    // Vertical moves.
    match s.kind {
        NodeKind::Swim if cfg.swim => {
            if let Some(t) = spot(g, x, y + 1, z) {
                push(
                    out,
                    [x, y + 1, z],
                    t,
                    MoveKind::Swim,
                    1.0 / c.swim_up_speed + floor_penalty(&t, per_block),
                );
            }
            if let Some(t) = spot(g, x, y - 1, z) {
                push(
                    out,
                    [x, y - 1, z],
                    t,
                    MoveKind::Swim,
                    1.0 / c.swim_down_speed,
                );
            }
        }
        NodeKind::Ground if cfg.swim && s.water => {
            // Rise out of shallow water to swim at the surface.
            if let Some(t) = spot(g, x, y + 1, z)
                && t.kind == NodeKind::Swim
            {
                push(out, [x, y + 1, z], t, MoveKind::Swim, 1.0 / c.swim_up_speed);
            }
        }
        _ => {}
    }
    if cfg.climb && s.climb {
        if let Some(t) = spot(g, x, y + 1, z)
            && t.climb
            && (climb_wall(g, x, y, z).is_some() || version == Version::V1_16_5)
        {
            push(
                out,
                [x, y + 1, z],
                t,
                MoveKind::ClimbUp,
                1.0 / c.climb_up_speed,
            );
        }
        if s.kind == NodeKind::Climb
            && let Some(t) = spot(g, x, y - 1, z)
            && (t.climb || t.kind == NodeKind::Ground)
        {
            push(
                out,
                [x, y - 1, z],
                t,
                MoveKind::ClimbDown,
                1.0 / c.climb_down_speed,
            );
        }
    }

    let swimming = s.kind == NodeKind::Swim;
    let climbing = s.kind == NodeKind::Climb;
    if (swimming && !cfg.swim) || (climbing && !cfg.climb) {
        return;
    }

    // Cardinal moves.
    for (dx, dz) in CARDINALS {
        let (bx, bz) = (x + dx, z + dz);
        let mut found = false;
        // One block up: step-up, jump-up, swim out, climb off.
        if let Some(t) = spot(g, bx, y + 1, bz)
            && t.kind == NodeKind::Ground
        {
            let dh = t.floor - f0;
            if climbing {
                if dh <= 1.5 + 1e-6 && clear(g, x, z, f0 + HEIGHT, t.floor + HEIGHT) {
                    push(
                        out,
                        [bx, y + 1, bz],
                        t,
                        MoveKind::ClimbExit,
                        1.0 / c.climb_up_speed + per_block + floor_penalty(&t, per_block),
                    );
                    found = true;
                }
            } else if swimming || s.water {
                if dh <= 1.2 && clear(g, x, z, f0 + HEIGHT, t.floor + HEIGHT + 0.4) {
                    push(
                        out,
                        [bx, y + 1, bz],
                        t,
                        MoveKind::Swim,
                        swim_block + c.jump_up + floor_penalty(&t, per_block),
                    );
                    found = true;
                }
            } else if dh <= STEP + 1e-6 {
                if clear(g, x, z, f0 + HEIGHT, t.floor + HEIGHT) {
                    push(
                        out,
                        [bx, y + 1, bz],
                        t,
                        MoveKind::Walk,
                        per_block + 0.5 + floor_penalty(&t, per_block),
                    );
                    found = true;
                }
            } else if dh <= MAX_JUMP_UP && jump_room(g, x, z, bx, bz, f0, &t) {
                push(
                    out,
                    [bx, y + 1, bz],
                    t,
                    MoveKind::Ascend,
                    c.jump_up + floor_penalty(&t, per_block),
                );
                found = true;
            }
        }
        // Dig through to reach the step above (jump-up into a tunnel).
        if !found
            && !swimming
            && !climbing
            && let Some(d) = &cfg.dig
            && (f0 - f64::from(y)).abs() < 1e-6
        {
            found |= dig_up(g, d, c, p, (dx, dz), out);
        }
        if found {
            continue;
        }
        // Same level and drops.
        let lowest = y - (cfg.max_fall.max(cfg.max_water_fall).ceil() as i32) - 1;
        let mut yt = y;
        while yt >= lowest.max(1) {
            if let Some(t) = spot(g, bx, yt, bz) {
                let dh = t.floor - f0;
                let top = t.floor.max(f0) + HEIGHT;
                if dh > STEP + 1e-6 {
                    // A higher floor in the same block (soul sand, a slab on a step).
                    if !swimming
                        && !climbing
                        && dh <= MAX_JUMP_UP
                        && jump_room(g, x, z, bx, bz, f0, &t)
                    {
                        push(
                            out,
                            [bx, yt, bz],
                            t,
                            MoveKind::Ascend,
                            c.jump_up + floor_penalty(&t, per_block),
                        );
                    } else if (swimming || s.water) && dh <= 1.2 {
                        push(out, [bx, yt, bz], t, MoveKind::Swim, swim_block + c.jump_up);
                    }
                } else if dh >= -STEP - 1e-6 {
                    let kind = if swimming || t.kind == NodeKind::Swim || (s.water && t.water) {
                        MoveKind::Swim
                    } else {
                        MoveKind::Walk
                    };
                    let ok = match t.kind {
                        NodeKind::Swim => cfg.swim,
                        NodeKind::Climb => cfg.climb,
                        NodeKind::Ground => true,
                    };
                    if ok
                        && clear(g, x, z, f0 + HEIGHT, top)
                        && clear(g, bx, bz, t.floor + HEIGHT, top)
                    {
                        let cost = if kind == MoveKind::Swim {
                            swim_block
                        } else {
                            per_block
                        };
                        push(
                            out,
                            [bx, yt, bz],
                            t,
                            kind,
                            cost + floor_penalty(&t, per_block),
                        );
                    }
                } else if t.kind == NodeKind::Climb && cfg.climb && -dh <= 1.5 + 1e-6 {
                    // Onto a ladder from above: fall into it and hold on.
                    if clear(g, bx, bz, t.floor + HEIGHT, f0 + HEIGHT) {
                        push(
                            out,
                            [bx, yt, bz],
                            t,
                            MoveKind::ClimbDown,
                            per_block + (-dh) / c.climb_down_speed,
                        );
                    }
                } else if !climbing {
                    let drop = -dh;
                    let deep = t.kind == NodeKind::Swim && g.class(bx, yt - 1, bz).has(flag::WATER);
                    let into_water = t.kind == NodeKind::Swim || t.water;
                    let limit = if deep {
                        cfg.max_water_fall
                    } else {
                        cfg.max_fall
                    };
                    let landing_ok = t.floor_flags & (flag::FRAGILE | flag::BOUNCY) == 0;
                    if drop <= limit + 1e-6
                        && landing_ok
                        && (cfg.swim || !into_water)
                        && t.kind != NodeKind::Climb
                        && clear(g, bx, bz, t.floor + HEIGHT, f0 + HEIGHT)
                    {
                        push(
                            out,
                            [bx, yt, bz],
                            t,
                            MoveKind::Descend,
                            per_block + c.fall_ticks(drop) + 2.0 + floor_penalty(&t, per_block),
                        );
                    }
                }
                found = true;
                break;
            }
            // Keep falling only through cells the body passes.
            if !clear(g, bx, bz, f64::from(yt), f64::from(yt) + 1.0) {
                break;
            }
            yt -= 1;
        }
        if !found
            && !swimming
            && !climbing
            && let Some(d) = &cfg.dig
        {
            dig_through(g, d, per_block, p, f0, (dx, dz), out);
        }
        // Filling a one-deep hole competes with dropping into it.
        if !swimming
            && !climbing
            && let Some(pc) = cfg.place
            && spot(g, bx, y, bz).is_none()
        {
            fill_hole(g, pc, per_block, p, f0, (dx, dz), out);
        }
        if cfg.parkour && !swimming && !climbing && s.kind == NodeKind::Ground && !s.water {
            parkour(g, cfg, c, p, s, (dx, dz), out);
        }
    }

    // Diagonals (flat-ish only).
    if climbing {
        return;
    }
    for (dx, dz) in DIAGONALS {
        let (bx, bz) = (x + dx, z + dz);
        for yt in [y + 1, y, y - 1] {
            let Some(t) = spot(g, bx, yt, bz) else {
                continue;
            };
            let dh = t.floor - f0;
            if dh.abs() > STEP + 1e-6 {
                if yt == y {
                    break;
                }
                continue;
            }
            if (t.kind == NodeKind::Swim) != swimming {
                break;
            }
            let lo = t.floor.max(f0);
            let hi = lo + HEIGHT;
            // The box crosses both side columns: each needs a floor near the path's (a hole
            // beside the corner catches a box that strays from the diagonal).
            let floored = |g: &mut Grid<'_, S>, sx: i32, sz: i32| {
                [y, yt].iter().any(|&yy| {
                    spot(g, sx, yy, sz).is_some_and(|sp| {
                        sp.kind == NodeKind::Ground && sp.floor >= t.floor.min(f0) - STEP - 1e-6
                    })
                })
            };
            if !swimming && (!floored(g, x + dx, z) || !floored(g, x, z + dz)) {
                break;
            }
            if clear(g, x + dx, z, lo, hi)
                && clear(g, x, z + dz, lo, hi)
                && clear(g, x, z, f0 + HEIGHT, hi)
                && clear(g, bx, bz, t.floor + HEIGHT, hi)
            {
                let base = if swimming || (s.water && t.water) {
                    swim_block
                } else {
                    per_block
                };
                let kind = if swimming || (s.water && t.water) {
                    MoveKind::Swim
                } else {
                    MoveKind::Diagonal
                };
                push(
                    out,
                    [bx, yt, bz],
                    t,
                    kind,
                    base * std::f64::consts::SQRT_2
                        + floor_penalty(&t, per_block)
                        + if dh > 1e-6 { 0.5 } else { 0.0 },
                );
            }
            break;
        }
    }
}

/// Room for a standing jump from `f0` in column (x, z) onto `t` in column (bx, bz).
fn jump_room<S: BlockSource + ?Sized>(
    g: &mut Grid<'_, S>,
    x: i32,
    z: i32,
    bx: i32,
    bz: i32,
    f0: f64,
    t: &Spot,
) -> bool {
    let top = f0 + HEIGHT + JUMP_APEX;
    let ca = ceiling(g, x, z, f0 + HEIGHT, top);
    let cb = ceiling(g, bx, bz, t.floor + HEIGHT, top);
    let apex = ca.min(cb) - HEIGHT - f0;
    apex >= (t.floor - f0) + JUMP_MARGIN
}

/// Gap jumps from `p` in direction `d`.
fn parkour<S: BlockSource + ?Sized>(
    g: &mut Grid<'_, S>,
    cfg: &Config,
    c: &Costs,
    p: [i32; 3],
    s: &Spot,
    (dx, dz): (i32, i32),
    out: &mut Vec<Edge>,
) {
    let [x, y, z] = p;
    let f0 = s.floor;
    // Standing jumps need exact floors (no slabs or soul sand at take-off).
    if (f0 - f64::from(y)).abs() > 1e-6
        || s.floor_flags & (flag::SLOW | flag::SLIPPERY | flag::BOUNCY) != 0
    {
        return;
    }
    let arc_top = f0 + HEIGHT + JUMP_APEX;
    if !clear(g, x, z, f0 + HEIGHT, arc_top) {
        return;
    }
    // A 2-block gap needs a sprint run-up: a block to stand on behind the take-off.
    let runup = spot(g, x - dx, y, z - dz)
        .is_some_and(|r| r.kind == NodeKind::Ground && (r.floor - f0).abs() < 1e-6 && !r.water)
        && clear(g, x - dx, z - dz, f0 + HEIGHT, arc_top);
    let max_gap = if cfg.sprint && runup { 2 } else { 1 };
    for gap in 1..=max_gap {
        // The gap columns: no floor at the take-off level and room for the arc.
        let (gx, gz) = (x + dx * gap, z + dz * gap);
        let below = g.class(gx, y - 1, gz);
        if below.has(flag::SUPPORT) || below.has(flag::LAVA) || g.unknown(gx, y - 1, gz) {
            return;
        }
        if !clear(g, gx, gz, f0 - 1.0, arc_top) {
            return;
        }
        let (tx, tz) = (x + dx * (gap + 1), z + dz * (gap + 1));
        for yt in [y, y - 1] {
            let Some(t) = spot(g, tx, yt, tz) else {
                continue;
            };
            if t.kind != NodeKind::Ground || t.water {
                break;
            }
            let dh = t.floor - f0;
            if dh > 1e-6 || dh < -1.0 - 1e-6 {
                break;
            }
            if t.floor_flags & (flag::FRAGILE | flag::BOUNCY | flag::SLIPPERY) != 0 {
                break;
            }
            if !clear(g, tx, tz, t.floor + HEIGHT, arc_top) {
                break;
            }
            let cost =
                6.0 + c.jump_air + 1.0 / c.walk_speed + floor_penalty(&t, 1.0 / c.walk_speed);
            push(
                out,
                [tx, yt, tz],
                t,
                MoveKind::Parkour { gap: gap as u8 },
                cost,
            );
            break;
        }
        // A floor in the next column ends the run of gaps.
        if spot(g, tx, y, tz).is_some() || spot(g, tx, y - 1, tz).is_some() {
            return;
        }
    }
}

/// Tunnel: walk into the neighbour at the same level after digging its body blocks.
fn dig_through<S: BlockSource + ?Sized>(
    g: &mut Grid<'_, S>,
    d: &DigCost,
    per_block: f64,
    p: [i32; 3],
    f0: f64,
    (dx, dz): (i32, i32),
    out: &mut Vec<Edge>,
) -> bool {
    let [x, y, z] = p;
    if (f0 - f64::from(y)).abs() > 1e-6 {
        return false;
    }
    let (bx, bz) = (x + dx, z + dz);
    let fl = g.class(bx, y - 1, bz);
    if !(fl.has(flag::SUPPORT) && fl.top >= 1.0 - 1e-6 && fl.top_any <= 1.0 + 1e-6)
        || fl.has(flag::AVOID_FLOOR | flag::IRREGULAR)
    {
        return false;
    }
    let mut cost = per_block;
    let mut dig = [None; 3];
    let mut n = 0;
    for yy in [y + 1, y] {
        let c = g.class(bx, yy, bz);
        if g.unknown(bx, yy, bz) || c.has(flag::AVOID | flag::LAVA | flag::WATER) {
            return false;
        }
        if c.has(flag::SOLID) {
            match dig_cost(g, d, [bx, yy, bz]) {
                Some(k) => {
                    cost += k;
                    dig[n] = Some([bx, yy, bz]);
                    n += 1;
                }
                None => return false,
            }
        }
    }
    if n == 0 {
        return false;
    }
    let spot = Spot {
        kind: NodeKind::Ground,
        floor: f0,
        floor_flags: fl.flags,
        water: false,
        submerged: false,
        climb: false,
    };
    out.push(Edge {
        to: [bx, y, bz],
        spot,
        kind: MoveKind::Walk,
        cost,
        dig,
        place: None,
    });
    true
}

/// Jump up into the neighbour one block higher after digging what is in the way.
fn dig_up<S: BlockSource + ?Sized>(
    g: &mut Grid<'_, S>,
    d: &DigCost,
    c: &Costs,
    p: [i32; 3],
    (dx, dz): (i32, i32),
    out: &mut Vec<Edge>,
) -> bool {
    let [x, y, z] = p;
    let (bx, bz) = (x + dx, z + dz);
    let fl = g.class(bx, y, bz);
    if !(fl.has(flag::SUPPORT) && fl.top >= 1.0 - 1e-6 && fl.top_any <= 1.0 + 1e-6)
        || fl.has(flag::AVOID_FLOOR | flag::IRREGULAR)
    {
        return false;
    }
    // Needs: (x, y+2) above the origin (the jump), (bx, y+1), (bx, y+2) for the body.
    let cells = [[x, y + 2, z], [bx, y + 2, bz], [bx, y + 1, bz]];
    let mut cost = c.jump_up;
    let mut dig = [None; 3];
    let mut n = 0;
    for q in cells {
        let cl = g.class(q[0], q[1], q[2]);
        if g.unknown(q[0], q[1], q[2]) || cl.has(flag::AVOID | flag::LAVA | flag::WATER) {
            return false;
        }
        if cl.has(flag::SOLID) {
            match dig_cost(g, d, q) {
                Some(k) => {
                    cost += k;
                    dig[n] = Some(q);
                    n += 1;
                }
                None => return false,
            }
        }
    }
    if n == 0 {
        return false;
    }
    // Headroom above the dug space for the jump arc (y + 3 at the origin and target).
    let f0 = f64::from(y);
    let t_floor = f0 + 1.0;
    let arc = (f0 + HEIGHT + JUMP_APEX - (f0 + 3.0)) as f32;
    if g.class(x, y + 3, z).blocks_between(0.0, arc)
        || g.class(bx, y + 3, bz).blocks_between(0.0, arc)
    {
        return false;
    }
    let spot = Spot {
        kind: NodeKind::Ground,
        floor: t_floor,
        floor_flags: fl.flags,
        water: false,
        submerged: false,
        climb: false,
    };
    out.push(Edge {
        to: [bx, y + 1, bz],
        spot,
        kind: MoveKind::Ascend,
        cost,
        dig,
        place: None,
    });
    true
}

/// Walk over a one-deep hole after placing a block into it.
fn fill_hole<S: BlockSource + ?Sized>(
    g: &mut Grid<'_, S>,
    cost: f64,
    per_block: f64,
    p: [i32; 3],
    f0: f64,
    (dx, dz): (i32, i32),
    out: &mut Vec<Edge>,
) -> bool {
    let [x, y, z] = p;
    if (f0 - f64::from(y)).abs() > 1e-6 {
        return false;
    }
    let (bx, bz) = (x + dx, z + dz);
    let hole = g.class(bx, y - 1, bz);
    let under = g.class(bx, y - 2, bz);
    if !hole.has(flag::REPLACEABLE)
        || g.unknown(bx, y - 1, bz)
        || !(under.has(flag::SUPPORT) && under.top >= 1.0 - 1e-6 && under.top_any <= 1.0 + 1e-6)
        || under.has(flag::IRREGULAR | flag::AVOID_FLOOR)
    {
        return false;
    }
    if !clear(g, bx, bz, f0, f0 + HEIGHT) {
        return false;
    }
    let spot = Spot {
        kind: NodeKind::Ground,
        floor: f0,
        floor_flags: 0,
        water: false,
        submerged: false,
        climb: false,
    };
    out.push(Edge {
        to: [bx, y, bz],
        spot,
        kind: MoveKind::Walk,
        cost: per_block + cost,
        dig: [None; 3],
        place: Some(Place {
            pos: [bx, y - 1, bz],
            against: [bx, y - 2, bz],
        }),
    });
    true
}
