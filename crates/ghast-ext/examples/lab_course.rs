//! Drives the pathfinder through the parkour corridor and the maze that
//! `lab/pathfinder-course.sh` builds on the local 1.8, 1.12 or 26.1 lab, then follows an armor stand through
//! the maze with `Bot::follow_path`. Server teleports (setbacks) are counted; Grim's flags are
//! in the server console (`lab/alerts.sh 1.8 --all`).
//!
//! ```text
//! lab/pathfinder-course.sh 1.8 build          # or 1.12
//! timeout -s TERM 600 cargo run -p ghast-ext --release --example lab_course -- \
//!     127.0.0.1 25618 GhastBot12 480 2 1.8.9      # or: 127.0.0.1 25612 ... 2 1.12.2,
//!                                                  #     127.0.0.1 25626 ... 2 26.1
//! ```
//! Teleport the bot to the corridor start with `lab/pathfinder-course.sh 1.8 tp GhastBot12`
//! (it need not be an op) and move the armor stand with `... follow` once it prints `FOLLOW`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use ghast_client::connect::{Connector, ProxyPolicy, Route, TargetAddr};
use ghast_client::{Account, Client, GameVersion, Interests};
use ghast_ext::pathfinder::Goal;
use ghast_ext::{Error, Event, Ext, WhenDone};

/// Prints `ghast_pathfinder` / `ghast_ext` debug lines when `GHAST_DEBUG` is set.
struct Logger;

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= log::Level::Info
            || m.target().starts_with("ghast_pathfinder")
            || m.target().starts_with("ghast_ext")
    }

    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            println!("{} {}: {}", r.level(), r.target(), r.args());
        }
    }

    fn flush(&self) {}
}

fn main() {
    if std::env::var_os("GHAST_DEBUG").is_some() {
        let _ = log::set_logger(&Logger);
        log::set_max_level(log::LevelFilter::Debug);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let host = args.first().cloned().unwrap_or_else(|| "127.0.0.1".into());
    let port: u16 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(25618);
    let name = args.get(2).cloned().unwrap_or_else(|| "GhastBot12".into());
    let secs: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(480);
    let rounds: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(2);
    let version = match args.get(5).map(String::as_str) {
        None | Some("1.8.9") => GameVersion::V1_8_9,
        Some("1.12.2") => GameVersion::V1_12_2,
        Some("1.16.5") => GameVersion::V1_16_5,
        Some("26.1") => GameVersion::V26_1,
        Some(v) => panic!("version {v}: 1.8.9, 1.12.2, 1.16.5 or 26.1 (the lab course scripts)"),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let setbacks = Arc::new(AtomicU32::new(0));
    let failures = Arc::new(AtomicU32::new(0));
    let (sb, fl) = (setbacks.clone(), failures.clone());
    let mut ext = Ext::new(move |bot| {
        let (setbacks, failures) = (sb.clone(), fl.clone());
        async move {
            let started = Instant::now();
            println!("JOINED");
            // Wait for the script's teleport to the corridor start.
            let mut ev = bot.events();
            let _ = bot
                .timeout(20 * 60, async {
                    while let Some(e) = ev.recv().await {
                        if let Event::Teleport(t) = e
                            && (t.after.pos.x - 62.5).abs() < 0.01
                        {
                            return;
                        }
                    }
                })
                .await;
            bot.wait_ticks(20).await;
            println!("START at {:?}", bot.pose().pos);
            {
                let b = bot.clone();
                let setbacks = setbacks.clone();
                let _w = bot.spawn(async move {
                    let mut ev = b.events();
                    while let Some(e) = ev.recv().await {
                        if let Event::Teleport(t) = e {
                            setbacks.fetch_add(1, Ordering::Relaxed);
                            println!("SETBACK to {:?}", t.after.pos);
                        }
                    }
                });
            }
            let leg = async |label: &str, goal: Goal| {
                let t = Instant::now();
                let r = bot.goto(goal).await;
                let p = bot.pose().pos;
                println!(
                    "LEG {label}: {} at {:.2} {:.2} {:.2} in {:.1} s",
                    match &r {
                        Ok(()) => "ok".to_owned(),
                        Err(e) => format!("FAILED ({e})"),
                    },
                    p.x,
                    p.y,
                    p.z,
                    t.elapsed().as_secs_f64()
                );
                if r.is_err() {
                    failures.fetch_add(1, Ordering::Relaxed);
                }
            };
            for round in 0..rounds {
                leg(
                    &format!("corridor east #{round}"),
                    Goal::Xz { x: 119, z: 0 },
                )
                .await;
                leg(&format!("corridor west #{round}"), Goal::Xz { x: 62, z: 0 }).await;
            }
            leg(
                "maze centre",
                Goal::Near {
                    pos: [35, 4, 75],
                    radius: 1.0,
                },
            )
            .await;
            leg("out of the maze", Goal::Block([17, 4, 61])).await;
            leg(
                "maze centre again",
                Goal::Near {
                    pos: [35, 4, 75],
                    radius: 1.0,
                },
            )
            .await;
            // Follow the armor stand the script moves around the maze.
            let stand = bot
                .nearest_entity(
                    |e| e.name.eq_ignore_ascii_case("ArmorStand") || e.name == "armor_stand",
                    64.0,
                )
                .await?;
            match stand {
                Some(s) => {
                    println!("FOLLOW {}", s.id);
                    // Every 2 s: how far the bot is from the stand.
                    let b = bot.clone();
                    let id = s.id;
                    let _m = bot.spawn(async move {
                        loop {
                            b.wait_ticks(40).await;
                            if let (Ok(Some(e)), p) = (b.entity(id).await, b.pose().pos) {
                                let d = ((p.x - e.pos.x).powi(2)
                                    + (p.y - e.pos.y).powi(2)
                                    + (p.z - e.pos.z).powi(2))
                                .sqrt();
                                println!(
                                    "FOLLOWING stand {:.1} {:.1} {:.1}, bot {:.1} {:.1} {:.1}, distance {d:.1}",
                                    e.pos.x, e.pos.y, e.pos.z, p.x, p.y, p.z
                                );
                            }
                        }
                    });
                    let left = Duration::from_secs(secs).saturating_sub(started.elapsed());
                    let ticks = (left.as_secs().min(150) * 20).max(20 * 30);
                    let r = bot.timeout(ticks, bot.follow_path(s.id, 2.0)).await;
                    println!("FOLLOW ended: {r:?} at {:?}", bot.pose().pos);
                }
                None => println!("no armor stand found"),
            }
            println!(
                "DONE in {:.0} s: {} failed legs, {} setbacks",
                started.elapsed().as_secs_f64(),
                failures.load(Ordering::Relaxed),
                setbacks.load(Ordering::Relaxed)
            );
            Ok::<(), Error>(())
        }
    })
    .name("lab_course")
    .interests(Interests::CHAT | Interests::ENTITIES)
    .interrupt_on_teleport(false)
    .when_done(WhenDone::Disconnect);
    rt.block_on(async {
        let client = Client::new(
            TargetAddr::new(&host, port).expect("address"),
            version,
            Account::offline(&name).expect("name"),
        )
        .connector(Connector::new().with_policy(ProxyPolicy::custom(|_| Route::Direct)));
        let limit = tokio::time::sleep(Duration::from_secs(secs));
        match client.run_until(&mut ext, limit).await {
            Ok(e) => println!("session ended: {e:?}; result {:?}", ext.result()),
            Err(e) => println!("session failed: {e}"),
        }
        println!(
            "SUMMARY failed legs {}, setbacks {}",
            failures.load(Ordering::Relaxed),
            setbacks.load(Ordering::Relaxed)
        );
    });
}
