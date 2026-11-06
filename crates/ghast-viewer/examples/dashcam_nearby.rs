//! The integration guide's example (INTEGRATION.md): a bot whose screen is served with a token,
//! that keeps a dash cam and saves a clip when another player comes near.
//!
//! ```text
//! cargo run --release --example dashcam_nearby -- <host:port> <name> <viewer port> <token> <replay dir>
//! ```

use std::sync::Arc;
use std::time::Duration;

use ghast_client::{Account, Behavior, Client, GameVersion, Interests, TickContext};
use ghast_viewer::{DashCam, DashCamOptions, Viewer, ViewerConfig, prune_recordings};
use tokio::sync::mpsc;

/// The app's behaviour. It only reports what it sees; the clip itself is saved off the tick
/// thread (it writes the buffered minutes to disk).
struct App {
    me: String,
    /// Other bots of this app: never a reason for a clip.
    own: Vec<String>,
    clips: mpsc::UnboundedSender<String>,
    /// Ticks until the next clip may be asked for.
    quiet: u32,
}

impl Behavior for App {
    fn name(&self) -> &str {
        "farm"
    }

    fn interests(&self) -> Interests {
        // the guard needs entities and their names, whatever the viewer tracks
        Interests::ENTITIES | Interests::PLAYER_LIST
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        self.quiet = self.quiet.saturating_sub(1);
        let Some(ents) = ctx.entities() else { return };
        let near: Vec<&str> = ents
            .iter()
            .filter(|e| e.is_player() && e.uuid().get_version_num() != 2) // version 2: server NPCs
            .filter_map(|e| ents.player_name(e.id()))
            .filter(|n| !n.eq_ignore_ascii_case(&self.me))
            .filter(|n| !self.own.iter().any(|o| o.eq_ignore_ascii_case(n)))
            .collect();
        if !near.is_empty() && self.quiet == 0 {
            let _ = self.clips.send(format!("player near: {}", near.join(", ")));
            self.quiet = 20 * 60; // at most one request a minute (a running clip is extended anyway)
        }
        // … the farm itself
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let a: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, d: &str| a.get(i).cloned().unwrap_or_else(|| d.to_string());
    let (addr, name) = (arg(0, "127.0.0.1:25565"), arg(1, "ViewerBot"));
    let (port, token, dir) = (
        arg(2, "8090").parse::<u16>()?,
        arg(3, "change-me"),
        arg(4, "replays"),
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let viewer = Viewer::new(
            ViewerConfig::new(&name)
                .port(port)
                .token(token)
                .replay_dir(&dir)
                .cache_dir("/var/cache/ghast-viewer"),
        );
        // the dash cam buffers only while the bot is in a world
        let v = viewer.clone();
        let cam = DashCam::start(
            &viewer,
            &dir,
            DashCamOptions {
                active: Box::new(move || v.status().in_game),
                max_bytes: 256 << 20,
                max_age: Duration::from_secs(7 * 86_400),
                ..DashCamOptions::default()
            },
        );
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let saver = Arc::clone(&cam);
        tokio::spawn(async move {
            while let Some(note) = rx.recv().await {
                let cam = Arc::clone(&saver);
                let name = tokio::task::spawn_blocking(move || cam.clip(&note)).await;
                if let Ok(Some(name)) = name {
                    log::info!("clip {name}: open /?replay={name}");
                }
            }
        });

        let client = Client::new(addr.parse()?, GameVersion::V1_8_9, Account::offline(&name)?)
            .world_config(ghast_viewer::world_config());
        let mut bot = viewer.watch(App {
            me: name.clone(),
            own: Vec::new(),
            clips: tx,
            quiet: 0,
        });
        // the page is served on demand; it stops itself after `idle` without a page
        viewer.start().await?;
        let end = client.run(&mut bot).await;
        log::info!("session ended: {end:?}");
        cam.stop();
        prune_recordings(
            dir.as_ref(),
            256 << 20,
            Duration::from_secs(7 * 86_400),
            None,
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
