//! Minecraft 26.1 item and dig tables (`generated/v26_1/items.rs`).
//!
//! Source of truth is the jar, dumped by the oracle (`tests/golden/26.1/items.jsonl.gz`): every
//! item's default max stack size, max damage, food, block form and tool component, the block tags
//! the tool rules name (resolved by the game's own tag load), and per block its destroy time,
//! whether drops need the correct tool, and its item form. minecraft-data (`pc/26.1/items.json`)
//! only supplies display names and is cross-checked: ids and names must agree (else generation
//! fails); stack sizes / durabilities that differ are listed as notes (the jar wins).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;

use crate::emit::{f32l, header, lit, name_index};
use crate::generate::oracle_lines;
use crate::model::{self, Paths};

type R<T> = Result<T, String>;

/// Oracle item / dig dump.
pub const ORACLE_ITEMS: &str = "tests/golden/26.1/items.jsonl.gz";

fn hex32(v: &Value) -> R<f32> {
    let h = v
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("not a [dec, hex] float: {v}"))?;
    Ok(f32::from_bits(
        u32::from_str_radix(h, 16).map_err(|e| e.to_string())?,
    ))
}

fn f32_lit(x: f32) -> String {
    if x == f32::MAX {
        "f32::MAX".into()
    } else {
        f32l(x)
    }
}

/// `items.rs` for 26.1; `block_count` must match the dump.
pub fn generate(
    p: &Paths,
    dump: &std::path::Path,
    block_count: usize,
    notes: &mut Vec<String>,
) -> R<String> {
    let lines = oracle_lines(dump)?;
    let md_items = model::items(&p.json("items")?)?;
    let md: BTreeMap<u16, &model::Item> = md_items.iter().map(|i| (i.id, i)).collect();

    let mut block_ids: BTreeMap<String, u16> = BTreeMap::new();
    let mut blocks = Vec::new();
    let mut tags: BTreeMap<String, Vec<u16>> = BTreeMap::new();
    let mut items = Vec::new();
    for v in &lines {
        match v["type"].as_str() {
            Some("block") => {
                let id = v["id"].as_u64().ok_or("block id")? as u16;
                if usize::from(id) != blocks.len() {
                    return Err(format!("item dump: block ids not dense at {id}"));
                }
                if !v["differing_states"]
                    .as_array()
                    .ok_or("differing_states")?
                    .is_empty()
                {
                    return Err(format!(
                        "{}: states differ in destroy time / correct tool",
                        v["name"]
                    ));
                }
                block_ids.insert(v["name"].as_str().ok_or("name")?.to_owned(), id);
                blocks.push(v);
            }
            Some("tag") => {
                let ids = v["blocks"]
                    .as_array()
                    .ok_or("tag blocks")?
                    .iter()
                    .map(|x| x.as_u64().unwrap_or(0) as u16)
                    .collect();
                tags.insert(v["name"].as_str().ok_or("tag name")?.to_owned(), ids);
            }
            Some("item") => items.push(v),
            t => return Err(format!("unknown item dump line type {t:?}")),
        }
    }
    if blocks.len() != block_count {
        return Err(format!(
            "item dump has {} blocks, tables {block_count}",
            blocks.len()
        ));
    }
    let mut item_ids: BTreeMap<String, u16> = BTreeMap::new();
    for (i, v) in items.iter().enumerate() {
        let id = v["id"].as_u64().ok_or("item id")? as usize;
        if id != i {
            return Err(format!("item ids not dense at {id}"));
        }
        item_ids.insert(v["name"].as_str().ok_or("item name")?.to_owned(), id as u16);
    }

    // Block sets: the named tags first (sorted), then direct lists in first-use order.
    let words = block_count.div_ceil(64);
    let mut sets: Vec<(String, Vec<u16>)> = tags.into_iter().collect();
    let set_of = |sets: &mut Vec<(String, Vec<u16>)>, rule: &Value| -> R<usize> {
        if let Some(t) = rule["tag"].as_str() {
            return sets
                .iter()
                .position(|s| s.0 == t)
                .ok_or_else(|| format!("tag {t} not dumped"));
        }
        let mut ids: Vec<u16> = rule["blocks"]
            .as_array()
            .ok_or("rule blocks")?
            .iter()
            .map(|n| {
                block_ids
                    .get(n.as_str().unwrap_or_default())
                    .copied()
                    .ok_or_else(|| format!("block {n}"))
            })
            .collect::<R<_>>()?;
        ids.sort_unstable();
        if let Some(i) = sets.iter().position(|s| s.0.is_empty() && s.1 == ids) {
            return Ok(i);
        }
        sets.push((String::new(), ids));
        Ok(sets.len() - 1)
    };

    let mut tools: Vec<String> = Vec::new();
    let mut item_rows = Vec::new();
    let mut md_diffs = Vec::new();
    for v in &items {
        let id = v["id"].as_u64().unwrap_or(0) as u16;
        let name = v["name"].as_str().unwrap_or_default();
        let m = md
            .get(&id)
            .ok_or_else(|| format!("minecraft-data has no item {id} ({name})"))?;
        if m.name != name {
            return Err(format!("item {id}: jar {name}, minecraft-data {}", m.name));
        }
        let stack = v["max_stack_size"].as_u64().ok_or("max_stack_size")?;
        let damage = v["max_damage"].as_u64();
        if u64::from(m.stack_size) != stack || m.max_durability.map(u64::from) != damage {
            md_diffs.push(format!(
                "{name} (stack {} vs {stack}, durability {:?} vs {damage:?})",
                m.stack_size, m.max_durability
            ));
        }
        let food = match v["food"].as_object() {
            None => "None".to_owned(),
            Some(f) => format!(
                "Some(Food {{ nutrition: {}, saturation: {}, can_always_eat: {} }})",
                f["nutrition"].as_u64().ok_or("nutrition")?,
                f32l(hex32(&f["saturation"])?),
                f["can_always_eat"].as_bool().ok_or("can_always_eat")?
            ),
        };
        let block = match v["block"].as_str() {
            None => "None".to_owned(),
            Some(b) => format!(
                "Some({})",
                block_ids.get(b).ok_or_else(|| format!("item block {b}"))?
            ),
        };
        let tool = match v["tool"].as_object() {
            None => "None".to_owned(),
            Some(t) => {
                let mut rules = Vec::new();
                for r in t["rules"].as_array().ok_or("rules")? {
                    let set = set_of(&mut sets, r)?;
                    let speed = match r.get("speed") {
                        Some(s) => format!("Some({})", f32_lit(hex32(s)?)),
                        None => "None".into(),
                    };
                    let correct = match r.get("correct_for_drops") {
                        Some(c) => format!("Some({})", c.as_bool().ok_or("correct_for_drops")?),
                        None => "None".into(),
                    };
                    rules.push(format!("ToolRule {{ blocks: BlockSet({set}), speed: {speed}, correct_for_drops: {correct} }}"));
                }
                let lit = format!(
                    "Tool {{ rules: &[{}], default_mining_speed: {}, damage_per_block: {}, can_destroy_blocks_in_creative: {} }}",
                    rules.join(", "),
                    f32_lit(hex32(&t["default_mining_speed"])?),
                    t["damage_per_block"].as_u64().ok_or("damage_per_block")?,
                    t["can_destroy_blocks_in_creative"]
                        .as_bool()
                        .ok_or("can_destroy")?
                );
                let i = tools.iter().position(|x| *x == lit).unwrap_or_else(|| {
                    tools.push(lit);
                    tools.len() - 1
                });
                format!("Some({i})")
            }
        };
        item_rows.push(format!(
            "    ItemData {{ name: {}, #[cfg(feature = \"display-names\")] display_name: {}, max_stack_size: {stack}, max_damage: {}, block: {block}, food: {food}, tool: {tool} }},",
            lit(name),
            lit(&m.display_name),
            damage.map_or("None".into(), |d| format!("Some({d})")),
        ));
    }
    if !md_diffs.is_empty() {
        notes.push(format!(
            "26.1 items, minecraft-data vs the jar (jar used): {} differ: {}",
            md_diffs.len(),
            md_diffs.join(", ")
        ));
    }

    let mut out = header(
        "Minecraft 26.1 item and dig tables (jar; display names from minecraft-data).",
        &[ORACLE_ITEMS.to_owned(), format!("{}/items.json", p.version)],
    );
    out.push_str("use crate::v26_1::items::{BlockSet, Food, ItemData, Tool, ToolRule};\n\n");
    let _ = writeln!(
        out,
        "pub(crate) static ITEMS: [ItemData; {}] = [",
        item_rows.len()
    );
    for r in &item_rows {
        out.push_str(r);
        out.push('\n');
    }
    out.push_str("];\n\n");
    name_index(
        &mut out,
        "pub(crate) static ITEM_NAMES",
        "u16",
        item_ids
            .iter()
            .map(|(n, i)| (n.clone(), u32::from(*i)))
            .collect(),
    );
    let _ = writeln!(
        out,
        "\npub(crate) static TOOLS: [Tool; {}] = [",
        tools.len()
    );
    for t in &tools {
        let _ = writeln!(out, "    {t},");
    }
    out.push_str("];\n\n/// Block sets the tool rules test: named tags (resolved by the jar), then direct block lists.\n");
    let _ = writeln!(
        out,
        "pub(crate) static BLOCK_SET_NAMES: [&str; {}] = [",
        sets.len()
    );
    for (n, _) in &sets {
        let _ = writeln!(out, "    {},", lit(n));
    }
    let _ = writeln!(
        out,
        "];\n\npub(crate) static BLOCK_SETS: [[u64; {words}]; {}] = [",
        sets.len()
    );
    for (_, ids) in &sets {
        let mut w = vec![0u64; words];
        for &b in ids {
            w[usize::from(b) / 64] |= 1 << (b % 64);
        }
        let s: Vec<String> = w.iter().map(|x| format!("{x:#x}")).collect();
        let _ = writeln!(out, "    [{}],", s.join(", "));
    }
    out.push_str("];\n\n/// Per block: destroy time (`-1.0` = unbreakable).\n");
    let _ = writeln!(
        out,
        "pub(crate) static BLOCK_DESTROY_TIME: [f32; {block_count}] = ["
    );
    for chunk in blocks.chunks(16) {
        let s: Vec<String> = chunk
            .iter()
            .map(|b| hex32(&b["destroy_time"]).map(f32_lit))
            .collect::<R<_>>()?;
        let _ = writeln!(out, "    {},", s.join(", "));
    }
    out.push_str("];\n\n/// Per block: drops need the correct tool (bit per block).\n");
    let mut req = vec![0u64; words];
    for b in &blocks {
        if b["requires_correct_tool"]
            .as_bool()
            .ok_or("requires_correct_tool")?
        {
            let id = b["id"].as_u64().unwrap_or(0) as usize;
            req[id / 64] |= 1 << (id % 64);
        }
    }
    let s: Vec<String> = req.iter().map(|x| format!("{x:#x}")).collect();
    let _ = writeln!(
        out,
        "pub(crate) static BLOCK_REQUIRES_TOOL: [u64; {words}] = [{}];",
        s.join(", ")
    );
    out.push_str("\n/// Per block: its item form (`Block.asItem`; air for blocks without one).\n");
    let _ = writeln!(
        out,
        "pub(crate) static BLOCK_ITEM: [u16; {block_count}] = ["
    );
    for chunk in blocks.chunks(20) {
        let s: Vec<String> = chunk
            .iter()
            .map(|b| {
                let n = b["item"].as_str().unwrap_or_default();
                item_ids
                    .get(n)
                    .map(u16::to_string)
                    .ok_or_else(|| format!("block item {n}"))
            })
            .collect::<R<_>>()?;
        let _ = writeln!(out, "    {},", s.join(", "));
    }
    out.push_str("];\n");
    Ok(out)
}
