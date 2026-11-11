//! The client-side click simulation: vanilla's `ScreenHandler.onSlotClick` for
//! 1.8.9, 1.12.2 and 1.16.5, the containers' quick-move (`transferSlot`) rules
//! and the player inventory's insert logic, over a [`Model`] of the slots.
//!
//! Each function follows the vanilla control flow of its version (they differ in many small
//! ways: zero-count stacks, damage comparisons, slot limits in `insertItem`, the quick-move
//! retry, swap rules). Anything whose result needs data we do not model (recipes, fuel,
//! special slots) returns [`Unpredictable`]; the caller then discards the scratch model and
//! refuses the click, so the state never diverges from what the vanilla client would hold.

use crate::crafting::{self, Craft, Grid, RecipeBook, Rem};
use crate::rules::Version;
use crate::stack::Stack;
use crate::window::{SlotKind, WindowKind};

/// Slot number of a click outside the window.
pub const OUTSIDE: i16 = -999;

/// The click's outcome depends on rules this crate does not model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unpredictable;

type R<T> = Result<T, Unpredictable>;

/// Everything a click can change.
#[derive(Debug, Clone)]
pub(crate) struct Model {
    /// `PlayerInventory` by vanilla index: 0..9 hotbar, 9..36 main, 36..40 armor (feet, legs,
    /// chest, head), 40 offhand (1.12 / 1.16.5).
    pub player: [Option<Stack>; 41],
    /// Window 0's 2×2 crafting grid.
    pub craft: [Option<Stack>; 4],
    /// Window 0's crafting result.
    pub craft_result: Option<Stack>,
    /// Whether `craft_result` is what the vanilla client shows (see `Inventory::result_known`).
    pub craft_result_known: bool,
    /// The cursor ("carried") stack.
    pub cursor: Option<Stack>,
    /// The open window's own slots.
    pub container: Vec<Option<Stack>>,
    /// Whether the open window's crafting result (container slot 0) is known.
    pub container_result_known: bool,
    /// 1.12: the window 0 crafting result came from the server for the grid as it is now (no
    /// grid change since), so a recipe matches the grid. Taking a stale result is unpredictable
    /// (vanilla's remainders then depend on the recipes).
    pub craft_fresh: bool,
    /// 1.12: the same for the open crafting table.
    pub container_fresh: bool,
    /// Selected hotbar slot.
    pub selected: u8,
    /// 1.8: the last `recompute_1_8` of window 0's grid (see [`CraftMemo`]).
    pub craft_memo: Option<std::sync::Arc<CraftMemo>>,
    /// 1.8: the same for the open crafting table.
    pub table_memo: Option<std::sync::Arc<CraftMemo>>,
}

/// 1.8: `RecipeDispatcher`'s result for a grid, and whether ghast knows it.
fn craft_1_8(grid: &[Option<Stack>], width: usize) -> (Option<Stack>, bool) {
    let g = Grid {
        stacks: grid,
        width,
    };
    if grid.iter().all(Option::is_none) {
        // no recipe matches an empty grid (`empty_grid_crafts_nothing`)
        debug_assert_eq!(crafting::result_1_8(&g), Craft::Known(None));
        return (None, true);
    }
    match crafting::result_1_8(&g) {
        Craft::Known(s) => (s, true),
        Craft::Unknown => (None, false),
    }
}

/// 1.8: a grid and the result `recompute_1_8` computed for it. The result is a pure function of
/// the grid, so while the grid still equals the memo the recipe scan is skipped and the memo's
/// result is written again (the player window's `window_items` recomputes on every packet,
/// like vanilla's `onCraftMatrixChanged`).
#[derive(Debug, Clone)]
pub(crate) struct CraftMemo {
    grid: Vec<Option<Stack>>,
    result: Option<Stack>,
    known: bool,
}

impl Default for Model {
    fn default() -> Model {
        Model {
            player: std::array::from_fn(|_| None),
            craft: Default::default(),
            craft_result: None,
            craft_result_known: true,
            cursor: None,
            container: Vec::new(),
            container_result_known: true,
            craft_fresh: true,
            container_fresh: true,
            selected: 0,
            craft_memo: None,
            table_memo: None,
        }
    }
}

/// Where a screen handler slot stores its stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Loc {
    Player(usize),
    Craft(usize),
    CraftResult,
    Container(usize),
}

/// The screen handler a click runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    /// Window 0 (`PlayerScreenHandler`).
    Player,
    /// An open container window.
    Container(WindowKind),
}

/// Slot layout of a screen handler.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Layout {
    pub screen: Screen,
    /// Container slots (0 for window 0).
    pub n: usize,
    /// All slots.
    pub len: usize,
}

impl Layout {
    pub fn new(v: Version, screen: Screen) -> Layout {
        match screen {
            Screen::Player => Layout {
                screen,
                n: 0,
                len: if v == Version::V1_8 { 45 } else { 46 },
            },
            // 26.1 crafter: 9 grid slots, the player's 36, then the result
            Screen::Container(WindowKind::Crafter) => Layout {
                screen,
                n: 9,
                len: 46,
            },
            Screen::Container(k) => {
                let n = k.container_len(v);
                Layout {
                    screen,
                    n,
                    len: n + if k.has_player_inventory() { 36 } else { 0 },
                }
            }
        }
    }

    pub fn loc(&self, i: usize) -> Option<Loc> {
        if i >= self.len {
            return None;
        }
        Some(match self.screen {
            Screen::Player => match i {
                0 => Loc::CraftResult,
                1..=4 => Loc::Craft(i - 1),
                5..=8 => Loc::Player(39 - (i - 5)),
                9..=35 => Loc::Player(i),
                36..=44 => Loc::Player(i - 36),
                _ => Loc::Player(40),
            },
            Screen::Container(_) => {
                if i < self.n {
                    Loc::Container(i)
                } else if i < self.n + 27 {
                    Loc::Player(i - self.n + 9)
                } else {
                    let k = i - self.n - 27;
                    if k < 9 {
                        Loc::Player(k)
                    } else {
                        // only the crafter has a slot after the hotbar
                        Loc::Container(9)
                    }
                }
            }
        })
    }

    pub fn kind(&self, i: usize) -> SlotKind {
        match self.screen {
            Screen::Player => match i {
                0 => SlotKind::CraftResult,
                5..=8 => SlotKind::Armor((i - 5) as u8),
                45 => SlotKind::Offhand,
                _ => SlotKind::Normal,
            },
            Screen::Container(k) => {
                if i < self.n {
                    k.slot_kind(i)
                } else {
                    SlotKind::Normal
                }
            }
        }
    }

    /// The screen slot showing `PlayerInventory` index `k`, if any.
    pub fn slot_of_player(&self, k: usize) -> Option<usize> {
        (0..self.len).find(|&i| self.loc(i) == Some(Loc::Player(k)))
    }

    /// Whether the slot belongs to the player's inventory (`slot.inventory == playerInventory`).
    pub fn own(&self, i: usize) -> bool {
        matches!(self.loc(i), Some(Loc::Player(_)))
    }

    /// Whether the location's inventory is a `SimpleInventory` (container part of every window
    /// but the crafting table), whose `setStack` caps counts at 64.
    pub fn clamps(&self, l: Loc) -> bool {
        matches!(l, Loc::Container(_)) && self.screen != Screen::Container(WindowKind::Crafting)
    }

    /// 26.1: whether the location's container caps stacks in `setItem` (`SimpleContainer`,
    /// `MerchantContainer`); crafting grids, the crafter and result containers do not.
    pub fn clamps_26(&self, l: Loc) -> bool {
        let Loc::Container(k) = l else {
            return false;
        };
        let Screen::Container(kind) = self.screen else {
            return false;
        };
        !match kind {
            WindowKind::Crafting | WindowKind::Crafter => true,
            WindowKind::Anvil | WindowKind::Grindstone | WindowKind::Cartography => k == 2,
            WindowKind::Smithing => k == 3,
            WindowKind::Stonecutter => k == 1,
            // a mount's saddle / body slots are the entity's equipment (stored as is)
            WindowKind::Horse { .. } => k < 2,
            _ => false,
        }
    }

    /// Whether the result slot is a crafting result (its handler refuses it in PICKUP_ALL).
    fn is_result(&self, i: usize) -> bool {
        self.kind(i) == SlotKind::CraftResult
    }
}

/// `SimpleInventory.setStack`'s cap.
pub(crate) fn clamp(s: &mut Option<Stack>) {
    if let Some(st) = s
        && st.count > 64
    {
        st.count = 64;
    }
}

/// The version's `SimpleContainer.setItem` cap (26.1: `limitSize(min(99, max stack size))`).
#[inline]
pub(crate) fn clamp_v(v: Version, s: &mut Option<Stack>) {
    if v != Version::V26_1 {
        return clamp(s);
    }
    clamp_26_1(s);
}

#[inline(never)]
fn clamp_26_1(s: &mut Option<Stack>) {
    if let Some(st) = s
        && st.count > 1
    {
        let max = Version::V26_1.max_count(st).min(99);
        if st.count > max {
            st.count = max;
        }
    }
}

/// 26.1 binding curse check on a stack's `enchantments` (`Err` when it has enchantments but the
/// curse's registry id is unknown).
fn binding_curse_26(s: &Stack, id: Option<i32>) -> R<bool> {
    #[cfg(feature = "v26_1")]
    {
        let Some(list) = crate::v26_1::enchantments(s, false) else {
            return Ok(false);
        };
        if list.is_empty() {
            return Ok(false);
        }
        let id = id.ok_or(Unpredictable)?;
        Ok(list.iter().any(|&(e, level)| e == id && level > 0))
    }
    #[cfg(not(feature = "v26_1"))]
    {
        let _ = (s, id);
        Ok(false)
    }
}

impl Model {
    pub fn at(&self, l: Loc) -> &Option<Stack> {
        match l {
            Loc::Player(k) => &self.player[k],
            Loc::Craft(k) => &self.craft[k],
            Loc::CraftResult => &self.craft_result,
            Loc::Container(k) => &self.container[k],
        }
    }

    pub fn at_mut(&mut self, l: Loc) -> &mut Option<Stack> {
        match l {
            Loc::Player(k) => &mut self.player[k],
            Loc::Craft(k) => &mut self.craft[k],
            Loc::CraftResult => &mut self.craft_result,
            Loc::Container(k) => &mut self.container[k],
        }
    }
}

/// One click being simulated.
pub(crate) struct Sim<'a> {
    pub v: Version,
    pub lay: Layout,
    pub m: &'a mut Model,
    /// 1.16.5 crafting recipes from the server (remainders of a taken result).
    pub book: &'a RecipeBook,
    /// 26.1: registry id of `minecraft:binding_curse` (`None`: enchantment names unknown).
    pub binding_curse: Option<i32>,
}

impl Sim<'_> {
    fn loc(&self, i: usize) -> Loc {
        // Callers only pass indices < len.
        self.lay.loc(i).unwrap_or(Loc::Player(0))
    }

    fn get(&self, i: usize) -> Option<&Stack> {
        self.m.at(self.loc(i)).as_ref()
    }

    /// `Slot.setStack` -> the backing inventory's `setStack`. Container inventories other than
    /// crafting grids are `SimpleInventory`s, which cut a stack above 64 down to 64 (the stack
    /// object itself, so the excess is gone).
    fn set(&mut self, i: usize, mut s: Option<Stack>) {
        let l = self.loc(i);
        if self.lay.clamps(l) {
            clamp_v(self.v, &mut s);
        }
        *self.m.at_mut(l) = s;
        self.grid_touched(l);
    }

    /// The crafting grid of the screen: its slot numbers and width.
    fn grid_slots(&self) -> Option<(Vec<usize>, usize)> {
        match self.lay.screen {
            Screen::Player => Some(((1..=4).collect(), 2)),
            Screen::Container(WindowKind::Crafting) => Some(((1..=9).collect(), 3)),
            Screen::Container(_) => None,
        }
    }

    fn is_grid(&self, l: Loc) -> bool {
        match (self.lay.screen, l) {
            (Screen::Player, Loc::Craft(_)) => true,
            (Screen::Container(WindowKind::Crafting), Loc::Container(k)) => (1..=9).contains(&k),
            _ => false,
        }
    }

    /// 1.8: `CraftingInventory.setStack` / `removeStack` run `onContentChanged`, which makes the
    /// client compute the result again (in-place count changes do not).
    fn grid_touched(&mut self, l: Loc) {
        if self.v == Version::V1_8 && self.is_grid(l) {
            recompute_1_8(self.m, self.lay);
        }
    }

    fn slot_mut(&mut self, i: usize) -> &mut Option<Stack> {
        let l = self.loc(i);
        self.m.at_mut(l)
    }

    /// Whether a stack with this count counts as gone (1.8: exactly 0; 1.12 / 1.16: `isEmpty`).
    fn gone(&self, count: i32) -> bool {
        match self.v {
            Version::V1_8 => count == 0,
            Version::V1_12 | Version::V1_16_5 | Version::V26_1 => count <= 0,
        }
    }

    fn norm(&self, s: Stack) -> Option<Stack> {
        (!self.gone(s.count)).then_some(s)
    }

    fn result_known(&self, i: usize) -> bool {
        match self.loc(i) {
            Loc::CraftResult => self.m.craft_result_known,
            Loc::Container(_) => self.m.container_result_known,
            _ => true,
        }
    }

    /// `Slot.canInsert`.
    fn can_insert(&self, i: usize, s: &Stack) -> R<bool> {
        Ok(match self.lay.kind(i) {
            SlotKind::Normal | SlotKind::Offhand => true,
            SlotKind::Armor(a) => self.v.armor_accepts(a, s),
            SlotKind::CraftResult | SlotKind::Output => false,
            SlotKind::Shulker => !self.v.is_shulker_box(s),
            SlotKind::Special => return Err(Unpredictable),
        })
    }

    /// `Slot.getMaxStackAmount(stack)` / `getMaxItemCount()` (26.1: `Container.getMaxStackSize`
    /// is 99; the stack's own limit is applied separately).
    fn slot_limit(&self, i: usize) -> R<i32> {
        match self.lay.kind(i) {
            SlotKind::Armor(_) => Ok(1),
            SlotKind::Special => Err(Unpredictable),
            _ if self.v == Version::V26_1 => Ok(99),
            _ => Ok(64),
        }
    }

    /// `Slot.canTakeItems` (reads the slot, so an unknown crafting result is unpredictable).
    fn can_take(&self, i: usize) -> R<bool> {
        match self.lay.kind(i) {
            SlotKind::Special => Err(Unpredictable),
            SlotKind::CraftResult if !self.result_known(i) => Err(Unpredictable),
            SlotKind::Armor(_) if self.v == Version::V1_16_5 => {
                // binding curse keeps armor on (survival; ghast bots are never creative here)
                Ok(!self.get(i).is_some_and(Stack::has_binding_curse))
            }
            // 26.1: an enchantment with the `prevent_armor_change` effect (vanilla: the binding
            // curse; enchantments are data-driven, so only its registry id can be checked)
            SlotKind::Armor(_) if self.v == Version::V26_1 => match self.get(i) {
                None => Ok(true),
                Some(s) => binding_curse_26(s, self.binding_curse).map(|b| !b),
            },
            SlotKind::Armor(_) if self.v == Version::V1_12 => {
                Ok(!self.get(i).is_some_and(Stack::has_binding_curse_1_12))
            }
            _ => Ok(true),
        }
    }

    /// `Slot.takeStack(amount)`: the backing inventory's `removeStack`.
    fn take(&mut self, i: usize, amount: i32) -> Option<Stack> {
        let out = self.take_raw(i, amount);
        if out.is_some() {
            let l = self.loc(i);
            self.grid_touched(l);
        }
        out
    }

    fn take_raw(&mut self, i: usize, amount: i32) -> Option<Stack> {
        let whole = self.lay.kind(i) == SlotKind::CraftResult; // CraftingResultInventory
        let v = self.v;
        let slot = self.slot_mut(i);
        let s = slot.as_mut()?;
        match v {
            Version::V1_8 => {
                if whole || s.count <= amount {
                    slot.take()
                } else {
                    s.count -= amount;
                    let out = s.with_count(amount);
                    if s.count == 0 {
                        *slot = None;
                    }
                    Some(out)
                }
            }
            Version::V1_12 | Version::V1_16_5 | Version::V26_1 => {
                if whole {
                    return slot.take();
                }
                if amount <= 0 {
                    return None;
                }
                let n = amount.min(s.count);
                s.count -= n;
                let out = s.with_count(n);
                if s.count <= 0 {
                    *slot = None;
                }
                (n > 0).then_some(out)
            }
        }
    }

    /// `Slot.onTakeItem`: only the crafting result changes inventories on the client: one of
    /// each ingredient is consumed and the matching recipe's remainders are handed back
    /// (`CraftingResultSlot.onTakeItem`).
    fn on_take(&mut self, i: usize) -> R<()> {
        match self.lay.kind(i) {
            SlotKind::Special => Err(Unpredictable),
            SlotKind::CraftResult => {
                let (slots, width) = self.grid_slots().ok_or(Unpredictable)?;
                let stacks: Vec<Option<Stack>> =
                    slots.iter().map(|&g| self.get(g).cloned()).collect();
                let grid = Grid {
                    stacks: &stacks,
                    width,
                };
                let rems = match self.v {
                    Version::V1_8 => crafting::remainders_1_8(&grid),
                    Version::V1_12 => {
                        if !self.result_fresh(i) {
                            return Err(Unpredictable);
                        }
                        crafting::remainders_1_12(&grid)
                    }
                    Version::V1_16_5 => crafting::remainders_1_16(self.book, &grid),
                    // 26.1 recipes are not modelled
                    Version::V26_1 => return Err(Unpredictable),
                }
                .map_err(|()| Unpredictable)?;
                for (k, &g) in slots.iter().enumerate() {
                    let orig = self.get(g).cloned();
                    if orig.is_some() {
                        let _ = self.take(g, 1);
                    }
                    match &rems[k] {
                        Rem::None => {}
                        Rem::Same => {
                            if self.get(g).is_some() {
                                return Err(Unpredictable);
                            }
                            self.set(g, orig);
                        }
                        Rem::Item(r) => {
                            let cur = self.get(g).cloned();
                            match (self.v, cur) {
                                (_, None) => self.set(g, Some(r.clone())),
                                (Version::V1_16_5 | Version::V26_1, Some(c))
                                    if c.id == r.id && c.nbt_eq(r) =>
                                {
                                    self.set(g, Some(r.with_count(r.count + c.count)));
                                }
                                (Version::V1_12, Some(c))
                                    if c.id == r.id && c.damage == r.damage && c.nbt_eq(r) =>
                                {
                                    self.set(g, Some(r.with_count(r.count + c.count)));
                                }
                                (_, Some(_)) => self.insert_stack(r.clone()), // or dropped
                            }
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// 1.12: whether the crafting result at slot `i` belongs to the grid as it is now.
    fn result_fresh(&self, i: usize) -> bool {
        match self.loc(i) {
            Loc::CraftResult => self.m.craft_fresh,
            Loc::Container(_) => self.m.container_fresh,
            _ => true,
        }
    }

    // ---------------------------------------------------------------- onSlotClick

    /// Runs `onSlotClick(slot, button, mode)` and returns the stack vanilla returns (what the
    /// click packet carries). The caller validated `slot` (`OUTSIDE` or `0..len`).
    pub fn click(&mut self, slot: i16, button: u8, mode: u8) -> R<Option<Stack>> {
        if let Ok(i) = usize::try_from(slot) {
            // Whatever reads an unknown crafting result (its content is computed from recipes)
            // cannot be predicted.
            if self.lay.is_result(i) && !self.result_known(i) {
                return Err(Unpredictable);
            }
        }
        if self.v == Version::V26_1 {
            self.check_26_1(slot, button, mode)?;
        }
        match self.v {
            Version::V1_8 => self.click_1_8(slot, button, mode),
            Version::V1_12 | Version::V1_16_5 | Version::V26_1 => {
                self.click_1_16(slot, button, mode)
            }
        }
    }

    /// 26.1 cases the 1.16-shaped rules do not cover: item-specific click rules (bundles:
    /// `overrideStackedOnOther` / `overrideOtherStackedOnMe`) and stacks whose `equippable` is
    /// patched (their equipment slot is unknown here).
    fn check_26_1(&self, slot: i16, button: u8, mode: u8) -> R<()> {
        #[cfg(feature = "v26_1")]
        {
            use crate::v26_1::{equip, overrides_clicks};
            let at = usize::try_from(slot).ok().and_then(|i| self.get(i));
            let cursor = self.m.cursor.as_ref();
            if mode == 0 && button <= 1 && slot >= 0 {
                // the bundle rules: a carried bundle onto a full slot (left) or an empty one
                // (right); a bundle in the slot under a carried stack (left) or an empty
                // cursor (right)
                let left = button == 0;
                if cursor.is_some_and(overrides_clicks) && left == at.is_some() {
                    return Err(Unpredictable);
                }
                if at.is_some_and(overrides_clicks) && left == cursor.is_some() {
                    return Err(Unpredictable);
                }
            }
            let hot = (mode == 2)
                .then(|| self.m.player.get(usize::from(button)))
                .flatten()
                .and_then(Option::as_ref);
            for s in [at, cursor, hot].into_iter().flatten() {
                if equip(s).is_err() {
                    return Err(Unpredictable);
                }
            }
        }
        let _ = (slot, button, mode);
        Ok(())
    }

    fn click_1_8(&mut self, slot: i16, button: u8, mode: u8) -> R<Option<Stack>> {
        match mode {
            0 | 1 if button <= 1 => {
                if slot == OUTSIDE {
                    if let Some(c) = &mut self.m.cursor {
                        if button == 0 {
                            self.m.cursor = None;
                        } else {
                            c.count -= 1;
                            if c.count == 0 {
                                self.m.cursor = None;
                            }
                        }
                    }
                    return Ok(None);
                }
                let Ok(i) = usize::try_from(slot) else {
                    return Ok(None);
                };
                if mode == 1 {
                    self.quick_move_1_8(i)
                } else {
                    self.pickup_1_8(i, button)
                }
            }
            2 if button < 9 => {
                if let Ok(i) = usize::try_from(slot) {
                    self.swap_1_8(i, usize::from(button))?;
                }
                Ok(None)
            }
            4 if self.m.cursor.is_none() => {
                if let Ok(i) = usize::try_from(slot) {
                    self.throw(i, button)?;
                }
                Ok(None)
            }
            6 => {
                if let Ok(i) = usize::try_from(slot) {
                    self.pickup_all(i, button)?;
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn click_1_16(&mut self, slot: i16, button: u8, mode: u8) -> R<Option<Stack>> {
        match mode {
            0 | 1 if button <= 1 => {
                if slot == OUTSIDE {
                    if let Some(c) = &mut self.m.cursor {
                        if button == 0 {
                            self.m.cursor = None;
                        } else {
                            c.count -= 1;
                            if c.count <= 0 {
                                self.m.cursor = None;
                            }
                        }
                    }
                    return Ok(None);
                }
                let Ok(i) = usize::try_from(slot) else {
                    return Ok(None);
                };
                if mode == 1 {
                    self.quick_move_1_16(i)
                } else {
                    self.pickup_1_16(i, button)
                }
            }
            // 1.12 swaps with the hotbar only (buttons 0..9), 1.16 also with the offhand (40)
            2 if matches!(self.v, Version::V1_16_5 | Version::V26_1) || button < 9 => {
                if let Ok(i) = usize::try_from(slot) {
                    self.swap_1_16(i, usize::from(button))?;
                }
                Ok(None)
            }
            4 if self.m.cursor.is_none() => {
                if let Ok(i) = usize::try_from(slot) {
                    self.throw(i, button)?;
                }
                Ok(None)
            }
            6 => {
                if let Ok(i) = usize::try_from(slot) {
                    self.pickup_all(i, button)?;
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    // ---------------------------------------------------------------- PICKUP

    fn pickup_1_8(&mut self, i: usize, button: u8) -> R<Option<Stack>> {
        let v = self.v;
        let slot = self.get(i).cloned();
        let ret = slot.clone();
        let cursor = self.m.cursor.clone();
        match (slot, cursor) {
            (None, None) => {}
            (None, Some(mut c)) => {
                if self.can_insert(i, &c)? {
                    let mut l = if button == 0 { c.count } else { 1 };
                    let lim = self.slot_limit(i)?;
                    if l > lim {
                        l = lim;
                    }
                    if c.count >= l {
                        c.count -= l;
                        self.set(i, Some(c.with_count(l)));
                    }
                    self.m.cursor = (c.count != 0).then_some(c);
                }
            }
            (Some(s), cursor) => {
                if !self.can_take(i)? {
                    return Ok(ret);
                }
                match cursor {
                    None => {
                        let m = if button == 0 {
                            s.count
                        } else {
                            (s.count + 1) / 2
                        };
                        self.m.cursor = self.take(i, m);
                        self.on_take(i)?;
                    }
                    Some(mut c) => {
                        if self.can_insert(i, &c)? {
                            if s.id == c.id && s.damage == c.damage && s.nbt_eq(&c) {
                                let mut n = if button == 0 { c.count } else { 1 };
                                let lim = self.slot_limit(i)?;
                                if n > lim - s.count {
                                    n = lim - s.count;
                                }
                                let max = v.max_count(&c);
                                if n > max - s.count {
                                    n = max - s.count;
                                }
                                c.count -= n;
                                self.m.cursor = (c.count != 0).then_some(c);
                                if let Some(t) = self.slot_mut(i) {
                                    t.count += n;
                                }
                            } else if c.count <= self.slot_limit(i)? {
                                self.set(i, Some(c));
                                self.m.cursor = Some(s);
                            }
                        } else if s.id == c.id
                            && v.max_count(&c) > 1
                            && (!v.has_subtypes(s.id) || s.damage == c.damage)
                            && s.nbt_eq(&c)
                        {
                            let o = s.count;
                            if o > 0 && o + c.count <= v.max_count(&c) {
                                c.count += o;
                                self.m.cursor = Some(c);
                                let _ = self.take(i, o);
                                self.on_take(i)?;
                            }
                        }
                    }
                }
            }
        }
        Ok(ret)
    }

    fn pickup_1_16(&mut self, i: usize, button: u8) -> R<Option<Stack>> {
        let v = self.v;
        let slot = self.get(i).cloned();
        let ret = slot.clone();
        let cursor = self.m.cursor.clone();
        match (slot, cursor) {
            (None, None) => {}
            (None, Some(mut c)) => {
                if self.can_insert(i, &c)? {
                    let mut o = if button == 0 { c.count } else { 1 };
                    let lim = self.slot_limit(i)?;
                    if o > lim {
                        o = lim;
                    }
                    // split(o) moves min(o, count)
                    let n = o.min(c.count).max(0);
                    c.count -= n;
                    let placed = c.with_count(n);
                    let placed = self.norm(placed);
                    self.set(i, placed);
                    self.m.cursor = self.norm(c);
                }
            }
            (Some(s), cursor) => {
                if !self.can_take(i)? {
                    return Ok(ret);
                }
                match cursor {
                    None => {
                        let p = if button == 0 {
                            s.count
                        } else {
                            (s.count + 1) / 2
                        };
                        self.m.cursor = self.take(i, p);
                        self.on_take(i)?;
                    }
                    Some(mut c) => {
                        if self.can_insert(i, &c)? {
                            if s.id == c.id
                                && (v != Version::V1_12 || s.damage == c.damage)
                                && s.nbt_eq(&c)
                            {
                                let mut q = if button == 0 { c.count } else { 1 };
                                let lim = self.slot_limit(i)?;
                                if q > lim - s.count {
                                    q = lim - s.count;
                                }
                                let max = v.max_count(&c);
                                if q > max - s.count {
                                    q = max - s.count;
                                }
                                c.count -= q;
                                self.m.cursor = self.norm(c);
                                let t = s.with_count(s.count + q);
                                let t = self.norm(t);
                                self.set(i, t);
                            } else if c.count <= self.slot_limit(i)? {
                                self.set(i, Some(c));
                                self.m.cursor = Some(s);
                            }
                        } else if v.max_count(&c) > 1
                            && s.id == c.id
                            && (!v.has_subtypes(s.id) || s.damage == c.damage)
                            && s.nbt_eq(&c)
                        {
                            let r = s.count;
                            if r + c.count <= v.max_count(&c) {
                                c.count += r;
                                self.m.cursor = self.norm(c);
                                let _ = self.take(i, r);
                                self.on_take(i)?;
                            }
                        }
                    }
                }
            }
        }
        Ok(ret)
    }

    // ---------------------------------------------------------------- QUICK_MOVE

    fn quick_move_1_8(&mut self, i: usize) -> R<Option<Stack>> {
        if !self.can_take(i)? {
            return Ok(None);
        }
        let Some(first) = self.transfer(i)? else {
            return Ok(None);
        };
        // Vanilla's shift-click retry: run the shift-click again while the slot still holds the item the
        // last transfer moved (each round moves at least one item, so this ends).
        let mut item = first.id;
        for _ in 0..4096 {
            if self.get(i).is_none_or(|s| s.id != item) || !self.can_take(i)? {
                return Ok(Some(first));
            }
            match self.transfer(i)? {
                Some(t) => item = t.id,
                None => return Ok(Some(first)),
            }
        }
        Err(Unpredictable)
    }

    fn quick_move_1_16(&mut self, i: usize) -> R<Option<Stack>> {
        if !self.can_take(i)? {
            return Ok(None);
        }
        // Keeps transferring while the slot still holds the same item; returns the last such
        // transfer (EMPTY when the first one emptied the slot).
        let mut ret = None;
        for _ in 0..4096 {
            match self.transfer(i)? {
                // 1.12 `ItemStack.equalsIgnoreNbt`: same item and damage; 1.16: same item
                Some(t)
                    if self.get(i).is_some_and(|s| {
                        s.id == t.id && (self.v != Version::V1_12 || s.damage == t.damage)
                    }) =>
                {
                    ret = Some(t);
                }
                _ => return Ok(ret),
            }
        }
        Err(Unpredictable)
    }

    /// The screen handler's `transferSlot`.
    fn transfer(&mut self, i: usize) -> R<Option<Stack>> {
        let Some(orig) = self.get(i).cloned() else {
            return Ok(None);
        };
        let mut st = orig.clone();
        let v = self.v;
        let (ok, eq_check) = match self.lay.screen {
            Screen::Player => {
                let ok = if i == 0 {
                    self.insert(&mut st, 9, 45, true)?
                } else if (1..9).contains(&i) {
                    self.insert(&mut st, 9, 45, false)?
                } else if let Some(a) = v
                    .armor_target(&orig)
                    .filter(|&a| self.get(5 + usize::from(a)).is_none())
                {
                    let a = 5 + usize::from(a);
                    self.insert(&mut st, a, a + 1, false)?
                } else if v.prefers_offhand(&orig) && self.get(45).is_none() {
                    self.insert(&mut st, 45, 46, false)?
                } else if (9..36).contains(&i) {
                    self.insert(&mut st, 36, 45, false)?
                } else if (36..45).contains(&i) {
                    self.insert(&mut st, 9, 36, false)?
                } else {
                    self.insert(&mut st, 9, 45, false)?
                };
                (ok, true)
            }
            Screen::Container(kind) => {
                let (n, len) = (self.lay.n, self.lay.len);
                match kind {
                    WindowKind::Chest { .. } | WindowKind::Hopper | WindowKind::ShulkerBox => {
                        let ok = if i < n {
                            self.insert(&mut st, n, len, true)?
                        } else {
                            self.insert(&mut st, 0, n, false)?
                        };
                        (ok, false)
                    }
                    WindowKind::Generic3x3 => {
                        let ok = if i < 9 {
                            self.insert(&mut st, 9, 45, true)?
                        } else {
                            self.insert(&mut st, 0, 9, false)?
                        };
                        (ok, true)
                    }
                    WindowKind::Crafting => {
                        let ok = if i == 0 {
                            self.insert(&mut st, 10, 46, true)?
                        } else if (10..46).contains(&i) {
                            match v {
                                Version::V1_8 | Version::V1_12 => {
                                    if i < 37 {
                                        self.insert(&mut st, 37, 46, false)?
                                    } else {
                                        self.insert(&mut st, 10, 37, false)?
                                    }
                                }
                                Version::V1_16_5 | Version::V26_1 => {
                                    self.insert(&mut st, 1, 10, false)?
                                        || if i < 37 {
                                            self.insert(&mut st, 37, 46, false)?
                                        } else {
                                            self.insert(&mut st, 10, 37, false)?
                                        }
                                }
                            }
                        } else {
                            self.insert(&mut st, 10, 46, false)?
                        };
                        (ok, true)
                    }
                    WindowKind::Furnace => {
                        let ok = match i {
                            2 => self.insert(&mut st, 3, 39, true)?,
                            0 | 1 => self.insert(&mut st, 3, 39, false)?,
                            _ => return Err(Unpredictable), // smelting recipes and fuel
                        };
                        (ok, true)
                    }
                    _ => return Err(Unpredictable),
                }
            }
        };
        if !ok {
            return Ok(None);
        }
        let changed = st.count != orig.count;
        let left = self.norm(st);
        self.set(i, left);
        if eq_check && !changed {
            return Ok(None);
        }
        if eq_check {
            self.on_take(i)?;
        }
        Ok(Some(orig))
    }

    /// `ScreenHandler.insertItem` of `st` into slots `start..end`.
    fn insert(&mut self, st: &mut Stack, start: usize, end: usize, rev: bool) -> R<bool> {
        let v = self.v;
        let end = end.min(self.lay.len);
        if start >= end {
            return Ok(false);
        }
        let order: Vec<usize> = if rev {
            (start..end).rev().collect()
        } else {
            (start..end).collect()
        };
        let mut moved = false;
        let max = v.max_count(st);
        if v.is_stackable(st) {
            for &i in &order {
                if st.count <= 0 {
                    break;
                }
                let Some(t) = self.get(i) else { continue };
                let same = match v {
                    Version::V1_8 | Version::V1_12 => {
                        t.id == st.id
                            && (!v.has_subtypes(st.id) || st.damage == t.damage)
                            && t.nbt_eq(st)
                    }
                    Version::V1_16_5 | Version::V26_1 => t.id == st.id && t.nbt_eq(st),
                };
                if !same {
                    continue;
                }
                let tc = t.count;
                let j = tc + st.count;
                // 26.1 `slot.getMaxStackSize(target)`: the slot's limit counts too
                let max = if v == Version::V26_1 {
                    max.min(self.slot_limit(i)?)
                } else {
                    max
                };
                if j <= max {
                    st.count = 0;
                    if let Some(t) = self.slot_mut(i) {
                        t.count = j;
                    }
                    moved = true;
                } else if tc < max {
                    st.count -= max - tc;
                    if let Some(t) = self.slot_mut(i) {
                        t.count = max;
                    }
                    moved = true;
                }
            }
        }
        if st.count > 0 {
            for &i in &order {
                if self.get(i).is_some() {
                    continue;
                }
                match v {
                    Version::V1_8 => {
                        self.set(i, Some(st.clone()));
                        st.count = 0;
                    }
                    Version::V1_12 | Version::V1_16_5 | Version::V26_1 => {
                        if !self.can_insert(i, st)? {
                            continue;
                        }
                        let n = st.count.min(self.slot_limit(i)?);
                        st.count -= n;
                        self.set(i, Some(st.with_count(n)));
                    }
                }
                moved = true;
                break;
            }
        }
        Ok(moved)
    }

    // ---------------------------------------------------------------- SWAP

    fn first_empty_main(&self) -> Option<usize> {
        (0..36).find(|&k| self.m.player[k].is_none())
    }

    fn swap_1_8(&mut self, i: usize, k: usize) -> R<()> {
        if !self.can_take(i)? {
            return Ok(());
        }
        let hot = self.m.player[k].clone();
        let own = self.lay.own(i);
        let mut fits = match &hot {
            None => true,
            Some(h) => own && self.can_insert(i, h)?,
        };
        let mut empty = None;
        if !fits {
            empty = self.first_empty_main();
            fits = empty.is_some();
        }
        let slot = self.get(i).cloned();
        match slot {
            Some(s) if fits => {
                self.m.player[k] = Some(s.clone());
                let displaced = match &hot {
                    Some(h) => !own || !self.can_insert(i, h)?,
                    None => false,
                };
                if displaced {
                    if empty.is_some() {
                        if let Some(h) = hot {
                            self.insert_stack(h);
                        }
                        let _ = self.take(i, s.count);
                        self.set(i, None);
                        self.on_take(i)?;
                    }
                } else {
                    let _ = self.take(i, s.count);
                    self.set(i, hot);
                    self.on_take(i)?;
                }
            }
            None => {
                if let Some(h) = hot
                    && self.can_insert(i, &h)?
                {
                    self.m.player[k] = None;
                    self.set(i, Some(h));
                }
            }
            Some(_) => {}
        }
        Ok(())
    }

    fn swap_1_16(&mut self, i: usize, k: usize) -> R<()> {
        if k >= 41 {
            return Ok(());
        }
        let hot = self.m.player[k].clone();
        let slot = self.get(i).cloned();
        match (hot, slot) {
            (None, None) => {}
            (None, Some(s)) => {
                if self.can_take(i)? {
                    self.m.player[k] = Some(s);
                    self.set(i, None);
                    self.on_take(i)?;
                }
            }
            (Some(mut h), None) => {
                if self.can_insert(i, &h)? {
                    let lim = self.slot_limit(i)?;
                    if h.count > lim {
                        h.count -= lim;
                        self.set(i, Some(h.with_count(lim)));
                        self.m.player[k] = Some(h);
                    } else {
                        self.set(i, Some(h));
                        self.m.player[k] = None;
                    }
                }
            }
            (Some(mut h), Some(s)) => {
                if self.can_take(i)? && self.can_insert(i, &h)? {
                    let lim = self.slot_limit(i)?;
                    if h.count > lim {
                        h.count -= lim;
                        self.set(i, Some(h.with_count(lim)));
                        self.m.player[k] = Some(h);
                        self.on_take(i)?;
                        // what does not fit is dropped
                        self.insert_stack(s);
                    } else {
                        self.set(i, Some(h));
                        self.m.player[k] = Some(s);
                        self.on_take(i)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// `PlayerInventory.insertStack(stack)`; whatever does not
    /// fit is dropped by the caller (no inventory effect).
    fn insert_stack(&mut self, mut st: Stack) {
        let v = self.v;
        match v {
            Version::V1_8 => {
                if st.count == 0 || !v.known(st.id) {
                    return;
                }
            }
            Version::V1_12 | Version::V1_16_5 | Version::V26_1 => {
                if st.count <= 0 {
                    return;
                }
            }
        }
        if v.is_damaged(&st) {
            if let Some(j) = self.first_empty_main() {
                self.m.player[j] = Some(st);
            }
            return;
        }
        for _ in 0..4096 {
            let before = st.count;
            st.count = self.add_stack(&st);
            if !(st.count > 0 && st.count < before) {
                return;
            }
        }
    }

    /// One round of `insertStack`: the slot with room (1.16.5: selected slot, then offhand,
    /// then 0..36) or the first empty one. Returns what is left.
    fn add_stack(&mut self, st: &Stack) -> i32 {
        let v = self.v;
        let room = |m: &Model, j: usize| {
            m.player[j].as_ref().is_some_and(|t| {
                t.id == st.id
                    && v.is_stackable(t)
                    && t.count < v.max_count(t)
                    && t.count < if v == Version::V26_1 { 99 } else { 64 }
                    && match v {
                        Version::V1_8 | Version::V1_12 => {
                            !v.has_subtypes(t.id) || t.damage == st.damage
                        }
                        Version::V1_16_5 | Version::V26_1 => true,
                    }
                    && t.nbt_eq(st)
            })
        };
        let found = match v {
            Version::V1_8 => (0..36).find(|&j| room(self.m, j)),
            Version::V1_12 | Version::V1_16_5 | Version::V26_1 => {
                let sel = usize::from(self.m.selected.min(8));
                if room(self.m, sel) {
                    Some(sel)
                } else if room(self.m, 40) {
                    Some(40)
                } else {
                    (0..36).find(|&j| room(self.m, j))
                }
            }
        };
        let Some(j) = found.or_else(|| self.first_empty_main()) else {
            return st.count;
        };
        let mut i = st.count;
        let created = self.m.player[j].is_none();
        let t = self.m.player[j].get_or_insert_with(|| Stack {
            id: st.id,
            count: 0,
            damage: st.damage,
            data: st.data.clone(),
        });
        // 1.12 / 1.16.5: the fresh zero-count stack is empty, and an empty stack's item is AIR
        // (max 64)
        let empty_stacks = v != Version::V1_8;
        let max = if created && empty_stacks {
            64
        } else {
            v.max_count(t)
        };
        let mut k = i;
        if i > max - t.count {
            k = max - t.count;
        }
        let cap = if v == Version::V26_1 { 99 } else { 64 };
        if k > cap - t.count {
            k = cap - t.count;
        }
        if k == 0 {
            if created && empty_stacks {
                self.m.player[j] = None; // a zero-count stack is empty on 1.12 / 1.16
            }
            return i;
        }
        i -= k;
        t.count += k;
        if empty_stacks && t.count <= 0 {
            self.m.player[j] = None;
        }
        i
    }

    // ---------------------------------------------------------------- THROW, PICKUP_ALL

    fn throw(&mut self, i: usize, button: u8) -> R<()> {
        let Some(count) = self.get(i).map(|s| s.count) else {
            return Ok(());
        };
        if self.can_take(i)? {
            let _ = self.take(i, if button == 0 { 1 } else { count });
            self.on_take(i)?;
        }
        Ok(())
    }

    fn pickup_all(&mut self, i: usize, button: u8) -> R<()> {
        let v = self.v;
        let Some(mut c) = self.m.cursor.clone() else {
            return Ok(());
        };
        let clicked_takeable = match self.get(i) {
            None => false,
            Some(_) => self.can_take(i)?,
        };
        if clicked_takeable {
            return Ok(());
        }
        let len = self.lay.len;
        let max = v.max_count(&c);
        for pass in 0..2 {
            let order: Vec<usize> = if button == 0 {
                (0..len).collect()
            } else {
                (0..len).rev().collect()
            };
            for x in order {
                if c.count >= max {
                    break;
                }
                // canInsertIntoSlot(stack, slot) is false for crafting results
                if self.lay.is_result(x) {
                    continue;
                }
                let Some(t) = self.get(x) else { continue };
                // canInsertItemIntoSlot(slot, cursor, true)
                let same = match v {
                    Version::V1_8 | Version::V1_12 => c.id == t.id && c.damage == t.damage,
                    Version::V1_16_5 | Version::V26_1 => c.id == t.id,
                };
                if !(same && t.nbt_eq(&c) && t.count <= max) {
                    continue;
                }
                let tc = t.count;
                let tmax = v.max_count(t);
                if !self.can_take(x)? {
                    continue;
                }
                if pass == 0 && tc == tmax {
                    continue;
                }
                let u = (max - c.count).min(tc);
                let taken = self.take(x, u);
                c.count += u;
                if v == Version::V1_8 && taken.as_ref().is_some_and(|s| s.count <= 0) {
                    self.set(x, None);
                }
                self.on_take(x)?;
            }
        }
        // the cursor object is only grown in place (1.8 keeps it even at count 0)
        self.m.cursor = match v {
            Version::V1_8 => Some(c),
            Version::V1_12 | Version::V1_16_5 | Version::V26_1 => self.norm(c),
        };
        Ok(())
    }
}

/// 1.8: the client's own result for the screen's crafting grid (`RecipeDispatcher`), written
/// into the result slot; unknown when a recipe ghast does not model may decide it.
pub(crate) fn recompute_1_8(m: &mut Model, lay: Layout) {
    let width = match lay.screen {
        Screen::Player => 2,
        Screen::Container(WindowKind::Crafting) => 3,
        Screen::Container(_) => return,
    };
    let (grid, memo): (&[Option<Stack>], _) = match lay.screen {
        Screen::Player => (&m.craft[..], &m.craft_memo),
        _ => match m.container.get(1..10) {
            Some(g) => (g, &m.table_memo),
            None => return,
        },
    };
    if grid.len() != width * width {
        return;
    }
    let (value, known, memo) = match memo {
        Some(memo) if memo.grid[..] == *grid => {
            debug_assert_eq!(craft_1_8(grid, width), (memo.result.clone(), memo.known));
            (memo.result.clone(), memo.known, None)
        }
        _ => {
            let (value, known) = craft_1_8(grid, width);
            let memo = std::sync::Arc::new(CraftMemo {
                grid: grid.to_vec(),
                result: value.clone(),
                known,
            });
            (value, known, Some(memo))
        }
    };
    match lay.screen {
        Screen::Player => {
            m.craft_result = value;
            m.craft_result_known = known;
            if memo.is_some() {
                m.craft_memo = memo;
            }
        }
        _ => {
            m.container[0] = value;
            m.container_result_known = known;
            if memo.is_some() {
                m.table_memo = memo;
            }
        }
    }
}
