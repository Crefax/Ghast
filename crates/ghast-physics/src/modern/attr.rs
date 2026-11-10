//! Attributes as the 26.x client evaluates them.
//!
//! An attribute instance keeps its modifiers of each operation in a fastutil
//! `Object2ObjectOpenHashMap<Identifier, AttributeModifier>` and sums them in that map's iteration
//! order, so with several modifiers of one operation the floating-point result depends on the
//! table layout. [`Attribute`] reproduces it: 32 slots (the default expected size 16 at load factor
//! 0.75), linear probing from
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
    /// Equality key (`identifier_key`).
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

/// Slots of fastutil's default `Object2ObjectOpenHashMap` (expected size 16 at load factor 0.75).
const SLOTS: usize = 32;
/// Modifiers kept per attribute (all operations together); vanilla keeps any number, more than
/// this are dropped (no movement attribute carries that many in practice).
pub const MAX_MODIFIERS: usize = 12;

/// `it.unimi.dsi.fastutil.HashCommon.mix`.
#[inline]
fn mix(x: i32) -> i32 {
    let h = x.wrapping_mul(0x9E37_79B9_u32 as i32);
    h ^ ((h as u32) >> 16) as i32
}

#[inline]
fn home(id: ModifierId) -> usize {
    (mix(id.hash) as u32 as usize) & (SLOTS - 1)
}

/// A stored modifier: its operation and its slot in that operation's hash table.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry {
    m: Modifier,
    op: u8,
    slot: u8,
}

const EMPTY_ENTRY: Entry = Entry {
    m: Modifier {
        id: ModifierId { hash: 0, key: 0 },
        amount: 0.0,
    },
    op: 0,
    slot: 0,
};

/// An attribute's base value and modifiers (vanilla `AttributeInstance`), with the value cached.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attribute {
    base: f64,
    min: f64,
    max: f64,
    value: f64,
    entries: [Entry; MAX_MODIFIERS],
    len: u8,
}

impl Attribute {
    /// An attribute with no modifiers and vanilla's range.
    #[must_use]
    pub fn new(base: f64, min: f64, max: f64) -> Attribute {
        let mut a = Attribute {
            base,
            min,
            max,
            value: 0.0,
            entries: [EMPTY_ENTRY; MAX_MODIFIERS],
            len: 0,
        };
        a.recompute();
        a
    }

    /// The base value.
    #[must_use]
    pub fn base(&self) -> f64 {
        self.base
    }

    /// `AttributeInstance.getValue()` (cached).
    #[inline]
    #[must_use]
    pub fn value(&self) -> f64 {
        self.value
    }

    /// Sets the base value.
    pub fn set_base(&mut self, base: f64) {
        self.base = base;
        self.recompute();
    }

    fn entries(&self) -> &[Entry] {
        &self.entries[..usize::from(self.len)]
    }

    /// The operation's slot table: slot -> entry index + 1 (0 = empty).
    fn table(&self, op: u8) -> [u8; SLOTS] {
        let mut t = [0u8; SLOTS];
        for (i, e) in self.entries().iter().enumerate() {
            if e.op == op {
                t[usize::from(e.slot)] = i as u8 + 1;
            }
        }
        t
    }

    fn recompute(&mut self) {
        // fastutil iterates each table from the last slot down.
        let mut base = self.base;
        for &i in self.table(0).iter().rev() {
            if i != 0 {
                base += self.entries[usize::from(i - 1)].m.amount;
            }
        }
        let mut result = base;
        for &i in self.table(1).iter().rev() {
            if i != 0 {
                result += base * self.entries[usize::from(i - 1)].m.amount;
            }
        }
        for &i in self.table(2).iter().rev() {
            if i != 0 {
                result *= 1.0 + self.entries[usize::from(i - 1)].m.amount;
            }
        }
        // RangedAttribute.sanitizeValue: NaN -> min, else clamp.
        self.value = if result.is_nan() || result < self.min {
            self.min
        } else {
            crate::math::java_min(result, self.max)
        };
    }

    /// `true` when a modifier with this id is present.
    #[must_use]
    pub fn contains(&self, id: ModifierId) -> bool {
        self.entries().iter().any(|e| e.m.id == id)
    }

    /// `addTransientModifier` (vanilla refuses an id that is already present; so does this).
    pub fn add(&mut self, id: ModifierId, amount: f64, operation: u8) {
        if operation > 2 || self.contains(id) || usize::from(self.len) >= MAX_MODIFIERS {
            return;
        }
        let t = self.table(operation);
        let mut pos = home(id);
        while t[pos] != 0 {
            pos = (pos + 1) & (SLOTS - 1);
        }
        self.entries[usize::from(self.len)] = Entry {
            m: Modifier { id, amount },
            op: operation,
            slot: pos as u8,
        };
        self.len += 1;
        self.recompute();
    }

    /// `removeModifier(id)` with fastutil's backward-shift deletion in that operation's table;
    /// `true` if the id was present.
    pub fn remove(&mut self, id: ModifierId) -> bool {
        let Some(idx) = self.entries().iter().position(|e| e.m.id == id) else {
            return false;
        };
        let op = self.entries[idx].op;
        let mut t = self.table(op);
        let mut pos = usize::from(self.entries[idx].slot);
        t[pos] = 0;
        // shiftKeys(pos)
        'outer: loop {
            let last = pos;
            pos = (last + 1) & (SLOTS - 1);
            loop {
                let cur = t[pos];
                if cur == 0 {
                    break 'outer;
                }
                let slot = home(self.entries[usize::from(cur - 1)].m.id);
                let stays = if last <= pos {
                    last >= slot || slot > pos
                } else {
                    last >= slot && slot > pos
                };
                if stays {
                    t[last] = cur;
                    t[pos] = 0;
                    break;
                }
                pos = (pos + 1) & (SLOTS - 1);
            }
        }
        for (s, &i) in t.iter().enumerate() {
            if i != 0 {
                self.entries[usize::from(i - 1)].slot = s as u8;
            }
        }
        // Drop the entry (indices of the others shift, slots stay).
        let n = usize::from(self.len);
        self.entries.copy_within(idx + 1..n, idx);
        self.len -= 1;
        self.recompute();
        true
    }

    /// What the attributes packet does: set the base, remove every modifier, add the packet's
    /// modifiers `(id, amount, operation)` in packet order.
    pub fn apply_packet(&mut self, base: f64, modifiers: &[(ModifierId, f64, u8)]) {
        self.base = base;
        self.len = 0;
        for &(id, amount, op) in modifiers {
            self.add(id, amount, op);
        }
        self.recompute();
    }

    /// The modifiers of one operation in vanilla's iteration order.
    pub fn modifiers(&self, operation: u8) -> impl Iterator<Item = Modifier> + '_ {
        let t = self.table(operation);
        (0..SLOTS)
            .rev()
            .filter(move |&s| t[s] != 0)
            .map(move |s| self.entries[usize::from(t[s] - 1)].m)
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
        let mut a = Attribute::new(0.0, -1.0e9, 1.0e9);
        let ids: Vec<ModifierId> = (0..10)
            .map(|i| ModifierId::new(&format!("t:m{i}")))
            .collect();
        for (i, &id) in ids.iter().enumerate() {
            a.add(id, i as f64, 0);
        }
        for &id in &ids[..5] {
            assert!(a.remove(id));
        }
        for &id in &ids[5..] {
            assert!(a.contains(id));
        }
        assert_eq!(a.value(), 5.0 + 6.0 + 7.0 + 8.0 + 9.0);
    }

    /// JDK 25 + fastutil 8.5.18: an `Object2ObjectOpenHashMap` holding these two ids iterates
    /// `m9346f493eec5` first (slot 22 of 32) and `mb720f71bb819` second (slot 14).
    #[test]
    fn iteration_order_matches_fastutil() {
        let mut a = Attribute::new(0.0, -1.0e9, 1.0e9);
        let first = ModifierId::new("oracle:m9346f493eec5");
        let second = ModifierId::new("oracle:mb720f71bb819");
        a.add(second, 1.0, 0);
        a.add(first, 2.0, 0);
        let order: Vec<ModifierId> = a.modifiers(0).map(|m| m.id).collect();
        assert_eq!(order, vec![first, second]);
    }
}
