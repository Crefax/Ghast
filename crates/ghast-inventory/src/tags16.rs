//! 1.16.5 item tags from the server (`tags`): the slot rules that read them on the client
//! (beacon payment, furnace fuel, llama carpets) follow the server's data pack. Without a
//! `tags` packet (not read: ghast-client's `Interests::RECIPES` is off) those rules are
//! unpredictable on 1.16.5; 1.8 / 1.12 clients have them built in.

#![cfg_attr(not(feature = "v1_16_5"), allow(dead_code))]

use crate::rules::Version;
use crate::sim::{R, Unpredictable};
use crate::stack::Stack;

/// `AbstractFurnaceBlockEntity.createFuelTimeMap`'s single items (1.16.5).
const FUEL_ITEMS: &[&str] = &[
    "lava_bucket",
    "coal_block",
    "blaze_rod",
    "coal",
    "charcoal",
    "oak_fence",
    "birch_fence",
    "spruce_fence",
    "jungle_fence",
    "dark_oak_fence",
    "acacia_fence",
    "oak_fence_gate",
    "birch_fence_gate",
    "spruce_fence_gate",
    "jungle_fence_gate",
    "dark_oak_fence_gate",
    "acacia_fence_gate",
    "note_block",
    "bookshelf",
    "lectern",
    "jukebox",
    "chest",
    "trapped_chest",
    "crafting_table",
    "daylight_detector",
    "bow",
    "fishing_rod",
    "ladder",
    "wooden_shovel",
    "wooden_sword",
    "wooden_hoe",
    "wooden_axe",
    "wooden_pickaxe",
    "stick",
    "bowl",
    "dried_kelp_block",
    "crossbow",
    "bamboo",
    "dead_bush",
    "scaffolding",
    "loom",
    "barrel",
    "cartography_table",
    "fletching_table",
    "smithing_table",
    "composter",
];

/// The item tags `createFuelTimeMap` adds (all but `non_flammable_wood`).
const FUEL_TAGS: &[&str] = &[
    "logs",
    "planks",
    "wooden_stairs",
    "wooden_slabs",
    "wooden_trapdoors",
    "wooden_pressure_plates",
    "banners",
    "signs",
    "wooden_doors",
    "boats",
    "wool",
    "wooden_buttons",
    "saplings",
    "carpets",
];

/// What the 1.16.5 slot rules read from the server's item tags (item ids, sorted).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ItemTags16 {
    pub beacon_payment: Vec<i32>,
    pub carpets: Vec<i32>,
    /// `createFuelTimeMap().keySet()`: its items and tags minus `non_flammable_wood`.
    pub fuels: Vec<i32>,
}

impl ItemTags16 {
    /// From the item tags of a `tags` packet (`(name, item ids)`).
    pub(crate) fn from_item_tags<'a>(
        v: Version,
        tags: impl IntoIterator<Item = (&'a str, &'a [i32])>,
    ) -> ItemTags16 {
        let mut by_name: std::collections::HashMap<&str, &[i32]> = std::collections::HashMap::new();
        for (name, ids) in tags {
            by_name.insert(name.strip_prefix("minecraft:").unwrap_or(name), ids);
        }
        let get = |n: &str| -> Vec<i32> {
            let mut v = by_name.get(n).map(|x| x.to_vec()).unwrap_or_default();
            v.sort_unstable();
            v.dedup();
            v
        };
        let non_flammable = get("non_flammable_wood");
        let mut fuels: Vec<i32> = FUEL_ITEMS
            .iter()
            .filter_map(|n| (0..2000).find(|&i| v.name(i) == *n))
            .collect();
        for t in FUEL_TAGS {
            fuels.extend(get(t));
        }
        fuels.retain(|i| non_flammable.binary_search(i).is_err());
        fuels.sort_unstable();
        fuels.dedup();
        ItemTags16 {
            beacon_payment: get("beacon_payment_items"),
            carpets: get("carpets"),
            fuels,
        }
    }

    /// The vanilla data pack's sets (the built-in tables), for tests.
    #[cfg(test)]
    pub(crate) fn vanilla(v: Version) -> ItemTags16 {
        let ids = |f: &dyn Fn(&Stack) -> bool| -> Vec<i32> {
            (1..2000).filter(|&i| f(&Stack::new(i, 1, 0))).collect()
        };
        ItemTags16 {
            beacon_payment: ids(&|s| v.beacon_payment(s)),
            carpets: ids(&|s| v.is_carpet(s)),
            fuels: ids(&|s| v.is_fuel(s)),
        }
    }

    pub(crate) fn beacon_payment(&self, s: &Stack) -> bool {
        self.beacon_payment.binary_search(&s.id).is_ok()
    }

    pub(crate) fn carpet(&self, s: &Stack) -> bool {
        self.carpets.binary_search(&s.id).is_ok()
    }

    pub(crate) fn fuel(&self, s: &Stack) -> bool {
        self.fuels.binary_search(&s.id).is_ok()
    }
}

/// The rule with the server's tags (1.16.5: `Err` without them), else the built-in tables.
pub(crate) fn beacon_payment(v: Version, t: Option<&ItemTags16>, s: &Stack) -> R<bool> {
    match t {
        _ if v != Version::V1_16_5 => Ok(v.beacon_payment(s)),
        Some(t) => Ok(t.beacon_payment(s)),
        None => Err(Unpredictable),
    }
}

/// See [`beacon_payment`].
pub(crate) fn fuel(v: Version, t: Option<&ItemTags16>, s: &Stack) -> R<bool> {
    match t {
        _ if v != Version::V1_16_5 => Ok(v.is_fuel(s)),
        Some(t) => Ok(t.fuel(s)),
        None => Err(Unpredictable),
    }
}

/// See [`beacon_payment`].
pub(crate) fn carpet(v: Version, t: Option<&ItemTags16>, s: &Stack) -> R<bool> {
    match t {
        _ if v != Version::V1_16_5 => Ok(v.is_carpet(s)),
        Some(t) => Ok(t.carpet(s)),
        None => Err(Unpredictable),
    }
}

#[cfg(all(test, feature = "v1_16_5"))]
mod tests {
    use super::*;

    /// The vanilla tags of the oracle's `tags` line give the jar's fuel / beacon / carpet sets.
    #[test]
    fn vanilla_tags_match_the_tables() {
        use std::io::BufRead;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden/1.16.5/clicks_menus_seed1.jsonl.gz");
        let f = std::fs::File::open(&path).unwrap();
        let r = std::io::BufReader::new(flate2::read::GzDecoder::new(f));
        let v = Version::V1_16_5;
        for line in r.lines() {
            let line = line.unwrap();
            if !line.starts_with("{\"tags\"") {
                continue;
            }
            let c: serde_json::Value = serde_json::from_str(&line).unwrap();
            let h = c["tags"].as_str().unwrap();
            let b: Vec<u8> = (0..h.len() / 2)
                .map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap())
                .collect();
            let p: ghast_protocol::v1_16_5::play::clientbound::Tags =
                ghast_protocol::codec::decode_exact(&bytes::Bytes::from(b)).unwrap();
            let t = ItemTags16::from_item_tags(
                v,
                p.item_tags.iter().map(|t| (&*t.tag_name, &t.entries[..])),
            );
            for id in 1..1000 {
                let s = Stack::new(id, 1, 0);
                assert_eq!(t.fuel(&s), v.is_fuel(&s), "fuel {id}");
                assert_eq!(t.beacon_payment(&s), v.beacon_payment(&s), "beacon {id}");
                assert_eq!(t.carpet(&s), v.is_carpet(&s), "carpet {id}");
            }
            return;
        }
        panic!("no tags line in {}", path.display());
    }
}
