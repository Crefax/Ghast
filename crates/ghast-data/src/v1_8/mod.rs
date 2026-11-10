//! Minecraft 1.8.x (protocol 47) data: numeric block ids with 4-bit metadata.
//!
//! In 1.8 a block state travels on the wire and in chunk sections as one `u16`:
//! `block_id << 4 | meta` ([`BlockState::raw`]). Block ids are `0..=197`; every meta `0..=15`
//! is representable. Collision shapes are per (id, meta); blocks whose shape vanilla computes from
//! neighbours report a [`ShapeDependency::Neighbours`] instead of pretending to be static.
//!
//! Unregistered combinations: vanilla 1.8 reads a raw value that no registered block state
//! produces (unknown id, or a meta the block never uses, e.g. `stone:7`) as air.
//! [`BlockState::is_registered`] tells them apart (registered set from the 1.8.9 oracle),
//! [`BlockState::from_raw_vanilla`] applies vanilla's mapping, and the collision table already
//! gives unregistered states no boxes.

use crate::common::{
    Aabb, BiomeInfo, BlockInfo, BlockPhysics, Collision, DigMaterial, EffectInfo, EnchantmentInfo,
    EntityInfo, EntityKind, Fluid, FluidKind, FoodInfo, ItemInfo, NeighbourShape, PhysicsFlags,
    ShapeDependency, lookup_id, lookup_name,
};
use crate::generated::v1_8 as g;

pub mod dig;

/// Number of blocks (ids `0..BLOCK_COUNT`).
pub const BLOCK_COUNT: usize = g::blocks::BLOCKS.len();

/// Identity of the vanilla 1.8 materials that vanilla code compares with `==`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialKind {
    /// `Material.AIR` (only `air`).
    Air,
    /// `Material.WATER` (`water`, `flowing_water`).
    Water,
    /// `Material.LAVA` (`lava`, `flowing_lava`).
    Lava,
    /// `Material.ICE` (only `ice`; `packed_ice` has its own material).
    Ice,
    /// Any other material.
    Other,
}

/// A block's vanilla 1.8.9 material (flags from the oracle).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Material {
    /// Identity, for `== Material.X` checks.
    pub kind: MaterialKind,
    /// `Material.blocksMovement()`.
    pub blocks_movement: bool,
    /// `Material.isSolid()`.
    pub solid: bool,
    /// `Material.isFluid()`.
    pub fluid: bool,
    /// `Material.isOpaque()`.
    pub opaque: bool,
    /// `Material.isReplaceable()`.
    pub replaceable: bool,
}

/// Vanilla 1.8.9 per-block flags, dumped by the oracle (`tests/golden/1.8.9/collision_boxes.jsonl.gz`
/// `"type":"block"` lines). In 1.8 these are block-level, not per state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VanillaProps {
    /// `Block.isFullCube()` (Yarn): used e.g. by `ClientPlayerEntity.pushOutOfBlocks`.
    pub full_cube: bool,
    /// `Block.isNormalBlock()`: material blocks movement and renders as a normal block.
    pub normal_block: bool,
    /// The `Block.fullBlock` field.
    pub full_block: bool,
    /// `Block.renderAsNormalBlock()`.
    pub render_as_normal_block: bool,
    /// `Block.hasCollision()` (the raytrace "collidable" check, not the collision box).
    pub has_collision: bool,
    /// The block's material.
    pub material: Material,
}

/// What the 1.8.9 crosshair ray test (`Block.rayTrace` via `World.rayTrace`) sees of a block state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    /// Bounds after `setBoundingBox(world, pos)` with all neighbours air (block-local). For
    /// neighbour-dependent blocks this is the isolated shape, see `dependency`.
    pub bounds: Aabb,
    /// Same classes as the collision dependency (fences, panes, walls, doors, chests, stairs).
    pub dependency: ShapeDependency,
    /// Stairs: vanilla's ray test runs 8 passes over half-cube sub-boxes instead of using `bounds`.
    pub multi_pass_ray: bool,
    /// `Block.canCollide(state, false)`: rays skip the block when false (air, fire, fluids, ...).
    pub can_collide: bool,
    /// `Block.canCollide(state, true)` (ray tests that stop at fluids).
    pub can_collide_liquid: bool,
    /// The block has a collision box (for `ignoreBlockWithoutBoundingBox` ray tests).
    pub has_collision_box: bool,
}

/// A 1.8 block type (registry id `0..=197`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Block(pub(crate) u8);

impl Block {
    /// The block with this registry id, if it exists in 1.8.
    #[must_use]
    pub const fn from_id(id: u8) -> Option<Block> {
        if (id as usize) < BLOCK_COUNT {
            Some(Block(id))
        } else {
            None
        }
    }

    /// Looks a block up by registry name (`"stone"`, no namespace).
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
        &g::blocks::BLOCKS[self.0 as usize]
    }

    /// Registry name.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.info().name
    }

    /// Physics properties (slipperiness, climbable, ...).
    #[must_use]
    pub fn physics(self) -> &'static BlockPhysics {
        &g::blocks::BLOCKS[self.0 as usize].physics
    }

    /// Vanilla 1.8.9 shape and material flags.
    #[must_use]
    pub fn vanilla(self) -> &'static VanillaProps {
        &g::blocks::VANILLA_PROPS[self.0 as usize]
    }

    /// Vanilla material (shorthand for `self.vanilla().material`).
    #[must_use]
    pub fn material(self) -> Material {
        g::blocks::VANILLA_PROPS[self.0 as usize].material
    }

    /// `Block.isFullCube()` (shorthand for `self.vanilla().full_cube`).
    #[must_use]
    pub fn full_cube(self) -> bool {
        g::blocks::VANILLA_PROPS[self.0 as usize].full_cube
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
    pub fn all() -> impl ExactSizeIterator<Item = Block> {
        (0..BLOCK_COUNT as u8).map(Block)
    }
}

/// A 1.8 block state: `block_id << 4 | meta`, exactly the wire / chunk-section value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockState(u16);

impl BlockState {
    /// Air (raw 0).
    pub const AIR: BlockState = BlockState(0);

    /// Wraps a raw `id << 4 | meta` value; `None` if the block id is not a 1.8 block.
    #[must_use]
    pub const fn from_raw(raw: u16) -> Option<BlockState> {
        if ((raw >> 4) as usize) < BLOCK_COUNT {
            Some(BlockState(raw))
        } else {
            None
        }
    }

    /// Vanilla's chunk-read mapping: unknown block ids and unregistered metas become air.
    #[must_use]
    pub fn from_raw_vanilla(raw: u16) -> BlockState {
        match BlockState::from_raw(raw) {
            Some(s) if s.is_registered() => s,
            _ => BlockState::AIR,
        }
    }

    /// `true` when vanilla 1.8.9 has a block state for this (id, meta); otherwise it reads as air.
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

    /// Collision boxes, what else they depend on, and how vanilla places them in the world
    /// ([`Collision::world_box`]). Hot path: three table lookups.
    #[must_use]
    pub fn collision(self) -> Collision {
        Collision {
            boxes: g::blocks::SHAPES[g::blocks::STATE_SHAPE[self.0 as usize] as usize],
            dependency: g::blocks::BLOCK_SHAPE_DEPENDENCY[(self.0 >> 4) as usize],
            world_coords: g::blocks::WORLD_COORDS
                [g::blocks::STATE_WORLD_COORDS[self.0 as usize] as usize],
        }
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

    /// The collision boxes from the table (for dependent shapes: the isolated / default shape).
    #[must_use]
    pub fn shape(self) -> &'static [Aabb] {
        g::blocks::SHAPES[g::blocks::STATE_SHAPE[self.0 as usize] as usize]
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

/// A 1.8.9 crafting recipe (from the oracle's dump of `RecipeDispatcher.getAllRecipes()`).
///
/// Ingredients are `(item id, data)`; data `32767` matches any damage value (vanilla's wildcard,
/// used for block ingredients). Outputs are `(item id, count, damage)`; no output carries NBT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recipe {
    /// Shaped recipe: `inputs` is row-major, `width * height` slots, `None` = must be empty.
    Shaped {
        /// Pattern width.
        width: u8,
        /// Pattern height.
        height: u8,
        /// Row-major ingredients.
        inputs: &'static [Option<(u16, u16)>],
        /// Result `(id, count, damage)`.
        output: (u16, u8, u16),
    },
    /// Shapeless recipe: every input must be matched by exactly one grid item.
    Shapeless {
        /// Ingredients in vanilla's list order.
        inputs: &'static [(u16, u16)],
        /// Result `(id, count, damage)`.
        output: (u16, u8, u16),
    },
    /// A recipe implemented in code.
    Special(SpecialRecipe),
}

/// Vanilla 1.8.9 code-defined recipes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpecialRecipe {
    /// `ArmorDyeRecipeType`: dye leather armour.
    ArmorDye,
    /// `FireworkRecipeType`: rockets, stars, star fades.
    Firework,
    /// `PatternRecipeType`: copy a patterned banner onto a blank banner of the same colour.
    BannerDuplicate,
    /// `BookCloningRecipeType`: copy a written book.
    BookCloning,
    /// `MapCloningRecipeType`: copy a map.
    MapCloning,
    /// `MapUpscaleRecipeType`: zoom a map out.
    MapUpscale,
    /// `RepairingRecipeType`: combine two damaged tools.
    Repairing,
    /// `CopyingRecipeType`: add a pattern to a banner with dye (appends to `BlockEntityTag.Patterns`).
    BannerPattern,
}

/// Wildcard ingredient data value (matches any damage).
pub const RECIPE_ANY_DATA: u16 = 32767;

/// All 1.8.9 crafting recipes in vanilla's match order: the first that matches the grid wins.
#[must_use]
pub fn recipes() -> &'static [Recipe] {
    &g::recipes::RECIPES
}

/// 1.8.9 slime / magma cube dimensions for metadata size `n` (vanilla `SlimeEntity.setSize`:
/// `0.51000005F * n` square, eye height `0.625F * height`).
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
