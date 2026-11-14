//! 26.1 world benchmarks: chunk decode (real server chunks), block reads, and modern-core
//! physics ticks on the real world type against a dense array.
//!
//! `cargo bench -p ghast-world --bench world_26_1` — memory figures are printed first.

use std::hint::black_box;
use std::io::Read;

use bytes::{BufMut, Bytes, BytesMut};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ghast_data::v26_1::BlockState;
use ghast_physics::modern::{self, PlayerState, V26_1};
use ghast_physics::{BlockView, Keys, Packets, TickInput};
use ghast_protocol::codec::{McWrite, Reader};
use ghast_protocol::v26_1::configuration::clientbound::ClientboundConfigurationPacket;
use ghast_protocol::v26_1::play::clientbound::{self as cb, ClientboundPlayPacket};
use ghast_protocol::{PacketSet, RawPacket};
use ghast_world::v26_1::World;
use ghast_world::{SharedChunks, WorldConfig};

const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/world_26_1_2.bin.zz");

/// The fixture's configuration registries and chunk packets per dimension.
struct Fixture {
    registries: Vec<RawPacket>,
    /// `(dimension name, type index, chunks)`, one entry per visit.
    visits: Vec<(String, i32, Vec<RawPacket>)>,
}

fn fixture() -> Fixture {
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(FIXTURE)
        .read_to_end(&mut data)
        .unwrap();
    let data = Bytes::from(data);
    let mut r = Reader::new(&data);
    let mut f = Fixture {
        registries: Vec::new(),
        visits: Vec::new(),
    };
    while r.remaining() > 0 {
        let state = r.read_u8().unwrap();
        let dir = r.read_u8().unwrap();
        let id = r.read_varint().unwrap();
        let len = r.read_varint().unwrap() as usize;
        let raw = RawPacket::new(id, r.read_bytes(len).unwrap());
        if dir != 0 {
            continue;
        }
        if state == 3 {
            if f.visits.is_empty() {
                f.registries.push(raw);
            }
            continue;
        }
        match ClientboundPlayPacket::decode_raw(&raw) {
            Ok(ClientboundPlayPacket::Login(l)) => f.visits.push((
                l.world_state.name.to_string(),
                l.world_state.dimension,
                Vec::new(),
            )),
            Ok(ClientboundPlayPacket::Respawn(l)) => f.visits.push((
                l.world_state.name.to_string(),
                l.world_state.dimension,
                Vec::new(),
            )),
            _ if id == cb::id::MAP_CHUNK => f.visits.last_mut().unwrap().2.push(raw),
            _ => {}
        }
    }
    f
}

fn world_with_registries(f: &Fixture) -> World {
    let mut w = World::new(WorldConfig::default());
    for raw in &f.registries {
        if let Ok(ClientboundConfigurationPacket::RegistryData(p)) =
            ClientboundConfigurationPacket::decode_raw(raw)
        {
            w.apply_registry_data(&p);
        }
    }
    w
}

/// A flat overworld chunk: stone at y 63 (section 7, local y 15), air elsewhere.
fn flat_chunk(cx: i32, cz: i32, stone: u16) -> RawPacket {
    let mut d = BytesMut::new();
    for s in 0..24 {
        d.put_i16(if s == 7 { 256 } else { 0 });
        d.put_i16(0);
        if s == 7 {
            d.put_u8(4);
            d.put_varint(2);
            d.put_varint(0);
            d.put_varint(i32::from(stone));
            for w in 0..256 {
                // Row y 15 = indices 3840..4096 = words 240..256.
                d.put_u64(if w >= 240 { 0x1111_1111_1111_1111 } else { 0 });
            }
        } else {
            d.put_u8(0);
            d.put_varint(0);
        }
        d.put_u8(0);
        d.put_varint(0);
    }
    let mut b = BytesMut::new();
    b.put_i32(cx);
    b.put_i32(cz);
    b.put_varint(0); // heightmaps
    b.put_varint(d.len() as i32);
    b.put_slice(&d);
    b.put_varint(0); // block entities
    for _ in 0..4 {
        b.put_varint(0); // light masks
    }
    b.put_varint(0);
    b.put_varint(0);
    RawPacket::new(cb::id::MAP_CHUNK, b.freeze())
}

/// A dense copy of a region (the physics bench's array-backed world).
struct Dense {
    origin: [i32; 3],
    side: [i32; 3],
    blocks: Vec<BlockState>,
}

impl Dense {
    fn of<W: BlockView<State = BlockState>>(w: &W, origin: [i32; 3], side: [i32; 3]) -> Dense {
        let mut blocks = Vec::with_capacity((side[0] * side[1] * side[2]) as usize);
        for y in 0..side[1] {
            for z in 0..side[2] {
                for x in 0..side[0] {
                    blocks.push(w.block(origin[0] + x, origin[1] + y, origin[2] + z));
                }
            }
        }
        Dense {
            origin,
            side,
            blocks,
        }
    }
}

impl BlockView for Dense {
    type State = BlockState;

    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> BlockState {
        let (lx, ly, lz) = (x - self.origin[0], y - self.origin[1], z - self.origin[2]);
        if lx < 0
            || ly < 0
            || lz < 0
            || lx >= self.side[0]
            || ly >= self.side[1]
            || lz >= self.side[2]
        {
            return BlockState::AIR;
        }
        self.blocks[((ly * self.side[2] + lz) * self.side[0] + lx) as usize]
    }

    #[inline]
    fn is_chunk_loaded(&self, _: i32, _: i32) -> bool {
        true
    }
}

const TICKS: usize = 20;

fn run_ticks<W: BlockView<State = BlockState>>(
    w: &W,
    start: PlayerState,
    input: &TickInput,
) -> PlayerState {
    let mut out = Packets::new();
    let mut p = start;
    for _ in 0..TICKS {
        out.clear();
        modern::tick::<V26_1, _>(&mut p, input, black_box(w), &mut out);
    }
    p
}

fn bench(c: &mut Criterion) {
    let f = fixture();

    // ---------------------------------------------------------------- memory
    for (name, ty, chunks) in &f.visits {
        let mut w = world_with_registries(&f);
        w.enter_dimension(name, *ty, 32);
        for ch in chunks {
            w.handle_raw(ch).unwrap();
        }
        let n = w.columns().len().max(1);
        let bits = w
            .columns()
            .iter()
            .flat_map(|(_, c)| c.sections().iter().map(|s| s.bits()))
            .fold([0usize; 17], |mut a, b| {
                a[b as usize] += 1;
                a
            });
        println!(
            "{name}: {n} columns, {} B/column (sections by bits 0/1/2/4/8/16: {}/{}/{}/{}/{}/{}), wire {} B/chunk",
            w.heap_bytes() / n,
            bits[0],
            bits[1],
            bits[2],
            bits[4],
            bits[8],
            bits[16],
            chunks.iter().map(|c| c.body.len()).sum::<usize>() / chunks.len().max(1)
        );
    }

    // ---------------------------------------------------------------- decode
    let mut g = c.benchmark_group("26.1 chunk decode");
    for (k, (name, ty, chunks)) in f.visits.iter().enumerate() {
        if chunks.is_empty() {
            continue;
        }
        g.throughput(Throughput::Elements(chunks.len() as u64));
        let label = format!("{k} {name}");
        let mut w = world_with_registries(&f);
        g.bench_function(format!("{label} raw body"), |b| {
            b.iter(|| {
                w.enter_dimension(name, *ty, 32);
                for ch in chunks {
                    w.handle_raw(black_box(ch)).unwrap();
                }
            })
        });
        // Sharing on, nothing in the store yet (the previous iteration's sections die when
        // the dimension is entered again): hash + decode + store every section.
        let mut ws = world_with_registries(&f);
        ws.set_shared_chunks(Some(SharedChunks::new()));
        g.bench_function(format!("{label} raw body, shared (cold store)"), |b| {
            b.iter(|| {
                ws.enter_dimension(name, *ty, 32);
                for ch in chunks {
                    ws.handle_raw(black_box(ch)).unwrap();
                }
            })
        });
        // Sharing on, another world already holds every section: hash + take, no decode.
        let cache = SharedChunks::new();
        let mut keeper = world_with_registries(&f);
        keeper.set_shared_chunks(Some(cache.clone()));
        keeper.enter_dimension(name, *ty, 32);
        for ch in chunks {
            keeper.handle_raw(ch).unwrap();
        }
        ws.set_shared_chunks(Some(cache));
        g.bench_function(format!("{label} raw body, shared (warm store)"), |b| {
            b.iter(|| {
                ws.enter_dimension(name, *ty, 32);
                for ch in chunks {
                    ws.handle_raw(black_box(ch)).unwrap();
                }
            })
        });
        drop(keeper);
        if k == 0 {
            g.bench_function(format!("{label} generated packet + apply"), |b| {
                b.iter(|| {
                    w.enter_dimension(name, *ty, 32);
                    for ch in chunks {
                        let p = ClientboundPlayPacket::decode_raw(black_box(ch)).unwrap();
                        w.handle(&p).unwrap();
                    }
                })
            });
            g.bench_function(
                format!("{label} protocol Sections walk (lower bound)"),
                |b| {
                    b.iter(|| {
                        let mut n = 0usize;
                        for ch in chunks {
                            let ClientboundPlayPacket::MapChunk(p) =
                                ClientboundPlayPacket::decode_raw(ch).unwrap()
                            else {
                                unreachable!()
                            };
                            for s in ghast_protocol::types::chunk::Sections::new(&p.chunk_data) {
                                n += s.unwrap().block_count as usize;
                            }
                        }
                        black_box(n)
                    })
                },
            );
        }
    }
    g.finish();

    // ---------------------------------------------------------------- reads
    // The last visit: the origin area of the second session (overworld, edited blocks).
    let (name, ty, chunks) = f.visits.last().unwrap();
    let mut w = world_with_registries(&f);
    w.enter_dimension(name, *ty, 32);
    for ch in chunks {
        w.handle_raw(ch).unwrap();
    }
    // The same columns with shared sections (another world holds them too).
    let cache = SharedChunks::new();
    let mut sw = world_with_registries(&f);
    let mut sw2 = world_with_registries(&f);
    for x in [&mut sw, &mut sw2] {
        x.set_shared_chunks(Some(cache.clone()));
        x.enter_dimension(name, *ty, 32);
        for ch in chunks {
            x.handle_raw(ch).unwrap();
        }
    }
    let mut g = c.benchmark_group("26.1 block reads");
    let boxed: Vec<(i32, i32, i32)> = (0..512)
        .map(|i| (-2 + (i % 5), 98 + ((i / 5) % 6), -2 + ((i / 30) % 5)))
        .collect();
    g.throughput(Throughput::Elements(boxed.len() as u64));
    g.bench_function("World::block_state 5x6x5 box", |b| {
        b.iter(|| {
            let mut h = 0u32;
            for &(x, y, z) in &boxed {
                h = h.wrapping_add(u32::from(w.block_state(x, y, z).unwrap().id()));
            }
            black_box(h)
        })
    });
    g.bench_function("World::block_state 5x6x5 box, shared", |b| {
        b.iter(|| {
            let mut h = 0u32;
            for &(x, y, z) in &boxed {
                h = h.wrapping_add(u32::from(sw.block_state(x, y, z).unwrap().id()));
            }
            black_box(h)
        })
    });
    let view = w.view(0, 0);
    g.bench_function("View::block_state 5x6x5 box", |b| {
        b.iter(|| {
            let mut h = 0u32;
            for &(x, y, z) in &boxed {
                h = h.wrapping_add(u32::from(view.block_state(x, y, z).unwrap().id()));
            }
            black_box(h)
        })
    });
    let dense = Dense::of(&w, [-32, -64, -32], [64, 384, 64]);
    g.bench_function("dense array 5x6x5 box", |b| {
        b.iter(|| {
            let mut h = 0u32;
            for &(x, y, z) in &boxed {
                h = h.wrapping_add(u32::from(dense.block(x, y, z).id()));
            }
            black_box(h)
        })
    });
    g.finish();

    // ---------------------------------------------------------------- physics
    let stone = BlockState::parse("stone").unwrap().id();
    let mut flat = world_with_registries(&f);
    flat.enter_dimension("minecraft:overworld", 0, 32);
    assert_eq!(flat.min_y(), Some(-64));
    for cx in -4..=4 {
        for cz in -4..=4 {
            flat.handle_raw(&flat_chunk(cx, cz, stone)).unwrap();
        }
    }
    assert_eq!(flat.block_state(5, 63, 5), BlockState::from_id(stone));
    let flat_cache = SharedChunks::new();
    let mut flat_shared = world_with_registries(&f);
    flat_shared.set_shared_chunks(Some(flat_cache.clone()));
    flat_shared.enter_dimension("minecraft:overworld", 0, 32);
    for cx in -4..=4 {
        for cz in -4..=4 {
            flat_shared.handle_raw(&flat_chunk(cx, cz, stone)).unwrap();
        }
    }
    assert_eq!(flat_cache.sections(), 1, "81 columns, one shared section");
    assert_eq!(flat.block_state(5, 64, 5), Some(BlockState::AIR));
    let mut mem = modern::memory_world_26_1();
    mem.load_chunks(-4, -4, 4, 4);
    mem.fill_raw([-64, 63, -64], [63, 63, 63], stone);
    let forward = TickInput {
        keys: Keys {
            forward: true,
            ..Keys::NONE
        },
        yaw: -45.0,
        pitch: 0.0,
        using_item: false,
    };
    let start = {
        let mut p = PlayerState::new([0.5, 64.0, 0.5], -45.0, 0.0);
        let mut out = Packets::new();
        for _ in 0..10 {
            out.clear();
            modern::tick::<V26_1, _>(&mut p, &forward, &mem, &mut out);
        }
        p
    };
    // Same result on every world.
    let a = run_ticks(&mem, start, &forward);
    let b = run_ticks(&flat, start, &forward);
    let v = flat.view(start.pos[0] as i32, start.pos[2] as i32);
    let cv = run_ticks(&v, start, &forward);
    let dense = Dense::of(&flat, [-32, 0, -32], [64, 128, 64]);
    let d = run_ticks(&dense, start, &forward);
    assert_eq!((a.pos, a.vel), (b.pos, b.vel));
    assert_eq!((a.pos, a.vel), (cv.pos, cv.vel));
    assert_eq!((a.pos, a.vel), (d.pos, d.vel));
    let e = run_ticks(&flat_shared, start, &forward);
    assert_eq!((a.pos, a.vel), (e.pos, e.vel));

    let mut g = c.benchmark_group("26.1 modern_tick walk_flat");
    g.throughput(Throughput::Elements(TICKS as u64));
    g.bench_function("physics memory_world_26_1", |b| {
        b.iter(|| run_ticks(&mem, start, &forward))
    });
    g.bench_function("dense array", |b| {
        b.iter(|| run_ticks(&dense, start, &forward))
    });
    g.bench_function("ghast-world World", |b| {
        b.iter(|| run_ticks(&flat, start, &forward))
    });
    g.bench_function("ghast-world World, shared", |b| {
        b.iter(|| run_ticks(&flat_shared, start, &forward))
    });
    g.bench_function("ghast-world View (taken per tick)", |b| {
        b.iter(|| {
            let mut out = Packets::new();
            let mut p = start;
            for _ in 0..TICKS {
                out.clear();
                let v = flat.view(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
                modern::tick::<V26_1, _>(&mut p, &forward, black_box(&v), &mut out);
            }
            p
        })
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
