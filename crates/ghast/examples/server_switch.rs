//! Server switch bot (1.20.2+ reconfiguration): walks, then asks a proxy to move it to the
//! next backend, again and again. Every switch runs play → configuration → play: the proxy
//! sends `start_configuration`, the new backend's registries and a new join game.
//!
//! ```text
//! cargo run -p ghast --release --example server_switch -- --host 127.0.0.1 --port 25629 \
//!     --username GhastBot --version 26.1 --switches 10 --servers a,b [--command server]
//! ```
//!
//! Prints one `switch` line per join (backend, resident memory, lowest loaded block) and a
//! final `summary` line. Exit status 0 when every switch completed.

mod support;

use std::time::{Duration, Instant};

use ghast::{Behavior, Controls, Interests, SessionEnd, TickContext};

/// Ticks to stand still after a join (level loading), then to walk, then to wait.
const SETTLE: u64 = 60;
const WALK: u64 = 160;
const PAUSE: u64 = 20;
/// A switch must arrive within this many ticks of the command.
const SWITCH_TIMEOUT: u64 = 400;

struct Switch {
    servers: Vec<String>,
    command: String,
    switches: usize,
    /// Joins seen (the first is the initial login).
    joins: usize,
    /// Tick of the last join.
    joined_at: u64,
    /// Tick the last switch command was sent.
    asked_at: Option<u64>,
    rss: Vec<u64>,
    started: Instant,
    limit: Option<Duration>,
    failed: Option<String>,
}

/// Resident set size in KiB (Linux; 0 elsewhere).
fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

impl Switch {
    fn current(&self) -> &str {
        // The first join is on servers[0]; each switch moves to the next one.
        &self.servers[(self.joins.saturating_sub(1)) % self.servers.len()]
    }

    fn on_join(&mut self, ctx: &mut TickContext<'_>) {
        self.joins += 1;
        self.joined_at = ctx.tick();
        self.asked_at = None;
        let rss = rss_kib();
        self.rss.push(rss);
        let p = ctx.pose().pos;
        // The lowest block below the player that the world holds (the dimension's floor).
        let (bx, bz) = (p.x.floor() as i32, p.z.floor() as i32);
        let floor = (-512..0).find(|&y| ctx.world().raw_block(bx, y, bz).is_some_and(|b| b != 0));
        log::info!(
            "switch {}: on {} ({}), at {:.2} {:.2} {:.2}, rss {rss} KiB, lowest block y {}",
            self.joins - 1,
            self.current(),
            ctx.dimension(),
            p.x,
            p.y,
            p.z,
            floor.map_or_else(|| "none".to_owned(), |y| y.to_string()),
        );
    }
}

impl Behavior for Switch {
    fn name(&self) -> &str {
        "server_switch"
    }

    fn interests(&self) -> Interests {
        Interests::CHAT
    }

    fn start(&mut self, ctx: &mut TickContext<'_>) {
        self.on_join(ctx);
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat {
            log::info!("chat: {}", m.plain());
        }
        if ctx.events().joined && ctx.tick() != self.joined_at {
            self.on_join(ctx);
        }
        if ctx.is_dead() {
            ctx.respawn();
        }
        let since = ctx.tick() - self.joined_at;
        if let Some(at) = self.asked_at {
            ctx.set_controls(Controls::NONE);
            if ctx.tick() - at > SWITCH_TIMEOUT {
                self.failed = Some(format!(
                    "no switch {} ticks after the command",
                    SWITCH_TIMEOUT
                ));
                ctx.disconnect("switch timed out");
            }
            return;
        }
        if since < SETTLE {
            ctx.set_controls(Controls::NONE);
        } else if since < SETTLE + WALK {
            // Forward and back along a fixed line (sprinting half of the time).
            let t = since - SETTLE;
            let forward = (t / 40).is_multiple_of(2);
            ctx.set_aim(0.0, 0.0);
            ctx.set_controls(Controls {
                forward,
                back: !forward,
                sprint: forward && (t / 80).is_multiple_of(2),
                ..Controls::NONE
            });
        } else if since < SETTLE + WALK + PAUSE {
            ctx.set_controls(Controls::NONE);
        } else if self.joins > self.switches {
            ctx.disconnect("all switches done");
        } else {
            let next = &self.servers[self.joins % self.servers.len()];
            log::info!("switch {}: asking for {next}", self.joins);
            ctx.chat(format!("/{} {next}", self.command));
            self.asked_at = Some(ctx.tick());
        }
        if self.limit.is_some_and(|l| self.started.elapsed() >= l) {
            self.failed = Some("time limit reached".into());
            ctx.disconnect("time limit reached");
        }
    }
}

fn main() {
    // Own options first; the rest are the shared ones.
    let mut switches = 10;
    let mut servers = vec!["a".to_owned(), "b".to_owned()];
    let mut command = "server".to_owned();
    let mut rest = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--switches" => {
                switches = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--switches <n>");
            }
            "--servers" => {
                servers = args
                    .next()
                    .expect("--servers a,b")
                    .split(',')
                    .map(str::to_owned)
                    .collect();
            }
            "--command" => command = args.next().expect("--command <name>"),
            _ => rest.push(a),
        }
    }
    let opts = support::Options::parse_from(rest);
    support::init_logger(opts.verbose);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let ok = rt.block_on(async {
        let client = match opts.client() {
            Ok(c) => c,
            Err(e) => {
                log::error!("{e}");
                std::process::exit(2);
            }
        };
        let mut bot = Switch {
            servers,
            command,
            switches,
            joins: 0,
            joined_at: 0,
            asked_at: None,
            rss: Vec::new(),
            started: Instant::now(),
            limit: opts.limit(),
            failed: None,
        };
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        let end = client.run_until(&mut bot, shutdown).await;
        match &end {
            Ok(e) => log::info!("session ended: {e:?}"),
            Err(e) => log::error!("session failed: {e}"),
        }
        let done = bot.joins.saturating_sub(1);
        log::info!(
            "summary: {done}/{} switches, rss KiB per join {:?}{}",
            bot.switches,
            bot.rss,
            bot.failed
                .as_ref()
                .map_or(String::new(), |f| format!(", failed: {f}"))
        );
        matches!(end, Ok(SessionEnd::Quit(_))) && bot.failed.is_none() && done >= bot.switches
    });
    std::process::exit(if ok { 0 } else { 1 });
}
