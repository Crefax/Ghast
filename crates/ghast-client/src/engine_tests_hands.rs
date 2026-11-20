//! The hands' wave of the PvP findings on every version: the right-click armor equip
//! (`ArmorItem.use` / 26.x `Equippable`), the item use's steps as events, the non-mutating
//! click check, the full-resync inventory signal and the button press preview.

use super::tests::{Script, engine, join, names};
use super::tests_combat::{Kind, put, setup};
use super::*;
use crate::common::UseEvent;
use crate::hands::{Button, PressOutcome};
use ghast_inventory::Stack;

/// Every version the client engine runs.
const VERSIONS: [GameVersion; 5] = [
    GameVersion::V1_8_9,
    GameVersion::V1_12_2,
    GameVersion::V1_16_5,
    GameVersion::V26_1,
    GameVersion::V26_2,
];

const RIGHT: Buttons = Buttons {
    left: false,
    right: true,
};

/// Item ids per version.
fn item(v: GameVersion, name: &str) -> i32 {
    match v.instance() {
        GameVersion::V1_8_9 | GameVersion::V1_12_2 => match name {
            "diamond_helmet" => 310,
            "diamond_chestplate" => 311,
            "iron_chestplate" => 307,
            "golden_apple" => 322,
            "stone" => 1,
            "elytra" => 443,
            "pumpkin" => 86,
            _ => unreachable!("{name}"),
        },
        GameVersion::V1_16_5 => i32::from(ghast_data::v1_16_5::Item::from_name(name).unwrap().id()),
        GameVersion::V26_2 => i32::from(ghast_data::v26_2::Item::from_name(name).unwrap().id()),
        _ => i32::from(ghast_data::v26_1::Item::from_name(name).unwrap().id()),
    }
}

/// A server slot update of window 0 (hotbar 0 = slot 36, chest = slot 6).
fn set_slot(e: &mut Engine<Box<dyn Sim>>, slot: i16, stack: Option<Stack>) {
    if e.core.opts.caps.window_confirm {
        e.core.inv.on_set_slot(0, slot, stack);
    } else {
        e.core.inv.on_set_slot_modern(0, 0, slot, stack);
    }
}

/// One tick running `act`; returns the tick's packets.
fn tick_out(
    e: &mut Engine<Box<dyn Sim>>,
    t: Instant,
    act: impl FnMut(&mut TickContext<'_>),
) -> Vec<Outbound> {
    e.outbound().clear();
    e.note_inbound(t);
    let mut b = Script(act);
    e.tick(&mut b, t);
    std::mem::take(e.outbound())
}

/// One tick running `act`; returns the tick's packet names.
fn tick_with(
    e: &mut Engine<Box<dyn Sim>>,
    t: Instant,
    act: impl FnMut(&mut TickContext<'_>),
) -> Vec<&'static str> {
    e.outbound().clear();
    e.note_inbound(t);
    let mut b = Script(act);
    e.tick(&mut b, t);
    names(e.outbound())
}

/// The packets before the movement packet (and the 26.x per-tick input packets).
fn actions(n: &[&'static str]) -> Vec<&'static str> {
    n.iter()
        .copied()
        .take_while(|x| {
            !(x.starts_with('C')
                || x.contains("position")
                || *x == "flying"
                || *x == "look"
                || *x == "player_input"
                || *x == "tick_end")
        })
        .collect()
}

/// A joined engine that ticked once (the player is in the world, looking west at nothing).
fn ready(v: GameVersion) -> Engine<Box<dyn Sim>> {
    let mut e = engine(v);
    join(&mut e);
    tick_with(&mut e, Instant::now(), |_| {});
    e
}

/// The tick's events as the behaviour sees them on the next tick.
fn next_events(e: &mut Engine<Box<dyn Sim>>, t: Instant, buttons: Buttons) -> Vec<UseEvent> {
    let mut seen = Vec::new();
    tick_with(e, t, |ctx| {
        seen = ctx.events().item_use().to_vec();
        ctx.set_buttons(buttons);
    });
    seen
}

/// A right press with armor in the hand and its slot empty puts it on at once on every
/// version: the hand empties, no swing on 1.8.9 / 1.12.2, a swing on 1.16.5 / 26.x.
#[test]
fn right_click_puts_armor_on_into_an_empty_slot() {
    let t = Instant::now();
    for v in VERSIONS {
        let mut e = ready(v);
        let chest = item(v, "diamond_chestplate");
        set_slot(&mut e, 36, Some(Stack::new(chest, 1, 0)));
        let n = tick_with(&mut e, t, |ctx| ctx.set_buttons(RIGHT));
        let swings = v.caps().use_equip_swings;
        let want: &[&str] = if swings {
            &["block_place", "swing"]
        } else {
            &["block_place"]
        };
        assert_eq!(actions(&n), want, "{v}: {n:?}");
        assert!(e.core.inv.held().is_none(), "{v}: the hand is empty");
        assert_eq!(
            e.core.inv.armor(1).map(Stack::id),
            Some(chest),
            "{v}: the chestplate is on"
        );
        // The UseItem packet still carried the stack (it goes out before the use).
        assert!(!e.core.hands.using_item(), "{v}");
        // Held: re-used every 4 ticks, the empty hand sends nothing more.
        for _ in 0..5 {
            let n = tick_with(&mut e, t, |ctx| ctx.set_buttons(RIGHT));
            assert!(!actions(&n).contains(&"block_place"), "{v}: {n:?}");
        }
        assert_eq!(
            swings,
            !matches!(v, GameVersion::V1_8_9 | GameVersion::V1_12_2)
        );
    }
}

/// With the slot taken: 1.8.9 – 1.16.5 only send the use (FAIL, nothing moves); 26.x swaps the
/// pieces (SUCCESS, a swing), unless the worn piece is the same item.
#[test]
fn right_click_with_the_slot_taken() {
    let t = Instant::now();
    for v in VERSIONS {
        let mut e = ready(v);
        let (diamond, iron) = (item(v, "diamond_chestplate"), item(v, "iron_chestplate"));
        set_slot(&mut e, 36, Some(Stack::new(diamond, 1, 0)));
        set_slot(&mut e, 6, Some(Stack::new(iron, 1, 0)));
        let n = tick_with(&mut e, t, |ctx| ctx.set_buttons(RIGHT));
        let modern = matches!(v, GameVersion::V26_1 | GameVersion::V26_2);
        if modern {
            assert_eq!(actions(&n), ["block_place", "swing"], "{v}: {n:?}");
            assert_eq!(e.core.inv.held().map(Stack::id), Some(iron), "{v}");
            assert_eq!(e.core.inv.armor(1).map(Stack::id), Some(diamond), "{v}");
        } else {
            assert_eq!(actions(&n), ["block_place"], "{v}: {n:?}");
            assert_eq!(e.core.inv.held().map(Stack::id), Some(diamond), "{v}");
            assert_eq!(e.core.inv.armor(1).map(Stack::id), Some(iron), "{v}");
        }
        // The same item worn: nothing moves on any version.
        let mut e = ready(v);
        set_slot(&mut e, 36, Some(Stack::new(iron, 1, 0)));
        set_slot(&mut e, 6, Some(Stack::new(iron, 1, 0)));
        let n = tick_with(&mut e, t, |ctx| ctx.set_buttons(RIGHT));
        assert_eq!(actions(&n), ["block_place"], "{v}: {n:?}");
        assert_eq!(e.core.inv.held().map(Stack::id), Some(iron), "{v}");
    }
}

/// What a right-click use equips, per version: the helmet on the head; the elytra from 1.12.2;
/// a pumpkin never (only clicks put it on).
#[test]
fn use_equip_targets() {
    for v in VERSIONS {
        let e = ready(v);
        let put_on = |name: &str| {
            let mut i = e.core.inv.clone();
            let s = Some(Stack::new(item(v, name), 1, 0));
            if e.core.opts.caps.window_confirm {
                i.on_set_slot(0, 36, s);
            } else {
                i.on_set_slot_modern(0, 0, 36, s);
            }
            i.use_equips(false, false)
        };
        assert!(put_on("diamond_helmet"), "{v}");
        assert!(!put_on("stone"), "{v}");
        let pumpkin = if matches!(v, GameVersion::V1_8_9 | GameVersion::V1_12_2) {
            "pumpkin"
        } else {
            "carved_pumpkin"
        };
        assert!(!put_on(pumpkin), "{v}");
        if v != GameVersion::V1_8_9 {
            assert!(put_on("elytra"), "{v}");
        }
    }
}

/// The use's steps reach the behaviour on the tick after they happened: started by the key
/// handling, finished by the server's status 9, released with the button, interrupted by a
/// slot change.
#[test]
fn item_use_events() {
    let t = Instant::now();
    for v in VERSIONS {
        let apple = item(v, "golden_apple");
        let mut e = ready(v);
        set_slot(&mut e, 36, Some(Stack::new(apple, 8, 0)));
        assert_eq!(next_events(&mut e, t, RIGHT), [], "{v}");
        assert_eq!(
            next_events(&mut e, t, RIGHT),
            [UseEvent::Started {
                item: apple,
                off_hand: false
            }],
            "{v}"
        );
        assert_eq!(next_events(&mut e, t, RIGHT), [], "{v}");
        // The server finishes it after 3 player ticks.
        e.handle(Inbound::EntityStatus {
            entity_id: 7,
            status: 9,
        });
        assert_eq!(
            next_events(&mut e, t, Buttons::default()),
            [UseEvent::Finished {
                item: apple,
                off_hand: false,
                ticks: 3
            }],
            "{v}"
        );
        assert!(!e.core.hands.using_item(), "{v}");

        // Pressed for two ticks, then released.
        assert_eq!(next_events(&mut e, t, RIGHT), [], "{v}");
        assert_eq!(
            next_events(&mut e, t, RIGHT),
            [UseEvent::Started {
                item: apple,
                off_hand: false
            }],
            "{v}"
        );
        assert_eq!(next_events(&mut e, t, Buttons::default()), [], "{v}");
        assert_eq!(
            next_events(&mut e, t, Buttons::default()),
            [UseEvent::Released {
                item: apple,
                off_hand: false,
                ticks: 2
            }],
            "{v}"
        );

        // A slot change cuts it short.
        tick_with(&mut e, t, |ctx| ctx.set_buttons(RIGHT));
        assert!(e.core.hands.using_item(), "{v}");
        let mut seen = Vec::new();
        tick_with(&mut e, t, |ctx| {
            seen = ctx.events().item_use().to_vec();
            ctx.select_slot(1);
        });
        assert_eq!(seen.len(), 1, "{v}: {seen:?}");
        assert_eq!(
            next_events(&mut e, t, Buttons::default()),
            [UseEvent::Interrupted {
                item: apple,
                off_hand: false,
                ticks: 1
            }],
            "{v}"
        );
    }
}

/// `can_click` refuses like `click` without sending anything, and opening the inventory is
/// refused while turning.
#[test]
fn can_click_and_open_inventory_wait_for_a_still_player() {
    let t = Instant::now();
    for v in VERSIONS {
        let mut e = ready(v);
        let mut r = None;
        let n = tick_with(&mut e, t, |ctx| {
            ctx.set_aim(0.0, 0.0);
            r = Some((ctx.can_click(), ctx.open_inventory()));
        });
        assert_eq!(
            r,
            Some((Err(ClickRefused::Turning), Err(ClickRefused::Turning))),
            "{v}"
        );
        assert!(!n.contains(&"inventory"), "{v}: {n:?}");
        // The turn went out in that tick's movement step: still refused on the next tick.
        tick_with(&mut e, t, |ctx| {
            r = Some((ctx.can_click(), ctx.open_inventory()));
        });
        assert_eq!(
            r,
            Some((Err(ClickRefused::Turning), Err(ClickRefused::Turning))),
            "{v}"
        );
        // Still: no screen yet, then the inventory opens and clicks may follow next tick.
        let mut c = None;
        tick_with(&mut e, t, |ctx| {
            c = Some(ctx.can_click());
            r = Some((ctx.can_click(), ctx.open_inventory()));
        });
        assert_eq!(
            c,
            Some(Err(ClickRefused::Inventory(
                ghast_inventory::ClickError::NoScreen
            ))),
            "{v}"
        );
        assert_eq!(r.map(|r| r.1), Some(Ok(())), "{v}");
        tick_with(&mut e, t, |ctx| c = Some(ctx.can_click()));
        assert_eq!(c, Some(Ok(())), "{v}");
    }
}

/// A full window 0 resync is flagged; a single slot update is not.
#[test]
fn full_inventory_resync_is_signalled() {
    let t = Instant::now();
    for v in VERSIONS {
        let mut e = ready(v);
        let stone = item(v, "stone");
        let mut seen = None;
        let read = |seen: &mut Option<(bool, bool, bool)>, ctx: &TickContext<'_>| {
            let i = &ctx.events().inventory;
            *seen = Some((i.player_synced, i.window_synced, i.player_changed));
        };
        set_slot(&mut e, 36, Some(Stack::new(stone, 1, 0)));
        tick_with(&mut e, t, |ctx| read(&mut seen, ctx));
        assert_eq!(seen, Some((false, false, true)), "{v}");
        let items = (0..46).map(|i| (i == 37).then(|| Stack::new(stone, 2, 0)));
        if e.core.opts.caps.window_confirm {
            e.core.inv.on_window_items(0, items);
        } else {
            e.core.inv.on_window_items_modern(0, 1, items, None);
        }
        tick_with(&mut e, t, |ctx| read(&mut seen, ctx));
        assert_eq!(seen, Some((true, false, true)), "{v}");
        tick_with(&mut e, t, |ctx| read(&mut seen, ctx));
        assert_eq!(seen, Some((false, false, false)), "{v}");
    }
}

/// The press preview names what the next key handling acts on, and the buttons then do that:
/// the floor below, the held item in the air, a player in front.
#[test]
fn press_preview_matches_the_key_handling() {
    let t = Instant::now();
    for v in VERSIONS {
        let mut e = setup(v);
        let apple = item(v, "golden_apple");
        tick_with(&mut e, t, |_| {});
        // Looking west at nothing, empty hands.
        let mut p = None;
        tick_with(&mut e, t, |ctx| {
            p = Some((
                ctx.press_preview(Button::Left),
                ctx.press_preview(Button::Right),
            ));
        });
        assert_eq!(
            p,
            Some((PressOutcome::Nothing, PressOutcome::Nothing)),
            "{v}"
        );
        set_slot(&mut e, 36, Some(Stack::new(apple, 8, 0)));
        tick_with(&mut e, t, |ctx| {
            p = Some((
                ctx.press_preview(Button::Left),
                ctx.press_preview(Button::Right),
            ))
        });
        assert_eq!(p, Some((PressOutcome::Nothing, PressOutcome::Item)), "{v}");

        // Straight down: the stone floor under the feet.
        e.core.forced_look = Some((90.0, 90.0));
        tick_with(&mut e, t, |_| {});
        let mut r = None;
        let out = tick_out(&mut e, t, |ctx| {
            r = Some(ctx.press_preview(Button::Right));
            ctx.set_buttons(RIGHT);
        });
        match r {
            Some(PressOutcome::Block(h)) => assert_eq!(h.pos.y, 63, "{v}"),
            o => panic!("{v}: {o:?}"),
        }
        assert!(
            out.iter()
                .any(|o| matches!(o, Outbound::Place { pos, .. } if pos.y == 63)),
            "{v}: {out:?}"
        );
        tick_with(&mut e, t, |ctx| ctx.set_buttons(Buttons::default()));

        // Level again, a player 1.5 blocks to the west at the same height.
        e.core.forced_look = Some((90.0, 0.0));
        tick_with(&mut e, t, |_| {});
        let s = e.core.pose.pos;
        put(&mut e, 50, Kind::Player, Vec3::new(s.x - 1.5, s.y, s.z));
        tick_with(&mut e, t, |_| {});
        tick_with(&mut e, t, |_| {});
        tick_with(&mut e, t, |_| {});
        let mut r = None;
        let out = tick_out(&mut e, t, |ctx| {
            r = Some(ctx.press_preview(Button::Right));
            ctx.set_buttons(RIGHT);
        });
        match r {
            Some(PressOutcome::Entity(h)) => assert_eq!(h.id, 50, "{v}"),
            o => panic!("{v}: {o:?}"),
        }
        assert!(
            out.iter()
                .any(|o| matches!(o, Outbound::UseEntity { target: 50, .. })),
            "{v}: {out:?}"
        );
    }
}
