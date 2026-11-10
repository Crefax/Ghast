//! Statistics over the chunks of a 26.1 capture (the record format of
//! `ghast-protocol/examples/login_26_1.rs`): how sections are encoded on the wire and what
//! they cost in memory under several in-memory representations.
//!
//! ```text
//! cargo run --release -p ghast-world --example chunk_stats_26_1 -- capture.bin.zz
//! ```

use std::collections::BTreeMap;
use std::io::Read;

use bytes::Bytes;
use ghast_protocol::codec::Reader;
use ghast_protocol::types::chunk::{Palette, Sections};
use ghast_protocol::v26_1::play::clientbound::ClientboundPlayPacket;
use ghast_protocol::{PacketSet, RawPacket};

fn main() {
    let path = std::env::args().nth(1).expect("capture path");
    let mut data = Vec::new();
    let f = std::fs::read(&path).unwrap();
    flate2::read::ZlibDecoder::new(&f[..])
        .read_to_end(&mut data)
        .unwrap();
    let data = Bytes::from(data);
    let mut r = Reader::new(&data);
    // (dimension index, per chunk stats)
    let mut dim = 0usize;
    let mut per_dim: BTreeMap<usize, Vec<[usize; 6]>> = BTreeMap::new();
    let mut bits_hist: BTreeMap<(u8, usize), usize> = BTreeMap::new();
    let mut sections_total = 0usize;
    while r.remaining() > 0 {
        let state = r.read_u8().unwrap();
        let dir = r.read_u8().unwrap();
        let id = r.read_varint().unwrap();
        let len = r.read_varint().unwrap() as usize;
        let body = r.read_bytes(len).unwrap();
        if state != 4 || dir != 0 {
            continue;
        }
        let raw = RawPacket::new(id, body);
        let Ok(p) = ClientboundPlayPacket::decode_raw(&raw) else {
            continue;
        };
        match p {
            ClientboundPlayPacket::Login(l) => dim = l.world_state.dimension as usize,
            ClientboundPlayPacket::Respawn(l) => dim = l.world_state.dimension as usize,
            ClientboundPlayPacket::MapChunk(c) => {
                // [wire bytes, old repr, pow2 repr, pow2+2bit repr, sections, uniform sections]
                let mut st = [0usize; 6];
                st[0] = c.chunk_data.len();
                for s in Sections::new(&c.chunk_data) {
                    let s = s.unwrap();
                    sections_total += 1;
                    st[4] += 1;
                    let (distinct, bits) = match &s.blocks.palette {
                        Palette::Single(_) => (1, 0u8),
                        Palette::Indirect(v) => {
                            // distinct values actually used
                            let mut used = vec![false; v.len()];
                            for i in 0..4096 {
                                if let Some(id) = s.blocks.get(i)
                                    && let Some(k) = v.iter().position(|x| *x == id)
                                {
                                    used[k] = true;
                                }
                            }
                            (used.iter().filter(|u| **u).count().max(1), s.blocks.bits)
                        }
                        Palette::Direct => (300, s.blocks.bits),
                    };
                    *bits_hist.entry((bits, distinct.min(300))).or_default() += 1;
                    // old: Uniform 0, Indexed 4616 (u8 per block + 256 x u16 + 4), Direct 8194;
                    // all-air sections were not stored.
                    let air = matches!(s.blocks.palette, Palette::Single(0));
                    let uniform = distinct == 1;
                    if uniform {
                        st[5] += 1;
                    }
                    st[1] += if uniform || air {
                        0
                    } else if distinct <= 256 {
                        4616
                    } else {
                        8194
                    };
                    // pow2: nibble (<=16) 2048 + 32, byte (<=256) 4096 + 512, direct 8192
                    let pow2 = |two_bit: bool| {
                        if uniform {
                            0
                        } else if two_bit && distinct <= 2 {
                            512 + 4
                        } else if two_bit && distinct <= 4 {
                            1024 + 8
                        } else if distinct <= 16 {
                            2048 + 32
                        } else if distinct <= 256 {
                            4096 + 512
                        } else {
                            8192
                        }
                    };
                    st[2] += pow2(false);
                    st[3] += pow2(true);
                }
                per_dim.entry(dim).or_default().push(st);
            }
            _ => {}
        }
    }
    println!("sections: {sections_total}");
    for (d, v) in &per_dim {
        let n = v.len();
        let sum = |k: usize| v.iter().map(|s| s[k]).sum::<usize>();
        println!(
            "dim {d}: {n} chunks, {} sections/chunk, {} uniform/chunk; per chunk: wire {} B, old {} B, pow2 {} B, pow2+1/2bit {} B",
            sum(4) / n,
            sum(5) as f64 / n as f64,
            sum(0) / n,
            sum(1) / n,
            sum(2) / n,
            sum(3) / n
        );
    }
    println!("(bits, distinct used) histogram:");
    for ((b, d), c) in &bits_hist {
        println!("  bits {b:2} distinct {d:3}: {c}");
    }
}
