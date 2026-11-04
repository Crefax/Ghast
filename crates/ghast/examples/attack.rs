//! Entity test for the lab: tracks entities, aims at the nearest living entity within 3
//! blocks and attacks it when the sent crosshair is on it (1.16: only at full attack strength),
//! right-clicking it now and then; logs what it sees. Grim alerts in chat are logged as `ALERT`.
//!
//! Summon something next to the spawn first, e.g.
//! `lab/console.sh 1.8 "summon Pig 2.5 4 0.5 {NoAI:1}"` or
//! `lab/console.sh 1.16 "summon pig 2.5 4 0.5 {NoAI:1b}"`.
//!
//! ```text
//! cargo run -p ghast --release --example attack -- --host 127.0.0.1 --port 25618 \
//!     --username GhastBot2 --version 1.8.9 --minutes 1
//! ```

mod support;

use std::time::{Duration, Instant};

use ghast::{Behavior, Buttons, Interests, TickContext, Vec3};

struct Attack {
    n: u64,
    attacks: u32,
    interacts: u32,
    alerts: u32,
    started: Instant,
    limit: Option<Duration>,
}

impl Behavior for Attack {
    fn name(&self) -> &str {
        "attack"
    }

    fn interests(&self) -> Interests {
        Interests::CHAT | Interests::ENTITIES
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
                "time limit: {} attacks, {} interacts, {} alerts",
                self.attacks,
                self.interacts,
                self.alerts
            );
            ctx.disconnect("time limit reached");
            return;
        }
        self.n += 1;
        ctx.set_buttons(Buttons::default());
        let eyes = ctx.eyes(ctx.pose());
        let Some(ents) = ctx.entities() else {
            return;
        };
        let target = ents
            .within([eyes.x, eyes.y, eyes.z], 3.0)
            .filter(|e| !e.is_player() && e.bounding_box().is_some())
            .map(|e| (e.id(), e.bounding_box().unwrap_or_default()))
            .next();
        if self.n.is_multiple_of(100) {
            log::info!(
                "tick {}: {} entities tracked, target {target:?}",
                ctx.tick(),
                ents.len()
            );
            for e in ents.iter() {
                log::debug!(
                    "  {} {:?} {:?} at {:?} box {:?}",
                    e.id(),
                    e.entity_type(),
                    e.class(),
                    e.position(),
                    e.bounding_box()
                );
            }
        }
        let Some((id, b)) = target else {
            return;
        };
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
        let on_it = ctx.target_entity().is_some_and(|e| e.id == id);
        if on_it && self.n.is_multiple_of(40) {
            // Right click with the buttons (interact-at + interact, then the item).
            ctx.set_buttons(Buttons {
                left: false,
                right: true,
            });
            self.interacts += 1;
        } else if on_it
            && self.n.is_multiple_of(10)
            && ctx.attack_strength(4.0) >= 1.0
            && ctx.attack(id)
        {
            self.attacks += 1;
            if self.attacks.is_multiple_of(10) {
                log::info!("{} attacks on {id}", self.attacks);
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
        let mut b = Attack {
            n: 0,
            attacks: 0,
            interacts: 0,
            alerts: 0,
            started: Instant::now(),
            limit: opts.limit(),
        };
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        match client.run_until(&mut b, shutdown).await {
            Ok(e) => log::info!(
                "session ended: {e:?}; {} attacks, {} alerts",
                b.attacks,
                b.alerts
            ),
            Err(e) => log::error!("session failed: {e}"),
        }
    });
}
