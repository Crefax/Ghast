//! The engine replays of the 26.2 oracle goldens (`tests/golden/26.2/`): the shared replays
//! (`engine_modern_tests.rs`) with the 26.2 modules.

pub(super) use super::tests::*;
pub(super) use super::*;
pub(super) use crate::common::{EntityAction, JoinGame, Teleport};
pub(super) use crate::phys_golden::v261 as golden;
pub(super) use crate::sim::test_chunks::{v26_2 as chunk, v26_2_sections as chunk_sections};
pub(super) use crate::version::v26_2 as wire_row;
pub(super) use crate::version::v26_2::inv;
pub(super) use crate::version::{V26_2 as Marker, Wire26_2 as Wire};
pub(super) use ghast_data::v26_2 as data;
pub(super) use ghast_protocol::RawPacket;
pub(super) use ghast_protocol::v26_2 as proto;
pub(super) type ModernSim = crate::sim::Sim262;
pub(super) const GAME: GameVersion = GameVersion::V26_2;
pub(super) const INVENTORY: ghast_inventory::Version = ghast_inventory::Version::V26_2;
pub(super) const GOLDEN: &str = "26.2";

/// A component patch with one `enchantments` component (its body).
pub(super) fn enchantments_patch(body: &[u8]) -> ghast_protocol::types::slot::v775::ComponentPatch {
    let id = ghast_inventory::modern::component_id(INVENTORY, "enchantments").unwrap();
    ghast_protocol::types::slot::v775::ComponentPatch::from_parts_776(&[(id, body)], &[])
}

#[allow(clippy::duplicate_mod)]
#[path = "engine_modern_tests.rs"]
mod replays;
