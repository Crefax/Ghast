//! Digging down, doors, bridging and pillaring: planned on small fake worlds of every
//! version, then run through the vanilla-exact physics with the controller (`simulate`, which
//! carries out the digs, places and door clicks in its own copy of the world).

use ghast_pathfinder::{
    Budget, Config, ControllerConfig, DigCost, Goal, MoveKind, Path, Pathfinder, PlanError,
    SimOutcome, SimReport, Version, simulate,
};

/// The raw state of a block given in the modern notation (`oak_door[facing=east,half=lower]`).
fn raw(v: Version, spec: &str) -> u16 {
    let (name, props) = match spec.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']')),
        None => (spec, ""),
    };
    let props: Vec<(&str, &str)> = props
        .split(',')
        .filter(|p| !p.is_empty())
        .map(|p| p.split_once('=').expect("key=value"))
        .collect();
    match v {
        Version::V1_8 | Version::V1_12 => raw_legacy(name, &props),
        Version::V1_16_5 => {
            use ghast_data::v1_16_5::{Block, Property};
            let b = Block::from_name(name).unwrap_or_else(|| panic!("{name}"));
            let ps: Vec<(Property, &str)> = props
                .iter()
                .map(|(k, v)| (Property::from_name(k).unwrap_or_else(|| panic!("{k}")), *v))
                .collect();
            if ps.is_empty() {
                b.default_state().id()
            } else {
                b.state_with(&ps).unwrap_or_else(|| panic!("{spec}")).id()
            }
        }
        Version::V26_1 => ghast_data::v26_1::BlockState::parse(spec)
            .unwrap_or_else(|| panic!("{spec}"))
            .id(),
        Version::V26_2 => ghast_data::v26_2::BlockState::parse(spec)
            .unwrap_or_else(|| panic!("{spec}"))
            .id(),
        _ => unreachable!(),
    }
}

/// 1.8.9 / 1.12.2 `id << 4 | meta` of the few blocks the tests use.
fn raw_legacy(name: &str, props: &[(&str, &str)]) -> u16 {
    let prop = |k: &str| props.iter().find(|(p, _)| *p == k).map(|(_, v)| *v);
    let id: u16 = match name {
        "air" => 0,
        "stone" => 1,
        "dirt" => 3,
        "cobblestone" => 4,
        "water" => 9,
        "lava" => 11,
        "oak_log" => 17,
        "oak_leaves" => 18,
        "glass" => 20,
        "oak_door" => 64,
        "iron_door" => 71,
        "oak_fence_gate" => 107,
        _ => panic!("no 1.8 id for {name}"),
    };
    let meta: u16 = match name {
        "oak_door" | "iron_door" => {
            if prop("half") == Some("upper") {
                8 | u16::from(prop("hinge") == Some("right"))
            } else {
                let f = match prop("facing").unwrap_or("east") {
                    "east" => 0,
                    "south" => 1,
                    "west" => 2,
                    _ => 3,
                };
                f | if prop("open") == Some("true") { 4 } else { 0 }
            }
        }
        "oak_fence_gate" => {
            let f = match prop("facing").unwrap_or("south") {
                "south" => 0,
                "west" => 1,
                "north" => 2,
                _ => 3,
            };
            f | if prop("open") == Some("true") { 4 } else { 0 }
        }
        _ => 0,
    };
    id << 4 | meta
}

/// A world of one version.
enum World {
    V18(ghast_physics::v1_8::MemoryWorld),
    V12(ghast_physics::v1_12::MemoryWorld),
    V16(ghast_physics::v1_16_5::MemoryWorld),
    V26(ghast_physics::modern::MemoryWorld26),
    V262(ghast_physics::modern::MemoryWorld262),
}

const VERSIONS: [Version; 5] = [
    Version::V1_8,
    Version::V1_12,
    Version::V1_16_5,
    Version::V26_1,
    Version::V26_2,
];

impl World {
    /// Chunks -4..=4 loaded, a stone floor at y = 63 (feet at 64).
    fn flat(v: Version) -> World {
        let mut w = Self::empty(v);
        w.fill([-64, 63, -64], [79, 63, 79], "stone");
        w
    }

    /// Chunks -4..=4 loaded, nothing in them (air: a void below any floor the test builds).
    fn empty(v: Version) -> World {
        let mut w = match v {
            Version::V1_8 => World::V18(ghast_physics::v1_8::memory_world()),
            Version::V1_12 => World::V12(ghast_physics::v1_12::memory_world()),
            Version::V1_16_5 => World::V16(ghast_physics::v1_16_5::memory_world()),
            Version::V26_1 => World::V26(ghast_physics::modern::memory_world_26_1()),
            Version::V26_2 => World::V262(ghast_physics::modern::memory_world_26_2()),
            _ => unreachable!(),
        };
        match &mut w {
            World::V18(m) => m.load_chunks(-4, -4, 4, 4),
            World::V12(m) => m.load_chunks(-4, -4, 4, 4),
            World::V16(m) => m.load_chunks(-4, -4, 4, 4),
            World::V26(m) => m.load_chunks(-4, -4, 4, 4),
            World::V262(m) => m.load_chunks(-4, -4, 4, 4),
        }
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

    fn fill(&mut self, a: [i32; 3], b: [i32; 3], spec: &str) {
        let r = raw(self.version(), spec);
        match self {
            World::V18(m) => m.fill_raw(a, b, r),
            World::V12(m) => m.fill_raw(a, b, r),
            World::V16(m) => m.fill_raw(a, b, r),
            World::V26(m) => m.fill_raw(a, b, r),
            World::V262(m) => m.fill_raw(a, b, r),
        }
    }

    fn set(&mut self, p: [i32; 3], spec: &str) {
        self.fill(p, p, spec);
    }

    fn plan(&self, cfg: Config, from: [i32; 3], goal: &Goal) -> Result<Path, PlanError> {
        let pf = Pathfinder::new(cfg);
        let v = self.version();
        match self {
            World::V18(m) => pf.plan(v, m, from, goal),
            World::V12(m) => pf.plan(v, m, from, goal),
            World::V16(m) => pf.plan(v, m, from, goal),
            World::V26(m) => pf.plan(v, m, from, goal),
            World::V262(m) => pf.plan(v, m, from, goal),
        }
    }

    /// Plans and also returns the nodes the search expanded.
    fn plan_counted(
        &self,
        cfg: Config,
        from: [i32; 3],
        goal: &Goal,
    ) -> (Result<Path, PlanError>, usize) {
        let pf = Pathfinder::new(cfg);
        let v = self.version();
        let mut s = pf.search(v, from, goal.clone(), std::collections::HashSet::new());
        let r = match self {
            World::V18(m) => s.step(m, Budget::UNLIMITED),
            World::V12(m) => s.step(m, Budget::UNLIMITED),
            World::V16(m) => s.step(m, Budget::UNLIMITED),
            World::V26(m) => s.step(m, Budget::UNLIMITED),
            World::V262(m) => s.step(m, Budget::UNLIMITED),
        }
        .expect("an unlimited step finishes");
        (r, s.expanded())
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

/// Plans to `goal` and simulates: the run must arrive in a block that reaches the goal,
/// without fall damage. Returns the path.
fn check(w: &World, cfg: Config, from: [i32; 3], goal: &Goal) -> (Path, SimReport) {
    let v = w.version();
    let path = w
        .plan(cfg, from, goal)
        .unwrap_or_else(|e| panic!("{v:?}: no plan {from:?} -> {goal:?}: {e}"));
    let kinds: Vec<_> = path.steps.iter().map(|s| (s.pos, s.kind)).collect();
    assert!(!path.partial, "{v:?}: partial path {kinds:?}");
    let rep = w.simulate(&path);
    assert_eq!(
        rep.outcome,
        SimOutcome::Arrived,
        "{v:?}: {:?} at step {} of {}, end {:?}\npath {kinds:?}",
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
    assert!(
        goal.is_reached(end),
        "{v:?}: ended in {end:?}\npath {kinds:?}"
    );
    assert!(
        rep.max_fall <= 3.0 + 1e-3,
        "{v:?}: fall damage {}",
        rep.max_fall
    );
    (path, rep)
}

fn count(path: &Path, f: impl Fn(MoveKind) -> bool) -> usize {
    path.steps.iter().filter(|s| f(s.kind)).count()
}

/// Digging with a shovel-like cost, blocks up to hardness 1 (dirt, leaves; not logs).
fn soft_dig() -> Config {
    Config::default().with_dig(Some(DigCost {
        base: 8.0,
        ticks_per_hardness: 30.0,
        max_hardness: 1.0,
    }))
}

/// On a 7 × 7 × 6 block of dirt, a goal at the floor's level: dig straight down six times.
#[test]
fn dig_down_6() {
    for v in VERSIONS {
        let mut w = World::flat(v);
        w.fill([-3, 64, -3], [3, 69, 3], "dirt");
        let (path, _) = check(&w, soft_dig(), [0, 70, 0], &Goal::Y(64));
        // Down at the edge, then a three-block drop: cheaper than six digs.
        assert!(count(&path, |k| k == MoveKind::DigDown) >= 3, "{v:?}");
        // Straight under the start: six digs in a row.
        let (path, _) = check(&w, soft_dig(), [0, 70, 0], &Goal::Block([0, 64, 0]));
        assert_eq!(count(&path, |k| k == MoveKind::DigDown), 6, "{v:?}");
        // Without digging there is no way down (a six-block drop at every edge).
        let r = w.plan(Config::default(), [0, 70, 0], &Goal::Y(64));
        assert!(r.is_err() || r.unwrap().partial, "{v:?}");
    }
}

/// A tree crown of two leaf layers over a trunk: digging down through the leaves (hardness
/// 0.2) gets the player low enough to drop to the ground. The probe's single canopy eight
/// blocks up stays unreachable without a long fall.
#[test]
fn down_from_leaves_soft_dig() {
    for v in VERSIONS {
        let mut w = World::flat(v);
        w.fill([-2, 66, -2], [2, 67, 2], "oak_leaves");
        w.fill([0, 64, 0], [0, 66, 0], "oak_log");
        let (path, _) = check(&w, soft_dig(), [1, 68, 0], &Goal::Block([5, 64, 0]));
        assert!(count(&path, |k| k == MoveKind::DigDown) >= 1, "{v:?}");
        // The probe's layout: one canopy at floor + 8 over a trunk, logs too hard to dig.
        let mut w = World::flat(v);
        w.fill([-2, 71, -2], [2, 71, 2], "oak_leaves");
        w.fill([0, 64, 0], [0, 70, 0], "oak_log");
        let r = w.plan(soft_dig(), [1, 72, 0], &Goal::Block([5, 64, 0]));
        assert!(r.is_err() || r.unwrap().partial, "{v:?}");
        // ... reachable once a fall of that height is allowed.
        let path = w
            .plan(
                soft_dig().with_max_fall(8.0),
                [1, 72, 0],
                &Goal::Block([5, 64, 0]),
            )
            .unwrap();
        assert!(!path.partial, "{v:?}");
        assert_eq!(w.simulate(&path).outcome, SimOutcome::Arrived, "{v:?}");
    }
}

/// Never dig down into lava, next to water, or where the fall after the dig is too high.
#[test]
fn dig_down_is_safe() {
    for v in VERSIONS {
        for case in 0..3 {
            // A 1 × 1 dirt pillar: the player stands on top.
            let mut w = World::flat(v);
            w.fill([0, 64, 0], [0, 69, 0], "dirt");
            let forbidden = match case {
                0 => {
                    w.set([0, 64, 0], "lava");
                    [0, 65, 0]
                }
                1 => {
                    w.set([1, 66, 0], "water");
                    [0, 66, 0]
                }
                _ => {
                    // Floating: a dig at y 67 drops the player four blocks.
                    w.fill([0, 64, 0], [0, 66, 0], "air");
                    [0, 67, 0]
                }
            };
            let Ok(path) = w.plan(soft_dig(), [0, 70, 0], &Goal::Y(64)) else {
                continue;
            };
            assert!(
                path.steps.iter().all(|s| !s.dig.contains(&forbidden)),
                "{v:?} case {case}: digs {forbidden:?}: {:?}",
                path.steps
            );
            let rep = w.simulate(&path);
            assert_ne!(rep.outcome, SimOutcome::Hazard, "{v:?} case {case}");
            assert!(
                rep.max_fall <= 3.0 + 1e-3,
                "{v:?} case {case}: {}",
                rep.max_fall
            );
        }
    }
}

/// A stone wall across the way (x = 3, three high) with a door at z = 0.
fn door_wall(v: Version, door: &str, facing: &str, open: bool, hinge: &str) -> World {
    let mut w = World::flat(v);
    w.fill([3, 64, -64], [3, 66, 79], "stone");
    w.set(
        [3, 64, 0],
        &format!("{door}[facing={facing},half=lower,hinge={hinge},open={open}]"),
    );
    w.set(
        [3, 65, 0],
        &format!("{door}[facing={facing},half=upper,hinge={hinge},open={open}]"),
    );
    w
}

/// An open door is passed like a gap in the wall, whichever way it opened.
#[test]
fn door_open() {
    for v in VERSIONS {
        for facing in ["east", "west"] {
            for hinge in ["left", "right"] {
                let w = door_wall(v, "oak_door", facing, true, hinge);
                let (path, _) = check(&w, Config::default(), [0, 64, 0], &Goal::Block([6, 64, 0]));
                assert!(
                    path.steps.iter().any(|s| s.pos == [3, 64, 0]),
                    "{v:?} {facing} {hinge}: not through the door"
                );
                assert_eq!(
                    count(&path, |k| matches!(k, MoveKind::Door { .. })),
                    0,
                    "{v:?}: an open door needs no click"
                );
                // An open iron door too.
                let w = door_wall(v, "iron_door", facing, true, hinge);
                check(&w, Config::default(), [0, 64, 0], &Goal::Block([6, 64, 0]));
            }
        }
    }
}

/// A closed wooden door is opened with a click; a closed iron door never is.
#[test]
fn door_closed() {
    for v in VERSIONS {
        for facing in ["east", "west"] {
            let w = door_wall(v, "oak_door", facing, false, "left");
            let (path, _) = check(&w, Config::default(), [0, 64, 0], &Goal::Block([6, 64, 0]));
            assert_eq!(
                count(&path, |k| matches!(k, MoveKind::Door { .. })),
                1,
                "{v:?} {facing}"
            );
            let w = door_wall(v, "iron_door", facing, false, "left");
            let r = w.plan(Config::default(), [0, 64, 0], &Goal::Block([6, 64, 0]));
            assert!(
                r.is_err() || r.unwrap().partial,
                "{v:?}: through an iron door"
            );
        }
        // A door in a wall along x (crossed along z).
        let mut w = World::flat(v);
        w.fill([-64, 64, 3], [79, 66, 3], "stone");
        w.set(
            [0, 64, 3],
            "oak_door[facing=north,half=lower,hinge=left,open=false]",
        );
        w.set(
            [0, 65, 3],
            "oak_door[facing=north,half=upper,hinge=left,open=false]",
        );
        check(&w, Config::default(), [0, 64, 0], &Goal::Block([0, 64, 6]));
        // A closed fence gate in a wall.
        let mut w = World::flat(v);
        w.fill([3, 64, -64], [3, 66, 79], "stone");
        w.set([3, 64, 0], "oak_fence_gate[facing=east,open=false]");
        w.set([3, 65, 0], "air");
        let (path, _) = check(&w, Config::default(), [0, 64, 0], &Goal::Block([6, 64, 0]));
        assert_eq!(
            count(&path, |k| matches!(k, MoveKind::Door { .. })),
            1,
            "{v:?} gate"
        );
    }
}

/// Placing on, gap jumps off (as in the probe).
fn place() -> Config {
    Config::default().with_place(Some(20.0)).with_parkour(false)
}

/// A gap three wide over the void: bridged with three blocks, each placed against the side
/// of the floor block while sneaking at its edge.
#[test]
fn bridge_gap_3() {
    for v in VERSIONS {
        let mut w = World::empty(v);
        w.fill([-8, 63, -8], [1, 63, 8], "stone");
        w.fill([5, 63, -8], [12, 63, 8], "stone");
        let (path, _) = check(&w, place(), [0, 64, 0], &Goal::Block([7, 64, 0]));
        let bridged = path
            .steps
            .iter()
            .filter(|s| s.place.is_some_and(|p| p.face()[1] == 0))
            .count();
        assert_eq!(bridged, 3, "{v:?}");
        // Without placing: no way over.
        let r = w.plan(
            Config::default().with_parkour(false),
            [0, 64, 0],
            &Goal::Block([7, 64, 0]),
        );
        assert!(r.is_err() || r.unwrap().partial, "{v:?}");
        // Along z and toward negative x as well.
        let mut w = World::empty(v);
        w.fill([-8, 63, -8], [8, 63, 1], "stone");
        w.fill([-8, 63, 4], [8, 63, 8], "stone");
        check(&w, place(), [0, 64, 0], &Goal::Block([0, 64, 6]));
        check(&w, place(), [0, 64, 6], &Goal::Block([0, 64, -2]));
    }
}

/// The goal on top of a three-high glass pillar beside the start, no other way up: tower up
/// (jump, place under the feet), then over.
#[test]
fn tower_up_3() {
    for v in VERSIONS {
        let mut w = World::flat(v);
        w.fill([3, 64, 0], [3, 66, 0], "glass");
        let (path, _) = check(&w, place(), [0, 64, 0], &Goal::Block([3, 67, 0]));
        // Two blocks up, then a jump onto the pillar (or three, then a step).
        assert!(count(&path, |k| k == MoveKind::Pillar) >= 2, "{v:?}");
        // Straight up: a goal three above the start.
        let (path, _) = check(&w, place(), [0, 64, 0], &Goal::Block([0, 67, 0]));
        assert_eq!(count(&path, |k| k == MoveKind::Pillar), 3, "{v:?}");
        // Under a low ceiling there is no room to jump: no tower.
        let mut w = World::flat(v);
        w.fill([-8, 66, -8], [8, 66, 8], "stone");
        let r = w.plan(place(), [0, 64, 0], &Goal::Block([0, 65, 0]));
        assert!(r.is_err() || r.unwrap().partial, "{v:?}");
    }
}

/// An unreachable goal (sealed in a stone shell above the floor): with placing and digging
/// on, the search gives up after about as many nodes as without them, instead of towering,
/// bridging and digging over the whole loaded area.
#[test]
fn unreachable_goals_stay_cheap_with_building_moves() {
    for v in VERSIONS {
        let mut w = World::flat(v);
        // A floating platform (void below its edges) and the shell.
        w.fill([-64, 63, -64], [79, 63, 79], "air");
        w.fill([-20, 63, -20], [20, 63, 20], "stone");
        w.fill([9, 68, -1], [11, 70, 1], "stone");
        w.set([10, 69, 0], "air");
        let goal = Goal::Block([10, 69, 0]);
        let (r, plain) = w.plan_counted(Config::default(), [0, 64, 0], &goal);
        assert!(r.is_err() || r.unwrap().partial, "{v:?}");
        let building = Config::default()
            .with_place(Some(20.0))
            .with_dig(Some(DigCost {
                base: 6.0,
                ticks_per_hardness: 5.0,
                max_hardness: 1.0,
            }));
        let (r, built) = w.plan_counted(building, [0, 64, 0], &goal);
        assert!(r.is_err() || r.unwrap().partial, "{v:?}");
        eprintln!("{v:?}: {plain} nodes plain, {built} with building moves");
        assert!(
            built <= plain * 2 + 8192,
            "{v:?}: {built} nodes (plain {plain})"
        );
    }
}

/// Digging with an axe-like cost: blocks up to hardness 2 (logs, not stone).
fn axe_dig() -> Config {
    Config::default().with_dig(Some(DigCost {
        base: 6.0,
        ticks_per_hardness: 7.5,
        max_hardness: 2.0,
    }))
}

/// Staircase moves: steps down with digs.
fn stairs(path: &Path) -> usize {
    path.steps
        .iter()
        .filter(|s| s.kind == MoveKind::Descend && !s.dig.is_empty())
        .count()
}

/// Buried in dirt, a goal 20 blocks lower and 20 away: a staircase (cheaper than a tunnel and
/// a shaft); 20 blocks straight down to a `Goal::Y`; and with a lava pocket under the start
/// column, round it. Never any fall damage.
#[test]
fn dig_staircase_down() {
    for v in VERSIONS {
        let mut w = World::flat(v);
        w.fill([-40, 64, -40], [40, 100, 40], "dirt");
        w.fill([0, 84, 0], [0, 85, 0], "air");
        let (path, _) = check(&w, soft_dig(), [0, 84, 0], &Goal::Block([20, 64, 0]));
        let kinds: Vec<_> = path
            .steps
            .iter()
            .map(|s| (s.pos, s.kind, s.dig.len()))
            .collect();
        assert!(
            stairs(&path) >= 10,
            "{v:?}: {} stair steps {kinds:?}",
            stairs(&path)
        );
        let (path, _) = check(&w, soft_dig(), [0, 84, 0], &Goal::Y(64));
        assert!(path.steps.len() >= 21, "{v:?}");
        // Lava under the start column: straight down stops above it.
        w.set([0, 75, 0], "lava");
        let (path, _) = check(&w, soft_dig(), [0, 84, 0], &Goal::Y(64));
        assert!(
            path.steps.iter().all(|s| !s.dig.contains(&[0, 76, 0])),
            "{v:?}: dug above the lava"
        );
    }
}

/// The probe's tree: one leaf canopy eight up over a trunk. With an axe-level dig the bot
/// digs the leaf over the trunk, then down the trunk, all without a fall.
#[test]
fn down_from_leaves_axe_dig() {
    for v in VERSIONS {
        let mut w = World::flat(v);
        w.fill([-2, 71, -2], [2, 71, 2], "oak_leaves");
        w.fill([0, 64, 0], [0, 70, 0], "oak_log");
        let (path, _) = check(&w, axe_dig(), [1, 72, 0], &Goal::Block([5, 64, 0]));
        assert!(
            path.steps.iter().any(|s| s.dig.contains(&[0, 70, 0])),
            "{v:?}: not down the trunk"
        );
    }
}
