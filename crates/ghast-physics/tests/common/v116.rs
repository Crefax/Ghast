//! 1.16.5 golden replay (format: `oracle/README.md`, section "1.16.5").

use ghast_physics::v1_16_5::{
    self, Effects, Enchantments, MovementInput, PlayerState, Pose, SentState,
};
use ghast_physics::{Abilities, Packet, Packets, TickInput};
use serde_json::{Value, json};

use super::{
    Case, MOVEMENT_PACKETS, Mismatch, b, diff, f3, f32_of, f64_of, i, jf32, jf64, keys_of,
};

/// The case's world: fills of global state ids inside the loaded chunk rectangles.
pub fn world_of(header: &Value) -> v1_16_5::MemoryWorld {
    let mut w = v1_16_5::memory_world();
    for r in header["loaded_chunks"].as_array().unwrap() {
        let r: Vec<i32> = r.as_array().unwrap().iter().map(i).collect();
        w.load_chunks(r[0], r[1], r[2], r[3]);
    }
    for f in header["fills"].as_array().unwrap() {
        let f: Vec<i32> = f.as_array().unwrap().iter().map(i).collect();
        w.fill_raw([f[0], f[1], f[2]], [f[3], f[4], f[5]], f[6] as u16);
    }
    w
}

fn pose_of(s: &str) -> Pose {
    match s {
        "standing" => Pose::Standing,
        "crouching" => Pose::Crouching,
        "swimming" => Pose::Swimming,
        "fall_flying" => Pose::FallFlying,
        "sleeping" => Pose::Sleeping,
        other => panic!("pose {other}"),
    }
}

fn pose_name(p: Pose) -> &'static str {
    match p {
        Pose::Standing => "standing",
        Pose::Crouching => "crouching",
        Pose::Swimming => "swimming",
        Pose::FallFlying => "fall_flying",
        Pose::Sleeping => "sleeping",
    }
}

/// Effects from an `[[id, amplifier], …]` list (setup or an `effects` event).
fn effects_of(list: &Value) -> Effects {
    let mut effects = Effects::default();
    for e in list.as_array().unwrap() {
        match (i(&e[0]), i(&e[1])) {
            (8, amp) => effects.jump_boost = Some(amp),
            (15, _) => effects.blindness = true,
            (25, amp) => effects.levitation = Some(amp),
            (28, _) => effects.slow_falling = true,
            (30, _) => effects.dolphins_grace = true,
            (id, _) => panic!("effect {id} not modelled"),
        }
    }
    effects
}

fn uuid_of(s: &str) -> u128 {
    u128::from_str_radix(&s.replace('-', ""), 16).expect("uuid")
}

/// Player state from the tick-0 record plus header-only data.
pub fn player_of(header: &Value, t0: &Value) -> PlayerState {
    let s = &t0["state"];
    let setup = &header["setup"];
    let bb = &s["bb"];
    let mut p = PlayerState::new(f3(&s["pos"]), f32_of(&s["yaw"]), f32_of(&s["pitch"]));
    p.pose = pose_of(s["pose"].as_str().unwrap());
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
    p.fall_distance = f32_of(&s["fall_distance"]);
    p.movement_multiplier = f3(&s["movement_multiplier"]);
    p.touching_water = b(&s["touching_water"]);
    p.submerged_in_water = b(&s["submerged_in_water"]);
    p.water_height = f64_of(&s["water_height"]);
    p.lava_height = f64_of(&s["lava_height"]);
    p.first_update = b(&s["first_update"]);
    p.swimming = b(&s["swimming"]);
    p.sprinting = b(&s["sprinting"]);
    p.in_sneaking_pose = b(&s["in_sneaking_pose"]);
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
    p.movement_speed_base = f64_of(&setup["movement_speed_base"]);
    // The oracle gives setup modifier k the UUID (most 0x6f7261636c65, least k); the UUID decides
    // the order modifiers of one operation are applied in.
    for (k, m) in setup["speed_modifiers"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let uuid = (0x6f72_6163_6c65_u128 << 64) | k as u128;
        p.speed_modifiers
            .insert(uuid, f64_of(&m[0]), i(&m[1]) as u8);
    }
    // The client's sprint modifier comes after the server's (setup applies properties first).
    p.speed_modifiers.set_sprint(p.sprinting);
    p.spectator = setup["game_mode"] == "spectator";
    p.fly_toggle_timer = i(&s["fly_toggle_timer"]);
    p.sprint_toggle_timer = i(&s["sprint_toggle_timer"]);
    p.ticks_since_sprinting_changed = i(&s["ticks_since_sprinting_changed"]);
    p.food_level = i(&s["food_level"]);
    p.age = i(&s["age"]);
    p.auto_jump = setup["auto_jump"].as_bool().unwrap_or(false);
    p.auto_jump_enabled = b(&s["auto_jump_enabled"]);
    p.ticks_to_next_autojump = i(&s["ticks_to_next_autojump"]);
    let inp = &s["input"];
    p.input = MovementInput {
        forward: f32_of(&inp["forward"]),
        sideways: f32_of(&inp["sideways"]),
        jumping: b(&inp["jump"]),
        sneaking: b(&inp["sneak"]),
        pressing_forward: inp["pressing_forward"].as_bool().unwrap_or(false),
        pressing_back: inp["pressing_back"].as_bool().unwrap_or(false),
        pressing_left: inp["pressing_left"].as_bool().unwrap_or(false),
        pressing_right: inp["pressing_right"].as_bool().unwrap_or(false),
    };
    let sent = &s["sent"];
    p.sent = SentState {
        pos: f3(&sent["pos"]),
        yaw: f32_of(&sent["yaw"]),
        pitch: f32_of(&sent["pitch"]),
        on_ground: b(&sent["on_ground"]),
        sprinting: b(&sent["sprinting"]),
        sneaking: b(&sent["sneaking"]),
        ticks_since_pos: i(&sent["ticks_since_pos"]),
    };
    p.effects = effects_of(&setup["effects"]);
    p.enchantments = Enchantments {
        depth_strider: i(&setup["depth_strider"]) as u8,
        soul_speed: i(&setup["soul_speed"]) as u8,
    };
    p.eye_fluid = match s.get("submerged_fluid").and_then(Value::as_str) {
        Some("water") => Some(ghast_data::FluidKind::Water),
        Some("lava") => Some(ghast_data::FluidKind::Lava),
        _ => None,
    };
    p
}

/// The state in the oracle's 1.16.5 record layout.
pub fn state_json(p: &PlayerState, w: &v1_16_5::MemoryWorld, using_item: bool) -> Value {
    let bb = p.bb;
    let (wd, ht) = p.pose.dimensions();
    json!({
        "pos": [jf64(p.pos[0]), jf64(p.pos[1]), jf64(p.pos[2])],
        "bb": [jf64(bb.min_x), jf64(bb.min_y), jf64(bb.min_z), jf64(bb.max_x), jf64(bb.max_y), jf64(bb.max_z)],
        "vel": [jf64(p.vel[0]), jf64(p.vel[1]), jf64(p.vel[2])],
        "yaw": jf32(p.yaw),
        "pitch": jf32(p.pitch),
        "on_ground": p.on_ground,
        "horizontal_collision": p.horizontal_collision,
        "vertical_collision": p.vertical_collision,
        "fall_distance": jf32(p.fall_distance),
        "movement_multiplier": [jf64(p.movement_multiplier[0]), jf64(p.movement_multiplier[1]), jf64(p.movement_multiplier[2])],
        "pose": pose_name(p.pose),
        "width": jf32(wd),
        "height": jf32(ht),
        "eye_height": jf32(p.eye_height()),
        "touching_water": p.touching_water,
        "submerged_in_water": p.submerged_in_water,
        "submerged_fluid": match p.eye_fluid {
            Some(ghast_data::FluidKind::Water) => json!("water"),
            Some(ghast_data::FluidKind::Lava) => json!("lava"),
            None => Value::Null,
        },
        "velocity_modified": false,
        "water_height": jf64(p.water_height),
        "lava_height": jf64(p.lava_height),
        "in_lava": p.in_lava(),
        "first_update": p.first_update,
        "climbing": v1_16_5::is_climbing(p, w),
        "swimming": p.is_swimming(),
        "sprinting": p.sprinting,
        "sneaking": p.sneaking(),
        "in_sneaking_pose": p.in_sneaking_pose,
        "fall_flying": false,
        "using_item": using_item,
        "no_clip": p.no_clip,
        "step_height": jf32(v1_16_5::STEP_HEIGHT),
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
        "upward_speed": jf32(0.0),
        "flying_speed": jf32(p.flying_speed),
        "movement_speed": jf32(p.movement_speed),
        "movement_speed_attr": jf64(p.movement_speed_attr()),
        "last_block_pos": null,
        "fly_toggle_timer": p.fly_toggle_timer,
        "sprint_toggle_timer": p.sprint_toggle_timer,
        "ticks_since_sprinting_changed": p.ticks_since_sprinting_changed,
        "auto_jump_enabled": p.auto_jump_enabled,
        "ticks_to_next_autojump": p.ticks_to_next_autojump,
        "food_level": p.food_level,
        "age": p.age,
        "input": {
            "forward": jf32(p.input.forward),
            "sideways": jf32(p.input.sideways),
            "jump": p.input.jumping,
            "sneak": p.input.sneaking,
            "pressing_forward": p.input.pressing_forward,
            "pressing_back": p.input.pressing_back,
            "pressing_left": p.input.pressing_left,
            "pressing_right": p.input.pressing_right,
        },
        "sent": {
            "pos": [jf64(p.sent.pos[0]), jf64(p.sent.pos[1]), jf64(p.sent.pos[2])],
            "yaw": jf32(p.sent.yaw),
            "pitch": jf32(p.sent.pitch),
            "on_ground": p.sent.on_ground,
            "sprinting": p.sent.sprinting,
            "sneaking": p.sent.sneaking,
            "ticks_since_pos": p.sent.ticks_since_pos,
        },
    })
}

fn action(id: i32, name: &str) -> Value {
    json!({"name": "entity_action", "action": id, "action_name": name})
}

/// A packet in the oracle's 1.16.5 record layout.
pub fn packet_json(p: &Packet) -> Value {
    match *p {
        Packet::StartSneaking => action(0, "press_shift_key"),
        Packet::StopSneaking => action(1, "release_shift_key"),
        Packet::StartSprinting => action(3, "start_sprinting"),
        Packet::StopSprinting => action(4, "stop_sprinting"),
        Packet::Abilities(a) => json!({"name": "abilities", "flying": a.flying}),
        other => super::packet_json(&other),
    }
}

/// Replays one 1.16.5 case.
pub fn replay(case: &Case) -> Result<usize, Mismatch> {
    let name = case.header["name"].as_str().unwrap_or("?").to_string();
    let world = world_of(&case.header);
    let t0 = &case.ticks[0];
    let mut p = player_of(&case.header, t0);
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
        let (keys, use_key) = keys_of(input["keys"].as_str().unwrap());
        // Spectators cannot use items (`interactItem` passes in spectator mode), so the shield
        // is never raised; the physics only sees whether an item is in use.
        let using_item = use_key && !p.spectator;
        let ti = TickInput {
            keys,
            yaw: f32_of(&input["yaw"]),
            pitch: f32_of(&input["pitch"]),
            using_item,
        };
        v1_16_5::tick(&mut p, &ti, &world, &mut out);
        sent.extend(out.as_slice().iter().map(packet_json));
        if let Some(d) = diff("state", &rec["state"], &state_json(&p, &world, using_item)) {
            return Err(mm(t, d));
        }
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

/// Replays every case of a 1.16.5 file.
pub fn replay_file(path: &std::path::Path) -> super::Summary {
    let mut s = super::Summary::default();
    for case in super::cases(path) {
        s.cases += 1;
        match replay(&case) {
            Ok(n) => s.ticks += n,
            Err(m) => s.failures.push(m),
        }
    }
    s
}
