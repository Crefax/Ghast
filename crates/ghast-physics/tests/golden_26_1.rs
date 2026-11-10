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
