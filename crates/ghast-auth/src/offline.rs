//! Offline-mode identities.
//!
//! In offline mode the server never talks to Mojang: the client only sends a username and the
//! server derives the player's UUID from it. Vanilla (`ServerLoginNetworkHandler`) and
//! Bukkit/Spigot/Paper all compute it as
//! `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name).getBytes(UTF_8))`, i.e. a version-3 UUID
//! built from the MD5 of that string *without* a namespace. [`offline_uuid`] reproduces this
//! exactly, so a bot knows its own UUID (and the UUIDs of other offline players) up front.

use md5::{Digest, Md5};
use thiserror::Error;
use uuid::Uuid;

/// Maximum username length accepted by the Login Start packet (in Java `char`s).
pub const MAX_USERNAME_LEN: usize = 16;

/// Derives the offline-mode UUID of `name`, exactly as vanilla and Bukkit servers do.
///
/// The function accepts any string; it does not validate the name. Case matters: `"Steve"` and
/// `"steve"` get different UUIDs.
///
/// ```
/// use ghast_auth::offline::offline_uuid;
///
/// assert_eq!(
///     offline_uuid("Notch").to_string(),
///     "b50ad385-829d-3141-a216-7e7d7539ba7f",
/// );
/// ```
pub fn offline_uuid(name: &str) -> Uuid {
    let mut hasher = Md5::new();
    hasher.update(b"OfflinePlayer:");
    hasher.update(name.as_bytes());
    let mut bytes: [u8; 16] = hasher.finalize().into();
    // Same bit twiddling as java.util.UUID.nameUUIDFromBytes: version 3, IETF variant.
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

/// Why a username was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UsernameError {
    /// The name is empty.
    #[error("username is empty")]
    Empty,
    /// The name is longer than [`MAX_USERNAME_LEN`] Java characters.
    #[error("username is {len} characters long, the maximum is {MAX_USERNAME_LEN}")]
    TooLong {
        /// Length in UTF-16 code units (Java `String.length()`).
        len: usize,
    },
    /// The name contains a character that the chosen validation level does not allow.
    #[error("username contains the invalid character {0:?}")]
    InvalidChar(char),
}

/// Checks that `name` is a conventional Minecraft username: 1 to 16 characters from
/// `A-Z a-z 0-9 _`.
///
/// This is the character set Mojang accounts use and what modern servers enforce for offline
/// players. Use it for bot names unless a server is known to accept something else.
pub fn validate_username(name: &str) -> Result<(), UsernameError> {
    check_len(name)?;
    match name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
    {
        Some(c) => Err(UsernameError::InvalidChar(c)),
        None => Ok(()),
    }
}

/// Checks only what the protocol itself requires: 1 to 16 Java characters and no control
/// characters.
///
/// Old servers (for example 1.8.x) accept such names in offline mode; modern servers usually
/// reject anything outside [`validate_username`]'s character set.
pub fn validate_username_lenient(name: &str) -> Result<(), UsernameError> {
    check_len(name)?;
    match name.chars().find(|c| c.is_control()) {
        Some(c) => Err(UsernameError::InvalidChar(c)),
        None => Ok(()),
    }
}

fn check_len(name: &str) -> Result<(), UsernameError> {
    if name.is_empty() {
        return Err(UsernameError::Empty);
    }
    // Java measures strings in UTF-16 code units.
    let len = name.encode_utf16().count();
    if len > MAX_USERNAME_LEN {
        return Err(UsernameError::TooLong { len });
    }
    Ok(())
}

/// An offline-mode player: a validated username and its derived UUID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OfflineProfile {
    name: String,
    uuid: Uuid,
}

impl OfflineProfile {
    /// Creates a profile after checking `name` with [`validate_username`].
    pub fn new(name: impl Into<String>) -> Result<Self, UsernameError> {
        let name = name.into();
        validate_username(&name)?;
        Ok(Self::from_valid(name))
    }

    /// Creates a profile after checking `name` with [`validate_username_lenient`].
    pub fn new_lenient(name: impl Into<String>) -> Result<Self, UsernameError> {
        let name = name.into();
        validate_username_lenient(&name)?;
        Ok(Self::from_valid(name))
    }

    fn from_valid(name: String) -> Self {
        let uuid = offline_uuid(&name);
        Self { name, uuid }
    }

    /// The username, as sent in the Login Start packet.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The UUID the server will assign to this player.
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known answers produced by `java.util.UUID.nameUUIDFromBytes` (OpenJDK 25).
    const KAT: &[(&str, &str)] = &[
        ("Notch", "b50ad385-829d-3141-a216-7e7d7539ba7f"),
        ("jeb_", "a762f560-4fce-3236-812a-b80efff0b62b"),
        ("Crefax", "0b5961bd-6ee4-399a-9dd0-f0ea1ff523d0"),
        ("a", "52428a0e-1e30-3cb1-976c-e728b2614047"),
        (
            "\u{c4}\u{d6}\u{fc}_x",
            "d31745e6-b554-3a81-bbc7-cea01c69cc7a",
        ),
    ];

    #[test]
    fn offline_uuid_matches_java() {
        for (name, expected) in KAT {
            assert_eq!(offline_uuid(name).to_string(), *expected, "name {name:?}");
        }
    }

    #[test]
    fn offline_uuid_is_version_3() {
        let u = offline_uuid("Notch");
        assert_eq!(u.get_version_num(), 3);
        assert_eq!(u.get_variant(), uuid::Variant::RFC4122);
    }

    #[test]
    fn strict_validation() {
        assert!(validate_username("Steve").is_ok());
        assert!(validate_username("a").is_ok());
        assert!(validate_username("abcdefghijklmnop").is_ok());
        assert!(validate_username("Bot_01").is_ok());
        assert_eq!(validate_username(""), Err(UsernameError::Empty));
        assert_eq!(
            validate_username("abcdefghijklmnopq"),
            Err(UsernameError::TooLong { len: 17 })
        );
        assert_eq!(
            validate_username("a b"),
            Err(UsernameError::InvalidChar(' '))
        );
        assert_eq!(
            validate_username("caf\u{e9}"),
            Err(UsernameError::InvalidChar('\u{e9}'))
        );
        assert_eq!(
            validate_username("a-b"),
            Err(UsernameError::InvalidChar('-'))
        );
    }

    #[test]
    fn lenient_validation() {
        assert!(validate_username_lenient("caf\u{e9}-1").is_ok());
        assert_eq!(
            validate_username_lenient("a\nb"),
            Err(UsernameError::InvalidChar('\n'))
        );
        // 16 two-byte UTF-8 characters are 16 Java chars: allowed.
        assert!(validate_username_lenient(&"\u{e9}".repeat(16)).is_ok());
        // A character outside the BMP counts as two Java chars.
        assert_eq!(
            validate_username_lenient(&"\u{1f600}".repeat(9)),
            Err(UsernameError::TooLong { len: 18 })
        );
    }

    #[test]
    fn profile() {
        let p = OfflineProfile::new("Notch").unwrap();
        assert_eq!(p.name(), "Notch");
        assert_eq!(p.uuid(), offline_uuid("Notch"));
        assert!(OfflineProfile::new("no spaces").is_err());
        assert!(OfflineProfile::new_lenient("caf\u{e9}").is_ok());
    }
}
