//! Decode/encode benchmarks of the generated packet code, per version: the hot inbound
//! play packets of a busy server, decoded through the packet-set enum (`decode_raw`, the
//! path the client uses) and encoded back.
//!
//! The inputs are random but deterministic (`testgen`, fixed seeds); the checksum printed
//! at the start must be identical between two runs that are compared, otherwise the
//! inputs differ and the numbers are not comparable.
//!
//! Run: `cargo bench -p ghast-protocol --bench packets` (optionally `-- <filter>`).

use std::hint::black_box;

use bytes::BytesMut;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ghast_protocol::testgen::Gen;
use ghast_protocol::{PacketSet, RawPacket};

/// Packets per corpus (one corpus per version and packet kind).
const N: usize = 256;

/// FNV-1a over all bodies of a corpus.
fn checksum(raws: &[RawPacket]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for r in raws {
        for &b in r.id.to_le_bytes().iter().chain(r.body.iter()) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

fn corpus<S: PacketSet>(id: i32, generate: fn(i32, &mut Gen) -> Option<S>) -> (Vec<S>, Vec<RawPacket>) {
    let mut values = Vec::with_capacity(N);
    let mut raws = Vec::with_capacity(N);
    for seed in 0..N as u64 {
        let mut g = Gen::new(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ u64::from(id as u32));
        let p = generate(id, &mut g).expect("known id");
        let mut w = BytesMut::new();
        p.encode_body(&mut w).expect("generated packets encode");
        raws.push(RawPacket::new(p.id(), w.freeze()));
        values.push(p);
    }
    (values, raws)
}

fn bench_set<S: PacketSet>(
    c: &mut Criterion,
    version: &str,
    kinds: &[(&str, i32)],
    generate: fn(i32, &mut Gen) -> Option<S>,
) {
    let mut g = c.benchmark_group(format!("packets_{version}"));
    for &(name, id) in kinds {
        let (values, raws) = corpus::<S>(id, generate);
        let bytes: usize = raws.iter().map(|r| r.body.len()).sum();
        eprintln!(
            "{version} {name}: {N} packets, {bytes} bytes, checksum {:016x}",
            checksum(&raws)
        );
        g.throughput(Throughput::Bytes(bytes as u64));
        g.bench_function(format!("decode_{name}"), |b| {
            b.iter(|| {
                let mut n = 0usize;
                for r in black_box(&raws) {
                    n += S::decode_raw(r).unwrap().id() as usize;
                }
                black_box(n)
            })
        });
        let mut w = BytesMut::with_capacity(bytes + 64);
        g.bench_function(format!("encode_{name}"), |b| {
            b.iter(|| {
                w.clear();
                for p in black_box(&values) {
                    p.encode_body(&mut w).unwrap();
                }
                black_box(w.len())
            })
        });
    }
    g.finish();
}

macro_rules! kinds {
    ($cb:path: $($name:ident),* $(,)?) => {{
        use $cb as cb;
        &[$((stringify!($name), cb::id::$name)),*]
    }};
}

fn bench_v1_8(c: &mut Criterion) {
    use ghast_protocol::v1_8::play::clientbound as cb;
    let k: &[(&str, i32)] = kinds!(ghast_protocol::v1_8::play::clientbound:
        KEEP_ALIVE, MAP_CHUNK, WINDOW_ITEMS, SET_SLOT, PLAYER_INFO, REL_ENTITY_MOVE,
        ENTITY_MOVE_LOOK, ENTITY_TELEPORT, ENTITY_METADATA, ENTITY_VELOCITY, MULTI_BLOCK_CHANGE,
        CHAT, POSITION);
    bench_set(c, "1_8", k, cb::ClientboundPlayPacket::generate_by_id);
}

fn bench_v1_12_2(c: &mut Criterion) {
    use ghast_protocol::v1_12_2::play::clientbound as cb;
    let k: &[(&str, i32)] = kinds!(ghast_protocol::v1_12_2::play::clientbound:
        KEEP_ALIVE, MAP_CHUNK, WINDOW_ITEMS, SET_SLOT, PLAYER_INFO, REL_ENTITY_MOVE,
        ENTITY_MOVE_LOOK, ENTITY_TELEPORT, ENTITY_METADATA, ENTITY_VELOCITY, MULTI_BLOCK_CHANGE,
        CHAT, POSITION, TEAMS);
    bench_set(c, "1_12_2", k, cb::ClientboundPlayPacket::generate_by_id);
}

fn bench_v1_16_5(c: &mut Criterion) {
    use ghast_protocol::v1_16_5::play::clientbound as cb;
    let k: &[(&str, i32)] = kinds!(ghast_protocol::v1_16_5::play::clientbound:
        KEEP_ALIVE, MAP_CHUNK, UPDATE_LIGHT, WINDOW_ITEMS, SET_SLOT, PLAYER_INFO,
        REL_ENTITY_MOVE, ENTITY_MOVE_LOOK, ENTITY_TELEPORT, ENTITY_METADATA, ENTITY_VELOCITY,
        MULTI_BLOCK_CHANGE, CHAT, POSITION, TEAMS);
    bench_set(c, "1_16_5", k, cb::ClientboundPlayPacket::generate_by_id);
}

criterion_group!(benches, bench_v1_8, bench_v1_12_2, bench_v1_16_5);
criterion_main!(benches);
