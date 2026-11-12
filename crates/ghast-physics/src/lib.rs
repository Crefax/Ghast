//! Vanilla-exact player movement for the ghast Minecraft Java Edition client library.
//!
//! The crate is a **pure, deterministic function**: `(player state, input, world view) -> new
//! player state + the movement packets of the tick`. No IO, no async, no clock, no global mutable
//! state (the only static is vanilla's constant sine table). The same code path serves the live
//! client, trace replays and look-ahead planning (copy the state, simulate, discard).
//!
//! Rules are per protocol version, each in its own module selected at compile time:
//!
//! | module | version | status |
//! |---|---|---|
//! | [`v1_8`] | 1.8.9 (protocol 47) | bit-exact against the vanilla oracle |
//! | [`v1_12`] | 1.12.2 (protocol 340) | bit-exact against the vanilla oracle |
//! | [`v1_16_5`] | 1.16.5 (protocol 754) | bit-exact against the vanilla oracle |
//! | [`modern`] (`V26_1`) | 26.1 (protocol 775) | bit-exact against the vanilla oracle |
//!
//! Code that is generic over versions uses the [`Ruleset`] trait (static dispatch, no runtime
//! cost).
//!
//! [`pick`] is the crosshair target (block, face, hit point, or entity) for 1.16.5 and 26.1.
//!
//! ```
//! # #[cfg(feature = "v1_8")] {
//! use ghast_physics::{Keys, Packets, TickInput, v1_8};
//!
//! let mut world = v1_8::memory_world();
//! world.load_chunks(-1, -1, 0, 0);
//! world.fill_raw([-16, 63, -16], [15, 63, 15], 1 << 4); // stone floor
//! let mut player = v1_8::PlayerState::new([0.5, 64.0, 0.5], 0.0, 0.0);
//! let mut out = Packets::new();
//! let keys = Keys { forward: true, ..Keys::NONE };
//! let input = TickInput { keys, yaw: 0.0, pitch: 0.0, using_item: false };
//! for _ in 0..20 {
//!     out.clear();
//!     v1_8::tick(&mut player, &input, &world, &mut out);
//! }
//! assert!(player.on_ground && player.pos[2] > 2.0);
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, unnameable_types)]

pub mod attribute;
mod input;
pub mod math;
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
pub mod modern;
pub mod pick;
pub mod rotation;
#[cfg(feature = "v1_12")]
pub mod v1_12;
#[cfg(feature = "v1_16_5")]
pub mod v1_16_5;
#[cfg(feature = "v1_8")]
pub mod v1_8;
pub mod voxel;
mod world;

pub use input::{Abilities, Keys, Packet, Packets, TickInput};
pub use world::{BlockView, MemoryWorld, WorldBorder};

/// A version's movement rules, for code generic over protocol versions.
pub trait Ruleset {
    /// The version's block state type.
    type BlockState: Copy;
    /// The version's player movement state.
    type Player: Clone;
    /// The version's rule options (`Default` = the project policy).
    type Options: Default;

    /// Runs one client tick with explicit options (see the version module's `tick_with`).
    fn tick_with<W: BlockView<State = Self::BlockState>>(
        player: &mut Self::Player,
        input: &TickInput,
        world: &W,
        options: &Self::Options,
        out: &mut Packets,
    );

    /// Runs one client tick with the default options.
    fn tick<W: BlockView<State = Self::BlockState>>(
        player: &mut Self::Player,
        input: &TickInput,
        world: &W,
        out: &mut Packets,
    ) {
        Self::tick_with(player, input, world, &Self::Options::default(), out);
    }
}
