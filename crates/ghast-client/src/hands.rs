//! The mouse buttons in the world: digging, placing and using items, ported from 1.8.9
//! `MinecraftClient.tick` (`doAttack`, `doUse`, `handleBlockBreaking`) and
//! `ClientPlayerInteractionManager` (`attackBlock`, `updateBlockBreakingProgress`,
//! `cancelBlockBreaking`, `onRightClick`, `interactItem`, `stopUsingItem`), verified against recorded
//! lab packet traces.
//!
//! The hands run once per tick after the behaviour and before the movement step (vanilla's
//! key processing comes after the tick-start event and before the player's update), so their
//! packets precede this tick's movement packet and the server judges them with the previous
//! tick's pose.
//!
//! Version-specific facts (block hardness, tool groups, selection boxes, the held stack) come
//! from the [`BlockQuery`] answers and [`HeldItem`]; the state machine is the 1.8 one for both
//! supported versions (1.9+ attack cooldowns and off-hand are not modelled yet).

use ghast_data::DigMaterial;
use ghast_protocol::BlockPos;

use crate::common::{DigStatus, Face, Outbound, Vec3};

/// Survival block reach (`ClientPlayerInteractionManager.getReachDistance`).
pub const REACH: f64 = 4.5;

/// What the hands need to know about one block state.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct BlockQuery {
    /// The block is air (`Material.AIR`).
    pub air: bool,
    /// The block is water (eyes in it slow digging).
    pub water: bool,
    /// Version block id (1.8 `id`, 1.16 registry id).
    pub block: u16,
    /// Hardness; negative = unbreakable.
    pub hardness: f32,
    /// Tool group for the speed multiplier.
    pub dig_material: Option<DigMaterial>,
    /// Item ids that harvest the block (empty = any).
    pub harvest_tools: &'static [u16],
    /// Selection (ray) box relative to the block, `None` when rays pass through.
    pub selection: Option<[f64; 6]>,
    /// Stairs (1.8): vanilla's eight-pass half-cube ray test; the passes' block-local boxes
    /// (`None` = skipped pass). When set, `selection` is ignored for rays.
    pub stairs: Option<[Option<[f64; 6]>; 8]>,
    /// Right-clicking the block activates it (containers, doors, levers, …).
    pub activates: bool,
    /// The version's oracle-verified destroy progress (`ghast_data::<version>::dig`); `None`
    /// uses the generic formula over `hardness` / `dig_material` / `harvest_tools`.
    pub(crate) exact: Option<ExactDig>,
}

/// Which jar-derived destroy-progress function digs the block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ExactDig {
    /// `ghast_data::v1_8::dig` (block id in [`BlockQuery::block`]).
    #[cfg(feature = "v1_8")]
    V1_8,
    /// `ghast_data::v1_12::dig`.
    #[cfg(feature = "v1_12_2")]
    V1_12,
    /// `ghast_data::v1_16_5::dig`.
    #[cfg(feature = "v1_16_5")]
    V1_16_5,
    /// `ghast_data::v26_1::dig` with the player's mining attributes.
    #[cfg(feature = "v26_1")]
    V26_1(ModernDig),
}

/// The facts of the modern destroy-progress formula (attributes replace enchantment levels).
#[cfg(ghast_modern)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ModernDig {
    /// Block state id.
    pub(crate) state: u16,
    /// `minecraft:mining_efficiency`.
    pub(crate) mining_efficiency: f64,
    /// `minecraft:block_break_speed`.
    pub(crate) block_break_speed: f64,
    /// `minecraft:submerged_mining_speed`.
    pub(crate) submerged_mining_speed: f64,
    /// Conduit power amplifier.
    pub(crate) conduit_power: Option<i32>,
}

/// The held stack as far as digging and using need it.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct HeldItem {
    /// Item id.
    pub id: u16,
    /// Stack size.
    pub count: u8,
    /// Efficiency enchantment level.
    pub efficiency: u8,
    /// The item places a block (`BlockItem`).
    pub places_block: bool,
    /// The item is used by holding right click (food, potion, bow, sword, milk).
    pub usable: bool,
    /// The item is food (eaten only below a full food bar).
    pub food: bool,
    /// Food that can be eaten at any food level (golden apple).
    pub always_edible: bool,
    /// Tool speed multiplier per dig group, filled by the version (`tool_multiplier`).
    pub tool_speed: Option<fn(DigMaterial, u16) -> f32>,
    /// 1.20.5+: the item id for the exact destroy progress (`ghast_data::v26_1`).
    #[cfg_attr(not(feature = "v26_1"), allow(dead_code))]
    pub(crate) modern_item: Option<u16>,
    /// Using the item succeeds on the client and swings the arm (26.1: a firework rocket while
    /// gliding, `FireworkRocketItem.use`).
    pub(crate) swing_on_use: bool,
}

impl BlockQuery {
    /// A solid, instantly breakable block with no tool group, no ray box and no activation;
    /// set the rest with the `with_*` methods.
    #[must_use]
    pub const fn new(block: u16) -> Self {
        BlockQuery {
            exact: None,
            air: false,
            water: false,
            block,
            hardness: 0.0,
            dig_material: None,
            harvest_tools: &[],
            selection: None,
            stairs: None,
            activates: false,
        }
    }

    /// Air (id 0, no ray box).
    pub const AIR: BlockQuery = BlockQuery::new(0).with_air(true);

    /// The block is air.
    #[must_use]
    pub const fn with_air(mut self, on: bool) -> Self {
        self.air = on;
        self
    }

    /// The block is water.
    #[must_use]
    pub const fn with_water(mut self, on: bool) -> Self {
        self.water = on;
        self
    }

    /// Hardness (negative: unbreakable).
    #[must_use]
    pub const fn with_hardness(mut self, hardness: f32) -> Self {
        self.hardness = hardness;
        self
    }

    /// Tool group.
    #[must_use]
    pub const fn with_dig_material(mut self, m: Option<DigMaterial>) -> Self {
        self.dig_material = m;
        self
    }

    /// Item ids that harvest the block (empty: any).
    #[must_use]
    pub const fn with_harvest_tools(mut self, tools: &'static [u16]) -> Self {
        self.harvest_tools = tools;
        self
    }

    /// Selection (ray) box relative to the block.
    #[must_use]
    pub const fn with_selection(mut self, b: Option<[f64; 6]>) -> Self {
        self.selection = b;
        self
    }

    /// 1.8 stairs ray passes.
    #[must_use]
    pub const fn with_stairs(mut self, passes: Option<[Option<[f64; 6]>; 8]>) -> Self {
        self.stairs = passes;
        self
    }

    /// Right-clicking activates it.
    #[must_use]
    pub const fn with_activates(mut self, on: bool) -> Self {
        self.activates = on;
        self
    }
}

impl HeldItem {
    /// One item of `id` with nothing else known; set the rest with the `with_*` methods.
    #[must_use]
    pub const fn new(id: u16) -> Self {
        HeldItem {
            modern_item: None,
            swing_on_use: false,
            id,
            count: 1,
            efficiency: 0,
            places_block: false,
            usable: false,
            food: false,
            always_edible: false,
            tool_speed: None,
        }
    }

    /// Stack size.
    #[must_use]
    pub const fn with_count(mut self, count: u8) -> Self {
        self.count = count;
        self
    }

    /// Efficiency level.
    #[must_use]
    pub const fn with_efficiency(mut self, level: u8) -> Self {
        self.efficiency = level;
        self
    }

    /// The item places a block.
    #[must_use]
    pub const fn with_places_block(mut self, on: bool) -> Self {
        self.places_block = on;
        self
    }

    /// The item is used by holding right click.
    #[must_use]
    pub const fn with_usable(mut self, on: bool) -> Self {
        self.usable = on;
        self
    }

    /// Food (and whether it can be eaten at a full food bar).
    #[must_use]
    pub const fn with_food(mut self, food: bool, always_edible: bool) -> Self {
        self.food = food;
        self.always_edible = always_edible;
        self
    }

    /// Tool speed per dig group.
    #[must_use]
    pub const fn with_tool_speed(mut self, f: Option<fn(DigMaterial, u16) -> f32>) -> Self {
        self.tool_speed = f;
        self
    }
}

impl DigFactors {
    /// On ground.
    #[must_use]
    pub const fn with_on_ground(mut self, on: bool) -> Self {
        self.on_ground = on;
        self
    }

    /// Eyes in water.
    #[must_use]
    pub const fn with_in_water(mut self, on: bool) -> Self {
        self.in_water = on;
        self
    }

    /// Aqua affinity on the helmet.
    #[must_use]
    pub const fn with_aqua_affinity(mut self, on: bool) -> Self {
        self.aqua_affinity = on;
        self
    }

    /// Haste level (amplifier + 1), 0 = none.
    #[must_use]
    pub const fn with_haste(mut self, level: u8) -> Self {
        self.haste = level;
        self
    }

    /// Mining fatigue level (amplifier + 1), 0 = none.
    #[must_use]
    pub const fn with_fatigue(mut self, level: u8) -> Self {
        self.fatigue = level;
        self
    }
}

/// Mining speed modifiers of the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct DigFactors {
    /// On ground.
    pub on_ground: bool,
    /// Eyes in water.
    pub in_water: bool,
    /// Aqua affinity on the helmet.
    pub aqua_affinity: bool,
    /// Haste level (amplifier + 1), 0 = none.
    pub haste: u8,
    /// Mining fatigue level (amplifier + 1), 0 = none.
    pub fatigue: u8,
}

/// The version's destroy progress per tick (oracle-verified functions of `ghast-data`).
fn exact_progress(b: &BlockQuery, e: ExactDig, held: Option<&HeldItem>, f: DigFactors) -> f32 {
    let item = held.map(|h| h.id);
    let efficiency = held.map_or(0, |h| h.efficiency);
    let amp = |level: u8| (level > 0).then(|| i32::from(level) - 1);
    let _ = (item, efficiency, &amp, b);
    match e {
        #[cfg(feature = "v1_8")]
        ExactDig::V1_8 => ghast_data::v1_8::dig::destroy_progress(
            item,
            b.block,
            &ghast_data::v1_8::dig::DigConditions {
                efficiency,
                haste: amp(f.haste),
                mining_fatigue: amp(f.fatigue),
                eye_in_water: f.in_water,
                aqua_affinity: f.aqua_affinity,
                on_ground: f.on_ground,
            },
        ),
        #[cfg(feature = "v1_12_2")]
        ExactDig::V1_12 => ghast_data::v1_12::dig::destroy_progress(
            item,
            b.block,
            &ghast_data::v1_12::dig::DigConditions {
                efficiency,
                haste: amp(f.haste),
                mining_fatigue: amp(f.fatigue),
                eye_in_water: f.in_water,
                aqua_affinity: f.aqua_affinity,
                on_ground: f.on_ground,
            },
        ),
        #[cfg(feature = "v1_16_5")]
        ExactDig::V1_16_5 => ghast_data::v1_16_5::dig::destroy_progress(
            item,
            b.block,
            &ghast_data::v1_16_5::dig::DigConditions {
                efficiency,
                haste: amp(f.haste),
                conduit_power: None,
                mining_fatigue: amp(f.fatigue),
                eye_in_water: f.in_water,
                aqua_affinity: f.aqua_affinity,
                on_ground: f.on_ground,
            },
        ),
        #[cfg(feature = "v26_1")]
        ExactDig::V26_1(m) => crate::sim::v26_1::dig_progress(&m, held, f),
    }
}

/// `Block.calcBlockBreakingData` (float maths): break progress per tick.
#[must_use]
pub fn relative_hardness(b: &BlockQuery, held: Option<&HeldItem>, f: DigFactors) -> f32 {
    if let Some(e) = b.exact {
        return exact_progress(b, e, held, f);
    }
    let h = b.hardness;
    if h < 0.0 {
        return 0.0;
    }
    let can_harvest =
        b.harvest_tools.is_empty() || held.is_some_and(|i| b.harvest_tools.contains(&i.id));
    // PlayerEntity.getMiningSpeed
    let mut speed = match (held, b.dig_material) {
        (Some(i), Some(m)) => i.tool_speed.map_or(1.0, |t| t(m, i.id)),
        _ => 1.0,
    };
    if speed > 1.0
        && let Some(i) = held
        && i.efficiency > 0
    {
        let e = f32::from(i.efficiency);
        speed += e * e + 1.0;
    }
    if f.haste > 0 {
        speed *= 1.0 + f32::from(f.haste) * 0.2;
    }
    if f.fatigue > 0 {
        speed *= match f.fatigue {
            1 => 0.3,
            2 => 0.09,
            3 => 0.0027,
            _ => 8.1e-4,
        };
    }
    if f.in_water && !f.aqua_affinity {
        speed /= 5.0;
    }
    if !f.on_ground {
        speed /= 5.0;
    }
    if can_harvest {
        speed / h / 30.0
    } else {
        speed / h / 100.0
    }
}

/// `Entity.getRotationVector(pitch, yaw)` with vanilla's float sine table.
#[must_use]
pub fn look_vector(yaw: f32, pitch: f32) -> Vec3 {
    use ghast_physics::math::{cos, sin};
    const RAD: f32 = 0.017_453_292;
    let f = cos(-yaw * RAD - std::f32::consts::PI);
    let f1 = sin(-yaw * RAD - std::f32::consts::PI);
    let f2 = -cos(-pitch * RAD);
    let f3 = sin(-pitch * RAD);
    Vec3::new(f64::from(f1 * f2), f64::from(f3), f64::from(f * f2))
}

/// A block the crosshair is on.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct BlockHit {
    /// The block.
    pub pos: BlockPos,
    /// The face hit.
    pub face: Face,
    /// The point hit (absolute).
    pub hit: Vec3,
    /// Distance from the eyes.
    pub distance: f64,
}

/// Floor of a coordinate as vanilla's `MathHelper.floor`.
fn floor(v: f64) -> i32 {
    let i = v as i32;
    if v < f64::from(i) { i - 1 } else { i }
}

/// `Vec3d.lerpForX` and friends: the point on the segment `a → b` whose `axis`
/// coordinate is `v`, if it lies within the segment.
fn intermediate(a: Vec3, b: Vec3, axis: usize, v: f64) -> Option<Vec3> {
    let d = [b.x - a.x, b.y - a.y, b.z - a.z];
    let o = [a.x, a.y, a.z];
    if d[axis] * d[axis] < 1.0e-7 {
        return None;
    }
    let t = (v - o[axis]) / d[axis];
    (0.0..=1.0)
        .contains(&t)
        .then(|| Vec3::new(a.x + d[0] * t, a.y + d[1] * t, a.z + d[2] * t))
}

fn sq_dist(a: Vec3, b: Vec3) -> f64 {
    let (x, y, z) = (b.x - a.x, b.y - a.y, b.z - a.z);
    x * x + y * y + z * z
}

/// `Block.rayTrace`: the segment against the block's ray bounds, in
/// block-local coordinates; the nearest of the six plane hits (ties go to the earlier plane:
/// west, east, down, up, north, south).
fn block_ray(pos: BlockPos, b: &[f64; 6], start: Vec3, end: Vec3) -> Option<(Vec3, Face)> {
    let off = Vec3::new(f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
    let s = Vec3::new(start.x - off.x, start.y - off.y, start.z - off.z);
    let e = Vec3::new(end.x - off.x, end.y - off.y, end.z - off.z);
    let in_yz = |v: &Vec3| v.y >= b[1] && v.y <= b[4] && v.z >= b[2] && v.z <= b[5];
    let in_xz = |v: &Vec3| v.x >= b[0] && v.x <= b[3] && v.z >= b[2] && v.z <= b[5];
    let in_xy = |v: &Vec3| v.x >= b[0] && v.x <= b[3] && v.y >= b[1] && v.y <= b[4];
    let candidates = [
        (intermediate(s, e, 0, b[0]).filter(in_yz), Face::West),
        (intermediate(s, e, 0, b[3]).filter(in_yz), Face::East),
        (intermediate(s, e, 1, b[1]).filter(in_xz), Face::Down),
        (intermediate(s, e, 1, b[4]).filter(in_xz), Face::Up),
        (intermediate(s, e, 2, b[2]).filter(in_xy), Face::North),
        (intermediate(s, e, 2, b[5]).filter(in_xy), Face::South),
    ];
    let mut best: Option<(Vec3, Face)> = None;
    for (v, face) in candidates {
        if let Some(v) = v
            && best.is_none_or(|(bv, _)| sq_dist(s, v) < sq_dist(s, bv))
        {
            best = Some((v, face));
        }
    }
    best.map(|(v, f)| (Vec3::new(v.x + off.x, v.y + off.y, v.z + off.z), f))
}

/// What a ray found.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum RayResult {
    /// A block's ray bounds.
    Block(BlockHit),
    /// Nothing; the last block the ray passed that rays go through (vanilla's "miss" result
    /// of the crosshair pick).
    Miss {
        /// That block.
        pos: BlockPos,
        /// The face the ray entered it through.
        face: Face,
        /// The entry point.
        hit: Vec3,
    },
    /// Nothing at all.
    None,
}

/// 1.8 `World.rayTrace(start, end, false, false, return_last)`: walks the blocks from `start`
/// to `end` exactly like vanilla (at most 200 steps; −1e−4 for a −0 plane fraction) and tests
/// each block whose rays collide (`BlockQuery::selection`). Unloaded blocks read as air like
/// the vanilla client's empty chunks.
pub fn ray_trace(
    blocks: &dyn Fn(i32, i32, i32) -> Option<BlockQuery>,
    start: Vec3,
    end: Vec3,
    return_last: bool,
) -> RayResult {
    let nan = |v: Vec3| v.x.is_nan() || v.y.is_nan() || v.z.is_nan();
    if nan(start) || nan(end) {
        return RayResult::None;
    }
    let dist = |hit: Vec3, eyes: Vec3| sq_dist(eyes, hit).sqrt();
    let eyes = start;
    let (ex, ey, ez) = (floor(end.x), floor(end.y), floor(end.z));
    let (mut x, mut y, mut z) = (floor(start.x), floor(start.y), floor(start.z));
    let test = |p: BlockPos, s: Vec3| -> Option<BlockHit> {
        let b = blocks(p.x, p.y, p.z)?;
        let found = if let Some(passes) = &b.stairs {
            // StairsBlock.rayTrace: the pass hit farthest from the ray's end wins (strictly;
            // ties keep the earlier pass).
            let mut best: Option<(Vec3, Face)> = None;
            let mut best_d = 0.0;
            for bounds in passes.iter().flatten() {
                if let Some((hit, face)) = block_ray(p, bounds, s, end) {
                    let d = sq_dist(hit, end);
                    if d > best_d {
                        best = Some((hit, face));
                        best_d = d;
                    }
                }
            }
            best
        } else {
            block_ray(p, &b.selection?, s, end)
        };
        found.map(|(hit, face)| BlockHit {
            pos: p,
            face,
            hit,
            distance: dist(hit, eyes),
        })
    };
    let first = BlockPos::new(x, y, z);
    if let Some(h) = test(first, start) {
        return RayResult::Block(h);
    }
    let mut last = RayResult::None;
    let mut cur = start;
    let mut steps = 200;
    while steps >= 0 {
        steps -= 1;
        if nan(cur) {
            return RayResult::None;
        }
        if x == ex && y == ey && z == ez {
            break;
        }
        let plane = |e: i32, c: i32| {
            if e > c {
                Some(f64::from(c) + 1.0)
            } else if e < c {
                Some(f64::from(c))
            } else {
                None
            }
        };
        let (px, py, pz) = (plane(ex, x), plane(ey, y), plane(ez, z));
        let (dx, dy, dz) = (end.x - cur.x, end.y - cur.y, end.z - cur.z);
        let frac = |p: Option<f64>, o: f64, d: f64| {
            let t = p.map_or(999.0, |p| (p - o) / d);
            // Vanilla's `t == -0.0` also matches +0.0: any zero fraction becomes -1e-4.
            if t == 0.0 { -1.0e-4 } else { t }
        };
        let (tx, ty, tz) = (
            frac(px, cur.x, dx),
            frac(py, cur.y, dy),
            frac(pz, cur.z, dz),
        );
        let face;
        if tx < ty && tx < tz {
            face = if ex > x { Face::West } else { Face::East };
            cur = Vec3::new(px.unwrap_or(999.0), cur.y + dy * tx, cur.z + dz * tx);
        } else if ty < tz {
            face = if ey > y { Face::Down } else { Face::Up };
            cur = Vec3::new(cur.x + dx * ty, py.unwrap_or(999.0), cur.z + dz * ty);
        } else {
            face = if ez > z { Face::North } else { Face::South };
            cur = Vec3::new(cur.x + dx * tz, cur.y + dy * tz, pz.unwrap_or(999.0));
        }
        x = floor(cur.x) - i32::from(face == Face::East);
        y = floor(cur.y) - i32::from(face == Face::Up);
        z = floor(cur.z) - i32::from(face == Face::South);
        let p = BlockPos::new(x, y, z);
        match blocks(x, y, z).filter(|b| b.selection.is_some() || b.stairs.is_some()) {
            Some(_) => {
                if let Some(h) = test(p, cur) {
                    return RayResult::Block(h);
                }
            }
            None => {
                last = RayResult::Miss {
                    pos: p,
                    face,
                    hit: cur,
                }
            }
        }
    }
    if return_last { last } else { RayResult::None }
}

/// The crosshair block: `Entity.rayTrace(reach)` from `eyes` along `look`.
#[must_use]
pub fn pick(
    blocks: &dyn Fn(i32, i32, i32) -> Option<BlockQuery>,
    eyes: Vec3,
    look: Vec3,
    reach: f64,
) -> Option<BlockHit> {
    let end = Vec3::new(
        eyes.x + look.x * reach,
        eyes.y + look.y * reach,
        eyes.z + look.z * reach,
    );
    match ray_trace(blocks, eyes, end, true) {
        RayResult::Block(h) => Some(h),
        _ => None,
    }
}

/// An entity the crosshair is on.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct EntityHit {
    /// The entity id.
    pub id: i32,
    /// The point hit (absolute; the eyes when they are inside the box).
    pub hit: Vec3,
    /// The entity's position (for interact-at offsets).
    pub entity_pos: Vec3,
    /// Distance from the eyes.
    pub distance: f64,
}

/// `Box.method_585` (unnamed in Legacy Yarn): the nearest plane hit of the segment `a → b` on the
/// box (`x` planes, then `y`, then `z`; ties keep the earlier one).
fn box_intercept(b: &[f64; 6], a: Vec3, e: Vec3) -> Option<Vec3> {
    let in_yz = |v: &Vec3| v.y >= b[1] && v.y <= b[4] && v.z >= b[2] && v.z <= b[5];
    let in_xz = |v: &Vec3| v.x >= b[0] && v.x <= b[3] && v.z >= b[2] && v.z <= b[5];
    let in_xy = |v: &Vec3| v.x >= b[0] && v.x <= b[3] && v.y >= b[1] && v.y <= b[4];
    let cands = [
        intermediate(a, e, 0, b[0]).filter(in_yz),
        intermediate(a, e, 0, b[3]).filter(in_yz),
        intermediate(a, e, 1, b[1]).filter(in_xz),
        intermediate(a, e, 1, b[4]).filter(in_xz),
        intermediate(a, e, 2, b[2]).filter(in_xy),
        intermediate(a, e, 2, b[5]).filter(in_xy),
    ];
    let mut best: Option<Vec3> = None;
    for v in cands.into_iter().flatten() {
        if best.is_none_or(|bv| sq_dist(a, v) < sq_dist(a, bv)) {
            best = Some(v);
        }
    }
    best
}

/// Whether the ray from `eyes` along `look` (unit) meets the full cube at `pos` within
/// `reach`, or starts inside it.
#[must_use]
pub(crate) fn ray_hits_cube(eyes: Vec3, look: Vec3, pos: BlockPos, reach: f64) -> bool {
    let (x, y, z) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
    let b = [x, y, z, x + 1.0, y + 1.0, z + 1.0];
    let inside = eyes.x >= b[0]
        && eyes.x <= b[3]
        && eyes.y >= b[1]
        && eyes.y <= b[4]
        && eyes.z >= b[2]
        && eyes.z <= b[5];
    let end = Vec3::new(
        eyes.x + look.x * reach,
        eyes.y + look.y * reach,
        eyes.z + look.z * reach,
    );
    inside || box_intercept(&b, eyes, end).is_some()
}

/// Mouse buttons held in the world (kept until changed; released by a join or respawn).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Buttons {
    /// Left: mine / attack.
    pub left: bool,
    /// Right: use / place.
    pub right: bool,
}

/// What the hands need from the client each tick.
pub(crate) struct HandsIo<'a> {
    /// Block lookup (version-specific facts).
    pub blocks: &'a dyn Fn(i32, i32, i32) -> Option<BlockQuery>,
    /// The crosshair now.
    pub target: Option<BlockHit>,
    /// The held stack's dig / use facts.
    pub held: Option<HeldItem>,
    /// The held stack itself (sent with 1.8 placement packets).
    pub held_stack: Option<ghast_inventory::Stack>,
    /// Food level (food is only eaten below 20 unless always edible).
    pub food: i32,
    /// 1.9+ rules: swing after the click, both hands tried on right click.
    pub modern: bool,
    /// The off-hand stack's facts (1.9+).
    pub off_held: Option<HeldItem>,
    /// The off-hand stack itself.
    pub off_stack: Option<ghast_inventory::Stack>,
    /// Mining speed modifiers.
    pub factors: DigFactors,
    /// Sneaking (skips block activation when holding an item).
    pub sneaking: bool,
    /// The entity the crosshair is on (it wins over `target` when set).
    pub entity: Option<EntityHit>,
    /// Packets out (in order).
    pub out: &'a mut Vec<Outbound>,
    /// Blocks to predict as air (broken on the client).
    pub broken: &'a mut Vec<BlockPos>,
    /// The held stack's count went down by one (a block was placed).
    pub placed: &'a mut bool,
    /// Spectator mode: items are never used (vanilla's `useItem` passes without a packet).
    pub spectator: bool,
}

impl HandsIo<'_> {
    /// The non-air block at `pos` (`None`: air or not loaded). A block broken earlier in this
    /// tick (by the behaviour's `dig` or the hands) reads as air, like vanilla, which sets it
    /// to air in the world at once: the held button must not start it again in the same tick.
    fn solid(&self, pos: BlockPos) -> Option<BlockQuery> {
        if self.broken.contains(&pos) {
            return None;
        }
        (self.blocks)(pos.x, pos.y, pos.z).filter(|b| !b.air)
    }
}

/// `ClientPlayerInteractionManager` + the `MinecraftClient` click state.
#[derive(Debug, Clone, Default)]
pub struct Hands {
    hitting: bool,
    current: Option<BlockPos>,
    damage: f32,
    hit_item: Option<u16>,
    hit_delay: u32,
    left_click_counter: u32,
    right_click_delay: u32,
    using: bool,
    /// The hand and item id of the use in progress (`getUsedItemHand`, `useItem`).
    use_hand_off: bool,
    use_item: u16,
    /// Player ticks the current use has lasted (`getTicksUsingItem`).
    use_ticks: u32,
    /// This tick's key handling released a use: (off hand, item id, ticks it lasted).
    released: Option<(bool, u16, u32)>,
    left_was: bool,
    right_was: bool,
    /// Blocks broken (finish sent).
    pub broken: u64,
    /// 1.9+: ticks since the last swing / attack (`ticksSinceLastSwing`, attack cooldown).
    pub ticks_since_swing: u32,
}

impl Hands {
    /// A new player: nothing held, nothing in progress.
    pub fn reset(&mut self) {
        let broken = self.broken;
        *self = Hands::default();
        self.broken = broken;
    }

    /// The block being broken and its progress 0..1.
    #[must_use]
    pub fn mining(&self) -> Option<(BlockPos, f32)> {
        match (self.hitting, self.current) {
            (true, Some(p)) => Some((p, self.damage.min(1.0))),
            _ => None,
        }
    }

    /// Using the held item (eating, blocking, drawing a bow).
    #[must_use]
    pub fn using_item(&self) -> bool {
        self.using
    }

    /// The use in progress: (off hand, item id).
    #[must_use]
    pub fn use_item(&self) -> Option<(bool, u16)> {
        self.using.then_some((self.use_hand_off, self.use_item))
    }

    /// `startUsingItem`: remembers the hand and the item.
    fn start_using(&mut self, off_hand: bool, id: u16) {
        self.use_ticks = 0;
        self.using = true;
        self.use_hand_off = off_hand;
        self.use_item = id;
    }

    /// The use this tick's key handling released, if any: (off hand, item id, player ticks it
    /// lasted).
    #[must_use]
    pub(crate) fn released_use(&self) -> Option<(bool, u16, u32)> {
        self.released
    }

    /// One player tick of the use in progress (`updatingUsingItem`).
    pub(crate) fn count_use_tick(&mut self) {
        if self.using {
            self.use_ticks = self.use_ticks.saturating_add(1);
        }
    }

    /// The server finished our item use (entity status 9) or the held slot changed.
    pub fn stop_using(&mut self) {
        self.using = false;
    }

    /// One tick with the buttons as held now.
    /// 1.9+ attack strength: `clamp((ticks_since_swing + 0.5) / (20 / attack_speed), 0, 1)`
    /// (vanilla `getCooledAttackStrength(0.5F)`; attack speed 4.0 with an empty hand).
    #[must_use]
    pub fn attack_strength(&self, attack_speed: f64) -> f32 {
        let period = (1.0 / attack_speed * 20.0) as f32;
        ((self.ticks_since_swing as f32 + 0.5) / period).clamp(0.0, 1.0)
    }

    /// `PlayerController.attackEntity`: the attack packet (1.16: then the cooldown resets).
    pub(crate) fn attack(&mut self, id: i32, io: &mut HandsIo<'_>) {
        io.out.push(Outbound::UseEntity {
            target: id,
            action: crate::common::EntityUse::Attack,
            off_hand: false,
            sneaking: io.sneaking,
        });
        self.ticks_since_swing = 0;
    }

    /// Right click on an entity: interact-at then interact (both assumed not handled, so the
    /// item use follows like vanilla's fall-through for plain entities).
    fn interact(&mut self, e: EntityHit, off_hand: bool, io: &mut HandsIo<'_>) {
        let rel = Vec3::new(
            e.hit.x - e.entity_pos.x,
            e.hit.y - e.entity_pos.y,
            e.hit.z - e.entity_pos.z,
        );
        io.out.push(Outbound::UseEntity {
            target: e.id,
            action: crate::common::EntityUse::InteractAt(rel),
            off_hand,
            sneaking: io.sneaking,
        });
        io.out.push(Outbound::UseEntity {
            target: e.id,
            action: crate::common::EntityUse::Interact,
            off_hand,
            sneaking: io.sneaking,
        });
    }

    pub(crate) fn tick(&mut self, left: bool, right: bool, io: &mut HandsIo<'_>) {
        self.ticks_since_swing = self.ticks_since_swing.saturating_add(1);
        if self.left_click_counter > 0 {
            self.left_click_counter -= 1;
        }
        if self.right_click_delay > 0 {
            self.right_click_delay -= 1;
        }
        let t = io.target;
        self.released = None;
        if self.using {
            if !right {
                // releaseUsingItem: the item's releaseUsing (a riptide launch) runs on the client,
                // then the packet.
                self.released = Some((self.use_hand_off, self.use_item, self.use_ticks));
                io.out.push(Outbound::Dig {
                    status: DigStatus::ReleaseUseItem,
                    pos: BlockPos::new(0, 0, 0),
                    face: Face::Down,
                });
                self.using = false;
            }
        } else {
            if left && !self.left_was {
                self.click_mouse(t, io);
            }
            if right && !self.right_was {
                self.right_click_mouse(t, io);
            }
        }
        if right && self.right_click_delay == 0 && !self.using {
            self.right_click_mouse(t, io);
        }
        self.send_click_block_to_controller(left, t, io);
        self.left_was = left;
        self.right_was = right;
    }

    fn click_mouse(&mut self, t: Option<BlockHit>, io: &mut HandsIo<'_>) {
        if self.left_click_counter > 0 {
            return;
        }
        if let Some(e) = io.entity {
            if io.modern {
                self.attack(e.id, io);
                io.out.push(Outbound::Swing);
            } else {
                io.out.push(Outbound::Swing);
                self.attack(e.id, io);
            }
            return;
        }
        if io.modern {
            // 1.16 MinecraftClient.doAttack: the action, then the swing; a block hit on air falls
            // through to the miss case.
            match t {
                Some(hit) if io.solid(hit.pos).is_some() => {
                    self.click_block(hit.pos, hit.face, io);
                }
                _ => {
                    self.left_click_counter = 10;
                    // resetCooldown on a miss.
                    self.ticks_since_swing = 0;
                }
            }
            io.out.push(Outbound::Swing);
            return;
        }
        io.out.push(Outbound::Swing);
        match t {
            Some(hit) => {
                if io.solid(hit.pos).is_some() {
                    self.click_block(hit.pos, hit.face, io);
                }
            }
            None => self.left_click_counter = 10,
        }
    }

    fn send_click_block_to_controller(
        &mut self,
        left: bool,
        t: Option<BlockHit>,
        io: &mut HandsIo<'_>,
    ) {
        if !left {
            self.left_click_counter = 0;
        }
        if self.left_click_counter > 0 || self.using {
            return;
        }
        match t.filter(|_| io.entity.is_none()) {
            Some(hit) if left => {
                let solid = io.solid(hit.pos).is_some();
                if solid && self.damage_block(hit.pos, hit.face, io) {
                    io.out.push(Outbound::Swing);
                }
            }
            _ => self.reset_block_removing(io),
        }
    }

    fn hitting_position(&self, pos: BlockPos, io: &HandsIo<'_>) -> bool {
        self.hitting && self.current == Some(pos) && self.hit_item == io.held.map(|h| h.id)
    }

    /// `attackBlock`: START (aborting a previous block), instant break when fast enough.
    pub(crate) fn click_block(&mut self, pos: BlockPos, face: Face, io: &mut HandsIo<'_>) {
        if self.hitting && self.hitting_position(pos, io) {
            return;
        }
        if self.hitting
            && let Some(cur) = self.current
        {
            io.out.push(Outbound::Dig {
                status: DigStatus::Abort,
                pos: cur,
                face,
            });
        }
        io.out.push(Outbound::Dig {
            status: DigStatus::Start,
            pos,
            face,
        });
        if let Some(b) = io.solid(pos)
            && relative_hardness(&b, io.held.as_ref(), io.factors) >= 1.0
        {
            self.destroyed(pos, io);
            return;
        }
        self.hitting = true;
        self.current = Some(pos);
        self.hit_item = io.held.map(|h| h.id);
        self.damage = 0.0;
    }

    /// `updateBlockBreakingProgress`.
    fn damage_block(&mut self, pos: BlockPos, face: Face, io: &mut HandsIo<'_>) -> bool {
        if self.hit_delay > 0 {
            self.hit_delay -= 1;
            return true;
        }
        if !self.hitting_position(pos, io) {
            self.click_block(pos, face, io);
            return true;
        }
        let Some(b) = io.solid(pos) else {
            self.hitting = false;
            return false;
        };
        self.damage += relative_hardness(&b, io.held.as_ref(), io.factors);
        if self.damage >= 1.0 {
            self.hitting = false;
            io.out.push(Outbound::Dig {
                status: DigStatus::Finish,
                pos,
                face,
            });
            self.destroyed(pos, io);
            self.damage = 0.0;
            self.hit_delay = 5;
        }
        true
    }

    /// `breakBlock`: air on the client; the current block moves to y = −1.
    fn destroyed(&mut self, pos: BlockPos, io: &mut HandsIo<'_>) {
        io.broken.push(pos);
        if let Some(c) = self.current {
            self.current = Some(BlockPos::new(c.x, -1, c.z));
        }
        self.broken += 1;
    }

    /// `cancelBlockBreaking`.
    pub(crate) fn reset_block_removing(&mut self, io: &mut HandsIo<'_>) {
        if !self.hitting {
            return;
        }
        if let Some(cur) = self.current {
            io.out.push(Outbound::Dig {
                status: DigStatus::Abort,
                pos: cur,
                face: Face::Down,
            });
        }
        self.hitting = false;
        self.damage = 0.0;
    }

    /// `doUse` (nothing while a block is being hit).
    fn right_click_mouse(&mut self, t: Option<BlockHit>, io: &mut HandsIo<'_>) {
        if self.hitting {
            return;
        }
        self.right_click_delay = 4;
        if io.modern {
            self.right_click_modern(t, io);
            return;
        }
        if let Some(e) = io.entity {
            // 1.8: interact-at, interact; not handled -> the item is used.
            self.interact(e, false, io);
            if let Some(h) = io.held.filter(|_| !io.spectator) {
                io.out.push(Outbound::UseItem {
                    held: io.held_stack.clone(),
                    off_hand: false,
                });
                if h.usable && (!h.food || h.always_edible || io.food < 20) {
                    self.start_using(false, h.id);
                }
            }
            return;
        }
        let mut fall_through = true;
        if let Some(hit) = t
            && let Some(b) = io.solid(hit.pos)
        {
            if self.right_click_block(hit, &b, io) {
                fall_through = false;
                io.out.push(Outbound::Swing);
            }
            if io.held.is_none() {
                return;
            }
        }
        if fall_through
            && !io.spectator
            && let Some(h) = io.held
        {
            // interactItem: the item itself (position -1, face 255).
            io.out.push(Outbound::UseItem {
                held: io.held_stack.clone(),
                off_hand: false,
            });
            // FoodItem.onStartUse: only when the player can eat.
            if h.usable && (!h.food || h.always_edible || io.food < 20) {
                self.start_using(false, h.id);
            }
        }
    }

    /// 1.9+ `doItemUse` (1.16.5 Yarn): main hand, then off hand; per hand the block (activation, then
    /// the item on the block), then the item itself; the first success / consume ends it.
    fn right_click_modern(&mut self, t: Option<BlockHit>, io: &mut HandsIo<'_>) {
        let any_item = io.held.is_some() || io.off_held.is_some();
        for off_hand in [false, true] {
            let (held, stack) = if off_hand {
                (io.off_held, io.off_stack.clone())
            } else {
                (io.held, io.held_stack.clone())
            };
            let swing = if off_hand {
                Outbound::SwingOffHand
            } else {
                Outbound::Swing
            };
            if let Some(e) = io.entity {
                self.interact(e, off_hand, io);
            } else if let Some(hit) = t
                && let Some(b) = io.solid(hit.pos)
            {
                let cursor = Vec3::new(
                    hit.hit.x - f64::from(hit.pos.x),
                    hit.hit.y - f64::from(hit.pos.y),
                    hit.hit.z - f64::from(hit.pos.z),
                );
                io.out.push(Outbound::Place {
                    pos: hit.pos,
                    face: hit.face,
                    cursor,
                    held: stack.clone(),
                    off_hand,
                });
                // Sneaking with something in either hand skips the block's activation.
                if b.activates && !(io.sneaking && any_item) {
                    io.out.push(swing);
                    return;
                }
                if let Some(h) = held
                    && h.places_block
                    && h.count > 0
                {
                    if !off_hand {
                        *io.placed = true;
                    }
                    io.out.push(swing);
                    return;
                }
            }
            if let Some(h) = held.filter(|_| !io.spectator) {
                io.out.push(Outbound::UseItem {
                    held: stack,
                    off_hand,
                });
                if h.swing_on_use {
                    // InteractionResult.SUCCESS with a client swing: the use ends here.
                    io.out.push(swing);
                    return;
                }
                if h.usable && (!h.food || h.always_edible || io.food < 20) {
                    self.start_using(off_hand, h.id);
                    return;
                }
            }
        }
    }

    /// `onRightClick`: the placement packet; returns whether something was activated or
    /// placed (the caller swings).
    pub(crate) fn right_click_block(
        &mut self,
        hit: BlockHit,
        b: &BlockQuery,
        io: &mut HandsIo<'_>,
    ) -> bool {
        let cursor = Vec3::new(
            f64::from((hit.hit.x - f64::from(hit.pos.x)) as f32),
            f64::from((hit.hit.y - f64::from(hit.pos.y)) as f32),
            f64::from((hit.hit.z - f64::from(hit.pos.z)) as f32),
        );
        let activated = b.activates && (!io.sneaking || io.held.is_none());
        io.out.push(Outbound::Place {
            pos: hit.pos,
            face: hit.face,
            cursor,
            held: io.held_stack.clone(),
            off_hand: false,
        });
        if activated {
            return true;
        }
        match io.held {
            Some(h) if h.places_block && h.count > 0 => {
                *io.placed = true;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone() -> BlockQuery {
        BlockQuery {
            exact: None,
            air: false,
            water: false,
            block: 1,
            hardness: 1.5,
            dig_material: Some(DigMaterial::Rock),
            harvest_tools: &[270, 274],
            selection: Some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
            stairs: None,
            activates: false,
        }
    }

    #[test]
    fn hand_stone_time() {
        // Stone by hand, on ground: 1 / 1.5 / 100 per tick; the f32 sum (as vanilla's
        // currentBreakingProgress) reaches 1 on the 151st tick, not the 150th.
        let f = DigFactors {
            on_ground: true,
            ..DigFactors::default()
        };
        let per_tick = relative_hardness(&stone(), None, f);
        let mut d = 0.0_f32;
        let mut n = 0;
        while d < 1.0 {
            d += per_tick;
            n += 1;
        }
        assert_eq!(n, 151);
    }

    #[test]
    fn look_south_hits_block() {
        let look = look_vector(0.0, 0.0);
        assert!(look.z > 0.99 && look.x.abs() < 1e-3);
        let blocks = |x: i32, y: i32, z: i32| {
            Some(if (x, y, z) == (0, 1, 2) {
                stone()
            } else {
                BlockQuery {
                    air: true,
                    selection: None,
                    ..stone()
                }
            })
        };
        let hit = pick(&blocks, Vec3::new(0.5, 1.62, 0.5), look, REACH).unwrap();
        assert_eq!(hit.pos, BlockPos::new(0, 1, 2));
        assert_eq!(hit.face, Face::North);
        assert!((hit.distance - 1.5).abs() < 1e-3);
    }

    fn io_blocks(x: i32, y: i32, z: i32) -> Option<BlockQuery> {
        Some(if (x, y, z) == (0, 0, 0) {
            stone()
        } else {
            BlockQuery {
                air: true,
                selection: None,
                ..stone()
            }
        })
    }

    fn kinds(out: &[Outbound]) -> Vec<&'static str> {
        out.iter()
            .map(|o| match o {
                Outbound::Place {
                    off_hand: false, ..
                } => "place_main",
                Outbound::Place { off_hand: true, .. } => "place_off",
                Outbound::UseItem {
                    off_hand: false, ..
                } => "use_main",
                Outbound::UseItem { off_hand: true, .. } => "use_off",
                Outbound::Swing => "swing",
                Outbound::SwingOffHand => "swing_off",
                Outbound::Dig { .. } => "dig",
                _ => "other",
            })
            .collect()
    }

    /// 1.16 right click: empty hands try both hands on the block; a block item places with
    /// the main hand and swings.
    #[test]
    fn modern_right_click_hands() {
        let hit = Some(BlockHit {
            pos: BlockPos::new(0, 0, 0),
            face: Face::Up,
            hit: Vec3::new(0.5, 1.0, 0.5),
            distance: 1.6,
        });
        let run = |held: Option<HeldItem>| {
            let mut h = Hands::default();
            let (mut out, mut broken, mut placed) = (Vec::new(), Vec::new(), false);
            let mut io = HandsIo {
                blocks: &io_blocks,
                target: hit,
                held,
                held_stack: None,
                food: 20,
                modern: true,
                off_held: None,
                off_stack: None,
                entity: None,
                factors: DigFactors::default(),
                sneaking: false,
                out: &mut out,
                broken: &mut broken,
                placed: &mut placed,
                spectator: false,
            };
            h.tick(false, true, &mut io);
            kinds(&out)
        };
        assert_eq!(run(None), ["place_main", "place_off"]);
        let dirt = HeldItem {
            id: 3,
            count: 10,
            places_block: true,
            ..HeldItem::default()
        };
        assert_eq!(run(Some(dirt)), ["place_main", "swing"]);
        // An item that is neither a block nor usable: place on the block, then use it.
        let stick = HeldItem {
            id: 280,
            count: 1,
            ..HeldItem::default()
        };
        assert_eq!(run(Some(stick)), ["place_main", "use_main", "place_off"]);
    }

    /// Pressing the left button on an instant-break block sends one START: the held-button
    /// pass of the same tick sees the block as air (vanilla breaks it in the world at once),
    /// for 1.8 and 1.16 alike. Also with a block the behaviour broke earlier in the tick.
    #[test]
    fn instant_break_press_starts_once() {
        fn instant(x: i32, y: i32, z: i32) -> Option<BlockQuery> {
            Some(if (x, y, z) == (0, 0, 0) {
                BlockQuery {
                    hardness: 0.0,
                    ..stone()
                }
            } else {
                BlockQuery {
                    air: true,
                    selection: None,
                    ..stone()
                }
            })
        }
        let hit = Some(BlockHit {
            pos: BlockPos::new(0, 0, 0),
            face: Face::Up,
            hit: Vec3::new(0.5, 1.0, 0.5),
            distance: 1.6,
        });
        for modern in [false, true] {
            for pre_broken in [false, true] {
                let mut h = Hands::default();
                let (mut out, mut placed) = (Vec::new(), false);
                let mut broken = if pre_broken {
                    vec![BlockPos::new(0, 0, 0)]
                } else {
                    Vec::new()
                };
                let mut io = HandsIo {
                    blocks: &instant,
                    target: hit,
                    held: None,
                    held_stack: None,
                    food: 20,
                    modern,
                    off_held: None,
                    off_stack: None,
                    entity: None,
                    factors: DigFactors::default(),
                    sneaking: false,
                    out: &mut out,
                    broken: &mut broken,
                    placed: &mut placed,
                    spectator: false,
                };
                h.tick(true, false, &mut io);
                let starts = out
                    .iter()
                    .filter(|o| {
                        matches!(
                            o,
                            Outbound::Dig {
                                status: DigStatus::Start,
                                ..
                            }
                        )
                    })
                    .count();
                let want = usize::from(!pre_broken);
                assert_eq!(
                    starts, want,
                    "modern {modern} pre_broken {pre_broken}: {out:?}"
                );
                assert_eq!(h.broken, want as u64);
            }
        }
    }

    /// 1.16 left click: block_dig start, then the swing (1.8: swing first).
    #[test]
    fn modern_left_click_order() {
        let hit = Some(BlockHit {
            pos: BlockPos::new(0, 0, 0),
            face: Face::Up,
            hit: Vec3::new(0.5, 1.0, 0.5),
            distance: 1.6,
        });
        for (modern, want) in [(true, ["dig", "swing"]), (false, ["swing", "dig"])] {
            let mut h = Hands::default();
            let (mut out, mut broken, mut placed) = (Vec::new(), Vec::new(), false);
            let mut io = HandsIo {
                blocks: &io_blocks,
                target: hit,
                held: None,
                held_stack: None,
                food: 20,
                modern,
                off_held: None,
                off_stack: None,
                entity: None,
                factors: DigFactors::default(),
                sneaking: false,
                out: &mut out,
                broken: &mut broken,
                placed: &mut placed,
                spectator: false,
            };
            h.click_mouse(hit, &mut io);
            assert_eq!(kinds(&out)[..2], want);
        }
    }

    #[test]
    fn dig_sequence() {
        let blocks = |x: i32, y: i32, z: i32| {
            Some(if (x, y, z) == (0, 0, 0) {
                BlockQuery {
                    hardness: 0.6,
                    ..stone()
                }
            } else {
                BlockQuery {
                    air: true,
                    selection: None,
                    ..stone()
                }
            })
        };
        let target = Some(BlockHit {
            pos: BlockPos::new(0, 0, 0),
            face: Face::Up,
            hit: Vec3::new(0.5, 1.0, 0.5),
            distance: 1.6,
        });
        let mut h = Hands::default();
        let mut out = Vec::new();
        let mut broken = Vec::new();
        let mut placed = false;
        let mut ticks = 0;
        while broken.is_empty() && ticks < 200 {
            let mut io = HandsIo {
                blocks: &blocks,
                target,
                held: None,
                held_stack: None,
                food: 20,
                modern: false,
                off_held: None,
                off_stack: None,
                entity: None,
                factors: DigFactors {
                    on_ground: true,
                    ..DigFactors::default()
                },
                sneaking: false,
                out: &mut out,
                broken: &mut broken,
                placed: &mut placed,
                spectator: false,
            };
            h.tick(true, false, &mut io);
            ticks += 1;
        }
        // 0.6 hardness, not harvestable by hand: 1/0.6/100 per tick; the f32 sum reaches 1 on
        // the 61st tick (damage is added from the START tick on).
        assert_eq!(broken, [BlockPos::new(0, 0, 0)]);
        let digs: Vec<_> = out
            .iter()
            .filter_map(|o| match o {
                Outbound::Dig { status, .. } => Some(*status),
                _ => None,
            })
            .collect();
        assert_eq!(digs, [DigStatus::Start, DigStatus::Finish]);
        assert_eq!(ticks, 61);
        // The click swings, and every damage tick swings (two on the first tick).
        assert_eq!(
            out.iter().filter(|o| matches!(o, Outbound::Swing)).count(),
            62
        );
    }
}
