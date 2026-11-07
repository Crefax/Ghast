//! Collision boxes and face shapes vs the 1.12.2 oracle dumps (`tests/golden/1.12.2/`): every
//! block id × meta alone and in the dumped neighbour configurations, bit for bit.

use std::path::PathBuf;

use ghast_data::v1_12::{Block, Face};
use serde_json::Value;

use super::blocks::{boxes_of, face_shape, state_at};
use super::memory_world;

fn f64_of(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
}

fn lines(file: &str) -> Vec<Value> {
    use std::io::Read;
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.12.2")
        .join(file);
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path).expect("dump"))
        .read_to_string(&mut text)
        .expect("gzip");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn sorted(mut v: Vec<[u64; 6]>) -> Vec<[u64; 6]> {
    v.sort_unstable();
    v
}

/// The world of a dump line: the block at `pos` plus its listed neighbours.
fn world_of(v: &Value, (bx, by, bz): (i32, i32, i32)) -> super::MemoryWorld {
    let mut w = memory_world();
    w.load_chunks((bx >> 4) - 1, (bz >> 4) - 1, (bx >> 4) + 1, (bz >> 4) + 1);
    let id = v["id"].as_u64().unwrap() as u16;
    let meta = v["meta"].as_u64().unwrap() as u16;
    w.set_raw(bx, by, bz, (id << 4) | meta);
    for n in v["neighbours"].as_array().into_iter().flatten() {
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
    w
}

fn boxes(v: &Value, key: &str, offset: (f64, f64, f64)) -> Vec<[u64; 6]> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            let b: Vec<f64> = b.as_array().unwrap().iter().map(f64_of).collect();
            let (x, y, z) = offset;
            [b[0] + x, b[1] + y, b[2] + z, b[3] + x, b[4] + y, b[5] + z].map(f64::to_bits)
        })
        .collect()
}

fn ours(w: &super::MemoryWorld, (x, y, z): (i32, i32, i32)) -> Vec<[u64; 6]> {
    boxes_of(w, x, y, z)
        .iter()
        .map(|a| [a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z].map(f64::to_bits))
        .collect()
}

#[test]
fn collision_boxes_match_oracle_dump() {
    let pos = (0, 64, 0);
    let mut checked = 0;
    let mut failures = Vec::new();
    for v in lines("collision_boxes.jsonl.gz") {
        if v["type"] != "boxes" {
            continue;
        }
        let w = world_of(&v, pos);
        let expected = boxes(&v, "boxes", (0.0, 64.0, 0.0));
        let actual = ours(&w, pos);
        checked += 1;
        if sorted(expected.clone()) != sorted(actual.clone()) {
            let id = v["id"].as_u64().unwrap() as u8;
            let name = Block::from_id(id).map_or("?", |b| b.name());
            failures.push(format!(
                "{name} {id}:{} [{}] {}: expected {:?} actual {:?}",
                v["meta"],
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
    assert!(checked > 6000);
    assert!(failures.is_empty(), "{} box lines differ", failures.len());
}

#[test]
fn far_collision_boxes_match_oracle_dump() {
    let mut checked = 0;
    let mut failures = 0;
    for v in lines("collision_boxes_far.jsonl.gz") {
        if v["type"] != "boxes_abs" {
            continue;
        }
        let p: Vec<i32> = v["pos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap() as i32)
            .collect();
        let pos = (p[0], p[1], p[2]);
        let w = world_of(&v, pos);
        let expected = boxes(&v, "boxes", (0.0, 0.0, 0.0));
        checked += 1;
        if sorted(expected) != sorted(ours(&w, pos)) {
            failures += 1;
            if failures < 20 {
                eprintln!(
                    "far {} {}:{} at {:?} differs",
                    v["config"], v["id"], v["meta"], pos
                );
            }
        }
    }
    eprintln!("{checked} far box lines checked, {failures} differ");
    assert!(checked > 10_000);
    assert_eq!(failures, 0);
}

#[test]
fn stairs_face_shapes_match_oracle_dump() {
    let pos = (0, 64, 0);
    let faces = [
        ("down", Face::Down),
        ("up", Face::Up),
        ("north", Face::North),
        ("south", Face::South),
        ("west", Face::West),
        ("east", Face::East),
    ];
    let mut checked = 0;
    for v in lines("face_shapes.jsonl.gz") {
        if v["type"] != "faces" || v["state"].is_null() {
            continue;
        }
        let w = world_of(&v, pos);
        let s = state_at(&w, pos.0, pos.1, pos.2);
        for (name, face) in faces {
            let got = format!("{:?}", face_shape(&w, pos.0, pos.1, pos.2, s, face));
            let want = v["faces"][name]
                .as_str()
                .unwrap()
                .replace('_', "")
                .to_lowercase();
            assert_eq!(
                got.to_lowercase(),
                want,
                "{} {}:{} [{}] {}: face {name}",
                s.block().name(),
                v["id"],
                v["meta"],
                v["config"],
                v["neighbours"]
            );
        }
        checked += 1;
    }
    assert!(checked > 2000, "{checked}");
}
