//! Rotation in whole mouse counts.
//!
//! A real client turns by `mouse delta × multiplier × 0.15` degrees, so every yaw / pitch
//! change is an integer multiple of the sensitivity's GCD (`multiplier × 0.15`). Anti-cheats
//! (Grim's aim checks) look for that. This module reproduces the client's float arithmetic:
//! the multiplier is computed in `f32` like `GameRenderer.render`, the
//! angles are added in `f64` and stored as `f32` like `Entity.increaseTransforms`.
//!
//! The speed model (at most 14° yaw / 10.5° pitch per tick with a carried budget) is the one
//! the reference behaviours were tuned with; it is a behaviour choice, not vanilla.

/// `GameRenderer`: `f = sensitivity * 0.6 + 0.2; f * f * f * 8`, all `f32`.
#[must_use]
pub fn mouse_multiplier(sensitivity: f32) -> f32 {
    let s = sensitivity * 0.6 + 0.2;
    s * s * s * 8.0
}

/// The smallest rotation step for a sensitivity: `multiplier × 0.15` (as `f32`).
#[must_use]
pub fn mouse_gcd(sensitivity: f32) -> f32 {
    mouse_multiplier(sensitivity) * 0.15
}

/// `MathHelper.wrapDegrees`.
#[must_use]
pub fn wrap_degrees(a: f32) -> f32 {
    let mut v = a % 360.0;
    if v >= 180.0 {
        v -= 360.0;
    }
    if v < -180.0 {
        v += 360.0;
    }
    v
}

/// Java's `Math.round(double)`: `floor(x + 0.5)` (as an integer).
fn java_round(x: f64) -> i64 {
    // Out-of-range values saturate like Java's (long) cast.
    (x + 0.5).floor() as i64
}

/// Integer mouse counts with a fractional speed budget carried between ticks.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MouseStep {
    budget: f64,
}

impl MouseStep {
    /// Mouse counts toward `delta` degrees this tick, at most `degrees_per_tick` (plus the
    /// carried fraction).
    pub fn counts(&mut self, delta: f32, multiplier: f32, degrees_per_tick: f64) -> i64 {
        let step = f64::from(multiplier) * 0.15;
        let per_tick = degrees_per_tick / step;
        self.budget = (self.budget + per_tick).min(per_tick + 1.0);
        let wanted = java_round(f64::from(delta) / step);
        let lim = self.budget.trunc() as i64;
        let count = wanted.clamp(-lim, lim);
        self.budget -= count.unsigned_abs() as f64;
        if count == wanted {
            self.budget = 0.0;
        }
        count
    }
}

/// Turns toward a target in whole mouse counts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation {
    sensitivity: f32,
    yaw_step: MouseStep,
    pitch_step: MouseStep,
    target_yaw: f32,
    target_pitch: f32,
}

impl Rotation {
    /// Maximum yaw change per tick (degrees).
    pub const MAX_YAW_PER_TICK: f64 = 14.0;
    /// Maximum pitch change per tick (degrees).
    pub const MAX_PITCH_PER_TICK: f64 = 10.5;

    /// A controller for an options.txt mouse sensitivity (0 … 1; vanilla default 0.5).
    #[must_use]
    pub fn new(sensitivity: f32) -> Self {
        Rotation {
            sensitivity,
            yaw_step: MouseStep::default(),
            pitch_step: MouseStep::default(),
            target_yaw: 0.0,
            target_pitch: 0.0,
        }
    }

    /// The sensitivity.
    #[must_use]
    pub fn sensitivity(&self) -> f32 {
        self.sensitivity
    }

    /// Forgets carried budgets and aims at the current look.
    pub fn reset(&mut self, yaw: f32, pitch: f32) {
        self.yaw_step = MouseStep::default();
        self.pitch_step = MouseStep::default();
        self.target_yaw = yaw;
        self.target_pitch = pitch;
    }

    /// Sets the target (pitch clamped to ±89.5).
    pub fn set_target(&mut self, yaw: f32, pitch: f32) {
        self.target_yaw = yaw;
        self.target_pitch = pitch.clamp(-89.5, 89.5);
    }

    /// The target yaw / pitch.
    #[must_use]
    pub fn target(&self) -> (f32, f32) {
        (self.target_yaw, self.target_pitch)
    }

    /// One tick: the new (yaw, pitch) from the current one.
    ///
    /// `Entity.increaseTransforms(dx * m, dy * m)`: `yaw = (float)(yaw + dx * 0.15)`,
    /// `pitch = (float)(pitch - dy * 0.15)`, pitch clamped to ±90.
    pub fn step(&mut self, yaw: f32, pitch: f32) -> (f32, f32) {
        let m = mouse_multiplier(self.sensitivity);
        let dx = self.yaw_step.counts(
            wrap_degrees(self.target_yaw - yaw),
            m,
            Self::MAX_YAW_PER_TICK,
        );
        let dy = self
            .pitch_step
            .counts(self.target_pitch - pitch, m, Self::MAX_PITCH_PER_TICK);
        // Mouse counts are small integers; the float conversion is exact.
        let fx = dx as f32 * m;
        let fy = -(dy as f32) * m;
        let new_yaw = (f64::from(yaw) + f64::from(fx) * 0.15) as f32;
        let new_pitch = ((f64::from(pitch) - f64::from(fy) * 0.15) as f32).clamp(-90.0, 90.0);
        (new_yaw, new_pitch)
    }

    /// Remaining yaw error to the target (degrees, wrapped).
    #[must_use]
    pub fn yaw_error(&self, yaw: f32) -> f32 {
        wrap_degrees(self.target_yaw - yaw).abs()
    }

    /// Remaining pitch error to the target.
    #[must_use]
    pub fn pitch_error(&self, pitch: f32) -> f32 {
        (self.target_pitch - pitch).abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gcd_steps() {
        let sens = 0.338_028_2_f32;
        let gcd = f64::from(mouse_gcd(sens));
        let mut r = Rotation::new(sens);
        let (mut yaw, mut pitch) = (12.5_f32, 3.0_f32);
        r.set_target(100.0, 40.0);
        for _ in 0..40 {
            let (ny, np) = r.step(yaw, pitch);
            let dy = f64::from(ny) - f64::from(yaw);
            let dp = f64::from(np) - f64::from(pitch);
            assert!(dy.abs() <= Rotation::MAX_YAW_PER_TICK + gcd + 1e-3);
            assert!(dp.abs() <= Rotation::MAX_PITCH_PER_TICK + gcd + 1e-3);
            // Every change is a whole number of GCD steps (up to f32 rounding of the sum).
            let k = dy / gcd;
            assert!(
                (k - k.round()).abs() < 1e-2,
                "yaw step {dy} is not a multiple of {gcd}"
            );
            yaw = ny;
            pitch = np;
        }
        assert!(r.yaw_error(yaw) < gcd as f32);
        assert!(r.pitch_error(pitch) < gcd as f32);
    }

    #[test]
    fn wrap() {
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-190.0), 170.0);
        assert_eq!(wrap_degrees(540.0), -180.0);
        assert_eq!(wrap_degrees(180.0), -180.0);
    }

    #[test]
    fn java_rounding() {
        assert_eq!(java_round(2.5), 3);
        assert_eq!(java_round(-2.5), -2);
        assert_eq!(java_round(-2.6), -3);
    }

    /// Values computed with an independent TypeScript (`Math.fround`) port.
    #[test]
    fn multiplier_is_f32() {
        assert!((mouse_multiplier(0.5) - 1.0).abs() < 1e-6);
        let m = mouse_multiplier(0.338_028_2);
        assert_eq!(m, {
            let s: f32 = 0.338_028_2 * 0.6 + 0.2;
            s * s * s * 8.0
        });
    }
}
