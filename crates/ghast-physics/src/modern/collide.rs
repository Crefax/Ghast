//! Block collision of the modern client: vanilla's `BlockCollisions` cursor, the world border
//! shape, `collideWithShapes`, step-up by candidate heights, `noCollision`, the suffocation probe
//! and the supporting-block search.

// Vanilla's negated float comparisons are kept as written: they decide the NaN cases.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use ghast_data::v26_1::{BlockKind, StatePhys, VoxelClass, VoxelShapeRef, state_flags as sf};

use super::math::{V3, bx, expand_towards, floor, hi, intersects, java_max, java_min, lo, mv};
use super::rules::Rules;
use crate::math::Aabb;
use crate::voxel::{EPSILON, Points, VoxelShape};
use crate::world::{BlockView, WorldBorder};

/// What a block's collision shape may depend on from the colliding player
/// (`EntityCollisionContext`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShapeCtx {
    /// `entity.getY()` when the context was made.
    pub feet_y: f64,
    /// `isDescending()` (shift key).
    pub descending: bool,
    /// `fallDistance` (powder snow).
    pub fall_distance: f64,
    /// Leather boots in the feet slot (powder snow walkable).
    pub leather_boots: bool,
}

/// The world-border collision shape (everything outside the block-rounded border) and the
/// border's exact bounds.
pub(crate) struct Border {
    xs: [f64; 4],
    ys: [f64; 2],
    zs: [f64; 4],
    min_x: f64,
    max_x: f64,
    min_z: f64,
    max_z: f64,
}

/// The `Shapes.INFINITY` grid.
const INFINITE_POINTS: [f64; 2] = [f64::NEG_INFINITY, f64::INFINITY];

/// Cells of the 3 x 1 x 3 border grid minus the centre column.
const BORDER_CELLS: [u64; 1] = [0b1_1110_1111];

impl Border {
    pub(crate) fn new(b: &WorldBorder) -> Border {
        // 26.x clamps each bound to [-29,999,984, 29,999,984] on both sides (`Mth.clamp`); the
        // shared `WorldBorder` accessors clamp one side only, as the older versions do.
        // (`Mth.clamp` written out: with bounds of +-M it is the same for every value, NaN
        // included, and cheaper than `java_min`; the plain tick builds this every tick.)
        let max = 29_999_984.0;
        let c = |v: f64| {
            if v < -max {
                -max
            } else if v > max {
                max
            } else {
                v
            }
        };
        let (west, east) = (c(b.center_x - b.size / 2.0), c(b.center_x + b.size / 2.0));
        let (north, south) = (c(b.center_z - b.size / 2.0), c(b.center_z + b.size / 2.0));
        Border {
            xs: [f64::NEG_INFINITY, west.floor(), east.ceil(), f64::INFINITY],
            ys: [f64::NEG_INFINITY, f64::INFINITY],
            zs: [
                f64::NEG_INFINITY,
                north.floor(),
                south.ceil(),
                f64::INFINITY,
            ],
            min_x: west,
            max_x: east,
            min_z: north,
            max_z: south,
        }
    }

    /// The block-rounded inside is thinner than `1e-7` (a clamped border): `Shapes.box` gives the
    /// empty shape, so the border shape is `Shapes.INFINITY`. (Computed here, not in `new`: the
    /// shape is only needed near the border.)
    fn full(&self) -> bool {
        self.xs[2] - self.xs[1] < EPSILON || self.zs[2] - self.zs[1] < EPSILON
    }

    pub(crate) fn shape(&self) -> VoxelShape<'_> {
        if self.full() {
            return VoxelShape {
                axes: [
                    Points::Explicit(&INFINITE_POINTS),
                    Points::Explicit(&INFINITE_POINTS),
                    Points::Explicit(&INFINITE_POINTS),
                ],
                cells: &[1],
            };
        }
        VoxelShape {
            axes: [
                Points::Explicit(&self.xs),
                Points::Explicit(&self.ys),
                Points::Explicit(&self.zs),
            ],
            cells: &BORDER_CELLS,
        }
    }

    /// `WorldBorder.isInsideCloseToBorder(source, box)` for a source at `(x, z)`.
    pub(crate) fn inside_close(&self, x: f64, z: f64, query: &Aabb) -> bool {
        let size = java_max(
            (query.max_x - query.min_x).abs(),
            (query.max_z - query.min_z).abs(),
        );
        let bb_max = java_max(size, 1.0);
        let dist = java_min(
            java_min(java_min(x - self.min_x, self.max_x - x), z - self.min_z),
            self.max_z - z,
        );
        dist < bb_max * 2.0
            && x >= self.min_x - bb_max
            && x < self.max_x + bb_max
            && z >= self.min_z - bb_max
            && z < self.max_z + bb_max
    }
}

/// A collision shape placed in the world.
#[derive(Clone, Copy)]
pub(crate) enum Placed {
    /// `Shapes.block()` at a block position (corner).
    Cube([f64; 3]),
    /// Any other block shape (table entry, optional model offset), moved to its block position.
    Shape(&'static VoxelShapeRef, Option<[f64; 3]>, [f64; 3]),
    /// The world-border shape.
    Border,
}

/// `Shapes.create(box)` of an entity box as a one-cell shape with explicit points.
fn entity_shape_collide(e: &Aabb, axis: usize, b: &Aabb, d: f64) -> f64 {
    let xs = [e.min_x, e.max_x];
    let ys = [e.min_y, e.max_y];
    let zs = [e.min_z, e.max_z];
    VoxelShape {
        axes: [
            Points::Explicit(&xs),
            Points::Explicit(&ys),
            Points::Explicit(&zs),
        ],
        cells: &[1],
    }
    .max_distance(axis, b, d, None)
}

/// `true` when the entity box is too thin in some axis (`Shapes.create` gives the empty shape).
fn entity_shape_empty(e: &Aabb) -> bool {
    e.max_x - e.min_x < EPSILON || e.max_y - e.min_y < EPSILON || e.max_z - e.min_z < EPSILON
}

/// A voxel table entry as a [`VoxelShape`] (moved shapes always binary-search their points).
pub(crate) fn voxel(r: &'static VoxelShapeRef, shift: Option<[f64; 3]>) -> VoxelShape<'static> {
    let axis = |k: usize, n: u8, pts: &'static [f64]| match (shift, r.class) {
        (Some(s), _) => Points::Shifted(pts, s[k]),
        (None, VoxelClass::Simple) => Points::Fractional(u32::from(n)),
        (None, VoxelClass::Array) => Points::Explicit(pts),
    };
    VoxelShape {
        axes: [
            axis(0, r.sizes[0], r.xs),
            axis(1, r.sizes[1], r.ys),
            axis(2, r.sizes[2], r.zs),
        ],
        cells: r.cells,
    }
}

/// `VoxelShape.collide` of a full cube placed at `o` (vanilla's generic grid walk specialised to
/// one cell; identical results).
#[inline]
fn cube_collide(axis: usize, b: &Aabb, d: f64, o: [f64; 3]) -> f64 {
    if d.abs() < EPSILON {
        return 0.0;
    }
    let (a2, a3) = ((axis + 1) % 3, (axis + 2) % 3);
    // findIndex over the moved points [o, o + 1]: -1 below, 0 inside, 1 at or above the top.
    let overlaps = |ax: usize| {
        let p0 = 0.0 + o[ax];
        let p1 = 1.0 + o[ax];
        lo(b, ax) + EPSILON < p1 && hi(b, ax) - EPSILON >= p0
    };
    if !overlaps(a2) || !overlaps(a3) {
        return d;
    }
    let p0 = 0.0 + o[axis];
    let p1 = 1.0 + o[axis];
    if d > 0.0 {
        let max_a = hi(b, axis);
        if max_a - EPSILON < p0 {
            let gap = p0 - max_a;
            if gap >= -EPSILON {
                return java_min(d, gap);
            }
        }
        d
    } else {
        let min_a = lo(b, axis);
        if min_a + EPSILON >= p1 {
            let gap = p1 - min_a;
            if gap <= EPSILON {
                return java_max(d, gap);
            }
        }
        d
    }
}

/// One axis of a placed table shape: point `i` is `base(i) [+ model offset] + block position`,
/// the double additions vanilla's `OffsetDoubleList`s make.
#[derive(Clone, Copy)]
struct ShapeAxis {
    pts: &'static [f64],
    n: usize,
    simple: bool,
    shift: Option<f64>,
    o: f64,
}

impl ShapeAxis {
    #[inline]
    fn new(
        r: &'static VoxelShapeRef,
        axis: usize,
        shift: Option<[f64; 3]>,
        o: [f64; 3],
    ) -> ShapeAxis {
        let (pts, n) = match axis {
            0 => (r.xs, usize::from(r.sizes[0])),
            1 => (r.ys, usize::from(r.sizes[1])),
            _ => (r.zs, usize::from(r.sizes[2])),
        };
        ShapeAxis {
            pts,
            n,
            simple: r.class == VoxelClass::Simple && shift.is_none(),
            shift: shift.map(|s| s[axis]),
            o: o[axis],
        }
    }

    #[inline]
    fn point(&self, i: usize) -> f64 {
        let base = if self.simple {
            i as f64 / self.n as f64
        } else {
            self.pts[i]
        };
        match self.shift {
            Some(s) => (base + s) + self.o,
            None => base + self.o,
        }
    }

    /// `findIndex` of a moved shape: the binary search of `Mth.binarySearch(0, n + 1, ...)`.
    #[inline]
    fn find_index(&self, coord: f64) -> isize {
        let (mut from, mut len) = (0usize, self.n + 1);
        while len > 0 {
            let half = len / 2;
            let mid = from + half;
            if coord < self.point(mid) {
                len = half;
            } else {
                from = mid + 1;
                len -= half + 1;
            }
        }
        from as isize - 1
    }
}

#[inline]
fn filled_axes(r: &VoxelShapeRef, axis: usize, a: usize, b: usize, c: usize) -> bool {
    // (a, b, c) are the moving axis and the next two in cycle order.
    let mut x = [0usize; 3];
    x[axis] = a;
    x[(axis + 1) % 3] = b;
    x[(axis + 2) % 3] = c;
    r.filled(x[0], x[1], x[2])
}

/// `VoxelShape.collide(axis, box, distance)` of a moved table shape (vanilla `collideX`).
fn shape_collide(
    r: &'static VoxelShapeRef,
    shift: Option<[f64; 3]>,
    o: [f64; 3],
    axis: usize,
    b: &Aabb,
    d: f64,
) -> f64 {
    if r.empty {
        return d;
    }
    if d.abs() < EPSILON {
        return 0.0;
    }
    let (ax2, ax3) = ((axis + 1) % 3, (axis + 2) % 3);
    let pa = ShapeAxis::new(r, axis, shift, o);
    let pb = ShapeAxis::new(r, ax2, shift, o);
    let pc = ShapeAxis::new(r, ax3, shift, o);
    let (max_a, min_a) = (hi(b, axis), lo(b, axis));
    let a_min = pa.find_index(min_a + EPSILON);
    let a_max = pa.find_index(max_a - EPSILON);
    let b_min = pb.find_index(lo(b, ax2) + EPSILON).max(0);
    let b_max = (pb.find_index(hi(b, ax2) - EPSILON) + 1).min(pb.n as isize);
    let c_min = pc.find_index(lo(b, ax3) + EPSILON).max(0);
    let c_max = (pc.find_index(hi(b, ax3) - EPSILON) + 1).min(pc.n as isize);
    let layer = |a: isize| {
        (b_min..b_max).any(|bb| {
            (c_min..c_max).any(|cc| filled_axes(r, axis, a as usize, bb as usize, cc as usize))
        })
    };
    if d > 0.0 {
        for a in a_max + 1..pa.n as isize {
            if layer(a) {
                let gap = pa.point(a as usize) - max_a;
                return if gap >= -EPSILON { java_min(d, gap) } else { d };
            }
        }
    } else if d < 0.0 {
        for a in (0..a_min).rev() {
            if layer(a) {
                let gap = pa.point(a as usize + 1) - min_a;
                return if gap <= EPSILON { java_max(d, gap) } else { d };
            }
        }
    }
    d
}

/// `Shapes.joinIsNotEmpty(moved shape, Shapes.create(box), AND)`. When every shape point is
/// more than `2e-7` from the box bounds (nearly always) no point of the merge collapses and the
/// answer is a plain cell overlap test; otherwise vanilla's merge runs.
fn shape_intersects(
    r: &'static VoxelShapeRef,
    shift: Option<[f64; 3]>,
    o: [f64; 3],
    q: &Aabb,
) -> bool {
    if r.empty {
        return false;
    }
    let sep = 2.0 * EPSILON;
    let mut range = [(0isize, 0isize); 3];
    for (axis, rg) in range.iter_mut().enumerate() {
        let ax = ShapeAxis::new(r, axis, shift, o);
        let (b0, b1) = (lo(q, axis), hi(q, axis));
        if !(b1 - b0 > sep) {
            return voxel(r, shift).intersects(q, Some(o));
        }
        let (mut first, mut last) = (isize::MAX, -1isize);
        for i in 0..=ax.n {
            let p = ax.point(i);
            if !((p - b0).abs() > sep && (p - b1).abs() > sep) {
                return voxel(r, shift).intersects(q, Some(o));
            }
            // Cell i (between points i and i + 1) overlaps (b0, b1).
            if i < ax.n {
                let next = ax.point(i + 1);
                if p < b1 && next > b0 {
                    first = first.min(i as isize);
                    last = i as isize;
                }
            }
        }
        if last < 0 {
            return false;
        }
        *rg = (first, last);
    }
    for x in range[0].0..=range[0].1 {
        for y in range[1].0..=range[1].1 {
            for z in range[2].0..=range[2].1 {
                if r.filled(x as usize, y as usize, z as usize) {
                    return true;
                }
            }
        }
    }
    false
}

impl Placed {
    /// The shape's world boxes (`toAabbs()` of the moved shape), in vanilla's order.
    pub(crate) fn for_boxes(&self, mut f: impl FnMut(Aabb) -> bool) {
        match self {
            Placed::Cube(o) => {
                let _ = f(Aabb::new(
                    0.0 + o[0],
                    0.0 + o[1],
                    0.0 + o[2],
                    1.0 + o[0],
                    1.0 + o[1],
                    1.0 + o[2],
                ));
            }
            Placed::Shape(r, sh, o) => {
                let k = sh.unwrap_or([0.0; 3]);
                for b in r.boxes {
                    let w = match sh {
                        Some(_) => Aabb::new(
                            b.min_x + k[0] + o[0],
                            b.min_y + k[1] + o[1],
                            b.min_z + k[2] + o[2],
                            b.max_x + k[0] + o[0],
                            b.max_y + k[1] + o[1],
                            b.max_z + k[2] + o[2],
                        ),
                        None => Aabb::new(
                            b.min_x + o[0],
                            b.min_y + o[1],
                            b.min_z + o[2],
                            b.max_x + o[0],
                            b.max_y + o[1],
                            b.max_z + o[2],
                        ),
                    };
                    if !f(w) {
                        return;
                    }
                }
            }
            Placed::Border => {}
        }
    }

    /// `VoxelShape.max(Y)` of the block-local shape (model offset included, block position not).
    pub(crate) fn local_max_y(&self) -> Option<f64> {
        match self {
            Placed::Cube(_) => Some(1.0),
            Placed::Shape(r, sh, _) => {
                if r.empty {
                    return None;
                }
                let [nx, ny, nz] = r.sizes.map(usize::from);
                let top = (0..ny)
                    .rev()
                    .find(|&y| (0..nx).any(|x| (0..nz).any(|z| r.filled(x, y, z))))?;
                let s = voxel(r, *sh);
                Some(s.axes[1].get(top + 1))
            }
            Placed::Border => None,
        }
    }

    /// `true` for an empty shape.
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Placed::Shape(r, _, _) => r.empty,
            _ => false,
        }
    }

    #[inline(always)]
    pub(crate) fn collide(&self, border: &Border, axis: usize, b: &Aabb, d: f64) -> f64 {
        match self {
            Placed::Cube(o) => cube_collide(axis, b, d, *o),
            Placed::Shape(r, sh, o) => shape_collide(r, *sh, *o, axis, b, d),
            Placed::Border => border.shape().max_distance(axis, b, d, None),
        }
    }

    /// The shape's Y points (`getCoords(Y)` of the placed shape), passed one by one.
    pub(crate) fn for_y_coords(&self, border: &Border, mut f: impl FnMut(f64) -> bool) {
        match self {
            Placed::Cube(o) => {
                let _ = f(0.0 + o[1]) && f(1.0 + o[1]);
            }
            Placed::Shape(r, sh, o) => {
                let s = voxel(r, *sh);
                let n = s.axes[1].cells();
                for i in 0..=n {
                    if !f(s.axes[1].get(i) + o[1]) {
                        return;
                    }
                }
            }
            Placed::Border => {
                for &y in &border.ys {
                    if !f(y) {
                        return;
                    }
                }
            }
        }
    }
}

/// Fixed-capacity collider list that spills to the heap only for huge queries.
pub(crate) struct Colliders {
    buf: [Placed; INLINE],
    len: usize,
    spill: Vec<Placed>,
}

const INLINE: usize = 48;

impl Colliders {
    pub(crate) fn new() -> Colliders {
        Colliders {
            buf: [Placed::Border; INLINE],
            len: 0,
            spill: Vec::new(),
        }
    }

    #[inline]
    pub(crate) fn push(&mut self, p: Placed) {
        if self.len < INLINE {
            self.buf[self.len] = p;
            self.len += 1;
        } else {
            self.spill.push(p);
        }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub(crate) fn clear(&mut self) {
        self.len = 0;
        self.spill.clear();
    }

    #[inline]
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Placed> {
        self.buf[..self.len].iter().chain(self.spill.iter())
    }
}

/// `Shapes.collide(axis, box, shapes, distance)`.
#[inline]
pub(crate) fn shapes_collide(
    shapes: &Colliders,
    border: &Border,
    axis: usize,
    b: &Aabb,
    mut d: f64,
) -> f64 {
    for s in shapes.iter() {
        if d.abs() < EPSILON {
            return 0.0;
        }
        d = s.collide(border, axis, b, d);
    }
    d
}

/// `Entity.collideWithShapes(movement, box, shapes)`.
pub(crate) fn collide_with_shapes(m: V3, b: &Aabb, shapes: &Colliders, border: &Border) -> V3 {
    if shapes.is_empty() {
        return m;
    }
    let mut r = V3::ZERO;
    // Direction.axisStepOrder: Y, then the smaller of X / Z first... (YZX when |x| < |z|).
    let order: [usize; 3] = if m.x.abs() < m.z.abs() {
        [1, 2, 0]
    } else {
        [1, 0, 2]
    };
    for axis in order {
        let am = m.get(axis);
        if am != 0.0 {
            let c = shapes_collide(shapes, border, axis, &mv(b, r.x, r.y, r.z), am);
            r = r.with(axis, c);
        }
    }
    r
}

/// What a tick knows beyond blocks, as a type: [`Plain`] (the plain player: nothing, zero-sized,
/// every extension query is a compile-time constant) or [`Ents`] (the extended tick: the boxes of
/// the entities the player collides with). The plain tick then compiles to the code it was before
/// the extensions existed.
pub(crate) trait Extra: Copy {
    /// The extended tick (gliding, spin, entities can be present).
    const ON: bool;
    /// Boxes of the entities the player collides with (`canBeCollidedWith`).
    fn entities(&self) -> &[Aabb];
}

/// The plain player: no extensions.
#[derive(Clone, Copy, Default)]
pub(crate) struct Plain;

impl Extra for Plain {
    const ON: bool = false;
    #[inline(always)]
    fn entities(&self) -> &[Aabb] {
        &[]
    }
}

/// The extended tick with the entity boxes it collides with.
#[derive(Clone, Copy)]
pub(crate) struct Ents<'a>(pub &'a [Aabb]);

impl Extra for Ents<'_> {
    const ON: bool = true;
    #[inline(always)]
    fn entities(&self) -> &[Aabb] {
        self.0
    }
}

/// The block collision queries of one player for one tick.
pub(crate) struct World<'a, R: Rules, W, E: Extra = Plain> {
    pub w: &'a W,
    pub border: &'a Border,
    /// Build height `[min_y, max_y]`.
    pub min_y: i32,
    pub max_y: i32,
    /// The extension surroundings (`Plain`: none, a zero-sized type).
    pub ext: E,
    pub _r: core::marker::PhantomData<R>,
}

impl<R: Rules, W: BlockView<State = R::State>, E: Extra> World<'_, R, W, E> {
    /// `Level.getBlockState`: air outside the build height (and in unloaded columns, which the
    /// world view returns as air).
    #[inline]
    pub(crate) fn state(&self, x: i32, y: i32, z: i32) -> R::State {
        if y < self.min_y || y > self.max_y {
            R::AIR
        } else {
            self.w.block(x, y, z)
        }
    }

    #[inline]
    pub(crate) fn phys(&self, x: i32, y: i32, z: i32) -> StatePhys {
        R::phys(self.state(x, y, z))
    }

    /// The collision shape of a state at a position for a context (`None` = the empty context).
    #[inline]
    pub(crate) fn shape_of_state(
        &self,
        s: R::State,
        x: i32,
        y: i32,
        z: i32,
        ctx: Option<&ShapeCtx>,
    ) -> Option<Placed> {
        let p = R::phys(s);
        let f = p.flags();
        let o = [f64::from(x), f64::from(y), f64::from(z)];
        if f & sf::FULL_CUBE != 0 {
            return Some(Placed::Cube(o));
        }
        if f & sf::CONTEXT != 0 {
            return context_shape::<R>(p, y, ctx).map(|i| {
                if i == R::FULL_CUBE_VOXEL {
                    Placed::Cube(o)
                } else {
                    Placed::Shape(R::voxel(i), None, o)
                }
            });
        }
        if f & sf::EMPTY != 0 {
            return None;
        }
        let r = R::voxel(p.shape());
        if f & sf::OFFSET != 0 {
            let shift = model_offset(R::block(s).offset, x, z);
            return Some(Placed::Shape(r, Some(shift), o));
        }
        Some(Placed::Shape(r, None, o))
    }

    /// Vanilla's `BlockCollisions` over `query`: calls `f(x, y, z, state, shape)` for every block
    /// shape that collides, in cursor order; `f` returns `false` to stop.
    #[inline]
    pub(crate) fn each_collision(
        &self,
        query: &Aabb,
        ctx: Option<&ShapeCtx>,
        only_suffocating: bool,
        mut f: impl FnMut(i32, i32, i32, Placed) -> bool,
    ) {
        let x0 = floor(query.min_x - EPSILON) - 1;
        let x1 = floor(query.max_x + EPSILON) + 1;
        let y0 = floor(query.min_y - EPSILON) - 1;
        let y1 = floor(query.max_y + EPSILON) + 1;
        let z0 = floor(query.min_z - EPSILON) - 1;
        let z1 = floor(query.max_z + EPSILON) + 1;
        // Rows of the cursor outside the build height hold only air.
        let (ya, yb) = (y0.max(self.min_y), y1.min(self.max_y));
        for z in z0..=z1 {
            let ez = u8::from(z == z0 || z == z1);
            let mut chunk = (i32::MIN, false);
            for y in ya..=yb {
                let ey = u8::from(y == y0 || y == y1);
                for x in x0..=x1 {
                    let edges = u8::from(x == x0 || x == x1) + ey + ez;
                    if edges == 3 {
                        continue;
                    }
                    let cx = x >> 4;
                    if chunk.0 != cx {
                        chunk = (cx, self.w.is_chunk_loaded(cx, z >> 4));
                    }
                    if !chunk.1 {
                        continue;
                    }
                    let s = self.w.block(x, y, z);
                    let p = R::phys(s);
                    let fl = p.flags();
                    if (only_suffocating && fl & sf::SUFFOCATING == 0)
                        || (edges == 1 && fl & sf::LARGE == 0)
                        || (edges == 2 && fl & sf::MOVING_PISTON == 0)
                    {
                        continue;
                    }
                    // Empty context-free shapes never collide (context / offset shapes are
                    // resolved below).
                    if fl & (sf::EMPTY | sf::CONTEXT) == sf::EMPTY {
                        continue;
                    }
                    let Some(shape) = self.shape_of_state(s, x, y, z, ctx) else {
                        continue;
                    };
                    let hit = match shape {
                        Placed::Cube(o) => {
                            intersects(query, o[0], o[1], o[2], o[0] + 1.0, o[1] + 1.0, o[2] + 1.0)
                        }
                        Placed::Shape(r, sh, o) => shape_intersects(r, sh, o, query),
                        Placed::Border => false,
                    };
                    if hit && !f(x, y, z, shape) {
                        return;
                    }
                }
            }
        }
    }

    /// `collidesWithSuffocatingBlock(box)` for the client push-out probe of one block column.
    ///
    /// Only the cursor's interior cells are read: face and edge cells count only for suffocating
    /// states with large shapes (or moving pistons), and every such state's shape stays inside
    /// its own cube (`ghast-data` test `suffocating_large_shapes_stay_in_their_cube`), so it can
    /// never reach a probe area that lies inside the column.
    pub(crate) fn suffocates_in_column(&self, query: &Aabb, ctx: &ShapeCtx) -> bool {
        let x0 = floor(query.min_x - EPSILON) - 1;
        let x1 = floor(query.max_x + EPSILON) + 1;
        let y0 = floor(query.min_y - EPSILON) - 1;
        let y1 = floor(query.max_y + EPSILON) + 1;
        let z0 = floor(query.min_z - EPSILON) - 1;
        let z1 = floor(query.max_z + EPSILON) + 1;
        for z in z0 + 1..z1 {
            for y in (y0 + 1).max(self.min_y)..y1.min(self.max_y + 1) {
                for x in x0 + 1..x1 {
                    if !self.w.is_chunk_loaded(x >> 4, z >> 4) {
                        continue;
                    }
                    let s = self.w.block(x, y, z);
                    let p = R::phys(s);
                    if p.flags() & sf::SUFFOCATING == 0 {
                        continue;
                    }
                    let Some(shape) = self.shape_of_state(s, x, y, z, Some(ctx)) else {
                        continue;
                    };
                    let hit = match shape {
                        Placed::Cube(o) => {
                            intersects(query, o[0], o[1], o[2], o[0] + 1.0, o[1] + 1.0, o[2] + 1.0)
                        }
                        Placed::Shape(r, sh, o) => shape_intersects(r, sh, o, query),
                        Placed::Border => false,
                    };
                    if hit {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// `noCollision(entity, box)`: no block shape, no entity, no world-border overlap.
    pub(crate) fn no_collision(&self, src: [f64; 3], query: &Aabb, ctx: &ShapeCtx) -> bool {
        let mut empty = true;
        self.each_collision(query, Some(ctx), false, |_, _, _, _| {
            empty = false;
            false
        });
        if !empty {
            return false;
        }
        if E::ON && !self.ext.entities().is_empty() && self.any_entity_collider(query) {
            return false;
        }
        if self.border.inside_close(src[0], src[2], query) {
            return !self.border.shape().intersects(query, None);
        }
        true
    }

    #[cold]
    #[inline(never)]
    fn any_entity_collider(&self, area: &Aabb) -> bool {
        self.entity_colliders(area).next().is_some()
    }

    /// `getEntityCollisions(entity, area)`: the collidable entity boxes intersecting the area grown
    /// by `1e-7` (none for an area smaller than `1e-7` on average).
    pub(crate) fn entity_colliders<'q>(&'q self, area: &Aabb) -> impl Iterator<Item = Aabb> + 'q {
        let size =
            ((area.max_x - area.min_x) + (area.max_y - area.min_y) + (area.max_z - area.min_z))
                / 3.0;
        let q = super::math::inflate(area, 1.0e-7, 1.0e-7, 1.0e-7);
        let list = if size < 1.0e-7 {
            &[][..]
        } else {
            self.ext.entities()
        };
        list.iter()
            .copied()
            .filter(move |e| intersects(e, q.min_x, q.min_y, q.min_z, q.max_x, q.max_y, q.max_z))
    }

    /// `collectColliders(entity, [], box)` without the entity shapes (they come first; the
    /// extended [`collide`] keeps them in their own list): the border shape when the entity is
    /// inside and close to it, then every block shape.
    pub(crate) fn colliders(&self, src: [f64; 3], query: &Aabb, ctx: &ShapeCtx, c: &mut Colliders) {
        c.clear();
        if self.border.inside_close(src[0], src[2], query) {
            c.push(Placed::Border);
        }
        self.each_collision(query, Some(ctx), false, |_, _, _, s| {
            c.push(s);
            true
        });
    }

    /// `findSupportingBlock(entity, box)`: the colliding block whose centre is closest to the
    /// entity position (ties: the larger position by y, z, x).
    pub(crate) fn supporting_block(
        &self,
        pos: [f64; 3],
        query: &Aabb,
        ctx: &ShapeCtx,
    ) -> Option<[i32; 3]> {
        let mut best: Option<[i32; 3]> = None;
        let mut best_d = f64::MAX;
        self.each_collision(query, Some(ctx), false, |x, y, z, _| {
            let dx = f64::from(x) + 0.5 - pos[0];
            let dy = f64::from(y) + 0.5 - pos[1];
            let dz = f64::from(z) + 0.5 - pos[2];
            let d = dx * dx + dy * dy + dz * dz;
            if d < best_d || d == best_d && best.is_none_or(|b| cmp_pos(b, [x, y, z]) < 0) {
                best = Some([x, y, z]);
                best_d = d;
            }
            true
        });
        best
    }
}

/// `Vec3i.compareTo`: by y, then z, then x.
fn cmp_pos(a: [i32; 3], b: [i32; 3]) -> i32 {
    if a[1] != b[1] {
        a[1] - b[1]
    } else if a[2] != b[2] {
        a[2] - b[2]
    } else {
        a[0] - b[0]
    }
}

/// The voxel index of a context-dependent shape (scaffolding, powder snow); `None` = empty.
fn context_shape<R: Rules>(p: StatePhys, y: i32, ctx: Option<&ShapeCtx>) -> Option<u16> {
    let eps = f64::from(1.0e-5f32);
    if p.kind() == BlockKind::Scaffolding as u8 {
        // isAbove(shape, pos, default): feet > y + shape.maxY - 1.0E-5F; the empty context answers
        // the default (true) and is not descending.
        let above = |top: f64| ctx.is_none_or(|c| c.feet_y > f64::from(y) + top - eps);
        let descending = ctx.is_some_and(|c| c.descending);
        if above(1.0) && !descending {
            return Some(p.shape());
        }
        if p.aux() & 1 != 0 && above(0.0) {
            return Some(R::SCAFFOLDING_BOTTOM_VOXEL);
        }
        return None;
    }
    if p.kind() == BlockKind::PowderSnow as u8 {
        let c = ctx?;
        if c.fall_distance > 2.5 {
            return Some(R::POWDER_SNOW_FALLING_VOXEL);
        }
        if c.leather_boots && c.feet_y > f64::from(y) + 1.0 - eps && !c.descending {
            return Some(R::FULL_CUBE_VOXEL);
        }
        return None;
    }
    if p.flags() & sf::EMPTY != 0 {
        None
    } else {
        Some(p.shape())
    }
}

/// `BlockBehaviour.OffsetFunction` at `(x, z)` for a block's `(max_h, max_v, xyz)` limits.
pub(crate) fn model_offset(limits: Option<(f32, f32, bool)>, x: i32, z: i32) -> [f64; 3] {
    let Some((max_h, max_v, xyz)) = limits else {
        return [0.0; 3];
    };
    // Mth.getSeed(x, 0, z)
    let mut seed = i64::from(x.wrapping_mul(3_129_871)) ^ i64::from(z).wrapping_mul(116_129_781);
    seed = seed
        .wrapping_mul(seed)
        .wrapping_mul(42_317_861)
        .wrapping_add(seed.wrapping_mul(11));
    let seed = seed >> 16;
    let h = f64::from(max_h);
    let comp = |bits: i64| {
        let v = (f64::from((bits & 15) as f32 / 15.0) - 0.5) * 0.5;
        super::math::clamp(v, -h, h)
    };
    let y = if xyz {
        (f64::from(((seed >> 4) & 15) as f32 / 15.0) - 1.0) * f64::from(max_v)
    } else {
        0.0
    };
    [comp(seed), y, comp(seed >> 8)]
}

/// `Entity.collide(movement)` for the player: block collision, then step-up by the candidate
/// heights taken from the colliders' Y coordinates.
#[allow(clippy::too_many_arguments)]
pub(crate) fn collide<R: Rules, W: BlockView<State = R::State>, E: Extra>(
    world: &World<'_, R, W, E>,
    src: [f64; 3],
    bb: &Aabb,
    movement: V3,
    on_ground: bool,
    step: f32,
    ctx: &ShapeCtx,
) -> V3 {
    if E::ON && !world.ext.entities().is_empty() {
        return collide_among_entities(world, src, bb, movement, on_ground, step, ctx);
    }
    let mut colliders = Colliders::new();
    let step_v = if movement.length_sqr() == 0.0 {
        movement
    } else {
        world.colliders(src, &expand_towards(bb, movement), ctx, &mut colliders);
        collide_with_shapes(movement, bb, &colliders, world.border)
    };
    let cx = movement.x != step_v.x;
    let cy = movement.y != step_v.y;
    let cz = movement.z != step_v.z;
    let landing = cy && movement.y < 0.0;
    if step > 0.0 && (landing || on_ground) && (cx || cz) {
        let grounded = if landing {
            mv(bb, 0.0, step_v.y, 0.0)
        } else {
            *bb
        };
        let mut up = expand_towards(&grounded, V3::new(movement.x, f64::from(step), movement.z));
        if !landing {
            up = expand_towards(&up, V3::new(0.0, f64::from(-1.0e-5f32), 0.0));
        }
        world.colliders(src, &up, ctx, &mut colliders);
        let skip = step_v.y as f32;
        // FloatArraySet: bitwise-distinct values; inline for the usual few, spilling when a tall
        // step height meets finely divided shapes (e.g. stacked daylight detectors: one value per
        // 1/8 block).
        let mut inline = [0.0f32; 64];
        let mut n = 0usize;
        let mut spill: Vec<f32> = Vec::new();
        for c in colliders.iter() {
            c.for_y_coords(world.border, |coord| {
                let rel = (coord - grounded.min_y) as f32;
                if rel < 0.0 || rel == skip {
                    return true;
                }
                if rel > step {
                    return false;
                }
                let seen = |v: &f32| v.to_bits() == rel.to_bits();
                if !inline[..n].iter().any(seen) && !spill.iter().any(seen) {
                    if n < inline.len() {
                        inline[n] = rel;
                        n += 1;
                    } else {
                        spill.push(rel);
                    }
                }
                true
            });
        }
        let mut all = spill;
        let cands: &mut [f32] = if all.is_empty() {
            &mut inline[..n]
        } else {
            all.extend_from_slice(&inline[..n]);
            &mut all
        };
        cands.sort_by(f32::total_cmp);
        for &h in cands.iter() {
            let s = collide_with_shapes(
                V3::new(movement.x, f64::from(h), movement.z),
                &grounded,
                &colliders,
                world.border,
            );
            if s.horizontal_sqr() > step_v.horizontal_sqr() {
                let d = bb.min_y - grounded.min_y;
                return V3::new(s.x, s.y - d, s.z);
            }
        }
    }
    step_v
}

/// [`collide`] with collidable entities around: their shapes come before the border and block
/// shapes (`collectColliders`), and the step-up reuses the ones found for the movement (vanilla
/// passes that list on). Thin entity boxes are empty shapes and drop out. Out of line so the
/// plain [`collide`] stays as it was.
#[inline(never)]
fn collide_among_entities<R: Rules, W: BlockView<State = R::State>, E: Extra>(
    world: &World<'_, R, W, E>,
    src: [f64; 3],
    bb: &Aabb,
    movement: V3,
    on_ground: bool,
    step: f32,
    ctx: &ShapeCtx,
) -> V3 {
    let area = expand_towards(bb, movement);
    let ents: Vec<Aabb> = world
        .entity_colliders(&area)
        .filter(|e| !entity_shape_empty(e))
        .collect();
    let mut colliders = Colliders::new();
    let step_v = if movement.length_sqr() == 0.0 {
        movement
    } else {
        world.colliders(src, &area, ctx, &mut colliders);
        collide_with_entities_and_shapes(movement, bb, &ents, &colliders, world.border)
    };
    let cx = movement.x != step_v.x;
    let cy = movement.y != step_v.y;
    let cz = movement.z != step_v.z;
    let landing = cy && movement.y < 0.0;
    if step > 0.0 && (landing || on_ground) && (cx || cz) {
        let grounded = if landing {
            mv(bb, 0.0, step_v.y, 0.0)
        } else {
            *bb
        };
        let mut up = expand_towards(&grounded, V3::new(movement.x, f64::from(step), movement.z));
        if !landing {
            up = expand_towards(&up, V3::new(0.0, f64::from(-1.0e-5f32), 0.0));
        }
        world.colliders(src, &up, ctx, &mut colliders);
        let skip = step_v.y as f32;
        // FloatArraySet of the candidate heights (see `collide`).
        let mut cands: Vec<f32> = Vec::new();
        let mut add = |coord: f64| {
            let rel = (coord - grounded.min_y) as f32;
            if rel < 0.0 || rel == skip {
                return true;
            }
            if rel > step {
                return false;
            }
            if !cands.iter().any(|v| v.to_bits() == rel.to_bits()) {
                cands.push(rel);
            }
            true
        };
        for e in &ents {
            let _ = add(e.min_y) && add(e.max_y);
        }
        for c in colliders.iter() {
            c.for_y_coords(world.border, &mut add);
        }
        cands.sort_by(f32::total_cmp);
        for &h in &cands {
            let s = collide_with_entities_and_shapes(
                V3::new(movement.x, f64::from(h), movement.z),
                &grounded,
                &ents,
                &colliders,
                world.border,
            );
            if s.horizontal_sqr() > step_v.horizontal_sqr() {
                let d = bb.min_y - grounded.min_y;
                return V3::new(s.x, s.y - d, s.z);
            }
        }
    }
    step_v
}

/// [`collide_with_shapes`] over the entity shapes, then the other shapes (one list in vanilla).
fn collide_with_entities_and_shapes(
    m: V3,
    b: &Aabb,
    ents: &[Aabb],
    shapes: &Colliders,
    border: &Border,
) -> V3 {
    if ents.is_empty() {
        return collide_with_shapes(m, b, shapes, border);
    }
    let mut r = V3::ZERO;
    let order: [usize; 3] = if m.x.abs() < m.z.abs() {
        [1, 2, 0]
    } else {
        [1, 0, 2]
    };
    for axis in order {
        let am = m.get(axis);
        if am != 0.0 {
            let moved = mv(b, r.x, r.y, r.z);
            // Shapes.collide over the one list (written out: a second call of `shapes_collide`
            // would take it out of line in the plain path).
            let mut d = am;
            for e in ents {
                if d.abs() < EPSILON {
                    d = 0.0;
                    break;
                }
                d = entity_shape_collide(e, axis, &moved, d);
            }
            for s in shapes.iter() {
                if d.abs() < EPSILON {
                    d = 0.0;
                    break;
                }
                d = s.collide(border, axis, &moved, d);
            }
            r = r.with(axis, d);
        }
    }
    r
}

/// A box of a pose at a position (`EntityDimensions.makeBoundingBox`).
#[inline]
pub(crate) fn make_box(width: f32, height: f32, x: f64, y: f64, z: f64) -> Aabb {
    let w = f64::from(width / 2.0);
    bx(x - w, y, z - w, x + w, y + f64::from(height), z + w)
}
