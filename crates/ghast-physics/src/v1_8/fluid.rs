//! 1.8.9 liquid queries: water contact with its flow push, lava contact, and "any liquid in box".

use ghast_data::v1_8::BlockState;

use super::blocks::{
    blocks_movement, is_fluid, is_ice_material, is_lava, is_solid, is_water, state_at,
};
use crate::math::{Aabb, Vec3, floor};
use crate::world::BlockView;

/// Height of the liquid surface below the block top, as a fraction (`float`): levels 8+ (falling)
/// count as level 0.
#[inline]
fn surface_drop(level: u8) -> f32 {
    let level = if level >= 8 { 0 } else { level };
    (f32::from(level) + 1.0) / 9.0
}

/// `true` when the integer Y span `[y0, y1]` (inclusive bounds as vanilla passes them) touches the
/// world's height range (vanilla `isRegionLoaded`; every column "exists" on the client).
#[inline]
fn y_span_loaded(y0: i32, y1: i32) -> bool {
    y1 >= 0 && y0 < 256
}

/// Block grid range of a box the way vanilla's material queries iterate it:
/// `floor(min) .. floor(max + 1)` (exclusive end) per axis.
#[inline]
fn grid(b: &Aabb) -> [i32; 6] {
    [
        floor(b.min_x),
        floor(b.min_y),
        floor(b.min_z),
        floor(b.max_x + 1.0),
        floor(b.max_y + 1.0),
        floor(b.max_z + 1.0),
    ]
}

/// Water contact of a box (vanilla `World` material-acceleration query for water): whether any
/// water block's surface reaches the box top, and the summed flow vector of those blocks.
pub(crate) fn water_contact<W: BlockView<State = BlockState>>(w: &W, b: &Aabb) -> (bool, Vec3) {
    let [x0, y0, z0, x1, y1, z1] = grid(b);
    if !y_span_loaded(y0, y1) {
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
                // Surface height computed as vanilla does: `int - float`, a float.
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

/// Flow decrease of the water at a position: its level (falling water counts as 0), or `-1` when
/// the block is not water.
#[inline]
fn flow_level(s: BlockState) -> i32 {
    if is_water(s) {
        let l = i32::from(s.meta());
        if l >= 8 { 0 } else { l }
    } else {
        -1
    }
}

/// Whether a block face stops falling water from spreading sideways (vanilla fluid
/// `hasCollision` for horizontal directions).
#[inline]
fn blocks_side_flow(s: BlockState) -> bool {
    !is_water(s) && !is_ice_material(s) && is_solid(s)
}

/// The normalised flow direction of one water block.
fn flow_vector<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
) -> Vec3 {
    const SIDES: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
    let here = flow_level(s);
    let mut v = Vec3::ZERO;
    for (dx, dz) in SIDES {
        let n = state_at(w, x + dx, y, z + dz);
        let nl = flow_level(n);
        let weight = if nl >= 0 {
            Some(nl - here)
        } else if !blocks_movement(n) {
            // An open side: water one block lower pulls towards it.
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
        let walled = SIDES.iter().any(|&(dx, dz)| {
            blocks_side_flow(state_at(w, x + dx, y, z + dz))
                || blocks_side_flow(state_at(w, x + dx, y + 1, z + dz))
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

/// `true` when any water or lava block is in the box's block range. Unlike the other queries the
/// range is inclusive of `floor(max)` (vanilla `containsFluid`).
pub(crate) fn contains_liquid<W: BlockView<State = BlockState>>(w: &W, b: &Aabb) -> bool {
    let (x0, y0, z0) = (floor(b.min_x), floor(b.min_y), floor(b.min_z));
    let (x1, y1, z1) = (floor(b.max_x), floor(b.max_y), floor(b.max_z));
    (x0..=x1).any(|x| (y0..=y1).any(|y| (z0..=z1).any(|z| is_fluid(state_at(w, x, y, z)))))
}
