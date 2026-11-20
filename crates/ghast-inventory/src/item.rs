//! Version-agnostic item accessors: the facts bots read off a stack (name, stack size,
//! durability, custom name, lore, enchantments, potion), answered from NBT on 1.8 – 1.16.5 and
//! from data components (patch over the item's defaults) on 26.1. Nothing is decoded until an
//! accessor is called; each call reads only what it needs.
//!
//! | accessor | 1.8 / 1.12 | 1.16.5 | 26.1 |
//! |---|---|---|---|
//! | [`Stack::item_name`] | `ghast-data` item table | `ghast-data` | Mojang item registry |
//! | [`Stack::max_stack_size`] | `ghast-data` | `ghast-data` | `max_stack_size` component |
//! | [`Stack::max_damage`] | `ghast-data` durability | `ghast-data` | `max_damage` |
//! | [`Stack::damage_value`] | damage field | `Damage` tag | `damage` |
//! | [`Stack::unbreakable`] | `Unbreakable` tag | `Unbreakable` | `unbreakable` |
//! | [`Stack::custom_name`] | `display.Name` (string) | `display.Name` (JSON) | `custom_name` (NBT text) |
//! | [`Stack::lore`] | `display.Lore` | `display.Lore` (JSON) | `lore` |
//! | [`Stack::enchantments`] | `ench` (numeric ids) | `Enchantments` (names) | `enchantments` (registry ids) |
//! | [`Stack::stored_enchantments`] | `StoredEnchantments` | `StoredEnchantments` | `stored_enchantments` |
//! | [`Stack::potion`] | 1.12 `Potion` tag (1.8: none) | `Potion` | `potion_contents` |

use ghast_protocol::nbt::{Compound, Tag};

use crate::rules::Version;
use crate::stack::{Stack, last, number};

/// A text field of an item (custom name, lore line) in its version's form.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ItemText {
    /// 1.8 / 1.12: a string with `§` formatting codes.
    Legacy(String),
    /// 1.16.5: a JSON text component.
    Json(String),
    /// 26.1: an NBT text component.
    #[cfg(modern)]
    Nbt(ghast_protocol::nbt::AnonNbt),
}

impl ItemText {
    /// The text without formatting. JSON text (1.16.5) is returned as sent; parse it with
    /// `ghast_protocol::chat::to_plain_text` (feature `chat-json`).
    #[must_use]
    pub fn to_plain(&self) -> String {
        match self {
            ItemText::Legacy(s) => ghast_protocol::chat::strip_formatting(s),
            ItemText::Json(s) => s.clone(),
            #[cfg(modern)]
            ItemText::Nbt(n) => ghast_protocol::chat::nbt_to_plain_text(n),
        }
    }
}

/// An enchantment id in its version's form.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EnchantmentId {
    /// 1.8 / 1.12: numeric id.
    Legacy(i16),
    /// 1.16.5: name (`minecraft:sharpness`).
    Name(String),
    /// 26.1: index into the `minecraft:enchantment` registry the server sent during
    /// configuration (enchantments are data-driven).
    Registry(i32),
}

impl EnchantmentId {
    /// The name, given the 26.1 registry names (ignored for the other forms; `None` for a
    /// legacy numeric id).
    #[must_use]
    pub fn name<'a>(&'a self, registry: &'a [String]) -> Option<&'a str> {
        match self {
            EnchantmentId::Legacy(_) => None,
            EnchantmentId::Name(n) => Some(n),
            EnchantmentId::Registry(i) => {
                registry.get(usize::try_from(*i).ok()?).map(String::as_str)
            }
        }
    }
}

/// One enchantment of a stack.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Enchantment {
    /// Which.
    pub id: EnchantmentId,
    /// Level as stored.
    pub level: i32,
}

/// A broad item kind, version-independent ([`Stack::kind`]): what combat and tool logic asks
/// (`#[non_exhaustive]`: kinds may be added).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ItemKind {
    /// A sword.
    Sword,
    /// An axe.
    Axe,
    /// A pickaxe.
    Pickaxe,
    /// A shovel.
    Shovel,
    /// A hoe.
    Hoe,
    /// A bow.
    Bow,
    /// A crossbow (1.14+).
    Crossbow,
    /// A trident (1.13+).
    Trident,
    /// A mace (1.21+).
    Mace,
    /// A shield (1.9+).
    Shield,
    /// A fishing rod.
    FishingRod,
    /// A helmet (turtle shell included).
    Helmet,
    /// A chestplate.
    Chestplate,
    /// Leggings.
    Leggings,
    /// Boots.
    Boots,
    /// Anything else.
    Other,
}

impl ItemKind {
    /// The kind of an item by its registry name (without `minecraft:`), the same rule on every
    /// version (`diamond_sword`, `golden_axe`, `turtle_helmet` …).
    #[must_use]
    pub fn of_name(name: &str) -> ItemKind {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        let exact = match name {
            "bow" => Some(ItemKind::Bow),
            "crossbow" => Some(ItemKind::Crossbow),
            "trident" => Some(ItemKind::Trident),
            "mace" => Some(ItemKind::Mace),
            "shield" => Some(ItemKind::Shield),
            "fishing_rod" => Some(ItemKind::FishingRod),
            "turtle_helmet" => Some(ItemKind::Helmet),
            _ => None,
        };
        if let Some(k) = exact {
            return k;
        }
        // `_pickaxe` before `_axe`.
        for (suffix, k) in [
            ("_sword", ItemKind::Sword),
            ("_pickaxe", ItemKind::Pickaxe),
            ("_axe", ItemKind::Axe),
            ("_shovel", ItemKind::Shovel),
            ("_hoe", ItemKind::Hoe),
            ("_helmet", ItemKind::Helmet),
            ("_chestplate", ItemKind::Chestplate),
            ("_leggings", ItemKind::Leggings),
            ("_boots", ItemKind::Boots),
        ] {
            if name.ends_with(suffix) {
                return k;
            }
        }
        ItemKind::Other
    }
}

/// 1.8 / 1.12 enchantment registry ids by name (`Enchantment.REGISTRY`, from the jars: 9, 10,
/// 22, 70 and 71 exist since 1.9 – 1.11).
pub(crate) const LEGACY_ENCHANTMENTS: &[(i16, &str)] = &[
    (0, "protection"),
    (1, "fire_protection"),
    (2, "feather_falling"),
    (3, "blast_protection"),
    (4, "projectile_protection"),
    (5, "respiration"),
    (6, "aqua_affinity"),
    (7, "thorns"),
    (8, "depth_strider"),
    (9, "frost_walker"),
    (10, "binding_curse"),
    (16, "sharpness"),
    (17, "smite"),
    (18, "bane_of_arthropods"),
    (19, "knockback"),
    (20, "fire_aspect"),
    (21, "looting"),
    (22, "sweeping"),
    (32, "efficiency"),
    (33, "silk_touch"),
    (34, "unbreaking"),
    (35, "fortune"),
    (48, "power"),
    (49, "punch"),
    (50, "flame"),
    (51, "infinity"),
    (61, "luck_of_the_sea"),
    (62, "lure"),
    (70, "mending"),
    (71, "vanishing_curse"),
];

impl EnchantmentId {
    /// The name without `minecraft:` on every version: 1.8 / 1.12 numeric ids through the
    /// vanilla registry, 26.x registry ids through `registry` (the server's enchantment
    /// registry names, [`crate::Inventory::enchantment_name_modern`]'s list).
    #[must_use]
    pub fn plain_name<'a>(&'a self, registry: &'a [String]) -> Option<&'a str> {
        let n = match self {
            EnchantmentId::Legacy(id) => {
                return LEGACY_ENCHANTMENTS
                    .iter()
                    .find(|(i, _)| i == id)
                    .map(|(_, n)| *n);
            }
            other => other.name(registry)?,
        };
        Some(n.strip_prefix("minecraft:").unwrap_or(n))
    }
}

impl Stack {
    /// The broad kind of the item ([`ItemKind::of_name`] of [`Stack::item_name`]).
    #[must_use]
    pub fn kind(&self, v: Version) -> ItemKind {
        self.item_name(v).map_or(ItemKind::Other, ItemKind::of_name)
    }

    /// `true` for a sword.
    #[must_use]
    pub fn is_sword(&self, v: Version) -> bool {
        self.kind(v) == ItemKind::Sword
    }
}

fn root(s: &Stack) -> Option<Compound> {
    match s.nbt()?.parse().ok()??.tag {
        Tag::Compound(c) => Some(c),
        _ => None,
    }
}

fn display(s: &Stack) -> Option<Compound> {
    match last(&root(s)?, "display")? {
        Tag::Compound(c) => Some(c.clone()),
        _ => None,
    }
}

fn legacy_text(v: Version, s: String) -> ItemText {
    if v == Version::V1_16_5 {
        ItemText::Json(s)
    } else {
        ItemText::Legacy(s)
    }
}

fn nbt_enchantments(s: &Stack, v: Version, stored: bool) -> Vec<Enchantment> {
    let key = match (v, stored) {
        (_, true) => "StoredEnchantments",
        (Version::V1_16_5, false) => "Enchantments",
        _ => "ench",
    };
    let Some(r) = root(s) else { return Vec::new() };
    let Some(Tag::List(l)) = last(&r, key) else {
        return Vec::new();
    };
    l.items
        .iter()
        .filter_map(|e| {
            let Tag::Compound(e) = e else { return None };
            let level = last(e, "lvl").and_then(number).unwrap_or(0) as i32;
            let id = match last(e, "id")? {
                Tag::String(n) => EnchantmentId::Name(n.clone()),
                t => EnchantmentId::Legacy(number(t)? as i16),
            };
            Some(Enchantment { id, level })
        })
        .collect()
}

impl Stack {
    /// Item registry name without `minecraft:` (`None` when the id is unknown to the version).
    #[must_use]
    pub fn item_name(&self, v: Version) -> Option<&'static str> {
        let n = v.name(self.id);
        (!n.is_empty()).then_some(n)
    }

    /// Largest stack of this item (`ItemStack.getMaxStackSize`; 26.1 reads the component).
    #[must_use]
    pub fn max_stack_size(&self, v: Version) -> i32 {
        v.max_count(self)
    }

    /// Durability (`None` for items without one).
    #[must_use]
    pub fn max_damage(&self, v: Version) -> Option<i32> {
        match v {
            #[cfg(modern)]
            Version::V26_1 | Version::V26_2 => crate::item26::max_damage(
                crate::tables::Mv::of(v).unwrap_or(crate::tables::Mv::ANY),
                self,
            ),
            _ => v.max_durability(self.id),
        }
    }

    /// Whether the stack loses durability (`isDamageable`: it has durability and is not
    /// unbreakable).
    #[must_use]
    pub fn is_damageable(&self, v: Version) -> bool {
        v.is_damageable(self)
    }

    /// Damage taken (0 when not damageable; 1.8 / 1.12 items with variants use the damage field
    /// as the variant, not as damage).
    #[must_use]
    pub fn damage_value(&self, v: Version) -> i32 {
        match v {
            Version::V1_8 | Version::V1_12 => {
                if v.has_subtypes(self.id) {
                    0
                } else {
                    i32::from(self.damage)
                }
            }
            Version::V1_16_5 => self.nbt_int("Damage"),
            #[cfg(modern)]
            Version::V26_1 | Version::V26_2 => crate::item26::damage_value(
                crate::tables::Mv::of(v).unwrap_or(crate::tables::Mv::ANY),
                self,
            ),
            #[allow(unreachable_patterns)]
            _ => 0,
        }
    }

    /// `Unbreakable` (1.8 – 1.16.5 tag, 26.1 component, including the item default).
    #[must_use]
    pub fn unbreakable(&self, v: Version) -> bool {
        match v {
            #[cfg(modern)]
            Version::V26_1 | Version::V26_2 => crate::item26::unbreakable(
                crate::tables::Mv::of(v).unwrap_or(crate::tables::Mv::ANY),
                self,
            ),
            _ => self.nbt_bool("Unbreakable"),
        }
    }

    /// The custom name (anvil / plugin name), if any.
    #[must_use]
    pub fn custom_name(&self, v: Version) -> Option<ItemText> {
        match v {
            #[cfg(modern)]
            Version::V26_1 | Version::V26_2 => crate::item26::text(
                crate::tables::Mv::of(v).unwrap_or(crate::tables::Mv::ANY),
                self,
                crate::tables::kind::CUSTOM_NAME,
            )
            .map(ItemText::Nbt),
            _ => match last(&display(self)?, "Name")? {
                Tag::String(n) => Some(legacy_text(v, n.clone())),
                _ => None,
            },
        }
    }

    /// The lore lines (empty when none).
    #[must_use]
    pub fn lore(&self, v: Version) -> Vec<ItemText> {
        match v {
            #[cfg(modern)]
            Version::V26_1 | Version::V26_2 => crate::item26::lore(
                crate::tables::Mv::of(v).unwrap_or(crate::tables::Mv::ANY),
                self,
            )
            .into_iter()
            .map(ItemText::Nbt)
            .collect(),
            _ => {
                let Some(d) = display(self) else {
                    return Vec::new();
                };
                let Some(Tag::List(l)) = last(&d, "Lore") else {
                    return Vec::new();
                };
                l.items
                    .iter()
                    .filter_map(|t| match t {
                        Tag::String(s) => Some(legacy_text(v, s.clone())),
                        _ => None,
                    })
                    .collect()
            }
        }
    }

    /// The enchantments (empty when none).
    #[must_use]
    pub fn enchantments(&self, v: Version) -> Vec<Enchantment> {
        self.enchants(v, false)
    }

    /// The stored enchantments of an enchanted book (empty when none).
    #[must_use]
    pub fn stored_enchantments(&self, v: Version) -> Vec<Enchantment> {
        self.enchants(v, true)
    }

    fn enchants(&self, v: Version, stored: bool) -> Vec<Enchantment> {
        match v {
            #[cfg(modern)]
            Version::V26_1 | Version::V26_2 => crate::item26::enchantments(
                crate::tables::Mv::of(v).unwrap_or(crate::tables::Mv::ANY),
                self,
                stored,
            )
            .unwrap_or_default()
            .into_iter()
            .map(|(id, level)| Enchantment {
                id: EnchantmentId::Registry(id),
                level,
            })
            .collect(),
            _ => nbt_enchantments(self, v, stored),
        }
    }

    /// The potion type (`minecraft:healing` on 1.12 / 1.16.5; `healing` on 26.1), when the
    /// stack names one (1.8 encodes potions in the damage value: `None`).
    #[must_use]
    pub fn potion(&self, v: Version) -> Option<String> {
        match v {
            #[cfg(modern)]
            Version::V26_1 | Version::V26_2 => crate::item26::potion(
                crate::tables::Mv::of(v).unwrap_or(crate::tables::Mv::ANY),
                self,
            )
            .map(str::to_string),
            Version::V1_12 | Version::V1_16_5 => match last(&root(self)?, "Potion")? {
                Tag::String(p) => Some(p.clone()),
                _ => None,
            },
            _ => None,
        }
    }
}
