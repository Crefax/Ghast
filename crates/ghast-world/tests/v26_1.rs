//! 26.1 world against real server bytes.
//!
//! Fixtures:
//! - `fixtures/world_26_1_2.bin.zz`: two sessions on a local vanilla 26.1.2 server (offline
//!   throwaway player, default-generated world, a test data pack adding the dimension
//!   `ghast:tall` with `min_y` -128 and height 512), reduced to the configuration registries and
//!   the world packets, with `level_chunk_with_light` kept for 3×3 columns at each place visited
//!   (overworld spawn surface, a cave/deepslate area at 288, -20, 288, the origin, the nether,
//!   `ghast:tall`). The second session made live block and section updates near the player
//!   (fills, stairs, chests, redstone wire, water, blocks at the lowest and highest rows).
//! - `data/probes_26_1_2.tsv.gz`: `record dimension x y z state` lines. Each one was confirmed
//!   by the server the fixture came from (`execute in <dimension> if block <x y z> <state>`
//!   printed "Test passed", ticks frozen); 1747 of 1747 passed. The test checks that the world
//!   holds that state just before record number `record` is applied.
//! - `../../ghast-protocol/tests/fixtures/capture_26_1_2.bin.zz`: the protocol crate's flat-world
//!   capture (every chunk must load).
//!
//! Independent of the probes, every chunk is compared block by block (and biome by biome) with
//! `ghast_protocol::types::chunk` (a separate decoder), and the raw-body path
//! ([`World::handle_raw`]) with the generated-packet path ([`World::handle`]).
#![cfg(feature = "v26_1")]

use std::collections::HashMap;
use std::io::Read;

use bytes::Bytes;
use ghast_data::v26_1::{BlockState, STATE_COUNT};
use ghast_protocol::codec::Reader;
use ghast_protocol::types::chunk::Sections;
use ghast_protocol::v26_1::configuration::clientbound::ClientboundConfigurationPacket;
use ghast_protocol::v26_1::play::clientbound::{self as cb, ClientboundPlayPacket};
use ghast_protocol::{PacketSet, RawPacket};
use ghast_world::v26_1::World;
use ghast_world::{Applied, WorldConfig};

const FIXTURE: &[u8] = include_bytes!("fixtures/world_26_1_2.bin.zz");
const PROTO_FIXTURE: &[u8] =
    include_bytes!("../../ghast-protocol/tests/fixtures/capture_26_1_2.bin.zz");
const PROBES: &[u8] = include_bytes!("data/probes_26_1_2.tsv.gz");

struct Record {
    state: u8,
    dir: u8,
    raw: RawPacket,
}

fn records(z: &[u8]) -> Vec<Record> {
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
        let body = r.read_bytes(len).unwrap();
        out.push(Record {
            state,
            dir,
            raw: RawPacket::new(id, body),
        });
    }
    out
}

struct Probe {
    dim: String,
    pos: [i32; 3],
    state: BlockState,
}

fn probes() -> HashMap<usize, Vec<Probe>> {
    let mut s = String::new();
    flate2::read::GzDecoder::new(PROBES)
        .read_to_string(&mut s)
        .unwrap();
    let mut m: HashMap<usize, Vec<Probe>> = HashMap::new();
    for l in s.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        let state = BlockState::parse(f[5]).unwrap_or_else(|| panic!("state {}", f[5]));
        assert_eq!(state.to_string(), f[5], "display round trip");
        m.entry(f[0].parse().unwrap()).or_default().push(Probe {
            dim: f[1].to_string(),
            pos: [
                f[2].parse().unwrap(),
                f[3].parse().unwrap(),
                f[4].parse().unwrap(),
            ],
            state,
        });
    }
    m
}

/// Compares a freshly loaded column with the protocol crate's decoder.
fn check_against_protocol_decoder(w: &World, p: &cb::MapChunk) {
    let min_y = w.min_y().unwrap();
    let col = w.column(p.x, p.z).expect("column loaded");
    let mut n = 0;
    for (j, s) in Sections::new(&p.chunk_data).enumerate() {
        let s = s.unwrap();
        if j >= col.sections().len() {
            break; // trailing data
        }
        n += 1;
        for i in 0..4096 {
            let (x, y, z) = (
                p.x * 16 + (i & 15) as i32,
                min_y + j as i32 * 16 + (i >> 8) as i32,
                p.z * 16 + ((i >> 4) & 15) as i32,
            );
            let want = s.blocks.get(i).unwrap();
            let got = w.block_state(x, y, z).unwrap();
            assert_eq!(
                i32::from(got.id()),
                want,
                "chunk {} {} section {j} index {i}",
                p.x,
                p.z
            );
        }
        let biomes = &col.biomes().unwrap()[j];
        for i in 0..64 {
            assert_eq!(i32::from(biomes.get(i)), s.biomes.get(i).unwrap());
        }
    }
    assert_eq!(n, col.sections().len());
}

fn same_world(a: &World, b: &World) {
    assert_eq!(a.columns().len(), b.columns().len());
    assert_eq!(a.dimension(), b.dimension());
    for (p, ca) in a.columns().iter() {
        let cb = b.column(p.x, p.z).expect("same columns");
        assert_eq!(ca.sections().len(), cb.sections().len());
        for (sa, sb) in ca.sections().iter().zip(cb.sections()) {
            for i in 0..4096 {
                assert_eq!(sa.get(i), sb.get(i));
            }
        }
        assert_eq!(ca.biomes(), cb.biomes());
        assert!(ca.block_entities().eq(cb.block_entities()));
    }
}

struct Replay {
    raw: World,
    typed: World,
    chunks: usize,
    probes_checked: usize,
    sections_by_bits: [usize; 6],
    max_heap_per_column: usize,
}

fn replay(fixture: &[u8], probes: &HashMap<usize, Vec<Probe>>) -> Replay {
    let config = WorldConfig::default().with_block_entities(true);
    let mut rp = Replay {
        raw: World::new(config),
        typed: World::new(config),
        chunks: 0,
        probes_checked: 0,
        sections_by_bits: [0; 6],
        max_heap_per_column: 0,
    };
    let recs = records(fixture);
    for (k, rec) in recs.iter().enumerate() {
        if let Some(ps) = probes.get(&(k + 1)) {
            for p in ps {
                assert_eq!(rp.raw.dimension().unwrap().name, p.dim);
                let [x, y, z] = p.pos;
                assert_eq!(
                    rp.raw.block_state(x, y, z),
                    Some(p.state),
                    "probe {} {x} {y} {z} before record {}",
                    p.dim,
                    k + 1
                );
                assert_eq!(rp.raw.view(x, z).block_state(x, y, z), Some(p.state));
                rp.probes_checked += 1;
            }
        }
        if rec.dir != 0 {
            continue;
        }
        if rec.state == 3 {
            if let Ok(ClientboundConfigurationPacket::RegistryData(p)) =
                ClientboundConfigurationPacket::decode_raw(&rec.raw)
            {
                rp.raw.apply_registry_data(&p);
                rp.typed.apply_registry_data(&p);
            }
            continue;
        }
        if rec.state != 4 {
            continue;
        }
        let a = rp.raw.handle_raw(&rec.raw).expect("raw path");
        let Ok(p) = ClientboundPlayPacket::decode_raw(&rec.raw) else {
            continue;
        };
        let b = rp.typed.handle(&p).expect("typed path");
        assert_eq!(a, b, "{}", p.name());
        if let ClientboundPlayPacket::MapChunk(c) = &p {
            assert_eq!(a, Applied::Changed, "chunk {} {} not loaded", c.x, c.z);
            rp.chunks += 1;
            check_against_protocol_decoder(&rp.raw, c);
            let col = rp.raw.column(c.x, c.z).unwrap();
            for s in col.sections() {
                let k = match s.bits() {
                    0 => 0,
                    1 => 1,
                    2 => 2,
                    4 => 3,
                    8 => 4,
                    _ => 5,
                };
                rp.sections_by_bits[k] += 1;
            }
            rp.max_heap_per_column = rp.max_heap_per_column.max(col.heap_bytes());
        }
    }
    if let Some(ps) = probes.get(&(recs.len() + 1)) {
        for p in ps {
            let [x, y, z] = p.pos;
            assert_eq!(
                rp.raw.block_state(x, y, z),
                Some(p.state),
                "final probe {x} {y} {z}"
            );
            rp.probes_checked += 1;
        }
    }
    same_world(&rp.raw, &rp.typed);
    rp
}

#[test]
fn default_world_matches_the_server() {
    let probes = probes();
    let total: usize = probes.values().map(Vec::len).sum();
    let rp = replay(FIXTURE, &probes);
    assert_eq!(rp.chunks, 63);
    assert_eq!(rp.probes_checked, total);
    assert_eq!(total, 1747);
    eprintln!(
        "sections by width (uniform, 1, 2, 4, 8, direct bits): {:?}; largest column {} B",
        rp.sections_by_bits, rp.max_heap_per_column
    );
}

#[test]
fn dimension_heights_come_from_the_registry() {
    let recs = records(FIXTURE);
    let mut w = World::new(WorldConfig::default());
    let mut seen = Vec::new();
    for rec in &recs {
        if rec.dir != 0 {
            continue;
        }
        if rec.state == 3 {
            if let Ok(ClientboundConfigurationPacket::RegistryData(p)) =
                ClientboundConfigurationPacket::decode_raw(&rec.raw)
            {
                w.apply_registry_data(&p);
            }
            continue;
        }
        if rec.state == 4 && (rec.raw.id == cb::id::LOGIN || rec.raw.id == cb::id::RESPAWN) {
            w.handle_raw(&rec.raw).unwrap();
            let d = w.dimension().unwrap();
            seen.push((
                d.name.clone(),
                d.kind.min_y,
                d.kind.height,
                d.kind.fast_lava,
                d.kind.nbt.is_some(),
            ));
        }
    }
    seen.dedup();
    assert!(
        seen.contains(&("minecraft:overworld".into(), -64, 384, false, false)),
        "{seen:?}"
    );
    assert!(
        seen.contains(&("minecraft:the_nether".into(), 0, 256, true, false)),
        "{seen:?}"
    );
    // Not in a known pack: the server sent its data, the world read it.
    assert!(
        seen.contains(&("ghast:tall".into(), -128, 512, false, true)),
        "{seen:?}"
    );
    assert!(w.registries().biome_count > 50);
}

#[test]
fn tall_dimension_extremes() {
    // The first session set gold blocks at the lowest and highest rows of ghast:tall while the
    // player was there (block_update packets): find them in the replay.
    let recs = records(FIXTURE);
    let mut w = World::new(WorldConfig::default());
    let gold = BlockState::parse("gold_block").unwrap();
    let mut found = [false; 2];
    for rec in &recs {
        if rec.dir != 0 {
            continue;
        }
        if rec.state == 3 {
            if let Ok(ClientboundConfigurationPacket::RegistryData(p)) =
                ClientboundConfigurationPacket::decode_raw(&rec.raw)
            {
                w.apply_registry_data(&p);
            }
            continue;
        }
        if rec.state != 4 {
            continue;
        }
        w.handle_raw(&rec.raw).unwrap();
        if w.dimension().is_some_and(|d| d.name == "ghast:tall") {
            found[0] |= w.block_state(1, -127, 1) == Some(gold);
            found[1] |= w.block_state(1, 383, 1) == Some(gold);
            // Outside the height: void air, not unknown.
            if w.is_column_loaded(0, 0) {
                assert_eq!(
                    w.block_state(1, -129, 1).map(|s| s.block().name()),
                    Some("void_air")
                );
                assert_eq!(
                    w.block_state(1, 384, 1).map(|s| s.block().name()),
                    Some("void_air")
                );
            }
        }
    }
    assert_eq!(found, [true, true]);
}

#[test]
fn protocol_fixture_flat_world_loads() {
    let rp = replay(PROTO_FIXTURE, &HashMap::new());
    assert!(rp.chunks > 50, "{}", rp.chunks);
    assert!(rp.raw.columns().len() > 20);
}

#[test]
fn unloaded_is_not_air() {
    let rp = replay(FIXTURE, &HashMap::new());
    let w = &rp.raw;
    assert_eq!(w.block_state(100_000, 64, 100_000), None);
    let v = w.view(0, 0);
    assert_eq!(v.block_state(100_000, 64, 100_000), None);
    assert!(!v.is_column_loaded(6250, 6250));
    // Views agree with the world everywhere, inside and outside their 3×3 snapshot.
    for (p, _) in w.columns().iter() {
        for i in 0..200 {
            let (x, z) = (p.x * 16 + (i * 7) % 16, p.z * 16 + (i * 11) % 16);
            let y = -64 + (i * 37) % 384;
            assert_eq!(v.block_state(x, y, z), w.block_state(x, y, z));
            assert_eq!(w.view(x, z).block_state(x, y, z), w.block_state(x, y, z));
        }
    }
}

/// The 26.1 block state ids of `ghast-data` against Mojang's report (`--reports`
/// `blocks.json`, in the git-ignored `.cache/mojang/26.1.2/`): same id space, and every id
/// names the same block and property values. Skipped when the report is not cached.
#[test]
fn state_ids_match_the_mojang_report() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.cache/mojang/26.1.2/reports/reports/blocks.json"
    );
    let Ok(text) = std::fs::read_to_string(path) else {
        eprintln!("skipped: {path} not present");
        return;
    };
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut n = 0;
    let mut mislabelled = std::collections::BTreeSet::new();
    for (name, b) in v.as_object().unwrap() {
        for s in b["states"].as_array().unwrap() {
            let id = s["id"].as_u64().unwrap() as u16;
            let st = BlockState::from_id(id).unwrap_or_else(|| panic!("{name} {id} out of range"));
            assert_eq!(format!("minecraft:{}", st.block().name()), *name, "id {id}");
            if let Some(props) = s["properties"].as_object() {
                for (k, val) in props {
                    let p = ghast_data::v26_1::Property::from_name(k).unwrap();
                    let got = st.get(p);
                    let want = val.as_str().unwrap();
                    if got != Some(want) {
                        mislabelled.insert(k.clone());
                        // Known ghast-data bug: same index, value labelled one lower.
                        let gi = st.get_index(p).unwrap();
                        let wi = b["properties"][k]
                            .as_array()
                            .unwrap()
                            .iter()
                            .position(|v| v == val)
                            .unwrap();
                        assert_eq!(gi, wi, "{name} id {id} {k}: value index differs");
                    }
                }
            }
            n += 1;
        }
    }
    assert_eq!(n, STATE_COUNT);
    // ghast-data's v26_1 tables label integer properties that start at 1 from 0 (reported to the
    // ghast-data owner, 2026-11-11). When this assertion fails, the labels were fixed: replace
    // it with `assert!(mislabelled.is_empty())`.
    let known = [
        "candles",
        "delay",
        "distance",
        "eggs",
        "flower_amount",
        "layers",
        "level",
        "pickles",
        "segment_amount",
    ];
    assert_eq!(
        mislabelled.iter().map(String::as_str).collect::<Vec<_>>(),
        known
    );
}
