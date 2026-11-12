//! Modern (26.x) sessions on the page, written once and mounted per version by `v26_x.rs` (the
//! parent, `super`: the version's `d26` data set, `ent` entity module and `INV` inventory
//! version): the page draws with the 1.8 asset set (block models, item icons, mob models keyed
//! by 1.8 names), so the version's blocks, items and entity types are mapped onto their 1.8
//! equivalents here. Exact where 1.8 has the block or item (pre-flattening `id:meta` from
//! minecraft-data's legacy table, matched property by property); blocks and items added later
//! become a look-alike (new woods → old woods, deepslate → stone, copper → orange terracotta,
//! concrete → stained clay, any stair / slab / fence / door ... → a 1.8 one of the same shape);
//! anything else degrades to stone (a full cube), a stone slab (a low shape), water / lava (a
//! fluid) or air (no collision).
//!
//! The page's world is 256 blocks high; [`crate::window::Window`] picks the 256 rows of the
//! dimension around the bot that are shown (`y` on the page = world `y − lo`).

use std::collections::HashMap;
use std::sync::OnceLock;

use super::d26::{self, BlockState, Property};
use super::{INV, ent};
use ghast_data::v1_8 as d18;

use crate::legacy_table;

/// Names that changed between the flattening (1.13, the legacy table's names) and 26.1.
const RENAMED: &[(&str, &str)] = &[
    ("grass", "short_grass"),
    ("stone_slab", "smooth_stone_slab"),
    ("sign", "oak_sign"),
    ("wall_sign", "oak_wall_sign"),
    ("grass_path", "dirt_path"),
];

/// Woods 1.8 does not have → one it has.
const WOODS: &[(&str, &str)] = &[
    ("pale_oak", "birch"),
    ("cherry", "birch"),
    ("mangrove", "jungle"),
    ("bamboo", "jungle"),
    ("crimson", "dark_oak"),
    ("warped", "acacia"),
];

/// Whole-name look-alikes for blocks newer than 1.8.
const LOOKALIKES: &[(&str, &str)] = &[
    ("deepslate", "stone"),
    ("cobbled_deepslate", "cobblestone"),
    ("polished_deepslate", "stone_bricks"),
    ("deepslate_bricks", "stone_bricks"),
    ("cracked_deepslate_bricks", "cracked_stone_bricks"),
    ("deepslate_tiles", "stone_bricks"),
    ("cracked_deepslate_tiles", "cracked_stone_bricks"),
    ("chiseled_deepslate", "chiseled_stone_bricks"),
    ("reinforced_deepslate", "obsidian"),
    ("tuff", "andesite"),
    ("polished_tuff", "polished_andesite"),
    ("chiseled_tuff", "polished_andesite"),
    ("tuff_bricks", "stone_bricks"),
    ("chiseled_tuff_bricks", "chiseled_stone_bricks"),
    ("calcite", "diorite"),
    ("dripstone_block", "andesite"),
    ("smooth_basalt", "stone"),
    ("basalt", "stone"),
    ("polished_basalt", "stone"),
    ("blackstone", "cobblestone"),
    ("gilded_blackstone", "cobblestone"),
    ("polished_blackstone", "stone"),
    ("polished_blackstone_bricks", "stone_bricks"),
    ("cracked_polished_blackstone_bricks", "cracked_stone_bricks"),
    ("chiseled_polished_blackstone", "chiseled_stone_bricks"),
    ("mud", "coarse_dirt"),
    ("packed_mud", "dirt"),
    ("mud_bricks", "bricks"),
    ("rooted_dirt", "coarse_dirt"),
    ("dirt_path", "dirt"),
    ("moss_block", "grass_block"),
    ("moss_carpet", "green_carpet"),
    ("pale_moss_block", "grass_block"),
    ("pale_moss_carpet", "white_carpet"),
    ("sculk", "black_terracotta"),
    ("sculk_catalyst", "black_terracotta"),
    ("sculk_shrieker", "black_terracotta"),
    ("crimson_nylium", "netherrack"),
    ("warped_nylium", "netherrack"),
    ("nether_gold_ore", "nether_quartz_ore"),
    ("soul_soil", "soul_sand"),
    ("shroomlight", "glowstone"),
    ("crying_obsidian", "obsidian"),
    ("ancient_debris", "obsidian"),
    ("respawn_anchor", "obsidian"),
    ("lodestone", "stone_bricks"),
    ("netherite_block", "iron_block"),
    ("magma_block", "netherrack"),
    ("nether_wart_block", "red_wool"),
    ("warped_wart_block", "cyan_wool"),
    ("red_nether_bricks", "nether_bricks"),
    ("bone_block", "quartz_block"),
    ("amethyst_block", "purple_stained_glass"),
    ("budding_amethyst", "purple_stained_glass"),
    ("copper_ore", "iron_ore"),
    ("raw_iron_block", "iron_block"),
    ("raw_gold_block", "gold_block"),
    ("raw_copper_block", "orange_terracotta"),
    ("purpur_block", "quartz_block"),
    ("purpur_pillar", "quartz_pillar"),
    ("purpur_stairs", "quartz_stairs"),
    ("purpur_slab", "quartz_slab"),
    ("end_stone_bricks", "end_stone"),
    ("observer", "dispenser"),
    ("honey_block", "orange_stained_glass"),
    ("honeycomb_block", "orange_terracotta"),
    ("dried_kelp_block", "green_terracotta"),
    ("blue_ice", "packed_ice"),
    ("smooth_stone", "smooth_stone_slab"),
    ("shulker_box", "purple_wool"),
    ("target", "hay_block"),
    ("tinted_glass", "black_stained_glass"),
    ("bubble_column", "water"),
    ("kelp", "water"),
    ("kelp_plant", "water"),
    ("seagrass", "water"),
    ("tall_seagrass", "water"),
    ("barrel", "oak_planks"),
    ("composter", "oak_planks"),
    ("crafter", "dropper"),
    ("vault", "iron_block"),
    ("trial_spawner", "spawner"),
    ("creaking_heart", "oak_log"),
    ("resin_block", "orange_terracotta"),
    ("resin_bricks", "bricks"),
];

/// Name endings: a 1.8 block of the same shape (properties are matched by name).
const SHAPES: &[(&str, &str)] = &[
    ("_wall_hanging_sign", "oak_wall_sign"),
    ("_hanging_sign", "oak_sign"),
    ("_wall_sign", "oak_wall_sign"),
    ("_sign", "oak_sign"),
    ("_fence_gate", "oak_fence_gate"),
    ("_fence", "oak_fence"),
    ("_trapdoor", "oak_trapdoor"),
    ("_door", "oak_door"),
    ("_stairs", "cobblestone_stairs"),
    ("_slab", "cobblestone_slab"),
    ("_wall", "cobblestone_wall"),
    ("_button", "stone_button"),
    ("_pressure_plate", "stone_pressure_plate"),
    ("_carpet", "white_carpet"),
    ("_leaves", "oak_leaves"),
    ("_log", "oak_log"),
    ("_stem", "oak_log"),
    ("_wood", "oak_log"),
    ("_hyphae", "oak_log"),
    ("_planks", "oak_planks"),
    ("_sapling", "oak_sapling"),
    ("_bed", "red_bed"),
    ("_wall_banner", "white_wall_banner"),
    ("_banner", "white_banner"),
    ("_ore", "stone"),
    ("_bricks", "stone_bricks"),
    ("_tiles", "stone_bricks"),
    ("_glass_pane", "glass_pane"),
    ("_glass", "glass"),
    ("_wool", "white_wool"),
    ("_terracotta", "terracotta"),
    ("_bars", "iron_bars"),
    ("_chain", "iron_bars"),
    ("_lantern", "glowstone"),
    ("_rail", "rail"),
    ("_torch", "torch"),
    ("_wall_torch", "wall_torch"),
];

/// The flattened name of the legacy table, as 26.1 calls it.
fn renamed(n: &str) -> &str {
    RENAMED.iter().find(|r| r.0 == n).map_or(n, |r| r.1)
}

/// A 1.8 raw state if 1.8 has this `id:meta`.
fn raw_1_8(id: u16, meta: u8) -> Option<u16> {
    let raw = id.checked_shl(4)? | u16::from(meta);
    if id > 255 {
        return None;
    }
    let s = d18::BlockState::from_raw_vanilla(raw);
    (s.raw() == raw && (raw == 0 || !s.is_air())).then_some(raw)
}

type Candidates = HashMap<u16, Vec<(u16, Vec<(Property, &'static str)>)>>;

/// 26.1 block id → the 1.8 states that flatten to it, with the properties they set.
fn candidates() -> &'static Candidates {
    static C: OnceLock<Candidates> = OnceLock::new();
    C.get_or_init(|| {
        let mut c: Candidates = HashMap::new();
        for &(id, meta, spec) in legacy_table::BLOCKS {
            // A source block is the still fluid (9, 11), not the flowing one at level 0.
            if matches!(id, 8 | 10) && meta == 0 {
                continue;
            }
            let Some(raw) = raw_1_8(id, meta) else {
                continue;
            };
            let (name, props) = spec.split_once('[').unwrap_or((spec, ""));
            let Some(block) = d26::Block::from_name(renamed(name)) else {
                continue;
            };
            let props: Vec<(Property, &str)> = props
                .trim_end_matches(']')
                .split(',')
                .filter_map(|kv| kv.split_once('='))
                .filter_map(|(k, v)| Some((Property::from_name(k)?, v)))
                .collect();
            c.entry(block.id()).or_default().push((raw, props));
        }
        c
    })
}

/// The best 1.8 state of `block` for the properties of `s` (most matching properties).
fn best(block: d26::Block, s: BlockState) -> Option<u16> {
    let list = candidates().get(&block.id())?;
    let mut best: Option<(usize, u16)> = None;
    for (raw, props) in list {
        let score = props.iter().filter(|(p, v)| s.get(*p) == Some(*v)).count();
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, *raw));
        }
    }
    best.map(|b| b.1)
}

/// Look-alike block names for a 26.1 name, best first.
fn alternatives(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(l) = LOOKALIKES.iter().find(|l| l.0 == name) {
        out.push(l.1.to_string());
    }
    let mut n = name.to_string();
    for p in [
        "waxed_",
        "exposed_",
        "weathered_",
        "oxidized_",
        "stripped_",
        "infested_",
    ] {
        if let Some(rest) = n.strip_prefix(p) {
            n = rest.to_string();
        }
    }
    for &(new, old) in WOODS {
        if n.contains(new) {
            n = n.replace(new, old);
        }
    }
    if n.contains("copper") {
        n = match n.as_str() {
            s if s.ends_with("_door") => "iron_door".into(),
            s if s.ends_with("_trapdoor") => "iron_trapdoor".into(),
            s if s.ends_with("_stairs") => "brick_stairs".into(),
            s if s.ends_with("_slab") => "brick_slab".into(),
            s if s.ends_with("_bulb") => "redstone_lamp".into(),
            s if s.ends_with("_bars") || s.ends_with("_chain") => "iron_bars".into(),
            s if s.ends_with("_lantern") || s.ends_with("_torch") => "torch".into(),
            _ => "orange_terracotta".into(),
        };
    }
    n = n
        .replace("deepslate_", "")
        .replace("_concrete_powder", "_wool")
        .replace("_glazed_terracotta", "_terracotta")
        .replace("_concrete", "_terracotta")
        .replace("_shulker_box", "_wool")
        .replace("blackstone", "cobblestone")
        .replace("potted_", "potted_poppy_");
    if n.starts_with("potted_poppy_") {
        n = "flower_pot".into();
    }
    if n != name {
        out.push(n.clone());
    }
    if let Some(s) = SHAPES.iter().find(|s| n.ends_with(s.0)) {
        out.push(s.1.to_string());
    }
    out
}

/// Look-alike: a 1.8 state of another block with the properties of `s` matched by name.
fn lookalike(s: BlockState) -> Option<u16> {
    for alt in alternatives(s.block().name()) {
        let Some(b) = d26::Block::from_name(&alt) else {
            continue;
        };
        let Some(list) = candidates().get(&b.id()) else {
            continue;
        };
        let mine: Vec<(Property, &str)> = s.properties().collect();
        let mut best: Option<(usize, u16)> = None;
        for (raw, props) in list {
            let score = props.iter().filter(|p| mine.contains(p)).count();
            if best.is_none_or(|(b, _)| score > b) {
                best = Some((score, *raw));
            }
        }
        if let Some(b) = best {
            return Some(b.1);
        }
    }
    None
}

/// The last resort: by shape and fluid.
fn by_shape(s: BlockState) -> u16 {
    match s.phys().fluid() >> 6 {
        1 => return 9 << 4,
        2 => return 11 << 4,
        _ => {}
    }
    let shape = s.voxel_shape();
    if shape.empty {
        return 0;
    }
    let top = shape.boxes.iter().map(|b| b.max_y).fold(0.0, f64::max);
    if top <= 0.5 + 1e-9 { 44 << 4 } else { 1 << 4 }
}

/// Bit 15 of a table entry: the 1.8 state is a stand-in, not the same block (1.8 states use
/// 12 bits).
const STAND_IN: u16 = 1 << 15;

fn map_state(s: BlockState) -> u16 {
    if s.is_air() {
        return 0;
    }
    match best(s.block(), s) {
        Some(raw) => raw,
        None => lookalike(s).unwrap_or_else(|| by_shape(s)) | STAND_IN,
    }
}

fn block_entry(state: u32) -> u16 {
    static T: OnceLock<Vec<u16>> = OnceLock::new();
    let t = T.get_or_init(|| {
        (0..d26::STATE_COUNT)
            .map(|i| BlockState::from_id(i as u16).map_or(0, map_state))
            .collect()
    });
    t.get(state as usize).copied().unwrap_or(0)
}

/// The 1.8 state (`id << 4 | meta`) the page draws for a 26.1 state id.
#[must_use]
pub fn block(state: u32) -> u16 {
    block_entry(state) & !STAND_IN
}

/// Whether a 26.1 state is drawn with a stand-in (1.8 has no such block).
#[must_use]
pub fn block_is_stand_in(state: u32) -> bool {
    block_entry(state) & STAND_IN != 0
}

/// Items newer than 1.8 → a 1.8 item of the same kind.
const ITEM_LOOKALIKES: &[(&str, &str)] = &[
    ("copper_ingot", "gold_ingot"),
    ("copper_nugget", "gold_nugget"),
    ("raw_copper", "gold_nugget"),
    ("raw_iron", "iron_ingot"),
    ("raw_gold", "gold_ingot"),
    ("netherite_ingot", "iron_ingot"),
    ("netherite_scrap", "flint"),
    ("amethyst_shard", "prismarine_crystals"),
    ("echo_shard", "prismarine_shard"),
    ("trident", "stick"),
    ("crossbow", "bow"),
    ("shield", "oak_sign"),
    ("totem_of_undying", "gold_ingot"),
    ("spyglass", "stick"),
    ("elytra", "leather_chestplate"),
];

/// An item name with newer material prefixes swapped for 1.8 ones (netherite tools and armour
/// → diamond, copper ones → stone / chainmail).
fn item_alternative(name: &str) -> Option<String> {
    if let Some(l) = ITEM_LOOKALIKES.iter().find(|l| l.0 == name) {
        return Some(l.1.to_string());
    }
    if let Some(rest) = name.strip_prefix("netherite_") {
        return Some(format!("diamond_{rest}"));
    }
    if let Some(rest) = name.strip_prefix("copper_") {
        let armour = ["helmet", "chestplate", "leggings", "boots"].contains(&rest);
        return Some(format!(
            "{}_{rest}",
            if armour { "chainmail" } else { "stone" }
        ));
    }
    None
}

/// Paper: what an item 1.8 has no look-alike for is shown as.
const UNKNOWN_ITEM: (u16, u16) = (339, 0);

/// The 1.8 `(id, meta)` the page draws for a 26.1 item id with this damage.
#[must_use]
pub fn item(id: i32, damage: i16) -> (u16, u16) {
    item_drawn(id, damage).0
}

/// [`item`] and whether the icon is a stand-in (1.8 has no such item).
#[must_use]
pub fn item_drawn(id: i32, damage: i16) -> ((u16, u16), bool) {
    static T: OnceLock<HashMap<&'static str, (u16, u16)>> = OnceLock::new();
    let names = T.get_or_init(|| {
        let mut m = HashMap::new();
        for &(id, meta, name) in legacy_table::ITEMS {
            if d18::Item::from_id(id).is_some() {
                m.entry(renamed(name)).or_insert((id, meta));
            }
        }
        m
    });
    if id <= 0 {
        return ((0, 0), false);
    }
    let Some(it) = u16::try_from(id).ok().and_then(d26::Item::from_id) else {
        return (UNKNOWN_ITEM, true);
    };
    let name = it.name();
    let damaged = |(i, m): (u16, u16)| (i, if m == 0 { damage.max(0) as u16 } else { m });
    if let Some(&v) = names.get(name) {
        return (damaged(v), false);
    }
    if let Some(&v) = item_alternative(name).and_then(|n| names.get(n.as_str())) {
        return (damaged(v), true);
    }
    // A block item: its block's 1.8 look-alike.
    if let Some(b) = d26::Block::from_name(name) {
        let st = u32::from(b.default_state().id());
        let raw = block(st);
        if raw != 0 {
            return ((raw >> 4, raw & 15), block_is_stand_in(st));
        }
    }
    for alt in alternatives(name) {
        if let Some(&v) = names.get(alt.as_str()) {
            return (damaged(v), true);
        }
    }
    (UNKNOWN_ITEM, true)
}

/// The truth about a 26.1 item for text (labels, tooltips, scripts): its registry id and
/// English name, count, damage, enchantments (the server's `minecraft:enchantment` registry
/// index and, when `enchantment_name` knows it, its name), custom name, and whether its icon is
/// a stand-in (with the 1.8 `[id, meta]` drawn).
#[must_use]
pub fn stack_text<'a>(
    st: &ghast_inventory::Stack,
    enchantment_name: impl Fn(i32) -> Option<&'a str>,
) -> serde_json::Value {
    let dmg = st.damage_value(INV).clamp(0, i32::from(i16::MAX)) as i16;
    let ((did, dmeta), stand_in) = item_drawn(st.id(), dmg);
    let it = u16::try_from(st.id()).ok().and_then(d26::Item::from_id);
    let ench: Vec<serde_json::Value> = st
        .enchantments(INV)
        .iter()
        .map(|e| match &e.id {
            ghast_inventory::EnchantmentId::Registry(i) => match enchantment_name(*i) {
                Some(n) => serde_json::json!({ "id": n, "registry": i, "level": e.level }),
                None => serde_json::json!({ "registry": i, "level": e.level }),
            },
            ghast_inventory::EnchantmentId::Name(n) => {
                serde_json::json!({ "id": n, "level": e.level })
            }
            ghast_inventory::EnchantmentId::Legacy(i) => {
                serde_json::json!({ "legacy": i, "level": e.level })
            }
            _ => serde_json::json!({ "level": e.level }),
        })
        .collect();
    let mut j = serde_json::json!({
        "id": it.map_or_else(|| format!("#{}", st.id()), |i| format!("minecraft:{}", i.name())),
        "name": it.map_or("", |i| i.display_name()),
        "count": st.count(),
        "standIn": stand_in,
        "drawn": [did, dmeta],
    });
    if st.is_damageable(INV) {
        j["damage"] = st.damage_value(INV).into();
        j["maxDamage"] = st.max_damage(INV).into();
    }
    if !ench.is_empty() {
        j["enchantments"] = ench.into();
    }
    if let Some(n) = st.custom_name(INV) {
        j["customName"] = n.to_plain().into();
    }
    j
}

/// The truth about a 26.1 entity for text: registry id, English name, whether its model is a
/// stand-in, and a dropped item's real item.
#[must_use]
pub fn entity_text(e: &ghast_entity::Entity) -> Option<serde_json::Value> {
    let info = ent::info(e.entity_type())?;
    let mut j = serde_json::json!({
        "id": e.id(),
        "type": format!("minecraft:{}", info.name),
        "name": info.display_name,
        "standIn": entity_is_stand_in(info.name),
    });
    if let Some(st) = ent::item_stack(e) {
        let it = u16::try_from(st.id).ok().and_then(d26::Item::from_id);
        let (_, stand_in) = item_drawn(st.id, 0);
        j["item"] = serde_json::json!({
            "id": it.map_or_else(|| format!("#{}", st.id), |i| format!("minecraft:{}", i.name())),
            "name": it.map_or("", |i| i.display_name()),
            "count": st.count,
            "standIn": stand_in,
        });
    }
    Some(j)
}

/// 26.1 entity names → 1.8 entity list names where the CamelCase rule does not give one.
const ENTITY_NAMES: &[(&str, &str)] = &[
    ("zombified_piglin", "PigZombie"),
    ("magma_cube", "LavaSlime"),
    ("mooshroom", "MushroomCow"),
    ("iron_golem", "VillagerGolem"),
    ("snow_golem", "SnowMan"),
    ("ocelot", "Ozelot"),
    ("cat", "Ozelot"),
    ("horse", "EntityHorse"),
    ("donkey", "EntityHorse"),
    ("mule", "EntityHorse"),
    ("skeleton_horse", "EntityHorse"),
    ("zombie_horse", "EntityHorse"),
    ("wither", "WitherBoss"),
    ("elder_guardian", "Guardian"),
    ("wither_skeleton", "Skeleton"),
    ("stray", "Skeleton"),
    ("bogged", "Skeleton"),
    ("husk", "Zombie"),
    ("drowned", "Zombie"),
    ("zombie_villager", "Zombie"),
    ("end_crystal", "EnderCrystal"),
    ("tnt", "PrimedTnt"),
    ("falling_block", "FallingSand"),
    ("minecart", "MinecartRideable"),
    ("chest_minecart", "MinecartChest"),
    ("furnace_minecart", "MinecartFurnace"),
    ("tnt_minecart", "MinecartTNT"),
    ("spawner_minecart", "MinecartSpawner"),
    ("hopper_minecart", "MinecartHopper"),
    ("command_block_minecart", "MinecartCommandBlock"),
    ("egg", "ThrownEgg"),
    ("ender_pearl", "ThrownEnderpearl"),
    ("splash_potion", "ThrownPotion"),
    ("lingering_potion", "ThrownPotion"),
    ("experience_bottle", "ThrownExpBottle"),
    ("eye_of_ender", "EyeOfEnderSignal"),
    ("firework_rocket", "FireworksRocketEntity"),
    ("fishing_bobber", "FishingHook"),
    ("spectral_arrow", "Arrow"),
    ("llama", "EntityHorse"),
    ("trader_llama", "EntityHorse"),
    ("pillager", "Villager"),
    ("vindicator", "Villager"),
    ("evoker", "Villager"),
    ("wandering_trader", "Villager"),
];

/// Table entries that are the same entity in 1.8 (the rest of [`ENTITY_NAMES`] are stand-ins).
const SAME_ENTITY: &[&str] = &[
    "zombified_piglin",
    "magma_cube",
    "mooshroom",
    "iron_golem",
    "snow_golem",
    "ocelot",
    "horse",
    "wither",
    "end_crystal",
    "tnt",
    "falling_block",
    "minecart",
    "chest_minecart",
    "furnace_minecart",
    "tnt_minecart",
    "spawner_minecart",
    "hopper_minecart",
    "command_block_minecart",
    "egg",
    "ender_pearl",
    "splash_potion",
    "experience_bottle",
    "eye_of_ender",
    "firework_rocket",
    "fishing_bobber",
];

/// Whether a 26.1 entity type is drawn with a stand-in model (or a plain box).
#[must_use]
pub fn entity_is_stand_in(name: &str) -> bool {
    if ENTITY_NAMES.iter().any(|n| n.0 == name) {
        return !SAME_ENTITY.contains(&name);
    }
    if name.ends_with("_boat") {
        return name != "oak_boat";
    }
    entity_name(name).is_empty()
}

/// The 1.8 entity list name the page draws a 26.1 entity type with (`""`: a plain box).
#[must_use]
pub fn entity_name(name: &str) -> String {
    if let Some(n) = ENTITY_NAMES.iter().find(|n| n.0 == name) {
        return n.1.to_string();
    }
    if name.ends_with("_boat") || name.ends_with("_raft") {
        return "Boat".into();
    }
    let camel: String = name
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect();
    let objects = [
        "Arrow",
        "Snowball",
        "Fireball",
        "SmallFireball",
        "WitherSkull",
        "ItemFrame",
        "LeashKnot",
        "ArmorStand",
        "Painting",
    ];
    if objects.contains(&camel.as_str()) || d18::entity_by_name(&camel).is_some() {
        camel
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::Window;

    fn b(spec: &str) -> u16 {
        block(u32::from(BlockState::parse(spec).unwrap().id()))
    }

    #[test]
    fn old_blocks_are_exact() {
        assert_eq!(b("air"), 0);
        assert_eq!(b("stone"), 1 << 4);
        assert_eq!(b("granite"), 1 << 4 | 1);
        assert_eq!(b("grass_block"), 2 << 4);
        assert_eq!(b("oak_planks"), 5 << 4);
        assert_eq!(b("spruce_planks"), 5 << 4 | 1);
        assert_eq!(b("water"), 9 << 4);
        assert_eq!(b("smooth_stone_slab[type=bottom]"), 44 << 4);
        assert_eq!(b("smooth_stone_slab[type=top]"), 44 << 4 | 8);
        assert_eq!(b("oak_stairs[facing=east,half=bottom]"), 53 << 4);
        assert_eq!(b("oak_stairs[facing=west,half=top]"), 53 << 4 | 5);
        assert_eq!(b("short_grass"), 31 << 4 | 1);
        assert_eq!(b("red_wool"), 35 << 4 | 14);
        assert_eq!(b("oak_log[axis=x]"), 17 << 4 | 4);
        assert_eq!(b("ladder[facing=north]"), 65 << 4 | 2);
    }

    #[test]
    fn new_blocks_look_alike() {
        assert_eq!(b("deepslate"), 1 << 4);
        assert_eq!(b("cobbled_deepslate"), 4 << 4);
        assert_eq!(b("deepslate_iron_ore"), 15 << 4);
        assert_eq!(b("tuff"), 1 << 4 | 5);
        assert_eq!(b("cherry_planks"), 5 << 4 | 2);
        assert_eq!(b("crimson_planks"), 5 << 4 | 5);
        assert_eq!(b("mangrove_log[axis=y]"), 17 << 4 | 3);
        assert_eq!(b("white_concrete"), 159 << 4);
        assert_eq!(b("copper_block"), 159 << 4 | 1);
        assert_eq!(b("mud"), 3 << 4 | 1);
        // A new stair keeps its facing.
        let s = b("cherry_stairs[facing=west,half=top]");
        assert_eq!(s & 15, 5, "{s}");
        // Every 26.1 state maps onto a registered 1.8 state.
        for i in 0..d26::STATE_COUNT as u32 {
            let raw = block(i);
            let s = d18::BlockState::from_raw_vanilla(raw);
            assert_eq!(s.raw(), raw, "state {i}");
        }
    }

    #[test]
    fn items() {
        let id = |n: &str| i32::from(d26::Item::from_name(n).unwrap().id());
        assert_eq!(item(id("stone"), 0), (1, 0));
        assert_eq!(item(id("diamond_pickaxe"), 7), (278, 7));
        assert_eq!(item(id("oak_planks"), 0), (5, 0));
        assert_eq!(item(id("cherry_planks"), 0), (5, 2));
        assert_eq!(item(id("bread"), 0), (297, 0));
        assert_eq!(item(0, 0), (0, 0));
        assert_eq!(item(id("netherite_sword"), 9), (276, 9));
        assert_eq!(item(id("copper_ingot"), 0), (266, 0));
        assert_eq!(item(id("copper_block"), 0), (159, 1));
        assert_eq!(item(id("music_disc_creator"), 0), UNKNOWN_ITEM);
    }

    #[test]
    fn entities() {
        assert_eq!(entity_name("zombie"), "Zombie");
        assert_eq!(entity_name("cave_spider"), "CaveSpider");
        assert_eq!(entity_name("armor_stand"), "ArmorStand");
        assert_eq!(entity_name("zombified_piglin"), "PigZombie");
        assert_eq!(entity_name("oak_boat"), "Boat");
        assert_eq!(entity_name("warden"), "");
    }

    #[test]
    fn windows() {
        let ow = (-64, 384);
        assert_eq!(Window::around(4.0, ow).lo, -64);
        assert_eq!(Window::around(300.0, ow).lo, 64);
        assert_eq!(Window::around(100.0, ow).lo, -32);
        assert!(Window::around(100.0, ow).keeps(100.0, ow));
        assert!(!Window::around(100.0, ow).keeps(220.0, ow));
        assert_eq!(Window::around(64.0, (0, 256)).lo, 0);
    }
}
