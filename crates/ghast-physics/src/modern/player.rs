//! The modern client player tick (`LocalPlayer.tick`): input, sprint and flight toggles, poses
//! scaled by the `scale` attribute, swimming, fluids, climbing, block effects, the movement
//! attributes, collision with candidate step-up heights, the sneak edge back-off, the
//! supporting block, inside-block effects swept along the move, auto-jump and the packets.

// Vanilla's negated float comparisons are kept as written: they decide the NaN cases.
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::nonminimal_bool)]

use ghast_data::v26_1::{BlockKind, block_flags as bf, state_flags as sf};

use super::attr::{Attribute, ModifierId, SPRINT_AMOUNT, sprinting_id};
use super::collide::{Border, Placed, ShapeCtx, World, collide, make_box};
use super::fluid::kind;
use super::inside::{Movement, Visited, axis_order, for_each_block_between};
use super::math::{
    self as m, DEG_TO_RAD, V3, approx_equal, bx, clamp, deflate, floor, intersects, java_max,
    java_min, mv, sqrt_f,
};
use super::rules::Rules;
use crate::input::{Abilities, Keys, Packet, Packets, TickInput};
use crate::math::Aabb;
use crate::world::{BlockView, TickCache};

/// Entity pose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Pose {
    /// Standing (0.6 x 1.8, eyes 1.62).
    #[default]
    Standing,
    /// Crouching (0.6 x 1.5, eyes 1.27).
    Crouching,
    /// Swimming / crawling (0.6 x 0.6, eyes 0.4).
    Swimming,
    /// Elytra flight (0.6 x 0.6; not simulated).
    FallFlying,
    /// Riptide spin (0.6 x 0.6; not simulated).
    SpinAttack,
    /// Sleeping (0.2 x 0.2, not scaled; not simulated).
    Sleeping,
    /// Dying (0.2 x 0.2, fixed; not simulated).
    Dying,
}

impl Pose {
    /// Unscaled `(width, height, eye height)` of the player in this pose (`Avatar.POSES`).
    #[must_use]
    pub fn dimensions(self) -> (f32, f32, f32) {
        match self {
            Pose::Standing => (0.6, 1.8, 1.62),
            Pose::Crouching => (0.6, 1.5, 1.27),
            Pose::Swimming | Pose::FallFlying | Pose::SpinAttack => (0.6, 0.6, 0.4),
            Pose::Sleeping => (0.2, 0.2, 0.2),
            Pose::Dying => (0.2, 0.2, 1.62),
        }
    }

    /// The dimensions scaled like `LivingEntity.getDimensions` (sleeping and dying are fixed).
    #[must_use]
    pub fn scaled(self, scale: f32) -> Dimensions {
        let (w, h, e) = self.dimensions();
        if matches!(self, Pose::Sleeping | Pose::Dying) || scale == 1.0 {
            Dimensions {
                width: w,
                height: h,
                eye_height: e,
            }
        } else {
            Dimensions {
                width: w * scale,
                height: h * scale,
                eye_height: e * scale,
            }
        }
    }
}

/// An entity's current box size and eye height (vanilla `dimensions` + `eyeHeight`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dimensions {
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
    /// Eye height.
    pub eye_height: f32,
}

/// The movement attributes of the client player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attributes {
    /// `movement_speed` (player base 0.1F, range 0..1024).
    pub movement_speed: Attribute,
    /// `step_height` (0.6, 0..10).
    pub step_height: Attribute,
    /// `gravity` (0.08, -1..1).
    pub gravity: Attribute,
    /// `jump_strength` (0.42F, 0..32).
    pub jump_strength: Attribute,
    /// `sneaking_speed` (0.3, 0..1).
    pub sneaking_speed: Attribute,
    /// `movement_efficiency` (0, 0..1).
    pub movement_efficiency: Attribute,
    /// `water_movement_efficiency` (0, 0..1).
    pub water_movement_efficiency: Attribute,
    /// `scale` (1, 0.0625..16).
    pub scale: Attribute,
    /// `safe_fall_distance` (3, -1024..1024).
    pub safe_fall_distance: Attribute,
}

impl Default for Attributes {
    fn default() -> Self {
        Attributes {
            movement_speed: Attribute::new(f64::from(0.1f32), 0.0, 1024.0),
            step_height: Attribute::new(0.6, 0.0, 10.0),
            gravity: Attribute::new(0.08, -1.0, 1.0),
            jump_strength: Attribute::new(f64::from(0.42f32), 0.0, 32.0),
            sneaking_speed: Attribute::new(0.3, 0.0, 1.0),
            movement_efficiency: Attribute::new(0.0, 0.0, 1.0),
            water_movement_efficiency: Attribute::new(0.0, 0.0, 1.0),
            scale: Attribute::new(1.0, 0.0625, 16.0),
            safe_fall_distance: Attribute::new(3.0, -1024.0, 1024.0),
        }
    }
}

/// Which attribute an attributes packet entry is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AttributeKind {
    /// `minecraft:movement_speed`.
    MovementSpeed,
    /// `minecraft:step_height`.
    StepHeight,
    /// `minecraft:gravity`.
    Gravity,
    /// `minecraft:jump_strength`.
    JumpStrength,
    /// `minecraft:sneaking_speed`.
    SneakingSpeed,
    /// `minecraft:movement_efficiency`.
    MovementEfficiency,
    /// `minecraft:water_movement_efficiency`.
    WaterMovementEfficiency,
    /// `minecraft:scale`.
    Scale,
    /// `minecraft:safe_fall_distance`.
    SafeFallDistance,
}

impl AttributeKind {
    /// The kind for a registry name (`minecraft:` optional); `None` for attributes movement
    /// does not read.
    #[must_use]
    pub fn from_name(name: &str) -> Option<AttributeKind> {
        Some(match name.strip_prefix("minecraft:").unwrap_or(name) {
            "movement_speed" => AttributeKind::MovementSpeed,
            "step_height" => AttributeKind::StepHeight,
            "gravity" => AttributeKind::Gravity,
            "jump_strength" => AttributeKind::JumpStrength,
            "sneaking_speed" => AttributeKind::SneakingSpeed,
            "movement_efficiency" => AttributeKind::MovementEfficiency,
            "water_movement_efficiency" => AttributeKind::WaterMovementEfficiency,
            "scale" => AttributeKind::Scale,
            "safe_fall_distance" => AttributeKind::SafeFallDistance,
            _ => return None,
        })
    }
}

impl Attributes {
    /// The instance of an attribute kind.
    pub fn get_mut(&mut self, k: AttributeKind) -> &mut Attribute {
        match k {
            AttributeKind::MovementSpeed => &mut self.movement_speed,
            AttributeKind::StepHeight => &mut self.step_height,
            AttributeKind::Gravity => &mut self.gravity,
            AttributeKind::JumpStrength => &mut self.jump_strength,
            AttributeKind::SneakingSpeed => &mut self.sneaking_speed,
            AttributeKind::MovementEfficiency => &mut self.movement_efficiency,
            AttributeKind::WaterMovementEfficiency => &mut self.water_movement_efficiency,
            AttributeKind::Scale => &mut self.scale,
            AttributeKind::SafeFallDistance => &mut self.safe_fall_distance,
        }
    }
}

/// Status effects the client reads for movement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// Jump boost amplifier.
    pub jump_boost: Option<i32>,
    /// Blindness (no sprinting: `isMobilityRestricted`).
    pub blindness: bool,
    /// Slow falling (gravity capped at 0.01 while falling).
    pub slow_falling: bool,
    /// Levitation amplifier.
    pub levitation: Option<i32>,
    /// Dolphin's grace (water drag 0.96).
    pub dolphins_grace: bool,
    /// Weaving (cobweb slows less).
    pub weaving: bool,
}

/// The item use effects of the item in use (`minecraft:use_effects` component; vanilla default:
/// no sprinting, speed x0.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UseEffects {
    /// `can_sprint`.
    pub can_sprint: bool,
    /// `speed_multiplier`.
    pub speed_multiplier: f32,
}

impl Default for UseEffects {
    fn default() -> Self {
        UseEffects {
            can_sprint: false,
            speed_multiplier: 0.2,
        }
    }
}

/// The client's movement input (`ClientInput`): held keys and the normalised move vector.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClientInput {
    /// Key presses (`Input`).
    pub keys: Keys,
    /// `(left impulse, forward impulse)`, normalised.
    pub move_vector: [f32; 2],
}

impl ClientInput {
    fn has_forward_impulse(&self) -> bool {
        self.move_vector[1] > 1.0e-5
    }
}

/// What the server was last told (`LocalPlayer` `xLast`, ... fields).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SentState {
    /// Last sent position.
    pub pos: [f64; 3],
    /// Last sent yaw.
    pub yaw: f32,
    /// Last sent pitch.
    pub pitch: f32,
    /// Last sent on-ground flag.
    pub on_ground: bool,
    /// Last sent horizontal collision flag.
    pub horizontal_collision: bool,
    /// Last sent sprint state.
    pub sprinting: bool,
    /// Ticks since the last position packet.
    pub position_reminder: i32,
    /// Last sent key state (`player_input`).
    pub input: Keys,
}

/// The level the player is in (from the login / respawn packet's dimension type).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelInfo {
    /// Lowest block Y.
    pub min_y: i32,
    /// Number of block rows.
    pub height: i32,
    /// `fast_lava` dimension attribute (the nether: lava pushes 0.007 instead of 0.0023333).
    pub fast_lava: bool,
}

impl Default for LevelInfo {
    fn default() -> Self {
        LevelInfo {
            min_y: -64,
            height: 384,
            fast_lava: false,
        }
    }
}

/// Client options movement reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClientOptions {
    /// Auto-jump (vanilla default off in 26.1).
    pub auto_jump: bool,
    /// Sprint double-tap window in ticks (vanilla default 7).
    pub sprint_window: i32,
}

impl Default for ClientOptions {
    fn default() -> Self {
        ClientOptions {
            auto_jump: false,
            sprint_window: 7,
        }
    }
}

/// The complete modern movement state of the client player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerState {
    /// Position (feet).
    pub pos: [f64; 3],
    /// Bounding box.
    pub bb: Aabb,
    /// Velocity (`deltaMovement`).
    pub vel: [f64; 3],
    /// Yaw.
    pub yaw: f32,
    /// Pitch.
    pub pitch: f32,
    /// Pose.
    pub pose: Pose,
    /// Current box size (vanilla `dimensions`; refreshed on pose and scale changes).
    pub dims: Dimensions,
    /// On ground.
    pub on_ground: bool,
    /// Horizontal collision of the last move.
    pub horizontal_collision: bool,
    /// The horizontal collision was "minor" (sprinting continues).
    pub minor_horizontal_collision: bool,
    /// Vertical collision of the last move.
    pub vertical_collision: bool,
    /// Vertical collision while moving down.
    pub vertical_collision_below: bool,
    /// Fall distance (a double in 26.x).
    pub fall_distance: f64,
    /// Stuck multiplier for the next move (cobweb, berry bush, powder snow); zero when unset.
    pub stuck_multiplier: [f64; 3],
    /// Main supporting block.
    pub supporting_block: Option<[i32; 3]>,
    /// On ground without a supporting block.
    pub on_ground_no_blocks: bool,
    /// Touching water (`wasTouchingWater`).
    pub touching_water: bool,
    /// Water height above the box bottom.
    pub water_height: f64,
    /// Lava height above the box bottom.
    pub lava_height: f64,
    /// Eyes in water (from the last fluid sweep).
    pub eye_in_water: bool,
    /// Eyes in lava (from the last fluid sweep).
    pub eye_in_lava: bool,
    /// `Player.wasUnderwater` (eyes in water at the start of the tick).
    pub was_underwater: bool,
    /// In powder snow (set by the inside-block sweep).
    pub in_powder_snow: bool,
    /// In powder snow last tick.
    pub was_in_powder_snow: bool,
    /// Before the first entity tick (`firstTick`: lava is ignored).
    pub first_tick: bool,
    /// Swimming flag.
    pub swimming: bool,
    /// Sprinting flag.
    pub sprinting: bool,
    /// Client crouching decision of this tick.
    pub crouching: bool,
    /// Abilities.
    pub abilities: Abilities,
    /// Spectator mode.
    pub spectator: bool,
    /// No-clip movement (set from spectator mode each tick).
    pub no_physics: bool,
    /// Jump cooldown (`noJumpDelay`).
    pub no_jump_delay: i32,
    /// Jump input copied into the entity.
    pub jumping: bool,
    /// Sideways input after modification (`xxa`).
    pub xxa: f32,
    /// Forward input after modification (`zza`).
    pub zza: f32,
    /// Creative flight double-tap window (`jumpTriggerTime`).
    pub jump_trigger_time: i32,
    /// Sprint double-tap window (`sprintTriggerTime`).
    pub sprint_trigger_time: i32,
    /// `autoJumpEnabled`: starts `true` and copies the option after each tick's packets.
    pub auto_jump_enabled: bool,
    /// Ticks of forced jump input left from an auto-jump.
    pub auto_jump_time: i32,
    /// Food level.
    pub food_level: i32,
    /// Ticks lived (`tickCount`).
    pub age: i32,
    /// Movement input of the last tick.
    pub input: ClientInput,
    /// Packet bookkeeping.
    pub sent: SentState,
    /// Movement attributes.
    pub attributes: Attributes,
    /// Status effects.
    pub effects: Effects,
    /// Use effects of the item in use.
    pub use_effects: UseEffects,
    /// Leather boots in the feet slot (powder snow).
    pub leather_boots: bool,
    /// Client options.
    pub options: ClientOptions,
    /// The level.
    pub level: LevelInfo,
    /// The scale attribute changed since dimensions were last refreshed.
    pub scale_dirty: bool,
}

impl PlayerState {
    /// A freshly spawned client player (survival defaults, nothing sent yet).
    #[must_use]
    pub fn new(pos: [f64; 3], yaw: f32, pitch: f32) -> PlayerState {
        let dims = Pose::Standing.scaled(1.0);
        PlayerState {
            pos,
            bb: make_box(dims.width, dims.height, pos[0], pos[1], pos[2]),
            vel: [0.0; 3],
            yaw,
            pitch,
            pose: Pose::Standing,
            dims,
            on_ground: false,
            horizontal_collision: false,
            minor_horizontal_collision: false,
            vertical_collision: false,
            vertical_collision_below: false,
            fall_distance: 0.0,
            stuck_multiplier: [0.0; 3],
            supporting_block: None,
            on_ground_no_blocks: false,
            touching_water: false,
            water_height: 0.0,
            lava_height: 0.0,
            eye_in_water: false,
            eye_in_lava: false,
            was_underwater: false,
            in_powder_snow: false,
            was_in_powder_snow: false,
            first_tick: true,
            swimming: false,
            sprinting: false,
            crouching: false,
            abilities: Abilities::default(),
            spectator: false,
            no_physics: false,
            no_jump_delay: 0,
            jumping: false,
            xxa: 0.0,
            zza: 0.0,
            jump_trigger_time: 0,
            sprint_trigger_time: 0,
            auto_jump_enabled: true,
            auto_jump_time: 0,
            food_level: 20,
            age: 0,
            input: ClientInput::default(),
            sent: SentState::default(),
            attributes: Attributes::default(),
            effects: Effects::default(),
            use_effects: UseEffects::default(),
            leather_boots: false,
            options: ClientOptions::default(),
            level: LevelInfo::default(),
            scale_dirty: false,
        }
    }

    /// `getAttributeValue(MOVEMENT_SPEED)`.
    #[must_use]
    pub fn movement_speed(&self) -> f64 {
        self.attributes.movement_speed.value()
    }

    /// `Player.getSpeed()` (the attribute, as a float).
    #[must_use]
    pub fn speed(&self) -> f32 {
        self.movement_speed() as f32
    }

    /// `maxUpStep()`.
    #[must_use]
    pub fn step_height(&self) -> f32 {
        self.attributes.step_height.value() as f32
    }

    /// `getScale()`.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.attributes.scale.value() as f32
    }

    /// `isSwimming()` for players.
    #[must_use]
    pub fn is_swimming(&self) -> bool {
        !self.abilities.flying && !self.spectator && self.swimming
    }

    /// `isInLava()`.
    #[must_use]
    pub fn in_lava(&self) -> bool {
        !self.first_tick && self.lava_height > 0.0
    }

    /// `isInWater()`.
    #[must_use]
    pub fn in_water(&self) -> bool {
        self.touching_water
    }

    /// `LocalPlayer.isUnderWater()`.
    #[must_use]
    pub fn is_underwater(&self) -> bool {
        self.was_underwater
    }

    /// `isShiftKeyDown()`.
    #[must_use]
    pub fn shift(&self) -> bool {
        self.input.keys.sneak
    }

    /// `isVisuallyCrawling()`.
    #[must_use]
    pub fn visually_crawling(&self) -> bool {
        self.pose == Pose::Swimming && !self.touching_water
    }

    /// `getEyeY()`.
    #[must_use]
    pub fn eye_y(&self) -> f64 {
        self.pos[1] + f64::from(self.dims.eye_height)
    }

    /// Moves to a position, rebuilding the box from the current dimensions (vanilla `setPos`).
    pub fn set_pos(&mut self, x: f64, y: f64, z: f64) {
        self.pos = [x, y, z];
        self.bb = make_box(self.dims.width, self.dims.height, x, y, z);
    }

    /// Sets sprinting like vanilla `LivingEntity.setSprinting`: the flag and the client's sprint
    /// speed modifier (removed, then added again when sprinting).
    pub fn set_sprinting(&mut self, on: bool) {
        self.sprinting = on;
        let id = sprinting_id();
        self.attributes.movement_speed.remove(id);
        if on {
            self.attributes.movement_speed.add(id, SPRINT_AMOUNT, 2);
        }
    }

    /// The attributes packet entry for one attribute: base and modifiers `(id, amount,
    /// operation)` in packet order (the client's sprint modifier is dropped like in vanilla
    /// unless the packet carries it).
    pub fn apply_attribute(
        &mut self,
        kind: AttributeKind,
        base: f64,
        modifiers: &[(ModifierId, f64, u8)],
    ) {
        self.attributes.get_mut(kind).apply_packet(base, modifiers);
        if kind == AttributeKind::Scale {
            self.scale_dirty = true;
        }
    }

    /// Entity motion packet (already decoded from its `LpVec3`).
    pub fn apply_velocity(&mut self, v: [f64; 3]) {
        if v.iter().all(|c| c.is_finite()) {
            self.vel = v;
        }
    }

    /// Server position packet with its relative flags (`Relative` bit set: 0 X, 1 Y, 2 Z,
    /// 3 Y_ROT, 4 X_ROT, 5 DELTA_X, 6 DELTA_Y, 7 DELTA_Z, 8 ROTATE_DELTA) and its delta movement.
    /// The client answers with `teleport_confirm` and a position-rotation packet (returned); it
    /// does not update the last sent position.
    pub fn apply_teleport(
        &mut self,
        pos: [f64; 3],
        delta: [f64; 3],
        yaw: f32,
        pitch: f32,
        relative: u32,
    ) -> Packet {
        let rel = |bit: u32| relative & (1 << bit) != 0;
        let ox = if rel(0) { self.pos[0] } else { 0.0 };
        let oy = if rel(1) { self.pos[1] } else { 0.0 };
        let oz = if rel(2) { self.pos[2] } else { 0.0 };
        let oyaw = if rel(3) { self.yaw } else { 0.0 };
        let opitch = if rel(4) { self.pitch } else { 0.0 };
        let new_pos = [ox + pos[0], oy + pos[1], oz + pos[2]];
        let new_yaw = oyaw + yaw;
        let new_pitch = m::clamp(f64::from(opitch + pitch), -90.0, 90.0) as f32;
        let mut cur = V3::of(self.vel);
        if rel(8) {
            let dy = self.yaw - new_yaw;
            let dx = self.pitch - new_pitch;
            cur = m::x_rot(cur, f64::from(dx).to_radians() as f32);
            cur = m::y_rot(cur, f64::from(dy).to_radians() as f32);
        }
        let d = |c: f64, ch: f64, bit: u32| if rel(bit) { c + ch } else { ch };
        let vel = [
            d(cur.x, delta[0], 5),
            d(cur.y, delta[1], 6),
            d(cur.z, delta[2], 7),
        ];
        self.set_pos(new_pos[0], new_pos[1], new_pos[2]);
        self.apply_velocity(vel);
        if new_yaw.is_finite() {
            self.yaw = new_yaw;
        }
        if new_pitch.is_finite() {
            self.pitch = (new_pitch % 360.0).clamp(-90.0, 90.0);
        }
        Packet::Move {
            pos: Some(self.pos),
            look: Some((self.yaw, self.pitch)),
            on_ground: false,
            horizontal_collision: false,
        }
    }

    /// Server abilities packet.
    pub fn apply_abilities(&mut self, a: Abilities) {
        self.abilities = a;
    }
}

/// `true` when the player is on a climbable block (vanilla `onClimbable`).
#[must_use]
pub fn on_climbable<R: Rules, W: BlockView<State = R::State>>(p: &PlayerState, w: &W) -> bool {
    let border = Border::new(&w.world_border());
    let world = World::<R, W> {
        w,
        border: &border,
        min_y: p.level.min_y,
        max_y: p.level.min_y + p.level.height - 1,
        _r: core::marker::PhantomData,
    };
    climbable::<R, W>(&world, p)
}

fn climbable<R: Rules, W: BlockView<State = R::State>>(
    w: &World<'_, R, W>,
    p: &PlayerState,
) -> bool {
    // Player.onClimbable: never while flying; LivingEntity: never for spectators.
    if p.abilities.flying || p.spectator {
        return false;
    }
    let (x, y, z) = (floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2]));
    let s = w.state(x, y, z);
    let d = R::block(s);
    if d.flags & bf::CLIMBABLE != 0 {
        return true;
    }
    if d.flags & bf::TRAPDOOR != 0 {
        let aux = R::phys(s).aux();
        if aux & 1 == 0 {
            return false;
        }
        let below = R::phys(w.state(x, y - 1, z));
        return below.kind() == BlockKind::Ladder as u8 && below.aux() == (aux >> 1) & 3;
    }
    false
}

/// Runs one modern client tick: `LocalPlayer.tick` and its movement packets, in order.
///
/// `input.using_item` is "an item is in use" (`isUsingItem`); its slowdown and sprint rule come
/// from [`PlayerState::use_effects`]. `tick_end` (sent by `Minecraft.tick` after every tick) is not
/// part of the output.
pub fn tick<R: Rules, W: BlockView<State = R::State>>(
    p: &mut PlayerState,
    input: &TickInput,
    world: &W,
    out: &mut Packets,
) {
    if f32::is_finite(input.yaw) {
        p.yaw = input.yaw;
    }
    if f32::is_finite(input.pitch) {
        p.pitch = (input.pitch % 360.0).clamp(-90.0, 90.0);
    }
    let cache = TickCache::new(world, floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2]));
    let border = Border::new(&world.world_border());
    let w = World::<R, TickCache<'_, W>> {
        w: &cache,
        border: &border,
        min_y: p.level.min_y,
        max_y: p.level.min_y + p.level.height - 1,
        _r: core::marker::PhantomData,
    };
    let mut t = Ctx {
        p,
        w: &w,
        old_pos: V3::ZERO,
        moves: [None; 4],
        using_item: input.using_item,
    };
    t.tick(&input.keys, out);
}

struct Ctx<'a, 'b, R: Rules, W> {
    p: &'a mut PlayerState,
    w: &'a World<'b, R, W>,
    old_pos: V3,
    moves: [Option<Movement>; 4],
    using_item: bool,
}

impl<R: Rules, W: BlockView<State = R::State>> Ctx<'_, '_, R, W> {
    fn ctx(&self) -> ShapeCtx {
        ShapeCtx {
            feet_y: self.p.pos[1],
            descending: self.p.shift(),
            fall_distance: self.p.fall_distance,
            leather_boots: self.p.leather_boots,
        }
    }

    fn vel(&self) -> V3 {
        V3::of(self.p.vel)
    }

    fn set_vel(&mut self, v: V3) {
        if v.is_finite() {
            self.p.vel = v.arr();
        }
    }

    fn pos(&self) -> V3 {
        V3::of(self.p.pos)
    }

    fn tick(&mut self, keys: &Keys, out: &mut Packets) {
        // ClientLevel.tickNonPassenger
        self.old_pos = self.pos();
        self.p.age += 1;
        // Player.tick
        self.p.no_physics = self.p.spectator;
        if self.p.spectator {
            self.set_on_ground(false, None);
        }
        // LocalPlayer.updateIsUnderwater
        self.p.was_underwater = self.p.eye_in_water;
        // LivingEntity.tick -> Entity.baseTick
        self.base_tick();
        // (updatingUsingItem: the caller tracks item use.)
        self.ai_step(keys, out);
        // refreshDirtyAttributes: a scale change rebuilds the box from the position.
        if self.p.scale_dirty {
            self.p.scale_dirty = false;
            self.refresh_dimensions();
        }
        // Player.tick continues
        let x = clamp(self.p.pos[0], -2.999_999_9e7, 2.999_999_9e7);
        let z = clamp(self.p.pos[2], -2.999_999_9e7, 2.999_999_9e7);
        if x != self.p.pos[0] || z != self.p.pos[2] {
            let y = self.p.pos[1];
            self.p.set_pos(x, y, z);
        }
        self.update_pose();
        // LocalPlayer.tick: player input, then position.
        if self.p.input.keys != self.p.sent.input {
            out.push(Packet::PlayerInput(self.p.input.keys));
            self.p.sent.input = self.p.input.keys;
        }
        self.send_position(out);
    }

    fn base_tick(&mut self) {
        let p = &mut *self.p;
        p.was_in_powder_snow = p.in_powder_snow;
        p.in_powder_snow = false;
        self.update_fluid_interaction();
        // updateSwimming (Player: flying -> false)
        let p = &mut *self.p;
        if p.abilities.flying {
            p.swimming = false;
        } else if p.is_swimming() {
            p.swimming = p.sprinting && p.touching_water;
        } else {
            let fluid_here = kind(
                self.w
                    .fluid(floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2])),
            );
            p.swimming = p.sprinting && p.is_underwater() && fluid_here == 1;
        }
        let p = &mut *self.p;
        if p.in_lava() {
            p.fall_distance *= 0.5;
        }
        p.first_tick = false;
    }

    /// `Entity.updateFluidInteraction`.
    fn update_fluid_interaction(&mut self) {
        let p = &*self.p;
        let pushed = !p.abilities.flying;
        let block_xz = [floor(p.pos[0]), floor(p.pos[2])];
        let fi = self
            .w
            .fluid_interaction(&p.bb, block_xz, p.eye_y(), !pushed);
        let p = &mut *self.p;
        p.water_height = fi.water.height;
        p.lava_height = fi.lava.height;
        p.eye_in_water = fi.water.eyes_inside;
        p.eye_in_lava = fi.lava.eyes_inside;
        let in_water = fi.water.height > 0.0;
        if in_water {
            p.fall_distance = 0.0;
        }
        p.touching_water = in_water;
        if pushed {
            if in_water && let Some(i) = fi.water.current_impulse(V3::of(p.vel), 0.014) {
                let v = V3::of(p.vel).add(i);
                if v.is_finite() {
                    p.vel = v.arr();
                }
            }
            if fi.lava.height > 0.0 {
                let scale = if p.level.fast_lava {
                    0.007
                } else {
                    0.002_333_333_333_333_333_5
                };
                if let Some(i) = fi.lava.current_impulse(V3::of(p.vel), scale) {
                    let v = V3::of(p.vel).add(i);
                    if v.is_finite() {
                        p.vel = v.arr();
                    }
                }
            }
        }
    }

    /// `canPlayerFitWithinBlocksAndEntitiesWhen(pose)`.
    fn fits(&self, pose: Pose) -> bool {
        let d = pose.scaled(self.p.scale());
        let b = deflate(
            &make_box(
                d.width,
                d.height,
                self.p.pos[0],
                self.p.pos[1],
                self.p.pos[2],
            ),
            1.0e-7,
        );
        self.w.no_collision(self.p.pos, &b, &self.ctx())
    }

    fn ai_step(&mut self, keys: &Keys, out: &mut Packets) {
        let p = &mut *self.p;
        if p.sprint_trigger_time > 0 {
            p.sprint_trigger_time -= 1;
        }
        let was_jumping = p.input.keys.jump;
        let was_shift = p.input.keys.sneak;
        let had_forward = p.input.has_forward_impulse();
        // Vanilla: fits(crouching) && (shift || !fits(standing)). At one scale the pose boxes are
        // nested (same feet and width, heights 0.6 < 1.5 < 1.8), so a fitting standing box
        // answers the crouching one.
        let crouching = if p.abilities.flying || p.is_swimming() {
            false
        } else if self.fits(Pose::Standing) {
            self.p.shift()
        } else {
            self.fits(Pose::Crouching)
        };
        let p = &mut *self.p;
        p.crouching = crouching;
        // KeyboardInput.tick
        p.input = read_keys(keys);
        let mut was_auto_jump = false;
        if p.auto_jump_time > 0 {
            p.auto_jump_time -= 1;
            was_auto_jump = true;
            p.input.keys.jump = true;
        }
        if !p.no_physics {
            let r = f64::from(p.dims.width) * 0.35;
            let (x, z) = (p.pos[0], p.pos[2]);
            self.move_towards_closest_space(x - r, z + r);
            self.move_towards_closest_space(x - r, z - r);
            self.move_towards_closest_space(x + r, z - r);
            self.move_towards_closest_space(x + r, z + r);
        }
        let slow_using = self.using_item && !self.p.use_effects.can_sprint;
        let p = &mut *self.p;
        if was_shift || slow_using || p.input.keys.back {
            p.sprint_trigger_time = 0;
        }
        if self.can_start_sprinting() {
            if !had_forward {
                if self.p.sprint_trigger_time > 0 {
                    self.p.set_sprinting(true);
                } else {
                    self.p.sprint_trigger_time = self.p.options.sprint_window;
                }
            }
            if self.p.input.keys.sprint {
                self.p.set_sprinting(true);
            }
        }
        if self.p.sprinting {
            let stop = if self.p.is_swimming() {
                self.should_stop_swim_sprinting()
            } else {
                self.should_stop_run_sprinting()
            };
            if stop {
                self.p.set_sprinting(false);
            }
        }
        let mut just_toggled = false;
        let p = &mut *self.p;
        if p.abilities.allow_flying {
            if p.spectator {
                if !p.abilities.flying {
                    p.abilities.flying = true;
                    just_toggled = true;
                    out.push(Packet::Abilities(p.abilities));
                }
            } else if !was_jumping && p.input.keys.jump && !was_auto_jump {
                if p.jump_trigger_time == 0 {
                    p.jump_trigger_time = 7;
                } else if !p.is_swimming() {
                    p.abilities.flying = !p.abilities.flying;
                    if p.abilities.flying && p.on_ground {
                        self.jump_from_ground();
                    }
                    just_toggled = true;
                    out.push(Packet::Abilities(self.p.abilities));
                    self.p.jump_trigger_time = 0;
                }
            }
        }
        // (jump && !just_toggled && !onClimbable && tryToStartFallFlying: needs an elytra, not modelled.)
        let _ = just_toggled;
        let p = &mut *self.p;
        if p.touching_water && p.input.keys.sneak && !p.abilities.flying {
            p.vel[1] += f64::from(-0.04f32);
        }
        if p.abilities.flying {
            let ya = i32::from(p.input.keys.jump) - i32::from(p.input.keys.sneak);
            if ya != 0 {
                p.vel[1] += f64::from(ya as f32 * p.abilities.fly_speed * 3.0);
            }
        }
        // Player.aiStep
        if p.jump_trigger_time > 0 {
            p.jump_trigger_time -= 1;
        }
        if p.abilities.flying {
            p.fall_distance = 0.0;
        }
        self.living_ai_step();
        let p = &mut *self.p;
        if p.on_ground && p.abilities.flying && !p.spectator {
            p.abilities.flying = false;
            out.push(Packet::Abilities(p.abilities));
        }
    }

    fn sprinting_possible(&self, allow_shallow: bool) -> bool {
        let p = &self.p;
        let food = p.food_level as f32 > 6.0 || p.abilities.allow_flying;
        !p.effects.blindness && food && (allow_shallow || !(p.touching_water && !p.is_underwater()))
    }

    fn can_start_sprinting(&self) -> bool {
        let p = &self.p;
        let slow_using = self.using_item && !p.use_effects.can_sprint;
        let moving_slowly = p.crouching || p.visually_crawling();
        !p.sprinting
            && p.input.has_forward_impulse()
            && self.sprinting_possible(p.abilities.flying)
            && !slow_using
            && (!moving_slowly || p.is_underwater())
    }

    fn should_stop_run_sprinting(&self) -> bool {
        let p = &self.p;
        !self.sprinting_possible(p.abilities.flying)
            || !p.input.has_forward_impulse()
            || p.horizontal_collision && !p.minor_horizontal_collision
    }

    fn should_stop_swim_sprinting(&self) -> bool {
        let p = &self.p;
        !self.sprinting_possible(true)
            || !p.touching_water
            || !p.input.has_forward_impulse() && !p.on_ground && !p.input.keys.sneak
    }

    /// `LocalPlayer.moveTowardsClosestSpace(x, z)`.
    fn move_towards_closest_space(&mut self, x: f64, z: f64) {
        let (bx_, bz) = (floor(x), floor(z));
        if !self.suffocates_at(bx_, bz) {
            return;
        }
        let xd = x - f64::from(bx_);
        let zd = z - f64::from(bz);
        let mut best: Option<(i32, i32)> = None;
        let mut closest = f64::MAX;
        // WEST, EAST, NORTH, SOUTH
        for (dx, dz, d) in [(-1, 0, xd), (1, 0, 1.0 - xd), (0, -1, zd), (0, 1, 1.0 - zd)] {
            if d < closest && !self.suffocates_at(bx_ + dx, bz + dz) {
                closest = d;
                best = Some((dx, dz));
            }
        }
        match best {
            Some((dx, 0)) => self.p.vel[0] = 0.1 * f64::from(dx),
            Some((0, dz)) => self.p.vel[2] = 0.1 * f64::from(dz),
            _ => {}
        }
    }

    fn suffocates_at(&self, x: i32, z: i32) -> bool {
        let b = self.p.bb;
        let area = deflate(
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
        self.w.suffocates_in_column(&area, &self.ctx())
    }

    fn living_ai_step(&mut self) {
        let p = &mut *self.p;
        if p.no_jump_delay > 0 {
            p.no_jump_delay -= 1;
        }
        let mut v = V3::of(p.vel);
        if v.horizontal_sqr() < R::HORIZONTAL_CUTOFF_SQ {
            v.x = 0.0;
            v.z = 0.0;
        }
        if v.y.abs() < 0.003 {
            v.y = 0.0;
        }
        p.vel = v.arr();
        // LocalPlayer.applyInput
        let mv_in = self.modify_input(self.p.input.move_vector);
        let p = &mut *self.p;
        p.xxa = mv_in[0];
        p.zza = mv_in[1];
        p.jumping = p.input.keys.jump;
        // jump
        if p.jumping && !p.abilities.flying {
            let h = if p.in_lava() {
                p.lava_height
            } else {
                p.water_height
            };
            let in_water = p.touching_water && h > 0.0;
            let thr = if p.dims.eye_height < 0.4 { 0.0 } else { 0.4 };
            if !in_water || p.on_ground && !(h > thr) {
                if !p.in_lava() || p.on_ground && !(h > thr) {
                    if (p.on_ground || in_water && h <= thr) && p.no_jump_delay == 0 {
                        self.jump_from_ground();
                        self.p.no_jump_delay = 10;
                    }
                } else {
                    p.vel[1] += f64::from(0.04f32);
                }
            } else {
                p.vel[1] += f64::from(0.04f32);
            }
        } else {
            p.no_jump_delay = 0;
        }
        let p = &mut *self.p;
        let input = V3::new(f64::from(p.xxa), 0.0, f64::from(p.zza));
        if p.effects.slow_falling || p.effects.levitation.is_some() {
            p.fall_distance = 0.0;
        }
        self.player_travel(input);
        self.apply_effects_from_blocks();
    }

    /// `LocalPlayer.modifyInput`.
    fn modify_input(&self, v: [f32; 2]) -> [f32; 2] {
        let [x, y] = v;
        if x * x + y * y == 0.0 {
            return v;
        }
        let (mut x, mut y) = (x * 0.98, y * 0.98);
        if self.using_item {
            let k = self.p.use_effects.speed_multiplier;
            x *= k;
            y *= k;
        }
        if self.p.crouching || self.p.visually_crawling() {
            let k = self.p.attributes.sneaking_speed.value() as f32;
            x *= k;
            y *= k;
        }
        // modifyInputSpeedForSquareMovement
        let len = sqrt_f(x * x + y * y);
        if len <= 0.0 {
            return [x, y];
        }
        let inv = 1.0 / len;
        let (dx, dy) = (x * inv, y * inv);
        let (ax, ay) = (dx.abs(), dy.abs());
        let tan = if ay > ax { ax / ay } else { ay / ax };
        let to_square = sqrt_f(1.0 + tan * tan);
        let l = (len * to_square).min(1.0);
        [dx * l, dy * l]
    }

    fn jump_power(&self) -> f32 {
        let p = &self.p;
        let boost = p.effects.jump_boost.map_or(0.0, |a| 0.1 * (a as f32 + 1.0));
        p.attributes.jump_strength.value() as f32 * 1.0 * self.block_jump_factor() + boost
    }

    /// `jumpFromGround`.
    fn jump_from_ground(&mut self) {
        let jp = self.jump_power();
        if jp <= 1.0e-5 {
            return;
        }
        let p = &mut *self.p;
        p.vel[1] = java_max(f64::from(jp), p.vel[1]);
        if p.sprinting {
            let a = p.yaw * DEG_TO_RAD;
            let add = V3::new(
                f64::from(-m::sin(f64::from(a))) * 0.2,
                0.0,
                f64::from(m::cos(f64::from(a))) * 0.2,
            );
            let v = V3::of(p.vel).add(add);
            if v.is_finite() {
                p.vel = v.arr();
            }
        }
    }

    /// `getBlockJumpFactor`.
    fn block_jump_factor(&self) -> f32 {
        let p = &self.p;
        let here = R::block(
            self.w
                .state(floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2])),
        )
        .jump_factor;
        let [x, y, z] = self.on_pos(0.500_001);
        let below = R::block(self.w.state(x, y, z)).jump_factor;
        if here == 1.0 { below } else { here }
    }

    /// `getBlockSpeedFactor` (Player, LivingEntity, Entity).
    fn block_speed_factor(&self) -> f32 {
        let p = &self.p;
        if p.abilities.flying {
            return 1.0;
        }
        let s = self
            .w
            .state(floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2]));
        let here = R::block(s).speed_factor;
        let k = R::phys(s).kind();
        let base = if k == BlockKind::Water as u8 || k == BlockKind::BubbleColumn as u8 {
            here
        } else if here == 1.0 {
            let [x, y, z] = self.on_pos(0.500_001);
            R::block(self.w.state(x, y, z)).speed_factor
        } else {
            here
        };
        let eff = p.attributes.movement_efficiency.value() as f32;
        base + eff * (1.0 - base)
    }

    /// `getOnPos(offset)`.
    fn on_pos(&self, offset: f32) -> [i32; 3] {
        let p = &self.p;
        if let Some(s) = p.supporting_block {
            if !(offset > 1.0e-5) {
                return s;
            }
            let d = R::block(self.w.state(s[0], s[1], s[2]));
            let keep = (f64::from(offset) <= 0.5 && d.flags & bf::FENCES != 0)
                || d.flags & bf::WALLS != 0
                || d.flags & bf::FENCE_GATE != 0;
            if keep {
                return s;
            }
            return [s[0], floor(p.pos[1] - f64::from(offset)), s[2]];
        }
        [
            floor(p.pos[0]),
            floor(p.pos[1] - f64::from(offset)),
            floor(p.pos[2]),
        ]
    }

    /// `Player.travel`.
    fn player_travel(&mut self, input: V3) {
        if self.p.is_swimming() {
            let look_y = f64::from(-m::sin(f64::from(self.p.pitch * DEG_TO_RAD)));
            let k = if look_y < -0.2 { 0.085 } else { 0.06 };
            let p = &self.p;
            let above = kind(self.w.fluid(
                floor(p.pos[0]),
                floor(p.pos[1] + 1.0 - 0.1),
                floor(p.pos[2]),
            )) != 0;
            if look_y <= 0.0 || p.jumping || above {
                let v = self.vel();
                self.set_vel(v.add(V3::new(0.0, (look_y - v.y) * k, 0.0)));
            }
        }
        if self.p.abilities.flying {
            let y = self.p.vel[1];
            self.travel(input);
            self.p.vel[1] = y * 0.6;
        } else {
            self.travel(input);
        }
    }

    /// `getFlyingSpeed` (Player).
    fn flying_speed(&self) -> f32 {
        let p = &self.p;
        if p.abilities.flying {
            if p.sprinting {
                p.abilities.fly_speed * 2.0
            } else {
                p.abilities.fly_speed
            }
        } else if p.sprinting {
            0.025_999_999
        } else {
            0.02
        }
    }

    fn gravity(&self) -> f64 {
        let g = self.p.attributes.gravity.value();
        if self.p.vel[1] <= 0.0 && self.p.effects.slow_falling {
            java_min(g, 0.01)
        } else {
            g
        }
    }

    /// `LivingEntity.travel`.
    fn travel(&mut self, input: V3) {
        let p = &self.p;
        if (p.touching_water || p.in_lava()) && !p.abilities.flying {
            let falling = p.vel[1] <= 0.0;
            let old_y = p.pos[1];
            let g = self.gravity();
            if self.p.touching_water {
                self.travel_in_water(input, g, falling, old_y);
            } else {
                self.travel_in_lava(input, g, falling, old_y);
            }
        } else {
            self.travel_in_air(input);
        }
    }

    fn travel_in_water(&mut self, input: V3, g: f64, falling: bool, old_y: f64) {
        let p = &self.p;
        let mut slow: f32 = if p.sprinting { 0.9 } else { 0.8 };
        let mut speed: f32 = 0.02;
        let mut ww = p.attributes.water_movement_efficiency.value() as f32;
        if !p.on_ground {
            ww *= 0.5;
        }
        if ww > 0.0 {
            slow += (0.546_000_06 - slow) * ww;
            speed += (p.speed() - speed) * ww;
        }
        if p.effects.dolphins_grace {
            slow = 0.96;
        }
        self.move_relative(speed, input);
        let v = self.vel();
        self.move_self(v);
        let mut v = self.vel();
        if self.p.horizontal_collision && self.on_climbable() {
            v.y = 0.2;
        }
        let v = v.mul(f64::from(slow), f64::from(0.8f32), f64::from(slow));
        let v = self.fluid_falling(g, falling, v);
        self.set_vel(v);
        self.jump_out_of_fluid(old_y);
    }

    fn travel_in_lava(&mut self, input: V3, g: f64, falling: bool, old_y: f64) {
        self.move_relative(0.02, input);
        let v = self.vel();
        self.move_self(v);
        let thr = if self.p.dims.eye_height < 0.4 {
            0.0
        } else {
            0.4
        };
        if self.p.lava_height <= thr {
            let v = self.vel().mul(0.5, f64::from(0.8f32), 0.5);
            let v = self.fluid_falling(g, falling, v);
            self.set_vel(v);
        } else {
            let v = self.vel().scale(0.5);
            self.set_vel(v);
        }
        if g != 0.0 {
            let v = self.vel().add(V3::new(0.0, -g / 4.0, 0.0));
            self.set_vel(v);
        }
        self.jump_out_of_fluid(old_y);
    }

    /// `getFluidFallingAdjustedMovement`.
    fn fluid_falling(&self, g: f64, falling: bool, v: V3) -> V3 {
        if g != 0.0 && !self.p.sprinting {
            let y = if falling && (v.y - 0.005).abs() >= 0.003 && (v.y - g / 16.0).abs() < 0.003 {
                -0.003
            } else {
                v.y - g / 16.0
            };
            V3::new(v.x, y, v.z)
        } else {
            v
        }
    }

    fn jump_out_of_fluid(&mut self, old_y: f64) {
        let v = self.vel();
        if !self.p.horizontal_collision {
            return;
        }
        let dy = v.y + f64::from(0.6f32) - self.p.pos[1] + old_y;
        let probe = mv(&self.p.bb, v.x, dy, v.z);
        if self.w.no_collision(self.p.pos, &probe, &self.ctx())
            && !self.w.contains_any_liquid(&probe)
        {
            self.set_vel(V3::new(v.x, f64::from(0.3f32), v.z));
        }
    }

    fn travel_in_air(&mut self, input: V3) {
        let below = self.on_pos(0.500_001);
        let bf_ = if self.p.on_ground {
            R::block(self.w.state(below[0], below[1], below[2])).friction
        } else {
            1.0
        };
        let friction = bf_ * 0.91;
        // handleRelativeFrictionAndCalculateMovement
        let speed = if self.p.on_ground {
            self.p.speed() * (0.216_000_02 / (bf_ * bf_ * bf_))
        } else {
            self.flying_speed()
        };
        self.move_relative(speed, input);
        let v = self.handle_on_climbable(self.vel());
        self.set_vel(v);
        let v = self.vel();
        self.move_self(v);
        let mut mvm = self.vel();
        if (self.p.horizontal_collision || self.p.jumping)
            && (self.on_climbable() || self.p.was_in_powder_snow && self.p.leather_boots)
        {
            mvm.y = 0.2;
        }
        let mut y = mvm.y;
        if let Some(amp) = self.p.effects.levitation {
            y += (0.05 * f64::from(amp + 1) - mvm.y) * 0.2;
        } else {
            // The client always has the chunk (ClientLevel.hasChunk is true).
            y -= self.gravity();
        }
        let f = f64::from(friction);
        self.set_vel(V3::new(mvm.x * f, y * f64::from(0.98f32), mvm.z * f));
    }

    fn on_climbable(&self) -> bool {
        climbable::<R, W>(self.w, self.p)
    }

    /// `handleOnClimbable`.
    fn handle_on_climbable(&mut self, d: V3) -> V3 {
        if !self.on_climbable() {
            return d;
        }
        self.p.fall_distance = 0.0;
        let l = f64::from(0.15f32);
        let xd = clamp(d.x, -l, l);
        let zd = clamp(d.z, -l, l);
        let mut yd = java_max(d.y, -l);
        let p = &self.p;
        let in_scaffolding = R::phys(self.w.state(
            floor(p.pos[0]),
            floor(p.pos[1]),
            floor(p.pos[2]),
        ))
        .kind()
            == BlockKind::Scaffolding as u8;
        if yd < 0.0 && !in_scaffolding && !p.abilities.flying && p.shift() {
            yd = 0.0;
        }
        V3::new(xd, yd, zd)
    }

    /// `moveRelative(speed, input)`.
    fn move_relative(&mut self, speed: f32, input: V3) {
        let len = input.length_sqr();
        if len < 1.0e-7 {
            return;
        }
        let v = if len > 1.0 { input.normalize() } else { input }.scale(f64::from(speed));
        let r = f64::from(self.p.yaw * DEG_TO_RAD);
        let (s, c) = (f64::from(m::sin(r)), f64::from(m::cos(r)));
        let add = V3::new(v.x * c - v.z * s, v.y, v.z * c + v.x * s);
        let nv = self.vel().add(add);
        self.set_vel(nv);
    }

    /// `LocalPlayer.move`: the entity move, then auto-jump.
    fn move_self(&mut self, delta: V3) {
        let (x, z) = (self.p.pos[0], self.p.pos[2]);
        self.entity_move(delta);
        let dx = (self.p.pos[0] - x) as f32;
        let dz = (self.p.pos[2] - z) as f32;
        self.update_auto_jump(dx, dz);
    }

    fn set_on_ground(&mut self, on_ground: bool, movement: Option<V3>) {
        self.p.on_ground = on_ground;
        self.check_supporting_block(on_ground, movement);
    }

    /// `checkSupportingBlock`.
    fn check_supporting_block(&mut self, on_ground: bool, movement: Option<V3>) {
        if !on_ground {
            self.p.on_ground_no_blocks = false;
            self.p.supporting_block = None;
            return;
        }
        let b = self.p.bb;
        let test = bx(
            b.min_x,
            b.min_y - 1.0e-6,
            b.min_z,
            b.max_x,
            b.min_y,
            b.max_z,
        );
        let ctx = self.ctx();
        let mut s = self.w.supporting_block(self.p.pos, &test, &ctx);
        if s.is_some() || self.p.on_ground_no_blocks {
            self.p.supporting_block = s;
        } else if let Some(m) = movement {
            let t2 = mv(&test, -m.x, 0.0, -m.z);
            s = self.w.supporting_block(self.p.pos, &t2, &ctx);
            self.p.supporting_block = s;
        }
        self.p.on_ground_no_blocks = s.is_none();
    }

    /// `Entity.move(SELF, delta)`.
    #[allow(clippy::too_many_lines)]
    fn entity_move(&mut self, delta: V3) {
        if self.p.no_physics {
            let p = self.pos();
            let n = p.add(delta);
            self.p.set_pos(n.x, n.y, n.z);
            let p = &mut *self.p;
            p.horizontal_collision = false;
            p.vertical_collision = false;
            p.vertical_collision_below = false;
            p.minor_horizontal_collision = false;
            return;
        }
        let mut delta = delta;
        let sm = V3::of(self.p.stuck_multiplier);
        if sm.length_sqr() > 1.0e-7 {
            delta = delta.mul(sm.x, sm.y, sm.z);
            self.p.stuck_multiplier = [0.0; 3];
            self.set_vel(V3::ZERO);
        }
        delta = self.back_off_from_edge(delta);
        let ctx = self.ctx();
        let movement = collide::<R, _>(
            self.w,
            self.p.pos,
            &self.p.bb,
            delta,
            self.p.on_ground,
            self.p.step_height(),
            &ctx,
        );
        let len = movement.length_sqr();
        if len > 1.0e-7 || delta.length_sqr() - len < 1.0e-7 {
            if self.p.fall_distance != 0.0 && len >= 1.0 && self.fall_reset_raycast(movement) {
                self.p.fall_distance = 0.0;
            }
            let from = self.pos();
            let to = from.add(movement);
            self.record_movement(Movement {
                from,
                to,
                axis_dependent: Some(delta),
            });
            self.p.set_pos(to.x, to.y, to.z);
        }
        let xc = !approx_equal(delta.x, movement.x);
        let zc = !approx_equal(delta.z, movement.z);
        self.p.horizontal_collision = xc || zc;
        // isLocalInstanceAuthoritative: always for the client player.
        self.p.vertical_collision = delta.y != movement.y;
        self.p.vertical_collision_below = self.p.vertical_collision && delta.y < 0.0;
        let below = self.p.vertical_collision_below;
        self.set_on_ground(below, Some(movement));
        self.p.minor_horizontal_collision =
            self.p.horizontal_collision && self.minor_collision(movement);
        let effect_pos = self.on_pos(0.2);
        let effect = self.w.state(effect_pos[0], effect_pos[1], effect_pos[2]);
        self.check_fall_damage(movement.y);
        if self.p.horizontal_collision {
            let v = self.vel();
            self.set_vel(V3::new(
                if xc { 0.0 } else { v.x },
                v.y,
                if zc { 0.0 } else { v.z },
            ));
        }
        if delta.y != movement.y {
            self.after_fall_on(effect);
        }
        let k = f64::from(self.block_speed_factor());
        let v = self.vel();
        self.set_vel(V3::new(v.x * k, v.y, v.z * k));
    }

    /// `updateEntityMovementAfterFallOn` of the block landed on.
    fn after_fall_on(&mut self, s: R::State) {
        let d = R::block(s);
        let kind_ = R::phys(s).kind();
        let v = self.vel();
        if !self.p.shift() && kind_ == BlockKind::SlimeBlock as u8 {
            if v.y < 0.0 {
                self.set_vel(V3::new(v.x, -v.y * 1.0, v.z));
            }
        } else if !self.p.shift() && d.flags & bf::BED != 0 {
            if v.y < 0.0 {
                self.set_vel(V3::new(v.x, -v.y * f64::from(0.66f32) * 1.0, v.z));
            }
        } else {
            self.set_vel(V3::new(v.x, v.y * 0.0, v.z));
        }
    }

    /// `LivingEntity.checkFallDamage` + `Entity.checkFallDamage` (client: no damage).
    fn check_fall_damage(&mut self, ya: f64) {
        if !self.p.touching_water {
            self.update_fluid_interaction();
        }
        if !self.p.touching_water && ya < 0.0 {
            self.p.fall_distance -= f64::from(ya as f32);
        }
        if self.p.on_ground {
            self.p.fall_distance = 0.0;
        }
    }

    /// `LocalPlayer.isHorizontalCollisionMinor`.
    fn minor_collision(&self, mvm: V3) -> bool {
        let p = &self.p;
        let r = f64::from(p.yaw * DEG_TO_RAD);
        let (s, c) = (f64::from(m::sin(r)), f64::from(m::cos(r)));
        let gx = f64::from(p.xxa) * c - f64::from(p.zza) * s;
        let gz = f64::from(p.zza) * c + f64::from(p.xxa) * s;
        let a = gx * gx + gz * gz;
        let b = mvm.x * mvm.x + mvm.z * mvm.z;
        let tiny = f64::from(1.0e-5f32);
        if !(a < tiny) && !(b < tiny) {
            let dot = gx * mvm.x + gz * mvm.z;
            let angle = m::acos(dot / (a * b).sqrt());
            angle < f64::from(0.139_626_34f32)
        } else {
            false
        }
    }

    /// The fall-distance reset raycast of `Entity.move` for moves of a block or more: a hit on a
    /// `#fall_damage_resetting` block or water along up to 8 blocks of the move.
    fn fall_reset_raycast(&self, movement: V3) -> bool {
        let dist = java_min(movement.length(), 8.0);
        let from = self.pos();
        let to = from.add(movement.normalize().scale(dist));
        if from == to {
            return false;
        }
        let lerp = m::lerp;
        let t = V3::new(
            lerp(-1.0e-7, to.x, from.x),
            lerp(-1.0e-7, to.y, from.y),
            lerp(-1.0e-7, to.z, from.z),
        );
        let f = V3::new(
            lerp(-1.0e-7, from.x, to.x),
            lerp(-1.0e-7, from.y, to.y),
            lerp(-1.0e-7, from.z, to.z),
        );
        let mut c = [floor(f.x), floor(f.y), floor(f.z)];
        if self.clip_hits(c, from, to) {
            return true;
        }
        let d = t.sub(f);
        let s = [m::sign(d.x), m::sign(d.y), m::sign(d.z)];
        let td = [0, 1, 2].map(|a| {
            if s[a] == 0 {
                f64::MAX
            } else {
                f64::from(s[a]) / d.get(a)
            }
        });
        let fa = f.arr();
        let mut tt = [0, 1, 2].map(|a| {
            td[a]
                * if s[a] > 0 {
                    1.0 - m::frac(fa[a])
                } else {
                    m::frac(fa[a])
                }
        });
        while tt[0] <= 1.0 || tt[1] <= 1.0 || tt[2] <= 1.0 {
            if tt[0] < tt[1] {
                if tt[0] < tt[2] {
                    c[0] += s[0];
                    tt[0] += td[0];
                } else {
                    c[2] += s[2];
                    tt[2] += td[2];
                }
            } else if tt[1] < tt[2] {
                c[1] += s[1];
                tt[1] += td[1];
            } else {
                c[2] += s[2];
                tt[2] += td[2];
            }
            if self.clip_hits(c, from, to) {
                return true;
            }
        }
        false
    }

    /// One block of the raycast: the block's full cube when it resets fall damage, the water's
    /// box when the block holds water.
    fn clip_hits(&self, c: [i32; 3], from: V3, to: V3) -> bool {
        let s = self.w.state(c[0], c[1], c[2]);
        let p = R::phys(s);
        let cell = |h: f64| {
            bx(
                f64::from(c[0]),
                f64::from(c[1]),
                f64::from(c[2]),
                f64::from(c[0]) + 1.0,
                f64::from(c[1]) + h,
                f64::from(c[2]) + 1.0,
            )
        };
        let shape_clip = |b: &Aabb| {
            let diff = to.sub(from);
            if diff.length_sqr() < 1.0e-7 {
                return false;
            }
            let tp = from.add(diff.scale(0.001));
            super::math::contains(b, tp) || m::clip_box(b, from, to).is_some()
        };
        if R::block(s).flags & bf::FALL_DAMAGE_RESETTING != 0 && shape_clip(&cell(1.0)) {
            return true;
        }
        let f = p.fluid();
        if kind(f) == 1 {
            let h = f64::from(self.w.fluid_height(c[0], c[1], c[2], f));
            if shape_clip(&cell(h)) {
                return true;
            }
        }
        false
    }

    fn record_movement(&mut self, mvt: Movement) {
        if let Some(slot) = self.moves.iter_mut().find(|m| m.is_none()) {
            *slot = Some(mvt);
        } else {
            // Vanilla merges the oldest two above 100 entries; a tick records one or two.
            self.moves[3] = Some(mvt);
        }
    }

    /// `Player.maybeBackOffFromEdge`.
    fn back_off_from_edge(&self, d: V3) -> V3 {
        let p = &self.p;
        let step = p.step_height();
        if p.abilities.flying || d.y > 0.0 || !p.shift() || !self.above_ground(step) {
            return d;
        }
        let (mut dx, mut dz) = (d.x, d.z);
        let sx = sign_f(dx) * 0.05;
        let sz = sign_f(dz) * 0.05;
        let h = f64::from(step);
        while dx != 0.0 && self.can_fall_at_least(dx, 0.0, h) {
            if dx.abs() <= 0.05 {
                dx = 0.0;
                break;
            }
            dx -= sx;
        }
        while dz != 0.0 && self.can_fall_at_least(0.0, dz, h) {
            if dz.abs() <= 0.05 {
                dz = 0.0;
                break;
            }
            dz -= sz;
        }
        while dx != 0.0 && dz != 0.0 && self.can_fall_at_least(dx, dz, h) {
            if dx.abs() <= 0.05 {
                dx = 0.0;
            } else {
                dx -= sx;
            }
            if dz.abs() <= 0.05 {
                dz = 0.0;
            } else {
                dz -= sz;
            }
        }
        V3::new(dx, d.y, dz)
    }

    fn above_ground(&self, step: f32) -> bool {
        let p = &self.p;
        p.on_ground
            || p.fall_distance < f64::from(step)
                && !self.can_fall_at_least(0.0, 0.0, f64::from(step) - p.fall_distance)
    }

    fn can_fall_at_least(&self, dx: f64, dz: f64, h: f64) -> bool {
        let b = self.p.bb;
        let q = bx(
            b.min_x + 1.0e-7 + dx,
            b.min_y - h - 1.0e-7,
            b.min_z + 1.0e-7 + dz,
            b.max_x - 1.0e-7 + dx,
            b.min_y,
            b.max_z - 1.0e-7 + dz,
        );
        self.w.no_collision(self.p.pos, &q, &self.ctx())
    }

    /// `applyEffectsFromBlocks()`: step-on effects, then the inside-block sweep.
    fn apply_effects_from_blocks(&mut self) {
        let mut list = [None; 5];
        let mut n = 0;
        for mvt in self.moves.iter().flatten() {
            list[n] = Some(*mvt);
            n += 1;
        }
        self.moves = [None; 4];
        let pos = self.pos();
        if n == 0 {
            list[0] = Some(Movement {
                from: self.old_pos,
                to: pos,
                axis_dependent: None,
            });
            n = 1;
        } else if list[n - 1].is_some_and(|l| l.to.dist_sqr(pos) > f64::from(9.999_999_4e-11f32)) {
            let last = list[n - 1].map(|l| l.to).unwrap_or(pos);
            list[n] = Some(Movement {
                from: last,
                to: pos,
                axis_dependent: None,
            });
            n += 1;
        }
        if self.p.no_physics {
            return;
        }
        if self.p.on_ground {
            let ep = self.on_pos(0.2);
            let s = self.w.state(ep[0], ep[1], ep[2]);
            if R::phys(s).kind() == BlockKind::SlimeBlock as u8 {
                let v = self.vel();
                let ad = v.y.abs();
                if ad < 0.1 && !self.p.shift() {
                    let k = 0.4 + ad * 0.2;
                    self.set_vel(V3::new(v.x * k, v.y, v.z * k));
                }
            }
        }
        let mut visited = Visited::default();
        for mvt in list[..n].iter().flatten() {
            let mut budget = 16i32;
            let delta = mvt.to.sub(mvt.from);
            match mvt.axis_dependent {
                Some(orig) if delta.length_sqr() > 0.0 => {
                    let mut pos = mvt.from;
                    for axis in axis_order(orig) {
                        let am = delta.get(axis);
                        if am != 0.0 {
                            let to = pos.with(axis, pos.get(axis) + am);
                            budget -= self.check_inside(pos, to, &mut visited, budget);
                            pos = to;
                        }
                    }
                }
                _ => budget -= self.check_inside(mvt.from, mvt.to, &mut visited, 16),
            }
            if budget <= 0 {
                self.check_inside(mvt.to, mvt.to, &mut visited, 1);
            }
        }
        visited.clear();
    }

    /// One `checkInsideBlocks(from, to, collector, visited, maxIterations)`; returns
    /// `iterations + 1`.
    fn check_inside(&mut self, from: V3, to: V3, visited: &mut Visited, max: i32) -> i32 {
        let d = self.p.dims;
        let at_target = deflate(
            &make_box(d.width, d.height, to.x, to.y, to.z),
            f64::from(1.0e-5f32),
        );
        let far = from.dist_sqr(to) > 0.999_990_000_000_252_6 * 0.999_990_000_000_252_6;
        let mut last_iter = 0;
        for_each_block_between(from, to, &at_target, |pos, it| {
            if it >= max {
                return false;
            }
            last_iter = it;
            let s = self.w.state(pos[0], pos[1], pos[2]);
            let ph = R::phys(s);
            if ph.flags() & sf::AIR != 0 {
                return true;
            }
            // Only blocks with a movement effect are acted on (their "inside" test: the full block
            // for all but powder snow, whose shape is swept); every other block just counts.
            let k = ph.kind();
            let effect_kind = k == BlockKind::Cobweb as u8
                || k == BlockKind::SweetBerryBush as u8
                || k == BlockKind::PowderSnow as u8
                || k == BlockKind::HoneyBlock as u8
                || k == BlockKind::BubbleColumn as u8;
            if !effect_kind
                || (k == BlockKind::PowderSnow as u8 && !self.inside_powder_snow(pos, s, from, to))
                || !visited.add(pos)
            {
                return true;
            }
            let precise = far
                || intersects(
                    &at_target,
                    f64::from(pos[0]),
                    f64::from(pos[1]),
                    f64::from(pos[2]),
                    f64::from(pos[0]) + 1.0,
                    f64::from(pos[1]) + 1.0,
                    f64::from(pos[2]) + 1.0,
                );
            self.entity_inside(pos, s, precise);
            true
        });
        last_iter + 1
    }

    /// `getEntityInsideCollisionShape` of powder snow (its collision shape, or the full block when
    /// that is empty) swept from `from` to `to`.
    fn inside_powder_snow(&self, pos: [i32; 3], s: R::State, from: V3, to: V3) -> bool {
        let ctx = self.ctx();
        match self.w.shape_of_state(s, pos[0], pos[1], pos[2], Some(&ctx)) {
            None | Some(Placed::Cube(_)) => true,
            Some(sh @ Placed::Shape(..)) => {
                let d = self.p.dims;
                let at_from = make_box(d.width, d.height, from.x, from.y, from.z);
                let mut boxes = [Aabb::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0); 8];
                let mut n = 0;
                sh.for_boxes(|b| {
                    if n < boxes.len() {
                        boxes[n] = b;
                        n += 1;
                    }
                    true
                });
                m::collided_along(&at_from, to.sub(from), boxes[..n].iter().copied())
            }
            Some(Placed::Border) => false,
        }
    }

    /// `entityInside` of the movement-relevant blocks.
    fn entity_inside(&mut self, pos: [i32; 3], s: R::State, precise: bool) {
        let ph = R::phys(s);
        let k = ph.kind();
        // Player.makeStuckInBlock: flying players are not stuck.
        let stuck = !self.p.abilities.flying;
        if k == BlockKind::Cobweb as u8 {
            if !stuck {
                return;
            }
            self.p.fall_distance = 0.0;
            self.p.stuck_multiplier = if self.p.effects.weaving {
                [0.5, 0.25, 0.5]
            } else {
                [0.25, f64::from(0.05f32), 0.25]
            };
        } else if k == BlockKind::SweetBerryBush as u8 {
            if !stuck {
                return;
            }
            self.p.fall_distance = 0.0;
            self.p.stuck_multiplier = [f64::from(0.8f32), 0.75, f64::from(0.8f32)];
        } else if k == BlockKind::PowderSnow as u8 {
            let p = &self.p;
            let in_block = R::phys(
                self.w
                    .state(floor(p.pos[0]), floor(p.pos[1]), floor(p.pos[2])),
            )
            .kind()
                == BlockKind::PowderSnow as u8;
            if in_block && stuck {
                self.p.fall_distance = 0.0;
                self.p.stuck_multiplier = [f64::from(0.9f32), 1.5, f64::from(0.9f32)];
            }
            // FREEZE (deferred in vanilla; nothing reads it during the sweep).
            self.p.in_powder_snow = true;
        } else if k == BlockKind::HoneyBlock as u8 {
            if self.honey_sliding(pos) {
                let v = self.vel();
                let old = v.y / f64::from(0.98f32) + 0.08;
                let new_y = (-0.05 - 0.08) * f64::from(0.98f32);
                if old < -0.13 {
                    let r = -0.05 / old;
                    self.set_vel(V3::new(v.x * r, new_y, v.z * r));
                } else {
                    self.set_vel(V3::new(v.x, new_y, v.z));
                }
                self.p.fall_distance = 0.0;
            }
        } else if k == BlockKind::BubbleColumn as u8 && precise {
            let above = R::phys(self.w.state(pos[0], pos[1] + 1, pos[2]));
            let nothing_above = above.flags() & sf::EMPTY != 0 && above.flags() & sf::CONTEXT == 0
                || above.flags() & sf::CONTEXT != 0 && above.kind() == BlockKind::PowderSnow as u8;
            let nothing_above = nothing_above && kind(above.fluid()) == 0;
            let drag = ph.aux() & 1 != 0;
            if self.p.abilities.flying {
                return;
            }
            let v = self.vel();
            if nothing_above {
                let y = if drag {
                    java_max(-0.9, v.y - 0.03)
                } else {
                    java_min(1.8, v.y + 0.1)
                };
                self.set_vel(V3::new(v.x, y, v.z));
            } else {
                let y = if drag {
                    java_max(-0.3, v.y - 0.03)
                } else {
                    java_min(0.7, v.y + 0.06)
                };
                self.set_vel(V3::new(v.x, y, v.z));
                self.p.fall_distance = 0.0;
            }
        }
    }

    /// `HoneyBlock.isSlidingDown`.
    fn honey_sliding(&self, pos: [i32; 3]) -> bool {
        let p = &self.p;
        if p.on_ground {
            return false;
        }
        if p.pos[1] > f64::from(pos[1]) + 0.9375 - 1.0e-7 {
            return false;
        }
        if p.vel[1] / f64::from(0.98f32) + 0.08 >= -0.08 {
            return false;
        }
        let dx = (f64::from(pos[0]) + 0.5 - p.pos[0]).abs();
        let dz = (f64::from(pos[2]) + 0.5 - p.pos[2]).abs();
        let o = 0.4375 + f64::from(p.dims.width / 2.0);
        dx + 1.0e-7 > o || dz + 1.0e-7 > o
    }

    /// `LocalPlayer.updateAutoJump(xa, za)`.
    #[allow(clippy::too_many_lines)]
    fn update_auto_jump(&mut self, xa: f32, za: f32) {
        let p = &*self.p;
        let moving = p.input.move_vector[0] * p.input.move_vector[0]
            + p.input.move_vector[1] * p.input.move_vector[1]
            > 0.0;
        if !(p.auto_jump_enabled
            && p.auto_jump_time <= 0
            && p.on_ground
            && !p.shift()
            && moving
            && f64::from(self.block_jump_factor()) >= 1.0)
        {
            return;
        }
        let begin = self.pos();
        let end0 = begin.add(V3::new(f64::from(xa), 0.0, f64::from(za)));
        let mut diff = V3::new(f64::from(xa), 0.0, f64::from(za));
        let speed = p.speed();
        let mut dsq = diff.length_sqr() as f32;
        if dsq <= 0.001 {
            let mvv = p.input.move_vector;
            let ix = speed * mvv[0];
            let iz = speed * mvv[1];
            let r = f64::from(p.yaw * DEG_TO_RAD);
            let (s, c) = (m::sin(r), m::cos(r));
            diff = V3::new(
                f64::from(ix * c - iz * s),
                diff.y,
                f64::from(iz * c + ix * s),
            );
            dsq = diff.length_sqr() as f32;
            if dsq <= 0.001 {
                return;
            }
        }
        let inv = 1.0 / sqrt_f(dsq);
        let dir = diff.scale(f64::from(inv));
        let facing = direction_from_rotation(p.pitch, p.yaw);
        let dot = (facing.x * dir.x + facing.z * dir.z) as f32;
        if dot < -0.15 {
            return;
        }
        let ctx = self.ctx();
        let shape_empty = |x: i32, y: i32, z: i32| {
            self.w
                .shape_of_state(self.w.state(x, y, z), x, y, z, Some(&ctx))
                .is_none_or(|s| s.is_empty())
        };
        let (cx, mut cy, cz) = (floor(p.pos[0]), floor(p.bb.max_y), floor(p.pos[2]));
        if !shape_empty(cx, cy, cz) {
            return;
        }
        cy += 1;
        if !shape_empty(cx, cy, cz) {
            return;
        }
        let mut jump_h = 1.2f32;
        if let Some(a) = p.effects.jump_boost {
            jump_h += (a + 1) as f32 * 0.75;
        }
        let look = (speed * 7.0).max(1.0 / inv);
        let seg_end = end0.add(dir.scale(f64::from(look)));
        let (w, h) = (p.dims.width, p.dims.height);
        let raw = bx(
            begin.x,
            begin.y,
            begin.z,
            seg_end.x,
            seg_end.y + f64::from(h),
            seg_end.z,
        );
        let test = super::math::inflate(&raw, f64::from(w), 0.0, f64::from(w));
        let lift = f64::from(0.51f32);
        let sb = begin.add(V3::new(0.0, lift, 0.0));
        let se = seg_end.add(V3::new(0.0, lift, 0.0));
        let right = V3::new(
            dir.y * 0.0 - dir.z * 1.0,
            dir.z * 0.0 - dir.x * 0.0,
            dir.x * 1.0 - dir.y * 0.0,
        );
        let ro = right.scale(f64::from(w * 0.5));
        let (l0, l1) = (sb.sub(ro), se.sub(ro));
        let (r0, r1) = (sb.add(ro), se.add(ro));
        let seg = |b: &Aabb, u: V3, v: V3| {
            intersects(
                b,
                java_min(u.x, v.x),
                java_min(u.y, v.y),
                java_min(u.z, v.z),
                java_max(u.x, v.x),
                java_max(u.y, v.y),
                java_max(u.z, v.z),
            )
        };
        // Float.MIN_VALUE marks "nothing hit".
        let mut obstacle = f32::from_bits(1);
        let mut hit: Option<Aabb> = None;
        self.w
            .each_collision(&test, Some(&ctx), false, |_, _, _, s| {
                let mut go = true;
                s.for_boxes(|b| {
                    if seg(&b, l0, l1) || seg(&b, r0, r1) {
                        hit = Some(b);
                        go = false;
                        return false;
                    }
                    true
                });
                go
            });
        if let Some(b) = hit {
            obstacle = b.max_y as f32;
            let c = super::math::center(&b);
            let (ox, oy, oz) = (floor(c.x), floor(c.y), floor(c.z));
            let mut steps = 1;
            while (steps as f32) < jump_h {
                let ay = oy + steps;
                let st = self.w.state(ox, ay, oz);
                if let Some(my) = self
                    .w
                    .shape_of_state(st, ox, ay, oz, Some(&ctx))
                    .and_then(|sh| sh.local_max_y())
                {
                    obstacle = my as f32 + ay as f32;
                    if f64::from(obstacle) - p.pos[1] > f64::from(jump_h) {
                        return;
                    }
                }
                if steps > 1 {
                    cy += 1;
                    if !shape_empty(cx, cy, cz) {
                        return;
                    }
                }
                steps += 1;
            }
        }
        if obstacle != f32::from_bits(1) {
            let yd = (f64::from(obstacle) - p.pos[1]) as f32;
            if !(yd <= 0.5) && !(yd > jump_h) {
                self.p.auto_jump_time = 1;
            }
        }
    }

    /// `refreshDimensions` (pose / scale change): new dimensions, box rebuilt from the position.
    fn refresh_dimensions(&mut self) {
        let d = self.p.pose.scaled(self.p.scale());
        self.p.dims = d;
        let [x, y, z] = self.p.pos;
        self.p.set_pos(x, y, z);
    }

    /// `Player.updatePlayerPose`.
    fn update_pose(&mut self) {
        let p = &self.p;
        let desired = if p.is_swimming() {
            Pose::Swimming
        } else if p.shift() && !p.abilities.flying {
            Pose::Crouching
        } else {
            Pose::Standing
        };
        // Vanilla asks "does the swimming box fit", then the desired pose, then crouching. At one
        // scale the boxes are nested (same feet and width, heights 0.6 < 1.5 < 1.8), so a fitting
        // standing box answers all of them: the common case costs one query instead of two.
        let actual = if self.fits(Pose::Standing) {
            desired
        } else if !self.fits(Pose::Swimming) {
            return;
        } else if self.p.spectator || desired == Pose::Swimming || self.fits(desired) {
            desired
        } else if self.fits(Pose::Crouching) {
            Pose::Crouching
        } else {
            Pose::Swimming
        };
        if actual != self.p.pose {
            self.p.pose = actual;
            self.refresh_dimensions();
        }
    }

    /// `LocalPlayer.sendPosition` (with `sendIsSprintingIfNeeded`).
    fn send_position(&mut self, out: &mut Packets) {
        let p = &mut *self.p;
        if p.sprinting != p.sent.sprinting {
            out.push(if p.sprinting {
                Packet::StartSprinting
            } else {
                Packet::StopSprinting
            });
            p.sent.sprinting = p.sprinting;
        }
        let [x, y, z] = p.pos;
        let dx = x - p.sent.pos[0];
        let dy = y - p.sent.pos[1];
        let dz = z - p.sent.pos[2];
        let dyaw = f64::from(p.yaw) - f64::from(p.sent.yaw);
        let dpitch = f64::from(p.pitch) - f64::from(p.sent.pitch);
        p.sent.position_reminder += 1;
        let moved = dx * dx + dy * dy + dz * dz > R::POSITION_THRESHOLD_SQ
            || p.sent.position_reminder >= R::POSITION_REMINDER;
        let rot = dyaw != 0.0 || dpitch != 0.0;
        let (og, hc) = (p.on_ground, p.horizontal_collision);
        if moved || rot || p.sent.on_ground != og || p.sent.horizontal_collision != hc {
            out.push(Packet::Move {
                pos: moved.then_some(p.pos),
                look: rot.then_some((p.yaw, p.pitch)),
                on_ground: og,
                horizontal_collision: hc,
            });
        }
        if moved {
            p.sent.pos = p.pos;
            p.sent.position_reminder = 0;
        }
        if rot {
            p.sent.yaw = p.yaw;
            p.sent.pitch = p.pitch;
        }
        p.sent.on_ground = og;
        p.sent.horizontal_collision = hc;
        p.auto_jump_enabled = p.options.auto_jump;
    }
}

fn sign_f(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        v
    }
}

/// `Vec3.directionFromRotation(xRot, yRot)` (`getForward`).
fn direction_from_rotation(x_rot: f32, y_rot: f32) -> V3 {
    let pi = std::f32::consts::PI;
    let yc = m::cos(f64::from(-y_rot * DEG_TO_RAD - pi));
    let ys = m::sin(f64::from(-y_rot * DEG_TO_RAD - pi));
    let xc = -m::cos(f64::from(-x_rot * DEG_TO_RAD));
    let xs = m::sin(f64::from(-x_rot * DEG_TO_RAD));
    V3::new(f64::from(ys * xc), f64::from(xs), f64::from(yc * xc))
}

fn read_keys(k: &Keys) -> ClientInput {
    let imp = |a: bool, b: bool| {
        if a == b {
            0.0f32
        } else if a {
            1.0
        } else {
            -1.0
        }
    };
    let (x, y) = (imp(k.left, k.right), imp(k.forward, k.back));
    let d = sqrt_f(x * x + y * y);
    let mv = if d < 1.0e-4 {
        [0.0, 0.0]
    } else {
        [x / d, y / d]
    };
    ClientInput {
        keys: *k,
        move_vector: mv,
    }
}
