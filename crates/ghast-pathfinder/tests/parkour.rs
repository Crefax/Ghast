//! Gap jumps: planned only where the vanilla-exact physics lands them, and run cleanly by the
//! controller in the simulation. A gap is a trench across the whole world, 4 deep (falling
//! in is a dead end), so jumping is the only way over unless stated otherwise.

mod common;

use common::{B, VERSIONS, World, check, kinds, parkour_gaps};
use ghast_pathfinder::{Config, Goal};

/// A trench `width` wide starting at x = `x0`, from y = 60 up (open to the sky).
fn trench(w: &mut World, x0: i32, width: i32) {
    w.fill([x0, 60, -64], [x0 + width - 1, 63, 79], B::Air);
}

#[test]
fn flat_gaps_of_one_two_and_three() {
    for v in VERSIONS {
        for g in 1..=3 {
            let mut w = World::flat(v, 4);
            trench(&mut w, 5, g);
            let (p, _) = check(&w, Config::default(), [0, 64, 0], [5 + g + 4, 64, 0]);
            assert_eq!(
                parkour_gaps(&p),
                vec![g as u8],
                "{v:?} gap {g}: {:?}",
                kinds(&p)
            );
        }
    }
}

#[test]
fn gaps_up_one_block() {
    for v in VERSIONS {
        for g in 1..=2 {
            let mut w = World::flat(v, 4);
            trench(&mut w, 5, g);
            // The far side is one block higher.
            w.fill([5 + g, 64, -64], [79, 64, 79], B::Stone);
            let (p, _) = check(&w, Config::default(), [0, 64, 0], [5 + g + 4, 65, 0]);
            // One jump from the edge (it may land a block further than the first block up).
            let gaps = parkour_gaps(&p);
            assert!(
                gaps.len() == 1 && gaps[0] >= g as u8,
                "{v:?} gap {g} up: {:?}",
                kinds(&p)
            );
            assert!(p.steps.iter().any(|s| s.pos == [4, 64, 0]));
        }
    }
}

#[test]
fn gaps_down_one_block() {
    for v in VERSIONS {
        for g in 1..=3 {
            let mut w = World::flat(v, 4);
            trench(&mut w, 5, g);
            // The far side is one block lower.
            w.fill([5 + g, 63, -64], [79, 63, 79], B::Air);
            w.fill([5 + g, 62, -64], [79, 62, 79], B::Stone);
            let (p, _) = check(&w, Config::default(), [0, 64, 0], [5 + g + 4, 63, 0]);
            let gaps = parkour_gaps(&p);
            assert!(
                gaps.len() == 1 && gaps[0] >= g as u8,
                "{v:?} gap {g} down: {:?}",
                kinds(&p)
            );
        }
    }
}

#[test]
fn up_onto_a_narrow_ledge_and_down_again() {
    for v in VERSIONS {
        let mut w = World::flat(v, 4);
        // Gap, a one-block-wide ledge one block up, gap, the floor again.
        trench(&mut w, 5, 1);
        w.fill([6, 60, -64], [6, 64, 79], B::Stone);
        trench(&mut w, 7, 1);
        let (p, _) = check(&w, Config::default(), [0, 64, 0], [12, 64, 0]);
        // Up onto the ledge, then down (possibly over the first floor block behind it).
        assert_eq!(parkour_gaps(&p).len(), 2, "{v:?}: {:?}", kinds(&p));
        assert!(
            p.steps.iter().any(|s| s.pos == [6, 65, 0]),
            "{v:?}: {:?}",
            kinds(&p)
        );
    }
}

#[test]
fn stepping_stones_over_lava() {
    for v in VERSIONS {
        let mut w = World::flat(v, 4);
        w.fill([-64, 63, -64], [79, 63, 79], B::Air);
        w.fill([-64, 58, -64], [79, 58, 79], B::Lava);
        // A start pad, 1x1 pillars with gaps of 1, 2, 3 and 1 (one up), an end pad.
        w.fill([-2, 63, -2], [0, 63, 2], B::Stone);
        w.set([2, 63, 0], B::Stone);
        w.set([5, 63, 0], B::Stone);
        w.set([9, 63, 0], B::Stone);
        w.set([11, 64, 0], B::Stone);
        w.fill([11, 63, 0], [11, 63, 0], B::Stone);
        w.fill([13, 64, -2], [16, 64, 2], B::Stone);
        let (p, _) = check(&w, Config::default(), [-1, 64, 0], [15, 65, 0]);
        assert_eq!(
            parkour_gaps(&p),
            vec![1, 2, 3, 1, 1],
            "{v:?}: {:?}",
            kinds(&p)
        );
    }
}

#[test]
fn wide_gaps_are_not_jumped() {
    for v in VERSIONS {
        // Five wide: no jump; a bridge far to the side is the way.
        let mut w = World::flat(v, 4);
        trench(&mut w, 5, 5);
        w.fill([5, 63, 40], [9, 63, 40], B::Stone);
        let (p, _) = check(&w, Config::default(), [0, 64, 0], [14, 64, 0]);
        assert!(
            p.steps.iter().any(|s| s.pos[2] == 40),
            "{v:?}: {:?}",
            kinds(&p)
        );
        assert!(parkour_gaps(&p).is_empty(), "{v:?}: {:?}", kinds(&p));
        // Without the bridge there is no way over.
        let mut w = World::flat(v, 4);
        trench(&mut w, 5, 5);
        let r = w.try_plan(Config::default(), [0, 64, 0], &Goal::Block([14, 64, 0]));
        assert!(r.as_ref().map_or(true, |p| p.partial), "{v:?}: {r:?}");
        // Four wide on the level, or four wide and one up, is too far as well.
        for up in [0, 1] {
            let mut w = World::flat(v, 4);
            trench(&mut w, 5, 4);
            if up == 1 {
                w.fill([9, 64, -64], [79, 64, 79], B::Stone);
            }
            w.fill([5, 63, 40], [8, 63, 40], B::Stone);
            let (p, _) = check(&w, Config::default(), [0, 64, 0], [13, 64 + up, 0]);
            assert!(
                p.steps.iter().any(|s| s.pos[2] == 40),
                "{v:?}: {:?}",
                kinds(&p)
            );
            assert!(parkour_gaps(&p).is_empty(), "{v:?}: {:?}", kinds(&p));
        }
    }
}

#[test]
fn a_low_ceiling_blocks_the_jump() {
    for v in VERSIONS {
        // Glass two blocks above the floor over the gap: the jump arc would hit it.
        let mut w = World::flat(v, 4);
        trench(&mut w, 5, 2);
        w.fill([3, 66, -64], [9, 66, 79], B::Glass);
        w.fill([5, 63, 30], [6, 63, 30], B::Stone);
        let (p, _) = check(&w, Config::default(), [0, 64, 0], [11, 64, 0]);
        assert!(parkour_gaps(&p).is_empty(), "{v:?}: {:?}", kinds(&p));
    }
}

#[test]
fn no_parkour_without_the_option() {
    for v in VERSIONS {
        let mut w = World::flat(v, 4);
        trench(&mut w, 5, 1);
        let r = w.try_plan(
            Config::default().with_parkour(false),
            [0, 64, 0],
            &Goal::Block([10, 64, 0]),
        );
        assert!(r.as_ref().map_or(true, |p| p.partial), "{v:?}: {r:?}");
    }
}

#[test]
fn every_direction_and_a_walled_corridor() {
    for v in VERSIONS {
        // Pillars over lava along -x, +z and -z: gaps of 3, 2 (one up) and 1 (one down).
        for (dx, dz) in [(-1, 0), (0, 1), (0, -1), (1, 0)] {
            let mut w = World::flat(v, 4);
            w.fill([-64, 63, -64], [79, 63, 79], B::Air);
            w.fill([-64, 58, -64], [79, 58, 79], B::Lava);
            let p = |a: i32, y: i32| [dx * a, y, dz * a];
            w.set(p(0, 63), B::Stone);
            w.set(p(4, 63), B::Stone);
            w.fill(p(7, 63), p(7, 64), B::Stone);
            w.set(p(9, 63), B::Stone);
            let (path, _) = check(&w, Config::default(), p(0, 64), p(9, 64));
            assert_eq!(
                parkour_gaps(&path),
                vec![3, 2, 1],
                "{v:?} {dx} {dz}: {:?}",
                kinds(&path)
            );
        }
        // A jump between glass walls (a closed one-wide corridor over a trench).
        let mut w = World::flat(v, 4);
        trench(&mut w, 5, 2);
        for z in [-1, 1] {
            w.fill([-2, 60, z], [14, 68, z], B::Glass);
        }
        for x in [-2, 14] {
            w.fill([x, 60, -1], [x, 68, 1], B::Glass);
        }
        let (path, _) = check(&w, Config::default(), [0, 64, 0], [11, 64, 0]);
        assert_eq!(parkour_gaps(&path), vec![2], "{v:?}: {:?}", kinds(&path));
        assert!(
            path.steps.iter().all(|s| s.pos[2] == 0),
            "{v:?}: {:?}",
            kinds(&path)
        );
    }
}

#[test]
fn a_shallow_ditch_is_jumped() {
    for v in VERSIONS {
        // One deep and one or two wide (a ditch dug into a superflat world): one jump is
        // cheaper than climbing down and up again.
        for g in 1..=2 {
            let mut w = World::flat(v, 4);
            w.fill([0, 60, -64], [79, 62, 79], B::Stone);
            w.fill([5, 63, -64], [4 + g, 63, 79], B::Air);
            let (p, _) = check(&w, Config::default(), [0, 64, 0], [12, 64, 0]);
            assert_eq!(
                parkour_gaps(&p),
                vec![g as u8],
                "{v:?} {g}: {:?}",
                kinds(&p)
            );
        }
    }
}
