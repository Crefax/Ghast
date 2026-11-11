//! Minecraft 26.1 entity tables (`generated/v26_1/entities.rs`).
//!
//! Source of truth is the jar, dumped by the oracle (`tests/golden/26.1/entity_sizes.jsonl.gz`):
//! every registered type's dimensions, the client instance's dimensions per variant (baby, slime
//! size, armor stand small / marker) and pose, whether it is living, how it follows position
//! packets (interpolation steps), whether it is pickable, and the synched data index that selects
//! each variant. minecraft-data (`pc/26.1/entities.json`) supplies display names and categories;
//! ids and names must agree with the jar (else generation fails).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;

use crate::emit::{f32l, header, lit, name_index};
use crate::generate::oracle_lines;
use crate::model::{self, Paths};

type R<T> = Result<T, String>;

/// Oracle entity size dump.
pub const ORACLE_ENTITIES: &str = "tests/golden/26.1/entity_sizes.jsonl.gz";

/// `LivingEntity.SLEEPING_DIMENSIONS` (0.2 x 0.2, eye 0.2): every living type's sleeping pose.
const SLEEPING: [u32; 3] = [0x3e4c_cccd, 0x3e4c_cccd, 0x3e4c_cccd];

/// Types whose client box is not their centred dimensions box: hanging entities (box from facing /
/// variant), shulkers (attach face + peek), interactions (width / height metadata), wind charges
/// (box lowered by their own `makeBoundingBox`).
const NO_BOX: &[&str] = &[
    "wind_charge",
    "breeze_wind_charge",
    "item_frame",
    "glow_item_frame",
    "painting",
    "leash_knot",
    "shulker",
    "interaction",
];

/// `Pose` names by id.
const POSES: [&str; 18] = [
    "standing", "fall_flying", "sleeping", "swimming", "spin_attack", "crouching", "long_jumping",
    "dying", "croaking", "using_tongue", "sitting", "roaring", "sniffing", "emerging", "digging",
    "sliding", "shooting", "inhaling",
];

fn hex(v: &Value) -> R<u32> {
    let h = v
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("not a [dec, hex] float: {v}"))?;
    u32::from_str_radix(h, 16).map_err(|e| e.to_string())
}

fn dims(v: &Value) -> R<[u32; 3]> {
    Ok([hex(&v["width"])?, hex(&v["height"])?, hex(&v["eye_height"])?])
}

fn dims_lit(d: &[u32; 3]) -> String {
    format!(
        "EntityDimensions {{ width: {}, height: {}, eye_height: {} }}",
        f32l(f32::from_bits(d[0])),
        f32l(f32::from_bits(d[1])),
        f32l(f32::from_bits(d[2]))
    )
}

#[derive(Default)]
struct Type {
    name: String,
    registered: [u32; 3],
    fixed: bool,
    living: bool,
    pickable: bool,
    interpolation: i64,
    margin: u32,
    mob: bool,
    /// Poses (by id) whose dimensions are fixed (not scaled), from the default instance.
    fixed_poses: u32,
    max_head_y_rot: u64,
    default_body: bool,
    /// `(variant, dims)`, default first.
    variants: Vec<(String, [u32; 3])>,
    /// `(name, dims)`: poses whose dims differ from their variant's.
    poses: Vec<(String, [u32; 3])>,
    baby: Option<u64>,
    size: Option<u64>,
    flags: Option<u64>,
    measured: bool,
}

/// `entities.rs` for 26.1.
pub fn generate(p: &Paths, dump: &std::path::Path) -> R<String> {
    let lines = oracle_lines(dump)?;
    let mut md = model::entities(&p.json("entities")?)?;
    md.sort_by_key(|e| e.id);
    let mut types: BTreeMap<u64, Type> = BTreeMap::new();
    let mut by_name: BTreeMap<String, u64> = BTreeMap::new();
    for v in &lines {
        let id = v["id"].as_u64().ok_or("entity id")?;
        let raw = v["name"].as_str().ok_or("entity name")?;
        let name = raw
            .trim_start_matches("minecraft:")
            .trim_end_matches(" (client player)")
            .to_owned();
        match v["type"].as_str() {
            Some("entity_type") => {
                let d = dims(v)?;
                types.insert(
                    id,
                    Type {
                        name: name.clone(),
                        registered: d,
                        fixed: v["fixed"].as_bool().ok_or("fixed")?,
                        interpolation: -1,
                        ..Type::default()
                    },
                );
                by_name.insert(name, id);
            }
            Some("entity_size") => {
                let t = types.get_mut(&id).ok_or_else(|| format!("{name}: size before type"))?;
                if v.get("error").is_some() {
                    return Err(format!("{name}: variant error {}", v["error"]));
                }
                let variant = v["variant"].as_str().ok_or("variant")?.to_owned();
                let d = dims(v)?;
                if variant == "default" {
                    t.measured = true;
                    t.living = v["living"].as_bool().ok_or("living")?;
                    t.pickable = v["pickable"].as_bool().ok_or("pickable")?;
                    t.interpolation = v["interpolation"].as_i64().ok_or("interpolation")?;
                    t.margin = hex(&v["targeting_margin"])?;
                    t.mob = v["mob"].as_bool().unwrap_or(false);
                    if t.mob {
                        t.max_head_y_rot = v["max_head_y_rot"].as_u64().ok_or("max_head_y_rot")?;
                        t.default_body = v["body_control"] == "BodyRotationControl";
                    }
                } else {
                    for m in v["meta"].as_array().ok_or("meta")? {
                        let (idx, ser) = (m[0].as_u64().ok_or("index")?, m[1].as_u64().ok_or("ser")?);
                        let slot = match (variant.as_str(), ser) {
                            ("baby", 8) => &mut t.baby,
                            (s, 1) if s.starts_with("size_") => &mut t.size,
                            ("small" | "marker" | "small_marker", 0) => &mut t.flags,
                            _ => continue, // health follows the slime size; not a size key
                        };
                        if slot.is_some_and(|x| x != idx) {
                            return Err(format!("{name} {variant}: two size indices"));
                        }
                        *slot = Some(idx);
                    }
                }
                if t.variants.iter().any(|(n, _)| *n == variant) {
                    return Err(format!("{name}: duplicate variant {variant}"));
                }
                for (pose, pd) in v["poses"].as_object().ok_or("poses")? {
                    let a = pd.as_array().ok_or("pose dims")?;
                    let pi = POSES
                        .iter()
                        .position(|p| p == pose)
                        .ok_or_else(|| format!("unknown pose {pose}"))?;
                    if variant == "default" && a[3].as_bool().ok_or("pose fixed")? {
                        t.fixed_poses |= 1 << pi;
                    }
                    let pd = [hex(&a[0])?, hex(&a[1])?, hex(&a[2])?];
                    if pd == d || (pose == "sleeping" && t.living && pd == SLEEPING) {
                        continue;
                    }
                    let key = if variant == "default" {
                        pose.clone()
                    } else {
                        format!("{variant}/{pose}")
                    };
                    t.poses.push((key, pd));
                }
                t.variants.push((variant, d));
            }
            other => return Err(format!("unknown entity line {other:?}")),
        }
    }
    if md.len() != types.len() {
        return Err(format!(
            "26.1 entities: minecraft-data has {}, the jar {}",
            md.len(),
            types.len()
        ));
    }
    for (i, (e, (id, t))) in md.iter().zip(&types).enumerate() {
        if u64::from(e.id) != *id || e.name != t.name || *id != i as u64 {
            return Err(format!(
                "26.1 entity {id}: minecraft-data {} {}, jar {}",
                e.id, e.name, t.name
            ));
        }
    }

    let mut out = header(
        "Minecraft 26.1 entity types (jar facts from the oracle).",
        &[
            format!("{}/entities.json", p.dir("entities")?),
            ORACLE_ENTITIES.to_owned(),
        ],
    );
    out.push_str(
        "use crate::common::{EntityDimensions, EntityInfo, EntityKind, EntityVariant, VanillaEntitySize, VariantApplied};\nuse crate::v26_1::entities::{EntityTypeData, type_flags};\n\n",
    );
    let _ = writeln!(
        out,
        "/// By id.\npub(crate) static ENTITIES: [EntityInfo; {}] = [",
        md.len()
    );
    for (e, t) in md.iter().zip(types.values()) {
        let kind = if t.name == "player" {
            "Other"
        } else if t.living {
            "Mob"
        } else {
            "Object"
        };
        let _ = writeln!(
            out,
            "    EntityInfo {{ id: {}, name: {}, #[cfg(feature = \"display-names\")] display_name: {}, kind: EntityKind::{kind}, category: {}, width: Some({}), height: Some({}) }},",
            e.id,
            lit(&e.name),
            lit(&e.display_name),
            lit(&e.category),
            f32l(f32::from_bits(t.registered[0])),
            f32l(f32::from_bits(t.registered[1])),
        );
    }
    out.push_str("];\n\n/// Name -> id.\n");
    name_index(
        &mut out,
        "pub(crate) static ENTITY_NAMES",
        "u16",
        md.iter().map(|e| (e.name.clone(), u32::from(e.id))).collect(),
    );

    // Sizes, sorted by name.
    let mut sized: Vec<(&u64, &Type)> = types.iter().filter(|(_, t)| t.measured).collect();
    sized.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    let mut size_index: BTreeMap<u64, usize> = BTreeMap::new();
    let _ = writeln!(
        out,
        "\n/// Client instance dimensions measured by the oracle, sorted by name. `poses` lists only\n/// the poses whose dimensions differ from their variant's (`\"sitting\"`; other variants as\n/// `\"baby/sitting\"`); a living entity's sleeping pose is always `LIVING_SLEEPING`.\npub(crate) static VANILLA_ENTITY_SIZES: [VanillaEntitySize; {}] = [",
        sized.len()
    );
    for (i, (id, t)) in sized.iter().enumerate() {
        size_index.insert(**id, i);
        let vars: Vec<String> = t
            .variants
            .iter()
            .map(|(n, d)| {
                format!(
                    "EntityVariant {{ name: {}, dimensions: {}, applied: VariantApplied::{} }}",
                    lit(n),
                    dims_lit(d),
                    if n == "default" { "Spawn" } else { "Metadata" }
                )
            })
            .collect();
        let poses: Vec<String> = t
            .poses
            .iter()
            .map(|(n, d)| format!("({}, {})", lit(n), dims_lit(d)))
            .collect();
        let _ = writeln!(
            out,
            "    VanillaEntitySize {{ name: {}, registry_id: {id}, spawn: \"\", object_types: &[], targeting_margin: {}, fixed: {}, registered: Some(({}, {})), variants: &[{}], poses: &[{}] }},",
            lit(&t.name),
            f32l(f32::from_bits(t.margin)),
            t.fixed,
            f32l(f32::from_bits(t.registered[0])),
            f32l(f32::from_bits(t.registered[1])),
            vars.join(", "),
            poses.join(", ")
        );
    }
    out.push_str("];\n");

    let _ = writeln!(
        out,
        "\n/// Per-type client facts, by id.\npub(crate) static TYPES: [EntityTypeData; {}] = [",
        types.len()
    );
    let opt = |x: Option<u64>| x.map_or("None".to_owned(), |v| format!("Some({v})"));
    for (id, t) in &types {
        let mut flags = Vec::new();
        if t.living {
            flags.push("type_flags::LIVING");
        }
        if t.pickable {
            flags.push("type_flags::PICKABLE");
        }
        if NO_BOX.contains(&t.name.as_str()) {
            flags.push("type_flags::NO_BOX");
        }
        if t.fixed {
            flags.push("type_flags::FIXED");
        }
        if t.mob {
            flags.push("type_flags::MOB");
        }
        if t.default_body {
            flags.push("type_flags::DEFAULT_BODY");
        }
        let flags = if flags.is_empty() { "0".to_owned() } else { flags.join(" | ") };
        let interp = if t.interpolation < 0 {
            "None".to_owned()
        } else {
            format!("Some({})", t.interpolation)
        };
        let size = size_index
            .get(id)
            .map_or("None".to_owned(), |i| format!("Some(&VANILLA_ENTITY_SIZES[{i}])"));
        let _ = writeln!(
            out,
            "    EntityTypeData {{ id: {id}, name: {}, flags: {flags}, interpolation: {interp}, max_head_y_rot: {}, fixed_poses: {:#x}, baby_meta: {}, size_meta: {}, flags_meta: {}, size: {size} }},",
            lit(&t.name),
            t.max_head_y_rot,
            t.fixed_poses,
            opt(t.baby),
            opt(t.size),
            opt(t.flags),
        );
    }
    out.push_str("];\n");
    for n in NO_BOX {
        if !by_name.contains_key(*n) {
            return Err(format!("26.1: no entity type {n}"));
        }
    }
    Ok(out)
}
