//! Replays oracle golden vectors (format: `oracle/README.md`) against `ghast_physics::v1_8` and
//! compares every recorded field bit for bit.
#![allow(dead_code)]

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use flate2::read::MultiGzDecoder;
#[cfg(feature = "v1_8")]
use ghast_physics::v1_8::{self, Effects, MovementInput, PlayerState, SentState};
#[cfg(feature = "v1_8")]
use ghast_physics::{Abilities, Packets, TickInput};
use ghast_physics::{Keys, Packet};
use serde_json::{Value, json};

#[cfg(feature = "v1_12")]
pub mod v112;
#[cfg(feature = "v1_16_5")]
pub mod v116;
#[cfg(feature = "v26_1")]
pub mod v261;

/// Repository root (`crates/ghast-physics/../..`).
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Line reader over a `.jsonl` or `.jsonl.gz` file.
pub fn open_lines(path: &Path) -> Box<dyn Iterator<Item = String>> {
    let f = File::open(path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let r: Box<dyn Read> = if path.extension().is_some_and(|e| e == "gz") {
        Box::new(MultiGzDecoder::new(f))
    } else {
        Box::new(f)
    };
    Box::new(
        BufReader::with_capacity(1 << 20, r)
            .lines()
            .map(|l| l.expect("read line")),
    )
}

/// One case: the scenario header and its tick records.
pub struct Case {
    pub header: Value,
    pub ticks: Vec<Value>,
}

/// Streams the cases of a golden file.
pub fn cases(path: &Path) -> impl Iterator<Item = Case> {
    let mut lines = open_lines(path).peekable();
    std::iter::from_fn(move || {
        let header: Value = loop {
            let l = lines.next()?;
            if l.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(&l).expect("json");
            assert_eq!(v["type"], "scenario", "expected a scenario line");
            break v;
        };
        let mut ticks = Vec::new();
        while let Some(l) = lines.peek() {
            if l.contains("\"type\":\"scenario\"") || l.contains("\"type\": \"scenario\"") {
                break;
            }
            let l = lines.next().unwrap();
            if l.trim().is_empty() {
                continue;
            }
            ticks.push(serde_json::from_str(&l).expect("json"));
        }
        Some(Case { header, ticks })
    })
}

pub fn f64_of(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v[1].as_str().expect("[dec, hex]"), 16).unwrap())
}

pub fn f32_of(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v[1].as_str().expect("[dec, hex]"), 16).unwrap())
}

pub fn f3(v: &Value) -> [f64; 3] {
    [f64_of(&v[0]), f64_of(&v[1]), f64_of(&v[2])]
}

pub fn b(v: &Value) -> bool {
    v.as_bool().expect("bool")
}

pub fn i(v: &Value) -> i32 {
    v.as_i64().expect("int") as i32
}

/// Encodes a float like the oracle: `[decimal, "hex"]` (only the hex is compared).
pub fn jf64(x: f64) -> Value {
    json!([null, format!("{:016x}", x.to_bits())])
}

pub fn jf32(x: f32) -> Value {
    json!([null, format!("{:08x}", x.to_bits())])
}

#[cfg(feature = "v1_8")]
/// Builds the case's world.
pub fn world_of(header: &Value) -> v1_8::MemoryWorld {
    let mut w = v1_8::memory_world();
    for r in header["loaded_chunks"].as_array().unwrap() {
        let r: Vec<i32> = r.as_array().unwrap().iter().map(i).collect();
        w.load_chunks(r[0], r[1], r[2], r[3]);
    }
    for f in header["fills"].as_array().unwrap() {
        let f: Vec<i32> = f.as_array().unwrap().iter().map(i).collect();
        let raw = ((f[6] as u16) << 4) | (f[7] as u16 & 15);
        w.fill_raw([f[0], f[1], f[2]], [f[3], f[4], f[5]], raw);
    }
    w
}

#[cfg(feature = "v1_8")]
/// The player state from the tick-0 record (plus header-only data: speed base, effects).
pub fn player_of(header: &Value, t0: &Value) -> PlayerState {
    let s = &t0["state"];
    let bb = &s["bb"];
    let mut p = PlayerState::new(f3(&s["pos"]), f32_of(&s["yaw"]), f32_of(&s["pitch"]));
    p.bb = ghast_physics::math::Aabb::new(
        f64_of(&bb[0]),
        f64_of(&bb[1]),
        f64_of(&bb[2]),
        f64_of(&bb[3]),
        f64_of(&bb[4]),
        f64_of(&bb[5]),
    );
    p.vel = f3(&s["vel"]);
    p.on_ground = b(&s["on_ground"]);
    p.horizontal_collision = b(&s["horizontal_collision"]);
    p.vertical_collision = b(&s["vertical_collision"]);
    p.velocity_dirty = b(&s["velocity_dirty"]);
    p.fall_distance = f32_of(&s["fall_distance"]);
    p.in_web = b(&s["in_web"]);
    p.touching_water = b(&s["touching_water"]);
    p.outside_world_border = b(&s["outside_world_border"]);
    p.sprinting = b(&s["sprinting"]);
    p.abilities = Abilities {
        invulnerable: b(&s["invulnerable"]),
        flying: b(&s["flying"]),
        allow_flying: b(&s["allow_flying"]),
        creative: b(&s["creative"]),
        fly_speed: f32_of(&s["fly_speed"]),
        walk_speed: f32_of(&s["walk_speed"]),
    };
    p.jump_cooldown = i(&s["jump_cooldown"]);
    p.jumping = b(&s["jumping"]);
    p.forward_speed = f32_of(&s["forward_speed"]);
    p.sideways_speed = f32_of(&s["sideways_speed"]);
    p.flying_speed = f32_of(&s["flying_speed"]);
    p.movement_speed = f32_of(&s["movement_speed"]);
    p.movement_speed_base = f64_of(&header["setup"]["movement_speed_base"]);
    if let Some(mods) = header["setup"]
        .get("speed_modifiers")
        .and_then(Value::as_array)
    {
        // The oracle gives modifier k the UUID (most 0x6f7261636c65, least k).
        for (k, m) in mods.iter().enumerate() {
            let uuid = (0x6f72_6163_6c65_u128 << 64) | k as u128;
            p.speed_modifiers
                .insert(uuid, f64_of(&m[0]), i(&m[1]) as u8);
        }
    }
    p.speed_modifiers.set_sprint(p.sprinting);
    p.spectator = header["setup"]["game_mode"] == "spectator";
    p.depth_strider = header["setup"]
        .get("depth_strider")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u8;
    p.fly_toggle_timer = i(&s["fly_toggle_timer"]);
    p.sprint_toggle_timer = i(&s["sprint_toggle_timer"]);
    p.sprinting_ticks_left = i(&s["sprinting_ticks_left"]);
    p.food_level = i(&s["food_level"]);
    p.ticks_alive = i(&s["ticks_alive"]);
    let inp = &s["input"];
    p.input = MovementInput {
        forward: f32_of(&inp["forward"]),
        sideways: f32_of(&inp["sideways"]),
        jump: b(&inp["jump"]),
        sneak: b(&inp["sneak"]),
    };
    let sent = &s["sent"];
    p.sent = SentState {
        pos: f3(&sent["pos"]),
        yaw: f32_of(&sent["yaw"]),
        pitch: f32_of(&sent["pitch"]),
        sprinting: b(&sent["sprinting"]),
        sneaking: b(&sent["sneaking"]),
        ticks_since_pos: i(&sent["ticks_since_pos"]),
    };
    let mut effects = Effects::default();
    for e in header["setup"]["effects"].as_array().unwrap() {
        match (i(&e[0]), i(&e[1])) {
            (8, amp) => effects.jump_boost = Some(amp),
            (15, _) => effects.blindness = true,
            (id, _) => panic!("effect {id} not modelled"),
        }
    }
    p.effects = effects;
    p
}

#[cfg(feature = "v1_8")]
/// The state in the oracle's record layout.
pub fn state_json(p: &PlayerState, w: &v1_8::MemoryWorld, using_item: bool) -> Value {
    let bb = p.bb;
    json!({
        "pos": [jf64(p.pos[0]), jf64(p.pos[1]), jf64(p.pos[2])],
        "bb": [jf64(bb.min_x), jf64(bb.min_y), jf64(bb.min_z), jf64(bb.max_x), jf64(bb.max_y), jf64(bb.max_z)],
        "vel": [jf64(p.vel[0]), jf64(p.vel[1]), jf64(p.vel[2])],
        "yaw": jf32(p.yaw),
        "pitch": jf32(p.pitch),
        "on_ground": p.on_ground,
        "horizontal_collision": p.horizontal_collision,
        "vertical_collision": p.vertical_collision,
        "colliding": p.colliding(),
        "velocity_dirty": p.velocity_dirty,
        "fall_distance": jf32(p.fall_distance),
        "in_web": p.in_web,
        "touching_water": p.touching_water,
        "touching_lava": v1_8::is_touching_lava(p, w),
        "climbing": v1_8::is_climbing(p, w),
        "sprinting": p.sprinting,
        "sneaking": p.sneaking(),
        "outside_world_border": p.outside_world_border,
        "no_clip": p.no_clip,
        "step_height": jf32(v1_8::STEP_HEIGHT),
        "flying": p.abilities.flying,
        "allow_flying": p.abilities.allow_flying,
        "creative": p.abilities.creative,
        "invulnerable": p.abilities.invulnerable,
        "fly_speed": jf32(p.abilities.fly_speed),
        "walk_speed": jf32(p.abilities.walk_speed),
        "jump_cooldown": p.jump_cooldown,
        "jumping": p.jumping,
        "forward_speed": jf32(p.forward_speed),
        "sideways_speed": jf32(p.sideways_speed),
        "flying_speed": jf32(p.flying_speed),
        "movement_speed": jf32(p.movement_speed),
        "movement_speed_attr": jf64(p.movement_speed_attr()),
        "fly_toggle_timer": p.fly_toggle_timer,
        "sprint_toggle_timer": p.sprint_toggle_timer,
        "sprinting_ticks_left": p.sprinting_ticks_left,
        "food_level": p.food_level,
        "ticks_alive": p.ticks_alive,
        "using_item": using_item,
        "input": {
            "forward": jf32(p.input.forward),
            "sideways": jf32(p.input.sideways),
            "jump": p.input.jump,
            "sneak": p.input.sneak,
        },
        "sent": {
            "pos": [jf64(p.sent.pos[0]), jf64(p.sent.pos[1]), jf64(p.sent.pos[2])],
            "yaw": jf32(p.sent.yaw),
            "pitch": jf32(p.sent.pitch),
            "sprinting": p.sent.sprinting,
            "sneaking": p.sent.sneaking,
            "ticks_since_pos": p.sent.ticks_since_pos,
        },
    })
}

fn action(id: i32, name: &str) -> Value {
    json!({"name": "entity_action", "action": id, "action_name": name})
}

/// A packet in the oracle's record layout.
pub fn packet_json(p: &Packet) -> Value {
    match *p {
        Packet::StartSneaking => action(0, "start_sneaking"),
        Packet::StopSneaking => action(1, "stop_sneaking"),
        Packet::StartSprinting => action(3, "start_sprinting"),
        Packet::StopSprinting => action(4, "stop_sprinting"),
        Packet::Abilities(a) => json!({
            "name": "abilities", "invulnerable": a.invulnerable, "flying": a.flying,
            "allow_flying": a.allow_flying, "creative": a.creative,
            "fly_speed": jf32(a.fly_speed), "walk_speed": jf32(a.walk_speed),
        }),
        Packet::Flying { on_ground } => json!({"name": "flying", "on_ground": on_ground}),
        Packet::Position { x, y, z, on_ground } => json!({
            "name": "position", "pos": [jf64(x), jf64(y), jf64(z)], "on_ground": on_ground,
        }),
        Packet::Look {
            yaw,
            pitch,
            on_ground,
        } => {
            json!({"name": "look", "yaw": jf32(yaw), "pitch": jf32(pitch), "on_ground": on_ground})
        }
        Packet::PositionLook {
            x,
            y,
            z,
            yaw,
            pitch,
            on_ground,
        } => json!({
            "name": "position_look", "pos": [jf64(x), jf64(y), jf64(z)],
            "yaw": jf32(yaw), "pitch": jf32(pitch), "on_ground": on_ground,
        }),
        other => unimplemented!("{other:?}"),
    }
}

/// Held keys and the "use" (right mouse) key of a tick record.
pub fn keys_of(s: &str) -> (Keys, bool) {
    let mut k = Keys::NONE;
    let mut using = false;
    for name in s.split(',').filter(|n| !n.is_empty()) {
        match name {
            "forward" => k.forward = true,
            "back" => k.back = true,
            "left" => k.left = true,
            "right" => k.right = true,
            "jump" => k.jump = true,
            "sneak" => k.sneak = true,
            "sprint" => k.sprint = true,
            "use" => using = true,
            other => panic!("unknown key {other}"),
        }
    }
    (k, using)
}

/// Compares `actual` against `expected`; floats compare by their hex bits. Returns the path and
/// both values of the first difference.
pub fn diff(path: &str, expected: &Value, actual: &Value) -> Option<(String, String, String)> {
    let is_float = |v: &Value| {
        v.as_array().is_some_and(|a| {
            a.len() == 2 && a[1].is_string() && (a[0].is_number() || a[0].is_null())
        })
    };
    if is_float(expected) {
        let e = expected[1].as_str().unwrap();
        let a = actual.get(1).and_then(Value::as_str).unwrap_or("<missing>");
        if e != a {
            return Some((path.to_string(), describe(e), describe(a)));
        }
        return None;
    }
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                let av = a.get(k).unwrap_or(&Value::Null);
                if let Some(d) = diff(&format!("{path}.{k}"), ev, av) {
                    return Some(d);
                }
            }
            None
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                return Some((
                    format!("{path}.len"),
                    format!("{} {expected}", e.len()),
                    format!("{} {actual}", a.len()),
                ));
            }
            e.iter()
                .zip(a)
                .enumerate()
                .find_map(|(n, (ev, av))| diff(&format!("{path}[{n}]"), ev, av))
        }
        _ if expected == actual => None,
        _ => Some((path.to_string(), expected.to_string(), actual.to_string())),
    }
}

/// Hex bits plus the decimal value they encode.
pub fn describe(hex: &str) -> String {
    match hex.len() {
        16 => format!(
            "{hex} ({:?})",
            f64::from_bits(u64::from_str_radix(hex, 16).unwrap())
        ),
        8 => format!(
            "{hex} ({:?})",
            f32::from_bits(u32::from_str_radix(hex, 16).unwrap())
        ),
        _ => hex.to_string(),
    }
}

/// The first difference of a case replay.
pub struct Mismatch {
    pub case: String,
    pub tick: i64,
    pub field: String,
    pub expected: String,
    pub actual: String,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} tick {}: {}\n    expected {}\n    actual   {}",
            self.case, self.tick, self.field, self.expected, self.actual
        )
    }
}

/// Packet names ghast-physics produces.
pub const MOVEMENT_PACKETS: [&str; 6] = [
    "entity_action",
    "abilities",
    "flying",
    "position",
    "look",
    "position_look",
];

#[cfg(feature = "v1_8")]
/// Replays one case. Returns the number of ticks compared or the first mismatch.
pub fn replay(case: &Case) -> Result<usize, Mismatch> {
    let name = case.header["name"].as_str().unwrap_or("?").to_string();
    let world = world_of(&case.header);
    let t0 = &case.ticks[0];
    assert_eq!(t0["t"], 0);
    let mut p = player_of(&case.header, t0);
    let mm = |tick: i64, (field, expected, actual): (String, String, String)| Mismatch {
        case: name.clone(),
        tick,
        field,
        expected,
        actual,
    };
    // The tick-0 record itself must round-trip (checks the initialisation).
    if let Some(d) = diff("state", &t0["state"], &state_json(&p, &world, false)) {
        return Err(mm(0, d));
    }
    let mut out = Packets::new();
    for rec in &case.ticks[1..] {
        let t = rec["t"].as_i64().unwrap();
        let input = &rec["input"];
        out.clear();
        let mut sent: Vec<Value> = Vec::new();
        if let Some(events) = input.get("events").and_then(Value::as_array) {
            for e in events {
                match e["type"].as_str().unwrap() {
                    "velocity" => p.apply_velocity_packet(i(&e["x"]), i(&e["y"]), i(&e["z"])),
                    "teleport" => {
                        let reply = p.apply_teleport(
                            f3(&e["pos"]),
                            f32_of(&e["yaw"]),
                            f32_of(&e["pitch"]),
                            [false; 5],
                        );
                        sent.push(packet_json(&reply));
                    }
                    "abilities" => p.apply_abilities(Abilities {
                        invulnerable: b(&e["invulnerable"]),
                        flying: b(&e["flying"]),
                        allow_flying: b(&e["allow_flying"]),
                        creative: b(&e["creative"]),
                        fly_speed: f32_of(&e["fly_speed"]),
                        walk_speed: f32_of(&e["walk_speed"]),
                    }),
                    other => panic!("unknown event {other}"),
                }
            }
        }
        let (keys, using_item) = keys_of(input["keys"].as_str().unwrap());
        let ti = TickInput {
            keys,
            yaw: f32_of(&input["yaw"]),
            pitch: f32_of(&input["pitch"]),
            using_item,
        };
        // The oracle's harness never settles anvil / chest bounds.
        let options = v1_8::Options {
            history_shapes: v1_8::HistoryShapes::Constructor,
        };
        v1_8::tick_with(&mut p, &ti, &world, &options, &mut out);
        sent.extend(out.as_slice().iter().map(packet_json));
        if let Some(d) = diff("state", &rec["state"], &state_json(&p, &world, using_item)) {
            return Err(mm(t, d));
        }
        // Item-use packets (block_place / release) are interactions the client crate sends
        // before the movement; ghast-physics only produces movement-related packets.
        let expected: Vec<Value> = rec["packets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| MOVEMENT_PACKETS.contains(&p["name"].as_str().unwrap_or("")))
            .cloned()
            .collect();
        if let Some(d) = diff("packets", &Value::Array(expected), &Value::Array(sent)) {
            return Err(mm(t, d));
        }
    }
    Ok(case.ticks.len() - 1)
}

/// Result of replaying a file.
#[derive(Default)]
pub struct Summary {
    pub cases: usize,
    pub ticks: usize,
    pub failures: Vec<Mismatch>,
}

#[cfg(feature = "v1_8")]
/// Replays every case of a file.
pub fn replay_file(path: &Path) -> Summary {
    let mut s = Summary::default();
    for case in cases(path) {
        s.cases += 1;
        match replay(&case) {
            Ok(n) => s.ticks += n,
            Err(m) => s.failures.push(m),
        }
    }
    s
}
