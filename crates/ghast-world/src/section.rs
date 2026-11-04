//! One 16×16×16 chunk section of raw block states.
//!
//! Block states are stored as raw `u16` values (1.8: `id << 4 | meta`, 1.16.5: global palette
//! id), already mapped the way vanilla reads them (unknown values are air), so a lookup never
//! needs to validate anything.
//!
//! Three representations, chosen per section when it is decoded and changed on demand:
//!
//! | variant | memory | lookup |
//! |---|---|---|
//! | [`Section::Uniform`] (all blocks the same non-air state) | 0 heap bytes | 1 match |
//! | indexed (≤ 256 distinct states): `u8` index per block + 256-entry palette | 4.5 KiB | 2 loads |
//! | direct (> 256 distinct states): `u16` per block | 8 KiB | 1 load |
//!
//! All-air sections are not stored at all (the column keeps `None`), which is what vanilla does
//! for sections the server did not send.

/// Number of blocks in a section.
pub const SECTION_VOLUME: usize = 4096;

/// Index of a block inside its section: `y << 8 | z << 4 | x` on the low four bits of each
/// coordinate (vanilla's order in both 1.8 and 1.16).
#[inline]
#[must_use]
pub const fn block_index(x: i32, y: i32, z: i32) -> usize {
    (((y & 15) << 8) | ((z & 15) << 4) | (x & 15)) as usize
}

/// Indexed storage: one byte per block into a palette of at most 256 states.
#[derive(Clone)]
pub struct Indexed {
    /// Palette index per block (always `< len`).
    idx: [u8; SECTION_VOLUME],
    /// Palette. Entries `len..` are unused (0).
    palette: [u16; 256],
    /// Used palette length (`1..=256`).
    len: u16,
    /// Number of blocks whose state is not raw 0.
    non_air: u16,
}

/// Direct storage: the raw state of every block.
#[derive(Clone)]
pub struct Direct {
    states: [u16; SECTION_VOLUME],
    /// Number of blocks whose state is not raw 0.
    non_air: u16,
}

/// A non-empty chunk section (see the module docs).
#[derive(Clone)]
pub enum Section {
    /// Every block has this state (never raw 0: an all-air section is not stored).
    Uniform(u16),
    /// At most 256 distinct states.
    Indexed(Box<Indexed>),
    /// More than 256 distinct states.
    Direct(Box<Direct>),
}

impl std::fmt::Debug for Section {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Section::Uniform(s) => f.debug_tuple("Uniform").field(s).finish(),
            Section::Indexed(b) => f
                .debug_struct("Indexed")
                .field("palette", &&b.palette[..usize::from(b.len)])
                .field("non_air", &b.non_air)
                .finish(),
            Section::Direct(b) => f
                .debug_struct("Direct")
                .field("non_air", &b.non_air)
                .finish(),
        }
    }
}

impl Section {
    /// Raw state of the block at section index `i` (see [`block_index`]).
    #[inline]
    #[must_use]
    pub fn get(&self, i: usize) -> u16 {
        let i = i & (SECTION_VOLUME - 1);
        match self {
            Section::Uniform(s) => *s,
            Section::Indexed(b) => b.palette[usize::from(b.idx[i])],
            Section::Direct(b) => b.states[i],
        }
    }

    /// Number of blocks that are not raw state 0.
    #[must_use]
    pub fn non_air(&self) -> u16 {
        match self {
            Section::Uniform(_) => SECTION_VOLUME as u16,
            Section::Indexed(b) => b.non_air,
            Section::Direct(b) => b.non_air,
        }
    }

    /// Heap bytes owned by this section.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        match self {
            Section::Uniform(_) => 0,
            Section::Indexed(_) => size_of::<Indexed>(),
            Section::Direct(_) => size_of::<Direct>(),
        }
    }

    /// Calls `f(index, raw_state)` for every block, in index order.
    pub fn for_each(&self, mut f: impl FnMut(usize, u16)) {
        match self {
            Section::Uniform(s) => (0..SECTION_VOLUME).for_each(|i| f(i, *s)),
            Section::Indexed(b) => {
                for (i, &p) in b.idx.iter().enumerate() {
                    f(i, b.palette[usize::from(p)]);
                }
            }
            Section::Direct(b) => {
                for (i, &s) in b.states.iter().enumerate() {
                    f(i, s);
                }
            }
        }
    }

    /// Sets the block at index `i` to `state`. Returns `false` when the section became all air
    /// (the caller then drops it).
    pub fn set(&mut self, i: usize, state: u16) -> bool {
        let i = i & (SECTION_VOLUME - 1);
        match self {
            Section::Uniform(u) => {
                if *u == state {
                    return true;
                }
                let mut b = Box::new(Indexed {
                    idx: [0; SECTION_VOLUME],
                    palette: [0; 256],
                    len: 2,
                    non_air: SECTION_VOLUME as u16,
                });
                b.palette[0] = *u;
                b.palette[1] = state;
                b.idx[i] = 1;
                if state == 0 {
                    b.non_air -= 1;
                }
                *self = Section::Indexed(b);
                true
            }
            Section::Indexed(b) => {
                let old = b.palette[usize::from(b.idx[i])];
                if old == state {
                    return true;
                }
                let p = match b.find_or_insert(state) {
                    Some(p) => p,
                    None => {
                        // Palette full: go direct.
                        let mut d = Box::new(Direct {
                            states: [0; SECTION_VOLUME],
                            non_air: b.non_air,
                        });
                        for (dst, &p) in d.states.iter_mut().zip(b.idx.iter()) {
                            *dst = b.palette[usize::from(p)];
                        }
                        *self = Section::Direct(d);
                        return self.set(i, state);
                    }
                };
                b.idx[i] = p;
                adjust(&mut b.non_air, old, state);
                b.non_air != 0
            }
            Section::Direct(b) => {
                let old = b.states[i];
                b.states[i] = state;
                adjust(&mut b.non_air, old, state);
                b.non_air != 0
            }
        }
    }

    /// A section with every block air except `state` at index `i`; `None` if `state` is air.
    #[must_use]
    pub fn single(i: usize, state: u16) -> Option<Section> {
        if state == 0 {
            return None;
        }
        let mut b = Box::new(Indexed {
            idx: [0; SECTION_VOLUME],
            palette: [0; 256],
            len: 2,
            non_air: 1,
        });
        b.palette[1] = state;
        b.idx[i & (SECTION_VOLUME - 1)] = 1;
        Some(Section::Indexed(b))
    }
}

#[inline]
fn adjust(non_air: &mut u16, old: u16, new: u16) {
    match (old == 0, new == 0) {
        (true, false) => *non_air += 1,
        (false, true) => *non_air -= 1,
        _ => {}
    }
}

impl Indexed {
    /// Palette index of `state`, appending it if needed. When the palette is full, unused
    /// entries are compacted away first; `None` if it is still full.
    fn find_or_insert(&mut self, state: u16) -> Option<u8> {
        let len = usize::from(self.len);
        if let Some(p) = self.palette[..len].iter().position(|&s| s == state) {
            return Some(p as u8);
        }
        if len == 256 {
            self.compact();
        }
        let len = usize::from(self.len);
        if len == 256 {
            return None;
        }
        self.palette[len] = state;
        self.len += 1;
        Some(len as u8)
    }

    /// Drops palette entries no block uses.
    fn compact(&mut self) {
        let mut used = [false; 256];
        for &p in &self.idx {
            used[usize::from(p)] = true;
        }
        let mut remap = [0u8; 256];
        let mut palette = [0u16; 256];
        let mut n = 0usize;
        for p in 0..usize::from(self.len) {
            if used[p] {
                remap[p] = n as u8;
                palette[n] = self.palette[p];
                n += 1;
            }
        }
        for p in &mut self.idx {
            *p = remap[usize::from(*p)];
        }
        self.palette = palette;
        self.len = n as u16;
    }
}

/// Builds sections from decoded raw states (shared by every chunk format).
///
/// The builder keeps a reusable scratch table so decoding allocates only the section itself.
#[cfg_attr(not(any(feature = "v1_8", feature = "v1_16_5")), allow(dead_code))]
pub(crate) struct SectionBuilder {
    /// `state -> palette index + 1` (0 = not in the palette yet), sized for the largest state
    /// space handed to [`SectionBuilder::push_states`].
    lookup: Vec<u16>,
}

#[cfg_attr(not(any(feature = "v1_8", feature = "v1_16_5")), allow(dead_code))]
impl SectionBuilder {
    pub(crate) fn new(state_space: usize) -> Self {
        SectionBuilder {
            lookup: vec![0; state_space],
        }
    }

    /// Builds a section from 4096 raw states (already mapped the way vanilla reads them;
    /// values outside the state space are treated as air).
    pub(crate) fn build(&mut self, states: &[u16; SECTION_VOLUME]) -> Option<Section> {
        let space = self.lookup.len();
        let mut non_air = 0u16;
        for &s in states {
            non_air += u16::from(s != 0);
        }
        if non_air == 0 {
            return None;
        }
        let mut ix = Box::new(Indexed {
            idx: [0; SECTION_VOLUME],
            palette: [0; 256],
            len: 0,
            non_air,
        });
        let mut len = 0usize;
        let mut overflow = false;
        for (dst, &s) in ix.idx.iter_mut().zip(states.iter()) {
            let s = if usize::from(s) < space { s } else { 0 };
            let slot = &mut self.lookup[usize::from(s)];
            if *slot == 0 {
                if len == 256 {
                    overflow = true;
                    break;
                }
                ix.palette[len] = s;
                len += 1;
                *slot = len as u16;
            }
            *dst = (*slot - 1) as u8;
        }
        self.reset(&ix.palette[..len]);
        if overflow {
            let mut d = Box::new(Direct {
                states: *states,
                non_air: 0,
            });
            let mut n = 0u16;
            for s in &mut d.states {
                if usize::from(*s) >= space {
                    *s = 0;
                }
                n += u16::from(*s != 0);
            }
            d.non_air = n;
            return (n != 0).then_some(Section::Direct(d));
        }
        if len == 1 {
            return Some(Section::Uniform(ix.palette[0]));
        }
        // Out-of-space values were mapped to air: recount.
        if ix.palette[..len].contains(&0) {
            let air = ix.palette[..len].iter().position(|&s| s == 0).unwrap_or(0) as u8;
            ix.non_air = ix.idx.iter().filter(|&&p| p != air).count() as u16;
            if ix.non_air == 0 {
                return None;
            }
        }
        ix.len = len as u16;
        Some(Section::Indexed(ix))
    }

    /// Builds a section from 1.16-style packed palette indices: `bits` (`4..=8`) per value in
    /// big-endian words `data` (at least `words_needed(bits, 4096)` words), and `remap` = the
    /// wire palette already mapped to raw states. Indices past the palette read as air.
    #[cfg_attr(not(feature = "v1_16_5"), allow(dead_code))]
    pub(crate) fn build_packed(
        &mut self,
        remap: &[u16],
        bits: u32,
        data: &[u8],
    ) -> Option<Section> {
        debug_assert!((4..=8).contains(&bits));
        let reachable = 1usize << bits;
        // Deduplicate the reachable palette entries (plus air for indices past the palette).
        let mut palette = [0u16; 256];
        let mut len = 0usize;
        let mut to_palette = [0u8; 256];
        let space = self.lookup.len();
        for (w, slot_out) in to_palette.iter_mut().enumerate().take(reachable) {
            let s = remap.get(w).copied().unwrap_or(0);
            let s = if usize::from(s) < space { s } else { 0 };
            let slot = &mut self.lookup[usize::from(s)];
            if *slot == 0 {
                palette[len] = s;
                len += 1;
                *slot = len as u16;
            }
            *slot_out = (*slot - 1) as u8;
            if w >= remap.len() {
                // Every further index maps to air too.
                let air = *slot_out;
                to_palette[w..reachable].fill(air);
                break;
            }
        }
        self.reset(&palette[..len]);

        let mut ix = Box::new(Indexed {
            idx: [0; SECTION_VOLUME],
            palette: [0; 256],
            len: 0,
            non_air: 0,
        });
        let mut hist = [0u16; 256];
        crate::packed::unpack_be(bits, data, SECTION_VOLUME, |i, v| {
            let p = to_palette[(v & 255) as usize];
            ix.idx[i] = p;
            hist[usize::from(p)] += 1;
        });
        // Keep only used palette entries.
        let mut remap2 = [0u8; 256];
        let mut n = 0usize;
        let mut non_air = 0u16;
        for p in 0..len {
            if hist[p] != 0 {
                remap2[p] = n as u8;
                ix.palette[n] = palette[p];
                if palette[p] != 0 {
                    non_air += hist[p];
                }
                n += 1;
            }
        }
        if non_air == 0 {
            return None;
        }
        if n == 1 {
            return Some(Section::Uniform(ix.palette[0]));
        }
        if n != len {
            for p in &mut ix.idx {
                *p = remap2[usize::from(*p)];
            }
        }
        ix.len = n as u16;
        ix.non_air = non_air;
        Some(Section::Indexed(ix))
    }

    fn reset(&mut self, palette: &[u16]) {
        for &s in palette {
            self.lookup[usize::from(s)] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_eq(s: &Option<Section>, expect: &[u16]) {
        for (i, &e) in expect.iter().enumerate() {
            let got = s.as_ref().map_or(0, |s| s.get(i));
            assert_eq!(got, e, "index {i}");
        }
        let non_air = expect.iter().filter(|&&v| v != 0).count();
        assert_eq!(s.as_ref().map_or(0, |s| usize::from(s.non_air())), non_air);
    }

    #[test]
    fn build_variants() {
        let mut b = SectionBuilder::new(4096);
        assert!(b.build(&[0; 4096]).is_none());
        assert!(matches!(b.build(&[16; 4096]), Some(Section::Uniform(16))));
        let v: Vec<u16> = (0..4096).map(|i| (i % 7) as u16 * 16).collect();
        let s = b.build(v.as_slice().try_into().unwrap());
        assert!(matches!(s, Some(Section::Indexed(_))));
        check_eq(&s, &v);
        let v: Vec<u16> = (0..4096).map(|i| (i % 300) as u16).collect();
        let s = b.build(v.as_slice().try_into().unwrap());
        assert!(matches!(s, Some(Section::Direct(_))));
        check_eq(&s, &v);
        // Scratch table is clean afterwards.
        assert!(b.lookup.iter().all(|&x| x == 0));
    }

    #[test]
    fn set_transitions() {
        let mut b = SectionBuilder::new(4096);
        let mut expect = vec![1u16; 4096];
        let mut s = b.build(&[1; 4096]);
        // Uniform -> indexed -> direct -> all air.
        for (i, e) in expect.iter_mut().enumerate() {
            let v = (i % 400) as u16;
            *e = v;
            let alive = s.as_mut().unwrap().set(i, v);
            assert!(alive);
        }
        check_eq(&s, &expect);
        assert!(matches!(s, Some(Section::Direct(_))));
        for i in 0..4096usize {
            let alive = s.as_mut().unwrap().set(i, 0);
            assert_eq!(alive, i != 4095);
        }
    }

    #[test]
    fn palette_compaction_keeps_indexed() {
        let mut b = SectionBuilder::new(4096);
        let mut s = b.build(&[1; 4096]).unwrap();
        // Cycle 1000 distinct states through one block: stale palette entries get compacted.
        for v in 2..1000u16 {
            s.set(0, v);
        }
        assert!(matches!(s, Section::Indexed(_)), "{s:?}");
        assert_eq!(s.get(0), 999);
        assert_eq!(s.get(1), 1);
    }

    #[test]
    fn single() {
        let s = Section::single(block_index(1, 2, 3), 5).unwrap();
        assert_eq!(s.get(block_index(1, 2, 3)), 5);
        assert_eq!(s.get(0), 0);
        assert_eq!(s.non_air(), 1);
        assert!(Section::single(0, 0).is_none());
    }
}
