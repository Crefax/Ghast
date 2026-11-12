//! Live entity checks for the lab (26.1), chosen with `GHAST_PROBE`:
//!
//! * `walk` (default): walks straight along +z for `--minutes` (through whatever stands there,
//!   e.g. a crowd of NoAI mobs and a boat; `--entities` turns on entity tracking, which the 26.1
//!   client needs to collide with and be pushed by them). Prints the end position.
//! * `horse`: finds the nearest horse, looks at it, sneak-right-clicks it (the tamed horse's
//!   inventory), moves its saddle into the player inventory and back. `PASS` when the screen
//!   opened and every click was confirmed as predicted.
//! * `villager`: finds the nearest villager, right-clicks it (the trade screen), selects trade 0
//!   and shift-takes the result. `PASS` when the screen opened, the trade was selected and the
//!   take was confirmed as predicted.
//!
//! ```text
//! GHAST_PROBE=villager cargo run -p ghast --release --example entity_probe -- \
//!     --host 127.0.0.1 --port 25626 --username GhastBot11 --version 26.1 --minutes 1
//! ```
//! `lab/entities.sh` sets the scene and runs every mode.

mod support;

use std::time::{Duration, Instant};

use ghast::inventory::{Click, ClickError, ClickOutcome};
use ghast::{Behavior, Buttons, ClickRefused, Controls, Interests, TickContext, Vec3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Walk,
    Horse,
    Villager,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Settle,
    Aim,
    Use,
    WaitWindow,
    Trade,
    Clicks,
    Close,
    Done,
}

struct Probe {
    mode: Mode,
    entities: bool,
    step: Step,
    wait: u32,
    target: Option<i32>,
    opened: bool,
    awaiting: bool,
    clicks: Vec<Click>,
    sent: u32,
    accepted: u32,
    failures: Vec<String>,
    started: Instant,
    limit: Duration,
    reported: bool,
    /// Walk direction (`GHAST_YAW`, default 0 = +z).
    yaw: f32,
}

impl Probe {
    fn finish(&mut self, ctx: &mut TickContext<'_>) {
        if self.reported {
            return;
        }
        self.reported = true;
        let p = ctx.pose().pos;
        println!("end position {:.4} {:.4} {:.4}", p.x, p.y, p.z);
        if self.mode != Mode::Walk {
            println!(
                "screen opened {}, clicks sent {}, accepted {}, failures {}",
                self.opened,
                self.sent,
                self.accepted,
                self.failures.len()
            );
            for f in &self.failures {
                println!("  FAIL {f}");
            }
            if self.ok() {
                println!("PASS");
            }
        }
        ctx.disconnect("probe done");
    }

    fn ok(&self) -> bool {
        self.mode == Mode::Walk
            || (self.opened
                && self.failures.is_empty()
                && self.sent > 0
                && self.sent == self.accepted)
    }

    /// The nearest tracked entity of a type, with its box.
    fn find(ctx: &TickContext<'_>, name: &str) -> Option<(i32, [f64; 6])> {
        let ents = ctx.entities()?;
        let eyes = ctx.eyes(ctx.pose());
        ents.iter()
            .filter(|e| ghast::entity::v26_1::entity_type(e).is_some_and(|t| t.name == name))
            .filter_map(|e| Some((e.id(), e.bounding_box()?)))
            .min_by(|a, b| {
                let d = |b: &[f64; 6]| {
                    let c = [
                        (b[0] + b[3]) / 2.0,
                        (b[1] + b[4]) / 2.0,
                        (b[2] + b[5]) / 2.0,
                    ];
                    (c[0] - eyes.x).powi(2) + (c[1] - eyes.y).powi(2) + (c[2] - eyes.z).powi(2)
                };
                d(&a.1).total_cmp(&d(&b.1))
            })
    }
}

impl Behavior for Probe {
    fn name(&self) -> &str {
        "entity-probe"
    }

    fn interests(&self) -> Interests {
        if self.entities || self.mode != Mode::Walk {
            // The tab list gives other players' scoreboard names (their teams).
            Interests::ENTITIES | Interests::PLAYER_LIST | Interests::CHAT
        } else {
            Interests::CHAT
        }
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat {
            let line = m.plain();
            if line.contains("Grim") || line.contains("failed") {
                log::warn!("ALERT {line}");
            }
        }
        if ctx.is_dead() {
            // A bot left dead by an earlier run (the respawn screen): back to the spawn.
            ctx.respawn();
            return;
        }
        if self.started.elapsed() >= self.limit {
            if self.mode != Mode::Walk {
                self.failures.push(format!("timed out at {:?}", self.step));
            }
            self.finish(ctx);
            return;
        }
        let ev = ctx.events().inventory.clone();
        for c in &ev.clicks {
            self.awaiting = false;
            match c.outcome {
                ClickOutcome::Accepted => self.accepted += 1,
                o => self.failures.push(format!(
                    "action {} in window {}: {o:?}",
                    c.action, c.window_id
                )),
            }
        }
        if ev.opened.is_some() {
            self.opened = true;
        }
        if self.mode == Mode::Walk {
            if self.wait < 40 {
                self.wait += 1;
                return;
            }
            ctx.set_aim(self.yaw, 0.0);
            ctx.set_controls(Controls {
                forward: true,
                ..Controls::NONE
            });
            if ctx.tick().is_multiple_of(20) {
                let p = ctx.pose().pos;
                log::info!("at {:.3} {:.3} {:.3}", p.x, p.y, p.z);
            }
            return;
        }
        if self.wait > 0 {
            self.wait -= 1;
            return;
        }
        match self.step {
            Step::Settle => {
                self.wait = 40;
                self.step = Step::Aim;
            }
            Step::Aim => {
                let name = if self.mode == Mode::Horse {
                    "horse"
                } else {
                    "villager"
                };
                let Some((id, b)) = Self::find(ctx, name) else {
                    return;
                };
                self.target = Some(id);
                let eyes = ctx.eyes(ctx.pose());
                let c = Vec3::new(
                    (b[0] + b[3]) / 2.0,
                    (b[1] + b[4]) / 2.0,
                    (b[2] + b[5]) / 2.0,
                );
                let (dx, dy, dz) = (c.x - eyes.x, c.y - eyes.y, c.z - eyes.z);
                let yaw = (-dx).atan2(dz).to_degrees() as f32;
                let pitch = -(dy.atan2((dx * dx + dz * dz).sqrt())).to_degrees() as f32;
                let cur = ctx.pose().yaw;
                ctx.set_aim(cur + ghast::rotation::wrap_degrees(yaw - cur), pitch);
                // A tamed horse opens its inventory on a sneaking right click.
                ctx.set_controls(Controls {
                    sneak: self.mode == Mode::Horse,
                    ..Controls::NONE
                });
                if ctx.target_entity().is_some_and(|e| e.id == id) {
                    self.wait = 3;
                    self.step = Step::Use;
                }
            }
            Step::Use => {
                if ctx
                    .target_entity()
                    .is_none_or(|e| Some(e.id) != self.target)
                {
                    self.step = Step::Aim;
                    return;
                }
                ctx.set_buttons(Buttons {
                    left: false,
                    right: true,
                });
                self.step = Step::WaitWindow;
            }
            Step::WaitWindow => {
                ctx.set_buttons(Buttons::default());
                ctx.set_controls(Controls::NONE);
                if self.opened && !ctx.inventory().resyncing() {
                    let n = ctx.inventory().container_len();
                    println!(
                        "screen open: window {:?}, {n} container slots",
                        ctx.open_window()
                    );
                    self.wait = 10;
                    self.step = if self.mode == Mode::Villager {
                        Step::Trade
                    } else {
                        // Saddle (slot 0) into the inventory, then back (first free slot of the
                        // main inventory after the container: shift-click moves it there).
                        self.clicks = vec![Click::QuickMove {
                            slot: 0,
                            right: false,
                        }];
                        Step::Clicks
                    };
                }
            }
            Step::Trade => match ctx.select_trade(0) {
                Ok(()) => {
                    println!("trade 0 selected");
                    self.wait = 10;
                    self.clicks = vec![Click::QuickMove {
                        slot: 2,
                        right: false,
                    }];
                    self.step = Step::Clicks;
                }
                Err(ClickRefused::Turning) => {}
                Err(e) => {
                    self.failures.push(format!("select_trade refused: {e}"));
                    self.step = Step::Close;
                }
            },
            Step::Clicks => {
                if self.awaiting {
                    return;
                }
                if self.clicks.is_empty() {
                    if self.mode == Mode::Horse && self.sent == 1 {
                        // Back: find the saddle in the player part and shift-click it.
                        let inv = ctx.inventory();
                        let n = inv.container_len();
                        let id = ghast::inventory::v26_1::item_id("saddle");
                        let saddle = (n..inv.slot_count())
                            .find(|&i| inv.slot(i).is_some_and(|s| Some(s.id()) == id));
                        match saddle {
                            Some(i) => self.clicks.push(Click::QuickMove {
                                slot: i as i16,
                                right: false,
                            }),
                            None => self.failures.push("saddle not in the inventory".into()),
                        }
                        if !self.clicks.is_empty() {
                            return;
                        }
                    }
                    self.wait = 10;
                    self.step = Step::Close;
                    return;
                }
                let c = self.clicks[0];
                match ctx.click(c) {
                    Ok(_) => {
                        println!("  click {c:?}");
                        self.clicks.remove(0);
                        self.sent += 1;
                        self.awaiting = true;
                    }
                    Err(
                        ClickRefused::Turning
                        | ClickRefused::Inventory(
                            ClickError::Pending | ClickError::SameTick | ClickError::Resyncing,
                        ),
                    ) => {}
                    Err(e) => {
                        self.failures.push(format!("{c:?} refused: {e}"));
                        self.clicks.clear();
                    }
                }
            }
            Step::Close => {
                if ctx.close_window().is_ok() {
                    self.wait = 20;
                    self.step = Step::Done;
                }
            }
            Step::Done => self.finish(ctx),
        }
    }
}

fn main() {
    let opts = support::Options::parse();
    support::init_logger(opts.verbose);
    let mode = match std::env::var("GHAST_PROBE").as_deref() {
        Ok("horse") => Mode::Horse,
        Ok("villager") => Mode::Villager,
        Ok("walk") | Err(_) => Mode::Walk,
        Ok(other) => {
            eprintln!("GHAST_PROBE: walk | horse | villager, not {other}");
            std::process::exit(2);
        }
    };
    let limit = Duration::from_secs_f64(opts.minutes.unwrap_or(1.0) * 60.0);
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
        let mut probe = Probe {
            mode,
            entities: opts.entities,
            step: Step::Settle,
            wait: 0,
            target: None,
            opened: false,
            awaiting: false,
            clicks: Vec::new(),
            sent: 0,
            accepted: 0,
            failures: Vec::new(),
            started: Instant::now(),
            limit,
            reported: false,
            yaw: std::env::var("GHAST_YAW")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
        };
        let end = client.run(&mut probe).await;
        log::info!("session ended: {end:?}");
        std::process::exit(if probe.ok() { 0 } else { 1 });
    });
}
