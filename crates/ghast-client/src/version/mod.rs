//! Per-version glue between the generated packets of `ghast-protocol` and the common layer.
//!
//! [`ClientVersion`] is implemented for the protocol marker types ([`V1_8`], [`V1_16_5`]).
//! The connection and its tick loop are generic over it, so every hot path is monomorphised
//! per version (no trait objects); the bot API above it only sees [`crate::common`] types.

use std::fmt;
use std::str::FromStr;

use ghast_protocol::{DecodeError, EncodeError, FrameEncoder, PacketFilter, RawPacket, Version};

use crate::common::{Inbound, LoginInbound, Outbound, Pose, Teleport};
use crate::interest::Interests;

mod v1_16_5;
mod v1_8;

pub use ghast_protocol::v1_8::V1_8;
pub use ghast_protocol::v1_16_5::V1_16_5;

/// A protocol version the client supports, chosen at run time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GameVersion {
    /// 1.8 – 1.8.9 (protocol 47).
    V1_8_9,
    /// 1.16.4 – 1.16.5 (protocol 754).
    V1_16_5,
}

impl GameVersion {
    /// Every supported version.
    pub const ALL: [GameVersion; 2] = [GameVersion::V1_8_9, GameVersion::V1_16_5];

    /// Protocol number.
    #[must_use]
    pub const fn protocol(self) -> i32 {
        match self {
            Self::V1_8_9 => <V1_8 as Version>::PROTOCOL,
            Self::V1_16_5 => <V1_16_5 as Version>::PROTOCOL,
        }
    }

    /// Display name (`"1.8.9"`, `"1.16.5"`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::V1_8_9 => "1.8.9",
            Self::V1_16_5 => "1.16.5",
        }
    }

    /// Eye height above the feet while standing: `1.62F` widened to double (every supported
    /// version).
    #[must_use]
    pub fn eye_height(self) -> f64 {
        f64::from(1.62_f32)
    }

    /// Eye height while sneaking: 1.54 in 1.8 (`getEyeHeight` − 0.08), 1.27 in 1.16 (the
    /// crouching pose).
    #[must_use]
    pub fn sneak_eye_height(self) -> f64 {
        match self {
            // `1.62F - 0.08F` in float.
            Self::V1_8_9 => f64::from(1.62_f32 - 0.08_f32),
            Self::V1_16_5 => f64::from(1.27_f32),
        }
    }
}

impl fmt::Display for GameVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Error parsing a [`GameVersion`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unsupported Minecraft version `{0}` (supported: 1.8.9, 1.16.5)")]
pub struct UnknownVersion(pub String);

impl FromStr for GameVersion {
    type Err = UnknownVersion;

    /// Accepts `1.8`, `1.8.x`, `47`, `1.16.4`, `1.16.5` and `754`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "1.8" | "1.8.0" | "1.8.1" | "1.8.2" | "1.8.3" | "1.8.4" | "1.8.5" | "1.8.6"
            | "1.8.7" | "1.8.8" | "1.8.9" | "47" => Ok(Self::V1_8_9),
            "1.16.4" | "1.16.5" | "754" => Ok(Self::V1_16_5),
            other => Err(UnknownVersion(other.to_owned())),
        }
    }
}

/// What a protocol version needs to provide to the client.
///
/// Sealed: implemented for [`V1_8`] and [`V1_16_5`] only.
pub trait ClientVersion: Version + sealed::Sealed {
    /// The run-time tag of this version.
    const GAME: GameVersion;
    /// Longest chat line the server accepts, in UTF-16 units (vanilla truncates to it).
    const CHAT_LIMIT: usize;

    /// Queues the handshake packet (next state = login).
    fn write_handshake(enc: &mut FrameEncoder, host: &str, port: u16) -> Result<(), EncodeError>;
    /// Queues the login start packet.
    fn write_login_start(enc: &mut FrameEncoder, username: &str) -> Result<(), EncodeError>;
    /// Queues the encryption response.
    fn write_encryption_response(
        enc: &mut FrameEncoder,
        shared_secret: &[u8],
        verify_token: &[u8],
    ) -> Result<(), EncodeError>;
    /// Queues a "not understood" answer to a login plugin request.
    fn write_plugin_response(enc: &mut FrameEncoder, message_id: i32) -> Result<(), EncodeError>;
    /// Decodes a login-state packet; `Ok(None)` for packets the client ignores.
    fn decode_login(raw: &RawPacket) -> Result<Option<LoginInbound>, DecodeError>;

    /// The play-state packet filter for the given interests: everything the core needs plus
    /// what the interests ask for. Everything else is dropped before it is decoded.
    fn play_filter(interests: Interests) -> PacketFilter;
    /// Decodes a play-state packet; `Ok(None)` for packets the core does not use.
    fn decode_play(raw: &RawPacket) -> Result<Option<Inbound>, DecodeError>;
    /// Applies an inventory packet (windows, slots, confirms, held slot, login / respawn
    /// reset) to the inventory; returns the confirm answer to send now, if any. Other packets
    /// are ignored.
    fn inventory_packet(
        inv: &mut ghast_inventory::Inventory,
        raw: &RawPacket,
    ) -> Result<Option<ghast_inventory::Outgoing>, DecodeError>;
    /// The inventory version.
    const INVENTORY: ghast_inventory::Version;
    /// Queues a play-state packet.
    fn write_play(enc: &mut FrameEncoder, out: &Outbound) -> Result<(), EncodeError>;
    /// Applies a server teleport to the pose the way this version's client does
    /// (`onPlayerPositionLook` + `Entity.updatePositionAndAngles`).
    fn apply_teleport(pose: &mut Pose, t: &Teleport);
}

/// The world and movement implementation of a version (crate-internal).
pub(crate) trait HasSim {
    /// World + mover.
    type Sim: crate::sim::Sim;
    /// A fresh world and player.
    fn new_sim(opts: &crate::sim::SimOptions) -> Self::Sim;
}

mod sealed {
    // Deliberately unnameable: downstream crates cannot implement `ClientVersion`.
    #[allow(unnameable_types, reason = "sealed trait pattern")]
    pub trait Sealed {}
    impl Sealed for super::V1_8 {}
    impl Sealed for super::V1_16_5 {}
}

/// Truncates `s` to at most `max` UTF-16 units (Java's `String.length()`), at a character
/// boundary.
pub(crate) fn truncate_utf16(s: &str, max: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > max {
            return &s[..i];
        }
    }
    s
}

/// The common part of vanilla's teleport handling: relative axes are added, absolute axes
/// zero that axis' motion; rotations are added when relative. Returns the new yaw / pitch
/// before the version's `updatePositionAndAngles` normalisation.
pub(crate) fn teleport_target(pose: &mut Pose, t: &Teleport) -> (f32, f32) {
    use crate::common::teleport_flags as fl;
    let mut x = t.x;
    let mut y = t.y;
    let mut z = t.z;
    if t.flags & fl::X != 0 {
        x += pose.pos.x;
    } else {
        pose.motion.x = 0.0;
    }
    if t.flags & fl::Y != 0 {
        y += pose.pos.y;
    } else {
        pose.motion.y = 0.0;
    }
    if t.flags & fl::Z != 0 {
        z += pose.pos.z;
    } else {
        pose.motion.z = 0.0;
    }
    pose.pos = crate::common::Vec3::new(x, y, z);
    let mut yaw = t.yaw;
    let mut pitch = t.pitch;
    if t.flags & fl::PITCH != 0 {
        pitch += pose.pitch;
    }
    if t.flags & fl::YAW != 0 {
        yaw += pose.yaw;
    }
    (yaw, pitch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_versions() {
        assert_eq!("1.8.8".parse::<GameVersion>().unwrap(), GameVersion::V1_8_9);
        assert_eq!("754".parse::<GameVersion>().unwrap(), GameVersion::V1_16_5);
        assert!("1.12.2".parse::<GameVersion>().is_err());
        assert_eq!(GameVersion::V1_8_9.protocol(), 47);
        assert_eq!(GameVersion::V1_16_5.protocol(), 754);
    }

    #[test]
    fn utf16_truncation() {
        assert_eq!(truncate_utf16("hello", 3), "hel");
        assert_eq!(truncate_utf16("hello", 10), "hello");
        // U+1F600 is two UTF-16 units: never split.
        assert_eq!(truncate_utf16("a\u{1F600}b", 2), "a");
        assert_eq!(truncate_utf16("a\u{1F600}b", 3), "a\u{1F600}");
    }
}
