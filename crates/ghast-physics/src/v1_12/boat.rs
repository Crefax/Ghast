//! The boat the local player rides, as the 1.12.2 client ticks it (the boat is then
//! client-authoritative: the client moves it and tells the server where it is).
//!
//! # A tick while riding, in vanilla order
//!
//! 1. Mouse turning (before the tick): the player's rotation, clamped to 105 degrees either side of
//!    the boat's (`Entity.increaseTransforms` -> `BoatEntity.onPassengerLookAround` ->
//!    `copyEntityData`).
//! 2. The boat's tick (`BoatEntity.tick`): its location (in water, under water, under flowing
//!    water, on land, in the air), the entity base tick (the water flow push), floating and
//!    paddling with the keys the player held in the PREVIOUS tick, the `steer_boat` packet, its move
//!    (block and entity collisions, no step-up, slime / bed bounces, no stepping effects) and its
//!    block contacts (twice, as vanilla: cobweb, soul sand; a lily pad it touches is broken in the
//!    client's world). A boat with a passenger is never pushed by other entities.
//! 3. The passenger's tick (`World.checkChunk` -> `tickRiding`): its motion is zeroed, the whole
//!    player tick runs as a passenger (no water contact, no item slowdown, no flight travel), then
//!    the client sends a rotation-only movement packet with its own ground flag, `steer_vehicle`
//!    (the input after the tick) and `vehicle_move` with the boat's position and rotation. No
//!    sprint / sneak packets while riding.
//! 4. The boat places its passenger (`updatePassengerPosition`): 0.45 below the boat's position,
//!    turned by the boat's turn of this tick and clamped again; the fall distance is reset.
//! 5. The player's keys of this tick become the boat's paddle input (`ClientPlayerEntity.tickRiding`).
//!
//! Unlike 1.16 the 1.12 client never dismounts by itself: the sneak key only reaches the server in
//! `steer_vehicle`, whose `set_passengers` then takes the player off.
//!
//! [`tick_boat`](super::tick_boat) runs all five. Server packets: `set_passengers` naming the
//! player puts it on the boat; one without it takes it off (the client then walks again from
//! where it sits); `vehicle_move` is a correction ([`Boat::server_move`], which also gives the
//! packet the client answers with).

// Vanilla's negated float comparisons are kept as written: they decide the NaN cases.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use ghast_data::v1_12::BlockState;

use super::blocks::{
    BoxList, BoxSink, add_collision_boxes, id, is_air, is_tall_support, is_water, slipperiness,
    state_at,
};
use super::ext::PlayerExt;
use super::fluid::{contains_lava, water_contact};
use super::player::{PlayerState, box_at};
use crate::input::{Packet, Packets, TickInput};
use crate::math::{self, Aabb, BoxOps, floor};
use crate::world::{BlockView, WorldBorder};

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
    /// On ground after its last move.
    pub on_ground: bool,
    /// Horizontal collision of its last move.
    pub horizontal_collision: bool,
    /// Vertical collision of its last move.
    pub vertical_collision: bool,
    /// Fall distance.
    pub fall_distance: f32,
    /// Below y = -64 the boat is destroyed (its passenger gets off).
    pub removed: bool,
    in_web: bool,
    outside_world_border: bool,
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
    /// Lily pads broken in the last tick (the caller sets them to air in its world).
    broken: Vec<[i32; 3]>,
}

fn boat_box(x: f64, y: f64, z: f64) -> Aabb {
    box_at(x, y, z, BOAT_WIDTH, BOAT_HEIGHT)
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
            removed: false,
            in_web: false,
            outside_world_border: false,
            location: None,
            last_location: None,
            ticks_underwater: 0.0,
            yaw_velocity: 0.0,
            water_level: 0.0,
            land_friction: 0.0,
            fall_velocity: 0.0,
            input: [false; 4],
            paddle: [false; 2],
            broken: Vec::new(),
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

    /// The lily pads the boat broke in the last tick (`LilyPadBlock.onEntityCollision` removes
    /// them from the client's world at once): set them to air in the world.
    #[must_use]
    pub fn broken_lily_pads(&self) -> &[[i32; 3]] {
        &self.broken
    }

    /// Moves the boat to a position (vanilla `updatePosition`).
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

/// The world as the rest of a tick sees it after the boat broke lily pads: air there.
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

/// One client tick while riding `boat` (the module docs list the steps): the turn clamp, the
/// boat's tick (with `boat_entities`: the boxes it collides with, i.e. other boats, shulkers and
/// the living entities other than its passenger, as `World.doesBoxCollide` adds them for the
/// boat), the player's passenger tick (with `entities`: the boxes the player collides with, as
/// for [`tick_ext`](super::tick_ext), its own boat excluded), the `vehicle_move`, the seat and the
/// next paddle input. Packets in vanilla order: `steer_boat`, `look`, `steer_vehicle`,
/// `vehicle_move`. Lily pads the boat broke are in [`Boat::broken_lily_pads`].
///
/// Returns `false` when the boat was destroyed below y = -64: the player got off without its
/// own tick (only its age counted) and walks from the next tick on.
#[allow(clippy::too_many_arguments)]
pub fn tick_boat<W: BlockView<State = BlockState>>(
    p: &mut PlayerState,
    x: &mut PlayerExt,
    boat: &mut Boat,
    entities: &[Aabb],
    boat_entities: &[Aabb],
    input: &TickInput,
    world: &W,
    out: &mut Packets,
) -> bool {
    // The mouse turned the passenger: the boat clamps it (copyEntityData).
    let mut ti = *input;
    ti.yaw = boat.clamp_passenger_yaw(input.yaw);
    boat_tick(boat, world, boat_entities, out);
    if boat.removed {
        // Entity.tickRiding: the vehicle is gone, the player gets off without ticking.
        p.ticks_alive += 1;
        p.yaw = ti.yaw;
        p.pitch = ti.pitch;
        return false;
    }
    let gone = std::mem::take(&mut boat.broken);
    if gone.is_empty() {
        super::player::tick_passenger(p, x, entities, &ti, world, out);
    } else {
        let w = Gone {
            w: world,
            gone: &gone,
        };
        super::player::tick_passenger(p, x, entities, &ti, &w, out);
    }
    boat.broken = gone;
    out.push(boat.vehicle_move());
    // updatePassengerPosition, then LivingEntity.tickRiding resets the fall distance.
    let [sx, sy, sz] = boat.passenger_pos();
    p.set_position(sx, sy, sz);
    p.yaw += boat.yaw_velocity;
    p.yaw = boat.clamp_passenger_yaw(p.yaw);
    p.fall_distance = 0.0;
    let k = &input.keys;
    boat.set_input(k.left, k.right, k.forward, k.back);
    true
}

/// One boat tick (`BoatEntity.tick` with the main player as its primary passenger).
fn boat_tick<W: BlockView<State = BlockState>>(
    b: &mut Boat,
    w: &W,
    entities: &[Aabb],
    out: &mut Packets,
) {
    b.broken.clear();
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
    base_tick(b, w);
    update_velocity(b, w);
    update_paddles(b);
    out.push(Packet::PaddleBoat {
        left: b.paddle[0],
        right: b.paddle[1],
    });
    let [vx, vy, vz] = b.vel;
    boat_move(b, w, entities, vx, vy, vz);
    // After the move: the block contacts once more (BoatEntity.tick -> checkBlockCollision).
    block_contacts(b, w);
}

/// `AbstractFluidBlock.method_13709`: the water's height in its block (a full block under
/// water above a source or falling block).
fn water_height<W: BlockView<State = BlockState>>(
    w: &W,
    x: i32,
    y: i32,
    z: i32,
    s: BlockState,
) -> f32 {
    let level = s.meta();
    if level & 7 == 0 && is_water(state_at(w, x, y + 1, z)) {
        1.0
    } else {
        let l = if level >= 8 { 0 } else { level };
        1.0 - (f32::from(l) + 1.0) / 9.0
    }
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
                let s = state_at(w, x, y, z);
                if is_water(s) && d < f64::from(y as f32 + water_height(w, x, y, z, s)) {
                    if s.meta() != 0 {
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
                let s = state_at(w, x, y, z);
                if is_water(s) {
                    let top = y as f32 + water_height(w, x, y, z, s);
                    b.water_level = math::java_max(f64::from(top), b.water_level);
                    found |= bb.min_y < f64::from(top);
                }
            }
        }
    }
    found
}

/// `method_11333`: the average slipperiness of the blocks whose collision boxes touch the thin
/// box under the boat (lily pads have none for a boat).
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
    let mut list = BoxList::new();
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
                list.clear();
                let mut sink = BoxSink {
                    query: q,
                    out: &mut list,
                };
                // LilyPadBlock.appendCollisionBoxes: no box for a boat.
                if st.block().id() != id::LILY_PAD {
                    add_collision_boxes(w, p, s, qz, st, &mut sink);
                }
                if !list.is_empty() {
                    f += slipperiness(st);
                    o += 1;
                }
            }
        }
    }
    f / o as f32
}

/// `method_11332`: the water surface over the boat's box (used when it lands in water).
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
                let s = state_at(w, p, o, q);
                if is_water(s) {
                    f = f.max(water_height(w, p, o, q, s));
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

/// `Entity.baseTick` of the boat: water contact with its (normalised) flow push, lava halving
/// the fall distance, destroyed below y = -64.
fn base_tick<W: BlockView<State = BlockState>>(b: &mut Boat, w: &W) {
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
        b.water_level = b.bb.min_y + f64::from(BOAT_HEIGHT);
        let y = f64::from(water_height_below(b, w)) - f64::from(BOAT_HEIGHT) + 0.101;
        b.set_position(b.pos[0], y, b.pos[2]);
        b.vel[1] = 0.0;
        b.fall_velocity = 0.0;
        b.location = Some(BoatLocation::InWater);
        return;
    }
    match loc {
        Some(BoatLocation::InWater) => {
            f = (b.water_level - b.bb.min_y) / f64::from(BOAT_HEIGHT);
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
    b.vel[0] *= d;
    b.vel[2] *= d;
    b.yaw_velocity *= decay;
    b.vel[1] += e;
    if f > 0.0 {
        b.vel[1] += f * 0.061_538_460_162_969_73;
        b.vel[1] *= 0.75;
    }
}

fn update_paddles(b: &mut Boat) {
    let [left, right, forward, back] = b.input;
    let mut f = 0.0f32;
    if left {
        b.yaw_velocity += -1.0;
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
    b.vel[0] += f64::from(math::sin(-b.yaw * r) * f);
    b.vel[2] += f64::from(math::cos(b.yaw * r) * f);
    b.paddle = [right && !left || forward, left && !right || forward];
}

/// Java `MathHelper.ceil(double)`.
#[inline]
fn ceil(v: f64) -> i32 {
    let i = v as i32;
    if v > f64::from(i) { i + 1 } else { i }
}

/// `World.doesBoxCollide` for the boat: the block boxes intersecting `query` (1.9+ scan: one
/// block wider on x / z without the four corner columns and the top layer of the edge columns;
/// outside the world border blocks collide like stone while the boat is inside the world; lily
/// pads have no box for a boat), then the entity boxes intersecting it.
fn gather<W: BlockView<State = BlockState>>(
    b: &mut Boat,
    w: &W,
    border: &WorldBorder,
    entities: &[Aabb],
    query: Aabb,
    out: &mut BoxList,
) {
    out.clear();
    let (x0, x1) = (floor(query.min_x) - 1, ceil(query.max_x) + 1);
    let (y0, y1) = (floor(query.min_y) - 1, ceil(query.max_y) + 1);
    let (z0, z1) = (floor(query.min_z) - 1, ceil(query.max_z) + 1);
    let was_outside = b.outside_world_border;
    let inside = {
        let m = if b.outside_world_border { 1.0 } else { -1.0 };
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
    let mut sink = BoxSink { query, out };
    for x in x0..x1 {
        let edge_x = x == x0 || x == x1 - 1;
        for z in z0..z1 {
            let edge_z = z == z0 || z == z1 - 1;
            if edge_x && edge_z {
                continue;
            }
            let substitute = inside && !border.contains_block(x, z);
            for y in y0..y1 {
                if (edge_x || edge_z) && y == y1 - 1 {
                    continue;
                }
                visited = true;
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
    for e in entities {
        if e.intersects(&query) {
            sink.out.push(*e);
        }
    }
    if visited {
        if was_outside && inside {
            b.outside_world_border = false;
        } else if !was_outside && !inside {
            b.outside_world_border = true;
        }
    }
}

const STONE: BlockState = ghast_data::v1_12::Block::STONE.default_state();

/// `Entity.move(SELF, …)` of the boat: no step-up (step height 0), no sneak guard, no stepping
/// effects (`canClimb` is false); the boat's own fall.
fn boat_move<W: BlockView<State = BlockState>>(
    b: &mut Boat,
    w: &W,
    entities: &[Aabb],
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
    gather(
        b,
        w,
        &border,
        entities,
        b.bb.stretch(dx, dy, dz),
        &mut boxes,
    );
    let boxes = boxes.as_slice();
    let mut bb = b.bb;
    if dy != 0.0 {
        for o in boxes {
            dy = o.clip_y(&bb, dy);
        }
        bb = bb.offset(0.0, dy, 0.0);
    }
    if dx != 0.0 {
        for o in boxes {
            dx = o.clip_x(&bb, dx);
        }
        if dx != 0.0 {
            bb = bb.offset(dx, 0.0, 0.0);
        }
    }
    if dz != 0.0 {
        for o in boxes {
            dz = o.clip_z(&bb, dz);
        }
        if dz != 0.0 {
            bb = bb.offset(0.0, 0.0, dz);
        }
    }
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
    boat_fall(b, w, dy);
    if want_x != dx {
        b.vel[0] = 0.0;
    }
    if want_z != dz {
        b.vel[2] = 0.0;
    }
    if want_y != dy {
        // Block.setEntityVelocity: slime and beds bounce a non-living entity with 0.8 of the
        // factor (the boat never sneaks); everything else stops the fall.
        match landed.block().id() {
            id::SLIME => {
                if b.vel[1] < 0.0 {
                    b.vel[1] = -b.vel[1] * 0.8;
                }
            }
            id::BED => {
                if b.vel[1] < 0.0 {
                    b.vel[1] = -b.vel[1] * f64::from(0.66f32) * 0.8;
                }
            }
            _ => b.vel[1] = 0.0,
        }
    }
    block_contacts(b, w);
}

/// `BoatEntity.fall`.
fn boat_fall<W: BlockView<State = BlockState>>(b: &mut Boat, w: &W, dy: f64) {
    b.fall_velocity = b.vel[1];
    if b.on_ground {
        // A fall of more than 3 blocks on land only hurts (the server removes the boat).
        b.fall_distance = 0.0;
    } else {
        let (x, y, z) = (floor(b.pos[0]), floor(b.pos[1]) - 1, floor(b.pos[2]));
        if !is_water(state_at(w, x, y, z)) && dy < 0.0 {
            b.fall_distance = (f64::from(b.fall_distance) - dy) as f32;
        }
    }
}

/// `checkBlockCollision` of the boat: the blocks its box (shrunk by 0.001) is in. Cobwebs slow
/// the next move, soul sand slows it at once, lily pads break.
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
                    id::LILY_PAD if !b.broken.contains(&[x, y, z]) => b.broken.push([x, y, z]),
                    _ => {}
                }
            }
        }
    }
}
