//! 1.16.5 tables vs the vanilla oracle (`tests/golden/1.16.5/collision_shapes*.jsonl.gz`, format in
//! `oracle/README.md`). Every state's collision boxes, fluid and flags, and every block's physics
//! facts are compared bit for bit; far positions check that only position-offset blocks differ
//! from `local + pos`. Skipped with a notice if the files are missing.
#![cfg(feature = "v1_16_5")]

use ghast_data::v1_16_5::{Block, BlockState, OffsetType};
use ghast_data::{FluidKind, PhysicsFlags, ShapeDependency};
use serde_json::Value;
use std::io::Read as _;
use std::path::PathBuf;

type Boxes = Vec<[u64; 6]>;

fn lines(name: &str) -> Option<Vec<Value>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.16.5")
        .join(name);
    let Ok(file) = std::fs::File::open(&path) else {
        eprintln!("skipped: {} not found", path.display());
        return None;
    };
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .unwrap();
    Some(
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect(),
    )
}

fn bits(v: &Value) -> u64 {
    u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap()
}

fn f32_bits(v: &Value) -> u32 {
    u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap()
}

fn oracle_boxes(v: &Value) -> Boxes {
    let mut out: Boxes = v
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            let n: Vec<u64> = b.as_array().unwrap().iter().map(bits).collect();
            [n[0], n[1], n[2], n[3], n[4], n[5]]
        })
        .collect();
    out.sort_unstable();
    out
}

fn table_boxes(s: BlockState, pos: [i32; 3]) -> Boxes {
    let mut out: Boxes = s
        .collision()
        .world_boxes(pos[0], pos[1], pos[2])
        .map(|a| [a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z].map(f64::to_bits))
        .collect();
    out.sort_unstable();
    out
}

#[test]
fn v1_16_5_tables_match_oracle() {
    let Some(lines) = lines("collision_shapes.jsonl.gz") else {
        return;
    };
    let mut bad = Vec::new();
    let (mut states, mut blocks) = (0, 0);
    for v in &lines {
        if v["type"] == "block" {
            blocks += 1;
            let name = v["name"].as_str().unwrap().trim_start_matches("minecraft:");
            let b = Block::from_name(name).unwrap_or_else(|| panic!("{name} missing"));
            let p = b.physics();
            let checks = [
                (
                    "slipperiness",
                    f32_bits(&v["slipperiness"]) == p.slipperiness.to_bits(),
                ),
                (
                    "velocity_multiplier",
                    f32_bits(&v["velocity_multiplier"]) == p.velocity_multiplier().to_bits(),
                ),
                (
                    "jump_velocity_multiplier",
                    f32_bits(&v["jump_velocity_multiplier"]) == p.jump_factor.to_bits(),
                ),
                (
                    "climbable",
                    v["climbable"].as_bool().unwrap() == p.has(PhysicsFlags::CLIMBABLE),
                ),
                (
                    "soul_speed_block",
                    v["soul_speed_block"].as_bool().unwrap() == p.has(PhysicsFlags::SOUL_SPEED),
                ),
                (
                    "collidable",
                    v["collidable"].as_bool().unwrap() == b.vanilla().collidable,
                ),
            ];
            for (k, ok) in checks {
                if !ok {
                    bad.push(format!("{name}: {k} differs (oracle {})", v[k]));
                }
            }
            // A collidable block with a model offset must be flagged as position-dependent.
            let offset_shape =
                b.vanilla().offset != OffsetType::None && b.states().any(|s| !s.shape().is_empty());
            if offset_shape != (b.shape_dependency() == ShapeDependency::PositionOffset) {
                bad.push(format!(
                    "{name}: offset {:?} vs dependency {:?}",
                    b.vanilla().offset,
                    b.shape_dependency()
                ));
            }
            continue;
        }
        states += 1;
        let s = BlockState::from_id(v["id"].as_u64().unwrap() as u16).unwrap();
        let name = v["state"].as_str().unwrap();
        assert_eq!(s.to_string(), name);
        if table_boxes(s, [0, 0, 0]) != oracle_boxes(&v["boxes"]) {
            bad.push(format!("{name}: collision boxes differ"));
        }
        if v["full_cube"].as_bool().unwrap() != (s.shape() == [ghast_data::Aabb::FULL]) {
            bad.push(format!(
                "{name}: full_cube {} vs table box list",
                v["full_cube"]
            ));
        }
        let fluid = s.fluid();
        let kind = fluid.map(|f| match f.kind {
            FluidKind::Water => "water",
            FluidKind::Lava => "lava",
        });
        if v["fluid"].as_str() != kind {
            bad.push(format!("{name}: fluid {} vs {kind:?}", v["fluid"]));
        } else if let Some(f) = fluid
            && (v["fluid_level"].as_u64() != Some(u64::from(f.amount()))
                || v["fluid_still"].as_bool() != Some(f.still())
                || f32_bits(&v["fluid_height"]) != f.height().to_bits())
        {
            bad.push(format!("{name}: fluid amount/still/height differ"));
        }
        if let Some(ctx) = v.get("contexts") {
            let sc = s
                .scaffolding_shapes()
                .unwrap_or_else(|| panic!("{name}: contexts but not scaffolding"));
            let as_bits = |b: &[ghast_data::Aabb]| {
                let mut o: Boxes = b
                    .iter()
                    .map(|a| {
                        [a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z].map(f64::to_bits)
                    })
                    .collect();
                o.sort_unstable();
                o
            };
            let want = |k: &str| oracle_boxes(&ctx[k]);
            if want("above") != as_bits(sc.stable)
                || want("inside") != as_bits(sc.bottom)
                || want("inside_descending") != as_bits(sc.bottom)
                || want("above_descending") != as_bits(sc.bottom)
                || !want("below").is_empty()
            {
                bad.push(format!("{name}: scaffolding context shapes differ"));
            }
        }
    }
    for m in bad.iter().take(100) {
        eprintln!("MISMATCH: {m}");
    }
    assert_eq!((states, blocks), (17112, 763));
    assert!(
        bad.is_empty(),
        "{} mismatches vs the 1.16.5 oracle",
        bad.len()
    );
}

#[test]
fn v1_16_5_far_positions_match_oracle() {
    let Some(lines) = lines("collision_shapes_far.jsonl.gz") else {
        return;
    };
    let mut bad = Vec::new();
    let mut offset_lines = 0;
    for v in &lines {
        assert_eq!(v["type"], "shape_abs");
        let s = BlockState::from_id(v["id"].as_u64().unwrap() as u16).unwrap();
        let pos: Vec<i32> = v["pos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_i64().unwrap() as i32)
            .collect();
        let pos = [pos[0], pos[1], pos[2]];
        if s.collision().dependency == ShapeDependency::PositionOffset {
            // The table holds the un-offset shape; vanilla shifts it per position.
            offset_lines += usize::from(v["same_as_local"] == false);
            continue;
        }
        if v["same_as_local"] != true || table_boxes(s, pos) != oracle_boxes(&v["boxes"]) {
            bad.push(format!("{s} at {pos:?}"));
        }
    }
    for m in bad.iter().take(50) {
        eprintln!("MISMATCH: {m}");
    }
    eprintln!(
        "far lines: {}, position-offset (bamboo) lines differing: {offset_lines}",
        lines.len()
    );
    assert!(bad.is_empty(), "{} far-position mismatches", bad.len());
}

#[test]
fn v1_16_5_voxels_match_oracle() {
    use ghast_data::v1_16_5::{Side, VoxelClass};
    let Some(lines) = lines("voxel_shapes.jsonl.gz") else {
        return;
    };
    let mut shapes: Vec<Value> = Vec::new();
    let contexts: Vec<(String, f64, i32, bool)> = lines
        .iter()
        .filter(|v| v["type"] == "context")
        .map(|v| {
            (
                v["name"].as_str().unwrap().to_owned(),
                f64::from_bits(bits(&v["feet_y"])),
                v["block_y"].as_i64().unwrap() as i32,
                v["descending"].as_bool().unwrap(),
            )
        })
        .collect();
    assert!(contexts.len() >= 35);
    let mut bad = Vec::new();
    let mut states = 0;
    for v in &lines {
        match v["type"].as_str().unwrap() {
            "voxel_shape" => shapes.push(v.clone()),
            "block" => {
                let name = v["name"].as_str().unwrap().trim_start_matches("minecraft:");
                let b = Block::from_name(name).unwrap();
                let tags = [
                    ("climbable", PhysicsFlags::CLIMBABLE),
                    ("fences", PhysicsFlags::FENCE),
                    ("walls", PhysicsFlags::WALL),
                    ("fence_gates", PhysicsFlags::FENCE_GATE),
                    ("trapdoors", PhysicsFlags::TRAPDOOR),
                    ("soul_speed_blocks", PhysicsFlags::SOUL_SPEED),
                    ("fire", PhysicsFlags::FIRE),
                ];
                for (tag, flag) in tags {
                    if v["tags"][tag].as_bool().unwrap() != b.physics().has(flag) {
                        bad.push(format!("{name}: tag {tag}"));
                    }
                }
            }
            "state" => {
                states += 1;
                let s = BlockState::from_id(v["id"].as_u64().unwrap() as u16).unwrap();
                let name = v["state"].as_str().unwrap();
                assert_eq!(s.to_string(), name);
                let o = &shapes[v["collision_shape"].as_u64().unwrap() as usize];
                let vs = s.voxel_shape();
                let pts =
                    |k: &str| -> Vec<u64> { o[k].as_array().unwrap().iter().map(bits).collect() };
                let ours = |p: &[f64]| -> Vec<u64> { p.iter().map(|x| x.to_bits()).collect() };
                let sizes: Vec<u64> = o["sizes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_u64().unwrap())
                    .collect();
                let filled: String = (0..usize::from(vs.sizes[0]))
                    .flat_map(|x| (0..usize::from(vs.sizes[1])).map(move |y| (x, y)))
                    .flat_map(|(x, y)| (0..usize::from(vs.sizes[2])).map(move |z| (x, y, z)))
                    .map(|(x, y, z)| if vs.filled(x, y, z) { '1' } else { '0' })
                    .collect();
                let class = match vs.class {
                    VoxelClass::Simple => "simple",
                    VoxelClass::Array => "array",
                };
                if pts("points_x") != ours(vs.xs)
                    || pts("points_y") != ours(vs.ys)
                    || pts("points_z") != ours(vs.zs)
                    || sizes != vs.sizes.map(u64::from)
                    || o["filled"].as_str().unwrap() != filled
                    || o["class"].as_str().unwrap() != class
                    || o["empty"].as_bool().unwrap() != vs.empty
                {
                    bad.push(format!("{name}: voxel shape differs"));
                }
                // `shape()` keeps vanilla's getBoundingBoxes order (auto-jump iterates it).
                let ordered: Vec<u64> = o["boxes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|b| b.as_array().unwrap().iter().map(bits))
                    .collect();
                let ours_ordered: Vec<u64> = s
                    .shape()
                    .iter()
                    .flat_map(|a| {
                        [a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z].map(f64::to_bits)
                    })
                    .collect();
                if ordered != ours_ordered {
                    bad.push(format!("{name}: box order differs from getBoundingBoxes"));
                }
                // The voxel shape's boxes are the same shape as the box-list table.
                if oracle_boxes(&o["boxes"]) != table_boxes(s, [0, 0, 0]) {
                    bad.push(format!("{name}: voxel boxes differ from the box table"));
                }
                let sides = v["side_solid_full_square"].as_array().unwrap();
                let flags = [
                    ("full_cube_identity", s.full_cube_identity()),
                    ("exceeds_cube", s.exceeds_cube()),
                    ("should_suffocate", s.should_suffocate()),
                    ("is_air", s.is_air()),
                    ("has_dynamic_bounds", s.has_dynamic_bounds()),
                    ("material_blocks_movement", s.material_blocks_movement()),
                    ("empty_identity", s.empty_identity()),
                    ("material_liquid", s.material_liquid()),
                    ("material_solid", s.material_solid()),
                    ("material_replaceable", s.material_replaceable()),
                    ("side_solid_up", s.side_solid_full_square(Side::Up)),
                    ("side_solid_down", s.side_solid_full_square(Side::Down)),
                ];
                for (k, ours) in flags {
                    if v[k].as_bool().unwrap() != ours {
                        bad.push(format!("{name}: {k}"));
                    }
                }
                for (i, side) in [Side::North, Side::East, Side::South, Side::West]
                    .into_iter()
                    .enumerate()
                {
                    if sides[i].as_bool().unwrap() != s.side_solid_full_square(side) {
                        bad.push(format!("{name}: side_solid_full_square {side:?}"));
                    }
                }
                if v["material"].as_str().unwrap().replace('_', "")
                    != format!("{:?}", s.block().material()).to_lowercase()
                {
                    bad.push(format!(
                        "{name}: material {} vs {:?}",
                        v["material"],
                        s.block().material()
                    ));
                }
                let f = &v["fluid"];
                match s.fluid() {
                    None => {
                        if !f.is_null() {
                            bad.push(format!("{name}: fluid missing"));
                        }
                    }
                    Some(fl) => {
                        if f["level"].as_u64() != Some(u64::from(fl.amount()))
                            || f["falling"].as_bool() != Some(fl.falling())
                            || f["still"].as_bool() != Some(fl.still())
                            || f32_bits(&f["height"]) != fl.height().to_bits()
                            || f32_bits(&f["height_fluid_above"]) != 1.0_f32.to_bits()
                        {
                            bad.push(format!("{name}: fluid differs"));
                        }
                    }
                }
                if let Some(ctx) = v.get("context_shapes") {
                    let sc = s.scaffolding_shapes().unwrap();
                    for (cname, feet, block_y, descending) in &contexts {
                        let o = &shapes[ctx[cname.as_str()].as_u64().unwrap() as usize];
                        let (boxes, vs) = sc.select(*feet, *block_y, *descending);
                        let same = o["points_y"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(bits)
                            .collect::<Vec<_>>()
                            == ours(vs.ys)
                            && o["points_x"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(bits)
                                .collect::<Vec<_>>()
                                == ours(vs.xs)
                            && o["empty"].as_bool().unwrap() == vs.empty
                            && o["boxes"].as_array().unwrap().len() == boxes.len();
                        if !same {
                            bad.push(format!(
                                "{name}: scaffolding context {cname} differs from select()"
                            ));
                        }
                    }
                }
            }
            "context" => {}
            t => panic!("unknown line type {t}"),
        }
    }
    for m in bad.iter().take(100) {
        eprintln!("MISMATCH: {m}");
    }
    assert_eq!(states, 17112);
    assert!(bad.is_empty(), "{} voxel-dump mismatches", bad.len());
}
