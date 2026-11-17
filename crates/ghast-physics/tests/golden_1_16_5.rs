//! Replays the committed 1.16.5 golden vectors (`tests/golden/1.16.5/`) bit for bit.
#![cfg(feature = "v1_16_5")]
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

#[test]
fn fuzz_edges_seed1_1_16_5() {
    check("fuzz_edges_seed1.jsonl.gz");
}

/// Other entities: collision with boats and shulkers, pushes by mobs, other players and boats,
/// team collision rules (hand cases + the golden entities fuzz).
#[test]
fn entities_1_16_5() {
    check("entities.jsonl.gz");
}

/// The nether dimension type: lava pushes with 0.007 (hand cases + the golden nether fuzz).
#[test]
fn nether_1_16_5() {
    check("nether.jsonl.gz");
}

/// Riptide: the trident's use in water and rain, the launch, the spin counter, the spin pose and
/// the spin's contacts (hand cases + the golden riptide fuzz).
#[test]
fn riptide_1_16_5() {
    check("riptide.jsonl.gz");
}

/// Elytra: the glide start, the glide, the server's flag 7, creative flight, attached firework
/// rockets in the client world's tick order (hand cases + the golden elytra fuzz).
#[test]
fn elytra_1_16_5() {
    check("elytra.jsonl.gz");
}

/// Boats the player rides and controls: paddling on water, land, ice, walls, flowing water,
/// under water, other boats, server corrections, dismounting (hand cases + golden fuzz).
#[test]
fn boats_1_16_5() {
    check("boats.jsonl.gz");
}

/// Sitting in a minecart the server moves: its interpolation, the passenger's own tick, the
/// seat, the client's own dismount on sneak, the server's dismount (hand cases + the golden
/// minecart fuzz).
#[test]
fn minecarts_1_16_5() {
    check("minecarts.jsonl.gz");
}
