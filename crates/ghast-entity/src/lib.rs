//! Entity tracking for the ghast Minecraft Java Edition client library (1.8.9, 1.12.2, 1.16.5
//! and 26.1).
//!
//! [`v1_8::Tracker`] / [`v1_12::Tracker`] / [`v1_16_5::Tracker`] / `v26_1::Tracker` take raw
//! clientbound play packets
//! ([`ghast_protocol::RawPacket`]) and keep every entity the server announced, keyed by id:
//! type, UUID, **server position** (exactly as the vanilla client tracks it from the packets,
//! with each version's fixed-point rules), rotation, last velocity, on-ground, riding / leash
//! links, and optionally metadata, equipment, effects and the tab list.
//!
//! - **Cheap.** Packets are parsed by hand straight from the packet body; movement, look,
//!   teleport, velocity, head-rotation and destroy packets do not allocate. Lookups are one hash
//!   of the id. Parts nobody needs are switched off in [`Config`], and `Tracker::wants(id)` /
//!   `packet_ids()` tell the client which packets it can drop before decoding.
//! - **Vanilla semantics.** Spawns replace an existing id, unknown types spawn nothing, moves
//!   for unknown ids are ignored, `login` and world changes clear everything, pickups remove
//!   the item (1.16: when its stack runs out), riding follows each version's rules.
//! - **Never panics on server data**: malformed bodies return a [`ghast_protocol::DecodeError`].
//!
//! ```
//! use ghast_entity::{Config, v1_8::Tracker};
//! let tracker = Tracker::new(Config::default());
//! assert!(tracker.get(42).is_none());
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, unnameable_types)]
// A build with only some versions leaves helpers of the others unused.
#![cfg_attr(
    not(all(
        feature = "v1_8",
        feature = "v1_12",
        feature = "v1_16_5",
        feature = "v26_1"
    )),
    allow(dead_code)
)]
// A build with no version at all keeps only the shared store.
#![cfg_attr(
    not(any(
        feature = "v1_8",
        feature = "v1_12",
        feature = "v1_16_5",
        feature = "v26_1"
    )),
    allow(unused_imports, unused_variables)
)]

mod common;
mod entity;
mod store;
#[cfg(feature = "v1_12")]
pub mod v1_12;
#[cfg(feature = "v1_16_5")]
pub mod v1_16_5;
#[cfg(feature = "v1_8")]
pub mod v1_8;
#[cfg(feature = "v26_1")]
pub mod v26_1;

pub use entity::{
    EQUIPMENT_SLOTS, Effect, Entity, EntityClass, EntityType, Item, MetaValue, Metadata,
};
pub use store::{Config, Entities, Event, PlayerInfo};

/// What applying a packet did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// Tracked state changed.
    Changed,
    /// Valid, but nothing changed (unknown entity id, unknown type, the bot's own position, ...).
    Ignored,
    /// Not a packet this tracker handles (or one its configuration does not want).
    NotEntityPacket,
}
