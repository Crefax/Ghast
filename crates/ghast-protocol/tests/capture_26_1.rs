//! Decodes real bytes captured from a vanilla 26.1.2 server (protocol 775): an offline
//! login, the configuration state and 40 s of play on a flat world, recorded by
//! `examples/login_26_1.rs` (see there for the record format). During the capture the
//! server console gave the throwaway player items with many data components (enchanted
//! named sword with lore and damage, potion, dyed armour, fireworks, written book, banner,
//! player head), changed blocks (`setblock`, `fill`), and sent chat, a title, a
//! scoreboard with a styled number format, a team, a boss bar, effects, particles,
//! sounds, weather, time, a teleport, a game-mode change and entity spawns.
//!
//! Every packet must decode strictly (no trailing bytes) with the generated types and
//! re-encode to the identical bytes; chunk sections must parse completely.

#![cfg(feature = "v26_1")]

use std::collections::BTreeMap;
use std::io::Read;

use bytes::Bytes;
use ghast_protocol::codec::Reader;
use ghast_protocol::types::chunk::{Palette, Sections};
use ghast_protocol::v26_1::{configuration, handshaking, login, play, status};
use ghast_protocol::{PacketSet, RawPacket};

const FIXTURE: &[u8] = include_bytes!("fixtures/capture_26_1_2.bin.zz");

struct Record {
    state: u8,
    dir: u8,
    raw: RawPacket,
}

fn records() -> Vec<Record> {
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(FIXTURE)
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
        let body = r.read_bytes(len).unwrap();
        out.push(Record {
            state,
            dir,
            raw: RawPacket::new(id, body),
        });
    }
    out
}

/// Strict decode, then re-encode and compare bytes; returns the packet name.
fn check<S: PacketSet + std::fmt::Debug>(raw: &RawPacket) -> (&'static str, S) {
    let p = S::decode_raw(raw).unwrap_or_else(|e| {
        panic!(
            "{:?} {:#04x} ({:?}): {e}\n{:02x?}",
            S::STATE,
            raw.id,
            S::name_of(raw.id),
            &raw.body[..raw.body.len().min(128)]
        )
    });
    let mut w = bytes::BytesMut::new();
    p.encode_body(&mut w).unwrap();
    assert_eq!(
        &w[..],
        &raw.body[..],
        "re-encoding {} changed the bytes",
        p.name()
    );
    (p.name(), p)
}

#[test]
fn every_captured_packet_decodes_and_reencodes() {
    let recs = records();
    assert!(recs.len() > 50, "fixture too small: {}", recs.len());
    let mut counts: BTreeMap<(u8, u8, &'static str), usize> = BTreeMap::new();
    let mut chunks = 0;
    let mut sections = 0;
    let mut components = 0;
    for rec in &recs {
        let name = match (rec.state, rec.dir) {
            (0, 1) => check::<handshaking::serverbound::ServerboundHandshakingPacket>(&rec.raw).0,
            (1, 0) => check::<status::clientbound::ClientboundStatusPacket>(&rec.raw).0,
            (1, 1) => check::<status::serverbound::ServerboundStatusPacket>(&rec.raw).0,
            (2, 0) => check::<login::clientbound::ClientboundLoginPacket>(&rec.raw).0,
            (2, 1) => check::<login::serverbound::ServerboundLoginPacket>(&rec.raw).0,
            (3, 0) => {
                check::<configuration::clientbound::ClientboundConfigurationPacket>(&rec.raw).0
            }
            (3, 1) => {
                check::<configuration::serverbound::ServerboundConfigurationPacket>(&rec.raw).0
            }
            (4, 0) => {
                let (n, p) = check::<play::clientbound::ClientboundPlayPacket>(&rec.raw);
                if let play::clientbound::ClientboundPlayPacket::SetSlot(s) = &p
                    && let Some(body) = &s.item.body
                {
                    assert_eq!(body.components.len() as i32, body.added_component_count);
                    components += body.components.len();
                }
                if let play::clientbound::ClientboundPlayPacket::MapChunk(c) = p {
                    chunks += 1;
                    let mut nonempty = 0;
                    for s in Sections::new(&c.chunk_data) {
                        let s = s.unwrap_or_else(|e| panic!("chunk {} {}: {e}", c.x, c.z));
                        sections += 1;
                        if s.block_count > 0 {
                            nonempty += 1;
                            // Every non-empty flat-world section has real block ids.
                            assert!(!matches!(s.blocks.palette, Palette::Single(0)));
                        }
                        for i in [0, 4095] {
                            assert!(s.blocks.get(i).is_some());
                        }
                    }
                    assert!(nonempty >= 1, "flat chunk without blocks");
                }
                n
            }
            (4, 1) => check::<play::serverbound::ServerboundPlayPacket>(&rec.raw).0,
            other => panic!("bad record {other:?}"),
        };
        *counts.entry((rec.state, rec.dir, name)).or_default() += 1;
    }
    for ((s, d, n), c) in &counts {
        eprintln!("{s} {d} {n}: {c}");
    }
    let has = |s: u8, d: u8, n: &str| counts.keys().any(|k| k.0 == s && k.1 == d && k.2 == n);
    for (s, d, n) in [
        (2, 0, "success"),
        (3, 0, "select_known_packs"),
        (3, 0, "registry_data"),
        (3, 0, "tags"),
        (3, 0, "finish_configuration"),
        (4, 0, "login"),
        (4, 0, "position"),
        (4, 0, "chunk_batch_start"),
        (4, 0, "chunk_batch_finished"),
        (4, 1, "teleport_confirm"),
        (4, 0, "set_slot"),
        (4, 0, "multi_block_change"),
        (4, 0, "block_change"),
        (4, 0, "scoreboard_objective"),
        (4, 0, "teams"),
        (4, 0, "boss_bar"),
        (4, 0, "system_chat"),
        (4, 0, "keep_alive"),
        (4, 1, "keep_alive"),
    ] {
        assert!(has(s, d, n), "fixture has no {n}");
    }
    assert!(
        chunks > 0 && sections >= chunks * 16,
        "{chunks} chunks, {sections} sections"
    );
    assert!(components >= 8, "only {components} item components decoded");
}
