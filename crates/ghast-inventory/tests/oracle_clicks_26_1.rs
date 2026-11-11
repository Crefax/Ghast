//! Replays the 26.1 click vectors of the oracle (`oracle/run.sh clicks` with `MC=26.1`,
//! `oracle/src/mc_26_1/…/ClickDump.java`): the real client's `handleContainerInput` on every
//! menu type a server opens, filled through real `container_set_content` packets, with every
//! `ContainerInput` (drags included), survival and creative mode.
//!
//! For every click ghast predicts, the encoded `container_click` body must equal the bytes the
//! real client sent (changed slots in its map order, hashed stacks, carried stack), the client
//! must have sent as many arm swings before it, and every slot, the carried stack and the
//! player inventory must equal the client's afterwards (compared as the server compares them:
//! item, count and component hashes). A click ghast sends unpredicted is counted, then the model
//! is resynchronised from the client's state (what the server's corrections do) and the replay
//! continues (skipping to the end of a drag in progress).
//!
//! Committed vectors: `tests/golden/26.1/clicks_seed1.jsonl.gz`. Bigger runs:
//! `GHAST_CLICK_VECTORS_26_1=path cargo test -p ghast-inventory --release --test oracle_clicks_26_1 -- --nocapture`.
#![cfg(feature = "v26_1")]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};

use bytes::{Bytes, BytesMut};
use ghast_inventory::v26_1::{HashRegistries, MENUS, hashed_slot};
use ghast_inventory::{Click, DragKind, DragStage, Inventory, Outgoing, Stack, Version};
use ghast_protocol::RawPacket;
use ghast_protocol::codec::{Encode, Reader};
use ghast_protocol::types::slot::v775::ItemStack;
use ghast_protocol::v26_1::play::clientbound as cb;
use serde_json::Value;

fn hex(s: &str) -> Bytes {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect::<Vec<u8>>()
        .into()
}

fn stack(h: &Value) -> Option<Stack> {
    let b = hex(h.as_str().unwrap());
    let mut r = Reader::new(&b);
    let s = ItemStack::read(&mut r).unwrap().map(Stack::from_v775);
    assert_eq!(r.remaining(), 0, "stack bytes left: {h}");
    s
}

fn stacks(a: &Value) -> Vec<Option<Stack>> {
    a.as_array().unwrap().iter().map(stack).collect()
}

fn click_of(slot: i16, button: u8, mode: u8) -> Click {
    match mode {
        0 => Click::Pickup {
            slot,
            right: button == 1,
        },
        1 => Click::QuickMove {
            slot,
            right: button == 1,
        },
        2 => Click::Swap {
            slot,
            hotbar: button,
        },
        3 => Click::Clone { slot },
        4 => Click::Drop {
            slot,
            all: button == 1,
        },
        5 => Click::QuickCraft {
            slot,
            stage: match button & 3 {
                0 => DragStage::Start,
                1 => DragStage::Slot,
                _ => DragStage::End,
            },
            kind: match (button >> 2) & 3 {
                0 => DragKind::Split,
                1 => DragKind::One,
                _ => DragKind::Clone,
            },
        },
        6 => Click::PickupAll { slot },
        m => panic!("mode {m}"),
    }
}

/// What the server compares: item, count, component hashes (the hashed stack).
fn same(a: Option<&Stack>, b: Option<&Stack>, reg: &HashRegistries) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            let (hx, _) = hashed_slot(Some(x), reg);
            let (hy, _) = hashed_slot(Some(y), reg);
            hx == hy
        }
        _ => false,
    }
}

#[derive(Default)]
struct Stats {
    clicks: usize,
    predicted: usize,
    unpredicted: usize,
    skipped: usize,
    bytes_exact: usize,
    /// predicted clicks that changed at least one slot / the carried stack / swung
    effective: usize,
    carried_changed: usize,
    swung: usize,
    diffs: usize,
    /// per menu name: [predicted, unpredicted]
    per_menu: BTreeMap<String, [usize; 2]>,
    /// per mode: [predicted, unpredicted]
    per_mode: BTreeMap<u8, [usize; 2]>,
    first_diffs: Vec<String>,
}

fn load(inv: &mut Inventory, c: &Value, window: i32, menu: i32, stage: &Value) {
    let carried = stack(&stage["carried"]);
    let state = stage["state_id"].as_i64().unwrap() as i32;
    let w0 = stacks(&stage["w0"]);
    if menu < 0 {
        inv.on_window_items_26_1(0, state, w0, carried);
    } else {
        inv.on_window_items_26_1(0, 0, w0, None);
        inv.on_window_items_26_1(window, state, stacks(&stage["screen"]), carried);
    }
    let _ = c;
}

fn compare(inv: &Inventory, stage: &Value, menu: i32, reg: &HashRegistries) -> Result<(), String> {
    let w0 = stacks(&stage["w0"]);
    if menu < 0 {
        for (i, want) in w0.iter().enumerate() {
            if !same(inv.window0_slot(i), want.as_ref(), reg) {
                return Err(format!(
                    "w0 slot {i}: ghast {:?} client {want:?}",
                    inv.window0_slot(i)
                ));
            }
        }
    } else {
        for (i, want) in stacks(&stage["screen"]).iter().enumerate() {
            if !same(inv.slot(i), want.as_ref(), reg) {
                return Err(format!(
                    "screen slot {i}: ghast {:?} client {want:?}",
                    inv.slot(i)
                ));
            }
        }
    }
    let want = stack(&stage["carried"]);
    if !same(inv.cursor(), want.as_ref(), reg) {
        return Err(format!("carried: ghast {:?} client {want:?}", inv.cursor()));
    }
    for (k, want) in stacks(&stage["inv"]).iter().enumerate() {
        if !same(inv.player(k), want.as_ref(), reg) {
            return Err(format!(
                "inventory {k}: ghast {:?} client {want:?}",
                inv.player(k)
            ));
        }
    }
    Ok(())
}

fn replay(path: Option<&str>) -> Stats {
    let reader: Box<dyn BufRead> = match path {
        Some(p) => Box::new(BufReader::new(flate2::read::GzDecoder::new(
            std::fs::File::open(p).unwrap(),
        ))),
        None => {
            const VECTORS: &[u8] =
                include_bytes!("../../../tests/golden/26.1/clicks_seed1.jsonl.gz");
            Box::new(BufReader::new(flate2::read::GzDecoder::new(VECTORS)))
        }
    };
    let mut reg = HashRegistries::default();
    let mut recipes: Option<Bytes> = None;
    let mut st = Stats::default();
    for line in reader.lines() {
        let line = line.unwrap();
        let c: Value = serde_json::from_str(&line).unwrap();
        match c["type"].as_str().unwrap() {
            "update_recipes" => recipes = Some(hex(c["packet"].as_str().unwrap())),
            "registry" => {
                let names: Vec<&str> = c["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e.as_str().unwrap())
                    .collect();
                reg.add(c["name"].as_str().unwrap(), names);
            }
            "case" => case(&c, &reg, recipes.as_ref().unwrap(), &mut st),
            _ => {}
        }
    }
    st
}

fn case(c: &Value, reg: &HashRegistries, recipes: &Bytes, st: &mut Stats) {
    let mut inv = Inventory::new(Version::V26_1).unwrap();
    inv.set_registries_26_1(reg.clone());
    inv.set_click_settle_ticks(1);
    inv.apply_raw_v26_1(&RawPacket::new(cb::id::DECLARE_RECIPES, recipes.clone()))
        .expect("update_recipes decodes");
    inv.set_creative_26_1(c["creative"].as_bool().unwrap());
    inv.set_experience_level_26_1(c["xp"].as_i64().unwrap() as i32);
    inv.on_held_slot(c["selected"].as_i64().unwrap() as i8);
    let menu = c["menu"].as_i64().unwrap() as i32;
    let window = c["window"].as_i64().unwrap() as i32;
    let name = if menu < 0 {
        "player".to_string()
    } else {
        MENUS[menu as usize].to_string()
    };
    if menu >= 0 {
        inv.on_open_screen_26_1(window, menu, "".into());
        if let Some(d) = c["data"].as_array() {
            for (k, v) in d.iter().enumerate() {
                inv.on_window_property(window, k as i16, v.as_i64().unwrap() as i16);
            }
        }
    } else {
        inv.tick();
        let _ = inv.open_inventory();
    }
    load(&mut inv, c, window, menu, c);
    if let Err(e) = compare(&inv, c, menu, reg) {
        panic!("{}: initial state does not load: {e}", c["case"]);
    }
    let mut skip_drag = false;
    for k in c["clicks"].as_array().unwrap() {
        st.clicks += 1;
        let slot = k["slot"].as_i64().unwrap() as i16;
        let button = k["button"].as_i64().unwrap() as u8;
        let mode = k["mode"].as_i64().unwrap() as u8;
        if skip_drag {
            st.skipped += 1;
            if k["drag"].as_i64().unwrap() == 0 {
                skip_drag = false;
                load(&mut inv, c, window, menu, k);
            }
            continue;
        }
        inv.tick();
        inv.tick();
        let carried_before = inv.cursor().cloned();
        let out = inv.click(click_of(slot, button, mode)).unwrap_or_else(|e| {
            panic!("{}: click {slot}/{button}/{mode} refused: {e:?}", c["case"])
        });
        let Outgoing::ContainerClick(cc) = &out else {
            panic!("not a container click: {out:?}")
        };
        let packets: Vec<Bytes> = k["packets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| hex(p.as_str().unwrap()))
            .collect();
        let swings = packets.iter().filter(|p| p[0] == 2).count();
        let java = packets
            .iter()
            .find(|p| p[0] == 0)
            .expect("a container_click");
        let e = st.per_menu.entry(name.clone()).or_default();
        let em = st.per_mode.entry(mode).or_default();
        if !cc.predicted {
            st.unpredicted += 1;
            e[1] += 1;
            em[1] += 1;
            load(&mut inv, c, window, menu, k);
            skip_drag = k["drag"].as_i64().unwrap() != 0;
            continue;
        }
        st.predicted += 1;
        e[0] += 1;
        em[0] += 1;
        if !cc.changed.is_empty() {
            st.effective += 1;
        }
        if cc.swings > 0 {
            st.swung += 1;
        }
        let mut body = BytesMut::new();
        let ghast_protocol::v26_1::play::serverbound::ServerboundPlayPacket::WindowClick(w) =
            inv.encode_v26_1(&out).unwrap()
        else {
            unreachable!()
        };
        w.encode(&mut body).unwrap();
        let mut problems = Vec::new();
        if inv.cursor() != carried_before.as_ref() {
            st.carried_changed += 1;
        }
        if body[..] == java[1..] {
            st.bytes_exact += 1;
        } else {
            problems.push(format!(
                "packet ghast {} client {}",
                hexs(&body),
                hexs(&java[1..])
            ));
        }
        if usize::from(cc.swings) != swings {
            problems.push(format!("swings ghast {} client {swings}", cc.swings));
        }
        if let Err(e) = compare(&inv, k, menu, reg) {
            problems.push(e);
        }
        if !problems.is_empty() {
            st.diffs += 1;
            if st.first_diffs.len() < 25 {
                st.first_diffs.push(format!(
                    "{} ({name}, creative {}) click {slot}/{button}/{mode}: {}",
                    c["case"],
                    c["creative"],
                    problems.join("; ")
                ));
            }
            load(&mut inv, c, window, menu, k);
            skip_drag = k["drag"].as_i64().unwrap() != 0;
        }
    }
}

fn hexs(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn report(st: &Stats) {
    println!(
        "26.1 clicks: {} ({} predicted, {} unpredicted, {} skipped in desynced drags), {} byte-identical, {} differences",
        st.clicks, st.predicted, st.unpredicted, st.skipped, st.bytes_exact, st.diffs
    );
    println!(
        "  predicted clicks that changed slots {}, the carried stack {}, swung the arm {}",
        st.effective, st.carried_changed, st.swung
    );
    for (m, [p, u]) in &st.per_menu {
        println!("  {m:<18} predicted {p:>6}  unpredicted {u:>6}");
    }
    for (m, [p, u]) in &st.per_mode {
        println!("  mode {m}             predicted {p:>6}  unpredicted {u:>6}");
    }
    for d in &st.first_diffs {
        println!("DIFF {d}");
    }
}

#[test]
fn golden_26_1_clicks_match_the_real_client() {
    let st = replay(None);
    report(&st);
    assert!(st.clicks > 10_000, "too few vectors");
    assert_eq!(st.diffs, 0, "ghast differs from the real client");
}

#[test]
fn big_26_1_click_runs() {
    let Ok(path) = std::env::var("GHAST_CLICK_VECTORS_26_1") else {
        return;
    };
    let st = replay(Some(&path));
    report(&st);
    assert_eq!(st.diffs, 0);
}
