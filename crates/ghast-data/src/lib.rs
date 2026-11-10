//! Static Minecraft Java Edition game data for the ghast client library.
//!
//! One module per supported version, each behind a cargo feature:
//!
//! | module | feature | version | block model |
//! |---|---|---|---|
//! | [`v1_8`] | `v1_8` | 1.8.x (protocol 47) | `id << 4 \| meta` |
//! | [`v1_12`] | `v1_12` | 1.12.x (protocol 335 – 340) | `id << 4 \| meta` |
//! | [`v1_16_5`] | `v1_16_5` | 1.16.5 (protocol 754) | global palette with properties |
//! | [`v26_1`] | `v26_1` | 26.1 (protocol 775) | global palette with properties; movement facts from the jar |
//!
//! All tables are generated (`codegen/data`) from PrismarineJS minecraft-data 3.119.0 plus a small
//! hand-maintained file of physics facts, and live in `static` memory: no runtime parsing, no lazy
//! initialisation, lookups by id are array indexing, lookups by name are binary searches.
//!
//! ```
//! # #[cfg(feature = "v1_8")] {
//! use ghast_data::v1_8::{Block, BlockState};
//! let slab = BlockState::new(Block::STONE_SLAB, 0);
//! assert_eq!(slab.shape()[0].max_y, 0.5);
//! assert_eq!(Block::ICE.physics().slipperiness, 0.98);
//! # }
//! ```
//!
//! Collision shapes are vanilla-exact only where the source data is; blocks whose shape depends on
//! neighbours, entities or block entities say so via [`ShapeDependency`] instead of returning a
//! fake static shape. See the crate README for known data discrepancies.

#![forbid(unsafe_code)]
#![warn(missing_docs, unnameable_types)]

pub mod common;
mod generated;
#[cfg_attr(
    not(any(feature = "v1_8", feature = "v1_12", feature = "v1_16_5")),
    allow(unused_macros, unused_imports)
)]
mod macros;

#[cfg(feature = "v1_12")]
pub mod v1_12;
#[cfg(feature = "v1_16_5")]
pub mod v1_16_5;
#[cfg(feature = "v1_8")]
pub mod v1_8;
#[cfg(feature = "v26_1")]
pub mod v26_1;

pub use common::{
    Aabb, BiomeInfo, BlockInfo, BlockPhysics, BoundingBox, Collision, DigMaterial, Drop,
    EffectInfo, EnchantCost, EnchantmentInfo, EntityDimensions, EntityInfo, EntityKind, Fluid,
    FluidKind, FoodInfo, ItemInfo, NeighbourShape, PhysicsFlags, SelectionShape, ShapeDependency,
    VanillaEntitySize, WorldCoord, WorldCoords,
};
