//! The movement speed attribute with its modifiers, evaluated exactly like vanilla's attribute
//! instance (1.8 and 1.16 share the rules).
//!
//! Vanilla keeps the modifiers of each operation in a hash set and sums them in the set's
//! iteration order, so with several modifiers the floating-point result depends on that order.
//! [`Modifiers`] reproduces it: within an operation, modifiers are visited by the bucket a Java
//! `HashSet` of the default capacity puts their UUID in, and by insertion order within a bucket.

/// The client's own sprint modifier: UUID `662a6b8d-da3e-4c1c-8813-96ea6097278d`, amount `0.3F`
/// widened, operation 2 (multiply total). Vanilla adds and removes it in `setSprinting`.
pub const SPRINT_UUID: u128 = 0x662a6b8d_da3e_4c1c_8813_96ea6097278d;

/// Sprint modifier amount.
pub const SPRINT_AMOUNT: f64 = 0.3f32 as f64;

/// Capacity of [`Modifiers`] (vanilla movement speed rarely has more than three).
const CAPACITY: usize = 8;

/// One attribute modifier.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Modifier {
    /// Modifier UUID (it decides the summation order).
    pub uuid: u128,
    /// Amount.
    pub amount: f64,
    /// Operation: 0 add, 1 multiply base, 2 multiply total.
    pub operation: u8,
}

/// The modifiers of one attribute, in vanilla's insertion order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Modifiers {
    list: [Modifier; CAPACITY],
    len: usize,
}

/// `java.util.UUID.hashCode`.
#[inline]
fn uuid_hash(u: u128) -> i32 {
    let hilo = ((u >> 64) as u64) ^ (u as u64);
    ((hilo >> 32) as i32) ^ (hilo as i32)
}

/// The bucket of a hash in a Java `HashMap` with 16 buckets.
#[inline]
fn bucket(h: i32) -> u32 {
    let h = h as u32;
    (h ^ (h >> 16)) & 15
}

impl Modifiers {
    /// No modifiers.
    pub const NONE: Modifiers = Modifiers {
        list: [Modifier {
            uuid: 0,
            amount: 0.0,
            operation: 0,
        }; CAPACITY],
        len: 0,
    };

    /// The modifiers in insertion order.
    #[must_use]
    pub fn as_slice(&self) -> &[Modifier] {
        &self.list[..self.len]
    }

    /// Removes every modifier (vanilla `clearModifiers`, as the entity properties packet does —
    /// the client's sprint modifier included).
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Adds a modifier (vanilla `addModifier`); a modifier with the same UUID is replaced in
    /// place of the old one's removal. At most eight are kept.
    pub fn insert(&mut self, uuid: u128, amount: f64, operation: u8) {
        self.remove(uuid);
        if self.len < CAPACITY {
            self.list[self.len] = Modifier {
                uuid,
                amount,
                operation,
            };
            self.len += 1;
        }
    }

    /// Adds a modifier without a known UUID (treated as UUID 0).
    pub fn push(&mut self, amount: f64, operation: u8) {
        if self.len < CAPACITY {
            self.list[self.len] = Modifier {
                uuid: 0,
                amount,
                operation,
            };
            self.len += 1;
        }
    }

    /// Removes the modifier with this UUID; `true` if there was one.
    pub fn remove(&mut self, uuid: u128) -> bool {
        let Some(i) = self.as_slice().iter().position(|m| m.uuid == uuid) else {
            return false;
        };
        self.list.copy_within(i + 1..self.len, i);
        self.len -= 1;
        true
    }

    /// `true` when a modifier with this UUID is present.
    #[must_use]
    pub fn contains(&self, uuid: u128) -> bool {
        self.as_slice().iter().any(|m| m.uuid == uuid)
    }

    /// Vanilla `setSprinting`'s attribute part: remove the sprint modifier if present, add it
    /// again when sprinting.
    pub fn set_sprint(&mut self, sprinting: bool) {
        self.remove(SPRINT_UUID);
        if sprinting {
            self.insert(SPRINT_UUID, SPRINT_AMOUNT, 2);
        }
    }

    /// The modifiers of one operation in vanilla's iteration order.
    fn ordered(&self, operation: u8) -> impl Iterator<Item = &Modifier> {
        let mut idx: [usize; CAPACITY] = [0; CAPACITY];
        let mut n = 0;
        for (i, m) in self.as_slice().iter().enumerate() {
            if m.operation == operation {
                idx[n] = i;
                n += 1;
            }
        }
        // Stable sort by bucket keeps insertion order within a bucket.
        idx[..n].sort_by_key(|&i| bucket(uuid_hash(self.list[i].uuid)));
        (0..n).map(move |k| &self.list[idx[k]])
    }

    /// The attribute value for a base value, clamped to vanilla's movement speed range `0..=1024`.
    #[must_use]
    pub fn value(&self, base: f64) -> f64 {
        let mut d = base;
        for m in self.ordered(0) {
            d += m.amount;
        }
        let mut e = d;
        for m in self.ordered(1) {
            e += d * m.amount;
        }
        for m in self.ordered(2) {
            e *= 1.0 + m.amount;
        }
        e.clamp(0.0, 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_uuid_hash() {
        // new UUID(0x662a6b8dda3e4c1cL, 0x881396ea6097278dL).hashCode()
        let hilo: u64 = 0x662a_6b8d_da3e_4c1c ^ 0x8813_96ea_6097_278d;
        assert_eq!(
            uuid_hash(SPRINT_UUID),
            ((hilo >> 32) as i32) ^ (hilo as i32)
        );
    }

    #[test]
    fn sprint_toggle() {
        let mut m = Modifiers::NONE;
        m.set_sprint(true);
        assert_eq!(m.value(0.1), 0.1 * (1.0 + SPRINT_AMOUNT));
        m.set_sprint(false);
        assert_eq!(m.value(0.1), 0.1);
    }
}
