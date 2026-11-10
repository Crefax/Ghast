//! Replays the committed 1.12.2 golden vectors (`tests/golden/1.12.2/`) bit for bit.
#![cfg(feature = "v1_12")]
#![recursion_limit = "256"]

mod common;

use common::repo_root;
use common::v112::replay_file;

fn check(file: &str) {
    let path = repo_root().join("tests/golden/1.12.2").join(file);
    let s = replay_file(&path);
    for f in s.failures.iter().take(10) {
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
fn scenarios_1_12_2() {
    check("scenarios.jsonl.gz");
}

#[test]
fn fuzz_seed1_1_12_2() {
    check("fuzz_seed1.jsonl.gz");
}

#[test]
fn fuzz_edges_seed1_1_12_2() {
    check("fuzz_edges_seed1.jsonl.gz");
}
