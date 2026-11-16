//! The engine replay of the 1.12.2 entity goldens (`tests/golden/1.12.2/entities.jsonl.gz`):
//! other entities arrive as real 1.12.2 packets (spawns, teleports, relative moves, destroys,
//! tab-list entries, teams) through the sim's tracker, and every movement packet of every tick
//! must equal vanilla's.

use super::tests::opts;
use super::*;
use crate::common::{EntityAction, JoinGame};
use crate::phys_golden::v112 as golden;
use crate::sim::Sim12;
use ghast_protocol::RawPacket;
use ghast_protocol::v1_12_2::play::clientbound as cb;
use serde_json::Value;
use std::io::BufRead;

/// A packet body under construction (1.12.2 wire types).
#[derive(Default)]
struct Body(Vec<u8>);

impl Body {
    fn varint(mut self, v: i32) -> Self {
        let mut u = v as u32;
        loop {
            let b = (u & 0x7f) as u8;
            u >>= 7;
            if u == 0 {
                self.0.push(b);
                return self;
            }
            self.0.push(b | 0x80);
        }
    }
    fn uuid(mut self, most: u64, least: u64) -> Self {
        self.0.extend_from_slice(&most.to_be_bytes());
        self.0.extend_from_slice(&least.to_be_bytes());
        self
    }
    fn f64(mut self, v: f64) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn i8(mut self, v: i8) -> Self {
        self.0.push(v as u8);
        self
    }
    fn i16(mut self, v: i16) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn i32(mut self, v: i32) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn bool(mut self, v: bool) -> Self {
        self.0.push(u8::from(v));
        self
    }
    fn string(self, s: &str) -> Self {
        let mut b = self.varint(s.len() as i32);
        b.0.extend_from_slice(s.as_bytes());
        b
    }
    fn raw(self, id: i32) -> RawPacket {
        RawPacket {
            id,
            body: bytes::Bytes::from(self.0),
        }
    }
}

fn f64v(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
}

fn f32v(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
}

/// The protocol angle byte of a yaw (`floor(yaw * 256 / 360)`, as the oracle sends it).
fn angle(yaw: f32) -> i8 {
    (yaw * 256.0 / 360.0).floor() as i32 as i8
}

/// The 1.12.2 packet of one oracle entity event (see `oracle/src/mc_1_12_2/.../Ent.java`).
fn packet(ev: &Value) -> RawPacket {
    let id = |k: &str| ev[k].as_i64().unwrap() as i32;
    let pos = |b: Body| {
        let p = &ev["pos"];
        b.f64(f64v(&p[0])).f64(f64v(&p[1])).f64(f64v(&p[2]))
    };
    match ev["type"].as_str().unwrap() {
        "spawn" => {
            let eid = id("id");
            let yaw = angle(f32v(&ev["yaw"]));
            let ty = ev["entity"].as_str().unwrap();
            let object = match ty {
                "boat" => Some(1),
                "item" => Some(2),
                "minecart" => Some(10),
                "armor_stand" => Some(78),
                _ => None,
            };
            if ty == "player" {
                let b = Body::default().varint(eid).uuid(0x706c_6179, eid as u64);
                pos(b)
                    .i8(yaw)
                    .i8(0)
                    .i8(-1) // no metadata
                    .raw(cb::id::NAMED_ENTITY_SPAWN)
            } else if let Some(object) = object {
                let b = Body::default()
                    .varint(eid)
                    .uuid(0x0065_6e74, eid as u64)
                    .i8(object);
                pos(b)
                    .i8(0)
                    .i8(yaw)
                    .i32(0)
                    .i16(0)
                    .i16(0)
                    .i16(0)
                    .raw(cb::id::SPAWN_ENTITY)
            } else {
                let mob = ghast_data::v1_12::entity_by_name(ty)
                    .unwrap_or_else(|| panic!("entity {ty}"))
                    .id;
                let b = Body::default()
                    .varint(eid)
                    .uuid(0x0065_6e74, eid as u64)
                    .varint(i32::from(mob));
                pos(b)
                    .i8(yaw)
                    .i8(0)
                    .i8(yaw)
                    .i16(0)
                    .i16(0)
                    .i16(0)
                    .i8(-1) // no metadata
                    .raw(cb::id::SPAWN_ENTITY_LIVING)
            }
        }
        "entity_move" => {
            let b = Body::default().varint(id("id"));
            pos(b)
                .i8(angle(f32v(&ev["yaw"])))
                .i8(0)
                .bool(ev["on_ground"].as_bool().unwrap())
                .raw(cb::id::ENTITY_TELEPORT)
        }
        "entity_rel_move" => Body::default()
            .varint(id("id"))
            .i16(id("dx") as i16)
            .i16(id("dy") as i16)
            .i16(id("dz") as i16)
            .bool(ev["on_ground"].as_bool().unwrap())
            .raw(cb::id::REL_ENTITY_MOVE),
        "remove_entity" => Body::default()
            .varint(1)
            .varint(id("id"))
            .raw(cb::id::ENTITY_DESTROY),
        "tab" => {
            let pid = id("id");
            let add = ev["add"].as_bool().unwrap();
            let b = Body::default()
                .varint(if add { 0 } else { 1 })
                .varint(1)
                .uuid(0x706c_6179, pid as u64);
            if add {
                b.string(&format!("p{pid}"))
                    .varint(0)
                    .varint(id("game_mode"))
                    .varint(0)
                    .bool(false)
                    .raw(cb::id::PLAYER_INFO)
            } else {
                b.varint(id("game_mode")).raw(cb::id::PLAYER_INFO)
            }
        }
        "team" => {
            let name = ev["name"].as_str().unwrap();
            let mode = id("mode");
            let mut b = Body::default().string(name).i8(mode as i8);
            if mode == 0 || mode == 2 {
                b = b
                    .string(name)
                    .string("")
                    .string("")
                    .i8(0)
                    .string("always")
                    .string(ev["rule"].as_str().unwrap())
                    .i8(-1);
            }
            if matches!(mode, 0 | 3 | 4) {
                let m = ev["members"].as_array().unwrap();
                b = b.varint(m.len() as i32);
                for s in m {
                    b = b.string(s.as_str().unwrap());
                }
            }
            b.raw(cb::id::TEAMS)
        }
        other => panic!("event {other}"),
    }
}

/// The movement packets of a tick, as comparable values.
fn ours(o: &Outbound) -> Option<Value> {
    let bits = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
    Some(match *o {
        Outbound::Move(MovePacket::Position { pos, on_ground, .. }) => {
            serde_json::json!(["position", bits(pos), on_ground])
        }
        Outbound::Move(MovePacket::PositionLook {
            pos,
            yaw,
            pitch,
            on_ground,
            ..
        }) => serde_json::json!([
            "position_look",
            bits(pos),
            yaw.to_bits(),
            pitch.to_bits(),
            on_ground
        ]),
        Outbound::Move(MovePacket::Look {
            yaw,
            pitch,
            on_ground,
            ..
        }) => serde_json::json!(["look", yaw.to_bits(), pitch.to_bits(), on_ground]),
        Outbound::Move(MovePacket::Flying { on_ground, .. }) => {
            serde_json::json!(["flying", on_ground])
        }
        Outbound::EntityAction { action, .. } => serde_json::json!([
            "entity_action",
            match action {
                EntityAction::StartSprinting => "start_sprinting",
                EntityAction::StopSprinting => "stop_sprinting",
                EntityAction::StartSneaking => "start_sneaking",
                EntityAction::StopSneaking => "stop_sneaking",
                EntityAction::StartFallFlying => "start_fall_flying",
            }
        ]),
        _ => return None,
    })
}

fn theirs(p: &Value) -> Option<Value> {
    let bits = |v: &Value| {
        [
            f64v(&v[0]).to_bits(),
            f64v(&v[1]).to_bits(),
            f64v(&v[2]).to_bits(),
        ]
    };
    let og = &p["on_ground"];
    Some(match p["name"].as_str().unwrap() {
        "position" => serde_json::json!(["position", bits(&p["pos"]), og]),
        "position_look" => serde_json::json!([
            "position_look",
            bits(&p["pos"]),
            f32v(&p["yaw"]).to_bits(),
            f32v(&p["pitch"]).to_bits(),
            og
        ]),
        "look" => serde_json::json!([
            "look",
            f32v(&p["yaw"]).to_bits(),
            f32v(&p["pitch"]).to_bits(),
            og
        ]),
        "flying" => serde_json::json!(["flying", og]),
        "entity_action" => serde_json::json!(["entity_action", p["action_name"]]),
        _ => return None,
    })
}

struct Keys(Controls, bool);

impl Behavior for Keys {
    fn name(&self) -> &str {
        "entities-replay-1.12"
    }
    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        ctx.set_controls(self.0);
        ctx.set_buttons(crate::hands::Buttons {
            left: false,
            right: self.1,
        });
    }
}

/// A 1.12.2 `entity_metadata` for our entity (id 1): the shared flags byte (index 0).
fn shared_flags_packet(flags: u8) -> RawPacket {
    Body::default()
        .varint(1)
        .i8(0)
        .varint(0)
        .i8(flags as i8)
        .i8(-1)
        .raw(cb::id::ENTITY_METADATA)
}

/// A 1.12 stack of item `id` with a damage value (and the depth strider level of boots).
fn stack_1_12(id: i32, damage: i16, depth_strider: u8) -> ghast_inventory::Stack {
    let s = ghast_inventory::Stack::new(id, 1, damage);
    if depth_strider == 0 {
        return s;
    }
    // {ench: [{id: 8s, lvl: <level>s}]}
    let mut b = vec![10, 0, 0, 9, 0, 4];
    b.extend_from_slice(b"ench");
    b.push(10);
    b.extend_from_slice(&1i32.to_be_bytes());
    b.extend_from_slice(&[2, 0, 2]);
    b.extend_from_slice(b"id");
    b.extend_from_slice(&8i16.to_be_bytes());
    b.extend_from_slice(&[2, 0, 3]);
    b.extend_from_slice(b"lvl");
    b.extend_from_slice(&i16::from(depth_strider).to_be_bytes());
    b.extend_from_slice(&[0, 0]);
    s.with_nbt(Some(
        ghast_protocol::nbt::RawNbt::from_bytes(bytes::Bytes::from(b)).unwrap(),
    ))
}

/// The chest item of an elytra case (elytra 443, a chestplate 311).
fn chest_1_12(item: Option<&str>, damage: i64) -> Option<ghast_inventory::Stack> {
    let id = match item? {
        "elytra" => 443,
        "diamond_chestplate" => 311,
        other => panic!("chest item {other}"),
    };
    Some(stack_1_12(id, damage as i16, 0))
}

/// What the server sends for a rocket it spawns for us: the spawn-object packet (type 76, no
/// motion) and its entity data naming us as the shooter (index 7, a VarInt).
fn firework_packets(id: i32, pos: [f64; 3]) -> Vec<RawPacket> {
    vec![
        Body::default()
            .varint(id)
            .uuid(0x6669_7265, id as u64)
            .i8(76)
            .f64(pos[0])
            .f64(pos[1])
            .f64(pos[2])
            .i8(0)
            .i8(0)
            .i32(0)
            .i16(0)
            .i16(0)
            .i16(0)
            .raw(cb::id::SPAWN_ENTITY),
        Body::default()
            .varint(id)
            .i8(7)
            .varint(1)
            .varint(1)
            .i8(-1)
            .raw(cb::id::ENTITY_METADATA),
    ]
}

/// Replays every case of `entities.jsonl.gz` through the engine with entity tracking (`metadata`:
/// with the generic metadata store); returns (cases, ticks).
fn replay(metadata: bool) -> (usize, usize) {
    replay_file(
        &format!(
            "{}/../../tests/golden/1.12.2/entities.jsonl.gz",
            env!("CARGO_MANIFEST_DIR")
        ),
        metadata,
    )
}

/// [`replay`] of one file.
fn replay_file(path: &str, metadata: bool) -> (usize, usize) {
    let f = std::fs::File::open(path).unwrap();
    // One case in memory at a time (the cached files are large).
    let mut lines = std::io::BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str::<Value>(&l.unwrap()).unwrap())
        .peekable();
    let (mut cases, mut ticks_done, mut packets) = (0, 0, 0);
    let mut failed = Vec::new();
    while let Some(header) = lines.next() {
        let sc = &header;
        let name = sc["name"].as_str().unwrap().to_owned();
        let mut owned = Vec::new();
        while let Some(t) = lines.next_if(|l| l["type"] == "tick") {
            owned.push(t);
        }
        let ticks: Vec<&Value> = owned.iter().collect();
        cases += 1;
        let setup = &sc["setup"];
        let mut sim = Sim12::new();
        sim.test_set_profile_name("oracle");
        let mut e = Engine::new(opts(GameVersion::V1_12_2), sim);
        e.handle(Inbound::JoinGame(JoinGame {
            entity_id: 1,
            game_mode: match setup["game_mode"].as_str().unwrap() {
                "spectator" => GameMode::Spectator,
                "creative" => GameMode::Creative,
                "adventure" => GameMode::Adventure,
                _ => GameMode::Survival,
            },
            dimension: "0".into(),
            hardcore: false,
            dimension_nbt: None,
            respawn_screen: true,
        }));
        // What `Client` sets up for `Interests::ENTITY_TRACKING` (+ `ENTITIES`) on 1.12.2.
        e.sim.set_entities(Some(
            ghast_entity::Config::default()
                .with_metadata(metadata)
                .with_equipment(false)
                .with_effects(false)
                .with_player_list(false)
                .with_player_names(true)
                .with_events(false),
        ));
        // The oracle world: the loaded columns, air outside the fills (in order).
        for r in sc["loaded_chunks"].as_array().unwrap() {
            let r: Vec<i32> = (0..4).map(|k| r[k].as_i64().unwrap() as i32).collect();
            for x in r[0]..=r[2] {
                for z in r[1]..=r[3] {
                    e.sim
                        .world_packet(&crate::sim::test_chunks::v1_12_2(x, z))
                        .unwrap();
                }
            }
            for x in r[0] * 16..r[2] * 16 + 16 {
                for z in r[1] * 16..r[3] * 16 + 16 {
                    e.sim.predict_air(x, 63, z);
                }
            }
        }
        for fill in sc["fills"].as_array().unwrap() {
            let v: Vec<i64> = (0..8).map(|k| fill[k].as_i64().unwrap()).collect();
            let st = ghast_data::v1_12::BlockState::from_raw_vanilla(
                ((v[6] as u16) << 4) | (v[7] as u16 & 15),
            );
            for x in v[0]..=v[3] {
                for y in v[1]..=v[4] {
                    for z in v[2]..=v[5] {
                        e.sim.test_set_block(x as i32, y as i32, z as i32, st);
                    }
                }
            }
        }
        e.handle(Inbound::Teleport(crate::common::Teleport {
            x: f64v(&setup["pos"][0]),
            y: f64v(&setup["pos"][1]),
            z: f64v(&setup["pos"][2]),
            yaw: f32v(&setup["yaw"]),
            pitch: f32v(&setup["pitch"]),
            teleport_id: None,
            ..Default::default()
        }));
        let mut p = golden::player_of(sc, ticks[0]);
        // The oracle's setup put the player into its (loaded) chunk.
        p.added_to_chunk = true;
        e.sim.test_set_player(p);
        for ev in setup["entities"].as_array().into_iter().flatten() {
            e.sim.world_packet(&packet(ev)).unwrap();
        }
        // Elytra cases: the chest item and boots in the inventory window, rockets in the hand,
        // the server's flag 7 at setup.
        if let Some(x) = setup.get("ext") {
            e.sim.test_watch_glide();
            let damage = x.get("chest_damage").map_or(0, |v| v.as_i64().unwrap());
            e.inventory_mut().on_set_slot(
                0,
                6,
                chest_1_12(x.get("chest").and_then(Value::as_str), damage),
            );
            let ds = setup["depth_strider"].as_u64().unwrap_or(0) as u8;
            if ds > 0 {
                e.inventory_mut()
                    .on_set_slot(0, 8, Some(stack_1_12(313, 0, ds)));
            }
            if x.get("hand").and_then(Value::as_str) == Some("fireworks") {
                e.inventory_mut()
                    .on_set_slot(0, 36, Some(stack_1_12(401, 0, 0)));
            }
            if x.get("fall_flying").and_then(Value::as_bool) == Some(true) {
                let cur = e.sim.test_shared_flags_1_12();
                e.sim
                    .world_packet(&shared_flags_packet(cur | 0x80))
                    .unwrap();
            }
        }
        e.core.pose.pos = Vec3::new(p.pos[0], p.bb.min_y, p.pos[2]);
        e.core.pose.yaw = p.yaw;
        e.core.pose.pitch = p.pitch;
        e.core.ready = true;
        e.outbound().clear();
        let t0 = Instant::now();
        // The engine does not tick the player outside the loaded columns (on purpose; a
        // vanilla 1.12 client ticks it there): a case ends where the oracle's player leaves
        // them.
        let rects: Vec<[i32; 4]> = sc["loaded_chunks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| std::array::from_fn(|k| r[k].as_i64().unwrap() as i32))
            .collect();
        let column_loaded = |s: &Value| {
            let (cx, cz) = (
                (f64v(&s["pos"][0]).floor() as i32) >> 4,
                (f64v(&s["pos"][2]).floor() as i32) >> 4,
            );
            rects
                .iter()
                .any(|r| r[0] <= cx && cx <= r[2] && r[1] <= cz && cz <= r[3])
        };
        for (k, rec) in ticks.iter().enumerate().skip(1) {
            if !column_loaded(&ticks[k - 1]["state"]) {
                break;
            }
            let input = &rec["input"];
            for ev in input["events"].as_array().into_iter().flatten() {
                match ev["type"].as_str().unwrap() {
                    "velocity" => {
                        let c = |k: &str| f64::from(ev[k].as_i64().unwrap() as i32) / 8000.0;
                        e.handle(Inbound::Velocity {
                            entity_id: 1,
                            motion: Vec3::new(c("x"), c("y"), c("z")),
                        });
                    }
                    "shared_flags" => {
                        let set = ev["set"].as_u64().unwrap() as u8;
                        let clear = ev["clear"].as_u64().unwrap() as u8;
                        let cur = e.sim.test_shared_flags_1_12();
                        e.sim
                            .world_packet(&shared_flags_packet((cur & !clear) | set))
                            .unwrap();
                    }
                    "equip" => {
                        let item = ev.get("item").and_then(Value::as_str);
                        let damage = ev["damage"].as_i64().unwrap();
                        e.inventory_mut()
                            .on_set_slot(0, 6, chest_1_12(item, damage));
                    }
                    "firework" => {
                        // At our position when the server handles the use (after our last tick).
                        let s = &ticks[k - 1]["state"]["pos"];
                        let pos = [f64v(&s[0]), f64v(&s[1]), f64v(&s[2])];
                        for raw in firework_packets(ev["id"].as_i64().unwrap() as i32, pos) {
                            e.sim.world_packet(&raw).unwrap();
                        }
                    }
                    _ => e.sim.world_packet(&packet(ev)).unwrap(),
                }
            }
            let keys = input["keys"].as_str().unwrap();
            let has = |k: &str| keys.split(',').any(|n| n == k);
            let mut b = Keys(
                Controls {
                    forward: has("forward"),
                    back: has("back"),
                    left: has("left"),
                    right: has("right"),
                    jump: has("jump"),
                    sneak: has("sneak"),
                    sprint: has("sprint"),
                },
                has("use"),
            );
            e.core.forced_look = Some((f32v(&input["yaw"]), f32v(&input["pitch"])));
            e.note_inbound(t0);
            e.tick(&mut b, t0);
            let out = std::mem::take(e.outbound());
            let got: Vec<Value> = out.iter().filter_map(ours).collect();
            let want: Vec<Value> = rec["packets"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(theirs)
                .collect();
            if got != want {
                failed.push(format!("{name} tick {k}: got {got:?} want {want:?}"));
                break;
            }
            packets += want.len();
            ticks_done += 1;
        }
    }
    eprintln!(
        "1.12.2 {path} through the engine (metadata {metadata}): {cases} cases, {ticks_done} ticks, {packets} packets identical"
    );
    assert!(
        failed.is_empty(),
        "{} of {cases} cases failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
    (cases, ticks_done)
}

/// 1.12.2 entity pushes and entity collisions through the engine: 0 differences.
#[test]
fn entities_match_the_oracle_1_12_2() {
    let (cases, ticks) = replay(true);
    assert!(cases >= 80 && ticks > 4000, "{cases} cases, {ticks} ticks");
}

/// The same with `Interests::ENTITY_TRACKING` alone (no generic metadata store).
#[test]
fn entities_match_the_oracle_1_12_2_without_metadata() {
    replay(false);
}

/// The 1.12.2 elytra goldens through the engine: the chest item in the inventory window, the
/// server's flag 7 as our entity data, rockets as the server's spawn and entity data packets;
/// every movement and `entity_action` packet (the glide request) must equal the real client's.
#[test]
fn elytra_match_the_oracle_1_12_2() {
    let (cases, ticks) = replay_file(
        &format!(
            "{}/../../tests/golden/1.12.2/elytra.jsonl.gz",
            env!("CARGO_MANIFEST_DIR")
        ),
        false,
    );
    assert!(cases >= 120 && ticks > 7000, "{cases} cases, {ticks} ticks");
}

/// The cached elytra fuzz (`oracle/cache/1.12.2/fuzz-elytra/`, git-ignored) through the engine.
#[test]
#[ignore = "needs the oracle cache"]
fn elytra_cache_match_the_oracle_1_12_2() {
    let dir = format!(
        "{}/../../oracle/cache/1.12.2/fuzz-elytra",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("skipped: {dir} not found");
        return;
    };
    let mut files: Vec<_> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".jsonl.gz"))
        .collect();
    files.sort();
    let (mut cases, mut ticks) = (0, 0);
    for f in &files {
        let (c, t) = replay_file(&f.to_string_lossy(), false);
        cases += c;
        ticks += t;
    }
    eprintln!("1.12.2 elytra fuzz cache through the engine: {cases} cases, {ticks} ticks");
}

/// The cached entity fuzz (`oracle/cache/1.12.2/fuzz-entities/`, git-ignored; see the oracle
/// README) through the engine. Slow: run in release with `--ignored`.
#[test]
#[ignore = "needs the oracle cache"]
fn entities_cache_match_the_oracle_1_12_2() {
    let dir = format!(
        "{}/../../oracle/cache/1.12.2/fuzz-entities",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".jsonl.gz"))
        .collect();
    files.sort();
    let (mut cases, mut ticks) = (0, 0);
    for f in &files {
        let (c, t) = replay_file(&f.to_string_lossy(), true);
        cases += c;
        ticks += t;
    }
    eprintln!("1.12.2 entity fuzz cache through the engine: {cases} cases, {ticks} ticks");
    assert!(cases > 0);
}
