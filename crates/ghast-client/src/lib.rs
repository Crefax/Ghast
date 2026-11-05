//! Headless Minecraft Java Edition client for the ghast library.
//!
//! - [`Client`]: connect (directly or through a SOCKS5 proxy, see [`ghast_auth::connect`]),
//!   handshake, login (offline; online mode with the `microsoft` feature), compression and
//!   encryption, then the play state: keep-alives, teleports, window confirms, health,
//!   respawn, chat.
//! - A fixed 50 ms tick loop with vanilla 1.8.9's tick order (see [`engine`](crate::TickContext)
//!   for the details): inbound packets → [`Behavior::tick`] (actions written immediately) →
//!   rotation → movement → exactly one movement packet.
//! - A version-agnostic API: behaviours see [`TickContext`] and the types of [`common`],
//!   never packets. The connection is generic over the protocol version
//!   ([`version::ClientVersion`]), so each supported version is monomorphised.
//! - Packets nobody needs are dropped before they are decoded; a behaviour opts into more
//!   with [`Behavior::interests`].
//!
//! Supported versions: 1.8.9 (protocol 47) and 1.16.5 (protocol 754).
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

mod client;
pub mod common;
mod engine;
mod error;
pub mod hands;
mod interest;
pub mod movement;
mod record;
pub mod rotation;
mod sim;
#[cfg(test)]
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
