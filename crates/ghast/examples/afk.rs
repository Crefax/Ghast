//! AFK bot: connects, joins, stays connected, logs chat and respawns after dying.
//!
//! ```text
//! cargo run -p ghast --release --example afk -- --host localhost --port 25565 \
//!     --username AfkBot --version 1.8.9 [--proxy socks5h://user:pass@host:1080] [--minutes 10]
//! ```

mod support;

use std::time::{Duration, Instant};

use ghast::{Behavior, Interests, SessionEnd, TickContext};

struct Afk {
    started: Instant,
    limit: Option<Duration>,
    dead_ticks: u32,
}

impl Behavior for Afk {
    fn name(&self) -> &str {
        "afk"
    }

    fn interests(&self) -> Interests {
        Interests::CHAT
    }

    fn start(&mut self, ctx: &mut TickContext<'_>) {
        let p = ctx.pose().pos;
        log::info!(
            "afk: in the world at {:.2} {:.2} {:.2} ({})",
            p.x,
            p.y,
            p.z,
            ctx.version()
        );
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat {
            log::info!("chat: {}", m.plain());
        }
        if ctx.events().died {
            log::info!("afk: died");
        }
        if ctx.is_dead() {
            // GuiGameOver enables its buttons after 20 ticks.
            self.dead_ticks += 1;
            if self.dead_ticks == 20 {
                log::info!("afk: respawning");
                ctx.respawn();
            }
        } else {
            self.dead_ticks = 0;
        }
        if ctx.tick().is_multiple_of(1200) {
            let p = ctx.pose().pos;
            log::info!(
                "afk: tick {} at {:.2} {:.2} {:.2}, health {}",
                ctx.tick(),
                p.x,
                p.y,
                p.z,
                ctx.health().health
            );
        }
        if self.limit.is_some_and(|l| self.started.elapsed() >= l) {
            ctx.disconnect("time limit reached");
        }
    }
}

fn main() {
    let opts = support::Options::parse();
    support::init_logger(opts.verbose);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(run(opts));
}

async fn run(opts: support::Options) {
    let client = match opts.client() {
        Ok(c) => c,
        Err(e) => {
            log::error!("{e}");
            std::process::exit(2);
        }
    };
    let mut afk = Afk {
        started: Instant::now(),
        limit: opts.limit(),
        dead_ticks: 0,
    };
    loop {
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        let end = client.run_until(&mut afk, shutdown).await;
        match &end {
            Ok(e) => log::info!("session ended: {e:?}"),
            Err(e) => log::error!("session failed: {e}"),
        }
        let stop = matches!(end, Ok(SessionEnd::Shutdown | SessionEnd::Quit(_)));
        if stop || !opts.reconnect {
            break;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
