//! Fluids of the modern client: `EntityFluidInteraction` (one sweep for height, eyes and current),
//! fluid heights and `FlowingFluid.getFlow`.

use ghast_data::v26_1::{StatePhys, block_flags as bf, state_flags as sf};

use super::collide::{Extra, World};
use super::math::{V3, ceil, floor};
use super::rules::Rules;
use crate::math::Aabb;
use crate::world::BlockView;

/// Water (1) or lava (2) in a fluid byte; 0 = none.
#[inline]
pub(crate) fn kind(f: u8) -> u8 {
    f >> 6
}

/// Fluid amount `1..=8`.
#[inline]
pub(crate) fn amount(f: u8) -> u8 {
    f & 15
}

/// `FluidState.getOwnHeight()`: `amount / 9.0F`.
#[inline]
pub(crate) fn own_height(f: u8) -> f32 {
    f32::from(amount(f)) / 9.0
}

/// Per-fluid tracker result.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Tracker {
    pub height: f64,
    pub eyes_inside: bool,
    current: V3,
    count: u32,
}

/// The result of one `EntityFluidInteraction.update`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Interaction {
    pub water: Tracker,
    pub lava: Tracker,
}

impl Tracker {
    /// `Tracker.applyCurrentTo` for a player: the averaged current, scaled, with vanilla's minimum
    /// push when the entity barely moves.
    pub(crate) fn current_impulse(&self, vel: V3, scale: f64) -> Option<V3> {
        if self.count == 0 || self.current.length_sqr() < f64::from(1.0e-5f32) {
            return None;
        }
        let mut imp = self.current.scale(1.0 / f64::from(self.count)).scale(scale);
        if vel.x.abs() < 0.003 && vel.z.abs() < 0.003 && imp.length() < 0.004_500_000_000_000_000_5
        {
            imp = imp.normalize().scale(0.004_500_000_000_000_000_5);
        }
        Some(imp)
    }
}

impl<R: Rules, W: BlockView<State = R::State>, E: Extra> World<'_, R, W, E> {
    /// The fluid byte at a position (`Level.getFluidState`).
    #[inline]
    pub(crate) fn fluid(&self, x: i32, y: i32, z: i32) -> u8 {
        self.phys(x, y, z).fluid()
    }

    /// `FluidState.getHeight(level, pos)`: 1 with the same fluid above, else `amount / 9`.
    #[inline]
    pub(crate) fn fluid_height(&self, x: i32, y: i32, z: i32, f: u8) -> f32 {
        if kind(self.fluid(x, y + 1, z)) == kind(f) {
            1.0
        } else {
            own_height(f)
        }
    }

    /// `EntityFluidInteraction.update(entity, ignoreCurrent)`.
    pub(crate) fn fluid_interaction(
        &self,
        bb: &Aabb,
        block_xz: [i32; 2],
        eye_y: f64,
        ignore_current: bool,
    ) -> Interaction {
        let mut out = Interaction::default();
        // getFluidInteractionBox: the box deflated by 0.001.
        let b = super::math::deflate(bb, 0.001);
        let (x0, y0, z0) = (floor(b.min_x), floor(b.min_y), floor(b.min_z));
        let (x1, y1, z1) = (ceil(b.max_x) - 1, ceil(b.max_y) - 1, ceil(b.max_z) - 1);
        if !self.fluid_region_loaded(x0 - 1, y0, z0 - 1, x1 + 1, y1, z1 + 1) {
            return out;
        }
        let entity_y = bb.min_y;
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    let f = self.fluid(x, y, z);
                    let k = kind(f);
                    if k == 0 {
                        continue;
                    }
                    let bottom = f64::from(y);
                    let top = bottom + f64::from(self.fluid_height(x, y, z, f));
                    if top < b.min_y {
                        continue;
                    }
                    let t = if k == 1 {
                        &mut out.water
                    } else {
                        &mut out.lava
                    };
                    if x == block_xz[0] && z == block_xz[1] && eye_y >= bottom && eye_y <= top {
                        t.eyes_inside = true;
                    }
                    t.height = super::math::java_max(top - entity_y, t.height);
                    if !ignore_current {
                        let mut flow = self.flow(x, y, z, f);
                        if t.height < 0.4 {
                            flow = flow.scale(t.height);
                        }
                        t.current = t.current.add(flow);
                        t.count += 1;
                    }
                }
            }
        }
        out
    }

    /// `hasFluidAndLoaded`: every chunk of the range is loaded. (Vanilla also skips the sweep when
    /// no section in the range holds a fluid, which cannot change the result.)
    fn fluid_region_loaded(&self, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) -> bool {
        let _ = (y0, y1);
        for cz in (z0 >> 4)..=(z1 >> 4) {
            for cx in (x0 >> 4)..=(x1 >> 4) {
                if !self.w.is_chunk_loaded(cx, cz) {
                    return false;
                }
            }
        }
        true
    }

    /// `FlowingFluid.getFlow(level, pos, fluidState)`.
    pub(crate) fn flow(&self, x: i32, y: i32, z: i32, f: u8) -> V3 {
        // Direction.Plane.HORIZONTAL order: north, east, south, west.
        const SIDES: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
        let k = kind(f);
        let here = own_height(f);
        let affects = |n: u8| kind(n) == 0 || kind(n) == k;
        let (mut fx, mut fz) = (0.0f64, 0.0f64);
        for (dx, dz) in SIDES {
            let (nx, nz) = (x + dx, z + dz);
            let np = self.phys(nx, y, nz);
            let n = np.fluid();
            if !affects(n) {
                continue;
            }
            let mut nh = if kind(n) == 0 { 0.0 } else { own_height(n) };
            let mut dist = 0.0f32;
            if nh == 0.0 {
                if np.flags() & sf::BLOCKS_MOTION == 0 {
                    let below = self.fluid(nx, y - 1, nz);
                    if affects(below) {
                        nh = if kind(below) == 0 {
                            0.0
                        } else {
                            own_height(below)
                        };
                        if nh > 0.0 {
                            dist = here - (nh - 0.888_888_9);
                        }
                    }
                }
            } else if nh > 0.0 {
                dist = here - nh;
            }
            if dist != 0.0 {
                fx += f64::from(dx as f32 * dist);
                fz += f64::from(dz as f32 * dist);
            }
        }
        let mut flow = V3::new(fx, 0.0, fz);
        if f & 16 != 0 {
            for (i, (dx, dz)) in SIDES.into_iter().enumerate() {
                if self.solid_face(x + dx, y, z + dz, i, k)
                    || self.solid_face(x + dx, y + 1, z + dz, i, k)
                {
                    flow = flow.normalize().add(V3::new(0.0, -6.0, 0.0));
                    break;
                }
            }
        }
        flow.normalize()
    }

    /// `FlowingFluid.isSolidFace(level, pos, direction)` for a horizontal direction index.
    fn solid_face(&self, x: i32, y: i32, z: i32, side: usize, k: u8) -> bool {
        let s = self.state(x, y, z);
        let p: StatePhys = R::phys(s);
        if kind(p.fluid()) == k {
            return false;
        }
        if R::block(s).flags & bf::ICE != 0 {
            return false;
        }
        p.flags() & (sf::STURDY_NORTH << side) != 0
    }

    /// `containsAnyLiquid(box)`.
    pub(crate) fn contains_any_liquid(&self, b: &Aabb) -> bool {
        let (x0, x1) = (floor(b.min_x), ceil(b.max_x));
        let (y0, y1) = (floor(b.min_y), ceil(b.max_y));
        let (z0, z1) = (floor(b.min_z), ceil(b.max_z));
        for x in x0..x1 {
            for y in y0..y1 {
                for z in z0..z1 {
                    if kind(self.fluid(x, y, z)) != 0 {
                        return true;
                    }
                }
            }
        }
        false
    }
}
