//! [`SharedChunks`]: one copy of identical chunk sections for many worlds in one process.

use std::sync::Arc;

#[cfg(any(feature = "v26_1", feature = "v26_2"))]
use crate::paletted::{Section, WeakSection};
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
use std::collections::HashMap;
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
use std::hash::{BuildHasher, BuildHasherDefault, Hasher, RandomState};
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
use std::sync::{Mutex, MutexGuard, PoisonError, Weak};

/// A process-wide store that lets the worlds of many bots hold **one** copy of each identical
/// chunk section (16×16×16 blocks) instead of one copy per bot.
///
/// Bots standing in the same area receive the same chunks, and at view distance 10 a world's
/// section storage is almost all of a bot's memory. Give the same handle to every
/// [`Client`](https://docs.rs/ghast-client/latest/ghast_client/struct.Client.html) (its
/// `shared_chunks` option) or to every world ([`v26_1::World::set_shared_chunks`]) and the
/// section storage is held once per distinct section.
///
/// - **Opt-in.** A world without a handle stores its sections exactly as before.
/// - **Per section, copy on write.** A shared section is immutable and reference counted. A
///   block change (a server block update or a local [`set_block_state`]) first gives that
///   world its own copy of that one section, so no world ever sees another world's packets.
///   Copies never merge back (a section that keeps changing ends up one copy per world, which
///   is the cost without sharing, never more).
/// - **Reads do not change.** A block read is the same match and the same single pointer hop
///   as an unshared section: no lock, no atomic, no extra indirection. The handle's lock is
///   taken only while a chunk packet is decoded (once per non-uniform section) and when the
///   last world holding a section drops it.
/// - **Decoded once.** A section's key is a 128-bit keyed hash of its exact bytes in the chunk
///   packet, found with a cheap skip pass; on a hit the section is not decoded at all.
/// - **Freed with the last holder.** The store holds weak references only: when the last
///   world unloads a section (chunk unload, a later full chunk for the same column, a
///   dimension change, a dropped world, a copy on write) its memory is freed and its entry is
///   removed at once.
///
/// What is shared: the block states of non-uniform sections. Uniform sections (all air, all
/// stone) take no heap memory and are kept inline as before. Biomes (off by default in the
/// client, at most 128 bytes per section) and block entities stay per world; the modern
/// worlds do not keep light. Only the modern worlds (26.1, 26.2) use the store: the 1.8.9,
/// 1.12.2 and 1.16.5 worlds ignore it (their sections are byte-per-block and are not shared).
/// Worlds of different protocols can share one handle; their sections never mix.
///
/// The handle is cheap to clone (`Arc`) and `Send + Sync`: bots on different threads (for
/// example on the workers of a `Swarm`) can share it.
///
/// [`v26_1::World::set_shared_chunks`]: crate::v26_1::World::set_shared_chunks
/// [`set_block_state`]: crate::v26_1::World::set_block_state
///
/// ```
/// let cache = ghast_world::SharedChunks::new();
/// let for_second_bot = cache.clone();
/// assert_eq!(cache.sections(), 0);
/// # drop(for_second_bot);
/// ```
#[derive(Clone, Default)]
pub struct SharedChunks {
    inner: Arc<Inner>,
}

/// Collision safety of the key: SipHash-1-3 with its 128-bit output (the keyed
/// pseudo-random function std's `HashMap` uses, in the output width of rustc's `SipHasher128`)
/// over the section's exact bytes, under a secret 128-bit key drawn per store from std's
/// `RandomState` (OS randomness); the protocol number is folded into the key, so different
/// protocols never share. Accidental collisions: about n² / 2¹²⁹ for n distinct sections (below
/// 10⁻²⁶ for a million). A server cannot aim for a collision either: it does not know the key.
#[cfg_attr(not(any(feature = "v26_1", feature = "v26_2")), derive(Default))]
pub(crate) struct Inner {
    #[cfg(any(feature = "v26_1", feature = "v26_2"))]
    key: [u64; 2],
    #[cfg(any(feature = "v26_1", feature = "v26_2"))]
    map: Mutex<HashMap<u128, WeakSection, BuildHasherDefault<KeyHasher>>>,
}

#[cfg(any(feature = "v26_1", feature = "v26_2"))]
impl Default for Inner {
    fn default() -> Self {
        let rs = RandomState::new();
        Inner {
            key: [rs.hash_one(0x6b30_u64), rs.hash_one(0x6b31_u64)],
            map: Mutex::default(),
        }
    }
}

/// SipHash-c-d (Aumasson and Bernstein's reference algorithm), 128-bit output when `WIDE`,
/// else the 64-bit one (in the low half).
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
fn siphash<const C: usize, const D: usize, const WIDE: bool>(k0: u64, k1: u64, m: &[u8]) -> u128 {
    #[inline(always)]
    fn round(v: &mut [u64; 4]) {
        v[0] = v[0].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(13) ^ v[0];
        v[0] = v[0].rotate_left(32);
        v[2] = v[2].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(16) ^ v[2];
        v[0] = v[0].wrapping_add(v[3]);
        v[3] = v[3].rotate_left(21) ^ v[0];
        v[2] = v[2].wrapping_add(v[1]);
        v[1] = v[1].rotate_left(17) ^ v[2];
        v[2] = v[2].rotate_left(32);
    }
    let mut v = [
        k0 ^ 0x736f_6d65_7073_6575,
        k1 ^ 0x646f_7261_6e64_6f6d ^ if WIDE { 0xee } else { 0 },
        k0 ^ 0x6c79_6765_6e65_7261,
        k1 ^ 0x7465_6462_7974_6573,
    ];
    let mut compress = |w: u64| {
        v[3] ^= w;
        for _ in 0..C {
            round(&mut v);
        }
        v[0] ^= w;
    };
    let (words, tail) = m.as_chunks::<8>();
    for w in words {
        compress(u64::from_le_bytes(*w));
    }
    let mut last = (m.len() as u64) << 56;
    for (i, &b) in tail.iter().enumerate() {
        last |= u64::from(b) << (8 * i);
    }
    compress(last);
    v[2] ^= if WIDE { 0xee } else { 0xff };
    for _ in 0..D {
        round(&mut v);
    }
    let lo = v[0] ^ v[1] ^ v[2] ^ v[3];
    if !WIDE {
        return u128::from(lo);
    }
    v[1] ^= 0xdd;
    for _ in 0..D {
        round(&mut v);
    }
    let hi = v[0] ^ v[1] ^ v[2] ^ v[3];
    (u128::from(hi) << 64) | u128::from(lo)
}

/// The keys are already uniformly random: use their low bits.
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
#[derive(Default)]
pub(crate) struct KeyHasher(u64);

#[cfg(any(feature = "v26_1", feature = "v26_2"))]
impl Hasher for KeyHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(b);
        }
    }

    #[inline]
    fn write_u128(&mut self, v: u128) {
        self.0 = v as u64;
    }
}

impl std::fmt::Debug for SharedChunks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedChunks")
            .field("sections", &self.sections())
            .finish_non_exhaustive()
    }
}

impl SharedChunks {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        SharedChunks::default()
    }

    /// Number of distinct sections currently held by at least one world.
    #[must_use]
    pub fn sections(&self) -> usize {
        #[cfg(any(feature = "v26_1", feature = "v26_2"))]
        {
            self.lock().values().filter(|w| !w.is_dead()).count()
        }
        #[cfg(not(any(feature = "v26_1", feature = "v26_2")))]
        0
    }

    /// Heap bytes of the shared section storage (each distinct section counted once) plus the
    /// store's own table. A world's [`heap_bytes`](crate::v26_1::World::heap_bytes) does not
    /// count shared sections.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        #[cfg(any(feature = "v26_1", feature = "v26_2"))]
        {
            let m = self.lock();
            m.capacity() * (size_of::<(u128, WeakSection)>() + 1)
                + m.values()
                    .filter(|w| !w.is_dead())
                    .map(WeakSection::storage_bytes)
                    .sum::<usize>()
        }
        #[cfg(not(any(feature = "v26_1", feature = "v26_2")))]
        0
    }

    /// `true` when both handles are the same store.
    #[must_use]
    pub fn same_store(&self, other: &SharedChunks) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

#[cfg(any(feature = "v26_1", feature = "v26_2"))]
impl SharedChunks {
    fn lock(&self) -> MutexGuard<'_, HashMap<u128, WeakSection, BuildHasherDefault<KeyHasher>>> {
        self.inner
            .map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The key of a section's exact wire bytes for `protocol` (see [`Inner`]).
    pub(crate) fn key(&self, protocol: u16, bytes: &[u8]) -> u128 {
        let [k0, k1] = self.inner.key;
        siphash::<1, 3, true>(k0, k1 ^ (u64::from(protocol) << 48), bytes)
    }

    /// Number of entries, dead ones included (tests: dead entries must not linger).
    #[cfg(test)]
    pub(crate) fn entries(&self) -> usize {
        self.lock().len()
    }

    /// The live section stored under `key`.
    pub(crate) fn get(&self, key: u128) -> Option<Section> {
        // Upgrading under the lock is what makes the last-holder check in `forget` race-free.
        self.lock().get(&key).and_then(WeakSection::upgrade)
    }

    /// Stores a freshly decoded section under `key` and returns the shared section to keep. A
    /// uniform section is returned as it is (it owns no heap memory). When another thread
    /// stored the same key meanwhile, its section is returned and `section` is dropped.
    pub(crate) fn insert(&self, key: u128, section: Section) -> Section {
        let (shared, weak) = match section.into_shared(key, Arc::downgrade(&self.inner)) {
            Ok(s) => s,
            Err(uniform) => return uniform,
        };
        let existing = {
            let mut m = self.lock();
            match m.get(&key).and_then(WeakSection::upgrade) {
                Some(live) => Some(live),
                None => {
                    m.insert(key, weak);
                    None
                }
            }
        };
        // `shared` is dropped outside the lock (its last drop takes the lock).
        existing.unwrap_or(shared)
    }
}

/// Called by a shared section's storage when its last holder dropped it: removes the dead
/// entry (unless the key was stored again meanwhile).
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
pub(crate) fn forget(cache: &Weak<Inner>, key: u128) {
    let Some(inner) = cache.upgrade() else {
        return;
    };
    let mut m = inner.map.lock().unwrap_or_else(PoisonError::into_inner);
    if m.get(&key).is_some_and(WeakSection::is_dead) {
        m.remove(&key);
    }
    drop(m);
    drop(inner);
}

#[cfg(all(test, any(feature = "v26_1", feature = "v26_2")))]
mod tests {
    use super::siphash;

    /// The reference implementation's first SipHash-2-4-128 vector (key 00..0f, empty message).
    #[test]
    fn siphash_reference_vector() {
        let k0 = u64::from_le_bytes([0, 1, 2, 3, 4, 5, 6, 7]);
        let k1 = u64::from_le_bytes([8, 9, 10, 11, 12, 13, 14, 15]);
        let want: [u8; 16] = [
            0xa3, 0x81, 0x7f, 0x04, 0xba, 0x25, 0xa8, 0xe6, 0x6d, 0xf6, 0x72, 0x14, 0xc7, 0x55,
            0x02, 0x93,
        ];
        assert_eq!(siphash::<2, 4, true>(k0, k1, &[]).to_le_bytes(), want);
    }

    /// The same code in 64-bit mode is std's SipHash-2-4 on every length up to 70 (rounds,
    /// word order, the length / tail block).
    #[test]
    fn rounds_and_tail_match_std() {
        #[allow(deprecated)]
        fn std24(k0: u64, k1: u64, m: &[u8]) -> u64 {
            use std::hash::Hasher;
            let mut h = std::hash::SipHasher::new_with_keys(k0, k1);
            h.write(m);
            h.finish()
        }
        let m: Vec<u8> = (0..70u8).map(|i| i.wrapping_mul(37) ^ 0x5a).collect();
        for n in 0..=m.len() {
            let (k0, k1) = (0x0706_0504_0302_0100 ^ n as u64, 0x0f0e_0d0c_0b0a_0908);
            let ours = siphash::<2, 4, false>(k0, k1, &m[..n]);
            assert_eq!(ours, u128::from(std24(k0, k1, &m[..n])), "len {n}");
        }
        // One flipped bit, one byte less or another key: another key.
        let a = siphash::<1, 3, true>(1, 2, &m);
        assert_ne!(a, siphash::<1, 3, true>(1, 2, &m[..69]));
        assert_ne!(a, siphash::<1, 3, true>(1, 3, &m));
        let mut m2 = m.clone();
        m2[40] ^= 1;
        assert_ne!(a, siphash::<1, 3, true>(1, 2, &m2));
    }
}
