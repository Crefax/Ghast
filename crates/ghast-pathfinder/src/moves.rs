//! The move model: which feet blocks a player can stand in, and which moves connect them.
//!
//! All geometry here is checked against `ghast-physics` by the validation tests
//! (`tests/validation.rs`): every move the generator produces on random terrain is executed by
//! the follower's controller in the bit-exact physics and must end where the planner expects.

use crate::class::{Class, flag};
use crate::world::{BlockSource, Grid};

/// Player height standing.
pub(crate) const HEIGHT: f64 = 1.8;
/// Vanilla step height.
pub(crate) const STEP: f64 = 0.6;
/// Peak height of a standing jump (0.42 initial velocity, measured: 1.2492).
pub(crate) const JUMP_APEX: f64 = 1.2492;
/// Highest step the planner jumps onto.
pub(crate) const MAX_JUMP_UP: f64 = 1.0 + 1e-6;
/// Headroom a jump-up needs above the landing height (apex minus step).
pub(crate) const JUMP_MARGIN: f64 = 0.15;

/// How the player is held up in a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NodeKind {
    /// Standing on a floor (possibly in shallow water or on a ladder's foot).
    Ground,
    /// Swimming (feet in water, nothing to stand on).
    Swim,
    /// On a ladder or vine (no floor).
    Climb,
}

/// How a step is reached from the previous one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MoveKind {
    /// The first step (where the player starts).
    Start,
    /// Walk to a cardinal neighbour; includes step-ups up to 0.6 and drops down to 0.6.
    Walk,
    /// Walk to a diagonal neighbour (both side columns clear).
    Diagonal,
    /// Jump up onto a block (0.6 < rise <= 1).
    Ascend,
    /// Walk off an edge and fall (drop > 0.6).
    Descend,
    /// Jump over a gap along one axis, onto a block up to one higher or two lower (the
    /// height change is the steps' `feet_y` difference). Only where the physics lands it.
    Parkour {
        /// Gap width in blocks (the landing block is `gap + 1` blocks away).
        gap: u8,
    },
    /// Swim (horizontally, up or down) or wade through water.
    Swim,
    /// Climb up a ladder / vine.
    ClimbUp,
    /// Climb down a ladder / vine.
    ClimbDown,
    /// Leave a ladder onto a floor above or beside it.
    ClimbExit,
}

/// A node: feet block and how the player stands there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spot {
    pub(crate) kind: NodeKind,
    /// Absolute feet height when standing (for swim / climb nodes: the block's y).
    pub(crate) floor: f64,
    /// Flags of the floor block (0 when none).
    pub(crate) floor_flags: u32,
    /// Feet or head in water.
    pub(crate) water: bool,
    /// Head in water.
    pub(crate) submerged: bool,
    /// The feet block is climbable.
    pub(crate) climb: bool,
}

/// `true` when nothing in column (x, z) collides with a body between heights `lo` and `hi`
/// (absolute), no hazard overlaps it and nothing there is unknown.
pub(crate) fn clear<S: BlockSource + ?Sized>(
    g: &mut Grid<'_, S>,
    x: i32,
    z: i32,
    lo: f64,
    hi: f64,
) -> bool {
    if hi <= lo {
        return true;
    }
    let y0 = lo.floor() as i32 - 1;
    let y1 = (hi - 1e-6).floor() as i32;
    for y in y0..=y1 {
        let c = g.class(x, y, z);
        let fy = f64::from(y);
        // Hazards anywhere in the cells the body overlaps.
        if fy < hi - 1e-6 && fy + 1.0 > lo + 1e-6 && c.has(flag::AVOID | flag::LAVA) {
            return false;
        }
        let l = (lo - fy) as f32;
        let h = (hi - fy) as f32;
        if c.blocks_between(l, h) {
            return false;
        }
    }
    true
}

/// The lowest obstacle bottom in column (x, z) between `from` and `limit` (absolute), or
/// `limit` when it is clear.
pub(crate) fn ceiling<S: BlockSource + ?Sized>(
    g: &mut Grid<'_, S>,
    x: i32,
    z: i32,
    from: f64,
    limit: f64,
) -> f64 {
    let y0 = from.floor() as i32;
    let y1 = (limit - 1e-6).floor() as i32;
    for y in y0..=y1 {
        let c = g.class(x, y, z);
        let fy = f64::from(y);
        if c.has(flag::AVOID | flag::LAVA) && fy + 1.0 > from {
            return fy.max(from);
        }
        if c.blocks_between((from - fy) as f32, (limit - fy) as f32) {
            let b = fy + f64::from(c.bottom);
            return b.max(from);
        }
    }
    limit
}

/// Where and how a player stands with the feet in block (x, y, z), if at all.
pub(crate) fn spot<S: BlockSource + ?Sized>(
    g: &mut Grid<'_, S>,
    x: i32,
    y: i32,
    z: i32,
) -> Option<Spot> {
    if !(1..255).contains(&y) {
        return None;
    }
    let c0 = g.class(x, y, z);
    if c0.has(flag::AVOID | flag::LAVA) {
        return None;
    }
    let mut floor: Option<(f64, Class)> = None;
    if c0.has(flag::SOLID) && !(c0.has(flag::THIN) && c0.has(flag::CLIMB)) {
        // A partial block in the feet cell is the floor (slab, soul sand, carpet, ...).
        if c0.has(flag::SUPPORT)
            && !c0.has(flag::IRREGULAR)
            && c0.top < 1.0 - 1e-6
            && c0.bottom <= 1e-6
            && c0.top_any <= c0.top + 1e-6
        {
            floor = Some((f64::from(y) + f64::from(c0.top), c0));
        } else {
            return None;
        }
    } else {
        let cb = g.class(x, y - 1, z);
        if cb.has(flag::SUPPORT) && !cb.has(flag::IRREGULAR) && cb.top >= 1.0 - 1e-6 {
            floor = Some((f64::from(y - 1) + f64::from(cb.top), cb));
        }
    }
    let water = c0.has(flag::WATER);
    let climb = c0.has(flag::CLIMB);
    let (kind, base, floor_flags) = match floor {
        Some((f, c)) => {
            if c.has(flag::AVOID_FLOOR) || f >= f64::from(y + 1) - 1e-6 {
                return None;
            }
            (NodeKind::Ground, f, c.flags)
        }
        None if water => (NodeKind::Swim, f64::from(y), 0),
        None if climb => (NodeKind::Climb, f64::from(y), 0),
        None => return None,
    };
    if !clear(g, x, z, base, base + HEIGHT) {
        return None;
    }
    let head = g.class(x, (base + 1.62).floor() as i32, z);
    let submerged = head.has(flag::WATER);
    if kind == NodeKind::Swim {
        // Swimming needs room above the feet that is water or air (no ceiling right on top).
        let up = g.class(x, y + 1, z);
        if up.has(flag::SOLID) {
            return None;
        }
    }
    Some(Spot {
        kind,
        floor: base,
        floor_flags,
        water: water || submerged,
        submerged,
        climb,
    })
}

/// The horizontal direction (dx, dz) a climber pushes towards to climb up, if any.
pub(crate) fn climb_wall<S: BlockSource + ?Sized>(
    g: &mut Grid<'_, S>,
    x: i32,
    y: i32,
    z: i32,
) -> Option<(i32, i32)> {
    let c = g.class(x, y, z);
    if !c.has(flag::CLIMB) {
        return None;
    }
    let side = match c.wall {
        1 => Some((0, -1)),
        2 => Some((0, 1)),
        3 => Some((-1, 0)),
        4 => Some((1, 0)),
        _ => None,
    };
    if side.is_some() {
        return side;
    }
    for (dx, dz) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
        let n = g.class(x + dx, y, z + dz);
        if n.has(flag::SUPPORT) && n.top >= 1.0 && n.bottom <= 0.0 {
            return Some((dx, dz));
        }
    }
    None
}
