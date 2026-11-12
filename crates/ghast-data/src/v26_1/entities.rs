//! Minecraft 26.1 entity types: ids, names, the client instance's dimensions per variant and pose,
//! and the per-type facts an entity tracker needs (living, interpolation, pickable, the synched
//! data indices that select a size variant).
//!
//! Everything but display names and categories comes from the real 26.1 jar (oracle dump
//! `tests/golden/26.1/entity_sizes.jsonl.gz`; minecraft-data `pc/26.1` is cross-checked for ids and
//! names at generation time).
//!
//! Dimensions as the client computes them ([`EntityTypeData::dimensions`]): the variant's size
//! (`"default"`, `"baby"`, slime / magma cube size `n` = the type's size scaled by `n`, armor stand
//! `"small"` / `"marker"` / `"small_marker"`), the pose's override where one exists (camel sitting,
//! goat long jumping, player poses, ...), a living entity's sleeping pose is
//! [`LIVING_SLEEPING`], and a living entity's `minecraft:scale` attribute scales the result unless
//! the dimensions are fixed. Not modelled (the default size is returned): phantom size,
//! pufferfish puff state; no box at all for the types flagged [`type_flags::NO_BOX`].

use super::g;
use crate::common::{EntityInfo, VanillaEntitySize, lookup_name};
pub use crate::modern::{EntityTypeData, LIVING_SLEEPING, POSES, type_flags};

/// Network id of `minecraft:player`.
pub const PLAYER: u16 = g::entities::ID_PLAYER;
/// Network id of `minecraft:item`.
pub const ITEM: u16 = g::entities::ID_ITEM;
/// Network id of `minecraft:experience_orb`.
pub const EXPERIENCE_ORB: u16 = g::entities::ID_EXPERIENCE_ORB;
/// Network id of `minecraft:painting`.
pub const PAINTING: u16 = g::entities::ID_PAINTING;
/// Network id of `minecraft:armor_stand`.
pub const ARMOR_STAND: u16 = g::entities::ID_ARMOR_STAND;

/// All entity types by id.
#[must_use]
pub fn entities() -> &'static [EntityInfo] {
    &g::entities::ENTITIES
}

/// The entity type with network id `id`.
#[must_use]
pub fn entity(id: u16) -> Option<&'static EntityInfo> {
    g::entities::ENTITIES.get(usize::from(id))
}

/// Entity type by registry path (`"zombie"`).
#[must_use]
pub fn entity_by_name(name: &str) -> Option<&'static EntityInfo> {
    lookup_name(
        g::entities::ENTITY_NAMES_TEXT,
        &g::entities::ENTITY_NAMES,
        name,
    )
    .map(|i| &g::entities::ENTITIES[usize::from(i)])
}

/// The client facts of type `id` (O(1)).
#[inline]
#[must_use]
pub fn entity_type(id: u16) -> Option<&'static EntityTypeData> {
    g::entities::TYPES.get(usize::from(id))
}

/// All per-type facts by id.
#[must_use]
pub fn entity_types() -> &'static [EntityTypeData] {
    &g::entities::TYPES
}

/// Measured client sizes, sorted by name.
#[must_use]
pub fn vanilla_entity_sizes() -> &'static [VanillaEntitySize] {
    &g::entities::VANILLA_ENTITY_SIZES
}

/// Measured client sizes by registry path.
#[must_use]
pub fn vanilla_entity_size(name: &str) -> Option<&'static VanillaEntitySize> {
    g::entities::VANILLA_ENTITY_SIZES
        .binary_search_by(|e| e.name.cmp(name))
        .ok()
        .map(|i| &g::entities::VANILLA_ENTITY_SIZES[i])
}
