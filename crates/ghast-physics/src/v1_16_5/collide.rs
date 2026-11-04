//! 1.16.5 movement collision: block shape streams, the world border shape, the two axis-sweep
//! paths (shapes placed in the world for multi-axis moves; boxes shifted into block space for
//! single-axis moves) and step-up.

use ghast_data::v1_16_5::BlockState;

use super::blocks::{
    Shape, ShapeCtx, class, collision_class, collision_shape, exceeds_cube, state_at,
};
use super::fluid::V3;
use crate::math::{Aabb, floor, java_max, java_min};
use crate::voxel::{EPSILON, Points, VoxelShape};
use crate::world::{BlockView, WorldBorder};

/// Vanilla `Box` constructor (each axis sorted with Java `Math.min` / `Math.max`).
#[inline]
pub(crate) fn bx(x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> Aabb {
    Aabb::new(
        java_min(x0, x1),
        java_min(y0, y1),
        java_min(z0, z1),
        java_max(x0, x1),
        java_max(y0, y1),
        java_max(z0, z1),
    )
}

#[inline]
pub(crate) fn offset(b: &Aabb, x: f64, y: f64, z: f64) -> Aabb {
    bx(
        b.min_x + x,
        b.min_y + y,
        b.min_z + z,
        b.max_x + x,
        b.max_y + y,
        b.max_z + z,
    )
}

#[inline]
pub(crate) fn expand(b: &Aabb, v: f64) -> Aabb {
    bx(
        b.min_x - v,
        b.min_y - v,
        b.min_z - v,
        b.max_x + v,
        b.max_y + v,
        b.max_z + v,
    )
}

#[inline]
pub(crate) fn contract(b: &Aabb, v: f64) -> Aabb {
    expand(b, -v)
}

#[inline]
pub(crate) fn stretch(b: &Aabb, v: V3) -> Aabb {
    let (mut x0, mut y0, mut z0, mut x1, mut y1, mut z1) =
        (b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z);
    if v.x < 0.0 {
        x0 += v.x;
    } else if v.x > 0.0 {
        x1 += v.x;
    }
    if v.y < 0.0 {
        y0 += v.y;
    } else if v.y > 0.0 {
        y1 += v.y;
    }
    if v.z < 0.0 {
        z0 += v.z;
    } else if v.z > 0.0 {
        z1 += v.z;
    }
    bx(x0, y0, z0, x1, y1, z1)
}

/// 1.16 `Box.intersects(double...)` (strict, positive overlap).
#[inline]
fn intersects(b: &Aabb, x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> bool {
    b.min_x < x1 && b.max_x > x0 && b.min_y < y1 && b.max_y > y0 && b.min_z < z1 && b.max_z > z0
}

/// The world border as a collision shape: everything outside the (block-rounded) border.
pub(crate) struct BorderShape {
    xs: [f64; 4],
    ys: [f64; 2],
    zs: [f64; 4],
}

/// Cells of the border grid (3 × 1 × 3) minus the centre column.
const BORDER_CELLS: [u64; 1] = [0b1_1110_1111];

impl BorderShape {
    pub(crate) fn new(b: &WorldBorder) -> BorderShape {
        BorderShape {
            xs: [
                f64::NEG_INFINITY,
                b.west().floor(),
                b.east().ceil(),
                f64::INFINITY,
            ],
            ys: [f64::NEG_INFINITY, f64::INFINITY],
            zs: [
                f64::NEG_INFINITY,
                b.north().floor(),
                b.south().ceil(),
                f64::INFINITY,
            ],
        }
    }

    fn shape(&self) -> VoxelShape<'_> {
        VoxelShape {
            axes: [
                Points::Explicit(&self.xs),
                Points::Explicit(&self.ys),
                Points::Explicit(&self.zs),
            ],
            cells: &BORDER_CELLS,
        }
    }

    /// Vanilla `isInWorldBorder`: the box is strictly inside the block-rounded border.
    fn contains(&self, b: &Aabb) -> bool {
        let (w, e, n, s) = (self.xs[1], self.xs[2], self.zs[1], self.zs[2]);
        b.min_x > w
            && b.min_x < e
            && b.min_z > n
            && b.min_z < s
            && b.max_x > w
            && b.max_x < e
            && b.max_z > n
            && b.max_z < s
    }
}

/// A shape placed in the world.
#[derive(Clone, Copy)]
enum Placed<'b> {
    Block(VoxelShape<'static>, [f64; 3]),
    Border(&'b BorderShape),
}

impl Placed<'_> {
    #[inline]
    fn max_distance(&self, axis: usize, b: &Aabb, max: f64) -> f64 {
        match self {
            Placed::Block(s, o) => s.max_distance(axis, b, max, Some(*o)),
            Placed::Border(bs) => bs.shape().max_distance(axis, b, max, None),
        }
    }
}

/// Collision queries of one player for one tick.
pub(crate) struct Collider<'w, W> {
    pub w: &'w W,
    pub border: &'w BorderShape,
    pub ctx: ShapeCtx,
}

impl<'w, W: BlockView<State = BlockState>> Collider<'w, W> {
    /// Vanilla `BlockCollisionSpliterator` for the player whose box is `entity_box`: the border
    /// shape (when the player is near it) and every block shape overlapping `query`, in order.
    /// `pred` filters blocks (`isBlockSpaceEmpty`); `f` returns `false` to stop.
    fn each_collision(
        &self,
        entity_box: &Aabb,
        query: &Aabb,
        pred: impl Fn(BlockState) -> bool,
        mut f: impl FnMut(Placed<'w>) -> bool,
    ) {
        if !self.border.contains(entity_box) {
            let s = self.border.shape();
            if !s.intersects(&contract(entity_box, EPSILON), None)
                && s.intersects(&expand(entity_box, EPSILON), None)
                && !f(Placed::Border(self.border))
            {
                return;
            }
        }
        let x0 = floor(query.min_x - EPSILON) - 1;
        let x1 = floor(query.max_x + EPSILON) + 1;
        let y0 = floor(query.min_y - EPSILON) - 1;
        let y1 = floor(query.max_y + EPSILON) + 1;
        let z0 = floor(query.min_z - EPSILON) - 1;
        let z1 = floor(query.max_z + EPSILON) + 1;
        // Vanilla's cuboid iteration order: x fastest, then y, then z.
        const ROW: usize = 64;
        let row = (x1 - x0 + 1) as usize;
        let mut loaded = [false; ROW];
        for z in z0..=z1 {
            if row <= ROW {
                for (i, l) in loaded[..row].iter_mut().enumerate() {
                    *l = self.w.is_chunk_loaded((x0 + i as i32) >> 4, z >> 4);
                }
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let column_loaded = if row <= ROW {
                        loaded[(x - x0) as usize]
                    } else {
                        self.w.is_chunk_loaded(x >> 4, z >> 4)
                    };
                    if !column_loaded {
                        continue;
                    }
                    let edges = u8::from(x == x0 || x == x1)
                        + u8::from(y == y0 || y == y1)
                        + u8::from(z == z0 || z == z1);
                    if edges == 3 {
                        continue;
                    }
                    let s = state_at(self.w, x, y, z);
                    let c = collision_class(s);
                    // An empty shape never collides whatever the other filters say.
                    if c & class::NONEMPTY == 0
                        || (edges == 1 && c & class::EXCEEDS == 0)
                        || (edges == 2 && c & class::MOVING_PISTON == 0)
                        || !pred(s)
                    {
                        continue;
                    }
                    let (shape, full_cube) = if c & class::FULL_CUBE != 0 {
                        (VoxelShape::FULL_CUBE, true)
                    } else {
                        match collision_shape(s, x, y, z, Some(&self.ctx)) {
                            Shape::Grid { shape, full_cube } => (shape, full_cube),
                            Shape::Empty => continue,
                        }
                    };
                    let o = [f64::from(x), f64::from(y), f64::from(z)];
                    let hit = if full_cube {
                        intersects(query, o[0], o[1], o[2], o[0] + 1.0, o[1] + 1.0, o[2] + 1.0)
                    } else {
                        shape.intersects(query, Some(o))
                    };
                    if hit && !f(Placed::Block(shape, o)) {
                        return;
                    }
                }
            }
        }
    }

    /// Every block shape vanilla's `getCollisions(entity, box)` yields, in order, as
    /// `(state, block position)` (the world-border shape is left out).
    pub(crate) fn each_block_collision(
        &self,
        entity_box: &Aabb,
        query: &Aabb,
        mut f: impl FnMut([i32; 3]) -> bool,
    ) {
        self.each_collision(
            entity_box,
            query,
            |_| true,
            |p| match p {
                Placed::Block(_, o) => f([o[0] as i32, o[1] as i32, o[2] as i32]),
                Placed::Border(_) => true,
            },
        );
    }

    /// Vanilla `isSpaceEmpty(entity, box)`.
    pub(crate) fn is_space_empty(&self, entity_box: &Aabb, query: &Aabb) -> bool {
        let mut empty = true;
        self.each_collision(
            entity_box,
            query,
            |_| true,
            |_| {
                empty = false;
                false
            },
        );
        empty
    }

    /// Vanilla `isBlockSpaceEmpty(entity, box, predicate)`.
    pub(crate) fn is_block_space_empty(
        &self,
        entity_box: &Aabb,
        query: &Aabb,
        pred: impl Fn(BlockState) -> bool,
    ) -> bool {
        let mut empty = true;
        self.each_collision(entity_box, query, pred, |_| {
            empty = false;
            false
        });
        empty
    }

    /// Vanilla `Entity.adjustMovementForCollisions(movement)` including step-up.
    pub(crate) fn adjust(&self, movement: V3, entity_box: &Aabb, on_ground: bool, step: f32) -> V3 {
        let b = *entity_box;
        let border_extra = !self.border.shape().intersects(&contract(&b, EPSILON), None);
        let v = if movement.length_sq() == 0.0 {
            movement
        } else {
            self.adjust_static(movement, &b, entity_box, border_extra)
        };
        let changed_x = movement.x != v.x;
        let changed_y = movement.y != v.y;
        let changed_z = movement.z != v.z;
        let may_step = on_ground || changed_y && movement.y < 0.0;
        let step = f64::from(step);
        if step > 0.0 && may_step && (changed_x || changed_z) {
            let mut stepped = self.adjust_static(
                V3::new(movement.x, step, movement.z),
                &b,
                entity_box,
                border_extra,
            );
            let up = self.adjust_static(
                V3::new(0.0, step, 0.0),
                &stretch(&b, V3::new(movement.x, 0.0, movement.z)),
                entity_box,
                border_extra,
            );
            if up.y < step {
                let across = self
                    .adjust_static(
                        V3::new(movement.x, 0.0, movement.z),
                        &offset(&b, up.x, up.y, up.z),
                        entity_box,
                        border_extra,
                    )
                    .add(up);
                if horizontal_sq(across) > horizontal_sq(stepped) {
                    stepped = across;
                }
            }
            if horizontal_sq(stepped) > horizontal_sq(v) {
                let down = self.adjust_static(
                    V3::new(0.0, -stepped.y + movement.y, 0.0),
                    &offset(&b, stepped.x, stepped.y, stepped.z),
                    entity_box,
                    border_extra,
                );
                return stepped.add(down);
            }
        }
        v
    }

    /// Vanilla static `adjustMovementForCollisions(entity, movement, box, world, ctx, extra)`.
    fn adjust_static(&self, m: V3, b: &Aabb, entity_box: &Aabb, border_extra: bool) -> V3 {
        let zeros = u8::from(m.x == 0.0) + u8::from(m.y == 0.0) + u8::from(m.z == 0.0);
        if zeros <= 1 {
            self.sweep_shapes(m, b, entity_box, border_extra)
        } else {
            self.sweep_push(m, b, border_extra)
        }
    }

    /// Axis order Y, then the smaller of X / Z last.
    fn sweep_shapes(&self, m: V3, b: &Aabb, entity_box: &Aabb, border_extra: bool) -> V3 {
        let query = stretch(b, m);
        // Vanilla collects the shapes once (a reusable stream) and reuses them for every axis.
        let mut shapes: Vec<Placed<'w>> = Vec::with_capacity(16);
        if border_extra {
            shapes.push(Placed::Border(self.border));
        }
        self.each_collision(
            entity_box,
            &query,
            |_| true,
            |p| {
                shapes.push(p);
                true
            },
        );
        let max_offset = |axis: usize, bb: &Aabb, mut d: f64| -> f64 {
            for p in &shapes {
                if d.abs() < EPSILON {
                    return 0.0;
                }
                d = p.max_distance(axis, bb, d);
            }
            d
        };
        let (mut dx, mut dy, mut dz) = (m.x, m.y, m.z);
        let mut bb = *b;
        if dy != 0.0 {
            dy = max_offset(1, &bb, dy);
            if dy != 0.0 {
                bb = offset(&bb, 0.0, dy, 0.0);
            }
        }
        let z_first = dx.abs() < dz.abs();
        if z_first && dz != 0.0 {
            dz = max_offset(2, &bb, dz);
            if dz != 0.0 {
                bb = offset(&bb, 0.0, 0.0, dz);
            }
        }
        if dx != 0.0 {
            dx = max_offset(0, &bb, dx);
            if !z_first && dx != 0.0 {
                bb = offset(&bb, dx, 0.0, 0.0);
            }
        }
        if !z_first && dz != 0.0 {
            dz = max_offset(2, &bb, dz);
        }
        V3::new(dx, dy, dz)
    }

    /// Single-axis movement (vanilla `calculatePushVelocity`): blocks are queried with the box
    /// shifted into block-local space.
    fn sweep_push(&self, m: V3, b: &Aabb, border_extra: bool) -> V3 {
        let (mut dx, mut dy, mut dz) = (m.x, m.y, m.z);
        let mut bb = *b;
        if dy != 0.0 {
            dy = self.push_axis(1, &bb, dy, border_extra);
            if dy != 0.0 {
                bb = offset(&bb, 0.0, dy, 0.0);
            }
        }
        let z_first = dx.abs() < dz.abs();
        if z_first && dz != 0.0 {
            dz = self.push_axis(2, &bb, dz, border_extra);
            if dz != 0.0 {
                bb = offset(&bb, 0.0, 0.0, dz);
            }
        }
        if dx != 0.0 {
            dx = self.push_axis(0, &bb, dx, border_extra);
            if !z_first && dx != 0.0 {
                bb = offset(&bb, dx, 0.0, 0.0);
            }
        }
        if !z_first && dz != 0.0 {
            dz = self.push_axis(2, &bb, dz, border_extra);
        }
        V3::new(dx, dy, dz)
    }

    fn push_axis(&self, axis: usize, b: &Aabb, initial: f64, border_extra: bool) -> f64 {
        let lo = [b.min_x, b.min_y, b.min_z];
        let hi = [b.max_x, b.max_y, b.max_z];
        if (0..3).any(|a| hi[a] - lo[a] < 1.0e-6) {
            return initial;
        }
        if initial.abs() < EPSILON {
            return 0.0;
        }
        // The two other axes in vanilla's cycle order, then the moving axis.
        let (a1, a2) = ((axis + 1) % 3, (axis + 2) % 3);
        let i = floor(lo[a1] - EPSILON) - 1;
        let j = floor(hi[a1] + EPSILON) + 1;
        let k = floor(lo[a2] - EPSILON) - 1;
        let l = floor(hi[a2] + EPSILON) + 1;
        let d = lo[axis] - EPSILON;
        let e = hi[axis] + EPSILON;
        let positive = initial > 0.0;
        let m = if positive {
            floor(hi[axis] - EPSILON) - 1
        } else {
            floor(lo[axis] + EPSILON) + 1
        };
        let clamp = |v: f64| {
            if v > 0.0 {
                floor(e + v) + 1
            } else {
                floor(d + v) - 1
            }
        };
        let mut n = clamp(initial);
        let step = if positive { 1 } else { -1 };
        let mut initial = initial;
        let mut p = m;
        while if positive { p <= n } else { p >= n } {
            for q in i..=j {
                for r in k..=l {
                    let edges = u8::from(q == i || q == j)
                        + u8::from(r == k || r == l)
                        + u8::from(p == m || p == n);
                    if edges >= 3 {
                        continue;
                    }
                    let mut c = [0i32; 3];
                    c[axis] = p;
                    c[a1] = q;
                    c[a2] = r;
                    let s = state_at(self.w, c[0], c[1], c[2]);
                    if (edges == 1 && !exceeds_cube(s))
                        || (edges == 2 && s.block() != ghast_data::v1_16_5::Block::MOVING_PISTON)
                    {
                        continue;
                    }
                    let shape = match collision_shape(s, c[0], c[1], c[2], Some(&self.ctx)) {
                        Shape::Grid { shape, .. } => shape,
                        Shape::Empty => continue,
                    };
                    let local = offset(b, -f64::from(c[0]), -f64::from(c[1]), -f64::from(c[2]));
                    initial = shape.max_distance(axis, &local, initial, None);
                    if initial.abs() < EPSILON {
                        return 0.0;
                    }
                    n = clamp(initial);
                }
            }
            p += step;
        }
        if border_extra {
            initial = Placed::Border(self.border).max_distance(axis, b, initial);
        }
        initial
    }
}

#[inline]
pub(crate) fn horizontal_sq(v: V3) -> f64 {
    v.x * v.x + v.z * v.z
}
