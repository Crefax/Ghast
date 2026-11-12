//! Numerics of the modern client (26.1 `Mth`, `Vec3`, `AABB`), with Java's rounding.
//!
//! Differences from 1.16: the sine table is built from a double index (`i / 10430.378...`) and
//! looked up with a double product truncated through `long`; `Vec3` keeps signed zeros and
//! normalises with a double square root; boxes are rebuilt from the position after every move.

#![allow(clippy::many_single_char_names)]

use crate::math::Aabb;
pub use crate::math::{floor, java_max, java_min};

/// Radians to sine-table index scale (`65536 / (2 PI)`, a double).
const SIN_SCALE: f64 = 10430.378350470453;

// 26.1 builds its table as `(float) Math.sin(i / 10430.378350470453)`; a JDK 25 gives exactly the
// table of older versions (same hash, see the test), so the shared one is used.

/// `Mth.sin(double)`.
#[inline]
#[must_use]
pub fn sin(v: f64) -> f32 {
    crate::math::sin_table()[((v * SIN_SCALE) as i64 & 0xFFFF) as usize]
}

/// `Mth.cos(double)`.
#[inline]
#[must_use]
pub fn cos(v: f64) -> f32 {
    crate::math::sin_table()[((v * SIN_SCALE + 16384.0) as i64 & 0xFFFF) as usize]
}

/// `(float) (Math.PI / 180.0)`.
pub const DEG_TO_RAD: f32 = (std::f64::consts::PI / 180.0) as f32;

/// `Mth.sqrt(float)`: `(float) Math.sqrt(x)`.
#[inline]
#[must_use]
pub fn sqrt_f(v: f32) -> f32 {
    f64::from(v).sqrt() as f32
}

/// `Mth.ceil(double)`.
#[inline]
#[must_use]
pub fn ceil(v: f64) -> i32 {
    let i = v as i32;
    if v > f64::from(i) { i + 1 } else { i }
}

/// `Mth.clamp(double, double, double)` (`Math.min(Math.max(v, lo), hi)` semantics for NaN: a NaN
/// value stays NaN).
#[inline]
#[must_use]
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    // Mth.clamp: value < min ? min : Math.min(value, max).
    if v < lo { lo } else { java_min(v, hi) }
}

/// `Mth.frac(double)`: `x - floor(x)` with a `long` floor.
#[inline]
#[must_use]
pub fn frac(v: f64) -> f64 {
    v - lfloor(v) as f64
}

/// `Mth.lfloor(double)`.
#[inline]
#[must_use]
pub fn lfloor(v: f64) -> i64 {
    let i = v as i64;
    if v < i as f64 { i - 1 } else { i }
}

/// `Mth.sign(double)`.
#[inline]
#[must_use]
pub fn sign(v: f64) -> i32 {
    if v == 0.0 {
        0
    } else if v > 0.0 {
        1
    } else {
        -1
    }
}

/// `Mth.lerp(double, double, double)`: `a + t * (b - a)`.
#[inline]
#[must_use]
pub fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

/// `Mth.equal(double, double)`: `|b - a| < 1.0E-5F`.
#[inline]
#[must_use]
pub fn approx_equal(a: f64, b: f64) -> bool {
    (b - a).abs() < f64::from(1.0e-5f32)
}

/// A `Vec3` (no zero-sign normalisation in 26.1).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl V3 {
    pub const ZERO: V3 = V3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    #[inline]
    pub const fn new(x: f64, y: f64, z: f64) -> V3 {
        V3 { x, y, z }
    }

    #[inline]
    pub fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    #[inline]
    pub fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }

    #[inline]
    pub fn scale(self, k: f64) -> V3 {
        V3::new(self.x * k, self.y * k, self.z * k)
    }

    #[inline]
    pub fn mul(self, x: f64, y: f64, z: f64) -> V3 {
        V3::new(self.x * x, self.y * y, self.z * z)
    }

    #[inline]
    pub fn length_sqr(self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    #[inline]
    pub fn length(self) -> f64 {
        self.length_sqr().sqrt()
    }

    #[inline]
    pub fn horizontal_sqr(self) -> f64 {
        self.x * self.x + self.z * self.z
    }

    /// `Vec3.normalize`: double length; shorter than `1.0E-5F` becomes zero.
    #[inline]
    pub fn normalize(self) -> V3 {
        let d = self.length();
        if d < f64::from(1.0e-5f32) {
            V3::ZERO
        } else {
            V3::new(self.x / d, self.y / d, self.z / d)
        }
    }

    #[inline]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    #[inline]
    pub fn get(self, axis: usize) -> f64 {
        match axis {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }

    #[inline]
    pub fn with(self, axis: usize, v: f64) -> V3 {
        match axis {
            0 => V3::new(v, self.y, self.z),
            1 => V3::new(self.x, v, self.z),
            _ => V3::new(self.x, self.y, v),
        }
    }

    #[inline]
    pub fn arr(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    #[inline]
    pub fn of(a: [f64; 3]) -> V3 {
        V3::new(a[0], a[1], a[2])
    }

    /// `distanceToSqr`.
    #[inline]
    pub fn dist_sqr(self, o: V3) -> f64 {
        let (dx, dy, dz) = (self.x - o.x, self.y - o.y, self.z - o.z);
        dx * dx + dy * dy + dz * dz
    }
}

/// `new AABB(...)` (each axis sorted with Java `Math.min` / `Math.max`).
#[inline]
pub fn bx(x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> Aabb {
    Aabb::new(
        jmin(x0, x1),
        jmin(y0, y1),
        jmin(z0, z1),
        jmax(x0, x1),
        jmax(y0, y1),
        jmax(z0, z1),
    )
}

/// Java `Math.min` with the ordered case first (equal values, zeros and NaN take the full rule).
#[inline]
fn jmin(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else if b < a {
        b
    } else {
        java_min(a, b)
    }
}

/// Java `Math.max` with the ordered case first.
#[inline]
fn jmax(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else if b > a {
        b
    } else {
        java_max(a, b)
    }
}

/// `AABB.move`.
#[inline]
pub fn mv(b: &Aabb, x: f64, y: f64, z: f64) -> Aabb {
    bx(
        b.min_x + x,
        b.min_y + y,
        b.min_z + z,
        b.max_x + x,
        b.max_y + y,
        b.max_z + z,
    )
}

/// `AABB.inflate(x, y, z)` (negative deflates).
#[inline]
pub fn inflate(b: &Aabb, x: f64, y: f64, z: f64) -> Aabb {
    bx(
        b.min_x - x,
        b.min_y - y,
        b.min_z - z,
        b.max_x + x,
        b.max_y + y,
        b.max_z + z,
    )
}

/// `AABB.deflate(v)`.
#[inline]
pub fn deflate(b: &Aabb, v: f64) -> Aabb {
    inflate(b, -v, -v, -v)
}

/// `AABB.expandTowards`.
#[inline]
pub fn expand_towards(b: &Aabb, v: V3) -> Aabb {
    let (mut x0, mut y0, mut z0, mut x1, mut y1, mut z1) =
        (b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z);
    if v.x < 0.0 {
        x0 += v.x;
    } else if v.x > 0.0 {
        x1 += v.x;
    }
    if v.y < 0.0 {
        y0 += v.y;
    } else if v.y > 0.0 {
        y1 += v.y;
    }
    if v.z < 0.0 {
        z0 += v.z;
    } else if v.z > 0.0 {
        z1 += v.z;
    }
    bx(x0, y0, z0, x1, y1, z1)
}

/// `AABB.intersects(minX..maxZ)` (strict).
#[inline]
pub fn intersects(b: &Aabb, x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> bool {
    b.min_x < x1 && b.max_x > x0 && b.min_y < y1 && b.max_y > y0 && b.min_z < z1 && b.max_z > z0
}

/// `AABB.contains(x, y, z)` (half-open).
#[inline]
pub fn contains(b: &Aabb, p: V3) -> bool {
    p.x >= b.min_x
        && p.x < b.max_x
        && p.y >= b.min_y
        && p.y < b.max_y
        && p.z >= b.min_z
        && p.z < b.max_z
}

/// `AABB.getCenter()`.
#[inline]
pub fn center(b: &Aabb) -> V3 {
    V3::new(
        lerp(0.5, b.min_x, b.max_x),
        lerp(0.5, b.min_y, b.max_y),
        lerp(0.5, b.min_z, b.max_z),
    )
}

#[inline]
pub fn lo(b: &Aabb, axis: usize) -> f64 {
    match axis {
        0 => b.min_x,
        1 => b.min_y,
        _ => b.min_z,
    }
}

#[inline]
pub fn hi(b: &Aabb, axis: usize) -> f64 {
    match axis {
        0 => b.max_x,
        1 => b.max_y,
        _ => b.max_z,
    }
}

/// `AABB.clip(min.., max.., from, to)`: the entry point of the segment `from -> to` into the box.
pub fn clip_box(b: &Aabb, from: V3, to: V3) -> Option<V3> {
    let mut scale = 1.0f64;
    let (dx, dy, dz) = (to.x - from.x, to.y - from.y, to.z - from.z);
    let hit = clip_direction(b, from, &mut scale, dx, dy, dz);
    hit.then(|| {
        V3::new(
            from.x + scale * dx,
            from.y + scale * dy,
            from.z + scale * dz,
        )
    })
}

/// `AABB.getDirection`: `true` when some face was hit (`scale` is the entry fraction).
fn clip_direction(b: &Aabb, from: V3, scale: &mut f64, dx: f64, dy: f64, dz: f64) -> bool {
    let mut hit = false;
    let eps = 1.0e-7;
    let point = |s_ref: &mut f64,
                 da: f64,
                 db: f64,
                 dc: f64,
                 p: f64,
                 min_b: f64,
                 max_b: f64,
                 min_c: f64,
                 max_c: f64,
                 fa: f64,
                 fb: f64,
                 fc: f64| {
        let s = (p - fa) / da;
        let pb = fb + s * db;
        let pc = fc + s * dc;
        if 0.0 < s
            && s < *s_ref
            && min_b - eps < pb
            && pb < max_b + eps
            && min_c - eps < pc
            && pc < max_c + eps
        {
            *s_ref = s;
            true
        } else {
            false
        }
    };
    if dx > eps {
        hit |= point(
            scale, dx, dy, dz, b.min_x, b.min_y, b.max_y, b.min_z, b.max_z, from.x, from.y, from.z,
        );
    } else if dx < -eps {
        hit |= point(
            scale, dx, dy, dz, b.max_x, b.min_y, b.max_y, b.min_z, b.max_z, from.x, from.y, from.z,
        );
    }
    if dy > eps {
        hit |= point(
            scale, dy, dz, dx, b.min_y, b.min_z, b.max_z, b.min_x, b.max_x, from.y, from.z, from.x,
        );
    } else if dy < -eps {
        hit |= point(
            scale, dy, dz, dx, b.max_y, b.min_z, b.max_z, b.min_x, b.max_x, from.y, from.z, from.x,
        );
    }
    if dz > eps {
        hit |= point(
            scale, dz, dx, dy, b.min_z, b.min_x, b.max_x, b.min_y, b.max_y, from.z, from.x, from.y,
        );
    } else if dz < -eps {
        hit |= point(
            scale, dz, dx, dy, b.max_z, b.min_x, b.max_x, b.min_y, b.max_y, from.z, from.x, from.y,
        );
    }
    hit
}

/// `AABB.collidedAlongVector(vector, boxes)` for this box.
#[inline]
pub fn collided_along(b: &Aabb, vector: V3, boxes: impl Iterator<Item = Aabb>) -> bool {
    let from = center(b);
    let to = from.add(vector);
    let (hx, hy, hz) = (
        (b.max_x - b.min_x) * 0.5 - 1.0e-7,
        (b.max_y - b.min_y) * 0.5 - 1.0e-7,
        (b.max_z - b.min_z) * 0.5 - 1.0e-7,
    );
    for part in boxes {
        let inflated = inflate(&part, hx, hy, hz);
        if contains(&inflated, to) || contains(&inflated, from) {
            return true;
        }
        if clip_box(&inflated, from, to).is_some() {
            return true;
        }
    }
    false
}

/// `Vec3.xRot(radians)` (float sine/cosine).
pub fn x_rot(v: V3, radians: f32) -> V3 {
    let c = f64::from(cos(f64::from(radians)));
    let s = f64::from(sin(f64::from(radians)));
    V3::new(v.x, v.y * c + v.z * s, v.z * c - v.y * s)
}

/// `Vec3.yRot(radians)`.
pub fn y_rot(v: V3, radians: f32) -> V3 {
    let c = f64::from(cos(f64::from(radians)));
    let s = f64::from(sin(f64::from(radians)));
    V3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s)
}

/// `Math.acos` as the JDK computes it (`StrictMath.acos`, fdlibm `e_acos.c`).
#[allow(clippy::unreadable_literal, clippy::excessive_precision)]
pub fn acos(x: f64) -> f64 {
    const PIO2_HI: f64 = std::f64::consts::FRAC_PI_2;
    const PIO2_LO: f64 = 6.12323399573676603587e-17;
    const PS0: f64 = 1.66666666666666657415e-01;
    const PS1: f64 = -3.25565818622400915405e-01;
    const PS2: f64 = 2.01212532134862925881e-01;
    const PS3: f64 = -4.00555345006794114027e-02;
    const PS4: f64 = 7.91534994289814532176e-04;
    const PS5: f64 = 3.47933107596021167570e-05;
    const QS1: f64 = -2.40339491173441421878e+00;
    const QS2: f64 = 2.02094576023350569471e+00;
    const QS3: f64 = -6.88283971605453293030e-01;
    const QS4: f64 = 7.70381505559019352791e-02;
    let hx = (x.to_bits() >> 32) as i32;
    let ix = hx & 0x7fffffff;
    if ix >= 0x3ff00000 {
        let lx = x.to_bits() as u32;
        if ((ix - 0x3ff00000) as u32 | lx) == 0 {
            return if hx > 0 {
                0.0
            } else {
                std::f64::consts::PI + 2.0 * PIO2_LO
            };
        }
        return f64::NAN;
    }
    let r = |z: f64| {
        let p = z * (PS0 + z * (PS1 + z * (PS2 + z * (PS3 + z * (PS4 + z * PS5)))));
        let q = 1.0 + z * (QS1 + z * (QS2 + z * (QS3 + z * QS4)));
        p / q
    };
    if ix < 0x3fe00000 {
        if ix <= 0x3c600000 {
            return PIO2_HI + PIO2_LO;
        }
        let z = x * x;
        let rr = r(z);
        return PIO2_HI - (x - (PIO2_LO - x * rr));
    }
    if hx < 0 {
        let z = (1.0 + x) * 0.5;
        let s = z.sqrt();
        let w = r(z) * s - PIO2_LO;
        return std::f64::consts::PI - 2.0 * (s + w);
    }
    let z = (1.0 - x) * 0.5;
    let s = z.sqrt();
    let df = f64::from_bits(s.to_bits() & 0xffffffff00000000);
    let c = (z - df * df) / (s + df);
    let w = r(z) * s + c;
    2.0 * (df + w)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FNV-1a over the table bits, sign-extended like a Java `long ^ int`. The constant was
    /// printed by a JDK 25 building the table with `(float) Math.sin(i / 10430.378350470453)`.
    #[test]
    fn sin_table_matches_java() {
        let mut h: u64 = 1_469_598_103_934_665_603;
        for v in crate::math::sin_table().iter() {
            h = (h ^ (v.to_bits() as i32 as i64 as u64)).wrapping_mul(1_099_511_628_211);
        }
        assert_eq!(h, 0x3174_5480_87cb_fee1);
    }

    #[test]
    fn acos_matches_known_values() {
        assert_eq!(acos(1.0), 0.0);
        assert_eq!(acos(0.0), std::f64::consts::FRAC_PI_2);
        assert_eq!(acos(-1.0), std::f64::consts::PI);
        // JDK 25 Math.acos results.
        assert_eq!(acos(0.5).to_bits(), 0x3ff0_c152_382d_7366);
        assert_eq!(acos(0.123456789).to_bits(), 0x3ff7_2702_6158_be56);
        assert_eq!(acos(-0.987654321).to_bits(), 0x4007_dfd6_6cb6_7546);
    }
}
