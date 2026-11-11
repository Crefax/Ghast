//! Compact chunk sections for the modern (1.18+, 26.x) worlds: block states kept packed with a
//! small palette, at the smallest power-of-two width that holds the section's distinct states.
//!
//! | variant | distinct states | heap bytes | lookup |
//! |---|---|---|---|
//! | [`Section::Uniform`] | 1 (including all air) | 0 | 1 match |
//! | [`Section::Bits1`] | 2 | 512 + 4 | word load, shift, palette load |
//! | [`Section::Bits2`] | 3 – 4 | 1024 + 8 | same |
//! | [`Section::Bits4`] | 5 – 16 | 2048 + 32 | same |
//! | [`Section::Bits8`] | 17 – 256 | 4096 + 512 | same |
//! | [`Section::Direct`] | > 256 | 8192 | 1 load |
//!
//! Power-of-two widths keep every lookup to shifts and masks (no division). The widths match the
//! wire for the common case (vanilla sends 4-bit sections for up to 16 states), so decoding such a
//! section is a byte-swapping copy. Measured on a default-generated 26.1.2 world (see the crate
//! README): 18.6 KB of block storage per overworld chunk, against 38.8 KB for the byte-per-block
//! representation the older worlds use.
//!
//! A palette index past the palette's used entries reads as palette entry 0 of the padded
//! palette (air unless the section was built with air at 0): decoding rejects such indices, so
//! this only matters for sections built by hand.

/// Number of blocks in a section.
pub const VOLUME: usize = 4096;

/// Packed palette indices of one section: `B` bits per block (`W` = `4096 * B / 64` words) and a
/// palette of `P` = `2^B` raw states.
#[derive(Clone)]
pub struct Packed<const B: u32, const W: usize, const P: usize> {
    words: [u64; W],
    palette: [u16; P],
    /// Used palette entries (`1..=P`).
    len: u16,
}

/// 1 bit per block.
pub type Packed1 = Packed<1, 64, 2>;
/// 2 bits per block.
pub type Packed2 = Packed<2, 128, 4>;
/// 4 bits per block.
pub type Packed4 = Packed<4, 256, 16>;
/// 8 bits per block.
pub type Packed8 = Packed<8, 512, 256>;

impl<const B: u32, const W: usize, const P: usize> Packed<B, W, P> {
    const PER_WORD_LOG2: u32 = 6 - B.trailing_zeros();
    const MASK: u64 = (1u64 << B) - 1;

    fn zeroed() -> Box<Self> {
        Box::new(Packed {
            words: [0; W],
            palette: [0; P],
            len: 0,
        })
    }

    /// Palette index of block `i`.
    #[inline]
    fn index(&self, i: usize) -> usize {
        let w = self.words[(i & (VOLUME - 1)) >> Self::PER_WORD_LOG2];
        let shift = (i & ((1 << Self::PER_WORD_LOG2) - 1)) as u32 * B;
        ((w >> shift) & Self::MASK) as usize
    }

    #[inline]
    fn set_index(&mut self, i: usize, p: usize) {
        let wi = (i & (VOLUME - 1)) >> Self::PER_WORD_LOG2;
        let shift = (i & ((1 << Self::PER_WORD_LOG2) - 1)) as u32 * B;
        let w = &mut self.words[wi];
        *w = (*w & !(Self::MASK << shift)) | ((p as u64 & Self::MASK) << shift);
    }

    /// Raw state of block `i`.
    #[inline]
    #[must_use]
    pub fn get(&self, i: usize) -> u16 {
        self.palette[self.index(i) & (P - 1)]
    }

    /// The used palette entries.
    #[must_use]
    pub fn palette(&self) -> &[u16] {
        &self.palette[..usize::from(self.len)]
    }

    /// Palette index of `state`, appending it when there is room.
    fn find_or_insert(&mut self, state: u16) -> Option<usize> {
        let len = usize::from(self.len);
        if let Some(p) = self.palette[..len].iter().position(|&s| s == state) {
            return Some(p);
        }
        if len < P {
            self.palette[len] = state;
            self.len += 1;
            return Some(len);
        }
        None
    }

    /// Calls `f(i, palette index)` for every block.
    fn for_each_index(&self, mut f: impl FnMut(usize, usize)) {
        let per = 1usize << Self::PER_WORD_LOG2;
        for (wi, &w) in self.words.iter().enumerate() {
            let mut w = w;
            for j in 0..per {
                f(wi * per + j, (w & Self::MASK) as usize);
                w >>= B;
            }
        }
    }

    /// Bitmap of the palette entries some block uses.
    fn used(&self) -> [u64; 4] {
        let mut used = [0u64; 4];
        self.for_each_index(|_, p| used[p >> 6] |= 1 << (p & 63));
        used
    }
}

/// A chunk section of the modern worlds (see the module docs). Raw states are global palette
/// ids, already validated by the decoder.
#[derive(Clone)]
#[non_exhaustive]
pub enum Section {
    /// Every block has this state (an all-air section is `Uniform(0)`).
    Uniform(u16),
    /// Two distinct states.
    Bits1(Box<Packed1>),
    /// Up to 4 distinct states.
    Bits2(Box<Packed2>),
    /// Up to 16 distinct states.
    Bits4(Box<Packed4>),
    /// Up to 256 distinct states.
    Bits8(Box<Packed8>),
    /// More than 256 distinct states: one raw state per block.
    Direct(Box<[u16; VOLUME]>),
}

impl std::fmt::Debug for Section {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Section::Uniform(s) => f.debug_tuple("Uniform").field(s).finish(),
            Section::Bits1(p) => f.debug_tuple("Bits1").field(&p.palette()).finish(),
            Section::Bits2(p) => f.debug_tuple("Bits2").field(&p.palette()).finish(),
            Section::Bits4(p) => f.debug_tuple("Bits4").field(&p.palette()).finish(),
            Section::Bits8(p) => f.debug_tuple("Bits8").field(&p.palette()).finish(),
            Section::Direct(_) => f.write_str("Direct"),
        }
    }
}

impl Default for Section {
    fn default() -> Self {
        Section::Uniform(0)
    }
}

/// Section index of a block: `y << 8 | z << 4 | x` on the low four bits of each coordinate.
#[inline]
#[must_use]
pub const fn block_index(x: i32, y: i32, z: i32) -> usize {
    (((y & 15) << 8) | ((z & 15) << 4) | (x & 15)) as usize
}

impl Section {
    /// Raw state of block `i` ([`block_index`]).
    #[inline]
    #[must_use]
    pub fn get(&self, i: usize) -> u16 {
        match self {
            Section::Uniform(s) => *s,
            Section::Bits4(p) => p.get(i),
            Section::Bits8(p) => p.get(i),
            Section::Bits2(p) => p.get(i),
            Section::Bits1(p) => p.get(i),
            Section::Direct(d) => d[i & (VOLUME - 1)],
        }
    }

    /// Bits per block in memory (0 for uniform, 16 for direct).
    #[must_use]
    pub fn bits(&self) -> u32 {
        match self {
            Section::Uniform(_) => 0,
            Section::Bits1(_) => 1,
            Section::Bits2(_) => 2,
            Section::Bits4(_) => 4,
            Section::Bits8(_) => 8,
            Section::Direct(_) => 16,
        }
    }

    /// `true` when every block is raw state 0 (air).
    #[must_use]
    pub fn is_all_air(&self) -> bool {
        matches!(self, Section::Uniform(0))
    }

    /// Heap bytes owned by this section.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        match self {
            Section::Uniform(_) => 0,
            Section::Bits1(_) => size_of::<Packed1>(),
            Section::Bits2(_) => size_of::<Packed2>(),
            Section::Bits4(_) => size_of::<Packed4>(),
            Section::Bits8(_) => size_of::<Packed8>(),
            Section::Direct(_) => size_of::<[u16; VOLUME]>(),
        }
    }

    /// Calls `f(index, raw_state)` for every block, in index order.
    pub fn for_each(&self, mut f: impl FnMut(usize, u16)) {
        match self {
            Section::Uniform(s) => (0..VOLUME).for_each(|i| f(i, *s)),
            Section::Bits1(p) => p.for_each_index(|i, k| f(i, p.palette[k])),
            Section::Bits2(p) => p.for_each_index(|i, k| f(i, p.palette[k])),
            Section::Bits4(p) => p.for_each_index(|i, k| f(i, p.palette[k])),
            Section::Bits8(p) => p.for_each_index(|i, k| f(i, p.palette[k])),
            Section::Direct(d) => d.iter().enumerate().for_each(|(i, &s)| f(i, s)),
        }
    }

    /// Sets block `i` to `state`; returns the previous state.
    pub fn set(&mut self, i: usize, state: u16) -> u16 {
        let i = i & (VOLUME - 1);
        let old = self.get(i);
        if old == state {
            return old;
        }
        macro_rules! packed {
            ($p:expr, $next:ident) => {{
                if let Some(k) = $p.find_or_insert(state) {
                    $p.set_index(i, k);
                    return old;
                }
                // Full: drop unused entries, else widen.
                if let Some(s) = compact(&**$p) {
                    *self = s;
                } else {
                    *self = $next(&**$p);
                }
                self.set(i, state);
                return old;
            }};
        }
        match self {
            Section::Uniform(u) => {
                let mut p = Packed1::zeroed();
                p.palette = [*u, state];
                p.len = 2;
                p.set_index(i, 1);
                *self = Section::Bits1(p);
            }
            Section::Bits1(p) => packed!(p, widen1),
            Section::Bits2(p) => packed!(p, widen2),
            Section::Bits4(p) => packed!(p, widen4),
            Section::Bits8(p) => packed!(p, widen8),
            Section::Direct(d) => d[i] = state,
        }
        old
    }
}

/// Re-packs `p` without its unused palette entries; `None` when every entry is used.
fn compact<const B: u32, const W: usize, const P: usize>(p: &Packed<B, W, P>) -> Option<Section> {
    let used = p.used();
    let n: u32 = used.iter().map(|w| w.count_ones()).sum();
    if n as usize >= usize::from(p.len) {
        return None;
    }
    let mut palette = [0u16; 256];
    let mut remap = [0u8; 256];
    let mut k = 0usize;
    for e in 0..usize::from(p.len) {
        if used[e >> 6] & (1 << (e & 63)) != 0 {
            remap[e] = k as u8;
            palette[k] = p.palette[e];
            k += 1;
        }
    }
    let mut idx = [0u8; VOLUME];
    p.for_each_index(|i, e| idx[i] = remap[e]);
    Some(build_from_buffer(&palette[..k], &idx))
}

fn widen1(p: &Packed1) -> Section {
    Section::Bits2(repack(p))
}
fn widen2(p: &Packed2) -> Section {
    Section::Bits4(repack(p))
}
fn widen4(p: &Packed4) -> Section {
    Section::Bits8(repack(p))
}
fn widen8(p: &Packed8) -> Section {
    let mut d = Box::new([0u16; VOLUME]);
    p.for_each_index(|i, k| d[i] = p.palette[k]);
    Section::Direct(d)
}

/// Copies `p` into a wider packing with the same palette.
fn repack<
    const B: u32,
    const W: usize,
    const P: usize,
    const B2: u32,
    const W2: usize,
    const P2: usize,
>(
    p: &Packed<B, W, P>,
) -> Box<Packed<B2, W2, P2>> {
    let mut q = Packed::<B2, W2, P2>::zeroed();
    q.palette[..usize::from(p.len)].copy_from_slice(p.palette());
    q.len = p.len;
    p.for_each_index(|i, k| q.set_index(i, k));
    q
}

/// Builds the smallest section for `palette` (at most 256 distinct states) from palette indices.
fn build_from_buffer(palette: &[u16], idx: &[u8; VOLUME]) -> Section {
    fn mk<const B: u32, const W: usize, const P: usize>(
        palette: &[u16],
        idx: &[u8; VOLUME],
    ) -> Box<Packed<B, W, P>> {
        let mut q = Packed::<B, W, P>::zeroed();
        q.palette[..palette.len()].copy_from_slice(palette);
        q.len = palette.len() as u16;
        let per = VOLUME / W;
        for (w, chunk) in q.words.iter_mut().zip(idx.chunks_exact(per)) {
            let mut x = 0u64;
            for (j, &k) in chunk.iter().enumerate() {
                x |= u64::from(k) << (j as u32 * B);
            }
            *w = x;
        }
        q
    }
    match palette.len() {
        0 => Section::Uniform(0),
        1 => Section::Uniform(palette[0]),
        2 => Section::Bits1(mk(palette, idx)),
        3..=4 => Section::Bits2(mk(palette, idx)),
        5..=16 => Section::Bits4(mk(palette, idx)),
        _ => Section::Bits8(mk(palette, idx)),
    }
}

/// Unpacks `sb`-bit big-endian wire indices into `out` (values above 255 are truncated: only
/// used for widths up to 8).
fn unpack_indices(data: &[u8], sb: u32, out: &mut [u8; VOLUME]) {
    let per = (64 / sb) as usize;
    let mask = (1u64 << sb) - 1;
    for (chunk, b) in out.chunks_mut(per).zip(data.as_chunks::<8>().0) {
        let mut x = u64::from_be_bytes(*b);
        for o in chunk {
            *o = (x & mask) as u8;
            x >>= sb;
        }
    }
}

/// Why a paletted container was rejected.
pub(crate) type BuildError = &'static str;

/// Builds sections from wire containers; keeps a scratch table for direct (global) data.
pub(crate) struct Builder {
    /// `state -> new palette index + 1` (0 = not seen), sized to the state space.
    lookup: Vec<u16>,
}

/// One block container as read from the wire: storage bits, palette (validated raw states) and
/// the big-endian long array.
pub(crate) struct WireBlocks<'a> {
    /// Bits per entry in the long array (4..=8 for indirect, the global width for direct).
    pub storage_bits: u32,
    /// Indirect palette (`None` for direct data).
    pub palette: Option<&'a [u16]>,
    /// Big-endian longs (`ceil(4096 / (64 / storage_bits))` of them).
    pub data: &'a [u8],
}

impl Builder {
    pub(crate) fn new(state_space: usize) -> Self {
        Builder {
            lookup: vec![0; state_space],
        }
    }

    /// Builds a section from an indirect or direct wire container.
    pub(crate) fn build(&mut self, w: &WireBlocks<'_>) -> Result<Section, BuildError> {
        let sb = w.storage_bits;
        let per = (64 / sb) as usize;
        let mask = (1u64 << sb) - 1;
        let words = || {
            w.data
                .as_chunks::<8>()
                .0
                .iter()
                .map(|b| u64::from_be_bytes(*b))
        };
        match w.palette {
            Some(pal) => {
                let len = pal.len();
                if sb == 4 && len >= 5 {
                    // The common case: the wire packing is the target packing. Copy, check the
                    // indices against the palette (SWAR: one add per nibble lane), and notice a
                    // section whose blocks all use one entry.
                    let mut q = Packed4::zeroed();
                    let lo = 0x0F0F_0F0F_0F0F_0F0Fu64;
                    let k = (0x80 - len as u64) * 0x0101_0101_0101_0101;
                    let mut bad = 0u64;
                    for (dst, word) in q.words.iter_mut().zip(words()) {
                        *dst = word;
                        bad |= ((word & lo) + k) | (((word >> 4) & lo) + k);
                    }
                    if len < 16 && bad & 0x8080_8080_8080_8080 != 0 {
                        return Err("palette index past the palette");
                    }
                    let first = q.words[0] & 15;
                    if q.words.iter().all(|&x| x == first * 0x1111_1111_1111_1111) {
                        return Ok(Section::Uniform(pal[first as usize]));
                    }
                    q.palette[..len].copy_from_slice(pal);
                    q.len = len as u16;
                    return Ok(Section::Bits4(q));
                }
                if sb == 8 && len > 16 {
                    let max = w.data[..VOLUME].iter().copied().max().unwrap_or(0);
                    if usize::from(max) >= len {
                        return Err("palette index past the palette");
                    }
                    let mut q = Packed8::zeroed();
                    for (dst, word) in q.words.iter_mut().zip(words()) {
                        *dst = word;
                    }
                    q.palette[..len].copy_from_slice(pal);
                    q.len = len as u16;
                    return Ok(Section::Bits8(q));
                }
                if sb == 4 {
                    // Small palette: a section whose blocks all use one entry is common (flat
                    // layers, and palettes the server never shrinks after edits).
                    let first = u64::from_be_bytes(w.data[..8].try_into().unwrap_or([0; 8])) & 15;
                    let pat = first * 0x1111_1111_1111_1111;
                    if (first as usize) < len && words().all(|x| x == pat) {
                        return Ok(Section::Uniform(pal[first as usize]));
                    }
                }
                // Small palettes and 5 - 7 bit storage: unpack, keep only the used entries
                // (duplicate palette ids merge too), pack at the smallest width.
                let mut idx = [0u8; VOLUME];
                unpack_indices(w.data, sb, &mut idx);
                let mut used = [false; 256];
                for &k in &idx {
                    used[usize::from(k)] = true;
                }
                if used[len..].iter().any(|&u| u) {
                    return Err("palette index past the palette");
                }
                let mut palette = [0u16; 256];
                let mut remap = [0u8; 256];
                let mut k = 0usize;
                for (e, &s) in pal.iter().enumerate() {
                    if !used[e] {
                        continue;
                    }
                    let slot = &mut self.lookup[usize::from(s)];
                    if *slot == 0 {
                        palette[k] = s;
                        k += 1;
                        *slot = k as u16;
                    }
                    remap[e] = (*slot - 1) as u8;
                }
                for &s in &palette[..k] {
                    self.lookup[usize::from(s)] = 0;
                }
                for i in &mut idx {
                    *i = remap[usize::from(*i)];
                }
                Ok(build_from_buffer(&palette[..k], &idx))
            }
            None => {
                // Global ids: build a palette of the distinct states.
                let space = self.lookup.len();
                let mut states = Box::new([0u16; VOLUME]);
                let mut i = 0usize;
                'outer: for word in words() {
                    let mut x = word;
                    for _ in 0..per {
                        if i == VOLUME {
                            break 'outer;
                        }
                        let v = (x & mask) as usize;
                        if v >= space {
                            return Err("global palette id is not a block state");
                        }
                        states[i] = v as u16;
                        x >>= sb;
                        i += 1;
                    }
                }
                let mut palette = Vec::new();
                let mut idx = Box::new([0u16; VOLUME]);
                for (dst, &s) in idx.iter_mut().zip(states.iter()) {
                    let slot = &mut self.lookup[usize::from(s)];
                    if *slot == 0 {
                        palette.push(s);
                        *slot = palette.len() as u16;
                    }
                    *dst = *slot - 1;
                }
                for &s in &palette {
                    self.lookup[usize::from(s)] = 0;
                }
                if palette.len() > 256 {
                    return Ok(Section::Direct(states));
                }
                let mut small = [0u8; VOLUME];
                for (d, &k) in small.iter_mut().zip(idx.iter()) {
                    *d = k as u8;
                }
                Ok(build_from_buffer(&palette, &small))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be(words: &[u64]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    fn pack(bits: u32, vals: &[u32]) -> Vec<u8> {
        be(&crate::packed::pack(bits, vals))
    }

    fn check(s: &Section, expect: &[u16]) {
        for (i, &e) in expect.iter().enumerate() {
            assert_eq!(s.get(i), e, "index {i} in {s:?}");
        }
        let mut n = 0;
        s.for_each(|i, v| {
            assert_eq!(v, expect[i]);
            n += 1;
        });
        assert_eq!(n, VOLUME);
    }

    #[test]
    fn widths_follow_distinct_states() {
        let mut b = Builder::new(30000);
        for (distinct, bits) in [
            (1, 0),
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 4),
            (16, 4),
            (17, 8),
            (256, 8),
        ] {
            let sb = if distinct <= 16 { 4 } else { 8 };
            let pal: Vec<u16> = (0..distinct as u16).map(|k| k * 7 + 3).collect();
            let idx: Vec<u32> = (0..VOLUME as u32)
                .map(|i| (i.wrapping_mul(2_654_435_761) >> 7) % distinct)
                .collect();
            let idx: Vec<u32> = idx
                .iter()
                .enumerate()
                .map(|(i, &k)| if i < distinct as usize { i as u32 } else { k })
                .collect();
            let data = pack(sb, &idx);
            let s = b
                .build(&WireBlocks {
                    storage_bits: sb,
                    palette: Some(&pal),
                    data: &data,
                })
                .unwrap();
            assert_eq!(s.bits(), bits, "{distinct}");
            let expect: Vec<u16> = idx.iter().map(|&k| pal[k as usize]).collect();
            check(&s, &expect);
        }
        assert!(b.lookup.iter().all(|&x| x == 0));
    }

    #[test]
    fn unused_entries_and_errors() {
        let mut b = Builder::new(100);
        // palette of 10, only entries 3 and 7 used: kept as sent (no scan for 5+ entries)
        let pal: Vec<u16> = (10..20).collect();
        let idx: Vec<u32> = (0..VOLUME as u32)
            .map(|i| if i % 3 == 0 { 3 } else { 7 })
            .collect();
        let s = b
            .build(&WireBlocks {
                storage_bits: 4,
                palette: Some(&pal),
                data: &pack(4, &idx),
            })
            .unwrap();
        assert_eq!(s.bits(), 4);
        check(
            &s,
            &idx.iter().map(|&k| pal[k as usize]).collect::<Vec<_>>(),
        );
        // palette of 3 with 2 used -> 1 bit; palette of 6 all blocks one entry -> uniform
        let pal3: Vec<u16> = vec![5, 6, 7];
        let idx: Vec<u32> = (0..VOLUME as u32)
            .map(|i| if i % 5 == 0 { 0 } else { 2 })
            .collect();
        let s = b
            .build(&WireBlocks {
                storage_bits: 4,
                palette: Some(&pal3),
                data: &pack(4, &idx),
            })
            .unwrap();
        assert_eq!(s.bits(), 1);
        check(
            &s,
            &idx.iter().map(|&k| pal3[k as usize]).collect::<Vec<_>>(),
        );
        let s = b
            .build(&WireBlocks {
                storage_bits: 4,
                palette: Some(&pal[..6]),
                data: &pack(4, &vec![4; VOLUME]),
            })
            .unwrap();
        assert!(matches!(s, Section::Uniform(14)), "{s:?}");
        // index past the palette
        let idx = vec![12u32; VOLUME];
        assert!(
            b.build(&WireBlocks {
                storage_bits: 4,
                palette: Some(&pal),
                data: &pack(4, &idx)
            })
            .is_err()
        );
        // direct data with an id outside the state space
        let idx = vec![150u32; VOLUME];
        assert!(
            b.build(&WireBlocks {
                storage_bits: 15,
                palette: None,
                data: &pack(15, &idx)
            })
            .is_err()
        );
        // direct data, 300 distinct -> Direct; 3 distinct -> Bits2
        let idx: Vec<u32> = (0..VOLUME as u32).map(|i| i % 3).collect();
        let s = b
            .build(&WireBlocks {
                storage_bits: 15,
                palette: None,
                data: &pack(15, &idx),
            })
            .unwrap();
        assert_eq!(s.bits(), 2);
        let mut b = Builder::new(1000);
        let idx: Vec<u32> = (0..VOLUME as u32).map(|i| i % 300).collect();
        let s = b
            .build(&WireBlocks {
                storage_bits: 15,
                palette: None,
                data: &pack(15, &idx),
            })
            .unwrap();
        assert_eq!(s.bits(), 16);
        check(&s, &idx.iter().map(|&k| k as u16).collect::<Vec<_>>());
    }

    #[test]
    fn set_widens_and_compacts() {
        let mut s = Section::Uniform(1);
        let mut expect = vec![1u16; VOLUME];
        for (i, e) in expect.iter_mut().enumerate() {
            let v = (i % 400) as u16;
            *e = v;
            assert_eq!(s.set(i, v), 1);
        }
        assert_eq!(s.bits(), 16);
        check(&s, &expect);
        // compaction keeps a 1-bit section when states cycle through one block
        let mut s = Section::Uniform(1);
        for v in 2..1000u16 {
            s.set(0, v);
        }
        assert!(s.bits() <= 2, "{s:?}");
        assert_eq!((s.get(0), s.get(1)), (999, 1));
        // back to uniform content (still packed, reads right)
        s.set(0, 1);
        assert_eq!(s.get(0), 1);
    }
}
