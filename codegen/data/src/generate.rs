//! Table generation for each supported version.

use crate::emit::{camel_ident, f32l, f64l, header, lit, name_index, num_table, upper_ident};
use crate::facts::{self, Dep, Facts};
use crate::model::{self, Block, Paths, R};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// One generated file: path relative to `crates/ghast-data/src/generated/`, contents.
pub type File = (String, String);

/// Notes printed at the end of a run (data quirks found while generating).
pub type Notes = Vec<String>;

fn dig_material(m: &str) -> R<&'static str> {
    Ok(match m {
        "rock" => "Rock",
        "wood" => "Wood",
        "plant" => "Plant",
        "melon" => "Melon",
        "leaves" => "Leaves",
        "dirt" => "Dirt",
        "web" => "Web",
        "wool" => "Wool",
        "default" => "Default",
        _ => return Err(format!("unknown dig material {m}")),
    })
}

pub(crate) struct Physics {
    pub(crate) slipperiness: f32,
    pub(crate) speed: f64,
    pub(crate) jump: f32,
    pub(crate) stuck: [f64; 3],
    pub(crate) flags: u16,
}

/// Applies `facts` to the block list; every pattern must match something.
pub(crate) fn physics(facts: &Facts, blocks: &[Block]) -> R<(Vec<Physics>, Vec<Option<Dep>>)> {
    let mut phys: Vec<Physics> = blocks
        .iter()
        .map(|_| Physics {
            slipperiness: 0.6,
            speed: 1.0,
            jump: 1.0,
            stuck: [1.0; 3],
            flags: 0,
        })
        .collect();
    let mut deps: Vec<Option<Dep>> = vec![None; blocks.len()];
    let each = |pat: &str, f: &mut dyn FnMut(usize)| -> R<()> {
        let mut hit = false;
        for (i, b) in blocks.iter().enumerate() {
            if facts::matches(pat, &b.name) {
                hit = true;
                f(i);
            }
        }
        if hit {
            Ok(())
        } else {
            Err(format!("facts pattern {pat:?} matches no block"))
        }
    };
    for &(p, v) in facts.slipperiness {
        each(p, &mut |i| phys[i].slipperiness = v)?;
    }
    for &(p, v) in facts.speed_factor {
        each(p, &mut |i| phys[i].speed = v)?;
    }
    for &(p, v) in facts.jump_factor {
        each(p, &mut |i| phys[i].jump = v)?;
    }
    for &(p, v) in facts.stuck {
        each(p, &mut |i| phys[i].stuck = v)?;
    }
    for &(p, v) in facts.flags {
        each(p, &mut |i| phys[i].flags |= v)?;
    }
    for &(p, d) in facts.dependency {
        each(p, &mut |i| {
            if !facts
                .dependency_exclude
                .iter()
                .any(|x| facts::matches(x, &blocks[i].name))
            {
                deps[i] = Some(d);
            }
        })?;
    }
    Ok((phys, deps))
}

fn physics_lit(p: &Physics) -> String {
    if p.slipperiness == 0.6
        && p.speed == 1.0
        && p.jump == 1.0
        && p.stuck == [1.0; 3]
        && p.flags == 0
    {
        return "BlockPhysics::DEFAULT".into();
    }
    format!(
        "BlockPhysics {{ slipperiness: {}, speed_factor: {}, jump_factor: {}, stuck_multiplier: [{}, {}, {}], flags: PhysicsFlags({:#06x}) }}",
        f32l(p.slipperiness),
        f64l(p.speed),
        f32l(p.jump),
        f64l(p.stuck[0]),
        f64l(p.stuck[1]),
        f64l(p.stuck[2]),
        p.flags
    )
}

fn block_info_lit(b: &Block, p: &Physics) -> R<String> {
    let material = match &b.material {
        Some(m) => format!("Some(DigMaterial::{})", dig_material(m)?),
        None => "None".into(),
    };
    let drops: Vec<String> = b
        .drops
        .iter()
        .map(|d| {
            format!(
                "Drop {{ item: {}, meta: {}, min: {}, max: {} }}",
                d.item,
                d.meta.map_or("None".into(), |m| format!("Some({m})")),
                d.min,
                d.max
            )
        })
        .collect();
    let tools: Vec<String> = b.harvest_tools.iter().map(u16::to_string).collect();
    Ok(format!(
        "BlockInfo {{ id: {}, name: {}, #[cfg(feature = \"display-names\")] display_name: {}, hardness: {}, resistance: {}, stack_size: {}, diggable: {}, transparent: {}, emit_light: {}, filter_light: {}, bounding_box: BoundingBox::{}, material: {material}, harvest_tools: &[{}], drops: &[{}], physics: {} }}",
        b.id,
        lit(&b.name),
        lit(&b.display_name),
        b.hardness
            .map_or("None".into(), |h| format!("Some({})", f32l(h as f32))),
        f32l(b.resistance as f32),
        b.stack_size,
        b.diggable,
        b.transparent,
        b.emit_light,
        b.filter_light,
        if b.bbox_empty { "Empty" } else { "Block" },
        tools.join(", "),
        drops.join(", "),
        physics_lit(p)
    ))
}

fn check_dense(blocks: &[Block]) -> R<()> {
    for (i, b) in blocks.iter().enumerate() {
        if usize::from(b.id) != i {
            return Err(format!("block ids not dense: index {i} has id {}", b.id));
        }
    }
    Ok(())
}

pub(crate) fn check_unique<'a>(what: &str, names: impl Iterator<Item = &'a str>) -> R<()> {
    let mut seen = BTreeSet::new();
    for n in names {
        if !seen.insert(upper_ident(n)) {
            return Err(format!("duplicate {what} name/ident {n}"));
        }
    }
    Ok(())
}

/// Dense re-indexing of the source shape ids actually used; shape 0 must be the empty shape.
pub(crate) struct ShapeTable {
    pub(crate) remap: BTreeMap<u32, u32>,
    pub(crate) shapes: Vec<Vec<[f64; 6]>>,
}

fn shape_table(src: &model::Shapes, used: &BTreeSet<u32>) -> R<ShapeTable> {
    let mut remap = BTreeMap::new();
    let mut shapes = Vec::new();
    // Keep the empty shape at index 0.
    let mut order: Vec<u32> = used.iter().copied().collect();
    order.sort_by_key(|id| (src.shapes.get(id).is_none_or(|s| !s.is_empty()), *id));
    for id in order {
        let s = src
            .shapes
            .get(&id)
            .ok_or_else(|| format!("shape {id} referenced but not defined"))?;
        remap.insert(id, shapes.len() as u32);
        shapes.push(s.clone());
    }
    if shapes.first().is_none_or(|s| !s.is_empty()) {
        return Err("no empty shape".into());
    }
    Ok(ShapeTable { remap, shapes })
}

pub(crate) fn emit_shapes(out: &mut String, t: &ShapeTable) {
    let _ = writeln!(
        out,
        "pub(crate) static SHAPES: [&[Aabb]; {}] = [",
        t.shapes.len()
    );
    for s in &t.shapes {
        let boxes: Vec<String> = s
            .iter()
            .map(|b| {
                format!(
                    "Aabb::new({}, {}, {}, {}, {}, {})",
                    f64l(b[0]),
                    f64l(b[1]),
                    f64l(b[2]),
                    f64l(b[3]),
                    f64l(b[4]),
                    f64l(b[5])
                )
            })
            .collect();
        let _ = writeln!(out, "    &[{}],", boxes.join(", "));
    }
    out.push_str("];\n");
}

pub(crate) fn emit_dependencies(out: &mut String, deps: &[Option<Dep>]) {
    let _ = writeln!(
        out,
        "pub(crate) static BLOCK_SHAPE_DEPENDENCY: [ShapeDependency; {}] = [",
        deps.len()
    );
    for d in deps {
        let _ = writeln!(
            out,
            "    {},",
            d.map_or("ShapeDependency::None".into(), Dep::rust)
        );
    }
    out.push_str("];\n");
}

pub(crate) fn emit_blocks_common(
    out: &mut String,
    module: &str,
    blocks: &[Block],
    phys: &[Physics],
) -> R<()> {
    let _ = writeln!(
        out,
        "pub(crate) static BLOCKS: [BlockInfo; {}] = [",
        blocks.len()
    );
    for (b, p) in blocks.iter().zip(phys) {
        let _ = writeln!(out, "    {},", block_info_lit(b, p)?);
    }
    out.push_str("];\n\n");
    let id_ty = if module == "v1_16_5" { "u16" } else { "u8" };
    name_index(
        out,
        "pub(crate) static BLOCK_NAMES",
        id_ty,
        blocks
            .iter()
            .map(|b| (b.name.clone(), u32::from(b.id)))
            .collect(),
    );
    let _ = writeln!(
        out,
        "\n/// Block constants.\nimpl crate::{module}::Block {{"
    );
    for b in blocks {
        let _ = writeln!(
            out,
            "    /// `{}` ({}).\n    pub const {}: Self = Self({});",
            b.name,
            b.display_name.replace('`', "'"),
            upper_ident(&b.name),
            b.id
        );
    }
    out.push_str("}\n");
    Ok(())
}

pub(crate) fn items_file(module: &str, src: &str, items: &[model::Item]) -> R<String> {
    check_unique("item", items.iter().map(|i| i.name.as_str()))?;
    let mut out = header("Item table.", &[src.into()]);
    out.push_str("use crate::common::ItemInfo;\n\n");
    let _ = writeln!(
        out,
        "pub(crate) static ITEMS: [ItemInfo; {}] = [",
        items.len()
    );
    for i in items {
        let vars: Vec<String> = i
            .variations
            .iter()
            .map(|(m, n)| format!("({m}, {})", lit(n)))
            .collect();
        let _ = writeln!(
            out,
            "    ItemInfo {{ id: {}, name: {}, #[cfg(feature = \"display-names\")] display_name: {}, stack_size: {}, max_durability: {}, #[cfg(feature = \"display-names\")] variations: &[{}] }},",
            i.id,
            lit(&i.name),
            lit(&i.display_name),
            i.stack_size,
            i.max_durability
                .map_or("None".into(), |d| format!("Some({d})")),
            vars.join(", ")
        );
    }
    out.push_str("];\n\n");
    name_index(
        &mut out,
        "pub(crate) static ITEM_NAMES",
        "u16",
        items
            .iter()
            .map(|i| (i.name.clone(), u32::from(i.id)))
            .collect(),
    );
    let _ = writeln!(out, "\n/// Item constants.\nimpl crate::{module}::Item {{");
    for i in items {
        let _ = writeln!(
            out,
            "    /// `{}` ({}).\n    pub const {}: Self = Self({});",
            i.name,
            i.display_name.replace('`', "'"),
            upper_ident(&i.name),
            i.id
        );
    }
    out.push_str("}\n");
    Ok(out)
}

fn opt_f32(x: Option<f64>) -> String {
    x.map_or("None".into(), |v| format!("Some({})", f32l(v as f32)))
}

pub(crate) fn misc_files(
    p: &Paths,
    items: &[model::Item],
    (oracle_entities, size_fixes): (&std::path::Path, &[(&str, f32, f32)]),
    notes: &mut Notes,
) -> R<Vec<File>> {
    let mut files = Vec::new();

    // Entities.
    let mut ents = model::entities(&p.json("entities")?)?;
    let sizes = entity_sizes(oracle_entities, &p.version)?;
    apply_entity_size_fixes(&mut ents, &sizes, size_fixes, &p.version, notes)?;
    let mut out = header(
        "Entity types.",
        &[format!("{}/entities.json", p.dir("entities")?)],
    );
    out.push_str(
        "use crate::common::{\n    EntityDimensions, EntityInfo, EntityKind, EntityVariant, VanillaEntitySize, VariantApplied,\n};\n\n",
    );
    let _ = writeln!(
        out,
        "/// Sorted by (kind, id).\npub(crate) static ENTITIES: [EntityInfo; {}] = [",
        ents.len()
    );
    for e in &ents {
        let _ = writeln!(
            out,
            "    EntityInfo {{ id: {}, name: {}, #[cfg(feature = \"display-names\")] display_name: {}, kind: EntityKind::{}, category: {}, width: {}, height: {} }},",
            e.id,
            lit(&e.name),
            lit(&e.display_name),
            e.kind,
            lit(&e.category),
            opt_f32(e.width),
            opt_f32(e.height)
        );
    }
    out.push_str(
        "];\n\n/// Name -> index into `ENTITIES` (first entry wins for duplicate names).\n",
    );
    name_index(
        &mut out,
        "pub(crate) static ENTITY_NAMES",
        "u16",
        ents.iter()
            .enumerate()
            .map(|(i, e)| (e.name.clone(), i as u32))
            .collect(),
    );
    emit_entity_sizes(&mut out, &sizes)?;
    files.push(("entities.rs".to_string(), out));

    // Biomes.
    let biomes = model::biomes(&p.json("biomes")?)?;
    check_unique("biome", biomes.iter().map(|b| b.name.as_str()))?;
    let mut out = header("Biomes.", &[format!("{}/biomes.json", p.dir("biomes")?)]);
    out.push_str("use crate::common::BiomeInfo;\n\n");
    let _ = writeln!(
        out,
        "pub(crate) static BIOMES: [BiomeInfo; {}] = [",
        biomes.len()
    );
    for b in &biomes {
        let _ = writeln!(
            out,
            "    BiomeInfo {{ id: {}, name: {}, #[cfg(feature = \"display-names\")] display_name: {}, category: {}, temperature: {}, rainfall: {}, precipitation: {}, dimension: {} }},",
            b.id,
            lit(&b.name),
            lit(&b.display_name),
            lit(&b.category),
            f32l(b.temperature as f32),
            f32l(b.rainfall as f32),
            lit(&b.precipitation),
            lit(&b.dimension)
        );
    }
    out.push_str("];\n\n");
    name_index(
        &mut out,
        "pub(crate) static BIOME_NAMES",
        "u16",
        biomes
            .iter()
            .map(|b| (b.name.clone(), u32::from(b.id)))
            .collect(),
    );
    files.push(("biomes.rs".to_string(), out));

    // Enchantments.
    let ench = model::enchantments(&p.json("enchantments")?)?;
    let mut out = header(
        "Enchantments.",
        &[format!("{}/enchantments.json", p.dir("enchantments")?)],
    );
    out.push_str("use crate::common::{EnchantCost, EnchantmentInfo};\n\n");
    let _ = writeln!(
        out,
        "pub(crate) static ENCHANTMENTS: [EnchantmentInfo; {}] = [",
        ench.len()
    );
    for e in &ench {
        let ex: Vec<String> = e.exclude.iter().map(|x| lit(x)).collect();
        let _ = writeln!(
            out,
            "    EnchantmentInfo {{ id: {}, name: {}, #[cfg(feature = \"display-names\")] display_name: {}, max_level: {}, min_cost: EnchantCost {{ a: {}, b: {} }}, max_cost: EnchantCost {{ a: {}, b: {} }}, exclude: &[{}], category: {}, weight: {}, treasure_only: {}, curse: {} }},",
            e.id,
            lit(&e.name),
            lit(&e.display_name),
            e.max_level,
            e.min_cost.0,
            e.min_cost.1,
            e.max_cost.0,
            e.max_cost.1,
            ex.join(", "),
            lit(&e.category),
            e.weight,
            e.treasure_only,
            e.curse
        );
    }
    out.push_str("];\n\n");
    name_index(
        &mut out,
        "pub(crate) static ENCHANTMENT_NAMES",
        "u16",
        ench.iter()
            .map(|e| (e.name.clone(), u32::from(e.id)))
            .collect(),
    );
    files.push(("enchantments.rs".to_string(), out));

    // Foods, resolved by name against this version's items (source ids may come from another
    // minecraft-data version directory).
    let foods = model::foods(&p.json("foods")?)?;
    let by_name: BTreeMap<&str, u16> = items.iter().map(|i| (i.name.as_str(), i.id)).collect();
    let mut rows = Vec::new();
    let mut remapped = Vec::new();
    let mut skipped = Vec::new();
    for f in &foods {
        let Some(&id) = by_name.get(f.name.as_str()) else {
            skipped.push(format!("{} ({})", f.name, f.source_id));
            continue;
        };
        if id != f.source_id {
            remapped.push(format!("{} {}->{}", f.name, f.source_id, id));
        }
        rows.push((id, f));
    }
    if !skipped.is_empty() {
        notes.push(format!(
            "{}: foods.json entries that are not items (skipped): {}",
            p.version,
            skipped.join(", ")
        ));
    }
    rows.sort_by_key(|r| r.0);
    rows.dedup_by_key(|r| r.0);
    if !remapped.is_empty() {
        notes.push(format!(
            "{}: foods.json ({}) item ids differ from items.json ({}); resolved by name: {}",
            p.version,
            p.dir("foods")?,
            p.dir("items")?,
            remapped.join(", ")
        ));
    }
    let mut out = header(
        "Foods (item ids resolved by name against this version's items).",
        &[format!("{}/foods.json", p.dir("foods")?)],
    );
    out.push_str("use crate::common::FoodInfo;\n\n");
    let _ = writeln!(
        out,
        "/// Sorted by item id.\npub(crate) static FOODS: [FoodInfo; {}] = [",
        rows.len()
    );
    for (id, f) in &rows {
        let _ = writeln!(
            out,
            "    FoodInfo {{ item: {id}, name: {}, food_points: {}, saturation: {} }},",
            lit(&f.name),
            f.food_points,
            f32l(f.saturation as f32)
        );
    }
    out.push_str("];\n");
    files.push(("foods.rs".to_string(), out));

    // Effects.
    let eff = model::effects(&p.json("effects")?)?;
    let mut out = header(
        "Status effects.",
        &[format!("{}/effects.json", p.dir("effects")?)],
    );
    out.push_str("use crate::common::EffectInfo;\n\n");
    let _ = writeln!(
        out,
        "pub(crate) static EFFECTS: [EffectInfo; {}] = [",
        eff.len()
    );
    for e in &eff {
        let _ = writeln!(
            out,
            "    EffectInfo {{ id: {}, name: {}, #[cfg(feature = \"display-names\")] display_name: {}, good: {} }},",
            e.id,
            lit(&e.name),
            lit(&e.display_name),
            e.good
        );
    }
    out.push_str("];\n\n");
    name_index(
        &mut out,
        "pub(crate) static EFFECT_NAMES",
        "u8",
        eff.iter()
            .map(|e| (e.name.clone(), u32::from(e.id)))
            .collect(),
    );
    files.push(("effects.rs".to_string(), out));

    // Tool multipliers.
    let mats = model::materials(&p.json("materials")?)?;
    let mut out = header(
        "Tool speed multipliers per dig material.",
        &[format!("{}/materials.json", p.dir("materials")?)],
    );
    out.push_str("use crate::common::DigMaterial;\n\n");
    let _ = writeln!(
        out,
        "pub(crate) static TOOL_MULTIPLIERS: [(DigMaterial, &[(u16, f32)]); {}] = [",
        mats.len()
    );
    for (m, tools) in &mats {
        for (id, _) in tools {
            if !by_name.values().any(|x| x == id) {
                return Err(format!("materials {m}: tool {id} is not an item"));
            }
        }
        let t: Vec<String> = tools
            .iter()
            .map(|(id, x)| format!("({id}, {})", f32l(*x as f32)))
            .collect();
        let _ = writeln!(
            out,
            "    (DigMaterial::{}, &[{}]),",
            dig_material(m)?,
            t.join(", ")
        );
    }
    out.push_str("];\n");
    files.push(("materials.rs".to_string(), out));
    Ok(files)
}

pub(crate) fn check_harvest_tools(blocks: &[Block], items: &[model::Item]) -> R<()> {
    let ids: BTreeSet<u16> = items.iter().map(|i| i.id).collect();
    for b in blocks {
        for t in &b.harvest_tools {
            if !ids.contains(t) {
                return Err(format!("block {}: harvest tool {t} is not an item", b.name));
            }
        }
        for d in &b.drops {
            if !ids.contains(&d.item) && !blocks.iter().any(|x| x.id == d.item) {
                return Err(format!("block {}: drop {} is not an item", b.name, d.item));
            }
        }
    }
    Ok(())
}

pub(crate) fn mod_rs(what: &str, mods: &[&str]) -> String {
    let mut s = format!(
        "//! {what}\n//!\n//! @generated by `cargo run -p ghast-codegen-data` — do not edit.\n\n"
    );
    for m in mods {
        let _ = writeln!(s, "#[rustfmt::skip]\npub(crate) mod {m};");
    }
    s
}

pub(crate) const MODS: &[&str] = &[
    "biomes",
    "blocks",
    "effects",
    "enchantments",
    "entities",
    "foods",
    "items",
    "materials",
];

/// 1.8.x (protocol 47): numeric block id + 4-bit metadata.
pub fn v1_8(
    p: &Paths,
    oracle_boxes: &std::path::Path,
    oracle_selection: &std::path::Path,
    oracle_recipes: &std::path::Path,
    oracle_entities: &std::path::Path,
    notes: &mut Notes,
) -> R<Vec<File>> {
    let blocks = model::blocks(&p.json("blocks")?)?;
    check_dense(&blocks)?;
    check_unique("block", blocks.iter().map(|b| b.name.as_str()))?;
    if blocks.len() > 256 {
        return Err("1.8 has more than 256 blocks".into());
    }
    let mut items = model::items(&p.json("items")?)?;
    apply_item_fixes(&mut items, facts::V1_8_ITEM_FIXES, &p.version, notes)?;
    check_harvest_tools(&blocks, &items)?;
    let (phys, deps) = physics(&facts::V1_8, &blocks)?;

    let mut shapes = model::shapes(&p.json("blockCollisionShapes")?)?;
    apply_overrides(&mut shapes, &blocks, notes)?;
    let registered = registered_metas(oracle_boxes, blocks.len())?;
    let mut per_meta: Vec<[u32; 16]> = Vec::new();
    for b in &blocks {
        let ids = shapes
            .blocks
            .get(&b.name)
            .ok_or_else(|| format!("no collision shape for {}", b.name))?;
        let row: [u32; 16] = match ids.len() {
            1 => [ids[0]; 16],
            16 => ids.as_slice().try_into().map_err(|_| "len")?,
            n => return Err(format!("1.8 shape list for {} has {n} entries", b.name)),
        };
        per_meta.push(row);
    }
    // Unregistered metas read as air in vanilla: no collision.
    let empty_id = *shapes
        .shapes
        .iter()
        .find(|(_, s)| s.is_empty())
        .ok_or("no empty shape")?
        .0;
    let mut cleared = 0;
    for (row, mask) in per_meta.iter_mut().zip(&registered) {
        for (m, id) in row.iter_mut().enumerate() {
            if mask & (1 << m) == 0 && *id != empty_id {
                *id = empty_id;
                cleared += 1;
            }
        }
    }
    notes.push(format!("1.8: {cleared} unregistered (id, meta) states had a minecraft-data shape; set to empty (vanilla reads them as air)"));
    let used: BTreeSet<u32> = per_meta.iter().flatten().copied().collect();
    let table = shape_table(&shapes, &used)?;
    if table.shapes.len() > 256 {
        return Err("more than 256 1.8 shapes".into());
    }
    for (b, row) in blocks.iter().zip(&per_meta) {
        let empty = row
            .iter()
            .all(|s| table.shapes[table.remap[s] as usize].is_empty());
        if empty != b.bbox_empty {
            notes.push(format!(
                "1.8: {} boundingBox={} but collision shapes say empty={empty}",
                b.name,
                if b.bbox_empty { "empty" } else { "block" }
            ));
        }
    }

    let src = |k: &str| -> R<String> { Ok(format!("{}/{k}.json", p.dir(k)?)) };
    let mut out = header(
        "1.8 block table: per-block info, per-state (`id << 4 | meta`) collision shape index.",
        &[
            src("blocks")?,
            src("blockCollisionShapes")?,
            "codegen/data/src/facts.rs".into(),
        ],
    );
    out.push_str(
        "use crate::common::*;\nuse crate::v1_8::{Material, MaterialKind, VanillaProps};\n\n",
    );
    emit_blocks_common(&mut out, "v1_8", &blocks, &phys)?;
    out.push('\n');
    emit_shapes(&mut out, &table);
    out.push('\n');
    emit_dependencies(&mut out, &deps);
    out.push_str("\n/// Shape index per raw state `id << 4 | meta`.\n");
    let flat: Vec<u32> = per_meta.iter().flatten().map(|s| table.remap[s]).collect();
    num_table(&mut out, "pub(crate) static STATE_SHAPE", "u8", &flat, 16);
    emit_world_coords(&mut out, &blocks, &per_meta, &table, &registered)?;
    emit_vanilla_props(&mut out, &blocks, &phys, oracle_boxes)?;
    emit_v1_8_selection(&mut out, &blocks, oracle_selection)?;
    out.push_str("\n/// Bit `m` set = meta `m` is a registered state of the block (from the 1.8.9 oracle).\n");
    let masks: Vec<u32> = registered.iter().map(|&m| u32::from(m)).collect();
    num_table(
        &mut out,
        "pub(crate) static REGISTERED_METAS",
        "u16",
        &masks,
        16,
    );

    let mut files = vec![
        ("blocks.rs".to_string(), out),
        (
            "items.rs".to_string(),
            items_file("v1_8", &src("items")?, &items)?,
        ),
    ];
    files.extend(misc_files(
        p,
        &items,
        (oracle_entities, facts::V1_8_ENTITY_SIZE_FIXES),
        notes,
    )?);
    files.push((
        "recipes.rs".to_string(),
        v1_8_recipes(oracle_recipes, &items)?,
    ));
    files.push((
        "dig.rs".to_string(),
        crate::dig_tables::generate(
            "Minecraft 1.8.9 dig tables (oracle).",
            &oracle_recipes.with_file_name("dig_facts.jsonl.gz"),
            "tests/golden/1.8.9/dig_facts.jsonl.gz",
            "strength",
            "no_tool_needed",
        )?,
    ));
    let mods: Vec<&str> = MODS
        .iter()
        .copied()
        .chain(["recipes", "dig"])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    files.push(("mod.rs".to_string(), mod_rs("Generated 1.8 tables.", &mods)));
    Ok(files
        .into_iter()
        .map(|(n, c)| (format!("v1_8/{n}"), c))
        .collect())
}

/// 1.16.5 (protocol 754): global block-state palette with properties.
pub fn v1_16_5(
    p: &Paths,
    oracle_shapes: &std::path::Path,
    oracle_voxels: &std::path::Path,
    oracle_entities: &std::path::Path,
    notes: &mut Notes,
) -> R<Vec<File>> {
    let blocks = model::blocks(&p.json("blocks")?)?;
    check_dense(&blocks)?;
    check_unique("block", blocks.iter().map(|b| b.name.as_str()))?;
    let mut items = model::items(&p.json("items")?)?;
    apply_item_fixes(&mut items, facts::V1_16_5_ITEM_FIXES, &p.version, notes)?;
    check_harvest_tools(&blocks, &items)?;
    let (phys, deps) = physics(&facts::V1_16_5, &blocks)?;

    // State layout: contiguous, product of property sizes, first property most significant.
    let mut next = 0u32;
    for b in &blocks {
        let n: u32 = b.props.iter().map(|p| p.values.len() as u32).product();
        if b.min_state != next
            || b.max_state + 1 - b.min_state != n
            || !(b.min_state..=b.max_state).contains(&b.default_state)
        {
            return Err(format!(
                "{}: bad state range {}..={} default {}",
                b.name, b.min_state, b.max_state, b.default_state
            ));
        }
        next = b.max_state + 1;
    }
    let state_count = next as usize;
    if state_count > usize::from(u16::MAX) {
        return Err("too many states for u16".into());
    }

    // Property definitions (deduplicated by name + values).
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
    let prop_names: BTreeSet<&str> = defs.iter().map(|d| d.0.as_str()).collect();

    // Shapes per state.
    let shapes = model::shapes(&p.json("blockCollisionShapes")?)?;
    let mut per_state: Vec<u32> = Vec::with_capacity(state_count);
    for b in &blocks {
        let ids = shapes
            .blocks
            .get(&b.name)
            .ok_or_else(|| format!("no collision shape for {}", b.name))?;
        let n = (b.max_state - b.min_state + 1) as usize;
        match ids.len() {
            1 => per_state.extend(std::iter::repeat_n(ids[0], n)),
            m if m == n => per_state.extend(ids),
            m => return Err(format!("{}: {m} shapes for {n} states", b.name)),
        }
    }
    let used: BTreeSet<u32> = per_state.iter().copied().collect();
    let table = shape_table(&shapes, &used)?;

    // Fluids per state: 0 = none, 0x10 | level = water, 0x20 | level = lava.
    let mut fluid = vec![0u32; state_count];
    let mut state_block = vec![0u32; state_count];
    for b in &blocks {
        let level_prop = b.props.iter().position(|p| p.name == "level");
        let wl_prop = b.props.iter().position(|p| p.name == "waterlogged");
        let always = facts::V1_16_ALWAYS_WATER.contains(&b.name.as_str());
        for s in b.min_state..=b.max_state {
            let idx = decode(b, s);
            state_block[s as usize] = u32::from(b.id);
            let level = level_prop.map_or(0, |i| idx[i] as u32);
            fluid[s as usize] = match b.name.as_str() {
                "water" => 0x10 | level,
                "lava" => 0x20 | level,
                _ if always => 0x10,
                _ if wl_prop.is_some_and(|i| b.props[i].values[idx[i]] == "true") => 0x10,
                _ => 0,
            };
        }
    }
    for f in facts::V1_16_ALWAYS_WATER {
        if !blocks.iter().any(|b| b.name == *f) {
            return Err(format!("always-water block {f} missing"));
        }
    }

    let src = |k: &str| -> R<String> { Ok(format!("{}/{k}.json", p.dir(k)?)) };
    if p.dir("blocks")? != p.dir("blockCollisionShapes")? {
        notes.push(format!(
            "1.16.5: blocks from {} but collision shapes from {}; state counts checked per block and match",
            p.dir("blocks")?,
            p.dir("blockCollisionShapes")?
        ));
    }
    let mut out = header(
        "1.16.5 block table: per-block info and state layout, per-state block / shape / fluid.",
        &[
            src("blocks")?,
            src("blockCollisionShapes")?,
            "codegen/data/src/facts.rs".into(),
        ],
    );
    out.push_str("use crate::common::*;\nuse crate::v1_16_5::{OffsetType, PropertyDef, StateLayout, VanillaBlock, VoxelClass, VoxelShapeRef};\n\n");
    emit_blocks_common(&mut out, "v1_16_5", &blocks, &phys)?;

    // Property enum.
    out.push_str("\n/// Every block-state property name used by 1.16.5.\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]\npub enum Property {\n");
    for n in &prop_names {
        let _ = writeln!(out, "    /// `{n}`\n    {},", camel_ident(n));
    }
    out.push_str("}\n\nimpl Property {\n    /// The property's name as used in block-state strings.\n    #[must_use]\n    pub const fn name(self) -> &'static str {\n        match self {\n");
    for n in &prop_names {
        let _ = writeln!(
            out,
            "            Property::{} => {},",
            camel_ident(n),
            lit(n)
        );
    }
    out.push_str("        }\n    }\n}\n\n");
    let _ = writeln!(
        out,
        "pub(crate) static PROPERTY_NAMES: [(&str, Property); {}] = [",
        prop_names.len()
    );
    for n in &prop_names {
        let _ = writeln!(out, "    ({}, Property::{}),", lit(n), camel_ident(n));
    }
    out.push_str("];\n\n");
    let _ = writeln!(
        out,
        "pub(crate) static PROPERTY_DEFS: [PropertyDef; {}] = [",
        defs.len()
    );
    for (n, vals) in &defs {
        let v: Vec<String> = vals.iter().map(|x| lit(x)).collect();
        let _ = writeln!(
            out,
            "    PropertyDef {{ property: Property::{}, values: &[{}] }},",
            camel_ident(n),
            v.join(", ")
        );
    }
    out.push_str("];\n\n");
    let _ = writeln!(
        out,
        "pub(crate) static LAYOUTS: [StateLayout; {}] = [",
        blocks.len()
    );
    for (b, pr) in blocks.iter().zip(&block_props) {
        let ps: Vec<String> = pr.iter().map(usize::to_string).collect();
        let _ = writeln!(
            out,
            "    StateLayout {{ min_state: {}, state_count: {}, default_state: {}, properties: &[{}] }},",
            b.min_state,
            b.max_state - b.min_state + 1,
            b.default_state,
            ps.join(", ")
        );
    }
    out.push_str("];\n\n");
    emit_shapes(&mut out, &table);
    out.push('\n');
    emit_dependencies(&mut out, &deps);
    out.push_str("\n/// Block id per state.\n");
    num_table(
        &mut out,
        "pub(crate) static STATE_BLOCK",
        "u16",
        &state_block,
        20,
    );
    out.push_str("\n/// Shape index per state.\n");
    let flat: Vec<u32> = per_state.iter().map(|s| table.remap[s]).collect();
    num_table(&mut out, "pub(crate) static STATE_SHAPE", "u16", &flat, 20);
    out.push_str("\n/// Fluid per state: 0 none, `0x10 | level` water, `0x20 | level` lava.\n");
    num_table(&mut out, "pub(crate) static STATE_FLUID", "u8", &fluid, 32);
    emit_v1_16_oracle(&mut out, &blocks, &per_state, &table, oracle_shapes)?;
    emit_v1_16_voxels(&mut out, &blocks, &phys, oracle_voxels)?;

    let mut files = vec![
        ("blocks.rs".to_string(), out),
        (
            "items.rs".to_string(),
            items_file("v1_16_5", &src("items")?, &items)?,
        ),
    ];
    files.extend(misc_files(
        p,
        &items,
        (oracle_entities, facts::V1_16_5_ENTITY_SIZE_FIXES),
        notes,
    )?);
    files.push((
        "dig.rs".to_string(),
        crate::dig_tables::generate(
            "Minecraft 1.16.5 dig tables (oracle).",
            &oracle_voxels.with_file_name("dig_facts.jsonl.gz"),
            "tests/golden/1.16.5/dig_facts.jsonl.gz",
            "hardness",
            "requires_tool",
        )?,
    ));
    files.push((
        "selection.rs".to_string(),
        crate::selection::generate(
            "Minecraft 1.16.5 crosshair (outline / raycast) shape tables.",
            &oracle_voxels.with_file_name("selection_shapes.jsonl.gz"),
            facts::V1_16_5_ORACLE_SELECTION,
            state_count,
        )?,
    ));
    let mut mods = MODS.to_vec();
    mods.push("selection");
    mods.push("dig");
    mods.sort_unstable();
    files.push((
        "mod.rs".to_string(),
        mod_rs("Generated 1.16.5 tables.", &mods),
    ));
    Ok(files
        .into_iter()
        .map(|(n, c)| (format!("v1_16_5/{n}"), c))
        .collect())
}

/// Property value indices of global state `s` of block `b` (first property most significant).
fn decode(b: &Block, s: u32) -> Vec<usize> {
    let mut rem = s - b.min_state;
    let mut idx = vec![0; b.props.len()];
    for (i, p) in b.props.iter().enumerate().rev() {
        let n = p.values.len() as u32;
        idx[i] = (rem % n) as usize;
        rem /= n;
    }
    idx
}

/// Patches `facts::V1_8_SHAPE_OVERRIDES` into the minecraft-data shape map (as new shape ids).
fn apply_overrides(shapes: &mut model::Shapes, blocks: &[Block], notes: &mut Notes) -> R<()> {
    let first = shapes.shapes.keys().max().copied().unwrap_or(0) + 1;
    for (next, o) in (first..).zip(facts::V1_8_SHAPE_OVERRIDES) {
        if !blocks.iter().any(|b| b.name == o.block) {
            return Err(format!("shape override for unknown block {}", o.block));
        }
        let entry = shapes
            .blocks
            .get_mut(o.block)
            .ok_or_else(|| format!("no shapes for {}", o.block))?;
        if entry.len() == 1 {
            *entry = vec![entry[0]; 16];
        }
        shapes.shapes.insert(next, o.boxes.to_vec());
        for &m in o.metas {
            entry[usize::from(m)] = next;
        }
        notes.push(format!(
            "1.8 override {} metas {:?}: {}",
            o.block, o.metas, o.why
        ));
    }
    Ok(())
}

/// Per block id, a bit mask of registered metas, read from the oracle's collision dump.
fn registered_metas(path: &std::path::Path, count: usize) -> R<Vec<u16>> {
    let text = read_gz(path)?;
    let mut masks = vec![0u16; count];
    let mut seen = 0usize;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("{}: {e}", path.display()))?;
        if v["type"] != "boxes" || v["config"] != "alone" {
            continue;
        }
        let id = v["id"].as_u64().ok_or("oracle line without id")? as usize;
        let meta = v["meta"].as_u64().ok_or("oracle line without meta")?;
        if id >= count || meta > 15 {
            return Err(format!("oracle id/meta {id}:{meta} out of range"));
        }
        seen += 1;
        if !v["state"].is_null() {
            masks[id] |= 1 << meta;
        }
    }
    if seen != count * 16 {
        return Err(format!(
            "oracle dump has {seen} alone lines, expected {}",
            count * 16
        ));
    }
    Ok(masks)
}

fn wc_lit(w: facts::Wc) -> String {
    match w {
        facts::Wc::D => "WorldCoord::Double".into(),
        facts::Wc::F(k, t) => format!("WorldCoord::Float {{ int_offset: {k}, term: {} }}", f32l(t)),
    }
}

/// Per-state float world-coordinate rules (`facts::V1_8_FLOAT_COORDS`, cake, fence gates), each
/// checked against the table box: evaluated at block position 0 it must give the local bound.
fn emit_world_coords(
    out: &mut String,
    blocks: &[Block],
    per_meta: &[[u32; 16]],
    table: &ShapeTable,
    registered: &[u16],
) -> R<()> {
    let id_of = |name: &str| -> R<usize> {
        blocks
            .iter()
            .position(|b| b.name == name)
            .ok_or_else(|| format!("float-coords block {name} unknown"))
    };
    let mut rules: Vec<Option<Vec<[facts::Wc; 6]>>> = vec![None; blocks.len() * 16];
    for fc in facts::V1_8_FLOAT_COORDS
        .iter()
        .chain(facts::V1_8_CAKE_FLOAT_COORDS)
    {
        let id = id_of(fc.block)?;
        for &m in fc.metas {
            if registered[id] & (1 << m) != 0 {
                rules[id * 16 + usize::from(m)] = Some(fc.boxes.to_vec());
            }
        }
    }
    for g in facts::V1_8_FENCE_GATES {
        let id = id_of(g)?;
        for m in 0..16usize {
            if registered[id] & (1 << m) != 0 && m & 4 == 0 {
                let r = if m & 1 == 0 {
                    facts::FENCE_GATE_Z
                } else {
                    facts::FENCE_GATE_X
                };
                rules[id * 16 + m] = Some(vec![r]);
            }
        }
    }
    // Validate against the table and deduplicate.
    let mut sets: Vec<Vec<[facts::Wc; 6]>> = Vec::new();
    let mut idx = vec![0u32; rules.len()];
    for (raw, r) in rules.iter().enumerate() {
        let Some(r) = r else { continue };
        let shape = &table.shapes[table.remap[&per_meta[raw / 16][raw % 16]] as usize];
        if shape.len() != r.len() {
            return Err(format!(
                "float coords for {}:{}: {} rules, {} boxes",
                blocks[raw / 16].name,
                raw % 16,
                r.len(),
                shape.len()
            ));
        }
        for (b, rule) in shape.iter().zip(r) {
            for (local, w) in b.iter().zip(rule) {
                if let facts::Wc::F(k, t) = *w {
                    let at0 = f64::from(f32::from(k) + t);
                    if at0.to_bits() != local.to_bits() {
                        return Err(format!(
                            "float coords for {}:{}: rule gives {at0} at the origin, table {local}",
                            blocks[raw / 16].name,
                            raw % 16
                        ));
                    }
                }
            }
        }
        let i = sets.iter().position(|x| x == r).unwrap_or_else(|| {
            sets.push(r.clone());
            sets.len() - 1
        });
        idx[raw] = i as u32 + 1;
    }
    let _ = writeln!(
        out,
        "\n/// Float world-coordinate rule sets (index 0 = none, plain double placement).\npub(crate) static WORLD_COORDS: [Option<&[WorldCoords]>; {}] = [\n    None,",
        sets.len() + 1
    );
    for set in &sets {
        let boxes: Vec<String> = set
            .iter()
            .map(|r| {
                format!(
                    "[{}]",
                    r.iter().map(|w| wc_lit(*w)).collect::<Vec<_>>().join(", ")
                )
            })
            .collect();
        let _ = writeln!(out, "    Some(&[{}]),", boxes.join(", "));
    }
    out.push_str("];\n\n/// Index into `WORLD_COORDS` per raw state.\n");
    num_table(out, "pub(crate) static STATE_WORLD_COORDS", "u8", &idx, 16);
    Ok(())
}

/// Per-block vanilla shape / material flags from the oracle dump's `"type":"block"` lines, plus the
/// material identity (`facts::V1_8_MATERIAL_KINDS`, by name: the dump carries no identity).
fn emit_vanilla_props(
    out: &mut String,
    blocks: &[Block],
    phys: &[Physics],
    path: &std::path::Path,
) -> R<()> {
    let text = read_gz(path)?;
    let mut rows: Vec<Option<String>> = vec![None; blocks.len()];
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        if v["type"] != "block" {
            continue;
        }
        let id = v["id"].as_u64().ok_or("block line without id")? as usize;
        let b = blocks
            .get(id)
            .ok_or_else(|| format!("oracle block id {id} unknown"))?;
        if v["name"].as_str() != Some(format!("minecraft:{}", b.name).as_str()) {
            return Err(format!(
                "oracle block {id} is {}, table {}",
                v["name"], b.name
            ));
        }
        let flag = |k: &str| -> R<bool> {
            v[k].as_bool()
                .ok_or_else(|| format!("oracle block {id}: no {k}"))
        };
        let kind = facts::V1_8_MATERIAL_KINDS
            .iter()
            .find(|(n, _)| *n == b.name)
            .map_or("Other", |(_, k)| k);
        // Cross-check: the hand-written liquid physics flags must be exactly the fluid materials.
        let liquid = phys[id].flags & (facts::WATER | facts::LAVA) != 0;
        if liquid != flag("material_fluid")? || liquid != matches!(kind, "Water" | "Lava") {
            return Err(format!(
                "{}: liquid flags disagree with the oracle material",
                b.name
            ));
        }
        rows[id] = Some(format!(
            "VanillaProps {{ full_cube: {}, normal_block: {}, full_block: {}, render_as_normal_block: {}, has_collision: {}, material: Material {{ kind: MaterialKind::{kind}, blocks_movement: {}, solid: {}, fluid: {}, opaque: {}, replaceable: {} }} }}",
            flag("full_cube")?,
            flag("normal_block")?,
            flag("full_block")?,
            flag("render_as_normal_block")?,
            flag("has_collision")?,
            flag("material_blocks_movement")?,
            flag("material_solid")?,
            flag("material_fluid")?,
            flag("material_opaque")?,
            flag("material_replaceable")?,
        ));
    }
    for (n, _) in facts::V1_8_MATERIAL_KINDS {
        if !blocks.iter().any(|b| b.name == *n) {
            return Err(format!("material kind for unknown block {n}"));
        }
    }
    let _ = writeln!(
        out,
        "\n/// Vanilla per-block flags (1.8.9 oracle `block` lines).\npub(crate) static VANILLA_PROPS: [VanillaProps; {}] = [",
        blocks.len()
    );
    for (i, r) in rows.iter().enumerate() {
        let r = r
            .as_ref()
            .ok_or_else(|| format!("oracle has no block line for id {i}"))?;
        let _ = writeln!(out, "    {r},");
    }
    out.push_str("];\n");
    Ok(())
}

/// Applies `facts::*_ITEM_FIXES`: corrects the stack size of a listed item, or adds it if missing.
pub(crate) fn apply_item_fixes(
    items: &mut Vec<model::Item>,
    fixes: &[facts::ItemFix],
    version: &str,
    notes: &mut Notes,
) -> R<()> {
    for f in fixes {
        match items.iter_mut().find(|i| i.name == f.name) {
            Some(i) => {
                if i.id != f.id {
                    return Err(format!("item fix {}: id {} != {}", f.name, i.id, f.id));
                }
                if i.stack_size == f.stack_size {
                    return Err(format!(
                        "item fix {} is a no-op (stack {}); remove it",
                        f.name, f.stack_size
                    ));
                }
                i.stack_size = f.stack_size;
            }
            None => {
                if items.iter().any(|i| i.id == f.id) {
                    return Err(format!("item fix {}: id {} already used", f.name, f.id));
                }
                items.push(model::Item {
                    id: f.id,
                    name: f.name.into(),
                    display_name: f.display_name.into(),
                    stack_size: f.stack_size,
                    max_durability: None,
                    variations: Vec::new(),
                });
                items.sort_by_key(|i| i.id);
            }
        }
        notes.push(format!("{version} item fix {}: {}", f.name, f.why));
    }
    Ok(())
}

pub(crate) fn oracle_lines(path: &std::path::Path) -> R<Vec<serde_json::Value>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| format!("{}: {e}", path.display())))
        .collect()
}

pub(crate) fn boxes_of(v: &serde_json::Value) -> R<Vec<[u64; 6]>> {
    let mut out = Vec::new();
    for b in v.as_array().ok_or("boxes")? {
        let mut a = [0u64; 6];
        for (i, c) in b.as_array().ok_or("box")?.iter().enumerate() {
            a[i] =
                u64::from_str_radix(c[1].as_str().ok_or("hex")?, 16).map_err(|e| e.to_string())?;
        }
        out.push(a);
    }
    out.sort_unstable();
    Ok(out)
}

/// 1.16.5 facts from the oracle shape dump: per-state vanilla flags (full cube, solid block,
/// suffocates), per-block collidable / offset type, and the scaffolding context shapes.
fn emit_v1_16_oracle(
    out: &mut String,
    blocks: &[Block],
    per_state: &[u32],
    table: &ShapeTable,
    path: &std::path::Path,
) -> R<()> {
    let lines = oracle_lines(path)?;
    let mut flags = vec![None::<u32>; per_state.len()];
    let mut block_rows = vec![None::<String>; blocks.len()];
    let mut bottom: Option<Vec<[u64; 6]>> = None;
    let mut ctx_states = 0;
    for v in &lines {
        match v["type"].as_str() {
            Some("block") => {
                let name = v["name"]
                    .as_str()
                    .ok_or("block name")?
                    .trim_start_matches("minecraft:");
                let id = blocks
                    .iter()
                    .position(|b| b.name == name)
                    .ok_or_else(|| format!("oracle block {name} unknown"))?;
                let b = &blocks[id];
                if v["first_state"].as_u64() != Some(u64::from(b.min_state))
                    || v["states"].as_u64() != Some(u64::from(b.max_state - b.min_state + 1))
                    || v["default_state"].as_u64() != Some(u64::from(b.default_state))
                {
                    return Err(format!(
                        "oracle state layout of {name} differs from minecraft-data"
                    ));
                }
                let offset = match v["offset_type"].as_str() {
                    Some("none") => "None",
                    Some("xz") => "Xz",
                    Some("xyz") => "Xyz",
                    o => return Err(format!("{name}: offset_type {o:?}")),
                };
                block_rows[id] = Some(format!(
                    "VanillaBlock {{ collidable: {}, offset: OffsetType::{offset} }}",
                    v["collidable"].as_bool().ok_or("collidable")?
                ));
            }
            Some("shape") => {
                let id = v["id"].as_u64().ok_or("shape id")? as usize;
                if id >= flags.len() {
                    return Err(format!("oracle state {id} out of range"));
                }
                let bit = |k: &str, b: u32| -> R<u32> {
                    Ok(if v[k].as_bool().ok_or_else(|| k.to_string())? {
                        b
                    } else {
                        0
                    })
                };
                flags[id] =
                    Some(bit("full_cube", 1)? | bit("solid_block", 2)? | bit("suffocates", 4)?);
                if let Some(ctx) = v.get("contexts").and_then(|c| c.as_object()) {
                    ctx_states += 1;
                    let name = v["state"].as_str().unwrap_or_default();
                    if !name.starts_with("minecraft:scaffolding[") {
                        return Err(format!("context-dependent state {name} is not scaffolding"));
                    }
                    let own = boxes_of(&v["boxes"])?;
                    let table_own: Vec<[u64; 6]> = {
                        let mut t: Vec<[u64; 6]> = table.shapes
                            [table.remap[&per_state[id]] as usize]
                            .iter()
                            .map(|b| b.map(f64::to_bits))
                            .collect();
                        t.sort_unstable();
                        t
                    };
                    let has_bottom = name.contains("bottom=true") && !name.contains("distance=0");
                    let ctx_boxes = |k: &str| -> R<Vec<[u64; 6]>> {
                        boxes_of(
                            ctx.get(k)
                                .ok_or_else(|| format!("{name}: no context {k}"))?,
                        )
                    };
                    if ctx_boxes("above")? != own
                        || own != table_own
                        || !ctx_boxes("below")?.is_empty()
                    {
                        return Err(format!("{name}: stable / below context not as modelled"));
                    }
                    for k in ["inside", "inside_descending", "above_descending"] {
                        let b = ctx_boxes(k)?;
                        if has_bottom {
                            if bottom.get_or_insert_with(|| b.clone()) != &b {
                                return Err(format!("{name}: bottom shapes differ"));
                            }
                        } else if !b.is_empty() {
                            return Err(format!("{name}: context {k} not empty"));
                        }
                    }
                }
            }
            t => return Err(format!("unknown oracle line type {t:?}")),
        }
    }
    if ctx_states != 32 {
        return Err(format!(
            "expected 32 scaffolding context states, got {ctx_states}"
        ));
    }
    let _ = writeln!(
        out,
        "\n/// Vanilla per-block facts (1.16.5 oracle).\npub(crate) static VANILLA_BLOCKS: [VanillaBlock; {}] = [",
        blocks.len()
    );
    for (i, r) in block_rows.iter().enumerate() {
        let _ = writeln!(
            out,
            "    {},",
            r.as_ref()
                .ok_or_else(|| format!("oracle has no block line for {}", blocks[i].name))?
        );
    }
    out.push_str("];\n\n/// Per state: bit 0 full cube, bit 1 solid block, bit 2 suffocates (1.16.5 oracle).\n");
    let flags: Vec<u32> = flags
        .into_iter()
        .enumerate()
        .map(|(i, f)| f.ok_or_else(|| format!("oracle has no shape line for state {i}")))
        .collect::<R<_>>()?;
    num_table(
        out,
        "pub(crate) static STATE_VANILLA_FLAGS",
        "u8",
        &flags,
        32,
    );
    let bottom = bottom.ok_or("no scaffolding bottom shape")?;
    let b: Vec<String> = bottom
        .iter()
        .map(|b| {
            let f = b.map(f64::from_bits);
            format!(
                "Aabb::new({}, {}, {}, {}, {}, {})",
                f64l(f[0]),
                f64l(f[1]),
                f64l(f[2]),
                f64l(f[3]),
                f64l(f[4]),
                f64l(f[5])
            )
        })
        .collect();
    let _ = writeln!(
        out,
        "\n/// Scaffolding \"bottom\" collision shape (1.16.5 oracle contexts).\npub(crate) static SCAFFOLDING_BOTTOM: &[Aabb] = &[{}];",
        b.join(", ")
    );
    Ok(())
}

/// 1.16.5 VoxelShape grids and per-state facts from the oracle voxel dump. The per-block `tags`
/// are cross-checked against the hand-written tag flags (`facts::V1_16_5`).
fn emit_v1_16_voxels(
    out: &mut String,
    blocks: &[Block],
    phys: &[Physics],
    path: &std::path::Path,
) -> R<()> {
    let lines = oracle_lines(path)?;
    let n_states = blocks.last().map_or(0, |b| b.max_state as usize + 1);
    let mut shapes: Vec<String> = Vec::new();
    let mut state_sid = vec![u32::MAX; n_states];
    let mut state_flags = vec![u32::MAX; n_states];
    let mut block_material: Vec<Option<String>> = vec![None; blocks.len()];
    let mut scaffolding_bottom_sid: Option<u64> = None;
    let mut scaffolding_ctx: Vec<(usize, String, serde_json::Map<String, serde_json::Value>)> =
        Vec::new();
    let mut contexts: Vec<(String, f64, i32, bool)> = Vec::new();
    let state_block = |id: usize| {
        blocks
            .iter()
            .position(|b| (b.min_state as usize..=b.max_state as usize).contains(&id))
    };
    for v in &lines {
        match v["type"].as_str() {
            Some("block") => {
                let name = v["name"]
                    .as_str()
                    .ok_or("name")?
                    .trim_start_matches("minecraft:");
                let id = blocks
                    .iter()
                    .position(|b| b.name == name)
                    .ok_or_else(|| format!("voxel dump block {name} unknown"))?;
                let tags = &v["tags"];
                let checks = [
                    ("climbable", facts::CLIMBABLE),
                    ("fences", facts::FENCE),
                    ("walls", facts::WALL),
                    ("fence_gates", facts::FENCE_GATE),
                    ("trapdoors", facts::TRAPDOOR),
                    ("soul_speed_blocks", facts::SOUL_SPEED),
                    ("fire", facts::FIRE),
                ];
                for (tag, bit) in checks {
                    if tags[tag]
                        .as_bool()
                        .ok_or_else(|| format!("{name}: tag {tag}"))?
                        != (phys[id].flags & bit != 0)
                    {
                        return Err(format!(
                            "{name}: tag {tag} disagrees with facts.rs (oracle {})",
                            tags[tag]
                        ));
                    }
                }
            }
            Some("voxel_shape") => {
                let sid = v["sid"].as_u64().ok_or("sid")? as usize;
                if sid != shapes.len() {
                    return Err(format!(
                        "voxel sids not dense: {sid} after {}",
                        shapes.len()
                    ));
                }
                let class = match v["class"].as_str() {
                    Some("simple") => "Simple",
                    Some("array") => "Array",
                    c => return Err(format!("voxel shape {sid}: class {c:?} not supported")),
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
                        .map(|c| {
                            Ok(f64l(f64::from_bits(
                                u64::from_str_radix(c[1].as_str().ok_or("hex")?, 16)
                                    .map_err(|e| e.to_string())?,
                            )))
                        })
                        .collect::<R<_>>()?;
                    Ok(p.join(", "))
                };
                let filled = v["filled"].as_str().ok_or("filled")?;
                let n = (sizes[0] * sizes[1] * sizes[2]) as usize;
                if filled.len() != n {
                    return Err(format!(
                        "voxel shape {sid}: {} cells, sizes {sizes:?}",
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
                shapes.push(format!(
                    "VoxelShapeRef {{ class: VoxelClass::{class}, sizes: [{}, {}, {}], xs: &[{}], ys: &[{}], zs: &[{}], cells: &[{}], empty: {} }}",
                    sizes[0], sizes[1], sizes[2], pts("points_x")?, pts("points_y")?, pts("points_z")?, w.join(", "),
                    v["empty"].as_bool().ok_or("empty")?
                ));
            }
            Some("state") => {
                let id = v["id"].as_u64().ok_or("state id")? as usize;
                let b = state_block(id).ok_or_else(|| format!("voxel state {id} out of range"))?;
                state_sid[id] = v["collision_shape"].as_u64().ok_or("collision_shape")? as u32;
                let bit = |k: &str, b: u32| -> R<u32> {
                    Ok(if v[k].as_bool().ok_or_else(|| k.to_string())? {
                        b
                    } else {
                        0
                    })
                };
                let mut f = bit("full_cube_identity", 1)?
                    | bit("exceeds_cube", 2)?
                    | bit("should_suffocate", 4)?
                    | bit("is_air", 8)?
                    | bit("has_dynamic_bounds", 16)?
                    | bit("material_blocks_movement", 32)?
                    | bit("side_solid_up", 1 << 10)?
                    | bit("side_solid_down", 1 << 11)?
                    | bit("empty_identity", 1 << 12)?
                    | bit("material_liquid", 1 << 13)?
                    | bit("material_solid", 1 << 14)?
                    | bit("material_replaceable", 1 << 15)?;
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
                state_flags[id] = f;
                let m = v["material"].as_str().ok_or("material")?.to_owned();
                match &block_material[b] {
                    None => block_material[b] = Some(m),
                    Some(x) if *x == m => {}
                    Some(x) => {
                        return Err(format!(
                            "{}: material differs per state ({x} vs {m})",
                            blocks[b].name
                        ));
                    }
                }
                if let Some(ctx) = v.get("context_shapes").and_then(|c| c.as_object()) {
                    // Checked against the vanilla rule after all lines are read.
                    scaffolding_ctx.push((
                        id,
                        v["state"].as_str().unwrap_or_default().to_owned(),
                        ctx.clone(),
                    ));
                }
            }
            Some("context") => {
                let name = v["name"].as_str().ok_or("context name")?.to_owned();
                let feet = f64::from_bits(
                    u64::from_str_radix(v["feet_y"][1].as_str().ok_or("feet_y")?, 16)
                        .map_err(|e| e.to_string())?,
                );
                let block_y = v["block_y"].as_i64().ok_or("block_y")? as i32;
                contexts.push((
                    name,
                    feet,
                    block_y,
                    v["descending"].as_bool().ok_or("descending")?,
                ));
            }
            t => return Err(format!("unknown voxel dump line type {t:?}")),
        }
    }
    if state_sid.iter().any(|&s| s as usize >= shapes.len()) || state_flags.contains(&u32::MAX) {
        return Err("voxel dump does not cover every state".into());
    }
    // Scaffolding: every dumped context must follow the rule `ScaffoldingShapes::select` encodes
    // (stable above `y + 1 - 1.0E-5F` when not descending, else the bottom shape above
    // `y + 0 - 1.0E-5F` for bottom states with distance != 0, else empty).
    if scaffolding_ctx.len() != 32 || contexts.len() < 5 {
        return Err(format!(
            "expected 32 scaffolding states and the context definitions, got {} / {}",
            scaffolding_ctx.len(),
            contexts.len()
        ));
    }
    let eps = f64::from(1.0e-5_f32);
    for (id, name, ctx) in &scaffolding_ctx {
        let has_bottom = name.contains("bottom=true") && !name.contains("distance=0");
        for (cname, feet, block_y, descending) in &contexts {
            let got = ctx
                .get(cname)
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format!("{name}: no context {cname}"))?;
            let want = if *feet > f64::from(*block_y) + 1.0 - eps && !*descending {
                Some(u64::from(state_sid[*id]))
            } else if has_bottom && *feet > f64::from(*block_y) + 0.0 - eps {
                None // the bottom shape, the same for every state
            } else {
                Some(0)
            };
            match want {
                Some(w) if w != got => {
                    return Err(format!(
                        "{name} context {cname}: oracle sid {got}, rule {w}"
                    ));
                }
                None if *scaffolding_bottom_sid.get_or_insert(got) != got || got == 0 => {
                    return Err(format!("{name} context {cname}: bottom shape sid {got}"));
                }
                _ => {}
            }
        }
    }
    // Material enum values used.
    let mats: BTreeSet<&str> = block_material
        .iter()
        .map(|m| m.as_deref().unwrap_or(""))
        .collect();
    let _ = writeln!(
        out,
        "\n/// Distinct 1.16.5 collision VoxelShapes (oracle; index 0 = the empty shape).\npub(crate) static VOXEL_SHAPES: [VoxelShapeRef; {}] = [",
        shapes.len()
    );
    for sh in &shapes {
        let _ = writeln!(out, "    {sh},");
    }
    out.push_str("];\n\n/// VoxelShape index per state.\n");
    num_table(out, "pub(crate) static STATE_VOXEL", "u16", &state_sid, 20);
    out.push_str(
        "\n/// Per state (oracle voxel dump): bit 0 full-cube identity, 1 exceeds cube, 2 suffocates, 3 is air,\n/// 4 dynamic bounds, 5 material blocks movement, 6..=9 side solid full square N, E, S, W, 10 up,\n/// 11 down, 12 empty identity, 13 material liquid, 14 material solid, 15 material replaceable.\n",
    );
    num_table(
        out,
        "pub(crate) static STATE_VOXEL_FLAGS",
        "u16",
        &state_flags,
        20,
    );
    let _ = writeln!(
        out,
        "\n/// Scaffolding bottom VoxelShape index.\npub(crate) const SCAFFOLDING_BOTTOM_VOXEL: u16 = {};",
        scaffolding_bottom_sid.ok_or("no scaffolding bottom voxel")?
    );
    let _ = writeln!(
        out,
        "\n/// Vanilla material per block (Yarn `Material` field).\npub(crate) static BLOCK_MATERIAL: [Material; {}] = [",
        blocks.len()
    );
    for m in &block_material {
        let _ = writeln!(
            out,
            "    Material::{},",
            camel_ident(m.as_deref().ok_or("block without material")?)
        );
    }
    out.push_str("];\n\n/// Every vanilla material used by a 1.16.5 block (Yarn `Material` field names).\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum Material {\n");
    for m in &mats {
        let _ = writeln!(
            out,
            "    /// `Material.{}`\n    {},",
            m.to_uppercase(),
            camel_ident(m)
        );
    }
    out.push_str("}\n");
    Ok(())
}

/// 1.8.9 selection (ray-test) bounds and canCollide flags from the oracle's `alone` lines.
fn emit_v1_8_selection(out: &mut String, blocks: &[Block], path: &std::path::Path) -> R<()> {
    let lines = oracle_lines(path)?;
    let mut boxes: Vec<[u64; 6]> = Vec::new();
    let mut idx = vec![u32::MAX; blocks.len() * 16];
    let mut flags = vec![0u32; blocks.len() * 16];
    for v in &lines {
        if v["type"] != "selection" || v["config"] != "alone" {
            continue;
        }
        let id = v["id"].as_u64().ok_or("id")? as usize;
        let meta = v["meta"].as_u64().ok_or("meta")? as usize;
        if id >= blocks.len() || meta > 15 {
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
        let bit = |k: &str, m: u32| -> R<u32> {
            Ok(if v[k].as_bool().ok_or_else(|| k.to_string())? {
                m
            } else {
                0
            })
        };
        flags[raw] =
            bit("can_collide", 1)? | bit("can_collide_liquid", 2)? | bit("has_collision_box", 4)?;
    }
    if idx.contains(&u32::MAX) {
        return Err("selection dump does not cover every id x meta".into());
    }
    if boxes.len() > 256 {
        return Err("more than 256 selection boxes".into());
    }
    let _ = writeln!(
        out,
        "\n/// Distinct 1.8.9 selection (ray-test) bounds, block-local (oracle).\npub(crate) static SELECTION_BOUNDS: [Aabb; {}] = [",
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
    num_table(out, "pub(crate) static STATE_SELECTION", "u8", &idx, 16);
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

/// Reads a gzip-compressed text file.
fn read_gz(path: &std::path::Path) -> R<String> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(text)
}

/// 1.8.9 crafting recipes straight from the oracle dump, in vanilla's match order (first match
/// wins). minecraft-data's `recipes.json` has neither the sort order nor the special recipes, so
/// the dump is the source.
fn v1_8_recipes(path: &std::path::Path, items: &[model::Item]) -> R<String> {
    let lines = oracle_lines(path)?;
    let ids: BTreeSet<u16> = items.iter().map(|i| i.id).collect();
    let item = |id: u64| -> R<u16> {
        let id = u16::try_from(id).map_err(|e| e.to_string())?;
        if ids.contains(&id) {
            Ok(id)
        } else {
            Err(format!("recipe item {id} is not a 1.8 item"))
        }
    };
    let ingredient = |v: &serde_json::Value| -> R<String> {
        if v.is_null() {
            return Ok("None".into());
        }
        let a = v.as_array().ok_or("ingredient")?;
        Ok(format!(
            "Some(({}, {}))",
            item(a[0].as_u64().ok_or("id")?)?,
            a[1].as_u64().ok_or("data")?
        ))
    };
    let output = |hex: &str| -> R<String> {
        // Slot wire format: i16 id, i8 count, i16 damage, NBT (0x00 = none).
        let b: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0xff))
            .collect();
        if b.len() != 6 || b[5] != 0 {
            return Err(format!("recipe output {hex}: expected a slot without NBT"));
        }
        let id = i16::from_be_bytes([b[0], b[1]]);
        let damage = i16::from_be_bytes([b[3], b[4]]);
        Ok(format!(
            "({}, {}, {})",
            item(u64::try_from(id).map_err(|e| e.to_string())?)?,
            b[2],
            damage
        ))
    };
    let mut out = header(
        "1.8.9 crafting recipes in vanilla match order (first match wins).",
        &[facts::V1_8_ORACLE_RECIPES.into()],
    )
    .replace(
        &format!("from PrismarineJS minecraft-data {}", crate::fetch::VERSION),
        "from the vanilla 1.8.9 oracle's recipe dump",
    );
    out.push_str("use crate::v1_8::{Recipe, SpecialRecipe};\n\n");
    let mut rows = Vec::new();
    for (n, v) in lines.iter().enumerate() {
        if v["i"].as_u64() != Some(n as u64) {
            return Err(format!("recipe line {n} has index {}", v["i"]));
        }
        if v["copy_nbt"].as_bool() == Some(true) {
            return Err(format!("recipe {n}: copy_nbt not modelled"));
        }
        rows.push(match v["type"].as_str() {
            Some("shaped") => {
                let (w, h) = (v["w"].as_u64().ok_or("w")?, v["h"].as_u64().ok_or("h")?);
                let ins: Vec<String> = v["in"]
                    .as_array()
                    .ok_or("in")?
                    .iter()
                    .map(ingredient)
                    .collect::<R<_>>()?;
                if ins.len() as u64 != w * h {
                    return Err(format!("recipe {n}: {} inputs for {w}x{h}", ins.len()));
                }
                format!(
                    "Recipe::Shaped {{ width: {w}, height: {h}, inputs: &[{}], output: {} }}",
                    ins.join(", "),
                    output(v["out"].as_str().ok_or("out")?)?
                )
            }
            Some("shapeless") => {
                let ins: Vec<String> = v["in"]
                    .as_array()
                    .ok_or("in")?
                    .iter()
                    .map(|x| {
                        let s = ingredient(x)?;
                        s.strip_prefix("Some(")
                            .and_then(|s| s.strip_suffix(')'))
                            .map(str::to_owned)
                            .ok_or_else(|| format!("recipe {n}: empty shapeless ingredient"))
                    })
                    .collect::<R<_>>()?;
                format!(
                    "Recipe::Shapeless {{ inputs: &[{}], output: {} }}",
                    ins.join(", "),
                    output(v["out"].as_str().ok_or("out")?)?
                )
            }
            Some("special") => {
                let k = match v["class"].as_str() {
                    Some("ArmorDyeRecipeType") => "ArmorDye",
                    Some("FireworkRecipeType") => "Firework",
                    Some("CopyingRecipeType") => "BannerPattern",
                    Some("BookCloningRecipeType") => "BookCloning",
                    Some("MapCloningRecipeType") => "MapCloning",
                    Some("MapUpscaleRecipeType") => "MapUpscale",
                    Some("RepairingRecipeType") => "Repairing",
                    Some("PatternRecipeType") => "BannerDuplicate",
                    c => return Err(format!("recipe {n}: unknown special class {c:?}")),
                };
                format!("Recipe::Special(SpecialRecipe::{k})")
            }
            t => return Err(format!("recipe {n}: unknown type {t:?}")),
        });
    }
    let _ = writeln!(
        out,
        "pub(crate) static RECIPES: [Recipe; {}] = [",
        rows.len()
    );
    for r in rows {
        let _ = writeln!(out, "    {r},");
    }
    out.push_str("];\n");
    Ok(out)
}

/// One measured entity from the oracle dump.
struct VanillaEntity {
    registry_id: i64,
    name: String,
    margin: u32,
    fixed: bool,
    /// The registered type dimensions (1.16 `entity_type` lines), else the `default` variant.
    registered: Option<(u32, u32)>,
    /// 1.8 spawn kind (`mob`, `object`, `xp_orb`, `painting`, `global`, `player`, ...).
    spawn: String,
    /// 1.8 spawn-object packet types of this entity (one per minecart kind etc.).
    object_types: Vec<(i64, i64)>,
    /// `(variant, dims, applied)`.
    variants: Vec<(String, [u32; 3], String)>,
    poses: Vec<(String, [u32; 3])>,
}

fn f32_hex(v: &serde_json::Value) -> R<u32> {
    u32::from_str_radix(v[1].as_str().ok_or("f32 [dec, hex]")?, 16).map_err(|e| e.to_string())
}

fn entity_sizes(path: &std::path::Path, version: &str) -> R<Vec<VanillaEntity>> {
    let mut out: Vec<VanillaEntity> = Vec::new();
    let mut registered: BTreeMap<String, (u32, u32, bool)> = BTreeMap::new();
    for v in oracle_lines(path)? {
        let name = v["name"]
            .as_str()
            .ok_or("entity name")?
            .trim_start_matches("minecraft:")
            .to_owned();
        if v["type"] == "entity_type" {
            registered.insert(
                name,
                (
                    f32_hex(&v["width"])?,
                    f32_hex(&v["height"])?,
                    v["fixed"].as_bool().unwrap_or(false),
                ),
            );
            continue;
        }
        if v["type"] != "entity_size" {
            return Err(format!("{version}: unknown entity line {}", v["type"]));
        }
        if v.get("error").is_some() {
            continue; // not constructible on the client (lightning)
        }
        let name = name.trim_end_matches(" (client player)").to_owned();
        let dims = [
            f32_hex(&v["width"])?,
            f32_hex(&v["height"])?,
            f32_hex(&v["eye_height"])?,
        ];
        let margin = f32_hex(v.get("collision_border").unwrap_or(&v["targeting_margin"]))?;
        let variant = v["variant"].as_str().ok_or("variant")?.to_owned();
        let applied = v["applied"]
            .as_str()
            .unwrap_or(if variant == "default" {
                "spawn"
            } else {
                "metadata"
            })
            .to_owned();
        let idx = match out.iter().position(|e| e.name == name) {
            Some(i) => i,
            None => {
                out.push(VanillaEntity {
                    registry_id: v["id"].as_i64().ok_or("entity id")?,
                    name: name.clone(),
                    margin,
                    fixed: v["fixed"].as_bool().unwrap_or(false),
                    registered: None,
                    spawn: v["spawn"].as_str().unwrap_or("").to_owned(),
                    object_types: Vec::new(),
                    variants: Vec::new(),
                    poses: Vec::new(),
                });
                out.len() - 1
            }
        };
        let e = &mut out[idx];
        if e.margin != margin {
            return Err(format!(
                "{version} {name}: targeting margin differs between lines"
            ));
        }
        if let Some(t) = v["object_type"].as_i64() {
            let pair = (t, v["data"].as_i64().unwrap_or(0));
            if !e.object_types.contains(&pair) {
                e.object_types.push(pair);
            }
        }
        if variant == "default" && e.poses.is_empty() {
            for (pose, d) in v
                .get("poses")
                .and_then(serde_json::Value::as_object)
                .into_iter()
                .flatten()
            {
                let a = d.as_array().ok_or("pose dims")?;
                e.poses.push((
                    pose.clone(),
                    [f32_hex(&a[0])?, f32_hex(&a[1])?, f32_hex(&a[2])?],
                ));
            }
        }
        // Several lines per variant (painting motives, item frame facings, both player kinds) must
        // agree on the dimensions; their boxes differ by placement only.
        match e.variants.iter().find(|(n, _, _)| *n == variant) {
            // Dimensions must agree; `applied` may differ between spawn kinds (1.8 player sneaking:
            // metadata for other players, input for the client player): the first line wins.
            Some((_, d, _)) if *d != dims => {
                return Err(format!("{version} {name} {variant}: lines disagree"));
            }
            Some(_) => {}
            None => e.variants.push((variant, dims, applied)),
        }
    }
    for e in &mut out {
        e.registered = registered.get(&e.name).map(|r| (r.0, r.1));
        if !e.variants.iter().any(|(v, _, _)| v == "default") {
            return Err(format!(
                "{version}: entity {} has no default variant",
                e.name
            ));
        }
    }
    // 1.16 types without a client instance (fishing bobber, lightning, ...): registered dims only.
    for (name, (w, h, fixed)) in &registered {
        if !out.iter().any(|e| &e.name == name) {
            out.push(VanillaEntity {
                registry_id: -1,
                name: name.clone(),
                margin: 0,
                fixed: *fixed,
                registered: Some((*w, *h)),
                spawn: String::new(),
                object_types: Vec::new(),
                variants: Vec::new(),
                poses: Vec::new(),
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The vanilla default `(width, height)` of a minecraft-data entity, if the dump measured it.
/// 1.8: mobs match by registry id (= spawn-mob type), objects by spawn-object type; 1.16: by name.
fn vanilla_default(
    sizes: &[VanillaEntity],
    e: &model::Entity,
    version: &str,
) -> Option<(u32, u32)> {
    let v = sizes.iter().find(|v| match (version, e.kind) {
        ("1.8", "Mob") => v.spawn == "mob" && v.registry_id == i64::from(e.id),
        ("1.8", "Object") => v
            .object_types
            .iter()
            .any(|&(t, d)| t == i64::from(e.id) && (t != 10 || d == 0)),
        ("1.8", _) => false,
        // 1.12: minecraft-data's "mob" entries are the entity registry (any spawn path), the
        // "object" entries the spawn-object packet types.
        ("1.12.2", "Mob") => v.spawn != "painting" && v.registry_id == i64::from(e.id),
        ("1.12.2", "Object") => v
            .object_types
            .iter()
            .any(|&(t, d)| t == i64::from(e.id) && (t != 10 || d == 0)),
        ("1.12.2", _) => false,
        _ => v.name == e.name,
    })?;
    v.registered.or_else(|| {
        v.variants
            .iter()
            .find(|(n, _, _)| n == "default")
            .map(|(_, d, _)| (d[0], d[1]))
    })
}

fn apply_entity_size_fixes(
    ents: &mut [model::Entity],
    sizes: &[VanillaEntity],
    fixes: &[(&str, f32, f32)],
    version: &str,
    notes: &mut Notes,
) -> R<()> {
    for &(name, w, h) in fixes {
        let mut hit = false;
        let mut changed = false;
        for e in ents.iter_mut().filter(|e| e.name == name) {
            let van = vanilla_default(sizes, e, version)
                .ok_or_else(|| format!("{version} size fix {name}: no oracle measurement"))?;
            if van != (w.to_bits(), h.to_bits()) {
                return Err(format!(
                    "{version} size fix {name}: {w} x {h} is not the oracle's {} x {}",
                    f32::from_bits(van.0),
                    f32::from_bits(van.1)
                ));
            }
            let old = (
                e.width.map(|x| (x as f32).to_bits()),
                e.height.map(|x| (x as f32).to_bits()),
            );
            if old == (Some(van.0), Some(van.1)) {
                // Another entry with the same name (1.12: registry and spawn-object entries)
                // may still need it; a fix that changes nothing at all is an error below.
                continue;
            }
            changed = true;
            notes.push(format!(
                "{version} entity size fix {name}: {:?} x {:?} -> {w} x {h}",
                e.width, e.height
            ));
            e.width = Some(f64::from(w));
            e.height = Some(f64::from(h));
            hit = true;
        }
        if !hit {
            return Err(format!(
                "{version} size fix {name}: no such minecraft-data entity"
            ));
        }
        if !changed {
            return Err(format!("{version} size fix {name} is a no-op; remove it"));
        }
    }
    let mut left = Vec::new();
    for e in ents.iter() {
        if let Some(van) = vanilla_default(sizes, e, version) {
            let ours = (
                e.width.map(|x| (x as f32).to_bits()),
                e.height.map(|x| (x as f32).to_bits()),
            );
            if ours != (Some(van.0), Some(van.1)) {
                left.push(format!(
                    "{} ({:?} x {:?} vs vanilla {} x {})",
                    e.name,
                    e.width,
                    e.height,
                    f32::from_bits(van.0),
                    f32::from_bits(van.1)
                ));
            }
        }
    }
    if !left.is_empty() {
        return Err(format!(
            "{version}: entity sizes differ from the oracle, add fixes to facts.rs: {}",
            left.join(", ")
        ));
    }
    Ok(())
}

fn dims_lit(d: &[u32; 3]) -> String {
    format!(
        "EntityDimensions {{ width: {}, height: {}, eye_height: {} }}",
        f32l(f32::from_bits(d[0])),
        f32l(f32::from_bits(d[1])),
        f32l(f32::from_bits(d[2]))
    )
}

fn emit_entity_sizes(out: &mut String, sizes: &[VanillaEntity]) -> R<()> {
    let _ = writeln!(
        out,
        "\n/// Vanilla entity dimensions measured by the oracle, sorted by name.\npub(crate) static VANILLA_ENTITY_SIZES: [VanillaEntitySize; {}] = [",
        sizes.len()
    );
    for e in sizes {
        let mut vars = Vec::new();
        for (n, d, a) in &e.variants {
            let applied = match a.as_str() {
                "spawn" => "Spawn",
                "metadata" => "Metadata",
                "next_tick" => "NextTick",
                "input" => "Input",
                "server" => "Server",
                x => return Err(format!("{}: unknown applied {x}", e.name)),
            };
            vars.push(format!(
                "EntityVariant {{ name: {}, dimensions: {}, applied: VariantApplied::{applied} }}",
                lit(n),
                dims_lit(d)
            ));
        }
        let poses: Vec<String> = e
            .poses
            .iter()
            .map(|(n, d)| format!("({}, {})", lit(n), dims_lit(d)))
            .collect();
        let registered = e.registered.map_or("None".into(), |(w, h)| {
            format!(
                "Some(({}, {}))",
                f32l(f32::from_bits(w)),
                f32l(f32::from_bits(h))
            )
        });
        let objs: Vec<String> = e
            .object_types
            .iter()
            .map(|(t, d)| format!("({t}, {d})"))
            .collect();
        let _ = writeln!(
            out,
            "    VanillaEntitySize {{ name: {}, registry_id: {}, spawn: {}, object_types: &[{}], targeting_margin: {}, fixed: {}, registered: {registered}, variants: &[{}], poses: &[{}] }},",
            lit(&e.name),
            e.registry_id,
            lit(&e.spawn),
            objs.join(", "),
            f32l(f32::from_bits(e.margin)),
            e.fixed,
            vars.join(", "),
            poses.join(", ")
        );
    }
    out.push_str("];\n");
    Ok(())
}
