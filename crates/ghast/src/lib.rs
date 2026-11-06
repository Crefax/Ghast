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
//! | `ext` (feature `ext`) | `ghast-ext` | sequential async helpers: look, dig, place, equip, containers, chat, events |
//! | `pathfinder` (feature `pathfinder`) | `ghast-pathfinder` | physics-validated A* path planning and following |
//! | `viewer` (feature `viewer`) | `ghast-viewer` | bot screen: live 3D view in the browser, recordings, replays |
//!
//! Supported versions: 1.8.9 and 1.16.5. See the `afk` and `chat` examples:
//!
//! ```text
//! cargo run -p ghast --example afk -- --host localhost --port 25565 --username Bot --version 1.8.9
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, unnameable_types)]

pub use ghast_client::*;

/// The network protocol (`ghast-protocol`).
pub use ghast_protocol as protocol;

/// Connection setup and authentication (`ghast-auth`).
pub use ghast_auth as auth;

/// Static game data (`ghast-data`).
pub use ghast_data as data;

/// Sequential async helpers on the tick model (`ghast-ext`, feature `ext`). With the
/// `pathfinder` feature too, `Bot::goto` walks planned paths.
#[cfg(feature = "ext")]
pub use ghast_ext as ext;

/// Physics-validated path planning and following (`ghast-pathfinder`, feature `pathfinder`).
#[cfg(feature = "pathfinder")]
pub use ghast_pathfinder as pathfinder;

/// The bot screen: a live 3D view of the bot's world in the browser, with recordings and
/// replays (`ghast-viewer`, feature `viewer`). Textures come from Mojang's client jar, fetched
/// at run time.
#[cfg(feature = "viewer")]
pub use ghast_viewer as viewer;
