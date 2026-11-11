//! Adapters between the inventory and the generated packets of `ghast-protocol`.

#[cfg(feature = "v1_8")]
use ghast_protocol::v1_8::play::{clientbound as cb8, serverbound as sb8};
#[cfg(feature = "v1_12")]
use ghast_protocol::v1_12_2::play::{clientbound as cb12, serverbound as sb12};
#[cfg(feature = "v1_16_5")]
use ghast_protocol::v1_16_5::play::{clientbound as cb16, serverbound as sb16};

#[allow(unused_imports)]
use crate::inventory::{ClickPacket, Inventory, Outgoing};
#[allow(unused_imports)]
use crate::stack::{from_legacy_slot, from_modern_slot, to_legacy_slot, to_modern_slot};
#[allow(unused_imports)]
use crate::window::WindowKind;

impl ClickPacket {
    /// The 1.8 `window_click`.
    #[cfg(feature = "v1_8")]
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
    #[cfg(feature = "v1_12")]
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
    #[cfg(feature = "v1_16_5")]
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
    /// The 1.8 serverbound packet (`None` for [`Outgoing::ContainerClick`], which only a 26.1
    /// inventory produces).
    #[cfg(feature = "v1_8")]
    #[must_use]
    pub fn to_v1_8(&self) -> Option<sb8::ServerboundPlayPacket> {
        Some(match self {
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
            Outgoing::ContainerClick(_) => return None,
        })
    }

    /// The 1.12.2 serverbound packet (`None` for [`Outgoing::OpenInventory`], which 1.12 does
    /// not have; [`Inventory::open_inventory`] never returns it on 1.12).
    #[cfg(feature = "v1_12")]
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
            Outgoing::OpenInventory | Outgoing::ContainerClick(_) => return None,
        })
    }

    /// The 1.16.5 serverbound packet (`None` for [`Outgoing::OpenInventory`], which 1.16.5
    /// does not have; [`Inventory::open_inventory`] never returns it on 1.16.5).
    #[cfg(feature = "v1_16_5")]
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
            Outgoing::OpenInventory | Outgoing::ContainerClick(_) => return None,
        })
    }
}

impl Inventory {
    /// Applies a 1.8 clientbound play packet: `login` / `respawn` (new player entity),
    /// `open_window`, `close_window`, `window_items`, `set_slot`, `transaction`,
    /// `held_item_slot`, `craft_progress_bar`; everything else is ignored. Returns the confirm
    /// answer to send now, if any (see [`Inventory::on_transaction`]).
    #[cfg(feature = "v1_8")]
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
            P::WindowItems(w) => self.on_window_items_legacy(i32::from(w.window_id), &w.items),
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
    #[cfg(feature = "v1_12")]
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
            P::WindowItems(w) => self.on_window_items_legacy(i32::from(w.window_id), &w.items),
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
    #[cfg(feature = "v1_16_5")]
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

#[cfg(feature = "v26_1")]
mod v26_1 {
    use bytes::BytesMut;
    use ghast_protocol::codec::{Encode, Reader};
    use ghast_protocol::types::slot::v775::ItemStack;
    use ghast_protocol::v26_1::play::clientbound::{self as cb, ClientboundPlayPacket};
    use ghast_protocol::v26_1::play::serverbound as sb;
    use ghast_protocol::{DecodeError, RawPacket};

    use crate::inventory::{ContainerClick, Inventory, Outgoing};
    use crate::stack::Stack;
    use crate::v26_1::{HashRegistries, hashed_slot};

    /// Clientbound 26.1 play packet ids [`Inventory::apply_raw_v26_1`] handles.
    pub const PACKET_IDS_26_1: &[i32] = &[
        cb::id::LOGIN,
        cb::id::RESPAWN,
        cb::id::OPEN_WINDOW,
        cb::id::CLOSE_WINDOW,
        cb::id::WINDOW_ITEMS,
        cb::id::SET_SLOT,
        cb::id::SET_CURSOR_ITEM,
        cb::id::SET_PLAYER_INVENTORY,
        cb::id::HELD_ITEM_SLOT,
        cb::id::CRAFT_PROGRESS_BAR,
        cb::id::DECLARE_RECIPES,
        cb::id::ABILITIES,
        cb::id::EXPERIENCE,
    ];

    fn slot(r: &mut Reader<'_>) -> Result<Option<Stack>, DecodeError> {
        Ok(ItemStack::read(r)?.map(Stack::from_v775))
    }

    /// A generated 26.1 slot as a stack (re-read lazily from its encoding).
    fn from_generated(s: &ghast_protocol::defs::p775::types::Slot) -> Option<Stack> {
        let mut w = BytesMut::new();
        s.encode(&mut w).ok()?;
        let b = w.freeze();
        ItemStack::read(&mut Reader::new(&b))
            .ok()?
            .map(Stack::from_v775)
    }

    impl ContainerClick {
        /// The 26.1 `container_click` with hashed stacks (see [`hashed_slot`]).
        #[must_use]
        pub fn to_v26_1(&self, reg: &HashRegistries) -> sb::WindowClick {
            sb::WindowClick {
                window_id: self.window_id,
                state_id: self.state_id,
                slot: self.slot,
                mouse_button: self.button as i8,
                mode: i32::from(self.mode),
                changed_slots: self
                    .changed
                    .iter()
                    .map(|(location, s)| sb::WindowClickChangedSlots {
                        location: *location,
                        item: hashed_slot(s.as_ref(), reg).0,
                    })
                    .collect(),
                cursor_item: hashed_slot(self.carried.as_ref(), reg).0,
            }
        }
    }

    impl Outgoing {
        /// The 26.1 serverbound packet (`None` for the legacy-only variants, which a 26.1
        /// inventory never returns).
        #[must_use]
        pub fn to_v26_1(&self, reg: &HashRegistries) -> Option<sb::ServerboundPlayPacket> {
            Some(match self {
                Outgoing::ContainerClick(c) => {
                    sb::ServerboundPlayPacket::WindowClick(c.to_v26_1(reg))
                }
                Outgoing::CloseWindow(id) => {
                    sb::ServerboundPlayPacket::CloseWindow(sb::CloseWindow {
                        window_id: i32::from(*id),
                    })
                }
                _ => return None,
            })
        }
    }

    impl Inventory {
        /// [`Outgoing::to_v26_1`] with this inventory's registries.
        #[must_use]
        pub fn encode_v26_1(&self, out: &Outgoing) -> Option<sb::ServerboundPlayPacket> {
            out.to_v26_1(self.registries_26_1())
        }

        /// Applies a 26.1 clientbound play packet: `login` / `respawn`, `open_screen`,
        /// `container_close`, `container_set_content`, `container_set_slot`,
        /// `set_cursor_item`, `set_player_inventory`, `set_held_slot`, `container_set_data`.
        /// Slots are converted through their encoding (use [`Inventory::apply_raw_v26_1`] to
        /// avoid decoding components at all).
        pub fn apply_v26_1(&mut self, p: &ClientboundPlayPacket) {
            use ClientboundPlayPacket as P;
            match p {
                P::Login(_) => self.reset(),
                P::Respawn(r) => {
                    // keep-inventory flags do not matter: a new player entity either way
                    let _ = r;
                    self.reset();
                }
                P::OpenWindow(w) => self.open_window_26_1(w),
                P::CloseWindow(_) => self.on_close_window(),
                P::WindowItems(w) => self.on_window_items_26_1(
                    w.window_id,
                    w.state_id,
                    w.items.iter().map(from_generated),
                    from_generated(&w.carried_item),
                ),
                P::SetSlot(s) => {
                    self.on_set_slot_26_1(s.window_id, s.state_id, s.slot, from_generated(&s.item))
                }
                P::SetCursorItem(c) => self.on_set_cursor(from_generated(&c.contents)),
                P::SetPlayerInventory(p) => {
                    self.on_set_player_inventory(p.slot_id, from_generated(&p.contents))
                }
                P::HeldItemSlot(h) => self.on_held_slot(i8::try_from(h.slot).unwrap_or(-1)),
                P::CraftProgressBar(c) => self.on_window_property(c.window_id, c.property, c.value),
                P::DeclareRecipes(d) => self.declare_recipes_26_1(d),
                P::Abilities(a) => self.set_creative_26_1(a.flags & 0x08 != 0),
                P::Experience(e) => self.set_experience_level_26_1(e.level),
                _ => {}
            }
        }

        fn declare_recipes_26_1(&mut self, d: &cb::DeclareRecipes) {
            use ghast_protocol::types::holder::HolderSet;
            let mut inputs = Some(Vec::new());
            for r in &d.stone_cutter_recipes {
                match (&r.input, &mut inputs) {
                    (HolderSet::Ids(ids), Some(v)) => v.extend_from_slice(ids),
                    _ => inputs = None,
                }
            }
            self.on_recipes_26_1(
                d.recipes.iter().map(|r| (&*r.name, r.items.as_slice())),
                inputs,
            );
        }

        /// Applies a 26.1 window packet from its raw body ([`PACKET_IDS_26_1`]): slots are read
        /// with a structural skip and keep their components as bytes (nothing is decoded until an
        /// accessor asks). Other ids go through [`Inventory::apply_v26_1`].
        ///
        /// # Errors
        /// A malformed packet (nothing changed).
        pub fn apply_raw_v26_1(&mut self, raw: &RawPacket) -> Result<(), DecodeError> {
            let mut r = Reader::new(&raw.body);
            match raw.id {
                cb::id::WINDOW_ITEMS => {
                    let window = r.read_varint()?;
                    let state = r.read_varint()?;
                    let n = r.read_varint()?;
                    let n = r.check_count(i64::from(n), 1)?;
                    let mut items = Vec::with_capacity(n.min(256));
                    for _ in 0..n {
                        items.push(slot(&mut r)?);
                    }
                    let carried = slot(&mut r)?;
                    self.on_window_items_26_1(window, state, items, carried);
                }
                cb::id::SET_SLOT => {
                    let window = r.read_varint()?;
                    let state = r.read_varint()?;
                    let s = r.read_i16()?;
                    let item = slot(&mut r)?;
                    self.on_set_slot_26_1(window, state, s, item);
                }
                cb::id::SET_CURSOR_ITEM => {
                    let item = slot(&mut r)?;
                    self.on_set_cursor(item);
                }
                cb::id::SET_PLAYER_INVENTORY => {
                    let k = r.read_varint()?;
                    let item = slot(&mut r)?;
                    self.on_set_player_inventory(k, item);
                }
                // decoded as their own type: no whole-protocol decoder is linked
                cb::id::LOGIN => {
                    raw.decode_as::<cb::Login>()?;
                    self.reset();
                }
                cb::id::RESPAWN => {
                    raw.decode_as::<cb::Respawn>()?;
                    self.reset();
                }
                cb::id::OPEN_WINDOW => self.open_window_26_1(&raw.decode_as::<cb::OpenWindow>()?),
                cb::id::CLOSE_WINDOW => {
                    raw.decode_as::<cb::CloseWindow>()?;
                    self.on_close_window();
                }
                cb::id::HELD_ITEM_SLOT => {
                    let h = raw.decode_as::<cb::HeldItemSlot>()?;
                    self.on_held_slot(i8::try_from(h.slot).unwrap_or(-1));
                }
                cb::id::CRAFT_PROGRESS_BAR => {
                    let c = raw.decode_as::<cb::CraftProgressBar>()?;
                    self.on_window_property(c.window_id, c.property, c.value);
                }
                cb::id::DECLARE_RECIPES => {
                    self.declare_recipes_26_1(&raw.decode_as::<cb::DeclareRecipes>()?);
                }
                cb::id::ABILITIES => {
                    let a = raw.decode_as::<cb::Abilities>()?;
                    self.set_creative_26_1(a.flags & 0x08 != 0);
                }
                cb::id::EXPERIENCE => {
                    let e = raw.decode_as::<cb::Experience>()?;
                    self.set_experience_level_26_1(e.level);
                }
                _ => {}
            }
            Ok(())
        }

        fn open_window_26_1(&mut self, w: &cb::OpenWindow) {
            self.on_open_screen_26_1(
                w.window_id,
                w.inventory_type,
                ghast_protocol::chat::nbt_to_plain_text(&w.window_title).into(),
            );
        }
    }
}

#[cfg(feature = "v26_1")]
pub use v26_1::PACKET_IDS_26_1;
