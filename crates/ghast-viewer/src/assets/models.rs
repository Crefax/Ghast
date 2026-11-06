//! Builds the page's block data (`data.json`) and texture pack (`pack.bin`) from the 1.8 client's
//! blockstates, block models and textures.
//!
//! - data: `{layers, models, blocks, items, biomes, images}`: texture layers (name, animation
//!   frames, frame time, an optional crop of another image), resolved block models (elements with
//!   faces pointing at layers), per block id its variants (blockstates key → model refs), render
//!   hints, item textures by id (and `"id:meta"`), biome climate, the extra images.
//! - pack: `u32` header length, header JSON `{layers: [[off, len]], images: {name: [off, len]}}`,
//!   then the PNG files back to back (each file once).
//!
//! Chests and signs are block entities in 1.8 without a block model: their models are made here
//! from the entity textures.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use serde_json::Value;

use crate::blocks18::{NO_BLOCKSTATES, Rule, blockstates_file, variants};

/// Numbers as JavaScript prints them: integral values without a fraction (smaller JSON).
mod jsnum {
    use serde::Serializer;

    fn one<S: Serializer>(v: f64, s: S) -> Result<S::Ok, S::Error> {
        if v.fract() == 0.0 && v.abs() < 9.0e15 {
            s.serialize_i64(v as i64)
        } else {
            s.serialize_f64(v)
        }
    }
    pub(super) fn num<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
        one(*v, s)
    }
    pub(super) fn opt<S: Serializer>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => one(*v, s),
            None => s.serialize_none(),
        }
    }
    pub(super) fn vec<S: Serializer>(v: &[f64], s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq as _;
        let mut q = s.serialize_seq(Some(v.len()))?;
        for x in v {
            q.serialize_element(&J(*x))?;
        }
        q.end()
    }
    pub(super) fn opt_vec<S: Serializer>(v: &Option<Vec<f64>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => vec(v, s),
            None => s.serialize_none(),
        }
    }
    struct J(f64);
    impl serde::Serialize for J {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            one(self.0, s)
        }
    }
}

/// The client's files, by path under `assets/minecraft/` (e.g. `models/block/stone.json`).
pub type Files = HashMap<String, Vec<u8>>;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Face {
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "jsnum::opt_vec"
    )]
    pub uv: Option<Vec<f64>>,
    pub t: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cull: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "jsnum::opt")]
    pub r: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tint: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ElementRot {
    #[serde(serialize_with = "jsnum::vec")]
    pub o: Vec<f64>,
    pub axis: String,
    #[serde(serialize_with = "jsnum::num")]
    pub angle: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rescale: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Element {
    #[serde(serialize_with = "jsnum::vec")]
    pub from: Vec<f64>,
    #[serde(serialize_with = "jsnum::vec")]
    pub to: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rot: Option<ElementRot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shade: Option<bool>,
    pub faces: BTreeMap<String, Face>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Model {
    pub ao: bool,
    pub el: Vec<Element>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p: Option<usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VariantRef {
    pub m: usize,
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "jsnum::opt")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "jsnum::opt")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uvlock: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "jsnum::opt")]
    pub w: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BlockData {
    pub name: String,
    pub variants: BTreeMap<String, Vec<VariantRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fluid: Option<&'static str>,
    #[serde(rename = "solidHint")]
    pub solid_hint: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translucent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collides: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LayerInfo {
    pub name: String,
    pub frames: u32,
    pub frametime: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop: Option<[u32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ItemTex {
    pub name: String,
    pub layer: usize,
}

/// The page's block data.
#[derive(Debug, Clone, Serialize)]
pub struct ModelData {
    pub layers: Vec<LayerInfo>,
    pub models: Vec<Model>,
    pub blocks: Vec<Option<BlockData>>,
    pub items: BTreeMap<String, ItemTex>,
    pub biomes: Vec<(u16, f32, f32)>,
    pub images: Vec<String>,
}

/// Translucent pass (sorted, blended).
const TRANSLUCENT: &[&str] = &[
    "water",
    "flowing_water",
    "ice",
    "stained_glass",
    "stained_glass_pane",
    "slime",
    "portal",
];

fn png_size(b: &[u8]) -> (u32, u32) {
    crate::skins::png_size(b).unwrap_or((16, 16))
}

struct Builder<'a> {
    files: &'a Files,
    raw_models: HashMap<String, Value>,
    layers: Vec<LayerInfo>,
    layer_index: HashMap<String, usize>,
    models: Vec<Model>,
    model_index: HashMap<String, Option<usize>>,
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default()
}

impl Builder<'_> {
    fn png(&self, tex: &str) -> Option<&Vec<u8>> {
        self.files.get(&format!("textures/{tex}.png"))
    }

    fn layer(&mut self, tex: &str) -> usize {
        if let Some(&i) = self.layer_index.get(tex) {
            return i;
        }
        let Some(png) = self.png(tex) else {
            return self.layer("blocks/stone");
        };
        let (w, h) = png_size(png);
        let frametime = self
            .files
            .get(&format!("textures/{tex}.png.mcmeta"))
            .and_then(|m| serde_json::from_slice::<Value>(m).ok())
            .and_then(|m| m["animation"]["frametime"].as_f64())
            .map_or(1, |f| f.max(1.0) as u32);
        let i = self.layers.len();
        self.layers.push(LayerInfo {
            name: tex.to_string(),
            frames: if h > w && w > 0 { h / w } else { 1 },
            frametime,
            crop: None,
            src: None,
        });
        self.layer_index.insert(tex.to_string(), i);
        i
    }

    fn crop_layers(&mut self, src: &str, crops: &[(&str, [u32; 4])]) {
        for (k, crop) in crops {
            let name = format!("{src}#{k}");
            self.layer_index.insert(name.clone(), self.layers.len());
            self.layers.push(LayerInfo {
                name,
                frames: 1,
                frametime: 1,
                crop: Some(*crop),
                src: Some(src.to_string()),
            });
        }
    }

    fn named(&self, n: &str) -> usize {
        self.layer_index.get(n).copied().unwrap_or(0)
    }

    fn resolve(&mut self, name: &str) -> Option<usize> {
        let n = name.strip_prefix("block/").unwrap_or(name).to_string();
        if let Some(i) = self.model_index.get(&n) {
            return *i;
        }
        // the parent chain: textures merge child over parent; elements from the nearest model
        let mut chain: Vec<Value> = Vec::new();
        let mut cur = Some(n.clone());
        for _ in 0..16 {
            let Some(c) = cur.take() else { break };
            let Some(m) = self.raw_models.get(c.strip_prefix("block/").unwrap_or(&c)) else {
                break;
            };
            cur = m["parent"].as_str().map(str::to_string);
            chain.push(m.clone());
        }
        if chain.is_empty() {
            self.model_index.insert(n, None);
            return None;
        }
        let mut textures: HashMap<String, String> = HashMap::new();
        let mut elements: Option<Value> = None;
        let mut ao: Option<bool> = None;
        for m in chain.iter().rev() {
            if let Some(t) = m["textures"].as_object() {
                for (k, v) in t {
                    if let Some(v) = v.as_str() {
                        textures.insert(k.clone(), v.to_string());
                    }
                }
            }
            if m.get("elements").is_some_and(Value::is_array) {
                elements = Some(m["elements"].clone());
            }
            if let Some(a) = m["ambientocclusion"].as_bool() {
                ao = Some(a);
            }
        }
        let Some(elements) = elements else {
            self.model_index.insert(n, None);
            return None;
        };
        let tex = |r: &str| -> Option<String> {
            let mut r = r.to_string();
            for _ in 0..8 {
                match r.strip_prefix('#') {
                    Some(k) => r = textures.get(k)?.clone(),
                    None => break,
                }
            }
            (!r.starts_with('#')).then_some(r)
        };
        let mut el = Vec::new();
        for e in elements.as_array().into_iter().flatten() {
            let mut faces = BTreeMap::new();
            for (d, f) in e["faces"].as_object().into_iter().flatten() {
                let Some(t) = f["texture"].as_str().and_then(tex) else {
                    continue;
                };
                let t = self.layer(&t);
                let cull = f["cullface"].as_str().map(|c| {
                    if c == "bottom" {
                        "down".to_string()
                    } else {
                        c.to_string()
                    }
                });
                faces.insert(
                    d.clone(),
                    Face {
                        uv: f.get("uv").map(nums),
                        t,
                        cull,
                        r: f["rotation"].as_f64().filter(|r| *r != 0.0),
                        tint: f["tintindex"].as_i64(),
                    },
                );
            }
            let rot = e
                .get("rotation")
                .filter(|r| r.is_object())
                .map(|r| ElementRot {
                    o: nums(&r["origin"]),
                    axis: r["axis"].as_str().unwrap_or("y").to_string(),
                    angle: r["angle"].as_f64().unwrap_or(0.0),
                    rescale: r["rescale"].as_bool().filter(|b| *b),
                });
            el.push(Element {
                from: nums(&e["from"]),
                to: nums(&e["to"]),
                rot,
                shade: (e["shade"].as_bool() == Some(false)).then_some(false),
                faces,
            });
        }
        let p = textures
            .contains_key("particle")
            .then(|| tex("#particle"))
            .flatten()
            .map(|t| self.layer(&t));
        let i = self.models.len();
        self.models.push(Model {
            ao: ao != Some(false),
            el,
            p,
        });
        self.model_index.insert(n, Some(i));
        Some(i)
    }

    fn refs(&mut self, v: &Value, rename: &dyn Fn(&str) -> String) -> Vec<VariantRef> {
        let list: Vec<&Value> = match v {
            Value::Array(a) => a.iter().collect(),
            other => vec![other],
        };
        let mut out = Vec::new();
        for r in list {
            let Some(model) = r["model"].as_str() else {
                continue;
            };
            let Some(m) = self.resolve(&rename(model)) else {
                continue;
            };
            let nz = |k: &str| r[k].as_f64().filter(|x| *x != 0.0);
            out.push(VariantRef {
                m,
                x: nz("x"),
                y: nz("y"),
                uvlock: r["uvlock"].as_bool().filter(|b| *b),
                w: r["weight"].as_f64().filter(|w| *w != 1.0),
            });
        }
        out
    }
}

fn face(t: usize) -> Face {
    Face {
        uv: Some(vec![0.0, 0.0, 16.0, 16.0]),
        t,
        cull: None,
        r: None,
        tint: None,
    }
}

fn element(from: [f64; 3], to: [f64; 3], faces: &[(&str, usize)]) -> Element {
    Element {
        from: from.to_vec(),
        to: to.to_vec(),
        rot: None,
        shade: None,
        faces: faces
            .iter()
            .map(|(d, t)| ((*d).to_string(), face(*t)))
            .collect(),
    }
}

fn y_rot(y: f64) -> Option<f64> {
    (y != 0.0).then_some(y)
}

fn vref(m: usize, y: f64) -> VariantRef {
    VariantRef {
        m,
        x: None,
        y: y_rot(y),
        uvlock: None,
        w: None,
    }
}

/// Builds the data from the client's files; `images` are the extra images the page needs.
/// Returns the data and, per layer source / image name, its texture path.
#[must_use]
pub fn build(files: &Files, images: &[String]) -> (ModelData, HashMap<String, String>) {
    let raw_models: HashMap<String, Value> = files
        .iter()
        .filter_map(|(k, v)| {
            let n = k.strip_prefix("models/block/")?.strip_suffix(".json")?;
            Some((n.to_string(), serde_json::from_slice(v).ok()?))
        })
        .collect();
    let mut b = Builder {
        files,
        raw_models,
        layers: Vec::new(),
        layer_index: HashMap::new(),
        models: Vec::new(),
        model_index: HashMap::new(),
    };
    let mut sources: HashMap<String, String> = HashMap::new();

    // chests: lid, base and latch cropped from the entity texture
    let chests = [
        ("chest", "entity/chest/normal"),
        ("trapped_chest", "entity/chest/trapped"),
        ("ender_chest", "entity/chest/ender"),
    ];
    let mut chest_models: HashMap<&str, usize> = HashMap::new();
    let mut double_models: HashMap<&str, (usize, usize)> = HashMap::new();
    for (block, src) in chests {
        b.crop_layers(
            src,
            &[
                ("lid_top", [14, 0, 14, 14]),
                ("lid_front", [14, 14, 14, 5]),
                ("lid_side", [0, 14, 14, 5]),
                ("lid_back", [42, 14, 14, 5]),
                ("base_front", [14, 33, 14, 10]),
                ("base_side", [0, 33, 14, 10]),
                ("base_back", [42, 33, 14, 10]),
                ("base_bottom", [28, 19, 14, 14]),
                ("latch", [1, 1, 2, 4]),
            ],
        );
        sources.insert(src.to_string(), src.to_string());
        let l = |k: &str| b.named(&format!("{src}#{k}"));
        let m = Model {
            ao: false,
            el: vec![
                element(
                    [1.0, 0.0, 1.0],
                    [15.0, 10.0, 15.0],
                    &[
                        ("down", l("base_bottom")),
                        ("north", l("base_front")),
                        ("south", l("base_back")),
                        ("west", l("base_side")),
                        ("east", l("base_side")),
                    ],
                ),
                element(
                    [1.0, 9.0, 1.0],
                    [15.0, 14.0, 15.0],
                    &[
                        ("up", l("lid_top")),
                        ("down", l("base_bottom")),
                        ("north", l("lid_front")),
                        ("south", l("lid_back")),
                        ("west", l("lid_side")),
                        ("east", l("lid_side")),
                    ],
                ),
                element(
                    [7.0, 7.0, 0.0],
                    [9.0, 11.0, 1.0],
                    &[
                        ("north", l("latch")),
                        ("up", l("latch")),
                        ("down", l("latch")),
                        ("west", l("latch")),
                        ("east", l("latch")),
                    ],
                ),
            ],
            p: None,
        };
        chest_models.insert(block, b.models.len());
        b.models.push(m);
        let dsrc = format!("{src}_double");
        if block == "ender_chest" || b.png(&dsrc).is_none() {
            continue;
        }
        // a double chest (128×64, 30 px wide lid and base) as two halves
        let mut crops: Vec<(String, [u32; 4])> = Vec::new();
        for (k, [u, v, w, h]) in [
            ("lid_top", [14, 0, 30, 14]),
            ("lid_front", [14, 14, 30, 5]),
            ("lid_back", [58, 14, 30, 5]),
            ("base_front", [14, 33, 30, 10]),
            ("base_back", [58, 33, 30, 10]),
            ("base_bottom", [44, 19, 30, 14]),
        ] {
            crops.push((format!("{k}_l"), [u, v, w / 2, h]));
            crops.push((format!("{k}_r"), [u + w / 2, v, w / 2, h]));
        }
        for (k, c) in [
            ("lid_side_w", [0, 14, 14, 5]),
            ("lid_side_e", [44, 14, 14, 5]),
            ("base_side_w", [0, 33, 14, 10]),
            ("base_side_e", [44, 33, 14, 10]),
            ("latch_l", [1, 1, 1, 4]),
            ("latch_r", [2, 1, 1, 4]),
        ] {
            crops.push((k.to_string(), c));
        }
        let crop_refs: Vec<(&str, [u32; 4])> =
            crops.iter().map(|(k, c)| (k.as_str(), *c)).collect();
        b.crop_layers(&dsrc, &crop_refs);
        sources.insert(dsrc.clone(), dsrc.clone());
        let half = |b: &Builder<'_>, left: bool| -> Model {
            let l = |k: &str| b.named(&format!("{dsrc}#{k}"));
            let (fr, bk) = if left { ("_r", "_l") } else { ("_l", "_r") };
            let (x0, x1) = if left { (1.0, 16.0) } else { (0.0, 15.0) };
            let side = if left {
                ("east", l("base_side_e"))
            } else {
                ("west", l("base_side_w"))
            };
            let lid_side = if left {
                ("east", l("lid_side_e"))
            } else {
                ("west", l("lid_side_w"))
            };
            let latch: Vec<(&str, usize)> = if left {
                vec![
                    ("north", l("latch_l")),
                    ("up", l("latch_l")),
                    ("down", l("latch_l")),
                    ("west", l("latch_l")),
                ]
            } else {
                vec![
                    ("north", l("latch_r")),
                    ("up", l("latch_l")),
                    ("down", l("latch_l")),
                    ("east", l("latch_r")),
                ]
            };
            Model {
                ao: false,
                el: vec![
                    element(
                        [x0, 0.0, 1.0],
                        [x1, 10.0, 15.0],
                        &[
                            ("down", l(&format!("base_bottom{bk}"))),
                            ("north", l(&format!("base_front{fr}"))),
                            ("south", l(&format!("base_back{bk}"))),
                            side,
                        ],
                    ),
                    element(
                        [x0, 9.0, 1.0],
                        [x1, 14.0, 15.0],
                        &[
                            ("up", l(&format!("lid_top{bk}"))),
                            ("down", l(&format!("base_bottom{bk}"))),
                            ("north", l(&format!("lid_front{fr}"))),
                            ("south", l(&format!("lid_back{bk}"))),
                            lid_side,
                        ],
                    ),
                    element(
                        if left {
                            [15.0, 7.0, 0.0]
                        } else {
                            [0.0, 7.0, 0.0]
                        },
                        if left {
                            [16.0, 11.0, 1.0]
                        } else {
                            [1.0, 11.0, 1.0]
                        },
                        &latch,
                    ),
                ],
                p: None,
            }
        };
        let (lm, rm) = (half(&b, true), half(&b, false));
        double_models.insert(block, (b.models.len(), b.models.len() + 1));
        b.models.push(lm);
        b.models.push(rm);
    }

    // signs: standing (16 rotations = y steps of 90° × element turns of 0/22.5/45/67.5°) and wall
    let sign = "entity/sign";
    b.crop_layers(
        sign,
        &[
            ("front", [2, 2, 24, 12]),
            ("back", [28, 2, 24, 12]),
            ("top", [2, 0, 24, 2]),
            ("bottom", [26, 0, 24, 2]),
            ("side", [0, 2, 2, 12]),
            ("post", [0, 16, 2, 14]),
        ],
    );
    sources.insert(sign.into(), sign.into());
    let sl = |b: &Builder<'_>, k: &str| b.named(&format!("{sign}#{k}"));
    let board =
        |b: &Builder<'_>, from: [f64; 3], to: [f64; 3], front: &str, back: &str, angle: f64| {
            let mut e = element(
                from,
                to,
                &[
                    (front, sl(b, "front")),
                    (back, sl(b, "back")),
                    ("up", sl(b, "top")),
                    ("down", sl(b, "bottom")),
                    ("west", sl(b, "side")),
                    ("east", sl(b, "side")),
                ],
            );
            if angle != 0.0 {
                e.rot = Some(ElementRot {
                    o: vec![8.0, 8.0, 8.0],
                    axis: "y".into(),
                    angle,
                    rescale: None,
                });
            }
            e
        };
    let mut turn = Vec::new();
    for angle in [0.0, -22.5, -45.0, 22.5] {
        let mut post = element(
            [7.33, 0.0, 7.33],
            [8.67, 9.33, 8.67],
            &[
                ("north", sl(&b, "post")),
                ("south", sl(&b, "post")),
                ("west", sl(&b, "post")),
                ("east", sl(&b, "post")),
                ("up", sl(&b, "post")),
                ("down", sl(&b, "post")),
            ],
        );
        if angle != 0.0 {
            post.rot = Some(ElementRot {
                o: vec![8.0, 8.0, 8.0],
                axis: "y".into(),
                angle,
                rescale: None,
            });
        }
        let m = Model {
            ao: false,
            el: vec![
                board(
                    &b,
                    [0.0, 9.33, 7.33],
                    [16.0, 17.33, 8.67],
                    "south",
                    "north",
                    angle,
                ),
                post,
            ],
            p: None,
        };
        turn.push(b.models.len());
        b.models.push(m);
    }
    let mut standing_sign = BTreeMap::new();
    for r in 0..16usize {
        let k = r % 4;
        let q = r / 4 + usize::from(k == 3);
        standing_sign.insert(
            format!("rotation={r}"),
            vec![vref(turn[k], ((q % 4) * 90) as f64)],
        );
    }
    let wall_sign = b.models.len();
    let wm = Model {
        ao: false,
        el: vec![board(
            &b,
            [0.0, 4.33, 14.67],
            [16.0, 12.33, 16.0],
            "north",
            "south",
            0.0,
        )],
        p: None,
    };
    b.models.push(wm);

    let mut blocks: Vec<Option<BlockData>> = vec![None; 256];
    for blk in ghast_data::v1_8::Block::all() {
        let info = blk.info();
        let id = usize::from(blk.id());
        if id > 255 {
            continue;
        }
        let name = info.name;
        let mut vs: BTreeMap<String, Vec<VariantRef>> = BTreeMap::new();
        if name == "standing_sign" {
            vs = standing_sign.clone();
        } else if name == "wall_sign" {
            for (f, y) in [
                ("north", 0.0),
                ("south", 180.0),
                ("west", 270.0),
                ("east", 90.0),
            ] {
                vs.insert(format!("facing={f}"), vec![vref(wall_sign, y)]);
            }
        } else if let Some(&m) = chest_models.get(name) {
            for (f, y) in [
                ("north", 0.0),
                ("south", 180.0),
                ("west", 270.0),
                ("east", 90.0),
            ] {
                vs.insert(format!("facing={f}"), vec![vref(m, y)]);
                if let Some(&(l, r)) = double_models.get(name) {
                    vs.insert(format!("facing={f},type=left"), vec![vref(l, y)]);
                    vs.insert(format!("facing={f},type=right"), vec![vref(r, y)]);
                }
            }
        } else if !NO_BLOCKSTATES.contains(&name) {
            let file = blockstates_file(name).unwrap_or(name);
            let raw = files
                .get(&format!("blockstates/{file}.json"))
                .and_then(|f| serde_json::from_slice::<Value>(f).ok());
            if let Some(raw_vars) = raw.as_ref().and_then(|r| r["variants"].as_object()) {
                if let Some(rules) = variants(name) {
                    for (i, rule) in rules.iter().enumerate() {
                        match rule {
                            Rule::Model(m) => {
                                let r =
                                    b.refs(&serde_json::json!({ "model": m }), &|m| m.to_string());
                                if !r.is_empty() {
                                    vs.insert(format!("v={i},normal"), r);
                                }
                            }
                            Rule::Rename(rn) => {
                                for (k, val) in raw_vars {
                                    let r = b.refs(val, &|m| rn.apply(m));
                                    if !r.is_empty() {
                                        vs.insert(format!("v={i},{k}"), r);
                                    }
                                }
                            }
                        }
                    }
                }
                for (k, val) in raw_vars {
                    let r = b.refs(val, &|m| m.to_string());
                    if !r.is_empty() {
                        vs.insert(k.clone(), r);
                    }
                }
            }
        }
        let fluid = match name {
            "water" | "flowing_water" => Some("water"),
            "lava" | "flowing_lava" => Some("lava"),
            _ => None,
        };
        if let Some(f) = fluid {
            b.layer(&format!("blocks/{f}_still"));
            b.layer(&format!("blocks/{f}_flow"));
        }
        let boxed = info.bounding_box == ghast_data::BoundingBox::Block;
        blocks[id] = Some(BlockData {
            name: name.to_string(),
            variants: vs,
            fluid,
            solid_hint: boxed && !info.transparent,
            translucent: TRANSLUCENT.contains(&name).then_some(true),
            collides: boxed.then_some(true),
        });
    }

    // items: their own texture (the item model's layer0), else the block's model on the page
    let mut items: BTreeMap<String, ItemTex> = BTreeMap::new();
    for it in ghast_data::v1_8::Item::all() {
        let info = it.info();
        if info.id < 256 {
            continue;
        }
        let model = match info.name {
            "wooden_door" => "oak_door",
            "fish" => "pufferfish",
            "cooked_fish" => "cooked_salmon",
            "dye" => "dye_black",
            "potion" => "bottle_drinkable",
            "skull" => "skull_char",
            n => n,
        };
        let from_model = files
            .get(&format!("models/item/{model}.json"))
            .and_then(|f| serde_json::from_slice::<Value>(f).ok())
            .and_then(|m| m["textures"]["layer0"].as_str().map(str::to_string));
        let cands = [from_model, Some(format!("items/{}", info.name))];
        if let Some(found) = cands.into_iter().flatten().find(|c| b.png(c).is_some()) {
            let layer = b.layer(&found);
            items.insert(
                info.id.to_string(),
                ItemTex {
                    name: info.name.to_string(),
                    layer,
                },
            );
        }
    }
    for (id, tex) in ITEM_EXTRA {
        if b.png(tex).is_some() {
            let name = items
                .get(&id.to_string())
                .map_or_else(|| (*tex).to_string(), |i| i.name.clone());
            let layer = b.layer(tex);
            items.insert(id.to_string(), ItemTex { name, layer });
        }
    }
    let mut meta_items: Vec<(String, String)> = vec![("263:1".into(), "items/charcoal".into())];
    for (i, c) in [
        "black",
        "red",
        "green",
        "brown",
        "blue",
        "purple",
        "cyan",
        "silver",
        "gray",
        "pink",
        "lime",
        "yellow",
        "light_blue",
        "magenta",
        "orange",
        "white",
    ]
    .iter()
    .enumerate()
    {
        meta_items.push((format!("351:{i}"), format!("items/dye_powder_{c}")));
    }
    for (k, tex) in meta_items {
        if b.png(&tex).is_some() {
            let layer = b.layer(&tex);
            items.insert(k, ItemTex { name: tex, layer });
        }
    }
    // textures sprites and the hand need even if no model uses them
    for t in ["blocks/water_still", "blocks/lava_still"] {
        b.layer(t);
    }
    for i in 0..10 {
        b.layer(&format!("blocks/destroy_stage_{i}"));
    }
    for l in &b.layers {
        if l.src.is_none() {
            sources.insert(l.name.clone(), l.name.clone());
        }
    }
    let mut have_images = Vec::new();
    for img in images {
        if b.png(img).is_some() {
            sources.insert(img.clone(), img.clone());
            have_images.push(img.clone());
        }
    }
    let biomes = ghast_data::v1_8::biomes()
        .iter()
        .map(|bi| (bi.id, bi.temperature, bi.rainfall))
        .collect();
    let data = ModelData {
        layers: b.layers,
        models: b.models,
        blocks,
        items,
        biomes,
        images: have_images,
    };
    (data, sources)
}

/// The pack: header `{layers: [[offset, length]], images: {name: [offset, length]}}` and the
/// PNG files.
#[must_use]
pub fn build_pack(data: &ModelData, files: &Files) -> Vec<u8> {
    let mut parts: Vec<&[u8]> = Vec::new();
    let mut off = 0usize;
    let mut seen: HashMap<String, (usize, usize)> = HashMap::new();
    let mut add = |name: &str| -> (usize, usize) {
        if let Some(&k) = seen.get(name) {
            return k;
        }
        let body: &[u8] = files
            .get(&format!("textures/{name}.png"))
            .map_or(&[], Vec::as_slice);
        let at = (off, body.len());
        parts.push(body);
        off += body.len();
        seen.insert(name.to_string(), at);
        at
    };
    let layers: Vec<(usize, usize)> = data
        .layers
        .iter()
        .map(|l| add(l.src.as_deref().unwrap_or(&l.name)))
        .collect();
    let mut images: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for img in &data.images {
        images.insert(img.clone(), add(img));
    }
    let header = serde_json::to_vec(&serde_json::json!({ "layers": layers, "images": images }))
        .unwrap_or_default();
    let mut out = Vec::with_capacity(4 + header.len() + off);
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    for p in parts {
        out.extend_from_slice(p);
    }
    out
}

/// Items whose texture name differs from the item name.
const ITEM_EXTRA: &[(u16, &str)] = &[
    (295, "items/seeds_wheat"),
    (296, "items/wheat"),
    (287, "items/string"),
    (361, "items/seeds_pumpkin"),
    (362, "items/seeds_melon"),
    (372, "items/nether_wart"),
    (338, "items/reeds"),
    (391, "items/carrot"),
    (392, "items/potato"),
    (360, "items/melon"),
    (326, "items/bucket_water"),
    (327, "items/bucket_lava"),
    (325, "items/bucket_empty"),
    (335, "items/bucket_milk"),
    (268, "items/wood_sword"),
    (269, "items/wood_shovel"),
    (270, "items/wood_pickaxe"),
    (271, "items/wood_axe"),
    (290, "items/wood_hoe"),
    (283, "items/gold_sword"),
    (284, "items/gold_shovel"),
    (285, "items/gold_pickaxe"),
    (286, "items/gold_axe"),
    (294, "items/gold_hoe"),
    (267, "items/iron_sword"),
    (256, "items/iron_shovel"),
    (257, "items/iron_pickaxe"),
    (258, "items/iron_axe"),
    (292, "items/iron_hoe"),
    (276, "items/diamond_sword"),
    (277, "items/diamond_shovel"),
    (278, "items/diamond_pickaxe"),
    (279, "items/diamond_axe"),
    (293, "items/diamond_hoe"),
    (272, "items/stone_sword"),
    (273, "items/stone_shovel"),
    (274, "items/stone_pickaxe"),
    (275, "items/stone_axe"),
    (291, "items/stone_hoe"),
    (263, "items/coal"),
    (264, "items/diamond"),
    (265, "items/iron_ingot"),
    (266, "items/gold_ingot"),
    (280, "items/stick"),
    (281, "items/bowl"),
    (297, "items/bread"),
    (260, "items/apple"),
    (322, "items/apple_golden"),
    (364, "items/beef_cooked"),
    (363, "items/beef_raw"),
    (320, "items/porkchop_cooked"),
    (366, "items/chicken_cooked"),
    (393, "items/potato_baked"),
    (400, "items/pumpkin_pie"),
    (357, "items/cookie"),
    (354, "items/cake"),
    (344, "items/egg"),
    (332, "items/snowball"),
    (341, "items/slimeball"),
    (352, "items/bone"),
    (288, "items/feather"),
    (289, "items/gunpowder"),
    (334, "items/leather"),
    (339, "items/paper"),
    (340, "items/book_normal"),
    (345, "items/compass"),
    (347, "items/clock"),
    (346, "items/fishing_rod_uncast"),
    (261, "items/bow_standby"),
    (262, "items/arrow"),
    (259, "items/flint_and_steel"),
    (359, "items/shears"),
    (368, "items/ender_pearl"),
    (388, "items/emerald"),
    (331, "items/redstone_dust"),
    (348, "items/glowstone_dust"),
    (371, "items/gold_nugget"),
    (337, "items/clay_ball"),
    (336, "items/brick"),
    (405, "items/netherbrick"),
    (406, "items/quartz"),
    (323, "items/sign"),
    (324, "items/door_wood"),
    (330, "items/door_iron"),
    (380, "items/cauldron"),
    (379, "items/brewing_stand"),
    (390, "items/flower_pot"),
    (397, "items/skull_skeleton"),
    (384, "items/experience_bottle"),
    (373, "items/potion_bottle_drinkable"),
    (374, "items/potion_bottle_empty"),
    (375, "items/spider_eye"),
    (376, "items/spider_eye_fermented"),
    (377, "items/blaze_powder"),
    (369, "items/blaze_rod"),
    (370, "items/ghast_tear"),
    (378, "items/magma_cream"),
    (381, "items/ender_eye"),
    (382, "items/melon_speckled"),
    (396, "items/carrot_golden"),
    (398, "items/carrot_on_a_stick"),
    (399, "items/nether_star"),
    (403, "items/book_enchanted"),
    (407, "items/minecart_tnt"),
    (328, "items/minecart_normal"),
    (333, "items/boat"),
    (329, "items/saddle"),
    (342, "items/minecart_chest"),
    (343, "items/minecart_furnace"),
    (349, "items/fish_cod_raw"),
    (350, "items/fish_cod_cooked"),
    (353, "items/sugar"),
    (355, "items/bed"),
    (356, "items/repeater"),
    (404, "items/comparator"),
    (365, "items/chicken_raw"),
    (367, "items/rotten_flesh"),
    (319, "items/porkchop_raw"),
    (282, "items/mushroom_stew"),
    (413, "items/rabbit_stew"),
    (389, "items/item_frame"),
    (321, "items/painting"),
    (358, "items/map_filled"),
    (395, "items/map_empty"),
    (401, "items/fireworks"),
    (402, "items/fireworks_charge"),
    (385, "items/fireball"),
    (386, "items/book_writable"),
    (387, "items/book_written"),
    (417, "items/iron_horse_armor"),
    (418, "items/gold_horse_armor"),
    (419, "items/diamond_horse_armor"),
    (420, "items/lead"),
    (421, "items/name_tag"),
    (383, "items/spawn_egg"),
    (394, "items/potato_poisonous"),
    (298, "items/leather_helmet"),
    (299, "items/leather_chestplate"),
    (300, "items/leather_leggings"),
    (301, "items/leather_boots"),
    (302, "items/chainmail_helmet"),
    (303, "items/chainmail_chestplate"),
    (304, "items/chainmail_leggings"),
    (305, "items/chainmail_boots"),
    (306, "items/iron_helmet"),
    (307, "items/iron_chestplate"),
    (308, "items/iron_leggings"),
    (309, "items/iron_boots"),
    (310, "items/diamond_helmet"),
    (311, "items/diamond_chestplate"),
    (312, "items/diamond_leggings"),
    (313, "items/diamond_boots"),
    (314, "items/gold_helmet"),
    (315, "items/gold_chestplate"),
    (316, "items/gold_leggings"),
    (317, "items/gold_boots"),
    (351, "items/dye_powder_black"),
];
