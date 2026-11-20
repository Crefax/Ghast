//! The client view's drawn positions against the real client: replays
//! `tests/golden/<version>/interp.jsonl.gz` (oracle `InterpDump`: players, zombies and an armor
//! stand spawned by real packets, random relative-move, look, move-look, bare `entity` and
//! teleport packets handled by the real `ClientPlayNetworkHandler`, then `World.tickEntities()`)
//! and compares every entity's drawn position and rotation after every tick bit for bit.

use std::io::Read as _;

use bytes::Bytes;
use serde_json::Value;

/// One golden line per case: per tick the packets and every entity after the tick.
fn cases(path: &str) -> Vec<Value> {
    let f = std::fs::File::open(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut s = String::new();
    flate2::read::GzDecoder::new(f)
        .read_to_string(&mut s)
        .unwrap();
    s.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn f64_of(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
}

fn f32_of(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
}

fn hex(s: &str) -> Bytes {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect::<Vec<u8>>()
        .into()
}

/// Replays every case through `apply` / `tick` / `get`; panics at the first difference.
fn replay<T>(
    path: &str,
    mut new: impl FnMut() -> T,
    apply: impl Fn(&mut T, i32, &Bytes),
    tick: impl Fn(&mut T),
    get: impl Fn(&T, i32) -> Option<([f64; 3], (f32, f32))>,
) -> usize {
    let mut checked = 0;
    for case in cases(path) {
        let mut t = new();
        let n = case["case"].as_i64().unwrap();
        for (k, tk) in case["ticks"].as_array().unwrap().iter().enumerate() {
            for p in tk["packets"].as_array().unwrap() {
                apply(
                    &mut t,
                    p[0].as_i64().unwrap() as i32,
                    &hex(p[1].as_str().unwrap()),
                );
            }
            tick(&mut t);
            for e in tk["entities"].as_array().unwrap() {
                let id = e[0].as_i64().unwrap() as i32;
                let want = (
                    [f64_of(&e[1]), f64_of(&e[2]), f64_of(&e[3])],
                    (f32_of(&e[4]), f32_of(&e[5])),
                );
                let got = get(&t, id).unwrap_or_else(|| panic!("case {n} tick {k}: no {id}"));
                let same = (0..3).all(|i| got.0[i].to_bits() == want.0[i].to_bits())
                    && got.1.0.to_bits() == want.1.0.to_bits()
                    && got.1.1.to_bits() == want.1.1.to_bits();
                assert!(
                    same,
                    "case {n} tick {k} entity {id}: got {got:?}, vanilla {want:?}"
                );
                checked += 1;
            }
        }
    }
    checked
}

#[cfg(feature = "v1_8")]
#[test]
fn drawn_positions_1_8_9() {
    use ghast_entity::{Config, v1_8::Tracker};
    let n = replay(
        "../../tests/golden/1.8.9/interp.jsonl.gz",
        || Tracker::new(Config::default().with_client_motion(true)),
        |t, id, body| {
            t.apply_body(id, body).unwrap();
        },
        Tracker::tick_client,
        |t, id| t.get(id).map(|e| (e.drawn_position(), e.drawn_rotation())),
    );
    assert!(n > 10_000, "{n}");
}

#[cfg(feature = "v1_12")]
#[test]
fn drawn_positions_1_12_2() {
    use ghast_entity::{Config, v1_12::Tracker};
    let path = "../../tests/golden/1.12.2/interp.jsonl.gz";
    if !std::path::Path::new(path).exists() {
        return;
    }
    let n = replay(
        path,
        || Tracker::new(Config::default().with_client_motion(true)),
        |t, id, body| {
            t.apply_body(id, body).unwrap();
        },
        Tracker::tick_client,
        |t, id| t.get(id).map(|e| (e.drawn_position(), e.drawn_rotation())),
    );
    assert!(n > 10_000, "{n}");
}
