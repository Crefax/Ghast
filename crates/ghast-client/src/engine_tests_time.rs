//! The world's time (`update_time`) on 1.8.9, 1.12.2 and 1.16.5: decoded by the version's
//! packet path, kept by the engine and counted on between updates like the vanilla client
//! (`WorldClient.tick`: game time + 1, the day + 1 while `doDaylightCycle` is on; a negative
//! day time in the packet means the rule is off).

use super::tests::{Script, engine, join};
use super::*;
use crate::common::WorldTime;
use crate::version::{ClientVersion, V1_8, V1_12_2, V1_16_5};
use ghast_protocol::RawPacket;

/// Applies `raw` through the version's decoder, then runs `ticks` ticks; returns the time.
fn after<V: ClientVersion>(
    e: &mut Engine<Box<dyn Sim>>,
    raw: &RawPacket,
    ticks: u32,
) -> Option<WorldTime> {
    let mut wire = V::Wire::default();
    let inbound = V::decode_play(raw, &mut wire)
        .unwrap()
        .expect("update_time decoded");
    assert!(matches!(inbound, Inbound::Time(_)), "{inbound:?}");
    e.handle(inbound);
    let t0 = Instant::now();
    let mut seen = None;
    for _ in 0..ticks {
        e.note_inbound(t0);
        let mut b = Script(|ctx: &mut TickContext<'_>| seen = ctx.world_time());
        e.tick(&mut b, t0);
    }
    if ticks == 0 {
        seen = e.core.clocks.world_time(e.core.day_clock);
    }
    seen
}

fn check<V: ClientVersion>(v: GameVersion, packet: impl Fn(i64, i64) -> RawPacket) {
    let mut e = engine(v);
    join(&mut e);
    assert_eq!(e.core.clocks.world_time(e.core.day_clock), None, "{v:?}");
    // The cycle runs: both clocks count on.
    let t = after::<V>(&mut e, &packet(1000, 30_100), 0).unwrap();
    assert_eq!(
        (t.age, t.day_time, t.day_rate),
        (1000, Some(30_100), 1.0),
        "{v:?}"
    );
    assert_eq!((t.time_of_day(), t.day()), (Some(6100), Some(1)));
    let t = after::<V>(&mut e, &packet(1000, 30_100), 5).unwrap();
    // the behaviour of the 5th tick sees 5 ticks passed
    assert_eq!((t.age, t.day_time), (1005, Some(30_105)), "{v:?}");
    // doDaylightCycle off: the server sends the day negated; only the game time moves.
    let t = after::<V>(&mut e, &packet(2000, -18_000), 3).unwrap();
    assert_eq!((t.age, t.day_time), (2003, Some(18_000)), "{v:?}");
    assert!(t.day_frozen());
    assert_eq!(t.time_of_day(), Some(18_000));
}

#[test]
fn world_time_1_8_9() {
    use ghast_protocol::v1_8::play::clientbound as cb;
    check::<V1_8>(GameVersion::V1_8_9, |age, time| {
        RawPacket::from_packet(&cb::UpdateTime { age, time }).unwrap()
    });
}

#[test]
fn world_time_1_12_2() {
    use ghast_protocol::v1_12_2::play::clientbound as cb;
    check::<V1_12_2>(GameVersion::V1_12_2, |age, time| {
        RawPacket::from_packet(&cb::UpdateTime { age, time }).unwrap()
    });
}

#[test]
fn world_time_1_16_5() {
    use ghast_protocol::v1_16_5::play::clientbound as cb;
    check::<V1_16_5>(GameVersion::V1_16_5, |age, time| {
        RawPacket::from_packet(&cb::UpdateTime { age, time }).unwrap()
    });
}
