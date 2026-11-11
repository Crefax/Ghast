//! 26.1 (protocol 775) items: default components of every item (Mojang's `--reports` component
//! dump, see `examples/gen_items_26_1.rs`), readers for the components the click rules and the
//! item accessors use, the window types, and the hashed stacks a 26.1 client sends with every
//! click (vanilla `HashedStack` / `HashedPatchMap` over `HashOps.CRC32C_INSTANCE`).

use bytes::Bytes;
use ghast_protocol::codec::Reader;
use ghast_protocol::nbt::{AnonNbt, Tag};

use crate::stack::{ItemData, Stack};
use crate::window::WindowKind;

#[path = "v26_1_hash.rs"]
mod hash;
pub use hash::{HashRegistries, component_hash, hash_covers};

mod generated {
    use super::{Equip, ItemDefaults};
    include!("generated/items_26_1.rs");
}

pub(crate) use generated::ENTITY_TYPES;
pub use generated::{
    ATTRIBUTES, COMPONENT_NAMES, ITEM_COUNT, MENUS, POTIONS, SOUND_EVENTS, VILLAGER_TYPES,
    component,
};

mod menus {
    include!("generated/menus_26_1.rs");
}

pub(crate) use menus::{
    BANNER, BEACON_PAYMENT, BREWING_FUEL, BREWING_INGREDIENT, FITS_CONTAINER, FUEL, HAS_DYE,
    HAS_PATTERNS, LOOM_DYE_TAG, LOOM_PATTERN_TAG,
};

/// The menu slot facts of item `id` (bits of the `menus` table; 0 for unknown ids).
pub(crate) fn menu_facts(id: i32) -> u16 {
    usize::try_from(id)
        .ok()
        .and_then(|i| menus::MENU_FACTS.get(i))
        .copied()
        .unwrap_or(0)
}

/// `Item.getCraftingRemainder()` as a stack.
pub(crate) fn crafting_remainder(id: i32) -> Option<Stack> {
    menus::REMAINDERS
        .binary_search_by_key(&id, |r| r.0)
        .ok()
        .map(|i| Stack::new(menus::REMAINDERS[i].1, menus::REMAINDERS[i].2, 0))
}

/// The equipment slot of an item's `equippable` component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Equip {
    /// Not equippable.
    None,
    /// Head.
    Head,
    /// Chest.
    Chest,
    /// Legs.
    Legs,
    /// Feet.
    Feet,
    /// Offhand (the shield).
    Offhand,
    /// Main hand.
    MainHand,
    /// Animal body armor.
    Body,
    /// Saddle.
    Saddle,
}

impl Equip {
    /// The player armor slot index (0 head … 3 feet) of a humanoid armor slot.
    #[must_use]
    pub fn armor_index(self) -> Option<u8> {
        match self {
            Equip::Head => Some(0),
            Equip::Chest => Some(1),
            Equip::Legs => Some(2),
            Equip::Feet => Some(3),
            _ => None,
        }
    }
}

/// The default components of one item that ghast reads (from Mojang's report).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItemDefaults {
    /// Registry name without `minecraft:`.
    pub name: &'static str,
    /// `max_stack_size`.
    pub max_stack_size: u8,
    /// `max_damage` (0 = absent).
    pub max_damage: u16,
    /// Has a default `damage` component.
    pub damage: bool,
    /// Has a default `unbreakable` component.
    pub unbreakable: bool,
    /// `equippable.slot`.
    pub equip: Equip,
    /// `equippable.allowed_entities` is set (not wearable by players).
    pub equip_restricted: bool,
    /// Has `bundle_contents` (a bundle: clicks with it run the bundle's own rules).
    pub bundle: bool,
    /// `rarity` (0 common, 1 uncommon, 2 rare, 3 epic).
    pub rarity: u8,
    /// `use_effects`: can sprint, interact vibrations, speed multiplier (`f32` bits).
    pub(crate) use_effects: (bool, bool, u32),
    /// `consumable`: consume seconds (`f32` bits), animation id.
    pub(crate) consumable: Option<(u32, u8)>,
    /// `food`: nutrition, saturation (`f32` bits), can always eat.
    pub(crate) food: Option<(i32, u32, bool)>,
    /// Has `blocks_attacks` (a shield).
    pub(crate) blocks_attacks: bool,
    /// Has `kinetic_weapon` (a spear).
    pub(crate) kinetic_weapon: bool,
}

/// Default components of item `id`.
#[must_use]
pub fn defaults(id: i32) -> Option<&'static ItemDefaults> {
    usize::try_from(id)
        .ok()
        .and_then(|i| generated::ITEMS.get(i))
}

/// Item registry id of a name (`"diamond_sword"` or `"minecraft:diamond_sword"`).
#[must_use]
pub fn item_id(name: &str) -> Option<i32> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    generated::ITEMS
        .iter()
        .position(|d| d.name == name)
        .map(|i| i as i32)
}

/// Component type id of a name (`"damage"` or `"minecraft:damage"`).
#[must_use]
pub fn component_id(name: &str) -> Option<i32> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    COMPONENT_NAMES
        .iter()
        .position(|n| *n == name)
        .map(|i| i as i32)
}

/// The window type of a 26.1 `open_screen` (`minecraft:menu` registry id).
#[must_use]
pub fn window_kind(menu: i32) -> Option<WindowKind> {
    let name = MENUS.get(usize::try_from(menu).ok()?)?;
    Some(match *name {
        "generic_9x1" => WindowKind::Chest { rows: 1 },
        "generic_9x2" => WindowKind::Chest { rows: 2 },
        "generic_9x3" => WindowKind::Chest { rows: 3 },
        "generic_9x4" => WindowKind::Chest { rows: 4 },
        "generic_9x5" => WindowKind::Chest { rows: 5 },
        "generic_9x6" => WindowKind::Chest { rows: 6 },
        "generic_3x3" => WindowKind::Generic3x3,
        "anvil" => WindowKind::Anvil,
        "beacon" => WindowKind::Beacon,
        "blast_furnace" | "furnace" | "smoker" => WindowKind::Furnace,
        "brewing_stand" => WindowKind::BrewingStand,
        "crafting" => WindowKind::Crafting,
        "enchantment" => WindowKind::Enchanting,
        "grindstone" => WindowKind::Grindstone,
        "hopper" => WindowKind::Hopper,
        "lectern" => WindowKind::Lectern,
        "loom" => WindowKind::Loom,
        "merchant" => WindowKind::Merchant,
        "shulker_box" => WindowKind::ShulkerBox,
        "smithing" => WindowKind::Smithing,
        "cartography_table" => WindowKind::Cartography,
        "stonecutter" => WindowKind::Stonecutter,
        "crafter_3x3" => WindowKind::Crafter,
        _ => return None,
    })
}

// ---------------------------------------------------------------------- component values

#[cfg(test)]
fn patch(s: &Stack) -> Option<&ghast_protocol::types::slot::v775::ComponentPatch> {
    s.components()
}

/// The patch's verdict on component `ty`: `Some(Some(bytes))` set, `Some(None)` removed, `None`
/// not in the patch (the item's default applies).
pub(crate) fn patched(s: &Stack, ty: i32) -> Option<Option<Bytes>> {
    let ItemData::Components(p) = &s.data else {
        return None;
    };
    if let Some(b) = p.get(ty) {
        return Some(Some(b));
    }
    p.removes(ty).then_some(None)
}

fn varint(b: &Bytes) -> Option<i32> {
    Reader::new(b).read_varint().ok()
}

fn int_component(s: &Stack, ty: i32, default: Option<i32>) -> Option<i32> {
    match patched(s, ty) {
        Some(Some(b)) => varint(&b),
        Some(None) => None,
        None => default,
    }
}

fn has_component(s: &Stack, ty: i32, default: bool) -> bool {
    match patched(s, ty) {
        Some(v) => v.is_some(),
        None => default,
    }
}

/// `ItemStack.getMaxStackSize()`: the `max_stack_size` component (default 1 when absent).
pub(crate) fn max_stack_size(s: &Stack) -> i32 {
    let d = defaults(s.id).map(|d| i32::from(d.max_stack_size));
    int_component(s, component::MAX_STACK_SIZE, d).unwrap_or(1)
}

/// `max_damage` (`None` when absent).
pub(crate) fn max_damage(s: &Stack) -> Option<i32> {
    let d = defaults(s.id).and_then(|d| (d.max_damage > 0).then_some(i32::from(d.max_damage)));
    int_component(s, component::MAX_DAMAGE, d)
}

/// `ItemStack.isDamageableItem()`: `max_damage` and `damage` present, `unbreakable` absent.
pub(crate) fn is_damageable(s: &Stack) -> bool {
    let d = defaults(s.id);
    max_damage(s).is_some()
        && !unbreakable(s)
        && has_component(s, component::DAMAGE, d.is_some_and(|d| d.damage))
}

/// `ItemStack.getDamageValue()`: `damage` clamped to `0..=max_damage`.
pub(crate) fn damage_value(s: &Stack) -> i32 {
    let d = defaults(s.id).and_then(|d| d.damage.then_some(0));
    let v = int_component(s, component::DAMAGE, d).unwrap_or(0);
    v.clamp(0, max_damage(s).unwrap_or(0))
}

/// `unbreakable` present.
pub(crate) fn unbreakable(s: &Stack) -> bool {
    has_component(
        s,
        component::UNBREAKABLE,
        defaults(s.id).is_some_and(|d| d.unbreakable),
    )
}

/// The equipment slot of `equippable`: the item default, or the patched value's slot
/// (`Err` when a patch sets `allowed_entities`: whether a player may wear it is not modelled).
pub(crate) fn equip(s: &Stack) -> Result<Equip, ()> {
    match patched(s, component::EQUIPPABLE) {
        Some(Some(b)) => patched_equip(&b).ok_or(()),
        Some(None) => Ok(Equip::None),
        None => Ok(defaults(s.id).map_or(Equip::None, |d| {
            if d.equip_restricted {
                Equip::None
            } else {
                d.equip
            }
        })),
    }
}

/// The slot of a patched `equippable` (`Equippable.STREAM_CODEC`: slot, equip sound, asset id,
/// camera overlay, allowed entities, ...); `None` when `allowed_entities` is set or the bytes
/// do not read.
fn patched_equip(b: &Bytes) -> Option<Equip> {
    let mut r = Reader::new(b);
    let slot = r.read_varint().ok()?;
    // equip_sound: a registry id + 1, or 0 and an inline `SoundEvent` (id, optional range)
    if r.read_varint().ok()? == 0 {
        r.read_string().ok()?;
        if r.read_bool().ok()? {
            r.read_f32().ok()?;
        }
    }
    for _ in 0..2 {
        // asset_id, camera_overlay
        if r.read_bool().ok()? {
            r.read_string().ok()?;
        }
    }
    if r.read_bool().ok()? {
        return None;
    }
    Some(match slot {
        0 => Equip::MainHand,
        1 => Equip::Feet,
        2 => Equip::Legs,
        3 => Equip::Chest,
        4 => Equip::Head,
        5 => Equip::Offhand,
        6 => Equip::Body,
        7 => Equip::Saddle,
        _ => return None,
    })
}

/// Whether the stack has the `bundle_contents` component (a bundle: some clicks with it run
/// the bundle's own rules, `BundleItem.overrideStackedOnOther` / `overrideOtherStackedOnMe`).
pub(crate) fn overrides_clicks(s: &Stack) -> bool {
    has_component(
        s,
        component::BUNDLE_CONTENTS,
        defaults(s.id).is_some_and(|d| d.bundle),
    )
}

/// `(enchantment registry id, level)` of the `enchantments` (`stored = false`) or
/// `stored_enchantments` component, in wire order. `None` when the component is absent.
pub(crate) fn enchantments(s: &Stack, stored: bool) -> Option<Vec<(i32, i32)>> {
    let ty = if stored {
        component::STORED_ENCHANTMENTS
    } else {
        component::ENCHANTMENTS
    };
    let b = patched(s, ty)??;
    let mut r = Reader::new(&b);
    let n = r.read_varint().ok()?;
    let n = usize::try_from(n).ok()?;
    let mut v = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        v.push((r.read_varint().ok()?, r.read_varint().ok()?));
    }
    Some(v)
}

/// A text component (`custom_name`, `item_name`) as NBT.
pub(crate) fn text(s: &Stack, ty: i32) -> Option<AnonNbt> {
    let b = patched(s, ty)??;
    AnonNbt::read(&mut Reader::new(&b)).ok()
}

/// The `lore` lines as NBT text components.
pub(crate) fn lore(s: &Stack) -> Vec<AnonNbt> {
    let Some(Some(b)) = patched(s, component::LORE) else {
        return Vec::new();
    };
    let mut r = Reader::new(&b);
    let Ok(n) = r.read_varint() else {
        return Vec::new();
    };
    (0..n.max(0))
        .map_while(|_| AnonNbt::read(&mut r).ok())
        .collect()
}

/// The `potion_contents` potion (registry name), if it names one.
pub(crate) fn potion(s: &Stack) -> Option<&'static str> {
    let b = patched(s, component::POTION_CONTENTS)??;
    let mut r = Reader::new(&b);
    if !r.read_bool().ok()? {
        return None;
    }
    let id = r.read_varint().ok()?;
    POTIONS.get(usize::try_from(id).ok()?).copied()
}

// ---------------------------------------------------------------------- item use

/// How an item in use animates (`ItemUseAnimation`, by its network id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UseAnimation {
    /// `none`.
    None,
    /// `eat`.
    Eat,
    /// `drink`.
    Drink,
    /// `block` (shields: `blocks_attacks`).
    Block,
    /// `bow`.
    Bow,
    /// `trident`.
    Trident,
    /// `crossbow`.
    Crossbow,
    /// `spyglass`.
    Spyglass,
    /// `toot_horn` (goat horns).
    TootHorn,
    /// `brush`.
    Brush,
    /// `bundle`.
    Bundle,
    /// `spear` (`kinetic_weapon`).
    Spear,
}

impl UseAnimation {
    fn from_id(id: i32) -> Option<UseAnimation> {
        Some(match id {
            0 => UseAnimation::None,
            1 => UseAnimation::Eat,
            2 => UseAnimation::Drink,
            3 => UseAnimation::Block,
            4 => UseAnimation::Bow,
            5 => UseAnimation::Trident,
            6 => UseAnimation::Crossbow,
            7 => UseAnimation::Spyglass,
            8 => UseAnimation::TootHorn,
            9 => UseAnimation::Brush,
            10 => UseAnimation::Bundle,
            11 => UseAnimation::Spear,
            _ => return None,
        })
    }
}

/// `food` component: what eating the item restores.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Food {
    /// `nutrition`.
    pub nutrition: i32,
    /// `saturation`.
    pub saturation: f32,
    /// `can_always_eat` (eaten with a full hunger bar).
    pub can_always_eat: bool,
}

/// What using (right-click holding) a 26.1 item does, from its components (the patch over the
/// item's defaults) and its item class, as `Item.getUseAnimation` / `getUseDuration` and
/// `LocalPlayer`'s movement read them.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ItemUse {
    /// `getUseAnimation`.
    pub animation: UseAnimation,
    /// `getUseDuration` in ticks: 0 = the use does not last (no use state), 72000 = until
    /// released (bows, shields, spears, tridents, crossbow charging); `None` when it depends on
    /// data ghast does not have (a goat horn's instrument).
    pub duration: Option<i32>,
    /// `use_effects.speed_multiplier`: movement input factor while using (vanilla default
    /// 0.2; the physics input of `ghast_physics::modern`).
    pub speed_multiplier: f32,
    /// `use_effects.can_sprint`: sprinting continues while using.
    pub can_sprint: bool,
    /// `use_effects.interact_vibrations`.
    pub interact_vibrations: bool,
    /// The stack has a `consumable` component (eaten / drunk over `duration` ticks).
    pub consumable: bool,
    /// The `food` component.
    pub food: Option<Food>,
}

/// [`ItemUse`] of a 26.1 stack (`None` for an id outside the item registry). Components the
/// stack patches but that do not read give the item defaults' values.
#[must_use]
pub fn item_use(s: &Stack) -> Option<ItemUse> {
    let d = defaults(s.id)?;
    let (mut can_sprint, mut vibrations, mut speed) = (
        d.use_effects.0,
        d.use_effects.1,
        f32::from_bits(d.use_effects.2),
    );
    match patched(s, component::USE_EFFECTS) {
        Some(Some(b)) => {
            let mut r = Reader::new(&b);
            if let (Ok(a), Ok(v), Ok(f)) = (r.read_bool(), r.read_bool(), r.read_f32()) {
                (can_sprint, vibrations, speed) = (a, v, f);
            }
        }
        // removed: `getOrDefault(USE_EFFECTS, DEFAULT)`
        Some(None) => (can_sprint, vibrations, speed) = (false, true, 0.2),
        None => {}
    }
    let consumable = match patched(s, component::CONSUMABLE) {
        Some(Some(b)) => {
            let mut r = Reader::new(&b);
            match (r.read_f32(), r.read_varint()) {
                (Ok(secs), Ok(anim)) => Some((secs, UseAnimation::from_id(anim))),
                _ => d
                    .consumable
                    .map(|(b, a)| (f32::from_bits(b), UseAnimation::from_id(i32::from(a)))),
            }
        }
        Some(None) => None,
        None => d
            .consumable
            .map(|(b, a)| (f32::from_bits(b), UseAnimation::from_id(i32::from(a)))),
    };
    let food = match patched(s, component::FOOD) {
        Some(Some(b)) => {
            let mut r = Reader::new(&b);
            match (r.read_varint(), r.read_f32(), r.read_bool()) {
                (Ok(n), Ok(sat), Ok(a)) => Some(Food {
                    nutrition: n,
                    saturation: sat,
                    can_always_eat: a,
                }),
                _ => None,
            }
        }
        Some(None) => None,
        None => d.food.map(|(n, sat, a)| Food {
            nutrition: n,
            saturation: f32::from_bits(sat),
            can_always_eat: a,
        }),
    };
    let blocks = has_component(s, component::BLOCKS_ATTACKS, d.blocks_attacks);
    let kinetic = has_component(s, component::KINETIC_WEAPON, d.kinetic_weapon);
    // Item.getUseAnimation / getUseDuration, then the item classes that override them
    let (mut animation, mut duration) = match consumable {
        Some((secs, anim)) => (
            anim.unwrap_or(UseAnimation::Eat),
            Some((secs * 20.0) as i32),
        ),
        None if blocks => (UseAnimation::Block, Some(72000)),
        None if kinetic => (UseAnimation::Spear, Some(72000)),
        None => (UseAnimation::None, Some(0)),
    };
    match d.name {
        "bow" => (animation, duration) = (UseAnimation::Bow, Some(72000)),
        "crossbow" => (animation, duration) = (UseAnimation::Crossbow, Some(72000)),
        "trident" => (animation, duration) = (UseAnimation::Trident, Some(72000)),
        "spyglass" => (animation, duration) = (UseAnimation::Spyglass, Some(1200)),
        "brush" => (animation, duration) = (UseAnimation::Brush, Some(200)),
        "goat_horn" => (animation, duration) = (UseAnimation::TootHorn, None),
        "ender_eye" => duration = Some(0),
        _ if d.bundle => (animation, duration) = (UseAnimation::Bundle, Some(200)),
        _ => {}
    }
    Some(ItemUse {
        animation,
        duration,
        speed_multiplier: speed,
        can_sprint,
        interact_vibrations: vibrations,
        consumable: consumable.is_some(),
        food,
    })
}

// ---------------------------------------------------------------------- hashed stacks

/// Hash put into a click for a component ghast cannot hash: certainly not the real one, so the
/// server answers with the slot's true contents (the click itself is applied either way).
pub const UNKNOWN_HASH: i32 = 0;

/// Guava `Hashing.crc32c()`.
struct Crc32c(u32);

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0x82F6_3B78
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC_TABLE: [u32; 256] = crc_table();

impl Crc32c {
    fn new() -> Self {
        Crc32c(!0)
    }
    fn put(&mut self, b: &[u8]) -> &mut Self {
        let mut c = self.0;
        for &x in b {
            c = CRC_TABLE[((c ^ u32::from(x)) & 0xFF) as usize] ^ (c >> 8);
        }
        self.0 = c;
        self
    }
    fn byte(&mut self, b: u8) -> &mut Self {
        self.put(&[b])
    }
    fn finish(&self) -> u32 {
        !self.0
    }
}

fn hash_of(bytes: &[u8]) -> u32 {
    Crc32c::new().put(bytes).finish()
}

/// `HashOps` values (Guava hashers write little-endian).
mod ops {
    use super::{Crc32c, hash_of};

    pub fn int(v: i32) -> u32 {
        Crc32c::new().byte(8).put(&v.to_le_bytes()).finish()
    }
    pub fn byte(v: i8) -> u32 {
        Crc32c::new().byte(6).byte(v as u8).finish()
    }
    pub fn short(v: i16) -> u32 {
        Crc32c::new().byte(7).put(&v.to_le_bytes()).finish()
    }
    pub fn long(v: i64) -> u32 {
        Crc32c::new().byte(9).put(&v.to_le_bytes()).finish()
    }
    pub fn float(v: f32) -> u32 {
        Crc32c::new()
            .byte(10)
            .put(&v.to_bits().to_le_bytes())
            .finish()
    }
    pub fn double(v: f64) -> u32 {
        Crc32c::new()
            .byte(11)
            .put(&v.to_bits().to_le_bytes())
            .finish()
    }
    pub fn string(s: &str) -> u32 {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut h = Crc32c::new();
        h.byte(12).put(&(units.len() as i32).to_le_bytes());
        for u in units {
            h.put(&u.to_le_bytes());
        }
        h.finish()
    }
    pub fn boolean(b: bool) -> u32 {
        hash_of(&[13, u8::from(b)])
    }
    pub fn empty_map() -> u32 {
        hash_of(&[2, 3])
    }
    /// Entries sorted by key hash, then value hash (unsigned), as `HashOps.hashMap` does.
    pub fn map(mut entries: Vec<(u32, u32)>) -> u32 {
        entries.sort_unstable();
        let mut h = Crc32c::new();
        h.byte(2);
        for (k, v) in entries {
            h.put(&k.to_le_bytes()).put(&v.to_le_bytes());
        }
        h.byte(3).finish()
    }
    pub fn list(items: impl IntoIterator<Item = u32>) -> u32 {
        let mut h = Crc32c::new();
        h.byte(4);
        for v in items {
            h.put(&v.to_le_bytes());
        }
        h.byte(5).finish()
    }
    pub fn byte_list(b: &[u8]) -> u32 {
        Crc32c::new().byte(14).put(b).byte(15).finish()
    }
    pub fn int_list(v: &[i32]) -> u32 {
        let mut h = Crc32c::new();
        h.byte(16);
        for x in v {
            h.put(&x.to_le_bytes());
        }
        h.byte(17).finish()
    }
    pub fn long_list(v: &[i64]) -> u32 {
        let mut h = Crc32c::new();
        h.byte(18);
        for x in v {
            h.put(&x.to_le_bytes());
        }
        h.byte(19).finish()
    }
}

/// NBT converted into `HashOps` (vanilla `NbtOps.convertTo`); `text = true` reads the byte
/// fields a text component's codec writes as booleans (`bold`, `italic`, …) as booleans, which
/// is what encoding the component itself to `HashOps` produces.
fn nbt_hash(t: &Tag, text: bool) -> u32 {
    const BOOL_KEYS: &[&str] = &[
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
        "interpret",
    ];
    match t {
        Tag::Byte(v) => ops::byte(*v),
        Tag::Short(v) => ops::short(*v),
        Tag::Int(v) => ops::int(*v),
        Tag::Long(v) => ops::long(*v),
        Tag::Float(v) => ops::float(*v),
        Tag::Double(v) => ops::double(*v),
        Tag::String(s) => ops::string(s),
        Tag::ByteArray(b) => ops::byte_list(b),
        Tag::IntArray(v) => ops::int_list(v),
        Tag::LongArray(v) => ops::long_list(v),
        Tag::List(l) => ops::list(l.items.iter().map(|e| match e {
            // ListTag unwraps the `{"": value}` wrappers of mixed lists when it is read.
            Tag::Compound(c) if c.0.len() == 1 && c.0[0].0.is_empty() => nbt_hash(&c.0[0].1, text),
            e => nbt_hash(e, text),
        })),
        Tag::Compound(c) => {
            let mut seen: Vec<&str> = Vec::new();
            let mut entries = Vec::with_capacity(c.0.len());
            // Java maps keep the last duplicate.
            for (k, v) in c.0.iter().rev() {
                if seen.contains(&k.as_str()) {
                    continue;
                }
                seen.push(k);
                let vh = match v {
                    Tag::Byte(b)
                        if text && BOOL_KEYS.contains(&k.as_str()) && (*b == 0 || *b == 1) =>
                    {
                        ops::boolean(*b != 0)
                    }
                    v => nbt_hash(v, text),
                };
                entries.push((ops::string(k), vh));
            }
            ops::map(entries)
        }
    }
}

/// `TypedEntityData.codec`: the compound without its `id`, then `id` = the type's name.
fn typed_nbt_hash(n: &AnonNbt, type_name: &str) -> Option<u32> {
    let Tag::Compound(mut c) = n.parse().ok()?? else {
        return None;
    };
    c.0.retain(|(k, _)| k != "id");
    c.0.push(("id".into(), Tag::String(type_name.into())));
    Some(nbt_hash(&Tag::Compound(c), false))
}

fn anon_hash(n: &AnonNbt, text: bool) -> Option<u32> {
    if n.is_end() {
        return None;
    }
    Some(nbt_hash(&n.parse().ok()??, text))
}

/// One slot as a 26.1 click sends it (vanilla `HashedStack.create`), with
/// [`UNKNOWN_HASH`] for components ghast cannot hash. The second value is `false` when such a
/// component was present.
///
/// Every component kind a client can receive is hashed (checked against the real client's
/// hashes of 4000 stacks in `tests/golden/26.1/hashed_stacks.jsonl.gz`). What still falls back
/// to [`UNKNOWN_HASH`]: a registry name the server did not send in configuration, an inline
/// holder value without a modelled direct form (trim materials/patterns, banner patterns,
/// instruments, jukebox songs, painting variants; inline sound events are modelled), and an
/// adventure-mode block predicate with an `nbt` test or partial component predicates.
#[must_use]
pub fn hashed_slot(
    s: Option<&Stack>,
    reg: &HashRegistries,
) -> (Option<ghast_protocol::defs::p775::types::HashedSlot>, bool) {
    use ghast_protocol::defs::p775::types::{
        HashedSlot, HashedSlotComponents, HashedSlotRemoveComponents, SlotComponentType,
    };
    let Some(s) = s.filter(|s| s.count > 0 && s.id != 0) else {
        return (None, true);
    };
    let mut exact = true;
    let mut components = Vec::new();
    let mut removed = Vec::new();
    if let ItemData::Components(p) = &s.data {
        // DataComponentPatch is a map: the last value of a type wins.
        let mut seen: Vec<i32> = Vec::new();
        let all: Vec<(i32, Bytes)> = p.iter().collect();
        for (t, b) in all.iter().rev() {
            if seen.contains(t) {
                continue;
            }
            seen.push(*t);
            let h = component_hash(*t, b, reg).unwrap_or_else(|| {
                exact = false;
                UNKNOWN_HASH
            });
            components.push(HashedSlotComponents {
                r#type: SlotComponentType(*t),
                hash: h,
            });
        }
        for t in p.removed() {
            if !removed
                .iter()
                .any(|r: &HashedSlotRemoveComponents| r.r#type.0 == t)
            {
                removed.push(HashedSlotRemoveComponents {
                    r#type: SlotComponentType(t),
                });
            }
        }
        // a map and a set in vanilla: written here in type id order
        components.sort_by_key(|c| c.r#type.0);
        removed.sort_by_key(|c| c.r#type.0);
    }
    (
        Some(HashedSlot {
            item_id: s.id,
            item_count: s.count,
            components,
            remove_components: removed,
        }),
        exact,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghast_protocol::types::slot::v775::ComponentPatch;

    /// Known answers computed with the 26.1 jar's `HashOps.CRC32C_INSTANCE` (Guava crc32c):
    /// `createInt(5)`, `createString("minecraft:sharpness")`, `emptyMap()`,
    /// `createBoolean(true)`, `createMap({"a": 1})`, `createList([int 1, int 2])`.
    #[test]
    fn hash_ops_known_answers() {
        // crc32c("123456789") = 0xE3069283 (the standard check value).
        assert_eq!(hash_of(b"123456789"), 0xE306_9283);
        for (got, want) in KNOWN.iter() {
            assert_eq!(got(), *want);
        }
    }

    type Known = (fn() -> u32, u32);
    const KNOWN: &[Known] = &[
        (|| ops::int(5), 645_064_431),
        (
            || ops::string("minecraft:sharpness"),
            -1_560_913_390i32 as u32,
        ),
        (ops::empty_map, -982_207_288i32 as u32),
        (|| ops::boolean(true), -1_019_818_302i32 as u32),
        (
            || ops::map(vec![(ops::string("a"), ops::int(1))]),
            -108_872_706i32 as u32,
        ),
        (
            || ops::list([ops::int(1), ops::int(2)]),
            -1_373_960_668i32 as u32,
        ),
        (|| ops::byte(-3), -722_991_802i32 as u32),
        (|| ops::double(1.5), -1_387_852_210i32 as u32),
        (
            || {
                ops::map(vec![
                    (ops::string("minecraft:sharpness"), ops::int(5)),
                    (ops::string("minecraft:unbreaking"), ops::int(3)),
                ])
            },
            1_231_404_551,
        ),
    ];

    fn unhex(s: &str) -> Bytes {
        Bytes::from(
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect::<Vec<u8>>(),
        )
    }

    /// Text components: the jar's `ComponentSerialization.CODEC` encoded once to `HashOps`
    /// (the hash) and once to `NbtOps` (the bytes a server sends); ghast hashes the NBT.
    #[test]
    fn text_hashes_match_the_codec() {
        let cases: &[(&str, i32, &str)] = &[
            ("plain", 815_466_656, "08000548656c6c6f"),
            (
                "bold",
                -608_560_159,
                "0a080005636f6c6f720004676f6c6408000474657874000453686f70010004626f6c640100",
            ),
            (
                "notitalic",
                185_662_785,
                "0a080005636f6c6f720004677261790800047465787400044c696e650100066974616c69630000",
            ),
            (
                "mixed",
                -198_755_598,
                "0a09000565787472610a0000000208000000016200080005636f6c6f72000372656408000474657874000163000800047465787400016100",
            ),
            (
                "translate",
                1_855_830_573,
                "0a0800097472616e736c617465001c6974656d2e6d696e6563726166742e6469616d6f6e645f73776f726400",
            ),
            ("unicode", -45_199_928, "08000ac39cc3a9eda0bdedb880"),
        ];
        for (label, hash, hex) in cases {
            let nbt = AnonNbt::from_bytes(unhex(hex)).unwrap();
            assert_eq!(
                anon_hash(&nbt, true).map(|h| h as i32),
                Some(*hash),
                "{label}"
            );
            // the component value on the wire is the NBT itself
            let v = nbt.as_bytes().clone();
            let reg = HashRegistries::default();
            assert_eq!(
                component_hash(component::CUSTOM_NAME, &v, &reg),
                Some(*hash),
                "{label}"
            );
        }
    }

    #[test]
    fn defaults_table() {
        assert_eq!(ITEM_COUNT, 1506);
        let sword = item_id("diamond_sword").unwrap();
        let st = Stack::new(sword, 1, 0);
        assert_eq!(max_stack_size(&st), 1);
        assert_eq!(max_damage(&st), Some(1561));
        assert!(is_damageable(&st));
        assert_eq!(
            equip(&Stack::new(item_id("elytra").unwrap(), 1, 0)),
            Ok(Equip::Chest)
        );
        assert_eq!(
            equip(&Stack::new(item_id("shield").unwrap(), 1, 0)),
            Ok(Equip::Offhand)
        );
        assert_eq!(
            equip(&Stack::new(item_id("diamond_horse_armor").unwrap(), 1, 0)),
            Ok(Equip::None)
        );
        assert!(overrides_clicks(&Stack::new(
            item_id("red_bundle").unwrap(),
            1,
            0
        )));
        assert_eq!(
            max_stack_size(&Stack::new(item_id("ender_pearl").unwrap(), 1, 0)),
            16
        );
        assert_eq!(component_id("damage"), Some(component::DAMAGE));
        assert_eq!(window_kind(2), Some(WindowKind::Chest { rows: 3 }));
        assert_eq!(window_kind(7), Some(WindowKind::Crafter));
        assert_eq!(window_kind(20), Some(WindowKind::ShulkerBox));
    }

    #[test]
    fn patched_components() {
        let sword = item_id("diamond_sword").unwrap();
        let mut st = Stack::new(sword, 1, 0);
        // damage 37, max_stack_size 4 (wire: varints), unbreakable removed? no: added
        let p = ComponentPatch::from_parts(
            &[
                (component::DAMAGE, &[37]),
                (component::MAX_STACK_SIZE, &[4]),
            ],
            &[component::MAX_DAMAGE],
        );
        st.data = ItemData::Components(Box::new(p));
        assert_eq!(patch(&st).unwrap().added_len(), 2);
        assert_eq!(max_stack_size(&st), 4);
        assert_eq!(max_damage(&st), None);
        assert!(!is_damageable(&st));
        assert_eq!(damage_value(&st), 0); // clamped to max_damage 0
        let (h, exact) = hashed_slot(Some(&st), &HashRegistries::default());
        let h = h.unwrap();
        assert!(exact);
        assert_eq!(h.components.len(), 2);
        assert_eq!(h.remove_components.len(), 1);
        assert_eq!(
            h.components
                .iter()
                .find(|c| c.r#type.0 == component::DAMAGE)
                .unwrap()
                .hash,
            ops::int(37) as i32
        );
    }
}
