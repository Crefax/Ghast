//! Minecraft 1.16.5 (protocol 754) data: global block-state palette with properties.
//!
//! A [`BlockState`] is the global palette id used on the wire and in chunk sections
//! (`0..STATE_COUNT`). Each [`Block`] owns a contiguous id range; the state's properties are a
//! mixed-radix number over that range with the first property most significant (vanilla order).
//! In 1.16 neighbour connections (fences, panes, walls, stairs, doors, chests) are part of the
//! state, so their collision shapes are static per state; the few that are not are flagged with a
//! [`ShapeDependency`].

use crate::common::{
    Aabb, BiomeInfo, BlockInfo, BlockPhysics, Collision, DigMaterial, EffectInfo, EnchantmentInfo,
    EntityInfo, EntityKind, Fluid, FluidKind, FoodInfo, ItemInfo, ShapeDependency, lookup_id,
    lookup_name,
};
use crate::generated::v1_16_5 as g;
use core::fmt;

pub use g::blocks::{Material, Property};

/// Number of blocks (ids `0..BLOCK_COUNT`).
pub const BLOCK_COUNT: usize = g::blocks::BLOCKS.len();
/// Number of block states (ids `0..STATE_COUNT`).
pub const STATE_COUNT: usize = g::blocks::STATE_BLOCK.len();

impl Property {
    /// Looks a property up by name (`"facing"`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Property> {
        lookup_name(&g::blocks::PROPERTY_NAMES, name)
    }
}

/// One property of a block with its possible values, in palette order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyDef {
    /// Which property.
    pub property: Property,
    /// Values in palette order (booleans are `["true", "false"]`, integers `"0"`, `"1"`, ...).
    pub values: &'static [&'static str],
}

/// The state-id layout of one block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateLayout {
    /// First state id.
    pub min_state: u16,
    /// Number of states.
    pub state_count: u16,
    /// Default state id.
    pub default_state: u16,
    /// Indices into the property definition table, most significant first.
    pub(crate) properties: &'static [u8],
}

/// Vanilla model offset type (`AbstractBlock.OffsetType`): bamboo and plants are shifted by a
/// per-position pseudo-random offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OffsetType {
    /// No offset.
    None,
    /// Horizontal offset (bamboo's collision shape follows it).
    Xz,
    /// Horizontal and vertical offset.
    Xyz,
}

/// Vanilla 1.16.5 per-block facts dumped by the oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VanillaBlock {
    /// `AbstractBlock.collidable` (false for plants, cobweb, fluids, ...).
    pub collidable: bool,
    /// Model / shape offset type.
    pub offset: OffsetType,
}

/// Scaffolding's context-dependent collision shapes; pick one with [`ScaffoldingShapes::select`].
///
/// Vanilla (`ScaffoldingBlock.getCollisionShape`), confirmed by the oracle's feet-height sweep:
/// `stable` when the entity's feet `minY > y + 1 - 1.0E-5F` and it is not descending (sneaking);
/// otherwise `bottom` when the state has `bottom=true`, `distance != 0` and `minY > y + 0 - 1.0E-5F`
/// (`OUTLINE_SHAPE` is the full cube offset down by one, max Y 0); otherwise empty.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaffoldingShapes {
    /// The full (stable) shape, also [`BlockState::shape`].
    pub stable: &'static [Aabb],
    /// The bottom "collision" shape.
    pub bottom: &'static [Aabb],
    /// `stable` as a VoxelShape.
    pub stable_voxel: &'static VoxelShapeRef,
    /// `bottom` as a VoxelShape (the empty shape when there is none).
    pub bottom_voxel: &'static VoxelShapeRef,
    /// The state has a bottom shape (`bottom=true`, `distance != 0`).
    pub has_bottom: bool,
}

impl ScaffoldingShapes {
    /// The collision shape vanilla uses for an entity whose box bottom is `feet_min_y`, against
    /// the scaffolding block at height `block_y`. Returns the boxes and the VoxelShape.
    #[must_use]
    pub fn select(
        &self,
        feet_min_y: f64,
        block_y: i32,
        descending: bool,
    ) -> (&'static [Aabb], &'static VoxelShapeRef) {
        let eps = f64::from(1.0e-5_f32);
        if feet_min_y > f64::from(block_y) + 1.0 - eps && !descending {
            (self.stable, self.stable_voxel)
        } else if self.has_bottom && feet_min_y > f64::from(block_y) + 0.0 - eps {
            (self.bottom, self.bottom_voxel)
        } else {
            (&[], &g::blocks::VOXEL_SHAPES[0])
        }
    }
}

/// VoxelShape implementation class (decides how a coordinate maps to a cell index in vanilla).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VoxelClass {
    /// `SimpleVoxelShape`: points are `i / n`; vanilla indexes with `floor(c * n)`, clamped.
    Simple,
    /// `ArrayVoxelShape`: explicit point lists; vanilla binary-searches them.
    Array,
}

/// A vanilla 1.16.5 collision `VoxelShape` (block-local), exactly as the oracle dumped it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelShapeRef {
    /// Implementation class.
    pub class: VoxelClass,
    /// Cell counts `[nx, ny, nz]` (`0, 0, 0` for the empty shape).
    pub sizes: [u8; 3],
    /// `getPointPositions(X)` (`nx + 1` values; the empty shape has `[0.0]`).
    pub xs: &'static [f64],
    /// `getPointPositions(Y)`.
    pub ys: &'static [f64],
    /// `getPointPositions(Z)`.
    pub zs: &'static [f64],
    /// Filled-cell bitset, bit index `(x * ny + y) * nz + z`, 64 bits per word.
    pub cells: &'static [u64],
    /// `VoxelShape.isEmpty()`.
    pub empty: bool,
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

/// Block side for [`BlockState::side_solid_full_square`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// -Z.
    North = 0,
    /// +X.
    East = 1,
    /// +Z.
    South = 2,
    /// -X.
    West = 3,
    /// +Y.
    Up = 4,
    /// -Y.
    Down = 5,
}

/// A 1.16.5 block type (registry id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Block(pub(crate) u16);

impl Block {
    /// The block with this registry id.
    #[must_use]
    pub const fn from_id(id: u16) -> Option<Block> {
        if (id as usize) < BLOCK_COUNT {
            Some(Block(id))
        } else {
            None
        }
    }

    /// Looks a block up by registry name (`"oak_stairs"`, no namespace).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Block> {
        lookup_name(&g::blocks::BLOCK_NAMES, name).map(Block)
    }

    /// Registry id.
    #[must_use]
    pub const fn id(self) -> u16 {
        self.0
    }

    /// Static information (names, hardness, drops, physics).
    #[must_use]
    pub fn info(self) -> &'static BlockInfo {
        &g::blocks::BLOCKS[self.0 as usize]
    }

    /// Registry name.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.info().name
    }

    /// Physics properties.
    #[must_use]
    pub fn physics(self) -> &'static BlockPhysics {
        &g::blocks::BLOCKS[self.0 as usize].physics
    }

    /// What the collision shape depends on besides the state.
    #[must_use]
    pub fn shape_dependency(self) -> ShapeDependency {
        g::blocks::BLOCK_SHAPE_DEPENDENCY[self.0 as usize]
    }

    /// `true` for `air`, `cave_air` and `void_air` (vanilla `BlockState.isAir`).
    #[must_use]
    pub const fn is_air(self) -> bool {
        matches!(self, Block::AIR | Block::CAVE_AIR | Block::VOID_AIR)
    }

    /// `true` for the 16 coloured beds (contiguous ids `WHITE_BED..=BLACK_BED`).
    #[must_use]
    pub const fn is_bed(self) -> bool {
        self.0 >= Block::WHITE_BED.0 && self.0 <= Block::BLACK_BED.0
    }

    /// Vanilla material (Yarn `Material` field), per block as in vanilla.
    #[must_use]
    pub fn material(self) -> Material {
        g::blocks::BLOCK_MATERIAL[self.0 as usize]
    }

    /// Vanilla per-block facts (oracle).
    #[must_use]
    pub fn vanilla(self) -> &'static VanillaBlock {
        &g::blocks::VANILLA_BLOCKS[self.0 as usize]
    }

    /// State-id layout.
    #[must_use]
    pub fn layout(self) -> &'static StateLayout {
        &g::blocks::LAYOUTS[self.0 as usize]
    }

    /// The default state.
    #[must_use]
    pub fn default_state(self) -> BlockState {
        BlockState(self.layout().default_state)
    }

    /// All states of this block in id order.
    pub fn states(self) -> impl ExactSizeIterator<Item = BlockState> {
        let l = self.layout();
        (l.min_state..l.min_state + l.state_count).map(BlockState)
    }

    /// The block's properties, most significant first.
    pub fn properties(self) -> impl ExactSizeIterator<Item = &'static PropertyDef> {
        self.layout()
            .properties
            .iter()
            .map(|&i| &g::blocks::PROPERTY_DEFS[i as usize])
    }

    /// The default state with the given properties changed; `None` if a property or value does not
    /// exist for this block.
    #[must_use]
    pub fn state_with(self, props: &[(Property, &str)]) -> Option<BlockState> {
        props
            .iter()
            .try_fold(self.default_state(), |s, &(p, v)| s.with(p, v))
    }

    /// All blocks in id order.
    pub fn all() -> impl ExactSizeIterator<Item = Block> {
        (0..BLOCK_COUNT as u16).map(Block)
    }
}

/// A 1.16.5 block state (global palette id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockState(u16);

impl BlockState {
    /// `minecraft:air` (id 0).
    pub const AIR: BlockState = BlockState(0);

    /// Wraps a palette id; `None` if out of range.
    #[must_use]
    pub const fn from_id(id: u16) -> Option<BlockState> {
        if (id as usize) < STATE_COUNT {
            Some(BlockState(id))
        } else {
            None
        }
    }

    /// Palette id.
    #[must_use]
    pub const fn id(self) -> u16 {
        self.0
    }

    /// The block type.
    #[must_use]
    pub fn block(self) -> Block {
        Block(g::blocks::STATE_BLOCK[self.0 as usize])
    }

    /// `true` for `air`, `cave_air` and `void_air`.
    #[must_use]
    pub fn is_air(self) -> bool {
        self.block().is_air()
    }

    /// Collision boxes and what else they depend on. Hot path: three table lookups.
    #[must_use]
    pub fn collision(self) -> Collision {
        Collision {
            boxes: self.shape(),
            dependency: g::blocks::BLOCK_SHAPE_DEPENDENCY
                [g::blocks::STATE_BLOCK[self.0 as usize] as usize],
            // 1.16 places shapes with double arithmetic (VoxelShape offset).
            world_coords: None,
        }
    }

    /// `true` when the collision shape is exactly a full cube (oracle `full_cube`).
    #[must_use]
    pub fn full_cube(self) -> bool {
        g::blocks::STATE_VANILLA_FLAGS[self.0 as usize] & 1 != 0
    }

    /// `AbstractBlockState.isSolidBlock` (oracle `solid_block`).
    #[must_use]
    pub fn solid_block(self) -> bool {
        g::blocks::STATE_VANILLA_FLAGS[self.0 as usize] & 2 != 0
    }

    /// `AbstractBlockState.shouldSuffocate` (oracle `suffocates`).
    #[must_use]
    pub fn should_suffocate(self) -> bool {
        g::blocks::STATE_VANILLA_FLAGS[self.0 as usize] & 4 != 0
    }

    /// The collision `VoxelShape` (context-free; see [`BlockState::scaffolding_shapes`]).
    #[must_use]
    pub fn voxel_shape(self) -> &'static VoxelShapeRef {
        &g::blocks::VOXEL_SHAPES[g::blocks::STATE_VOXEL[self.0 as usize] as usize]
    }

    fn voxel_flag(self, bit: u16) -> bool {
        g::blocks::STATE_VOXEL_FLAGS[self.0 as usize] & bit != 0
    }

    /// The collision shape *is* `VoxelShapes.fullCube()` (identity, as vanilla's fast paths test).
    #[must_use]
    pub fn full_cube_identity(self) -> bool {
        self.voxel_flag(1)
    }

    /// The collision shape *is* `VoxelShapes.empty()` (identity).
    #[must_use]
    pub fn empty_identity(self) -> bool {
        self.voxel_flag(1 << 12)
    }

    /// `Material.isLiquid()` of the state's material.
    #[must_use]
    pub fn material_liquid(self) -> bool {
        self.voxel_flag(1 << 13)
    }

    /// `Material.isSolid()` of the state's material.
    #[must_use]
    pub fn material_solid(self) -> bool {
        self.voxel_flag(1 << 14)
    }

    /// `Material.isReplaceable()` of the state's material.
    #[must_use]
    pub fn material_replaceable(self) -> bool {
        self.voxel_flag(1 << 15)
    }

    /// `AbstractBlockState.exceedsCube()`.
    #[must_use]
    pub fn exceeds_cube(self) -> bool {
        self.voxel_flag(2)
    }

    /// `AbstractBlock.dynamicBounds` (no shape cache; shapes recomputed per query).
    #[must_use]
    pub fn has_dynamic_bounds(self) -> bool {
        self.voxel_flag(16)
    }

    /// `Material.blocksMovement()` of the state's material.
    #[must_use]
    pub fn material_blocks_movement(self) -> bool {
        self.voxel_flag(32)
    }

    /// `isSideSolidFullSquare(world, pos, side)` in an otherwise empty world.
    #[must_use]
    pub fn side_solid_full_square(self, side: Side) -> bool {
        self.voxel_flag(64 << side as u16)
    }

    /// Scaffolding context shapes; `None` for other blocks.
    #[must_use]
    pub fn scaffolding_shapes(self) -> Option<ScaffoldingShapes> {
        if self.block() != Block::SCAFFOLDING {
            return None;
        }
        let has_bottom =
            self.get(Property::Bottom) == Some("true") && self.get(Property::Distance) != Some("0");
        Some(ScaffoldingShapes {
            stable: self.shape(),
            bottom: if has_bottom {
                g::blocks::SCAFFOLDING_BOTTOM
            } else {
                &[]
            },
            stable_voxel: self.voxel_shape(),
            has_bottom,
            bottom_voxel: &g::blocks::VOXEL_SHAPES[if has_bottom {
                g::blocks::SCAFFOLDING_BOTTOM_VOXEL as usize
            } else {
                0
            }],
        })
    }

    /// Collision boxes from the table.
    #[must_use]
    pub fn shape(self) -> &'static [Aabb] {
        g::blocks::SHAPES[g::blocks::STATE_SHAPE[self.0 as usize] as usize]
    }

    /// Fluid in this state: water / lava blocks with their `level`, waterlogged states and
    /// always-wet blocks (kelp, seagrass, bubble column) as water level 0.
    #[must_use]
    pub fn fluid(self) -> Option<Fluid> {
        let f = g::blocks::STATE_FLUID[self.0 as usize];
        let kind = match f >> 4 {
            1 => FluidKind::Water,
            2 => FluidKind::Lava,
            _ => return None,
        };
        Some(Fluid {
            kind,
            level: f & 15,
        })
    }

    /// Index of `prop`'s value in [`PropertyDef::values`], if the block has that property.
    #[must_use]
    pub fn get_index(self, prop: Property) -> Option<usize> {
        let layout = self.block().layout();
        let mut rem = usize::from(self.0 - layout.min_state);
        for &d in layout.properties.iter().rev() {
            let def = &g::blocks::PROPERTY_DEFS[d as usize];
            let n = def.values.len();
            if def.property == prop {
                return Some(rem % n);
            }
            rem /= n;
        }
        None
    }

    /// The value of `prop` (`"north"`, `"true"`, `"3"`), if the block has that property.
    #[must_use]
    pub fn get(self, prop: Property) -> Option<&'static str> {
        let def = self.property_def(prop)?;
        self.get_index(prop).map(|i| def.values[i])
    }

    /// This state with `prop` set to `value`; `None` if the block lacks the property or value.
    #[must_use]
    pub fn with(self, prop: Property, value: &str) -> Option<BlockState> {
        let layout = self.block().layout();
        let mut stride = 1usize;
        for &d in layout.properties.iter().rev() {
            let def = &g::blocks::PROPERTY_DEFS[d as usize];
            let n = def.values.len();
            if def.property == prop {
                let new = def.values.iter().position(|v| *v == value)?;
                let base = usize::from(self.0 - layout.min_state);
                let old = base / stride % n;
                let id = base - old * stride + new * stride + usize::from(layout.min_state);
                return Some(BlockState(id as u16));
            }
            stride *= n;
        }
        None
    }

    /// `(property, value)` pairs of this state, most significant property first.
    pub fn properties(self) -> impl Iterator<Item = (Property, &'static str)> {
        let block = self.block();
        block
            .properties()
            .filter_map(move |d| self.get(d.property).map(|v| (d.property, v)))
    }

    fn property_def(self, prop: Property) -> Option<&'static PropertyDef> {
        self.block().properties().find(|d| d.property == prop)
    }
}

/// `minecraft:oak_stairs[facing=north,half=bottom,...]`, the vanilla block-state string form.
impl fmt::Display for BlockState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "minecraft:{}", self.block().name())?;
        let mut first = true;
        for (p, v) in self.properties() {
            f.write_str(if first { "[" } else { "," })?;
            write!(f, "{}={v}", p.name())?;
            first = false;
        }
        if !first {
            f.write_str("]")?;
        }
        Ok(())
    }
}

/// 1.16.5 slime / magma cube dimensions for metadata size `n` (vanilla: the type's 2.04 x 2.04
/// `scaled(0.255F * n)`, eye height `0.625F * height`).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn slime_dimensions(n: i32) -> crate::common::EntityDimensions {
    let ratio = 0.255_f32 * n as f32;
    let s = 2.04_f32 * ratio;
    crate::common::EntityDimensions {
        width: s,
        height: s,
        eye_height: 0.625 * s,
    }
}

crate::macros::common_api!();
