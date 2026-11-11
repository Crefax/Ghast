//! Chunk section data of the modern chunk packet (`map_chunk.chunkData`), as sent by
//! protocol 775 (26.1 – 26.1.2).
//!
//! The packet carries the sections as one byte array; this module walks it without
//! copying. Per section, bottom to top:
//!
//! - `i16` non-empty block count, `i16` fluid count;
//! - block states: a paletted container of 16³ entries;
//! - biomes: a paletted container of 4³ entries.
//!
//! A paletted container is a `u8` bit count, the palette and a long array whose length is
//! implied by the bit count (no length prefix since 1.21.5):
//!
//! | bits (blocks / biomes) | palette | entries |
//! |---|---|---|
//! | 0 | one VarInt id | no longs |
//! | 1–8 / 1–3 | VarInt count + ids | indices into the palette |
//! | larger | none | global ids |
//!
//! Block containers with 1–3 bits are stored with 4 bits (vanilla never sends fewer than
//! 4 for an indirect block palette). Entries are packed little-end first into each long and
//! do not span longs. The number of sections depends on the dimension's height
//! (`dimension_type` registry); [`Sections`] simply reads until the data ends.

use bytes::Bytes;

use crate::codec::{DecodeResult, Reader};
use crate::error::DecodeError;

/// What a paletted container holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerKind {
    /// 4096 block states.
    Blocks,
    /// 64 biomes.
    Biomes,
}

impl ContainerKind {
    /// Number of entries.
    pub const fn entries(self) -> usize {
        match self {
            ContainerKind::Blocks => 4096,
            ContainerKind::Biomes => 64,
        }
    }

    /// Largest bit count that still uses an indirect palette.
    const fn max_indirect(self) -> u8 {
        match self {
            ContainerKind::Blocks => 8,
            ContainerKind::Biomes => 3,
        }
    }

    /// Bits per entry in the long array for a wire bit count.
    const fn storage_bits(self, bits: u8) -> u8 {
        match self {
            ContainerKind::Blocks if bits >= 1 && bits <= 4 => 4,
            _ => bits,
        }
    }
}

/// The palette of a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Palette {
    /// Every entry has this id.
    Single(i32),
    /// Entries are indices into this list of ids.
    Indirect(Vec<i32>),
    /// Entries are global ids.
    Direct,
}

/// One paletted container; the long array stays a zero-copy slice of the packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PalettedContainer {
    /// Kind (entry count).
    pub kind: ContainerKind,
    /// Bit count as sent.
    pub bits: u8,
    /// The palette.
    pub palette: Palette,
    /// The packed big-endian longs.
    pub data: Bytes,
}

impl PalettedContainer {
    /// Reads one container.
    pub fn read(r: &mut Reader<'_>, kind: ContainerKind) -> DecodeResult<Self> {
        let bits = r.read_u8()?;
        let palette = if bits == 0 {
            Palette::Single(r.read_varint()?)
        } else if bits <= kind.max_indirect() {
            let n = r.read_varint()?;
            let n = r.check_count(i64::from(n), 1)?;
            let mut ids = Vec::with_capacity(n.min(256));
            for _ in 0..n {
                ids.push(r.read_varint()?);
            }
            Palette::Indirect(ids)
        } else {
            Palette::Direct
        };
        let longs = Self::long_count(kind, bits).ok_or(DecodeError::Invalid("palette bits"))?;
        let data = r.read_bytes(longs * 8)?;
        Ok(PalettedContainer {
            kind,
            bits,
            palette,
            data,
        })
    }

    /// Number of longs for a wire bit count (`None` if the bit count is invalid).
    pub fn long_count(kind: ContainerKind, bits: u8) -> Option<usize> {
        if bits == 0 {
            return Some(0);
        }
        let b = kind.storage_bits(bits);
        if b > 32 {
            return None;
        }
        let per_long = 64 / usize::from(b);
        Some(kind.entries().div_ceil(per_long))
    }

    /// The id at entry `index` (`y << 8 | z << 4 | x` for blocks, `y << 4 | z << 2 | x`
    /// for biomes). `None` if the index or a palette index is out of range.
    pub fn get(&self, index: usize) -> Option<i32> {
        if index >= self.kind.entries() {
            return None;
        }
        let raw = match &self.palette {
            Palette::Single(id) => return Some(*id),
            _ => {
                let b = usize::from(self.kind.storage_bits(self.bits));
                let per_long = 64 / b;
                let (li, slot) = (index / per_long, index % per_long);
                let at = li * 8;
                let long = u64::from_be_bytes(self.data.get(at..at + 8)?.try_into().ok()?);
                ((long >> (slot * b)) & ((1u64 << b) - 1)) as usize
            }
        };
        match &self.palette {
            Palette::Indirect(ids) => ids.get(raw).copied(),
            _ => i32::try_from(raw).ok(),
        }
    }
}

/// One chunk section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSection {
    /// Blocks that are not air.
    pub block_count: i16,
    /// Blocks with a fluid.
    pub fluid_count: i16,
    /// Block states.
    pub blocks: PalettedContainer,
    /// Biomes.
    pub biomes: PalettedContainer,
}

impl ChunkSection {
    /// Reads one section.
    pub fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Ok(ChunkSection {
            block_count: r.read_i16()?,
            fluid_count: r.read_i16()?,
            blocks: PalettedContainer::read(r, ContainerKind::Blocks)?,
            biomes: PalettedContainer::read(r, ContainerKind::Biomes)?,
        })
    }
}

/// Iterator over the sections of `map_chunk.chunkData`, bottom to top.
#[derive(Debug, Clone)]
pub struct Sections<'a> {
    r: Reader<'a>,
    failed: bool,
}

impl<'a> Sections<'a> {
    /// Walks `data`.
    pub fn new(data: &'a Bytes) -> Self {
        Sections {
            r: Reader::new(data),
            failed: false,
        }
    }
}

impl Iterator for Sections<'_> {
    type Item = DecodeResult<ChunkSection>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.r.remaining() == 0 {
            return None;
        }
        let s = ChunkSection::read(&mut self.r);
        self.failed = s.is_err();
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::McWrite;
    use bytes::{BufMut, BytesMut};

    #[test]
    fn single_and_indirect() {
        let mut w = BytesMut::new();
        // Section 1: all stone (id 1), plains biome (id 0).
        w.put_i16(4096);
        w.put_i16(0);
        w.put_u8(0);
        w.put_varint(1);
        w.put_u8(0);
        w.put_varint(0);
        // Section 2: indirect palette [air, grass] with 4 bits, entry 0 = grass.
        w.put_i16(1);
        w.put_i16(0);
        w.put_u8(4);
        w.put_varint(2);
        w.put_varint(0);
        w.put_varint(9);
        for i in 0..256 {
            w.put_u64(if i == 0 { 1 } else { 0 });
        }
        w.put_u8(0);
        w.put_varint(3);
        let data = w.freeze();
        let s: Vec<ChunkSection> = Sections::new(&data).map(Result::unwrap).collect();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].blocks.get(123), Some(1));
        assert_eq!(s[1].blocks.get(0), Some(9));
        assert_eq!(s[1].blocks.get(1), Some(0));
        assert_eq!(s[1].biomes.get(5), Some(3));
        assert_eq!(s[1].blocks.get(4096), None);
        assert_eq!(
            PalettedContainer::long_count(ContainerKind::Blocks, 15),
            Some(1024)
        );
        assert_eq!(
            PalettedContainer::long_count(ContainerKind::Biomes, 7),
            Some(8)
        );
    }
}
