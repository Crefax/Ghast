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
            Outgoing::ContainerClick(_)
            | Outgoing::Creative(_)
            | Outgoing::RenameItem(_)
            | Outgoing::ButtonClick { .. }
            | Outgoing::SelectTrade(_) => return None,
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
            Outgoing::OpenInventory
            | Outgoing::ContainerClick(_)
            | Outgoing::Creative(_)
            | Outgoing::RenameItem(_)
            | Outgoing::ButtonClick { .. }
            | Outgoing::SelectTrade(_) => {
                return None;
            }
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
            Outgoing::OpenInventory
            | Outgoing::ContainerClick(_)
            | Outgoing::Creative(_)
            | Outgoing::RenameItem(_)
            | Outgoing::ButtonClick { .. }
            | Outgoing::SelectTrade(_) => {
                return None;
            }
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

// ------------------------------------------------------------------ modern (26.x)

/// One instance per modern protocol: the generated packet types differ (registry-mapped
/// layouts are duplicated per protocol), the code does not. Raw window packets are applied
/// here, so the slot reader is a constant of each instance.
#[cfg(modern)]
macro_rules! modern_packets {
    ($m:ident, $proto:ident, $defs:ident, $mv:expr, $ty:path) => {
        pub(crate) mod $m {
            use bytes::BytesMut;
            use ghast_protocol::codec::{Encode, Reader};
            use ghast_protocol::defs::$defs::types as t;
            use ghast_protocol::$proto::play::clientbound::{self as cb, ClientboundPlayPacket};
            use ghast_protocol::$proto::play::serverbound as sb;
            use ghast_protocol::{DecodeError, RawPacket};

            use crate::inventory::{ContainerClick, CreativePacket, Inventory, Outgoing};
            use crate::item26::{HashRegistries, Hashed, hashed};
            use crate::stack::Stack;
            use crate::tables::Mv;

            const MV: Mv = $mv;

            /// Clientbound play packet ids [`apply_raw`] handles.
            pub(crate) const PACKET_IDS: &[i32] = &[
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
                cb::id::TAGS,
                cb::id::TRADE_LIST,
            ];

            #[inline]
            fn slot(r: &mut Reader<'_>) -> Result<Option<Stack>, DecodeError> {
                Ok(MV.read_stack(r)?.map(Stack::from_v775))
            }

            /// A generated slot as a stack (re-read lazily from its encoding).
            #[allow(dead_code)]
            fn from_generated(s: &t::Slot) -> Option<Stack> {
                let mut w = BytesMut::new();
                s.encode(&mut w).ok()?;
                let b = w.freeze();
                MV.read_stack(&mut Reader::new(&b))
                    .ok()?
                    .map(Stack::from_v775)
            }

            /// A hashed stack as the generated type.
            pub(crate) fn hashed_slot(h: Hashed) -> t::HashedSlot {
                t::HashedSlot {
                    item_id: h.item_id,
                    item_count: h.item_count,
                    components: h
                        .components
                        .into_iter()
                        .map(|(ty, hash)| t::HashedSlotComponents {
                            r#type: $ty(ty),
                            hash,
                        })
                        .collect(),
                    remove_components: h
                        .removed
                        .into_iter()
                        .map(|ty| t::HashedSlotRemoveComponents { r#type: $ty(ty) })
                        .collect(),
                }
            }

            fn hashed_opt(s: Option<&Stack>, reg: &HashRegistries) -> Option<t::HashedSlot> {
                hashed(MV, s, reg).0.map(hashed_slot)
            }

            /// The `container_click` with hashed stacks.
            pub(crate) fn click(c: &ContainerClick, reg: &HashRegistries) -> sb::WindowClick {
                sb::WindowClick {
                    window_id: c.window_id,
                    state_id: c.state_id,
                    slot: c.slot,
                    mouse_button: c.button as i8,
                    mode: i32::from(c.mode),
                    changed_slots: c
                        .changed
                        .iter()
                        .map(|(location, s)| sb::WindowClickChangedSlots {
                            location: *location,
                            item: hashed_opt(s.as_ref(), reg),
                        })
                        .collect(),
                    cursor_item: hashed_opt(c.carried.as_ref(), reg),
                }
            }

            /// `swing` (main hand) or `set_creative_mode_slot` (the stack in the untrusted
            /// layout, components in type id order).
            pub(crate) fn creative(c: &CreativePacket) -> sb::ServerboundPlayPacket {
                match c {
                    CreativePacket::Swing => {
                        sb::ServerboundPlayPacket::ArmAnimation(sb::ArmAnimation {
                            hand: sb::ArmAnimationHand(0),
                        })
                    }
                    CreativePacket::SetSlot { slot, item } => {
                        let item = match item.as_ref().filter(|s| s.count > 0 && s.id != 0) {
                            None => t::UntrustedSlot {
                                item_count: 0,
                                body: None,
                            },
                            Some(s) => {
                                let mut components: Vec<t::UntrustedSlotComponent> = Vec::new();
                                let mut removed: Vec<i32> = Vec::new();
                                if let Some(p) = s.components() {
                                    for (ty, b) in p.iter() {
                                        match components.iter_mut().find(|c| c.r#type.0 == ty) {
                                            Some(c) => c.data = b,
                                            None => components.push(t::UntrustedSlotComponent {
                                                r#type: $ty(ty),
                                                data: b,
                                            }),
                                        }
                                    }
                                    removed.extend(p.removed());
                                }
                                components.sort_by_key(|c| c.r#type.0);
                                removed.sort_unstable();
                                removed.dedup();
                                t::UntrustedSlot {
                                    item_count: s.count,
                                    body: Some(t::UntrustedSlotBodyDefault {
                                        item_id: s.id,
                                        added_component_count: components.len() as i32,
                                        removed_component_count: removed.len() as i32,
                                        components,
                                        remove_components: removed
                                            .into_iter()
                                            .map(|ty| t::UntrustedSlotBodyDefaultRemoveComponents {
                                                r#type: $ty(ty),
                                            })
                                            .collect(),
                                    }),
                                }
                            }
                        };
                        sb::ServerboundPlayPacket::SetCreativeSlot(sb::SetCreativeSlot {
                            slot: *slot,
                            item,
                        })
                    }
                }
            }

            /// The serverbound packet (`None` for the legacy-only variants and for
            /// [`Outgoing::Creative`], whose packets [`creative`] encodes one by one).
            pub(crate) fn outgoing(
                o: &Outgoing,
                reg: &HashRegistries,
            ) -> Option<sb::ServerboundPlayPacket> {
                Some(match o {
                    Outgoing::ContainerClick(c) => {
                        sb::ServerboundPlayPacket::WindowClick(click(c, reg))
                    }
                    Outgoing::CloseWindow(id) => {
                        sb::ServerboundPlayPacket::CloseWindow(sb::CloseWindow {
                            window_id: i32::from(*id),
                        })
                    }
                    Outgoing::RenameItem(n) => sb::ServerboundPlayPacket::NameItem(sb::NameItem {
                        name: n.as_str().into(),
                    }),
                    Outgoing::ButtonClick { window_id, button } => {
                        sb::ServerboundPlayPacket::EnchantItem(sb::EnchantItem {
                            window_id: *window_id,
                            enchantment: *button,
                        })
                    }
                    Outgoing::SelectTrade(i) => {
                        sb::ServerboundPlayPacket::SelectTrade(sb::SelectTrade { slot: *i })
                    }
                    _ => return None,
                })
            }

            /// A serverbound packet as raw id + body.
            pub(crate) fn raw(p: &sb::ServerboundPlayPacket) -> Option<RawPacket> {
                use ghast_protocol::PacketSet;
                let mut w = BytesMut::new();
                p.encode_body(&mut w).ok()?;
                Some(RawPacket::new(p.id(), w.freeze()))
            }

            /// Applies a typed clientbound play packet.
            #[allow(dead_code)]
            pub(crate) fn apply(inv: &mut Inventory, p: &ClientboundPlayPacket) {
                use ClientboundPlayPacket as P;
                match p {
                    P::Login(_) => inv.reset(),
                    // keep-inventory flags do not matter: a new player entity either way
                    P::Respawn(_) => inv.reset(),
                    P::OpenWindow(w) => open_window(inv, w),
                    P::CloseWindow(_) => inv.on_close_window(),
                    P::WindowItems(w) => inv.on_window_items_modern(
                        w.window_id,
                        w.state_id,
                        w.items.iter().map(from_generated),
                        from_generated(&w.carried_item),
                    ),
                    P::SetSlot(s) => inv.on_set_slot_modern(
                        s.window_id,
                        s.state_id,
                        s.slot,
                        from_generated(&s.item),
                    ),
                    P::SetCursorItem(c) => inv.on_set_cursor(from_generated(&c.contents)),
                    P::SetPlayerInventory(p) => {
                        inv.on_set_player_inventory(p.slot_id, from_generated(&p.contents))
                    }
                    P::HeldItemSlot(h) => inv.on_held_slot(i8::try_from(h.slot).unwrap_or(-1)),
                    P::CraftProgressBar(c) => {
                        inv.on_window_property(c.window_id, c.property, c.value)
                    }
                    P::DeclareRecipes(d) => declare_recipes(inv, d),
                    P::Abilities(a) => inv.set_creative_modern(a.flags & 0x08 != 0),
                    P::Experience(e) => inv.set_experience_level_modern(e.level),
                    _ => {}
                }
            }

            /// Parses the kept `update_recipes` bytes (once; a packet that does not decode
            /// leaves the item sets empty, like a client that got none).
            pub(crate) fn ensure_recipes(inv: &mut Inventory) {
                if let Some(body) = inv.take_recipes_raw_modern()
                    && let Ok(d) = RawPacket::new(cb::id::DECLARE_RECIPES, body)
                        .decode_as::<cb::DeclareRecipes>()
                {
                    declare_recipes(inv, &d);
                }
            }

            fn declare_recipes(inv: &mut Inventory, d: &cb::DeclareRecipes) {
                use ghast_protocol::types::holder::HolderSet;
                let mut inputs = Some(Vec::new());
                for r in &d.stone_cutter_recipes {
                    match (&r.input, &mut inputs) {
                        (HolderSet::Ids(ids), Some(v)) => v.extend_from_slice(ids),
                        _ => inputs = None,
                    }
                }
                inv.on_recipes_modern(
                    d.recipes.iter().map(|r| (&*r.name, r.items.as_slice())),
                    inputs,
                );
            }

            /// Applies a window packet from its raw body ([`PACKET_IDS`]).
            pub(crate) fn apply_raw(
                inv: &mut Inventory,
                raw: &RawPacket,
            ) -> Result<(), DecodeError> {
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
                        inv.on_window_items_modern(window, state, items, carried);
                    }
                    cb::id::SET_SLOT => {
                        let window = r.read_varint()?;
                        let state = r.read_varint()?;
                        let s = r.read_i16()?;
                        let item = slot(&mut r)?;
                        inv.on_set_slot_modern(window, state, s, item);
                    }
                    cb::id::SET_CURSOR_ITEM => {
                        let item = slot(&mut r)?;
                        inv.on_set_cursor(item);
                    }
                    cb::id::SET_PLAYER_INVENTORY => {
                        let k = r.read_varint()?;
                        let item = slot(&mut r)?;
                        inv.on_set_player_inventory(k, item);
                    }
                    // decoded as their own type: no whole-protocol decoder is linked
                    cb::id::LOGIN => {
                        raw.decode_as::<cb::Login>()?;
                        inv.reset();
                    }
                    cb::id::RESPAWN => {
                        raw.decode_as::<cb::Respawn>()?;
                        inv.reset();
                    }
                    cb::id::OPEN_WINDOW => open_window(inv, &raw.decode_as::<cb::OpenWindow>()?),
                    cb::id::CLOSE_WINDOW => {
                        raw.decode_as::<cb::CloseWindow>()?;
                        inv.on_close_window();
                    }
                    cb::id::HELD_ITEM_SLOT => {
                        let h = raw.decode_as::<cb::HeldItemSlot>()?;
                        inv.on_held_slot(i8::try_from(h.slot).unwrap_or(-1));
                    }
                    cb::id::CRAFT_PROGRESS_BAR => {
                        let c = raw.decode_as::<cb::CraftProgressBar>()?;
                        inv.on_window_property(c.window_id, c.property, c.value);
                    }
                    // large and sent at every join: kept as bytes, parsed when a menu first
                    // needs its item sets (`ensure_recipes`)
                    cb::id::DECLARE_RECIPES => {
                        inv.set_recipes_raw_modern(bytes::Bytes::copy_from_slice(&raw.body));
                    }
                    cb::id::TAGS => {
                        inv.on_update_tags_modern(bytes::Bytes::copy_from_slice(&raw.body))
                    }
                    cb::id::TRADE_LIST => inv.on_merchant_offers_modern(&raw.body),
                    cb::id::ABILITIES => {
                        let a = raw.decode_as::<cb::Abilities>()?;
                        inv.set_creative_modern(a.flags & 0x08 != 0);
                    }
                    cb::id::EXPERIENCE => {
                        let e = raw.decode_as::<cb::Experience>()?;
                        inv.set_experience_level_modern(e.level);
                    }
                    _ => {}
                }
                Ok(())
            }

            fn open_window(inv: &mut Inventory, w: &cb::OpenWindow) {
                inv.on_open_screen_modern(
                    w.window_id,
                    w.inventory_type,
                    ghast_protocol::chat::nbt_to_plain_text(&w.window_title).into(),
                );
            }
        }
    };
}

#[cfg(feature = "v26_1")]
modern_packets!(m775, v26_1, p775, Mv::V26_1, t::SlotComponentType);
#[cfg(feature = "v26_2")]
modern_packets!(m776, v26_2, p776, Mv::V26_2, t::SlotComponentType::new);

#[cfg(modern)]
use crate::tables::Mv;

/// Clientbound modern play packet ids the inventory handles (the same in every modern version).
#[cfg(modern)]
#[cfg(feature = "v26_1")]
pub(crate) const PACKET_IDS_MODERN: &[i32] = m775::PACKET_IDS;
#[cfg(modern)]
#[cfg(not(feature = "v26_1"))]
pub(crate) const PACKET_IDS_MODERN: &[i32] = m776::PACKET_IDS;

#[cfg(all(feature = "v26_1", feature = "v26_2"))]
const _: () = {
    let (a, b) = (m775::PACKET_IDS, m776::PACKET_IDS);
    assert!(a.len() == b.len());
    let mut i = 0;
    while i < a.len() {
        assert!(
            a[i] == b[i],
            "modern inventory packet ids differ: split PACKET_IDS_MODERN"
        );
        i += 1;
    }
};

/// Clientbound 26.1 play packet ids [`Inventory::apply_raw_v26_1`] handles.
#[cfg(feature = "v26_1")]
pub const PACKET_IDS_26_1: &[i32] = m775::PACKET_IDS;

#[cfg(feature = "v26_1")]
pub(crate) use m775::hashed_slot as hashed_775;

#[cfg(feature = "v26_1")]
mod v26_1_api {
    use ghast_protocol::v26_1::play::clientbound::ClientboundPlayPacket;
    use ghast_protocol::v26_1::play::serverbound as sb;
    use ghast_protocol::{DecodeError, RawPacket};

    use super::m775;
    use crate::inventory::{ContainerClick, CreativePacket, Inventory, Outgoing};
    use crate::v26_1::HashRegistries;

    impl ContainerClick {
        /// The 26.1 `container_click` with hashed stacks (see [`crate::v26_1::hashed_slot`]).
        #[must_use]
        pub fn to_v26_1(&self, reg: &HashRegistries) -> sb::WindowClick {
            m775::click(self, reg)
        }
    }

    impl CreativePacket {
        /// The 26.1 serverbound packet: `swing` (main hand) or `set_creative_mode_slot` (the
        /// stack in the untrusted layout, components in type id order).
        #[must_use]
        pub fn to_v26_1(&self) -> sb::ServerboundPlayPacket {
            m775::creative(self)
        }
    }

    impl Outgoing {
        /// The 26.1 serverbound packet (`None` for the legacy-only variants, which a 26.1
        /// inventory never returns, and for [`Outgoing::Creative`], whose packets
        /// [`CreativePacket::to_v26_1`] encodes one by one).
        #[must_use]
        pub fn to_v26_1(&self, reg: &HashRegistries) -> Option<sb::ServerboundPlayPacket> {
            m775::outgoing(self, reg)
        }
    }

    impl Inventory {
        /// [`Outgoing::to_v26_1`] with this inventory's registries.
        #[must_use]
        pub fn encode_v26_1(&self, out: &Outgoing) -> Option<sb::ServerboundPlayPacket> {
            out.to_v26_1(self.registries_modern())
        }

        /// Applies a 26.1 clientbound play packet: `login` / `respawn`, `open_screen`,
        /// `container_close`, `container_set_content`, `container_set_slot`,
        /// `set_cursor_item`, `set_player_inventory`, `set_held_slot`, `container_set_data`.
        /// Slots are converted through their encoding (use [`Inventory::apply_raw_modern`] to
        /// avoid decoding components at all).
        pub fn apply_v26_1(&mut self, p: &ClientboundPlayPacket) {
            m775::apply(self, p);
        }

        /// The 26.1 name of [`Inventory::apply_raw_modern`] (a 26.1 inventory's packets).
        ///
        /// # Errors
        /// A malformed packet (nothing changed).
        pub fn apply_raw_v26_1(&mut self, raw: &RawPacket) -> Result<(), DecodeError> {
            m775::apply_raw(self, raw)
        }
    }
}

/// The typed 26.2 serverbound packets of the window intents (a client that writes 26.2 packets
/// itself; [`Inventory::encode_modern`] gives them as raw id + body).
#[cfg(feature = "v26_2")]
mod v26_2_api {
    use ghast_protocol::v26_2::play::serverbound as sb;

    use super::m776;
    use crate::inventory::{CreativePacket, Outgoing};
    use crate::modern::HashRegistries;

    impl CreativePacket {
        /// The 26.2 serverbound packet: `swing` (main hand) or `set_creative_mode_slot`.
        #[must_use]
        pub fn to_v26_2(&self) -> sb::ServerboundPlayPacket {
            m776::creative(self)
        }
    }

    impl Outgoing {
        /// The 26.2 serverbound packet (`None` for the legacy-only variants and `Creative`).
        #[must_use]
        pub fn to_v26_2(&self, reg: &HashRegistries) -> Option<sb::ServerboundPlayPacket> {
            m776::outgoing(self, reg)
        }
    }
}

#[cfg(modern)]
impl Inventory {
    /// Applies a modern window packet from its raw body (ids: [`crate::modern::packet_ids`]):
    /// slots are read with a structural skip and keep their components as bytes (nothing is
    /// decoded until an accessor asks). Dispatches on the inventory's version; does nothing
    /// for a legacy inventory.
    ///
    /// # Errors
    /// A malformed packet (nothing changed).
    pub fn apply_raw_modern(
        &mut self,
        raw: &ghast_protocol::RawPacket,
    ) -> Result<(), ghast_protocol::DecodeError> {
        match Mv::of(self.version()) {
            #[cfg(feature = "v26_1")]
            Some(Mv::V26_1) => m775::apply_raw(self, raw),
            #[cfg(feature = "v26_2")]
            Some(Mv::V26_2) => m776::apply_raw(self, raw),
            None => Ok(()),
        }
    }

    /// The serverbound packet of an [`Outgoing`] as raw id + body, for the inventory's version
    /// (`None` for a legacy inventory, the legacy-only variants and [`Outgoing::Creative`],
    /// whose packets [`Inventory::encode_creative_modern`] encodes one by one).
    #[must_use]
    pub fn encode_modern(&self, out: &Outgoing) -> Option<ghast_protocol::RawPacket> {
        match Mv::of(self.version())? {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => m775::raw(&m775::outgoing(out, self.registries_modern())?),
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => m776::raw(&m776::outgoing(out, self.registries_modern())?),
        }
    }

    /// One packet of [`Outgoing::Creative`] as raw id + body for the inventory's version:
    /// `swing` (main hand) or `set_creative_mode_slot` (`None` for a legacy inventory).
    #[must_use]
    pub fn encode_creative_modern(
        &self,
        p: &crate::CreativePacket,
    ) -> Option<ghast_protocol::RawPacket> {
        match Mv::of(self.version())? {
            #[cfg(feature = "v26_1")]
            Mv::V26_1 => m775::raw(&m775::creative(p)),
            #[cfg(feature = "v26_2")]
            Mv::V26_2 => m776::raw(&m776::creative(p)),
        }
    }

    /// Parses the kept `update_recipes` bytes (see the modern packet instances).
    pub(crate) fn ensure_recipes_modern(&mut self) {
        match Mv::of(self.version()) {
            #[cfg(feature = "v26_1")]
            Some(Mv::V26_1) => m775::ensure_recipes(self),
            #[cfg(feature = "v26_2")]
            Some(Mv::V26_2) => m776::ensure_recipes(self),
            None => {}
        }
    }
}
