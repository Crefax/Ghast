//! 1.16.5 block facts and collision shapes for the movement code, from `ghast-data`'s tables
//! of vanilla's exact voxel grids (generated from the oracle's dump).

use ghast_data::v1_16_5::{Block, BlockState, Material, Property, Side, VoxelClass, VoxelShapeRef};
use ghast_data::{FluidKind, PhysicsFlags};

use crate::math::Aabb;
use crate::voxel::{Points, VoxelShape};
use crate::world::BlockView;

/// A `ghast-data` voxel table entry as a [`VoxelShape`].
fn voxel(r: &'static VoxelShapeRef) -> VoxelShape<'static> {
    let axis = |n: u8, pts: &'static [f64]| match r.class {
        VoxelClass::Simple => Points::Fractional(u32::from(n)),
        VoxelClass::Array => Points::Explicit(pts),
    };
    VoxelShape {
        axes: [
            axis(r.sizes[0], r.xs),
            axis(r.sizes[1], r.ys),
            axis(r.sizes[2], r.zs),
        ],
        cells: r.cells,
    }
}

/// Per-state collision summary for the hot cell loop (one byte per state, built once from the
/// `ghast-data` tables; constant data like the sine table).
pub(crate) mod class {
    /// The collision shape has filled cells (always set for context / position dependent shapes).
    pub const NONEMPTY: u8 = 1;
    /// The shape is vanilla's shared full cube (`BlockCollisionSpliterator` fast path).
    pub const FULL_CUBE: u8 = 2;
    /// `exceedsCube()`.
    pub const EXCEEDS: u8 = 4;
    /// Moving piston (collides even at the cuboid's two-edge cells).
    pub const MOVING_PISTON: u8 = 8;
}

static CLASS: std::sync::LazyLock<Box<[u8]>> = std::sync::LazyLock::new(|| {
    (0..ghast_data::v1_16_5::STATE_COUNT as u16)
        .map(|id| {
            let Some(s) = BlockState::from_id(id) else {
                return 0;
            };
            let special = s.block() == Block::SCAFFOLDING || s.block() == Block::BAMBOO;
            let mut c = 0;
            if special || !s.voxel_shape().empty {
                c |= class::NONEMPTY;
            }
            if !special && s.full_cube_identity() {
                c |= class::FULL_CUBE;
            }
            if s.exceeds_cube() {
                c |= class::EXCEEDS;
            }
            if s.block() == Block::MOVING_PISTON {
                c |= class::MOVING_PISTON;
            }
            c
        })
        .collect()
});

/// The collision summary byte of a state (see [`class`]).
#[inline]
pub(crate) fn collision_class(s: BlockState) -> u8 {
    CLASS.get(s.id() as usize).copied().unwrap_or(0)
}

/// What a block's collision may depend on from the colliding player.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShapeCtx {
    /// The entity's feet Y when the context was made.
    pub min_y: f64,
    /// Sneaking (vanilla `isDescending`).
    pub descending: bool,
}

/// A block's collision shape at a position.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Shape {
    /// No collision.
    Empty,
    /// A grid shape (the full cube is `VoxelShape::FULL_CUBE`).
    Grid {
        shape: VoxelShape<'static>,
        /// `true` for vanilla's shared full-cube instance (collision fast path).
        full_cube: bool,
    },
}

/// Which scaffolding variant applies: 0 = full (stable) shape, 1 = bottom slab, 2 = none.
fn scaffolding_variant(s: BlockState, y: i32, ctx: Option<&ShapeCtx>) -> usize {
    // Vanilla `isAbove(shape, pos, true)`: feet above the shape's top (minus 1e-5); a context-free
    // query counts as above. The bottom test uses a cube shifted one block down (top 0.0).
    let above = |top: f64| ctx.is_none_or(|c| c.min_y > f64::from(y) + top - f64::from(1.0e-5f32));
    let descending = ctx.is_some_and(|c| c.descending);
    if above(1.0) && !descending {
        return 0;
    }
    let bottom = s.get(Property::Bottom) == Some("true");
    let distance0 = s.get(Property::Distance) == Some("0");
    if !distance0 && bottom && above(0.0) {
        1
    } else {
        2
    }
}

/// The collision shape's boxes (vanilla `getBoundingBoxes` order), block-local.
pub(crate) fn collision_boxes(s: BlockState, y: i32, ctx: Option<&ShapeCtx>) -> &'static [Aabb] {
    if let Some(sc) = s.scaffolding_shapes() {
        return match scaffolding_variant(s, y, ctx) {
            0 => sc.stable,
            1 => sc.bottom,
            _ => &[],
        };
    }
    s.shape()
}

/// Bamboo's collision box before its per-position offset (6.5/16 .. 9.5/16, full height; an
/// `ArrayVoxelShape` because 13/32 needs more than vanilla's 3-bit simple-shape resolution).
static BAMBOO_XZ: [f64; 2] = [0.40625, 0.59375];
static BAMBOO_Y: [f64; 2] = [0.0, 1.0];

/// Vanilla's per-position horizontal model offset (`getModelOffset`, offset type XZ).
fn model_offset_xz(x: i32, z: i32) -> (f64, f64) {
    let mut l = i64::from(x.wrapping_mul(3_129_871)) ^ i64::from(z).wrapping_mul(116_129_781);
    l = l
        .wrapping_mul(l)
        .wrapping_mul(42_317_861)
        .wrapping_add(l.wrapping_mul(11));
    let l = l >> 16;
    let f = |bits: i64| (f64::from((bits & 15) as f32 / 15.0) - 0.5) * 0.5;
    (f(l), f(l >> 8))
}

/// Bamboo's collision box at `(x, z)` with its model offset, block-local (the shifted shape's
/// `getBoundingBoxes`: each boundary is `point + offset`).
pub(crate) fn bamboo_box(x: i32, z: i32) -> Aabb {
    let (ox, oz) = model_offset_xz(x, z);
    Aabb::new(
        BAMBOO_XZ[0] + ox,
        BAMBOO_Y[0],
        BAMBOO_XZ[0] + oz,
        BAMBOO_XZ[1] + ox,
        BAMBOO_Y[1],
        BAMBOO_XZ[1] + oz,
    )
}

/// The collision shape of a state at `(x, y, z)` for a player context.
pub(crate) fn collision_shape(
    s: BlockState,
    x: i32,
    y: i32,
    z: i32,
    ctx: Option<&ShapeCtx>,
) -> Shape {
    if s.block() == Block::BAMBOO {
        let (ox, oz) = model_offset_xz(x, z);
        return Shape::Grid {
            shape: VoxelShape {
                axes: [
                    Points::Shifted(&BAMBOO_XZ, ox),
                    Points::Shifted(&BAMBOO_Y, 0.0),
                    Points::Shifted(&BAMBOO_XZ, oz),
                ],
                cells: &[1],
            },
            full_cube: false,
        };
    }
    let r = match s.scaffolding_shapes() {
        Some(sc) => match scaffolding_variant(s, y, ctx) {
            0 => sc.stable_voxel,
            1 => sc.bottom_voxel,
            _ => return Shape::Empty,
        },
        None => s.voxel_shape(),
    };
    if r.empty {
        return Shape::Empty;
    }
    Shape::Grid {
        shape: voxel(r),
        full_cube: s.scaffolding_shapes().is_none() && s.full_cube_identity(),
    }
}

/// `BlockState.exceedsCube()`.
#[inline]
pub(crate) fn exceeds_cube(s: BlockState) -> bool {
    s.exceeds_cube()
}

/// `BlockState.shouldSuffocate`.
#[inline]
pub(crate) fn should_suffocate(s: BlockState) -> bool {
    s.should_suffocate()
}

/// `Material.blocksMovement()`.
#[inline]
pub(crate) fn blocks_movement(s: BlockState) -> bool {
    s.material_blocks_movement()
}

/// `isSideSolidFullSquare` for a horizontal side, given as the unit offset `(dx, dz)`.
#[inline]
pub(crate) fn side_solid(s: BlockState, dx: i32, dz: i32) -> bool {
    let side = match (dx, dz) {
        (0, -1) => Side::North,
        (1, 0) => Side::East,
        (0, 1) => Side::South,
        _ => Side::West,
    };
    s.side_solid_full_square(side)
}

/// Ice material (`ice`, `frosted_ice`).
pub(crate) fn is_ice_material(s: BlockState) -> bool {
    s.block().material() == Material::Ice
}

/// A liquid in a block state: kind, vanilla fluid amount `1..=8`, falling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FluidAt {
    pub kind: FluidKind,
    pub amount: u8,
    pub falling: bool,
}

/// The fluid state of a block state (vanilla `getFluidState`).
#[inline]
pub(crate) fn fluid(s: BlockState) -> Option<FluidAt> {
    s.fluid().map(|f| FluidAt {
        kind: f.kind,
        amount: f.amount(),
        falling: f.falling(),
    })
}

#[inline]
pub(crate) fn flags(s: BlockState) -> PhysicsFlags {
    s.block().physics().flags
}

/// Ground friction.
#[inline]
pub(crate) fn slipperiness(s: BlockState) -> f32 {
    s.block().physics().slipperiness
}

/// Horizontal velocity multiplier (soul sand, honey: 0.4).
#[inline]
pub(crate) fn velocity_multiplier(s: BlockState) -> f32 {
    s.block().physics().velocity_multiplier()
}

/// Jump velocity multiplier (honey: 0.5).
#[inline]
pub(crate) fn jump_multiplier(s: BlockState) -> f32 {
    s.block().physics().jump_factor
}

/// Vanilla's block range: y in `0..256`, |x|, |z| below 30,000,000.
#[inline]
pub(crate) fn is_valid_pos(x: i32, y: i32, z: i32) -> bool {
    (-30_000_000..30_000_000).contains(&x)
        && (-30_000_000..30_000_000).contains(&z)
        && (0..256).contains(&y)
}

/// `World.getBlockState`: void air outside the valid range.
#[inline]
pub(crate) fn state_at<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
) -> BlockState {
    if is_valid_pos(x, y, z) {
        w.block(x, y, z)
    } else {
        Block::VOID_AIR.default_state()
    }
}

/// `true` when the state is a bed of any colour.
#[inline]
pub(crate) fn is_bed(s: BlockState) -> bool {
    (Block::WHITE_BED.id()..=Block::BLACK_BED.id()).contains(&s.block().id())
}

/// `true` for a climbable position: the climbable tag, or an open trapdoor above a ladder that
/// faces the same way.
pub(crate) fn is_climbable_at<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    let s = state_at(w, x, y, z);
    let f = flags(s);
    if f.contains(PhysicsFlags::CLIMBABLE) {
        return true;
    }
    if f.contains(PhysicsFlags::TRAPDOOR) && s.get(Property::Open) == Some("true") {
        let below = state_at(w, x, y - 1, z);
        return below.block() == Block::LADDER
            && below.get(Property::Facing) == s.get(Property::Facing);
    }
    false
}
