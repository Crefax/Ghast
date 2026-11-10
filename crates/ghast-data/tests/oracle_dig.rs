//! Block breaking against the real clients (`tests/golden/<v>/dig.jsonl.gz`): the per-tick
//! progress bit for bit and the call on which the finish packet went out, plus the jar's tool
//! tables (`dig_facts.jsonl.gz`).

use std::io::{BufRead, BufReader};

use ghast_data::{BreakTime, break_time};
use serde_json::Value;

const LIMIT: u32 = 2000;

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

fn f32_hex(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v[1].as_str().unwrap(), 16).unwrap())
}

fn amp(v: &Value) -> Option<i32> {
    Some(v.as_i64().unwrap() as i32).filter(|a| *a >= 0)
}

/// Checks the dig outcomes of one case against `progress` (creative: always the start packet).
fn check_timing(v: &Value, progress: f32) -> usize {
    let k = v["k"].as_u64().unwrap();
    let mut n = 0;
    for (i, d) in v["first"].as_array().unwrap().iter().enumerate() {
        let got = d["result"].as_str().unwrap();
        if v["creative"].as_bool().unwrap() {
            assert_eq!(got, "instant", "case {k}: creative");
            continue;
        }
        let calls = d["calls"].as_u64().unwrap() as u32;
        let want = match got {
            "instant" => BreakTime::Instant,
            "ticks" => BreakTime::Ticks(calls),
            "none" => BreakTime::Never,
            r => panic!("case {k}: result {r}"),
        };
        assert_eq!(
            break_time(progress, if i == 0 { 0 } else { 5 }, LIMIT),
            want,
            "case {k} dig {i}"
        );
        n += 1;
    }
    n
}

#[cfg(feature = "v1_8")]
#[test]
fn dig_1_8_9() {
    use ghast_data::v1_8::dig::{self, DigConditions};
    for v in golden("1.8.9", "dig_facts.jsonl.gz") {
        let id = v["id"].as_u64().unwrap() as u16;
        match v["type"].as_str().unwrap() {
            "block" => {
                assert_eq!(
                    dig::strength(id).to_bits(),
                    f32_hex(&v["strength"]).to_bits()
                );
                assert_eq!(
                    dig::no_tool_needed(id),
                    v["no_tool_needed"].as_bool().unwrap()
                );
            }
            _ => {
                let mut want = std::collections::BTreeMap::new();
                for s in v["speed_blocks"].as_array().unwrap() {
                    for b in s["blocks"].as_array().unwrap() {
                        want.insert(b.as_u64().unwrap() as u16, f32_hex(&s["speed"]));
                    }
                }
                let eff: Vec<u16> = v["effective_blocks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| b.as_u64().unwrap() as u16)
                    .collect();
                for b in 0..198u16 {
                    assert_eq!(
                        dig::speed_multiplier(id, b),
                        *want.get(&b).unwrap_or(&1.0),
                        "item {id} block {b}"
                    );
                    assert_eq!(
                        dig::is_effective(id, b),
                        eff.contains(&b),
                        "item {id} block {b}"
                    );
                }
            }
        }
    }
    let (mut cases, mut digs) = (0, 0);
    for v in golden("1.8.9", "dig.jsonl.gz") {
        let block = (v["raw"].as_u64().unwrap() >> 4) as u16;
        let held = Some(v["held_id"].as_i64().unwrap())
            .filter(|i| *i >= 0)
            .map(|i| i as u16);
        let c = DigConditions {
            efficiency: v["efficiency"].as_u64().unwrap() as u8,
            haste: amp(&v["haste"]),
            mining_fatigue: amp(&v["mining_fatigue"]),
            eye_in_water: v["eye_in_water"].as_bool().unwrap(),
            aqua_affinity: v["aqua_affinity"].as_bool().unwrap(),
            on_ground: v["on_ground"].as_bool().unwrap(),
        };
        let k = v["k"].as_u64().unwrap();
        assert_eq!(
            dig::mining_speed(held, block, &c).to_bits(),
            f32_hex(&v["mining_speed"]).to_bits(),
            "case {k}"
        );
        assert_eq!(
            dig::effective_tool(held, block),
            v["effective_tool"].as_bool().unwrap(),
            "case {k}"
        );
        let p = dig::destroy_progress(held, block, &c);
        assert_eq!(p.to_bits(), f32_hex(&v["progress"]).to_bits(), "case {k}");
        digs += check_timing(&v, p);
        cases += 1;
    }
    assert!(cases >= 3000 && digs > 2500, "{cases} / {digs}");
}
