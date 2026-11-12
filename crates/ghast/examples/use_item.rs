//! Item use loop for the lab: looks at the sky and holds the right button with whatever is in
//! hotbar slot 0 (give food first, e.g. `lab/console.sh 1.8 "give GhastBot2 bread 16"` while
//! the bot is online), releases after 40 ticks, waits, repeats; walks forward while using to
//! exercise the ×0.2 input slowdown. Grim alerts in chat are logged as `ALERT`.
//!
//! ```text
//! cargo run -p ghast --release --example use_item -- --host 127.0.0.1 --port 25618 \
//!     --username GhastBot2 --version 1.8.9 --minutes 2
//! ```

mod support;

use std::time::{Duration, Instant};

use ghast::{Behavior, Buttons, Controls, Interests, TickContext};

struct UseItem {
    n: u64,
    dead_ticks: u32,
    launched: bool,
    was_using: bool,
    alerts: u32,
    started: Instant,
    limit: Option<Duration>,
}

impl Behavior for UseItem {
    fn name(&self) -> &str {
        "use-item"
    }

    fn interests(&self) -> Interests {
        Interests::CHAT
    }

    fn start(&mut self, ctx: &mut TickContext<'_>) {
        ctx.select_slot(0);
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat {
            let t = m.plain();
            if t.contains("Grim") {
                self.alerts += 1;
                log::warn!("ALERT chat: {t}");
            }
        }
        if ctx.events().teleported && self.n > 40 {
            self.alerts += 1;
            log::warn!("ALERT setback");
        }
        if self.limit.is_some_and(|l| self.started.elapsed() >= l) {
            log::info!("time limit: {} alerts", self.alerts);
            ctx.disconnect("time limit reached");
            return;
        }
        if ctx.is_dead() {
            // DeathScreen: respawn after 20 ticks.
            self.dead_ticks += 1;
            if self.dead_ticks == 20 {
                ctx.respawn();
            }
            return;
        }
        self.dead_ticks = 0;
        // A riptide launch (26.1) shows as a fast climb.
        if ctx.pose().motion.y > 1.0 && !self.launched {
            log::info!("launched: motion y {:.3}", ctx.pose().motion.y);
        }
        self.launched = ctx.pose().motion.y > 1.0;
        self.n += 1;
        ctx.set_aim(ctx.pose().yaw, -89.0);
        // 60-tick cycle: 40 ticks right held (walking slowly in a circle), 20 ticks idle.
        let phase = self.n % 60;
        let right = (5..45).contains(&phase);
        ctx.set_buttons(Buttons { left: false, right });
        ctx.set_controls(Controls {
            forward: right,
            ..Controls::NONE
        });
        if self.n.is_multiple_of(100) {
            log::info!("tick {}: health {:?}", ctx.tick(), ctx.health());
        }
        let using = ctx.using_item();
        if using != self.was_using {
            log::info!(
                "tick {}: using_item {using} (held {:?}, health {:?})",
                ctx.tick(),
                ctx.inventory().held().map(|s| (s.id(), s.count())),
                ctx.health()
            );
            self.was_using = using;
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
    rt.block_on(async {
        let client = match opts.client() {
            Ok(c) => c,
            Err(e) => {
                log::error!("{e}");
                std::process::exit(2);
            }
        };
        let mut b = UseItem {
            n: 0,
            was_using: false,
            dead_ticks: 0,
            launched: false,
            alerts: 0,
            started: Instant::now(),
            limit: opts.limit(),
        };
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        match client.run_until(&mut b, shutdown).await {
            Ok(e) => log::info!("session ended: {e:?}; {} alerts", b.alerts),
            Err(e) => log::error!("session failed: {e}"),
        }
    });
}
