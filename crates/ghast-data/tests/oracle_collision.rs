//! 1.8.9 tables vs the vanilla oracle's ground truth, `tests/golden/1.8.9/collision_boxes.jsonl.gz`
//! (format: `oracle/README.md`, "Golden file format"). Skipped with a notice if the file is missing.
//!
//! - `block` lines: slipperiness (f32 bits), climbable, fluid material vs `BlockPhysics`.
//! - `boxes` lines without neighbours (every id × meta): the table shape must equal the oracle's
//!   boxes bit for bit, and on `alone` lines `is_registered` must equal `state != null`. This
//!   includes neighbour-dependent blocks, whose table shape is documented as the isolated one.
//!   Anvils and chests follow the oracle's policy: lines with `shape_source = "settled"` are the
//!   reference, `"constructor"` lines (fresh mutable bounds) are only reported.
//! - other configs (neighbours present): blocks whose table claims `ShapeDependency::None` must
//!   not change shape; for neighbour-dependent blocks the differences are only counted (the table
//!   cannot express them; the world crate implements the connection rules).
#![cfg(feature = "v1_8")]

use ghast_data::v1_8::{Block, BlockState};
use ghast_data::{PhysicsFlags, ShapeDependency};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn f64_of(v: &Value) -> f64 {
    let hex = v[1].as_str().expect("[decimal, hex] float");
    f64::from_bits(u64::from_str_radix(hex, 16).unwrap())
}

fn f32_of(v: &Value) -> f32 {
    let hex = v[1].as_str().expect("[decimal, hex] float");
    f32::from_bits(u32::from_str_radix(hex, 16).unwrap())
}

type Boxes = Vec<[u64; 6]>;

fn oracle_boxes(v: &Value) -> Boxes {
    let mut out: Boxes = v
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            let n: Vec<u64> = b
                .as_array()
                .unwrap()
                .iter()
                .map(|x| f64_of(x).to_bits())
                .collect();
            [n[0], n[1], n[2], n[3], n[4], n[5]]
        })
        .collect();
    out.sort_unstable();
    out
}

fn table_boxes(s: BlockState) -> Boxes {
    let mut out: Boxes = s
        .shape()
        .iter()
        .map(|a| [a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z].map(f64::to_bits))
        .collect();
    out.sort_unstable();
    out
}

fn show(b: &Boxes) -> String {
    let v: Vec<[f64; 6]> = b.iter().map(|x| x.map(f64::from_bits)).collect();
    format!("{v:?}")
}

#[test]
fn v1_8_9_tables_match_oracle() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.8.9/collision_boxes.jsonl.gz");
    let Ok(file) = std::fs::File::open(&path) else {
        eprintln!("skipped: {} not found", path.display());
        return;
    };
    let mut text = String::new();
    std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(file), &mut text).unwrap();
    let mut hard = Vec::new();
    let mut soft = Vec::new();
    let mut alone = 0usize;
    let mut blocks = 0usize;
    // block name -> (configs compared, configs differing from the isolated table shape)
    let mut dependent: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let id = v["id"].as_u64().unwrap() as u8;
        let block =
            Block::from_id(id).unwrap_or_else(|| panic!("oracle block id {id} not in table"));
        if v["type"] == "block" {
            blocks += 1;
            assert_eq!(
                v["name"].as_str().unwrap(),
                format!("minecraft:{}", block.name())
            );
            let p = block.physics();
            if f32_of(&v["slipperiness"]).to_bits() != p.slipperiness.to_bits() {
                hard.push(format!(
                    "{}: slipperiness {} != oracle {}",
                    block.name(),
                    p.slipperiness,
                    f32_of(&v["slipperiness"])
                ));
            }
            if v["climbable"].as_bool().unwrap() != p.has(PhysicsFlags::CLIMBABLE) {
                hard.push(format!("{}: climbable mismatch", block.name()));
            }
            let vp = block.vanilla();
            let props = [
                ("full_cube", vp.full_cube),
                ("normal_block", vp.normal_block),
                ("full_block", vp.full_block),
                ("render_as_normal_block", vp.render_as_normal_block),
                ("has_collision", vp.has_collision),
                ("material_blocks_movement", vp.material.blocks_movement),
                ("material_solid", vp.material.solid),
                ("material_fluid", vp.material.fluid),
                ("material_opaque", vp.material.opaque),
                ("material_replaceable", vp.material.replaceable),
            ];
            for (k, ours) in props {
                if v[k].as_bool().unwrap() != ours {
                    hard.push(format!("{}: {k} = {ours}, oracle {}", block.name(), v[k]));
                }
            }
            let fluid = p.has(PhysicsFlags::WATER) || p.has(PhysicsFlags::LAVA);
            if v["material_fluid"].as_bool().unwrap() != fluid {
                hard.push(format!("{}: fluid flag mismatch", block.name()));
            }
            continue;
        }
        let meta = v["meta"].as_u64().unwrap() as u8;
        let state = block.with_meta(meta);
        let want = oracle_boxes(&v["boxes"]);
        let got = table_boxes(state);
        let dep = block.shape_dependency();
        let what = || {
            format!(
                "{}:{meta} ({}) [{}]",
                id,
                block.name(),
                v["config"].as_str().unwrap()
            )
        };
        let neighbours = v["neighbours"].as_array().is_some_and(|n| !n.is_empty());
        // Anvils and chests: only `settled` lines are the ghast reference; `constructor` lines
        // (fresh mutable bounds) are kept by the oracle for information.
        let constructor = v["shape_source"] == "constructor";
        if v["config"] == "alone" {
            alone += 1;
            if state.is_registered() == v["state"].is_null() {
                hard.push(format!(
                    "{}: is_registered() = {}",
                    what(),
                    state.is_registered()
                ));
            }
        }
        if constructor {
            if got != want {
                soft.push(format!(
                    "{}: table {} != constructor bounds {}",
                    what(),
                    show(&got),
                    show(&want)
                ));
            }
        } else if !neighbours || dep == ShapeDependency::None {
            // Isolated (incl. settled-alone) lines, and any line for a block claiming a static shape.
            if got != want {
                hard.push(format!(
                    "{}: table {} != oracle {}",
                    what(),
                    show(&got),
                    show(&want)
                ));
            }
        } else {
            let e = dependent.entry(block.name()).or_default();
            e.0 += 1;
            e.1 += usize::from(got != want);
        }
    }
    eprintln!("checked {blocks} block property lines and {alone} isolated states");
    for (name, (n, diff)) in &dependent {
        eprintln!(
            "neighbour configs for {name}: {n} checked, {diff} differ from the isolated shape (expected)"
        );
    }
    for m in &soft {
        eprintln!("reported only: {m}");
    }
    for m in hard.iter().take(100) {
        eprintln!("MISMATCH: {m}");
    }
    assert_eq!(blocks, 198);
    assert_eq!(alone, 198 * 16);
    assert!(
        hard.is_empty(),
        "{} mismatches vs the vanilla oracle (see stderr)",
        hard.len()
    );
}

/// Far-position dump (`collision_boxes_far.jsonl.gz`): absolute boxes at three far block positions,
/// compared bit for bit with `Collision::world_boxes` (float world-coordinate rules).
#[test]
fn v1_8_9_far_positions_match_oracle() {
    use std::io::Read as _;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.8.9/collision_boxes_far.jsonl.gz");
    let Ok(file) = std::fs::File::open(&path) else {
        eprintln!("skipped: {} not found", path.display());
        return;
    };
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .unwrap();
    let mut hard = Vec::new();
    let (mut compared, mut float_lines) = (0usize, 0usize);
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["type"], "boxes_abs");
        if v["shape_source"] == "constructor"
            || v["neighbours"].as_array().is_some_and(|n| !n.is_empty())
        {
            continue;
        }
        let block = Block::from_id(v["id"].as_u64().unwrap() as u8).unwrap();
        let state = block.with_meta(v["meta"].as_u64().unwrap() as u8);
        let pos: Vec<i32> = v["pos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_i64().unwrap() as i32)
            .collect();
        let c = state.collision();
        let mut got: Boxes = c
            .world_boxes(pos[0], pos[1], pos[2])
            .map(|a| [a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z].map(f64::to_bits))
            .collect();
        got.sort_unstable();
        let want = oracle_boxes(&v["boxes"]);
        compared += 1;
        if v["same_as_local"] == false {
            float_lines += 1;
            if !c.uses_float_world_coords() {
                hard.push(format!("{}:{} ({}) at {pos:?}: oracle differs from double placement but no float rules", block.id(), state.meta(), block.name()));
            }
        }
        if got != want {
            hard.push(format!(
                "{}:{} ({}) at {pos:?}: table {} != oracle {}",
                block.id(),
                state.meta(),
                block.name(),
                show(&got),
                show(&want)
            ));
        }
    }
    eprintln!(
        "compared {compared} far-position states ({float_lines} where float placement matters)"
    );
    for m in hard.iter().take(100) {
        eprintln!("MISMATCH: {m}");
    }
    assert!(compared > 0);
    assert!(
        hard.is_empty(),
        "{} far-position mismatches vs the vanilla oracle (see stderr)",
        hard.len()
    );
}

/// Selection (crosshair ray-test) data vs `selection_boxes.jsonl.gz`.
#[test]
fn v1_8_9_selection_matches_oracle() {
    use std::io::Read as _;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.8.9/selection_boxes.jsonl.gz");
    let Ok(file) = std::fs::File::open(&path) else {
        eprintln!("skipped: {} not found", path.display());
        return;
    };
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .unwrap();
    let mut bad = Vec::new();
    let (mut alone, mut dependent, mut dependent_diff) = (0, 0, 0);
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["type"], "selection");
        let block = Block::from_id(v["id"].as_u64().unwrap() as u8).unwrap();
        let state = block.with_meta(v["meta"].as_u64().unwrap() as u8);
        let sel = state.selection();
        let want: Vec<u64> = v["bounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| f64_of(x).to_bits())
            .collect();
        let b = sel.bounds;
        let got: Vec<u64> = [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]
            .iter()
            .map(|x| x.to_bits())
            .collect();
        let what = format!(
            "{}:{} ({}) [{}]",
            block.id(),
            state.meta(),
            block.name(),
            v["config"].as_str().unwrap()
        );
        if v["config"] == "alone" {
            alone += 1;
            let flags = [
                ("can_collide", sel.can_collide),
                ("can_collide_liquid", sel.can_collide_liquid),
                ("has_collision_box", sel.has_collision_box),
            ];
            for (k, ours) in flags {
                if v[k].as_bool().unwrap() != ours {
                    bad.push(format!("{what}: {k}"));
                }
            }
            if got != want {
                bad.push(format!("{what}: bounds differ"));
            }
        } else if sel.dependency == ShapeDependency::None {
            if got != want {
                bad.push(format!(
                    "{what}: static selection bounds change with neighbours"
                ));
            }
        } else {
            dependent += 1;
            dependent_diff += usize::from(got != want);
        }
    }
    eprintln!(
        "selection: {alone} isolated states; {dependent} neighbour lines, {dependent_diff} differ from the isolated bounds (expected)"
    );
    for m in bad.iter().take(100) {
        eprintln!("MISMATCH: {m}");
    }
    assert_eq!(alone, 198 * 16);
    assert!(bad.is_empty(), "{} selection mismatches", bad.len());
}
