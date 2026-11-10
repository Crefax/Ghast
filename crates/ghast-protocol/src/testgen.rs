//! Deterministic random value generation for tests (feature `testgen`).
//!
//! Every generated type implements [`Generate`]. Generated values are always encodable:
//! switch discriminants are generated first and conditional fields follow them, counts
//! match their arrays, packed fields are in range. Values are biased toward the case
//! keys that switches compare against, so every branch gets exercised.
//!
//! This module is test support, not part of the stable API.

use bytes::{Bytes, BytesMut};
use bytestring::ByteString;
use uuid::Uuid;

use crate::codec::EncodeResult;
use crate::nbt::{AnonNbt, Compound, List, NamedTag, RawNbt, Tag, tag};
use crate::types::BlockPos;

/// A type that can produce random values.
pub trait Generate: Sized {
    /// Generates one value.
    fn generate(g: &mut Gen) -> Self;
}

/// Seeded pseudo-random generator (SplitMix64).
#[derive(Debug, Clone)]
pub struct Gen {
    state: u64,
    /// Restrict values to what node-minecraft-protocol round-trips (its `varlong` is
    /// really 32-bit, its NBT root reader cannot take a bare TAG_End).
    pub node_compat: bool,
}

impl Gen {
    /// Creates a generator from a seed.
    pub fn new(seed: u64) -> Self {
        Gen {
            state: seed ^ 0x9e37_79b9_7f4a_7c15,
            node_compat: false,
        }
    }

    /// Next raw 64 random bits.
    pub fn u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: u64) -> u64 {
        self.u64() % n
    }

    /// True with probability `percent`/100.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// A random boolean.
    pub fn bool(&mut self) -> bool {
        self.u64() & 1 == 1
    }

    /// An integer in `[lo, hi]`, preferring `hints`, small values and the bounds.
    pub fn int_in(&mut self, lo: i64, hi: i64, hints: &[i64]) -> i64 {
        let ok: Vec<i64> = hints
            .iter()
            .copied()
            .filter(|v| (lo..=hi).contains(v))
            .collect();
        if !ok.is_empty() && self.chance(65) {
            return ok[self.below(ok.len() as u64) as usize];
        }
        match self.below(10) {
            0..=2 => (self.below(20) as i64 - 3).clamp(lo, hi),
            3 => lo,
            4 => hi,
            _ => {
                let span = (hi as i128 - lo as i128 + 1) as u128;
                (lo as i128 + (u128::from(self.u64()) % span) as i128) as i64
            }
        }
    }

    /// A count for a field that sizes an array (0..=3).
    pub fn small_count(&mut self) -> u8 {
        self.below(4) as u8
    }

    /// A collection length (mostly small).
    pub fn count(&mut self) -> usize {
        if self.chance(90) {
            self.below(4) as usize
        } else {
            self.below(13) as usize
        }
    }

    /// A random `i64`.
    pub fn i64(&mut self) -> i64 {
        self.int_in(i64::MIN, i64::MAX, &[])
    }

    /// A random VarLong value (non-negative 32-bit in node-compat mode: node's varlong
    /// is really an unsigned 32-bit VarInt).
    pub fn varlong(&mut self) -> i64 {
        if self.node_compat {
            self.int_in(0, i64::from(i32::MAX), &[])
        } else {
            self.i64()
        }
    }

    /// A random finite `f32`.
    pub fn f32(&mut self) -> f32 {
        match self.below(4) {
            0 => [0.0, -0.0, 1.0, -1.5, 0.25, 64.0][self.below(6) as usize],
            _ => loop {
                let v = f32::from_bits(self.u64() as u32);
                if v.is_finite() {
                    break v;
                }
            },
        }
    }

    /// A random finite `f64`.
    pub fn f64(&mut self) -> f64 {
        match self.below(4) {
            0 => [0.0, -0.0, 1.0, -1.5, 0.25, 1e300][self.below(6) as usize],
            _ => loop {
                let v = f64::from_bits(self.u64());
                if v.is_finite() {
                    break v;
                }
            },
        }
    }

    /// A random UUID.
    pub fn uuid(&mut self) -> Uuid {
        Uuid::from_u64_pair(self.u64(), self.u64())
    }

    fn text(&mut self, bmp_only: bool) -> String {
        const CHARS: &[char] = &[
            'a',
            'b',
            'z',
            'A',
            'Q',
            '0',
            '9',
            ' ',
            '_',
            ':',
            '{',
            '"',
            '\u{a7}',
            '\u{e9}',
            '\u{131}',
            '\u{65e5}',
            '\u{1f600}',
        ];
        let n = self.below(12) as usize;
        (0..n)
            .map(|_| CHARS[self.below(CHARS.len() as u64 - u64::from(bmp_only)) as usize])
            .collect()
    }

    /// A random string.
    pub fn string(&mut self) -> ByteString {
        ByteString::from(self.text(false))
    }

    /// One of `hints` (usually) or a random string.
    pub fn string_in(&mut self, hints: &[&str]) -> ByteString {
        if !hints.is_empty() && self.chance(80) {
            ByteString::from(hints[self.below(hints.len() as u64) as usize].to_string())
        } else {
            self.string()
        }
    }

    /// Random bytes.
    pub fn bytes(&mut self) -> Bytes {
        let n = self.below(25) as usize;
        (0..n).map(|_| self.u64() as u8).collect::<Vec<u8>>().into()
    }

    /// A random packable block position.
    pub fn position(&mut self) -> BlockPos {
        BlockPos::new(
            self.int_in(-(1 << 25), (1 << 25) - 1, &[0]) as i32,
            self.int_in(-2048, 2047, &[0, 64]) as i32,
            self.int_in(-(1 << 25), (1 << 25) - 1, &[0]) as i32,
        )
    }

    fn nbt_tag(&mut self, ty: u8, depth: u32) -> Tag {
        match ty {
            tag::BYTE => Tag::Byte(self.u64() as i8),
            tag::SHORT => Tag::Short(self.u64() as i16),
            tag::INT => Tag::Int(self.u64() as i32),
            tag::LONG => Tag::Long(self.u64() as i64),
            tag::FLOAT => Tag::Float(self.f32()),
            tag::DOUBLE => Tag::Double(self.f64()),
            tag::BYTE_ARRAY => Tag::ByteArray(self.bytes()),
            tag::STRING => Tag::String(self.text(true)),
            tag::LIST => {
                let elem = if depth >= 3 {
                    1 + self.below(8) as u8
                } else {
                    1 + self.below(12) as u8
                };
                let n = self.count();
                let items: Vec<Tag> = (0..n).map(|_| self.nbt_tag(elem, depth + 1)).collect();
                let elem = if items.is_empty() && self.bool() {
                    tag::END
                } else {
                    elem
                };
                Tag::List(List {
                    elem_type: elem,
                    items,
                })
            }
            tag::COMPOUND => {
                let n = if depth >= 3 { 0 } else { self.count() };
                // Unique keys that do not look like array indices (JavaScript objects
                // reorder those, which would break byte comparisons with node).
                Tag::Compound(Compound(
                    (0..n)
                        .map(|i| {
                            let t = 1 + self.below(12) as u8;
                            (
                                format!("k{i}{}", self.text(true)),
                                self.nbt_tag(t, depth + 1),
                            )
                        })
                        .collect(),
                ))
            }
            tag::INT_ARRAY => Tag::IntArray((0..self.count()).map(|_| self.u64() as i32).collect()),
            _ => Tag::LongArray((0..self.count()).map(|_| self.u64() as i64).collect()),
        }
    }

    /// A random named root compound as raw NBT.
    pub fn nbt(&mut self) -> RawNbt {
        if !self.node_compat && self.chance(10) {
            return RawNbt::END;
        }
        let t = NamedTag {
            name: if self.chance(70) {
                String::new()
            } else {
                self.text(true)
            },
            tag: self.nbt_tag(tag::COMPOUND, 0),
        };
        RawNbt::from_named(&t).expect("generated NBT is encodable")
    }

    /// A random unnamed network NBT root (any tag type, TAG_End 10% of the time).
    pub fn anon_nbt(&mut self) -> AnonNbt {
        if self.chance(10) {
            return AnonNbt::END;
        }
        let ty = if self.chance(50) {
            tag::COMPOUND
        } else {
            1 + self.below(12) as u8
        };
        let t = self.nbt_tag(ty, 0);
        AnonNbt::from_tag(&t).expect("generated NBT is encodable")
    }

    /// Optional unnamed NBT (`None` 30% of the time).
    pub fn anon_opt_nbt(&mut self) -> Option<AnonNbt> {
        if self.chance(30) {
            return None;
        }
        let t = self.nbt_tag(tag::COMPOUND, 0);
        Some(AnonNbt::from_tag(&t).expect("generated NBT is encodable"))
    }

    /// Exactly `n` random bytes.
    pub fn fixed_bytes(&mut self, n: usize) -> Bytes {
        Bytes::from((0..n).map(|_| self.u64() as u8).collect::<Vec<u8>>())
    }

    /// Optional NBT (`None` 30% of the time).
    pub fn opt_nbt(&mut self) -> Option<RawNbt> {
        if self.chance(30) {
            None
        } else {
            let t = NamedTag {
                name: String::new(),
                tag: self.nbt_tag(tag::COMPOUND, 0),
            };
            Some(RawNbt::from_named(&t).expect("generated NBT is encodable"))
        }
    }
}

/// Encodes with `f` and checks the first written byte with `pred` (used to keep
/// generated entity-metadata entries from looking like terminators).
pub fn first_byte_ok(
    f: impl FnOnce(&mut BytesMut) -> EncodeResult,
    pred: impl Fn(u8) -> bool,
) -> bool {
    let mut w = BytesMut::new();
    f(&mut w).is_ok() && w.first().is_some_and(|&b| pred(b))
}

impl Generate for crate::types::slot::legacy::Slot {
    fn generate(g: &mut Gen) -> Self {
        use crate::types::slot::legacy::{ItemStack, Slot};
        if g.chance(25) {
            return Slot(None);
        }
        let id = loop {
            let id = g.int_in(i64::from(i16::MIN), i64::from(i16::MAX), &[1, 276]) as i16;
            if id != -1 {
                break id;
            }
        };
        Slot(Some(ItemStack {
            id,
            count: g.u64() as i8,
            damage: g.u64() as i16,
            nbt: g.opt_nbt(),
        }))
    }
}

impl Generate for crate::types::slot::modern::Slot {
    fn generate(g: &mut Gen) -> Self {
        use crate::types::slot::modern::{ItemStack, Slot};
        if g.chance(25) {
            return Slot(None);
        }
        Slot(Some(ItemStack {
            id: g.int_in(i64::from(i32::MIN), i64::from(i32::MAX), &[1, 600]) as i32,
            count: g.u64() as i8,
            nbt: g.opt_nbt(),
        }))
    }
}
