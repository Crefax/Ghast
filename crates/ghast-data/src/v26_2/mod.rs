//! Minecraft 26.2 (protocol 776) data: the global block-state palette with properties, collision
//! voxel shapes and the per-state facts the movement code reads, items and entities.
//!
//! The same API as [`crate::v26_1`] (the shared items are the same Rust types: [`StatePhys`],
//! [`BlockKind`], [`VoxelShapeRef`], ...; the shape tables are one pool shared by both). Every
//! value comes from the real 26.2 jar (oracle dumps in `tests/golden/26.2/`): ids, names,
//! state layout and every movement fact. minecraft-data has no 26.2 yet: display fields
//! (display names, resistance, light) are minecraft-data's 26.1 values by name, and the
//! blocks, items and entity new in 26.2 take them from a reviewed table
//! (`codegen/data/src/v26_2.rs`).
//!
//! Unlike 26.1 the per-block and per-item rows hold no pointers ([`BlockData`] and
//! [`ItemData`] have no `name` / `display_name` fields): use [`Block::name`],
//! [`Block::display_name`], [`Item::name`], [`Item::display_name`], which exist in `v26_1`
//! too, so code written against those works with both versions.
//!
//! The hot path is one table read per state: [`BlockState::phys`] returns a packed
//! [`StatePhys`] (shape index, flags, fluid, per-kind extra bits, [`BlockKind`]).

use crate::common::{SelectionShape, lookup_name};
use crate::generated::v26_2 as g;

pub use crate::modern::{
    BlockKind, StatePhys, VoxelClass, VoxelShapeRef, block_flags, state_flags,
};
use core::fmt;

pub use g::blocks::Property;

// The 26.1 source, compiled over the 26.2 tables.
#[allow(clippy::duplicate_mod)]
#[path = "../v26_1/dig.rs"]
pub mod dig;
// The 26.1 source, compiled over the 26.2 tables.
#[allow(clippy::duplicate_mod)]
#[path = "../v26_1/entities.rs"]
pub mod entities;
// The 26.1 source, compiled over the 26.2 tables.
#[allow(clippy::duplicate_mod)]
#[path = "../v26_1/items.rs"]
pub mod items;

pub use entities::{
    EntityTypeData, entities, entity, entity_by_name, entity_type, entity_types,
    vanilla_entity_size, vanilla_entity_sizes,
};
pub use items::{BlockSet, Food, Item, Tool, ToolRule, food};

/// Static per-block data (jar values for everything movement reads).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockData {
    /// Destroy time (minecraft-data; `None` = unbreakable).
    pub hardness: Option<f32>,
    /// Explosion resistance (minecraft-data).
    pub resistance: f32,
    /// Can be mined (minecraft-data).
    pub diggable: bool,
    /// Lets light through (minecraft-data).
    pub transparent: bool,
    /// Light emitted (minecraft-data).
    pub emit_light: u8,
    /// Light filtered (minecraft-data).
    pub filter_light: u8,
    /// `Block.getFriction()` (jar).
    pub friction: f32,
    /// `Block.getSpeedFactor()` (jar).
    pub speed_factor: f32,
    /// `Block.getJumpFactor()` (jar).
    pub jump_factor: f32,
    /// [`block_flags`] bits.
    pub flags: u32,
    /// Special movement behaviour.
    pub kind: BlockKind,
    /// Model offset function: `(max horizontal, max vertical, xyz)`; bamboo and pointed dripstone
    /// shift their collision shape by it.
    pub offset: Option<(f32, f32, bool)>,
}

/// String `i` of a text table (`ends` = end offsets).
fn nth(text: &'static str, ends: &[u32], i: u16) -> &'static str {
    let i = usize::from(i);
    let end = ends[i] as usize;
    let start = if i == 0 { 0 } else { ends[i - 1] as usize };
    &text[start..end]
}

/// Registry name of block `id` (no namespace).
fn block_name(id: u16) -> &'static str {
    nth(
        g::blocks::BLOCK_ID_NAMES_TEXT,
        &g::blocks::BLOCK_ID_NAMES_ENDS,
        id,
    )
}

/// English display name of block `id`.
#[cfg(feature = "display-names")]
fn block_display_name(id: u16) -> &'static str {
    nth(
        g::blocks::BLOCK_DISPLAY_NAMES_TEXT,
        &g::blocks::BLOCK_DISPLAY_NAMES_ENDS,
        id,
    )
}

/// Static item data (default components).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemData {
    /// Default `minecraft:max_stack_size`.
    pub max_stack_size: u8,
    /// Default `minecraft:max_damage` (damageable items).
    pub max_damage: Option<u16>,
    /// The block a `BlockItem` places (block id).
    pub block: Option<u16>,
    /// Default `minecraft:food`.
    pub food: Option<Food>,
    /// Default `minecraft:tool` (index into the tool table).
    pub tool: Option<u16>,
}

/// Registry name of item `id` (no namespace).
fn item_name(id: u16) -> &'static str {
    nth(
        g::items::ITEM_ID_NAMES_TEXT,
        &g::items::ITEM_ID_NAMES_ENDS,
        id,
    )
}

/// English display name of item `id`.
#[cfg(feature = "display-names")]
fn item_display_name(id: u16) -> &'static str {
    nth(
        g::items::ITEM_DISPLAY_NAMES_TEXT,
        &g::items::ITEM_DISPLAY_NAMES_ENDS,
        id,
    )
}

// The API shared with the other modern versions (`v26_2` compiles the same source).
include!("../modern_api.rs");

#[cfg(test)]
mod tests;
