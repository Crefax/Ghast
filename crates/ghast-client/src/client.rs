//! Connection setup and the session driver: connect → handshake → login → play loop.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ghast_auth::connect::{Connector, TargetAddr};
use ghast_auth::offline::OfflineProfile;
use ghast_protocol::{FrameDecoder, FrameEncoder, PacketFilter, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{Instant, MissedTickBehavior};

use crate::common::{ClientSettings, Inbound, LoginInbound, chat_to_plain};
use crate::engine::{Behavior, CoreOptions, Engine};
use crate::error::{ClientError, SessionEnd};
use crate::interest::Interests;
use crate::sim::{Sim, SimError};
use crate::version::{ClientVersion, GameVersion, HasSim, V1_8, V1_16_5};

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
}

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
        }
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
        match self.version {
            GameVersion::V1_8_9 => session::<V1_8, B, S>(self, behavior, shutdown).await,
            GameVersion::V1_16_5 => session::<V1_16_5, B, S>(self, behavior, shutdown).await,
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

async fn session<V, B, S>(
    client: &Client,
    behavior: &mut B,
    mut shutdown: Pin<&mut S>,
) -> Result<SessionEnd, ClientError>
where
    V: ClientVersion + HasSim,
    B: Behavior,
    S: Future<Output = ()>,
{
    let name: Arc<str> = client.account.name().into();
    let mut dec = FrameDecoder::new();
    let mut enc = FrameEncoder::new();

    let login = async {
        let stream = client.connector.connect(&client.address).await?;
        let (mut rd, mut wr) = stream.into_split();
        login::<V, _, _>(client, &mut rd, &mut wr, &mut dec, &mut enc).await?;
        Ok::<_, ClientError>((rd, wr))
    };
    let (rd, wr) = tokio::select! {
        r = tokio::time::timeout(client.login_timeout, login) => match r {
            Ok(r) => r?,
            Err(_) => return Err(ClientError::Timeout("connect and login")),
        },
        () = shutdown.as_mut() => return Ok(SessionEnd::Shutdown),
    };
    log::info!("[{name}] logged in to {} ({})", client.address, V::GAME);

    let opts = CoreOptions {
        name,
        version: V::GAME,
        settings: client.settings.clone(),
        brand: client.brand.clone(),
        sensitivity: client.sensitivity,
        silent_hold: client.silent_hold,
        inventory: V::INVENTORY,
        hold_turn_on_action: client.hold_turn_on_action,
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
                entities: behavior.interests().contains(Interests::ENTITIES).then(|| {
                    // Positions, metadata (item stacks, sneak / pose flags); equipment and other
                    // entities' effects are not decoded (cheaper).
                    ghast_entity::Config {
                        metadata: true,
                        equipment: false,
                        effects: false,
                        player_list: behavior.interests().contains(Interests::PLAYER_LIST),
                        events: false,
                    }
                }),
            }),
        ),
        read_timeout: client.read_timeout,
        interests: Interests::NONE,
        raw_ids: Vec::new(),
        raw_mask: [0; 4],
        _v: std::marker::PhantomData,
    };
    let result = play.run(behavior, shutdown).await;
    let reason = match &result {
        Ok(SessionEnd::Kicked { reason, .. }) => format!("kicked: {reason}"),
        Ok(SessionEnd::ServerClosed) => "server closed the connection".to_owned(),
        Ok(SessionEnd::Quit(r)) => format!("quit: {r}"),
        Ok(SessionEnd::Shutdown) => "shutdown".to_owned(),
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
    result
}

/// Handshake and login. Returns once the server sent login success.
async fn login<V, R, W>(
    client: &Client,
    rd: &mut R,
    wr: &mut W,
    dec: &mut FrameDecoder,
    enc: &mut FrameEncoder,
) -> Result<(), ClientError>
where
    V: ClientVersion,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    V::write_handshake(enc, &client.address.host(), client.address.port())?;
    V::write_login_start(enc, client.account.name())?;
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
                }
                Some(LoginInbound::Success { uuid, username }) => {
                    log::debug!("login success: {username} {uuid:?}");
                    return Ok(());
                }
            }
        }
        if read_more(rd, dec).await? == 0 {
            return Err(ClientError::LoginClosed);
        }
    }
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
struct Play<V: HasSim, R, W> {
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
    _v: std::marker::PhantomData<V>,
}

/// What woke the play loop.
enum Wake {
    Read(std::io::Result<usize>),
    Tick,
    Shutdown,
}

impl<V, R, W> Play<V, R, W>
where
    V: ClientVersion + HasSim,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
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
        let mut filter: PacketFilter = V::play_filter(interests);
        for id in self.engine.sim.entity_packet_ids() {
            filter.insert(id);
        }
        for &id in raw {
            filter.insert(id);
        }
        self.dec.set_filter(filter);
    }

    /// Encodes the engine's outbound packets.
    fn encode_outbound(&mut self) -> Result<(), ClientError> {
        let out = self.engine.outbound();
        let mut res = Ok(());
        for p in out.drain(..) {
            if let Err(e) = V::write_play(&mut self.enc, &p) {
                res = Err(e);
                break;
            }
        }
        out.clear();
        res.map_err(ClientError::from)
    }

    /// Decodes and applies everything buffered. Returns a session end on kick.
    fn drain_inbound(&mut self) -> Result<Option<SessionEnd>, ClientError> {
        let mut at = None;
        while let Some(raw) = self.dec.next_packet()? {
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
            self.engine.sim.world_packet(&raw).map_err(|e| match e {
                SimError::Decode(source) => ClientError::Decode {
                    state: State::Play,
                    id: raw.id,
                    source,
                },
                SimError::World(e) => ClientError::World(e),
            })?;
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
            let p = V::decode_play(&raw).map_err(|source| ClientError::Decode {
                state: State::Play,
                id: raw.id,
                source,
            })?;
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
        self.encode_outbound()?;
        flush(&mut self.wr, &mut self.enc).await?;

        let mut ticker = tokio::time::interval_at(Instant::now() + TICK, TICK);
        // After a stall the missed ticks are skipped, never caught up: a burst of movement
        // packets is what anti-cheat timer checks flag.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            let wake = tokio::select! {
                biased;
                () = shutdown.as_mut() => Wake::Shutdown,
                _ = ticker.tick() => Wake::Tick,
                r = read_more(&mut self.rd, &mut self.dec) => Wake::Read(r),
            };
            match wake {
                Wake::Shutdown => {
                    self.encode_outbound()?;
                    flush(&mut self.wr, &mut self.enc).await?;
                    let _ = self.wr.shutdown().await;
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
                            }
                        }
                    }
                    let now = Instant::now().into_std();
                    if let Some(last) = self.engine.last_inbound()
                        && now.saturating_duration_since(last) > self.read_timeout
                    {
                        return Err(ClientError::Timeout("no data from the server"));
                    }
                    self.engine.tick(behavior, now);
                    self.encode_outbound()?;
                    flush(&mut self.wr, &mut self.enc).await?;
                    if let Some(reason) = self.engine.quit_requested() {
                        let reason = reason.to_owned();
                        let _ = self.wr.shutdown().await;
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
