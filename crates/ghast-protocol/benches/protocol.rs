//! Protocol micro-benchmarks: VarInt, frame splitting, inflate (zlib-rs vs libdeflate),
//! CFB8 decrypt (batched vs serial), packet decode, and the filter's skip path.
//!
//! Run: `cargo bench -p ghast-protocol` (optionally `-- <filter>`).

use std::hint::black_box;

use bytes::{Bytes, BytesMut};
use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress};
use ghast_protocol::codec::{McWrite, Reader, decode_exact};
use ghast_protocol::crypto::{Cfb8Decryptor, Cfb8Encryptor};
use ghast_protocol::nbt::{Compound, List, NamedTag, RawNbt, Tag, tag};
use ghast_protocol::types::ByteString;
use ghast_protocol::types::slot::legacy::{ItemStack, Slot};
use ghast_protocol::v1_8::play::clientbound as cb;
use ghast_protocol::{Encode, FrameDecoder, FrameEncoder, PacketFilter, PacketSet, RawPacket};

/// Pseudo-random bytes (xorshift).
fn noise(n: usize, mut s: u64) -> Vec<u8> {
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s as u8
        })
        .collect()
}

/// 1.8-style chunk column payload: 16 sections of block ids (u16 LE: id << 4 | meta),
/// block light and sky light nibbles, biomes. Underground has caves and ~20 kinds of
/// ores/stone variants, surface has height noise; light has gradients and noise. It
/// compresses to roughly a fifth, in the range of real survival chunks.
fn chunk_like() -> Vec<u8> {
    let mut v = Vec::with_capacity(16 * 4096 * 3 + 256);
    let r = noise(4096 * 16 * 2, 7);
    let heights = noise(256, 99);
    for section in 0..16usize {
        for i in 0..4096usize {
            let y = section * 16 + i / 256;
            let col = i % 256;
            let surface = 60 + (heights[col] % 6) as usize;
            let n = r[section * 4096 + i];
            let id: u16 = if y == 0 {
                7 << 4
            } else if y < surface - 4 {
                match n {
                    0..=24 => 0,                                                // caves
                    25..=40 => (u16::from(n % 20) + 1) << 4 | u16::from(n % 4), // ores, variants
                    _ => 1 << 4,
                }
            } else if y < surface {
                3 << 4
            } else if y == surface {
                2 << 4
            } else if y == surface + 1 && n < 30 {
                31 << 4 | 1 // tall grass
            } else {
                0
            };
            v.extend_from_slice(&id.to_le_bytes());
        }
    }
    let r2 = noise(16 * 2048, 5);
    v.extend(
        r2.iter()
            .enumerate()
            .map(|(i, &x)| if i < 8 * 2048 { x & 0x11 } else { 0 }),
    ); // block light
    v.extend(
        r2.iter()
            .enumerate()
            .map(|(i, &x)| if i < 7 * 2048 { x & 0x33 } else { 0xff }),
    ); // sky light
    v.extend(heights.iter().map(|h| h % 3 + 1)); // biomes
    v
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut c = Compress::new(Compression::default(), true);
    let mut out = Vec::with_capacity(data.len() + 1024);
    c.compress_vec(data, &mut out, FlushCompress::Finish)
        .unwrap();
    out
}

/// An item with typical server NBT: display name, 4 lore lines, 2 enchantments.
fn fancy_item(i: i16) -> Slot {
    let lore = (0..4)
        .map(|k| Tag::String(format!("\u{a7}7Line {k} of the item lore for item {i}")))
        .collect();
    let ench = (0..2)
        .map(|k| {
            Tag::Compound(Compound(vec![
                ("id".into(), Tag::Short(16 + k)),
                ("lvl".into(), Tag::Short(5)),
            ]))
        })
        .collect();
    let display = Compound(vec![
        (
            "Name".into(),
            Tag::String(format!("\u{a7}6\u{a7}lLegendary Sword #{i}")),
        ),
        (
            "Lore".into(),
            Tag::List(List {
                elem_type: tag::STRING,
                items: lore,
            }),
        ),
    ]);
    let root = NamedTag {
        name: String::new(),
        tag: Tag::Compound(Compound(vec![
            ("display".into(), Tag::Compound(display)),
            (
                "ench".into(),
                Tag::List(List {
                    elem_type: tag::COMPOUND,
                    items: ench,
                }),
            ),
            ("HideFlags".into(), Tag::Int(1)),
        ])),
    };
    Slot(Some(ItemStack {
        id: 276,
        count: 1,
        damage: 0,
        nbt: Some(RawNbt::from_named(&root).unwrap()),
    }))
}

/// A compressed frame (`id | body`) made with flate2, i.e. a zlib-style encoder like the
/// Java `Deflater` servers use.
fn zlib_frame(id: i32, body: &[u8]) -> Vec<u8> {
    let mut data = BytesMut::new();
    data.put_varint(id);
    data.extend_from_slice(body);
    let zc = zlib(&data);
    let mut f = BytesMut::new();
    f.put_varint((ghast_protocol::codec::varint_len(data.len() as i32) + zc.len()) as i32);
    f.put_varint(data.len() as i32);
    f.extend_from_slice(&zc);
    f.to_vec()
}

fn window_items_body() -> Bytes {
    // A 54-slot chest + player inventory (90 slots), 2/3 of them NBT-heavy items.
    let items = (0..90)
        .map(|i| {
            if i % 3 == 0 {
                Slot(None)
            } else {
                fancy_item(i)
            }
        })
        .collect();
    let p = cb::WindowItems {
        window_id: 1,
        items,
    };
    let mut w = BytesMut::new();
    p.encode(&mut w).unwrap();
    w.freeze()
}

fn bench_varint(c: &mut Criterion) {
    let mut g = c.benchmark_group("varint");
    let vals: Vec<i32> = (0..1024)
        .map(|i| [5, 300, 70_000, 25_000_000, -1][i % 5])
        .collect();
    let mut w = BytesMut::new();
    for &v in &vals {
        w.put_varint(v);
    }
    let buf = w.freeze();
    g.throughput(Throughput::Elements(vals.len() as u64));
    g.bench_function("decode_mixed_1024", |b| {
        b.iter(|| {
            let mut r = Reader::new(&buf);
            let mut s = 0i64;
            while r.remaining() > 0 {
                s += i64::from(r.read_varint().unwrap());
            }
            black_box(s)
        })
    });
    let ones: Vec<i32> = (0..1024).map(|i| i % 128).collect();
    let mut w = BytesMut::new();
    for &v in &ones {
        w.put_varint(v);
    }
    let buf1 = w.freeze();
    g.bench_function("decode_1byte_1024", |b| {
        b.iter(|| {
            let mut r = Reader::new(&buf1);
            let mut s = 0i64;
            while r.remaining() > 0 {
                s += i64::from(r.read_varint().unwrap());
            }
            black_box(s)
        })
    });
    g.bench_function("encode_mixed_1024", |b| {
        let mut w = BytesMut::with_capacity(8192);
        b.iter(|| {
            w.clear();
            for &v in &vals {
                w.put_varint(black_box(v));
            }
            black_box(w.len())
        })
    });
    g.finish();
}

fn bench_frames(c: &mut Criterion) {
    let mut g = c.benchmark_group("frame");
    // 1000 small packets (movement/keepalive sized), uncompressed.
    let mut enc = FrameEncoder::new();
    for i in 0..1000 {
        enc.write_raw(&RawPacket::new(
            i % 0x40,
            Bytes::from(noise(12 + (i as usize % 30), i as u64 + 1)),
        ))
        .unwrap();
    }
    let wire = enc.take().freeze();
    g.throughput(Throughput::Elements(1000));
    g.bench_function("split_1000_small", |b| {
        b.iter_batched(
            FrameDecoder::new,
            |mut d| {
                d.push(&wire);
                let mut n = 0;
                while let Some(p) = d.next_packet().unwrap() {
                    n += p.body.len();
                }
                black_box(n)
            },
            BatchSize::SmallInput,
        )
    });
    // Same with compression on (threshold 256 -> mostly "uncompressed" marker frames).
    let mut enc = FrameEncoder::new();
    enc.set_compression(256);
    for i in 0..1000 {
        enc.write_raw(&RawPacket::new(
            i % 0x40,
            Bytes::from(noise(12 + (i as usize % 30), i as u64 + 1)),
        ))
        .unwrap();
    }
    let wire_c = enc.take().freeze();
    g.bench_function("split_1000_small_compression_on", |b| {
        b.iter_batched(
            || {
                let mut d = FrameDecoder::new();
                d.set_compression(256);
                d
            },
            |mut d| {
                d.push(&wire_c);
                let mut n = 0;
                while let Some(p) = d.next_packet().unwrap() {
                    n += p.body.len();
                }
                black_box(n)
            },
            BatchSize::SmallInput,
        )
    });
    g.finish();
}

fn bench_inflate(c: &mut Criterion) {
    let chunk = chunk_like();
    let z = zlib(&chunk);
    eprintln!(
        "chunk-like payload: {} bytes -> {} compressed",
        chunk.len(),
        z.len()
    );
    let mut g = c.benchmark_group("inflate_chunk");
    g.throughput(Throughput::Bytes(chunk.len() as u64));
    g.bench_function("flate2_zlib_rs", |b| {
        let mut d = Decompress::new(true);
        b.iter(|| {
            d.reset(true);
            let mut out = Vec::with_capacity(chunk.len());
            d.decompress_vec(&z, &mut out, FlushDecompress::Finish)
                .unwrap();
            black_box(out.len())
        })
    });
    g.bench_function("libdeflater", |b| {
        let mut d = libdeflater::Decompressor::new();
        let mut out = vec![0u8; chunk.len()];
        b.iter(|| black_box(d.zlib_decompress(&z, &mut out).unwrap()))
    });
    g.finish();

    // The same chunk as a compressed frame through the decoder: no filter, a filter that
    // wants it (peek + full inflate), and a filter that drops it (peek only).
    // Built by hand with flate2 (zlib-style encoder, like a Java server's Deflater):
    // libdeflate's own output inflates measurably slower, which is not what servers send.
    let frame = Bytes::from(zlib_frame(0x21, &chunk));
    let mut g = c.benchmark_group("frame_chunk");
    g.throughput(Throughput::Bytes(chunk.len() as u64));
    for (label, filter) in [
        ("no_filter", PacketFilter::all()),
        ("filter_wants_it", PacketFilter::all().without(0x38)),
        ("filter_drops_it", PacketFilter::all().without(0x21)),
    ] {
        g.bench_function(label, |b| {
            let mut d = FrameDecoder::new();
            d.set_compression(256);
            d.set_filter(filter);
            b.iter(|| {
                d.push(&frame);
                black_box(d.next_packet().unwrap().map(|p| p.body.len()))
            })
        });
    }
    // Retained case: the caller keeps the last 16 chunk packets alive (e.g. a world
    // that stores chunk data zero-copy), so decoder buffers cannot be recycled at once.
    g.bench_function("no_filter_retain_16", |b| {
        let mut d = FrameDecoder::new();
        d.set_compression(256);
        let mut keep = std::collections::VecDeque::new();
        b.iter(|| {
            d.push(&frame);
            let p = d.next_packet().unwrap().unwrap();
            keep.push_back(p);
            if keep.len() > 16 {
                keep.pop_front();
            }
            black_box(keep.len())
        })
    });
    g.finish();

    // window_items-sized compressed frame, and a stream of mid-sized compressed packets.
    let wi = window_items_body();
    let wi_frame = Bytes::from(zlib_frame(0x30, &wi));
    let mut mid = Vec::new();
    let mut mid_bytes = 0;
    for i in 0..100u64 {
        let n = 300 + (i as usize * 37) % 1800;
        let mut body = noise(n / 4, i + 1);
        body.resize(n, 0);
        mid_bytes += n;
        mid.extend(zlib_frame((i % 0x40) as i32, &body));
    }
    let mid_stream = Bytes::from(mid);
    let mut g = c.benchmark_group("frame_inflate");
    g.throughput(Throughput::Bytes(wi.len() as u64));
    g.bench_function("window_items_17k", |b| {
        let mut d = FrameDecoder::new();
        d.set_compression(256);
        b.iter(|| {
            d.push(&wi_frame);
            black_box(d.next_packet().unwrap().map(|p| p.body.len()))
        })
    });
    g.throughput(Throughput::Bytes(mid_bytes as u64));
    g.bench_function("stream_100_mid_packets", |b| {
        let mut d = FrameDecoder::new();
        d.set_compression(256);
        b.iter(|| {
            d.push(&mid_stream);
            let mut n = 0;
            while let Some(p) = d.next_packet().unwrap() {
                n += p.body.len();
            }
            black_box(n)
        })
    });
    g.finish();

    // Output-buffer strategies for a whole-chunk inflate (same data, same run):
    // fresh zeroed Vec per packet, recycled zeroed block, and a reused buffer (lower
    // bound: no allocation, no zeroing).
    let mut g = c.benchmark_group("inflate_out_buffer");
    g.throughput(Throughput::Bytes(chunk.len() as u64));
    g.bench_function("fresh_vec", |b| {
        let mut d = libdeflater::Decompressor::new();
        b.iter(|| {
            let mut out = vec![0u8; chunk.len()];
            d.zlib_decompress(&z, &mut out).unwrap();
            black_box(Bytes::from(out))
        })
    });
    g.bench_function("recycled_block_256k", |b| {
        let mut d = libdeflater::Decompressor::new();
        let mut pool = BytesMut::new();
        b.iter(|| {
            if pool.len() < chunk.len() {
                pool.clear();
                pool.reserve(256 * 1024);
                pool.resize(256 * 1024, 0);
            }
            let mut out = pool.split_to(chunk.len());
            d.zlib_decompress(&z, &mut out).unwrap();
            black_box(out.freeze())
        })
    });
    g.bench_function("reused_buffer_lower_bound", |b| {
        let mut d = libdeflater::Decompressor::new();
        let mut out = vec![0u8; chunk.len()];
        b.iter(|| black_box(d.zlib_decompress(&z, &mut out).unwrap()))
    });
    let z_ld = {
        let mut c = libdeflater::Compressor::new(libdeflater::CompressionLvl::default());
        let mut out = vec![0u8; c.zlib_compress_bound(chunk.len())];
        let n = c.zlib_compress(&chunk, &mut out).unwrap();
        out.truncate(n);
        out
    };
    eprintln!("libdeflate-compressed chunk: {} bytes", z_ld.len());
    g.bench_function("reused_buffer_libdeflate_stream", |b| {
        let mut d = libdeflater::Decompressor::new();
        let mut out = vec![0u8; chunk.len()];
        b.iter(|| black_box(d.zlib_decompress(&z_ld, &mut out).unwrap()))
    });
    g.bench_function("memset_only", |b| {
        let mut out = vec![0u8; chunk.len()];
        b.iter(|| {
            out.fill(black_box(0));
            black_box(out[7])
        })
    });
    g.finish();

    let mut g = c.benchmark_group("deflate_small");
    let small = noise(300, 3);
    g.throughput(Throughput::Bytes(small.len() as u64));
    g.bench_function("flate2_zlib_rs_300B", |b| {
        let mut c = Compress::new(Compression::default(), true);
        let mut out = Vec::with_capacity(1024);
        b.iter(|| {
            c.reset();
            out.clear();
            c.compress_vec(&small, &mut out, FlushCompress::Finish)
                .unwrap();
            black_box(out.len())
        })
    });
    g.bench_function("flate2_zlib_rs_300B_level1", |b| {
        let mut c = Compress::new(Compression::fast(), true);
        let mut out = Vec::with_capacity(1024);
        b.iter(|| {
            c.reset();
            out.clear();
            c.compress_vec(&small, &mut out, FlushCompress::Finish)
                .unwrap();
            black_box(out.len())
        })
    });
    g.bench_function("libdeflater_300B", |b| {
        let mut c = libdeflater::Compressor::new(libdeflater::CompressionLvl::default());
        let mut out = vec![0u8; 1024];
        b.iter(|| black_box(c.zlib_compress(&small, &mut out).unwrap()))
    });
    g.finish();
}

fn bench_crypto(c: &mut Criterion) {
    use aes::Aes128;
    use aes::cipher::KeyIvInit;
    let key = [0x42u8; 16];
    let data = noise(64 * 1024, 11);
    let mut g = c.benchmark_group("cfb8_64KiB");
    g.throughput(Throughput::Bytes(data.len() as u64));
    g.bench_function("decrypt_batched", |b| {
        let mut d = Cfb8Decryptor::new(&key);
        let mut buf = data.clone();
        b.iter(|| {
            d.decrypt(&mut buf);
            black_box(buf[0])
        })
    });
    g.bench_function("decrypt_serial_cfb8_crate", |b| {
        let mut d = cfb8::Decryptor::<Aes128>::new(&key.into(), &key.into());
        let mut buf = data.clone();
        b.iter(|| {
            d.decrypt(&mut buf);
            black_box(buf[0])
        })
    });
    g.bench_function("encrypt", |b| {
        let mut e = Cfb8Encryptor::new(&key);
        let mut buf = data.clone();
        b.iter(|| {
            e.encrypt(&mut buf);
            black_box(buf[0])
        })
    });
    g.finish();
    eprintln!("aes hardware acceleration: {}", aes::hardware_accelerated());
}

fn bench_decode(c: &mut Criterion) {
    let mut g = c.benchmark_group("decode");
    // window_items with heavy NBT: lazy (skip) vs parsing every NBT tree.
    let body = window_items_body();
    eprintln!("window_items body: {} bytes", body.len());
    g.throughput(Throughput::Bytes(body.len() as u64));
    g.bench_function("window_items_90_slots_lazy_nbt", |b| {
        b.iter(|| {
            let p: cb::WindowItems = decode_exact(black_box(&body)).unwrap();
            black_box(p.items.len())
        })
    });
    g.bench_function("window_items_90_slots_parse_all_nbt", |b| {
        b.iter(|| {
            let p: cb::WindowItems = decode_exact(black_box(&body)).unwrap();
            let mut n = 0;
            for s in &p.items {
                if let Some(nbt) = s.item().and_then(|i| i.nbt.as_ref()) {
                    n += usize::from(nbt.parse().unwrap().is_some());
                }
            }
            black_box(n)
        })
    });
    // 1.8 map_chunk with a full column: zero-copy chunk data.
    let chunk = Bytes::from(chunk_like());
    let mc = cb::MapChunk {
        x: 3,
        z: -7,
        ground_up: true,
        bit_map: 0xffff,
        chunk_data: chunk,
    };
    let mut w = BytesMut::new();
    mc.encode(&mut w).unwrap();
    let mc_body = w.freeze();
    g.throughput(Throughput::Bytes(mc_body.len() as u64));
    g.bench_function("map_chunk_1_8_full_column", |b| {
        b.iter(|| {
            let p: cb::MapChunk = decode_exact(black_box(&mc_body)).unwrap();
            black_box(p.chunk_data.len())
        })
    });
    // Chat: zero-copy ByteString vs allocating a String.
    let msg = r#"{"extra":[{"color":"gold","text":"[Server] "},{"color":"white","text":"Welcome to the server, have fun and follow the rules!"}],"text":""}"#;
    let chat = cb::Chat {
        message: ByteString::from(msg),
        position: 0,
    };
    let mut w = BytesMut::new();
    chat.encode(&mut w).unwrap();
    let chat_body = w.freeze();
    g.throughput(Throughput::Elements(1));
    g.bench_function("chat_zero_copy", |b| {
        b.iter(|| {
            let p: cb::Chat = decode_exact(black_box(&chat_body)).unwrap();
            black_box(p.message.len())
        })
    });
    g.bench_function("chat_string_alloc_baseline", |b| {
        b.iter(|| {
            let mut r = Reader::new(black_box(&chat_body));
            let n = r.read_varint().unwrap() as usize;
            let s = String::from_utf8(r.take(n).unwrap().to_vec()).unwrap();
            let pos = r.read_i8().unwrap();
            black_box((s.len(), pos))
        })
    });
    // Typed dispatch through the packet-set enum.
    let raw = RawPacket::new(0x30, body.clone());
    g.bench_function("dispatch_enum_window_items", |b| {
        b.iter(|| {
            black_box(
                cb::ClientboundPlayPacket::decode_raw(black_box(&raw))
                    .unwrap()
                    .id(),
            )
        })
    });
    g.finish();
}

/// A compressed busy-server stream: tab-list/team/particle spam plus a few wanted
/// packets; compare decoding everything with dropping the spam via the filter.
fn bench_filter(c: &mut Criterion) {
    use ghast_protocol::types::Uuid;
    let mut enc = FrameEncoder::new();
    enc.set_compression(256);
    let mut wanted = 0;
    for i in 0..300u32 {
        let pi = cb::PlayerInfo {
            action: cb::PlayerInfoAction::UPDATE_LATENCY,
            data: (0..40u32)
                .map(|k| cb::PlayerInfoData {
                    uuid: Uuid::from_u128(u128::from(k) * 7919 + u128::from(i)),
                    body: cb::PlayerInfoDataBody::UpdateLatency(
                        cb::PlayerInfoDataBodyUpdateLatency {
                            ping: (k * 3) as i32,
                        },
                    ),
                })
                .collect(),
        };
        enc.write_packet(&pi).unwrap();
        let wp = cb::WorldParticles {
            particle_id: 3,
            long_distance: false,
            x: 1.0,
            y: 2.0,
            z: 3.0,
            offset_x: 0.1,
            offset_y: 0.1,
            offset_z: 0.1,
            particle_data: 0.0,
            particles: 20,
            data: cb::WorldParticlesData::Other,
        };
        for _ in 0..5 {
            enc.write_packet(&wp).unwrap();
        }
        if i % 10 == 0 {
            enc.write_packet(&cb::KeepAlive {
                keep_alive_id: i as i32,
            })
            .unwrap();
            wanted += 1;
        }
    }
    let wire = enc.take().freeze();
    let filter = PacketFilter::none().with(cb::id::KEEP_ALIVE);
    let mut g = c.benchmark_group("filter_busy_stream");
    g.throughput(Throughput::Bytes(wire.len() as u64));
    g.bench_function("decode_everything", |b| {
        b.iter_batched(
            || {
                let mut d = FrameDecoder::new();
                d.set_compression(256);
                d
            },
            |mut d| {
                d.push(&wire);
                let mut n = 0;
                while let Some(p) = d.next_packet().unwrap() {
                    let p = cb::ClientboundPlayPacket::decode_raw(&p).unwrap();
                    n += usize::from(matches!(p, cb::ClientboundPlayPacket::KeepAlive(_)));
                }
                assert_eq!(n, wanted);
            },
            BatchSize::SmallInput,
        )
    });
    g.bench_function("filtered_keepalive_only", |b| {
        b.iter_batched(
            || {
                let mut d = FrameDecoder::new();
                d.set_compression(256);
                d.set_filter(filter);
                d
            },
            |mut d| {
                d.push(&wire);
                let mut n = 0;
                while let Some(p) = d.next_packet().unwrap() {
                    let p = cb::ClientboundPlayPacket::decode_raw(&p).unwrap();
                    n += usize::from(matches!(p, cb::ClientboundPlayPacket::KeepAlive(_)));
                }
                assert_eq!(n, wanted);
            },
            BatchSize::SmallInput,
        )
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_varint,
    bench_frames,
    bench_inflate,
    bench_crypto,
    bench_decode,
    bench_filter
);
criterion_main!(benches);
