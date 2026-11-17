//! The 1.16.5 anvil like its client: `AnvilScreenHandler.updateResult` (repair with
//! materials, combining two items or a book, enchantment compatibility and levels, renaming,
//! the level cost), taking the result (level check, the inputs used up) and the anvil screen's
//! name field (`AnvilScreen.onRenamed`: it follows the first input and sends `rename_item`).
//! The enchantment and repair data were read off the jar (unit-tested against the dump).

#![cfg_attr(not(feature = "v1_16_5"), allow(dead_code))]

use ghast_protocol::nbt::{Compound, Tag};

use crate::nbt16 as n;
use crate::rules::Version;
use crate::sim::{R, Sim, Unpredictable};
use crate::stack::{Stack, last};

/// Per enchantment (registry id order, as [`ENCHANTMENTS_1_16`]): max level, the anvil's cost
/// weight by rarity (common 1, uncommon 2, rare 4, very rare 8), the enchantments it cannot
/// combine with (bits by registry id, itself included). Read off the jar (unit-tested).
pub(crate) const ANVIL_ENCHANTMENTS_1_16: &[(i32, i32, u64)] = &[
    (4, 1, 0x1b),         // protection
    (4, 2, 0x1b),         // fire_protection
    (4, 2, 0x4),          // feather_falling
    (4, 4, 0x1b),         // blast_protection
    (4, 2, 0x1b),         // projectile_protection
    (3, 4, 0x20),         // respiration
    (1, 4, 0x40),         // aqua_affinity
    (3, 8, 0x80),         // thorns
    (3, 4, 0x300),        // depth_strider
    (2, 4, 0x300),        // frost_walker
    (1, 8, 0x400),        // binding_curse
    (3, 8, 0x800),        // soul_speed
    (5, 1, 0x7000),       // sharpness
    (5, 2, 0x7000),       // smite
    (5, 2, 0x7000),       // bane_of_arthropods
    (2, 2, 0x8000),       // knockback
    (2, 4, 0x10000),      // fire_aspect
    (3, 4, 0x120000),     // looting
    (3, 4, 0x40000),      // sweeping
    (5, 1, 0x80000),      // efficiency
    (1, 8, 0x8520000),    // silk_touch
    (3, 2, 0x200000),     // unbreaking
    (3, 4, 0x500000),     // fortune
    (5, 1, 0x800000),     // power
    (2, 4, 0x1000000),    // punch
    (1, 4, 0x2000000),    // flame
    (1, 8, 0x1004000000), // infinity
    (3, 4, 0x8100000),    // luck_of_the_sea
    (3, 4, 0x10000000),   // lure
    (3, 2, 0xa0000000),   // loyalty
    (5, 4, 0x40000000),   // impaling
    (3, 4, 0x1a0000000),  // riptide
    (1, 8, 0x180000000),  // channeling
    (1, 4, 0xa00000000),  // multishot
    (3, 2, 0x400000000),  // quick_charge
    (4, 1, 0xa00000000),  // piercing
    (1, 4, 0x1004000000), // mending
    (1, 8, 0x2000000000), // vanishing_curse
];

/// The items each enchantment accepts (`isAcceptableItem`), by registry id order, item ids sorted.
pub(crate) const ENCHANTABLE_1_16: &[&[i32]] = &[
    &[
        570, 622, 623, 624, 625, 626, 627, 628, 629, 630, 631, 632, 633, 634, 635, 636, 637, 638,
        639, 640, 641, 642, 643, 644, 645,
    ],
    &[
        570, 622, 623, 624, 625, 626, 627, 628, 629, 630, 631, 632, 633, 634, 635, 636, 637, 638,
        639, 640, 641, 642, 643, 644, 645,
    ],
    &[625, 629, 633, 637, 641, 645],
    &[
        570, 622, 623, 624, 625, 626, 627, 628, 629, 630, 631, 632, 633, 634, 635, 636, 637, 638,
        639, 640, 641, 642, 643, 644, 645,
    ],
    &[
        570, 622, 623, 624, 625, 626, 627, 628, 629, 630, 631, 632, 633, 634, 635, 636, 637, 638,
        639, 640, 641, 642, 643, 644, 645,
    ],
    &[570, 622, 626, 630, 634, 638, 642],
    &[570, 622, 626, 630, 634, 638, 642],
    &[
        570, 622, 623, 624, 625, 626, 627, 628, 629, 630, 631, 632, 633, 634, 635, 636, 637, 638,
        639, 640, 641, 642, 643, 644, 645,
    ],
    &[625, 629, 633, 637, 641, 645],
    &[625, 629, 633, 637, 641, 645],
    &[
        217, 225, 570, 622, 623, 624, 625, 626, 627, 628, 629, 630, 631, 632, 633, 634, 635, 636,
        637, 638, 639, 640, 641, 642, 643, 644, 645, 836, 837, 838, 839, 840, 841, 898,
    ],
    &[625, 629, 633, 637, 641, 645],
    &[583, 586, 588, 591, 593, 596, 598, 601, 603, 606, 608, 611],
    &[583, 586, 588, 591, 593, 596, 598, 601, 603, 606, 608, 611],
    &[583, 586, 588, 591, 593, 596, 598, 601, 603, 606, 608, 611],
    &[583, 588, 593, 598, 603, 608],
    &[583, 588, 593, 598, 603, 608],
    &[583, 588, 593, 598, 603, 608],
    &[583, 588, 593, 598, 603, 608],
    &[
        584, 585, 586, 587, 589, 590, 591, 592, 594, 595, 596, 597, 599, 600, 601, 602, 604, 605,
        606, 607, 609, 610, 611, 612, 734,
    ],
    &[
        584, 585, 586, 587, 589, 590, 591, 592, 594, 595, 596, 597, 599, 600, 601, 602, 604, 605,
        606, 607, 609, 610, 611, 612,
    ],
    &[
        570, 572, 574, 583, 584, 585, 586, 587, 588, 589, 590, 591, 592, 593, 594, 595, 596, 597,
        598, 599, 600, 601, 602, 603, 604, 605, 606, 607, 608, 609, 610, 611, 612, 622, 623, 624,
        625, 626, 627, 628, 629, 630, 631, 632, 633, 634, 635, 636, 637, 638, 639, 640, 641, 642,
        643, 644, 645, 684, 734, 842, 843, 897, 898, 922, 926,
    ],
    &[
        584, 585, 586, 587, 589, 590, 591, 592, 594, 595, 596, 597, 599, 600, 601, 602, 604, 605,
        606, 607, 609, 610, 611, 612,
    ],
    &[574],
    &[574],
    &[574],
    &[574],
    &[684],
    &[684],
    &[922],
    &[922],
    &[922],
    &[922],
    &[926],
    &[926],
    &[926],
    &[
        570, 572, 574, 583, 584, 585, 586, 587, 588, 589, 590, 591, 592, 593, 594, 595, 596, 597,
        598, 599, 600, 601, 602, 603, 604, 605, 606, 607, 608, 609, 610, 611, 612, 622, 623, 624,
        625, 626, 627, 628, 629, 630, 631, 632, 633, 634, 635, 636, 637, 638, 639, 640, 641, 642,
        643, 644, 645, 684, 734, 842, 843, 897, 898, 922, 926,
    ],
    &[
        217, 225, 570, 572, 574, 583, 584, 585, 586, 587, 588, 589, 590, 591, 592, 593, 594, 595,
        596, 597, 598, 599, 600, 601, 602, 603, 604, 605, 606, 607, 608, 609, 610, 611, 612, 622,
        623, 624, 625, 626, 627, 628, 629, 630, 631, 632, 633, 634, 635, 636, 637, 638, 639, 640,
        641, 642, 643, 644, 645, 683, 684, 734, 836, 837, 838, 839, 840, 841, 842, 843, 897, 898,
        922, 926,
    ],
];

/// Damageable items and what repairs them (`Item.canRepair`, vanilla tags), item ids sorted.
pub(crate) const REPAIR_1_16: &[(i32, &[i32])] = &[
    (570, &[571]),
    (572, &[]),
    (574, &[]),
    (583, &[15, 16, 17, 18, 19, 20, 21, 22]),
    (584, &[15, 16, 17, 18, 19, 20, 21, 22]),
    (585, &[15, 16, 17, 18, 19, 20, 21, 22]),
    (586, &[15, 16, 17, 18, 19, 20, 21, 22]),
    (587, &[15, 16, 17, 18, 19, 20, 21, 22]),
    (588, &[14, 963]),
    (589, &[14, 963]),
    (590, &[14, 963]),
    (591, &[14, 963]),
    (592, &[14, 963]),
    (593, &[580]),
    (594, &[580]),
    (595, &[580]),
    (596, &[580]),
    (597, &[580]),
    (598, &[579]),
    (599, &[579]),
    (600, &[579]),
    (601, &[579]),
    (602, &[579]),
    (603, &[578]),
    (604, &[578]),
    (605, &[578]),
    (606, &[578]),
    (607, &[578]),
    (608, &[581]),
    (609, &[581]),
    (610, &[581]),
    (611, &[581]),
    (612, &[581]),
    (622, &[668]),
    (623, &[668]),
    (624, &[668]),
    (625, &[668]),
    (626, &[579]),
    (627, &[579]),
    (628, &[579]),
    (629, &[579]),
    (630, &[579]),
    (631, &[579]),
    (632, &[579]),
    (633, &[579]),
    (634, &[578]),
    (635, &[578]),
    (636, &[578]),
    (637, &[578]),
    (638, &[580]),
    (639, &[580]),
    (640, &[580]),
    (641, &[580]),
    (642, &[581]),
    (643, &[581]),
    (644, &[581]),
    (645, &[581]),
    (684, &[]),
    (734, &[]),
    (842, &[]),
    (843, &[]),
    (897, &[15, 16, 17, 18, 19, 20, 21, 22]),
    (898, &[923]),
    (922, &[]),
    (926, &[]),
];

/// The anvil screen's state on the client.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Anvil {
    /// `newItemName` (`None`: never set, blank).
    pub name: Option<String>,
    /// The handler's tracked copy of slot 0 (`sendContentUpdates` tells the screen when the
    /// slot differs from it).
    pub tracked: Option<Stack>,
    /// `levelCost`.
    pub cost: i32,
    /// `repairItemUsage`.
    pub usage: i32,
}

/// `StringUtils.isBlank`.
fn blank(s: Option<&str>) -> bool {
    s.is_none_or(|s| s.chars().all(char::is_whitespace))
}

/// `Text.getString()` of a custom name's JSON: plain strings, `text` with `extra`; anything
/// else (translations, scores, ...) is not modelled.
fn plain(json: &str) -> R<String> {
    let v = json::parse(json).ok_or(Unpredictable)?;
    let mut out = String::new();
    walk(&v, &mut out)?;
    Ok(out)
}

fn walk(v: &json::Value, out: &mut String) -> R<()> {
    use json::Value as V;
    match v {
        V::Str(s) => out.push_str(s),
        V::Num(s) => out.push_str(s),
        V::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        // an empty array is an error for Text.Serializer
        V::Arr(a) if a.is_empty() => return Err(Unpredictable),
        V::Arr(a) => {
            for x in a {
                walk(x, out)?;
            }
        }
        V::Obj(o) => {
            let mut text = None;
            for (k, x) in o {
                match k.as_str() {
                    "text" => text = Some(x),
                    "extra" | "color" | "bold" | "italic" | "underlined" | "strikethrough"
                    | "obfuscated" | "font" | "insertion" | "clickEvent" | "hoverEvent" => {}
                    _ => return Err(Unpredictable),
                }
            }
            match text {
                Some(V::Str(s)) => out.push_str(s),
                _ => return Err(Unpredictable),
            }
            if let Some((_, e)) = o.iter().find(|(k, _)| k == "extra") {
                walk(e, out)?;
            }
        }
        V::Null => return Err(Unpredictable),
    }
    Ok(())
}

/// `Text.Serializer.toJson(new LiteralText(name))`.
fn literal_json(name: &str) -> R<String> {
    let mut s = String::from("{\"text\":\"");
    for c in name.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                return Err(Unpredictable);
            }
            c => s.push(c),
        }
    }
    s.push_str("\"}");
    Ok(s)
}

/// The custom name (`display.Name` string) of a tag.
fn custom_name(t: Option<&Compound>) -> Option<&str> {
    match t.and_then(|c| last(c, "display")) {
        Some(Tag::Compound(d)) => match last(d, "Name") {
            Some(Tag::String(s)) => Some(s),
            _ => None,
        },
        _ => None,
    }
}

/// `getName().getString()` when the stack has a custom name (`None`: the translated item name,
/// which ghast does not know).
fn custom_plain(t: Option<&Compound>) -> R<Option<String>> {
    custom_name(t).map(plain).transpose()
}

fn acceptable(e: usize, id: i32) -> bool {
    ENCHANTABLE_1_16
        .get(e)
        .is_some_and(|l| l.binary_search(&id).is_ok())
}

fn can_combine(e: usize, o: usize) -> bool {
    ANVIL_ENCHANTMENTS_1_16
        .get(e)
        .is_some_and(|x| x.2 & (1u64 << o) == 0)
}

fn can_repair(id: i32, with: i32) -> bool {
    REPAIR_1_16
        .binary_search_by_key(&id, |r| r.0)
        .is_ok_and(|k| REPAIR_1_16[k].1.binary_search(&with).is_ok())
}

/// `isDamageable` of a stack with tag `t`.
fn damageable(v: Version, id: i32, t: Option<&Compound>) -> bool {
    v.max_durability(id).is_some()
        && !t.is_some_and(|c| {
            last(c, "Unbreakable")
                .and_then(crate::stack::number)
                .is_some_and(|x| x as i8 != 0)
        })
}

/// `setDamage(d)` (through `getOrCreateTag`).
fn set_damage(t: &mut Option<Compound>, durable: bool, d: i32) {
    n::put(n::create(t, durable), "Damage", Tag::Int(d.max(0)));
}

/// The result of `updateResult` and the state it leaves.
struct Outcome {
    out: Option<Stack>,
    cost: i32,
    usage: i32,
    /// Whether it ended with `sendContentUpdates` (the screen hears about slot 0).
    synced: bool,
}

fn update_result(
    v: Version,
    a: Option<&Stack>,
    b: Option<&Stack>,
    name: Option<&str>,
    creative: bool,
) -> R<Outcome> {
    let empty = |usage| Outcome {
        out: None,
        cost: 0,
        usage,
        synced: false,
    };
    let Some(a) = a else {
        return Ok(empty(0));
    };
    let ta = n::tree(a)?;
    let tb = b.map(n::tree).transpose()?.flatten();
    let durable = v.max_durability(a.id).is_some();
    let max = v.max_durability(a.id).unwrap_or(0);
    let mut t2 = ta.clone();
    let mut map = n::enchantments(v, a, ta.as_ref());
    let mut i = 0i32;
    let j = n::repair_cost(ta.as_ref()) + b.map_or(0, |_| n::repair_cost(tb.as_ref()));
    let mut k = 0;
    let mut usage = 0;
    if let Some(b) = b {
        let book = v.name(b.id) == "enchanted_book"
            && tb
                .as_ref()
                .is_some_and(|t| !n::compound_list(t, "StoredEnchantments").is_empty());
        if damageable(v, a.id, t2.as_ref()) && can_repair(a.id, b.id) {
            let mut l = n::damage(t2.as_ref()).min(max / 4);
            if l <= 0 {
                return Ok(empty(0));
            }
            let mut m = 0;
            while l > 0 && m < b.count {
                let d = n::damage(t2.as_ref()) - l;
                set_damage(&mut t2, durable, d);
                i += 1;
                l = n::damage(t2.as_ref()).min(max / 4);
                m += 1;
            }
            usage = m;
        } else {
            if !book && (a.id != b.id || !damageable(v, a.id, t2.as_ref())) {
                return Ok(empty(0));
            }
            if damageable(v, a.id, t2.as_ref()) && !book {
                let o = max - n::damage(ta.as_ref());
                let p = v.max_durability(b.id).unwrap_or(0) - n::damage(tb.as_ref());
                let q = p + max * 12 / 100;
                let s = (max - (o + q)).max(0);
                if s < n::damage(t2.as_ref()) {
                    set_damage(&mut t2, durable, s);
                    i += 2;
                }
            }
            let mut any_ok = false;
            let mut any_bad = false;
            for (e, lvl) in n::enchantments(v, b, tb.as_ref()) {
                let t = map.iter().find(|x| x.0 == e).map_or(0, |x| x.1);
                let mut u = if t == lvl { lvl + 1 } else { lvl.max(t) };
                let mut ok = acceptable(e, a.id) || creative || v.name(a.id) == "enchanted_book";
                for &(e2, _) in &map {
                    if e2 != e && !can_combine(e, e2) {
                        ok = false;
                        i += 1;
                    }
                }
                if !ok {
                    any_bad = true;
                } else {
                    any_ok = true;
                    let (maxl, weight, _) = ANVIL_ENCHANTMENTS_1_16[e];
                    u = u.min(maxl);
                    match map.iter_mut().find(|x| x.0 == e) {
                        Some(x) => x.1 = u,
                        None => map.push((e, u)),
                    }
                    let w = if book { (weight / 2).max(1) } else { weight };
                    i += w * u;
                    if a.count > 1 {
                        i = 40;
                    }
                }
            }
            if any_bad && !any_ok {
                return Ok(empty(usage));
            }
        }
    }
    if blank(name) {
        if custom_name(ta.as_ref()).is_some() {
            k = 1;
            i += 1;
            // removeCustomName
            if let Some(c) = &mut t2
                && let Some(Tag::Compound(d)) =
                    c.0.iter_mut()
                        .rev()
                        .find(|x| x.0 == "display")
                        .map(|x| &mut x.1)
            {
                n::remove(d, "Name");
                if d.0.is_empty() {
                    n::remove_sub(&mut t2, "display");
                }
            }
            if t2.as_ref().is_some_and(|c| c.0.is_empty()) {
                t2 = None;
            }
        }
    } else {
        let name = name.unwrap_or_default();
        // getName().getString(): the custom name, or the translated item name (taken as
        // different from what was typed)
        if custom_plain(ta.as_ref())?.as_deref() != Some(name) {
            k = 1;
            i += 1;
            let json = literal_json(name)?;
            let c = n::create(&mut t2, durable);
            let mut d = match last(c, "display") {
                Some(Tag::Compound(d)) => d.clone(),
                _ => Compound::default(),
            };
            n::put(&mut d, "Name", Tag::String(json));
            n::put(c, "display", Tag::Compound(d));
        }
    }
    let mut cost = j + i;
    let mut keep = i > 0;
    if k == i && k > 0 && cost >= 40 {
        cost = 39;
    }
    if cost >= 40 && !creative {
        keep = false;
    }
    let out = if keep {
        let mut w = n::repair_cost(t2.as_ref());
        if b.is_some() && w < n::repair_cost(tb.as_ref()) {
            w = n::repair_cost(tb.as_ref());
        }
        if k != i || k == 0 {
            w = w * 2 + 1;
        }
        n::put(n::create(&mut t2, durable), "RepairCost", Tag::Int(w));
        // EnchantmentHelper.set(map, stack)
        let book = v.name(a.id) == "enchanted_book";
        let mut list = Vec::new();
        for &(e, lvl) in &map {
            list.push(n::entry(e, lvl as i16));
            if book {
                n::add_stored(&mut t2, durable, e, lvl);
            }
        }
        if list.is_empty() {
            n::remove_sub(&mut t2, "Enchantments");
        } else if !book {
            n::put(
                n::create(&mut t2, durable),
                "Enchantments",
                n::list_tag(list),
            );
        }
        Some(n::with_tree(a.id, a.count, t2)?)
    } else {
        None
    };
    Ok(Outcome {
        out,
        cost,
        usage,
        synced: true,
    })
}

impl Sim<'_> {
    fn anvil_state(&self) -> Anvil {
        match self.m.station.as_deref() {
            Some(crate::station::Station::Anvil(a)) => a.clone(),
            _ => Anvil::default(),
        }
    }

    fn set_anvil_state(&mut self, a: Anvil) {
        if let Some(crate::station::Station::Anvil(s)) = self.m.station.as_deref_mut() {
            *s = a;
        }
    }

    /// `AnvilScreenHandler.updateResult`, then (when it syncs) the screen's slot-0 listener.
    pub(crate) fn anvil_update(&mut self) {
        if matches!(self.v, Version::V1_12 | Version::V1_8) {
            return self.anvil12_update();
        }
        let mut st = self.anvil_state();
        let r = update_result(
            self.v,
            self.m.container[0].as_ref(),
            self.m.container[1].as_ref(),
            st.name.as_deref(),
            self.m.creative,
        );
        let o = match r {
            Ok(o) => o,
            Err(Unpredictable) => {
                self.m.container_result_known = false;
                return;
            }
        };
        // levelCost is set to 1 first, then to the outcome's
        st.cost = o.cost;
        st.usage = o.usage;
        self.m.container[2] = o.out;
        self.m.container_result_known = true;
        self.set_anvil_state(st);
        if o.synced {
            self.anvil_sync();
        }
    }

    /// `sendContentUpdates` as the anvil screen sees it: slot 0 differs from the tracked copy
    /// → the name field gets the item's name → `onRenamed`.
    pub(crate) fn anvil_sync(&mut self) {
        if matches!(self.v, Version::V1_12 | Version::V1_8) {
            return self.anvil12_sync();
        }
        let mut st = self.anvil_state();
        let cur = self.m.container[0].clone();
        if crate::stack::stacks_equal(cur.as_ref(), st.tracked.as_ref()) {
            return;
        }
        st.tracked.clone_from(&cur);
        self.set_anvil_state(st);
        let Some(cur) = cur else {
            // setText(""): onRenamed ignores an empty name
            return;
        };
        let t = match n::tree(&cur) {
            Ok(t) => t,
            Err(_) => {
                self.m.container_result_known = false;
                return;
            }
        };
        // name field = getName().getString(); onRenamed: not a custom name → "" (it equals the
        // item's own name), else the custom name
        let name = match custom_plain(t.as_ref()) {
            Ok(Some(p)) => p,
            Ok(None) => String::new(),
            Err(_) => {
                self.m.container_result_known = false;
                return;
            }
        };
        if custom_name(t.as_ref()).is_none() || !name.is_empty() {
            // the custom name's plain text can be empty: the field then stays as it was
            self.anvil_set_name(name);
        }
    }

    /// `setNewItemName` + `rename_item`.
    pub(crate) fn anvil_set_name(&mut self, name: String) {
        if matches!(self.v, Version::V1_12 | Version::V1_8) {
            return self.anvil12_set_name(name);
        }
        let mut st = self.anvil_state();
        st.name = Some(name.clone());
        self.set_anvil_state(st);
        self.m.screen_out.push(crate::Outgoing::RenameItem(name));
        self.anvil_update();
    }

    /// `canTakeOutput`.
    pub(crate) fn anvil_can_take(&self) -> bool {
        let st = self.anvil_state();
        (self.m.creative || self.m.xp_level >= st.cost) && st.cost > 0
    }

    /// `onTakeOutput`: the levels, the inputs used up (the anvil's damage is the server's).
    pub(crate) fn anvil_take(&mut self) {
        let st = self.anvil_state();
        if !self.m.creative {
            self.m.xp_level = (self.m.xp_level - st.cost).max(0);
        }
        self.inventory_set(0, None);
        let b = self.m.container[1].clone();
        let left = match b {
            Some(b) if st.usage > 0 && b.count > st.usage => Some(b.with_count(b.count - st.usage)),
            _ => None,
        };
        self.inventory_set(1, left);
        let mut st = self.anvil_state();
        st.cost = 0;
        self.set_anvil_state(st);
    }

    /// The input inventory's own `setStack` (notifies the menu once).
    fn inventory_set(&mut self, k: usize, s: Option<Stack>) {
        self.m.container[k] = s;
        self.anvil_update();
    }
}

/// A minimal JSON reader for item names.
mod json {
    pub(super) enum Value {
        Null,
        Bool(bool),
        Num(String),
        Str(String),
        Arr(Vec<Value>),
        Obj(Vec<(String, Value)>),
    }

    pub(super) fn parse(s: &str) -> Option<Value> {
        let mut p = P {
            s: s.as_bytes(),
            i: 0,
        };
        let v = p.value()?;
        p.ws();
        (p.i == p.s.len()).then_some(v)
    }

    struct P<'a> {
        s: &'a [u8],
        i: usize,
    }

    impl P<'_> {
        fn ws(&mut self) {
            while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
                self.i += 1;
            }
        }

        fn value(&mut self) -> Option<Value> {
            self.ws();
            match *self.s.get(self.i)? {
                b'{' => {
                    self.i += 1;
                    let mut o = Vec::new();
                    self.ws();
                    if self.s.get(self.i) == Some(&b'}') {
                        self.i += 1;
                        return Some(Value::Obj(o));
                    }
                    loop {
                        self.ws();
                        let Value::Str(k) = self.value()? else {
                            return None;
                        };
                        self.ws();
                        if self.s.get(self.i) != Some(&b':') {
                            return None;
                        }
                        self.i += 1;
                        let v = self.value()?;
                        o.push((k, v));
                        self.ws();
                        match *self.s.get(self.i)? {
                            b',' => self.i += 1,
                            b'}' => {
                                self.i += 1;
                                return Some(Value::Obj(o));
                            }
                            _ => return None,
                        }
                    }
                }
                b'[' => {
                    self.i += 1;
                    let mut a = Vec::new();
                    self.ws();
                    if self.s.get(self.i) == Some(&b']') {
                        self.i += 1;
                        return Some(Value::Arr(a));
                    }
                    loop {
                        a.push(self.value()?);
                        self.ws();
                        match *self.s.get(self.i)? {
                            b',' => self.i += 1,
                            b']' => {
                                self.i += 1;
                                return Some(Value::Arr(a));
                            }
                            _ => return None,
                        }
                    }
                }
                b'"' => {
                    self.i += 1;
                    let mut out = Vec::new();
                    loop {
                        let c = *self.s.get(self.i)?;
                        self.i += 1;
                        match c {
                            b'"' => return String::from_utf8(out).ok().map(Value::Str),
                            b'\\' => {
                                let e = *self.s.get(self.i)?;
                                self.i += 1;
                                let ch = match e {
                                    b'"' => '"',
                                    b'\\' => '\\',
                                    b'/' => '/',
                                    b'b' => '\u{8}',
                                    b'f' => '\u{c}',
                                    b'n' => '\n',
                                    b'r' => '\r',
                                    b't' => '\t',
                                    b'u' => {
                                        let h =
                                            std::str::from_utf8(self.s.get(self.i..self.i + 4)?)
                                                .ok()?;
                                        self.i += 4;
                                        // surrogate pairs are left unmodelled
                                        char::from_u32(u32::from_str_radix(h, 16).ok()?)?
                                    }
                                    _ => return None,
                                };
                                let mut buf = [0u8; 4];
                                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                            }
                            c => out.push(c),
                        }
                    }
                }
                b't' if self.s[self.i..].starts_with(b"true") => {
                    self.i += 4;
                    Some(Value::Bool(true))
                }
                b'f' if self.s[self.i..].starts_with(b"false") => {
                    self.i += 5;
                    Some(Value::Bool(false))
                }
                b'n' if self.s[self.i..].starts_with(b"null") => {
                    self.i += 4;
                    Some(Value::Null)
                }
                _ => {
                    let st = self.i;
                    while self.i < self.s.len()
                        && matches!(
                            self.s[self.i],
                            b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                        )
                    {
                        self.i += 1;
                    }
                    // a number's text as Gson prints it is not modelled beyond integers
                    let t = std::str::from_utf8(&self.s[st..self.i]).ok()?;
                    (st < self.i && t.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
                        .then(|| Value::Num(t.to_owned()))
                }
            }
        }
    }
}

#[cfg(all(test, feature = "v1_16_5"))]
mod tests {
    use super::*;

    /// The tables as the jar has them (the anvil facts line of the golden).
    #[test]
    fn anvil_facts_1_16_5() {
        use std::io::BufRead;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden/1.16.5/clicks_menus_seed1.jsonl.gz");
        let f = std::fs::File::open(&path).unwrap();
        let r = std::io::BufReader::new(flate2::read::GzDecoder::new(f));
        for line in r.lines() {
            let line = line.unwrap();
            if !line.starts_with("{\"facts\":\"anvil\"") {
                continue;
            }
            let c: serde_json::Value = serde_json::from_str(&line).unwrap();
            let es = c["enchantments"].as_array().unwrap();
            assert_eq!(es.len(), ANVIL_ENCHANTMENTS_1_16.len());
            for (k, e) in es.iter().enumerate() {
                let w = match e[2].as_str().unwrap() {
                    "COMMON" => 1,
                    "UNCOMMON" => 2,
                    "RARE" => 4,
                    _ => 8,
                };
                assert_eq!(
                    ANVIL_ENCHANTMENTS_1_16[k].0,
                    e[1].as_i64().unwrap() as i32,
                    "{k}"
                );
                assert_eq!(ANVIL_ENCHANTMENTS_1_16[k].1, w, "{k}");
                for o in 0..es.len() {
                    let bad = e[3]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|x| x.as_u64() == Some(o as u64));
                    assert_eq!(can_combine(k, o), !bad, "{k} {o}");
                }
                for id in 0..1000 {
                    let want = e[4]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|x| x.as_i64() == Some(id));
                    assert_eq!(acceptable(k, id as i32), want, "{k} item {id}");
                }
            }
            for rep in c["repair"].as_array().unwrap() {
                let id = rep[0].as_i64().unwrap() as i32;
                for m in 0..1000 {
                    let want = rep[1]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|x| x.as_i64() == Some(m));
                    assert_eq!(can_repair(id, m as i32), want, "{id} with {m}");
                }
            }
            return;
        }
        panic!("no anvil facts in {}", path.display());
    }

    #[test]
    fn names() {
        assert_eq!(plain(r#"{"text":"a"}"#).unwrap(), "a");
        assert_eq!(
            plain(r#"{"extra":[{"text":"b","color":"red"}],"text":"a"}"#).unwrap(),
            "ab"
        );
        assert!(plain(r#"{"translate":"x"}"#).is_err());
        assert_eq!(literal_json("a\"b").unwrap(), r#"{"text":"a\"b"}"#);
    }
}
