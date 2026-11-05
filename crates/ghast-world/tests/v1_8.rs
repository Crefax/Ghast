//! 1.8 world: decode round trips against a vanilla-format encoder and packet semantics.
#![cfg(feature = "v1_8")]

mod common;

use bytes::Bytes;
use common::{LegacySection, Rng, encode_1_8};
use ghast_data::v1_8::{Block, BlockState};
use ghast_protocol::BlockPos;
use ghast_protocol::v1_8::play::clientbound::{
    BlockChange, ClientboundPlayPacket, Explosion, ExplosionAffectedBlockOffsets, Login, MapChunk,
    MapChunkBulk, MapChunkBulkMeta, MultiBlockChange, MultiBlockChangeRecords, Respawn,
};
use ghast_world::v1_8::World;
use ghast_world::{Applied, Biomes, WorldConfig, WorldError};

fn registered(rng: &mut Rng) -> u16 {
    loop {
        let raw = rng.below(198 * 16) as u16;
        if BlockState::from_raw(raw).unwrap().is_registered() {
            return raw;
        }
    }
}

/// A random section: `kind` 0 = few states, 1 = many (> 256 distinct), 2 = raw garbage.
fn section(rng: &mut Rng, kind: u32) -> LegacySection {
    let pool: Vec<u16> = (0..8).map(|_| registered(rng)).collect();
    let blocks = (0..4096)
        .map(|_| match kind {
            0 => pool[rng.below(8) as usize],
            1 => registered(rng),
            _ => rng.next() as u16,
        })
        .collect();
    LegacySection {
        blocks,
        block_light: (0..2048).map(|_| rng.next() as u8).collect(),
        sky_light: (0..2048).map(|_| rng.next() as u8).collect(),
    }
}

fn random_column(rng: &mut Rng) -> [Option<LegacySection>; 16] {
    std::array::from_fn(|_| match rng.below(4) {
        0 => None,
        k => Some(section(rng, (k - 1) as u32)),
    })
}

fn map_chunk(x: i32, z: i32, ground_up: bool, mask: u16, data: Vec<u8>) -> MapChunk {
    MapChunk {
        x,
        z,
        ground_up,
        bit_map: mask,
        chunk_data: Bytes::from(data),
    }
}

fn check_column(w: &World, cx: i32, cz: i32, secs: &[Option<LegacySection>; 16]) {
    for (j, s) in secs.iter().enumerate() {
        for i in 0..4096usize {
            let (x, y, z) = (
                cx * 16 + (i & 15) as i32,
                (j * 16 + (i >> 8)) as i32,
                cz * 16 + ((i >> 4) & 15) as i32,
            );
            let expect = s.as_ref().map_or(BlockState::AIR, |s| {
                BlockState::from_raw_vanilla(s.blocks[i])
            });
            assert_eq!(w.block_state(x, y, z), Some(expect), "{x} {y} {z}");
        }
    }
}

#[test]
fn ground_up_round_trip() {
    let mut rng = Rng(0x1234_5678);
    for (dim, sky) in [(0i8, true), (-1, false), (1, false)] {
        let mut w = World::new(WorldConfig {
            biomes: true,
            light: true,
        });
        w.apply_login(&login(dim));
        for n in 0..6 {
            let secs = random_column(&mut rng);
            let biomes: [u8; 256] = std::array::from_fn(|i| i as u8 ^ 0x5A);
            let (mask, data) = encode_1_8(&secs, sky, Some(&biomes));
            if mask == 0 {
                continue;
            }
            let (cx, cz) = (n - 3, 7 - n * 5);
            let r = w.apply_map_chunk(&map_chunk(cx, cz, true, mask, data));
            assert_eq!(r, Ok(Applied::Changed));
            check_column(&w, cx, cz, &secs);
            let col = w.column(cx, cz).unwrap();
            match col.biomes() {
                Biomes::Legacy(b) => assert_eq!(**b, biomes),
                other => panic!("{other:?}"),
            }
            for (j, s) in secs.iter().enumerate() {
                let Some(s) = s else { continue };
                for i in [0usize, 1, 2047, 4094, 4095] {
                    let (x, y, z) = (
                        (i & 15) as i32,
                        (j * 16 + (i >> 8)) as i32,
                        ((i >> 4) & 15) as i32,
                    );
                    let byte = s.block_light[i >> 1];
                    let nib = if i & 1 == 0 { byte & 15 } else { byte >> 4 };
                    assert_eq!(col.block_light(x, y, z), Some(nib));
                    let sky_byte = s.sky_light[i >> 1];
                    let sky_nib = if i & 1 == 0 {
                        sky_byte & 15
                    } else {
                        sky_byte >> 4
                    };
                    assert_eq!(col.sky_light(x, y, z), sky.then_some(sky_nib));
                }
            }
        }
    }
}

#[test]
fn sky_light_inferred_from_length() {
    // Overworld data received while the client thinks it is in the nether (and vice versa):
    // the biome bytes must still be found.
    let mut rng = Rng(9);
    for (dim, sky) in [(-1i8, true), (0, false)] {
        let mut w = World::new(WorldConfig::default());
        w.apply_login(&login(dim));
        let mut secs: [Option<LegacySection>; 16] = Default::default();
        secs[3] = Some(section(&mut rng, 0));
        let biomes = [7u8; 256];
        let (mask, data) = encode_1_8(&secs, sky, Some(&biomes));
        w.apply_map_chunk(&map_chunk(0, 0, true, mask, data))
            .unwrap();
        check_column(&w, 0, 0, &secs);
        assert!(matches!(w.column(0, 0).unwrap().biomes(), Biomes::Legacy(b) if **b == biomes));
    }
}

#[test]
fn unregistered_states_read_as_air() {
    let mut secs: [Option<LegacySection>; 16] = Default::default();
    let mut blocks = vec![0u16; 4096];
    blocks[0] = 1 << 4 | 7; // stone:7 (unregistered meta)
    blocks[1] = 250 << 4; // unknown id
    blocks[2] = 1 << 4; // stone
    secs[0] = Some(LegacySection {
        blocks,
        block_light: vec![0; 2048],
        sky_light: vec![0; 2048],
    });
    let (mask, data) = encode_1_8(&secs, true, Some(&[0; 256]));
    let mut w = World::new(WorldConfig::default());
    w.apply_map_chunk(&map_chunk(0, 0, true, mask, data))
        .unwrap();
    assert_eq!(w.block_state(0, 0, 0), Some(BlockState::AIR));
    assert_eq!(w.block_state(1, 0, 0), Some(BlockState::AIR));
    assert_eq!(w.block_state(2, 0, 0), Some(Block::STONE.default_state()));
}

#[test]
fn load_unload_partial() {
    let mut rng = Rng(77);
    let mut w = World::new(WorldConfig::default());
    assert_eq!(w.block_state(0, 64, 0), None);

    // Partial chunk for an unloaded column: ignored.
    let mut secs: [Option<LegacySection>; 16] = Default::default();
    secs[4] = Some(section(&mut rng, 0));
    let (mask, data) = encode_1_8(&secs, true, None);
    assert_eq!(
        w.apply_map_chunk(&map_chunk(2, 2, false, mask, data.clone())),
        Ok(Applied::Ignored)
    );
    assert!(!w.is_column_loaded(2, 2));

    // Ground-up load, then a partial update of one section keeps the others.
    let full = random_column(&mut rng);
    let mut full = full;
    full[0] = Some(section(&mut rng, 0));
    let (fmask, fdata) = encode_1_8(&full, true, Some(&[1; 256]));
    w.apply_map_chunk(&map_chunk(2, 2, true, fmask, fdata.clone()))
        .unwrap();
    check_column(&w, 2, 2, &full);
    w.apply_map_chunk(&map_chunk(2, 2, false, mask, data))
        .unwrap();
    let mut merged = full;
    merged[4] = secs[4].take();
    check_column(&w, 2, 2, &merged);
    // Biomes untouched by the partial chunk.
    assert!(matches!(w.column(2, 2).unwrap().biomes(), Biomes::Legacy(b) if b[0] == 1));

    // Ground-up again replaces everything (sections outside the mask become air).
    let mut only0: [Option<LegacySection>; 16] = Default::default();
    only0[0] = Some(section(&mut rng, 0));
    let (m, d) = encode_1_8(&only0, true, Some(&[0; 256]));
    w.apply_map_chunk(&map_chunk(2, 2, true, m, d)).unwrap();
    check_column(&w, 2, 2, &only0);

    // Ground-up with an empty mask unloads.
    assert_eq!(
        w.apply_map_chunk(&map_chunk(2, 2, true, 0, vec![])),
        Ok(Applied::Changed)
    );
    assert!(!w.is_column_loaded(2, 2));
    assert_eq!(w.block_state(32, 0, 32), None);
}

#[test]
fn truncated_ground_up_keeps_an_empty_column() {
    // Seen on a real server (End auth lobby): ground-up, mask 1, only the 256 biome bytes.
    let mut w = World::new(WorldConfig::default());
    w.apply_login(&login(1));
    let biomes: Vec<u8> = (0..=255u8).map(|i| i.wrapping_mul(7)).collect();
    let r = w.apply_map_chunk(&map_chunk(0, 0, true, 1, biomes.clone()));
    assert!(matches!(r, Err(WorldError::Truncated { .. })), "{r:?}");
    assert!(
        w.is_column_loaded(0, 0),
        "vanilla has already created the column"
    );
    // The first 128 blocks of section 0 come from the bytes (as vanilla reads them), the rest is air.
    for i in 0..4096usize {
        let (x, y, z) = ((i & 15) as i32, (i >> 8) as i32, ((i >> 4) & 15) as i32);
        let expect = if i < 128 {
            BlockState::from_raw_vanilla(u16::from_le_bytes([biomes[2 * i], biomes[2 * i + 1]]))
        } else {
            BlockState::AIR
        };
        assert_eq!(w.block_state(x, y, z), Some(expect), "index {i}");
    }
    assert_eq!(w.block_state(0, 100, 0), Some(BlockState::AIR));
    assert!(matches!(w.column(0, 0).unwrap().biomes(), Biomes::None));
}

/// Dense model of vanilla 1.8 `Chunk.fillChunk` block copying on short data.
fn vanilla_fill(old: Option<&[u16]>, ground_up: bool, mask: u16, data: &[u8]) -> Vec<u16> {
    let mut col = match (ground_up, old) {
        (false, Some(o)) => o.to_vec(),
        _ => vec![0u16; 65536],
    };
    let mut i = 0usize;
    for j in 0..16 {
        if mask & (1 << j) != 0 {
            for k in 0..4096 {
                if i + 1 >= data.len() {
                    return col;
                }
                let raw = u16::from(data[i + 1]) << 8 | u16::from(data[i]);
                col[j * 4096 + k] = BlockState::from_raw_vanilla(raw).raw();
                i += 2;
            }
        } else if ground_up {
            col[j * 4096..(j + 1) * 4096].fill(0);
        }
    }
    col
}

#[test]
fn truncated_chunks_match_vanilla_model() {
    let mut rng = Rng(0xC0FFEE);
    for round in 0..60 {
        let mut w = World::new(WorldConfig::default());
        let old = random_column(&mut rng);
        let (omask, odata) = encode_1_8(&old, true, Some(&[0; 256]));
        if omask == 0 {
            continue;
        }
        w.apply_map_chunk(&map_chunk(0, 0, true, omask, odata))
            .unwrap();
        let before: Vec<u16> = (0..65536usize)
            .map(|i| {
                let (x, y, z) = ((i & 15) as i32, (i >> 8) as i32, ((i >> 4) & 15) as i32);
                w.block_state(x, y, z).unwrap().raw()
            })
            .collect();
        let new = random_column(&mut rng);
        let (mask, data) = encode_1_8(&new, true, Some(&[0; 256]));
        if mask == 0 {
            continue;
        }
        let ground_up = round % 2 == 0;
        let cut = rng.below(mask.count_ones() as u64 * 8192) as usize;
        let short = &data[..cut];
        let r = w.apply_map_chunk(&map_chunk(0, 0, ground_up, mask, short.to_vec()));
        assert!(r.is_err());
        let expect = vanilla_fill(Some(&before), ground_up, mask, short);
        for (i, &e) in expect.iter().enumerate() {
            let (x, y, z) = ((i & 15) as i32, (i >> 8) as i32, ((i >> 4) & 15) as i32);
            assert_eq!(
                w.block_state(x, y, z).map(BlockState::raw),
                Some(e),
                "round {round} ground_up {ground_up} cut {cut} index {i}"
            );
        }
    }
}

#[test]
fn bulk() {
    let mut rng = Rng(11);
    let mut w = World::new(WorldConfig::default());
    let cols: Vec<[Option<LegacySection>; 16]> = (0..4)
        .map(|k| {
            if k == 2 {
                Default::default() // empty mask: loads an empty column in a bulk
            } else {
                random_column(&mut rng)
            }
        })
        .collect();
    let mut data = Vec::new();
    let mut meta = Vec::new();
    for (k, c) in cols.iter().enumerate() {
        let (mask, d) = encode_1_8(c, true, Some(&[k as u8; 256]));
        data.extend_from_slice(&d);
        meta.push(MapChunkBulkMeta {
            x: k as i32,
            z: -(k as i32),
            bit_map: mask,
        });
    }
    let p = MapChunkBulk {
        sky_light_sent: true,
        meta: meta.clone(),
        data: Bytes::from(data.clone()),
    };
    assert_eq!(w.apply_map_chunk_bulk(&p), Ok(Applied::Changed));
    for (k, c) in cols.iter().enumerate() {
        check_column(&w, k as i32, -(k as i32), c);
    }
    assert!(w.is_column_loaded(2, -2));

    let mut w2 = World::new(WorldConfig::default());
    let p = MapChunkBulk {
        sky_light_sent: true,
        meta,
        data: Bytes::from(data[..data.len() - 1].to_vec()),
    };
    assert!(matches!(
        w2.apply_map_chunk_bulk(&p),
        Err(WorldError::BulkTruncated { .. })
    ));
    assert_eq!(w2.columns().len(), 0, "nothing applied");
}

fn loaded_world() -> World {
    let mut w = World::new(WorldConfig::default());
    for (cx, cz) in [(0, 0), (-1, -1), (-1, 0), (0, -1)] {
        let mut secs: [Option<LegacySection>; 16] = Default::default();
        secs[4] = Some(LegacySection {
            blocks: vec![1 << 4; 4096],
            block_light: vec![0; 2048],
            sky_light: vec![0; 2048],
        });
        let (m, d) = encode_1_8(&secs, true, Some(&[0; 256]));
        w.apply_map_chunk(&map_chunk(cx, cz, true, m, d)).unwrap();
    }
    w
}

#[test]
fn block_changes() {
    let mut w = loaded_world();
    let stone = Some(Block::STONE.default_state());
    let dirt = Block::DIRT.default_state();
    let set = |w: &mut World, x, y, z, t| {
        w.apply_block_change(&BlockChange {
            location: BlockPos { x, y, z },
            r#type: t,
        })
    };
    assert_eq!(
        set(&mut w, -1, 70, -1, i32::from(dirt.raw())),
        Applied::Changed
    );
    assert_eq!(w.block_state(-1, 70, -1), Some(dirt));
    assert_eq!(w.block_state(-2, 70, -1), stone);
    // Unknown / out-of-range values become air.
    set(&mut w, 0, 70, 0, 1 << 4 | 9);
    assert_eq!(w.block_state(0, 70, 0), Some(BlockState::AIR));
    set(&mut w, 1, 70, 0, -5);
    assert_eq!(w.block_state(1, 70, 0), Some(BlockState::AIR));
    set(&mut w, 2, 70, 0, 1 << 20);
    assert_eq!(w.block_state(2, 70, 0), Some(BlockState::AIR));
    // A new section appears for a block in an empty section; air in an empty section stays.
    set(&mut w, 3, 200, 3, i32::from(dirt.raw()));
    assert_eq!(w.block_state(3, 200, 3), Some(dirt));
    // Unloaded column / outside height: ignored.
    assert_eq!(set(&mut w, 100, 70, 0, 16), Applied::Ignored);
    assert_eq!(w.block_state(100, 70, 0), None);
    assert_eq!(set(&mut w, 0, 256, 0, 16), Applied::Ignored);
    assert_eq!(set(&mut w, 0, -1, 0, 16), Applied::Ignored);
    assert_eq!(w.block_state(0, 256, 0), Some(BlockState::AIR));

    let records = vec![
        MultiBlockChangeRecords {
            horizontal_pos: 0x3A, // x 3, z 10
            y: 65,
            block_id: i32::from(dirt.raw()),
        },
        MultiBlockChangeRecords {
            horizontal_pos: 0xFF,
            y: 66,
            block_id: 0,
        },
    ];
    let p = MultiBlockChange {
        chunk_x: -1,
        chunk_z: 0,
        records,
    };
    assert_eq!(w.apply_multi_block_change(&p), Applied::Changed);
    assert_eq!(w.block_state(-13, 65, 10), Some(dirt));
    assert_eq!(w.block_state(-1, 66, 15), Some(BlockState::AIR));
    let p = MultiBlockChange {
        chunk_x: 9,
        chunk_z: 9,
        records: vec![],
    };
    assert_eq!(w.apply_multi_block_change(&p), Applied::Ignored);
}

#[test]
fn explosion_truncates_towards_zero() {
    let mut w = loaded_world();
    // x = -0.5: vanilla 1.8 `(int) x` = 0 (floor would be -1).
    let p = Explosion {
        x: -0.5,
        y: 70.9,
        z: -0.5,
        radius: 3.0,
        affected_block_offsets: vec![
            ExplosionAffectedBlockOffsets { x: 0, y: 0, z: 0 },
            ExplosionAffectedBlockOffsets {
                x: -1,
                y: -1,
                z: -1,
            },
            ExplosionAffectedBlockOffsets { x: 100, y: 0, z: 0 }, // unloaded: ignored
        ],
        player_motion_x: 0.0,
        player_motion_y: 0.0,
        player_motion_z: 0.0,
    };
    assert_eq!(w.apply_explosion(&p), Applied::Changed);
    assert_eq!(w.block_state(0, 70, 0), Some(BlockState::AIR));
    assert_eq!(w.block_state(-1, 69, -1), Some(BlockState::AIR));
    assert_eq!(
        w.block_state(-1, 70, -1),
        Some(Block::STONE.default_state())
    );
    assert_eq!(w.block_state(100, 70, 0), None);
}

fn login(dimension: i8) -> Login {
    Login {
        entity_id: 1,
        game_mode: 0,
        dimension,
        difficulty: 0,
        max_players: 20,
        level_type: "default".into(),
        reduced_debug_info: false,
    }
}

fn respawn(dimension: i32) -> Respawn {
    Respawn {
        dimension,
        difficulty: 0,
        gamemode: 0,
        level_type: "default".into(),
    }
}

#[test]
fn login_respawn() {
    let mut w = loaded_world();
    assert_eq!(w.apply_respawn(&respawn(0)), Applied::Ignored);
    assert!(
        w.is_column_loaded(0, 0),
        "same-dimension respawn keeps the world (vanilla 1.8)"
    );
    assert_eq!(w.apply_respawn(&respawn(-1)), Applied::Changed);
    assert!(!w.is_column_loaded(0, 0));
    assert_eq!(w.dimension(), -1);
    let mut w = loaded_world();
    w.apply_login(&login(0));
    assert!(!w.is_column_loaded(0, 0));
}

#[test]
fn handle_dispatch() {
    let mut w = World::new(WorldConfig::default());
    let p = ClientboundPlayPacket::MapChunk(map_chunk(0, 0, true, 0, vec![]));
    assert_eq!(w.handle(&p), Ok(Applied::Ignored));
    let p = ClientboundPlayPacket::KeepAlive(ghast_protocol::v1_8::play::clientbound::KeepAlive {
        keep_alive_id: 1,
    });
    assert_eq!(w.handle(&p), Ok(Applied::NotWorldPacket));
}

#[test]
fn set_block_state_prediction() {
    let mut w = loaded_world();
    assert!(w.set_block_state(5, 10, 5, Block::DIRT.default_state()));
    assert_eq!(w.block_state(5, 10, 5), Some(Block::DIRT.default_state()));
    assert!(!w.set_block_state(500, 10, 5, Block::DIRT.default_state()));
    assert_eq!(w.block_state(500, 10, 5), None);
}

#[cfg(feature = "physics")]
#[test]
fn physics_view_unloaded_is_air() {
    use ghast_physics::BlockView;
    let w = loaded_world();
    assert_eq!(w.block(0, 70, 0), Block::STONE.default_state());
    assert_eq!(w.block(1000, 70, 0), BlockState::AIR);
    assert!(w.is_chunk_loaded(-1, -1));
    assert!(!w.is_chunk_loaded(1, 1));
}

/// Captured clean chunk packets (from the lab) go into `tests/captured/1.8.9/*.bin`: each file
/// is one uncompressed packet (VarInt id + body). Every file must apply without error.
#[test]
fn captured_packets() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/captured/1.8.9");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut w = World::new(WorldConfig::default());
    for e in entries.flatten() {
        let bytes = std::fs::read(e.path()).unwrap();
        let (id, n) = ghast_protocol::codec::read_varint_prefix(&bytes)
            .unwrap()
            .unwrap();
        let raw = ghast_protocol::RawPacket::new(id, Bytes::copy_from_slice(&bytes[n..]));
        use ghast_protocol::PacketSet;
        let p = ClientboundPlayPacket::decode_raw(&raw).unwrap();
        w.handle(&p)
            .unwrap_or_else(|err| panic!("{}: {err}", e.path().display()));
    }
}

#[test]
fn world_border_moves_with_time() {
    use ghast_protocol::v1_8::play::clientbound::WorldBorder;
    let mut w = loaded_world();
    let p = ClientboundPlayPacket::WorldBorder(WorldBorder {
        action: 3,
        radius: None,
        x: Some(100.0),
        z: Some(-50.0),
        old_radius: Some(1000.0),
        new_radius: Some(500.0),
        speed: Some(10_000),
        portal_boundary: Some(29_999_984),
        warning_time: Some(15),
        warning_blocks: Some(5),
    });
    assert_eq!(w.handle(&p), Ok(Applied::Changed));
    assert_eq!(w.border().size(), 1000.0);
    for _ in 0..100 {
        w.advance_time(50);
    }
    assert_eq!(w.border().size(), 750.0);
    w.advance_time(5000);
    assert_eq!(w.border().size(), 500.0);
    #[cfg(feature = "physics")]
    {
        use ghast_physics::BlockView;
        let b = w.world_border();
        assert_eq!((b.center_x, b.center_z, b.size), (100.0, -50.0, 500.0));
    }
    // A dimension change resets the border.
    w.apply_respawn(&respawn(1));
    assert_eq!(w.border().size(), 6.0e7);
}
