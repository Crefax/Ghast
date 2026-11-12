//! Family check: replays 26.2 oracle vectors through the modern core with the 26.1 rules. Block
//! states are bridged by name (the 26.2 header's `states` map → `ghast_data::v26_1::BlockState::parse`);
//! a case that uses a state 26.1 does not have is reported as "unmapped", not replayed.
//!
//! `GHAST_FAMILY=<file or dir>[:<file or dir>…]` (default: `tests/golden/26.2/*.jsonl.gz`).
#![cfg(feature = "v26_1")]
#![recursion_limit = "256"]

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use common::repo_root;
use serde_json::Value;

/// The 26.1 rules with the 26.2 mechanics switched on (the 26.2 tables are not needed: every
/// state the vectors use exists in 26.1 with the same shapes and properties).
struct V26_2On261;

impl ghast_physics::modern::Rules for V26_2On261 {
    type State = <ghast_physics::modern::V26_1 as ghast_physics::modern::Rules>::State;
    const PROTOCOL: i32 = 776;
    const AIR: Self::State = <ghast_physics::modern::V26_1 as ghast_physics::modern::Rules>::AIR;
    fn phys(s: Self::State) -> ghast_data::v26_1::StatePhys {
        <ghast_physics::modern::V26_1 as ghast_physics::modern::Rules>::phys(s)
    }
    fn block(s: Self::State) -> &'static ghast_data::v26_1::BlockData {
        <ghast_physics::modern::V26_1 as ghast_physics::modern::Rules>::block(s)
    }
    fn voxel(i: u16) -> &'static ghast_data::v26_1::VoxelShapeRef {
        <ghast_physics::modern::V26_1 as ghast_physics::modern::Rules>::voxel(i)
    }
    const FULL_CUBE_VOXEL: u16 = ghast_data::v26_1::FULL_CUBE_VOXEL;
    const SCAFFOLDING_BOTTOM_VOXEL: u16 = ghast_data::v26_1::SCAFFOLDING_BOTTOM_VOXEL;
    const POWDER_SNOW_FALLING_VOXEL: u16 = ghast_data::v26_1::POWDER_SNOW_FALLING_VOXEL;
    const POSITION_THRESHOLD_SQ: f64 = 2.0e-4 * 2.0e-4;
    const POSITION_REMINDER: i32 = 20;
    const HORIZONTAL_CUTOFF_SQ: f64 = 9.0e-6;
    const RESTITUTION: bool = true;
}

impl common::v261::Row for V26_2On261 {
    fn empty_world() -> ghast_physics::modern::ModernMemoryWorld<Self::State> {
        ghast_physics::modern::memory_world_26_1()
    }
}

/// A 26.1 stand-in for a block 26.2 added: the 26.1 block whose every state has the same collision
/// shape, state flags, block properties and tags in the 26.2 dumps (checked for all 2,493 new
/// states): sulfur / cinnabar blocks and potent sulfur → stone, their slabs / stairs / walls →
/// stone slab / stone stairs / cobblestone wall, sulfur spikes → pointed dripstone (same
/// properties).
fn surrogate(name: &str) -> String {
    let (block, props) = name.split_once('[').map_or((name, ""), |(b, p)| (b, p));
    let props = if props.is_empty() {
        String::new()
    } else {
        format!("[{props}")
    };
    let b = block.strip_prefix("minecraft:").unwrap_or(block);
    if b == "sulfur_spike" {
        format!("minecraft:pointed_dripstone{props}")
    } else if b == "potent_sulfur" {
        "minecraft:stone".to_string()
    } else if b.ends_with("_slab") {
        format!("minecraft:stone_slab{props}")
    } else if b.ends_with("_stairs") {
        format!("minecraft:stone_stairs{props}")
    } else if b.ends_with("_wall") {
        format!("minecraft:cobblestone_wall{props}")
    } else {
        "minecraft:stone".to_string()
    }
}

fn bridge(case: &mut common::Case) -> Result<(), String> {
    let states = case.header["states"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut map = BTreeMap::new();
    for (id, name) in &states {
        let name = name.as_str().unwrap();
        match ghast_data::v26_1::BlockState::parse(name)
            .or_else(|| ghast_data::v26_1::BlockState::parse(&surrogate(name)))
        {
            Some(s) => {
                map.insert(id.parse::<u64>().unwrap(), u64::from(s.id()));
            }
            None => return Err(name.to_string()),
        }
    }
    if let Some(fills) = case.header["fills"].as_array_mut() {
        for f in fills {
            let id = f[6].as_u64().unwrap();
            f[6] = Value::from(*map.get(&id).ok_or_else(|| format!("state id {id}"))?);
        }
    }
    Ok(())
}

fn inputs() -> Vec<PathBuf> {
    let list = std::env::var("GHAST_FAMILY")
        .unwrap_or_else(|_| repo_root().join("tests/golden/26.2").display().to_string());
    let mut out = Vec::new();
    for p in list.split(':').map(PathBuf::from) {
        if p.is_dir() {
            let mut v: Vec<_> = std::fs::read_dir(&p)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.to_string_lossy().ends_with(".jsonl.gz"))
                .collect();
            v.sort();
            out.extend(v);
        } else {
            out.push(p);
        }
    }
    out
}

#[test]
#[ignore = "family measurement; run with --ignored"]
fn family_26_2_on_26_1() {
    let mut by_field: BTreeMap<String, usize> = BTreeMap::new();
    for path in inputs() {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let first = common::open_lines(&path).next().unwrap_or_default();
        if !first.contains("\"type\":\"scenario\"") {
            continue;
        }
        let (mut cases, mut ticks, mut unmapped, mut failed) = (0, 0, 0, 0);
        for mut case in common::cases(&path) {
            cases += 1;
            if let Err(s) = bridge(&mut case) {
                unmapped += 1;
                eprintln!("UNMAPPED {name} {}: {s}", case.header["name"]);
                continue;
            }
            match std::panic::catch_unwind(|| common::v261::replay_with::<V26_2On261>(&case)) {
                Ok(Ok(n)) => ticks += n,
                Ok(Err(m)) => {
                    failed += 1;
                    *by_field.entry(m.field.clone()).or_default() += 1;
                    eprintln!("MISMATCH {name} {m}");
                }
                Err(_) => {
                    failed += 1;
                    eprintln!("PANIC {name} {}", case.header["name"]);
                }
            }
        }
        eprintln!(
            "FILE {name}: {cases} cases, {ticks} ticks matched, {failed} differ, {unmapped} unmapped"
        );
    }
    eprintln!("FIELDS {by_field:?}");
}
