//! 1.12.2 movement state beyond [`PlayerState`](super::PlayerState), kept apart so the 0.4.x
//! types stay as they were: gliding (elytra) with firework rockets.
//!
//! [`tick_ext`](super::tick_ext) runs the same tick as [`tick_in`](super::tick_in) with this
//! state; with a default [`PlayerExt`] the two are identical.
//!
//! # Gliding (elytra), as the 1.12.2 client does it
//!
//! * **Request.** A jump press (not held from the previous tick) while not on the ground, falling
//!   (vertical velocity below 0 after the tick's flight toggle), not gliding and not flying, with
//!   [`PlayerExt::elytra`] set, sends
//!   [`Packet::StartFallFlying`](crate::Packet::StartFallFlying) (`entity_action`
//!   `start_fall_flying`) in the tick, before the movement packet. Unlike later versions the
//!   1.12 client does not start gliding itself (and checks neither water nor levitation): the
//!   server does, by setting shared flag 7 in the player's entity data
//!   ([`PlayerExt::apply_shared_flags`]).
//! * **Glide.** While flag 7 is set, out of water and lava, the travel is the glide
//!   (`EntityLivingBase` travel; gravity 0.08, no levitation or slow falling); the box is
//!   0.6 × 0.6 (the size update at the end of the tick, when that box is free). Creative flight
//!   clears the flag after its travel. Everything else (landing, water, a broken elytra) is the
//!   server's: the client glides on, on the ground too, until the server clears flag 7.
//! * **Rockets.** A rocket the server spawns for the player (spawn-object type 76 whose entity
//!   data names the player as its shooter) ticks after the player in the client world's entity
//!   list (it was added later), also over columns the client has not loaded: each tick it boosts
//!   a gliding player along the look ([`rocket_boost`](super::rocket_boost)). The server
//!   removes it when its random life (11 to 22 ticks for a plain rocket) runs out.

/// Movement state beyond [`PlayerState`](super::PlayerState). Default: no elytra, not gliding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlayerExt {
    /// Shared entity flag 7 (`isFallFlying`): the player glides. Set by the server's entity
    /// data ([`PlayerExt::apply_shared_flags`]); cleared by it or by creative flight.
    pub fall_flying: bool,
    /// A usable elytra is in the chest slot (`Items.ELYTRA` with damage below 431). Kept by the
    /// caller from the player's equipment; read only when a jump press asks to glide.
    pub elytra: bool,
}

impl PlayerExt {
    /// No elytra, not gliding.
    #[must_use]
    pub fn new() -> PlayerExt {
        PlayerExt::default()
    }

    /// `true` when the state changes nothing: [`tick_ext`](super::tick_ext) is then the same tick
    /// as [`tick_in`](super::tick_in).
    #[must_use]
    pub fn is_inert(&self) -> bool {
        *self == PlayerExt::default()
    }

    /// The server's entity data for the player's own shared flags byte (index 0): sets sprinting
    /// (bit value 8; the flag only, the sprint speed modifier is not touched, as in vanilla) and
    /// fall flying (128). Other bits are not read by movement.
    pub fn apply_shared_flags(&mut self, p: &mut super::PlayerState, flags: u8) {
        p.sprinting = flags & 0x08 != 0;
        self.fall_flying = flags & 0x80 != 0;
    }

    /// The bits of the player's shared flags byte that movement reads: 8 sprinting, 128 fall
    /// flying.
    #[must_use]
    pub fn shared_flags(&self, p: &super::PlayerState) -> u8 {
        (u8::from(p.sprinting) << 3) | (u8::from(self.fall_flying) << 7)
    }
}
