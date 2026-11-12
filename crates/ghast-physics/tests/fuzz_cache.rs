//! Large-fuzz replay: every `*.jsonl.gz` in `oracle/cache/1.8.9/fuzz/`, `oracle/cache/1.8.9/fuzz-edges/`,
//! `oracle/cache/1.16.5/fuzz{,-edges,-extras}/` and `oracle/cache/1.12.2/fuzz{,-edges,-extras}/` (or the file / directory in `GHAST_FUZZ`) produced by
//! `oracle/run.sh fuzz`, compared bit for bit. The version is taken from the path.
//!
//! Needs the git-ignored oracle cache, so it is behind the `oracle-cache` feature:
//!
//! ```sh
//! oracle/run.sh fuzz --seed 11 --count 2000 --ticks 100 --out "$PWD/oracle/cache/1.8.9/fuzz/s11.jsonl.gz"
//! cargo test --release -p ghast-physics --features oracle-cache --test fuzz_cache -- --nocapture
//! ```
#![cfg(feature = "oracle-cache")]
#![recursion_limit = "256"]

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(feature = "v1_8")]
use common::replay_file;
use common::{Summary, repo_root};

fn inputs() -> Vec<PathBuf> {
    let specs = match std::env::var("GHAST_FUZZ") {
        Ok(s) => vec![PathBuf::from(s)],
        Err(_) => vec![
            repo_root().join("oracle/cache/1.8.9/fuzz"),
            repo_root().join("oracle/cache/1.8.9/fuzz-edges"),
            repo_root().join("oracle/cache/1.8.9/fuzz-extras"),
            repo_root().join("oracle/cache/1.16.5/fuzz"),
            repo_root().join("oracle/cache/1.16.5/fuzz-edges"),
            repo_root().join("oracle/cache/1.16.5/fuzz-extras"),
            repo_root().join("oracle/cache/1.12.2/fuzz"),
            repo_root().join("oracle/cache/1.12.2/fuzz-edges"),
            repo_root().join("oracle/cache/1.12.2/fuzz-extras"),
            repo_root().join("oracle/cache/26.1/fuzz"),
            repo_root().join("oracle/cache/26.1/fuzz-edges"),
            repo_root().join("oracle/cache/26.1/fuzz-extras"),
            repo_root().join("oracle/cache/26.1/fuzz-teleports"),
            repo_root().join("oracle/cache/26.1/fuzz-elytra"),
            repo_root().join("oracle/cache/26.1/fuzz-riptide"),
            repo_root().join("oracle/cache/26.1/fuzz-entities"),
            repo_root().join("oracle/cache/26.1/fuzz-nether"),
            repo_root().join("oracle/cache/26.1/fuzz-border"),
        ],
    };
    let mut files = Vec::new();
    for spec in specs {
        if spec.is_file() {
            files.push(spec);
            continue;
        }
        let Ok(dir) = std::fs::read_dir(&spec) else {
            continue;
        };
        files.extend(
            dir.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.to_string_lossy().ends_with(".jsonl.gz")),
        );
    }
    files.sort();
    files
}

fn replay_any(path: &std::path::Path) -> Summary {
    let name = path.to_string_lossy();
    #[cfg(feature = "v26_1")]
    if name.contains("26.1") {
        return common::v261::replay_file(path);
    }
    #[cfg(feature = "v1_16_5")]
    if name.contains("1.16.5") {
        return common::v116::replay_file(path);
    }
    #[cfg(feature = "v1_12")]
    if name.contains("1.12.2") {
        return common::v112::replay_file(path);
    }
    #[cfg(feature = "v1_8")]
    if !["26.1", "1.16.5", "1.12.2"]
        .iter()
        .any(|v| name.contains(v))
    {
        return replay_file(path);
    }
    panic!("{name}: its version feature is off")
}

#[test]
fn oracle_fuzz_cache() {
    let files = inputs();
    assert!(
        !files.is_empty(),
        "no fuzz files; run oracle/run.sh fuzz first"
    );
    let results: Vec<(PathBuf, Summary)> = std::thread::scope(|s| {
        let handles: Vec<_> = files
            .iter()
            .map(|f| s.spawn(move || (f.clone(), replay_any(f))))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let (mut cases, mut ticks, mut failed) = (0, 0, 0);
    let mut by_field: BTreeMap<String, usize> = BTreeMap::new();
    for (path, s) in &results {
        eprintln!(
            "{}: {} cases, {} ticks matched, {} cases differ",
            path.file_name().unwrap().to_string_lossy(),
            s.cases,
            s.ticks,
            s.failures.len()
        );
        for f in s.failures.iter().take(5) {
            eprintln!("MISMATCH {f}");
        }
        for f in &s.failures {
            *by_field.entry(f.field.clone()).or_default() += 1;
        }
        cases += s.cases;
        ticks += s.ticks;
        failed += s.failures.len();
    }
    eprintln!("TOTAL: {cases} cases, {ticks} ticks matched, {failed} cases differ");
    for (field, n) in &by_field {
        eprintln!("  first differing field {field}: {n} cases");
    }
    assert_eq!(failed, 0, "{failed} fuzz cases differ from vanilla");
}
