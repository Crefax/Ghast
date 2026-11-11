//! 1.8.9 crosshair ray shapes: the bounds a block's ray test uses (vanilla `Block.rayTrace`
//! after `setBoundingBox(world, pos)`), including the neighbour-dependent ones (fences, panes,
//! walls, doors, chests) and the eight-pass stairs test.

use ghast_data::v1_8::BlockState;

use super::blocks::{
    BoxList, BoxSink, HistoryShapes, ShapeKind, add_collision_boxes, fence_links, pane_links,
    shape_kind, state_at, wall_links,
};
use crate::math::Aabb;
use crate::world::BlockView;

/// What a ray tests against at one block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RayShape {
    /// The block is skipped by the ray (vanilla `canCollide` false for this ray kind).
    None,
    /// One box, block-local.
    Bounds(Aabb),
    /// Stairs: eight half-cube passes (see [`StairsPasses`]).
    Stairs(StairsPasses),
}

/// The eight-pass stairs ray test: pass `j` tests the half-cube sub-box at
/// `(j % 2, j / 4 % 2, j / 2 % 2) * 0.5`, except two passes skipped for the stairs' facing / half.
/// Vanilla keeps the hit **farthest from the ray's end** (nearest to its start); ties keep the
/// earlier pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StairsPasses {
    /// The two skipped pass indices.
    pub skipped: [u8; 2],
}

impl StairsPasses {
    /// The sub-box of each pass in order, block-local; `None` for a skipped pass.
    #[must_use]
    pub fn boxes(&self) -> [Option<Aabb>; 8] {
        let mut passes = [None; 8];
        for (j, p) in passes.iter_mut().enumerate() {
            if self.skipped.contains(&(j as u8)) {
                continue;
            }
            let (bx, by, bz) = (
                0.5 * (j % 2) as f64,
                0.5 * (j / 4 % 2) as f64,
                0.5 * (j / 2 % 2) as f64,
            );
            *p = Some(Aabb::new(bx, by, bz, bx + 0.5, by + 0.5, bz + 0.5));
        }
        passes
    }
}

/// Passes the stairs ray test skips, by horizontal facing index (south, west, north, east) and
/// +4 for the top half.
const IGNORED_PASSES: [[u8; 2]; 8] = [
    [4, 5],
    [5, 7],
    [6, 7],
    [4, 6],
    [0, 1],
    [1, 3],
    [2, 3],
    [0, 2],
];

/// The ray shape of the block at `(x, y, z)`. `liquids` = the ray stops at liquids (vanilla's
/// `canCollide(state, true)` instead of `false`).
pub fn ray_shape<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    liquids: bool,
) -> RayShape {
    let s = state_at(w, x, y, z);
    let sel = s.selection();
    let collides = if liquids {
        sel.can_collide_liquid
    } else {
        sel.can_collide
    };
    if !collides {
        return RayShape::None;
    }
    let local = |b: [f64; 6]| RayShape::Bounds(Aabb::new(b[0], b[1], b[2], b[3], b[4], b[5]));
    // Torches (torch, unlit / lit redstone torch): `TorchBlock.rayTrace` sets the bounds from the
    // facing itself (the table's bounds are whatever was set last).
    if matches!(s.block().id(), 50 | 75 | 76) {
        return torch_bounds(s.meta());
    }
    match shape_kind(s.block().id()) {
        kind @ (ShapeKind::WoodFence | ShapeKind::NetherFence) => {
            let [n, so, we, e] = fence_links(w, x, y, z, kind);
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
            let [n, so, we, e] = pane_links(w, x, y, z);
            let any = n || so || we || e;
            // Along each axis: full length with both or no connections, else towards the one.
            let (mut x0, mut x1, mut z0, mut z1) = (0.4375, 0.5625, 0.4375, 0.5625);
            if we && e || !any {
                (x0, x1) = (0.0, 1.0);
            } else if we {
                x0 = 0.0;
            } else if e {
                x1 = 1.0;
            }
            if n && so || !any {
                (z0, z1) = (0.0, 1.0);
            } else if n {
                z0 = 0.0;
            } else if so {
                z1 = 1.0;
            }
            local([x0, 0.0, z0, x1, 1.0, z1])
        }
        ShapeKind::Wall => {
            let [n, so, we, e] = wall_links(w, x, y, z);
            let mut b = [
                if we { 0.0 } else { 0.25 },
                0.0,
                if n { 0.0 } else { 0.25 },
                if e { 1.0 } else { 0.75 },
                1.0,
                if so { 1.0 } else { 0.75 },
            ];
            if n && so && !we && !e {
                (b[0], b[3], b[4]) = (0.3125, 0.6875, 0.8125);
            } else if we && e && !n && !so {
                (b[2], b[5], b[4]) = (0.3125, 0.6875, 0.8125);
            }
            local(b)
        }
        ShapeKind::Door | ShapeKind::Chest => {
            // The ray bounds are the same single box the (settled) collision builds.
            let mut out = BoxList::new();
            let query = Aabb::new(
                f64::from(x) - 1.0,
                f64::from(y) - 1.0,
                f64::from(z) - 1.0,
                f64::from(x) + 2.0,
                f64::from(y) + 2.0,
                f64::from(z) + 2.0,
            );
            let mut sink = BoxSink {
                query,
                out: &mut out,
                history: HistoryShapes::Settled,
            };
            add_collision_boxes(w, x, y, z, s, &mut sink);
            match out.as_slice().first() {
                Some(b) => {
                    let (fx, fy, fz) = (f64::from(x), f64::from(y), f64::from(z));
                    RayShape::Bounds(Aabb::new(
                        b.min_x - fx,
                        b.min_y - fy,
                        b.min_z - fz,
                        b.max_x - fx,
                        b.max_y - fy,
                        b.max_z - fz,
                    ))
                }
                None => RayShape::Bounds(sel.bounds),
            }
        }
        ShapeKind::Stairs => {
            let meta = s.meta();
            // Stairs metadata facing (east, west, south, north) as a horizontal index.
            let horizontal = [3usize, 1, 0, 2][(meta & 3) as usize];
            let skipped = IGNORED_PASSES[horizontal + if meta & 4 != 0 { 4 } else { 0 }];
            RayShape::Stairs(StairsPasses { skipped })
        }
        _ => RayShape::Bounds(sel.bounds),
    }
}

/// `TorchBlock.rayTrace`'s bounds (float maths, widened): facing from metadata 1 east, 2 west,
/// 3 south, 4 north, otherwise standing.
fn torch_bounds(meta: u8) -> RayShape {
    let f = 0.15_f32;
    let b: [f32; 6] = match meta {
        1 => [0.0, 0.2, 0.5 - f, f * 2.0, 0.8, 0.5 + f],
        2 => [1.0 - f * 2.0, 0.2, 0.5 - f, 1.0, 0.8, 0.5 + f],
        3 => [0.5 - f, 0.2, 0.0, 0.5 + f, 0.8, f * 2.0],
        4 => [0.5 - f, 0.2, 1.0 - f * 2.0, 0.5 + f, 0.8, 1.0],
        _ => {
            let f = 0.1_f32;
            [0.5 - f, 0.0, 0.5 - f, 0.5 + f, 0.6, 0.5 + f]
        }
    };
    let b = b.map(f64::from);
    RayShape::Bounds(Aabb::new(b[0], b[1], b[2], b[3], b[4], b[5]))
}
