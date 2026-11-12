//! 26.1 (protocol 775) items: default components of every item (Mojang's `--reports` component
//! dump, see `examples/gen_items_26_1.rs`), readers for the components the click rules and the
//! item accessors use, the window types, and the hashed stacks a 26.1 client sends with every
//! click (vanilla `HashedStack` / `HashedPatchMap` over `HashOps.CRC32C_INSTANCE`).
//!
//! The functions here are the 26.1 rows of [`crate::modern`], which takes the version as an
//! argument and covers every modern version.

use bytes::Bytes;

use crate::stack::Stack;
use crate::tables::Mv;
use crate::window::WindowKind;

pub use crate::item26::{
    Equip, Food, HashRegistries, ItemDefaults, ItemUse, UNKNOWN_HASH, UseAnimation,
    enchantment_specs,
};
pub use crate::tables::g261::{
    ATTRIBUTES, COMPONENT_NAMES, ITEM_COUNT, MENUS, POTIONS, SOUND_EVENTS, VILLAGER_TYPES,
    component,
};

const M: Mv = Mv::V26_1;

/// Registry name of item `id` without `minecraft:` (`"diamond_sword"`).
#[must_use]
pub fn item_name(id: i32) -> Option<&'static str> {
    crate::item26::item_name(M, id)
}

/// Default components of item `id`.
///
/// The table of these is built on the first call (ghast itself reads its compact,
/// relocation-free table; see [`item_name`] for the name alone).
#[must_use]
pub fn defaults(id: i32) -> Option<&'static ItemDefaults> {
    crate::item26::defaults(M, id)
}

/// Item registry id of a name (`"diamond_sword"` or `"minecraft:diamond_sword"`).
#[must_use]
pub fn item_id(name: &str) -> Option<i32> {
    crate::item26::item_id(M, name)
}

/// Component type id of a name (`"damage"` or `"minecraft:damage"`).
#[must_use]
pub fn component_id(name: &str) -> Option<i32> {
    crate::item26::component_id(M, name)
}

/// The window type of a 26.1 `open_screen` (`minecraft:menu` registry id).
#[must_use]
pub fn window_kind(menu: i32) -> Option<WindowKind> {
    crate::item26::window_kind(M, menu)
}

/// The slot a glider works from: `Some(slot)` when the stack has the `glider` component (the
/// item default or patched in, not patched out) and an `equippable` component naming `slot`;
/// `None` otherwise. Durability is not looked at: see [`can_glide_using`].
#[must_use]
pub fn glider(s: &Stack) -> Option<Equip> {
    crate::item26::glider(M, s)
}

/// `ItemStack.nextDamageWillBreak()`: damageable (`max_damage` and `damage` present, not
/// `unbreakable`) and `damage >= max_damage - 1`.
#[must_use]
pub fn next_damage_will_break(s: &Stack) -> bool {
    crate::item26::next_damage_will_break(M, s)
}

/// `LivingEntity.canGlideUsing(stack, slot)`: the stack is a glider equipped in `slot` (see
/// [`glider`]) and its next damage will not break it.
#[must_use]
pub fn can_glide_using(s: &Stack, slot: Equip) -> bool {
    crate::item26::can_glide_using(M, s, slot)
}

/// [`ItemUse`] of a 26.1 stack (`None` for an id outside the item registry). Components the
/// stack patches but that do not read give the item defaults' values.
#[must_use]
pub fn item_use(s: &Stack) -> Option<ItemUse> {
    crate::item26::item_use(M, s)
}

/// The `HashOps` hash of one 26.1 component value as it appears on the wire (`None`: the type
/// is not covered, or a registry name it needs is unknown).
#[must_use]
pub fn component_hash(ty: i32, value: &Bytes, regs: &HashRegistries) -> Option<i32> {
    crate::item26::component_hash(ty, value, M, regs)
}

/// Whether ghast can hash 26.1 component `ty` at all (registry names permitting).
#[must_use]
pub fn hash_covers(ty: i32) -> bool {
    crate::item26::hash_covers(M, ty)
}

/// One slot as a 26.1 click sends it (vanilla `HashedStack.create`), with
/// [`UNKNOWN_HASH`] for components ghast cannot hash. The second value is `false` when such a
/// component was present.
///
/// Every component kind a client can receive is hashed (checked against the real client's
/// hashes of 4000 stacks in `tests/golden/26.1/hashed_stacks.jsonl.gz`). What still falls back
/// to [`UNKNOWN_HASH`]: a registry name the server did not send in configuration, an inline
/// holder value without a modelled direct form (trim materials/patterns, banner patterns,
/// instruments, jukebox songs, painting variants; inline sound events are modelled), and an
/// adventure-mode block predicate with an `nbt` test or partial component predicates.
#[must_use]
pub fn hashed_slot(
    s: Option<&Stack>,
    reg: &HashRegistries,
) -> (Option<ghast_protocol::defs::p775::types::HashedSlot>, bool) {
    let (h, exact) = crate::item26::hashed(M, s, reg);
    (h.map(crate::packets::hashed_775), exact)
}
