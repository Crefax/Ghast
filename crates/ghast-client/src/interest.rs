//! Which optional packet groups a behaviour wants.
//!
//! The client always receives what its core needs (keep-alive, teleports, health, windows'
//! confirm transactions, chunk headers, …). Everything else is dropped by the frame
//! decoder *before* it is decoded — when compression is on, after inflating only the bytes
//! that hold the packet id. A busy server's tab list, teams, particles, sounds and entity
//! movement therefore cost almost nothing unless a behaviour asks for them.

use std::ops::{BitOr, BitOrAssign};

/// A set of optional packet groups (see the associated constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Interests(u32);

impl Interests {
    /// Nothing optional.
    pub const NONE: Interests = Interests(0);
    /// Chat messages ([`crate::TickEvents::chat`]).
    pub const CHAT: Interests = Interests(1 << 0);
    /// Entities (spawns, moves, metadata, equipment, effects): [`crate::TickContext::entities`]
    /// and the entity crosshair / attack / interact. Can be switched at run time: tracking then
    /// starts empty (entities already spawned stay unknown until the server sends them again)
    /// and switching off frees the store.
    pub const ENTITIES: Interests = Interests(1 << 1);
    /// The tab list (player names and UUIDs) on top of [`Interests::ENTITIES`] — the most
    /// expensive packet on busy servers. Switching it restarts the tracker empty.
    pub const PLAYER_LIST: Interests = Interests(1 << 2);
    /// Skin / cape textures (the base64 `"textures"` profile property) of tab-list entries,
    /// on top of [`Interests::PLAYER_LIST`] (`PlayerInfo::textures`). Switching it restarts
    /// the tracker empty.
    pub const PLAYER_SKINS: Interests = Interests(1 << 4);
    /// Block changes (1.8 S23/S22, 1.16 block_change / multi_block_change) as
    /// [`crate::TickEvents::block_updates`]. The world applies them either way; this only
    /// collects them for the behaviour.
    pub const BLOCK_UPDATES: Interests = Interests(1 << 3);

    /// Whether every group in `other` is in `self`.
    #[must_use]
    pub const fn contains(self, other: Interests) -> bool {
        self.0 & other.0 == other.0
    }

    /// The union of two sets.
    #[must_use]
    pub const fn union(self, other: Interests) -> Interests {
        Interests(self.0 | other.0)
    }
}

impl BitOr for Interests {
    type Output = Interests;
    fn bitor(self, rhs: Interests) -> Interests {
        self.union(rhs)
    }
}

impl BitOrAssign for Interests {
    fn bitor_assign(&mut self, rhs: Interests) {
        *self = self.union(rhs);
    }
}
