//! The engine replays of the 26.1 oracle goldens (`tests/golden/26.1/`): the shared replays
//! (`engine_modern_tests.rs`) with the 26.1 modules.

pub(super) use super::tests::*;
pub(super) use super::*;
pub(super) use crate::common::{EntityAction, JoinGame, Teleport};
pub(super) use crate::phys_golden::v261 as golden;
pub(super) use crate::sim::test_chunks::{v26_1 as chunk, v26_1_sections as chunk_sections};
pub(super) use crate::version::v26_1 as wire_row;
pub(super) use crate::version::{V26_1 as Marker, Wire26_1 as Wire};
pub(super) use ghast_data::v26_1 as data;
pub(super) use ghast_inventory::v26_1 as inv;
pub(super) use ghast_protocol::RawPacket;
pub(super) use ghast_protocol::v26_1 as proto;
pub(super) type ModernSim = crate::sim::Sim26;
pub(super) const GAME: GameVersion = GameVersion::V26_1;
pub(super) const INVENTORY: ghast_inventory::Version = ghast_inventory::Version::V26_1;
pub(super) const GOLDEN: &str = "26.1";

/// A component patch with one `enchantments` component (its body).
pub(super) fn enchantments_patch(body: &[u8]) -> ghast_protocol::types::slot::v775::ComponentPatch {
    ghast_protocol::types::slot::v775::ComponentPatch::from_parts(
        &[(inv::component::ENCHANTMENTS, body)],
        &[],
    )
}

#[allow(clippy::duplicate_mod)]
#[path = "engine_modern_tests.rs"]
mod replays;
