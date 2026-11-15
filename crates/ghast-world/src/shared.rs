//! [`SharedChunks`]: one copy of identical chunk sections for many worlds in one process.

use std::sync::Arc;

#[cfg(any(feature = "v26_1", feature = "v26_2"))]
use crate::paletted::{Ident, Section, VOLUME, WeakSection};
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
/// - **Per section, copy on write, and shared again.** A shared section is immutable and
///   reference counted. A block change (a server block update or a local
///   [`set_block_state`]) never touches storage another world can read: the changing world
///   moves to the section with the new content, which is looked up in the store first and
///   only copied when no world holds it yet. So bots that receive the same block updates keep
///   holding one copy, and no world ever sees another world's packets: every world reads
///   exactly the blocks its own packets produced.
/// - **Whole columns too.** The per-column array of section handles is shared the same way
///   between worlds whose columns hold the same sections.
/// - **Reads do not change.** A block read is the same match and the same pointer hops as an
///   unshared section: no lock, no atomic, no extra indirection. The handle's lock is taken
///   while a chunk packet is applied (once per column that is already in the store), for a
///   block change in a shared section, and when the last holder of a section drops it.
/// - **Decoded once.** A section's key is a 128-bit keyed hash of its exact bytes in the chunk
///   packet, found with a cheap skip pass; on a hit the section is not decoded at all.
/// - **Freed with the last holder.** The store holds weak references only: when the last
///   world unloads a section (chunk unload, a later full chunk for the same column, a
///   dimension change, a dropped world, a block change) its memory is freed and its entry is
///   removed at once.
///
/// What is shared: the block states of non-uniform sections and the columns' section arrays.
/// Uniform sections (all air, all stone) take no heap memory and are kept inline as before.
/// Biomes (off by default in the client, at most 128 bytes per section) and block entities
/// stay per world; the modern worlds do not keep light. Only the modern worlds (26.1, 26.2)
/// use the store: the 1.8.9, 1.12.2 and 1.16.5 worlds ignore it (their sections are
/// byte-per-block and are not shared). Worlds of different protocols can share one handle;
/// their sections never mix.
///
/// A section that changed is found again by its *content*: its key is the XOR, over the 4096
/// blocks, of a keyed 128-bit hash of (protocol, block index, state), so a block change moves
/// the key by two hashes and the order of the changes does not matter. A section a world
/// decoded without a store, or changed while it had none, stays that world's own.
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

/// Two kinds of keys name a section in the store, and a key always names one content:
///
/// - the **wire key** of a decoded section: SipHash-1-3 with its 128-bit output (the keyed
///   pseudo-random function std's `HashMap` uses, in the output width of rustc's
///   `SipHasher128`) over the section's exact bytes;
/// - the **content key** of a section made by block changes: the XOR over all 4096 blocks of
///   the same function of (block index, state), under a different key. Two different contents
///   differ in at least one block, so the XOR of their keys is the XOR of at least two outputs
///   of the function on distinct inputs: for a pseudo-random function that is 0 with
///   probability 2⁻¹²⁸, as for the wire key. It can be updated per block change, and it does
///   not depend on how the content is packed (which decides no read).
///
/// Both use a secret 128-bit key drawn per store from std's `RandomState` (OS randomness); the
/// protocol number is folded into the key, so different protocols never share. Accidental
/// collisions: about n² / 2¹²⁹ for n distinct sections (below 10⁻²⁶ for a million). A server
/// cannot aim for a collision either: it does not know the key.
///
/// The table of section arrays needs no such argument: its 64-bit hash only finds a candidate,
/// which is then compared handle by handle (uniform value or section key).
#[cfg_attr(not(any(feature = "v26_1", feature = "v26_2")), derive(Default))]
pub(crate) struct Inner {
    #[cfg(any(feature = "v26_1", feature = "v26_2"))]
    key: [u64; 2],
    #[cfg(any(feature = "v26_1", feature = "v26_2"))]
    tables: Mutex<Tables>,
}

/// What the store's lock guards. Nothing that can be the last handle to a section or to a
/// section array is ever dropped while the lock is held (the last drop of a section takes it).
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
#[derive(Default)]
pub(crate) struct Tables {
    /// Shared section storage by wire key or content key.
    sections: HashMap<u128, WeakSection, BuildHasherDefault<KeyHasher>>,
    /// Shared section arrays by the hash of their handles ([`SharedChunks::column_hash`]).
    columns: HashMap<u64, Weak<[Section]>, BuildHasherDefault<KeyHasher>>,
    /// `columns` is swept for dead entries when it reaches this size (arrays have no drop
    /// hook: worlds remove their entry when they let go of the last handle, and this sweep
    /// catches what that misses, such as two threads letting go at once).
    sweep_at: usize,
    /// Content keys of uniform sections by `protocol << 16 | state` (a small cache).
    uniform: HashMap<u32, u128, BuildHasherDefault<KeyHasher>>,
}

#[cfg(any(feature = "v26_1", feature = "v26_2"))]
impl Default for Inner {
    fn default() -> Self {
        let rs = RandomState::new();
        Inner {
            key: [rs.hash_one(0x6b30_u64), rs.hash_one(0x6b31_u64)],
            tables: Mutex::default(),
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

    #[inline]
    fn write_u64(&mut self, v: u64) {
        self.0 = v;
    }

    #[inline]
    fn write_u32(&mut self, v: u32) {
        self.0 = u64::from(v).wrapping_mul(0x9e37_79b9_7f4a_7c15);
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
            self.lock()
                .sections
                .values()
                .filter(|w| !w.is_dead())
                .count()
        }
        #[cfg(not(any(feature = "v26_1", feature = "v26_2")))]
        0
    }

    /// Number of distinct section arrays (one per column whose sections are all uniform or
    /// shared) currently held by at least one world.
    #[must_use]
    pub fn columns(&self) -> usize {
        #[cfg(any(feature = "v26_1", feature = "v26_2"))]
        {
            self.lock()
                .columns
                .values()
                .filter(|w| w.strong_count() > 0)
                .count()
        }
        #[cfg(not(any(feature = "v26_1", feature = "v26_2")))]
        0
    }

    /// Heap bytes of the shared section storage and the shared section arrays (each distinct
    /// one counted once) plus the store's own tables. A world's
    /// [`heap_bytes`](crate::v26_1::World::heap_bytes) does not count what it shares.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        #[cfg(any(feature = "v26_1", feature = "v26_2"))]
        {
            let t = self.lock();
            let sections = t.sections.capacity() * (size_of::<(u128, WeakSection)>() + 1)
                + t.sections
                    .values()
                    .filter(|w| !w.is_dead())
                    .map(WeakSection::storage_bytes)
                    .sum::<usize>();
            let columns = t.columns.capacity() * (size_of::<(u64, Weak<[Section]>)>() + 1)
                + t.columns
                    .values()
                    .filter(|w| w.strong_count() > 0)
                    .map(|w| 2 * size_of::<usize>() + w.as_ptr().len() * size_of::<Section>())
                    .sum::<usize>();
            sections + columns + t.uniform.capacity() * (size_of::<(u32, u128)>() + 1)
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

/// What a call collected to drop once the store's lock is released (see [`Tables`]).
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
#[derive(Default)]
struct Trash {
    sections: [Option<Section>; 2],
    arrays: [Option<Arc<[Section]>>; 2],
}

/// `true` when `entry` is the store entry of `arr`.
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
fn is_entry_of(entry: &Weak<[Section]>, arr: &Arc<[Section]>) -> bool {
    std::ptr::addr_eq(entry.as_ptr(), Arc::as_ptr(arr))
}

#[cfg(any(feature = "v26_1", feature = "v26_2"))]
impl Tables {
    /// Stores `arr` under `hash` unless a live array is there (then `arr` stays unshared).
    fn register_column(&mut self, hash: u64, arr: &Arc<[Section]>) {
        if self
            .columns
            .get(&hash)
            .is_some_and(|w| w.strong_count() > 0)
        {
            return;
        }
        if self.columns.len() >= self.sweep_at {
            self.columns.retain(|_, w| w.strong_count() > 0);
            self.sweep_at = (self.columns.len() * 2).max(64);
        }
        // A dead entry dropped here frees memory only (its sections went with the last handle).
        self.columns.insert(hash, Arc::downgrade(arr));
    }

    /// Removes the entry of `arr` (when `hash` is its hash and the entry is its own).
    fn unregister_column(&mut self, hash: Option<u64>, arr: &Arc<[Section]>) {
        if let Some(h) = hash
            && self.columns.get(&h).is_some_and(|w| is_entry_of(w, arr))
        {
            self.columns.remove(&h);
        }
    }
}

#[cfg(any(feature = "v26_1", feature = "v26_2"))]
impl SharedChunks {
    fn lock(&self) -> MutexGuard<'_, Tables> {
        self.inner
            .tables
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The address that identifies this store in its sections.
    fn id(&self) -> *const Inner {
        Arc::as_ptr(&self.inner)
    }

    /// The key of a section's exact wire bytes for `protocol` (see [`Inner`]).
    pub(crate) fn key(&self, protocol: u16, bytes: &[u8]) -> u128 {
        let [k0, k1] = self.inner.key;
        siphash::<1, 3, true>(k0, k1 ^ (u64::from(protocol) << 48), bytes)
    }

    /// One block's share of a content key (see [`Inner`]).
    fn term(&self, protocol: u16, i: usize, state: u16) -> u128 {
        let [k0, k1] = self.inner.key;
        let m = ((i as u64) << 16) | u64::from(state);
        siphash::<1, 3, true>(
            k0 ^ 0x636f_6e74_656e_7421,
            k1 ^ (u64::from(protocol) << 48),
            &m.to_le_bytes(),
        )
    }

    /// The content key of a section of this store: `None` for owned storage and for storage
    /// shared through another store (such sections are not this store's to replace).
    fn content_key(&self, protocol: u16, section: &Section) -> Option<u128> {
        if let Section::Uniform(u) = *section {
            let id = (u32::from(protocol) << 16) | u32::from(u);
            if let Some(&k) = self.lock().uniform.get(&id) {
                return Some(k);
            }
            let mut k = 0;
            for i in 0..VOLUME {
                k ^= self.term(protocol, i, u);
            }
            let mut t = self.lock();
            if t.uniform.len() < 1024 {
                t.uniform.insert(id, k);
            }
            return Some(k);
        }
        let cell = section.content_cell(self.id())?;
        Some(*cell.get_or_init(|| {
            let mut k = 0;
            section.for_each(|i, s| k ^= self.term(protocol, i, s));
            k
        }))
    }

    /// Hash of a section array's handles for the table of arrays (a candidate is always
    /// compared handle by handle, so this needs no collision resistance; the secret key keeps
    /// a server from crowding one bucket).
    fn column_hash(&self, idents: impl Iterator<Item = Ident>) -> u64 {
        let mut h = self.inner.key[1];
        for id in idents {
            h = self.hash_step(h, id);
        }
        h ^ (h >> 31)
    }

    /// One handle's step of [`SharedChunks::column_hash`].
    #[inline]
    fn hash_step(&self, h: u64, ident: Ident) -> u64 {
        let v = match ident {
            Ident::Uniform(u) => u64::from(u) ^ 0x756e_6966_6f72_6d00,
            Ident::Keyed(k) => k as u64,
        };
        (h.rotate_left(27) ^ v).wrapping_mul(self.inner.key[0] | 1)
    }

    /// The hashes of `arr` as it is and with `new` at index `j`, in one pass; `None` when a
    /// section in it (other than `j`) is not this store's.
    fn hashes_with(&self, arr: &[Section], j: usize, new: Ident) -> Option<(u64, u64)> {
        let id = self.id();
        let (mut old_h, mut new_h) = (self.inner.key[1], self.inner.key[1]);
        for (k, s) in arr.iter().enumerate() {
            let ident = s.ident(id)?;
            old_h = self.hash_step(old_h, ident);
            new_h = self.hash_step(new_h, if k == j { new } else { ident });
        }
        Some((old_h ^ (old_h >> 31), new_h ^ (new_h >> 31)))
    }

    /// [`SharedChunks::column_hash`] of an array; `None` when a section in it is not this
    /// store's (such an array is never in the table).
    fn hash_of(&self, arr: &[Section]) -> Option<u64> {
        let id = self.id();
        if arr.iter().any(|s| s.ident(id).is_none()) {
            return None;
        }
        Some(self.column_hash(arr.iter().filter_map(|s| s.ident(id))))
    }

    /// Number of entries, dead ones included (tests: dead entries must not linger).
    #[cfg(test)]
    pub(crate) fn entries(&self) -> usize {
        self.lock().sections.len()
    }

    /// Number of section array entries, dead ones included.
    #[cfg(test)]
    pub(crate) fn column_entries(&self) -> usize {
        self.lock().columns.len()
    }

    /// The live section stored under `key`.
    pub(crate) fn get(&self, key: u128) -> Option<Section> {
        // Upgrading under the lock is what makes the last-holder check in `forget` race-free.
        self.lock()
            .sections
            .get(&key)
            .and_then(WeakSection::upgrade)
    }

    /// Stores a freshly decoded section under `key` and returns the shared section to keep. A
    /// uniform section is returned as it is (it owns no heap memory). When another thread
    /// stored the same key meanwhile, its section is returned and `section` is dropped.
    pub(crate) fn insert(&self, key: u128, section: Section) -> Section {
        let (shared, weak) = match section.into_shared(key, None, Arc::downgrade(&self.inner)) {
            Ok(s) => s,
            Err(uniform) => return uniform,
        };
        let existing = {
            let mut t = self.lock();
            match t.sections.get(&key).and_then(WeakSection::upgrade) {
                Some(live) => Some(live),
                None => {
                    t.sections.insert(key, weak);
                    None
                }
            }
        };
        // `shared` is dropped outside the lock (its last drop takes the lock).
        existing.unwrap_or(shared)
    }

    /// The shared section array whose handles are exactly `idents` (in a chunk packet: the
    /// uniform values and wire keys of its sections), if a world holds one.
    pub(crate) fn find_column(
        &self,
        idents: impl Iterator<Item = Ident> + Clone,
    ) -> Option<Arc<[Section]>> {
        let hash = self.column_hash(idents.clone());
        let id = self.id();
        let found = self.lock().columns.get(&hash).and_then(Weak::upgrade)?;
        // Compared (and, on a mismatch, dropped) outside the lock.
        let mut idents = idents;
        let same = found.iter().all(|s| s.ident(id) == idents.next()) && idents.next().is_none();
        same.then_some(found)
    }

    /// Shares a freshly decoded section array: returns the array already in the store for the
    /// same handles, else stores `arr` and returns it. An array with a section that is not
    /// this store's (owned storage) is returned as it is.
    pub(crate) fn intern_column(&self, arr: Arc<[Section]>) -> Arc<[Section]> {
        let Some(hash) = self.hash_of(&arr) else {
            return arr;
        };
        let id = self.id();
        let found = {
            let mut t = self.lock();
            let found = t.columns.get(&hash).and_then(Weak::upgrade);
            if found.is_none() {
                t.register_column(hash, &arr);
            }
            found
        };
        match found {
            Some(f)
                if f.len() == arr.len()
                    && f.iter()
                        .zip(arr.iter())
                        .all(|(a, b)| a.ident(id) == b.ident(id)) =>
            {
                f
            }
            // Another array with the same hash: `arr` stays unshared.
            _ => arr,
        }
    }

    /// Lets go of a section array a world held, removing its entry with the last handle.
    pub(crate) fn release_column(&self, arr: Arc<[Section]>) {
        // The counts are a hint: two threads letting go of the last two handles at once both
        // skip this, and the entry is swept later (see `Tables::sweep_at`).
        if Arc::strong_count(&arr) == 1 && Arc::weak_count(&arr) > 0 {
            let hash = self.hash_of(&arr);
            let mut t = self.lock();
            // Under the lock nobody can take a new handle from the entry.
            if Arc::strong_count(&arr) == 1 {
                t.unregister_column(hash, &arr);
            }
        }
        drop(arr);
    }

    /// Applies block changes to section `j` of a world's section array: `recs` are `(block
    /// index, new state)` in packet order, `on_change(index, old, new)` (if any) is called for
    /// every record in order. Returns `false` and does nothing when the section is not this
    /// store's (owned storage: the caller changes it in place as a world without a store does).
    ///
    /// Exactness: the world ends up reading the section it had with `recs` applied in order.
    /// The new section is named by its content key, the old content key moved by the two
    /// terms of every changed block; a section found under that key has exactly that content.
    /// Nothing another world can read is ever written: storage is changed in place only
    /// through `Arc::get_mut`, on the array and on the section, which succeeds only for the
    /// one and only handle.
    pub(crate) fn update<I>(
        &self,
        protocol: u16,
        arr: &mut Arc<[Section]>,
        j: usize,
        recs: I,
        mut on_change: Option<&mut dyn FnMut(usize, u16, u16)>,
    ) -> bool
    where
        I: Iterator<Item = (usize, u16)> + Clone,
    {
        let Some(section) = arr.get(j) else {
            return false;
        };
        let Some(old_key) = self.content_key(protocol, section) else {
            return false;
        };
        // The new key without touching the storage, when no block is named twice.
        let mut key = old_key;
        let mut changes = 0usize;
        let mut single = (0, 0);
        let mut twice = false;
        let mut rest = recs.clone();
        if let (Some((i, s)), None) = (rest.next(), rest.next()) {
            // One record (a `block_update`, a prediction).
            let i = i & (VOLUME - 1);
            let old = section.get(i);
            if old != s {
                key ^= self.term(protocol, i, old) ^ self.term(protocol, i, s);
                changes = 1;
                single = (i, s);
            }
        } else {
            let mut seen = [0u64; VOLUME / 64];
            for (i, s) in recs.clone() {
                let i = i & (VOLUME - 1);
                let bit = 1u64 << (i & 63);
                if seen[i >> 6] & bit != 0 {
                    twice = true;
                    break;
                }
                seen[i >> 6] |= bit;
                let old = section.get(i);
                if old != s {
                    key ^= self.term(protocol, i, old) ^ self.term(protocol, i, s);
                    changes += 1;
                    single = (i, s);
                }
            }
        }
        // The section with the changes applied, once it had to be built.
        let mut built = None;
        if twice {
            let mut owned = section.to_owned_storage();
            key = old_key;
            for (i, s) in recs.clone() {
                let i = i & (VOLUME - 1);
                let old = owned.set(i, s);
                if old != s {
                    key ^= self.term(protocol, i, old) ^ self.term(protocol, i, s);
                }
                if let Some(f) = on_change.as_mut() {
                    f(i, old, s);
                }
            }
            if key == old_key {
                return true;
            }
            built = Some(owned);
            changes = usize::MAX;
        } else {
            if let Some(f) = on_change.as_mut() {
                for (i, s) in recs.clone() {
                    f(i & (VOLUME - 1), section.get(i), s);
                }
            }
            if changes == 0 {
                return true;
            }
        }
        // The array's hash now and with the new section, which will be stored under `key`.
        let hashes = self.hashes_with(arr, j, Ident::Keyed(key));
        // Declared before the lock guard: dropped after it.
        let mut trash = Trash::default();
        loop {
            let mut t = self.lock();
            let live = t.sections.get(&key).and_then(WeakSection::upgrade);
            let new = match (live, built.take()) {
                (Some(live), _) => live,
                (None, Some(owned)) => {
                    match owned.into_shared(key, Some(key), Arc::downgrade(&self.inner)) {
                        Ok((shared, weak)) => {
                            t.sections.insert(key, weak);
                            shared
                        }
                        Err(uniform) => uniform,
                    }
                }
                (None, None) => {
                    if changes == 1 && self.set_in_place(&mut t, arr, hashes, j, single, key) {
                        return true;
                    }
                    drop(t);
                    // Copy and apply outside the lock, then look again.
                    let mut owned = arr[j].to_owned_storage();
                    for (i, s) in recs.clone() {
                        owned.set(i, s);
                    }
                    built = Some(owned);
                    continue;
                }
            };
            self.replace_section(&mut t, arr, hashes, j, new, &mut trash);
            return true;
        }
    }

    /// The sole-holder path of [`SharedChunks::update`]: when this world is the only holder of
    /// the array and the array the only holder of the section's storage, the block is changed
    /// in the storage itself and the storage moves to its new key. `false` when that is not
    /// possible (then at most the entries of the array and the section were removed, which is
    /// harmless: the caller replaces the section next).
    fn set_in_place(
        &self,
        t: &mut Tables,
        arr: &mut Arc<[Section]>,
        hashes: Option<(u64, u64)>,
        j: usize,
        (i, state): (usize, u16),
        key: u128,
    ) -> bool {
        // Under the lock no handle can be made from a store entry, and every other way to a
        // new handle needs one of the handles counted here.
        if Arc::strong_count(arr) != 1 || arr[j].shared_holders() != Some(1) {
            return false;
        }
        let Some(Ident::Keyed(old_key)) = arr[j].ident(self.id()) else {
            return false;
        };
        t.unregister_column(hashes.map(|h| h.0), arr);
        let Some(slots) = Arc::get_mut(arr) else {
            return false;
        };
        if t.sections
            .get(&old_key)
            .is_some_and(|w| w.points_to(&slots[j]))
        {
            t.sections.remove(&old_key);
        }
        let Some(weak) = slots[j].set_sole(i, state, key) else {
            return false;
        };
        // `key` had no live entry (the caller looked under this lock).
        t.sections.insert(key, weak);
        if let Some((_, hash)) = hashes {
            t.register_column(hash, arr);
        }
        true
    }

    /// Puts `new` at index `j` of a world's section array without writing to an array another
    /// world can read: takes the array already in the store for the resulting handles, else
    /// changes the world's array in place when it is its only holder, else builds a new array.
    fn replace_section(
        &self,
        t: &mut Tables,
        arr: &mut Arc<[Section]>,
        hashes: Option<(u64, u64)>,
        j: usize,
        new: Section,
        trash: &mut Trash,
    ) {
        let id = self.id();
        let new_ident = new.ident(id);
        let old_hash = hashes.map(|h| h.0);
        // `hashes` assumed keyed storage; anything else (a section that became uniform, which
        // a change never yields) is hashed again.
        let new_hash = match (hashes, new_ident) {
            (Some((_, h)), Some(Ident::Keyed(_))) => Some(h),
            (Some(_), Some(ident)) => self.hashes_with(arr, j, ident).map(|h| h.1),
            _ => None,
        };
        if let Some(hash) = new_hash
            && let Some(found) = t.columns.get(&hash).and_then(Weak::upgrade)
        {
            let same =
                found.len() == arr.len()
                    && found.iter().zip(arr.iter()).enumerate().all(|(k, (f, s))| {
                        f.ident(id) == if k == j { new_ident } else { s.ident(id) }
                    });
            if same {
                let old = std::mem::replace(arr, found);
                if Arc::strong_count(&old) == 1 {
                    t.unregister_column(old_hash, &old);
                }
                trash.arrays[0] = Some(old);
                trash.sections[0] = Some(new);
                return;
            }
            trash.arrays[1] = Some(found);
        }
        if Arc::strong_count(arr) == 1 {
            t.unregister_column(old_hash, arr);
            if let Some(slots) = Arc::get_mut(arr) {
                trash.sections[0] = Some(std::mem::replace(&mut slots[j], new));
                if let Some(hash) = new_hash {
                    t.register_column(hash, arr);
                }
                return;
            }
        }
        let built: Arc<[Section]> = arr
            .iter()
            .enumerate()
            .map(|(k, s)| if k == j { new.clone() } else { s.clone() })
            .collect();
        trash.sections[0] = Some(new);
        if let Some(hash) = new_hash {
            t.register_column(hash, &built);
        }
        trash.arrays[0] = Some(std::mem::replace(arr, built));
    }
}

/// Called by a shared section's storage when its last holder dropped it: removes the dead
/// entry (unless the key was stored again meanwhile).
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
pub(crate) fn forget(cache: &Weak<Inner>, key: u128) {
    let Some(inner) = cache.upgrade() else {
        return;
    };
    let mut t = inner.tables.lock().unwrap_or_else(PoisonError::into_inner);
    if t.sections.get(&key).is_some_and(WeakSection::is_dead) {
        t.sections.remove(&key);
    }
    drop(t);
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
