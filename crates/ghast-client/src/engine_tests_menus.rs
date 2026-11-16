//! The legacy special menus through the connection's packet path: the server's `open_window`
//! reaches the inventory as the version's `inventory_packet` decodes it, the inventory builds that
//! menu's model, and a behaviour's click in the menu is predicted (sent at once, slots changed
//! like the vanilla client) instead of refused. The click rules themselves are proven against
//! the vanilla client by `ghast-inventory`'s oracle replays.

use super::tests::{Script, engine, join};
use super::*;
use crate::version::{ClientVersion, V1_8, V1_12_2};
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
