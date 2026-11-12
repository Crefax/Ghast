//! 26.2 (protocol 776) items: the real 26.2 client's component hashes and use facts, replayed
//! through the version-neutral API (`ghast_inventory::modern`).

#![cfg(feature = "v26_2")]

use std::io::Read;
use std::sync::LazyLock;

use bytes::Bytes;
use ghast_inventory::modern::{self, HashRegistries, UseAnimation};
use ghast_inventory::{Stack, Version};
use ghast_protocol::codec::Reader;
use ghast_protocol::defs::p776::types::{
    HashedSlot, HashedSlotComponents, HashedSlotRemoveComponents, SlotComponentType,
};

const V: Version = Version::V26_2;

/// Component type names by wire id.
static COMPONENT_NAMES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    (0..modern::component_count(V) as i32)
        .map(|t| modern::component_name(V, t).unwrap())
        .collect()
});

fn component_hash(ty: i32, v: &Bytes, reg: &HashRegistries) -> Option<i32> {
    modern::component_hash(V, ty, v, reg)
}

fn hash_covers(ty: i32) -> bool {
    modern::hash_covers(V, ty)
}

fn item_use(s: &Stack) -> Option<modern::ItemUse> {
    modern::item_use(V, s)
}

fn item_id(n: &str) -> Option<i32> {
    modern::item_id(V, n)
}

/// The hashed stack a 26.2 click carries (as the client builds it: last value of a type wins).
fn hashed_slot(s: Option<&Stack>, reg: &HashRegistries) -> (Option<HashedSlot>, bool) {
    let Some(s) = s.filter(|s| s.count() > 0 && s.id() != 0) else {
        return (None, true);
    };
    let mut exact = true;
    let mut components: Vec<HashedSlotComponents> = Vec::new();
    let mut remove_components = Vec::new();
    if let Some(p) = s.components() {
        let all: Vec<(i32, Bytes)> = p.iter().collect();
        for (t, b) in all.iter().rev() {
            if components.iter().any(|c| c.r#type.0 == *t) {
                continue;
            }
            let hash = component_hash(*t, b, reg).unwrap_or_else(|| {
                exact = false;
                0
            });
            components.push(HashedSlotComponents {
                r#type: SlotComponentType::new(*t),
                hash,
            });
        }
        let mut rm: Vec<i32> = p.removed().collect();
        rm.sort_unstable();
        rm.dedup();
        remove_components = rm
            .into_iter()
            .map(|t| HashedSlotRemoveComponents {
                r#type: SlotComponentType::new(t),
            })
            .collect();
    }
    components.sort_by_key(|c| c.r#type.0);
    (
        Some(HashedSlot {
            item_id: s.id(),
            item_count: s.count(),
            components,
            remove_components,
        }),
        exact,
    )
}

/// `tests/golden/26.2/hashed_stacks.jsonl.gz`: 4000 random stacks covering every component
/// type, with the client's registries). Every hash ghast computes must equal the client's; types
/// ghast does not hash are listed (they are sent with a hash the server corrects).
#[test]
fn hashed_stacks_match_the_real_client() {
    use ghast_protocol::types::slot::v775::ItemStack;
    use std::collections::BTreeMap;
    const VECTORS: &[u8] = include_bytes!("../../../tests/golden/26.2/hashed_stacks.jsonl.gz");
    let mut text = String::new();
    flate2::read::GzDecoder::new(VECTORS)
        .read_to_string(&mut text)
        .unwrap();
    let unhex = |s: &str| -> Bytes {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect::<Vec<u8>>()
            .into()
    };
    let mut reg = HashRegistries::default();
    // per type: (exact, wrong, not hashed)
    let mut per: BTreeMap<i32, [usize; 3]> = BTreeMap::new();
    let mut stacks = 0;
    let mut whole_exact = 0;
    let mut undecodable: Vec<String> = Vec::new();
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        match v["type"].as_str().unwrap() {
            "registry" => {
                let names: Vec<&str> = v["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e.as_str().unwrap())
                    .collect();
                reg.add(v["name"].as_str().unwrap(), names);
            }
            "stack" => {
                stacks += 1;
                let wire = unhex(v["wire"].as_str().unwrap());
                let s = match ItemStack::read_776(&mut Reader::new(&wire)) {
                    Ok(s) => s.unwrap(),
                    Err(e) => {
                        // name the component the generated layout cannot read
                        let mut r = Reader::new(&wire);
                        for _ in 0..4 {
                            r.read_varint().unwrap();
                        }
                        let mut bad = String::from("?");
                        for _ in 0..v["added"].as_array().unwrap().len() {
                            let ty = r.clone().read_varint().unwrap();
                            if ghast_protocol::defs::p776::types::SlotComponent::skip(&mut r)
                                .is_err()
                            {
                                bad = COMPONENT_NAMES[ty as usize].to_string();
                                break;
                            }
                        }
                        undecodable.push(format!("{bad} ({e})"));
                        continue;
                    }
                };
                assert_eq!(s.id, v["item_id"].as_i64().unwrap() as i32);
                let mut want_types: Vec<i32> = v["added"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a["id"].as_i64().unwrap() as i32)
                    .collect();
                let mut got_types: Vec<i32> = s.components.iter().map(|(t, _)| t).collect();
                want_types.sort();
                got_types.sort();
                if want_types != got_types
                    || s.components.removed().count() != v["removed"].as_array().unwrap().len()
                {
                    let wire_order: Vec<&str> = s
                        .components
                        .iter()
                        .map(|(t, _)| COMPONENT_NAMES.get(t as usize).copied().unwrap_or("?"))
                        .collect();
                    undecodable.push(format!("types differ: wire order read {wire_order:?}"));
                    continue;
                }
                let mut all = true;
                for a in v["added"].as_array().unwrap() {
                    let ty = a["id"].as_i64().unwrap() as i32;
                    let want = a["hash"].as_i64().unwrap() as i32;
                    let value = s.components.get(ty).expect("component on the wire");
                    let e = per.entry(ty).or_default();
                    match component_hash(ty, &value, &reg) {
                        Some(h) if h == want => e[0] += 1,
                        Some(_) => {
                            e[1] += 1;
                            all = false;
                        }
                        None => {
                            e[2] += 1;
                            all = false;
                        }
                    }
                }
                if all {
                    // the whole HashedStack, removed set included, byte for byte
                    let st = ghast_inventory::Stack::from_v775(s);
                    let (h, exact) = hashed_slot(Some(&st), &reg);
                    assert!(exact);
                    let want = unhex(v["hashed_wire"].as_str().unwrap());
                    // optional HashedStack; HashedPatchMap is a map: compare sorted
                    let mut r = Reader::new(&want);
                    assert!(r.read_bool().unwrap());
                    let back = <ghast_protocol::defs::p776::types::HashedSlot as ghast_protocol::Decode>::decode(&mut r).unwrap();
                    let (mut x, mut y) = (h.unwrap(), back);
                    x.components.sort_by_key(|c| c.r#type.0);
                    y.components.sort_by_key(|c| c.r#type.0);
                    x.remove_components.sort_by_key(|c| c.r#type.0);
                    y.remove_components.sort_by_key(|c| c.r#type.0);
                    assert_eq!(x, y, "stack {}", v["k"]);
                    whole_exact += 1;
                }
            }
            _ => {}
        }
    }
    assert_eq!(stacks, 4000);
    undecodable.sort();
    undecodable.dedup();
    assert!(
        undecodable.is_empty(),
        "stacks ghast cannot read: {undecodable:?}"
    );
    let name = |t: i32| COMPONENT_NAMES[t as usize];
    let exact: Vec<&str> = per
        .iter()
        .filter(|(_, c)| c[0] > 0 && c[1] == 0 && c[2] == 0)
        .map(|(t, _)| name(*t))
        .collect();
    let partial: Vec<String> = per
        .iter()
        .filter(|(_, c)| c[0] > 0 && (c[1] > 0 || c[2] > 0))
        .map(|(t, c)| format!("{}:{c:?}", name(*t)))
        .collect();
    let wrong: Vec<String> = per
        .iter()
        .filter(|(_, c)| c[1] > 0)
        .map(|(t, c)| format!("{}:{c:?}", name(*t)))
        .collect();
    let none: Vec<&str> = per
        .iter()
        .filter(|(_, c)| c[0] == 0 && c[1] == 0)
        .map(|(t, _)| name(*t))
        .collect();
    eprintln!(
        "exact ({}): {exact:?}\npartial ({}): {partial:?}\nnot hashed ({}): {none:?}\nwhole stacks exact: {whole_exact}",
        exact.len(),
        partial.len(),
        none.len()
    );
    assert!(wrong.is_empty(), "wrong hashes: {wrong:?}");

    // Coverage of every component type: proven by the vectors, hashed but not in the vectors,
    // or not hashed (sent as an unknown hash; the server re-syncs that slot).
    let mut table = String::new();
    let mut classes = [0usize; 4];
    for (t, n) in COMPONENT_NAMES.iter().enumerate() {
        let c = per.get(&(t as i32)).copied().unwrap_or_default();
        let class = match (c[0] > 0, c[2] > 0, hash_covers(t as i32)) {
            (true, false, _) => 0,  // every vector value hashed exactly
            (true, true, _) => 1,   // some values outside the schema
            (false, _, true) => 2,  // schema row, no vector value
            (false, _, false) => 3, // no schema row
        };
        classes[class] += 1;
        let label = [
            "vector-exact",
            "vector-partial",
            "schema-unproven",
            "not-hashed",
        ][class];
        table.push_str(&format!("{t}\t{n}\t{label}\t{}\t{}\n", c[0], c[2]));
    }
    eprintln!("coverage (id, name, class, exact values, unhashed values):\n{table}{classes:?}");
    assert_eq!(classes.iter().sum::<usize>(), COMPONENT_NAMES.len());
    // Every kind a client can receive is hashed like the real client; only the three
    // transient kinds (never sent to clients) are not.
    let unhashed: Vec<&str> = COMPONENT_NAMES
        .iter()
        .enumerate()
        .filter(|(t, _)| !hash_covers(*t as i32))
        .map(|(_, n)| *n)
        .collect();
    assert_eq!(
        unhashed,
        [
            "creative_slot_lock",
            "additional_trade_cost",
            "map_post_processing"
        ]
    );
    assert_eq!(classes[1], 0, "partially hashed kinds");
    assert_eq!(whole_exact, stacks, "every whole HashedStack is exact");
}

/// The use facts the client engine reads (`modern::item_use`) decode the `use_effects`,
/// `consumable` and `food` components the real client's codecs wrote: every such component in
/// the oracle's `hashed_stacks.jsonl.gz` (values as the codec's JSON, defaults omitted).
#[test]
fn item_use_reads_the_real_components() {
    use ghast_protocol::types::slot::v775::ItemStack;
    const VECTORS: &[u8] = include_bytes!("../../../tests/golden/26.2/hashed_stacks.jsonl.gz");
    let mut text = String::new();
    flate2::read::GzDecoder::new(VECTORS)
        .read_to_string(&mut text)
        .unwrap();
    let unhex = |s: &str| -> Bytes {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect::<Vec<u8>>()
            .into()
    };
    let anim = |n: &str| match n {
        "none" => UseAnimation::None,
        "eat" => UseAnimation::Eat,
        "drink" => UseAnimation::Drink,
        "block" => UseAnimation::Block,
        "bow" => UseAnimation::Bow,
        "trident" => UseAnimation::Trident,
        "crossbow" => UseAnimation::Crossbow,
        "spyglass" => UseAnimation::Spyglass,
        "toot_horn" => UseAnimation::TootHorn,
        "brush" => UseAnimation::Brush,
        "bundle" => UseAnimation::Bundle,
        "spear" => UseAnimation::Spear,
        o => panic!("{o}"),
    };
    let mut checked = [0usize; 3];
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if v["type"] != "stack" {
            continue;
        }
        let wire = unhex(v["wire"].as_str().unwrap());
        let Ok(Some(st)) = ItemStack::read_776(&mut Reader::new(&wire)) else {
            continue;
        };
        let name = v["item"].as_str().unwrap();
        let st = ghast_inventory::Stack::from_v775(st);
        let u = item_use(&st).unwrap();
        for a in v["added"].as_array().unwrap() {
            let j = &a["json"];
            match a["component"].as_str().unwrap() {
                "minecraft:use_effects" => {
                    assert_eq!(u.can_sprint, j["can_sprint"].as_bool().unwrap_or(false));
                    assert_eq!(
                        u.interact_vibrations,
                        j["interact_vibrations"].as_bool().unwrap_or(true)
                    );
                    let speed = j["speed_multiplier"].as_f64().unwrap_or(0.2) as f32;
                    assert_eq!(u.speed_multiplier.to_bits(), speed.to_bits(), "{line}");
                    checked[0] += 1;
                }
                "minecraft:consumable" => {
                    assert!(u.consumable);
                    let secs = j["consume_seconds"].as_f64().unwrap_or(1.6) as f32;
                    // item classes that override the animation / duration win (bows, …)
                    let plain = !matches!(
                        name,
                        "minecraft:bow"
                            | "minecraft:crossbow"
                            | "minecraft:trident"
                            | "minecraft:spyglass"
                            | "minecraft:brush"
                            | "minecraft:goat_horn"
                            | "minecraft:ender_eye"
                    ) && !name.ends_with("bundle");
                    if plain {
                        assert_eq!(u.duration, Some((secs * 20.0) as i32), "{line}");
                        assert_eq!(
                            u.animation,
                            anim(j["animation"].as_str().unwrap_or("eat")),
                            "{line}"
                        );
                    }
                    checked[1] += 1;
                }
                "minecraft:food" => {
                    let f = u.food.unwrap();
                    assert_eq!(f.nutrition, j["nutrition"].as_i64().unwrap() as i32);
                    assert_eq!(
                        f.saturation.to_bits(),
                        (j["saturation"].as_f64().unwrap() as f32).to_bits()
                    );
                    assert_eq!(
                        f.can_always_eat,
                        j["can_always_eat"].as_bool().unwrap_or(false)
                    );
                    checked[2] += 1;
                }
                _ => {}
            }
        }
    }
    eprintln!(
        "use_effects {} consumable {} food {}",
        checked[0], checked[1], checked[2]
    );
    assert!(checked.iter().all(|&n| n >= 20), "{checked:?}");
    // item defaults
    let id = |n: &str| item_id(n).unwrap();
    let plain = |n: &str| item_use(&ghast_inventory::Stack::new(id(n), 1, 0)).unwrap();
    let shield = plain("shield");
    assert_eq!(
        (shield.animation, shield.duration),
        (UseAnimation::Block, Some(72000))
    );
    assert_eq!((shield.speed_multiplier, shield.can_sprint), (0.2, false));
    let spear = plain("diamond_spear");
    assert_eq!(
        (spear.animation, spear.speed_multiplier, spear.can_sprint),
        (UseAnimation::Spear, 1.0, true)
    );
    assert_eq!(plain("potion").duration, Some(32));
    assert_eq!(plain("potion").animation, UseAnimation::Drink);
    assert_eq!(plain("honey_bottle").duration, Some(40));
    assert_eq!(plain("dried_kelp").duration, Some(16));
    assert_eq!(plain("bread").food.unwrap().nutrition, 5);
    assert_eq!(plain("bow").animation, UseAnimation::Bow);
    assert_eq!(plain("stone").duration, Some(0));
}
