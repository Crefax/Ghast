//! 1.12.2 golden replay (format: `oracle/README.md`, section "1.12.2"): the 1.8.9 record layout
//! plus the 1.9+ fields (`width`, `height`, `upward_speed`, `sent.on_ground`, `teleport_confirm`).

use ghast_physics::v1_12::{self, Effects, MovementInput, PlayerState, SentState};
use ghast_physics::{Abilities, Packets, TickInput};
use serde_json::{Value, json};

use super::{Case, Mismatch, Summary, b, cases, diff, f3, f32_of, f64_of, i, jf32, jf64, keys_of};

/// Packets ghast-physics produces plus the teleport confirmation the replay adds itself.
const PACKETS: [&str; 7] = [
    "entity_action",
    "abilities",
    "flying",
    "position",
    "look",
    "position_look",
    "teleport_confirm",
];

/// The case's world: `[x0, y0, z0, x1, y1, z1, id, meta]` fills inside the loaded rectangles.
pub fn world_of(header: &Value) -> v1_12::MemoryWorld {
    let mut w = v1_12::memory_world();
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

fn effects_of(list: &Value) -> Effects {
    let mut effects = Effects::default();
    for e in list.as_array().unwrap() {
        match (i(&e[0]), i(&e[1])) {
            (8, amp) => effects.jump_boost = Some(amp),
            (15, _) => effects.blindness = true,
            (25, amp) => effects.levitation = Some(amp),
            (id, _) => panic!("effect {id} not modelled"),
        }
    }
    effects
}

fn uuid_of(s: &str) -> u128 {
    u128::from_str_radix(&s.replace('-', ""), 16).expect("uuid")
}

/// The player state from the tick-0 record (plus header-only data).
pub fn player_of(header: &Value, t0: &Value) -> PlayerState {
    let s = &t0["state"];
    let setup = &header["setup"];
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
    p.width = f32_of(&s["width"]);
    p.height = f32_of(&s["height"]);
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
    p.upward_speed = f32_of(&s["upward_speed"]);
    p.flying_speed = f32_of(&s["flying_speed"]);
    p.movement_speed = f32_of(&s["movement_speed"]);
    p.movement_speed_base = f64_of(&setup["movement_speed_base"]);
    if let Some(mods) = setup.get("speed_modifiers").and_then(Value::as_array) {
        // The oracle gives modifier k the UUID (most 0x6f7261636c65, least k).
        for (k, m) in mods.iter().enumerate() {
            let uuid = (0x6f72_6163_6c65_u128 << 64) | k as u128;
            p.speed_modifiers
                .insert(uuid, f64_of(&m[0]), i(&m[1]) as u8);
        }
    }
    p.speed_modifiers.set_sprint(p.sprinting);
    p.spectator = setup["game_mode"] == "spectator";
    p.depth_strider = setup
        .get("depth_strider")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u8;
    p.fly_toggle_timer = i(&s["fly_toggle_timer"]);
    p.sprint_toggle_timer = i(&s["sprint_toggle_timer"]);
    p.ticks_since_sprint_change = i(&s["sprinting_ticks_left"]);
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
        on_ground: b(&sent["on_ground"]),
    };
    p.effects = effects_of(&setup["effects"]);
    // The oracle's setup ran the chunk bookkeeping once at the spawn position.
    p.added_to_chunk = false;
    p
}

/// The state in the oracle's record layout.
pub fn state_json(p: &PlayerState, w: &v1_12::MemoryWorld, using_item: bool) -> Value {
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
        "touching_lava": v1_12::is_touching_lava(p, w),
        "climbing": v1_12::is_climbing(p, w),
        "sprinting": p.sprinting,
        "sneaking": p.sneaking(),
        "outside_world_border": p.outside_world_border,
        "no_clip": p.no_clip,
        "step_height": jf32(v1_12::STEP_HEIGHT),
        "width": jf32(p.width),
        "height": jf32(p.height),
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
        "upward_speed": jf32(p.upward_speed),
        "flying_speed": jf32(p.flying_speed),
        "movement_speed": jf32(p.movement_speed),
        "movement_speed_attr": jf64(p.movement_speed_attr()),
        "fly_toggle_timer": p.fly_toggle_timer,
        "sprint_toggle_timer": p.sprint_toggle_timer,
        "sprinting_ticks_left": p.ticks_since_sprint_change,
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
            "on_ground": p.sent.on_ground,
        },
    })
}

/// Replays one case. Returns the number of ticks compared or the first mismatch.
pub fn replay(case: &Case) -> Result<usize, Mismatch> {
    let name = case.header["name"].as_str().unwrap_or("?").to_string();
    let world = world_of(&case.header);
    let t0 = &case.ticks[0];
    assert_eq!(t0["t"], 0);
    let mut p = player_of(&case.header, t0);
    p.added_to_chunk = ghast_physics::BlockView::is_chunk_loaded(
        &world,
        (p.pos[0].floor() as i32) >> 4,
        (p.pos[2].floor() as i32) >> 4,
    );
    let mm = |tick: i64, (field, expected, actual): (String, String, String)| Mismatch {
        case: name.clone(),
        tick,
        field,
        expected,
        actual,
    };
    if let Some(d) = diff("state", &t0["state"], &state_json(&p, &world, false)) {
        return Err(mm(0, d));
    }
    let mut out = Packets::new();
    let mut teleport_id = 0;
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
                        let flags = e.get("flags").and_then(Value::as_u64).unwrap_or(0);
                        let rel = [0, 1, 2, 3, 4].map(|bit| flags & (1 << bit) != 0);
                        let reply = p.apply_teleport(
                            f3(&e["pos"]),
                            f32_of(&e["yaw"]),
                            f32_of(&e["pitch"]),
                            rel,
                        );
                        teleport_id += 1;
                        sent.push(json!({"name": "teleport_confirm", "teleport_id": teleport_id}));
                        sent.push(super::packet_json(&reply));
                    }
                    "abilities" => p.apply_abilities(Abilities {
                        invulnerable: b(&e["invulnerable"]),
                        flying: b(&e["flying"]),
                        allow_flying: b(&e["allow_flying"]),
                        creative: b(&e["creative"]),
                        fly_speed: f32_of(&e["fly_speed"]),
                        walk_speed: f32_of(&e["walk_speed"]),
                    }),
                    "effects" => p.effects = effects_of(&e["effects"]),
                    "speed_attribute" => {
                        let mods: Vec<(u128, f64, u8)> = e["modifiers"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|m| {
                                (
                                    uuid_of(m[0].as_str().unwrap()),
                                    f64_of(&m[1]),
                                    i(&m[2]) as u8,
                                )
                            })
                            .collect();
                        p.apply_speed_attribute(f64_of(&e["base"]), &mods);
                    }
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
        v1_12::tick(&mut p, &ti, &world, &mut out);
        sent.extend(out.as_slice().iter().map(super::packet_json));
        if let Some(from) = std::env::var("GHAST_DEBUG_FROM")
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            && t >= from
        {
            let s = &rec["state"];
            eprintln!(
                "t{t} ours pos {:?} vel {:?} og {} hc {} h {} bb {:?}\n    vanilla pos {:?} vel {:?} h {:?} bb {:?}",
                p.pos,
                p.vel,
                p.on_ground,
                p.horizontal_collision,
                p.height,
                p.bb,
                f3(&s["pos"]),
                f3(&s["vel"]),
                f32_of(&s["height"]),
                s["bb"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(f64_of)
                    .collect::<Vec<_>>()
            );
        }
        if let Some(d) = diff("state", &rec["state"], &state_json(&p, &world, using_item)) {
            return Err(mm(t, d));
        }
        let expected: Vec<Value> = rec["packets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| PACKETS.contains(&p["name"].as_str().unwrap_or("")))
            .cloned()
            .collect();
        if let Some(d) = diff("packets", &Value::Array(expected), &Value::Array(sent)) {
            return Err(mm(t, d));
        }
    }
    Ok(case.ticks.len() - 1)
}

/// Replays every case of a file.
pub fn replay_file(path: &std::path::Path) -> Summary {
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
