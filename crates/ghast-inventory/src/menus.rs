//! The special menus of the legacy versions (1.8.9, 1.12.2, 1.16.5): which of them ghast
//! predicts, their slot rules and their shift-click (`transferSlot`) routes.
//!
//! The item lists below were read off the real slots of each version's client (`Slot.canInsert`
//! over every item id and damage value, dumped by the oracle's `MenuClickDump`); the click
//! replays (`tests/oracle_clicks.rs`) check them against the dump on every run.

use crate::rules::Version;
use crate::sim::{R, Sim};
use crate::stack::Stack;
use crate::window::WindowKind;

/// Whether ghast predicts clicks in this menu on this version (otherwise its own slots are
/// [`SlotKind::Special`](crate::SlotKind::Special) and clicks reading them are refused).
pub(crate) fn modelled(v: Version, kind: WindowKind) -> bool {
    match kind {
        WindowKind::BrewingStand
        | WindowKind::Enchanting
        | WindowKind::Anvil
        | WindowKind::Beacon
        | WindowKind::Merchant
        | WindowKind::Horse { .. } => {
            matches!(v, Version::V1_8 | Version::V1_12 | Version::V1_16_5)
        }
        _ => false,
    }
}

/// 1.8.9 items with a brewing effect string (`Item.hasStatusEffectString`) for every damage
/// value. Sorted.
const BREWING_1_8: &[i32] = &[
    289, 331, 348, 353, 370, 372, 375, 376, 377, 378, 382, 396, 414,
];

/// 1.12.2 brewing ingredients (`PotionHelper.isReagent`: the registered mixes' ingredients),
/// damage 0 only. Sorted.
const BREWING_1_12: &[i32] = &[
    289, 331, 348, 353, 370, 372, 375, 376, 377, 378, 382, 396, 414, 437,
];

/// 1.16.5 brewing ingredients (`BrewingRecipeRegistry.isValidIngredient`: the item and potion
/// mixes' ingredients), by registry name. Sorted.
const BREWING_1_16: &[&str] = &[
    "blaze_powder",
    "dragon_breath",
    "fermented_spider_eye",
    "ghast_tear",
    "glistering_melon_slice",
    "glowstone_dust",
    "golden_carrot",
    "gunpowder",
    "magma_cream",
    "nether_wart",
    "phantom_membrane",
    "pufferfish",
    "rabbit_foot",
    "redstone",
    "spider_eye",
    "sugar",
    "turtle_helmet",
];

impl Version {
    /// The brewing stand's bottle slots (`PotionSlot.matches`): potion (373) and glass bottle
    /// (374), 1.12 also splash (438) and lingering (441) potions; any damage.
    pub(crate) fn potion_slot_accepts(self, s: &Stack) -> bool {
        match self {
            Version::V1_8 => matches!(s.id, 373 | 374),
            Version::V1_12 => matches!(s.id, 373 | 374 | 438 | 441),
            Version::V1_16_5 => matches!(
                self.name(s.id),
                "potion" | "splash_potion" | "lingering_potion" | "glass_bottle"
            ),
            _ => false,
        }
    }

    /// The brewing stand's ingredient slot (1.8 `Item.hasStatusEffectString`: the list above,
    /// plus raw and cooked pufferfish, fish 349 / 350 with damage 3; 1.12 the list above with
    /// damage 0, plus the raw pufferfish).
    pub(crate) fn brewing_ingredient(self, s: &Stack) -> bool {
        match self {
            Version::V1_8 => match s.id {
                349 | 350 => s.damage == 3,
                id => BREWING_1_8.binary_search(&id).is_ok(),
            },
            Version::V1_12 => match s.id {
                349 => s.damage == 3,
                id => s.damage == 0 && BREWING_1_12.binary_search(&id).is_ok(),
            },
            Version::V1_16_5 => BREWING_1_16.binary_search(&self.name(s.id)).is_ok(),
            _ => false,
        }
    }

    /// The enchanting table's lapis slot: 1.8 / 1.12 dye (351) with damage 4 (`DyeColor.BLUE`),
    /// 1.16.5 `lapis_lazuli`.
    pub(crate) fn is_lapis(self, s: &Stack) -> bool {
        match self {
            Version::V1_8 | Version::V1_12 => s.id == 351 && s.damage == 4,
            Version::V1_16_5 => self.name(s.id) == "lapis_lazuli",
            _ => false,
        }
    }

    /// The beacon's payment slot (`PaymentSlot.canInsert`): 1.8 / 1.12 emerald (388), diamond
    /// (264), iron ingot (265), gold ingot (266), any damage; 1.16.5 the vanilla
    /// `beacon_payment_items` tag (those four and the netherite ingot). The 1.16.5 client reads
    /// the tag the server synchronised; ghast assumes the vanilla tag.
    pub(crate) fn beacon_payment(self, s: &Stack) -> bool {
        match self {
            Version::V1_8 | Version::V1_12 => matches!(s.id, 264 | 265 | 266 | 388),
            Version::V1_16_5 => matches!(
                self.name(s.id),
                "emerald" | "diamond" | "iron_ingot" | "gold_ingot" | "netherite_ingot"
            ),
            _ => false,
        }
    }

    /// 1.12+ brewing stand fuel slot: blaze powder (1.12: 377, any damage).
    pub(crate) fn brewing_fuel(self, s: &Stack) -> bool {
        match self {
            Version::V1_12 => s.id == 377,
            Version::V1_16_5 => self.name(s.id) == "blaze_powder",
            _ => false,
        }
    }
}

impl Sim<'_> {
    /// `BrewingScreenHandler.transferSlot`'s routes (0..=2 bottles, 3 ingredient, 1.12+ 4 fuel,
    /// then the player's 27 + 9): a menu slot goes to the player part (last slot first); a player
    /// stack goes to the ingredient slot if it is an ingredient (1.8: only into an empty one),
    /// else to the bottle slots if it is a bottle (1.12: a single one), else (1.12) to the fuel
    /// slot if it is fuel, else between main inventory and hotbar. 1.8's `insertItem` ignores the
    /// slots' rules and limits, so a stack of glass bottles lands whole in one bottle slot.
    ///
    /// 1.16.5 tries blaze powder as fuel first: when it fits there vanilla returns EMPTY at once
    /// (`Ok(None)`: the moved items stay moved, no retry); when the fuel slot is full it goes on
    /// as an ingredient. `Ok(Some(ok))`: the ordinary end of `transferSlot`.
    pub(crate) fn transfer_brewing(
        &mut self,
        i: usize,
        st: &mut Stack,
        orig: &Stack,
    ) -> R<Option<bool>> {
        let v = self.v;
        let n = self.lay.n;
        let len = self.lay.len;
        if i < n {
            return self.insert(st, n, len, true).map(Some);
        }
        if v == Version::V1_16_5 && v.brewing_fuel(orig) {
            if self.insert(st, 4, 5, false)? {
                return Ok(None);
            }
            // not an ingredient: nothing moved, the ordinary end (counts equal: EMPTY)
            return Ok(Some(
                !v.brewing_ingredient(st) || self.insert(st, 3, 4, false)?,
            ));
        }
        let main_end = n + 27;
        let legacy = v == Version::V1_8;
        Ok(Some(
            if (!legacy || self.get(3).is_none()) && v.brewing_ingredient(orig) {
                self.insert(st, 3, 4, false)
            } else if v.potion_slot_accepts(orig) && (legacy || orig.count == 1) {
                self.insert(st, 0, 3, false)
            } else if !legacy && v.brewing_fuel(orig) {
                self.insert(st, 4, 5, false)
            } else if (n..main_end).contains(&i) {
                self.insert(st, main_end, len, false)
            } else if (main_end..len).contains(&i) {
                self.insert(st, n, main_end, false)
            } else {
                self.insert(st, n, len, false)
            }?,
        ))
    }
}

impl Sim<'_> {
    /// `BeaconScreenHandler.transferSlot` (payment slot 0, then the player's 27 + 9): the
    /// payment goes to the player part (last slot first); a single payment item goes into an
    /// empty payment slot; anything else between main inventory and hotbar.
    pub(crate) fn transfer_beacon(&mut self, i: usize, st: &mut Stack) -> R<bool> {
        let len = self.lay.len;
        if i == 0 {
            return self.insert(st, 1, len, true);
        }
        if self.get(0).is_none() && self.v.beacon_payment(st) && st.count == 1 {
            return self.insert(st, 0, 1, false);
        }
        if i < 28 {
            self.insert(st, 28, len, false)
        } else {
            self.insert(st, 1, 28, false)
        }
    }
}

impl Sim<'_> {
    /// `EnchantingScreenHandler.transferSlot`: a menu slot goes to the player part (last slot
    /// first); lapis to the lapis slot; anything else, when the item slot is empty, puts one item
    /// there (1.8 / 1.12: a copy with its NBT only when the stack is a single item, otherwise a
    /// fresh stack of the item and damage without NBT; 1.16: always a copy with the NBT).
    pub(crate) fn transfer_enchanting(&mut self, i: usize, st: &mut Stack) -> R<bool> {
        let v = self.v;
        let (n, len) = (self.lay.n, self.lay.len);
        if i < n {
            return self.insert(st, n, len, true);
        }
        if v.is_lapis(st) {
            return self.insert(st, 1, 2, true);
        }
        if self.get(0).is_some() {
            return Ok(false);
        }
        let one = match v {
            Version::V1_8 | Version::V1_12
                if !matches!(st.data, crate::stack::ItemData::Nbt(_)) || st.count != 1 =>
            {
                if st.count < 1 {
                    // 1.8 zero-count stack: nothing is placed (1.12 never gets here empty)
                    return Ok(true);
                }
                Stack::new(st.id, 1, st.damage)
            }
            _ => st.with_count(1),
        };
        st.count -= 1;
        self.set(0, Some(one));
        Ok(true)
    }
}

#[cfg(all(test, any(feature = "v1_8", feature = "v1_12", feature = "v1_16_5")))]
fn accepts(v: Version, kind: crate::SlotKind, s: &Stack) -> Option<bool> {
    use crate::SlotKind as K;
    Some(match kind {
        K::Potion => v.potion_slot_accepts(s),
        K::BrewingIngredient => v.brewing_ingredient(s),
        K::BrewingFuel => v.brewing_fuel(s),
        K::EnchantItem => true,
        K::Lapis => v.is_lapis(s),
        K::AnvilOutput => false,
        K::BeaconPayment => v.beacon_payment(s),
        K::TradeOutput => false,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `{"facts":menu,...}` lines of a version's golden menu clicks: the real slots'
    /// `canInsert` over every item id and damage 0..15, and `getMaxStackAmount`.
    #[cfg(any(feature = "v1_8", feature = "v1_12", feature = "v1_16_5"))]
    fn check_facts(v: Version, dir: &str) {
        use std::io::BufRead;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden")
            .join(dir)
            .join("clicks_menus_seed1.jsonl.gz");
        let f = std::fs::File::open(&path).unwrap();
        let r = std::io::BufReader::new(flate2::read::GzDecoder::new(f));
        let mut menus = 0;
        for line in r.lines() {
            let line = line.unwrap();
            if !line.starts_with("{\"facts\"") {
                continue;
            }
            let c: serde_json::Value = serde_json::from_str(&line).unwrap();
            if c["facts"] == "horse" {
                check_mount_facts(v, &c);
                menus += 1;
                continue;
            }
            let kind = match c["facts"].as_str().unwrap() {
                "brewing" => WindowKind::BrewingStand,
                "enchanting" => WindowKind::Enchanting,
                "anvil" => WindowKind::Anvil,
                "beacon" => WindowKind::Beacon,
                "merchant" => WindowKind::Merchant,
                m => panic!("unknown menu {m}"),
            };
            assert!(modelled(v, kind), "{kind:?} facts but not modelled");
            menus += 1;
            for (k, list) in c["accepts"].as_array().unwrap().iter().enumerate() {
                let sk = kind.slot_kind(k);
                let mut want = std::collections::HashMap::new();
                for e in list.as_array().unwrap() {
                    want.insert(e[0].as_i64().unwrap() as i32, e[1].as_u64().unwrap() as u32);
                }
                for id in 0..4096 {
                    if id == 0 {
                        // air (an item in 1.12+) is an empty slot, never a stack
                        continue;
                    }
                    if !v.known(id) {
                        // ids the jar has no item for are never offered
                        assert!(!want.contains_key(&id), "{kind:?} slot {k} id {id}");
                        continue;
                    }
                    for d in 0..16i16 {
                        let s = Stack::new(id, 1, d);
                        let w = want.get(&id).is_some_and(|m| m & (1 << d) != 0);
                        match accepts(v, sk, &s) {
                            Some(got) => assert_eq!(got, w, "{kind:?} slot {k} {id}:{d}"),
                            None => assert!(sk == crate::SlotKind::Normal, "{kind:?} slot {k}"),
                        }
                    }
                }
            }
            for (k, m) in c["max"].as_array().unwrap().iter().enumerate() {
                let want = m.as_i64().unwrap() as i32;
                let got = match kind.slot_kind(k) {
                    crate::SlotKind::Potion
                    | crate::SlotKind::EnchantItem
                    | crate::SlotKind::BeaconPayment => 1,
                    _ => 64,
                };
                assert_eq!(got, want, "{kind:?} slot {k} max");
            }
        }
        assert!(menus > 0, "no facts in {}", path.display());
    }

    /// The mount rules vanilla's entity gives (the facts line's or a case's `mount`): 1.8
    /// horse `type`; 1.12 / 1.16.5 `entity` name, 1.16.5 also `tame` and `baby`.
    #[cfg(any(feature = "v1_8", feature = "v1_12", feature = "v1_16_5"))]
    fn mount_rules_of(v: Version, m: &serde_json::Value) -> crate::MountRules {
        use crate::{MountArmor, MountRules};
        if v == Version::V1_8 {
            let t = m["type"].as_i64().unwrap();
            return MountRules::new(
                true,
                if t == 0 {
                    MountArmor::HorseArmor
                } else {
                    MountArmor::None
                },
            );
        }
        let e = m["entity"].as_str().unwrap();
        let llama = matches!(e, "llama" | "trader_llama");
        let saddle = !llama && (v != Version::V1_16_5 || (m["tame"] == true && m["baby"] == false));
        let armor = match e {
            "horse" => MountArmor::HorseArmor,
            _ if llama => MountArmor::Carpet,
            _ => MountArmor::None,
        };
        MountRules::new(saddle, armor)
    }

    /// A mount's facts line: the saddle and armor slots' `canInsert` for that entity (chest
    /// slots are plain).
    #[cfg(any(feature = "v1_8", feature = "v1_12", feature = "v1_16_5"))]
    fn check_mount_facts(v: Version, c: &serde_json::Value) {
        let rules = Some(mount_rules_of(v, c));
        for (k, list) in c["accepts"].as_array().unwrap().iter().enumerate() {
            let mut want = std::collections::HashMap::new();
            for e in list.as_array().unwrap() {
                want.insert(e[0].as_i64().unwrap() as i32, e[1].as_u64().unwrap() as u32);
            }
            for id in 1..4096 {
                if !v.known(id) {
                    continue;
                }
                for d in 0..16i16 {
                    let s = Stack::new(id, 1, d);
                    let w = want.get(&id).is_some_and(|m| m & (1 << d) != 0);
                    let got = match k {
                        0 => crate::mount::saddle_accepts(v, rules, &s, false).unwrap(),
                        1 => crate::mount::armor_accepts(v, rules, &s).unwrap(),
                        _ => true,
                    };
                    assert_eq!(got, w, "{c} slot {k} {id}:{d}");
                    // without the rules: only what the rules decide is unknown
                    if k < 2 {
                        let blind = match k {
                            0 => crate::mount::saddle_accepts(v, None, &s, false),
                            _ => crate::mount::armor_accepts(v, None, &s),
                        };
                        assert!(
                            blind.map_or(true, |b| b == w),
                            "{c} slot {k} {id}:{d} blind"
                        );
                    }
                }
            }
        }
        let max: Vec<i64> = c["max"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m.as_i64().unwrap())
            .collect();
        assert_eq!(
            max[1],
            if v == Version::V1_8 { 64 } else { 1 },
            "{c} armor max"
        );
    }

    #[cfg(feature = "v1_8")]
    #[test]
    fn facts_1_8_9() {
        check_facts(Version::V1_8, "1.8.9");
    }

    #[cfg(feature = "v1_12")]
    #[test]
    fn facts_1_12_2() {
        check_facts(Version::V1_12, "1.12.2");
    }

    #[cfg(feature = "v1_16_5")]
    #[test]
    fn facts_1_16_5() {
        check_facts(Version::V1_16_5, "1.16.5");
    }

    #[test]
    fn tables_sorted() {
        assert!(BREWING_1_16.windows(2).all(|w| w[0] < w[1]));
        assert!(BREWING_1_8.windows(2).all(|w| w[0] < w[1]));
        assert!(BREWING_1_12.windows(2).all(|w| w[0] < w[1]));
    }
}
