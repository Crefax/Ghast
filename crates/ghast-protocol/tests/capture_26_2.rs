//! Decodes real bytes captured from a vanilla 26.2 server (protocol 776): an offline
//! login, the configuration state and 55 s of play on a flat world, recorded by
//! `examples/login_26_2.rs` (see there for the record format). During the capture the
//! server console gave the throwaway player items with many data components (enchanted
//! named sword with lore and damage, potion, dyed armour, fireworks, written book, banner,
//! player head), changed blocks (`setblock`, `fill`), and sent chat, a title, a
//! scoreboard with a styled number format, a team, a boss bar, effects, particles,
//! sounds, weather, time, a teleport, a game-mode change and entity spawns.
//!
//! Every packet must decode strictly (no trailing bytes) with the generated types and
//! re-encode to the identical bytes; chunk sections must parse completely.
//!
//! The capture also exercises what changed from 775 to 776 (`codegen/protocol/delta/776.json`):
//! the session id after the login profile, `onlineMode` in the play login, the new team
//! parameter order with an optional team colour (set to red, then reset), the geyser
//! particles with their data, an item carrying `sulfur_cube_content`, new attributes, and
//! the shifted registries (new blocks, items, entities, sounds).

#![cfg(feature = "v26_2")]

use std::collections::BTreeMap;
use std::io::Read;

use bytes::Bytes;
use ghast_protocol::codec::Reader;
use ghast_protocol::types::chunk::{Palette, Sections};
use ghast_protocol::v26_2::{configuration, handshaking, login, play, status};
use ghast_protocol::{PacketSet, RawPacket};

const FIXTURE: &[u8] = include_bytes!("fixtures/capture_26_2.bin.zz");

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
    let mut delta = Delta::default();
    for rec in &recs {
        let name = match (rec.state, rec.dir) {
            (0, 1) => check::<handshaking::serverbound::ServerboundHandshakingPacket>(&rec.raw).0,
            (1, 0) => check::<status::clientbound::ClientboundStatusPacket>(&rec.raw).0,
            (1, 1) => check::<status::serverbound::ServerboundStatusPacket>(&rec.raw).0,
            (2, 0) => {
                let (n, p) = check::<login::clientbound::ClientboundLoginPacket>(&rec.raw);
                if let login::clientbound::ClientboundLoginPacket::Success(s) = &p {
                    delta.session_id = s.session_id != s.uuid;
                }
                n
            }
            (2, 1) => check::<login::serverbound::ServerboundLoginPacket>(&rec.raw).0,
            (3, 0) => {
                check::<configuration::clientbound::ClientboundConfigurationPacket>(&rec.raw).0
            }
            (3, 1) => {
                check::<configuration::serverbound::ServerboundConfigurationPacket>(&rec.raw).0
            }
            (4, 0) => {
                let (n, p) = check::<play::clientbound::ClientboundPlayPacket>(&rec.raw);
                delta.see(&p);
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
    delta.assert_complete();
}

/// What the capture must show of the 775 -> 776 changes.
#[derive(Default, Debug)]
struct Delta {
    session_id: bool,
    login_offline: bool,
    team_red: bool,
    team_reset: bool,
    geyser: bool,
    geyser_base: bool,
    sulfur_component: bool,
    new_attributes: bool,
}

impl Delta {
    fn see(&mut self, p: &play::clientbound::ClientboundPlayPacket) {
        use ghast_protocol::defs::p776::types::{
            ParticleData, ParticleType, Slot, SlotComponentType,
        };
        use play::clientbound::ClientboundPlayPacket as P;
        let slot = |s: &Slot| {
            s.body.as_ref().is_some_and(|b| {
                b.components
                    .iter()
                    .any(|c| c.r#type == SlotComponentType::SULFUR_CUBE_CONTENT)
            })
        };
        match p {
            P::Login(l) => self.login_offline = !l.online_mode,
            P::Teams(t) => {
                if let Some(b) = &t.body {
                    // TeamColor.RED has id 12 (the ChatFormatting colour order).
                    self.team_red |= b.color == Some(12);
                    self.team_reset |= t.team.as_ref() as &str == "plain" && b.color.is_none();
                }
            }
            P::WorldParticles(w) => match &w.particle.data {
                ParticleData::Geyser(g) => {
                    assert_eq!(w.particle.r#type, ParticleType::GEYSER);
                    assert_eq!(g.water_blocks, 3);
                    self.geyser = true;
                }
                ParticleData::GeyserBase(g) => {
                    assert_eq!(w.particle.r#type, ParticleType::GEYSER_BASE);
                    assert_eq!((g.water_blocks, g.burst_impulse_base), (3, 0.5));
                    self.geyser_base = true;
                }
                _ => {}
            },
            P::SetSlot(s) => self.sulfur_component |= slot(&s.item),
            P::WindowItems(w) => self.sulfur_component |= w.items.iter().any(slot),
            P::EntityUpdateAttributes(a) => {
                // New in 776 and set by the console (ids renumbered: 9 was burning_time).
                use play::clientbound::EntityUpdateAttributesPropertiesKey as K;
                let has = |k: K, v: f64| a.properties.iter().any(|p| p.key == k && p.value == v);
                self.new_attributes |= has(K::BOUNCINESS, 0.5) && has(K::FRICTION_MODIFIER, 1.2);
            }
            _ => {}
        }
    }

    fn assert_complete(&self) {
        let Delta {
            session_id,
            login_offline,
            team_red,
            team_reset,
            geyser,
            geyser_base,
            sulfur_component,
            new_attributes,
        } = *self;
        assert!(
            session_id
                && login_offline
                && team_red
                && team_reset
                && geyser
                && geyser_base
                && sulfur_component
                && new_attributes,
            "{self:?}"
        );
    }
}
