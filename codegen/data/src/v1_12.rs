//! 1.12.x tables (protocol 340 and the other 1.12 releases): numeric block id + 4-bit metadata like
//! 1.8, but every collision shape and per-state flag comes straight from the vanilla 1.12.2 oracle
//! dump (`tests/golden/1.12.2/collision_boxes.jsonl.gz`): 1.12 builds its boxes from `double`
//! constants, so the block-local boxes the oracle measured alone are exactly what vanilla places.

use crate::emit::{f64l, header, lit, num_table};
use crate::facts;
use crate::generate::{self, File, Notes};
use crate::model::{self, Paths, R};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

/// Number of raw states `id << 4 | meta`.
const RAW: usize = 256 * 16;

/// Per-state flag bits (must match `ghast_data::v1_12::StateFlags`).
const FULL_CUBE: u32 = 1 << 0;
const NORMAL_CUBE: u32 = 1 << 1;
const BLOCK_NORMAL_CUBE: u32 = 1 << 2;
const TOP_SOLID: u32 = 1 << 3;
const OPAQUE_CUBE: u32 = 1 << 4;
const FULL_BLOCK: u32 = 1 << 5;
const CAUSES_SUFFOCATION: u32 = 1 << 6;

/// Face-shape names in `ghast_data::v1_12::FaceShape` order (value = index).
const FACE_SHAPES: [&str; 9] = [
    "SOLID",
    "BOWL",
    "CENTER_SMALL",
    "MIDDLE_POLE_THIN",
    "CENTER",
    "MIDDLE_POLE",
    "CENTER_BIG",
    "MIDDLE_POLE_THICK",
    "UNDEFINED",
];
/// Faces in `ghast_data::v1_12::Face` order.
const FACES: [&str; 6] = ["down", "up", "north", "south", "west", "east"];

struct Alone {
    registered: bool,
    boxes: Vec<[u64; 6]>,
    flags: u32,
    material: Option<String>,
}

struct BlockLine {
    slipperiness: u32,
    has_collision: bool,
    material: String,
    material_flags: [bool; 5],
    climbable: bool,
}

fn hex6(v: &serde_json::Value) -> R<Vec<[u64; 6]>> {
    let mut out = Vec::new();
    for b in v.as_array().ok_or("boxes")? {
        let mut a = [0u64; 6];
        for (i, c) in b.as_array().ok_or("box")?.iter().enumerate() {
            a[i] =
                u64::from_str_radix(c[1].as_str().ok_or("hex")?, 16).map_err(|e| e.to_string())?;
        }
        out.push(a);
    }
    Ok(out)
}

fn bool_of(v: &serde_json::Value, k: &str) -> R<bool> {
    v[k].as_bool()
        .ok_or_else(|| format!("oracle line without {k}: {v}"))
}

fn material_kind(name: &str) -> &'static str {
    match name {
        "air" => "Air",
        "water" => "Water",
        "lava" => "Lava",
        "ice" => "Ice",
        _ => "Other",
    }
}

/// Generates `v1_12/*`.
pub fn generate(
    p: &Paths,
    oracle_boxes: &Path,
    oracle_selection: &Path,
    oracle_entities: &Path,
    oracle_faces: &Path,
    notes: &mut Notes,
) -> R<Vec<File>> {
    let blocks = model::blocks(&p.json("blocks")?)?;
    generate::check_unique("block", blocks.iter().map(|b| b.name.as_str()))?;
    let mut slot = [255u8; 256];
    for (i, b) in blocks.iter().enumerate() {
        let id = usize::from(b.id);
        if id >= 256 || slot[id] != 255 {
            return Err(format!("1.12 block id {} out of range or duplicate", b.id));
        }
        slot[id] = u8::try_from(i).map_err(|_| "more than 255 blocks")?;
    }
    let mut items = model::items(&p.json("items")?)?;
    generate::apply_item_fixes(&mut items, facts::V1_12_ITEM_FIXES, &p.version, notes)?;
    generate::check_harvest_tools(&blocks, &items)?;
    let (phys, deps) = generate::physics(&facts::V1_12, &blocks)?;

    // The oracle collision dump.
    let mut block_lines: BTreeMap<usize, BlockLine> = BTreeMap::new();
    let mut alone: Vec<Option<Alone>> = (0..RAW).map(|_| None).collect();
    for v in generate::oracle_lines(oracle_boxes)? {
        let id = v["id"].as_u64().ok_or("oracle line without id")? as usize;
        match v["type"].as_str() {
            Some("block") => {
                let i = slot.get(id).copied().filter(|&s| s != 255).ok_or_else(|| {
                    format!("oracle block {id} is not a minecraft-data 1.12 block")
                })?;
                let b = &blocks[usize::from(i)];
                let vanilla = v["name"].as_str().unwrap_or_default();
                let alias = facts::V1_12_NAME_ALIASES
                    .iter()
                    .any(|(ours, van)| *ours == b.name && vanilla == format!("minecraft:{van}"));
                if !alias && vanilla != format!("minecraft:{}", b.name) {
                    return Err(format!(
                        "oracle block {id} is {}, table {}",
                        v["name"], b.name
                    ));
                }
                let slip = u32::from_str_radix(
                    v["slipperiness"][1].as_str().ok_or("slipperiness hex")?,
                    16,
                )
                .map_err(|e| e.to_string())?;
                block_lines.insert(
                    id,
                    BlockLine {
                        slipperiness: slip,
                        has_collision: bool_of(&v, "has_collision")?,
                        material: v["material"].as_str().ok_or("material")?.to_owned(),
                        material_flags: [
                            bool_of(&v, "material_blocks_movement")?,
                            bool_of(&v, "material_solid")?,
                            bool_of(&v, "material_fluid")?,
                            bool_of(&v, "material_opaque")?,
                            bool_of(&v, "material_replaceable")?,
                        ],
                        climbable: bool_of(&v, "climbable")?,
                    },
                );
            }
            Some("boxes") if v["config"] == "alone" => {
                let meta = v["meta"].as_u64().ok_or("meta")? as usize;
                if id >= 256 || meta > 15 {
                    return Err(format!("oracle state {id}:{meta} out of range"));
                }
                if v["exact"].as_bool() != Some(true) {
                    return Err(format!("oracle {id}:{meta} alone is not exact"));
                }
                let registered = !v["state"].is_null();
                let mut flags = 0;
                if registered {
                    for (k, bit) in [
                        ("full_cube", FULL_CUBE),
                        ("normal_cube", NORMAL_CUBE),
                        ("block_normal_cube", BLOCK_NORMAL_CUBE),
                        ("top_solid", TOP_SOLID),
                        ("opaque_cube", OPAQUE_CUBE),
                        ("full_block", FULL_BLOCK),
                        ("causes_suffocation", CAUSES_SUFFOCATION),
                    ] {
                        if bool_of(&v, k)? {
                            flags |= bit;
                        }
                    }
                }
                alone[id * 16 + meta] = Some(Alone {
                    registered,
                    boxes: hex6(&v["boxes"])?,
                    flags,
                    material: v["material"].as_str().map(str::to_owned),
                });
            }
            _ => {}
        }
    }
    for (i, b) in blocks.iter().enumerate() {
        let id = usize::from(b.id);
        let l = block_lines
            .get(&id)
            .ok_or_else(|| format!("oracle has no block line for {}", b.name))?;
        if l.slipperiness != phys[i].slipperiness.to_bits() {
            return Err(format!(
                "{}: facts slipperiness {} but the oracle has {}",
                b.name,
                phys[i].slipperiness,
                f32::from_bits(l.slipperiness)
            ));
        }
        if l.climbable != (phys[i].flags & facts::CLIMBABLE != 0) {
            return Err(format!("{}: climbable disagrees with the oracle", b.name));
        }
        let liquid = phys[i].flags & (facts::WATER | facts::LAVA) != 0;
        if liquid != l.material_flags[2] {
            return Err(format!(
                "{}: liquid flags disagree with the oracle material",
                b.name
            ));
        }
        for m in 0..16 {
            let a = alone[id * 16 + m]
                .as_ref()
                .ok_or_else(|| format!("oracle has no alone line for {}:{m}", b.name))?;
            if a.registered && a.material.as_deref() != Some(l.material.as_str()) {
                return Err(format!("{}:{m}: per-state material differs", b.name));
            }
        }
    }
    // Ids without a block (253, 254) read as air: no boxes, unregistered.
    for (id, s) in slot.iter().enumerate() {
        if *s == 255 {
            for m in 0..16 {
                alone[id * 16 + m] = Some(Alone {
                    registered: false,
                    boxes: Vec::new(),
                    flags: 0,
                    material: None,
                });
            }
        }
    }
    let alone: Vec<Alone> = alone
        .into_iter()
        .enumerate()
        .map(|(raw, a)| a.ok_or_else(|| format!("oracle has no alone line for raw {raw}")))
        .collect::<R<_>>()?;

    // Shapes: distinct box lists, index 0 = empty; unregistered states have none.
    let mut shapes: Vec<Vec<[u64; 6]>> = vec![Vec::new()];
    let mut state_shape = vec![0u32; RAW];
    for (raw, a) in alone.iter().enumerate() {
        if !a.registered || a.boxes.is_empty() {
            continue;
        }
        let i = shapes
            .iter()
            .position(|s| *s == a.boxes)
            .unwrap_or_else(|| {
                shapes.push(a.boxes.clone());
                shapes.len() - 1
            });
        state_shape[raw] = i as u32;
    }
    let registered: Vec<u32> = (0..256)
        .map(|id| {
            (0..16)
                .filter(|m| alone[id * 16 + m].registered)
                .fold(0u32, |acc, m| acc | (1 << m))
        })
        .collect();

    // Materials.
    let mut materials: BTreeMap<String, [bool; 5]> = BTreeMap::new();
    for l in block_lines.values() {
        if let Some(f) = materials.insert(l.material.clone(), l.material_flags)
            && f != l.material_flags
        {
            return Err(format!("material {} has differing flags", l.material));
        }
    }
    let material_names: Vec<&String> = materials.keys().collect();
    let air_material = material_names
        .iter()
        .position(|n| n.as_str() == "air")
        .ok_or("no air material")?;

    let src = |k: &str| -> R<String> { Ok(format!("{}/{k}.json", p.dir(k)?)) };
    let mut out = header(
        "1.12 block table: per-block info, per-state (`id << 4 | meta`) collision shapes and flags.",
        &[
            src("blocks")?,
            "codegen/data/src/facts.rs".into(),
            facts::V1_12_ORACLE_BOXES.into(),
            facts::V1_12_ORACLE_SELECTION.into(),
            facts::V1_12_ORACLE_FACES.into(),
        ],
    );
    out.push_str("use crate::common::*;\nuse crate::v1_12::{Material, MaterialKind};\n\n");
    generate::emit_blocks_common(&mut out, "v1_12", &blocks, &phys)?;
    out.push_str("\n/// Index into `BLOCKS` per block id (255 = no block with this id).\n");
    let slots: Vec<u32> = slot.iter().map(|&s| u32::from(s)).collect();
    num_table(&mut out, "pub(crate) static BLOCK_SLOT", "u8", &slots, 32);

    out.push_str(
        "\n/// Distinct collision shapes (oracle `alone` boxes, block-local; index 0 = empty).\n",
    );
    let spans: Vec<Vec<[f64; 6]>> = shapes
        .iter()
        .map(|s| s.iter().map(|b| b.map(f64::from_bits)).collect())
        .collect();
    generate::emit_shape_slices(&mut out, &spans, "");
    out.push('\n');
    let mut deps_by_id = vec![None; 256];
    for (b, d) in blocks.iter().zip(&deps) {
        deps_by_id[usize::from(b.id)] = *d;
    }
    generate::emit_dependencies(&mut out, &deps_by_id);
    out.push_str("\n/// Shape index per raw state `id << 4 | meta`.\n");
    num_table(
        &mut out,
        "pub(crate) static STATE_SHAPE",
        "u16",
        &state_shape,
        16,
    );
    out.push_str("\n/// Per raw state (oracle): bit 0 full cube, 1 normal cube, 2 block-normal cube, 3 top solid,\n/// 4 opaque cube, 5 full block, 6 causes suffocation.\n");
    let flags: Vec<u32> = alone.iter().map(|a| a.flags).collect();
    num_table(&mut out, "pub(crate) static STATE_FLAGS", "u8", &flags, 16);
    out.push_str(
        "\n/// Bit `m` set = meta `m` is a registered state of the block (1.12.2 oracle).\n",
    );
    num_table(
        &mut out,
        "pub(crate) static REGISTERED_METAS",
        "u16",
        &registered,
        16,
    );

    let _ = writeln!(
        out,
        "\n/// Every 1.12.2 block material (oracle; names are ours, from the Yarn field names).\npub(crate) static MATERIALS: [Material; {}] = [",
        material_names.len()
    );
    for n in &material_names {
        let f = materials[*n];
        let _ = writeln!(
            out,
            "    Material {{ name: {}, kind: MaterialKind::{}, blocks_movement: {}, solid: {}, liquid: {}, opaque: {}, replaceable: {} }},",
            lit(n),
            material_kind(n),
            f[0],
            f[1],
            f[2],
            f[3],
            f[4]
        );
    }
    out.push_str("];\n\n/// Material index per block id.\n");
    let block_material: Vec<u32> = (0..256)
        .map(|id| {
            block_lines.get(&id).map_or(air_material as u32, |l| {
                material_names
                    .iter()
                    .position(|n| **n == l.material)
                    .unwrap_or(air_material) as u32
            })
        })
        .collect();
    num_table(
        &mut out,
        "pub(crate) static BLOCK_MATERIAL",
        "u8",
        &block_material,
        32,
    );

    // Faces (optional dump: absent faces read as UNDEFINED / not excluded).
    let (state_faces, block_bits) = faces(oracle_faces, &alone, &block_lines, notes)?;
    out.push_str("\n/// Per block id: bit 0 `Block.isCollidable` (ray tests), 1 excluded from fence connections,\n/// 2 excluded from wall connections, 3 excluded from pane connections.\n");
    num_table(
        &mut out,
        "pub(crate) static BLOCK_BITS",
        "u8",
        &block_bits,
        32,
    );
    out.push_str("\n/// Per raw state: face shape per face (4 bits each: down, up, north, south, west, east), the\n/// block alone (air neighbours).\n");
    num_table(
        &mut out,
        "pub(crate) static STATE_FACES",
        "u32",
        &state_faces,
        8,
    );

    selection(&mut out, oracle_selection, &alone)?;

    let mut files = vec![
        ("blocks.rs".to_string(), out),
        (
            "items.rs".to_string(),
            generate::items_file("v1_12", &src("items")?, &items)?,
        ),
    ];
    files.extend(generate::misc_files(
        p,
        &items,
        (oracle_entities, facts::V1_12_ENTITY_SIZE_FIXES),
        notes,
    )?);
    files.push((
        "dig.rs".to_string(),
        crate::dig_tables::generate(
            "Minecraft 1.12.2 dig tables (oracle).",
            &oracle_selection.with_file_name("dig_facts.jsonl.gz"),
            "tests/golden/1.12.2/dig_facts.jsonl.gz",
            "strength",
            "no_tool_needed",
        )?,
    ));
    let mut mods = generate::MODS.to_vec();
    mods.push("dig");
    mods.sort_unstable();
    files.push((
        "mod.rs".to_string(),
        generate::mod_rs("Generated 1.12 tables.", &mods),
    ));
    Ok(files
        .into_iter()
        .map(|(n, c)| (format!("v1_12/{n}"), c))
        .collect())
}

fn faces(
    path: &Path,
    alone: &[Alone],
    block_lines: &BTreeMap<usize, BlockLine>,
    notes: &mut Notes,
) -> R<(Vec<u32>, Vec<u32>)> {
    let undefined = (FACE_SHAPES.len() - 1) as u32;
    let all_undefined = (0..6).fold(0u32, |acc, f| acc | (undefined << (4 * f)));
    let mut state_faces = vec![all_undefined; RAW];
    let mut bits: Vec<u32> = (0..256)
        .map(|id| u32::from(block_lines.get(&id).is_some_and(|l| l.has_collision)))
        .collect();
    if !path.exists() {
        notes.push(format!(
            "1.12: {} missing, face shapes default to UNDEFINED",
            path.display()
        ));
        return Ok((state_faces, bits));
    }
    let mut seen = BTreeSet::new();
    for v in generate::oracle_lines(path)? {
        let id = v["id"].as_u64().ok_or("faces id")? as usize;
        match v["type"].as_str() {
            Some("faces") if v.get("config").is_none_or(|c| c == "alone") => {
                let meta = v["meta"].as_u64().ok_or("faces meta")? as usize;
                let raw = id * 16 + meta;
                if raw >= RAW {
                    return Err(format!("faces {id}:{meta} out of range"));
                }
                if !alone[raw].registered {
                    continue; // reads as air
                }
                let mut packed = 0u32;
                for (fi, f) in FACES.iter().enumerate() {
                    let name = v["faces"][*f]
                        .as_str()
                        .ok_or_else(|| format!("faces {id}:{meta}: no {f}"))?;
                    let k = FACE_SHAPES
                        .iter()
                        .position(|n| *n == name)
                        .ok_or_else(|| format!("unknown face shape {name}"))?;
                    packed |= (k as u32) << (4 * fi);
                }
                state_faces[raw] = packed;
                seen.insert(raw);
            }
            Some("faces_block") if id < 256 => {
                for (k, bit) in [
                    ("fence_excluded", 2),
                    ("wall_excluded", 4),
                    ("pane_excluded", 8),
                ] {
                    if bool_of(&v, k)? {
                        bits[id] |= bit;
                    }
                }
            }
            _ => {}
        }
    }
    // Air reads as UNDEFINED anyway; every registered state must be covered.
    let missing = alone
        .iter()
        .enumerate()
        .filter(|(raw, a)| a.registered && !seen.contains(raw))
        .count();
    if missing != 0 {
        return Err(format!("face dump misses {missing} registered states"));
    }
    Ok((state_faces, bits))
}

fn selection(out: &mut String, path: &Path, alone: &[Alone]) -> R<()> {
    let mut boxes: Vec<[u64; 6]> = Vec::new();
    let mut idx = vec![u32::MAX; RAW];
    let mut flags = vec![0u32; RAW];
    for v in generate::oracle_lines(path)? {
        if v["type"] != "selection" || v["config"] != "alone" {
            continue;
        }
        let id = v["id"].as_u64().ok_or("id")? as usize;
        let meta = v["meta"].as_u64().ok_or("meta")? as usize;
        if id >= 256 || meta > 15 {
            return Err(format!("selection {id}:{meta} out of range"));
        }
        let mut b = [0u64; 6];
        for (i, c) in v["bounds"].as_array().ok_or("bounds")?.iter().enumerate() {
            b[i] =
                u64::from_str_radix(c[1].as_str().ok_or("hex")?, 16).map_err(|e| e.to_string())?;
        }
        let i = boxes.iter().position(|x| *x == b).unwrap_or_else(|| {
            boxes.push(b);
            boxes.len() - 1
        });
        let raw = id * 16 + meta;
        idx[raw] = i as u32;
        let bit = |k: &str, m: u32| -> R<u32> { Ok(if bool_of(&v, k)? { m } else { 0 }) };
        flags[raw] =
            bit("can_collide", 1)? | bit("can_collide_liquid", 2)? | bit("has_collision_box", 4)?;
    }
    // Missing ids (no block) read as air.
    let air = idx[0];
    for (raw, i) in idx.iter_mut().enumerate() {
        if *i == u32::MAX {
            if alone[raw].registered {
                return Err(format!("selection dump misses raw {raw}"));
            }
            *i = air;
        }
    }
    let _ = writeln!(
        out,
        "\n/// Distinct 1.12.2 selection (ray-test) bounds, block-local (oracle).\npub(crate) static SELECTION_BOUNDS: [Aabb; {}] = [",
        boxes.len()
    );
    for b in &boxes {
        let f = b.map(f64::from_bits);
        let _ = writeln!(
            out,
            "    Aabb::new({}, {}, {}, {}, {}, {}),",
            f64l(f[0]),
            f64l(f[1]),
            f64l(f[2]),
            f64l(f[3]),
            f64l(f[4]),
            f64l(f[5])
        );
    }
    out.push_str("];\n\n/// Selection bounds index per raw state.\n");
    num_table(out, "pub(crate) static STATE_SELECTION", "u16", &idx, 16);
    out.push_str("\n/// Per raw state: bit 0 canCollide(state, false), 1 canCollide(state, true), 2 has a collision box.\n");
    num_table(
        out,
        "pub(crate) static STATE_SELECTION_FLAGS",
        "u8",
        &flags,
        16,
    );
    Ok(())
}
