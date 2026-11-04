//! Minecraft 1.8.9 (protocol 47) client player movement.
//!
//! [`tick`] advances a [`PlayerState`] by one client tick against a [`BlockView`] of 1.8 block
//! states and returns the movement packets of that tick. The rules reproduce vanilla 1.8.9's
//! `ClientPlayerEntity` bit for bit (verified against the oracle's golden vectors, see the crate
//! README for coverage).
//!
//! [`BlockView`]: crate::BlockView

mod blocks;
mod fluid;
mod player;
mod ray;

pub use blocks::HistoryShapes;
pub use player::{
    Effects, HEIGHT, MovementInput, Options, PlayerState, STEP_HEIGHT, SentState, WIDTH, box_at,
    is_climbing, is_touching_lava, tick, tick_with,
};
pub use ray::{RayShape, StairsPasses, ray_shape};

/// The 1.8 block state the world view returns (re-exported from `ghast-data`).
pub use ghast_data::v1_8::BlockState;

use crate::input::{Packets, TickInput};
use crate::world::BlockView;

/// The 1.8.9 rule set as a type, for code generic over versions (see [`crate::Ruleset`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct V1_8;

impl crate::Ruleset for V1_8 {
    type BlockState = BlockState;
    type Player = PlayerState;
    type Options = Options;

    #[inline]
    fn tick_with<W: BlockView<State = BlockState>>(
        player: &mut PlayerState,
        input: &TickInput,
        world: &W,
        options: &Options,
        out: &mut Packets,
    ) {
        tick_with(player, input, world, options, out);
    }
}

/// A [`crate::MemoryWorld`] of 1.8 block states (raw values decoded the vanilla way).
pub type MemoryWorld = crate::MemoryWorld<BlockState>;

/// An empty 1.8 [`MemoryWorld`] (nothing loaded).
#[must_use]
pub fn memory_world() -> MemoryWorld {
    crate::MemoryWorld::new(BlockState::from_raw_vanilla)
}

#[cfg(test)]
mod tests;
