//! A busy-server entity packet stream: 300 tracked entities, a realistic packet mix.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ghast_entity::Config;
use ghast_protocol::RawPacket;
use ghast_protocol::v1_8::play::clientbound::{ClientboundPlayPacket as C18, id as id18};
use ghast_protocol::v1_16_5::play::clientbound::id as id16;

#[path = "../tests/common/mod.rs"]
mod common;
use common::W;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const ENTITIES: i32 = 300;

fn stream_1_8(n: usize) -> (Vec<RawPacket>, Vec<RawPacket>) {
    let mut rng = Rng(0x1234_5678);
    let spawns = (2..2 + ENTITIES)
        .map(|eid| {
            W::new()
                .varint(eid)
                .u8(54)
                .i32(0)
                .i32(2048)
                .i32(0)
                .i8(0)
                .i8(0)
                .i8(0)
                .i16(0)
                .i16(0)
                .i16(0)
                .u8(0)
                .i8(0)
                .u8((3 << 5) | 6)
                .f32(20.0)
                .u8(0x7F)
                .raw(id18::SPAWN_ENTITY_LIVING)
        })
        .collect();
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let eid = 2 + rng.below(ENTITIES as u64) as i32;
        let d = || (0i8, 1i8, -1i8);
        let (a, b, c) = d();
        let p = match rng.below(100) {
            0..=34 => W::new()
                .varint(eid)
                .i8(a)
                .i8(b)
                .i8(c)
                .bool(true)
                .raw(id18::REL_ENTITY_MOVE),
            35..=54 => W::new()
                .varint(eid)
                .i8(c)
                .i8(a)
                .i8(b)
                .i8(rng.next() as i8)
                .i8(0)
                .bool(false)
                .raw(id18::ENTITY_MOVE_LOOK),
            55..=69 => W::new()
                .varint(eid)
                .i8(rng.next() as i8)
                .i8(0)
                .bool(true)
                .raw(id18::ENTITY_LOOK),
            70..=79 => W::new()
                .varint(eid)
                .i8(rng.next() as i8)
                .raw(id18::ENTITY_HEAD_ROTATION),
            80..=86 => W::new()
                .varint(eid)
                .i16(10)
                .i16(-20)
                .i16(30)
                .raw(id18::ENTITY_VELOCITY),
            87..=92 => W::new()
                .varint(eid)
                .i32(rng.next() as i32 >> 8)
                .i32(2048)
                .i32(rng.next() as i32 >> 8)
                .i8(0)
                .i8(0)
                .bool(true)
                .raw(id18::ENTITY_TELEPORT),
            93..=97 => W::new()
                .varint(eid)
                .u8(0)
                .i8(rng.next() as i8 & 0x2B)
                .u8((3 << 5) | 6)
                .f32(rng.below(20) as f32)
                .u8(0x7F)
                .raw(id18::ENTITY_METADATA),
            _ => W::new().i32(eid).i8(2).raw(id18::ENTITY_STATUS),
        };
        out.push(p);
    }
    (spawns, out)
}

fn stream_1_16(n: usize) -> (Vec<RawPacket>, Vec<RawPacket>) {
    let mut rng = Rng(0x8765_4321);
    let spawns = (2..2 + ENTITIES)
        .map(|eid| {
            W::new()
                .varint(eid)
                .uuid(eid as u128)
                .varint(102)
                .f64(0.5)
                .f64(64.0)
                .f64(0.5)
                .i8(0)
                .i8(0)
                .i8(0)
                .i16(0)
                .i16(0)
                .i16(0)
                .raw(id16::SPAWN_ENTITY_LIVING)
        })
        .collect();
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let eid = 2 + rng.below(ENTITIES as u64) as i32;
        let d = (rng.next() as i16) >> 6;
        let p = match rng.below(100) {
            0..=34 => W::new()
                .varint(eid)
                .i16(d)
                .i16(0)
                .i16(-d)
                .bool(true)
                .raw(id16::REL_ENTITY_MOVE),
            35..=54 => W::new()
                .varint(eid)
                .i16(d)
                .i16(1)
                .i16(d)
                .i8(rng.next() as i8)
                .i8(0)
                .bool(false)
                .raw(id16::ENTITY_MOVE_LOOK),
            55..=69 => W::new()
                .varint(eid)
                .i8(rng.next() as i8)
                .i8(0)
                .bool(true)
                .raw(id16::ENTITY_LOOK),
            70..=79 => W::new()
                .varint(eid)
                .i8(rng.next() as i8)
                .raw(id16::ENTITY_HEAD_ROTATION),
            80..=86 => W::new()
                .varint(eid)
                .i16(10)
                .i16(-20)
                .i16(30)
                .raw(id16::ENTITY_VELOCITY),
            87..=92 => W::new()
                .varint(eid)
                .f64(rng.below(1000) as f64 + 0.25)
                .f64(64.0)
                .f64(-(rng.below(1000) as f64))
                .i8(0)
                .i8(0)
                .bool(true)
                .raw(id16::ENTITY_TELEPORT),
            93..=97 => W::new()
                .varint(eid)
                .u8(0)
                .varint(0)
                .i8(rng.next() as i8 & 0x2B)
                .u8(8)
                .varint(2)
                .f32(rng.below(20) as f32)
                .u8(0xFF)
                .raw(id16::ENTITY_METADATA),
            _ => W::new().i32(eid).i8(2).raw(id16::ENTITY_STATUS),
        };
        out.push(p);
    }
    (spawns, out)
}

fn benches(c: &mut Criterion) {
    const N: usize = 10_000;
    let mut g = c.benchmark_group("entity stream");
    g.throughput(Throughput::Elements(N as u64));

    let (spawns, stream) = stream_1_8(N);
    let mut t = ghast_entity::v1_8::Tracker::new(Config::default());
    for s in &spawns {
        t.apply(s).unwrap();
    }
    g.bench_function("1.8 tracker (all on)", |b| {
        b.iter(|| {
            for p in &stream {
                black_box(t.apply(p).unwrap());
            }
        })
    });
    g.bench_function("1.8 generated decode only (baseline)", |b| {
        use ghast_protocol::PacketSet;
        b.iter(|| {
            for p in &stream {
                black_box(C18::decode_raw(p).unwrap());
            }
        })
    });
    let off = ghast_entity::v1_8::Tracker::new(Config::default());
    g.bench_function("1.8 wants() filter check only", |b| {
        b.iter(|| {
            let mut n = 0;
            for p in &stream {
                n += usize::from(off.wants(black_box(p.id)));
            }
            n
        })
    });

    let (spawns, stream) = stream_1_16(N);
    let mut t = ghast_entity::v1_16_5::Tracker::new(Config::default());
    for s in &spawns {
        t.apply(s).unwrap();
    }
    g.bench_function("1.16.5 tracker (all on)", |b| {
        b.iter(|| {
            for p in &stream {
                black_box(t.apply(p).unwrap());
            }
        })
    });
    g.finish();
}

criterion_group!(entity, benches);
criterion_main!(entity);
