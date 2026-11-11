//! Data-driven 26.1 component hashes: one schema per component type says how the value is
//! read from the wire (the component's stream codec) and what its persistent codec writes to
//! `HashOps` (vanilla `TypedDataComponent.encodeValue(HashOps)` hashes the codec's output, not
//! the wire bytes). A small interpreter does the rest, so most components are covered by a
//! table row instead of code. Rows were transcribed from the 26.1 `DataComponents` registrations
//! and the codecs they name; record fields declared `optionalFieldOf(name, default)` are omitted
//! when they equal the default, as DFU does.
//!
//! The function is pure: `(type, value bytes, registry names) -> hash`, so oracle vectors (a real
//! client hashing stacks) can be replayed against it directly.

use std::collections::BTreeMap;

use bytes::Bytes;
use ghast_protocol::codec::Reader;
use ghast_protocol::nbt::AnonNbt;

use super::{COMPONENT_NAMES, POTIONS, anon_hash, component as c, generated, ops};

/// Names of the registries a click hash needs: the configuration phase's `registry_data` keys
/// in order (data-driven registries such as enchantments, damage types, entity variants).
///
/// Feed it every configuration `registry_data` packet with [`HashRegistries::add`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HashRegistries {
    /// `minecraft:enchantment` entry names by id (`"minecraft:sharpness"`).
    pub(crate) enchantments: Vec<String>,
    /// Every other registry received, by registry id (`"minecraft:damage_type"`).
    others: BTreeMap<String, Vec<String>>,
}

impl HashRegistries {
    /// `minecraft:enchantment` entry names by registry id.
    #[must_use]
    pub fn enchantments(&self) -> &[String] {
        &self.enchantments
    }

    /// Records one `registry_data` packet (`registry` id and entry keys in order).
    pub fn add<'a>(&mut self, registry: &str, keys: impl IntoIterator<Item = &'a str>) {
        let keys: Vec<String> = keys.into_iter().map(str::to_string).collect();
        if registry == "minecraft:enchantment" {
            self.enchantments = keys;
        } else {
            self.others.insert(registry.to_string(), keys);
        }
    }

    /// The entry names of a data-driven registry, if received.
    #[must_use]
    pub fn names(&self, registry: &str) -> Option<&[String]> {
        if registry == "minecraft:enchantment" {
            return Some(&self.enchantments);
        }
        self.others.get(registry).map(Vec::as_slice)
    }
}

/// Where a registry id's name comes from.
#[derive(Clone, Copy)]
enum Reg {
    /// The item registry (generated from Mojang's report).
    Item,
    /// The data component type registry.
    ComponentType,
    /// The potion registry.
    Potion,
    /// The attribute registry.
    Attribute,
    /// The sound event registry.
    SoundEvent,
    /// The villager type registry.
    VillagerType,
    /// A data-driven registry the server sent in configuration.
    Data(&'static str),
}

/// A value's wire form and codec form.
#[derive(Clone, Copy)]
enum S {
    /// VarInt -> int.
    VarInt,
    /// Big-endian i32 -> int.
    Int,
    /// f64 -> double.
    Double,
    /// f32 -> float.
    Float,
    /// bool -> boolean.
    Bool,
    /// UTF-8 string -> string.
    Str,
    /// Identifier string -> string (namespace kept as sent).
    Ident,
    /// Nothing -> empty map (`Unit.CODEC`).
    Unit,
    /// Anonymous NBT -> converted NBT.
    Nbt,
    /// Anonymous NBT text component -> the text codec's output.
    Text,
    /// VarInt id -> enum name (`StringRepresentable`; `(id, name)` pairs, sparse allowed).
    Enum(&'static [(i32, &'static str)]),
    /// VarInt registry id -> identifier.
    Reg(Reg),
    /// `ByteBufCodecs.holder`: VarInt id + 1 -> identifier (0 = an inline value, not modelled).
    Holder(Reg),
    /// `ItemAttributeModifiers.Display` (`optionalFieldOf("display", Default)`): type 0 omitted,
    /// 1 `{type: "hidden"}`, 2 `{type: "override", value: text}`.
    AttrDisplay,
    /// `ByteBufCodecs.holderSet`: VarInt 0 + tag name -> `"#tag"`; n + 1 ids -> one identifier
    /// or a list (`compactListCodec`).
    HolderSet(Reg),
    /// VarInt count + elements -> list.
    List(&'static S),
    /// VarInt count + key / value pairs -> map.
    Map(&'static S, &'static S),
    /// Fields in wire order -> map of the present fields.
    Rec(&'static [Field]),
    /// A value ghast does not model: hashing fails when one is present.
    Never,
}

/// A record field: name, schema, and how the codec treats it.
#[derive(Clone, Copy)]
struct Field(&'static str, S, Def);

/// Field presence in the codec's output.
#[derive(Clone, Copy)]
enum Def {
    /// Always written (`fieldOf`).
    Required,
    /// Wire `Optional` (bool + value); written only when present (`optionalFieldOf(name)`).
    Optional,
    /// Written unless equal to this default (`optionalFieldOf(name, default)`).
    Default(Value),
}

/// A decoded value (codec form).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Value {
    Int(i32),
    Float(f32),
    Bool(bool),
    /// An empty list (the only list default the rows use).
    EmptyList,
    /// A string default (enum names).
    Name(&'static str),
}

/// A decoded value ready to hash.
#[derive(Debug, PartialEq)]
enum V {
    /// A record field the codec leaves out.
    Omit,
    Double(f64),
    Int(i32),
    Float(f32),
    Bool(bool),
    Str(String),
    Hash(u32),
    List(Vec<V>),
    Map(Vec<(V, V)>),
}

impl V {
    fn hash(&self) -> u32 {
        match self {
            V::Omit => ops::empty_map(),
            V::Double(d) => ops::double(*d),
            V::Int(i) => ops::int(*i),
            V::Float(f) => ops::float(*f),
            V::Bool(b) => ops::boolean(*b),
            V::Str(s) => ops::string(s),
            V::Hash(h) => *h,
            V::List(l) => ops::list(l.iter().map(V::hash)),
            V::Map(m) => ops::map(m.iter().map(|(k, v)| (k.hash(), v.hash())).collect()),
        }
    }

    fn is(&self, d: &Value) -> bool {
        match (self, d) {
            (V::Int(a), Value::Int(b)) => a == b,
            // Java Float.equals: bit equality
            (V::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            (V::Bool(a), Value::Bool(b)) => a == b,
            (V::List(l), Value::EmptyList) => l.is_empty(),
            (V::Str(s), Value::Name(n)) => s == n,
            _ => false,
        }
    }
}

const DYE: &[(i32, &str)] = &[
    (0, "white"),
    (1, "orange"),
    (2, "magenta"),
    (3, "light_blue"),
    (4, "yellow"),
    (5, "lime"),
    (6, "pink"),
    (7, "gray"),
    (8, "light_gray"),
    (9, "cyan"),
    (10, "purple"),
    (11, "blue"),
    (12, "brown"),
    (13, "green"),
    (14, "red"),
    (15, "black"),
];
const OPERATION: &[(i32, &str)] = &[
    (0, "add_value"),
    (1, "add_multiplied_base"),
    (2, "add_multiplied_total"),
];
const SLOT_GROUP: &[(i32, &str)] = &[
    (0, "any"),
    (1, "mainhand"),
    (2, "offhand"),
    (3, "hand"),
    (4, "feet"),
    (5, "legs"),
    (6, "chest"),
    (7, "head"),
    (8, "armor"),
    (9, "body"),
    (10, "saddle"),
];
const RARITY: &[(i32, &str)] = &[(0, "common"), (1, "uncommon"), (2, "rare"), (3, "epic")];
const SWING: &[(i32, &str)] = &[(0, "none"), (1, "whack"), (2, "stab")];
const FOX: &[(i32, &str)] = &[(0, "red"), (1, "snow")];
const SALMON: &[(i32, &str)] = &[(0, "small"), (1, "medium"), (2, "large")];
const PARROT: &[(i32, &str)] = &[
    (0, "red_blue"),
    (1, "blue"),
    (2, "green"),
    (3, "yellow_blue"),
    (4, "gray"),
];
/// `TropicalFish.Pattern` packed ids (`base | index << 8`).
const FISH_PATTERN: &[(i32, &str)] = &[
    (0, "kob"),
    (256, "sunstreak"),
    (512, "snooper"),
    (768, "dasher"),
    (1024, "brinely"),
    (1280, "spotty"),
    (1, "flopper"),
    (257, "stripey"),
    (513, "glitter"),
    (769, "blockfish"),
    (1025, "betty"),
    (1281, "clayfish"),
];
const MOOSHROOM: &[(i32, &str)] = &[(0, "red"), (1, "brown")];
const RABBIT: &[(i32, &str)] = &[
    (0, "brown"),
    (1, "white"),
    (2, "black"),
    (3, "white_splotched"),
    (4, "gold"),
    (5, "salt"),
    (99, "evil"),
];
const HORSE: &[(i32, &str)] = &[
    (0, "white"),
    (1, "creamy"),
    (2, "chestnut"),
    (3, "brown"),
    (4, "black"),
    (5, "gray"),
    (6, "dark_brown"),
];
const LLAMA: &[(i32, &str)] = &[(0, "creamy"), (1, "white"), (2, "brown"), (3, "gray")];
const AXOLOTL: &[(i32, &str)] = &[
    (0, "lucy"),
    (1, "wild"),
    (2, "gold"),
    (3, "cyan"),
    (4, "blue"),
];

/// The schema of a component type (`None`: not covered).
fn schema(ty: i32) -> Option<S> {
    use Def::{Default as D, Optional as O, Required as R};
    Some(match ty {
        c::CUSTOM_DATA | c::BUCKET_ENTITY_DATA => S::Nbt,
        // Not network-synchronised types travel as the NBT of their persistent codec
        // (`ByteBufCodecs.fromCodecWithRegistries`), so converting that NBT is their hash
        // (exact unless the codec writes booleans, which NBT turns into bytes).
        c::RECIPES
        | c::LOCK
        | c::CONTAINER_LOOT
        | c::DEBUG_STICK_STATE
        | c::MAP_DECORATIONS
        | c::INTANGIBLE_PROJECTILE => S::Nbt,
        c::MAX_STACK_SIZE | c::MAX_DAMAGE | c::DAMAGE | c::REPAIR_COST | c::MAP_ID => S::VarInt,
        c::OMINOUS_BOTTLE_AMPLIFIER => S::VarInt,
        c::UNBREAKABLE | c::GLIDER => S::Unit,
        c::USE_EFFECTS => S::Rec(&[
            Field("can_sprint", S::Bool, D(Value::Bool(false))),
            Field("interact_vibrations", S::Bool, D(Value::Bool(true))),
            Field("speed_multiplier", S::Float, D(Value::Float(0.2))),
        ]),
        c::CUSTOM_NAME | c::ITEM_NAME => S::Text,
        c::MINIMUM_ATTACK_CHARGE | c::POTION_DURATION_SCALE => S::Float,
        c::ITEM_MODEL | c::TOOLTIP_STYLE | c::NOTE_BLOCK_SOUND => S::Ident,
        c::LORE => S::List(&S::Text),
        c::RARITY => S::Enum(RARITY),
        c::ENCHANTMENTS | c::STORED_ENCHANTMENTS => {
            S::Map(&S::Reg(Reg::Data("minecraft:enchantment")), &S::VarInt)
        }
        c::CUSTOM_MODEL_DATA => S::Rec(&[
            Field("floats", S::List(&S::Float), D(Value::EmptyList)),
            Field("flags", S::List(&S::Bool), D(Value::EmptyList)),
            Field("strings", S::List(&S::Str), D(Value::EmptyList)),
            Field("colors", S::List(&S::Int), D(Value::EmptyList)),
        ]),
        c::TOOLTIP_DISPLAY => S::Rec(&[
            Field("hide_tooltip", S::Bool, D(Value::Bool(false))),
            Field(
                "hidden_components",
                S::List(&S::Reg(Reg::ComponentType)),
                D(Value::EmptyList),
            ),
        ]),
        c::ENCHANTMENT_GLINT_OVERRIDE => S::Bool,
        c::USE_COOLDOWN => S::Rec(&[
            Field("seconds", S::Float, R),
            Field("cooldown_group", S::Ident, O),
        ]),
        c::DAMAGE_RESISTANT => S::Rec(&[Field(
            "types",
            S::HolderSet(Reg::Data("minecraft:damage_type")),
            R,
        )]),
        c::WEAPON => S::Rec(&[
            Field("item_damage_per_attack", S::VarInt, D(Value::Int(1))),
            Field(
                "disable_blocking_for_seconds",
                S::Float,
                D(Value::Float(0.0)),
            ),
        ]),
        c::ATTACK_RANGE => S::Rec(&[
            Field("min_reach", S::Float, D(Value::Float(0.0))),
            Field("max_reach", S::Float, D(Value::Float(3.0))),
            Field("min_creative_reach", S::Float, D(Value::Float(0.0))),
            Field("max_creative_reach", S::Float, D(Value::Float(5.0))),
            Field("hitbox_margin", S::Float, D(Value::Float(0.3))),
            Field("mob_factor", S::Float, D(Value::Float(1.0))),
        ]),
        c::ENCHANTABLE => S::Rec(&[Field("value", S::VarInt, R)]),
        c::REPAIRABLE => S::Rec(&[Field("items", S::HolderSet(Reg::Item), R)]),
        c::SWING_ANIMATION => S::Rec(&[
            Field("type", S::Enum(SWING), D(Value::Name("whack"))),
            Field("duration", S::VarInt, D(Value::Int(6))),
        ]),
        c::DYE
        | c::BASE_COLOR
        | c::WOLF_COLLAR
        | c::CAT_COLLAR
        | c::SHEEP_COLOR
        | c::SHULKER_COLOR
        | c::TROPICAL_FISH_BASE_COLOR
        | c::TROPICAL_FISH_PATTERN_COLOR => S::Enum(DYE),
        c::DYED_COLOR | c::MAP_COLOR => S::Int,
        c::PROVIDES_BANNER_PATTERNS => S::HolderSet(Reg::Data("minecraft:banner_pattern")),
        c::FOX_VARIANT => S::Enum(FOX),
        c::SALMON_SIZE => S::Enum(SALMON),
        c::PARROT_VARIANT => S::Enum(PARROT),
        c::TROPICAL_FISH_PATTERN => S::Enum(FISH_PATTERN),
        c::MOOSHROOM_VARIANT => S::Enum(MOOSHROOM),
        c::RABBIT_VARIANT => S::Enum(RABBIT),
        c::HORSE_VARIANT => S::Enum(HORSE),
        c::LLAMA_VARIANT => S::Enum(LLAMA),
        c::AXOLOTL_VARIANT => S::Enum(AXOLOTL),
        c::WOLF_VARIANT => S::Reg(Reg::Data("minecraft:wolf_variant")),
        c::WOLF_SOUND_VARIANT => S::Reg(Reg::Data("minecraft:wolf_sound_variant")),
        c::PIG_VARIANT => S::Reg(Reg::Data("minecraft:pig_variant")),
        c::PIG_SOUND_VARIANT => S::Reg(Reg::Data("minecraft:pig_sound_variant")),
        c::COW_VARIANT => S::Reg(Reg::Data("minecraft:cow_variant")),
        c::COW_SOUND_VARIANT => S::Reg(Reg::Data("minecraft:cow_sound_variant")),
        c::CHICKEN_VARIANT => S::Reg(Reg::Data("minecraft:chicken_variant")),
        c::CHICKEN_SOUND_VARIANT => S::Reg(Reg::Data("minecraft:chicken_sound_variant")),
        c::FROG_VARIANT => S::Reg(Reg::Data("minecraft:frog_variant")),
        c::CAT_VARIANT => S::Reg(Reg::Data("minecraft:cat_variant")),
        c::CAT_SOUND_VARIANT => S::Reg(Reg::Data("minecraft:cat_sound_variant")),
        c::ZOMBIE_NAUTILUS_VARIANT => S::Reg(Reg::Data("minecraft:zombie_nautilus_variant")),
        c::DAMAGE_TYPE => S::Reg(Reg::Data("minecraft:damage_type")),
        c::VILLAGER_VARIANT => S::Reg(Reg::VillagerType),
        c::PAINTING_VARIANT => S::Holder(Reg::Data("minecraft:painting_variant")),
        c::PROVIDES_TRIM_MATERIAL => S::Holder(Reg::Data("minecraft:trim_material")),
        c::JUKEBOX_PLAYABLE => S::Holder(Reg::Data("minecraft:jukebox_song")),
        c::INSTRUMENT => S::Holder(Reg::Data("minecraft:instrument")),
        c::BREAK_SOUND => S::Holder(Reg::SoundEvent),
        c::FOOD => S::Rec(&[
            Field("nutrition", S::VarInt, R),
            Field("saturation", S::Float, R),
            Field("can_always_eat", S::Bool, D(Value::Bool(false))),
        ]),
        c::ATTRIBUTE_MODIFIERS => S::List(&S::Rec(&[
            Field("type", S::Reg(Reg::Attribute), R),
            Field("id", S::Ident, R),
            Field("amount", S::Double, R),
            Field("operation", S::Enum(OPERATION), R),
            Field("slot", S::Enum(SLOT_GROUP), D(Value::Name("any"))),
            Field("display", S::AttrDisplay, R),
        ])),
        c::POTION_CONTENTS => S::Rec(&[
            Field("potion", S::Reg(Reg::Potion), O),
            Field("custom_color", S::Int, O),
            // non-empty custom effects make the row fail (MobEffectInstance is not modelled)
            Field("custom_effects", S::List(&S::Never), D(Value::EmptyList)),
            Field("custom_name", S::Str, O),
        ]),
        _ => return None,
    })
}

fn name_of(reg: Reg, id: i32, regs: &HashRegistries) -> Option<&str> {
    let i = usize::try_from(id).ok()?;
    match reg {
        Reg::Item => generated::ITEMS.get(i).map(|d| d.name),
        Reg::ComponentType => COMPONENT_NAMES.get(i).copied(),
        Reg::Potion => POTIONS.get(i).copied(),
        Reg::Attribute => super::ATTRIBUTES.get(i).copied(),
        Reg::SoundEvent => super::SOUND_EVENTS.get(i).copied(),
        Reg::VillagerType => super::VILLAGER_TYPES.get(i).copied(),
        Reg::Data(r) => regs.names(r)?.get(i).map(String::as_str),
    }
}

/// Static registries' names come without a namespace from the generated tables.
fn ident(reg: Reg, name: &str) -> String {
    match reg {
        Reg::Data(_) => name.to_string(),
        _ => format!("minecraft:{name}"),
    }
}

fn read(s: &S, r: &mut Reader<'_>, regs: &HashRegistries) -> Option<V> {
    Some(match *s {
        S::VarInt => V::Int(r.read_varint().ok()?),
        S::Int => V::Int(r.read_i32().ok()?),
        S::Double => V::Double(r.read_f64().ok()?),
        S::Holder(reg) => {
            let id = r.read_varint().ok()?;
            if id == 0 {
                return None;
            }
            V::Str(ident(reg, name_of(reg, id - 1, regs)?))
        }
        S::AttrDisplay => match r.read_varint().ok()? {
            0 => V::Omit,
            1 => V::Map(vec![(V::Str("type".into()), V::Str("hidden".into()))]),
            2 => V::Map(vec![
                (V::Str("type".into()), V::Str("override".into())),
                (
                    V::Str("value".into()),
                    V::Hash(anon_hash(&AnonNbt::read(r).ok()?, true)?),
                ),
            ]),
            _ => return None,
        },
        S::Float => V::Float(r.read_f32().ok()?),
        S::Bool => V::Bool(r.read_bool().ok()?),
        S::Str | S::Ident => V::Str(r.read_string().ok()?.to_string()),
        S::Unit => V::Map(Vec::new()),
        S::Nbt => V::Hash(anon_hash(&AnonNbt::read(r).ok()?, false)?),
        S::Text => V::Hash(anon_hash(&AnonNbt::read(r).ok()?, true)?),
        S::Enum(names) => {
            let id = r.read_varint().ok()?;
            V::Str(names.iter().find(|(i, _)| *i == id)?.1.to_string())
        }
        S::Reg(reg) => V::Str(ident(reg, name_of(reg, r.read_varint().ok()?, regs)?)),
        S::HolderSet(reg) => {
            let n = r.read_varint().ok()?;
            if n == 0 {
                let tag = r.read_string().ok()?;
                V::Str(format!("#{tag}"))
            } else {
                let mut v = Vec::new();
                for _ in 0..n - 1 {
                    v.push(V::Str(ident(
                        reg,
                        name_of(reg, r.read_varint().ok()?, regs)?,
                    )));
                }
                if v.len() == 1 { v.pop()? } else { V::List(v) }
            }
        }
        S::List(e) => {
            let n = r.read_varint().ok()?;
            let mut v = Vec::new();
            for _ in 0..n.max(0) {
                v.push(read(e, r, regs)?);
            }
            V::List(v)
        }
        S::Map(k, val) => {
            let n = r.read_varint().ok()?;
            let mut v = Vec::new();
            for _ in 0..n.max(0) {
                v.push((read(k, r, regs)?, read(val, r, regs)?));
            }
            V::Map(v)
        }
        S::Never => return None,
        S::Rec(fields) => {
            let mut out = Vec::new();
            for Field(name, fs, def) in fields {
                let v = match def {
                    Def::Optional => {
                        if !r.read_bool().ok()? {
                            continue;
                        }
                        read(fs, r, regs)?
                    }
                    _ => read(fs, r, regs)?,
                };
                if v == V::Omit {
                    continue;
                }
                if let Def::Default(d) = def
                    && v.is(d)
                {
                    continue;
                }
                out.push((V::Str((*name).to_string()), v));
            }
            V::Map(out)
        }
    })
}

/// The `HashOps` hash of one component value as it appears on the wire (`None`: the type is
/// not covered, or a registry name it needs is unknown).
#[must_use]
pub fn component_hash(ty: i32, value: &Bytes, regs: &HashRegistries) -> Option<i32> {
    let s = schema(ty)?;
    let mut r = Reader::new(value);
    let v = read(&s, &mut r, regs)?;
    // the whole value must have been understood
    if r.remaining() != 0 {
        return None;
    }
    Some(v.hash() as i32)
}

/// Whether ghast can hash component `ty` at all (registry names permitting).
#[must_use]
pub fn hash_covers(ty: i32) -> bool {
    schema(ty).is_some()
}
