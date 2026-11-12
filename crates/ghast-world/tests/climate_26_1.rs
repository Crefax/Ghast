//! Biome precipitation and temperature against the real 26.1 classes
//! (`tests/golden/26.1/biome_climate.jsonl.gz`, oracle `biome-climate`).
#![cfg(feature = "v26_1")]

use std::io::Read;

use ghast_world::climate::{Climate, Precipitation, zoom_cell};

#[test]
fn climate_matches_the_real_biomes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/golden/26.1/biome_climate.jsonl.gz");
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    let (mut biomes, mut samples, mut zooms) = (0, 0, 0);
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if v["type"] == "zoom" {
            let seed: i64 = v["seed"].as_str().unwrap().parse().unwrap();
            for s in v["samples"].as_array().unwrap() {
                let i = |k: usize| s[k].as_i64().unwrap() as i32;
                assert_eq!(
                    zoom_cell(seed, i(0), i(1), i(2)),
                    [i(3), i(4), i(5)],
                    "seed {seed} at {} {} {}",
                    i(0),
                    i(1),
                    i(2)
                );
                zooms += 1;
            }
            continue;
        }
        let name = v["name"].as_str().unwrap();
        let c = Climate::vanilla(name).unwrap_or_else(|| panic!("{name}"));
        let bits = u32::from_str_radix(v["temperature"].as_str().unwrap(), 16).unwrap();
        assert_eq!(c.temperature.to_bits(), bits, "{name}");
        assert_eq!(c.has_precipitation, v["has_precipitation"].as_bool().unwrap(), "{name}");
        assert_eq!(c.frozen, v["modifier"] == "frozen", "{name}");
        for s in v["samples"].as_array().unwrap() {
            let i = |k: usize| s[k].as_i64().unwrap() as i32;
            let (x, y, z, sea) = (i(0), i(1), i(2), i(3));
            let t = u32::from_str_radix(s[4].as_str().unwrap(), 16).unwrap();
            assert_eq!(
                c.temperature_at(x, y, z, sea).to_bits(),
                t,
                "{name} at {x} {y} {z} (sea {sea}): {} vs {}",
                c.temperature_at(x, y, z, sea),
                f32::from_bits(t)
            );
            let p = match s[5].as_str().unwrap() {
                "rain" => Precipitation::Rain,
                "snow" => Precipitation::Snow,
                _ => Precipitation::None,
            };
            assert_eq!(c.precipitation_at(x, y, z, sea), p, "{name} at {x} {y} {z}");
            samples += 1;
        }
        biomes += 1;
    }
    eprintln!(
        "biome climate: {biomes} biomes, {samples} positions identical; {zooms} zoomed cells identical"
    );
    assert!(biomes >= 60);
    assert!(zooms >= 3000);
}
