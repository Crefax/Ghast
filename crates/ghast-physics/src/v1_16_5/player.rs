//! The 1.16.5 client player tick: poses (crouching 1.5, swimming 0.6), sprint and flight toggles,
//! swimming, fluid heights, block speed / jump factors, voxel-shape collision with step-up and the
//! 1.16 ledge rule, and the movement packets.

// Vanilla's negated float comparisons are kept as written: they decide the NaN cases.
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::nonminimal_bool)]

use ghast_data::FluidKind;
use ghast_data::PhysicsFlags;
use ghast_data::v1_16_5::{Block, BlockState};

use super::blocks::{
    ShapeCtx, bamboo_box, collision_boxes, flags, is_bed, is_climbable_at, jump_multiplier,
    should_suffocate, slipperiness, state_at, velocity_multiplier,
};
use super::collide::{BorderShape, Collider, bx, contract, offset};
use super::fluid::{
    FluidContact, V3, ceil, fluid_at_point, height, region_loaded, update_in_fluid,
};
use crate::attribute::Modifiers;
use crate::input::{Abilities, Keys, Packet, Packets, TickInput};
use crate::math::{self, Aabb, floor};
use crate::world::{BlockView, TickCache};

/// Step-up height (`float`).
pub const STEP_HEIGHT: f32 = 0.6;
/// Player box width in every pose the movement code uses (`float`).
pub const WIDTH: f32 = 0.6;
const POSITION_LIMIT: f64 = 2.9999999e7;

/// Entity pose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Pose {
    /// Standing (1.8 high).
    #[default]
    Standing,
    /// Crouching (1.5 high).
    Crouching,
    /// Swimming / crawling (0.6 high).
    Swimming,
    /// Elytra flight (0.6 high; not simulated).
    FallFlying,
    /// Sleeping (0.2 × 0.2; not simulated).
    Sleeping,
}

impl Pose {
    /// Box `(width, height)` (`float`).
    #[must_use]
    pub fn dimensions(self) -> (f32, f32) {
        match self {
            Pose::Standing => (0.6, 1.8),
            Pose::Crouching => (0.6, 1.5),
            Pose::Swimming | Pose::FallFlying => (0.6, 0.6),
            Pose::Sleeping => (0.2, 0.2),
        }
    }

    /// Eye height (`float`).
    #[must_use]
    pub fn eye_height(self) -> f32 {
        match self {
            Pose::Swimming | Pose::FallFlying => 0.4,
            Pose::Crouching => 1.27,
            Pose::Standing => 1.62,
            Pose::Sleeping => 0.2,
        }
    }
}

/// Client movement input after this tick (vanilla `Input`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MovementInput {
    /// Forward impulse (×0.3 when slowed, ×0.2 using an item).
    pub forward: f32,
    /// Sideways impulse.
    pub sideways: f32,
    /// Forward key.
    pub pressing_forward: bool,
    /// Back key.
    pub pressing_back: bool,
    /// Left key.
    pub pressing_left: bool,
    /// Right key.
    pub pressing_right: bool,
    /// Jump key.
    pub jumping: bool,
    /// Sneak key.
    pub sneaking: bool,
}

/// Movement packet bookkeeping (what the server was last told).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SentState {
    /// Last sent x, y (feet), z.
    pub pos: [f64; 3],
    /// Last sent yaw.
    pub yaw: f32,
    /// Last sent pitch.
    pub pitch: f32,
    /// Last on-ground flag (an on-ground-only packet goes out only when it changes).
    pub on_ground: bool,
    /// Last sent sprint state.
    pub sprinting: bool,
    /// Last sent sneak (shift key) state.
    pub sneaking: bool,
    /// Ticks since the last position packet.
    pub ticks_since_pos: i32,
}

/// Status effects the client reads for movement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// Jump boost amplifier.
    pub jump_boost: Option<i32>,
    /// Blindness (blocks sprint starts).
    pub blindness: bool,
    /// Slow falling (gravity 0.01 while falling).
    pub slow_falling: bool,
    /// Levitation amplifier.
    pub levitation: Option<i32>,
    /// Dolphin's grace (water drag 0.96).
    pub dolphins_grace: bool,
}

/// Equipment enchantment levels that change movement (the client knows its own equipment).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Enchantments {
    /// Depth strider level on the boots.
    pub depth_strider: u8,
    /// Soul speed level on the boots (soul sand / soil then ignore their velocity factor).
    pub soul_speed: u8,
}

/// Movement speed attribute modifiers (see [`crate::attribute::Modifiers`]).
pub type SpeedModifiers = Modifiers;

/// The complete 1.16.5 movement state of the client player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    /// Position (feet).
    pub pos: [f64; 3],
    /// Bounding box.
    pub bb: Aabb,
    /// Velocity.
    pub vel: [f64; 3],
    /// Yaw.
    pub yaw: f32,
    /// Pitch.
    pub pitch: f32,
    /// Pose (decides the box size).
    pub pose: Pose,
    /// On ground.
    pub on_ground: bool,
    /// Horizontal collision of the last move (1e-5 tolerance).
    pub horizontal_collision: bool,
    /// Vertical collision of the last move.
    pub vertical_collision: bool,
    /// Set by jumps.
    pub velocity_dirty: bool,
    /// Fall distance.
    pub fall_distance: f32,
    /// Motion multiplier from cobweb / berry bush for the next move.
    pub movement_multiplier: [f64; 3],
    /// Touching water.
    pub touching_water: bool,
    /// Water height above the box bottom.
    pub water_height: f64,
    /// Lava height above the box bottom.
    pub lava_height: f64,
    /// Fluid at the eyes (vanilla `submergedFluidTag`).
    pub eye_fluid: Option<FluidKind>,
    /// Player "submerged in water" flag (eye fluid of the previous tick).
    pub submerged_in_water: bool,
    /// Swimming flag.
    pub swimming: bool,
    /// Sprinting flag.
    pub sprinting: bool,
    /// Client sneaking-pose decision of this tick.
    pub in_sneaking_pose: bool,
    /// Abilities.
    pub abilities: Abilities,
    /// Jump cooldown.
    pub jump_cooldown: i32,
    /// Jump input copied into the entity.
    pub jumping: bool,
    /// Forward speed after ×0.98.
    pub forward_speed: f32,
    /// Sideways speed after ×0.98.
    pub sideways_speed: f32,
    /// Air acceleration for the next tick.
    pub flying_speed: f32,
    /// Movement speed field.
    pub movement_speed: f32,
    /// Movement speed attribute base.
    pub movement_speed_base: f64,
    /// Movement speed modifiers: the server's (entity properties packet: speed / slowness
    /// potions, the soul speed boost) and the client's own sprint modifier, which sprinting adds
    /// and removes.
    pub speed_modifiers: SpeedModifiers,
    /// Spectator mode (no collision, no swimming or climbing).
    pub spectator: bool,
    /// No-clip movement this tick (set from spectator mode at the start of each tick).
    pub no_clip: bool,
    /// The client's auto-jump option (vanilla default on; bots normally turn it off).
    pub auto_jump: bool,
    /// Vanilla `autoJumpEnabled`: starts `true` and copies the option after each tick's packets,
    /// so the first tick can auto-jump even with the option off.
    pub auto_jump_enabled: bool,
    /// Ticks of forced jump input left from an auto-jump.
    pub ticks_to_next_autojump: i32,
    /// Creative flight double-tap window.
    pub fly_toggle_timer: i32,
    /// Sprint double-tap window.
    pub sprint_toggle_timer: i32,
    /// Ticks since sprinting last changed.
    pub ticks_since_sprinting_changed: i32,
    /// Food level.
    pub food_level: i32,
    /// Entity age in ticks.
    pub age: i32,
    /// Before the first entity tick (vanilla `firstUpdate`: lava is ignored).
    pub first_update: bool,
    /// Movement input after this tick.
    pub input: MovementInput,
    /// Packet bookkeeping.
    pub sent: SentState,
    /// Status effects.
    pub effects: Effects,
    /// Equipment enchantments.
    pub enchantments: Enchantments,
    /// Fell below y = −64 (vanilla marks the entity removed; the 1.16 client still ticks it).
    pub removed: bool,
}

/// Box of a pose at a position (vanilla `EntityDimensions.getBoxAt`).
#[must_use]
pub fn box_at(pose: Pose, x: f64, y: f64, z: f64) -> Aabb {
    let (w, h) = pose.dimensions();
    let r = f64::from(w / 2.0);
    bx(x - r, y, z - r, x + r, y + f64::from(h), z + r)
}

impl PlayerState {
    /// A freshly spawned client player (survival defaults, nothing sent yet).
    #[must_use]
    pub fn new(pos: [f64; 3], yaw: f32, pitch: f32) -> PlayerState {
        PlayerState {
            pos,
            bb: box_at(Pose::Standing, pos[0], pos[1], pos[2]),
            vel: [0.0; 3],
            yaw,
            pitch,
            pose: Pose::Standing,
            on_ground: false,
            horizontal_collision: false,
            vertical_collision: false,
            velocity_dirty: false,
            fall_distance: 0.0,
            movement_multiplier: [0.0; 3],
            touching_water: false,
            water_height: 0.0,
            lava_height: 0.0,
            eye_fluid: None,
            submerged_in_water: false,
            swimming: false,
            sprinting: false,
            in_sneaking_pose: false,
            abilities: Abilities::default(),
            jump_cooldown: 0,
            jumping: false,
            forward_speed: 0.0,
            sideways_speed: 0.0,
            flying_speed: 0.02,
            movement_speed: 0.0,
            movement_speed_base: f64::from(0.1f32),
            speed_modifiers: Modifiers::NONE,
            spectator: false,
            no_clip: false,
            auto_jump: false,
            auto_jump_enabled: true,
            ticks_to_next_autojump: 0,
            fly_toggle_timer: 0,
            sprint_toggle_timer: 0,
            ticks_since_sprinting_changed: 0,
            food_level: 20,
            age: 0,
            first_update: true,
            input: MovementInput::default(),
            sent: SentState::default(),
            effects: Effects::default(),
            enchantments: Enchantments::default(),
            removed: false,
        }
    }

    /// Movement speed attribute value including the sprint modifier.
    #[must_use]
    pub fn movement_speed_attr(&self) -> f64 {
        self.speed_modifiers.value(self.movement_speed_base)
    }

    /// Entity properties packet for the movement speed attribute: sets the base and replaces
    /// every modifier, the client's sprint modifier included (vanilla clears it too).
    /// `modifiers` are `(uuid, amount, operation)` in packet order.
    pub fn apply_speed_attribute(&mut self, base: f64, modifiers: &[(u128, f64, u8)]) {
        self.movement_speed_base = base;
        self.speed_modifiers.clear();
        for &(uuid, amount, op) in modifiers {
            self.speed_modifiers.insert(uuid, amount, op);
        }
    }

    /// Sneaking as vanilla's client reports it (shift key held).
    #[must_use]
    pub fn sneaking(&self) -> bool {
        self.input.sneaking
    }

    /// Vanilla `isSwimming()` for players.
    #[must_use]
    pub fn is_swimming(&self) -> bool {
        !self.abilities.flying && !self.spectator && self.swimming
    }

    /// Vanilla `isInLava()`.
    #[must_use]
    pub fn in_lava(&self) -> bool {
        !self.first_update && self.lava_height > 0.0
    }

    /// Eye height of the current pose.
    #[must_use]
    pub fn eye_height(&self) -> f32 {
        self.pose.eye_height()
    }

    /// Moves to a position with the current pose's box (vanilla `setPosition`).
    pub fn set_position(&mut self, x: f64, y: f64, z: f64) {
        self.pos = [x, y, z];
        self.bb = box_at(self.pose, x, y, z);
    }

    /// Entity velocity packet (raw 1/8000 units).
    pub fn apply_velocity_packet(&mut self, x: i32, y: i32, z: i32) {
        self.vel = [
            f64::from(x) / 8000.0,
            f64::from(y) / 8000.0,
            f64::from(z) / 8000.0,
        ];
    }

    /// Server position-and-look packet; `relative` = flags `[x, y, z, yaw, pitch]`. Returns the
    /// `PositionLook` reply (the client also sends a teleport confirm with the packet's id).
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
        let x = p[0].clamp(-3.0e7, 3.0e7);
        let z = p[2].clamp(-3.0e7, 3.0e7);
        self.set_position(x, p[1], z);
        self.yaw = yaw % 360.0;
        self.pitch = pitch.clamp(-90.0, 90.0) % 360.0;
        Packet::PositionLook {
            x: self.pos[0],
            y: self.pos[1],
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
    /// "ticks since sprinting changed" counter.
    pub fn set_sprinting(&mut self, on: bool) {
        self.sprinting = on;
        self.speed_modifiers.set_sprint(on);
        self.ticks_since_sprinting_changed = 0;
    }
}

/// `true` when the player is on a climbable block (vanilla `isClimbing`).
#[must_use]
pub fn is_climbing<W: BlockView<State = BlockState>>(p: &PlayerState, w: &W) -> bool {
    !p.spectator && is_climbable_at(w, floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2]))
}

/// Runs one 1.16.5 client tick (see [`crate::v1_8::tick`] for the calling convention).
pub fn tick<W: BlockView<State = BlockState>>(
    p: &mut PlayerState,
    input: &TickInput,
    world: &W,
    out: &mut Packets,
) {
    p.yaw = input.yaw;
    p.pitch = input.pitch;
    // The 1.16 client keeps ticking its own player even after `remove()` (below y = -64), and
    // the 1.16 client world reports every chunk as loaded: the player always ticks, and an
    // unloaded column is just an empty one.
    p.age += 1;
    let cache = TickCache::new(world, floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2]));
    let mut t = Ctx {
        p,
        w: &cache,
        border: BorderShape::new(&world.world_border()),
    };
    t.player_tick(&input.keys, input.using_item, out);
    t.send_movement(out);
}

struct Ctx<'a, W> {
    p: &'a mut PlayerState,
    w: &'a W,
    border: BorderShape,
}

impl<W: BlockView<State = BlockState>> Ctx<'_, W> {
    fn collider(&self) -> Collider<'_, W> {
        Collider {
            w: self.w,
            border: &self.border,
            ctx: ShapeCtx {
                min_y: self.p.pos[1],
                descending: self.p.input.sneaking,
            },
        }
    }

    fn block(&self, x: i32, y: i32, z: i32) -> BlockState {
        state_at(self.w, x, y, z)
    }

    fn vel(&self) -> V3 {
        V3::new(self.p.vel[0], self.p.vel[1], self.p.vel[2])
    }

    fn set_vel(&mut self, v: V3) {
        self.p.vel = [v.x, v.y, v.z];
    }

    fn player_tick(&mut self, keys: &Keys, using_item: bool, out: &mut Packets) {
        self.p.no_clip = self.p.spectator;
        if self.p.spectator {
            self.p.on_ground = false;
        }
        self.p.submerged_in_water = self.p.eye_fluid == Some(FluidKind::Water);
        self.base_tick();
        self.movement(keys, using_item, out);
        let p = &mut *self.p;
        let x = p.pos[0].clamp(-POSITION_LIMIT, POSITION_LIMIT);
        let z = p.pos[2].clamp(-POSITION_LIMIT, POSITION_LIMIT);
        if x != p.pos[0] || z != p.pos[2] {
            let y = p.pos[1];
            p.set_position(x, y, z);
        }
        self.update_pose();
    }

    fn base_tick(&mut self) {
        self.update_water_state();
        self.update_eye_fluid();
        // Swimming flag.
        let p = &mut *self.p;
        p.swimming = if p.abilities.flying {
            false
        } else if p.swimming {
            p.sprinting && p.touching_water
        } else {
            p.sprinting && p.submerged_in_water
        };
        if p.in_lava() {
            p.fall_distance *= 0.5;
        }
        if p.pos[1] < -64.0 {
            p.removed = true;
        }
        p.first_update = false;
    }

    fn check_water(&mut self) {
        let mut v = self.vel();
        let c: FluidContact = update_in_fluid(
            self.w,
            &self.p.bb,
            FluidKind::Water,
            0.014,
            !self.p.abilities.flying,
            &mut v,
        );
        self.set_vel(v);
        self.p.water_height = c.height;
        if c.touching {
            self.p.fall_distance = 0.0;
        }
        self.p.touching_water = c.touching;
    }

    fn update_water_state(&mut self) {
        self.p.water_height = 0.0;
        self.p.lava_height = 0.0;
        self.check_water();
        let mut v = self.vel();
        let c = update_in_fluid(
            self.w,
            &self.p.bb,
            FluidKind::Lava,
            0.002_333_333_333_333_333_5,
            !self.p.abilities.flying,
            &mut v,
        );
        self.set_vel(v);
        self.p.lava_height = c.height;
    }

    fn update_eye_fluid(&mut self) {
        let p = &mut *self.p;
        let eye = p.pos[1] + f64::from(p.eye_height()) - f64::from(0.111_111_11f32);
        p.eye_fluid = None;
        if let Some((f, [x, y, z])) = fluid_at_point(self.w, p.pos[0], eye, p.pos[2]) {
            // `int + float`: a float sum.
            let top = f64::from(y as f32 + height(self.w, x, y, z, f));
            if top > eye {
                p.eye_fluid = Some(f.kind);
            }
        }
    }

    /// Box of a pose at the current position (vanilla `calculateBoundsForPose`).
    fn pose_box(&self, pose: Pose) -> Aabb {
        let (w, h) = pose.dimensions();
        let f = f64::from(w / 2.0);
        let (x, y, z) = (self.p.pos[0], self.p.pos[1], self.p.pos[2]);
        bx(x - f, y, z - f, x + f, y + f64::from(h), z + f)
    }

    fn pose_fits(&self, pose: Pose) -> bool {
        let b = contract(&self.pose_box(pose), 1.0e-7);
        self.collider().is_space_empty(&self.p.bb, &b)
    }

    fn update_pose(&mut self) {
        let wanted = if self.p.is_swimming() {
            Pose::Swimming
        } else if self.p.input.sneaking && !self.p.abilities.flying {
            Pose::Crouching
        } else {
            Pose::Standing
        };
        // Vanilla asks "does the swimming box fit" first, then the wanted pose, then crouching.
        // The pose boxes are nested (same feet and width, heights 0.6 < 1.5 < 1.8), so a fitting
        // larger box answers the smaller ones: the common case costs one query instead of two.
        let pose = if self.p.spectator {
            if !self.pose_fits(Pose::Swimming) {
                return;
            }
            wanted
        } else if self.pose_fits(wanted) {
            wanted
        } else if wanted == Pose::Swimming || !self.pose_fits(Pose::Swimming) {
            return;
        } else if wanted == Pose::Standing && self.pose_fits(Pose::Crouching) {
            Pose::Crouching
        } else {
            Pose::Swimming
        };
        self.set_pose(pose);
    }

    /// Vanilla `calculateDimensions` on a pose change.
    fn set_pose(&mut self, pose: Pose) {
        let p = &mut *self.p;
        if pose == p.pose {
            return;
        }
        let (old_w, _) = p.pose.dimensions();
        let (w, h) = pose.dimensions();
        p.pose = pose;
        if w < old_w {
            let d = f64::from(w) / 2.0;
            p.bb = bx(
                p.pos[0] - d,
                p.pos[1],
                p.pos[2] - d,
                p.pos[0] + d,
                p.pos[1] + f64::from(h),
                p.pos[2] + d,
            );
        } else {
            let b = p.bb;
            p.bb = bx(
                b.min_x,
                b.min_y,
                b.min_z,
                b.min_x + f64::from(w),
                b.min_y + f64::from(h),
                b.min_z + f64::from(w),
            );
        }
    }

    fn is_walking(&self) -> bool {
        if self.p.submerged_in_water {
            self.p.input.forward > 1.0e-5
        } else {
            self.p.input.forward >= 0.8
        }
    }

    fn movement(&mut self, keys: &Keys, using_item: bool, out: &mut Packets) {
        let p = &mut *self.p;
        p.ticks_since_sprinting_changed += 1;
        if p.sprint_toggle_timer > 0 {
            p.sprint_toggle_timer -= 1;
        }
        let prev_jump = p.input.jumping;
        let prev_sneak = p.input.sneaking;
        let prev_walking = self.is_walking();
        // Vanilla: fits(crouching) && (sneaking || !fits(standing)). A fitting standing box
        // implies a fitting crouching box (nested boxes), so one query usually decides.
        let sneak_pose = if self.p.abilities.flying || self.p.is_swimming() {
            false
        } else if self.pose_fits(Pose::Standing) {
            self.p.input.sneaking
        } else {
            self.pose_fits(Pose::Crouching)
        };
        let p = &mut *self.p;
        p.in_sneaking_pose = sneak_pose;
        let slow = sneak_pose || (p.pose == Pose::Swimming && !p.touching_water);
        p.input = read_keys(keys, slow);
        if using_item {
            p.input.sideways *= 0.2;
            p.input.forward *= 0.2;
            p.sprint_toggle_timer = 0;
        }
        let mut autojumping = false;
        if p.ticks_to_next_autojump > 0 {
            p.ticks_to_next_autojump -= 1;
            autojumping = true;
            p.input.jumping = true;
        }

        let w = f64::from(WIDTH) * 0.35;
        let (x, z) = (p.pos[0], p.pos[2]);
        if !p.no_clip {
            // The four probes usually test the same few columns, and the box does not move
            // in between: remember each column's answer.
            let mut memo = ColumnMemo::default();
            self.push_out_of_blocks(x - w, z + w, &mut memo);
            self.push_out_of_blocks(x - w, z - w, &mut memo);
            self.push_out_of_blocks(x + w, z - w, &mut memo);
            self.push_out_of_blocks(x + w, z + w, &mut memo);
        }

        if prev_sneak {
            self.p.sprint_toggle_timer = 0;
        }
        let walking = self.is_walking();
        let p = &mut *self.p;
        let can_sprint = p.food_level as f32 > 6.0 || p.abilities.allow_flying;
        let allowed = can_sprint && !using_item && !p.effects.blindness;
        if (p.on_ground || p.submerged_in_water)
            && !prev_sneak
            && !prev_walking
            && walking
            && !p.sprinting
            && allowed
        {
            if p.sprint_toggle_timer <= 0 && !keys.sprint {
                p.sprint_toggle_timer = 7;
            } else {
                p.set_sprinting(true);
            }
        }
        if !p.sprinting
            && (!p.touching_water || p.submerged_in_water)
            && walking
            && allowed
            && keys.sprint
        {
            p.set_sprinting(true);
        }
        if p.sprinting {
            let no_forward = !(p.input.forward > 1.0e-5) || !can_sprint;
            let stop =
                no_forward || p.horizontal_collision || p.touching_water && !p.submerged_in_water;
            if p.is_swimming() {
                if !p.on_ground && !p.input.sneaking && no_forward || !p.touching_water {
                    p.set_sprinting(false);
                }
            } else if stop {
                p.set_sprinting(false);
            }
        }

        if p.abilities.allow_flying && p.spectator {
            // Spectators are locked into flight.
            if !p.abilities.flying {
                p.abilities.flying = true;
                out.push(Packet::Abilities(p.abilities));
            }
        } else if p.abilities.allow_flying && !prev_jump && p.input.jumping && !autojumping {
            if p.fly_toggle_timer == 0 {
                p.fly_toggle_timer = 7;
            } else if !p.is_swimming() {
                p.abilities.flying = !p.abilities.flying;
                out.push(Packet::Abilities(p.abilities));
                p.fly_toggle_timer = 0;
            }
        }
        // `method_29920` is "not flying" for players: fliers ignore liquids here.
        if p.touching_water && p.input.sneaking && !p.abilities.flying {
            p.vel[1] += f64::from(-0.04f32);
        }
        if p.abilities.flying {
            let j = i32::from(p.input.jumping) - i32::from(p.input.sneaking);
            if j != 0 {
                p.vel[1] += f64::from(j as f32 * p.abilities.fly_speed * 3.0);
            }
        }
        if p.fly_toggle_timer > 0 {
            p.fly_toggle_timer -= 1;
        }
        self.living_movement();
        let p = &mut *self.p;
        p.flying_speed = 0.02;
        if p.sprinting {
            p.flying_speed = (f64::from(p.flying_speed) + 0.005_999_999_865_889_549) as f32;
        }
        p.movement_speed = p.movement_speed_attr() as f32;
        if p.on_ground && p.abilities.flying && !p.spectator {
            p.abilities.flying = false;
            out.push(Packet::Abilities(p.abilities));
        }
    }

    /// Vanilla 1.16 client `pushOutOfBlocks(x, z)`.
    fn push_out_of_blocks(&mut self, x: f64, z: f64, memo: &mut ColumnMemo) {
        let (bx_, bz) = (floor(x), floor(z));
        if !self.would_collide_memo(bx_, bz, memo) {
            return;
        }
        let fx = x - f64::from(bx_);
        let fz = z - f64::from(bz);
        let mut best: Option<(i32, i32)> = None;
        let mut dist = f64::MAX;
        for (dx, dz, d) in [(-1, 0, fx), (1, 0, 1.0 - fx), (0, -1, fz), (0, 1, 1.0 - fz)] {
            if d < dist && !self.would_collide_memo(bx_ + dx, bz + dz, memo) {
                dist = d;
                best = Some((dx, dz));
            }
        }
        match best {
            Some((dx, 0)) => self.p.vel[0] = 0.1 * f64::from(dx),
            Some((0, dz)) => self.p.vel[2] = 0.1 * f64::from(dz),
            _ => {}
        }
    }

    fn would_collide_memo(&self, x: i32, z: i32, memo: &mut ColumnMemo) -> bool {
        if let Some(&(_, r)) = memo.entries[..memo.len].iter().find(|e| e.0 == (x, z)) {
            return r;
        }
        let r = self.would_collide_at(x, z);
        if memo.len < memo.entries.len() {
            memo.entries[memo.len] = ((x, z), r);
            memo.len += 1;
        }
        r
    }

    fn would_collide_at(&self, x: i32, z: i32) -> bool {
        let b = self.p.bb;
        let q = contract(
            &bx(
                f64::from(x),
                b.min_y,
                f64::from(z),
                f64::from(x) + 1.0,
                b.max_y,
                f64::from(z) + 1.0,
            ),
            1.0e-7,
        );
        !self
            .collider()
            .is_block_space_empty(&b, &q, should_suffocate)
    }

    fn living_movement(&mut self) {
        let p = &mut *self.p;
        if p.jump_cooldown > 0 {
            p.jump_cooldown -= 1;
        }
        for v in &mut p.vel {
            if v.abs() < 0.003 {
                *v = 0.0;
            }
        }
        p.sideways_speed = p.input.sideways;
        p.forward_speed = p.input.forward;
        p.jumping = p.input.jumping;
        if p.jumping && !p.abilities.flying {
            let k = if p.in_lava() {
                p.lava_height
            } else {
                p.water_height
            };
            let in_water = p.touching_water && k > 0.0;
            let m = if f64::from(p.eye_height()) < 0.4 {
                0.0
            } else {
                0.4
            };
            let swim_up =
                in_water && !(p.on_ground && !(k > m)) || p.in_lava() && !(p.on_ground && !(k > m));
            if swim_up {
                p.vel[1] += f64::from(0.04f32);
            } else if (p.on_ground || in_water && k <= m) && p.jump_cooldown == 0 {
                self.jump();
                self.p.jump_cooldown = 10;
            }
        } else {
            p.jump_cooldown = 0;
        }
        let p = &mut *self.p;
        p.sideways_speed *= 0.98;
        p.forward_speed *= 0.98;
        let input = V3::new(f64::from(p.sideways_speed), 0.0, f64::from(p.forward_speed));
        self.player_travel(input);
    }

    fn jump_velocity_multiplier(&self) -> f32 {
        let p = &self.p;
        let at = jump_multiplier(self.block(floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2])));
        if at == 1.0 {
            let [x, y, z] = self.velocity_pos();
            jump_multiplier(self.block(x, y, z))
        } else {
            at
        }
    }

    /// Vanilla `getVelocityAffectingPos`.
    fn velocity_pos(&self) -> [i32; 3] {
        [
            floor(self.p.pos[0]),
            floor(self.p.bb.min_y - 0.500_000_1),
            floor(self.p.pos[2]),
        ]
    }

    fn jump(&mut self) {
        let mut f = 0.42f32 * self.jump_velocity_multiplier();
        let p = &mut *self.p;
        if let Some(amp) = p.effects.jump_boost {
            f += 0.1 * (amp + 1) as f32;
        }
        p.vel[1] = f64::from(f);
        if p.sprinting {
            let g = p.yaw * (std::f64::consts::PI / 180.0) as f32;
            p.vel[0] += f64::from(-math::sin(g) * 0.2);
            p.vel[2] += f64::from(math::cos(g) * 0.2);
        }
        p.velocity_dirty = true;
    }

    fn rotation_y(&self) -> f64 {
        let f = self.p.pitch * (std::f64::consts::PI / 180.0) as f32;
        f64::from(-math::sin(f))
    }

    fn player_travel(&mut self, input: V3) {
        if self.p.is_swimming() {
            let g = self.rotation_y();
            let h = if g < -0.2 { 0.085 } else { 0.06 };
            let p = &self.p;
            let above = fluid_at_point(self.w, p.pos[0], p.pos[1] + 1.0 - 0.1, p.pos[2]).is_some();
            if g <= 0.0 || p.jumping || above {
                let v = self.vel();
                self.p.vel[1] = v.y + (g - v.y) * h;
            }
        }
        if self.p.abilities.flying {
            let vy = self.p.vel[1];
            let air = self.p.flying_speed;
            self.p.flying_speed =
                self.p.abilities.fly_speed * if self.p.sprinting { 2.0 } else { 1.0 };
            self.travel(input);
            self.p.vel[1] = vy * 0.6;
            self.p.flying_speed = air;
            self.p.fall_distance = 0.0;
        } else {
            self.travel(input);
        }
    }

    fn accelerate(&mut self, speed: f32, input: V3) {
        let d = input.length_sq();
        if d < 1.0e-7 {
            return;
        }
        let v = if d > 1.0 { input.normalize() } else { input }.scale(f64::from(speed));
        let r = self.p.yaw * (std::f64::consts::PI / 180.0) as f32;
        let (f, g) = (f64::from(math::sin(r)), f64::from(math::cos(r)));
        let add = V3::new(v.x * g - v.z * f, v.y, v.z * g + v.x * f);
        let nv = self.vel().add(add);
        self.set_vel(nv);
    }

    /// Sink rule while not sprinting in a liquid (vanilla `method_26317`).
    fn fluid_fall(&self, gravity: f64, falling: bool, v: V3) -> V3 {
        if self.p.sprinting {
            return v;
        }
        let y = if falling && (v.y - 0.005).abs() >= 0.003 && (v.y - gravity / 16.0).abs() < 0.003 {
            -0.003
        } else {
            v.y - gravity / 16.0
        };
        V3::new(v.x, y, v.z)
    }

    fn travel(&mut self, input: V3) {
        let mut gravity = 0.08;
        let falling = self.p.vel[1] <= 0.0;
        if falling && self.p.effects.slow_falling {
            gravity = 0.01;
            self.p.fall_distance = 0.0;
        }
        if self.p.touching_water && !self.p.abilities.flying {
            let start_y = self.p.pos[1];
            let mut f: f32 = if self.p.sprinting { 0.9 } else { 0.8 };
            let mut g: f32 = 0.02;
            let mut h = f32::from(self.p.enchantments.depth_strider);
            if h > 3.0 {
                h = 3.0;
            }
            if !self.p.on_ground {
                h *= 0.5;
            }
            if h > 0.0 {
                f += (0.546_000_06 - f) * h / 3.0;
                g += (self.p.movement_speed_attr() as f32 - g) * h / 3.0;
            }
            if self.p.effects.dolphins_grace {
                f = 0.96;
            }
            self.accelerate(g, input);
            let v = self.vel();
            self.move_self(v);
            let mut v = self.vel();
            if self.p.horizontal_collision && is_climbing(self.p, self.w) {
                v.y = 0.2;
            }
            let v = V3::new(
                v.x * f64::from(f),
                v.y * f64::from(0.8f32),
                v.z * f64::from(f),
            );
            let v = self.fluid_fall(gravity, falling, v);
            self.set_vel(v);
            self.jump_out(v, start_y);
        } else if self.p.in_lava() && !self.p.abilities.flying {
            let start_y = self.p.pos[1];
            self.accelerate(0.02, input);
            let v = self.vel();
            self.move_self(v);
            let swim_height = if f64::from(self.p.eye_height()) < 0.4 {
                0.0
            } else {
                0.4
            };
            let v = self.vel();
            let v = if self.p.lava_height <= swim_height {
                let v = V3::new(v.x * 0.5, v.y * f64::from(0.8f32), v.z * 0.5);
                self.fluid_fall(gravity, falling, v)
            } else {
                v.scale(0.5)
            };
            let v = V3::new(v.x, v.y + -gravity / 4.0, v.z);
            self.set_vel(v);
            self.jump_out(v, start_y);
        } else {
            let [px, py, pz] = self.velocity_pos();
            let t = slipperiness(self.block(px, py, pz));
            let u = if self.p.on_ground { t * 0.91 } else { 0.91 };
            let speed = if self.p.on_ground {
                self.p.movement_speed_attr() as f32 * (0.216_000_02 / (t * t * t))
            } else {
                self.p.flying_speed
            };
            self.accelerate(speed, input);
            self.climbing_speed();
            let v = self.vel();
            self.move_self(v);
            let mut v = self.vel();
            if (self.p.horizontal_collision || self.p.jumping) && is_climbing(self.p, self.w) {
                v.y = 0.2;
            }
            let mut y = v.y;
            if let Some(amp) = self.p.effects.levitation {
                y += (0.05 * f64::from(amp + 1) - v.y) * 0.2;
                self.p.fall_distance = 0.0;
            } else {
                y -= gravity;
            }
            let u = f64::from(u);
            self.set_vel(V3::new(v.x * u, y * f64::from(0.98f32), v.z * u));
        }
    }

    fn jump_out(&mut self, v: V3, start_y: f64) {
        if !self.p.horizontal_collision {
            return;
        }
        let dy = v.y + f64::from(0.6f32) - self.p.pos[1] + start_y;
        let probe = offset(&self.p.bb, v.x, dy, v.z);
        if self.collider().is_space_empty(&self.p.bb, &probe) && !self.contains_fluid(&probe) {
            self.p.vel[1] = f64::from(0.3f32);
        }
    }

    fn contains_fluid(&self, b: &Aabb) -> bool {
        let (x0, x1) = (floor(b.min_x), ceil(b.max_x));
        let (y0, y1) = (floor(b.min_y), ceil(b.max_y));
        let (z0, z1) = (floor(b.min_z), ceil(b.max_z));
        (x0..x1).any(|x| {
            (y0..y1).any(|y| (z0..z1).any(|z| super::blocks::fluid(self.block(x, y, z)).is_some()))
        })
    }

    fn climbing_speed(&mut self) {
        if !is_climbing(self.p, self.w) {
            return;
        }
        let p = &mut *self.p;
        p.fall_distance = 0.0;
        let lim = f64::from(0.15f32);
        p.vel[0] = p.vel[0].clamp(-lim, lim);
        p.vel[2] = p.vel[2].clamp(-lim, lim);
        let mut y = math::java_max(p.vel[1], -lim);
        let at = state_at(self.w, floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2]));
        if y < 0.0 && at.block() != Block::SCAFFOLDING && p.input.sneaking && !p.abilities.flying {
            y = 0.0;
        }
        p.vel[1] = y;
    }

    /// The client player's `move`: the entity move, then the auto-jump check.
    fn move_self(&mut self, movement: V3) {
        let (x, z) = (self.p.pos[0], self.p.pos[2]);
        self.entity_move(movement);
        let (dx, dz) = ((self.p.pos[0] - x) as f32, (self.p.pos[2] - z) as f32);
        self.auto_jump(dx, dz);
    }

    /// Vanilla `Entity.move(SELF, movement)`.
    fn entity_move(&mut self, movement: V3) {
        if self.p.no_clip {
            let p = &mut *self.p;
            p.bb = offset(&p.bb, movement.x, movement.y, movement.z);
            p.pos = [
                (p.bb.min_x + p.bb.max_x) / 2.0,
                p.bb.min_y,
                (p.bb.min_z + p.bb.max_z) / 2.0,
            ];
            return;
        }
        let mut m = movement;
        let mm = self.p.movement_multiplier;
        if mm[0] * mm[0] + mm[1] * mm[1] + mm[2] * mm[2] > 1.0e-7 {
            m = V3::new(m.x * mm[0], m.y * mm[1], m.z * mm[2]);
            self.p.movement_multiplier = [0.0; 3];
            self.p.vel = [0.0; 3];
        }
        m = self.adjust_for_ledge(m);
        let entity_box = self.p.bb;
        let v = self
            .collider()
            .adjust(m, &entity_box, self.p.on_ground, STEP_HEIGHT);
        let p = &mut *self.p;
        if v.length_sq() > 1.0e-7 {
            p.bb = offset(&p.bb, v.x, v.y, v.z);
            p.pos = [
                (p.bb.min_x + p.bb.max_x) / 2.0,
                p.bb.min_y,
                (p.bb.min_z + p.bb.max_z) / 2.0,
            ];
        }
        let approx = |a: f64, b: f64| (b - a).abs() < f64::from(1.0e-5f32);
        p.horizontal_collision = !approx(m.x, v.x) || !approx(m.z, v.z);
        p.vertical_collision = m.y != v.y;
        p.on_ground = p.vertical_collision && m.y < 0.0;

        let (lx, ly, lz) = (
            floor(p.pos[0]),
            floor(p.pos[1] - f64::from(0.2f32)),
            floor(p.pos[2]),
        );
        let mut landed = self.block(lx, ly, lz);
        if landed.is_air() {
            let below = self.block(lx, ly - 1, lz);
            let f = flags(below);
            if f.contains(PhysicsFlags::FENCE)
                || f.contains(PhysicsFlags::WALL)
                || f.contains(PhysicsFlags::FENCE_GATE)
            {
                landed = below;
            }
        }
        // Landing re-checks water (only when not already touching it).
        if !self.p.touching_water {
            self.check_water();
        }
        let p = &mut *self.p;
        if p.on_ground {
            p.fall_distance = 0.0;
        } else if v.y < 0.0 {
            p.fall_distance = (f64::from(p.fall_distance) - v.y) as f32;
        }
        let old = p.vel;
        if m.x != v.x {
            p.vel = [0.0, old[1], old[2]];
        }
        if m.z != v.z {
            // Vanilla rebuilds from the velocity read before the X reset.
            p.vel = [old[0], old[1], 0.0];
        }
        let block = landed.block();
        let sneaking = p.input.sneaking;
        if m.y != v.y {
            let bounce = (block == Block::SLIME_BLOCK || is_bed(landed)) && !sneaking;
            if bounce {
                if p.vel[1] < 0.0 {
                    let k = if block == Block::SLIME_BLOCK {
                        1.0
                    } else {
                        f64::from(0.66f32)
                    };
                    p.vel[1] = -p.vel[1] * k;
                }
            } else {
                p.vel[1] *= 0.0;
            }
        }
        if p.on_ground && !sneaking && block == Block::SLIME_BLOCK {
            let d = p.vel[1].abs();
            if d < 0.1 {
                let e = 0.4 + d * 0.2;
                p.vel[0] *= e;
                p.vel[2] *= e;
            }
        }
        self.block_contacts();
        let k = f64::from(self.velocity_multiplier());
        self.p.vel[0] *= k;
        self.p.vel[2] *= k;
    }

    fn shape_empty(&self, x: i32, y: i32, z: i32, ctx: &ShapeCtx) -> bool {
        collision_boxes(self.block(x, y, z), y, Some(ctx)).is_empty()
    }

    /// Vanilla client `autoJump`: schedules a jump when walking into a step 0.5..1.2 high.
    #[allow(clippy::too_many_lines)]
    fn auto_jump(&mut self, dx: f32, dz: f32) {
        let p = &*self.p;
        let has_input = p.input.sideways != 0.0 || p.input.forward != 0.0;
        if !(p.auto_jump_enabled
            && p.ticks_to_next_autojump <= 0
            && p.on_ground
            && !p.input.sneaking
            && has_input
            && f64::from(self.jump_velocity_multiplier()) >= 1.0)
        {
            return;
        }
        let pos = V3::new(p.pos[0], p.pos[1], p.pos[2]);
        let moved_to = pos.add(V3::new(f64::from(dx), 0.0, f64::from(dz)));
        let mut dir = V3::new(f64::from(dx), 0.0, f64::from(dz));
        let speed = p.movement_speed_attr() as f32;
        let mut g = dir.length_sq() as f32;
        if g <= 0.001 {
            let h = speed * p.input.sideways;
            let i = speed * p.input.forward;
            let r = p.yaw * (std::f64::consts::PI / 180.0) as f32;
            let (j, k) = (math::sin(r), math::cos(r));
            dir = V3::new(f64::from(h * k - i * j), dir.y, f64::from(i * k + h * j));
            g = dir.length_sq() as f32;
            if g <= 0.001 {
                return;
            }
        }
        let l = fast_inverse_sqrt(g);
        let unit = dir.scale(f64::from(l));
        let look = rotation_vector(p.pitch, p.yaw);
        let m = (look.x * unit.x + look.z * unit.z) as f32;
        if m < -0.15 {
            return;
        }
        let ctx = ShapeCtx {
            min_y: p.pos[1],
            descending: p.input.sneaking,
        };
        let (bx_, mut by, bz) = (floor(p.pos[0]), floor(p.bb.max_y), floor(p.pos[2]));
        if !self.shape_empty(bx_, by, bz, &ctx) {
            return;
        }
        by += 1;
        if !self.shape_empty(bx_, by, bz, &ctx) {
            return;
        }
        let mut o = 1.2f32;
        if let Some(amp) = p.effects.jump_boost {
            o += (amp + 1) as f32 * 0.75;
        }
        let reach = (speed * 7.0).max(1.0 / l);
        let end = moved_to.add(unit.scale(f64::from(reach)));
        let (w, h) = p.pose.dimensions();
        let (q, r) = (f64::from(w), f64::from(h));
        let raw = bx(pos.x, pos.y, pos.z, end.x, end.y + r, end.z);
        let search = bx(
            raw.min_x - q,
            raw.min_y,
            raw.min_z - q,
            raw.max_x + q,
            raw.max_y,
            raw.max_z + q,
        );
        let lift = f64::from(0.51f32);
        let start = pos.add(V3::new(0.0, lift, 0.0));
        let end = end.add(V3::new(0.0, lift, 0.0));
        // unit × (0, 1, 0), scaled by half the width.
        let cross = V3::new(
            unit.y * 0.0 - unit.z * 1.0,
            unit.z * 0.0 - unit.x * 0.0,
            unit.x * 1.0 - unit.y * 0.0,
        );
        let side = cross.scale(f64::from(w * 0.5));
        let a0 = V3::new(start.x - side.x, start.y - side.y, start.z - side.z);
        let a1 = V3::new(end.x - side.x, end.y - side.y, end.z - side.z);
        let b0 = start.add(side);
        let b1 = end.add(side);
        let seg = |bb: &Aabb, u: V3, v: V3| {
            bb.min_x < u.x.max(v.x)
                && bb.max_x > u.x.min(v.x)
                && bb.min_y < u.y.max(v.y)
                && bb.max_y > u.y.min(v.y)
                && bb.min_z < u.z.max(v.z)
                && bb.max_z > u.z.min(v.z)
        };
        // Java's Float.MIN_VALUE (the smallest subnormal) marks "nothing hit".
        let mut top = f32::from_bits(1);
        let mut hit: Option<Aabb> = None;
        let col = self.collider();
        col.each_block_collision(&p.bb, &search, |[x, y, z]| {
            let s = state_at(self.w, x, y, z);
            for b in collision_boxes(s, y, Some(&ctx)) {
                // Bamboo's boxes carry its per-position model offset (the static one does not).
                let bb = if s.block() == Block::BAMBOO {
                    bamboo_box(x, z).at(x, y, z)
                } else {
                    b.at(x, y, z)
                };
                if seg(&bb, a0, a1) || seg(&bb, b0, b1) {
                    hit = Some(bb);
                    return false;
                }
            }
            true
        });
        if let Some(bb) = hit {
            top = bb.max_y as f32;
            let c = V3::new(
                bb.min_x + 0.5 * (bb.max_x - bb.min_x),
                bb.min_y + 0.5 * (bb.max_y - bb.min_y),
                bb.min_z + 0.5 * (bb.max_z - bb.min_z),
            );
            let (cx, cy, cz) = (floor(c.x), floor(c.y), floor(c.z));
            let mut t = 1;
            while (t as f32) < o {
                let ty = cy + t;
                let s = state_at(self.w, cx, ty, cz);
                let boxes = collision_boxes(s, ty, Some(&ctx));
                if !boxes.is_empty() {
                    let max_y = boxes
                        .iter()
                        .map(|b| b.max_y)
                        .fold(f64::NEG_INFINITY, f64::max);
                    top = max_y as f32 + ty as f32;
                    if f64::from(top) - p.pos[1] > f64::from(o) {
                        return;
                    }
                }
                if t > 1 {
                    by += 1;
                    if !self.shape_empty(bx_, by, bz, &ctx) {
                        return;
                    }
                }
                t += 1;
            }
        }
        if top != f32::from_bits(1) {
            let u = (f64::from(top) - p.pos[1]) as f32;
            if !(u <= 0.5) && !(u > o) {
                self.p.ticks_to_next_autojump = 1;
            }
        }
    }

    fn velocity_multiplier(&self) -> f32 {
        let p = &self.p;
        // Flying players ignore block speed factors.
        if p.abilities.flying {
            return 1.0;
        }
        let [vx, vy, vz] = self.velocity_pos();
        let soul_speed = self.p.enchantments.soul_speed > 0
            && !p.abilities.flying
            && flags(self.block(vx, vy, vz)).contains(PhysicsFlags::SOUL_SPEED);
        if soul_speed {
            return 1.0;
        }
        let here = self.block(floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2]));
        let f = velocity_multiplier(here);
        if here.block() == Block::WATER || here.block() == Block::BUBBLE_COLUMN || f != 1.0 {
            f
        } else {
            velocity_multiplier(self.block(vx, vy, vz))
        }
    }

    /// 1.16 sneak edge guard (vanilla `adjustMovementForSneaking`).
    fn adjust_for_ledge(&self, m: V3) -> V3 {
        let p = &self.p;
        if p.abilities.flying || !p.input.sneaking {
            return m;
        }
        let step = f64::from(STEP_HEIGHT);
        let col = self.collider();
        let b = p.bb;
        let supported = p.on_ground
            || p.fall_distance < STEP_HEIGHT
                && !col.is_space_empty(
                    &b,
                    &offset(&b, 0.0, f64::from(p.fall_distance - STEP_HEIGHT), 0.0),
                );
        if !supported {
            return m;
        }
        let (mut d, mut e) = (m.x, m.z);
        let shrink = |v: f64| {
            if (-0.05..0.05).contains(&v) {
                0.0
            } else if v > 0.0 {
                v - 0.05
            } else {
                v + 0.05
            }
        };
        while d != 0.0 && col.is_space_empty(&b, &offset(&b, d, -step, 0.0)) {
            d = shrink(d);
        }
        while e != 0.0 && col.is_space_empty(&b, &offset(&b, 0.0, -step, e)) {
            e = shrink(e);
        }
        while d != 0.0 && e != 0.0 && col.is_space_empty(&b, &offset(&b, d, -step, e)) {
            d = shrink(d);
            e = shrink(e);
        }
        V3::new(d, m.y, e)
    }

    /// Effects of the blocks the box is in (vanilla `checkBlockCollision`).
    fn block_contacts(&mut self) {
        let b = self.p.bb;
        let (x0, y0, z0) = (
            floor(b.min_x + 0.001),
            floor(b.min_y + 0.001),
            floor(b.min_z + 0.001),
        );
        let (x1, y1, z1) = (
            floor(b.max_x - 0.001),
            floor(b.max_y - 0.001),
            floor(b.max_z - 0.001),
        );
        if !region_loaded(self.w, x0, y0, z0, x1, y1, z1) {
            return;
        }
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    let s = self.block(x, y, z);
                    self.on_collision(s, x, y, z);
                }
            }
        }
    }

    fn on_collision(&mut self, s: BlockState, x: i32, y: i32, z: i32) {
        let block = s.block();
        let above_air = self.block(x, y + 1, z).is_air();
        let p = &mut *self.p;
        if block == Block::COBWEB {
            if !p.abilities.flying {
                p.fall_distance = 0.0;
                p.movement_multiplier = [0.25, f64::from(0.05f32), 0.25];
            }
        } else if block == Block::SWEET_BERRY_BUSH {
            if !p.abilities.flying {
                p.fall_distance = 0.0;
                p.movement_multiplier = [f64::from(0.8f32), 0.75, f64::from(0.8f32)];
            }
        } else if block == Block::HONEY_BLOCK {
            let sliding = !p.on_ground
                && !(p.pos[1] > f64::from(y) + 0.9375 - 1.0e-7)
                && !(p.vel[1] >= -0.08)
                && {
                    let d = (f64::from(x) + 0.5 - p.pos[0]).abs();
                    let e = (f64::from(z) + 0.5 - p.pos[2]).abs();
                    let f = 0.4375 + f64::from(WIDTH / 2.0);
                    d + 1.0e-7 > f || e + 1.0e-7 > f
                };
            if sliding {
                if p.vel[1] < -0.13 {
                    let d = -0.05 / p.vel[1];
                    p.vel = [p.vel[0] * d, -0.05, p.vel[2] * d];
                } else {
                    p.vel[1] = -0.05;
                }
                p.fall_distance = 0.0;
            }
        } else if block == Block::BUBBLE_COLUMN {
            let drag = s.get(ghast_data::v1_16_5::Property::Drag) == Some("true");
            let v = p.vel[1];
            if above_air {
                p.vel[1] = if drag {
                    math::java_max(-0.9, v - 0.03)
                } else {
                    math::java_min(1.8, v + 0.1)
                };
            } else {
                p.vel[1] = if drag {
                    math::java_max(-0.3, v - 0.03)
                } else {
                    math::java_min(0.7, v + 0.06)
                };
                p.fall_distance = 0.0;
            }
        }
    }

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
        let sneaking = p.input.sneaking;
        if sneaking != p.sent.sneaking {
            out.push(if sneaking {
                Packet::StartSneaking
            } else {
                Packet::StopSneaking
            });
            p.sent.sneaking = sneaking;
        }
        let [x, y, z] = p.pos;
        let dx = x - p.sent.pos[0];
        let dy = y - p.sent.pos[1];
        let dz = z - p.sent.pos[2];
        p.sent.ticks_since_pos += 1;
        let moved = dx * dx + dy * dy + dz * dz > 9.0e-4 || p.sent.ticks_since_pos >= 20;
        let turned = p.yaw - p.sent.yaw != 0.0 || p.pitch - p.sent.pitch != 0.0;
        let (yaw, pitch, on_ground) = (p.yaw, p.pitch, p.on_ground);
        if moved && turned {
            out.push(Packet::PositionLook {
                x,
                y,
                z,
                yaw,
                pitch,
                on_ground,
            });
        } else if moved {
            out.push(Packet::Position { x, y, z, on_ground });
        } else if turned {
            out.push(Packet::Look {
                yaw,
                pitch,
                on_ground,
            });
        } else if p.sent.on_ground != on_ground {
            out.push(Packet::Flying { on_ground });
        }
        if moved {
            p.sent.pos = [x, y, z];
            p.sent.ticks_since_pos = 0;
        }
        if turned {
            p.sent.yaw = yaw;
            p.sent.pitch = pitch;
        }
        p.sent.on_ground = on_ground;
        p.auto_jump_enabled = p.auto_jump;
    }
}

/// Vanilla `MathHelper.fastInverseSqrt` (the bit-trick approximation).
fn fast_inverse_sqrt(x: f32) -> f32 {
    let half = 0.5 * x;
    let i = 1_597_463_007 - ((x.to_bits() as i32) >> 1);
    let y = f32::from_bits(i as u32);
    y * (1.5 - half * y * y)
}

/// Vanilla `getRotationVector(pitch, yaw)` (all `float`, widened).
fn rotation_vector(pitch: f32, yaw: f32) -> V3 {
    let f = pitch * (std::f64::consts::PI / 180.0) as f32;
    let g = -yaw * (std::f64::consts::PI / 180.0) as f32;
    let (h, i) = (math::cos(g), math::sin(g));
    let (j, k) = (math::cos(f), math::sin(f));
    V3::new(f64::from(i * j), f64::from(-k), f64::from(h * j))
}

/// Answers of the push-out column test within one tick.
#[derive(Default)]
struct ColumnMemo {
    entries: [((i32, i32), bool); 12],
    len: usize,
}

fn read_keys(k: &Keys, slow: bool) -> MovementInput {
    let axis = |a: bool, b: bool| {
        if a == b {
            0.0f32
        } else if a {
            1.0
        } else {
            -1.0
        }
    };
    let mut forward = axis(k.forward, k.back);
    let mut sideways = axis(k.left, k.right);
    if slow {
        sideways = (f64::from(sideways) * 0.3) as f32;
        forward = (f64::from(forward) * 0.3) as f32;
    }
    MovementInput {
        forward,
        sideways,
        pressing_forward: k.forward,
        pressing_back: k.back,
        pressing_left: k.left,
        pressing_right: k.right,
        jumping: k.jump,
        sneaking: k.sneak,
    }
}
