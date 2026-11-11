//! 26.1 elytra lab bot: walks off a tower, starts gliding with a jump press in the air, glides
//! and steers, boosts with a firework rocket (hotbar slot 0), dives and lands. Give it an elytra
//! in the chest slot and rockets first (see `lab/elytra.sh`).
//!
//! ```text
//! cargo run -p ghast --release --example elytra_flight -- --host 127.0.0.1 --port 25626 \
//!     --username GhastBot10 --version 26.1 [--seed <rockets, default 2>]
//! ```
//!
//! Prints a `summary` line: ticks gliding, rockets used, whether it landed.

mod support;

use ghast::{Behavior, Buttons, Controls, Interests, TickContext};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Wait,
    Walk,
    Fall(u32),
    Glide(u32),
    Dive,
    Landed(u32),
}

struct Flight {
    phase: Phase,
    yaw: f32,
    rockets: u32,
    rockets_used: u32,
    glide_ticks: u32,
    started: u64,
}

impl Behavior for Flight {
    fn name(&self) -> &str {
        "elytra_flight"
    }

    fn interests(&self) -> Interests {
        Interests::CHAT
    }

    fn start(&mut self, ctx: &mut TickContext<'_>) {
        self.started = ctx.tick();
        ctx.select_slot(0);
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat {
            log::info!("chat: {}", m.plain());
        }
        let pose = *ctx.pose();
        let gliding = ctx.is_gliding();
        if gliding {
            self.glide_ticks += 1;
        }
        ctx.set_buttons(Buttons::default());
        let next = match self.phase {
            Phase::Wait => {
                ctx.set_controls(Controls::NONE);
                (ctx.tick() - self.started > 100).then_some(Phase::Walk)
            }
            Phase::Walk => {
                ctx.set_aim(self.yaw, 0.0);
                ctx.set_controls(Controls {
                    forward: true,
                    ..Controls::NONE
                });
                (!pose.on_ground).then_some(Phase::Fall(0))
            }
            Phase::Fall(n) => {
                ctx.set_aim(self.yaw, 0.0);
                ctx.set_controls(Controls::NONE);
                if gliding {
                    log::info!("gliding at y {:.2}", pose.pos.y);
                    Some(Phase::Glide(0))
                } else if n == 4 {
                    // A fresh jump press in the air: jump was released on the ticks before.
                    ctx.start_gliding();
                    Some(Phase::Fall(n + 1))
                } else if n > 30 {
                    log::warn!("no glide started");
                    Some(Phase::Landed(0))
                } else {
                    Some(Phase::Fall(n + 1))
                }
            }
            Phase::Glide(n) => {
                // Level flight, a slow turn; a rocket every 60 ticks.
                self.yaw += 1.5;
                ctx.set_aim(self.yaw, 8.0);
                ctx.set_controls(Controls::NONE);
                if n % 60 == 30 && self.rockets_used < self.rockets {
                    ctx.set_buttons(Buttons {
                        left: false,
                        right: true,
                    });
                    self.rockets_used += 1;
                    log::info!("rocket {} at y {:.2}", self.rockets_used, pose.pos.y);
                }
                if !gliding {
                    Some(Phase::Landed(0))
                } else if n > 60 * u32::max(self.rockets, 1) + 40 {
                    Some(Phase::Dive)
                } else {
                    Some(Phase::Glide(n + 1))
                }
            }
            Phase::Dive => {
                ctx.set_aim(self.yaw, 35.0);
                ctx.set_controls(Controls::NONE);
                (!gliding && pose.on_ground).then_some(Phase::Landed(0))
            }
            Phase::Landed(n) => {
                ctx.set_controls(Controls::NONE);
                if n == 0 {
                    log::info!(
                        "landed at {:.2} {:.2} {:.2}",
                        pose.pos.x,
                        pose.pos.y,
                        pose.pos.z
                    );
                }
                if n > 40 {
                    log::info!(
                        "summary: {} ticks gliding, {} rockets used, landed {}",
                        self.glide_ticks,
                        self.rockets_used,
                        pose.on_ground && !gliding
                    );
                    ctx.disconnect("flight done");
                }
                Some(Phase::Landed(n + 1))
            }
        };
        if let Some(p) = next {
            self.phase = p;
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
        let client = opts.client().expect("client");
        let mut f = Flight {
            phase: Phase::Wait,
            yaw: 0.0,
            rockets: u32::try_from(opts.seed).unwrap_or(2).min(10),
            rockets_used: 0,
            glide_ticks: 0,
            started: 0,
        };
        match client.run(&mut f).await {
            Ok(e) => log::info!("session ended: {e:?}"),
            Err(e) => log::error!("session failed: {e}"),
        }
    });
}
