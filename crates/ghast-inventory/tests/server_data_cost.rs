//! What the server's menu data costs a client that keeps it (ignored; numbers only):
//! the vanilla 1.16.5 `declare_recipes` and `tags` the oracle recorded (decode time, transient
//! allocations, heap kept by the inventory) and a 1.8 plugin-channel message decoded or skipped
//! by the frame filter.
//!
//! ```text
//! cargo test --release -p ghast-inventory --test server_data_cost -- --ignored --nocapture
//! ```

#![cfg(all(feature = "v1_8", feature = "v1_16_5"))]

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::BufRead;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let n = LIVE.fetch_add(l.size(), Relaxed) + l.size();
        PEAK.fetch_max(n, Relaxed);
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        if new > l.size() {
            let n = LIVE.fetch_add(new - l.size(), Relaxed) + new - l.size();
            PEAK.fetch_max(n, Relaxed);
        } else {
            LIVE.fetch_sub(l.size() - new, Relaxed);
        }
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// The first `{"<key>":"<hex>"}` line of the 1.16.5 menu vectors, as bytes.
fn recorded(key: &str) -> bytes::Bytes {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.16.5/clicks_menus_seed1.jsonl.gz");
    let f = std::fs::File::open(&path).unwrap();
    let r = std::io::BufReader::new(flate2::read::GzDecoder::new(f));
    let head = format!("{{\"{key}\"");
    for line in r.lines() {
        let line = line.unwrap();
        if line.starts_with(&head) {
            let v: serde_json::Value = serde_json::from_str(&line).unwrap();
            let h = v[key].as_str().unwrap();
            return (0..h.len() / 2)
                .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
                .collect::<Vec<u8>>()
                .into();
        }
    }
    panic!("no {key} line");
}

/// Runs `f` once with counting on: (allocations, peak bytes above the start, bytes still live
/// after `f` returned, ns).
fn measure<T>(f: impl FnOnce() -> T) -> (T, usize, usize, isize, u128) {
    let start = LIVE.load(Relaxed);
    PEAK.store(start, Relaxed);
    let a0 = ALLOCS.load(Relaxed);
    let t = Instant::now();
    let out = f();
    let ns = t.elapsed().as_nanos();
    let kept = LIVE.load(Relaxed) as isize - start as isize;
    (
        out,
        ALLOCS.load(Relaxed) - a0,
        PEAK.load(Relaxed) - start,
        kept,
        ns,
    )
}

#[test]
#[ignore = "measurement"]
fn server_data_cost() {
    use ghast_inventory::{Inventory, RecipeBook, Version};
    use ghast_protocol::codec::decode_exact;
    use ghast_protocol::v1_16_5::play::clientbound as cb;
    let recipes = recorded("declare_recipes");
    let tags = recorded("tags");
    println!(
        "vanilla 1.16.5 bodies: declare_recipes {} B, tags {} B",
        recipes.len(),
        tags.len()
    );
    // warm up (lazy statics, interning)
    for _ in 0..3 {
        let d: cb::DeclareRecipes = decode_exact(&recipes).unwrap();
        drop(RecipeBook::from_v1_16_5(&d));
        let t: cb::Tags = decode_exact(&tags).unwrap();
        let mut inv = Inventory::new(Version::V1_16_5).unwrap();
        inv.on_tags_v1_16_5(&t);
    }
    let mut inv = Inventory::new(Version::V1_16_5).unwrap();
    let ((), n, peak, kept, ns) = measure(|| {
        let d: cb::DeclareRecipes = decode_exact(&recipes).unwrap();
        inv.on_declare_recipes(RecipeBook::from_v1_16_5(&d));
    });
    println!(
        "declare_recipes: {n} allocations, peak {peak} B, kept by the inventory {kept} B, {} us",
        ns / 1000
    );
    let ((), n, peak, kept, ns) = measure(|| {
        let t: cb::Tags = decode_exact(&tags).unwrap();
        inv.on_tags_v1_16_5(&t);
    });
    println!(
        "tags: {n} allocations, peak {peak} B, kept by the inventory {kept} B, {} us",
        ns / 1000
    );
    println!(
        "recipes in the book: {}, RecipeBook::heap_bytes {} B",
        inv.recipes().len(),
        inv.recipes().heap_bytes()
    );
    // the process-wide copy: the first bot on a server decodes, the others share its book
    let bots: Vec<Inventory> = (0..100)
        .map(|_| Inventory::new(Version::V1_16_5).unwrap())
        .collect();
    let mut bots = bots;
    let (first, n, _, kept, ns) = measure(|| RecipeBook::from_v1_16_5_body(&recipes).unwrap());
    println!(
        "from_v1_16_5_body, first bot: {n} allocations, kept {kept} B, {} us",
        ns / 1000
    );
    bots[0].on_declare_recipes(first);
    let ((), n, _, kept, ns) = measure(|| {
        for b in &mut bots[1..] {
            b.on_declare_recipes(RecipeBook::from_v1_16_5_body(&recipes).unwrap());
        }
    });
    println!(
        "from_v1_16_5_body, 99 more bots: {n} allocations, kept {kept} B in all, {} us in all",
        ns / 1000
    );

    // Small packets through the frame decoder (uncompressed, like most small packets):
    // passed and decoded vs dropped by the filter.
    use ghast_protocol::v1_8::play::clientbound as cb8;
    use ghast_protocol::v1_12_2::play::clientbound as cb12;
    let payload = || vec![7u8; 64].into();
    frame_cost(
        "1.8 custom_payload (64 B)",
        cb8::id::CUSTOM_PAYLOAD,
        &cb8::CustomPayload {
            channel: "BungeeCord".into(),
            data: payload(),
        },
        |r| drop(r.decode_as::<cb8::CustomPayload>().unwrap()),
    );
    frame_cost(
        "1.12.2 custom_payload (64 B)",
        cb12::id::CUSTOM_PAYLOAD,
        &cb12::CustomPayload {
            channel: "BungeeCord".into(),
            data: payload(),
        },
        |r| drop(r.decode_as::<cb12::CustomPayload>().unwrap()),
    );
    let xp8 = cb8::Experience {
        experience_bar: 0.25,
        level: 17,
        total_experience: 400,
    };
    frame_cost("1.8 experience", cb8::id::EXPERIENCE, &xp8, |r| {
        std::hint::black_box(r.decode_as::<cb8::Experience>().unwrap());
    });
    let xp16 = cb::Experience {
        experience_bar: 0.25,
        level: 17,
        total_experience: 400,
    };
    frame_cost("1.16.5 experience", cb::id::EXPERIENCE, &xp16, |r| {
        std::hint::black_box(r.decode_as::<cb::Experience>().unwrap());
    });
}

/// ns and allocations per packet for 100,000 frames of `p`, passed + decoded vs filtered.
fn frame_cost<P: ghast_protocol::Packet>(
    what: &str,
    id: i32,
    p: &P,
    decode: impl Fn(&ghast_protocol::RawPacket),
) {
    use ghast_protocol::{FrameDecoder, FrameEncoder, PacketFilter};
    const K: usize = 100_000;
    let mut enc = FrameEncoder::new();
    for _ in 0..K {
        enc.write_packet(p).unwrap();
    }
    let wire = enc.take().freeze();
    for pass in [true, false] {
        let mut dec = FrameDecoder::new();
        dec.set_filter(if pass {
            PacketFilter::none().with(id)
        } else {
            PacketFilter::none()
        });
        dec.push(&wire);
        let ((), n, _, _, ns) = measure(|| {
            while let Some(raw) = dec.next_packet().unwrap() {
                if pass {
                    decode(&raw);
                }
            }
        });
        println!(
            "{what} {}: {} ns/packet, {:.2} allocations/packet",
            if pass { "passed + decoded" } else { "filtered" },
            ns / K as u128,
            n as f64 / K as f64
        );
    }
}
