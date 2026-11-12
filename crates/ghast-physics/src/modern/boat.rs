//! Boats the local player controls, as the 26.1 client ticks them (the boat is then
//! client-authoritative: the client moves it and tells the server where it is).
//!
//! # A tick while riding, in vanilla order
//!
//! 1. Mouse turning (before the tick): the player's rotation, clamped to 105 degrees either side of
//!    the boat's (`AbstractBoat.onPassengerTurned`).
//! 2. The boat's tick (`ClientLevel.tickNonPassenger`): its status (in water, under water, under
//!    flowing water, on land, in the air), the vanilla entity base tick (fluid pushing), floating
//!    and paddling with the keys the player held in the PREVIOUS tick, the `steer_boat` packet, its
//!    move (block and entity collisions, no step-up) and its block effects (twice, as vanilla).
//! 3. The passenger's tick (`tickPassenger` -> `rideTick`): its motion is zeroed, then the whole
//!    player tick runs as a passenger (no sprinting, no crouching, no flying toggle, its fluid box
//!    cut at the boat's top) and sends `player_input` (when the keys changed), a rotation-only
//!    movement packet with its own ground / collision flags, `vehicle_move` with the boat's
//!    position, rotation and ground flag, then a sprint packet if its sprint state changed.
//! 4. The boat places its passenger (`positionRider`): on the boat's attachment point, turned by
//!    the boat's turn of this tick and clamped again; the fall distance is reset.
//! 5. The player's keys of this tick become the boat's paddle input (`LocalPlayer.rideTick`).
//!
//! [`tick_boat`] runs all five. Server packets: `set_passengers` naming the player puts it on
//! the boat ([`Boat::board`]), one without it takes it off (nothing moves on the client; the
//! server then teleports the player); `move_vehicle` is a correction ([`Boat::server_move`],
//! which also gives the packet the client answers with). Sneaking does not dismount on the client:
//! the shift key goes to the server in `player_input`.

use super::collide::{Border, Ents, Vehicle, World};
use super::ext::PlayerExt;
use super::fluid::{amount, kind};
use super::math::{self as m, DEG_TO_RAD, V3, ceil, floor};
use super::player::{Ctx, Glide, PlayerState, Pose, Ride, tick_core};
use super::rules::Rules;
use crate::math::Aabb;
use crate::world::{BlockView, TickCache};
use crate::{Packet, Packets, TickInput};

/// The kind of a boat (its ride height and passenger seat).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BoatKind {
    /// A boat (`*_boat`).
    Boat,
    /// A chest boat (`*_chest_boat`): one passenger, seated 0.15 forward.
    ChestBoat,
    /// A raft (`bamboo_raft`): a higher seat.
    Raft,
    /// A chest raft (`bamboo_chest_raft`).
    ChestRaft,
}

impl BoatKind {
    /// The kind of a 26.1 entity type name (`oak_boat`, `birch_chest_boat`, `bamboo_raft`, ...);
    /// `None` for anything that is not a boat.
    #[must_use]
    pub fn from_entity_name(name: &str) -> Option<BoatKind> {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        if name == "bamboo_raft" {
            Some(BoatKind::Raft)
        } else if name == "bamboo_chest_raft" {
            Some(BoatKind::ChestRaft)
        } else if name.ends_with("_chest_boat") {
            Some(BoatKind::ChestBoat)
        } else if name.ends_with("_boat") {
            Some(BoatKind::Boat)
        } else {
            None
        }
    }

    /// `rideHeight(dimensions)`: a third of the height, or 8/9 of it for rafts.
    fn ride_height(self) -> f64 {
        match self {
            BoatKind::Boat | BoatKind::ChestBoat => f64::from(HEIGHT / 3.0),
            BoatKind::Raft | BoatKind::ChestRaft => f64::from(HEIGHT * 0.888_888_9),
        }
    }

    /// `getSinglePassengerXOffset`.
    fn seat_offset(self) -> f32 {
        match self {
            BoatKind::ChestBoat | BoatKind::ChestRaft => 0.15,
            BoatKind::Boat | BoatKind::Raft => 0.0,
        }
    }
}

/// The boat's status of a tick (`AbstractBoat.Status`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BoatStatus {
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

const WIDTH: f32 = 1.375;
const HEIGHT: f32 = 0.5625;

/// A boat the player rides and controls. Create it from the add-entity packet ([`Boat::new`]).
#[derive(Clone, Debug)]
pub struct Boat {
    kind: BoatKind,
    /// The generic entity state (position, box, motion, ground and collision flags, fall
    /// distance, fluids).
    e: PlayerState,
    s: BoatState,
}

/// The boat's own fields (`AbstractBoat`).
#[derive(Clone, Copy, Debug, Default)]
struct BoatState {
    status: Option<BoatStatus>,
    old_status: Option<BoatStatus>,
    out_of_control: f32,
    delta_rotation: f32,
    water_level: f64,
    land_friction: f32,
    last_yd: f64,
    /// Paddle input: left, right, up (forward), down (back).
    input: [bool; 4],
    paddle: [bool; 2],
    /// `paddlePositions` (the paddle sound check re-reads the status, which resets the water level
    /// and the land friction).
    paddle_pos: [f32; 2],
}

impl Boat {
    /// A boat as the add-entity packet creates it: at `pos` with the packet's rotation (`yaw`,
    /// `pitch` as decoded from the packet's angle bytes).
    #[must_use]
    pub fn new(kind: BoatKind, pos: [f64; 3], yaw: f32, pitch: f32) -> Boat {
        let mut e = PlayerState::new(pos, yaw % 360.0, (pitch % 360.0).clamp(-90.0, 90.0));
        e.dims = super::player::Dimensions {
            width: WIDTH,
            height: HEIGHT,
            eye_height: HEIGHT,
        };
        e.set_pos(pos[0], pos[1], pos[2]);
        e.pose = Pose::Standing;
        e.on_ground = false;
        Boat {
            kind,
            e,
            s: BoatState::default(),
        }
    }

    /// The kind.
    #[must_use]
    pub fn kind(&self) -> BoatKind {
        self.kind
    }

    /// Position (feet).
    #[must_use]
    pub fn pos(&self) -> [f64; 3] {
        self.e.pos
    }

    /// Yaw.
    #[must_use]
    pub fn yaw(&self) -> f32 {
        self.e.yaw
    }

    /// Pitch.
    #[must_use]
    pub fn pitch(&self) -> f32 {
        self.e.pitch
    }

    /// Motion.
    #[must_use]
    pub fn vel(&self) -> [f64; 3] {
        self.e.vel
    }

    /// The box.
    #[must_use]
    pub fn bounding_box(&self) -> Aabb {
        self.e.bb
    }

    /// On the ground.
    #[must_use]
    pub fn on_ground(&self) -> bool {
        self.e.on_ground
    }

    /// Horizontal collision of the last move.
    #[must_use]
    pub fn horizontal_collision(&self) -> bool {
        self.e.horizontal_collision
    }

    /// Fall distance.
    #[must_use]
    pub fn fall_distance(&self) -> f64 {
        self.e.fall_distance
    }

    /// The status of the last tick (`None` before the first).
    #[must_use]
    pub fn status(&self) -> Option<BoatStatus> {
        self.s.status
    }

    /// Vertical collision of the last move.
    #[must_use]
    pub fn vertical_collision(&self) -> bool {
        self.e.vertical_collision
    }

    /// The status of the tick before the last.
    #[must_use]
    pub fn old_status(&self) -> Option<BoatStatus> {
        self.s.old_status
    }

    /// Ticks spent under water in a row (`outOfControlTicks`; the server ejects at 60).
    #[must_use]
    pub fn out_of_control_ticks(&self) -> f32 {
        self.s.out_of_control
    }

    /// The water level of the last status check (`waterLevel`).
    #[must_use]
    pub fn water_level(&self) -> f64 {
        self.s.water_level
    }

    /// The ground friction found on land (`landFriction`, halved each tick while paddled).
    #[must_use]
    pub fn land_friction(&self) -> f32 {
        self.s.land_friction
    }

    /// The vertical motion of the last move (`lastYd`).
    #[must_use]
    pub fn last_yd(&self) -> f64 {
        self.s.last_yd
    }

    /// The paddle input for the next tick: left, right, forward, back.
    #[must_use]
    pub fn input(&self) -> [bool; 4] {
        self.s.input
    }

    /// The turn speed (`deltaRotation`).
    #[must_use]
    pub fn delta_rotation(&self) -> f32 {
        self.s.delta_rotation
    }

    /// The paddle states of the last tick (left, right).
    #[must_use]
    pub fn paddles(&self) -> [bool; 2] {
        self.s.paddle
    }

    /// `isUnderWater()` (the status).
    #[must_use]
    pub fn is_under_water(&self) -> bool {
        matches!(
            self.s.status,
            Some(BoatStatus::UnderWater | BoatStatus::UnderFlowingWater)
        )
    }

    /// The server's `set_passengers` put the player on this boat (`startRiding`): standing pose,
    /// the player turned to the boat's yaw.
    pub fn board(&self, p: &mut PlayerState) {
        if p.pose != Pose::Standing {
            p.pose = Pose::Standing;
            p.dims = Pose::Standing.scaled(p.scale());
            let [x, y, z] = p.pos;
            p.set_pos(x, y, z);
        }
        p.yaw = self.e.yaw;
    }

    /// A `move_vehicle` packet from the server (`handleMoveVehicle`): the boat snaps to it unless
    /// it is within 1e-5; returns the `vehicle_move` packet the client answers with at once.
    pub fn server_move(&mut self, pos: [f64; 3], yaw: f32, pitch: f32) -> Packet {
        let d = V3::of(pos).sub(V3::of(self.e.pos));
        if d.length() > f64::from(1.0e-5f32) {
            // absSnapTo: position, then the rotation (yaw wrapped into 360, pitch clamped).
            self.e.set_pos(pos[0], pos[1], pos[2]);
            if yaw.is_finite() {
                self.e.yaw = yaw % 360.0;
            }
            if pitch.is_finite() {
                self.e.pitch = (pitch % 360.0).clamp(-90.0, 90.0);
            }
        }
        Packet::MoveVehicle {
            pos: self.e.pos,
            yaw: self.e.yaw,
            pitch: self.e.pitch,
            on_ground: self.e.on_ground,
        }
    }
}

/// One client tick of a player riding (and controlling) `boat`: mouse turning, the boat's tick,
/// the passenger's tick and its placing on the boat, in vanilla order (see the module docs).
/// `boat_colliders` are the boxes of the entities the boat collides with (other boats, shulkers,
/// mobs and other pushable entities, not its passengers); `around` is what the player's own tick
/// collides with (its boat excluded).
#[allow(clippy::too_many_arguments)]
pub fn tick_boat<R: Rules, W: BlockView<State = R::State>>(
    boat: &mut Boat,
    p: &mut PlayerState,
    x: &mut PlayerExt,
    around: &super::Surroundings<'_>,
    boat_colliders: &[Aabb],
    input: &TickInput,
    world: &W,
    out: &mut Packets,
) {
    // Mouse turning (Entity.turn -> onPassengerTurned).
    let turned = input.yaw.is_finite() || input.pitch.is_finite();
    if input.yaw.is_finite() {
        p.yaw = input.yaw;
    }
    if input.pitch.is_finite() {
        p.pitch = (input.pitch % 360.0).clamp(-90.0, 90.0);
    }
    if turned {
        clamp_rotation(p, boat.e.yaw);
    }
    boat_tick::<R, W>(boat, boat_colliders, world, out);
    // tickPassenger -> rideTick: motion zeroed, the player's tick, positionRider.
    p.vel = [0.0; 3];
    let mut g = Glide {
        fall_flying: x.fall_flying,
        glider: x.glider,
        spin_attack: x.spin_attack,
        auto_spin_ticks: x.auto_spin_ticks,
        riding: Some(Ride {
            boat: true,
            controlled: true,
            top: boat.e.bb.max_y,
            under_water: boat.is_under_water(),
            pos: boat.e.pos,
            yaw: boat.e.yaw,
            pitch: boat.e.pitch,
            on_ground: boat.e.on_ground,
        }),
        last_yd: 0.0,
    };
    let pending = x.pending_move.take();
    let keep = TickInput {
        yaw: f32::NAN,
        pitch: f32::NAN,
        ..*input
    };
    tick_core::<R, W, Ents<'_>>(p, &mut g, pending, Ents(around.entities), &keep, world, out);
    x.fall_flying = g.fall_flying;
    x.auto_spin_ticks = g.auto_spin_ticks;
    position_rider(boat, p);
    // LivingEntity.rideTick
    p.fall_distance = 0.0;
    // LocalPlayer.rideTick: the boat paddles with these keys in its next tick.
    let k = p.input.keys;
    boat.s.input = [k.left, k.right, k.forward, k.back];
}

/// A vehicle the player rides but the server moves (a minecart): where it is after its own client
/// tick (the entity tracker's interpolated position) and the height of its seat.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Seat {
    /// The vehicle's position after its tick.
    pub pos: [f64; 3],
    /// The passenger attachment height above the vehicle's position (`EntityType` passenger
    /// attachment: [`Seat::MINECART`] for every minecart).
    pub height: f64,
}

impl Seat {
    /// The passenger attachment height of minecarts (`passengerAttachments(0.1875F)`).
    pub const MINECART: f64 = 0.1875;

    /// The server's `set_passengers` put the player on such a vehicle (`startRiding`): the
    /// standing pose (the rotation is kept).
    pub fn board(p: &mut PlayerState) {
        if p.pose != Pose::Standing {
            p.pose = Pose::Standing;
            p.dims = Pose::Standing.scaled(p.scale());
            let [x, y, z] = p.pos;
            p.set_pos(x, y, z);
        }
    }

    /// A seat at `height` above a vehicle at `pos`.
    #[must_use]
    pub fn new(pos: [f64; 3], height: f64) -> Seat {
        Seat { pos, height }
    }
}

/// One client tick of a player riding a vehicle the server moves (a minecart): mouse turning (no
/// clamp), the passenger's tick (motion zeroed first; packets: `player_input` when the keys changed
/// and a rotation-only movement packet, no `vehicle_move`), then the seat (`positionRider`: the
/// vehicle's position plus the seat height minus the player's 0.6 vehicle attachment) and the
/// fall distance reset. The vehicle ticks before its passenger: pass its position after that tick.
pub fn tick_passenger<R: Rules, W: BlockView<State = R::State>>(
    p: &mut PlayerState,
    x: &mut PlayerExt,
    around: &super::Surroundings<'_>,
    seat: &Seat,
    input: &TickInput,
    world: &W,
    out: &mut Packets,
) {
    if input.yaw.is_finite() {
        p.yaw = input.yaw;
    }
    if input.pitch.is_finite() {
        p.pitch = (input.pitch % 360.0).clamp(-90.0, 90.0);
    }
    p.vel = [0.0; 3];
    let mut g = Glide {
        fall_flying: x.fall_flying,
        glider: x.glider,
        spin_attack: x.spin_attack,
        auto_spin_ticks: x.auto_spin_ticks,
        riding: Some(Ride {
            boat: false,
            controlled: false,
            top: 0.0,
            under_water: false,
            pos: seat.pos,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
        }),
        last_yd: 0.0,
    };
    let pending = x.pending_move.take();
    let keep = TickInput {
        yaw: f32::NAN,
        pitch: f32::NAN,
        ..*input
    };
    tick_core::<R, W, Ents<'_>>(p, &mut g, pending, Ents(around.entities), &keep, world, out);
    x.fall_flying = g.fall_flying;
    x.auto_spin_ticks = g.auto_spin_ticks;
    let attach = 0.6 * f64::from(p.scale());
    let y = seat.pos[1] + seat.height;
    p.set_pos(seat.pos[0], y - attach, seat.pos[2]);
    p.fall_distance = 0.0;
}

/// `Mth.wrapDegrees(float)`.
fn wrap_degrees(v: f32) -> f32 {
    let mut f = v % 360.0;
    if f >= 180.0 {
        f -= 360.0;
    }
    if f < -180.0 {
        f += 360.0;
    }
    f
}

/// `AbstractBoat.clampRotation`: the passenger's yaw within 105 degrees of the boat's.
fn clamp_rotation(p: &mut PlayerState, boat_yaw: f32) {
    let delta = wrap_degrees(p.yaw - boat_yaw);
    let target = if delta < -105.0 {
        -105.0
    } else {
        f32::min(delta, 105.0)
    };
    p.yaw = p.yaw + target - delta;
}

/// `positionRider`: the passenger on the boat's attachment point (minus its own vehicle
/// attachment, 0.6 blocks scaled), turned with the boat, rotation clamped.
fn position_rider(boat: &Boat, p: &mut PlayerState) {
    let off = f64::from(boat.kind.seat_offset());
    let a = -boat.e.yaw * DEG_TO_RAD;
    let (c, s) = (
        f64::from(m::cos(f64::from(a))),
        f64::from(m::sin(f64::from(a))),
    );
    // Vec3(0, rideHeight, offset).yRot(a)
    let px = 0.0 * c + off * s;
    let pz = off * c - 0.0 * s;
    let pos = V3::of(boat.e.pos).add(V3::new(px, boat.kind.ride_height(), pz));
    let attach = 0.6 * f64::from(p.scale());
    p.set_pos(pos.x, pos.y - attach, pos.z);
    p.yaw += boat.s.delta_rotation;
    clamp_rotation(p, boat.e.yaw);
}

/// `AbstractBoat.tick` of the controlled boat (`isLocalInstanceAuthoritative`, client side).
fn boat_tick<R: Rules, W: BlockView<State = R::State>>(
    b: &mut Boat,
    colliders: &[Aabb],
    world: &W,
    out: &mut Packets,
) {
    let (e, b) = (&mut b.e, &mut b.s);
    let cache = TickCache::new(world, floor(e.pos[0]), floor(e.pos[1]), floor(e.pos[2]));
    let border = Border::new(&world.world_border());
    let w = World::<R, TickCache<'_, W>, Vehicle<'_>> {
        w: &cache,
        border: &border,
        min_y: e.level.min_y,
        max_y: e.level.min_y + e.level.height - 1,
        ext: Vehicle(colliders),
        _r: core::marker::PhantomData,
    };
    // ClientLevel.tickNonPassenger: old position, tick count.
    let old_pos = V3::of(e.pos);
    e.age += 1;
    b.old_status = b.status;
    let st = get_status::<R, _>(&w, e, &mut b.water_level, &mut b.land_friction, b.last_yd);
    let status = st;
    b.status = Some(status);
    if matches!(
        status,
        BoatStatus::UnderWater | BoatStatus::UnderFlowingWater
    ) {
        b.out_of_control += 1.0;
    } else {
        b.out_of_control = 0.0;
    }
    let mut t = Ctx {
        p: e,
        x: Glide {
            last_yd: b.last_yd,
            ..Glide::default()
        },
        w: &w,
        old_pos,
        moves: [None; 4],
        using_item: false,
    };
    // Entity.baseTick
    t.p.was_in_powder_snow = t.p.in_powder_snow;
    t.p.in_powder_snow = false;
    t.update_fluid_interaction();
    if t.p.in_lava() {
        t.p.fall_distance *= 0.5;
    }
    // floatBoat, controlBoat, the paddle packet, the move.
    float_boat(b, &mut t, status);
    control_boat(b, &mut t);
    out.push(Packet::PaddleBoat {
        left: b.paddle[0],
        right: b.paddle[1],
    });
    let v = V3::of(t.p.vel);
    t.entity_move(v);
    t.apply_effects_from_blocks();
    t.apply_effects_from_blocks();
    b.last_yd = t.x.last_yd;
    // The paddles: where a stroke's sound would play, getPaddleSound() calls getStatus() again.
    let tau = (core::f64::consts::PI * 2.0) as f32;
    let quarter = (core::f64::consts::PI / 4.0) as f32;
    let eighth = (core::f64::consts::PI / 8.0) as f32;
    for i in 0..2 {
        if b.paddle[i] {
            let pp = b.paddle_pos[i];
            if pp % tau <= quarter && (pp + eighth) % tau >= quarter {
                let _ = get_status::<R, _>(&w, t.p, &mut b.water_level, &mut b.land_friction, 0.0);
            }
            b.paddle_pos[i] = pp + eighth;
        } else {
            b.paddle_pos[i] = 0.0;
        }
    }
}

/// `getStatus()` (also sets the water level and the land friction).
fn get_status<R: Rules, W: BlockView<State = R::State>>(
    w: &World<'_, R, W, Vehicle<'_>>,
    e: &PlayerState,
    water_level: &mut f64,
    land_friction: &mut f32,
    _last_yd: f64,
) -> BoatStatus {
    let bb = &e.bb;
    if let Some(s) = under_water(w, bb) {
        *water_level = bb.max_y;
        return s;
    }
    if check_in_water(w, bb, water_level) {
        return BoatStatus::InWater;
    }
    let f = ground_friction(w, bb);
    if f > 0.0 {
        *land_friction = f;
        BoatStatus::OnLand
    } else {
        BoatStatus::InAir
    }
}

/// `isUnderwater()`: water above the top of the box (+0.001); flowing water answers at once.
fn under_water<R: Rules, W: BlockView<State = R::State>>(
    w: &World<'_, R, W, Vehicle<'_>>,
    bb: &Aabb,
) -> Option<BoatStatus> {
    let max_y = bb.max_y + 0.001;
    let (x0, x1) = (floor(bb.min_x), ceil(bb.max_x));
    let (y0, y1) = (floor(bb.max_y), ceil(max_y));
    let (z0, z1) = (floor(bb.min_z), ceil(bb.max_z));
    let mut under = false;
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                let f = w.fluid(x, y, z);
                if kind(f) == 1 && max_y < f64::from(y) + f64::from(w.fluid_height(x, y, z, f)) {
                    if !is_source(f) {
                        return Some(BoatStatus::UnderFlowingWater);
                    }
                    under = true;
                }
            }
        }
    }
    under.then_some(BoatStatus::UnderWater)
}

/// `FluidState.isSource()`: amount 8, not falling.
fn is_source(f: u8) -> bool {
    amount(f) == 8 && f & 0x10 == 0
}

/// `checkInWater()`: water at the bottom of the box; the water level is the highest water top.
fn check_in_water<R: Rules, W: BlockView<State = R::State>>(
    w: &World<'_, R, W, Vehicle<'_>>,
    bb: &Aabb,
    water_level: &mut f64,
) -> bool {
    let (x0, x1) = (floor(bb.min_x), ceil(bb.max_x));
    let (y0, y1) = (floor(bb.min_y), ceil(bb.min_y + 0.001));
    let (z0, z1) = (floor(bb.min_z), ceil(bb.max_z));
    let mut in_water = false;
    *water_level = -f64::MAX;
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                let f = w.fluid(x, y, z);
                if kind(f) == 1 {
                    let height = y as f32 + w.fluid_height(x, y, z, f);
                    *water_level = m::java_max(f64::from(height), *water_level);
                    in_water |= bb.min_y < f64::from(height);
                }
            }
        }
    }
    in_water
}

/// `getGroundFriction()`: the mean friction of the blocks whose collision shapes touch a 0.001
/// thin box under the boat (lily pads excluded; corner columns and the top / bottom rows of edge
/// columns skipped). NaN when nothing touches.
fn ground_friction<R: Rules, W: BlockView<State = R::State>>(
    w: &World<'_, R, W, Vehicle<'_>>,
    bb: &Aabb,
) -> f32 {
    let q = Aabb {
        min_x: bb.min_x,
        min_y: bb.min_y - 0.001,
        min_z: bb.min_z,
        max_x: bb.max_x,
        max_y: bb.min_y,
        max_z: bb.max_z,
    };
    let (x0, x1) = (floor(q.min_x) - 1, ceil(q.max_x) + 1);
    let (y0, y1) = (floor(q.min_y) - 1, ceil(q.max_y) + 1);
    let (z0, z1) = (floor(q.min_z) - 1, ceil(q.max_z) + 1);
    let mut friction = 0.0f32;
    let mut count = 0i32;
    for x in x0..x1 {
        for z in z0..z1 {
            let edges = i32::from(x == x0 || x == x1 - 1) + i32::from(z == z0 || z == z1 - 1);
            if edges == 2 {
                continue;
            }
            for y in y0..y1 {
                if edges <= 0 || y != y0 && y != y1 - 1 {
                    let s = w.state(x, y, z);
                    if R::block(s).name != "lily_pad" && w.shape_overlaps(s, x, y, z, &q) {
                        friction += R::block(s).friction;
                        count += 1;
                    }
                }
            }
        }
    }
    friction / count as f32
}

/// `getWaterLevelAbove()`: the first row above the box top (down to `lastYd` below it) whose
/// water does not fill a block.
fn water_level_above<R: Rules, W: BlockView<State = R::State>>(
    w: &World<'_, R, W, Vehicle<'_>>,
    bb: &Aabb,
    last_yd: f64,
) -> f32 {
    let (x0, x1) = (floor(bb.min_x), ceil(bb.max_x));
    let (y0, y1) = (floor(bb.max_y), ceil(bb.max_y - last_yd));
    let (z0, z1) = (floor(bb.min_z), ceil(bb.max_z));
    'rows: for y in y0..y1 {
        let mut block_height = 0.0f32;
        for x in x0..x1 {
            for z in z0..z1 {
                let f = w.fluid(x, y, z);
                if kind(f) == 1 {
                    block_height = f32::max(block_height, w.fluid_height(x, y, z, f));
                }
                if block_height >= 1.0 {
                    continue 'rows;
                }
            }
        }
        if block_height < 1.0 {
            return y as f32 + block_height;
        }
    }
    (y1 + 1) as f32
}

/// `floatBoat()`.
fn float_boat<R: Rules, W: BlockView<State = R::State>>(
    b: &mut BoatState,
    t: &mut Ctx<'_, '_, R, TickCache<'_, W>, Vehicle<'_>>,
    status: BoatStatus,
) {
    let vspeed = -0.04;
    let mut buoyancy = 0.0;
    let inv_friction: f32;
    if b.old_status == Some(BoatStatus::InAir)
        && status != BoatStatus::InAir
        && status != BoatStatus::OnLand
    {
        b.water_level = t.p.pos[1] + f64::from(HEIGHT);
        let target_y = f64::from(water_level_above(t.w, &t.p.bb, t.x.last_yd) - HEIGHT) + 0.101;
        let q = super::math::mv(&t.p.bb, 0.0, target_y - t.p.pos[1], 0.0);
        let ctx = super::collide::ShapeCtx {
            feet_y: t.p.pos[1],
            descending: false,
            fall_distance: t.p.fall_distance,
            leather_boots: false,
        };
        if t.w.no_collision(t.p.pos, &q, &ctx) {
            let [x, _, z] = t.p.pos;
            t.p.set_pos(x, target_y, z);
            t.p.vel = [t.p.vel[0] * 1.0, t.p.vel[1] * 0.0, t.p.vel[2] * 1.0];
            t.x.last_yd = 0.0;
        }
        b.status = Some(BoatStatus::InWater);
        return;
    }
    let mut vs = vspeed;
    match status {
        BoatStatus::InWater => {
            buoyancy = (b.water_level - t.p.pos[1]) / f64::from(HEIGHT);
            inv_friction = 0.9;
        }
        BoatStatus::UnderFlowingWater => {
            vs = -7.0e-4;
            inv_friction = 0.9;
        }
        BoatStatus::UnderWater => {
            buoyancy = f64::from(0.01f32);
            inv_friction = 0.45;
        }
        BoatStatus::InAir => inv_friction = 0.9,
        BoatStatus::OnLand => {
            inv_friction = b.land_friction;
            // The controlling passenger is a player.
            b.land_friction /= 2.0;
        }
    }
    let v = t.p.vel;
    let k = f64::from(inv_friction);
    t.p.vel = [v[0] * k, v[1] + vs, v[2] * k];
    b.delta_rotation *= inv_friction;
    if buoyancy > 0.0 {
        let v = t.p.vel;
        t.p.vel = [v[0], (v[1] + buoyancy * (0.04 / 0.65)) * 0.75, v[2]];
    }
}

/// `controlBoat()` (the boat has a passenger): turning, acceleration, paddles.
fn control_boat<R: Rules, W: BlockView<State = R::State>>(
    b: &mut BoatState,
    t: &mut Ctx<'_, '_, R, TickCache<'_, W>, Vehicle<'_>>,
) {
    let [left, right, up, down] = b.input;
    let mut acc = 0.0f32;
    if left {
        b.delta_rotation -= 1.0;
    }
    if right {
        b.delta_rotation += 1.0;
    }
    if right != left && !up && !down {
        acc += 0.005;
    }
    t.p.yaw += b.delta_rotation;
    if up {
        acc += 0.04;
    }
    if down {
        acc -= 0.005;
    }
    let yaw = t.p.yaw;
    let dx = m::sin(f64::from(-yaw * DEG_TO_RAD)) * acc;
    let dz = m::cos(f64::from(yaw * DEG_TO_RAD)) * acc;
    let v = V3::of(t.p.vel).add(V3::new(f64::from(dx), 0.0, f64::from(dz)));
    t.p.vel = v.arr();
    b.paddle = [right && !left || up, left && !right || up];
}
