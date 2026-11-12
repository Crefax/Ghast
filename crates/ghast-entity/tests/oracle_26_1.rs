//! The 26.1 tracker against the real client: replays `tests/golden/26.1/entity_tracking.jsonl.gz`
//! (oracle `EntityTrackDump`: random entity packets handled by the real `ClientPacketListener`,
//! entity state recorded at pick time every tick) and compares every entity bit for bit:
//! position, rotation, head rotation, box, on-ground, pickable, chunk ticking, riding links, item
//! counts, and the tab list. Each case runs with metadata tracking on and off.
//!
//! `ORACLE_TRACKING=<file>` replays another dump (e.g. a large one under `oracle/cache/26.1/`).

mod common;

use std::io::Read as _;
use std::path::PathBuf;

use bytes::Bytes;
use common::W;
use ghast_entity::v26_1::{self, Tracker};
use ghast_entity::{Config, Entity, EntityType};
use ghast_protocol::v26_1::play::clientbound::id;
use serde_json::Value;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn h64(v: &Value) -> u64 {
    u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()
}

fn h32(v: &Value) -> u32 {
    u32::from_str_radix(v.as_str().unwrap(), 16).unwrap()
}

fn packet_id(name: &str) -> i32 {
    match name {
        "add_entity" => id::SPAWN_ENTITY,
        "move_entity_pos" => id::REL_ENTITY_MOVE,
        "move_entity_pos_rot" => id::ENTITY_MOVE_LOOK,
        "move_entity_rot" => id::ENTITY_LOOK,
        "entity_position_sync" => id::SYNC_ENTITY_POSITION,
        "teleport_entity" => id::ENTITY_TELEPORT,
        "rotate_head" => id::ENTITY_HEAD_ROTATION,
        "set_entity_motion" => id::ENTITY_VELOCITY,
        "set_entity_data" => id::ENTITY_METADATA,
        "update_attributes" => id::ENTITY_UPDATE_ATTRIBUTES,
        "remove_entities" => id::ENTITY_DESTROY,
        "take_item_entity" => id::COLLECT,
        "set_passengers" => id::SET_PASSENGERS,
        "set_entity_link" => id::ATTACH_ENTITY,
        "player_info_update" => id::PLAYER_INFO,
        "player_info_remove" => id::PLAYER_REMOVE,
        "forget_level_chunk" => id::UNLOAD_CHUNK,
        "level_chunk_with_light" => id::MAP_CHUNK,
        other => panic!("unknown packet {other}"),
    }
}

fn login(player: i32) -> W {
    W::new()
        .i32(player)
        .bool(false)
        .varint(1)
        .str("minecraft:overworld")
        .varint(20)
        .varint(12)
        .varint(12)
        .bool(false)
        .bool(true)
        .bool(false)
        // spawn info: dimension type, dimension, seed, game mode, previous, debug, flat, death,
        // portal cooldown, sea level
        .varint(0)
        .str("minecraft:overworld")
        .i64(0)
        .i8(0)
        .u8(0xFF)
        .bool(false)
        .bool(false)
        .bool(false)
        .varint(0)
        .varint(63)
        .bool(false)
}

fn player_uuid(k: u64) -> uuid::Uuid {
    uuid::Uuid::from_u64_pair(0x7072_6f66_696c_6500, k)
}

/// The dump's world for `Level.noCollision`: the 7 x 7 stone floor at y 60 (no collidable
/// entities), through ghast-physics' block collision query.
fn world() -> ghast_physics::modern::MemoryWorld26 {
    let mut w = ghast_physics::modern::memory_world_26_1();
    w.load_chunks(-15, -15, 15, 15);
    w.fill_raw([-3, 60, -3], [3, 60, 3], 1); // stone
    w
}

fn no_collision(
    w: &ghast_physics::modern::MemoryWorld26,
) -> impl Fn(&Entity, [f64; 6]) -> bool + '_ {
    use ghast_physics::modern::{CollisionContext, LevelInfo, V26_1, no_block_collision};
    move |e, bb| {
        let ctx = CollisionContext {
            feet_y: e.position()[1],
            ..CollisionContext::default()
        };
        no_block_collision::<V26_1, _>(w, &LevelInfo::default(), e.position(), bb, &ctx)
    }
}

static ORDER_MISS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static ORDER_SEEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct Case {
    tracker: Tracker,
    name: String,
}

fn check(t: &Tracker, tick: &Value, label: &str) -> usize {
    let mut fields = 0;
    let want = tick["entities"].as_array().unwrap();
    let mut got: Vec<&Entity> = t.iter().collect();
    got.sort_by_key(|e| e.id());
    let ids: Vec<i32> = got.iter().map(|e| e.id()).collect();
    let want_ids: Vec<i32> = want
        .iter()
        .map(|e| e["id"].as_i64().unwrap() as i32)
        .collect();
    assert_eq!(ids, want_ids, "{label}: entity ids");
    if let Some(order) = tick.get("tick_order").and_then(Value::as_array) {
        let want: Vec<i32> = order.iter().map(|v| v.as_i64().unwrap() as i32).collect();
        if t.tick_order() != want {
            ORDER_MISS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if std::env::var("ORDER_DEBUG").is_ok() {
                eprintln!(
                    "{label}: tick order ours {:?} real {want:?}",
                    t.tick_order()
                );
            }
        }
        ORDER_SEEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    for (e, w) in got.iter().zip(want) {
        let l = format!("{label} entity {} {}", e.id(), w["type"].as_str().unwrap());
        if let Ok(d) = std::env::var("ORACLE_DEBUG")
            && label.starts_with(&format!("case {} ", d.split(':').next().unwrap()))
            && d.split(':').nth(1) == Some(&e.id().to_string())
        {
            eprintln!(
                "{l}: ours yaw {} head {} body {} pass {:?} | {:?}",
                e.yaw(),
                e.head_yaw(),
                v26_1::body_yaw(e),
                e.passengers(),
                w.get("body").map(|b| f32::from_bits(h32(b)))
            );
        }
        let ty = w["type"].as_str().unwrap();
        match e.entity_type() {
            EntityType::Player => assert_eq!(ty, "player", "{l}"),
            t => assert_eq!(v26_1::info(t).unwrap().name, ty, "{l}"),
        }
        let pickable = w["pickable"].as_bool().unwrap() && !w["spectator"].as_bool().unwrap();
        assert_eq!(t.pickable(e), pickable, "{l}: pickable");
        let vehicle = w["vehicle"].as_i64().unwrap();
        assert_eq!(e.vehicle().map_or(-1, i64::from), vehicle, "{l}: vehicle");
        let passengers: Vec<i32> = w["passengers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_i64().unwrap() as i32)
            .collect();
        assert_eq!(e.passengers(), &passengers[..], "{l}: passengers");
        if let Some(c) = w.get("count") {
            assert_eq!(
                i64::from(v26_1::item_count(e)),
                c.as_i64().unwrap(),
                "{l}: count"
            );
        }
        fields += 4;
        if w["rode"].as_bool().unwrap() {
            continue; // the position comes from the vehicle
        }
        let pos = e.position().map(f64::to_bits);
        let wp: Vec<u64> = w["pos"].as_array().unwrap().iter().map(h64).collect();
        assert_eq!(pos.to_vec(), wp, "{l}: pos {:?}", e.position());
        let rot = w["rot"].as_array().unwrap();
        assert_eq!(e.yaw().to_bits(), h32(&rot[0]), "{l}: yaw {}", e.yaw());
        assert_eq!(
            e.pitch().to_bits(),
            h32(&rot[1]),
            "{l}: pitch {}",
            e.pitch()
        );
        // Head and body turn: exact for living entities except mobs with their own body control
        // (camels here) and players' body (turned from movement).
        let td = v26_1::entity_type(e).unwrap();
        use ghast_data::v26_1::entities::type_flags as tf;
        let custom_body = td.flags & tf::MOB != 0 && td.flags & tf::DEFAULT_BODY == 0;
        if let Some(h) = w.get("head").filter(|_| !custom_body) {
            assert_eq!(e.head_yaw().to_bits(), h32(h), "{l}: head {}", e.head_yaw());
            let body_known = td.flags & tf::MOB != 0 || td.flags_meta.is_some();
            if body_known {
                let b = v26_1::body_yaw(e);
                assert_eq!(b.to_bits(), h32(&w["body"]), "{l}: body {b}");
            }
        }
        assert_eq!(
            e.on_ground(),
            w["ground"].as_bool().unwrap(),
            "{l}: on ground"
        );
        assert_eq!(
            t.is_ticking(e),
            w["ticking"].as_bool().unwrap(),
            "{l}: ticking"
        );
        let wb: Vec<u64> = w["box"].as_array().unwrap().iter().map(h64).collect();
        match e.bounding_box() {
            Some(b) => assert_eq!(b.map(f64::to_bits).to_vec(), wb, "{l}: box {b:?}"),
            None => assert!(
                v26_1::entity_type(e)
                    .is_none_or(|t| t.flags & ghast_data::v26_1::entities::type_flags::NO_BOX != 0),
                "{l}: no box"
            ),
        }
        fields += 9;
    }
    let tab = tick["tab"].as_array().unwrap();
    let mut known = 0;
    for k in 0..4u64 {
        let info = t.player_info(&player_uuid(k));
        let w = tab.iter().find(|x| x[0].as_u64() == Some(k));
        match (info, w) {
            (None, None) => {}
            (Some(i), Some(w)) => {
                known += 1;
                assert_eq!(&*i.name, w[1].as_str().unwrap(), "{label}: tab name");
                assert_eq!(
                    i64::from(i.game_mode),
                    w[2].as_i64().unwrap(),
                    "{label}: game mode"
                );
                assert_eq!(
                    i64::from(i.ping),
                    w[3].as_i64().unwrap(),
                    "{label}: latency"
                );
                assert_eq!(
                    i.display_name_nbt.is_some(),
                    w[4].as_bool().unwrap(),
                    "{label}: display"
                );
            }
            (a, b) => panic!("{label}: tab entry {k}: tracker {a:?}, client {b:?}"),
        }
    }
    fields + known * 4
}

fn replay(path: &PathBuf) -> (usize, usize, usize) {
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    let (mut ticks, mut fields, mut packets) = (0, 0, 0);
    let mut cases: Vec<Case> = Vec::new();
    let w = world();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        match v["type"].as_str().unwrap() {
            "case" => {
                assert_eq!(
                    v["scale_attribute"].as_i64().unwrap(),
                    i64::from(v26_1::SCALE_ATTRIBUTE)
                );
                let player = v["player_id"].as_i64().unwrap() as i32;
                cases.clear();
                for (name, config) in [
                    ("metadata on", Config::default()),
                    (
                        "metadata off",
                        Config::default().with_metadata(false).with_equipment(false),
                    ),
                ] {
                    let mut t = Tracker::new(config);
                    t.apply(&login(player).raw(id::LOGIN)).unwrap();
                    for r in v["loaded"].as_array().unwrap() {
                        let r: Vec<i32> = r
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|x| x.as_i64().unwrap() as i32)
                            .collect();
                        for x in r[0]..=r[2] {
                            for z in r[1]..=r[3] {
                                t.apply(&W::new().i32(x).i32(z).raw(id::MAP_CHUNK)).unwrap();
                            }
                        }
                    }
                    cases.push(Case {
                        tracker: t,
                        name: format!("case {} ({name})", v["case"]),
                    });
                }
            }
            "tick" => {
                ticks += 1;
                for c in &mut cases {
                    for p in v["packets"].as_array().unwrap() {
                        let pid = packet_id(p[0].as_str().unwrap());
                        let body = Bytes::from(hex(p[1].as_str().unwrap()));
                        c.tracker.apply_body(pid, &body).unwrap_or_else(|e| {
                            panic!("{} tick {}: {} {e:?}", c.name, v["tick"], p[0])
                        });
                        packets += 1;
                    }
                    let label = format!("{} tick {}", c.name, v["tick"]);
                    fields += check(&c.tracker, &v, &label);
                    c.tracker.tick_with(no_collision(&w));
                }
            }
            other => panic!("unknown line {other}"),
        }
    }
    (ticks, packets, fields)
}

#[test]
fn entity_tracking_matches_the_real_client() {
    let path = std::env::var_os("ORACLE_TRACKING").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/golden/26.1/entity_tracking.jsonl.gz")
        },
        PathBuf::from,
    );
    if !path.exists() {
        eprintln!("skipped: {} not found", path.display());
        return;
    }
    let (ticks, packets, fields) = replay(&path);
    eprintln!(
        "{}: {ticks} ticks, {packets} packets, {fields} fields equal",
        path.display()
    );
    assert!(ticks > 100);
    let (miss, seen) = (
        ORDER_MISS.load(std::sync::atomic::Ordering::Relaxed),
        ORDER_SEEN.load(std::sync::atomic::Ordering::Relaxed),
    );
    eprintln!(
        "tick order: {} of {seen} ticks equal to the real client's",
        seen - miss
    );
    // Known gap (a ratchet, not a proof): the tracker does not place passengers on their
    // vehicles, so a passenger's column (and the order after it dismounts) can differ.
    assert!(miss <= 94, "tick order: {miss} ticks differ (was 94)");
}
