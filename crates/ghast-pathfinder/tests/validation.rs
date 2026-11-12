//! Plans on small fake worlds, then runs each path through the vanilla-exact physics with the
//! controller (`simulate`): the player must reach the goal block without unexpected
//! collisions, hazards or fall damage. Every version.

use ghast_pathfinder::{
    Config, ControllerConfig, DigCost, Goal, MoveKind, Path, Pathfinder, SimOutcome, SimReport,
    Version, simulate,
};
use proptest::prelude::*;

/// Blocks the tests build with.
#[derive(Clone, Copy, Debug)]
enum B {
    Air,
    Stone,
    Dirt,
    Slab,
    SoulSand,
    Water,
    Lava,
    Glass,
    Fence,
    /// A ladder whose wall is on the +Z side.
    LadderSouthWall,
}

fn raw(v: Version, b: B) -> u16 {
    match v {
        Version::V1_8 | Version::V1_12 => match b {
            B::Air => 0,
            B::Stone => 1 << 4,
            B::Dirt => 3 << 4,
            B::Slab => 44 << 4,
            B::SoulSand => 88 << 4,
            B::Water => 9 << 4,
            B::Lava => 11 << 4,
            B::Glass => 20 << 4,
            B::Fence => 85 << 4,
            B::LadderSouthWall => 65 << 4 | 2,
        },
        Version::V26_1 => {
            use ghast_data::v26_1::{Block, BlockState};
            let s = match b {
                B::Air => Block::AIR.default_state(),
                B::Stone => Block::STONE.default_state(),
                B::Dirt => Block::DIRT.default_state(),
                B::Slab => BlockState::parse("smooth_stone_slab[type=bottom]").unwrap(),
                B::SoulSand => Block::SOUL_SAND.default_state(),
                B::Water => Block::WATER.default_state(),
                B::Lava => Block::LAVA.default_state(),
                B::Glass => Block::GLASS.default_state(),
                B::Fence => Block::OAK_FENCE.default_state(),
                B::LadderSouthWall => BlockState::parse("ladder[facing=north]").unwrap(),
            };
            s.id()
        }
        Version::V26_2 => {
            use ghast_data::v26_2::{Block, BlockState};
            let s = match b {
                B::Air => Block::AIR.default_state(),
                B::Stone => Block::STONE.default_state(),
                B::Dirt => Block::DIRT.default_state(),
                B::Slab => BlockState::parse("smooth_stone_slab[type=bottom]").unwrap(),
                B::SoulSand => Block::SOUL_SAND.default_state(),
                B::Water => Block::WATER.default_state(),
                B::Lava => Block::LAVA.default_state(),
                B::Glass => Block::GLASS.default_state(),
                B::Fence => Block::OAK_FENCE.default_state(),
                B::LadderSouthWall => BlockState::parse("ladder[facing=north]").unwrap(),
            };
            s.id()
        }
        _ => {
            use ghast_data::v1_16_5::{Block, Property};
            let s = match b {
                B::Air => Block::AIR.default_state(),
                B::Stone => Block::STONE.default_state(),
                B::Dirt => Block::DIRT.default_state(),
                B::Slab => Block::SMOOTH_STONE_SLAB.default_state(),
                B::SoulSand => Block::SOUL_SAND.default_state(),
                B::Water => Block::WATER.default_state(),
                B::Lava => Block::LAVA.default_state(),
                B::Glass => Block::GLASS.default_state(),
                B::Fence => Block::OAK_FENCE.default_state(),
                B::LadderSouthWall => Block::LADDER
                    .state_with(&[(Property::Facing, "north")])
                    .unwrap(),
            };
            s.id()
        }
    }
}

/// A world of either version.
enum World {
    V18(ghast_physics::v1_8::MemoryWorld),
    V12(ghast_physics::v1_12::MemoryWorld),
    V16(ghast_physics::v1_16_5::MemoryWorld),
    V26(ghast_physics::modern::MemoryWorld26),
    V262(ghast_physics::modern::MemoryWorld262),
}

impl World {
    /// Chunks -4..=4 loaded (x, z in -64..=79), a stone floor at y = 63 over all of it.
    fn flat(v: Version) -> World {
        let mut w = match v {
            Version::V1_8 => World::V18(ghast_physics::v1_8::memory_world()),
            Version::V1_12 => World::V12(ghast_physics::v1_12::memory_world()),
            Version::V26_1 => World::V26(ghast_physics::modern::memory_world_26_1()),
            Version::V26_2 => World::V262(ghast_physics::modern::memory_world_26_2()),
            _ => World::V16(ghast_physics::v1_16_5::memory_world()),
        };
        match &mut w {
            World::V18(m) => m.load_chunks(-4, -4, 4, 4),
            World::V12(m) => m.load_chunks(-4, -4, 4, 4),
            World::V16(m) => m.load_chunks(-4, -4, 4, 4),
            World::V26(m) => m.load_chunks(-4, -4, 4, 4),
            World::V262(m) => m.load_chunks(-4, -4, 4, 4),
        }
        w.fill([-64, 63, -64], [79, 63, 79], B::Stone);
        w
    }

    fn version(&self) -> Version {
        match self {
            World::V18(_) => Version::V1_8,
            World::V12(_) => Version::V1_12,
            World::V16(_) => Version::V1_16_5,
            World::V26(_) => Version::V26_1,
            World::V262(_) => Version::V26_2,
        }
    }

    fn fill(&mut self, a: [i32; 3], b: [i32; 3], blk: B) {
        let r = raw(self.version(), blk);
        match self {
            World::V18(m) => m.fill_raw(a, b, r),
            World::V12(m) => m.fill_raw(a, b, r),
            World::V16(m) => m.fill_raw(a, b, r),
            World::V26(m) => m.fill_raw(a, b, r),
            World::V262(m) => m.fill_raw(a, b, r),
        }
    }

    fn set(&mut self, p: [i32; 3], blk: B) {
        self.fill(p, p, blk);
    }

    fn plan(&self, cfg: Config, from: [i32; 3], goal: &Goal) -> Path {
        let pf = Pathfinder::new(cfg);
        let r = match self {
            World::V18(m) => pf.plan(Version::V1_8, m, from, goal),
            World::V12(m) => pf.plan(Version::V1_12, m, from, goal),
            World::V16(m) => pf.plan(Version::V1_16_5, m, from, goal),
            World::V26(m) => pf.plan(Version::V26_1, m, from, goal),
            World::V262(m) => pf.plan(Version::V26_2, m, from, goal),
        };
        r.unwrap_or_else(|e| panic!("{:?}: no plan {from:?} -> {goal:?}: {e}", self.version()))
    }

    fn simulate(&self, path: &Path) -> SimReport {
        let cfg = ControllerConfig::default();
        match self {
            World::V18(m) => simulate(path, m, &cfg),
            World::V12(m) => simulate(path, m, &cfg),
            World::V16(m) => simulate(path, m, &cfg),
            World::V26(m) => simulate(path, m, &cfg),
            World::V262(m) => simulate(path, m, &cfg),
        }
    }
}

const VERSIONS: [Version; 5] = [
    Version::V1_8,
    Version::V1_12,
    Version::V1_16_5,
    Version::V26_1,
    Version::V26_2,
];

/// Plans and simulates; asserts the run reached the goal block cleanly.
fn check(w: &World, cfg: Config, from: [i32; 3], to: [i32; 3]) -> (Path, SimReport) {
    let path = w.plan(cfg, from, &Goal::Block(to));
    assert!(!path.partial, "{:?}: partial path {path:#?}", w.version());
    assert_eq!(path.steps.last().unwrap().pos, to);
    let rep = w.simulate(&path);
    let kinds: Vec<_> = path.steps.iter().map(|s| (s.pos, s.kind)).collect();
    assert_eq!(
        rep.outcome,
        SimOutcome::Arrived,
        "{:?}: {:?} at step {} of {}, end {:?}\npath {kinds:?}",
        w.version(),
        rep.outcome,
        rep.reached,
        path.steps.len() - 1,
        rep.end,
    );
    let end = [
        rep.end[0].floor() as i32,
        (rep.end[1] + 1e-4).floor() as i32,
        rep.end[2].floor() as i32,
    ];
    assert_eq!(end, to, "{:?}: ended in {end:?}", w.version());
    assert_eq!(
        rep.collisions,
        0,
        "{:?}: unexpected collisions\npath {kinds:?}",
        w.version()
    );
    assert!(rep.max_fall <= 3.0 + 1e-3, "fall damage: {}", rep.max_fall);
    (path, rep)
}

fn has(path: &Path, f: impl Fn(MoveKind) -> bool) -> bool {
    path.steps.iter().any(|s| f(s.kind))
}

#[test]
fn flat_straight_and_diagonal() {
    for v in VERSIONS {
        let w = World::flat(v);
        let (p, r) = check(&w, Config::default(), [0, 64, 0], [30, 64, 0]);
        // Sprinting a straight line: about 30 / 0.28 ticks plus acceleration.
        assert!(r.ticks < 130, "{v:?}: {} ticks", r.ticks);
        assert!(p.steps.len() == 31);
        let (p, _) = check(&w, Config::default(), [0, 64, 0], [17, 64, -23]);
        assert!(has(&p, |k| k == MoveKind::Diagonal));
        check(
            &w,
            Config::default().with_sprint(false),
            [-5, 64, 3],
            [12, 64, 9],
        );
    }
}

#[test]
fn walls_and_a_maze() {
    for v in VERSIONS {
        let mut w = World::flat(v);
        // A wall with one opening at z = 6.
        w.fill([5, 64, -20], [5, 66, 20], B::Stone);
        w.fill([5, 64, 6], [5, 65, 6], B::Air);
        let (p, _) = check(&w, Config::default(), [0, 64, 0], [10, 64, 0]);
        assert!(p.steps.iter().any(|s| s.pos == [5, 64, 6]));
        // Corridors: glass walls one block apart.
        let mut w = World::flat(v);
        for z in [-1, 1] {
            w.fill([0, 64, z], [12, 65, z], B::Glass);
        }
        w.fill([12, 64, -1], [12, 65, 1], B::Glass);
        w.fill([11, 64, 1], [11, 65, 1], B::Air);
        w.fill([10, 64, 2], [12, 65, 2], B::Glass);
        w.fill([10, 64, 3], [10, 65, 6], B::Glass);
        w.fill([12, 64, 3], [12, 65, 6], B::Glass);
        check(&w, Config::default(), [1, 64, 0], [11, 64, 5]);
    }
}

#[test]
fn steps_up_and_down() {
    for v in VERSIONS {
        let mut w = World::flat(v);
        // A staircase up: one block per two steps forward, then a plateau, then a drop.
        for i in 0..5 {
            w.fill([2 + 2 * i, 64, -2], [40, 64 + i, 2], B::Stone);
        }
        let (p, _) = check(&w, Config::default(), [0, 64, 0], [14, 69, 0]);
        assert!(has(&p, |k| k == MoveKind::Ascend));
        // Back down by drops of up to 3.
        let mut w = World::flat(v);
        w.fill([-3, 64, -3], [3, 66, 3], B::Stone);
        w.fill([4, 64, -3], [4, 65, 3], B::Stone);
        let (p, _) = check(&w, Config::default(), [0, 67, 0], [9, 64, 0]);
        assert!(has(&p, |k| k == MoveKind::Descend));
        // A 3-block drop straight down an edge.
        let mut w = World::flat(v);
        w.fill([-3, 64, -3], [0, 66, 3], B::Stone);
        let (p, _) = check(&w, Config::default(), [-1, 67, 0], [2, 64, 0]);
        assert!(has(&p, |k| k == MoveKind::Descend));
    }
}

#[test]
fn slabs_soul_sand_and_fences() {
    for v in VERSIONS {
        let mut w = World::flat(v);
        // Slab steps up and down, soul sand in between.
        w.fill([3, 64, -2], [3, 64, 2], B::Slab);
        w.fill([4, 64, -2], [4, 64, 2], B::Stone);
        w.fill([5, 64, -2], [5, 64, 2], B::Slab);
        w.fill([6, 63, -40], [9, 63, 40], B::SoulSand);
        w.fill([10, 64, -2], [10, 64, 2], B::Slab);
        let (p, _) = check(&w, Config::default(), [0, 64, 0], [13, 64, 0]);
        assert!(p.steps.iter().any(|s| (s.feet_y - 64.5).abs() < 1e-9));
        assert!(p.steps.iter().any(|s| (s.feet_y - 63.875).abs() < 1e-9));
        // A fence row is not jumped (1.5 high): the path goes around it.
        let mut w = World::flat(v);
        w.fill([4, 64, -6], [4, 64, 6], B::Fence);
        let (p, _) = check(&w, Config::default(), [0, 64, 0], [8, 64, 0]);
        assert!(p.steps.iter().all(|s| s.pos[0] != 4 || s.pos[2].abs() > 6));
    }
}

#[test]
fn gap_jumps() {
    for v in VERSIONS {
        // A 1-block and a 2-block gap in a bridge over lava (no way around).
        let mut w = World::flat(v);
        w.fill([-64, 63, -64], [79, 63, 79], B::Air);
        w.fill([-40, 60, -40], [40, 60, 40], B::Lava);
        w.fill([0, 63, 0], [20, 63, 0], B::Stone);
        w.set([5, 63, 0], B::Air);
        w.fill([11, 63, 0], [12, 63, 0], B::Air);
        let (p, _) = check(&w, Config::default(), [0, 64, 0], [20, 64, 0]);
        assert!(has(&p, |k| k == MoveKind::Parkour { gap: 1 }));
        assert!(has(&p, |k| k == MoveKind::Parkour { gap: 2 }));
        // Without parkour there is no path.
        let pf = Pathfinder::new(Config::default().with_parkour(false));
        let r = match &w {
            World::V18(m) => pf.plan(v, m, [0, 64, 0], &Goal::Block([20, 64, 0])),
            World::V12(m) => pf.plan(v, m, [0, 64, 0], &Goal::Block([20, 64, 0])),
            World::V16(m) => pf.plan(v, m, [0, 64, 0], &Goal::Block([20, 64, 0])),
            World::V26(m) => pf.plan(v, m, [0, 64, 0], &Goal::Block([20, 64, 0])),
            World::V262(m) => pf.plan(v, m, [0, 64, 0], &Goal::Block([20, 64, 0])),
        };
        assert!(r.is_err() || r.unwrap().partial);
        // A gap jump down one block.
        let mut w = World::flat(v);
        w.fill([-64, 63, -64], [79, 63, 79], B::Air);
        w.fill([-40, 60, -40], [40, 60, 40], B::Lava);
        w.fill([0, 63, 0], [4, 63, 0], B::Stone);
        w.fill([6, 62, 0], [10, 62, 0], B::Stone);
        let (p, _) = check(&w, Config::default(), [0, 64, 0], [10, 63, 0]);
        assert!(has(&p, |k| matches!(k, MoveKind::Parkour { .. })));
    }
}

#[test]
fn water() {
    for v in VERSIONS {
        // A pool 3 deep across the way, walls on the sides.
        let mut w = World::flat(v);
        w.fill([3, 60, -64], [8, 63, 79], B::Water);
        w.fill([3, 59, -64], [8, 59, 79], B::Stone);
        let (p, _) = check(&w, Config::default(), [0, 64, 0], [12, 64, 0]);
        assert!(has(&p, |k| k == MoveKind::Swim), "{v:?}");
        // Without swimming the pool cannot be crossed (a 6-wide gap).
        let pf = Pathfinder::new(Config::default().with_swim(false));
        let r = match &w {
            World::V18(m) => pf.plan(v, m, [0, 64, 0], &Goal::Block([12, 64, 0])),
            World::V12(m) => pf.plan(v, m, [0, 64, 0], &Goal::Block([12, 64, 0])),
            World::V16(m) => pf.plan(v, m, [0, 64, 0], &Goal::Block([12, 64, 0])),
            World::V26(m) => pf.plan(v, m, [0, 64, 0], &Goal::Block([12, 64, 0])),
            World::V262(m) => pf.plan(v, m, [0, 64, 0], &Goal::Block([12, 64, 0])),
        };
        assert!(r.is_err() || r.unwrap().partial);
    }
}

#[test]
fn ladders() {
    for v in VERSIONS {
        // A 6-high cliff with a ladder on its face.
        let mut w = World::flat(v);
        w.fill([-10, 64, 3], [10, 69, 10], B::Stone);
        w.fill([0, 64, 2], [0, 69, 2], B::LadderSouthWall);
        let (p, _) = check(&w, Config::default(), [0, 64, -3], [0, 70, 6]);
        assert!(has(&p, |k| k == MoveKind::ClimbUp), "{v:?}");
        // And down again.
        let (p, _) = check(&w, Config::default(), [0, 70, 6], [0, 64, -3]);
        assert!(has(&p, |k| k == MoveKind::ClimbDown), "{v:?}");
    }
}

#[test]
fn digging_and_placing() {
    for v in VERSIONS {
        // A closed dirt wall: only a tunnel gets through.
        let mut w = World::flat(v);
        w.fill([5, 64, -64], [5, 70, 79], B::Dirt);
        let dig = Config::default().with_dig(Some(DigCost::default()));
        let (p, _) = check(&w, dig, [0, 64, 0], [9, 64, 0]);
        assert!(p.steps.iter().any(|s| !s.dig.is_empty()), "{v:?}");
        // A trench one deep and three wide: cheaper to fill than to climb through.
        let mut w = World::flat(v);
        w.fill([5, 63, -64], [7, 63, 79], B::Air);
        w.fill([5, 62, -64], [7, 62, 79], B::Stone);
        let place = Config::default().with_place(Some(1.0)).with_parkour(false);
        let path = w.plan(place, [0, 64, 0], &Goal::Block([10, 64, 0]));
        assert!(!path.partial);
        assert!(path.steps.iter().any(|s| s.place.is_some()), "{v:?}");
        assert_eq!(w.simulate(&path).outcome, SimOutcome::Arrived, "{v:?}");
    }
}

#[test]
fn unloaded_terrain_is_not_air() {
    for v in VERSIONS {
        let w = World::flat(v);
        // x = 80 is beyond the loaded chunks: the path stops at the loaded edge.
        let path = w.plan(Config::default(), [0, 64, 0], &Goal::Block([80, 64, 0]));
        assert!(path.partial);
        let end = path.steps.last().unwrap().pos;
        assert!(end[0] <= 79 && end[0] >= 70, "{end:?}");
        assert_eq!(w.simulate(&path).outcome, SimOutcome::Arrived);
    }
}

#[test]
fn goals() {
    for v in VERSIONS {
        let w = World::flat(v);
        let pf = Pathfinder::new(Config::default());
        let plan = |g: &Goal| match &w {
            World::V18(m) => pf.plan(v, m, [0, 64, 0], g).unwrap(),
            World::V12(m) => pf.plan(v, m, [0, 64, 0], g).unwrap(),
            World::V16(m) => pf.plan(v, m, [0, 64, 0], g).unwrap(),
            World::V26(m) => pf.plan(v, m, [0, 64, 0], g).unwrap(),
            World::V262(m) => pf.plan(v, m, [0, 64, 0], g).unwrap(),
        };
        let p = plan(&Goal::Near {
            pos: [10, 64, 10],
            radius: 3.0,
        });
        let e = p.steps.last().unwrap().pos;
        assert!(
            Goal::Near {
                pos: [10, 64, 10],
                radius: 3.0
            }
            .is_reached(e)
        );
        let p = plan(&Goal::Xz { x: -7, z: 4 });
        assert_eq!(p.steps.last().unwrap().pos, [-7, 64, 4]);
        let p = plan(&Goal::AnyOf(vec![
            Goal::Block([20, 64, 0]),
            Goal::Block([-3, 64, 0]),
        ]));
        assert_eq!(p.steps.last().unwrap().pos, [-3, 64, 0]);
        let p = plan(&Goal::Block([0, 64, 0]));
        assert_eq!(p.steps.len(), 1);
        assert_eq!(w.simulate(&p).outcome, SimOutcome::Arrived);
    }
}

#[test]
fn verified_plans_repair_failed_moves() {
    for v in VERSIONS {
        let mut w = World::flat(v);
        w.fill([5, 64, -20], [5, 64, 20], B::Stone);
        w.fill([7, 63, -20], [7, 63, 20], B::Air);
        let pf = Pathfinder::new(Config::default());
        let (path, sim) = match &w {
            World::V18(m) => pf.plan_verified(v, m, [0, 64, 0], &Goal::Block([10, 64, 0]), 3),
            World::V12(m) => pf.plan_verified(v, m, [0, 64, 0], &Goal::Block([10, 64, 0]), 3),
            World::V16(m) => pf.plan_verified(v, m, [0, 64, 0], &Goal::Block([10, 64, 0]), 3),
            World::V26(m) => pf.plan_verified(v, m, [0, 64, 0], &Goal::Block([10, 64, 0]), 3),
            World::V262(m) => pf.plan_verified(v, m, [0, 64, 0], &Goal::Block([10, 64, 0]), 3),
        }
        .unwrap();
        assert_eq!(sim.outcome, SimOutcome::Arrived);
        assert!(!path.partial);
    }
}

/// Random rolling terrain (heights 0..=3 over the floor, some slabs, soul sand and holes):
/// every plan the planner finds must run in the physics.
fn terrain(v: Version, seed: u64) -> World {
    let mut w = World::flat(v);
    let mut s = seed | 1;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for x in -12..=12 {
        for z in -12..=12 {
            let r = rnd();
            let h = (r % 4) as i32;
            if h > 0 {
                w.fill([x, 64, z], [x, 63 + h, z], B::Stone);
            }
            match (r >> 8) % 16 {
                0 => w.set([x, 64 + h, z], B::Slab),
                1 => w.set([x, 63 + h, z], B::SoulSand),
                2 if h == 0 => w.set([x, 63, z], B::Air),
                _ => {}
            }
        }
    }
    // Keep the start and the goal clear.
    for p in [[-10, 64, -10], [10, 64, 10]] {
        w.fill([p[0], 63, p[2]], [p[0], 63, p[2]], B::Stone);
        w.fill(p, [p[0], 70, p[2]], B::Air);
    }
    w
}

fn run_terrain(seed: u64, v: Version) -> Result<(), String> {
    let w = terrain(v, seed);
    let pf = Pathfinder::new(Config::default());
    let r = match &w {
        World::V18(m) => pf.plan(v, m, [-10, 64, -10], &Goal::Block([10, 64, 10])),
        World::V12(m) => pf.plan(v, m, [-10, 64, -10], &Goal::Block([10, 64, 10])),
        World::V16(m) => pf.plan(v, m, [-10, 64, -10], &Goal::Block([10, 64, 10])),
        World::V26(m) => pf.plan(v, m, [-10, 64, -10], &Goal::Block([10, 64, 10])),
        World::V262(m) => pf.plan(v, m, [-10, 64, -10], &Goal::Block([10, 64, 10])),
    };
    let Ok(path) = r else { return Ok(()) };
    let rep = w.simulate(&path);
    let kinds: Vec<_> = path
        .steps
        .iter()
        .map(|s| (s.pos, s.feet_y, s.kind))
        .collect();
    if rep.outcome != SimOutcome::Arrived || rep.max_fall > 3.0 + 1e-3 {
        return Err(format!(
            "{v:?} seed {seed}: {:?} at step {} end {:?} max fall {}\n{kinds:?}",
            rep.outcome, rep.reached, rep.end, rep.max_fall
        ));
    }
    Ok(())
}

/// A fixed set of random terrains (deterministic, for CI).
#[test]
fn random_terrain_paths_run_in_the_physics() {
    let mut fails = Vec::new();
    for i in 0..48u64 {
        let seed = i.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x5eed;
        for v in VERSIONS {
            if let Err(e) = run_terrain(seed, v) {
                fails.push(e);
            }
        }
    }
    assert!(
        fails.is_empty(),
        "{} of {} failed:\n{}",
        fails.len(),
        48 * VERSIONS.len(),
        fails.join("\n")
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    /// Fuzzing: `cargo test --release -p ghast-pathfinder -- --ignored`. About 1 in 400
    /// random terrains still fails (see the crate README).
    #[test]
    #[ignore = "fuzzing; run on demand"]
    fn fuzz_random_terrain(seed in any::<u64>(), vi in 0usize..4) {
        let v = VERSIONS[vi];
        if let Err(e) = run_terrain(seed, v) {
            prop_assert!(false, "{}", e);
        }
    }
}

/// 26.1: negative heights, 26.x blocks classified for standing, passing and danger.
#[test]
fn modern_blocks_and_negative_heights_26_1() {
    use ghast_data::v26_1::BlockState;
    let id = |s: &str| BlockState::parse(s).unwrap().id();
    let mut m = ghast_physics::modern::memory_world_26_1();
    m.load_chunks(-2, -2, 2, 2);
    // Deepslate floor at y = -61 (feet at -60).
    m.fill_raw([-32, -61, -32], [31, -61, 31], id("deepslate"));
    // Powder snow across the way (a player sinks in it) with one gap at z = 5.
    m.fill_raw([3, -60, -32], [3, -59, 31], id("powder_snow"));
    m.fill_raw([3, -60, 5], [3, -59, 5], 0);
    // Mud makes the floor of the next strip (14/16 high), copper grates the one after.
    m.fill_raw([6, -61, -32], [6, -61, 31], id("mud"));
    m.fill_raw([7, -61, -32], [7, -61, 31], id("copper_grate"));
    // A row of pointed dripstone and magma on the floor except at z = 0.
    m.fill_raw([9, -60, -32], [9, -60, 31], id("pointed_dripstone"));
    m.fill_raw([9, -61, -32], [9, -61, 31], id("magma_block"));
    m.set_raw(9, -60, 0, 0);
    m.set_raw(9, -61, 0, id("tuff"));
    let pf = Pathfinder::new(Config::default());
    let path = pf
        .plan(Version::V26_1, &m, [0, -60, 0], &Goal::Block([12, -60, 0]))
        .unwrap();
    assert!(!path.partial);
    for s in &path.steps {
        if s.pos[0] == 3 {
            assert_eq!(s.pos[2], 5, "through powder snow: {:?}", s.pos);
        }
        if s.pos[0] == 9 {
            assert_eq!(s.pos[2], 0, "over dripstone / magma: {:?}", s.pos);
        }
        if s.pos[0] == 6 {
            assert!(
                (s.feet_y - (-61.0 + 0.875)).abs() < 1e-9,
                "mud floor {}",
                s.feet_y
            );
        }
    }
    let rep = simulate(&path, &m, &ControllerConfig::default());
    assert_eq!(rep.outcome, SimOutcome::Arrived, "{rep:?}");
    // Below the build height is unknown, never walked into: a shaft down to the bottom row.
    m.fill_raw([-5, -64, -5], [-5, -61, -5], 0);
    let r = pf.plan(Version::V26_1, &m, [0, -60, 0], &Goal::Block([-5, -64, -5]));
    assert!(r.is_err() || r.unwrap().partial);
}
