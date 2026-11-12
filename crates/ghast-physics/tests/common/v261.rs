//! 26.1 golden replay (format: `oracle/README.md`, section "26.1").

use ghast_physics::modern::{
    self, AttributeKind, ClientInput, Dimensions, Effects, MemoryWorld26, ModifierId, PlayerExt,
    PlayerState, Pose, SentState, V26_1,
};
use ghast_physics::{Abilities, Keys, Packet, Packets, TickInput};
use serde_json::{Value, json};

use super::{Case, Mismatch, b, diff, f3, f32_of, f64_of, i, jf32, jf64, keys_of};

/// The case's world: fills of 26.1 global state ids inside the loaded chunk rectangles.
pub fn world_of(header: &Value) -> MemoryWorld26 {
    let mut w = modern::memory_world_26_1();
    for r in header["loaded_chunks"].as_array().unwrap() {
        let r: Vec<i32> = r.as_array().unwrap().iter().map(i).collect();
        w.load_chunks(r[0], r[1], r[2], r[3]);
    }
    for f in header["fills"].as_array().unwrap() {
        let f: Vec<i32> = f.as_array().unwrap().iter().map(i).collect();
        w.fill_raw([f[0], f[1], f[2]], [f[3], f[4], f[5]], f[6] as u16);
    }
    if let Some(bd) = header.get("border") {
        w.set_border(ghast_physics::WorldBorder {
            center_x: f64_of(&bd["center"][0]),
            center_z: f64_of(&bd["center"][1]),
            size: f64_of(&bd["size"]),
        });
    }
    w
}

fn pose_of(s: &str) -> Pose {
    match s {
        "standing" => Pose::Standing,
        "crouching" => Pose::Crouching,
        "swimming" => Pose::Swimming,
        "fall_flying" => Pose::FallFlying,
        "spin_attack" => Pose::SpinAttack,
        "sleeping" => Pose::Sleeping,
        "dying" => Pose::Dying,
        other => panic!("pose {other}"),
    }
}

fn pose_name(p: Pose) -> &'static str {
    match p {
        Pose::Standing => "standing",
        Pose::Crouching => "crouching",
        Pose::Swimming => "swimming",
        Pose::FallFlying => "fall_flying",
        Pose::SpinAttack => "spin_attack",
        Pose::Sleeping => "sleeping",
        Pose::Dying => "dying",
    }
}

/// Effects from `[[26.1 registry id, amplifier, ...], …]`.
fn effects_of(list: &Value) -> Effects {
    let mut e = Effects::default();
    for x in list.as_array().unwrap() {
        match (i(&x[0]), i(&x[1])) {
            (7, a) => e.jump_boost = Some(a),
            (14, _) => e.blindness = true,
            (24, a) => e.levitation = Some(a),
            (27, _) => e.slow_falling = true,
            (29, _) => e.dolphins_grace = true,
            (36, _) => e.weaving = true,
            // Effects that do not change the client's movement.
            (
                0 | 1 | 2 | 3 | 4 | 5 | 6 | 8..=13 | 15..=23 | 25 | 26 | 28 | 30..=35 | 37..=39,
                _,
            ) => {}
            (id, _) => panic!("effect {id} not modelled"),
        }
    }
    e
}

fn keys_json(k: &Keys) -> Value {
    json!({"forward": k.forward, "backward": k.back, "left": k.left, "right": k.right,
        "jump": k.jump, "shift": k.sneak, "sprint": k.sprint})
}

fn keys_from(v: &Value) -> Keys {
    Keys {
        forward: b(&v["forward"]),
        back: b(&v["backward"]),
        left: b(&v["left"]),
        right: b(&v["right"]),
        jump: b(&v["jump"]),
        sneak: b(&v["shift"]),
        sprint: b(&v["sprint"]),
    }
}

/// The modifier id a scenario UUID becomes (the sprint UUID keeps vanilla's id).
fn modifier_id(uuid: &str) -> ModifierId {
    if uuid == "662a6b8d-da3e-4c1c-8813-96ea6097278d" {
        modern::sprinting_id()
    } else {
        ModifierId::new(&format!("oracle:{uuid}"))
    }
}

/// Player state from the tick-0 record plus header-only data.
pub fn player_of(header: &Value, t0: &Value) -> PlayerState {
    let s = &t0["state"];
    let setup = &header["setup"];
    let bb = &s["bb"];
    let mut p = PlayerState::new(f3(&s["pos"]), f32_of(&s["yaw"]), f32_of(&s["pitch"]));
    p.pose = pose_of(s["pose"].as_str().unwrap());
    p.dims = Dimensions {
        width: f32_of(&s["width"]),
        height: f32_of(&s["height"]),
        eye_height: f32_of(&s["eye_height"]),
    };
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
    p.minor_horizontal_collision = b(&s["minor_horizontal_collision"]);
    p.vertical_collision = b(&s["vertical_collision"]);
    p.vertical_collision_below = b(&s["vertical_collision_below"]);
    p.fall_distance = f64_of(&s["fall_distance"]);
    p.stuck_multiplier = f3(&s["movement_multiplier"]);
    p.supporting_block = s["supporting_block"]
        .as_array()
        .map(|a| [i(&a[0]), i(&a[1]), i(&a[2])]);
    p.on_ground_no_blocks = b(&s["on_ground_no_blocks"]);
    p.touching_water = b(&s["touching_water"]);
    p.was_underwater = b(&s["submerged_in_water"]);
    p.eye_in_water = s["submerged_fluid"] == "water";
    p.eye_in_lava = s["submerged_fluid"] == "lava";
    p.water_height = f64_of(&s["water_height"]);
    p.lava_height = f64_of(&s["lava_height"]);
    p.in_powder_snow = b(&s["in_powder_snow"]);
    p.was_in_powder_snow = b(&s["was_in_powder_snow"]);
    p.first_tick = b(&s["first_update"]);
    p.swimming = b(&s["swimming"]);
    p.sprinting = b(&s["sprinting"]);
    p.crouching = b(&s["in_sneaking_pose"]);
    p.abilities = Abilities {
        invulnerable: b(&s["invulnerable"]),
        flying: b(&s["flying"]),
        allow_flying: b(&s["allow_flying"]),
        creative: b(&s["creative"]),
        fly_speed: f32_of(&s["fly_speed"]),
        walk_speed: f32_of(&s["walk_speed"]),
    };
    p.spectator = setup["game_mode"] == "spectator";
    p.no_physics = b(&s["no_clip"]);
    p.no_jump_delay = i(&s["jump_cooldown"]);
    p.jumping = b(&s["jumping"]);
    p.zza = f32_of(&s["forward_speed"]);
    p.xxa = f32_of(&s["sideways_speed"]);
    p.jump_trigger_time = i(&s["fly_toggle_timer"]);
    p.sprint_trigger_time = i(&s["sprint_toggle_timer"]);
    p.auto_jump_enabled = b(&s["auto_jump_enabled"]);
    p.auto_jump_time = i(&s["ticks_to_next_autojump"]);
    p.food_level = i(&s["food_level"]);
    p.age = i(&s["age"]);
    let inp = &s["input"];
    p.input = ClientInput {
        keys: keys_from(&inp["keys"]),
        move_vector: [
            f32_of(&inp["move_vector"][0]),
            f32_of(&inp["move_vector"][1]),
        ],
    };
    let sent = &s["sent"];
    p.sent = SentState {
        pos: f3(&sent["pos"]),
        yaw: f32_of(&sent["yaw"]),
        pitch: f32_of(&sent["pitch"]),
        on_ground: b(&sent["on_ground"]),
        horizontal_collision: b(&sent["horizontal_collision"]),
        sprinting: b(&sent["sprinting"]),
        position_reminder: i(&sent["ticks_since_pos"]),
        input: keys_from(&sent["input"]),
    };
    // Attributes. Movement speed is rebuilt as the setup packet sent it (base + oracle modifiers +
    // the scenario's movement_speed modifiers, then the client's sprint modifier); the others only
    // ever change at setup, so their recorded value is used as the base.
    let attrs = &s["attributes"];
    for (name, kind) in [
        ("step_height", AttributeKind::StepHeight),
        ("gravity", AttributeKind::Gravity),
        ("jump_strength", AttributeKind::JumpStrength),
        ("sneaking_speed", AttributeKind::SneakingSpeed),
        ("movement_efficiency", AttributeKind::MovementEfficiency),
        (
            "water_movement_efficiency",
            AttributeKind::WaterMovementEfficiency,
        ),
        ("scale", AttributeKind::Scale),
        ("safe_fall_distance", AttributeKind::SafeFallDistance),
    ] {
        p.apply_attribute(kind, f64_of(&attrs[name]), &[]);
    }
    // The setup attributes packet marked scale dirty when it carried scale: the first tick's
    // refreshDirtyAttributes then rebuilds the box.
    p.scale_dirty = setup["attribute_bases"].get("minecraft:scale").is_some()
        || setup["attribute_modifiers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m[0] == "minecraft:scale");
    let mut mods: Vec<(ModifierId, f64, u8)> = Vec::new();
    for (k, m) in setup["speed_modifiers"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        mods.push((
            ModifierId::new(&format!("oracle:speed_{k}")),
            f64_of(&m[0]),
            i(&m[1]) as u8,
        ));
    }
    p.apply_attribute(
        AttributeKind::MovementSpeed,
        f64_of(&setup["movement_speed_base"]),
        &mods,
    );
    let speed_extra: Vec<&Value> = setup["attribute_modifiers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m[0] == "minecraft:movement_speed")
        .collect();
    let speed_base = setup["attribute_bases"].get("minecraft:movement_speed");
    if !speed_extra.is_empty() || speed_base.is_some() {
        let base = speed_base.map_or(f64::from(0.1f32), f64_of);
        let extra: Vec<(ModifierId, f64, u8)> = speed_extra
            .iter()
            .map(|m| {
                (
                    ModifierId::new(m[1].as_str().unwrap()),
                    f64_of(&m[2]),
                    i(&m[3]) as u8,
                )
            })
            .collect();
        p.apply_attribute(AttributeKind::MovementSpeed, base, &extra);
    }
    if p.sprinting {
        p.set_sprinting(true);
    }
    p.effects = effects_of(&setup["effects"]);
    p.options.auto_jump = setup["auto_jump"].as_bool().unwrap_or(false);
    if let Some(w) = setup.get("sprint_window").and_then(Value::as_i64) {
        p.options.sprint_window = w as i32;
    }
    p.leather_boots = setup
        .get("leather_boots")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(u) = setup.get("use_effects") {
        p.use_effects = modern::UseEffects {
            can_sprint: b(&u["can_sprint"]),
            speed_multiplier: f32_of(&u["speed_multiplier"]),
        };
    }
    p
}

/// The extension state from the header's `setup.ext` and the tick-0 record.
pub fn ext_of(header: &Value, t0: &Value) -> PlayerExt {
    let mut x = PlayerExt::default();
    if let Some(e) = header["setup"].get("ext") {
        x.glider = glider(e.get("chest"), e.get("chest_damage"));
        x.rocket_arm_left = e.get("left_handed").is_some_and(b);
    }
    x.fall_flying = b(&t0["state"]["fall_flying"]);
    x
}

/// `canGlideUsing` of a chest item: an elytra whose next damage does not break it (max 432).
fn glider(item: Option<&Value>, damage: Option<&Value>) -> bool {
    item.and_then(Value::as_str) == Some("elytra") && damage.map_or(0, i) < 431
}

/// The state in the oracle's 26.1 record layout.
pub fn state_json(p: &PlayerState, w: &MemoryWorld26, using_item: bool) -> Value {
    state_json_ext(p, &PlayerExt::default(), w, using_item)
}

/// [`state_json`] with the extension state.
pub fn state_json_ext(
    p: &PlayerState,
    x: &PlayerExt,
    w: &MemoryWorld26,
    using_item: bool,
) -> Value {
    let bb = p.bb;
    let a = &p.attributes;
    let fly = if p.abilities.flying {
        if p.sprinting {
            p.abilities.fly_speed * 2.0
        } else {
            p.abilities.fly_speed
        }
    } else if p.sprinting {
        0.025_999_999
    } else {
        0.02
    };
    json!({
        "pos": [jf64(p.pos[0]), jf64(p.pos[1]), jf64(p.pos[2])],
        "bb": [jf64(bb.min_x), jf64(bb.min_y), jf64(bb.min_z), jf64(bb.max_x), jf64(bb.max_y), jf64(bb.max_z)],
        "vel": [jf64(p.vel[0]), jf64(p.vel[1]), jf64(p.vel[2])],
        "yaw": jf32(p.yaw),
        "pitch": jf32(p.pitch),
        "on_ground": p.on_ground,
        "horizontal_collision": p.horizontal_collision,
        "minor_horizontal_collision": p.minor_horizontal_collision,
        "vertical_collision": p.vertical_collision,
        "vertical_collision_below": p.vertical_collision_below,
        "velocity_modified": false,
        "fall_distance": jf64(p.fall_distance),
        "movement_multiplier": [jf64(p.stuck_multiplier[0]), jf64(p.stuck_multiplier[1]), jf64(p.stuck_multiplier[2])],
        "supporting_block": match p.supporting_block { Some(s) => json!(s), None => Value::Null },
        "on_ground_no_blocks": p.on_ground_no_blocks,
        "pose": pose_name(p.pose),
        "width": jf32(p.dims.width),
        "height": jf32(p.dims.height),
        "eye_height": jf32(p.dims.eye_height),
        "touching_water": p.touching_water,
        "submerged_in_water": p.is_underwater(),
        "submerged_fluid": if p.eye_in_water { json!("water") } else if p.eye_in_lava { json!("lava") } else { Value::Null },
        "water_height": jf64(p.water_height),
        "lava_height": jf64(p.lava_height),
        "in_lava": p.in_lava(),
        "in_powder_snow": p.in_powder_snow,
        "was_in_powder_snow": p.was_in_powder_snow,
        "first_update": p.first_tick,
        "climbing": modern::on_climbable_ext::<V26_1, _>(p, x, w),
        "swimming": p.is_swimming(),
        "sprinting": p.sprinting,
        "sneaking": p.shift(),
        "in_sneaking_pose": p.crouching,
        "visually_crawling": x.visually_crawling(p),
        "fall_flying": x.fall_flying,
        "using_item": using_item,
        "no_clip": p.no_physics,
        "step_height": jf32(p.step_height()),
        "flying": p.abilities.flying,
        "allow_flying": p.abilities.allow_flying,
        "creative": p.abilities.creative,
        "invulnerable": p.abilities.invulnerable,
        "fly_speed": jf32(p.abilities.fly_speed),
        "walk_speed": jf32(p.abilities.walk_speed),
        "jump_cooldown": p.no_jump_delay,
        "jumping": p.jumping,
        "forward_speed": jf32(p.zza),
        "sideways_speed": jf32(p.xxa),
        "upward_speed": jf32(0.0),
        "flying_speed": jf32(fly),
        "movement_speed": jf32(p.speed()),
        "attributes": {
            "movement_speed": jf64(a.movement_speed.value()),
            "step_height": jf64(a.step_height.value()),
            "gravity": jf64(a.gravity.value()),
            "jump_strength": jf64(a.jump_strength.value()),
            "sneaking_speed": jf64(a.sneaking_speed.value()),
            "movement_efficiency": jf64(a.movement_efficiency.value()),
            "water_movement_efficiency": jf64(a.water_movement_efficiency.value()),
            "scale": jf64(a.scale.value()),
            "safe_fall_distance": jf64(a.safe_fall_distance.value()),
        },
        "movement_speed_attr": jf64(p.movement_speed()),
        "last_block_pos": null,
        "fly_toggle_timer": p.jump_trigger_time,
        "sprint_toggle_timer": p.sprint_trigger_time,
        "auto_jump_enabled": p.auto_jump_enabled,
        "ticks_to_next_autojump": p.auto_jump_time,
        "food_level": p.food_level,
        "age": p.age,
        "input": {
            "move_vector": [jf32(p.input.move_vector[0]), jf32(p.input.move_vector[1])],
            "keys": keys_json(&p.input.keys),
        },
        "sent": {
            "pos": [jf64(p.sent.pos[0]), jf64(p.sent.pos[1]), jf64(p.sent.pos[2])],
            "yaw": jf32(p.sent.yaw),
            "pitch": jf32(p.sent.pitch),
            "on_ground": p.sent.on_ground,
            "horizontal_collision": p.sent.horizontal_collision,
            "sprinting": p.sent.sprinting,
            "ticks_since_pos": p.sent.position_reminder,
            "input": keys_json(&p.sent.input),
        },
    })
}

/// A packet in the oracle's 26.1 record layout.
pub fn packet_json(p: &Packet) -> Value {
    match *p {
        Packet::StartSprinting => {
            json!({"name": "entity_action", "action": 1, "action_name": "start_sprinting"})
        }
        Packet::StopSprinting => {
            json!({"name": "entity_action", "action": 2, "action_name": "stop_sprinting"})
        }
        Packet::Abilities(a) => json!({"name": "abilities", "flying": a.flying}),
        Packet::PlayerInput(k) => json!({"name": "player_input", "input": keys_json(&k)}),
        Packet::PlayerLoaded => json!({"name": "player_loaded"}),
        Packet::StartFallFlying => {
            json!({"name": "entity_action", "action": 6, "action_name": "start_fall_flying"})
        }
        Packet::Move {
            pos,
            look,
            on_ground,
            horizontal_collision,
        } => {
            let name = match (pos.is_some(), look.is_some()) {
                (true, true) => "position_look",
                (true, false) => "position",
                (false, true) => "look",
                (false, false) => "flying",
            };
            let mut v = json!({"name": name});
            if let Some(p) = pos {
                v["pos"] = json!([jf64(p[0]), jf64(p[1]), jf64(p[2])]);
            }
            if let Some((y, pi)) = look {
                v["yaw"] = jf32(y);
                v["pitch"] = jf32(pi);
            }
            v["on_ground"] = json!(on_ground);
            v["horizontal_collision"] = json!(horizontal_collision);
            v
        }
        other => unimplemented!("{other:?}"),
    }
}

/// Packet names ghast-physics produces for 26.1.
const PACKETS_26: [&str; 8] = [
    "player_loaded",
    "entity_action",
    "abilities",
    "flying",
    "position",
    "look",
    "position_look",
    "player_input",
];

/// `LpVec3` round trip of a velocity in blocks per tick (what the motion packet delivers).
pub fn lpvec3(v: [f64; 3]) -> [f64; 3] {
    let sanitize = |x: f64| {
        if x.is_nan() {
            0.0
        } else {
            x.clamp(-1.717_986_918_3e10, 1.717_986_918_3e10)
        }
    };
    let (x, y, z) = (sanitize(v[0]), sanitize(v[1]), sanitize(v[2]));
    let cl = x.abs().max(y.abs()).max(z.abs());
    if cl < 3.051_944_088_384_301e-5 {
        return [0.0; 3];
    }
    let scale = cl.ceil() as i64;
    let pack = |c: f64| ((c * 0.5 + 0.5) * 32766.0).round_ties_even_java();
    let unpack = |p: i64| (((p & 32767) as f64).min(32766.0)) * 2.0 / 32766.0 - 1.0;
    let s = scale as f64;
    let (xn, yn, zn) = (pack(x / s), pack(y / s), pack(z / s));
    [unpack(xn) * s, unpack(yn) * s, unpack(zn) * s]
}

trait JavaRound {
    fn round_ties_even_java(self) -> i64;
}

impl JavaRound for f64 {
    /// `Math.round(double)`: the exact `floor(x + 1/2)`.
    fn round_ties_even_java(self) -> i64 {
        let f = self.floor();
        if self - f >= 0.5 {
            f as i64 + 1
        } else {
            f as i64
        }
    }
}

/// The use-key model of the harness (`Minecraft.handleKeybinds` with the crosshair on nothing and
/// a shield in the main hand).
#[derive(Default)]
struct UseKey {
    down: bool,
    clicks: u32,
    delay: i32,
    using: bool,
    /// Player ticks the current use has lasted.
    held: u32,
}

impl UseKey {
    /// Returns how long the use lasted when the key released it this tick.
    fn tick(&mut self, down: bool, can_use: bool) -> Option<u32> {
        if down && !self.down {
            self.clicks += 1;
        }
        self.down = down;
        if self.delay > 0 {
            self.delay -= 1;
        }
        let mut released = None;
        if self.using {
            if !down {
                self.using = false;
                released = Some(self.held);
            }
            self.clicks = 0;
        } else {
            while self.clicks > 0 {
                self.clicks -= 1;
                self.start(can_use);
            }
        }
        if down && self.delay == 0 && !self.using {
            self.start(can_use);
        }
        released
    }

    fn start(&mut self, can_use: bool) {
        self.delay = 4;
        if can_use {
            self.using = true;
            self.held = 0;
        }
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

/// Replays one 26.1 case.
pub fn replay(case: &Case) -> Result<usize, Mismatch> {
    let name = case.header["name"].as_str().unwrap_or("?").to_string();
    let world = world_of(&case.header);
    let t0 = &case.ticks[0];
    let mut p = player_of(&case.header, t0);
    let mut x = ext_of(&case.header, t0);
    if case.header["setup"]
        .get("ext")
        .and_then(|e| e.get("dimension"))
        .and_then(Value::as_str)
        == Some("the_nether")
    {
        p.level = modern::LevelInfo {
            min_y: 0,
            height: 256,
            fast_lava: true,
        };
    }
    let mm = |tick: i64, (field, expected, actual): (String, String, String)| Mismatch {
        case: name.clone(),
        tick,
        field,
        expected,
        actual,
    };
    if let Some(d) = diff(
        "state",
        &t0["state"],
        &state_json_ext(&p, &x, &world, false),
    ) {
        return Err(mm(0, d));
    }
    // A hand item without a use duration (a firework rocket) is never "in use".
    let ext = case.header["setup"].get("ext");
    let hand = ext.and_then(|e| e.get("hand")).and_then(Value::as_str);
    let durable_use = hand.is_none() || hand == Some("trident");
    let riptide = ext.and_then(|e| e.get("riptide")).map_or(0, i);
    let raining = ext
        .and_then(|e| e.get("raining"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // The riptide enchantment's trident_spin_attack_strength: 1.5 + 0.75 per level above I.
    let strength = 1.5f32 + 0.75 * (riptide - 1) as f32;
    let has_item = durable_use
        && case.ticks.iter().any(|t| {
            t["input"]["keys"]
                .as_str()
                .is_some_and(|k| k.split(',').any(|n| n == "use"))
        });
    let mut use_key = UseKey::default();
    let delayed = case.header["setup"]["delayed_load"]
        .as_bool()
        .unwrap_or(false);
    let mut gate = if delayed {
        modern::LoadGate::waiting()
    } else {
        modern::LoadGate::loaded()
    };
    if case.header["setup"]
        .get("health")
        .is_some_and(|h| f32_of(h) <= 0.0)
    {
        gate.player_alive(false);
    }
    let mut compiled = false;
    let mut out = Packets::new();
    for rec in &case.ticks[1..] {
        let t = rec["t"].as_i64().unwrap();
        let input = &rec["input"];
        out.clear();
        let mut sent: Vec<Value> = Vec::new();
        if let Some(events) = input.get("events").and_then(Value::as_array) {
            for e in events {
                match e["type"].as_str().unwrap() {
                    "velocity" => {
                        let v = [
                            f64::from(i(&e["x"])) / 8000.0,
                            f64::from(i(&e["y"])) / 8000.0,
                            f64::from(i(&e["z"])) / 8000.0,
                        ];
                        p.apply_velocity(lpvec3(v));
                    }
                    "teleport" => {
                        let flags = e.get("flags").and_then(Value::as_u64).unwrap_or(0) as u32;
                        let delta = e.get("delta").map_or([0.0; 3], f3);
                        let reply = p.apply_teleport(
                            f3(&e["pos"]),
                            delta,
                            f32_of(&e["yaw"]),
                            f32_of(&e["pitch"]),
                            flags,
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
                        let mods: Vec<(ModifierId, f64, u8)> = e["modifiers"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|m| {
                                (
                                    modifier_id(m[0].as_str().unwrap()),
                                    f64_of(&m[1]),
                                    i(&m[2]) as u8,
                                )
                            })
                            .collect();
                        p.apply_attribute(AttributeKind::MovementSpeed, f64_of(&e["base"]), &mods);
                    }
                    "attribute" => {
                        let kind = AttributeKind::from_name(e["attribute"].as_str().unwrap())
                            .expect("movement attribute");
                        let mods: Vec<(ModifierId, f64, u8)> = e["modifiers"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|m| {
                                (
                                    ModifierId::new(m[0].as_str().unwrap()),
                                    f64_of(&m[1]),
                                    i(&m[2]) as u8,
                                )
                            })
                            .collect();
                        p.apply_attribute(kind, f64_of(&e["base"]), &mods);
                    }
                    "level_chunks_load_start" => gate.level_chunks_load_start(),
                    "section_compiled" => compiled = b(&e["compiled"]),
                    "load_timeout" => gate.time_out(),
                    "shared_flags" => {
                        let cur = x.shared_flags(&p);
                        let next = (cur & !(i(&e["clear"]) as u8)) | i(&e["set"]) as u8;
                        x.apply_shared_flags(&mut p, next);
                    }
                    "equipment" => {
                        assert_eq!(e["slot"], "chest");
                        x.glider = glider(e.get("item"), e.get("damage"));
                    }
                    "living_flags" => {
                        let cur = x.living_flags();
                        let next = (cur & !(i(&e["clear"]) as u8)) | i(&e["set"]) as u8;
                        x.apply_living_flags(next);
                    }
                    "firework" => {
                        assert!(x.attach_rocket(i(&e["id"]), p.pos));
                    }
                    "remove_entity" => x.remove_rocket(i(&e["id"])),
                    // Other entities: their boxes come with the tick record.
                    "spawn" | "entity_move" => {}
                    other => panic!("unknown event {other}"),
                }
            }
        }
        let (keys, use_down) = keys_of(input["keys"].as_str().unwrap());
        // Mouse turning happens before the key handling: a release launches along the new look.
        let (yaw, pitch) = (f32_of(&input["yaw"]), f32_of(&input["pitch"]));
        let (old_yaw, old_pitch) = (p.yaw, p.pitch);
        if riptide > 0 {
            p.yaw = yaw;
            p.pitch = pitch;
        }
        let wet = modern::in_water_or_rain::<V26_1, _>(&p, &world, raining);
        let can_use = has_item && !p.spectator && (riptide == 0 || wet);
        if let Some(held) = use_key.tick(use_down, can_use)
            && riptide > 0
            && held >= 10
            && wet
        {
            modern::riptide::<V26_1, _>(&mut p, &mut x, &world, strength);
        }
        if riptide > 0 && !gate.is_loaded() {
            p.yaw = old_yaw;
            p.pitch = old_pitch;
        }
        let ti = TickInput {
            keys,
            yaw: f32_of(&input["yaw"]),
            pitch: f32_of(&input["pitch"]),
            using_item: use_key.using,
        };
        // Other entities in the client's tick order: those before the player push it first (with
        // their box after their tick); the player collides with the collidable ones as they are
        // when it ticks; those after it push it after its tick.
        let ents = rec.get("entities").and_then(Value::as_array);
        let mut colliders: Vec<(i64, ghast_physics::math::Aabb)> = Vec::new();
        let mut after: Vec<modern::Pusher> = Vec::new();
        if let Some(ents) = ents {
            let mut before_player = true;
            for e in ents {
                if e.get("player").is_some() {
                    before_player = false;
                    continue;
                }
                let name = e["type"].as_str().unwrap();
                let living = ghast_data::v26_1::entity_by_name(name)
                    .and_then(|i| ghast_data::v26_1::entity_type(i.id))
                    .is_some_and(|t| t.is_living());
                let post = e.get("post").map(aabb_of);
                let pre = e.get("pre").map(aabb_of);
                let at_tick = if before_player { post } else { pre };
                if modern::collides_26_1(name)
                    && b(&e["alive"])
                    && let Some(bb) = at_tick
                {
                    let order = e
                        .get("query_order")
                        .and_then(Value::as_i64)
                        .unwrap_or(i64::MAX);
                    colliders.push((order, bb));
                }
                if let (Some(kind), Some(bb)) = (modern::push_kind_26_1(name, living), post) {
                    let pusher = modern::Pusher::new(f3(&e["pos"]), bb, kind);
                    if before_player {
                        modern::push_by::<V26_1, _>(&mut p, &x, &world, &pusher);
                    } else {
                        after.push(pusher);
                    }
                }
            }
        }
        // In the level's getEntities order (sections, then the order entities entered them).
        colliders.sort_by_key(|c| c.0);
        let colliders: Vec<ghast_physics::math::Aabb> =
            colliders.into_iter().map(|c| c.1).collect();
        let around = modern::Surroundings::new().with_entities(&colliders);
        gate.step_in::<V26_1, _>(&mut p, &mut x, &around, &ti, &world, compiled, &mut out);
        if gate.is_loaded() {
            for pusher in &after {
                modern::push_by::<V26_1, _>(&mut p, &x, &world, pusher);
            }
        }
        if use_key.using {
            use_key.held += 1;
        }
        if let Some(l) = rec.get("client_loaded")
            && l.as_bool() != Some(gate.is_loaded())
        {
            return Err(mm(
                t,
                (
                    "client_loaded".into(),
                    l.to_string(),
                    gate.is_loaded().to_string(),
                ),
            ));
        }
        sent.extend(out.as_slice().iter().map(packet_json));
        if let Some(d) = diff(
            "state",
            &rec["state"],
            &state_json_ext(&p, &x, &world, use_key.using),
        ) {
            return Err(mm(t, d));
        }
        let expected: Vec<Value> = rec["packets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| PACKETS_26.contains(&p["name"].as_str().unwrap_or("")))
            .cloned()
            .collect();
        if let Some(d) = diff("packets", &Value::Array(expected), &Value::Array(sent)) {
            return Err(mm(t, d));
        }
    }
    Ok(case.ticks.len() - 1)
}

/// Replays every case of a 26.1 file.
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
