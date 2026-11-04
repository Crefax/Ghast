//! Voxel-grid collision shapes as used from 1.14 on (vanilla `VoxelShape`).
//!
//! A shape is a 3-D grid of cells, each filled or empty, whose cell boundaries ("points") are
//! either `i / n` fractions (vanilla's simple, power-of-two shapes) or explicit coordinates (union
//! shapes, offset shapes). Collision does not treat a shape as a list of boxes: the distance query
//! scans grid layers and stops at the first filled one, with vanilla's `1e-7` tolerances, so the
//! grid itself (including boundaries that no single box would have) decides the result.

use crate::math::Aabb;

/// Tolerance vanilla uses in shape queries.
pub const EPSILON: f64 = 1.0e-7;

/// Cell boundaries along one axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Points<'a> {
    /// `n` equal cells: boundary `i` is `i / n`.
    Fractional(u32),
    /// Explicit boundaries (`len = cells + 1`, ascending).
    Explicit(&'a [f64]),
    /// Explicit boundaries plus a shift (vanilla's shape `offset`: `point + shift`).
    Shifted(&'a [f64], f64),
}

impl Points<'_> {
    /// Number of cells.
    #[inline]
    #[must_use]
    pub fn cells(&self) -> usize {
        match *self {
            Points::Fractional(n) => n as usize,
            Points::Explicit(p) | Points::Shifted(p, _) => p.len().saturating_sub(1),
        }
    }

    /// Boundary `i` (vanilla `DoubleList.getDouble`).
    #[inline]
    #[must_use]
    pub fn get(&self, i: usize) -> f64 {
        match *self {
            Points::Fractional(n) => i as f64 / f64::from(n),
            Points::Explicit(p) => p[i],
            Points::Shifted(p, k) => p[i] + k,
        }
    }
}

/// A collision shape: per-axis boundaries and the filled-cell bit set.
///
/// `cells` is indexed `(x * ny + y) * nz + z` (bit `i % 64` of word `i / 64`), vanilla's
/// `BitSetVoxelSet` order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelShape<'a> {
    /// Boundaries along X, Y, Z.
    pub axes: [Points<'a>; 3],
    /// Filled cells.
    pub cells: &'a [u64],
}

/// An axis index: 0 = X, 1 = Y, 2 = Z.
pub type Axis = usize;

#[inline]
fn lo(b: &Aabb, axis: Axis) -> f64 {
    [b.min_x, b.min_y, b.min_z][axis]
}

#[inline]
fn hi(b: &Aabb, axis: Axis) -> f64 {
    [b.max_x, b.max_y, b.max_z][axis]
}

impl<'a> VoxelShape<'a> {
    /// The unit cube, one filled cell (vanilla `VoxelShapes.fullCube()`).
    pub const FULL_CUBE: VoxelShape<'static> = VoxelShape {
        axes: [Points::Fractional(1); 3],
        cells: &[1],
    };

    /// `true` for a "simple" shape (all axes fractional), whose coordinate lookup rounds with
    /// `floor(coord * n)` instead of searching the boundaries.
    #[inline]
    fn simple(&self) -> bool {
        self.axes.iter().all(|p| matches!(p, Points::Fractional(_)))
    }

    #[inline]
    fn size(&self, axis: Axis) -> usize {
        self.axes[axis].cells()
    }

    /// `true` when cell `(x, y, z)` exists and is filled.
    #[inline]
    #[must_use]
    pub fn filled(&self, x: isize, y: isize, z: isize) -> bool {
        let (nx, ny, nz) = (self.size(0), self.size(1), self.size(2));
        if x < 0 || y < 0 || z < 0 {
            return false;
        }
        let (x, y, z) = (x as usize, y as usize, z as usize);
        if x >= nx || y >= ny || z >= nz {
            return false;
        }
        let i = (x * ny + y) * nz + z;
        self.cells
            .get(i / 64)
            .is_some_and(|w| w >> (i % 64) & 1 != 0)
    }

    /// `true` when no cell is filled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let n = self.size(0) * self.size(1) * self.size(2);
        let full = n / 64;
        self.cells[..full.min(self.cells.len())]
            .iter()
            .all(|&w| w == 0)
            && (n.is_multiple_of(64)
                || self
                    .cells
                    .get(full)
                    .is_none_or(|&w| w & ((1u64 << (n % 64)) - 1) == 0))
    }

    /// Boundary `i` on an axis, with an optional placement offset added (vanilla
    /// `OffsetDoubleList`: `point + offset`; without an offset nothing is added).
    #[inline]
    fn point(&self, axis: Axis, i: usize, offset: Option<[f64; 3]>) -> f64 {
        let p = self.axes[axis].get(i);
        match offset {
            Some(o) => p + o[axis],
            None => p,
        }
    }

    /// Index of the cell containing `coord`: `-1` below the first boundary, `size` at or above the
    /// last (vanilla `getCoordIndex`).
    fn coord_index(&self, axis: Axis, coord: f64, offset: Option<[f64; 3]>) -> isize {
        let n = self.size(axis);
        if offset.is_none() && self.simple() {
            // floor(coord * n) clamped to [-1, n] (SimpleVoxelShape).
            let v = crate::math::floor(coord * n as f64);
            return (v as isize).clamp(-1, n as isize);
        }
        // First boundary strictly above coord, searched over 0..=n (n+1 when none), minus one.
        let (mut start, mut len) = (0usize, n + 1);
        while len > 0 {
            let half = len / 2;
            let k = start + half;
            let above = k > n || coord < self.point(axis, k, offset);
            if above {
                len = half;
            } else {
                start = k + 1;
                len -= half + 1;
            }
        }
        start as isize - 1
    }

    /// Limits a movement `max` of box `b` along `axis` so it stops at this shape (vanilla
    /// `calculateMaxDistance`). `offset` places the shape (`None` = shape at the origin).
    #[must_use]
    pub fn max_distance(&self, axis: Axis, b: &Aabb, max: f64, offset: Option<[f64; 3]>) -> f64 {
        if self.is_empty() {
            return max;
        }
        if max.abs() < EPSILON {
            return 0.0;
        }
        let (a2, a3) = ((axis + 1) % 3, (axis + 2) % 3);
        let (bmax, bmin) = (hi(b, axis), lo(b, axis));
        let i = self.coord_index(axis, bmin + EPSILON, offset);
        let j = self.coord_index(axis, bmax - EPSILON, offset);
        let k = self.coord_index(a2, lo(b, a2) + EPSILON, offset).max(0);
        let l = (self.coord_index(a2, hi(b, a2) - EPSILON, offset) + 1).min(self.size(a2) as isize);
        let m = self.coord_index(a3, lo(b, a3) + EPSILON, offset).max(0);
        let n = (self.coord_index(a3, hi(b, a3) - EPSILON, offset) + 1).min(self.size(a3) as isize);
        let o = self.size(axis) as isize;
        let cell = |p: isize, q: isize, r: isize| {
            let mut c = [0isize; 3];
            c[axis] = p;
            c[a2] = q;
            c[a3] = r;
            self.filled(c[0], c[1], c[2])
        };
        let layer = |p: isize| (k..l).any(|q| (m..n).any(|r| cell(p, q, r)));
        // The first filled layer in the direction of travel decides the result.
        if max > 0.0 {
            let Some(p) = (j + 1..o).find(|&p| layer(p)) else {
                return max;
            };
            let gap = self.point(axis, p as usize, offset) - bmax;
            if gap >= -EPSILON { max.min(gap) } else { max }
        } else if max < 0.0 {
            let Some(s) = (0..i).rev().find(|&s| layer(s)) else {
                return max;
            };
            let gap = self.point(axis, (s + 1) as usize, offset) - bmin;
            if gap <= EPSILON { max.max(gap) } else { max }
        } else {
            max
        }
    }

    /// `true` when a filled cell overlaps `b` (vanilla `matchesAnywhere(shape, cuboid(b), AND)`:
    /// both grids are merged with a `1e-7` tolerance, so boundaries closer than that coincide).
    #[must_use]
    pub fn intersects(&self, b: &Aabb, offset: Option<[f64; 3]>) -> bool {
        // `VoxelShapes.cuboid` turns a box thinner than the tolerance into the empty shape.
        if (0..3).any(|axis| hi(b, axis) - lo(b, axis) < EPSILON) || self.is_empty() {
            return false;
        }
        // (Vanilla first rejects shapes more than 1e-7 apart; the merged test below gives the same
        // answer for them.)
        let merged = |axis: Axis| {
            let k = self.size(axis) + 1;
            let first = |i: usize| self.point(axis, i, offset);
            merge_points(k, first, [lo(b, axis), hi(b, axis)])
        };
        // Only merged cells inside the box (its single cell, index 0) matter.
        let (mx, my, mz) = (merged(0), merged(1), merged(2));
        mx.cells().iter().filter(|c| c.1 == 0).any(|&(x1, _)| {
            my.cells().iter().filter(|c| c.1 == 0).any(|&(y1, _)| {
                mz.cells()
                    .iter()
                    .filter(|c| c.1 == 0)
                    .any(|&(z1, _)| self.filled(x1, y1, z1))
            })
        })
    }
}

/// Merged cells of one axis: `(cell index in the shape, cell index in the box)`.
struct Merged {
    buf: [(isize, isize); MAX_MERGED],
    len: usize,
}

/// Enough for every vanilla block shape (at most 16 cells per axis) plus the box's two points.
const MAX_MERGED: usize = 64;

impl Merged {
    fn cells(&self) -> &[(isize, isize)] {
        &self.buf[..self.len]
    }
}

/// Vanilla `SimplePairList` for an AND query: merges the shape's ascending boundaries with the
/// box's two (boundaries within `1e-7` collapse) and lists, per merged cell, the cell index in
/// each (`-1` / out of range where one has no cell).
fn merge_points(k: usize, first: impl Fn(usize) -> f64, second: [f64; 2]) -> Merged {
    let l = second.len();
    let (mut i, mut j) = (0usize, 0usize);
    let mut d = f64::NAN;
    let mut out = Merged {
        buf: [(0, 0); MAX_MERGED],
        len: 0,
    };
    loop {
        let bl = i < k;
        let bl2 = j < l;
        if !bl && !bl2 {
            break;
        }
        let take_first = bl && (!bl2 || first(i) < second[j] + EPSILON);
        let e = if take_first {
            i += 1;
            first(i - 1)
        } else {
            j += 1;
            second[j - 1]
        };
        if (i != 0 && bl || take_first) && (j != 0 && bl2 || !take_first) {
            #[allow(clippy::neg_cmp_op_on_partial_ord)]
            let new_value = !(d >= e - EPSILON);
            let entry = (i as isize - 1, j as isize - 1);
            if new_value {
                if out.len < MAX_MERGED {
                    out.buf[out.len] = entry;
                }
                out.len += 1;
                d = e;
            } else if out.len > 0 && out.len <= MAX_MERGED {
                out.buf[out.len - 1] = entry;
            }
        }
    }
    // Cell n lies between merged values n and n + 1: the last value starts no cell.
    out.len = out.len.saturating_sub(1).min(MAX_MERGED);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLAB: VoxelShape<'static> = VoxelShape {
        axes: [
            Points::Fractional(1),
            Points::Fractional(2),
            Points::Fractional(1),
        ],
        cells: &[0b01],
    };

    #[test]
    fn slab_stops_falling_box() {
        let b = Aabb::new(0.2, 1.0, 0.2, 0.8, 2.8, 0.8);
        assert_eq!(SLAB.max_distance(1, &b, -1.0, None), -0.5);
        assert_eq!(SLAB.max_distance(1, &b, -0.3, None), -0.3);
        let off = Some([10.0, 64.0, 10.0]);
        let b = Aabb::new(10.2, 65.0, 10.2, 10.8, 66.8, 10.8);
        assert_eq!(SLAB.max_distance(1, &b, -1.0, off), -0.5);
        assert!(SLAB.intersects(&Aabb::new(10.0, 64.4, 10.0, 11.0, 65.0, 11.0), off));
        assert!(!SLAB.intersects(&Aabb::new(10.0, 64.5, 10.0, 11.0, 65.0, 11.0), off));
    }

    #[test]
    fn full_cube_side() {
        let b = Aabb::new(-0.7, 0.0, 0.2, -0.1, 1.8, 0.8);
        assert_eq!(VoxelShape::FULL_CUBE.max_distance(0, &b, 0.5, None), 0.1);
    }
}
