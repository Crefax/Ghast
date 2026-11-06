//! Walks planned paths on the 1.8 lab test pad (`lab/reset-field.sh 1.8`) with the
//! `ghast-ext` helpers and `ghast-pathfinder`: over the step-up course, around the wall and
//! the fence, through the water pool, over soul sand and ice, then to random points of the
//! pad until the time limit. Every path is checked in the vanilla-exact physics before the
//! bot moves.
//!
//! The bot must be opped (it starts with `/tp <name> 0.5 4 0.5`). Grim alerts that reach the
//! chat (ops see them) and server teleports (setbacks) are logged as `ALERT`.
//!
//! ```text
//! cargo run -p ghast --release --features ext,pathfinder --example goto -- \
//!     --host 127.0.0.1 --port 25618 --username GhastBot11 --version 1.8.9 --minutes 5
//! ```

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use ghast::Interests;
use ghast::ext::pathfinder::Goal;
use ghast::ext::{Error, Event, Ext, WhenDone};

/// The fixed part of the course: (name, x, z).
const COURSE: [(&str, i32, i32); 8] = [
    ("step course start", 44, 4),
    ("over the step course", 44, 20),
    ("water pool", 36, 16),
    ("soul sand", 20, 16),
    ("slime", 28, 6),
    ("packed ice", 12, 16),
    ("ice", 4, 20),
    ("spawn", 0, 0),
];

/// xorshift64*.
fn next(s: &mut u64) -> u64 {
    *s ^= *s >> 12;
    *s ^= *s << 25;
    *s ^= *s >> 27;
    s.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

fn main() {
    let opts = support::Options::parse();
    support::init_logger(opts.verbose);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let alerts = Arc::new(AtomicU32::new(0));
    let name = opts.username.clone();
    let limit = opts.limit();
    let mut seed = opts.seed.max(1);
    let a = alerts.clone();
    let mut ext = Ext::new(move |bot| {
        let name = name.clone();
        let alerts = a.clone();
        async move {
            let started = Instant::now();
            // Start on the spawn pad.
            let teleported = {
                let mut ev = bot.events();
                bot.chat(format!("/tp {name} 0.5 4 0.5"));
                bot.timeout(100, async {
                    while let Some(e) = ev.recv().await {
                        if let Event::Teleport(t) = e
                            && (t.after.pos.x - 0.5).abs() < 1e-6
                            && (t.after.pos.z - 0.5).abs() < 1e-6
                        {
                            return;
                        }
                    }
                })
                .await
            };
            if teleported.is_err() {
                log::warn!("no teleport after /tp: is {name} opped?");
            }
            bot.wait_ticks(20).await;
            // Watch for alerts and setbacks from now on.
            {
                let b = bot.clone();
                let alerts = alerts.clone();
                let _watch = bot.spawn(async move {
                    let mut ev = b.events();
                    while let Some(e) = ev.recv().await {
                        match e {
                            Event::Chat(m) => {
                                let t = m.plain();
                                if t.contains("Grim") || t.contains("failed") {
                                    alerts.fetch_add(1, Ordering::Relaxed);
                                    log::warn!("ALERT chat: {t}");
                                }
                            }
                            Event::Teleport(t) => {
                                alerts.fetch_add(1, Ordering::Relaxed);
                                let p = t.after.pos;
                                log::warn!("ALERT setback to {:.3} {:.3} {:.3}", p.x, p.y, p.z);
                            }
                            _ => {}
                        }
                    }
                });
            }
            let mut legs = 0u32;
            let mut go = async |label: String, x: i32, z: i32| -> Result<(), Error> {
                let t = Instant::now();
                let r = bot.goto(Goal::Xz { x, z }).await;
                let p = bot.pose().pos;
                match &r {
                    Ok(()) => log::info!(
                        "{label}: arrived at {:.2} {:.2} {:.2} in {:.1} s",
                        p.x,
                        p.y,
                        p.z,
                        t.elapsed().as_secs_f64()
                    ),
                    Err(e) => log::warn!("{label}: {e} at {:.2} {:.2} {:.2}", p.x, p.y, p.z),
                }
                legs += 1;
                r
            };
            for (label, x, z) in COURSE {
                go(label.to_owned(), x, z).await?;
            }
            while limit.is_some_and(|l| started.elapsed() < l) {
                let x = (next(&mut seed) % 62) as i32 - 15;
                let z = (next(&mut seed) % 62) as i32 - 15;
                go(format!("random ({x}, {z})"), x, z).await?;
            }
            log::info!(
                "{legs} legs in {:.0} s, {} alerts",
                started.elapsed().as_secs_f64(),
                alerts.load(Ordering::Relaxed)
            );
            Ok(())
        }
    })
    .name("goto")
    .interests(Interests::CHAT)
    // The follower replans after a setback by itself.
    .interrupt_on_teleport(false)
    .when_done(WhenDone::Disconnect);
    rt.block_on(async {
        let client = match opts.client() {
            Ok(c) => c,
            Err(e) => {
                log::error!("{e}");
                std::process::exit(2);
            }
        };
        let shutdown = async {
            #[cfg(unix)]
            {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                        .expect("SIGTERM handler");
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
        };
        match client.run_until(&mut ext, shutdown).await {
            Ok(e) => log::info!(
                "session ended: {e:?}; result {:?}; {} alerts",
                ext.result(),
                alerts.load(Ordering::Relaxed)
            ),
            Err(e) => log::error!("session failed: {e}"),
        }
    });
}
