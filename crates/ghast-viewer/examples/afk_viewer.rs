//! An AFK bot with its bot screen: connects, stands there (respawning after a death) and serves
//! the viewer on 127.0.0.1.
//!
//! ```text
//! cargo run --release --example afk_viewer -- <host:port> <name> [viewer port] [minutes]
//!     [--record <dir>] [--token <token>]
//! ```
//!
//! Then open `http://127.0.0.1:<viewer port>/` (third person: `?mode=tp`). Recordings made with
//! `--record` are listed on the page.

use std::time::{Duration, Instant};

use ghast_client::{Account, Behavior, Client, GameVersion, TickContext};
use ghast_viewer::{FeedRecorder, Viewer, ViewerConfig};

struct Afk {
    started: Instant,
    limit: Option<Duration>,
    dead_ticks: u32,
}

impl Behavior for Afk {
    fn name(&self) -> &str {
        "afk"
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        if ctx.is_dead() {
            self.dead_ticks += 1;
            if self.dead_ticks == 20 {
                ctx.respawn();
            }
        } else {
            self.dead_ticks = 0;
        }
        if self.limit.is_some_and(|l| self.started.elapsed() >= l) {
            ctx.disconnect("time limit reached");
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |args: &mut Vec<String>, name: &str| -> Option<String> {
        let i = args.iter().position(|a| a == name)?;
        args.remove(i);
        (i < args.len()).then(|| args.remove(i))
    };
    let record = flag(&mut args, "--record");
    let token = flag(&mut args, "--token");
    let addr = args
        .first()
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:25565".into());
    let name = args.get(1).cloned().unwrap_or_else(|| "ViewerBot".into());
    let port: u16 = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(8090);
    let minutes: Option<u64> = args.get(3).and_then(|m| m.parse().ok());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let mut cfg = ViewerConfig::new(&name).port(port);
        if let Some(t) = token {
            cfg = cfg.token(t);
        }
        if let Some(d) = &record {
            cfg = cfg.replay_dir(d);
        }
        // never stop on idle in this example
        let viewer = Viewer::new(cfg.idle(Duration::from_secs(u64::MAX / 4)));
        let at = viewer.start().await?;
        log::info!("bot screen: http://{at}/");
        let mut rec = record.as_ref().map(|d| FeedRecorder::start(&viewer, d));
        let client = Client::new(addr.parse()?, GameVersion::V1_8_9, Account::offline(&name)?)
            .world_config(ghast_viewer::world_config());
        let mut bot = viewer.watch(Afk {
            started: Instant::now(),
            limit: minutes.map(|m| Duration::from_secs(m * 60)),
            dead_ticks: 0,
        });
        let end = client.run(&mut bot).await;
        log::info!("session ended: {end:?}");
        if let Some(r) = rec.as_mut() {
            let m = r.stop();
            log::info!(
                "recording {} ({} frames, {} KB)",
                m.name,
                m.frames,
                m.bytes >> 10
            );
        }
        viewer.stop().await;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
