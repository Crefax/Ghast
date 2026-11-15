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

struct Keys(Controls);

impl Behavior for Keys {
    fn name(&self) -> &str {
        "entities-replay-1.16.5"
    }
    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        ctx.set_controls(self.0);
    }
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
                | "effects"
        )
    )
}

/// Replays the cases of an entities file through the engine with entity tracking (`metadata`:
/// with the generic metadata store): every case whose events this replay drives. Returns
/// (cases, ticks, cases skipped).
pub(super) fn replay_file(path: &std::path::Path, metadata: bool) -> (usize, usize, usize) {
    let f = std::fs::File::open(path).unwrap();
    // One case at a time (the cached files are large).
    let mut lines = std::io::BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str::<Value>(&l.unwrap()).unwrap())
        .peekable();
    let (mut cases, mut skipped, mut ticks_done, mut packets, mut orders) = (0, 0, 0, 0, 0);
    // Cases cut short where the player left the loaded columns.
    let mut left = 0;
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
        e.sim.test_tracker_login(1);
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
            let mut b = Keys(Controls {
                forward: has("forward"),
                back: has("back"),
                left: has("left"),
                right: has("right"),
                jump: has("jump"),
                sneak: has("sneak"),
                sprint: has("sprint"),
            });
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
        "1.16.5 {} through the engine (metadata {metadata}): {cases} cases ({skipped} skipped, {left} left the loaded columns), {ticks_done} ticks, {orders} tick orders, {packets} packets identical",
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
