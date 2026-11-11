//! `window_items` cost: decode only, decode + apply, and (26.1) the lazy path with and without
//! reading every stack's name afterwards.
//!
//! - 1.8.9: a 45-slot player inventory and a 54-slot shop window whose stacks carry NBT
//!   (display name, three lore lines, an enchantment) — the shape that dominated a busy server's
//!   inbound bytes.
//! - 26.1: the same shop as data components (custom_name, lore, enchantment_glint_override),
//!   and the component-rich `window_items` of the protocol crate's real capture.
//!
//! `cargo bench -p ghast-inventory --bench window_items`

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ghast_inventory::{Inventory, Version};
use ghast_protocol::nbt::{Compound, List, NamedTag, RawNbt, Tag};
use ghast_protocol::{PacketSet, RawPacket};

fn text(s: &str) -> Tag {
    Tag::String(format!(
        "{{\"text\":\"{s}\",\"color\":\"gold\",\"italic\":false}}"
    ))
}

/// 1.8 item NBT: display name, three lore lines, one enchantment.
fn legacy_nbt(i: usize) -> RawNbt {
    let display = Compound(vec![
        ("Name".into(), text(&format!("Shop item {i}"))),
        (
            "Lore".into(),
            Tag::List(List {
                elem_type: 8,
                items: vec![text("Price: 100"), text("Click to buy"), text("Stock: 64")],
            }),
        ),
    ]);
    let ench = Tag::List(List {
        elem_type: 10,
        items: vec![Tag::Compound(Compound(vec![
            ("id".into(), Tag::Short(34)),
            ("lvl".into(), Tag::Short(1)),
        ]))],
    });
    RawNbt::from_named(&NamedTag {
        name: String::new(),
        tag: Tag::Compound(Compound(vec![
            ("display".into(), Tag::Compound(display)),
            ("ench".into(), ench),
        ])),
    })
    .unwrap()
}

fn v1_8_window(slots: usize, window_id: u8) -> RawPacket {
    use ghast_protocol::types::slot::legacy::{ItemStack, Slot};
    use ghast_protocol::v1_8::play::clientbound as cb;
    let items = (0..slots)
        .map(|i| {
            Slot(Some(ItemStack {
                id: 1 + (i % 300) as i16,
                count: 1 + (i % 64) as i8,
                damage: 0,
                nbt: Some(legacy_nbt(i)),
            }))
        })
        .collect();
    RawPacket::from_packet(&cb::WindowItems { window_id, items }).unwrap()
}

#[cfg(feature = "v26_1")]
fn v26_1_shop(slots: usize, window_id: i32) -> RawPacket {
    use bytes::{BufMut, BytesMut};
    use ghast_inventory::v26_1::component as c;
    use ghast_protocol::McWrite;
    use ghast_protocol::nbt::AnonNbt;
    use ghast_protocol::types::slot::v775::{ComponentPatch, ItemStack};
    let nbt_text = |s: &str| {
        AnonNbt::from_tag(&Tag::Compound(Compound(vec![
            ("text".into(), Tag::String(s.into())),
            ("color".into(), Tag::String("gold".into())),
            ("italic".into(), Tag::Byte(0)),
        ])))
        .unwrap()
    };
    let mut w = BytesMut::new();
    w.put_varint(window_id);
    w.put_varint(7);
    w.put_varint(slots as i32);
    for i in 0..slots {
        let name = nbt_text(&format!("Shop item {i}"));
        let mut lore = BytesMut::new();
        lore.put_varint(3);
        for l in ["Price: 100", "Click to buy", "Stock: 64"] {
            lore.put_slice(nbt_text(l).as_bytes());
        }
        let patch = ComponentPatch::from_parts(
            &[
                (c::CUSTOM_NAME, name.as_bytes()),
                (c::LORE, &lore),
                (c::ENCHANTMENT_GLINT_OVERRIDE, &[1]),
            ],
            &[],
        );
        let s = ItemStack {
            id: 1 + (i % 1000) as i32,
            count: 1 + (i % 64) as i32,
            components: patch,
        };
        ItemStack::write(Some(&s), &mut w);
    }
    ItemStack::write(None, &mut w);
    RawPacket::new(
        ghast_protocol::v26_1::play::clientbound::id::WINDOW_ITEMS,
        w.freeze(),
    )
}

#[cfg(feature = "v26_1")]
fn captured_26_1() -> Vec<RawPacket> {
    use bytes::Bytes;
    use std::io::Read;
    const CAPTURE: &[u8] =
        include_bytes!("../../ghast-protocol/tests/fixtures/capture_26_1_2.bin.zz");
    let mut data = Vec::new();
    flate2::read::ZlibDecoder::new(CAPTURE)
        .read_to_end(&mut data)
        .unwrap();
    let data = Bytes::from(data);
    let mut r = ghast_protocol::Reader::new(&data);
    let mut out = Vec::new();
    while r.remaining() > 0 {
        let state = r.read_u8().unwrap();
        let dir = r.read_u8().unwrap();
        let id = r.read_varint().unwrap();
        let len = r.read_varint().unwrap() as usize;
        let body = r.read_bytes(len).unwrap();
        if state == 4
            && dir == 0
            && id == ghast_protocol::v26_1::play::clientbound::id::WINDOW_ITEMS
        {
            out.push(RawPacket::new(id, body));
        }
    }
    out
}

fn bench(c: &mut Criterion) {
    // ------------------------------------------------------------------ 1.8.9
    {
        use ghast_protocol::v1_8::play::clientbound::ClientboundPlayPacket as P8;
        let mut g = c.benchmark_group("window_items 1.8.9");
        for (label, raw) in [
            ("player 45", v1_8_window(45, 0)),
            ("shop 54+36", v1_8_window(90, 1)),
        ] {
            g.throughput(Throughput::Bytes(raw.body.len() as u64));
            g.bench_function(format!("{label} decode"), |b| {
                b.iter(|| black_box(P8::decode_raw(black_box(&raw)).unwrap()))
            });
            let mut inv = Inventory::new(Version::V1_8);
            if label.starts_with("shop") {
                inv.on_open_window(
                    1,
                    Some(ghast_inventory::WindowKind::Chest { rows: 6 }),
                    "".into(),
                );
            }
            g.bench_function(format!("{label} decode + apply"), |b| {
                b.iter(|| {
                    let p = P8::decode_raw(black_box(&raw)).unwrap();
                    black_box(inv.apply_v1_8(&p));
                })
            });
            let decoded = P8::decode_raw(&raw).unwrap();
            g.bench_function(format!("{label} apply only"), |b| {
                b.iter(|| black_box(inv.apply_v1_8(black_box(&decoded))))
            });
        }
        g.finish();
    }

    // ------------------------------------------------------------------ 26.1
    #[cfg(feature = "v26_1")]
    {
        use ghast_protocol::v26_1::play::clientbound::ClientboundPlayPacket as P26;
        let v = Version::V26_1;
        let mut g = c.benchmark_group("window_items 26.1");
        let mut cases = vec![("shop 54+36".to_string(), v26_1_shop(90, 1))];
        for (k, raw) in captured_26_1().into_iter().enumerate() {
            cases.push((format!("capture {k}"), raw));
        }
        for (label, raw) in &cases {
            g.throughput(Throughput::Bytes(raw.body.len() as u64));
            let window = ghast_protocol::Reader::new(&raw.body)
                .read_varint()
                .unwrap();
            let mut inv = Inventory::new(v);
            if window != 0 {
                inv.on_open_window(
                    window,
                    Some(ghast_inventory::WindowKind::Chest { rows: 6 }),
                    "".into(),
                );
            }
            g.bench_function(format!("{label} generated decode (all components)"), |b| {
                b.iter(|| black_box(P26::decode_raw(black_box(raw)).unwrap()))
            });
            g.bench_function(format!("{label} generated decode + apply"), |b| {
                b.iter(|| {
                    let p = P26::decode_raw(black_box(raw)).unwrap();
                    inv.apply_v26_1(&p);
                })
            });
            g.bench_function(format!("{label} lazy read only (no apply)"), |b| {
                b.iter(|| {
                    let mut r = ghast_protocol::Reader::new(&raw.body);
                    r.read_varint().unwrap();
                    r.read_varint().unwrap();
                    let n = r.read_varint().unwrap();
                    let mut k = 0;
                    for _ in 0..=n {
                        k += usize::from(
                            ghast_protocol::types::slot::v775::ItemStack::read(&mut r)
                                .unwrap()
                                .is_some(),
                        );
                    }
                    black_box(k)
                })
            });
            g.bench_function(format!("{label} slot skip only"), |b| {
                b.iter(|| {
                    let mut r = ghast_protocol::Reader::new(&raw.body);
                    r.read_varint().unwrap();
                    r.read_varint().unwrap();
                    let n = r.read_varint().unwrap();
                    for _ in 0..=n {
                        ghast_protocol::defs::p775::types::Slot::skip(&mut r).unwrap();
                    }
                    black_box(r.remaining())
                })
            });
            g.bench_function(format!("{label} lazy apply (skip)"), |b| {
                b.iter(|| inv.apply_raw_v26_1(black_box(raw)).unwrap())
            });
            g.bench_function(format!("{label} lazy apply + read every name"), |b| {
                b.iter(|| {
                    inv.apply_raw_v26_1(black_box(raw)).unwrap();
                    let mut n = 0usize;
                    for i in 0..inv.slot_count() {
                        if let Some(s) = inv.slot(i)
                            && let Some(t) = s.custom_name(v)
                        {
                            n += t.to_plain().len();
                        }
                    }
                    black_box(n)
                })
            });
        }
        g.finish();
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
