//! The client's world border as 26.1 moves it: a lerping size advanced once per tick (in
//! `ClientLevel.tick`, after the entities), and the size the collision code reads.
//!
//! The engine keeps one [`BorderState`] per level, applies the border packets to it
//! (`initialize_border`, `set_border_center`, `set_border_size`, `set_border_lerp_size`), calls
//! [`BorderState::tick`] after the tick's entities (the player included), and hands
//! [`BorderState::collision_border`] to the player's tick through its world view
//! (`BlockView::world_border`).

use crate::world::WorldBorder;

/// The size part of the border: fixed, or moving from one size to another.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Extent {
    Static(f64),
    Moving {
        from: f64,
        to: f64,
        duration: f64,
        progress: i64,
        size: f64,
        previous: f64,
    },
}

/// The client's world border: centre and (possibly moving) size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderState {
    center_x: f64,
    center_z: f64,
    extent: Extent,
}

impl Default for BorderState {
    fn default() -> Self {
        BorderState {
            center_x: 0.0,
            center_z: 0.0,
            extent: Extent::Static(5.999_996_8e7),
        }
    }
}

fn lerp(delta: f64, a: f64, b: f64) -> f64 {
    a + delta * (b - a)
}

fn moving_size(from: f64, to: f64, duration: f64, progress: i64) -> f64 {
    let t = (duration - progress as f64) / duration;
    if t < 1.0 { lerp(t, from, to) } else { to }
}

impl BorderState {
    /// The border a client has before any border packet (centre 0, 0, size 59,999,968).
    #[must_use]
    pub fn new() -> BorderState {
        BorderState::default()
    }

    /// `setCenter` (set-border-center packet, or the centre of initialize-border).
    pub fn set_center(&mut self, x: f64, z: f64) {
        self.center_x = x;
        self.center_z = z;
    }

    /// `setSize`: a static border (set-border-size packet).
    pub fn set_size(&mut self, size: f64) {
        self.extent = Extent::Static(size);
    }

    /// `lerpSizeBetween(from, to, ticks)` (set-border-lerp-size packet; initialize-border with a
    /// lerp time): equal sizes give a static border.
    pub fn lerp_size(&mut self, from: f64, to: f64, ticks: i64) {
        self.extent = if from == to {
            Extent::Static(to)
        } else {
            let size = moving_size(from, to, ticks as f64, ticks);
            Extent::Moving {
                from,
                to,
                duration: ticks as f64,
                progress: ticks,
                size,
                previous: size,
            }
        };
    }

    /// `WorldBorder.tick` (in `ClientLevel.tick`, after the entities): a moving border advances
    /// one tick and becomes static at its target.
    pub fn tick(&mut self) {
        if let Extent::Moving {
            from,
            to,
            duration,
            progress,
            size,
            ..
        } = self.extent
        {
            let progress = progress - 1;
            let new_size = moving_size(from, to, duration, progress);
            self.extent = if progress <= 0 {
                Extent::Static(to)
            } else {
                Extent::Moving {
                    from,
                    to,
                    duration,
                    progress,
                    size: new_size,
                    previous: size,
                }
            };
        }
    }

    /// `getSize()`: the current size.
    #[must_use]
    pub fn size(&self) -> f64 {
        match self.extent {
            Extent::Static(s) => s,
            Extent::Moving { size, .. } => size,
        }
    }

    /// The border as collision reads it (`getMinX()` = `getMinX(0.0F)`): a moving border is at
    /// its size before the last [`BorderState::tick`].
    #[must_use]
    pub fn collision_border(&self) -> WorldBorder {
        let size = match self.extent {
            Extent::Static(s) => s,
            Extent::Moving { size, previous, .. } => lerp(0.0, previous, size),
        };
        WorldBorder {
            center_x: self.center_x,
            center_z: self.center_z,
            size,
        }
    }
}
