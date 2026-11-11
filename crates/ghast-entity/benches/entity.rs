//! A busy-server entity packet stream: 300 tracked entities, a realistic packet mix.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ghast_entity::Config;
use ghast_protocol::RawPacket;
use ghast_protocol::v1_8::play::clientbound::{ClientboundPlayPacket as C18, id as id18};
use ghast_protocol::v1_16_5::play::clientbound::id as id16;
use ghast_protocol::v26_1::play::clientbound::id as id26;

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

/// The 1.16.5 mix in 26.1 formats (zombies; teleports are position syncs, velocities LpVec3).
fn stream_26_1(n: usize) -> (Vec<RawPacket>, Vec<RawPacket>) {
    let mut rng = Rng(0x8765_4321);
    let mut spawns: Vec<RawPacket> = vec![login_26_1()];
    for x in -4..=4 {
        for z in -4..=4 {
            spawns.push(W::new().i32(x).i32(z).raw(id26::MAP_CHUNK));
        }
    }
    spawns.extend((2..2 + ENTITIES).map(|eid| {
        W::new()
            .varint(eid)
            .uuid(eid as u128)
            .varint(150)
            .f64(0.5)
            .f64(64.0)
            .f64(0.5)
            .u8(0)
            .i8(0)
            .i8(0)
            .i8(0)
            .varint(0)
            .raw(id26::SPAWN_ENTITY)
    }));
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
                .raw(id26::REL_ENTITY_MOVE),
            35..=54 => W::new()
                .varint(eid)
                .i16(d)
                .i16(1)
                .i16(d)
                .i8(rng.next() as i8)
                .i8(0)
                .bool(false)
                .raw(id26::ENTITY_MOVE_LOOK),
            55..=69 => W::new()
                .varint(eid)
                .i8(rng.next() as i8)
                .i8(0)
                .bool(true)
                .raw(id26::ENTITY_LOOK),
            70..=79 => W::new()
                .varint(eid)
                .i8(rng.next() as i8)
                .raw(id26::ENTITY_HEAD_ROTATION),
            80..=86 => W::new()
                .varint(eid)
                .bytes(&lp_vec3([0.1, -0.2, 0.3]))
                .raw(id26::ENTITY_VELOCITY),
            87..=92 => W::new()
                .varint(eid)
                .f64(0.5 + rng.below(40) as f64)
                .f64(64.0)
                .f64(-(rng.below(40) as f64))
                .f64(0.0)
                .f64(0.0)
                .f64(0.0)
                .f32(0.0)
                .f32(0.0)
                .bool(true)
                .raw(id26::SYNC_ENTITY_POSITION),
            93..=97 => W::new()
                .varint(eid)
                .u8(0)
                .varint(0)
                .i8(rng.next() as i8 & 0x2B)
                .u8(9)
                .varint(3)
                .f32(rng.below(20) as f32)
                .u8(0xFF)
                .raw(id26::ENTITY_METADATA),
            _ => W::new().i32(eid).i8(2).raw(id26::ENTITY_STATUS),
        };
        out.push(p);
    }
    (spawns, out)
}

fn lp_vec3(v: [f64; 3]) -> Vec<u8> {
    let mut w = bytes::BytesMut::new();
    ghast_protocol::types::LpVec3::from_vec3(v).write(&mut w);
    w.to_vec()
}

fn login_26_1() -> RawPacket {
    W::new()
        .i32(1)
        .bool(false)
        .varint(1)
        .str("minecraft:overworld")
        .varint(20)
        .varint(12)
        .varint(12)
        .bool(false)
        .bool(true)
        .bool(false)
        .varint(0)
        .str("minecraft:overworld")
        .i64(0)
        .i8(0)
        .u8(0xFF)
        .bool(false)
        .bool(false)
        .bool(false)
        .varint(0)
        .varint(63)
        .bool(false)
        .raw(id26::LOGIN)
}

fn benches(c: &mut Criterion) {
    eprintln!(
        "size_of::<Entity>() = {} bytes (hash map slot: + 4-byte key, padded; 26.1 state included: {})",
        std::mem::size_of::<ghast_entity::Entity>(),
        cfg!(feature = "v26_1")
    );
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
    let moves: Vec<RawPacket> = stream
        .iter()
        .filter(|p| p.id == id16::REL_ENTITY_MOVE)
        .cloned()
        .collect();
    g.throughput(Throughput::Elements(moves.len() as u64));
    g.bench_function("1.16.5 rel move only", |b| {
        b.iter(|| {
            for p in &moves {
                black_box(t.apply(p).unwrap());
            }
        })
    });
    for (name, pid) in [
        ("move+look", id16::ENTITY_MOVE_LOOK),
        ("look", id16::ENTITY_LOOK),
        ("head", id16::ENTITY_HEAD_ROTATION),
        ("velocity", id16::ENTITY_VELOCITY),
        ("teleport", id16::ENTITY_TELEPORT),
        ("metadata", id16::ENTITY_METADATA),
    ] {
        let only: Vec<RawPacket> = stream.iter().filter(|p| p.id == pid).cloned().collect();
        g.throughput(Throughput::Elements(only.len() as u64));
        g.bench_function(format!("1.16.5 {name} only"), |b| {
            b.iter(|| {
                for p in &only {
                    black_box(t.apply(p).unwrap());
                }
            })
        });
    }
    g.throughput(Throughput::Elements(N as u64));

    let (spawns, stream) = stream_26_1(N);
    let mut t = ghast_entity::v26_1::Tracker::new(Config::default());
    for s in &spawns {
        t.apply(s).unwrap();
    }
    g.bench_function("26.1 tracker (all on)", |b| {
        b.iter(|| {
            for p in &stream {
                black_box(t.apply(p).unwrap());
            }
        })
    });
    // A client tick every 33 packets (a busy server's ~330 entity packets per tick / 10).
    g.bench_function("26.1 tracker + tick every 33 packets", |b| {
        b.iter(|| {
            for (i, p) in stream.iter().enumerate() {
                black_box(t.apply(p).unwrap());
                if i % 33 == 32 {
                    t.tick();
                }
            }
        })
    });
    for (name, pid) in [
        ("move+look", id26::ENTITY_MOVE_LOOK),
        ("look", id26::ENTITY_LOOK),
        ("head", id26::ENTITY_HEAD_ROTATION),
        ("velocity", id26::ENTITY_VELOCITY),
        ("sync", id26::SYNC_ENTITY_POSITION),
        ("metadata", id26::ENTITY_METADATA),
    ] {
        let only: Vec<RawPacket> = stream.iter().filter(|p| p.id == pid).cloned().collect();
        g.throughput(Throughput::Elements(only.len() as u64));
        g.bench_function(format!("26.1 {name} only"), |b| {
            b.iter(|| {
                for p in &only {
                    black_box(t.apply(p).unwrap());
                }
            })
        });
    }
    let moves: Vec<RawPacket> = stream
        .iter()
        .filter(|p| p.id == id26::REL_ENTITY_MOVE)
        .cloned()
        .collect();
    g.throughput(Throughput::Elements(moves.len() as u64));
    g.bench_function("26.1 rel move only", |b| {
        b.iter(|| {
            for p in &moves {
                black_box(t.apply(p).unwrap());
            }
        })
    });
    g.throughput(Throughput::Elements(ENTITIES as u64));
    g.bench_function("26.1 tick (300 zombies, all interpolating)", |b| {
        b.iter_batched_ref(
            || {
                let mut t2 = t.clone();
                for p in &moves {
                    t2.apply(p).unwrap();
                }
                t2
            },
            |t2| t2.tick(),
            criterion::BatchSize::SmallInput,
        )
    });
    let mut idle = t.clone();
    for _ in 0..40 {
        idle.tick();
    }
    g.bench_function("26.1 tick (300 zombies, settled)", |b| {
        b.iter(|| idle.tick())
    });
    g.finish();
}

criterion_group!(entity, benches);
criterion_main!(entity);
