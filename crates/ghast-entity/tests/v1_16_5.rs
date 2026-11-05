//! 1.16.5 known-answer tests.

mod common;

use common::{W, player_add};
use ghast_entity::v1_16_5::{self, Tracker, meta};
use ghast_entity::{Applied, Config, EntityType, MetaValue};
use ghast_protocol::v1_16_5::play::clientbound::id;

const SELF: i32 = 1;

fn tracker(config: Config) -> Tracker {
    let mut t = Tracker::new(config);
    // login: entity id, hardcore, gamemode, previous gamemode, world names, codec, dimension, ...
    let w = W::new()
        .i32(SELF)
        .bool(false)
        .u8(0)
        .u8(0xFF)
        .varint(1)
        .str("minecraft:overworld")
        .bytes(&[10, 0, 0, 0]) // empty compound
        .bytes(&[10, 0, 0, 0])
        .str("minecraft:overworld")
        .i64(0)
        .varint(20)
        .varint(10)
        .bool(false)
        .bool(true)
        .bool(false)
        .bool(false);
    assert_eq!(t.apply(&w.raw(id::LOGIN)), Ok(Applied::Changed));
    t
}

fn spawn_living(t: &mut Tracker, eid: i32, ty: i32, p: [f64; 3], v: [i16; 3]) {
    let w = W::new()
        .varint(eid)
        .uuid(u128::from(eid as u32))
        .varint(ty)
        .f64(p[0])
        .f64(p[1])
        .f64(p[2])
        .i8(64)
        .i8(0)
        .i8(-64)
        .i16(v[0])
        .i16(v[1])
        .i16(v[2]);
    assert_eq!(
        t.apply(&w.raw(id::SPAWN_ENTITY_LIVING)),
        Ok(Applied::Changed)
    );
}

fn spawn_object(t: &mut Tracker, eid: i32, ty: i32, p: [f64; 3], data: i32, v: [i16; 3]) {
    let w = W::new()
        .varint(eid)
        .uuid(7)
        .varint(ty)
        .f64(p[0])
        .f64(p[1])
        .f64(p[2])
        .i8(0)
        .i8(0)
        .i32(data)
        .i16(v[0])
        .i16(v[1])
        .i16(v[2]);
    assert_eq!(t.apply(&w.raw(id::SPAWN_ENTITY)), Ok(Applied::Changed));
}

fn rel(eid: i32, d: [i16; 3]) -> ghast_protocol::RawPacket {
    W::new()
        .varint(eid)
        .i16(d[0])
        .i16(d[1])
        .i16(d[2])
        .bool(true)
        .raw(id::REL_ENTITY_MOVE)
}

#[test]
fn fixed_point_relative_moves() {
    let mut t = tracker(Config::default());
    spawn_living(&mut t, 7, 102, [0.1, 64.0, -0.1], [1, 0, 0]);
    let e = t.get(7).unwrap();
    assert_eq!(v1_16_5::info(e.entity_type()).unwrap().name, "zombie");
    assert_eq!(e.position(), [0.1, 64.0, -0.1]);
    assert_eq!(
        e.velocity()[0],
        f64::from(1.0f32 / 8000.0),
        "mob spawn divides in float"
    );
    assert_eq!((e.yaw(), e.head_yaw()), (90.0, -90.0));
    // Delta 0 keeps the exact double; non-zero deltas go through floor(x * 4096).
    t.apply(&rel(7, [0, 0, 0])).unwrap();
    assert_eq!(t.get(7).unwrap().position(), [0.1, 64.0, -0.1]);
    t.apply(&rel(7, [1, -4096, -5])).unwrap();
    // floor(0.1 * 4096) = 409; floor(-0.1 * 4096) = floor(-409.6) = -410.
    assert_eq!(
        t.get(7).unwrap().position(),
        [410.0 / 4096.0, 63.0, -415.0 / 4096.0]
    );
    // move_look / look.
    let w = W::new()
        .varint(7)
        .i16(0)
        .i16(1)
        .i16(0)
        .i8(-128)
        .i8(1)
        .bool(false)
        .raw(id::ENTITY_MOVE_LOOK);
    t.apply(&w).unwrap();
    let e = t.get(7).unwrap();
    assert_eq!(e.position()[1], (63 * 4096 + 1) as f64 / 4096.0);
    assert_eq!(
        (e.yaw(), e.pitch(), e.on_ground()),
        (-180.0, 1.40625, false)
    );
    // Teleport sets the exact doubles.
    let w = W::new()
        .varint(7)
        .f64(1e7 + 0.123)
        .f64(-3.3)
        .f64(5.5)
        .i8(0)
        .i8(0)
        .bool(true)
        .raw(id::ENTITY_TELEPORT);
    t.apply(&w).unwrap();
    assert_eq!(t.get(7).unwrap().position(), [1e7 + 0.123, -3.3, 5.5]);
    // Velocity divides in double.
    let w = W::new()
        .varint(7)
        .i16(-1)
        .i16(8000)
        .i16(0)
        .raw(id::ENTITY_VELOCITY);
    t.apply(&w).unwrap();
    assert_eq!(t.get(7).unwrap().velocity(), [-1.0 / 8000.0, 1.0, 0.0]);
    // Bare `entity` packet clears on_ground.
    t.apply(&W::new().varint(7).raw(id::ENTITY)).unwrap();
    assert!(!t.get(7).unwrap().on_ground());
}

#[test]
fn spawns_and_unknown_types() {
    let mut t = tracker(Config::default());
    spawn_object(&mut t, 10, 2, [1.0, 2.0, 3.0], 0, [800, 0, -800]);
    let e = t.get(10).unwrap();
    assert_eq!(e.entity_type(), EntityType::Registry(2));
    assert_eq!(e.velocity(), [0.1, 0.0, -0.1]);
    let w = W::new()
        .varint(11)
        .uuid(1)
        .varint(5000)
        .f64(0.0)
        .f64(0.0)
        .f64(0.0)
        .i8(0)
        .i8(0)
        .i32(0)
        .i16(0)
        .i16(0)
        .i16(0);
    assert_eq!(t.apply(&w.raw(id::SPAWN_ENTITY)), Ok(Applied::Ignored));
    // Player, orb, painting.
    let uuid = 0xAAu128 << 64 | 0x55;
    t.apply(&player_add(uuid, "Alice", true).raw(id::PLAYER_INFO))
        .unwrap();
    let w = W::new()
        .varint(12)
        .uuid(uuid)
        .f64(0.5)
        .f64(70.0)
        .f64(-0.5)
        .i8(32)
        .i8(0);
    t.apply(&w.raw(id::NAMED_ENTITY_SPAWN)).unwrap();
    assert_eq!(t.player_name(12), Some("Alice"));
    assert_eq!(t.get(12).unwrap().yaw(), 45.0);
    let w = W::new().varint(13).f64(1.0).f64(2.0).f64(3.0).i16(17);
    t.apply(&w.raw(id::SPAWN_ENTITY_EXPERIENCE_ORB)).unwrap();
    assert_eq!(t.get(13).unwrap().entity_type(), EntityType::Registry(24));
    assert_eq!(t.get(13).unwrap().data(), 17);
    let pos = ((10i64 & 0x3FF_FFFF) << 38) | ((-5i64 & 0x3FF_FFFF) << 12) | (64 & 0xFFF);
    let w = W::new().varint(14).uuid(3).varint(4).i64(pos).u8(1);
    t.apply(&w.raw(id::SPAWN_ENTITY_PAINTING)).unwrap();
    assert_eq!(t.get(14).unwrap().position(), [10.0, 64.0, -5.0]);
}

/// Every metadata serializer 0..=18.
#[test]
fn metadata_all_serializers() {
    let mut t = tracker(Config::default());
    spawn_living(&mut t, 7, 102, [0.0; 3], [0; 3]);
    let pos = ((1i64) << 38) | ((3i64) << 12) | 2;
    let w = W::new()
        .varint(7)
        .u8(0)
        .varint(0)
        .i8(0x22)
        .u8(1)
        .varint(1)
        .varint(300)
        .u8(8)
        .varint(2)
        .f32(12.5)
        .u8(9)
        .varint(3)
        .str("s")
        .u8(10)
        .varint(4)
        .str("{}")
        .u8(2)
        .varint(5)
        .bool(true)
        .str("{\"text\":\"N\"}")
        .u8(11)
        .varint(6)
        .bool(true)
        .varint(1)
        .i8(2)
        .u8(0)
        .u8(12)
        .varint(7)
        .bool(true)
        .u8(13)
        .varint(8)
        .f32(1.0)
        .f32(2.0)
        .f32(3.0)
        .u8(14)
        .varint(9)
        .i64(pos)
        .u8(15)
        .varint(10)
        .bool(true)
        .i64(pos)
        .u8(16)
        .varint(11)
        .varint(3)
        .u8(17)
        .varint(12)
        .bool(true)
        .uuid(9)
        .u8(18)
        .varint(13)
        .varint(1)
        .u8(19)
        .varint(14)
        .bytes(&[10, 0, 0, 0])
        .u8(20)
        .varint(15)
        .varint(3)
        .varint(1) // particle 3 (block) with a block state
        .u8(21)
        .varint(16)
        .varint(1)
        .varint(2)
        .varint(3)
        .u8(22)
        .varint(17)
        .varint(5)
        .u8(23)
        .varint(18)
        .varint(1)
        .u8(0xFF);
    assert_eq!(t.apply(&w.raw(id::ENTITY_METADATA)), Ok(Applied::Changed));
    let e = t.get(7).unwrap();
    let m = e.metadata();
    assert_eq!(m.len(), 19);
    assert_eq!(v1_16_5::flags(e), 0x22);
    assert_eq!(v1_16_5::health(e), Some(12.5));
    assert_eq!(v1_16_5::custom_name(e), Some("{\"text\":\"N\"}"));
    assert_eq!(m.get(1), Some(&MetaValue::VarInt(300)));
    assert_eq!(m.item(11).unwrap().unwrap().id, 1);
    assert_eq!(m.get(14), Some(&MetaValue::BlockPos([1, 2, 3])));
    assert_eq!(m.get(15), Some(&MetaValue::OptBlockPos(Some([1, 2, 3]))));
    assert_eq!(m.get(20), Some(&MetaValue::Particle(3)));
    assert_eq!(m.get(21), Some(&MetaValue::VillagerData([1, 2, 3])));
    assert_eq!(m.get(23), Some(&MetaValue::Pose(1)));
    // Unknown serializer: error, nothing applied.
    let w = W::new()
        .varint(7)
        .u8(8)
        .varint(2)
        .f32(1.0)
        .u8(30)
        .varint(99)
        .u8(0xFF);
    assert!(t.apply(&w.raw(id::ENTITY_METADATA)).is_err());
    assert_eq!(v1_16_5::health(t.get(7).unwrap()), Some(12.5));
}

fn item_meta(eid: i32, count: i8) -> ghast_protocol::RawPacket {
    W::new()
        .varint(eid)
        .u8(meta::ITEM)
        .varint(6)
        .bool(true)
        .varint(1)
        .i8(count)
        .u8(0)
        .u8(0xFF)
        .raw(id::ENTITY_METADATA)
}

#[test]
fn pickup_shrinks_item_stacks() {
    for metadata in [true, false] {
        let mut t = tracker(Config::default().with_metadata(metadata));
        spawn_object(&mut t, 10, 37, [0.0; 3], 1, [0; 3]);
        t.apply(&item_meta(10, 5)).unwrap();
        assert_eq!(v1_16_5::item(t.get(10).unwrap()).unwrap().count, 5);
        let collect = |n: i32| W::new().varint(10).varint(SELF).varint(n).raw(id::COLLECT);
        t.apply(&collect(2)).unwrap();
        assert_eq!(v1_16_5::item(t.get(10).unwrap()).unwrap().count, 3);
        t.apply(&collect(3)).unwrap();
        assert!(t.get(10).is_none());
        // XP orbs and arrows go at once.
        spawn_object(&mut t, 11, 2, [0.0; 3], 0, [0; 3]);
        t.apply(&W::new().varint(11).varint(SELF).varint(1).raw(id::COLLECT))
            .unwrap();
        assert!(t.get(11).is_none());
        // Without metadata only item entities keep their stack.
        spawn_living(&mut t, 12, 102, [0.0; 3], [0; 3]);
        let w = W::new().varint(12).u8(8).varint(2).f32(5.0).u8(0xFF);
        t.apply(&w.raw(id::ENTITY_METADATA)).unwrap();
        assert_eq!(v1_16_5::health(t.get(12).unwrap()).is_some(), metadata);
    }
}

#[test]
fn passengers_leash_and_controlled_boat() {
    let mut t = tracker(Config::default());
    spawn_object(&mut t, 20, 6, [0.0, 64.0, 0.0], 0, [0; 3]); // boat
    spawn_living(&mut t, 21, 59, [0.0; 3], [0; 3]); // pig
    let pass = |v: i32, ids: &[i32]| {
        let mut w = W::new().varint(v).varint(ids.len() as i32);
        for &i in ids {
            w = w.varint(i);
        }
        w.raw(id::SET_PASSENGERS)
    };
    t.apply(&pass(20, &[21])).unwrap();
    assert_eq!(t.get(20).unwrap().passengers(), &[21]);
    t.apply(&pass(20, &[SELF, 21])).unwrap();
    assert_eq!(t.get(20).unwrap().passengers(), &[SELF, 21]);
    assert_eq!(t.me().unwrap().vehicle(), Some(20));
    // The bot steers the boat: relative moves are ignored, teleports set the packet position
    // but not the rotation.
    assert_eq!(t.apply(&rel(20, [4096, 0, 0])), Ok(Applied::Ignored));
    assert_eq!(t.get(20).unwrap().position(), [0.0, 64.0, 0.0]);
    let w = W::new()
        .varint(20)
        .f64(5.0)
        .f64(64.0)
        .f64(1.0)
        .i8(64)
        .i8(0)
        .bool(true);
    t.apply(&w.raw(id::ENTITY_TELEPORT)).unwrap();
    assert_eq!(t.get(20).unwrap().position(), [5.0, 64.0, 1.0]);
    assert_eq!(t.get(20).unwrap().yaw(), 0.0);
    // Pig as first passenger: the boat moves normally again.
    t.apply(&pass(20, &[21, SELF])).unwrap();
    t.apply(&rel(20, [4096, 0, 0])).unwrap();
    assert_eq!(t.get(20).unwrap().position(), [6.0, 64.0, 1.0]);
    t.apply(&pass(20, &[])).unwrap();
    assert_eq!(t.me().unwrap().vehicle(), None);
    // Leash: holder id, 0 = none.
    t.apply(&W::new().i32(21).i32(SELF).raw(id::ATTACH_ENTITY))
        .unwrap();
    assert_eq!(t.get(21).unwrap().leash_holder(), Some(SELF));
    t.apply(&W::new().i32(21).i32(0).raw(id::ATTACH_ENTITY))
        .unwrap();
    assert_eq!(t.get(21).unwrap().leash_holder(), None);
}

#[test]
fn equipment_list_and_effects() {
    let mut t = tracker(Config::default());
    spawn_living(&mut t, 7, 102, [0.0; 3], [0; 3]);
    // main hand (0, more follow), head (5, last).
    let w = W::new()
        .varint(7)
        .u8(0x80)
        .bool(true)
        .varint(600)
        .i8(1)
        .u8(0)
        .u8(5)
        .bool(true)
        .varint(601)
        .i8(1)
        .u8(0);
    t.apply(&w.raw(id::ENTITY_EQUIPMENT)).unwrap();
    let e = t.get(7).unwrap();
    assert_eq!(
        (e.equipment(0).unwrap().id, e.equipment(5).unwrap().id),
        (600, 601)
    );
    assert!(e.equipment(1).is_none());
    let w = W::new().varint(7).i8(1).i8(1).varint(100).i8(6);
    t.apply(&w.raw(id::ENTITY_EFFECT)).unwrap();
    assert_eq!(t.get(7).unwrap().effect(1).unwrap().flags, 6);
}

fn respawn(name: &str) -> ghast_protocol::RawPacket {
    W::new()
        .bytes(&[10, 0, 0, 0])
        .str(name)
        .i64(0)
        .u8(0)
        .u8(0xFF)
        .bool(false)
        .bool(false)
        .bool(true)
        .raw(id::RESPAWN)
}

#[test]
fn respawn_rules() {
    let mut t = tracker(Config::default());
    spawn_living(&mut t, 7, 102, [0.0; 3], [0; 3]);
    let w = W::new().varint(SELF).i8(1).i8(1).varint(100).i8(0);
    t.apply(&w.raw(id::ENTITY_EFFECT)).unwrap();
    let w = W::new().varint(SELF).u8(0).varint(0).i8(2).u8(0xFF);
    t.apply(&w.raw(id::ENTITY_METADATA)).unwrap();
    t.apply(&respawn("minecraft:overworld")).unwrap();
    assert!(t.get(7).is_some());
    let me = t.me().unwrap();
    assert!(me.effects().is_empty());
    assert_eq!(
        me.metadata().byte(0),
        Some(2),
        "1.16 copies the player's metadata"
    );
    t.apply(&respawn("minecraft:the_nether")).unwrap();
    assert!(t.get(7).is_none());
}

#[test]
fn class_item_and_box() {
    use ghast_entity::EntityClass;
    let mut t = tracker(Config::default().with_metadata(false));
    spawn_living(&mut t, 7, 102, [1.0, 64.0, 1.0], [0; 3]);
    spawn_object(&mut t, 8, 37, [0.0; 3], 1, [0; 3]);
    spawn_object(&mut t, 9, 6, [0.0; 3], 0, [0; 3]);
    t.apply(&item_meta(8, 3)).unwrap();
    assert_eq!(t.get(7).unwrap().class(), EntityClass::Mob);
    assert_eq!(t.get(8).unwrap().class(), EntityClass::Item);
    assert_eq!(t.get(8).unwrap().item().unwrap().count, 3);
    assert_eq!(t.get(9).unwrap().class(), EntityClass::Object);
    assert_eq!(t.get(7).unwrap().size(), Some((0.6, 1.95)));
    let b = t.get(7).unwrap().bounding_box().unwrap();
    assert_eq!(b[1], 64.0);
    assert!((b[0] - 0.7).abs() < 1e-6 && (b[4] - 65.95).abs() < 1e-6);
    // Metadata off: variants need metadata, so this tracker keeps the adult size.
    let mut t = tracker(Config::default());
    spawn_living(&mut t, 7, 102, [0.0; 3], [0; 3]);
    let baby = W::new().varint(7).u8(15).varint(7).bool(true).u8(0xFF);
    t.apply(&baby.raw(id::ENTITY_METADATA)).unwrap();
    assert_eq!(
        t.get(7).unwrap().size(),
        Some((0.3, 0.975)),
        "1.16 baby zombie"
    );
    // Slime size 3, small armor stand, crouching / swimming player.
    spawn_living(&mut t, 8, 75, [0.0; 3], [0; 3]);
    t.apply(
        &W::new()
            .varint(8)
            .u8(15)
            .varint(1)
            .varint(3)
            .u8(0xFF)
            .raw(id::ENTITY_METADATA),
    )
    .unwrap();
    let s3 = ghast_data::v1_16_5::slime_dimensions(3);
    assert_eq!(t.get(8).unwrap().size(), Some((s3.width, s3.height)));
    spawn_object(&mut t, 9, 1, [0.0; 3], 0, [0; 3]);
    t.apply(
        &W::new()
            .varint(9)
            .u8(14)
            .varint(0)
            .i8(1)
            .u8(0xFF)
            .raw(id::ENTITY_METADATA),
    )
    .unwrap();
    assert_eq!(t.get(9).unwrap().size(), Some((0.25, 0.9875)));
    let w = W::new()
        .varint(12)
        .uuid(5)
        .f64(0.0)
        .f64(0.0)
        .f64(0.0)
        .i8(0)
        .i8(0);
    t.apply(&w.raw(id::NAMED_ENTITY_SPAWN)).unwrap();
    assert_eq!(t.get(12).unwrap().size(), Some((0.6, 1.8)));
    t.apply(
        &W::new()
            .varint(12)
            .u8(6)
            .varint(18)
            .varint(5)
            .u8(0xFF)
            .raw(id::ENTITY_METADATA),
    )
    .unwrap();
    assert_eq!(t.get(12).unwrap().size(), Some((0.6, 1.5)), "crouching");
    t.apply(
        &W::new()
            .varint(12)
            .u8(6)
            .varint(18)
            .varint(3)
            .u8(0xFF)
            .raw(id::ENTITY_METADATA),
    )
    .unwrap();
    assert_eq!(t.get(12).unwrap().size(), Some((0.6, 0.6)), "swimming");
    // Paintings have no box from the size table.
    let pos = ((10i64 & 0x3FF_FFFF) << 38) | ((-5i64 & 0x3FF_FFFF) << 12) | (64 & 0xFFF);
    t.apply(
        &W::new()
            .varint(14)
            .uuid(3)
            .varint(4)
            .i64(pos)
            .u8(1)
            .raw(id::SPAWN_ENTITY_PAINTING),
    )
    .unwrap();
    assert_eq!(t.get(14).unwrap().bounding_box(), None);
    assert_eq!(t.near([10.0, 64.0, -5.0], 0.5).count(), 1);
}
