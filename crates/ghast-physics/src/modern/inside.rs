//! Inside-block effects of the modern client (`Entity.checkInsideBlocks`): the blocks the box
//! sweeps through during the tick's movements, visited in vanilla's order
//! (`BlockGetter.forEachBlockIntersectedBetween`), deduplicated, each applying its effect.
//!
//! Only blocks with a movement effect are acted on (cobweb, sweet berry bush, powder snow, honey,
//! bubble column); every other block is skipped after its position is visited, which keeps the
//! iteration order and limits exact.

use super::math::{V3, clamp, floor, frac, sign};
use crate::math::Aabb;

/// One recorded movement (`Entity.Movement`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Movement {
    pub from: V3,
    pub to: V3,
    /// The requested movement (decides the axis order), absent for the fallback movement.
    pub axis_dependent: Option<V3>,
}

/// `Direction.axisStepOrder`: Y first, then the smaller of X / Z.
#[inline]
pub(crate) fn axis_order(v: V3) -> [usize; 3] {
    if v.x.abs() < v.z.abs() {
        [1, 2, 0]
    } else {
        [1, 0, 2]
    }
}

/// `BlockPos.betweenCornersInDirection(c0, c1, direction)`: visits the inclusive box from the
/// corner the direction starts at, along the axis order of the direction (third axis fastest).
pub(crate) fn between_corners(
    c0: [i32; 3],
    c1: [i32; 3],
    dir: V3,
    mut f: impl FnMut([i32; 3]) -> bool,
) -> bool {
    let mn = [c0[0].min(c1[0]), c0[1].min(c1[1]), c0[2].min(c1[2])];
    let mx = [c0[0].max(c1[0]), c0[1].max(c1[1]), c0[2].max(c1[2])];
    let d = dir.arr();
    let start = [0, 1, 2].map(|a| if d[a] >= 0.0 { mn[a] } else { mx[a] });
    let step = [0, 1, 2].map(|a| if d[a] >= 0.0 { 1 } else { -1 });
    let diff = [0, 1, 2].map(|a| mx[a] - mn[a]);
    let [a1, a2, a3] = axis_order(dir);
    for i in 0..=diff[a1] {
        for j in 0..=diff[a2] {
            for k in 0..=diff[a3] {
                let mut p = start;
                p[a1] += step[a1] * i;
                p[a2] += step[a2] * j;
                p[a3] += step[a3] * k;
                if !f(p) {
                    return false;
                }
            }
        }
    }
    true
}

/// `BlockGetter.forEachBlockIntersectedBetween(from, to, boxAtTarget, visitor)`; the visitor gets
/// `(pos, iteration)` and returns `false` to stop. Returns `false` when stopped.
pub(crate) fn for_each_block_between(
    from: V3,
    to: V3,
    at_target: &Aabb,
    mut visit: impl FnMut([i32; 3], i32) -> bool,
) -> bool {
    let travel = to.sub(from);
    let tiny = f64::from(1.0e-5f32);
    if travel.length_sqr() < tiny * tiny {
        // BlockPos.betweenClosed(box): x fastest, then y, then z.
        let (x0, y0, z0) = (
            floor(at_target.min_x),
            floor(at_target.min_y),
            floor(at_target.min_z),
        );
        let (x1, y1, z1) = (
            floor(at_target.max_x),
            floor(at_target.max_y),
            floor(at_target.max_z),
        );
        for z in z0.min(z1)..=z0.max(z1) {
            for y in y0.min(y1)..=y0.max(y1) {
                for x in x0.min(x1)..=x0.max(x1) {
                    if !visit([x, y, z], 0) {
                        return false;
                    }
                }
            }
        }
        return true;
    }
    let mut visited = Visited::default();
    let start = super::math::mv(at_target, -travel.x, -travel.y, -travel.z);
    let corners = |b: &Aabb| {
        (
            [floor(b.min_x), floor(b.min_y), floor(b.min_z)],
            [floor(b.max_x), floor(b.max_y), floor(b.max_z)],
        )
    };
    let (c0, c1) = corners(&start);
    let mut go = true;
    between_corners(c0, c1, travel, |p| {
        if !visit(p, 0) {
            go = false;
            return false;
        }
        visited.add(p);
        true
    });
    if !go {
        return false;
    }
    let Some(iterations) = along_travel(&mut visited, travel, at_target, &mut visit) else {
        return false;
    };
    let (c0, c1) = corners(at_target);
    between_corners(c0, c1, travel, |p| {
        if visited.add(p) && !visit(p, iterations + 1) {
            go = false;
            return false;
        }
        true
    })
}

/// `BlockGetter.addCollisionsAlongTravel`; `None` when the visitor stopped.
fn along_travel(
    visited: &mut Visited,
    d: V3,
    at_target: &Aabb,
    visit: &mut impl FnMut([i32; 3], i32) -> bool,
) -> Option<i32> {
    let size = [
        at_target.max_x - at_target.min_x,
        at_target.max_y - at_target.min_y,
        at_target.max_z - at_target.min_z,
    ];
    let corner = furthest_corner(d);
    let c = super::math::center(at_target);
    let to_corner = V3::new(
        c.x + size[0] * 0.5 * f64::from(corner[0]),
        c.y + size[1] * 0.5 * f64::from(corner[1]),
        c.z + size[2] * 0.5 * f64::from(corner[2]),
    );
    let from_corner = to_corner.sub(d);
    let mut cb = [
        floor(from_corner.x),
        floor(from_corner.y),
        floor(from_corner.z),
    ];
    let s = [sign(d.x), sign(d.y), sign(d.z)];
    let td = [0, 1, 2].map(|a| {
        if s[a] == 0 {
            f64::MAX
        } else {
            f64::from(s[a]) / d.get(a)
        }
    });
    let fc = from_corner.arr();
    let mut t = [0, 1, 2].map(|a| {
        td[a]
            * if s[a] > 0 {
                1.0 - frac(fc[a])
            } else {
                frac(fc[a])
            }
    });
    let mut iterations = 0;
    let eps = f64::from(1.0e-5f32);
    while t[0] <= 1.0 || t[1] <= 1.0 || t[2] <= 1.0 {
        if t[0] < t[1] {
            if t[0] < t[2] {
                cb[0] += s[0];
                t[0] += td[0];
            } else {
                cb[2] += s[2];
                t[2] += td[2];
            }
        } else if t[1] < t[2] {
            cb[1] += s[1];
            t[1] += td[1];
        } else {
            cb[2] += s[2];
            t[2] += td[2];
        }
        let cell = Aabb::new(
            f64::from(cb[0]),
            f64::from(cb[1]),
            f64::from(cb[2]),
            f64::from(cb[0] + 1),
            f64::from(cb[1] + 1),
            f64::from(cb[2] + 1),
        );
        let Some(hit) = super::math::clip_box(&cell, from_corner, to_corner) else {
            continue;
        };
        iterations += 1;
        let hc = [
            clamp(hit.x, f64::from(cb[0]) + eps, f64::from(cb[0]) + 1.0 - eps),
            clamp(hit.y, f64::from(cb[1]) + eps, f64::from(cb[1]) + 1.0 - eps),
            clamp(hit.z, f64::from(cb[2]) + eps, f64::from(cb[2]) + 1.0 - eps),
        ];
        let opp = [0, 1, 2].map(|a| floor(hc[a] - size[a] * f64::from(corner[a])));
        let it = iterations;
        let mut stopped = false;
        between_corners(cb, opp, d, |p| {
            if visited.add(p) && !visit(p, it) {
                stopped = true;
                return false;
            }
            true
        });
        if stopped {
            return None;
        }
    }
    Some(iterations)
}

/// `BlockGetter.getFurthestCorner(direction)`.
fn furthest_corner(d: V3) -> [i32; 3] {
    let (xd, yd, zd) = (d.x.abs(), d.y.abs(), d.z.abs());
    let xs = if d.x >= 0.0 { 1 } else { -1 };
    let ys = if d.y >= 0.0 { 1 } else { -1 };
    let zs = if d.z >= 0.0 { 1 } else { -1 };
    if xd <= yd && xd <= zd {
        [-xs, -zs, ys]
    } else if yd <= zd {
        [zs, -ys, -xs]
    } else {
        [-ys, xs, -zs]
    }
}

/// A small set of block positions (a tick visits a few dozen at most; larger sweeps spill).
#[derive(Default)]
pub(crate) struct Visited {
    buf: Vec<[i32; 3]>,
}

impl Visited {
    /// Adds a position; `true` when it was new.
    pub(crate) fn add(&mut self, p: [i32; 3]) -> bool {
        if self.buf.contains(&p) {
            return false;
        }
        self.buf.push(p);
        true
    }

    pub(crate) fn clear(&mut self) {
        self.buf.clear();
    }
}
