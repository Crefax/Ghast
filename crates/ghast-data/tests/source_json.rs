//! Generated tables vs the minecraft-data JSON they came from, re-read independently of the
//! generator. Needs the git-ignored cache (`cargo run -p ghast-codegen-data` fills it); the tests
//! print a notice and pass when it is missing so plain `cargo test` works offline.
#![cfg(any(feature = "v1_8", feature = "v1_16_5"))]

use serde_json::Value;
use std::path::{Path, PathBuf};

fn data_root() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.cache/minecraft-data/3.119.0/package/minecraft-data/data");
    if p.join("dataPaths.json").is_file() {
        Some(p)
    } else {
        eprintln!("skipped: minecraft-data cache missing at {}", p.display());
        None
    }
}

fn json(root: &Path, version: &str, kind: &str) -> Value {
    let dp: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("dataPaths.json")).unwrap())
            .unwrap();
    let dir = dp["pc"][version][kind].as_str().unwrap();
    serde_json::from_str(
        &std::fs::read_to_string(root.join(dir).join(format!("{kind}.json"))).unwrap(),
    )
    .unwrap()
}

fn boxes_of(shapes: &Value, id: &Value) -> Vec<[f64; 6]> {
    shapes[id.as_u64().unwrap().to_string()]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            let v: Vec<f64> = b
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect();
            [v[0], v[1], v[2], v[3], v[4], v[5]]
        })
        .collect()
}

fn to_arrays(a: &[ghast_data::Aabb]) -> Vec<[f64; 6]> {
    a.iter()
        .map(|a| [a.min_x, a.min_y, a.min_z, a.max_x, a.max_y, a.max_z])
        .collect()
}

fn check_info(info: &ghast_data::BlockInfo, b: &Value) {
    let name = b["name"].as_str().unwrap();
    assert_eq!(info.name, name);
    assert_eq!(
        info.display_name,
        b["displayName"].as_str().unwrap(),
        "{name}"
    );
    assert_eq!(
        info.hardness,
        b["hardness"].as_f64().map(|h| h as f32),
        "{name}"
    );
    assert_eq!(
        info.resistance,
        b["resistance"].as_f64().unwrap() as f32,
        "{name}"
    );
    assert_eq!(
        u64::from(info.stack_size),
        b["stackSize"].as_u64().unwrap(),
        "{name}"
    );
    assert_eq!(info.diggable, b["diggable"].as_bool().unwrap(), "{name}");
    assert_eq!(
        info.transparent,
        b["transparent"].as_bool().unwrap(),
        "{name}"
    );
    assert_eq!(
        u64::from(info.emit_light),
        b["emitLight"].as_u64().unwrap(),
        "{name}"
    );
    assert_eq!(
        u64::from(info.filter_light),
        b["filterLight"].as_u64().unwrap(),
        "{name}"
    );
    let tools: Vec<u16> = b
        .get("harvestTools")
        .and_then(Value::as_object)
        .map(|m| {
            let mut v: Vec<u16> = m.keys().map(|k| k.parse().unwrap()).collect();
            v.sort_unstable();
            v
        })
        .unwrap_or_default();
    assert_eq!(info.harvest_tools, tools.as_slice(), "{name}");
    assert_eq!(
        info.drops.len(),
        b["drops"].as_array().map_or(0, Vec::len),
        "{name}"
    );
}

/// Blocks with entries in `facts::V1_8_SHAPE_OVERRIDES`.
#[cfg(feature = "v1_8")]
const V1_8_OVERRIDDEN: &[&str] = &[
    "farmland",
    "ladder",
    "snow_layer",
    "waterlily",
    "piston_head",
    "glass_pane",
    "stained_glass_pane",
    "iron_bars",
    "wooden_door",
    "iron_door",
    "spruce_door",
    "birch_door",
    "jungle_door",
    "acacia_door",
    "dark_oak_door",
];

#[cfg(feature = "v1_8")]
#[test]
fn v1_8_matches_source() {
    use ghast_data::v1_8::{self, Block, BlockState, Item};
    let Some(root) = data_root() else { return };
    let blocks = json(&root, "1.8", "blocks");
    let shapes = json(&root, "1.8", "blockCollisionShapes");
    let blocks = blocks.as_array().unwrap();
    assert_eq!(blocks.len(), v1_8::BLOCK_COUNT);
    let mut states = 0;
    for b in blocks {
        let block = Block::from_id(b["id"].as_u64().unwrap() as u8).unwrap();
        check_info(block.info(), b);
        let entry = &shapes["blocks"][block.name()];
        for meta in 0..16u8 {
            let id = entry.as_array().map_or(entry, |a| &a[usize::from(meta)]);
            let s = BlockState::new(block, meta);
            // Unregistered metas (empty in the table) and the oracle-sourced overrides in
            // codegen/data/src/facts.rs (V1_8_SHAPE_OVERRIDES) deliberately differ from the JSON.
            if !s.is_registered() || V1_8_OVERRIDDEN.contains(&block.name()) {
                states += 1;
                continue;
            }
            assert_eq!(
                to_arrays(s.shape()),
                boxes_of(&shapes["shapes"], id),
                "{}:{meta}",
                block.name()
            );
            states += 1;
        }
    }
    assert_eq!(states, 198 * 16);
    let items = json(&root, "1.8", "items");
    let items = items.as_array().unwrap();
    // + lit_furnace (facts::V1_8_ITEM_FIXES).
    assert_eq!(items.len() + 1, Item::all().len());
    for i in items {
        let item = Item::from_id(i["id"].as_u64().unwrap() as u16).unwrap();
        let info = item.info();
        assert_eq!(info.name, i["name"].as_str().unwrap());
        assert_eq!(u64::from(info.stack_size), i["stackSize"].as_u64().unwrap());
        assert_eq!(
            info.max_durability.map(u64::from),
            i["maxDurability"].as_u64()
        );
        assert_eq!(
            info.variations.len(),
            i["variations"].as_array().map_or(0, Vec::len)
        );
    }
    let n = |k: &str| json(&root, "1.8", k).as_array().unwrap().len();
    assert_eq!(n("biomes"), v1_8::biomes().len());
    assert_eq!(n("enchantments"), v1_8::enchantments().len());
    assert_eq!(n("entities"), v1_8::entities().len());
    assert_eq!(n("effects"), v1_8::effects().len());
}

#[cfg(feature = "v1_16_5")]
#[test]
fn v1_16_5_matches_source() {
    use ghast_data::v1_16_5::{self, Block, Item};
    let Some(root) = data_root() else { return };
    let blocks = json(&root, "1.16.5", "blocks");
    let shapes = json(&root, "1.16.5", "blockCollisionShapes");
    let blocks = blocks.as_array().unwrap();
    assert_eq!(blocks.len(), v1_16_5::BLOCK_COUNT);
    let mut states = 0;
    for b in blocks {
        let block = Block::from_id(b["id"].as_u64().unwrap() as u16).unwrap();
        check_info(block.info(), b);
        let l = block.layout();
        assert_eq!(u64::from(l.min_state), b["minStateId"].as_u64().unwrap());
        assert_eq!(
            u64::from(l.min_state + l.state_count - 1),
            b["maxStateId"].as_u64().unwrap()
        );
        assert_eq!(
            u64::from(l.default_state),
            b["defaultState"].as_u64().unwrap()
        );
        let props: Vec<&str> = block.properties().map(|d| d.property.name()).collect();
        let src: Vec<&str> = b["states"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert_eq!(props, src, "{}", block.name());
        let entry = &shapes["blocks"][block.name()];
        for (i, s) in block.states().enumerate() {
            let id = entry.as_array().map_or(entry, |a| &a[i]);
            assert_eq!(to_arrays(s.shape()), boxes_of(&shapes["shapes"], id), "{s}");
            states += 1;
        }
    }
    assert_eq!(states, v1_16_5::STATE_COUNT);
    let items = json(&root, "1.16.5", "items");
    let items = items.as_array().unwrap();
    assert_eq!(items.len(), Item::all().len());
    for i in items {
        let item = Item::from_id(i["id"].as_u64().unwrap() as u16).unwrap();
        assert_eq!(item.info().name, i["name"].as_str().unwrap());
        if item.info().name == "warped_fungus_on_a_stick" {
            continue; // facts::V1_16_5_ITEM_FIXES (vanilla stack size 1)
        }
        assert_eq!(
            u64::from(item.info().stack_size),
            i["stackSize"].as_u64().unwrap()
        );
        assert_eq!(
            item.info().max_durability.map(u64::from),
            i["maxDurability"].as_u64()
        );
    }
    let n = |k: &str| json(&root, "1.16.5", k).as_array().unwrap().len();
    assert_eq!(n("biomes"), v1_16_5::biomes().len());
    assert_eq!(n("enchantments"), v1_16_5::enchantments().len());
    assert_eq!(n("entities"), v1_16_5::entities().len());
    assert_eq!(n("effects"), v1_16_5::effects().len());
}
