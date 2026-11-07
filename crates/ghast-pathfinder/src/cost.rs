//! Move costs in ticks, measured once per version by running `ghast-physics` on canonical
//! terrain (flat floor, a one-block step, drops, a ladder, a pool).

use std::sync::OnceLock;

use ghast_physics::Keys;

use crate::Version;
use crate::physics::{P12, P16, P18, Physics, SourceView};

/// Measured movement facts of one version (ticks and blocks per tick).
#[derive(Clone, Debug, PartialEq)]
pub struct Costs {
    /// Steady walking speed on stone (blocks per tick).
    pub walk_speed: f64,
    /// Steady sprinting speed on stone (blocks per tick).
    pub sprint_speed: f64,
    /// Ticks to jump onto a one-block step from a standing start at the block centre.
    pub jump_up: f64,
    /// `fall[n]`: ticks to fall `n` blocks from standing at an edge (index 0 unused).
    pub fall: Vec<f64>,
    /// Climbing speed on a ladder (blocks per tick).
    pub climb_up_speed: f64,
    /// Descending speed on a ladder (blocks per tick).
    pub climb_down_speed: f64,
    /// Horizontal swimming speed (blocks per tick).
    pub swim_speed: f64,
    /// Rising speed in water with the jump key (blocks per tick).
    pub swim_up_speed: f64,
    /// Sinking speed in water (blocks per tick).
    pub swim_down_speed: f64,
    /// Ticks in the air of a standing jump onto the same height.
    pub jump_air: f64,
}

impl Costs {
    /// The measured costs of a version.
    #[must_use]
    pub fn of(version: Version) -> &'static Costs {
        static C18: OnceLock<Costs> = OnceLock::new();
        static C12: OnceLock<Costs> = OnceLock::new();
        static C16: OnceLock<Costs> = OnceLock::new();
        match version {
            Version::V1_8 => C18.get_or_init(measure::<P18>),
            Version::V1_12 => C12.get_or_init(measure::<P12>),
            Version::V1_16_5 => C16.get_or_init(measure::<P16>),
        }
    }

    /// Ticks to fall `blocks` (interpolated; beyond the table: terminal speed).
    #[must_use]
    pub fn fall_ticks(&self, blocks: f64) -> f64 {
        if blocks <= 0.0 {
            return 0.0;
        }
        let n = blocks.floor() as usize;
        let last = self.fall.len() - 1;
        if n >= last {
            // ~3.92 blocks per tick terminal velocity.
            return self.fall[last] + (blocks - last as f64) / 3.5;
        }
        let a = self.fall[n];
        let b = self.fall[n + 1];
        a + (b - a) * (blocks - n as f64)
    }
}

/// Stone, air, ladder and water raw states of a version.
fn raws(version: Version) -> (u32, u32, u32) {
    match version {
        Version::V1_8 | Version::V1_12 => (1 << 4, 65 << 4 | 2, 9 << 4),
        Version::V1_16_5 => {
            use ghast_data::v1_16_5::{Block, Property};
            let ladder = Block::LADDER
                .state_with(&[(Property::Facing, "north")])
                .map_or(0, |s| u32::from(s.id()));
            (
                u32::from(Block::STONE.default_state().id()),
                ladder,
                u32::from(Block::WATER.default_state().id()),
            )
        }
    }
}

/// A tiny dense world for measurements.
struct Lab {
    blocks: std::collections::HashMap<[i32; 3], u32>,
}

impl crate::BlockSource for Lab {
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        if !(0..256).contains(&y) {
            return None;
        }
        Some(self.blocks.get(&[x, y, z]).copied().unwrap_or(0))
    }

    fn is_column_loaded(&self, _: i32, _: i32) -> bool {
        true
    }
}

impl Lab {
    fn floor(stone: u32) -> Lab {
        let mut blocks = std::collections::HashMap::new();
        for x in -4..=4 {
            for z in -4..=40 {
                blocks.insert([x, 63, z], stone);
            }
        }
        Lab { blocks }
    }
}

fn run<P: Physics>(
    lab: &Lab,
    s: &mut P::State,
    keys: Keys,
    yaw: f32,
    mut until: impl FnMut(&P::State) -> bool,
    max: usize,
) -> usize {
    let view = SourceView::<P, Lab>::new(lab);
    for t in 0..max {
        P::tick(s, keys, yaw, 0.0, &view);
        if until(s) {
            return t + 1;
        }
    }
    max
}

fn settle<P: Physics>(lab: &Lab, pos: [f64; 3]) -> P::State {
    let mut s = P::new_state(pos, 0.0, 0.0);
    run::<P>(lab, &mut s, Keys::NONE, 0.0, |_| false, 3);
    s
}

fn measure<P: Physics>() -> Costs {
    let (stone, ladder, water) = raws(P::VERSION);
    let fwd = Keys {
        forward: true,
        ..Keys::NONE
    };
    // Steady speeds (yaw 0 walks towards +z).
    let lab = Lab::floor(stone);
    let speed = |keys: Keys| {
        let mut s = settle::<P>(&lab, [0.5, 64.0, 0.5]);
        run::<P>(&lab, &mut s, keys, 0.0, |_| false, 30);
        let z0 = P::observe(&s).pos[2];
        run::<P>(&lab, &mut s, keys, 0.0, |_| false, 5);
        (P::observe(&s).pos[2] - z0) / 5.0
    };
    let walk_speed = speed(fwd);
    let sprint_speed = speed(Keys {
        sprint: true,
        ..fwd
    });

    // Jump onto a step at z = 1.
    let mut step = Lab::floor(stone);
    step.blocks.insert([0, 64, 1], stone);
    let mut s = settle::<P>(&step, [0.5, 64.0, 0.5]);
    let jump_up = run::<P>(
        &step,
        &mut s,
        Keys { jump: true, ..fwd },
        0.0,
        |s| {
            let o = P::observe(s);
            o.on_ground && o.pos[1] >= 65.0 - 1e-9 && o.pos[2] >= 1.3
        },
        60,
    ) as f64;

    // Standing jump in place.
    let mut s = settle::<P>(&lab, [0.5, 64.0, 0.5]);
    let mut left = false;
    let jump_air = run::<P>(
        &lab,
        &mut s,
        Keys {
            jump: true,
            ..Keys::NONE
        },
        0.0,
        |s| {
            let g = P::observe(s).on_ground;
            if !g {
                left = true;
            }
            left && g
        },
        60,
    ) as f64;

    // Falls: drop from y = 64 + n onto the floor (start in the air, at rest).
    let mut fall = vec![0.0];
    for n in 1..=24 {
        let mut s = P::new_state([0.5, 64.0 + f64::from(n), 0.5], 0.0, 0.0);
        let t = run::<P>(
            &lab,
            &mut s,
            Keys::NONE,
            0.0,
            |s| P::observe(s).on_ground,
            200,
        );
        fall.push(t as f64);
    }

    // Ladder: column of ladders at (0, 64..80, 0) facing north (on the block to the south).
    let mut lad = Lab::floor(stone);
    for y in 64..80 {
        lad.blocks.insert([0, y, 0], ladder);
        lad.blocks.insert([0, y, 1], stone);
    }
    let mut s = settle::<P>(&lad, [0.5, 64.0, 0.5]);
    run::<P>(&lad, &mut s, fwd, 0.0, |_| false, 10);
    let y0 = P::observe(&s).pos[1];
    run::<P>(&lad, &mut s, fwd, 0.0, |_| false, 20);
    let climb_up_speed = (P::observe(&s).pos[1] - y0) / 20.0;
    let mut s = P::new_state([0.5, 76.0, 0.5], 0.0, 0.0);
    run::<P>(&lad, &mut s, Keys::NONE, 0.0, |_| false, 10);
    let y0 = P::observe(&s).pos[1];
    run::<P>(&lad, &mut s, Keys::NONE, 0.0, |_| false, 10);
    let climb_down_speed = (y0 - P::observe(&s).pos[1]) / 10.0;

    // Pool: water at y 60..64 over x -4..4, z -4..40.
    let mut pool = Lab::floor(stone);
    for x in -4..=4 {
        for z in -4..=40 {
            pool.blocks.insert([x, 59, z], stone);
            for y in 60..=63 {
                pool.blocks.insert([x, y, z], water);
            }
        }
    }
    let swim_keys = Keys { jump: true, ..fwd };
    let mut s = P::new_state([0.5, 62.6, 0.5], 0.0, 0.0);
    run::<P>(&pool, &mut s, swim_keys, 0.0, |_| false, 40);
    let z0 = P::observe(&s).pos[2];
    run::<P>(&pool, &mut s, swim_keys, 0.0, |_| false, 20);
    let swim_speed = (P::observe(&s).pos[2] - z0) / 20.0;
    let mut s = P::new_state([0.5, 60.0, 0.5], 0.0, 0.0);
    let up = Keys {
        jump: true,
        ..Keys::NONE
    };
    run::<P>(&pool, &mut s, up, 0.0, |_| false, 5);
    let y0 = P::observe(&s).pos[1];
    run::<P>(&pool, &mut s, up, 0.0, |_| false, 10);
    let swim_up_speed = (P::observe(&s).pos[1] - y0) / 10.0;
    let mut s = P::new_state([0.5, 63.0, 0.5], 0.0, 0.0);
    run::<P>(&pool, &mut s, Keys::NONE, 0.0, |_| false, 10);
    let y0 = P::observe(&s).pos[1];
    run::<P>(&pool, &mut s, Keys::NONE, 0.0, |_| false, 10);
    let swim_down_speed = ((y0 - P::observe(&s).pos[1]) / 10.0).max(0.02);

    Costs {
        walk_speed,
        sprint_speed,
        jump_up,
        fall,
        climb_up_speed: climb_up_speed.max(0.05),
        climb_down_speed: climb_down_speed.max(0.05),
        swim_speed: swim_speed.max(0.05),
        swim_up_speed: swim_up_speed.max(0.02),
        swim_down_speed,
        jump_air,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_values_are_vanilla_like() {
        for v in [Version::V1_8, Version::V1_12, Version::V1_16_5] {
            let c = Costs::of(v);
            eprintln!("{v:?}: {c:?}");
            assert!(
                (c.walk_speed - 0.2158).abs() < 0.002,
                "{v:?} walk {}",
                c.walk_speed
            );
            assert!(
                (c.sprint_speed - 0.2806).abs() < 0.003,
                "{v:?} sprint {}",
                c.sprint_speed
            );
            assert!(
                c.jump_up > 4.0 && c.jump_up < 20.0,
                "{v:?} jump_up {}",
                c.jump_up
            );
            assert_eq!(c.jump_air, 12.0);
            assert!(c.fall[3] > 5.0 && c.fall[3] < 12.0);
            assert!(
                c.climb_up_speed > 0.08 && c.climb_up_speed < 0.25,
                "{}",
                c.climb_up_speed
            );
            assert!(c.climb_down_speed > 0.1 && c.climb_down_speed <= 0.16);
            assert!(
                c.swim_speed > 0.05 && c.swim_speed < 0.25,
                "{}",
                c.swim_speed
            );
        }
    }
}
