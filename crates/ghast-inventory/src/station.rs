//! The 1.16.5 workstation menus whose result slot the client fills itself: smithing table and
//! stonecutter (from the server's recipes), grindstone, loom and cartography table.
//!
//! Each menu keeps the state its screen handler keeps on the client (the remembered recipe, the
//! stonecutter's recipe list and selection, ...) in [`Station`]; every change of an input slot
//! runs the handler's `onContentChanged` / `updateResult` like vanilla's input inventory does.

use crate::nbt16;
use crate::rules::Version;
use crate::sim::{Loc, R, Sim, Unpredictable};
use crate::stack::{ItemData, Stack};
use crate::window::WindowKind;
use ghast_protocol::nbt::{Compound, Tag};

/// A workstation menu's client-side state.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Station {
    /// Smithing table: the recipe of the last lookup that found one (`field_25386`; kept when
    /// a later lookup finds none).
    Smithing { recipe: Option<usize> },
    /// Stonecutter: the input item the list belongs to (`inputStack`), the matching recipes
    /// sorted like `getAllMatches` (indices into the book) and the selected one.
    Stonecutter {
        input: i32,
        recipes: Vec<usize>,
        selected: i32,
    },
    /// Grindstone (no state of its own).
    Grindstone,
    /// Loom: the selected pattern (`selectedPattern`, 0 = none).
    Loom { selected: i32 },
    /// 1.16.5 anvil: the name field, the tracked first input, the cost.
    Anvil(crate::anvil16::Anvil),
    /// Cartography table: its client never computes a result (the screen's context is empty);
    /// it only clears the server's result when the inputs no longer pay for it.
    Cartography,
}

impl Station {
    /// The state a freshly opened screen has (`None`: not a workstation menu ghast models).
    pub(crate) fn of(v: Version, kind: WindowKind) -> Option<Station> {
        if v != Version::V1_16_5 {
            return None;
        }
        Some(match kind {
            WindowKind::Smithing => Station::Smithing { recipe: None },
            // `Property.create()` starts at 0; only an input change sets -1
            WindowKind::Stonecutter => Station::Stonecutter {
                input: 0,
                recipes: Vec::new(),
                selected: 0,
            },
            WindowKind::Grindstone => Station::Grindstone,
            WindowKind::Loom => Station::Loom { selected: 0 },
            WindowKind::Cartography => Station::Cartography,
            WindowKind::Anvil => Station::Anvil(crate::anvil16::Anvil::default()),
            _ => return None,
        })
    }

    /// Whether container slot `k` is one of the menu's inputs (its inventory notifies the menu).
    pub(crate) fn is_input(&self, k: usize) -> bool {
        match self {
            Station::Smithing { .. } | Station::Grindstone | Station::Anvil(_) => k < 2,
            Station::Stonecutter { .. } => k == 0,
            Station::Loom { .. } => k < 3,
            // its result inventory notifies the menu too
            Station::Cartography => k < 3,
        }
    }

    /// The container slot of the result.
    pub(crate) fn output(&self) -> usize {
        match self {
            Station::Smithing { .. }
            | Station::Grindstone
            | Station::Cartography
            | Station::Anvil(_) => 2,
            Station::Stonecutter { .. } => 1,
            Station::Loom { .. } => 3,
        }
    }

    /// Whether double-click gathers from the result slot (`canInsertIntoSlot`: the smithing
    /// table and the stonecutter refuse it, the grindstone does not).
    pub(crate) fn gathers_result(&self) -> bool {
        matches!(
            self,
            Station::Grindstone | Station::Loom { .. } | Station::Anvil(_)
        )
    }
}

/// 1.16.5 block items: their translation key is `block.minecraft.<name>` (all others:
/// `item.minecraft.<name>`), as the item registry ids of the jar (checked by the unit test).
const BLOCK_ITEMS_1_16: &[(i32, i32)] = &[
    (0, 569),
    (652, 659),
    (676, 676),
    (715, 731),
    (755, 756),
    (829, 829),
    (836, 841),
    (870, 885),
    (928, 928),
    (935, 947),
    (949, 951),
    (953, 954),
    (956, 975),
];

/// `Item.getTranslationKey()` of a 1.16.5 item (what `RecipeManager.getAllMatches` sorts by).
pub(crate) fn translation_key(v: Version, id: i32) -> (bool, &'static str) {
    let block = BLOCK_ITEMS_1_16.iter().any(|&(a, b)| (a..=b).contains(&id));
    // air has no entry in the item name table
    (!block, if id == 0 { "air" } else { v.name(id) })
}

/// The sort key of a recipe result: `("block" | "item") + ".minecraft." + name` compared as a
/// string, i.e. "block" before "item", then the name.
fn result_key(v: Version, s: Option<&Stack>) -> (bool, &'static str) {
    // an empty result is air: `block.minecraft.air`
    translation_key(v, s.map_or(0, |s| s.id))
}

/// `Ingredient.test`: the item alone (an empty ingredient matches only an empty stack).
fn test(options: &[i32], s: Option<&Stack>) -> bool {
    match s {
        None => options.is_empty(),
        Some(s) => options.contains(&s.id),
    }
}

impl Sim<'_> {
    /// A change reached container slot `l` (set, taken, or changed in place and marked dirty).
    pub(crate) fn station_touched(&mut self, l: Loc) {
        let Loc::Container(k) = l else { return };
        if !self.m.station.as_ref().is_some_and(|s| s.is_input(k)) {
            return;
        }
        match self.m.station.as_deref() {
            Some(Station::Smithing { .. }) => self.smithing_update(),
            Some(Station::Stonecutter { .. }) => self.stonecutter_changed(),
            Some(Station::Grindstone) => self.grindstone_update(),
            Some(Station::Loom { .. }) => self.loom_changed(),
            // ForgingScreenHandler.onContentChanged: super (sendContentUpdates) first
            Some(Station::Anvil(_)) => {
                self.anvil_sync();
                self.anvil_update();
            }
            Some(Station::Cartography) => {
                // onContentChanged: no result computed on the client, only cleared
                let both = self.m.container[0].is_some() && self.m.container[1].is_some();
                if !both {
                    self.m.container[2] = None;
                }
            }
            None => {}
        }
    }

    /// `SmithingScreenHandler.updateResult`.
    fn smithing_update(&mut self) {
        let book = &self.book.smithing;
        let (a, b) = (self.m.container[0].as_ref(), self.m.container[1].as_ref());
        let mut matches: Vec<usize> = (0..book.len())
            .filter(|&k| test(&book[k].0, a) && test(&book[k].1, b))
            .collect();
        let v = self.v;
        matches.sort_by_key(|&k| result_key(v, book[k].2.as_ref()));
        let Some(&first) = matches.first() else {
            self.m.container[2] = None;
            return;
        };
        let mut out = book[first].2.clone();
        // the result with the base's NBT
        if let (Some(o), Some(ItemData::Nbt(n))) = (&mut out, a.map(|a| &a.data)) {
            o.data = ItemData::Nbt(n.clone());
        }
        self.m.container[2] = out;
        if let Some(Station::Smithing { recipe }) = self.m.station.as_deref_mut() {
            *recipe = Some(first);
        }
    }

    /// `StonecutterScreenHandler.onContentChanged`: a new input item gets a new recipe list,
    /// no selection and an empty output.
    fn stonecutter_changed(&mut self) {
        let item = self.m.container[0].as_ref().map_or(0, |s| s.id);
        let v = self.v;
        let book = &self.book.stonecutting;
        let Some(Station::Stonecutter {
            input,
            recipes,
            selected,
        }) = self.m.station.as_deref_mut()
        else {
            return;
        };
        if item == *input {
            return;
        }
        *input = item;
        recipes.clear();
        *selected = -1;
        self.m.container[1] = None;
        if item != 0 {
            let s = self.m.container[0].as_ref();
            *recipes = (0..book.len()).filter(|&k| test(&book[k].0, s)).collect();
            recipes.sort_by_key(|&k| result_key(v, book[k].1.as_ref()));
        }
    }

    /// `StonecutterScreenHandler.populateResult`.
    pub(crate) fn stonecutter_populate(&mut self) {
        let book = &self.book.stonecutting;
        let Some(Station::Stonecutter {
            recipes, selected, ..
        }) = self.m.station.as_deref()
        else {
            return;
        };
        let out = usize::try_from(*selected)
            .ok()
            .and_then(|k| recipes.get(k))
            .and_then(|&r| book[r].1.clone());
        self.m.container[1] = out;
    }

    /// Whether a stonecutting recipe takes the stack (the shift-click into the input).
    pub(crate) fn stonecuttable(&self, s: &Stack) -> bool {
        self.book.stonecutting.iter().any(|r| test(&r.0, Some(s)))
    }

    /// Whether a smithing recipe takes the stack as its addition (shift-click to slot 1).
    pub(crate) fn smithing_addition(&self, s: &Stack) -> bool {
        self.book.smithing.iter().any(|r| test(&r.1, Some(s)))
    }

    /// The result slot's `canTakeItems` (`Err`: taking it runs code ghast does not model: a
    /// cartography result's `onCraft` rescales or locks the map with the world's map data).
    pub(crate) fn station_can_take(&self) -> R<bool> {
        if let Some(Station::Cartography) = self.m.station.as_deref() {
            let t = self.m.container[2]
                .as_ref()
                .map(nbt16::tree)
                .transpose()?
                .flatten();
            if t.is_some_and(|t| {
                crate::stack::last(&t, "map_scale_direction").is_some()
                    || crate::stack::last(&t, "map_to_lock").is_some()
            }) {
                return Err(Unpredictable);
            }
        }
        Ok(match self.m.station.as_deref() {
            Some(Station::Anvil(_)) => self.anvil_can_take(),
            Some(Station::Smithing { recipe: Some(k) }) => {
                let r = &self.book.smithing[*k];
                test(&r.0, self.m.container[0].as_ref()) && test(&r.1, self.m.container[1].as_ref())
            }
            Some(Station::Smithing { recipe: None }) => false,
            _ => true,
        })
    }

    /// The result slot's `onTakeItem`: the inputs pay (each set again, which looks the result
    /// up again).
    pub(crate) fn station_take(&mut self) -> R<()> {
        match self.m.station.as_deref() {
            Some(Station::Smithing { .. }) => {
                for k in 0..2 {
                    let s = self.m.container[k]
                        .clone()
                        .map(|s| s.with_count(s.count - 1));
                    let s = s.and_then(|s| self.norm(s));
                    self.set(k, s);
                }
                Ok(())
            }
            Some(Station::Stonecutter { .. }) => {
                if self.take(0, 1).is_some() {
                    self.stonecutter_populate();
                }
                Ok(())
            }
            Some(Station::Grindstone) => {
                self.set(0, None);
                self.set(1, None);
                Ok(())
            }
            Some(Station::Anvil(_)) => {
                self.anvil_take();
                Ok(())
            }
            Some(Station::Cartography) => {
                let _ = self.take(0, 1);
                let _ = self.take(1, 1);
                // `super.onTakeItem`: the result slot marked dirty (its inventory notifies)
                self.station_touched(Loc::Container(2));
                Ok(())
            }
            Some(Station::Loom { .. }) => {
                let _ = self.take(0, 1);
                let _ = self.take(1, 1);
                if self.get(0).is_none() || self.get(1).is_none() {
                    self.set_loom_selected(0);
                }
                Ok(())
            }
            None => Err(Unpredictable),
        }
    }

    /// The menu's `transferSlot` routes for a player stack (`Ok(None)`: not a workstation).
    pub(crate) fn station_route(&mut self, i: usize, st: &mut Stack) -> R<Option<bool>> {
        let kind = match self.m.station.as_deref() {
            Some(s) => std::mem::discriminant(s),
            None => return Ok(None),
        };
        let smithing = std::mem::discriminant(&Station::Smithing { recipe: None });
        if kind == std::mem::discriminant(&Station::Cartography) {
            let name = self.v.name(st.id);
            return Ok(Some(match i {
                2 => self.insert(st, 3, 39, true)?,
                0 | 1 => self.insert(st, 3, 39, false)?,
                _ if name == "filled_map" => self.insert(st, 0, 1, false)?,
                _ if matches!(name, "paper" | "map" | "glass_pane") => {
                    self.insert(st, 1, 2, false)?
                }
                3..30 => self.insert(st, 30, 39, false)?,
                30..39 => self.insert(st, 3, 30, false)?,
                _ => false,
            }));
        }
        if kind == std::mem::discriminant(&Station::Loom { selected: 0 }) {
            let v = self.v;
            return Ok(Some(match i {
                3 => self.insert(st, 4, 40, true)?,
                0..3 => self.insert(st, 4, 40, false)?,
                _ if loom_banner(v, st).is_some() => self.insert(st, 0, 1, false)?,
                _ if loom_dye(v, st).is_some() => self.insert(st, 1, 2, false)?,
                _ if loom_pattern(v, st).is_some() => self.insert(st, 2, 3, false)?,
                4..31 => self.insert(st, 31, 40, false)?,
                31..40 => self.insert(st, 4, 31, false)?,
                _ => false,
            }));
        }
        if kind == std::mem::discriminant(&Station::Grindstone) {
            let both = self.get(0).is_some() && self.get(1).is_some();
            return Ok(Some(match i {
                2 => self.insert(st, 3, 39, true)?,
                0 | 1 => self.insert(st, 3, 39, false)?,
                3..30 if both => self.insert(st, 30, 39, false)?,
                30..39 if both => self.insert(st, 3, 30, false)?,
                _ if both => false,
                _ => self.insert(st, 0, 2, false)?,
            }));
        }
        Ok(Some(if kind == smithing {
            match i {
                2 => self.insert(st, 3, 39, true)?,
                0 | 1 => self.insert(st, 3, 39, false)?,
                _ => {
                    let from = usize::from(self.smithing_addition(st));
                    self.insert(st, from, 2, false)?
                }
            }
        } else {
            match i {
                1 => self.insert(st, 2, 38, true)?,
                0 => self.insert(st, 2, 38, false)?,
                _ if self.stonecuttable(st) => self.insert(st, 0, 1, false)?,
                2..29 => self.insert(st, 29, 38, false)?,
                29..38 => self.insert(st, 2, 29, false)?,
                _ => false,
            }
        }))
    }
}

/// A workstation input's `canInsert` (grindstone: damageable, an enchanted book, or
/// enchanted; loom: a banner, a dye, a banner pattern).
pub(crate) fn input_accepts(v: Version, w: WindowKind, k: usize, s: &Stack) -> R<bool> {
    Ok(match (w, k) {
        (WindowKind::Grindstone, _) => {
            let t = nbt16::tree(s)?;
            v.is_damageable(s)
                || v.name(s.id) == "enchanted_book"
                || nbt16::has_enchantments(t.as_ref())
        }
        (WindowKind::Loom, 0) => loom_banner(v, s).is_some(),
        (WindowKind::Loom, 1) => loom_dye(v, s).is_some(),
        (WindowKind::Loom, 2) => loom_pattern(v, s).is_some(),
        (WindowKind::Cartography, 0) => v.name(s.id) == "filled_map",
        (WindowKind::Cartography, 1) => matches!(v.name(s.id), "paper" | "map" | "glass_pane"),
        _ => return Err(Unpredictable),
    })
}

/// 1.16.5 banner patterns by ordinal (`BannerPattern.getId`); the last six come from pattern
/// items (checked by the unit test against the jar).
pub(crate) const PATTERNS_1_16: &[&str] = &[
    "b", "bl", "br", "tl", "tr", "bs", "ts", "ls", "rs", "cs", "ms", "drs", "dls", "ss", "cr",
    "sc", "bt", "tt", "bts", "tts", "ld", "rd", "lud", "rud", "mc", "mr", "vh", "hh", "vhr", "hhb",
    "bo", "cbo", "gra", "gru", "bri", "glb", "cre", "sku", "flo", "moj", "pig",
];
/// Patterns that come from a pattern item (`BannerPattern.field_24417`).
const ITEM_PATTERNS_1_16: i32 = 6;
/// `BannerPattern.LOOM_APPLICABLE_COUNT`: the buttons 1..=34.
const LOOM_BUTTONS_1_16: i32 = PATTERNS_1_16.len() as i32 - ITEM_PATTERNS_1_16 - 1;

/// A `BannerItem`: its colour (white_banner .. black_banner, ids 870..=885).
pub(crate) fn loom_banner(v: Version, s: &Stack) -> Option<i32> {
    (v == Version::V1_16_5 && (870..=885).contains(&s.id)).then(|| s.id - 870)
}

/// A `DyeItem`: its colour id (white_dye .. black_dye, ids 696..=711).
pub(crate) fn loom_dye(v: Version, s: &Stack) -> Option<i32> {
    (v == Version::V1_16_5 && (696..=711).contains(&s.id)).then(|| s.id - 696)
}

/// A `BannerPatternItem`: its pattern's ordinal (ids 929..=934).
pub(crate) fn loom_pattern(v: Version, s: &Stack) -> Option<i32> {
    if v != Version::V1_16_5 {
        return None;
    }
    match s.id {
        929 => Some(38),
        930 => Some(36),
        931 => Some(37),
        932 => Some(39),
        933 => Some(35),
        934 => Some(40),
        _ => None,
    }
}

impl Sim<'_> {
    fn loom_selected(&self) -> i32 {
        match self.m.station.as_deref() {
            Some(Station::Loom { selected }) => *selected,
            _ => 0,
        }
    }

    pub(crate) fn set_loom_selected(&mut self, value: i32) {
        if let Some(Station::Loom { selected }) = self.m.station.as_deref_mut() {
            *selected = value;
        }
    }

    /// `LoomScreenHandler.onContentChanged`. A banner next to a pattern item gets an empty
    /// `BlockEntityTag` if it had none (`getOrCreateSubTag`, on the slot's own stack).
    fn loom_changed(&mut self) {
        let sel = self.loom_selected();
        let banner = self.m.container[0].is_some();
        let dye = self.m.container[1].is_some();
        let pattern = self.m.container[2].clone();
        let applies =
            sel > 0 && (sel < PATTERNS_1_16.len() as i32 - ITEM_PATTERNS_1_16 || pattern.is_some());
        if self.m.container[3].is_none() || (banner && dye && applies) {
            if let Some(p) = pattern.as_ref().and_then(|p| loom_pattern(self.v, p)) {
                let full = match self.m.container[0].clone() {
                    Some(b) => {
                        let Ok(t) = be_tag(&b) else {
                            self.m.container_result_known = false;
                            return;
                        };
                        let full = patterns_of(&t).is_some_and(|(present, n)| present && n >= 6);
                        match nbt16::with_tree(b.id, b.count, Some(t)) {
                            Ok(s) => self.m.container[0] = Some(s),
                            Err(_) => {
                                self.m.container_result_known = false;
                                return;
                            }
                        }
                        full
                    }
                    None => false,
                };
                self.set_loom_selected(if full { 0 } else { p });
            }
        } else {
            self.m.container[3] = None;
            self.set_loom_selected(0);
        }
        self.loom_output();
    }

    /// `LoomScreenHandler.updateOutputSlot` (`Err` cases: vanilla throws or drops the pattern;
    /// the result is then left unknown).
    pub(crate) fn loom_output(&mut self) {
        if self.loom_result().is_err() {
            self.m.container_result_known = false;
        }
    }

    fn loom_result(&mut self) -> R<()> {
        let sel = self.loom_selected();
        if sel <= 0 {
            return Ok(());
        }
        // BannerPattern.values()[sel] throws past the end
        let pattern = usize::try_from(sel)
            .ok()
            .and_then(|k| PATTERNS_1_16.get(k))
            .ok_or(Unpredictable)?;
        let out = match (self.m.container[0].clone(), self.m.container[1].clone()) {
            (Some(b), Some(d)) => {
                // the dye slot's item is cast to DyeItem
                let color = loom_dye(self.v, &d).ok_or(Unpredictable)?;
                let mut t = be_tag(&b)?;
                let be = compound_mut(&mut t, "BlockEntityTag").ok_or(Unpredictable)?;
                let mut list = match crate::stack::last(be, "Patterns") {
                    Some(Tag::List(l))
                        if l.items.is_empty()
                            || l.elem_type == ghast_protocol::nbt::tag::COMPOUND =>
                    {
                        nbt16::compound_list(be, "Patterns")
                    }
                    // getList hands back a detached list: the pattern is lost
                    Some(Tag::List(_)) => return Err(Unpredictable),
                    _ => Vec::new(),
                };
                list.push(Compound(vec![
                    ("Pattern".into(), Tag::String((*pattern).to_owned())),
                    ("Color".into(), Tag::Int(color)),
                ]));
                nbt16::put(be, "Patterns", nbt16::list_tag(list));
                Some(nbt16::with_tree(b.id, 1, Some(t))?)
            }
            _ => None,
        };
        if !crate::stack::stacks_equal(out.as_ref(), self.m.container[3].as_ref()) {
            let mut out = out;
            crate::sim::clamp(&mut out);
            self.m.container[3] = out;
        }
        Ok(())
    }

    /// The loom's `onButtonClick`: a pattern button 1..=34 selects it.
    pub(crate) fn loom_button(&mut self, id: i32) -> bool {
        if id > 0 && id <= LOOM_BUTTONS_1_16 {
            self.set_loom_selected(id);
            self.loom_output();
            true
        } else {
            false
        }
    }
}

/// The banner's tag with its `BlockEntityTag` compound present (`getOrCreateSubTag`).
fn be_tag(b: &Stack) -> R<Compound> {
    let mut t = nbt16::tree(b)?.unwrap_or_default();
    if !matches!(
        crate::stack::last(&t, "BlockEntityTag"),
        Some(Tag::Compound(_))
    ) {
        nbt16::put(&mut t, "BlockEntityTag", Tag::Compound(Compound::default()));
    }
    Ok(t)
}

/// `contains("Patterns", 9)` and `getList("Patterns", 10).size()` of the banner's block entity.
fn patterns_of(t: &Compound) -> Option<(bool, usize)> {
    let Some(Tag::Compound(be)) = crate::stack::last(t, "BlockEntityTag") else {
        return None;
    };
    let present = matches!(crate::stack::last(be, "Patterns"), Some(Tag::List(_)));
    Some((present, nbt16::compound_list(be, "Patterns").len()))
}

/// The compound entry `key` of `t` (the last of its name), mutable.
fn compound_mut<'a>(t: &'a mut Compound, key: &str) -> Option<&'a mut Compound> {
    t.0.iter_mut()
        .rev()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| match v {
            Tag::Compound(c) => Some(c),
            _ => None,
        })
}

impl Sim<'_> {
    /// `GrindstoneScreenHandler.updateResult` (`Err` for a case ghast does not model: then the
    /// result is left unknown, which the next click reading it refuses).
    fn grindstone_update(&mut self) {
        let r = grindstone_result(
            self.v,
            self.m.container[0].as_ref(),
            self.m.container[1].as_ref(),
        );
        match r {
            Ok(out) => {
                self.m.container[2] = out;
                self.m.container_result_known = true;
            }
            Err(Unpredictable) => self.m.container_result_known = false,
        }
    }
}

/// The grindstone's result for the two inputs.
fn grindstone_result(v: Version, a: Option<&Stack>, b: Option<&Stack>) -> R<Option<Stack>> {
    use nbt16 as n;
    if a.is_none() && b.is_none() {
        return Ok(None);
    }
    let ta = a.map(n::tree).transpose()?.flatten();
    let tb = b.map(n::tree).transpose()?.flatten();
    let plain = |s: Option<&Stack>, t: Option<&Compound>| {
        s.is_some_and(|s| v.name(s.id) != "enchanted_book" && !n::has_enchantments(t))
    };
    let both = a.is_some() && b.is_some();
    let any_plain = plain(a, ta.as_ref()) || plain(b, tb.as_ref());
    if a.is_some_and(|s| s.count > 1) || b.is_some_and(|s| s.count > 1) || (!both && any_plain) {
        return Ok(None);
    }
    let (item, mut tag, damage, amount) = match (a, b) {
        (Some(a), Some(b)) => {
            if a.id != b.id {
                return Ok(None);
            }
            let max = v.max_durability(a.id).unwrap_or(0);
            let j = max - n::damage(ta.as_ref());
            let k = max - n::damage(tb.as_ref());
            let l = j + k + max * 5 / 100;
            let m = (max - l).max(0);
            // transferEnchantments(a, b): b's enchantments added to a copy of a, curses only
            // where a has none of them
            let mut t = ta.clone();
            for (e, lvl) in n::enchantments(v, b, tb.as_ref()) {
                if !n::cursed(e) || n::level(t.as_ref(), e) == 0 {
                    n::add_enchantment(&mut t, v.max_durability(a.id).is_some(), e, lvl)?;
                }
            }
            let damageable = v.max_durability(a.id).is_some()
                && !t.as_ref().is_some_and(|c| {
                    crate::stack::last(c, "Unbreakable")
                        .and_then(crate::stack::number)
                        .is_some_and(|x| x as i8 != 0)
                });
            let mut amount = 1;
            if !damageable {
                if !(a.count == b.count && a.nbt_eq(b)) {
                    return Ok(None);
                }
                amount = 2;
            }
            (a, t, m, amount)
        }
        (Some(a), None) => (a, ta.clone(), n::damage(ta.as_ref()), 1),
        (None, Some(b)) => (b, tb.clone(), n::damage(tb.as_ref()), 1),
        (None, None) => return Ok(None),
    };
    let durable = v.max_durability(item.id).is_some();
    // grind(item, damage, amount)
    let named = n::has_custom_name(tag.as_ref());
    let source = n::with_tree(item.id, item.count, tag.clone())?;
    let curses: Vec<(usize, i32)> = n::enchantments(v, &source, tag.as_ref())
        .into_iter()
        .filter(|e| n::cursed(e.0))
        .collect();
    if curses.len() > 1 {
        // `Collectors.toMap` builds a HashMap keyed by the enchantment objects: its order
        // follows their identity hash codes
        return Err(Unpredictable);
    }
    n::remove_sub(&mut tag, "Enchantments");
    n::remove_sub(&mut tag, "StoredEnchantments");
    if damage > 0 {
        n::put(n::create(&mut tag, durable), "Damage", Tag::Int(damage));
    } else {
        n::remove_sub(&mut tag, "Damage");
    }
    let book = v.name(item.id) == "enchanted_book";
    // EnchantmentHelper.set(curses, stack)
    let mut list = Vec::new();
    for &(e, lvl) in &curses {
        list.push(n::entry(e, lvl as i16));
        if book {
            n::add_stored(&mut tag, durable, e, lvl);
        }
    }
    if list.is_empty() {
        n::remove_sub(&mut tag, "Enchantments");
    } else if !book {
        n::put(
            n::create(&mut tag, durable),
            "Enchantments",
            n::list_tag(list),
        );
    }
    n::put(n::create(&mut tag, durable), "RepairCost", Tag::Int(0));
    let mut id = item.id;
    let mut amount = amount;
    if book && curses.is_empty() {
        if named {
            // setCustomName(item.getName()): the name re-serialised from its parsed text
            return Err(Unpredictable);
        }
        // a fresh book
        id = (1..2000)
            .find(|&i| v.name(i) == "book")
            .ok_or(Unpredictable)?;
        tag = None;
        amount = 1;
    }
    for _ in 0..curses.len() {
        let c = n::repair_cost(tag.as_ref());
        n::put(
            n::create(&mut tag, durable),
            "RepairCost",
            Tag::Int(c * 2 + 1),
        );
    }
    n::with_tree(id, amount, tag).map(Some)
}

#[cfg(all(test, feature = "v1_16_5"))]
mod tests {
    use super::*;

    /// The loom's data as the jar has it: pattern ids and which come from items, the banner,
    /// dye and pattern items with their colour / pattern.
    #[test]
    fn loom_facts_1_16_5() {
        use std::io::BufRead;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden/1.16.5/clicks_menus_seed1.jsonl.gz");
        let f = std::fs::File::open(&path).unwrap();
        let r = std::io::BufReader::new(flate2::read::GzDecoder::new(f));
        let v = Version::V1_16_5;
        for line in r.lines() {
            let line = line.unwrap();
            if !line.starts_with("{\"facts\":\"loom\"") {
                continue;
            }
            let c: serde_json::Value = serde_json::from_str(&line).unwrap();
            let ps = c["patterns"].as_array().unwrap();
            assert_eq!(ps.len(), PATTERNS_1_16.len());
            for (k, p) in ps.iter().enumerate() {
                assert_eq!(p[0].as_str().unwrap(), PATTERNS_1_16[k]);
                let item = k as i32 >= PATTERNS_1_16.len() as i32 - ITEM_PATTERNS_1_16;
                assert_eq!(p[1].as_bool().unwrap(), item, "pattern {k}");
            }
            let mut want = std::collections::HashMap::new();
            for e in c["items"].as_array().unwrap() {
                want.insert(
                    e[0].as_i64().unwrap() as i32,
                    (
                        e[1].as_str().unwrap().to_owned(),
                        e[2].as_i64().unwrap() as i32,
                    ),
                );
            }
            for id in 1..1000 {
                let s = Stack::new(id, 1, 0);
                let got = loom_banner(v, &s)
                    .map(|c| ("banner", c))
                    .or(loom_dye(v, &s).map(|c| ("dye", c)))
                    .or(loom_pattern(v, &s).map(|c| ("pattern", c)));
                let w = want.get(&id).map(|(a, b)| (a.as_str(), *b));
                assert_eq!(got, w, "item {id}");
            }
            return;
        }
        panic!("no loom facts in {}", path.display());
    }

    /// Every item's translation key as the jar has it (the stonecutter facts line).
    #[test]
    fn translation_keys_1_16_5() {
        use std::io::BufRead;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden/1.16.5/clicks_menus_seed1.jsonl.gz");
        let f = std::fs::File::open(&path).unwrap();
        let r = std::io::BufReader::new(flate2::read::GzDecoder::new(f));
        let mut n = 0;
        for line in r.lines() {
            let line = line.unwrap();
            if !line.starts_with("{\"facts\":\"stonecutter\"") {
                continue;
            }
            let c: serde_json::Value = serde_json::from_str(&line).unwrap();
            for e in c["keys"].as_array().unwrap() {
                let id = e[0].as_i64().unwrap() as i32;
                let (item, name) = translation_key(Version::V1_16_5, id);
                let got = format!("{}.minecraft.{name}", if item { "item" } else { "block" });
                assert_eq!(got, e[1].as_str().unwrap(), "item {id}");
                n += 1;
            }
        }
        assert!(n > 900, "no keys in {}", path.display());
    }
}
