//! Typed view of the minecraft-data JSON files we consume, plus `dataPaths.json` resolution.
//! All fields are validated here so the emitters can assume clean data.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub type R<T> = Result<T, String>;

pub fn load(path: &Path) -> R<Value> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Resolves `<kind>.json` for one version through `dataPaths.json`.
pub struct Paths {
    root: PathBuf,
    map: BTreeMap<String, String>,
    pub version: String,
    /// Files given in memory instead of read (versions minecraft-data has no data for; see
    /// `v26_2::synthesise`).
    overrides: BTreeMap<String, Value>,
}

impl Paths {
    pub fn new(data_root: &Path, version: &str) -> R<Paths> {
        let dp = load(&data_root.join("dataPaths.json"))?;
        let entry = dp["pc"][version]
            .as_object()
            .ok_or_else(|| format!("dataPaths.json has no pc/{version}"))?;
        let map = entry
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
            .collect();
        Ok(Paths {
            root: data_root.to_owned(),
            map,
            version: version.into(),
            overrides: BTreeMap::new(),
        })
    }

    /// These paths with `files` served from memory instead of minecraft-data.
    pub fn with_overrides(mut self, files: BTreeMap<String, Value>) -> Paths {
        self.overrides = files;
        self
    }

    /// The `pc/<x>` directory a kind resolves to (for provenance comments).
    pub fn dir(&self, kind: &str) -> R<&str> {
        self.map
            .get(kind)
            .map(String::as_str)
            .ok_or_else(|| format!("pc/{} has no {kind} entry", self.version))
    }

    pub fn json(&self, kind: &str) -> R<Value> {
        if let Some(v) = self.overrides.get(kind) {
            return Ok(v.clone());
        }
        let dir = self.dir(kind)?;
        load(&self.root.join(dir).join(format!("{kind}.json")))
    }
}

fn s(v: &Value, k: &str) -> R<String> {
    v[k].as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("missing string {k} in {v}"))
}
fn u(v: &Value, k: &str) -> R<u64> {
    v[k].as_u64()
        .ok_or_else(|| format!("missing uint {k} in {v}"))
}
fn f(v: &Value, k: &str) -> R<f64> {
    v[k].as_f64()
        .ok_or_else(|| format!("missing number {k} in {v}"))
}
fn of(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}
fn arr<'a>(v: &'a Value, what: &str) -> R<&'a Vec<Value>> {
    v.as_array()
        .ok_or_else(|| format!("{what}: expected array"))
}

#[derive(Clone, Debug)]
pub struct Prop {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Drop {
    pub item: u16,
    pub meta: Option<u8>,
    pub min: u8,
    pub max: u8,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub id: u16,
    pub name: String,
    pub display_name: String,
    pub hardness: Option<f64>,
    pub resistance: f64,
    pub stack_size: u8,
    pub diggable: bool,
    pub transparent: bool,
    pub emit_light: u8,
    pub filter_light: u8,
    pub bbox_empty: bool,
    pub material: Option<String>,
    pub harvest_tools: Vec<u16>,
    pub drops: Vec<Drop>,
    // Flattened (1.13+) only.
    pub min_state: u32,
    pub max_state: u32,
    pub default_state: u32,
    pub props: Vec<Prop>,
}

pub fn blocks(v: &Value) -> R<Vec<Block>> {
    let mut out = Vec::new();
    for b in arr(v, "blocks")? {
        let mut harvest_tools: Vec<u16> = match b.get("harvestTools").and_then(Value::as_object) {
            Some(m) => m
                .keys()
                .map(|k| {
                    k.parse::<u16>()
                        .map_err(|e| format!("harvestTools key {k}: {e}"))
                })
                .collect::<R<_>>()?,
            None => Vec::new(),
        };
        harvest_tools.sort_unstable();
        let mut drops = Vec::new();
        for d in b
            .get("drops")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (item, meta, min, max) = if let Some(id) = d.as_u64() {
                (id, None, 1, 1)
            } else {
                let dd = &d["drop"];
                let (item, meta) = if let Some(id) = dd.as_u64() {
                    (id, None)
                } else {
                    (u(dd, "id")?, Some(u(dd, "metadata")? as u8))
                };
                let min = d.get("minCount").and_then(Value::as_u64).unwrap_or(1);
                let max = d.get("maxCount").and_then(Value::as_u64).unwrap_or(min);
                (item, meta, min, max)
            };
            drops.push(Drop {
                item: item as u16,
                meta,
                min: min as u8,
                max: max as u8,
            });
        }
        let mut props = Vec::new();
        for st in b
            .get("states")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let name = s(st, "name")?;
            let n = u(st, "num_values")? as usize;
            let values: Vec<String> = match s(st, "type")?.as_str() {
                // Vanilla BooleanProperty order is [true, false].
                "bool" => vec!["true".into(), "false".into()],
                // minecraft-data lists int values where they do not start at 0 (leaves'
                // distance 1..=7, turtle eggs 1..=4); older files may omit them.
                "int" => match st.get("values").and_then(Value::as_array) {
                    Some(v) => v
                        .iter()
                        .map(|x| x.as_str().map(str::to_owned).ok_or("int value".to_string()))
                        .collect::<R<_>>()?,
                    None => (0..n).map(|i| i.to_string()).collect(),
                },
                "enum" => arr(&st["values"], "values")?
                    .iter()
                    .map(|x| {
                        x.as_str()
                            .map(str::to_owned)
                            .ok_or("enum value".to_string())
                    })
                    .collect::<R<_>>()?,
                t => return Err(format!("unknown property type {t}")),
            };
            if values.len() != n {
                return Err(format!(
                    "{}: property {name} has {} values, num_values {n}",
                    s(b, "name")?,
                    values.len()
                ));
            }
            props.push(Prop { name, values });
        }
        let id = u(b, "id")? as u16;
        out.push(Block {
            id,
            name: s(b, "name")?,
            display_name: s(b, "displayName")?,
            hardness: of(b, "hardness"),
            resistance: f(b, "resistance")?,
            stack_size: u(b, "stackSize")? as u8,
            diggable: b["diggable"].as_bool().unwrap_or(false),
            transparent: b["transparent"].as_bool().unwrap_or(false),
            emit_light: u(b, "emitLight")? as u8,
            filter_light: u(b, "filterLight")? as u8,
            bbox_empty: s(b, "boundingBox")? == "empty",
            material: b.get("material").and_then(Value::as_str).map(str::to_owned),
            harvest_tools,
            drops,
            min_state: b.get("minStateId").and_then(Value::as_u64).unwrap_or(0) as u32,
            max_state: b.get("maxStateId").and_then(Value::as_u64).unwrap_or(0) as u32,
            default_state: b.get("defaultState").and_then(Value::as_u64).unwrap_or(0) as u32,
            props,
        });
    }
    out.sort_by_key(|b| b.id);
    for w in out.windows(2) {
        if w[0].id == w[1].id {
            return Err(format!("duplicate block id {}", w[0].id));
        }
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: u16,
    pub name: String,
    pub display_name: String,
    pub stack_size: u8,
    pub max_durability: Option<u16>,
    pub variations: Vec<(u8, String)>,
}

pub fn items(v: &Value) -> R<Vec<Item>> {
    let mut out = Vec::new();
    for i in arr(v, "items")? {
        let mut variations = Vec::new();
        for var in i
            .get("variations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            variations.push((u(var, "metadata")? as u8, s(var, "displayName")?));
        }
        out.push(Item {
            id: u(i, "id")? as u16,
            name: s(i, "name")?,
            display_name: s(i, "displayName")?,
            stack_size: u(i, "stackSize")? as u8,
            max_durability: i
                .get("maxDurability")
                .and_then(Value::as_u64)
                .map(|d| d as u16),
            variations,
        });
    }
    out.sort_by_key(|i| i.id);
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct Entity {
    pub id: u16,
    pub name: String,
    pub display_name: String,
    pub kind: &'static str,
    pub category: String,
    pub width: Option<f64>,
    pub height: Option<f64>,
}

pub fn entities(v: &Value) -> R<Vec<Entity>> {
    let mut out = Vec::new();
    for e in arr(v, "entities")? {
        let kind = match e["type"].as_str() {
            Some("mob") => "Mob",
            Some("object") => "Object",
            _ => "Other",
        };
        out.push(Entity {
            id: u(e, "id")? as u16,
            name: s(e, "name")?,
            display_name: s(e, "displayName")?,
            kind,
            category: e["category"].as_str().unwrap_or("").to_owned(),
            width: of(e, "width"),
            height: of(e, "height"),
        });
    }
    out.sort_by(|a, b| (a.kind, a.id).cmp(&(b.kind, b.id)));
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct Biome {
    pub id: u16,
    pub name: String,
    pub display_name: String,
    pub category: String,
    pub temperature: f64,
    pub rainfall: f64,
    pub precipitation: String,
    pub dimension: String,
}

pub fn biomes(v: &Value) -> R<Vec<Biome>> {
    let mut out = Vec::new();
    for b in arr(v, "biomes")? {
        out.push(Biome {
            id: u(b, "id")? as u16,
            name: s(b, "name")?,
            display_name: s(b, "displayName")?,
            category: s(b, "category")?,
            temperature: f(b, "temperature")?,
            rainfall: f(b, "rainfall")?,
            precipitation: s(b, "precipitation")?,
            dimension: s(b, "dimension")?,
        });
    }
    out.sort_by_key(|b| b.id);
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct Enchantment {
    pub id: u16,
    pub name: String,
    pub display_name: String,
    pub max_level: u8,
    pub min_cost: (i64, i64),
    pub max_cost: (i64, i64),
    pub exclude: Vec<String>,
    pub category: String,
    pub weight: u8,
    pub treasure_only: bool,
    pub curse: bool,
}

pub fn enchantments(v: &Value) -> R<Vec<Enchantment>> {
    let cost = |c: &Value| -> R<(i64, i64)> {
        Ok((
            c["a"].as_i64().ok_or("cost.a")?,
            c["b"].as_i64().ok_or("cost.b")?,
        ))
    };
    let mut out = Vec::new();
    for e in arr(v, "enchantments")? {
        out.push(Enchantment {
            id: u(e, "id")? as u16,
            name: s(e, "name")?,
            display_name: s(e, "displayName")?,
            max_level: u(e, "maxLevel")? as u8,
            min_cost: cost(&e["minCost"])?,
            max_cost: cost(&e["maxCost"])?,
            exclude: arr(&e["exclude"], "exclude")?
                .iter()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect(),
            category: s(e, "category")?,
            weight: u(e, "weight")? as u8,
            treasure_only: e["treasureOnly"].as_bool().unwrap_or(false),
            curse: e["curse"].as_bool().unwrap_or(false),
        });
    }
    out.sort_by_key(|e| e.id);
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct Food {
    pub source_id: u16,
    pub name: String,
    pub food_points: u8,
    pub saturation: f64,
}

pub fn foods(v: &Value) -> R<Vec<Food>> {
    let mut out = Vec::new();
    for x in arr(v, "foods")? {
        out.push(Food {
            source_id: u(x, "id")? as u16,
            name: s(x, "name")?,
            food_points: u(x, "foodPoints")? as u8,
            saturation: f(x, "saturation")?,
        });
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct Effect {
    pub id: u8,
    pub name: String,
    pub display_name: String,
    pub good: bool,
}

pub fn effects(v: &Value) -> R<Vec<Effect>> {
    let mut out = Vec::new();
    for x in arr(v, "effects")? {
        out.push(Effect {
            id: u(x, "id")? as u8,
            name: s(x, "name")?,
            display_name: s(x, "displayName")?,
            good: x["type"].as_str() == Some("good"),
        });
    }
    out.sort_by_key(|e| e.id);
    Ok(out)
}

/// `materials.json`: group -> [(tool item id, multiplier)].
pub fn materials(v: &Value) -> R<BTreeMap<String, Vec<(u16, f64)>>> {
    let mut out = BTreeMap::new();
    for (k, m) in v.as_object().ok_or("materials: expected object")? {
        let mut tools: Vec<(u16, f64)> = m
            .as_object()
            .ok_or("materials entry")?
            .iter()
            .map(|(id, x)| {
                Ok((
                    id.parse::<u16>().map_err(|e| e.to_string())?,
                    x.as_f64().ok_or("mult")?,
                ))
            })
            .collect::<R<_>>()?;
        tools.sort_by_key(|t| t.0);
        out.insert(k.clone(), tools);
    }
    Ok(out)
}

/// `blockCollisionShapes.json`.
pub struct Shapes {
    /// block name -> one shape id, or one per state (1.8: per meta).
    pub blocks: BTreeMap<String, Vec<u32>>,
    pub shapes: BTreeMap<u32, Vec<[f64; 6]>>,
}

pub fn shapes(v: &Value) -> R<Shapes> {
    let mut blocks = BTreeMap::new();
    for (k, x) in v["blocks"].as_object().ok_or("shapes.blocks")? {
        let ids = if let Some(n) = x.as_u64() {
            vec![n as u32]
        } else {
            arr(x, "shape list")?
                .iter()
                .map(|n| n.as_u64().map(|n| n as u32).ok_or("shape id".to_string()))
                .collect::<R<_>>()?
        };
        blocks.insert(k.clone(), ids);
    }
    let mut shapes = BTreeMap::new();
    for (k, x) in v["shapes"].as_object().ok_or("shapes.shapes")? {
        let id: u32 = k.parse().map_err(|_| format!("shape key {k}"))?;
        let mut boxes = Vec::new();
        for b in arr(x, "boxes")? {
            let b = arr(b, "box")?;
            if b.len() != 6 {
                return Err(format!("shape {k}: box with {} numbers", b.len()));
            }
            let mut a = [0.0; 6];
            for (i, n) in b.iter().enumerate() {
                a[i] = n.as_f64().ok_or("box number")?;
            }
            boxes.push(a);
        }
        shapes.insert(id, boxes);
    }
    Ok(Shapes { blocks, shapes })
}
