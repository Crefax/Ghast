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
        WindowKind::BrewingStand => matches!(v, Version::V1_8 | Version::V1_12),
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

impl Version {
    /// The brewing stand's bottle slots (`PotionSlot.matches`): potion (373) and glass bottle
    /// (374), 1.12 also splash (438) and lingering (441) potions; any damage.
    pub(crate) fn potion_slot_accepts(self, s: &Stack) -> bool {
        match self {
            Version::V1_8 => matches!(s.id, 373 | 374),
            Version::V1_12 => matches!(s.id, 373 | 374 | 438 | 441),
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
            _ => false,
        }
    }

    /// 1.12+ brewing stand fuel slot: blaze powder (1.12: 377, any damage).
    pub(crate) fn brewing_fuel(self, s: &Stack) -> bool {
        match self {
            Version::V1_12 => s.id == 377,
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
    pub(crate) fn transfer_brewing(&mut self, i: usize, st: &mut Stack, orig: &Stack) -> R<bool> {
        let v = self.v;
        let n = self.lay.n;
        let len = self.lay.len;
        if i < n {
            return self.insert(st, n, len, true);
        }
        let main_end = n + 27;
        let legacy = v == Version::V1_8;
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
        }
    }
}

/// `Slot.canInsert` of a special menu slot kind (`None`: not a special menu slot).
#[cfg(all(test, any(feature = "v1_8", feature = "v1_12")))]
fn accepts(v: Version, kind: crate::SlotKind, s: &Stack) -> Option<bool> {
    use crate::SlotKind as K;
    Some(match kind {
        K::Potion => v.potion_slot_accepts(s),
        K::BrewingIngredient => v.brewing_ingredient(s),
        K::BrewingFuel => v.brewing_fuel(s),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `{"facts":menu,...}` lines of a version's golden menu clicks: the real slots'
    /// `canInsert` over every item id and damage 0..15, and `getMaxStackAmount`.
    #[cfg(any(feature = "v1_8", feature = "v1_12"))]
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
            let kind = match c["facts"].as_str().unwrap() {
                "brewing" => WindowKind::BrewingStand,
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
                    crate::SlotKind::Potion => 1,
                    _ => 64,
                };
                assert_eq!(got, want, "{kind:?} slot {k} max");
            }
        }
        assert!(menus > 0, "no facts in {}", path.display());
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

    #[test]
    fn tables_sorted() {
        assert!(BREWING_1_8.windows(2).all(|w| w[0] < w[1]));
        assert!(BREWING_1_12.windows(2).all(|w| w[0] < w[1]));
    }
}
