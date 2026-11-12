//! A bot screen for bots built on [ghast](https://crates.io/crates/ghast): a live 3D view of
//! the bot's world in the browser, plus recordings and replays.
//!
//! - [`Viewer`]: one per bot. [`Viewer::watch`] wraps the bot's behaviour so each client tick
//!   feeds the viewer (read-only: it never writes to the connection), [`Viewer::start`] serves
//!   the page over HTTP + WebSocket (127.0.0.1 by default).
//! - Zero cost while nobody watches: no frames are built and no columns copied; the server
//!   stops itself after [`ViewerConfig::idle`] without a page.
//! - Textures and models are taken at run time from the vanilla client jar (downloaded from
//!   Mojang's servers into a local cache, or [`ViewerConfig::client_jar`]); nothing of the game
//!   is shipped with this crate.
//! - [`FeedRecorder`] writes what a page would receive to disk; [`DashCam`] keeps the last
//!   minutes in memory and saves them on [`DashCam::clip`]; recordings play back on the same page
//!   (`/?replay=<name>`).
//!
//! ```no_run
//! use ghast_client::{Account, Client, GameVersion, Idle};
//! use ghast_viewer::{Viewer, ViewerConfig};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let viewer = Viewer::new(ViewerConfig::new("Steve").port(8080));
//! let addr = viewer.start().await?;
//! println!("open http://{addr}/");
//! let client = Client::new("127.0.0.1:25565".parse()?, GameVersion::V1_8_9, Account::offline("Steve")?)
//!     .world_config(ghast_viewer::world_config());
//! let mut behaviour = viewer.watch(Idle);
//! client.run(&mut behaviour).await?;
//! # Ok(())
//! # }
//! ```
//!
//! Wire protocol: `docs/protocol.md`. 1.8.9 and 26.1 (feature `v26_1`, mapped onto the 1.8.9
//! assets, see [`v26_1`]) sessions are drawn.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod assets;
pub mod blocks18;
pub mod dashcam;
pub mod entities;
pub mod feed;
pub mod hub;
#[cfg(any(feature = "v26_1", feature = "v26_2"))]
mod legacy_table;
pub mod protocol;
pub mod record;
pub mod replay;
mod server;
pub mod skins;
pub mod stream;
pub mod tiles;
#[cfg(feature = "v26_1")]
pub mod v26_1;
#[cfg(feature = "v26_2")]
pub mod v26_2;
mod watch;
mod window;

/// A modern version's mapping onto the page's 1.8 assets and its entity accessors (one constant
/// per modern version module, chosen once per session).
#[derive(Clone, Copy)]
pub(crate) struct ModernMap {
    pub(crate) block: fn(u32) -> u16,
    pub(crate) item: fn(i32, i16) -> (u16, u16),
    pub(crate) entity_text: fn(&ghast_entity::Entity) -> Option<serde_json::Value>,
    pub(crate) stack_text:
        for<'a> fn(&ghast_inventory::Stack, &dyn Fn(i32) -> Option<&'a str>) -> serde_json::Value,
    pub(crate) entity_name: fn(&str) -> String,
    pub(crate) info: fn(ghast_entity::EntityType) -> Option<&'static ghast_data::EntityInfo>,
    pub(crate) item_stack:
        fn(&ghast_entity::Entity) -> Option<&ghast_protocol::types::slot::v775::ItemStack>,
    pub(crate) flags: fn(&ghast_entity::Entity) -> u8,
    pub(crate) inventory: ghast_inventory::Version,
    /// The game version name (`"26.1"`).
    pub(crate) game: &'static str,
}

impl std::fmt::Debug for ModernMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModernMap")
            .field("inventory", &self.inventory)
            .finish_non_exhaustive()
    }
}

/// The modern mapping of a session's version (`None`: not a modern version, or its feature is
/// off).
pub(crate) fn modern_map(v: ghast_client::GameVersion) -> Option<&'static ModernMap> {
    match v {
        #[cfg(feature = "v26_1")]
        ghast_client::GameVersion::V26_1 => Some(&v26_1::MAP),
        #[cfg(feature = "v26_2")]
        ghast_client::GameVersion::V26_2 => Some(&v26_2::MAP),
        _ => None,
    }
}

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub use dashcam::{DashCam, DashCamOptions};
pub use feed::{BotView, now_ms};
pub use record::{FeedRecorder, ReplayMeta, list_recordings, prune_recordings};
pub use watch::Watched;

use crate::feed::FeedConfig;
use crate::hub::Hub;
use crate::skins::SkinStore;

/// Viewer options.
#[derive(Debug, Clone)]
pub struct ViewerConfig {
    /// Shown on the page (the bot's name).
    pub account: String,
    /// Listen address (default 127.0.0.1: put a proxy with access control in front for remote use).
    pub host: IpAddr,
    /// Port (0 = any free port).
    pub port: u16,
    /// Columns sent around the bot: a (2d − 1)² square, 1..=12 (default 8 → 15×15).
    pub view_distance: u8,
    /// Stop the server after this long without a page (default 60 s).
    pub idle: Duration,
    /// Required on every request as `Authorization: Bearer <token>`, `?token=<token>` (the page
    /// then keeps it in a cookie) or that cookie. `None`: no check (fine on 127.0.0.1).
    pub token: Option<String>,
    /// Recordings folder (`/replays`, `?replay=`); `None`: no replays.
    pub replay_dir: Option<PathBuf>,
    /// Cache folder for the client jar, extracted textures and built assets
    /// (default `.cache/ghast-viewer`).
    pub cache_dir: PathBuf,
    /// A local vanilla 1.8.9 client jar; `None`: downloaded from Mojang into the cache.
    pub client_jar: Option<PathBuf>,
    /// When to track entities, the tab list and skins for the page (default
    /// [`EntityTracking::Always`]).
    pub entities: EntityTracking,
}

impl ViewerConfig {
    /// Defaults for a bot named `account`.
    #[must_use]
    pub fn new(account: impl Into<String>) -> Self {
        ViewerConfig {
            account: account.into(),
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            view_distance: 8,
            idle: Duration::from_secs(60),
            token: None,
            replay_dir: None,
            cache_dir: PathBuf::from(".cache/ghast-viewer"),
            client_jar: None,
            entities: EntityTracking::Always,
        }
    }

    /// Sets the port.
    #[must_use]
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the listen address.
    #[must_use]
    pub fn host(mut self, host: IpAddr) -> Self {
        self.host = host;
        self
    }

    /// Sets the view distance (clamped to 1..=12).
    #[must_use]
    pub fn view_distance(mut self, d: u8) -> Self {
        self.view_distance = d.clamp(1, 12);
        self
    }

    /// Sets the access token.
    #[must_use]
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Sets the recordings folder.
    #[must_use]
    pub fn replay_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.replay_dir = Some(dir.into());
        self
    }

    /// Sets the cache folder.
    #[must_use]
    pub fn cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = dir.into();
        self
    }

    /// Uses a local client jar.
    #[must_use]
    pub fn client_jar(mut self, jar: impl Into<PathBuf>) -> Self {
        self.client_jar = Some(jar.into());
        self
    }

    /// Sets when entities are tracked.
    #[must_use]
    pub fn entities(mut self, e: EntityTracking) -> Self {
        self.entities = e;
        self
    }

    /// Sets the idle timeout.
    #[must_use]
    pub fn idle(mut self, d: Duration) -> Self {
        self.idle = d;
        self
    }

    fn feed(&self) -> FeedConfig {
        FeedConfig {
            account: self.account.clone(),
            view_distance: i32::from(self.view_distance.clamp(1, 12)),
            unbudgeted: false,
        }
    }
}

/// When the viewer has ghast track entities (`Interests::ENTITIES`, `PLAYER_LIST`,
/// `PLAYER_SKINS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntityTracking {
    /// For the whole session: a page opened later shows every entity at once.
    #[default]
    Always,
    /// Only while a page or recording is attached (no cost otherwise). Tracking starts empty
    /// when the first page opens: entities already around appear once the server sends them
    /// again (when they move into view or respawn).
    WhileWatched,
    /// Never: no entities on the page.
    Off,
}

/// The world settings a watched client should use (light for shading, biomes for grass and
/// leaf colours, block entities for sign text, skulls, banners and flower pots):
/// `Client::new(..).world_config(ghast_viewer::world_config())`.
#[must_use]
pub fn world_config() -> ghast_world::WorldConfig {
    ghast_world::WorldConfig::default()
        .with_light(true)
        .with_biomes(true)
        .with_block_entities(true)
}

/// Viewer state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ViewerStatus {
    pub running: bool,
    pub port: Option<u16>,
    pub viewers: usize,
    #[serde(rename = "inGame")]
    pub in_game: bool,
}

struct Inner {
    cfg: ViewerConfig,
    hub: Arc<Hub>,
    server: server::Control,
}

/// The bot screen of one bot. Cheap to clone (a handle).
#[derive(Clone)]
pub struct Viewer {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Viewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Viewer")
            .field("account", &self.inner.cfg.account)
            .field("status", &self.status())
            .finish()
    }
}

impl Viewer {
    /// A viewer (no server yet: see [`Viewer::start`]).
    #[must_use]
    pub fn new(cfg: ViewerConfig) -> Self {
        Self::with_options(cfg, false)
    }

    /// Tests: encode columns on the tick thread.
    #[doc(hidden)]
    #[must_use]
    pub fn new_inline(cfg: ViewerConfig) -> Self {
        Self::with_options(cfg, true)
    }

    fn with_options(cfg: ViewerConfig, inline: bool) -> Self {
        let mut feed = cfg.feed();
        feed.unbudgeted = inline;
        let hub = Hub::new(feed, Arc::clone(SkinStore::global()), inline);
        Viewer {
            inner: Arc::new(Inner {
                cfg,
                hub,
                server: server::Control::default(),
            }),
        }
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &ViewerConfig {
        &self.inner.cfg
    }

    /// The hub (feed and streams).
    #[must_use]
    pub fn hub(&self) -> &Arc<Hub> {
        &self.inner.hub
    }

    /// Wraps a behaviour so its ticks feed this viewer.
    pub fn watch<B>(&self, behavior: B) -> Watched<B> {
        Watched::new(behavior, self.clone())
    }

    /// Starts the server (idempotent; builds the assets on first use) and returns its address.
    pub async fn start(&self) -> std::io::Result<SocketAddr> {
        server::start(self).await
    }

    /// Stops the server (pages are disconnected; recordings go on).
    pub async fn stop(&self) {
        self.inner.server.stop().await;
    }

    /// The status line under the bot's name on the page, and the message shown while the bot
    /// is not in game (empty: the default).
    pub fn set_status(&self, status: &str, message: &str) {
        self.inner.hub.set_status(status, message);
    }

    /// Running, port, pages watching, in game.
    #[must_use]
    pub fn status(&self) -> ViewerStatus {
        ViewerStatus {
            running: self.inner.server.port().is_some(),
            port: self.inner.server.port(),
            viewers: self
                .inner
                .hub
                .viewers
                .load(std::sync::atomic::Ordering::Relaxed)
                + self.inner.server.replays(),
            in_game: self.inner.hub.in_game(),
        }
    }

    pub(crate) fn control(&self) -> &server::Control {
        &self.inner.server
    }
}
