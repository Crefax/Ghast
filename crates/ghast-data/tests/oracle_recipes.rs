//! 1.8.9 crafting recipes vs the oracle dump (`tests/golden/1.8.9/recipes.jsonl.gz`): every entry,
//! in order, field by field. Skipped with a notice if the file is missing.
#![cfg(feature = "v1_8")]

use ghast_data::v1_8::{Item, RECIPE_ANY_DATA, Recipe, SpecialRecipe, recipes};
use serde_json::Value;
use std::io::Read as _;
use std::path::PathBuf;

fn ingredient(v: &Value) -> Option<(u16, u16)> {
    v.as_array()
        .map(|a| (a[0].as_u64().unwrap() as u16, a[1].as_u64().unwrap() as u16))
}

fn output(hex: &str) -> (u16, u8, u16) {
    let b: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    assert_eq!((b.len(), b[5]), (6, 0), "output {hex}");
    (
        u16::from_be_bytes([b[0], b[1]]),
        b[2],
        u16::from_be_bytes([b[3], b[4]]),
    )
}

#[test]
fn v1_8_9_recipes_match_oracle() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/1.8.9/recipes.jsonl.gz");
    let Ok(file) = std::fs::File::open(&path) else {
        eprintln!("skipped: {} not found", path.display());
        return;
    };
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .unwrap();
    let lines: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), recipes().len());
    for (i, (v, r)) in lines.iter().zip(recipes()).enumerate() {
        assert_eq!(v["i"].as_u64(), Some(i as u64));
        let want = match v["type"].as_str().unwrap() {
            "shaped" => {
                let ins: Vec<Option<(u16, u16)>> =
                    v["in"].as_array().unwrap().iter().map(ingredient).collect();
                (
                    v["w"].as_u64().unwrap() as u8,
                    v["h"].as_u64().unwrap() as u8,
                    ins,
                    output(v["out"].as_str().unwrap()),
                )
            }
            "shapeless" => {
                let ins: Vec<Option<(u16, u16)>> =
                    v["in"].as_array().unwrap().iter().map(ingredient).collect();
                (0, 0, ins, output(v["out"].as_str().unwrap()))
            }
            _ => {
                let kind = match v["class"].as_str().unwrap() {
                    "ArmorDyeRecipeType" => SpecialRecipe::ArmorDye,
                    "FireworkRecipeType" => SpecialRecipe::Firework,
                    "CopyingRecipeType" => SpecialRecipe::BannerPattern,
                    "BookCloningRecipeType" => SpecialRecipe::BookCloning,
                    "MapCloningRecipeType" => SpecialRecipe::MapCloning,
                    "MapUpscaleRecipeType" => SpecialRecipe::MapUpscale,
                    "RepairingRecipeType" => SpecialRecipe::Repairing,
                    "PatternRecipeType" => SpecialRecipe::BannerDuplicate,
                    c => panic!("unknown special {c}"),
                };
                assert_eq!(*r, Recipe::Special(kind), "recipe {i}");
                continue;
            }
        };
        let got = match *r {
            Recipe::Shaped {
                width,
                height,
                inputs,
                output,
            } => (width, height, inputs.to_vec(), output),
            Recipe::Shapeless { inputs, output } => {
                (0, 0, inputs.iter().map(|&x| Some(x)).collect(), output)
            }
            Recipe::Special(_) => panic!("recipe {i}: special in table, {} in oracle", v["type"]),
        };
        assert_eq!(got, want, "recipe {i}");
        if v["type"] == "shapeless" {
            assert!(matches!(r, Recipe::Shapeless { .. }), "recipe {i}");
        }
    }
}

#[test]
fn recipe_spot_checks() {
    let r = recipes();
    assert_eq!(r.len(), 373);
    assert_eq!(
        r.iter()
            .filter(|x| matches!(x, Recipe::Shaped { .. }))
            .count(),
        307
    );
    assert_eq!(
        r.iter()
            .filter(|x| matches!(x, Recipe::Shapeless { .. }))
            .count(),
        58
    );
    assert_eq!(r[0], Recipe::Special(SpecialRecipe::ArmorDye));
    // Crafting table: 2x2 planks (any wood type) -> 1 crafting table.
    let table = r
        .iter()
        .find(|x| {
            matches!(
                x,
                Recipe::Shaped {
                    output: (58, _, _),
                    ..
                }
            )
        })
        .unwrap();
    assert_eq!(
        *table,
        Recipe::Shaped {
            width: 2,
            height: 2,
            inputs: &[Some((5, RECIPE_ANY_DATA)); 4],
            output: (58, 1, 0)
        }
    );
    // Every ingredient and output is a real 1.8 item.
    for x in r {
        match *x {
            Recipe::Shaped { inputs, output, .. } => {
                assert!(Item::from_id(output.0).is_some());
                assert!(
                    inputs
                        .iter()
                        .flatten()
                        .all(|i| Item::from_id(i.0).is_some())
                );
            }
            Recipe::Shapeless { inputs, output } => {
                assert!(Item::from_id(output.0).is_some());
                assert!(inputs.iter().all(|i| Item::from_id(i.0).is_some()));
            }
            Recipe::Special(_) => {}
        }
    }
}
