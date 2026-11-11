//! 1.12.2 crosshair ray shapes: the box a block's ray test uses (vanilla
//! `Block.getBoundingBox(state, world, pos)`, Legacy Yarn `BlockState.getCollisionBox(view, pos)`),
//! including the neighbour-dependent ones (fences, panes, walls, chorus plants from their actual
//! connections, doors and chests as their collision box), and stairs, whose ray test runs per
//! collision box of the actual corner shape.

use ghast_data::v1_12::{BlockState, Face, FaceShape, StateFlags};

use super::blocks::{
    ShapeKind, boxes_of, face_shape, fence_links, id, pane_links, shape_kind, state_at, wall_links,
};
use crate::math::Aabb;
use crate::world::BlockView;

/// What a ray tests against at one block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RayShape {
    /// The block is skipped by the ray (vanilla `canCollideCheck` false for this ray kind).
    None,
    /// The ray considers the block but never hits it (moving piston: its ray test returns null).
    Never,
    /// One box, block-local.
    Bounds(Aabb),
    /// Stairs: one ray test per collision box (block-local, in collision order); vanilla keeps the
    /// hit farthest from the ray's end (strictly; ties keep the earlier box).
    Boxes {
        /// The boxes (`len` used).
        boxes: [Aabb; 4],
        /// Number of boxes.
        len: u8,
    },
}

fn local(b: [f64; 6]) -> RayShape {
    RayShape::Bounds(Aabb::new(b[0], b[1], b[2], b[3], b[4], b[5]))
}

/// The ray shape of the block at `(x, y, z)`. `liquids` = the ray stops at liquids (vanilla's
/// `canCollideCheck(state, true)`).
pub fn ray_shape<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    liquids: bool,
) -> RayShape {
    let s = state_at(w, x, y, z);
    let sel = s.selection();
    if !(if liquids {
        sel.can_collide_liquid
    } else {
        sel.can_collide
    }) {
        return RayShape::None;
    }
    if s.block().id() == id::PISTON_EXTENSION {
        return RayShape::Never;
    }
    match s.block().id() {
        55 => return RayShape::Bounds(wire_bounds(w, x, y, z)),
        106 => return vine_bounds(w, x, y, z, s),
        107 | 183..=187 => return gate_bounds(w, x, y, z, s),
        _ => {}
    }
    // Flowers (yellow, red): the plant box moved by the per-position model offset (XZ).
    if matches!(s.block().id(), 37 | 38) {
        let [ox, oy, oz] = flower_offset(x, z);
        let lo = f64::from(0.3_f32);
        let hi = f64::from(0.7_f32);
        return RayShape::Bounds(Aabb::new(
            lo + ox,
            0.0 + oy,
            lo + oz,
            hi + ox,
            f64::from(0.6_f32) + oy,
            hi + oz,
        ));
    }
    match shape_kind(s.block().id()) {
        ShapeKind::Fence => {
            let [n, e, so, we] = fence_links(w, x, y, z, s);
            local([
                if we { 0.0 } else { 0.375 },
                0.0,
                if n { 0.0 } else { 0.375 },
                if e { 1.0 } else { 0.625 },
                1.0,
                if so { 1.0 } else { 0.625 },
            ])
        }
        ShapeKind::Pane => {
            let [n, e, so, we] = pane_links(w, x, y, z);
            local([
                if we { 0.0 } else { 0.4375 },
                0.0,
                if n { 0.0 } else { 0.4375 },
                if e { 1.0 } else { 0.5625 },
                1.0,
                if so { 1.0 } else { 0.5625 },
            ])
        }
        ShapeKind::Wall => {
            let [n, e, so, we] = wall_links(w, x, y, z);
            let mut b = [
                if we { 0.0 } else { 0.25 },
                0.0,
                if n { 0.0 } else { 0.25 },
                if e { 1.0 } else { 0.75 },
                1.0,
                if so { 1.0 } else { 0.75 },
            ];
            if n && so && !we && !e {
                (b[0], b[3], b[4]) = (0.3125, 0.6875, 0.875);
            } else if we && e && !n && !so {
                (b[2], b[5], b[4]) = (0.3125, 0.6875, 0.875);
            }
            local(b)
        }
        ShapeKind::ChorusPlant => {
            let chorus = |dx: i32, dy: i32, dz: i32| {
                let n = state_at(w, x + dx, y + dy, z + dz).block().id();
                n == id::CHORUS_PLANT || n == id::CHORUS_FLOWER || (dy == -1 && n == id::END_STONE)
            };
            const A: f64 = 0.1875;
            const B: f64 = 0.8125;
            local([
                if chorus(-1, 0, 0) { 0.0 } else { A },
                if chorus(0, -1, 0) { 0.0 } else { A },
                if chorus(0, 0, -1) { 0.0 } else { A },
                if chorus(1, 0, 0) { 1.0 } else { B },
                if chorus(0, 1, 0) { 1.0 } else { B },
                if chorus(0, 0, 1) { 1.0 } else { B },
            ])
        }
        kind @ (ShapeKind::Door | ShapeKind::Chest | ShapeKind::Stairs) => {
            // The bounding box is the (single) collision box for doors and chests; stairs test
            // every collision box of the actual shape.
            let (fx, fy, fz) = (f64::from(x), f64::from(y), f64::from(z));
            let all = boxes_of(w, x, y, z);
            let mut boxes = [Aabb::FULL; 4];
            let mut len = 0u8;
            for b in all.iter().take(4) {
                boxes[len as usize] = Aabb::new(
                    b.min_x - fx,
                    b.min_y - fy,
                    b.min_z - fz,
                    b.max_x - fx,
                    b.max_y - fy,
                    b.max_z - fz,
                );
                len += 1;
            }
            if kind == ShapeKind::Stairs {
                RayShape::Boxes { boxes, len }
            } else if len > 0 {
                RayShape::Bounds(boxes[0])
            } else {
                RayShape::Bounds(sel.bounds)
            }
        }
        _ => RayShape::Bounds(sel.bounds),
    }
}

/// `Block.getOffset` for offset type XZ (`MathHelper.hashCode(x, 0, z)`).
fn flower_offset(x: i32, z: i32) -> [f64; 3] {
    let mut l = i64::from(x.wrapping_mul(3_129_871)) ^ i64::from(z).wrapping_mul(116_129_781);
    l = l
        .wrapping_mul(l)
        .wrapping_mul(42_317_861)
        .wrapping_add(l.wrapping_mul(11));
    let c = |shift: u32| (f64::from(((l >> shift) & 15) as f32 / 15.0) - 0.5) * 0.5;
    [c(16), 0.0, c(24)]
}

/// Redstone wire: `REDSTONE_WIRE_AABB[index of the actual connections]`.
fn wire_bounds<W: BlockView<State = BlockState>>(w: &W, x: i32, y: i32, z: i32) -> Aabb {
    // Horizontal directions with their offsets, in the order north, east, south, west.
    let dirs: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
    let at = |dx: i32, dy: i32, dz: i32| state_at(w, x + dx, y + dy, z + dz);
    let connected = |(dx, dz): (i32, i32)| -> bool {
        let n = at(dx, 0, dz);
        if connects_to(n, Some((dx, 0, dz))) {
            return true;
        }
        if !(n.flags().contains(StateFlags::NORMAL_CUBE) || !is_wire(at(dx, -1, dz))) {
            return true;
        }
        if at(0, 1, 0).flags().contains(StateFlags::NORMAL_CUBE) {
            return false;
        }
        let top = n.flags().contains(StateFlags::TOP_SOLID) || n.block().id() == 89;
        top && is_wire(at(dx, 1, dz))
    };
    let [n, e, so, we] = dirs.map(connected);
    let mut i = 0usize;
    // Horizontal indexes: south 0, west 1, north 2, east 3.
    if n || so && !n && !e && !we {
        i |= 1 << 2;
    }
    if e || we && !n && !e && !so {
        i |= 1 << 3;
    }
    if so || n && !e && !so && !we {
        i |= 1;
    }
    if we || e && !n && !so && !we {
        i |= 1 << 1;
    }
    let lo = |bit: usize, v: f64| if i & bit != 0 { 0.0 } else { v };
    let hi = |bit: usize, v: f64| if i & bit != 0 { 1.0 } else { v };
    // field_12731[i]: west (bit 2) lowers min x, east (8) raises max x, north (4) lowers min z,
    // south (1) raises max z.
    Aabb::new(
        lo(2, 0.1875),
        0.0,
        lo(4, 0.1875),
        hi(8, 0.8125),
        0.0625,
        hi(1, 0.8125),
    )
}

#[inline]
fn is_wire(s: BlockState) -> bool {
    s.block().id() == 55
}

/// `RedstoneWireBlock.connectsTo(state, dir)` (`dir` = from the wire towards the block).
fn connects_to(s: BlockState, dir: Option<(i32, i32, i32)>) -> bool {
    let id = s.block().id();
    if id == 55 {
        return true;
    }
    let horizontal = |meta: u8| [(0, 0, 1), (-1, 0, 0), (0, 0, -1), (1, 0, 0)][(meta & 3) as usize];
    match id {
        // Repeaters: along their facing axis.
        93 | 94 => {
            let f = horizontal(s.meta());
            dir.is_some_and(|d| d == f || d == (-f.0, -f.1, -f.2))
        }
        // Observer: facing (getById(meta & 7), 6 and 7 wrap to down / up).
        218 => {
            let f = [
                (0, -1, 0),
                (0, 1, 0),
                (0, 0, -1),
                (0, 0, 1),
                (-1, 0, 0),
                (1, 0, 0),
            ][((s.meta() & 7) % 6) as usize];
            dir == Some(f)
        }
        _ => dir.is_some() && emits_power(id),
    }
}

/// `Block.emitsRedstonePower` (1.12.2: the power sources).
fn emits_power(id: u8) -> bool {
    matches!(
        id,
        28 | 69
            | 70
            | 72
            | 75
            | 76
            | 77
            | 93
            | 94
            | 131
            | 143
            | 146
            | 147
            | 148
            | 149
            | 150
            | 151
            | 152
            | 178
            | 218
    )
}

/// Vine: the face box when exactly one face is attached (the top counts when the block above has a
/// solid bottom), else the full cube.
fn vine_bounds<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
) -> RayShape {
    let above = state_at(w, x, y + 1, z);
    let up = face_shape(w, x, y + 1, z, above, Face::Down) == FaceShape::Solid;
    let m = s.meta();
    let (south, west, north, east) = (m & 1 != 0, m & 2 != 0, m & 4 != 0, m & 8 != 0);
    let mut n = 0;
    let mut b = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    if up {
        b = [0.0, 0.9375, 0.0, 1.0, 1.0, 1.0];
        n += 1;
    }
    if north {
        b = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0625];
        n += 1;
    }
    if east {
        b = [0.9375, 0.0, 0.0, 1.0, 1.0, 1.0];
        n += 1;
    }
    if south {
        b = [0.0, 0.0, 0.9375, 1.0, 1.0, 1.0];
        n += 1;
    }
    if west {
        b = [0.0, 0.0, 0.0, 0.0625, 1.0, 1.0];
        n += 1;
    }
    if n != 1 {
        b = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    }
    local(b)
}

/// Fence gate: lowered (0.8125) when a cobblestone wall is beside it along its axis.
fn gate_bounds<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
) -> RayShape {
    // Direction.fromHorizontal(meta): south, west, north, east; south / north = Z axis.
    let z_axis = s.meta() & 1 == 0;
    let wall = |dx: i32, dz: i32| state_at(w, x + dx, y, z + dz).block().id() == 139;
    let in_wall = if z_axis {
        wall(-1, 0) || wall(1, 0)
    } else {
        wall(0, -1) || wall(0, 1)
    };
    let h = if in_wall { 0.8125 } else { 1.0 };
    if z_axis {
        local([0.0, 0.0, 0.375, 1.0, h, 0.625])
    } else {
        local([0.375, 0.0, 0.0, 0.625, h, 1.0])
    }
}
