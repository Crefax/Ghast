//! Mixed soak test for the lab: walking / sprinting / jumping on open grass, digging and placing
//! a block, and inventory clicks, in a loop. Grim alerts that reach the chat and unexpected
//! setbacks are logged as `ALERT`.
//!
//! The bot must be opped (each round starts with `/tp`); give it a stack of dirt in hotbar slot
//! 0 first, e.g. `lab/console.sh 1.8 "give GhastBot2 dirt 64"` while it is online.
//!
//! ```text
//! cargo run -p ghast --release --example soak -- --host 127.0.0.1 --port 25618 \
//!     --username GhastBot2 --version 1.8.9 --minutes 30 --record lab/results/soak18.jsonl
//! ```

mod support;

use std::time::{Duration, Instant};

use ghast::inventory::Click;
use ghast::{Behavior, BlockPos, Buttons, Controls, Face, Interests, TickContext, Vec3};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Teleport(u32),
    Walk {
        target: (f64, f64),
        mode: u8,
        ticks: u32,
    },
    Dig(u32),
    Place(u32),
    Inventory(u32),
}

struct Soak {
    phase: Phase,
    round: u32,
    walks_left: u32,
    digs_left: u32,
    rng: u64,
    broken: u64,
    alerts: u32,
    expect_tp: u32,
    hold: u32,
    started: Instant,
    limit: Option<Duration>,
    stats: [u32; 4],
}

const BLOCK: BlockPos = BlockPos::new(1, 3, 0);

fn look_at(eyes: Vec3, p: Vec3) -> (f32, f32) {
    let (dx, dy, dz) = (p.x - eyes.x, p.y - eyes.y, p.z - eyes.z);
    let yaw = (-dx).atan2(dz).to_degrees();
    let pitch = -(dy.atan2((dx * dx + dz * dz).sqrt())).to_degrees();
    (yaw as f32, pitch as f32)
}

impl Soak {
    fn rand(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    fn next_walk(&mut self) -> Phase {
        // Open grass west of the spawn: no strips, no course.
        let target = (-14.0 + self.rand() * 12.0, -12.0 + self.rand() * 24.0);
        let mode = (self.rand() * 4.0) as u8;
        Phase::Walk {
            target,
            mode,
            ticks: 0,
        }
    }

    fn aim_smooth(ctx: &mut TickContext<'_>, yaw: f32, pitch: f32) {
        let cur = ctx.pose().yaw;
        let yaw = cur + ghast::rotation::wrap_degrees(yaw - cur);
        ctx.set_aim(yaw, pitch);
    }
}

impl Behavior for Soak {
    fn name(&self) -> &str {
        "soak"
    }

    fn interests(&self) -> Interests {
        Interests::CHAT
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat {
            let t = m.plain();
            if t.contains("Grim") {
                self.alerts += 1;
                log::warn!("ALERT chat: {t}");
            }
        }
        if ctx.events().teleported {
            if self.expect_tp > 0 {
                self.expect_tp = 0;
            } else {
                self.alerts += 1;
                self.hold = 10;
                log::warn!("ALERT setback in {:?}", self.phase);
            }
        }
        self.expect_tp = self.expect_tp.saturating_sub(1);
        for c in &ctx.events().inventory.clicks {
            if c.outcome != ghast::inventory::ClickOutcome::Accepted {
                self.alerts += 1;
                log::warn!("ALERT click {c:?}");
            }
        }
        if self.limit.is_some_and(|l| self.started.elapsed() >= l) {
            log::info!(
                "time limit: {} rounds, {} alerts, walks/digs/places/clicks {:?}",
                self.round,
                self.alerts,
                self.stats
            );
            ctx.disconnect("time limit reached");
            return;
        }
        if ctx.is_dead() {
            ctx.respawn();
            return;
        }
        ctx.set_controls(Controls::NONE);
        ctx.set_buttons(Buttons::default());
        if self.hold > 0 {
            self.hold -= 1;
            return;
        }
        let eyes = ctx.eyes(ctx.pose());
        self.phase = match self.phase {
            Phase::Teleport(0) => {
                ctx.chat("/tp 0.5 4 0.5");
                self.expect_tp = 40;
                self.round += 1;
                Phase::Teleport(1)
            }
            Phase::Teleport(n) if n < 30 => Phase::Teleport(n + 1),
            Phase::Teleport(_) => {
                ctx.select_slot(0);
                self.walks_left = 6;
                self.digs_left = 3;
                self.next_walk()
            }
            Phase::Walk {
                target,
                mode,
                ticks,
            } => {
                let p = ctx.pose().pos;
                let (dx, dz) = (target.0 - p.x, target.1 - p.z);
                let dist = (dx * dx + dz * dz).sqrt();
                let (yaw, _) = look_at(eyes, Vec3::new(target.0, eyes.y, target.1));
                Self::aim_smooth(ctx, yaw, 5.0);
                let err = ghast::rotation::wrap_degrees(yaw - ctx.pose().yaw).abs();
                if dist < 0.7 || ticks > 400 {
                    self.stats[0] += 1;
                    self.walks_left = self.walks_left.saturating_sub(1);
                    if self.walks_left == 0 {
                        Phase::Dig(0)
                    } else {
                        self.next_walk()
                    }
                } else {
                    // 0 walk, 1 sprint, 2 sprint-jump, 3 sneak.
                    ctx.set_controls(Controls {
                        forward: err < 60.0,
                        sprint: mode == 1 || mode == 2,
                        jump: mode == 2,
                        sneak: mode == 3,
                        ..Controls::NONE
                    });
                    Phase::Walk {
                        target,
                        mode,
                        ticks: ticks + 1,
                    }
                }
            }
            Phase::Dig(n) => {
                // Walk back next to the block first.
                let p = ctx.pose().pos;
                let (dx, dz) = (0.5 - p.x, 0.5 - p.z);
                if (dx * dx + dz * dz).sqrt() > 0.4 && n < 400 {
                    let (yaw, _) = look_at(eyes, Vec3::new(0.5, eyes.y, 0.5));
                    Self::aim_smooth(ctx, yaw, 5.0);
                    ctx.set_controls(Controls {
                        forward: ghast::rotation::wrap_degrees(yaw - ctx.pose().yaw).abs() < 30.0,
                        ..Controls::NONE
                    });
                    Phase::Dig(n + 1)
                } else {
                    let top = Vec3::new(1.5, 4.0, 0.5);
                    let (yaw, pitch) = look_at(eyes, top);
                    Self::aim_smooth(ctx, yaw, pitch);
                    let on_it = ctx.target().is_some_and(|t| t.pos == BLOCK);
                    ctx.set_buttons(Buttons {
                        left: on_it,
                        right: false,
                    });
                    if ctx.broken() > self.broken {
                        self.broken = ctx.broken();
                        self.stats[1] += 1;
                        ctx.set_buttons(Buttons::default());
                        Phase::Place(0)
                    } else if n > 900 {
                        log::warn!("dig gave up");
                        Phase::Inventory(0)
                    } else {
                        Phase::Dig(n + 1)
                    }
                }
            }
            Phase::Place(n) => {
                let support = Face::Down.offset(BLOCK);
                let hit = Vec3::new(1.5, 3.0, 0.5);
                let (yaw, pitch) = look_at(eyes, hit);
                Self::aim_smooth(ctx, yaw, pitch);
                if n > 2 && ctx.place(support, Face::Up, hit) {
                    self.stats[2] += 1;
                    self.digs_left = self.digs_left.saturating_sub(1);
                    if self.digs_left == 0 {
                        Phase::Inventory(0)
                    } else {
                        Phase::Dig(0)
                    }
                } else if n > 200 {
                    log::warn!("place gave up");
                    Phase::Inventory(0)
                } else {
                    Phase::Place(n + 1)
                }
            }
            Phase::Inventory(n) => {
                // Still: open the inventory, move the hotbar-0 stack to slot 9 and back, close.
                let step = n / 10;
                let mut next = Phase::Inventory(n + 1);
                if n % 10 == 0 {
                    let r = match step {
                        0 => ctx.open_inventory(),
                        1 => ctx
                            .click(Click::Pickup {
                                slot: 36,
                                right: false,
                            })
                            .map(drop),
                        2 => ctx
                            .click(Click::Pickup {
                                slot: 9,
                                right: false,
                            })
                            .map(drop),
                        3 => ctx
                            .click(Click::Pickup {
                                slot: 9,
                                right: false,
                            })
                            .map(drop),
                        4 => ctx
                            .click(Click::Pickup {
                                slot: 36,
                                right: false,
                            })
                            .map(drop),
                        5 => ctx.close_window(),
                        _ => {
                            next = Phase::Teleport(0);
                            Ok(())
                        }
                    };
                    match r {
                        Ok(()) if (1..=4).contains(&step) => self.stats[3] += 1,
                        Ok(()) => {}
                        Err(e) => {
                            log::debug!("inventory step {step} refused ({e}); retry");
                            next = Phase::Inventory(n);
                        }
                    }
                }
                next
            }
        };
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
        let mut b = Soak {
            phase: Phase::Teleport(0),
            round: 0,
            walks_left: 0,
            digs_left: 0,
            rng: opts.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
            broken: 0,
            alerts: 0,
            expect_tp: 40,
            hold: 0,
            started: Instant::now(),
            limit: opts.limit(),
            stats: [0; 4],
        };
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        match client.run_until(&mut b, shutdown).await {
            Ok(e) => log::info!("session ended: {e:?}; {} alerts, {:?}", b.alerts, b.stats),
            Err(e) => log::error!("session failed: {e}"),
        }
    });
}
