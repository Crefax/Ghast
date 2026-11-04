//! Minecraft 1.16.5 (protocol 754) client player movement.
//!
//! Same calling convention as [`crate::v1_8`]; the state carries 1.16's extra movement state
//! (pose, swimming, fluid heights, motion multiplier). Collision uses voxel-grid shapes
//! ([`crate::voxel`]).

mod blocks;
mod collide;
mod fluid;
mod player;

/// The 1.16.5 block state the world view returns (re-exported from `ghast-data`).
pub use ghast_data::v1_16_5::BlockState;
pub use player::{
    Effects, Enchantments, MovementInput, PlayerState, Pose, STEP_HEIGHT, SentState,
    SpeedModifiers, WIDTH, box_at, is_climbing, tick,
};

use crate::input::{Packets, TickInput};
use crate::world::BlockView;

/// The 1.16.5 rule set as a type (see [`crate::Ruleset`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct V1_16_5;

/// 1.16.5 has no rule options yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Options {}

impl crate::Ruleset for V1_16_5 {
    type BlockState = BlockState;
    type Player = PlayerState;
    type Options = Options;

    #[inline]
    fn tick_with<W: BlockView<State = BlockState>>(
        player: &mut PlayerState,
        input: &TickInput,
        world: &W,
        _options: &Options,
        out: &mut Packets,
    ) {
        tick(player, input, world, out);
    }
}

/// A [`crate::MemoryWorld`] of 1.16.5 block states (global palette ids).
pub type MemoryWorld = crate::MemoryWorld<BlockState>;

/// An empty 1.16.5 [`MemoryWorld`]; unknown ids decode to air.
#[must_use]
pub fn memory_world() -> MemoryWorld {
    crate::MemoryWorld::new(|raw| BlockState::from_id(raw).unwrap_or(BlockState::AIR))
}

#[cfg(test)]
mod tests;
