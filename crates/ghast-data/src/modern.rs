//! Types shared by the modern versions (26.1, 26.2): block kinds and flags, collision voxel
//! shapes and the packed per-state movement record. `v26_1` and `v26_2` re-export them, so
//! code written for one works with the other.

use crate::common::Aabb;

/// Blocks the movement code treats specially.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BlockKind {
    /// Nothing special.
    Other = 0,
    /// `cobweb` (stuck multiplier 0.25 / 0.05).
    Cobweb,
    /// `sweet_berry_bush` (stuck multiplier 0.8 / 0.75).
    SweetBerryBush,
    /// `powder_snow` (entity-dependent collision, stuck multiplier, climbing with leather boots).
    PowderSnow,
    /// `honey_block` (wall slide).
    HoneyBlock,
    /// `bubble_column` (`aux` bit 0 = drag down).
    BubbleColumn,
    /// `slime_block` (bounce, step slowdown).
    SlimeBlock,
    /// `scaffolding` (`aux` bit 0 = has the bottom shape).
    Scaffolding,
    /// `ladder` (`aux` = facing: 0 N, 1 E, 2 S, 3 W).
    Ladder,
    /// `water`.
    Water,
    /// `lava`.
    Lava,
    /// `moving_piston`.
    MovingPiston,
}

/// Bits of `BlockData::flags` (vanilla tags and `instanceof` tests of the movement code).
pub mod block_flags {
    /// `#minecraft:climbable`.
    pub const CLIMBABLE: u32 = 1;
    /// `#minecraft:fences`.
    pub const FENCES: u32 = 2;
    /// `#minecraft:walls`.
    pub const WALLS: u32 = 4;
    /// `instanceof FenceGateBlock`.
    pub const FENCE_GATE: u32 = 8;
    /// `instanceof TrapDoorBlock` (the state's `aux`: bit 0 open, bits 1-2 facing N/E/S/W).
    pub const TRAPDOOR: u32 = 16;
    /// `instanceof IceBlock` (ice, frosted ice).
    pub const ICE: u32 = 32;
    /// `instanceof BedBlock`.
    pub const BED: u32 = 64;
    /// `#minecraft:fall_damage_resetting`.
    pub const FALL_DAMAGE_RESETTING: u32 = 128;
    /// `#minecraft:soul_speed_blocks`.
    pub const SOUL_SPEED_BLOCKS: u32 = 256;
    /// `#minecraft:fire`.
    pub const FIRE: u32 = 512;
    /// `#minecraft:can_glide_through` (vines: not climbable for a gliding player).
    pub const CAN_GLIDE_THROUGH: u32 = 1024;
}

/// VoxelShape implementation class (decides how a coordinate maps to a cell index in vanilla).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VoxelClass {
    /// `CubeVoxelShape`: points are `i / n`; vanilla indexes with `floor(c * n)`, clamped.
    Simple,
    /// `ArrayVoxelShape`: explicit point lists; vanilla binary-searches them.
    Array,
}

/// A vanilla collision `VoxelShape` (block-local), exactly as the jar has it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelShapeRef {
    /// Implementation class.
    pub class: VoxelClass,
    /// Cell counts `[nx, ny, nz]` (`0, 0, 0` for the empty shape).
    pub sizes: [u8; 3],
    /// `getCoords(X)` (`nx + 1` values; the empty shape has `[0.0]`).
    pub xs: &'static [f64],
    /// `getCoords(Y)`.
    pub ys: &'static [f64],
    /// `getCoords(Z)`.
    pub zs: &'static [f64],
    /// Filled-cell bitset, bit index `(x * ny + y) * nz + z`, 64 bits per word.
    pub cells: &'static [u64],
    /// `VoxelShape.isEmpty()`.
    pub empty: bool,
    /// `toAabbs()` in vanilla's order.
    pub boxes: &'static [Aabb],
}

impl VoxelShapeRef {
    /// `true` when cell (`x`, `y`, `z`) is filled; `false` outside the grid.
    #[must_use]
    pub fn filled(&self, x: usize, y: usize, z: usize) -> bool {
        let [nx, ny, nz] = self.sizes.map(usize::from);
        if x >= nx || y >= ny || z >= nz {
            return false;
        }
        let i = (x * ny + y) * nz + z;
        self.cells[i / 64] & (1 << (i % 64)) != 0
    }
}

/// The packed per-state record of the movement code (one `u64` per state).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StatePhys(pub u64);

/// Bits of [`StatePhys::flags`].
pub mod state_flags {
    /// The collision shape is `Shapes.block()` (identity; vanilla's fast path).
    pub const FULL_CUBE: u16 = 1;
    /// The context-free collision shape is empty.
    pub const EMPTY: u16 = 2;
    /// `hasLargeCollisionShape()` (shape exceeds the unit cube, or dynamic bounds).
    pub const LARGE: u16 = 4;
    /// `isSuffocating()`.
    pub const SUFFOCATING: u16 = 8;
    /// `isAir()`.
    pub const AIR: u16 = 16;
    /// `blocksMotion()`.
    pub const BLOCKS_MOTION: u16 = 32;
    /// `isFaceSturdy(NORTH)`; the next three bits are EAST, SOUTH, WEST.
    pub const STURDY_NORTH: u16 = 64;
    /// The collision shape depends on the colliding entity (scaffolding, powder snow).
    pub const CONTEXT: u16 = 1 << 10;
    /// The collision shape is shifted by the block's model offset (bamboo, pointed dripstone);
    /// [`super::StatePhys::shape`] is the unshifted shape.
    pub const OFFSET: u16 = 1 << 11;
    /// `moving_piston`.
    pub const MOVING_PISTON: u16 = 1 << 12;
}

impl StatePhys {
    /// Collision shape index into `voxel_shapes()` of the version (context-free; unshifted for offset blocks).
    #[inline]
    #[must_use]
    pub const fn shape(self) -> u16 {
        self.0 as u16
    }

    /// [`state_flags`] bits.
    #[inline]
    #[must_use]
    pub const fn flags(self) -> u16 {
        (self.0 >> 16) as u16
    }

    /// `true` when all of `bits` are set.
    #[inline]
    #[must_use]
    pub const fn has(self, bits: u16) -> bool {
        self.flags() & bits == bits
    }

    /// Fluid byte: bits 6-7 kind (0 none, 1 water, 2 lava), bit 4 falling, bits 0-3 amount 1..=8.
    #[inline]
    #[must_use]
    pub const fn fluid(self) -> u8 {
        (self.0 >> 32) as u8
    }

    /// Per-kind extra bits (see [`BlockKind`] and [`block_flags::TRAPDOOR`]).
    #[inline]
    #[must_use]
    pub const fn aux(self) -> u8 {
        (self.0 >> 40) as u8
    }

    /// The block's [`BlockKind`] discriminant.
    #[inline]
    #[must_use]
    pub const fn kind(self) -> u8 {
        (self.0 >> 48) as u8
    }
}
