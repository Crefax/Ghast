//! Replays the committed 1.16.5 golden vectors (`tests/golden/1.16.5/`) bit for bit.
#![recursion_limit = "256"]

mod common;

use common::repo_root;
use common::v116::replay_file;

fn check(file: &str) {
    let path = repo_root().join("tests/golden/1.16.5").join(file);
    if !path.exists() {
        eprintln!("skipped: {} not found", path.display());
        return;
    }
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
    assert!(
        s.failures.is_empty(),
        "{} of {} cases in {file} differ",
        s.failures.len(),
        s.cases
    );
}

#[test]
fn scenarios_1_16_5() {
    check("scenarios.jsonl.gz");
}

#[test]
fn fuzz_seed1_1_16_5() {
    check("fuzz_seed1.jsonl.gz");
}
