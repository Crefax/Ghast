//! Per-version item facts the click logic needs: stack limits, damageability, which items
//! have subtypes (1.8, 1.12), which go into armor / offhand slots, and which crafting ingredients
//! leave something behind.
//!
//! Stack sizes and durability come from `ghast-data` (checked against the vanilla 1.8.9, 1.12.2
//! and 1.16.5 item registries; the two 1.12.2 differences are fixed below); the short lists below were read off the vanilla registries
//! (the damage-variant flag, which Legacy Yarn names `Item.isUnbreakable`; `getRecipeRemainder`;
//! `MobEntity.getPreferredEquipmentSlot`).

use crate::stack::Stack;

/// Protocol version of the connection; selects the vanilla rules being reproduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Version {
    /// 1.8 – 1.8.9 (protocol 47).
    V1_8,
    /// 1.12 – 1.12.2 (protocol 335 – 340).
    V1_12,
    /// 1.16.4 – 1.16.5 (protocol 754).
    V1_16_5,
    /// 26.1 – 26.1.2 (protocol 775).
    V26_1,
}

/// [`crate::Inventory::new`] for a [`Version`] whose crate feature (`v1_8`, `v1_12`, `v1_16_5`,
/// `v26_1`) is off in this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct VersionNotEnabled(pub Version);

impl std::fmt::Display for VersionNotEnabled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ghast-inventory was built without the feature of {:?}",
            self.0
        )
    }
}

impl std::error::Error for VersionNotEnabled {}

/// 1.8.9 items with damage variants (Legacy Yarn `Item.isUnbreakable()` is true; the damage value selects a variant, so merging
/// compares it). Sorted.
const SUBTYPES_1_8: &[i32] = &[
    1, 3, 5, 6, 12, 17, 18, 19, 24, 31, 35, 37, 38, 44, 78, 95, 97, 98, 126, 139, 145, 155, 159,
    160, 161, 162, 168, 171, 175, 179, 182, 263, 322, 349, 350, 351, 358, 373, 383, 397, 425,
];

/// 1.12.2 items with damage variants (`Item.isUnbreakable()` in Legacy Yarn, read off the vanilla
/// 1.12.2 item registry). Sorted.
const SUBTYPES_1_12: &[i32] = &[
    1, 3, 5, 6, 12, 17, 18, 19, 24, 31, 35, 37, 38, 44, 95, 97, 98, 126, 139, 145, 155, 159, 160,
    161, 162, 168, 171, 175, 179, 182, 205, 251, 252, 263, 322, 349, 350, 351, 355, 358, 397, 425,
];

/// Equipment slot index used by every version: 0 head, 1 chest, 2 legs, 3 feet. Window 0 slot
/// `5 + index` is that armor slot.
pub(crate) type ArmorIndex = u8;

impl Version {
    /// Whether this crate was built with the version's feature (`v1_8`, `v1_12`, `v1_16_5`,
    /// `v26_1`). An [`Inventory`](crate::Inventory) can only be created for enabled versions.
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        match self {
            Version::V1_8 => cfg!(feature = "v1_8"),
            Version::V1_12 => cfg!(feature = "v1_12"),
            Version::V1_16_5 => cfg!(feature = "v1_16_5"),
            Version::V26_1 => cfg!(feature = "v26_1"),
        }
    }

    /// Item facts from `ghast-data` (1.8 – 1.16.5; 26.1 items come from [`crate::v26_1`]).
    #[allow(unused_variables)]
    fn info(self, id: i32) -> Option<&'static ghast_data::ItemInfo> {
        let id = u16::try_from(id).ok()?;
        match self {
            #[cfg(feature = "v1_8")]
            Version::V1_8 => ghast_data::v1_8::Item::from_id(id).map(|i| i.info()),
            #[cfg(feature = "v1_12")]
            Version::V1_12 => ghast_data::v1_12::Item::from_id(id).map(|i| i.info()),
            #[cfg(feature = "v1_16_5")]
            Version::V1_16_5 => ghast_data::v1_16_5::Item::from_id(id).map(|i| i.info()),
            _ => None,
        }
    }

    /// Registry name of an item id (`""` when unknown).
    pub(crate) fn name(self, id: i32) -> &'static str {
        #[cfg(feature = "v26_1")]
        if self == Version::V26_1 {
            return crate::v26_1::item_name(id).unwrap_or("");
        }
        self.info(id).map_or("", |i| i.name)
    }

    /// Whether the item id exists in this version.
    pub(crate) fn known(self, id: i32) -> bool {
        #[cfg(feature = "v26_1")]
        if self == Version::V26_1 {
            return crate::v26_1::row(id).is_some();
        }
        self.info(id).is_some()
    }

    /// `Item.getMaxCount()` (26.1: the stack's `max_stack_size` component). Unknown ids get 64.
    pub(crate) fn max_count(self, s: &Stack) -> i32 {
        #[cfg(feature = "v26_1")]
        if self == Version::V26_1 {
            return crate::v26_1::max_stack_size(s);
        }
        if self == Version::V1_16_5 && s.id == 843 {
            // warped_fungus_on_a_stick: minecraft-data says 64, vanilla 1.16.5 says 1.
            return 1;
        }
        if self == Version::V1_12 && matches!(s.id, 453 | 2256) {
            // knowledge_book and record_13: minecraft-data says 64, vanilla 1.12.2 says 1.
            return 1;
        }
        self.info(s.id).map_or(64, |i| i32::from(i.stack_size))
    }

    /// Durability from `ghast-data` (1.8 – 1.16.5).
    pub(crate) fn max_durability(self, id: i32) -> Option<i32> {
        self.info(id)
            .and_then(|i| i.max_durability)
            .map(i32::from)
            .filter(|&d| d > 0)
    }

    /// Whether the item has damage variants (1.8 / 1.12; 1.16 has no damage-selected variants).
    pub(crate) fn has_subtypes(self, id: i32) -> bool {
        match self {
            Version::V1_8 => SUBTYPES_1_8.binary_search(&id).is_ok(),
            Version::V1_12 => SUBTYPES_1_12.binary_search(&id).is_ok(),
            Version::V1_16_5 | Version::V26_1 => false,
        }
    }

    /// `ItemStack.isDamageable()`: the item has durability and the stack is not `Unbreakable`.
    pub(crate) fn is_damageable(self, s: &Stack) -> bool {
        #[cfg(feature = "v26_1")]
        if self == Version::V26_1 {
            return crate::v26_1::is_damageable(s);
        }
        self.info(s.id)
            .and_then(|i| i.max_durability)
            .is_some_and(|d| d > 0)
            && !s.nbt_bool("Unbreakable")
    }

    /// `ItemStack.isDamaged()` (1.8 / 1.12: damage value; 1.16: `Damage` tag).
    pub(crate) fn is_damaged(self, s: &Stack) -> bool {
        self.is_damageable(s)
            && match self {
                Version::V1_8 | Version::V1_12 => s.damage > 0,
                Version::V1_16_5 => s.nbt_int("Damage") > 0,
                Version::V26_1 => damage_26(s) > 0,
            }
    }

    /// `ItemStack.isStackable()`.
    pub(crate) fn is_stackable(self, s: &Stack) -> bool {
        self.max_count(s) > 1 && (!self.is_damageable(s) || !self.is_damaged(s))
    }

    /// The armor slot an `ArmorItem` belongs to (1.8: `ArmorItem.slot`; 1.16: the preferred
    /// equipment slot when it is an armor slot, which includes elytra, carved pumpkins and
    /// heads). Used by window 0's shift-click.
    pub(crate) fn armor_target(self, s: &Stack) -> Option<ArmorIndex> {
        match self {
            Version::V1_8 => (298..=317)
                .contains(&s.id)
                .then(|| ((s.id - 298) % 4) as u8),
            Version::V1_12 => equipment_1_12(s.id).filter(|&e| e < 4),
            Version::V1_16_5 => self.equipment_1_16(s.id).filter(|&e| e < 4),
            Version::V26_1 => equip_26(s).and_then(|e| e.filter(|&e| e < 4)),
        }
    }

    /// Whether window 0's armor slot `a` accepts the stack (`Slot.canInsert`).
    pub(crate) fn armor_accepts(self, a: ArmorIndex, s: &Stack) -> bool {
        match self {
            Version::V1_8 => {
                if (298..=317).contains(&s.id) {
                    ((s.id - 298) % 4) as u8 == a
                } else {
                    // pumpkin (block item 86) and skull (397) fit the helmet slot
                    (s.id == 86 || s.id == 397) && a == 0
                }
            }
            Version::V1_12 => equipment_1_12(s.id) == Some(a),
            Version::V1_16_5 => self.equipment_1_16(s.id) == Some(a),
            Version::V26_1 => equip_26(s) == Some(Some(a)),
        }
    }

    /// 1.12 / 1.16.5 shift-click target "offhand" (`getPreferredEquipmentSlot == OFFHAND`: the
    /// shield).
    pub(crate) fn prefers_offhand(self, s: &Stack) -> bool {
        match self {
            Version::V1_8 => false,
            Version::V1_12 => equipment_1_12(s.id) == Some(4),
            Version::V1_16_5 => self.equipment_1_16(s.id) == Some(4),
            Version::V26_1 => equip_26(s) == Some(Some(4)),
        }
    }

    /// 1.16.5 `MobEntity.getPreferredEquipmentSlot`: 0..=3 armor (head, chest, legs, feet),
    /// 4 offhand, `None` main hand.
    fn equipment_1_16(self, id: i32) -> Option<u8> {
        let n = self.name(id);
        if n == "carved_pumpkin" || n.ends_with("_skull") || n.ends_with("_head") {
            // every *_skull / *_head item is an AbstractSkullBlock item in 1.16.5
            return Some(0);
        }
        if n == "elytra" {
            return Some(1);
        }
        if n == "shield" {
            return Some(4);
        }
        // ArmorItem: leather/chainmail/iron/golden/diamond/netherite pieces and turtle_helmet
        if n.ends_with("_helmet") {
            Some(0)
        } else if n.ends_with("_chestplate") {
            Some(1)
        } else if n.ends_with("_leggings") {
            Some(2)
        } else if n.ends_with("_boots") {
            Some(3)
        } else {
            None
        }
    }

    /// 1.12 / 1.16.5 shulker box slots refuse shulker boxes.
    pub(crate) fn is_shulker_box(self, s: &Stack) -> bool {
        self != Version::V1_8 && self.name(s.id).ends_with("shulker_box")
    }
}

/// 26.1 `ItemStack.getDamageValue()`.
fn damage_26(s: &Stack) -> i32 {
    #[cfg(feature = "v26_1")]
    return crate::v26_1::damage_value(s);
    #[cfg(not(feature = "v26_1"))]
    {
        let _ = s;
        0
    }
}

/// 26.1 equipment slot of the `equippable` component as 0..=3 armor, 4 offhand; `None`
/// (outer) when the stack patches `equippable` (unknown to ghast).
fn equip_26(s: &Stack) -> Option<Option<u8>> {
    #[cfg(feature = "v26_1")]
    {
        use crate::v26_1::Equip;
        let e = crate::v26_1::equip(s).ok()?;
        Some(match e {
            Equip::Offhand => Some(4),
            e => e.armor_index(),
        })
    }
    #[cfg(not(feature = "v26_1"))]
    {
        let _ = s;
        None
    }
}

/// 1.12.2 `MobEntity.method_13083` (preferred equipment slot): pumpkin and skull on the head,
/// armor pieces by type, elytra on the chest, the shield in the offhand. 0..=3 armor (head,
/// chest, legs, feet), 4 offhand, `None` main hand.
fn equipment_1_12(id: i32) -> Option<u8> {
    match id {
        86 | 397 => Some(0),
        298..=317 => Some(((id - 298) % 4) as u8),
        443 => Some(1),
        442 => Some(4),
        _ => None,
    }
}

#[cfg(all(test, feature = "v1_8", feature = "v1_16_5"))]
mod tests {
    use super::*;

    #[test]
    fn tables_sorted_and_facts() {
        assert!(SUBTYPES_1_8.windows(2).all(|w| w[0] < w[1]));
        assert!(SUBTYPES_1_12.windows(2).all(|w| w[0] < w[1]));
        let v = Version::V1_8;
        assert!(v.has_subtypes(351)); // dye
        assert!(!v.has_subtypes(276)); // diamond sword
        assert_eq!(v.max_count(&Stack::new(276, 1, 0)), 1);
        assert_eq!(v.max_count(&Stack::new(368, 1, 0)), 16); // ender pearl
        assert_eq!(v.armor_target(&Stack::new(310, 1, 0)), Some(0));
        assert_eq!(v.armor_target(&Stack::new(317, 1, 0)), Some(3));
        assert!(v.armor_accepts(0, &Stack::new(86, 1, 0)));
        assert!(v.is_damaged(&Stack::new(276, 1, 3)));
        assert!(!v.is_stackable(&Stack::new(276, 1, 3)));
        let w = Version::V1_16_5;
        let id = |n: &str| i32::from(ghast_data::v1_16_5::Item::from_name(n).unwrap().id());
        assert_eq!(w.armor_target(&Stack::new(id("elytra"), 1, 0)), Some(1));
        assert_eq!(
            w.armor_target(&Stack::new(id("turtle_helmet"), 1, 0)),
            Some(0)
        );
        assert_eq!(
            w.armor_target(&Stack::new(id("dragon_head"), 1, 0)),
            Some(0)
        );
        assert_eq!(
            w.armor_target(&Stack::new(id("netherite_boots"), 1, 0)),
            Some(3)
        );
        assert_eq!(w.armor_target(&Stack::new(id("shield"), 1, 0)), None);
        assert!(w.prefers_offhand(&Stack::new(id("shield"), 1, 0)));
        assert!(w.is_shulker_box(&Stack::new(id("red_shulker_box"), 1, 0)));
        assert_eq!(w.max_count(&Stack::new(843, 1, 0)), 1);
        assert_eq!(w.name(843), "warped_fungus_on_a_stick");
    }
}
