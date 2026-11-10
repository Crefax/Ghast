use super::*;

#[test]
fn state_strings_round_trip() {
    for id in 0..STATE_COUNT as u16 {
        let s = BlockState::from_id(id).unwrap();
        let text = s.to_string();
        assert_eq!(BlockState::parse(&text), Some(s), "{text}");
    }
}

/// Every state's text is the jar's (`BlockState.toString`), including int properties that do
/// not start at 0 (leaves' distance, snow layers).
#[test]
fn state_strings_match_the_jar() {
    use std::io::{BufRead, BufReader};
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/golden/26.1/voxel_shapes.jsonl.gz"
    );
    let f = std::fs::File::open(path).unwrap();
    let mut n = 0;
    for l in BufReader::new(flate2::read::GzDecoder::new(f)).lines() {
        let v: serde_json::Value = serde_json::from_str(&l.unwrap()).unwrap();
        if v["type"] == "state" {
            let s = BlockState::from_id(v["id"].as_u64().unwrap() as u16).unwrap();
            assert_eq!(s.to_string(), v["state"].as_str().unwrap());
            n += 1;
        }
    }
    assert_eq!(n, STATE_COUNT);
}

/// Every state's property map equals Mojang's `blocks.json` report (`--reports` of the 26.1.2
/// server, same protocol and states), when the git-ignored cache is present.
#[test]
fn properties_match_the_mojang_report() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.cache/mojang/26.1.2/reports/reports/blocks.json"
    );
    let Ok(text) = std::fs::read_to_string(path) else {
        eprintln!("skipped: {path} not present");
        return;
    };
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut n = 0;
    for (name, b) in v.as_object().unwrap() {
        for st in b["states"].as_array().unwrap() {
            let s = BlockState::from_id(st["id"].as_u64().unwrap() as u16).unwrap();
            assert_eq!(format!("minecraft:{}", s.block().name()), *name);
            let want: Vec<(String, String)> = st["properties"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
                        .collect()
                })
                .unwrap_or_default();
            let mut got: Vec<(String, String)> = s
                .properties()
                .map(|(p, v)| (p.name().to_owned(), v.to_owned()))
                .collect();
            let mut want = want;
            want.sort();
            got.sort();
            assert_eq!(got, want, "{name} state {}", s.id());
            n += 1;
        }
    }
    assert_eq!(n, STATE_COUNT);
}

/// Committed spot checks of integer properties that do not start at 0 (labels once ran from 0).
#[test]
fn one_based_int_properties() {
    let cases = [
        (
            "oak_leaves[distance=1,persistent=true,waterlogged=true]",
            "distance",
            "1",
        ),
        (
            "oak_leaves[distance=7,persistent=false,waterlogged=false]",
            "distance",
            "7",
        ),
        ("snow[layers=1]", "layers", "1"),
        ("snow[layers=8]", "layers", "8"),
        ("turtle_egg[eggs=4,hatch=2]", "eggs", "4"),
        ("sea_pickle[pickles=1,waterlogged=true]", "pickles", "1"),
        (
            "repeater[delay=4,facing=north,locked=false,powered=false]",
            "delay",
            "4",
        ),
        ("water_cauldron[level=3]", "level", "3"),
        (
            "candle[candles=4,lit=false,waterlogged=false]",
            "candles",
            "4",
        ),
        (
            "scaffolding[bottom=false,distance=0,waterlogged=false]",
            "distance",
            "0",
        ),
        ("water[level=0]", "level", "0"),
    ];
    for (text, prop, value) in cases {
        let s = BlockState::parse(&format!("minecraft:{text}")).unwrap_or_else(|| panic!("{text}"));
        assert_eq!(
            s.get(Property::from_name(prop).unwrap()),
            Some(value),
            "{text}"
        );
        assert_eq!(format!("minecraft:{text}"), s.to_string());
    }
    // State ids are unchanged by the labels: the first snow state is still layers=1.
    assert_eq!(
        BlockState::parse("minecraft:snow[layers=1]").unwrap(),
        Block::from_name("snow").unwrap().states().next().unwrap()
    );
}

#[test]
fn packed_records_are_consistent() {
    let air = BlockState::AIR.phys();
    assert!(air.has(state_flags::AIR | state_flags::EMPTY));
    let stone = Block::STONE.default_state().phys();
    assert!(stone.has(state_flags::FULL_CUBE | state_flags::SUFFOCATING));
    assert_eq!(stone.shape(), FULL_CUBE_VOXEL);
    let water = Block::WATER.default_state().phys();
    assert_eq!(water.fluid() >> 6, 1);
    assert_eq!(water.fluid() & 15, 8);
    assert_eq!(water.kind(), BlockKind::Water as u8);
    assert_eq!(Block::ICE.data().friction, 0.98);
    assert_eq!(Block::SOUL_SAND.data().speed_factor, 0.4);
    assert_eq!(Block::HONEY_BLOCK.data().jump_factor, 0.5);
    assert!(Block::LADDER.data().flags & block_flags::CLIMBABLE != 0);
}

/// The client push-out (`moveTowardsClosestSpace`) only reads the cells inside its probe column:
/// the face / edge cells of vanilla's cursor count only for suffocating large-shape (or moving
/// piston) states, and every such state's shape stays inside its own cube, so it can never reach
/// the probe area. If a future data set breaks this, the physics shortcut must go.
#[test]
fn suffocating_large_shapes_stay_in_their_cube() {
    for id in 0..STATE_COUNT as u16 {
        let s = BlockState::from_id(id).unwrap();
        let p = s.phys();
        if !p.has(state_flags::SUFFOCATING) {
            continue;
        }
        if p.has(state_flags::LARGE) || p.has(state_flags::MOVING_PISTON) {
            assert!(
                !p.has(state_flags::CONTEXT) && !p.has(state_flags::OFFSET),
                "{s}"
            );
            for b in s.voxel_shape().boxes {
                assert!(
                    b.min_x >= 0.0
                        && b.min_y >= 0.0
                        && b.min_z >= 0.0
                        && b.max_x <= 1.0
                        && b.max_y <= 1.0
                        && b.max_z <= 1.0,
                    "{s}: {b:?}"
                );
            }
        }
    }
}
