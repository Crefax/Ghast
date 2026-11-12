//! 1.16.5 block breaking time, from the jar (oracle `tests/golden/1.16.5/dig_facts.jsonl.gz`) and
//! proven against the real client (`dig.jsonl.gz`: `ClientPlayerInteractionManager.attackBlock` /
//! `updateBlockBreakingProgress`, `AbstractBlock.calcBlockBreakingDelta`,
//! `PlayerEntity.getBlockBreakingSpeed`), in `f32` exactly as the client computes it. Hardness,
//! the required-tool flag and the tool facts are the same for every state of a block in 1.16.5
//! (checked by the dump), so everything here is keyed by block id.
//!
//! The tool tables here are the jar's (`ItemStack.getMiningSpeedMultiplier` / `isSuitableFor` per
//! item and block); minecraft-data's material / harvest-tool tables in [`super::BlockInfo`] differ
//! from them in places — use these for timing.

use crate::generated::v1_16_5 as g;

pub use crate::common::{BreakTime, break_time};

/// The player's mining conditions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DigConditions {
    /// Efficiency enchantment level of the held stack (0 = none).
    pub efficiency: u8,
    /// Haste amplifier (0 = Haste I), if active.
    pub haste: Option<i32>,
    /// Conduit power amplifier, if active (counts like haste).
    pub conduit_power: Option<i32>,
    /// Mining fatigue amplifier, if active.
    pub mining_fatigue: Option<i32>,
    /// The eyes are in water (`isSubmergedIn(FluidTags.WATER)`).
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
            conduit_power: None,
            mining_fatigue: None,
            eye_in_water: false,
            aqua_affinity: false,
            on_ground: true,
        }
    }
}

fn item_entry(item: u16) -> Option<crate::common::ItemDig> {
    let i = g::dig::ITEM_DIG.binary_search_by_key(&item, |e| e.0).ok()?;
    Some(crate::common::item_dig(
        &g::dig::ITEM_DIG[i],
        &g::dig::ITEM_DIG_SPEEDS,
        &g::dig::ITEM_DIG_BLOCKS,
    ))
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

/// `ItemStack.isSuitableFor(state)` of a stack of `item`.
#[must_use]
pub fn is_effective(item: u16, block: u16) -> bool {
    item_entry(item).is_some_and(|e| e.2.binary_search(&block).is_ok())
}

/// Block hardness from the jar; `-1.0` = unbreakable.
#[must_use]
pub fn strength(block: u16) -> f32 {
    g::dig::BLOCK_STRENGTH
        .get(usize::from(block))
        .copied()
        .unwrap_or(f32::NAN)
}

/// The block's drops need a suitable tool (`AbstractBlockState.isToolRequired`).
#[must_use]
pub fn requires_tool(block: u16) -> bool {
    let i = usize::from(block);
    g::dig::BLOCK_TOOL_FLAG
        .get(i / 64)
        .is_some_and(|w| w & (1 << (i % 64)) != 0)
}

/// `PlayerEntity.getBlockBreakingSpeed(state)`: `held` is the main-hand item id (`None` = empty
/// hand).
#[must_use]
pub fn mining_speed(held: Option<u16>, block: u16, c: &DigConditions) -> f32 {
    let mut f = held.map_or(1.0, |i| speed_multiplier(i, block));
    if f > 1.0 && c.efficiency > 0 && held.is_some() {
        let i = i32::from(c.efficiency);
        f += (i * i + 1) as f32;
    }
    // StatusEffectUtil.hasHaste / getHasteAmplifier: haste or conduit power, the larger amplifier.
    if c.haste.is_some() || c.conduit_power.is_some() {
        let amp = c.haste.unwrap_or(0).max(c.conduit_power.unwrap_or(0));
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

/// `PlayerEntity.canHarvest(state)`.
#[must_use]
pub fn effective_tool(held: Option<u16>, block: u16) -> bool {
    !requires_tool(block) || held.is_some_and(|i| is_effective(i, block))
}

/// `AbstractBlock.calcBlockBreakingDelta`: the fraction of the block broken per tick.
#[must_use]
pub fn destroy_progress(held: Option<u16>, block: u16, c: &DigConditions) -> f32 {
    let f = strength(block);
    if f == -1.0 {
        return 0.0;
    }
    let i = if effective_tool(held, block) { 30 } else { 100 };
    mining_speed(held, block, c) / f / i as f32
}
