//! Crosshair (selection) shape tables from an oracle `selection_shapes.jsonl.gz` dump: per state the
//! outline shape (empty hand), the outline while holding the block's own item where it differs, the
//! interaction / raycast shape, and whether the outline is moved by the block's model offset. Used
//! for 1.16.5 and 26.1 (same dump schema; see oracle/README.md).

use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

use crate::emit::{f64l, header, num_table};
use crate::generate::oracle_lines;

type R<T> = Result<T, String>;

fn hex(v: &Value) -> R<f64> {
    let h = v
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("not a [dec, hex] float: {v}"))?;
    Ok(f64::from_bits(
        u64::from_str_radix(h, 16).map_err(|e| e.to_string())?,
    ))
}

fn shape_lit(v: &Value) -> R<String> {
    let sid = v["sid"].as_u64().ok_or("sid")?;
    let cube = match v["class"].as_str() {
        Some("simple") => true,
        Some("array") => false,
        c => return Err(format!("selection shape {sid}: class {c:?} not supported")),
    };
    let sizes: Vec<u64> = v["sizes"]
        .as_array()
        .ok_or("sizes")?
        .iter()
        .map(|x| x.as_u64().unwrap_or(0))
        .collect();
    let pts = |k: &str| -> R<String> {
        let p: Vec<String> = v[k]
            .as_array()
            .ok_or("points")?
            .iter()
            .map(|c| hex(c).map(f64l))
            .collect::<R<_>>()?;
        Ok(p.join(", "))
    };
    let filled = v["filled"].as_str().ok_or("filled")?;
    let n = (sizes[0] * sizes[1] * sizes[2]) as usize;
    if filled.len() != n {
        return Err(format!(
            "selection shape {sid}: {} cells, sizes {sizes:?}",
            filled.len()
        ));
    }
    let mut words = vec![0u64; n.div_ceil(64)];
    for (i, c) in filled.bytes().enumerate() {
        if c == b'1' {
            words[i / 64] |= 1 << (i % 64);
        }
    }
    let w: Vec<String> = words.iter().map(|x| format!("{x:#x}")).collect();
    let mut boxes = Vec::new();
    for b in v["boxes"].as_array().ok_or("boxes")? {
        let c: Vec<String> = b
            .as_array()
            .ok_or("box")?
            .iter()
            .map(|x| hex(x).map(f64l))
            .collect::<R<_>>()?;
        if c.len() != 6 {
            return Err(format!("selection shape {sid}: box {b}"));
        }
        boxes.push(format!("Aabb::new({})", c.join(", ")));
    }
    Ok(format!(
        "SelectionShape {{ cube: {cube}, sizes: [{}, {}, {}], xs: &[{}], ys: &[{}], zs: &[{}], cells: &[{}], empty: {}, boxes: &[{}] }}",
        sizes[0],
        sizes[1],
        sizes[2],
        pts("points_x")?,
        pts("points_y")?,
        pts("points_z")?,
        w.join(", "),
        v["empty"].as_bool().ok_or("empty")?,
        boxes.join(", ")
    ))
}

/// `selection.rs` for one version: `what` names it in the header, `state_count` must match the dump.
pub fn generate(what: &str, dump: &Path, rel: &str, state_count: usize) -> R<String> {
    let lines = oracle_lines(dump)?;
    let mut shapes = Vec::new();
    let mut outline = vec![u32::MAX; state_count];
    let mut interaction = vec![0u32; state_count];
    let mut offset = vec![0u64; state_count.div_ceil(64)];
    let mut holding = Vec::new();
    for v in &lines {
        match v["type"].as_str() {
            Some("voxel_shape") => {
                let sid = v["sid"].as_u64().ok_or("sid")? as usize;
                if sid != shapes.len() {
                    return Err(format!(
                        "selection sids not dense: {sid} after {}",
                        shapes.len()
                    ));
                }
                shapes.push(shape_lit(v)?);
            }
            Some("state") => {
                let id = v["id"].as_u64().ok_or("state id")? as usize;
                if id >= state_count {
                    return Err(format!("selection state {id} out of range"));
                }
                let name = v["state"].as_str().unwrap_or_default();
                if v["position_independent"].as_bool() != Some(true) {
                    return Err(format!("{name}: outline depends on the position"));
                }
                // Picks always run with the player's context; the empty context only differs for
                // shapes the pick never asks for that way.
                let sid = |k: &str| v[k].as_u64().map(|x| x as u32);
                if let Some(base) = sid("outline_base") {
                    outline[id] = base;
                    offset[id / 64] |= 1 << (id % 64);
                } else {
                    outline[id] = sid("outline").ok_or("outline")?;
                }
                interaction[id] = sid("interaction").ok_or("interaction")?;
                if let Some(h) = sid("outline_holding_own_item") {
                    if v.get("outline_base").is_some() {
                        return Err(format!("{name}: offset outline with a held-item variant"));
                    }
                    holding.push((id as u32, h));
                }
            }
            t => return Err(format!("unknown selection dump line type {t:?}")),
        }
    }
    if outline.iter().any(|&s| s as usize >= shapes.len())
        || interaction.iter().any(|&s| s as usize >= shapes.len())
    {
        return Err("selection dump does not cover every state".into());
    }
    let mut out = header(what, &[rel.to_owned()]);
    out.push_str("use crate::common::{Aabb, SelectionShape};\n\n");
    let _ = writeln!(
        out,
        "/// Distinct outline / interaction shapes (oracle; offset outlines list their unshifted shape).\npub(crate) static SELECTION_SHAPES: [SelectionShape; {}] = [",
        shapes.len()
    );
    for s in &shapes {
        let _ = writeln!(out, "    {s},");
    }
    out.push_str("];\n\n/// Outline shape per state (empty hand; the unshifted shape for offset outlines).\n");
    num_table(
        &mut out,
        "pub(crate) static STATE_OUTLINE",
        "u16",
        &outline,
        20,
    );
    out.push_str("\n/// Interaction (raycast) shape per state.\n");
    num_table(
        &mut out,
        "pub(crate) static STATE_INTERACTION",
        "u16",
        &interaction,
        20,
    );
    out.push_str(
        "\n/// States whose outline is moved by the block's model offset (bit per state).\n",
    );
    let _ = writeln!(
        out,
        "pub(crate) static STATE_OUTLINE_OFFSET: [u64; {}] = [",
        offset.len()
    );
    for chunk in offset.chunks(8) {
        let w: Vec<String> = chunk.iter().map(|x| format!("{x:#x}")).collect();
        let _ = writeln!(out, "    {},", w.join(", "));
    }
    out.push_str("];\n\n/// `(state, shape)`: the outline while holding the block's own item, where it differs (sorted).\n");
    let _ = writeln!(
        out,
        "pub(crate) static OUTLINE_HOLDING_OWN_ITEM: [(u16, u16); {}] = [",
        holding.len()
    );
    for (s, h) in &holding {
        let _ = writeln!(out, "    ({s}, {h}),");
    }
    out.push_str("];\n");
    Ok(out)
}
