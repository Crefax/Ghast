//! Differential checks against the generated (node-minecraft-protocol-verified) packet codecs,
//! never-panic fuzzing, and random packet sequences with link invariants.

mod common;

use bytes::{Bytes, BytesMut};
use common::W;
use ghast_entity::{Config, Entities, MetaValue};
use ghast_protocol::testgen::Gen;
use ghast_protocol::{PacketSet, RawPacket};
use proptest::prelude::*;

fn body<P: PacketSet>(p: &P) -> Bytes {
    let mut b = BytesMut::new();
    p.encode_body(&mut b).unwrap();
    b.freeze()
}

fn all_config() -> Config {
    Config::default().with_events(true).with_skins(true)
}

/// Every packet in the store links consistently.
fn check_links(s: &Entities) {
    for e in s.iter().chain(s.me()) {
        if let Some(v) = e.vehicle() {
            let v = s.get(v).expect("vehicle exists");
            assert!(
                v.passengers().contains(&e.id()),
                "{} not in {:?}",
                e.id(),
                v.passengers()
            );
        }
        for &p in e.passengers() {
            assert_eq!(s.get(p).and_then(|p| p.vehicle()), Some(e.id()));
        }
    }
}

macro_rules! version {
    ($m:ident, $tracker:ty, $set:ty, $ids:expr, $spawn:expr, $conv:path) => {
        mod $m {
            use super::*;
            use ghast_protocol::$m::play::clientbound::id;

            fn wanted() -> Vec<i32> {
                <$tracker>::new(all_config()).packet_ids().collect()
            }

            /// Generated (valid) packets of every wanted id are accepted by the hand parser.
            #[test]
            fn generated_packets_parse() {
                let mut t = <$tracker>::new(all_config());
                let mut n = 0;
                for &pid in &wanted() {
                    for seed in 0..300u64 {
                        let mut g = Gen::new(seed ^ (pid as u64) << 32);
                        let p = <$set>::generate_by_id(pid, &mut g).unwrap();
                        let raw = RawPacket::new(pid, body(&p));
                        let r = t.apply(&raw);
                        // `Other` = an unknown enum / serializer value, which vanilla rejects too.
                        let unknown = format!("{p:?}").contains("Other");
                        assert!(r.is_ok() || unknown, "{} seed {seed}: {r:?} {p:?}", <$set>::name_of(pid).unwrap());
                        n += 1;
                    }
                }
                assert!(n > 1000);
            }

            /// Metadata: the tracker's values equal the generated codec's decoded entries.
            #[test]
            fn metadata_matches_generated_codec() {
                use ghast_protocol::$m::play::clientbound::{ClientboundPlayPacket as C};
                let mut compared = 0;
                for seed in 0..500u64 {
                    let mut g = Gen::new(seed);
                    let Some(C::EntityMetadata(p)) = <$set>::generate_by_id(id::ENTITY_METADATA, &mut g) else {
                        unreachable!()
                    };
                    if format!("{p:?}").contains("Other") {
                        continue; // unknown serializer: rejected (tested elsewhere)
                    }
                    let mut t = <$tracker>::new(all_config());
                    let spawn: fn(i32) -> RawPacket = $spawn;
                    t.apply(&spawn(p.entity_id)).unwrap();
                    let raw = RawPacket::new(id::ENTITY_METADATA, body(&C::EntityMetadata(p.clone())));
                    t.apply(&raw).unwrap();
                    let e = t.get(p.entity_id).unwrap();
                    // Last value per key wins.
                    let mut expect: Vec<(u8, String)> = Vec::new();
                    for en in &p.metadata {
                        let s = format!("{:?}", $conv(&en.value));
                        match expect.iter_mut().find(|(k, _)| *k == en.key) {
                            Some(slot) => slot.1 = s,
                            None => expect.push((en.key, s)),
                        }
                    }
                    assert_eq!(e.metadata().len(), expect.len(), "seed {seed}");
                    for (k, s) in expect {
                        let got = e.metadata().get(k).unwrap();
                        assert_eq!(format!("{got:?}"), s, "seed {seed} key {k}");
                    }
                    compared += 1;
                }
                assert!(compared > 100, "{compared}");
            }

            proptest! {
                #![proptest_config(ProptestConfig::with_cases(2000))]

                /// Random bodies for random wanted ids never panic.
                #[test]
                fn random_bodies(pick: usize, data in proptest::collection::vec(any::<u8>(), 0..200)) {
                    let ids = wanted();
                    let mut t = <$tracker>::new(all_config());
                    for eid in 0..4 {
                        let spawn: fn(i32) -> RawPacket = $spawn;
                        t.apply(&spawn(eid)).unwrap();
                    }
                    let raw = RawPacket::new(ids[pick % ids.len()], Bytes::from(data));
                    let _ = t.apply(&raw);
                    check_links(&t);
                }

                /// Mutated / truncated generated packets never panic.
                #[test]
                fn mutated_packets(pick: usize, seed: u64, flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 0..6), cut: usize) {
                    let ids = wanted();
                    let pid = ids[pick % ids.len()];
                    let mut g = Gen::new(seed);
                    let p = <$set>::generate_by_id(pid, &mut g).unwrap();
                    let mut b = body(&p).to_vec();
                    for (i, v) in flips {
                        if !b.is_empty() { let n = b.len(); b[i % n] = v; }
                    }
                    if cut % 2 == 0 && !b.is_empty() { let n = b.len(); b.truncate(cut % n); }
                    let mut t = <$tracker>::new(all_config());
                    let _ = t.apply(&RawPacket::new(pid, Bytes::from(b)));
                }

                /// Random sequences over a few ids keep riding links consistent.
                #[test]
                fn random_sequences(ops in proptest::collection::vec((0u8..6, 0i32..6, 0i32..6, any::<i16>()), 1..80)) {
                    let mut t = <$tracker>::new(all_config());
                    let ids: [i32; 5] = $ids;
                    let spawn: fn(i32) -> RawPacket = $spawn;
                    for (op, a, b, d) in ops {
                        let raw = match op {
                            0 => spawn(a),
                            1 => W::new().varint(1).varint(a).raw(ids[0]), // destroy one
                            2 => W::new().varint(a).i16(d).i16(d).i16(d).bool(true).raw(ids[1]),
                            3 => attach(ids[2], a, b),
                            4 => W::new().varint(a).varint(b).varint(1).raw(ids[3]),
                            _ => W::new().varint(b).varint(2).varint(a).varint(b).raw(ids[4]),
                        };
                        let _ = t.apply(&raw);
                        check_links(&t);
                    }
                }
            }
        }
    };
}

fn attach(id: i32, a: i32, b: i32) -> RawPacket {
    W::new().i32(a).i32(b).u8(0).raw(id)
}

fn nbt_copy(n: &Option<ghast_protocol::nbt::RawNbt>) -> Option<Bytes> {
    n.as_ref().map(|n| Bytes::copy_from_slice(n.as_bytes()))
}

fn conv_1_8(v: &ghast_protocol::v1_8::types::EntityMetadataValue) -> MetaValue {
    use ghast_protocol::v1_8::types::EntityMetadataValue as V;
    match v {
        V::Byte(b) => MetaValue::Byte(*b),
        V::Short(b) => MetaValue::Short(*b),
        V::Int(b) => MetaValue::Int(*b),
        V::Float(f) => MetaValue::Float(*f),
        V::String(s) => MetaValue::String(Box::from(&**s)),
        V::Slot(s) => MetaValue::Item(s.0.as_ref().map(|i| ghast_entity::Item {
            id: i32::from(i.id),
            count: i.count,
            damage: i.damage,
            nbt: nbt_copy(&i.nbt),
        })),
        V::BlockPos(p) => MetaValue::BlockPos([p.x, p.y, p.z]),
        V::Rotation(r) => MetaValue::Rotation([r.pitch, r.yaw, r.roll]),
        V::Other => unreachable!(),
    }
}

fn conv_1_16(v: &ghast_protocol::v1_16_5::types::EntityMetadataValue) -> MetaValue {
    use ghast_protocol::v1_16_5::types::EntityMetadataValue as V;
    let s = |s: &bytestring_like::B| Box::<str>::from(&**s);
    match v {
        V::Byte(b) => MetaValue::Byte(*b),
        V::VarInt(b) => MetaValue::VarInt(*b),
        V::Float(f) => MetaValue::Float(*f),
        V::String(x) => MetaValue::String(s(x)),
        V::Chat(x) => MetaValue::Chat(s(x)),
        V::OptChat(x) => MetaValue::OptChat(x.as_ref().map(s)),
        V::Slot(sl) => MetaValue::Item(sl.0.as_ref().map(|i| ghast_entity::Item {
            id: i.id,
            count: i.count,
            damage: 0,
            nbt: nbt_copy(&i.nbt),
        })),
        V::Boolean(b) => MetaValue::Bool(*b),
        V::Rotation(r) => MetaValue::Rotation([r.pitch, r.yaw, r.roll]),
        V::Position(p) => MetaValue::BlockPos([p.x, p.y, p.z]),
        V::OptPosition(p) => MetaValue::OptBlockPos(p.map(|p| [p.x, p.y, p.z])),
        V::Direction(d) => MetaValue::Direction(*d),
        V::OptUuid(u) => MetaValue::OptUuid(*u),
        V::BlockState(b) => MetaValue::BlockState(*b),
        V::Nbt(n) => MetaValue::Nbt(Bytes::copy_from_slice(n.as_bytes())),
        V::Particle(p) => MetaValue::Particle(p.particle_id),
        V::VillagerData(v) => {
            MetaValue::VillagerData([v.villager_type, v.villager_profession, v.level])
        }
        V::OptVarInt(v) => MetaValue::OptVarInt(*v),
        V::Pose(p) => MetaValue::Pose(*p),
        V::Other => unreachable!(),
    }
}

mod bytestring_like {
    pub type B = ghast_protocol::types::ByteString;
}

version!(
    v1_8,
    ghast_entity::v1_8::Tracker,
    ghast_protocol::v1_8::play::clientbound::ClientboundPlayPacket,
    {
        use ghast_protocol::v1_8::play::clientbound::id;
        [
            id::ENTITY_DESTROY,
            id::REL_ENTITY_MOVE,
            id::ATTACH_ENTITY,
            id::COLLECT,
            id::ENTITY_DESTROY,
        ]
    },
    |eid| W::new()
        .varint(eid)
        .u8(54)
        .i32(0)
        .i32(0)
        .i32(0)
        .i8(0)
        .i8(0)
        .i8(0)
        .i16(0)
        .i16(0)
        .i16(0)
        .u8(0x7F)
        .raw(ghast_protocol::v1_8::play::clientbound::id::SPAWN_ENTITY_LIVING),
    conv_1_8
);

version!(
    v1_16_5,
    ghast_entity::v1_16_5::Tracker,
    ghast_protocol::v1_16_5::play::clientbound::ClientboundPlayPacket,
    {
        use ghast_protocol::v1_16_5::play::clientbound::id;
        [
            id::ENTITY_DESTROY,
            id::REL_ENTITY_MOVE,
            id::ATTACH_ENTITY,
            id::COLLECT,
            id::SET_PASSENGERS,
        ]
    },
    |eid| W::new()
        .varint(eid)
        .uuid(0)
        .varint(6)
        .f64(0.0)
        .f64(0.0)
        .f64(0.0)
        .i8(0)
        .i8(0)
        .i32(0)
        .i16(0)
        .i16(0)
        .i16(0)
        .raw(ghast_protocol::v1_16_5::play::clientbound::id::SPAWN_ENTITY),
    conv_1_16
);
