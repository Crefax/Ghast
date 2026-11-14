//! Many bots in one process: [`Swarm`].
//!
//! A swarm is a small scheduler, nothing more: every bot is still an independent [`Client`]
//! session. The swarm owns N worker OS threads, each with its own current-thread tokio
//! runtime and a [`LocalSet`](tokio::task::LocalSet); a bot is placed on the least-loaded
//! worker when it is spawned and stays there. Bots on one worker share that thread
//! cooperatively, bots on different workers run in parallel. Nothing is shared between bots,
//! so they may join different servers with different versions.
//!
//! [`Client`]: crate::Client

use std::any::Any;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, mpsc, oneshot};

/// One bot's start-up work, sent to its worker thread.
type Job = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()>>> + Send>;

/// Default thread name prefix of the workers (`ghast-swarm-0`, `ghast-swarm-1`, …).
const DEFAULT_THREAD_NAME: &str = "ghast-swarm";

/// Default time [`Drop`] gives the bots to end on their own before they are aborted.
const DEFAULT_DROP_GRACE: Duration = Duration::from_secs(5);

/// Minimum time a shutdown waits for the workers after aborting their bots.
const ABORT_WAIT_MIN: Duration = Duration::from_secs(1);

/// A one-way flag with async waiters.
#[derive(Default)]
struct Signal {
    set: AtomicBool,
    notify: Notify,
}

impl Signal {
    fn fire(&self) {
        self.set.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    fn is_set(&self) -> bool {
        self.set.load(Ordering::SeqCst)
    }

    async fn wait(&self) {
        loop {
            // A `Notified` receives `notify_waiters` from the moment it is created, so checking
            // the flag after creating it cannot miss a `fire`.
            let notified = self.notify.notified();
            if self.is_set() {
                return;
            }
            notified.await;
        }
    }
}

/// State shared by the swarm, its workers, its bots and their handles.
struct Shared {
    /// Soft stop of every bot (their [`BotContext::stopped`] completes).
    stop: Signal,
    /// Hard stop: the workers drop every bot that is still running.
    abort: Signal,
    /// Minimum spacing between bot starts.
    join_delay: Duration,
    /// When the most recently spawned bot is allowed to start.
    last_start: Mutex<Option<Instant>>,
    /// Worker threads still running.
    alive: Mutex<usize>,
    alive_changed: Condvar,
    next_id: AtomicU64,
}

impl Shared {
    /// The start slot of a bot spawned now: at least `join_delay` after the previous one.
    fn start_slot(&self) -> Option<Instant> {
        if self.join_delay.is_zero() {
            return None;
        }
        let now = Instant::now();
        let mut last = self
            .last_start
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let at = match *last {
            Some(prev) => (prev + self.join_delay).max(now),
            None => now,
        };
        *last = Some(at);
        Some(at)
    }

    /// Waits until at most `target` workers are alive or `deadline` passes; true if reached.
    fn wait_alive(&self, target: usize, deadline: Instant) -> bool {
        let mut alive = self.alive.lock().unwrap_or_else(PoisonError::into_inner);
        while *alive > target {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            alive = self
                .alive_changed
                .wait_timeout(alive, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

/// Builder of a [`Swarm`] ([`Swarm::builder`]).
#[derive(Debug, Clone)]
pub struct SwarmBuilder {
    workers: Option<usize>,
    join_delay: Duration,
    thread_name: String,
    drop_grace: Duration,
}

impl Default for SwarmBuilder {
    fn default() -> Self {
        SwarmBuilder {
            workers: None,
            join_delay: Duration::ZERO,
            thread_name: DEFAULT_THREAD_NAME.to_owned(),
            drop_grace: DEFAULT_DROP_GRACE,
        }
    }
}

impl SwarmBuilder {
    /// Number of worker threads (default: [`std::thread::available_parallelism`]; at least 1).
    #[must_use]
    pub fn workers(mut self, n: usize) -> Self {
        self.workers = Some(n.max(1));
        self
    }

    /// Minimum time between two bot starts across the whole swarm (default: none). Servers
    /// often throttle logins per address; with a delay, `spawn` still returns at once and the
    /// bot waits on its worker until its start slot. Slots are handed out at spawn time, in
    /// spawn order, each at least `delay` after the previous one (and not before the spawn);
    /// a bot never starts before its slot, but a busy worker may start it a little later.
    #[must_use]
    pub fn join_delay(mut self, delay: Duration) -> Self {
        self.join_delay = delay;
        self
    }

    /// Name prefix of the worker threads (default `ghast-swarm`; threads are `<prefix>-<n>`).
    #[must_use]
    pub fn thread_name(mut self, prefix: impl Into<String>) -> Self {
        self.thread_name = prefix.into();
        self
    }

    /// How long dropping the swarm waits for the bots to end on their own after the stop
    /// signal, before the rest are aborted (default 5 s). See [`Swarm::shutdown`].
    #[must_use]
    pub fn drop_grace(mut self, grace: Duration) -> Self {
        self.drop_grace = grace;
        self
    }

    /// Starts the worker threads.
    ///
    /// # Errors
    ///
    /// Creating a runtime or a thread failed (the threads started so far are stopped again).
    pub fn build(self) -> std::io::Result<Swarm> {
        let n = self.workers.unwrap_or_else(|| {
            thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
        });
        let shared = Arc::new(Shared {
            stop: Signal::default(),
            abort: Signal::default(),
            join_delay: self.join_delay,
            last_start: Mutex::new(None),
            alive: Mutex::new(0),
            alive_changed: Condvar::new(),
            next_id: AtomicU64::new(0),
        });
        let mut swarm = Swarm {
            shared: Arc::clone(&shared),
            workers: Vec::with_capacity(n),
            cursor: Mutex::new(0),
            drop_grace: self.drop_grace,
            done: false,
        };
        for i in 0..n {
            // A failed start drops `swarm`, which stops the workers started so far.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let (tx, rx) = mpsc::unbounded_channel();
            *shared.alive.lock().unwrap_or_else(PoisonError::into_inner) += 1;
            let worker_shared = Arc::clone(&shared);
            let spawned = thread::Builder::new()
                .name(format!("{}-{i}", self.thread_name))
                .spawn(move || worker_main(rt, rx, worker_shared));
            let thread = match spawned {
                Ok(t) => t,
                Err(e) => {
                    *shared.alive.lock().unwrap_or_else(PoisonError::into_inner) -= 1;
                    return Err(e);
                }
            };
            swarm.workers.push(Worker {
                tx: Some(tx),
                load: Arc::new(AtomicUsize::new(0)),
                thread_id: thread.thread().id(),
                thread: Some(thread),
            });
        }
        Ok(swarm)
    }
}

/// Marks a worker thread as gone when it exits (also by a panic).
struct AliveGuard(Arc<Shared>);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        let mut alive = self.0.alive.lock().unwrap_or_else(PoisonError::into_inner);
        *alive -= 1;
        self.0.alive_changed.notify_all();
    }
}

fn worker_main(
    rt: tokio::runtime::Runtime,
    mut rx: mpsc::UnboundedReceiver<Job>,
    shared: Arc<Shared>,
) {
    let _alive = AliveGuard(Arc::clone(&shared));
    let local = tokio::task::LocalSet::new();
    // Accept bots until the swarm closes the channel (shutdown / drop) or aborts.
    let aborted = rt.block_on(local.run_until(async {
        let accept = async {
            while let Some(job) = rx.recv().await {
                drop(tokio::task::spawn_local(job()));
            }
        };
        tokio::select! {
            () = accept => false,
            () = shared.abort.wait() => true,
        }
    }));
    if !aborted {
        // Then run the remaining bots to their end, unless the swarm aborts them.
        rt.block_on(async {
            tokio::select! {
                () = local => {}
                () = shared.abort.wait() => {}
            }
        });
    } else {
        // Dropping the set drops every bot still in it; their handles report `Aborted`.
        let _enter = rt.enter();
        drop(local);
    }
    drop(rt);
}

/// One worker thread as the swarm sees it.
struct Worker {
    /// `None` once the swarm closed it.
    tx: Option<mpsc::UnboundedSender<Job>>,
    /// Bots assigned to this worker that have not ended yet.
    load: Arc<AtomicUsize>,
    thread_id: ThreadId,
    thread: Option<thread::JoinHandle<()>>,
}

/// Decrements a worker's load when a bot ends (also when it is aborted).
struct LoadGuard(Arc<AtomicUsize>);

impl Drop for LoadGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Many bots in one process, spread over a few worker threads.
///
/// Each worker thread runs a current-thread tokio runtime, which is the cheap layout for many
/// bots (a multi-thread runtime moves bots between threads and pays for the wake-ups). Each
/// bot is still an independent [`Client`](crate::Client) session: the swarm only decides which
/// thread runs it and when it starts.
///
/// [`Swarm::spawn`] takes a closure that is sent to the bot's worker thread and called there,
/// so the future it returns does not need to be `Send` (behaviours may hold `Rc`s, for
/// example). The bot gets a [`BotContext`] whose [`stopped`](BotContext::stopped) future is
/// meant for [`Client::run_until`](crate::Client::run_until). A bot that panics only ends
/// itself: its [`BotHandle`] reports [`BotError::Panicked`], the other bots and the worker go
/// on.
///
/// ```no_run
/// use std::time::Duration;
/// use ghast_client::{Account, Behavior, Client, GameVersion, Swarm, TickContext};
///
/// struct Afk;
/// impl Behavior for Afk {
///     fn tick(&mut self, ctx: &mut TickContext<'_>) {
///         if ctx.is_dead() {
///             ctx.respawn();
///         }
///     }
/// }
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let swarm = Swarm::builder()
///     .join_delay(Duration::from_millis(200))
///     .build()?;
/// let mut bots = Vec::new();
/// for i in 0..100 {
///     let client = Client::new(
///         "localhost:25565".parse()?,
///         GameVersion::V1_8_9,
///         Account::offline(&format!("Bot{i}"))?,
///     );
///     bots.push(swarm.spawn(move |bot| async move {
///         client.run_until(&mut Afk, bot.stopped()).await
///     }));
/// }
/// // ... later: ask every bot to log out, give them 10 s, abort the rest.
/// let report = swarm.shutdown(Duration::from_secs(10));
/// for bot in bots {
///     println!("{:?}", bot.join_blocking());
/// }
/// println!("{} bots had to be aborted", report.aborted);
/// # Ok(())
/// # }
/// ```
///
/// Dropping the swarm does the same as [`shutdown`](Swarm::shutdown) with the builder's
/// [`drop_grace`](SwarmBuilder::drop_grace), so it blocks for a bounded time. [`BotHandle`]s stay usable after the swarm is gone, and
/// dropping a handle does not stop its bot.
pub struct Swarm {
    shared: Arc<Shared>,
    workers: Vec<Worker>,
    /// Round-robin start of the least-loaded search; the lock also makes picking a worker and
    /// counting the bot on it one step.
    cursor: Mutex<usize>,
    drop_grace: Duration,
    /// Set once the workers are shut down.
    done: bool,
}

impl fmt::Debug for Swarm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Swarm")
            .field("workers", &self.workers.len())
            .field("loads", &self.worker_loads())
            .field("join_delay", &self.shared.join_delay)
            .field("stopping", &self.shared.stop.is_set())
            .finish()
    }
}

impl Swarm {
    /// A builder (worker count, join delay, thread names).
    #[must_use]
    pub fn builder() -> SwarmBuilder {
        SwarmBuilder::default()
    }

    /// A swarm with the default settings: one worker per available core, no join delay.
    ///
    /// # Errors
    ///
    /// See [`SwarmBuilder::build`].
    pub fn new() -> std::io::Result<Swarm> {
        SwarmBuilder::default().build()
    }

    /// Number of worker threads.
    #[must_use]
    pub fn workers(&self) -> usize {
        self.workers.len()
    }

    /// Bots that were spawned and have not ended yet (including those waiting for their start
    /// slot).
    #[must_use]
    pub fn bots(&self) -> usize {
        self.worker_loads().iter().sum()
    }

    /// Live bots per worker thread.
    #[must_use]
    pub fn worker_loads(&self) -> Vec<usize> {
        self.workers
            .iter()
            .map(|w| w.load.load(Ordering::SeqCst))
            .collect()
    }

    /// Starts a bot. `bot` is sent to the least-loaded worker thread (ties: round-robin) and
    /// called there once the bot's start slot (see [`SwarmBuilder::join_delay`]) has come; the
    /// bot runs until the future it returns completes. Returns at once.
    ///
    /// A bot spawned after [`stop_all`](Swarm::stop_all) does not start; its handle reports
    /// [`BotError::NotStarted`].
    pub fn spawn<F, Fut>(&self, bot: F) -> BotHandle<Fut::Output>
    where
        F: FnOnce(BotContext) -> Fut + Send + 'static,
        Fut: Future + 'static,
        Fut::Output: Send + 'static,
    {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let stop = Arc::new(Signal::default());
        let (tx, rx) = oneshot::channel();
        let handle = BotHandle {
            id,
            worker: 0,
            stop: Arc::clone(&stop),
            rx,
        };
        if self.done || self.workers.is_empty() {
            let _ = tx.send(Err(BotError::NotStarted));
            return handle;
        }
        let (worker, load) = self.pick_worker();
        let start = self.shared.start_slot();
        let ctx = BotContext {
            id,
            worker,
            stop,
            shared: Arc::clone(&self.shared),
        };
        let load = LoadGuard(load);
        let job: Job = Box::new(move || Box::pin(run_bot(ctx, start, load, bot, tx)));
        if let Some(sender) = &self.workers[worker].tx {
            // A send only fails when the worker is gone; the dropped job (and its result
            // sender) then makes the handle report `Aborted`.
            let _ = sender.send(job);
        }
        BotHandle { worker, ..handle }
    }

    fn pick_worker(&self) -> (usize, Arc<AtomicUsize>) {
        let mut cursor = self.cursor.lock().unwrap_or_else(PoisonError::into_inner);
        let n = self.workers.len();
        let mut best = *cursor % n;
        let mut best_load = usize::MAX;
        for k in 0..n {
            let i = (*cursor + k) % n;
            let load = self.workers[i].load.load(Ordering::SeqCst);
            if load < best_load {
                best = i;
                best_load = load;
            }
        }
        *cursor = (best + 1) % n;
        let load = Arc::clone(&self.workers[best].load);
        load.fetch_add(1, Ordering::SeqCst);
        (best, load)
    }

    /// Asks every bot to stop: their [`BotContext::stopped`] futures complete (a bot running
    /// [`Client::run_until`](crate::Client::run_until) with it logs out). Does not wait; bots
    /// spawned afterwards do not start. Await the [`BotHandle`]s, or call
    /// [`shutdown`](Swarm::shutdown), to wait for the bots.
    pub fn stop_all(&self) {
        self.shared.stop.fire();
    }

    /// Stops every bot and the worker threads: fires [`stop_all`](Swarm::stop_all), waits up to
    /// `grace` for the bots to end on their own, then aborts the rest (their futures are
    /// dropped and their handles report [`BotError::Aborted`]) and joins the threads.
    ///
    /// Blocks the calling thread for at most `grace` plus, if bots had to be aborted, up to
    /// `max(grace, 1 s)` more; a worker still blocked then (a bot blocking its thread without
    /// awaiting) is left running detached and counted in
    /// [`ShutdownReport::stuck_workers`]. From async code,
    /// call [`stop_all`](Swarm::stop_all) and await the handles instead, then drop the swarm.
    pub fn shutdown(mut self, grace: Duration) -> ShutdownReport {
        self.shutdown_inner(grace)
    }

    fn shutdown_inner(&mut self, grace: Duration) -> ShutdownReport {
        if self.done {
            return ShutdownReport {
                aborted: 0,
                stuck_workers: 0,
            };
        }
        self.done = true;
        self.shared.stop.fire();
        for w in &mut self.workers {
            // Closing the channel lets the worker exit once its bots have ended.
            w.tx = None;
        }
        // Called from one of our own workers (a bot dropped the swarm): that worker cannot
        // finish while we block it, so only wait for the others and leave it detached.
        let me = thread::current().id();
        let own = self.workers.iter().any(|w| w.thread_id == me);
        let target = usize::from(own);
        let deadline = Instant::now() + grace;
        let mut aborted = 0;
        if !self.shared.wait_alive(target, deadline) {
            aborted = self.bots();
            self.shared.abort.fire();
            // The workers drop their bots at their next await point. A bot that blocks its
            // thread never gets there: do not wait for it forever.
            let again = grace.max(ABORT_WAIT_MIN);
            self.shared.wait_alive(target, Instant::now() + again);
        }
        let mut stuck_workers = 0;
        for w in &mut self.workers {
            if let Some(t) = w.thread.take() {
                if w.thread_id == me {
                    continue;
                }
                if t.is_finished() {
                    let _ = t.join();
                } else {
                    // Detached: it exits on its own once the blocking bot returns.
                    stuck_workers += 1;
                }
            }
        }
        if stuck_workers > 0 {
            log::warn!("swarm: {stuck_workers} worker thread(s) blocked by a bot, left detached");
        }
        ShutdownReport {
            aborted,
            stuck_workers,
        }
    }
}

impl Drop for Swarm {
    fn drop(&mut self) {
        let grace = self.drop_grace;
        self.shutdown_inner(grace);
    }
}

/// What [`Swarm::shutdown`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ShutdownReport {
    /// Bots that had not ended within the grace time and were aborted.
    pub aborted: usize,
    /// Worker threads that were still blocked after the abort (a bot blocking its thread
    /// without awaiting) and were left running detached; normally 0.
    pub stuck_workers: usize,
}

async fn run_bot<F, Fut>(
    ctx: BotContext,
    start: Option<Instant>,
    load: LoadGuard,
    bot: F,
    tx: oneshot::Sender<Result<Fut::Output, BotError>>,
) where
    F: FnOnce(BotContext) -> Fut + 'static,
    Fut: Future + 'static,
{
    if let Some(at) = start {
        tokio::select! {
            () = tokio::time::sleep_until(at.into()) => {}
            () = ctx.stopped() => {}
        }
    }
    let result = if ctx.is_stopped() {
        Err(BotError::NotStarted)
    } else {
        // Its own task, so a panic (also one in `bot` itself) is caught by tokio and only
        // ends this bot.
        match tokio::task::spawn_local(async move { bot(ctx).await }).await {
            Ok(v) => Ok(v),
            Err(e) => match e.try_into_panic() {
                Ok(payload) => Err(BotError::Panicked(panic_message(payload.as_ref()))),
                Err(_) => Err(BotError::Aborted),
            },
        }
    };
    // Load first, so a caller that saw the result also sees the lower load.
    drop(load);
    let _ = tx.send(result);
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "a non-string panic payload".to_owned()
    }
}

/// What a bot gets from the swarm when it starts.
pub struct BotContext {
    id: u64,
    worker: usize,
    stop: Arc<Signal>,
    shared: Arc<Shared>,
}

impl fmt::Debug for BotContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BotContext")
            .field("id", &self.id)
            .field("worker", &self.worker)
            .field("stopped", &self.is_stopped())
            .finish()
    }
}

impl BotContext {
    /// The bot's number in the swarm (0, 1, … in spawn order).
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The worker thread the bot runs on (`0..Swarm::workers()`).
    #[must_use]
    pub fn worker(&self) -> usize {
        self.worker
    }

    /// True once the bot was asked to stop ([`BotHandle::stop`], [`Swarm::stop_all`],
    /// [`Swarm::shutdown`] or dropping the swarm).
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.stop.is_set() || self.shared.stop.is_set()
    }

    /// Completes once the bot is asked to stop; pass it to
    /// [`Client::run_until`](crate::Client::run_until). Can be called any number of times
    /// (e.g. once per reconnect).
    pub fn stopped(&self) -> impl Future<Output = ()> + Send + 'static {
        let own = Arc::clone(&self.stop);
        let shared = Arc::clone(&self.shared);
        async move {
            tokio::select! {
                () = own.wait() => {}
                () = shared.stop.wait() => {}
            }
        }
    }
}

/// Why a bot has no result.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BotError {
    /// The bot panicked (the panic message).
    Panicked(String),
    /// The bot was asked to stop before its start slot, or spawned after the swarm stopped.
    NotStarted,
    /// The bot was still running when the swarm aborted it (shutdown grace time over, or the
    /// swarm dropped).
    Aborted,
}

impl fmt::Display for BotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BotError::Panicked(m) => write!(f, "bot panicked: {m}"),
            BotError::NotStarted => f.write_str("bot was stopped before it started"),
            BotError::Aborted => f.write_str("bot was aborted by the swarm"),
        }
    }
}

impl std::error::Error for BotError {}

/// A bot in a [`Swarm`]: stop it, wait for its result. Dropping the handle does not stop
/// the bot. Works with any runtime (or none, see [`join_blocking`](BotHandle::join_blocking)),
/// also after the swarm is gone.
pub struct BotHandle<T> {
    id: u64,
    worker: usize,
    stop: Arc<Signal>,
    rx: oneshot::Receiver<Result<T, BotError>>,
}

impl<T> fmt::Debug for BotHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BotHandle")
            .field("id", &self.id)
            .field("worker", &self.worker)
            .finish_non_exhaustive()
    }
}

impl<T> BotHandle<T> {
    /// The bot's number in the swarm ([`BotContext::id`]).
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The worker thread the bot was assigned to.
    #[must_use]
    pub fn worker(&self) -> usize {
        self.worker
    }

    /// Asks this bot to stop (its [`BotContext::stopped`] completes). Does not wait.
    pub fn stop(&self) {
        self.stop.fire();
    }

    /// Waits for the bot's end and returns what its future returned.
    ///
    /// # Errors
    ///
    /// The bot panicked, never started or was aborted ([`BotError`]).
    pub async fn join(self) -> Result<T, BotError> {
        self.rx.await.unwrap_or(Err(BotError::Aborted))
    }

    /// [`join`](BotHandle::join) for synchronous code.
    ///
    /// # Errors
    ///
    /// See [`join`](BotHandle::join).
    ///
    /// # Panics
    ///
    /// When called from inside an async runtime (it blocks the thread).
    pub fn join_blocking(self) -> Result<T, BotError> {
        self.rx.blocking_recv().unwrap_or(Err(BotError::Aborted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc as std_mpsc;

    #[test]
    fn assignment_spreads_over_workers() {
        let swarm = Swarm::builder()
            .workers(4)
            .thread_name("spread")
            .build()
            .unwrap();
        assert_eq!(swarm.workers(), 4);
        let (started, wait_started) = std_mpsc::channel();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let started = started.clone();
                swarm.spawn(move |bot| async move {
                    let name = thread::current().name().map(str::to_owned);
                    started.send(()).unwrap();
                    bot.stopped().await;
                    (bot.worker(), name)
                })
            })
            .collect();
        let mut per_worker = [0; 4];
        for h in &handles {
            per_worker[h.worker()] += 1;
        }
        assert_eq!(per_worker, [2, 2, 2, 2]);
        assert_eq!(swarm.bots(), 8);
        assert_eq!(swarm.worker_loads(), vec![2, 2, 2, 2]);
        for _ in 0..8 {
            wait_started.recv().unwrap();
        }
        swarm.stop_all();
        for h in handles {
            let assigned = h.worker();
            let (worker, name) = h.join_blocking().unwrap();
            assert_eq!(worker, assigned);
            assert_eq!(name.as_deref(), Some(format!("spread-{worker}").as_str()));
        }
        assert_eq!(swarm.bots(), 0);
    }

    #[test]
    fn least_loaded_worker_is_picked() {
        let swarm = Swarm::builder().workers(3).build().unwrap();
        let (started, wait_started) = std_mpsc::channel();
        let idle = |started: std_mpsc::Sender<()>| {
            move |bot: BotContext| async move {
                started.send(()).unwrap();
                bot.stopped().await;
            }
        };
        let a = swarm.spawn(idle(started.clone()));
        let b = swarm.spawn(idle(started.clone()));
        let c = swarm.spawn(idle(started.clone()));
        assert_eq!((a.worker(), b.worker(), c.worker()), (0, 1, 2));
        for _ in 0..3 {
            wait_started.recv().unwrap();
        }
        b.stop();
        b.join_blocking().unwrap();
        // Worker 1 is now the only empty one.
        let d = swarm.spawn(idle(started.clone()));
        assert_eq!(d.worker(), 1);
        wait_started.recv().unwrap();
        assert_eq!(swarm.shutdown(Duration::from_secs(5)).aborted, 0);
        for h in [a, c, d] {
            h.join_blocking().unwrap();
        }
    }

    #[test]
    fn join_delay_is_respected() {
        let delay = Duration::from_millis(60);
        let swarm = Swarm::builder()
            .workers(2)
            .join_delay(delay)
            .build()
            .unwrap();
        let t0 = Instant::now();
        let handles: Vec<_> = (0..4)
            .map(|_| swarm.spawn(|_| async { Instant::now() }))
            .collect();
        let starts: Vec<Instant> = handles
            .into_iter()
            .map(|h| h.join_blocking().unwrap())
            .collect();
        for (k, start) in starts.iter().enumerate() {
            // Bot k never starts before its slot, k delays after the first spawn. (A busy
            // worker may start it later, so the measured gaps between two bots can be shorter.)
            // The timer wheel has 1 ms granularity and never fires early; allow 1 ms of rounding.
            let after = start.duration_since(t0) + Duration::from_millis(1);
            assert!(after >= delay * k as u32, "bot {k} started after {after:?}");
        }
        // And the slots do not drift: the last bot is not far behind its slot either.
        let last = starts[3].duration_since(t0);
        assert!(
            last < delay * 3 + Duration::from_secs(2),
            "last bot after {last:?}"
        );
    }

    #[test]
    fn panicking_bot_does_not_affect_others() {
        let swarm = Swarm::builder().workers(1).build().unwrap();
        let ok1 = swarm.spawn(|_| async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            1
        });
        let bad = swarm.spawn(|_| async {
            tokio::task::yield_now().await;
            panic!("boom");
        });
        let bad_early = swarm.spawn(|_| -> std::future::Ready<i32> { panic!("in the factory") });
        let ok2 = swarm.spawn(|_| async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            2
        });
        assert_eq!(
            bad.join_blocking(),
            Err::<(), _>(BotError::Panicked("boom".into()))
        );
        assert_eq!(
            bad_early.join_blocking(),
            Err(BotError::Panicked("in the factory".into()))
        );
        assert_eq!(ok1.join_blocking(), Ok(1));
        assert_eq!(ok2.join_blocking(), Ok(2));
        // The worker is still alive.
        assert_eq!(swarm.spawn(|_| async { 3 }).join_blocking(), Ok(3));
    }

    #[test]
    fn shutdown_stops_everything() {
        let swarm = Swarm::builder().workers(2).build().unwrap();
        let (started, wait_started) = std_mpsc::channel();
        let polite: Vec<_> = (0..4)
            .map(|i| {
                let started = started.clone();
                swarm.spawn(move |bot| async move {
                    started.send(()).unwrap();
                    bot.stopped().await;
                    i
                })
            })
            .collect();
        let stubborn = swarm.spawn(move |_| async move {
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        for _ in 0..5 {
            wait_started.recv().unwrap();
        }
        let t0 = Instant::now();
        let report = swarm.shutdown(Duration::from_millis(200));
        let took = t0.elapsed();
        assert_eq!(report.aborted, 1);
        assert!(took < Duration::from_secs(2), "shutdown took {took:?}");
        for (i, h) in polite.into_iter().enumerate() {
            assert_eq!(h.join_blocking(), Ok(i));
        }
        assert_eq!(stubborn.join_blocking(), Err(BotError::Aborted));
    }

    #[test]
    fn shutdown_does_not_hang_on_a_blocking_bot() {
        let swarm = Swarm::builder().workers(2).build().unwrap();
        let (started, wait_started) = std_mpsc::channel();
        let started2 = started.clone();
        let blocker = swarm.spawn(move |_| async move {
            started.send(()).unwrap();
            // Blocks its worker thread without ever awaiting.
            thread::sleep(Duration::from_secs(3));
        });
        let other = swarm.spawn(move |bot| async move {
            started2.send(()).unwrap();
            bot.stopped().await;
        });
        wait_started.recv().unwrap();
        wait_started.recv().unwrap();
        let t0 = Instant::now();
        let report = swarm.shutdown(Duration::from_millis(100));
        let took = t0.elapsed();
        assert_eq!(report.stuck_workers, 1);
        assert!(took < Duration::from_millis(2500), "shutdown took {took:?}");
        assert_eq!(other.join_blocking(), Ok(()));
        // The blocked worker finishes the bot's synchronous part, then drops it at its next
        // await point (its end): the handle reports the result or the abort, never hangs.
        let _ = blocker.join_blocking();
    }

    #[test]
    fn stop_before_start_slot() {
        let swarm = Swarm::builder()
            .workers(1)
            .join_delay(Duration::from_secs(30))
            .build()
            .unwrap();
        let first = swarm.spawn(|_| async { 1 });
        let second = swarm.spawn(|_| async { 2 });
        assert_eq!(first.join_blocking(), Ok(1));
        second.stop();
        assert_eq!(second.join_blocking(), Err(BotError::NotStarted));
        swarm.stop_all();
        assert_eq!(
            swarm.spawn(|_| async { 3 }).join_blocking(),
            Err(BotError::NotStarted)
        );
    }

    #[test]
    fn handles_outlive_the_swarm() {
        let swarm = Swarm::builder()
            .workers(2)
            .drop_grace(Duration::from_secs(5))
            .build()
            .unwrap();
        let (started, wait_started) = std_mpsc::channel();
        let h = swarm.spawn(move |bot| async move {
            started.send(()).unwrap();
            bot.stopped().await;
            "logged out"
        });
        wait_started.recv().unwrap();
        drop(swarm);
        assert_eq!(h.join_blocking(), Ok("logged out"));
    }

    #[test]
    fn swarm_outlives_dropped_handles() {
        let swarm = Swarm::builder().workers(2).build().unwrap();
        let (tx, rx) = std_mpsc::channel();
        let h = swarm.spawn(move |_| async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            tx.send("still ran").unwrap();
        });
        drop(h);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok("still ran"));
        assert_eq!(swarm.spawn(|_| async { 7 }).join_blocking(), Ok(7));
    }

    #[test]
    fn async_join_from_another_runtime() {
        let swarm = Swarm::builder().workers(1).build().unwrap();
        let h = swarm.spawn(|bot| async move { bot.id() + 10 });
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        assert_eq!(rt.block_on(h.join()), Ok(10));
    }

    #[test]
    fn non_send_futures_run() {
        let swarm = Swarm::builder().workers(1).build().unwrap();
        let h = swarm.spawn(|_| async {
            let rc = std::rc::Rc::new(5);
            tokio::task::yield_now().await;
            *rc
        });
        assert_eq!(h.join_blocking(), Ok(5));
    }

    #[test]
    fn swarm_dropped_inside_a_bot() {
        let swarm = Swarm::builder()
            .workers(2)
            .drop_grace(Duration::from_millis(100))
            .build()
            .unwrap();
        let (started, wait_started) = std_mpsc::channel();
        let other = swarm.spawn(move |bot| async move {
            started.send(()).unwrap();
            bot.stopped().await;
        });
        wait_started.recv().unwrap();
        let (tx, rx) = std_mpsc::channel::<Swarm>();
        let dropper = swarm.spawn(move |_| async move {
            let s = rx.recv().unwrap();
            drop(s);
            "dropped"
        });
        tx.send(swarm).unwrap();
        assert_eq!(dropper.join_blocking(), Ok("dropped"));
        assert_eq!(other.join_blocking(), Ok(()));
    }
}
