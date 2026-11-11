//! The planner's world access: raw block states from any source, cached per block as classes.

use std::collections::HashMap;

use crate::Version;
use crate::class::{Class, Table, table};

/// Read access to raw block states, as the version encodes them on the wire: 1.8
/// `id << 4 | meta`, 1.16.5 the global palette id.
///
/// `None` means **unknown** (column not loaded, or below the world): the planner never walks
/// through unknown blocks. Implemented for ghast-client's `WorldView`, the ghast-world worlds
/// (feature `client`) and the physics crate's `MemoryWorld`s.
pub trait BlockSource {
    /// The raw state at a position; `None` when unknown.
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32>;

    /// Whether the chunk column is loaded (default: probes one block of it).
    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.raw_block(chunk_x << 4, 0, chunk_z << 4).is_some()
    }
}

impl<T: BlockSource + ?Sized> BlockSource for &T {
    #[inline]
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        (**self).raw_block(x, y, z)
    }

    #[inline]
    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        (**self).is_column_loaded(chunk_x, chunk_z)
    }
}

#[cfg(feature = "v1_8")]
impl BlockSource for ghast_physics::MemoryWorld<ghast_data::v1_8::BlockState> {
    #[inline]
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        use ghast_physics::BlockView;
        if !(0..256).contains(&y) || !self.is_chunk_loaded(x >> 4, z >> 4) {
            return None;
        }
        Some(u32::from(self.block(x, y, z).raw()))
    }

    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        ghast_physics::BlockView::is_chunk_loaded(self, chunk_x, chunk_z)
    }
}

#[cfg(feature = "v1_12_2")]
impl BlockSource for ghast_physics::MemoryWorld<ghast_data::v1_12::BlockState> {
    #[inline]
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        use ghast_physics::BlockView;
        if !(0..256).contains(&y) || !self.is_chunk_loaded(x >> 4, z >> 4) {
            return None;
        }
        Some(u32::from(self.block(x, y, z).raw()))
    }

    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        ghast_physics::BlockView::is_chunk_loaded(self, chunk_x, chunk_z)
    }
}

#[cfg(feature = "v1_16_5")]
impl BlockSource for ghast_physics::MemoryWorld<ghast_data::v1_16_5::BlockState> {
    #[inline]
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        use ghast_physics::BlockView;
        if !(0..256).contains(&y) || !self.is_chunk_loaded(x >> 4, z >> 4) {
            return None;
        }
        Some(u32::from(self.block(x, y, z).id()))
    }

    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        ghast_physics::BlockView::is_chunk_loaded(self, chunk_x, chunk_z)
    }
}

#[cfg(feature = "client")]
mod client_impls {
    use super::BlockSource;

    impl BlockSource for dyn ghast_client::WorldView + '_ {
        #[inline]
        fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
            if !(0..256).contains(&y) {
                return None;
            }
            ghast_client::WorldView::raw_block(self, x, y, z)
        }

        fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
            ghast_client::WorldView::is_column_loaded(self, chunk_x, chunk_z)
        }
    }

    #[cfg(feature = "v1_8")]
    impl BlockSource for ghast_client::world::v1_8::World {
        #[inline]
        fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
            if !(0..256).contains(&y) {
                return None;
            }
            self.block_state(x, y, z).map(|s| u32::from(s.raw()))
        }

        fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
            ghast_client::world::v1_8::World::is_column_loaded(self, chunk_x, chunk_z)
        }
    }

    #[cfg(feature = "v1_12_2")]
    impl BlockSource for ghast_client::world::v1_12::World {
        #[inline]
        fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
            if !(0..256).contains(&y) {
                return None;
            }
            self.block_state(x, y, z).map(|s| u32::from(s.raw()))
        }

        fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
            ghast_client::world::v1_12::World::is_column_loaded(self, chunk_x, chunk_z)
        }
    }

    #[cfg(feature = "v1_16_5")]
    impl BlockSource for ghast_client::world::v1_16_5::World {
        #[inline]
        fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
            if !(0..256).contains(&y) {
                return None;
            }
            self.block_state(x, y, z).map(|s| u32::from(s.id()))
        }

        fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
            ghast_client::world::v1_16_5::World::is_column_loaded(self, chunk_x, chunk_z)
        }
    }
}

/// Cache sentinel: not looked up yet.
const NOT_CACHED: u16 = u16::MAX;
/// Cache sentinel: unknown (not loaded / outside the world).
const UNKNOWN: u16 = u16::MAX - 1;

/// One 16×16×16 section of cached class indices.
type Section = Box<[u16; 4096]>;

/// The class cache of a planning run, kept between the slices of an incremental search.
#[derive(Clone)]
pub(crate) struct Sections {
    index: HashMap<u64, u32, BuildKeyHasher>,
    secs: Vec<Section>,
    /// The last section looked up (most lookups hit the same section in a row).
    last: (u64, u32),
}

impl Default for Sections {
    fn default() -> Self {
        Sections {
            index: HashMap::with_capacity_and_hasher(64, BuildKeyHasher),
            secs: Vec::with_capacity(64),
            last: (u64::MAX, u32::MAX),
        }
    }
}

/// Per-planning-run cache of block classes (1 lookup in the source per block at most).
pub(crate) struct Grid<'w, S: ?Sized> {
    src: &'w S,
    table: &'static Table,
    sections: Sections,
}

impl<'w, S: BlockSource + ?Sized> Grid<'w, S> {
    pub(crate) fn new(src: &'w S, version: Version) -> Self {
        Self::with_cache(src, version, Sections::default())
    }

    /// A grid over `src` that continues an earlier run's cache.
    pub(crate) fn with_cache(src: &'w S, version: Version, sections: Sections) -> Self {
        Grid {
            src,
            table: table(version),
            sections,
        }
    }

    /// The cache, for the next slice.
    pub(crate) fn into_cache(self) -> Sections {
        self.sections
    }

    /// Air's class index (always 0).
    pub(crate) const AIR: u16 = 0;

    #[inline]
    fn key(x: i32, y: i32, z: i32) -> u64 {
        let sx = (x >> 4) as u32 as u64 & 0x3f_ffff;
        let sz = (z >> 4) as u32 as u64 & 0x3f_ffff;
        let sy = (y >> 4) as u32 as u64 & 0xfffff;
        sx << 42 | sz << 20 | sy
    }

    /// Class index at a position (`UNKNOWN` sentinel resolved by [`Grid::class`]).
    #[inline]
    pub(crate) fn index(&mut self, x: i32, y: i32, z: i32) -> u16 {
        if !(0..256).contains(&y) {
            return if y >= 256 { Self::AIR } else { UNKNOWN };
        }
        let key = Self::key(x, y, z);
        let i = (((y & 15) as usize) << 8) | (((z & 15) as usize) << 4) | ((x & 15) as usize);
        let c = &mut self.sections;
        let si = if c.last.0 == key {
            c.last.1
        } else {
            let n = c.secs.len() as u32;
            let si = *c.index.entry(key).or_insert(n);
            if si == n {
                c.secs.push(Box::new([NOT_CACHED; 4096]));
            }
            c.last = (key, si);
            si
        };
        let v = c.secs[si as usize][i];
        if v != NOT_CACHED {
            return v;
        }
        let v = match self.src.raw_block(x, y, z) {
            Some(raw) => self.table.index(raw),
            None => UNKNOWN,
        };
        self.sections.secs[si as usize][i] = v;
        v
    }

    /// The class at a position ([`Class::UNKNOWN`] when unknown).
    #[inline]
    pub(crate) fn class(&mut self, x: i32, y: i32, z: i32) -> Class {
        let i = self.index(x, y, z);
        if i == UNKNOWN {
            Class::UNKNOWN
        } else {
            self.table.classes[i as usize]
        }
    }

    /// `true` when the position is unknown.
    #[inline]
    pub(crate) fn unknown(&mut self, x: i32, y: i32, z: i32) -> bool {
        self.index(x, y, z) == UNKNOWN
    }
}

/// A fast hasher for packed section keys (one multiply; keys are well spread already).
#[derive(Clone, Copy, Default)]
pub(crate) struct BuildKeyHasher;

impl std::hash::BuildHasher for BuildKeyHasher {
    type Hasher = KeyHasher;
    fn build_hasher(&self) -> KeyHasher {
        KeyHasher(0)
    }
}

/// See [`BuildKeyHasher`].
#[derive(Clone, Copy, Default)]
pub(crate) struct KeyHasher(u64);

impl std::hash::Hasher for KeyHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(b)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        }
    }

    #[inline]
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0 ^ v)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .rotate_left(29);
    }

    #[inline]
    fn write_u32(&mut self, v: u32) {
        self.write_u64(u64::from(v));
    }

    #[inline]
    fn write_i32(&mut self, v: i32) {
        self.write_u64(u64::from(v as u32));
    }
}
