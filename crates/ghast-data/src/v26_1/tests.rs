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

fn golden_26_1(name: &str) -> Vec<serde_json::Value> {
    use std::io::{BufRead, BufReader};
    let path = format!(
        "{}/../../tests/golden/26.1/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let f = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect()
}

fn f32_hex(v: &serde_json::Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
}

/// Every item's destroy speed and "correct for drops" on every block, as the jar's
/// `ItemStack.getDestroySpeed` / `isCorrectToolForDrops` gave them (oracle item dump).
#[test]
fn tool_rules_match_the_jar() {
    let blocks: Vec<Block> = (0..BLOCK_COUNT as u16)
        .map(|i| Block::from_id(i).unwrap())
        .collect();
    let mut n = 0;
    for v in golden_26_1("items.jsonl.gz") {
        match v["type"].as_str().unwrap() {
            "item" => {
                let item = Item::from_id(v["id"].as_u64().unwrap() as u16).unwrap();
                assert_eq!(item.name(), v["name"].as_str().unwrap());
                assert_eq!(Item::from_name(item.name()), Some(item));
                let mut want = vec![1.0f32; BLOCK_COUNT];
                for s in v["speed_blocks"].as_array().unwrap() {
                    for b in s["blocks"].as_array().unwrap() {
                        want[b.as_u64().unwrap() as usize] = f32_hex(&s["speed"]);
                    }
                }
                let correct: std::collections::BTreeSet<u64> = v["correct_blocks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_u64().unwrap())
                    .collect();
                for &b in &blocks {
                    assert_eq!(
                        item.destroy_speed(b).to_bits(),
                        want[b.id() as usize].to_bits(),
                        "{} on {}",
                        item.name(),
                        b.name()
                    );
                    assert_eq!(
                        item.correct_for_drops(b),
                        correct.contains(&u64::from(b.id())),
                        "{} on {}",
                        item.name(),
                        b.name()
                    );
                }
                n += 1;
            }
            "block" => {
                let b = Block::from_id(v["id"].as_u64().unwrap() as u16).unwrap();
                assert_eq!(
                    b.destroy_time().to_bits(),
                    f32_hex(&v["destroy_time"]).to_bits(),
                    "{}",
                    b.name()
                );
                assert_eq!(
                    b.requires_correct_tool_for_drops(),
                    v["requires_correct_tool"].as_bool().unwrap()
                );
                assert_eq!(b.item().name(), v["item"].as_str().unwrap());
            }
            _ => {}
        }
    }
    assert_eq!(n, Item::all().len());
}

/// Default components against Mojang's item component report (26.1.2 server `--reports`), when
/// the git-ignored cache is present: stack size, damage, food, tool rules.
#[test]
fn items_match_the_mojang_report() {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.cache/mojang/26.1.2/reports/reports/minecraft/components/item"
    );
    if std::fs::metadata(dir).is_err() {
        eprintln!("skipped: {dir} not present");
        return;
    }
    for item in Item::all() {
        if item == Item::AIR {
            continue;
        }
        let text = std::fs::read_to_string(format!("{dir}/{}.json", item.name()))
            .unwrap_or_else(|e| panic!("{}: {e}", item.name()));
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let c = &v["components"];
        assert_eq!(
            c["minecraft:max_stack_size"].as_u64(),
            Some(u64::from(item.max_stack_size())),
            "{}",
            item.name()
        );
        assert_eq!(
            c["minecraft:max_damage"].as_u64(),
            item.data().max_damage.map(u64::from),
            "{}",
            item.name()
        );
        match (item.food(), c.get("minecraft:food")) {
            (None, None) => {}
            (Some(f), Some(r)) => {
                assert_eq!(
                    r["nutrition"].as_u64(),
                    Some(u64::from(f.nutrition)),
                    "{}",
                    item.name()
                );
                assert_eq!(
                    r["saturation"].as_f64().unwrap() as f32,
                    f.saturation,
                    "{}",
                    item.name()
                );
                assert_eq!(
                    r["can_always_eat"].as_bool().unwrap_or(false),
                    f.can_always_eat,
                    "{}",
                    item.name()
                );
            }
            (a, b) => panic!("{}: food {a:?} vs report {b:?}", item.name()),
        }
        match (item.tool(), c.get("minecraft:tool")) {
            (None, None) => {}
            (Some(t), Some(r)) => {
                let rules = r["rules"].as_array().unwrap();
                assert_eq!(rules.len(), t.rules.len(), "{}", item.name());
                for (rule, want) in t.rules.iter().zip(rules) {
                    // "#tag", one block name, or a list of block names.
                    match want["blocks"].as_str() {
                        Some(tag) if tag.starts_with('#') => {
                            assert_eq!(rule.blocks.tag(), Some(&tag[1..]), "{}", item.name());
                        }
                        _ => {
                            assert!(rule.blocks.tag().is_none(), "{}", item.name());
                            let names: Vec<&str> = match want["blocks"].as_array() {
                                Some(a) => a.iter().map(|x| x.as_str().unwrap()).collect(),
                                None => vec![want["blocks"].as_str().unwrap()],
                            };
                            let mut members: Vec<&str> = (0..BLOCK_COUNT as u16)
                                .map(|i| Block::from_id(i).unwrap())
                                .filter(|b| rule.blocks.contains(*b))
                                .map(Block::name)
                                .collect();
                            let mut want_names: Vec<&str> = names
                                .iter()
                                .map(|n| n.trim_start_matches("minecraft:"))
                                .collect();
                            members.sort_unstable();
                            want_names.sort_unstable();
                            assert_eq!(members, want_names, "{}", item.name());
                        }
                    }
                    assert_eq!(
                        want["speed"].as_f64().map(|x| x as f32),
                        rule.speed,
                        "{}",
                        item.name()
                    );
                    assert_eq!(
                        want["correct_for_drops"].as_bool(),
                        rule.correct_for_drops,
                        "{}",
                        item.name()
                    );
                }
                assert_eq!(
                    r["default_mining_speed"].as_f64().unwrap_or(1.0) as f32,
                    t.default_mining_speed
                );
                assert_eq!(
                    r["damage_per_block"].as_u64().unwrap_or(1),
                    u64::from(t.damage_per_block)
                );
                assert_eq!(
                    r["can_destroy_blocks_in_creative"]
                        .as_bool()
                        .unwrap_or(true),
                    t.can_destroy_blocks_in_creative
                );
            }
            (a, b) => panic!("{}: tool {a:?} vs report {b:?}", item.name()),
        }
    }
}

/// The jar's destroy times against minecraft-data's hardness.
#[test]
fn destroy_time_matches_minecraft_data_hardness() {
    let mut diffs = Vec::new();
    for b in (0..BLOCK_COUNT as u16).map(|i| Block::from_id(i).unwrap()) {
        let md = b.data().hardness.unwrap_or(-1.0);
        if md != b.destroy_time() {
            diffs.push(format!("{} {md} vs {}", b.name(), b.destroy_time()));
        }
    }
    assert!(diffs.is_empty(), "{diffs:?}");
}

/// Block breaking against the real client (`MultiPlayerGameMode` driven tick by tick, oracle
/// `dig.jsonl.gz`): destroy speed, correct-tool flag and per-tick progress bit for bit, and the
/// call on which `STOP_DESTROY_BLOCK` went out (second digs right after a break: 5-tick delay).
#[test]
fn dig_matches_the_client() {
    use super::dig::{BreakTime, DigConditions, break_time, destroy_progress_with, destroy_speed};
    const LIMIT: u32 = 2000;
    let f64_hex = |v: &serde_json::Value| {
        f64::from_bits(u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
    };
    let amp = |v: &serde_json::Value| Some(v.as_i64().unwrap() as i32).filter(|a| *a >= 0);
    let (mut n, mut timed) = (0, 0);
    for v in golden_26_1("dig.jsonl.gz") {
        let state = BlockState::from_id(v["state"].as_u64().unwrap() as u16).unwrap();
        assert_eq!(state.to_string(), v["name"].as_str().unwrap());
        let held = v["held"].as_str().map(|n| Item::from_name(n).unwrap());
        let c = DigConditions {
            mining_efficiency: f64_hex(&v["mining_efficiency"]),
            block_break_speed: f64_hex(&v["block_break_speed"]),
            submerged_mining_speed: f64_hex(&v["submerged_mining_speed"]),
            haste: amp(&v["haste"]),
            conduit_power: amp(&v["conduit_power"]),
            mining_fatigue: amp(&v["mining_fatigue"]),
            eye_in_water: v["eye_in_water"].as_bool().unwrap(),
            on_ground: v["on_ground"].as_bool().unwrap(),
        };
        let k = v["k"].as_u64().unwrap();
        let b = state.block();
        let tool_speed = held.map_or(1.0, |i| i.destroy_speed(b));
        let correct = held.is_some_and(|i| i.correct_for_drops(b));
        assert_eq!(
            destroy_speed(tool_speed, &c).to_bits(),
            f32_hex(&v["destroy_speed"]).to_bits(),
            "case {k}"
        );
        assert_eq!(
            !state.requires_correct_tool_for_drops() || correct,
            v["correct_tool"].as_bool().unwrap(),
            "case {k}"
        );
        let progress = destroy_progress_with(state, held, &c);
        assert_eq!(
            progress.to_bits(),
            f32_hex(&v["progress"]).to_bits(),
            "case {k}"
        );
        for (i, d) in v["first"].as_array().unwrap().iter().enumerate() {
            let calls = d["calls"].as_u64().unwrap() as u32;
            let got = d["result"].as_str().unwrap();
            if v["creative"].as_bool().unwrap() {
                assert_eq!(got, "instant", "case {k}: creative");
                continue;
            }
            let want = match got {
                "instant" => BreakTime::Instant,
                "ticks" => BreakTime::Ticks(calls),
                "none" => BreakTime::Never,
                r => panic!("case {k}: result {r}"),
            };
            let delay = if i == 0 { 0 } else { 5 };
            assert_eq!(break_time(progress, delay, LIMIT), want, "case {k} dig {i}");
            timed += 1;
        }
        n += 1;
    }
    assert!(n >= 3000 && timed > 3000, "{n} cases, {timed} digs");
}

/// The named entity ids are this version's (resolved from the generated table, not literals).
#[test]
fn named_entity_ids_match_their_names() {
    use super::entities::{ARMOR_STAND, EXPERIENCE_ORB, ITEM, PAINTING, PLAYER};
    for (id, name) in [
        (PLAYER, "player"),
        (ITEM, "item"),
        (EXPERIENCE_ORB, "experience_orb"),
        (PAINTING, "painting"),
        (ARMOR_STAND, "armor_stand"),
    ] {
        assert_eq!(super::entity(id).map(|e| e.name), Some(name), "{id}");
    }
}
