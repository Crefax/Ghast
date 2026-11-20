//! The client view of other entities through the engine on every version: the crosshair pick
//! tests the drawn (interpolated) box like vanilla's `updateTargetedEntity`, the entity events
//! reach the behaviour, and the store's clock is the tick in which the behaviour sees a packet.

use super::tests::{Script, engine, join};
use super::*;
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

/// A zombie spawn at `p` in the version's layout.
fn zombie(v: GameVersion, id: i32, p: [f64; 3]) -> RawPacket {
    let mut b = bytes::BytesMut::new();
    b.put_varint(id);
    let pid = match v {
        GameVersion::V1_8_9 => {
            b.put_u8(54);
            for c in p {
                b.put_i32((c * 32.0).floor() as i32);
            }
            b.put_slice(&[0; 9]);
            b.put_u8(0x7f);
            ghast_protocol::v1_8::play::clientbound::id::SPAWN_ENTITY_LIVING
        }
        GameVersion::V1_12_2 | GameVersion::V1_16_5 => {
            b.put_u128(0x7465_7374 + id as u128);
            b.put_varint(if v == GameVersion::V1_12_2 { 54 } else { 102 });
            for c in p {
                b.put_f64(c);
            }
            b.put_slice(&[0; 9]);
            if v == GameVersion::V1_12_2 {
                b.put_u8(0xff);
                ghast_protocol::v1_12_2::play::clientbound::id::SPAWN_ENTITY_LIVING
            } else {
                ghast_protocol::v1_16_5::play::clientbound::id::SPAWN_ENTITY_LIVING
            }
        }
        _ => {
            b.put_u128(0x7465_7374 + id as u128);
            let (ty, pid) = if v == GameVersion::V26_1 {
                (
                    ghast_data::v26_1::entity_by_name("zombie").unwrap().id,
                    ghast_protocol::v26_1::play::clientbound::id::SPAWN_ENTITY,
                )
            } else {
                (
                    ghast_data::v26_2::entity_by_name("zombie").unwrap().id,
                    ghast_protocol::v26_2::play::clientbound::id::SPAWN_ENTITY,
                )
            };
            b.put_varint(i32::from(ty));
            for c in p {
                b.put_f64(c);
            }
            // Zero velocity (LpVec3), pitch, yaw, head yaw, data.
            b.put_slice(&[0, 0, 0, 0]);
            b.put_varint(0);
            pid
        }
    };
    RawPacket::new(pid, b.freeze())
}

/// A relative move by `dz` blocks on z (on ground).
fn move_z(v: GameVersion, id: i32, dz: f64) -> RawPacket {
    let mut b = bytes::BytesMut::new();
    b.put_varint(id);
    let pid = if v == GameVersion::V1_8_9 {
        b.put_slice(&[0, 0, (dz * 32.0) as i8 as u8]);
        ghast_protocol::v1_8::play::clientbound::id::REL_ENTITY_MOVE
    } else {
        b.put_i16(0);
        b.put_i16(0);
        b.put_i16((dz * 4096.0) as i16);
        match v {
            GameVersion::V1_12_2 => ghast_protocol::v1_12_2::play::clientbound::id::REL_ENTITY_MOVE,
            GameVersion::V1_16_5 => ghast_protocol::v1_16_5::play::clientbound::id::REL_ENTITY_MOVE,
            GameVersion::V26_1 => ghast_protocol::v26_1::play::clientbound::id::REL_ENTITY_MOVE,
            _ => ghast_protocol::v26_2::play::clientbound::id::REL_ENTITY_MOVE,
        }
    };
    b.put_u8(1);
    RawPacket::new(pid, b.freeze())
}

/// What the client sets up for entity tracking plus [`Interests::ENTITY_EVENTS`].
fn tracking(v: GameVersion) -> ghast_entity::Config {
    crate::client::entity_config(
        Interests::ENTITY_TRACKING | Interests::ENTITY_EVENTS,
        v.caps(),
    )
    .unwrap()
}

/// A zombie 2 blocks west of the eyes (which look west along z = 8.5) moves 0.5 blocks south.
/// Its server box leaves the ray at once, but vanilla's pick tests the drawn box, which
/// follows over the 3 interpolation steps after the packet: the pick keeps the zombie until
/// the drawn box (grown by the 1.8 margin of 0.1) no longer reaches z = 8.5. 26.x: the drawn
/// position was the tracker's position already.
#[test]
fn pick_follows_the_drawn_position() {
    for v in VERSIONS {
        let mut e = engine(v);
        e.sim.set_entities(Some(tracking(v)));
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        let mut picks = Vec::new();
        let mut events = Vec::new();
        let mut ticks = Vec::new();
        let mut tick = |e: &mut Engine<Box<dyn Sim>>, picks: &mut Vec<bool>| {
            let mut b = Script(|ctx: &mut TickContext<'_>| {
                picks.push(ctx.target_entity().is_some_and(|h| h.id == 50));
                events.extend(ctx.events().entities.iter().copied());
                if let Some(z) = ctx.entities().and_then(|t| t.get(50)) {
                    ticks.push((ctx.tick(), z.spawn_tick(), z.position_tick()));
                }
            });
            e.tick(&mut b, t0);
        };
        // A few ticks until the level is in and the pose is sent.
        for _ in 0..3 {
            tick(&mut e, &mut Vec::new());
        }
        e.sim
            .world_packet(&zombie(v, 50, [6.5, 64.0, 8.5]))
            .unwrap();
        tick(&mut e, &mut picks);
        e.sim.world_packet(&move_z(v, 50, 0.5)).unwrap();
        for _ in 0..4 {
            tick(&mut e, &mut picks);
        }
        drop(tick);
        let expected: &[bool] = match v {
            // Drawn z after each step: 8.5, 8.667, 8.833, 9.0; box half width 0.3 + margin 0.1.
            GameVersion::V1_8_9 => &[true, true, true, true, false],
            // No margin since 1.9: 8.833 - 0.3 > 8.5.
            GameVersion::V1_12_2 | GameVersion::V1_16_5 => &[true, true, true, false, false],
            // 26.x: the tracker's own interpolation.
            _ => &[true, true, true, false, false],
        };
        assert_eq!(picks, expected, "{v:?}");
        // The spawn and the move reached the behaviour as events, in order.
        assert_eq!(events[0], ghast_entity::Event::Spawned(50), "{v:?}");
        assert!(events.contains(&ghast_entity::Event::Moved(50)), "{v:?}");
        // The clocks: spawned and moved in the ticks the behaviour first saw the packets.
        let (spawn_seen, spawned, _) = ticks[0];
        assert_eq!(spawned, Some(spawn_seen), "{v:?}");
        assert_eq!(ticks[1].2, Some(ticks[1].0), "{v:?}");
    }
}

/// Without [`Interests::ENTITY_EVENTS`] nothing is queued.
#[test]
fn entity_events_need_the_interest() {
    let v = GameVersion::V1_8_9;
    let mut e = engine(v);
    e.sim.set_entities(crate::client::entity_config(
        Interests::ENTITY_TRACKING,
        v.caps(),
    ));
    join(&mut e);
    let t0 = Instant::now();
    e.note_inbound(t0);
    e.sim
        .world_packet(&zombie(v, 50, [6.5, 64.0, 8.5]))
        .unwrap();
    let mut n = usize::MAX;
    let mut b = Script(|ctx: &mut TickContext<'_>| n = ctx.events().entities.len());
    e.tick(&mut b, t0);
    assert_eq!(n, 0);
}
