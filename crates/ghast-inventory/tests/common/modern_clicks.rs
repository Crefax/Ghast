// Shared body of the modern click replays (mounted by oracle_clicks_26_1.rs / _26_2.rs, which
// define V, LABEL, CLICKS, CREATIVE, ENV, DECLARE_RECIPES, SET_CREATIVE_SLOT and read_stack).

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};

use bytes::{Bytes, BytesMut};
use ghast_inventory::modern::{self, HashRegistries};
use ghast_inventory::{Click, DragKind, DragStage, Inventory, Outgoing, Stack};
use ghast_protocol::RawPacket;
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
    let s = read_stack(&mut r).unwrap().map(Stack::from_v775);
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

/// A hashed stack: item, count, sorted (type, hash) with the last value of a type winning,
/// sorted removed types.
#[allow(clippy::type_complexity)]
fn hashed(s: &Stack, reg: &HashRegistries) -> Option<(i32, i32, Vec<(i32, Option<i32>)>, Vec<i32>)> {
    if s.count() <= 0 || s.id() == 0 {
        return None;
    }
    let mut c: Vec<(i32, Option<i32>)> = Vec::new();
    let mut rm = Vec::new();
    if let Some(p) = s.components() {
        let all: Vec<(i32, Bytes)> = p.iter().collect();
        for (t, b) in all.iter().rev() {
            if !c.iter().any(|x| x.0 == *t) {
                c.push((*t, modern::component_hash(V, *t, b, reg)));
            }
        }
        rm = p.removed().collect();
    }
    c.sort_unstable();
    rm.sort_unstable();
    rm.dedup();
    Some((s.id(), s.count(), c, rm))
}

/// What the server compares: item, count, component hashes (the hashed stack).
fn same(a: Option<&Stack>, b: Option<&Stack>, reg: &HashRegistries) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => hashed(x, reg) == hashed(y, reg),
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

/// The `menu` of a mount screen case (`horse_screen_open`).
const MOUNT: i32 = 1000;

fn load(inv: &mut Inventory, c: &Value, window: i32, menu: i32, stage: &Value) {
    let carried = stack(&stage["carried"]);
    let state = stage["state_id"].as_i64().unwrap() as i32;
    let w0 = stacks(&stage["w0"]);
    if menu < 0 {
        inv.on_window_items_modern(0, state, w0, carried);
    } else {
        inv.on_window_items_modern(0, 0, w0, None);
        inv.on_window_items_modern(window, state, stacks(&stage["screen"]), carried);
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
                CLICKS;
            Box::new(BufReader::new(flate2::read::GzDecoder::new(VECTORS)))
        }
    };
    let mut reg = HashRegistries::default();
    let mut recipes: Option<Bytes> = None;
    let mut sync: Vec<(bool, Bytes)> = Vec::new();
    let mut st = Stats::default();
    for line in reader.lines() {
        let line = line.unwrap();
        let c: Value = serde_json::from_str(&line).unwrap();
        match c["type"].as_str().unwrap() {
            "update_recipes" => recipes = Some(hex(c["packet"].as_str().unwrap())),
            "registry_data" => sync.push((false, hex(c["packet"].as_str().unwrap()))),
            "update_tags" => sync.push((true, hex(c["packet"].as_str().unwrap()))),
            "registry" => {
                let names: Vec<&str> = c["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e.as_str().unwrap())
                    .collect();
                reg.add(c["name"].as_str().unwrap(), names);
            }
            "case" => case(&c, &reg, recipes.as_ref().unwrap(), &sync, &mut st),
            _ => {}
        }
    }
    st
}

/// The anvil / loom / trade state a resync needs: the menu's data slots after the click.
fn load_data(inv: &mut Inventory, window: i32, stage: &Value) {
    if let Some(d) = stage["slots_data"].as_array() {
        for (k, v) in d.iter().enumerate() {
            inv.on_window_property(window, k as i16, v.as_i64().unwrap() as i16);
        }
    }
}

fn case(c: &Value, reg: &HashRegistries, recipes: &Bytes, sync: &[(bool, Bytes)], st: &mut Stats) {
    let mut inv = Inventory::new(V).unwrap();
    inv.set_registries_modern(reg.clone());
    inv.set_click_settle_ticks(1);
    inv.apply_raw_modern(&RawPacket::new(DECLARE_RECIPES, recipes.clone()))
        .expect("update_recipes decodes");
    for (tags, b) in sync {
        if *tags {
            inv.on_update_tags_modern(b.clone());
        } else {
            inv.on_registry_data_modern(b.clone());
        }
    }
    inv.set_creative_modern(c["creative"].as_bool().unwrap());
    inv.set_experience_level_modern(c["xp"].as_i64().unwrap() as i32);
    inv.on_held_slot(c["selected"].as_i64().unwrap() as i8);
    let menu = c["menu"].as_i64().unwrap() as i32;
    let window = c["window"].as_i64().unwrap() as i32;
    let name = if menu < 0 {
        "player".to_string()
    } else if menu == MOUNT {
        format!("mount:{}", c["mount"]["type"].as_str().unwrap())
    } else {
        modern::menu_name(V, menu).unwrap().to_string()
    };
    if menu == MOUNT {
        let m = &c["mount"];
        inv.on_mount_screen_modern(
            window,
            m["columns"].as_i64().unwrap() as i32,
            m["type_id"].as_i64().unwrap() as i32,
            m["saddle"].as_bool().unwrap(),
            m["body"].as_bool().unwrap(),
        );
    } else if menu >= 0 {
        inv.on_open_screen_modern(window, menu, "".into());
    } else {
        inv.tick();
        let _ = inv.open_inventory();
    }
    load(&mut inv, c, window, menu, c);
    // as the server sends them: content, then the data slots, then the offers
    if let Some(d) = c["data"].as_array() {
        for (k, v) in d.iter().enumerate() {
            inv.on_window_property(window, k as i16, v.as_i64().unwrap() as i16);
        }
    }
    if let Some(o) = c["offers"].as_str() {
        inv.on_merchant_offers_modern(&hex(o));
    }
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
                load_data(&mut inv, window, k);
            }
            continue;
        }
        inv.tick();
        inv.tick();
        if mode >= 100 {
            // a screen action: trade picked, loom pattern button, anvil name
            let out = match mode {
                100 => Some(inv.select_trade_modern(i32::from(slot)).unwrap()),
                101 => inv
                    .select_loom_pattern_modern(i32::from(slot))
                    .unwrap_or(None),
                _ => match k["name"].as_str() {
                    Some(n) => inv.anvil_rename_modern(n).unwrap(),
                    None => None,
                },
            };
            let mut ours: Vec<u8> = Vec::new();
            match &out {
                Some(Outgoing::SelectTrade(i)) => {
                    ours.push(3);
                    let mut b = BytesMut::new();
                    ghast_protocol::McWrite::put_varint(&mut b, *i);
                    ours.extend_from_slice(&b);
                }
                Some(Outgoing::ButtonClick { window_id, button }) => {
                    ours.push(4);
                    let mut b = BytesMut::new();
                    ghast_protocol::McWrite::put_varint(&mut b, *window_id);
                    ghast_protocol::McWrite::put_varint(&mut b, *button);
                    ours.extend_from_slice(&b);
                }
                Some(Outgoing::RenameItem(n)) => {
                    ours.push(5);
                    let mut b = BytesMut::new();
                    ghast_protocol::McWrite::put_varint(&mut b, n.len() as i32);
                    b.extend_from_slice(n.as_bytes());
                    ours.extend_from_slice(&b);
                }
                None => {}
                other => panic!("{other:?}"),
            }
            let want: Vec<u8> = k["packets"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|p| hex(p.as_str().unwrap()).to_vec())
                .collect();
            let e = st.per_menu.entry(name.clone()).or_default();
            e[0] += 1;
            st.predicted += 1;
            let mut problems = Vec::new();
            if ours != want {
                problems.push(format!(
                    "action packets ghast {} client {}",
                    hexs(&ours),
                    hexs(&want)
                ));
            } else {
                st.bytes_exact += 1;
            }
            if let Err(e) = compare(&inv, k, menu, reg) {
                problems.push(e);
            }
            if !problems.is_empty() {
                st.diffs += 1;
                if st.first_diffs.len() < 25 {
                    st.first_diffs.push(format!(
                        "{} ({name}) action {slot}/{mode}: {}",
                        c["case"],
                        problems.join("; ")
                    ));
                }
                load(&mut inv, c, window, menu, k);
                load_data(&mut inv, window, k);
            }
            continue;
        }
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
            load_data(&mut inv, window, k);
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
        let body = inv.encode_modern(&out).unwrap().body;
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
            load_data(&mut inv, window, k);
            skip_drag = k["drag"].as_i64().unwrap() != 0;
        }
    }
}

fn hexs(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn report(st: &Stats) {
    println!(
        "{LABEL} clicks: {} ({} predicted, {} unpredicted, {} skipped in desynced drags), {} byte-identical, {} differences",
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
fn golden_clicks_match_the_real_client() {
    let st = replay(None);
    report(&st);
    assert!(st.clicks > 10_000, "too few vectors");
    assert_eq!(st.diffs, 0, "ghast differs from the real client");
}

#[test]
fn big_click_runs() {
    let Ok(path) = std::env::var(ENV) else {
        return;
    };
    let st = replay(Some(&path));
    report(&st);
    assert_eq!(st.diffs, 0);
}

/// The creative inventory screen (oracle `creative-clicks`, `clicks_creative_seed1.jsonl.gz`:
/// the real `CreativeModeInventoryScreen.slotClicked` with its listener on the player's
/// `InventoryMenu`): opening it, every click and every server `container_set_slot` while it
/// is open must make ghast send the same `set_creative_mode_slot` / `swing` packets byte for
/// byte, and leave the same window 0 and cursor.
#[test]
fn golden_creative_screen_matches_the_real_client() {
    use ghast_inventory::CreativePacket;
    const VECTORS: &[u8] =
        CREATIVE;
    let reader = BufReader::new(flate2::read::GzDecoder::new(VECTORS));
    let reg = HashRegistries::default();
    let (mut cases, mut events, mut packets, mut diffs) = (0, 0, 0, Vec::new());
    let encoder = Inventory::new(V).unwrap();
    let encode = |p: &CreativePacket| -> Vec<u8> {
        let raw = encoder.encode_creative_modern(p).unwrap();
        if raw.id == SET_CREATIVE_SLOT {
            let mut b = vec![1];
            b.extend_from_slice(&raw.body);
            b
        } else {
            vec![2]
        }
    };
    let java = |v: &Value| -> Vec<Vec<u8>> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let b = hex(p.as_str().unwrap());
                // a swing is tag 2 + its type name
                if b[0] == 2 { vec![2] } else { b.to_vec() }
            })
            .collect()
    };
    for line in reader.lines() {
        let c: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if c["type"] != "creative" {
            continue;
        }
        cases += 1;
        let mut inv = Inventory::new(V).unwrap();
        inv.set_creative_modern(true);
        inv.on_held_slot(c["selected"].as_i64().unwrap() as i8);
        // the menu's last broadcast: open and close the screen once over those stacks
        let last = stacks(&c["last"]);
        inv.on_window_items_modern(0, 0, last, None);
        inv.tick();
        let _ = inv.open_inventory().unwrap();
        inv.tick();
        inv.close().unwrap();
        inv.tick();
        inv.on_window_items_modern(
            0,
            c["state_id"].as_i64().unwrap() as i32,
            stacks(&c["w0"]),
            None,
        );
        let open = match inv.open_inventory().unwrap() {
            Some(Outgoing::Creative(p)) => p,
            None => Vec::new(),
            other => panic!("{other:?}"),
        };
        let ours: Vec<Vec<u8>> = open.iter().map(encode).collect();
        packets += ours.len();
        if ours != java(&c["open"]) {
            diffs.push(format!(
                "{} open: ghast {} packets, client {}",
                c["case"],
                ours.len(),
                java(&c["open"]).len()
            ));
        }
        for e in c["events"].as_array().unwrap() {
            events += 1;
            inv.tick();
            let ours: Vec<CreativePacket> = if let Some(slot) = e["server_slot"].as_i64() {
                inv.on_set_slot_modern(0, 1, slot as i16, stack(&e["item"]));
                inv.take_creative_echo_modern()
            } else {
                let slot = e["slot"].as_i64().unwrap() as i16;
                let button = e["button"].as_i64().unwrap() as u8;
                let mode = e["mode"].as_i64().unwrap() as u8;
                match inv.click(click_of(slot, button, mode)) {
                    Ok(Outgoing::Creative(p)) => p,
                    other => {
                        diffs.push(format!(
                            "{} click {slot}/{button}/{mode}: {other:?}",
                            c["case"]
                        ));
                        break;
                    }
                }
            };
            let ours: Vec<Vec<u8>> = ours.iter().map(encode).collect();
            packets += ours.len();
            let want = java(&e["packets"]);
            let mut problems = Vec::new();
            if ours != want {
                problems.push(format!(
                    "packets ghast {:?} client {:?}",
                    ours.iter().map(|b| hexs(b)).collect::<Vec<_>>(),
                    want.iter().map(|b| hexs(b)).collect::<Vec<_>>()
                ));
            }
            for (i, w) in stacks(&e["w0"]).iter().enumerate() {
                if !same(inv.window0_slot(i), w.as_ref(), &reg) {
                    problems.push(format!("w0 slot {i}"));
                }
            }
            if !same(inv.cursor(), stack(&e["carried"]).as_ref(), &reg) {
                problems.push("carried".into());
            }
            if !problems.is_empty() {
                diffs.push(format!(
                    "{} event {e_short}: {}",
                    c["case"],
                    problems.join("; "),
                    e_short = e
                        .get("mode")
                        .map_or("server".to_string(), |m| m.to_string())
                ));
                break;
            }
        }
    }
    println!(
        "{LABEL} creative screen: {cases} cases, {events} events, {packets} packets, {} differences",
        diffs.len()
    );
    for d in diffs.iter().take(20) {
        println!("DIFF {d}");
    }
    assert!(cases >= 1000);
    assert!(diffs.is_empty());
}
