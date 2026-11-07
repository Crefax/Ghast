//! Fake worlds of both versions for the integration tests.

#![allow(dead_code)]

use ghast_pathfinder::{
    Config, ControllerConfig, Goal, MoveKind, Path, Pathfinder, PlanError, SimOutcome, SimReport,
    Version, simulate,
};

/// Blocks the tests build with.
#[derive(Clone, Copy, Debug)]
pub enum B {
    Air,
    Stone,
    Lava,
    Glass,
}

pub fn raw(v: Version, b: B) -> u16 {
    match v {
        Version::V1_8 => match b {
            B::Air => 0,
            B::Stone => 1 << 4,
            B::Lava => 11 << 4,
            B::Glass => 20 << 4,
        },
        _ => {
            use ghast_data::v1_16_5::Block;
            let s = match b {
                B::Air => Block::AIR.default_state(),
                B::Stone => Block::STONE.default_state(),
                B::Lava => Block::LAVA.default_state(),
                B::Glass => Block::GLASS.default_state(),
            };
            s.id()
        }
    }
}

/// A world of either version.
pub enum World {
    V18(ghast_physics::v1_8::MemoryWorld),
    V16(ghast_physics::v1_16_5::MemoryWorld),
}

pub const VERSIONS: [Version; 2] = [Version::V1_8, Version::V1_16_5];

impl World {
    /// Chunks -r..=r loaded, a stone floor at y = 63 over all of them.
    pub fn flat(v: Version, r: i32) -> World {
        let mut w = match v {
            Version::V1_8 => World::V18(ghast_physics::v1_8::memory_world()),
            _ => World::V16(ghast_physics::v1_16_5::memory_world()),
        };
        match &mut w {
            World::V18(m) => m.load_chunks(-r, -r, r, r),
            World::V16(m) => m.load_chunks(-r, -r, r, r),
        }
        w.fill(
            [-16 * r, 63, -16 * r],
            [16 * r + 15, 63, 16 * r + 15],
            B::Stone,
        );
        w
    }

    pub fn version(&self) -> Version {
        match self {
            World::V18(_) => Version::V1_8,
            World::V16(_) => Version::V1_16_5,
        }
    }

    pub fn fill(&mut self, a: [i32; 3], b: [i32; 3], blk: B) {
        let r = raw(self.version(), blk);
        match self {
            World::V18(m) => m.fill_raw(a, b, r),
            World::V16(m) => m.fill_raw(a, b, r),
        }
    }

    pub fn set(&mut self, p: [i32; 3], blk: B) {
        self.fill(p, p, blk);
    }

    pub fn try_plan(&self, cfg: Config, from: [i32; 3], goal: &Goal) -> Result<Path, PlanError> {
        let pf = Pathfinder::new(cfg);
        match self {
            World::V18(m) => pf.plan(Version::V1_8, m, from, goal),
            World::V16(m) => pf.plan(Version::V1_16_5, m, from, goal),
        }
    }

    pub fn plan(&self, cfg: Config, from: [i32; 3], goal: &Goal) -> Path {
        self.try_plan(cfg, from, goal)
            .unwrap_or_else(|e| panic!("{:?}: no plan {from:?} -> {goal:?}: {e}", self.version()))
    }

    pub fn simulate(&self, path: &Path) -> SimReport {
        let cfg = ControllerConfig::default();
        match self {
            World::V18(m) => simulate(path, m, &cfg),
            World::V16(m) => simulate(path, m, &cfg),
        }
    }
}

/// The moves of a path, for messages.
pub fn kinds(path: &Path) -> Vec<([i32; 3], MoveKind)> {
    path.steps.iter().map(|s| (s.pos, s.kind)).collect()
}

/// Plans to `goal` and simulates; asserts the run reached a goal block cleanly (no
/// unexpected collision, no fall damage).
pub fn check_goal(w: &World, cfg: Config, from: [i32; 3], goal: &Goal) -> (Path, SimReport) {
    check_goal_touching(w, cfg, from, goal, 0)
}

/// [`check_goal`] allowing up to `touches` ticks of horizontal collision (a box grazing the
/// wall of a narrow corridor's corner while turning at walking speed).
pub fn check_goal_touching(
    w: &World,
    cfg: Config,
    from: [i32; 3],
    goal: &Goal,
    touches: u32,
) -> (Path, SimReport) {
    let path = w.plan(cfg, from, goal);
    assert!(
        !path.partial,
        "{:?}: partial path {:?}",
        w.version(),
        kinds(&path)
    );
    let end = path.steps.last().unwrap().pos;
    assert!(goal.is_reached(end));
    let rep = w.simulate(&path);
    assert_eq!(
        rep.outcome,
        SimOutcome::Arrived,
        "{:?}: {:?} at step {} of {}, end {:?}\npath {:?}",
        w.version(),
        rep.outcome,
        rep.reached,
        path.steps.len() - 1,
        rep.end,
        kinds(&path),
    );
    let fb = [
        rep.end[0].floor() as i32,
        (rep.end[1] + 1e-4).floor() as i32,
        rep.end[2].floor() as i32,
    ];
    assert_eq!(fb, end, "{:?}: ended in {fb:?}", w.version());
    assert!(
        rep.collisions <= touches,
        "{:?}: {} ticks of unexpected collisions\npath {:?}",
        w.version(),
        rep.collisions,
        kinds(&path)
    );
    assert!(rep.max_fall <= 3.0 + 1e-3, "fall damage: {}", rep.max_fall);
    (path, rep)
}

/// [`check_goal`] to one block.
pub fn check(w: &World, cfg: Config, from: [i32; 3], to: [i32; 3]) -> (Path, SimReport) {
    check_goal(w, cfg, from, &Goal::Block(to))
}

/// Number of gap jumps of each width in a path.
pub fn parkour_gaps(path: &Path) -> Vec<u8> {
    path.steps
        .iter()
        .filter_map(|s| match s.kind {
            MoveKind::Parkour { gap } => Some(gap),
            _ => None,
        })
        .collect()
}
