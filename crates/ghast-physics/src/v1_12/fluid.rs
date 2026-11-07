//! 1.12.2 liquid queries: water contact with its flow push, lava contact, and "any liquid in box".
//!
//! Since 1.9 every material query iterates `floor(min) .. ceil(max)` (exclusive end) per axis, so
//! a box whose maximum is an exact integer no longer reaches into the next block (1.8 used
//! `floor(max + 1)`). Falling water pulls down next to a block face that is solid (by its 1.12
//! face shape) instead of next to any solid material.

use ghast_data::v1_12::{BlockState, Face, FaceShape};

use super::blocks::{
    ShapeKind, attach_excluded, blocks_movement, is_ice_material, is_lava, is_liquid, is_water,
    shape_kind, state_at,
};
use crate::math::{Aabb, Vec3, floor};
use crate::world::BlockView;

/// Java `MathHelper.ceil(double)`.
#[inline]
fn ceil(v: f64) -> i32 {
    let i = v as i32;
    if v > f64::from(i) { i + 1 } else { i }
}

/// Height of the liquid surface below the block top, as a fraction (`float`): levels 8+ (falling)
/// count as level 0.
#[inline]
fn surface_drop(level: u8) -> f32 {
    let level = if level >= 8 { 0 } else { level };
    (f32::from(level) + 1.0) / 9.0
}

/// Block grid range of a box the way 1.12's material queries iterate it:
/// `floor(min) .. ceil(max)` (exclusive end) per axis.
#[inline]
fn grid(b: &Aabb) -> [i32; 6] {
    [
        floor(b.min_x),
        floor(b.min_y),
        floor(b.min_z),
        ceil(b.max_x),
        ceil(b.max_y),
        ceil(b.max_z),
    ]
}

/// Water contact of a box (vanilla `World.method_3610` for water): whether any water block's
/// surface reaches the box's top grid line, and the summed flow vector of those blocks.
pub(crate) fn water_contact<W: BlockView<State = BlockState>>(w: &W, b: &Aabb) -> (bool, Vec3) {
    let [x0, y0, z0, x1, y1, z1] = grid(b);
    // `isRegionLoaded`: on the client only the height range matters.
    if !(y1 >= 0 && y0 < 256) {
        return (false, Vec3::ZERO);
    }
    let mut touching = false;
    let mut flow = Vec3::ZERO;
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                let s = state_at(w, x, y, z);
                if !is_water(s) {
                    continue;
                }
                // `int - float`: a float, widened.
                let surface = f64::from((y + 1) as f32 - surface_drop(s.meta()));
                if f64::from(y1) >= surface {
                    touching = true;
                    let v = flow_vector(w, x, y, z, s);
                    flow = flow.add(v.x, v.y, v.z);
                }
            }
        }
    }
    (touching, flow)
}

/// Flow level of the water at a position (falling water counts as 0), `-1` when it is not water.
#[inline]
fn flow_level(s: BlockState) -> i32 {
    if is_water(s) {
        let l = i32::from(s.meta());
        if l >= 8 { 0 } else { l }
    } else {
        -1
    }
}

/// Whether the block at a side makes falling water flow down (vanilla `method_8665`): never
/// water itself; never ice; otherwise a solid face shape on `face` of a block that is neither
/// stairs nor on the attach exclusion list.
fn pulls_down(s: BlockState, face: Face) -> bool {
    if is_water(s) || is_ice_material(s) {
        return false;
    }
    let excluded = attach_excluded(s) || shape_kind(s.block().id()) == ShapeKind::Stairs;
    !excluded && s.face_shape(face) == FaceShape::Solid
}

/// The normalised flow direction of one water block.
fn flow_vector<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
) -> Vec3 {
    const SIDES: [(i32, i32, Face); 4] = [
        (0, -1, Face::North),
        (1, 0, Face::East),
        (0, 1, Face::South),
        (-1, 0, Face::West),
    ];
    let here = flow_level(s);
    let mut v = Vec3::ZERO;
    for (dx, dz, _) in SIDES {
        let n = state_at(w, x + dx, y, z + dz);
        let nl = flow_level(n);
        let weight = if nl >= 0 {
            Some(nl - here)
        } else if !blocks_movement(n) {
            let below = flow_level(state_at(w, x + dx, y - 1, z + dz));
            (below >= 0).then(|| below - (here - 8))
        } else {
            None
        };
        if let Some(k) = weight {
            v = v.add(f64::from(dx * k), 0.0, f64::from(dz * k));
        }
    }
    if s.meta() >= 8 {
        let walled = SIDES.iter().any(|&(dx, dz, face)| {
            pulls_down(state_at(w, x + dx, y, z + dz), face)
                || pulls_down(state_at(w, x + dx, y + 1, z + dz), face)
        });
        if walled {
            v = v.normalize().add(0.0, -6.0, 0.0);
        }
    }
    v.normalize()
}

/// `true` when any lava block is in the box's block range (vanilla `containsMaterial`).
pub(crate) fn contains_lava<W: BlockView<State = BlockState>>(w: &W, b: &Aabb) -> bool {
    let [x0, y0, z0, x1, y1, z1] = grid(b);
    (x0..x1).any(|x| (y0..y1).any(|y| (z0..z1).any(|z| is_lava(state_at(w, x, y, z)))))
}

/// `true` when any water or lava block is in the box's block range (vanilla `containsFluid`).
pub(crate) fn contains_liquid<W: BlockView<State = BlockState>>(w: &W, b: &Aabb) -> bool {
    let [x0, y0, z0, x1, y1, z1] = grid(b);
    (x0..x1).any(|x| (y0..y1).any(|y| (z0..z1).any(|z| is_liquid(state_at(w, x, y, z)))))
}
