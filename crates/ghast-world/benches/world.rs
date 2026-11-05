//! Chunk decode, block lookup and memory benchmarks.
//!
//! `cargo bench -p ghast-world` — memory figures are printed before the timings.

use criterion::{Criterion, criterion_group, criterion_main};
use ghast_world::WorldConfig;
use std::hint::black_box;

#[path = "../tests/common/mod.rs"]
mod common;
use common::{LegacySection, Palette16, Rng, encode_1_8, encode_section_1_16};

/// Synthetic overworld terrain as `[y][z][x]` raw ids of a few roles, for both versions.
#[derive(Clone, Copy)]
enum Role {
    Air,
    Bedrock,
    Stone,
    Ore(u8),
    Dirt,
    Grass,
    Water,
    Log,
    Leaves,
}

fn terrain(rng: &mut Rng) -> Vec<Role> {
    let mut v = vec![Role::Air; 65536];
    for z in 0..16 {
        for x in 0..16 {
            let h = 62 + (rng.below(5) as usize);
            for y in 0..=h {
                let r = if y == 0 || (y < 4 && rng.below(2) == 0) {
                    Role::Bedrock
                } else if y + 4 < h {
                    match rng.below(100) {
                        0..=2 => Role::Ore(rng.below(6) as u8),
                        3..=5 => Role::Air, // caves
                        6..=7 => Role::Water,
                        _ => Role::Stone,
                    }
                } else if y < h {
                    Role::Dirt
                } else {
                    Role::Grass
                };
                v[y << 8 | z << 4 | x] = r;
            }
            if rng.below(40) == 0 {
                for y in h + 1..h + 6 {
                    v[y << 8 | z << 4 | x] = Role::Log;
                }
                v[(h + 6) << 8 | z << 4 | x] = Role::Leaves;
            }
        }
    }
    v
}

fn id_1_8(r: Role) -> u16 {
    match r {
        Role::Air => 0,
        Role::Bedrock => 7 << 4,
        Role::Stone => 1 << 4,
        Role::Ore(k) => [14u16, 15, 16, 21, 56, 73][usize::from(k)] << 4,
        Role::Dirt => 3 << 4,
        Role::Grass => 2 << 4,
        Role::Water => 9 << 4,
        Role::Log => 17 << 4,
        Role::Leaves => 18 << 4,
    }
}

fn id_1_16(r: Role) -> u16 {
    use ghast_data::v1_16_5::Block;
    let b = match r {
        Role::Air => Block::AIR,
        Role::Bedrock => Block::BEDROCK,
        Role::Stone => Block::STONE,
        Role::Ore(k) => [
            Block::GOLD_ORE,
            Block::IRON_ORE,
            Block::COAL_ORE,
            Block::LAPIS_ORE,
            Block::DIAMOND_ORE,
            Block::REDSTONE_ORE,
        ][usize::from(k)],
        Role::Dirt => Block::DIRT,
        Role::Grass => Block::GRASS_BLOCK,
        Role::Water => Block::WATER,
        Role::Log => Block::OAK_LOG,
        Role::Leaves => Block::OAK_LEAVES,
    };
    b.default_state().id()
}

fn chunk_1_8(t: &[Role]) -> (u16, Vec<u8>) {
    let secs: [Option<LegacySection>; 16] = std::array::from_fn(|j| {
        let blocks: Vec<u16> = t[j * 4096..(j + 1) * 4096]
            .iter()
            .map(|&r| id_1_8(r))
            .collect();
        blocks.iter().any(|&b| b != 0).then(|| LegacySection {
            blocks,
            block_light: vec![0; 2048],
            sky_light: vec![0xFF; 2048],
        })
    });
    encode_1_8(&secs, true, Some(&[1; 256]))
}

fn chunk_1_16(t: &[Role]) -> (i32, Vec<u8>) {
    let mut data = Vec::new();
    let mut mask = 0;
    for j in 0..16 {
        let ids: Vec<u16> = t[j * 4096..(j + 1) * 4096]
            .iter()
            .map(|&r| id_1_16(r))
            .collect();
        if ids.iter().all(|&b| b == 0) {
            continue;
        }
        mask |= 1 << j;
        let mut distinct = ids.clone();
        distinct.sort_unstable();
        distinct.dedup();
        let bits = (usize::BITS - distinct.len().saturating_sub(1).leading_zeros()).max(4);
        encode_section_1_16(&mut data, &ids, Palette16::Indirect(bits as u8), &[]);
    }
    (mask, data)
}

fn benches(c: &mut Criterion) {
    let mut rng = Rng(0x5EED);
    let t = terrain(&mut rng);
    let (m8, d8) = chunk_1_8(&t);
    let (m16, d16) = chunk_1_16(&t);

    // A full 16-section 1.8 column (worst case: every section present, varied).
    let full8: Vec<Role> = (0..65536)
        .map(|i| match rng.below(10) {
            0 => Role::Ore((i % 6) as u8),
            1 => Role::Dirt,
            2 => Role::Air,
            _ => Role::Stone,
        })
        .collect();
    let (fm8, fd8) = chunk_1_8(&full8);
    let (fm16, fd16) = chunk_1_16(&full8);

    // Memory per loaded column (21 x 21 view, the default view distance 10).
    {
        let mut w = ghast_world::v1_8::World::new(WorldConfig::default());
        let mut w16 = ghast_world::v1_16_5::World::new(WorldConfig::default());
        let mut wl = ghast_world::v1_8::World::new(
            WorldConfig::default()
                .with_biomes(true)
                .with_light(true)
                .with_block_entities(false),
        );
        for cx in -10..=10 {
            for cz in -10..=10 {
                let t = terrain(&mut rng);
                let (m, d) = chunk_1_8(&t);
                w.apply_chunk_data(cx, cz, true, m, &d).unwrap();
                wl.apply_chunk_data(cx, cz, true, m, &d).unwrap();
                let (m, d) = chunk_1_16(&t);
                w16.apply_chunk_data(cx, cz, true, m, Some(&[1; 1024]), &d)
                    .unwrap();
            }
        }
        let n = w.columns().len();
        eprintln!(
            "terrain column: {} sections, wire bytes 1.8 {} / 1.16.5 {}",
            w.column(0, 0).unwrap().section_count(),
            d8.len(),
            d16.len()
        );
        eprintln!(
            "memory, 441 columns: 1.8 {} KiB ({} B/column), 1.8+light {} KiB, 1.16.5 {} KiB ({} B/column)",
            w.heap_bytes() / 1024,
            w.heap_bytes() / n,
            wl.heap_bytes() / 1024,
            w16.heap_bytes() / 1024,
            w16.heap_bytes() / n
        );
    }

    let mut g = c.benchmark_group("decode");
    let mut w8 = ghast_world::v1_8::World::new(WorldConfig::default());
    g.bench_function("1.8 terrain column", |b| {
        b.iter(|| w8.apply_chunk_data(0, 0, true, m8, black_box(&d8)).unwrap())
    });
    g.bench_function("1.8 full 16-section column", |b| {
        b.iter(|| {
            w8.apply_chunk_data(0, 0, true, fm8, black_box(&fd8))
                .unwrap()
        })
    });
    let mut w16 = ghast_world::v1_16_5::World::new(WorldConfig::default());
    let biomes = vec![1i32; 1024];
    g.bench_function("1.16.5 terrain column", |b| {
        b.iter(|| {
            w16.apply_chunk_data(0, 0, true, m16, Some(&biomes), black_box(&d16))
                .unwrap()
        })
    });
    g.bench_function("1.16.5 full 16-section column", |b| {
        b.iter(|| {
            w16.apply_chunk_data(0, 0, true, fm16, Some(&biomes), black_box(&fd16))
                .unwrap()
        })
    });
    g.finish();

    // Lookups: 441 loaded terrain columns, random positions around the surface.
    let mut w = ghast_world::v1_8::World::new(WorldConfig::default());
    for cx in -10..=10 {
        for cz in -10..=10 {
            w.apply_chunk_data(cx, cz, true, m8, &d8).unwrap();
        }
    }
    let pts: Vec<(i32, i32, i32)> = (0..4096)
        .map(|_| {
            (
                rng.below(336) as i32 - 168,
                50 + rng.below(30) as i32,
                rng.below(336) as i32 - 168,
            )
        })
        .collect();
    let mut g = c.benchmark_group("lookup");
    g.throughput(criterion::Throughput::Elements(pts.len() as u64));
    g.bench_function("1.8 block_state random", |b| {
        b.iter(|| {
            let mut acc = 0u32;
            for &(x, y, z) in &pts {
                acc += u32::from(w.block_state(x, y, z).map_or(0, |s| s.raw()));
            }
            acc
        })
    });
    // Physics-like access: a 3x4x3 box around a point, 36 lookups.
    g.bench_function("1.8 block_state box 3x4x3", |b| {
        b.iter(|| {
            let mut acc = 0u32;
            for &(px, py, pz) in &pts[..114] {
                for x in px - 1..=px + 1 {
                    for z in pz - 1..=pz + 1 {
                        for y in py - 1..=py + 2 {
                            acc += u32::from(w.block_state(x, y, z).map_or(0, |s| s.raw()));
                        }
                    }
                }
            }
            acc
        })
    });
    let col = w.column(0, 0).unwrap();
    g.bench_function("1.8 Column::raw (no hash)", |b| {
        b.iter(|| {
            let mut acc = 0u32;
            for &(x, y, z) in &pts {
                acc += u32::from(col.raw(x, y, z));
            }
            acc
        })
    });
    g.finish();
}

criterion_group!(world, benches);
criterion_main!(world);
