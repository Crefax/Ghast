//! Many bots in one process with [`ghast::Swarm`]: N bots join the server and wander (a new
//! random direction every 1–3 s, mostly walking, sometimes jumping, now and then a short stop).
//! Usernames are `<username>0`, `<username>1`, …
//!
//! ```text
//! cargo run -p ghast --release --example swarm -- --count 50 --workers 1 --join-delay-ms 200 \
//!     --host localhost --port 25565 --username Wander --version 26.2 [--minutes 10]
//! ```
//!
//! Every option of the other examples works (`--proxy`, `--reconnect`, …). `--count`,
//! `--workers` and `--join-delay-ms` can also come from `GHAST_SWARM_COUNT`,
//! `GHAST_SWARM_WORKERS` and `GHAST_SWARM_JOIN_DELAY_MS`. Ctrl-C makes every bot log out.

mod support;

use std::time::{Duration, Instant};

use ghast::{Behavior, Controls, SessionEnd, Swarm, TickContext};

/// Wanders around: random yaw every 20–60 ticks, mostly walking, sometimes jumping.
struct Wander {
    rng: u64,
    /// Ticks until the next change of direction.
    next_turn: u32,
    yaw: f32,
    controls: Controls,
    dead_ticks: u32,
    started: Instant,
    limit: Option<Duration>,
}

impl Wander {
    fn new(seed: u64, limit: Option<Duration>) -> Self {
        Wander {
            // xorshift must not start at 0.
            rng: seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
            next_turn: 0,
            yaw: 0.0,
            controls: Controls::NONE,
            dead_ticks: 0,
            started: Instant::now(),
            limit,
        }
    }

    fn rand(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    fn turn(&mut self, ctx: &TickContext<'_>) {
        let cur = ctx.pose().yaw;
        let target = (self.rand() * 360.0 - 180.0) as f32;
        self.yaw = cur + ghast::rotation::wrap_degrees(target - cur);
        let r = self.rand();
        self.controls = Controls {
            forward: r < 0.9,
            // One leg in four jumps all the way.
            jump: r < 0.225,
            ..Controls::NONE
        };
        self.next_turn = 20 + (self.rand() * 40.0) as u32;
    }
}

impl Behavior for Wander {
    fn name(&self) -> &str {
        "wander"
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        if self.limit.is_some_and(|l| self.started.elapsed() >= l) {
            ctx.disconnect("time limit reached");
            return;
        }
        if ctx.is_dead() {
            // DeathScreen enables its buttons after 20 ticks.
            self.dead_ticks += 1;
            if self.dead_ticks == 20 {
                ctx.respawn();
            }
            return;
        }
        self.dead_ticks = 0;
        if self.next_turn == 0 {
            self.turn(ctx);
        }
        self.next_turn -= 1;
        ctx.set_aim(self.yaw, 0.0);
        ctx.set_controls(self.controls);
    }
}

/// Takes `--name value` out of `args` (or reads `env`), parsed as `T`.
fn take_opt<T: std::str::FromStr>(args: &mut Vec<String>, name: &str, env: &str) -> Option<T> {
    let raw = match args.iter().position(|a| a == name) {
        Some(i) if i + 1 < args.len() => {
            let v = args.remove(i + 1);
            args.remove(i);
            Some(v)
        }
        Some(_) => {
            eprintln!("{name} needs a value");
            std::process::exit(2);
        }
        None => std::env::var(env).ok(),
    }?;
    match raw.parse() {
        Ok(v) => Some(v),
        Err(_) => {
            eprintln!("{name}: cannot parse {raw:?}");
            std::process::exit(2);
        }
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let count: usize = take_opt(&mut args, "--count", "GHAST_SWARM_COUNT").unwrap_or(10);
    let workers: Option<usize> = take_opt(&mut args, "--workers", "GHAST_SWARM_WORKERS");
    let join_delay_ms: u64 =
        take_opt(&mut args, "--join-delay-ms", "GHAST_SWARM_JOIN_DELAY_MS").unwrap_or(100);
    let mut opts = support::Options::parse_from(args);
    support::init_logger(opts.verbose);

    let mut builder = Swarm::builder().join_delay(Duration::from_millis(join_delay_ms));
    if let Some(n) = workers {
        builder = builder.workers(n);
    }
    let swarm = builder.build().expect("swarm worker threads");
    log::info!(
        "swarm: {count} bots on {} worker threads, {join_delay_ms} ms between joins",
        swarm.workers()
    );

    let base = opts.username.clone();
    let mut bots = Vec::with_capacity(count);
    for i in 0..count {
        opts.username = format!("{base}{i}");
        let client = match opts.client() {
            Ok(c) => c,
            Err(e) => {
                log::error!("{}: {e}", opts.username);
                std::process::exit(2);
            }
        };
        let name = opts.username.clone();
        let (seed, limit, reconnect) = (opts.seed ^ i as u64, opts.limit(), opts.reconnect);
        let handle = swarm.spawn(move |bot| async move {
            let mut wander = Wander::new(seed, limit);
            loop {
                let end = client.run_until(&mut wander, bot.stopped()).await;
                match &end {
                    Ok(e) => log::info!("{}: session ended: {e:?}", client.account().name()),
                    Err(e) => log::warn!("{}: session failed: {e}", client.account().name()),
                }
                let done = matches!(end, Ok(SessionEnd::Shutdown | SessionEnd::Quit(_)));
                if done || !reconnect || bot.is_stopped() {
                    return end;
                }
                tokio::select! {
                    () = tokio::time::sleep(Duration::from_secs(5)) => {}
                    () = bot.stopped() => return end,
                }
            }
        });
        bots.push((name, handle));
    }

    // The main thread only waits: for every bot to end, or for Ctrl-C.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async {
        let wait_all = async {
            for (name, handle) in bots {
                if let Err(e) = handle.join().await {
                    log::error!("{name}: {e}");
                }
            }
        };
        tokio::pin!(wait_all);
        tokio::select! {
            () = &mut wait_all => return,
            _ = tokio::signal::ctrl_c() => {}
        }
        log::info!("swarm: stopping {} bots", swarm.bots());
        swarm.stop_all();
        if tokio::time::timeout(Duration::from_secs(10), wait_all)
            .await
            .is_err()
        {
            log::warn!("swarm: some bots did not log out within 10 s");
        }
    });
    let report = swarm.shutdown(Duration::from_secs(2));
    if report.aborted > 0 {
        log::warn!("swarm: {} bots aborted", report.aborted);
    }
}
