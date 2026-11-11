//! The low-precision velocity vector of 1.21.9+ (`spawn_entity`, `entity_velocity`, …).
//!
//! Wire format: a single `0` byte is the zero vector. Otherwise 6 bytes form a 48-bit
//! little-endian-ordered word — byte 0 (bits 0–7), byte 1 (bits 8–15), then a big-endian
//! `u32` (bits 16–47). Bits 0–1 are the low bits of an integer scale, bit 2 says that a
//! VarInt with the remaining scale bits (`scale >> 2`) follows, and bits 3–17, 18–32,
//! 33–47 hold x, y, z as 15-bit values `q` meaning `min(q, 32766) * 2 / 32766 - 1`
//! (times the scale).
//!
//! [`LpVec3`] keeps the exact wire form, so decoding and re-encoding is lossless;
//! [`LpVec3::to_vec3`] / [`LpVec3::from_vec3`] convert to and from blocks per tick.

use bytes::{BufMut, BytesMut};

use crate::codec::{Decode, DecodeResult, Encode, EncodeResult, McWrite, Reader};

const MAX_Q: f64 = 32766.0;
const ABS_MAX: f64 = 1.717_986_918_3e10;
const ABS_MIN: f64 = 3.051_944_088_384_301e-5;

/// A quantised velocity vector in its wire form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LpVec3 {
    /// The 48-bit packed word; `0` is the zero vector (its low byte is never 0 otherwise).
    pub packed: u64,
    /// Scale bits above the two in `packed` (present on the wire iff bit 2 is set).
    pub scale_high: u32,
}

impl LpVec3 {
    /// The zero vector.
    pub const ZERO: LpVec3 = LpVec3 {
        packed: 0,
        scale_high: 0,
    };

    /// Reads the wire form.
    pub fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let lowest = r.read_u8()?;
        if lowest == 0 {
            return Ok(Self::ZERO);
        }
        let middle = r.read_u8()?;
        let highest = r.read_u32()?;
        let packed = u64::from(highest) << 16 | u64::from(middle) << 8 | u64::from(lowest);
        let scale_high = if lowest & 4 != 0 {
            r.read_varint()? as u32
        } else {
            0
        };
        Ok(LpVec3 { packed, scale_high })
    }

    /// Writes the wire form. A vector whose low byte is 0 but is not [`LpVec3::ZERO`]
    /// cannot be represented and is written as zero; without the continuation bit
    /// `scale_high` is not written.
    pub fn write(&self, w: &mut BytesMut) {
        let lowest = self.packed as u8;
        if lowest == 0 {
            w.put_u8(0);
            return;
        }
        w.put_u8(lowest);
        w.put_u8((self.packed >> 8) as u8);
        w.put_u32((self.packed >> 16) as u32);
        if lowest & 4 != 0 {
            w.put_varint(self.scale_high as i32);
        }
    }

    /// The integer scale.
    pub fn scale(&self) -> u64 {
        let low = self.packed & 3;
        if self.packed & 4 != 0 {
            low | u64::from(self.scale_high) << 2
        } else {
            low
        }
    }

    /// The vector in blocks per tick.
    pub fn to_vec3(&self) -> [f64; 3] {
        if self.packed == 0 {
            return [0.0; 3];
        }
        let s = self.scale() as f64;
        let unpack =
            |shift: u32| ((self.packed >> shift & 0x7fff) as f64).min(MAX_Q) * 2.0 / MAX_Q - 1.0;
        [unpack(3) * s, unpack(18) * s, unpack(33) * s]
    }

    /// Quantises a vector the way the server does.
    pub fn from_vec3(v: [f64; 3]) -> Self {
        let clean = |x: f64| {
            if x.is_nan() {
                0.0
            } else {
                x.clamp(-ABS_MAX, ABS_MAX)
            }
        };
        let [x, y, z] = v.map(clean);
        let len = x.abs().max(y.abs()).max(z.abs());
        if len < ABS_MIN {
            return Self::ZERO;
        }
        // Smallest integer >= len (`len` is at most 2^34).
        let scale = len.ceil() as u64;
        let partial = scale & 3 != scale;
        let markers = if partial { scale & 3 | 4 } else { scale };
        // Non-negative, so `round` (ties away from zero) rounds like Java's Math.round.
        let pack = |c: f64| ((c / scale as f64 * 0.5 + 0.5) * MAX_Q).round() as u64;
        let packed = markers | pack(x) << 3 | pack(y) << 18 | pack(z) << 33;
        LpVec3 {
            packed,
            scale_high: if partial { (scale >> 2) as u32 } else { 0 },
        }
    }
}

impl Decode for LpVec3 {
    #[inline]
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self> {
        Self::read(r)
    }
}

impl Encode for LpVec3 {
    #[inline]
    fn encode(&self, w: &mut BytesMut) -> EncodeResult {
        self.write(w);
        Ok(())
    }
}

#[cfg(feature = "testgen")]
impl crate::testgen::Generate for LpVec3 {
    fn generate(g: &mut crate::testgen::Gen) -> Self {
        if g.chance(15) {
            return Self::ZERO;
        }
        // Any 48-bit word with a non-zero low byte is a valid wire form.
        let mut packed = g.u64() & 0xffff_ffff_ffff;
        if packed & 0xff == 0 {
            packed |= 1;
        }
        let scale_high = if packed & 4 != 0 { g.u64() as u32 } else { 0 };
        LpVec3 { packed, scale_high }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::decode_exact;

    fn rt(v: LpVec3) -> Vec<u8> {
        let mut w = BytesMut::new();
        v.write(&mut w);
        assert_eq!(decode_exact::<LpVec3>(&w.clone().freeze()).unwrap(), v);
        w.to_vec()
    }

    #[test]
    fn zero_is_one_byte() {
        assert_eq!(rt(LpVec3::from_vec3([0.0, 1e-6, -1e-6])), vec![0]);
        assert_eq!(LpVec3::ZERO.to_vec3(), [0.0; 3]);
    }

    #[test]
    fn quantisation() {
        // Typical falling velocity: scale 1, no continuation.
        let v = LpVec3::from_vec3([0.0, -0.0784000015258789, 0.0]);
        assert_eq!(v.scale(), 1);
        assert_eq!(rt(v).len(), 6);
        let back = v.to_vec3();
        assert!((back[1] + 0.0784).abs() < 1.0 / 16383.0);
        assert!(back[0].abs() < 1e-4 && back[2].abs() < 1e-4);
        // Large values need the VarInt scale extension.
        let big = LpVec3::from_vec3([100.0, -3.0, 0.5]);
        assert_eq!(big.scale(), 100);
        assert_eq!(big.packed & 4, 4);
        assert_eq!(rt(big).len(), 7);
        assert!((big.to_vec3()[0] - 100.0).abs() < 0.01);
        // NaN is treated as 0.
        assert_eq!(LpVec3::from_vec3([f64::NAN; 3]), LpVec3::ZERO);
    }
}
