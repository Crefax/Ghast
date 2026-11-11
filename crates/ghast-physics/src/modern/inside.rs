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
#[derive(Clone, Copy, Debug, PartialEq)]
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
    let start = super::math::mv(at_target, -travel.x, -travel.y, -travel.z);
    let mut visited = BoxSet::new(
        [
            floor(start.min_x.min(at_target.min_x)) - 1,
            floor(start.min_y.min(at_target.min_y)) - 1,
            floor(start.min_z.min(at_target.min_z)) - 1,
        ],
        [
            floor(start.max_x.max(at_target.max_x)) + 1,
            floor(start.max_y.max(at_target.max_y)) + 1,
            floor(start.max_z.max(at_target.max_z)) + 1,
        ],
    );
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
    visited: &mut BoxSet,
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

/// A small set of block positions: inline for the few dozen a tick visits, spilling to the heap
/// only for very long sweeps.
pub(crate) struct Visited {
    buf: [[i32; 3]; VISITED_INLINE],
    len: usize,
    spill: std::collections::HashSet<[i32; 3]>,
}

const VISITED_INLINE: usize = 64;

impl Default for Visited {
    fn default() -> Self {
        Visited {
            buf: [[0; 3]; VISITED_INLINE],
            len: 0,
            spill: std::collections::HashSet::new(),
        }
    }
}

impl Visited {
    /// Adds a position; `true` when it was new.
    pub(crate) fn add(&mut self, p: [i32; 3]) -> bool {
        if self.buf[..self.len].contains(&p) {
            return false;
        }
        if self.len < VISITED_INLINE {
            self.buf[self.len] = p;
            self.len += 1;
            true
        } else {
            self.spill.insert(p)
        }
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
        self.spill.clear();
    }
}

/// A set of block positions inside known bounds: a bit per cell when the bounds are small (the
/// usual sweep), a hash set otherwise.
pub(crate) struct BoxSet {
    lo: [i32; 3],
    dims: [i32; 3],
    bits: [u64; BOX_WORDS],
    big: Option<std::collections::HashSet<[i32; 3]>>,
}

const BOX_WORDS: usize = 16;

impl BoxSet {
    pub(crate) fn new(lo: [i32; 3], hi: [i32; 3]) -> BoxSet {
        let dims =
            [0, 1, 2].map(|a| (i64::from(hi[a]) - i64::from(lo[a]) + 1).clamp(0, 1 << 20) as i32);
        let cells = i64::from(dims[0]) * i64::from(dims[1]) * i64::from(dims[2]);
        BoxSet {
            lo,
            dims,
            bits: [0; BOX_WORDS],
            big: (cells > (BOX_WORDS * 64) as i64).then(std::collections::HashSet::new),
        }
    }

    /// Adds a position; `true` when it was new.
    pub(crate) fn add(&mut self, p: [i32; 3]) -> bool {
        if let Some(h) = &mut self.big {
            return h.insert(p);
        }
        let d = [0, 1, 2].map(|a| p[a].wrapping_sub(self.lo[a]));
        if (0..3).any(|a| d[a] < 0 || d[a] >= self.dims[a]) {
            // Outside the expected bounds (cannot happen for vanilla's sweep): fall back.
            let mut h = std::collections::HashSet::new();
            for i in 0..(self.dims[0] * self.dims[1] * self.dims[2]) as usize {
                if self.bits[i / 64] >> (i % 64) & 1 != 0 {
                    let i = i as i32;
                    let x = i % self.dims[0];
                    let y = i / self.dims[0] % self.dims[1];
                    let z = i / self.dims[0] / self.dims[1];
                    h.insert([self.lo[0] + x, self.lo[1] + y, self.lo[2] + z]);
                }
            }
            let new = h.insert(p);
            self.big = Some(h);
            return new;
        }
        let i = ((d[2] * self.dims[1] + d[1]) * self.dims[0] + d[0]) as usize;
        let (w, b) = (i / 64, 1u64 << (i % 64));
        let new = self.bits[w] & b == 0;
        self.bits[w] |= b;
        new
    }
}
