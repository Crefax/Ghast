//! Per-click cost of the legacy click prediction on an ordinary container: a 3-row chest with
//! a mixed inventory, one cycle of pick up / put down / shift-click / number key / double click /
//! drop, each click confirmed before the next (as on a server), contents restored by
//! `window_items` after every cycle.
//!
//! `cargo bench -p ghast-inventory --bench clicks`

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ghast_inventory::{Click, Inventory, Outgoing, Stack, Version, WindowKind};

/// Items present in every version's numbering with stack limit 64 (stone 1, dirt 3 in the
/// legacy ids; 1.16.5 registry ids 1 and 9 are stone and dirt as well).
fn contents(n: usize) -> Vec<Option<Stack>> {
    (0..n)
        .map(|i| match i % 5 {
            0 => Some(Stack::new(1, 1 + (i as i32 % 63), 0)),
            1 => Some(Stack::new(3, 64, 0)),
            2 => None,
            3 => Some(Stack::new(1, 32, 0)),
            _ => None,
        })
        .collect()
}

const CYCLE: [Click; 8] = [
    Click::Pickup {
        slot: 0,
        right: false,
    },
    Click::Pickup {
        slot: 2,
        right: true,
    },
    Click::Pickup {
        slot: 2,
        right: false,
    },
    Click::QuickMove {
        slot: 1,
        right: false,
    },
    Click::QuickMove {
        slot: 40,
        right: false,
    },
    Click::Swap { slot: 5, hotbar: 3 },
    Click::PickupAll { slot: 7 },
    Click::Drop {
        slot: 8,
        all: false,
    },
];

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("clicks chest 3x9");
    g.throughput(Throughput::Elements(CYCLE.len() as u64));
    for v in [Version::V1_8, Version::V1_12, Version::V1_16_5] {
        if !v.is_enabled() {
            continue;
        }
        let mut inv = Inventory::new(v).unwrap();
        inv.on_open_window(1, Some(WindowKind::Chest { rows: 3 }), "".into());
        let items = contents(27 + 36);
        g.bench_function(format!("{v:?} cycle of {}", CYCLE.len()), |b| {
            b.iter(|| {
                inv.on_window_items(1, items.iter().cloned());
                inv.on_set_slot(-1, -1, None);
                for click in CYCLE {
                    inv.tick();
                    if let Ok(Outgoing::Click(p)) = inv.click(black_box(click)) {
                        inv.on_transaction(p.window_id as i8, p.action, true);
                    }
                }
                black_box(inv.cursor().is_some())
            })
        });
    }
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
