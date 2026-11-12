//! Replays the committed 26.2 golden vectors (`tests/golden/26.2/`) bit for bit with the `V26_2` rules.
#![cfg(all(feature = "v26_1", feature = "v26_2"))]
#![recursion_limit = "256"]

mod common;

use common::repo_root;
use common::v261::replay_file_with;
use ghast_physics::modern::V26_2;

fn check(file: &str) {
    let path = repo_root().join("tests/golden/26.2").join(file);
    if !path.exists() {
        eprintln!("skipped: {} not found", path.display());
        return;
    }
    let s = replay_file_with::<V26_2>(&path);
    for f in s.failures.iter().take(40) {
        eprintln!("MISMATCH {f}");
    }
    eprintln!(
        "{file}: {} cases, {} ticks matched, {} cases differ",
        s.cases,
        s.ticks,
        s.failures.len()
    );
    assert!(
        s.failures.is_empty(),
        "{} of {} cases in {file} differ",
        s.failures.len(),
        s.cases
    );
}

#[test]
fn scenarios_26_2() {
    check("scenarios.jsonl.gz");
}

/// Joins through the level-load gate: no ticking until `player_loaded`, then the normal tick.
#[test]
fn joins_26_2() {
    check("joins.jsonl.gz");
}

#[test]
fn fuzz_seed1_26_2() {
    check("fuzz_seed1.jsonl.gz");
}

#[test]
fn fuzz_edges_seed1_26_2() {
    check("fuzz_edges_seed1.jsonl.gz");
}

#[test]
fn fuzz_extras_seed1_26_2() {
    check("fuzz_extras_seed1.jsonl.gz");
}

/// 26.2's collision restitution: bounciness, friction and air drag modifiers, the new blocks.
#[test]
fn fuzz_restitution_seed1_26_2() {
    check("fuzz_restitution_seed1.jsonl.gz");
}

/// Position packets with delta movement under all relative-bit combinations, with entity motion
/// around them (hand-written cases + the `withTeleports` fuzz stream).
#[test]
fn teleports_26_2() {
    check("teleports.jsonl.gz");
}

/// Gliding with an elytra: start, glide physics, rockets, climbables, the server's shared flags.
#[test]
fn elytra_26_2() {
    check("elytra.jsonl.gz");
}

/// Riptide: the trident launch on release, the spin counter, the server's spin flag and pose.
#[test]
fn riptide_26_2() {
    check("riptide.jsonl.gz");
}

/// Other entities: collision with boats and shulkers, pushes by mobs and boats.
#[test]
fn entities_26_2() {
    check("entities.jsonl.gz");
}

/// The nether: fast lava flow (the dimension's fast_lava attribute), build height 0-255.
#[test]
fn nether_26_2() {
    check("nether.jsonl.gz");
}

/// The world border: set, moved and lerping (the collision size lags the lerp by a tick).
#[test]
fn border_26_2() {
    check("border.jsonl.gz");
}

/// Boats the player rides and controls: water, land, ice, falls, walls, flowing water, under
/// water, other boats, server corrections, dismounts.
#[test]
fn boats_26_2() {
    check("boats.jsonl.gz");
}

/// Riding a minecart the server moves: the passenger's tick and seat.
#[test]
fn minecarts_26_2() {
    check("minecarts.jsonl.gz");
}

/// The player's pose dimensions (`Avatar.POSES`) against the oracle's entity size dump.
#[test]
fn player_pose_dimensions_26_2() {
    use ghast_physics::modern::Pose;
    let path = repo_root().join("tests/golden/26.2/entity_sizes.jsonl.gz");
    if !path.exists() {
        eprintln!("skipped: {} not found", path.display());
        return;
    }
    let line = common::open_lines(&path)
        .find(|l| l.contains("\"minecraft:player (client player)\""))
        .expect("client player line");
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    let f = |x: &serde_json::Value| common::f32_of(x);
    for (name, pose) in [
        ("standing", Pose::Standing),
        ("crouching", Pose::Crouching),
        ("swimming", Pose::Swimming),
        ("fall_flying", Pose::FallFlying),
        ("spin_attack", Pose::SpinAttack),
        ("sleeping", Pose::Sleeping),
        ("dying", Pose::Dying),
    ] {
        let d = &v["poses"][name];
        assert!(!d.is_null(), "pose {name} missing from the dump");
        let (w, h, e) = pose.dimensions();
        assert_eq!((f(&d[0]), f(&d[1]), f(&d[2])), (w, h, e), "pose {name}");
    }
}
