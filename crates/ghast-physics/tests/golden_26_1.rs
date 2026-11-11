//! Replays the committed 26.1 golden vectors (`tests/golden/26.1/`) bit for bit.
#![cfg(feature = "v26_1")]
#![recursion_limit = "256"]

mod common;

use common::repo_root;
use common::v261::replay_file;

fn check(file: &str) {
    let path = repo_root().join("tests/golden/26.1").join(file);
    if !path.exists() {
        eprintln!("skipped: {} not found", path.display());
        return;
    }
    let s = replay_file(&path);
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
fn scenarios_26_1() {
    check("scenarios.jsonl.gz");
}

#[test]
fn fuzz_seed1_26_1() {
    check("fuzz_seed1.jsonl.gz");
}

#[test]
fn fuzz_edges_seed1_26_1() {
    check("fuzz_edges_seed1.jsonl.gz");
}

#[test]
fn fuzz_extras_seed1_26_1() {
    check("fuzz_extras_seed1.jsonl.gz");
}

/// Position packets with delta movement under all relative-bit combinations, with entity motion
/// around them (hand-written cases + the `withTeleports` fuzz stream).
#[test]
fn teleports_26_1() {
    check("teleports.jsonl.gz");
}

/// The player's pose dimensions (`Avatar.POSES`) against the oracle's entity size dump.
#[test]
fn player_pose_dimensions_26_1() {
    use ghast_physics::modern::Pose;
    let path = repo_root().join("tests/golden/26.1/entity_sizes.jsonl.gz");
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
