//! Chunk storage for the ghast Minecraft Java Edition client library.
//!
//! One world type per protocol version ([`v1_8::World`], [`v1_16_5::World`]) decodes the
//! version's chunk and block-change packets into a version-independent [`ColumnMap`] of
//! [`Column`]s made of compact [`Section`]s.
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
//! - **Never panics on server data.** Malformed chunk data returns a [`WorldError`] and leaves the
//!   column unloaded (never half-applied, never guessed as air).
//!
//! ```
//! # #[cfg(feature = "v1_8")] {
//! use ghast_world::{WorldConfig, v1_8::World};
//! let mut world = World::new(WorldConfig::default());
//! assert_eq!(world.block_state(0, 64, 0), None); // nothing loaded yet: unknown, not air
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod border;
mod column;
mod error;
pub mod packed;
mod section;

#[cfg(feature = "v1_16_5")]
pub mod v1_16_5;
#[cfg(feature = "v1_8")]
pub mod v1_8;

pub use border::Border;
pub use column::{Biomes, ChunkPos, Column, ColumnMap, MAX_Y, MIN_Y, SECTIONS, SectionLight};
pub use error::WorldError;
pub use section::{SECTION_VOLUME, Section, block_index};

/// What a world keeps besides block states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldConfig {
    /// Keep biome ids (1.8: 256 bytes, 1.16.5: 2 KiB per column). Default `true`.
    pub biomes: bool,
    /// Keep the 1.8 block / sky light arrays sent with chunks (4 KiB per section, never
    /// recomputed locally). Default `false`: the physics does not need light.
    pub light: bool,
}

impl Default for WorldConfig {
    fn default() -> Self {
        WorldConfig {
            biomes: true,
            light: false,
        }
    }
}

/// What applying a packet did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// The world changed.
    Changed,
    /// The packet was valid but vanilla ignores it in this situation (e.g. a partial chunk or a
    /// block change for a column that is not loaded).
    Ignored,
    /// The packet is not one the world handles.
    NotWorldPacket,
}
