//! 1.12.2 known-answer tests (formulas from the vanilla 1.12.2 client's packet handlers).

mod common;

use common::{W, player_add};
use ghast_entity::v1_12::{self, Tracker, meta};
use ghast_entity::{Applied, Config, EntityClass, EntityType, MetaValue};
use ghast_protocol::v1_12_2::play::clientbound::id;

const SELF: i32 = 1;

fn login(dim: i32) -> ghast_protocol::RawPacket {
    // entity id, game mode, dimension (int), difficulty, max players, level type, reduced debug.
    W::new()
        .i32(SELF)
        .u8(0)
        .i32(dim)
        .u8(2)
        .u8(20)
        .str("default")
        .bool(false)
        .raw(id::LOGIN)
}

fn tracker(config: Config) -> Tracker {
    let mut t = Tracker::new(config);
    assert_eq!(t.apply(&login(0)), Ok(Applied::Changed));
    t
}

/// Mob spawn with empty metadata.
fn spawn_mob(t: &mut Tracker, eid: i32, ty: i32, p: [f64; 3], v: [i16; 3]) -> Applied {
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
        .i16(v[2])
        .u8(0xFF);
    t.apply(&w.raw(id::SPAWN_ENTITY_LIVING)).unwrap()
}

fn spawn_object(t: &mut Tracker, eid: i32, ty: i8, p: [f64; 3], data: i32, v: [i16; 3]) -> Applied {
    let w = W::new()
        .varint(eid)
        .uuid(7)
        .i8(ty)
        .f64(p[0])
        .f64(p[1])
        .f64(p[2])
        .i8(0)
        .i8(0)
        .i32(data)
        .i16(v[0])
        .i16(v[1])
        .i16(v[2]);
    t.apply(&w.raw(id::SPAWN_ENTITY)).unwrap()
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
fn tracked_fixed_point_moves() {
    let mut t = tracker(Config::default());
    assert_eq!(
        spawn_mob(&mut t, 7, 54, [0.1, 64.0, -0.1], [1, 0, 0]),
        Applied::Changed
    );
    let e = t.get(7).unwrap();
    assert_eq!(e.entity_type(), EntityType::LegacyMob(54));
    assert_eq!(v1_12::info(e.entity_type()).unwrap().name, "zombie");
    assert_eq!(e.position(), [0.1, 64.0, -0.1]);
    assert_eq!(
        e.velocity()[0],
        f64::from(1.0f32 / 8000.0),
        "mob spawn divides in float"
    );
    assert_eq!((e.yaw(), e.head_yaw()), (90.0, -90.0));
    // Unlike 1.16, every axis snaps to the tracked 1/4096 grid, also with delta 0:
    // lfloor(0.1 * 4096) = 409, lfloor(-0.1 * 4096) = -410.
    t.apply(&rel(7, [0, 0, 0])).unwrap();
    assert_eq!(
        t.get(7).unwrap().position(),
        [409.0 / 4096.0, 64.0, -410.0 / 4096.0]
    );
    t.apply(&rel(7, [1, -4096, -5])).unwrap();
    assert_eq!(
        t.get(7).unwrap().position(),
        [410.0 / 4096.0, 63.0, -415.0 / 4096.0]
    );
    // The bare `entity` packet also re-derives the position and clears on_ground.
    t.apply(&W::new().varint(7).raw(id::ENTITY)).unwrap();
    assert!(!t.get(7).unwrap().on_ground());
    // move_look / look.
    let w = W::new()
        .varint(7)
        .i16(0)
        .i16(1)
        .i16(0)
        .i8(-128)
        .i8(1)
        .bool(true)
        .raw(id::ENTITY_MOVE_LOOK);
    t.apply(&w).unwrap();
    let e = t.get(7).unwrap();
    assert_eq!(e.position()[1], (63 * 4096 + 1) as f64 / 4096.0);
    assert_eq!((e.yaw(), e.pitch(), e.on_ground()), (-180.0, 1.40625, true));
    // Teleport: the exact doubles, and the tracked value restarts from them.
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
    t.apply(&rel(7, [0, 0, 1])).unwrap();
    let x = ((1e7_f64 + 0.123) * 4096.0).floor();
    let y = (-3.3f64 * 4096.0).floor();
    assert_eq!(
        t.get(7).unwrap().position(),
        [x / 4096.0, y / 4096.0, (5.5 * 4096.0 + 1.0) / 4096.0]
    );
    // Velocity divides in double.
    let w = W::new()
        .varint(7)
        .i16(-1)
        .i16(8000)
        .i16(0)
        .raw(id::ENTITY_VELOCITY);
    t.apply(&w).unwrap();
    assert_eq!(t.get(7).unwrap().velocity(), [-1.0 / 8000.0, 1.0, 0.0]);
}

#[test]
fn spawns_objects_and_unknown_types() {
    let mut t = tracker(Config::default());
    // Item (object 2): velocity only with positive object data.
    spawn_object(&mut t, 10, 2, [1.0, 2.0, 3.0], 1, [800, 0, -800]);
    let e = t.get(10).unwrap();
    assert_eq!(e.entity_type(), EntityType::LegacyObject(2));
    assert_eq!(e.class(), EntityClass::Item);
    assert_eq!(e.velocity(), [0.1, 0.0, -0.1]);
    spawn_object(&mut t, 11, 60, [0.0; 3], 0, [800, 0, 0]);
    assert_eq!(
        t.get(11).unwrap().velocity(),
        [0.0; 3],
        "arrow without shooter: no velocity"
    );
    // Falling block: data is the block state, reset before the velocity decision.
    spawn_object(&mut t, 12, 70, [0.0; 3], 12 | (1 << 12), [800, 0, 0]);
    assert_eq!(t.get(12).unwrap().velocity(), [0.0; 3]);
    assert_eq!(t.get(12).unwrap().data(), 12 | (1 << 12));
    // Llama spit takes its velocity through the constructor.
    spawn_object(&mut t, 13, 68, [0.0; 3], 0, [0, 8000, 0]);
    assert_eq!(t.get(13).unwrap().velocity(), [0.0, 1.0, 0.0]);
    // Unknown object type and an orphan fishing bobber spawn nothing.
    assert_eq!(
        spawn_object(&mut t, 14, 4, [0.0; 3], 0, [0; 3]),
        Applied::Ignored
    );
    assert_eq!(
        spawn_object(&mut t, 15, 90, [0.0; 3], 99, [0; 3]),
        Applied::Ignored
    );
    assert_eq!(
        spawn_object(&mut t, 15, 90, [0.0; 3], SELF, [0; 3]),
        Applied::Changed
    );
    // Unknown mob id.
    assert_eq!(
        spawn_mob(&mut t, 16, 5000, [0.0; 3], [0; 3]),
        Applied::Ignored
    );
    // Player (metadata in the packet), orb, painting, lightning.
    let uuid = 0xAAu128 << 64 | 0x55;
    t.apply(&player_add(uuid, "Alice", true).raw(id::PLAYER_INFO))
        .unwrap();
    let w = W::new()
        .varint(20)
        .uuid(uuid)
        .f64(0.5)
        .f64(70.0)
        .f64(-0.5)
        .i8(32)
        .i8(0)
        .u8(meta::FLAGS)
        .varint(0)
        .i8(0x02)
        .u8(0xFF);
    t.apply(&w.raw(id::NAMED_ENTITY_SPAWN)).unwrap();
    let p = t.get(20).unwrap();
    assert_eq!(t.player_name(20), Some("Alice"));
    assert_eq!((p.yaw(), p.position()), (45.0, [0.5, 70.0, -0.5]));
    assert_eq!(v1_12::flags(p), 0x02);
    assert_eq!(
        p.size(),
        Some((0.6, 1.65)),
        "sneaking players are 1.65 high"
    );
    let w = W::new().varint(21).f64(1.0).f64(2.0).f64(3.0).i16(17);
    t.apply(&w.raw(id::SPAWN_ENTITY_EXPERIENCE_ORB)).unwrap();
    assert_eq!(t.get(21).unwrap().class(), EntityClass::ExperienceOrb);
    assert_eq!(t.get(21).unwrap().data(), 17);
    let pos = ((10i64 & 0x3FF_FFFF) << 38) | ((64i64 & 0xFFF) << 26) | (-5i64 & 0x3FF_FFFF);
    let w = W::new().varint(22).uuid(3).str("Kebab").i64(pos).u8(1);
    t.apply(&w.raw(id::SPAWN_ENTITY_PAINTING)).unwrap();
    assert_eq!(t.get(22).unwrap().position(), [10.0, 64.0, -5.0]);
    assert_eq!(t.get(22).unwrap().class(), EntityClass::Painting);
    let w = W::new().varint(23).i8(1).f64(0.0).f64(0.0).f64(0.0);
    assert_eq!(
        t.apply(&w.raw(id::SPAWN_ENTITY_WEATHER)),
        Ok(Applied::Ignored)
    );
    assert!(t.get(23).is_none());
}

/// Every 1.12 metadata serializer 0..=13.
#[test]
fn metadata_all_serializers() {
    let mut t = tracker(Config::default());
    spawn_mob(&mut t, 7, 54, [0.0; 3], [0; 3]);
    let pos = (1i64 << 38) | (2i64 << 26) | 3;
    let w = W::new()
        .varint(7)
        .u8(0)
        .varint(0)
        .i8(0x22)
        .u8(1)
        .varint(1)
        .varint(300)
        .u8(7)
        .varint(2)
        .f32(12.5)
        .u8(2)
        .varint(3)
        .str("Bob")
        .u8(20)
        .varint(4)
        .str("{}")
        .u8(21)
        .varint(5)
        .i16(1)
        .i8(2)
        .i16(3)
        .u8(0)
        .u8(12)
        .varint(6)
        .bool(true)
        .u8(13)
        .varint(7)
        .f32(1.0)
        .f32(2.0)
        .f32(3.0)
        .u8(14)
        .varint(8)
        .i64(pos)
        .u8(15)
        .varint(9)
        .bool(true)
        .i64(pos)
        .u8(16)
        .varint(10)
        .varint(3)
        .u8(17)
        .varint(11)
        .bool(true)
        .uuid(9)
        .u8(18)
        .varint(12)
        .varint(1 << 4)
        .u8(19)
        .varint(13)
        .bytes(&[10, 0, 0, 0])
        .u8(0xFF);
    assert_eq!(t.apply(&w.raw(id::ENTITY_METADATA)), Ok(Applied::Changed));
    let e = t.get(7).unwrap();
    let m = e.metadata();
    assert_eq!(m.len(), 14);
    assert_eq!(v1_12::flags(e), 0x22);
    assert_eq!(v1_12::health(e), Some(12.5));
    assert_eq!(v1_12::custom_name(e), Some("Bob"));
    assert_eq!(m.get(1), Some(&MetaValue::VarInt(300)));
    let it = m.item(21).unwrap().unwrap();
    assert_eq!((it.id, it.count, it.damage), (1, 2, 3));
    assert_eq!(m.get(14), Some(&MetaValue::BlockPos([1, 2, 3])));
    assert_eq!(m.get(15), Some(&MetaValue::OptBlockPos(Some([1, 2, 3]))));
    assert_eq!(m.get(18), Some(&MetaValue::BlockState(16)));
    // Baby zombie (boolean at 12).
    assert_eq!(e.size(), Some((0.3, 0.975)));
    // Unknown serializer: error, nothing applied.
    let w = W::new()
        .varint(7)
        .u8(7)
        .varint(2)
        .f32(1.0)
        .u8(30)
        .varint(14)
        .u8(0xFF);
    assert!(t.apply(&w.raw(id::ENTITY_METADATA)).is_err());
    assert_eq!(v1_12::health(t.get(7).unwrap()), Some(12.5));
}

fn item_meta(eid: i32, count: i8) -> ghast_protocol::RawPacket {
    W::new()
        .varint(eid)
        .u8(meta::ITEM)
        .varint(5)
        .i16(264)
        .i8(count)
        .i16(0)
        .u8(0)
        .u8(0xFF)
        .raw(id::ENTITY_METADATA)
}

#[test]
fn pickup_removes_and_item_stacks() {
    let mut t = tracker(Config::default());
    spawn_object(&mut t, 10, 2, [0.0; 3], 1, [0; 3]);
    t.apply(&item_meta(10, 5)).unwrap();
    let e = t.get(10).unwrap();
    assert_eq!(v1_12::item(e).unwrap().count, 5);
    assert_eq!(e.item().unwrap().id, 264);
    // 1.12 always removes the collected entity, whatever the count.
    let w = W::new().varint(10).varint(SELF).varint(2).raw(id::COLLECT);
    assert_eq!(t.apply(&w), Ok(Applied::Changed));
    assert!(t.get(10).is_none());
    // Without metadata tracking no metadata is read at all.
    let mut t = tracker(Config::default().with_metadata(false));
    assert!(!t.wants(id::ENTITY_METADATA));
    spawn_object(&mut t, 10, 2, [0.0; 3], 1, [0; 3]);
    assert_eq!(t.apply(&item_meta(10, 5)), Ok(Applied::NotEntityPacket));
}

#[test]
fn passengers_leash_and_controlled_boat() {
    let mut t = tracker(Config::default());
    spawn_object(&mut t, 20, 1, [0.0, 64.0, 0.0], 0, [0; 3]); // boat
    spawn_mob(&mut t, 21, 90, [0.0; 3], [0; 3]); // pig
    let pass = |v: i32, ids: &[i32]| {
        let mut w = W::new().varint(v).varint(ids.len() as i32);
        for &i in ids {
            w = w.varint(i);
        }
        w.raw(id::SET_PASSENGERS)
    };
    t.apply(&pass(20, &[SELF, 21])).unwrap();
    assert_eq!(t.get(20).unwrap().passengers(), &[SELF, 21]);
    assert_eq!(t.me().unwrap().vehicle(), Some(20));
    // Steered by the bot: moves and teleports leave position and rotation alone, but the
    // tracked value keeps counting.
    assert_eq!(t.apply(&rel(20, [4096, 0, 0])), Ok(Applied::Ignored));
    assert_eq!(t.get(20).unwrap().position(), [0.0, 64.0, 0.0]);
    t.apply(&pass(20, &[21, SELF])).unwrap();
    t.apply(&rel(20, [4096, 0, 0])).unwrap();
    assert_eq!(t.get(20).unwrap().position(), [2.0, 64.0, 0.0]);
    t.apply(&pass(20, &[])).unwrap();
    assert_eq!(t.me().unwrap().vehicle(), None);
    // Leash: mobs only; an unknown holder detaches.
    t.apply(&W::new().i32(21).i32(SELF).raw(id::ATTACH_ENTITY))
        .unwrap();
    assert_eq!(t.get(21).unwrap().leash_holder(), Some(SELF));
    t.apply(&W::new().i32(21).i32(999).raw(id::ATTACH_ENTITY))
        .unwrap();
    assert_eq!(t.get(21).unwrap().leash_holder(), None);
    assert_eq!(
        t.apply(&W::new().i32(20).i32(SELF).raw(id::ATTACH_ENTITY)),
        Ok(Applied::Ignored),
        "a boat cannot be leashed"
    );
}

#[test]
fn equipment_and_effects() {
    let mut t = tracker(Config::default());
    spawn_mob(&mut t, 7, 54, [0.0; 3], [0; 3]);
    let eq = |slot: i32, item: i16| {
        W::new()
            .varint(7)
            .varint(slot)
            .i16(item)
            .i8(1)
            .i16(0)
            .u8(0)
            .raw(id::ENTITY_EQUIPMENT)
    };
    t.apply(&eq(0, 267)).unwrap();
    t.apply(&eq(1, 442)).unwrap();
    t.apply(&eq(5, 310)).unwrap();
    assert_eq!(t.apply(&eq(6, 1)), Ok(Applied::Ignored));
    let e = t.get(7).unwrap();
    assert_eq!(
        (
            e.equipment(0).unwrap().id,
            e.equipment(1).unwrap().id,
            e.equipment(5).unwrap().id
        ),
        (267, 442, 310)
    );
    let w = W::new().varint(7).i8(1).i8(1).varint(100).i8(2);
    t.apply(&w.raw(id::ENTITY_EFFECT)).unwrap();
    assert_eq!(t.get(7).unwrap().effect(1).unwrap().flags, 2);
    t.apply(&W::new().varint(7).i8(1).raw(id::REMOVE_ENTITY_EFFECT))
        .unwrap();
    assert!(t.get(7).unwrap().effects().is_empty());
    // Non-living entities keep no effects.
    spawn_object(&mut t, 8, 1, [0.0; 3], 0, [0; 3]);
    let w = W::new().varint(8).i8(1).i8(1).varint(100).i8(2);
    assert_eq!(t.apply(&w.raw(id::ENTITY_EFFECT)), Ok(Applied::Ignored));
}

fn respawn(dim: i32) -> ghast_protocol::RawPacket {
    W::new()
        .i32(dim)
        .u8(2)
        .u8(0)
        .str("default")
        .raw(id::RESPAWN)
}

#[test]
fn respawn_rules() {
    let mut t = tracker(Config::default());
    spawn_mob(&mut t, 7, 54, [0.0; 3], [0; 3]);
    let w = W::new().varint(SELF).i8(1).i8(1).varint(100).i8(0);
    t.apply(&w.raw(id::ENTITY_EFFECT)).unwrap();
    let w = W::new().varint(SELF).u8(0).varint(0).i8(2).u8(0xFF);
    t.apply(&w.raw(id::ENTITY_METADATA)).unwrap();
    t.apply(&respawn(0)).unwrap();
    assert!(t.get(7).is_some());
    let me = t.me().unwrap();
    assert!(me.effects().is_empty());
    assert_eq!(
        me.metadata().byte(0),
        Some(2),
        "1.12 copies the player's metadata"
    );
    t.apply(&respawn(-1)).unwrap();
    assert!(t.get(7).is_none());
    t.apply(&login(1)).unwrap();
    assert_eq!(t.self_id(), Some(SELF));
}

#[test]
fn sizes_and_variants() {
    let mut t = tracker(Config::default());
    spawn_mob(&mut t, 7, 54, [1.0, 64.0, 1.0], [0; 3]);
    assert_eq!(t.get(7).unwrap().size(), Some((0.6, 1.95)));
    let b = t.get(7).unwrap().bounding_box().unwrap();
    assert_eq!(b[1], 64.0);
    // Slime size VarInt at 12.
    spawn_mob(&mut t, 8, 55, [0.0; 3], [0; 3]);
    let w = W::new()
        .varint(8)
        .u8(meta::SLIME_SIZE)
        .varint(1)
        .varint(4)
        .u8(0xFF);
    t.apply(&w.raw(id::ENTITY_METADATA)).unwrap();
    assert_eq!(t.get(8).unwrap().size(), Some((2.0400002, 2.0400002)));
    // Baby pig.
    spawn_mob(&mut t, 9, 90, [0.0; 3], [0; 3]);
    let w = W::new()
        .varint(9)
        .u8(meta::BABY)
        .varint(6)
        .bool(true)
        .u8(0xFF);
    t.apply(&w.raw(id::ENTITY_METADATA)).unwrap();
    assert_eq!(t.get(9).unwrap().size(), Some((0.45, 0.45)));
    // Small armor stand (object 78), flags at 11.
    spawn_object(&mut t, 10, 78, [0.0; 3], 0, [0; 3]);
    assert_eq!(t.get(10).unwrap().size(), Some((0.5, 1.975)));
    let w = W::new()
        .varint(10)
        .u8(meta::ARMOR_STAND_FLAGS)
        .varint(0)
        .i8(1)
        .u8(0xFF);
    t.apply(&w.raw(id::ENTITY_METADATA)).unwrap();
    assert_eq!(t.get(10).unwrap().size(), Some((0.25, 0.9875)));
    // Boat, minecart, item frame (hanging: no box).
    spawn_object(&mut t, 11, 1, [0.0; 3], 0, [0; 3]);
    assert_eq!(t.get(11).unwrap().size(), Some((1.375, 0.5625)));
    spawn_object(&mut t, 12, 10, [0.0; 3], 0, [0; 3]);
    assert_eq!(t.get(12).unwrap().size(), Some((0.98, 0.7)));
    spawn_object(&mut t, 13, 71, [0.0; 3], 0, [0; 3]);
    assert_eq!(t.get(13).unwrap().size(), None);
    // The bot itself.
    assert_eq!(t.me().unwrap().size(), Some((0.6, 1.8)));
}

#[test]
fn filters_and_destroy() {
    let t = Tracker::new(Config::default().with_metadata(false).with_equipment(false));
    assert!(t.wants(id::SPAWN_ENTITY) && t.wants(id::SET_PASSENGERS));
    assert!(!t.wants(id::ENTITY_METADATA) && !t.wants(id::ENTITY_EQUIPMENT));
    let mut t = tracker(Config::default());
    for i in 30..33 {
        spawn_mob(&mut t, i, 54, [0.0; 3], [0; 3]);
    }
    let w = W::new()
        .varint(2)
        .varint(30)
        .varint(32)
        .raw(id::ENTITY_DESTROY);
    t.apply(&w).unwrap();
    assert!(t.get(30).is_none() && t.get(31).is_some() && t.get(32).is_none());
    // Moves of our own entity are ignored.
    assert_eq!(t.apply(&rel(SELF, [1, 1, 1])), Ok(Applied::Ignored));
}
