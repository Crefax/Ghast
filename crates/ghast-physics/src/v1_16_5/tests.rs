//! 1.16.5 block facts and voxel grids (as the rules see them) vs the oracle's
//! `voxel_shapes.jsonl.gz`, for every state.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

use ghast_data::v1_16_5::BlockState;
use serde_json::Value;

use super::blocks::{
    Shape, blocks_movement, collision_shape, exceeds_cube, is_ice_material, should_suffocate,
    side_solid,
};
use crate::voxel::Points;

fn f64_of(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
}

#[test]
fn facts_vs_voxel_dump() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/1.16.5/voxel_shapes.jsonl.gz");
    let Ok(f) = std::fs::File::open(&path) else {
        eprintln!("skipped: no voxel dump");
        return;
    };
    let mut text = String::new();
    flate2::read::GzDecoder::new(f)
        .read_to_string(&mut text)
        .unwrap();
    let mut shapes: HashMap<u64, Value> = HashMap::new();
    let mut diffs: HashMap<&str, usize> = HashMap::new();
    let mut n = 0;
    for line in text.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        match v["type"].as_str() {
            Some("voxel_shape") => {
                shapes.insert(v["sid"].as_u64().unwrap(), v);
            }
            Some("state") => {
                n += 1;
                let s = BlockState::from_id(v["id"].as_u64().unwrap() as u16).unwrap();
                let mut bump = |k: &'static str, ok: bool| {
                    if !ok {
                        *diffs.entry(k).or_default() += 1;
                    }
                };
                bump(
                    "should_suffocate",
                    should_suffocate(s) == v["should_suffocate"].as_bool().unwrap(),
                );
                bump(
                    "blocks_movement",
                    blocks_movement(s) == v["material_blocks_movement"].as_bool().unwrap(),
                );
                bump(
                    "exceeds_cube",
                    exceeds_cube(s) == v["exceeds_cube"].as_bool().unwrap(),
                );
                bump("ice", is_ice_material(s) == (v["material"] == "ice"));
                let sides = v["side_solid_full_square"].as_array().unwrap();
                let dirs = [(0, -1), (1, 0), (0, 1), (-1, 0)];
                bump(
                    "side_solid",
                    sides
                        .iter()
                        .zip(dirs)
                        .all(|(x, (dx, dz))| x.as_bool().unwrap() == side_solid(s, dx, dz)),
                );
                let sh = &shapes[&v["collision_shape"].as_u64().unwrap()];
                let grid_ok = match collision_shape(s, 0, 64, 0, None) {
                    Shape::Empty => sh["empty"].as_bool().unwrap(),
                    Shape::Grid { shape, .. } => {
                        let pts = |a: usize, key: &str| {
                            let want: Vec<f64> =
                                sh[key].as_array().unwrap().iter().map(f64_of).collect();
                            let n = shape.axes[a].cells();
                            let have: Vec<f64> = (0..=n).map(|i| shape.axes[a].get(i)).collect();
                            want == have
                        };
                        let cells_ok = {
                            let filled = sh["filled"].as_str().unwrap().as_bytes();
                            let n = [0, 1, 2].map(|a| shape.axes[a].cells());
                            let mut ok = true;
                            for x in 0..n[0] {
                                for y in 0..n[1] {
                                    for z in 0..n[2] {
                                        let i = (x * n[1] + y) * n[2] + z;
                                        ok &= (filled[i] == b'1')
                                            == shape.filled(x as isize, y as isize, z as isize);
                                    }
                                }
                            }
                            ok
                        };
                        pts(0, "points_x") && pts(1, "points_y") && pts(2, "points_z") && cells_ok
                    }
                };
                bump("grid_points", grid_ok);
                if let Shape::Grid { shape, .. } = collision_shape(s, 0, 64, 0, None) {
                    let simple = shape
                        .axes
                        .iter()
                        .all(|p| matches!(p, Points::Fractional(_)));
                    bump("grid_class", simple == (sh["class"] == "simple"));
                }
            }
            _ => {}
        }
    }
    eprintln!("{n} states checked; differences: {diffs:?}");
    assert!(n > 17000);
    assert!(
        diffs.is_empty(),
        "block facts differ from the oracle: {diffs:?}"
    );
}
