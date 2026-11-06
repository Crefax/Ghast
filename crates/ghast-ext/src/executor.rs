//! The tick-driven executor: [`Ext`] (a [`Behavior`]) polls async tasks inside
//! [`Behavior::tick`], and [`Bot`] is the handle the tasks use.
//!
//! A task never holds the [`TickContext`]. It sends a closure ([`Bot::with`]) that the
//! executor runs with the context during the same tick, then resumes the task with the
//! result. So every action is still written inside the tick, before the tick's movement
//! packet, in call order, and the crate needs no `unsafe`.

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll, Wake, Waker};

use ghast_client::{Behavior, GameMode, GameVersion, Health, Interests, Pose, TickContext, Vec3};

use crate::Error;
use crate::events::{Event, SubState};

/// A closure run with the tick's context.
pub(crate) type Request = Box<dyn FnOnce(&mut TickContext<'_>) + Send>;
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type Factory = Box<dyn FnMut(Bot) -> BoxFuture<Result<(), Error>> + Send>;
/// Gives a spawned task's handle [`Error::Panicked`].
type PanicHook = Box<dyn FnOnce(String) + Send>;

/// Most poll / request rounds in one tick (a task that loops on [`Bot::with`] without
/// waiting for a tick cannot hang the tick).
const MAX_ROUNDS: usize = 256;

/// What the bot looked like at the start of this tick (cheap copies for sync reads).
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub(crate) version: GameVersion,
    pub(crate) ctx_tick: u64,
    pub(crate) pose: Pose,
    pub(crate) sent: Pose,
    pub(crate) eyes: Vec3,
    pub(crate) health: Health,
    pub(crate) dead: bool,
    pub(crate) game_mode: GameMode,
    pub(crate) held_slot: u8,
    pub(crate) open_window: Option<i32>,
    pub(crate) entity_id: i32,
    pub(crate) dimension: String,
}

impl Snapshot {
    fn empty() -> Self {
        Snapshot {
            version: GameVersion::V1_8_9,
            ctx_tick: 0,
            pose: Pose::default(),
            sent: Pose::default(),
            eyes: Vec3::default(),
            health: Health::default(),
            dead: false,
            game_mode: GameMode::Survival,
            held_slot: 0,
            open_window: None,
            entity_id: 0,
            dimension: String::new(),
        }
    }

    fn refresh(&mut self, ctx: &TickContext<'_>) {
        self.version = ctx.version();
        self.ctx_tick = ctx.tick();
        self.pose = *ctx.pose();
        self.sent = *ctx.sent();
        self.eyes = ctx.eyes(ctx.pose());
        self.health = ctx.health();
        self.dead = ctx.is_dead();
        self.game_mode = ctx.game_mode();
        self.held_slot = ctx.held_slot();
        self.open_window = ctx.open_window();
        self.entity_id = ctx.entity_id();
        if self.dimension != ctx.dimension() {
            ctx.dimension().clone_into(&mut self.dimension);
        }
    }
}

/// State shared between the executor and the handles.
pub(crate) struct Shared {
    /// Session counter (a handle's futures fail once their session ended).
    pub(crate) session: u64,
    /// The session ended (reason), or none started yet.
    pub(crate) closed: Option<String>,
    /// Behaviour ticks of this session (frozen while the server is silent).
    pub(crate) tick: u64,
    pub(crate) snap: Snapshot,
    pub(crate) requests: VecDeque<Request>,
    pub(crate) tick_wakers: Vec<Waker>,
    pub(crate) spawned: Vec<(u64, BoxFuture<()>, PanicHook)>,
    pub(crate) aborted: Vec<u64>,
    pub(crate) next_task: u64,
    pub(crate) subs: Vec<Weak<Mutex<SubState>>>,
    /// Teleports and deaths so far (helpers fail when these change under them).
    pub(crate) teleports: u64,
    pub(crate) deaths: u64,
    /// Interests the helpers asked for on top of the configured ones.
    pub(crate) extra_interests: Interests,
    /// Entity ids seen last tick (only while someone listens to events).
    entity_ids: Option<HashSet<i32>>,
    pub(crate) interrupt_on_teleport: bool,
}

impl Shared {
    fn new() -> Self {
        Shared {
            session: 0,
            closed: Some("not started".into()),
            tick: 0,
            snap: Snapshot::empty(),
            requests: VecDeque::new(),
            tick_wakers: Vec::new(),
            spawned: Vec::new(),
            aborted: Vec::new(),
            next_task: 1,
            subs: Vec::new(),
            teleports: 0,
            deaths: 0,
            extra_interests: Interests::NONE,
            entity_ids: None,
            interrupt_on_teleport: true,
        }
    }

    fn wake_ticks(&mut self) {
        for w in self.tick_wakers.drain(..) {
            w.wake();
        }
    }

    fn wake_subs(&mut self) {
        self.subs.retain(|s| s.strong_count() > 0);
        for s in &self.subs {
            if let Some(s) = s.upgrade() {
                let mut s = lock(&s);
                if let Some(w) = s.waker.take() {
                    w.wake();
                }
            }
        }
    }

    /// Turns this tick's events into [`Event`]s for the subscribers.
    fn publish(&mut self, ctx: &TickContext<'_>) {
        self.subs.retain(|s| s.strong_count() > 0);
        if self.subs.is_empty() {
            self.entity_ids = None;
            return;
        }
        let ev = ctx.events();
        let mut out = Vec::new();
        if ev.joined {
            out.push(Event::Joined);
        }
        if ev.respawned {
            out.push(Event::Respawned);
        }
        for t in &ev.teleports {
            out.push(Event::Teleport(*t));
        }
        if ev.died {
            out.push(Event::Died);
        }
        if ev.health_changed {
            out.push(Event::Health(ctx.health()));
        }
        if ev.game_mode_changed {
            out.push(Event::GameMode(ctx.game_mode()));
        }
        for m in &ev.chat {
            out.push(Event::Chat(m.clone()));
        }
        if let Some(id) = ev.inventory.opened {
            out.push(Event::WindowOpened(id));
        }
        if ev.inventory.closed {
            out.push(Event::WindowClosed);
        }
        if ev.inventory.player_changed {
            out.push(Event::InventoryChanged);
        }
        for c in &ev.inventory.clicks {
            out.push(Event::Click(*c));
        }
        for (pos, raw) in &ev.block_updates {
            out.push(Event::BlockUpdate {
                pos: *pos,
                raw: *raw,
            });
        }
        if let Some(es) = ctx.entities() {
            let me = es.self_id();
            let now: HashSet<i32> = es
                .iter()
                .map(|e| e.id())
                .filter(|id| Some(*id) != me)
                .collect();
            if let Some(before) = &self.entity_ids {
                for id in now.difference(before) {
                    if let Some(e) = crate::find::entity_info(ctx, *id) {
                        out.push(Event::EntitySpawned(e));
                    }
                }
                for id in before.difference(&now) {
                    out.push(Event::EntityGone(*id));
                }
            }
            self.entity_ids = Some(now);
        } else {
            self.entity_ids = None;
        }
        if out.is_empty() {
            return;
        }
        for s in &self.subs {
            if let Some(s) = s.upgrade() {
                let mut s = lock(&s);
                s.queue.extend(out.iter().cloned());
                if let Some(w) = s.waker.take() {
                    w.wake();
                }
            }
        }
    }
}

/// Locks a mutex, ignoring poisoning (no lock is held while user code runs).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A handle to the bot for async tasks: cheap to clone, `Send + Sync`.
///
/// Reads of the per-tick snapshot ([`Bot::pose`], [`Bot::health`], ...) are synchronous.
/// Everything that touches the world, the inventory or the server goes through
/// [`Bot::with`] (or a helper built on it) and runs inside the tick.
///
/// Handles work best in tasks the [`Ext`] executor polls ([`Ext::new`], [`Bot::spawn`]):
/// those resume within the same tick. A handle moved into a task of another executor (e.g.
/// a tokio task reading stdin) works too, one tick later.
#[derive(Clone)]
pub struct Bot {
    pub(crate) shared: Arc<Mutex<Shared>>,
}

impl std::fmt::Debug for Bot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = lock(&self.shared);
        f.debug_struct("Bot")
            .field("session", &s.session)
            .field("tick", &s.tick)
            .field("pos", &s.snap.pose.pos)
            .finish_non_exhaustive()
    }
}

impl Bot {
    fn new() -> Self {
        Bot {
            shared: Arc::new(Mutex::new(Shared::new())),
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Shared> {
        lock(&self.shared)
    }

    // ---- sync reads (start of this tick) ----

    /// Ticks this behaviour ran in the current session (it does not advance while the
    /// server is silent and the behaviour is on hold).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) {
    /// let t = bot.tick();
    /// bot.wait_ticks(20).await;
    /// assert!(bot.tick() >= t + 20);
    /// # }
    /// ```
    #[must_use]
    pub fn tick(&self) -> u64 {
        self.lock().tick
    }

    /// The protocol version of the session.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// if bot.version() == ghast_client::GameVersion::V1_8_9 { /* ... */ }
    /// # }
    /// ```
    #[must_use]
    pub fn version(&self) -> GameVersion {
        self.lock().snap.version
    }

    /// The client pose at the start of this tick.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// let p = bot.pose().pos;
    /// println!("at {:.1} {:.1} {:.1}", p.x, p.y, p.z);
    /// # }
    /// ```
    #[must_use]
    pub fn pose(&self) -> Pose {
        self.lock().snap.pose
    }

    /// The pose the server has (the last movement packet): what it judges actions against.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// let server_yaw = bot.sent().yaw;
    /// # }
    /// ```
    #[must_use]
    pub fn sent(&self) -> Pose {
        self.lock().snap.sent
    }

    /// Eye position at the start of this tick.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// let eyes = bot.eyes();
    /// # }
    /// ```
    #[must_use]
    pub fn eyes(&self) -> Vec3 {
        self.lock().snap.eyes
    }

    /// Health, food and saturation.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// if bot.health().food < 15 { /* eat */ }
    /// # }
    /// ```
    #[must_use]
    pub fn health(&self) -> Health {
        self.lock().snap.health
    }

    /// The player is dead (see [`Bot::respawn`]).
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// if bot.is_dead() { bot.respawn(); }
    /// # }
    /// ```
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.lock().snap.dead
    }

    /// Game mode.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// let creative = bot.game_mode() == ghast_client::GameMode::Creative;
    /// # }
    /// ```
    #[must_use]
    pub fn game_mode(&self) -> GameMode {
        self.lock().snap.game_mode
    }

    /// The selected hotbar slot (0 … 8).
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// let slot = bot.held_slot();
    /// # }
    /// ```
    #[must_use]
    pub fn held_slot(&self) -> u8 {
        self.lock().snap.held_slot
    }

    /// Id of the open container window.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// if bot.open_window().is_some() { /* a chest is open */ }
    /// # }
    /// ```
    #[must_use]
    pub fn open_window(&self) -> Option<i32> {
        self.lock().snap.open_window
    }

    /// Our entity id.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// let me = bot.entity_id();
    /// # }
    /// ```
    #[must_use]
    pub fn entity_id(&self) -> i32 {
        self.lock().snap.entity_id
    }

    /// The current dimension (`"0"` / `"-1"` / `"1"` on 1.8, the world name on 1.16).
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// let nether = bot.dimension() == "-1";
    /// # }
    /// ```
    #[must_use]
    pub fn dimension(&self) -> String {
        self.lock().snap.dimension.clone()
    }

    // ---- the tick context ----

    /// Runs `f` with this tick's [`TickContext`] and returns its result. From a task the
    /// [`Ext`] executor polls, `f` runs in the same tick (before the movement packet), so
    /// several `with` calls in a row cost no ticks. Actions inside `f` follow the usual
    /// rules (`set_aim` only for this tick, aim first and act on a later tick, ...).
    ///
    /// A panic in `f` resumes in the awaiting task. Fails with [`Error::Disconnected`] once
    /// the session ended.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let (stone, raw) = bot
    ///     .with(|ctx| {
    ///         let p = ghast_client::BlockPos::new(0, 63, 0);
    ///         (ctx.world().raw_block(p.x, p.y, p.z), ctx.broken())
    ///     })
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn with<R, F>(&self, f: F) -> With<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut TickContext<'_>) -> R + Send + 'static,
    {
        let slot = Arc::new(Mutex::new(WithSlot {
            result: None,
            waker: None,
            dropped: false,
        }));
        let s2 = slot.clone();
        let guard = DropWake(slot.clone());
        let req: Request = Box::new(move |ctx| {
            let r = catch_unwind(AssertUnwindSafe(|| f(ctx)));
            let mut s = lock(&s2);
            s.result = Some(r);
            if let Some(w) = s.waker.take() {
                w.wake();
            }
            drop(s);
            std::mem::forget(guard);
        });
        With {
            bot: self.clone(),
            session: self.lock().session,
            req: Some(req),
            slot,
        }
    }

    /// Queues `f` for this tick (or the next one) without waiting for it: requests run in
    /// the order they were made. For fire-and-forget actions.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// bot.act(|ctx| ctx.swing());
    /// # }
    /// ```
    pub fn act<F>(&self, f: F)
    where
        F: FnOnce(&mut TickContext<'_>) + Send + 'static,
    {
        let mut s = self.lock();
        if s.closed.is_none() {
            s.requests.push_back(Box::new(f));
        }
    }

    /// Waits `n` behaviour ticks (`0` returns at once). Resolves at once when the session
    /// ended.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) {
    /// bot.wait_ticks(20).await; // one second
    /// # }
    /// ```
    pub fn wait_ticks(&self, n: u64) -> WaitTicks {
        let s = self.lock();
        WaitTicks {
            bot: self.clone(),
            session: s.session,
            until: s.tick + n,
        }
    }

    /// Waits for the next tick.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) {
    /// loop {
    ///     if bot.health().health < 10.0 { break; }
    ///     bot.next_tick().await;
    /// }
    /// # }
    /// ```
    pub fn next_tick(&self) -> WaitTicks {
        self.wait_ticks(1)
    }

    /// Waits about `d` of game time, in whole ticks (50 ms each, rounded up).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) {
    /// bot.sleep(std::time::Duration::from_secs(2)).await;
    /// # }
    /// ```
    pub fn sleep(&self, d: std::time::Duration) -> WaitTicks {
        let ms = d.as_millis();
        self.wait_ticks(u64::try_from(ms.div_ceil(50)).unwrap_or(u64::MAX))
    }

    /// Runs `fut` and gives up after `ticks` ticks with [`Error::Timeout`] (`fut` is dropped,
    /// which cancels it; helpers release their buttons when dropped).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let pos = ghast_client::BlockPos::new(3, 64, 0);
    /// bot.timeout(200, bot.dig(pos)).await??;
    /// # Ok(()) }
    /// ```
    pub fn timeout<F: Future>(&self, ticks: u64, fut: F) -> Timeout<F> {
        Timeout {
            fut: Box::pin(fut),
            wait: self.wait_ticks(ticks),
        }
    }

    /// Waits until `cond` holds, checked once per tick with the tick's context (also at
    /// once).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.wait_until(|ctx| ctx.pose().on_ground).await?;
    /// # Ok(()) }
    /// ```
    pub async fn wait_until<F>(&self, cond: F) -> Result<(), Error>
    where
        F: Fn(&TickContext<'_>) -> bool + Send + Sync + 'static,
    {
        let cond = Arc::new(cond);
        loop {
            let c = cond.clone();
            if self.with(move |ctx| c(ctx)).await? {
                return Ok(());
            }
            self.next_tick().await;
        }
    }

    /// Runs `fut` as another task of this bot (polled by the same executor, inside the
    /// ticks). It is cancelled when the session ends. The handle gives its result.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) {
    /// let b = bot.clone();
    /// let guard = bot.spawn(async move {
    ///     loop {
    ///         if b.is_dead() { b.respawn(); }
    ///         b.wait_ticks(20).await;
    ///     }
    /// });
    /// // ... later
    /// guard.abort();
    /// # }
    /// ```
    pub fn spawn<T, F>(&self, fut: F) -> JoinHandle<T>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let slot = Arc::new(Mutex::new(JoinSlot {
            result: None,
            waker: None,
        }));
        let done = Completer(slot.clone());
        let on_panic = slot.clone();
        let hook: PanicHook = Box::new(move |msg| {
            let mut s = lock(&on_panic);
            // Unset, or the placeholder the completer left while unwinding.
            if s.result.is_none()
                || matches!(&s.result, Some(Err(Error::Panicked(m))) if m.is_empty())
            {
                s.result = Some(Err(Error::Panicked(msg)));
                if let Some(w) = s.waker.take() {
                    w.wake();
                }
            }
        });
        let wrapped = async move {
            let v = fut.await;
            done.complete(Ok(v));
        };
        let mut s = self.lock();
        let id = s.next_task;
        s.next_task += 1;
        if s.closed.is_some() {
            drop(s);
            lock(&slot).result = Some(Err(Error::Disconnected));
        } else {
            s.spawned.push((id, Box::pin(wrapped), hook));
        }
        JoinHandle {
            bot: self.clone(),
            id,
            slot,
        }
    }

    /// Asks the helpers' packet groups on top of the configured [`Ext::interests`] (e.g.
    /// [`Interests::ENTITIES`] for entity helpers). Better set up front with
    /// [`Ext::interests`]: a tracker started late misses entities that already exist.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// bot.require(ghast_client::Interests::BLOCK_UPDATES);
    /// # }
    /// ```
    pub fn require(&self, interests: Interests) {
        let mut s = self.lock();
        s.extra_interests |= interests;
    }

    /// The point where helpers that must not survive a teleport or a death started.
    pub(crate) fn mark(&self) -> Mark {
        let s = self.lock();
        Mark {
            teleports: s.teleports,
            deaths: s.deaths,
        }
    }

    /// Fails when the player was teleported (if [`Ext::interrupt_on_teleport`]) or died
    /// since `m`.
    pub(crate) fn check(&self, m: Mark) -> Result<(), Error> {
        let s = self.lock();
        if s.closed.is_some() {
            return Err(Error::Disconnected);
        }
        if s.deaths != m.deaths {
            return Err(Error::Died);
        }
        if s.interrupt_on_teleport && s.teleports != m.teleports {
            return Err(Error::Teleported);
        }
        Ok(())
    }
}

/// Teleport / death counters at a helper's start.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mark {
    teleports: u64,
    deaths: u64,
}

struct WithSlot<R> {
    result: Option<std::thread::Result<R>>,
    waker: Option<Waker>,
    /// The request was dropped without running (the session ended).
    dropped: bool,
}

/// Wakes the waiting future when a request is dropped without running.
struct DropWake<R>(Arc<Mutex<WithSlot<R>>>);

impl<R> Drop for DropWake<R> {
    fn drop(&mut self) {
        let mut s = lock(&self.0);
        s.dropped = true;
        if let Some(w) = s.waker.take() {
            w.wake();
        }
    }
}

/// The future of [`Bot::with`].
#[must_use = "futures do nothing unless awaited"]
pub struct With<R> {
    bot: Bot,
    session: u64,
    req: Option<Request>,
    slot: Arc<Mutex<WithSlot<R>>>,
}

impl<R> std::fmt::Debug for With<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("With")
            .field("sent", &self.req.is_none())
            .finish()
    }
}

impl<R> Future for With<R> {
    type Output = Result<R, Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        if let Some(req) = this.req.take() {
            let mut s = this.bot.lock();
            if s.closed.is_some() || s.session != this.session {
                return Poll::Ready(Err(Error::Disconnected));
            }
            lock(&this.slot).waker = Some(cx.waker().clone());
            s.requests.push_back(req);
            return Poll::Pending;
        }
        let mut slot = lock(&this.slot);
        match slot.result.take() {
            Some(Ok(v)) => Poll::Ready(Ok(v)),
            Some(Err(panic)) => {
                drop(slot);
                std::panic::resume_unwind(panic)
            }
            None if slot.dropped => Poll::Ready(Err(Error::Disconnected)),
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// The future of [`Bot::wait_ticks`].
#[must_use = "futures do nothing unless awaited"]
#[derive(Debug)]
pub struct WaitTicks {
    bot: Bot,
    session: u64,
    until: u64,
}

impl Future for WaitTicks {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut s = self.bot.lock();
        if s.closed.is_some() || s.session != self.session || s.tick >= self.until {
            return Poll::Ready(());
        }
        if !s.tick_wakers.iter().any(|w| w.will_wake(cx.waker())) {
            s.tick_wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// The future of [`Bot::timeout`].
#[must_use = "futures do nothing unless awaited"]
pub struct Timeout<F> {
    fut: Pin<Box<F>>,
    wait: WaitTicks,
}

impl<F> std::fmt::Debug for Timeout<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Timeout")
            .field("until", &self.wait.until)
            .finish()
    }
}

impl<F: Future> Future for Timeout<F> {
    type Output = Result<F::Output, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Poll::Ready(v) = this.fut.as_mut().poll(cx) {
            return Poll::Ready(Ok(v));
        }
        let closed = {
            let s = this.wait.bot.lock();
            s.closed.is_some() || s.session != this.wait.session
        };
        match Pin::new(&mut this.wait).poll(cx) {
            Poll::Ready(()) if closed => Poll::Ready(Err(Error::Disconnected)),
            Poll::Ready(()) => Poll::Ready(Err(Error::Timeout)),
            Poll::Pending => Poll::Pending,
        }
    }
}

struct JoinSlot<T> {
    result: Option<Result<T, Error>>,
    waker: Option<Waker>,
}

/// Completes a [`JoinHandle`]; when dropped unfinished (the task was cancelled or
/// panicked) the handle gets [`Error::Cancelled`].
struct Completer<T>(Arc<Mutex<JoinSlot<T>>>);

impl<T> Completer<T> {
    fn complete(self, r: Result<T, Error>) {
        let mut s = lock(&self.0);
        s.result = Some(r);
        if let Some(w) = s.waker.take() {
            w.wake();
        }
    }
}

impl<T> Drop for Completer<T> {
    fn drop(&mut self) {
        let mut s = lock(&self.0);
        if s.result.is_none() {
            // Dropped while the task unwinds: the executor's panic hook fills in the message.
            s.result = Some(Err(if std::thread::panicking() {
                Error::Panicked(String::new())
            } else {
                Error::Cancelled
            }));
            if let Some(w) = s.waker.take() {
                w.wake();
            }
        }
    }
}

/// A spawned task ([`Bot::spawn`]): await it for its result, or [`JoinHandle::abort`] it.
/// Dropping the handle does not stop the task.
#[must_use = "dropping the handle detaches the task"]
pub struct JoinHandle<T> {
    bot: Bot,
    id: u64,
    slot: Arc<Mutex<JoinSlot<T>>>,
}

impl<T> std::fmt::Debug for JoinHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinHandle").field("id", &self.id).finish()
    }
}

impl<T> JoinHandle<T> {
    /// Stops the task at the next tick (it is dropped, which cancels what it awaits).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) {
    /// let h = bot.spawn(async {});
    /// h.abort();
    /// # }
    /// ```
    pub fn abort(&self) {
        self.bot.lock().aborted.push(self.id);
    }

    /// Whether the task finished (or was cancelled).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) {
    /// let h = bot.spawn(async {});
    /// bot.next_tick().await;
    /// assert!(h.is_finished());
    /// # }
    /// ```
    #[must_use]
    pub fn is_finished(&self) -> bool {
        lock(&self.slot).result.is_some()
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut s = lock(&self.slot);
        match s.result.take() {
            Some(r) => Poll::Ready(r),
            None => {
                s.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// Wakes a task of the executor: queues its id.
struct TaskWaker {
    id: u64,
    ready: Arc<Mutex<Vec<u64>>>,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        lock(&self.ready).push(self.id);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        lock(&self.ready).push(self.id);
    }
}

struct Task {
    id: u64,
    fut: BoxFuture<()>,
    waker: Waker,
    on_panic: Option<PanicHook>,
}

/// What [`Ext`] does when the main task returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum WhenDone {
    /// End the session (`SessionEnd::Quit` with "done", or the error). The default: the
    /// main task is the bot's `main`.
    #[default]
    Disconnect,
    /// Keep the session: other spawned tasks go on; once none are left the behaviour
    /// finishes and the player stands and answers the server.
    Stay,
}

/// A [`Behavior`] that runs async tasks inside the ticks: write a bot as sequential code.
///
/// Each session (each `Client::run`), the factory is called with a fresh [`Bot`] handle on
/// the first tick and its future becomes the main task. Every tick the executor polls the
/// tasks that are ready and runs the closures they sent ([`Bot::with`]) with the tick's
/// context, until no task can make progress in this tick. When the session ends all tasks
/// are dropped (cancelled).
///
/// ```no_run
/// use ghast_client::{Account, Client, GameVersion, Interests};
/// use ghast_ext::Ext;
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let mut bot = Ext::new(|bot| async move {
///     bot.chat("hello");
///     let msg = bot.wait_for_chat(|m| m.plain().contains("bye")).await?;
///     println!("{}", msg.plain());
///     Ok(())
/// })
/// .interests(Interests::CHAT);
/// let client = Client::new("localhost:25565".parse()?, GameVersion::V1_8_9, Account::offline("Bot")?);
/// client.run(&mut bot).await?;
/// # Ok(()) }
/// ```
pub struct Ext {
    name: String,
    factory: Factory,
    interests: Interests,
    when_done: WhenDone,
    auto_respawn: bool,
    interrupt_on_teleport: bool,
    bot: Bot,
    tasks: Vec<Task>,
    ready: Arc<Mutex<Vec<u64>>>,
    main: Option<JoinHandle<Result<(), Error>>>,
    result: Option<Result<(), Error>>,
    finished: bool,
    started: bool,
}

impl std::fmt::Debug for Ext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ext")
            .field("name", &self.name)
            .field("tasks", &self.tasks.len())
            .field("result", &self.result)
            .finish_non_exhaustive()
    }
}

impl Ext {
    /// A behaviour whose main task is `main(bot)`, called once per session.
    ///
    /// ```no_run
    /// let ext = ghast_ext::Ext::new(|bot| async move {
    ///     bot.wait_ticks(100).await;
    ///     Ok(())
    /// });
    /// ```
    pub fn new<F, Fut>(mut main: F) -> Self
    where
        F: FnMut(Bot) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), Error>> + Send + 'static,
    {
        Ext {
            name: "ext".into(),
            factory: Box::new(move |bot| Box::pin(main(bot))),
            interests: Interests::CHAT,
            when_done: WhenDone::Disconnect,
            auto_respawn: false,
            interrupt_on_teleport: true,
            bot: Bot::new(),
            tasks: Vec::new(),
            ready: Arc::new(Mutex::new(Vec::new())),
            main: None,
            result: None,
            finished: false,
            started: false,
        }
    }

    /// Name for logs (default `"ext"`).
    ///
    /// ```no_run
    /// # let ext = ghast_ext::Ext::new(|_bot| async { Ok(()) });
    /// let ext = ext.name("miner");
    /// ```
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Packet groups to decode (default [`Interests::CHAT`]). Entity helpers need
    /// [`Interests::ENTITIES`] (player names also [`Interests::PLAYER_LIST`]),
    /// [`crate::Event::BlockUpdate`] needs [`Interests::BLOCK_UPDATES`].
    ///
    /// ```no_run
    /// use ghast_client::Interests;
    /// # let ext = ghast_ext::Ext::new(|_bot| async { Ok(()) });
    /// let ext = ext.interests(Interests::CHAT | Interests::ENTITIES | Interests::PLAYER_LIST);
    /// ```
    #[must_use]
    pub fn interests(mut self, interests: Interests) -> Self {
        self.interests = interests;
        self
    }

    /// What happens when the main task returns (default [`WhenDone::Disconnect`]).
    ///
    /// ```no_run
    /// # let ext = ghast_ext::Ext::new(|_bot| async { Ok(()) });
    /// let ext = ext.when_done(ghast_ext::WhenDone::Stay);
    /// ```
    #[must_use]
    pub fn when_done(mut self, when: WhenDone) -> Self {
        self.when_done = when;
        self
    }

    /// Respawn at once after a death (default off: helpers fail with [`Error::Died`] and the
    /// task decides).
    ///
    /// ```no_run
    /// # let ext = ghast_ext::Ext::new(|_bot| async { Ok(()) });
    /// let ext = ext.auto_respawn(true);
    /// ```
    #[must_use]
    pub fn auto_respawn(mut self, on: bool) -> Self {
        self.auto_respawn = on;
        self
    }

    /// Whether running helpers (look, dig, place, walk, ...) fail with
    /// [`Error::Teleported`] when the server teleports the player (default on: after a
    /// setback the plan they made is stale).
    ///
    /// ```no_run
    /// # let ext = ghast_ext::Ext::new(|_bot| async { Ok(()) });
    /// let ext = ext.interrupt_on_teleport(false);
    /// ```
    #[must_use]
    pub fn interrupt_on_teleport(mut self, on: bool) -> Self {
        self.interrupt_on_teleport = on;
        self
    }

    /// A handle to the bot (the same one the tasks get), e.g. for a task of another
    /// executor that sends chat lines.
    ///
    /// ```no_run
    /// # let ext = ghast_ext::Ext::new(|_bot| async { Ok(()) });
    /// let bot = ext.bot();
    /// std::thread::spawn(move || bot.act(|ctx| ctx.chat("from another thread")));
    /// ```
    #[must_use]
    pub fn bot(&self) -> Bot {
        self.bot.clone()
    }

    /// The main task's result in the last session (`None` while running or if it never
    /// finished).
    ///
    /// ```no_run
    /// # let ext = ghast_ext::Ext::new(|_bot| async { Ok(()) });
    /// if let Some(Err(e)) = ext.result() { eprintln!("bot failed: {e}"); }
    /// ```
    #[must_use]
    pub fn result(&self) -> Option<&Result<(), Error>> {
        self.result.as_ref()
    }

    fn begin_session(&mut self, ctx: &TickContext<'_>) {
        self.tasks.clear();
        lock(&self.ready).clear();
        self.result = None;
        self.finished = false;
        {
            let mut s = self.bot.lock();
            s.session += 1;
            s.closed = None;
            s.tick = 0;
            s.requests.clear();
            s.spawned.clear();
            s.aborted.clear();
            s.teleports = 0;
            s.deaths = 0;
            s.interrupt_on_teleport = self.interrupt_on_teleport;
            s.snap.refresh(ctx);
        }
        let fut = (self.factory)(self.bot.clone());
        self.main = Some(self.bot.spawn(fut));
    }

    fn end_session(&mut self, reason: &str) {
        let mut s = self.bot.lock();
        s.closed = Some(reason.to_owned());
        let reqs: Vec<Request> = s.requests.drain(..).collect();
        let spawned = std::mem::take(&mut s.spawned);
        s.wake_ticks();
        s.wake_subs();
        drop(s);
        // Dropped outside the lock: their guards wake the waiters.
        drop(reqs);
        drop(spawned);
        self.tasks.clear();
    }

    /// Polls ready tasks and runs their requests until nothing moves in this tick.
    fn run(&mut self, ctx: &mut TickContext<'_>) {
        for _ in 0..MAX_ROUNDS {
            // New tasks are ready at once; aborted ones go.
            let (spawned, aborted) = {
                let mut s = self.bot.lock();
                (
                    std::mem::take(&mut s.spawned),
                    std::mem::take(&mut s.aborted),
                )
            };
            if !aborted.is_empty() {
                self.tasks.retain(|t| !aborted.contains(&t.id));
            }
            for (id, fut, on_panic) in spawned {
                let waker = Waker::from(Arc::new(TaskWaker {
                    id,
                    ready: self.ready.clone(),
                }));
                self.tasks.push(Task {
                    id,
                    fut,
                    waker,
                    on_panic: Some(on_panic),
                });
                lock(&self.ready).push(id);
            }
            let mut ready = std::mem::take(&mut *lock(&self.ready));
            ready.sort_unstable();
            ready.dedup();
            for id in ready {
                let Some(i) = self.tasks.iter().position(|t| t.id == id) else {
                    continue;
                };
                let t = &mut self.tasks[i];
                let mut cx = Context::from_waker(&t.waker);
                match catch_unwind(AssertUnwindSafe(|| t.fut.as_mut().poll(&mut cx))) {
                    Ok(Poll::Pending) => {}
                    Ok(Poll::Ready(())) => {
                        self.tasks.swap_remove(i);
                    }
                    Err(p) => {
                        let msg = panic_message(&*p);
                        log::error!("[{}] task {id} panicked: {msg}", self.name);
                        let mut t = self.tasks.swap_remove(i);
                        if let Some(h) = t.on_panic.take() {
                            h(msg);
                        }
                    }
                }
            }
            // Run the requests in order (one at a time: a request may queue more).
            let mut ran = false;
            loop {
                let r = self.bot.lock().requests.pop_front();
                let Some(r) = r else { break };
                ran = true;
                r(ctx);
            }
            if ran {
                self.bot.lock().snap.refresh(ctx);
            }
            let idle = lock(&self.ready).is_empty() && {
                let s = self.bot.lock();
                s.spawned.is_empty() && s.aborted.is_empty() && s.requests.is_empty()
            };
            if idle {
                return;
            }
        }
        log::warn!(
            "[{}] t{} tasks still busy after {MAX_ROUNDS} rounds: continuing next tick",
            self.name,
            ctx.tick()
        );
    }
}

/// The text of a panic payload.
pub(crate) fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_owned()
    }
}

impl Behavior for Ext {
    fn name(&self) -> &str {
        &self.name
    }

    fn interests(&self) -> Interests {
        self.interests | self.bot.lock().extra_interests
    }

    fn start(&mut self, ctx: &mut TickContext<'_>) {
        if self.started {
            self.end_session("new session");
        }
        self.started = true;
        self.begin_session(ctx);
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        {
            let mut s = self.bot.lock();
            s.tick += 1;
            s.snap.refresh(ctx);
            let ev = ctx.events();
            s.teleports += ev.teleports.len() as u64;
            if ev.died {
                s.deaths += 1;
            }
            s.publish(ctx);
            s.wake_ticks();
        }
        if self.auto_respawn && ctx.is_dead() {
            ctx.respawn();
        }
        self.run(ctx);
        // The main task finished?
        if self.result.is_none()
            && let Some(h) = &self.main
            && h.is_finished()
        {
            let r = lock(&h.slot).result.take();
            let r = match r {
                Some(Ok(r)) => r,
                Some(Err(e)) => Err(e),
                None => Err(Error::Cancelled),
            };
            self.main = None;
            match (&r, self.when_done) {
                (Ok(()), WhenDone::Disconnect) => ctx.disconnect("done"),
                (Err(e), WhenDone::Disconnect) => ctx.disconnect(format!("failed: {e}")),
                (Ok(()), WhenDone::Stay) => {
                    log::info!("[{}] main task done", self.name);
                }
                (Err(e), WhenDone::Stay) => log::warn!("[{}] main task failed: {e}", self.name),
            }
            self.result = Some(r);
        }
        if self.result.is_some() && self.when_done == WhenDone::Stay && self.tasks.is_empty() {
            self.finished = true;
        }
    }

    fn running(&self) -> bool {
        !self.finished
    }

    fn stop(&mut self, reason: &str) {
        self.end_session(reason);
        if self.result.is_none() && self.main.take().is_some() {
            self.result = Some(Err(Error::Disconnected));
        }
        self.started = false;
    }
}
