//! The 26.1 crosshair block pick (vanilla `LocalPlayer.raycastHitResult`, which `Minecraft.pick`
//! stores as the crosshair target), without entities.
//!
//! The block ray runs to `max(block_range, entity_range)` (the `block_interaction_range` and
//! `entity_interaction_range` attribute values); a block hit farther than `block_range` (or the
//! miss at the ray's end beyond it) becomes a miss at that point. Verified against the oracle:
//! `tests/golden/26.1/raytrace.jsonl.gz`.

use ghast_data::v26_1::{Block, BlockState};

use super::{BlockHit, Direction, PlacedShape};
use crate::modern::collide::model_offset;
use crate::modern::math::{DEG_TO_RAD, cos, sin};
use crate::world::BlockView;

/// `Entity.getEyePosition(1.0F)`: the eye at the current position, interpolated from the old one
/// (`lerp(1, old, pos)`, which is not always exactly `pos`) plus the pose's eye height.
#[must_use]
pub fn eye_position(pos: [f64; 3], old: [f64; 3], eye_height: f32) -> [f64; 3] {
    let lerp = |o: f64, p: f64| o + 1.0 * (p - o);
    [
        lerp(old[0], pos[0]),
        lerp(old[1], pos[1]) + f64::from(eye_height),
        lerp(old[2], pos[2]),
    ]
}

/// `Entity.calculateViewVector(pitch, yaw)`: the unit look vector.
#[must_use]
pub fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let x_rot = pitch * DEG_TO_RAD;
    let y_rot = -yaw * DEG_TO_RAD;
    let y_cos = cos(f64::from(y_rot));
    let y_sin = sin(f64::from(y_rot));
    let x_cos = cos(f64::from(x_rot));
    let x_sin = sin(f64::from(x_rot));
    [
        f64::from(y_sin * x_cos),
        f64::from(-x_sin),
        f64::from(y_cos * x_cos),
    ]
}

/// The shapes the pick tests at a block: the outline (the held-item variant when the player holds
/// this block's own item; moved by the model offset where the block does that) and the
/// interaction shape.
#[must_use]
pub fn shapes(
    state: BlockState,
    pos: [i32; 3],
    held_block: Option<Block>,
) -> (PlacedShape, PlacedShape) {
    let outline = if held_block == Some(state.block()) {
        state.outline_shape_holding_own_item()
    } else {
        state.outline_shape()
    };
    let shift = state
        .outline_offset()
        .then(|| model_offset(state.block().data().offset, pos[0], pos[2]));
    (
        PlacedShape {
            shape: outline,
            shift,
        },
        PlacedShape::fixed(state.interaction_shape()),
    )
}

/// The block ray `from -> to` (vanilla `Level.clip` with `ClipContext.Block.OUTLINE`,
/// `Fluid.NONE` and the player's collision context).
///
/// `held_block` is the block form of the main-hand item (`None` for an empty hand or a non-block
/// item); it only matters for blocks that show a different outline to a player holding them.
/// The world must read as air outside its build height (as vanilla's void air) and in columns it
/// does not have (the client's empty chunks).
pub fn clip<W: BlockView<State = BlockState>>(
    world: &W,
    from: [f64; 3],
    to: [f64; 3],
    held_block: Option<Block>,
) -> BlockHit {
    super::clip(from, to, |pos| {
        let (o, i) = shapes(world.block(pos[0], pos[1], pos[2]), pos, held_block);
        (!o.shape.empty).then_some((o, i))
    })
}

/// The crosshair target with no entity in reach: the block ray from `eye` along `view` to
/// `max(block_range, entity_range)`, filtered to `block_range` (vanilla `LocalPlayer.pick` /
/// `filterHitResult`).
pub fn pick<W: BlockView<State = BlockState>>(
    world: &W,
    eye: [f64; 3],
    view: [f64; 3],
    block_range: f64,
    entity_range: f64,
    held_block: Option<Block>,
) -> BlockHit {
    // Math.max (no NaN in attribute values).
    let max = if block_range >= entity_range {
        block_range
    } else {
        entity_range
    };
    let to = [
        eye[0] + view[0] * max,
        eye[1] + view[1] * max,
        eye[2] + view[2] * max,
    ];
    let hit = clip(world, eye, to, held_block);
    filter(hit, eye, block_range)
}

/// `LocalPlayer.filterHitResult`: a hit (or miss point) not closer than `range` becomes a miss
/// there, facing along the ray.
#[must_use]
pub fn filter(hit: BlockHit, from: [f64; 3], range: f64) -> BlockHit {
    let l = hit.location;
    let (dx, dy, dz) = (from[0] - l[0], from[1] - l[1], from[2] - l[2]);
    if dx * dx + dy * dy + dz * dz < range * range {
        return hit;
    }
    let face = Direction::approximate_nearest(l[0] - from[0], l[1] - from[1], l[2] - from[2]);
    BlockHit::miss(l, face)
}
