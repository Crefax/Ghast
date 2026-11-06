//! 1.8 block facts the asset builder needs: blocks whose variant lives in the metadata
//! (the page's mesher picks `"v=<n>,<key>"` entries for them), and the blockstates file of a
//! block name when it differs.

/// Dye colours in damage order.
pub const COLORS: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "silver",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];
/// Wood names in planks order.
pub const WOODS: [&str; 6] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak"];

/// How one metadata variant gets its models.
#[derive(Debug, Clone)]
pub enum Rule {
    /// One model replaces the whole block: the entry `"v=<n>,normal"`.
    Model(String),
    /// The blockstates' own entries with their models renamed: `"v=<n>,<key>"`.
    Rename(Rename),
}

/// A model rename.
#[derive(Debug, Clone)]
pub enum Rename {
    /// Unchanged.
    Same,
    /// `from` at the start becomes `to`.
    Prefix(&'static str, String),
    /// `from` at the end becomes `to`.
    Suffix(&'static str, String),
    /// The first `from` anywhere becomes `to`.
    First(&'static str, String),
}

impl Rename {
    /// Applies the rename to a model name.
    #[must_use]
    pub fn apply(&self, m: &str) -> String {
        match self {
            Rename::Same => m.to_string(),
            Rename::Prefix(from, to) => match m.strip_prefix(from) {
                Some(rest) => format!("{to}{rest}"),
                None => m.to_string(),
            },
            Rename::Suffix(from, to) => match m.strip_suffix(from) {
                Some(rest) => format!("{rest}{to}"),
                None => m.to_string(),
            },
            Rename::First(from, to) => m.replacen(from, to, 1),
        }
    }
}

/// The variant rules of a block name (`None`: no variant in the metadata).
#[must_use]
pub fn variants(name: &str) -> Option<Vec<Rule>> {
    let models = |l: &[&str]| {
        l.iter()
            .map(|m| Rule::Model((*m).to_string()))
            .collect::<Vec<_>>()
    };
    let pre = |from: &'static str, to: String| Rule::Rename(Rename::Prefix(from, to));
    Some(match name {
        "stone" => models(&[
            "stone",
            "granite",
            "granite_smooth",
            "diorite",
            "diorite_smooth",
            "andesite",
            "andesite_smooth",
        ]),
        "dirt" => models(&["dirt", "coarse_dirt", "podzol"]),
        "planks" | "double_wooden_slab" => WOODS
            .iter()
            .map(|w| Rule::Model(format!("{w}_planks")))
            .collect(),
        "sapling" => WOODS.iter().map(|w| pre("oak_", format!("{w}_"))).collect(),
        "sand" => models(&["sand", "red_sand"]),
        "log" => WOODS[..4]
            .iter()
            .map(|w| pre("oak_", format!("{w}_")))
            .collect(),
        "log2" => ["acacia", "dark_oak"]
            .iter()
            .map(|w| pre("acacia_", format!("{w}_")))
            .collect(),
        "leaves" => WOODS[..4]
            .iter()
            .map(|w| Rule::Model(format!("{w}_leaves")))
            .collect(),
        "leaves2" => models(&["acacia_leaves", "dark_oak_leaves"]),
        "sandstone" => models(&["sandstone_normal", "sandstone_chiseled", "sandstone_smooth"]),
        "red_sandstone" => models(&[
            "red_sandstone_normal",
            "red_sandstone_chiseled",
            "red_sandstone_smooth",
        ]),
        "tallgrass" => models(&["dead_bush", "tall_grass", "fern"]),
        "wool" => COLORS
            .iter()
            .map(|c| Rule::Model(format!("{c}_wool")))
            .collect(),
        "red_flower" => models(&[
            "poppy",
            "orchid",
            "allium",
            "houstonia",
            "red_tulip",
            "orange_tulip",
            "white_tulip",
            "pink_tulip",
            "daisy",
        ]),
        "double_stone_slab" => models(&[
            "double_stone",
            "sandstone_normal",
            "oak_planks",
            "cobblestone",
            "brick",
            "stonebrick_normal",
            "nether_brick",
            "quartz_normal",
        ]),
        "stone_slab" => [
            "stone",
            "sandstone",
            "oak",
            "cobblestone",
            "brick",
            "stone_brick",
            "nether_brick",
            "quartz",
        ]
        .iter()
        .map(|v| Rule::Rename(Rename::Suffix("_stone", format!("_{v}"))))
        .collect(),
        "wooden_slab" => WOODS
            .iter()
            .map(|w| Rule::Rename(Rename::Suffix("_oak", format!("_{w}"))))
            .collect(),
        "stained_glass" => COLORS
            .iter()
            .map(|c| Rule::Model(format!("glass_{c}")))
            .collect(),
        "stained_glass_pane" => COLORS
            .iter()
            .map(|c| pre("white_", format!("{c}_")))
            .collect(),
        "stained_hardened_clay" => COLORS
            .iter()
            .map(|c| Rule::Model(format!("hardened_clay_{c}")))
            .collect(),
        "carpet" => COLORS
            .iter()
            .map(|c| Rule::Model(format!("carpet_{c}")))
            .collect(),
        "monster_egg" => models(&[
            "stone",
            "cobblestone",
            "stonebrick_normal",
            "stonebrick_mossy",
            "stonebrick_cracked",
            "stonebrick_chiseled",
        ]),
        "stonebrick" => models(&[
            "stonebrick_normal",
            "stonebrick_mossy",
            "stonebrick_cracked",
            "stonebrick_chiseled",
        ]),
        "prismarine" => models(&["prismarine_rough", "prismarine_bricks", "prismarine_dark"]),
        "cobblestone_wall" => vec![
            Rule::Rename(Rename::Same),
            pre("cobblestone_wall_", "mossy_wall_".into()),
        ],
        "double_plant" => ["sunflower", "syringa", "grass", "fern", "rose", "paeonia"]
            .iter()
            .map(|v| Rule::Rename(Rename::First("sunflower", (*v).to_string())))
            .collect(),
        _ => return None,
    })
}

/// The vanilla blockstates file of a 1.8 block name, when it is not the name itself. `None`
/// for blocks without one (drawn by the page itself, or not at all).
#[must_use]
pub fn blockstates_file(name: &str) -> Option<&'static str> {
    Some(match name {
        "planks" => "oak_planks",
        "sapling" => "oak_sapling",
        "log" => "oak_log",
        "leaves" => "oak_leaves",
        "tallgrass" => "tall_grass",
        "deadbush" => "dead_bush",
        "wool" => "white_wool",
        "piston_extension" => "piston",
        "yellow_flower" => "dandelion",
        "red_flower" => "poppy",
        "double_stone_slab" => "stone_double_slab",
        "stained_glass" => "white_stained_glass",
        "double_wooden_slab" => "oak_double_slab",
        "wooden_slab" => "oak_slab",
        "stained_hardened_clay" => "white_stained_hardened_clay",
        "stained_glass_pane" => "white_stained_glass_pane",
        "leaves2" => "acacia_leaves",
        "log2" => "acacia_log",
        "carpet" => "white_carpet",
        "double_plant" => "sunflower",
        "double_stone_slab2" => "red_sandstone_double_slab",
        "stone_slab2" => "red_sandstone_slab",
        _ => return None,
    })
}

/// Block names whose blockstates file must not be used (the page draws them itself, or the
/// file under that name describes another block).
pub const NO_BLOCKSTATES: &[&str] = &[
    "air",
    "water",
    "flowing_water",
    "lava",
    "flowing_lava",
    "chest",
    "trapped_chest",
    "ender_chest",
    "standing_sign",
    "wall_sign",
    "monster_egg",
    "end_portal",
    "skull",
    "barrier",
    "standing_banner",
    "wall_banner",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_match_the_js_regexes() {
        assert_eq!(
            Rename::Prefix("oak_", "birch_".into()).apply("oak_sapling"),
            "birch_sapling"
        );
        assert_eq!(
            Rename::Prefix("oak_", "birch_".into()).apply("half_oak_x"),
            "half_oak_x"
        );
        assert_eq!(
            Rename::Suffix("_oak", "_birch".into()).apply("half_slab_oak"),
            "half_slab_birch"
        );
        assert_eq!(
            Rename::First("sunflower", "rose".into()).apply("double_sunflower_top"),
            "double_rose_top"
        );
    }
}
