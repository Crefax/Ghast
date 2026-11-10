//! The crosshair block pick against the vanilla oracles: `tests/golden/<version>/raytrace.jsonl.gz`
//! (random worlds of random states, random viewpoints, the real client's pick) and
//! `selection_shapes.jsonl.gz` (per-state outline / interaction shapes, model offsets), for 1.16.5
//! and 26.1. Every field is compared bit for bit.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};

use ghast_physics::pick::{BlockHit, HitKind};
use serde_json::Value;

fn golden(version: &str, name: &str) -> impl Iterator<Item = Value> {
    lines(&format!(
        "{}/../../tests/golden/{version}/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
}

fn lines(path: &str) -> impl Iterator<Item = Value> + use<> {
    let path = path.to_owned();
    let f = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    BufReader::new(flate2::read::GzDecoder::new(f))
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
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
fn i3(v: &Value) -> [i32; 3] {
    [0, 1, 2].map(|i| v[i].as_i64().unwrap() as i32)
}
fn bits(a: [f64; 3]) -> [u64; 3] {
    a.map(f64::to_bits)
}

/// Compares a pick with an oracle hit object; `Err` names the first differing field.
fn same_hit(got: &BlockHit, want: &Value) -> Result<(), String> {
    let kind = match want["type"].as_str().unwrap() {
        "block" => HitKind::Block,
        "miss" => HitKind::Miss,
        t => return Err(format!("oracle hit type {t}")),
    };
    if got.kind != kind {
        return Err(format!("type {:?}, oracle {kind:?}", got.kind));
    }
    if bits(got.location) != bits(v3(&want["location"])) {
        return Err(format!(
            "location {:?}, oracle {:?}",
            got.location,
            v3(&want["location"])
        ));
    }
    if got.pos != i3(&want["block"]) {
        return Err(format!(
            "block {:?}, oracle {:?}",
            got.pos,
            i3(&want["block"])
        ));
    }
    if got.face.name() != want["face"].as_str().unwrap() {
        return Err(format!("face {}, oracle {}", got.face.name(), want["face"]));
    }
    if got.inside != want["inside"].as_bool().unwrap() {
        return Err(format!("inside {}, oracle {}", got.inside, want["inside"]));
    }
    Ok(())
}

/// Shape content as a comparable value (points, cells, boxes as bits).
fn shape_key(s: &ghast_data::SelectionShape) -> String {
    let b: Vec<[u64; 6]> = s
        .boxes
        .iter()
        .map(|b| [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z].map(f64::to_bits))
        .collect();
    let p = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    format!(
        "{} {:?} {:?} {:?} {:?} {:?} {} {:?}",
        s.cube,
        s.sizes,
        p(s.xs),
        p(s.ys),
        p(s.zs),
        s.cells,
        s.empty,
        b
    )
}

fn dumped_shapes(version: &str) -> (Vec<String>, Vec<Value>) {
    let mut shapes = Vec::new();
    let mut states = Vec::new();
    for l in golden(version, "selection_shapes.jsonl.gz") {
        match l["type"].as_str().unwrap() {
            "voxel_shape" => {
                let filled = l["filled"].as_str().unwrap();
                let mut words = vec![0u64; filled.len().div_ceil(64)];
                for (i, c) in filled.bytes().enumerate() {
                    if c == b'1' {
                        words[i / 64] |= 1 << (i % 64);
                    }
                }
                let pts = |k: &str| {
                    l[k].as_array()
                        .unwrap()
                        .iter()
                        .map(|x| f64v(x).to_bits())
                        .collect::<Vec<_>>()
                };
                let boxes: Vec<[u64; 6]> = l["boxes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| [0, 1, 2, 3, 4, 5].map(|i| f64v(&b[i]).to_bits()))
                    .collect();
                let sizes: Vec<u8> = l["sizes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_u64().unwrap() as u8)
                    .collect();
                shapes.push(format!(
                    "{} {:?} {:?} {:?} {:?} {:?} {} {:?}",
                    l["class"] == "simple",
                    [sizes[0], sizes[1], sizes[2]],
                    pts("points_x"),
                    pts("points_y"),
                    pts("points_z"),
                    words,
                    l["empty"].as_bool().unwrap(),
                    boxes
                ));
            }
            "state" => states.push(l),
            t => panic!("line type {t}"),
        }
    }
    (shapes, states)
}

#[test]
fn selection_shapes_match_vanilla_1_16_5() {
    use ghast_data::v1_16_5::BlockState;
    use ghast_physics::pick::v1_16_5::model_offset;
    let (shapes, states) = dumped_shapes("1.16.5");
    assert_eq!(states.len(), ghast_data::v1_16_5::STATE_COUNT);
    for l in &states {
        let s = BlockState::from_id(l["id"].as_u64().unwrap() as u16).unwrap();
        assert_eq!(s.to_string(), l["state"].as_str().unwrap());
        let sid = |k: &str| l[k].as_u64().map(|x| x as usize);
        let outline = sid("outline_base").or(sid("outline")).unwrap();
        assert_eq!(shape_key(s.outline_shape()), shapes[outline], "{s}");
        assert_eq!(s.outline_offset(), l.get("outline_base").is_some(), "{s}");
        let holding = sid("outline_holding_own_item").unwrap_or(outline);
        assert_eq!(
            shape_key(s.outline_shape_holding_own_item()),
            shapes[holding],
            "{s}"
        );
        assert_eq!(
            shape_key(s.raycast_shape()),
            shapes[sid("interaction").unwrap()],
            "{s}"
        );
        if s.outline_offset() {
            let off = model_offset(s.block().vanilla().offset, 0, 0);
            assert_eq!(bits(off), bits(v3(&l["offset_at_dump"])), "{s}");
        }
    }
}

#[cfg(feature = "v26_1")]
#[test]
fn selection_shapes_match_vanilla_26_1() {
    use ghast_data::v26_1::BlockState;
    let (shapes, states) = dumped_shapes("26.1");
    assert_eq!(states.len(), ghast_data::v26_1::STATE_COUNT);
    for l in &states {
        let s = BlockState::from_id(l["id"].as_u64().unwrap() as u16).unwrap();
        assert_eq!(s.to_string(), l["state"].as_str().unwrap());
        let sid = |k: &str| l[k].as_u64().map(|x| x as usize);
        let outline = sid("outline_base").or(sid("outline")).unwrap();
        assert_eq!(shape_key(s.outline_shape()), shapes[outline], "{s}");
        assert_eq!(s.outline_offset(), l.get("outline_base").is_some(), "{s}");
        let holding = sid("outline_holding_own_item").unwrap_or(outline);
        assert_eq!(
            shape_key(s.outline_shape_holding_own_item()),
            shapes[holding],
            "{s}"
        );
        assert_eq!(
            shape_key(s.interaction_shape()),
            shapes[sid("interaction").unwrap()],
            "{s}"
        );
        if s.outline_offset() {
            let (o, _) = ghast_physics::pick::v26_1::shapes(s, [0, 64, 0], None);
            assert_eq!(
                bits(o.shift.unwrap()),
                bits(v3(&l["offset_at_dump"])),
                "{s}"
            );
        }
    }
}

/// Raw state ids of a ray world line, checked against the dumped state names.
fn world_blocks(l: &Value, name_of: impl Fn(u16) -> String) -> Vec<([i32; 3], u16)> {
    let names: BTreeMap<String, String> = l["states"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
        .collect();
    for (id, name) in &names {
        assert_eq!(&name_of(id.parse().unwrap()), name, "state id {id}");
    }
    l["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| (i3(b), b[3].as_u64().unwrap() as u16))
        .collect()
}

fn report(version: &str, ok: usize, fails: &[String]) {
    assert!(ok > 0, "{version}: no rays");
    if !fails.is_empty() {
        panic!(
            "{version}: {} of {} rays differ; first:\n{}",
            fails.len(),
            ok + fails.len(),
            fails
                .iter()
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    eprintln!("{version}: {ok} rays identical");
}

#[test]
fn crosshair_matches_vanilla_1_16_5() {
    let (ok, fails) = replay_1_16_5(golden("1.16.5", "raytrace.jsonl.gz"));
    report("1.16.5", ok, &fails);
}

fn replay_1_16_5(input: impl Iterator<Item = Value>) -> (usize, Vec<String>) {
    use ghast_data::v1_16_5::{Block, BlockState};
    use ghast_physics::pick::v1_16_5 as pick;
    let mut world = ghast_physics::v1_16_5::memory_world();
    let (mut ok, mut fails) = (0, Vec::new());
    let mut w = 0;
    for l in input {
        if l["type"] == "ray_world" {
            world = ghast_physics::v1_16_5::memory_world();
            world.load_chunks(-2, -2, 2, 2);
            for (p, id) in world_blocks(&l, |id| BlockState::from_id(id).unwrap().to_string()) {
                world.set_raw(p[0], p[1], p[2], id);
            }
            w = l["world"].as_u64().unwrap();
            continue;
        }
        let k = l["k"].as_u64().unwrap();
        let feet = v3(&l["feet"]);
        let eye = pick::eye_position(feet, f32v(&l["eye_height"]));
        let look = pick::rotation_vector(f32v(&l["yaw"]), f32v(&l["pitch"]));
        let held = l["held"]
            .as_str()
            .and_then(|n| Block::from_name(n.trim_start_matches("minecraft:")));
        let r = if bits(eye) != bits(v3(&l["eyes"])) {
            Err(format!("eyes {eye:?}, oracle {:?}", v3(&l["eyes"])))
        } else if bits(look) != bits(v3(&l["view"])) {
            Err(format!("look {look:?}, oracle {:?}", v3(&l["view"])))
        } else {
            let hit = pick::pick(&world, eye, look, f64v(&l["reach"]), held);
            same_hit(&hit, &l["pick"])
        };
        match r {
            Ok(()) => ok += 1,
            Err(e) => fails.push(format!("world {w} ray {k}: {e}")),
        }
    }
    (ok, fails)
}

#[cfg(feature = "v26_1")]
#[test]
fn crosshair_matches_vanilla_26_1() {
    let (ok, fails) = replay_26_1(golden("26.1", "raytrace.jsonl.gz"));
    report("26.1", ok, &fails);
}

#[cfg(feature = "v26_1")]
fn replay_26_1(input: impl Iterator<Item = Value>) -> (usize, Vec<String>) {
    use ghast_data::v26_1::{Block, BlockState};
    use ghast_physics::pick::v26_1 as pick;
    let mut world = ghast_physics::modern::memory_world_26_1();
    let (mut ok, mut fails) = (0, Vec::new());
    let mut w = 0;
    for l in input {
        if l["type"] == "ray_world" {
            world = ghast_physics::modern::memory_world_26_1();
            world.load_chunks(-2, -2, 2, 2);
            for (p, id) in world_blocks(&l, |id| BlockState::from_id(id).unwrap().to_string()) {
                world.set_raw(p[0], p[1], p[2], id);
            }
            w = l["world"].as_u64().unwrap();
            continue;
        }
        let k = l["k"].as_u64().unwrap();
        let eye = pick::eye_position(v3(&l["feet"]), v3(&l["old"]), f32v(&l["eye_height"]));
        let view = pick::view_vector(f32v(&l["yaw"]), f32v(&l["pitch"]));
        let held = l["held"]
            .as_str()
            .and_then(|n| Block::from_name(n.trim_start_matches("minecraft:")));
        let (br, er) = (f64v(&l["block_range"]), f64v(&l["entity_range"]));
        let r = if bits(eye) != bits(v3(&l["eyes"])) {
            Err(format!("eyes {eye:?}, oracle {:?}", v3(&l["eyes"])))
        } else if bits(view) != bits(v3(&l["view"])) {
            Err(format!("view {view:?}, oracle {:?}", v3(&l["view"])))
        } else {
            let max = br.max(er);
            let to = [
                eye[0] + view[0] * max,
                eye[1] + view[1] * max,
                eye[2] + view[2] * max,
            ];
            let clip = pick::clip(&world, eye, to, held);
            same_hit(&clip, &l["clip"])
                .map_err(|e| format!("clip: {e}"))
                .and_then(|()| same_hit(&pick::pick(&world, eye, view, br, er, held), &l["pick"]))
        };
        match r {
            Ok(()) => ok += 1,
            Err(e) => fails.push(format!("world {w} ray {k}: {e}")),
        }
    }
    (ok, fails)
}

/// Large ray sets from `oracle/run.sh rays --seed N --worlds N --rays N --out
/// oracle/cache/<version>/rays/<name>.jsonl.gz` (feature `oracle-cache`; not committed).
#[cfg(feature = "oracle-cache")]
#[test]
fn crosshair_matches_vanilla_cached_rays() {
    let root = format!("{}/../../oracle/cache", env!("CARGO_MANIFEST_DIR"));
    let mut total = 0;
    for version in ["1.16.5", "26.1"] {
        let Ok(dir) = std::fs::read_dir(format!("{root}/{version}/rays")) else {
            continue;
        };
        let mut files: Vec<_> = dir.map(|e| e.unwrap().path()).collect();
        files.sort();
        for f in files {
            let path = f.to_string_lossy().into_owned();
            let (ok, fails) = match version {
                "1.16.5" => replay_1_16_5(lines(&path)),
                #[cfg(feature = "v26_1")]
                _ => replay_26_1(lines(&path)),
                #[cfg(not(feature = "v26_1"))]
                _ => continue,
            };
            report(&path, ok, &fails);
            total += ok;
        }
    }
    eprintln!("cached rays: {total} identical");
}
