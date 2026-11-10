//! Replays the committed 1.8.9 golden vectors (`tests/golden/1.8.9/*.jsonl.gz`, made by the
//! vanilla oracle) and requires every recorded field of every tick to match bit for bit.
#![cfg(feature = "v1_8")]
#![recursion_limit = "256"]

mod common;

use common::{replay_file, repo_root};

fn check(file: &str) {
    let path = repo_root().join("tests/golden/1.8.9").join(file);
    let s = replay_file(&path);
    for f in &s.failures {
        eprintln!("MISMATCH {f}");
    }
    eprintln!(
        "{file}: {} cases, {} ticks matched, {} cases differ",
        s.cases,
        s.ticks,
        s.failures.len()
    );
    assert!(s.cases > 0, "no cases in {file}");
    assert!(
        s.failures.is_empty(),
        "{} of {} cases in {file} differ from vanilla (first: {})",
        s.failures.len(),
        s.cases,
        s.failures[0]
    );
}

#[test]
fn scenarios_1_8_9() {
    check("scenarios.jsonl.gz");
}

#[test]
fn fuzz_seed1_1_8_9() {
    check("fuzz_seed1.jsonl.gz");
}

#[test]
fn fuzz_edges_seed1_1_8_9() {
    if repo_root()
        .join("tests/golden/1.8.9/fuzz_edges_seed1.jsonl.gz")
        .exists()
    {
        check("fuzz_edges_seed1.jsonl.gz");
    }
}

/// Regression (live Grim test): landing on soul sand while walking into a grass edge 0.125 higher
/// steps up in the landing tick. Vanilla output of `oracle/run.sh replay` on our own cases.
#[test]
fn soulsand_edge_landing_step_up() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/soulsand_edge_1_8_9.jsonl.gz");
    let s = replay_file(&path);
    for f in &s.failures {
        eprintln!("MISMATCH {f}");
    }
    assert_eq!(s.cases, 4);
    assert!(s.failures.is_empty());
}

/// Regression (live lab run, our own throwaway account): a recorded run restarted from ghast's
/// state with its exact box; vanilla agrees through the ledge clip, the landing step-up and the
/// look-only packet. Guards the "box is the position" rule (a box rebuilt from pos differs in
/// the last bit after a clip).
#[test]
fn lab_soulsand_restart_with_box() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/lab_soulsand_restart_1_8_9.jsonl.gz");
    let s = replay_file(&path);
    for f in &s.failures {
        eprintln!("MISMATCH {f}");
    }
    assert_eq!(s.cases, 1);
    assert!(s.failures.is_empty());
}
