//! The 1.12.2 crosshair block pick (vanilla `Entity.rayTrace(reach, 1.0F)`:
//! `World.rayTrace(eye, eye + look * reach, false, false, true)`), from
//! [`crate::v1_12::ray_shape`] (actual-state bounds, the per-box stairs test).
//! Verified against the oracle (`tests/golden/1.12.2/raytrace.jsonl.gz`).

use ghast_data::v1_12::BlockState;

pub use super::legacy::Target;
use super::legacy::{Shape, target, trace};
pub use super::legacy::{eye_position, look_vector, ray_end};
use super::{BlockHit, PickEntity};
use crate::math::Aabb;
use crate::v1_12::{RayShape, ray_shape};
use crate::world::BlockView;

/// Survival block reach.
pub const REACH: f64 = 4.5;
/// Creative block reach.
pub const CREATIVE_REACH: f64 = 5.0;

/// What the ray sees at a block.
#[must_use]
pub fn shape<W: BlockView<State = BlockState>>(world: &W, pos: [i32; 3], liquids: bool) -> Shape {
    match ray_shape(world, pos[0], pos[1], pos[2], liquids) {
        RayShape::None => Shape::Skip,
        RayShape::Never => Shape::Never,
        RayShape::Bounds(b) => Shape::One(b),
        RayShape::Boxes { boxes, len } => {
            let mut all = [Aabb::FULL; 8];
            all[..4].copy_from_slice(&boxes);
            Shape::Many(all, len)
        }
    }
}

/// `World.rayTrace(start, end, liquids, false, return_last)`.
pub fn clip<W: BlockView<State = BlockState>>(
    world: &W,
    start: [f64; 3],
    end: [f64; 3],
    liquids: bool,
    return_last: bool,
) -> Option<BlockHit> {
    trace(start, end, return_last, |p| shape(world, p, liquids))
}

/// The crosshair block: `Entity.rayTrace(reach, 1.0F)` from `eye` along `look`. In 1.12 the look
/// vector uses the body yaw: `look_vector(yaw, pitch)` (1.8 used the head yaw). `None` when the
/// ray ends without hitting or passing a block it skips.
pub fn pick<W: BlockView<State = BlockState>>(
    world: &W,
    eye: [f64; 3],
    look: [f64; 3],
    reach: f64,
) -> Option<BlockHit> {
    clip(world, eye, ray_end(eye, look, reach), false, true)
}

/// The crosshair target (vanilla `GameRenderer.updateTargetedEntity(1.0F)`): the block pick at the
/// game mode's reach (4.5, creative 5.0), unless an entity of `entities` is hit nearer along the
/// ray to the reach (creative: 6.0); outside creative an entity farther than 3 blocks gives a miss
/// at its hit point ([`Target::TooFar`]). `player_bb` is the player's bounding box; `entities` the
/// pickable ones (`collides()`, no spectators) in the world's iteration order.
pub fn pick_target<W: BlockView<State = BlockState>>(
    world: &W,
    eye: [f64; 3],
    look: [f64; 3],
    creative: bool,
    player_bb: &Aabb,
    entities: &[PickEntity],
) -> Target {
    let reach = if creative { CREATIVE_REACH } else { REACH };
    target(
        pick(world, eye, look, reach),
        eye,
        look,
        reach,
        creative,
        player_bb,
        entities,
    )
}
