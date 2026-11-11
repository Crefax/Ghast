//! The 1.8.9 crosshair block pick (vanilla `Entity.rayTrace(reach, 1.0F)`:
//! `World.rayTrace(eye, eye + look * reach, false, false, true)`), from
//! [`crate::v1_8::ray_shape`] (neighbour-dependent bounds, the eight-pass stairs test).
//! Verified against the oracle (`tests/golden/1.8.9/raytrace.jsonl.gz`).

use ghast_data::v1_8::BlockState;

use super::BlockHit;
use super::legacy::{Shape, trace};
pub use super::legacy::{eye_position, look_vector, ray_end};
use crate::math::Aabb;
use crate::v1_8::{RayShape, ray_shape};
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
        RayShape::Bounds(b) => Shape::One(b),
        RayShape::Stairs(p) => {
            let mut boxes = [Aabb::FULL; 8];
            let mut len = 0u8;
            for b in p.boxes().into_iter().flatten() {
                boxes[usize::from(len)] = b;
                len += 1;
            }
            Shape::Many(boxes, len)
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

/// The crosshair block: `Entity.rayTrace(reach, 1.0F)` from `eye` along `look`. In 1.8 the
/// player's look vector uses the **head** yaw (`LivingEntity.getRotationVector` override):
/// `look_vector(head_yaw, pitch)`. `None` when the
/// ray ends without hitting or passing a block it skips.
pub fn pick<W: BlockView<State = BlockState>>(
    world: &W,
    eye: [f64; 3],
    look: [f64; 3],
    reach: f64,
) -> Option<BlockHit> {
    clip(world, eye, ray_end(eye, look, reach), false, true)
}
