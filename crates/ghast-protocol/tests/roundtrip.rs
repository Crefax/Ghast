//! Round-trip property tests for every generated packet of every version:
//! generate a random (always encodable) packet, encode it, decode it, and check that the
//! value and the bytes survive unchanged. Also checks that decoding never panics on
//! arbitrary or corrupted input.

use bytes::{Bytes, BytesMut};
use ghast_protocol::testgen::Gen;
use ghast_protocol::{PacketSet, RawPacket};
use proptest::prelude::*;

/// Encodes, decodes and re-encodes one packet; returns the encoded body.
fn roundtrip_one<S: PacketSet + PartialEq + std::fmt::Debug>(p: &S) -> Bytes {
    let mut w = BytesMut::new();
    p.encode_body(&mut w)
        .unwrap_or_else(|e| panic!("encode {}: {e}\n{p:?}", p.name()));
    let body = w.freeze();
    let raw = RawPacket::new(p.id(), body.clone());
    let back = S::decode_raw(&raw)
        .unwrap_or_else(|e| panic!("decode {}: {e}\n{p:?}\n{body:02x?}", p.name()));
    assert_eq!(&back, p, "value changed for {}", p.name());
    let mut w2 = BytesMut::new();
    back.encode_body(&mut w2).unwrap();
    assert_eq!(&w2[..], &body[..], "bytes changed for {}", p.name());
    body
}

/// Decoding arbitrary bodies must return (never panic).
fn decode_garbage<S: PacketSet>(id: i32, body: &[u8]) {
    let _ = S::decode_raw(&RawPacket::new(id, Bytes::copy_from_slice(body)));
}

macro_rules! sets {
    ($m:ident: $($state:ident :: $dir:ident :: $set:ident),* $(,)?) => {
        mod $m {
            use super::*;
            use ghast_protocol::$m as v;

            /// Every packet id, many seeds each (deterministic, full coverage).
            #[test]
            fn every_packet_many_seeds() {
                let mut total = 0usize;
                $(
                    for &id in v::$state::$dir::$set::IDS {
                        for seed in 0..400u64 {
                            let mut g = Gen::new(seed.wrapping_mul(0x1234_5678_9abc_def1) ^ id as u64);
                            let p = v::$state::$dir::$set::generate_by_id(id, &mut g).unwrap();
                            roundtrip_one(&p);
                            total += 1;
                        }
                    }
                )*
                assert!(total > 0);
                eprintln!("{} round trips", total);
            }

            proptest! {
                #![proptest_config(ProptestConfig::with_cases(512))]

                /// Random packet of a random id with a random seed.
                #[test]
                fn random_packets(pick in any::<usize>(), seed in any::<u64>()) {
                    let ids: Vec<(usize, i32)> = {
                        let mut all = Vec::new();
                        let mut k = 0usize;
                        $( for &id in v::$state::$dir::$set::IDS { all.push((k, id)); } k += 1; )*
                        let _ = k;
                        all
                    };
                    let (set, id) = ids[pick % ids.len()];
                    let mut g = Gen::new(seed);
                    let mut k = 0usize;
                    $(
                        if set == k {
                            let p = v::$state::$dir::$set::generate_by_id(id, &mut g).unwrap();
                            roundtrip_one(&p);
                        }
                        k += 1;
                    )*
                    let _ = k;
                }

                /// Arbitrary bytes as the body of every packet id.
                #[test]
                fn garbage_never_panics(body in proptest::collection::vec(any::<u8>(), 0..200)) {
                    $(
                        for &id in v::$state::$dir::$set::IDS {
                            decode_garbage::<v::$state::$dir::$set>(id, &body);
                        }
                    )*
                }

                /// Valid packets with one byte changed, truncated or extended.
                #[test]
                fn mutations_never_panic(pick in any::<usize>(), seed in any::<u64>(), pos in any::<usize>(), byte in any::<u8>(), cut in any::<usize>()) {
                    let mut g = Gen::new(seed);
                    let mut k = 0usize;
                    let ids: Vec<(usize, i32)> = {
                        let mut all = Vec::new();
                        let mut j = 0usize;
                        $( for &id in v::$state::$dir::$set::IDS { all.push((j, id)); } j += 1; )*
                        let _ = j;
                        all
                    };
                    let (set, id) = ids[pick % ids.len()];
                    $(
                        if set == k {
                            let p = v::$state::$dir::$set::generate_by_id(id, &mut g).unwrap();
                            let body = roundtrip_one(&p).to_vec();
                            if !body.is_empty() {
                                let mut m = body.clone();
                                let i = pos % m.len();
                                m[i] = byte;
                                decode_garbage::<v::$state::$dir::$set>(id, &m);
                                decode_garbage::<v::$state::$dir::$set>(id, &body[..cut % body.len()]);
                            }
                            let mut m = body.clone();
                            m.push(byte);
                            decode_garbage::<v::$state::$dir::$set>(id, &m);
                        }
                        k += 1;
                    )*
                    let _ = k;
                }
            }
        }
    };
}

#[cfg(feature = "v1_8")]
sets!(v1_8:
    handshaking::serverbound::ServerboundHandshakingPacket,
    status::clientbound::ClientboundStatusPacket,
    status::serverbound::ServerboundStatusPacket,
    login::clientbound::ClientboundLoginPacket,
    login::serverbound::ServerboundLoginPacket,
    play::clientbound::ClientboundPlayPacket,
    play::serverbound::ServerboundPlayPacket,
);

#[cfg(feature = "v1_12_2")]
sets!(v1_12_2:
    handshaking::serverbound::ServerboundHandshakingPacket,
    status::clientbound::ClientboundStatusPacket,
    status::serverbound::ServerboundStatusPacket,
    login::clientbound::ClientboundLoginPacket,
    login::serverbound::ServerboundLoginPacket,
    play::clientbound::ClientboundPlayPacket,
    play::serverbound::ServerboundPlayPacket,
);

#[cfg(feature = "v1_16_5")]
sets!(v1_16_5:
    handshaking::serverbound::ServerboundHandshakingPacket,
    status::clientbound::ClientboundStatusPacket,
    status::serverbound::ServerboundStatusPacket,
    login::clientbound::ClientboundLoginPacket,
    login::serverbound::ServerboundLoginPacket,
    play::clientbound::ClientboundPlayPacket,
    play::serverbound::ServerboundPlayPacket,
);
