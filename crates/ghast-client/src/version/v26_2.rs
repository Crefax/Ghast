//! Protocol 776 (26.2): the row of the modern wire layer (`modern.rs`, mounted below as `imp`) —
//! the protocol module, the tables and the inventory entry points of this version. Everything
//! else is shared with the other modern versions. The true 26.2 deltas of the wire: the play
//! login's `onlineMode` (here), the team parameter order (the sim row), the attribute
//! renumbering (resolved by name in the body). The login's session id is telemetry in the
//! vanilla client and is not used.

use ghast_protocol::RawPacket;
#[cfg(test)]
use ghast_protocol::types::ByteString;

use super::GameVersion;

#[cfg(test)]
pub(crate) use ghast_data::v26_2 as data;
pub(crate) use ghast_protocol::v26_2 as proto;
pub(crate) use ghast_protocol::v26_2::V26_2 as Marker;
pub(crate) use ghast_world::v26_2 as world;

/// The run-time tag of this version.
pub(crate) const GAME: GameVersion = GameVersion::V26_2;
/// The inventory version.
pub(crate) const INVENTORY: ghast_inventory::Version = ghast_inventory::Version::V26_2;
/// Every play packet the inventory reads.
#[inline(always)]
pub(crate) fn inventory_packet_ids() -> &'static [i32] {
    ghast_inventory::modern::packet_ids(INVENTORY)
}
/// The oracle goldens of this version's wire (`tests/golden/<dir>/`).
#[cfg(test)]
pub(crate) const GOLDEN_DIR: &str = "26.2";
/// The world and movement of this version.
pub(crate) type Sim = crate::sim::Sim262;

// One body, mounted once per modern version (see CONTRIBUTING.md).
#[allow(clippy::duplicate_mod)]
#[path = "modern.rs"]
mod imp;

pub use imp::Wire;

/// The play login's `onlineMode`: the client prepares a chat session only then
/// (`ClientPacketListener.handleLogin`; 26.1 asked whether the connection is encrypted).
#[inline(always)]
pub(crate) fn login_online_mode(p: &proto::play::clientbound::Login) -> Option<bool> {
    Some(p.online_mode)
}

/// This version's modern item functions (`ghast_inventory::modern` with the version fixed).
pub(crate) mod inv {
    pub(crate) use ghast_inventory::modern::HashRegistries;
    #[cfg(test)]
    pub(crate) fn item_id(name: &str) -> Option<i32> {
        ghast_inventory::modern::item_id(super::INVENTORY, name)
    }
    #[cfg(test)]
    pub(crate) fn window_kind(menu: i32) -> Option<ghast_inventory::WindowKind> {
        ghast_inventory::modern::window_kind(super::INVENTORY, menu)
    }
}

// The inventory's entry points of this version (the shared body calls these).

pub(crate) fn inv_on_update_tags(inv: &mut ghast_inventory::Inventory, body: bytes::Bytes) {
    inv.on_update_tags_modern(body);
}

pub(crate) fn inv_on_registry_data(inv: &mut ghast_inventory::Inventory, body: bytes::Bytes) {
    inv.on_registry_data_modern(body);
}

pub(crate) fn inv_on_mount_screen(
    inv: &mut ghast_inventory::Inventory,
    window_id: i32,
    columns: i32,
    entity_type: i32,
    can_saddle: bool,
    can_body: bool,
) {
    inv.on_mount_screen_modern(window_id, columns, entity_type, can_saddle, can_body);
}

pub(crate) fn inv_apply_raw(
    inv: &mut ghast_inventory::Inventory,
    raw: &RawPacket,
) -> Result<(), ghast_protocol::DecodeError> {
    inv.apply_raw_modern(raw)
}

pub(crate) fn inv_take_creative_echo(
    inv: &mut ghast_inventory::Inventory,
) -> Vec<ghast_inventory::CreativePacket> {
    inv.take_creative_echo_modern()
}

pub(crate) fn inv_set_registries(inv: &mut ghast_inventory::Inventory, reg: inv::HashRegistries) {
    inv.set_registries_modern(reg);
}

pub(crate) fn inv_packet(
    o: &ghast_inventory::Outgoing,
    reg: &inv::HashRegistries,
) -> Option<proto::play::serverbound::ServerboundPlayPacket> {
    o.to_v26_2(reg)
}

pub(crate) fn inv_creative_packet(
    c: &ghast_inventory::CreativePacket,
) -> proto::play::serverbound::ServerboundPlayPacket {
    c.to_v26_2()
}

#[cfg(test)]
pub(crate) fn inv_set_creative(inv: &mut ghast_inventory::Inventory, on: bool) {
    inv.set_creative_modern(on);
}

#[cfg(test)]
pub(crate) fn inv_on_window_items(
    inv: &mut ghast_inventory::Inventory,
    window_id: i32,
    state_id: i32,
    items: Vec<Option<ghast_inventory::Stack>>,
    carried: Option<ghast_inventory::Stack>,
) {
    inv.on_window_items_modern(window_id, state_id, items, carried);
}

#[cfg(test)]
pub(crate) fn inv_on_open_screen(
    inv: &mut ghast_inventory::Inventory,
    id: i32,
    menu: i32,
    title: ByteString,
) {
    inv.on_open_screen_modern(id, menu, title);
}

#[cfg(test)]
pub(crate) fn inv_anvil_rename(
    inv: &mut ghast_inventory::Inventory,
    name: &str,
) -> Result<Option<ghast_inventory::Outgoing>, ghast_inventory::ClickError> {
    inv.anvil_rename_modern(name)
}

#[cfg(test)]
pub(crate) fn inv_select_trade(
    inv: &mut ghast_inventory::Inventory,
    index: i32,
) -> Result<ghast_inventory::Outgoing, ghast_inventory::ClickError> {
    inv.select_trade_modern(index)
}

#[cfg(test)]
mod tests {
    /// The 26.2 attribute ids resolve to the names movement reads, the three 26.2 additions
    /// included (26.2 registries report: 40 entries, renumbered).
    #[test]
    fn attribute_ids_by_name() {
        use super::proto::play::clientbound::EntityUpdateAttributesPropertiesKey as K;
        let name = |id| super::imp::attribute_name_of(K::new(id));
        assert_eq!(name(7), Some("block_break_speed"));
        assert_eq!(name(18), Some("gravity"));
        assert_eq!(name(26), Some("movement_speed"));
        assert_eq!(name(30), Some("scale"));
        assert_eq!(name(33), Some("step_height"));
        assert_eq!(name(0), Some("air_drag_modifier"));
        assert_eq!(name(9), Some("bounciness"));
        assert_eq!(name(17), Some("friction_modifier"));
        assert_eq!((0..64).filter(|&i| name(i).is_some()).count(), 17);
    }
}
