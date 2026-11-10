//! The client's world border (`world_border` packet, identical in 1.8 and 1.16.5).
//!
//! Vanilla interpolates a moving border with the wall clock. Here time only advances when the
//! owner calls [`Border::advance`] (the client does it once per 50 ms tick), so the result is
//! deterministic and replayable.

// Each version uses one interpolation flavour; builds with one version leave the other unused.
#![cfg_attr(
    not(all(feature = "v1_8", feature = "v1_16_5")),
    allow(dead_code, unused_macros, unused_imports)
)]

/// Interpolation arithmetic of the version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LerpMath {
    /// 1.8: progress is `(float) elapsed / (float) duration`.
    Float,
    /// 1.16: progress is `(double) elapsed / duration`.
    Double,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Lerp {
    from: f64,
    to: f64,
    duration_ms: i64,
    elapsed_ms: i64,
}

/// World border state: centre, diameter (possibly moving) and warning settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    center_x: f64,
    center_z: f64,
    size: f64,
    lerp: Option<Lerp>,
    math: LerpMath,
    /// Portal teleport boundary (`portalBoundary`).
    pub portal_boundary: i32,
    /// Warning time in seconds.
    pub warning_time: i32,
    /// Warning distance in blocks.
    pub warning_blocks: i32,
}

/// The fields of a `world_border` packet (same in 1.8 and 1.16.5).
pub(crate) struct BorderPacket {
    pub action: i32,
    pub diameter: Option<f64>,
    pub x: Option<f64>,
    pub z: Option<f64>,
    pub old_diameter: Option<f64>,
    pub new_diameter: Option<f64>,
    pub speed_ms: Option<i64>,
    pub portal_boundary: Option<i32>,
    pub warning_time: Option<i32>,
    pub warning_blocks: Option<i32>,
}

impl BorderPacket {
    /// A packet with every optional field absent.
    #[cfg_attr(not(feature = "v26_1"), allow(dead_code))]
    pub(crate) const EMPTY: BorderPacket = BorderPacket {
        action: 0,
        diameter: None,
        x: None,
        z: None,
        old_diameter: None,
        new_diameter: None,
        speed_ms: None,
        portal_boundary: None,
        warning_time: None,
        warning_blocks: None,
    };
}

impl Border {
    pub(crate) fn new(math: LerpMath) -> Self {
        Border {
            center_x: 0.0,
            center_z: 0.0,
            size: 6.0e7,
            lerp: None,
            math,
            portal_boundary: 29_999_984,
            warning_time: 15,
            warning_blocks: 5,
        }
    }

    /// Centre `(x, z)`.
    #[must_use]
    pub fn center(&self) -> (f64, f64) {
        (self.center_x, self.center_z)
    }

    /// Current diameter (interpolated while moving).
    #[must_use]
    pub fn size(&self) -> f64 {
        let Some(l) = self.lerp else {
            return self.size;
        };
        let t = match self.math {
            LerpMath::Float => f64::from(l.elapsed_ms as f32 / l.duration_ms as f32),
            LerpMath::Double => l.elapsed_ms as f64 / l.duration_ms as f64,
        };
        if t < 1.0 {
            l.from + (l.to - l.from) * t
        } else {
            l.to
        }
    }

    /// Target diameter of a moving border (the current one when it is stationary).
    #[must_use]
    pub fn target_size(&self) -> f64 {
        self.lerp.map_or(self.size, |l| l.to)
    }

    /// Advances a moving border by `ms` milliseconds.
    pub fn advance(&mut self, ms: i64) {
        if let Some(l) = &mut self.lerp {
            l.elapsed_ms = l.elapsed_ms.saturating_add(ms);
            if l.elapsed_ms >= l.duration_ms {
                self.size = l.to;
                self.lerp = None;
            }
        }
    }

    fn set_size(&mut self, d: f64) {
        self.size = d;
        self.lerp = None;
    }

    fn set_lerp(&mut self, from: f64, to: f64, ms: i64) {
        if ms <= 0 || (self.math == LerpMath::Double && from == to) {
            self.set_size(to);
        } else {
            self.size = from;
            self.lerp = Some(Lerp {
                from,
                to,
                duration_ms: ms,
                elapsed_ms: 0,
            });
        }
    }

    pub(crate) fn apply(&mut self, p: &BorderPacket) -> bool {
        match p.action {
            0 => self.set_size(p.diameter.unwrap_or(self.size)),
            1 => self.set_lerp(
                p.old_diameter.unwrap_or(self.size),
                p.new_diameter.unwrap_or(self.size),
                p.speed_ms.unwrap_or(0),
            ),
            2 => {
                self.center_x = p.x.unwrap_or(self.center_x);
                self.center_z = p.z.unwrap_or(self.center_z);
            }
            3 => {
                self.center_x = p.x.unwrap_or(self.center_x);
                self.center_z = p.z.unwrap_or(self.center_z);
                self.set_lerp(
                    p.old_diameter.unwrap_or(self.size),
                    p.new_diameter.unwrap_or(self.size),
                    p.speed_ms.unwrap_or(0),
                );
                self.portal_boundary = p.portal_boundary.unwrap_or(self.portal_boundary);
                self.warning_time = p.warning_time.unwrap_or(self.warning_time);
                self.warning_blocks = p.warning_blocks.unwrap_or(self.warning_blocks);
            }
            4 => self.warning_time = p.warning_time.unwrap_or(self.warning_time),
            5 => self.warning_blocks = p.warning_blocks.unwrap_or(self.warning_blocks),
            _ => return false,
        }
        true
    }
}

macro_rules! border_packet {
    ($p:expr) => {
        $crate::border::BorderPacket {
            action: $p.action,
            diameter: $p.radius,
            x: $p.x,
            z: $p.z,
            old_diameter: $p.old_radius,
            new_diameter: $p.new_radius,
            speed_ms: $p.speed,
            portal_boundary: $p.portal_boundary,
            warning_time: $p.warning_time,
            warning_blocks: $p.warning_blocks,
        }
    };
}
pub(crate) use border_packet;

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(action: i32) -> BorderPacket {
        BorderPacket {
            action,
            diameter: Some(100.0),
            x: Some(10.0),
            z: Some(-20.0),
            old_diameter: Some(200.0),
            new_diameter: Some(100.0),
            speed_ms: Some(1000),
            portal_boundary: Some(5),
            warning_time: Some(7),
            warning_blocks: Some(9),
        }
    }

    #[test]
    fn actions() {
        let mut b = Border::new(LerpMath::Float);
        assert_eq!(b.size(), 6.0e7);
        b.apply(&pkt(3));
        assert_eq!(b.center(), (10.0, -20.0));
        assert_eq!(b.size(), 200.0);
        b.advance(250);
        assert_eq!(b.size(), 175.0);
        b.advance(750);
        assert_eq!(b.size(), 100.0);
        assert_eq!(b.target_size(), 100.0);
        assert_eq!(
            (b.portal_boundary, b.warning_time, b.warning_blocks),
            (5, 7, 9)
        );
        b.apply(&BorderPacket {
            diameter: Some(50.0),
            ..pkt(0)
        });
        assert_eq!(b.size(), 50.0);
        assert!(!b.apply(&pkt(9)));
    }

    #[test]
    fn float_progress_1_8() {
        // 1/3 in float differs from 1/3 in double.
        let mut f = Border::new(LerpMath::Float);
        let mut d = Border::new(LerpMath::Double);
        for b in [&mut f, &mut d] {
            b.apply(&BorderPacket {
                old_diameter: Some(0.0),
                new_diameter: Some(3.0),
                speed_ms: Some(3),
                ..pkt(1)
            });
            b.advance(1);
        }
        assert_eq!(f.size(), 3.0 * f64::from(1.0f32 / 3.0));
        assert_eq!(d.size(), 3.0 * (1.0 / 3.0));
        assert_ne!(f.size(), d.size());
    }
}
