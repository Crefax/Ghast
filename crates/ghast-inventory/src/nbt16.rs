//! 1.16.5 item NBT edits the client does to compute menu results (grindstone, anvil): the
//! stack's tag as a tree, enchantment lists read like `EnchantmentHelper.fromNbt` and written
//! like `ItemStack.addEnchantment` / `EnchantedBookItem.addEnchantment`.

use ghast_protocol::nbt::{Compound, List, NamedTag, RawNbt, Tag, tag};

use crate::rules::Version;
use crate::sim::{R, Unpredictable};
use crate::stack::{ItemData, Stack, last, number};

/// 1.16.5 enchantments by registry id (the jar's order; checked by the unit test).
pub(crate) const ENCHANTMENTS_1_16: &[&str] = &[
    "protection",
    "fire_protection",
    "feather_falling",
    "blast_protection",
    "projectile_protection",
    "respiration",
    "aqua_affinity",
    "thorns",
    "depth_strider",
    "frost_walker",
    "binding_curse",
    "soul_speed",
    "sharpness",
    "smite",
    "bane_of_arthropods",
    "knockback",
    "fire_aspect",
    "looting",
    "sweeping",
    "efficiency",
    "silk_touch",
    "unbreaking",
    "fortune",
    "power",
    "punch",
    "flame",
    "infinity",
    "luck_of_the_sea",
    "lure",
    "loyalty",
    "impaling",
    "riptide",
    "channeling",
    "multishot",
    "quick_charge",
    "piercing",
    "mending",
    "vanishing_curse",
];

/// `Enchantment.isCursed` (binding and vanishing).
pub(crate) fn cursed(e: usize) -> bool {
    matches!(
        ENCHANTMENTS_1_16.get(e),
        Some(&("binding_curse" | "vanishing_curse"))
    )
}

/// `Identifier.tryParse(s)` then the enchantment registry: the enchantment's id.
pub(crate) fn enchantment(s: &str) -> Option<usize> {
    let (ns, path) = s.split_once(':').unwrap_or(("minecraft", s));
    let ns_ok = ns
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'));
    let path_ok = path
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'));
    if !ns_ok || !path_ok || ns != "minecraft" {
        return None;
    }
    ENCHANTMENTS_1_16.iter().position(|&n| n == path)
}

/// A stack's tag as a tree (`Ok(None)`: no tag; `Err`: unreadable).
pub(crate) fn tree(s: &Stack) -> R<Option<Compound>> {
    match &s.data {
        ItemData::Nbt(n) => match n.parse().map_err(|_| Unpredictable)? {
            Some(NamedTag {
                tag: Tag::Compound(c),
                ..
            }) => Ok(Some(c)),
            None => Ok(None),
            Some(_) => Err(Unpredictable),
        },
        _ => Ok(None),
    }
}

/// A stack of `id` × `count` with the tag.
pub(crate) fn with_tree(id: i32, count: i32, t: Option<Compound>) -> R<Stack> {
    let data = match t {
        None => ItemData::None,
        Some(c) => ItemData::Nbt(
            RawNbt::from_named(&NamedTag {
                name: String::new(),
                tag: Tag::Compound(c),
            })
            .map_err(|_| Unpredictable)?,
        ),
    };
    Ok(Stack::new(id, count, 0).with_data(data))
}

/// `NbtCompound.getInt(key)` (0 when absent or not a number).
pub(crate) fn get_int(c: &Compound, key: &str) -> i32 {
    last(c, key).and_then(number).unwrap_or(0) as i32
}

/// `NbtCompound.getString(key)`.
pub(crate) fn get_string<'a>(c: &'a Compound, key: &str) -> &'a str {
    match last(c, key) {
        Some(Tag::String(s)) => s,
        _ => "",
    }
}

/// `ItemStack.getOrCreateTag`: a new tag goes through `setTag`, which writes `Damage: 0` on an
/// item with durability.
pub(crate) fn create(t: &mut Option<Compound>, durable: bool) -> &mut Compound {
    t.get_or_insert_with(|| {
        let mut c = Compound::default();
        if durable {
            c.0.push(("Damage".into(), Tag::Int(0)));
        }
        c
    })
}

/// `put(key, value)`: replaces the entry.
pub(crate) fn put(c: &mut Compound, key: &str, v: Tag) {
    c.0.retain(|(k, _)| k != key);
    c.0.push((key.to_owned(), v));
}

/// `remove(key)`.
pub(crate) fn remove(c: &mut Compound, key: &str) {
    c.0.retain(|(k, _)| k != key);
}

/// `ItemStack.removeSubTag(key)`: the tag goes away when it gets empty.
pub(crate) fn remove_sub(t: &mut Option<Compound>, key: &str) {
    if let Some(c) = t
        && last(c, key).is_some()
    {
        remove(c, key);
        if c.0.is_empty() {
            *t = None;
        }
    }
}

/// `getList(key, 10)`: the compounds of a compound list (an empty or other list: none).
/// `Err` when it holds something else that vanilla would replace silently (left unmodelled).
pub(crate) fn compound_list(c: &Compound, key: &str) -> Vec<Compound> {
    match last(c, key) {
        Some(Tag::List(l)) if l.elem_type == tag::COMPOUND => l
            .items
            .iter()
            .filter_map(|t| match t {
                Tag::Compound(c) => Some(c.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A list of compounds as a tag.
pub(crate) fn list_tag(items: Vec<Compound>) -> Tag {
    Tag::List(List {
        elem_type: if items.is_empty() {
            tag::END
        } else {
            tag::COMPOUND
        },
        items: items.into_iter().map(Tag::Compound).collect(),
    })
}

/// `EnchantmentHelper.get(stack)`: `StoredEnchantments` of an enchanted book, else
/// `Enchantments`, as `fromNbt` reads them (registered ones only; a repeated enchantment keeps
/// its first place and its last level).
pub(crate) fn enchantments(v: Version, s: &Stack, t: Option<&Compound>) -> Vec<(usize, i32)> {
    let key = if v.name(s.id) == "enchanted_book" {
        "StoredEnchantments"
    } else {
        "Enchantments"
    };
    let mut out: Vec<(usize, i32)> = Vec::new();
    let Some(t) = t else { return out };
    for e in compound_list(t, key) {
        if let Some(id) = enchantment(get_string(&e, "id")) {
            let lvl = get_int(&e, "lvl");
            match out.iter_mut().find(|x| x.0 == id) {
                Some(x) => x.1 = lvl,
                None => out.push((id, lvl)),
            }
        }
    }
    out
}

/// An enchantment list entry: `{id: "minecraft:<name>", lvl: <short>}`.
pub(crate) fn entry(e: usize, lvl: i16) -> Compound {
    Compound(vec![
        (
            "id".into(),
            Tag::String(format!("minecraft:{}", ENCHANTMENTS_1_16[e])),
        ),
        ("lvl".into(), Tag::Short(lvl)),
    ])
}

/// `EnchantmentHelper.getLevel(e, stack)`: the first matching `Enchantments` entry (also on a
/// book), clamped to 0..=255.
pub(crate) fn level(t: Option<&Compound>, e: usize) -> i32 {
    let Some(t) = t else { return 0 };
    compound_list(t, "Enchantments")
        .iter()
        .find(|c| enchantment(get_string(c, "id")) == Some(e))
        .map_or(0, |c| get_int(c, "lvl").clamp(0, 255))
}

/// `ItemStack.addEnchantment(e, level)`: appended to `Enchantments` (created when it is not a
/// list), the level stored as `(short)(byte)level`.
pub(crate) fn add_enchantment(
    t: &mut Option<Compound>,
    durable: bool,
    e: usize,
    lvl: i32,
) -> R<()> {
    let c = create(t, durable);
    let mut list = match last(c, "Enchantments") {
        Some(Tag::List(l)) if l.elem_type == tag::COMPOUND || l.items.is_empty() => {
            compound_list(c, "Enchantments")
        }
        // a list of something else: vanilla's getList hands back a detached list
        Some(Tag::List(_)) => return Err(Unpredictable),
        _ => Vec::new(),
    };
    list.push(entry(e, i16::from(lvl as i8)));
    put(c, "Enchantments", list_tag(list));
    Ok(())
}

/// `EnchantedBookItem.addEnchantment`: raises an existing entry's level or appends one; the
/// list is put back as `StoredEnchantments`.
pub(crate) fn add_stored(t: &mut Option<Compound>, durable: bool, e: usize, lvl: i32) {
    let mut list = t
        .as_ref()
        .map(|c| compound_list(c, "StoredEnchantments"))
        .unwrap_or_default();
    match list
        .iter_mut()
        .find(|c| enchantment(get_string(c, "id")) == Some(e))
    {
        Some(c) => {
            if get_int(c, "lvl") < lvl {
                put(c, "lvl", Tag::Short(lvl as i16));
            }
        }
        None => list.push(entry(e, lvl as i16)),
    }
    let c = create(t, durable);
    put(c, "StoredEnchantments", list_tag(list));
}

/// `ItemStack.hasEnchantments`: a non-empty `Enchantments` list.
pub(crate) fn has_enchantments(t: Option<&Compound>) -> bool {
    t.and_then(|c| last(c, "Enchantments"))
        .is_some_and(|l| match l {
            Tag::List(l) => !l.items.is_empty() && (l.elem_type == tag::COMPOUND),
            _ => false,
        })
}

/// `ItemStack.getDamage`.
pub(crate) fn damage(t: Option<&Compound>) -> i32 {
    t.map_or(0, |c| get_int(c, "Damage"))
}

/// `ItemStack.getRepairCost` (an int tag only).
pub(crate) fn repair_cost(t: Option<&Compound>) -> i32 {
    match t.and_then(|c| last(c, "RepairCost")) {
        Some(Tag::Int(v)) => *v,
        _ => 0,
    }
}

/// `ItemStack.hasCustomName`: a string `display.Name`.
pub(crate) fn has_custom_name(t: Option<&Compound>) -> bool {
    matches!(
        t.and_then(|c| last(c, "display")),
        Some(Tag::Compound(d)) if matches!(last(d, "Name"), Some(Tag::String(_)))
    )
}

#[cfg(all(test, feature = "v1_16_5"))]
mod tests {
    use super::*;

    /// The enchantment registry as the jar has it (the grindstone facts line).
    #[test]
    fn enchantments_1_16_5() {
        use std::io::BufRead;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden/1.16.5/clicks_menus_seed1.jsonl.gz");
        let f = std::fs::File::open(&path).unwrap();
        let r = std::io::BufReader::new(flate2::read::GzDecoder::new(f));
        for line in r.lines() {
            let line = line.unwrap();
            if !line.starts_with("{\"facts\":\"grindstone\"") {
                continue;
            }
            let c: serde_json::Value = serde_json::from_str(&line).unwrap();
            let list = c["enchantments"].as_array().unwrap();
            assert_eq!(list.len(), ENCHANTMENTS_1_16.len());
            for (k, e) in list.iter().enumerate() {
                assert_eq!(
                    e[0].as_str().unwrap(),
                    format!("minecraft:{}", ENCHANTMENTS_1_16[k])
                );
                assert_eq!(e[1].as_bool().unwrap(), cursed(k), "{k}");
            }
            return;
        }
        panic!("no grindstone facts in {}", path.display());
    }
}
