//! 1.16.5 world: paletted section decoding (every bits-per-block / palette type), known-answer
//! compact-array indexing, and packet semantics.
#![cfg(feature = "v1_16_5")]

mod common;

use bytes::Bytes;
use common::{Palette16, Rng, encode_section_1_16, put_varint, put_words};
use ghast_data::v1_16_5::{Block, BlockState, STATE_COUNT};
use ghast_protocol::BlockPos;
use ghast_protocol::nbt::RawNbt;
use ghast_protocol::v1_16_5::play::clientbound::{
    BlockChange, ClientboundPlayPacket, Explosion, ExplosionAffectedBlockOffsets, Login, MapChunk,
    MultiBlockChange, MultiBlockChangeChunkCoordinates, Respawn, UnloadChunk, UpdateViewDistance,
    UpdateViewPosition,
};
use ghast_world::v1_16_5::{GLOBAL_BITS, World};
use ghast_world::{Applied, Biomes, WorldConfig, WorldError};

const STONE: u16 = 1;

fn st(id: u16) -> Option<BlockState> {
    BlockState::from_id(id)
}

fn pos(cx: i32, cz: i32, j: usize, i: usize) -> (i32, i32, i32) {
    (
        cx * 16 + (i & 15) as i32,
        (j * 16 + (i >> 8)) as i32,
        cz * 16 + ((i >> 4) & 15) as i32,
    )
}

/// Random ids drawn from a pool of `distinct` states (excluding air sometimes).
fn ids(rng: &mut Rng, distinct: usize) -> Vec<u16> {
    let pool: Vec<u16> = (0..distinct)
        .map(|_| rng.below(STATE_COUNT as u64) as u16)
        .collect();
    (0..4096)
        .map(|_| pool[rng.below(distinct as u64) as usize])
        .collect()
}

fn check(w: &World, cx: i32, cz: i32, j: usize, expect: &[u16]) {
    for (i, &e) in expect.iter().enumerate() {
        let (x, y, z) = pos(cx, cz, j, i);
        assert_eq!(w.block_state(x, y, z), st(e), "{x} {y} {z}");
    }
}

fn full_chunk(
    w: &mut World,
    cx: i32,
    cz: i32,
    mask: i32,
    data: &[u8],
) -> Result<Applied, WorldError> {
    w.apply_chunk_data(cx, cz, true, mask, Some(&[0; 1024]), data)
}

#[test]
fn every_indirect_bits_value() {
    let mut rng = Rng(42);
    let mut w = World::new(WorldConfig::default());
    // Header values 0..=8 (0..=4 all mean 4-bit array palette).
    for header in 0u8..=8 {
        let bits = u32::from(header.max(4));
        let distinct = 1usize << bits;
        let v = ids(&mut rng, distinct.min(1 << bits));
        let mut data = Vec::new();
        encode_section_1_16(&mut data, &v, Palette16::Indirect(header), &[]);
        full_chunk(&mut w, 0, 0, 1 << 5, &data).unwrap();
        check(&w, 0, 0, 5, &v);
    }
}

#[test]
fn global_palette_ignores_header_bits() {
    let mut rng = Rng(43);
    let mut w = World::new(WorldConfig::default());
    assert_eq!(GLOBAL_BITS, 15);
    for header in [9u8, 10, 11, 12, 13, 14, 15, 16, 64, 127] {
        let v = ids(&mut rng, 600);
        let mut data = Vec::new();
        encode_section_1_16(
            &mut data,
            &v,
            Palette16::Direct {
                header,
                bits: GLOBAL_BITS,
            },
            &[],
        );
        full_chunk(&mut w, 1, 1, 1, &data).unwrap();
        check(&w, 1, 1, 0, &v);
    }
    // Data packed with the header's own bit count (not 15) has the wrong length: rejected
    // (13 and 14 bits pack 4 values per word like 15, so their length matches).
    for header in 9u8..=12 {
        let v = ids(&mut rng, 600);
        let mut data = Vec::new();
        encode_section_1_16(
            &mut data,
            &v,
            Palette16::Direct {
                header,
                bits: u32::from(header),
            },
            &[],
        );
        let r = full_chunk(&mut w, 1, 1, 1, &data);
        assert!(
            matches!(r, Err(WorldError::Section { .. })),
            "header {header}: {r:?}"
        );
        assert!(!w.is_column_loaded(1, 1));
    }
    // Negative header byte: array palette (vanilla reads a signed byte).
    let v = ids(&mut rng, 3);
    let mut data = Vec::new();
    encode_section_1_16(&mut data, &v, Palette16::Indirect(4), &[]);
    data[2] = 0xF0;
    full_chunk(&mut w, 1, 1, 1, &data).unwrap();
    check(&w, 1, 1, 0, &v);
}

/// Hand-built words: checks which block each bit field lands in, for bits 4..=8 and 15.
#[test]
fn known_answer_indexing() {
    for bits in [4u32, 5, 6, 7, 8, 15] {
        let per = (64 / bits) as usize;
        let words = 4096usize.div_ceil(per);
        // Target: the last value slot of word 3 and the first slot of word `words - 1`.
        let a = 3 * per + per - 1;
        let b = (words - 1) * per;
        let mut raw = vec![0u64; words];
        raw[3] = 1u64 << ((per as u32 - 1) * bits);
        raw[words - 1] = 2;
        // Garbage in padding bits must be ignored.
        let pad = 64 - per as u32 * bits;
        if pad > 0 {
            raw[0] |= !0u64 << (64 - pad);
        }
        let mut data = Vec::new();
        data.extend_from_slice(&4096i16.to_be_bytes());
        if bits <= 8 {
            data.push(bits as u8);
            put_varint(&mut data, 3);
            put_varint(&mut data, 0); // index 0: air
            put_varint(&mut data, i32::from(STONE)); // index 1: stone
            put_varint(&mut data, 9); // index 2: granite... any state 9
        } else {
            data.push(15);
        }
        put_varint(&mut data, words as i32);
        for w in &raw {
            data.extend_from_slice(&w.to_be_bytes());
        }
        let mut w = World::new(WorldConfig::default());
        full_chunk(&mut w, 0, 0, 1, &data).unwrap();
        let (one, two) = if bits <= 8 { (STONE, 9) } else { (1, 2) };
        for i in 0..4096 {
            let (x, y, z) = pos(0, 0, 0, i);
            let expect = if i == a {
                one
            } else if i == b {
                two
            } else {
                0
            };
            assert_eq!(w.block_state(x, y, z), st(expect), "bits {bits} index {i}");
        }
    }
}

#[test]
fn palette_edge_cases() {
    let mut w = World::new(WorldConfig::default());
    // Indices beyond the palette and palette ids that are not states read as air.
    let mut data = Vec::new();
    data.extend_from_slice(&1i16.to_be_bytes());
    data.push(5);
    put_varint(&mut data, 2);
    put_varint(&mut data, i32::from(STONE));
    put_varint(&mut data, STATE_COUNT as i32 + 5); // not a state
    let mut vals = vec![7u32; 4096]; // index 7: beyond the palette
    vals[0] = 0;
    vals[1] = 1;
    put_words(&mut data, 5, &vals);
    full_chunk(&mut w, 0, 0, 1, &data).unwrap();
    assert_eq!(w.block_state(0, 0, 0), st(STONE));
    assert_eq!(w.block_state(1, 0, 0), st(0));
    assert_eq!(w.block_state(2, 0, 0), st(0));

    // A 4-bit array palette longer than 16 entries: vanilla fails, we reject and unload.
    let v: Vec<u16> = (0..4096).map(|i| (i % 16) as u16 + 1).collect();
    let mut data = Vec::new();
    encode_section_1_16(&mut data, &v, Palette16::Indirect(4), &[1]);
    let r = full_chunk(&mut w, 0, 0, 1, &data);
    assert!(matches!(r, Err(WorldError::Section { .. })), "{r:?}");
    assert!(!w.is_column_loaded(0, 0));

    // A hash palette may be longer than 2^bits (only reachable entries matter).
    let extra: Vec<i32> = (0..300).collect();
    let v: Vec<u16> = (0..4096).map(|i| (i % 32) as u16 + 100).collect();
    let mut data = Vec::new();
    encode_section_1_16(&mut data, &v, Palette16::Indirect(5), &extra);
    full_chunk(&mut w, 0, 0, 1, &data).unwrap();
    check(&w, 0, 0, 0, &v);

    // Trailing bytes after the last section are ignored (vanilla reads only what it needs).
    data.extend_from_slice(&[1, 2, 3]);
    full_chunk(&mut w, 0, 0, 1, &data).unwrap();
    check(&w, 0, 0, 0, &v);
}

#[test]
fn full_partial_unload_and_biomes() {
    let mut rng = Rng(7);
    let mut w = World::new(WorldConfig::default());
    let secs: Vec<Option<Vec<u16>>> = (0..16)
        .map(|j| (j % 3 != 0).then(|| ids(&mut rng, 1 + j * 7)))
        .collect();
    let mut data = Vec::new();
    let mut mask = 0;
    for (j, s) in secs.iter().enumerate() {
        if let Some(v) = s {
            mask |= 1 << j;
            let distinct = 1 + j * 7;
            let mode = if distinct > 256 {
                Palette16::Direct {
                    header: 15,
                    bits: 15,
                }
            } else {
                Palette16::Indirect((usize::BITS - distinct.leading_zeros()).max(4) as u8)
            };
            encode_section_1_16(&mut data, v, mode, &[]);
        }
    }
    let biomes: Vec<i32> = (0..1024).map(|i| i % 50).collect();
    // Mask bits above 15 are ignored.
    let r = w.apply_chunk_data(-3, 4, true, mask | 1 << 20, Some(&biomes), &data);
    assert_eq!(r, Ok(Applied::Changed));
    for (j, s) in secs.iter().enumerate() {
        let empty = vec![0u16; 4096];
        check(&w, -3, 4, j, s.as_ref().unwrap_or(&empty));
    }
    assert!(matches!(w.column(-3, 4).unwrap().biomes(), Biomes::Cells(b) if b[49] == 49));
    // Outside the build height: void_air (vanilla 1.16), but only in loaded columns.
    assert_eq!(
        w.block_state(-48, 256, 64),
        Some(Block::VOID_AIR.default_state())
    );
    assert_eq!(
        w.block_state(-48, -1, 64),
        Some(Block::VOID_AIR.default_state())
    );
    assert_eq!(w.block_state(1000, -1, 64), None);

    // Partial: only section 1 replaced.
    let new1 = vec![STONE; 4096];
    let mut d = Vec::new();
    encode_section_1_16(&mut d, &new1, Palette16::Indirect(4), &[]);
    assert_eq!(
        w.apply_chunk_data(-3, 4, false, 1 << 1, None, &d),
        Ok(Applied::Changed)
    );
    check(&w, -3, 4, 1, &new1);
    check(&w, -3, 4, 2, secs[2].as_ref().unwrap());
    // Partial for an unloaded column: ignored.
    assert_eq!(
        w.apply_chunk_data(50, 50, false, 2, None, &d),
        Ok(Applied::Ignored)
    );
    assert!(!w.is_column_loaded(50, 50));

    // Full with an empty mask: an empty loaded column (1.16 unloads only via unload_chunk).
    assert_eq!(
        w.apply_chunk_data(-3, 4, true, 0, None, &[]),
        Ok(Applied::Changed)
    );
    assert_eq!(w.block_state(-48, 20, 64), st(0));
    assert_eq!(
        w.apply_unload_chunk(&UnloadChunk {
            chunk_x: -3,
            chunk_z: 4
        }),
        Applied::Changed
    );
    assert_eq!(w.block_state(-48, 20, 64), None);
}

fn stone_world(chunks: &[(i32, i32)]) -> World {
    let mut w = World::new(WorldConfig::default());
    for &(cx, cz) in chunks {
        let mut d = Vec::new();
        encode_section_1_16(&mut d, &[STONE; 4096], Palette16::Indirect(4), &[]);
        full_chunk(&mut w, cx, cz, 1 << 4, &d).unwrap();
    }
    w
}

#[test]
fn block_changes() {
    let mut w = stone_world(&[(0, 0), (-1, -1)]);
    let dirt = Block::DIRT.default_state();
    let r = w.apply_block_change(&BlockChange {
        location: BlockPos {
            x: -1,
            y: 70,
            z: -1,
        },
        r#type: i32::from(dirt.id()),
    });
    assert_eq!(r, Applied::Changed);
    assert_eq!(w.block_state(-1, 70, -1), Some(dirt));
    w.apply_block_change(&BlockChange {
        location: BlockPos { x: 0, y: 70, z: 0 },
        r#type: STATE_COUNT as i32,
    });
    assert_eq!(w.block_state(0, 70, 0), st(0));
    let r = w.apply_block_change(&BlockChange {
        location: BlockPos { x: 99, y: 70, z: 0 },
        r#type: 1,
    });
    assert_eq!(r, Applied::Ignored);

    // Section-position multi block change: chunk (-1, -1), section 4.
    let rec = |state: i64, x: i64, y: i64, z: i64| state << 12 | x << 8 | z << 4 | y;
    let p = MultiBlockChange {
        chunk_coordinates: MultiBlockChangeChunkCoordinates { x: -1, z: -1, y: 4 },
        not_trust_edges: false,
        records: vec![rec(i64::from(dirt.id()), 15, 0, 15), rec(0, 0, 15, 0)],
    };
    assert_eq!(w.apply_multi_block_change(&p), Applied::Changed);
    assert_eq!(w.block_state(-1, 64, -1), Some(dirt));
    assert_eq!(w.block_state(-16, 79, -16), st(0));
    assert_eq!(w.block_state(-16, 78, -16), st(STONE));
    // Negative section Y (20-bit signed): below the world, ignored.
    let p = MultiBlockChange {
        chunk_coordinates: MultiBlockChangeChunkCoordinates {
            x: 0,
            z: 0,
            y: 0xFFFFF,
        },
        not_trust_edges: false,
        records: vec![rec(1, 0, 15, 0)],
    };
    w.apply_multi_block_change(&p);
    assert_eq!(w.block_state(0, 15, 0), st(0));
}

#[test]
fn explosion_floors() {
    let mut w = stone_world(&[(0, 0), (-1, -1), (-1, 0), (0, -1)]);
    let p = Explosion {
        x: -0.5,
        y: 70.5,
        z: -0.5,
        radius: 2.0,
        affected_block_offsets: vec![ExplosionAffectedBlockOffsets { x: 0, y: 0, z: 0 }],
        player_motion_x: 0.0,
        player_motion_y: 0.0,
        player_motion_z: 0.0,
    };
    assert_eq!(w.apply_explosion(&p), Applied::Changed);
    assert_eq!(w.block_state(-1, 70, -1), st(0), "floor(-0.5) = -1");
    assert_eq!(w.block_state(0, 70, 0), st(STONE));
}

fn login(view_distance: i32) -> Login {
    Login {
        entity_id: 1,
        is_hardcore: false,
        game_mode: 0,
        previous_game_mode: 0,
        world_names: vec!["minecraft:overworld".into()],
        dimension_codec: RawNbt::END,
        dimension: RawNbt::END,
        world_name: "minecraft:overworld".into(),
        hashed_seed: 0,
        max_players: 20,
        view_distance,
        reduced_debug_info: false,
        enable_respawn_screen: true,
        is_debug: false,
        is_flat: false,
    }
}

fn respawn(name: &'static str) -> Respawn {
    Respawn {
        dimension: RawNbt::END,
        world_name: name.into(),
        hashed_seed: 0,
        gamemode: 0,
        previous_gamemode: 0,
        is_debug: false,
        is_flat: false,
        copy_metadata: false,
    }
}

#[test]
fn view_window() {
    let mut w = World::new(WorldConfig::default());
    w.apply_login(&login(2)); // radius max(2, 2) + 3 = 5
    let mut d = Vec::new();
    encode_section_1_16(&mut d, &[STONE; 4096], Palette16::Indirect(4), &[]);
    // No centre yet: everything accepted.
    assert_eq!(full_chunk(&mut w, 100, 100, 1, &d), Ok(Applied::Changed));
    w.apply_view_position(&UpdateViewPosition {
        chunk_x: 0,
        chunk_z: 0,
    });
    assert!(!w.is_column_loaded(100, 100), "outside the window: dropped");
    assert_eq!(full_chunk(&mut w, 5, -5, 1, &d), Ok(Applied::Changed));
    assert_eq!(full_chunk(&mut w, 6, 0, 1, &d), Ok(Applied::Ignored));
    w.apply_view_position(&UpdateViewPosition {
        chunk_x: 1,
        chunk_z: 0,
    });
    assert!(w.is_column_loaded(5, -5));
    w.apply_view_distance(&UpdateViewDistance { view_distance: 1 }); // radius stays 5
    assert!(w.is_column_loaded(5, -5));
    w.apply_view_position(&UpdateViewPosition {
        chunk_x: -1,
        chunk_z: 0,
    });
    assert!(!w.is_column_loaded(5, -5));
}

#[test]
fn login_respawn() {
    let mut w = stone_world(&[(0, 0)]);
    w.apply_login(&login(10));
    assert!(!w.is_column_loaded(0, 0));
    let mut d = Vec::new();
    encode_section_1_16(&mut d, &[STONE; 4096], Palette16::Indirect(4), &[]);
    full_chunk(&mut w, 0, 0, 1, &d).unwrap();
    assert_eq!(
        w.apply_respawn(&respawn("minecraft:overworld")),
        Applied::Ignored
    );
    assert!(w.is_column_loaded(0, 0));
    assert_eq!(
        w.apply_respawn(&respawn("minecraft:the_nether")),
        Applied::Changed
    );
    assert!(!w.is_column_loaded(0, 0));
    assert_eq!(w.world_name(), "minecraft:the_nether");
}

#[test]
fn handle_map_chunk_packet() {
    let mut w = World::new(WorldConfig::default());
    let mut d = Vec::new();
    encode_section_1_16(&mut d, &[STONE; 4096], Palette16::Indirect(4), &[]);
    let p = ClientboundPlayPacket::MapChunk(MapChunk {
        x: 2,
        z: 3,
        ground_up: true,
        bit_map: 1,
        heightmaps: RawNbt::END,
        biomes: Some(vec![1; 1024]),
        chunk_data: Bytes::from(d),
        block_entities: vec![],
    });
    assert_eq!(w.handle(&p), Ok(Applied::Changed));
    assert_eq!(w.block_state(32, 0, 48), st(STONE));
}

#[cfg(feature = "physics")]
#[test]
fn physics_view() {
    use ghast_physics::BlockView;
    let w = stone_world(&[(0, 0)]);
    assert_eq!(w.block(0, 64, 0), BlockState::from_id(STONE).unwrap());
    assert_eq!(w.block(1000, 64, 0), Block::VOID_AIR.default_state());
    assert!(w.is_chunk_loaded(0, 0) && !w.is_chunk_loaded(0, 1));
}

/// Captured clean chunk packets go into `tests/captured/1.16.5/*.bin` (VarInt id + body).
#[test]
fn captured_packets() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/captured/1.16.5");
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
fn block_entities_from_chunks_and_updates() {
    use ghast_protocol::v1_16_5::play::clientbound::{ClientboundPlayPacket as C, TileEntityData};
    use ghast_world::BlockEntity;
    let mut w = World::new(WorldConfig::default().with_block_entities(true));
    let mut d = Vec::new();
    encode_section_1_16(&mut d, &[STONE; 4096], Palette16::Indirect(4), &[]);
    let nbt = |x, y, z| {
        RawNbt::from_bytes(Bytes::from(common::be_nbt(x, y, z, "minecraft:chest"))).unwrap()
    };
    let chunk = |full: bool, list: Vec<RawNbt>| {
        C::MapChunk(MapChunk {
            x: 1,
            z: 0,
            ground_up: full,
            bit_map: 1 << 4,
            heightmaps: RawNbt::END,
            biomes: full.then(|| vec![1; 1024]),
            chunk_data: Bytes::from(d.clone()),
            block_entities: list,
        })
    };
    // Entries in two sections plus one without coordinates (skipped).
    let bad = RawNbt::from_bytes(Bytes::from_static(&[10, 0, 0, 0])).unwrap();
    w.handle(&chunk(true, vec![nbt(17, 70, 2), nbt(20, 10, 3), bad]))
        .unwrap();
    let col = w.column(1, 0).unwrap();
    assert_eq!(col.block_entities().count(), 2);
    assert!(matches!(
        w.block_entity(17, 70, 2),
        Some(BlockEntity::Nbt { action: 0, .. })
    ));
    // A partial chunk replaces the block entities of its sections only.
    w.handle(&chunk(false, vec![])).unwrap();
    assert!(w.block_entity(17, 70, 2).is_none(), "section 4 replaced");
    assert!(w.block_entity(20, 10, 3).is_some(), "section 0 untouched");
    // tile_entity_data, then a block change to another block removes it.
    let p = TileEntityData {
        location: BlockPos { x: 18, y: 64, z: 1 },
        action: 9,
        nbt_data: Some(nbt(18, 64, 1)),
    };
    assert_eq!(w.handle(&C::TileEntityData(p)), Ok(Applied::Changed));
    assert!(matches!(
        w.block_entity(18, 64, 1),
        Some(BlockEntity::Nbt { action: 9, .. })
    ));
    // Same block (stone -> stone): kept; different block: removed.
    w.apply_block_change(&BlockChange {
        location: BlockPos { x: 18, y: 64, z: 1 },
        r#type: i32::from(STONE),
    });
    assert!(w.block_entity(18, 64, 1).is_some());
    w.apply_block_change(&BlockChange {
        location: BlockPos { x: 18, y: 64, z: 1 },
        r#type: 0,
    });
    assert!(w.block_entity(18, 64, 1).is_none());
    // Off: not in the packet ids, nothing stored.
    let mut off = World::new(WorldConfig::default());
    assert!(!off.packet_ids().any(|id| id == 0x09));
    off.handle(&chunk(true, vec![nbt(17, 70, 2)])).unwrap();
    assert_eq!(off.column(1, 0).unwrap().block_entities().count(), 0);
}

#[test]
fn full_chunk_without_section_data_is_an_empty_column() {
    // A full chunk with a non-empty mask but no section data: an empty loaded column, no error.
    let mut w = stone_world(&[(0, 0)]);
    assert_eq!(w.block_state(1, 70, 1), st(STONE));
    let biomes: Vec<i32> = vec![3; 1024];
    assert_eq!(
        w.apply_chunk_data(0, 0, true, 1 << 4, Some(&biomes), &[]),
        Ok(Applied::Changed)
    );
    assert!(w.is_column_loaded(0, 0));
    assert_eq!(w.block_state(1, 70, 1), st(0));
    assert!(matches!(w.column(0, 0).unwrap().biomes(), Biomes::Cells(b) if b[0] == 3));
    // A partial chunk without data for its mask is still malformed.
    assert!(w.apply_chunk_data(0, 0, false, 1 << 4, None, &[]).is_err());
}
