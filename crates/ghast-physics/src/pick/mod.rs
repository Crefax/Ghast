//! The crosshair block pick of the voxel-shape versions (1.14 on): which block, face and exact
//! point the player's look ray hits (vanilla `BlockGetter.clip` / `BlockView.raycast` with the
//! outline shape and no fluids, as the client's crosshair target uses it).
//!
//! The ray walks the blocks it crosses in order (a 3-D DDA over the segment shortened by `1e-7`
//! at both ends) and stops at the first block whose outline shape it hits. Per block:
//!
//! * if the ray's first test point (`from + (to - from) * 0.001`) lies inside the outline's grid,
//!   the hit is that point, facing against the ray, flagged `inside`;
//! * otherwise each of the shape's boxes is clipped in the shape's own order and the nearest
//!   entry wins (ties keep the earlier box);
//! * a hit is then re-tested against the block's interaction shape: if that one is hit nearer,
//!   the hit keeps its point and block but takes the interaction hit's face.
//!
//! This module is the version-independent part; [`v1_16_5`] and [`v26_1`] add the eye position,
//! the look vector, the range rules and the per-state shapes. 1.16.5 and 26.1 share every step
//! here bit for bit (checked against both oracles: `tests/golden/{1.16.5,26.1}/raytrace.jsonl.gz`).
//!
//! Entities: [`entity_hit`] is the shared entity part (vanilla `ProjectileUtil.getEntityHitResult`
//! / 1.16.5 `ProjectileUtil.raycast`); the version modules combine it with the block pick as the
//! client does (`pick_target`). The caller lists the pickable entities in the world's iteration
//! order (entity sections in order, then insertion order): it only decides between overlapping
//! boxes that both contain the eye, or an entity after one that contains the eye.
//!
//! Not covered: fluid picks (the crosshair never uses them), items with an attack range component
//! (26.1 spears), multipart entities (ender dragon), and block-entity-dependent outlines (shulker
//! boxes are a full cube here; moving pistons have no outline here).

use ghast_data::{Aabb, SelectionShape};

pub mod v1_16_5;
#[cfg(feature = "v26_1")]
pub mod v26_1;

/// A block face / direction, in vanilla's `Direction` order (its ordinal).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// -Y.
    Down = 0,
    /// +Y.
    Up = 1,
    /// -Z.
    North = 2,
    /// +Z.
    South = 3,
    /// -X.
    West = 4,
    /// +X.
    East = 5,
}

impl Direction {
    /// Every direction in vanilla's order.
    pub const ALL: [Direction; 6] = [
        Direction::Down,
        Direction::Up,
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ];

    /// The unit normal.
    #[must_use]
    pub const fn normal(self) -> [i32; 3] {
        match self {
            Direction::Down => [0, -1, 0],
            Direction::Up => [0, 1, 0],
            Direction::North => [0, 0, -1],
            Direction::South => [0, 0, 1],
            Direction::West => [-1, 0, 0],
            Direction::East => [1, 0, 0],
        }
    }

    /// The opposite direction.
    #[must_use]
    pub const fn opposite(self) -> Direction {
        match self {
            Direction::Down => Direction::Up,
            Direction::Up => Direction::Down,
            Direction::North => Direction::South,
            Direction::South => Direction::North,
            Direction::West => Direction::East,
            Direction::East => Direction::West,
        }
    }

    /// Vanilla's lower-case name (`"down"`, `"up"`, ...).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Direction::Down => "down",
            Direction::Up => "up",
            Direction::North => "north",
            Direction::South => "south",
            Direction::West => "west",
            Direction::East => "east",
        }
    }

    /// The direction whose normal has the largest dot product with the vector, computed in `f32`
    /// (vanilla `Direction.getApproximateNearest` / 1.16.5 `Direction.getFacing`; `North` when no
    /// dot product is positive).
    #[must_use]
    pub fn approximate_nearest(dx: f64, dy: f64, dz: f64) -> Direction {
        let (x, y, z) = (dx as f32, dy as f32, dz as f32);
        let mut result = Direction::North;
        let mut highest = f32::from_bits(1); // Float.MIN_VALUE
        for d in Direction::ALL {
            let [nx, ny, nz] = d.normal();
            let dot = x * nx as f32 + y * ny as f32 + z * nz as f32;
            if dot > highest {
                highest = dot;
                result = d;
            }
        }
        result
    }
}

/// An entity the crosshair can pick (spectators and entities that are not pickable left out).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickEntity {
    /// The entity's bounding box (world coordinates).
    pub bb: Aabb,
    /// Its pick radius (`getPickRadius` / 1.16.5 `getTargetingMargin`; 0 for almost every entity).
    pub pick_radius: f32,
    /// It has the same root vehicle as the player (the player's own vehicle and its passengers).
    pub same_root_vehicle: bool,
}

/// An entity pick: the index into the caller's entity list and the point on its (inflated) box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityHit {
    /// Index of the entity in the slice passed to the pick.
    pub index: usize,
    /// The hit point (the eye itself when the eye is inside the box).
    pub location: [f64; 3],
}

/// The crosshair target: a block (or a miss), or an entity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// A block hit or a miss.
    Block(BlockHit),
    /// An entity.
    Entity(EntityHit),
}

/// Whether a pick hit a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HitKind {
    /// A block was hit.
    Block,
    /// Nothing within range (vanilla `HitResult.Type.MISS`): `location` is the ray's end (or the
    /// out-of-range hit point), `pos` the block containing it, `face` the direction facing back
    /// along the ray.
    Miss,
}

/// A block pick result (vanilla `BlockHitResult`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockHit {
    /// Hit or miss.
    pub kind: HitKind,
    /// The block.
    pub pos: [i32; 3],
    /// The face hit (the side the server is told about when using / placing against the block).
    pub face: Direction,
    /// The exact hit point (world coordinates).
    pub location: [f64; 3],
    /// The ray started inside the block's shape.
    pub inside: bool,
}

impl BlockHit {
    /// `true` for a block hit.
    #[must_use]
    pub fn is_block(&self) -> bool {
        self.kind == HitKind::Block
    }

    /// A miss at `location` (vanilla `BlockHitResult.miss`).
    #[must_use]
    pub fn miss(location: [f64; 3], face: Direction) -> BlockHit {
        BlockHit {
            kind: HitKind::Miss,
            pos: location.map(floor),
            face,
            location,
            inside: false,
        }
    }
}

/// A shape placed at a block for the ray: the block-local shape plus, for offset outlines, the
/// block's model offset at that position (vanilla `shape.move(offset)`).
#[derive(Clone, Copy, Debug)]
pub struct PlacedShape {
    /// The unshifted shape.
    pub shape: &'static SelectionShape,
    /// The model offset, when the shape is moved by one (it then indexes like an array shape).
    pub shift: Option<[f64; 3]>,
}

impl PlacedShape {
    /// A shape that is not moved.
    #[must_use]
    pub const fn fixed(shape: &'static SelectionShape) -> PlacedShape {
        PlacedShape { shape, shift: None }
    }
}

/// `Mth.floor` (Java `(int) Math.floor`, saturating; NaN gives 0 in both).
#[inline]
fn floor(v: f64) -> i32 {
    v.floor() as i32
}

/// `Mth.frac`.
#[inline]
fn frac(v: f64) -> f64 {
    v - (v.floor() as i64) as f64
}

/// `Mth.sign(double)`.
#[inline]
fn sign(v: f64) -> i32 {
    if v == 0.0 {
        0
    } else if v > 0.0 {
        1
    } else {
        -1
    }
}

/// The cell of `coord` along `axis` (vanilla `VoxelShape.findIndex`; `-1..=n`).
fn find_index(p: &PlacedShape, axis: usize, coord: f64) -> i32 {
    let s = p.shape;
    let n = i32::from(s.sizes[axis]);
    match p.shift {
        None if s.cube => {
            // floor(clamp(coord * n, -1, n)) in 26.1, clamp(floor(coord * n), -1, n) in 1.16.5:
            // the same integer for every input.
            let v = coord * f64::from(n);
            let v = if v < -1.0 {
                -1.0
            } else if v > f64::from(n) {
                f64::from(n)
            } else {
                v
            };
            floor(v)
        }
        shift => {
            let pts = match axis {
                0 => s.xs,
                1 => s.ys,
                _ => s.zs,
            };
            let k = shift.map_or(0.0, |d| d[axis]);
            // Mth.binarySearch(0, n + 1, i -> coord < point(i)) - 1
            let (mut from, mut len) = (0usize, pts.len());
            while len > 0 {
                let half = len / 2;
                let middle = from + half;
                let point = if shift.is_some() {
                    pts[middle] + k
                } else {
                    pts[middle]
                };
                if coord < point {
                    len = half;
                } else {
                    from = middle + 1;
                    len -= half + 1;
                }
            }
            from as i32 - 1
        }
    }
}

/// `AABB.clipPoint` for one face plane.
#[allow(clippy::too_many_arguments)]
#[inline]
fn clip_point(
    scale: &mut f64,
    dir: Option<Direction>,
    da: f64,
    db: f64,
    dc: f64,
    point: f64,
    min_b: f64,
    max_b: f64,
    min_c: f64,
    max_c: f64,
    new_dir: Direction,
    from_a: f64,
    from_b: f64,
    from_c: f64,
) -> Option<Direction> {
    let s = (point - from_a) / da;
    let pb = from_b + s * db;
    let pc = from_c + s * dc;
    if 0.0 < s
        && s < *scale
        && min_b - 1.0e-7 < pb
        && pb < max_b + 1.0e-7
        && min_c - 1.0e-7 < pc
        && pc < max_c + 1.0e-7
    {
        *scale = s;
        Some(new_dir)
    } else {
        dir
    }
}

/// `AABB.getDirection` for one (world) box.
#[inline]
fn box_direction(
    b: [f64; 6],
    from: [f64; 3],
    scale: &mut f64,
    mut dir: Option<Direction>,
    d: [f64; 3],
) -> Option<Direction> {
    let [min_x, min_y, min_z, max_x, max_y, max_z] = b;
    let [dx, dy, dz] = d;
    let [fx, fy, fz] = from;
    if dx > 1.0e-7 {
        dir = clip_point(
            scale,
            dir,
            dx,
            dy,
            dz,
            min_x,
            min_y,
            max_y,
            min_z,
            max_z,
            Direction::West,
            fx,
            fy,
            fz,
        );
    } else if dx < -1.0e-7 {
        dir = clip_point(
            scale,
            dir,
            dx,
            dy,
            dz,
            max_x,
            min_y,
            max_y,
            min_z,
            max_z,
            Direction::East,
            fx,
            fy,
            fz,
        );
    }
    if dy > 1.0e-7 {
        dir = clip_point(
            scale,
            dir,
            dy,
            dz,
            dx,
            min_y,
            min_z,
            max_z,
            min_x,
            max_x,
            Direction::Down,
            fy,
            fz,
            fx,
        );
    } else if dy < -1.0e-7 {
        dir = clip_point(
            scale,
            dir,
            dy,
            dz,
            dx,
            max_y,
            min_z,
            max_z,
            min_x,
            max_x,
            Direction::Up,
            fy,
            fz,
            fx,
        );
    }
    if dz > 1.0e-7 {
        dir = clip_point(
            scale,
            dir,
            dz,
            dx,
            dy,
            min_z,
            min_x,
            max_x,
            min_y,
            max_y,
            Direction::North,
            fz,
            fx,
            fy,
        );
    } else if dz < -1.0e-7 {
        dir = clip_point(
            scale,
            dir,
            dz,
            dx,
            dy,
            max_z,
            min_x,
            max_x,
            min_y,
            max_y,
            Direction::South,
            fz,
            fx,
            fy,
        );
    }
    dir
}

/// `AABB.clip(from, to)` for one box.
#[must_use]
pub fn clip_box(b: &Aabb, from: [f64; 3], to: [f64; 3]) -> Option<[f64; 3]> {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let mut scale = 1.0;
    let c = [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z];
    box_direction(c, from, &mut scale, None, d)?;
    Some([
        from[0] + scale * d[0],
        from[1] + scale * d[1],
        from[2] + scale * d[2],
    ])
}

/// `AABB.intersects(AABB)`.
#[inline]
fn intersects(a: &Aabb, b: &Aabb) -> bool {
    a.min_x < b.max_x
        && a.max_x > b.min_x
        && a.min_y < b.max_y
        && a.max_y > b.min_y
        && a.min_z < b.max_z
        && a.max_z > b.min_z
}

/// `AABB.contains(Vec3)`.
#[inline]
fn contains(b: &Aabb, p: [f64; 3]) -> bool {
    p[0] >= b.min_x
        && p[0] < b.max_x
        && p[1] >= b.min_y
        && p[1] < b.max_y
        && p[2] >= b.min_z
        && p[2] < b.max_z
}

/// The entity search box: the player's box stretched along `delta`, grown by 1 (vanilla
/// `getBoundingBox().expandTowards(delta).inflate(1.0)`).
#[must_use]
pub fn search_box(player_bb: &Aabb, delta: [f64; 3]) -> Aabb {
    let mut lo = [player_bb.min_x, player_bb.min_y, player_bb.min_z];
    let mut hi = [player_bb.max_x, player_bb.max_y, player_bb.max_z];
    for i in 0..3 {
        if delta[i] < 0.0 {
            lo[i] += delta[i];
        } else if delta[i] > 0.0 {
            hi[i] += delta[i];
        }
    }
    Aabb::new(
        lo[0] - 1.0,
        lo[1] - 1.0,
        lo[2] - 1.0,
        hi[0] + 1.0,
        hi[1] + 1.0,
        hi[2] + 1.0,
    )
}

/// The entity the segment `from -> to` picks among the entities whose box intersects `search`
/// (vanilla `ProjectileUtil.getEntityHitResult`): the nearest box hit within `max_dist_sq`; a box
/// containing `from` counts as distance 0.
#[must_use]
pub fn entity_hit(
    from: [f64; 3],
    to: [f64; 3],
    search: &Aabb,
    entities: &[PickEntity],
    max_dist_sq: f64,
) -> Option<EntityHit> {
    let mut nearest = max_dist_sq;
    let mut hovered = None;
    for (index, e) in entities.iter().enumerate() {
        if !intersects(&e.bb, search) {
            continue;
        }
        let r = f64::from(e.pick_radius);
        let bb = Aabb::new(
            e.bb.min_x - r,
            e.bb.min_y - r,
            e.bb.min_z - r,
            e.bb.max_x + r,
            e.bb.max_y + r,
            e.bb.max_z + r,
        );
        let clip = clip_box(&bb, from, to);
        if contains(&bb, from) {
            if nearest >= 0.0 {
                hovered = Some(EntityHit {
                    index,
                    location: clip.unwrap_or(from),
                });
                nearest = 0.0;
            }
        } else if let Some(location) = clip {
            let dd = length_sqr(location, from);
            if dd < nearest || nearest == 0.0 {
                if e.same_root_vehicle {
                    if nearest == 0.0 {
                        hovered = Some(EntityHit { index, location });
                    }
                } else {
                    hovered = Some(EntityHit { index, location });
                    nearest = dd;
                }
            }
        }
    }
    hovered
}

/// One shape against the segment `from -> to` at block `pos` (vanilla `VoxelShape.clip`).
#[must_use]
pub fn clip_shape(
    p: &PlacedShape,
    from: [f64; 3],
    to: [f64; 3],
    pos: [i32; 3],
) -> Option<BlockHit> {
    let s = p.shape;
    if s.empty {
        return None;
    }
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 1.0e-7 {
        return None;
    }
    let test = [
        from[0] + d[0] * 0.001,
        from[1] + d[1] * 0.001,
        from[2] + d[2] * 0.001,
    ];
    let pf = pos.map(f64::from);
    let ix = find_index(p, 0, test[0] - pf[0]);
    let iy = find_index(p, 1, test[1] - pf[1]);
    let iz = find_index(p, 2, test[2] - pf[2]);
    if s.filled(ix, iy, iz) {
        return Some(BlockHit {
            kind: HitKind::Block,
            pos,
            face: Direction::approximate_nearest(d[0], d[1], d[2]).opposite(),
            location: test,
            inside: true,
        });
    }
    // AABB.clip(toAabbs(), from, to, pos)
    let mut scale = 1.0;
    let mut dir = None;
    let k = p.shift.unwrap_or([0.0; 3]);
    let shifted = p.shift.is_some();
    for b in s.boxes {
        let mut c = [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z];
        if shifted {
            for (i, v) in c.iter_mut().enumerate() {
                *v += k[i % 3];
            }
        }
        for (i, v) in c.iter_mut().enumerate() {
            *v += pf[i % 3];
        }
        dir = box_direction(c, from, &mut scale, dir, d);
    }
    dir.map(|face| BlockHit {
        kind: HitKind::Block,
        pos,
        face,
        location: [
            from[0] + scale * d[0],
            from[1] + scale * d[1],
            from[2] + scale * d[2],
        ],
        inside: false,
    })
}

#[inline]
pub(crate) fn length_sqr(a: [f64; 3], from: [f64; 3]) -> f64 {
    let (x, y, z) = (a[0] - from[0], a[1] - from[1], a[2] - from[2]);
    x * x + y * y + z * z
}

/// The outline hit, re-faced by a nearer interaction-shape hit (vanilla
/// `BlockGetter.clipWithInteractionOverride`).
#[must_use]
pub fn clip_block(
    outline: &PlacedShape,
    interaction: &PlacedShape,
    from: [f64; 3],
    to: [f64; 3],
    pos: [i32; 3],
) -> Option<BlockHit> {
    let mut hit = clip_shape(outline, from, to, pos)?;
    if let Some(o) = clip_shape(interaction, from, to, pos)
        && length_sqr(o.location, from) < length_sqr(hit.location, from)
    {
        hit.face = o.face;
    }
    Some(hit)
}

/// Walks the blocks the segment crosses, in order, until `visit` returns a result (vanilla
/// `BlockGetter.traverseBlocks`); `None` when the walk ends without one.
pub fn traverse<T>(
    from: [f64; 3],
    to: [f64; 3],
    mut visit: impl FnMut([i32; 3]) -> Option<T>,
) -> Option<T> {
    // Vec3.equals: Double.compare per component (-0.0 differs from 0.0, NaN equals NaN).
    let same = |a: f64, b: f64| a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan());
    if same(from[0], to[0]) && same(from[1], to[1]) && same(from[2], to[2]) {
        return None;
    }
    let lerp = |a: f64, b: f64| a + -1.0e-7 * (b - a);
    let end = [
        lerp(to[0], from[0]),
        lerp(to[1], from[1]),
        lerp(to[2], from[2]),
    ];
    let start = [
        lerp(from[0], to[0]),
        lerp(from[1], to[1]),
        lerp(from[2], to[2]),
    ];
    let mut cur = start.map(floor);
    if let Some(r) = visit(cur) {
        return Some(r);
    }
    let d = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
    let sg = d.map(sign);
    let mut delta = [0.0; 3];
    let mut t = [0.0; 3];
    for i in 0..3 {
        delta[i] = if sg[i] == 0 {
            f64::MAX
        } else {
            f64::from(sg[i]) / d[i]
        };
        t[i] = delta[i]
            * if sg[i] > 0 {
                1.0 - frac(start[i])
            } else {
                frac(start[i])
            };
    }
    while t[0] <= 1.0 || t[1] <= 1.0 || t[2] <= 1.0 {
        let axis = if t[0] < t[1] {
            if t[0] < t[2] { 0 } else { 2 }
        } else if t[1] < t[2] {
            1
        } else {
            2
        };
        cur[axis] += sg[axis];
        t[axis] += delta[axis];
        if let Some(r) = visit(cur) {
            return Some(r);
        }
    }
    None
}

/// The block pick along `from -> to` with the shapes `shapes(pos)` gives at each block crossed
/// (outline, interaction; `None` skips the block), no fluids: the first block hit, else a miss at
/// `to`.
pub fn clip(
    from: [f64; 3],
    to: [f64; 3],
    mut shapes: impl FnMut([i32; 3]) -> Option<(PlacedShape, PlacedShape)>,
) -> BlockHit {
    traverse(from, to, |pos| {
        let (outline, interaction) = shapes(pos)?;
        clip_block(&outline, &interaction, from, to, pos)
    })
    .unwrap_or_else(|| {
        BlockHit::miss(
            to,
            Direction::approximate_nearest(from[0] - to[0], from[1] - to[1], from[2] - to[2]),
        )
    })
}
