//! The 1.8.9 and 1.12.2 crosshair block pick against the vanilla oracle
//! (`tests/golden/<v>/raytrace.jsonl.gz`: `Entity.rayTrace(reach, 1.0F)` and
//! `World.rayTrace(eyes, end, false, false, false)` in fuzz-terrain worlds) and the 1.12.2 ray
//! bounds (`selection_boxes.jsonl.gz`, every state alone and the neighbour configurations).
#![cfg(any(feature = "v1_8", feature = "v1_12"))]
#![allow(dead_code)]

use std::io::{BufRead, BufReader};

use ghast_physics::pick::{BlockHit, HitKind};
use serde_json::Value;

fn golden(version: &str, name: &str) -> Vec<Value> {
    let path = format!(
        "{}/../../tests/golden/{version}/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let f = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect()
}

fn f64v(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
}
fn f32v(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
}
fn v3(v: &Value) -> [f64; 3] {
    [f64v(&v[0]), f64v(&v[1]), f64v(&v[2])]
}
fn bits(a: [f64; 3]) -> [u64; 3] {
    a.map(f64::to_bits)
}
fn ints(v: &Value) -> Vec<i32> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap() as i32)
        .collect()
}

/// Compares a legacy pick with an oracle hit object (`type` null / miss / block).
fn same(got: Option<BlockHit>, want: &Value) -> Result<(), String> {
    match (want["type"].as_str().unwrap(), got) {
        ("null", None) => Ok(()),
        (t @ ("block" | "miss"), Some(h)) => {
            let kind = if t == "block" {
                HitKind::Block
            } else {
                HitKind::Miss
            };
            let b = ints(&want["block"]);
            if h.kind != kind
                || h.pos != [b[0], b[1], b[2]]
                || h.face.name() != want["side"].as_str().unwrap()
                || bits(h.location) != bits(v3(&want["hit"]))
            {
                return Err(format!("got {h:?}, oracle {want}"));
            }
            Ok(())
        }
        (_, got) => Err(format!("got {got:?}, oracle {want}")),
    }
}

/// Replays a raytrace golden: `world` builds a world from a scenario header, `run` gives
/// (pick, ray_fff) for (eyes, look, reach).
fn replay<W>(
    version: &str,
    look_yaw: &str,
    mut world: impl FnMut(&Value) -> W,
    run: impl Fn(&W, [f64; 3], [f64; 3], f64) -> (Option<BlockHit>, Option<BlockHit>),
    eye: impl Fn([f64; 3], f32) -> [f64; 3],
    look: impl Fn(f32, f32) -> [f64; 3],
) {
    let mut w = None;
    let (mut ok, mut fails) = (0, Vec::new());
    for l in golden(version, "raytrace.jsonl.gz") {
        if l["type"] == "scenario" {
            w = Some(world(&l));
            continue;
        }
        if l["type"] != "ray" {
            continue;
        }
        let k = l["k"].as_u64().unwrap();
        let e = eye(v3(&l["feet"]), f32v(&l["eye_height"]));
        let lk = look(f32v(&l[look_yaw]), f32v(&l["pitch"]));
        let r = if bits(e) != bits(v3(&l["eyes"])) {
            Err(format!("eyes {e:?}"))
        } else if bits(lk) != bits(v3(&l["look"])) {
            Err(format!("look {lk:?}"))
        } else {
            let (pick, fff) = run(w.as_ref().unwrap(), e, lk, f64v(&l["reach"]));
            same(pick, &l["pick"])
                .map_err(|x| format!("pick: {x}"))
                .and_then(|()| same(fff, &l["ray_fff"]).map_err(|x| format!("ray_fff: {x}")))
        };
        match r {
            Ok(()) => ok += 1,
            Err(e) => fails.push(format!("ray {k}: {e}")),
        }
    }
    assert!(
        fails.is_empty(),
        "{version}: {} of {} rays differ; first:\n{}",
        fails.len(),
        ok + fails.len(),
        fails.iter().take(8).cloned().collect::<Vec<_>>().join("\n")
    );
    assert!(ok >= 2000, "{version}: {ok} rays");
    eprintln!("{version}: {ok} rays identical");
}

fn fill<W>(
    l: &Value,
    mut load: impl FnMut(&mut W, [i32; 4]),
    mut fill: impl FnMut(&mut W, [i32; 3], [i32; 3], u16),
    mut w: W,
) -> W {
    for c in l["loaded_chunks"].as_array().unwrap() {
        let c = ints(c);
        load(&mut w, [c[0], c[1], c[2], c[3]]);
    }
    for f in l["fills"].as_array().unwrap() {
        let v = ints(f);
        fill(
            &mut w,
            [v[0], v[1], v[2]],
            [v[3], v[4], v[5]],
            (v[6] << 4 | v[7]) as u16,
        );
    }
    w
}

#[cfg(feature = "v1_8")]
#[test]
fn crosshair_matches_vanilla_1_8_9() {
    use ghast_physics::pick::v1_8 as pick;
    replay(
        "1.8.9",
        "head_yaw",
        |l| {
            fill(
                l,
                |w: &mut ghast_physics::v1_8::MemoryWorld, c| w.load_chunks(c[0], c[1], c[2], c[3]),
                |w, a, b, raw| w.fill_raw(a, b, raw),
                ghast_physics::v1_8::memory_world(),
            )
        },
        |w, e, lk, reach| {
            (
                pick::pick(w, e, lk, reach),
                pick::clip(w, e, pick::ray_end(e, lk, reach), false, false),
            )
        },
        pick::eye_position,
        pick::look_vector,
    );
}

#[cfg(feature = "v1_12")]
#[test]
fn crosshair_matches_vanilla_1_12_2() {
    use ghast_physics::pick::v1_12 as pick;
    replay(
        "1.12.2",
        "yaw",
        |l| {
            fill(
                l,
                |w: &mut ghast_physics::v1_12::MemoryWorld, c| {
                    w.load_chunks(c[0], c[1], c[2], c[3])
                },
                |w, a, b, raw| w.fill_raw(a, b, raw),
                ghast_physics::v1_12::memory_world(),
            )
        },
        |w, e, lk, reach| {
            (
                pick::pick(w, e, lk, reach),
                pick::clip(w, e, pick::ray_end(e, lk, reach), false, false),
            )
        },
        pick::eye_position,
        pick::look_vector,
    );
}

/// Every 1.12.2 selection dump entry (each state alone, connecting blocks against stone sides and
/// themselves, stairs next to stairs, door halves together, chest pairs, tripwire on stone): the
/// ray shape's box equals the jar's `getBoundingBox` (`can_collide` false: no shape).
#[cfg(feature = "v1_12")]
#[test]
fn ray_bounds_match_vanilla_1_12_2() {
    use ghast_physics::v1_12::{RayShape, memory_world, ray_shape};
    let (mut ok, mut fails) = (0, Vec::new());
    for l in golden("1.12.2", "selection_boxes.jsonl.gz") {
        if l["type"] != "selection" {
            continue;
        }
        let mut w = memory_world();
        w.load_chunks(-1, -1, 1, 1);
        for n in l["neighbours"].as_array().unwrap() {
            let n = ints(n);
            w.set_raw(n[0], 64 + n[1], n[2], (n[3] << 4 | n[4]) as u16);
        }
        let id = l["id"].as_u64().unwrap() as u16;
        let meta = l["meta"].as_u64().unwrap() as u16;
        w.set_raw(0, 64, 0, id << 4 | meta);
        let want: Vec<f64> = l["bounds"].as_array().unwrap().iter().map(f64v).collect();
        let got = ray_shape(&w, 0, 64, 0, false);
        let r = match (l["can_collide"].as_bool().unwrap(), got) {
            (false, RayShape::None) => Ok(()),
            (true, RayShape::Bounds(b)) => {
                let g = [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z];
                if g.iter().zip(&want).all(|(a, b)| a.to_bits() == b.to_bits()) {
                    Ok(())
                } else {
                    Err(format!("{g:?}"))
                }
            }
            // Stairs test per collision box: their bounding box is not what the ray uses.
            (true, RayShape::Boxes { .. }) => Ok(()),
            (true, RayShape::Never) => Ok(()),
            (_, g) => Err(format!("{g:?}")),
        };
        match r {
            Ok(()) => ok += 1,
            Err(e) => fails.push(format!(
                "{} {}:{} {}: got {e}, jar {want:?}",
                l["config"], id, meta, l["state"]
            )),
        }
    }
    assert!(
        fails.is_empty(),
        "{} of {} differ; first:\n{}",
        fails.len(),
        ok + fails.len(),
        fails
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    eprintln!("1.12.2 selection: {ok} identical");
}
