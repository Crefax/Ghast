//! Doors and fence gates: which side of its cell a door's slab is on, and the block changes a
//! right click makes.
//!
//! A door's collision is one 3/16-thick full-height slab along a side of its cell (closed: the
//! side opposite its facing; open: the side its hinge swings it to). A player at the cell
//! centre does not touch it, and it blocks only moves across that side ([`crate::class::flag::PANEL`]).
//! 1.16.5 and 26.x states carry facing, open and hinge, so their classes are exact. In 1.8.9 /
//! 1.12.2 each half holds only part of the state (the lower half facing and open, the upper
//! half the hinge): the planner's grid combines both halves ([`resolve_legacy`]), as vanilla
//! does for the collision box.

use crate::Version;
use crate::class::{Class, Table, flag, table};
use crate::world::BlockSource;

/// Wall codes ([`Class::wall`]) of the four sides: north (-z), south (+z), west (-x), east (+x).
const NORTH: u8 = 1;
const SOUTH: u8 = 2;
const WEST: u8 = 3;
const EAST: u8 = 4;

/// The wall code of the side a horizontal unit offset points to (0 for anything else).
#[inline]
pub(crate) fn wall_of(dx: i32, dz: i32) -> u8 {
    match (dx, dz) {
        (0, -1) => NORTH,
        (0, 1) => SOUTH,
        (-1, 0) => WEST,
        (1, 0) => EAST,
        _ => 0,
    }
}

/// The synthetic door-slab classes of the legacy tables: index `openable * 4 + wall - 1`.
pub(crate) fn panel_classes() -> [Class; 8] {
    std::array::from_fn(|i| {
        let openable = i >= 4;
        Class {
            flags: flag::SOLID
                | flag::THIN
                | flag::PANEL
                | if openable { flag::OPENABLE } else { 0 },
            bottom: 0.0,
            top: 0.0,
            top_any: 1.0,
            hardness: -1.0,
            wall: (i % 4) as u8 + 1,
        }
    })
}

/// Legacy door ids: wooden doors and the iron door.
#[cfg(any(feature = "v1_8", feature = "v1_12_2"))]
#[inline]
fn legacy_door(id: u32) -> bool {
    matches!(id, 64 | 71 | 193..=197)
}

/// Legacy fence gate and wooden trapdoor ids (the open bit is 4 in both).
#[cfg(any(feature = "v1_8", feature = "v1_12_2"))]
#[inline]
fn legacy_gate(id: u32) -> bool {
    matches!(id, 96 | 107 | 183..=187)
}

/// Door direction from the lower half's metadata bits 0-1: east, south, west, north.
const DOOR_FACING: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// The class index of a 1.8.9 / 1.12.2 door half at (x, y, z) with raw state `raw`, combined
/// with the other half (`None` when the other half is unknown: the caller keeps the half's
/// own, irregular class).
pub(crate) fn resolve_legacy<S: BlockSource + ?Sized>(
    t: &Table,
    src: &S,
    [x, y, z]: [i32; 3],
    raw: u32,
) -> Option<u16> {
    let id = raw >> 4;
    let meta = raw & 15;
    let upper = meta & 8 != 0;
    let other = src.raw_block(x, if upper { y - 1 } else { y + 1 }, z)?;
    // Each half's own properties, the others at the values vanilla reads for a lone half
    // (facing east, closed, right hinge).
    let props = |m: u32| -> ((i32, i32), bool, bool) {
        if m & 8 != 0 {
            ((1, 0), false, m & 1 != 0)
        } else {
            (DOOR_FACING[(m & 3) as usize], m & 4 != 0, true)
        }
    };
    let (mut facing, mut open, mut right) = props(meta);
    if other >> 4 == id {
        let (f, o, r) = props(other & 15);
        if upper {
            facing = f;
            open = o;
        } else {
            right = r;
        }
    }
    let side = if !open {
        (-facing.0, -facing.1)
    } else if right {
        (-facing.1, facing.0)
    } else {
        (facing.1, -facing.0)
    };
    let openable = id != 71;
    let wall = wall_of(side.0, side.1);
    Some(t.panels[usize::from(openable) * 4 + usize::from(wall) - 1])
}

/// The class at `p` as the planner's grid sees it (legacy doors combined with their other half).
pub(crate) fn class_at<S: BlockSource + ?Sized>(version: Version, src: &S, p: [i32; 3]) -> Class {
    let t = table(version);
    let Some(raw) = src.raw_block(p[0], p[1], p[2]) else {
        return Class::UNKNOWN;
    };
    let mut i = t.index(raw);
    if t.classes[i as usize].has(flag::DOOR_HALF)
        && let Some(r) = resolve_legacy(t, src, p, raw)
    {
        i = r;
    }
    t.classes[i as usize]
}

/// [`toggle`] for a modern version's state types.
#[cfg(any(feature = "v1_16_5", feature = "v26_1", feature = "v26_2"))]
macro_rules! toggle_modern {
    ($state:ident, $prop:ident, $src:expr, $p:expr, $raw:expr) => {{
        let flip = |r: u32| -> Option<u32> {
            let s = $state::from_id(u16::try_from(r).ok()?)?;
            let open = s.get($prop::Open)? == "true";
            let n = s.with($prop::Open, if open { "false" } else { "true" })?;
            Some(u32::from(n.id()))
        };
        let p = $p;
        let s = $state::from_id(u16::try_from($raw).ok()?)?;
        let mut out = vec![(p, flip($raw)?)];
        if let Some(h) = s
            .get($prop::Half)
            .filter(|h| *h == "upper" || *h == "lower")
        {
            let q = [p[0], if h == "upper" { p[1] - 1 } else { p[1] + 1 }, p[2]];
            if let Some(o) = $src.raw_block(q[0], q[1], q[2])
                && $state::from_id(u16::try_from(o).ok()?).is_some_and(|t| t.block() == s.block())
                && let Some(n) = flip(o)
            {
                out.push((q, n));
            }
        }
        Some(out)
    }};
}

/// The class the block at `p` has once clicked (`None`: nothing a hand opens).
pub(crate) fn toggled_class<S: BlockSource + ?Sized>(
    version: Version,
    src: &S,
    p: [i32; 3],
) -> Option<Class> {
    let raw = toggle(version, src, p)?
        .into_iter()
        .find(|(q, _)| *q == p)?
        .1;
    let t = table(version);
    Some(t.classes[t.index(raw) as usize])
}

/// The block changes a right click on the door or fence gate at `p` makes (the server's
/// answer): both halves of a modern door, the lower half of a legacy one. `None` when `p` is
/// not something a hand opens.
pub(crate) fn toggle<S: BlockSource + ?Sized>(
    version: Version,
    src: &S,
    p: [i32; 3],
) -> Option<Vec<([i32; 3], u32)>> {
    let raw = src.raw_block(p[0], p[1], p[2])?;
    if !class_at(version, src, p).has(flag::OPENABLE) {
        return None;
    }
    match version {
        #[cfg(feature = "v1_8")]
        Version::V1_8 => toggle_legacy(src, p, raw),
        #[cfg(feature = "v1_12_2")]
        Version::V1_12 => toggle_legacy(src, p, raw),
        #[cfg(feature = "v1_16_5")]
        Version::V1_16_5 => {
            use ghast_data::v1_16_5::{BlockState, Property};
            toggle_modern!(BlockState, Property, src, p, raw)
        }
        #[cfg(feature = "v26_1")]
        Version::V26_1 => {
            use ghast_data::v26_1::{BlockState, Property};
            toggle_modern!(BlockState, Property, src, p, raw)
        }
        #[cfg(feature = "v26_2")]
        Version::V26_2 => {
            use ghast_data::v26_2::{BlockState, Property};
            toggle_modern!(BlockState, Property, src, p, raw)
        }
    }
}

/// [`toggle`] for 1.8.9 / 1.12.2: the open bit (4) of a gate or of a door's lower half.
#[cfg(any(feature = "v1_8", feature = "v1_12_2"))]
fn toggle_legacy<S: BlockSource + ?Sized>(
    src: &S,
    p: [i32; 3],
    raw: u32,
) -> Option<Vec<([i32; 3], u32)>> {
    let id = raw >> 4;
    if legacy_gate(id) {
        return Some(vec![(p, raw ^ 4)]);
    }
    if !legacy_door(id) {
        return None;
    }
    if raw & 8 == 0 {
        return Some(vec![(p, raw ^ 4)]);
    }
    let q = [p[0], p[1] - 1, p[2]];
    let lower = src.raw_block(q[0], q[1], q[2])?;
    (lower >> 4 == id).then(|| vec![(q, lower ^ 4)])
}
