//! Physics-exact path planning and following for the ghast Minecraft Java Edition bot library.
//!
//! - [`Pathfinder`]: A* over feet blocks. Moves: walk (incl. step-ups onto slabs and partial
//!   blocks), diagonal, jump up, drop down (configurable maximum fall), 1- and 2-block gap
//!   jumps, swimming, ladder / vine climbing, and optionally digging and placing. Costs are
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

mod class;
mod control;
mod cost;
#[cfg(feature = "client")]
mod follower;
mod goal;
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
    V1_8,
    /// 1.16.5 (protocol 754).
    V1_16_5,
}

#[cfg(feature = "client")]
impl From<ghast_client::GameVersion> for Version {
    fn from(v: ghast_client::GameVersion) -> Self {
        match v {
            ghast_client::GameVersion::V1_8_9 => Version::V1_8,
            _ => Version::V1_16_5,
        }
    }
}
