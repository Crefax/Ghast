//! Test-side encoders that write chunk data exactly in the vanilla wire formats, plus a tiny
//! deterministic RNG.

#![allow(dead_code)]

/// xorshift64* — deterministic, no dependency.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// One 1.8 section to encode.
pub struct LegacySection {
    pub blocks: Vec<u16>,
    pub block_light: Vec<u8>,
    pub sky_light: Vec<u8>,
}

/// Encodes 1.8 chunk data (vanilla `ChunkDataS2CPacket` layout): all block arrays, then all
/// block-light arrays, then (if `sky`) all sky-light arrays, then 256 biomes when `biomes` is
/// `Some` (ground-up). Returns `(mask, data)`.
pub fn encode_1_8(
    sections: &[Option<LegacySection>; 16],
    sky: bool,
    biomes: Option<&[u8; 256]>,
) -> (u16, Vec<u8>) {
    let mut mask = 0u16;
    let mut out = Vec::new();
    for (j, s) in sections.iter().enumerate() {
        if let Some(s) = s {
            mask |= 1 << j;
            for &b in &s.blocks {
                out.extend_from_slice(&b.to_le_bytes());
            }
        }
    }
    for s in sections.iter().flatten() {
        out.extend_from_slice(&s.block_light);
    }
    if sky {
        for s in sections.iter().flatten() {
            out.extend_from_slice(&s.sky_light);
        }
    }
    if let Some(b) = biomes {
        out.extend_from_slice(b);
    }
    (mask, out)
}

pub fn put_varint(out: &mut Vec<u8>, v: i32) {
    let mut v = v as u32;
    loop {
        if v & !0x7F == 0 {
            out.push(v as u8);
            return;
        }
        out.push((v as u8 & 0x7F) | 0x80);
        v >>= 7;
    }
}

/// Palette mode of an encoded 1.16 section.
#[derive(Clone, Copy, Debug)]
pub enum Palette16 {
    /// Indirect palette with this many bits per block written in the header and used for the data.
    Indirect(u8),
    /// Global ids; header value and data bits given separately (vanilla uses 15 bits whatever the
    /// header says).
    Direct { header: u8, bits: u32 },
}

/// Encodes one 1.16 section (vanilla `ChunkSection.toPacket` layout) from global ids.
/// For `Indirect`, the palette is the distinct ids in first-seen order (plus `extra_palette`
/// entries appended), indices are packed with the header bits (4 minimum, as vanilla).
pub fn encode_section_1_16(out: &mut Vec<u8>, ids: &[u16], mode: Palette16, extra_palette: &[i32]) {
    let non_air = ids.iter().filter(|&&s| s != 0).count() as i16;
    out.extend_from_slice(&non_air.to_be_bytes());
    match mode {
        Palette16::Indirect(header) => {
            out.push(header);
            let bits = u32::from(header.max(4));
            let mut palette: Vec<u16> = Vec::new();
            let mut idx = Vec::with_capacity(4096);
            for &s in ids {
                let p = match palette.iter().position(|&q| q == s) {
                    Some(p) => p,
                    None => {
                        palette.push(s);
                        palette.len() - 1
                    }
                };
                assert!(p < (1 << bits), "palette too large for {bits} bits");
                idx.push(p as u32);
            }
            put_varint(out, (palette.len() + extra_palette.len()) as i32);
            for &p in &palette {
                put_varint(out, i32::from(p));
            }
            for &p in extra_palette {
                put_varint(out, p);
            }
            put_words(out, bits, &idx);
        }
        Palette16::Direct { header, bits } => {
            out.push(header);
            let vals: Vec<u32> = ids.iter().map(|&s| u32::from(s)).collect();
            put_words(out, bits, &vals);
        }
    }
}

/// Writes a VarInt-prefixed long array in the 1.16 compact layout (big-endian words).
pub fn put_words(out: &mut Vec<u8>, bits: u32, vals: &[u32]) {
    let words = ghast_world::packed::pack(bits, vals);
    put_varint(out, words.len() as i32);
    for w in words {
        out.extend_from_slice(&w.to_be_bytes());
    }
}

/// NBT compound `{x: int, y: int, z: int, id: string}` (network form: unnamed root).
pub fn be_nbt(x: i32, y: i32, z: i32, id: &str) -> Vec<u8> {
    let mut v = vec![10u8, 0, 0];
    for (k, val) in [("x", x), ("y", y), ("z", z)] {
        v.push(3);
        v.extend_from_slice(&(k.len() as u16).to_be_bytes());
        v.extend_from_slice(k.as_bytes());
        v.extend_from_slice(&val.to_be_bytes());
    }
    v.push(8);
    v.extend_from_slice(&2u16.to_be_bytes());
    v.extend_from_slice(b"id");
    v.extend_from_slice(&(id.len() as u16).to_be_bytes());
    v.extend_from_slice(id.as_bytes());
    v.push(0);
    v
}
