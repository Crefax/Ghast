//! The player side of attacks and the movement keys on every version: the attack slow-down
//! (`PlayerEntity.attack`: velocity x/z × 0.6 and `setSprinting(false)` on a hit with
//! knockback), our own entity data's flags byte, the dead player's and an open screen's
//! released keys, the server velocity before the step and the step's collision flags.

use super::tests::{Script, engine, join, names};
use super::*;
use crate::common::EntityAction;
use bytes::BufMut;
use ghast_protocol::RawPacket;
use ghast_protocol::codec::McWrite;

/// Every version the client engine runs.
const VERSIONS: [GameVersion; 5] = [
    GameVersion::V1_8_9,
    GameVersion::V1_12_2,
    GameVersion::V1_16_5,
    GameVersion::V26_1,
    GameVersion::V26_2,
];

/// What an attack target is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Player,
    Zombie,
}

fn uuid(id: i32) -> uuid::Uuid {
    uuid::Uuid::from_u128(0x7465_7374_0000 + id as u128)
}

/// Spawns a player or a zombie at `pos`, facing south (26.x: a player's tab-list entry first,
/// the client adds no player without one).
fn put(e: &mut Engine<Box<dyn Sim>>, id: i32, kind: Kind, pos: Vec3) {
    let v = e.core.opts.caps.version;
    if kind == Kind::Player && v.caps().configuration_state {
        // player_info: actions {add player}, one entry (uuid, name, no properties).
        let mut b = bytes::BytesMut::new();
        b.put_u8(1);
        b.put_varint(1);
        b.put_u128(uuid(id).as_u128());
        b.put_varint(6);
        b.put_slice(b"Target");
        b.put_varint(0);
        let pid = if v == GameVersion::V26_1 {
            ghast_protocol::v26_1::play::clientbound::id::PLAYER_INFO
        } else {
            ghast_protocol::v26_2::play::clientbound::id::PLAYER_INFO
        };
        e.sim
            .world_packet(&RawPacket::new(pid, b.freeze()))
            .unwrap();
    }
    e.sim.world_packet(&spawn(v, id, kind, pos)).unwrap();
}

/// A stone floor at y 63 under x −15 … 15, z 7 … 9 (1.16.5's test columns are empty).
fn floor(e: &mut Engine<Box<dyn Sim>>) {
    if e.core.opts.caps.version != GameVersion::V1_16_5 {
        return;
    }
    let id = ghast_protocol::v1_16_5::play::clientbound::id::BLOCK_CHANGE;
    for x in -15i64..=15 {
        for z in 7i64..=9 {
            let pos = ((x & 0x3ff_ffff) << 38) | ((z & 0x3ff_ffff) << 12) | 63;
            let mut b = bytes::BytesMut::new();
            b.put_i64(pos);
            b.put_varint(1);
            e.sim.world_packet(&RawPacket::new(id, b.freeze())).unwrap();
        }
    }
}

/// The version's spawn packet of a player or a zombie at `pos`, facing south.
fn spawn(v: GameVersion, id: i32, kind: Kind, pos: Vec3) -> RawPacket {
    let fixed = |d: f64| (d * 32.0).floor() as i32;
    let vel = ghast_protocol::defs::p47::types::Vec3i16 { x: 0, y: 0, z: 0 };
    match v.instance() {
        GameVersion::V1_8_9 => {
            use ghast_protocol::v1_8::play::clientbound as cb;
            match kind {
                Kind::Player => RawPacket::from_packet(&cb::NamedEntitySpawn {
                    entity_id: id,
                    player_uuid: uuid(id),
                    x: fixed(pos.x),
                    y: fixed(pos.y),
                    z: fixed(pos.z),
                    yaw: 0,
                    pitch: 0,
                    current_item: 0,
                    metadata: Vec::new(),
                }),
                Kind::Zombie => RawPacket::from_packet(&cb::SpawnEntityLiving {
                    entity_id: id,
                    r#type: 54,
                    x: fixed(pos.x),
                    y: fixed(pos.y),
                    z: fixed(pos.z),
                    yaw: 0,
                    pitch: 0,
                    head_pitch: 0,
                    velocity: vel,
                    metadata: Vec::new(),
                }),
            }
        }
        GameVersion::V1_12_2 => {
            use ghast_protocol::v1_12_2::play::clientbound as cb;
            match kind {
                Kind::Player => RawPacket::from_packet(&cb::NamedEntitySpawn {
                    entity_id: id,
                    player_uuid: uuid(id),
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    yaw: 0,
                    pitch: 0,
                    metadata: Vec::new(),
                }),
                Kind::Zombie => RawPacket::from_packet(&cb::SpawnEntityLiving {
                    entity_id: id,
                    entity_uuid: uuid(id),
                    r#type: 54,
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    yaw: 0,
                    pitch: 0,
                    head_pitch: 0,
                    velocity: vel,
                    metadata: Vec::new(),
                }),
            }
        }
        GameVersion::V1_16_5 => {
            use ghast_protocol::v1_16_5::play::clientbound as cb;
            match kind {
                Kind::Player => RawPacket::from_packet(&cb::NamedEntitySpawn {
                    entity_id: id,
                    player_uuid: uuid(id),
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    yaw: 0,
                    pitch: 0,
                }),
                Kind::Zombie => RawPacket::from_packet(&cb::SpawnEntityLiving {
                    entity_id: id,
                    entity_uuid: uuid(id),
                    r#type: ghast_data::v1_16_5::entities()
                        .iter()
                        .find(|e| e.name == "zombie")
                        .map(|e| i32::from(e.id))
                        .unwrap(),
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    yaw: 0,
                    pitch: 0,
                    head_pitch: 0,
                    velocity: vel,
                }),
            }
        }
        modern => {
            let name = match kind {
                Kind::Player => "player",
                Kind::Zombie => "zombie",
            };
            let (ty, id_spawn) = if modern == GameVersion::V26_1 {
                (
                    ghast_data::v26_1::entity_by_name(name).unwrap().id,
                    ghast_protocol::v26_1::play::clientbound::id::SPAWN_ENTITY,
                )
            } else {
                (
                    ghast_data::v26_2::entity_by_name(name).unwrap().id,
                    ghast_protocol::v26_2::play::clientbound::id::SPAWN_ENTITY,
                )
            };
            let mut b = bytes::BytesMut::new();
            b.put_varint(id);
            b.put_u128(uuid(id).as_u128());
            b.put_varint(i32::from(ty));
            for c in [pos.x, pos.y, pos.z] {
                b.put_f64(c);
            }
            b.put_u8(0);
            b.put_i8(0);
            b.put_i8(0);
            b.put_i8(0);
            b.put_varint(0);
            return RawPacket::new(id_spawn, b.freeze());
        }
    }
    .unwrap()
}

/// Our entity data with only the flags byte (index 0).
fn own_flags(v: GameVersion, flags: u8) -> RawPacket {
    let mut b = bytes::BytesMut::new();
    b.put_varint(7);
    let id = match v.instance() {
        GameVersion::V1_8_9 => {
            // type 0 (byte) << 5 | index 0, the byte, the end marker.
            b.put_u8(0);
            b.put_u8(flags);
            b.put_u8(0x7f);
            return RawPacket::new(
                ghast_protocol::v1_8::play::clientbound::id::ENTITY_METADATA,
                b.freeze(),
            );
        }
        GameVersion::V1_12_2 => ghast_protocol::v1_12_2::play::clientbound::id::ENTITY_METADATA,
        GameVersion::V1_16_5 => ghast_protocol::v1_16_5::play::clientbound::id::ENTITY_METADATA,
        GameVersion::V26_1 => ghast_protocol::v26_1::play::clientbound::id::ENTITY_METADATA,
        _ => ghast_protocol::v26_2::play::clientbound::id::ENTITY_METADATA,
    };
    // index 0, type 0 (byte), the byte, the end marker.
    b.put_u8(0);
    b.put_u8(0);
    b.put_u8(flags);
    b.put_u8(0xff);
    RawPacket::new(id, b.freeze())
}

/// A joined engine with entity tracking, full food (sprinting needs it) and our entity id
/// known to the sim, standing at (8.5, 64, 8.5) facing west.
fn setup(v: GameVersion) -> Engine<Box<dyn Sim>> {
    let mut e = engine(v);
    join(&mut e);
    floor(&mut e);
    e.sim.test_set_own_id(7);
    e.sim.set_entities(Some(ghast_entity::Config::default()));
    e.handle(Inbound::Health(Health {
        health: 20.0,
        food: 20,
        saturation: 5.0,
    }));
    e
}

/// One tick with a behaviour closure; returns the tick's packet names.
fn tick(
    e: &mut Engine<Box<dyn Sim>>,
    t0: Instant,
    f: impl FnMut(&mut TickContext<'_>),
) -> Vec<&'static str> {
    e.note_inbound(t0);
    let mut b = Script(f);
    e.tick(&mut b, t0);
    names(e.outbound())
}

/// The tick's sprint actions.
fn sprint_actions(out: &[Outbound]) -> Vec<EntityAction> {
    out.iter()
        .filter_map(|o| match o {
            Outbound::EntityAction { action, .. }
                if matches!(
                    action,
                    EntityAction::StartSprinting | EntityAction::StopSprinting
                ) =>
            {
                Some(*action)
            }
            _ => None,
        })
        .collect()
}

const RUN: Controls = Controls {
    forward: true,
    sprint: true,
    ..Controls::NONE
};
const WALK: Controls = Controls {
    forward: true,
    ..Controls::NONE
};

/// Sprints west for 8 ticks, then walks on with the sprint key up (still sprinting), the
/// target 1.5 blocks ahead of the sent position; returns the engine just before the attack
/// tick.
fn sprinting_at(v: GameVersion, kind: Kind, t0: Instant) -> Engine<Box<dyn Sim>> {
    let mut e = setup(v);
    for _ in 0..8 {
        tick(&mut e, t0, |ctx| ctx.set_controls(RUN));
    }
    tick(&mut e, t0, |ctx| ctx.set_controls(WALK));
    assert!(e.core.pose.sprinting, "{v}: sprinting");
    let s = e.core.sent.pos;
    put(&mut e, 50, kind, Vec3::new(s.x - 1.5, s.y, s.z));
    e
}

/// The attack tick: `attack` decides whether the behaviour attacks entity 50. Returns the
/// tick's sprint actions, the motion after the step and the attack's result.
fn attack_tick(
    e: &mut Engine<Box<dyn Sim>>,
    t0: Instant,
    controls: Controls,
    attack: bool,
) -> (Vec<EntityAction>, Vec3, bool) {
    let mut hit = false;
    e.note_inbound(t0);
    let mut b = Script(|ctx: &mut TickContext<'_>| {
        ctx.set_controls(controls);
        if attack {
            hit = ctx.attack(50);
        }
    });
    e.tick(&mut b, t0);
    let actions = sprint_actions(e.outbound());
    e.outbound().clear();
    (actions, e.core.pose.motion, hit)
}

/// A sprint hit on a player with the sprint key up: the attack unsprints (a stop goes out)
/// and slows; on a zombie (not hurt on the client) nothing happens. Every version.
#[test]
fn sprint_hit_slows_and_unsprints_on_players_only() {
    let t0 = Instant::now();
    for v in VERSIONS {
        let mut e = sprinting_at(v, Kind::Player, t0);
        let (actions, _, hit) = attack_tick(&mut e, t0, WALK, true);
        assert!(hit, "{v}: the attack went out");
        assert_eq!(actions, [EntityAction::StopSprinting], "{v}");
        assert!(!e.core.pose.sprinting, "{v}");

        let mut e = sprinting_at(v, Kind::Zombie, t0);
        let (actions, _, hit) = attack_tick(&mut e, t0, WALK, true);
        assert!(hit, "{v}: the attack went out");
        assert!(actions.is_empty(), "{v}: {actions:?}");
        assert!(e.core.pose.sprinting, "{v}");
    }
}

/// With the sprint key held the sprint starts again in the same movement update (no packet)
/// and the step starts from 0.6 × the horizontal velocity: the motion differs from the same
/// tick without the attack by exactly −0.4 × v × the ground friction (0.6 × 0.91).
#[test]
fn sprint_hit_keeps_the_held_sprint_and_slows_by_0_6() {
    let t0 = Instant::now();
    for v in VERSIONS {
        let mut a = sprinting_at(v, Kind::Player, t0);
        let mut b = sprinting_at(v, Kind::Player, t0);
        let v0 = a.core.pose.motion;
        assert_eq!(v0, b.core.pose.motion, "{v}");
        assert!(v0.x < -0.1, "{v}: running west {v0:?}");
        let (_, plain, _) = attack_tick(&mut a, t0, RUN, false);
        let (actions, slowed, hit) = attack_tick(&mut b, t0, RUN, true);
        assert!(hit, "{v}");
        assert!(actions.is_empty(), "{v}: {actions:?}");
        assert!(b.core.pose.sprinting, "{v}");
        let f = f64::from(0.6f32 * 0.91);
        for (p, s, v0) in [(plain.x, slowed.x, v0.x), (plain.z, slowed.z, v0.z)] {
            let want = p - 0.4 * v0 * f;
            assert!(
                (s - want).abs() < 1e-9,
                "{v}: {s} vs {want} (plain {p}, v0 {v0})"
            );
        }
        assert_eq!(plain.y, slowed.y, "{v}");
    }
}

/// 1.9+: the sprint bonus needs full attack strength (above 0.9). An attack on a zombie resets
/// the cooldown; the player hit on the next tick then has no knockback and changes nothing.
/// 1.8 has no cooldown: the second hit unsprints.
#[test]
fn weak_hit_keeps_the_sprint_on_1_9_plus() {
    let t0 = Instant::now();
    for v in VERSIONS {
        let mut e = sprinting_at(v, Kind::Zombie, t0);
        let (actions, _, hit) = attack_tick(&mut e, t0, WALK, true);
        assert!(hit && actions.is_empty(), "{v}: {actions:?}");
        // The zombie is replaced by a player at the same spot.
        let s = e.core.sent.pos;
        let destroy = destroy(v, 50);
        e.sim.world_packet(&destroy).unwrap();
        put(&mut e, 50, Kind::Player, Vec3::new(s.x - 1.5, s.y, s.z));
        let (actions, _, hit) = attack_tick(&mut e, t0, WALK, true);
        assert!(hit, "{v}");
        if v.caps().combat_1_9 {
            assert!(actions.is_empty(), "{v}: {actions:?}");
        } else {
            assert_eq!(actions, [EntityAction::StopSprinting], "{v}");
        }
    }
}

/// The version's packet removing entity `id`.
fn destroy(v: GameVersion, id: i32) -> RawPacket {
    let mut b = bytes::BytesMut::new();
    b.put_varint(1);
    b.put_varint(id);
    let pid = match v.instance() {
        GameVersion::V1_8_9 => ghast_protocol::v1_8::play::clientbound::id::ENTITY_DESTROY,
        GameVersion::V1_12_2 => ghast_protocol::v1_12_2::play::clientbound::id::ENTITY_DESTROY,
        GameVersion::V1_16_5 => ghast_protocol::v1_16_5::play::clientbound::id::ENTITY_DESTROY,
        GameVersion::V26_1 => ghast_protocol::v26_1::play::clientbound::id::ENTITY_DESTROY,
        _ => ghast_protocol::v26_2::play::clientbound::id::ENTITY_DESTROY,
    };
    RawPacket::new(pid, b.freeze())
}

/// A knockback sword slows a walking (not sprinting) attacker on 1.8 – 1.16.5 (the client reads
/// the enchantment); the same hit without the enchantment changes nothing.
#[test]
fn knockback_enchantment_slows_without_sprint() {
    let t0 = Instant::now();
    for v in [
        GameVersion::V1_8_9,
        GameVersion::V1_12_2,
        GameVersion::V1_16_5,
    ] {
        let run = |kb: Option<u8>, attack: bool| {
            let mut e = setup(v);
            if let Some(level) = kb {
                e.inventory_mut()
                    .on_set_slot(0, 36, Some(knockback_sword(v, level)));
            }
            for _ in 0..8 {
                tick(&mut e, t0, |ctx| ctx.set_controls(WALK));
            }
            assert!(!e.core.pose.sprinting, "{v}");
            let s = e.core.sent.pos;
            put(&mut e, 50, Kind::Player, Vec3::new(s.x - 1.5, s.y, s.z));
            let v0 = e.core.pose.motion;
            let (actions, after, hit) = attack_tick(&mut e, t0, WALK, attack);
            assert!(hit == attack && actions.is_empty(), "{v}");
            (v0, after)
        };
        let (v0, plain) = run(Some(1), false);
        let (_, slowed) = run(Some(1), true);
        let f = f64::from(0.6f32 * 0.91);
        let want = plain.x - 0.4 * v0.x * f;
        assert!(
            (slowed.x - want).abs() < 1e-9,
            "{v}: {} vs {want}",
            slowed.x
        );
        let (_, no_kb) = run(None, true);
        assert_eq!(no_kb, plain, "{v}: no knockback, no slow-down");
    }
}

/// A diamond sword with Knockback `level` (1.8 / 1.12 `ench` id 19, 1.16 `minecraft:knockback`).
fn knockback_sword(v: GameVersion, level: u8) -> ghast_inventory::Stack {
    let mut b = vec![10, 0, 0, 9];
    if v == GameVersion::V1_16_5 {
        let id = b"minecraft:knockback";
        b.extend_from_slice(&[0, 12]);
        b.extend_from_slice(b"Enchantments");
        b.push(10);
        b.extend_from_slice(&1i32.to_be_bytes());
        b.extend_from_slice(&[8, 0, 2]);
        b.extend_from_slice(b"id");
        b.extend_from_slice(&(id.len() as u16).to_be_bytes());
        b.extend_from_slice(id);
    } else {
        b.extend_from_slice(&[0, 4]);
        b.extend_from_slice(b"ench");
        b.push(10);
        b.extend_from_slice(&1i32.to_be_bytes());
        b.extend_from_slice(&[2, 0, 2]);
        b.extend_from_slice(b"id");
        b.extend_from_slice(&19i16.to_be_bytes());
    }
    b.extend_from_slice(&[2, 0, 3]);
    b.extend_from_slice(b"lvl");
    b.extend_from_slice(&i16::from(level).to_be_bytes());
    b.extend_from_slice(&[0, 0]);
    let id = match v {
        GameVersion::V1_16_5 => i32::from(
            ghast_data::v1_16_5::Item::from_name("diamond_sword")
                .unwrap()
                .id(),
        ),
        _ => 276,
    };
    ghast_inventory::Stack::new(id, 1, 0).with_nbt(Some(
        ghast_protocol::nbt::RawNbt::from_bytes(bytes::Bytes::from(b)).unwrap(),
    ))
}

/// Our flags byte from the server: the sprint bit sets the flag only. With it cleared while
/// sprinting (sprint key up) the client reports the stop on its next movement update, the
/// accessors show the byte, and an attack then is no sprint attack.
#[test]
fn own_flags_byte_sets_the_sprint_flag() {
    let t0 = Instant::now();
    for v in VERSIONS {
        let mut e = setup(v);
        let mut seen = None;
        tick(&mut e, t0, |ctx| seen = Some(ctx.server_flags()));
        assert_eq!(seen, Some(None), "{v}");
        for _ in 0..8 {
            tick(&mut e, t0, |ctx| ctx.set_controls(RUN));
        }
        e.sim.world_packet(&own_flags(v, 0x08)).unwrap();
        let mut seen = (None, false);
        tick(&mut e, t0, |ctx| {
            ctx.set_controls(WALK);
            seen = (ctx.server_flags(), ctx.server_sprinting());
        });
        assert_eq!(seen, (Some(0x08), true), "{v}");
        assert!(e.core.pose.sprinting, "{v}");
        e.sim.world_packet(&own_flags(v, 0)).unwrap();
        e.note_inbound(t0);
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            ctx.set_controls(WALK);
            seen = (ctx.server_flags(), ctx.server_sprinting());
        });
        e.tick(&mut b, t0);
        let actions = sprint_actions(e.outbound());
        e.outbound().clear();
        assert_eq!(seen, (Some(0), false), "{v}");
        assert_eq!(actions, [EntityAction::StopSprinting], "{v}");
        assert!(!e.core.pose.sprinting, "{v}");
        // Not ours: ignored.
        let mut other = bytes::BytesMut::from(&own_flags(v, 0x08).body[..]);
        other[0] = 8;
        let id = own_flags(v, 0).id;
        e.sim
            .world_packet(&RawPacket::new(id, other.freeze()))
            .unwrap();
        tick(&mut e, t0, |ctx| {
            seen = (ctx.server_flags(), ctx.server_sprinting())
        });
        assert_eq!(seen, (Some(0), false), "{v}");
        // A new client player forgets it.
        e.sim.reset_player();
        tick(&mut e, t0, |ctx| {
            seen = (ctx.server_flags(), ctx.server_sprinting())
        });
        assert_eq!(seen, (None, false), "{v}");
    }
}

/// A dead player's keys do nothing (`isMovementBlocked`); after the respawn they apply again.
#[test]
fn dead_player_does_not_walk() {
    let t0 = Instant::now();
    for v in VERSIONS {
        let mut e = setup(v);
        tick(&mut e, t0, |ctx| ctx.set_controls(WALK));
        e.handle(Inbound::Health(Health {
            health: 0.0,
            food: 20,
            saturation: 0.0,
        }));
        let x0 = e.core.pose.pos.x;
        for _ in 0..20 {
            tick(&mut e, t0, |ctx| ctx.set_controls(WALK));
        }
        // The walk's motion runs out (0.546 friction): far less than 20 ticks of walking.
        let moved = x0 - e.core.pose.pos.x;
        assert!(moved < 0.25, "{v}: moved {moved}");
        assert!(e.core.pose.motion.x.abs() < 1e-3, "{v}");
    }
}

/// An open screen releases the keys (the sprint and the sneak end); with the option off the
/// controls apply through it.
#[test]
fn open_inventory_releases_the_keys() {
    let t0 = Instant::now();
    for v in VERSIONS {
        for release in [true, false] {
            let mut e = setup(v);
            e.core.opts.gui_releases_keys = release;
            for _ in 0..8 {
                tick(&mut e, t0, |ctx| ctx.set_controls(RUN));
            }
            assert!(e.core.pose.sprinting, "{v}");
            e.note_inbound(t0);
            let mut b = Script(|ctx: &mut TickContext<'_>| {
                ctx.set_controls(RUN);
                ctx.open_inventory().unwrap();
            });
            e.tick(&mut b, t0);
            let actions = sprint_actions(e.outbound());
            e.outbound().clear();
            if release {
                assert_eq!(actions, [EntityAction::StopSprinting], "{v}");
            } else {
                assert!(actions.is_empty(), "{v}: {actions:?}");
            }
            let x0 = e.core.pose.pos.x;
            for _ in 0..20 {
                tick(&mut e, t0, |ctx| ctx.set_controls(RUN));
            }
            let moved = x0 - e.core.pose.pos.x;
            assert_eq!(moved < 0.5, release, "{v}: moved {moved}");
            // Closed: the held keys apply again.
            tick(&mut e, t0, |ctx| {
                ctx.set_controls(RUN);
                ctx.close_window().unwrap();
            });
            let x1 = e.core.pose.pos.x;
            for _ in 0..5 {
                tick(&mut e, t0, |ctx| ctx.set_controls(RUN));
            }
            assert!(x1 - e.core.pose.pos.x > 0.5, "{v}");
        }
    }
}

/// `ctx.velocity()` shows the server's velocity before the step; the step's collisions show
/// the floor.
#[test]
fn velocity_and_collisions() {
    let t0 = Instant::now();
    for v in VERSIONS {
        let mut e = setup(v);
        for _ in 0..3 {
            tick(&mut e, t0, |_| {});
        }
        let mut seen = (Vec3::default(), false, false);
        tick(&mut e, t0, |ctx| {
            seen = (
                ctx.velocity(),
                ctx.horizontal_collision(),
                ctx.vertical_collision(),
            );
        });
        assert!(!seen.1 && seen.2, "{v}: standing on the floor {seen:?}");
        assert_eq!(seen.0, e.core.pose.motion, "{v}");
        let push = Vec3::new(0.25, 0.375, -0.125);
        e.handle(Inbound::Velocity {
            entity_id: 7,
            motion: push,
        });
        tick(&mut e, t0, |ctx| seen.0 = ctx.velocity());
        assert_eq!(seen.0, push, "{v}");
    }
}
