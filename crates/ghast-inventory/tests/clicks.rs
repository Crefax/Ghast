//! Click prediction against hand-derived vanilla expectations (1.8.9 `Container.slotClick`
//! and 1.16.5 `ScreenHandler.onSlotClick` traced by hand for each case), and the confirm
//! protocol.

use ghast_inventory::{
    Click, ClickError, ClickOutcome, ClickPacket, Inventory, OUTSIDE, Outgoing, Stack, Version,
    WindowKind,
};

const STONE: i32 = 1;
const DIRT: i32 = 3;
const PLANKS: i32 = 5;
const WOOL: i32 = 35;
const PUMPKIN: i32 = 86;
const DIAMOND_SWORD: i32 = 276;
const STICK: i32 = 280;
const IRON_CHESTPLATE: i32 = 307;
const DIAMOND_HELMET: i32 = 310;
const ENDER_PEARL: i32 = 368;

fn s(id: i32, count: i32) -> Stack {
    Stack::new(id, count, 0)
}

fn sd(id: i32, count: i32, damage: i16) -> Stack {
    Stack::new(id, count, damage)
}

fn id16(name: &str) -> i32 {
    i32::from(ghast_data::v1_16_5::Item::from_name(name).unwrap().id())
}

fn s16(name: &str, count: i32) -> Stack {
    Stack::new(id16(name), count, 0)
}

/// Window 0 with the given slots and the player GUI open.
fn player_inv(v: Version, slots: &[(usize, Stack)]) -> Inventory {
    let mut inv = Inventory::new(v);
    let n = if v == Version::V1_8 { 45 } else { 46 };
    let mut items = vec![None; n];
    for (i, st) in slots {
        items[*i] = Some(st.clone());
    }
    inv.on_window_items(0, items);
    inv.tick();
    inv.open_inventory().unwrap();
    inv.tick();
    inv
}

/// A container window of `kind` (id 1) with `container` contents and window-0 player slots.
fn window_inv(
    v: Version,
    kind: WindowKind,
    container: &[(usize, Stack)],
    player: &[(usize, Stack)],
) -> Inventory {
    let mut inv = player_inv(v, player);
    inv.close().unwrap();
    inv.tick();
    inv.on_open_window(1, Some(kind), "{}".into());
    let n = kind.container_len(v);
    let mut items = vec![None; n + 36];
    for (i, st) in container {
        items[*i] = Some(st.clone());
    }
    // copy window 0's main + hotbar into the window's player part
    for i in 0..36 {
        items[n + i] = inv.window0_slot(9 + i).cloned();
    }
    inv.on_window_items(1, items);
    inv.tick();
    inv
}

/// Click, expect a packet, accept it.
fn click(inv: &mut Inventory, c: Click) -> ClickPacket {
    inv.tick();
    match inv.click(c) {
        Ok(Outgoing::Click(p)) => {
            assert_eq!(inv.on_transaction(p.window_id as i8, p.action, true), None);
            p
        }
        other => panic!("{c:?}: {other:?}"),
    }
}

fn count(inv: &Inventory, slot: usize) -> Option<(i32, i32)> {
    inv.slot(slot).map(|s| (s.id, s.count))
}

fn cursor(inv: &Inventory) -> Option<(i32, i32)> {
    inv.cursor().map(|s| (s.id, s.count))
}

fn item(p: &ClickPacket) -> Option<(i32, i32)> {
    p.item.as_ref().map(|s| (s.id, s.count))
}

const BOTH: [Version; 2] = [Version::V1_8, Version::V1_16_5];

// ------------------------------------------------------------------ PICKUP (mode 0)

#[test]
fn pickup_take_split_place() {
    for v in BOTH {
        let mut inv = player_inv(v, &[(36, s(STONE, 7)), (37, s(STONE, 60))]);
        // right click takes half rounded up
        let p = click(
            &mut inv,
            Click::Pickup {
                slot: 36,
                right: true,
            },
        );
        assert_eq!((p.slot, p.button, p.mode, p.action), (36, 1, 0, 1));
        assert_eq!(
            item(&p),
            Some((STONE, 7)),
            "clicked item = slot before the click"
        );
        assert_eq!(
            (cursor(&inv), count(&inv, 36)),
            (Some((STONE, 4)), Some((STONE, 3)))
        );
        // right click on an empty slot puts one
        let p = click(
            &mut inv,
            Click::Pickup {
                slot: 38,
                right: true,
            },
        );
        assert_eq!((item(&p), p.action), (None, 2));
        assert_eq!(
            (cursor(&inv), count(&inv, 38)),
            (Some((STONE, 3)), Some((STONE, 1)))
        );
        // left click merges into 60 up to 64
        let p = click(
            &mut inv,
            Click::Pickup {
                slot: 37,
                right: false,
            },
        );
        assert_eq!(item(&p), Some((STONE, 60)));
        assert_eq!((cursor(&inv), count(&inv, 37)), (None, Some((STONE, 63))));
        // left click takes the whole stack, then puts it down in an empty slot
        click(
            &mut inv,
            Click::Pickup {
                slot: 37,
                right: false,
            },
        );
        assert_eq!(cursor(&inv), Some((STONE, 63)));
        click(
            &mut inv,
            Click::Pickup {
                slot: 9,
                right: false,
            },
        );
        assert_eq!((cursor(&inv), count(&inv, 9)), (None, Some((STONE, 63))));
    }
}

#[test]
fn pickup_merge_overflow_and_swap() {
    for v in BOTH {
        let mut inv = player_inv(
            v,
            &[(9, s(STONE, 60)), (10, s(STONE, 10)), (11, s(DIRT, 5))],
        );
        click(
            &mut inv,
            Click::Pickup {
                slot: 10,
                right: false,
            },
        );
        // 60 + 10: 4 fit, 6 stay on the cursor
        click(
            &mut inv,
            Click::Pickup {
                slot: 9,
                right: false,
            },
        );
        assert_eq!(
            (cursor(&inv), count(&inv, 9)),
            (Some((STONE, 6)), Some((STONE, 64)))
        );
        // different item: swap
        let p = click(
            &mut inv,
            Click::Pickup {
                slot: 11,
                right: false,
            },
        );
        assert_eq!(item(&p), Some((DIRT, 5)));
        assert_eq!(
            (cursor(&inv), count(&inv, 11)),
            (Some((DIRT, 5)), Some((STONE, 6)))
        );
        // right click: one item into an empty slot, then one more onto the same item
        click(
            &mut inv,
            Click::Pickup {
                slot: 12,
                right: true,
            },
        );
        click(
            &mut inv,
            Click::Pickup {
                slot: 12,
                right: true,
            },
        );
        assert_eq!(
            (cursor(&inv), count(&inv, 12)),
            (Some((DIRT, 3)), Some((DIRT, 2)))
        );
    }
}

#[test]
fn v1_8_damage_matters_when_merging() {
    // white and orange wool: different damage = different item for PICKUP
    let mut inv = player_inv(
        Version::V1_8,
        &[(9, sd(WOOL, 10, 0)), (10, sd(WOOL, 10, 1))],
    );
    click(
        &mut inv,
        Click::Pickup {
            slot: 9,
            right: false,
        },
    );
    click(
        &mut inv,
        Click::Pickup {
            slot: 10,
            right: false,
        },
    );
    assert_eq!(
        inv.cursor().map(|s| (s.damage, s.count)),
        Some((1, 10)),
        "swapped"
    );
    assert_eq!(inv.slot(10).map(|s| (s.damage, s.count)), Some((0, 10)));
}

#[test]
fn pickup_outside_drops() {
    for v in BOTH {
        let mut inv = player_inv(v, &[(9, s(STONE, 3))]);
        click(
            &mut inv,
            Click::Pickup {
                slot: 9,
                right: false,
            },
        );
        let p = click(
            &mut inv,
            Click::Pickup {
                slot: OUTSIDE,
                right: true,
            },
        );
        assert_eq!((p.slot, item(&p)), (-999, None));
        assert_eq!(cursor(&inv), Some((STONE, 2)));
        click(
            &mut inv,
            Click::Pickup {
                slot: OUTSIDE,
                right: false,
            },
        );
        assert_eq!(cursor(&inv), None);
        // empty cursor outside: the GUI sends mode 4, nothing happens
        let p = click(
            &mut inv,
            Click::Drop {
                slot: OUTSIDE,
                all: false,
            },
        );
        assert_eq!((p.mode, p.slot, item(&p)), (4, -999, None));
    }
}

#[test]
fn armor_slots() {
    let mut inv = player_inv(
        Version::V1_8,
        &[
            (9, s(DIRT, 5)),
            (10, s(PUMPKIN, 2)),
            (11, s(DIAMOND_HELMET, 1)),
        ],
    );
    // dirt does not go into the helmet slot (5)
    click(
        &mut inv,
        Click::Pickup {
            slot: 9,
            right: false,
        },
    );
    click(
        &mut inv,
        Click::Pickup {
            slot: 5,
            right: false,
        },
    );
    assert_eq!((cursor(&inv), count(&inv, 5)), (Some((DIRT, 5)), None));
    click(
        &mut inv,
        Click::Pickup {
            slot: 9,
            right: false,
        },
    );
    // a pumpkin does, one at a time
    click(
        &mut inv,
        Click::Pickup {
            slot: 10,
            right: false,
        },
    );
    click(
        &mut inv,
        Click::Pickup {
            slot: 5,
            right: false,
        },
    );
    assert_eq!(
        (cursor(&inv), count(&inv, 5)),
        (Some((PUMPKIN, 1)), Some((PUMPKIN, 1)))
    );
    assert_eq!(inv.armor(0).map(|s| s.id), Some(PUMPKIN));
    // the chestplate slot refuses the pumpkin
    click(
        &mut inv,
        Click::Pickup {
            slot: 6,
            right: false,
        },
    );
    assert_eq!(count(&inv, 6), None);
}

// ------------------------------------------------------------------ QUICK_MOVE (mode 1)

#[test]
fn shift_click_window0() {
    for v in BOTH {
        let mut inv = player_inv(
            v,
            &[
                (9, s(STONE, 30)),
                (36, s(DIRT, 10)),
                (20, s(DIRT, 60)),
                (40, s(STONE, 50)),
            ],
        );
        // main -> hotbar: merges into hotbar stone (50 + 14), rest into the first empty hotbar slot
        let p = click(
            &mut inv,
            Click::QuickMove {
                slot: 9,
                right: false,
            },
        );
        assert_eq!(
            (count(&inv, 40), count(&inv, 37), count(&inv, 9)),
            (Some((STONE, 64)), Some((STONE, 16)), None)
        );
        let expect = if v == Version::V1_8 {
            Some((STONE, 30))
        } else {
            None
        };
        assert_eq!(
            item(&p),
            expect,
            "1.8 returns the first transfer, 1.16 EMPTY when the slot emptied"
        );
        // hotbar -> main: dirt 10 merges into main dirt 60 (4) and the rest into slot 9
        let p = click(
            &mut inv,
            Click::QuickMove {
                slot: 36,
                right: false,
            },
        );
        assert_eq!(
            (count(&inv, 20), count(&inv, 9), count(&inv, 36)),
            (Some((DIRT, 64)), Some((DIRT, 6)), None)
        );
        assert_eq!(p.mode, 1);
    }
}

#[test]
fn shift_click_partial_returns_the_stack_on_both() {
    // Hotbar full except room for 4 stone: 4 move, 60 stay. 1.8 returns the first transfer's
    // copy; 1.16 loops while the slot still holds stone and returns that copy. Both: 64 stone.
    for v in BOTH {
        let mut slots = vec![(9, s(STONE, 64))];
        for i in 36..44 {
            slots.push((i, s(DIRT, 64)));
        }
        slots.push((44, s(STONE, 60)));
        let mut inv = player_inv(v, &slots);
        let p = click(
            &mut inv,
            Click::QuickMove {
                slot: 9,
                right: false,
            },
        );
        assert_eq!(item(&p), Some((STONE, 64)));
        assert_eq!(
            (count(&inv, 9), count(&inv, 44)),
            (Some((STONE, 60)), Some((STONE, 64)))
        );
    }
}

#[test]
fn shift_click_armor() {
    let mut inv = player_inv(
        Version::V1_8,
        &[(9, s(DIAMOND_HELMET, 1)), (10, s(DIAMOND_HELMET, 1))],
    );
    click(
        &mut inv,
        Click::QuickMove {
            slot: 9,
            right: false,
        },
    );
    assert_eq!(count(&inv, 5), Some((DIAMOND_HELMET, 1)));
    // helmet slot taken: the second one goes to the hotbar
    click(
        &mut inv,
        Click::QuickMove {
            slot: 10,
            right: false,
        },
    );
    assert_eq!(count(&inv, 36), Some((DIAMOND_HELMET, 1)));
    // from the armor slot back into the inventory (9..45, first empty = 9)
    click(
        &mut inv,
        Click::QuickMove {
            slot: 5,
            right: false,
        },
    );
    assert_eq!(
        (count(&inv, 9), count(&inv, 5)),
        (Some((DIAMOND_HELMET, 1)), None)
    );
    // a pumpkin is no ArmorItem: no shift into the helmet slot on 1.8
    let mut inv = player_inv(Version::V1_8, &[(9, s(PUMPKIN, 1))]);
    click(
        &mut inv,
        Click::QuickMove {
            slot: 9,
            right: false,
        },
    );
    assert_eq!(
        (count(&inv, 5), count(&inv, 36)),
        (None, Some((PUMPKIN, 1)))
    );
    // ... but on 1.16 a carved pumpkin's preferred slot is the head
    let mut inv = player_inv(
        Version::V1_16_5,
        &[
            (9, s16("carved_pumpkin", 1)),
            (10, s16("elytra", 1)),
            (11, s16("shield", 1)),
        ],
    );
    click(
        &mut inv,
        Click::QuickMove {
            slot: 9,
            right: false,
        },
    );
    click(
        &mut inv,
        Click::QuickMove {
            slot: 10,
            right: false,
        },
    );
    click(
        &mut inv,
        Click::QuickMove {
            slot: 11,
            right: false,
        },
    );
    assert_eq!(inv.armor(0).map(|s| s.id), Some(id16("carved_pumpkin")));
    assert_eq!(inv.armor(1).map(|s| s.id), Some(id16("elytra")));
    assert_eq!(inv.offhand().map(|s| s.id), Some(id16("shield")));
}

#[test]
fn shift_click_chest() {
    for v in BOTH {
        let kind = if v == Version::V1_8 {
            WindowKind::from_v1_8("minecraft:chest", 27).unwrap()
        } else {
            WindowKind::from_v1_16_5(2).unwrap()
        };
        let mut inv = window_inv(
            v,
            kind,
            &[(0, s(STONE, 64)), (1, s(DIRT, 5))],
            &[(9, s(DIRT, 60))],
        );
        assert_eq!(inv.slot_count(), 63);
        // chest -> player: reverse order, hotbar end (62) first
        let p = click(
            &mut inv,
            Click::QuickMove {
                slot: 0,
                right: false,
            },
        );
        assert_eq!(
            (p.window_id, p.action),
            (1, 1),
            "the window's own action counter"
        );
        assert_eq!((count(&inv, 62), count(&inv, 0)), (Some((STONE, 64)), None));
        assert_eq!(
            inv.hotbar(8).map(|s| s.count),
            Some(64),
            "same PlayerInventory slot"
        );
        // chest dirt 5: 4 merge into the player's dirt 60 (window slot 27 = main 9), the last one
        // goes into the last empty player slot (61, reverse order)
        let p = click(
            &mut inv,
            Click::QuickMove {
                slot: 1,
                right: false,
            },
        );
        assert_eq!(p.action, 2);
        assert_eq!(
            (count(&inv, 27), count(&inv, 61), count(&inv, 1)),
            (Some((DIRT, 64)), Some((DIRT, 1)), None)
        );
        // player -> chest: forward into the first chest slot
        click(
            &mut inv,
            Click::QuickMove {
                slot: 62,
                right: false,
            },
        );
        assert_eq!((count(&inv, 0), count(&inv, 62)), (Some((STONE, 64)), None));
    }
}

#[test]
fn v1_16_crafting_table_shift_fills_the_grid_first() {
    let v = Version::V1_16_5;
    let mut inv = window_inv(v, WindowKind::Crafting, &[], &[(9, s16("oak_planks", 10))]);
    // window slot 10 = main 9
    click(
        &mut inv,
        Click::QuickMove {
            slot: 10,
            right: false,
        },
    );
    assert_eq!(count(&inv, 1), Some((id16("oak_planks"), 10)));
    assert!(!inv.result_known(), "the server sends the new result");
    inv.tick();
    assert_eq!(
        inv.click(Click::Pickup {
            slot: 0,
            right: false
        }),
        Err(ClickError::Unpredictable)
    );
    inv.on_set_slot(1, 0, Some(s16("crafting_table", 1)));
    assert!(inv.result_known());
    // 1.8 crafting table: player -> hotbar/main, never the grid
    let mut inv = window_inv(
        Version::V1_8,
        WindowKind::Crafting,
        &[],
        &[(9, s(PLANKS, 10))],
    );
    click(
        &mut inv,
        Click::QuickMove {
            slot: 10,
            right: false,
        },
    );
    assert_eq!(
        (count(&inv, 1), count(&inv, 37)),
        (None, Some((PLANKS, 10)))
    );
}

#[test]
fn shulker_box_refuses_shulker_boxes() {
    let v = Version::V1_16_5;
    let mut inv = window_inv(
        v,
        WindowKind::ShulkerBox,
        &[],
        &[(9, s16("red_shulker_box", 1)), (10, s16("stone", 3))],
    );
    // window slot 27 = main 9
    let p = click(
        &mut inv,
        Click::QuickMove {
            slot: 27,
            right: false,
        },
    );
    assert_eq!(item(&p), None);
    assert_eq!(count(&inv, 0), None);
    click(
        &mut inv,
        Click::Pickup {
            slot: 27,
            right: false,
        },
    );
    click(
        &mut inv,
        Click::Pickup {
            slot: 0,
            right: false,
        },
    );
    assert_eq!(count(&inv, 0), None);
    click(
        &mut inv,
        Click::Pickup {
            slot: 27,
            right: false,
        },
    );
    click(
        &mut inv,
        Click::QuickMove {
            slot: 28,
            right: false,
        },
    );
    assert_eq!(count(&inv, 0), Some((id16("stone"), 3)));
}

#[test]
fn furnace() {
    for v in BOTH {
        let kind = if v == Version::V1_8 {
            WindowKind::from_v1_8("minecraft:furnace", 3).unwrap()
        } else {
            WindowKind::from_v1_16_5(13).unwrap()
        };
        let mut inv = window_inv(v, kind, &[(2, s(STONE, 5))], &[(9, s(DIRT, 3))]);
        // output -> player (reverse: hotbar end = 38)
        click(
            &mut inv,
            Click::QuickMove {
                slot: 2,
                right: false,
            },
        );
        assert_eq!(count(&inv, 38), Some((STONE, 5)));
        // player -> furnace needs smelting recipes and fuel: refused
        inv.tick();
        assert_eq!(
            inv.click(Click::QuickMove {
                slot: 3,
                right: false
            }),
            Err(ClickError::Unpredictable)
        );
        // nothing can be put into the output
        click(
            &mut inv,
            Click::Pickup {
                slot: 3,
                right: false,
            },
        );
        click(
            &mut inv,
            Click::Pickup {
                slot: 2,
                right: false,
            },
        );
        assert_eq!(count(&inv, 2), None);
        // the fuel slot's rules are not modelled
        inv.tick();
        assert_eq!(
            inv.click(Click::Pickup {
                slot: 1,
                right: false
            }),
            Err(ClickError::Unpredictable)
        );
        // the input slot takes anything
        click(
            &mut inv,
            Click::Pickup {
                slot: 0,
                right: false,
            },
        );
        assert_eq!(count(&inv, 0), Some((DIRT, 3)));
    }
}

// ------------------------------------------------------------------ SWAP (mode 2)

#[test]
fn number_key_swap_window0() {
    for v in BOTH {
        let mut inv = player_inv(v, &[(9, s(STONE, 5)), (36, s(DIRT, 7))]);
        let p = click(&mut inv, Click::Swap { slot: 9, hotbar: 0 });
        assert_eq!((p.button, p.mode, item(&p)), (0, 2, None));
        assert_eq!(
            (count(&inv, 9), count(&inv, 36)),
            (Some((DIRT, 7)), Some((STONE, 5)))
        );
        // into an empty hotbar slot
        click(&mut inv, Click::Swap { slot: 9, hotbar: 4 });
        assert_eq!((count(&inv, 9), count(&inv, 40)), (None, Some((DIRT, 7))));
    }
}

#[test]
fn v1_8_number_key_swap_with_a_chest_puts_the_hotbar_item_into_the_inventory() {
    // 1.8: the chest slot is not the player's, so the hotbar item does not go into the chest:
    // it is inserted into the player's inventory and the chest slot empties.
    let kind = WindowKind::Chest { rows: 1 };
    let mut inv = window_inv(
        Version::V1_8,
        kind,
        &[(0, s(STONE, 5))],
        &[(36, s(DIRT, 7)), (9, s(DIRT, 60))],
    );
    click(&mut inv, Click::Swap { slot: 0, hotbar: 0 });
    // hotbar 0 = window slot 9 + 27 = 36
    assert_eq!(count(&inv, 36), Some((STONE, 5)));
    assert_eq!(count(&inv, 0), None);
    // dirt 7: 4 merge into the main dirt 60 (main 9 = window 9), 3 into the first empty main slot (hotbar 1 = window 37)
    assert_eq!(
        (count(&inv, 9), count(&inv, 37)),
        (Some((DIRT, 64)), Some((DIRT, 3)))
    );
    // 1.16 swaps directly
    let mut inv = window_inv(
        Version::V1_16_5,
        kind,
        &[(0, s(STONE, 5))],
        &[(36, s(DIRT, 7))],
    );
    click(&mut inv, Click::Swap { slot: 0, hotbar: 0 });
    assert_eq!(
        (count(&inv, 0), count(&inv, 36)),
        (Some((DIRT, 7)), Some((STONE, 5)))
    );
}

#[test]
fn offhand_swap_key() {
    let mut inv = player_inv(Version::V1_16_5, &[(9, s16("torch", 5))]);
    click(
        &mut inv,
        Click::Swap {
            slot: 9,
            hotbar: 40,
        },
    );
    assert_eq!(inv.offhand().map(|s| s.count), Some(5));
    assert_eq!(count(&inv, 45), Some((id16("torch"), 5)));
    let mut inv = player_inv(Version::V1_8, &[]);
    inv.tick();
    assert_eq!(
        inv.click(Click::Swap {
            slot: 9,
            hotbar: 40
        }),
        Err(ClickError::InvalidButton)
    );
    assert_eq!(
        inv.click(Click::Swap {
            slot: OUTSIDE,
            hotbar: 0
        }),
        Err(ClickError::InvalidSlot)
    );
}

// ------------------------------------------------------------------ THROW (mode 4), PICKUP_ALL (mode 6)

#[test]
fn drop_keys() {
    for v in BOTH {
        let mut inv = player_inv(v, &[(9, s(STONE, 5)), (10, s(DIRT, 5))]);
        let p = click(
            &mut inv,
            Click::Drop {
                slot: 9,
                all: false,
            },
        );
        assert_eq!((p.button, p.mode, item(&p)), (0, 4, None));
        assert_eq!(count(&inv, 9), Some((STONE, 4)));
        click(
            &mut inv,
            Click::Drop {
                slot: 10,
                all: true,
            },
        );
        assert_eq!(count(&inv, 10), None);
        // with a cursor stack the drop key does nothing
        click(
            &mut inv,
            Click::Pickup {
                slot: 9,
                right: false,
            },
        );
        click(&mut inv, Click::Drop { slot: 9, all: true });
        assert_eq!(cursor(&inv), Some((STONE, 4)));
    }
}

#[test]
fn double_click_gathers() {
    for v in BOTH {
        let mut inv = player_inv(
            v,
            &[
                (9, s(STONE, 10)),
                (10, s(STONE, 64)),
                (11, s(STONE, 20)),
                (36, s(STONE, 30)),
                (12, s(DIRT, 3)),
            ],
        );
        click(
            &mut inv,
            Click::Pickup {
                slot: 9,
                right: false,
            },
        );
        let p = click(&mut inv, Click::PickupAll { slot: 9 });
        assert_eq!((p.button, p.mode, item(&p)), (0, 6, None));
        // pass 1 skips full stacks: 10 + 20 (slot 11) + 30 (slot 36) = 60, pass 2 takes 4 from slot 10
        assert_eq!(cursor(&inv), Some((STONE, 64)));
        assert_eq!(
            (count(&inv, 11), count(&inv, 36), count(&inv, 10)),
            (None, None, Some((STONE, 60)))
        );
        assert_eq!(count(&inv, 12), Some((DIRT, 3)));
    }
}

// ------------------------------------------------------------------ crafting result

#[test]
fn v1_8_crafting_result() {
    // The 1.8 client computes the result itself whenever a grid slot is set: two planks stacked
    // vertically give 4 sticks, whatever the server put into slot 0.
    let mut inv = player_inv(
        Version::V1_8,
        &[(0, s(DIRT, 1)), (1, s(PLANKS, 3)), (3, s(PLANKS, 1))],
    );
    assert!(inv.result_known());
    assert_eq!(count(&inv, 0), Some((STICK, 4)));
    let p = click(
        &mut inv,
        Click::Pickup {
            slot: 0,
            right: false,
        },
    );
    assert_eq!(item(&p), Some((STICK, 4)));
    assert_eq!(cursor(&inv), Some((STICK, 4)));
    assert_eq!(
        (count(&inv, 1), count(&inv, 3)),
        (Some((PLANKS, 2)), None),
        "one of each consumed"
    );
    assert_eq!(count(&inv, 0), Some((143, 1)), "one plank: a wooden button");
    assert!(inv.result_known());
    // shift-click on the result crafts repeatedly while it stays the same item
    let mut inv = player_inv(Version::V1_8, &[(1, s(PLANKS, 3)), (3, s(PLANKS, 2))]);
    let p = click(
        &mut inv,
        Click::QuickMove {
            slot: 0,
            right: false,
        },
    );
    assert_eq!(item(&p), Some((STICK, 4)));
    assert_eq!(
        count(&inv, 44),
        Some((STICK, 8)),
        "two crafts into the last hotbar slot"
    );
    // stops when the result changes (one plank left: a wooden button)
    assert_eq!(
        (count(&inv, 1), count(&inv, 3), count(&inv, 0)),
        (Some((PLANKS, 1)), None, Some((143, 1)))
    );
}

#[test]
fn v1_8_cake_leaves_buckets() {
    const MILK: i32 = 335;
    const SUGAR: i32 = 353;
    const EGG: i32 = 344;
    const WHEAT: i32 = 296;
    let grid = [
        (1, s(MILK, 1)),
        (2, s(MILK, 1)),
        (3, s(MILK, 1)),
        (4, s(SUGAR, 1)),
        (5, s(EGG, 1)),
        (6, s(SUGAR, 1)),
        (7, s(WHEAT, 1)),
        (8, s(WHEAT, 1)),
        (9, s(WHEAT, 1)),
    ];
    let mut inv = window_inv(Version::V1_8, WindowKind::Crafting, &grid, &[]);
    assert_eq!(count(&inv, 0), Some((354, 1)), "cake");
    click(
        &mut inv,
        Click::Pickup {
            slot: 0,
            right: false,
        },
    );
    assert_eq!(cursor(&inv), Some((354, 1)));
    for k in 1..=3 {
        assert_eq!(count(&inv, k), Some((325, 1)), "milk buckets leave buckets");
    }
    for k in 4..=9 {
        assert_eq!(count(&inv, k), None);
    }
}

#[test]
fn closing_the_1_8_inventory_empties_the_grid() {
    let mut inv = player_inv(Version::V1_8, &[(9, s(PLANKS, 3))]);
    click(
        &mut inv,
        Click::Pickup {
            slot: 9,
            right: false,
        },
    );
    click(
        &mut inv,
        Click::Pickup {
            slot: 1,
            right: true,
        },
    );
    inv.tick();
    assert_eq!(inv.close(), Ok(Outgoing::CloseWindow(0)));
    assert_eq!((cursor(&inv), count(&inv, 1)), (None, None));
}

// ------------------------------------------------------------------ 1.16 specifics

#[test]
fn binding_curse_keeps_armor_on() {
    use ghast_protocol::nbt::{Compound, List, NamedTag, RawNbt, Tag};
    let nbt = RawNbt::from_named(&NamedTag {
        name: String::new(),
        tag: Tag::Compound(Compound(vec![(
            "Enchantments".into(),
            Tag::List(List {
                elem_type: 10,
                items: vec![Tag::Compound(Compound(vec![
                    ("id".into(), Tag::String("minecraft:binding_curse".into())),
                    ("lvl".into(), Tag::Short(1)),
                ]))],
            }),
        )])),
    })
    .unwrap();
    let mut helmet = s16("iron_helmet", 1);
    helmet.nbt = Some(nbt);
    let mut inv = player_inv(Version::V1_16_5, &[(5, helmet)]);
    let p = click(
        &mut inv,
        Click::Pickup {
            slot: 5,
            right: false,
        },
    );
    assert_eq!(item(&p), Some((id16("iron_helmet"), 1)));
    assert_eq!((cursor(&inv), inv.armor(0).is_some()), (None, true));
}

#[test]
fn v1_16_empty_stacks_are_dropped() {
    let mut inv = Inventory::new(Version::V1_16_5);
    inv.on_set_slot(0, 36, Some(s16("stone", 0)));
    assert_eq!(inv.hotbar(0), None);
    inv.on_set_slot(-2, 40, Some(s16("stone", 2)));
    assert_eq!(inv.offhand().map(|s| s.count), Some(2));
    // 1.8 keeps zero-count stacks
    let mut inv = Inventory::new(Version::V1_8);
    inv.on_set_slot(0, 36, Some(s(STONE, 0)));
    assert_eq!(inv.hotbar(0).map(|s| s.count), Some(0));
}

#[test]
fn warped_fungus_on_a_stick_does_not_stack() {
    let id = id16("warped_fungus_on_a_stick");
    let mut inv = player_inv(
        Version::V1_16_5,
        &[(9, Stack::new(id, 1, 0)), (10, Stack::new(id, 1, 0))],
    );
    click(
        &mut inv,
        Click::Pickup {
            slot: 9,
            right: false,
        },
    );
    click(
        &mut inv,
        Click::Pickup {
            slot: 10,
            right: false,
        },
    );
    assert_eq!(
        (cursor(&inv), count(&inv, 10)),
        (Some((id, 1)), Some((id, 1))),
        "no room: nothing moves"
    );
}

#[test]
fn ender_pearls_stack_to_16() {
    let mut inv = player_inv(
        Version::V1_8,
        &[(9, s(ENDER_PEARL, 10)), (10, s(ENDER_PEARL, 10))],
    );
    click(
        &mut inv,
        Click::Pickup {
            slot: 9,
            right: false,
        },
    );
    click(
        &mut inv,
        Click::Pickup {
            slot: 10,
            right: false,
        },
    );
    assert_eq!(
        (cursor(&inv), count(&inv, 10)),
        (Some((ENDER_PEARL, 4)), Some((ENDER_PEARL, 16)))
    );
}

#[test]
fn damaged_tools_do_not_merge_in_shift_click() {
    let mut inv = player_inv(
        Version::V1_8,
        &[(9, sd(DIAMOND_SWORD, 1, 5)), (36, sd(DIAMOND_SWORD, 1, 5))],
    );
    click(
        &mut inv,
        Click::QuickMove {
            slot: 9,
            right: false,
        },
    );
    assert_eq!(
        (count(&inv, 36), count(&inv, 37)),
        (Some((DIAMOND_SWORD, 1)), Some((DIAMOND_SWORD, 1)))
    );
    let _ = IRON_CHESTPLATE;
}

#[test]
fn v1_8_items_without_subtypes_merge_across_damage() {
    // insertItem compares damage only for hasSubtypes items: pumpkin:1 merges into pumpkin:0
    let mut inv = player_inv(
        Version::V1_8,
        &[(9, sd(PUMPKIN, 3, 1)), (36, sd(PUMPKIN, 5, 0))],
    );
    click(
        &mut inv,
        Click::QuickMove {
            slot: 9,
            right: false,
        },
    );
    assert_eq!(inv.slot(36).map(|s| (s.damage, s.count)), Some((0, 8)));
    assert_eq!(count(&inv, 9), None);
}

// ------------------------------------------------------------------ confirm protocol

#[test]
fn one_click_in_flight_and_per_tick() {
    let mut inv = player_inv(Version::V1_8, &[(9, s(STONE, 5))]);
    inv.tick();
    let Ok(Outgoing::Click(p)) = inv.click(Click::Pickup {
        slot: 9,
        right: false,
    }) else {
        panic!()
    };
    assert_eq!(
        inv.click(Click::Pickup {
            slot: 10,
            right: false
        }),
        Err(ClickError::Pending)
    );
    inv.tick();
    assert_eq!(
        inv.click(Click::Pickup {
            slot: 10,
            right: false
        }),
        Err(ClickError::Pending)
    );
    assert_eq!(inv.on_transaction(0, p.action, true), None);
    // confirmed: the next click may go (this is a later tick than the first click)
    assert!(
        inv.click(Click::Pickup {
            slot: 10,
            right: false
        })
        .is_ok()
    );
    let ev = inv.drain_events();
    assert_eq!(ev.clicks.len(), 1);
    assert_eq!(ev.clicks[0].outcome, ClickOutcome::Accepted);
    assert!(ev.player_changed && ev.cursor_changed);
}

#[test]
fn second_click_in_the_same_tick_is_refused() {
    let mut inv = player_inv(Version::V1_16_5, &[(9, s16("stone", 5))]);
    inv.tick();
    let Ok(Outgoing::Click(p)) = inv.click(Click::Pickup {
        slot: 9,
        right: false,
    }) else {
        panic!()
    };
    inv.on_transaction(0, p.action, true);
    assert_eq!(
        inv.click(Click::Pickup {
            slot: 10,
            right: false
        }),
        Err(ClickError::SameTick)
    );
    inv.tick();
    assert!(
        inv.click(Click::Pickup {
            slot: 10,
            right: false
        })
        .is_ok()
    );
}

#[test]
fn rejected_click_waits_for_the_resync() {
    let mut inv = player_inv(Version::V1_8, &[(9, s(STONE, 5))]);
    inv.tick();
    let Ok(Outgoing::Click(p)) = inv.click(Click::Pickup {
        slot: 9,
        right: false,
    }) else {
        panic!()
    };
    // vanilla answers a rejected confirm with accepted = true, once
    assert_eq!(
        inv.on_transaction(0, p.action, false),
        Some(Outgoing::Confirm {
            window_id: 0,
            action: p.action
        })
    );
    assert!(inv.resyncing());
    inv.tick();
    assert_eq!(
        inv.click(Click::Pickup {
            slot: 9,
            right: false
        }),
        Err(ClickError::Resyncing)
    );
    // the server resends the window and the cursor
    let mut items = vec![None; 45];
    items[9] = Some(s(STONE, 5));
    inv.on_window_items(0, items);
    inv.on_set_slot(-1, -1, None);
    assert!(!inv.resyncing());
    assert_eq!((cursor(&inv), count(&inv, 9)), (None, Some((STONE, 5))));
    let p2 = click(
        &mut inv,
        Click::Pickup {
            slot: 9,
            right: false,
        },
    );
    assert_eq!(p2.action, p.action + 1);
    let ev = inv.drain_events();
    assert_eq!(
        ev.clicks.iter().map(|c| c.outcome).collect::<Vec<_>>(),
        [ClickOutcome::Rejected, ClickOutcome::Accepted]
    );
}

#[test]
fn foreign_confirms() {
    let mut inv = player_inv(Version::V1_8, &[]);
    // Grim-style ping in window 0: answered, no click affected
    assert_eq!(
        inv.on_transaction(0, -5, false),
        Some(Outgoing::Confirm {
            window_id: 0,
            action: -5
        })
    );
    // a window that is not open: vanilla ignores it
    assert_eq!(inv.on_transaction(3, 1, false), None);
    inv.close().ok();
    inv.on_open_window(3, Some(WindowKind::Chest { rows: 1 }), "{}".into());
    assert_eq!(
        inv.on_transaction(3, 1, false),
        Some(Outgoing::Confirm {
            window_id: 3,
            action: 1
        })
    );
    assert_eq!(inv.on_transaction(3, 1, true), None);
}

#[test]
fn click_timeout() {
    let mut inv = player_inv(Version::V1_8, &[(9, s(STONE, 5))]);
    inv.set_click_timeout(3);
    inv.tick();
    inv.click(Click::Pickup {
        slot: 9,
        right: false,
    })
    .unwrap();
    for _ in 0..3 {
        inv.tick();
    }
    assert!(!inv.pending());
    assert_eq!(inv.drain_events().clicks[0].outcome, ClickOutcome::TimedOut);
}

#[test]
fn screens_and_packet_order() {
    let mut inv = Inventory::new(Version::V1_8);
    inv.tick();
    assert_eq!(
        inv.click(Click::Pickup {
            slot: 9,
            right: false
        }),
        Err(ClickError::NoScreen)
    );
    assert_eq!(inv.open_inventory(), Ok(Some(Outgoing::OpenInventory)));
    assert_eq!(
        inv.click(Click::Pickup {
            slot: 9,
            right: false
        }),
        Err(ClickError::SameTick)
    );
    assert_eq!(inv.close(), Err(ClickError::SameTick));
    inv.tick();
    assert_eq!(inv.close(), Ok(Outgoing::CloseWindow(0)));
    assert_eq!(inv.open_inventory(), Err(ClickError::SameTick));
    // 1.16 sends nothing when opening the inventory
    let mut inv = Inventory::new(Version::V1_16_5);
    assert_eq!(inv.open_inventory(), Ok(None));
    // the server opens a chest over it, then closes it
    inv.on_open_window(
        7,
        Some(WindowKind::Chest { rows: 2 }),
        "{\"text\":\"x\"}".into(),
    );
    assert_eq!(
        inv.window().map(|w| (w.id(), w.kind())),
        Some((7, WindowKind::Chest { rows: 2 }))
    );
    assert_eq!(inv.open_inventory(), Err(ClickError::WindowOpen));
    inv.on_close_window();
    assert!(inv.window().is_none() && !inv.gui_open());
    let ev = inv.drain_events();
    assert_eq!((ev.opened, ev.closed), (Some(7), true));
}

#[test]
fn set_slot_routing() {
    let mut inv = window_inv(Version::V1_8, WindowKind::Chest { rows: 3 }, &[], &[]);
    // window 0 slot 10 while a chest is open: vanilla drops it
    inv.on_set_slot(0, 10, Some(s(STONE, 1)));
    assert_eq!(inv.window0_slot(10), None);
    // window 0 hotbar slots always reach window 0
    inv.on_set_slot(0, 36, Some(s(STONE, 2)));
    assert_eq!(inv.window0_slot(36).map(|s| s.count), Some(2));
    assert_eq!(
        count(&inv, 54),
        Some((STONE, 2)),
        "and the chest's view of hotbar 0"
    );
    // the open window's slots
    inv.on_set_slot(1, 0, Some(s(DIRT, 3)));
    assert_eq!(count(&inv, 0), Some((DIRT, 3)));
    // cursor
    inv.on_set_slot(-1, -1, Some(s(DIRT, 1)));
    assert_eq!(cursor(&inv), Some((DIRT, 1)));
    // out of range / unknown windows are ignored
    inv.on_set_slot(1, 500, Some(s(DIRT, 1)));
    inv.on_set_slot(9, 0, Some(s(DIRT, 9)));
    assert_eq!(count(&inv, 0), Some((DIRT, 3)));
    // a new window resets the action counter; the cursor is dropped client-side
    inv.on_open_window(2, Some(WindowKind::Hopper), "{}".into());
    assert_eq!(cursor(&inv), None);
    let p = click(
        &mut inv,
        Click::Pickup {
            slot: 0,
            right: false,
        },
    );
    assert_eq!((p.window_id, p.action), (2, 1));
}

#[test]
fn wire_packets() {
    let mut inv = player_inv(Version::V1_8, &[(9, s(STONE, 5))]);
    inv.tick();
    let out = inv
        .click(Click::QuickMove {
            slot: 9,
            right: false,
        })
        .unwrap();
    match out.to_v1_8() {
        ghast_protocol::v1_8::play::serverbound::ServerboundPlayPacket::WindowClick(w) => {
            assert_eq!(
                (w.window_id, w.slot, w.mouse_button, w.mode, w.action),
                (0, 9, 0, 1, 1)
            );
            assert_eq!(w.item.0.map(|i| (i.id, i.count)), Some((1, 5)));
        }
        other => panic!("{other:?}"),
    }
    assert!(Outgoing::OpenInventory.to_v1_16_5().is_none());
    assert!(matches!(
        Outgoing::Confirm { window_id: 2, action: 7 }.to_v1_16_5(),
        Some(ghast_protocol::v1_16_5::play::serverbound::ServerboundPlayPacket::Transaction(t)) if t.accepted && t.action == 7
    ));
}

#[test]
fn consume_held_and_offhand() {
    let mut inv = player_inv(Version::V1_8, &[(36, s(STONE, 2)), (37, s(DIRT, 5))]);
    inv.drain_events();
    assert!(inv.consume_held(1));
    assert_eq!(inv.hotbar(0).map(|s| s.count), Some(1));
    assert!(inv.drain_events().player_changed);
    assert!(inv.consume_held(1));
    assert_eq!(inv.hotbar(0), None, "emptied at 0");
    assert!(!inv.consume_held(1), "nothing held");
    inv.select(1);
    assert!(inv.consume_held(2));
    assert_eq!(inv.held().map(|s| s.count), Some(3));
    assert!(!inv.consume_offhand(1), "1.8 has no offhand");
    let mut inv = player_inv(Version::V1_16_5, &[(45, s16("torch", 3))]);
    assert!(inv.consume_offhand(1));
    assert_eq!(inv.offhand().map(|s| s.count), Some(2));
}
