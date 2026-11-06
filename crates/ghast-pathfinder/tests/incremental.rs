//! Planning and verifying in slices (`Pathfinder::search`, `Simulation::step`) gives exactly
//! the one-shot results, and every slice stays within its budget.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use ghast_pathfinder::{
    Budget, Config, ControllerConfig, Goal, Path, Pathfinder, PlanError, Simulation, Version,
    simulate,
};
use ghast_physics::v1_8::{MemoryWorld, memory_world};

const STONE: u16 = 1 << 4;

fn flat() -> MemoryWorld {
    let mut w = memory_world();
    w.load_chunks(-4, -4, 4, 4);
    w.fill_raw([-64, 63, -64], [79, 63, 79], STONE);
    w
}

/// Rolling terrain with a long wall: a search of tens of thousands of nodes.
fn rough() -> MemoryWorld {
    let mut w = flat();
    let mut s = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for x in -30..=70 {
        for z in -30..=70 {
            let r = rnd();
            let h = (r % 3) as i32;
            if h > 0 {
                w.fill_raw([x, 64, z], [x, 63 + h, z], STONE);
            }
            match (r >> 8) % 20 {
                0 => w.set_raw(x, 64 + h, z, 44 << 4),
                1 => w.set_raw(x, 63 + h, z, 88 << 4),
                2 if h == 0 => w.set_raw(x, 63, z, 0),
                _ => {}
            }
        }
    }
    w.fill_raw([25, 64, -30], [25, 70, 70], STONE);
    w.fill_raw([25, 64, 60], [25, 70, 61], 0);
    for p in [[0, 64, 0], [50, 64, 30]] {
        w.set_raw(p[0], 63, p[2], STONE);
        w.fill_raw(p, [p[0], 72, p[2]], 0);
    }
    w
}

/// (world, from, goal) cases: open ground, a detour, a partial plan, the error cases.
fn cases() -> Vec<(MemoryWorld, [i32; 3], Goal)> {
    let mut wall = flat();
    wall.fill_raw([5, 64, -40], [5, 66, 40], STONE);
    wall.fill_raw([5, 64, 39], [5, 65, 39], 0);
    let mut blocked = flat();
    blocked.fill_raw([0, 64, 0], [0, 65, 0], STONE);
    vec![
        (flat(), [0, 64, 0], Goal::Block([40, 64, 25])),
        (wall, [0, 64, 0], Goal::Block([10, 64, 0])),
        (rough(), [0, 64, 0], Goal::Block([50, 64, 30])),
        (flat(), [0, 64, 0], Goal::Xz { x: 120, z: 0 }),
        (flat(), [0, 64, 0], Goal::Block([0, 64, 0])),
        (flat(), [0, 64, 0], Goal::Block([0, 70, 0])),
        (blocked, [0, 64, 0], Goal::Block([5, 64, 0])),
        (flat(), [500, 64, 0], Goal::Block([5, 64, 0])),
    ]
}

/// Runs a search to the end in slices of `budget`; checks every slice's node count.
fn sliced(
    pf: &Pathfinder,
    w: &MemoryWorld,
    from: [i32; 3],
    goal: &Goal,
    budget: Budget,
) -> (Result<Path, PlanError>, usize) {
    let mut s = pf.search(Version::V1_8, from, goal.clone(), HashSet::new());
    let mut slices = 0;
    loop {
        let before = s.expanded();
        let r = s.step(w, budget);
        slices += 1;
        assert!(
            s.expanded() - before <= budget.nodes,
            "slice expanded {} > {}",
            s.expanded() - before,
            budget.nodes
        );
        if let Some(r) = r {
            // Done: further steps give the same result.
            assert_eq!(s.step(w, budget), Some(r.clone()));
            assert!(s.is_done());
            return (r, slices);
        }
    }
}

#[test]
fn sliced_search_equals_the_one_shot_plan() {
    let pf = Pathfinder::new(Config::default());
    for (w, from, goal) in cases() {
        let one = pf.plan(Version::V1_8, &w, from, &goal);
        for n in [1, 7, 64, 500, 100_000] {
            let (r, slices) = sliced(&pf, &w, from, &goal, Budget::nodes(n));
            assert_eq!(r, one, "{goal:?} in slices of {n}");
            if let Ok(p) = &one {
                assert!(
                    slices >= p.expanded / n,
                    "{slices} slices for {}",
                    p.expanded
                );
            }
        }
    }
}

#[test]
fn sliced_search_respects_the_node_budget_on_a_long_detour() {
    let pf = Pathfinder::new(Config::default());
    let w = rough();
    let goal = Goal::Block([50, 64, 30]);
    let one = pf.plan(Version::V1_8, &w, [0, 64, 0], &goal).unwrap();
    assert!(one.expanded > 5_000, "{}", one.expanded);
    let (r, slices) = sliced(&pf, &w, [0, 64, 0], &goal, Budget::nodes(2000));
    assert_eq!(r.unwrap(), one);
    assert_eq!(
        slices,
        one.expanded.div_ceil(2000).max(1) + usize::from(one.expanded.is_multiple_of(2000))
    );
}

#[test]
fn sliced_search_respects_the_time_budget() {
    let pf = Pathfinder::new(Config::default());
    let w = rough();
    let goal = Goal::Block([50, 64, 30]);
    let one = pf.plan(Version::V1_8, &w, [0, 64, 0], &goal).unwrap();
    let budget = Budget::nodes(usize::MAX).with_time(Duration::from_millis(1));
    let mut s = pf.search(Version::V1_8, [0, 64, 0], goal, HashSet::new());
    let mut slowest = Duration::ZERO;
    let mut slices = 0;
    let r = loop {
        let t = Instant::now();
        let r = s.step(&w, budget);
        slowest = slowest.max(t.elapsed());
        slices += 1;
        if let Some(r) = r {
            break r;
        }
    };
    assert_eq!(r.unwrap(), one);
    assert!(slices > 1, "the search should not fit in 1 ms");
    // The clock is read every 64 nodes: a slice overshoots by at most that much work.
    assert!(
        slowest < Duration::from_millis(20),
        "slowest slice {slowest:?}"
    );
}

#[test]
fn sliced_simulation_equals_simulate() {
    let pf = Pathfinder::new(Config::default());
    let cfg = ControllerConfig::default();
    for (w, from, goal) in cases() {
        let Ok(path) = pf.plan(Version::V1_8, &w, from, &goal) else {
            continue;
        };
        let one = simulate(&path, &w, &cfg);
        for n in [1, 13, 400] {
            let mut sim = Simulation::new(&path, &cfg);
            let mut slices = 0u32;
            let before = |s: &Simulation| s.report().ticks;
            let r = loop {
                let t = before(&sim);
                let done = sim.step(&w, n, None).cloned();
                assert!(sim.report().ticks - t <= n);
                slices += 1;
                if let Some(r) = done {
                    break r;
                }
            };
            assert_eq!(r, one, "{goal:?} in slices of {n}");
            assert!(slices >= one.ticks / n);
        }
    }
}

#[test]
fn sliced_search_works_on_1_16_5() {
    let mut w = ghast_physics::v1_16_5::memory_world();
    let stone = ghast_data::v1_16_5::Block::STONE.default_state().id();
    w.load_chunks(-2, -2, 2, 2);
    w.fill_raw([-30, 63, -30], [30, 63, 30], stone);
    w.fill_raw([5, 64, -20], [5, 66, 20], stone);
    let pf = Pathfinder::new(Config::default());
    let goal = Goal::Block([10, 64, 0]);
    let one = pf.plan(Version::V1_16_5, &w, [0, 64, 0], &goal).unwrap();
    let mut s = pf.search(Version::V1_16_5, [0, 64, 0], goal, HashSet::new());
    let r = loop {
        if let Some(r) = s.step(&w, Budget::nodes(3)) {
            break r;
        }
    };
    assert_eq!(r.unwrap(), one);
}
