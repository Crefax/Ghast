//! Where to go.

/// A path goal. Positions are feet blocks (the block the player's feet are in).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Goal {
    /// Stand with the feet in this block.
    Block([i32; 3]),
    /// Feet block centre within `radius` blocks (3D distance) of this block's centre.
    Near {
        /// Centre block.
        pos: [i32; 3],
        /// Radius in blocks.
        radius: f64,
    },
    /// Any height in this column.
    Xz {
        /// Block X.
        x: i32,
        /// Block Z.
        z: i32,
    },
    /// Feet at this block Y anywhere.
    Y(i32),
    /// Any of the goals (the nearest reachable one wins).
    AnyOf(Vec<Goal>),
}

impl Goal {
    /// `true` when standing with the feet in `p` satisfies the goal.
    #[must_use]
    pub fn is_reached(&self, p: [i32; 3]) -> bool {
        match self {
            Goal::Block(b) => *b == p,
            Goal::Near { pos, radius } => dist2(*pos, p) <= radius * radius + 1e-9,
            Goal::Xz { x, z } => p[0] == *x && p[2] == *z,
            Goal::Y(y) => p[1] == *y,
            Goal::AnyOf(gs) => gs.iter().any(|g| g.is_reached(p)),
        }
    }

    /// Distance in blocks from feet block `p` to the nearest block that satisfies the goal
    /// (0 when reached; `Y`: the height difference).
    pub(crate) fn distance(&self, p: [i32; 3]) -> f64 {
        match self {
            Goal::Block(b) => dist2(*b, p).sqrt(),
            Goal::Near { pos, radius } => (dist2(*pos, p).sqrt() - radius).max(0.0),
            Goal::Xz { x, z } => f64::from(x - p[0]).hypot(f64::from(z - p[2])),
            Goal::Y(y) => f64::from(y - p[1]).abs(),
            Goal::AnyOf(gs) => gs
                .iter()
                .map(|g| g.distance(p))
                .fold(f64::INFINITY, f64::min),
        }
    }

    /// Admissible estimate of the remaining cost (ticks) from `p`.
    pub(crate) fn estimate(&self, p: [i32; 3], h: &Heuristic) -> f64 {
        match self {
            Goal::Block(b) => h.between(p, *b),
            Goal::Near { pos, radius } => {
                let d = dist2(*pos, p).sqrt();
                if d <= *radius {
                    0.0
                } else {
                    // Scale the full estimate by the part of the distance left.
                    h.between(p, *pos) * ((d - radius) / d).max(0.0)
                }
            }
            Goal::Xz { x, z } => h.horizontal(f64::from(x - p[0]).abs(), f64::from(z - p[2]).abs()),
            Goal::Y(y) => h.vertical(y - p[1]),
            Goal::AnyOf(gs) => gs
                .iter()
                .map(|g| g.estimate(p, h))
                .fold(f64::INFINITY, f64::min),
        }
    }
}

fn dist2(a: [i32; 3], b: [i32; 3]) -> f64 {
    let d = |i: usize| f64::from(a[i] - b[i]);
    d(0) * d(0) + d(1) * d(1) + d(2) * d(2)
}

/// Lower bounds of the cost per block, from the cheapest move of each kind.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Heuristic {
    /// Cost per horizontal block (straight).
    pub(crate) per_block: f64,
    /// Cost per block up.
    pub(crate) per_up: f64,
    /// Cost per block down.
    pub(crate) per_down: f64,
}

impl Heuristic {
    pub(crate) fn horizontal(&self, dx: f64, dz: f64) -> f64 {
        let (a, b) = if dx > dz { (dx, dz) } else { (dz, dx) };
        // Octile distance: diagonals cost sqrt(2) straight blocks.
        (a - b + b * std::f64::consts::SQRT_2) * self.per_block
    }

    pub(crate) fn vertical(&self, dy: i32) -> f64 {
        if dy > 0 {
            f64::from(dy) * self.per_up
        } else {
            f64::from(-dy) * self.per_down
        }
    }

    pub(crate) fn between(&self, a: [i32; 3], b: [i32; 3]) -> f64 {
        let h = self.horizontal(f64::from(b[0] - a[0]).abs(), f64::from(b[2] - a[2]).abs());
        let v = self.vertical(b[1] - a[1]);
        h.max(v)
    }
}
