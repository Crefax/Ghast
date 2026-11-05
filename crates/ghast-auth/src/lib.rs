//! Authentication and connection setup for the ghast Minecraft Java Edition client library.
//!
//! This crate covers everything that happens *before* the game protocol takes over a socket:
//!
//! * [`offline`]: offline-mode identities. [`offline::offline_uuid`] derives the same UUID that
//!   vanilla and Bukkit/Paper servers assign to an offline player.
//! * [`socks5`] and [`connect`]: an RFC 1928 / RFC 1929 SOCKS5 client (username/password auth,
//!   **remote DNS only**) and a [`connect::Connector`] that opens the game TCP connection, either
//!   directly or through a proxy, under a configurable [`connect::ProxyPolicy`].
//! * [`crypto`]: the client side of the login Encryption Request: a fresh shared secret, RSA
//!   PKCS#1 v1.5 encryption with the server's key and the session-server hash.
//! * `microsoft` (cargo feature `microsoft`, off by default): Microsoft account login through the
//!   OAuth device-code flow, Xbox Live, XSTS and Minecraft services, a token cache, and the
//!   session-server `join` call used by online-mode servers.
//!
//! # Example: offline identity over a SOCKS5 proxy
//!
//! ```no_run
//! use ghast_auth::connect::{Connector, ProxyPolicy, TargetAddr};
//! use ghast_auth::offline::OfflineProfile;
//! use ghast_auth::socks5::Socks5Proxy;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let profile = OfflineProfile::new("Steve")?;
//! println!("{} has offline UUID {}", profile.name(), profile.uuid());
//!
//! let proxy: Socks5Proxy = "socks5h://user:secret@proxy.example:1080".parse()?;
//! let connector = Connector::new()
//!     .with_proxy(proxy)
//!     .with_policy(ProxyPolicy::RequireProxyExceptLocal);
//!
//! // The domain name is sent to the proxy as-is; it is never resolved locally.
//! let target: TargetAddr = "mc.example.net:25565".parse()?;
//! let stream = connector.connect(&target).await?;
//! // `stream` is a `tokio::net::TcpStream` with TCP_NODELAY set, ready for the handshake packet.
//! # drop(stream);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms, unreachable_pub, unnameable_types)]

pub mod connect;
pub mod crypto;
pub mod offline;
pub mod socks5;

#[cfg(feature = "microsoft")]
pub mod microsoft;

mod secret;

pub use secret::SecretString;
