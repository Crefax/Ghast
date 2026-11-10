//! Lab tool: replays a 26.1 capture (record format of `ghast-protocol/examples/login_26_1.rs`)
//! into a [`ghast_world::v26_1::World`] and writes block probes for a live server check.
//!
//! ```text
//! cargo run --release -p ghast-world --example probe_26_1 -- capture.bin[.zz] probes.tsv [per_column]
//! ```
//!
//! Before a column leaves the world (`forget_level_chunk`, a dimension change, the end of the
//! capture) `per_column` positions are sampled from it (half uniform in the column, half among
//! blocks that are not air, stone, deepslate, water or bedrock). Each line of `probes.tsv` is
//! `dimension x y z state`; `execute in <dimension> if block <x y z> <state>` must print
//! "Test passed" on the server the capture came from (ticks frozen).

use std::io::{Read, Write};

use bytes::Bytes;
use ghast_protocol::codec::Reader;
use ghast_protocol::v26_1::configuration::clientbound::ClientboundConfigurationPacket;
use ghast_protocol::v26_1::play::clientbound::{self as cb, ClientboundPlayPacket};
use ghast_protocol::{PacketSet, RawPacket};
use ghast_world::WorldConfig;
use ghast_world::v26_1::World;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn sample_column(
    rec: usize,
    w: &World,
    cx: i32,
    cz: i32,
    per: usize,
    rng: &mut Rng,
    out: &mut Vec<String>,
) {
    let Some(d) = w.dimension() else { return };
    let (min_y, h) = (d.kind.min_y, d.kind.height);
    let dim = d.name.clone();
    let boring = |s: &str| {
        s == "minecraft:air"
            || s == "minecraft:stone"
            || s.starts_with("minecraft:deepslate[")
            || s.starts_with("minecraft:water[")
            || s == "minecraft:bedrock"
            || s == "minecraft:netherrack"
    };
    let mut uniform = 0;
    let mut tries = 0;
    let mut interesting = 0;
    while (uniform < per / 2 || interesting < per - per / 2) && tries < 20_000 {
        tries += 1;
        let x = cx * 16 + rng.below(16) as i32;
        let z = cz * 16 + rng.below(16) as i32;
        let y = min_y + rng.below(h as u64) as i32;
        let s = w.block_state(x, y, z).unwrap().to_string();
        if uniform < per / 2 {
            uniform += 1;
        } else if !boring(&s) {
            interesting += 1;
        } else {
            continue;
        }
        out.push(format!("{rec}\t{dim}\t{x}\t{y}\t{z}\t{s}"));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let input = std::fs::read(&args[0]).unwrap();
    let mut data = Vec::new();
    if flate2::read::ZlibDecoder::new(&input[..])
        .read_to_end(&mut data)
        .is_err()
    {
        data = input;
    }
    let per: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(6);
    let data = Bytes::from(data);
    let mut r = Reader::new(&data);
    let mut w = World::new(WorldConfig::default().with_block_entities(true));
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut out = Vec::new();
    let mut chunks = 0;
    let mut errors = 0;
    let mut decode_ns = 0u128;
    let mut peak = (0usize, 0usize);
    let mut rec = 0usize;
    while r.remaining() > 0 {
        rec += 1;
        let state = r.read_u8().unwrap();
        let dir = r.read_u8().unwrap();
        let id = r.read_varint().unwrap();
        let len = r.read_varint().unwrap() as usize;
        let body = r.read_bytes(len).unwrap();
        if dir != 0 {
            continue;
        }
        let raw = RawPacket::new(id, body);
        if state == 3 {
            if let Ok(ClientboundConfigurationPacket::RegistryData(p)) =
                ClientboundConfigurationPacket::decode_raw(&raw)
            {
                w.apply_registry_data(&p);
            }
            continue;
        }
        if state != 4 {
            continue;
        }
        match id {
            cb::id::UNLOAD_CHUNK => {
                if let Ok(ClientboundPlayPacket::UnloadChunk(p)) =
                    ClientboundPlayPacket::decode_raw(&raw)
                    && w.is_column_loaded(p.chunk_x, p.chunk_z)
                {
                    sample_column(rec, &w, p.chunk_x, p.chunk_z, per, &mut rng, &mut out);
                }
            }
            cb::id::RESPAWN => {
                let cols: Vec<_> = w.columns().iter().map(|(p, _)| p).collect();
                for p in cols {
                    sample_column(rec, &w, p.x, p.z, per, &mut rng, &mut out);
                }
            }
            _ => {}
        }
        let t = std::time::Instant::now();
        match w.handle_raw(&raw) {
            Ok(_) => {}
            Err(e) => {
                errors += 1;
                eprintln!("error: {e}");
            }
        }
        if id == cb::id::MAP_CHUNK {
            decode_ns += t.elapsed().as_nanos();
            chunks += 1;
            let n = w.columns().len();
            if n >= peak.0 {
                peak = (n, w.heap_bytes());
            }
        }
    }
    rec += 1;
    let cols: Vec<_> = w.columns().iter().map(|(p, _)| p).collect();
    for p in cols {
        sample_column(rec, &w, p.x, p.z, per, &mut rng, &mut out);
    }
    // Every position of the boxes in PROBE_BOXES ("x0,y0,z0,x1,y1,z1;...") at the end.
    if let (Ok(boxes), Some(d)) = (std::env::var("PROBE_BOXES"), w.dimension()) {
        for b in boxes.split(';').filter(|b| !b.is_empty()) {
            let v: Vec<i32> = b.split(',').map(|n| n.trim().parse().unwrap()).collect();
            for x in v[0]..=v[3] {
                for y in v[1]..=v[4] {
                    for z in v[2]..=v[5] {
                        if let Some(s) = w.block_state(x, y, z) {
                            out.push(format!("{rec}\t{}\t{x}\t{y}\t{z}\t{s}", d.name));
                        }
                    }
                }
            }
        }
    }
    let mut f = std::fs::File::create(&args[1]).unwrap();
    for l in &out {
        writeln!(f, "{l}").unwrap();
    }
    println!(
        "{chunks} chunks, {errors} errors, {:.1} µs per chunk (debug build if slow); peak {} columns {} bytes ({} B/column); {} probes",
        decode_ns as f64 / chunks.max(1) as f64 / 1000.0,
        peak.0,
        peak.1,
        peak.1 / peak.0.max(1),
        out.len()
    );
}
