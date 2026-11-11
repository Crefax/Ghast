//! 26.1 known-answer tests, and the real vanilla 26.1.2 capture
//! (`ghast-protocol/tests/fixtures/capture_26_1_2.bin.zz`) through the tracker.

mod common;

use std::io::Read as _;

use bytes::Bytes;
use common::W;
use ghast_entity::v26_1::{self, Tracker};
use ghast_entity::{Applied, Config, EntityClass, EntityType};
use ghast_protocol::codec::Reader;
use ghast_protocol::v26_1::play::clientbound::{ClientboundPlayPacket, id};
use ghast_protocol::{PacketSet, RawPacket};

const SELF: i32 = 1;
const ZOMBIE: i32 = 150;
const PIG: i32 = 100;
const SLIME: i32 = 117;
const ARMOR_STAND: i32 = 5;
const ITEM: i32 = 71;
const PLAYER: i32 = 155;

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

fn spawn(t: &mut Tracker, eid: i32, ty: i32, p: [f64; 3], rot: [i8; 3]) -> Applied {
    let w = W::new()
        .varint(eid)
        .uuid(0x1000 + eid as u128)
        .varint(ty)
        .f64(p[0])
        .f64(p[1])
        .f64(p[2])
        .u8(0) // zero LpVec3
        .i8(rot[0])
        .i8(rot[1])
        .i8(rot[2])
        .varint(0);
    t.apply(&w.raw(id::SPAWN_ENTITY)).unwrap()
}

fn metadata(t: &mut Tracker, eid: i32, entries: W) {
    let w = W::new().varint(eid).bytes(&entries.raw(0).body).u8(0xFF);
    assert_eq!(t.apply(&w.raw(id::ENTITY_METADATA)), Ok(Applied::Changed));
}

#[test]
fn spawn_rotations_and_types() {
    let mut t = tracker(Config::default());
    assert_eq!(
        spawn(&mut t, 10, ZOMBIE, [0.5, 64.0, 0.5], [32, 64, -64]),
        Applied::Changed
    );
    let z = t.get(10).unwrap();
    assert_eq!(z.class(), EntityClass::Mob);
    assert_eq!(z.position(), [0.5, 64.0, 0.5]);
    assert_eq!((z.pitch(), z.yaw(), z.head_yaw()), (45.0, 90.0, -90.0));
    assert_eq!(v26_1::body_yaw(z), -90.0);
    // Zombie: 0.6 x 1.95 (the box is the centred dimensions box).
    let b = z.bounding_box().unwrap();
    assert_eq!(
        b,
        [
            0.5 - f64::from(0.6f32 / 2.0),
            64.0,
            0.5 - f64::from(0.6f32 / 2.0),
            0.5 + f64::from(0.6f32 / 2.0),
            64.0 + f64::from(1.95f32),
            0.5 + f64::from(0.6f32 / 2.0)
        ]
    );
    assert!(t.pickable(z));
    // A non-living entity: pitch clamped, no head.
    spawn(&mut t, 11, ITEM, [1.0, 64.0, 1.0], [127, 0, 0]);
    let i = t.get(11).unwrap();
    assert_eq!(i.class(), EntityClass::Item);
    assert_eq!(i.pitch(), 90.0);
    assert!(!t.pickable(i));
    // Unknown type ids spawn nothing.
    assert_eq!(spawn(&mut t, 12, 4000, [0.0; 3], [0; 3]), Applied::Ignored);
    // A player needs its tab list entry first.
    assert_eq!(
        spawn(&mut t, 13, PLAYER, [0.0; 3], [0; 3]),
        Applied::Ignored
    );
    let info = W::new()
        .u8(0x01 | 0x04)
        .varint(1)
        .uuid(0x1000 + 13)
        .str("someone")
        .varint(0)
        .varint(3);
    t.apply(&info.raw(id::PLAYER_INFO)).unwrap();
    assert_eq!(
        spawn(&mut t, 13, PLAYER, [0.0, 64.0, 0.0], [0; 3]),
        Applied::Changed
    );
    let p = t.get(13).unwrap();
    assert_eq!(p.entity_type(), EntityType::Player);
    assert_eq!(t.player_name(13), Some("someone"));
    assert!(!t.pickable(p), "spectators are not picked");
    // Same id again: replaced; same UUID under another id: refused.
    assert_eq!(
        spawn(&mut t, 10, PIG, [2.0, 64.0, 2.0], [0; 3]),
        Applied::Changed
    );
    assert_eq!(
        v26_1::info(t.get(10).unwrap().entity_type()).unwrap().name,
        "pig"
    );
    let dup = W::new()
        .varint(14)
        .uuid(0x1000 + 13)
        .varint(PIG)
        .f64(0.0)
        .f64(0.0)
        .f64(0.0)
        .u8(0)
        .i8(0)
        .i8(0)
        .i8(0)
        .varint(0);
    assert_eq!(t.apply(&dup.raw(id::SPAWN_ENTITY)), Ok(Applied::Ignored));
    assert!(t.get(14).is_none());
}

#[test]
fn moves_interpolate_over_three_ticks() {
    let mut t = tracker(Config::default());
    spawn(&mut t, 10, ZOMBIE, [0.0, 64.0, 0.0], [0; 3]);
    // +1 block in x (4096 units): the packet only sets the target.
    let mv = W::new().varint(10).i16(4096).i16(0).i16(0).bool(true);
    t.apply(&mv.raw(id::REL_ENTITY_MOVE)).unwrap();
    let z = t.get(10).unwrap();
    assert_eq!(z.position(), [0.0, 64.0, 0.0]);
    assert_eq!(v26_1::server_position(z), [1.0, 64.0, 0.0]);
    assert!(z.on_ground());
    let mut xs = Vec::new();
    for _ in 0..4 {
        t.tick();
        xs.push(t.get(10).unwrap().position()[0]);
    }
    let a = 0.0 + (1.0 / 3.0) * (1.0 - 0.0);
    let b = a + 0.5 * (1.0 - a);
    assert_eq!(xs, vec![a, b, 1.0, 1.0]);
    // An item moves at once.
    spawn(&mut t, 11, ITEM, [0.0, 64.0, 0.0], [0; 3]);
    t.apply(
        &W::new()
            .varint(11)
            .i16(-2048)
            .i16(0)
            .i16(0)
            .bool(false)
            .raw(id::REL_ENTITY_MOVE),
    )
    .unwrap();
    assert_eq!(t.get(11).unwrap().position(), [-0.5, 64.0, 0.0]);
}

#[test]
fn sync_snaps_far_and_teleports_are_relative_to_the_target() {
    let mut t = tracker(Config::default());
    spawn(&mut t, 10, ZOMBIE, [0.0, 64.0, 0.0], [0; 3]);
    let sync = |x: f64| {
        W::new()
            .varint(10)
            .f64(x)
            .f64(64.0)
            .f64(0.0)
            .f64(0.0)
            .f64(0.0)
            .f64(0.0)
            .f32(10.0)
            .f32(5.0)
            .bool(false)
            .raw(id::SYNC_ENTITY_POSITION)
    };
    t.apply(&sync(70.0)).unwrap();
    assert_eq!(
        t.get(10).unwrap().position(),
        [70.0, 64.0, 0.0],
        "> 64 blocks snaps"
    );
    t.apply(&sync(72.0)).unwrap();
    assert_eq!(
        t.get(10).unwrap().position(),
        [70.0, 64.0, 0.0],
        "near interpolates"
    );
    // Relative x teleport of +1 from the target (72), absolute y / z.
    let tp = W::new()
        .varint(10)
        .f64(1.0)
        .f64(70.0)
        .f64(3.0)
        .f64(0.0)
        .f64(0.0)
        .f64(0.0)
        .f32(0.0)
        .f32(0.0)
        .i32(1)
        .bool(true)
        .raw(id::ENTITY_TELEPORT);
    t.apply(&tp).unwrap();
    let z = t.get(10).unwrap();
    assert_eq!(v26_1::interpolation_target(z).unwrap().0, [73.0, 70.0, 3.0]);
    assert_eq!(
        v26_1::server_position(z),
        [72.0, 64.0, 0.0],
        "teleports keep the codec base"
    );
}

#[test]
fn size_metadata_and_scale() {
    let mut t = tracker(Config::default().with_metadata(false));
    spawn(&mut t, 10, PIG, [0.0, 64.0, 0.0], [0; 3]);
    let adult = t.get(10).unwrap().size().unwrap();
    metadata(&mut t, 10, W::new().u8(16).varint(8).bool(true));
    let baby = t.get(10).unwrap().size().unwrap();
    assert!(baby.0 < adult.0 && baby.1 < adult.1);
    // Sleeping pose: the living entity constant.
    metadata(&mut t, 10, W::new().u8(6).varint(20).varint(2));
    assert_eq!(t.get(10).unwrap().size(), Some((0.2, 0.2)));
    // Slime size 3.
    spawn(&mut t, 11, SLIME, [0.0, 64.0, 0.0], [0; 3]);
    metadata(&mut t, 11, W::new().u8(16).varint(1).varint(3));
    assert_eq!(t.get(11).unwrap().size(), Some((0.52 * 3.0, 0.52 * 3.0)));
    // Armor stand marker: no size, not pickable.
    spawn(&mut t, 12, ARMOR_STAND, [0.0, 64.0, 0.0], [0; 3]);
    metadata(&mut t, 12, W::new().u8(15).varint(0).u8(0x10));
    assert_eq!(t.get(12).unwrap().size(), Some((0.0, 0.0)));
    assert!(!t.pickable(t.get(12).unwrap()));
    // Scale 2 on the zombie: applied in the entity's next tick.
    spawn(&mut t, 13, ZOMBIE, [0.0, 64.0, 0.0], [0; 3]);
    let attrs = W::new()
        .varint(13)
        .varint(1)
        .varint(v26_1::SCALE_ATTRIBUTE)
        .f64(2.0)
        .varint(0);
    t.apply(&attrs.raw(id::ENTITY_UPDATE_ATTRIBUTES)).unwrap();
    assert_eq!(t.get(13).unwrap().size(), Some((0.6, 1.95)));
    t.tick();
    assert_eq!(t.get(13).unwrap().size(), Some((0.6 * 2.0, 1.95 * 2.0)));
    assert_eq!(v26_1::scale(t.get(13).unwrap()), 2.0);
}

#[test]
fn chunks_respawn_and_pickups() {
    let mut t = tracker(Config::default());
    spawn(&mut t, 10, ZOMBIE, [40.0, 64.0, 40.0], [0; 3]);
    let z = t.get(10).unwrap();
    assert!(t.is_ticking(z));
    let key = 2u64 | (2u64 << 32);
    t.apply(&W::new().i64(key as i64).raw(id::UNLOAD_CHUNK))
        .unwrap();
    assert!(!t.is_ticking(t.get(10).unwrap()));
    t.apply(
        &W::new()
            .varint(10)
            .i16(4096)
            .i16(0)
            .i16(0)
            .bool(false)
            .raw(id::REL_ENTITY_MOVE),
    )
    .unwrap();
    t.tick();
    assert_eq!(
        t.get(10).unwrap().position()[0],
        40.0,
        "no interpolation in an unloaded chunk"
    );
    // A floating item with 5 in its stack: picking up 2 leaves 3, then 3 more removes it.
    spawn(&mut t, 11, ITEM, [0.0, 64.0, 0.0], [0; 3]);
    metadata(
        &mut t,
        11,
        W::new()
            .u8(8)
            .varint(7)
            .varint(5)
            .varint(1)
            .varint(0)
            .varint(0),
    );
    assert_eq!(v26_1::item_count(t.get(11).unwrap()), 5);
    assert_eq!(v26_1::item_stack(t.get(11).unwrap()).unwrap().id, 1);
    let take = |n: i32| W::new().varint(11).varint(SELF).varint(n).raw(id::COLLECT);
    t.apply(&take(2)).unwrap();
    assert_eq!(v26_1::item_count(t.get(11).unwrap()), 3);
    t.apply(&take(3)).unwrap();
    assert!(t.get(11).is_none());
    // An item whose stack never arrived is discarded in its first tick.
    spawn(&mut t, 12, ITEM, [0.0, 64.0, 0.0], [0; 3]);
    t.tick();
    assert!(t.get(12).is_none());
    // Respawn into another dimension clears everything but the bot.
    let respawn = W::new()
        .varint(1)
        .str("minecraft:the_nether")
        .i64(0)
        .i8(0)
        .u8(0xFF)
        .bool(false)
        .bool(false)
        .bool(false)
        .varint(0)
        .varint(32)
        .u8(0);
    t.apply(&respawn.raw(id::RESPAWN)).unwrap();
    assert_eq!(t.iter().count(), 0);
    assert_eq!(t.self_id(), Some(SELF));
}

const FIXTURE: &[u8] = include_bytes!("../../ghast-protocol/tests/fixtures/capture_26_1_2.bin.zz");

/// Every clientbound play packet of the real capture through the tracker (metadata on and off):
/// no errors, spawns at the packet coordinates, the tab list holds the captured player.
#[test]
fn real_capture_26_1_2() {
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
                    assert_eq!(v26_1::server_position(e), [p.x, p.y, p.z]);
                    assert!(v26_1::entity_type(e).is_some());
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
    spawn(&mut t, 10, ZOMBIE, [0.5, 64.0, 0.5], [0; 3]);
    spawn(&mut t, 11, ITEM, [0.5, 64.0, 0.5], [0; 3]);
    let ids: Vec<i32> = t.packet_ids().filter(|&i| i != id::LOGIN && i != id::RESPAWN).collect();
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
            body[0] = [10u8, 11][(next() % 2) as usize]; // aim at the tracked ids
        }
        let _ = t.apply_body(pid, &Bytes::from(body));
        if next() % 16 == 0 {
            t.tick();
        }
    }
    t.tick();
}
