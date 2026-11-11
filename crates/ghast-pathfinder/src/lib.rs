//! Physics-exact path planning and following for the ghast Minecraft Java Edition bot library.
//!
//! - [`Pathfinder`]: A* over feet blocks. Moves: walk (incl. step-ups onto slabs and partial
//!   blocks), diagonal, jump up, drop down (configurable maximum fall), gap jumps (up to 3
//!   blocks, also one up or down; each validated in the physics), swimming, ladder / vine
//!   climbing, and optionally digging and placing. Costs are
//!   ticks **measured with `ghast-physics`** (see [`Costs`]). Unknown (unloaded) blocks are
//!   never treated as air: they are impassable, and a goal beyond them yields a partial path.
//! - [`Controller`]: turns a path into per-tick keys and aim. It picks the keys by simulating
//!   the vanilla-exact physics one tick ahead, and turns in whole mouse counts exactly like the
//!   client does, so a simulated run and a live run make the same decisions.
//! - [`simulate`]: runs a path with the controller in `ghast-physics`; the validation tests use
//!   it to check every move type on random terrain, and [`Pathfinder::plan_verified`] uses it
//!   to repair a plan before the bot moves.
//! - `Follower` (feature `client`, default): a `ghast_client::Behavior` helper that plans,
//!   follows, replans on deviations, teleports and blocked moves, all through `TickContext`.
//!
//! ```
//! use ghast_pathfinder::{Config, Goal, Pathfinder, Version};
//!
//! let mut world = ghast_physics::v1_8::memory_world();
//! world.load_chunks(-2, -2, 2, 2);
//! world.fill_raw([-20, 63, -20], [20, 63, 20], 1 << 4); // stone floor
//! world.fill_raw([3, 64, -5], [3, 64, 5], 1 << 4); // a low wall to jump over
//! let pf = Pathfinder::new(Config::default());
//! let path = pf.plan(Version::V1_8, &world, [0, 64, 0], &Goal::Block([8, 64, 0])).unwrap();
//! assert!(!path.partial);
//! assert_eq!(path.steps.last().unwrap().pos, [8, 64, 0]);
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, unnameable_types)]
// Without a version feature the crate builds (nothing can be planned: `Version` has no
// variant and `Version::from_game` is always `None`) but the per-version machinery is unused.
#![cfg_attr(
    not(any(feature = "v1_8", feature = "v1_12_2", feature = "v1_16_5")),
    allow(
        dead_code,
        unused_imports,
        unused_variables,
        unused_mut,
        unreachable_code
    )
)]

mod class;
mod control;
mod cost;
#[cfg(feature = "client")]
mod follower;
mod goal;
mod jump;
mod moves;
mod physics;
mod plan;
mod sim;
mod world;

pub use control::{Command, Controller, ControllerConfig, Progress};
pub use cost::Costs;
#[cfg(feature = "client")]
pub use follower::{FollowConfig, FollowError, FollowStatus, Follower};
pub use goal::Goal;
pub use moves::{MoveKind, NodeKind};
pub use physics::Observation;
pub use plan::{Budget, Config, DigCost, Path, Pathfinder, Place, PlanError, Search, Step};
pub use sim::{SimOutcome, SimReport, Simulation, simulate};
pub use world::BlockSource;

/// The protocol version whose blocks and physics a plan uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Version {
    /// 1.8.9 (protocol 47).
    #[cfg(feature = "v1_8")]
    V1_8,
    /// 1.12.2 (protocol 340).
    #[cfg(feature = "v1_12_2")]
    V1_12,
    /// 1.16.5 (protocol 754).
    #[cfg(feature = "v1_16_5")]
    V1_16_5,
}

impl Version {
    /// The planner's version for a client version: `None` when the planner does not support it
    /// (26.1) or its feature is off.
    #[cfg(feature = "client")]
    #[must_use]
    pub fn from_game(v: ghast_client::GameVersion) -> Option<Version> {
        match v {
            #[cfg(feature = "v1_8")]
            ghast_client::GameVersion::V1_8_9 => Some(Version::V1_8),
            #[cfg(feature = "v1_12_2")]
            ghast_client::GameVersion::V1_12_2 => Some(Version::V1_12),
            #[cfg(feature = "v1_16_5")]
            ghast_client::GameVersion::V1_16_5 => Some(Version::V1_16_5),
            _ => None,
        }
    }
}
