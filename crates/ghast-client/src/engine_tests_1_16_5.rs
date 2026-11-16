//! The engine replay of the 1.16.5 entity goldens (`tests/golden/1.16.5/entities.jsonl.gz`):
//! other entities arrive as real 1.16.5 packets (spawns with their entity data, teleports,
//! relative moves, looks, destroys, tab-list entries, teams) through the sim's tracker, the
//! tracker's client world tick order must equal vanilla's on every tick, and every movement
//! packet of every tick must equal vanilla's.

use super::tests::opts;
use super::*;
use crate::common::{EntityAction, JoinGame};
use crate::phys_golden::v116 as golden;
use crate::sim::Sim16;
use ghast_protocol::RawPacket;
use ghast_protocol::v1_16_5::play::clientbound as cb;
use serde_json::Value;
use std::io::BufRead;

/// A packet body under construction (1.16.5 wire types).
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
    fn uuid(mut self, u: uuid::Uuid) -> Self {
        self.0.extend_from_slice(u.as_bytes());
        self
    }
    fn f64(mut self, v: f64) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn f32(mut self, v: f32) -> Self {
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
    fn i64(mut self, v: i64) -> Self {
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
    fn raw_bytes(mut self, b: &[u8]) -> Self {
        self.0.extend_from_slice(b);
        self
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

/// The protocol angle byte of a yaw (`(int) (yaw * 256 / 360)`, as the oracle sends it).
fn angle(yaw: f32) -> i8 {
    (yaw * 256.0 / 360.0) as i32 as i8
}

/// The 1.16.5 packets of one oracle entity event (see `oracle/src/mc_1_16_5/.../Ents.java`).
fn event_packets(ev: &Value) -> Vec<RawPacket> {
    let id = |k: &str| ev[k].as_i64().unwrap() as i32;
    // The oracle's server-side shulker sits on its attached block (the block it was placed
    // in): its spawn packet carries the block centre.
    let shulker = ev["entity"] == "shulker";
    let pos = |b: Body| {
        let p = &ev["pos"];
        let (x, y, z) = (f64v(&p[0]), f64v(&p[1]), f64v(&p[2]));
        if shulker {
            b.f64(x.floor() + 0.5).f64(y.floor()).f64(z.floor() + 0.5)
        } else {
            b.f64(x).f64(y).f64(z)
        }
    };
    let uuid = || uuid::Uuid::parse_str(ev["uuid"].as_str().unwrap()).unwrap();
    match ev["type"].as_str().unwrap_or("spawn") {
        "spawn" => {
            let eid = id("id");
            let yaw = angle(f32v(&ev["yaw"]));
            let ty = ev["entity"].as_str().unwrap();
            let mut out = Vec::new();
            let living;
            if ty == "player" {
                living = true;
                out.push(
                    Body::default()
                        .varint(0)
                        .varint(1)
                        .uuid(uuid())
                        .string(ev["name"].as_str().unwrap())
                        .varint(0)
                        .varint(id("game_mode"))
                        .varint(0)
                        .bool(false)
                        .raw(cb::id::PLAYER_INFO),
                );
                out.push(
                    pos(Body::default().varint(eid).uuid(uuid()))
                        .i8(yaw)
                        .i8(0)
                        .raw(cb::id::NAMED_ENTITY_SPAWN),
                );
            } else {
                let t = ghast_data::v1_16_5::entities()
                    .iter()
                    .find(|t| t.name == ty)
                    .unwrap_or_else(|| panic!("entity {ty}"))
                    .id;
                living = !matches!(ty, "boat" | "minecart" | "item");
                if living {
                    out.push(
                        pos(Body::default()
                            .varint(eid)
                            .uuid(uuid())
                            .varint(i32::from(t)))
                        .i8(yaw)
                        .i8(0)
                        .i8(yaw)
                        .i16(0)
                        .i16(0)
                        .i16(0)
                        .raw(cb::id::SPAWN_ENTITY_LIVING),
                    );
                } else {
                    out.push(
                        pos(Body::default()
                            .varint(eid)
                            .uuid(uuid())
                            .varint(i32::from(t)))
                        .i8(0)
                        .i8(yaw)
                        .i32(0)
                        .i16(0)
                        .i16(0)
                        .i16(0)
                        .raw(cb::id::SPAWN_ENTITY),
                    );
                }
            }
            // The entity data a server sends at spawn, as far as the client's sizes and states
            // go (every entry has its default value): the pose, and the size entries.
            let mut m = Body::default().varint(eid).i8(6).varint(18).varint(0);
            match ty {
                "slime" | "magma_cube" => m = m.i8(15).varint(1).varint(1),
                "armor_stand" => m = m.i8(14).varint(0).i8(0),
                "shulker" => {
                    let p = &ev["pos"];
                    let (x, y, z) = (
                        f64v(&p[0]).floor() as i64,
                        f64v(&p[1]).floor() as i64,
                        f64v(&p[2]).floor() as i64,
                    );
                    let packed = ((x & 0x3FF_FFFF) << 38) | ((z & 0x3FF_FFFF) << 12) | (y & 0xFFF);
                    m = m.i8(16).varint(10).bool(true).i64(packed);
                }
                _ => {}
            }
            let baby = ghast_data::v1_16_5::vanilla_entity_sizes()
                .iter()
                .any(|v| v.name == ty && v.baby().is_some());
            if baby {
                m = m
                    .i8(if ty == "piglin" { 16 } else { 15 })
                    .varint(7)
                    .bool(false);
            }
            if living && ty != "player" {
                m = m.i8(8).varint(2).f32(20.0);
            }
            out.push(m.i8(-1).raw(cb::id::ENTITY_METADATA));
            out
        }
        "entity_move" => vec![
            pos(Body::default().varint(id("id")))
                .i8(angle(f32v(&ev["yaw"])))
                .i8(0)
                .bool(ev["on_ground"].as_bool().unwrap())
                .raw(cb::id::ENTITY_TELEPORT),
        ],
        "entity_rel" => {
            let d = &ev["delta"];
            vec![
                Body::default()
                    .varint(id("id"))
                    .i16(d[0].as_i64().unwrap() as i16)
                    .i16(d[1].as_i64().unwrap() as i16)
                    .i16(d[2].as_i64().unwrap() as i16)
                    .bool(ev["on_ground"].as_bool().unwrap())
                    .raw(cb::id::REL_ENTITY_MOVE),
            ]
        }
        "entity_look" => vec![
            Body::default()
                .varint(id("id"))
                .i8(angle(f32v(&ev["yaw"])))
                .i8(0)
                .bool(true)
                .raw(cb::id::ENTITY_LOOK),
        ],
        "remove_entity" => vec![
            Body::default()
                .varint(1)
                .varint(id("id"))
                .raw(cb::id::ENTITY_DESTROY),
        ],
        "game_mode" => {
            let pid = id("id");
            vec![
                Body::default()
                    .varint(1)
                    .varint(1)
                    .uuid(uuid::Uuid::from_u64_pair(0x706c_6179, pid as u64))
                    .varint(id("game_mode"))
                    .raw(cb::id::PLAYER_INFO),
            ]
        }
        "team" => {
            let name = ev["name"].as_str().unwrap();
            let mode = id("mode");
            let mut b = Body::default().string(name).i8(mode as i8);
            if mode == 0 || mode == 2 {
                b = b
                    .string(&format!("{{\"text\":\"{name}\"}}"))
                    .i8(0)
                    .string("always")
                    .string(ev["rule"].as_str().unwrap_or("always"))
                    .varint(21)
                    .string("{\"text\":\"\"}")
                    .string("{\"text\":\"\"}");
            }
            if matches!(mode, 0 | 3 | 4) {
                let m = ev["members"].as_array().unwrap();
                b = b.varint(m.len() as i32);
                for s in m {
                    b = b.string(s.as_str().unwrap());
                }
            }
            vec![b.raw(cb::id::TEAMS)]
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
                EntityAction::StartSneaking => "press_shift_key",
                EntityAction::StopSneaking => "release_shift_key",
                _ => "other",
            }
        ]),
        Outbound::UseItem { off_hand, .. } => serde_json::json!(["use_item", u8::from(off_hand)]),
        Outbound::Dig {
            status: crate::common::DigStatus::ReleaseUseItem,
            ..
        } => serde_json::json!(["release_use_item"]),
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
        "use_item" => serde_json::json!(["use_item", p["hand"]]),
        "block_dig" if p["status_name"] == "release_use_item" => {
            serde_json::json!(["release_use_item"])
        }
        _ => return None,
    })
}

/// The tick's movement keys and the use key (right button).
struct Keys(Controls, bool);

impl Behavior for Keys {
    fn name(&self) -> &str {
        "entities-replay-1.16.5"
    }
    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        ctx.set_controls(self.0);
        ctx.set_buttons(crate::hands::Buttons {
            left: false,
            right: self.1,
        });
    }
}

/// A 1.16.5 `entity_metadata` for our entity (id 1): the living entity flags byte (index 7).
fn living_flags_packet(flags: u8) -> RawPacket {
    Body::default()
        .varint(1)
        .i8(7)
        .varint(0)
        .i8(flags as i8)
        .i8(-1)
        .raw(cb::id::ENTITY_METADATA)
}

/// A stack's NBT `{Enchantments: [{id: "minecraft:<name>", lvl: <level>s}, ...]}`.
fn enchantments_nbt(list: &[(&str, u8)]) -> ghast_protocol::nbt::RawNbt {
    let mut b = vec![10, 0, 0, 9, 0, 12];
    b.extend_from_slice(b"Enchantments");
    b.push(10);
    b.extend_from_slice(&(list.len() as i32).to_be_bytes());
    for (name, level) in list {
        let id = format!("minecraft:{name}");
        b.extend_from_slice(&[8, 0, 2]);
        b.extend_from_slice(b"id");
        b.extend_from_slice(&(id.len() as u16).to_be_bytes());
        b.extend_from_slice(id.as_bytes());
        b.extend_from_slice(&[2, 0, 3]);
        b.extend_from_slice(b"lvl");
        b.extend_from_slice(&i16::from(*level).to_be_bytes());
        b.push(0);
    }
    b.push(0);
    ghast_protocol::nbt::RawNbt::from_bytes(bytes::Bytes::from(b)).unwrap()
}

/// The events this replay drives (the player's own events of the other fuzz kinds are not
/// sent through the engine here).
fn driven(ev: &Value) -> bool {
    matches!(
        ev["type"].as_str(),
        Some(
            "spawn"
                | "entity_move"
                | "entity_rel"
                | "entity_look"
                | "remove_entity"
                | "team"
                | "game_mode"
                | "velocity"
                | "teleport"
                | "abilities"
                | "speed_attribute"
                | "living_flags"
                | "effects"
        )
    )
}

/// Replays the cases of an entities file through the engine with entity tracking (`metadata`:
/// with the generic metadata store): every case whose events this replay drives. Returns
/// (cases, ticks, cases skipped).
pub(super) fn replay_file(path: &std::path::Path, metadata: bool) -> (usize, usize, usize) {
    replay_file_with(path, metadata, true)
}

/// A 1.16.5 join packet (our entity id 1) for a world `name` whose dimension type compound
/// holds `ultrawarm` (the only field the movement reads).
fn login_packet(name: &str, ultrawarm: bool) -> RawPacket {
    Body::default()
        .i32(1)
        .bool(false)
        .i8(0)
        .i8(-1)
        .varint(1)
        .string(name)
        .raw_bytes(&[10, 0, 0, 0])
        .raw_bytes(&dimension_nbt(ultrawarm))
        .string(name)
        .i64(0)
        .varint(20)
        .varint(10)
        .bool(false)
        .bool(true)
        .bool(false)
        .bool(false)
        .raw(cb::id::LOGIN)
}

/// A 1.16.5 respawn packet into world `name` with that dimension type.
fn respawn_packet(name: &str, ultrawarm: bool) -> RawPacket {
    Body::default()
        .raw_bytes(&dimension_nbt(ultrawarm))
        .string(name)
        .i64(0)
        .i8(0)
        .i8(-1)
        .bool(false)
        .bool(false)
        .bool(false)
        .raw(cb::id::RESPAWN)
}

/// A nameless compound `{ultrawarm: <byte>, has_skylight: <byte>}` (the nether: ultrawarm,
/// no sky light).
fn dimension_nbt(ultrawarm: bool) -> Vec<u8> {
    let mut b = vec![10, 0, 0, 1, 0, 9];
    b.extend_from_slice(b"ultrawarm");
    b.push(u8::from(ultrawarm));
    b.extend_from_slice(&[1, 0, 12]);
    b.extend_from_slice(b"has_skylight");
    b.push(u8::from(!ultrawarm));
    b.push(0);
    b
}

/// [`replay_file`]; `tracking`: with entity tracking (else the plain client, what a bot without
/// `Interests::ENTITY_TRACKING` runs). A case with `setup.ext.dimension` joins that dimension
/// through a real join packet.
pub(super) fn replay_file_with(
    path: &std::path::Path,
    metadata: bool,
    tracking: bool,
) -> (usize, usize, usize) {
    let f = std::fs::File::open(path).unwrap();
    // One case at a time (the cached files are large).
    let mut lines = std::io::BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str::<Value>(&l.unwrap()).unwrap())
        .peekable();
    let (mut cases, mut skipped, mut ticks_done, mut packets, mut orders) = (0, 0, 0, 0, 0);
    // Cases cut short where the player left the loaded columns, or used an item on a block.
    let mut left = 0;
    let mut picked = 0;
    // Ticks whose block click passed on to the item (its block packet left out).
    let mut passed_on = 0;
    let mut failed = Vec::new();
    while let Some(header) = lines.next() {
        let sc = &header;
        let name = sc["name"].as_str().unwrap().to_owned();
        let mut owned = Vec::new();
        while let Some(t) = lines.next_if(|l| l["type"] == "tick") {
            owned.push(t);
        }
        let ticks: Vec<&Value> = owned.iter().collect();
        let plain = ticks.iter().skip(1).all(|t| {
            t["input"]["events"]
                .as_array()
                .into_iter()
                .flatten()
                .all(driven)
        });
        if !plain {
            skipped += 1;
            continue;
        }
        cases += 1;
        let setup = &sc["setup"];
        let mut sim = Sim16::new();
        sim.test_set_profile_name("oracle");
        let mut e = Engine::new(opts(GameVersion::V1_16_5), sim);
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
        // The join packet's dimension type (ClientPlayNetworkHandler.onGameJoin); riptide cases
        // join too (our entity id for the spin flag).
        let ext = setup.get("ext");
        let dimension = ext.and_then(|x| x.get("dimension")).and_then(Value::as_str);
        let hand = ext.and_then(|x| x.get("hand")).and_then(Value::as_str);
        let joined = dimension.is_some() || hand.is_some();
        if joined {
            let d = dimension.unwrap_or("overworld");
            let ultrawarm = match d {
                "the_nether" => true,
                "overworld" => false,
                other => panic!("dimension {other}"),
            };
            e.sim
                .world_packet(&login_packet(&format!("minecraft:{d}"), ultrawarm))
                .unwrap();
            assert_eq!(e.sim.test_ext().ultrawarm, ultrawarm, "{name}: join packet");
        }
        if tracking {
            // What `Client` sets up for `Interests::ENTITY_TRACKING` (+ `ENTITIES`) on 1.16.5.
            e.sim.set_entities(Some(
                ghast_entity::Config::default()
                    .with_metadata(metadata)
                    .with_equipment(false)
                    .with_effects(false)
                    .with_player_list(false)
                    .with_player_names(true)
                    .with_events(false),
            ));
            // The login the tracker saw: our entity id 1, a new client world.
            if !joined {
                e.sim.test_tracker_login(1);
            }
        }
        // The oracle world: the loaded columns, air outside the fills (in order).
        for r in sc["loaded_chunks"].as_array().unwrap() {
            let r: Vec<i32> = (0..4).map(|k| r[k].as_i64().unwrap() as i32).collect();
            for x in r[0]..=r[2] {
                for z in r[1]..=r[3] {
                    e.sim
                        .world_packet(&crate::sim::test_chunks::v1_16_5(x, z))
                        .unwrap();
                }
            }
        }
        for fill in sc["fills"].as_array().unwrap() {
            let v: Vec<i64> = (0..7).map(|k| fill[k].as_i64().unwrap()).collect();
            let st = ghast_data::v1_16_5::BlockState::from_id(v[6] as u16).unwrap();
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
        let p = golden::player_of(sc, ticks[0]);
        e.sim.test_set_player(p);
        if let Some(x) = setup.get("ext") {
            for t in x["teams"].as_array().unwrap() {
                for raw in event_packets(
                    &serde_json::json!({"type": "team", "name": t["name"], "mode": t["mode"], "rule": t["rule"], "members": t["members"]}),
                ) {
                    e.sim.world_packet(&raw).unwrap();
                }
            }
            for ev in x["entities"].as_array().unwrap() {
                for raw in event_packets(ev) {
                    e.sim.world_packet(&raw).unwrap();
                }
            }
        }
        // Riptide: the trident in the selected hotbar slot, the rain as game state packets.
        if let Some(h) = hand {
            assert_eq!(h, "trident", "{name}");
            let id = ghast_data::v1_16_5::Item::from_name(h).unwrap().id();
            let level = ext
                .and_then(|x| x.get("riptide"))
                .map_or(0, |v| v.as_u64().unwrap());
            let mut stack = ghast_inventory::Stack::new(i32::from(id), 1, 0);
            if level > 0 {
                stack = stack.with_nbt(Some(enchantments_nbt(&[("riptide", level as u8)])));
            }
            e.inventory_mut().on_set_slot(0, 36, Some(stack));
            // The inventory now drives the boots' enchantments: the oracle's boots, as the
            // server's equipment would show them in the player's armour slot.
            let boots: Vec<(&str, u8)> = [
                ("depth_strider", "depth_strider"),
                ("soul_speed", "soul_speed"),
            ]
            .iter()
            .filter_map(|(k, n)| {
                let l = setup[*k].as_u64().unwrap_or(0) as u8;
                (l > 0).then_some((*n, l))
            })
            .collect();
            if !boots.is_empty() {
                let id = ghast_data::v1_16_5::Item::from_name("diamond_boots")
                    .unwrap()
                    .id();
                let stack = ghast_inventory::Stack::new(i32::from(id), 1, 0)
                    .with_nbt(Some(enchantments_nbt(&boots)));
                e.inventory_mut().on_set_slot(0, 8, Some(stack));
            }
            if ext.and_then(|x| x.get("raining")).and_then(Value::as_bool) == Some(true) {
                e.handle(Inbound::GameState {
                    reason: 1,
                    value: 0.0,
                });
                e.handle(Inbound::GameState {
                    reason: 7,
                    value: 1.0,
                });
            }
        }
        // The spin flag as the server sends it (living entity flags, index 7).
        let mut living = 0u8;
        e.core.pose.pos = Vec3::new(p.pos[0], p.pos[1], p.pos[2]);
        e.core.pose.yaw = p.yaw;
        e.core.pose.pitch = p.pitch;
        e.core.ready = true;
        e.outbound().clear();
        let t0 = Instant::now();
        let mut teleport_id = 1;
        // The loaded columns: the engine does not tick the player outside them (on purpose: a
        // vanilla 1.16.5 client ticks it there, its blocks read as air), so a case ends where
        // the oracle's player leaves them.
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
        // The player's effects as the server set them (an `effects` event replaces the set).
        let mut effects: Vec<i8> = setup["effects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e[0].as_i64().unwrap() as i8)
            .collect();
        for (k, rec) in ticks.iter().enumerate().skip(1) {
            if !column_loaded(&ticks[k - 1]["state"]) {
                left += 1;
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
                    "teleport" => {
                        let flags = ev.get("flags").and_then(Value::as_i64).unwrap_or(0);
                        e.handle(Inbound::Teleport(crate::common::Teleport {
                            x: f64v(&ev["pos"][0]),
                            y: f64v(&ev["pos"][1]),
                            z: f64v(&ev["pos"][2]),
                            yaw: f32v(&ev["yaw"]),
                            pitch: f32v(&ev["pitch"]),
                            flags: flags as i8,
                            relative: flags as u32,
                            teleport_id: Some(teleport_id),
                            ..Default::default()
                        }));
                        teleport_id += 1;
                    }
                    "abilities" => {
                        let b = |k: &str| ev[k].as_bool().unwrap();
                        let flags = i8::from(b("invulnerable"))
                            | i8::from(b("flying")) << 1
                            | i8::from(b("allow_flying")) << 2
                            | i8::from(b("creative")) << 3;
                        e.handle(Inbound::Abilities(crate::common::Abilities {
                            flags,
                            flying_speed: f32v(&ev["fly_speed"]),
                            walking_speed: f32v(&ev["walk_speed"]),
                        }));
                    }
                    "speed_attribute" => {
                        let modifiers = ev["modifiers"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|m| {
                                (
                                    uuid::Uuid::parse_str(m[0].as_str().unwrap()).unwrap(),
                                    f64v(&m[1]),
                                    m[2].as_i64().unwrap() as i8,
                                )
                            })
                            .collect();
                        e.handle(Inbound::MovementSpeed {
                            entity_id: 1,
                            base: f64v(&ev["base"]),
                            modifiers,
                        });
                    }
                    "effects" => {
                        let list: Vec<(i8, i8)> = ev["effects"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|f| (f[0].as_i64().unwrap() as i8, f[1].as_i64().unwrap() as i8))
                            .collect();
                        for id in effects.clone() {
                            if !list.iter().any(|f| f.0 == id) {
                                e.handle(Inbound::EffectRemoved { entity_id: 1, id });
                            }
                        }
                        for &(id, amplifier) in &list {
                            e.handle(Inbound::Effect {
                                entity_id: 1,
                                id,
                                amplifier,
                            });
                        }
                        effects = list.iter().map(|f| f.0).collect();
                    }
                    "living_flags" => {
                        let set = ev["set"].as_u64().unwrap() as u8;
                        let clear = ev["clear"].as_u64().unwrap() as u8;
                        living = (living & !clear) | set;
                        e.sim.world_packet(&living_flags_packet(living)).unwrap();
                    }
                    _ => {
                        for raw in event_packets(ev) {
                            e.sim.world_packet(&raw).unwrap();
                        }
                    }
                }
            }
            // The client world's tick order, as vanilla iterates it this tick.
            if let Some(want) = rec.get("entities").and_then(Value::as_array) {
                let want: Vec<i64> = want.iter().map(|x| x["id"].as_i64().unwrap()).collect();
                let mut got: Vec<i64> =
                    e.sim.test_tick_order().into_iter().map(i64::from).collect();
                // An entity that removed itself on the client (an item burnt in lava, ...) is
                // still in ours (not modelled; the server's destroy follows): the order of the
                // others must agree.
                if got.len() > want.len() {
                    got.retain(|id| want.contains(id));
                }
                if got != want {
                    failed.push(format!("{name} tick {k}: tick order {got:?} want {want:?}"));
                    break;
                }
                orders += 1;
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
            let mut out = std::mem::take(e.outbound());
            // The oracle uses the item with the crosshair on nothing; the engine picks the
            // block it looks at. A block click that passes on to the item (vanilla sends the
            // block packet, then uses the item) is the oracle's miss plus that block packet:
            // drop it and go on. An activation or a placement (no item use) ends the case.
            if out.iter().any(|o| matches!(o, Outbound::Place { .. })) {
                if out.iter().any(|o| matches!(o, Outbound::UseItem { .. })) {
                    out.retain(|o| !matches!(o, Outbound::Place { .. }));
                    passed_on += 1;
                } else {
                    picked += 1;
                    break;
                }
            }
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
            // The player's velocity after the tick (pushes included).
            {
                let v = e.sim.test_player().vel;
                let want: Vec<u64> = (0..3)
                    .map(|k| f64v(&rec["state"]["vel"][k]).to_bits())
                    .collect();
                let got: Vec<u64> = v.iter().map(|x| x.to_bits()).collect();
                if got != want {
                    failed.push(format!(
                        "{name} tick {k}: velocity {v:?} want {:?}",
                        want.iter().map(|b| f64::from_bits(*b)).collect::<Vec<_>>()
                    ));
                    break;
                }
            }
            // The client positions and boxes after the entity tick.
            let mut boxes_ok = true;
            for x in rec
                .get("entities")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(id) = x.get("type").and(x["id"].as_i64()) else {
                    continue;
                };
                // Only what pushes or collides: bats, armor stands and parrots place themselves.
                let ty = x["type"].as_str().unwrap();
                let living = !matches!(ty, "boat" | "minecart" | "item");
                if ghast_physics::v1_16_5::push_kind(ty, living).is_none()
                    && !ghast_physics::v1_16_5::collides(ty)
                {
                    continue;
                }
                let Some((pos, bb)) = e.sim.test_client_box(id as i32) else {
                    continue;
                };
                let want_bb: Vec<u64> = (0..6).map(|k| f64v(&x["post"][k]).to_bits()).collect();
                let want_pos: Vec<u64> = (0..3).map(|k| f64v(&x["pos"][k]).to_bits()).collect();
                let got_bb: Vec<u64> = bb.iter().map(|v| v.to_bits()).collect();
                let got_pos: Vec<u64> = pos.iter().map(|v| v.to_bits()).collect();
                if got_bb != want_bb || got_pos != want_pos {
                    failed.push(format!(
                        "{name} tick {k}: entity {id} {} box {bb:?} at {pos:?}, want {:?} at {:?}",
                        x["type"],
                        want_bb
                            .iter()
                            .map(|b| f64::from_bits(*b))
                            .collect::<Vec<_>>(),
                        want_pos
                            .iter()
                            .map(|b| f64::from_bits(*b))
                            .collect::<Vec<_>>()
                    ));
                    boxes_ok = false;
                    break;
                }
            }
            if !boxes_ok {
                break;
            }
            packets += want.len();
            ticks_done += 1;
        }
    }
    eprintln!(
        "1.16.5 {} through the engine (tracking {tracking}, metadata {metadata}): {cases} cases ({skipped} skipped, {left} left the loaded columns, {picked} used an item on a block, {passed_on} block clicks passed on to the item), {ticks_done} ticks, {orders} tick orders, {packets} packets identical",
        path.display()
    );
    assert!(
        failed.is_empty(),
        "{} of {cases} cases failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
    (cases, ticks_done, skipped)
}

fn golden_file() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.16.5/entities.jsonl.gz")
}

/// 1.16.5 entity pushes and entity collisions through the engine: 0 differences.
#[test]
fn entities_match_the_oracle_1_16_5() {
    let (cases, ticks, _) = replay_file(&golden_file(), true);
    assert!(cases >= 40 && ticks > 2000, "{cases} cases, {ticks} ticks");
}

/// The same with `Interests::ENTITY_TRACKING` alone (no generic metadata store).
#[test]
fn entities_match_the_oracle_1_16_5_without_metadata() {
    replay_file(&golden_file(), false);
}

/// The cached entities fuzz (`oracle/cache/1.16.5/fuzz-entities/*.jsonl.gz`, not committed)
/// through the engine; `GHAST_FUZZ_ENTITIES` names another directory. Skipped when absent.
#[test]
#[ignore = "cache run: cargo test --release -p ghast-client --lib -- --ignored entities_cache_1_16_5"]
fn entities_cache_1_16_5() {
    let dir = std::env::var("GHAST_FUZZ_ENTITIES").map_or_else(
        |_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../oracle/cache/1.16.5/fuzz-entities")
        },
        std::path::PathBuf::from,
    );
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("skipped: {} not found", dir.display());
        return;
    };
    let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    files.sort();
    let (mut cases, mut ticks) = (0, 0);
    for f in files {
        let (c, t, _) = replay_file(&f, false);
        cases += c;
        ticks += t;
    }
    eprintln!("1.16.5 entities cache through the engine: {cases} cases, {ticks} ticks");
}

fn nether_file() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.16.5/nether.jsonl.gz")
}

/// The nether goldens (lava pushes with 0.007 in an ultrawarm dimension) through the engine: the
/// dimension type arrives in the join packet; without and with entity tracking.
#[test]
fn nether_matches_the_oracle_1_16_5() {
    let (cases, ticks, skipped) = replay_file_with(&nether_file(), false, false);
    assert!(
        cases >= 100 && ticks > 5000 && skipped == 0,
        "{cases} cases, {ticks} ticks, {skipped} skipped"
    );
    replay_file_with(&nether_file(), false, true);
}

/// The cached nether fuzz (`oracle/cache/1.16.5/fuzz-nether/*.jsonl.gz`, not committed) through
/// the engine; `GHAST_FUZZ_NETHER` names another directory. Skipped when absent.
#[test]
#[ignore = "cache run: cargo test --release -p ghast-client --lib -- --ignored nether_cache_1_16_5"]
fn nether_cache_1_16_5() {
    let dir = std::env::var("GHAST_FUZZ_NETHER").map_or_else(
        |_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../oracle/cache/1.16.5/fuzz-nether")
        },
        std::path::PathBuf::from,
    );
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("skipped: {} not found", dir.display());
        return;
    };
    let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    files.sort();
    let (mut cases, mut ticks) = (0, 0);
    for f in files {
        let (c, t, _) = replay_file_with(&f, false, false);
        cases += c;
        ticks += t;
    }
    eprintln!("1.16.5 nether cache through the engine: {cases} cases, {ticks} ticks");
}

/// The dimension type follows the join packet and a respawn into another world; a respawn into
/// the same world keeps it (vanilla keeps the world there).
#[test]
fn dimension_type_from_join_and_respawn_1_16_5() {
    let mut sim = Sim16::new();
    sim.world_packet(&login_packet("minecraft:overworld", false))
        .unwrap();
    assert!(!sim.test_ext().ultrawarm);
    sim.world_packet(&respawn_packet("minecraft:the_nether", true))
        .unwrap();
    assert!(sim.test_ext().ultrawarm);
    // Same world name: the client keeps its world and its dimension type.
    sim.world_packet(&respawn_packet("minecraft:the_nether", false))
        .unwrap();
    assert!(sim.test_ext().ultrawarm);
    sim.world_packet(&respawn_packet("minecraft:overworld", false))
        .unwrap();
    assert!(!sim.test_ext().ultrawarm);
    sim.world_packet(&login_packet("minecraft:the_nether", true))
        .unwrap();
    assert!(sim.test_ext().ultrawarm);
}

fn riptide_file() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.16.5/riptide.jsonl.gz")
}

/// The riptide goldens through the engine: the trident (with its riptide enchantment in the
/// stack NBT) in the main hand, the rain as game state packets, the server's spin flag as our
/// entity data, the use key from the behaviour; every movement, `entity_action`, `use_item` and
/// `release_use_item` packet and the velocity of every tick must equal the real client's.
/// With entity tracking (the spin's hits need it). The oracle uses the item with the
/// crosshair on nothing: a case whose player looks at a block in reach when it uses the item
/// ends there (the engine uses the item on the block first, as the real client would).
#[test]
fn riptide_matches_the_oracle_1_16_5() {
    let (cases, ticks, skipped) = replay_file_with(&riptide_file(), false, true);
    assert!(
        cases >= 100 && ticks > 4000 && skipped == 0,
        "{cases} cases, {ticks} ticks, {skipped} skipped"
    );
}

/// The cached riptide fuzz (`oracle/cache/1.16.5/fuzz-riptide/*.jsonl.gz`, not committed)
/// through the engine; `GHAST_FUZZ_RIPTIDE` names another directory. Skipped when absent.
#[test]
#[ignore = "cache run: cargo test --release -p ghast-client --lib -- --ignored riptide_cache_1_16_5"]
fn riptide_cache_1_16_5() {
    let dir = std::env::var("GHAST_FUZZ_RIPTIDE").map_or_else(
        |_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../oracle/cache/1.16.5/fuzz-riptide")
        },
        std::path::PathBuf::from,
    );
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("skipped: {} not found", dir.display());
        return;
    };
    let mut files: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    files.sort();
    let (mut cases, mut ticks) = (0, 0);
    for f in files {
        let (c, t, _) = replay_file_with(&f, false, true);
        cases += c;
        ticks += t;
    }
    eprintln!("1.16.5 riptide cache through the engine: {cases} cases, {ticks} ticks");
}

/// The riptide's rain check: the level rains, the biome rains (by the biome cell the client's
/// voronoi zoom picks) and the dimension has sky light; a riptide trident in the hand also
/// makes the sim read our entity data (the spin flag).
#[test]
fn riptide_rain_and_spin_flag_1_16_5() {
    use ghast_protocol::v1_16_5::play::clientbound as cb;
    let chunk = |biome: i32| {
        RawPacket::from_packet(&cb::MapChunk {
            x: 0,
            z: 0,
            ground_up: true,
            bit_map: 0,
            heightmaps: ghast_protocol::nbt::RawNbt::from_bytes(bytes::Bytes::from_static(&[
                10, 0, 0, 0,
            ]))
            .unwrap(),
            biomes: Some(vec![biome; 1024]),
            chunk_data: bytes::Bytes::new(),
            block_entities: Vec::new(),
        })
        .unwrap()
    };
    let trident = ghast_data::v1_16_5::Item::from_name("trident")
        .unwrap()
        .id();
    let stack = ghast_inventory::Stack::new(i32::from(trident), 1, 0)
        .with_nbt(Some(enchantments_nbt(&[("riptide", 3)])));
    let usable = |sim: &Sim16| {
        let held = sim.held_item(trident, 1, 0);
        sim.item_use_facts(&stack, false, held).usable
    };
    let mut p = ghast_physics::v1_16_5::PlayerState::new([8.5, 64.0, 8.5], 0.0, 0.0);
    p.on_ground = true;
    // The world keeps biomes (`WorldConfig::biomes`; without them every biome counts as rainy).
    let opts = crate::sim::SimOptions {
        world: Some(ghast_world::WorldConfig::default().with_biomes(true)),
        ..Default::default()
    };
    for (biome, overworld, rain, want) in [
        (1, true, true, true),   // plains
        (2, true, true, false),  // desert: no precipitation
        (12, true, true, false), // snowy tundra: snow
        (1, true, false, false), // no rain
        (8, false, true, false), // the nether: no sky light
        (1, false, true, false), // a rainy biome in a dimension without sky light
    ] {
        let mut sim = Sim16::with(&opts);
        let name = if overworld {
            "minecraft:overworld"
        } else {
            "minecraft:the_nether"
        };
        sim.world_packet(&login_packet(name, !overworld)).unwrap();
        sim.world_packet(&chunk(biome)).unwrap();
        sim.test_set_player(p);
        if rain {
            sim.weather(1, 0.0);
            sim.weather(7, 1.0);
        }
        assert_eq!(
            usable(&sim),
            want,
            "biome {biome}, overworld {overworld}, rain {rain}"
        );
    }
    // A riptide trident in a hand: our entity data joins the packet filter; the spin flag
    // arrives with it.
    let mut sim = Sim16::new();
    sim.world_packet(&login_packet("minecraft:overworld", false))
        .unwrap();
    assert!(!sim.world_packet_ids().contains(&cb::id::ENTITY_METADATA));
    let _ = usable(&sim);
    assert!(sim.take_filter_dirty());
    assert!(!sim.take_filter_dirty());
    assert!(sim.world_packet_ids().contains(&cb::id::ENTITY_METADATA));
    sim.world_packet(&living_flags_packet(4)).unwrap();
    assert!(sim.test_ext().spin_attack);
    sim.world_packet(&living_flags_packet(1)).unwrap();
    assert!(!sim.test_ext().spin_attack);
}
