//! Hand-written protocol types shared by the generated packet code.

pub mod position;
pub mod slot;

pub use bytestring::ByteString;
pub use position::{BlockPos, PositionLayout};
pub use uuid::Uuid;

/// Converts a protocol angle byte (1/256 of a full turn) to degrees in `[-180, 180)`.
#[inline]
pub fn angle_to_degrees(a: i8) -> f32 {
    f32::from(a) * (360.0 / 256.0)
}

/// Converts degrees to a protocol angle byte the way vanilla does
/// (`(int)(deg * 256 / 360)`, truncated, then wrapped to a byte).
#[inline]
pub fn degrees_to_angle(deg: f32) -> i8 {
    ((deg * 256.0 / 360.0) as i32) as i8
}

/// Converts a 1.8-style fixed-point coordinate (1/32 block, as in spawn/teleport
/// packets) to blocks.
#[inline]
pub fn fixed_to_f64(v: i32) -> f64 {
    f64::from(v) / 32.0
}

/// Converts blocks to a 1.8-style fixed-point coordinate (`floor(v * 32)`, like vanilla).
#[inline]
pub fn f64_to_fixed(v: f64) -> i32 {
    (v * 32.0).floor() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_and_fixed_point() {
        assert_eq!(angle_to_degrees(64), 90.0);
        assert_eq!(angle_to_degrees(-128), -180.0);
        assert_eq!(degrees_to_angle(90.0), 64);
        assert_eq!(degrees_to_angle(270.0), -64);
        assert_eq!(fixed_to_f64(48), 1.5);
        assert_eq!(f64_to_fixed(-0.01), -1);
        assert_eq!(f64_to_fixed(1.5), 48);
    }
}
