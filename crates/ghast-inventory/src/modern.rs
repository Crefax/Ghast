//! Modern (26.x) items for any modern [`Version`]: the item registry and default components
//! (Mojang's reports), the component readers, the window types and the component hashes a
//! modern click carries.
//!
//! One implementation serves every modern version; the version picks its tables (item and
//! component ids differ between versions, the rules do not). Every function returns `None` /
//! `false` for a legacy version or one whose feature is off. The `v26_1` module has the same
//! functions with the version fixed to 26.1.

use bytes::Bytes;

use crate::rules::Version;
use crate::stack::Stack;
use crate::tables::{Mv, Names};
use crate::window::WindowKind;

pub use crate::item26::{
    Equip, Food, HashRegistries, ItemDefaults, ItemUse, UNKNOWN_HASH, UseAnimation,
    enchantment_specs,
};

/// Registry name of item `id` without `minecraft:` (`"diamond_sword"`).
#[must_use]
pub fn item_name(v: Version, id: i32) -> Option<&'static str> {
    crate::item26::item_name(Mv::of(v)?, id)
}

/// Item registry id of a name (`"diamond_sword"` or `"minecraft:diamond_sword"`).
#[must_use]
pub fn item_id(v: Version, name: &str) -> Option<i32> {
    crate::item26::item_id(Mv::of(v)?, name)
}

/// Number of items (registry ids `0..item_count`); 0 for a legacy version.
#[must_use]
pub fn item_count(v: Version) -> usize {
    Mv::of(v).map_or(0, |m| m.item_rows().len())
}

/// Default components of item `id` (the table is built on the first call per version).
#[must_use]
pub fn defaults(v: Version, id: i32) -> Option<&'static ItemDefaults> {
    crate::item26::defaults(Mv::of(v)?, id)
}

/// Component type id of a name (`"damage"` or `"minecraft:damage"`) in this version.
#[must_use]
pub fn component_id(v: Version, name: &str) -> Option<i32> {
    crate::item26::component_id(Mv::of(v)?, name)
}

/// Component type name (without `minecraft:`) of a type id in this version.
#[must_use]
pub fn component_name(v: Version, id: i32) -> Option<&'static str> {
    Mv::of(v)?.name(Names::Component, usize::try_from(id).ok()?)
}

/// Number of data component types (26.1: 110, 26.2: 111); 0 for a legacy version.
#[must_use]
pub fn component_count(v: Version) -> usize {
    Mv::of(v).map_or(0, |m| m.names(Names::Component).1.len())
}

/// `minecraft:menu` registry name (without `minecraft:`) of an `open_screen` menu id.
#[must_use]
pub fn menu_name(v: Version, menu: i32) -> Option<&'static str> {
    Mv::of(v)?.name(Names::Menu, usize::try_from(menu).ok()?)
}

/// The window type of an `open_screen` (`minecraft:menu` registry id).
#[must_use]
pub fn window_kind(v: Version, menu: i32) -> Option<WindowKind> {
    crate::item26::window_kind(Mv::of(v)?, menu)
}

/// The slot a glider works from.
#[must_use]
pub fn glider(v: Version, s: &Stack) -> Option<Equip> {
    crate::item26::glider(Mv::of(v)?, s)
}

/// `ItemStack.nextDamageWillBreak()`.
#[must_use]
pub fn next_damage_will_break(v: Version, s: &Stack) -> bool {
    Mv::of(v).is_some_and(|m| crate::item26::next_damage_will_break(m, s))
}

/// `LivingEntity.canGlideUsing(stack, slot)`.
#[must_use]
pub fn can_glide_using(v: Version, s: &Stack, slot: Equip) -> bool {
    Mv::of(v).is_some_and(|m| crate::item26::can_glide_using(m, s, slot))
}

/// [`ItemUse`] of a stack.
#[must_use]
pub fn item_use(v: Version, s: &Stack) -> Option<ItemUse> {
    crate::item26::item_use(Mv::of(v)?, s)
}

/// The `HashOps` hash of one component value as it appears on the wire (`ty` is this
/// version's component type id; `None`: not covered, or a registry name it needs is unknown).
#[must_use]
pub fn component_hash(v: Version, ty: i32, value: &Bytes, regs: &HashRegistries) -> Option<i32> {
    crate::item26::component_hash(ty, value, Mv::of(v)?, regs)
}

/// Whether ghast can hash component type `ty` of this version at all.
#[must_use]
pub fn hash_covers(v: Version, ty: i32) -> bool {
    Mv::of(v).is_some_and(|m| crate::item26::hash_covers(m, ty))
}

/// Clientbound play packet ids [`crate::Inventory::apply_raw_modern`] handles (the ids are the
/// same in 26.1 and 26.2); empty for a legacy version.
#[must_use]
pub fn packet_ids(v: Version) -> &'static [i32] {
    match Mv::of(v) {
        Some(_) => crate::packets::PACKET_IDS_MODERN,
        None => &[],
    }
}
