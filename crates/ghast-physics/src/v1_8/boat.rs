//! The boat the local player rides, as the 1.8.9 client ticks it.
//!
//! The 1.8 client simulates the boat its player rides: the attach packet that puts the local
//! player on a boat clears the boat's "empty" flag, and the boat's tick then runs the physics
//! instead of following the server. The server still decides where the boat is: its teleport
//! snaps the boat (and zeroes its motion), a relative move restores the last motion the server
//! sent when the boat is more than a block off (otherwise it is ignored), its velocity packet
//! sets the motion; teleports and relative moves also set the boat's ground flag.
//!
//! # A tick while riding, in vanilla order
//!
//! 1. The boat's tick (`BoatEntity.tick`): the entity base tick (the water flow push, lava
//!    halving the fall distance, removed below y = -64), how deep it is in water (five slices of
//!    its box), buoyancy, the rider's forward input along the rider's yaw (the rider's
//!    `forwardSpeed` / `sidewaysSpeed` of the PREVIOUS tick), the speed cap of 0.35 and the
//!    speed factor that grows while it accelerates, snow layers and lily pads at its four corners
//!    removed from the client's world, halved on the ground, its move (block collisions, no
//!    step-up, slime bounce, cobweb and soul sand), drag unless it hit something at speed, then
//!    its yaw turned towards its motion by at most 20 degrees.
//! 2. The rider's tick (`Entity.tickRiding`): its motion is zeroed, the whole player tick runs
//!    (in water when its box is), then the client sends `look` and `steer_vehicle` instead of the
//!    movement packets.
//! 3. The boat places its rider (`updatePassengerPosition`): 0.4 ahead along the boat's yaw,
//!    0.65 below the boat's position; the rider's fall distance is reset.
//!
//! The 1.8 client never dismounts by itself: sneaking only reaches the server in
//! `steer_vehicle`, whose attach packet then takes the player off (it stays where it sat).
//!
//! # The JVM's `Math.sin` / `Math.cos`
//!
//! Unlike the rest of 1.8 movement the boat calls `Math.sin` / `Math.cos` (the rider's input,
//! the seat), whose last bit depends on the client's JVM (HotSpot's intrinsics are not
//! correctly rounded and differ between versions and platforms). [`tick_boat`] uses the
//! platform's correctly rounded functions; [`tick_boat_with`] takes the values from a
//! [`JavaTrig`] (the oracle replay feeds the JVM's).

// Vanilla's negated float comparisons are kept as written: they decide the NaN cases.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use ghast_data::v1_8::BlockState;

use super::atan_table::{ARCSINE, COSINE};
use super::blocks::{
    BoxList, BoxSink, add_collision_boxes, id, is_air, is_tall_support, is_valid_pos, is_water,
    state_at,
};
use super::fluid::{contains_lava, water_contact};
use super::player::{Options, PlayerState};
use crate::input::{Packets, TickInput};
use crate::math::{Aabb, BoxOps, floor};
use crate::world::{BlockView, WorldBorder};

/// Boat box width (`float`).
pub const BOAT_WIDTH: f32 = 1.5;
/// Boat box height (`float`).
pub const BOAT_HEIGHT: f32 = 0.6;

/// `Math.sin` / `Math.cos` as the client's JVM computes them (see the module docs).
pub trait JavaTrig {
    /// `Math.sin(x)`.
    fn sin(&self, x: f64) -> f64;
    /// `Math.cos(x)`.
    fn cos(&self, x: f64) -> f64;
}

/// The platform's correctly rounded sine and cosine.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdTrig;

impl JavaTrig for StdTrig {
    #[inline]
    fn sin(&self, x: f64) -> f64 {
        x.sin()
    }
    #[inline]
    fn cos(&self, x: f64) -> f64 {
        x.cos()
    }
}

/// A boat the player rides. Create it from the spawn packet ([`Boat::new`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Boat {
    /// Position.
    pub pos: [f64; 3],
    /// Box.
    pub bb: Aabb,
    /// Velocity.
    pub vel: [f64; 3],
    /// Yaw.
    pub yaw: f32,
    /// Pitch.
    pub pitch: f32,
    /// On the ground (its last move, or the server's last teleport / relative move).
    pub on_ground: bool,
    /// Horizontal collision of its last move (cleared by a snow layer or lily pad it removed).
    pub horizontal_collision: bool,
    /// Vertical collision of its last move.
    pub vertical_collision: bool,
    /// Fall distance.
    pub fall_distance: f32,
    /// Destroyed below y = -64 (its rider gets off).
    pub removed: bool,
    /// The speed factor of the rider's input (0.07 – 0.35).
    speed: f64,
    in_web: bool,
    outside_world_border: bool,
    /// The last motion the server sent (restored by a far relative move).
    server_vel: [f64; 3],
    /// The server's position of it in 1/32 blocks (`trackedX`…: the spawn's, the teleports',
    /// plus the relative moves).
    tracked: [i32; 3],
    /// Blocks the boat removed in the last tick (the caller sets them to air in its world).
    broken: Vec<[i32; 3]>,
}

fn boat_box(x: f64, y: f64, z: f64) -> Aabb {
    let r = f64::from(BOAT_WIDTH) / 2.0;
    Aabb::new(x - r, y, z - r, x + r, y + f64::from(BOAT_HEIGHT), z + r)
}

impl Boat {
    /// A boat as the spawn-object packet creates it: at the packet's fixed-point position
    /// (`/ 32`), with its angle bytes (`* 360 / 256`), not moving.
    #[must_use]
    pub fn spawned(fixed: [i32; 3], yaw: i8, pitch: i8) -> Boat {
        let [x, y, z] = fixed.map(|v| f64::from(v) / 32.0);
        let mut b = Boat::new(
            [x, y, z],
            f32::from(yaw) * 360.0 / 256.0,
            f32::from(pitch) * 360.0 / 256.0,
        );
        b.tracked = fixed;
        b
    }

    /// A boat at `pos` with a rotation, not moving (its server position the nearest 1/32).
    #[must_use]
    pub fn new(pos: [f64; 3], yaw: f32, pitch: f32) -> Boat {
        Boat {
            tracked: pos.map(|v| (v * 32.0).floor() as i32),
            pos,
            bb: boat_box(pos[0], pos[1], pos[2]),
            vel: [0.0; 3],
            yaw,
            pitch,
            on_ground: false,
            horizontal_collision: false,
            vertical_collision: false,
            fall_distance: 0.0,
            removed: false,
            speed: 0.07,
            in_web: false,
            outside_world_border: false,
            server_vel: [0.0; 3],
            broken: Vec::new(),
        }
    }

    /// The speed factor of the rider's input (`0.07` at rest, up to `0.35`).
    #[must_use]
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// The snow layers and lily pads the boat removed from the client's world in the last tick:
    /// set them to air in the world.
    #[must_use]
    pub fn broken_blocks(&self) -> &[[i32; 3]] {
        &self.broken
    }

    /// Moves the boat to a position (vanilla `updatePosition`).
    pub fn set_position(&mut self, x: f64, y: f64, z: f64) {
        self.pos = [x, y, z];
        self.bb = boat_box(x, y, z);
    }

    /// The server's entity teleport (`onEntityPosition` with a rider): the fixed-point position
    /// (`/ 32`; kept when within 1/32, 1/64 vertically, of the client's), the angle bytes, the
    /// motion zeroed, the packet's ground flag.
    pub fn server_teleport(&mut self, fixed: [i32; 3], yaw: i8, pitch: i8, on_ground: bool) {
        self.tracked = fixed;
        let [x, y, z] = fixed.map(|v| f64::from(v) / 32.0);
        let yaw = f32::from(yaw) * 360.0 / 256.0;
        let pitch = f32::from(pitch) * 360.0 / 256.0;
        let near = !((self.pos[0] - x).abs() >= 0.03125)
            && !((self.pos[1] - y).abs() >= 0.015625)
            && !((self.pos[2] - z).abs() >= 0.03125);
        let (x, y, z) = if near {
            (self.pos[0], self.pos[1], self.pos[2])
        } else {
            (x, y, z)
        };
        self.yaw = yaw;
        self.pitch = pitch;
        self.set_position(x, y, z);
        self.vel = [0.0; 3];
        self.server_vel = [0.0; 3];
        self.on_ground = on_ground;
    }

    /// The server's relative move (or look) of the boat (`onEntityUpdate`, the deltas in 1/32
    /// blocks). More than a block from the client's boat, the last server motion is restored;
    /// the packet's ground flag is taken either way.
    pub fn server_move(&mut self, delta: [i8; 3], on_ground: bool) {
        for (t, d) in self.tracked.iter_mut().zip(delta) {
            *t += i32::from(d);
        }
        let tracked = self.tracked.map(|v| f64::from(v) / 32.0);
        let d = [
            tracked[0] - self.pos[0],
            tracked[1] - self.pos[1],
            tracked[2] - self.pos[2],
        ];
        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 1.0 {
            self.vel = self.server_vel;
        }
        self.on_ground = on_ground;
    }

    /// The server's entity velocity (`setVelocityClient`, `packet / 8000`).
    pub fn server_velocity(&mut self, v: [f64; 3]) {
        self.vel = v;
        self.server_vel = v;
    }

    /// The rider's position after the boat moved (`updatePassengerPosition`): 0.4 ahead along
    /// the boat's yaw, `-0.3 - 0.35` below it.
    #[must_use]
    pub fn seat<T: JavaTrig>(&self, trig: &T) -> [f64; 3] {
        let a = f64::from(self.yaw) * std::f64::consts::PI / 180.0;
        let dx = trig.cos(a) * 0.4;
        let dz = trig.sin(a) * 0.4;
        [
            self.pos[0] + dx,
            self.pos[1] + -0.3 + -0.35,
            self.pos[2] + dz,
        ]
    }
}

/// One client tick while riding `boat` (the module docs list the steps), with the platform's
/// sine and cosine ([`StdTrig`]). Packets: `look`, `steer_vehicle`. Snow layers and lily pads
/// the boat removed are in [`Boat::broken_blocks`].
///
/// Returns `false` when the boat was destroyed below y = -64: the player got off without its own
/// tick (only its age counted) and walks from the next tick on.
pub fn tick_boat<W: BlockView<State = BlockState>>(
    p: &mut PlayerState,
    boat: &mut Boat,
    input: &TickInput,
    world: &W,
    options: &Options,
    out: &mut Packets,
) -> bool {
    tick_boat_with(p, boat, input, world, options, &StdTrig, out)
}

/// [`tick_boat`] with the JVM's `Math.sin` / `Math.cos` from `trig`.
pub fn tick_boat_with<W: BlockView<State = BlockState>, T: JavaTrig>(
    p: &mut PlayerState,
    boat: &mut Boat,
    input: &TickInput,
    world: &W,
    options: &Options,
    trig: &T,
    out: &mut Packets,
) -> bool {
    // The mouse turned the rider before the tick; the boat reads its yaw.
    p.yaw = input.yaw;
    p.pitch = input.pitch;
    boat_tick(boat, p, world, options, trig);
    if boat.removed {
        // Entity.tickRiding: the vehicle is gone, the rider gets off without ticking.
        p.ticks_alive += 1;
        return false;
    }
    // The rider collides with its own boat (1.8's `doesBoxCollide` compares the entity list,
    // not its entries, with the rider's vehicle).
    let solid = [boat.bb];
    let gone = std::mem::take(&mut boat.broken);
    if gone.is_empty() {
        super::player::tick_rider(p, input, world, options, &solid, out);
    } else {
        let w = Gone {
            w: world,
            gone: &gone,
        };
        super::player::tick_rider(p, input, &w, options, &solid, out);
    }
    boat.broken = gone;
    let [x, y, z] = boat.seat(trig);
    p.set_position(x, y, z);
    p.fall_distance = 0.0;
    true
}

/// The world as the rest of a tick sees it after the boat removed blocks: air there.
struct Gone<'a, W> {
    w: &'a W,
    gone: &'a [[i32; 3]],
}

impl<W: BlockView<State = BlockState>> BlockView for Gone<'_, W> {
    type State = BlockState;

    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> BlockState {
        if self.gone.contains(&[x, y, z]) {
            BlockState::AIR
        } else {
            self.w.block(x, y, z)
        }
    }

    #[inline]
    fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.w.is_chunk_loaded(chunk_x, chunk_z)
    }

    #[inline]
    fn world_border(&self) -> WorldBorder {
        self.w.world_border()
    }
}

/// The 1.8 water test of `World.containsBlockWithMaterial(box, WATER)`: a water block in the
/// box's block range whose surface (`y + 1 - level / 8`, a full block for falling water) is at or
/// above the box's bottom.
fn contains_water<W: BlockView<State = BlockState>>(w: &W, b: &Aabb) -> bool {
    let (x0, x1) = (floor(b.min_x), floor(b.max_x + 1.0));
    let (y0, y1) = (floor(b.min_y), floor(b.max_y + 1.0));
    let (z0, z1) = (floor(b.min_z), floor(b.max_z + 1.0));
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                let s = state_at(w, x, y, z);
                if is_water(s) {
                    let level = s.meta();
                    let mut top = f64::from(y + 1);
                    if level < 8 {
                        top = f64::from(y + 1) - f64::from(level) / 8.0;
                    }
                    if top >= b.min_y {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// `MathHelper.atan2` of 1.8 (table-driven, see `atan_table`).
fn atan2(mut y: f64, mut x: f64) -> f64 {
    let d = x * x + y * y;
    if d.is_nan() {
        return f64::NAN;
    }
    let neg_y = y < 0.0;
    if neg_y {
        y = -y;
    }
    let neg_x = x < 0.0;
    if neg_x {
        x = -x;
    }
    let swap = y > x;
    if swap {
        std::mem::swap(&mut x, &mut y);
    }
    let f = fast_inverse_sqrt(d);
    x *= f;
    y *= f;
    let shift = f64::from_bits(4_805_340_802_404_319_232);
    let g = shift + y;
    let i = g.to_bits() as i32 as usize;
    let h = f64::from_bits(ARCSINE[i]);
    let j = f64::from_bits(COSINE[i]);
    let k = g - shift;
    let l = y * j - x * k;
    let m = (6.0 + l * l) * l * 0.166_666_666_666_666_66;
    let mut n = h + m;
    if swap {
        n = std::f64::consts::FRAC_PI_2 - n;
    }
    if neg_x {
        n = std::f64::consts::PI - n;
    }
    if neg_y {
        n = -n;
    }
    n
}

/// `MathHelper.fastInverseSqrt`.
fn fast_inverse_sqrt(x: f64) -> f64 {
    let d = 0.5 * x;
    let l = 6_910_469_410_427_058_090_i64 - ((x.to_bits() as i64) >> 1);
    let x = f64::from_bits(l as u64);
    x * (1.5 - d * x * x)
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

/// `BoatEntity.tick` of the boat the client simulates, its rider the local player.
fn boat_tick<W: BlockView<State = BlockState>, T: JavaTrig>(
    b: &mut Boat,
    rider: &PlayerState,
    w: &W,
    options: &Options,
    trig: &T,
) {
    b.broken.clear();
    // Entity.baseTick.
    let prev = b.pos;
    let probe =
        b.bb.grow(0.0, -f64::from(0.4f32), 0.0)
            .shrink(0.001, 0.001, 0.001);
    let (touching, flow) = water_contact(w, &probe);
    if touching {
        if flow.length() > 0.0 {
            let f = flow.normalize();
            const PUSH: f64 = 0.014;
            b.vel[0] += f.x * PUSH;
            b.vel[1] += f.y * PUSH;
            b.vel[2] += f.z * PUSH;
        }
        b.fall_distance = 0.0;
    }
    let lava =
        b.bb.grow(-f64::from(0.1f32), -f64::from(0.4f32), -f64::from(0.1f32));
    if contains_lava(w, &lava) {
        b.fall_distance *= 0.5;
    }
    if b.pos[1] < -64.0 {
        b.removed = true;
    }

    // How deep it is: five horizontal slices of its box, 0.125 lower.
    let mut depth = 0.0;
    let bb = b.bb;
    for j in 0..5 {
        let lo = bb.min_y + (bb.max_y - bb.min_y) * f64::from(j) / 5.0 - 0.125;
        let hi = bb.min_y + (bb.max_y - bb.min_y) * f64::from(j + 1) / 5.0 - 0.125;
        if contains_water(
            w,
            &Aabb::new(bb.min_x, lo, bb.min_z, bb.max_x, hi, bb.max_z),
        ) {
            depth += 1.0 / 5.0;
        }
    }
    let speed_before = (b.vel[0] * b.vel[0] + b.vel[2] * b.vel[2]).sqrt();

    if depth < 1.0 {
        let z = depth * 2.0 - 1.0;
        b.vel[1] += f64::from(0.04f32) * z;
    } else {
        if b.vel[1] < 0.0 {
            b.vel[1] /= 2.0;
        }
        b.vel[1] += f64::from(0.007f32);
    }

    // The rider's input along its yaw (its speeds of the previous tick).
    let aa = rider.yaw + -rider.sideways_speed * 90.0;
    let arg = f64::from(aa * std::f32::consts::PI / 180.0);
    let fwd = f64::from(rider.forward_speed);
    let k = f64::from(0.05f32);
    b.vel[0] += -trig.sin(arg) * b.speed * fwd * k;
    b.vel[2] += trig.cos(arg) * b.speed * fwd * k;

    let mut speed = (b.vel[0] * b.vel[0] + b.vel[2] * b.vel[2]).sqrt();
    if speed > 0.35 {
        let c = 0.35 / speed;
        b.vel[0] *= c;
        b.vel[2] *= c;
        speed = 0.35;
    }
    if speed > speed_before && b.speed < 0.35 {
        b.speed += (0.35 - b.speed) / 35.0;
        if b.speed > 0.35 {
            b.speed = 0.35;
        }
    } else {
        b.speed -= (b.speed - 0.07) / 35.0;
        if b.speed < 0.07 {
            b.speed = 0.07;
        }
    }

    // Snow layers and lily pads at the four corners, at its height and one above.
    for corner in 0..4 {
        let cx = floor(b.pos[0] + (f64::from(corner % 2) - 0.5) * 0.8);
        let cz = floor(b.pos[2] + (f64::from(corner / 2) - 0.5) * 0.8);
        for dy in 0..2 {
            let cy = floor(b.pos[1]) + dy;
            let at = [cx, cy, cz];
            let s = if b.broken.contains(&at) {
                BlockState::AIR
            } else {
                state_at(w, cx, cy, cz)
            };
            if matches!(s.block().id(), id::SNOW_LAYER | id::LILY_PAD) {
                b.broken.push(at);
                b.horizontal_collision = false;
            }
        }
    }

    if b.on_ground {
        b.vel[0] *= 0.5;
        b.vel[1] *= 0.5;
        b.vel[2] *= 0.5;
    }
    let gone = std::mem::take(&mut b.broken);
    let [vx, vy, vz] = b.vel;
    let rider_box = rider.bb;
    if gone.is_empty() {
        boat_move(b, w, options, &rider_box, vx, vy, vz);
    } else {
        boat_move(b, &Gone { w, gone: &gone }, options, &rider_box, vx, vy, vz);
    }
    b.broken = gone;
    if !b.horizontal_collision || !(speed_before > 0.2975) {
        b.vel[0] *= f64::from(0.99f32);
        b.vel[1] *= f64::from(0.95f32);
        b.vel[2] *= f64::from(0.99f32);
    }

    // The yaw turns towards the motion by at most 20 degrees.
    b.pitch = 0.0;
    let mut target = f64::from(b.yaw);
    let (dx, dz) = (prev[0] - b.pos[0], prev[2] - b.pos[2]);
    if dx * dx + dz * dz > 0.001 {
        target = f64::from((atan2(dz, dx) * 180.0 / std::f64::consts::PI) as f32);
    }
    let turn = wrap_degrees(target - f64::from(b.yaw)).clamp(-20.0, 20.0);
    b.yaw = (f64::from(b.yaw) + turn) as f32;
    // Entity.setRotation.
    b.yaw %= 360.0;
    b.pitch %= 360.0;
}

/// `World.doesBoxCollide` for the boat: the block boxes intersecting `query` (the 1.8 scan; outside
/// the world border blocks collide like stone while the boat is inside the world; lily pads have
/// no box for a boat), then its rider's box (`getHardCollisionBox`: any entity's box; 1.8 does
/// not leave the rider out). Other entities are not modelled on 1.8.9.
fn gather<W: BlockView<State = BlockState>>(
    b: &mut Boat,
    w: &W,
    border: &WorldBorder,
    options: &Options,
    rider: &Aabb,
    query: Aabb,
    out: &mut BoxList,
) {
    out.clear();
    let (x0, x1) = (floor(query.min_x), floor(query.max_x + 1.0));
    let (y0, y1) = (floor(query.min_y), floor(query.max_y + 1.0));
    let (z0, z1) = (floor(query.min_z), floor(query.max_z + 1.0));
    let was_outside = b.outside_world_border;
    let inside = {
        let m = if was_outside { 1.0 } else { -1.0 };
        let (wx, nz, ex, sz) = (
            border.west() + m,
            border.north() + m,
            border.east() - m,
            border.south() - m,
        );
        let (x, z) = (b.pos[0], b.pos[2]);
        x > wx && x < ex && z > nz && z < sz
    };
    let mut visited = false;
    let mut sink = BoxSink {
        query,
        out,
        history: options.history_shapes,
    };
    for x in x0..x1 {
        for z in z0..z1 {
            if !is_valid_pos(x, 64, z) {
                continue;
            }
            visited = true;
            let substitute = inside && !border.contains_block(x, z);
            for y in (y0 - 1)..y1 {
                let s = if substitute {
                    STONE
                } else {
                    state_at(w, x, y, z)
                };
                // LilyPadBlock.appendCollisionBoxes: no box for a boat.
                if s.block().id() != id::LILY_PAD {
                    add_collision_boxes(w, x, y, z, s, &mut sink);
                }
            }
        }
    }
    if visited {
        if was_outside && inside {
            b.outside_world_border = false;
        } else if !was_outside && !inside {
            b.outside_world_border = true;
        }
    }
    if rider.intersects(&query) {
        sink.out.push(*rider);
    }
}

const STONE: BlockState = ghast_data::v1_8::Block::STONE.default_state();

/// `Entity.move` of the boat: no step-up (step height 0), no sneak guard, no stepping effects;
/// the boat's own fall; slime bounces it.
fn boat_move<W: BlockView<State = BlockState>>(
    b: &mut Boat,
    w: &W,
    options: &Options,
    rider: &Aabb,
    mut dx: f64,
    mut dy: f64,
    mut dz: f64,
) {
    if b.in_web {
        b.in_web = false;
        dx *= 0.25;
        dy *= f64::from(0.05f32);
        dz *= 0.25;
        b.vel = [0.0; 3];
    }
    let (want_x, want_y, want_z) = (dx, dy, dz);
    let border = w.world_border();
    let mut boxes = BoxList::new();
    let query = b.bb.stretch(dx, dy, dz);
    gather(b, w, &border, options, rider, query, &mut boxes);
    let boxes = boxes.as_slice();
    let mut bb = b.bb;
    for o in boxes {
        dy = o.clip_y(&bb, dy);
    }
    bb = bb.offset(0.0, dy, 0.0);
    for o in boxes {
        dx = o.clip_x(&bb, dx);
    }
    bb = bb.offset(dx, 0.0, 0.0);
    for o in boxes {
        dz = o.clip_z(&bb, dz);
    }
    bb = bb.offset(0.0, 0.0, dz);
    b.bb = bb;
    b.pos = [
        (bb.min_x + bb.max_x) / 2.0,
        bb.min_y,
        (bb.min_z + bb.max_z) / 2.0,
    ];
    b.horizontal_collision = want_x != dx || want_z != dz;
    b.vertical_collision = want_y != dy;
    b.on_ground = b.vertical_collision && want_y < 0.0;
    let (lx, ly, lz) = (
        floor(b.pos[0]),
        floor(b.pos[1] - f64::from(0.2f32)),
        floor(b.pos[2]),
    );
    let mut landed = state_at(w, lx, ly, lz);
    if is_air(landed) {
        let below = state_at(w, lx, ly - 1, lz);
        if is_tall_support(below) {
            landed = below;
        }
    }
    // BoatEntity.fall.
    if b.on_ground {
        // Only a fall of more than 3 blocks ends (it hurts; the server breaks the boat); a
        // shorter one is kept.
        if b.fall_distance > 3.0 {
            b.fall_distance = 0.0;
        }
    } else {
        let (x, y, z) = (floor(b.pos[0]), floor(b.pos[1]) - 1, floor(b.pos[2]));
        if !is_water(state_at(w, x, y, z)) && dy < 0.0 {
            b.fall_distance = (f64::from(b.fall_distance) - dy) as f32;
        }
    }
    if want_x != dx {
        b.vel[0] = 0.0;
    }
    if want_z != dz {
        b.vel[2] = 0.0;
    }
    if want_y != dy {
        // Block.setEntityVelocity: slime bounces (the boat never sneaks), the rest stops it.
        if landed.block().id() == id::SLIME {
            if b.vel[1] < 0.0 {
                b.vel[1] = -b.vel[1];
            }
        } else {
            b.vel[1] = 0.0;
        }
    }
    block_contacts(b, w);
}

/// `checkBlockCollision` of the boat: the blocks its box (shrunk by 0.001) is in. Cobwebs slow the
/// next move, soul sand slows it at once.
fn block_contacts<W: BlockView<State = BlockState>>(b: &mut Boat, w: &W) {
    let bb = b.bb;
    let (x0, y0, z0) = (
        floor(bb.min_x + 0.001),
        floor(bb.min_y + 0.001),
        floor(bb.min_z + 0.001),
    );
    let (x1, y1, z1) = (
        floor(bb.max_x - 0.001),
        floor(bb.max_y - 0.001),
        floor(bb.max_z - 0.001),
    );
    if !(y1 >= 0 && y0 < 256) {
        return;
    }
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                match state_at(w, x, y, z).block().id() {
                    id::WEB => {
                        b.in_web = true;
                        b.fall_distance = 0.0;
                    }
                    id::SOUL_SAND => {
                        b.vel[0] *= 0.4;
                        b.vel[2] *= 0.4;
                    }
                    _ => {}
                }
            }
        }
    }
}
