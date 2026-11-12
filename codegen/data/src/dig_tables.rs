//! Dig tables of the pre-component versions (1.8.9, 1.12.2, 1.16.5) from an oracle
//! `dig_facts.jsonl.gz` dump (`generated/<v>/dig.rs`): per block its strength (hardness) and the
//! version's tool flag (1.8 / 1.12: the material needs no tool; 1.16.5: the state requires the
//! tool), and per item the blocks on which the real `ItemStack` has a mining speed multiplier other
//! than 1 and those it is effective on.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

use crate::emit::{f32l, header};
use crate::generate::oracle_lines;

type R<T> = Result<T, String>;

fn hex32(v: &Value) -> R<f32> {
    let h = v
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("not a [dec, hex] float: {v}"))?;
    Ok(f32::from_bits(
        u32::from_str_radix(h, 16).map_err(|e| e.to_string())?,
    ))
}

/// `dig.rs` for one version. `flag_field` is the block line's boolean (`no_tool_needed` or
/// `requires_tool`); `strength_field` its hardness (`strength` or `hardness`).
pub fn generate(
    what: &str,
    dump: &Path,
    rel: &str,
    strength_field: &str,
    flag_field: &str,
) -> R<String> {
    let lines = oracle_lines(dump)?;
    let mut strength: Vec<Option<f32>> = Vec::new();
    let mut flags: Vec<bool> = Vec::new();
    let mut items = Vec::new();
    for v in &lines {
        match v["type"].as_str() {
            Some("block") => {
                let id = v["id"].as_u64().ok_or("block id")? as usize;
                if strength.len() <= id {
                    strength.resize(id + 1, None);
                    flags.resize(id + 1, false);
                }
                if let Some(d) = v.get("differing_states").and_then(Value::as_array)
                    && !d.is_empty()
                {
                    return Err(format!(
                        "{}: states differ in hardness / tool flag",
                        v["name"]
                    ));
                }
                strength[id] = Some(hex32(&v[strength_field])?);
                flags[id] = v[flag_field].as_bool().ok_or("flag")?;
            }
            Some("item") => {
                let id = v["id"].as_u64().ok_or("item id")? as u16;
                let mut speeds = Vec::new();
                for s in v["speed_blocks"].as_array().ok_or("speed_blocks")? {
                    let sp = hex32(&s["speed"])?;
                    for b in s["blocks"].as_array().ok_or("blocks")? {
                        speeds.push((b.as_u64().ok_or("block")? as u16, sp));
                    }
                }
                speeds.sort_by_key(|x| x.0);
                let mut eff: Vec<u16> = v["effective_blocks"]
                    .as_array()
                    .ok_or("effective_blocks")?
                    .iter()
                    .map(|b| b.as_u64().unwrap_or(0) as u16)
                    .collect();
                eff.sort_unstable();
                if !speeds.is_empty() || !eff.is_empty() {
                    items.push((id, speeds, eff));
                }
            }
            t => return Err(format!("unknown dig facts line type {t:?}")),
        }
    }
    items.sort_by_key(|x| x.0);
    let mut out = header(what, &[rel.to_owned()]);
    out.push_str(
        "/// Per block id: strength (hardness; `-1.0` unbreakable; `f32::NAN` for unused ids).\n",
    );
    let _ = writeln!(
        out,
        "pub(crate) static BLOCK_STRENGTH: [f32; {}] = [",
        strength.len()
    );
    for chunk in strength.chunks(12) {
        let s: Vec<String> = chunk
            .iter()
            .map(|x| x.map_or("f32::NAN".to_owned(), f32l))
            .collect();
        let _ = writeln!(out, "    {},", s.join(", "));
    }
    let words = flags.len().div_ceil(64);
    let mut w = vec![0u64; words];
    for (i, f) in flags.iter().enumerate() {
        if *f {
            w[i / 64] |= 1 << (i % 64);
        }
    }
    let ws: Vec<String> = w.iter().map(|x| format!("{x:#x}")).collect();
    let _ = writeln!(
        out,
        "];\n\n/// Per block id: the version's tool flag (`{flag_field}`), bit per block.\npub(crate) static BLOCK_TOOL_FLAG: [u64; {words}] = [{}];\n",
        ws.join(", ")
    );
    out.push_str("/// `(item id, (block, speed multiplier) sorted, effective blocks sorted)` for every item\n/// with a multiplier other than 1 or an effective block (sorted by item id).\n");
    out.push_str("/// Spans are `(start, end)` in `ITEM_DIG_SPEEDS` / `ITEM_DIG_BLOCKS` (plain data, no slices).\n");
    let _ = writeln!(
        out,
        "pub(crate) static ITEM_DIG: [(u16, (u16, u16), (u16, u16)); {}] = [",
        items.len()
    );
    let (mut all_s, mut all_e): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    for (id, speeds, eff) in &items {
        let (s0, e0) = (all_s.len(), all_e.len());
        all_s.extend(speeds.iter().map(|(b, sp)| format!("({b}, {})", f32l(*sp))));
        all_e.extend(eff.iter().map(u16::to_string));
        let _ = writeln!(
            out,
            "    ({id}, ({s0}, {}), ({e0}, {})),",
            all_s.len(),
            all_e.len()
        );
    }
    out.push_str("];\n");
    let _ = writeln!(
        out,
        "pub(crate) static ITEM_DIG_SPEEDS: [(u16, f32); {}] = [{}];",
        all_s.len(),
        all_s.join(", ")
    );
    let _ = writeln!(
        out,
        "pub(crate) static ITEM_DIG_BLOCKS: [u16; {}] = [{}];",
        all_e.len(),
        all_e.join(", ")
    );
    Ok(out)
}
