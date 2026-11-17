//! A minecart the local player sits in, as the 1.12.2 client ticks it.
//!
//! The server moves a minecart (it has no controlling passenger, so the client never sends
//! `vehicle_move` for it). The client's own part:
//!
//! 1. The cart's tick (`AbstractMinecartEntity.tick` on the client): one interpolation step
//!    towards the server's last position and rotation (a position packet sets the packet's
//!    steps + 2), else it stays.
//! 2. The passenger's tick (`Entity.tickRiding`): motion zeroed, the whole player tick (a
//!    minecart's passenger touches water like anyone), then `look` and `steer_vehicle` instead
//!    of the movement packets. The 1.12 client never gets off by itself: sneaking only reaches
//!    the server in `steer_vehicle`.
//! 3. The cart places its passenger (`updatePassengerPosition`): 0.35 below the cart's position
//!    (`getMountedHeightOffset` 0); the fall distance is reset. The cart does not turn its
//!    passenger.
//!
//! Server packets ([`Minecart::server_teleport`], [`Minecart::server_update`]) keep the cart's
//! 1/4096 server position (`EntityTracker.updateServerPosition`, the relative moves' base) as the
//! client does: a teleport within 1/32 (1/64 vertically) of the client's cart only turns it.

use ghast_data::v1_12::BlockState;

use super::ext::PlayerExt;
use super::player::PlayerState;
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
    /// The server's position of it in 1/4096 blocks (`serverPosX`…).
    pub tracked: [i64; 3],
    target: [f64; 3],
    target_yaw: f64,
    target_pitch: f64,
    steps: i32,
}

fn cart_box(p: [f64; 3]) -> Aabb {
    super::player::box_at(p[0], p[1], p[2], MINECART_WIDTH, MINECART_HEIGHT)
}

/// `MathHelper.wrapDegrees(double)`.
fn wrap_degrees(d: f64) -> f64 {
    let mut e = d % 360.0;
    if e >= 180.0 {
        e -= 360.0;
    }
    if e < -180.0 {
        e += 360.0;
    }
    e
}

/// `EntityTracker.method_12764`: `lfloor(v * 4096)`.
fn fixed(v: f64) -> i64 {
    (v * 4096.0).floor() as i64
}

/// An angle byte as `float` degrees (`byte * 360 / 256.0F`).
fn angle(b: i8) -> f32 {
    (i32::from(b) * 360) as f32 / 256.0
}

impl Minecart {
    /// A minecart at `pos` with a rotation, not interpolating (as the spawn packet leaves it, or
    /// where the client's interpolation of it ended); its server position is `pos`.
    #[must_use]
    pub fn new(pos: [f64; 3], yaw: f32, pitch: f32) -> Minecart {
        Minecart {
            pos,
            bb: cart_box(pos),
            yaw,
            pitch,
            tracked: pos.map(fixed),
            target: pos,
            target_yaw: f64::from(yaw),
            target_pitch: f64::from(pitch),
            steps: 0,
        }
    }

    /// Steps of interpolation left.
    #[must_use]
    pub fn interpolation_steps(&self) -> i32 {
        self.steps
    }

    /// `AbstractMinecartEntity.updateTrackedPositionAndAngles`: the packet's steps + 2.
    fn retarget(&mut self, target: [f64; 3], yaw: f32, pitch: f32, steps: i32) {
        self.target = target;
        self.target_yaw = f64::from(yaw);
        self.target_pitch = f64::from(pitch);
        self.steps = steps + 2;
    }

    /// The server's entity teleport (`onEntityPosition`): the position and the angle bytes.
    pub fn server_teleport(&mut self, pos: [f64; 3], yaw: i8, pitch: i8) {
        self.tracked = pos.map(fixed);
        let (yaw, pitch) = (angle(yaw), angle(pitch));
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let near = !((self.pos[0] - pos[0]).abs() >= 0.03125)
            && !((self.pos[1] - pos[1]).abs() >= 0.015625)
            && !((self.pos[2] - pos[2]).abs() >= 0.03125);
        if near {
            self.retarget(self.pos, yaw, pitch, 0);
        } else {
            self.retarget(pos, yaw, pitch, 3);
        }
    }

    /// The server's relative move, look, move-look or bare entity packet (`onEntityUpdate`):
    /// `delta` in 1/4096 blocks (zero for a look), `rotation` the angle bytes when the packet
    /// has them. Each retargets the cart to its server position.
    pub fn server_update(&mut self, delta: [i16; 3], rotation: Option<(i8, i8)>) {
        for (t, d) in self.tracked.iter_mut().zip(delta) {
            *t += i64::from(d);
        }
        let target = self.tracked.map(|v| v as f64 / 4096.0);
        let (yaw, pitch) = rotation.map_or((self.yaw, self.pitch), |(y, p)| (angle(y), angle(p)));
        self.retarget(target, yaw, pitch, 3);
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
        // updatePosition, then setRotation.
        self.bb = cart_box(self.pos);
        self.yaw %= 360.0;
        self.pitch %= 360.0;
    }

    /// The passenger's position after the cart's tick (`updatePassengerPosition`).
    #[must_use]
    pub fn passenger_pos(&self) -> [f64; 3] {
        [self.pos[0], self.pos[1] + 0.0 + -0.35, self.pos[2]]
    }
}

/// One client tick in the minecart (the module docs list the steps): the cart's interpolation
/// step, the player's passenger tick (with `entities`: the boxes it collides with, as for
/// [`tick_ext`](super::tick_ext)) and the seat. Packets: `look`, `steer_vehicle`.
pub fn tick_minecart<W: BlockView<State = BlockState>>(
    p: &mut PlayerState,
    x: &mut PlayerExt,
    cart: &mut Minecart,
    entities: &[Aabb],
    input: &TickInput,
    world: &W,
    out: &mut Packets,
) {
    cart.tick();
    super::player::tick_cart_passenger(p, x, entities, input, world, out);
    let [sx, sy, sz] = cart.passenger_pos();
    p.set_position(sx, sy, sz);
    p.fall_distance = 0.0;
}
