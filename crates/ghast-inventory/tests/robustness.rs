//! Random packet and click sequences: never panic, keep the version's invariants, and conserve
//! items in clicks that only move them.
#![cfg(all(feature = "v1_8", feature = "v1_12", feature = "v1_16_5"))]

use ghast_inventory::{Click, Inventory, OUTSIDE, Outgoing, SlotKind, Stack, Version, WindowKind};
use ghast_protocol::nbt::{Compound, NamedTag, RawNbt, Tag};
use proptest::prelude::*;

fn nbt(k: u8) -> RawNbt {
    RawNbt::from_named(&NamedTag {
        name: String::new(),
        tag: Tag::Compound(Compound(vec![
            ("a".into(), Tag::Byte(k as i8)),
            ("Unbreakable".into(), Tag::Byte((k & 1) as i8)),
        ])),
    })
    .unwrap()
}

type St = (i32, i32, i16, Option<u8>);

#[derive(Debug, Clone)]
enum Op {
    Items(u8, Vec<Option<St>>),
    SetSlot(i8, i16, Option<St>),
    Open8(u8, u8),
    Open16(i32),
    CloseServer,
    Confirm(i8, i16, bool),
    ConfirmPending(bool),
    Held(i8),
    Tick,
    Click(u8, i16, u8),
    OpenInv,
    Close,
    Reset,
}

fn stack_strategy(wild: bool) -> impl Strategy<Value = Option<(i32, i32, i16, Option<u8>)>> {
    let id = if wild {
        prop_oneof![Just(0), 1..400i32, 0..1000i32, Just(-5), Just(70_000)].boxed()
    } else {
        prop_oneof![1..6i32, Just(276), Just(310), Just(86), Just(368)].boxed()
    };
    let count = if wild {
        (-3..130i32).boxed()
    } else {
        (1..65i32).boxed()
    };
    let damage = if wild {
        (-2..20i16).boxed()
    } else {
        (0..2i16).boxed()
    };
    prop::option::weighted(0.6, (id, count, damage, prop::option::of(0..3u8)))
}

fn op_strategy(wild: bool) -> impl Strategy<Value = Op> {
    let st = stack_strategy(wild);
    prop_oneof![
        3 => (prop_oneof![Just(0u8), Just(1), 0..10u8], prop::collection::vec(stack_strategy(wild), 0..110))
            .prop_map(|(w, v)| Op::Items(w, v)),
        3 => (-3..4i8, -1000..120i16, st).prop_map(|(w, s, st)| Op::SetSlot(w, s, st)),
        1 => (0..14u8, 0..60u8).prop_map(|(t, n)| Op::Open8(t, n)),
        1 => (-2..26i32).prop_map(Op::Open16),
        1 => Just(Op::CloseServer),
        1 => (-2..4i8, -3..5i16, any::<bool>()).prop_map(|(w, a, b)| Op::Confirm(w, a, b)),
        3 => any::<bool>().prop_map(Op::ConfirmPending),
        1 => (-2..12i8).prop_map(Op::Held),
        4 => Just(Op::Tick),
        8 => (0..7u8, prop_oneof![Just(OUTSIDE), -2..100i16], 0..42u8).prop_map(|(m, s, b)| Op::Click(m, s, b)),
        1 => Just(Op::OpenInv),
        1 => Just(Op::Close),
        1 => Just(Op::Reset),
    ]
}

const TYPES_1_8: [&str; 15] = [
    "minecraft:chest",
    "minecraft:container",
    "minecraft:crafting_table",
    "minecraft:furnace",
    "minecraft:dispenser",
    "minecraft:dropper",
    "minecraft:hopper",
    "minecraft:brewing_stand",
    "minecraft:enchanting_table",
    "minecraft:anvil",
    "minecraft:beacon",
    "minecraft:villager",
    "EntityHorse",
    "plugin:custom",
    "minecraft:shulker_box",
];

/// The item's stack size (64 for unknown ids).
fn max_count(v: Version, id: i32) -> i32 {
    let id = u16::try_from(id).ok();
    let n = match v {
        Version::V1_8 => id
            .and_then(ghast_data::v1_8::Item::from_id)
            .map(|i| i.info().stack_size),
        Version::V1_12 => id
            .and_then(ghast_data::v1_12::Item::from_id)
            .map(|i| i.info().stack_size),
        Version::V1_16_5 => id
            .and_then(ghast_data::v1_16_5::Item::from_id)
            .map(|i| i.info().stack_size),
        other => unimplemented!("{other:?}"),
    };
    i32::from(n.unwrap_or(64))
}

/// Builds a stack; `valid`: its count capped at the item's stack size (the conservation runs:
/// vanilla only loses items when a stack is bigger than its item allows, e.g. a number-key swap
/// that splits an over-stacked tool into the inventory and drops the rest).
fn mk(v: Version, t: (i32, i32, i16, Option<u8>), valid: bool) -> Stack {
    Stack {
        id: t.0,
        count: if valid {
            t.1.min(max_count(v, t.0))
        } else {
            t.1
        },
        // 1.16.5 slots carry no damage field
        damage: if v == Version::V1_16_5 { 0 } else { t.2 },
        data: ghast_inventory::ItemData::from_nbt(t.3.map(nbt)),
    }
}

fn click_of(mode: u8, slot: i16, b: u8) -> Click {
    match mode {
        0 => Click::Pickup {
            slot,
            right: b & 1 == 1,
        },
        1 => Click::QuickMove {
            slot,
            right: b & 1 == 1,
        },
        2 => Click::Swap {
            slot,
            hotbar: if b > 9 { 40 } else { b },
        },
        3 | 4 => Click::Drop {
            slot,
            all: b & 1 == 1,
        },
        _ => Click::PickupAll { slot },
    }
}

fn run(v: Version, ops: Vec<Op>, wild: bool) {
    let mut inv = Inventory::new(v);
    let mut last: Option<(i8, i16)> = None;
    for op in ops {
        match op {
            Op::Items(w, items) => {
                let id = if w == 1 {
                    inv.window().map_or(1, |w| w.id())
                } else {
                    i32::from(w)
                };
                inv.on_window_items(id, items.into_iter().map(|s| s.map(|t| mk(v, t, !wild))));
            }
            Op::SetSlot(w, s, st) => inv.on_set_slot(i32::from(w), s, st.map(|t| mk(v, t, !wild))),
            Op::Open8(t, n) => {
                let t = TYPES_1_8[usize::from(t) % TYPES_1_8.len()];
                let kind = if v == Version::V1_12 {
                    WindowKind::from_v1_12_2(t, n)
                } else {
                    WindowKind::from_v1_8(t, n)
                };
                inv.on_open_window(1 + i32::from(n % 3), kind, "{}".into());
            }
            Op::Open16(t) => inv.on_open_window(2, WindowKind::from_v1_16_5(t), "{}".into()),
            Op::CloseServer => inv.on_close_window(),
            Op::Confirm(w, a, b) => {
                let _ = inv.on_transaction(w, a, b);
            }
            Op::ConfirmPending(ok) => {
                if let Some((w, a)) = last.take()
                    && let Some(Outgoing::Confirm { window_id, action }) =
                        inv.on_transaction(w, a, ok)
                {
                    assert!(!ok && window_id == w && action == a);
                }
            }
            Op::Held(h) => inv.on_held_slot(h),
            Op::Tick => inv.tick(),
            Op::Click(m, s, b) => {
                let c = click_of(m, s, b);
                // A crafting screen's result slot is computed (1.8) or sent by the server (1.16):
                // it is not counted; taking it is checked against the recipe below.
                let crafting = inv.slot_kind(0) == Some(SlotKind::CraftResult);
                let before = total(&inv, crafting);
                let result_before = inv.slot(0).cloned();
                let grid_before = grid(&inv);
                if let Ok(Outgoing::Click(p)) = inv.click(c) {
                    assert!(inv.pending());
                    last = Some((p.window_id as i8, p.action));
                    let moves_only = !matches!(c, Click::Drop { .. })
                        && !matches!(c, Click::Pickup { slot: OUTSIDE, .. })
                        && !matches!(c, Click::QuickMove { slot: OUTSIDE, .. });
                    let on_result = crafting
                        && matches!(
                            c,
                            Click::Pickup { slot: 0, .. }
                                | Click::QuickMove { slot: 0, .. }
                                | Click::Swap { slot: 0, .. }
                                | Click::Drop { slot: 0, .. }
                        );
                    let after = total(&inv, crafting);
                    if !wild && moves_only && !on_result {
                        assert_eq!(before, after, "{c:?} must only move items");
                    }
                    if !wild && on_result && matches!(c, Click::Pickup { .. }) {
                        // taking the result: + the result, - one of each ingredient
                        // (+ a bucket for each milk/water/lava bucket; none in this palette)
                        let mut want: std::collections::BTreeMap<i32, i64> =
                            before.into_iter().collect();
                        if grid(&inv) != grid_before {
                            let r = result_before.expect("a take needs a result");
                            *want.entry(r.id).or_default() += i64::from(r.count);
                            for g in grid_before.iter().flatten() {
                                *want.entry(g.id).or_default() -= 1;
                            }
                        }
                        want.retain(|_, n| *n != 0);
                        assert_eq!(
                            want.into_iter().collect::<Vec<_>>(),
                            after,
                            "{c:?}: a craft"
                        );
                    }
                }
            }
            Op::OpenInv => {
                let _ = inv.open_inventory();
            }
            Op::Close => {
                let _ = inv.close();
            }
            Op::Reset => inv.reset(),
        }
        if v != Version::V1_8 {
            for i in 0..inv.slot_count() {
                if let Some(s) = inv.slot(i) {
                    assert!(
                        s.count > 0 && s.id != 0,
                        "1.12 / 1.16 store no empty stacks: {s:?}"
                    );
                }
            }
            if let Some(c) = inv.cursor() {
                assert!(c.count > 0 && c.id != 0);
            }
        }
    }
}

/// The crafting grid of the current screen.
fn grid(inv: &Inventory) -> Vec<Option<Stack>> {
    let n = if inv.window().is_some() { 9 } else { 4 };
    if inv.slot_kind(0) != Some(SlotKind::CraftResult) {
        return Vec::new();
    }
    (1..=n).map(|i| inv.slot(i).cloned()).collect()
}

/// All items of the current screen plus the cursor, by id (1.8 merges different damage values
/// of items without subtypes, like vanilla); `skip_result`: without slot 0.
fn total(inv: &Inventory, skip_result: bool) -> Vec<(i32, i64)> {
    let mut m = std::collections::BTreeMap::new();
    let mut add = |s: &Stack| *m.entry(s.id).or_insert(0i64) += i64::from(s.count);
    for i in usize::from(skip_result)..inv.slot_count() {
        if let Some(s) = inv.slot(i) {
            add(s);
        }
    }
    // armor / offhand are not part of a container window but swaps can reach the offhand
    for k in 36..41 {
        if inv.window().is_some()
            && let Some(s) = inv.player(k)
        {
            add(s);
        }
    }
    if let Some(s) = inv.cursor() {
        add(s);
    }
    m.into_iter().collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    #[test]
    fn never_panics_1_8(ops in prop::collection::vec(op_strategy(true), 0..200)) {
        run(Version::V1_8, ops, true);
    }

    #[test]
    fn never_panics_1_12(ops in prop::collection::vec(op_strategy(true), 0..200)) {
        run(Version::V1_12, ops, true);
    }

    #[test]
    fn clicks_conserve_items_1_12(ops in prop::collection::vec(op_strategy(false), 0..200)) {
        run(Version::V1_12, ops, false);
    }

    #[test]
    fn never_panics_1_16(ops in prop::collection::vec(op_strategy(true), 0..200)) {
        run(Version::V1_16_5, ops, true);
    }

    #[test]
    fn clicks_conserve_items_1_8(ops in prop::collection::vec(op_strategy(false), 0..200)) {
        run(Version::V1_8, ops, false);
    }

    #[test]
    fn clicks_conserve_items_1_16(ops in prop::collection::vec(op_strategy(false), 0..200)) {
        run(Version::V1_16_5, ops, false);
    }
}
