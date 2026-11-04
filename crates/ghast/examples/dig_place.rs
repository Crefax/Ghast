//! Dig and place loop for the lab: breaks the grass block next to the spawn with the left
//! button (vanilla dig timing for the held item), then places a block back on the dirt below
//! with `place`, over and over. Grim alerts that reach the chat are logged as `ALERT`.
//!
//! Give the bot something to place first, e.g. `lab/console.sh 1.8 "give GhastBot2 dirt 64"`.
//!
//! ```text
//! cargo run -p ghast --release --example dig_place -- --host 127.0.0.1 --port 25618 \
//!     --username GhastBot2 --version 1.8.9 --minutes 5
//! ```

mod support;

use std::time::{Duration, Instant};

use ghast::{Behavior, BlockPos, Buttons, Controls, Face, Interests, TickContext, Vec3};

/// What the loop is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Teleport to the start and settle.
    Start(u32),
    /// Turn to the block and hold left until it breaks.
    Dig,
    /// Turn to the dirt below and place.
    Place(u32),
}

struct DigPlace {
    phase: Phase,
    block: BlockPos,
    broken: u64,
    placed: u32,
    alerts: u32,
    started: Instant,
    limit: Option<Duration>,
}

/// Yaw / pitch from `eyes` to `p` (vanilla convention).
fn look_at(eyes: Vec3, p: Vec3) -> (f32, f32) {
    let (dx, dy, dz) = (p.x - eyes.x, p.y - eyes.y, p.z - eyes.z);
    let yaw = (-dx).atan2(dz).to_degrees();
    let pitch = -(dy.atan2((dx * dx + dz * dz).sqrt())).to_degrees();
    (yaw as f32, pitch as f32)
}

impl Behavior for DigPlace {
    fn name(&self) -> &str {
        "dig-place"
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
        if self.limit.is_some_and(|l| self.started.elapsed() >= l) {
            log::info!(
                "time limit: {} broken, {} placed, {} alerts",
                ctx.broken(),
                self.placed,
                self.alerts
            );
            ctx.disconnect("time limit reached");
            return;
        }
        ctx.set_controls(Controls::NONE);
        let eyes = ctx.eyes(ctx.pose());
        match self.phase {
            Phase::Start(0) => {
                ctx.chat("/tp 0.5 4 0.5");
                self.phase = Phase::Start(1);
            }
            Phase::Start(n) if n < 30 => self.phase = Phase::Start(n + 1),
            Phase::Start(_) => {
                // Whatever is in hotbar slot 0 (the /give target).
                ctx.select_slot(0);
                self.phase = Phase::Dig;
            }
            Phase::Dig => {
                let b = self.block;
                let top = Vec3::new(
                    f64::from(b.x) + 0.5,
                    f64::from(b.y) + 1.0,
                    f64::from(b.z) + 0.5,
                );
                let (yaw, pitch) = look_at(eyes, top);
                ctx.set_aim(yaw, pitch);
                let on_it = ctx.target().is_some_and(|t| t.pos == b);
                if ctx.tick().is_multiple_of(10) {
                    log::debug!(
                        "dig: target {:?} mining {:?} held {:?} block {:?}",
                        ctx.target().map(|t| (t.pos, t.face)),
                        ctx.mining(),
                        ctx.inventory().held().map(|s| (s.id, s.count)),
                        ctx.world().raw_block(b.x, b.y, b.z)
                    );
                }
                ctx.set_buttons(Buttons {
                    left: on_it,
                    right: false,
                });
                if ctx.broken() > self.broken {
                    self.broken = ctx.broken();
                    ctx.set_buttons(Buttons::default());
                    log::info!("broken #{} at tick {}", self.broken, ctx.tick());
                    self.phase = Phase::Place(0);
                }
            }
            Phase::Place(n) => {
                let support = Face::Down.offset(self.block);
                let hit = Vec3::new(
                    f64::from(support.x) + 0.5,
                    f64::from(support.y) + 1.0,
                    f64::from(support.z) + 0.5,
                );
                let (yaw, pitch) = look_at(eyes, hit);
                ctx.set_aim(yaw, pitch);
                // Place once the sent look is on the dirt's top face (the server judges with it).
                if n > 2 && ctx.place(support, Face::Up, hit) {
                    self.placed += 1;
                    log::info!("placed #{} at tick {}", self.placed, ctx.tick());
                    self.phase = Phase::Dig;
                } else if n > 200 {
                    log::warn!("could not place");
                    self.phase = Phase::Dig;
                } else {
                    self.phase = Phase::Place(n + 1);
                }
            }
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
        let mut b = DigPlace {
            phase: Phase::Start(0),
            block: BlockPos::new(1, 3, 0),
            broken: 0,
            placed: 0,
            alerts: 0,
            started: Instant::now(),
            limit: opts.limit(),
        };
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        match client.run_until(&mut b, shutdown).await {
            Ok(e) => log::info!(
                "session ended: {e:?}; {} placed, {} alerts",
                b.placed,
                b.alerts
            ),
            Err(e) => log::error!("session failed: {e}"),
        }
    });
}
