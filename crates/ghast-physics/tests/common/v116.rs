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
        // The physics never glides: its FallFlying pose is the riptide spin's box.
        "fall_flying" | "spin_attack" => Pose::FallFlying,
        "sleeping" => Pose::Sleeping,
        other => panic!("pose {other}"),
    }
}

fn pose_name(p: Pose, glide: bool) -> &'static str {
    match p {
        Pose::Standing => "standing",
        Pose::Crouching => "crouching",
        Pose::Swimming => "swimming",
        Pose::FallFlying if glide => "fall_flying",
        Pose::FallFlying => "spin_attack",
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
    state_json_ext(p, None, w, using_item)
}

/// [`state_json`] with the extension state (the glide flag and pose).
pub fn state_json_ext(
    p: &PlayerState,
    x: Option<&v1_16_5::PlayerExt>,
    w: &v1_16_5::MemoryWorld,
    using_item: bool,
) -> Value {
    let (fall_flying, glide_pose) = x.map_or((false, false), |x| (x.fall_flying, x.glide_pose));
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
        "pose": pose_name(p.pose, glide_pose),
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
        "fall_flying": fall_flying,
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
        Packet::StartFallFlying => action(8, "start_fall_flying"),
        Packet::SteerVehicle {
            sideways,
            forward,
            jump,
            sneak,
        } => {
            json!({"name": "steer_vehicle", "sideways": jf32(sideways), "forward": jf32(forward), "jump": jump, "sneak": sneak})
        }
        Packet::MoveVehicle {
            pos, yaw, pitch, ..
        } => json!({
            "name": "vehicle_move", "pos": [jf64(pos[0]), jf64(pos[1]), jf64(pos[2])],
            "yaw": jf32(yaw), "pitch": jf32(pitch),
        }),
        Packet::PaddleBoat { left, right } => {
            json!({"name": "steer_boat", "left": left, "right": right})
        }
        other => super::packet_json(&other),
    }
}

fn aabb_of(v: &Value) -> ghast_physics::math::Aabb {
    ghast_physics::math::Aabb::new(
        f64_of(&v[0]),
        f64_of(&v[1]),
        f64_of(&v[2]),
        f64_of(&v[3]),
        f64_of(&v[4]),
        f64_of(&v[5]),
    )
}

/// The client scoreboard's teams as far as collision rules go (an independent model of
/// `EntityPredicates.canBePushedBy` for the replay).
#[derive(Default)]
pub struct Teams {
    /// Team name -> collision rule.
    rules: std::collections::HashMap<String, String>,
    /// Scoreboard name -> team name.
    members: std::collections::HashMap<String, String>,
}

impl Teams {
    /// Applies a `team` event / setup entry.
    pub fn apply(&mut self, t: &Value) {
        let name = t["name"].as_str().unwrap().to_string();
        let mode = i(&t["mode"]);
        let members = t["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_str().unwrap().to_string());
        match mode {
            0 => {
                self.rules
                    .entry(name.clone())
                    .or_insert_with(|| "always".into());
                if let Some(r) = t.get("rule").and_then(Value::as_str) {
                    self.rules.insert(name.clone(), r.into());
                }
                for m in members {
                    self.members.insert(m, name.clone());
                }
            }
            1 => {
                self.rules.remove(&name);
                self.members.retain(|_, v| *v != name);
            }
            2 => {
                if let Some(r) = t.get("rule").and_then(Value::as_str) {
                    self.rules.insert(name, r.into());
                }
            }
            3 => {
                for m in members {
                    self.members.insert(m, name.clone());
                }
            }
            4 => {
                for m in members {
                    if self.members.get(&m) == Some(&name) {
                        self.members.remove(&m);
                    }
                }
            }
            _ => panic!("team mode {mode}"),
        }
    }

    fn of(&self, who: &str) -> (Option<&str>, &str) {
        match self.members.get(who) {
            Some(t) => (
                Some(t.as_str()),
                self.rules.get(t).map_or("always", |r| r.as_str()),
            ),
            None => (None, "always"),
        }
    }

    /// `canBePushedBy(pusher)` for the local player `me`.
    pub fn may_push(&self, pusher: &str, me: &str) -> bool {
        let (own, own_rule) = self.of(pusher);
        if own_rule == "never" {
            return false;
        }
        let (theirs, their_rule) = self.of(me);
        if their_rule == "never" {
            return false;
        }
        let same = own.is_some() && own == theirs;
        if (own_rule == "pushOwnTeam" || their_rule == "pushOwnTeam") && same {
            return false;
        }
        (own_rule != "pushOtherTeams" && their_rule != "pushOtherTeams") || same
    }
}

/// The local player's scoreboard name in the oracle.
const ORACLE_NAME: &str = "oracle";

/// Spawned entities of a case: id -> (type, scoreboard name).
fn spawned(e: &Value, out: &mut std::collections::HashMap<i64, (String, String)>) {
    let ty = e["entity"].as_str().unwrap().to_string();
    let name = match e.get("name").and_then(Value::as_str) {
        Some(n) => n.to_string(),
        None => e["uuid"].as_str().unwrap().to_string(),
    };
    out.insert(e["id"].as_i64().unwrap(), (ty, name));
}

/// `instanceof LivingEntity` for the types of the oracle's entity cases.
fn living(name: &str) -> bool {
    !matches!(name, "boat" | "minecart" | "item" | "firework_rocket")
}

/// The use-key model of the harness (`MinecraftClient.handleInputEvents` with the crosshair on
/// nothing): a press uses the item, a held key retries once the 4-tick cooldown has run out.
#[derive(Default)]
struct UseKey {
    down: bool,
    cooldown: i32,
    using: bool,
    /// Player ticks the current use has lasted.
    held: u32,
}

impl UseKey {
    /// Returns how long the use lasted when the key released it this tick.
    fn tick(&mut self, down: bool, can_use: bool) -> Option<u32> {
        let press = down && !self.down;
        self.down = down;
        if self.cooldown > 0 {
            self.cooldown -= 1;
        }
        let mut released = None;
        if self.using {
            if !down {
                self.using = false;
                released = Some(self.held);
            }
        } else if press {
            self.start(can_use);
        }
        if down && self.cooldown == 0 && !self.using {
            self.start(can_use);
        }
        released
    }

    fn start(&mut self, can_use: bool) {
        self.cooldown = 4;
        if can_use {
            self.using = true;
            self.held = 0;
        }
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
    let mut out = Packets::new();
    // Entity cases: the teams and the spawned entities.
    let mut teams = Teams::default();
    let mut kinds: std::collections::HashMap<i64, (String, String)> =
        std::collections::HashMap::new();
    // Movement state beyond the player (`setup.ext`: dimension, riptide): the extended tick.
    let mut ext: Option<v1_16_5::PlayerExt> = None;
    // Riptide: the trident's level, the level's rain (the oracle's biome is plains; the nether
    // has no sky light), the harness's use key.
    let (mut riptide, mut raining, mut trident) = (0u8, false, false);
    // Elytra cases: the chest item (an elytra at max damage - 1 cannot glide).
    let mut glide_case = false;
    // A firework rocket in the hand: `FireworkItem.use` never starts an item use.
    let mut rocket_hand = false;
    if let Some(x) = case.header["setup"].get("ext") {
        for t in x["teams"].as_array().unwrap() {
            teams.apply(t);
        }
        for e in x["entities"].as_array().unwrap() {
            spawned(e, &mut kinds);
        }
        let mut e = v1_16_5::PlayerExt::new();
        let mut on = false;
        if let Some(d) = x.get("dimension").and_then(Value::as_str) {
            e.ultrawarm = match d {
                "the_nether" => true,
                other => panic!("dimension {other}"),
            };
            on = true;
        }
        if x.get("glide").and_then(Value::as_bool) == Some(true) {
            glide_case = true;
            e.elytra = x.get("chest").and_then(Value::as_str) == Some("elytra")
                && x.get("chest_damage").map_or(0, i) < 431;
            e.fall_flying = x.get("fall_flying").and_then(Value::as_bool) == Some(true);
            on = true;
        }
        rocket_hand = x.get("hand").and_then(Value::as_str) == Some("firework_rocket");
        if let Some(h) = x.get("hand").and_then(Value::as_str)
            && h != "firework_rocket"
        {
            assert_eq!(h, "trident", "hand item {h}");
            trident = true;
            riptide = x.get("riptide").map_or(0, |v| i(v) as u8);
            raining = x.get("raining").and_then(Value::as_bool).unwrap_or(false)
                && x.get("dimension").is_none();
            on = true;
        }
        if on {
            ext = Some(e);
        }
    }
    if let Some(d) = diff(
        "state",
        &t0["state"],
        &state_json_ext(&p, ext.as_ref(), &world, false),
    ) {
        return Err(mm(0, d));
    }
    // Riding cases (`setup.ext.ride`): the boat (entity 100) and whether the player is on it.
    let mut boat: Option<v1_16_5::Boat> = None;
    let mut riding = false;
    if let Some(x) = case.header["setup"].get("ext")
        && x.get("ride").and_then(Value::as_bool) == Some(true)
    {
        let sp = x["entities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == 100)
            .expect("the boat");
        // The spawn packet's angle byte.
        let byte = (f32_of(&sp["yaw"]) * 256.0 / 360.0).floor() as i32 as i8;
        boat = Some(v1_16_5::Boat::new(
            f3(&sp["pos"]),
            f32::from(byte) * 360.0 / 256.0,
            0.0,
        ));
        if ext.is_none() {
            ext = Some(v1_16_5::PlayerExt::new());
        }
    }
    let mut use_key = UseKey::default();
    // Where the client world files the player (its position after its last tick).
    let mut filed = p.pos;
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
                    // Other entities: their boxes come with the tick record.
                    "spawn" => spawned(e, &mut kinds),
                    "team" => teams.apply(e),
                    "entity_move" | "entity_rel" | "entity_look" | "remove_entity"
                    | "game_mode" => {}
                    "shared_flags" => {
                        let x = ext.as_mut().expect("shared flags need the extended tick");
                        let cur = x.shared_flags(&p);
                        let next = (cur & !(i(&e["clear"]) as u8)) | i(&e["set"]) as u8;
                        x.apply_shared_flags(&mut p, next);
                    }
                    "equip" => {
                        let x = ext.as_mut().expect("equipment needs the extended tick");
                        assert_eq!(e["slot"], "chest");
                        x.elytra = e.get("item").and_then(Value::as_str) == Some("elytra")
                            && i(&e["damage"]) < 431;
                    }
                    // The rocket entity comes with the tick records (tick order, ticking).
                    "firework" => {}
                    "ride" => {
                        let on = e["passengers"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|v| v.as_i64() == Some(1));
                        if on && !riding {
                            // Entity.startRiding: the standing pose.
                            p.pose = Pose::Standing;
                            let [px, py, pz] = p.pos;
                            p.set_position(px, py, pz);
                        }
                        riding = on && boat.is_some();
                    }
                    "vehicle_move" => {
                        if riding {
                            let b = boat.as_mut().unwrap();
                            let reply = b.server_move(
                                f3(&e["pos"]),
                                f32_of(&e["yaw"]),
                                f32_of(&e["pitch"]),
                            );
                            sent.push(packet_json(&reply));
                        }
                    }
                    "living_flags" => {
                        let x = ext.as_mut().expect("living flags need the extended tick");
                        let cur = if x.spin_attack { 4u8 } else { 0 };
                        let next = (cur & !(i(&e["clear"]) as u8)) | i(&e["set"]) as u8;
                        x.apply_living_flags(next);
                    }
                    other => panic!("unknown event {other}"),
                }
            }
        }
        let (keys, use_down) = keys_of(input["keys"].as_str().unwrap());
        // Spectators cannot use items (`interactItem` passes in spectator mode), so the shield
        // is never raised; the physics only sees whether an item is in use.
        let mut using_item = use_down && !p.spectator && !rocket_hand;
        // The trident: `TridentItem.use` refuses a riptide trident out of water and rain; the
        // release launches (before the tick, along the tick's rotation).
        let mut launch = false;
        if trident {
            let wet = v1_16_5::in_water_or_rain(&p, &world, raining);
            let can_use = !p.spectator && (riptide == 0 || wet);
            if let Some(held) = use_key.tick(use_down, can_use) {
                launch = riptide > 0 && held >= 10 && wet;
            }
            using_item = use_key.using;
        }
        let ti = TickInput {
            keys,
            yaw: f32_of(&input["yaw"]),
            pitch: f32_of(&input["pitch"]),
            using_item,
        };
        // The launch happens in the key handling, before any entity ticks: its move up collides
        // with the entities' boxes before their tick.
        if launch {
            let mut pre: Vec<(i64, ghast_physics::math::Aabb)> = Vec::new();
            for e in rec
                .get("entities")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if e.get("player").is_none()
                    && v1_16_5::collides(e["type"].as_str().unwrap())
                    && b(&e["alive"])
                    && b(&e["in_section_pre"])
                    && let Some(bb) = e.get("pre")
                {
                    let order = e.get("query_order").and_then(Value::as_i64);
                    pre.push((order.unwrap_or(i64::MAX), aabb_of(bb)));
                }
            }
            pre.sort_by_key(|c| c.0);
            let boxes: Vec<ghast_physics::math::Aabb> = pre.iter().map(|c| c.1).collect();
            let at = v1_16_5::Surroundings::new().with_entities(&boxes);
            let x = ext.as_mut().unwrap();
            v1_16_5::riptide(&mut p, x, &at, &world, riptide, ti.yaw, ti.pitch);
        }
        // Other entities in the client's tick order: those before the player push it first (with
        // their box after their tick); the player collides with the collidable ones as they are
        // when it ticks; those after it push it after its tick.
        let mut colliders: Vec<(i64, ghast_physics::math::Aabb)> = Vec::new();
        let mut after: Vec<v1_16_5::Pusher> = Vec::new();
        // The ones after the player in tick order: a pusher (index into `after`) or a rocket.
        let mut after_order: Vec<Option<usize>> = Vec::new();
        if let Some(ents) = rec.get("entities").and_then(Value::as_array) {
            let mut before_player = true;
            for e in ents {
                if e.get("player").is_some() {
                    before_player = false;
                    continue;
                }
                let id = e["id"].as_i64().unwrap();
                let name = e["type"].as_str().unwrap();
                let at_tick = if before_player {
                    b(&e["in_section_post"]).then(|| aabb_of(&e["post"]))
                } else {
                    (b(&e["in_section_pre"]) && e.get("pre").is_some()).then(|| aabb_of(&e["pre"]))
                };
                if v1_16_5::collides(name)
                    && b(&e["alive"])
                    && !(riding && id == 100)
                    && let Some(bb) = at_tick
                {
                    colliders.push((
                        e.get("query_order")
                            .and_then(Value::as_i64)
                            .unwrap_or(i64::MAX),
                        bb,
                    ));
                }
                if name == "firework_rocket" && b(&e["ticks"]) {
                    if before_player {
                        let x = ext.as_ref().expect("rockets need the extended tick");
                        v1_16_5::rocket_boost(
                            &mut p,
                            x,
                            f32_of(&input["yaw"]),
                            f32_of(&input["pitch"]),
                        );
                    } else {
                        after_order.push(None);
                    }
                    continue;
                }
                let spectator = e.get("spectator").and_then(Value::as_bool).unwrap_or(false);
                let score_name = kinds.get(&id).map_or("", |k| k.1.as_str());
                if let Some(kind) = v1_16_5::push_kind(name, living(name))
                    && b(&e["ticks"])
                    && !(riding && id == 100)
                    && !spectator
                    && teams.may_push(score_name, ORACLE_NAME)
                {
                    let pusher = v1_16_5::Pusher::new(f3(&e["pos"]), aabb_of(&e["post"]), kind);
                    if before_player {
                        v1_16_5::push_by(&mut p, filed, &world, &pusher);
                    } else {
                        after_order.push(Some(after.len()));
                        after.push(pusher);
                    }
                }
            }
        }
        colliders.sort_by_key(|c| c.0);
        let colliders: Vec<ghast_physics::math::Aabb> =
            colliders.into_iter().map(|c| c.1).collect();
        // What the riptide spin can touch: every other entity in the client world's entity
        // sections (no spectators), with its box when the player ticks.
        let mut others: Vec<v1_16_5::OtherEntity> = Vec::new();
        if let Some(ents) = rec.get("entities").and_then(Value::as_array) {
            let mut before_player = true;
            for e in ents {
                if e.get("player").is_some() {
                    before_player = false;
                    continue;
                }
                let name = e["type"].as_str().unwrap();
                if e.get("spectator").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                let at_tick = if before_player {
                    b(&e["in_section_post"]).then(|| aabb_of(&e["post"]))
                } else {
                    (b(&e["in_section_pre"]) && e.get("pre").is_some()).then(|| aabb_of(&e["pre"]))
                };
                if let Some(bb) = at_tick {
                    others.push(v1_16_5::OtherEntity::new(bb, living(name)));
                }
            }
        }
        let around = v1_16_5::Surroundings::new()
            .with_entities(&colliders)
            .with_others(&others);
        match &mut ext {
            Some(x) if riding => {
                // The boat collides with the other boats as they are when it ticks.
                let mut boat_boxes: Vec<ghast_physics::math::Aabb> = Vec::new();
                if let Some(ents) = rec.get("entities").and_then(Value::as_array) {
                    let mut before_boat = true;
                    for e in ents {
                        if e["id"] == 100 {
                            before_boat = false;
                            continue;
                        }
                        if e.get("player").is_some() || e["type"] != "boat" {
                            continue;
                        }
                        let bb = if before_boat {
                            e.get("post")
                        } else {
                            e.get("pre")
                        };
                        if let Some(bb) = bb {
                            boat_boxes.push(aabb_of(bb));
                        }
                    }
                }
                let bt = boat.as_mut().unwrap();
                if !v1_16_5::tick_boat(&mut p, x, bt, &around, &boat_boxes, &ti, &world, &mut out) {
                    riding = false;
                    // Got off: the player ticks in this tick when it comes after the boat.
                    let ids: Vec<i64> = rec["entities"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|e| {
                            if e.get("player").is_some() {
                                1
                            } else {
                                e["id"].as_i64().unwrap()
                            }
                        })
                        .collect();
                    let at = |id: i64| ids.iter().position(|&v| v == id);
                    if at(1) > at(100) {
                        // Off the boat: it is solid again (as it is after its tick).
                        let mut with_boat = colliders.clone();
                        with_boat.push(bt.bb);
                        let around = v1_16_5::Surroundings::new()
                            .with_entities(&with_boat)
                            .with_others(&others);
                        // The turn of this tick was clamped while it still rode.
                        let mut ti2 = ti;
                        ti2.yaw = p.yaw;
                        ti2.pitch = p.pitch;
                        v1_16_5::tick_ext(&mut p, x, &around, &ti2, &world, &mut out);
                    }
                }
            }
            Some(x) => v1_16_5::tick_ext(&mut p, x, &around, &ti, &world, &mut out),
            None => v1_16_5::tick_in(&mut p, &around, &ti, &world, &mut out),
        }
        if trident && use_key.using {
            use_key.held += 1;
        }
        filed = p.pos;
        for k in &after_order {
            match k {
                Some(k) => {
                    v1_16_5::push_by(&mut p, filed, &world, &after[*k]);
                }
                None => {
                    let x = ext.as_ref().expect("rockets need the extended tick");
                    let (yaw, pitch) = (p.yaw, p.pitch);
                    v1_16_5::rocket_boost(&mut p, x, yaw, pitch);
                }
            }
        }
        sent.extend(out.as_slice().iter().map(packet_json));
        if let Some(d) = diff(
            "state",
            &rec["state"],
            &state_json_ext(&p, ext.as_ref(), &world, using_item),
        ) {
            return Err(mm(t, d));
        }
        if let (true, Some(want), Some(bt)) = (riding, rec.get("vehicle"), &boat) {
            let got = json!({
                "pos": [jf64(bt.pos[0]), jf64(bt.pos[1]), jf64(bt.pos[2])],
                "vel": [jf64(bt.vel[0]), jf64(bt.vel[1]), jf64(bt.vel[2])],
                "yaw": jf32(bt.yaw),
                "input": bt.input(),
                "paddle": bt.paddles(),
            });
            let want = json!({
                "pos": want["pos"], "vel": want["vel"], "yaw": want["yaw"],
                "input": want["input"], "paddle": want["paddle"],
            });
            if let Some(d) = diff("vehicle", &want, &got) {
                return Err(mm(t, d));
            }
        }
        if let (true, Some(want), Some(x)) = (glide_case, rec.get("ext"), &ext) {
            let got = json!({
                "fall_flying": x.fall_flying,
                "shared_flags": i(&want["shared_flags"]) as u8 & !0x98 | x.shared_flags(&p),
            });
            if let Some(d) = diff("ext", want, &got) {
                return Err(mm(t, d));
            }
        } else if let (Some(want), Some(x)) = (rec.get("ext"), &ext) {
            let got = json!({
                "riptide_ticks": x.riptide_ticks,
                "spin_attack": x.spin_attack,
                "living_flags": want["living_flags"],
                "item_use_time": if use_key.using { json!(use_key.held) } else { json!(0) },
            });
            if let Some(d) = diff("ext", want, &got) {
                return Err(mm(t, d));
            }
        }
        let expected: Vec<Value> = rec["packets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| {
                let n = p["name"].as_str().unwrap_or("");
                MOVEMENT_PACKETS.contains(&n)
                    || matches!(n, "steer_vehicle" | "vehicle_move" | "steer_boat")
            })
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
