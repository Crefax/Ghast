//! The engine replay of the 1.8.9 boat goldens (`tests/golden/1.8.9/boats.jsonl.gz`): the boat
//! arrives as a real 1.8 spawn-object packet through the sim's tracker, the player boards it by
//! `attach_entity`, the server's teleports, relative moves and velocity of the boat are real 1.8
//! packets, and every packet of every tick must equal vanilla's.

use super::tests::opts;
use super::*;
use crate::common::{EntityAction, JoinGame};
use crate::phys_golden as golden;
use crate::sim::Sim18;
use ghast_protocol::RawPacket;
use ghast_protocol::v1_8::play::clientbound as cb;
use serde_json::Value;
use std::io::BufRead;

/// A packet body under construction (1.8 wire types).
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

/// The boat of the riding cases.
const BOAT: i32 = 100;

/// The 1.8 packet of one oracle boat event (see `oracle/src/mc_1_8_9/.../Boats.java`).
fn packet(ev: &Value) -> RawPacket {
    let n = |k: &str| ev[k].as_i64().unwrap() as i32;
    match ev["type"].as_str().unwrap() {
        "ride" => Body::default()
            .i32(1)
            .i32(if ev["on"].as_bool().unwrap() {
                BOAT
            } else {
                -1
            })
            .bool(false)
            .raw(cb::id::ATTACH_ENTITY),
        "boat_teleport" => Body::default()
            .varint(BOAT)
            .i32(n("x"))
            .i32(n("y"))
            .i32(n("z"))
            .i8(n("yaw") as i8)
            .i8(n("pitch") as i8)
            .bool(ev["on_ground"].as_bool().unwrap())
            .raw(cb::id::ENTITY_TELEPORT),
        "boat_move" => Body::default()
            .varint(BOAT)
            .i8(n("dx") as i8)
            .i8(n("dy") as i8)
            .i8(n("dz") as i8)
            .bool(ev["on_ground"].as_bool().unwrap())
            .raw(cb::id::REL_ENTITY_MOVE),
        "boat_velocity" => Body::default()
            .varint(BOAT)
            .i16(n("x") as i16)
            .i16(n("y") as i16)
            .i16(n("z") as i16)
            .raw(cb::id::ENTITY_VELOCITY),
        other => panic!("event {other}"),
    }
}

/// The boat's spawn-object packet (type 1, no motion), as the oracle sends it at setup.
fn spawn_packet(boat: &Value) -> RawPacket {
    let p = &boat["pos"];
    let fixed = |k: usize| (f64v(&p[k]) * 32.0).floor() as i32;
    let yaw = (f32v(&boat["yaw"]) * 256.0 / 360.0).floor() as i32 as i8;
    Body::default()
        .varint(BOAT)
        .i8(1)
        .i32(fixed(0))
        .i32(fixed(1))
        .i32(fixed(2))
        .i8(0)
        .i8(yaw)
        .i32(0)
        .raw(cb::id::SPAWN_ENTITY)
}

/// A 1.8 join packet: our entity id 1, survival, the overworld.
fn login_packet() -> RawPacket {
    Body::default()
        .i32(1)
        .i8(0)
        .i8(0)
        .i8(1)
        .i8(20)
        .string("flat")
        .bool(false)
        .raw(cb::id::LOGIN)
}

/// The packets of a tick, as comparable values.
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
        Outbound::SteerVehicle {
            sideways,
            forward,
            jump,
            sneak,
        } => serde_json::json!([
            "steer_vehicle",
            sideways.to_bits(),
            forward.to_bits(),
            jump,
            sneak
        ]),
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
        "steer_vehicle" => serde_json::json!([
            "steer_vehicle",
            f32v(&p["sideways"]).to_bits(),
            f32v(&p["forward"]).to_bits(),
            p["jump"],
            p["sneak"]
        ]),
        "entity_action" => serde_json::json!(["entity_action", p["action_name"]]),
        _ => return None,
    })
}

struct Keys(Controls);

impl Behavior for Keys {
    fn name(&self) -> &str {
        "boats-replay-1.8"
    }
    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        ctx.set_controls(self.0);
    }
}

/// The JVM's `Math.sin` / `Math.cos` of a tick record (`jvm`): argument bits, sine, cosine.
fn jvm_trig(rec: &Value) -> Vec<(u64, f64, f64)> {
    let mut v = Vec::new();
    if let Some(j) = rec.get("jvm") {
        if j.get("in_arg").is_some() {
            let a = f64::from(f32v(&j["in_arg"]));
            v.push((a.to_bits(), f64v(&j["in_sin"]), f64v(&j["in_cos"])));
        }
        v.push((
            f64v(&j["seat_arg"]).to_bits(),
            f64v(&j["seat_sin"]),
            f64v(&j["seat_cos"]),
        ));
    }
    v
}

/// Replays every case of a 1.8.9 boat file through the engine with entity tracking; returns
/// (cases, ticks, cases that differ). `jvm`: the boat's sine / cosine are the oracle JVM's (from
/// the records), else the platform's (what the client computes).
fn replay_file(path: &std::path::Path, jvm: bool) -> (usize, usize, Vec<String>) {
    let f = std::fs::File::open(path).unwrap();
    // One case in memory at a time (the cached files are large).
    let mut lines = std::io::BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str::<Value>(&l.unwrap()).unwrap())
        .peekable();
    let (mut cases, mut ticks_done) = (0, 0);
    let mut failed = Vec::new();
    while let Some(sc) = lines.next() {
        let name = sc["name"].as_str().unwrap().to_owned();
        let mut ticks = Vec::new();
        while let Some(t) = lines.next_if(|l| l["type"] == "tick") {
            ticks.push(t);
        }
        cases += 1;
        let setup = &sc["setup"];
        let mut e = Engine::new(opts(GameVersion::V1_8_9), Sim18::new());
        e.handle(Inbound::JoinGame(JoinGame {
            entity_id: 1,
            game_mode: GameMode::Survival,
            dimension: "0".into(),
            hardcore: false,
            dimension_nbt: None,
            respawn_screen: true,
        }));
        // What `Client` sets up for `Interests::ENTITY_TRACKING` on 1.8.9.
        e.sim.set_entities(Some(
            ghast_entity::Config::default()
                .with_metadata(false)
                .with_equipment(false)
                .with_effects(false)
                .with_player_list(false)
                .with_player_names(false)
                .with_events(false),
        ));
        e.sim.world_packet(&login_packet()).unwrap();
        for r in sc["loaded_chunks"].as_array().unwrap() {
            let r: Vec<i32> = (0..4).map(|k| r[k].as_i64().unwrap() as i32).collect();
            for x in r[0]..=r[2] {
                for z in r[1]..=r[3] {
                    e.sim
                        .world_packet(&crate::sim::test_chunks::v1_8(x, z))
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
            let st = ghast_data::v1_8::BlockState::from_raw_vanilla(
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
        let p = golden::player_of(&sc, &ticks[0]);
        let mut pose = e.core.pose;
        e.sim.test_set_player(&mut pose, p);
        e.core.pose = pose;
        e.sim
            .world_packet(&spawn_packet(&setup["ext"]["boat"]))
            .unwrap();
        e.core.ready = true;
        e.outbound().clear();
        let t0 = Instant::now();
        for (k, rec) in ticks.iter().enumerate().skip(1) {
            let input = &rec["input"];
            for ev in input["events"].as_array().into_iter().flatten() {
                e.sim.world_packet(&packet(ev)).unwrap();
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
            if jvm {
                e.sim.test_set_trig(jvm_trig(rec));
            }
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
            ticks_done += 1;
        }
    }
    eprintln!(
        "1.8.9 {} through the engine: {cases} cases, {ticks_done} ticks, {} cases differ",
        path.display(),
        failed.len()
    );
    (cases, ticks_done, failed)
}

/// The 1.8.9 boat goldens through the engine: boarding by `attach_entity`, the boat the client
/// simulates, `look` + `steer_vehicle` every tick, the server's boat packets, the dismount.
#[test]
fn boats_match_the_oracle_1_8_9() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.8.9/boats.jsonl.gz");
    let (cases, ticks, failed) = replay_file(&path, true);
    assert!(
        failed.is_empty(),
        "{} cases differ:\n{}",
        failed.len(),
        failed.join("\n")
    );
    assert!(cases >= 118 && ticks > 7000, "{cases} cases, {ticks} ticks");
}

/// The cached boat fuzz (`oracle/cache/1.8.9/fuzz-boats/`, git-ignored) through the engine.
#[test]
#[ignore = "needs the oracle cache"]
fn boats_cache_match_the_oracle_1_8_9() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../oracle/cache/1.8.9/fuzz-boats");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("skipped: {} not found", dir.display());
        return;
    };
    let mut files: Vec<_> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".jsonl.gz"))
        .collect();
    files.sort();
    let (mut cases, mut ticks, mut failed) = (0, 0, Vec::new());
    for f in &files {
        let (c, t, x) = replay_file(f, true);
        cases += c;
        ticks += t;
        failed.extend(x);
    }
    eprintln!("1.8.9 boat fuzz cache through the engine: {cases} cases, {ticks} ticks");
    assert!(
        failed.is_empty(),
        "{} cases differ:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// The cached boat fuzz through the engine with the platform's sine and cosine (what the client
/// computes): the cases whose packets differ from the oracle JVM's (a 1-ulp seat after a
/// dismount) are listed, not failed.
#[test]
#[ignore = "needs the oracle cache"]
fn boats_cache_platform_trig_1_8_9() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../oracle/cache/1.8.9/fuzz-boats");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("skipped: {} not found", dir.display());
        return;
    };
    let mut files: Vec<_> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".jsonl.gz"))
        .collect();
    files.sort();
    let (mut cases, mut differ) = (0, 0);
    for f in &files {
        let (c, _, x) = replay_file(f, false);
        cases += c;
        differ += x.len();
        for d in x {
            eprintln!("platform trig: {}", &d[..d.len().min(200)]);
        }
    }
    eprintln!("1.8.9 boat fuzz cache, platform trig: {differ} of {cases} cases differ");
}
