//! Item stacks: one version-independent type over the 1.8 / 1.12 (`slot::legacy`), 1.16.5
//! (`slot::modern`) and 26.1 (`slot::v775`) wire formats.
//!
//! Item data stays raw: NBT as [`RawNbt`], 26.1 data components as a
//! [`ComponentPatch`](ghast_protocol::types::slot::v775::ComponentPatch) slice. Converting a
//! `window_items` packet into stacks only moves reference-counted bytes. Comparisons compare
//! the raw bytes first and only look inside when the bytes differ (vanilla compares NBT trees,
//! which ignore compound key order and the root name, and component maps, which ignore order).

use ghast_protocol::nbt::{RawNbt, Tag};
use ghast_protocol::types::slot::{legacy, modern};

/// The item data a stack carries, in its version's form.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ItemData {
    /// Nothing beyond the item id (no NBT; 26.1: the item's default components).
    #[default]
    None,
    /// 1.8 – 1.16.5 item NBT exactly as received.
    Nbt(RawNbt),
    /// 26.1 data component patch (added / removed against the item's defaults) exactly as
    /// received; components are decoded only when asked for. Boxed so that a [`Stack`] stays
    /// small for the versions without components.
    #[cfg(feature = "v26_1")]
    Components(Box<ghast_protocol::types::slot::v775::ComponentPatch>),
}

impl ItemData {
    /// From optional NBT.
    #[must_use]
    pub fn from_nbt(nbt: Option<RawNbt>) -> ItemData {
        nbt.map_or(ItemData::None, ItemData::Nbt)
    }
}

/// A non-empty item stack as the client holds it.
///
/// `count` is a Java `int` in vanilla (sent as a byte before 26.1). 1.8 keeps stacks with a
/// count of zero or less as real stacks (they render as a red "0"); later versions treat them as
/// empty, so their [`Inventory`](crate::Inventory) never stores one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Stack {
    /// Numeric item id (1.8 / 1.12: legacy id; 1.16.5 / 26.1: item registry id).
    pub id: i32,
    /// Stack size.
    pub count: i32,
    /// 1.8 / 1.12 damage / metadata. Always 0 on 1.16.5 (the `Damage` NBT tag) and 26.1 (the
    /// `damage` component); see [`Stack::damage_value`].
    pub damage: i16,
    /// NBT or data components exactly as received.
    pub data: ItemData,
}

impl Stack {
    /// A stack without NBT or component changes.
    #[must_use]
    pub fn new(id: i32, count: i32, damage: i16) -> Stack {
        Stack {
            id,
            count,
            damage,
            data: ItemData::None,
        }
    }

    /// The NBT (1.8 – 1.16.5), if any.
    #[must_use]
    pub fn nbt(&self) -> Option<&RawNbt> {
        match &self.data {
            ItemData::Nbt(n) => Some(n),
            _ => None,
        }
    }

    /// This stack with its NBT replaced.
    #[must_use]
    pub fn with_nbt(mut self, nbt: Option<RawNbt>) -> Stack {
        self.data = ItemData::from_nbt(nbt);
        self
    }

    /// The 26.1 component patch, if the stack changes any component.
    #[cfg(feature = "v26_1")]
    #[must_use]
    pub fn components(&self) -> Option<&ghast_protocol::types::slot::v775::ComponentPatch> {
        match &self.data {
            ItemData::Components(c) => Some(c),
            _ => None,
        }
    }

    /// The same item (id, damage, NBT) with another count.
    #[must_use]
    pub fn with_count(&self, count: i32) -> Stack {
        Stack {
            count,
            ..self.clone()
        }
    }

    /// Converts a 1.8 wire stack.
    #[must_use]
    pub fn from_legacy(s: legacy::ItemStack) -> Stack {
        Stack {
            id: i32::from(s.id),
            count: i32::from(s.count),
            damage: s.damage,
            data: ItemData::from_nbt(s.nbt),
        }
    }

    /// Converts a 1.16.5 wire stack.
    #[must_use]
    pub fn from_modern(s: modern::ItemStack) -> Stack {
        Stack {
            id: s.id,
            count: i32::from(s.count),
            damage: 0,
            data: ItemData::from_nbt(s.nbt),
        }
    }

    /// Converts a 26.1 wire stack (an empty patch becomes [`ItemData::None`]).
    #[cfg(feature = "v26_1")]
    #[must_use]
    pub fn from_v775(s: ghast_protocol::types::slot::v775::ItemStack) -> Stack {
        Stack {
            id: s.id,
            count: s.count,
            damage: 0,
            data: if s.components.is_empty() {
                ItemData::None
            } else {
                ItemData::Components(Box::new(s.components))
            },
        }
    }

    /// The 26.1 wire form (any NBT is dropped: 26.1 has none).
    #[cfg(feature = "v26_1")]
    #[must_use]
    pub fn to_v775(&self) -> ghast_protocol::types::slot::v775::ItemStack {
        ghast_protocol::types::slot::v775::ItemStack {
            id: self.id,
            count: self.count,
            components: self.components().cloned().unwrap_or_default(),
        }
    }

    /// The 1.8 wire form (count truncated to a byte like Java's `writeByte`).
    #[must_use]
    pub fn to_legacy(&self) -> legacy::ItemStack {
        legacy::ItemStack {
            // An id outside i16 cannot come from a 1.8 server; -1 would encode "empty".
            id: i16::try_from(self.id)
                .ok()
                .filter(|&i| i != -1)
                .unwrap_or(0),
            count: self.count as i8,
            damage: self.damage,
            nbt: self.nbt().cloned(),
        }
    }

    /// The 1.16.5 wire form.
    #[must_use]
    pub fn to_modern(&self) -> modern::ItemStack {
        modern::ItemStack {
            id: self.id,
            count: self.count as i8,
            nbt: self.nbt().cloned(),
        }
    }

    /// Same item, damage and data (vanilla's item + damage check and its NBT check; 26.1
    /// `isSameItemSameComponents`); counts are not compared.
    #[must_use]
    pub fn same_item(&self, other: &Stack) -> bool {
        self.id == other.id && self.damage == other.damage && self.data_eq(other)
    }

    /// Vanilla's data comparison: NBT both absent, or both present and equal as trees; 26.1
    /// component patches equal as maps.
    #[must_use]
    pub fn data_eq(&self, other: &Stack) -> bool {
        match (&self.data, &other.data) {
            (ItemData::None, ItemData::None) => true,
            (ItemData::Nbt(a), ItemData::Nbt(b)) => nbt_eq(Some(a), Some(b)),
            #[cfg(feature = "v26_1")]
            (ItemData::Components(a), ItemData::Components(b)) => a.same_components(b),
            #[cfg(feature = "v26_1")]
            (ItemData::Components(c), ItemData::None)
            | (ItemData::None, ItemData::Components(c)) => c.is_empty(),
            _ => false,
        }
    }

    /// Same as [`Stack::data_eq`] (kept from 0.3, where item data was always NBT).
    #[must_use]
    pub fn nbt_eq(&self, other: &Stack) -> bool {
        self.data_eq(other)
    }

    /// Exact equality as the server checks the clicked item (count, id, damage, NBT).
    #[must_use]
    pub fn equals_exact(&self, other: &Stack) -> bool {
        self.count == other.count && self.same_item(other)
    }

    /// `getBoolean(key)` of the root compound: a numeric tag whose byte value is not zero.
    pub(crate) fn nbt_bool(&self, key: &str) -> bool {
        self.nbt_number(key).is_some_and(|v| v as i8 != 0)
    }

    /// `getInt(key)` of the root compound (0 when absent or not numeric).
    pub(crate) fn nbt_int(&self, key: &str) -> i32 {
        self.nbt_number(key).unwrap_or(0) as i32
    }

    fn nbt_number(&self, key: &str) -> Option<i64> {
        let tree = self.nbt()?.parse().ok()??;
        let c = tree.tag.as_compound()?;
        number(last(c, key)?)
    }

    /// 1.16.5 `EnchantmentHelper.hasBindingCurse`: an `Enchantments` entry with id
    /// `minecraft:binding_curse` and `lvl` > 0.
    pub(crate) fn has_binding_curse(&self) -> bool {
        let Some(Ok(Some(tree))) = self.nbt().map(RawNbt::parse) else {
            return false;
        };
        let Some(Tag::List(list)) = tree.tag.as_compound().and_then(|c| last(c, "Enchantments"))
        else {
            return false;
        };
        for e in &list.items {
            let Tag::Compound(e) = e else { return false };
            let id = match last(e, "id") {
                Some(Tag::String(s)) => s.as_str(),
                _ => "",
            };
            if id == "minecraft:binding_curse" || id == "binding_curse" {
                // getLevel returns the first matching entry, clamped to 0..=255.
                return last(e, "lvl").and_then(number).unwrap_or(0) as i32 > 0;
            }
        }
        false
    }
}

impl Stack {
    /// 1.12 `EnchantmentHelper.hasBindingCurse`: the first `ench` entry whose short `id` is 10
    /// (binding curse) decides, by its short `lvl` > 0.
    pub(crate) fn has_binding_curse_1_12(&self) -> bool {
        let Some(Ok(Some(tree))) = self.nbt().map(RawNbt::parse) else {
            return false;
        };
        let Some(Tag::List(list)) = tree.tag.as_compound().and_then(|c| last(c, "ench")) else {
            return false;
        };
        for e in &list.items {
            // `getList("ench", 10)`: a list of anything but compounds reads as empty
            let Tag::Compound(e) = e else { return false };
            let short = |k: &str| last(e, k).and_then(number).unwrap_or(0) as i16;
            if short("id") == 10 {
                return short("lvl") > 0;
            }
        }
        false
    }
}

/// The value of the last entry named `key` (a Java `HashMap` keeps the last duplicate).
pub(crate) fn last<'a>(c: &'a ghast_protocol::nbt::Compound, key: &str) -> Option<&'a Tag> {
    c.0.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
}

pub(crate) fn number(t: &Tag) -> Option<i64> {
    Some(match *t {
        Tag::Byte(v) => i64::from(v),
        Tag::Short(v) => i64::from(v),
        Tag::Int(v) => i64::from(v),
        Tag::Long(v) => v,
        // NbtFloat/NbtDouble int and byte values are `MathHelper.floor(value)` (then `& 0xFF`).
        Tag::Float(v) => {
            let i = v as i32; // Java (int): saturating, NaN -> 0
            i64::from(if v < i as f32 { i - 1 } else { i })
        }
        Tag::Double(v) => {
            let i = v as i32;
            i64::from(if v < f64::from(i) { i - 1 } else { i })
        }
        _ => return None,
    })
}

/// Vanilla tag equality on raw NBT: equal bytes are equal; otherwise both trees are parsed and
/// compared (compounds as maps, root names ignored). Unparsable NBT only equals itself.
#[must_use]
pub fn nbt_eq(a: Option<&RawNbt>, b: Option<&RawNbt>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            if a.as_bytes() == b.as_bytes() {
                return true;
            }
            match (a.parse(), b.parse()) {
                (Ok(Some(x)), Ok(Some(y))) => tag_eq(&x.tag, &y.tag),
                (Ok(None), Ok(None)) => true,
                _ => false,
            }
        }
        _ => false,
    }
}

fn tag_eq(a: &Tag, b: &Tag) -> bool {
    match (a, b) {
        (Tag::Compound(x), Tag::Compound(y)) => {
            // Map semantics: the last value of each key, same key set.
            let distinct = |c: &ghast_protocol::nbt::Compound| {
                c.0.iter()
                    .enumerate()
                    .filter(|(i, (k, _))| !c.0[i + 1..].iter().any(|(k2, _)| k2 == k))
                    .count()
            };
            if distinct(x) != distinct(y) {
                return false;
            }
            x.0.iter().enumerate().all(|(i, (k, v))| {
                if x.0[i + 1..].iter().any(|(k2, _)| k2 == k) {
                    return true; // a later duplicate decides
                }
                last(y, k).is_some_and(|w| tag_eq(v, w))
            })
        }
        (Tag::List(x), Tag::List(y)) => {
            x.elem_type == y.elem_type
                && x.items.len() == y.items.len()
                && x.items.iter().zip(&y.items).all(|(v, w)| tag_eq(v, w))
        }
        // Java `==` on float/double values: NaN != NaN, 0.0 == -0.0.
        (Tag::Float(x), Tag::Float(y)) => x == y,
        (Tag::Double(x), Tag::Double(y)) => x == y,
        _ => a == b,
    }
}

/// `Option<&Stack>` exact equality (the server's `ItemStack.equalsAll`).
#[must_use]
pub fn stacks_equal(a: Option<&Stack>, b: Option<&Stack>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.equals_exact(b),
        _ => false,
    }
}

/// Converts a 1.8 wire slot.
#[must_use]
pub fn from_legacy_slot(s: legacy::Slot) -> Option<Stack> {
    s.0.map(Stack::from_legacy)
}

/// Converts a 1.16.5 wire slot; stacks that vanilla treats as empty (air, count <= 0) are `None`.
#[must_use]
pub fn from_modern_slot(s: modern::Slot) -> Option<Stack> {
    s.0.map(Stack::from_modern)
        .filter(|s| s.id != 0 && s.count > 0)
}

/// The 1.8 wire slot of an optional stack.
#[must_use]
pub fn to_legacy_slot(s: Option<&Stack>) -> legacy::Slot {
    legacy::Slot(s.map(Stack::to_legacy))
}

/// The 1.16.5 wire slot of an optional stack.
#[must_use]
pub fn to_modern_slot(s: Option<&Stack>) -> modern::Slot {
    modern::Slot(s.map(Stack::to_modern))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ghast_protocol::nbt::{Compound, NamedTag};

    fn raw(entries: Vec<(&str, Tag)>, root: &str) -> RawNbt {
        RawNbt::from_named(&NamedTag {
            name: root.into(),
            tag: Tag::Compound(Compound(
                entries
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            )),
        })
        .unwrap()
    }

    #[test]
    fn nbt_equality_is_tree_equality() {
        let a = raw(vec![("a", Tag::Int(1)), ("b", Tag::Byte(2))], "");
        let b = raw(vec![("b", Tag::Byte(2)), ("a", Tag::Int(1))], "");
        let c = raw(vec![("a", Tag::Int(1)), ("b", Tag::Byte(3))], "");
        let d = raw(vec![("a", Tag::Int(1)), ("b", Tag::Byte(2))], "root");
        assert!(nbt_eq(Some(&a), Some(&a)));
        assert!(nbt_eq(Some(&a), Some(&b)));
        assert!(!nbt_eq(Some(&a), Some(&c)));
        assert!(nbt_eq(Some(&a), Some(&d)));
        assert!(!nbt_eq(Some(&a), None));
        let empty = raw(vec![], "");
        assert!(!nbt_eq(Some(&empty), None));
        let dup = raw(
            vec![("a", Tag::Int(5)), ("a", Tag::Int(1)), ("b", Tag::Byte(2))],
            "",
        );
        assert!(nbt_eq(Some(&a), Some(&dup)));
        // Java float `==`: NaN is never equal (only reached when the bytes differ).
        let nan = raw(vec![("f", Tag::Float(f32::NAN)), ("g", Tag::Byte(0))], "");
        let nan_reordered = raw(vec![("g", Tag::Byte(0)), ("f", Tag::Float(f32::NAN))], "");
        assert!(!nbt_eq(Some(&nan), Some(&nan_reordered)));
    }

    #[test]
    fn nbt_helpers() {
        let mut s = Stack::new(1, 1, 0);
        assert!(!s.nbt_bool("Unbreakable"));
        s.data = ItemData::Nbt(raw(
            vec![("Unbreakable", Tag::Byte(1)), ("Damage", Tag::Int(7))],
            "",
        ));
        assert!(s.nbt_bool("Unbreakable"));
        assert_eq!(s.nbt_int("Damage"), 7);
        s.data = ItemData::Nbt(raw(vec![("Unbreakable", Tag::Int(256))], ""));
        assert!(!s.nbt_bool("Unbreakable"), "(byte) 256 == 0");
        let ench = |id: &str, lvl: i16| {
            Tag::List(ghast_protocol::nbt::List {
                elem_type: 10,
                items: vec![Tag::Compound(Compound(vec![
                    ("id".into(), Tag::String(id.into())),
                    ("lvl".into(), Tag::Short(lvl)),
                ]))],
            })
        };
        s.data = ItemData::Nbt(raw(
            vec![("Enchantments", ench("minecraft:binding_curse", 1))],
            "",
        ));
        assert!(s.has_binding_curse());
        s.data = ItemData::Nbt(raw(
            vec![("Enchantments", ench("minecraft:binding_curse", 0))],
            "",
        ));
        assert!(!s.has_binding_curse());
        s.data = ItemData::Nbt(raw(
            vec![("StoredEnchantments", ench("minecraft:binding_curse", 1))],
            "",
        ));
        assert!(!s.has_binding_curse());
        // Garbage NBT never panics.
        s.data = ItemData::from_nbt(RawNbt::from_bytes(Bytes::from_static(&[0x0a, 0, 0, 0])).ok());
        assert!(!s.has_binding_curse());
    }

    #[test]
    fn wire_round_trip() {
        let s = Stack::new(276, 1, 5);
        assert_eq!(Stack::from_legacy(s.to_legacy()), s);
        let m = Stack::new(600, 64, 0);
        assert_eq!(Stack::from_modern(m.to_modern()), m);
        assert_eq!(
            from_modern_slot(modern::Slot(Some(modern::ItemStack {
                id: 1,
                count: 0,
                nbt: None
            }))),
            None
        );
        assert_eq!(
            from_modern_slot(modern::Slot(Some(modern::ItemStack {
                id: 0,
                count: 5,
                nbt: None
            }))),
            None
        );
        assert!(
            from_legacy_slot(legacy::Slot(Some(legacy::ItemStack {
                id: 1,
                count: 0,
                damage: 0,
                nbt: None
            })))
            .is_some()
        );
        assert_eq!(Stack::new(1, 300, 0).to_legacy().count, 44);
    }
}
