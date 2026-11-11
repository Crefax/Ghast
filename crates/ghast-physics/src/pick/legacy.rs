//! The crosshair block pick of the pre-voxel-shape versions (1.8.9, 1.12.2): vanilla
//! `World.rayTrace` (the same block walk in both versions: plane steps towards the end block, at
//! most 200 of them, `-1e-4` for a zero step fraction) with `Block.rayTrace` per block (the six
//! face planes of one box, nearest to the ray's start wins; 1.12's `Box.calculateIntercept` is the
//! same test). Every vector is a vanilla `Vec3d`, whose constructor turns `-0.0` into `0.0`.
//!
//! The version modules supply the box(es) per block ([`crate::v1_8::ray_shape`],
//! [`crate::v1_12::ray_shape`]).

use super::{BlockHit, Direction, HitKind};
use crate::math::{cos, sin};

/// What the ray sees at one block.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // a per-block temporary on the stack
pub enum Shape {
    /// Rays skip the block (`canCollide` false): the walk records it as the last miss.
    Skip,
    /// The ray considers the block but never hits it (1.12 moving piston).
    Never,
    /// One block-local box.
    One(crate::math::Aabb),
    /// Several block-local boxes tested in order (stairs); the hit farthest from the ray's end
    /// wins (strictly; ties keep the earlier one). `len` boxes used.
    Many([crate::math::Aabb; 8], u8),
}

/// `new Vec3d(x, y, z)`: `-0.0` becomes `0.0`.
#[inline]
fn v3(x: f64, y: f64, z: f64) -> [f64; 3] {
    let n = |v: f64| if v == 0.0 { 0.0 } else { v };
    [n(x), n(y), n(z)]
}

#[inline]
fn sq(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (x, y, z) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    x * x + y * y + z * z
}

/// `Entity.getCameraPosVec(1.0F)`.
#[must_use]
pub fn eye_position(pos: [f64; 3], eye_height: f32) -> [f64; 3] {
    v3(pos[0], pos[1] + f64::from(eye_height), pos[2])
}

/// `Entity.getRotationVector(pitch, yaw)` (float sine table).
#[must_use]
pub fn look_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    const DEG: f32 = (std::f64::consts::PI / 180.0) as f32;
    const PI: f32 = std::f64::consts::PI as f32;
    let f = cos(-yaw * DEG - PI);
    let g = sin(-yaw * DEG - PI);
    let h = -cos(-pitch * DEG);
    let i = sin(-pitch * DEG);
    v3(f64::from(g * h), f64::from(i), f64::from(f * h))
}

/// `eye + look * reach` (`Vec3d.add`).
#[must_use]
pub fn ray_end(eye: [f64; 3], look: [f64; 3], reach: f64) -> [f64; 3] {
    v3(
        eye[0] + look[0] * reach,
        eye[1] + look[1] * reach,
        eye[2] + look[2] * reach,
    )
}

/// `Vec3d.lerpForX/Y/Z`: the point where the segment crosses `value` on `axis`, if within it.
#[inline]
fn lerp_for(s: [f64; 3], e: [f64; 3], axis: usize, value: f64) -> Option<[f64; 3]> {
    let d = [e[0] - s[0], e[1] - s[1], e[2] - s[2]];
    if d[axis] * d[axis] < f64::from(1.0e-7_f32) {
        return None;
    }
    let g = (value - s[axis]) / d[axis];
    // `!(g < 0) && !(g > 1)`: NaN passes.
    if g < 0.0 || g > 1.0 {
        return None;
    }
    Some(v3(s[0] + d[0] * g, s[1] + d[1] * g, s[2] + d[2] * g))
}

/// `Block.rayTrace` / `Block.rayTrace(pos, start, end, box)` for one block-local box: the hit
/// point (world coordinates) and face, nearest to `start`.
#[must_use]
pub fn box_ray(
    start: [f64; 3],
    end: [f64; 3],
    pos: [i32; 3],
    b: &crate::math::Aabb,
) -> Option<([f64; 3], Direction)> {
    let p = pos.map(f64::from);
    let s = v3(start[0] + -p[0], start[1] + -p[1], start[2] + -p[2]);
    let e = v3(end[0] + -p[0], end[1] + -p[1], end[2] + -p[2]);
    let yz =
        |v: &[f64; 3]| v[1] >= b.min_y && v[1] <= b.max_y && v[2] >= b.min_z && v[2] <= b.max_z;
    let xz =
        |v: &[f64; 3]| v[0] >= b.min_x && v[0] <= b.max_x && v[2] >= b.min_z && v[2] <= b.max_z;
    let xy =
        |v: &[f64; 3]| v[0] >= b.min_x && v[0] <= b.max_x && v[1] >= b.min_y && v[1] <= b.max_y;
    let candidates = [
        (lerp_for(s, e, 0, b.min_x).filter(yz), Direction::West),
        (lerp_for(s, e, 0, b.max_x).filter(yz), Direction::East),
        (lerp_for(s, e, 1, b.min_y).filter(xz), Direction::Down),
        (lerp_for(s, e, 1, b.max_y).filter(xz), Direction::Up),
        (lerp_for(s, e, 2, b.min_z).filter(xy), Direction::North),
        (lerp_for(s, e, 2, b.max_z).filter(xy), Direction::South),
    ];
    let mut best: Option<([f64; 3], Direction)> = None;
    for (c, face) in candidates {
        if let Some(c) = c
            && best.is_none_or(|(b, _)| sq(s, c) < sq(s, b))
        {
            best = Some((c, face));
        }
    }
    best.map(|(h, face)| (v3(h[0] + p[0], h[1] + p[1], h[2] + p[2]), face))
}

/// The block test of one position: one box, or the farthest-from-`end` hit of several.
fn test(shape: &Shape, start: [f64; 3], end: [f64; 3], pos: [i32; 3]) -> Option<BlockHit> {
    let hit = match shape {
        Shape::Skip | Shape::Never => None,
        Shape::One(b) => box_ray(start, end, pos, b),
        Shape::Many(boxes, len) => {
            let mut best = None;
            let mut best_d = 0.0;
            for b in &boxes[..usize::from(*len)] {
                if let Some((h, f)) = box_ray(start, end, pos, b) {
                    let d = sq(h, end);
                    if d > best_d {
                        best = Some((h, f));
                        best_d = d;
                    }
                }
            }
            best
        }
    };
    hit.map(|(location, face)| BlockHit {
        kind: HitKind::Block,
        pos,
        face,
        location,
        inside: false,
    })
}

/// `World.rayTrace(start, end, stopOnLiquid, false, returnLast)`: `shape(pos)` is what the ray
/// sees at each block (the version's shape lookup; `stopOnLiquid` is its business). `None` when
/// nothing is hit and `return_last` is off or nothing was skipped on the way.
pub fn trace(
    start: [f64; 3],
    end: [f64; 3],
    return_last: bool,
    mut shape: impl FnMut([i32; 3]) -> Shape,
) -> Option<BlockHit> {
    let nan = |v: [f64; 3]| v[0].is_nan() || v[1].is_nan() || v[2].is_nan();
    if nan(start) || nan(end) {
        return None;
    }
    let fl = |v: f64| v.floor() as i32;
    let (ei, ej, ek) = (fl(end[0]), fl(end[1]), fl(end[2]));
    let (mut l, mut m, mut n) = (fl(start[0]), fl(start[1]), fl(start[2]));
    let mut cur = start;
    if let Some(h) = test(&shape([l, m, n]), cur, end, [l, m, n]) {
        return Some(h);
    }
    let mut last: Option<BlockHit> = None;
    let mut steps = 200;
    loop {
        // while (steps-- >= 0)
        let go = steps >= 0;
        steps -= 1;
        if !go {
            break;
        }
        if nan(cur) {
            return None;
        }
        if l == ei && m == ej && n == ek {
            return if return_last { last } else { None };
        }
        let plane = |e: i32, c: i32| {
            if e > c {
                Some(f64::from(c) + 1.0)
            } else if e < c {
                Some(f64::from(c) + 0.0)
            } else {
                None
            }
        };
        let (pd, pe, pf) = (plane(ei, l), plane(ej, m), plane(ek, n));
        let (q, r, s) = (end[0] - cur[0], end[1] - cur[1], end[2] - cur[2]);
        let frac = |p: Option<f64>, o: f64, d: f64| {
            let t = p.map_or(999.0, |p| (p - o) / d);
            // `t == -0.0` is true for +0.0 too.
            if t == 0.0 { -1.0e-4 } else { t }
        };
        let (g, h, p) = (
            frac(pd, cur[0], q),
            frac(pe, cur[1], r),
            frac(pf, cur[2], s),
        );
        let face;
        if g < h && g < p {
            face = if ei > l {
                Direction::West
            } else {
                Direction::East
            };
            cur = v3(pd.unwrap_or(999.0), cur[1] + r * g, cur[2] + s * g);
        } else if h < p {
            face = if ej > m {
                Direction::Down
            } else {
                Direction::Up
            };
            cur = v3(cur[0] + q * h, pe.unwrap_or(999.0), cur[2] + s * h);
        } else {
            face = if ek > n {
                Direction::North
            } else {
                Direction::South
            };
            cur = v3(cur[0] + q * p, cur[1] + r * p, pf.unwrap_or(999.0));
        }
        l = fl(cur[0]) - i32::from(face == Direction::East);
        m = fl(cur[1]) - i32::from(face == Direction::Up);
        n = fl(cur[2]) - i32::from(face == Direction::South);
        let pos = [l, m, n];
        let sh = shape(pos);
        if sh == Shape::Skip {
            last = Some(BlockHit {
                kind: HitKind::Miss,
                pos,
                face,
                location: cur,
                inside: false,
            });
        } else if let Some(h) = test(&sh, cur, end, pos) {
            return Some(h);
        }
    }
    if return_last { last } else { None }
}
