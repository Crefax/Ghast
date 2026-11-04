//! 1.8 known-answer tests: every packet type, hand-built bodies in the vanilla layout.

mod common;

use common::{W, player_add};
use ghast_entity::v1_8::{self, Tracker, meta};
use ghast_entity::{Applied, Config, EntityType, Event, MetaValue};
use ghast_protocol::v1_8::play::clientbound::id;
use uuid::Uuid;

const SELF: i32 = 1;

fn tracker(config: Config) -> Tracker {
    let mut t = Tracker::new(config);
    let login = W::new()
        .i32(SELF)
        .u8(0)
        .i8(0)
        .u8(2)
        .u8(20)
        .str("default")
        .bool(false);
    assert_eq!(t.apply(&login.raw(id::LOGIN)), Ok(Applied::Changed));
    t
}

fn events() -> Config {
    Config {
        events: true,
        ..Config::default()
    }
}

/// Zombie (54) at fixed (x, y, z), velocity (vx, vy, vz), with a metadata list.
fn spawn_zombie(t: &mut Tracker, eid: i32, x: i32, y: i32, z: i32, v: [i16; 3], meta: W) {
    let w = W::new()
        .varint(eid)
        .u8(54)
        .i32(x)
        .i32(y)
        .i32(z)
        .i8(64)
        .i8(-32)
        .i8(-128)
        .i16(v[0])
        .i16(v[1])
        .i16(v[2])
        .bytes(&meta.0);
    assert_eq!(
        t.apply(&w.raw(id::SPAWN_ENTITY_LIVING)),
        Ok(Applied::Changed)
    );
}

fn no_meta() -> W {
    W::new().u8(0x7F)
}

#[test]
fn mob_spawn_moves_and_teleport() {
    let mut t = tracker(events());
    spawn_zombie(
        &mut t,
        7,
        100 * 32 + 5,
        64 * 32,
        -3 * 32 - 1,
        [1, -8000, 3],
        no_meta(),
    );
    let e = t.get(7).unwrap();
    assert_eq!(e.entity_type(), EntityType::LegacyMob(54));
    assert_eq!(e.position(), [100.15625, 64.0, -3.03125]);
    assert_eq!(e.fixed_position(), [3205, 2048, -97]);
    assert_eq!((e.yaw(), e.pitch(), e.head_yaw()), (90.0, -45.0, -180.0));
    // Mob spawn velocity is divided in float, then widened.
    assert_eq!(e.velocity()[0], f64::from(1.0f32 / 8000.0));
    assert_ne!(e.velocity()[0], 1.0 / 8000.0);
    assert_eq!(e.velocity()[1], -1.0);
    assert_eq!(v1_8::info(e.entity_type()).unwrap().name, "Zombie");

    // rel_entity_move: deltas in 1/32 added to the fixed position.
    let w = W::new().varint(7).i8(127).i8(-128).i8(1).bool(true);
    assert_eq!(t.apply(&w.raw(id::REL_ENTITY_MOVE)), Ok(Applied::Changed));
    let e = t.get(7).unwrap();
    assert_eq!(e.fixed_position(), [3332, 1920, -96]);
    assert_eq!(e.position(), [104.125, 60.0, -3.0]);
    assert!(e.on_ground());
    assert_eq!(e.yaw(), 90.0, "no rotation in a pure move");

    // entity_move_look and entity_look.
    let w = W::new()
        .varint(7)
        .i8(-1)
        .i8(0)
        .i8(0)
        .i8(1)
        .i8(2)
        .bool(false);
    t.apply(&w.raw(id::ENTITY_MOVE_LOOK)).unwrap();
    let e = t.get(7).unwrap();
    assert_eq!(e.position()[0], 3331.0 / 32.0);
    assert_eq!((e.yaw(), e.pitch()), (1.40625, 2.8125));
    let w = W::new().varint(7).i8(-64).i8(0).bool(true);
    t.apply(&w.raw(id::ENTITY_LOOK)).unwrap();
    assert_eq!(t.get(7).unwrap().yaw(), -90.0);
    assert!(t.get(7).unwrap().on_ground());

    // The bare `entity` packet goes through the same handler and clears on_ground.
    t.apply(&W::new().varint(7).raw(id::ENTITY)).unwrap();
    assert!(!t.get(7).unwrap().on_ground());

    // Teleport sets the fixed position absolutely.
    let w = W::new()
        .varint(7)
        .i32(-1)
        .i32(2)
        .i32(-33)
        .i8(0)
        .i8(0)
        .bool(true);
    t.apply(&w.raw(id::ENTITY_TELEPORT)).unwrap();
    let e = t.get(7).unwrap();
    assert_eq!(e.position(), [-0.03125, 0.0625, -1.03125]);

    // Velocity packet divides in double.
    let w = W::new().varint(7).i16(1).i16(0).i16(-1);
    t.apply(&w.raw(id::ENTITY_VELOCITY)).unwrap();
    assert_eq!(
        t.get(7).unwrap().velocity(),
        [1.0 / 8000.0, 0.0, -1.0 / 8000.0]
    );

    // Head rotation.
    t.apply(&W::new().varint(7).i8(32).raw(id::ENTITY_HEAD_ROTATION))
        .unwrap();
    assert_eq!(t.get(7).unwrap().head_yaw(), 45.0);

    // Moves for unknown ids are ignored.
    let w = W::new().varint(99).i8(1).i8(1).i8(1).bool(true);
    assert_eq!(t.apply(&w.raw(id::REL_ENTITY_MOVE)), Ok(Applied::Ignored));

    let ev: Vec<Event> = t.store_mut().drain_events().collect();
    assert!(ev.contains(&Event::Spawned(7)));
    assert!(ev.iter().filter(|e| **e == Event::Moved(7)).count() >= 5);
}

#[test]
fn unknown_and_abstract_mobs_spawn_nothing() {
    let mut t = tracker(Config::default());
    for ty in [48u8, 49, 200] {
        let w = W::new()
            .varint(5)
            .u8(ty)
            .i32(0)
            .i32(0)
            .i32(0)
            .i8(0)
            .i8(0)
            .i8(0)
            .i16(0)
            .i16(0)
            .i16(0)
            .u8(0x7F);
        assert_eq!(
            t.apply(&w.raw(id::SPAWN_ENTITY_LIVING)),
            Ok(Applied::Ignored)
        );
    }
    assert!(t.get(5).is_none());
}

#[test]
fn object_velocity_only_when_data_positive() {
    let mut t = tracker(Config::default());
    // Arrow (60) with data 3 (> 0): velocity present.
    let w = W::new()
        .varint(10)
        .i8(60)
        .i32(32)
        .i32(64)
        .i32(96)
        .i8(0)
        .i8(64)
        .i32(3)
        .i16(800)
        .i16(-800)
        .i16(0);
    t.apply(&w.raw(id::SPAWN_ENTITY)).unwrap();
    let e = t.get(10).unwrap();
    assert_eq!(e.position(), [1.0, 2.0, 3.0]);
    assert_eq!(e.velocity(), [0.1, -0.1, 0.0]);
    assert_eq!(e.data(), 3);
    assert_eq!(e.yaw(), 90.0);
    // Falling block (70) with a negative data value: no velocity on the wire.
    let w = W::new()
        .varint(11)
        .i8(70)
        .i32(0)
        .i32(0)
        .i32(0)
        .i8(0)
        .i8(0)
        .i32(-5);
    t.apply(&w.raw(id::SPAWN_ENTITY)).unwrap();
    assert_eq!(t.get(11).unwrap().velocity(), [0.0; 3]);
    assert_eq!(t.get(11).unwrap().data(), -5);
    // Item (2), data 0: no velocity.
    let w = W::new()
        .varint(12)
        .i8(2)
        .i32(0)
        .i32(0)
        .i32(0)
        .i8(0)
        .i8(0)
        .i32(0);
    t.apply(&w.raw(id::SPAWN_ENTITY)).unwrap();
    assert_eq!(
        t.get(12).unwrap().entity_type(),
        EntityType::LegacyObject(2)
    );
    // Unknown object id: nothing.
    let w = W::new()
        .varint(13)
        .i8(42)
        .i32(0)
        .i32(0)
        .i32(0)
        .i8(0)
        .i8(0)
        .i32(0);
    assert_eq!(t.apply(&w.raw(id::SPAWN_ENTITY)), Ok(Applied::Ignored));
    assert!(t.get(13).is_none());
}

#[test]
fn players_orbs_paintings_and_tab_list() {
    let mut t = tracker(Config::default());
    let uuid = 0x1234_5678_9abc_def0_1122_3344_5566_7788u128;
    t.apply(&player_add(uuid, "Bob", true).raw(id::PLAYER_INFO))
        .unwrap();
    let w = W::new()
        .varint(20)
        .uuid(uuid)
        .i32(16)
        .i32(32)
        .i32(48)
        .i8(0)
        .i8(0)
        .i16(276) // diamond sword in hand
        .u8(0x00) // flags byte entry: type 0, index 0
        .i8(0x02)
        .u8(0x7F);
    t.apply(&w.raw(id::NAMED_ENTITY_SPAWN)).unwrap();
    let e = t.get(20).unwrap();
    assert!(e.is_player());
    assert_eq!(e.uuid(), Uuid::from_u128(uuid));
    assert_eq!(e.position(), [0.5, 1.0, 1.5]);
    assert_eq!(e.equipment(0).unwrap().id, 276);
    assert_eq!(v1_8::flags(e), 0x02);
    assert_eq!(t.player_name(20), Some("Bob"));
    let info = t.player_info(&Uuid::from_u128(uuid)).unwrap();
    assert_eq!((info.game_mode, info.ping), (1, 42));
    assert_eq!(info.display_name.as_deref(), Some("{\"text\":\"Bob\"}"));
    // Latency, game mode, display name, remove.
    let w = W::new().varint(2).varint(1).uuid(uuid).varint(150);
    t.apply(&w.raw(id::PLAYER_INFO)).unwrap();
    let w = W::new().varint(1).varint(1).uuid(uuid).varint(3);
    t.apply(&w.raw(id::PLAYER_INFO)).unwrap();
    let w = W::new().varint(3).varint(1).uuid(uuid).bool(false);
    t.apply(&w.raw(id::PLAYER_INFO)).unwrap();
    let info = t.player_info(&Uuid::from_u128(uuid)).unwrap();
    assert_eq!(
        (info.game_mode, info.ping, info.display_name.clone()),
        (3, 150, None)
    );
    let w = W::new().varint(4).varint(1).uuid(uuid);
    t.apply(&w.raw(id::PLAYER_INFO)).unwrap();
    assert!(t.player_info(&Uuid::from_u128(uuid)).is_none());
    assert_eq!(t.player_name(20), None);

    let w = W::new().varint(21).i32(-32).i32(0).i32(16).i16(7);
    t.apply(&w.raw(id::SPAWN_ENTITY_EXPERIENCE_ORB)).unwrap();
    let e = t.get(21).unwrap();
    assert_eq!(
        (e.entity_type(), e.position(), e.data()),
        (EntityType::ExperienceOrb, [-1.0, 0.0, 0.5], 7)
    );

    let pos = ((10i64 & 0x3FF_FFFF) << 38) | ((64i64 & 0xFFF) << 26) | (-5i64 & 0x3FF_FFFF);
    let w = W::new().varint(22).str("Kebab").i64(pos).u8(2);
    t.apply(&w.raw(id::SPAWN_ENTITY_PAINTING)).unwrap();
    let e = t.get(22).unwrap();
    assert_eq!(
        (e.entity_type(), e.position(), e.data()),
        (EntityType::Painting, [10.0, 64.0, -5.0], 2)
    );

    // Lightning is not tracked by id.
    let w = W::new().varint(23).i8(1).i32(0).i32(0).i32(0);
    assert_eq!(
        t.apply(&w.raw(id::SPAWN_ENTITY_WEATHER)),
        Ok(Applied::Ignored)
    );
}

#[test]
fn metadata_equipment_effects() {
    let mut t = tracker(Config::default());
    // Spawn with flags (byte), health (float idx 6), custom name (string idx 2).
    let meta_list = W::new()
        .u8(0) // byte 0
        .i8(0x01)
        .u8((3 << 5) | 6)
        .f32(20.0)
        .u8((4 << 5) | 2)
        .str("Grumm")
        .u8(0x7F);
    spawn_zombie(&mut t, 7, 0, 0, 0, [0; 3], meta_list);
    let e = t.get(7).unwrap();
    assert_eq!(v1_8::health(e), Some(20.0));
    assert_eq!(v1_8::custom_name(e), Some("Grumm"));
    assert_eq!(v1_8::flags(e), 1);
    // Update: health, a short (air), an item, block pos and rotation entries.
    let w = W::new()
        .varint(7)
        .u8((3 << 5) | 6)
        .f32(3.5)
        .u8((1 << 5) | 1)
        .i16(300)
        .u8((5 << 5) | 10)
        .i16(264)
        .i8(3)
        .i16(0)
        .u8(0) // no NBT
        .u8((6 << 5) | 11)
        .i32(1)
        .i32(2)
        .i32(3)
        .u8((7 << 5) | 12)
        .f32(1.0)
        .f32(2.0)
        .f32(3.0)
        .u8((2 << 5) | 13)
        .i32(-7)
        .u8(0x7F);
    t.apply(&w.raw(id::ENTITY_METADATA)).unwrap();
    let e = t.get(7).unwrap();
    let m = e.metadata();
    assert_eq!(v1_8::health(e), Some(3.5));
    assert_eq!(m.get(meta::AIR), Some(&MetaValue::Short(300)));
    assert_eq!(m.item(10).unwrap().unwrap().count, 3);
    assert_eq!(m.get(11), Some(&MetaValue::BlockPos([1, 2, 3])));
    assert_eq!(m.get(12), Some(&MetaValue::Rotation([1.0, 2.0, 3.0])));
    assert_eq!(m.get(13), Some(&MetaValue::Int(-7)));
    assert_eq!(m.len(), 8, "updates overwrite in place");
    // A truncated metadata packet errors and changes nothing.
    let w = W::new()
        .varint(7)
        .u8((3 << 5) | 6)
        .f32(1.0)
        .u8((3 << 5) | 7);
    assert!(t.apply(&w.raw(id::ENTITY_METADATA)).is_err());
    assert_eq!(v1_8::health(t.get(7).unwrap()), Some(3.5));

    // Equipment: helmet (slot 4) with NBT; slot 5 is out of range in 1.8.
    let nbt = [10u8, 0, 0, 1, 0, 1, b'a', 5, 0]; // compound "" { byte "a" = 5 }
    let w = W::new().varint(7).i16(4).i16(298).i8(1).i16(0).bytes(&nbt);
    t.apply(&w.raw(id::ENTITY_EQUIPMENT)).unwrap();
    let it = t.get(7).unwrap().equipment(4).unwrap();
    assert_eq!((it.id, it.nbt.as_deref()), (298, Some(&nbt[..])));
    let w = W::new().varint(7).i16(5).i16(298).i8(1).i16(0).u8(0);
    assert_eq!(t.apply(&w.raw(id::ENTITY_EQUIPMENT)), Ok(Applied::Ignored));

    // Effects: add, overwrite, remove.
    let w = W::new().varint(7).i8(1).i8(0).varint(600).bool(true);
    t.apply(&w.raw(id::ENTITY_EFFECT)).unwrap();
    let w = W::new().varint(7).i8(1).i8(2).varint(100).bool(false);
    t.apply(&w.raw(id::ENTITY_EFFECT)).unwrap();
    let w = W::new().varint(7).i8(8).i8(0).varint(20).bool(false);
    t.apply(&w.raw(id::ENTITY_EFFECT)).unwrap();
    let e = t.get(7).unwrap();
    assert_eq!(e.effects().len(), 2);
    assert_eq!(e.effect(1).unwrap().amplifier, 2);
    t.apply(&W::new().varint(7).i8(1).raw(id::REMOVE_ENTITY_EFFECT))
        .unwrap();
    assert!(t.get(7).unwrap().effect(1).is_none());
    assert!(t.get(7).unwrap().effect(8).is_some());
}

#[test]
fn riding_leash_collect_destroy() {
    let mut t = tracker(events());
    spawn_zombie(&mut t, 7, 0, 0, 0, [0; 3], no_meta());
    spawn_zombie(&mut t, 8, 0, 0, 0, [0; 3], no_meta());
    let boat = W::new()
        .varint(9)
        .i8(1)
        .i32(0)
        .i32(0)
        .i32(0)
        .i8(0)
        .i8(0)
        .i32(0);
    t.apply(&boat.raw(id::SPAWN_ENTITY)).unwrap();
    let attach =
        |e: i32, v: i32, leash: u8| W::new().i32(e).i32(v).u8(leash).raw(id::ATTACH_ENTITY);
    t.apply(&attach(7, 9, 0)).unwrap();
    assert_eq!(t.get(7).unwrap().vehicle(), Some(9));
    assert_eq!(t.get(9).unwrap().passengers(), &[7]);
    // The bot boards the same boat: 1.8 has one rider per vehicle.
    t.apply(&attach(SELF, 9, 0)).unwrap();
    assert_eq!(t.get(9).unwrap().passengers(), &[SELF]);
    assert_eq!(t.me().unwrap().vehicle(), Some(9));
    assert_eq!(t.get(7).unwrap().vehicle(), None);
    // An unknown vehicle id dismounts.
    t.apply(&attach(SELF, 12345, 0)).unwrap();
    assert_eq!(t.me().unwrap().vehicle(), None);
    // Leash.
    t.apply(&attach(8, SELF, 1)).unwrap();
    assert_eq!(t.get(8).unwrap().leash_holder(), Some(SELF));
    t.apply(&attach(8, -1, 1)).unwrap();
    assert_eq!(t.get(8).unwrap().leash_holder(), None);

    // Collect removes the collected entity.
    t.apply(&W::new().varint(8).varint(SELF).raw(id::COLLECT))
        .unwrap();
    assert!(t.get(8).is_none());
    // Destroy several (one unknown).
    let w = W::new().varint(3).varint(7).varint(9).varint(555);
    assert_eq!(t.apply(&w.raw(id::ENTITY_DESTROY)), Ok(Applied::Changed));
    assert_eq!(t.iter().count(), 0);
    let ev: Vec<Event> = t.store_mut().drain_events().collect();
    assert!(ev.contains(&Event::Collected {
        collected: 8,
        collector: SELF
    }));
    assert!(ev.contains(&Event::Removed {
        id: 9,
        ty: EntityType::LegacyObject(1)
    }));
}

#[test]
fn spawn_replaces_and_self_is_not_moved() {
    let mut t = tracker(Config::default());
    spawn_zombie(&mut t, 7, 32, 0, 0, [0; 3], no_meta());
    let w = W::new()
        .varint(7)
        .i8(2)
        .i32(0)
        .i32(0)
        .i32(0)
        .i8(0)
        .i8(0)
        .i32(0);
    t.apply(&w.raw(id::SPAWN_ENTITY)).unwrap();
    assert_eq!(t.get(7).unwrap().entity_type(), EntityType::LegacyObject(2));
    // Movement for the bot's own id is ignored (the client owns its position).
    let w = W::new().varint(SELF).i8(10).i8(10).i8(10).bool(true);
    assert_eq!(t.apply(&w.raw(id::REL_ENTITY_MOVE)), Ok(Applied::Ignored));
    assert_eq!(t.me().unwrap().position(), [0.0; 3]);
    // ... but its effects are tracked.
    let w = W::new().varint(SELF).i8(1).i8(1).varint(200).bool(false);
    t.apply(&w.raw(id::ENTITY_EFFECT)).unwrap();
    assert_eq!(t.me().unwrap().effect(1).unwrap().amplifier, 1);
}

#[test]
fn respawn_and_login() {
    let mut t = tracker(Config::default());
    spawn_zombie(&mut t, 7, 0, 0, 0, [0; 3], no_meta());
    let w = W::new().varint(SELF).i8(1).i8(1).varint(200).bool(false);
    t.apply(&w.raw(id::ENTITY_EFFECT)).unwrap();
    let respawn = |dim: i32| {
        W::new()
            .i32(dim)
            .u8(0)
            .u8(0)
            .str("default")
            .raw(id::RESPAWN)
    };
    t.apply(&respawn(0)).unwrap();
    assert!(t.get(7).is_some(), "same dimension keeps entities");
    assert!(t.me().unwrap().effects().is_empty(), "new player entity");
    t.apply(&respawn(-1)).unwrap();
    assert!(t.get(7).is_none());
    assert_eq!(t.self_id(), Some(SELF));
    let login = W::new()
        .i32(77)
        .u8(0)
        .i8(0)
        .u8(2)
        .u8(20)
        .str("default")
        .bool(false);
    t.apply(&login.raw(id::LOGIN)).unwrap();
    assert_eq!(t.self_id(), Some(77));
    assert!(t.get(SELF).is_none());
}

#[test]
fn config_filters_packets() {
    let t = Tracker::new(Config {
        metadata: false,
        equipment: false,
        effects: false,
        player_list: false,
        events: false,
    });
    assert!(t.wants(id::REL_ENTITY_MOVE));
    assert!(!t.wants(id::ENTITY_METADATA));
    assert!(!t.wants(id::PLAYER_INFO));
    assert!(!t.wants(id::CHAT));
    let mut t = t;
    let w = W::new().varint(7).u8(0x7F);
    assert_eq!(
        t.apply(&w.raw(id::ENTITY_METADATA)),
        Ok(Applied::NotEntityPacket)
    );
    assert_eq!(t.packet_ids().count(), v1_8::CORE_IDS.len());
}

#[test]
fn class_item_and_box() {
    use ghast_entity::EntityClass;
    let mut t = tracker(Config::default());
    spawn_zombie(&mut t, 7, 0, 0, 0, [0; 3], no_meta());
    let w = W::new()
        .varint(12)
        .i8(2)
        .i32(0)
        .i32(0)
        .i32(0)
        .i8(0)
        .i8(0)
        .i32(1)
        .i16(0)
        .i16(0)
        .i16(0);
    t.apply(&w.raw(id::SPAWN_ENTITY)).unwrap();
    let w = W::new()
        .varint(12)
        .u8((5 << 5) | 10)
        .i16(4)
        .i8(9)
        .i16(0)
        .u8(0)
        .u8(0x7F);
    t.apply(&w.raw(id::ENTITY_METADATA)).unwrap();
    assert_eq!(t.get(7).unwrap().class(), EntityClass::Mob);
    assert_eq!(t.get(12).unwrap().class(), EntityClass::Item);
    assert_eq!(t.get(12).unwrap().item().unwrap().count, 9);
    // Vanilla sizes (oracle-checked in ghast-data): zombie 0.6 x 1.95, box bit-exact.
    assert_eq!(t.get(7).unwrap().size(), Some((0.6, 1.95)));
    let w = W::new()
        .varint(7)
        .i32(16)
        .i32(64 * 32)
        .i32(16)
        .i8(0)
        .i8(0)
        .bool(true);
    t.apply(&w.raw(id::ENTITY_TELEPORT)).unwrap();
    let b = t.get(7).unwrap().bounding_box().unwrap();
    assert_eq!(b[0].to_bits(), 0x3fc9_9999_8000_0000, "{}", b[0]);
    assert_eq!(b[3], 0.800000011920929);
    // A baby zombie keeps the adult box on the 1.8 client.
    let w = W::new().varint(7).u8(12).i8(1).u8(0x7F);
    t.apply(&w.raw(id::ENTITY_METADATA)).unwrap();
    assert_eq!(t.get(7).unwrap().size(), Some((0.6, 1.95)));
    // Slime size 2.
    let slime = W::new()
        .varint(30)
        .u8(55)
        .i32(0)
        .i32(0)
        .i32(0)
        .i8(0)
        .i8(0)
        .i8(0)
        .i16(0)
        .i16(0)
        .i16(0)
        .u8(16)
        .i8(2)
        .u8(0x7F);
    t.apply(&slime.raw(id::SPAWN_ENTITY_LIVING)).unwrap();
    assert_eq!(t.get(30).unwrap().size(), Some((1.0200001, 1.0200001)));
    // A baby sheep (age byte -1) uses the baby size.
    let sheep = W::new()
        .varint(31)
        .u8(91)
        .i32(0)
        .i32(0)
        .i32(0)
        .i8(0)
        .i8(0)
        .i8(0)
        .i16(0)
        .i16(0)
        .i16(0)
        .u8(12)
        .i8(-1)
        .u8(0x7F);
    t.apply(&sheep.raw(id::SPAWN_ENTITY_LIVING)).unwrap();
    assert_eq!(t.get(31).unwrap().size(), Some((0.45, 0.65)));
    assert_eq!(t.get(12).unwrap().size(), Some((0.25, 0.25)), "item");
}
