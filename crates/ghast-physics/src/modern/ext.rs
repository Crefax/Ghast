//! Player state for the movement mechanics added after 0.4.0, kept next to [`PlayerState`] so the
//! 0.4.0 types stay as they were: gliding (elytra) with firework rockets, and riptide.
//!
//! [`tick_ext`](super::tick_ext) runs the same tick as [`tick`](super::tick) with this state; with a
//! default [`PlayerExt`] the two are identical.
//!
//! # Gliding (elytra), as the 26.1 client does it
//!
//! * **Start.** Pressing jump (not held from the previous tick, not the tick creative flight was
//!   toggled, not on a climbable) starts gliding when the player is not already gliding, not
//!   flying, not on the ground, not riding, has no levitation, is not in water and
//!   [`PlayerExt::glider`] is set. The client sets the flag itself and sends
//!   [`Packet::StartFallFlying`](crate::Packet::StartFallFlying) (`player_command`
//!   `START_FALL_FLYING`) in the tick, before the movement packet.
//! * **Stop.** The client stops by itself only when it glides onto a climbable block that is not
//!   `#can_glide_through` (it then moves like a falling player for that tick). Everything else
//!   (landing, water, a broken or removed elytra) is the server's decision: it clears shared flag 7
//!   in the player's entity data, which the client applies with
//!   [`PlayerExt::apply_shared_flags`]. Until then the client keeps gliding, also on the ground.
//! * **Rockets.** A rocket the server spawns attached to the player (add-entity of a firework rocket
//!   whose entity data names the player as its attached target) is an entity the client ticks
//!   after its player: each tick it boosts a gliding player along the look vector and moves to the
//!   player's hand. Like every entity it only ticks while the chunk column it is in is loaded (a
//!   rocket spawned where the client has no chunk, e.g. a player who outran chunk loading, never
//!   boosts). Register it with [`PlayerExt::attach_rocket`] (its spawn position) and drop it with
//!   [`PlayerExt::remove_rocket`] when the server removes the entity.

//!
//! # Riptide, as the 26.1 client does it
//!
//! * **Launch.** On releasing the use key with a riptide trident (used for at least 10 player
//!   ticks, in water or rain, not riding, not about to break) the client itself calls
//!   [`riptide`](super::riptide): a push along the look vector (after the tick's mouse turning),
//!   a 20-tick counter ([`PlayerExt::auto_spin_ticks`]) and, on the ground, a 1.2-block move up.
//!   Starting the use already needs water or rain ([`in_water_or_rain`](super::in_water_or_rain)).
//! * **Spin.** Each tick the counter goes down; a horizontal collision zeroes it (so does touching
//!   another entity, which is not modelled here). The spin pose (0.6 x 0.6) follows the server's
//!   living entity flag ([`PlayerExt::apply_living_flags`]), not the counter.

use super::math::{self as m, DEG_TO_RAD, V3};
use super::player::{PlayerState, Pose};

/// Movement state beyond [`PlayerState`]. Default: nothing equipped, not gliding.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct PlayerExt {
    /// Shared entity flag 7, `isFallFlying()`: the player glides. Set by the client when it starts
    /// gliding; cleared by the server's entity data (see [`PlayerExt::apply_shared_flags`]) or by a
    /// climbable block.
    pub fall_flying: bool,
    /// A usable glider is equipped (`canGlideUsing` for some equipment slot): an item with the
    /// `minecraft:glider` component in the slot its `minecraft:equippable` component names, whose
    /// next point of damage will not break it (damage below max damage - 1). Kept by the caller
    /// from the player's equipment; it is read only when a glide starts.
    pub glider: bool,
    /// The arm a rocket is held in for its position (`getHandHoldingItemAngle`) is the left one:
    /// the main arm is left (client option), or rockets are only in the off hand of a right-handed
    /// player. Default: right.
    pub rocket_arm_left: bool,
    /// Living entity flag 4, `isAutoSpinAttack()`: the riptide spin (pose `SpinAttack`). Set and
    /// cleared only by the server's entity data ([`PlayerExt::apply_living_flags`]).
    pub spin_attack: bool,
    /// The client's riptide spin counter (`autoSpinAttackTicks`): 20 after a launch
    /// ([`riptide`](super::riptide)), counted down each tick, zeroed by a horizontal collision.
    pub auto_spin_ticks: i32,
    rocket_count: u8,
    rockets: [Rocket; MAX_ROCKETS],
    /// A movement made before the tick (the riptide lift), swept by the tick's inside-block effects.
    pub(super) pending_move: Option<super::inside::Movement>,
}

/// Attached rockets [`PlayerExt`] keeps track of.
pub const MAX_ROCKETS: usize = 16;

/// An attached firework rocket: its entity id and position (the chunk it ticks in; below the
/// world it is discarded).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Rocket {
    id: i32,
    x: f64,
    y: f64,
    z: f64,
}

impl PlayerExt {
    /// Nothing equipped, not gliding.
    #[must_use]
    pub fn new() -> PlayerExt {
        PlayerExt::default()
    }

    /// The bits of the player's shared entity flags byte (entity data index 0) that movement reads:
    /// 3 sprinting, 4 swimming, 7 fall flying.
    #[must_use]
    pub fn shared_flags(&self, p: &PlayerState) -> u8 {
        (u8::from(p.sprinting) << 3)
            | (u8::from(p.swimming) << 4)
            | (u8::from(self.fall_flying) << 7)
    }

    /// `isVisuallyCrawling()`: the swimming pose out of water, or the gliding pose kept after
    /// gliding stopped (until the pose is updated at the end of the tick).
    #[must_use]
    pub fn visually_crawling(&self, p: &PlayerState) -> bool {
        (p.pose == Pose::Swimming || !self.fall_flying && p.pose == Pose::FallFlying)
            && !p.touching_water
    }

    /// A firework rocket attached to the player appeared (entity `id`, spawned at `pos`): it boosts
    /// from the next tick on (the client ticks it after the player in the same tick it was added,
    /// see the module docs). Returns `false` when [`MAX_ROCKETS`] are attached already (vanilla has
    /// no limit; the rocket is then ignored).
    pub fn attach_rocket(&mut self, id: i32, pos: [f64; 3]) -> bool {
        self.remove_rocket(id);
        let n = usize::from(self.rocket_count);
        if n == MAX_ROCKETS {
            return false;
        }
        self.rockets[n] = Rocket {
            id,
            x: pos[0],
            y: pos[1],
            z: pos[2],
        };
        self.rocket_count += 1;
        true
    }

    /// The entity `id` was removed (a no-op for an entity that is not an attached rocket).
    pub fn remove_rocket(&mut self, id: i32) {
        // Keep the order of the others: it is the client's tick order.
        let n = usize::from(self.rocket_count);
        if let Some(i) = self.rockets[..n].iter().position(|r| r.id == id) {
            self.rockets.copy_within(i + 1..n, i);
            self.rocket_count -= 1;
        }
    }

    /// Number of attached rockets.
    #[must_use]
    pub fn rocket_count(&self) -> usize {
        usize::from(self.rocket_count)
    }

    /// Ticks the attached rockets after the player (`FireworkRocketEntity.tick` of each rocket in a
    /// loaded chunk): a rocket more than 64 blocks below the world is discarded (but finishes that
    /// tick); boost while gliding, then the rocket moves to the player's hand.
    pub(super) fn tick_rockets(&mut self, p: &mut PlayerState, loaded: impl Fn(i32, i32) -> bool) {
        let count = usize::from(self.rocket_count);
        if count == 0 {
            return;
        }
        let (yaw, pitch) = (p.yaw, p.pitch);
        let below = f64::from(p.level.min_y - 64);
        let mut discarded = [0i32; MAX_ROCKETS];
        let mut n = 0;
        for r in &mut self.rockets[..count] {
            let cx = m::floor(r.x) >> 4;
            let cz = m::floor(r.z) >> 4;
            if !loaded(cx, cz) {
                continue;
            }
            if r.y < below {
                discarded[n] = r.id;
                n += 1;
            }
            let hand = if self.fall_flying {
                let v = rocket_boost(V3::of(p.vel), yaw, pitch);
                if v.is_finite() {
                    p.vel = v.arr();
                }
                let a = if self.rocket_arm_left { -80.0 } else { 80.0 };
                look_angle(yaw + a, 0.0).scale(0.5)
            } else {
                V3::ZERO
            };
            r.x = p.pos[0] + hand.x;
            r.y = p.pos[1] + hand.y;
            r.z = p.pos[2] + hand.z;
        }
        for &id in &discarded[..n] {
            self.remove_rocket(id);
        }
    }

    /// The bit of the player's living entity flags byte (entity data index 8) that movement reads:
    /// 4, the riptide spin.
    #[must_use]
    pub fn living_flags(&self) -> u8 {
        u8::from(self.spin_attack) << 2
    }

    /// The server's entity data for the player's living entity flags byte (index 8): the riptide
    /// spin (bit value 4) decides the pose. (Bit value 1, "using an item", is the client's own
    /// use state; a server that clears it stops the client's use, which is the caller's.)
    pub fn apply_living_flags(&mut self, flags: u8) {
        self.spin_attack = flags & 4 != 0;
    }

    /// The server's entity data for the player's own shared flags byte (`set_entity_data`, entity
    /// data index 0): sets sprinting (bit 3; the flag only, the sprint speed modifier is not
    /// touched, as in vanilla), swimming (bit 4) and fall flying (bit 7). Other bits are not read by
    /// movement.
    pub fn apply_shared_flags(&mut self, p: &mut PlayerState, flags: u8) {
        p.sprinting = flags & 0x08 != 0;
        p.swimming = flags & 0x10 != 0;
        self.fall_flying = flags & 0x80 != 0;
    }
}

/// `Entity.calculateViewVector(xRot, yRot)` (`getLookAngle`).
pub(super) fn look_angle(yaw: f32, pitch: f32) -> V3 {
    let x_rot = pitch * DEG_TO_RAD;
    let y_rot = -yaw * DEG_TO_RAD;
    let y_cos = m::cos(f64::from(y_rot));
    let y_sin = m::sin(f64::from(y_rot));
    let x_cos = m::cos(f64::from(x_rot));
    let x_sin = m::sin(f64::from(x_rot));
    V3::new(
        f64::from(y_sin * x_cos),
        f64::from(-x_sin),
        f64::from(y_cos * x_cos),
    )
}

/// `LivingEntity.updateFallFlyingMovement`: lift from the pitch, conversion of falling into forward
/// speed and of forward speed into climb, steering towards the look direction, drag.
pub(super) fn glide(mv: V3, yaw: f32, pitch: f32, gravity: f64) -> V3 {
    let look = look_angle(yaw, pitch);
    let lean = pitch * DEG_TO_RAD;
    let look_h = (look.x * look.x + look.z * look.z).sqrt();
    let move_h = (mv.x * mv.x + mv.z * mv.z).sqrt();
    let c = super::jdk_math::cos(lean);
    let lift = c * c;
    let mut v = mv.add(V3::new(0.0, gravity * (-1.0 + lift * 0.75), 0.0));
    if v.y < 0.0 && look_h > 0.0 {
        let k = v.y * -0.1 * lift;
        v = v.add(V3::new(look.x * k / look_h, k, look.z * k / look_h));
    }
    if lean < 0.0 && look_h > 0.0 {
        let k = move_h * f64::from(-m::sin(f64::from(lean))) * 0.04;
        v = v.add(V3::new(-look.x * k / look_h, k * 3.2, -look.z * k / look_h));
    }
    if look_h > 0.0 {
        v = v.add(V3::new(
            (look.x / look_h * move_h - v.x) * 0.1,
            0.0,
            (look.z / look_h * move_h - v.z) * 0.1,
        ));
    }
    v.mul(f64::from(0.99f32), f64::from(0.98f32), f64::from(0.99f32))
}

/// One attached rocket's tick (`FireworkRocketEntity.tick`) on a gliding player.
pub(super) fn rocket_boost(v: V3, yaw: f32, pitch: f32) -> V3 {
    let look = look_angle(yaw, pitch);
    v.add(V3::new(
        look.x * 0.1 + (look.x * 1.5 - v.x) * 0.5,
        look.y * 0.1 + (look.y * 1.5 - v.y) * 0.5,
        look.z * 0.1 + (look.z * 1.5 - v.z) * 0.5,
    ))
}
