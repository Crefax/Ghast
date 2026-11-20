//! The player's own state from real packet bodies on every version, decoded by the version's
//! wire layer: status effects with durations and their events, absorption from our entity
//! data, the spawn point, our hurts and our death with its killer.

use super::tests::{Script, engine, join};
use super::*;
use crate::common::{ActiveEffect, EffectEvent};
use crate::version::ClientVersion;
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

/// Our entity id in the test join.
const ME: i32 = 7;

/// The version's packet ids by name.
fn pid(v: GameVersion, name: &str) -> i32 {
    macro_rules! ids {
        ($m:ident) => {{
            use ghast_protocol::$m::play::clientbound::id;
            match name {
                "effect" => id::ENTITY_EFFECT,
                "remove_effect" => id::REMOVE_ENTITY_EFFECT,
                "status" => id::ENTITY_STATUS,
                "metadata" => id::ENTITY_METADATA,
                "spawn" => id::SPAWN_POSITION,
                _ => unreachable!("{name}"),
            }
        }};
    }
    match v {
        GameVersion::V1_8_9 => ids!(v1_8),
        GameVersion::V1_12_2 => ids!(v1_12_2),
        GameVersion::V1_16_5 => ids!(v1_16_5),
        GameVersion::V26_1 => ids!(v26_1),
        _ => ids!(v26_2),
    }
}

/// Decodes `raw` with the version's wire layer and hands it to the engine.
fn feed(e: &mut Engine<Box<dyn Sim>>, raw: &RawPacket) {
    let v = e.core.opts.caps.version;
    let p = match v {
        GameVersion::V1_8_9 => crate::version::V1_8::decode_play(raw, &mut ()),
        GameVersion::V1_12_2 => {
            crate::version::V1_12_2::decode_play(raw, &mut crate::version::Wire1_12::default())
        }
        GameVersion::V1_16_5 => crate::version::V1_16_5::decode_play(raw, &mut ()),
        GameVersion::V26_1 => {
            crate::version::V26_1::decode_play(raw, &mut crate::version::Wire26_1::default())
        }
        _ => crate::version::V26_2::decode_play(raw, &mut crate::version::Wire26_2::default()),
    }
    .unwrap();
    match p {
        Some(p) => e.handle(p),
        // Entity data is the sim's alone.
        None => assert_eq!(raw.id, pid(v, "metadata"), "a packet the core decodes"),
    }
    // Our entity data reaches the sim as a world packet.
    if raw.id == pid(v, "metadata") {
        e.sim.world_packet(raw).unwrap();
    }
}

fn modern(v: GameVersion) -> bool {
    v.caps().configuration_state
}

/// `entity_effect` for us: speed (1) II for `duration` ticks.
fn effect(v: GameVersion, amplifier: i8, duration: i32) -> RawPacket {
    let mut b = bytes::BytesMut::new();
    b.put_varint(ME);
    if modern(v) {
        b.put_varint(0); // registry id 0 = speed
        b.put_varint(i32::from(amplifier));
        b.put_varint(duration);
        b.put_u8(0x06);
    } else {
        b.put_i8(1);
        b.put_i8(amplifier);
        b.put_varint(duration);
        b.put_u8(if v == GameVersion::V1_8_9 { 1 } else { 0x06 });
    }
    RawPacket::new(pid(v, "effect"), b.freeze())
}

fn remove_effect(v: GameVersion) -> RawPacket {
    let mut b = bytes::BytesMut::new();
    b.put_varint(ME);
    if modern(v) {
        b.put_varint(0);
    } else {
        b.put_i8(1);
    }
    RawPacket::new(pid(v, "remove_effect"), b.freeze())
}

/// Our absorption amount in our entity data (index 17 / 11 / 14 / 17).
fn absorption(v: GameVersion, a: f32) -> RawPacket {
    let mut b = bytes::BytesMut::new();
    b.put_varint(ME);
    match v {
        GameVersion::V1_8_9 => {
            // type 3 (float) << 5 | index 17
            b.put_u8(3 << 5 | 17);
            b.put_f32(a);
            b.put_u8(0x7f);
        }
        _ => {
            let (index, float) = match v {
                GameVersion::V1_12_2 => (11, 2),
                GameVersion::V1_16_5 => (14, 2),
                _ => (17, 3),
            };
            b.put_u8(index);
            b.put_varint(float);
            b.put_f32(a);
            b.put_u8(0xff);
        }
    }
    RawPacket::new(pid(v, "metadata"), b.freeze())
}

fn tick_with(e: &mut Engine<Box<dyn Sim>>, f: impl FnMut(&mut TickContext<'_>)) {
    let mut b = Script(f);
    e.tick(&mut b, Instant::now());
}

/// Effects keep their duration and the tick the behaviour first saw them; every packet is an
/// event (a re-application with the same values too), a respawn clears them.
#[test]
fn own_effects_with_durations_and_events() {
    for v in VERSIONS {
        let mut e = engine(v);
        join(&mut e);
        e.note_inbound(Instant::now());
        for _ in 0..2 {
            tick_with(&mut e, |_| {});
        }
        feed(&mut e, &effect(v, 1, 600));
        let mut seen: (u64, Vec<ActiveEffect>, Vec<EffectEvent>) = (0, Vec::new(), Vec::new());
        tick_with(&mut e, |ctx| {
            seen = (
                ctx.tick(),
                ctx.effects().to_vec(),
                ctx.events().effects().to_vec(),
            )
        });
        let (t, effects, events) = seen.clone();
        assert_eq!(effects.len(), 1, "{v:?}");
        let a = effects[0];
        assert_eq!((a.id, a.amplifier, a.duration), (1, 1, 600), "{v:?}");
        assert_eq!(a.received_tick, t, "{v:?}");
        assert_eq!(a.remaining(t + 100), Some(500));
        assert_eq!(a.remaining(t + 1000), Some(0));
        assert_eq!(events, [EffectEvent::Applied(a)], "{v:?}");
        // The same effect again: a new event and a new receive tick.
        tick_with(&mut e, |_| {});
        feed(&mut e, &effect(v, 1, 600));
        tick_with(&mut e, |ctx| {
            seen = (
                ctx.tick(),
                ctx.effects().to_vec(),
                ctx.events().effects().to_vec(),
            )
        });
        assert_eq!(seen.1.len(), 1, "{v:?}");
        assert_eq!(seen.1[0].received_tick, seen.0, "{v:?}");
        assert!(matches!(seen.2[..], [EffectEvent::Applied(_)]), "{v:?}");
        feed(&mut e, &remove_effect(v));
        tick_with(&mut e, |ctx| {
            seen = (0, ctx.effects().to_vec(), ctx.events().effects().to_vec())
        });
        assert!(seen.1.is_empty(), "{v:?}");
        assert_eq!(seen.2, [EffectEvent::Removed(1)], "{v:?}");
        // Infinite (1.19.4+ duration −1), then a respawn drops it (the behaviour waits for
        // the server position then: the events stay pending).
        feed(&mut e, &effect(v, 0, -1));
        e.handle(Inbound::Respawn {
            dimension: e.core.dimension.clone(),
            game_mode: GameMode::Survival,
            keep: 0,
        });
        assert!(e.core.active_effects.is_empty(), "{v:?}");
        assert!(
            matches!(e.core.pending.effects()[..], [EffectEvent::Applied(a), EffectEvent::Cleared] if a.remaining(1 << 40).is_none()),
            "{v:?} {:?}",
            e.core.pending.effects()
        );
    }
}

/// `Health::absorption` from our entity data, version-independent.
#[test]
fn absorption_from_own_entity_data() {
    for v in VERSIONS {
        let mut e = engine(v);
        join(&mut e);
        e.sim.test_set_own_id(ME);
        e.note_inbound(Instant::now());
        tick_with(&mut e, |_| {});
        feed(&mut e, &absorption(v, 4.0));
        let mut seen = (0.0, false);
        tick_with(&mut e, |ctx| {
            seen = (ctx.health().absorption, ctx.events().health_changed)
        });
        assert_eq!(seen, (4.0, true), "{v:?}");
        // update_health does not touch it.
        e.handle(Inbound::Health(Health {
            health: 10.0,
            ..Health::default()
        }));
        tick_with(&mut e, |ctx| seen = (ctx.health().absorption, false));
        assert_eq!(seen.0, 4.0, "{v:?}");
    }
}

/// The spawn point, our hurt statuses (26.x: the damage event) and our death with its killer.
#[test]
fn spawn_point_hurt_and_death() {
    for v in VERSIONS {
        let mut e = engine(v);
        join(&mut e);
        e.note_inbound(Instant::now());
        tick_with(&mut e, |_| {});
        // spawn_position: (-3, 70, 12).
        let mut b = bytes::BytesMut::new();
        let (x, y, z) = (-3i64, 70i64, 12i64);
        if v == GameVersion::V1_8_9 || v == GameVersion::V1_12_2 {
            b.put_i64(((x & 0x3ff_ffff) << 38) | ((y & 0xfff) << 26) | (z & 0x3ff_ffff));
        } else {
            if modern(v) {
                b.put_varint(19);
                b.put_slice(b"minecraft:overworld");
            }
            b.put_i64(((x & 0x3ff_ffff) << 38) | ((z & 0x3ff_ffff) << 12) | (y & 0xfff));
            if modern(v) {
                b.put_f32(90.0);
                b.put_f32(-10.0);
            }
        }
        feed(&mut e, &RawPacket::new(pid(v, "spawn"), b.freeze()));
        // Hurt: status 2 (legacy) / damage_event with attacker 42 (26.x).
        let hurt = if modern(v) {
            let mut b = bytes::BytesMut::new();
            b.put_varint(ME);
            b.put_varint(3);
            b.put_varint(43);
            b.put_varint(43);
            b.put_u8(0);
            let id = if v == GameVersion::V26_1 {
                ghast_protocol::v26_1::play::clientbound::id::DAMAGE_EVENT
            } else {
                ghast_protocol::v26_2::play::clientbound::id::DAMAGE_EVENT
            };
            RawPacket::new(id, b.freeze())
        } else {
            let mut b = bytes::BytesMut::new();
            b.put_i32(ME);
            b.put_i8(2);
            RawPacket::new(pid(v, "status"), b.freeze())
        };
        feed(&mut e, &hurt);
        // Death: 1.8 – 1.16.5 combat_event "entity dead" (player, killer 42, JSON message);
        // 26.x player_combat_kill (player, NBT text component).
        let mut b = bytes::BytesMut::new();
        let death_id = match v {
            GameVersion::V1_8_9 | GameVersion::V1_12_2 | GameVersion::V1_16_5 => {
                b.put_varint(2);
                b.put_varint(ME);
                b.put_i32(42);
                let msg = br#"{"text":"slain"}"#;
                b.put_varint(msg.len() as i32);
                b.put_slice(msg);
                match v {
                    GameVersion::V1_8_9 => {
                        ghast_protocol::v1_8::play::clientbound::id::COMBAT_EVENT
                    }
                    GameVersion::V1_12_2 => {
                        ghast_protocol::v1_12_2::play::clientbound::id::COMBAT_EVENT
                    }
                    _ => ghast_protocol::v1_16_5::play::clientbound::id::COMBAT_EVENT,
                }
            }
            _ => {
                b.put_varint(ME);
                // TAG_String "slain" (network NBT: no root name).
                b.put_u8(8);
                b.put_u16(5);
                b.put_slice(b"slain");
                if v == GameVersion::V26_1 {
                    ghast_protocol::v26_1::play::clientbound::id::DEATH_COMBAT_EVENT
                } else {
                    ghast_protocol::v26_2::play::clientbound::id::DEATH_COMBAT_EVENT
                }
            }
        };
        feed(&mut e, &RawPacket::new(death_id, b.freeze()));
        let mut seen = None;
        tick_with(&mut e, |ctx| {
            seen = Some((
                ctx.tick(),
                ctx.spawn_position().cloned(),
                ctx.events().hurt().to_vec(),
                ctx.last_hurt_tick(),
                ctx.events().deaths().to_vec(),
            ))
        });
        let (t, spawn, hurt, last_hurt, deaths) = seen.unwrap();
        let spawn = spawn.unwrap();
        assert_eq!(
            spawn.pos,
            ghast_protocol::BlockPos::new(-3, 70, 12),
            "{v:?}"
        );
        if modern(v) {
            assert_eq!((spawn.yaw, spawn.pitch), (90.0, -10.0));
            assert_eq!(spawn.dimension.as_deref(), Some("minecraft:overworld"));
            assert_eq!(hurt.len(), 1);
            assert_eq!(
                (hurt[0].source_type, hurt[0].attacker, hurt[0].direct),
                (Some(3), Some(42), Some(42))
            );
        } else {
            assert_eq!(hurt.len(), 1, "{v:?}");
            assert_eq!(hurt[0].status, Some(2), "{v:?}");
        }
        assert_eq!(last_hurt, Some(t), "{v:?}");
        assert_eq!(deaths.len(), 1, "{v:?}");
        assert_eq!(deaths[0].entity_id, ME);
        let killer = if modern(v) { None } else { Some(42) };
        assert_eq!(deaths[0].killer_id, killer, "{v:?}");
        assert_eq!(deaths[0].message.plain(), "slain", "{v:?}");
    }
}

/// A team with Alice on it in the version's team packet layout (red, no friendly fire).
fn team_packet(v: GameVersion) -> RawPacket {
    let mut b = bytes::BytesMut::new();
    b.put_string("red").unwrap();
    b.put_i8(0);
    let nbt = |b: &mut bytes::BytesMut, s: &str| {
        b.put_u8(8);
        b.put_u16(s.len() as u16);
        b.put_slice(s.as_bytes());
    };
    let id = match v {
        GameVersion::V1_8_9 | GameVersion::V1_12_2 => {
            for s in ["Red", "[R] ", ""] {
                b.put_string(s).unwrap();
            }
            b.put_u8(0);
            b.put_string("always").unwrap();
            if v == GameVersion::V1_12_2 {
                b.put_string("never").unwrap();
            }
            b.put_i8(12);
            if v == GameVersion::V1_8_9 {
                ghast_protocol::v1_8::play::clientbound::id::SCOREBOARD_TEAM
            } else {
                ghast_protocol::v1_12_2::play::clientbound::id::TEAMS
            }
        }
        GameVersion::V1_16_5 => {
            b.put_string(r#"{"text":"Red"}"#).unwrap();
            b.put_u8(0);
            b.put_string("always").unwrap();
            b.put_string("never").unwrap();
            b.put_varint(12);
            b.put_string(r#"{"text":"[R] "}"#).unwrap();
            b.put_string(r#"{"text":""}"#).unwrap();
            ghast_protocol::v1_16_5::play::clientbound::id::TEAMS
        }
        GameVersion::V26_1 => {
            nbt(&mut b, "Red");
            b.put_u8(0);
            b.put_varint(0);
            b.put_varint(1);
            b.put_varint(12);
            nbt(&mut b, "[R] ");
            nbt(&mut b, "");
            ghast_protocol::v26_1::play::clientbound::id::TEAMS
        }
        _ => {
            nbt(&mut b, "Red");
            nbt(&mut b, "[R] ");
            nbt(&mut b, "");
            b.put_varint(0);
            b.put_varint(1);
            b.put_u8(1);
            b.put_varint(12);
            b.put_u8(0);
            ghast_protocol::v26_2::play::clientbound::id::TEAMS
        }
    };
    b.put_varint(1);
    b.put_string("Alice").unwrap();
    RawPacket::new(id, b.freeze())
}

/// The scoreboard teams on every version, only with the interest; a join clears them.
#[test]
fn teams_on_every_version() {
    for v in VERSIONS {
        let mut e = engine(v);
        join(&mut e);
        e.note_inbound(Instant::now());
        // Without the interest the packet is ignored.
        feed(&mut e, &team_packet(v));
        assert!(e.core.teams.is_none());
        e.track_teams(true);
        feed(&mut e, &team_packet(v));
        let mut seen = None;
        tick_with(&mut e, |ctx| {
            seen = ctx.teams().and_then(|t| t.team_of("Alice")).cloned();
        });
        let t = seen.unwrap_or_else(|| panic!("{v:?}"));
        assert_eq!((&*t.name, t.prefix.as_str()), ("red", "[R] "), "{v:?}");
        assert_eq!((t.color, t.friendly_fire), (Some(12), false), "{v:?}");
        let rule = if v == GameVersion::V1_8_9 {
            "always"
        } else {
            "never"
        };
        assert_eq!(t.collision_rule, rule, "{v:?}");
        join(&mut e);
        assert!(e.core.teams.as_ref().unwrap().is_empty(), "{v:?}");
    }
}
