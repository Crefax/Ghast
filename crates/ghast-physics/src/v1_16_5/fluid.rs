//! 1.16.5 liquids: fluid heights, flow vectors and the per-tick fluid contact of the player.

use ghast_data::FluidKind;
use ghast_data::v1_16_5::BlockState;

use super::blocks::{FluidAt, blocks_movement, fluid, is_ice_material, side_solid, state_at};
use crate::math::{Aabb, floor, sqrt_f64_as_f32};
use crate::world::BlockView;

/// A plain `Vec3d` (1.16 has no zero-sign normalisation).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub(crate) struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl V3 {
    pub(crate) const ZERO: V3 = V3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    #[inline]
    pub(crate) fn new(x: f64, y: f64, z: f64) -> V3 {
        V3 { x, y, z }
    }

    #[inline]
    pub(crate) fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }

    #[inline]
    pub(crate) fn scale(self, k: f64) -> V3 {
        V3::new(self.x * k, self.y * k, self.z * k)
    }

    /// Length with vanilla's float square root.
    #[inline]
    pub(crate) fn length(self) -> f64 {
        f64::from(sqrt_f64_as_f32(
            self.x * self.x + self.y * self.y + self.z * self.z,
        ))
    }

    #[inline]
    pub(crate) fn length_sq(self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    /// Unit vector (float-rounded length); shorter than `1e-4` becomes zero.
    #[inline]
    pub(crate) fn normalize(self) -> V3 {
        let d = self.length();
        if d < 1.0e-4 {
            V3::ZERO
        } else {
            V3::new(self.x / d, self.y / d, self.z / d)
        }
    }
}

#[inline]
fn fluid_at<W: BlockView<State = BlockState>>(w: &W, x: i32, y: i32, z: i32) -> Option<FluidAt> {
    fluid(state_at(w, x, y, z))
}

/// Fluid surface fraction without neighbours (`amount / 9`, float).
#[inline]
fn own_height(f: FluidAt) -> f32 {
    f32::from(f.amount) / 9.0
}

/// Vanilla `FluidState.getHeight(world, pos)`: 1 when the same fluid is above.
pub(crate) fn height<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    f: FluidAt,
) -> f32 {
    if fluid_at(w, x, y + 1, z).is_some_and(|a| a.kind == f.kind) {
        1.0
    } else {
        own_height(f)
    }
}

/// Vanilla `FlowableFluid.getVelocity`: the normalised flow direction of the fluid at a block.
fn flow<W: BlockView<State = BlockState>>(w: &W, x: i32, y: i32, z: i32, f: FluidAt) -> V3 {
    const SIDES: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
    let here = own_height(f);
    let compatible = |n: Option<FluidAt>| n.is_none_or(|n| n.kind == f.kind);
    let (mut dx, mut dz) = (0.0f64, 0.0f64);
    for (ox, oz) in SIDES {
        let n = fluid_at(w, x + ox, y, z + oz);
        if !compatible(n) {
            continue;
        }
        let h = n.map_or(0.0, own_height);
        let mut diff = 0.0f32;
        if h == 0.0 {
            if !blocks_movement(state_at(w, x + ox, y, z + oz)) {
                let below = fluid_at(w, x + ox, y - 1, z + oz);
                if compatible(below) {
                    let hb = below.map_or(0.0, own_height);
                    if hb > 0.0 {
                        diff = here - (hb - 0.888_888_9);
                    }
                }
            }
        } else if h > 0.0 {
            diff = here - h;
        }
        if diff != 0.0 {
            dx += f64::from(ox as f32 * diff);
            dz += f64::from(oz as f32 * diff);
        }
    }
    let mut v = V3::new(dx, 0.0, dz);
    if f.falling {
        let blocks = |bx: i32, by: i32, bz: i32, ox_: i32, oz_: i32| {
            let s = state_at(w, bx, by, bz);
            if fluid(s).is_some_and(|n| n.kind == f.kind) {
                false
            } else {
                !is_ice_material(s) && side_solid(s, ox_, oz_)
            }
        };
        if SIDES.iter().any(|&(ox, oz)| {
            blocks(x + ox, y, z + oz, ox, oz) || blocks(x + ox, y + 1, z + oz, ox, oz)
        }) {
            v = v.normalize().add(V3::new(0.0, -6.0, 0.0));
        }
    }
    v.normalize()
}

/// Result of one fluid query (vanilla `updateMovementInFluid`).
pub(crate) struct FluidContact {
    /// Touching the fluid.
    pub touching: bool,
    /// Fluid height above the box bottom (0 when not touching).
    pub height: f64,
}

/// `true` when every chunk in the block range is loaded and the Y range overlaps the world.
pub(crate) fn region_loaded<W: BlockView<State = BlockState>>(
    w: &W,
    x0: i32,
    y0: i32,
    z0: i32,
    x1: i32,
    y1: i32,
    z1: i32,
) -> bool {
    // Every chunk counts as loaded on the 1.16 client; only the height range matters.
    let _ = (w, x0, z0, x1, z1);
    y1 >= 0 && y0 < 256
}

/// Vanilla `updateMovementInFluid`: contact, height and the flow push added to `vel` when the
/// player is pushed by fluids.
pub(crate) fn update_in_fluid<W: BlockView<State = BlockState>>(
    w: &W,
    bb: &Aabb,
    kind: FluidKind,
    speed: f64,
    pushed: bool,
    vel: &mut V3,
) -> FluidContact {
    let b = Aabb::new(
        bb.min_x + 0.001,
        bb.min_y + 0.001,
        bb.min_z + 0.001,
        bb.max_x - 0.001,
        bb.max_y - 0.001,
        bb.max_z - 0.001,
    );
    let (x0, x1) = (floor(b.min_x), ceil(b.max_x));
    let (y0, y1) = (floor(b.min_y), ceil(b.max_y));
    let (z0, z1) = (floor(b.min_z), ceil(b.max_z));
    let none = FluidContact {
        touching: false,
        height: 0.0,
    };
    if !region_loaded(w, x0, y0, z0, x1, y1, z1) {
        return none;
    }
    let mut height = 0.0f64;
    let mut touching = false;
    let mut push = V3::ZERO;
    let mut count = 0u32;
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                let Some(f) = fluid_at(w, x, y, z).filter(|f| f.kind == kind) else {
                    continue;
                };
                // `int + float`: a float sum.
                let top = f64::from(y as f32 + height_at(w, x, y, z, f));
                if top >= b.min_y {
                    touching = true;
                    height = (top - b.min_y).max(height);
                    if pushed {
                        let mut v = flow(w, x, y, z, f);
                        if height < 0.4 {
                            v = v.scale(height);
                        }
                        push = push.add(v);
                        count += 1;
                    }
                }
            }
        }
    }
    if push.length() > 0.0 {
        if count > 0 {
            push = push.scale(1.0 / f64::from(count));
        }
        push = push.scale(speed * 1.0);
        if vel.x.abs() < 0.003 && vel.z.abs() < 0.003 && push.length() < 0.004_500_000_000_000_000_5
        {
            push = push.normalize().scale(0.004_500_000_000_000_000_5);
        }
        *vel = vel.add(push);
    }
    FluidContact { touching, height }
}

#[inline]
fn height_at<W: BlockView<State = BlockState>>(w: &W, x: i32, y: i32, z: i32, f: FluidAt) -> f32 {
    height(w, x, y, z, f)
}

/// `MathHelper.ceil(double)`.
#[inline]
pub(crate) fn ceil(v: f64) -> i32 {
    let i = v as i32;
    if v > f64::from(i) { i + 1 } else { i }
}

/// The fluid in the block containing a point (vanilla `world.getFluidState(new BlockPos(..))`).
pub(crate) fn fluid_at_point<W: BlockView<State = BlockState>>(
    w: &W,
    x: f64,
    y: f64,
    z: f64,
) -> Option<(FluidAt, [i32; 3])> {
    let (bx, by, bz) = (floor(x), floor(y), floor(z));
    fluid_at(w, bx, by, bz).map(|f| (f, [bx, by, bz]))
}
