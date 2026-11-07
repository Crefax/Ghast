//! Labyrinths: generated perfect mazes of growing size whose way is many times the straight
//! distance, planned in one go and in per-tick slices, and run in the physics; unreachable
//! goals end the search cleanly.

mod common;

use std::collections::HashSet;

use common::{B, VERSIONS, World, check_goal, check_goal_touching, kinds};
use ghast_pathfinder::{Budget, Config, Goal, Pathfinder, PlanError, Version};

fn rng(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

/// A perfect maze (recursive backtracker) of `n` × `n` cells, walls `h` high, grid (2n + 1)²
/// centred on the origin. Cell (i, j) has its feet block at (2i + 1 - n', 64, 2j + 1 - n')
/// with n' = n. Openings west of the first cell and east of the last one.
fn maze(w: &mut World, n: i32, h: i32, seed: u64) {
    let o = -n;
    let size = 2 * n + 1;
    w.fill([o, 64, o], [o + size - 1, 63 + h, o + size - 1], B::Stone);
    let carve = |w: &mut World, gx: i32, gz: i32| {
        w.fill([o + gx, 64, o + gz], [o + gx, 63 + h, o + gz], B::Air);
    };
    let mut s = seed | 1;
    let mut seen = vec![false; (n * n) as usize];
    let mut stack = vec![(0, 0)];
    seen[0] = true;
    carve(w, 1, 1);
    while let Some(&(cx, cz)) = stack.last() {
        let nb: Vec<(i32, i32)> = [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .map(|(dx, dz)| (cx + dx, cz + dz))
            .filter(|&(x, z)| x >= 0 && z >= 0 && x < n && z < n && !seen[(z * n + x) as usize])
            .collect();
        if nb.is_empty() {
            stack.pop();
            continue;
        }
        let (nx, nz) = nb[(rng(&mut s) % nb.len() as u64) as usize];
        seen[(nz * n + nx) as usize] = true;
        carve(w, 2 * nx + 1, 2 * nz + 1);
        carve(w, cx + nx + 1, cz + nz + 1);
        stack.push((nx, nz));
    }
    carve(w, 0, 1);
    carve(w, size - 1, size - 2);
}

/// The feet block of maze cell (i, j).
fn cell(n: i32, i: i32, j: i32) -> [i32; 3] {
    [2 * i + 1 - n, 64, 2 * j + 1 - n]
}

fn plan_sliced(
    w: &World,
    cfg: Config,
    from: [i32; 3],
    goal: &Goal,
    nodes: usize,
) -> (Result<ghast_pathfinder::Path, PlanError>, usize, bool) {
    let pf = Pathfinder::new(cfg);
    let mut search = pf.search(w.version(), from, goal.clone(), HashSet::new());
    let mut slices = 0;
    loop {
        slices += 1;
        let before = search.expanded();
        let r = match w {
            World::V18(m) => search.step(m, Budget::nodes(nodes)),
            World::V16(m) => search.step(m, Budget::nodes(nodes)),
        };
        assert!(search.expanded() - before <= nodes);
        if let Some(r) = r {
            return (r, slices, search.exhausted());
        }
    }
}

#[test]
fn mazes_of_growing_size() {
    for v in VERSIONS {
        for (n, seed) in [(7, 21u64), (15, 22), (31, 23)] {
            let mut w = World::flat(v, 4);
            maze(&mut w, n, 2, seed);
            // Corner to corner inside, and from outside to the centre cell.
            let centre = cell(n, n / 2, n / 2);
            for (from, goal) in [
                (cell(n, 0, 0), Goal::Block(cell(n, n - 1, n - 1))),
                (
                    [-n - 4, 64, n + 4],
                    Goal::Near {
                        pos: centre,
                        radius: 1.0,
                    },
                ),
            ] {
                // Corners of one-wide corridors: at most a tick or two of grazing a wall.
                let (p, _) = check_goal_touching(&w, Config::default(), from, &goal, 2);
                // Sliced like the follower does it: the same path.
                let (r, slices, _) = plan_sliced(&w, Config::default(), from, &goal, 2000);
                assert_eq!(r.as_ref(), Ok(&p), "{v:?} maze {}", 2 * n + 1);
                eprintln!(
                    "{v:?} maze {}: {} steps, {} nodes, {} slices of 2000",
                    2 * n + 1,
                    p.steps.len(),
                    p.expanded,
                    slices
                );
            }
        }
        // The way to the centre of the biggest maze is several times the straight line.
        let n = 31;
        let mut w = World::flat(v, 4);
        maze(&mut w, n, 3, 23);
        let from = [-n - 4, 64, n + 4];
        let centre = cell(n, n / 2, n / 2);
        let goal = Goal::Near {
            pos: centre,
            radius: 1.0,
        };
        let (p, _) = check_goal_touching(&w, Config::default(), from, &goal, 2);
        let straight = f64::from(from[0] - centre[0]).hypot(f64::from(from[2] - centre[2]));
        assert!(
            p.steps.len() as f64 > 3.0 * straight,
            "{v:?}: {} steps",
            p.steps.len()
        );
    }
}

#[test]
fn unreachable_goals_end_cleanly() {
    for v in VERSIONS {
        let n = 15;
        // A cell walled in on all sides.
        let mut w = World::flat(v, 4);
        maze(&mut w, n, 2, 31);
        let goal = cell(n, 7, 7);
        w.fill(
            [goal[0] - 1, 64, goal[2] - 1],
            [goal[0] + 1, 65, goal[2] + 1],
            B::Stone,
        );
        w.set(goal, B::Air);
        w.set([goal[0], 65, goal[2]], B::Air);
        let (r, _, exhausted) = plan_sliced(
            &w,
            Config::default(),
            cell(n, 0, 0),
            &Goal::Block(goal),
            2000,
        );
        assert!(exhausted, "{v:?}");
        let p = r.expect("a partial path to the closest block");
        assert!(p.partial, "{v:?}");
        // Every reachable block, and nothing more: the inside of the maze only (the openings
        // lead out, so the outside counts too), far below the node budget.
        assert!(p.expanded < 30_000, "{v:?}: {}", p.expanded);
        // A goal in the air: the path ends under it.
        let w = World::flat(v, 4);
        let goal = Goal::Near {
            pos: [10, 70, -4],
            radius: 2.0,
        };
        let (r, _, exhausted) = plan_sliced(&w, Config::default(), [-20, 64, 3], &goal, 2000);
        let p = r.unwrap();
        assert!(p.partial && exhausted, "{v:?}");
        let end = p.steps.last().unwrap().pos;
        assert!(
            (end[0] - 10).abs() <= 1 && (end[2] + 4).abs() <= 1,
            "{v:?}: {end:?}"
        );
        // From there, nothing is closer: no path.
        let (r, _, _) = plan_sliced(&w, Config::default(), end, &goal, 2000);
        assert_eq!(r, Err(PlanError::NoPath), "{v:?}");
    }
}

#[test]
fn out_of_nodes_in_a_dead_end() {
    for v in VERSIONS {
        // A tiny node budget from the end of a dead-end corridor pointing at the goal: no
        // expanded block is closer than the start, so the partial path goes to the most
        // promising unexpanded one instead of giving up.
        let mut w = World::flat(v, 4);
        w.fill([0, 64, -1], [12, 65, -1], B::Stone);
        w.fill([0, 64, 1], [12, 65, 1], B::Stone);
        w.fill([12, 64, 0], [12, 65, 0], B::Stone);
        w.fill([14, 64, -30], [14, 65, 30], B::Stone);
        let cfg = Config::default().with_max_nodes(30);
        let (r, _, exhausted) = plan_sliced(&w, cfg, [11, 64, 0], &Goal::Block([20, 64, 0]), 2000);
        let p = r.unwrap_or_else(|e| panic!("{v:?}: {e}"));
        assert!(p.partial && !exhausted, "{v:?}: {:?}", kinds(&p));
        assert!(p.steps.len() > 1, "{v:?}");
        // With the default budget the way around is found.
        check_goal(
            &w,
            Config::default(),
            [11, 64, 0],
            &Goal::Block([20, 64, 0]),
        );
        let _ = Version::V1_8;
    }
}
