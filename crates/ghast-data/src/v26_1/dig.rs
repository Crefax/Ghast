//! 26.1 block breaking time: vanilla's destroy progress per tick and the tick on which the client
//! reports the block broken (`MultiPlayerGameMode.startDestroyBlock` / `continueDestroyBlock`,
//! `BlockBehaviour.getDestroyProgress`, `Player.getDestroySpeed`), in `f32` exactly as the client
//! computes it. Verified against the oracle (`tests/golden/26.1/dig.jsonl.gz`).
//!
//! The functions take the tool's numbers rather than an item, so a stack whose `minecraft:tool`
//! component a server changed works the same: pass that component's speed and correctness (for
//! default stacks: [`Item::destroy_speed`](super::Item::destroy_speed) /
//! [`Item::correct_for_drops`](super::Item::correct_for_drops)).

use super::{BlockState, Item};

/// The player's mining conditions (attribute values as the server sent them, effect amplifiers).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DigConditions {
    /// `minecraft:mining_efficiency` (efficiency enchantment: level² + 1; default 0).
    pub mining_efficiency: f64,
    /// `minecraft:block_break_speed` (default 1).
    pub block_break_speed: f64,
    /// `minecraft:submerged_mining_speed` (default 0.2; aqua affinity raises it).
    pub submerged_mining_speed: f64,
    /// Haste amplifier (0 = Haste I), if active.
    pub haste: Option<i32>,
    /// Conduit power amplifier, if active (counts like haste).
    pub conduit_power: Option<i32>,
    /// Mining fatigue amplifier, if active.
    pub mining_fatigue: Option<i32>,
    /// The eyes are in water (`isEyeInFluid(WATER)`).
    pub eye_in_water: bool,
    /// On the ground (otherwise the speed is divided by 5).
    pub on_ground: bool,
}

impl Default for DigConditions {
    fn default() -> Self {
        DigConditions {
            mining_efficiency: 0.0,
            block_break_speed: 1.0,
            submerged_mining_speed: 0.2,
            haste: None,
            conduit_power: None,
            mining_fatigue: None,
            eye_in_water: false,
            on_ground: true,
        }
    }
}

/// `Player.getDestroySpeed`: the held tool's speed on the block with efficiency, effects,
/// attributes, water and air penalties applied.
#[must_use]
pub fn destroy_speed(tool_speed: f32, c: &DigConditions) -> f32 {
    let mut speed = tool_speed;
    if speed > 1.0 {
        speed += c.mining_efficiency as f32;
    }
    if c.haste.is_some() || c.conduit_power.is_some() {
        let amp = c.haste.unwrap_or(0).max(c.conduit_power.unwrap_or(0));
        speed *= 1.0 + amp.wrapping_add(1) as f32 * 0.2;
    }
    if let Some(amp) = c.mining_fatigue {
        speed *= match amp {
            0 => 0.3,
            1 => 0.09,
            2 => 0.0027,
            _ => 8.1e-4,
        };
    }
    speed *= c.block_break_speed as f32;
    if c.eye_in_water {
        speed *= c.submerged_mining_speed as f32;
    }
    if !c.on_ground {
        speed /= 5.0;
    }
    speed
}

/// `BlockBehaviour.getDestroyProgress`: the fraction of the block broken per tick.
/// `tool_correct` is the held stack's "correct for drops" on this block.
#[must_use]
pub fn destroy_progress(
    state: BlockState,
    tool_speed: f32,
    tool_correct: bool,
    c: &DigConditions,
) -> f32 {
    let time = state.destroy_time();
    if time == -1.0 {
        return 0.0;
    }
    let modifier = if !state.requires_correct_tool_for_drops() || tool_correct {
        30
    } else {
        100
    };
    destroy_speed(tool_speed, c) / time / modifier as f32
}

/// [`destroy_progress`] for a held item with its default components (`None` = empty hand).
#[must_use]
pub fn destroy_progress_with(state: BlockState, held: Option<Item>, c: &DigConditions) -> f32 {
    let b = state.block();
    let (speed, correct) = held.map_or((1.0, false), |i| {
        (i.destroy_speed(b), i.correct_for_drops(b))
    });
    destroy_progress(state, speed, correct, c)
}

pub use crate::common::{BreakTime, break_time};
