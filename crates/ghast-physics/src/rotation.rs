//! Mouse-step rotation as the vanilla client applies it (1.8 renderer + `Entity` angle update).
//!
//! A real mouse moves in whole counts, so every yaw / pitch change the server sees is an integer
//! multiple of `multiplier * 0.15` degrees (the "GCD" anti-cheats check). These helpers turn a
//! sensitivity setting and integer mouse counts into the exact `f32` angles a vanilla client
//! would have. How many counts to move per tick (aim speed, smoothing) is client policy and lives
//! above this crate.

/// Mouse sensitivity factor (`f * f * f * 8` with `f = sensitivity * 0.6 + 0.2`, all `float`).
/// `sensitivity` is the options value `0.0..=1.0` (0.5 = 100 %).
#[must_use]
pub fn mouse_multiplier(sensitivity: f32) -> f32 {
    let f = sensitivity * 0.6 + 0.2;
    f * f * f * 8.0
}

/// Smallest possible rotation change in degrees: one mouse count (`multiplier * 0.15`, as
/// `float`).
#[must_use]
pub fn mouse_step_degrees(sensitivity: f32) -> f32 {
    mouse_multiplier(sensitivity) * 0.15
}

/// Applies whole mouse counts to an angle pair: `yaw_counts` turn right (positive yaw),
/// `pitch_counts` look down (positive pitch). Pitch is clamped to ±90 like vanilla.
#[must_use]
pub fn turn(
    yaw: f32,
    pitch: f32,
    yaw_counts: i32,
    pitch_counts: i32,
    multiplier: f32,
) -> (f32, f32) {
    // The renderer scales the raw mouse deltas in float; the entity adds them ×0.15 in double.
    // Mouse "up" is positive and lowers the pitch, so looking down is a negative mouse delta.
    let dx = yaw_counts as f32 * multiplier;
    let dy = (-pitch_counts) as f32 * multiplier;
    let yaw = (f64::from(yaw) + f64::from(dx) * 0.15) as f32;
    let pitch = ((f64::from(pitch) - f64::from(dy) * 0.15) as f32).clamp(-90.0, 90.0);
    (yaw, pitch)
}

/// Vanilla `MathHelper.wrapDegrees(float)`: an angle in `[-180, 180)`.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_whole_multiples() {
        let m = mouse_multiplier(0.338_028_2);
        let (y, p) = turn(0.0, 0.0, 10, -4, m);
        let step = f64::from(m) * 0.15;
        assert!((f64::from(y) / step - 10.0).abs() < 1e-3);
        assert!((f64::from(p) / step + 4.0).abs() < 1e-3);
        assert_eq!(turn(0.0, 89.0, 0, 1000, m).1, 90.0);
    }

    #[test]
    fn wrap() {
        assert_eq!(wrap_degrees(190.0), -170.0);
        assert_eq!(wrap_degrees(-180.0), -180.0);
        assert_eq!(wrap_degrees(540.0), -180.0);
    }
}
