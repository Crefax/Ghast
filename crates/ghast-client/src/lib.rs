//! Headless Minecraft Java Edition client for the ghast library.
//!
//! - [`Client`]: connect (directly or through a SOCKS5 proxy, see [`ghast_auth::connect`]),
//!   handshake, login (offline; online mode with the `microsoft` feature), compression and
//!   encryption, then the play state: keep-alives, teleports, window confirms, health,
//!   respawn, chat.
//! - A fixed 50 ms tick loop with vanilla's tick order (see [`engine`](crate::TickContext)
//!   for the details): inbound packets → [`Behavior::tick`] (actions written immediately) →
//!   rotation → movement → exactly one movement packet (26.1 also: `player_loaded` once the
//!   level is in, `player_input`, and `tick_end` closing every tick).
//! - A version-agnostic API: behaviours see [`TickContext`] and the types of [`common`],
//!   never packets. The connection is generic over the protocol version
//!   ([`version::ClientVersion`]), so each supported version is monomorphised.
//! - Packets nobody needs are dropped before they are decoded; a behaviour opts into more
//!   with [`Behavior::interests`].
//!
//! Supported versions: 1.8.9 (protocol 47), 1.12.2 (340), 1.16.5 (754) and 26.1 – 26.1.2 (775),
//! one cargo feature each (`v1_8`, `v1_12_2`, `v1_16_5`, `v26_1`; default: all). What a version
//! can do is its [`version::Capabilities`] row ([`GameVersion::caps`]).
//!
//! ```no_run
//! use ghast_client::{Account, Behavior, Client, GameVersion, Interests, TickContext};
//!
//! struct Echo;
//! impl Behavior for Echo {
//!     fn interests(&self) -> Interests {
//!         Interests::CHAT
//!     }
//!     fn tick(&mut self, ctx: &mut TickContext<'_>) {
//!         let lines: Vec<String> = ctx.events().chat.iter().map(|m| m.plain()).collect();
//!         for line in lines {
//!             println!("chat: {line}");
//!         }
//!         if ctx.is_dead() {
//!             ctx.respawn();
//!         }
//!     }
//! }
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::new("localhost:25565".parse()?, GameVersion::V1_8_9, Account::offline("Steve")?);
//! let end = client.run(&mut Echo).await?;
//! println!("session ended: {end:?}");
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms, unreachable_pub, unnameable_types)]
// Without a version feature the client compiles (every connect fails with
// `ClientError::VersionNotEnabled`) but the per-version machinery is unused.
#![cfg_attr(
    not(any(
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    )),
    allow(dead_code, unused_variables, unused_imports)
)]

mod client;
pub mod common;
mod engine;
#[cfg(all(test, feature = "v1_8", feature = "v1_12_2", feature = "v1_16_5"))]
mod engine_bench;
mod error;
pub mod hands;
mod interest;
pub mod movement;
#[cfg(any(feature = "v1_8", feature = "v1_16_5"))]
mod record;
pub mod rotation;
mod sim;
#[cfg(all(test, feature = "v1_8"))]
mod trace_replay;
pub mod version;

pub use client::{Account, Client, TICK};
pub use common::{ChatMessage, ClientSettings, Controls, Face, GameMode, Health, Pose, Vec3};
pub use engine::{
    Behavior, ClickRefused, ClickTicket, Idle, RawInbound, TeleportEvent, TickContext, TickEvents,
    TickView,
};
pub use error::{ClientError, SessionEnd};
pub use ghast_entity as entity;
pub use ghast_inventory as inventory;
pub use ghast_protocol::BlockPos;
pub use ghast_world as world;
pub use hands::{BlockHit, Buttons};
pub use interest::Interests;
pub use sim::{ForceStepPosition, WorldView};
pub use version::GameVersion;

/// Re-exports of the connection setup types from `ghast-auth`.
pub mod connect {
    pub use ghast_auth::connect::{
        ConnectError, Connector, ProxyPolicy, Route, TargetAddr, Timeouts,
    };
    pub use ghast_auth::socks5::Socks5Proxy;
}
