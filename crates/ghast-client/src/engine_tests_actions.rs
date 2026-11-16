//! Server resource packs on 1.8 – 1.16 (`ClientPlayNetworkHandler.onResourcePackSend`): the
//! answers per URL and version, byte-exact. The drop and swap-hands keys
//! (`MinecraftClient.handleInputEvents`, `ClientPlayerEntity.dropSelectedItem`,
//! `LocalPlayer.drop`): packets byte-exact, the client's prediction, the server's correction and
//! the order of the actions in a tick.

use super::*;
use crate::version::{ClientVersion, V1_8, V1_12_2, V1_16_5};
use ghast_protocol::{FrameEncoder, RawPacket};

const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

/// A `resource_pack_send` (two strings).
fn prompt(id: i32, url: &str, hash: &str) -> RawPacket {
    let mut b = Vec::new();
    for s in [url, hash] {
        b.push(u8::try_from(s.len()).unwrap());
        b.extend_from_slice(s.as_bytes());
    }
    RawPacket {
        id,
        body: bytes::Bytes::from(b),
    }
}

/// The answers to a prompt, each as its framed bytes (uncompressed).
fn answer<V: ClientVersion>(id: i32, url: &str, hash: &str, accept: bool) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    V::resource_pack(&prompt(id, url, hash), accept, &mut out).unwrap();
    out.iter()
        .map(|o| {
            assert!(matches!(o, Outbound::Raw(_)), "{o:?}");
            let mut enc = FrameEncoder::new();
            V::write_play(&mut enc, o, &mut V::Wire::default()).unwrap();
            enc.take().to_vec()
        })
        .collect()
}

/// The expected frames: 1.8 `C19` = hash string + status VarInt; 1.10+ the status alone.
fn want(v: GameVersion, statuses: &[u8], hash: &str) -> Vec<Vec<u8>> {
    statuses
        .iter()
        .map(|&r| match v {
            GameVersion::V1_8_9 => {
                let h = &hash.as_bytes()[..hash.len().min(40)];
                let mut f = vec![u8::try_from(h.len() + 3).unwrap(), 0x19, h.len() as u8];
                f.extend_from_slice(h);
                f.push(r);
                f
            }
            GameVersion::V1_12_2 => vec![2, 0x18, r],
            _ => vec![2, 0x21, r],
        })
        .collect()
}

fn answer_of(v: GameVersion, url: &str, hash: &str, accept: bool) -> Vec<Vec<u8>> {
    use ghast_protocol::{v1_8, v1_12_2, v1_16_5};
    match v {
        GameVersion::V1_8_9 => answer::<V1_8>(
            v1_8::play::clientbound::id::RESOURCE_PACK_SEND,
            url,
            hash,
            accept,
        ),
        GameVersion::V1_12_2 => answer::<V1_12_2>(
            v1_12_2::play::clientbound::id::RESOURCE_PACK_SEND,
            url,
            hash,
            accept,
        ),
        _ => answer::<V1_16_5>(
            v1_16_5::play::clientbound::id::RESOURCE_PACK_SEND,
            url,
            hash,
            accept,
        ),
    }
}

const LEGACY: [GameVersion; 3] = [
    GameVersion::V1_8_9,
    GameVersion::V1_12_2,
    GameVersion::V1_16_5,
];

#[test]
fn packs_are_answered_byte_exact() {
    // (url, 1.8.9 loadable, 1.12+ loadable)
    let cases = [
        ("https://example.invalid/pack.zip", true, true),
        ("http://example.invalid/p.zip?x=%41#f", true, true),
        // A world save's pack: the bot has none.
        ("level://world/resources.zip", false, false),
        // 1.8.9 checks nothing; 1.12+ wants http, https or level (case-sensitive) and URI
        // syntax.
        ("ftp://example.invalid/pack.zip", true, false),
        ("HTTP://example.invalid/pack.zip", true, false),
        ("example.invalid/pack.zip", true, false),
        ("https://example.invalid/a pack.zip", true, false),
        ("https://example.invalid/%zz.zip", true, false),
        ("https://example.invalid/a#b#c", true, false),
        ("https://example.invalid/a\u{85}b", true, false),
        ("https://[::1]/pack.zip", true, true),
        ("https://example.invalid/[x]", true, false),
        ("https://", true, false),
        ("https:", true, false),
        // A `level:` URL that is not `level://` passes the check and takes the remote branch.
        ("level:/x/resources.zip", true, true),
        ("level:/x/../resources.zip", true, false),
        ("level:/x/pack.zip", true, false),
    ];
    for (url, on_18, on_112) in cases {
        for v in LEGACY {
            let loadable = if v == GameVersion::V1_8_9 {
                on_18
            } else {
                on_112
            };
            for accept in [true, false] {
                let statuses: &[u8] = match (loadable, accept) {
                    (false, _) => &[2],
                    (true, true) => &[3, 0],
                    (true, false) => &[1],
                };
                assert_eq!(
                    answer_of(v, url, HASH, accept),
                    want(v, statuses, HASH),
                    "{v:?} {url} accept {accept}"
                );
            }
        }
    }
}

#[test]
fn hash_is_echoed_on_1_8_only() {
    // An empty hash, a short one, and one longer than the packet's 40 characters (vanilla's
    // decoder refuses it; the answer constructor would cut it).
    let long = format!("{HASH}ff");
    for hash in ["", "abc", long.as_str()] {
        for v in LEGACY {
            let got = answer_of(v, "https://example.invalid/p.zip", hash, false);
            assert_eq!(got, want(v, &[1], hash), "{v:?} {hash}");
        }
    }
}

#[test]
fn other_packets_and_versions_push_nothing() {
    use ghast_protocol::{v1_8, v1_12_2, v1_16_5};
    let mut out = Vec::new();
    let url = "https://example.invalid/p.zip";
    V1_8::resource_pack(
        &prompt(v1_8::play::clientbound::id::CHAT, url, HASH),
        true,
        &mut out,
    )
    .unwrap();
    V1_12_2::resource_pack(
        &prompt(v1_12_2::play::clientbound::id::CHAT, url, HASH),
        true,
        &mut out,
    )
    .unwrap();
    V1_16_5::resource_pack(
        &prompt(v1_16_5::play::clientbound::id::CHAT, url, HASH),
        true,
        &mut out,
    )
    .unwrap();
    // 26.x packs have ids and arrive as `Inbound::ResourcePackPush`.
    let id = ghast_protocol::v26_1::play::clientbound::id::ADD_RESOURCE_PACK;
    crate::version::V26_1::resource_pack(&prompt(id, url, HASH), true, &mut out).unwrap();
    assert!(out.is_empty(), "{out:?}");
    // The legacy decoders leave the prompt to `resource_pack`.
    let p = prompt(v1_8::play::clientbound::id::RESOURCE_PACK_SEND, url, HASH);
    assert_eq!(V1_8::decode_play(&p, &mut ()).unwrap(), None);
    // A truncated prompt is a decode error.
    let mut bad = p.clone();
    bad.body = bad.body.slice(..5);
    assert!(V1_8::resource_pack(&bad, true, &mut out).is_err());
}

// ---- drop and swap-hands keys ------------------------------------------------------------

use super::tests::{Script, engine, join, names};
use crate::common::{DigStatus, Face};
use ghast_inventory::Stack;

/// Every version the engine runs here.
fn all_versions() -> Vec<GameVersion> {
    let mut v = vec![
        GameVersion::V1_8_9,
        GameVersion::V1_12_2,
        GameVersion::V1_16_5,
        GameVersion::V26_1,
    ];
    if cfg!(feature = "v26_2") {
        v.push(GameVersion::V26_2);
    }
    v
}

fn frame_of(v: GameVersion, o: &Outbound) -> Vec<u8> {
    let mut enc = FrameEncoder::new();
    match v {
        GameVersion::V1_8_9 => V1_8::write_play(&mut enc, o, &mut ()).unwrap(),
        GameVersion::V1_12_2 => V1_12_2::write_play(&mut enc, o, &mut Default::default()).unwrap(),
        GameVersion::V1_16_5 => V1_16_5::write_play(&mut enc, o, &mut ()).unwrap(),
        #[cfg(feature = "v26_2")]
        GameVersion::V26_2 => {
            let mut w = crate::version::Wire26_2::default();
            crate::version::V26_2::write_play(&mut enc, o, &mut w).unwrap();
        }
        _ => {
            let mut w = crate::version::Wire26_1::default();
            crate::version::V26_1::write_play(&mut enc, o, &mut w).unwrap();
        }
    }
    enc.take().to_vec()
}

#[test]
fn drop_and_swap_packets_are_byte_exact() {
    // `player_action` / `block_dig`: status VarInt, BlockPos.ORIGIN (packed 0), Direction.DOWN
    // (0); 1.19+ a sequence, 0 for these (the constructor without one).
    for v in all_versions() {
        let (id, tail): (u8, &[u8]) = match v {
            GameVersion::V1_8_9 => (0x07, &[]),
            GameVersion::V1_12_2 => (0x14, &[]),
            GameVersion::V1_16_5 => (0x1b, &[]),
            _ => (0x29, &[0]),
        };
        for (status, wire) in [
            (DigStatus::DropStack, 3),
            (DigStatus::DropItem, 4),
            (DigStatus::SwapHands, 6),
        ] {
            assert_eq!(status.id(), wire);
            let o = Outbound::Dig {
                status,
                pos: ghast_protocol::BlockPos::new(0, 0, 0),
                face: Face::Down,
            };
            let mut want = vec![0, id, wire as u8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            want.extend_from_slice(tail);
            want[0] = (want.len() - 1) as u8;
            assert_eq!(frame_of(v, &o), want, "{v:?} {status:?}");
        }
        // The swing after a predicted drop: main hand (1.8 has no hand field).
        let swing = match v {
            GameVersion::V1_8_9 => vec![1, 0x0a],
            GameVersion::V1_12_2 => vec![2, 0x1d, 0],
            GameVersion::V1_16_5 => vec![2, 0x2c, 0],
            _ => vec![2, 0x3f, 0],
        };
        assert_eq!(frame_of(v, &Outbound::Swing), swing, "{v:?}");
    }
}

/// Item ids per version.
fn item(v: GameVersion, name: &str) -> i32 {
    match v {
        GameVersion::V1_8_9 | GameVersion::V1_12_2 => match name {
            "bread" => 297,
            "stone" => 1,
            _ => unreachable!(),
        },
        GameVersion::V1_16_5 => i32::from(ghast_data::v1_16_5::Item::from_name(name).unwrap().id()),
        #[cfg(feature = "v26_2")]
        GameVersion::V26_2 => i32::from(ghast_data::v26_2::Item::from_name(name).unwrap().id()),
        _ => i32::from(ghast_data::v26_1::Item::from_name(name).unwrap().id()),
    }
}

/// A server slot update of window 0 (hotbar 0 = slot 36, off hand = slot 45).
fn set_slot(e: &mut Engine<Box<dyn Sim>>, slot: i16, stack: Option<Stack>) {
    match e.core.opts.caps.version {
        GameVersion::V1_8_9 | GameVersion::V1_12_2 | GameVersion::V1_16_5 => {
            e.core.inv.on_set_slot(0, slot, stack);
        }
        _ => e.core.inv.on_set_slot_modern(0, 0, slot, stack),
    }
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

fn held_count(e: &Engine<Box<dyn Sim>>) -> Option<i32> {
    e.core.inv.held().map(Stack::count)
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

#[test]
fn drop_predicts_like_vanilla_and_the_server_corrects() {
    for v in all_versions() {
        let predicts = matches!(
            v,
            GameVersion::V1_16_5 | GameVersion::V26_1 | GameVersion::V26_2
        );
        let mut e = engine(v);
        join(&mut e);
        let t = Instant::now();
        tick_with(&mut e, t, |_| {});
        let stone = item(v, "stone");
        set_slot(&mut e, 36, Some(Stack::new(stone, 5, 0)));

        // One item: the drop packet, then (1.16.5+) the swing; in call order with the other
        // actions, before the movement packet.
        let mut ok = false;
        let n = tick_with(&mut e, t, |ctx| {
            ctx.chat("a");
            ok = ctx.drop_held(false);
            ctx.chat("b");
        });
        assert!(ok, "{v:?}");
        let want: &[&str] = if predicts {
            &["chat", "block_dig", "swing", "chat"]
        } else {
            &["chat", "block_dig", "chat"]
        };
        assert_eq!(actions(&n), want, "{v:?} {n:?}");
        assert_eq!(held_count(&e), Some(if predicts { 4 } else { 5 }), "{v:?}");
        // The server's slot update confirms it (or sets the count back: a refused drop).
        set_slot(&mut e, 36, Some(Stack::new(stone, 4, 0)));
        assert_eq!(held_count(&e), Some(4), "{v:?}");

        // The whole stack.
        let n = tick_with(&mut e, t, |ctx| assert!(ctx.drop_held(true)));
        let mut stack_out = Vec::new();
        tick_with(&mut e, t, |_| {});
        assert_eq!(
            actions(&n),
            if predicts {
                &["block_dig", "swing"][..]
            } else {
                &["block_dig"][..]
            },
            "{v:?}"
        );
        assert_eq!(
            held_count(&e),
            if predicts { None } else { Some(4) },
            "{v:?}"
        );
        // A correction: the server kept the stack.
        set_slot(&mut e, 36, Some(Stack::new(stone, 4, 0)));
        assert_eq!(held_count(&e), Some(4), "{v:?}");
        set_slot(&mut e, 36, None);

        // An empty hand: the packet goes out, nothing is taken, no swing.
        e.outbound().clear();
        let n = tick_with(&mut e, t, |ctx| assert!(ctx.drop_held(false)));
        assert_eq!(actions(&n), ["block_dig"], "{v:?}");
        // The statuses on the wire.
        e.outbound().clear();
        e.note_inbound(t);
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            ctx.drop_held(false);
            ctx.drop_held(true);
            ctx.swap_hands();
        });
        e.tick(&mut b, t);
        stack_out.extend(e.outbound().iter().filter_map(|o| match o {
            Outbound::Dig { status, pos, face } => {
                assert_eq!((pos.x, pos.y, pos.z, *face), (0, 0, 0, Face::Down));
                Some(*status)
            }
            _ => None,
        }));
        let want: &[DigStatus] = if v == GameVersion::V1_8_9 {
            &[DigStatus::DropItem, DigStatus::DropStack]
        } else {
            &[
                DigStatus::DropItem,
                DigStatus::DropStack,
                DigStatus::SwapHands,
            ]
        };
        assert_eq!(stack_out, want, "{v:?}");
    }
}

#[test]
fn swap_predicts_nothing_and_needs_an_off_hand() {
    for v in all_versions() {
        let mut e = engine(v);
        join(&mut e);
        let t = Instant::now();
        tick_with(&mut e, t, |_| {});
        let stone = item(v, "stone");
        set_slot(&mut e, 36, Some(Stack::new(stone, 3, 0)));
        let mut ok = false;
        let n = tick_with(&mut e, t, |ctx| ok = ctx.swap_hands());
        assert_eq!(ok, v != GameVersion::V1_8_9, "{v:?}");
        if v == GameVersion::V1_8_9 {
            assert!(actions(&n).is_empty(), "{n:?}");
            continue;
        }
        assert_eq!(actions(&n), ["block_dig"], "{v:?} {n:?}");
        // Nothing moves until the server's slot updates.
        assert_eq!(held_count(&e), Some(3));
        assert!(e.core.inv.offhand().is_none());
        set_slot(&mut e, 45, Some(Stack::new(stone, 3, 0)));
        set_slot(&mut e, 36, None);
        assert!(e.core.inv.held().is_none());
        assert_eq!(e.core.inv.offhand().map(Stack::count), Some(3), "{v:?}");
    }
}

#[test]
fn keys_are_refused_in_screens_dead_and_spectating() {
    for v in all_versions() {
        let mut e = engine(v);
        join(&mut e);
        let t = Instant::now();
        tick_with(&mut e, t, |_| {});
        set_slot(&mut e, 36, Some(Stack::new(item(v, "stone"), 2, 0)));
        // The player inventory screen.
        let _ = e.core.inv.open_inventory();
        let n = tick_with(&mut e, t, |ctx| {
            assert!(!ctx.drop_held(false));
            assert!(!ctx.swap_hands());
        });
        assert!(!n.contains(&"block_dig"), "{v:?} {n:?}");
        assert_eq!(held_count(&e), Some(2));
        let _ = e.core.inv.close();
        // Spectators.
        e.handle(Inbound::GameState {
            reason: 3,
            value: 3.0,
        });
        let n = tick_with(&mut e, t, |ctx| {
            assert!(!ctx.drop_held(true));
            assert!(!ctx.swap_hands());
        });
        assert!(!n.contains(&"block_dig"), "{v:?} {n:?}");
        e.handle(Inbound::GameState {
            reason: 3,
            value: 0.0,
        });
        // Dead (the death screen).
        e.core.dead = true;
        let n = tick_with(&mut e, t, |ctx| assert!(!ctx.drop_held(false)));
        assert!(!n.contains(&"block_dig"), "{v:?} {n:?}");
        e.core.dead = false;
        let n = tick_with(&mut e, t, |ctx| assert!(ctx.drop_held(false)));
        assert!(n.contains(&"block_dig"), "{v:?} {n:?}");
    }
}

/// A use of the held item ends once its hand no longer holds that item (1.16.5
/// `tickActiveItemStack`, 26.1 `updatingUsingItem`): after a predicted drop of the whole stack
/// at once, after a swap's slot updates on the next tick; one item dropped of several keeps it.
#[test]
fn item_use_ends_with_its_stack() {
    for v in [GameVersion::V1_16_5, GameVersion::V26_1] {
        let bread = item(v, "bread");
        let start = |e: &mut Engine<Box<dyn Sim>>| {
            let t = Instant::now();
            e.handle(Inbound::Health(Health {
                health: 20.0,
                food: 10,
                saturation: 0.0,
            }));
            set_slot(e, 36, Some(Stack::new(bread, 3, 0)));
            tick_with(e, t, |ctx| {
                ctx.set_buttons(Buttons {
                    left: false,
                    right: true,
                })
            });
            assert!(e.core.hands.using_item(), "{v:?}");
        };
        let t = Instant::now();
        let mut e = engine(v);
        join(&mut e);
        tick_with(&mut e, t, |_| {});
        start(&mut e);
        // One of three: still eating.
        tick_with(&mut e, t, |ctx| assert!(ctx.drop_held(false)));
        assert!(e.core.hands.using_item(), "{v:?}");
        // The rest: the use ends this tick, silently (no release packet), the button held.
        let n = tick_with(&mut e, t, |ctx| assert!(ctx.drop_held(true)));
        assert!(!e.core.hands.using_item(), "{v:?}");
        // The drop and its swing only: no release of the use.
        assert_eq!(actions(&n), ["block_dig", "swing"], "{v:?} {n:?}");
        assert_eq!(
            n.iter().filter(|x| **x == "block_dig").count(),
            1,
            "{v:?} {n:?}"
        );

        // Swap: the server moves the bread to the off hand; the main-hand use ends.
        let mut e = engine(v);
        join(&mut e);
        tick_with(&mut e, t, |_| {});
        start(&mut e);
        tick_with(&mut e, t, |ctx| assert!(ctx.swap_hands()));
        assert!(e.core.hands.using_item(), "{v:?}");
        set_slot(&mut e, 45, Some(Stack::new(bread, 3, 0)));
        set_slot(&mut e, 36, None);
        tick_with(&mut e, t, |_| {});
        assert!(!e.core.hands.using_item(), "{v:?}");
    }
}
