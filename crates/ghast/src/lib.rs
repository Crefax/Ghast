//! ghast: a headless Minecraft Java Edition client / bot library.
//!
//! This facade re-exports the bot API of [`ghast-client`](ghast_client) at the top level and
//! the lower layers as modules:
//!
//! | module | crate | what |
//! |---|---|---|
//! | (root) | `ghast-client` | [`Client`], [`Behavior`], [`TickContext`], the tick loop |
//! | [`protocol`] | `ghast-protocol` | packets per version, framing, compression, encryption, NBT |
//! | [`auth`] | `ghast-auth` | offline identities, SOCKS5, login crypto, Microsoft accounts |
//! | [`data`] | `ghast-data` | blocks, collision shapes, items, entities |
//!
//! Supported versions: 1.8.9 and 1.16.5. See the `afk` and `chat` examples:
//!
//! ```text
//! cargo run -p ghast --example afk -- --host localhost --port 25565 --username Bot --version 1.8.9
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub use ghast_client::*;

/// The network protocol (`ghast-protocol`).
pub use ghast_protocol as protocol;

/// Connection setup and authentication (`ghast-auth`).
pub use ghast_auth as auth;

/// Static game data (`ghast-data`).
pub use ghast_data as data;
