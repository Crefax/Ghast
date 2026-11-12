//! Inventory and container windows for the ghast Minecraft Java Edition client library
//! (1.8.9, 1.12.2, 1.16.5, 26.1 and 26.2).
//!
//! [`Inventory`] holds what the vanilla client holds — the player inventory, the cursor stack
//! and the open window — fed by the server's window packets, and turns click intents into
//! `window_click` packets with **vanilla's client-side prediction**: the new slot contents, the
//! "clicked item" field (what vanilla's `onSlotClick` returns; the server rejects the click when
//! its own result differs) and the per-window action number.
//!
//! - **One click in flight.** A click stays pending until the server's confirm (`transaction`)
//!   with its action number arrives. A rejected click makes the server resend the window;
//!   clicks wait for that resync. [`Inventory::on_transaction`] also produces vanilla's
//!   answer to rejected confirms — exactly once per confirm (route every confirm through it).
//! - **Exact or refused.** Clicks are simulated on a scratch copy; when the outcome depends on
//!   data this crate does not model (recipes, furnace fuel, brewing, trading, …) the click is
//!   refused with [`ClickError::Unpredictable`] instead of guessing, so the client state never
//!   silently diverges from the server's (after an accepted click the server sends no
//!   corrections — it assumes the client predicted right).
//! - **Lazy NBT.** Stacks keep their NBT as raw bytes ([`ghast_protocol::nbt::RawNbt`]);
//!   `window_items` is applied without parsing any NBT. Stack comparisons compare the bytes
//!   and only parse when they differ (vanilla compares trees).
//! - **26.1 / 26.2** have no confirms: the server applies every click and answers only the slots
//!   whose hash (vanilla `HashedStack`) differs from its own result. A modern click is never refused:
//!   one ghast cannot predict is sent unpredicted ([`ContainerClick::predicted`]) and the
//!   server's corrections bring the result. Data components stay raw too
//!   ([`ItemData::Components`]); `window_items` is applied with a structural skip
//!   ([`Inventory::apply_raw_modern`]). Item facts come from Mojang's reports ([`modern`]; one
//!   implementation over per-version tables).
//! - **Item accessors** ([`Stack::item_name`], [`Stack::max_stack_size`],
//!   [`Stack::damage_value`], [`Stack::custom_name`], [`Stack::enchantments`], …) answer from NBT
//!   or components, decoding only what they read.
//! - **Pure.** No I/O, no clock, no async: packets in, packets out. The client writes the
//!   returned packets immediately, in call order, before the tick's movement packet, and calls
//!   [`Inventory::tick`] once per tick.
//!
//! ```
//! use ghast_inventory::{Click, Inventory, Outgoing, Stack, Version};
//!
//! let mut inv = Inventory::new(Version::V1_8).unwrap();
//! // The server sends window 0 (45 slots): 64 stone in hotbar slot 0 (window slot 36).
//! let mut items = vec![None; 45];
//! items[36] = Some(Stack::new(1, 64, 0));
//! inv.on_window_items(0, items);
//!
//! inv.tick();
//! assert_eq!(inv.open_inventory(), Ok(Some(Outgoing::OpenInventory))); // 1.8: C16
//! inv.tick();
//! let Ok(Outgoing::Click(c)) = inv.click(Click::Pickup { slot: 36, right: true }) else { panic!() };
//! assert_eq!((c.action, c.item.as_ref().unwrap().count()), (1, 64)); // carries the slot before the click
//! assert_eq!(inv.cursor().unwrap().count(), 32);
//! assert_eq!(inv.window0_slot(36).unwrap().count(), 32);
//! assert!(inv.pending());
//! assert_eq!(inv.on_transaction(0, 1, true), None); // accepted: nothing to answer
//! assert!(!inv.pending());
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, unnameable_types)]

mod crafting;
#[cfg(modern)]
mod data26;
#[cfg(modern)]
mod ench_value;
mod inventory;
mod item;
#[cfg(modern)]
mod item26;
#[cfg(modern)]
pub mod modern;
mod packets;
mod rules;
mod sim;
#[cfg(modern)]
mod sim26;
mod stack;
#[cfg(modern)]
mod tables;
#[cfg(feature = "v26_1")]
pub mod v26_1;
mod window;

pub use crafting::RecipeBook;
pub use inventory::{
    Click, ClickError, ClickOutcome, ClickPacket, ClickResult, ContainerClick, CreativePacket,
    DEFAULT_CLICK_SETTLE_TICKS, DEFAULT_CLICK_TIMEOUT_TICKS, DragKind, DragStage, Events,
    Inventory, Outgoing, Window,
};
pub use item::{Enchantment, EnchantmentId, ItemText};
#[cfg(feature = "v26_1")]
pub use packets::PACKET_IDS_26_1;
pub use rules::{Version, VersionNotEnabled};
pub use sim::OUTSIDE;
pub use stack::{
    ItemData, Stack, from_legacy_slot, from_modern_slot, nbt_eq, stacks_equal, to_legacy_slot,
    to_modern_slot,
};
pub use window::{SlotKind, WindowKind};
