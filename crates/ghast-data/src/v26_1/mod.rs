//! Minecraft 26.1 (protocol 775) data: the global block-state palette with properties, collision
//! voxel shapes and the per-state facts the movement code reads.
//!
//! Every movement-relevant value comes from the real 26.1 jar (oracle dump
//! `tests/golden/26.1/voxel_shapes.jsonl.gz`): collision voxel grids, per-state flags, fluids,
//! friction / speed / jump factors, the block tags and `instanceof` tests vanilla's movement code
//! makes. PrismarineJS minecraft-data (`pc/26.1`) supplies the property layout and display data
//! and is cross-checked against the jar at generation time.
//!
//! The hot path is one table read per state: [`BlockState::phys`] returns a packed
//! [`StatePhys`] (shape index, flags, fluid, per-kind extra bits, [`BlockKind`]).

use crate::common::{SelectionShape, lookup_name};
use crate::generated::v26_1 as g;

pub use crate::modern::{
    BlockKind, StatePhys, VoxelClass, VoxelShapeRef, block_flags, state_flags,
};
use core::fmt;

pub use g::blocks::Property;

pub mod dig;
pub mod entities;
pub mod items;

pub use entities::{
    EntityTypeData, entities, entity, entity_by_name, entity_type, entity_types,
    vanilla_entity_size, vanilla_entity_sizes,
};
pub use items::{BlockSet, Food, Item, Tool, ToolRule, food};

/// Static per-block data (jar values for everything movement reads).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockData {
    /// Registry name without namespace.
    pub name: &'static str,
    /// English display name (minecraft-data).
    #[cfg(feature = "display-names")]
    pub display_name: &'static str,
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

/// Registry name of block `id` (no namespace).
fn block_name(id: u16) -> &'static str {
    g::blocks::BLOCKS[usize::from(id)].name
}

/// English display name of block `id`.
#[cfg(feature = "display-names")]
fn block_display_name(id: u16) -> &'static str {
    g::blocks::BLOCKS[usize::from(id)].display_name
}

/// Static item data (default components).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemData {
    /// Registry name without namespace.
    pub name: &'static str,
    /// English display name (minecraft-data).
    #[cfg(feature = "display-names")]
    pub display_name: &'static str,
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
    g::items::ITEMS[usize::from(id)].name
}

/// English display name of item `id`.
#[cfg(feature = "display-names")]
fn item_display_name(id: u16) -> &'static str {
    g::items::ITEMS[usize::from(id)].display_name
}

// The API shared with the other modern versions (`v26_2` compiles the same source).
include!("../modern_api.rs");

#[cfg(test)]
mod tests;
