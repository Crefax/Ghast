//! 1.8.9 block breaking time, from the jar (oracle `tests/golden/1.8.9/dig_facts.jsonl.gz`) and
//! proven against the real client (`dig.jsonl.gz`: `ClientPlayerInteractionManager.attackBlock` /
//! `updateBlockBreakingProgress`, `Block.calcBlockBreakingData`, `PlayerEntity.getMiningSpeed`),
//! in `f32` exactly as the client computes it.
//!
//! The tool tables here are the jar's (`ItemStack.getMiningSpeedMultiplier` / `isEffectiveOn` per
//! item and block); minecraft-data's material / harvest-tool tables in [`super::BlockInfo`] differ
//! from them in places — use these for timing.

use crate::generated::v1_8 as g;

pub use crate::common::{BreakTime, break_time};

/// The player's mining conditions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DigConditions {
    /// Efficiency enchantment level of the held stack (0 = none).
    pub efficiency: u8,
    /// Haste amplifier (0 = Haste I), if active.
    pub haste: Option<i32>,
    /// Mining fatigue amplifier, if active.
    pub mining_fatigue: Option<i32>,
    /// The eyes are in water (`isSubmergedIn(Material.WATER)`).
    pub eye_in_water: bool,
    /// Aqua affinity on any armour piece.
    pub aqua_affinity: bool,
    /// On the ground (otherwise the speed is divided by 5).
    pub on_ground: bool,
}

impl Default for DigConditions {
    fn default() -> Self {
        DigConditions {
            efficiency: 0,
            haste: None,
            mining_fatigue: None,
            eye_in_water: false,
            aqua_affinity: false,
            on_ground: true,
        }
    }
}

fn item_entry(item: u16) -> Option<&'static crate::common::ItemDig> {
    g::dig::ITEM_DIG
        .binary_search_by_key(&item, |e| e.0)
        .ok()
        .map(|i| &g::dig::ITEM_DIG[i])
}

/// `ItemStack.getMiningSpeedMultiplier(block)` of a stack of `item` (1 for every other pair).
#[must_use]
pub fn speed_multiplier(item: u16, block: u16) -> f32 {
    item_entry(item)
        .and_then(|e| {
            e.1.binary_search_by_key(&block, |x| x.0)
                .ok()
                .map(|i| e.1[i].1)
        })
        .unwrap_or(1.0)
}

/// `ItemStack.isEffectiveOn(block)` of a stack of `item`.
#[must_use]
pub fn is_effective(item: u16, block: u16) -> bool {
    item_entry(item).is_some_and(|e| e.2.binary_search(&block).is_ok())
}

/// Block strength (hardness) from the jar; `-1.0` = unbreakable.
#[must_use]
pub fn strength(block: u16) -> f32 {
    g::dig::BLOCK_STRENGTH
        .get(usize::from(block))
        .copied()
        .unwrap_or(f32::NAN)
}

/// The block's material needs no tool (`Material.doesBlockMovement` in Legacy Yarn; MCP
/// `isToolNotRequired`): any hand harvests it.
#[must_use]
pub fn no_tool_needed(block: u16) -> bool {
    let i = usize::from(block);
    g::dig::BLOCK_TOOL_FLAG
        .get(i / 64)
        .is_some_and(|w| w & (1 << (i % 64)) != 0)
}

/// `PlayerEntity.getMiningSpeed(block)`: `held` is the main-hand item id (`None` = empty hand).
#[must_use]
pub fn mining_speed(held: Option<u16>, block: u16, c: &DigConditions) -> f32 {
    let mut f = held.map_or(1.0, |i| speed_multiplier(i, block));
    if f > 1.0 && c.efficiency > 0 && held.is_some() {
        let i = i32::from(c.efficiency);
        f += (i * i + 1) as f32;
    }
    if let Some(amp) = c.haste {
        f *= 1.0 + amp.wrapping_add(1) as f32 * 0.2;
    }
    if let Some(amp) = c.mining_fatigue {
        f *= match amp {
            0 => 0.3,
            1 => 0.09,
            2 => 0.0027,
            _ => 8.1e-4,
        };
    }
    if c.eye_in_water && !c.aqua_affinity {
        f /= 5.0;
    }
    if !c.on_ground {
        f /= 5.0;
    }
    f
}

/// `PlayerEntity.isUsingEffectiveTool(block)`.
#[must_use]
pub fn effective_tool(held: Option<u16>, block: u16) -> bool {
    no_tool_needed(block) || held.is_some_and(|i| is_effective(i, block))
}

/// `Block.calcBlockBreakingData`: the fraction of the block broken per tick.
#[must_use]
pub fn destroy_progress(held: Option<u16>, block: u16, c: &DigConditions) -> f32 {
    let f = strength(block);
    if f < 0.0 {
        return 0.0;
    }
    let speed = mining_speed(held, block, c);
    if effective_tool(held, block) {
        speed / f / 30.0
    } else {
        speed / f / 100.0
    }
}
