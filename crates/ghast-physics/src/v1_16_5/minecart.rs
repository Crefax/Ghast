//! A minecart the local player sits in, as the 1.16.5 client ticks it.
//!
//! The server moves a minecart: its primary passenger is not a controlling one
//! (`getPrimaryPassenger` is null for minecarts), so the client never sends `vehicle_move` for it
//! and only follows the server's position packets. The client's own part:
//!
//! 1. The cart's tick (`AbstractMinecartEntity.tick` on the client): one interpolation step
//!    towards the server's last position and rotation (a position packet sets the packet's 3
//!    steps + 2), else it stays.
//! 2. The passenger's tick (`tickRiding`): motion zeroed, the whole player tick (the passenger
//!    of a minecart touches water like anyone, no swimming, no auto-jump), then `look` and
//!    `steer_vehicle` instead of the movement packets. With the shift key held in the previous
//!    tick the client gets off by itself instead (`PlayerEntity.tickRiding`).
//! 3. The cart places its passenger (`updatePassengerPosition`): 0.35 below the cart's position
//!    (`getMountedHeightOffset` 0); the fall distance is reset. The cart does not turn its
//!    passenger.

use ghast_data::v1_16_5::BlockState;

use crate::math::Aabb;
use crate::world::BlockView;
use crate::{Packets, TickInput};

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
    /// The server's position of it (`trackedPosition`, what relative moves add to).
    pub tracked: [f64; 3],
    target: [f64; 3],
    target_yaw: f64,
    target_pitch: f64,
    steps: i32,
}

fn cart_box(p: [f64; 3]) -> Aabb {
    // EntityDimensions.getBoxAt: half width in float.
    let f = f64::from(MINECART_WIDTH / 2.0);
    Aabb::new(
        p[0] - f,
        p[1],
        p[2] - f,
        p[0] + f,
        p[1] + f64::from(MINECART_HEIGHT),
        p[2] + f,
    )
}

/// 1.16 `MathHelper.wrapDegrees(double)`.
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

/// The 1/4096 fixed point of relative moves (`EntityS2CPacket.encodePacketCoordinate`).
fn encode(v: f64) -> i64 {
    (v * 4096.0).floor() as i64
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
            tracked: pos,
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

    /// `AbstractMinecartEntity.updateTrackedPositionAndAngles`: towards `target` in the
    /// packet's steps + 2.
    fn retarget(&mut self, target: [f64; 3], yaw: f32, pitch: f32, steps: i32) {
        self.target = target;
        self.target_yaw = f64::from(yaw);
        self.target_pitch = f64::from(pitch);
        self.steps = steps + 2;
    }

    /// The server's entity teleport (`onEntityPosition`): the position, the angle bytes.
    pub fn server_teleport(&mut self, pos: [f64; 3], yaw: i8, pitch: i8) {
        self.tracked = pos;
        let yaw = f32::from(yaw) * 360.0 / 256.0;
        let pitch = f32::from(pitch) * 360.0 / 256.0;
        self.retarget(pos, yaw, pitch, 3);
    }

    /// The server's relative move, look or move-look (`onEntityUpdate`): `delta` in 1/4096
    /// blocks (`None`: a look only), `rotation` the angle bytes when the packet has them.
    pub fn server_update(&mut self, delta: Option<[i16; 3]>, rotation: Option<(i8, i8)>) {
        let angles =
            |(y, p): (i8, i8)| (f32::from(y) * 360.0 / 256.0, f32::from(p) * 360.0 / 256.0);
        match delta {
            Some(d) => {
                // calculateDeltaPosition: a zero delta keeps the coordinate as it is.
                if d != [0; 3] {
                    for k in 0..3 {
                        if d[k] != 0 {
                            self.tracked[k] =
                                (encode(self.tracked[k]) + i64::from(d[k])) as f64 / 4096.0;
                        }
                    }
                }
                let (yaw, pitch) = rotation.map_or((self.yaw, self.pitch), angles);
                self.retarget(self.tracked, yaw, pitch, 3);
            }
            None => {
                if let Some(r) = rotation {
                    let (yaw, pitch) = angles(r);
                    self.retarget(self.pos, yaw, pitch, 3);
                }
            }
        }
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
        // setPosition / refreshPosition, then setRotation.
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
/// step, the player's passenger tick (with `around` as for [`tick_ext`](super::tick_ext)) and the
/// seat. Packets: `look`, `steer_vehicle`.
///
/// Returns `false` when the player got off (the shift key held in the previous tick): the cart
/// has ticked, the player has not (only its age counted); it then ticks normally in the same
/// tick when it comes after the cart in the client world's entity order, else from the next tick
/// on.
pub fn tick_minecart<W: BlockView<State = BlockState>>(
    p: &mut super::PlayerState,
    x: &mut super::PlayerExt,
    cart: &mut Minecart,
    around: &super::Surroundings<'_>,
    input: &TickInput,
    world: &W,
    out: &mut Packets,
) -> bool {
    cart.tick();
    if p.input.sneaking {
        p.age += 1;
        p.yaw = input.yaw;
        p.pitch = input.pitch;
        return false;
    }
    let ride = super::player::RideCtx {
        bb: cart.bb,
        submerged: false,
        boat: false,
    };
    super::player::tick_passenger(p, x, around, ride, input, world, out);
    let [sx, sy, sz] = cart.passenger_pos();
    p.set_position(sx, sy, sz);
    p.fall_distance = 0.0;
    true
}
