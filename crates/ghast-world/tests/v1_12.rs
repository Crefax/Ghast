//! 1.12.2 world: paletted section decoding (linear, hash and global palettes, values spanning
//! longs), light, biomes, partial chunks, unloads, block changes.
#![cfg(feature = "v1_12")]

use bytes::{BufMut, Bytes, BytesMut};
use ghast_data::v1_12::{Block, BlockState};
use ghast_protocol::BlockPos;
use ghast_protocol::v1_12_2::play::clientbound::{
    BlockChange, MapChunk, MultiBlockChange, MultiBlockChangeRecords, UnloadChunk,
};
use ghast_world::packed::pack_spanning;
use ghast_world::v1_12::{GLOBAL_BITS, World};
use ghast_world::{Applied, WorldConfig};

fn varint(d: &mut BytesMut, mut v: u32) {
    loop {
        if v < 0x80 {
            d.put_u8(v as u8);
            return;
        }
        d.put_u8((v as u8 & 0x7F) | 0x80);
        v >>= 7;
    }
}

/// One section: `bits` sent, `palette` (None = global), the 4096 values (palette indices or
/// raw states), light, sky light.
fn section(d: &mut BytesMut, bits: u8, palette: Option<&[u16]>, values: &[u32], used_bits: u32) {
    d.put_u8(bits);
    match palette {
        Some(p) => {
            varint(d, p.len() as u32);
            for &s in p {
                varint(d, u32::from(s));
            }
        }
        None => varint(d, 0),
    }
    let words = pack_spanning(used_bits, values);
    varint(d, words.len() as u32);
    for w in words {
        d.put_u64(w);
    }
    d.put_bytes(0x12, 2048);
    d.put_bytes(0xff, 2048);
}

fn chunk(x: i32, z: i32, full: bool, mask: i32, data: Bytes) -> MapChunk {
    MapChunk {
        x,
        z,
        ground_up: full,
        bit_map: mask,
        chunk_data: data,
        block_entities: Vec::new(),
    }
}

fn idx(x: i32, y: i32, z: i32) -> usize {
    ((y as usize & 15) << 8) | ((z as usize) << 4) | x as usize
}

const STONE: u16 = 1 << 4;
const GRANITE: u16 = 1 << 4 | 1;
const DIRT: u16 = 3 << 4;

#[test]
fn linear_hash_and_global_palettes() {
    let mut d = BytesMut::new();
    // Section 0: linear palette (bits 2 sent -> 4), stone floor at y 0, an index past the
    // palette at (1, 1, 1) reads as air.
    let mut v = vec![0u32; 4096];
    for x in 0..16 {
        for z in 0..16 {
            v[idx(x, 0, z)] = 1;
        }
    }
    v[idx(1, 1, 1)] = 7;
    section(&mut d, 2, Some(&[0, STONE]), &v, 4);
    // Section 1: hash palette with 5 bits, granite and dirt, and an unregistered meta (stone:9)
    // that reads as air.
    let mut v = vec![0u32; 4096];
    v[idx(2, 3, 4)] = 1;
    v[idx(5, 6, 7)] = 2;
    v[idx(8, 8, 8)] = 3;
    section(&mut d, 5, Some(&[0, GRANITE, DIRT, 1 << 4 | 9]), &v, 5);
    // Section 2: global palette (sent 9, read with 13 bits), dirt at (15, 15, 15).
    let mut v = vec![0u32; 4096];
    v[idx(15, 15, 15)] = u32::from(DIRT);
    v[idx(0, 0, 0)] = u32::from(Block::OBSIDIAN.default_state().raw());
    section(&mut d, 9, None, &v, GLOBAL_BITS);
    d.put_bytes(1, 256);

    let mut w = World::new(WorldConfig::default().with_light(true));
    let r = w.apply_map_chunk(&chunk(-1, 2, true, 0b111, d.freeze()));
    assert_eq!(r, Ok(Applied::Changed));
    let at = |x, y, z| w.block_state(x, y, z).map(BlockState::raw);
    assert_eq!(at(-16, 0, 32), Some(STONE));
    assert_eq!(at(-1, 0, 47), Some(STONE));
    assert_eq!(at(-15, 1, 33), Some(0));
    assert_eq!(at(-14, 19, 36), Some(GRANITE));
    assert_eq!(at(-11, 22, 39), Some(DIRT));
    assert_eq!(at(-8, 24, 40), Some(0));
    assert_eq!(at(-1, 47, 47), Some(DIRT));
    assert_eq!(at(-16, 32, 32), Some(Block::OBSIDIAN.default_state().raw()));
    // Above the sent sections: air in a loaded column; another column: unknown.
    assert_eq!(at(-16, 100, 32), Some(0));
    assert_eq!(w.block_state(0, 0, 0), None);
    let col = w.column(-1, 2).unwrap();
    assert_eq!(col.block_light(0, 0, 0), Some(2));
    assert_eq!(col.sky_light(0, 0, 0), Some(15));
}

#[test]
fn partial_chunks_unloads_and_block_changes() {
    let mut w = World::new(WorldConfig::default());
    // A partial chunk for a column that is not loaded is ignored.
    let mut d = BytesMut::new();
    section(&mut d, 4, Some(&[STONE]), &[0; 4096], 4);
    let partial = d.freeze();
    assert_eq!(
        w.apply_map_chunk(&chunk(0, 0, false, 1, partial.clone())),
        Ok(Applied::Ignored)
    );
    // Full chunk without sections, then the partial one fills section 0.
    let mut d = BytesMut::new();
    d.put_bytes(0, 256);
    assert_eq!(
        w.apply_map_chunk(&chunk(0, 0, true, 0, d.freeze())),
        Ok(Applied::Changed)
    );
    assert_eq!(w.block_state(3, 3, 3), Some(BlockState::AIR));
    assert_eq!(
        w.apply_map_chunk(&chunk(0, 0, false, 1, partial)),
        Ok(Applied::Changed)
    );
    assert_eq!(w.block_state(3, 3, 3).map(BlockState::raw), Some(STONE));
    // Block changes: unknown states read as air; multi block change records.
    w.apply_block_change(&BlockChange {
        location: BlockPos::new(1, 2, 3),
        r#type: i32::from(DIRT),
    });
    assert_eq!(w.block_state(1, 2, 3).map(BlockState::raw), Some(DIRT));
    w.apply_block_change(&BlockChange {
        location: BlockPos::new(1, 2, 3),
        r#type: 253 << 4,
    });
    assert_eq!(w.block_state(1, 2, 3), Some(BlockState::AIR));
    w.apply_multi_block_change(&MultiBlockChange {
        chunk_x: 0,
        chunk_z: 0,
        records: vec![MultiBlockChangeRecords {
            horizontal_pos: 4 << 4 | 5,
            y: 70,
            block_id: i32::from(GRANITE),
        }],
    });
    assert_eq!(w.block_state(4, 70, 5).map(BlockState::raw), Some(GRANITE));
    assert_eq!(
        w.apply_unload_chunk(&UnloadChunk {
            chunk_x: 0,
            chunk_z: 0
        }),
        Applied::Changed
    );
    assert_eq!(w.block_state(4, 70, 5), None);
}

#[test]
fn malformed_sections_unload_the_column() {
    let mut w = World::new(WorldConfig::default());
    let mut d = BytesMut::new();
    d.put_bytes(0, 256);
    w.apply_map_chunk(&chunk(0, 0, true, 0, d.freeze()))
        .unwrap();
    // Data array of the wrong length for 4 bits.
    let mut d = BytesMut::new();
    d.put_u8(4);
    varint(&mut d, 1);
    varint(&mut d, u32::from(STONE));
    varint(&mut d, 3);
    d.put_bytes(0, 24);
    assert!(
        w.apply_map_chunk(&chunk(0, 0, true, 1, d.freeze()))
            .is_err()
    );
    assert!(!w.is_column_loaded(0, 0));
    // Truncated light.
    let mut d = BytesMut::new();
    section(&mut d, 4, Some(&[STONE]), &[0; 4096], 4);
    let short = d.freeze().slice(..2000);
    assert!(w.apply_map_chunk(&chunk(1, 0, true, 1, short)).is_err());
    assert!(!w.is_column_loaded(1, 0));
}
