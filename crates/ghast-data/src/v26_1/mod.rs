//! Minecraft 26.1 (protocol 775) data: the global block-state palette with properties, collision
//! voxel shapes and the per-state facts the movement code reads.
//!
//! Every movement-relevant value comes from the real 26.1 jar (oracle dump
//! `tests/golden/26.1/voxel_shapes.jsonl.gz`): collision voxel grids, per-state flags, fluids,
//! friction / speed / jump factors, the block tags and `instanceof` tests vanilla's movement code
//! makes. PrismarineJS minecraft-data (`pc/26.1`) supplies the property layout and display data
//! and is cross-checked against the jar at generation time.
//!
//! The hot path is one table read per state: [`BlockState::phys`] returns a packed
//! [`StatePhys`] (shape index, flags, fluid, per-kind extra bits, [`BlockKind`]).

use crate::common::{Aabb, SelectionShape, lookup_name};
use crate::generated::v26_1 as g;
use core::fmt;

pub use g::blocks::Property;

pub mod dig;
pub mod entities;
pub mod items;

pub use entities::{
    EntityTypeData, entities, entity, entity_by_name, entity_type, entity_types, vanilla_entity_size,
    vanilla_entity_sizes,
};
pub use items::{BlockSet, Food, Item, ItemData, Tool, ToolRule, food};

/// Number of blocks (ids `0..BLOCK_COUNT`).
pub const BLOCK_COUNT: usize = g::blocks::BLOCKS.len();
/// Number of block states (ids `0..STATE_COUNT`).
pub const STATE_COUNT: usize = g::blocks::STATE_BLOCK.len();

impl Property {
    /// Looks a property up by name (`"facing"`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Property> {
        g::blocks::PROPERTY_NAMES
            .binary_search_by(|e| e.0.cmp(name))
            .ok()
            .map(|i| g::blocks::PROPERTY_NAMES[i].1)
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

/// Bits of [`BlockData::flags`] (vanilla tags and `instanceof` tests of the movement code).
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
}

/// Static per-block data (jar values for everything movement reads).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockData {
    /// Registry name without namespace.
    pub name: &'static str,
    /// English display name (minecraft-data).
    #[cfg(feature = "display-names")]
    pub display_name: &'static str,
    /// Destroy time (minecraft-data; `None` = unbreakable).
    pub hardness: Option<f32>,
    /// Explosion resistance (minecraft-data).
    pub resistance: f32,
    /// Can be mined (minecraft-data).
    pub diggable: bool,
    /// Lets light through (minecraft-data).
    pub transparent: bool,
    /// Light emitted (minecraft-data).
    pub emit_light: u8,
    /// Light filtered (minecraft-data).
    pub filter_light: u8,
    /// `Block.getFriction()` (jar).
    pub friction: f32,
    /// `Block.getSpeedFactor()` (jar).
    pub speed_factor: f32,
    /// `Block.getJumpFactor()` (jar).
    pub jump_factor: f32,
    /// [`block_flags`] bits.
    pub flags: u32,
    /// Special movement behaviour.
    pub kind: BlockKind,
    /// Model offset function: `(max horizontal, max vertical, xyz)`; bamboo and pointed dripstone
    /// shift their collision shape by it.
    pub offset: Option<(f32, f32, bool)>,
}

/// VoxelShape implementation class (decides how a coordinate maps to a cell index in vanilla).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VoxelClass {
    /// `CubeVoxelShape`: points are `i / n`; vanilla indexes with `floor(c * n)`, clamped.
    Simple,
    /// `ArrayVoxelShape`: explicit point lists; vanilla binary-searches them.
    Array,
}

/// A vanilla 26.1 collision `VoxelShape` (block-local), exactly as the jar has it.
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

/// The collision voxel shape table (index 0 = empty).
#[must_use]
pub fn voxel_shapes() -> &'static [VoxelShapeRef] {
    &g::blocks::VOXEL_SHAPES
}

/// Index of `Shapes.block()` in [`voxel_shapes`].
pub const FULL_CUBE_VOXEL: u16 = g::blocks::FULL_CUBE_VOXEL;
/// Index of scaffolding's bottom shape in [`voxel_shapes`].
pub const SCAFFOLDING_BOTTOM_VOXEL: u16 = g::blocks::SCAFFOLDING_BOTTOM_VOXEL;
/// Index of powder snow's falling shape (`box(0, 0, 0, 1, 0.9F, 1)`) in [`voxel_shapes`].
pub const POWDER_SNOW_FALLING_VOXEL: u16 = g::blocks::POWDER_SNOW_FALLING_VOXEL;

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
    /// Collision shape index into [`voxel_shapes`] (context-free; unshifted for offset blocks).
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

/// A 26.1 block type (registry id).
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

    /// Static data.
    #[inline]
    #[must_use]
    pub fn data(self) -> &'static BlockData {
        &g::blocks::BLOCKS[self.0 as usize]
    }

    /// Registry name.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.data().name
    }

    /// `true` for `air`, `cave_air` and `void_air` (vanilla `BlockState.isAir`).
    #[must_use]
    pub const fn is_air(self) -> bool {
        matches!(self, Block::AIR | Block::CAVE_AIR | Block::VOID_AIR)
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

/// A 26.1 block state (global palette id).
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
    #[inline]
    #[must_use]
    pub const fn id(self) -> u16 {
        self.0
    }

    /// The block type.
    #[inline]
    #[must_use]
    pub fn block(self) -> Block {
        Block(g::blocks::STATE_BLOCK[self.0 as usize])
    }

    /// `true` for `air`, `cave_air` and `void_air`.
    #[must_use]
    pub fn is_air(self) -> bool {
        self.phys().has(state_flags::AIR)
    }

    /// The packed movement record (one table read).
    #[inline]
    #[must_use]
    pub fn phys(self) -> StatePhys {
        StatePhys(g::blocks::STATE_PHYS[self.0 as usize])
    }

    /// The context-free collision voxel shape (unshifted for offset blocks).
    #[must_use]
    pub fn voxel_shape(self) -> &'static VoxelShapeRef {
        &g::blocks::VOXEL_SHAPES[self.phys().shape() as usize]
    }

    /// The crosshair outline shape (`getShape` / `getOutlineShape` with the player's context,
    /// empty hand). For states with [`BlockState::outline_offset`] this is the unshifted shape: the
    /// block moves it by its per-position model offset.
    #[must_use]
    pub fn outline_shape(self) -> &'static SelectionShape {
        &g::selection::SELECTION_SHAPES[g::selection::STATE_OUTLINE[self.0 as usize] as usize]
    }

    /// The outline while the player holds this block's own item (scaffolding and, where it
    /// exists, the light block show a full cube then); otherwise [`BlockState::outline_shape`].
    #[must_use]
    pub fn outline_shape_holding_own_item(self) -> &'static SelectionShape {
        let t = &g::selection::OUTLINE_HOLDING_OWN_ITEM;
        match t.binary_search_by_key(&self.0, |e| e.0) {
            Ok(i) => &g::selection::SELECTION_SHAPES[t[i].1 as usize],
            Err(_) => self.outline_shape(),
        }
    }

    /// `true` when the outline is moved by the block's model offset (flowers, bamboo, ...).
    #[must_use]
    pub fn outline_offset(self) -> bool {
        g::selection::STATE_OUTLINE_OFFSET[self.0 as usize / 64] & (1 << (self.0 % 64)) != 0
    }

    /// The interaction shape (`getInteractionShape`): when the ray hits it nearer than the outline,
    /// the hit keeps the outline's position but takes this shape's face (cauldrons, composters,
    /// hoppers, scaffolding).
    #[must_use]
    pub fn interaction_shape(self) -> &'static SelectionShape {
        &g::selection::SELECTION_SHAPES[g::selection::STATE_INTERACTION[self.0 as usize] as usize]
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
        let def = self.block().properties().find(|d| d.property == prop)?;
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

    /// Parses `minecraft:name[prop=value,...]` (the namespace is optional).
    #[must_use]
    pub fn parse(s: &str) -> Option<BlockState> {
        let s = s.strip_prefix("minecraft:").unwrap_or(s);
        let (name, props) = match s.split_once('[') {
            Some((n, rest)) => (n, rest.strip_suffix(']')?),
            None => (s, ""),
        };
        let mut st = Block::from_name(name)?.default_state();
        for kv in props.split(',').filter(|p| !p.is_empty()) {
            let (k, v) = kv.split_once('=')?;
            st = st.with(Property::from_name(k)?, v)?;
        }
        Some(st)
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

#[cfg(test)]
mod tests;
