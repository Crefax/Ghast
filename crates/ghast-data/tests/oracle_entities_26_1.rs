//! 26.1 entity tables vs the oracle dump (`tests/golden/26.1/entity_sizes.jsonl.gz`): every type's
//! id, name, registered size and client facts, and for every measured variant and every pose the
//! dimensions `EntityTypeData::dimensions` computes, plus the bounding box at the dump's feet
//! position. Skipped if the file is missing.
#![cfg(feature = "v26_1")]

use ghast_data::EntityDimensions;
use ghast_data::v26_1::entities::{POSES, entity_type, type_flags};
use serde_json::Value;
use std::io::Read as _;
use std::path::PathBuf;

fn lines() -> Option<Vec<Value>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/26.1/entity_sizes.jsonl.gz");
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

fn hex32(v: &Value) -> u32 {
    u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap()
}

fn hex64(v: &Value) -> u64 {
    u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap()
}

fn bits(d: EntityDimensions) -> [u32; 3] {
    [d.width.to_bits(), d.height.to_bits(), d.eye_height.to_bits()]
}

/// Types whose client box is offset from the feet-centred dimensions box (custom `makeBoundingBox`).
const OFFSET_BOXES: &[&str] = &[];

#[test]
fn v26_1_entity_tables_match_oracle() {
    let Some(lines) = lines() else { return };
    let (mut types, mut checked) = (0, 0);
    let mut bad_boxes = Vec::new();
    for v in &lines {
        let id = v["id"].as_u64().unwrap() as u16;
        let name = v["name"].as_str().unwrap();
        let path = name
            .trim_start_matches("minecraft:")
            .trim_end_matches(" (client player)");
        let t = entity_type(id).unwrap_or_else(|| panic!("{name}: no type {id}"));
        assert_eq!(t.name, path);
        let info = ghast_data::v26_1::entity(id).unwrap();
        assert_eq!(info.name, path);
        match v["type"].as_str().unwrap() {
            "entity_type" => {
                types += 1;
                assert_eq!(info.width.unwrap().to_bits(), hex32(&v["width"]), "{name}");
                assert_eq!(info.height.unwrap().to_bits(), hex32(&v["height"]), "{name}");
                assert_eq!(t.flags & type_flags::FIXED != 0, v["fixed"].as_bool().unwrap());
                if let Some(s) = t.size {
                    let (w, h) = s.registered.unwrap();
                    assert_eq!((w.to_bits(), h.to_bits()), (hex32(&v["width"]), hex32(&v["height"])));
                }
            }
            "entity_size" => {
                let variant = v["variant"].as_str().unwrap();
                if variant == "default" {
                    assert_eq!(t.is_living(), v["living"].as_bool().unwrap(), "{name}");
                    assert_eq!(t.is_pickable(), v["pickable"].as_bool().unwrap(), "{name}");
                    let i = v["interpolation"].as_i64().unwrap();
                    assert_eq!(t.interpolation.map_or(-1, i64::from), i, "{name}");
                    let s = t.size.unwrap();
                    assert_eq!(s.targeting_margin.to_bits(), hex32(&v["targeting_margin"]));
                    let mob = v["mob"].as_bool().unwrap_or(false);
                    assert_eq!(t.flags & type_flags::MOB != 0, mob, "{name}");
                    if mob {
                        assert_eq!(u64::from(t.max_head_y_rot), v["max_head_y_rot"].as_u64().unwrap());
                        let default = v["body_control"] == "BodyRotationControl";
                        assert_eq!(t.flags & type_flags::DEFAULT_BODY != 0, default, "{name}");
                    }
                }
                if t.flags & type_flags::NO_BOX != 0 {
                    assert!(t.dimensions(variant, None, 0).is_none());
                    continue;
                }
                let slime = variant
                    .strip_prefix("size_")
                    .map(|n| n.parse::<i32>().unwrap());
                let var = if slime.is_some() { "default" } else { variant };
                if slime.is_some() {
                    assert!(t.size_meta.is_some(), "{name}: size index");
                }
                if variant == "baby" && t.baby_meta.is_none() {
                    // setBaby is a no-op for this type (frog, parrot, ...): same size as the default.
                    let s = t.size.unwrap();
                    assert_eq!(s.baby(), s.default_dimensions(), "{name}: baby index");
                }
                for (pi, pose) in POSES.iter().enumerate() {
                    let want = &v["poses"][*pose];
                    let want = [hex32(&want[0]), hex32(&want[1]), hex32(&want[2])];
                    let got = t.dimensions(var, slime, pi as i32).unwrap();
                    assert_eq!(bits(got), want, "{name} {variant} {pose}");
                    checked += 1;
                }
                // The current pose (standing) and the box at the dump's feet position (0.5, 64, 0.5).
                let d = t.dimensions(var, slime, 0).unwrap();
                assert_eq!(
                    bits(d),
                    [hex32(&v["width"]), hex32(&v["height"]), hex32(&v["eye_height"])],
                    "{name} {variant}"
                );
                let b = d.bounding_box(0.5, 64.0, 0.5);
                let want: Vec<u64> = v["box"].as_array().unwrap().iter().map(hex64).collect();
                let got = [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z].map(f64::to_bits);
                if name.ends_with("(client player)") {
                    // The client player's box sits wherever the harness spawned it.
                    continue;
                }
                if got.to_vec() != want {
                    bad_boxes.push(format!("{path} {variant}"));
                }
            }
            other => panic!("unknown line {other}"),
        }
    }
    assert_eq!(bad_boxes, OFFSET_BOXES, "types whose box is not the centred dimensions box");
    assert_eq!(types, ghast_data::v26_1::entities().len());
    assert!(checked > 3000, "{checked}");
    // Slime sizes beyond the dump follow the same formula.
    let slime = entity_type(ghast_data::v26_1::entity_by_name("slime").unwrap().id).unwrap();
    let d = slime.dimensions("default", Some(10), 0).unwrap();
    assert_eq!(d.width, 0.52 * 10.0);
}
