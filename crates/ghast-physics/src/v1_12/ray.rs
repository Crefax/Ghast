//! 1.12.2 crosshair ray shapes: the box a block's ray test uses (vanilla
//! `Block.getBoundingBox(state, world, pos)`, Legacy Yarn `BlockState.getCollisionBox(view, pos)`),
//! including the neighbour-dependent ones (fences, panes, walls, chorus plants from their actual
//! connections, doors and chests as their collision box), and stairs, whose ray test runs per
//! collision box of the actual corner shape.

use ghast_data::v1_12::BlockState;

use super::blocks::{
    ShapeKind, boxes_of, fence_links, id, pane_links, shape_kind, state_at, wall_links,
};
use crate::math::Aabb;
use crate::world::BlockView;

/// What a ray tests against at one block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RayShape {
    /// The block is skipped by the ray (vanilla `canCollideCheck` false for this ray kind).
    None,
    /// The ray considers the block but never hits it (moving piston: its ray test returns null).
    Never,
    /// One box, block-local.
    Bounds(Aabb),
    /// Stairs: one ray test per collision box (block-local, in collision order); vanilla keeps the
    /// hit farthest from the ray's end (strictly; ties keep the earlier box).
    Boxes {
        /// The boxes (`len` used).
        boxes: [Aabb; 4],
        /// Number of boxes.
        len: u8,
    },
}

fn local(b: [f64; 6]) -> RayShape {
    RayShape::Bounds(Aabb::new(b[0], b[1], b[2], b[3], b[4], b[5]))
}

/// The ray shape of the block at `(x, y, z)`. `liquids` = the ray stops at liquids (vanilla's
/// `canCollideCheck(state, true)`).
pub fn ray_shape<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    liquids: bool,
) -> RayShape {
    let s = state_at(w, x, y, z);
    let sel = s.selection();
    if !(if liquids {
        sel.can_collide_liquid
    } else {
        sel.can_collide
    }) {
        return RayShape::None;
    }
    if s.block().id() == id::PISTON_EXTENSION {
        return RayShape::Never;
    }
    match shape_kind(s.block().id()) {
        ShapeKind::Fence => {
            let [n, e, so, we] = fence_links(w, x, y, z, s);
            local([
                if we { 0.0 } else { 0.375 },
                0.0,
                if n { 0.0 } else { 0.375 },
                if e { 1.0 } else { 0.625 },
                1.0,
                if so { 1.0 } else { 0.625 },
            ])
        }
        ShapeKind::Pane => {
            let [n, e, so, we] = pane_links(w, x, y, z);
            local([
                if we { 0.0 } else { 0.4375 },
                0.0,
                if n { 0.0 } else { 0.4375 },
                if e { 1.0 } else { 0.5625 },
                1.0,
                if so { 1.0 } else { 0.5625 },
            ])
        }
        ShapeKind::Wall => {
            let [n, e, so, we] = wall_links(w, x, y, z);
            let mut b = [
                if we { 0.0 } else { 0.25 },
                0.0,
                if n { 0.0 } else { 0.25 },
                if e { 1.0 } else { 0.75 },
                1.0,
                if so { 1.0 } else { 0.75 },
            ];
            if n && so && !we && !e {
                (b[0], b[3], b[4]) = (0.3125, 0.6875, 0.875);
            } else if we && e && !n && !so {
                (b[2], b[5], b[4]) = (0.3125, 0.6875, 0.875);
            }
            local(b)
        }
        ShapeKind::ChorusPlant => {
            let chorus = |dx: i32, dy: i32, dz: i32| {
                let n = state_at(w, x + dx, y + dy, z + dz).block().id();
                n == id::CHORUS_PLANT || n == id::CHORUS_FLOWER || (dy == -1 && n == id::END_STONE)
            };
            const A: f64 = 0.1875;
            const B: f64 = 0.8125;
            local([
                if chorus(-1, 0, 0) { 0.0 } else { A },
                if chorus(0, -1, 0) { 0.0 } else { A },
                if chorus(0, 0, -1) { 0.0 } else { A },
                if chorus(1, 0, 0) { 1.0 } else { B },
                if chorus(0, 1, 0) { 1.0 } else { B },
                if chorus(0, 0, 1) { 1.0 } else { B },
            ])
        }
        kind @ (ShapeKind::Door | ShapeKind::Chest | ShapeKind::Stairs) => {
            // The bounding box is the (single) collision box for doors and chests; stairs test
            // every collision box of the actual shape.
            let (fx, fy, fz) = (f64::from(x), f64::from(y), f64::from(z));
            let all = boxes_of(w, x, y, z);
            let mut boxes = [Aabb::FULL; 4];
            let mut len = 0u8;
            for b in all.iter().take(4) {
                boxes[len as usize] = Aabb::new(
                    b.min_x - fx,
                    b.min_y - fy,
                    b.min_z - fz,
                    b.max_x - fx,
                    b.max_y - fy,
                    b.max_z - fz,
                );
                len += 1;
            }
            if kind == ShapeKind::Stairs {
                RayShape::Boxes { boxes, len }
            } else if len > 0 {
                RayShape::Bounds(boxes[0])
            } else {
                RayShape::Bounds(sel.bounds)
            }
        }
        _ => RayShape::Bounds(sel.bounds),
    }
}
