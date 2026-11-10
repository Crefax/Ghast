//! Minecraft 26.1 (protocol 775) block tables.
//!
//! Source of truth is the real 26.1 jar, dumped by the oracle (`tests/golden/26.1/voxel_shapes.jsonl.gz`):
//! collision voxel grids, per-state flags, fluids, per-block friction / speed / jump factors, the
//! tags and `instanceof` tests the movement code makes, model offsets. PrismarineJS minecraft-data
//! (`pc/26.1`) supplies the block-state property layout and display data, and is cross-checked
//! against the jar: block names, state ranges and default states must agree (else generation
//! fails); collision shapes that disagree are listed as notes (the jar wins).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;

use crate::emit::{camel_ident, f32l, f64l, header, lit, name_index, upper_ident};
use crate::generate::oracle_lines;
use crate::model::{self, Block, Paths};

type R<T> = Result<T, String>;
type File = (String, String);

/// Oracle voxel dump the 26.1 tables come from.
pub const ORACLE_VOXELS: &str = "tests/golden/26.1/voxel_shapes.jsonl.gz";

fn hex64(v: &Value) -> R<f64> {
    let h = v
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("not a [dec, hex] float: {v}"))?;
    Ok(f64::from_bits(
        u64::from_str_radix(h, 16).map_err(|e| e.to_string())?,
    ))
}

fn hex32(v: &Value) -> R<f32> {
    let h = v
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("not a [dec, hex] float: {v}"))?;
    Ok(f32::from_bits(
        u32::from_str_radix(h, 16).map_err(|e| e.to_string())?,
    ))
}

fn flag(v: &Value, k: &str) -> R<bool> {
    v[k].as_bool().ok_or_else(|| format!("missing bool {k}: {v}"))
}

/// Block kinds the movement code treats specially (`v26_1::BlockKind` discriminants).
const KINDS: &[(&str, &str)] = &[
    ("cobweb", "Cobweb"),
    ("sweet_berry_bush", "SweetBerryBush"),
    ("powder_snow", "PowderSnow"),
    ("honey_block", "HoneyBlock"),
    ("bubble_column", "BubbleColumn"),
    ("slime_block", "SlimeBlock"),
    ("scaffolding", "Scaffolding"),
    ("ladder", "Ladder"),
    ("water", "Water"),
    ("lava", "Lava"),
    ("moving_piston", "MovingPiston"),
];

/// Block flag bits (`v26_1::block_flags`), from the dump's tags and `instanceof` tests.
const BLOCK_FLAG_BITS: &[(&str, u32)] = &[
    ("tags.climbable", 1),
    ("tags.fences", 2),
    ("tags.walls", 4),
    ("is.fence_gate", 8),
    ("is.trapdoor", 16),
    ("is.ice", 32),
    ("is.bed", 64),
    ("tags.fall_damage_resetting", 128),
    ("tags.soul_speed_blocks", 256),
    ("tags.fire", 512),
];

struct OracleBlock {
    first: u32,
    count: u32,
    default: u32,
    friction: f32,
    speed: f32,
    jump: f32,
    flags: u32,
    kind: &'static str,
    offset: Option<(f32, f32, bool)>,
}

/// A voxel shape as emitted (deduplicated by content).
#[derive(Clone, PartialEq)]
struct Voxel {
    class: &'static str,
    sizes: [u64; 3],
    pts: [Vec<u64>; 3],
    cells: Vec<u64>,
    empty: bool,
    boxes: Vec<[u64; 6]>,
}

fn voxel_of(v: &Value) -> R<Voxel> {
    let class = match v["class"].as_str() {
        Some("simple") => "Simple",
        Some("array") => "Array",
        c => return Err(format!("voxel shape class {c:?} not supported")),
    };
    let sizes: Vec<u64> = v["sizes"]
        .as_array()
        .ok_or("sizes")?
        .iter()
        .map(|x| x.as_u64().unwrap_or(0))
        .collect();
    let pts = |k: &str| -> R<Vec<u64>> {
        v[k].as_array()
            .ok_or("points")?
            .iter()
            .map(|c| hex64(c).map(f64::to_bits))
            .collect()
    };
    let filled = v["filled"].as_str().ok_or("filled")?;
    let n = (sizes[0] * sizes[1] * sizes[2]) as usize;
    if filled.len() != n {
        return Err(format!("voxel shape: {} cells, sizes {sizes:?}", filled.len()));
    }
    let mut cells = vec![0u64; n.div_ceil(64).max(1)];
    for (i, c) in filled.bytes().enumerate() {
        if c == b'1' {
            cells[i / 64] |= 1 << (i % 64);
        }
    }
    let mut boxes = Vec::new();
    for b in v["boxes"].as_array().ok_or("boxes")? {
        let mut a = [0u64; 6];
        for (i, c) in b.as_array().ok_or("box")?.iter().enumerate() {
            a[i] = hex64(c)?.to_bits();
        }
        boxes.push(a);
    }
    Ok(Voxel {
        class,
        sizes: [sizes[0], sizes[1], sizes[2]],
        pts: [pts("points_x")?, pts("points_y")?, pts("points_z")?],
        cells,
        empty: flag(v, "empty")?,
        boxes,
    })
}

fn intern(shapes: &mut Vec<Voxel>, v: Voxel) -> u32 {
    match shapes.iter().position(|s| *s == v) {
        Some(i) => i as u32,
        None => {
            shapes.push(v);
            (shapes.len() - 1) as u32
        }
    }
}

fn voxel_lit(v: &Voxel) -> String {
    let p = |a: &[u64]| -> String {
        a.iter()
            .map(|&b| f64l(f64::from_bits(b)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let cells: Vec<String> = v.cells.iter().map(|x| format!("{x:#x}")).collect();
    let boxes: Vec<String> = v
        .boxes
        .iter()
        .map(|b| {
            format!(
                "Aabb::new({}, {}, {}, {}, {}, {})",
                f64l(f64::from_bits(b[0])),
                f64l(f64::from_bits(b[1])),
                f64l(f64::from_bits(b[2])),
                f64l(f64::from_bits(b[3])),
                f64l(f64::from_bits(b[4])),
                f64l(f64::from_bits(b[5]))
            )
        })
        .collect();
    format!(
        "VoxelShapeRef {{ class: VoxelClass::{}, sizes: [{}, {}, {}], xs: &[{}], ys: &[{}], zs: &[{}], cells: &[{}], empty: {}, boxes: &[{}] }}",
        v.class,
        v.sizes[0],
        v.sizes[1],
        v.sizes[2],
        p(&v.pts[0]),
        p(&v.pts[1]),
        p(&v.pts[2]),
        cells.join(", "),
        v.empty,
        boxes.join(", ")
    )
}

/// Decodes a state id into property value indices (first property most significant).
fn decode(b: &Block, s: u32) -> Vec<usize> {
    let mut rem = (s - b.min_state) as usize;
    let mut idx = vec![0; b.props.len()];
    for (i, p) in b.props.iter().enumerate().rev() {
        let n = p.values.len();
        idx[i] = rem % n;
        rem /= n;
    }
    idx
}

fn prop_value<'a>(b: &'a Block, idx: &[usize], name: &str) -> Option<&'a str> {
    b.props
        .iter()
        .position(|p| p.name == name)
        .map(|i| b.props[i].values[idx[i]].as_str())
}

fn facing_index(f: Option<&str>) -> u32 {
    match f {
        Some("north") => 0,
        Some("east") => 1,
        Some("south") => 2,
        Some("west") => 3,
        Some("up") => 4,
        Some("down") => 5,
        _ => 7,
    }
}

/// Geometric union equality of two box lists (block-local), on the grid of all their coordinates.
fn same_union(a: &[[f64; 6]], b: &[[f64; 6]]) -> bool {
    let mut axes: [Vec<f64>; 3] = Default::default();
    for bx in a.iter().chain(b) {
        for k in 0..3 {
            axes[k].push(bx[k]);
            axes[k].push(bx[k + 3]);
        }
    }
    for ax in &mut axes {
        ax.sort_by(f64::total_cmp);
        ax.dedup();
    }
    let covered = |set: &[[f64; 6]], p: [f64; 3]| {
        set.iter()
            .any(|bx| (0..3).all(|k| bx[k] < p[k] && p[k] < bx[k + 3]))
    };
    for x in axes[0].windows(2) {
        for y in axes[1].windows(2) {
            for z in axes[2].windows(2) {
                let p = [(x[0] + x[1]) / 2.0, (y[0] + y[1]) / 2.0, (z[0] + z[1]) / 2.0];
                if covered(a, p) != covered(b, p) {
                    return false;
                }
            }
        }
    }
    true
}

pub fn generate(p: &Paths, oracle_voxels: &std::path::Path, notes: &mut Vec<String>) -> R<Vec<File>> {
    let blocks = model::blocks(&p.json("blocks")?)?;
    for (i, b) in blocks.iter().enumerate() {
        if usize::from(b.id) != i {
            return Err(format!("26.1 block ids not dense at {}", b.name));
        }
    }
    // State layout as in 1.16.5: contiguous ranges, product of property sizes.
    let mut next = 0u32;
    for b in &blocks {
        let n: u32 = b.props.iter().map(|p| p.values.len() as u32).product();
        if b.min_state != next || b.max_state + 1 - b.min_state != n {
            return Err(format!("26.1 {}: bad state range", b.name));
        }
        next = b.max_state + 1;
    }
    let state_count = next as usize;
    if state_count > usize::from(u16::MAX) {
        return Err("too many 26.1 states for u16".into());
    }

    // --- the jar's facts ---
    let lines = oracle_lines(oracle_voxels)?;
    let mut ob: BTreeMap<String, OracleBlock> = BTreeMap::new();
    let mut dump_shapes: BTreeMap<u64, Voxel> = BTreeMap::new();
    let mut shapes: Vec<Voxel> = Vec::new();
    // Index 0 = the empty shape (as the dump's sid 0).
    let mut state_shape = vec![u32::MAX; state_count];
    let mut state_flags = vec![0u32; state_count];
    let mut state_fluid = vec![0u32; state_count];
    let mut state_offset_shape: BTreeMap<usize, Voxel> = BTreeMap::new();
    let mut ctx_shapes: BTreeMap<usize, serde_json::Map<String, Value>> = BTreeMap::new();
    let mut contexts: BTreeMap<String, (f64, bool, f64, bool)> = BTreeMap::new();
    let mut boxes_of_state: Vec<Vec<[f64; 6]>> = vec![Vec::new(); state_count];
    for v in &lines {
        match v["type"].as_str() {
            Some("block") => {
                let name = v["name"].as_str().ok_or("name")?;
                let short = name.trim_start_matches("minecraft:");
                let mut flags = 0u32;
                for (path, bit) in BLOCK_FLAG_BITS {
                    let (group, key) = path.split_once('.').ok_or("flag path")?;
                    if v[group][key]
                        .as_bool()
                        .ok_or_else(|| format!("{name}: {path}"))?
                    {
                        flags |= bit;
                    }
                }
                let kind = KINDS
                    .iter()
                    .find(|k| k.0 == short)
                    .map_or("Other", |k| k.1);
                let offset = match &v["offset"] {
                    Value::Null => None,
                    o => Some((hex32(&o["max_h"])?, hex32(&o["max_v"])?, flag(o, "xyz")?)),
                };
                ob.insert(
                    short.to_owned(),
                    OracleBlock {
                        first: v["first_state"].as_u64().ok_or("first_state")? as u32,
                        count: v["states"].as_u64().ok_or("states")? as u32,
                        default: v["default_state"].as_u64().ok_or("default_state")? as u32,
                        friction: hex32(&v["slipperiness"])?,
                        speed: hex32(&v["velocity_multiplier"])?,
                        jump: hex32(&v["jump_velocity_multiplier"])?,
                        flags,
                        kind,
                        offset,
                    },
                );
            }
            Some("voxel_shape") => {
                let sid = v["sid"].as_u64().ok_or("sid")?;
                dump_shapes.insert(sid, voxel_of(v)?);
            }
            Some("context") => {
                let name = v["name"].as_str().ok_or("context")?.to_owned();
                contexts.insert(
                    name,
                    (
                        hex64(&v["feet_dy"])?,
                        flag(v, "descending")?,
                        hex64(&v["fall_distance"])?,
                        flag(v, "leather_boots")?,
                    ),
                );
            }
            Some("state") => {
                let id = v["id"].as_u64().ok_or("state id")? as usize;
                if id >= state_count {
                    return Err(format!("26.1 dump state {id} beyond minecraft-data's {state_count}"));
                }
                let sid = v["collision_shape"].as_u64().ok_or("collision_shape")?;
                let shape = dump_shapes
                    .get(&sid)
                    .ok_or_else(|| format!("state {id}: shape {sid} not yet defined"))?
                    .clone();
                boxes_of_state[id] = shape
                    .boxes
                    .iter()
                    .map(|b| b.map(f64::from_bits))
                    .collect();
                let mut f = 0u32;
                let bit = |k: &str, b: u32| -> R<u32> { Ok(if flag(v, k)? { b } else { 0 }) };
                f |= bit("full_cube_identity", 1)?;
                f |= if shape.empty { 2 } else { 0 };
                f |= bit("exceeds_cube", 4)?;
                f |= bit("should_suffocate", 8)?;
                f |= bit("is_air", 16)?;
                f |= bit("blocks_motion", 32)?;
                for (i, side) in v["side_solid_full_square"]
                    .as_array()
                    .ok_or("sides")?
                    .iter()
                    .enumerate()
                {
                    if side.as_bool().ok_or("side")? {
                        f |= 64 << i;
                    }
                }
                if v.get("context_shapes").is_some_and(Value::is_object) {
                    f |= 1 << 10;
                    ctx_shapes.insert(id, v["context_shapes"].as_object().cloned().unwrap_or_default());
                }
                if v.get("base_points_x").is_some() {
                    f |= 1 << 11;
                    let mut base = shape.clone();
                    base.class = "Array";
                    for (k, axis) in ["base_points_x", "base_points_y", "base_points_z"].iter().enumerate() {
                        base.pts[k] = v[*axis]
                            .as_array()
                            .ok_or("base points")?
                            .iter()
                            .map(|c| hex64(c).map(f64::to_bits))
                            .collect::<R<_>>()?;
                    }
                    // Box list without the dump position's offset (the block-local unshifted boxes).
                    let off: Vec<f64> = v["offset_at_dump"]
                        .as_array()
                        .ok_or("offset_at_dump")?
                        .iter()
                        .map(hex64)
                        .collect::<R<_>>()?;
                    base.boxes = shape
                        .boxes
                        .iter()
                        .map(|b| {
                            let mut o = *b;
                            for k in 0..6 {
                                o[k] = (f64::from_bits(b[k]) - off[k % 3]).to_bits();
                            }
                            o
                        })
                        .collect();
                    state_offset_shape.insert(id, base);
                }
                if v["state"].as_str().is_some_and(|s| s.starts_with("minecraft:moving_piston")) {
                    f |= 1 << 12;
                }
                state_flags[id] = f;
                state_fluid[id] = match &v["fluid"] {
                    Value::Null => 0,
                    fl => {
                        let kind = match fl["kind"].as_str() {
                            Some("water") => 1,
                            Some("lava") => 2,
                            k => return Err(format!("state {id}: fluid kind {k:?}")),
                        };
                        let level = fl["level"].as_u64().ok_or("fluid level")? as u32;
                        if !(1..=8).contains(&level) {
                            return Err(format!("state {id}: fluid amount {level}"));
                        }
                        // Height must be amount / 9 (and 1 with the same fluid above).
                        let h = hex32(&fl["height"])?;
                        if h != level as f32 / 9.0 || hex32(&fl["height_fluid_above"])? != 1.0 {
                            return Err(format!("state {id}: fluid height {h} for amount {level}"));
                        }
                        (kind << 6) | (u32::from(flag(fl, "falling")?) << 4) | level
                    }
                };
                state_shape[id] = intern(&mut shapes, shape);
            }
            t => return Err(format!("unknown 26.1 voxel dump line {t:?}")),
        }
    }
    if state_shape.contains(&u32::MAX) {
        return Err("26.1 voxel dump does not cover every state".into());
    }
    if shapes.first().is_none_or(|s| !s.empty) {
        return Err("26.1: first shape is not the empty shape".into());
    }
    // Offset states point at their unshifted shape.
    for (id, base) in state_offset_shape {
        state_shape[id] = intern(&mut shapes, base);
    }

    // --- cross-check with minecraft-data ---
    if ob.len() != blocks.len() {
        return Err(format!(
            "26.1: jar has {} blocks, minecraft-data {}",
            ob.len(),
            blocks.len()
        ));
    }
    let mut block_rows = Vec::new();
    for b in &blocks {
        let o = ob
            .get(&b.name)
            .ok_or_else(|| format!("26.1: minecraft-data block {} not in the jar", b.name))?;
        if o.first != b.min_state || o.count != b.max_state + 1 - b.min_state || o.default != b.default_state {
            return Err(format!(
                "26.1 {}: jar states {}+{} default {}, minecraft-data {}..={} default {}",
                b.name, o.first, o.count, o.default, b.min_state, b.max_state, b.default_state
            ));
        }
        block_rows.push(o);
    }
    let md_shapes = model::shapes(&p.json("blockCollisionShapes")?)?;
    let (mut same, mut decomposition, mut differ) = (0usize, 0usize, Vec::new());
    for b in &blocks {
        let ids = md_shapes
            .blocks
            .get(&b.name)
            .ok_or_else(|| format!("minecraft-data has no collision shape for {}", b.name))?;
        for s in b.min_state..=b.max_state {
            let k = (s - b.min_state) as usize;
            let sid = if ids.len() == 1 { ids[0] } else { ids[k] };
            let md = md_shapes.shapes.get(&sid).ok_or("minecraft-data shape id")?;
            let jar = &boxes_of_state[s as usize];
            let mut a: Vec<[u64; 6]> = md.iter().map(|x| x.map(f64::to_bits)).collect();
            let mut j: Vec<[u64; 6]> = jar.iter().map(|x| x.map(f64::to_bits)).collect();
            a.sort_unstable();
            j.sort_unstable();
            if a == j {
                same += 1;
            } else if same_union(md, jar) {
                decomposition += 1;
            } else {
                differ.push(s);
            }
        }
    }
    let mut by_block: BTreeMap<&str, usize> = BTreeMap::new();
    for &s in &differ {
        let b = blocks
            .iter()
            .find(|b| (b.min_state..=b.max_state).contains(&s))
            .ok_or("state block")?;
        *by_block.entry(b.name.as_str()).or_default() += 1;
    }
    notes.push(format!(
        "26.1 collision shapes, minecraft-data vs the jar: {same} states identical, {decomposition} only split into boxes differently, {} differ (jar used): {}",
        differ.len(),
        by_block
            .iter()
            .map(|(n, c)| format!("{n} x{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));

    // --- context-dependent shapes: scaffolding and powder snow ---
    let full_cube = shapes
        .iter()
        .position(|s| s.class == "Simple" && s.sizes == [1, 1, 1] && !s.empty)
        .ok_or("no full cube shape")? as u32;
    let mut scaffolding_bottom: Option<u32> = None;
    let mut powder_falling: Option<u32> = None;
    let eps = f64::from(1.0e-5_f32);
    for (&id, ctx) in &ctx_shapes {
        let b = blocks
            .iter()
            .find(|b| (b.min_state as usize..=b.max_state as usize).contains(&id))
            .ok_or("ctx block")?;
        let idx = decode(b, id as u32);
        for (cname, sidv) in ctx {
            let sid = sidv.as_u64().ok_or("ctx sid")?;
            let got = dump_shapes.get(&sid).ok_or("ctx shape")?.clone();
            let got_i = shapes.iter().position(|s| *s == got).map(|i| i as u32);
            let &(dy, descending, fall, boots) =
                contexts.get(cname).ok_or_else(|| format!("unknown context {cname}"))?;
            // Contexts sit on the block at y = 64 with feet at 64 + dy.
            let feet = 64.0 + dy;
            match b.name.as_str() {
                "scaffolding" => {
                    let has_bottom = prop_value(b, &idx, "bottom") == Some("true")
                        && prop_value(b, &idx, "distance") != Some("0");
                    if feet > 64.0 + 1.0 - eps && !descending {
                        if got_i != Some(state_shape[id]) {
                            return Err(format!("scaffolding {id} {cname}: not the stable shape"));
                        }
                    } else if has_bottom && feet > 64.0 - eps {
                        let gi = got_i.unwrap_or_else(|| intern(&mut shapes, got.clone()));
                        if *scaffolding_bottom.get_or_insert(gi) != gi || got.empty {
                            return Err(format!("scaffolding {id} {cname}: bottom shape differs"));
                        }
                    } else if !got.empty {
                        return Err(format!("scaffolding {id} {cname}: expected empty"));
                    }
                }
                "powder_snow" => {
                    // fallDistance > 2.5 -> falling shape; leather boots, above (feet > y + 1 - 1e-5F)
                    // and not descending -> full block; else empty.
                    if fall > 2.5 {
                        let gi = got_i.unwrap_or_else(|| intern(&mut shapes, got.clone()));
                        if *powder_falling.get_or_insert(gi) != gi || got.empty {
                            return Err(format!("powder snow {cname}: falling shape differs"));
                        }
                    } else if boots && feet > 64.0 + 1.0 - eps && !descending {
                        if got_i != Some(full_cube) {
                            return Err(format!("powder snow {cname}: expected a full block"));
                        }
                    } else if !got.empty {
                        return Err(format!("powder snow {cname}: expected empty"));
                    }
                }
                n => return Err(format!("unexpected context-dependent block {n}")),
            }
        }
    }
    let scaffolding_bottom = scaffolding_bottom.ok_or("no scaffolding bottom shape")?;
    let powder_falling = powder_falling.ok_or("no powder snow falling shape")?;

    // --- per-state packed record ---
    let mut packed: Vec<u64> = Vec::with_capacity(state_count);
    let mut state_block = Vec::with_capacity(state_count);
    let kind_ids: Vec<&str> = std::iter::once("Other")
        .chain(KINDS.iter().map(|k| k.1))
        .collect();
    for b in &blocks {
        let o = &ob[&b.name];
        let kind = kind_ids.iter().position(|k| *k == o.kind).ok_or("kind")? as u64;
        for s in b.min_state..=b.max_state {
            let idx = decode(b, s);
            let aux: u32 = match b.name.as_str() {
                "bubble_column" => u32::from(prop_value(b, &idx, "drag") == Some("true")),
                "scaffolding" => u32::from(
                    prop_value(b, &idx, "bottom") == Some("true")
                        && prop_value(b, &idx, "distance") != Some("0"),
                ),
                "ladder" => facing_index(prop_value(b, &idx, "facing")),
                _ if o.flags & 16 != 0 => {
                    u32::from(prop_value(b, &idx, "open") == Some("true"))
                        | (facing_index(prop_value(b, &idx, "facing")) << 1)
                }
                _ => 0,
            };
            let s = s as usize;
            packed.push(
                u64::from(state_shape[s])
                    | (u64::from(state_flags[s]) << 16)
                    | (u64::from(state_fluid[s]) << 32)
                    | (u64::from(aux) << 40)
                    | (kind << 48),
            );
            state_block.push(u32::from(b.id));
        }
    }

    // --- properties ---
    let mut defs: Vec<(String, Vec<String>)> = Vec::new();
    let mut block_props: Vec<Vec<usize>> = Vec::new();
    for b in &blocks {
        let mut v = Vec::new();
        for pr in &b.props {
            let key = (pr.name.clone(), pr.values.clone());
            let i = defs.iter().position(|d| *d == key).unwrap_or_else(|| {
                defs.push(key);
                defs.len() - 1
            });
            v.push(i);
        }
        block_props.push(v);
    }
    let prop_names: std::collections::BTreeSet<&str> = defs.iter().map(|d| d.0.as_str()).collect();

    // --- emit ---
    let mut out = header(
        "26.1 block table: per-block data, state layout, per-state packed physics record, collision voxel shapes.",
        &[
            format!("{}/blocks.json", p.dir("blocks")?),
            format!("{}/blockCollisionShapes.json (cross-check only)", p.dir("blockCollisionShapes")?),
            ORACLE_VOXELS.into(),
        ],
    );
    out.push_str("use crate::common::Aabb;\nuse crate::v26_1::{BlockData, BlockKind, PropertyDef, StateLayout, VoxelClass, VoxelShapeRef};\n\n");
    let _ = writeln!(out, "pub(crate) static BLOCKS: [BlockData; {}] = [", blocks.len());
    for (b, o) in blocks.iter().zip(&block_rows) {
        let offset = match o.offset {
            None => "None".to_owned(),
            Some((h, v, xyz)) => format!("Some(({}, {}, {xyz}))", f32l(h), f32l(v)),
        };
        let _ = writeln!(
            out,
            "    BlockData {{ name: {}, display_name: {}, hardness: {}, resistance: {}, diggable: {}, transparent: {}, emit_light: {}, filter_light: {}, friction: {}, speed_factor: {}, jump_factor: {}, flags: {}, kind: BlockKind::{}, offset: {offset} }},",
            lit(&b.name),
            lit(&b.display_name),
            b.hardness.map_or("None".into(), |h| format!("Some({})", f32l(h as f32))),
            f32l(b.resistance as f32),
            b.diggable,
            b.transparent,
            b.emit_light,
            b.filter_light,
            f32l(o.friction),
            f32l(o.speed),
            f32l(o.jump),
            o.flags,
            o.kind
        );
    }
    out.push_str("];\n\n");
    name_index(
        &mut out,
        "pub(crate) static BLOCK_NAMES",
        "u16",
        blocks.iter().map(|b| (b.name.clone(), u32::from(b.id))).collect(),
    );
    out.push_str("\n/// Block constants.\nimpl crate::v26_1::Block {\n");
    for b in &blocks {
        let _ = writeln!(
            out,
            "    /// `{}` ({}).\n    pub const {}: Self = Self({});",
            b.name,
            b.display_name.replace('`', "'"),
            upper_ident(&b.name),
            b.id
        );
    }
    out.push_str("}\n\n/// Every block-state property name used by 26.1.\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]\npub enum Property {\n");
    for n in &prop_names {
        let _ = writeln!(out, "    /// `{n}`\n    {},", camel_ident(n));
    }
    out.push_str("}\n\nimpl Property {\n    /// The property's name as used in block-state strings.\n    #[must_use]\n    pub const fn name(self) -> &'static str {\n        match self {\n");
    for n in &prop_names {
        let _ = writeln!(out, "            Property::{} => {},", camel_ident(n), lit(n));
    }
    out.push_str("        }\n    }\n}\n\n");
    let _ = writeln!(out, "pub(crate) static PROPERTY_NAMES: [(&str, Property); {}] = [", prop_names.len());
    for n in &prop_names {
        let _ = writeln!(out, "    ({}, Property::{}),", lit(n), camel_ident(n));
    }
    out.push_str("];\n\n");
    let _ = writeln!(out, "pub(crate) static PROPERTY_DEFS: [PropertyDef; {}] = [", defs.len());
    for (name, values) in &defs {
        let vs: Vec<String> = values.iter().map(|v| lit(v)).collect();
        let _ = writeln!(
            out,
            "    PropertyDef {{ property: Property::{}, values: &[{}] }},",
            camel_ident(name),
            vs.join(", ")
        );
    }
    out.push_str("];\n\n");
    let _ = writeln!(out, "pub(crate) static LAYOUTS: [StateLayout; {}] = [", blocks.len());
    for (b, props) in blocks.iter().zip(&block_props) {
        let ps: Vec<String> = props.iter().map(usize::to_string).collect();
        let _ = writeln!(
            out,
            "    StateLayout {{ min_state: {}, state_count: {}, default_state: {}, properties: &[{}] }},",
            b.min_state,
            b.max_state + 1 - b.min_state,
            b.default_state,
            ps.join(", ")
        );
    }
    out.push_str("];\n\n/// Block id per state.\n");
    crate::emit::num_table(&mut out, "pub(crate) static STATE_BLOCK", "u16", &state_block, 20);
    out.push_str("\n/// Packed per-state record (see `v26_1::StatePhys`).\n");
    let _ = writeln!(out, "pub(crate) static STATE_PHYS: [u64; {}] = [", packed.len());
    for chunk in packed.chunks(8) {
        out.push_str("    ");
        for (i, v) in chunk.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            let _ = write!(out, "{v:#x},");
        }
        out.push('\n');
    }
    out.push_str("];\n");
    let _ = writeln!(
        out,
        "\n/// Distinct 26.1 collision VoxelShapes (jar; index 0 = the empty shape; offset blocks list\n/// their unshifted shape).\npub(crate) static VOXEL_SHAPES: [VoxelShapeRef; {}] = [",
        shapes.len()
    );
    for s in &shapes {
        let _ = writeln!(out, "    {},", voxel_lit(s));
    }
    out.push_str("];\n");
    let _ = writeln!(
        out,
        "\n/// The full cube (`Shapes.block()`).\npub(crate) const FULL_CUBE_VOXEL: u16 = {full_cube};\n/// Scaffolding's bottom shape (the \"collision shape\" below the stable top).\npub(crate) const SCAFFOLDING_BOTTOM_VOXEL: u16 = {scaffolding_bottom};\n/// Powder snow's shape for entities falling faster than 2.5 blocks.\npub(crate) const POWDER_SNOW_FALLING_VOXEL: u16 = {powder_falling};"
    );
    let mods = "//! Generated 26.1 tables.\n//!\n//! @generated by `cargo run -p ghast-codegen-data` — do not edit.\n\n#[rustfmt::skip]\npub(crate) mod blocks;\n".to_owned();
    Ok(vec![
        ("v26_1/blocks.rs".into(), out),
        ("v26_1/mod.rs".into(), mods),
    ])
}

