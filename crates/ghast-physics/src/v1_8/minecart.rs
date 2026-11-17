//! A minecart the local player sits in, as the 1.8.9 client ticks it.
//!
//! The server moves a minecart; the 1.8 client only interpolates it. The client's own part:
//!
//! 1. The cart's tick (`AbstractMinecartEntity.tick` on the client): one interpolation step
//!    towards the server's last position and rotation (every position packet sets 3 + 2 steps;
//!    a teleport within 1/32, 1/64 vertically, of the client's cart keeps its position), else it
//!    stays.
//! 2. The rider's tick (`Entity.tickRiding`): motion zeroed, the whole player tick (a minecart
//!    has no collision box for its rider, unlike a 1.8 boat), then `look` and `steer_vehicle`
//!    instead of the movement packets. The 1.8 client never gets off by itself.
//! 3. The cart places its rider (`updatePassengerPosition`): 0.35 below the cart's position
//!    (`getMountedHeightOffset` 0); the fall distance is reset.
//!
//! The server's position of the cart is kept in 1/32 blocks (`trackedX`…), as the relative moves
//! add to it.

use ghast_data::v1_8::BlockState;

use super::player::{Options, PlayerState};
use crate::input::{Packets, TickInput};
use crate::math::Aabb;
use crate::world::BlockView;

/// Minecart box width (`float`).
pub const MINECART_WIDTH: f32 = 0.98;
/// Minecart box height (`float`).
pub const MINECART_HEIGHT: f32 = 0.7;

/// A minecart the player sits in. Create it where the client has it ([`Minecart::new`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Minecart {
    /// Position.
    pub pos: [f64; 3],
    /// Box.
    pub bb: Aabb,
    /// Yaw.
    pub yaw: f32,
    /// Pitch.
    pub pitch: f32,
    /// The server's position of it in 1/32 blocks.
    pub tracked: [i32; 3],
    target: [f64; 3],
    target_yaw: f64,
    target_pitch: f64,
    steps: i32,
}

fn cart_box(p: [f64; 3]) -> Aabb {
    let r = f64::from(MINECART_WIDTH) / 2.0;
    Aabb::new(
        p[0] - r,
        p[1],
        p[2] - r,
        p[0] + r,
        p[1] + f64::from(MINECART_HEIGHT),
        p[2] + r,
    )
}

/// `MathHelper.wrapDegrees(double)`.
fn wrap_degrees(mut d: f64) -> f64 {
    d %= 360.0;
    if d >= 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

/// An angle byte as `float` degrees (`byte * 360 / 256.0F`).
fn angle(b: i8) -> f32 {
    (i32::from(b) * 360) as f32 / 256.0
}

impl Minecart {
    /// A minecart at `pos` with a rotation, not interpolating; its server position is `pos`
    /// (to 1/32).
    #[must_use]
    pub fn new(pos: [f64; 3], yaw: f32, pitch: f32) -> Minecart {
        Minecart {
            pos,
            bb: cart_box(pos),
            yaw,
            pitch,
            tracked: pos.map(|v| (v * 32.0).floor() as i32),
            target: pos,
            target_yaw: f64::from(yaw),
            target_pitch: f64::from(pitch),
            steps: 0,
        }
    }

    /// A minecart as the spawn-object packet creates it: at the packet's fixed-point position
    /// (`/ 32`), with its angle bytes.
    #[must_use]
    pub fn spawned(fixed: [i32; 3], yaw: i8, pitch: i8) -> Minecart {
        let mut m = Minecart::new(fixed.map(|v| f64::from(v) / 32.0), angle(yaw), angle(pitch));
        m.tracked = fixed;
        m
    }

    /// Steps of interpolation left.
    #[must_use]
    pub fn interpolation_steps(&self) -> i32 {
        self.steps
    }

    /// `AbstractMinecartEntity.updateTrackedPositionAndAngles(…, 3)`: 3 + 2 steps.
    fn retarget(&mut self, target: [f64; 3], yaw: f32, pitch: f32) {
        self.target = target;
        self.target_yaw = f64::from(yaw);
        self.target_pitch = f64::from(pitch);
        self.steps = 3 + 2;
    }

    /// The server's entity teleport (`onEntityPosition`): fixed-point position, angle bytes.
    pub fn server_teleport(&mut self, fixed: [i32; 3], yaw: i8, pitch: i8) {
        self.tracked = fixed;
        let [x, y, z] = fixed.map(|v| f64::from(v) / 32.0);
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let near = !((self.pos[0] - x).abs() >= 0.03125)
            && !((self.pos[1] - y).abs() >= 0.015625)
            && !((self.pos[2] - z).abs() >= 0.03125);
        let target = if near { self.pos } else { [x, y, z] };
        self.retarget(target, angle(yaw), angle(pitch));
    }

    /// The server's relative move, look, move-look or bare entity packet (`onEntityUpdate`):
    /// `delta` in 1/32 blocks, `rotation` the angle bytes when the packet has them.
    pub fn server_update(&mut self, delta: [i8; 3], rotation: Option<(i8, i8)>) {
        for (t, d) in self.tracked.iter_mut().zip(delta) {
            *t += i32::from(d);
        }
        let target = self.tracked.map(|v| f64::from(v) / 32.0);
        let (yaw, pitch) = rotation.map_or((self.yaw, self.pitch), |(y, p)| (angle(y), angle(p)));
        self.retarget(target, yaw, pitch);
    }

    /// The cart's client tick: one interpolation step.
    pub fn tick(&mut self) {
        if self.steps > 0 {
            let n = f64::from(self.steps);
            let x = self.pos[0] + (self.target[0] - self.pos[0]) / n;
            let y = self.pos[1] + (self.target[1] - self.pos[1]) / n;
            let z = self.pos[2] + (self.target[2] - self.pos[2]) / n;
            let g = wrap_degrees(self.target_yaw - f64::from(self.yaw));
            self.yaw = (f64::from(self.yaw) + g / n) as f32;
            self.pitch =
                (f64::from(self.pitch) + (self.target_pitch - f64::from(self.pitch)) / n) as f32;
            self.steps -= 1;
            self.pos = [x, y, z];
        }
        self.bb = cart_box(self.pos);
        self.yaw %= 360.0;
        self.pitch %= 360.0;
    }

    /// The rider's position after the cart's tick (`updatePassengerPosition`).
    #[must_use]
    pub fn passenger_pos(&self) -> [f64; 3] {
        [self.pos[0], self.pos[1] + 0.0 + -0.35, self.pos[2]]
    }
}

/// One client tick in the minecart (the module docs list the steps). Packets: `look`,
/// `steer_vehicle`.
pub fn tick_minecart<W: BlockView<State = BlockState>>(
    p: &mut PlayerState,
    cart: &mut Minecart,
    input: &TickInput,
    world: &W,
    options: &Options,
    out: &mut Packets,
) {
    cart.tick();
    super::player::tick_rider(p, input, world, options, &[], out);
    let [x, y, z] = cart.passenger_pos();
    p.set_position(x, y, z);
    p.fall_distance = 0.0;
}
