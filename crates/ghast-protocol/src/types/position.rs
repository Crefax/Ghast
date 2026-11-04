//! Block positions packed into 64 bits.
//!
//! The packing changed in 1.14: before it the layout (high to low bits) is
//! `x:26 | y:12 | z:26`, from 1.14 on it is `x:26 | z:26 | y:12`. The generated packet code
//! picks the right function for its version; [`BlockPos`] itself is version-agnostic.

use bytes::{BufMut, BytesMut};

use crate::codec::{DecodeResult, EncodeResult, Reader};
use crate::error::EncodeError;

/// A block position. `x`/`z` must fit 26 signed bits, `y` 12 signed bits to be encodable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct BlockPos {
    /// X coordinate.
    pub x: i32,
    /// Y coordinate.
    pub y: i32,
    /// Z coordinate.
    pub z: i32,
}

/// Bit layout of a packed [`BlockPos`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionLayout {
    /// `x:26 | y:12 | z:26` (1.8 – 1.13.2).
    Xyz,
    /// `x:26 | z:26 | y:12` (1.14+).
    Xzy,
}

#[inline]
const fn sign_extend(v: u64, bits: u32) -> i32 {
    let shift = 64 - bits;
    (((v << shift) as i64) >> shift) as i32
}

#[inline]
const fn fits(v: i32, bits: u32) -> bool {
    let min = -(1i64 << (bits - 1));
    let max = (1i64 << (bits - 1)) - 1;
    (v as i64) >= min && (v as i64) <= max
}

impl BlockPos {
    /// Creates a position.
    #[inline]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        BlockPos { x, y, z }
    }

    /// Unpacks a 64-bit value with the given layout.
    #[inline]
    pub const fn unpack(v: u64, layout: PositionLayout) -> Self {
        match layout {
            PositionLayout::Xyz => BlockPos {
                x: sign_extend(v >> 38, 26),
                y: sign_extend((v >> 26) & 0xfff, 12),
                z: sign_extend(v & 0x3ff_ffff, 26),
            },
            PositionLayout::Xzy => BlockPos {
                x: sign_extend(v >> 38, 26),
                z: sign_extend((v >> 12) & 0x3ff_ffff, 26),
                y: sign_extend(v & 0xfff, 12),
            },
        }
    }

    /// Whether the coordinates fit the packed representation.
    #[inline]
    pub const fn is_packable(&self) -> bool {
        fits(self.x, 26) && fits(self.y, 12) && fits(self.z, 26)
    }

    /// Packs into 64 bits; fails if a coordinate does not fit.
    #[inline]
    pub fn pack(&self, layout: PositionLayout) -> Result<u64, EncodeError> {
        if !self.is_packable() {
            return Err(EncodeError::OutOfRange("position"));
        }
        let x = (self.x as u64) & 0x3ff_ffff;
        let y = (self.y as u64) & 0xfff;
        let z = (self.z as u64) & 0x3ff_ffff;
        Ok(match layout {
            PositionLayout::Xyz => (x << 38) | (y << 26) | z,
            PositionLayout::Xzy => (x << 38) | (z << 12) | y,
        })
    }

    /// Reads a packed position with the given layout.
    #[inline]
    pub fn read(r: &mut Reader<'_>, layout: PositionLayout) -> DecodeResult<Self> {
        Ok(Self::unpack(r.read_u64()?, layout))
    }

    /// Writes a packed position with the given layout.
    #[inline]
    pub fn write(&self, w: &mut BytesMut, layout: PositionLayout) -> EncodeResult {
        w.put_u64(self.pack(layout)?);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn known_answers() {
        // Hand-computed: x = 18357644 (0x118_1d8c), y = 831 (0x33f), z = -20882616.
        let p = BlockPos::new(18357644, 831, -20882616);
        let z = (-20882616i64 as u64) & 0x3ff_ffff;
        let xyz = (0x118_1d8cu64 << 38) | (0x33fu64 << 26) | z;
        let xzy = (0x118_1d8cu64 << 38) | (z << 12) | 0x33f;
        assert_eq!(p.pack(PositionLayout::Xyz), Ok(xyz));
        assert_eq!(p.pack(PositionLayout::Xzy), Ok(xzy));
        assert_eq!(BlockPos::unpack(xyz, PositionLayout::Xyz), p);
        assert_eq!(BlockPos::unpack(xzy, PositionLayout::Xzy), p);

        // (-1, -1, -1) is all ones in both layouts.
        let m = BlockPos::new(-1, -1, -1);
        assert_eq!(m.pack(PositionLayout::Xyz), Ok(u64::MAX));
        assert_eq!(m.pack(PositionLayout::Xzy), Ok(u64::MAX));

        // (1, 2, 3) bytes in 1.8 layout: x<<38 | y<<26 | z.
        let mut w = BytesMut::new();
        BlockPos::new(1, 2, 3)
            .write(&mut w, PositionLayout::Xyz)
            .unwrap();
        assert_eq!(&w[..], &[0x00, 0x00, 0x00, 0x40, 0x08, 0x00, 0x00, 0x03]);
        let b = Bytes::from(w.to_vec());
        assert_eq!(
            BlockPos::read(&mut Reader::new(&b), PositionLayout::Xyz),
            Ok(BlockPos::new(1, 2, 3))
        );
        // Same position in the 1.14+ layout: x<<38 | z<<12 | y.
        let mut w = BytesMut::new();
        BlockPos::new(1, 2, 3)
            .write(&mut w, PositionLayout::Xzy)
            .unwrap();
        assert_eq!(&w[..], &[0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x30, 0x02]);
    }

    #[test]
    fn range_checks() {
        assert!(BlockPos::new(33554431, 2047, -33554432).is_packable());
        assert_eq!(
            BlockPos::new(33554432, 0, 0).pack(PositionLayout::Xyz),
            Err(EncodeError::OutOfRange("position"))
        );
        assert!(BlockPos::new(0, 2048, 0).pack(PositionLayout::Xzy).is_err());
        assert!(
            BlockPos::new(0, -2049, 0)
                .pack(PositionLayout::Xzy)
                .is_err()
        );
    }
}
