//! Replays the live 26.2 menu session (`fixtures/menus_26_2.bin.zz`: `examples/live_clicks_26_2.rs`
//! with `MENUS=1` on a vanilla 26.2 server, 2026-11-12, offline throwaway player): the
//! configuration registries and tags, every inventory packet the server sent, and every click /
//! screen action ghast sent. Fed the same inbound packets and asked for the same clicks, ghast
//! must send the same bytes. The live run predicted all 69 clicks and the server corrected none
//! of them, nor any of the three screen actions (anvil rename, two trade selections).

#![cfg(feature = "v26_2")]

use std::io::Read;

use bytes::Bytes;
use ghast_inventory::modern::HashRegistries;
use ghast_inventory::{Click, DragKind, DragStage, Inventory, Outgoing, Version};
use ghast_protocol::codec::Reader;
use ghast_protocol::v26_2::configuration::clientbound::ClientboundConfigurationPacket;
use ghast_protocol::v26_2::play::serverbound::{self as sb, ServerboundPlayPacket};
use ghast_protocol::{PacketSet, RawPacket};

const MENUS: &[u8] = include_bytes!("fixtures/menus_26_2.bin.zz");

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
    let (slot, b) = (c.slot, c.mouse_button as u8);
    match c.mode {
        0 => Click::Pickup {
            slot,
            right: b == 1,
        },
        1 => Click::QuickMove {
            slot,
            right: b == 1,
        },
        2 => Click::Swap { slot, hotbar: b },
        3 => Click::Clone { slot },
        4 => Click::Drop { slot, all: b == 1 },
        5 => Click::QuickCraft {
            slot,
            stage: match b & 3 {
                0 => DragStage::Start,
                1 => DragStage::Slot,
                _ => DragStage::End,
            },
            kind: match (b >> 2) & 3 {
                0 => DragKind::Split,
                1 => DragKind::One,
                _ => DragKind::Clone,
            },
        },
        6 => Click::PickupAll { slot },
        m => panic!("mode {m}"),
    }
}

#[test]
fn menus_session_replays_26_2() {
    let mut inv = Inventory::new(Version::V26_2).unwrap();
    inv.set_click_settle_ticks(1);
    let mut reg = HashRegistries::default();
    let (mut clicks, mut actions, mut unpredicted) = (0, 0, 0);
    for (state, dir, raw) in records(MENUS) {
        match (state, dir) {
            (3, 0) => match ClientboundConfigurationPacket::decode_raw(&raw).unwrap() {
                ClientboundConfigurationPacket::RegistryData(r) => {
                    reg.add(&r.id, r.entries.iter().map(|e| &*e.key));
                    inv.set_registries_modern(reg.clone());
                    inv.on_registry_data_modern(raw.body.clone());
                }
                ClientboundConfigurationPacket::Tags(_) => {
                    inv.on_update_tags_modern(raw.body.clone())
                }
                other => panic!("{other:?}"),
            },
            (4, 0) => inv.apply_raw_modern(&raw).unwrap(),
            (4, 1) => {
                let p = ServerboundPlayPacket::decode_raw(&raw).unwrap();
                inv.tick();
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
                    ServerboundPlayPacket::NameItem(n) => {
                        actions += 1;
                        inv.anvil_rename_modern(&n.name).unwrap().expect("a rename")
                    }
                    ServerboundPlayPacket::SelectTrade(t) => {
                        actions += 1;
                        inv.select_trade_modern(t.slot).unwrap()
                    }
                    other => panic!("{other:?}"),
                };
                let ours = inv.encode_modern(&out).unwrap();
                assert_eq!(ours.id, raw.id, "packet {}", p.name());
                assert_eq!(ours.body, raw.body, "click {clicks}: {p:?}");
            }
            _ => panic!("record state {state} dir {dir}"),
        }
    }
    eprintln!("26.2 menus session: {clicks} clicks, {actions} screen actions");
    assert_eq!(clicks, 69);
    assert_eq!(actions, 3);
    assert_eq!(unpredicted, 0);
}
