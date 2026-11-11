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

use ghast_protocol::types::slot::v775::ComponentPatch;

use super::{COMPONENT_NAMES, POTIONS, anon_hash, component as c, generated, ops, typed_nbt_hash};

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
    /// The mob effect registry.
    MobEffect,
    /// The entity type registry.
    EntityType,
    /// The block entity type registry.
    BlockEntityType,
    /// The block registry (from ghast-data).
    Block,
    /// A data-driven registry the server sent in configuration.
    Data(&'static str),
}

/// A value's wire form and codec form.
#[derive(Clone, Copy)]
enum S {
    /// VarInt -> int.
    VarInt,
    /// VarInt -> byte (`ExtraCodecs.UNSIGNED_BYTE`).
    UByte,
    /// Packed i64 (`BlockPos.STREAM_CODEC`) -> int stream `[x, y, z]`.
    BlockPos,
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
    /// `ByteBufCodecs.holder` whose inline (id 0) value has this schema (the direct codec).
    HolderOr(Reg, &'static S),
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
    /// Another schema (for recursive rows).
    Ref(&'static S),
    /// `ItemStackTemplate`: item id, count, component patch -> `{id, count (unless 1),
    /// components (unless empty)}`; the patch's values hash through their own rows.
    Template,
    /// `ItemContainerContents`: VarInt count + optional templates -> list of the present
    /// `{slot, item}`.
    Container,
    /// `PotDecorations`: VarInt count + item ids (at most 4) -> four item names, empty sides as
    /// `minecraft:brick`.
    PotItems,
    /// `TypedEntityData`: VarInt type id + compound -> the compound with `id` set to the type.
    TypedNbt(Reg),
    /// `ByteBufCodecs.registry(type).dispatch(..)` over a `byNameCodec().dispatch("type", ..)`:
    /// VarInt type id -> the type's record with `type` added (`(name, record)` by id).
    Dispatch(&'static [(&'static str, S)]),
    /// Two big-endian i64 (`UUIDUtil.STREAM_CODEC`) -> int stream of 4 (`UUIDUtil.CODEC`).
    Uuid,
    /// `ByteBufCodecs.GAME_PROFILE_PROPERTIES` -> `ExtraCodecs.PROPERTY_MAP`'s list form, grouped
    /// by name in order of first appearance (a Guava `ImmutableMultimap`).
    Properties,
    /// bool -> one of two names (`false`, `true`).
    BoolEnum(&'static str, &'static str),
    /// `ByteBufCodecs.either`: bool, then left (`true`) or right.
    Either(&'static S, &'static S),
    /// Map codecs in one record: their maps concatenated.
    Merge(&'static [S]),
    /// `ExtraCodecs.compactListCodec`: VarInt count + elements -> the element alone when there is
    /// one, else a list.
    CompactList(&'static S),
    /// `DataComponentExactPredicate`: VarInt count + typed components -> map of component names
    /// to their hashes.
    ExactComponents,
    /// A VarInt-counted collection ghast only hashes when empty (empty map).
    EmptyOnly,
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
#[derive(Clone, Copy, Debug)]
enum Value {
    Int(i32),
    Float(f32),
    Bool(bool),
    /// An empty list (the only list default the rows use).
    EmptyList,
    /// A string default (enum names, holder identifiers).
    Name(&'static str),
    /// A structured default, given by its hash.
    Of(fn() -> u32),
}

/// A decoded value ready to hash.
#[derive(Debug, PartialEq)]
enum V {
    /// A record field the codec leaves out.
    Omit,
    Double(f64),
    Int(i32),
    Byte(i8),
    IntList(Vec<i32>),
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
            V::Byte(b) => ops::byte(*b),
            V::IntList(l) => ops::int_list(l),
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
            (V::Byte(a), Value::Int(b)) => i32::from(*a as u8) == *b,
            // Java Float.equals: bit equality
            (V::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            (V::Bool(a), Value::Bool(b)) => a == b,
            (V::List(l), Value::EmptyList) => l.is_empty(),
            (V::Str(s), Value::Name(n)) => s == n,
            (v, Value::Of(f)) => v.hash() == f(),
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
const FIREWORK_SHAPE: &[(i32, &str)] = &[
    (0, "small_ball"),
    (1, "large_ball"),
    (2, "star"),
    (3, "creeper"),
    (4, "burst"),
];

/// `FireworkExplosion.CODEC`.
const EXPLOSION: S = S::Rec(&[
    Field("shape", S::Enum(FIREWORK_SHAPE), Def::Required),
    Field("colors", S::List(&S::Int), Def::Default(Value::EmptyList)),
    Field(
        "fade_colors",
        S::List(&S::Int),
        Def::Default(Value::EmptyList),
    ),
    Field("has_trail", S::Bool, Def::Default(Value::Bool(false))),
    Field("has_twinkle", S::Bool, Def::Default(Value::Bool(false))),
]);

/// `MobEffectInstance.Details.MAP_CODEC` (recursive through `hidden_effect`).
static EFFECT_DETAILS: S = S::Rec(&[
    Field("amplifier", S::UByte, Def::Default(Value::Int(0))),
    Field("duration", S::VarInt, Def::Default(Value::Int(0))),
    Field("ambient", S::Bool, Def::Default(Value::Bool(false))),
    Field("show_particles", S::Bool, Def::Default(Value::Bool(true))),
    Field("show_icon", S::Bool, Def::Required),
    Field("hidden_effect", S::Ref(&EFFECT_DETAILS), Def::Optional),
]);

/// `MobEffectInstance.CODEC`: the effect id and its details in one map.
static EFFECT: S = S::Rec(&[
    Field("id", S::Reg(Reg::MobEffect), Def::Required),
    Field("amplifier", S::UByte, Def::Default(Value::Int(0))),
    Field("duration", S::VarInt, Def::Default(Value::Int(0))),
    Field("ambient", S::Bool, Def::Default(Value::Bool(false))),
    Field("show_particles", S::Bool, Def::Default(Value::Bool(true))),
    Field("show_icon", S::Bool, Def::Required),
    Field("hidden_effect", S::Ref(&EFFECT_DETAILS), Def::Optional),
]);

/// `PotionContents.CODEC`.
static POTION_CONTENTS: S = S::Rec(&[
    Field("potion", S::Reg(Reg::Potion), Def::Optional),
    Field("custom_color", S::Int, Def::Optional),
    Field(
        "custom_effects",
        S::List(&EFFECT),
        Def::Default(Value::EmptyList),
    ),
    Field("custom_name", S::Str, Def::Optional),
]);

/// `Filterable.codec(Codec.STRING)` (the full form is the one encoded).
const FILTERABLE_STR: S = S::Rec(&[
    Field("raw", S::Str, Def::Required),
    Field("filtered", S::Str, Def::Optional),
]);

/// `Filterable.codec(ComponentSerialization.CODEC)`.
const FILTERABLE_TEXT: S = S::Rec(&[
    Field("raw", S::Text, Def::Required),
    Field("filtered", S::Text, Def::Optional),
]);

const USE_ANIMATION: &[(i32, &str)] = &[
    (0, "none"),
    (1, "eat"),
    (2, "drink"),
    (3, "block"),
    (4, "bow"),
    (5, "trident"),
    (6, "crossbow"),
    (7, "spyglass"),
    (8, "toot_horn"),
    (9, "brush"),
    (10, "bundle"),
    (11, "spear"),
];
const EQUIPMENT_SLOT: &[(i32, &str)] = &[
    (0, "mainhand"),
    (1, "feet"),
    (2, "legs"),
    (3, "chest"),
    (4, "head"),
    (5, "offhand"),
    (6, "body"),
    (7, "saddle"),
];

/// `SoundEvent.CODEC`: a registry sound's id, or an inline `{sound_id, range}`.
const SOUND: S = S::HolderOr(
    Reg::SoundEvent,
    &S::Rec(&[
        Field("sound_id", S::Ident, Def::Required),
        Field("range", S::Float, Def::Optional),
    ]),
);

/// `ConsumeEffect.CODEC` (dispatched on the effect type).
static CONSUME_EFFECT: S = S::Dispatch(&[
    (
        "minecraft:apply_effects",
        S::Rec(&[
            Field("effects", S::List(&EFFECT), Def::Required),
            Field("probability", S::Float, Def::Default(Value::Float(1.0))),
        ]),
    ),
    (
        "minecraft:remove_effects",
        S::Rec(&[Field(
            "effects",
            S::HolderSet(Reg::MobEffect),
            Def::Required,
        )]),
    ),
    ("minecraft:clear_all_effects", S::Rec(&[])),
    (
        "minecraft:teleport_randomly",
        S::Rec(&[Field(
            "diameter",
            S::Float,
            Def::Default(Value::Float(16.0)),
        )]),
    ),
    (
        "minecraft:play_sound",
        S::Rec(&[Field("sound", SOUND, Def::Required)]),
    ),
]);

/// `Consumable.CODEC`.
static CONSUMABLE: S = S::Rec(&[
    Field("consume_seconds", S::Float, Def::Default(Value::Float(1.6))),
    Field(
        "animation",
        S::Enum(USE_ANIMATION),
        Def::Default(Value::Name("eat")),
    ),
    Field(
        "sound",
        SOUND,
        Def::Default(Value::Name("minecraft:entity.generic.eat")),
    ),
    Field(
        "has_consume_particles",
        S::Bool,
        Def::Default(Value::Bool(true)),
    ),
    Field(
        "on_consume_effects",
        S::List(&CONSUME_EFFECT),
        Def::Default(Value::EmptyList),
    ),
]);

/// `DeathProtection.CODEC`.
static DEATH_PROTECTION: S = S::Rec(&[Field(
    "death_effects",
    S::List(&CONSUME_EFFECT),
    Def::Default(Value::EmptyList),
)]);

/// `StatePropertiesPredicate.ValueMatcher.CODEC`: an exact value, or `{min, max}`.
const STATE_MATCHER: S = S::Either(
    &S::Str,
    &S::Rec(&[
        Field("min", S::Str, Def::Optional),
        Field("max", S::Str, Def::Optional),
    ]),
);

/// `BlockPredicate.CODEC` (an NBT predicate encodes as SNBT text, which ghast does not print).
const BLOCK_PREDICATE: S = S::Rec(&[
    Field("blocks", S::HolderSet(Reg::Block), Def::Optional),
    Field("state", S::Map(&S::Str, &STATE_MATCHER), Def::Optional),
    Field("nbt", S::Never, Def::Optional),
    Field(
        "components",
        S::ExactComponents,
        Def::Default(Value::Of(ops::empty_map)),
    ),
    Field(
        "predicates",
        S::EmptyOnly,
        Def::Default(Value::Of(ops::empty_map)),
    ),
]);

/// `ResolvableProfile.CODEC` (its full form): a game profile or a partial one, plus the skin
/// patch.
const PROFILE: S = S::Merge(&[
    S::Either(
        &S::Rec(&[
            Field("id", S::Uuid, Def::Required),
            Field("name", S::Str, Def::Required),
            Field("properties", S::Properties, Def::Default(Value::EmptyList)),
        ]),
        &S::Rec(&[
            Field("name", S::Str, Def::Optional),
            Field("id", S::Uuid, Def::Optional),
            Field("properties", S::Properties, Def::Default(Value::EmptyList)),
        ]),
    ),
    S::Rec(&[
        Field("texture", S::Ident, Def::Optional),
        Field("cape", S::Ident, Def::Optional),
        Field("elytra", S::Ident, Def::Optional),
        Field("model", S::BoolEnum("wide", "slim"), Def::Optional),
    ]),
]);

/// `KineticWeapon.Condition.CODEC`.
const KINETIC_CONDITION: S = S::Rec(&[
    Field("max_duration_ticks", S::VarInt, Def::Required),
    Field("min_speed", S::Float, Def::Default(Value::Float(0.0))),
    Field(
        "min_relative_speed",
        S::Float,
        Def::Default(Value::Float(0.0)),
    ),
]);

/// `BlocksAttacks` default `damage_reductions`: one `DamageReduction(90, empty, 0, 1)`.
fn default_damage_reductions() -> u32 {
    ops::list([ops::map(vec![
        (ops::string("base"), ops::float(0.0)),
        (ops::string("factor"), ops::float(1.0)),
    ])])
}

/// `BlocksAttacks.ItemDamageFunction.DEFAULT` (1, 0, 1).
fn default_item_damage() -> u32 {
    ops::map(vec![
        (ops::string("threshold"), ops::float(1.0)),
        (ops::string("base"), ops::float(0.0)),
        (ops::string("factor"), ops::float(1.0)),
    ])
}

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
        c::POTION_CONTENTS => S::Ref(&POTION_CONTENTS),
        c::SUSPICIOUS_STEW_EFFECTS => S::List(&S::Rec(&[
            Field("id", S::Reg(Reg::MobEffect), R),
            Field("duration", S::VarInt, D(Value::Int(160))),
        ])),
        c::LODESTONE_TRACKER => S::Rec(&[
            Field(
                "target",
                S::Rec(&[
                    Field("dimension", S::Ident, R),
                    Field("pos", S::BlockPos, R),
                ]),
                O,
            ),
            Field("tracked", S::Bool, D(Value::Bool(true))),
        ]),
        c::BLOCK_STATE => S::Map(&S::Str, &S::Str),
        c::POT_DECORATIONS => S::PotItems,
        c::TRIM => S::Rec(&[
            Field(
                "material",
                S::Holder(Reg::Data("minecraft:trim_material")),
                R,
            ),
            Field("pattern", S::Holder(Reg::Data("minecraft:trim_pattern")), R),
        ]),
        c::BANNER_PATTERNS => S::List(&S::Rec(&[
            Field(
                "pattern",
                S::Holder(Reg::Data("minecraft:banner_pattern")),
                R,
            ),
            Field("color", S::Enum(DYE), R),
        ])),
        c::FIREWORK_EXPLOSION => S::Ref(&EXPLOSION),
        c::FIREWORKS => S::Rec(&[
            Field("flight_duration", S::UByte, D(Value::Int(0))),
            Field("explosions", S::List(&EXPLOSION), D(Value::EmptyList)),
        ]),
        c::CONTAINER => S::Container,
        c::CAN_PLACE_ON | c::CAN_BREAK => S::CompactList(&BLOCK_PREDICATE),
        c::PROFILE => PROFILE,
        c::CONSUMABLE => S::Ref(&CONSUMABLE),
        c::DEATH_PROTECTION => S::Ref(&DEATH_PROTECTION),
        c::TOOL => S::Rec(&[
            Field(
                "rules",
                S::List(&S::Rec(&[
                    Field("blocks", S::HolderSet(Reg::Block), R),
                    Field("speed", S::Float, O),
                    Field("correct_for_drops", S::Bool, O),
                ])),
                R,
            ),
            Field("default_mining_speed", S::Float, D(Value::Float(1.0))),
            Field("damage_per_block", S::VarInt, D(Value::Int(1))),
            Field(
                "can_destroy_blocks_in_creative",
                S::Bool,
                D(Value::Bool(true)),
            ),
        ]),
        c::EQUIPPABLE => S::Rec(&[
            Field("slot", S::Enum(EQUIPMENT_SLOT), R),
            Field(
                "equip_sound",
                SOUND,
                D(Value::Name("minecraft:item.armor.equip_generic")),
            ),
            Field("asset_id", S::Ident, O),
            Field("camera_overlay", S::Ident, O),
            Field("allowed_entities", S::HolderSet(Reg::EntityType), O),
            Field("dispensable", S::Bool, D(Value::Bool(true))),
            Field("swappable", S::Bool, D(Value::Bool(true))),
            Field("damage_on_hurt", S::Bool, D(Value::Bool(true))),
            Field("equip_on_interact", S::Bool, D(Value::Bool(false))),
            Field("can_be_sheared", S::Bool, D(Value::Bool(false))),
            Field(
                "shearing_sound",
                SOUND,
                D(Value::Name("minecraft:item.shears.snip")),
            ),
        ]),
        c::BLOCKS_ATTACKS => S::Rec(&[
            Field("block_delay_seconds", S::Float, D(Value::Float(0.0))),
            Field("disable_cooldown_scale", S::Float, D(Value::Float(1.0))),
            Field(
                "damage_reductions",
                S::List(&S::Rec(&[
                    Field("horizontal_blocking_angle", S::Float, D(Value::Float(90.0))),
                    Field("type", S::HolderSet(Reg::Data("minecraft:damage_type")), O),
                    Field("base", S::Float, R),
                    Field("factor", S::Float, R),
                ])),
                D(Value::Of(default_damage_reductions)),
            ),
            Field(
                "item_damage",
                S::Rec(&[
                    Field("threshold", S::Float, R),
                    Field("base", S::Float, R),
                    Field("factor", S::Float, R),
                ]),
                D(Value::Of(default_item_damage)),
            ),
            Field(
                "bypassed_by",
                S::HolderSet(Reg::Data("minecraft:damage_type")),
                O,
            ),
            Field("block_sound", SOUND, O),
            Field("disabled_sound", SOUND, O),
        ]),
        c::PIERCING_WEAPON => S::Rec(&[
            Field("deals_knockback", S::Bool, D(Value::Bool(true))),
            Field("dismounts", S::Bool, D(Value::Bool(false))),
            Field("sound", SOUND, O),
            Field("hit_sound", SOUND, O),
        ]),
        c::KINETIC_WEAPON => S::Rec(&[
            Field("contact_cooldown_ticks", S::VarInt, D(Value::Int(10))),
            Field("delay_ticks", S::VarInt, D(Value::Int(0))),
            Field("dismount_conditions", KINETIC_CONDITION, O),
            Field("knockback_conditions", KINETIC_CONDITION, O),
            Field("damage_conditions", KINETIC_CONDITION, O),
            Field("forward_movement", S::Float, D(Value::Float(0.0))),
            Field("damage_multiplier", S::Float, D(Value::Float(1.0))),
            Field("sound", SOUND, O),
            Field("hit_sound", SOUND, O),
        ]),
        c::BUNDLE_CONTENTS | c::CHARGED_PROJECTILES => S::List(&S::Template),
        c::USE_REMAINDER => S::Template,
        c::ENTITY_DATA => S::TypedNbt(Reg::EntityType),
        c::BLOCK_ENTITY_DATA => S::TypedNbt(Reg::BlockEntityType),
        c::BEES => S::List(&S::Rec(&[
            Field("entity_data", S::TypedNbt(Reg::EntityType), R),
            Field("ticks_in_hive", S::VarInt, R),
            Field("min_ticks_in_hive", S::VarInt, R),
        ])),
        c::WRITABLE_BOOK_CONTENT => S::Rec(&[Field(
            "pages",
            S::List(&FILTERABLE_STR),
            D(Value::EmptyList),
        )]),
        c::WRITTEN_BOOK_CONTENT => S::Rec(&[
            Field("title", S::Ref(&FILTERABLE_STR), R),
            Field("author", S::Str, R),
            Field("generation", S::VarInt, D(Value::Int(0))),
            Field("pages", S::List(&FILTERABLE_TEXT), D(Value::EmptyList)),
            Field("resolved", S::Bool, D(Value::Bool(false))),
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
        Reg::MobEffect => generated::MOB_EFFECTS.get(i).copied(),
        Reg::EntityType => generated::ENTITY_TYPES.get(i).copied(),
        Reg::BlockEntityType => generated::BLOCK_ENTITY_TYPES.get(i).copied(),
        Reg::Block => ghast_data::v26_1::Block::from_id(u16::try_from(i).ok()?).map(|b| b.name()),
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
        S::UByte => V::Byte(r.read_varint().ok()? as i8),
        S::BlockPos => {
            let l = r.read_i64().ok()?;
            V::IntList(vec![
                (l >> 38) as i32,
                ((l << 52) >> 52) as i32,
                ((l << 26) >> 38) as i32,
            ])
        }
        S::HolderOr(reg, inline) => {
            let id = r.read_varint().ok()?;
            if id == 0 {
                read(inline, r, regs)?
            } else {
                V::Str(ident(reg, name_of(reg, id - 1, regs)?))
            }
        }
        S::Ref(inner) => read(inner, r, regs)?,
        S::Uuid => {
            let (hi, lo) = (r.read_i64().ok()?, r.read_i64().ok()?);
            V::IntList(vec![
                (hi >> 32) as i32,
                hi as i32,
                (lo >> 32) as i32,
                lo as i32,
            ])
        }
        S::Properties => {
            let n = r.read_varint().ok()?;
            let mut props: Vec<(String, V)> = Vec::new();
            for _ in 0..n.max(0) {
                let name = r.read_string().ok()?.to_string();
                let value = r.read_string().ok()?.to_string();
                let mut m = vec![
                    (V::Str("name".into()), V::Str(name.clone())),
                    (V::Str("value".into()), V::Str(value)),
                ];
                if r.read_bool().ok()? {
                    let sig = r.read_string().ok()?.to_string();
                    m.push((V::Str("signature".into()), V::Str(sig)));
                }
                props.push((name, V::Map(m)));
            }
            // ImmutableMultimap: keys in first-appearance order, each key's values together
            let mut keys: Vec<String> = Vec::new();
            for (k, _) in &props {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
            let mut out = Vec::with_capacity(props.len());
            for k in &keys {
                for (pk, v) in props.iter_mut() {
                    if pk == k {
                        out.push(std::mem::replace(v, V::Omit));
                    }
                }
            }
            V::List(out)
        }
        S::BoolEnum(f, t) => V::Str(if r.read_bool().ok()? { t } else { f }.to_string()),
        S::Either(left, right) => {
            if r.read_bool().ok()? {
                read(left, r, regs)?
            } else {
                read(right, r, regs)?
            }
        }
        S::Merge(parts) => {
            let mut all = Vec::new();
            for p in parts {
                let V::Map(m) = read(p, r, regs)? else {
                    return None;
                };
                all.extend(m);
            }
            V::Map(all)
        }
        S::CompactList(e) => {
            let n = r.read_varint().ok()?;
            let mut v = Vec::new();
            for _ in 0..n.max(0) {
                v.push(read(e, r, regs)?);
            }
            if v.len() == 1 { v.pop()? } else { V::List(v) }
        }
        S::ExactComponents => {
            let n = r.read_varint().ok()?;
            let patch = ComponentPatch::read(r, n, 0).ok()?;
            let mut m = Vec::new();
            for (t, value) in patch.iter() {
                let name = COMPONENT_NAMES.get(usize::try_from(t).ok()?)?;
                let h = component_hash(t, &value, regs)?;
                m.push((V::Str(format!("minecraft:{name}")), V::Hash(h as u32)));
            }
            V::Map(m)
        }
        S::EmptyOnly => {
            if r.read_varint().ok()? != 0 {
                return None;
            }
            V::Map(Vec::new())
        }
        S::Never => return None,
        S::Dispatch(types) => {
            let id = r.read_varint().ok()?;
            let (name, rec) = types.get(usize::try_from(id).ok()?)?;
            let V::Map(mut m) = read(rec, r, regs)? else {
                return None;
            };
            m.push((V::Str("type".into()), V::Str((*name).to_string())));
            V::Map(m)
        }
        S::Template => template(r, regs)?,
        S::Container => {
            let n = r.read_varint().ok()?;
            let mut v = Vec::new();
            for i in 0..n.max(0) {
                if r.read_bool().ok()? {
                    v.push(V::Map(vec![
                        (V::Str("slot".into()), V::Int(i)),
                        (V::Str("item".into()), template(r, regs)?),
                    ]));
                }
            }
            V::List(v)
        }
        S::PotItems => {
            let n = r.read_varint().ok()?;
            if !(0..=4).contains(&n) {
                return None;
            }
            let mut v = Vec::with_capacity(4);
            for _ in 0..n {
                let id = r.read_varint().ok()?;
                v.push(V::Str(ident(Reg::Item, name_of(Reg::Item, id, regs)?)));
            }
            while v.len() < 4 {
                v.push(V::Str("minecraft:brick".into()));
            }
            V::List(v)
        }
        S::TypedNbt(reg) => {
            let id = r.read_varint().ok()?;
            let name = ident(reg, name_of(reg, id, regs)?);
            V::Hash(typed_nbt_hash(&AnonNbt::read(r).ok()?, &name)?)
        }
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

/// `ItemStackTemplate.STREAM_CODEC` read, `ItemStackTemplate.CODEC` (its map form) written.
fn template(r: &mut Reader<'_>, regs: &HashRegistries) -> Option<V> {
    let item = r.read_varint().ok()?;
    let count = r.read_varint().ok()?;
    let added = r.read_varint().ok()?;
    let removed = r.read_varint().ok()?;
    let patch = ComponentPatch::read(r, added, removed).ok()?;
    let mut m = vec![(
        V::Str("id".into()),
        V::Str(ident(Reg::Item, name_of(Reg::Item, item, regs)?)),
    )];
    if count != 1 {
        m.push((V::Str("count".into()), V::Int(count)));
    }
    let mut comps = Vec::new();
    for (t, value) in patch.iter() {
        let name = COMPONENT_NAMES.get(usize::try_from(t).ok()?)?;
        let h = component_hash(t, &value, regs)?;
        comps.push((V::Str(format!("minecraft:{name}")), V::Hash(h as u32)));
    }
    for t in patch.removed() {
        let name = COMPONENT_NAMES.get(usize::try_from(t).ok()?)?;
        comps.push((V::Str(format!("!minecraft:{name}")), V::Map(Vec::new())));
    }
    if !comps.is_empty() {
        m.push((V::Str("components".into()), V::Map(comps)));
    }
    Some(V::Map(m))
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
