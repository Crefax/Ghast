//! Attributes as the 26.x client evaluates them.
//!
//! An attribute instance keeps its modifiers of each operation in a fastutil
//! `Object2ObjectOpenHashMap<Identifier, AttributeModifier>` and sums them in that map's iteration
//! order, so with several modifiers of one operation the floating-point result depends on the
//! table layout. [`OpTable`] reproduces it: 16 slots, linear probing from
//! `mix(identifier.hashCode())`, backward-shift deletion, and iteration from the last slot down.

/// Java `String.hashCode` of a UTF-16 string given as UTF-8 (BMP characters only; identifiers
/// are ASCII).
#[must_use]
pub fn java_string_hash(s: &str) -> i32 {
    let mut h: i32 = 0;
    for u in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(i32::from(u));
    }
    h
}

/// `Identifier.hashCode()` of `namespace:path` (`31 * namespace.hashCode() + path.hashCode()`); a
/// bare path means the `minecraft` namespace.
#[must_use]
pub fn identifier_hash(id: &str) -> i32 {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    java_string_hash(ns)
        .wrapping_mul(31)
        .wrapping_add(java_string_hash(path))
}

/// A stable 64-bit key of an identifier string, for equality (FNV-1a of `namespace:path`).
#[must_use]
pub fn identifier_key(id: &str) -> u64 {
    let full;
    let id = if id.contains(':') {
        id
    } else {
        full = format!("minecraft:{id}");
        &full
    };
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A modifier identifier: its Java hash (decides the slot) and a key for equality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModifierId {
    /// `Identifier.hashCode()`.
    pub hash: i32,
    /// Equality key ([`identifier_key`]).
    pub key: u64,
}

impl ModifierId {
    /// The id of `namespace:path`.
    #[must_use]
    pub fn new(id: &str) -> ModifierId {
        ModifierId {
            hash: identifier_hash(id),
            key: identifier_key(id),
        }
    }
}

/// The client's sprint modifier id, `minecraft:sprinting`.
#[must_use]
pub fn sprinting_id() -> ModifierId {
    ModifierId::new("minecraft:sprinting")
}

/// The sprint modifier amount: `0.3F` widened, operation 2 (multiply total).
pub const SPRINT_AMOUNT: f64 = 0.3f32 as f64;

/// One modifier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Modifier {
    /// Identifier.
    pub id: ModifierId,
    /// Amount.
    pub amount: f64,
}

const SLOTS: usize = 16;
/// fastutil's `maxFill` for 16 slots at load factor 0.75: the 13th entry would grow the table.
const MAX_ENTRIES: usize = 12;

/// `it.unimi.dsi.fastutil.HashCommon.mix`.
#[inline]
fn mix(x: i32) -> i32 {
    let h = x.wrapping_mul(0x9E37_79B9_u32 as i32);
    h ^ ((h as u32) >> 16) as i32
}

/// The modifiers of one operation, laid out like fastutil's open hash map of 16 slots.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct OpTable {
    slots: [Option<Modifier>; SLOTS],
    len: u8,
}

impl OpTable {
    #[inline]
    fn home(id: ModifierId) -> usize {
        (mix(id.hash) as u32 as usize) & (SLOTS - 1)
    }

    fn find(&self, id: ModifierId) -> Option<usize> {
        let mut pos = Self::home(id);
        loop {
            match self.slots[pos] {
                None => return None,
                Some(m) if m.id == id => return Some(pos),
                Some(_) => pos = (pos + 1) & (SLOTS - 1),
            }
        }
    }

    /// `put`: replaces the amount in place when the id is present, else inserts. More than 12
    /// modifiers of one operation are not modelled (vanilla would grow the table); extra ones are
    /// dropped.
    pub fn put(&mut self, m: Modifier) {
        if let Some(i) = self.find(m.id) {
            self.slots[i] = Some(m);
            return;
        }
        if usize::from(self.len) >= MAX_ENTRIES {
            return;
        }
        let mut pos = Self::home(m.id);
        while self.slots[pos].is_some() {
            pos = (pos + 1) & (SLOTS - 1);
        }
        self.slots[pos] = Some(m);
        self.len += 1;
    }

    /// `remove` with fastutil's backward-shift deletion; `true` if the id was present.
    pub fn remove(&mut self, id: ModifierId) -> bool {
        let Some(mut pos) = self.find(id) else {
            return false;
        };
        self.len -= 1;
        // shiftKeys(pos)
        loop {
            let last = pos;
            pos = (last + 1) & (SLOTS - 1);
            let curr = loop {
                let Some(curr) = self.slots[pos] else {
                    self.slots[last] = None;
                    return true;
                };
                let slot = Self::home(curr.id);
                let stays = if last <= pos {
                    last >= slot || slot > pos
                } else {
                    last >= slot && slot > pos
                };
                if stays {
                    break curr;
                }
                pos = (pos + 1) & (SLOTS - 1);
            };
            self.slots[last] = Some(curr);
        }
    }

    /// Removes everything.
    pub fn clear(&mut self) {
        *self = OpTable::default();
    }

    /// Number of modifiers.
    #[must_use]
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }

    /// `true` when empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The modifiers in vanilla's iteration order (last slot first).
    pub fn iter(&self) -> impl Iterator<Item = &Modifier> {
        self.slots.iter().rev().flatten()
    }
}

/// An attribute's base value and modifiers (vanilla `AttributeInstance`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attribute {
    /// Base value.
    pub base: f64,
    /// Modifiers of operations 0 (add value), 1 (add multiplied base), 2 (add multiplied total).
    pub ops: [OpTable; 3],
    /// Vanilla range (`RangedAttribute` min / max).
    pub min: f64,
    /// Upper bound.
    pub max: f64,
}

impl Attribute {
    /// An attribute with no modifiers.
    #[must_use]
    pub fn new(base: f64, min: f64, max: f64) -> Attribute {
        Attribute {
            base,
            ops: [OpTable::default(); 3],
            min,
            max,
        }
    }

    /// `AttributeInstance.getValue()`.
    #[must_use]
    pub fn value(&self) -> f64 {
        let mut base = self.base;
        for m in self.ops[0].iter() {
            base += m.amount;
        }
        let mut result = base;
        for m in self.ops[1].iter() {
            result += base * m.amount;
        }
        for m in self.ops[2].iter() {
            result *= 1.0 + m.amount;
        }
        // RangedAttribute.sanitizeValue: NaN -> min, else clamp.
        if result.is_nan() || result < self.min {
            self.min
        } else {
            crate::math::java_min(result, self.max)
        }
    }

    /// Removes the modifier with this id from every operation (`removeModifier`).
    pub fn remove(&mut self, id: ModifierId) -> bool {
        self.ops.iter_mut().any(|t| t.remove(id))
    }

    /// `addTransientModifier` (vanilla refuses an id that is already present; so does this).
    pub fn add(&mut self, id: ModifierId, amount: f64, operation: u8) {
        if self.ops.iter().any(|t| t.find(id).is_some()) {
            return;
        }
        if let Some(t) = self.ops.get_mut(usize::from(operation)) {
            t.put(Modifier { id, amount });
        }
    }

    /// What the attributes packet does: set the base, remove every modifier, add the packet's
    /// modifiers `(id, amount, operation)` in packet order.
    pub fn apply_packet(&mut self, base: f64, modifiers: &[(ModifierId, f64, u8)]) {
        self.base = base;
        for t in &mut self.ops {
            t.clear();
        }
        for &(id, amount, op) in modifiers {
            self.add(id, amount, op);
        }
    }

    /// `true` when a modifier with this id is present.
    #[must_use]
    pub fn contains(&self, id: ModifierId) -> bool {
        self.ops.iter().any(|t| t.find(id).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_hash_is_java() {
        // Java: "minecraft".hashCode() == 695073197, "sprinting".hashCode() == 1429809032.
        assert_eq!(java_string_hash("minecraft"), 695_073_197);
        assert_eq!(java_string_hash("sprinting"), 1_429_809_032);
        assert_eq!(
            identifier_hash("minecraft:sprinting"),
            java_string_hash("minecraft")
                .wrapping_mul(31)
                .wrapping_add(java_string_hash("sprinting"))
        );
    }

    #[test]
    fn shift_delete_keeps_chain() {
        let mut t = OpTable::default();
        let ids: Vec<ModifierId> = (0..10)
            .map(|i| ModifierId::new(&format!("t:m{i}")))
            .collect();
        for (i, &id) in ids.iter().enumerate() {
            t.put(Modifier {
                id,
                amount: i as f64,
            });
        }
        for &id in &ids[..5] {
            assert!(t.remove(id));
        }
        for &id in &ids[5..] {
            assert!(t.find(id).is_some());
        }
        assert_eq!(t.len(), 5);
    }
}
