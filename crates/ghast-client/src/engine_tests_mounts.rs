//! Mount screens on 1.8.9, 1.12.2 and 1.16.5 with entity tracking: the saddle and armor slot
//! rules are read off the tracked entity when its window opens (`Engine::mount_screen_opened`,
//! what the connection calls after the inventory opened the window), and a behaviour's
//! `TickContext::set_mount_rules` still wins.

use super::tests::{Script, engine, join};
use super::*;
use crate::version::{ClientVersion, V1_8, V1_12_2, V1_16_5};
use ghast_inventory::{MountArmor, MountRules};
use ghast_protocol::RawPacket;

/// A packet body under construction.
#[derive(Default)]
struct Body(Vec<u8>);

impl Body {
    fn varint(mut self, v: i32) -> Self {
        let mut u = v as u32;
        loop {
            let b = (u & 0x7f) as u8;
            u >>= 7;
            if u == 0 {
                self.0.push(b);
                return self;
            }
            self.0.push(b | 0x80);
        }
    }
    fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn bytes(mut self, n: usize) -> Self {
        self.0.extend(std::iter::repeat_n(0, n));
        self
    }
    fn f64(mut self, v: f64) -> Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn raw(self, id: i32) -> RawPacket {
        RawPacket {
            id,
            body: bytes::Bytes::from(self.0),
        }
    }
}

/// The mount entity of the cases.
const MOUNT: i32 = 77;

fn tracked(v: GameVersion, metadata: bool) -> Engine<Box<dyn Sim>> {
    let mut e = engine(v);
    join(&mut e);
    e.sim.set_entities(Some(
        ghast_entity::Config::default().with_metadata(metadata),
    ));
    e
}

/// Opens the mount's window like the connection does (the inventory first, then the rules).
fn open<V: ClientVersion>(e: &mut Engine<Box<dyn Sim>>, raw: &RawPacket, id: i32) {
    let eid = V::mount_screen_entity(raw).expect("a mount screen");
    assert_eq!(eid, MOUNT);
    assert!(
        V::inventory_packet(e.inventory_mut(), raw)
            .unwrap()
            .is_none()
    );
    e.handle(Inbound::OpenWindow(id));
    e.mount_screen_opened(eid);
}

/// A behaviour's rules replace the derived ones.
fn caller_wins(e: &mut Engine<Box<dyn Sim>>) {
    let t0 = std::time::Instant::now();
    e.note_inbound(t0);
    let mut b = Script(|ctx: &mut TickContext<'_>| {
        ctx.set_mount_rules(Some(MountRules::new(false, MountArmor::Carpet)));
    });
    e.tick(&mut b, t0);
    let _ = b;
    assert_eq!(
        e.inventory_mut().mount_rules(),
        Some(MountRules::new(false, MountArmor::Carpet))
    );
}

/// 1.8.9: the saddle always, horse armor for horse type 0 (entity data 19).
#[test]
fn mount_rules_from_entity_1_8_9() {
    use ghast_protocol::v1_8::play::clientbound as cb;
    let window = |id: u8| {
        RawPacket::from_packet(&cb::OpenWindow {
            window_id: id,
            inventory_type: "EntityHorse".into(),
            window_title: "{\"text\":\"\"}".into(),
            slot_count: 2,
            entity_id: Some(MOUNT),
        })
        .unwrap()
    };
    // spawn_entity_living EntityHorse (100) with entity data 19 (horse type, a byte).
    let spawn = |ty: u8| {
        Body::default()
            .varint(MOUNT)
            .u8(100)
            .bytes(12 + 3 + 6)
            .u8(19)
            .u8(ty)
            .u8(0x7f)
            .raw(cb::id::SPAWN_ENTITY_LIVING)
    };
    for (ty, armor) in [(0, MountArmor::HorseArmor), (1, MountArmor::None)] {
        let mut e = tracked(GameVersion::V1_8_9, true);
        e.sim.world_packet(&spawn(ty)).unwrap();
        open::<V1_8>(&mut e, &window(3), 3);
        assert_eq!(
            e.inventory_mut().mount_rules(),
            Some(MountRules::new(true, armor))
        );
        caller_wins(&mut e);
    }
    // Without entity data the type is unknown: the caller's rules (none yet).
    let mut e = tracked(GameVersion::V1_8_9, false);
    e.sim.world_packet(&spawn(0)).unwrap();
    open::<V1_8>(&mut e, &window(3), 3);
    assert_eq!(e.inventory_mut().mount_rules(), None);
}

/// 1.12.2: the entity type decides (saddle for all but llamas; horse armor for horses,
/// carpets for llamas).
#[test]
fn mount_rules_from_entity_1_12_2() {
    use ghast_protocol::v1_12_2::play::clientbound as cb;
    let window = RawPacket::from_packet(&cb::OpenWindow {
        window_id: 4,
        inventory_type: "EntityHorse".into(),
        window_title: "{\"text\":\"\"}".into(),
        slot_count: 2,
        entity_id: Some(MOUNT),
    })
    .unwrap();
    for (name, rules) in [
        ("horse", MountRules::new(true, MountArmor::HorseArmor)),
        ("mule", MountRules::new(true, MountArmor::None)),
        ("llama", MountRules::new(false, MountArmor::Carpet)),
    ] {
        let ty = ghast_data::v1_12::entity_by_name(name).unwrap().id;
        let spawn = Body::default()
            .varint(MOUNT)
            .bytes(16)
            .varint(i32::from(ty))
            .f64(0.5)
            .f64(64.0)
            .f64(0.5)
            .bytes(3 + 6)
            .u8(0xff)
            .raw(cb::id::SPAWN_ENTITY_LIVING);
        // Without entity data too: the type is enough.
        let mut e = tracked(GameVersion::V1_12_2, false);
        e.sim.world_packet(&spawn).unwrap();
        open::<V1_12_2>(&mut e, &window, 4);
        assert_eq!(e.inventory_mut().mount_rules(), Some(rules), "{name}");
    }
}

/// 1.16.5: the saddle for a tame (horse flags 16, bit 2) adult (entity data 15) horse-like
/// mount, never for llamas; horse armor for horses, carpets for (trader) llamas.
#[test]
fn mount_rules_from_entity_1_16_5() {
    use ghast_protocol::v1_16_5::play::clientbound as cb;
    let window = RawPacket::from_packet(&cb::OpenHorseWindow {
        window_id: 5,
        nb_slots: 2,
        entity_id: MOUNT,
    })
    .unwrap();
    let spawn = |name: &str| {
        let ty = ghast_data::v1_16_5::entity_by_name(name).unwrap().id;
        Body::default()
            .varint(MOUNT)
            .bytes(16)
            .varint(i32::from(ty))
            .f64(0.5)
            .f64(64.0)
            .f64(0.5)
            .bytes(3 + 6)
            .raw(cb::id::SPAWN_ENTITY_LIVING)
    };
    // entity_metadata: 15 baby (boolean, type 7), 16 horse flags (byte, type 0).
    let data = |baby: bool, flags: u8| {
        Body::default()
            .varint(MOUNT)
            .u8(15)
            .varint(7)
            .u8(u8::from(baby))
            .u8(16)
            .varint(0)
            .u8(flags)
            .u8(0xff)
            .raw(cb::id::ENTITY_METADATA)
    };
    for (name, baby, flags, rules) in [
        (
            "horse",
            false,
            2,
            MountRules::new(true, MountArmor::HorseArmor),
        ),
        (
            "horse",
            true,
            2,
            MountRules::new(false, MountArmor::HorseArmor),
        ),
        ("donkey", false, 0, MountRules::new(false, MountArmor::None)),
        (
            "skeleton_horse",
            false,
            2,
            MountRules::new(true, MountArmor::None),
        ),
        (
            "trader_llama",
            false,
            2,
            MountRules::new(false, MountArmor::Carpet),
        ),
    ] {
        let mut e = tracked(GameVersion::V1_16_5, true);
        e.sim.world_packet(&spawn(name)).unwrap();
        e.sim.world_packet(&data(baby, flags)).unwrap();
        open::<V1_16_5>(&mut e, &window, 5);
        assert_eq!(e.inventory_mut().mount_rules(), Some(rules), "{name}");
        caller_wins(&mut e);
    }
    // Without entity data a horse's saddle is unknown; a llama's rules are its type's.
    let mut e = tracked(GameVersion::V1_16_5, false);
    e.sim.world_packet(&spawn("horse")).unwrap();
    open::<V1_16_5>(&mut e, &window, 5);
    assert_eq!(e.inventory_mut().mount_rules(), None);
    let mut e = tracked(GameVersion::V1_16_5, false);
    e.sim.world_packet(&spawn("llama")).unwrap();
    open::<V1_16_5>(&mut e, &window, 5);
    assert_eq!(
        e.inventory_mut().mount_rules(),
        Some(MountRules::new(false, MountArmor::Carpet))
    );
}
