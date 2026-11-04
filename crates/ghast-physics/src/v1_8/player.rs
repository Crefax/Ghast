//! The 1.8.9 client player tick: input, sprint and flight toggles, jumping, travel (ground, air,
//! water, lava, ladders, flight), collision with step-up and the sneak edge guard, block contact
//! effects, and the movement packets of the tick.
//!
//! Every arithmetic step keeps vanilla's precision: `f32` where vanilla computes in `float`,
//! `f64` where it computes in `double`, with the widening (`f64::from(x_f32)`) at the same place.

use ghast_data::v1_8::BlockState;

use super::blocks::{
    BoxList, BoxSink, HistoryShapes, add_collision_boxes, id, is_air, is_full_cube,
    is_tall_support, is_valid_pos, slipperiness, state_at,
};
use super::fluid::{contains_lava, contains_liquid, water_contact};
use crate::attribute::Modifiers;
use crate::input::{Abilities, Keys, Packet, Packets, TickInput};
use crate::math::{self, Aabb, BoxOps, floor, sorted_box, sqrt_f32};
use crate::world::{BlockView, TickCache, WorldBorder};

/// Player box width (`float`).
pub const WIDTH: f32 = 0.6;
/// Player box height (`float`).
pub const HEIGHT: f32 = 1.8;
/// Step-up height (`float`).
pub const STEP_HEIGHT: f32 = 0.6;
/// Forward input needed to sprint (`float`).
const SPRINT_INPUT: f32 = 0.8;
/// Sprint speed modifier (multiplies the movement speed attribute; vanilla `0.3F` widened).
/// Air acceleration (`float`).
const AIR_ACCELERATION: f32 = 0.02;
/// Double-tap window for sprint and creative flight, in ticks.
const DOUBLE_TAP_TICKS: i32 = 7;
/// Ticks a sprint lasts before vanilla's client stops it.
const SPRINT_DURATION: i32 = 600;
/// Vanilla's position clamp for players.
const POSITION_LIMIT: f64 = 2.9999999e7;

/// The movement input after this tick (vanilla `Input`): next tick's toggles compare against it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MovementInput {
    /// Forward impulse `-1..=1`, ×0.3 while sneaking.
    pub forward: f32,
    /// Sideways impulse `-1..=1` (left positive), ×0.3 while sneaking.
    pub sideways: f32,
    /// Jump held.
    pub jump: bool,
    /// Sneak held.
    pub sneak: bool,
}

/// What the server was last told by movement packets (vanilla `sendMovementPackets` bookkeeping).
/// The server judges this tick's actions with this pose (see the project brief, tick model).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SentState {
    /// Last sent position: x, bounding box minimum y, z.
    pub pos: [f64; 3],
    /// Last sent yaw.
    pub yaw: f32,
    /// Last sent pitch.
    pub pitch: f32,
    /// Last sprint state sent by entity action.
    pub sprinting: bool,
    /// Last sneak state sent by entity action.
    pub sneaking: bool,
    /// Ticks since the last packet with a position (one is forced at 20).
    pub ticks_since_pos: i32,
}

/// Status effects the client reads for movement (1.8 sends speed / slowness only as attribute
/// modifiers: see [`PlayerState::speed_modifiers`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// Jump boost amplifier (level − 1), if active.
    pub jump_boost: Option<i32>,
    /// Blindness active (blocks starting to sprint).
    pub blindness: bool,
}

/// The complete movement state of the client player. Plain data: copy it to plan ahead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    /// Entity position (feet). After a move it is derived from [`PlayerState::bb`].
    pub pos: [f64; 3],
    /// Bounding box: the authoritative position during movement.
    pub bb: Aabb,
    /// Velocity (blocks per tick).
    pub vel: [f64; 3],
    /// Yaw in degrees (not wrapped).
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// On ground after the last move.
    pub on_ground: bool,
    /// The last move was blocked horizontally.
    pub horizontal_collision: bool,
    /// The last move was blocked vertically.
    pub vertical_collision: bool,
    /// Set by jumps (informational).
    pub velocity_dirty: bool,
    /// Fall distance.
    pub fall_distance: f32,
    /// Cobweb contact, consumed by the next move.
    pub in_web: bool,
    /// Touching water (updated at the start of the tick and on landing).
    pub touching_water: bool,
    /// Entity flag maintained by every collision query (world border handling).
    pub outside_world_border: bool,
    /// Sprinting.
    pub sprinting: bool,
    /// Abilities (flight).
    pub abilities: Abilities,
    /// Jump cooldown while jump is held.
    pub jump_cooldown: i32,
    /// Jump input copied into the entity this tick.
    pub jumping: bool,
    /// Forward input of the entity after the ×0.98 of travel.
    pub forward_speed: f32,
    /// Sideways input of the entity after the ×0.98 of travel.
    pub sideways_speed: f32,
    /// Air acceleration for the next tick (0.02, 0.026 sprinting).
    pub flying_speed: f32,
    /// Movement speed field (informational; ground speed reads the attribute).
    pub movement_speed: f32,
    /// Movement speed attribute base value (entity properties packet; 0.1 default).
    pub movement_speed_base: f64,
    /// Movement speed modifiers: the server's (entity properties packet, e.g. speed / slowness
    /// potions) and the client's own sprint modifier, which sprinting adds and removes.
    pub speed_modifiers: Modifiers,
    /// Spectator mode: no-clip flight that cannot be turned off, no climbing.
    pub spectator: bool,
    /// No-clip movement (vanilla sets it from spectator mode at the start of each tick).
    pub no_clip: bool,
    /// Boots depth strider level (water movement).
    pub depth_strider: u8,
    /// Creative flight double-tap window.
    pub fly_toggle_timer: i32,
    /// Sprint double-tap window.
    pub sprint_toggle_timer: i32,
    /// Counts down from 600 while sprinting; sprint stops at 0.
    pub sprinting_ticks_left: i32,
    /// Food level (sprinting needs more than 6 unless flight is allowed).
    pub food_level: i32,
    /// Ticks the entity has existed.
    pub ticks_alive: i32,
    /// Movement input after the last tick.
    pub input: MovementInput,
    /// Movement packet bookkeeping.
    pub sent: SentState,
    /// Movement-relevant status effects.
    pub effects: Effects,
    /// Fell below y = −64: vanilla removes the entity and stops ticking it.
    pub removed: bool,
}

/// Bounding box of a player standing at `(x, y, z)` (vanilla `updatePosition`).
#[inline]
#[must_use]
pub fn box_at(x: f64, y: f64, z: f64) -> Aabb {
    let r = f64::from(WIDTH / 2.0);
    sorted_box(x - r, y, z - r, x + r, y + f64::from(HEIGHT), z + r)
}

impl PlayerState {
    /// A freshly spawned client player at a position (what the join position packet produces):
    /// survival defaults, nothing sent yet.
    #[must_use]
    pub fn new(pos: [f64; 3], yaw: f32, pitch: f32) -> PlayerState {
        PlayerState {
            pos,
            bb: box_at(pos[0], pos[1], pos[2]),
            vel: [0.0; 3],
            yaw,
            pitch,
            on_ground: false,
            horizontal_collision: false,
            vertical_collision: false,
            velocity_dirty: false,
            fall_distance: 0.0,
            in_web: false,
            touching_water: false,
            outside_world_border: false,
            sprinting: false,
            abilities: Abilities::default(),
            jump_cooldown: 0,
            jumping: false,
            forward_speed: 0.0,
            sideways_speed: 0.0,
            flying_speed: AIR_ACCELERATION,
            movement_speed: 0.0,
            movement_speed_base: f64::from(0.1f32),
            speed_modifiers: Modifiers::NONE,
            spectator: false,
            no_clip: false,
            depth_strider: 0,
            fly_toggle_timer: 0,
            sprint_toggle_timer: 0,
            sprinting_ticks_left: 0,
            food_level: 20,
            ticks_alive: 0,
            input: MovementInput::default(),
            sent: SentState::default(),
            effects: Effects::default(),
            removed: false,
        }
    }

    /// Either collision flag.
    #[must_use]
    pub fn colliding(&self) -> bool {
        self.horizontal_collision || self.vertical_collision
    }

    /// Sneaking as the server and the movement code see it (= sneak key held).
    #[must_use]
    pub fn sneaking(&self) -> bool {
        self.input.sneak
    }

    /// Movement speed attribute value including the sprint modifier.
    #[must_use]
    pub fn movement_speed_attr(&self) -> f64 {
        self.speed_modifiers.value(self.movement_speed_base)
    }

    /// Entity properties packet for the movement speed attribute: sets the base and replaces
    /// every modifier, the client's sprint modifier included (vanilla clears it too; the
    /// server's list carries its own copy while the player sprints). `modifiers` are
    /// `(uuid, amount, operation)` in packet order.
    pub fn apply_speed_attribute(&mut self, base: f64, modifiers: &[(u128, f64, u8)]) {
        self.movement_speed_base = base;
        self.speed_modifiers.clear();
        for &(uuid, amount, op) in modifiers {
            self.speed_modifiers.insert(uuid, amount, op);
        }
    }

    /// Moves the player to a position, rebuilding the box (vanilla `updatePosition`).
    pub fn set_position(&mut self, x: f64, y: f64, z: f64) {
        self.pos = [x, y, z];
        self.bb = box_at(x, y, z);
    }

    /// Entity velocity packet (raw 1/8000 block-per-tick units).
    pub fn apply_velocity_packet(&mut self, x: i32, y: i32, z: i32) {
        self.vel = [
            f64::from(x) / 8000.0,
            f64::from(y) / 8000.0,
            f64::from(z) / 8000.0,
        ];
    }

    /// Server position-and-look packet. `relative` = flags `[x, y, z, yaw, pitch]`: a relative
    /// component is added to the current value, an absolute position component also zeroes that
    /// velocity component. Returns the confirmation packet vanilla sends right away (outside the
    /// tick; it does not update [`PlayerState::sent`]).
    pub fn apply_teleport(
        &mut self,
        pos: [f64; 3],
        yaw: f32,
        pitch: f32,
        relative: [bool; 5],
    ) -> Packet {
        let mut p = pos;
        for axis in 0..3 {
            if relative[axis] {
                p[axis] += self.pos[axis];
            } else {
                self.vel[axis] = 0.0;
            }
        }
        let yaw = if relative[3] { yaw + self.yaw } else { yaw };
        let pitch = if relative[4] {
            pitch + self.pitch
        } else {
            pitch
        };
        self.set_position(p[0], p[1], p[2]);
        self.yaw = yaw % 360.0;
        self.pitch = pitch % 360.0;
        Packet::PositionLook {
            x: self.pos[0],
            y: self.bb.min_y,
            z: self.pos[2],
            yaw: self.yaw,
            pitch: self.pitch,
            on_ground: false,
        }
    }

    /// Server abilities packet.
    pub fn apply_abilities(&mut self, a: Abilities) {
        self.abilities = a;
    }

    /// Sets sprinting like vanilla `setSprinting`: the flag, the sprint speed modifier and the
    /// 600-tick sprint countdown.
    pub fn set_sprinting(&mut self, on: bool) {
        self.sprinting = on;
        self.speed_modifiers.set_sprint(on);
        self.sprinting_ticks_left = if on { SPRINT_DURATION } else { 0 };
    }
}

/// `true` when the player's feet are in a ladder or vine block.
#[must_use]
pub fn is_climbing<W: BlockView<State = BlockState>>(p: &PlayerState, w: &W) -> bool {
    let s = state_at(w, floor(p.pos[0]), floor(p.bb.min_y), floor(p.pos[2]));
    matches!(s.block().id(), id::LADDER | id::VINE) && !p.spectator
}

/// `true` when the player's box (shrunk by 0.1 / 0.4 / 0.1) is in lava.
#[must_use]
pub fn is_touching_lava<W: BlockView<State = BlockState>>(p: &PlayerState, w: &W) -> bool {
    contains_lava(
        w,
        &p.bb
            .grow(-f64::from(0.1f32), -f64::from(0.4f32), -f64::from(0.1f32)),
    )
}

/// Runs one client tick: `input` is applied, the player moves, and the packets the client sends
/// at the end of the tick are appended to `out` (entity actions and abilities updates first,
/// then exactly one movement packet).
///
/// Inbound packets that arrived before the tick (velocity, teleport, abilities) must be applied
/// with the `apply_*` methods first, as vanilla handles them before ticking.
pub fn tick<W: BlockView<State = BlockState>>(
    p: &mut PlayerState,
    input: &TickInput,
    world: &W,
    out: &mut Packets,
) {
    tick_with(p, input, world, &Options::default(), out);
}

/// Rule options that select between vanilla behaviours which depend on client history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Options {
    /// Bounds of anvils and chests (default: [`HistoryShapes::Settled`]).
    pub history_shapes: HistoryShapes,
}

/// [`tick`] with explicit [`Options`].
pub fn tick_with<W: BlockView<State = BlockState>>(
    p: &mut PlayerState,
    input: &TickInput,
    world: &W,
    options: &Options,
    out: &mut Packets,
) {
    p.yaw = input.yaw;
    p.pitch = input.pitch;
    if p.removed {
        return;
    }
    p.ticks_alive += 1;
    p.no_clip = p.spectator;
    if p.spectator {
        p.on_ground = false;
    }
    // The client only ticks a player whose column is a valid position.
    if !is_valid_pos(floor(p.pos[0]), 0, floor(p.pos[2])) {
        return;
    }
    let cache = TickCache::new(world, floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2]));
    let mut cx = Ctx {
        p,
        w: &cache,
        border: world.world_border(),
        boxes: BoxList::new(),
        history: options.history_shapes,
    };
    cx.base_tick();
    cx.movement(&input.keys, input.using_item, out);
    cx.clamp_position();
    cx.send_movement(out);
}

struct Ctx<'a, W> {
    p: &'a mut PlayerState,
    w: &'a W,
    border: WorldBorder,
    boxes: BoxList,
    history: HistoryShapes,
}

impl<W: BlockView<State = BlockState>> Ctx<'_, W> {
    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> BlockState {
        state_at(self.w, x, y, z)
    }

    /// Start of the entity tick: water contact (with flow push), lava halves the fall distance,
    /// falling out of the world removes the entity.
    fn base_tick(&mut self) {
        self.update_water_state();
        if is_touching_lava(self.p, self.w) {
            self.p.fall_distance *= 0.5;
        }
        if self.p.pos[1] < -64.0 {
            self.p.removed = true;
        }
    }

    fn update_water_state(&mut self) {
        let probe = self
            .p
            .bb
            .grow(0.0, -f64::from(0.4f32), 0.0)
            .shrink(0.001, 0.001, 0.001);
        let (touching, flow) = water_contact(self.w, &probe);
        if touching && flow.length() > 0.0 && !self.p.abilities.flying {
            let f = flow.normalize();
            const PUSH: f64 = 0.014;
            self.p.vel[0] += f.x * PUSH;
            self.p.vel[1] += f.y * PUSH;
            self.p.vel[2] += f.z * PUSH;
        }
        if touching {
            self.p.fall_distance = 0.0;
        }
        self.p.touching_water = touching;
    }

    /// The client player's movement update (input, toggles, then the living-entity movement).
    fn movement(&mut self, keys: &Keys, using_item: bool, out: &mut Packets) {
        let p = &mut *self.p;
        if p.sprinting_ticks_left > 0 {
            p.sprinting_ticks_left -= 1;
            if p.sprinting_ticks_left == 0 {
                p.set_sprinting(false);
            }
        }
        if p.sprint_toggle_timer > 0 {
            p.sprint_toggle_timer -= 1;
        }

        let prev = p.input;
        let prev_forward_strong = prev.forward >= SPRINT_INPUT;
        p.input = read_keys(keys);
        if using_item {
            // Eating, drinking, blocking or drawing a bow: input ×0.2, no sprint double tap.
            p.input.sideways *= 0.2;
            p.input.forward *= 0.2;
            p.sprint_toggle_timer = 0;
        }

        let (x, y, z) = (p.pos[0], p.bb.min_y + 0.5, p.pos[2]);
        let r = f64::from(WIDTH) * 0.35;
        if !p.spectator {
            self.push_out_of_blocks(x - r, y, z + r);
            self.push_out_of_blocks(x - r, y, z - r);
            self.push_out_of_blocks(x + r, y, z - r);
            self.push_out_of_blocks(x + r, y, z + r);
        }

        let p = &mut *self.p;
        let can_sprint = p.food_level as f32 > 6.0 || p.abilities.allow_flying;
        let sprint_allowed = can_sprint && !using_item && !p.effects.blindness;
        let forward_strong = p.input.forward >= SPRINT_INPUT;
        if p.on_ground
            && !prev.sneak
            && !prev_forward_strong
            && forward_strong
            && !p.sprinting
            && sprint_allowed
        {
            if p.sprint_toggle_timer <= 0 && !keys.sprint {
                p.sprint_toggle_timer = DOUBLE_TAP_TICKS;
            } else {
                p.set_sprinting(true);
            }
        }
        if !p.sprinting && forward_strong && sprint_allowed && keys.sprint {
            p.set_sprinting(true);
        }
        if p.sprinting && (!forward_strong || p.horizontal_collision || !can_sprint) {
            p.set_sprinting(false);
        }

        if p.abilities.allow_flying && p.spectator {
            // Spectators are locked into flight.
            if !p.abilities.flying {
                p.abilities.flying = true;
                out.push(Packet::Abilities(p.abilities));
            }
        } else if p.abilities.allow_flying && !prev.jump && p.input.jump {
            if p.fly_toggle_timer == 0 {
                p.fly_toggle_timer = DOUBLE_TAP_TICKS;
            } else {
                p.abilities.flying = !p.abilities.flying;
                out.push(Packet::Abilities(p.abilities));
                p.fly_toggle_timer = 0;
            }
        }
        if p.abilities.flying {
            let impulse = f64::from(p.abilities.fly_speed * 3.0);
            if p.input.sneak {
                p.vel[1] -= impulse;
            }
            if p.input.jump {
                p.vel[1] += impulse;
            }
        }
        if p.fly_toggle_timer > 0 {
            p.fly_toggle_timer -= 1;
        }

        self.living_movement();

        let p = &mut *self.p;
        p.flying_speed = AIR_ACCELERATION;
        if p.sprinting {
            p.flying_speed =
                (f64::from(AIR_ACCELERATION) + f64::from(AIR_ACCELERATION) * 0.3) as f32;
        }
        p.movement_speed = p.movement_speed_attr() as f32;
        if p.on_ground && p.abilities.flying && !p.spectator {
            p.abilities.flying = false;
            out.push(Packet::Abilities(p.abilities));
        }
    }

    /// Nudges the player out of a full block it overlaps at one probe point (vanilla client
    /// `pushOutOfBlocks`): towards the nearest open horizontal side.
    fn push_out_of_blocks(&mut self, x: f64, y: f64, z: f64) {
        let (bx, by, bz) = (floor(x), floor(y), floor(z));
        let open = |cx: &Self, x: i32, z: i32| {
            !is_full_cube(cx.block(x, by, z)) && !is_full_cube(cx.block(x, by + 1, z))
        };
        if open(self, bx, bz) {
            return;
        }
        let fx = x - f64::from(bx);
        let fz = z - f64::from(bz);
        let mut best = None;
        let mut dist = 9999.0;
        for (dx, dz, d) in [(-1, 0, fx), (1, 0, 1.0 - fx), (0, -1, fz), (0, 1, 1.0 - fz)] {
            if open(self, bx + dx, bz + dz) && d < dist {
                dist = d;
                best = Some((dx, dz));
            }
        }
        let push = f64::from(0.1f32);
        match best {
            Some((dx, 0)) => self.p.vel[0] = f64::from(dx) * push,
            Some((0, dz)) => self.p.vel[2] = f64::from(dz) * push,
            _ => {}
        }
    }

    /// Living-entity movement: velocity cut-off, jump, input decay, travel.
    fn living_movement(&mut self) {
        let p = &mut *self.p;
        if p.jump_cooldown > 0 {
            p.jump_cooldown -= 1;
        }
        for v in &mut p.vel {
            if v.abs() < 0.005 {
                *v = 0.0;
            }
        }
        p.sideways_speed = p.input.sideways;
        p.forward_speed = p.input.forward;
        p.jumping = p.input.jump;
        if self.p.jumping {
            if self.p.touching_water || is_touching_lava(self.p, self.w) {
                // Swimming up in water or lava.
                self.p.vel[1] += f64::from(0.04f32);
            } else if self.p.on_ground && self.p.jump_cooldown == 0 {
                jump(self.p);
                self.p.jump_cooldown = 10;
            }
        } else {
            self.p.jump_cooldown = 0;
        }
        let p = &mut *self.p;
        p.sideways_speed *= 0.98;
        p.forward_speed *= 0.98;
        let (strafe, forward) = (p.sideways_speed, p.forward_speed);

        if p.abilities.flying {
            let vy = p.vel[1];
            let air = p.flying_speed;
            p.flying_speed = p.abilities.fly_speed * if p.sprinting { 2.0 } else { 1.0 };
            self.travel(strafe, forward);
            self.p.vel[1] = vy * 0.6;
            self.p.flying_speed = air;
        } else {
            self.travel(strafe, forward);
        }
    }

    /// Friction of the block under the player (×0.91), or plain 0.91 in the air.
    fn friction(&self) -> f32 {
        if self.p.on_ground {
            let s = self.block(
                floor(self.p.pos[0]),
                floor(self.p.bb.min_y) - 1,
                floor(self.p.pos[2]),
            );
            slipperiness(s) * 0.91
        } else {
            0.91
        }
    }

    fn travel(&mut self, strafe: f32, forward: f32) {
        let flying = self.p.abilities.flying;
        if self.p.touching_water && !flying {
            // Depth strider moves water drag and acceleration towards walking values.
            let mut drag = 0.8f32;
            let mut accel = 0.02f32;
            let mut j = f32::from(self.p.depth_strider).min(3.0);
            if !self.p.on_ground {
                j *= 0.5;
            }
            if j > 0.0 {
                drag += (0.546_000_06 - drag) * j / 3.0;
                accel += (self.p.movement_speed_attr() as f32 * 1.0 - accel) * j / 3.0;
            }
            self.travel_liquid(strafe, forward, accel, f64::from(drag), f64::from(0.8f32));
        } else if !flying && is_touching_lava(self.p, self.w) {
            self.travel_liquid(strafe, forward, 0.02, 0.5, 0.5);
        } else {
            self.travel_air(strafe, forward);
        }
    }

    /// Walking, falling, flying and climbing.
    fn travel_air(&mut self, strafe: f32, forward: f32) {
        let k = self.friction();
        let ground_accel = 0.162_771_36_f32 / (k * k * k);
        let accel = if self.p.on_ground {
            self.p.movement_speed_attr() as f32 * ground_accel
        } else {
            self.p.flying_speed
        };
        self.accelerate(strafe, forward, accel);
        let k = self.friction();

        let climbing = is_climbing(self.p, self.w);
        if climbing {
            let p = &mut *self.p;
            let lim = f64::from(0.15f32);
            p.vel[0] = p.vel[0].clamp(-lim, lim);
            p.vel[2] = p.vel[2].clamp(-lim, lim);
            p.fall_distance = 0.0;
            if p.vel[1] < -0.15 {
                p.vel[1] = -0.15;
            }
            if p.input.sneak && p.vel[1] < 0.0 {
                p.vel[1] = 0.0;
            }
        }

        let [vx, vy, vz] = self.p.vel;
        self.move_by(vx, vy, vz);
        if self.p.horizontal_collision && is_climbing(self.p, self.w) {
            self.p.vel[1] = 0.2;
        }

        // Gravity; a column the client has not received keeps the player hovering down slowly.
        let (tx, tz) = (self.p.pos[0] as i32, self.p.pos[2] as i32);
        let p = &mut *self.p;
        if !self.w.is_chunk_loaded(tx >> 4, tz >> 4) {
            p.vel[1] = if p.pos[1] > 0.0 { -0.1 } else { 0.0 };
        } else {
            p.vel[1] -= 0.08;
        }
        p.vel[1] *= f64::from(0.98f32);
        p.vel[0] *= f64::from(k);
        p.vel[2] *= f64::from(k);
    }

    /// Swimming in water (`drag` 0.8) or lava (0.5), including the jump out of the liquid at a
    /// ledge.
    fn travel_liquid(&mut self, strafe: f32, forward: f32, accel: f32, drag: f64, drag_y: f64) {
        let start_y = self.p.pos[1];
        self.accelerate(strafe, forward, accel);
        let [vx, vy, vz] = self.p.vel;
        self.move_by(vx, vy, vz);
        let p = &mut *self.p;
        p.vel[0] *= drag;
        p.vel[1] *= drag_y;
        p.vel[2] *= drag;
        p.vel[1] -= 0.02;
        if p.horizontal_collision {
            let [vx, vy, vz] = p.vel;
            let dy = vy + f64::from(0.6f32) - p.pos[1] + start_y;
            let probe = p.bb.offset(vx, dy, vz);
            if !self.collides(probe) && !contains_liquid(self.w, &probe) {
                self.p.vel[1] = f64::from(0.3f32);
            }
        }
    }

    /// Adds input acceleration in the look direction (vanilla `updateVelocity`, all `float`).
    fn accelerate(&mut self, strafe: f32, forward: f32, speed: f32) {
        let mut len = strafe * strafe + forward * forward;
        if len < 1.0e-4 {
            return;
        }
        len = sqrt_f32(len);
        if len < 1.0 {
            len = 1.0;
        }
        let scale = speed / len;
        let (s, f) = (strafe * scale, forward * scale);
        let angle = self.p.yaw * std::f32::consts::PI / 180.0;
        let (sin, cos) = (math::sin(angle), math::cos(angle));
        self.p.vel[0] += f64::from(s * cos - f * sin);
        self.p.vel[2] += f64::from(f * cos + s * sin);
    }

    /// World collision query (vanilla `World.doesBoxCollide` without entities): fills
    /// `self.boxes` with every block box intersecting `query`. With `first_only` it stops at the
    /// first box (the caller only needs emptiness). Updates the world-border flag like vanilla.
    fn gather(&mut self, query: Aabb, first_only: bool) -> bool {
        self.boxes.clear();
        let (x0, x1) = (floor(query.min_x), floor(query.max_x + 1.0));
        let (y0, y1) = (floor(query.min_y), floor(query.max_y + 1.0));
        let (z0, z1) = (floor(query.min_z), floor(query.max_z + 1.0));
        let was_outside = self.p.outside_world_border;
        let inside = self.inside_world();
        let mut visited = false;
        let (w, border) = (self.w, self.border);
        let mut sink = BoxSink {
            query,
            out: &mut self.boxes,
            history: self.history,
        };
        'scan: for x in x0..x1 {
            for z in z0..z1 {
                if !is_valid_pos(x, 64, z) {
                    continue;
                }
                visited = true;
                let substitute = inside && !border.contains_block(x, z);
                for y in (y0 - 1)..y1 {
                    let s = if substitute {
                        // Outside the border, blocks collide like stone while the entity is
                        // inside the world.
                        STONE
                    } else {
                        state_at(w, x, y, z)
                    };
                    add_collision_boxes(w, x, y, z, s, &mut sink);
                    if first_only && !sink.out.is_empty() {
                        break 'scan;
                    }
                }
            }
        }
        if visited {
            if was_outside && inside {
                self.p.outside_world_border = false;
            } else if !was_outside && !inside {
                self.p.outside_world_border = true;
            }
        }
        !self.boxes.is_empty()
    }

    /// `true` when any block box intersects `query`.
    #[inline]
    fn collides(&mut self, query: Aabb) -> bool {
        self.gather(query, true)
    }

    /// Vanilla `isInsideWorld`: the border shrunk (when already outside) or grown by one block.
    fn inside_world(&self) -> bool {
        let b = &self.border;
        let m = if self.p.outside_world_border {
            1.0
        } else {
            -1.0
        };
        let (w, n, e, s) = (b.west() + m, b.north() + m, b.east() - m, b.south() - m);
        let (x, z) = (self.p.pos[0], self.p.pos[2]);
        x > w && x < e && z > n && z < s
    }

    /// Moves the box by the wanted offset with collisions (vanilla `Entity.move`).
    fn move_by(&mut self, mut dx: f64, mut dy: f64, mut dz: f64) {
        if self.p.no_clip {
            // No-clip: the box just moves (vanilla keeps every collision flag as it was).
            let p = &mut *self.p;
            p.bb = p.bb.offset(dx, dy, dz);
            p.pos = [
                (p.bb.min_x + p.bb.max_x) / 2.0,
                p.bb.min_y,
                (p.bb.min_z + p.bb.max_z) / 2.0,
            ];
            return;
        }
        if self.p.in_web {
            self.p.in_web = false;
            dx *= 0.25;
            dy *= f64::from(0.05f32);
            dz *= 0.25;
            self.p.vel = [0.0; 3];
        }

        // Sneaking on the ground: shorten horizontal movement in 0.05 steps until the box would
        // still have ground under it one block down.
        let edge_guard = self.p.on_ground && self.p.input.sneak;
        if edge_guard {
            let bb = self.p.bb;
            while dx != 0.0 && !self.collides(bb.offset(dx, -1.0, 0.0)) {
                dx = edge_step(dx);
            }
            while dz != 0.0 && !self.collides(bb.offset(0.0, -1.0, dz)) {
                dz = edge_step(dz);
            }
            while dx != 0.0 && dz != 0.0 && !self.collides(bb.offset(dx, -1.0, dz)) {
                dx = edge_step(dx);
                dz = edge_step(dz);
            }
        }
        let (want_x, want_y, want_z) = (dx, dy, dz);
        let start = self.p.bb;

        self.gather(start.stretch(dx, dy, dz), false);
        let (mut bb, mx, my, mz) = sweep(self.boxes.as_slice(), start, dx, dy, dz);
        (dx, dy, dz) = (mx, my, mz);

        let may_step = self.p.on_ground || (want_y != dy && want_y < 0.0);
        if may_step && (want_x != dx || want_z != dz) {
            let up = f64::from(STEP_HEIGHT);
            self.gather(start.stretch(want_x, up, want_z), false);
            let boxes = self.boxes.as_slice();
            // Two ways to step up: rise against the whole horizontal sweep, or rise from the
            // start box; the one that gets further horizontally wins.
            let mut over = start;
            let mut over_y = up;
            let swept = start.stretch(want_x, 0.0, want_z);
            for b in boxes {
                over_y = b.clip_y(&swept, over_y);
            }
            over = over.offset(0.0, over_y, 0.0);
            let (over, over_x, over_z) = slide_xz(boxes, over, want_x, want_z);
            let (plain, plain_x, plain_y, plain_z) = sweep(boxes, start, want_x, up, want_z);
            let (sx, mut sy, sz, sbb) =
                if over_x * over_x + over_z * over_z > plain_x * plain_x + plain_z * plain_z {
                    (over_x, -over_y, over_z, over)
                } else {
                    (plain_x, -plain_y, plain_z, plain)
                };
            for b in boxes {
                sy = b.clip_y(&sbb, sy);
            }
            let sbb = sbb.offset(0.0, sy, 0.0);
            // Keep the step only when it gets further horizontally than the plain move (NaN
            // keeps the step, as in vanilla).
            #[allow(clippy::neg_cmp_op_on_partial_ord)]
            if !(dx * dx + dz * dz >= sx * sx + sz * sz) {
                (dx, dy, dz, bb) = (sx, sy, sz, sbb);
            }
        }

        let p = &mut *self.p;
        p.bb = bb;
        p.pos = [
            (bb.min_x + bb.max_x) / 2.0,
            bb.min_y,
            (bb.min_z + bb.max_z) / 2.0,
        ];
        p.horizontal_collision = want_x != dx || want_z != dz;
        p.vertical_collision = want_y != dy;
        p.on_ground = p.vertical_collision && want_y < 0.0;

        // The block that receives landing / stepping effects: under the feet, or a fence, wall or
        // gate one further down when that block is air.
        let (lx, ly, lz) = (
            floor(p.pos[0]),
            floor(p.pos[1] - f64::from(0.2f32)),
            floor(p.pos[2]),
        );
        let mut landed = self.block(lx, ly, lz);
        if is_air(landed) {
            let below = self.block(lx, ly - 1, lz);
            if is_tall_support(below) {
                landed = below;
            }
        }

        self.fall(dy);

        let p = &mut *self.p;
        if want_x != dx {
            p.vel[0] = 0.0;
        }
        if want_z != dz {
            p.vel[2] = 0.0;
        }
        let slime = landed.block().id() == id::SLIME;
        if want_y != dy {
            if slime && !p.input.sneak {
                if p.vel[1] < 0.0 {
                    p.vel[1] = -p.vel[1];
                }
            } else {
                p.vel[1] = 0.0;
            }
        }
        if !p.abilities.flying && !edge_guard && p.on_ground && slime {
            let vy = p.vel[1].abs();
            if vy < 0.1 && !p.input.sneak {
                let d = 0.4 + vy * 0.2;
                p.vel[0] *= d;
                p.vel[2] *= d;
            }
        }

        self.block_contacts();
    }

    /// Fall distance bookkeeping after a move; touching water on the way resets it.
    fn fall(&mut self, dy: f64) {
        if !self.p.touching_water {
            self.update_water_state();
        }
        let p = &mut *self.p;
        if p.on_ground {
            // Landing effects (fall damage) do not change client movement state.
            p.fall_distance = 0.0;
        } else if dy < 0.0 {
            p.fall_distance = (f64::from(p.fall_distance) - dy) as f32;
        }
    }

    /// Effects of blocks inside the (slightly shrunk) box: cobweb, soul sand.
    fn block_contacts(&mut self) {
        let bb = self.p.bb;
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
                    match self.block(x, y, z).block().id() {
                        id::WEB => {
                            if !self.p.abilities.flying {
                                self.p.in_web = true;
                                self.p.fall_distance = 0.0;
                            }
                        }
                        id::SOUL_SAND => {
                            self.p.vel[0] *= 0.4;
                            self.p.vel[2] *= 0.4;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Vanilla clamps player x / z to ±29,999,999 after the tick (no collision flag).
    fn clamp_position(&mut self) {
        let p = &mut *self.p;
        let x = p.pos[0].clamp(-POSITION_LIMIT, POSITION_LIMIT);
        let z = p.pos[2].clamp(-POSITION_LIMIT, POSITION_LIMIT);
        if x != p.pos[0] || z != p.pos[2] {
            let y = p.pos[1];
            p.set_position(x, y, z);
        }
    }

    /// Sprint / sneak entity actions and the one movement packet of the tick.
    fn send_movement(&mut self, out: &mut Packets) {
        let p = &mut *self.p;
        if p.sprinting != p.sent.sprinting {
            out.push(if p.sprinting {
                Packet::StartSprinting
            } else {
                Packet::StopSprinting
            });
            p.sent.sprinting = p.sprinting;
        }
        let sneaking = p.input.sneak;
        if sneaking != p.sent.sneaking {
            out.push(if sneaking {
                Packet::StartSneaking
            } else {
                Packet::StopSneaking
            });
            p.sent.sneaking = sneaking;
        }

        let (x, y, z) = (p.pos[0], p.bb.min_y, p.pos[2]);
        let dx = x - p.sent.pos[0];
        let dy = y - p.sent.pos[1];
        let dz = z - p.sent.pos[2];
        let moved = dx * dx + dy * dy + dz * dz > 9.0e-4 || p.sent.ticks_since_pos >= 20;
        let turned = p.yaw - p.sent.yaw != 0.0 || p.pitch - p.sent.pitch != 0.0;
        let on_ground = p.on_ground;
        let (yaw, pitch) = (p.yaw, p.pitch);
        out.push(match (moved, turned) {
            (true, true) => Packet::PositionLook {
                x,
                y,
                z,
                yaw,
                pitch,
                on_ground,
            },
            (true, false) => Packet::Position { x, y, z, on_ground },
            (false, true) => Packet::Look {
                yaw,
                pitch,
                on_ground,
            },
            (false, false) => Packet::Flying { on_ground },
        });
        p.sent.ticks_since_pos += 1;
        if moved {
            p.sent.pos = [x, y, z];
            p.sent.ticks_since_pos = 0;
        }
        if turned {
            p.sent.yaw = yaw;
            p.sent.pitch = pitch;
        }
    }
}

/// Stone, the block positions outside the world border collide as.
const STONE: BlockState = ghast_data::v1_8::Block::STONE.default_state();

/// One step of the sneak edge guard: towards zero by 0.05, snapping to zero inside ±0.05.
#[inline]
fn edge_step(v: f64) -> f64 {
    const STEP: f64 = 0.05;
    if (-STEP..STEP).contains(&v) {
        0.0
    } else if v > 0.0 {
        v - STEP
    } else {
        v + STEP
    }
}

/// Clips and applies a move in vanilla's axis order Y, X, Z. Returns the moved box and the
/// clipped offsets.
#[inline]
fn sweep(boxes: &[Aabb], start: Aabb, dx: f64, dy: f64, dz: f64) -> (Aabb, f64, f64, f64) {
    let mut dy = dy;
    for b in boxes {
        dy = b.clip_y(&start, dy);
    }
    let bb = start.offset(0.0, dy, 0.0);
    let (bb, dx, dz) = slide_xz(boxes, bb, dx, dz);
    (bb, dx, dy, dz)
}

/// The X then Z part of [`sweep`].
#[inline]
fn slide_xz(boxes: &[Aabb], start: Aabb, dx: f64, dz: f64) -> (Aabb, f64, f64) {
    let mut dx = dx;
    for b in boxes {
        dx = b.clip_x(&start, dx);
    }
    let bb = start.offset(dx, 0.0, 0.0);
    let mut dz = dz;
    for b in boxes {
        dz = b.clip_z(&bb, dz);
    }
    (bb.offset(0.0, 0.0, dz), dx, dz)
}

/// Movement input from held keys (vanilla `KeyboardInput`).
fn read_keys(k: &Keys) -> MovementInput {
    let mut forward = 0.0f32;
    let mut sideways = 0.0f32;
    if k.forward {
        forward += 1.0;
    }
    if k.back {
        forward -= 1.0;
    }
    if k.left {
        sideways += 1.0;
    }
    if k.right {
        sideways -= 1.0;
    }
    if k.sneak {
        sideways = (f64::from(sideways) * 0.3) as f32;
        forward = (f64::from(forward) * 0.3) as f32;
    }
    MovementInput {
        forward,
        sideways,
        jump: k.jump,
        sneak: k.sneak,
    }
}

/// A jump from the ground: 0.42 up (+0.1 per jump boost level), sprinting adds 0.2 forward.
fn jump(p: &mut PlayerState) {
    p.vel[1] = f64::from(0.42f32);
    if let Some(amp) = p.effects.jump_boost {
        p.vel[1] += f64::from((amp + 1) as f32 * 0.1);
    }
    if p.sprinting {
        let rad = p.yaw * (std::f64::consts::PI / 180.0) as f32;
        p.vel[0] -= f64::from(math::sin(rad) * 0.2);
        p.vel[2] += f64::from(math::cos(rad) * 0.2);
    }
    p.velocity_dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_matches_vanilla_widening() {
        let b = box_at(0.5, 64.0, 0.5);
        assert_eq!(b.min_x.to_bits(), 0x3fc9_9999_8000_0000);
        assert_eq!(b.max_y.to_bits(), 0x4050_7333_3300_0000);
    }

    #[test]
    fn stone_constant() {
        assert!(is_full_cube(STONE));
    }
}
