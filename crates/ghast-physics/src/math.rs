//! Java-exact numeric helpers.
//!
//! Vanilla mixes `float` and `double` arithmetic; every helper here documents which precision it
//! reproduces. Rust's `f32`/`f64` operations are IEEE-754 round-to-nearest like Java's (Java 17+ is
//! always strict), Rust never contracts `a * b + c` into a fused multiply-add, and `as` casts
//! float → int saturate and map NaN to 0 exactly like Java's `(int)` cast. What remains to get
//! right is *where* vanilla rounds, which the call sites in the rule modules spell out.

use std::sync::LazyLock;

pub use ghast_data::Aabb;

/// Number of entries in the vanilla sine table.
const SIN_TABLE_LEN: usize = 65536;

/// The 65536-entry sine table vanilla's `MathHelper` uses for every `sin`/`cos` in movement:
/// entry `i` is `(float) Math.sin(i * PI * 2 / 65536)`.
///
/// Computed once from `f64::sin`; `tests` pins its bit-exact hash against the table a JDK builds
/// (glibc / fdlibm results agree for all 65536 entries after rounding to `f32`).
static SIN_TABLE: LazyLock<Box<[f32; SIN_TABLE_LEN]>> = LazyLock::new(|| {
    let mut t = Box::new([0.0f32; SIN_TABLE_LEN]);
    for (i, v) in t.iter_mut().enumerate() {
        *v = (i as f64 * std::f64::consts::PI * 2.0 / SIN_TABLE_LEN as f64).sin() as f32;
    }
    t
});

/// The shared sine table (also the 26.x table: a JDK builds identical entries from
/// `i / 10430.378350470453`).
#[inline]
#[cfg_attr(not(feature = "v26_1"), allow(dead_code))]
pub(crate) fn sin_table() -> &'static [f32; SIN_TABLE_LEN] {
    &SIN_TABLE
}

/// Radians to sine-table index scale, `65536 / (2 PI)` as vanilla's float literal.
const SIN_SCALE: f32 = 10430.378;

/// Vanilla's table sine of a float angle in radians.
#[inline]
#[must_use]
pub fn sin(rad: f32) -> f32 {
    SIN_TABLE[((rad * SIN_SCALE) as i32 & 0xFFFF) as usize]
}

/// Vanilla's table cosine of a float angle in radians (quarter-turn offset into the sine table).
#[inline]
#[must_use]
pub fn cos(rad: f32) -> f32 {
    SIN_TABLE[((rad * SIN_SCALE + 16384.0) as i32 & 0xFFFF) as usize]
}

/// `MathHelper.sqrt(float)`: square root computed in double, rounded to float.
#[inline]
#[must_use]
pub fn sqrt_f32(v: f32) -> f32 {
    f64::from(v).sqrt() as f32
}

/// `MathHelper.sqrt(double)`: square root rounded to float.
#[inline]
#[must_use]
pub fn sqrt_f64_as_f32(v: f64) -> f32 {
    v.sqrt() as f32
}

/// `MathHelper.floor(double)`: truncate, then step down for negative fractions. Saturates like
/// Java's `(int)` cast for out-of-range values.
#[inline]
#[must_use]
pub fn floor(v: f64) -> i32 {
    let i = v as i32;
    if v < f64::from(i) { i - 1 } else { i }
}

/// Java `Math.min(double, double)`: NaN wins, and `-0.0` is smaller than `0.0`.
#[inline]
#[must_use]
pub fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.is_sign_negative() {
        return b;
    }
    if a <= b { a } else { b }
}

/// Java `Math.max(double, double)`: NaN wins, and `0.0` is larger than `-0.0`.
#[inline]
#[must_use]
pub fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return b;
    }
    if a >= b { a } else { b }
}

/// Builds a box the way vanilla's `Box` constructor does: each axis is sorted with Java's
/// `Math.min` / `Math.max`.
#[inline]
#[must_use]
pub fn sorted_box(x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> Aabb {
    Aabb::new(
        java_min(x0, x1),
        java_min(y0, y1),
        java_min(z0, z1),
        java_max(x0, x1),
        java_max(y0, y1),
        java_max(z0, z1),
    )
}

/// The box operations vanilla movement uses, with vanilla's exact arithmetic.
pub trait BoxOps {
    /// Translated copy.
    fn offset(&self, dx: f64, dy: f64, dz: f64) -> Aabb;
    /// Extends the box in the direction of each component (the swept box of a move).
    fn stretch(&self, dx: f64, dy: f64, dz: f64) -> Aabb;
    /// Grows every side by `(x, y, z)` (negative values shrink; the result is re-sorted).
    fn grow(&self, x: f64, y: f64, z: f64) -> Aabb;
    /// Shrinks every side by `(x, y, z)` (vanilla's `Box.increment`; the result is re-sorted).
    fn shrink(&self, x: f64, y: f64, z: f64) -> Aabb;
    /// Strict overlap test (touching faces do not intersect).
    fn intersects(&self, other: &Aabb) -> bool;
    /// Limits a movement `d` of `mover` along X so it stops at this box.
    fn clip_x(&self, mover: &Aabb, d: f64) -> f64;
    /// Limits a movement `d` of `mover` along Y so it stops at this box.
    fn clip_y(&self, mover: &Aabb, d: f64) -> f64;
    /// Limits a movement `d` of `mover` along Z so it stops at this box.
    fn clip_z(&self, mover: &Aabb, d: f64) -> f64;
}

/// One-axis clip: `lo`/`hi` are this box's bounds on the moving axis, `m_lo`/`m_hi` the mover's.
#[inline]
fn clip_axis(lo: f64, hi: f64, m_lo: f64, m_hi: f64, d: f64) -> f64 {
    if d > 0.0 && m_hi <= lo {
        let gap = lo - m_hi;
        if gap < d { gap } else { d }
    } else if d < 0.0 && m_lo >= hi {
        let gap = hi - m_lo;
        if gap > d { gap } else { d }
    } else {
        d
    }
}

/// `true` when the open intervals `(a0, a1)` and `(b0, b1)` overlap. Written with negated
/// comparisons on purpose: with a NaN bound this is `true`, like vanilla's `!(a <= b)` tests.
#[inline]
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn overlaps(a0: f64, a1: f64, b0: f64, b1: f64) -> bool {
    !(b1 <= a0) && !(b0 >= a1)
}

impl BoxOps for Aabb {
    #[inline]
    fn offset(&self, dx: f64, dy: f64, dz: f64) -> Aabb {
        sorted_box(
            self.min_x + dx,
            self.min_y + dy,
            self.min_z + dz,
            self.max_x + dx,
            self.max_y + dy,
            self.max_z + dz,
        )
    }

    #[inline]
    fn stretch(&self, dx: f64, dy: f64, dz: f64) -> Aabb {
        let (mut x0, mut y0, mut z0) = (self.min_x, self.min_y, self.min_z);
        let (mut x1, mut y1, mut z1) = (self.max_x, self.max_y, self.max_z);
        if dx < 0.0 {
            x0 += dx;
        } else if dx > 0.0 {
            x1 += dx;
        }
        if dy < 0.0 {
            y0 += dy;
        } else if dy > 0.0 {
            y1 += dy;
        }
        if dz < 0.0 {
            z0 += dz;
        } else if dz > 0.0 {
            z1 += dz;
        }
        sorted_box(x0, y0, z0, x1, y1, z1)
    }

    #[inline]
    fn grow(&self, x: f64, y: f64, z: f64) -> Aabb {
        sorted_box(
            self.min_x - x,
            self.min_y - y,
            self.min_z - z,
            self.max_x + x,
            self.max_y + y,
            self.max_z + z,
        )
    }

    #[inline]
    fn shrink(&self, x: f64, y: f64, z: f64) -> Aabb {
        sorted_box(
            self.min_x + x,
            self.min_y + y,
            self.min_z + z,
            self.max_x - x,
            self.max_y - y,
            self.max_z - z,
        )
    }

    #[inline]
    fn intersects(&self, o: &Aabb) -> bool {
        overlaps(self.min_x, self.max_x, o.min_x, o.max_x)
            && overlaps(self.min_y, self.max_y, o.min_y, o.max_y)
            && overlaps(self.min_z, self.max_z, o.min_z, o.max_z)
    }

    #[inline]
    fn clip_x(&self, m: &Aabb, d: f64) -> f64 {
        if overlaps(self.min_y, self.max_y, m.min_y, m.max_y)
            && overlaps(self.min_z, self.max_z, m.min_z, m.max_z)
        {
            clip_axis(self.min_x, self.max_x, m.min_x, m.max_x, d)
        } else {
            d
        }
    }

    #[inline]
    fn clip_y(&self, m: &Aabb, d: f64) -> f64 {
        if overlaps(self.min_x, self.max_x, m.min_x, m.max_x)
            && overlaps(self.min_z, self.max_z, m.min_z, m.max_z)
        {
            clip_axis(self.min_y, self.max_y, m.min_y, m.max_y, d)
        } else {
            d
        }
    }

    #[inline]
    fn clip_z(&self, m: &Aabb, d: f64) -> f64 {
        if overlaps(self.min_x, self.max_x, m.min_x, m.max_x)
            && overlaps(self.min_y, self.max_y, m.min_y, m.max_y)
        {
            clip_axis(self.min_z, self.max_z, m.min_z, m.max_z, d)
        } else {
            d
        }
    }
}

/// Vanilla's `Vec3d` constructor turns `-0.0` into `0.0`; that sign matters when the vector is
/// later added to a velocity, so flow-vector maths goes through this.
#[cfg_attr(not(any(feature = "v1_8", feature = "v1_12")), allow(dead_code))]
#[inline]
#[must_use]
pub(crate) fn vec_component(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

/// A `Vec3d`-like triple with vanilla's zero-sign normalisation.
#[cfg_attr(not(any(feature = "v1_8", feature = "v1_12")), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[cfg_attr(not(any(feature = "v1_8", feature = "v1_12")), allow(dead_code))]
impl Vec3 {
    pub(crate) const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    #[inline]
    pub(crate) fn new(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 {
            x: vec_component(x),
            y: vec_component(y),
            z: vec_component(z),
        }
    }

    #[inline]
    pub(crate) fn add(self, x: f64, y: f64, z: f64) -> Vec3 {
        Vec3::new(self.x + x, self.y + y, self.z + z)
    }

    /// Length with vanilla's float square root.
    #[inline]
    pub(crate) fn length(self) -> f64 {
        f64::from(sqrt_f64_as_f32(
            self.x * self.x + self.y * self.y + self.z * self.z,
        ))
    }

    /// Unit vector (float-rounded length); vectors shorter than `1e-4` become zero.
    #[inline]
    pub(crate) fn normalize(self) -> Vec3 {
        let len = self.length();
        if len < 1.0e-4 {
            Vec3::ZERO
        } else {
            Vec3::new(self.x / len, self.y / len, self.z / len)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FNV-1a-style hash of the table bits, sign-extended like a Java `long ^ int`. The constant
    /// was printed by a JDK 21 building the table with `(float) Math.sin(...)` (and with
    /// `StrictMath.sin`, which gave the same value).
    #[test]
    fn sin_table_matches_java() {
        let mut h: u64 = 1_469_598_103_934_665_603;
        for v in SIN_TABLE.iter() {
            h = (h ^ (v.to_bits() as i32 as i64 as u64)).wrapping_mul(1_099_511_628_211);
        }
        assert_eq!(h, 0x3174_5480_87cb_fee1);
    }

    #[test]
    fn floor_matches_java() {
        assert_eq!(floor(-0.5), -1);
        assert_eq!(floor(-1.0), -1);
        assert_eq!(floor(2.9), 2);
        assert_eq!(floor(1e300), i32::MAX);
        assert_eq!(floor(f64::NAN), 0);
    }

    #[test]
    fn min_max_zero_signs() {
        assert!(java_min(0.0, -0.0).is_sign_negative());
        assert!(java_min(-0.0, 0.0).is_sign_negative());
        assert!(java_max(-0.0, 0.0).is_sign_positive());
        assert!(java_max(0.0, -0.0).is_sign_positive());
    }
}
