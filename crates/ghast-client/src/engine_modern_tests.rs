//! The engine replays of a modern version's oracle goldens (`tests/golden/<version>/`): joins,
//! teleports, elytra, riptide, entities, the border, the nether, boats, minecarts, item use and the
//! entity pick, through the whole engine. Mounted once per modern version by `engine.rs` with
//! that version's modules (`proto`, `inv`, `data`, `golden`), `ModernSim`, `Marker`, `Wire`,
//! `GAME`, `INVENTORY` and `GOLDEN` as `super::*`.

#![allow(clippy::too_many_lines)]

use super::*;

/// As in the oracle's teleport goldens (`tests/golden/<version>/teleports.jsonl.gz`):
/// the tick a position packet arrives sends teleport_confirm, then position_look at the
/// pose `apply_teleport` gives, then the tick's own movement packet and tick_end.
#[test]
fn teleport_reply_matches_the_golden_order() {
    let mut e = engine(GAME);
    join(&mut e);
    let t0 = Instant::now();
    e.note_inbound(t0);
    let mut b = Idle;
    for _ in 0..3 {
        e.tick(&mut b, t0);
    }
    e.outbound().clear();
    let tp = Teleport {
        x: 2.0,
        y: 0.0,
        z: -1.0,
        yaw: 120.0,
        pitch: -20.0,
        // Relative x, y, z and yaw (0x1 | 0x2 | 0x4 | 0x8); delta absolute.
        flags: 0x0f,
        relative: 0x0f,
        teleport_id: Some(9),
        delta: Vec3::new(0.3, 0.42, -0.25),
    };
    let before = e.core.pose;
    e.handle(Inbound::Teleport(tp));
    e.tick(&mut b, t0);
    let out = std::mem::take(e.outbound());
    assert_eq!(out[0], Outbound::TeleportConfirm(9));
    match out[1] {
        Outbound::Move(MovePacket::PositionLook {
            pos,
            yaw,
            pitch,
            on_ground,
            ..
        }) => {
            assert_eq!(
                pos,
                Vec3::new(before.pos.x + 2.0, before.pos.y, before.pos.z - 1.0)
            );
            assert_eq!((yaw, pitch, on_ground), (before.yaw + 120.0, -20.0, false));
        }
        ref o => panic!("unexpected {o:?}"),
    }
    assert!(matches!(out[2], Outbound::Move(_)), "{out:?}");
    assert_eq!(out[3..], [Outbound::TickEnd]);
}

/// The oracle's elytra cases (`tests/golden/<version>/elytra.jsonl.gz`) through the whole engine:
/// the inbound events as the packets a server sends (own shared flags and firework rockets as
/// `set_entity_data` / `add_entity` / `remove_entities`, the chest slot as a window 0 slot),
/// the keys from the behaviour, the oracle's mouse; every movement, `entity_action` and
/// `player_input` packet must equal the real client's.
#[test]
fn elytra_matches_the_oracle() {
    use golden::{ext_of, player_of};
    use proto::play::clientbound as cb;
    use proto::types::{
        EntityMetadataEntry, EntityMetadataEntryType as T, EntityMetadataEntryValue as V,
    };
    use serde_json::Value;
    use std::io::BufRead;
    let path = format!(
        "{}/../../tests/golden/{GOLDEN}/elytra.jsonl.gz",
        env!("CARGO_MANIFEST_DIR")
    );
    let f = std::fs::File::open(path).unwrap();
    let lines: Vec<Value> = std::io::BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    let f64v = |v: &Value| f64::from_bits(u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap());
    let f32v = |v: &Value| f32::from_bits(u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap());
    // Packet body as the generated encoder writes it.
    fn body<P: ghast_protocol::Packet + ghast_protocol::Encode>(p: &P) -> bytes::Bytes {
        let mut enc = ghast_protocol::FrameEncoder::new();
        enc.write_packet(p).unwrap();
        let out = enc.out_mut().split().freeze();
        let mut r = &out[..];
        let skip_varint = |r: &mut &[u8]| {
            while r[0] & 0x80 != 0 {
                *r = &r[1..];
            }
            *r = &r[1..];
        };
        skip_varint(&mut r); // length
        skip_varint(&mut r); // id
        bytes::Bytes::copy_from_slice(r)
    }
    let metadata = |id: i32, key: u8, ty: T, value: V| RawPacket {
        id: cb::id::ENTITY_METADATA,
        body: body(&cb::EntityMetadata {
            entity_id: id,
            metadata: vec![EntityMetadataEntry {
                key,
                r#type: ty,
                value,
            }],
        }),
    };
    let key_name = |o: &Outbound| -> Option<Value> {
        Some(match *o {
            Outbound::Move(MovePacket::Position {
                pos,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!([
                    "position",
                    [pos.x.to_bits(), pos.y.to_bits(), pos.z.to_bits()],
                    on_ground,
                    horizontal_collision
                ])
            }
            Outbound::Move(MovePacket::PositionLook {
                pos,
                yaw,
                pitch,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!([
                    "position_look",
                    [pos.x.to_bits(), pos.y.to_bits(), pos.z.to_bits()],
                    yaw.to_bits(),
                    pitch.to_bits(),
                    on_ground,
                    horizontal_collision
                ])
            }
            Outbound::Move(MovePacket::Look {
                yaw,
                pitch,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!([
                    "look",
                    yaw.to_bits(),
                    pitch.to_bits(),
                    on_ground,
                    horizontal_collision
                ])
            }
            Outbound::Move(MovePacket::Flying {
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!(["flying", on_ground, horizontal_collision])
            }
            Outbound::EntityAction { action, .. } => serde_json::json!([
                "entity_action",
                match action {
                    EntityAction::StartSprinting => "start_sprinting",
                    EntityAction::StopSprinting => "stop_sprinting",
                    EntityAction::StartFallFlying => "start_fall_flying",
                    _ => "other",
                }
            ]),
            Outbound::UseItem { off_hand, .. } => {
                serde_json::json!(["use_item", i32::from(off_hand)])
            }
            Outbound::Swing => serde_json::json!(["arm_animation", 0]),
            Outbound::SwingOffHand => serde_json::json!(["arm_animation", 1]),
            Outbound::PlayerInput(c) => serde_json::json!([
                "player_input",
                [
                    c.forward, c.back, c.left, c.right, c.jump, c.sneak, c.sprint
                ]
            ]),
            _ => return None,
        })
    };
    let golden = |p: &Value| -> Option<Value> {
        let v3 = |v: &Value| {
            [
                f64v(&v[0]).to_bits(),
                f64v(&v[1]).to_bits(),
                f64v(&v[2]).to_bits(),
            ]
        };
        let (og, hc) = (&p["on_ground"], &p["horizontal_collision"]);
        Some(match p["name"].as_str().unwrap() {
            "position" => serde_json::json!(["position", v3(&p["pos"]), og, hc]),
            "position_look" => serde_json::json!([
                "position_look",
                v3(&p["pos"]),
                f32v(&p["yaw"]).to_bits(),
                f32v(&p["pitch"]).to_bits(),
                og,
                hc
            ]),
            "look" => serde_json::json!([
                "look",
                f32v(&p["yaw"]).to_bits(),
                f32v(&p["pitch"]).to_bits(),
                og,
                hc
            ]),
            "flying" => serde_json::json!(["flying", og, hc]),
            "entity_action" => serde_json::json!(["entity_action", p["action_name"]]),
            "use_item" => serde_json::json!(["use_item", p["hand"]]),
            "arm_animation" => serde_json::json!(["arm_animation", p["hand"]]),
            "player_input" => {
                let i = &p["input"];
                serde_json::json!([
                    "player_input",
                    [
                        i["forward"],
                        i["backward"],
                        i["left"],
                        i["right"],
                        i["jump"],
                        i["shift"],
                        i["sprint"]
                    ]
                ])
            }
            _ => return None,
        })
    };
    /// Holds the golden keys for the tick.
    struct Keys(Controls, bool);
    impl Behavior for Keys {
        fn name(&self) -> &str {
            "elytra-replay"
        }
        fn tick(&mut self, ctx: &mut TickContext<'_>) {
            ctx.set_controls(self.0);
            ctx.set_buttons(crate::Buttons {
                left: false,
                right: self.1,
            });
        }
    }
    let elytra = inv::item_id("elytra").unwrap();
    // Hand-written cases with the events and setup this replay drives.
    let wanted = [
        "elytra_start_glide",
        "elytra_jump_from_ground",
        "elytra_no_chest",
        "elytra_landing",
        "elytra_steep",
        "elytra_fireworks",
        "elytra_firework_not_gliding",
        "elytra_server_flags",
        "elytra_setup_gliding",
        "elytra_unequip",
        "elytra_rocket_chunk_edge",
        "elytra_rocket_chunk_edge_left",
        "elytra_firework_use",
    ];
    let (mut cases, mut ticks_done, mut packets, mut uses) = (0, 0, 0, 0);
    let mut i = 0;
    while i < lines.len() {
        let sc = &lines[i];
        let name = sc["name"].as_str().unwrap().to_owned();
        let mut ticks = Vec::new();
        i += 1;
        while i < lines.len() && lines[i]["type"] == "tick" {
            ticks.push(&lines[i]);
            i += 1;
        }
        if !wanted.contains(&name.as_str()) {
            continue;
        }
        cases += 1;
        let setup = &sc["setup"];
        let mut o = opts(GAME);
        o.inventory = INVENTORY;
        let ext = setup.get("ext");
        if ext
            .and_then(|x| x.get("left_handed"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            o.settings.main_hand = 0;
        }
        let mut e: Engine<Box<dyn Sim>> = Engine::new(o, Box::new(ModernSim::test_overworld()));
        e.handle(Inbound::JoinGame(JoinGame {
            entity_id: 7,
            game_mode: GameMode::Survival,
            dimension: "0".into(),
            hardcore: false,
            dimension_nbt: None,
            respawn_screen: true,
        }));
        // Our entity id for the own-entity packets (the login packet's).
        e.sim.test_set_own_id(7);
        for r in sc["loaded_chunks"].as_array().unwrap() {
            let r: Vec<i32> = (0..4).map(|k| r[k].as_i64().unwrap() as i32).collect();
            for x in r[0]..=r[2] {
                for z in r[1]..=r[3] {
                    load(&mut e, x, z);
                }
            }
        }
        for fill in sc["fills"].as_array().unwrap() {
            assert_eq!(
                sc["states"][fill[6].as_i64().unwrap().to_string()],
                "minecraft:stone",
                "{name}"
            );
            assert_eq!(
                (fill[1].as_i64(), fill[4].as_i64()),
                (Some(63), Some(63)),
                "{name}"
            );
        }
        if let Some(h) = ext.and_then(|x| x.get("hand")).and_then(Value::as_str) {
            let id = inv::item_id(h).unwrap();
            e.inventory_mut()
                .on_set_slot(0, 36, Some(ghast_inventory::Stack::new(id, 16, 0)));
        }
        if ext.and_then(|x| x.get("chest")).and_then(Value::as_str) == Some("elytra") {
            assert!(ext.unwrap().get("chest_damage").is_none(), "{name}");
            e.inventory_mut()
                .on_set_slot(0, 6, Some(ghast_inventory::Stack::new(elytra, 1, 0)));
        }
        e.handle(Inbound::Teleport(Teleport {
            x: f64v(&setup["pos"][0]),
            y: f64v(&setup["pos"][1]),
            z: f64v(&setup["pos"][2]),
            yaw: f32v(&setup["yaw"]),
            pitch: f32v(&setup["pitch"]),
            teleport_id: None,
            ..Default::default()
        }));
        let p = player_of(sc, ticks[0]);
        e.sim.test_set_modern(p, ext_of(sc, ticks[0]));
        e.core.pose.pos = Vec3::new(p.pos[0], p.pos[1], p.pos[2]);
        e.core.pose.yaw = p.yaw;
        e.core.pose.pitch = p.pitch;
        e.core.level_load_started = true;
        e.core.client_loaded = true;
        e.core.ready = true;
        e.outbound().clear();
        let t0 = Instant::now();
        for (k, rec) in ticks.iter().enumerate().skip(1) {
            let input = &rec["input"];
            for ev in input["events"].as_array().into_iter().flatten() {
                match ev["type"].as_str().unwrap() {
                    "shared_flags" => {
                        let cur = e.sim.test_shared_flags();
                        let set = ev["set"].as_u64().unwrap() as u8;
                        let clear = ev["clear"].as_u64().unwrap() as u8;
                        let next = (cur & !clear) | set;
                        e.sim
                            .world_packet(&metadata(7, 0, T::BYTE, V::Byte(next as i8)))
                            .unwrap();
                    }
                    "firework" => {
                        let id = ev["id"].as_i64().unwrap() as i32;
                        let pos = e.core.pose.pos;
                        let spawn = RawPacket {
                            id: cb::id::SPAWN_ENTITY,
                            body: body(&cb::SpawnEntity {
                                entity_id: id,
                                object_uuid: uuid::Uuid::from_u64_pair(0x6669_7265, id as u64),
                                r#type: 53,
                                x: pos.x,
                                y: pos.y,
                                z: pos.z,
                                velocity: ghast_protocol::types::LpVec3::ZERO,
                                pitch: 0,
                                yaw: 0,
                                head_pitch: 0,
                                object_data: 0,
                            }),
                        };
                        e.sim.world_packet(&spawn).unwrap();
                        e.sim
                            .world_packet(&metadata(
                                id,
                                9,
                                T::OPTIONAL_UNSIGNED_INT,
                                V::OptionalUnsignedInt(8),
                            ))
                            .unwrap();
                    }
                    "remove_entity" => {
                        let raw = RawPacket {
                            id: cb::id::ENTITY_DESTROY,
                            body: body(&cb::EntityDestroy {
                                entity_ids: vec![ev["id"].as_i64().unwrap() as i32],
                            }),
                        };
                        e.sim.world_packet(&raw).unwrap();
                    }
                    "equipment" => {
                        assert_eq!(ev["slot"], "chest");
                        let item = (ev["item"] == "elytra")
                            .then(|| ghast_inventory::Stack::new(elytra, 1, 0));
                        e.inventory_mut().on_set_slot(0, 6, item);
                    }
                    other => panic!("{name}: event {other}"),
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
            let sent_look = (e.core.sent.yaw, e.core.sent.pitch);
            e.tick(&mut b, t0);
            let out = std::mem::take(e.outbound());
            // use_item carries the rotation of the tick's movement packet (the wire layer's
            // prepare_play), the rotation the real client sends.
            if let Some(u) = rec["packets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == "use_item")
            {
                use crate::version::ClientVersion;
                let mut wire = Wire::default();
                wire.set_last_look(sent_look.0, sent_look.1);
                Marker::prepare_play(&out, &mut wire);
                let mut enc = ghast_protocol::FrameEncoder::new();
                for o in out.iter().filter(|o| matches!(o, Outbound::UseItem { .. })) {
                    Marker::write_play(&mut enc, o, &mut wire).unwrap();
                }
                let frame = enc.out_mut().split().freeze();
                // ... then yaw and pitch (f32 big-endian) end the packet.
                let n = frame.len();
                let yaw = f32::from_be_bytes(frame[n - 8..n - 4].try_into().unwrap());
                let pitch = f32::from_be_bytes(frame[n - 4..].try_into().unwrap());
                assert_eq!(
                    (yaw.to_bits(), pitch.to_bits()),
                    (f32v(&u["yaw"]).to_bits(), f32v(&u["pitch"]).to_bits()),
                    "{name} tick {k}: use_item rotation"
                );
                uses += 1;
            }
            let got: Vec<Value> = out.iter().filter_map(&key_name).collect();
            let want: Vec<Value> = rec["packets"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(&golden)
                .collect();
            assert_eq!(got, want, "{name} tick {k}");
            packets += want.len();
            ticks_done += 1;
        }
    }
    eprintln!(
        "elytra through the engine: {cases} cases, {ticks_done} ticks, {packets} packets identical ({uses} use_item rotations)"
    );
    assert_eq!(cases, wanted.len());
    assert!(uses >= 2, "{uses}");
}

/// The oracle's riptide cases (`tests/golden/<version>/riptide.jsonl.gz`) through the whole engine:
/// the trident with its riptide enchantment in the main hand (the enchantment registry
/// synced as in configuration), the rain as weather game events, the server's living flags
/// as `set_entity_data`, the use key from the behaviour; every movement, `entity_action`,
/// `player_input`, `use_item` and `player_action` packet must equal the real client's.
#[test]
fn riptide_matches_the_oracle() {
    use golden::{ext_of, player_of};
    use proto::play::clientbound as cb;
    use proto::types::{
        EntityMetadataEntry, EntityMetadataEntryType as T, EntityMetadataEntryValue as V,
    };
    use serde_json::Value;
    use std::io::BufRead;
    let path = format!(
        "{}/../../tests/golden/{GOLDEN}/riptide.jsonl.gz",
        env!("CARGO_MANIFEST_DIR")
    );
    let f = std::fs::File::open(path).unwrap();
    let lines: Vec<Value> = std::io::BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    let f64v = |v: &Value| f64::from_bits(u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap());
    let f32v = |v: &Value| f32::from_bits(u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap());
    // Packet body as the generated encoder writes it.
    fn body<P: ghast_protocol::Packet + ghast_protocol::Encode>(p: &P) -> bytes::Bytes {
        let mut enc = ghast_protocol::FrameEncoder::new();
        enc.write_packet(p).unwrap();
        let out = enc.out_mut().split().freeze();
        let mut r = &out[..];
        let skip_varint = |r: &mut &[u8]| {
            while r[0] & 0x80 != 0 {
                *r = &r[1..];
            }
            *r = &r[1..];
        };
        skip_varint(&mut r); // length
        skip_varint(&mut r); // id
        bytes::Bytes::copy_from_slice(r)
    }
    let metadata = |id: i32, key: u8, ty: T, value: V| RawPacket {
        id: cb::id::ENTITY_METADATA,
        body: body(&cb::EntityMetadata {
            entity_id: id,
            metadata: vec![EntityMetadataEntry {
                key,
                r#type: ty,
                value,
            }],
        }),
    };
    let key_name = |o: &Outbound| -> Option<Value> {
        Some(match *o {
            Outbound::Move(MovePacket::Position {
                pos,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!([
                    "position",
                    [pos.x.to_bits(), pos.y.to_bits(), pos.z.to_bits()],
                    on_ground,
                    horizontal_collision
                ])
            }
            Outbound::Move(MovePacket::PositionLook {
                pos,
                yaw,
                pitch,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!([
                    "position_look",
                    [pos.x.to_bits(), pos.y.to_bits(), pos.z.to_bits()],
                    yaw.to_bits(),
                    pitch.to_bits(),
                    on_ground,
                    horizontal_collision
                ])
            }
            Outbound::Move(MovePacket::Look {
                yaw,
                pitch,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!([
                    "look",
                    yaw.to_bits(),
                    pitch.to_bits(),
                    on_ground,
                    horizontal_collision
                ])
            }
            Outbound::Move(MovePacket::Flying {
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!(["flying", on_ground, horizontal_collision])
            }
            Outbound::EntityAction { action, .. } => serde_json::json!([
                "entity_action",
                match action {
                    EntityAction::StartSprinting => "start_sprinting",
                    EntityAction::StopSprinting => "stop_sprinting",
                    EntityAction::StartFallFlying => "start_fall_flying",
                    _ => "other",
                }
            ]),
            Outbound::Dig {
                status: crate::common::DigStatus::ReleaseUseItem,
                ..
            } => serde_json::json!(["block_dig", 5]),
            Outbound::UseItem { off_hand, .. } => {
                serde_json::json!(["use_item", i32::from(off_hand)])
            }
            Outbound::Swing => serde_json::json!(["arm_animation", 0]),
            Outbound::SwingOffHand => serde_json::json!(["arm_animation", 1]),
            Outbound::PlayerInput(c) => serde_json::json!([
                "player_input",
                [
                    c.forward, c.back, c.left, c.right, c.jump, c.sneak, c.sprint
                ]
            ]),
            _ => return None,
        })
    };
    let golden = |p: &Value| -> Option<Value> {
        let v3 = |v: &Value| {
            [
                f64v(&v[0]).to_bits(),
                f64v(&v[1]).to_bits(),
                f64v(&v[2]).to_bits(),
            ]
        };
        let (og, hc) = (&p["on_ground"], &p["horizontal_collision"]);
        Some(match p["name"].as_str().unwrap() {
            "position" => serde_json::json!(["position", v3(&p["pos"]), og, hc]),
            "position_look" => serde_json::json!([
                "position_look",
                v3(&p["pos"]),
                f32v(&p["yaw"]).to_bits(),
                f32v(&p["pitch"]).to_bits(),
                og,
                hc
            ]),
            "look" => serde_json::json!([
                "look",
                f32v(&p["yaw"]).to_bits(),
                f32v(&p["pitch"]).to_bits(),
                og,
                hc
            ]),
            "flying" => serde_json::json!(["flying", og, hc]),
            "entity_action" => serde_json::json!(["entity_action", p["action_name"]]),
            "use_item" => serde_json::json!(["use_item", p["hand"]]),
            "block_dig" => serde_json::json!(["block_dig", p["status"]]),
            "arm_animation" => serde_json::json!(["arm_animation", p["hand"]]),
            "player_input" => {
                let i = &p["input"];
                serde_json::json!([
                    "player_input",
                    [
                        i["forward"],
                        i["backward"],
                        i["left"],
                        i["right"],
                        i["jump"],
                        i["shift"],
                        i["sprint"]
                    ]
                ])
            }
            _ => return None,
        })
    };
    /// Holds the golden keys for the tick.
    struct Keys(Controls, bool);
    impl Behavior for Keys {
        fn name(&self) -> &str {
            "elytra-replay"
        }
        fn tick(&mut self, ctx: &mut TickContext<'_>) {
            ctx.set_controls(self.0);
            ctx.set_buttons(crate::Buttons {
                left: false,
                right: self.1,
            });
        }
    }
    let elytra = inv::item_id("elytra").unwrap();
    // Hand-written cases with the events and setup this replay drives.
    let wanted = [
        "riptide_water",
        "riptide_short_hold",
        "riptide_dry",
        "riptide_rain",
        "riptide_rain_roof",
        "riptide_wall",
        "riptide_level1",
        "riptide_level2",
        "riptide_level3",
        "riptide_spin_pose",
        "riptide_then_glide",
    ];
    let (mut cases, mut ticks_done, mut packets, mut uses) = (0, 0, 0, 0);
    let mut i = 0;
    while i < lines.len() {
        let sc = &lines[i];
        let name = sc["name"].as_str().unwrap().to_owned();
        let mut ticks = Vec::new();
        i += 1;
        while i < lines.len() && lines[i]["type"] == "tick" {
            ticks.push(&lines[i]);
            i += 1;
        }
        if !wanted.contains(&name.as_str()) {
            continue;
        }
        cases += 1;
        let setup = &sc["setup"];
        let mut o = opts(GAME);
        o.inventory = INVENTORY;
        let ext = setup.get("ext");
        if ext
            .and_then(|x| x.get("left_handed"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            o.settings.main_hand = 0;
        }
        let mut e: Engine<Box<dyn Sim>> = Engine::new(o, Box::new(ModernSim::test_overworld()));
        e.handle(Inbound::JoinGame(JoinGame {
            entity_id: 7,
            game_mode: GameMode::Survival,
            dimension: "0".into(),
            hardcore: false,
            dimension_nbt: None,
            respawn_screen: true,
        }));
        // Our entity id for the own-entity packets (the login packet's).
        e.sim.test_set_own_id(7);
        for r in sc["loaded_chunks"].as_array().unwrap() {
            let r: Vec<i32> = (0..4).map(|k| r[k].as_i64().unwrap() as i32).collect();
            for x in r[0]..=r[2] {
                for z in r[1]..=r[3] {
                    load(&mut e, x, z);
                }
            }
        }
        // The stone layer y 63 is the test chunks' own; the other fills as block changes.
        for fill in sc["fills"].as_array().unwrap() {
            let v: Vec<i64> = (0..7).map(|k| fill[k].as_i64().unwrap()).collect();
            let state = sc["states"][v[6].to_string()].as_str().unwrap();
            if state == "minecraft:stone" && v[1] == 63 && v[4] == 63 {
                continue;
            }
            let id = data::BlockState::parse(state.strip_prefix("minecraft:").unwrap_or(state))
                .or_else(|| data::BlockState::parse(state))
                .unwrap()
                .id();
            for x in v[0]..=v[3] {
                for y in v[1]..=v[4] {
                    for z in v[2]..=v[5] {
                        let pos = ((x & 0x3FF_FFFF) << 38) | ((z & 0x3FF_FFFF) << 12) | (y & 0xFFF);
                        let mut b = pos.to_be_bytes().to_vec();
                        let mut st = u32::from(id);
                        loop {
                            let byte = (st & 0x7f) as u8;
                            st >>= 7;
                            if st == 0 {
                                b.push(byte);
                                break;
                            }
                            b.push(byte | 0x80);
                        }
                        e.sim
                            .world_packet(&RawPacket {
                                id: cb::id::BLOCK_CHANGE,
                                body: bytes::Bytes::from(b),
                            })
                            .unwrap();
                    }
                }
            }
        }
        // The enchantment registry as configuration syncs it (names only: vanilla data).
        {
            use proto::configuration::clientbound as cc;
            let reg = cc::RegistryData {
                id: "minecraft:enchantment".into(),
                entries: ["minecraft:riptide"]
                    .iter()
                    .map(|k| cc::RegistryDataEntries {
                        key: (*k).into(),
                        value: None,
                    })
                    .collect(),
            };
            let raw = RawPacket {
                id: cc::id::REGISTRY_DATA,
                body: body(&reg),
            };
            e.sim.registry_data(&raw);
            // ... and as the inventory gets it (the riptide strength reads it).
            let mut names = inv::HashRegistries::default();
            names.add("minecraft:enchantment", ["minecraft:riptide"]);
            wire_row::inv_set_registries(e.inventory_mut(), names);
            wire_row::inv_on_registry_data(e.inventory_mut(), raw.body.clone());
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
        let mut living = 0u8;
        if let Some(h) = ext.and_then(|x| x.get("hand")).and_then(Value::as_str) {
            let id = inv::item_id(h).unwrap();
            let mut stack = ghast_inventory::Stack::new(id, 1, 0);
            if let Some(level) = ext.and_then(|x| x.get("riptide")).and_then(Value::as_u64) {
                // enchantments: one entry, registry index 0 (riptide), the level.
                let patch = enchantments_patch(&[1, 0, level as u8]);
                stack = stack.with_data(ghast_inventory::ItemData::Components(Box::new(patch)));
            }
            e.inventory_mut().on_set_slot(0, 36, Some(stack));
        }
        if ext.and_then(|x| x.get("chest")).and_then(Value::as_str) == Some("elytra") {
            assert!(ext.unwrap().get("chest_damage").is_none(), "{name}");
            e.inventory_mut()
                .on_set_slot(0, 6, Some(ghast_inventory::Stack::new(elytra, 1, 0)));
        }
        e.handle(Inbound::Teleport(Teleport {
            x: f64v(&setup["pos"][0]),
            y: f64v(&setup["pos"][1]),
            z: f64v(&setup["pos"][2]),
            yaw: f32v(&setup["yaw"]),
            pitch: f32v(&setup["pitch"]),
            teleport_id: None,
            ..Default::default()
        }));
        let p = player_of(sc, ticks[0]);
        e.sim.test_set_modern(p, ext_of(sc, ticks[0]));
        e.core.pose.pos = Vec3::new(p.pos[0], p.pos[1], p.pos[2]);
        e.core.pose.yaw = p.yaw;
        e.core.pose.pitch = p.pitch;
        e.core.level_load_started = true;
        e.core.client_loaded = true;
        e.core.ready = true;
        e.outbound().clear();
        let t0 = Instant::now();
        for (k, rec) in ticks.iter().enumerate().skip(1) {
            let input = &rec["input"];
            for ev in input["events"].as_array().into_iter().flatten() {
                match ev["type"].as_str().unwrap() {
                    "shared_flags" => {
                        let cur = e.sim.test_shared_flags();
                        let set = ev["set"].as_u64().unwrap() as u8;
                        let clear = ev["clear"].as_u64().unwrap() as u8;
                        let next = (cur & !clear) | set;
                        e.sim
                            .world_packet(&metadata(7, 0, T::BYTE, V::Byte(next as i8)))
                            .unwrap();
                    }
                    "firework" => {
                        let id = ev["id"].as_i64().unwrap() as i32;
                        let pos = e.core.pose.pos;
                        let spawn = RawPacket {
                            id: cb::id::SPAWN_ENTITY,
                            body: body(&cb::SpawnEntity {
                                entity_id: id,
                                object_uuid: uuid::Uuid::from_u64_pair(0x6669_7265, id as u64),
                                r#type: 53,
                                x: pos.x,
                                y: pos.y,
                                z: pos.z,
                                velocity: ghast_protocol::types::LpVec3::ZERO,
                                pitch: 0,
                                yaw: 0,
                                head_pitch: 0,
                                object_data: 0,
                            }),
                        };
                        e.sim.world_packet(&spawn).unwrap();
                        e.sim
                            .world_packet(&metadata(
                                id,
                                9,
                                T::OPTIONAL_UNSIGNED_INT,
                                V::OptionalUnsignedInt(8),
                            ))
                            .unwrap();
                    }
                    "remove_entity" => {
                        let raw = RawPacket {
                            id: cb::id::ENTITY_DESTROY,
                            body: body(&cb::EntityDestroy {
                                entity_ids: vec![ev["id"].as_i64().unwrap() as i32],
                            }),
                        };
                        e.sim.world_packet(&raw).unwrap();
                    }
                    "living_flags" => {
                        let set = ev["set"].as_u64().unwrap() as u8;
                        let clear = ev["clear"].as_u64().unwrap() as u8;
                        living = (living & !clear) | set;
                        e.sim
                            .world_packet(&metadata(7, 8, T::BYTE, V::Byte(living as i8)))
                            .unwrap();
                    }
                    "equipment" => {
                        assert_eq!(ev["slot"], "chest");
                        let item = (ev["item"] == "elytra")
                            .then(|| ghast_inventory::Stack::new(elytra, 1, 0));
                        e.inventory_mut().on_set_slot(0, 6, item);
                    }
                    other => panic!("{name}: event {other}"),
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
            let sent_look = (e.core.sent.yaw, e.core.sent.pitch);
            e.tick(&mut b, t0);
            let out = std::mem::take(e.outbound());
            // use_item carries the rotation of the tick's movement packet (the wire layer's
            // prepare_play), the rotation the real client sends.
            if let Some(u) = rec["packets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == "use_item")
            {
                use crate::version::ClientVersion;
                let mut wire = Wire::default();
                wire.set_last_look(sent_look.0, sent_look.1);
                Marker::prepare_play(&out, &mut wire);
                let mut enc = ghast_protocol::FrameEncoder::new();
                for o in out.iter().filter(|o| matches!(o, Outbound::UseItem { .. })) {
                    Marker::write_play(&mut enc, o, &mut wire).unwrap();
                }
                let frame = enc.out_mut().split().freeze();
                // ... then yaw and pitch (f32 big-endian) end the packet.
                let n = frame.len();
                let yaw = f32::from_be_bytes(frame[n - 8..n - 4].try_into().unwrap());
                let pitch = f32::from_be_bytes(frame[n - 4..].try_into().unwrap());
                assert_eq!(
                    (yaw.to_bits(), pitch.to_bits()),
                    (f32v(&u["yaw"]).to_bits(), f32v(&u["pitch"]).to_bits()),
                    "{name} tick {k}: use_item rotation"
                );
                uses += 1;
            }
            let got: Vec<Value> = out.iter().filter_map(&key_name).collect();
            let want: Vec<Value> = rec["packets"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(&golden)
                .collect();
            assert_eq!(got, want, "{name} tick {k}");
            packets += want.len();
            ticks_done += 1;
        }
    }
    eprintln!(
        "riptide through the engine: {cases} cases, {ticks_done} ticks, {packets} packets identical ({uses} use_item rotations)"
    );
    assert_eq!(cases, wanted.len());
    assert!(uses >= 10, "{uses}");
}

/// 26.1 entity collision and pushing through the engine (entity tracking on): the
/// hand-written cases of `tests/golden/<version>/entities.jsonl.gz` with their entities as real
/// `add_entity` / `teleport_entity` / `remove_entities` packets into the tracker, and every
/// fuzz case whose events this replay drives; every movement, `entity_action` and
/// `player_input` packet must equal the real client's.
#[test]
fn entities_match_the_oracle() {
    let (cases, fuzz) = ext_replay(
        "entities.jsonl.gz",
        &[
            "push_zombie",
            "push_crowd",
            "push_walk_into_mobs",
            "no_push_armor_stand_bat_parrot",
            "boat_collide",
            "boat_push",
            "shulker_block",
            "minecart_near",
            "push_moving_zombie",
            "push_ladder",
            "push_spectator",
            "boat_stand_on",
            "push_spawned_in",
        ],
    );
    assert!(cases >= 13 && fuzz >= 5, "{cases} {fuzz}");
}

/// 26.1 world border through the engine: `initialize` / `set_border_center` /
/// `set_border_size` / `set_border_lerp_size` packets into the world, the border stepping
/// after the entities (`tests/golden/<version>/border.jsonl.gz`).
#[test]
fn border_matches_the_oracle() {
    let (cases, fuzz) = ext_replay(
        "border.jsonl.gz",
        &[
            "border_shrinking",
            "border_growing",
            "border_static_changes",
            "border_outside",
        ],
    );
    assert!(cases >= 4 && fuzz >= 5, "{cases} {fuzz}");
}

/// 26.1 nether through the engine: the level facts (build height 0-255, fast lava) from the
/// synced `the_nether` dimension type (`tests/golden/<version>/nether.jsonl.gz`).
#[test]
fn nether_matches_the_oracle() {
    let (cases, fuzz) = ext_replay(
        "nether.jsonl.gz",
        &[
            "nether_lava_flow",
            "nether_lava_fall",
            "nether_low_world",
            "nether_top",
        ],
    );
    assert!(cases >= 4 && fuzz >= 5, "{cases} {fuzz}");
}

/// 26.1 boats through the engine (`tests/golden/<version>/boats.jsonl.gz`): boarding from
/// `set_passengers`, the boat's tick with its passenger, `steer_boat` / look / `vehicle_move`
/// packets, the server's `move_vehicle` answered at once; with entity tracking (the boat's
/// position from the tracker, other boats collide) and without (the boat known from its spawn
/// packet).
#[test]
fn boats_match_the_oracle() {
    let hand = [
        "boat_water_paddle",
        "boat_raft_turn_look",
        "boat_land",
        "boat_ice",
        "boat_drop_into_water",
        "boat_wall",
        "boat_flowing",
        "boat_under_water",
        "boat_slab_soul_sand",
        "boat_hits_boat",
        "boat_server_correction",
        "boat_dismount",
    ];
    let (cases, fuzz) = ext_replay_with("boats.jsonl.gz", &hand, true);
    assert!(cases >= 12 && fuzz >= 5, "{cases} {fuzz}");
    // Without the tracker: only the cases without other entities near the boat.
    let alone: Vec<&str> = hand
        .iter()
        .copied()
        .filter(|n| *n != "boat_hits_boat")
        .collect();
    let (cases, fuzz) = ext_replay_with("boats.jsonl.gz", &alone, false);
    assert!(cases >= 11 && fuzz >= 5, "{cases} {fuzz}");
}

/// 26.1 minecarts through the engine (`tests/golden/<version>/minecarts.jsonl.gz`): seated from
/// `set_passengers`, the cart's position after its own client tick (interpolating the
/// server's teleports), the passenger's tick on it; with and without entity tracking.
#[test]
fn minecarts_match_the_oracle() {
    let hand = [
        "cart_idle_keys",
        "cart_moved_by_server",
        "cart_into_wall",
        "cart_through_water",
        "cart_dismount",
    ];
    let (cases, fuzz) = ext_replay_with("minecarts.jsonl.gz", &hand, true);
    assert!(cases >= 5 && fuzz >= 5, "{cases} {fuzz}");
    let (cases, fuzz) = ext_replay_with("minecarts.jsonl.gz", &hand, false);
    assert!(cases >= 5 && fuzz >= 5, "{cases} {fuzz}");
}

/// Replays the hand-written cases `hand` and the fuzz cases whose events it drives (entities,
/// border, own velocity) of a 26.1 oracle file through the engine; returns (cases, fuzz).
fn ext_replay(file: &str, hand: &[&str]) -> (usize, usize) {
    ext_replay_with(file, hand, true)
}

/// [`ext_replay`] with or without entity tracking.
fn ext_replay_with(file: &str, hand: &[&str], tracking: bool) -> (usize, usize) {
    use golden::{ext_of, player_of};
    use proto::play::clientbound as cb;
    use serde_json::Value;
    use std::io::BufRead;
    let path = format!(
        "{}/../../tests/golden/{GOLDEN}/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    let f = std::fs::File::open(path).unwrap();
    let lines: Vec<Value> = std::io::BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    let f64v = |v: &Value| f64::from_bits(u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap());
    let f32v = |v: &Value| f32::from_bits(u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap());
    fn body<P: ghast_protocol::Packet + ghast_protocol::Encode>(p: &P) -> bytes::Bytes {
        let mut enc = ghast_protocol::FrameEncoder::new();
        enc.write_packet(p).unwrap();
        let out = enc.out_mut().split().freeze();
        let mut r = &out[..];
        for _ in 0..2 {
            while r[0] & 0x80 != 0 {
                r = &r[1..];
            }
            r = &r[1..];
        }
        bytes::Bytes::copy_from_slice(r)
    }
    let v3 = |v: &Value| [f64v(&v[0]), f64v(&v[1]), f64v(&v[2])];
    let bits = |p: [f64; 3]| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
    let key_name = |o: &Outbound| -> Option<Value> {
        Some(match *o {
            Outbound::Move(MovePacket::Position {
                pos,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!([
                    "position",
                    bits([pos.x, pos.y, pos.z]),
                    on_ground,
                    horizontal_collision
                ])
            }
            Outbound::Move(MovePacket::PositionLook {
                pos,
                yaw,
                pitch,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!([
                    "position_look",
                    bits([pos.x, pos.y, pos.z]),
                    yaw.to_bits(),
                    pitch.to_bits(),
                    on_ground,
                    horizontal_collision
                ])
            }
            Outbound::Move(MovePacket::Look {
                yaw,
                pitch,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!([
                    "look",
                    yaw.to_bits(),
                    pitch.to_bits(),
                    on_ground,
                    horizontal_collision
                ])
            }
            Outbound::Move(MovePacket::Flying {
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!(["flying", on_ground, horizontal_collision])
            }
            Outbound::EntityAction { action, .. } => serde_json::json!([
                "entity_action",
                match action {
                    EntityAction::StartSprinting => "start_sprinting",
                    EntityAction::StopSprinting => "stop_sprinting",
                    _ => "other",
                }
            ]),
            Outbound::VehicleMove {
                pos,
                yaw,
                pitch,
                on_ground,
            } => serde_json::json!([
                "vehicle_move",
                bits([pos.x, pos.y, pos.z]),
                yaw.to_bits(),
                pitch.to_bits(),
                on_ground
            ]),
            Outbound::SteerBoat { left, right } => {
                serde_json::json!(["steer_boat", left, right])
            }
            Outbound::PlayerInput(c) => serde_json::json!([
                "player_input",
                [
                    c.forward, c.back, c.left, c.right, c.jump, c.sneak, c.sprint
                ]
            ]),
            _ => return None,
        })
    };
    let golden = |p: &Value| -> Option<Value> {
        let (og, hc) = (&p["on_ground"], &p["horizontal_collision"]);
        Some(match p["name"].as_str().unwrap() {
            "position" => serde_json::json!(["position", bits(v3(&p["pos"])), og, hc]),
            "position_look" => serde_json::json!([
                "position_look",
                bits(v3(&p["pos"])),
                f32v(&p["yaw"]).to_bits(),
                f32v(&p["pitch"]).to_bits(),
                og,
                hc
            ]),
            "look" => serde_json::json!([
                "look",
                f32v(&p["yaw"]).to_bits(),
                f32v(&p["pitch"]).to_bits(),
                og,
                hc
            ]),
            "flying" => serde_json::json!(["flying", og, hc]),
            "vehicle_move" => serde_json::json!([
                "vehicle_move",
                bits(v3(&p["pos"])),
                f32v(&p["yaw"]).to_bits(),
                f32v(&p["pitch"]).to_bits(),
                og
            ]),
            "steer_boat" => serde_json::json!(["steer_boat", p["left"], p["right"]]),
            "entity_action" => serde_json::json!(["entity_action", p["action_name"]]),
            "player_input" => {
                let i = &p["input"];
                serde_json::json!([
                    "player_input",
                    [
                        i["forward"],
                        i["backward"],
                        i["left"],
                        i["right"],
                        i["jump"],
                        i["shift"],
                        i["sprint"]
                    ]
                ])
            }
            _ => return None,
        })
    };
    struct Keys(Controls);
    impl Behavior for Keys {
        fn name(&self) -> &str {
            "entities-replay"
        }
        fn tick(&mut self, ctx: &mut TickContext<'_>) {
            ctx.set_controls(self.0);
        }
    }
    let spawn = |e: &mut Engine<Box<dyn Sim>>, ev: &Value| {
        let name = ev["entity"].as_str().unwrap();
        let ty = data::entity_by_name(name).unwrap().id;
        let [x, y, z] = v3(&ev["pos"]);
        let id = ev["id"].as_i64().unwrap() as i32;
        let raw = RawPacket {
            id: cb::id::SPAWN_ENTITY,
            body: body(&cb::SpawnEntity {
                entity_id: id,
                object_uuid: uuid::Uuid::from_u64_pair(0x656e_7469, id as u64),
                r#type: i32::from(ty),
                x,
                y,
                z,
                velocity: ghast_protocol::types::LpVec3::ZERO,
                pitch: 0,
                yaw: (f32v(&ev["yaw"]) * 256.0 / 360.0).floor() as i32 as i8,
                head_pitch: 0,
                object_data: 0,
            }),
        };
        e.sim.world_packet(&raw).unwrap();
    };
    let wanted = hand;
    let (mut cases, mut fuzz_cases, mut ticks_done, mut packets) = (0, 0, 0, 0);
    let mut failed = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let sc = &lines[i];
        let name = sc["name"].as_str().unwrap().to_owned();
        let mut ticks = Vec::new();
        i += 1;
        while i < lines.len() && lines[i]["type"] == "tick" {
            ticks.push(&lines[i]);
            i += 1;
        }
        // Fuzz cases whose events this replay drives (entities only), plain setup.
        let plain = ticks.iter().all(|t| {
            t["input"]["events"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|ev| {
                    matches!(
                        ev["type"].as_str(),
                        Some(
                            "spawn"
                                | "entity_move"
                                | "remove_entity"
                                | "border_center"
                                | "border_size"
                                | "border_lerp"
                                | "velocity"
                                | "ride"
                                | "vehicle_move"
                        )
                    )
                })
        }) && sc["setup"]["ext"].as_object().is_none_or(|x| {
            x.keys()
                .all(|k| matches!(k.as_str(), "entities" | "border" | "dimension"))
        });
        // Without the tracker only cases where the ridden boat is the only entity.
        let alone = tracking
            || (sc["setup"]["ext"]["entities"]
                .as_array()
                .is_some_and(|l| l.len() <= 1)
                && ticks.iter().all(|t| {
                    t["input"]["events"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .all(|ev| ev["type"] != "spawn")
                }));
        let fuzz = name.starts_with("fuzz") && plain && alone;
        if !wanted.contains(&name.as_str()) && !fuzz {
            continue;
        }
        cases += 1;
        fuzz_cases += usize::from(fuzz);
        let setup = &sc["setup"];
        let mut o = opts(GAME);
        o.inventory = INVENTORY;
        let nether = setup["ext"]["dimension"] == "the_nether";
        let sim = if nether {
            ModernSim::test_nether()
        } else {
            ModernSim::test_overworld()
        };
        let mut e: Engine<Box<dyn Sim>> = Engine::new(o, Box::new(sim));
        e.handle(Inbound::JoinGame(JoinGame {
            entity_id: 7,
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
        e.sim.test_set_own_id(7);
        e.sim.set_entities(tracking.then(|| {
            ghast_entity::Config::default()
                .with_metadata(true)
                .with_equipment(false)
                .with_effects(false)
                .with_events(false)
        }));
        // The chunk window around the spawn (far cases).
        let (cx, cz) = (
            (f64v(&setup["pos"][0]).floor() as i32) >> 4,
            (f64v(&setup["pos"][2]).floor() as i32) >> 4,
        );
        e.sim
            .world_packet(&RawPacket {
                id: cb::id::UPDATE_VIEW_POSITION,
                body: body(&cb::UpdateViewPosition {
                    chunk_x: cx,
                    chunk_z: cz,
                }),
            })
            .unwrap();
        for r in sc["loaded_chunks"].as_array().unwrap() {
            let r: Vec<i32> = (0..4).map(|k| r[k].as_i64().unwrap() as i32).collect();
            for x in r[0]..=r[2] {
                for z in r[1]..=r[3] {
                    let raw = if nether {
                        chunk_sections(x, z, 16, 3)
                    } else {
                        chunk(x, z)
                    };
                    e.sim.world_packet(&raw).unwrap();
                }
            }
        }
        // The test chunks have a stone layer at y 63; the oracle world is air outside the
        // fills, which come in order (a later fill overwrites an earlier one).
        {
            for r in sc["loaded_chunks"].as_array().unwrap() {
                let r: Vec<i32> = (0..4).map(|k| r[k].as_i64().unwrap() as i32).collect();
                for x in r[0] * 16..r[2] * 16 + 16 {
                    for z in r[1] * 16..r[3] * 16 + 16 {
                        e.sim.predict_air(x, 63, z);
                    }
                }
            }
        }
        for fill in sc["fills"].as_array().unwrap() {
            let v: Vec<i64> = (0..7).map(|k| fill[k].as_i64().unwrap()).collect();
            // The fill's value is the raw 26.1 state id.
            let id = v[6] as u16;
            for x in v[0]..=v[3] {
                for y in v[1]..=v[4] {
                    for z in v[2]..=v[5] {
                        let pos = ((x & 0x3FF_FFFF) << 38) | ((z & 0x3FF_FFFF) << 12) | (y & 0xFFF);
                        let mut b = pos.to_be_bytes().to_vec();
                        let mut st = u32::from(id);
                        loop {
                            let byte = (st & 0x7f) as u8;
                            st >>= 7;
                            if st == 0 {
                                b.push(byte);
                                break;
                            }
                            b.push(byte | 0x80);
                        }
                        e.sim
                            .world_packet(&RawPacket {
                                id: cb::id::BLOCK_CHANGE,
                                body: bytes::Bytes::from(b),
                            })
                            .unwrap();
                    }
                }
            }
        }
        e.handle(Inbound::Teleport(Teleport {
            x: f64v(&setup["pos"][0]),
            y: f64v(&setup["pos"][1]),
            z: f64v(&setup["pos"][2]),
            yaw: f32v(&setup["yaw"]),
            pitch: f32v(&setup["pitch"]),
            teleport_id: None,
            ..Default::default()
        }));
        let p = player_of(sc, ticks[0]);
        e.sim.test_set_modern(p, ext_of(sc, ticks[0]));
        // Spawned after the player: they tick after it.
        for ev in setup["ext"]["entities"].as_array().into_iter().flatten() {
            spawn(&mut e, ev);
        }
        e.core.pose.pos = Vec3::new(p.pos[0], p.pos[1], p.pos[2]);
        e.core.pose.yaw = p.yaw;
        e.core.pose.pitch = p.pitch;
        e.core.level_load_started = true;
        e.core.client_loaded = true;
        e.core.ready = true;
        e.outbound().clear();
        let t0 = Instant::now();
        for (k, rec) in ticks.iter().enumerate().skip(1) {
            let input = &rec["input"];
            for ev in input["events"].as_array().into_iter().flatten() {
                match ev["type"].as_str().unwrap() {
                    "spawn" => spawn(&mut e, ev),
                    "entity_move" => {
                        let [x, y, z] = v3(&ev["pos"]);
                        let raw = RawPacket {
                            id: cb::id::ENTITY_TELEPORT,
                            body: body(&cb::EntityTeleport {
                                entity_id: ev["id"].as_i64().unwrap() as i32,
                                x,
                                y,
                                z,
                                dx: 0.0,
                                dy: 0.0,
                                dz: 0.0,
                                yaw: f32v(&ev["yaw"]),
                                pitch: 0.0,
                                flags: cb::PositionUpdateRelatives(0),
                                on_ground: ev["on_ground"].as_bool().unwrap(),
                            }),
                        };
                        e.sim.world_packet(&raw).unwrap();
                    }
                    "border_center" => {
                        let raw = RawPacket {
                            id: cb::id::WORLD_BORDER_CENTER,
                            body: body(&cb::WorldBorderCenter {
                                x: f64v(&ev["x"]),
                                z: f64v(&ev["z"]),
                            }),
                        };
                        e.sim.world_packet(&raw).unwrap();
                    }
                    "border_size" => {
                        let raw = RawPacket {
                            id: cb::id::WORLD_BORDER_SIZE,
                            body: body(&cb::WorldBorderSize {
                                diameter: f64v(&ev["size"]),
                            }),
                        };
                        e.sim.world_packet(&raw).unwrap();
                    }
                    "border_lerp" => {
                        let raw = RawPacket {
                            id: cb::id::WORLD_BORDER_LERP_SIZE,
                            body: body(&cb::WorldBorderLerpSize {
                                old_diameter: f64v(&ev["from"]),
                                new_diameter: f64v(&ev["to"]),
                                speed: ev["ticks"].as_i64().unwrap(),
                            }),
                        };
                        e.sim.world_packet(&raw).unwrap();
                    }
                    "velocity" => {
                        // set_entity_motion: 1/8000 units through the LpVec3 codec.
                        let c = |k: &str| ev[k].as_f64().unwrap() / 8000.0;
                        let [x, y, z] = golden::lpvec3([c("x"), c("y"), c("z")]);
                        e.handle(Inbound::Velocity {
                            entity_id: 7,
                            motion: Vec3::new(x, y, z),
                        });
                    }
                    "ride" => {
                        // The oracle's player is entity 1, ours 7.
                        let passengers = ev["passengers"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| match v.as_i64().unwrap() as i32 {
                                1 => 7,
                                x => x,
                            })
                            .collect();
                        let raw = RawPacket {
                            id: cb::id::SET_PASSENGERS,
                            body: body(&cb::SetPassengers {
                                entity_id: ev["vehicle"].as_i64().unwrap() as i32,
                                passengers,
                            }),
                        };
                        e.sim.world_packet(&raw).unwrap();
                    }
                    "vehicle_move" => {
                        let [x, y, z] = v3(&ev["pos"]);
                        let raw = RawPacket {
                            id: cb::id::VEHICLE_MOVE,
                            body: body(&cb::VehicleMove {
                                x,
                                y,
                                z,
                                yaw: f32v(&ev["yaw"]),
                                pitch: f32v(&ev["pitch"]),
                            }),
                        };
                        e.sim.world_packet(&raw).unwrap();
                        // The client sends handleMoveVehicle's answer at once.
                        if let Some(o) = e.sim.take_reply() {
                            e.outbound().push(o);
                        }
                    }
                    "remove_entity" => {
                        let raw = RawPacket {
                            id: cb::id::ENTITY_DESTROY,
                            body: body(&cb::EntityDestroy {
                                entity_ids: vec![ev["id"].as_i64().unwrap() as i32],
                            }),
                        };
                        e.sim.world_packet(&raw).unwrap();
                    }
                    other => panic!("{name}: event {other}"),
                }
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
            let got: Vec<Value> = out.iter().filter_map(&key_name).collect();
            let want: Vec<Value> = rec["packets"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(&golden)
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
        "{file} through the engine: {cases} cases ({fuzz_cases} fuzz), {ticks_done} ticks, {packets} packets identical"
    );
    assert!(
        failed.is_empty(),
        "{file}: {} failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
    assert_eq!(cases - fuzz_cases, wanted.len());
    (cases, fuzz_cases)
}

/// 26.1 item use from the stack's components (`item_use`): which items have a use state,
/// the effects fed to the movement while using, the stop when the item changes.
#[test]
fn item_use_facts() {
    use ghast_inventory::{Inventory, Stack};
    let pre_tick = |using| crate::sim::PreTick {
        using,
        released: None,
        controls: Controls::NONE,
        look: (0.0, 0.0),
    };
    let id = |n| inv::item_id(n).unwrap();
    let mut inv = Inventory::new(INVENTORY).unwrap();
    let mut sim = ModernSim::test_overworld();
    let facts = |inv: &Inventory, sim: &ModernSim| held_item(inv, sim).unwrap();
    // A stick has no use state; bread is eaten below a full bar; a shield blocks.
    inv.on_set_slot(0, 36, Some(Stack::new(id("stick"), 1, 0)));
    assert!(!facts(&inv, &sim).usable);
    inv.on_set_slot(0, 36, Some(Stack::new(id("bread"), 3, 0)));
    let f = facts(&inv, &sim);
    assert!(f.usable && f.food && !f.always_edible);
    inv.on_set_slot(0, 36, Some(Stack::new(id("golden_apple"), 1, 0)));
    assert!(facts(&inv, &sim).always_edible);
    inv.on_set_slot(0, 36, Some(Stack::new(id("shield"), 1, 0)));
    let shield = u16::try_from(id("shield")).unwrap();
    assert!(facts(&inv, &sim).usable && !facts(&inv, &sim).food);
    // While using: the stack's use_effects (vanilla default: no sprint, x0.2) ...
    assert!(sim.pre_tick(&inv, &pre_tick(Some((false, shield)))));
    assert_eq!(sim.use_effects_now(), Some((false, 0.2)));
    assert!(sim.pre_tick(&inv, &pre_tick(None)));
    assert_eq!(sim.use_effects_now(), None);
    // ... and the use stops when the hand no longer holds that item.
    inv.on_set_slot(0, 36, Some(Stack::new(id("stick"), 1, 0)));
    assert!(!sim.pre_tick(&inv, &pre_tick(Some((false, shield)))));
    assert!(!sim.pre_tick(&inv, &pre_tick(Some((true, shield)))));
}

/// The same through the version's tracker and `pick_target` (entity range 3,
/// block range 4.5, no pick margin).
#[test]
fn entity_pick() {
    use proto::play::clientbound::id;
    use std::cell::Cell;
    let raw = |id: i32, b: Vec<u8>| RawPacket {
        id,
        body: bytes::Bytes::from(b),
    };
    let zombie = |eid: u8, x: f64| {
        // entity id, uuid, type (zombie, var-int), x y z, zero velocity, pitch yaw head, data.
        let mut b = vec![eid];
        b.extend((0x1000u128 + u128::from(eid)).to_be_bytes());
        let ty = data::entity_by_name("zombie").unwrap().id;
        b.extend([(ty & 0x7f) as u8 | 0x80, (ty >> 7) as u8]);
        for v in [x, 64.0, 8.5] {
            b.extend(v.to_be_bytes());
        }
        b.extend([0, 0, 0, 0, 0]);
        raw(id::SPAWN_ENTITY, b)
    };
    let mut e = engine(GAME);
    join(&mut e);
    e.sim.set_entities(Some(ghast_entity::Config::default()));
    let pos: i64 = (4 << 38) | (8 << 12) | 65;
    let mut b = pos.to_be_bytes().to_vec();
    b.push(1);
    e.sim.world_packet(&raw(id::BLOCK_CHANGE, b)).unwrap();
    let t0 = Instant::now();
    e.note_inbound(t0);
    let seen = Cell::new((None, None));
    let look = |e: &mut Engine<Box<dyn Sim>>| {
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            seen.set((
                ctx.target_entity().map(|h| (h.id, h.distance)),
                ctx.target().map(|t| (t.pos, t.face)),
            ));
        });
        e.tick(&mut b, t0);
        seen.get()
    };
    assert_eq!(
        look(&mut e),
        (
            None,
            Some((ghast_protocol::BlockPos::new(4, 65, 8), crate::Face::East))
        )
    );
    e.sim.world_packet(&zombie(50, 6.5)).unwrap();
    let (ent, block) = look(&mut e);
    let (eid, d) = ent.unwrap();
    assert_eq!(eid, 50);
    assert!(
        (d - (8.5 - 6.5 - f64::from(0.6f32 / 2.0))).abs() < 1e-6,
        "{d}"
    );
    assert_eq!(block, None);
    e.sim
        .world_packet(&raw(id::ENTITY_DESTROY, vec![1, 50]))
        .unwrap();
    e.sim.world_packet(&zombie(51, 5.0)).unwrap();
    assert_eq!(look(&mut e), (None, None));
}

/// 1.21.4+ LevelLoadTracker: `player_loaded` waits for the player's column and its eight
/// neighbours (vanilla's first compile of the player's section needs them) and goes out a
/// tick after they are all there, not before.
#[test]
fn player_loaded_waits_for_the_neighbourhood() {
    let mut e = engine(GAME);
    e.handle(Inbound::JoinGame(JoinGame {
        entity_id: 7,
        game_mode: GameMode::Survival,
        dimension: "0".into(),
        hardcore: false,
        dimension_nbt: None,
        respawn_screen: true,
    }));
    e.handle(Inbound::Teleport(Teleport {
        x: 8.5,
        y: 70.0,
        z: 8.5,
        teleport_id: Some(1),
        ..Default::default()
    }));
    e.handle(Inbound::GameState {
        reason: 13,
        value: 0.0,
    });
    load(&mut e, 0, 0);
    let t0 = Instant::now();
    let mut b = Idle;
    let mut loaded_at = None;
    for t in 0..8 {
        if t == 3 {
            for (x, z) in [(-1, -1), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0)] {
                load(&mut e, x, z);
            }
        }
        if t == 5 {
            load(&mut e, 1, 1);
        }
        e.note_inbound(t0);
        e.tick(&mut b, t0);
        if std::mem::take(e.outbound()).contains(&Outbound::PlayerLoaded) {
            loaded_at.get_or_insert(t);
        }
    }
    // All nine from tick 5 on: loaded on tick 6.
    assert_eq!(loaded_at, Some(6));
}

/// The engine through the oracle's 26.1 joins (`tests/golden/<version>/joins.jsonl.gz`, the real
/// LevelLoadTracker): every packet of every tick identical (frozen before the gate, position
/// packets answered while loading, gate tick player_loaded → position_look on-ground false →
/// tick_end, the on-ground status packet after it, water, timeout, compiled before the load
/// start, never loaded). The gate's "section compiled" and "timed out" inputs come from the
/// golden (the engine's own signal: the nine columns on two consecutive ticks, tested in
/// `player_loaded_waits_for_the_neighbourhood`).
#[test]
fn joins_match_the_oracle() {
    use serde_json::Value;
    use std::io::BufRead;
    let path = format!(
        "{}/../../tests/golden/{GOLDEN}/joins.jsonl.gz",
        env!("CARGO_MANIFEST_DIR")
    );
    let f = std::fs::File::open(path).unwrap();
    let lines: Vec<Value> = std::io::BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    let f64v = |v: &Value| f64::from_bits(u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap());
    let f32v = |v: &Value| f32::from_bits(u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap());
    let v3 = |v: &Value| Vec3::new(f64v(&v[0]), f64v(&v[1]), f64v(&v[2]));
    // Engine packets as the golden names them (others are not in the golden).
    let wire = |o: &Outbound| -> Option<Value> {
        let bits = |p: Vec3| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()];
        Some(match *o {
            Outbound::TeleportConfirm(id) => serde_json::json!(["teleport_confirm", id]),
            Outbound::PlayerLoaded => serde_json::json!(["player_loaded"]),
            Outbound::ChatSessionUpdate => serde_json::json!(["chat_session_update"]),
            Outbound::TickEnd => serde_json::json!(["tick_end"]),
            Outbound::Move(MovePacket::PositionLook {
                pos,
                yaw,
                pitch,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!([
                    "position_look",
                    bits(pos),
                    yaw.to_bits(),
                    pitch.to_bits(),
                    on_ground,
                    horizontal_collision
                ])
            }
            Outbound::Move(MovePacket::Position {
                pos,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!(["position", bits(pos), on_ground, horizontal_collision])
            }
            Outbound::Move(MovePacket::Look {
                yaw,
                pitch,
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!([
                    "look",
                    yaw.to_bits(),
                    pitch.to_bits(),
                    on_ground,
                    horizontal_collision
                ])
            }
            Outbound::Move(MovePacket::Flying {
                on_ground,
                horizontal_collision,
            }) => {
                serde_json::json!(["flying", on_ground, horizontal_collision])
            }
            _ => return None,
        })
    };
    let golden = |p: &Value| -> Value {
        let b = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
        let og = &p["on_ground"];
        let hc = &p["horizontal_collision"];
        match p["name"].as_str().unwrap() {
            "teleport_confirm" => serde_json::json!(["teleport_confirm", p["teleport_id"]]),
            "position_look" => serde_json::json!([
                "position_look",
                b(v3(&p["pos"])),
                f32v(&p["yaw"]).to_bits(),
                f32v(&p["pitch"]).to_bits(),
                og,
                hc
            ]),
            "position" => serde_json::json!(["position", b(v3(&p["pos"])), og, hc]),
            "look" => serde_json::json!([
                "look",
                f32v(&p["yaw"]).to_bits(),
                f32v(&p["pitch"]).to_bits(),
                og,
                hc
            ]),
            "flying" => serde_json::json!(["flying", og, hc]),
            n => serde_json::json!([n]),
        }
    };
    let water = data::BlockState::parse("water[level=0]").unwrap().id();
    let mut cases = 0;
    let mut i = 0;
    while i < lines.len() {
        let sc = &lines[i];
        assert_eq!(sc["type"], "scenario");
        let name = sc["name"].as_str().unwrap().to_owned();
        let mut ticks = Vec::new();
        i += 1;
        while i < lines.len() && lines[i]["type"] == "tick" {
            ticks.push(&lines[i]);
            i += 1;
        }
        cases += 1;
        let setup = &sc["setup"];
        assert_eq!(setup["game_mode"], "survival", "{name}");
        let health = setup.get("health").map_or(20.0, f32v);
        let mut e = engine(GAME);
        e.handle(Inbound::JoinGame(JoinGame {
            entity_id: 7,
            game_mode: GameMode::Survival,
            dimension: "0".into(),
            hardcore: false,
            dimension_nbt: None,
            respawn_screen: true,
        }));
        let r = &sc["loaded_chunks"][0];
        let r: Vec<i32> = (0..4).map(|k| r[k].as_i64().unwrap() as i32).collect();
        for x in r[0]..=r[2] {
            for z in r[1]..=r[3] {
                load(&mut e, x, z);
            }
        }
        // The stone layer y 63 is the test chunks' own; other fills as block changes.
        for fill in sc["fills"].as_array().unwrap() {
            let v: Vec<i64> = (0..7).map(|k| fill[k].as_i64().unwrap()).collect();
            let state = sc["states"][v[6].to_string()].as_str().unwrap();
            if state == "minecraft:stone" {
                assert_eq!((v[1], v[4]), (63, 63), "{name}");
                continue;
            }
            assert_eq!(state, "minecraft:water[level=0]", "{name}");
            for x in v[0]..=v[3] {
                for y in v[1]..=v[4] {
                    for z in v[2]..=v[5] {
                        let pos = ((x & 0x3FF_FFFF) << 38) | ((z & 0x3FF_FFFF) << 12) | (y & 0xFFF);
                        let mut b = pos.to_be_bytes().to_vec();
                        let mut st = u32::from(water);
                        loop {
                            let byte = (st & 0x7f) as u8;
                            st >>= 7;
                            if st == 0 {
                                b.push(byte);
                                break;
                            }
                            b.push(byte | 0x80);
                        }
                        let raw = RawPacket {
                            id: proto::play::clientbound::id::BLOCK_CHANGE,
                            body: bytes::Bytes::from(b),
                        };
                        e.sim.world_packet(&raw).unwrap();
                    }
                }
            }
        }
        e.handle(Inbound::Health(Health {
            health,
            food: 20,
            saturation: 5.0,
        }));
        e.outbound().clear();
        let mut compiled = false;
        let mut timed_out = false;
        let t0 = Instant::now();
        let mut b = Idle;
        let mut teleport_id = 0;
        for (k, rec) in ticks.iter().enumerate() {
            let want: Vec<Value> = rec["packets"]
                .as_array()
                .unwrap()
                .iter()
                .map(&golden)
                .collect();
            let confirm = rec["packets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == "teleport_confirm")
                .map(|p| p["teleport_id"].as_i64().unwrap() as i32);
            if k == 0 {
                e.handle(Inbound::Teleport(Teleport {
                    x: f64v(&setup["pos"][0]),
                    y: f64v(&setup["pos"][1]),
                    z: f64v(&setup["pos"][2]),
                    yaw: f32v(&setup["yaw"]),
                    pitch: f32v(&setup["pitch"]),
                    flags: 0,
                    teleport_id: Some(confirm.unwrap_or(teleport_id)),
                    ..Default::default()
                }));
            } else {
                let input = &rec["input"];
                assert_eq!(input["keys"], "", "{name}");
                for ev in input["events"].as_array().into_iter().flatten() {
                    match ev["type"].as_str().unwrap() {
                        "level_chunks_load_start" => e.handle(Inbound::GameState {
                            reason: 13,
                            value: 0.0,
                        }),
                        "section_compiled" => compiled = ev["compiled"].as_bool().unwrap(),
                        "load_timeout" => timed_out = true,
                        "teleport" => {
                            teleport_id += 1;
                            let p = v3(&ev["pos"]);
                            e.handle(Inbound::Teleport(Teleport {
                                x: p.x,
                                y: p.y,
                                z: p.z,
                                yaw: f32v(&ev["yaw"]),
                                pitch: f32v(&ev["pitch"]),
                                flags: 0,
                                teleport_id: Some(confirm.unwrap_or(teleport_id)),
                                ..Default::default()
                            }));
                        }
                        other => panic!("{name}: event {other}"),
                    }
                }
                e.core.test_gate = Some((compiled, timed_out));
                // The oracle's mouse: the input look, from the tick after the gate on.
                if e.core.client_loaded {
                    e.core.forced_look = Some((f32v(&input["yaw"]), f32v(&input["pitch"])));
                }
                e.note_inbound(t0);
                e.tick(&mut b, t0);
            }
            let got: Vec<Value> = std::mem::take(e.outbound())
                .iter()
                .filter_map(&wire)
                .collect();
            assert_eq!(got, want, "{name} tick {k}");
            assert_eq!(
                e.core.client_loaded,
                rec["client_loaded"].as_bool().unwrap(),
                "{name} tick {k}"
            );
        }
    }
    assert_eq!(cases, 27);
}
