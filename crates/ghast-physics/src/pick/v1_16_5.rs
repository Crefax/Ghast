//! The 1.16.5 crosshair block pick (vanilla `Entity.raycast(reach, 1.0F, false)`, which
//! `GameRenderer.updateTargetedEntity` stores as the crosshair target when no entity is nearer),
//! without entities. The client's reach is 4.5 (survival / adventure) or 5.0 (creative).
//! Verified against the oracle: `tests/golden/1.16.5/raytrace.jsonl.gz`.

use ghast_data::v1_16_5::{Block, BlockState, OffsetType};

use super::{BlockHit, PlacedShape};
use crate::math::{cos, sin};
use crate::world::BlockView;

/// Survival / adventure block reach.
pub const REACH: f64 = 4.5;
/// Creative block reach.
pub const CREATIVE_REACH: f64 = 5.0;

/// `Entity.getCameraPosVec(1.0F)`: `(x, y + eye height, z)` (no interpolation at 1.0).
#[must_use]
pub fn eye_position(pos: [f64; 3], eye_height: f32) -> [f64; 3] {
    [pos[0], pos[1] + f64::from(eye_height), pos[2]]
}

/// `Entity.getRotationVector(pitch, yaw)`: the unit look vector (float sine table).
#[must_use]
pub fn rotation_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    const DEG: f32 = (std::f64::consts::PI / 180.0) as f32;
    let f = pitch * DEG;
    let g = -yaw * DEG;
    let h = cos(g);
    let i = sin(g);
    let j = cos(f);
    let k = sin(f);
    [f64::from(i * j), f64::from(-k), f64::from(h * j)]
}

/// `AbstractBlockState.getModelOffset` at `(x, z)`.
#[must_use]
pub fn model_offset(offset: OffsetType, x: i32, z: i32) -> [f64; 3] {
    if offset == OffsetType::None {
        return [0.0; 3];
    }
    // MathHelper.hashCode(x, 0, z)
    let mut l = i64::from(x.wrapping_mul(3_129_871)) ^ i64::from(z).wrapping_mul(116_129_781);
    l = l
        .wrapping_mul(l)
        .wrapping_mul(42_317_861)
        .wrapping_add(l.wrapping_mul(11));
    let l = l >> 16;
    let h = |bits: i64| (f64::from((bits & 15) as f32 / 15.0) - 0.5) * 0.5;
    let y = if offset == OffsetType::Xyz {
        (f64::from(((l >> 4) & 15) as f32 / 15.0) - 1.0) * 0.2
    } else {
        0.0
    };
    [h(l), y, h(l >> 8)]
}

/// The shapes the pick tests at a block: the outline (the held-item variant when the player holds
/// this block's own item; moved by the model offset where the block does that) and the raycast
/// shape.
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
        .then(|| model_offset(state.block().vanilla().offset, pos[0], pos[2]));
    (
        PlacedShape {
            shape: outline,
            shift,
        },
        PlacedShape::fixed(state.raycast_shape()),
    )
}

/// The block ray `from -> to` (vanilla `World.raycast` with `ShapeType.OUTLINE`,
/// `FluidHandling.NONE` and the player's shape context).
///
/// `held_block` is the block form of the main-hand item (`None` for an empty hand or a non-block
/// item); it only matters for scaffolding, whose outline is a full cube to a player holding it.
pub fn clip<W: BlockView<State = BlockState>>(
    world: &W,
    from: [f64; 3],
    to: [f64; 3],
    held_block: Option<Block>,
) -> BlockHit {
    super::clip(from, to, |pos| {
        // World.getBlockState: void air outside the build height.
        if !(0..256).contains(&pos[1]) {
            return None;
        }
        let (o, i) = shapes(world.block(pos[0], pos[1], pos[2]), pos, held_block);
        (!o.shape.empty).then_some((o, i))
    })
}

/// The crosshair target with no entity in reach: the block ray from `eye` along `look` to `reach`.
pub fn pick<W: BlockView<State = BlockState>>(
    world: &W,
    eye: [f64; 3],
    look: [f64; 3],
    reach: f64,
    held_block: Option<Block>,
) -> BlockHit {
    let to = [
        eye[0] + look[0] * reach,
        eye[1] + look[1] * reach,
        eye[2] + look[2] * reach,
    ];
    clip(world, eye, to, held_block)
}
