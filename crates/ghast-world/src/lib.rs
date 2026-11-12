//! Chunk storage for the ghast Minecraft Java Edition client library.
//!
//! One world type per protocol version ([`v1_8::World`], [`v1_12::World`], [`v1_16_5::World`])
//! decodes the version's chunk and block-change packets into a version-independent
//! [`ColumnMap`] of [`Column`]s made of compact [`Section`]s.
//!
//! - **Loaded vs unloaded is explicit.** [`v1_8::World::block_state`] returns `None` for a
//!   column the server has not sent (or has unloaded); it never reports air there. Behaviour code
//!   must treat `None` as "unknown". Only the `ghast_physics::BlockView` implementation (feature
//!   `physics`) maps unloaded columns to air, because that is exactly what the vanilla client's
//!   empty chunk does and what the physics must reproduce.
//! - **Vanilla decoding.** States are stored as vanilla reads them: unknown 1.8 `id << 4 | meta`
//!   values and unregistered metas, unknown 1.16.5 palette ids and out-of-range palette indices
//!   all read as air. Packet handling follows the vanilla client (ground-up chunks replace the
//!   column, partial chunks for unloaded columns are ignored, block changes in unloaded columns
//!   are ignored, respawn into the same dimension keeps the world, ...). See each method.
//! - **Cheap lookups.** A block lookup is one hash of the column position plus one or two array
//!   reads; all-air sections are not stored.
//! - **Never panics on server data.** Malformed chunk data returns a [`WorldError`]. A 1.8 chunk
//!   shorter than its mask is applied as far as the vanilla client gets before its copy fails
//!   (a ground-up chunk still loads its column, as vanilla does); a malformed 1.16 section
//!   unloads the column.
//!
//! ```
//! # #[cfg(feature = "v1_8")] {
//! use ghast_world::{WorldConfig, v1_8::World};
//! let mut world = World::new(WorldConfig::default());
//! assert_eq!(world.block_state(0, 64, 0), None); // nothing loaded yet: unknown, not air
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, unnameable_types)]

mod border;
#[cfg(feature = "v26_1")]
pub mod climate;
mod column;
mod error;
pub mod packed;
#[cfg(feature = "v26_1")]
pub mod paletted;
mod section;

#[cfg(feature = "v1_12")]
pub mod v1_12;
#[cfg(feature = "v1_16_5")]
pub mod v1_16_5;
#[cfg(feature = "v1_8")]
pub mod v1_8;
/// The modern world source (`src/v26_1.rs`) compiled for one protocol: its `data` and `proto`
/// name that protocol's `ghast-data` and `ghast-protocol` modules.
#[cfg(feature = "v26_1")]
#[path = "."]
mod modern_775 {
    pub(crate) use ghast_data::v26_1 as data;
    pub(crate) use ghast_protocol::v26_1 as proto;
    #[path = "v26_1.rs"]
    pub mod world;
}
#[cfg(feature = "v26_1")]
#[doc(inline)]
pub use modern_775::world as v26_1;

pub use border::Border;
pub use column::{
    Biomes, BlockEntity, ChunkPos, Column, ColumnMap, MAX_Y, MIN_Y, SECTIONS, SectionLight,
};
pub use error::WorldError;
pub use section::{Direct, Indexed, SECTION_VOLUME, Section, block_index};

/// What a world keeps besides block states.
///
/// `#[non_exhaustive]`: build it from [`WorldConfig::default`] with the `with_*` methods, so new
/// options can be added without breaking callers:
///
/// ```
/// let c = ghast_world::WorldConfig::default().with_biomes(false).with_block_entities(true);
/// assert!(!c.biomes && c.block_entities && !c.light);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct WorldConfig {
    /// Keep biome ids (1.8: 256 bytes, 1.16.5: 2 KiB per column). Default `true`.
    pub biomes: bool,
    /// Keep the 1.8 block / sky light arrays sent with chunks (4 KiB per section, never
    /// recomputed locally). Default `false`: the physics does not need light.
    pub light: bool,
    /// Keep block entities (1.8 `update_sign` / `tile_entity_data`, 1.16.5 chunk block entity
    /// lists and `tile_entity_data`), see [`Column::block_entities`]. Default `false`: off costs
    /// nothing (the packets are not in the world's packet ids, nothing is parsed).
    pub block_entities: bool,
}

impl WorldConfig {
    /// Sets [`WorldConfig::biomes`].
    #[must_use]
    pub const fn with_biomes(mut self, on: bool) -> Self {
        self.biomes = on;
        self
    }

    /// Sets [`WorldConfig::light`].
    #[must_use]
    pub const fn with_light(mut self, on: bool) -> Self {
        self.light = on;
        self
    }

    /// Sets [`WorldConfig::block_entities`].
    #[must_use]
    pub const fn with_block_entities(mut self, on: bool) -> Self {
        self.block_entities = on;
        self
    }
}

impl Default for WorldConfig {
    fn default() -> Self {
        WorldConfig {
            biomes: true,
            light: false,
            block_entities: false,
        }
    }
}

/// What applying a packet did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Applied {
    /// The world changed.
    Changed,
    /// The packet was valid but vanilla ignores it in this situation (e.g. a partial chunk or a
    /// block change for a column that is not loaded).
    Ignored,
    /// The packet is not one the world handles.
    NotWorldPacket,
}
