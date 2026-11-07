//! Adapters between the inventory and the generated packets of `ghast-protocol`.

use ghast_protocol::v1_8::play::{clientbound as cb8, serverbound as sb8};
use ghast_protocol::v1_12_2::play::{clientbound as cb12, serverbound as sb12};
use ghast_protocol::v1_16_5::play::{clientbound as cb16, serverbound as sb16};

use crate::inventory::{ClickPacket, Inventory, Outgoing};
use crate::stack::{from_legacy_slot, from_modern_slot, to_legacy_slot, to_modern_slot};
use crate::window::WindowKind;

impl ClickPacket {
    /// The 1.8 `window_click`.
    #[must_use]
    pub fn to_v1_8(&self) -> sb8::WindowClick {
        sb8::WindowClick {
            window_id: self.window_id,
            slot: self.slot,
            mouse_button: self.button as i8,
            action: self.action,
            mode: self.mode as i8,
            item: to_legacy_slot(self.item.as_ref()),
        }
    }

    /// The 1.12.2 `window_click`.
    #[must_use]
    pub fn to_v1_12_2(&self) -> sb12::WindowClick {
        sb12::WindowClick {
            window_id: self.window_id,
            slot: self.slot,
            mouse_button: self.button as i8,
            action: self.action,
            mode: self.mode as i8,
            item: to_legacy_slot(self.item.as_ref()),
        }
    }

    /// The 1.16.5 `window_click`.
    #[must_use]
    pub fn to_v1_16_5(&self) -> sb16::WindowClick {
        sb16::WindowClick {
            window_id: self.window_id,
            slot: self.slot,
            mouse_button: self.button as i8,
            action: self.action,
            mode: self.mode as i8,
            item: to_modern_slot(self.item.as_ref()),
        }
    }
}

impl Outgoing {
    /// The 1.8 serverbound packet.
    #[must_use]
    pub fn to_v1_8(&self) -> sb8::ServerboundPlayPacket {
        match self {
            Outgoing::Click(c) => sb8::ServerboundPlayPacket::WindowClick(c.to_v1_8()),
            Outgoing::Confirm { window_id, action } => {
                sb8::ServerboundPlayPacket::Transaction(sb8::Transaction {
                    window_id: *window_id,
                    action: *action,
                    accepted: true,
                })
            }
            Outgoing::CloseWindow(id) => {
                sb8::ServerboundPlayPacket::CloseWindow(sb8::CloseWindow { window_id: *id })
            }
            Outgoing::OpenInventory => {
                sb8::ServerboundPlayPacket::ClientCommand(sb8::ClientCommand {
                    payload: sb8::ClientCommandPayload(2),
                })
            }
        }
    }

    /// The 1.12.2 serverbound packet (`None` for [`Outgoing::OpenInventory`], which 1.12 does
    /// not have; [`Inventory::open_inventory`] never returns it on 1.12).
    #[must_use]
    pub fn to_v1_12_2(&self) -> Option<sb12::ServerboundPlayPacket> {
        Some(match self {
            Outgoing::Click(c) => sb12::ServerboundPlayPacket::WindowClick(c.to_v1_12_2()),
            Outgoing::Confirm { window_id, action } => {
                sb12::ServerboundPlayPacket::Transaction(sb12::Transaction {
                    window_id: *window_id,
                    action: *action,
                    accepted: true,
                })
            }
            Outgoing::CloseWindow(id) => {
                sb12::ServerboundPlayPacket::CloseWindow(sb12::CloseWindow { window_id: *id })
            }
            Outgoing::OpenInventory => return None,
        })
    }

    /// The 1.16.5 serverbound packet (`None` for [`Outgoing::OpenInventory`], which 1.16.5
    /// does not have; [`Inventory::open_inventory`] never returns it on 1.16.5).
    #[must_use]
    pub fn to_v1_16_5(&self) -> Option<sb16::ServerboundPlayPacket> {
        Some(match self {
            Outgoing::Click(c) => sb16::ServerboundPlayPacket::WindowClick(c.to_v1_16_5()),
            Outgoing::Confirm { window_id, action } => {
                sb16::ServerboundPlayPacket::Transaction(sb16::Transaction {
                    window_id: *window_id,
                    action: *action,
                    accepted: true,
                })
            }
            Outgoing::CloseWindow(id) => {
                sb16::ServerboundPlayPacket::CloseWindow(sb16::CloseWindow { window_id: *id })
            }
            Outgoing::OpenInventory => return None,
        })
    }
}

impl Inventory {
    /// Applies a 1.8 clientbound play packet: `login` / `respawn` (new player entity),
    /// `open_window`, `close_window`, `window_items`, `set_slot`, `transaction`,
    /// `held_item_slot`, `craft_progress_bar`; everything else is ignored. Returns the confirm
    /// answer to send now, if any (see [`Inventory::on_transaction`]).
    pub fn apply_v1_8(&mut self, p: &cb8::ClientboundPlayPacket) -> Option<Outgoing> {
        use cb8::ClientboundPlayPacket as P;
        match p {
            P::Login(_) | P::Respawn(_) => self.reset(),
            P::OpenWindow(w) => self.on_open_window(
                i32::from(w.window_id),
                WindowKind::from_v1_8(&w.inventory_type, w.slot_count),
                w.window_title.clone(),
            ),
            P::CloseWindow(_) => self.on_close_window(),
            P::WindowItems(w) => self.on_window_items(
                i32::from(w.window_id),
                w.items.iter().cloned().map(from_legacy_slot),
            ),
            P::SetSlot(s) => {
                self.on_set_slot(
                    i32::from(s.window_id),
                    s.slot,
                    from_legacy_slot(s.item.clone()),
                );
            }
            P::Transaction(t) => return self.on_transaction(t.window_id, t.action, t.accepted),
            P::HeldItemSlot(h) => self.on_held_slot(h.slot),
            P::CraftProgressBar(c) => {
                self.on_window_property(i32::from(c.window_id), c.property, c.value);
            }
            _ => {}
        }
        None
    }

    /// Applies a 1.12.2 clientbound play packet, like [`apply_v1_8`](Inventory::apply_v1_8)
    /// (`open_window` types as [`WindowKind::from_v1_12_2`] maps them).
    pub fn apply_v1_12_2(&mut self, p: &cb12::ClientboundPlayPacket) -> Option<Outgoing> {
        use cb12::ClientboundPlayPacket as P;
        match p {
            P::Login(_) | P::Respawn(_) => self.reset(),
            P::OpenWindow(w) => self.on_open_window(
                i32::from(w.window_id),
                WindowKind::from_v1_12_2(&w.inventory_type, w.slot_count),
                w.window_title.clone(),
            ),
            P::CloseWindow(_) => self.on_close_window(),
            P::WindowItems(w) => self.on_window_items(
                i32::from(w.window_id),
                w.items.iter().cloned().map(from_legacy_slot),
            ),
            P::SetSlot(s) => {
                self.on_set_slot(
                    i32::from(s.window_id),
                    s.slot,
                    from_legacy_slot(s.item.clone()),
                );
            }
            P::Transaction(t) => return self.on_transaction(t.window_id, t.action, t.accepted),
            P::HeldItemSlot(h) => self.on_held_slot(h.slot),
            P::CraftProgressBar(c) => {
                self.on_window_property(i32::from(c.window_id), c.property, c.value);
            }
            _ => {}
        }
        None
    }

    /// Applies a 1.16.5 clientbound play packet, like [`apply_v1_8`](Inventory::apply_v1_8)
    /// plus `open_horse_window` and `declare_recipes`.
    pub fn apply_v1_16_5(&mut self, p: &cb16::ClientboundPlayPacket) -> Option<Outgoing> {
        use cb16::ClientboundPlayPacket as P;
        match p {
            P::Login(_) | P::Respawn(_) => self.reset(),
            P::OpenWindow(w) => self.on_open_window(
                w.window_id,
                WindowKind::from_v1_16_5(w.inventory_type),
                w.window_title.clone(),
            ),
            P::OpenHorseWindow(w) => self.on_open_window(
                i32::from(w.window_id),
                // the slot count is a VarInt; vanilla horse inventories have at most 17
                u8::try_from(w.nb_slots)
                    .ok()
                    .map(|slots| WindowKind::Horse { slots }),
                bytestring::ByteString::new(),
            ),
            P::CloseWindow(_) => self.on_close_window(),
            P::WindowItems(w) => self.on_window_items(
                i32::from(w.window_id),
                w.items.iter().cloned().map(from_modern_slot),
            ),
            P::SetSlot(s) => {
                self.on_set_slot(
                    i32::from(s.window_id),
                    s.slot,
                    from_modern_slot(s.item.clone()),
                );
            }
            P::Transaction(t) => return self.on_transaction(t.window_id, t.action, t.accepted),
            P::HeldItemSlot(h) => self.on_held_slot(h.slot),
            P::CraftProgressBar(c) => {
                self.on_window_property(i32::from(c.window_id), c.property, c.value);
            }
            P::DeclareRecipes(d) => self.on_declare_recipes(crate::RecipeBook::from_v1_16_5(d)),
            _ => {}
        }
        None
    }
}
