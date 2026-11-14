//! Connection setup and the session driver: connect → handshake → login → play loop.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ghast_auth::connect::{Connector, TargetAddr};
use ghast_auth::offline::OfflineProfile;
use ghast_protocol::{FrameDecoder, FrameEncoder, PacketFilter, RawPacket, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{Instant, MissedTickBehavior};

use crate::common::{
    ClientSettings, ConfigInbound, ConfigOutbound, Inbound, KnownPack, LoginInbound, chat_to_plain,
};
use crate::engine::{Behavior, CoreOptions, Engine};
use crate::error::{ClientError, SessionEnd};
use crate::interest::Interests;
use crate::sim::{Sim, SimError};
#[cfg(feature = "v1_8")]
use crate::version::V1_8;
#[cfg(feature = "v1_12_2")]
use crate::version::V1_12_2;
#[cfg(feature = "v1_16_5")]
use crate::version::V1_16_5;
#[cfg(feature = "v26_1")]
use crate::version::V26_1;
#[cfg(feature = "v26_2")]
use crate::version::V26_2;
use crate::version::{ClientVersion, GameVersion, HasSim};

/// Length of one client tick.
pub const TICK: Duration = Duration::from_millis(50);
/// Minimum free space requested from the socket per read.
const READ_CHUNK: usize = 16 * 1024;

/// The account a client logs in with.
#[derive(Clone)]
#[non_exhaustive]
pub enum Account {
    /// Offline mode (the server does not authenticate; the UUID derives from the name).
    Offline(OfflineProfile),
    /// A Microsoft account session (needed for online-mode servers).
    #[cfg(feature = "microsoft")]
    Microsoft {
        /// The logged-in session (access token and profile).
        session: ghast_auth::microsoft::MinecraftSession,
        /// The client that makes the session-server `join` call.
        auth: Arc<ghast_auth::microsoft::MicrosoftAuth>,
    },
}

impl Account {
    /// An offline account (the name is checked like vanilla: 1–16 of `[A-Za-z0-9_]`).
    pub fn offline(name: &str) -> Result<Self, ClientError> {
        Ok(Account::Offline(OfflineProfile::new(name)?))
    }

    /// The player name.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Account::Offline(p) => p.name(),
            #[cfg(feature = "microsoft")]
            Account::Microsoft { session, .. } => &session.profile.name,
        }
    }

    /// The profile id (offline: derived from the name like vanilla's offline mode).
    #[must_use]
    pub fn uuid(&self) -> uuid::Uuid {
        match self {
            Account::Offline(p) => p.uuid(),
            #[cfg(feature = "microsoft")]
            Account::Microsoft { session, .. } => session.profile.id,
        }
    }
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Account::Offline(p) => f.debug_tuple("Offline").field(&p.name()).finish(),
            #[cfg(feature = "microsoft")]
            Account::Microsoft { session, .. } => f
                .debug_struct("Microsoft")
                .field("name", &session.profile.name)
                .finish_non_exhaustive(),
        }
    }
}

/// A configured client: where to connect, which version, which account, and how.
///
/// One `Client` can run any number of sessions one after another (reconnects); run several
/// clients concurrently for several bots (one per process, or many tasks in one process —
/// a failing session never affects another).
#[derive(Debug, Clone)]
pub struct Client {
    address: TargetAddr,
    version: GameVersion,
    account: Account,
    connector: Connector,
    settings: ClientSettings,
    brand: Option<String>,
    sensitivity: f32,
    silent_hold: Duration,
    read_timeout: Duration,
    login_timeout: Duration,
    record: Option<std::path::PathBuf>,
    force_step_position: crate::sim::ForceStepPosition,
    hold_turn_on_action: bool,
    entity_push: bool,
    world_config: Option<ghast_world::WorldConfig>,
    shared_chunks: Option<ghast_world::SharedChunks>,
    accept_required_packs: bool,
    signed_chat: bool,
    follow_transfers: bool,
    /// Profile keys for signed chat, shared by this client's sessions (fetched once, then
    /// reused until they are due for a refresh).
    profile_keys: Arc<std::sync::Mutex<Option<Arc<ProfileKeys>>>>,
}

use ghast_auth::chat::ProfileKeys;

impl Client {
    /// A client with vanilla defaults: direct connection (see [`Client::connector`]),
    /// brand `vanilla`, sensitivity 0.5, 30 s read timeout.
    #[must_use]
    pub fn new(address: TargetAddr, version: GameVersion, account: Account) -> Self {
        Client {
            address,
            version,
            account,
            connector: Connector::new(),
            settings: ClientSettings::default(),
            brand: Some("vanilla".to_owned()),
            sensitivity: 0.5,
            silent_hold: Duration::from_millis(1500),
            read_timeout: Duration::from_secs(30),
            login_timeout: Duration::from_secs(30),
            record: None,
            force_step_position: crate::sim::ForceStepPosition::Off,
            hold_turn_on_action: true,
            entity_push: true,
            world_config: None,
            shared_chunks: None,
            accept_required_packs: true,
            signed_chat: true,
            follow_transfers: false,
            profile_keys: Arc::default(),
        }
    }

    /// Signed chat on encrypted (online-mode) connections, 1.19.3+ (default on, like vanilla):
    /// the client gets the account's profile key pair (with the `microsoft` feature from the
    /// Minecraft services, or the one given to [`Client::profile_keys`]), announces a chat
    /// session after each join and signs chat messages and signable command arguments. Off:
    /// everything is sent unsigned (servers with `enforce-secure-profile` then refuse chat).
    /// Offline-mode connections never sign (vanilla has no key pair there).
    #[must_use]
    pub fn signed_chat(mut self, on: bool) -> Self {
        self.signed_chat = on;
        self
    }

    /// Follow server transfers (1.20.5+ `transfer`, default off): the client closes the
    /// connection and logs in to the target with the transfer intent, carrying the cookies the
    /// server stored, within the same [`Client::run`] (the behaviour stops and starts again, as
    /// on a reconnect; at most 8 transfers in a row). Off, the session ends with
    /// [`SessionEnd::Transferred`] and the application decides (vanilla always follows; a
    /// library should not let a server move a bot to any host, through its proxy, unasked).
    #[must_use]
    pub fn follow_transfers(mut self, on: bool) -> Self {
        self.follow_transfers = on;
        self
    }

    /// Uses this profile key pair for signed chat instead of fetching one (for example a pair
    /// kept from an earlier run; it must belong to the account's profile id).
    #[must_use]
    pub fn profile_keys(self, keys: ProfileKeys) -> Self {
        if let Ok(mut k) = self.profile_keys.lock() {
            *k = Some(Arc::new(keys));
        }
        self
    }

    /// How to open the TCP connection (SOCKS5 proxy, proxy policy, timeouts).
    #[must_use]
    pub fn connector(mut self, connector: Connector) -> Self {
        self.connector = connector;
        self
    }

    /// Client settings sent after joining.
    #[must_use]
    pub fn settings(mut self, settings: ClientSettings) -> Self {
        self.settings = settings;
        self
    }

    /// The client brand sent after joining (`None`: send none).
    #[must_use]
    pub fn brand(mut self, brand: Option<String>) -> Self {
        self.brand = brand;
        self
    }

    /// Mouse sensitivity (options.txt value, 0 … 1) for rotation steps.
    #[must_use]
    pub fn sensitivity(mut self, sensitivity: f32) -> Self {
        self.sensitivity = sensitivity;
        self
    }

    /// The behaviour is frozen after this long without data from the server (default 1.5 s).
    #[must_use]
    pub fn silent_hold(mut self, d: Duration) -> Self {
        self.silent_hold = d;
        self
    }

    /// The session fails after this long without data from the server (default 30 s, like
    /// vanilla's read timeout).
    #[must_use]
    pub fn read_timeout(mut self, d: Duration) -> Self {
        self.read_timeout = d;
        self
    }

    /// Connecting plus login must finish within this time (default 30 s).
    #[must_use]
    pub fn login_timeout(mut self, d: Duration) -> Self {
        self.login_timeout = d;
        self
    }

    /// Records every session's movement in the oracle's replay format (1.8.9; the file is
    /// overwritten by each session). For debugging anticheat flags against vanilla.
    #[must_use]
    pub fn record(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.record = Some(path.into());
        self
    }

    /// What the world keeps besides block states. Default: nothing (no light, no biomes:
    /// the bot does not need them, and they cost memory and decoding time). A viewer that
    /// draws the world can turn them on; [`crate::WorldView::column`] gives the data.
    #[must_use]
    pub fn world_config(mut self, config: ghast_world::WorldConfig) -> Self {
        self.world_config = Some(config);
        self
    }

    /// Share the block storage of chunk sections with every other client given the same
    /// store (see [`ghast_world::SharedChunks`]): bots in the same area then hold one copy of
    /// each identical section instead of one each. Reads stay as fast as without it; a block
    /// change copies only the section it touches, for this bot only. Opt-in (default: every
    /// bot owns its sections). Used by the modern versions (26.1, 26.2); the older versions
    /// ignore it.
    ///
    /// ```no_run
    /// use ghast_client::{Account, Client, GameVersion, world::SharedChunks};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let cache = SharedChunks::new(); // one store for all bots of the process
    /// let mut clients = Vec::new();
    /// for i in 0..10 {
    ///     let client = Client::new(
    ///         "localhost:25565".parse()?,
    ///         GameVersion::V26_2,
    ///         Account::offline(&format!("Bot{i}"))?,
    ///     )
    ///     .shared_chunks(&cache);
    ///     clients.push(client);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn shared_chunks(mut self, cache: &ghast_world::SharedChunks) -> Self {
        self.shared_chunks = Some(cache.clone());
        self
    }

    /// Keep the look still on a tick that dug or placed when this tick's turn would take the
    /// crosshair off the clicked block (on by default). Grim's
    /// RotationPlace re-checks a dig / place with the look of the movement packet that follows
    /// it; holding the mouse for one tick is what a player does anyway. The held turn
    /// continues on the next tick the behaviour aims.
    #[must_use]
    pub fn hold_turn_on_action(mut self, on: bool) -> Self {
        self.hold_turn_on_action = on;
        self
    }

    /// Whether other entities push the player (default on, like vanilla). Pushes happen only
    /// with [`Interests::ENTITIES`] (26.1+: mobs, players and boats around the player push it
    /// in the client's entity tick; 1.8.9 – 1.16.5 clients are never pushed by the client, so
    /// the switch is a no-op there). **Off is not vanilla**: a vanilla client always applies
    /// these pushes itself; off keeps entity tracking (crosshair, attack, interact) without
    /// them. A behaviour can switch it at run time with [`TickContext::set_entity_push`](crate::TickContext::set_entity_push).
    #[must_use]
    pub fn entity_push(mut self, on: bool) -> Self {
        self.entity_push = on;
        self
    }

    /// **Not vanilla** (Grim 2.3 compatibility, off by default): on a tick whose feet height
    /// changed (a step-up or landing) but whose move stayed under the 0.03-block resend
    /// threshold, send the position anyway. Vanilla sends only the look / on-ground there,
    /// which Grim 2.3.73 sets back (soul sand → grass hops).
    #[must_use]
    pub fn force_step_position(mut self, on: bool) -> Self {
        self.force_step_position = if on {
            crate::sim::ForceStepPosition::Always
        } else {
            crate::sim::ForceStepPosition::Off
        };
        self
    }

    /// **Not vanilla**: [`Client::force_step_position`] with a choice of rule, e.g.
    /// [`ForceStepPosition::StepUpOnly`](crate::ForceStepPosition::StepUpOnly), which forces the
    /// position only on step-ups (on ground after the move, feet higher) and leaves sub-0.03
    /// falls and landings vanilla.
    #[must_use]
    pub fn force_step_position_mode(mut self, mode: crate::sim::ForceStepPosition) -> Self {
        self.force_step_position = mode;
        self
    }

    /// Server resource packs (1.20.3+): optional packs are always declined (the client never
    /// shows the prompt); a pack the server requires is reported accepted and loaded when this
    /// is on (default), declined otherwise (the server then usually disconnects). Nothing is
    /// downloaded.
    #[must_use]
    pub fn accept_required_resource_packs(mut self, on: bool) -> Self {
        self.accept_required_packs = on;
        self
    }

    /// The server address.
    #[must_use]
    pub fn address(&self) -> &TargetAddr {
        &self.address
    }

    /// The protocol version.
    #[must_use]
    pub fn version(&self) -> GameVersion {
        self.version
    }

    /// The account.
    #[must_use]
    pub fn account(&self) -> &Account {
        &self.account
    }

    /// Runs one session with `behavior` until the server ends it, the behaviour quits or an
    /// error occurs.
    pub async fn run<B: Behavior>(&self, behavior: &mut B) -> Result<SessionEnd, ClientError> {
        self.run_until(behavior, std::future::pending()).await
    }

    /// Like [`Client::run`], but also ends cleanly (the behaviour is stopped, the socket shut
    /// down) when `shutdown` completes.
    pub async fn run_until<B, S>(
        &self,
        behavior: &mut B,
        shutdown: S,
    ) -> Result<SessionEnd, ClientError>
    where
        B: Behavior,
        S: Future<Output = ()>,
    {
        let shutdown = std::pin::pin!(shutdown);
        // The version is chosen once: everything below is monomorphised per version.
        match self.version {
            #[cfg(feature = "v1_8")]
            GameVersion::V1_8_9 => sessions::<V1_8, B, S>(self, behavior, shutdown).await,
            #[cfg(feature = "v1_12_2")]
            GameVersion::V1_12_2 => sessions::<V1_12_2, B, S>(self, behavior, shutdown).await,
            #[cfg(feature = "v1_16_5")]
            GameVersion::V1_16_5 => sessions::<V1_16_5, B, S>(self, behavior, shutdown).await,
            #[cfg(feature = "v26_1")]
            GameVersion::V26_1 => sessions::<V26_1, B, S>(self, behavior, shutdown).await,
            #[cfg(feature = "v26_2")]
            GameVersion::V26_2 => sessions::<V26_2, B, S>(self, behavior, shutdown).await,
            #[allow(unreachable_patterns)]
            v => {
                let _ = (behavior, shutdown);
                Err(ClientError::VersionNotEnabled(v))
            }
        }
    }
}

/// Reads more bytes into the decoder. Cancel safe.
async fn read_more<R: AsyncRead + Unpin>(
    rd: &mut R,
    dec: &mut FrameDecoder,
) -> std::io::Result<usize> {
    let want = dec.needed().unwrap_or(0).max(READ_CHUNK);
    let buf = dec.buffer_mut();
    buf.reserve(want);
    let old = buf.len();
    let n = rd.read_buf(buf).await?;
    dec.commit(old);
    Ok(n)
}

/// Writes everything the encoder has queued.
async fn flush<W: AsyncWrite + Unpin>(wr: &mut W, enc: &mut FrameEncoder) -> std::io::Result<()> {
    let out = enc.out_mut();
    if !out.is_empty() {
        wr.write_all_buf(out).await?;
    }
    Ok(())
}

/// A connection that follows a server transfer: the target and the carried cookies.
struct Hop {
    address: TargetAddr,
    cookies: Vec<(ghast_protocol::types::ByteString, bytes::Bytes)>,
}

/// Most transfers followed in a row.
const MAX_TRANSFERS: usize = 8;

/// One session, then the sessions of the transfers it follows.
async fn sessions<V, B, S>(
    client: &Client,
    behavior: &mut B,
    mut shutdown: Pin<&mut S>,
) -> Result<SessionEnd, ClientError>
where
    V: ClientVersion + HasSim,
    B: Behavior,
    S: Future<Output = ()>,
{
    let mut hop = None;
    for n in 0.. {
        let (end, cookies) =
            session::<V, B, S>(client, behavior, shutdown.as_mut(), hop.take()).await?;
        match end {
            SessionEnd::Transferred { host, port }
                if client.follow_transfers && n < MAX_TRANSFERS =>
            {
                match TargetAddr::new(&host, port) {
                    Ok(address) => {
                        log::info!(
                            "[{}] following the transfer to {host}:{port}",
                            client.account.name()
                        );
                        hop = Some(Hop { address, cookies });
                    }
                    Err(e) => {
                        log::warn!(
                            "[{}] cannot follow the transfer to {host}:{port}: {e}",
                            client.account.name()
                        );
                        return Ok(SessionEnd::Transferred { host, port });
                    }
                }
            }
            end => return Ok(end),
        }
    }
    unreachable!()
}

async fn session<V, B, S>(
    client: &Client,
    behavior: &mut B,
    mut shutdown: Pin<&mut S>,
    hop: Option<Hop>,
) -> Result<
    (
        SessionEnd,
        Vec<(ghast_protocol::types::ByteString, bytes::Bytes)>,
    ),
    ClientError,
>
where
    V: ClientVersion + HasSim,
    B: Behavior,
    S: Future<Output = ()>,
{
    let name: Arc<str> = client.account.name().into();
    let mut dec = FrameDecoder::new();
    let mut enc = FrameEncoder::new();
    let mut wire = V::Wire::default();
    let transfer = hop.is_some();
    let address = match hop {
        Some(h) => {
            V::put_cookies(&mut wire, h.cookies);
            h.address
        }
        None => client.address.clone(),
    };
    // Configuration-state registries, applied to the world once it exists.
    let mut registries = Vec::new();
    // The connection is encrypted (vanilla starts a chat session only then).
    let mut encrypted = false;

    let login = async {
        let stream = client.connector.connect(&address).await?;
        let (mut rd, mut wr) = stream.into_split();
        login::<V, _, _, B>(
            client,
            &mut *behavior,
            &mut rd,
            &mut wr,
            &mut dec,
            &mut enc,
            &mut wire,
            &mut registries,
            &mut encrypted,
            (&address, transfer),
        )
        .await
        .map(|end| (rd, wr, end))
    };
    let (rd, wr, end) = tokio::select! {
        r = tokio::time::timeout(client.login_timeout, login) => match r {
            Ok(r) => r?,
            Err(_) => return Err(ClientError::Timeout("connect and login")),
        },
        () = shutdown.as_mut() => return Ok((SessionEnd::Shutdown, Vec::new())),
    };
    if let ConfigEnd::Transfer { host, port } = end {
        // Sent on during the configuration state: the connection closes.
        drop((rd, wr));
        return Ok((
            SessionEnd::Transferred { host, port },
            V::take_cookies(&mut wire),
        ));
    }
    log::info!("[{name}] logged in to {address} ({})", V::GAME);

    let opts = CoreOptions {
        name,
        caps: V::GAME.caps(),
        settings: client.settings.clone(),
        brand: client.brand.clone(),
        sensitivity: client.sensitivity,
        silent_hold: client.silent_hold,
        inventory: V::INVENTORY,
        hold_turn_on_action: client.hold_turn_on_action,
        accept_required_packs: client.accept_required_packs,
        entity_push: client.entity_push,
    };
    let mut play = Play::<V, _, _> {
        rd,
        wr,
        dec,
        enc,
        engine: Engine::new(
            opts,
            V::new_sim(&crate::sim::SimOptions {
                record: client.record.clone(),
                force_step_position: client.force_step_position,
                entities: entity_config(behavior.interests()),
                world: client.world_config,
                shared_chunks: client.shared_chunks.clone(),
                profile_name: Some(client.account.name().to_string()),
            }),
        ),
        read_timeout: client.read_timeout,
        interests: Interests::NONE,
        raw_ids: Vec::new(),
        raw_mask: [0; 4],
        world_errors: 0,
        wire,
        client,
        reconfigure: false,
        encrypted,
        keys_task: None,
        keys_checked: None,
    };
    for raw in &registries {
        play.engine.sim.registry_data(raw);
    }
    drop(registries);
    V::configure_inventory(play.engine.inventory_mut(), &play.wire);
    V::feed_inventory_config(play.engine.inventory_mut(), &mut play.wire);
    let result = play.run(behavior, shutdown).await;
    let reason = match &result {
        Ok(SessionEnd::Kicked { reason, .. }) => format!("kicked: {reason}"),
        Ok(SessionEnd::ServerClosed) => "server closed the connection".to_owned(),
        Ok(SessionEnd::Quit(r)) => format!("quit: {r}"),
        Ok(SessionEnd::Shutdown) => "shutdown".to_owned(),
        Ok(SessionEnd::Transferred { host, port }) => format!("transferred to {host}:{port}"),
        Err(e) => format!("error: {e}"),
    };
    play.engine.stop(behavior, &reason);
    let stats = play.dec.stats();
    log::info!(
        "[{}] session ended after {} ticks ({reason}); frames {} skipped {} (compressed {}), inflated {} B",
        play.engine.core_name(),
        play.engine.tick_count(),
        stats.frames,
        stats.skipped,
        stats.skipped_compressed,
        stats.inflated_bytes
    );
    if matches!(result, Ok(SessionEnd::Transferred { .. })) {
        play.close().await;
    }
    let cookies = V::take_cookies(&mut play.wire);
    result.map(|end| (end, cookies))
}

/// How a configuration phase ended.
enum ConfigEnd {
    /// Finished: the connection is in the play state.
    Play,
    /// The server sent a transfer.
    Transfer {
        /// Target host.
        host: String,
        /// Target port.
        port: u16,
    },
}

/// Handshake and login (and on 1.20.2+ the configuration state). Returns once the
/// connection is in the play state.
#[allow(clippy::too_many_arguments)] // the session's connection state, borrowed piecewise
async fn login<V, R, W, B>(
    client: &Client,
    behavior: &mut B,
    rd: &mut R,
    wr: &mut W,
    dec: &mut FrameDecoder,
    enc: &mut FrameEncoder,
    wire: &mut V::Wire,
    registries: &mut Vec<RawPacket>,
    encrypted: &mut bool,
    (address, transfer): (&TargetAddr, bool),
) -> Result<ConfigEnd, ClientError>
where
    V: ClientVersion,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    B: Behavior,
{
    if transfer {
        V::write_transfer_handshake(enc, &address.host(), address.port())?;
    } else {
        V::write_handshake(enc, &address.host(), address.port())?;
    }
    V::write_login_start(enc, client.account.name(), client.account.uuid())?;
    flush(wr, enc).await?;
    loop {
        while let Some(raw) = dec.next_packet()? {
            let p = V::decode_login(&raw).map_err(|source| ClientError::Decode {
                state: State::Login,
                id: raw.id,
                source,
            })?;
            match p {
                None => {}
                Some(LoginInbound::Disconnect(reason)) => {
                    return Err(ClientError::LoginRefused(chat_to_plain(&reason)));
                }
                Some(LoginInbound::Compress(threshold)) => {
                    dec.set_compression(threshold);
                    enc.set_compression(threshold);
                }
                Some(LoginInbound::PluginRequest {
                    message_id,
                    channel,
                }) => {
                    log::debug!("login plugin request {message_id} on {channel}: not understood");
                    V::write_plugin_response(enc, message_id)?;
                    flush(wr, enc).await?;
                }
                Some(LoginInbound::EncryptionRequest {
                    server_id,
                    public_key,
                    verify_token,
                }) => {
                    let secret =
                        encryption::<V>(client, enc, &server_id, &public_key, &verify_token)
                            .await?;
                    flush(wr, enc).await?;
                    enc.enable_encryption(&secret);
                    dec.enable_encryption(&secret);
                    *encrypted = true;
                }
                Some(LoginInbound::EncryptionRequestUnauthenticated {
                    server_id,
                    public_key,
                    verify_token,
                }) => {
                    // handleHello without shouldAuthenticate: no session-server join.
                    let resp = ghast_auth::crypto::respond_to_encryption_request(
                        &server_id,
                        &public_key,
                        &verify_token,
                    )?;
                    V::write_encryption_response(
                        enc,
                        &resp.encrypted_shared_secret,
                        &resp.encrypted_verify_token,
                    )?;
                    flush(wr, enc).await?;
                    enc.enable_encryption(resp.shared_secret.as_bytes());
                    dec.enable_encryption(resp.shared_secret.as_bytes());
                    *encrypted = true;
                }
                Some(LoginInbound::CookieRequest(key)) => {
                    V::write_login_cookie_response_from(enc, &key, wire)?;
                    flush(wr, enc).await?;
                }
                Some(LoginInbound::Success { uuid, username }) => {
                    log::debug!("login success: {username} {uuid:?}");
                    if V::GAME.caps().configuration_state {
                        // ClientHandshakePacketListenerImpl.handleLoginFinished: acknowledge,
                        // then the brand and the client information.
                        V::write_login_acknowledged(enc)?;
                        if let Some(brand) = &client.brand {
                            let b = ConfigOutbound::Brand(brand.clone());
                            V::write_configuration(enc, &b, wire)?;
                        }
                        let settings = ConfigOutbound::Settings(client.settings.clone());
                        V::write_configuration(enc, &settings, wire)?;
                        flush(wr, enc).await?;
                        return configure::<V, _, _, B>(
                            client, behavior, rd, wr, dec, enc, wire, registries,
                        )
                        .await;
                    }
                    return Ok(ConfigEnd::Play);
                }
            }
        }
        if read_more(rd, dec).await? == 0 {
            return Err(ClientError::LoginClosed);
        }
    }
}

/// The built-in data packs of the vanilla client (`KnownPacksManager`): core and the feature
/// packs, all in the `minecraft` namespace.
const VANILLA_PACKS: &[&str] = &[
    "core",
    "minecart_improvements",
    "redstone_experiments",
    "trade_rebalance",
];

/// The configuration state (1.20.2+), until the server finishes it. Registry data is collected
/// into `registries` (raw) for the world.
#[allow(clippy::too_many_arguments)] // the session's connection state, borrowed piecewise
async fn configure<V, R, W, B>(
    client: &Client,
    behavior: &mut B,
    rd: &mut R,
    wr: &mut W,
    dec: &mut FrameDecoder,
    enc: &mut FrameEncoder,
    wire: &mut V::Wire,
    registries: &mut Vec<RawPacket>,
) -> Result<ConfigEnd, ClientError>
where
    V: ClientVersion,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    B: Behavior,
{
    let family = V::GAME.protocol_info().game_versions;
    loop {
        while let Some(raw) = dec.next_packet()? {
            let p = V::decode_configuration(&raw, wire).map_err(|source| ClientError::Decode {
                state: State::Configuration,
                id: raw.id,
                source,
            })?;
            let Some(p) = p else { continue };
            let reply = match p {
                ConfigInbound::KeepAlive(id) => vec![ConfigOutbound::KeepAlive(id)],
                ConfigInbound::Ping(id) => vec![ConfigOutbound::Pong(id)],
                ConfigInbound::Finish => {
                    V::write_configuration(enc, &ConfigOutbound::Finish, wire)?;
                    flush(wr, enc).await?;
                    log::debug!("configuration done ({} registries)", registries.len());
                    return Ok(ConfigEnd::Play);
                }
                ConfigInbound::Disconnect(reason) => return Err(ClientError::LoginRefused(reason)),
                ConfigInbound::SelectKnownPacks(packs) => {
                    // The vanilla packs of this version family (the server's own patch).
                    let known: Vec<KnownPack> = packs
                        .into_iter()
                        .filter(|k| {
                            &*k.namespace == "minecraft"
                                && VANILLA_PACKS.contains(&&*k.id)
                                && family.contains(&&*k.version)
                        })
                        .collect();
                    vec![ConfigOutbound::SelectKnownPacks(known)]
                }
                ConfigInbound::RegistryData(raw) => {
                    V::note_registry(&raw, wire);
                    registries.push(raw);
                    continue;
                }
                ConfigInbound::ResourcePackPush { id, required } => {
                    if required && client.accept_required_packs {
                        [3, 4, 0]
                            .into_iter()
                            .map(|result| ConfigOutbound::ResourcePackStatus { id, result })
                            .collect()
                    } else {
                        vec![ConfigOutbound::ResourcePackStatus { id, result: 1 }]
                    }
                }
                ConfigInbound::CookieRequest(key) => vec![ConfigOutbound::CookieResponse { key }],
                ConfigInbound::CodeOfConduct(text) => {
                    log::info!("server code of conduct (accepted): {text}");
                    vec![ConfigOutbound::AcceptCodeOfConduct]
                }
                ConfigInbound::Transfer(to) => {
                    let (host, port) = to.rsplit_once(':').unwrap_or((&to, "25565"));
                    return Ok(ConfigEnd::Transfer {
                        host: host.to_owned(),
                        port: port.parse().unwrap_or(25565),
                    });
                }
                ConfigInbound::ShowDialog(d) => {
                    configuration_dialog(client.account.name(), behavior, *d)
                }
                // A dialog left open closes; nothing is sent.
                ConfigInbound::ClearDialog => continue,
            };
            for o in &reply {
                V::write_configuration(enc, o, wire)?;
            }
            flush(wr, enc).await?;
        }
        if read_more(rd, dec).await? == 0 {
            return Err(ClientError::LoginClosed);
        }
    }
}

/// A dialog shown during the configuration state: the behaviour's answers
/// ([`Behavior::configuration_dialog`]) as the packets vanilla sends there. Commands do nothing
/// in this state (vanilla logs them); a dialog an answer opens is offered again (at most 16 in a
/// row).
fn configuration_dialog<B: Behavior>(
    name: &str,
    behavior: &mut B,
    dialog: crate::dialog::Dialog,
) -> Vec<ConfigOutbound> {
    use crate::dialog::{Effect, Next};
    let mut out = Vec::new();
    if !behavior.interests().contains(Interests::DIALOGS) {
        log::warn!(
            "[{name}] the server shows a dialog during configuration ({:?}, \"{}\"); no behaviour \
             answers dialogs (Interests::DIALOGS), so the server may wait for an answer",
            dialog.kind,
            dialog.title
        );
        return out;
    }
    log::info!("[{name}] configuration dialog \"{}\"", dialog.title);
    let mut current = dialog;
    for _ in 0..16 {
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            behavior.configuration_dialog(&current)
        }));
        let answer = match answer {
            Ok(Some(a)) => a,
            Ok(None) => break,
            Err(_) => {
                log::error!(
                    "[{name}] behaviour {} panicked in configuration_dialog",
                    behavior.name()
                );
                break;
            }
        };
        let (effect, next) = match crate::dialog::resolve(&current, &answer) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("[{name}] configuration dialog answer refused: {e}");
                break;
            }
        };
        match effect {
            Effect::Nothing => {}
            Effect::Custom { id, payload } => out.push(ConfigOutbound::CustomClickAction {
                id,
                payload: crate::dialog::payload_nbt(payload.as_ref()),
            }),
            Effect::Command(c) => {
                // ClientConfigurationPacketListenerImpl: the dialog stays open.
                log::warn!("[{name}] commands are not supported in configuration phase: {c:?}");
                break;
            }
        }
        match next {
            Next::Open(d) => current = (*d).clone(),
            _ => break,
        }
    }
    out
}

/// Answers an encryption request; returns the shared secret to enable.
// `V` is only used for online mode.
#[cfg_attr(
    not(feature = "microsoft"),
    allow(clippy::extra_unused_type_parameters)
)]
async fn encryption<V: ClientVersion>(
    client: &Client,
    enc: &mut FrameEncoder,
    server_id: &str,
    public_key: &[u8],
    verify_token: &[u8],
) -> Result<[u8; 16], ClientError> {
    match &client.account {
        Account::Offline(_) => {
            let _ = (enc, server_id, public_key, verify_token);
            Err(ClientError::OnlineModeRequired)
        }
        #[cfg(feature = "microsoft")]
        Account::Microsoft { session, auth } => {
            let resp = ghast_auth::crypto::respond_to_encryption_request(
                server_id,
                public_key,
                verify_token,
            )?;
            auth.join_server(&session.access_token, session.profile.id, &resp.server_hash)
                .await?;
            V::write_encryption_response(
                enc,
                &resp.encrypted_shared_secret,
                &resp.encrypted_verify_token,
            )?;
            Ok(*resp.shared_secret.as_bytes())
        }
    }
}

/// The play state of one session.
struct Play<'c, V: ClientVersion + HasSim, R, W> {
    rd: R,
    wr: W,
    dec: FrameDecoder,
    enc: FrameEncoder,
    engine: Engine<V::Sim>,
    read_timeout: Duration,
    interests: Interests,
    /// Packet ids copied for the behaviour (`Behavior::raw_packets`), as a bit set.
    raw_ids: Vec<i32>,
    raw_mask: [u64; 4],
    /// World packets the world rejected (malformed chunk data).
    world_errors: u64,
    /// The version's wire state (sequence numbers, chat acknowledgements).
    wire: V::Wire,
    client: &'c Client,
    /// The server moved the connection to the configuration state (1.20.2+).
    reconfigure: bool,
    /// The connection is encrypted (signed chat needs it).
    encrypted: bool,
    /// A profile key pair request in flight.
    keys_task: Option<KeysTask<'c>>,
    /// When the key pair was last checked for a refresh (vanilla: at most hourly).
    keys_checked: Option<Instant>,
}

/// A profile key pair request (`ProfileKeyPairManager.prepareKeyPair`).
type KeysTask<'c> = Pin<Box<dyn Future<Output = Result<ProfileKeys, String>> + Send + 'c>>;

/// The entity tracking the interests ask for: positions and metadata (item stacks, sneak /
/// pose flags); equipment and other entities' effects are not decoded (cheaper).
fn entity_config(interests: Interests) -> Option<ghast_entity::Config> {
    interests.contains(Interests::ENTITIES).then(|| {
        ghast_entity::Config::default()
            .with_metadata(true)
            .with_equipment(false)
            .with_effects(false)
            .with_player_list(interests.contains(Interests::PLAYER_LIST))
            .with_events(false)
            .with_skins(interests.contains(Interests::PLAYER_SKINS))
    })
}

/// What woke the play loop.
enum Wake {
    Read(std::io::Result<usize>),
    Tick,
    Shutdown,
    Keys(Box<Result<ProfileKeys, String>>),
}

impl<'c, V, R, W> Play<'c, V, R, W>
where
    V: ClientVersion + HasSim,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    /// The cached key pair, if any.
    fn cached_keys(&self) -> Option<Arc<ProfileKeys>> {
        self.client.profile_keys.lock().ok().and_then(|k| k.clone())
    }

    /// handleLogin on an online-mode connection (26.1: encrypted; 26.2+: the login packet's
    /// `onlineMode`): `prepareKeyPair` — the cached pair right away unless it is due for a
    /// refresh, else a request.
    fn on_join(&mut self) {
        if !V::online_mode(&self.wire, self.encrypted) || !self.client.signed_chat {
            return;
        }
        match self.cached_keys() {
            Some(k) if !k.due_refresh() => self.start_chat_session(&k),
            cached => {
                if !self.request_keys()
                    && let Some(k) = cached
                {
                    // Nothing to refresh with (no account service): keep using it.
                    self.start_chat_session(&k);
                }
            }
        }
    }

    fn start_chat_session(&mut self, keys: &Arc<ProfileKeys>) {
        if V::begin_chat_session(&mut self.wire, keys, self.client.account.uuid()) {
            log::debug!("[{}] chat session started", self.engine.name());
            self.engine.request_chat_session();
        }
    }

    /// Starts a key pair request if the account can make one; `false` if not.
    fn request_keys(&mut self) -> bool {
        if self.keys_task.is_some() {
            return true;
        }
        self.keys_checked = Some(Instant::now());
        match &self.client.account {
            Account::Offline(_) => false,
            #[cfg(feature = "microsoft")]
            Account::Microsoft { session, auth } => {
                let (auth, token) = (auth.clone(), session.access_token.clone());
                self.keys_task = Some(Box::pin(async move {
                    auth.profile_keys(&token).await.map_err(|e| e.to_string())
                }));
                true
            }
        }
    }

    /// The key pair request finished.
    fn keys_done(&mut self, r: Result<ProfileKeys, String>) {
        self.keys_task = None;
        match r {
            Ok(k) => {
                let k = Arc::new(k);
                if let Ok(mut c) = self.client.profile_keys.lock() {
                    *c = Some(k.clone());
                }
                if self.engine.joined() {
                    self.start_chat_session(&k);
                }
            }
            Err(e) => {
                // Vanilla logs it and keeps the cached pair (chat stays unsigned without one).
                log::warn!(
                    "[{}] profile key pair request failed: {e}",
                    self.engine.name()
                );
                if let Some(k) = self.cached_keys()
                    && self.engine.joined()
                {
                    self.start_chat_session(&k);
                }
            }
        }
    }

    /// ClientPacketListener.tick: `shouldRefreshKeyPair` (at most hourly, once due).
    fn maybe_refresh_keys(&mut self) {
        if self.keys_task.is_some()
            || !V::online_mode(&self.wire, self.encrypted)
            || !self.client.signed_chat
        {
            return;
        }
        let hour = Duration::from_secs(3600);
        if self.keys_checked.is_some_and(|t| t.elapsed() < hour) {
            return;
        }
        if self.cached_keys().is_some_and(|k| k.due_refresh()) {
            self.request_keys();
        } else {
            self.keys_checked = Some(Instant::now());
        }
    }

    fn set_filter(&mut self, interests: Interests, raw: &[i32]) {
        self.interests = interests;
        raw.clone_into(&mut self.raw_ids);
        self.raw_mask = [0; 4];
        for &id in raw {
            if (0..256).contains(&id) {
                self.raw_mask[id as usize >> 6] |= 1 << (id & 63);
            }
        }
        self.engine
            .sim
            .collect_block_updates(interests.contains(Interests::BLOCK_UPDATES));
        // Entity tracking follows the interests at run time (a tracker started late begins
        // empty: entities spawned before are not known until the server sends them again).
        self.engine.sim.set_entities(entity_config(interests));
        let mut filter: PacketFilter = V::play_filter(interests);
        for id in self.engine.sim.entity_packet_ids() {
            filter.insert(id);
        }
        for id in self.engine.sim.world_packet_ids() {
            filter.insert(id);
        }
        for &id in raw {
            filter.insert(id);
        }
        self.dec.set_filter(filter);
    }

    /// Logs (rate-limited) and optionally dumps a world packet the world rejected.
    fn bad_world_packet(&mut self, raw: &RawPacket, e: &ghast_world::WorldError) {
        self.world_errors += 1;
        let n = self.world_errors;
        if n <= 3 || n.is_power_of_two() {
            log::warn!(
                "[{}] bad world packet 0x{:02x} ({} bytes): {e} ({n} so far)",
                self.engine.name(),
                raw.id,
                raw.body.len()
            );
        }
        // Opt-in capture for bug reports: GHAST_DUMP_BAD_CHUNK=<dir> writes the first few
        // rejected bodies there.
        if n <= 8
            && let Some(dir) = std::env::var_os("GHAST_DUMP_BAD_CHUNK")
        {
            let path = std::path::Path::new(&dir).join(format!(
                "ghast-bad-world-{}-0x{:02x}-{n}.bin",
                std::process::id(),
                raw.id
            ));
            match std::fs::write(&path, &raw.body) {
                Ok(()) => log::warn!("[{}] wrote {}", self.engine.name(), path.display()),
                Err(err) => log::warn!("cannot write {}: {err}", path.display()),
            }
        }
    }

    /// Encodes the engine's outbound packets.
    fn encode_outbound(&mut self) -> Result<(), ClientError> {
        let tick = self.engine.tick_count();
        let out = self.engine.outbound();
        V::prepare_play(out, &mut self.wire);
        let mut res = Ok(());
        for p in out.drain(..) {
            log::trace!(target: "ghast::packets", "t{tick} -> {p:?}");
            if let Err(e) = V::write_play(&mut self.enc, &p, &mut self.wire) {
                res = Err(e);
                break;
            }
        }
        out.clear();
        self.engine
            .note_sequence(V::prediction_sequence(&self.wire));
        res.map_err(ClientError::from)
    }

    /// Decodes and applies everything buffered. Returns a session end on kick.
    fn drain_inbound(&mut self) -> Result<Option<SessionEnd>, ClientError> {
        let mut at = None;
        while let Some(raw) = self.dec.next_packet()? {
            log::trace!(
                target: "ghast::packets",
                "t{} <- 0x{:02x} ({} B)",
                self.engine.tick_count(),
                raw.id,
                raw.body.len()
            );
            if (0..256).contains(&raw.id)
                && self.raw_mask[raw.id as usize >> 6] & (1 << (raw.id & 63)) != 0
            {
                let at = *at.get_or_insert_with(std::time::Instant::now);
                self.engine.push_raw(crate::engine::RawInbound {
                    id: raw.id,
                    body: raw.body.clone(),
                    at,
                });
            }
            match self.engine.sim.world_packet(&raw) {
                Ok(()) => {}
                Err(SimError::Decode(source)) => {
                    return Err(ClientError::Decode {
                        state: State::Play,
                        id: raw.id,
                        source,
                    });
                }
                Err(SimError::World(e)) => {
                    // Malformed chunk data for one column: the world has unloaded that column
                    // (never guessed as air); the session goes on, like vanilla, which logs the
                    // failed task and keeps playing.
                    self.bad_world_packet(&raw, &e);
                    continue;
                }
            }
            // 26.1 handleMoveVehicle answers at once (sent with this read's replies).
            if let Some(o) = self.engine.sim.take_reply() {
                self.engine.outbound().push(o);
            }
            // 1.20.2+ mount screens read the entity (type, saddle / body slots).
            if let Some(eid) = V::mount_screen_entity(&raw) {
                let mount = self.engine.sim.mount_info(eid);
                V::open_mount_screen(self.engine.inventory_mut(), &raw, mount).map_err(
                    |source| ClientError::Decode {
                        state: State::Play,
                        id: raw.id,
                        source,
                    },
                )?;
            }
            // The inventory answers window confirms itself (exactly once).
            let inv = V::inventory_packet(self.engine.inventory_mut(), &raw).map_err(|source| {
                ClientError::Decode {
                    state: State::Play,
                    id: raw.id,
                    source,
                }
            })?;
            if let Some(o) = inv {
                self.engine
                    .outbound()
                    .push(crate::common::Outbound::Inventory(o));
            }
            let p = V::decode_play(&raw, &mut self.wire).map_err(|source| ClientError::Decode {
                state: State::Play,
                id: raw.id,
                source,
            })?;
            V::write_wire_replies(&mut self.enc, &mut self.wire)?;
            match p {
                None => {}
                Some(Inbound::Disconnect(json)) => {
                    let reason = chat_to_plain(&json);
                    log::warn!("[{}] kicked: {reason}", self.engine.core_name());
                    return Ok(Some(SessionEnd::Kicked {
                        reason,
                        json: json.to_string(),
                    }));
                }
                Some(Inbound::StartConfiguration) => {
                    // Everything after this packet is configuration state: stop here.
                    self.engine.handle(Inbound::StartConfiguration);
                    self.reconfigure = true;
                    return Ok(None);
                }
                Some(Inbound::Transfer { host, port }) => {
                    // ClientCommonPacketListenerImpl.handleTransfer: leave for the target.
                    log::info!(
                        "[{}] server transfer to {host}:{port}",
                        self.engine.core_name()
                    );
                    return Ok(Some(SessionEnd::Transferred { host, port }));
                }
                Some(p @ Inbound::JoinGame(_)) => {
                    self.engine.handle(p);
                    self.on_join();
                }
                Some(Inbound::SetCompression(t)) => {
                    // 1.8 play-state Set Compression: applies to the next frame both ways.
                    self.dec.set_compression(t);
                    self.encode_outbound()?;
                    self.enc.set_compression(t);
                }
                Some(p) => self.engine.handle(p),
            }
        }
        Ok(None)
    }

    /// Play → configuration → play (1.20.2+ `start_configuration`): acknowledges, runs the
    /// configuration state (no ticks meanwhile, like vanilla without a level) and resumes play;
    /// the server then sends a new join game.
    async fn reconfigure_if_asked<B: Behavior>(
        &mut self,
        behavior: &mut B,
    ) -> Result<Option<SessionEnd>, ClientError> {
        if !self.reconfigure {
            return Ok(None);
        }
        self.reconfigure = false;
        log::info!("[{}] server asked for reconfiguration", self.engine.name());
        self.encode_outbound()?;
        V::write_configuration_start_replies(&mut self.enc, &mut self.wire)?;
        V::write_configuration_acknowledged(&mut self.enc)?;
        flush(&mut self.wr, &mut self.enc).await?;
        let play_filter = *self.dec.filter();
        self.dec.set_filter(PacketFilter::all());
        let mut registries = Vec::new();
        let end = configure::<V, _, _, B>(
            self.client,
            behavior,
            &mut self.rd,
            &mut self.wr,
            &mut self.dec,
            &mut self.enc,
            &mut self.wire,
            &mut registries,
        )
        .await?;
        if let ConfigEnd::Transfer { host, port } = end {
            return Ok(Some(SessionEnd::Transferred { host, port }));
        }
        for raw in &registries {
            self.engine.sim.registry_data(raw);
        }
        V::configure_inventory(self.engine.inventory_mut(), &self.wire);
        V::feed_inventory_config(self.engine.inventory_mut(), &mut self.wire);
        self.dec.set_filter(play_filter);
        self.engine.note_inbound(Instant::now().into_std());
        Ok(None)
    }

    /// Ends the connection the way a client that keeps reading does: half-close (FIN), then
    /// read and drop whatever the server still sends until it closes too (at most 1 s). Closing
    /// with unread bytes in the receive buffer makes the kernel answer with a reset, which the
    /// server logs as "Connection reset by peer" instead of a normal disconnect.
    async fn close(&mut self) {
        let _ = self.wr.shutdown().await;
        let mut buf = [0u8; 16 * 1024];
        let drain = async {
            loop {
                match self.rd.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(1), drain).await;
    }

    /// Reads if data is available right now; `None` if the read would wait.
    async fn read_ready(&mut self) -> Option<std::io::Result<usize>> {
        let mut fut = std::pin::pin!(read_more(&mut self.rd, &mut self.dec));
        std::future::poll_fn(|cx| {
            std::task::Poll::Ready(match fut.as_mut().poll(cx) {
                std::task::Poll::Ready(r) => Some(r),
                std::task::Poll::Pending => None,
            })
        })
        .await
    }

    async fn run<B: Behavior, S: Future<Output = ()>>(
        &mut self,
        behavior: &mut B,
        mut shutdown: Pin<&mut S>,
    ) -> Result<SessionEnd, ClientError> {
        let interests = self.engine.interests(behavior);
        let raw = self.engine.raw_packets(behavior);
        self.set_filter(interests, &raw);
        self.engine.note_inbound(Instant::now().into_std());
        // Frames that arrived together with login success.
        if let Some(end) = self.drain_inbound()? {
            return Ok(end);
        }
        if let Some(end) = self.reconfigure_if_asked(behavior).await? {
            return Ok(end);
        }
        self.encode_outbound()?;
        flush(&mut self.wr, &mut self.enc).await?;

        let mut ticker = tokio::time::interval_at(Instant::now() + TICK, TICK);
        // After a stall the missed ticks are skipped, never caught up: a burst of movement
        // packets is what anti-cheat timer checks flag.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            let keys = async {
                match self.keys_task.as_mut() {
                    Some(t) => t.await,
                    None => std::future::pending().await,
                }
            };
            let wake = tokio::select! {
                biased;
                () = shutdown.as_mut() => Wake::Shutdown,
                _ = ticker.tick() => Wake::Tick,
                r = read_more(&mut self.rd, &mut self.dec) => Wake::Read(r),
                k = keys => Wake::Keys(Box::new(k)),
            };
            match wake {
                Wake::Keys(k) => self.keys_done(*k),
                Wake::Shutdown => {
                    self.encode_outbound()?;
                    flush(&mut self.wr, &mut self.enc).await?;
                    self.close().await;
                    return Ok(SessionEnd::Shutdown);
                }
                Wake::Read(r) => {
                    if r? == 0 {
                        return Ok(SessionEnd::ServerClosed);
                    }
                    let now = Instant::now().into_std();
                    self.engine.note_inbound(now);
                    if let Some(end) = self.drain_inbound()? {
                        return Ok(end);
                    }
                    if self.engine.sim.take_filter_dirty() {
                        let raw = self.raw_ids.clone();
                        self.set_filter(self.interests, &raw);
                    }
                    if let Some(end) = self.reconfigure_if_asked(behavior).await? {
                        return Ok(end);
                    }
                    // Replies (keep-alive, teleport, confirms) go out right away, like
                    // vanilla's per-frame packet handling.
                    self.encode_outbound()?;
                    flush(&mut self.wr, &mut self.enc).await?;
                }
                Wake::Tick => {
                    // Apply what arrived since the last read first (inbound before the tick),
                    // without waiting for more.
                    for _ in 0..64 {
                        match self.read_ready().await {
                            None => break,
                            Some(r) => {
                                if r? == 0 {
                                    return Ok(SessionEnd::ServerClosed);
                                }
                                self.engine.note_inbound(Instant::now().into_std());
                                if let Some(end) = self.drain_inbound()? {
                                    return Ok(end);
                                }
                                if let Some(end) = self.reconfigure_if_asked(behavior).await? {
                                    return Ok(end);
                                }
                            }
                        }
                    }
                    let now = Instant::now().into_std();
                    if let Some(last) = self.engine.last_inbound()
                        && now.saturating_duration_since(last) > self.read_timeout
                    {
                        return Err(ClientError::Timeout("no data from the server"));
                    }
                    self.maybe_refresh_keys();
                    {
                        // Dialog commands are checked against the server's command tree.
                        let wire = std::cell::RefCell::new(&mut self.wire);
                        let check = |c: &str| V::check_command(&mut wire.borrow_mut(), c);
                        self.engine.tick_with(behavior, now, &check);
                    }
                    self.encode_outbound()?;
                    flush(&mut self.wr, &mut self.enc).await?;
                    if let Some(reason) = self.engine.quit_requested() {
                        let reason = reason.to_owned();
                        self.close().await;
                        return Ok(SessionEnd::Quit(reason));
                    }
                    let interests = self.engine.interests(behavior);
                    let raw = self.engine.raw_packets(behavior);
                    if interests != self.interests || *raw != *self.raw_ids {
                        self.set_filter(interests, &raw);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The interests map to the tracker configuration (none without `ENTITIES`).
    #[test]
    fn entity_config_follows_the_interests() {
        assert_eq!(entity_config(Interests::PLAYER_LIST), None);
        let c = entity_config(Interests::ENTITIES).unwrap();
        assert!(c.metadata && !c.player_list && !c.skins);
        let c =
            entity_config(Interests::ENTITIES | Interests::PLAYER_LIST | Interests::PLAYER_SKINS)
                .unwrap();
        assert!(c.player_list && c.skins);
    }
}
