//! 26.1 inventory against real server bytes.
//!
//! - `fixtures/clicks_26_1_2.bin.zz`: a session of `examples/live_clicks_26_1.rs` on a local
//!   vanilla 26.1.2 server (offline throwaway player): the enchantment registry, every inbound
//!   inventory packet and every click / close the tool sent. The server answered 34 of the 36
//!   clicks with no correction at all (it sends `container_set_slot` / `set_cursor_item` only
//!   for slots whose hash differs from its own result), so those clicks' predictions and
//!   hashes were right; the 2 others involve a bundle (sent unpredicted, and a
//!   `bundle_contents` hash ghast does not compute). Replaying the inbound packets, ghast must
//!   produce byte-identical click packets.
//! - `../../ghast-protocol/tests/fixtures/capture_26_1_2.bin.zz`: items with many component
//!   kinds given by the console; the lazy (raw) path and the generated-packet path must agree.
#![cfg(feature = "v26_1")]

use std::io::Read;

use bytes::{Bytes, BytesMut};
use ghast_inventory::v26_1::{HashRegistries, component, item_id};
use ghast_inventory::{Click, Inventory, ItemData, Outgoing, Version};
use ghast_protocol::codec::Reader;
use ghast_protocol::v26_1::configuration::clientbound::ClientboundConfigurationPacket;
use ghast_protocol::v26_1::play::clientbound::ClientboundPlayPacket;
use ghast_protocol::v26_1::play::serverbound::{self as sb, ServerboundPlayPacket};
use ghast_protocol::{PacketSet, RawPacket};

const CLICKS: &[u8] = include_bytes!("fixtures/clicks_26_1_2.bin.zz");
const HASHES: &[u8] = include_bytes!("fixtures/hashes_26_1_2.bin.zz");
const CAPTURE: &[u8] = include_bytes!("../../ghast-protocol/tests/fixtures/capture_26_1_2.bin.zz");

fn records(z: &[u8]) -> Vec<(u8, u8, RawPacket)> {
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(z)
        .read_to_end(&mut data)
        .unwrap();
    let data = Bytes::from(data);
    let mut r = Reader::new(&data);
    let mut out = Vec::new();
    while r.remaining() > 0 {
        let state = r.read_u8().unwrap();
        let dir = r.read_u8().unwrap();
        let id = r.read_varint().unwrap();
        let len = r.read_varint().unwrap() as usize;
        out.push((state, dir, RawPacket::new(id, r.read_bytes(len).unwrap())));
    }
    out
}

fn click_of(c: &sb::WindowClick) -> Click {
    let b = c.mouse_button as u8;
    match c.mode {
        0 => Click::Pickup {
            slot: c.slot,
            right: b == 1,
        },
        1 => Click::QuickMove {
            slot: c.slot,
            right: b == 1,
        },
        2 => Click::Swap {
            slot: c.slot,
            hotbar: b,
        },
        4 => Click::Drop {
            slot: c.slot,
            all: b == 1,
        },
        6 => Click::PickupAll { slot: c.slot },
        m => panic!("mode {m}"),
    }
}

/// Replays a live session: inbound packets in, the same clicks out; returns (clicks,
/// unpredicted, outcomes).
fn replay(fixture: &[u8]) -> (usize, usize, Vec<ghast_inventory::ClickOutcome>) {
    let mut inv = Inventory::new(Version::V26_1);
    // the sessions waited 400 - 700 ms after each click: one tick of silence settles it here
    inv.set_click_settle_ticks(1);
    let mut reg = HashRegistries::default();
    let mut clicks = 0;
    let mut unpredicted = 0;
    let mut outcomes = Vec::new();
    for (state, dir, raw) in records(fixture) {
        match (state, dir) {
            (3, 0) => {
                let ClientboundConfigurationPacket::RegistryData(r) =
                    ClientboundConfigurationPacket::decode_raw(&raw).unwrap()
                else {
                    panic!()
                };
                reg.add(&r.id, r.entries.iter().map(|e| &*e.key));
                inv.set_registries_26_1(reg.clone());
            }
            (4, 0) => inv.apply_raw_v26_1(&raw).unwrap(),
            (4, 1) => {
                let p = ServerboundPlayPacket::decode_raw(&raw).unwrap();
                inv.tick();
                outcomes.extend(inv.drain_events().clicks.iter().map(|c| c.outcome));
                let out = match &p {
                    ServerboundPlayPacket::WindowClick(c) => {
                        if !inv.gui_open() {
                            assert_eq!(inv.open_inventory(), Ok(None));
                            inv.tick();
                        }
                        clicks += 1;
                        let out = inv.click(click_of(c)).unwrap();
                        if let Outgoing::ContainerClick(cc) = &out
                            && !cc.predicted
                        {
                            unpredicted += 1;
                        }
                        out
                    }
                    ServerboundPlayPacket::CloseWindow(_) => inv.close().unwrap(),
                    other => panic!("{other:?}"),
                };
                let ours = inv.encode_v26_1(&out).unwrap();
                let mut a = BytesMut::new();
                let mut b = BytesMut::new();
                ours.encode_body(&mut a).unwrap();
                p.encode_body(&mut b).unwrap();
                assert_eq!(a, b, "click {clicks}: {ours:?}\nsent then: {p:?}");
            }
            _ => panic!(),
        }
    }
    inv.tick();
    outcomes.extend(inv.drain_events().clicks.iter().map(|c| c.outcome));
    (clicks, unpredicted, outcomes)
}

#[test]
fn replayed_session_sends_the_same_click_bytes() {
    let (clicks, unpredicted, outcomes) = replay(CLICKS);
    assert_eq!(clicks, 36);
    assert_eq!(unpredicted, 1);
    // every click resolves once: the 2 corrected bundle clicks as rejected, the rest accepted
    use ghast_inventory::ClickOutcome as O;
    assert_eq!(outcomes.len(), 36, "{outcomes:?}");
    assert_eq!(
        outcomes.iter().filter(|o| **o == O::Rejected).count(),
        2,
        "{outcomes:?}"
    );
}

/// `fixtures/hashes_26_1_2.bin.zz`: 117 stacks, one component kind each (plus removed-component
/// and combined cases), picked up and put back on a vanilla 26.1.2 server. The server sent no
/// correction for every stack whose components ghast hashes (84 component kinds); the stacks
/// that drew corrections carry exactly the kinds ghast does not hash (listed in the README).
/// Replaying the session reproduces every click byte for byte, hashes included.
#[test]
fn component_hashes_session_replays() {
    let (clicks, unpredicted, outcomes) = replay(HASHES);
    assert_eq!(clicks, 2 * 117);
    // a stack with a patched `equippable` is sent unpredicted (its slot rules are unknown)
    assert_eq!(unpredicted, 2);
    use ghast_inventory::ClickOutcome as O;
    let rejected = outcomes.iter().filter(|o| **o == O::Rejected).count();
    // 23 stacks with unhashed kinds (and potion custom_effects), corrected on both clicks,
    // plus the batch ends: the driver's `/clear` lands before the batch's last click settles
    assert!(
        (2 * 23..=2 * 23 + 4).contains(&rejected),
        "{rejected}: {outcomes:?}"
    );
}

#[test]
fn lazy_and_generated_paths_agree_on_captured_items() {
    let mut lazy = Inventory::new(Version::V26_1);
    let mut typed = Inventory::new(Version::V26_1);
    let mut seen_components = std::collections::BTreeSet::new();
    for (state, dir, raw) in records(CAPTURE) {
        if state != 4 || dir != 0 || !ghast_inventory::PACKET_IDS_26_1.contains(&raw.id) {
            continue;
        }
        lazy.apply_raw_v26_1(&raw).unwrap();
        typed.apply_v26_1(&ClientboundPlayPacket::decode_raw(&raw).unwrap());
        for i in 0..lazy.slot_count() {
            assert_eq!(lazy.slot(i), typed.slot(i), "slot {i}");
            if let Some(s) = lazy.slot(i)
                && let ItemData::Components(p) = &s.data()
            {
                for (t, _) in p.iter() {
                    seen_components.insert(t);
                }
                // every added component decodes
                assert_eq!(p.decode_all().unwrap().len(), p.added_len());
            }
        }
    }
    assert!(seen_components.len() >= 10, "{seen_components:?}");
    let v = Version::V26_1;
    // The capture's console gave an enchanted named sword with lore and damage.
    let sword = lazy
        .find(|s| s.item_name(v) == Some("diamond_sword"))
        .and_then(|i| lazy.slot(i as usize))
        .expect("sword");
    assert!(sword.custom_name(v).is_some());
    assert!(!sword.enchantments(v).is_empty());
    assert!(sword.damage_value(v) > 0);
    assert!(!sword.lore(v).is_empty());
    assert_eq!(sword.max_stack_size(v), 1);
}

#[test]
fn accessors_from_defaults_and_patches() {
    use ghast_protocol::types::slot::v775::ComponentPatch;
    let v = Version::V26_1;
    let pick = item_id("diamond_pickaxe").unwrap();
    let s = ghast_inventory::Stack::new(pick, 1, 0);
    assert_eq!(s.item_name(v), Some("diamond_pickaxe"));
    assert_eq!(s.max_damage(v), Some(1561));
    assert!(s.is_damageable(v));
    assert!(!s.unbreakable(v));
    let s = s.with_data(ItemData::Components(Box::new(ComponentPatch::from_parts(
        &[
            (component::DAMAGE, &[0x96, 0x01]),
            (component::UNBREAKABLE, &[]),
        ],
        &[],
    ))));
    assert_eq!(s.damage_value(v), 150);
    assert!(s.unbreakable(v));
    assert!(!s.is_damageable(v));
    // removing max_stack_size makes the default 1
    let dirt = ghast_inventory::Stack::new(item_id("dirt").unwrap(), 1, 0).with_data(
        ItemData::Components(Box::new(ComponentPatch::from_parts(
            &[],
            &[component::MAX_STACK_SIZE],
        ))),
    );
    assert_eq!(dirt.max_stack_size(v), 1);
}

#[test]
fn print_sizes() {
    eprintln!(
        "Stack {} ItemData {} ComponentPatch {} RawNbt {} Option<Stack> {}",
        size_of::<ghast_inventory::Stack>(),
        size_of::<ItemData>(),
        size_of::<ghast_protocol::types::slot::v775::ComponentPatch>(),
        size_of::<ghast_protocol::nbt::RawNbt>(),
        size_of::<Option<ghast_inventory::Stack>>()
    );
}
