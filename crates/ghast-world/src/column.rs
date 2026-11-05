//! Chunk columns and the column map.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use crate::section::{Section, block_index};

/// Number of sections in a column (both 1.8 and 1.16.5 have a fixed 0..256 build height).
pub const SECTIONS: usize = 16;

/// Lowest block Y (inclusive).
pub const MIN_Y: i32 = 0;
/// Build height: highest block Y + 1.
pub const MAX_Y: i32 = 256;

/// Position of a chunk column (block coordinates `>> 4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ChunkPos {
    /// Chunk X (`block_x >> 4`).
    pub x: i32,
    /// Chunk Z (`block_z >> 4`).
    pub z: i32,
}

impl ChunkPos {
    /// Creates a chunk position.
    #[must_use]
    pub const fn new(x: i32, z: i32) -> Self {
        ChunkPos { x, z }
    }

    /// The column containing block `(x, z)` (floor division by 16).
    #[must_use]
    pub const fn of_block(x: i32, z: i32) -> Self {
        ChunkPos {
            x: x >> 4,
            z: z >> 4,
        }
    }
}

/// Biome data of a column as the server sent it.
#[derive(Clone, Debug, Default)]
pub enum Biomes {
    /// Not received (partial chunk only so far) or not kept ([`crate::WorldConfig::biomes`]).
    #[default]
    None,
    /// 1.8: one biome id per (x, z), index `z << 4 | x`.
    Legacy(Box<[u8; 256]>),
    /// 1.16.5: one biome id per 4×4×4 cell, index `(y >> 2) << 4 | (z >> 2) << 2 | (x >> 2)`
    /// (vanilla `BiomeArray`). Ids that do not fit `u16` are stored as `u16::MAX`.
    Cells(Box<[u16; 1024]>),
}

/// 1.8 light of one section, as sent in the chunk packet (never recomputed locally).
#[derive(Clone, Debug)]
pub struct SectionLight {
    /// Block light nibbles, index [`block_index`] (low nibble = even index).
    pub block: Box<[u8; 2048]>,
    /// Sky light nibbles (`None` in dimensions without sky).
    pub sky: Option<Box<[u8; 2048]>>,
}

impl SectionLight {
    fn nibble(arr: &[u8; 2048], i: usize) -> u8 {
        let b = arr[(i >> 1) & 2047];
        if i & 1 == 0 { b & 15 } else { b >> 4 }
    }
}

/// A loaded chunk column: 16 sections (`None` = all air), biomes and optional light.
#[derive(Clone, Debug, Default)]
pub struct Column {
    pub(crate) sections: [Option<Section>; SECTIONS],
    pub(crate) biomes: Biomes,
    pub(crate) light: Option<Box<[Option<SectionLight>; SECTIONS]>>,
    /// Block entities by [`be_key`] (only with [`crate::WorldConfig::block_entities`]).
    pub(crate) block_entities: Vec<(u16, BlockEntity)>,
}

/// A block entity's data as the server sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockEntity {
    /// Raw NBT compound (a private copy, never a view into a packet buffer). `action` is the
    /// `tile_entity_data` action (1.8: 1 spawner, 2 command block, 3 beacon, 4 skull, 5 flower
    /// pot, 6 banner; 1.16 adds 7..15, 9 = sign), or 0 for the block entity list of a 1.16 chunk.
    Nbt {
        /// Update action.
        action: u8,
        /// The NBT bytes (`TAG_Compound` root).
        nbt: bytes::Bytes,
    },
    /// 1.8 `update_sign`: the four lines as raw chat JSON.
    Sign([Box<str>; 4]),
}

/// Stores a `tile_entity_data` / chunk block entity NBT payload (both versions): ignored for
/// unloaded columns, `y` outside `0..256` and the "no compound" value.
#[cfg_attr(not(any(feature = "v1_8", feature = "v1_16_5")), allow(dead_code))]
pub(crate) fn put_nbt(
    columns: &mut ColumnMap,
    l: ghast_protocol::BlockPos,
    action: u8,
    nbt: Option<&ghast_protocol::nbt::RawNbt>,
) -> crate::Applied {
    let Some(nbt) = nbt.filter(|n| !n.is_end()) else {
        return crate::Applied::Ignored;
    };
    if !(MIN_Y..MAX_Y).contains(&l.y) {
        return crate::Applied::Ignored;
    }
    let Some(col) = columns.get_mut(l.x >> 4, l.z >> 4) else {
        return crate::Applied::Ignored;
    };
    col.put_block_entity(
        l.x,
        l.y,
        l.z,
        BlockEntity::Nbt {
            action,
            nbt: bytes::Bytes::copy_from_slice(nbt.as_bytes()),
        },
    );
    crate::Applied::Changed
}

/// Column-local key of a block position: `y << 8 | z << 4 | x` (`y` in `0..256`).
#[inline]
pub(crate) fn be_key(x: i32, y: i32, z: i32) -> u16 {
    (((y & 0xFF) << 8) | ((z & 15) << 4) | (x & 15)) as u16
}

impl Column {
    /// An all-air column.
    #[must_use]
    pub fn empty() -> Self {
        Column::default()
    }

    /// Raw state at column-local `x`, `z` (only the low four bits are used) and world `y`.
    /// `y` outside `0..256` is air (0), as in vanilla.
    #[inline]
    #[must_use]
    pub fn raw(&self, x: i32, y: i32, z: i32) -> u16 {
        if !(MIN_Y..MAX_Y).contains(&y) {
            return 0;
        }
        match &self.sections[(y >> 4) as usize & (SECTIONS - 1)] {
            Some(s) => s.get(block_index(x, y, z)),
            None => 0,
        }
    }

    /// Sets the raw state at column-local `x`, `z` and world `y` (ignored outside `0..256`).
    pub fn set_raw(&mut self, x: i32, y: i32, z: i32, state: u16) {
        if !(MIN_Y..MAX_Y).contains(&y) {
            return;
        }
        let slot = &mut self.sections[(y >> 4) as usize & (SECTIONS - 1)];
        let i = block_index(x, y, z);
        match slot {
            Some(s) => {
                if !s.set(i, state) {
                    *slot = None;
                }
            }
            None => *slot = Section::single(i, state),
        }
    }

    /// Block entities as `([x, y, z] column-local, data)`, in arrival order. Empty unless
    /// [`crate::WorldConfig::block_entities`] is on.
    pub fn block_entities(&self) -> impl Iterator<Item = ([u8; 3], &BlockEntity)> {
        self.block_entities
            .iter()
            .map(|(k, b)| ([(k & 15) as u8, (k >> 8) as u8, ((k >> 4) & 15) as u8], b))
    }

    /// The block entity at column-local `x`, `z` (low four bits) and world `y`.
    #[must_use]
    pub fn block_entity(&self, x: i32, y: i32, z: i32) -> Option<&BlockEntity> {
        if !(MIN_Y..MAX_Y).contains(&y) {
            return None;
        }
        let k = be_key(x, y, z);
        self.block_entities
            .iter()
            .find(|(key, _)| *key == k)
            .map(|(_, b)| b)
    }

    #[cfg_attr(not(any(feature = "v1_8", feature = "v1_16_5")), allow(dead_code))]
    pub(crate) fn put_block_entity(&mut self, x: i32, y: i32, z: i32, be: BlockEntity) {
        if !(MIN_Y..MAX_Y).contains(&y) {
            return;
        }
        let k = be_key(x, y, z);
        match self.block_entities.iter_mut().find(|(key, _)| *key == k) {
            Some(slot) => slot.1 = be,
            None => self.block_entities.push((k, be)),
        }
    }

    #[cfg_attr(not(any(feature = "v1_8", feature = "v1_16_5")), allow(dead_code))]
    pub(crate) fn remove_block_entity(&mut self, x: i32, y: i32, z: i32) -> bool {
        if !(MIN_Y..MAX_Y).contains(&y) {
            return false;
        }
        let k = be_key(x, y, z);
        let before = self.block_entities.len();
        self.block_entities.retain(|(key, _)| *key != k);
        before != self.block_entities.len()
    }

    /// Drops the block entities in the sections of `mask` (bit = section index).
    #[cfg_attr(not(feature = "v1_16_5"), allow(dead_code))]
    pub(crate) fn remove_block_entities_in(&mut self, mask: u32) {
        self.block_entities
            .retain(|(k, _)| mask & (1 << (k >> 12)) == 0);
    }

    /// Sets a block like [`Column::set_raw`] and drops the block entity there when the block
    /// type changes (`same_block(old, new)` is the version's block-type comparison), as vanilla
    /// does.
    #[cfg_attr(not(any(feature = "v1_8", feature = "v1_16_5")), allow(dead_code))]
    pub(crate) fn set_raw_with(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        state: u16,
        same_block: fn(u16, u16) -> bool,
    ) {
        if !self.block_entities.is_empty() && !same_block(self.raw(x, y, z), state) {
            self.remove_block_entity(x, y, z);
        }
        self.set_raw(x, y, z, state);
    }

    /// The section at index `0..16` (`y >> 4`); `None` when it is all air.
    #[must_use]
    pub fn section(&self, index: usize) -> Option<&Section> {
        self.sections.get(index)?.as_ref()
    }

    /// Biomes as received.
    #[must_use]
    pub fn biomes(&self) -> &Biomes {
        &self.biomes
    }

    /// 1.8 block light at column-local `x`, `z` and world `y`, when light is kept and the section
    /// was sent. Light is never recomputed after block changes.
    #[must_use]
    pub fn block_light(&self, x: i32, y: i32, z: i32) -> Option<u8> {
        let l = self.light_section(y)?;
        Some(SectionLight::nibble(&l.block, block_index(x, y, z)))
    }

    /// 1.8 sky light (see [`Column::block_light`]); `None` also in dimensions without sky.
    #[must_use]
    pub fn sky_light(&self, x: i32, y: i32, z: i32) -> Option<u8> {
        let l = self.light_section(y)?;
        Some(SectionLight::nibble(
            l.sky.as_deref()?,
            block_index(x, y, z),
        ))
    }

    fn light_section(&self, y: i32) -> Option<&SectionLight> {
        if !(MIN_Y..MAX_Y).contains(&y) {
            return None;
        }
        self.light.as_ref()?[(y >> 4) as usize].as_ref()
    }

    /// Number of non-empty sections.
    #[must_use]
    pub fn section_count(&self) -> usize {
        self.sections.iter().filter(|s| s.is_some()).count()
    }

    /// Heap bytes owned by this column (sections, biomes, light), excluding the map entry.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        let sections: usize = self
            .sections
            .iter()
            .flatten()
            .map(Section::heap_bytes)
            .sum();
        let biomes = match self.biomes {
            Biomes::None => 0,
            Biomes::Legacy(_) => 256,
            Biomes::Cells(_) => 2048,
        };
        let light = self.light.as_ref().map_or(0, |l| {
            size_of::<[Option<SectionLight>; SECTIONS]>()
                + l.iter()
                    .flatten()
                    .map(|s| 2048 + s.sky.as_ref().map_or(0, |_| 2048))
                    .sum::<usize>()
        });
        let be = self.block_entities.capacity() * size_of::<(u16, BlockEntity)>()
            + self
                .block_entities
                .iter()
                .map(|(_, b)| match b {
                    BlockEntity::Nbt { nbt, .. } => nbt.len(),
                    BlockEntity::Sign(l) => l.iter().map(|s| s.len()).sum(),
                })
                .sum::<usize>();
        sections + biomes + light + be
    }
}

/// Hasher for packed chunk positions: one multiply plus a fold, so both halves reach the low
/// bits that pick the bucket.
#[derive(Default, Clone, Copy)]
pub(crate) struct ChunkHasher(u64);

impl Hasher for ChunkHasher {
    #[inline]
    fn finish(&self) -> u64 {
        let h = (self.0 ^ (self.0 >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        h ^ (h >> 32)
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 << 8) | u64::from(b);
        }
    }

    #[inline]
    fn write_u64(&mut self, v: u64) {
        self.0 = v;
    }
}

#[inline]
fn key(x: i32, z: i32) -> u64 {
    u64::from(x as u32) | (u64::from(z as u32) << 32)
}

/// The loaded columns. Version-independent; the version worlds wrap it.
#[derive(Clone, Debug, Default)]
pub struct ColumnMap {
    map: HashMap<u64, (ChunkPos, Column), BuildHasherDefault<ChunkHasher>>,
}

impl ColumnMap {
    /// The column at `(cx, cz)` if loaded.
    #[inline]
    #[must_use]
    pub fn get(&self, cx: i32, cz: i32) -> Option<&Column> {
        self.map.get(&key(cx, cz)).map(|(_, c)| c)
    }

    /// Mutable access to a loaded column.
    #[inline]
    pub fn get_mut(&mut self, cx: i32, cz: i32) -> Option<&mut Column> {
        self.map.get_mut(&key(cx, cz)).map(|(_, c)| c)
    }

    /// `true` when the column has been received and not unloaded.
    #[inline]
    #[must_use]
    pub fn contains(&self, cx: i32, cz: i32) -> bool {
        self.map.contains_key(&key(cx, cz))
    }

    /// Raw state at a block position; `None` when the column is not loaded.
    #[inline]
    #[must_use]
    pub fn raw(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        self.get(x >> 4, z >> 4).map(|c| c.raw(x, y, z))
    }

    /// Inserts (replaces) a column.
    pub fn insert(&mut self, cx: i32, cz: i32, column: Column) {
        self.map
            .insert(key(cx, cz), (ChunkPos::new(cx, cz), column));
    }

    /// Removes a column; `true` if it was loaded.
    pub fn remove(&mut self, cx: i32, cz: i32) -> bool {
        self.map.remove(&key(cx, cz)).is_some()
    }

    /// Unloads everything.
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Keeps only the columns for which `keep` returns `true`.
    pub fn retain(&mut self, mut keep: impl FnMut(ChunkPos) -> bool) {
        self.map.retain(|_, (p, _)| keep(*p));
    }

    /// Number of loaded columns.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// `true` when no column is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Iterates the loaded columns (arbitrary order).
    pub fn iter(&self) -> impl Iterator<Item = (ChunkPos, &Column)> {
        self.map.values().map(|(p, c)| (*p, c))
    }

    /// Approximate heap bytes: columns plus the hash table.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        let table = self.map.capacity() * (size_of::<(u64, (ChunkPos, Column))>() + 1);
        table
            + self
                .map
                .values()
                .map(|(_, c)| c.heap_bytes())
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_drop_sections() {
        let mut c = Column::empty();
        c.set_raw(3, 70, 5, 16);
        assert_eq!(c.raw(3, 70, 5), 16);
        assert_eq!(
            c.raw(3 + 16, 70, 5 - 32),
            16,
            "only low bits of x/z are used"
        );
        assert_eq!(c.section_count(), 1);
        c.set_raw(3, 70, 5, 0);
        assert_eq!(c.section_count(), 0);
        c.set_raw(0, -1, 0, 16);
        c.set_raw(0, 256, 0, 16);
        assert_eq!(c.section_count(), 0);
        assert_eq!(c.raw(0, -1, 0), 0);
    }

    #[test]
    fn map_negative_coords() {
        let mut m = ColumnMap::default();
        let mut c = Column::empty();
        c.set_raw(15, 0, 15, 7);
        m.insert(-1, -1, c);
        assert_eq!(m.raw(-1, 0, -1), Some(7));
        assert_eq!(m.raw(0, 0, 0), None);
        assert!(m.contains(-1, -1));
        assert!(!m.contains(-1, 0));
    }
}
