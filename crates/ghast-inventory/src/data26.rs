//! 26.1 configuration data some menus read (the anvil, grindstone, loom and mount screens):
//! the tags of `update_tags` and the enchantment definitions of `registry_data`. Both arrive at
//! every join and are kept as their raw bytes; they are parsed only when a click in such a menu
//! first needs them (a bot that never opens one pays a copy of the bytes, nothing more).
//! Enchantments the server sends without data (the client knows the vanilla data pack) take
//! their vanilla definitions from `src/generated/enchantments_26_1.rs`.

use std::collections::HashMap;

use bytes::Bytes;
use ghast_protocol::codec::Reader;
use ghast_protocol::nbt::{AnonNbt, Tag};

/// A holder set as data components and definitions write it: a tag or listed registry ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Holders {
    Tag(String),
    Ids(Vec<i32>),
}

/// What the anvil (and the trident spin) reads off an enchantment.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ench {
    pub max_level: i32,
    pub anvil_cost: i32,
    pub supported: Holders,
    pub exclusive: Option<Holders>,
    /// The `minecraft:trident_spin_attack_strength` effect: `None` without one, `Some(None)`
    /// when it does not read.
    pub spin: Option<Option<crate::ench_value::Fx>>,
}

/// The tag registries the menus read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Reg {
    Item,
    Enchantment,
    BannerPattern,
    EntityType,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Data26 {
    tags_raw: Option<Bytes>,
    tags: Option<HashMap<(Reg, String), Vec<i32>>>,
    ench_raw: Option<Bytes>,
    /// By enchantment registry id (`None`: no definition known).
    ench: Option<Vec<Option<Ench>>>,
    /// Enchantment names by registry id (from `registry_data`, else the hash registries).
    ench_names: Vec<String>,
}

fn reg_of(name: &str) -> Option<Reg> {
    Some(match name.strip_prefix("minecraft:").unwrap_or(name) {
        "item" => Reg::Item,
        "enchantment" => Reg::Enchantment,
        "banner_pattern" => Reg::BannerPattern,
        "entity_type" => Reg::EntityType,
        _ => return None,
    })
}

/// A `Holders` from its definition / report text: `#tag`, one name or a list of names.
pub(crate) fn holders_from_names(spec: &str, id: impl Fn(&str) -> Option<i32>) -> Option<Holders> {
    if let Some(t) = spec.strip_prefix('#') {
        return Some(Holders::Tag(t.to_string()));
    }
    let mut ids = Vec::new();
    for n in spec.split(',').filter(|n| !n.is_empty()) {
        ids.push(id(n)?);
    }
    Some(Holders::Ids(ids))
}

fn names_spec(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => Some(s.clone()),
        Tag::List(l) => {
            let mut out = Vec::new();
            for e in &l.items {
                match e {
                    Tag::String(s) => out.push(s.clone()),
                    _ => return None,
                }
            }
            Some(out.join(","))
        }
        _ => None,
    }
}

/// One enchantment entry: name and, when sent with data, `(max_level, anvil_cost,
/// supported_items, exclusive_set, trident_spin_attack_strength)` (the effect in the text form
/// of `ench_value::Fx`, `""` without one, `"?"` when it does not read).
pub type EnchantmentSpec = (String, Option<(i32, i32, String, String, String)>);

/// `(name, max_level, anvil_cost, supported_items, exclusive_set)` of every enchantment entry
/// with data in a `registry_data` body (holder sets as `#tag` or comma-separated names).
pub(crate) fn enchantment_specs(body: &[u8]) -> Option<Vec<EnchantmentSpec>> {
    let b = Bytes::copy_from_slice(body);
    let mut r = Reader::new(&b);
    let registry = r.read_string().ok()?;
    if reg_of(&registry) != Some(Reg::Enchantment) {
        return None;
    }
    let n = r.read_varint().ok()?;
    let mut out = Vec::new();
    for _ in 0..n.max(0) {
        let key = r.read_string().ok()?.to_string();
        let def = if r.read_bool().ok()? {
            let nbt = AnonNbt::read(&mut r).ok()?;
            let tag = nbt.parse().ok()??;
            let c = tag.as_compound()?;
            let get = |k: &str| c.0.iter().rev().find(|(n, _)| n == k).map(|(_, v)| v);
            let int = |k: &str| match get(k) {
                Some(Tag::Int(v)) => Some(*v),
                _ => None,
            };
            Some((
                int("max_level")?,
                int("anvil_cost")?,
                names_spec(get("supported_items")?)?,
                get("exclusive_set")
                    .and_then(names_spec)
                    .unwrap_or_default(),
                spin_spec(get("effects")),
            ))
        } else {
            None
        };
        out.push((key, def));
    }
    Some(out)
}

/// The text form of an `effects` compound's `minecraft:trident_spin_attack_strength`.
fn spin_spec(effects: Option<&Tag>) -> String {
    let Some(c) = effects.and_then(Tag::as_compound) else {
        return String::new();
    };
    match c
        .0
        .iter()
        .rev()
        .find(|(n, _)| n == "minecraft:trident_spin_attack_strength")
    {
        None => String::new(),
        Some((_, t)) => {
            crate::ench_value::Fx::from_tag(t).map_or_else(|| "?".into(), |fx| fx.to_string())
        }
    }
}

impl Data26 {
    /// `update_tags` (configuration or play): replaces the tags.
    pub(crate) fn set_tags_raw(&mut self, body: Bytes) {
        self.tags_raw = Some(body);
        self.tags = None;
    }

    /// A `registry_data` body; kept when it is the enchantment registry.
    pub(crate) fn set_registry_raw(&mut self, body: Bytes) {
        let mut r = Reader::new(&body);
        if r.read_string().ok().as_deref().and_then(reg_of) == Some(Reg::Enchantment) {
            self.ench_raw = Some(body);
            self.ench = None;
        }
    }

    /// Bytes kept unparsed (for measurements).
    #[allow(dead_code)]
    pub(crate) fn raw_len(&self) -> usize {
        self.tags_raw.as_ref().map_or(0, Bytes::len) + self.ench_raw.as_ref().map_or(0, Bytes::len)
    }

    /// Parses what is still raw (the first click in a menu that reads tags or enchantments).
    pub(crate) fn ensure(&mut self, ench_names: &[String]) {
        if let Some(raw) = self.tags_raw.take() {
            self.tags = parse_tags(&raw);
        }
        if self.ench.is_none() || (self.ench_names.is_empty() && !ench_names.is_empty()) {
            self.parse_enchantments(ench_names);
        }
    }

    fn parse_enchantments(&mut self, hash_names: &[String]) {
        let from_packet = self.ench_raw.take().and_then(|b| enchantment_specs(&b));
        let mut names: Vec<String> = match &from_packet {
            Some(v) => v.iter().map(|(n, _)| n.clone()).collect(),
            None => hash_names.to_vec(),
        };
        if names.is_empty() {
            names = hash_names.to_vec();
        }
        let id_of = |n: &str| names.iter().position(|x| x == n).map(|i| i as i32);
        let item_id = |n: &str| crate::v26_1::item_id(n);
        let mut defs = Vec::with_capacity(names.len());
        for (i, name) in names.iter().enumerate() {
            let spec = from_packet
                .as_ref()
                .and_then(|v| v.get(i))
                .and_then(|(_, d)| d.clone())
                .or_else(|| {
                    builtin(name)
                        .map(|(m, a, s, e, x)| (m, a, s.to_string(), e.to_string(), x.to_string()))
                });
            defs.push(spec.and_then(|(max_level, anvil_cost, s, e, x)| {
                Some(Ench {
                    max_level,
                    anvil_cost,
                    supported: holders_from_names(&s, item_id)?,
                    exclusive: if e.is_empty() {
                        None
                    } else {
                        Some(holders_from_names(&e, id_of)?)
                    },
                    spin: (!x.is_empty()).then(|| crate::ench_value::Fx::parse(&x)),
                })
            }));
        }
        self.ench = Some(defs);
        self.ench_names = names;
    }

    /// The definition of enchantment `id`.
    pub(crate) fn enchantment(&self, id: i32) -> Option<&Ench> {
        self.ench.as_ref()?.get(usize::try_from(id).ok()?)?.as_ref()
    }

    /// The ids of a tag (`None` when no tags were received or the tag is unknown).
    pub(crate) fn tag(&self, reg: Reg, name: &str) -> Option<&[i32]> {
        let name = name.strip_prefix('#').unwrap_or(name);
        let full = if name.contains(':') {
            name.to_string()
        } else {
            format!("minecraft:{name}")
        };
        self.tags.as_ref()?.get(&(reg, full)).map(Vec::as_slice)
    }

    /// Whether `id` is in a holder set (`None` when a tag is not known).
    pub(crate) fn contains(&self, reg: Reg, set: &Holders, id: i32) -> Option<bool> {
        match set {
            Holders::Ids(v) => Some(v.contains(&id)),
            Holders::Tag(t) => self.tag(reg, t).map(|v| v.contains(&id)),
        }
    }

    /// The ids of a holder set, in order.
    pub(crate) fn resolve(&self, reg: Reg, set: &Holders) -> Option<Vec<i32>> {
        match set {
            Holders::Ids(v) => Some(v.clone()),
            Holders::Tag(t) => self.tag(reg, t).map(<[i32]>::to_vec),
        }
    }
}

fn parse_tags(raw: &Bytes) -> Option<HashMap<(Reg, String), Vec<i32>>> {
    let mut r = Reader::new(raw);
    let mut out = HashMap::new();
    let n = r.read_varint().ok()?;
    for _ in 0..n.max(0) {
        let reg = r.read_string().ok()?;
        let keep = reg_of(&reg);
        let k = r.read_varint().ok()?;
        for _ in 0..k.max(0) {
            let name = r.read_string().ok()?;
            let c = r.read_varint().ok()?;
            let c = r.check_count(i64::from(c), 1).ok()?;
            let mut ids = Vec::with_capacity(if keep.is_some() { c } else { 0 });
            for _ in 0..c {
                let v = r.read_varint().ok()?;
                if keep.is_some() {
                    ids.push(v);
                }
            }
            if let Some(reg) = keep {
                out.insert((reg, name.to_string()), ids);
            }
        }
    }
    Some(out)
}

mod builtin_table {
    include!("generated/enchantments_26_1.rs");
}

/// The vanilla definition of an enchantment (`max_level`, `anvil_cost`, supported items,
/// exclusive set, trident spin attack strength effect).
fn builtin(name: &str) -> Option<(i32, i32, &'static str, &'static str, &'static str)> {
    builtin_table::ENCHANTMENTS
        .iter()
        .find(|e| e.0 == name)
        .map(|e| (e.1, e.2, e.3, e.4, e.5))
}
