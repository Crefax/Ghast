//! The protocol version table: one row per protocol number this crate knows.
//!
//! A row is the protocol layer's view of a version: its number, the generated module, the
//! game versions that speak it (e.g. 775 = 26.1, 26.1.1 and 26.1.2) and the few wire facts
//! that differ between versions outside the packet definitions. Rows of versions whose
//! Cargo feature is off stay in the table with `enabled == false`, so a caller can tell
//! "unknown" from "not compiled in".
//!
//! Layers above (data sets, physics rules) keep their own columns keyed by the protocol
//! number or the game version; [`ProtocolVersion`] is `#[non_exhaustive]`, so columns of
//! this layer can be added without a breaking change.

use crate::types::PositionLayout;

/// One protocol number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProtocolVersion {
    /// Protocol number sent in the handshake.
    pub protocol: i32,
    /// Name of the generated module (`"v1_8"`, `"v26_1"`).
    pub module: &'static str,
    /// Cargo feature that compiles the module in.
    pub feature: &'static str,
    /// The version the module is named after and tested with (`"1.8.9"`).
    pub name: &'static str,
    /// Every release that speaks this protocol, oldest first (from minecraft-data's
    /// `protocolVersions.json`).
    pub game_versions: &'static [&'static str],
    /// Bit layout of packed block positions.
    pub position_layout: PositionLayout,
    /// Whether the version has the configuration state (1.20.2+).
    pub configuration: bool,
    /// Whether the module is compiled in (its feature is on).
    pub enabled: bool,
}

include!("generated/versions_table.rs");

/// The row of a protocol number.
#[must_use]
pub fn by_protocol(protocol: i32) -> Option<&'static ProtocolVersion> {
    TABLE.iter().find(|v| v.protocol == protocol)
}

/// The row of a game version name (`"1.8.8"`, `"26.1.2"`).
#[must_use]
pub fn by_game_version(name: &str) -> Option<&'static ProtocolVersion> {
    TABLE.iter().find(|v| v.game_versions.contains(&name))
}

/// The rows that are compiled in.
pub fn enabled() -> impl Iterator<Item = &'static ProtocolVersion> {
    TABLE.iter().filter(|v| v.enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_unique() {
        for w in TABLE.windows(2) {
            assert!(w[0].protocol < w[1].protocol);
        }
        for v in TABLE {
            assert!(!v.game_versions.is_empty());
            assert_eq!(by_protocol(v.protocol), Some(v));
            for g in v.game_versions {
                assert_eq!(by_game_version(g), Some(v));
            }
        }
        assert_eq!(by_game_version("1.8.8").map(|v| v.protocol), Some(47));
        assert_eq!(by_game_version("1.16.4").map(|v| v.protocol), Some(754));
        assert_eq!(
            by_protocol(775).map(|v| v.game_versions),
            Some(&["26.1", "26.1.1", "26.1.2"][..])
        );
        assert!(by_protocol(775).is_some_and(|v| v.configuration));
        assert!(by_protocol(1).is_none());
    }
}
