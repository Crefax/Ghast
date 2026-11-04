//! Entity sizes vs the oracle dumps (`tests/golden/<ver>/entity_sizes.jsonl.gz`): every measured
//! variant, the vanilla bounding box at the dump's feet position, targeting margins, 1.16 poses and
//! registered dimensions, and the slime size formula. Skipped if a file is missing.
#![cfg(any(feature = "v1_8", feature = "v1_16_5"))]

use ghast_data::{EntityDimensions, VanillaEntitySize};
use serde_json::Value;
use std::io::Read as _;
use std::path::PathBuf;

fn lines(ver: &str) -> Option<Vec<Value>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../tests/golden/{ver}/entity_sizes.jsonl.gz"));
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

fn f32_of(v: &Value) -> u32 {
    u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap()
}

fn f64_of(v: &Value) -> u64 {
    u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap()
}

fn bits(d: EntityDimensions) -> [u32; 3] {
    [
        d.width.to_bits(),
        d.height.to_bits(),
        d.eye_height.to_bits(),
    ]
}

fn check(
    lines: &[Value],
    lookup: impl Fn(&str) -> Option<&'static VanillaEntitySize>,
    slime: impl Fn(i32) -> EntityDimensions,
) {
    let mut bad = Vec::new();
    let mut checked = 0;
    for v in lines {
        let name = v["name"]
            .as_str()
            .unwrap()
            .trim_start_matches("minecraft:")
            .trim_end_matches(" (client player)");
        if v["type"] == "entity_type" {
            let e = lookup(name).unwrap_or_else(|| panic!("{name} missing"));
            let (w, h) = e.registered.unwrap();
            if (w.to_bits(), h.to_bits()) != (f32_of(&v["width"]), f32_of(&v["height"]))
                || e.fixed != v["fixed"].as_bool().unwrap()
            {
                bad.push(format!("{name}: registered dimensions"));
            }
            continue;
        }
        if v.get("error").is_some() {
            continue;
        }
        checked += 1;
        let variant = v["variant"].as_str().unwrap();
        let e = lookup(name).unwrap_or_else(|| panic!("{name} missing"));
        let Some(var) = e.variant(variant) else {
            bad.push(format!("{name} {variant}: missing"));
            continue;
        };
        let d = var.dimensions;
        if bits(d)
            != [
                f32_of(&v["width"]),
                f32_of(&v["height"]),
                f32_of(&v["eye_height"]),
            ]
        {
            bad.push(format!("{name} {variant}: dimensions"));
        }
        // The centred spawn box. Not for hanging entities (box from facing / motive), not after a
        // resize (vanilla keeps the old min corner until the next move), and not for the two 1.8
        // projectiles that spawn with the fireball box and shrink afterwards.
        let hanging = v.get("facing").is_some()
            || matches!(name, "LeashKnot" | "leash_knot" | "item_frame" | "painting");
        let shrunk = matches!(name, "SmallFireball" | "WitherSkull");
        if variant == "default" && !hanging && !shrunk {
            let pos: Vec<f64> = match v.get("pos") {
                Some(p) => p
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| f64::from_bits(f64_of(x)))
                    .collect(),
                // 1.16 lines have no "pos": feet at x = z = 0.5 and y = the box bottom (64, or 100
                // for the client player).
                None => vec![0.5, f64::from_bits(f64_of(&v["box"][1])), 0.5],
            };
            let b = d.bounding_box(pos[0], pos[1], pos[2]);
            let want: Vec<u64> = v["box"].as_array().unwrap().iter().map(f64_of).collect();
            if [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]
                .map(f64::to_bits)
                .to_vec()
                != want
            {
                bad.push(format!("{name} {variant}: bounding box"));
            }
        }
        let margin = v.get("collision_border").unwrap_or(&v["targeting_margin"]);
        if e.targeting_margin.to_bits() != f32_of(margin) {
            bad.push(format!("{name}: targeting margin"));
        }
        if let Some(poses) = v
            .get("poses")
            .and_then(Value::as_object)
            .filter(|_| variant == "default")
        {
            for (pose, p) in poses {
                let p = p.as_array().unwrap();
                if e.pose(pose).map(bits) != Some([f32_of(&p[0]), f32_of(&p[1]), f32_of(&p[2])]) {
                    bad.push(format!("{name}: pose {pose}"));
                }
            }
        }
        if let Some(n) = variant.strip_prefix("size_")
            && bits(slime(n.parse().unwrap())) != bits(d)
        {
            bad.push(format!("{name} {variant}: slime_dimensions formula"));
        }
    }
    for m in &bad {
        eprintln!("MISMATCH: {m}");
    }
    assert!(checked > 50);
    assert!(bad.is_empty(), "{} entity size mismatches", bad.len());
}

#[cfg(feature = "v1_8")]
#[test]
fn v1_8_9_entity_sizes_match_oracle() {
    use ghast_data::v1_8;
    let Some(l) = lines("1.8.9") else { return };
    check(&l, v1_8::vanilla_entity_size, v1_8::slime_dimensions);
    let zombie = v1_8::vanilla_entity_size("Zombie").unwrap();
    // The 1.8.9 client never resizes baby zombies; the server-side size is listed separately.
    assert_eq!(zombie.baby().unwrap().height, 1.95);
    assert_eq!(
        zombie.variant("baby_server").unwrap().dimensions.height,
        0.975
    );
    assert_eq!(
        v1_8::vanilla_entity_size("Skeleton")
            .unwrap()
            .variant("wither")
            .unwrap()
            .dimensions
            .width,
        0.72
    );
    assert_eq!(v1_8::vanilla_entity_size("Pig").unwrap().registry_id, 90);
    assert_eq!(v1_8::entity_by_name("Zombie").unwrap().height, Some(1.95));
}

#[cfg(feature = "v1_16_5")]
#[test]
fn v1_16_5_entity_sizes_match_oracle() {
    use ghast_data::v1_16_5;
    let Some(l) = lines("1.16.5") else { return };
    check(&l, v1_16_5::vanilla_entity_size, v1_16_5::slime_dimensions);
    let player = v1_16_5::vanilla_entity_size("player").unwrap();
    assert_eq!(player.pose("crouching").unwrap().height, 1.5);
    assert_eq!(
        v1_16_5::entity_by_name("horse").unwrap().width,
        Some(1.396_484_4)
    );
}
