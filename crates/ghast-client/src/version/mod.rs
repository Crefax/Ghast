//! Per-version glue between the generated packets of `ghast-protocol` and the common layer.
//!
//! [`ClientVersion`] is implemented for the protocol marker types ([`V1_8`], [`V1_12_2`],
//! [`V1_16_5`]).
//! The connection and its tick loop are generic over it, so every hot path is monomorphised
//! per version (no trait objects); the bot API above it only sees [`crate::common`] types.

use std::fmt;
use std::str::FromStr;

use ghast_protocol::{DecodeError, EncodeError, FrameEncoder, PacketFilter, RawPacket, Version};

use crate::common::{
    ConfigInbound, ConfigOutbound, Inbound, LoginInbound, Outbound, Pose, Teleport,
};
use crate::interest::Interests;

mod caps;
#[cfg(feature = "v1_12_2")]
mod v1_12_2;
#[cfg(feature = "v1_16_5")]
mod v1_16_5;
#[cfg(feature = "v1_8")]
mod v1_8;
#[cfg(feature = "v26_1")]
mod v26_1;

pub use caps::{Capabilities, DataSet, ItemModel, PhysicsRules};
#[cfg(feature = "v1_8")]
pub use ghast_protocol::v1_8::V1_8;
#[cfg(feature = "v1_12_2")]
pub use ghast_protocol::v1_12_2::V1_12_2;
#[cfg(feature = "v1_16_5")]
pub use ghast_protocol::v1_16_5::V1_16_5;
#[cfg(feature = "v26_1")]
pub use ghast_protocol::v26_1::V26_1;
#[cfg(feature = "v26_1")]
pub use v26_1::Wire26_1;

/// A protocol version the client supports, chosen at run time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GameVersion {
    /// 1.8 – 1.8.9 (protocol 47).
    V1_8_9,
    /// 1.12.2 (protocol 340).
    V1_12_2,
    /// 1.16.4 – 1.16.5 (protocol 754).
    V1_16_5,
    /// 26.1 – 26.1.2 (protocol 775).
    V26_1,
}

impl GameVersion {
    /// Every supported version, oldest first. A slice, so that adding a version is not a
    /// breaking change.
    pub const ALL: &'static [GameVersion] = &[
        GameVersion::V1_8_9,
        GameVersion::V1_12_2,
        GameVersion::V1_16_5,
        GameVersion::V26_1,
    ];

    /// What this version can do (the row the client core reads instead of comparing
    /// versions).
    #[must_use]
    pub const fn caps(self) -> &'static Capabilities {
        match self {
            Self::V1_8_9 => &caps::V1_8_9,
            Self::V1_12_2 => &caps::V1_12_2,
            Self::V1_16_5 => &caps::V1_16_5,
            Self::V26_1 => &caps::V26_1,
        }
    }

    /// Whether this build of the library includes the version (its cargo feature is on).
    #[must_use]
    pub const fn enabled(self) -> bool {
        match self {
            Self::V1_8_9 => cfg!(feature = "v1_8"),
            Self::V1_12_2 => cfg!(feature = "v1_12_2"),
            Self::V1_16_5 => cfg!(feature = "v1_16_5"),
            Self::V26_1 => cfg!(feature = "v26_1"),
        }
    }

    /// Protocol number.
    #[must_use]
    pub const fn protocol(self) -> i32 {
        self.caps().protocol
    }

    /// This version's row in the protocol table ([`ghast_protocol::versions`]): module,
    /// feature, every game version sharing the protocol number, wire facts (the row exists
    /// whether or not the version is compiled in; see its `enabled`).
    #[must_use]
    pub fn protocol_info(self) -> &'static ghast_protocol::ProtocolVersion {
        ghast_protocol::versions::by_protocol(self.protocol())
            .expect("every GameVersion has a row in the protocol table")
    }

    /// Display name (`"1.8.9"`, `"1.12.2"`, `"1.16.5"`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::V1_8_9 => "1.8.9",
            Self::V1_12_2 => "1.12.2",
            Self::V1_16_5 => "1.16.5",
            Self::V26_1 => "26.1",
        }
    }

    /// `true` for 1.9 and later: the attack cooldown, the off hand, swinging after the click,
    /// no 0.1 targeting border on entity boxes ([`Capabilities::combat_1_9`]).
    #[must_use]
    pub const fn combat_1_9(self) -> bool {
        self.caps().combat_1_9
    }

    /// Eye height above the feet while standing ([`Capabilities::eye_height`]).
    #[must_use]
    pub const fn eye_height(self) -> f64 {
        self.caps().eye_height
    }

    /// Eye height while sneaking: 1.54 in 1.8 – 1.12 (`getEyeHeight` − 0.08), 1.27 in 1.16
    /// (the crouching pose) ([`Capabilities::sneak_eye_height`]).
    #[must_use]
    pub const fn sneak_eye_height(self) -> f64 {
        self.caps().sneak_eye_height
    }
}

impl fmt::Display for GameVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Error parsing a [`GameVersion`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unsupported Minecraft version `{0}` (supported: 1.8.9, 1.12.2, 1.16.5, 26.1)")]
pub struct UnknownVersion(pub String);

impl FromStr for GameVersion {
    type Err = UnknownVersion;

    /// Accepts `1.8`, `1.8.x`, `47`, `1.12.2`, `340`, `1.16.4`, `1.16.5`, `754`, `26.1`,
    /// `26.1.1`, `26.1.2` and `775`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "1.8" | "1.8.0" | "1.8.1" | "1.8.2" | "1.8.3" | "1.8.4" | "1.8.5" | "1.8.6"
            | "1.8.7" | "1.8.8" | "1.8.9" | "47" => Ok(Self::V1_8_9),
            "1.12.2" | "340" => Ok(Self::V1_12_2),
            "1.16.4" | "1.16.5" | "754" => Ok(Self::V1_16_5),
            "26.1" | "26.1.0" | "26.1.1" | "26.1.2" | "775" => Ok(Self::V26_1),
            other => Err(UnknownVersion(other.to_owned())),
        }
    }
}

/// What a protocol version needs to provide to the client.
///
/// Sealed: implemented for [`V1_8`], [`V1_12_2`] and [`V1_16_5`] only.
pub trait ClientVersion: Version + sealed::Sealed {
    /// The run-time tag of this version.
    const GAME: GameVersion;
    /// Longest chat line the server accepts, in UTF-16 units (vanilla truncates to it).
    const CHAT_LIMIT: usize;

    /// Queues the handshake packet (next state = login).
    fn write_handshake(enc: &mut FrameEncoder, host: &str, port: u16) -> Result<(), EncodeError>;
    /// Per-connection state of the wire layer (block action sequence numbers, chat
    /// acknowledgements, cookies); `()` for versions without any.
    type Wire: Default + fmt::Debug + Send;

    /// Queues the login start packet (1.19+ also send the profile id).
    fn write_login_start(
        enc: &mut FrameEncoder,
        username: &str,
        uuid: uuid::Uuid,
    ) -> Result<(), EncodeError>;
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
    /// Queues a login-state cookie answer (1.20.5+; never called for older versions).
    fn write_login_cookie_response(enc: &mut FrameEncoder, key: &str) -> Result<(), EncodeError> {
        let _ = (enc, key);
        Ok(())
    }
    /// Queues `login_acknowledged` (1.20.2+; never called for older versions).
    fn write_login_acknowledged(enc: &mut FrameEncoder) -> Result<(), EncodeError> {
        let _ = enc;
        Ok(())
    }
    /// Decodes a configuration-state packet (1.20.2+); `Ok(None)` for packets the client
    /// ignores. Registry data is returned raw ([`ConfigInbound::RegistryData`]).
    fn decode_configuration(
        raw: &RawPacket,
        wire: &mut Self::Wire,
    ) -> Result<Option<ConfigInbound>, DecodeError> {
        let _ = (raw, wire);
        Ok(None)
    }
    /// Queues a configuration-state packet (1.20.2+).
    fn write_configuration(
        enc: &mut FrameEncoder,
        out: &ConfigOutbound,
        wire: &mut Self::Wire,
    ) -> Result<(), EncodeError> {
        let _ = (enc, out, wire);
        Ok(())
    }
    /// A `registry_data` packet of the configuration state, for the wire layer (chat types).
    fn note_registry(raw: &RawPacket, wire: &mut Self::Wire) {
        let _ = (raw, wire);
    }
    /// Queues `configuration_acknowledged` (play → configuration, 1.20.2+).
    fn write_configuration_acknowledged(enc: &mut FrameEncoder) -> Result<(), EncodeError> {
        let _ = enc;
        Ok(())
    }
    /// Packets the wire layer itself owes the server after decoding (chat acknowledgements).
    fn write_wire_replies(
        enc: &mut FrameEncoder,
        wire: &mut Self::Wire,
    ) -> Result<(), EncodeError> {
        let _ = (enc, wire);
        Ok(())
    }

    /// The play-state packet filter for the given interests: everything the core needs plus
    /// what the interests ask for. Everything else is dropped before it is decoded.
    fn play_filter(interests: Interests) -> PacketFilter;
    /// Decodes a play-state packet; `Ok(None)` for packets the core does not use.
    fn decode_play(raw: &RawPacket, wire: &mut Self::Wire) -> Result<Option<Inbound>, DecodeError>;
    /// Applies an inventory packet (windows, slots, confirms, held slot, login / respawn
    /// reset) to the inventory; returns the confirm answer to send now, if any. Other packets
    /// are ignored.
    fn inventory_packet(
        inv: &mut ghast_inventory::Inventory,
        raw: &RawPacket,
    ) -> Result<Option<ghast_inventory::Outgoing>, DecodeError>;
    /// The inventory version.
    const INVENTORY: ghast_inventory::Version;
    /// Hands the wire layer's registry facts to the inventory (1.20.5+ hashed stacks).
    fn configure_inventory(inv: &mut ghast_inventory::Inventory, wire: &Self::Wire) {
        let _ = (inv, wire);
    }
    /// Queues a play-state packet.
    fn write_play(
        enc: &mut FrameEncoder,
        out: &Outbound,
        wire: &mut Self::Wire,
    ) -> Result<(), EncodeError>;
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
    #[cfg(feature = "v1_8")]
    impl Sealed for super::V1_8 {}
    #[cfg(feature = "v1_12_2")]
    impl Sealed for super::V1_12_2 {}
    #[cfg(feature = "v1_16_5")]
    impl Sealed for super::V1_16_5 {}
    #[cfg(feature = "v26_1")]
    impl Sealed for super::V26_1 {}
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
    fn protocol_rows() {
        for &v in GameVersion::ALL {
            assert_eq!(v.caps().version, v);
            let row = v.protocol_info();
            assert_eq!(row.protocol, v.protocol());
            assert!(row.enabled || !v.enabled());
            assert!(row.game_versions.contains(&v.name()));
        }
    }

    #[test]
    fn parse_versions() {
        assert_eq!("1.8.8".parse::<GameVersion>().unwrap(), GameVersion::V1_8_9);
        assert_eq!("754".parse::<GameVersion>().unwrap(), GameVersion::V1_16_5);
        assert_eq!(
            "1.12.2".parse::<GameVersion>().unwrap(),
            GameVersion::V1_12_2
        );
        assert_eq!("340".parse::<GameVersion>().unwrap(), GameVersion::V1_12_2);
        assert!("1.12.1".parse::<GameVersion>().is_err());
        assert_eq!(GameVersion::V1_8_9.protocol(), 47);
        assert_eq!(GameVersion::V1_12_2.protocol(), 340);
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
