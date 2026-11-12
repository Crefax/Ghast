//! Protocol 775 (26.1 – 26.1.2): the row of the modern wire layer (`modern.rs`, mounted below
//! as `imp`) — the protocol module, the tables and the inventory entry points of this version.
//! Everything else is shared with the other modern versions.

use ghast_protocol::RawPacket;
#[cfg(test)]
use ghast_protocol::types::ByteString;

use super::GameVersion;

#[cfg(test)]
pub(crate) use ghast_data::v26_1 as data;
pub(crate) use ghast_inventory::v26_1 as inv;
pub(crate) use ghast_protocol::v26_1 as proto;
pub(crate) use ghast_protocol::v26_1::V26_1 as Marker;
pub(crate) use ghast_world::v26_1 as world;

/// The run-time tag of this version.
pub(crate) const GAME: GameVersion = GameVersion::V26_1;
/// The inventory version.
pub(crate) const INVENTORY: ghast_inventory::Version = ghast_inventory::Version::V26_1;
/// Every play packet the inventory reads.
pub(crate) const INVENTORY_PACKET_IDS: &[i32] = ghast_inventory::PACKET_IDS_26_1;
/// The oracle goldens of this version's wire (`tests/golden/<dir>/`).
#[cfg(test)]
pub(crate) const GOLDEN_DIR: &str = "26.1";
/// The world and movement of this version.
pub(crate) type Sim = crate::sim::Sim26;

#[path = "modern.rs"]
mod imp;

pub use imp::Wire;

/// The play login's `onlineMode` (26.2+); 26.1 decides by encryption.
#[inline(always)]
pub(crate) fn login_online_mode(p: &proto::play::clientbound::Login) -> Option<bool> {
    let _ = p;
    None
}

// The inventory's entry points of this version (the shared body calls these).

pub(crate) fn inv_on_update_tags(inv: &mut ghast_inventory::Inventory, body: bytes::Bytes) {
    inv.on_update_tags_26_1(body);
}

pub(crate) fn inv_on_registry_data(inv: &mut ghast_inventory::Inventory, body: bytes::Bytes) {
    inv.on_registry_data_26_1(body);
}

pub(crate) fn inv_on_mount_screen(
    inv: &mut ghast_inventory::Inventory,
    window_id: i32,
    columns: i32,
    entity_type: i32,
    can_saddle: bool,
    can_body: bool,
) {
    inv.on_mount_screen_26_1(window_id, columns, entity_type, can_saddle, can_body);
}

pub(crate) fn inv_apply_raw(
    inv: &mut ghast_inventory::Inventory,
    raw: &RawPacket,
) -> Result<(), ghast_protocol::DecodeError> {
    inv.apply_raw_v26_1(raw)
}

pub(crate) fn inv_take_creative_echo(
    inv: &mut ghast_inventory::Inventory,
) -> Vec<ghast_inventory::CreativePacket> {
    inv.take_creative_echo_26_1()
}

pub(crate) fn inv_set_registries(inv: &mut ghast_inventory::Inventory, reg: inv::HashRegistries) {
    inv.set_registries_26_1(reg);
}

pub(crate) fn inv_packet(
    o: &ghast_inventory::Outgoing,
    reg: &inv::HashRegistries,
) -> Option<proto::play::serverbound::ServerboundPlayPacket> {
    o.to_v26_1(reg)
}

pub(crate) fn inv_creative_packet(
    c: &ghast_inventory::CreativePacket,
) -> proto::play::serverbound::ServerboundPlayPacket {
    c.to_v26_1()
}

#[cfg(test)]
pub(crate) fn inv_set_creative(inv: &mut ghast_inventory::Inventory, on: bool) {
    inv.set_creative_26_1(on);
}

#[cfg(test)]
pub(crate) fn inv_on_window_items(
    inv: &mut ghast_inventory::Inventory,
    window_id: i32,
    state_id: i32,
    items: Vec<Option<ghast_inventory::Stack>>,
    carried: Option<ghast_inventory::Stack>,
) {
    inv.on_window_items_26_1(window_id, state_id, items, carried);
}

#[cfg(test)]
pub(crate) fn inv_on_open_screen(
    inv: &mut ghast_inventory::Inventory,
    id: i32,
    menu: i32,
    title: ByteString,
) {
    inv.on_open_screen_26_1(id, menu, title);
}

#[cfg(test)]
pub(crate) fn inv_anvil_rename(
    inv: &mut ghast_inventory::Inventory,
    name: &str,
) -> Result<Option<ghast_inventory::Outgoing>, ghast_inventory::ClickError> {
    inv.anvil_rename_26_1(name)
}

#[cfg(test)]
pub(crate) fn inv_select_trade(
    inv: &mut ghast_inventory::Inventory,
    index: i32,
) -> Result<ghast_inventory::Outgoing, ghast_inventory::ClickError> {
    inv.select_trade_26_1(index)
}

#[cfg(test)]
mod tests {
    /// The registry-name attribute table gives exactly the 26.1 ids movement read before (the
    /// numeric table it replaced, checked against the 26.1.2 registries report).
    #[test]
    fn attribute_ids_by_name() {
        use super::proto::play::clientbound::EntityUpdateAttributesPropertiesKey as K;
        let old = [
            (5, "block_break_speed"),
            (6, "block_interaction_range"),
            (10, "entity_interaction_range"),
            (14, "gravity"),
            (15, "jump_strength"),
            (20, "mining_efficiency"),
            (21, "movement_efficiency"),
            (22, "movement_speed"),
            (24, "safe_fall_distance"),
            (25, "scale"),
            (26, "sneaking_speed"),
            (28, "step_height"),
            (29, "submerged_mining_speed"),
            (32, "water_movement_efficiency"),
        ];
        for id in 0..64 {
            let want = old.iter().find(|o| o.0 == id).map(|o| o.1);
            assert_eq!(super::imp::attribute_name_of(K(id)), want, "{id}");
        }
    }
}
