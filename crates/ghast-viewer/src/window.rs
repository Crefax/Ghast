//! The 256 rows of a taller dimension the page shows (modern versions).

/// The 256 world rows the page shows: page `y` = world `y − lo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// The lowest world row shown.
    pub lo: i32,
}

impl Window {
    /// The window for a bot at `y` in a dimension of `(min_y, height)`: the bot in the middle,
    /// inside the dimension, on a section boundary.
    #[must_use]
    pub fn around(y: f64, (min_y, height): (i32, i32)) -> Window {
        let top = min_y + height.max(256);
        let want = (y.floor() as i32 - 128).div_euclid(16) * 16;
        Window {
            lo: want.clamp(min_y, top - 256),
        }
    }

    /// Whether a bot at `y` is still well inside (else the page is reset with a new window).
    #[must_use]
    pub fn keeps(&self, y: f64, (min_y, height): (i32, i32)) -> bool {
        let y = y.floor() as i32;
        let at_floor = self.lo == min_y;
        let at_top = self.lo + 256 >= min_y + height;
        (at_floor || y >= self.lo + 32) && (at_top || y < self.lo + 224)
    }
}
