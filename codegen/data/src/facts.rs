//! Hand-maintained vanilla facts that minecraft-data does not carry (physics properties, shape
//! dependencies). Constants are facts about the game's behaviour, written down by us.
//!
//! Status: these values are from our reading of vanilla behaviour and earlier physics ports
//! verified against recorded traces; they become *verified* only once oracle golden vectors cover them
//! (`tests/golden/<version>/`). Every name or pattern must match at least one block of the version,
//! otherwise codegen fails (catches typos and renamed blocks).

/// Shape dependency classes, mirrored as text into the generated code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dep {
    Neighbours(&'static str),
    EntityContext,
    BlockEntity,
    PositionOffset,
}

impl Dep {
    pub fn rust(self) -> String {
        match self {
            Dep::Neighbours(k) => format!("ShapeDependency::Neighbours(NeighbourShape::{k})"),
            Dep::EntityContext => "ShapeDependency::EntityContext".into(),
            Dep::BlockEntity => "ShapeDependency::BlockEntity".into(),
            Dep::PositionOffset => "ShapeDependency::PositionOffset".into(),
        }
    }
}

// Flag bits, must match `ghast_data::PhysicsFlags`.
pub const CLIMBABLE: u16 = 1 << 0;
pub const WATER: u16 = 1 << 1;
pub const LAVA: u16 = 1 << 2;
pub const STUCK: u16 = 1 << 3;
pub const SLIME: u16 = 1 << 4;
pub const BED_BOUNCE: u16 = 1 << 5;
pub const HONEY: u16 = 1 << 6;
pub const BUBBLE_COLUMN: u16 = 1 << 7;
pub const COLLIDE_SLOWDOWN: u16 = 1 << 8;
pub const FENCE: u16 = 1 << 9;
pub const WALL: u16 = 1 << 10;
pub const FENCE_GATE: u16 = 1 << 11;
pub const TRAPDOOR: u16 = 1 << 12;
pub const SOUL_SPEED: u16 = 1 << 13;
pub const FIRE: u16 = 1 << 14;

/// Per-version facts. Patterns: `"name"` exact, `"*_suffix"`, `"prefix_*"`.
pub struct Facts {
    pub slipperiness: &'static [(&'static str, f32)],
    pub speed_factor: &'static [(&'static str, f64)],
    pub jump_factor: &'static [(&'static str, f32)],
    pub stuck: &'static [(&'static str, [f64; 3])],
    pub flags: &'static [(&'static str, u16)],
    pub dependency: &'static [(&'static str, Dep)],
    /// Patterns excluded from `dependency` matches (e.g. `*_fence` must not hit `*_fence_gate`).
    pub dependency_exclude: &'static [&'static str],
}

pub fn matches(pattern: &str, name: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix('*') {
        name.ends_with(suffix)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        pattern == name
    }
}

/// `(double) 0.05F`, the y factor of vanilla's cobweb slowdown in both versions.
const F32_0_05: f64 = 0.05_f32 as f64;

/// 1.8.9 (protocol 47).
pub const V1_8: Facts = Facts {
    // Block.slipperiness defaults to 0.6F.
    slipperiness: &[("ice", 0.98), ("packed_ice", 0.98), ("slime", 0.8)],
    // Soul sand: motionX/Z *= 0.4D when the entity box touches the block.
    speed_factor: &[("soul_sand", 0.4)],
    jump_factor: &[],
    // Entity.moveEntity while in web: x *= 0.25D, y *= 0.05000000074505806D, z *= 0.25D.
    stuck: &[("web", [0.25, F32_0_05, 0.25])],
    flags: &[
        // EntityLivingBase.isOnLadder: block == ladder || block == vine.
        ("ladder", CLIMBABLE),
        ("vine", CLIMBABLE),
        ("water", WATER),
        ("flowing_water", WATER),
        ("lava", LAVA),
        ("flowing_lava", LAVA),
        ("web", STUCK),
        ("slime", SLIME),
        ("soul_sand", COLLIDE_SLOWDOWN),
        // Block families vanilla 1.8 checks with `instanceof` (FenceBlock, WallBlock, ...).
        ("fence", FENCE),
        ("*_fence", FENCE),
        ("cobblestone_wall", WALL),
        ("fence_gate", FENCE_GATE),
        ("*_fence_gate", FENCE_GATE),
        ("trapdoor", TRAPDOOR),
        ("iron_trapdoor", TRAPDOOR),
    ],
    // 1.8 blocks whose collision comes from getActualState / neighbour checks.
    dependency: &[
        ("fence", Dep::Neighbours("Fence")),
        ("*_fence", Dep::Neighbours("Fence")),
        ("glass_pane", Dep::Neighbours("Pane")),
        ("stained_glass_pane", Dep::Neighbours("Pane")),
        ("iron_bars", Dep::Neighbours("Pane")),
        ("cobblestone_wall", Dep::Neighbours("Wall")),
        ("*_stairs", Dep::Neighbours("Stairs")),
        ("*_door", Dep::Neighbours("Door")),
        // Oracle `settled` configs: a chest next to a chest of the same kind extends to that edge
        // (ghast models the settled shape, see oracle/README.md "History-dependent shapes").
        ("chest", Dep::Neighbours("Chest")),
        ("trapped_chest", Dep::Neighbours("Chest")),
        // Moving block: the box comes from the piston block entity (pushed block, progress).
        ("piston_extension", Dep::BlockEntity),
        // Anvil: vanilla collision uses the bounds of the last anvil shape update anywhere (fresh
        // client: full cube). Policy (oracle/README.md "History-dependent shapes"): model the
        // settled shape, which depends only on the meta, so the anvil stays a static table entry.
    ],
    dependency_exclude: &["*_fence_gate"],
};

/// 1.16.5 (protocol 754).
pub const V1_16_5: Facts = Facts {
    slipperiness: &[
        ("ice", 0.98),
        ("packed_ice", 0.98),
        ("frosted_ice", 0.98),
        ("blue_ice", 0.989),
        ("slime_block", 0.8),
    ],
    // Block speedFactor(0.4F): applied as (double) of the float.
    speed_factor: &[
        ("soul_sand", 0.4_f32 as f64),
        ("honey_block", 0.4_f32 as f64),
    ],
    jump_factor: &[("honey_block", 0.5)],
    // makeStuckInBlock(state, new Vec3(...)).
    stuck: &[
        ("cobweb", [0.25, F32_0_05, 0.25]),
        ("sweet_berry_bush", [0.8_f32 as f64, 0.75, 0.8_f32 as f64]),
    ],
    flags: &[
        // Block tag minecraft:climbable (1.16).
        ("ladder", CLIMBABLE),
        ("vine", CLIMBABLE),
        ("scaffolding", CLIMBABLE),
        ("weeping_vines", CLIMBABLE),
        ("weeping_vines_plant", CLIMBABLE),
        ("twisting_vines", CLIMBABLE),
        ("twisting_vines_plant", CLIMBABLE),
        ("water", WATER),
        ("bubble_column", WATER | BUBBLE_COLUMN),
        ("lava", LAVA),
        ("cobweb", STUCK),
        ("sweet_berry_bush", STUCK),
        ("slime_block", SLIME),
        ("*_bed", BED_BOUNCE),
        ("honey_block", HONEY),
        // Vanilla 1.16.5 block tags (data/minecraft/tags/blocks), by name for now; to be checked
        // against the oracle's 1.16.5 dump. fences = wooden_fences + nether_brick_fence; walls,
        // fence_gates, trapdoors (wooden_trapdoors + iron_trapdoor); soul_speed_blocks.
        ("*_fence", FENCE),
        ("*_wall", WALL),
        ("*_fence_gate", FENCE_GATE),
        ("*_trapdoor", TRAPDOOR),
        ("soul_sand", SOUL_SPEED),
        ("soul_soil", SOUL_SPEED),
        // #minecraft:fire
        ("fire", FIRE),
        ("soul_fire", FIRE),
    ],
    // In 1.16 neighbour connections are part of the block state; only these remain non-static.
    dependency: &[
        ("scaffolding", Dep::EntityContext),
        ("shulker_box", Dep::BlockEntity),
        ("*_shulker_box", Dep::BlockEntity),
        ("moving_piston", Dep::BlockEntity),
        ("bamboo", Dep::PositionOffset),
    ],
    dependency_exclude: &[],
};

/// Blocks whose every state is a water source in 1.16 (besides `water` itself, whose level comes
/// from its `level` property, and waterlogged states).
pub const V1_16_ALWAYS_WATER: &[&str] = &[
    "bubble_column",
    "kelp",
    "kelp_plant",
    "seagrass",
    "tall_seagrass",
];

/// Replaces minecraft-data's 1.8 collision boxes for some (block, meta) pairs. Every entry is
/// ground truth from the vanilla 1.8.9 oracle, `tests/golden/1.8.9/collision_boxes.jsonl.gz`
/// (`"config":"alone"` lines, i.e. the block with all neighbours air). For neighbour-dependent
/// blocks this sets the isolated shape that `Collision::boxes` documents.
pub struct ShapeOverride {
    pub block: &'static str,
    pub metas: &'static [u8],
    pub boxes: &'static [[f64; 6]],
    pub why: &'static str,
}

const FULL: [f64; 6] = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
const PANE_CROSS: &[[f64; 6]] = &[
    [0.0, 0.0, 0.4375, 1.0, 1.0, 0.5625],
    [0.4375, 0.0, 0.0, 0.5625, 1.0, 1.0],
];
const ALL_METAS: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

macro_rules! door_upper {
    ($name:literal) => {
        ShapeOverride {
            block: $name,
            metas: &[9, 10, 11],
            boxes: &[[0.0, 0.0, 0.0, 0.1875, 1.0, 1.0]],
            why: "oracle: an isolated upper door half (no lower half) uses the default facing",
        }
    };
}

pub const V1_8_SHAPE_OVERRIDES: &[ShapeOverride] = &[
    ShapeOverride {
        block: "farmland",
        metas: ALL_METAS,
        boxes: &[FULL],
        why: "oracle: 1.8 farmland collides as a full cube (minecraft-data: 15/16)",
    },
    ShapeOverride {
        block: "ladder",
        metas: &[2],
        boxes: &[[0.0, 0.0, 0.875, 1.0, 1.0, 1.0]],
        why: "oracle: 1.8 ladders are 2/16 thick (minecraft-data: 3/16, the 1.9+ value)",
    },
    ShapeOverride {
        block: "ladder",
        metas: &[3],
        boxes: &[[0.0, 0.0, 0.0, 1.0, 1.0, 0.125]],
        why: "oracle: 1.8 ladders are 2/16 thick",
    },
    ShapeOverride {
        block: "ladder",
        metas: &[4],
        boxes: &[[0.875, 0.0, 0.0, 1.0, 1.0, 1.0]],
        why: "oracle: 1.8 ladders are 2/16 thick",
    },
    ShapeOverride {
        block: "ladder",
        metas: &[5],
        boxes: &[[0.0, 0.0, 0.0, 0.125, 1.0, 1.0]],
        why: "oracle: 1.8 ladders are 2/16 thick",
    },
    ShapeOverride {
        block: "snow_layer",
        metas: &[0],
        boxes: &[[0.0, 0.0, 0.0, 1.0, 0.0, 1.0]],
        why: "oracle: one snow layer has a zero-height box at y = 0 (minecraft-data: none)",
    },
    ShapeOverride {
        block: "waterlily",
        metas: &[0],
        boxes: &[[0.0, 0.0, 0.0, 1.0, 0.015625, 1.0]],
        why: "oracle: 1.8 lily pad is full width, 1/64 high (minecraft-data: 1.9+ shape)",
    },
    ShapeOverride {
        block: "piston_head",
        metas: &[0, 8],
        boxes: &[
            [0.0, 0.0, 0.0, 1.0, 0.25, 1.0],
            [0.375, 0.25, 0.375, 0.625, 1.0, 0.625],
        ],
        why: "oracle: 1.8 piston arm ends at the block edge (minecraft-data: extends 0.25 out)",
    },
    ShapeOverride {
        block: "piston_head",
        metas: &[1, 9],
        boxes: &[
            [0.0, 0.75, 0.0, 1.0, 1.0, 1.0],
            [0.375, 0.0, 0.375, 0.625, 0.75, 0.625],
        ],
        why: "oracle: 1.8 piston arm",
    },
    ShapeOverride {
        block: "piston_head",
        metas: &[2, 10],
        boxes: &[
            [0.0, 0.0, 0.0, 1.0, 1.0, 0.25],
            [0.25, 0.375, 0.25, 0.75, 0.625, 1.0],
        ],
        why: "oracle: 1.8 piston arm (horizontal arms are 0.5 wide)",
    },
    ShapeOverride {
        block: "piston_head",
        metas: &[3, 11],
        boxes: &[
            [0.0, 0.0, 0.75, 1.0, 1.0, 1.0],
            [0.25, 0.375, 0.0, 0.75, 0.625, 0.75],
        ],
        why: "oracle: 1.8 piston arm",
    },
    ShapeOverride {
        block: "piston_head",
        metas: &[4, 12],
        boxes: &[
            [0.0, 0.0, 0.0, 0.25, 1.0, 1.0],
            [0.375, 0.25, 0.25, 0.625, 0.75, 1.0],
        ],
        why: "oracle: 1.8 piston arm",
    },
    ShapeOverride {
        block: "piston_head",
        metas: &[5, 13],
        boxes: &[
            [0.75, 0.0, 0.0, 1.0, 1.0, 1.0],
            [0.0, 0.375, 0.25, 0.75, 0.625, 0.75],
        ],
        why: "oracle: 1.8 piston arm",
    },
    ShapeOverride {
        block: "glass_pane",
        metas: ALL_METAS,
        boxes: PANE_CROSS,
        why: "oracle: an isolated 1.8 pane collides as a full-length cross",
    },
    ShapeOverride {
        block: "stained_glass_pane",
        metas: ALL_METAS,
        boxes: PANE_CROSS,
        why: "oracle: an isolated 1.8 pane collides as a full-length cross",
    },
    ShapeOverride {
        block: "iron_bars",
        metas: ALL_METAS,
        boxes: PANE_CROSS,
        why: "oracle: isolated iron bars collide as a full-length cross",
    },
    door_upper!("wooden_door"),
    door_upper!("iron_door"),
    door_upper!("spruce_door"),
    door_upper!("birch_door"),
    door_upper!("jungle_door"),
    door_upper!("acacia_door"),
    door_upper!("dark_oak_door"),
];

/// Oracle file that also supplies the registered (id, meta) set: a raw value whose `alone` line has
/// `"state": null` decodes to air in vanilla 1.8.9.
pub const V1_8_ORACLE_BOXES: &str = "tests/golden/1.8.9/collision_boxes.jsonl.gz";

/// One world-coordinate rule: `D` = `pos + local` in double, `F(k, t)` = `(float)(pos + k) + t`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wc {
    D,
    F(i8, f32),
}

/// 1.8 blocks whose `getCollisionBox` builds the box with `float` arithmetic on the block
/// position (found by scanning every block class's `getCollisionBox` / `appendCollisionBoxes`
/// bytecode for `fadd`/`fsub` on `i2f` of the position; the other float hits — cauldron and
/// hopper `1 - f` on local bounds, the moving piston's progress — are not world-coordinate
/// arithmetic). Codegen checks every rule against the table box at the origin.
pub struct FloatCoords {
    pub block: &'static str,
    pub metas: &'static [u8],
    pub boxes: &'static [[Wc; 6]],
}

use Wc::{D, F};
const S: f32 = 0.0625;

macro_rules! cake {
    ($($m:literal),*) => {
        &[$(FloatCoords {
            block: "cake",
            metas: &[$m],
            // (float)x + (1 + 2 * bites) / 16F, y, (float)z + 1/16F, (float)(x + 1) - 1/16F,
            // (float)y + 0.5F, (float)(z + 1) - 1/16F
            boxes: &[[F(0, (1 + 2 * $m) as f32 / 16.0), D, F(0, S), F(1, -S), F(0, 0.5), F(1, -S)]],
        }),*]
    };
}

pub const V1_8_CAKE_FLOAT_COORDS: &[FloatCoords] = cake!(0, 1, 2, 3, 4, 5, 6);

pub const V1_8_FLOAT_COORDS: &[FloatCoords] = &[
    FloatCoords {
        block: "cactus",
        metas: ALL_METAS,
        // (float)x + f, y, (float)z + f, (float)(x + 1) - f, (float)(y + 1) - f, (float)(z + 1) - f
        boxes: &[[F(0, S), D, F(0, S), F(1, -S), F(1, -S), F(1, -S)]],
    },
    FloatCoords {
        block: "soul_sand",
        metas: &[0],
        // maxY = (float)(y + 1) - 0.125F
        boxes: &[[D, D, D, D, F(1, -0.125), D]],
    },
    // Snow layer: maxY = (float)y + (layers - 1) * 0.125F, layers = meta + 1.
    FloatCoords {
        block: "snow_layer",
        metas: &[0],
        boxes: &[[D, D, D, D, F(0, 0.0), D]],
    },
    FloatCoords {
        block: "snow_layer",
        metas: &[1],
        boxes: &[[D, D, D, D, F(0, 0.125), D]],
    },
    FloatCoords {
        block: "snow_layer",
        metas: &[2],
        boxes: &[[D, D, D, D, F(0, 0.25), D]],
    },
    FloatCoords {
        block: "snow_layer",
        metas: &[3],
        boxes: &[[D, D, D, D, F(0, 0.375), D]],
    },
    FloatCoords {
        block: "snow_layer",
        metas: &[4],
        boxes: &[[D, D, D, D, F(0, 0.5), D]],
    },
    FloatCoords {
        block: "snow_layer",
        metas: &[5],
        boxes: &[[D, D, D, D, F(0, 0.625), D]],
    },
    FloatCoords {
        block: "snow_layer",
        metas: &[6],
        boxes: &[[D, D, D, D, F(0, 0.75), D]],
    },
    FloatCoords {
        block: "snow_layer",
        metas: &[7],
        boxes: &[[D, D, D, D, F(0, 0.875), D]],
    },
];

/// Closed fence gates: facing axis Z (metas with even facing: south/north) gives
/// `x, y, (float)z + 0.375F, x + 1, (float)y + 1.5F, (float)z + 0.625F`; axis X gives
/// `(float)x + 0.375F, y, z, (float)x + 0.625F, (float)y + 1.5F, z + 1`. Open gates have no box.
pub const V1_8_FENCE_GATES: &[&str] = &[
    "fence_gate",
    "spruce_fence_gate",
    "birch_fence_gate",
    "jungle_fence_gate",
    "dark_oak_fence_gate",
    "acacia_fence_gate",
];
pub const FENCE_GATE_Z: [Wc; 6] = [D, D, F(0, 0.375), D, F(0, 1.5), F(0, 0.625)];
pub const FENCE_GATE_X: [Wc; 6] = [F(0, 0.375), D, D, F(0, 0.625), F(0, 1.5), D];

/// 1.8.9 material identity for the materials vanilla compares by identity (`== Material.AIR` etc.),
/// from the block registration in vanilla `Block` (air: `AirBlock`; ice: `IceBlock`, while packed ice
/// has its own `PACKED_ICE` material). All other blocks are `Other`.
pub const V1_8_MATERIAL_KINDS: &[(&str, &str)] = &[
    ("air", "Air"),
    ("flowing_water", "Water"),
    ("water", "Water"),
    ("flowing_lava", "Lava"),
    ("lava", "Lava"),
    ("ice", "Ice"),
];

/// Item fixes where minecraft-data disagrees with the vanilla jar (`Item.getMaxCount` /
/// registration), each verified against the decompiled Yarn source and an oracle
/// jar dump (`Item.getMaxCount`, `getMaxDamage` for every item).
pub struct ItemFix {
    /// Item name; `id` / `display_name` are only used when the item is missing (then it is added).
    pub name: &'static str,
    pub id: u16,
    pub display_name: &'static str,
    pub stack_size: u8,
    pub why: &'static str,
}

pub const V1_8_ITEM_FIXES: &[ItemFix] = &[ItemFix {
    name: "lit_furnace",
    id: 62,
    display_name: "Burning Furnace",
    stack_size: 64,
    why: "vanilla 1.8.9 registers a block item for lit_furnace (id 62); minecraft-data omits it",
}];

pub const V1_16_5_ITEM_FIXES: &[ItemFix] = &[ItemFix {
    name: "warped_fungus_on_a_stick",
    id: 843,
    display_name: "Warped Fungus on a Stick",
    stack_size: 1,
    why: "vanilla: Item.Settings().maxDamage(100) forces maxCount 1; minecraft-data says 64",
}];

/// 1.16.5 oracle shape dump (per-state flags, fluids, scaffolding contexts; `oracle/README.md`).
pub const V1_16_5_ORACLE_SHAPES: &str = "tests/golden/1.16.5/collision_shapes.jsonl.gz";

/// 1.16.5 oracle voxel dump (VoxelShape grids, per-state material / solidity facts, tags).
pub const V1_16_5_ORACLE_VOXELS: &str = "tests/golden/1.16.5/voxel_shapes.jsonl.gz";

/// 1.8.9 oracle selection dump (ray-test bounds, canCollide flags).
pub const V1_8_ORACLE_SELECTION: &str = "tests/golden/1.8.9/selection_boxes.jsonl.gz";

/// 1.8.9 oracle crafting recipe dump (`RecipeDispatcher.getAllRecipes()` in match order).
pub const V1_8_ORACLE_RECIPES: &str = "tests/golden/1.8.9/recipes.jsonl.gz";

/// Oracle entity size dumps (`oracle/README.md`, "entity_sizes.jsonl.gz").
pub const V1_8_ORACLE_ENTITIES: &str = "tests/golden/1.8.9/entity_sizes.jsonl.gz";
pub const V1_16_5_ORACLE_ENTITIES: &str = "tests/golden/1.16.5/entity_sizes.jsonl.gz";

/// Entity default sizes where minecraft-data differs from vanilla, `(minecraft-data name, width,
/// height)` as vanilla `float`s. Every entry is the oracle's measured default (codegen checks it
/// against the dump, and fails if any other entity still differs).
pub const V1_8_ENTITY_SIZE_FIXES: &[(&str, f32, f32)] = &[
    // minecraft-data lists the wither-skeleton height for the base type.
    ("Skeleton", 0.6, 1.8),
    // vanilla computes 0.6F * 6.0F and 1.8F * 6.0F in float.
    ("Giant", 3.600_000_1, 10.799_999),
    // Zombies, zombie pigmen and witches are 1.95 high in 1.8.
    ("Zombie", 0.6, 1.95),
    ("PigZombie", 0.6, 1.95),
    ("Witch", 0.6, 1.95),
    // Slimes: the constructor size; the real size comes from metadata (`slime_dimensions`).
    ("Slime", 0.6, 1.8),
    ("LavaSlime", 0.6, 1.8),
    ("Ozelot", 0.6, 0.7),
    // Armor stand: 1.975 high (minecraft-data rounds to 2).
    ("ArmorStand", 0.5, 1.975),
    // Item frame: minecraft-data has no size (the box comes from the facing; width/height 0.5).
    ("ItemFrame", 0.5, 0.5),
];

/// 1.16.5: minecraft-data rounds the horse family's `1.3964844F` width to 1.39648.
pub const V1_16_5_ENTITY_SIZE_FIXES: &[(&str, f32, f32)] = &[
    ("donkey", 1.396_484_4, 1.5),
    ("hoglin", 1.396_484_4, 1.4),
    ("horse", 1.396_484_4, 1.6),
    ("mule", 1.396_484_4, 1.6),
    ("skeleton_horse", 1.396_484_4, 1.6),
    ("zoglin", 1.396_484_4, 1.4),
    ("zombie_horse", 1.396_484_4, 1.6),
];
