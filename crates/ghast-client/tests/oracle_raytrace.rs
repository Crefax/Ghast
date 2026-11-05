//! The crosshair against the vanilla 1.8.9 oracle: look vector and `Entity.rayTrace` pick
//! (`tests/golden/1.8.9/raytrace.jsonl.gz`), with the ray bounds of
//! `tests/golden/1.8.9/selection_boxes.jsonl.gz` ("alone" configurations).
//!
//! The world of each case is a ghast-physics `MemoryWorld`; ray shapes come from
//! `ghast_physics::v1_8::ray_shape` (neighbour-dependent bounds, the stairs passes).

use std::collections::HashMap;
use std::io::{BufRead, BufReader};

use ghast_client::common::{Face, Vec3};
use ghast_client::hands::{self, BlockQuery, RayResult};
use serde_json::Value;

fn golden(name: &str) -> impl Iterator<Item = Value> {
    let path = format!(
        "{}/../../tests/golden/1.8.9/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
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
fn vec3(v: &Value) -> Vec3 {
    Vec3::new(f64v(&v[0]), f64v(&v[1]), f64v(&v[2]))
}

/// Ray bounds of a state (`None`: rays pass).
type Ray = Option<[f64; 6]>;

#[test]
fn crosshair_matches_vanilla() {
    let mut sel: HashMap<(u16, u8), (Ray, bool)> = HashMap::new();
    for l in golden("selection_boxes.jsonl.gz") {
        if l["type"] != "selection" || l["config"] != "alone" {
            continue;
        }
        let id = l["id"].as_u64().unwrap() as u16;
        let meta = l["meta"].as_u64().unwrap() as u8;
        let b: Vec<f64> = l["bounds"].as_array().unwrap().iter().map(f64v).collect();
        let collide = l["can_collide"].as_bool().unwrap();
        sel.insert(
            (id, meta),
            (
                collide.then(|| [b[0], b[1], b[2], b[3], b[4], b[5]]),
                collide,
            ),
        );
    }
    let mut world = ghast_physics::v1_8::memory_world();
    let (mut ok, mut total) = (0, 0);
    let mut fails = Vec::new();
    for l in golden("raytrace.jsonl.gz") {
        if l["type"] == "scenario" {
            world = ghast_physics::v1_8::memory_world();
            for c in l["loaded_chunks"].as_array().unwrap() {
                let c: Vec<i32> = c
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_i64().unwrap() as i32)
                    .collect();
                world.load_chunks(c[0], c[1], c[2], c[3]);
            }
            for f in l["fills"].as_array().unwrap() {
                let v: Vec<i32> = f
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_i64().unwrap() as i32)
                    .collect();
                world.fill_raw(
                    [v[0], v[1], v[2]],
                    [v[3], v[4], v[5]],
                    (v[6] << 4 | v[7]) as u16,
                );
            }
            continue;
        }
        if l["type"] != "ray" {
            continue;
        }
        total += 1;
        let look = hands::look_vector(f32v(&l["head_yaw"]), f32v(&l["pitch"]));
        assert_eq!(look, vec3(&l["look"]), "look vector of ray {}", l["k"]);
        let eye = f64::from(f32v(&l["eye_height"]));
        let feet = vec3(&l["feet"]);
        let eyes = Vec3::new(feet.x, feet.y + eye, feet.z);
        assert_eq!(eyes, vec3(&l["eyes"]), "eyes of ray {}", l["k"]);
        let reach = f64v(&l["reach"]);
        let w = &world;
        let blocks = |x: i32, y: i32, z: i32| {
            use ghast_physics::BlockView;
            let state = w.block(x, y, z);
            // The static table (ghast-data) must equal the dump for every non-dependent state.
            let s = state.selection();
            if s.dependency == ghast_data::ShapeDependency::None {
                let b = s.bounds;
                let mine = s
                    .can_collide
                    .then_some([b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]);
                let (dumped, _) = sel
                    .get(&(u16::from(state.block().id()), state.meta()))
                    .copied()
                    .unwrap_or((None, false));
                assert_eq!(mine, dumped, "selection of {state:?}");
            }
            let arr = |b: &ghast_physics::math::Aabb| {
                [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]
            };
            let (selection, stairs) = match ghast_physics::v1_8::ray_shape(w, x, y, z, false) {
                ghast_physics::v1_8::RayShape::None => (None, None),
                ghast_physics::v1_8::RayShape::Bounds(b) => (Some(arr(&b)), None),
                ghast_physics::v1_8::RayShape::Stairs(p) => {
                    (None, Some(p.boxes().map(|b| b.as_ref().map(arr))))
                }
            };
            Some(
                BlockQuery::new(u16::from(state.block().id()))
                    .with_air(state.is_air())
                    .with_hardness(1.0)
                    .with_selection(selection)
                    .with_stairs(stairs),
            )
        };
        let end = Vec3::new(
            eyes.x + look.x * reach,
            eyes.y + look.y * reach,
            eyes.z + look.z * reach,
        );
        let got = hands::ray_trace(&blocks, eyes, end, true);
        let want = &l["pick"];
        let side = |f: Face| match f {
            Face::Down => "down",
            Face::Up => "up",
            Face::North => "north",
            Face::South => "south",
            Face::West => "west",
            Face::East => "east",
        };
        let block_of = |v: &Value| -> (i64, i64, i64) {
            let b: Vec<i64> = v
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap())
                .collect();
            (b[0], b[1], b[2])
        };
        let same = match (want["type"].as_str().unwrap(), got) {
            ("null", RayResult::None) => true,
            ("block", RayResult::Block(h)) => {
                (i64::from(h.pos.x), i64::from(h.pos.y), i64::from(h.pos.z))
                    == block_of(&want["block"])
                    && side(h.face) == want["side"]
                    && h.hit == vec3(&want["hit"])
            }
            ("miss", RayResult::Miss { pos, face, hit }) => {
                (i64::from(pos.x), i64::from(pos.y), i64::from(pos.z)) == block_of(&want["block"])
                    && side(face) == want["side"]
                    && hit == vec3(&want["hit"])
            }
            _ => false,
        };
        if same {
            ok += 1;
        } else {
            fails.push(format!("ray {}: want {want} got {got:?}", l["k"]));
        }
    }
    println!("rays: {total}, matched {ok}, failed {}", fails.len());
    assert!(
        fails.is_empty(),
        "{} mismatches, first: {:#?}",
        fails.len(),
        &fails[..fails.len().min(5)]
    );
    assert_eq!(ok, 2000);
}
