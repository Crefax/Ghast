//! 1.16.5 movement state beyond [`PlayerState`](super::PlayerState), kept apart so the 0.4.0
//! types stay as they were: the level's dimension type and riptide.
//!
//! [`tick_ext`](super::tick_ext) runs the same tick as [`tick_in`](super::tick_in) with this
//! state; with a default [`PlayerExt`] the two are identical.
//!
//! # The dimension type, as the 1.16.5 client does it
//!
//! The client world is built with the dimension type of the join packet (`dimension`, the whole
//! dimension type compound), and a respawn packet replaces the world (and its dimension type)
//! only when the world name changes. Of its fields only `ultrawarm` changes the player's
//! movement: `Entity.updateWaterState` pushes with lava's flow at 0.007 instead of
//! 0.0023333333333333335 (the nether). Set [`PlayerExt::ultrawarm`] from it.
//!
//! # Riptide, as the 1.16.5 client does it
//!
//! * **Use.** `TridentItem.use` refuses a riptide trident unless the player is in water or rain
//!   ([`in_water_or_rain`](super::in_water_or_rain)) or the next point of damage would break
//!   it; a held use key retries every 4 ticks (`MinecraftClient.itemUseCooldown`).
//! * **Launch.** Releasing the use key after at least 10 player ticks of use, still in water or
//!   rain, calls [`riptide`](super::riptide) on the client: a push along the look, the 20-tick
//!   counter ([`PlayerExt::riptide_ticks`]) and, on the ground, a 1.2-block move up. No riding or
//!   durability check at the release.
//! * **Spin.** After each tick's travel the counter goes down; a living entity in the swept box
//!   ends it and reverses the motion (×−0.2); with no entity in that box a horizontal collision
//!   ends it (`LivingEntity.tickRiptide`; pass the other entities as
//!   [`Surroundings::others`](super::Surroundings)). The spin pose follows the server's living
//!   entity flag 4 ([`PlayerExt::apply_living_flags`]), not the counter.
/// Movement state beyond [`PlayerState`](super::PlayerState). Default: the overworld, no spin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlayerExt {
    /// The level's dimension type is `ultrawarm` (the nether): lava pushes harder.
    pub ultrawarm: bool,
    /// Living entity flag 4, `isUsingRiptide()`: the spin pose (vanilla `SPIN_ATTACK`, held as [`Pose::FallFlying`](super::Pose), the same box).
    /// Set and cleared only by the server's entity data ([`PlayerExt::apply_living_flags`]).
    pub spin_attack: bool,
    /// The client's riptide spin counter (`riptideTicks`): 20 after a launch
    /// ([`riptide`](super::riptide)), counted down each tick, ended by a hit or a horizontal
    /// collision.
    pub riptide_ticks: i32,
}

impl PlayerExt {
    /// The default state: the overworld, nothing else.
    #[must_use]
    pub fn new() -> PlayerExt {
        PlayerExt::default()
    }

    /// The server's entity data for the player's living entity flags byte (index 7): bit 4 is
    /// the spin. Bits 1 and 2 (the item use the server sees) are the caller's: the client starts
    /// using the held item when bit 1 arrives set and it is not using one, and stops when it
    /// arrives clear while it is (`ClientPlayerEntity.onTrackedDataSet`).
    pub fn apply_living_flags(&mut self, flags: u8) {
        self.spin_attack = flags & 4 != 0;
    }

    /// `true` when the state changes nothing: [`tick_ext`](super::tick_ext) is then the same
    /// tick as [`tick_in`](super::tick_in).
    #[must_use]
    pub fn is_inert(&self) -> bool {
        *self == PlayerExt::default()
    }
}
