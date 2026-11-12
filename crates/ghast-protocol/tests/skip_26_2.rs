//! `Slot::skip` / `SlotComponent::skip` (26.2, protocol 776) read exactly the bytes `Decode`
//! reads: on random values of every component kind (testgen) and on every slot of the real
//! capture; the lazy `ItemStack::read_776` agrees.
#![cfg(feature = "v26_2")]

use std::io::Read;

use bytes::{Bytes, BytesMut};
use ghast_protocol::codec::Reader;
use ghast_protocol::defs::p776::types::{Slot, SlotComponent};
use ghast_protocol::testgen::{Gen, Generate};
use ghast_protocol::types::slot::v775::ItemStack;
use ghast_protocol::{Decode, Encode};

fn same_length<T: Decode + Encode>(
    v: &T,
    skip: fn(&mut Reader<'_>) -> ghast_protocol::codec::DecodeResult<()>,
) {
    let mut w = BytesMut::new();
    v.encode(&mut w).unwrap();
    // Trailing marker: skip must stop exactly before it.
    w.extend_from_slice(&[0xAB, 0xCD]);
    let b = w.freeze();
    let mut r = Reader::new(&b);
    skip(&mut r).unwrap();
    assert_eq!(r.remaining(), 2, "skip read a different length");
    let mut r = Reader::new(&b);
    T::decode(&mut r).unwrap();
    assert_eq!(r.remaining(), 2);
    // A truncated value fails to skip (never panics).
    for cut in [1usize, 2, 3, b.len() / 2] {
        if cut + 2 < b.len() {
            let t = b.slice(..b.len() - 2 - cut);
            let mut r = Reader::new(&t);
            let _ = skip(&mut r);
        }
    }
}

#[test]
fn skip_matches_decode_on_generated_values() {
    let mut kinds = std::collections::BTreeSet::new();
    for seed in 0..40_000u64 {
        let mut g = Gen::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let c = SlotComponent::generate(&mut g);
        kinds.insert(c.r#type.0);
        same_length(&c, SlotComponent::skip);
    }
    for seed in 0..5_000u64 {
        let mut g = Gen::new(seed ^ 0x5151);
        let s = Slot::generate(&mut g);
        same_length(&s, Slot::skip);
        let mut w = BytesMut::new();
        s.encode(&mut w).unwrap();
        let b = w.freeze();
        let lazy = ItemStack::read_776(&mut Reader::new(&b)).unwrap();
        match (&lazy, &s.body) {
            (Some(l), Some(f)) => {
                assert_eq!(l.components.decode_all_776().unwrap(), f.components);
                assert_eq!(l.components.removed().count(), f.remove_components.len());
                let mut w2 = BytesMut::new();
                ItemStack::write(Some(l), &mut w2);
                assert_eq!(w2.freeze(), b);
            }
            (None, None) => {}
            other => panic!("{other:?}"),
        }
    }
    assert!(
        kinds.len() > 105,
        "{} component kinds generated",
        kinds.len()
    );
}

#[test]
fn skip_matches_decode_on_captured_slots() {
    const FIXTURE: &[u8] = include_bytes!("fixtures/capture_26_2.bin.zz");
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(FIXTURE)
        .read_to_end(&mut data)
        .unwrap();
    let data = Bytes::from(data);
    let mut r = Reader::new(&data);
    let mut slots = 0;
    let mut components = 0;
    while r.remaining() > 0 {
        let state = r.read_u8().unwrap();
        let dir = r.read_u8().unwrap();
        let id = r.read_varint().unwrap();
        let len = r.read_varint().unwrap() as usize;
        let body = r.read_bytes(len).unwrap();
        use ghast_protocol::v26_2::play::clientbound::id;
        if state != 4 || dir != 0 || (id != id::WINDOW_ITEMS && id != id::SET_SLOT) {
            continue;
        }
        let mut b = Reader::new(&body);
        b.read_varint().unwrap();
        b.read_varint().unwrap();
        let n = if id == id::WINDOW_ITEMS {
            b.read_varint().unwrap() as usize + 1
        } else {
            b.read_i16().unwrap();
            1
        };
        for _ in 0..n {
            let start = b.position();
            let mut probe = b.clone();
            Slot::skip(&mut probe).unwrap();
            let mut lazy_r = b.clone();
            let lazy = ItemStack::read_776(&mut lazy_r).unwrap();
            let full = Slot::decode(&mut b).unwrap();
            assert_eq!(probe.position(), b.position(), "slot at {start}");
            assert_eq!(lazy_r.position(), b.position(), "lazy slot at {start}");
            match (&lazy, &full.body) {
                (None, None) => {}
                (Some(l), Some(f)) => {
                    assert_eq!((l.id, l.count), (f.item_id, full.item_count));
                    assert_eq!(l.components.decode_all_776().unwrap(), f.components);
                    let types: Vec<i32> = l.components.iter().map(|(t, _)| t).collect();
                    let want: Vec<i32> = f.components.iter().map(|c| c.r#type.0).collect();
                    assert_eq!(types, want);
                    let removed: Vec<i32> = l.components.removed().collect();
                    let want: Vec<i32> = f.remove_components.iter().map(|c| c.r#type.0).collect();
                    assert_eq!(removed, want);
                    for c in &f.components {
                        assert_eq!(
                            l.components.decode_776(c.r#type.0).unwrap().unwrap().r#type,
                            c.r#type
                        );
                    }
                    components += f.components.len();
                }
                other => panic!("{other:?}"),
            }
            let mut w = BytesMut::new();
            ItemStack::write(lazy.as_ref(), &mut w);
            assert_eq!(&w[..], &body[start..b.position()], "re-encoded slot");
            slots += 1;
        }
        assert_eq!(b.remaining(), 0);
    }
    assert!(slots > 50, "{slots}");
    assert!(components > 10, "{components}");
}
