//! Replays the vanilla click vectors of the oracle (`oracle/run.sh clicks`, `ClickDump.java`):
//! the real client's `clickSlot` on real screen handlers, random contents and click sequences.
//! For every click ghast-inventory predicts, the encoded `window_click` body must equal the bytes
//! vanilla sent (slot, button, action number, mode, clicked item incl. NBT) and every slot, the
//! cursor and the player inventory must equal vanilla's afterwards. Clicks ghast refuses as
//! unpredictable are counted, then the model is resynchronised from vanilla's state (what the
//! server's resync would do) and the replay continues.
//!
//! Committed vectors: `tests/golden/<ver>/clicks_seed1.jsonl.gz` (all windows) and
//! `clicks_craft_seed1.jsonl.gz` (crafting). Bigger runs: `GHAST_CLICK_VECTORS_1_8`,
//! `GHAST_CLICK_VECTORS_1_12`, `GHAST_CLICK_VECTORS_1_16`, `GHAST_CRAFT_VECTORS_1_8`,
//! `GHAST_CRAFT_VECTORS_1_12`, `GHAST_CRAFT_VECTORS_1_16` = path, then
//! `cargo test -p ghast-inventory --release --test oracle_clicks -- --nocapture`.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use bytes::{Bytes, BytesMut};
use ghast_inventory::{
    Click, ClickError, Inventory, Outgoing, RecipeBook, Stack, Version, WindowKind,
    from_legacy_slot, from_modern_slot,
};
use ghast_protocol::codec::{Encode, decode_exact};
use ghast_protocol::types::slot::{legacy, modern};
use ghast_protocol::v1_8::play::serverbound as sb8;
use ghast_protocol::v1_12_2::play::serverbound as sb12;
use ghast_protocol::v1_16_5::play::clientbound as cb16;
use ghast_protocol::v1_16_5::play::serverbound as sb16;
use serde_json::Value;

fn hex(s: &str) -> Bytes {
    let b: Vec<u8> = (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    Bytes::from(b)
}

fn stack(v: Version, h: &Value) -> Option<Stack> {
    let b = hex(h.as_str().unwrap());
    match v {
        Version::V1_8 | Version::V1_12 => {
            from_legacy_slot(decode_exact::<legacy::Slot>(&b).unwrap())
        }
        Version::V1_16_5 => from_modern_slot(decode_exact::<modern::Slot>(&b).unwrap()),
    }
}

fn stacks(v: Version, a: &Value) -> Vec<Option<Stack>> {
    a.as_array().unwrap().iter().map(|h| stack(v, h)).collect()
}

fn kind(v: Version, c: &Value) -> Option<WindowKind> {
    match c["kind"].as_str().unwrap() {
        "player" => None,
        _ if v == Version::V1_16_5 => WindowKind::from_v1_16_5(c["menu"].as_i64().unwrap() as i32),
        "chest" => Some(WindowKind::Chest {
            rows: c["rows"].as_u64().unwrap() as u8,
        }),
        "generic3x3" => Some(WindowKind::Generic3x3),
        "hopper" => Some(WindowKind::Hopper),
        "crafting" => Some(WindowKind::Crafting),
        "furnace" => Some(WindowKind::Furnace),
        k => panic!("unknown kind {k}"),
    }
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
        4 => Click::Drop {
            slot,
            all: button == 1,
        },
        6 => Click::PickupAll { slot },
        m => panic!("mode {m}"),
    }
}

/// Vanilla's stack equality (count, item, damage, NBT trees). 1.16.5 NBT bytes can differ in key
/// order between vanilla's own copies (`NbtCompound.copy` builds a `HashMap` of another capacity),
/// which the server's comparison ignores.
fn same(a: Option<&Stack>, b: Option<&Stack>) -> bool {
    ghast_inventory::stacks_equal(a, b)
}

#[derive(Default, Debug)]
struct Stats {
    cases: usize,
    clicks: usize,
    predicted: usize,
    byte_exact: usize,
    result_clicks: usize,
    unpredictable: usize,
    unpredictable_by: std::collections::BTreeMap<String, usize>,
    by_mode: [usize; 7],
    diffs: Vec<String>,
}

/// Loads the vanilla state into the model (window 0, the open window, cursor).
fn load(inv: &mut Inventory, v: Version, sync: i32, st: &Value) {
    inv.on_window_items(0, stacks(v, &st["w0"]));
    if !st["screen"].is_null() {
        inv.on_window_items(sync, stacks(v, &st["screen"]));
    }
    inv.on_set_slot(-1, -1, stack(v, &st["cursor"]));
}

/// Compares the model with vanilla's state; returns the differences.
fn compare(inv: &Inventory, v: Version, st: &Value, out: &mut Vec<String>) {
    let screen = if st["screen"].is_null() {
        &st["w0"]
    } else {
        &st["screen"]
    };
    let want = stacks(v, screen);
    let result_slot = matches!(
        inv.slot_kind(0),
        Some(ghast_inventory::SlotKind::CraftResult)
    );
    for (i, w) in want.iter().enumerate() {
        if i == 0 && result_slot && !inv.result_known() {
            continue; // the client computes it from recipes (1.8) / the server sends it (1.16)
        }
        if !same(inv.slot(i), w.as_ref()) {
            out.push(format!(
                "slot {i}: ghast {:?}, vanilla {:?}",
                inv.slot(i),
                w
            ));
        }
    }
    if !st["screen"].is_null() {
        // window 0 behind the container: crafting grid and armor (slot 0 = result skipped)
        for (i, w) in stacks(v, &st["w0"]).iter().enumerate().skip(1) {
            if !same(inv.window0_slot(i), w.as_ref()) {
                out.push(format!(
                    "w0 slot {i}: ghast {:?}, vanilla {:?}",
                    inv.window0_slot(i),
                    w
                ));
            }
        }
    }
    let c = stack(v, &st["cursor"]);
    if !same(inv.cursor(), c.as_ref()) {
        out.push(format!("cursor: ghast {:?}, vanilla {:?}", inv.cursor(), c));
    }
    for (k, w) in stacks(v, &st["inv"]).iter().enumerate() {
        if !same(inv.player(k), w.as_ref()) {
            out.push(format!(
                "inv {k}: ghast {:?}, vanilla {:?}",
                inv.player(k),
                w
            ));
        }
    }
}

fn replay_case(v: Version, c: &Value, s: &mut Stats, book: &RecipeBook) {
    let name = c["case"].as_str().unwrap().to_owned();
    let sync = c["sync"].as_i64().unwrap() as i32;
    let mut inv = Inventory::new(v);
    inv.on_declare_recipes(book.clone());
    inv.on_held_slot(c["selected"].as_i64().unwrap() as i8);
    let kind = kind(v, c);
    inv.on_window_items(0, stacks(v, &c["w0"]));
    inv.tick();
    match kind {
        None => {
            inv.open_inventory().unwrap();
        }
        Some(k) => inv.on_open_window(sync, Some(k), "{}".into()),
    }
    load(&mut inv, v, sync, c);
    let mut d = Vec::new();
    compare(&inv, v, c, &mut d);
    assert!(d.is_empty(), "{name}: initial state not loaded: {d:?}");
    s.cases += 1;
    for (n, k) in c["clicks"].as_array().unwrap().iter().enumerate() {
        inv.tick();
        s.clicks += 1;
        let (slot, button, mode) = (
            k["slot"].as_i64().unwrap() as i16,
            k["button"].as_u64().unwrap() as u8,
            k["mode"].as_u64().unwrap() as u8,
        );
        let click = click_of(slot, button, mode);
        let at = format!("{name} click {n} {click:?}");
        match inv.click(click) {
            Ok(Outgoing::Click(p)) => {
                s.predicted += 1;
                let diffs_before = s.diffs.len();
                s.by_mode[usize::from(mode)] += 1;
                let mut body = BytesMut::new();
                let want = hex(k["packet"].as_str().unwrap());
                // vanilla's packet fields; the item compared like the server does
                let (vslot, vbutton, vaction, vmode, vitem) = match v {
                    Version::V1_8 => {
                        p.to_v1_8().encode(&mut body).unwrap();
                        let w: sb8::WindowClick = decode_exact(&want).unwrap();
                        (
                            w.slot,
                            w.mouse_button,
                            w.action,
                            w.mode,
                            from_legacy_slot(w.item),
                        )
                    }
                    Version::V1_12 => {
                        p.to_v1_12_2().encode(&mut body).unwrap();
                        let w: sb12::WindowClick = decode_exact(&want).unwrap();
                        (
                            w.slot,
                            w.mouse_button,
                            w.action,
                            w.mode,
                            from_legacy_slot(w.item),
                        )
                    }
                    Version::V1_16_5 => {
                        p.to_v1_16_5().encode(&mut body).unwrap();
                        let w: sb16::WindowClick = decode_exact(&want).unwrap();
                        (
                            w.slot,
                            w.mouse_button,
                            w.action,
                            w.mode,
                            from_modern_slot(w.item),
                        )
                    }
                };
                let fields_ok = (vslot, vbutton, vaction, vmode)
                    == (p.slot, p.button as i8, p.action, p.mode as i8)
                    && same(p.item.as_ref(), vitem.as_ref());
                if body[..] == want[..] {
                    s.byte_exact += 1;
                }
                // 1.8 copies keep NBT key order: require the exact bytes there
                if !fields_ok || (v == Version::V1_8 && body[..] != want[..]) {
                    s.diffs.push(format!(
                        "{at}: packet ghast {:02x?} vanilla {:02x?} (item ghast {:?})",
                        &body[..],
                        &want[..],
                        p.item
                    ));
                }
                let mut d = Vec::new();
                compare(&inv, v, k, &mut d);
                for x in d {
                    s.diffs.push(format!("{at}: {x}"));
                }
                inv.on_transaction(p.window_id as i8, p.action, true);
                if slot == 0
                    && p.item.is_some()
                    && inv.slot_kind(0) == Some(ghast_inventory::SlotKind::CraftResult)
                {
                    s.result_clicks += 1;
                }
                if let Some(h) = k["server"].as_str() {
                    // the server's new crafting result (set_slot)
                    inv.on_set_slot(sync, 0, stack(v, &Value::String(h.to_owned())));
                }
                if s.diffs.len() > diffs_before {
                    // resync so one difference does not cascade
                    load(&mut inv, v, sync, k);
                }
            }
            Err(ClickError::Unpredictable) => {
                s.unpredictable += 1;
                let what = format!("{} mode {mode}", c["kind"].as_str().unwrap());
                *s.unpredictable_by.entry(what).or_default() += 1;
                // Vanilla clicked anyway (and used an action number ghast did not): the rest of
                // the case would differ in action numbers, so it ends here.
                return; // the action counter is now one behind vanilla's: end the case here
            }
            other => s.diffs.push(format!("{at}: refused {other:?}")),
        }
    }
}

fn replay(v: Version, path: &Path) -> Stats {
    let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let r = BufReader::new(flate2::read::GzDecoder::new(f));
    let mut s = Stats::default();
    let mut book = RecipeBook::default();
    for line in r.lines() {
        let line = line.unwrap();
        let c: Value = serde_json::from_str(&line).unwrap();
        if let Some(h) = c["declare_recipes"].as_str() {
            // the recipes the (emulated) server sent: decoded by ghast-protocol like on the wire
            let p: cb16::DeclareRecipes = decode_exact(&hex(h)).unwrap();
            book = RecipeBook::from_v1_16_5(&p);
            eprintln!("declare_recipes: {} crafting recipes", book.len());
            continue;
        }
        replay_case(v, &c, &mut s, &book);
    }
    s
}

fn check(v: Version, path: PathBuf) {
    if !path.exists() {
        panic!("missing {} (oracle/run.sh clicks)", path.display());
    }
    let s = replay(v, &path);
    eprintln!(
        "{v:?} {}: {} cases, {} clicks, {} predicted ({} byte-identical packets; by mode {:?}), {} unpredictable, {} diffs",
        path.display(),
        s.cases,
        s.clicks,
        s.predicted,
        s.byte_exact,
        s.by_mode,
        s.unpredictable,
        s.diffs.len()
    );
    eprintln!(
        "  unpredictable by window and mode: {:?}",
        s.unpredictable_by
    );
    eprintln!(
        "  clicks on a crafting result that carried a stack: {}",
        s.result_clicks
    );
    for d in s.diffs.iter().take(20) {
        eprintln!("  {d}");
    }
    assert!(s.diffs.is_empty(), "{} differences", s.diffs.len());
    assert!(s.predicted > s.clicks / 2, "too few predicted clicks");
}

fn golden(ver: &str, file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden")
        .join(ver)
        .join(file)
}

fn vectors(var: &str, ver: &str, file: &str) -> PathBuf {
    std::env::var_os(var).map_or_else(|| golden(ver, file), PathBuf::from)
}

#[test]
fn vanilla_clicks_1_8_9() {
    check(
        Version::V1_8,
        vectors("GHAST_CLICK_VECTORS_1_8", "1.8.9", "clicks_seed1.jsonl.gz"),
    );
}

#[test]
fn vanilla_clicks_1_12_2() {
    check(
        Version::V1_12,
        vectors(
            "GHAST_CLICK_VECTORS_1_12",
            "1.12.2",
            "clicks_seed1.jsonl.gz",
        ),
    );
}

/// 1.12.2 crafting: the client never computes results; the emulated server's result is set
/// once after the grid is filled and stays (stale) until a click takes it.
#[test]
fn vanilla_crafting_1_12_2() {
    check(
        Version::V1_12,
        vectors(
            "GHAST_CRAFT_VECTORS_1_12",
            "1.12.2",
            "clicks_craft_seed1.jsonl.gz",
        ),
    );
}

#[test]
fn vanilla_clicks_1_16_5() {
    check(
        Version::V1_16_5,
        vectors(
            "GHAST_CLICK_VECTORS_1_16",
            "1.16.5",
            "clicks_seed1.jsonl.gz",
        ),
    );
}

/// Crafting cases: grids from real recipes, clicks on the result and the grid (1.8: results the
/// client computes; 1.16.5: recipes from `declare_recipes`, results from the emulated server).
#[test]
fn vanilla_crafting_1_8_9() {
    check(
        Version::V1_8,
        vectors(
            "GHAST_CRAFT_VECTORS_1_8",
            "1.8.9",
            "clicks_craft_seed1.jsonl.gz",
        ),
    );
}

#[test]
fn vanilla_crafting_1_16_5() {
    check(
        Version::V1_16_5,
        vectors(
            "GHAST_CRAFT_VECTORS_1_16",
            "1.16.5",
            "clicks_craft_seed1.jsonl.gz",
        ),
    );
}
