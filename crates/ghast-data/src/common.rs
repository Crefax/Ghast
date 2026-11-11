//! Version-independent data types shared by every version module.
//!
//! The generated per-version tables (`crate::v1_8`, `crate::v1_16_5`) are built from these types.
//! Everything here is plain `Copy` data or `&'static` slices so the tables can live in `static`
//! memory with no runtime initialisation.

/// Axis-aligned box in block-local coordinates (`0.0..=1.0` for a full cube; some shapes such as
/// fences or piston arms extend outside that range).
///
/// Field names follow Yarn's `Box` (`minX` .. `maxZ`). Values are exact `f64` copies of the source
/// data; every value in the shipped tables is a dyadic fraction, so it is also exactly the `f64`
/// produced by vanilla's `(double)someFloat` bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// Minimum X (west edge).
    pub min_x: f64,
    /// Minimum Y (bottom).
    pub min_y: f64,
    /// Minimum Z (north edge).
    pub min_z: f64,
    /// Maximum X (east edge).
    pub max_x: f64,
    /// Maximum Y (top).
    pub max_y: f64,
    /// Maximum Z (south edge).
    pub max_z: f64,
}

impl Aabb {
    /// The full unit cube `[0, 1]³`.
    pub const FULL: Aabb = Aabb::new(0.0, 0.0, 0.0, 1.0, 1.0, 1.0);

    /// Creates a box from its six bounds.
    #[must_use]
    pub const fn new(
        min_x: f64,
        min_y: f64,
        min_z: f64,
        max_x: f64,
        max_y: f64,
        max_z: f64,
    ) -> Self {
        Aabb {
            min_x,
            min_y,
            min_z,
            max_x,
            max_y,
            max_z,
        }
    }

    /// The box translated by a block position (`x`, `y`, `z` are the block's integer coordinates),
    /// in `double` arithmetic. Use [`Collision::world_box`] for vanilla-exact placement.
    #[must_use]
    pub fn at(&self, x: i32, y: i32, z: i32) -> Aabb {
        let (x, y, z) = (f64::from(x), f64::from(y), f64::from(z));
        Aabb::new(
            self.min_x + x,
            self.min_y + y,
            self.min_z + z,
            self.max_x + x,
            self.max_y + y,
            self.max_z + z,
        )
    }
}

/// A block's outline (selection) or interaction `VoxelShape`, block-local, as the crosshair ray
/// tests it (1.14+ versions; oracle `selection_shapes.jsonl.gz`).
///
/// The ray uses two views of a shape: the filled-cell grid (is the ray's first test point inside
/// the shape?) and the box list (`toAabbs()` / `getBoundingBoxes()` in vanilla's order, which
/// decides ties between equally near faces).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionShape {
    /// Vanilla's cube shape class (`CubeVoxelShape` / `SimpleVoxelShape`): a coordinate's cell is
    /// `floor(c * n)` clamped to `-1..=n`; otherwise (array shapes) it is found by binary search
    /// over the points.
    pub cube: bool,
    /// Cell counts `[nx, ny, nz]` (`0, 0, 0` for the empty shape).
    pub sizes: [u8; 3],
    /// Cell boundaries along X (`nx + 1` values; the empty shape has `[0.0]`).
    pub xs: &'static [f64],
    /// Cell boundaries along Y.
    pub ys: &'static [f64],
    /// Cell boundaries along Z.
    pub zs: &'static [f64],
    /// Filled-cell bitset, bit index `(x * ny + y) * nz + z`, 64 bits per word.
    pub cells: &'static [u64],
    /// `VoxelShape.isEmpty()`.
    pub empty: bool,
    /// The shape's boxes in vanilla's order.
    pub boxes: &'static [Aabb],
}

impl SelectionShape {
    /// `true` when cell (`x`, `y`, `z`) is filled; `false` outside the grid.
    #[must_use]
    pub fn filled(&self, x: i32, y: i32, z: i32) -> bool {
        let [nx, ny, nz] = self.sizes.map(i32::from);
        if x < 0 || y < 0 || z < 0 || x >= nx || y >= ny || z >= nz {
            return false;
        }
        let i = ((x * ny + y) * nz + z) as usize;
        self.cells[i / 64] & (1 << (i % 64)) != 0
    }
}

/// What a block's collision shape depends on besides its own block state.
///
/// When this is not [`ShapeDependency::None`], [`Collision::boxes`] is only the shape for the
/// "default" situation described on each variant and the caller (world / physics crate) must
/// compute the real shape. The tables never pretend such shapes are static.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ShapeDependency {
    /// The shape is fully determined by the block state.
    None,
    /// The shape depends on neighbouring blocks (1.8 "actual state" blocks). `boxes` is the
    /// shape with no connecting neighbours, as given by minecraft-data.
    Neighbours(NeighbourShape),
    /// The shape depends on the colliding entity (1.16 scaffolding: sneaking / standing above).
    /// `boxes` is the shape minecraft-data reports.
    EntityContext,
    /// The shape depends on a block entity (1.16 shulker box lid, moving piston). `boxes` is the
    /// shape minecraft-data reports.
    BlockEntity,
    /// The shape is shifted by a per-position pseudo-random XZ offset (1.16 bamboo). `boxes` is the
    /// un-offset shape.
    PositionOffset,
}

/// Family of a neighbour-dependent shape. The world/physics crate implements the vanilla
/// connection rules per family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NeighbourShape {
    /// Fences: centre post plus arms towards connecting neighbours, 1.5 high.
    Fence,
    /// Glass panes and iron bars.
    Pane,
    /// Cobblestone walls (1.5 high, post omitted on straight runs).
    Wall,
    /// Stairs: straight / inner / outer corner shapes from adjacent stairs.
    Stairs,
    /// Doors: the shape combines the lower half (facing, open) with the upper half (hinge).
    Door,
    /// Chests: the box extends to the block edge towards an adjacent chest of the same kind.
    Chest,
    /// 1.12 chorus plants: a centre box plus an arm towards each connecting neighbour (chorus
    /// plant / flower, end stone below).
    ChorusPlant,
}

/// How vanilla turns one bound of a collision box into an absolute world coordinate.
///
/// Most 1.8 blocks compute `blockPos + localBound` in `double` ([`WorldCoord::Double`]), which is
/// exact. A few (cactus, cake, fence gate, soul sand, snow layer) do the addition in `float`:
/// `(float) (blockPos + int_offset) + term`, widened to `double`. That rounds once
/// `|blockPos|` is large (from 2^20 ≈ 1M blocks for 1/16 steps), and the rounded value is what
/// vanilla collides with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WorldCoord {
    /// `f64::from(pos) + local` (the matching [`Aabb`] bound).
    Double,
    /// `f64::from((pos + int_offset) as f32 + term)`: Java `i2f`, `fadd`, `f2d`.
    Float {
        /// Integer added to the block coordinate before the `float` conversion (`pos.getX() + 1`).
        int_offset: i8,
        /// `float` term added afterwards (negative for `pos.getX() + 1 - f`).
        term: f32,
    },
}

/// Per-bound world-coordinate rules of one box, in [`Aabb`] field order
/// (`min_x, min_y, min_z, max_x, max_y, max_z`).
pub type WorldCoords = [WorldCoord; 6];

impl WorldCoord {
    /// The absolute coordinate for block coordinate `pos` and local bound `local`.
    #[inline]
    #[must_use]
    pub fn apply(self, pos: i32, local: f64) -> f64 {
        match self {
            WorldCoord::Double => f64::from(pos) + local,
            #[allow(clippy::cast_precision_loss)]
            WorldCoord::Float { int_offset, term } => {
                f64::from(pos.wrapping_add(i32::from(int_offset)) as f32 + term)
            }
        }
    }
}

/// `(min, max)` of two coordinates (never NaN here).
fn ordered(a: f64, b: f64) -> (f64, f64) {
    if a <= b { (a, b) } else { (b, a) }
}

/// The collision of one block state.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Collision {
    /// Boxes in block-local coordinates; empty means no collision.
    pub boxes: &'static [Aabb],
    /// Whether `boxes` is the final shape or needs more context.
    pub dependency: ShapeDependency,
    /// `Some` when vanilla builds these boxes with `float` world coordinates (one rule set per
    /// box, parallel to `boxes`); `None` = plain `double` addition, i.e. [`Aabb::at`].
    pub world_coords: Option<&'static [WorldCoords]>,
}

impl Collision {
    /// `true` when at least one bound is computed in `float` world coordinates.
    #[must_use]
    pub fn uses_float_world_coords(&self) -> bool {
        self.world_coords.is_some()
    }

    /// Box `i` placed at block position (`x`, `y`, `z`), bit-exact as vanilla computes it.
    ///
    /// # Panics
    /// If `i >= self.boxes.len()`.
    #[inline]
    #[must_use]
    pub fn world_box(&self, i: usize, x: i32, y: i32, z: i32) -> Aabb {
        let b = &self.boxes[i];
        match self.world_coords {
            None => b.at(x, y, z),
            Some(rules) => {
                let r = &rules[i];
                // Vanilla's Box constructor orders each axis (Math.min / Math.max); float rounding
                // can invert a thin bound pair far from the origin (cake below x = -2^24).
                let (x0, x1) = ordered(r[0].apply(x, b.min_x), r[3].apply(x, b.max_x));
                let (y0, y1) = ordered(r[1].apply(y, b.min_y), r[4].apply(y, b.max_y));
                let (z0, z1) = ordered(r[2].apply(z, b.min_z), r[5].apply(z, b.max_z));
                Aabb::new(x0, y0, z0, x1, y1, z1)
            }
        }
    }

    /// All boxes placed at block position (`x`, `y`, `z`), bit-exact as vanilla computes them.
    pub fn world_boxes(&self, x: i32, y: i32, z: i32) -> impl ExactSizeIterator<Item = Aabb> + '_ {
        (0..self.boxes.len()).map(move |i| self.world_box(i, x, y, z))
    }
    /// `true` when the block state has no collision boxes and nothing else can add one.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty() && self.dependency == ShapeDependency::None
    }

    /// `true` when the shape is exactly one full unit cube and does not depend on anything else.
    #[must_use]
    pub fn is_full_cube(&self) -> bool {
        self.dependency == ShapeDependency::None && self.boxes == [Aabb::FULL]
    }
}

/// minecraft-data's coarse bounding-box classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BoundingBox {
    /// No collision ("empty").
    Empty,
    /// Has some collision ("block"); see the block state's [`Collision`] for the real shape.
    Block,
}

/// minecraft-data's harvest "material" group: selects the tool speed multipliers
/// (see the per-version `tool_multiplier`). This is NOT vanilla's `Material` class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DigMaterial {
    /// Pickaxe group.
    Rock,
    /// Axe group.
    Wood,
    /// Axe/sword group (plants).
    Plant,
    /// Sword-only group (melons, pumpkins).
    Melon,
    /// Leaves (sword, shears).
    Leaves,
    /// Shovel group.
    Dirt,
    /// Cobweb (sword, shears).
    Web,
    /// Wool (shears).
    Wool,
    /// 1.16 data's explicit "default" group (no tool bonus).
    Default,
}

/// A block's item drop as listed by minecraft-data (no loot-table logic, no fortune).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drop {
    /// Item id.
    pub item: u16,
    /// Item metadata / damage (1.8 only); `None` = 0 / unspecified.
    pub meta: Option<u8>,
    /// Minimum count.
    pub min: u8,
    /// Maximum count.
    pub max: u8,
}

/// Bit set of physics-relevant block flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct PhysicsFlags(pub u16);

impl PhysicsFlags {
    /// No flags.
    pub const NONE: PhysicsFlags = PhysicsFlags(0);
    /// The entity can climb while inside this block (ladder, vine, ...).
    pub const CLIMBABLE: PhysicsFlags = PhysicsFlags(1 << 0);
    /// The block is a water block (still / flowing). In 1.16 waterlogged states are reported by the
    /// per-state fluid table instead.
    pub const WATER: PhysicsFlags = PhysicsFlags(1 << 1);
    /// The block is a lava block (still / flowing).
    pub const LAVA: PhysicsFlags = PhysicsFlags(1 << 2);
    /// Entities inside are slowed by the block's `stuck_multiplier` (cobweb, sweet berry bush).
    pub const STUCK: PhysicsFlags = PhysicsFlags(1 << 3);
    /// Slime block: bounces on landing and slows walking.
    pub const SLIME: PhysicsFlags = PhysicsFlags(1 << 4);
    /// Bed: bounces on landing (1.12+ behaviour; never set for 1.8).
    pub const BED_BOUNCE: PhysicsFlags = PhysicsFlags(1 << 5);
    /// Honey block: wall sliding (1.15+).
    pub const HONEY: PhysicsFlags = PhysicsFlags(1 << 6);
    /// Bubble column (1.13+): pushes entities up or down.
    pub const BUBBLE_COLUMN: PhysicsFlags = PhysicsFlags(1 << 7);
    /// 1.8 soul sand: `speed_factor` is applied to horizontal motion when the entity's box
    /// intersects the block (vanilla `Block.onEntityCollision`), not as a ground speed factor.
    pub const COLLIDE_SLOWDOWN: PhysicsFlags = PhysicsFlags(1 << 8);
    /// Fence: 1.16 `#minecraft:fences` tag; 1.8 `FenceBlock` (wooden + nether brick fences).
    pub const FENCE: PhysicsFlags = PhysicsFlags(1 << 9);
    /// Wall: 1.16 `#minecraft:walls` tag; 1.8 `WallBlock` (cobblestone wall).
    pub const WALL: PhysicsFlags = PhysicsFlags(1 << 10);
    /// Fence gate: 1.16 `#minecraft:fence_gates` tag; 1.8 `FenceGateBlock`.
    pub const FENCE_GATE: PhysicsFlags = PhysicsFlags(1 << 11);
    /// Trapdoor: 1.16 `#minecraft:trapdoors` tag; 1.8 `TrapdoorBlock`.
    pub const TRAPDOOR: PhysicsFlags = PhysicsFlags(1 << 12);
    /// 1.16 `#minecraft:soul_speed_blocks` (soul sand, soul soil); never set for 1.8.
    pub const SOUL_SPEED: PhysicsFlags = PhysicsFlags(1 << 13);
    /// 1.16 `#minecraft:fire` (fire, soul fire); never set for 1.8.
    pub const FIRE: PhysicsFlags = PhysicsFlags(1 << 14);

    /// `true` when every flag of `other` is set in `self`.
    #[must_use]
    pub const fn contains(self, other: PhysicsFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// Union of two flag sets.
    #[must_use]
    pub const fn union(self, other: PhysicsFlags) -> PhysicsFlags {
        PhysicsFlags(self.0 | other.0)
    }
}

/// Physics properties of a block (vanilla facts that minecraft-data does not carry; maintained in
/// `codegen/data/src/facts.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BlockPhysics {
    /// Ground friction ("slipperiness"), vanilla `float`: default 0.6, ice 0.98, slime 0.8.
    pub slipperiness: f32,
    /// Horizontal speed multiplier. The exact `f64` vanilla multiplies by: `0.4` (a `double`
    /// literal) for 1.8 soul sand, `0.4f32 as f64` for 1.16 speed factors. `1.0` = none.
    pub speed_factor: f64,
    /// Jump velocity multiplier (vanilla `float`; 1.16 honey block 0.5). `1.0` = none.
    pub jump_factor: f32,
    /// Motion multiplier `[x, y, z]` while inside the block (when [`PhysicsFlags::STUCK`]).
    pub stuck_multiplier: [f64; 3],
    /// Flag bits.
    pub flags: PhysicsFlags,
}

impl BlockPhysics {
    /// Properties of an ordinary block.
    pub const DEFAULT: BlockPhysics = BlockPhysics {
        slipperiness: 0.6,
        speed_factor: 1.0,
        jump_factor: 1.0,
        stuck_multiplier: [1.0, 1.0, 1.0],
        flags: PhysicsFlags::NONE,
    };

    /// The speed factor as the `float` 1.16 `Block.getVelocityMultiplier()` returns (exact: the
    /// 1.16 tables store `f64::from(f32)`). For 1.8 use [`BlockPhysics::speed_factor`] directly.
    #[allow(clippy::cast_possible_truncation)]
    #[must_use]
    pub const fn velocity_multiplier(&self) -> f32 {
        self.speed_factor as f32
    }

    /// Shorthand for `self.flags.contains(flag)`.
    #[must_use]
    pub const fn has(&self, flag: PhysicsFlags) -> bool {
        self.flags.contains(flag)
    }
}

/// Static description of a block (cold data: names, digging, light, drops).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BlockInfo {
    /// Registry id (1.8: 0..=255, 1.16: 0..763).
    pub id: u16,
    /// Registry name without namespace (`"stone"`).
    pub name: &'static str,
    /// English display name.
    #[cfg(feature = "display-names")]
    pub display_name: &'static str,
    /// Hardness (vanilla "strength"); `None` when minecraft-data lists `null` (unbreakable in 1.8).
    /// Note: 1.16 data lists unbreakable blocks (bedrock, barrier, ...) with hardness `0` and
    /// `diggable == false`; always check [`BlockInfo::diggable`].
    pub hardness: Option<f32>,
    /// Blast resistance.
    pub resistance: f32,
    /// Max stack size of the block's item.
    pub stack_size: u8,
    /// Whether the block can be dug at all.
    pub diggable: bool,
    /// minecraft-data "transparent" flag (light passes / not an opaque cube).
    pub transparent: bool,
    /// Emitted light level 0..=15.
    pub emit_light: u8,
    /// Light absorbed 0..=15.
    pub filter_light: u8,
    /// Coarse bounding-box class.
    pub bounding_box: BoundingBox,
    /// Tool-speed group; `None` = no tool bonus.
    pub material: Option<DigMaterial>,
    /// Item ids that can harvest this block (get drops). Empty = any tool / hand.
    pub harvest_tools: &'static [u16],
    /// Item drops (basic list, no loot tables).
    pub drops: &'static [Drop],
    /// Physics properties.
    pub physics: BlockPhysics,
}

impl BlockInfo {
    /// `true` when the block needs a specific tool to drop anything.
    #[must_use]
    pub fn requires_tool(&self) -> bool {
        !self.harvest_tools.is_empty()
    }

    /// `true` when `item` (an item id, or `None` for the empty hand) harvests the block.
    #[must_use]
    pub fn can_harvest_with(&self, item: Option<u16>) -> bool {
        self.harvest_tools.is_empty() || item.is_some_and(|i| self.harvest_tools.contains(&i))
    }
}

/// Fluid kind of a block state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FluidKind {
    /// Water (incl. 1.16 waterlogged states and always-wet plants / bubble columns).
    Water,
    /// Lava.
    Lava,
}

/// A fluid occupying a block state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fluid {
    /// Kind of fluid.
    pub kind: FluidKind,
    /// Vanilla `level` value (0 = source, 1..=7 flowing, 8..=15 falling). Waterlogged / always-wet
    /// blocks report 0.
    pub level: u8,
}

impl Fluid {
    /// Vanilla `FluidState` amount 1..=8: source 8, flowing `8 - level`, falling 8
    /// (`FluidBlock.getFluidState` maps `level >= 8` to falling with amount 8).
    #[must_use]
    pub const fn amount(self) -> u8 {
        if self.level == 0 || self.level >= 8 {
            8
        } else {
            8 - self.level
        }
    }

    /// `true` for falling fluid (`level >= 8`).
    #[must_use]
    pub const fn falling(self) -> bool {
        self.level >= 8
    }

    /// Vanilla `FlowableFluid.getHeight(state)` = `amount / 9F`: the surface height when the block
    /// above holds no fluid of the same kind (then vanilla uses 1.0).
    #[must_use]
    pub fn height(self) -> f32 {
        f32::from(self.amount()) / 9.0
    }

    /// `true` for a source / still fluid (`level == 0`, also waterlogged states).
    #[must_use]
    pub const fn still(self) -> bool {
        self.level == 0
    }
}

/// An item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemInfo {
    /// Item id.
    pub id: u16,
    /// Registry name without namespace.
    pub name: &'static str,
    /// English display name.
    #[cfg(feature = "display-names")]
    pub display_name: &'static str,
    /// Max stack size.
    pub stack_size: u8,
    /// Durability for damageable items.
    pub max_durability: Option<u16>,
    /// 1.8 metadata variants `(meta, display name)`; empty for 1.16.
    #[cfg(feature = "display-names")]
    pub variations: &'static [(u8, &'static str)],
}

/// Broad entity kind from minecraft-data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntityKind {
    /// Living entity ("mob"). In 1.8 these use the mob-spawn packet's id space.
    Mob,
    /// Non-living "object" (1.8 spawn-object packet id space: boats, items, arrows, ...).
    Object,
    /// Anything else minecraft-data reports (`"UNKNOWN"`, `"player"`, ...).
    Other,
}

/// An entity type.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct EntityInfo {
    /// Network type id (in 1.8 mob and object ids are separate spaces, see [`EntityKind`]).
    pub id: u16,
    /// Name as listed by minecraft-data (1.8: legacy CamelCase names such as `"PigZombie"`).
    pub name: &'static str,
    /// English display name.
    #[cfg(feature = "display-names")]
    pub display_name: &'static str,
    /// Mob / object.
    pub kind: EntityKind,
    /// minecraft-data category (`"Hostile mobs"`, ...).
    pub category: &'static str,
    /// Box width; `None` when unknown. From minecraft-data, NOT verified float-exact.
    pub width: Option<f32>,
    /// Box height; `None` when unknown. From minecraft-data, NOT verified float-exact.
    pub height: Option<f32>,
}

/// Vanilla entity dimensions (`float`s, as the game stores them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityDimensions {
    /// Box width (x and z).
    pub width: f32,
    /// Box height.
    pub height: f32,
    /// Eye height above the feet.
    pub eye_height: f32,
}

impl EntityDimensions {
    /// The bounding box with its feet centre at (`x`, `y`), `z`, bit-exact as vanilla builds it:
    /// half width in `float`, then `x - (double) halfWidth`, `y + (double) height`, ...
    #[must_use]
    pub fn bounding_box(&self, x: f64, y: f64, z: f64) -> Aabb {
        let half = f64::from(self.width / 2.0);
        Aabb::new(
            x - half,
            y,
            z - half,
            x + half,
            y + f64::from(self.height),
            z + half,
        )
    }
}

/// When the vanilla client applies a size variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VariantApplied {
    /// The size right after the spawn packet (the `"default"` variant).
    Spawn,
    /// Immediately when the entity metadata packet arrives.
    Metadata,
    /// In the entity's next client tick (passive babies, 1.8 wither skeleton, armor stand marker).
    NextTick,
    /// From the client player's own input (1.8 sneaking eye height).
    Input,
    /// Only on the server: the client never applies it (e.g. 1.8 baby zombie's 0.3 x 0.975); listed
    /// for reference.
    Server,
}

/// One measured size variant of an entity type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityVariant {
    /// `"default"`, `"baby"`, `"baby_server"`, `"wither"`, `"elder"`, `"size_1"`..`"size_4"`,
    /// `"small"`, `"marker"`, `"small_marker"`, `"sneaking"`, `"villager"`.
    pub name: &'static str,
    /// The dimensions.
    pub dimensions: EntityDimensions,
    /// When the client applies them.
    pub applied: VariantApplied,
}

/// Vanilla sizes of one entity type, measured by the oracle on the client, spawned the way the
/// client spawns it from its packet, then metadata-dependent variants applied like the client does
/// (`oracle/README.md`, "entity_sizes.jsonl.gz").
///
/// Boxes: [`EntityDimensions::bounding_box`] is the centred box vanilla builds on spawn and every
/// position update. Hanging entities (paintings, item frames) build their box from facing and motive
/// instead, and vanilla's resize keeps the old min corner until the next position update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VanillaEntitySize {
    /// Name: 1.8 legacy registry name (`"Zombie"`, `"PigZombie"`, `"Player"`), 1.16 registry path
    /// (`"zombie"`, `"player"`).
    pub name: &'static str,
    /// Vanilla registry id (1.8 `EntityType` id = the spawn-mob type for mobs; 1.16 registry id);
    /// `-1` when the type has none (1.8 players, lightning, fishing bobber) or no client instance.
    pub registry_id: i32,
    /// 1.8 spawn packet kind: `"mob"`, `"object"`, `"xp_orb"`, `"painting"`, `"global"`,
    /// `"player"`; empty for 1.16.
    pub spawn: &'static str,
    /// 1.8 spawn-object packet `(type, data)` pairs that create this entity (minecart kinds share
    /// type 10 with different data).
    pub object_types: &'static [(i32, i32)],
    /// Extra growth of the box when picking entities with the crosshair (`getTargetingMargin`).
    pub targeting_margin: f32,
    /// 1.16 `EntityDimensions.fixed` (not scaled for babies); `false` in 1.8.
    pub fixed: bool,
    /// 1.16 registered type dimensions `(width, height)`; `None` in 1.8.
    pub registered: Option<(f32, f32)>,
    /// Measured variants, `"default"` first.
    pub variants: &'static [EntityVariant],
    /// 1.16 player: dimensions per pose (`"standing"`, `"crouching"`, `"swimming"`, ...).
    pub poses: &'static [(&'static str, EntityDimensions)],
}

impl VanillaEntitySize {
    /// A measured variant by name.
    #[must_use]
    pub fn variant(&self, name: &str) -> Option<&'static EntityVariant> {
        self.variants.iter().find(|v| v.name == name)
    }

    /// The size right after spawning (`None` for 1.16 types without a client instance; use
    /// [`VanillaEntitySize::registered`]).
    #[must_use]
    pub fn default_dimensions(&self) -> Option<EntityDimensions> {
        self.variant("default").map(|v| v.dimensions)
    }

    /// The `"baby"` variant as the client applies it, for types that have one.
    #[must_use]
    pub fn baby(&self) -> Option<EntityDimensions> {
        self.variant("baby").map(|v| v.dimensions)
    }

    /// 1.16 player dimensions for a pose.
    #[must_use]
    pub fn pose(&self, name: &str) -> Option<EntityDimensions> {
        self.poses.iter().find(|(n, _)| *n == name).map(|(_, d)| *d)
    }
}

/// A biome.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BiomeInfo {
    /// Biome id.
    pub id: u16,
    /// Registry name.
    pub name: &'static str,
    /// English display name.
    #[cfg(feature = "display-names")]
    pub display_name: &'static str,
    /// Category (`"ocean"`, `"plains"`, ...).
    pub category: &'static str,
    /// Temperature.
    pub temperature: f32,
    /// Rainfall / downfall.
    pub rainfall: f32,
    /// Precipitation (`"rain"`, `"snow"`, `"none"`).
    pub precipitation: &'static str,
    /// Dimension (`"overworld"`, `"nether"`, `"end"`).
    pub dimension: &'static str,
}

/// Linear enchanting cost `a * level + b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnchantCost {
    /// Per-level factor.
    pub a: i16,
    /// Offset.
    pub b: i16,
}

/// An enchantment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EnchantmentInfo {
    /// Enchantment id (1.8: the NBT `id` short; 1.16: registry id).
    pub id: u16,
    /// Registry name.
    pub name: &'static str,
    /// English display name.
    #[cfg(feature = "display-names")]
    pub display_name: &'static str,
    /// Highest level.
    pub max_level: u8,
    /// Minimum enchanting cost.
    pub min_cost: EnchantCost,
    /// Maximum enchanting cost.
    pub max_cost: EnchantCost,
    /// Names of incompatible enchantments.
    pub exclude: &'static [&'static str],
    /// Applicable item category (`"armor"`, `"digger"`, ...).
    pub category: &'static str,
    /// Random-selection weight.
    pub weight: u8,
    /// Only from loot / trades.
    pub treasure_only: bool,
    /// Curse.
    pub curse: bool,
}

/// A food item.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct FoodInfo {
    /// Item id (resolved by name against this version's item table).
    pub item: u16,
    /// Item name.
    pub name: &'static str,
    /// Hunger points restored.
    pub food_points: u8,
    /// Saturation restored.
    pub saturation: f32,
}

/// A status effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EffectInfo {
    /// Effect id.
    pub id: u8,
    /// Name as listed by minecraft-data (`"Speed"`).
    pub name: &'static str,
    /// English display name.
    #[cfg(feature = "display-names")]
    pub display_name: &'static str,
    /// `true` for beneficial effects.
    pub good: bool,
}

/// Binary search in a name-sorted `(name, id)` index.
#[cfg_attr(not(any(feature = "v1_8", feature = "v1_16_5")), allow(dead_code))]
pub(crate) fn lookup_name<T: Copy>(index: &[(&'static str, T)], name: &str) -> Option<T> {
    index
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .map(|i| index[i].1)
}

/// Binary search in an id-sorted slice by a key function.
#[cfg_attr(not(any(feature = "v1_8", feature = "v1_16_5")), allow(dead_code))]
pub(crate) fn lookup_id<T, K: Ord>(
    items: &'static [T],
    key: K,
    f: impl Fn(&T) -> K,
) -> Option<&'static T> {
    items
        .binary_search_by(|x| f(x).cmp(&key))
        .ok()
        .map(|i| &items[i])
}

/// Looks a tool's multiplier up in a generated `TOOL_MULTIPLIERS` table.
#[cfg_attr(not(any(feature = "v1_8", feature = "v1_16_5")), allow(dead_code))]
pub(crate) fn tool_multiplier(
    table: &[(DigMaterial, &[(u16, f32)])],
    material: DigMaterial,
    tool: u16,
) -> f32 {
    table
        .iter()
        .find(|(m, _)| *m == material)
        .and_then(|(_, tools)| tools.iter().find(|(t, _)| *t == tool))
        .map_or(1.0, |(_, x)| *x)
}

/// When a survival dig with a constant per-tick progress finishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakTime {
    /// Broken by the start packet itself (`START_DESTROY_BLOCK` / 1.8 `attackBlock`, progress ≥ 1
    /// in one tick).
    Instant,
    /// Broken on the `n`-th `continueDestroyBlock` call, counting the one in the start tick:
    /// the client sends `STOP_DESTROY_BLOCK` on that tick (`n` = 1 is the start tick itself).
    Ticks(u32),
    /// Never (progress 0: unbreakable, or a speed of 0).
    Never,
}

/// The tick the client finishes the block, accumulating `progress` in `f32` as
/// `MultiPlayerGameMode` does: the start tick runs `startDestroyBlock` and then
/// `continueDestroyBlock`, every later tick `continueDestroyBlock`. The first `destroy_delay`
/// continue calls only count the delay down (5 after the client finished a block, else 0).
/// `limit` bounds the search (vanilla has none).
#[must_use]
pub fn break_time(progress: f32, destroy_delay: u32, limit: u32) -> BreakTime {
    if progress >= 1.0 {
        return BreakTime::Instant;
    }
    if progress.is_nan() || progress <= 0.0 {
        return BreakTime::Never;
    }
    let mut acc = 0.0f32;
    for n in 1..=limit {
        if n <= destroy_delay {
            continue;
        }
        acc += progress;
        if acc >= 1.0 {
            return BreakTime::Ticks(n);
        }
    }
    BreakTime::Never
}

/// One row of a version's generated `ITEM_DIG` table: item id, `(block, speed multiplier)` sorted
/// by block, effective blocks sorted.
#[cfg(any(feature = "v1_8", feature = "v1_12", feature = "v1_16_5"))]
pub(crate) type ItemDig = (u16, &'static [(u16, f32)], &'static [u16]);
