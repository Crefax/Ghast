//! The 1.16.5 client's own view of the entities around the bot, as far as the local player's
//! pushes and entity collisions read it (opt-in, [`crate::v1_16_5::Tracker::track_client_motion`]):
//!
//! * **Tick order.** `ClientWorld.tickEntities` walks the client world's entity map, a fastutil
//!   `Int2ObjectOpenHashMap<Entity>` keyed by entity id: its iteration order is the order of the
//!   hash table's slots (the id 0 first, then the slots from the top down), which depends on
//!   every id added and removed since the world was created. [`ClientMap`] keeps the same table
//!   (keys only) with fastutil 8.2.1's hashing, probing, removal shifting and resizing.
//! * **Positions and boxes.** Position packets do not move a living entity or a boat at once:
//!   the client interpolates over 3 ticks (living) or 10 ticks (boats), each step one
//!   `setPosition` that also re-centres the box. A shulker ignores position packets and sits on
//!   its attached block. Entity data that changes the size (pose, baby, slime size, armor stand
//!   flags) runs `calculateDimensions`: a box that does not get narrower keeps its minimum corner
//!   (`maxX = minX + width`, not `x + width / 2`) until the next `setPosition`.
//!
//! Not modelled: entities that remove themselves on the client (a dying mob after 20 ticks,
//! an item without a stack, old experience orbs; the server's destroy comes at about the same
//! time), a sleeping entity's bed position, pufferfish and phantom sizes, minecart
//! interpolation, passengers' positions, an open shulker's lid.

use crate::entity::{Entity, EntityType, MetaValue};
use crate::store::{Entities, Fast};

/// fastutil `HashCommon.INT_PHI`.
const INT_PHI: i32 = 0x9E37_79B9_u32 as i32;
/// `Hash.DEFAULT_INITIAL_SIZE`.
const DEFAULT_INITIAL_SIZE: usize = 16;
/// `Hash.DEFAULT_LOAD_FACTOR`.
const LOAD_FACTOR: f32 = 0.75;

/// fastutil `HashCommon.mix(int)`.
#[inline]
fn mix(x: i32) -> i32 {
    let h = x.wrapping_mul(INT_PHI);
    h ^ ((h as u32) >> 16) as i32
}

/// fastutil `HashCommon.arraySize(expected, f)` (float division, rounded up to a power of two).
fn array_size(expected: usize, f: f32) -> usize {
    let s = (expected as f32 / f).ceil() as u64;
    s.next_power_of_two().max(2) as usize
}

/// fastutil `HashCommon.maxFill(n, f)`.
fn max_fill(n: usize, f: f32) -> usize {
    ((n as f32 * f).ceil() as usize).min(n - 1)
}

/// The keys of the client world's `Int2ObjectOpenHashMap` (fastutil 8.2.1), slot for slot.
#[derive(Clone, Debug)]
pub(crate) struct ClientMap {
    /// Slots `0..n` (0 = free); the null key (id 0) lives outside them.
    key: Vec<i32>,
    n: usize,
    mask: usize,
    size: usize,
    max_fill: usize,
    min_n: usize,
    has_null: bool,
}

impl Default for ClientMap {
    fn default() -> Self {
        // `new Int2ObjectOpenHashMap<>()`: expected 16, load factor 0.75.
        let n = array_size(DEFAULT_INITIAL_SIZE, LOAD_FACTOR);
        ClientMap {
            key: vec![0; n],
            n,
            mask: n - 1,
            size: 0,
            max_fill: max_fill(n, LOAD_FACTOR),
            min_n: n,
            has_null: false,
        }
    }
}

impl ClientMap {
    /// The slot of `k`, or `None`.
    fn find(&self, k: i32) -> Option<usize> {
        if k == 0 {
            return self.has_null.then_some(self.n);
        }
        let mut pos = (mix(k) as usize) & self.mask;
        loop {
            let cur = self.key[pos];
            if cur == 0 {
                return None;
            }
            if cur == k {
                return Some(pos);
            }
            pos = (pos + 1) & self.mask;
        }
    }

    /// `put(k, v)` for a key not in the map (vanilla removes the old entity first).
    pub(crate) fn insert(&mut self, k: i32) {
        if k == 0 {
            if self.has_null {
                return;
            }
            self.has_null = true;
        } else {
            let mut pos = (mix(k) as usize) & self.mask;
            loop {
                let cur = self.key[pos];
                if cur == 0 {
                    break;
                }
                if cur == k {
                    return;
                }
                pos = (pos + 1) & self.mask;
            }
            self.key[pos] = k;
        }
        let before = self.size;
        self.size += 1;
        if before >= self.max_fill {
            self.rehash(array_size(self.size + 1, LOAD_FACTOR));
        }
    }

    /// `remove(k)`; `false` when `k` was not in the map.
    pub(crate) fn remove(&mut self, k: i32) -> bool {
        if k == 0 {
            if !self.has_null {
                return false;
            }
            self.has_null = false;
        } else {
            let Some(pos) = self.find(k) else {
                return false;
            };
            self.shift_keys(pos);
        }
        self.size -= 1;
        if self.n > self.min_n && self.size < self.max_fill / 4 && self.n > DEFAULT_INITIAL_SIZE {
            self.rehash(self.n / 2);
        }
        true
    }

    /// fastutil `shiftKeys`: closes the gap at `pos` (linear probing).
    fn shift_keys(&mut self, mut pos: usize) {
        loop {
            let last = pos;
            pos = (last + 1) & self.mask;
            let cur = loop {
                let cur = self.key[pos];
                if cur == 0 {
                    self.key[last] = 0;
                    return;
                }
                let slot = (mix(cur) as usize) & self.mask;
                let stays = if last <= pos {
                    last >= slot || slot > pos
                } else {
                    last >= slot && slot > pos
                };
                if stays {
                    break cur;
                }
                pos = (pos + 1) & self.mask;
            };
            self.key[last] = cur;
        }
    }

    /// fastutil `rehash(newN)`: the keys from the top slot down into a new table.
    fn rehash(&mut self, new_n: usize) {
        let mask = new_n - 1;
        let mut next = vec![0; new_n];
        for i in (0..self.n).rev() {
            let k = self.key[i];
            if k == 0 {
                continue;
            }
            let mut pos = (mix(k) as usize) & mask;
            while next[pos] != 0 {
                pos = (pos + 1) & mask;
            }
            next[pos] = k;
        }
        self.key = next;
        self.n = new_n;
        self.mask = mask;
        self.max_fill = max_fill(new_n, LOAD_FACTOR);
    }

    /// The iteration order (`int2ObjectEntrySet().iterator()`): the id 0, then the slots from
    /// the top down.
    pub(crate) fn order(&self) -> impl Iterator<Item = i32> + '_ {
        self.has_null
            .then_some(0)
            .into_iter()
            .chain(self.key.iter().rev().copied().filter(|&k| k != 0))
    }

    /// `true` when `a` comes before `b` in the iteration order (both in the map).
    pub(crate) fn before(&self, a: i32, b: i32) -> bool {
        match (self.find(a), self.find(b)) {
            (Some(x), Some(y)) => x > y,
            _ => false,
        }
    }

    /// The place of `k` in the iteration order as a sort key (smaller ticks earlier).
    pub(crate) fn rank(&self, k: i32) -> Option<u32> {
        self.find(k).map(|slot| (self.n - slot) as u32)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.size
    }
}

/// How the client moves an entity after a position packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lerp {
    /// `LivingEntity.updateTrackedPositionAndAngles`: the packet's steps (3).
    Living,
    /// `BoatEntity.updateTrackedPositionAndAngles`: 10 steps.
    Boat,
    /// `ShulkerEntity`: position packets do nothing.
    Still,
}

/// An entity's client position, interpolation and box.
#[derive(Clone, Debug)]
pub(crate) struct Motion {
    pub(crate) pos: [f64; 3],
    target: [f64; 3],
    steps: u8,
    lerp: Lerp,
    pub(crate) bb: [f64; 6],
    /// `this.dimensions` (width, height).
    dims: (f32, f32),
    pose: i32,
    slime: i32,
    stand: u8,
    baby: bool,
    /// Shulkers: the attached block.
    attached: Option<[i32; 3]>,
    /// Living: the last health the server sent is not above 0 (`isAlive` is false).
    pub(crate) dead: bool,
    /// Players: the profile name (the tab-list entry the client spawned it with).
    pub(crate) name: Option<Box<str>>,
}

/// Registry ids of the 1.16.5 types the client state reads.
pub(crate) mod ty {
    pub(crate) const ARMOR_STAND: u16 = 1;
    pub(crate) const BOAT: u16 = 6;
    pub(crate) const MAGMA_CUBE: u16 = 44;
    pub(crate) const PIGLIN: u16 = 60;
    pub(crate) const SHULKER: u16 = 70;
    pub(crate) const SLIME: u16 = 75;
}

fn box_at(dims: (f32, f32), p: [f64; 3]) -> [f64; 6] {
    // EntityDimensions.getBoxAt: half width in float.
    let f = f64::from(dims.0 / 2.0);
    let g = f64::from(dims.1);
    [p[0] - f, p[1], p[2] - f, p[0] + f, p[1] + g, p[2] + f]
}

impl Motion {
    fn set_position(&mut self, p: [f64; 3]) {
        self.pos = p;
        self.bb = box_at(self.dims, p);
    }

    /// `updateTrackedPositionAndAngles(target, 3)`.
    fn retarget(&mut self, target: [f64; 3]) {
        match self.lerp {
            Lerp::Living => {
                self.target = target;
                self.steps = 3;
            }
            Lerp::Boat => {
                self.target = target;
                self.steps = 10;
            }
            Lerp::Still => {}
        }
    }

    /// `calculateDimensions` with the size data as it is now.
    fn resize(&mut self, e: &Entity) {
        let Some(vs) = e.sizer else { return };
        let Some(d) =
            crate::v1_16_5::dims_with(e.ty, vs, self.pose, Some(self.slime), self.stand, self.baby)
        else {
            return;
        };
        let new = (d.width, d.height);
        let old = self.dims;
        self.dims = new;
        if new.0 < old.0 {
            let h = f64::from(new.0) / 2.0;
            let [x, y, z] = self.pos;
            self.bb = [x - h, y, z - h, x + h, y + f64::from(new.1), z + h];
        } else {
            let b = self.bb;
            self.bb = [
                b[0],
                b[1],
                b[2],
                b[0] + f64::from(new.0),
                b[1] + f64::from(new.1),
                b[2] + f64::from(new.0),
            ];
        }
        // Slimes, magma cubes and armor stands set their position again afterwards.
        if matches!(
            e.ty,
            EntityType::Registry(ty::SLIME | ty::MAGMA_CUBE | ty::ARMOR_STAND)
        ) {
            let p = self.pos;
            self.set_position(p);
        }
    }
}

/// The client view (see the module docs).
#[derive(Clone, Debug, Default)]
pub(crate) struct Client16 {
    pub(crate) map: ClientMap,
    pub(crate) motion: Fast<i32, Motion>,
}

impl Client16 {
    /// A new client world holding only the local player.
    pub(crate) fn new_world(&mut self, self_id: Option<i32>) {
        self.map = ClientMap::default();
        self.motion.clear();
        crate::store::shrink_map(&mut self.motion);
        if let Some(id) = self_id {
            self.map.insert(id);
        }
    }

    /// `addEntityPrivate`: the old entity of that id leaves the map first.
    pub(crate) fn add(&mut self, id: i32) {
        self.map.remove(id);
        self.map.insert(id);
        self.motion.remove(&id);
    }

    /// `removeEntity`.
    pub(crate) fn remove(&mut self, id: i32) {
        self.map.remove(id);
        if self.motion.remove(&id).is_some() {
            crate::store::shrink_map(&mut self.motion);
        }
    }

    /// A spawned entity at its spawn position (`setPosition`), with the type's registered size.
    pub(crate) fn spawned(&mut self, e: &Entity, name: Option<Box<str>>) {
        let lerp = match e.ty {
            EntityType::Registry(ty::BOAT) => Lerp::Boat,
            EntityType::Registry(ty::SHULKER) => Lerp::Still,
            EntityType::Player => Lerp::Living,
            _ if e.living => Lerp::Living,
            _ => return,
        };
        let Some((w, h)) = e.sizer.and_then(|s| s.registered) else {
            return;
        };
        let mut m = Motion {
            pos: e.pos,
            target: e.pos,
            steps: 0,
            lerp,
            bb: [0.0; 6],
            dims: (w, h),
            pose: 0,
            slime: 1,
            stand: 0,
            baby: false,
            attached: None,
            dead: false,
            name,
        };
        m.set_position(e.pos);
        let cap = self.motion.capacity();
        self.motion.insert(e.id, m);
        crate::store::after_insert(&mut self.motion, cap);
    }

    /// A teleport or a relative move to `target` (the tracked position).
    pub(crate) fn moved(&mut self, id: i32, target: [f64; 3]) {
        if let Some(m) = self.motion.get_mut(&id) {
            m.retarget(target);
        }
    }

    /// A look-only packet: the interpolation target becomes the current position.
    pub(crate) fn looked(&mut self, id: i32) {
        if let Some(m) = self.motion.get_mut(&id) {
            let p = m.pos;
            m.retarget(p);
        }
    }

    /// Entity data entries in packet order: the ones that change the size run
    /// `calculateDimensions` one after the other.
    pub(crate) fn data(&mut self, e: &Entity, entries: &[(u8, MetaValue)]) {
        let Some(m) = self.motion.get_mut(&e.id) else {
            return;
        };
        let id = match e.ty {
            EntityType::Registry(id) => Some(id),
            _ => None,
        };
        let baby_index = if id == Some(ty::PIGLIN) { 16 } else { 15 };
        let has_baby = e.sizer.is_some_and(|s| s.baby().is_some());
        for (i, v) in entries {
            let resize = match (*i, v) {
                (6, MetaValue::Pose(p)) => {
                    m.pose = *p;
                    true
                }
                (8, MetaValue::Float(h)) if e.living || e.ty == EntityType::Player => {
                    // Vanilla isAlive is `health > 0`: NaN counts as dead.
                    m.dead = h.is_nan() || *h <= 0.0;
                    false
                }
                (15, MetaValue::VarInt(n)) if matches!(id, Some(ty::SLIME | ty::MAGMA_CUBE)) => {
                    m.slime = *n;
                    true
                }
                (14, MetaValue::Byte(b)) if id == Some(ty::ARMOR_STAND) => {
                    m.stand = *b as u8;
                    true
                }
                (16, MetaValue::OptBlockPos(p)) if id == Some(ty::SHULKER) => {
                    m.attached = *p;
                    false
                }
                (i, MetaValue::Bool(b)) if has_baby && i == baby_index => {
                    m.baby = *b;
                    true
                }
                _ => false,
            };
            if resize {
                m.resize(e);
            }
        }
    }

    /// One client tick of every entity that ticks (`ticks(entity)`): one interpolation step;
    /// a shulker sits on its attached block.
    /// The drawn state ([`Entity::drawn_position`]) of every ticked entity follows.
    pub(crate) fn tick(&mut self, store: &mut Entities, ticks: impl Fn(&Entity) -> bool) {
        for (id, m) in &mut self.motion {
            let Some(e) = store.map.get_mut(id) else {
                continue;
            };
            if !ticks(e) {
                continue;
            }
            m.tick_one();
            if let Some(c) = e.client.as_deref_mut() {
                c.pos = m.pos;
                c.bb = Some(m.bb);
            }
        }
    }

    /// The drawn position and box of entity `id`.
    pub(crate) fn drawn(&self, id: i32) -> Option<([f64; 3], [f64; 6])> {
        self.motion.get(&id).map(|m| (m.pos, m.bb))
    }
}

impl Motion {
    /// One client tick: one interpolation step; a shulker sits on its attached block.
    fn tick_one(&mut self) {
        if let Some(b) = self.attached {
            // ShulkerEntity.tick: resetPosition (no box) and the closed shell's box.
            let (x, y, z) = (f64::from(b[0]), f64::from(b[1]), f64::from(b[2]));
            self.pos = [x + 0.5, y, z + 0.5];
            self.bb = [
                self.pos[0] - 0.5,
                y,
                self.pos[2] - 0.5,
                self.pos[0] + 0.5,
                y + 1.0,
                self.pos[2] + 0.5,
            ];
            return;
        }
        if self.steps > 0 && self.lerp != Lerp::Still {
            let n = f64::from(self.steps);
            let p = [
                self.pos[0] + (self.target[0] - self.pos[0]) / n,
                self.pos[1] + (self.target[1] - self.pos[1]) / n,
                self.pos[2] + (self.target[2] - self.pos[2]) / n,
            ];
            self.steps -= 1;
            self.set_position(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain model of the table, written from fastutil's documented behaviour, against
    /// [`ClientMap`] over random adds and removes (orders must agree).
    #[test]
    fn grows_and_shrinks() {
        let mut m = ClientMap::default();
        for k in 1..=100 {
            m.insert(k * 7);
        }
        assert_eq!(m.len(), 100);
        assert_eq!(m.n, 256);
        for k in 1..=95 {
            assert!(m.remove(k * 7));
        }
        assert_eq!(m.len(), 5);
        assert!(m.n < 256);
        let order: Vec<i32> = m.order().collect();
        assert_eq!(order.len(), 5);
        m.insert(0);
        assert_eq!(m.order().next(), Some(0));
        assert!(m.before(0, 96 * 7));
    }
}
