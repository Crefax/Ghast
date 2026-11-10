//! Minecraft 1.12.x (protocol 335 – 340) data: numeric block ids with 4-bit metadata.
//!
//! As in 1.8, a block state travels on the wire and in chunk palettes as one `u16`:
//! `block_id << 4 | meta` ([`BlockState::raw`]). Block ids are `0..=255` (253 and 254 are unused).
//! Every table here comes from the vanilla 1.12.2 oracle (`tests/golden/1.12.2/`): collision boxes
//! per (id, meta) with air neighbours, the per-state shape flags, block materials, ray-test bounds
//! and the 1.12 "face shapes" fences, walls and panes connect by.
//!
//! Unlike 1.8 there are no history-dependent shapes: 1.12 builds every box from constants and the
//! block's actual state. Blocks whose actual state depends on neighbours (fences, panes, walls,
//! stairs, doors, chests, chorus plants) report a [`ShapeDependency::Neighbours`]; their table
//! shape is the one vanilla has with only air around.
//!
//! Unregistered combinations (an unknown id, or a meta the block never produces) read as air, as
//! vanilla's global palette maps them: [`BlockState::from_raw_vanilla`].

use crate::common::{
    Aabb, BiomeInfo, BlockInfo, BlockPhysics, Collision, DigMaterial, EffectInfo, EnchantmentInfo,
    EntityInfo, EntityKind, Fluid, FluidKind, FoodInfo, ItemInfo, NeighbourShape, PhysicsFlags,
    ShapeDependency, lookup_id, lookup_name,
};
use crate::generated::v1_12 as g;

pub mod dig;

/// Size of the block id space (`0..=255`; not every id is a block, see [`Block::from_id`]).
pub const BLOCK_ID_SPACE: usize = 256;

/// Identity of the vanilla materials that 1.12 movement code compares with `==`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialKind {
    /// `Material.AIR`.
    Air,
    /// `Material.WATER`.
    Water,
    /// `Material.LAVA`.
    Lava,
    /// `Material.ICE` (only `ice`; packed ice and frosted ice have their own).
    Ice,
    /// Any other material.
    Other,
}

/// A block's vanilla 1.12.2 material (flags from the oracle).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Material {
    /// Our name for the material (from the Yarn field name, e.g. `"stone"`, `"wood"`).
    pub name: &'static str,
    /// Identity, for `== Material.X` checks.
    pub kind: MaterialKind,
    /// `Material.blocksMovement()`.
    pub blocks_movement: bool,
    /// `Material.isSolid()`.
    pub solid: bool,
    /// `Material.isLiquid()`.
    pub liquid: bool,
    /// `Material.isOpaque()`.
    pub opaque: bool,
    /// `Material.isReplaceable()`.
    pub replaceable: bool,
}

/// Per-state shape flags (1.12 moved these from the block to the state).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct StateFlags(pub u8);

impl StateFlags {
    /// `BlockState.isFullCube()` (Legacy Yarn `method_11730`).
    pub const FULL_CUBE: StateFlags = StateFlags(1 << 0);
    /// `BlockState.isNormalCube()` (`method_11734`): opaque material, full cube, no redstone power.
    /// `ClientPlayerEntity.pushOutOfBlocks` tests it.
    pub const NORMAL_CUBE: StateFlags = StateFlags(1 << 1);
    /// `BlockState.isBlockNormalCube()` (`method_11733`): material blocks movement and full cube.
    pub const BLOCK_NORMAL_CUBE: StateFlags = StateFlags(1 << 2);
    /// `BlockState.isTopSolid()` (`method_11739`).
    pub const TOP_SOLID: StateFlags = StateFlags(1 << 3);
    /// `BlockState.isOpaqueCube()` (`isFullBoundsCubeForCulling`).
    pub const OPAQUE_CUBE: StateFlags = StateFlags(1 << 4);
    /// The `Block.fullBlock` field.
    pub const FULL_BLOCK: StateFlags = StateFlags(1 << 5);
    /// `BlockState.causesSuffocation()` (`method_13763`).
    pub const CAUSES_SUFFOCATION: StateFlags = StateFlags(1 << 6);

    /// `true` when every flag of `other` is set.
    #[must_use]
    pub const fn contains(self, other: StateFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

/// A block face (`Direction` order: down, up, north, south, west, east).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    /// `-Y`.
    Down,
    /// `+Y`.
    Up,
    /// `-Z`.
    North,
    /// `+Z`.
    South,
    /// `-X`.
    West,
    /// `+X`.
    East,
}

/// What a block face looks like to connecting blocks (vanilla 1.12 `BlockFaceShape`; Legacy Yarn
/// names the enum `BlockRenderLayer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceShape {
    /// A full solid face.
    Solid,
    /// `BOWL` (cauldron, hopper top).
    Bowl,
    /// `CENTER_SMALL`.
    CenterSmall,
    /// `MIDDLE_POLE_THIN` (panes, iron bars).
    MiddlePoleThin,
    /// `CENTER`.
    Center,
    /// `MIDDLE_POLE` (fences, fence gates).
    MiddlePole,
    /// `CENTER_BIG`.
    CenterBig,
    /// `MIDDLE_POLE_THICK` (walls).
    MiddlePoleThick,
    /// `UNDEFINED`.
    Undefined,
}

const FACE_SHAPES: [FaceShape; 9] = [
    FaceShape::Solid,
    FaceShape::Bowl,
    FaceShape::CenterSmall,
    FaceShape::MiddlePoleThin,
    FaceShape::Center,
    FaceShape::MiddlePole,
    FaceShape::CenterBig,
    FaceShape::MiddlePoleThick,
    FaceShape::Undefined,
];

/// What the 1.12.2 crosshair ray test sees of a block state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    /// `getBoundingBox(state, world, pos)` with all neighbours air (block-local).
    pub bounds: Aabb,
    /// Same classes as the collision dependency.
    pub dependency: ShapeDependency,
    /// Stairs: the ray test runs per collision box instead of using `bounds`.
    pub multi_pass_ray: bool,
    /// `Block.canCollideCheck(state, false)`.
    pub can_collide: bool,
    /// `Block.canCollideCheck(state, true)` (ray tests that stop at fluids).
    pub can_collide_liquid: bool,
    /// The state has a collision box.
    pub has_collision_box: bool,
}

/// A 1.12 block type (registry id `0..=255`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Block(pub(crate) u8);

impl Block {
    /// The block with this registry id, if it exists in 1.12.
    #[must_use]
    pub fn from_id(id: u8) -> Option<Block> {
        (g::blocks::BLOCK_SLOT[id as usize] != 255).then_some(Block(id))
    }

    /// Looks a block up by registry name (`"stone"`, no namespace; minecraft-data names).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Block> {
        lookup_name(&g::blocks::BLOCK_NAMES, name).map(Block)
    }

    /// Registry id.
    #[must_use]
    pub const fn id(self) -> u8 {
        self.0
    }

    /// Static information (names, hardness, drops, physics).
    #[must_use]
    pub fn info(self) -> &'static BlockInfo {
        &g::blocks::BLOCKS[g::blocks::BLOCK_SLOT[self.0 as usize] as usize]
    }

    /// Registry name.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.info().name
    }

    /// Physics properties (slipperiness, climbable, ...).
    #[must_use]
    pub fn physics(self) -> &'static BlockPhysics {
        &self.info().physics
    }

    /// Vanilla material.
    #[must_use]
    pub fn material(self) -> Material {
        g::blocks::MATERIALS[g::blocks::BLOCK_MATERIAL[self.0 as usize] as usize]
    }

    /// `Block.isCollidable()`: ray tests skip the block when false.
    #[must_use]
    pub fn is_collidable(self) -> bool {
        g::blocks::BLOCK_BITS[self.0 as usize] & 1 != 0
    }

    /// Fences never connect to this block's solid faces (`FenceBlock`'s exception list: leaves,
    /// glass, shulker boxes, pistons, barrier, pumpkins, melon, ...).
    #[must_use]
    pub fn fence_excluded(self) -> bool {
        g::blocks::BLOCK_BITS[self.0 as usize] & 2 != 0
    }

    /// Walls never connect to this block's solid faces.
    #[must_use]
    pub fn wall_excluded(self) -> bool {
        g::blocks::BLOCK_BITS[self.0 as usize] & 4 != 0
    }

    /// Panes and iron bars never connect to this block's solid faces.
    #[must_use]
    pub fn pane_excluded(self) -> bool {
        g::blocks::BLOCK_BITS[self.0 as usize] & 8 != 0
    }

    /// What the collision shape depends on besides (id, meta).
    #[must_use]
    pub fn shape_dependency(self) -> ShapeDependency {
        g::blocks::BLOCK_SHAPE_DEPENDENCY[self.0 as usize]
    }

    /// The state with this metadata (`meta & 15`).
    #[must_use]
    pub const fn with_meta(self, meta: u8) -> BlockState {
        BlockState(((self.0 as u16) << 4) | (meta & 15) as u16)
    }

    /// The state with metadata 0.
    #[must_use]
    pub const fn default_state(self) -> BlockState {
        self.with_meta(0)
    }

    /// All blocks in id order.
    pub fn all() -> impl Iterator<Item = Block> {
        (0..=255u8).filter_map(Block::from_id)
    }
}

/// A 1.12 block state: `block_id << 4 | meta`, exactly the wire / palette value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockState(u16);

impl BlockState {
    /// Air (raw 0).
    pub const AIR: BlockState = BlockState(0);

    /// Wraps a raw `id << 4 | meta` value; `None` if the id is not a 1.12 block.
    #[must_use]
    pub fn from_raw(raw: u16) -> Option<BlockState> {
        (raw < 4096 && g::blocks::BLOCK_SLOT[(raw >> 4) as usize] != 255).then_some(BlockState(raw))
    }

    /// Vanilla's palette mapping: unknown block ids and unregistered metas become air.
    #[must_use]
    pub fn from_raw_vanilla(raw: u16) -> BlockState {
        match BlockState::from_raw(raw) {
            Some(s) if s.is_registered() => s,
            _ => BlockState::AIR,
        }
    }

    /// `true` when vanilla 1.12.2 has a block state for this (id, meta); otherwise it reads as air.
    #[must_use]
    pub fn is_registered(self) -> bool {
        g::blocks::REGISTERED_METAS[(self.0 >> 4) as usize] & (1 << (self.0 & 15)) != 0
    }

    /// Builds a state from a block and metadata (`meta & 15`).
    #[must_use]
    pub const fn new(block: Block, meta: u8) -> BlockState {
        block.with_meta(meta)
    }

    /// The raw `id << 4 | meta` value.
    #[must_use]
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// The block type.
    #[must_use]
    pub const fn block(self) -> Block {
        Block((self.0 >> 4) as u8)
    }

    /// Metadata `0..=15`.
    #[must_use]
    pub const fn meta(self) -> u8 {
        (self.0 & 15) as u8
    }

    /// `true` for air.
    #[must_use]
    pub const fn is_air(self) -> bool {
        self.0 >> 4 == 0
    }

    /// Collision boxes and what else they depend on (1.12 places them in `double`:
    /// [`Aabb::at`]).
    #[must_use]
    pub fn collision(self) -> Collision {
        Collision {
            boxes: self.shape(),
            dependency: g::blocks::BLOCK_SHAPE_DEPENDENCY[(self.0 >> 4) as usize],
            world_coords: None,
        }
    }

    /// The collision boxes from the table (for dependent shapes: the shape with air neighbours).
    #[must_use]
    pub fn shape(self) -> &'static [Aabb] {
        g::blocks::SHAPES[g::blocks::STATE_SHAPE[self.0 as usize] as usize]
    }

    /// Per-state shape flags.
    #[must_use]
    pub fn flags(self) -> StateFlags {
        StateFlags(g::blocks::STATE_FLAGS[self.0 as usize])
    }

    /// `BlockState.isNormalCube()` (see [`StateFlags::NORMAL_CUBE`]).
    #[must_use]
    pub fn is_normal_cube(self) -> bool {
        self.flags().contains(StateFlags::NORMAL_CUBE)
    }

    /// `BlockState.isFullCube()`.
    #[must_use]
    pub fn is_full_cube(self) -> bool {
        self.flags().contains(StateFlags::FULL_CUBE)
    }

    /// The face shape of `face` with only air around (stairs: the straight shape; their real
    /// face shape follows the actual stairs shape).
    #[must_use]
    pub fn face_shape(self, face: Face) -> FaceShape {
        let v = g::blocks::STATE_FACES[self.0 as usize] >> (4 * face as u32);
        FACE_SHAPES[(v & 15) as usize]
    }

    /// Crosshair ray-test data (oracle `selection_boxes.jsonl.gz`).
    #[must_use]
    pub fn selection(self) -> Selection {
        let f = g::blocks::STATE_SELECTION_FLAGS[self.0 as usize];
        let dependency = g::blocks::BLOCK_SHAPE_DEPENDENCY[(self.0 >> 4) as usize];
        Selection {
            bounds: g::blocks::SELECTION_BOUNDS
                [g::blocks::STATE_SELECTION[self.0 as usize] as usize],
            dependency,
            multi_pass_ray: dependency == ShapeDependency::Neighbours(NeighbourShape::Stairs),
            can_collide: f & 1 != 0,
            can_collide_liquid: f & 2 != 0,
            has_collision_box: f & 4 != 0,
        }
    }

    /// Fluid in this state (water / lava blocks; `level` = meta).
    #[must_use]
    pub fn fluid(self) -> Option<Fluid> {
        let flags = self.block().physics().flags;
        let kind = if flags.contains(PhysicsFlags::WATER) {
            FluidKind::Water
        } else if flags.contains(PhysicsFlags::LAVA) {
            FluidKind::Lava
        } else {
            return None;
        };
        Some(Fluid {
            kind,
            level: self.meta(),
        })
    }
}

/// 1.12.2 slime / magma cube dimensions for metadata size `n` (`0.51000005F * n` square, eye height
/// `0.625F * height`), as in 1.8.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn slime_dimensions(n: i32) -> crate::common::EntityDimensions {
    let s = 0.510_000_05_f32 * n as f32;
    crate::common::EntityDimensions {
        width: s,
        height: s,
        eye_height: 0.625 * s,
    }
}

crate::macros::common_api!();
