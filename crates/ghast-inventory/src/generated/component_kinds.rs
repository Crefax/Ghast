// Canonical data component kinds of the modern (26.x) inventory: the 26.1 component type ids,
// then every kind a later version added, in the order it was added. Code works in kinds;
// `crate::tables` maps them to each version's wire ids at compile time (by name).
// A new version's component that is not listed here fails the build: append it.

/// Number of kinds.
pub(crate) const KINDS: usize = 111;

/// Kind names (without `minecraft:`), by kind.
pub(crate) const KIND_NAMES: [&str; KINDS] = [
    "custom_data",
    "max_stack_size",
    "max_damage",
    "damage",
    "unbreakable",
    "use_effects",
    "custom_name",
    "minimum_attack_charge",
    "damage_type",
    "item_name",
    "item_model",
    "lore",
    "rarity",
    "enchantments",
    "can_place_on",
    "can_break",
    "attribute_modifiers",
    "custom_model_data",
    "tooltip_display",
    "repair_cost",
    "creative_slot_lock",
    "enchantment_glint_override",
    "intangible_projectile",
    "food",
    "consumable",
    "use_remainder",
    "use_cooldown",
    "damage_resistant",
    "tool",
    "weapon",
    "attack_range",
    "enchantable",
    "equippable",
    "repairable",
    "glider",
    "tooltip_style",
    "death_protection",
    "blocks_attacks",
    "piercing_weapon",
    "kinetic_weapon",
    "swing_animation",
    "additional_trade_cost",
    "stored_enchantments",
    "dye",
    "dyed_color",
    "map_color",
    "map_id",
    "map_decorations",
    "map_post_processing",
    "charged_projectiles",
    "bundle_contents",
    "potion_contents",
    "potion_duration_scale",
    "suspicious_stew_effects",
    "writable_book_content",
    "written_book_content",
    "trim",
    "debug_stick_state",
    "entity_data",
    "bucket_entity_data",
    "block_entity_data",
    "instrument",
    "provides_trim_material",
    "ominous_bottle_amplifier",
    "jukebox_playable",
    "provides_banner_patterns",
    "recipes",
    "lodestone_tracker",
    "firework_explosion",
    "fireworks",
    "profile",
    "note_block_sound",
    "banner_patterns",
    "base_color",
    "pot_decorations",
    "container",
    "block_state",
    "bees",
    "lock",
    "container_loot",
    "break_sound",
    "villager/variant",
    "wolf/variant",
    "wolf/sound_variant",
    "wolf/collar",
    "fox/variant",
    "salmon/size",
    "parrot/variant",
    "tropical_fish/pattern",
    "tropical_fish/base_color",
    "tropical_fish/pattern_color",
    "mooshroom/variant",
    "rabbit/variant",
    "pig/variant",
    "pig/sound_variant",
    "cow/variant",
    "cow/sound_variant",
    "chicken/variant",
    "chicken/sound_variant",
    "zombie_nautilus/variant",
    "frog/variant",
    "horse/variant",
    "painting/variant",
    "llama/variant",
    "axolotl/variant",
    "cat/variant",
    "cat/sound_variant",
    "cat/collar",
    "sheep/color",
    "shulker/color",
    "sulfur_cube_content",
];

/// `minecraft:custom_data`.
pub(crate) const CUSTOM_DATA: i32 = 0;
/// `minecraft:max_stack_size`.
pub(crate) const MAX_STACK_SIZE: i32 = 1;
/// `minecraft:max_damage`.
pub(crate) const MAX_DAMAGE: i32 = 2;
/// `minecraft:damage`.
pub(crate) const DAMAGE: i32 = 3;
/// `minecraft:unbreakable`.
pub(crate) const UNBREAKABLE: i32 = 4;
/// `minecraft:use_effects`.
pub(crate) const USE_EFFECTS: i32 = 5;
/// `minecraft:custom_name`.
pub(crate) const CUSTOM_NAME: i32 = 6;
/// `minecraft:minimum_attack_charge`.
pub(crate) const MINIMUM_ATTACK_CHARGE: i32 = 7;
/// `minecraft:damage_type`.
pub(crate) const DAMAGE_TYPE: i32 = 8;
/// `minecraft:item_name`.
pub(crate) const ITEM_NAME: i32 = 9;
/// `minecraft:item_model`.
pub(crate) const ITEM_MODEL: i32 = 10;
/// `minecraft:lore`.
pub(crate) const LORE: i32 = 11;
/// `minecraft:rarity`.
pub(crate) const RARITY: i32 = 12;
/// `minecraft:enchantments`.
pub(crate) const ENCHANTMENTS: i32 = 13;
/// `minecraft:can_place_on`.
pub(crate) const CAN_PLACE_ON: i32 = 14;
/// `minecraft:can_break`.
pub(crate) const CAN_BREAK: i32 = 15;
/// `minecraft:attribute_modifiers`.
pub(crate) const ATTRIBUTE_MODIFIERS: i32 = 16;
/// `minecraft:custom_model_data`.
pub(crate) const CUSTOM_MODEL_DATA: i32 = 17;
/// `minecraft:tooltip_display`.
pub(crate) const TOOLTIP_DISPLAY: i32 = 18;
/// `minecraft:repair_cost`.
pub(crate) const REPAIR_COST: i32 = 19;
/// `minecraft:creative_slot_lock`.
pub(crate) const CREATIVE_SLOT_LOCK: i32 = 20;
/// `minecraft:enchantment_glint_override`.
pub(crate) const ENCHANTMENT_GLINT_OVERRIDE: i32 = 21;
/// `minecraft:intangible_projectile`.
pub(crate) const INTANGIBLE_PROJECTILE: i32 = 22;
/// `minecraft:food`.
pub(crate) const FOOD: i32 = 23;
/// `minecraft:consumable`.
pub(crate) const CONSUMABLE: i32 = 24;
/// `minecraft:use_remainder`.
pub(crate) const USE_REMAINDER: i32 = 25;
/// `minecraft:use_cooldown`.
pub(crate) const USE_COOLDOWN: i32 = 26;
/// `minecraft:damage_resistant`.
pub(crate) const DAMAGE_RESISTANT: i32 = 27;
/// `minecraft:tool`.
pub(crate) const TOOL: i32 = 28;
/// `minecraft:weapon`.
pub(crate) const WEAPON: i32 = 29;
/// `minecraft:attack_range`.
pub(crate) const ATTACK_RANGE: i32 = 30;
/// `minecraft:enchantable`.
pub(crate) const ENCHANTABLE: i32 = 31;
/// `minecraft:equippable`.
pub(crate) const EQUIPPABLE: i32 = 32;
/// `minecraft:repairable`.
pub(crate) const REPAIRABLE: i32 = 33;
/// `minecraft:glider`.
pub(crate) const GLIDER: i32 = 34;
/// `minecraft:tooltip_style`.
pub(crate) const TOOLTIP_STYLE: i32 = 35;
/// `minecraft:death_protection`.
pub(crate) const DEATH_PROTECTION: i32 = 36;
/// `minecraft:blocks_attacks`.
pub(crate) const BLOCKS_ATTACKS: i32 = 37;
/// `minecraft:piercing_weapon`.
pub(crate) const PIERCING_WEAPON: i32 = 38;
/// `minecraft:kinetic_weapon`.
pub(crate) const KINETIC_WEAPON: i32 = 39;
/// `minecraft:swing_animation`.
pub(crate) const SWING_ANIMATION: i32 = 40;
/// `minecraft:additional_trade_cost`.
pub(crate) const ADDITIONAL_TRADE_COST: i32 = 41;
/// `minecraft:stored_enchantments`.
pub(crate) const STORED_ENCHANTMENTS: i32 = 42;
/// `minecraft:dye`.
pub(crate) const DYE: i32 = 43;
/// `minecraft:dyed_color`.
pub(crate) const DYED_COLOR: i32 = 44;
/// `minecraft:map_color`.
pub(crate) const MAP_COLOR: i32 = 45;
/// `minecraft:map_id`.
pub(crate) const MAP_ID: i32 = 46;
/// `minecraft:map_decorations`.
pub(crate) const MAP_DECORATIONS: i32 = 47;
/// `minecraft:map_post_processing`.
pub(crate) const MAP_POST_PROCESSING: i32 = 48;
/// `minecraft:charged_projectiles`.
pub(crate) const CHARGED_PROJECTILES: i32 = 49;
/// `minecraft:bundle_contents`.
pub(crate) const BUNDLE_CONTENTS: i32 = 50;
/// `minecraft:potion_contents`.
pub(crate) const POTION_CONTENTS: i32 = 51;
/// `minecraft:potion_duration_scale`.
pub(crate) const POTION_DURATION_SCALE: i32 = 52;
/// `minecraft:suspicious_stew_effects`.
pub(crate) const SUSPICIOUS_STEW_EFFECTS: i32 = 53;
/// `minecraft:writable_book_content`.
pub(crate) const WRITABLE_BOOK_CONTENT: i32 = 54;
/// `minecraft:written_book_content`.
pub(crate) const WRITTEN_BOOK_CONTENT: i32 = 55;
/// `minecraft:trim`.
pub(crate) const TRIM: i32 = 56;
/// `minecraft:debug_stick_state`.
pub(crate) const DEBUG_STICK_STATE: i32 = 57;
/// `minecraft:entity_data`.
pub(crate) const ENTITY_DATA: i32 = 58;
/// `minecraft:bucket_entity_data`.
pub(crate) const BUCKET_ENTITY_DATA: i32 = 59;
/// `minecraft:block_entity_data`.
pub(crate) const BLOCK_ENTITY_DATA: i32 = 60;
/// `minecraft:instrument`.
pub(crate) const INSTRUMENT: i32 = 61;
/// `minecraft:provides_trim_material`.
pub(crate) const PROVIDES_TRIM_MATERIAL: i32 = 62;
/// `minecraft:ominous_bottle_amplifier`.
pub(crate) const OMINOUS_BOTTLE_AMPLIFIER: i32 = 63;
/// `minecraft:jukebox_playable`.
pub(crate) const JUKEBOX_PLAYABLE: i32 = 64;
/// `minecraft:provides_banner_patterns`.
pub(crate) const PROVIDES_BANNER_PATTERNS: i32 = 65;
/// `minecraft:recipes`.
pub(crate) const RECIPES: i32 = 66;
/// `minecraft:lodestone_tracker`.
pub(crate) const LODESTONE_TRACKER: i32 = 67;
/// `minecraft:firework_explosion`.
pub(crate) const FIREWORK_EXPLOSION: i32 = 68;
/// `minecraft:fireworks`.
pub(crate) const FIREWORKS: i32 = 69;
/// `minecraft:profile`.
pub(crate) const PROFILE: i32 = 70;
/// `minecraft:note_block_sound`.
pub(crate) const NOTE_BLOCK_SOUND: i32 = 71;
/// `minecraft:banner_patterns`.
pub(crate) const BANNER_PATTERNS: i32 = 72;
/// `minecraft:base_color`.
pub(crate) const BASE_COLOR: i32 = 73;
/// `minecraft:pot_decorations`.
pub(crate) const POT_DECORATIONS: i32 = 74;
/// `minecraft:container`.
pub(crate) const CONTAINER: i32 = 75;
/// `minecraft:block_state`.
pub(crate) const BLOCK_STATE: i32 = 76;
/// `minecraft:bees`.
pub(crate) const BEES: i32 = 77;
/// `minecraft:lock`.
pub(crate) const LOCK: i32 = 78;
/// `minecraft:container_loot`.
pub(crate) const CONTAINER_LOOT: i32 = 79;
/// `minecraft:break_sound`.
pub(crate) const BREAK_SOUND: i32 = 80;
/// `minecraft:villager/variant`.
pub(crate) const VILLAGER_VARIANT: i32 = 81;
/// `minecraft:wolf/variant`.
pub(crate) const WOLF_VARIANT: i32 = 82;
/// `minecraft:wolf/sound_variant`.
pub(crate) const WOLF_SOUND_VARIANT: i32 = 83;
/// `minecraft:wolf/collar`.
pub(crate) const WOLF_COLLAR: i32 = 84;
/// `minecraft:fox/variant`.
pub(crate) const FOX_VARIANT: i32 = 85;
/// `minecraft:salmon/size`.
pub(crate) const SALMON_SIZE: i32 = 86;
/// `minecraft:parrot/variant`.
pub(crate) const PARROT_VARIANT: i32 = 87;
/// `minecraft:tropical_fish/pattern`.
pub(crate) const TROPICAL_FISH_PATTERN: i32 = 88;
/// `minecraft:tropical_fish/base_color`.
pub(crate) const TROPICAL_FISH_BASE_COLOR: i32 = 89;
/// `minecraft:tropical_fish/pattern_color`.
pub(crate) const TROPICAL_FISH_PATTERN_COLOR: i32 = 90;
/// `minecraft:mooshroom/variant`.
pub(crate) const MOOSHROOM_VARIANT: i32 = 91;
/// `minecraft:rabbit/variant`.
pub(crate) const RABBIT_VARIANT: i32 = 92;
/// `minecraft:pig/variant`.
pub(crate) const PIG_VARIANT: i32 = 93;
/// `minecraft:pig/sound_variant`.
pub(crate) const PIG_SOUND_VARIANT: i32 = 94;
/// `minecraft:cow/variant`.
pub(crate) const COW_VARIANT: i32 = 95;
/// `minecraft:cow/sound_variant`.
pub(crate) const COW_SOUND_VARIANT: i32 = 96;
/// `minecraft:chicken/variant`.
pub(crate) const CHICKEN_VARIANT: i32 = 97;
/// `minecraft:chicken/sound_variant`.
pub(crate) const CHICKEN_SOUND_VARIANT: i32 = 98;
/// `minecraft:zombie_nautilus/variant`.
pub(crate) const ZOMBIE_NAUTILUS_VARIANT: i32 = 99;
/// `minecraft:frog/variant`.
pub(crate) const FROG_VARIANT: i32 = 100;
/// `minecraft:horse/variant`.
pub(crate) const HORSE_VARIANT: i32 = 101;
/// `minecraft:painting/variant`.
pub(crate) const PAINTING_VARIANT: i32 = 102;
/// `minecraft:llama/variant`.
pub(crate) const LLAMA_VARIANT: i32 = 103;
/// `minecraft:axolotl/variant`.
pub(crate) const AXOLOTL_VARIANT: i32 = 104;
/// `minecraft:cat/variant`.
pub(crate) const CAT_VARIANT: i32 = 105;
/// `minecraft:cat/sound_variant`.
pub(crate) const CAT_SOUND_VARIANT: i32 = 106;
/// `minecraft:cat/collar`.
pub(crate) const CAT_COLLAR: i32 = 107;
/// `minecraft:sheep/color`.
pub(crate) const SHEEP_COLOR: i32 = 108;
/// `minecraft:shulker/color`.
pub(crate) const SHULKER_COLOR: i32 = 109;
/// `minecraft:sulfur_cube_content`.
pub(crate) const SULFUR_CUBE_CONTENT: i32 = 110;
