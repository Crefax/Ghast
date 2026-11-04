//! Property tests for NBT and the framing layer: round trips through every
//! compression/encryption mode with arbitrary chunking, and no panics on arbitrary input.

use bytes::Bytes;
use ghast_protocol::codec::Reader;
use ghast_protocol::nbt::RawNbt;
use ghast_protocol::testgen::Gen;
use ghast_protocol::{FrameDecoder, FrameEncoder, PacketFilter, RawPacket};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Random NBT: skipping finds exactly the encoded bytes, and the tree survives.
    #[test]
    fn nbt_roundtrip(seed in any::<u64>()) {
        let mut g = Gen::new(seed);
        let raw = g.nbt();
        let bytes = raw.as_bytes().clone();
        let again = RawNbt::from_bytes(bytes.clone()).unwrap();
        prop_assert_eq!(&again, &raw);
        if let Some(tree) = raw.parse().unwrap() {
            let back = RawNbt::from_named(&tree).unwrap();
            prop_assert_eq!(back.as_bytes(), &bytes);
        }
    }

    /// Arbitrary bytes never panic the NBT reader or parser.
    #[test]
    fn nbt_garbage(data in proptest::collection::vec(any::<u8>(), 0..300)) {
        let b = Bytes::from(data);
        let mut r = Reader::new(&b);
        if let Ok(raw) = RawNbt::read(&mut r) {
            let _ = raw.parse();
        }
        let mut r = Reader::new(&b);
        let _ = RawNbt::read_optional(&mut r);
    }

    /// NBT with one byte changed never panics either.
    #[test]
    fn nbt_mutated(seed in any::<u64>(), pos in any::<usize>(), byte in any::<u8>()) {
        let mut g = Gen::new(seed);
        let mut v = g.nbt().as_bytes().to_vec();
        let i = pos % v.len();
        v[i] = byte;
        let b = Bytes::from(v);
        if let Ok(raw) = RawNbt::read(&mut Reader::new(&b)) {
            let _ = raw.parse();
        }
    }

    /// Arbitrary bytes never panic the frame decoder (with and without compression).
    #[test]
    fn frames_garbage(data in proptest::collection::vec(any::<u8>(), 0..3000), compression in any::<bool>(), filtered in any::<bool>()) {
        let mut dec = FrameDecoder::new();
        if compression {
            dec.set_compression(0);
        }
        if filtered {
            dec.set_filter(PacketFilter::none().with(1));
        }
        dec.push(&data);
        for _ in 0..100 {
            match dec.next_packet() {
                Ok(Some(_)) => {}
                _ => break,
            }
        }
    }

    /// A valid compressed stream with one byte changed never panics.
    #[test]
    fn frames_mutated(seed in any::<u64>(), pos in any::<usize>(), byte in any::<u8>(), filtered in any::<bool>()) {
        let mut g = Gen::new(seed);
        let mut enc = FrameEncoder::new();
        enc.set_compression(8);
        for _ in 0..5 {
            let n = g.below(300) as usize;
            enc.write_raw(&RawPacket::new(g.below(8) as i32, Bytes::from(vec![g.u64() as u8; n]))).unwrap();
        }
        let mut wire = enc.take().to_vec();
        let i = pos % wire.len();
        wire[i] = byte;
        let mut dec = FrameDecoder::new();
        dec.set_compression(8);
        if filtered {
            dec.set_filter(PacketFilter::none().with(3));
        }
        dec.push(&wire);
        for _ in 0..10 {
            match dec.next_packet() {
                Ok(Some(_)) => {}
                _ => break,
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Packets through encoder and decoder in any mode, fed in random pieces.
    #[test]
    fn frames_roundtrip(
        seed in any::<u64>(),
        threshold in prop_oneof![Just(-1i32), 0i32..300],
        encrypt in any::<bool>(),
        filter_bits in any::<u64>(),
        chunk in 1usize..2000,
    ) {
        let mut g = Gen::new(seed);
        let packets: Vec<RawPacket> = (0..1 + g.below(30))
            .map(|_| {
                let n = if g.chance(10) { g.below(70_000) } else { g.below(600) } as usize;
                let body: Vec<u8> = (0..n).map(|i| if g.chance(50) { 0 } else { (i as u8) ^ (g.u64() as u8) }).collect();
                RawPacket::new(g.below(64) as i32, Bytes::from(body))
            })
            .collect();
        let mut filter = PacketFilter::none();
        for id in 0..64 {
            if filter_bits & (1 << id) != 0 {
                filter.insert(id);
            }
        }
        let key: [u8; 16] = std::array::from_fn(|i| (seed >> (i % 8)) as u8);
        let mut enc = FrameEncoder::new();
        let mut dec = FrameDecoder::new();
        enc.set_compression(threshold);
        dec.set_compression(threshold);
        if encrypt {
            enc.enable_encryption(&key);
            dec.enable_encryption(&key);
        }
        dec.set_filter(filter);
        for p in &packets {
            enc.write_raw(p).unwrap();
        }
        let wire = enc.take();
        let mut got = Vec::new();
        for piece in wire.chunks(chunk) {
            dec.push(piece);
            while let Some(p) = dec.next_packet().unwrap() {
                got.push(p);
            }
        }
        let want: Vec<RawPacket> = packets.into_iter().filter(|p| filter.contains(p.id)).collect();
        prop_assert_eq!(got, want);
        prop_assert_eq!(dec.buffered(), 0);
    }
}
