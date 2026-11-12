//! 26.2 known-answer tests (what differs from 26.1: ids, the new sulfur cube, cube mobs as
//! ageable mobs, the scale attribute id), and the real vanilla 26.2 capture
//! (`ghast-protocol/tests/fixtures/capture_26_2.bin.zz`) through the tracker.

mod common;

use std::io::Read as _;

use bytes::Bytes;
use common::W;
use ghast_entity::v26_2::{self, Tracker};
use ghast_entity::{Applied, Config, EntityClass, EntityType};
use ghast_protocol::codec::Reader;
use ghast_protocol::v26_2::play::clientbound::{ClientboundPlayPacket, id};
use ghast_protocol::{PacketSet, RawPacket};

const SELF: i32 = 1;

/// The 26.2 network id of an entity type, by name.
fn ty(name: &str) -> i32 {
    i32::from(ghast_data::v26_2::entity_by_name(name).unwrap().id)
}

fn login(dimension: &str) -> W {
    W::new()
        .i32(SELF)
        .bool(false)
        .varint(1)
        .str(dimension)
        .varint(20)
        .varint(12)
        .varint(12)
        .bool(false)
        .bool(true)
        .bool(false)
        .varint(0)
        .str(dimension)
        .i64(0)
        .i8(0)
        .u8(0xFF)
        .bool(false)
        .bool(false)
        .bool(false)
        .varint(0)
        .varint(63)
        // online mode (new in 26.2), enforces secure chat
        .bool(false)
        .bool(false)
}

fn tracker(config: Config) -> Tracker {
    let mut t = Tracker::new(config);
    assert_eq!(
        t.apply(&login("minecraft:overworld").raw(id::LOGIN)),
        Ok(Applied::Changed)
    );
    for x in -4..=4 {
        for z in -4..=4 {
            t.apply(&W::new().i32(x).i32(z).raw(id::MAP_CHUNK)).unwrap();
        }
    }
    t
}

fn spawn(t: &mut Tracker, eid: i32, ty: i32, p: [f64; 3]) -> Applied {
    let w = W::new()
        .varint(eid)
        .uuid(0x1000 + eid as u128)
        .varint(ty)
        .f64(p[0])
        .f64(p[1])
        .f64(p[2])
        .u8(0) // zero LpVec3
        .i8(0)
        .i8(0)
        .i8(0)
        .varint(0);
    t.apply(&w.raw(id::SPAWN_ENTITY)).unwrap()
}

fn metadata(t: &mut Tracker, eid: i32, entries: W) {
    let w = W::new().varint(eid).bytes(&entries.raw(0).body).u8(0xFF);
    assert_eq!(t.apply(&w.raw(id::ENTITY_METADATA)), Ok(Applied::Changed));
}

#[test]
fn ids_come_from_the_26_2_registry() {
    // Player moved from 155 (26.1) to 156; the sulfur cube is new.
    assert_eq!(ty("player"), 156);
    let mut t = tracker(Config::default().with_player_list(false));
    assert_eq!(
        spawn(&mut t, 10, ty("player"), [0.5, 64.0, 0.5]),
        Applied::Changed
    );
    assert_eq!(t.get(10).unwrap().entity_type(), EntityType::Player);
    assert_eq!(t.get(10).unwrap().class(), EntityClass::Player);
    spawn(&mut t, 11, ty("item"), [0.5, 64.0, 0.5]);
    assert_eq!(t.get(11).unwrap().class(), EntityClass::Item);
    spawn(&mut t, 12, ty("experience_orb"), [0.5, 64.0, 0.5]);
    assert_eq!(t.get(12).unwrap().class(), EntityClass::ExperienceOrb);
    spawn(&mut t, 13, ty("sulfur_cube"), [0.5, 64.0, 0.5]);
    let cube = t.get(13).unwrap();
    assert_eq!(cube.class(), EntityClass::Mob);
    assert_eq!(v26_2::info(cube.entity_type()).unwrap().name, "sulfur_cube");
    assert_eq!(v26_2::SCALE_ATTRIBUTE, 30);
}

#[test]
fn cube_mobs_size_by_their_size_not_their_age() {
    let mut t = tracker(Config::default().with_metadata(false));
    let td = ghast_data::v26_2::entity_type(ty("sulfur_cube") as u16).unwrap();
    let (baby, size) = (td.baby_meta.unwrap(), td.size_meta.unwrap());
    for (eid, name, w) in [(10, "sulfur_cube", 0.49f32), (11, "slime", 0.52)] {
        spawn(&mut t, eid, ty(name), [0.0, 64.0, 0.0]);
        // Size 3; the baby flag (cube mobs are ageable mobs in 26.2) changes nothing:
        // `AbstractCubeMob.getDefaultDimensions` scales the type's size by the size only.
        metadata(&mut t, eid, W::new().u8(size).varint(1).varint(3));
        assert_eq!(
            t.get(eid).unwrap().size(),
            Some((w * 3.0, w * 3.0)),
            "{name}"
        );
        metadata(&mut t, eid, W::new().u8(baby).varint(8).bool(true));
        assert_eq!(
            t.get(eid).unwrap().size(),
            Some((w * 3.0, w * 3.0)),
            "{name}"
        );
    }
    // Scale 2 on a zombie through the 26.2 attribute id: applied in the entity's next tick.
    spawn(&mut t, 13, ty("zombie"), [0.0, 64.0, 0.0]);
    let attrs = W::new()
        .varint(13)
        .varint(1)
        .varint(v26_2::SCALE_ATTRIBUTE)
        .f64(2.0)
        .varint(0);
    t.apply(&attrs.raw(id::ENTITY_UPDATE_ATTRIBUTES)).unwrap();
    t.tick();
    assert_eq!(t.get(13).unwrap().size(), Some((0.6 * 2.0, 1.95 * 2.0)));
}

const FIXTURE: &[u8] = include_bytes!("../../ghast-protocol/tests/fixtures/capture_26_2.bin.zz");

/// Every clientbound play packet of the real capture through the tracker (metadata on and off):
/// no errors, spawns at the packet coordinates with known types.
#[test]
fn real_capture_26_2() {
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(FIXTURE)
        .read_to_end(&mut data)
        .unwrap();
    let data = Bytes::from(data);
    for config in [
        Config::default(),
        Config::default()
            .with_metadata(false)
            .with_player_list(false),
    ] {
        let mut t = Tracker::new(config);
        let mut r = Reader::new(&data);
        let (mut spawns, mut applied, mut metas) = (0, 0, 0);
        while r.remaining() > 0 {
            let state = r.read_u8().unwrap();
            let dir = r.read_u8().unwrap();
            let pid = r.read_varint().unwrap();
            let len = r.read_varint().unwrap() as usize;
            let body = r.read_bytes(len).unwrap();
            if state != 4 || dir != 0 {
                continue;
            }
            let raw = RawPacket::new(pid, body);
            let res = t.apply(&raw).unwrap_or_else(|e| panic!("{pid:#x}: {e:?}"));
            if res == Applied::Changed {
                applied += 1;
            }
            match ClientboundPlayPacket::decode_raw(&raw).unwrap() {
                ClientboundPlayPacket::SpawnEntity(p) if res == Applied::Changed => {
                    spawns += 1;
                    let e = t.get(p.entity_id).unwrap();
                    assert_eq!(v26_2::server_position(e), [p.x, p.y, p.z]);
                    assert!(v26_2::entity_type(e).is_some());
                }
                ClientboundPlayPacket::EntityMetadata(_) if res == Applied::Changed => metas += 1,
                _ => {}
            }
            t.tick();
        }
        assert!(spawns > 0, "the capture has summons");
        assert!(metas > 0 && applied > spawns);
        eprintln!(
            "{config:?}: {spawns} spawns, {metas} metadata packets, {applied} changes, {} entities",
            t.len()
        );
    }
}

/// Random and truncated bodies for every packet the tracker wants never panic (errors are fine),
/// and ticking afterwards stays sound.
#[test]
fn garbage_never_panics() {
    let mut t = tracker(Config::default().with_skins(true));
    spawn(&mut t, 10, ty("zombie"), [0.5, 64.0, 0.5]);
    spawn(&mut t, 11, ty("item"), [0.5, 64.0, 0.5]);
    spawn(&mut t, 12, ty("sulfur_cube"), [0.5, 64.0, 0.5]);
    let ids: Vec<i32> = t
        .packet_ids()
        .filter(|&i| i != id::LOGIN && i != id::RESPAWN)
        .collect();
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..20_000 {
        let pid = ids[(next() % ids.len() as u64) as usize];
        let len = (next() % 48) as usize;
        let mut body: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        if !body.is_empty() && next() % 2 == 0 {
            body[0] = [10u8, 11, 12][(next() % 3) as usize]; // aim at the tracked ids
        }
        let _ = t.apply_body(pid, &Bytes::from(body));
        if next() % 16 == 0 {
            t.tick();
        }
    }
    t.tick();
}
