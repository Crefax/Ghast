//! The 26.1 click simulation: what the vanilla 26.1 client does with a click
//! (`AbstractContainerMenu.doClick` with every `ContainerInput`, quick-craft drags included),
//! the slot rules of every menu a server can open (`Slot` and its subclasses, the containers'
//! `setItem` / `removeItem` / `setChanged`), each menu's quick move and the client-side result
//! updates some menus make (smithing, stonecutter, cartography, grindstone when its result is
//! empty, crafting results taken with their remainders), bundles and creative mode.
//!
//! The simulation works on the same [`Model`] as the older versions but follows the 26.1
//! control flow, which differs in many places (slot-dependent stack limits, `safeInsert` /
//! `tryRemove`, quick-move loops, drags, bundles). Whatever depends on data the client gets
//! from elsewhere (villager offers, anvil and loom results, the player's experience when it is
//! unknown) returns [`Unpredictable`]; the caller then sends the click without a prediction.
//!
//! Proven against the real client's click handling on every menu type
//! (`tests/oracle_clicks_26_1.rs`).

use bytes::{BufMut, Bytes, BytesMut};
use ghast_protocol::codec::{McWrite, Reader};
use ghast_protocol::types::slot::v775::ComponentPatch;

use crate::sim::{Loc, Model, OUTSIDE, Unpredictable};
use crate::stack::{ItemData, Stack};
use crate::v26_1::{self as v, Equip, component as c};

type R<T> = Result<T, Unpredictable>;

/// The menus a 26.1 server opens with `open_screen` (`minecraft:menu` registry order), plus
/// window 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    /// `InventoryMenu` (window 0).
    Player,
    /// `ChestMenu` with this many rows.
    Chest(u8),
    /// `DispenserMenu`.
    Dispenser,
    /// `CrafterMenu`: 9 grid slots, the player's 36, then a result slot (index 45).
    Crafter,
    Anvil,
    Beacon,
    /// `AbstractFurnaceMenu`: furnace, blast furnace, smoker (their inputs differ).
    Furnace(Oven),
    Brewing,
    Crafting,
    Enchanting,
    Grindstone,
    Hopper,
    /// `LecternMenu`: one slot, no player inventory.
    Lectern,
    Loom,
    Merchant,
    Shulker,
    Smithing,
    Cartography,
    Stonecutter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Oven {
    Furnace,
    Blast,
    Smoker,
}

impl Kind {
    /// The menu of `open_screen`'s `minecraft:menu` registry id.
    pub(crate) fn from_menu(menu: i32) -> Option<Kind> {
        Some(match menu {
            0..=5 => Kind::Chest(menu as u8 + 1),
            6 => Kind::Dispenser,
            7 => Kind::Crafter,
            8 => Kind::Anvil,
            9 => Kind::Beacon,
            10 => Kind::Furnace(Oven::Blast),
            11 => Kind::Brewing,
            12 => Kind::Crafting,
            13 => Kind::Enchanting,
            14 => Kind::Furnace(Oven::Furnace),
            15 => Kind::Grindstone,
            16 => Kind::Hopper,
            17 => Kind::Lectern,
            18 => Kind::Loom,
            19 => Kind::Merchant,
            20 => Kind::Shulker,
            21 => Kind::Smithing,
            22 => Kind::Furnace(Oven::Smoker),
            23 => Kind::Cartography,
            24 => Kind::Stonecutter,
            _ => return None,
        })
    }

    /// Container slots before the player's 36 (all of them for the lectern).
    pub(crate) fn head(self) -> usize {
        match self {
            Kind::Player => 0,
            Kind::Chest(r) => usize::from(r) * 9,
            Kind::Dispenser | Kind::Crafter => 9,
            Kind::Anvil | Kind::Grindstone | Kind::Merchant | Kind::Cartography => 3,
            Kind::Furnace(_) => 3,
            Kind::Beacon | Kind::Lectern => 1,
            Kind::Brewing | Kind::Hopper => 5,
            Kind::Crafting => 10,
            Kind::Enchanting | Kind::Stonecutter => 2,
            Kind::Loom | Kind::Smithing => 4,
            Kind::Shulker => 27,
        }
    }

    /// Screen slots.
    pub(crate) fn len(self) -> usize {
        match self {
            Kind::Player => 46,
            Kind::Lectern => 1,
            Kind::Crafter => 46,
            k => k.head() + 36,
        }
    }

    /// Where screen slot `i` keeps its stack.
    pub(crate) fn loc(self, i: usize) -> Option<Loc> {
        if i >= self.len() {
            return None;
        }
        Some(match self {
            Kind::Player => match i {
                0 => Loc::CraftResult,
                1..=4 => Loc::Craft(i - 1),
                5..=8 => Loc::Player(39 - (i - 5)),
                9..=35 => Loc::Player(i),
                36..=44 => Loc::Player(i - 36),
                _ => Loc::Player(40),
            },
            Kind::Lectern => Loc::Container(0),
            k => {
                let n = k.head();
                if i < n {
                    Loc::Container(i)
                } else if i < n + 27 {
                    Loc::Player(i - n + 9)
                } else if i < n + 36 {
                    Loc::Player(i - n - 27)
                } else {
                    Loc::Container(n + i - n - 36)
                }
            }
        })
    }

    /// The container's stack cap applied by `setItem` (`SimpleContainer` / `MerchantContainer`
    /// `limitSize(min(getMaxStackSize(), stack max))`); `None` for containers that store as is
    /// (player inventory, crafting grids, result containers).
    pub(crate) fn cap(self, l: Loc) -> Option<i32> {
        let Loc::Container(k) = l else {
            return None;
        };
        match (self, k) {
            (Kind::Crafting | Kind::Crafter, _) => None,
            (Kind::Beacon, _) => Some(1),
            (Kind::Anvil | Kind::Grindstone | Kind::Cartography, 2) => None,
            (Kind::Smithing, 3) | (Kind::Stonecutter, 1) => None,
            _ => Some(99),
        }
    }
}

/// What a slot is (`Slot` subclass and the container it reads).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ty {
    /// Player inventory slot (main, hotbar) or window 0's offhand.
    Inv,
    /// Window 0 armor slot: 0 head … 3 feet.
    Armor(u8),
    /// Crafting `ResultSlot` (window 0 slot 0, crafting table slot 0).
    Result,
    /// Crafting grid (`TransientCraftingContainer`).
    Grid,
    /// `CrafterSlot` (index into the lock flags).
    CrafterGrid,
    /// `NonInteractiveResultSlot` (crafter result).
    Fixed,
    /// Plain `Slot` over a `SimpleContainer` (chests, hopper, dispenser, lectern, furnace input).
    Plain,
    Shulker,
    FurnaceFuel,
    FurnaceResult,
    Potion,
    BrewIngredient,
    BrewFuel,
    EnchantItem,
    Lapis,
    Payment,
    /// Anvil / smithing input `k`.
    CombinerIn(u8),
    CombinerResult,
    GrindIn,
    GrindResult,
    CutterIn,
    CutterResult,
    LoomBanner,
    LoomDye,
    LoomPattern,
    LoomResult,
    MapIn,
    MapExtra,
    MapResult,
    TradeIn,
    TradeResult,
}

fn ty(kind: Kind, i: usize) -> Ty {
    if kind == Kind::Player && (5..=8).contains(&i) {
        return Ty::Armor((i - 5) as u8);
    }
    let l = kind.loc(i);
    match l {
        Some(Loc::Player(_)) | None => return Ty::Inv,
        Some(Loc::Craft(_)) => return Ty::Grid,
        Some(Loc::CraftResult) => return Ty::Result,
        Some(Loc::Container(_)) => {}
    }
    let Some(Loc::Container(k)) = l else {
        return Ty::Inv;
    };
    match kind {
        Kind::Player => Ty::Inv,
        Kind::Chest(_) | Kind::Dispenser | Kind::Hopper | Kind::Lectern => Ty::Plain,
        Kind::Shulker => Ty::Shulker,
        Kind::Crafter => {
            if k < 9 {
                Ty::CrafterGrid
            } else {
                Ty::Fixed
            }
        }
        Kind::Crafting => {
            if k == 0 {
                Ty::Result
            } else {
                Ty::Grid
            }
        }
        Kind::Furnace(_) => match k {
            0 => Ty::Plain,
            1 => Ty::FurnaceFuel,
            _ => Ty::FurnaceResult,
        },
        Kind::Brewing => match k {
            0..=2 => Ty::Potion,
            3 => Ty::BrewIngredient,
            _ => Ty::BrewFuel,
        },
        Kind::Enchanting => {
            if k == 0 {
                Ty::EnchantItem
            } else {
                Ty::Lapis
            }
        }
        Kind::Beacon => Ty::Payment,
        Kind::Anvil => {
            if k < 2 {
                Ty::CombinerIn(k as u8)
            } else {
                Ty::CombinerResult
            }
        }
        Kind::Smithing => {
            if k < 3 {
                Ty::CombinerIn(k as u8)
            } else {
                Ty::CombinerResult
            }
        }
        Kind::Grindstone => {
            if k < 2 {
                Ty::GrindIn
            } else {
                Ty::GrindResult
            }
        }
        Kind::Stonecutter => {
            if k == 0 {
                Ty::CutterIn
            } else {
                Ty::CutterResult
            }
        }
        Kind::Loom => match k {
            0 => Ty::LoomBanner,
            1 => Ty::LoomDye,
            2 => Ty::LoomPattern,
            _ => Ty::LoomResult,
        },
        Kind::Cartography => match k {
            0 => Ty::MapIn,
            1 => Ty::MapExtra,
            _ => Ty::MapResult,
        },
        Kind::Merchant => {
            if k < 2 {
                Ty::TradeIn
            } else {
                Ty::TradeResult
            }
        }
    }
}

/// A quick-craft (drag) in progress: `AbstractContainerMenu.quickcraftStatus`,
/// `quickcraftType` and `quickcraftSlots`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Drag {
    status: u8,
    ty: u8,
    slots: Vec<u16>,
}

impl Drag {
    /// `resetQuickCraft`.
    pub(crate) fn reset(&mut self) {
        self.status = 0;
        self.slots.clear();
    }
}

/// What the client knows from elsewhere: the server's recipe item sets, its game mode and
/// experience level, the open menu's data slots.
#[derive(Debug, Clone)]
pub(crate) struct World26 {
    /// `RecipePropertySet`s from `update_recipes` (sorted item ids).
    pub furnace_input: Vec<i32>,
    pub blast_input: Vec<i32>,
    pub smoker_input: Vec<i32>,
    pub smithing_base: Vec<i32>,
    pub smithing_template: Vec<i32>,
    pub smithing_addition: Vec<i32>,
    /// Stonecutter recipe inputs (sorted item ids); `None` when one names a tag.
    pub cutter_inputs: Option<Vec<i32>>,
    /// `hasInfiniteMaterials()` (abilities: instant build) and `isCreative()`.
    pub creative: bool,
    /// `experienceLevel`, if the server sent it.
    pub xp_level: Option<i32>,
    /// `minecraft:binding_curse` registry id (the `prevent_armor_change` enchantment).
    pub binding_curse: Option<i32>,
    /// `player.canDropItems()` (the drop spam throttle is below its threshold).
    pub can_drop: bool,
}

impl Default for World26 {
    fn default() -> Self {
        World26 {
            furnace_input: Vec::new(),
            blast_input: Vec::new(),
            smoker_input: Vec::new(),
            smithing_base: Vec::new(),
            smithing_template: Vec::new(),
            smithing_addition: Vec::new(),
            cutter_inputs: Some(Vec::new()),
            creative: false,
            xp_level: None,
            binding_curse: None,
            can_drop: true,
        }
    }
}

fn has(set: &[i32], id: i32) -> bool {
    set.binary_search(&id).is_ok()
}

/// Per-screen state the client keeps between clicks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Screen26 {
    /// Drag in progress.
    pub drag: Drag,
    /// Stonecutter: the item of `StonecutterMenu.input` (0 = empty).
    pub cutter_input: i32,
    /// Anvil: the client's `cost` data slot (set by the server, and by `createResult`).
    pub anvil_cost: i32,
    /// Crafter: locked slots (data slots 0..9 = 1).
    pub crafter_locked: u16,
}

/// One click being simulated.
pub(crate) struct Sim26<'a> {
    pub kind: Kind,
    pub m: &'a mut Model,
    pub s: &'a mut Screen26,
    pub w: &'a World26,
    /// `ServerboundSwingPacket`s the click makes the client send first (each drop swings).
    pub swings: u8,
}

/// Fraction arithmetic of `org.apache.commons.lang3.math.Fraction` (reduced, positive
/// denominators), for bundle weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Frac(i64, i64);

impl Frac {
    const ZERO: Frac = Frac(0, 1);
    const ONE: Frac = Frac(1, 1);
    fn new(n: i64, d: i64) -> Frac {
        let g = gcd(n.abs(), d.abs()).max(1);
        let (n, d) = (n / g, d / g);
        if d < 0 { Frac(-n, -d) } else { Frac(n, d) }
    }
    fn add(self, o: Frac) -> Frac {
        Frac::new(self.0 * o.1 + o.0 * self.1, self.1 * o.1)
    }
    fn sub(self, o: Frac) -> Frac {
        Frac::new(self.0 * o.1 - o.0 * self.1, self.1 * o.1)
    }
    fn mul(self, k: i64) -> Frac {
        Frac::new(self.0 * k, self.1)
    }
    /// `divideBy(o).intValue()` (truncates toward zero).
    fn div_int(self, o: Frac) -> i64 {
        let f = Frac::new(self.0 * o.1, self.1 * o.0);
        f.0 / f.1
    }
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

// ------------------------------------------------------------------ item helpers

fn max_size(s: &Stack) -> i32 {
    v::max_stack_size(s)
}

/// `ItemStack.isSameItemSameComponents`.
fn same(a: &Stack, b: &Stack) -> bool {
    a.id == b.id && a.data_eq(b)
}

fn is_damaged(s: &Stack) -> bool {
    v::is_damageable(s) && v::damage_value(s) > 0
}

fn stackable(s: &Stack) -> bool {
    max_size(s) > 1 && !is_damaged(s)
}

fn norm(s: Stack) -> Option<Stack> {
    (s.count > 0 && s.id != 0).then_some(s)
}

/// `split(n)`: takes up to `n` off `s` (which may become empty).
fn split(s: &mut Option<Stack>, n: i32) -> Option<Stack> {
    let st = s.as_mut()?;
    let k = n.min(st.count);
    let out = st.with_count(k);
    st.count -= k;
    if st.count <= 0 {
        *s = None;
    }
    norm(out)
}

/// The equippable slot (`getEquipmentSlotForItem` ignores `allowed_entities`) and whether a
/// player may wear it (`canBeEquippedBy`).
fn equip(s: &Stack) -> R<(Equip, bool)> {
    match v::patched(s, c::EQUIPPABLE) {
        Some(Some(b)) => patched_equip(&b).ok_or(Unpredictable),
        Some(None) => Ok((Equip::None, true)),
        None => {
            Ok(v::defaults(s.id).map_or((Equip::None, true), |d| (d.equip, !d.equip_restricted)))
        }
    }
}

fn patched_equip(b: &Bytes) -> Option<(Equip, bool)> {
    let mut r = Reader::new(b);
    let slot = r.read_varint().ok()?;
    if r.read_varint().ok()? == 0 {
        r.read_string().ok()?;
        if r.read_bool().ok()? {
            r.read_f32().ok()?;
        }
    }
    for _ in 0..2 {
        if r.read_bool().ok()? {
            r.read_string().ok()?;
        }
    }
    let player = if r.read_bool().ok()? {
        // HolderSet: 0 + tag name (unknown contents) or n + 1 ids
        let n = r.read_varint().ok()?;
        if n == 0 {
            return None;
        }
        let me = v::ENTITY_TYPES.iter().position(|e| *e == "player")? as i32;
        let mut found = false;
        for _ in 0..n - 1 {
            found |= r.read_varint().ok()? == me;
        }
        found
    } else {
        true
    };
    let e = match slot {
        0 => Equip::MainHand,
        1 => Equip::Feet,
        2 => Equip::Legs,
        3 => Equip::Chest,
        4 => Equip::Head,
        5 => Equip::Offhand,
        6 => Equip::Body,
        7 => Equip::Saddle,
        _ => return None,
    };
    Some((e, player))
}

/// `isEquippableInSlot(stack, armor slot a)` for the player.
fn wears(s: &Stack, a: u8) -> R<bool> {
    let (e, player) = equip(s)?;
    Ok(player && e.armor_index() == Some(a))
}

/// `EnchantmentHelper.has(stack, PREVENT_ARMOR_CHANGE)`: the binding curse.
fn bound(s: &Stack, curse: Option<i32>) -> R<bool> {
    let Some(list) = v::enchantments(s, false) else {
        return Ok(false);
    };
    if list.is_empty() {
        return Ok(false);
    }
    let id = curse.ok_or(Unpredictable)?;
    Ok(list.iter().any(|&(e, level)| e == id && level > 0))
}

/// `EnchantmentHelper.hasAnyEnchantments`.
fn enchanted(s: &Stack) -> bool {
    v::enchantments(s, false).is_some_and(|l| !l.is_empty())
        || v::enchantments(s, true).is_some_and(|l| !l.is_empty())
}

fn has_component(s: &Stack, ty: i32, default: bool) -> bool {
    match v::patched(s, ty) {
        Some(x) => x.is_some(),
        None => default,
    }
}

fn fact(id: i32, bit: u16) -> bool {
    crate::v26_1::menu_facts(id) & bit != 0
}

// ------------------------------------------------------------------ bundles

/// One `ItemStackTemplate` of a `bundle_contents` list.
fn read_template(r: &mut Reader<'_>) -> Option<Stack> {
    let id = r.read_varint().ok()?;
    let count = r.read_varint().ok()?;
    let added = r.read_varint().ok()?;
    let removed = r.read_varint().ok()?;
    let p = ComponentPatch::read(r, added, removed).ok()?;
    Some(Stack {
        id,
        count,
        damage: 0,
        data: if p.is_empty() {
            ItemData::None
        } else {
            ItemData::Components(Box::new(p))
        },
    })
}

fn write_template(s: &Stack, w: &mut BytesMut) {
    w.put_varint(s.id);
    w.put_varint(s.count);
    match s.components() {
        Some(p) => {
            w.put_varint(p.added_len() as i32);
            w.put_varint(p.removed_len() as i32);
            w.put_slice(p.as_bytes());
        }
        None => {
            w.put_varint(0);
            w.put_varint(0);
        }
    }
}

/// `bundle_contents` of a stack: `Ok(None)` when it has none (not a bundle).
fn bundle_items(s: &Stack) -> R<Option<Vec<Stack>>> {
    let bytes = match v::patched(s, c::BUNDLE_CONTENTS) {
        Some(Some(b)) => b,
        Some(None) => return Ok(None),
        None => {
            return Ok(v::defaults(s.id).is_some_and(|d| d.bundle).then(Vec::new));
        }
    };
    let mut r = Reader::new(&bytes);
    let n = r.read_varint().map_err(|_| Unpredictable)?;
    let mut out = Vec::new();
    for _ in 0..n.max(0) {
        out.push(read_template(&mut r).ok_or(Unpredictable)?);
    }
    Ok(Some(out))
}

/// `BundleContents.getWeight(item)`.
fn weight(s: &Stack, depth: u32) -> R<Frac> {
    if depth > 16 {
        return Err(Unpredictable);
    }
    if let Some(items) = bundle_items(s)? {
        return Ok(items_weight(&items, depth + 1)?.add(Frac::new(1, 16)));
    }
    let bees = match v::patched(s, c::BEES) {
        Some(Some(b)) => Reader::new(&b).read_varint().map_err(|_| Unpredictable)? > 0,
        _ => false,
    };
    if bees {
        return Ok(Frac::ONE);
    }
    Ok(Frac::new(1, i64::from(max_size(s).max(1))))
}

fn items_weight(items: &[Stack], depth: u32) -> R<Frac> {
    let mut w = Frac::ZERO;
    for s in items {
        w = w.add(weight(s, depth)?.mul(i64::from(s.count)));
    }
    Ok(w)
}

/// `BundleContents.Mutable`.
struct Bundle {
    items: Vec<Stack>,
    weight: Frac,
}

impl Bundle {
    fn of(s: &Stack) -> R<Option<Bundle>> {
        let Some(items) = bundle_items(s)? else {
            return Ok(None);
        };
        let weight = items_weight(&items, 0)?;
        Ok(Some(Bundle { items, weight }))
    }

    fn max_to_add(&self, w: Frac) -> i32 {
        Frac::ONE.sub(self.weight).div_int(w).max(0) as i32
    }

    /// `tryInsert`: moves what fits off `add`; returns the amount.
    fn try_insert(&mut self, add: &mut Option<Stack>) -> R<i32> {
        let Some(st) = add.as_ref() else { return Ok(0) };
        if !fact(st.id, crate::v26_1::FITS_CONTAINER) {
            return Ok(0);
        }
        let w = weight(st, 0)?;
        let n = st.count.min(self.max_to_add(w));
        if n == 0 {
            return Ok(0);
        }
        self.weight = self.weight.add(w.mul(i64::from(n)));
        let idx = if stackable(st) {
            self.items.iter().position(|x| same(x, st))
        } else {
            None
        };
        match idx {
            Some(i) => {
                let old = self.items.remove(i);
                let merged = old.with_count(old.count + n);
                let _ = split(add, n);
                self.items.insert(0, merged);
            }
            None => {
                let part = split(add, n).ok_or(Unpredictable)?;
                self.items.insert(0, part);
            }
        }
        Ok(n)
    }

    /// `removeOne` (no item selected: the first one).
    fn remove_one(&mut self) -> R<Option<Stack>> {
        if self.items.is_empty() {
            return Ok(None);
        }
        let s = self.items.remove(0);
        self.weight = self.weight.sub(weight(&s, 0)?.mul(i64::from(s.count)));
        Ok(Some(s))
    }

    /// The stack with this content (`set(BUNDLE_CONTENTS, …)`: an empty list is the default
    /// of a bundle item and leaves the patch).
    fn store(&self, s: &mut Stack) {
        let mut w = BytesMut::new();
        w.put_varint(self.items.len() as i32);
        for t in &self.items {
            write_template(t, &mut w);
        }
        let value = w.freeze();
        let default_empty = v::defaults(s.id).is_some_and(|d| d.bundle);
        let keep_default = default_empty && self.items.is_empty();
        set_component(
            s,
            c::BUNDLE_CONTENTS,
            if keep_default { None } else { Some(&value) },
        );
    }
}

/// Replaces component `ty` in the stack's patch: `Some(value)` sets it, `None` drops the
/// patch entry (the item default applies again).
fn set_component(s: &mut Stack, ty: i32, value: Option<&[u8]>) {
    let mut added: Vec<(i32, Bytes)> = Vec::new();
    let mut removed: Vec<i32> = Vec::new();
    if let Some(p) = s.components() {
        for (t, b) in p.iter() {
            if t != ty {
                added.push((t, b));
            }
        }
        removed.extend(p.removed().filter(|&t| t != ty));
    }
    if let Some(v) = value {
        added.push((ty, Bytes::copy_from_slice(v)));
    }
    let parts: Vec<(i32, &[u8])> = added.iter().map(|(t, b)| (*t, b.as_ref())).collect();
    let p = ComponentPatch::from_parts(&parts, &removed);
    s.data = if p.is_empty() {
        ItemData::None
    } else {
        ItemData::Components(Box::new(p))
    };
}

// ------------------------------------------------------------------ the simulation

impl Sim26<'_> {
    fn len(&self) -> usize {
        self.kind.len()
    }

    fn loc(&self, i: usize) -> Loc {
        self.kind.loc(i).unwrap_or(Loc::Player(0))
    }

    fn get(&self, i: usize) -> Option<&Stack> {
        self.m.at(self.loc(i)).as_ref()
    }

    fn slot_mut(&mut self, i: usize) -> &mut Option<Stack> {
        let l = self.loc(i);
        self.m.at_mut(l)
    }

    fn ty(&self, i: usize) -> Ty {
        ty(self.kind, i)
    }

    /// The menu's container slot `k` (container index, not screen index).
    fn c(&self, k: usize) -> Option<&Stack> {
        self.m.container.get(k).and_then(Option::as_ref)
    }

    fn c_set_raw(&mut self, k: usize, s: Option<Stack>) {
        if let Some(x) = self.m.container.get_mut(k) {
            *x = s;
        }
    }

    // ---------------------------------------------------------- Slot rules

    /// `Slot.mayPlace`.
    fn may_place(&self, i: usize, s: &Stack) -> R<bool> {
        let id = s.id;
        Ok(match self.ty(i) {
            Ty::Inv | Ty::Grid | Ty::Plain | Ty::EnchantItem => true,
            Ty::Armor(a) => wears(s, a)?,
            Ty::Result
            | Ty::Fixed
            | Ty::FurnaceResult
            | Ty::CombinerResult
            | Ty::GrindResult
            | Ty::CutterResult
            | Ty::LoomResult
            | Ty::MapResult
            | Ty::TradeResult => false,
            Ty::CrafterGrid => self.s.crafter_locked & (1 << i) == 0,
            Ty::Shulker => fact(id, v::FITS_CONTAINER),
            Ty::FurnaceFuel => fact(id, v::FUEL) || is_item(s, "bucket"),
            Ty::Potion => potion_slot(s),
            Ty::BrewIngredient => fact(id, v::BREWING_INGREDIENT),
            Ty::BrewFuel => fact(id, v::BREWING_FUEL),
            Ty::Lapis => is_item(s, "lapis_lazuli"),
            Ty::Payment => fact(id, v::BEACON_PAYMENT),
            Ty::CombinerIn(k) => match self.kind {
                Kind::Smithing => match k {
                    0 => has(&self.w.smithing_template, id),
                    1 => has(&self.w.smithing_base, id),
                    _ => has(&self.w.smithing_addition, id),
                },
                _ => true,
            },
            Ty::GrindIn => v::is_damageable(s) || enchanted(s),
            Ty::CutterIn | Ty::TradeIn => true,
            Ty::LoomBanner => fact(id, v::BANNER),
            Ty::LoomDye => loom_dye(s),
            Ty::LoomPattern => loom_pattern(s),
            Ty::MapIn => has_component(s, c::MAP_ID, false),
            Ty::MapExtra => is_item(s, "paper") || is_item(s, "map") || is_item(s, "glass_pane"),
        })
    }

    /// `Slot.getMaxStackSize()`.
    fn slot_max(&self, i: usize) -> i32 {
        match self.ty(i) {
            Ty::Armor(_) | Ty::Potion | Ty::EnchantItem | Ty::Payment => 1,
            _ => 99,
        }
    }

    /// `Slot.getMaxStackSize(stack)`.
    fn max_for(&self, i: usize, s: &Stack) -> i32 {
        if self.ty(i) == Ty::FurnaceFuel && is_item(s, "bucket") {
            return 1;
        }
        self.slot_max(i).min(max_size(s))
    }

    /// `Slot.mayPickup`.
    fn may_pickup(&self, i: usize) -> R<bool> {
        Ok(match self.ty(i) {
            Ty::Armor(_) => match self.get(i) {
                Some(s) if !self.w.creative => !bound(s, self.w.binding_curse)?,
                _ => true,
            },
            Ty::Fixed => false,
            Ty::CombinerResult if self.kind == Kind::Anvil => {
                let cost = self.s.anvil_cost;
                if cost <= 0 {
                    false
                } else if self.w.creative {
                    true
                } else {
                    self.w.xp_level.ok_or(Unpredictable)? >= cost
                }
            }
            _ => true,
        })
    }

    /// `Slot.allowModification`.
    fn allow_modification(&self, i: usize) -> R<bool> {
        if self.ty(i) == Ty::Fixed {
            return Ok(false);
        }
        if !self.may_pickup(i)? {
            return Ok(false);
        }
        match self.get(i) {
            Some(s) => self.may_place(i, s),
            // mayPlace(EMPTY): only reached for empty slots, which never need it
            None => Ok(true),
        }
    }

    // ---------------------------------------------------------- containers

    /// `Slot.set(stack)`: the container's `setItem` (with its cap and hooks), then
    /// `setChanged`.
    fn set(&mut self, i: usize, mut s: Option<Stack>) -> R<()> {
        let l = self.loc(i);
        if let Some(cap) = self.kind.cap(l)
            && let Some(st) = &mut s
        {
            let max = cap.min(max_size(st));
            if st.count > max {
                st.count = max;
            }
        }
        *self.m.at_mut(l) = s;
        if self.ty(i) == Ty::TradeIn {
            self.trade_update()?;
        }
        self.changed(i)
    }

    /// `Slot.setChanged` → the container's `setChanged` (the menus' result hooks).
    fn changed(&mut self, i: usize) -> R<()> {
        match self.ty(i) {
            Ty::CombinerIn(_) => self.combiner_result(),
            Ty::GrindIn => self.grind_result(),
            Ty::CutterIn => self.cutter_changed(),
            Ty::LoomBanner | Ty::LoomDye | Ty::LoomPattern => self.loom_changed(),
            Ty::MapIn | Ty::MapExtra | Ty::MapResult => {
                self.map_changed();
                Ok(())
            }
            Ty::TradeIn | Ty::TradeResult => self.trade_update(),
            _ => Ok(()),
        }
    }

    /// `Slot.remove(n)` → the container's `removeItem`.
    fn remove(&mut self, i: usize, n: i32) -> R<Option<Stack>> {
        match self.ty(i) {
            Ty::Fixed => Ok(None),
            // ResultContainer: the whole stack; MerchantContainer slot 2: the whole stack
            Ty::Result
            | Ty::CombinerResult
            | Ty::GrindResult
            | Ty::CutterResult
            | Ty::MapResult
            | Ty::TradeResult => Ok(self.slot_mut(i).take()),
            t => {
                if n <= 0 {
                    return Ok(None);
                }
                let out = split(self.slot_mut(i), n);
                if out.is_some() {
                    match t {
                        // SimpleContainer.removeItem: setChanged; MerchantContainer: update
                        Ty::Inv | Ty::Armor(_) | Ty::Grid | Ty::CrafterGrid => {}
                        _ => self.changed(i)?,
                    }
                }
                Ok(out)
            }
        }
    }

    /// `Slot.tryRemove(amount, maxAmount)`.
    fn try_remove(&mut self, i: usize, amount: i32, max: i32) -> R<Option<Stack>> {
        if self.ty(i) == Ty::Fixed || !self.may_pickup(i)? {
            return Ok(None);
        }
        let count = self.get(i).map_or(0, |s| s.count);
        if !self.allow_modification(i)? && max < count {
            return Ok(None);
        }
        let out = self.remove(i, amount.min(max))?;
        if out.is_none() {
            return Ok(None);
        }
        if self.get(i).is_none() {
            self.set(i, None)?;
        }
        Ok(out)
    }

    /// `Slot.safeTake`.
    fn safe_take(&mut self, i: usize, amount: i32, max: i32) -> R<Option<Stack>> {
        let out = self.try_remove(i, amount, max)?;
        if let Some(t) = &out {
            let t = t.clone();
            self.on_take(i, Some(&t))?;
        }
        Ok(out)
    }

    /// `Slot.safeInsert(stack, amount)`: puts up to `amount` of `input` into the slot.
    fn safe_insert(&mut self, i: usize, input: &mut Option<Stack>, amount: i32) -> R<()> {
        if self.ty(i) == Ty::Fixed {
            return Ok(());
        }
        let Some(inp) = input.as_ref() else {
            return Ok(());
        };
        if !self.may_place(i, inp)? {
            return Ok(());
        }
        let have = self.get(i).map_or(0, |s| s.count);
        let n = amount.min(inp.count).min(self.max_for(i, inp) - have);
        if n <= 0 {
            return Ok(());
        }
        match self.get(i).cloned() {
            None => {
                let part = split(input, n);
                self.set(i, part)?;
            }
            Some(mut cur) => {
                if same(&cur, inp) {
                    let _ = split(input, n);
                    cur.count += n;
                    self.set(i, Some(cur))?;
                }
            }
        }
        Ok(())
    }

    /// `Slot.onTake`.
    fn on_take(&mut self, i: usize, _taken: Option<&Stack>) -> R<()> {
        match self.ty(i) {
            Ty::Result => self.craft_take(i),
            Ty::CombinerResult => match self.kind {
                Kind::Smithing => {
                    for k in 0..3 {
                        if let Some(mut st) = self.c(k).cloned() {
                            st.count -= 1;
                            self.c_set(k, norm(st))?;
                        }
                    }
                    Ok(())
                }
                _ => Err(Unpredictable),
            },
            Ty::GrindResult => {
                self.c_set(0, None)?;
                self.c_set(1, None)
            }
            Ty::CutterResult => {
                // inputSlot.remove(1) (its slotsChanged resets a changed input); with an item
                // removed `setupResultSlot`: the client has no recipe, so the result is empty
                if self.remove_container(0, 1)?.is_some() {
                    self.c_set_raw(1, None);
                }
                Ok(())
            }
            Ty::MapResult => {
                let _ = self.remove_container(0, 1)?;
                let _ = self.remove_container(1, 1)?;
                self.map_changed();
                Ok(())
            }
            Ty::LoomResult | Ty::TradeResult => Err(Unpredictable),
            _ => {
                // Slot.onTake: setChanged
                self.changed(i)
            }
        }
    }

    /// `setItem(k, s)` on the menu's own container (slot `k` of the container part).
    fn c_set(&mut self, k: usize, s: Option<Stack>) -> R<()> {
        let i = self.screen_of_container(k);
        self.set(i, s)
    }

    fn remove_container(&mut self, k: usize, n: i32) -> R<Option<Stack>> {
        let i = self.screen_of_container(k);
        self.remove(i, n)
    }

    fn screen_of_container(&self, k: usize) -> usize {
        match self.kind {
            Kind::Crafter if k == 9 => 45,
            _ => k,
        }
    }

    // ---------------------------------------------------------- menu result hooks

    /// `menu.slotsChanged(player inventory)` (a bundle click calls it): the menus whose
    /// `slotsChanged` ignores which container changed.
    fn menu_changed(&mut self) -> R<()> {
        match self.kind {
            Kind::Loom => self.loom_changed(),
            Kind::Stonecutter => self.cutter_changed(),
            Kind::Cartography => {
                self.map_changed();
                Ok(())
            }
            Kind::Merchant => self.trade_update(),
            _ => Ok(()),
        }
    }

    /// Anvil `createResult` / smithing `createResult` (the client has no smithing recipes:
    /// always empty).
    fn combiner_result(&mut self) -> R<()> {
        match self.kind {
            Kind::Smithing => {
                self.c_set_raw(3, None);
                Ok(())
            }
            _ => {
                let Some(input) = self.c(0) else {
                    self.c_set_raw(2, None);
                    self.s.anvil_cost = 0;
                    return Ok(());
                };
                // `canStoreEnchantments`: the enchantments component (stored for books)
                let ty = if is_item(input, "enchanted_book") {
                    c::STORED_ENCHANTMENTS
                } else {
                    c::ENCHANTMENTS
                };
                if v::patched(input, ty) == Some(None) {
                    self.c_set_raw(2, None);
                    self.s.anvil_cost = 0;
                    return Ok(());
                }
                Err(Unpredictable)
            }
        }
    }

    /// Grindstone `createResult` (predicted when the result is empty).
    fn grind_result(&mut self) -> R<()> {
        let (a, b) = (self.c(0).cloned(), self.c(1).cloned());
        let empty = match (&a, &b) {
            (None, None) => true,
            _ if a.as_ref().is_some_and(|s| s.count > 1)
                || b.as_ref().is_some_and(|s| s.count > 1) =>
            {
                true
            }
            (Some(x), None) | (None, Some(x)) => !enchanted(x),
            (Some(x), Some(y)) => x.id != y.id,
        };
        if !empty {
            return Err(Unpredictable);
        }
        self.c_set_raw(2, None);
        Ok(())
    }

    /// Stonecutter `slotsChanged`: a new input item resets the recipe list and the result.
    fn cutter_changed(&mut self) -> R<()> {
        let id = self.c(0).map_or(0, |s| s.id);
        if id != self.s.cutter_input {
            self.s.cutter_input = id;
            self.c_set_raw(1, None);
        }
        Ok(())
    }

    /// Loom `slotsChanged`: predicted while the banner or the dye slot is empty.
    fn loom_changed(&mut self) -> R<()> {
        if self.c(0).is_some() && self.c(1).is_some() {
            return Err(Unpredictable);
        }
        self.c_set_raw(3, None);
        Ok(())
    }

    /// Cartography `slotsChanged`: a result without both inputs is removed (the client never
    /// computes one: its container access is empty).
    fn map_changed(&mut self) {
        let both = self.c(0).is_some() && self.c(1).is_some();
        if self.c(2).is_some() && !both {
            self.c_set_raw(2, None);
        }
    }

    /// `MerchantContainer.updateSellItem`: without a payment the result is empty; with one
    /// it comes from the villager's offers.
    fn trade_update(&mut self) -> R<()> {
        let buy = if self.c(0).is_none() {
            self.c(1)
        } else {
            self.c(0)
        };
        if buy.is_some() {
            return Err(Unpredictable);
        }
        self.c_set_raw(2, None);
        Ok(())
    }

    /// Crafting `ResultSlot.onTake`: one of each ingredient of the positioned input is used up
    /// and the items' crafting remainders replace them (the client has no recipes: vanilla's
    /// `defaultCraftingReminder`).
    fn craft_take(&mut self, i: usize) -> R<()> {
        let (grid, width): (Vec<Loc>, usize) = match self.loc(i) {
            Loc::CraftResult => ((0..4).map(Loc::Craft).collect(), 2),
            _ => ((1..10).map(Loc::Container).collect(), 3),
        };
        let filled = |m: &Model, k: usize| m.at(grid[k]).is_some();
        let (mut x0, mut y0, mut x1, mut y1) = (width, width, 0, 0);
        let mut any = false;
        for k in 0..grid.len() {
            if filled(self.m, k) {
                let (x, y) = (k % width, k / width);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
                any = true;
            }
        }
        if !any {
            return Ok(());
        }
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let remainders: Vec<Option<Stack>> = (0..w * h)
            .map(|k| {
                let slot = x0 + k % w + (y0 + k / w) * width;
                self.m
                    .at(grid[slot])
                    .as_ref()
                    .and_then(|s| v::crafting_remainder(s.id))
            })
            .collect();
        for y in 0..h {
            for x in 0..w {
                let slot = x + x0 + (y + y0) * width;
                let l = grid[slot];
                if self.m.at(l).is_some() {
                    let _ = split(self.m.at_mut(l), 1);
                }
                let Some(mut rep) = remainders[x + y * w].clone() else {
                    continue;
                };
                match self.m.at(l).clone() {
                    None => *self.m.at_mut(l) = Some(rep),
                    Some(cur) if same(&cur, &rep) => {
                        rep.count += cur.count;
                        *self.m.at_mut(l) = Some(rep);
                    }
                    Some(_) => {
                        let mut r = Some(rep);
                        if !self.inv_add(&mut r)? {
                            self.drop_stack(r.as_ref());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// `player.drop(stack, …)` on the client: nothing but the arm swing (when not empty).
    fn drop_stack(&mut self, s: Option<&Stack>) {
        if s.is_some() {
            self.swings = self.swings.saturating_add(1);
        }
    }

    // ---------------------------------------------------------- player Inventory.add

    /// `Inventory.add(stack)` (`slot = -1`): what does not fit stays in `st`.
    fn inv_add(&mut self, st: &mut Option<Stack>) -> R<bool> {
        let Some(s) = st.as_ref() else {
            return Ok(false);
        };
        if is_damaged(s) {
            if let Some(k) = (0..36).find(|&k| self.m.player[k].is_none()) {
                self.m.player[k] = st.take();
                return Ok(true);
            }
            if self.w.creative {
                *st = None;
                return Ok(true);
            }
            return Ok(false);
        }
        let mut last;
        loop {
            last = st.as_ref().map_or(0, |s| s.count);
            let left = self.add_resource(st);
            if let Some(s) = st.as_mut() {
                s.count = left;
            }
            if left <= 0 {
                *st = None;
            }
            if !(st.is_some() && left < last) {
                break;
            }
        }
        let now = st.as_ref().map_or(0, |s| s.count);
        if now == last && self.w.creative {
            *st = None;
            return Ok(true);
        }
        Ok(now < last)
    }

    fn has_room(&self, k: usize, s: &Stack) -> bool {
        self.m.player[k]
            .as_ref()
            .is_some_and(|t| same(t, s) && stackable(t) && t.count < 99.min(max_size(t)))
    }

    /// `addResource(stack)`: one round; returns the count left.
    fn add_resource(&mut self, st: &Option<Stack>) -> i32 {
        let Some(s) = st else { return 0 };
        let sel = usize::from(self.m.selected.min(8));
        let slot = if self.has_room(sel, s) {
            Some(sel)
        } else if self.has_room(40, s) {
            Some(40)
        } else {
            (0..36).find(|&k| self.has_room(k, s))
        }
        .or_else(|| (0..36).find(|&k| self.m.player[k].is_none()));
        let Some(k) = slot else { return s.count };
        let count = s.count;
        // an empty slot gets a zero-count copy first, whose components are EMPTY's: max 1
        let (cur, max) = match &self.m.player[k] {
            None => (0, 1),
            Some(t) => (t.count, 99.min(max_size(t))),
        };
        let add = count.min(max - cur);
        if add <= 0 {
            return count;
        }
        match &mut self.m.player[k] {
            Some(t) => t.count += add,
            None => self.m.player[k] = Some(s.with_count(add)),
        }
        count - add
    }

    // ---------------------------------------------------------- doClick

    /// `AbstractContainerMenu.doClick(slot, button, input)`.
    pub(crate) fn click(&mut self, slot: i16, button: i8, mode: u8) -> R<()> {
        let carried_empty = self.m.cursor.is_none();
        if mode == 5 {
            return self.quick_craft(slot, button);
        }
        if self.s.drag.status != 0 {
            self.s.drag.reset();
            return Ok(());
        }
        let idx = usize::try_from(slot).ok().filter(|&i| i < self.len());
        match mode {
            0 | 1 if button == 0 || button == 1 => {
                if slot == OUTSIDE {
                    if let Some(c) = self.m.cursor.clone() {
                        if button == 0 {
                            self.drop_stack(Some(&c));
                            self.m.cursor = None;
                        } else {
                            let one = split(&mut self.m.cursor, 1);
                            self.drop_stack(one.as_ref());
                        }
                    }
                    return Ok(());
                }
                let Some(i) = idx else { return Ok(()) };
                if mode == 1 {
                    if !self.may_pickup(i)? {
                        return Ok(());
                    }
                    let mut clicked = self.quick_move(i)?;
                    let mut guard = 0;
                    while let Some(cl) = &clicked {
                        if self.get(i).is_none_or(|s| s.id != cl.id) {
                            break;
                        }
                        guard += 1;
                        if guard > 4096 {
                            return Err(Unpredictable);
                        }
                        clicked = self.quick_move(i)?;
                    }
                    Ok(())
                } else {
                    self.pickup(i, button == 0)
                }
            }
            2 if (0..9).contains(&button) || button == 40 => {
                let Some(i) = idx else { return Ok(()) };
                self.swap(i, button as usize)
            }
            3 if self.w.creative && carried_empty => {
                let Some(i) = idx else { return Ok(()) };
                if let Some(s) = self.get(i).cloned() {
                    let max = max_size(&s);
                    self.m.cursor = norm(s.with_count(max));
                }
                Ok(())
            }
            4 if carried_empty => {
                let Some(i) = idx else { return Ok(()) };
                if !self.w.can_drop {
                    return Ok(());
                }
                let amount = if button == 0 {
                    1
                } else {
                    self.get(i).map_or(0, |s| s.count)
                };
                let mut st = self.safe_take(i, amount, i32::MAX)?;
                self.drop_stack(st.as_ref());
                if button == 1 {
                    let mut guard = 0;
                    while let Some(t) = &st {
                        if self.get(i).is_none_or(|s| s.id != t.id) {
                            break;
                        }
                        guard += 1;
                        if guard > 4096 {
                            return Err(Unpredictable);
                        }
                        st = self.safe_take(i, amount, i32::MAX)?;
                        self.drop_stack(st.as_ref());
                    }
                }
                Ok(())
            }
            6 => {
                let Some(i) = idx else { return Ok(()) };
                self.pickup_all(i, button)
            }
            _ => Ok(()),
        }
    }

    fn pickup(&mut self, i: usize, primary: bool) -> R<()> {
        if !self.click_override(i, primary)? {
            let clicked = self.get(i).cloned();
            let carried = self.m.cursor.clone();
            match (clicked, carried) {
                (None, None) => {}
                (None, Some(_)) => {
                    let amount = if primary {
                        self.m.cursor.as_ref().map_or(0, |c| c.count)
                    } else {
                        1
                    };
                    let mut cur = self.m.cursor.take();
                    self.safe_insert(i, &mut cur, amount)?;
                    self.m.cursor = cur;
                }
                (Some(cl), carried) => {
                    if self.may_pickup(i)? {
                        match carried {
                            None => {
                                let amount = if primary {
                                    cl.count
                                } else {
                                    (cl.count + 1) / 2
                                };
                                if let Some(t) = self.try_remove(i, amount, i32::MAX)? {
                                    self.m.cursor = Some(t.clone());
                                    self.on_take(i, Some(&t))?;
                                }
                            }
                            Some(ca) => {
                                if self.may_place(i, &ca)? {
                                    if same(&cl, &ca) {
                                        let amount = if primary { ca.count } else { 1 };
                                        let mut cur = self.m.cursor.take();
                                        self.safe_insert(i, &mut cur, amount)?;
                                        self.m.cursor = cur;
                                    } else if ca.count <= self.max_for(i, &ca) {
                                        self.m.cursor = Some(cl);
                                        self.set(i, Some(ca))?;
                                    }
                                } else if same(&cl, &ca) {
                                    let room = max_size(&ca) - ca.count;
                                    if let Some(t) = self.try_remove(i, cl.count, room)? {
                                        if let Some(c) = &mut self.m.cursor {
                                            c.count += t.count;
                                        }
                                        self.on_take(i, Some(&t))?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        self.changed(i)
    }

    /// `tryItemClickBehaviourOverride`: bundles.
    fn click_override(&mut self, i: usize, primary: bool) -> R<bool> {
        // the carried stack stacked on the slot (BundleItem.overrideStackedOnOther)
        if let Some(carried) = self.m.cursor.clone()
            && is_bundle(&carried)
            && let Some(mut b) = Bundle::of(&carried)?
        {
            let other = self.get(i).cloned();
            if primary && other.is_some() {
                let other = other.unwrap_or_else(|| carried.clone());
                if fact(other.id, v::FITS_CONTAINER) {
                    let max = b.max_to_add(weight(&other, 0)?);
                    let mut taken = self.safe_take(i, other.count, max)?;
                    b.try_insert(&mut taken)?;
                }
                if let Some(c) = &mut self.m.cursor {
                    b.store(c);
                }
                self.menu_changed()?;
                return Ok(true);
            }
            if !primary && other.is_none() {
                if let Some(one) = b.remove_one()? {
                    let mut one = Some(one);
                    self.safe_insert(i, &mut one, i32::MAX)?;
                    if one.as_ref().is_some_and(|s| s.count > 0) {
                        b.try_insert(&mut one)?;
                    }
                }
                if let Some(c) = &mut self.m.cursor {
                    b.store(c);
                }
                self.menu_changed()?;
                return Ok(true);
            }
        }
        // a bundle in the slot (BundleItem.overrideOtherStackedOnMe)
        let Some(clicked) = self.get(i).cloned() else {
            return Ok(false);
        };
        if !is_bundle(&clicked) || (primary && self.m.cursor.is_none()) {
            return Ok(false);
        }
        let Some(mut b) = Bundle::of(&clicked)? else {
            return Ok(false);
        };
        if primary && self.m.cursor.is_some() {
            if self.allow_modification(i)? {
                let mut cur = self.m.cursor.take();
                b.try_insert(&mut cur)?;
                self.m.cursor = cur;
            }
            if let Some(s) = self.slot_mut(i) {
                b.store(s);
            }
            self.menu_changed()?;
            return Ok(true);
        }
        if !primary && self.m.cursor.is_none() {
            if self.allow_modification(i)?
                && let Some(one) = b.remove_one()?
            {
                self.m.cursor = Some(one);
            }
            if let Some(s) = self.slot_mut(i) {
                b.store(s);
            }
            self.menu_changed()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn swap(&mut self, i: usize, k: usize) -> R<()> {
        let source = self.m.player[k].clone();
        let target = self.get(i).cloned();
        let alias = self.loc(i) == Loc::Player(k);
        match (source, target) {
            (None, None) => {}
            (None, Some(t)) => {
                if self.may_pickup(i)? {
                    self.m.player[k] = Some(t.clone());
                    self.set(i, None)?;
                    self.on_take(i, Some(&t))?;
                }
            }
            (Some(mut s), None) => {
                if self.may_place(i, &s)? {
                    let max = self.max_for(i, &s);
                    if s.count > max {
                        let part = s.with_count(max);
                        s.count -= max;
                        self.m.player[k] = Some(s);
                        self.set(i, Some(part))?;
                    } else {
                        self.m.player[k] = None;
                        self.set(i, Some(s))?;
                    }
                }
            }
            (Some(mut s), Some(t)) => {
                if self.may_pickup(i)? && self.may_place(i, &s)? {
                    let max = self.max_for(i, &s);
                    if s.count > max {
                        let part = s.with_count(max);
                        s.count -= max;
                        if alias {
                            // source and target are one object: the slot gets the split part,
                            // the rest (the object) goes into the inventory
                            self.set(i, Some(part))?;
                            self.on_take(i, Some(&s))?;
                            let mut rest = Some(s);
                            if !self.inv_add(&mut rest)? {
                                self.drop_stack(rest.as_ref());
                            }
                        } else {
                            self.m.player[k] = Some(s);
                            self.set(i, Some(part))?;
                            self.on_take(i, Some(&t))?;
                            let mut t = Some(t);
                            if !self.inv_add(&mut t)? {
                                self.drop_stack(t.as_ref());
                            }
                        }
                    } else {
                        self.m.player[k] = Some(t.clone());
                        self.set(i, Some(s))?;
                        self.on_take(i, Some(&t))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn pickup_all(&mut self, i: usize, button: i8) -> R<()> {
        if self.m.cursor.is_none() {
            return Ok(());
        }
        if self.get(i).is_some() && self.may_pickup(i)? {
            return Ok(());
        }
        let len = self.len();
        for pass in 0..2 {
            let order: Vec<usize> = if button == 0 {
                (0..len).collect()
            } else {
                (0..len).rev().collect()
            };
            for x in order {
                let Some(c) = self.m.cursor.clone() else {
                    break;
                };
                if c.count >= max_size(&c) {
                    break;
                }
                let Some(t) = self.get(x).cloned() else {
                    continue;
                };
                if !(same(&t, &c) && t.count <= max_size(&c)) {
                    continue;
                }
                if !self.may_pickup(x)? || !self.pick_all_ok(x) {
                    continue;
                }
                if pass != 0 || t.count != max_size(&t) {
                    let room = max_size(&c) - c.count;
                    if let Some(r) = self.safe_take(x, t.count, room)?
                        && let Some(cur) = &mut self.m.cursor
                    {
                        cur.count += r.count;
                    }
                }
            }
        }
        Ok(())
    }

    /// `canTakeItemForPickAll`.
    fn pick_all_ok(&self, x: usize) -> bool {
        match self.kind {
            Kind::Merchant => false,
            Kind::Player | Kind::Crafting => self.ty(x) != Ty::Result,
            Kind::Smithing => self.ty(x) != Ty::CombinerResult,
            Kind::Stonecutter => self.ty(x) != Ty::CutterResult,
            Kind::Cartography => self.ty(x) != Ty::MapResult,
            _ => true,
        }
    }

    // ---------------------------------------------------------- QUICK_CRAFT

    fn quick_craft(&mut self, slot: i16, button: i8) -> R<()> {
        let b = button as u8;
        let expected = self.s.drag.status;
        let status = b & 3;
        self.s.drag.status = status;
        if (expected != 1 || status != 2) && expected != status {
            self.s.drag.reset();
            return Ok(());
        }
        if self.m.cursor.is_none() {
            self.s.drag.reset();
            return Ok(());
        }
        match status {
            0 => {
                let ty = (b >> 2) & 3;
                self.s.drag.ty = ty;
                if ty == 0 || ty == 1 || (ty == 2 && self.w.creative) {
                    self.s.drag.status = 1;
                    self.s.drag.slots.clear();
                } else {
                    self.s.drag.reset();
                }
                Ok(())
            }
            1 => {
                let Some(i) = usize::try_from(slot).ok().filter(|&i| i < self.len()) else {
                    // the real client fails here (index out of range); the GUI never sends it
                    return Err(Unpredictable);
                };
                let Some(c) = self.m.cursor.clone() else {
                    return Ok(());
                };
                let n = self.s.drag.slots.len();
                if self.quick_replace(i, &c)
                    && self.may_place(i, &c)?
                    && (self.s.drag.ty == 2 || c.count > n as i32)
                    && !self.s.drag.slots.contains(&(i as u16))
                {
                    self.s.drag.slots.push(i as u16);
                }
                Ok(())
            }
            2 => {
                let slots = std::mem::take(&mut self.s.drag.slots);
                let ty = self.s.drag.ty;
                if !slots.is_empty() {
                    if slots.len() == 1 {
                        let i = slots[0];
                        self.s.drag.reset();
                        return self.click(i as i16, ty as i8, 0);
                    }
                    let Some(source) = self.m.cursor.clone() else {
                        self.s.drag.reset();
                        return Ok(());
                    };
                    let mut remaining = source.count;
                    for &i in &slots {
                        let i = usize::from(i);
                        let Some(c) = self.m.cursor.clone() else {
                            break;
                        };
                        if self.quick_replace(i, &c)
                            && self.may_place(i, &c)?
                            && (ty == 2 || c.count >= slots.len() as i32)
                        {
                            let carry = self.get(i).map_or(0, |s| s.count);
                            let max = max_size(&source).min(self.max_for(i, &source));
                            let place = match ty {
                                0 => source.count / slots.len() as i32,
                                1 => 1,
                                2 => max_size(&source),
                                _ => source.count,
                            };
                            let new = (place + carry).min(max);
                            remaining -= new - carry;
                            self.set(i, norm(source.with_count(new)))?;
                        }
                    }
                    self.m.cursor = norm(source.with_count(remaining));
                }
                self.s.drag.reset();
                Ok(())
            }
            _ => {
                self.s.drag.reset();
                Ok(())
            }
        }
    }

    /// `canItemQuickReplace(slot, stack, true)`.
    fn quick_replace(&self, i: usize, s: &Stack) -> bool {
        match self.get(i) {
            None => true,
            Some(t) => same(s, t) && t.count <= max_size(s),
        }
    }

    // ---------------------------------------------------------- quickMoveStack

    /// `moveItemStackTo(slot i's own stack, start, end, backwards)`; the source stack is
    /// changed in place (it is the slot's object in vanilla).
    fn move_to(&mut self, src: usize, start: usize, end: usize, back: bool) -> R<bool> {
        let order: Vec<usize> = if back {
            (start..end).rev().collect()
        } else {
            (start..end).collect()
        };
        let mut moved = false;
        let src_loc = self.loc(src);
        let Some(st) = self.m.at(src_loc).clone() else {
            return Ok(false);
        };
        if stackable(&st) {
            for &d in &order {
                let Some(st) = self.m.at(src_loc).clone() else {
                    break;
                };
                let Some(t) = self.get(d).cloned() else {
                    continue;
                };
                if !same(&st, &t) {
                    continue;
                }
                let total = t.count + st.count;
                let max = self.max_for(d, &t);
                if total <= max {
                    *self.m.at_mut(src_loc) = None;
                    if let Some(x) = self.slot_mut(d) {
                        x.count = total;
                    }
                    self.changed(d)?;
                    moved = true;
                } else if t.count < max {
                    if let Some(x) = self.m.at_mut(src_loc) {
                        x.count -= max - t.count;
                    }
                    if let Some(x) = self.slot_mut(d) {
                        x.count = max;
                    }
                    self.changed(d)?;
                    moved = true;
                }
            }
        }
        if let Some(st) = self.m.at(src_loc).clone() {
            for &d in &order {
                if self.get(d).is_some() || !self.may_place(d, &st)? {
                    continue;
                }
                let max = self.max_for(d, &st);
                let part = split(self.m.at_mut(src_loc), st.count.min(max));
                self.set(d, part)?;
                self.changed(d)?;
                moved = true;
                break;
            }
        }
        Ok(moved)
    }

    /// The menu's `quickMoveStack(slot)`: returns the clicked copy (or `None` = EMPTY).
    fn quick_move(&mut self, i: usize) -> R<Option<Stack>> {
        let Some(clicked) = self.get(i).cloned() else {
            return Ok(None);
        };
        let src = self.loc(i);
        let mut drop_rest = false;
        let ok = match self.kind {
            Kind::Player => {
                let (e, _) = equip(&clicked)?;
                if i == 0 {
                    drop_rest = true;
                    self.move_to(i, 9, 45, true)?
                } else if (1..9).contains(&i) {
                    self.move_to(i, 9, 45, false)?
                } else if let Some(a) = e.armor_index()
                    && self.get(5 + usize::from(a)).is_none()
                {
                    let p = 5 + usize::from(a);
                    self.move_to(i, p, p + 1, false)?
                } else if e == Equip::Offhand && self.get(45).is_none() {
                    self.move_to(i, 45, 46, false)?
                } else if (9..36).contains(&i) {
                    self.move_to(i, 36, 45, false)?
                } else if (36..45).contains(&i) {
                    self.move_to(i, 9, 36, false)?
                } else {
                    self.move_to(i, 9, 45, false)?
                }
            }
            Kind::Chest(_) | Kind::Hopper | Kind::Shulker => {
                let n = self.kind.head();
                if i < n {
                    self.move_to(i, n, n + 36, true)?
                } else {
                    self.move_to(i, 0, n, false)?
                }
            }
            Kind::Dispenser => {
                if i < 9 {
                    self.move_to(i, 9, 45, true)?
                } else {
                    self.move_to(i, 0, 9, false)?
                }
            }
            Kind::Crafter => {
                if i < 9 {
                    self.move_to(i, 9, 45, true)?
                } else {
                    self.move_to(i, 0, 9, false)?
                }
            }
            Kind::Crafting => {
                if i == 0 {
                    drop_rest = true;
                    self.move_to(i, 10, 46, true)?
                } else if (10..46).contains(&i) {
                    self.move_to(i, 1, 10, false)?
                        || if i < 37 {
                            self.move_to(i, 37, 46, false)?
                        } else {
                            self.move_to(i, 10, 37, false)?
                        }
                } else {
                    self.move_to(i, 10, 46, false)?
                }
            }
            Kind::Furnace(o) => {
                if i == 2 {
                    self.move_to(i, 3, 39, true)?
                } else if i != 1 && i != 0 {
                    let set = match o {
                        Oven::Furnace => &self.w.furnace_input,
                        Oven::Blast => &self.w.blast_input,
                        Oven::Smoker => &self.w.smoker_input,
                    };
                    if has(set, clicked.id) {
                        self.move_to(i, 0, 1, false)?
                    } else if fact(clicked.id, v::FUEL) {
                        self.move_to(i, 1, 2, false)?
                    } else if (3..30).contains(&i) {
                        self.move_to(i, 30, 39, false)?
                    } else if (30..39).contains(&i) {
                        self.move_to(i, 3, 30, false)?
                    } else {
                        true
                    }
                } else {
                    self.move_to(i, 3, 39, false)?
                }
            }
            Kind::Brewing => {
                if i > 4 {
                    if fact(clicked.id, v::BREWING_FUEL) {
                        if self.move_to(i, 4, 5, false)? {
                            false
                        } else if fact(clicked.id, v::BREWING_INGREDIENT) {
                            // `ingredientSlot.mayPlace(stack) && !moveItemStackTo(3, 4)`: EMPTY
                            self.move_to(i, 3, 4, false)?
                        } else {
                            true
                        }
                    } else if fact(clicked.id, v::BREWING_INGREDIENT) {
                        self.move_to(i, 3, 4, false)?
                    } else if potion_slot(&clicked) {
                        self.move_to(i, 0, 3, false)?
                    } else if (5..32).contains(&i) {
                        self.move_to(i, 32, 41, false)?
                    } else if (32..41).contains(&i) {
                        self.move_to(i, 5, 32, false)?
                    } else {
                        self.move_to(i, 5, 41, false)?
                    }
                } else {
                    self.move_to(i, 5, 41, true)?
                }
            }
            Kind::Enchanting => {
                if i == 0 || i == 1 {
                    self.move_to(i, 2, 38, true)?
                } else if is_item(&clicked, "lapis_lazuli") {
                    self.move_to(i, 1, 2, true)?
                } else {
                    if self.get(0).is_some() || !self.may_place(0, &clicked)? {
                        return Ok(None);
                    }
                    let one = split(self.m.at_mut(src), 1);
                    self.set(0, one)?;
                    true
                }
            }
            Kind::Beacon => {
                if i == 0 {
                    self.move_to(i, 1, 37, true)?
                } else if self.get(0).is_none()
                    && self.may_place(0, &clicked)?
                    && clicked.count == 1
                {
                    self.move_to(i, 0, 1, false)?
                } else if (1..28).contains(&i) {
                    self.move_to(i, 28, 37, false)?
                } else if (28..37).contains(&i) {
                    self.move_to(i, 1, 28, false)?
                } else {
                    self.move_to(i, 1, 37, false)?
                }
            }
            Kind::Anvil | Kind::Smithing => {
                let r = if self.kind == Kind::Anvil { 2 } else { 3 };
                let (inv, inv_end, use_end) = (r + 1, r + 28, r + 37);
                if i == r {
                    self.move_to(i, inv, use_end, true)?
                } else if i < r {
                    self.move_to(i, inv, use_end, false)?
                } else if self.combiner_accepts(&clicked) && (inv..use_end).contains(&i) {
                    self.move_to(i, 0, r, false)?
                } else if (inv..inv_end).contains(&i) {
                    self.move_to(i, inv_end, use_end, false)?
                } else if (inv_end..use_end).contains(&i) {
                    self.move_to(i, inv, inv_end, false)?
                } else {
                    true
                }
            }
            Kind::Grindstone => {
                if i == 2 {
                    self.move_to(i, 3, 39, true)?
                } else if i != 0 && i != 1 {
                    if self.c(0).is_some() && self.c(1).is_some() {
                        if (3..30).contains(&i) {
                            self.move_to(i, 30, 39, false)?
                        } else if (30..39).contains(&i) {
                            self.move_to(i, 3, 30, false)?
                        } else {
                            true
                        }
                    } else {
                        self.move_to(i, 0, 2, false)?
                    }
                } else {
                    self.move_to(i, 3, 39, false)?
                }
            }
            Kind::Stonecutter => {
                if i == 1 {
                    drop_rest = true;
                    self.move_to(i, 2, 38, true)?
                } else if i == 0 {
                    self.move_to(i, 2, 38, false)?
                } else if self.cutter_accepts(clicked.id)? {
                    self.move_to(i, 0, 1, false)?
                } else if (2..29).contains(&i) {
                    self.move_to(i, 29, 38, false)?
                } else if (29..38).contains(&i) {
                    self.move_to(i, 2, 29, false)?
                } else {
                    true
                }
            }
            Kind::Loom => {
                if i == 3 {
                    self.move_to(i, 4, 40, true)?
                } else if i > 3 {
                    if fact(clicked.id, v::BANNER) {
                        self.move_to(i, 0, 1, false)?
                    } else if loom_dye(&clicked) {
                        self.move_to(i, 1, 2, false)?
                    } else if loom_pattern(&clicked) {
                        self.move_to(i, 2, 3, false)?
                    } else if (4..31).contains(&i) {
                        self.move_to(i, 31, 40, false)?
                    } else if (31..40).contains(&i) {
                        self.move_to(i, 4, 31, false)?
                    } else {
                        true
                    }
                } else {
                    self.move_to(i, 4, 40, false)?
                }
            }
            Kind::Cartography => {
                if i == 2 {
                    self.move_to(i, 3, 39, true)?
                } else if i != 1 && i != 0 {
                    if has_component(&clicked, c::MAP_ID, false) {
                        self.move_to(i, 0, 1, false)?
                    } else if !is_item(&clicked, "paper")
                        && !is_item(&clicked, "map")
                        && !is_item(&clicked, "glass_pane")
                    {
                        if (3..30).contains(&i) {
                            self.move_to(i, 30, 39, false)?
                        } else if (30..39).contains(&i) {
                            self.move_to(i, 3, 30, false)?
                        } else {
                            true
                        }
                    } else {
                        self.move_to(i, 1, 2, false)?
                    }
                } else {
                    self.move_to(i, 3, 39, false)?
                }
            }
            Kind::Merchant => {
                if i == 2 {
                    self.move_to(i, 3, 39, true)?
                } else if i != 0 && i != 1 {
                    if (3..30).contains(&i) {
                        self.move_to(i, 30, 39, false)?
                    } else if (30..39).contains(&i) {
                        self.move_to(i, 3, 30, false)?
                    } else {
                        true
                    }
                } else {
                    self.move_to(i, 3, 39, false)?
                }
            }
            Kind::Lectern => return Ok(None),
        };
        if !ok {
            return Ok(None);
        }
        // the slot's stack after the moves (vanilla: the same object, maybe emptied)
        let left = self.m.at(src).clone();
        let left_count = left.as_ref().map_or(0, |s| s.count);
        if left.is_none() {
            self.set(i, None)?;
        } else {
            self.changed(i)?;
        }
        if left_count == clicked.count {
            return Ok(None);
        }
        let arg = if self.kind == Kind::Brewing {
            Some(clicked.clone())
        } else {
            left.clone()
        };
        self.on_take(i, arg.as_ref())?;
        if drop_rest {
            self.drop_stack(left.as_ref());
        }
        Ok(Some(clicked))
    }

    /// `ItemCombinerMenu.canMoveIntoInputSlots` (smithing checks its property sets).
    fn combiner_accepts(&self, s: &Stack) -> bool {
        match self.kind {
            Kind::Smithing => {
                (has(&self.w.smithing_template, s.id) && self.c(0).is_none())
                    || (has(&self.w.smithing_base, s.id) && self.c(1).is_none())
                    || (has(&self.w.smithing_addition, s.id) && self.c(2).is_none())
            }
            _ => true,
        }
    }

    /// `stonecutterRecipes().acceptsInput(stack)`.
    fn cutter_accepts(&self, id: i32) -> R<bool> {
        match &self.w.cutter_inputs {
            Some(set) => Ok(has(set, id)),
            None => Err(Unpredictable),
        }
    }
}

/// A `BundleItem` (only those override clicks; any item's `bundle_contents` counts for weights).
fn is_bundle(s: &Stack) -> bool {
    v::defaults(s.id).is_some_and(|d| d.bundle)
}

fn is_item(s: &Stack, name: &str) -> bool {
    v::defaults(s.id).is_some_and(|d| d.name == name)
}

/// Brewing `PotionSlot.mayPlaceItem`.
fn potion_slot(s: &Stack) -> bool {
    is_item(s, "potion")
        || is_item(s, "splash_potion")
        || is_item(s, "lingering_potion")
        || is_item(s, "glass_bottle")
}

/// Loom `isDyeItem`: the tag and a `dye` component.
fn loom_dye(s: &Stack) -> bool {
    fact(s.id, v::LOOM_DYE_TAG) && has_component(s, c::DYE, fact(s.id, v::HAS_DYE))
}

/// Loom `isPatternItem`: the tag and a `provides_banner_patterns` component.
fn loom_pattern(s: &Stack) -> bool {
    fact(s.id, v::LOOM_PATTERN_TAG)
        && has_component(s, c::PROVIDES_BANNER_PATTERNS, fact(s.id, v::HAS_PATTERNS))
}

/// The order `Int2ObjectOpenHashMap` (fastutil, default capacity) iterates keys inserted in
/// this order: the client's `changedSlots` map is written in it.
pub(crate) fn fastutil_order(keys: &[i32]) -> Vec<i32> {
    fn mix(x: i32) -> u32 {
        let h = (x as u32).wrapping_mul(0x9E37_79B9);
        h ^ (h >> 16)
    }
    fn array_size(expected: usize) -> usize {
        // HashCommon.arraySize(expected, 0.75)
        let need = (expected as f64 / 0.75).ceil() as usize;
        need.next_power_of_two().max(2)
    }
    let mut n = 32usize;
    let mut table: Vec<i32> = vec![0; n];
    let mut has_zero = false;
    let mut size = 0usize;
    let max_fill = |n: usize| ((n as f64 * 0.75).ceil() as usize).min(n - 1);
    let insert = |table: &mut Vec<i32>, n: usize, k: i32| {
        let mask = n - 1;
        let mut pos = (mix(k) as usize) & mask;
        while table[pos] != 0 {
            pos = (pos + 1) & mask;
        }
        table[pos] = k;
    };
    for &k in keys {
        if k == 0 {
            if !has_zero {
                has_zero = true;
                size += 1;
            }
        } else {
            insert(&mut table, n, k);
            size += 1;
        }
        if size > max_fill(n) {
            // rehash(arraySize(size + 1)): old positions in descending order
            let new_n = array_size(size + 1);
            let mut nt = vec![0; new_n];
            for i in (0..n).rev() {
                if table[i] != 0 {
                    insert(&mut nt, new_n, table[i]);
                }
            }
            table = nt;
            n = new_n;
        }
    }
    let mut out = Vec::with_capacity(keys.len());
    if has_zero {
        out.push(0);
    }
    for i in (0..n).rev() {
        if table[i] != 0 {
            out.push(table[i]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions() {
        let w = Frac::new(1, 64).mul(64);
        assert_eq!(w, Frac::ONE);
        assert_eq!(
            Frac::ONE.sub(Frac::new(3, 16)).div_int(Frac::new(1, 16)),
            13
        );
        assert_eq!(Frac::ONE.sub(Frac::new(1, 3)).div_int(Frac::new(1, 64)), 42);
    }

    #[test]
    fn fastutil_small_maps() {
        // a single key, zero first, then table positions from the top
        assert_eq!(fastutil_order(&[5]), vec![5]);
        assert_eq!(fastutil_order(&[0, 3]), vec![0, 3]);
        let keys: Vec<i32> = (0..40).collect();
        let out = fastutil_order(&keys);
        assert_eq!(out.len(), 40);
        assert_eq!(out[0], 0);
    }
}
