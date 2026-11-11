//! Minecraft 1.12.2 (protocol 340) client player movement; the rules hold for every 1.12 release.
//!
//! [`tick`] advances a [`PlayerState`] by one client tick against a [`BlockView`] of 1.12 block
//! states and returns the movement packets of that tick. The rules reproduce vanilla 1.12.2's
//! `ClientPlayerEntity` bit for bit (verified against the oracle's golden vectors, see the crate
//! README). What changed since 1.8.9 is listed in the crate README ("1.12.2").
//!
//! [`BlockView`]: crate::BlockView

mod blocks;
mod fluid;
mod player;
mod ray;

pub use player::{
    Effects, HEIGHT, MovementInput, Options, PlayerState, SNEAK_HEIGHT, STEP_HEIGHT, SentState,
    WIDTH, box_at, is_climbing, is_touching_lava, tick, tick_with,
};

/// The 1.12 block state the world view returns (re-exported from `ghast-data`).
pub use ghast_data::v1_12::BlockState;
pub use ray::{RayShape, ray_shape};

use crate::input::{Packets, TickInput};
use crate::math::Aabb;
use crate::world::BlockView;

/// The 1.12.2 rule set as a type, for code generic over versions (see [`crate::Ruleset`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct V1_12;

impl crate::Ruleset for V1_12 {
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

/// A [`crate::MemoryWorld`] of 1.12 block states (raw values decoded the vanilla way).
pub type MemoryWorld = crate::MemoryWorld<BlockState>;

/// An empty 1.12 [`MemoryWorld`] (nothing loaded).
#[must_use]
pub fn memory_world() -> MemoryWorld {
    crate::MemoryWorld::new(BlockState::from_raw_vanilla)
}

/// Every collision box of the block at `(x, y, z)`, block-local shapes placed in the world with
/// their actual state (neighbour connections) applied: what the player collides with there.
#[must_use]
pub fn collision_boxes<W: BlockView<State = BlockState>>(
    world: &W,
    x: i32,
    y: i32,
    z: i32,
) -> Vec<Aabb> {
    blocks::boxes_of(world, x, y, z)
}

#[cfg(test)]
mod tests;
