//! Collision boxes vs the oracle's box dump (`tests/golden/1.8.9/collision_boxes.jsonl.gz`): every
//! block id × meta alone and in the dumped neighbour configurations, bit for bit.

use std::path::PathBuf;

use ghast_data::v1_8::Block;
use serde_json::Value;

use super::HistoryShapes;
use super::blocks::boxes_of;
use super::memory_world;

fn f64_of(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
}

/// The option a dump line is the reference for.
fn history_of(v: &Value) -> HistoryShapes {
    if v.get("shape_source").and_then(Value::as_str) == Some("settled") {
        HistoryShapes::Settled
    } else {
        HistoryShapes::Constructor
    }
}

/// Reads a gzip-compressed text file.
fn read_gz(path: &std::path::Path) -> String {
    use std::io::Read;
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).expect("box dump"))
        .read_to_string(&mut text)
        .expect("gzip");
    text
}

fn sorted(mut v: Vec<[u64; 6]>) -> Vec<[u64; 6]> {
    v.sort_unstable();
    v
}

#[test]
fn collision_boxes_match_oracle_dump() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.8.9/collision_boxes.jsonl.gz");
    let text = read_gz(&path);
    let (bx, by, bz) = (0, 64, 0);
    let mut checked = 0;
    let mut failures = Vec::new();
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["type"] != "boxes" {
            continue;
        }
        let block = v["id"].as_u64().unwrap() as u8;
        // History-dependent shapes (anvil, chests): "settled" lines are the reference for
        // `HistoryShapes::Settled`, the others (constructor bounds) for `Constructor`.
        let history = history_of(&v);
        let meta = v["meta"].as_u64().unwrap() as u16;
        let mut w = memory_world();
        w.load_chunks(-1, -1, 0, 0);
        w.set_raw(bx, by, bz, (u16::from(block) << 4) | meta);
        for n in v["neighbours"].as_array().unwrap() {
            let n: Vec<i64> = n
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap())
                .collect();
            w.set_raw(
                bx + n[0] as i32,
                by + n[1] as i32,
                bz + n[2] as i32,
                ((n[3] as u16) << 4) | n[4] as u16,
            );
        }
        let expected: Vec<[u64; 6]> = v["boxes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                let b: Vec<f64> = b.as_array().unwrap().iter().map(f64_of).collect();
                let (x, y, z) = (f64::from(bx), f64::from(by), f64::from(bz));
                [b[0] + x, b[1] + y, b[2] + z, b[3] + x, b[4] + y, b[5] + z].map(f64::to_bits)
            })
            .collect();
        let actual: Vec<[u64; 6]> = boxes_of(&w, bx, by, bz, history)
            .iter()
            .map(|a| [a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z].map(f64::to_bits))
            .collect();
        checked += 1;
        if sorted(expected.clone()) != sorted(actual.clone()) {
            let name = Block::from_id(block).map_or("?", |b| b.name());
            failures.push(format!(
                "{name} {block}:{meta} [{}] {}: expected {:?} actual {:?}",
                v["config"],
                v["neighbours"],
                expected
                    .iter()
                    .map(|b| b.map(f64::from_bits))
                    .collect::<Vec<_>>(),
                actual
                    .iter()
                    .map(|b| b.map(f64::from_bits))
                    .collect::<Vec<_>>(),
            ));
        }
    }
    for f in failures.iter().take(40) {
        eprintln!("{f}");
    }
    eprintln!("{checked} box lines checked, {} differ", failures.len());
    assert!(checked > 4000);
    assert!(failures.is_empty(), "{} box lines differ", failures.len());
}

/// Cactus, cake and fence gates build their boxes in `float` from absolute coordinates: far from
/// the origin the box snaps to the float grid (vanilla behaviour the fuzz relies on).
#[test]
fn float_boxes_far_away() {
    let mut w = memory_world();
    let x = 29_999_901;
    w.load_chunks(x >> 4, 0, x >> 4, 0);
    w.set_raw(x, 64, 3, 81 << 4); // cactus
    let b = boxes_of(&w, x, 64, 3, HistoryShapes::Settled)[0];
    assert_eq!(b.min_x, f64::from(x as f32 + 0.0625));
    assert_eq!(b.max_x, f64::from((x + 1) as f32 - 0.0625));
    assert_eq!(b.min_y, 64.0);
}

/// The block predicates the rules use (backed by `ghast-data`) agree with the dump's block lines.
#[test]
fn block_predicates_match_oracle_dump() {
    use super::blocks::{blocks_movement, connects_as_solid, is_fluid, is_full_cube, is_solid};
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.8.9/collision_boxes.jsonl.gz");
    let text = read_gz(&path);
    let mut n = 0;
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["type"] != "block" {
            continue;
        }
        let id = v["id"].as_u64().unwrap() as u8;
        let name = v["name"].as_str().unwrap().trim_start_matches("minecraft:");
        let on = |k: &str| v[k].as_bool().unwrap();
        let block = Block::from_id(id).unwrap();
        assert_eq!(block.name(), name);
        let s = block.default_state();
        assert_eq!(is_full_cube(s), on("full_cube"), "{name}");
        assert_eq!(blocks_movement(s), on("material_blocks_movement"), "{name}");
        assert_eq!(is_solid(s), on("material_solid"), "{name}");
        assert_eq!(is_fluid(s), on("material_fluid"), "{name}");
        assert_eq!(block.vanilla().full_block, on("full_block"), "{name}");
        let gate = name.ends_with("fence_gate");
        let connects = gate
            || (on("material_opaque")
                && on("render_as_normal_block")
                && !matches!(name, "pumpkin" | "lit_pumpkin" | "melon_block" | "barrier"));
        assert_eq!(connects_as_solid(s), connects, "{name}");
        n += 1;
    }
    assert_eq!(n, ghast_data::v1_8::BLOCK_COUNT);
}

/// Block ids the rules compare by identity have the expected names.
#[test]
fn identity_ids() {
    use super::blocks::id;
    for (block, name) in [
        (id::GLASS, "glass"),
        (id::WEB, "web"),
        (id::LADDER, "ladder"),
        (id::SOUL_SAND, "soul_sand"),
        (id::STAINED_GLASS, "stained_glass"),
        (id::VINE, "vine"),
        (id::NETHER_BRICK_FENCE, "nether_brick_fence"),
        (id::COBBLESTONE_WALL, "cobblestone_wall"),
        (id::ANVIL, "anvil"),
        (id::SLIME, "slime"),
        (id::BARRIER, "barrier"),
        (id::PUMPKIN, "pumpkin"),
        (id::LIT_PUMPKIN, "lit_pumpkin"),
        (id::MELON_BLOCK, "melon_block"),
    ] {
        assert_eq!(Block::from_id(block).map(Block::name), Some(name));
    }
    let names = |ids: &[u8]| -> Vec<&str> {
        ids.iter()
            .map(|&i| Block::from_id(i).unwrap().name())
            .collect()
    };
    for n in names(&[
        53, 67, 108, 109, 114, 128, 134, 135, 136, 156, 163, 164, 180,
    ]) {
        assert!(n.ends_with("_stairs"), "{n}");
    }
    for n in names(&[64, 71, 193, 194, 195, 196, 197]) {
        assert!(n.ends_with("_door"), "{n}");
    }
    for n in names(&[107, 183, 184, 185, 186, 187]) {
        assert!(n.ends_with("fence_gate"), "{n}");
    }
    for n in names(&[85, 188, 189, 190, 191, 192]) {
        assert!(n.ends_with("fence") && !n.contains("nether"), "{n}");
    }
    assert_eq!(
        names(&[101, 102, 160]),
        ["iron_bars", "glass_pane", "stained_glass_pane"]
    );
}

/// Absolute boxes at far positions (`collision_boxes_far.jsonl.gz`): float-built boxes (cactus,
/// cake, fence gates) lose precision there exactly like vanilla's.
#[test]
fn collision_boxes_far_match_oracle_dump() {
    use std::io::Read;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.8.9/collision_boxes_far.jsonl.gz");
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path).expect("far box dump"))
        .read_to_string(&mut text)
        .unwrap();
    let (mut checked, mut failures) = (0, Vec::new());
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["type"] != "boxes_abs" {
            continue;
        }
        let history = history_of(&v);
        let block = v["id"].as_u64().unwrap() as u16;
        let meta = v["meta"].as_u64().unwrap() as u16;
        let pos: Vec<i32> = v["pos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap() as i32)
            .collect();
        let mut w = memory_world();
        w.load_chunks(pos[0] >> 4, pos[2] >> 4, pos[0] >> 4, pos[2] >> 4);
        w.set_raw(pos[0], pos[1], pos[2], (block << 4) | meta);
        let expected: Vec<[u64; 6]> = v["boxes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| {
                let b: Vec<u64> = b
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| f64_of(x).to_bits())
                    .collect();
                [b[0], b[1], b[2], b[3], b[4], b[5]]
            })
            .collect();
        let actual: Vec<[u64; 6]> = boxes_of(&w, pos[0], pos[1], pos[2], history)
            .iter()
            .map(|a| [a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z].map(f64::to_bits))
            .collect();
        checked += 1;
        if sorted(expected) != sorted(actual) {
            failures.push(format!("{block}:{meta} at {pos:?}"));
        }
    }
    eprintln!(
        "{checked} far box lines checked, {} differ: {:?}",
        failures.len(),
        &failures[..failures.len().min(20)]
    );
    assert!(checked > 3000);
    assert!(failures.is_empty());
}

/// Ray bounds (`ray_shape`) vs the oracle's `selection_boxes.jsonl.gz`: every id × meta alone and
/// the neighbour configurations, bit for bit (stairs are checked by the raytrace vectors).
#[test]
fn ray_bounds_match_selection_dump() {
    use super::{RayShape, ray_shape};
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.8.9/selection_boxes.jsonl.gz");
    if !path.exists() {
        eprintln!("skipped: no selection dump");
        return;
    }
    let text = read_gz(&path);
    let (bx, by, bz) = (0, 64, 0);
    let (mut checked, mut failures) = (0, Vec::new());
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["type"] != "selection" {
            continue;
        }
        let block = v["id"].as_u64().unwrap() as u16;
        let meta = v["meta"].as_u64().unwrap() as u16;
        // Torches set their ray bounds from the facing in `TorchBlock.rayTrace`; the dump's
        // `setBoundingBox` bounds are stale for them (the ray bounds are proven by the
        // `picks.jsonl.gz` replay in tests/pick_legacy.rs).
        if matches!(block, 50 | 75 | 76) {
            continue;
        }
        let mut w = memory_world();
        w.load_chunks(-1, -1, 0, 0);
        w.set_raw(bx, by, bz, (block << 4) | meta);
        for n in v["neighbours"].as_array().unwrap() {
            let n: Vec<i64> = n
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap())
                .collect();
            w.set_raw(
                bx + n[0] as i32,
                by + n[1] as i32,
                bz + n[2] as i32,
                ((n[3] as u16) << 4) | n[4] as u16,
            );
        }
        let want: Vec<u64> = v["bounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| f64_of(x).to_bits())
            .collect();
        match ray_shape(&w, bx, by, bz, false) {
            RayShape::Stairs(_) => continue,
            RayShape::None => {
                if v["can_collide"].as_bool().unwrap() {
                    failures.push(format!(
                        "{block}:{meta} {}: None but can_collide",
                        v["config"]
                    ));
                }
            }
            RayShape::Bounds(b) => {
                let have = [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z].map(f64::to_bits);
                if have.as_slice() != want.as_slice() {
                    failures.push(format!(
                        "{block}:{meta} {} {}: want {:?} have {:?}",
                        v["config"],
                        v["neighbours"],
                        want.iter().map(|&b| f64::from_bits(b)).collect::<Vec<_>>(),
                        have.map(f64::from_bits)
                    ));
                }
            }
        }
        checked += 1;
    }
    for f in failures.iter().take(20) {
        eprintln!("{f}");
    }
    eprintln!(
        "{checked} selection lines checked, {} differ",
        failures.len()
    );
    assert!(checked > 3000);
    assert!(failures.is_empty());
}
