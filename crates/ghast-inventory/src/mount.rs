//! Mount inventories (horse, donkey, mule, zombie / skeleton horse, 1.12+ llama) on 1.8.9,
//! 1.12.2 and 1.16.5: the saddle and armor slots' rules, which depend on the entity, and the
//! screen's shift-click routes.

use crate::rules::Version;
use crate::sim::{R, Sim, Unpredictable};
use crate::stack::Stack;

/// What a mount's saddle and armor slots accept, read off the entity the screen belongs to
/// (vanilla's `HorseScreenHandler` asks the entity). Give it with
/// [`Inventory::set_mount_rules`](crate::Inventory::set_mount_rules); without it only stacks the
/// rules cannot concern (no saddle, horse armor or carpet) are predicted in those slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub struct MountRules {
    /// The saddle slot takes a saddle: 1.8 always; 1.12 every mount but llamas
    /// (`canBeSaddled`); 1.16.5 a tame adult horse, donkey, mule, zombie or skeleton horse.
    pub saddle: bool,
    /// What the armor slot takes.
    pub armor: MountArmor,
}

impl MountRules {
    /// Rules from their parts.
    #[must_use]
    pub fn new(saddle: bool, armor: MountArmor) -> MountRules {
        MountRules { saddle, armor }
    }
}

/// The item a mount wears in its armor slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum MountArmor {
    /// Nothing (donkeys, mules, zombie and skeleton horses).
    #[default]
    None,
    /// Horse armor (1.8: horse type 0; 1.12 / 1.16.5: horses): iron, gold and diamond, 1.16.5
    /// also leather.
    HorseArmor,
    /// Carpets (1.12 / 1.16.5 llamas and trader llamas): any colour.
    Carpet,
}

impl Version {
    pub(crate) fn is_saddle(self, s: &Stack) -> bool {
        match self {
            Version::V1_8 | Version::V1_12 => s.id == 329,
            Version::V1_16_5 => self.name(s.id) == "saddle",
            _ => false,
        }
    }

    pub(crate) fn is_horse_armor(self, s: &Stack) -> bool {
        match self {
            Version::V1_8 | Version::V1_12 => matches!(s.id, 417..=419),
            Version::V1_16_5 => matches!(
                self.name(s.id),
                "leather_horse_armor"
                    | "iron_horse_armor"
                    | "golden_horse_armor"
                    | "diamond_horse_armor"
            ),
            _ => false,
        }
    }

    /// 1.12 the carpet block item (171, any colour); 1.16.5 the vanilla `carpets` tag.
    pub(crate) fn is_carpet(self, s: &Stack) -> bool {
        match self {
            Version::V1_12 => s.id == 171,
            Version::V1_16_5 => self.name(s.id).ends_with("_carpet"),
            _ => false,
        }
    }
}

/// The saddle slot's `canInsert` (an occupied slot takes nothing; `Err`: the rules are needed
/// and unknown).
pub(crate) fn saddle_accepts(
    v: Version,
    rules: Option<MountRules>,
    s: &Stack,
    occupied: bool,
) -> R<bool> {
    if !v.is_saddle(s) || occupied {
        return Ok(false);
    }
    if v == Version::V1_8 {
        return Ok(true);
    }
    rules.map(|r| r.saddle).ok_or(Unpredictable)
}

/// The armor slot's `canInsert`.
pub(crate) fn armor_accepts(v: Version, rules: Option<MountRules>, s: &Stack) -> R<bool> {
    let horse = v.is_horse_armor(s);
    let carpet = v.is_carpet(s);
    if !horse && !carpet {
        return Ok(false);
    }
    Ok(match rules.ok_or(Unpredictable)?.armor {
        MountArmor::None => false,
        MountArmor::HorseArmor => horse,
        MountArmor::Carpet => carpet,
    })
}

impl Sim<'_> {
    pub(crate) fn mount_saddle_accepts(&self, s: &Stack) -> R<bool> {
        saddle_accepts(self.v, self.m.mount, s, self.get(0).is_some())
    }

    pub(crate) fn mount_armor_accepts(&self, s: &Stack) -> R<bool> {
        armor_accepts(self.v, self.m.mount, s)
    }

    /// `HorseScreenHandler.transferSlot`: the mount's slots go to the player part (last slot
    /// first); a player stack goes into an empty armor slot it fits, else the saddle slot, else
    /// the chest. 1.16.5 then moves it between main inventory and hotbar and returns EMPTY
    /// (`Ok(None)` after the move); 1.8 / 1.12 give up. `Ok(Some(ok))`: the ordinary end, which
    /// on this screen never compares counts.
    pub(crate) fn transfer_mount(&mut self, i: usize, st: &mut Stack) -> R<Option<bool>> {
        let (n, len) = (self.lay.n, self.lay.len);
        if i < n {
            return self.insert(st, n, len, true).map(Some);
        }
        if n > 1 && self.mount_armor_accepts(st)? && self.get(1).is_none() {
            return self.insert(st, 1, 2, false).map(Some);
        }
        if n > 0 && self.mount_saddle_accepts(st)? {
            return self.insert(st, 0, 1, false).map(Some);
        }
        if n > 2 && self.insert(st, 2, n, false)? {
            return Ok(Some(true));
        }
        if self.v != Version::V1_16_5 {
            return Ok(Some(false));
        }
        let k = n + 27;
        if (k..len).contains(&i) {
            self.insert(st, n, k, false)?;
        } else {
            self.insert(st, k, len, false)?;
        }
        Ok(None)
    }
}
