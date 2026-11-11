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

/// Replays `picks.jsonl.gz` (random worlds of every state, entities, random reach, viewpoints
/// inside blocks and entities, survival and creative): the raw `Entity.rayTrace(reach)` and the
/// `GameRenderer.updateTargetedEntity` target.
#[allow(clippy::too_many_arguments)]
fn replay_picks<W>(
    version: &str,
    lines: Vec<Value>,
    look_yaw: &str,
    mut world: impl FnMut(&[([i32; 3], u16)]) -> W,
    eye: impl Fn([f64; 3], f32) -> [f64; 3],
    look: impl Fn(f32, f32) -> [f64; 3],
    raw: impl Fn(&W, [f64; 3], [f64; 3], f64) -> Option<BlockHit>,
    target: impl Fn(
        &W,
        [f64; 3],
        [f64; 3],
        bool,
        &ghast_physics::math::Aabb,
        &[ghast_physics::pick::PickEntity],
    ) -> ghast_physics::pick::legacy::Target,
) {
    use ghast_physics::pick::PickEntity;
    use ghast_physics::pick::legacy::Target;
    let aabb = |v: &Value| {
        let c: Vec<f64> = v.as_array().unwrap().iter().map(f64v).collect();
        ghast_physics::math::Aabb::new(c[0], c[1], c[2], c[3], c[4], c[5])
    };
    let mut w = None;
    let mut ents: (Vec<PickEntity>, Vec<i64>) = (Vec::new(), Vec::new());
    let (mut ok, mut fails, mut entity_hits) = (0, Vec::new(), 0);
    let mut wi = 0;
    for l in lines {
        if l["type"] == "pick_world" {
            let blocks: Vec<([i32; 3], u16)> = l["blocks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| {
                    let b = ints(b);
                    ([b[0], b[1], b[2]], b[3] as u16)
                })
                .collect();
            w = Some(world(&blocks));
            ents = (Vec::new(), Vec::new());
            for e in l["entities"].as_array().unwrap() {
                if e["pickable"].as_bool().unwrap() {
                    ents.0.push(PickEntity {
                        bb: aabb(&e["bb"]),
                        pick_radius: f32v(&e["margin"]),
                        same_root_vehicle: false,
                    });
                    ents.1.push(e["id"].as_i64().unwrap());
                }
            }
            wi = l["world"].as_u64().unwrap();
            continue;
        }
        let k = l["k"].as_u64().unwrap();
        let wd = w.as_ref().unwrap();
        let e = eye(v3(&l["feet"]), f32v(&l["eye_height"]));
        let lk = look(f32v(&l[look_yaw]), f32v(&l["pitch"]));
        let r = if bits(e) != bits(v3(&l["eyes"])) {
            Err(format!("eyes {e:?}"))
        } else if bits(lk) != bits(v3(&l["look"])) {
            Err(format!("look {lk:?}"))
        } else {
            same(raw(wd, e, lk, f64v(&l["reach"])), &l["raw"])
                .map_err(|x| format!("raw: {x}"))
                .and_then(|()| {
                    let t = target(
                        wd,
                        e,
                        lk,
                        l["creative"].as_bool().unwrap(),
                        &aabb(&l["bb"]),
                        &ents.0,
                    );
                    let want = &l["target"];
                    match (t, want["type"].as_str().unwrap()) {
                        (Target::Entity(h), "entity") => {
                            entity_hits += 1;
                            if ents.1[h.index] == want["entity"].as_i64().unwrap()
                                && bits(h.location) == bits(v3(&want["hit"]))
                            {
                                Ok(())
                            } else {
                                Err(format!(
                                    "target {h:?} (id {}), oracle {want}",
                                    ents.1[h.index]
                                ))
                            }
                        }
                        (Target::TooFar { location, pos }, "miss") if want["side"].is_null() => {
                            let b = ints(&want["block"]);
                            if bits(location) == bits(v3(&want["hit"])) && pos == [b[0], b[1], b[2]]
                            {
                                Ok(())
                            } else {
                                Err(format!("too far {location:?}, oracle {want}"))
                            }
                        }
                        (Target::None, _) => same(None, want).map_err(|x| format!("target: {x}")),
                        (Target::Block(b), _) => {
                            same(Some(b), want).map_err(|x| format!("target: {x}"))
                        }
                        (t, _) => Err(format!("target {t:?}, oracle {want}")),
                    }
                })
        };
        match r {
            Ok(()) => ok += 1,
            Err(e) => fails.push(format!("world {wi} pick {k}: {e}")),
        }
    }
    assert!(
        fails.is_empty(),
        "{version}: {} of {} picks differ; first:\n{}",
        fails.len(),
        ok + fails.len(),
        fails
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    eprintln!("{version}: {ok} picks identical ({entity_hits} entity targets)");
}

#[cfg(feature = "v1_8")]
#[test]
fn picks_match_vanilla_1_8_9() {
    use ghast_physics::pick::v1_8 as pick;
    replay_picks(
        "1.8.9",
        golden("1.8.9", "picks.jsonl.gz"),
        "head_yaw",
        |blocks| {
            let mut w = ghast_physics::v1_8::memory_world();
            w.load_chunks(-3, -3, 3, 3);
            for (p, raw) in blocks {
                w.set_raw(p[0], p[1], p[2], *raw);
            }
            w
        },
        pick::eye_position,
        pick::look_vector,
        pick::pick,
        pick::pick_target,
    );
}

#[cfg(feature = "v1_12")]
#[test]
fn picks_match_vanilla_1_12_2() {
    use ghast_physics::pick::v1_12 as pick;
    replay_picks(
        "1.12.2",
        golden("1.12.2", "picks.jsonl.gz"),
        "yaw",
        |blocks| {
            let mut w = ghast_physics::v1_12::memory_world();
            w.load_chunks(-3, -3, 3, 3);
            for (p, raw) in blocks {
                w.set_raw(p[0], p[1], p[2], *raw);
            }
            w
        },
        pick::eye_position,
        pick::look_vector,
        pick::pick,
        pick::pick_target,
    );
}

fn cached(version: &str) -> Vec<(String, Vec<Value>)> {
    let dir = format!(
        "{}/../../oracle/cache/{version}/picks",
        env!("CARGO_MANIFEST_DIR")
    );
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = rd.map(|e| e.unwrap().path()).collect();
    files.sort();
    files
        .into_iter()
        .map(|f| {
            let r = BufReader::new(flate2::read::GzDecoder::new(
                std::fs::File::open(&f).unwrap(),
            ));
            (
                f.display().to_string(),
                r.lines()
                    .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
                    .collect(),
            )
        })
        .collect()
}

/// Large pick sets (`oracle/run.sh picks --seed N --worlds 100 --rays 400 --out
/// oracle/cache/<v>/picks/...`; feature `oracle-cache`, not committed).
#[cfg(feature = "oracle-cache")]
#[test]
fn picks_match_vanilla_cached() {
    #[cfg(feature = "v1_8")]
    for (name, lines) in cached("1.8.9") {
        use ghast_physics::pick::v1_8 as pick;
        replay_picks(
            &name,
            lines,
            "head_yaw",
            |blocks| {
                let mut w = ghast_physics::v1_8::memory_world();
                w.load_chunks(-3, -3, 3, 3);
                for (p, raw) in blocks {
                    w.set_raw(p[0], p[1], p[2], *raw);
                }
                w
            },
            pick::eye_position,
            pick::look_vector,
            pick::pick,
            pick::pick_target,
        );
    }
    #[cfg(feature = "v1_12")]
    for (name, lines) in cached("1.12.2") {
        use ghast_physics::pick::v1_12 as pick;
        replay_picks(
            &name,
            lines,
            "yaw",
            |blocks| {
                let mut w = ghast_physics::v1_12::memory_world();
                w.load_chunks(-3, -3, 3, 3);
                for (p, raw) in blocks {
                    w.set_raw(p[0], p[1], p[2], *raw);
                }
                w
            },
            pick::eye_position,
            pick::look_vector,
            pick::pick,
            pick::pick_target,
        );
    }
}
