//! What the planner needs to know about a block state, per protocol version.
//!
//! Every state of a version is reduced once (lazily, on first use) to a small [`Class`]: its
//! collision extent as seen by a player standing at the block centre, fluid and climbing flags,
//! hazards and floor properties. States with the same class share one entry, so the planner's
//! world cache stores a `u16` class index per block.

use std::sync::OnceLock;

use ghast_data::common::{Aabb, NeighbourShape, PhysicsFlags, ShapeDependency};

use crate::Version;

/// Class flag bits.
pub(crate) mod flag {
    /// Has collision boxes (blocks the body unless [`THIN`] and climbable).
    pub(crate) const SOLID: u32 = 1 << 0;
    /// A collision box covers the centre footprint of a standing player (it can carry one).
    pub(crate) const SUPPORT: u32 = 1 << 1;
    /// Collision boxes exist but none reaches the centre footprint (ladders, open trapdoors).
    pub(crate) const THIN: u32 = 1 << 2;
    /// Water (also waterlogged states in 1.16).
    pub(crate) const WATER: u32 = 1 << 3;
    /// Lava: never entered.
    pub(crate) const LAVA: u32 = 1 << 4;
    /// Climbable (ladder, vine; 1.16 also twisting / weeping vines).
    pub(crate) const CLIMB: u32 = 1 << 5;
    /// Never put the body in it (fire, cobweb, berry bush, portals, wither rose, ...).
    pub(crate) const AVOID: u32 = 1 << 6;
    /// Never stand on it (magma, campfire, cactus).
    pub(crate) const AVOID_FLOOR: u32 = 1 << 7;
    /// Slow floor (soul sand, honey): extra cost.
    pub(crate) const SLOW: u32 = 1 << 8;
    /// Slippery floor (ice): extra cost.
    pub(crate) const SLIPPERY: u32 = 1 << 9;
    /// Bouncy floor (slime): extra cost, never landed on from a fall.
    pub(crate) const BOUNCY: u32 = 1 << 10;
    /// Falls when unsupported (sand, gravel, concrete powder, anvils): never dug under.
    pub(crate) const FALLING: u32 = 1 << 11;
    /// Can be dug (has a hardness, diggable, not a fluid).
    pub(crate) const DIGGABLE: u32 = 1 << 12;
    /// Trampled or damaged by landing (farmland, turtle eggs): no falls onto it.
    pub(crate) const FRAGILE: u32 = 1 << 13;
    /// The shape depends on things the planner does not model (doors, scaffolding, moving
    /// pistons, bamboo offsets): treated as a solid obstacle that cannot carry the player.
    pub(crate) const IRREGULAR: u32 = 1 << 14;
    /// Air-like for placing: empty collision, no fluid (a block can be placed into it).
    pub(crate) const REPLACEABLE: u32 = 1 << 15;
}

/// The planner's view of one block state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Class {
    /// [`flag`] bits.
    pub(crate) flags: u32,
    /// Lowest collision bound in the block (block-local y), `1.0` when there is no collision.
    pub(crate) bottom: f32,
    /// Highest bound of the boxes covering the centre footprint (block-local y; may exceed 1
    /// for fences and walls); `0.0` without [`flag::SUPPORT`].
    pub(crate) top: f32,
    /// Highest bound of any box (for clearance above fences next to the path).
    pub(crate) top_any: f32,
    /// Dig hardness (vanilla strength); negative when not diggable.
    pub(crate) hardness: f32,
    /// Side a thin climbable box sits on: 0 none, 1 north (-z), 2 south (+z), 3 west (-x),
    /// 4 east (+x).
    pub(crate) wall: u8,
}

impl Class {
    pub(crate) const UNKNOWN: Class = Class {
        flags: flag::SOLID | flag::AVOID,
        bottom: 0.0,
        top: 0.0,
        top_any: 1.0,
        hardness: -1.0,
        wall: 0,
    };

    #[inline]
    pub(crate) fn has(&self, f: u32) -> bool {
        self.flags & f != 0
    }

    /// Blocks the body of a player passing through the cell between `lo` and `hi` (cell-local
    /// heights). Climbable thin boxes (ladders) do not.
    #[inline]
    pub(crate) fn blocks_between(&self, lo: f32, hi: f32) -> bool {
        if !self.has(flag::SOLID) {
            return false;
        }
        if self.has(flag::THIN) && self.has(flag::CLIMB) {
            return false;
        }
        // Collision occupies [bottom, top_any] of the cell.
        self.top_any > lo + 1e-4 && self.bottom < hi - 1e-4
    }
}

/// Per-version table: state → class index, plus the distinct classes.
pub(crate) struct Table {
    pub(crate) classes: Vec<Class>,
    pub(crate) map: Vec<u16>,
}

impl Table {
    /// The class index of a raw state (out-of-range values read as air, like vanilla).
    #[inline]
    pub(crate) fn index(&self, raw: u32) -> u16 {
        self.map.get(raw as usize).copied().unwrap_or(0)
    }
}

/// The table of a version (built on first use: ~1 ms).
pub(crate) fn table(version: Version) -> &'static Table {
    static T18: OnceLock<Table> = OnceLock::new();
    static T16: OnceLock<Table> = OnceLock::new();
    match version {
        Version::V1_8 => T18.get_or_init(build_1_8),
        Version::V1_16_5 => T16.get_or_init(build_1_16_5),
    }
}

/// Footprint of a player standing at the block centre (0.3 either side), slightly shrunk so a
/// box that only touches it does not count.
const CORE_MIN: f64 = 0.2 + 1e-3;
const CORE_MAX: f64 = 0.8 - 1e-3;

struct Facts<'a> {
    name: &'a str,
    boxes: &'a [Aabb],
    dependency: ShapeDependency,
    physics: PhysicsFlags,
    slipperiness: f32,
    speed_factor: f64,
    fluid_water: bool,
    fluid_lava: bool,
    hardness: Option<f32>,
    diggable: bool,
}

fn classify(f: &Facts<'_>) -> Class {
    use flag::*;
    let mut flags = 0u32;
    let mut bottom = 1.0f32;
    let mut top = 0.0f32;
    let mut top_any = 0.0f32;
    let mut core = false;
    for b in f.boxes {
        bottom = bottom.min(b.min_y as f32);
        top_any = top_any.max(b.max_y as f32);
        if b.min_x < CORE_MAX && b.max_x > CORE_MIN && b.min_z < CORE_MAX && b.max_z > CORE_MIN {
            core = true;
            top = top.max(b.max_y as f32);
        }
    }
    let mut wall = 0u8;
    if !f.boxes.is_empty() {
        flags |= SOLID;
        if core {
            flags |= SUPPORT;
        } else {
            flags |= THIN;
            // Which side the thin shape hugs (ladders).
            let b = f.boxes[0];
            wall = if b.max_z <= 0.25 {
                1
            } else if b.min_z >= 0.75 {
                2
            } else if b.max_x <= 0.25 {
                3
            } else if b.min_x >= 0.75 {
                4
            } else {
                0
            };
        }
    } else {
        bottom = 1.0;
    }
    match f.dependency {
        ShapeDependency::None | ShapeDependency::Neighbours(NeighbourShape::Chest) => {}
        ShapeDependency::Neighbours(NeighbourShape::Fence | NeighbourShape::Wall) => {
            // Isolated post; arms only grow inside the cell. Height 1.5 either way.
        }
        ShapeDependency::Neighbours(NeighbourShape::Pane | NeighbourShape::Stairs) => {
            // Panes: thin arms; stairs: corner shapes. Both still cover part of the core in
            // every configuration except a pane's single arm, which the planner never stands on.
            if f.dependency == ShapeDependency::Neighbours(NeighbourShape::Pane) {
                flags &= !SUPPORT;
                flags |= IRREGULAR;
            }
        }
        ShapeDependency::Neighbours(NeighbourShape::Door | NeighbourShape::ChorusPlant)
        | ShapeDependency::EntityContext
        | ShapeDependency::BlockEntity
        | ShapeDependency::PositionOffset => {
            flags |= SOLID | IRREGULAR;
            flags &= !(SUPPORT | THIN);
            bottom = 0.0;
            top_any = top_any.max(1.0);
        }
    }
    if flags & IRREGULAR != 0 {
        bottom = 0.0;
        top_any = top_any.max(1.0);
    }
    if f.fluid_water {
        flags |= WATER;
    }
    if f.fluid_lava {
        flags |= LAVA;
    }
    if f.physics.contains(PhysicsFlags::CLIMBABLE) {
        flags |= CLIMB;
    }
    if f.physics.contains(PhysicsFlags::STUCK)
        || f.physics.contains(PhysicsFlags::FIRE)
        || f.physics.contains(PhysicsFlags::BUBBLE_COLUMN)
    {
        flags |= AVOID;
    }
    if f.physics.contains(PhysicsFlags::SLIME) {
        flags |= BOUNCY;
    }
    if f.physics.contains(PhysicsFlags::COLLIDE_SLOWDOWN)
        || f.physics.contains(PhysicsFlags::HONEY)
        || f.speed_factor < 0.99
    {
        flags |= SLOW;
    }
    if f.slipperiness > 0.7 {
        flags |= SLIPPERY;
    }
    match f.name {
        "fire" | "soul_fire" | "web" | "cobweb" | "sweet_berry_bush" | "wither_rose" | "portal"
        | "nether_portal" | "end_portal" | "end_gateway" | "scaffolding" | "powder_snow"
        | "tripwire" | "bubble_column" => flags |= AVOID,
        "magma_block" | "campfire" | "soul_campfire" | "cactus" => flags |= AVOID_FLOOR | AVOID,
        "farmland" | "turtle_egg" => flags |= FRAGILE,
        "sand" | "red_sand" | "gravel" | "anvil" | "chipped_anvil" | "damaged_anvil"
        | "dragon_egg" => flags |= FALLING,
        n if n.ends_with("concrete_powder") => flags |= FALLING,
        _ => {}
    }
    if f.diggable
        && !f.fluid_water
        && !f.fluid_lava
        && f.hardness.is_some_and(|h| h >= 0.0)
        && flags & (IRREGULAR | AVOID) == 0
    {
        flags |= DIGGABLE;
    }
    if f.boxes.is_empty() && !f.fluid_water && !f.fluid_lava && flags & (AVOID | CLIMB) == 0 {
        flags |= REPLACEABLE;
    }
    if flags & SUPPORT == 0 {
        top = 0.0;
    }
    Class {
        flags,
        bottom,
        top,
        top_any,
        hardness: if flags & DIGGABLE != 0 {
            f.hardness.unwrap_or(-1.0)
        } else {
            -1.0
        },
        wall,
    }
}

fn intern(classes: &mut Vec<Class>, c: Class) -> u16 {
    if let Some(i) = classes.iter().position(|e| *e == c) {
        return i as u16;
    }
    classes.push(c);
    (classes.len() - 1) as u16
}

fn build_1_8() -> Table {
    use ghast_data::v1_8::BlockState;
    let mut classes = Vec::new();
    let mut map = vec![0u16; 1 << 12];
    // Index 0 must be air.
    let air = classify_1_8(BlockState::AIR);
    intern(&mut classes, air);
    for (raw, slot) in map.iter_mut().enumerate() {
        let s = BlockState::from_raw_vanilla(raw as u16);
        *slot = intern(&mut classes, classify_1_8(s));
    }
    Table { classes, map }
}

fn classify_1_8(s: ghast_data::v1_8::BlockState) -> Class {
    let block = s.block();
    let info = block.info();
    let physics = block.physics();
    let collision = s.collision();
    let fluid = s.fluid();
    classify(&Facts {
        name: info.name,
        boxes: collision.boxes,
        dependency: collision.dependency,
        physics: physics.flags,
        slipperiness: physics.slipperiness,
        speed_factor: physics.speed_factor,
        fluid_water: fluid.is_some_and(|f| f.kind == ghast_data::common::FluidKind::Water),
        fluid_lava: fluid.is_some_and(|f| f.kind == ghast_data::common::FluidKind::Lava),
        hardness: info.hardness,
        diggable: info.diggable,
    })
}

fn build_1_16_5() -> Table {
    use ghast_data::v1_16_5::{BlockState, STATE_COUNT};
    let mut classes = Vec::new();
    let mut map = vec![0u16; STATE_COUNT];
    intern(&mut classes, classify_1_16_5(BlockState::AIR));
    for (id, slot) in map.iter_mut().enumerate() {
        let s = BlockState::from_id(id as u16).unwrap_or(BlockState::AIR);
        *slot = intern(&mut classes, classify_1_16_5(s));
    }
    Table { classes, map }
}

fn classify_1_16_5(s: ghast_data::v1_16_5::BlockState) -> Class {
    let block = s.block();
    let info = block.info();
    let physics = block.physics();
    let collision = s.collision();
    let fluid = s.fluid();
    classify(&Facts {
        name: info.name,
        boxes: collision.boxes,
        dependency: collision.dependency,
        physics: physics.flags,
        slipperiness: physics.slipperiness,
        speed_factor: physics.speed_factor,
        fluid_water: fluid.is_some_and(|f| f.kind == ghast_data::common::FluidKind::Water),
        fluid_lava: fluid.is_some_and(|f| f.kind == ghast_data::common::FluidKind::Lava),
        hardness: info.hardness,
        diggable: info.diggable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class_1_8(id: u8, meta: u8) -> Class {
        let t = table(Version::V1_8);
        t.classes[t.index(u32::from(id) << 4 | u32::from(meta)) as usize]
    }

    #[test]
    fn basic_1_8() {
        let air = class_1_8(0, 0);
        assert_eq!(air.flags & flag::SOLID, 0);
        assert!(air.has(flag::REPLACEABLE));
        let stone = class_1_8(1, 0);
        assert!(stone.has(flag::SOLID | flag::SUPPORT));
        assert!(stone.has(flag::DIGGABLE));
        assert_eq!(stone.top, 1.0);
        let slab = class_1_8(44, 0);
        assert_eq!(slab.top, 0.5);
        let top_slab = class_1_8(44, 8);
        assert_eq!(top_slab.bottom, 0.5);
        let soul = class_1_8(88, 0);
        assert_eq!(soul.top, 0.875);
        assert!(soul.has(flag::SLOW));
        let fence = class_1_8(85, 0);
        assert_eq!(fence.top, 1.5);
        let ladder = class_1_8(65, 2);
        assert!(ladder.has(flag::THIN | flag::CLIMB));
        assert_eq!(ladder.wall, 2);
        assert!(!ladder.blocks_between(0.0, 1.0));
        let water = class_1_8(9, 0);
        assert!(water.has(flag::WATER));
        let lava = class_1_8(11, 0);
        assert!(lava.has(flag::LAVA));
        assert!(class_1_8(30, 0).has(flag::AVOID)); // web
        assert!(class_1_8(81, 0).has(flag::AVOID_FLOOR)); // cactus
        assert!(class_1_8(12, 0).has(flag::FALLING)); // sand
        assert!(class_1_8(64, 0).has(flag::IRREGULAR)); // door
        assert!(!class_1_8(7, 0).has(flag::DIGGABLE)); // bedrock
    }

    #[test]
    fn basic_1_16_5() {
        use ghast_data::v1_16_5::Block;
        let t = table(Version::V1_16_5);
        let c = |b: Block| t.classes[t.index(u32::from(b.default_state().id())) as usize];
        assert!(c(Block::STONE).has(flag::SUPPORT));
        assert!(c(Block::WATER).has(flag::WATER));
        assert!(c(Block::LADDER).has(flag::CLIMB));
        assert!(c(Block::MAGMA_BLOCK).has(flag::AVOID_FLOOR));
        assert!(c(Block::COBWEB).has(flag::AVOID));
        assert!(c(Block::SOUL_SAND).has(flag::SLOW));
        assert!(c(Block::OAK_FENCE).top >= 1.5);
        assert_eq!(c(Block::AIR).flags & flag::SOLID, 0);
        assert!(t.classes.len() < u16::MAX as usize);
    }
}
