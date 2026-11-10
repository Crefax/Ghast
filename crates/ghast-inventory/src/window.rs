//! Container window types: how the vanilla client builds the screen handler for an
//! `open_window` packet, how many slots its container part has, and what kind each slot is.

use crate::rules::Version;

/// The vanilla screen handler (container class) the client created for a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WindowKind {
    /// Chest-like grid of `rows` × 9 slots (1.8 `minecraft:chest`, `minecraft:container` and
    /// unknown types with slots; 1.16.5 `generic_9x1` … `generic_9x6`).
    Chest {
        /// Number of rows (1.8: `slotCount / 9`, may be 0).
        rows: u8,
    },
    /// Dispenser / dropper (1.16.5 `generic_3x3`).
    Generic3x3,
    /// Hopper (5 slots).
    Hopper,
    /// 1.12 / 1.16.5 shulker box (27 slots that refuse shulker boxes).
    ShulkerBox,
    /// Crafting table: result slot 0, grid 1..=9.
    Crafting,
    /// Furnace (also 1.16.5 smoker and blast furnace): input 0, fuel 1, output 2.
    Furnace,
    /// Brewing stand (1.8: 4 slots; 1.12 / 1.16.5: 5 with the blaze powder slot).
    BrewingStand,
    /// Enchanting table (item, lapis).
    Enchanting,
    /// Anvil (2 inputs, result).
    Anvil,
    /// Beacon (payment slot).
    Beacon,
    /// Villager trading (2 inputs, result).
    Merchant,
    /// Horse / donkey / llama inventory with `slots` container slots (saddle, armor, chest).
    Horse {
        /// Container slots as sent by the server.
        slots: u8,
    },
    /// 1.16.5 grindstone.
    Grindstone,
    /// 1.16.5 lectern (one book slot, no player inventory).
    Lectern,
    /// 1.16.5 loom.
    Loom,
    /// 1.16.5 smithing table.
    Smithing,
    /// 1.16.5 cartography table.
    Cartography,
    /// 1.16.5 stonecutter.
    Stonecutter,
}

/// What a slot of a screen handler is, as far as click prediction is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SlotKind {
    /// Plain slot: takes any item, up to 64.
    Normal,
    /// Window 0 armor slot (0 head … 3 feet): its armor type only, one item.
    Armor(u8),
    /// 1.12 / 1.16.5 offhand slot (window 0 slot 45): any item.
    Offhand,
    /// Crafting result: nothing can be put in; taking it consumes the grid.
    CraftResult,
    /// Output slot without side effects on the client (furnace output): nothing can be put in.
    Output,
    /// 1.12 / 1.16.5 shulker box slot: any item but shulker boxes.
    Shulker,
    /// A slot whose rules need data this crate does not model (fuel, brewing ingredients,
    /// enchanting, trading, anvil, beacon, horse equipment, …): clicks whose outcome depends on
    /// it are refused as unpredictable.
    Special,
}

impl WindowKind {
    /// The window type of a 1.8 `open_window` (`inventoryType`, `slotCount`), as
    /// `ClientPlayNetworkHandler.onOpenScreen` builds it. `None`: the vanilla client opens no
    /// container for it (a slotless type it has no GUI for).
    #[must_use]
    pub fn from_v1_8(inventory_type: &str, slot_count: u8) -> Option<WindowKind> {
        Some(match inventory_type {
            // checked before "has slots": a generic container may have none
            "minecraft:container" => WindowKind::Chest {
                rows: slot_count / 9,
            },
            "minecraft:villager" => WindowKind::Merchant,
            "EntityHorse" => WindowKind::Horse { slots: slot_count },
            _ if slot_count == 0 => match inventory_type {
                "minecraft:crafting_table" => WindowKind::Crafting,
                "minecraft:enchanting_table" => WindowKind::Enchanting,
                "minecraft:anvil" => WindowKind::Anvil,
                _ => return None,
            },
            "minecraft:hopper" => WindowKind::Hopper,
            "minecraft:furnace" => WindowKind::Furnace,
            "minecraft:brewing_stand" => WindowKind::BrewingStand,
            "minecraft:beacon" => WindowKind::Beacon,
            "minecraft:dispenser" | "minecraft:dropper" => WindowKind::Generic3x3,
            // minecraft:chest, minecraft:container and anything else with slots
            _ => WindowKind::Chest {
                rows: slot_count / 9,
            },
        })
    }

    /// The window type of a 1.12.2 `open_window` (`inventoryType`, `slotCount`): the 1.8 rules
    /// plus `minecraft:shulker_box`.
    #[must_use]
    pub fn from_v1_12_2(inventory_type: &str, slot_count: u8) -> Option<WindowKind> {
        if inventory_type == "minecraft:shulker_box" && slot_count > 0 {
            return Some(WindowKind::ShulkerBox);
        }
        WindowKind::from_v1_8(inventory_type, slot_count)
    }

    /// The window type of a 1.16.5 `open_window` (menu type registry id).
    #[must_use]
    pub fn from_v1_16_5(menu_type: i32) -> Option<WindowKind> {
        Some(match menu_type {
            0..=5 => WindowKind::Chest {
                rows: menu_type as u8 + 1,
            },
            6 => WindowKind::Generic3x3,
            7 => WindowKind::Anvil,
            8 => WindowKind::Beacon,
            9 | 13 | 21 => WindowKind::Furnace, // blast_furnace, furnace, smoker
            10 => WindowKind::BrewingStand,
            11 => WindowKind::Crafting,
            12 => WindowKind::Enchanting,
            14 => WindowKind::Grindstone,
            15 => WindowKind::Hopper,
            16 => WindowKind::Lectern,
            17 => WindowKind::Loom,
            18 => WindowKind::Merchant,
            19 => WindowKind::ShulkerBox,
            20 => WindowKind::Smithing,
            22 => WindowKind::Cartography,
            23 => WindowKind::Stonecutter,
            _ => return None,
        })
    }

    /// Number of container slots (before the player's 27 + 9).
    #[must_use]
    pub fn container_len(self, version: Version) -> usize {
        match self {
            WindowKind::Chest { rows } => usize::from(rows) * 9,
            WindowKind::Generic3x3 => 9,
            WindowKind::Hopper => 5,
            WindowKind::ShulkerBox => 27,
            WindowKind::Crafting => 10,
            WindowKind::Furnace => 3,
            WindowKind::BrewingStand => match version {
                Version::V1_8 => 4,
                Version::V1_12 | Version::V1_16_5 | Version::V26_1 => 5,
            },
            WindowKind::Enchanting => 2,
            WindowKind::Anvil => 3,
            WindowKind::Beacon => 1,
            WindowKind::Merchant => 3,
            WindowKind::Horse { slots } => usize::from(slots),
            WindowKind::Grindstone => 3,
            WindowKind::Lectern => 1,
            WindowKind::Loom => 4,
            WindowKind::Smithing => 3,
            WindowKind::Cartography => 3,
            WindowKind::Stonecutter => 2,
        }
    }

    /// Whether the player's main inventory and hotbar follow the container slots.
    #[must_use]
    pub fn has_player_inventory(self) -> bool {
        self != WindowKind::Lectern
    }

    /// Kind of container slot `i` (`i < container_len`).
    #[must_use]
    pub fn slot_kind(self, i: usize) -> SlotKind {
        match self {
            WindowKind::Chest { .. } | WindowKind::Generic3x3 | WindowKind::Hopper => {
                SlotKind::Normal
            }
            WindowKind::ShulkerBox => SlotKind::Shulker,
            WindowKind::Crafting => {
                if i == 0 {
                    SlotKind::CraftResult
                } else {
                    SlotKind::Normal
                }
            }
            WindowKind::Furnace => match i {
                0 => SlotKind::Normal,
                1 => SlotKind::Special,
                _ => SlotKind::Output,
            },
            WindowKind::Horse { .. } => {
                if i < 2 {
                    SlotKind::Special
                } else {
                    SlotKind::Normal
                }
            }
            _ => SlotKind::Special,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_8_types() {
        assert_eq!(
            WindowKind::from_v1_8("minecraft:chest", 54),
            Some(WindowKind::Chest { rows: 6 })
        );
        assert_eq!(
            WindowKind::from_v1_8("minecraft:container", 27),
            Some(WindowKind::Chest { rows: 3 })
        );
        assert_eq!(
            WindowKind::from_v1_8("plugin:whatever", 10),
            Some(WindowKind::Chest { rows: 1 })
        );
        assert_eq!(
            WindowKind::from_v1_8("minecraft:crafting_table", 0),
            Some(WindowKind::Crafting)
        );
        assert_eq!(WindowKind::from_v1_8("minecraft:unknown", 0), None);
        assert_eq!(
            WindowKind::from_v1_8("minecraft:dropper", 9),
            Some(WindowKind::Generic3x3)
        );
        assert_eq!(
            WindowKind::from_v1_8("minecraft:brewing_stand", 4)
                .unwrap()
                .container_len(Version::V1_8),
            4
        );
    }

    #[test]
    fn v1_16_types() {
        assert_eq!(
            WindowKind::from_v1_16_5(2),
            Some(WindowKind::Chest { rows: 3 })
        );
        assert_eq!(WindowKind::from_v1_16_5(21), Some(WindowKind::Furnace));
        assert_eq!(WindowKind::from_v1_16_5(24), None);
        assert_eq!(WindowKind::from_v1_16_5(-1), None);
        assert_eq!(WindowKind::BrewingStand.container_len(Version::V1_16_5), 5);
        assert!(!WindowKind::Lectern.has_player_inventory());
    }
}
