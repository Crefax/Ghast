//! `SharedChunks` on real server bytes (the 26.1.2 fixture of `tests/v26_1.rs`): worlds that
//! share sections read exactly what an unshared world reads (the 1747 server-confirmed probes
//! included, with the session's live block and section updates), identical sections are held
//! once, across threads too, and everything is freed with the last world.
#![cfg(feature = "v26_1")]

use std::collections::HashMap;
use std::io::Read;

use bytes::Bytes;
use ghast_data::v26_1::BlockState;
use ghast_protocol::codec::Reader;
use ghast_protocol::v26_1::configuration::clientbound::ClientboundConfigurationPacket;
use ghast_protocol::{PacketSet, RawPacket};
use ghast_world::paletted::{Section, Shared};
use ghast_world::v26_1::World;
use ghast_world::{SharedChunks, WorldConfig};

const FIXTURE: &[u8] = include_bytes!("fixtures/world_26_1_2.bin.zz");
const PROBES: &[u8] = include_bytes!("data/probes_26_1_2.tsv.gz");

struct Record {
    state: u8,
    dir: u8,
    raw: RawPacket,
}

fn records() -> Vec<Record> {
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(FIXTURE)
        .read_to_end(&mut data)
        .unwrap();
    let data = Bytes::from(data);
    let mut r = Reader::new(&data);
    let mut out = Vec::new();
    while r.remaining() > 0 {
        let state = r.read_u8().unwrap();
        let dir = r.read_u8().unwrap();
        let id = r.read_varint().unwrap();
        let len = r.read_varint().unwrap() as usize;
        let body = r.read_bytes(len).unwrap();
        out.push(Record {
            state,
            dir,
            raw: RawPacket::new(id, body),
        });
    }
    out
}

/// `record -> [(x, y, z, state)]` (see `tests/v26_1.rs`).
fn probes() -> HashMap<usize, Vec<([i32; 3], BlockState)>> {
    let mut s = String::new();
    flate2::read::GzDecoder::new(PROBES)
        .read_to_string(&mut s)
        .unwrap();
    let mut m: HashMap<usize, Vec<_>> = HashMap::new();
    for l in s.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        let pos = [
            f[2].parse().unwrap(),
            f[3].parse().unwrap(),
            f[4].parse().unwrap(),
        ];
        m.entry(f[0].parse().unwrap())
            .or_default()
            .push((pos, BlockState::parse(f[5]).unwrap()));
    }
    m
}

/// Applies one fixture record to a world (server-to-client configuration and play only).
fn apply(w: &mut World, rec: &Record) -> Option<ghast_world::Applied> {
    if rec.dir != 0 {
        return None;
    }
    match rec.state {
        3 => {
            if let Ok(ClientboundConfigurationPacket::RegistryData(p)) =
                ClientboundConfigurationPacket::decode_raw(&rec.raw)
            {
                w.apply_registry_data(&p);
            }
            None
        }
        4 => Some(w.handle_raw(&rec.raw).expect("raw path")),
        _ => None,
    }
}

fn same_storage(a: &Section, b: &Section) -> bool {
    match (a, b) {
        (Section::Shared1(x), Section::Shared1(y)) => Shared::ptr_eq(x, y),
        (Section::Shared2(x), Section::Shared2(y)) => Shared::ptr_eq(x, y),
        (Section::Shared4(x), Section::Shared4(y)) => Shared::ptr_eq(x, y),
        (Section::Shared8(x), Section::Shared8(y)) => Shared::ptr_eq(x, y),
        (Section::SharedDirect(x), Section::SharedDirect(y)) => Shared::ptr_eq(x, y),
        _ => false,
    }
}

/// Every block (and the column set) of `w` equals `plain`.
fn same_blocks(plain: &World, w: &World) {
    assert_eq!(plain.columns().len(), w.columns().len());
    for (p, c) in plain.columns().iter() {
        let d = w.column(p.x, p.z).expect("same columns");
        assert_eq!(c.sections().len(), d.sections().len());
        for (a, b) in c.sections().iter().zip(d.sections()) {
            for i in 0..4096 {
                assert_eq!(a.get(i), b.get(i));
            }
        }
    }
}

/// Shared sections of `a` and `b` (same bytes received in the same order) are one allocation.
/// Returns how many sections of `a` are shared.
fn check_shared_pairs(a: &World, b: &World) -> usize {
    let mut shared = 0;
    for (p, c) in a.columns().iter() {
        let d = b.column(p.x, p.z).unwrap();
        for (x, y) in c.sections().iter().zip(d.sections()) {
            assert_eq!(x.is_shared(), y.is_shared(), "{p:?}");
            if x.is_shared() {
                assert!(same_storage(x, y), "{p:?}: one copy per key");
                shared += 1;
            }
        }
    }
    shared
}

#[test]
fn shared_worlds_replay_the_server_session() {
    let probes = probes();
    let cache = SharedChunks::new();
    let config = WorldConfig::default().with_block_entities(true);
    let mut plain = World::new(config);
    let mut a = World::new(config);
    let mut b = World::new(config);
    a.set_shared_chunks(Some(cache.clone()));
    b.set_shared_chunks(Some(cache.clone()));
    let recs = records();
    let mut checked = 0;
    let mut max_shared = 0;
    for (k, rec) in recs.iter().enumerate() {
        for (pos, state) in probes.get(&(k + 1)).into_iter().flatten() {
            let [x, y, z] = *pos;
            for w in [&a, &b] {
                assert_eq!(w.block_state(x, y, z), Some(*state), "probe {x} {y} {z}");
                assert_eq!(w.view(x, z).block_state(x, y, z), Some(*state));
            }
            checked += 1;
        }
        let r = apply(&mut plain, rec);
        assert_eq!(apply(&mut a, rec), r);
        assert_eq!(apply(&mut b, rec), r);
        if r.is_some() {
            max_shared = max_shared.max(check_shared_pairs(&a, &b));
        }
    }
    for (pos, state) in probes.get(&(recs.len() + 1)).into_iter().flatten() {
        let [x, y, z] = *pos;
        assert_eq!(a.block_state(x, y, z), Some(*state));
        assert_eq!(b.block_state(x, y, z), Some(*state));
        checked += 1;
    }
    assert_eq!(checked, 1747);
    same_blocks(&plain, &a);
    same_blocks(&plain, &b);
    let shared_now = check_shared_pairs(&a, &b);
    eprintln!(
        "shared sections: at most {max_shared} at once, {shared_now} at the end; store {} \
         sections, {} B; world heap without shared storage {} B (plain {} B)",
        cache.sections(),
        cache.heap_bytes(),
        a.heap_bytes(),
        plain.heap_bytes()
    );
    assert!(max_shared > 50, "{max_shared}");
    assert!(cache.sections() > 0 && cache.sections() <= shared_now);
    // Edited sections were copied: the plain world and the shared ones still agree (above),
    // and some sections of the last area are owned again.
    let owned_non_uniform = a
        .columns()
        .iter()
        .flat_map(|(_, c)| c.sections())
        .filter(|s| !s.is_shared() && s.bits() > 0)
        .count();
    assert!(
        owned_non_uniform > 0,
        "the session's block updates copied some sections"
    );
    drop(a);
    assert!(cache.sections() > 0);
    drop(b);
    assert_eq!(cache.sections(), 0);
    assert!(cache.heap_bytes() < 64 * 1024, "{}", cache.heap_bytes());
}

#[test]
fn bots_on_threads_share_one_store() {
    let recs = std::sync::Arc::new(records());
    let cache = SharedChunks::new();
    let mut plain = World::new(WorldConfig::default());
    for rec in recs.iter() {
        apply(&mut plain, rec);
    }
    let worlds: Vec<World> = std::thread::scope(|s| {
        let hs: Vec<_> = (0..4)
            .map(|_| {
                let (cache, recs) = (cache.clone(), recs.clone());
                s.spawn(move || {
                    let mut w = World::new(WorldConfig::default());
                    w.set_shared_chunks(Some(cache));
                    for rec in recs.iter() {
                        apply(&mut w, rec);
                    }
                    w
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for w in &worlds {
        same_blocks(&plain, w);
    }
    let shared = check_shared_pairs(&worlds[0], &worlds[1]);
    check_shared_pairs(&worlds[0], &worlds[2]);
    check_shared_pairs(&worlds[0], &worlds[3]);
    assert!(shared > 50, "{shared}");
    assert!(
        cache.sections() <= shared,
        "at most one entry per shared section"
    );
    // Dropped on other threads: the store empties.
    std::thread::scope(|s| {
        for w in worlds {
            s.spawn(move || drop(w));
        }
    });
    assert_eq!(cache.sections(), 0);
}

/// The same bytes mean different blocks under another protocol's id table: a 26.1 and a 26.2
/// world on one store never share a section.
#[cfg(feature = "v26_2")]
#[test]
fn protocols_never_mix() {
    use bytes::BufMut;
    use ghast_protocol::codec::McWrite;
    let mut d = bytes::BytesMut::new();
    for s in 0..24 {
        d.put_i16(if s == 7 { 4096 } else { 0 });
        d.put_i16(0);
        if s == 7 {
            d.put_u8(4);
            d.put_varint(2);
            d.put_varint(10);
            d.put_varint(1);
            for w in 0..256 {
                d.put_u64(if w >= 240 { 0x1111_1111_1111_1111 } else { 0 });
            }
        } else {
            d.put_u8(0);
            d.put_varint(0);
        }
        d.put_u8(0);
        d.put_varint(0);
    }
    let mut b = bytes::BytesMut::new();
    b.put_i32(0);
    b.put_i32(0);
    b.put_varint(0);
    b.put_varint(d.len() as i32);
    b.put_slice(&d);
    b.put_varint(0);
    let body = b.freeze();
    let cache = SharedChunks::new();
    let mut w1 = World::new(WorldConfig::default());
    let mut w2 = ghast_world::v26_2::World::new(WorldConfig::default());
    w1.set_shared_chunks(Some(cache.clone()));
    w2.set_shared_chunks(Some(cache.clone()));
    let dim = [("minecraft:overworld", None)];
    w1.apply_registry("minecraft:dimension_type", dim.into_iter());
    w2.apply_registry("minecraft:dimension_type", dim.into_iter());
    w1.enter_dimension("minecraft:overworld", 0, 8);
    w2.enter_dimension("minecraft:overworld", 0, 8);
    w1.apply_map_chunk_body(&body).unwrap();
    assert_eq!(cache.sections(), 1);
    w2.apply_map_chunk_body(&body).unwrap();
    assert_eq!(cache.sections(), 2);
    let (a, b) = (
        &w1.column(0, 0).unwrap().sections()[7],
        &w2.column(0, 0).unwrap().sections()[7],
    );
    assert!(a.is_shared() && b.is_shared() && !same_storage(a, b));
}

/// Mutated and truncated server chunks: a sharing world returns exactly what an unshared world
/// returns (same `Ok` / `Err`, same blocks), never panics, and two sharing worlds fed the same
/// corrupt bytes still agree with it.
#[test]
fn corrupt_chunks_behave_like_unshared() {
    use proptest::prelude::*;
    let recs = records();
    let mut chunks = Vec::new();
    for rec in &recs {
        if rec.state == 4
            && rec.dir == 0
            && rec.raw.id == ghast_protocol::v26_1::play::clientbound::id::MAP_CHUNK
        {
            chunks.push(rec.raw.body.clone());
        }
    }
    let fresh = |cache: Option<&SharedChunks>| {
        let mut w = World::new(WorldConfig::default());
        for rec in recs.iter().filter(|r| r.state == 3) {
            apply(&mut w, rec);
        }
        w.enter_dimension("minecraft:overworld", 0, 32);
        w.set_shared_chunks(cache.cloned());
        w
    };
    let cache = SharedChunks::new();
    let worlds = std::cell::RefCell::new([fresh(None), fresh(Some(&cache)), fresh(Some(&cache))]);
    let mut runner = proptest::test_runner::TestRunner::new(ProptestConfig {
        cases: 300,
        ..ProptestConfig::default()
    });
    let strategy = (
        0..chunks.len(),
        proptest::collection::vec((any::<usize>(), any::<u8>()), 0..12),
        any::<usize>(),
    );
    runner
        .run(&strategy, |(k, cuts, trunc)| {
            let mut body = chunks[k].to_vec();
            // Keep the column position (8 bytes) so the window accepts it.
            for (at, v) in cuts {
                let i = 8 + at % (body.len() - 8);
                body[i] = v;
            }
            if trunc % 3 == 0 {
                body.truncate(8 + trunc % (body.len() - 8));
            }
            let body = bytes::Bytes::from(body);
            let [plain, a, b] = &mut *worlds.borrow_mut();
            let r = plain.apply_map_chunk_body(&body).map_err(|e| e.to_string());
            prop_assert_eq!(
                &a.apply_map_chunk_body(&body).map_err(|e| e.to_string()),
                &r
            );
            prop_assert_eq!(
                &b.apply_map_chunk_body(&body).map_err(|e| e.to_string()),
                &r
            );
            Ok(())
        })
        .unwrap();
    let [plain, a, b] = worlds.into_inner();
    assert!(!plain.columns().is_empty());
    same_blocks(&plain, &a);
    same_blocks(&plain, &b);
    check_shared_pairs(&a, &b);
    drop((a, b));
    assert_eq!(cache.sections(), 0);
}
