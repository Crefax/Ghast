//! The legacy special menus through the connection's packet path: the server's `open_window`
//! reaches the inventory as the version's `inventory_packet` decodes it, the inventory builds that
//! menu's model, and a behaviour's click in the menu is predicted (sent at once, slots changed
//! like the vanilla client) instead of refused. The click rules themselves are proven against
//! the vanilla client by `ghast-inventory`'s oracle replays.

use super::tests::{Script, engine, join};
use super::*;
use crate::version::{ClientVersion, V1_8, V1_12_2, V1_16_5};
use ghast_inventory::{Click, SlotKind, Stack};
use ghast_protocol::RawPacket;

/// A 1.8 `open_window` as the server sends it.
fn open_1_8(id: u8, ty: &str, slots: u8) -> RawPacket {
    use ghast_protocol::v1_8::play::clientbound as cb;
    RawPacket::from_packet(&cb::OpenWindow {
        window_id: id,
        inventory_type: ty.into(),
        window_title: "{\"text\":\"\"}".into(),
        slot_count: slots,
        entity_id: None,
    })
    .unwrap()
}

/// A 1.12.2 `open_window` as the server sends it.
fn open_1_12(id: u8, ty: &str, slots: u8) -> RawPacket {
    use ghast_protocol::v1_12_2::play::clientbound as cb;
    RawPacket::from_packet(&cb::OpenWindow {
        window_id: id,
        inventory_type: ty.into(),
        window_title: "{\"text\":\"\"}".into(),
        slot_count: slots,
        entity_id: None,
    })
    .unwrap()
}

/// A 1.16.5 `open_window` (menu type registry id) as the server sends it.
fn open_1_16(id: i32, menu: i32) -> RawPacket {
    use ghast_protocol::v1_16_5::play::clientbound as cb;
    RawPacket::from_packet(&cb::OpenWindow {
        window_id: id,
        inventory_type: menu,
        window_title: "{\"text\":\"\"}".into(),
    })
    .unwrap()
}

/// A 1.16.5 item registry id.
fn id16(name: &str) -> i32 {
    i32::from(ghast_data::v1_16_5::Item::from_name(name).unwrap().id())
}

/// Opens the window through the connection's path and fills it; returns the engine.
fn opened<V: ClientVersion>(
    v: GameVersion,
    raw: &RawPacket,
    id: i32,
    container: Vec<Option<Stack>>,
    player: &[(usize, Stack)],
) -> Engine<Box<dyn Sim>> {
    let mut e = engine(v);
    join(&mut e);
    assert!(
        V::inventory_packet(e.inventory_mut(), raw)
            .unwrap()
            .is_none()
    );
    e.handle(Inbound::OpenWindow(id));
    let n = container.len();
    let mut items = container;
    items.resize(n + 36, None);
    for (k, s) in player {
        // PlayerInventory index k: main 9..36 at n + k - 9, hotbar 0..9 at n + 27 + k
        let at = if *k < 9 { n + 27 + k } else { n + k - 9 };
        items[at] = Some(s.clone());
    }
    e.inventory_mut().on_window_items(id, items);
    e
}

/// Runs `click` on the first tick and returns what the tick sent.
fn click_once(e: &mut Engine<Box<dyn Sim>>, click: Click) -> Vec<Outbound> {
    let t0 = std::time::Instant::now();
    e.note_inbound(t0);
    let mut done = false;
    let mut b = Script(|ctx: &mut TickContext<'_>| {
        if !done {
            done = true;
            ctx.click(click)
                .unwrap_or_else(|r| panic!("{click:?} refused: {r:?}"));
        }
    });
    e.tick(&mut b, t0);
    let _ = b;
    std::mem::take(e.outbound())
}

fn clicked(out: &[Outbound]) -> bool {
    out.iter()
        .any(|o| matches!(o, Outbound::Inventory(ghast_inventory::Outgoing::Click(_))))
}

/// 1.8.9 brewing stand: shift-clicking nether wart from the hotbar fills the empty ingredient
/// slot; a stack of glass bottles lands whole in the first bottle slot (1.8's `insertItem`).
#[test]
fn brewing_stand_1_8_9() {
    let mut e = opened::<V1_8>(
        GameVersion::V1_8_9,
        &open_1_8(3, "minecraft:brewing_stand", 4),
        3,
        vec![None, Some(Stack::new(373, 1, 16)), None, None],
        &[(0, Stack::new(372, 5, 0)), (1, Stack::new(374, 10, 0))],
    );
    let inv = &e.core.inv;
    assert_eq!(inv.slot_kind(0), Some(SlotKind::Potion));
    assert_eq!(inv.slot_kind(3), Some(SlotKind::BrewingIngredient));
    // hotbar 0 is screen slot 4 + 27
    let out = click_once(
        &mut e,
        Click::QuickMove {
            slot: 31,
            right: false,
        },
    );
    assert!(clicked(&out), "{out:?}");
    let inv = &e.core.inv;
    assert_eq!(inv.slot(3).map(|s| (s.id(), s.count())), Some((372, 5)));
    assert!(inv.slot(31).is_none());
    // the click's confirm, then the bottles
    let p = 3i8;
    let _ = e.inventory_mut().on_transaction(p, 1, true);
    let out = click_once(
        &mut e,
        Click::QuickMove {
            slot: 32,
            right: false,
        },
    );
    assert!(clicked(&out), "{out:?}");
    let inv = &e.core.inv;
    assert_eq!(inv.slot(0).map(|s| (s.id(), s.count())), Some((374, 10)));
    assert!(inv.slot(32).is_none());
}

/// 1.12.2 brewing stand: blaze powder is an ingredient first (slot 3 takes it even when not
/// empty); a stack of more than one glass bottle skips the bottle slots; a single splash potion
/// goes to the first bottle slot.
#[test]
fn brewing_stand_1_12_2() {
    let mut e = opened::<V1_12_2>(
        GameVersion::V1_12_2,
        &open_1_12(4, "minecraft:brewing_stand", 5),
        4,
        vec![None, None, None, None, None],
        &[
            (0, Stack::new(377, 3, 0)),
            (1, Stack::new(374, 10, 0)),
            (2, Stack::new(438, 1, 0)),
        ],
    );
    assert_eq!(e.core.inv.slot_kind(4), Some(SlotKind::BrewingFuel));
    // hotbar k is screen slot 5 + 27 + k
    let out = click_once(
        &mut e,
        Click::QuickMove {
            slot: 32,
            right: false,
        },
    );
    assert!(clicked(&out), "{out:?}");
    assert_eq!(
        e.core.inv.slot(3).map(|s| (s.id(), s.count())),
        Some((377, 3))
    );
    let _ = e.inventory_mut().on_transaction(4, 1, true);
    let out = click_once(
        &mut e,
        Click::QuickMove {
            slot: 33,
            right: false,
        },
    );
    assert!(clicked(&out), "{out:?}");
    let inv = &e.core.inv;
    assert!(inv.slot(0).is_none());
    // hotbar -> main inventory (first free slot: screen 5)
    assert_eq!(inv.slot(5).map(|s| (s.id(), s.count())), Some((374, 10)));
    let _ = e.inventory_mut().on_transaction(4, 2, true);
    let out = click_once(
        &mut e,
        Click::QuickMove {
            slot: 34,
            right: false,
        },
    );
    assert!(clicked(&out), "{out:?}");
    assert_eq!(e.core.inv.slot(0).map(|s| s.id()), Some(438));
}

/// 1.16.5 brewing stand (menu 10): blaze powder goes to the fuel slot first; with the fuel slot
/// full it goes on as an ingredient.
#[test]
fn brewing_stand_1_16_5() {
    let blaze = id16("blaze_powder");
    let mut e = opened::<V1_16_5>(
        GameVersion::V1_16_5,
        &open_1_16(2, 10),
        2,
        vec![None, None, None, None, None],
        &[(0, Stack::new(blaze, 64, 0)), (1, Stack::new(blaze, 5, 0))],
    );
    let out = click_once(
        &mut e,
        Click::QuickMove {
            slot: 32,
            right: false,
        },
    );
    assert!(clicked(&out), "{out:?}");
    assert_eq!(e.core.inv.slot(4).map(Stack::count), Some(64));
    assert!(e.core.inv.slot(32).is_none());
    let _ = e.inventory_mut().on_transaction(2, 1, true);
    let out = click_once(
        &mut e,
        Click::QuickMove {
            slot: 33,
            right: false,
        },
    );
    assert!(clicked(&out), "{out:?}");
    assert_eq!(e.core.inv.slot(3).map(Stack::count), Some(5));
}

/// Shift-clicks hotbar 0 (lapis) and hotbar 1 (a tool) into an open enchanting table; returns the
/// table's two slots afterwards.
fn enchant_two(mut e: Engine<Box<dyn Sim>>, window: i8) -> (Option<Stack>, Option<Stack>) {
    assert_eq!(e.core.inv.slot_kind(0), Some(SlotKind::EnchantItem));
    assert_eq!(e.core.inv.slot_kind(1), Some(SlotKind::Lapis));
    // hotbar k is screen slot 2 + 27 + k
    for (n, slot) in [29i16, 30].into_iter().enumerate() {
        let out = click_once(&mut e, Click::QuickMove { slot, right: false });
        assert!(clicked(&out), "{out:?}");
        let _ = e.inventory_mut().on_transaction(window, n as i16 + 1, true);
    }
    let inv = &e.core.inv;
    (inv.slot(0).cloned(), inv.slot(1).cloned())
}

/// The enchanting table on every legacy version: lapis to the lapis slot, one tool to the item
/// slot.
#[test]
fn enchanting_table_legacy() {
    let lapis_8 = Stack::new(351, 10, 4);
    let e = opened::<V1_8>(
        GameVersion::V1_8_9,
        &open_1_8(5, "minecraft:enchanting_table", 0),
        5,
        vec![None, None],
        &[(0, lapis_8.clone()), (1, Stack::new(278, 1, 0))],
    );
    let (item, lapis) = enchant_two(e, 5);
    assert_eq!(item.map(|s| s.id()), Some(278));
    assert_eq!(lapis.map(|s| s.count()), Some(10));
    let e = opened::<V1_12_2>(
        GameVersion::V1_12_2,
        &open_1_12(6, "minecraft:enchanting_table", 0),
        6,
        vec![None, None],
        &[(0, lapis_8), (1, Stack::new(278, 1, 0))],
    );
    let (item, lapis) = enchant_two(e, 6);
    assert_eq!(item.map(|s| s.id()), Some(278));
    assert_eq!(lapis.map(|s| s.count()), Some(10));
    let pick = id16("iron_pickaxe");
    let e = opened::<V1_16_5>(
        GameVersion::V1_16_5,
        &open_1_16(7, 12),
        7,
        vec![None, None],
        &[
            (0, Stack::new(id16("lapis_lazuli"), 10, 0)),
            (1, Stack::new(pick, 1, 0)),
        ],
    );
    let (item, lapis) = enchant_two(e, 7);
    assert_eq!(item.map(|s| s.id()), Some(pick));
    assert_eq!(lapis.map(|s| s.count()), Some(10));
}

/// Runs `click` on the first tick; returns the refusal, if any.
fn try_click(e: &mut Engine<Box<dyn Sim>>, click: Click) -> Result<(), ClickRefused> {
    let t0 = std::time::Instant::now();
    e.note_inbound(t0);
    let mut r = None;
    let mut b = Script(|ctx: &mut TickContext<'_>| {
        if r.is_none() {
            r = Some(ctx.click(click).map(|_| ()));
        }
    });
    e.tick(&mut b, t0);
    let _ = b;
    e.outbound().clear();
    r.unwrap()
}

/// 1.8.9 anvil: an input click is predicted; the output the client computes is unknown to ghast
/// (refused) until the server's `set_slot`, and taking it (level cost) is always refused.
#[test]
fn anvil_1_8_9() {
    let mut e = opened::<V1_8>(
        GameVersion::V1_8_9,
        &open_1_8(8, "minecraft:anvil", 0),
        8,
        vec![None, None, None],
        &[(0, Stack::new(276, 1, 30)), (1, Stack::new(264, 3, 0))],
    );
    assert_eq!(e.core.inv.slot_kind(2), Some(SlotKind::AnvilOutput));
    // hotbar k is screen slot 3 + 27 + k
    try_click(
        &mut e,
        Click::QuickMove {
            slot: 30,
            right: false,
        },
    )
    .unwrap();
    assert_eq!(e.core.inv.slot(0).map(Stack::id), Some(276));
    assert!(!e.core.inv.result_known());
    let _ = e.inventory_mut().on_transaction(8, 1, true);
    let pick = Click::Pickup {
        slot: 2,
        right: false,
    };
    let refused = try_click(&mut e, pick);
    assert!(
        matches!(
            refused,
            Err(ClickRefused::Inventory(
                ghast_inventory::ClickError::Unpredictable
            ))
        ),
        "{refused:?}"
    );
    // the server's output
    e.inventory_mut()
        .on_set_slot(8, 2, Some(Stack::new(276, 1, 0)));
    assert!(e.core.inv.result_known());
    assert!(try_click(&mut e, pick).is_err());
    // the second input still takes the diamonds
    try_click(
        &mut e,
        Click::QuickMove {
            slot: 31,
            right: false,
        },
    )
    .unwrap();
    assert_eq!(e.core.inv.slot(1).map(Stack::count), Some(3));
}

/// Shift-clicks hotbar 0 (a stack of payment items) and hotbar 1 (a single one) in an open
/// beacon; returns the payment slot and hotbar 0's new screen slot content afterwards.
fn beacon_two(mut e: Engine<Box<dyn Sim>>, window: i8) -> Option<Stack> {
    assert_eq!(e.core.inv.slot_kind(0), Some(SlotKind::BeaconPayment));
    // hotbar k is screen slot 1 + 27 + k; a stack of several stays in the player part (moved
    // to the main inventory), the single item goes into the empty payment slot
    for (n, slot) in [28i16, 29].into_iter().enumerate() {
        let out = click_once(&mut e, Click::QuickMove { slot, right: false });
        assert!(clicked(&out), "{out:?}");
        let _ = e.inventory_mut().on_transaction(window, n as i16 + 1, true);
    }
    e.core.inv.slot(0).cloned()
}

/// The beacon on every legacy version: one payment item into the payment slot by shift-click,
/// a stack of them never; 1.16.5's payment inventory caps a server's stack at one.
#[test]
fn beacon_legacy() {
    let e = opened::<V1_8>(
        GameVersion::V1_8_9,
        &open_1_8(5, "minecraft:beacon", 1),
        5,
        vec![None],
        &[(0, Stack::new(264, 5, 0)), (1, Stack::new(388, 1, 0))],
    );
    assert_eq!(
        beacon_two(e, 5).map(|s| (s.id(), s.count())),
        Some((388, 1))
    );
    let e = opened::<V1_12_2>(
        GameVersion::V1_12_2,
        &open_1_12(6, "minecraft:beacon", 1),
        6,
        vec![None],
        &[(0, Stack::new(264, 5, 0)), (1, Stack::new(265, 1, 0))],
    );
    assert_eq!(
        beacon_two(e, 6).map(|s| (s.id(), s.count())),
        Some((265, 1))
    );
    let ingot = id16("netherite_ingot");
    let mut e = opened::<V1_16_5>(
        GameVersion::V1_16_5,
        &open_1_16(7, 8),
        7,
        vec![None],
        &[
            (0, Stack::new(id16("diamond"), 5, 0)),
            (1, Stack::new(ingot, 1, 0)),
        ],
    );
    e.inventory_mut()
        .on_set_slot(7, 0, Some(Stack::new(ingot, 4, 0)));
    assert_eq!(e.core.inv.slot(0).map(Stack::count), Some(1));
    e.inventory_mut().on_set_slot(7, 0, None);
    assert_eq!(
        beacon_two(e, 7).map(|s| (s.id(), s.count())),
        Some((ingot, 1))
    );
}
