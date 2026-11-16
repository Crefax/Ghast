//! The boat the local player rides, as the 1.16.5 client ticks it (the boat is then
//! client-authoritative: the client moves it and tells the server where it is).
//!
//! # A tick while riding, in vanilla order
//!
//! 1. Mouse turning (before the tick): the player's rotation, clamped to 105 degrees either side of
//!    the boat's (`BoatEntity.onPassengerLookAround` -> `copyEntityData`).
//! 2. The boat's tick (`BoatEntity.tick`): its location (in water, under water, under flowing
//!    water, on land, in the air), the entity base tick (fluid pushing), floating and paddling with
//!    the keys the player held in the PREVIOUS tick, the `steer_boat` packet, its move (block and
//!    entity collisions, no step-up) and its block effects (twice, as vanilla).
//! 3. The passenger's tick (`ClientWorld.tickPassenger` -> `tickRiding`): its motion is zeroed, the
//!    whole player tick runs as a passenger (no water contact, no swimming, no auto-jump, no item
//!    slowdown, no flight), then the client sends a rotation-only movement packet with its own
//!    ground flag, `steer_vehicle` (the input after the tick) and `vehicle_move` with the boat's
//!    position and rotation. No sprint / sneak packets while riding.
//! 4. The boat places its passenger (`updatePassengerPosition`): 0.45 below the boat's position,
//!    turned by the boat's turn of this tick and clamped again.
//! 5. The player's keys of this tick become the boat's paddle input (`ClientPlayerEntity.tickRiding`).
//!
//! [`tick_boat`](super::tick_boat) runs all five. Server packets: `set_passengers` naming the
//! player puts it on the boat ([`Boat::board`]); one without it takes it off (the client then
//! walks again from where it sits); `vehicle_move` is a correction ([`Boat::server_move`], which
//! also gives the packet the client answers with).

// Vanilla's negated float comparisons are kept as written: they decide the NaN cases.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use ghast_data::FluidKind;
use ghast_data::PhysicsFlags;
use ghast_data::v1_16_5::{Block, BlockState};

use super::blocks::{
    ShapeCtx, collision_boxes, flags, fluid, is_bed, slipperiness, state_at, velocity_multiplier,
};
use super::collide::{BorderShape, Collider, bx, offset};
use super::fluid::{V3, ceil, height, region_loaded, update_in_fluid_as};
use crate::math::{self, Aabb, floor};
use crate::world::BlockView;
use crate::{Packet, Packets};

/// Boat box width (`float`).
pub const BOAT_WIDTH: f32 = 1.375;
/// Boat box height (`float`).
pub const BOAT_HEIGHT: f32 = 0.5625;

/// The boat's location of a tick (`BoatEntity.Location`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BoatLocation {
    /// Floating on water.
    InWater,
    /// Under still water (the top of the box is in water).
    UnderWater,
    /// Under flowing water.
    UnderFlowingWater,
    /// On a block (land friction).
    OnLand,
    /// In the air.
    InAir,
}

/// A boat the player rides and controls. Create it from the spawn packet ([`Boat::new`]).
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// On ground after its last move.
    pub on_ground: bool,
    /// Horizontal collision of its last move.
    pub horizontal_collision: bool,
    /// Vertical collision of its last move.
    pub vertical_collision: bool,
    /// Fall distance.
    pub fall_distance: f32,
    touching_water: bool,
    first_update: bool,
    movement_multiplier: [f64; 3],
    location: Option<BoatLocation>,
    last_location: Option<BoatLocation>,
    ticks_underwater: f32,
    yaw_velocity: f32,
    water_level: f64,
    land_friction: f32,
    fall_velocity: f64,
    /// Paddle input: left, right, forward, back.
    input: [bool; 4],
    paddle: [bool; 2],
}

/// `x - w/2 .. x + w/2` with the boat's float half width widened (vanilla `getBoxAt`).
fn boat_box(x: f64, y: f64, z: f64) -> Aabb {
    let r = f64::from(BOAT_WIDTH / 2.0);
    bx(x - r, y, z - r, x + r, y + f64::from(BOAT_HEIGHT), z + r)
}

impl Boat {
    /// A boat as the spawn packet creates it: at `pos` with the packet's rotation (`yaw`, `pitch`
    /// decoded from the angle bytes: `byte * 360 / 256`).
    #[must_use]
    pub fn new(pos: [f64; 3], yaw: f32, pitch: f32) -> Boat {
        Boat {
            pos,
            bb: boat_box(pos[0], pos[1], pos[2]),
            vel: [0.0; 3],
            yaw,
            pitch,
            on_ground: false,
            horizontal_collision: false,
            vertical_collision: false,
            fall_distance: 0.0,
            touching_water: false,
            first_update: true,
            movement_multiplier: [0.0; 3],
            location: None,
            last_location: None,
            ticks_underwater: 0.0,
            yaw_velocity: 0.0,
            water_level: 0.0,
            land_friction: 0.0,
            fall_velocity: 0.0,
            input: [false; 4],
            paddle: [false; 2],
        }
    }

    /// The location of the last tick (`None` before the first one).
    #[must_use]
    pub fn location(&self) -> Option<BoatLocation> {
        self.location
    }

    /// The turn of the last tick (`yawVelocity`, degrees).
    #[must_use]
    pub fn yaw_velocity(&self) -> f32 {
        self.yaw_velocity
    }

    /// The paddle input of the next tick: left, right, forward, back.
    #[must_use]
    pub fn input(&self) -> [bool; 4] {
        self.input
    }

    /// The paddles moving (the `steer_boat` packet of the last tick): left, right.
    #[must_use]
    pub fn paddles(&self) -> [bool; 2] {
        self.paddle
    }

    /// Moves the boat to a position (vanilla `setPosition`).
    pub fn set_position(&mut self, x: f64, y: f64, z: f64) {
        self.pos = [x, y, z];
        self.bb = boat_box(x, y, z);
    }

    /// The server's `vehicle_move` (`onVehicleMove`: `updatePositionAndAngles`); returns the
    /// `vehicle_move` the client answers with at once.
    pub fn server_move(&mut self, pos: [f64; 3], yaw: f32, pitch: f32) -> Packet {
        self.yaw = yaw % 360.0;
        self.pitch = pitch.clamp(-90.0, 90.0) % 360.0;
        self.set_position(
            pos[0].clamp(-3.0e7, 3.0e7),
            pos[1],
            pos[2].clamp(-3.0e7, 3.0e7),
        );
        self.vehicle_move()
    }

    /// The client's `vehicle_move` packet for this boat.
    #[must_use]
    pub fn vehicle_move(&self) -> Packet {
        Packet::MoveVehicle {
            pos: self.pos,
            yaw: self.yaw,
            pitch: self.pitch,
            on_ground: self.on_ground,
        }
    }

    /// `ClientPlayerEntity.tickRiding`: the passenger's keys become the paddle input.
    pub fn set_input(&mut self, left: bool, right: bool, forward: bool, back: bool) {
        self.input = [left, right, forward, back];
    }

    /// `copyEntityData`: the passenger's yaw clamped to 105 degrees either side of the boat's.
    #[must_use]
    pub fn clamp_passenger_yaw(&self, yaw: f32) -> f32 {
        let f = crate::rotation::wrap_degrees(yaw - self.yaw);
        let g = f.clamp(-105.0, 105.0);
        yaw + (g - f)
    }

    /// The passenger's position after the boat moved (`updatePassengerPosition` of a single
    /// player passenger: `getMountedHeightOffset() + getHeightOffset()` as a float).
    #[must_use]
    pub fn passenger_pos(&self) -> [f64; 3] {
        let g = (-0.1f64 + -0.35f64) as f32;
        [self.pos[0], self.pos[1] + f64::from(g), self.pos[2]]
    }
}

/// One client tick while riding `boat` (the module docs list the steps): the turn clamp, the
/// boat's tick (with `boat_entities`: the boxes it collides with, i.e. other boats, shulkers
/// and living entities other than its passenger), the player's passenger tick (with `around`
/// as in [`tick_ext`](super::tick_ext)), the `vehicle_move`, the seat and the next paddle
/// input. Packets in vanilla order: `steer_boat`, `look`, `steer_vehicle`, `vehicle_move`.
///
/// Returns `false` when the player got off: the 1.16.5 client dismounts by itself when the
/// shift key was held in the previous tick (`PlayerEntity.tickRiding`); the boat has ticked,
/// the player has not (only its age counted). The player then ticks normally in the same tick
/// when it comes after the boat in the client world's entity order (call
/// [`tick_ext`](super::tick_ext) for it, with the rotation it has now: the turn was clamped
/// while it still rode), else from the next tick on.
#[allow(clippy::too_many_arguments)]
pub fn tick_boat<W: BlockView<State = BlockState>>(
    p: &mut super::PlayerState,
    x: &mut super::PlayerExt,
    boat: &mut Boat,
    around: &super::Surroundings<'_>,
    boat_entities: &[Aabb],
    input: &crate::TickInput,
    world: &W,
    out: &mut Packets,
) -> bool {
    // The mouse turned the passenger: the boat clamps it (copyEntityData).
    let mut ti = *input;
    ti.yaw = boat.clamp_passenger_yaw(input.yaw);
    boat_tick(boat, world, boat_entities, x.ultrawarm, out);
    if p.input.sneaking {
        // tickPassenger counts the age, then tickRiding dismounts (shouldDismount).
        p.age += 1;
        p.yaw = ti.yaw;
        p.pitch = ti.pitch;
        return false;
    }
    let ride = super::player::RideCtx {
        bb: boat.bb,
        // BoatEntity.isSubmergedInWater: its location is under water.
        submerged: matches!(
            boat.location,
            Some(BoatLocation::UnderWater | BoatLocation::UnderFlowingWater)
        ),
    };
    super::player::tick_passenger(p, x, around, ride, &ti, world, out);
    out.push(boat.vehicle_move());
    // updatePassengerPosition, then LivingEntity.tickRiding resets the fall distance.
    let [sx, sy, sz] = boat.passenger_pos();
    p.set_position(sx, sy, sz);
    p.yaw += boat.yaw_velocity;
    p.yaw = boat.clamp_passenger_yaw(p.yaw);
    p.fall_distance = 0.0;
    let i = p.input;
    boat.set_input(
        i.pressing_left,
        i.pressing_right,
        i.pressing_forward,
        i.pressing_back,
    );
    true
}

/// One boat tick (`BoatEntity.tick` with the main player as its primary passenger), with the
/// boxes of the entities it collides with (other boats, shulkers, and living entities other
/// than its passenger).
pub(super) fn boat_tick<W: BlockView<State = BlockState>>(
    b: &mut Boat,
    w: &W,
    entities: &[Aabb],
    ultrawarm: bool,
    out: &mut Packets,
) {
    b.last_location = b.location;
    b.location = Some(check_location(b, w));
    if matches!(
        b.location,
        Some(BoatLocation::UnderWater | BoatLocation::UnderFlowingWater)
    ) {
        b.ticks_underwater += 1.0;
    } else {
        b.ticks_underwater = 0.0;
    }
    base_tick(b, w, ultrawarm);
    update_velocity(b, w);
    update_paddles(b);
    out.push(Packet::PaddleBoat {
        left: b.paddle[0],
        right: b.paddle[1],
    });
    let v = V3::new(b.vel[0], b.vel[1], b.vel[2]);
    boat_move(b, w, entities, v);
    // After the move: the block effects once more (BoatEntity.tick -> checkBlockCollision).
    block_contacts(b, w);
}

fn fluid_water<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
) -> Option<super::blocks::FluidAt> {
    fluid(state_at(w, x, y, z)).filter(|f| f.kind == FluidKind::Water)
}

fn check_location<W: BlockView<State = BlockState>>(b: &mut Boat, w: &W) -> BoatLocation {
    if let Some(l) = under_water_location(b, w) {
        b.water_level = b.bb.max_y;
        return l;
    }
    if boat_in_water(b, w) {
        return BoatLocation::InWater;
    }
    let f = nearby_slipperiness(b, w);
    if f > 0.0 {
        b.land_friction = f;
        BoatLocation::OnLand
    } else {
        BoatLocation::InAir
    }
}

fn under_water_location<W: BlockView<State = BlockState>>(b: &Boat, w: &W) -> Option<BoatLocation> {
    let bb = b.bb;
    let d = bb.max_y + 0.001;
    let (x0, x1) = (floor(bb.min_x), ceil(bb.max_x));
    let (y0, y1) = (floor(bb.max_y), ceil(d));
    let (z0, z1) = (floor(bb.min_z), ceil(bb.max_z));
    let mut under = false;
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                if let Some(f) = fluid_water(w, x, y, z)
                    && d < f64::from(y as f32 + height(w, x, y, z, f))
                {
                    if !(f.amount == 8 && !f.falling) {
                        return Some(BoatLocation::UnderFlowingWater);
                    }
                    under = true;
                }
            }
        }
    }
    under.then_some(BoatLocation::UnderWater)
}

fn boat_in_water<W: BlockView<State = BlockState>>(b: &mut Boat, w: &W) -> bool {
    let bb = b.bb;
    let (x0, x1) = (floor(bb.min_x), ceil(bb.max_x));
    let (y0, y1) = (floor(bb.min_y), ceil(bb.min_y + 0.001));
    let (z0, z1) = (floor(bb.min_z), ceil(bb.max_z));
    let mut found = false;
    // Java's Double.MIN_VALUE: the smallest positive subnormal.
    b.water_level = f64::from_bits(1);
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                if let Some(f) = fluid_water(w, x, y, z) {
                    let top = y as f32 + height(w, x, y, z, f);
                    b.water_level = math::java_max(f64::from(top), b.water_level);
                    found |= bb.min_y < f64::from(top);
                }
            }
        }
    }
    found
}

/// `method_7548`: the average slipperiness of the blocks under the boat (lily pads excluded).
fn nearby_slipperiness<W: BlockView<State = BlockState>>(b: &Boat, w: &W) -> f32 {
    let bb = b.bb;
    let q = Aabb::new(
        bb.min_x,
        bb.min_y - 0.001,
        bb.min_z,
        bb.max_x,
        bb.min_y,
        bb.max_z,
    );
    let i = floor(q.min_x) - 1;
    let j = ceil(q.max_x) + 1;
    let k = floor(q.min_y) - 1;
    let l = ceil(q.max_y) + 1;
    let m = floor(q.min_z) - 1;
    let n = ceil(q.max_z) + 1;
    let mut f = 0.0f32;
    let mut o = 0i32;
    // A context-free shape query (`getCollisionShape(world, pos)`: the absent context).
    for p in i..j {
        for qz in m..n {
            let r = i32::from(p == i || p == j - 1) + i32::from(qz == m || qz == n - 1);
            if r == 2 {
                continue;
            }
            for s in k..l {
                if r > 0 && (s == k || s == l - 1) {
                    continue;
                }
                let st = state_at(w, p, s, qz);
                if st.block() == Block::LILY_PAD {
                    continue;
                }
                let hit = shape_boxes(st, p, s, qz, None)
                    .iter()
                    .any(|bb| super::blocks::boxes_match(bb, &q));
                if hit {
                    f += slipperiness(st);
                    o += 1;
                }
            }
        }
    }
    f / o as f32
}

/// A block's collision boxes placed in the world (bamboo's with its model offset).
fn shape_boxes(st: BlockState, x: i32, y: i32, z: i32, ctx: Option<&ShapeCtx>) -> Vec<Aabb> {
    if st.block() == Block::BAMBOO {
        return vec![super::blocks::bamboo_box(x, z).at(x, y, z)];
    }
    collision_boxes(st, y, ctx)
        .iter()
        .map(|b| b.at(x, y, z))
        .collect()
}

/// `method_7544`: the water surface over the boat's box (used when it lands in water).
fn water_height_below<W: BlockView<State = BlockState>>(b: &Boat, w: &W) -> f32 {
    let bb = b.bb;
    let (i, j) = (floor(bb.min_x), ceil(bb.max_x));
    let k = floor(bb.max_y);
    let l = ceil(bb.max_y - b.fall_velocity);
    let (m, n) = (floor(bb.min_z), ceil(bb.max_z));
    'rows: for o in k..l {
        let mut f = 0.0f32;
        for p in i..j {
            for q in m..n {
                if let Some(fl) = fluid_water(w, p, o, q) {
                    f = f.max(height(w, p, o, q, fl));
                }
                if f >= 1.0 {
                    continue 'rows;
                }
            }
        }
        if f < 1.0 {
            return o as f32 + f;
        }
    }
    (l + 1) as f32
}

/// `Entity.baseTick` of the boat: water and lava contact with their flow push, the eye fluid,
/// lava halving the fall distance.
fn base_tick<W: BlockView<State = BlockState>>(b: &mut Boat, w: &W, ultrawarm: bool) {
    let mut v = V3::new(b.vel[0], b.vel[1], b.vel[2]);
    let water = update_in_fluid_as::<W, false>(w, &b.bb, FluidKind::Water, 0.014, true, &mut v);
    if water.touching {
        b.fall_distance = 0.0;
    }
    b.touching_water = water.touching;
    let lava_speed = if ultrawarm {
        0.007
    } else {
        0.002_333_333_333_333_333_5
    };
    let lava = update_in_fluid_as::<W, false>(w, &b.bb, FluidKind::Lava, lava_speed, true, &mut v);
    b.vel = [v.x, v.y, v.z];
    if !b.first_update && lava.height > 0.0 {
        b.fall_distance *= 0.5;
    }
    b.first_update = false;
}

fn update_velocity<W: BlockView<State = BlockState>>(b: &mut Boat, w: &W) {
    let mut e = f64::from(-0.04f32);
    let mut f = 0.0;
    let mut decay = 0.05f32;
    let loc = b.location;
    if b.last_location == Some(BoatLocation::InAir)
        && loc != Some(BoatLocation::InAir)
        && loc != Some(BoatLocation::OnLand)
    {
        // getBodyY(1.0): the top of the box.
        b.water_level = b.pos[1] + f64::from(BOAT_HEIGHT);
        let y = f64::from(water_height_below(b, w)) - f64::from(BOAT_HEIGHT) + 0.101;
        b.set_position(b.pos[0], y, b.pos[2]);
        b.vel = [b.vel[0] * 1.0, b.vel[1] * 0.0, b.vel[2] * 1.0];
        b.fall_velocity = 0.0;
        b.location = Some(BoatLocation::InWater);
        return;
    }
    match loc {
        Some(BoatLocation::InWater) => {
            f = (b.water_level - b.pos[1]) / f64::from(BOAT_HEIGHT);
            decay = 0.9;
        }
        Some(BoatLocation::UnderFlowingWater) => {
            e = -7.0e-4;
            decay = 0.9;
        }
        Some(BoatLocation::UnderWater) => {
            f = f64::from(0.01f32);
            decay = 0.45;
        }
        Some(BoatLocation::InAir) => decay = 0.9,
        Some(BoatLocation::OnLand) => {
            decay = b.land_friction;
            // The primary passenger is a player.
            b.land_friction /= 2.0;
        }
        None => {}
    }
    let d = f64::from(decay);
    b.vel = [b.vel[0] * d, b.vel[1] + e, b.vel[2] * d];
    b.yaw_velocity *= decay;
    if f > 0.0 {
        b.vel[1] = (b.vel[1] + f * 0.061_538_460_162_969_73) * 0.75;
    }
}

fn update_paddles(b: &mut Boat) {
    let [left, right, forward, back] = b.input;
    let mut f = 0.0f32;
    if left {
        b.yaw_velocity -= 1.0;
    }
    if right {
        b.yaw_velocity += 1.0;
    }
    if right != left && !forward && !back {
        f += 0.005;
    }
    b.yaw += b.yaw_velocity;
    if forward {
        f += 0.04;
    }
    if back {
        f -= 0.005;
    }
    let r = (std::f64::consts::PI / 180.0) as f32;
    // Vec3d.add: the y component gets + 0.0 too (a -0.0 becomes 0.0).
    b.vel[0] += f64::from(math::sin(-b.yaw * r) * f);
    b.vel[1] += 0.0;
    b.vel[2] += f64::from(math::cos(b.yaw * r) * f);
    b.paddle = [right && !left || forward, left && !right || forward];
}

/// `Entity.move(SELF, movement)` of the boat: no step-up, no sneak guard; the boat's own fall.
fn boat_move<W: BlockView<State = BlockState>>(
    b: &mut Boat,
    w: &W,
    entities: &[Aabb],
    movement: V3,
) {
    let mut m = movement;
    let mm = b.movement_multiplier;
    if mm[0] * mm[0] + mm[1] * mm[1] + mm[2] * mm[2] > 1.0e-7 {
        m = V3::new(m.x * mm[0], m.y * mm[1], m.z * mm[2]);
        b.movement_multiplier = [0.0; 3];
        b.vel = [0.0; 3];
    }
    let border = BorderShape::new(&w.world_border());
    let col = Collider {
        w,
        border: &border,
        ctx: ShapeCtx {
            min_y: b.pos[1],
            descending: false,
        },
        entities,
    };
    let v = col.adjust(m, &b.bb, b.on_ground, 0.0);
    if v.length_sq() > 1.0e-7 {
        b.bb = offset(&b.bb, v.x, v.y, v.z);
        b.pos = [
            (b.bb.min_x + b.bb.max_x) / 2.0,
            b.bb.min_y,
            (b.bb.min_z + b.bb.max_z) / 2.0,
        ];
    }
    let approx = |a: f64, c: f64| (c - a).abs() < f64::from(1.0e-5f32);
    b.horizontal_collision = !approx(m.x, v.x) || !approx(m.z, v.z);
    b.vertical_collision = m.y != v.y;
    b.on_ground = b.vertical_collision && m.y < 0.0;
    let (lx, ly, lz) = (
        floor(b.pos[0]),
        floor(b.pos[1] - f64::from(0.2f32)),
        floor(b.pos[2]),
    );
    let mut landed = state_at(w, lx, ly, lz);
    if landed.is_air() {
        let below = state_at(w, lx, ly - 1, lz);
        let f = flags(below);
        if f.contains(PhysicsFlags::FENCE)
            || f.contains(PhysicsFlags::WALL)
            || f.contains(PhysicsFlags::FENCE_GATE)
        {
            landed = below;
        }
    }
    boat_fall(b, w, v.y);
    let old = b.vel;
    if m.x != v.x {
        b.vel = [0.0, old[1], old[2]];
    }
    if m.z != v.z {
        b.vel = [old[0], old[1], 0.0];
    }
    let block = landed.block();
    if m.y != v.y {
        // Block.onEntityLand: slime and beds bounce a non-living entity with 0.8 of the factor.
        if block == Block::SLIME_BLOCK {
            if b.vel[1] < 0.0 {
                b.vel[1] = -b.vel[1] * 0.8;
            }
        } else if is_bed(landed) {
            if b.vel[1] < 0.0 {
                b.vel[1] = -b.vel[1] * f64::from(0.66f32) * 0.8;
            }
        } else {
            b.vel[1] *= 0.0;
        }
    }
    if b.on_ground && block == Block::SLIME_BLOCK {
        let d = b.vel[1].abs();
        if d < 0.1 {
            let e = 0.4 + d * 0.2;
            b.vel[0] *= e;
            b.vel[2] *= e;
        }
    }
    block_contacts(b, w);
    let k = f64::from(boat_velocity_multiplier(b, w));
    b.vel[0] *= k;
    b.vel[2] *= k;
}

/// `BoatEntity.fall`.
fn boat_fall<W: BlockView<State = BlockState>>(b: &mut Boat, w: &W, dy: f64) {
    b.fall_velocity = b.vel[1];
    if b.on_ground {
        if b.fall_distance > 3.0 && b.location != Some(BoatLocation::OnLand) {
            b.fall_distance = 0.0;
            return;
        }
        b.fall_distance = 0.0;
    } else {
        let (x, y, z) = (floor(b.pos[0]), floor(b.pos[1]) - 1, floor(b.pos[2]));
        if fluid_water(w, x, y, z).is_none() && dy < 0.0 {
            b.fall_distance = (f64::from(b.fall_distance) - dy) as f32;
        }
    }
}

fn boat_velocity_multiplier<W: BlockView<State = BlockState>>(b: &Boat, w: &W) -> f32 {
    let here = state_at(w, floor(b.pos[0]), floor(b.pos[1]), floor(b.pos[2]));
    let f = velocity_multiplier(here);
    if here.block() == Block::WATER || here.block() == Block::BUBBLE_COLUMN || f != 1.0 {
        f
    } else {
        let (x, y, z) = (
            floor(b.pos[0]),
            floor(b.bb.min_y - 0.500_000_1),
            floor(b.pos[2]),
        );
        velocity_multiplier(state_at(w, x, y, z))
    }
}

/// `checkBlockCollision` of the boat: the blocks its box (shrunk by 0.001) is in.
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
    if !region_loaded(w, x0, y0, z0, x1, y1, z1) {
        return;
    }
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                let s = state_at(w, x, y, z);
                let block = s.block();
                if block == Block::COBWEB {
                    b.fall_distance = 0.0;
                    b.movement_multiplier = [0.25, f64::from(0.05f32), 0.25];
                } else if block == Block::HONEY_BLOCK {
                    let sliding = !b.on_ground
                        && !(b.pos[1] > f64::from(y) + 0.9375 - 1.0e-7)
                        && !(b.vel[1] >= -0.08)
                        && {
                            let d = (f64::from(x) + 0.5 - b.pos[0]).abs();
                            let e = (f64::from(z) + 0.5 - b.pos[2]).abs();
                            let f = 0.4375 + f64::from(BOAT_WIDTH / 2.0);
                            d + 1.0e-7 > f || e + 1.0e-7 > f
                        };
                    if sliding {
                        if b.vel[1] < -0.13 {
                            let d = -0.05 / b.vel[1];
                            b.vel = [b.vel[0] * d, -0.05, b.vel[2] * d];
                        } else {
                            b.vel[1] = -0.05;
                        }
                        b.fall_distance = 0.0;
                    }
                }
            }
        }
    }
}
