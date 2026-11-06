//! Measurement harness: ghast's movement course on the 1.8 lab test pad, wrapped in the viewer.
//! The course logic is ghast's `movement_course` example (walks, sprints, jumps, sneaks, steps
//! up, crosses ice / soul sand / slime / water, turns while moving); each lap starts with
//! `/tp 0.5 4 0.5`, so the account must be opped. Used to measure what the viewer costs while
//! the bot moves.
//!
//! ```text
//! cargo run --release --example course_viewer -- <host:port> <name> <seed> <minutes> \
//!     [--port <viewer port>] [--record <dir>]
//! ```

use std::time::{Duration, Instant};

use ghast_client::{Account, Behavior, Client, Controls, GameVersion, Interests, TickContext};
use ghast_viewer::{FeedRecorder, Viewer, ViewerConfig};

/// How the keys are held on a leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Walk,
    Sprint,
    SprintJump,
    Jump,
    Sneak,
    Swim,
}

#[derive(Debug, Clone)]
enum Leg {
    /// Run a chat command, then wait for the teleport it causes.
    Command(String),
    /// Stand still.
    Wait(u32),
    /// Go to (x, z).
    Goto {
        name: &'static str,
        x: f64,
        z: f64,
        mode: Mode,
    },
    /// Walk toward (x, z) for a fixed number of ticks (into a wall, over an edge).
    Push {
        name: &'static str,
        x: f64,
        z: f64,
        mode: Mode,
        ticks: u32,
    },
    /// Strafe left while turning right: a circle.
    Circle { ticks: u32, yaw_per_tick: f32 },
}

/// xorshift64*.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[(self.next() % v.len() as u64) as usize]
    }
}

/// One lap of the course.
fn lap(rng: &mut Rng) -> Vec<Leg> {
    let mut legs = vec![Leg::Command("/tp 0.5 4 0.5".into()), Leg::Wait(20)];
    // The five floor strips (x0, name, modes) in random order and direction.
    let mut strips = vec![
        (
            0.0,
            "ice",
            &[Mode::Walk, Mode::Sprint, Mode::SprintJump][..],
        ),
        (
            8.0,
            "packed ice",
            &[Mode::Walk, Mode::Sprint, Mode::SprintJump][..],
        ),
        (
            16.0,
            "soul sand",
            &[Mode::Walk, Mode::Sprint, Mode::Jump, Mode::Sneak][..],
        ),
        (
            24.0,
            "slime",
            &[Mode::Walk, Mode::Jump, Mode::SprintJump][..],
        ),
        (32.0, "water", &[Mode::Swim][..]),
    ];
    for i in (1..strips.len()).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        strips.swap(i, j);
    }
    let mut last_x = 0.5;
    for (x0, name, modes) in strips {
        let x = x0 + 1.5 + rng.unit() * 5.0;
        last_x = x;
        let (z_from, z_to) = if rng.next().is_multiple_of(2) {
            (5.5, 26.5)
        } else {
            (26.5, 5.5)
        };
        legs.push(Leg::Goto {
            name: "approach",
            x,
            z: z_from,
            mode: Mode::Walk,
        });
        legs.push(Leg::Goto {
            name,
            x,
            z: z_to,
            mode: rng.pick(modes),
        });
    }
    // Step-up course at x 40..47: slab (z 8), block (z 10), two-high wall (z 12), fence (z 14).
    let x = 41.5 + rng.unit() * 5.0;
    // Leave the strips northward so the walk to the course does not cut through it.
    legs.push(Leg::Goto {
        name: "leave the strips",
        x: last_x,
        z: -2.0,
        mode: Mode::Walk,
    });
    legs.push(Leg::Goto {
        name: "north of the course",
        x,
        z: -2.0,
        mode: Mode::Walk,
    });
    legs.push(Leg::Goto {
        name: "course start",
        x,
        z: 6.0,
        mode: Mode::Walk,
    });
    legs.push(Leg::Goto {
        name: "slab step-up",
        x,
        z: 8.5,
        mode: Mode::Walk,
    });
    legs.push(Leg::Goto {
        name: "jump onto block",
        x,
        z: 10.5,
        mode: Mode::Jump,
    });
    legs.push(Leg::Push {
        name: "sneak at the edge",
        x,
        z: 14.0,
        mode: Mode::Sneak,
        ticks: 30,
    });
    legs.push(Leg::Push {
        name: "drop off, into the wall",
        x,
        z: 14.0,
        mode: Mode::Walk,
        ticks: 30,
    });
    legs.push(Leg::Push {
        name: "jump at the wall",
        x,
        z: 14.0,
        mode: Mode::Jump,
        ticks: 20,
    });
    legs.push(Leg::Goto {
        name: "around the wall",
        x: 49.0,
        z: 11.0,
        mode: Mode::Walk,
    });
    legs.push(Leg::Goto {
        name: "around the wall",
        x: 49.0,
        z: 13.5,
        mode: Mode::Sprint,
    });
    legs.push(Leg::Goto {
        name: "to the fence",
        x,
        z: 13.4,
        mode: Mode::Walk,
    });
    legs.push(Leg::Push {
        name: "into the fence",
        x,
        z: 17.0,
        mode: Mode::Walk,
        ticks: 25,
    });
    legs.push(Leg::Push {
        name: "jump at the fence",
        x,
        z: 17.0,
        mode: Mode::Jump,
        ticks: 25,
    });
    legs.push(Leg::Goto {
        name: "open grass",
        x: 20.0,
        z: -6.0,
        mode: Mode::Sprint,
    });
    let dir = if rng.next().is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    legs.push(Leg::Circle {
        ticks: 60 + (rng.next() % 60) as u32,
        yaw_per_tick: dir * (3.0 + rng.unit() as f32 * 6.0),
    });
    legs.push(Leg::Goto {
        name: "home",
        x: 0.5,
        z: 0.5,
        mode: rng.pick(&[Mode::Walk, Mode::Sprint, Mode::SprintJump]),
    });
    legs
}

struct Course {
    rng: Rng,
    legs: Vec<Leg>,
    leg: usize,
    leg_ticks: u32,
    laps: u32,
    expect_teleport: u32,
    /// Ticks to stand still after a setback (a player stops and looks).
    hold: u32,
    alerts: u32,
    pitch: f32,
    started: Instant,
    limit: Option<Duration>,
    /// `GHAST_TRACE=<file>`: one JSON line per tick (pose at the tick start, keys, aim).
    trace: Option<std::io::BufWriter<std::fs::File>>,
}

impl Course {
    fn next_leg(&mut self, ctx: &TickContext<'_>, why: &str) {
        let p = ctx.pose().pos;
        if let Some(l) = self.legs.get(self.leg) {
            if let Leg::Goto { name, .. } | Leg::Push { name, .. } = l {
                log::info!(
                    "leg {} {name}: {why} after {} ticks at {:.2} {:.2} {:.2}",
                    self.leg,
                    self.leg_ticks,
                    p.x,
                    p.y,
                    p.z
                );
            }
            log::debug!(
                "leg {} {why} after {} ticks at {:.3} {:.3} {:.3}: {l:?}",
                self.leg,
                self.leg_ticks,
                p.x,
                p.y,
                p.z
            );
        }
        self.leg += 1;
        self.leg_ticks = 0;
        if self.leg >= self.legs.len() {
            self.laps += 1;
            log::info!("lap {} done ({} alerts so far)", self.laps, self.alerts);
            self.legs = lap(&mut self.rng);
            self.leg = 0;
            self.pitch = (self.rng.unit() * 30.0) as f32 - 5.0;
        }
    }

    fn controls(mode: Mode) -> Controls {
        let mut c = Controls {
            forward: true,
            ..Controls::NONE
        };
        match mode {
            Mode::Walk => {}
            Mode::Sprint => c.sprint = true,
            Mode::SprintJump => {
                c.sprint = true;
                c.jump = true;
            }
            Mode::Jump | Mode::Swim => c.jump = true,
            Mode::Sneak => c.sneak = true,
        }
        c
    }

    /// Aims at (x, z) and returns the remaining horizontal distance and yaw error.
    fn steer(&self, ctx: &mut TickContext<'_>, x: f64, z: f64) -> (f64, f32) {
        let p = ctx.pose().pos;
        let (dx, dz) = (x - p.x, z - p.z);
        let target = (-dx).atan2(dz).to_degrees() as f32;
        // Unwrapped yaw close to the current one.
        let yaw = ctx.pose().yaw;
        let target = yaw + ghast_client::rotation::wrap_degrees(target - yaw);
        ctx.set_aim(target, self.pitch);
        (
            (dx * dx + dz * dz).sqrt(),
            ghast_client::rotation::wrap_degrees(target - yaw).abs(),
        )
    }
}

impl Behavior for Course {
    fn name(&self) -> &str {
        "movement-course"
    }

    fn interests(&self) -> Interests {
        Interests::CHAT
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        self.drive(ctx);
        if let Some(w) = &mut self.trace {
            use std::io::Write;
            let p = ctx.pose();
            let s = ctx.sent();
            let c = ctx.controls();
            let (ay, ap) = ctx.aim();
            let keys: String = [
                (c.forward, 'F'),
                (c.back, 'B'),
                (c.left, 'L'),
                (c.right, 'R'),
                (c.jump, 'J'),
                (c.sneak, 'S'),
                (c.sprint, 'P'),
            ]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, k)| *k)
            .collect();
            let _ = writeln!(
                w,
                r#"{{"t":{},"leg":{},"tp":{},"pos":[{:?},{:?},{:?}],"vel":[{:?},{:?},{:?}],"yaw":{:?},"pitch":{:?},"ground":{},"sneak":{},"sprint":{},"sent":[{:?},{:?},{:?},{:?},{:?},{}],"keys":"{keys}","aim":[{ay:?},{ap:?}]}}"#,
                ctx.tick(),
                self.leg,
                ctx.events().teleported,
                p.pos.x,
                p.pos.y,
                p.pos.z,
                p.motion.x,
                p.motion.y,
                p.motion.z,
                p.yaw,
                p.pitch,
                p.on_ground,
                p.sneaking,
                p.sprinting,
                s.pos.x,
                s.pos.y,
                s.pos.z,
                s.yaw,
                s.pitch,
                s.on_ground
            );
        }
    }
}

impl Course {
    fn drive(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat {
            let t = m.plain();
            if t.contains("Grim") {
                self.alerts += 1;
                log::warn!("ALERT chat: {t}");
            }
        }
        if ctx.events().teleported {
            if self.expect_teleport > 0 {
                self.expect_teleport = 0;
            } else {
                self.alerts += 1;
                self.hold = 10;
                let p = ctx.pose().pos;
                log::warn!(
                    "ALERT setback to {:.3} {:.3} {:.3} on leg {} {:?}",
                    p.x,
                    p.y,
                    p.z,
                    self.leg,
                    self.legs.get(self.leg)
                );
            }
        }
        if self.expect_teleport > 0 {
            self.expect_teleport -= 1;
        }
        if self.limit.is_some_and(|l| self.started.elapsed() >= l) {
            log::info!("time limit: {} laps, {} alerts", self.laps, self.alerts);
            ctx.disconnect("time limit reached");
            return;
        }
        if ctx.is_dead() {
            ctx.respawn();
            return;
        }
        if self.hold > 0 {
            self.hold -= 1;
            ctx.set_controls(Controls::NONE);
            return;
        }
        self.leg_ticks += 1;
        let leg = self.legs[self.leg].clone();
        match leg {
            Leg::Command(cmd) => {
                ctx.set_controls(Controls::NONE);
                ctx.chat(cmd);
                self.expect_teleport = 40;
                self.next_leg(ctx, "sent");
            }
            Leg::Wait(n) => {
                ctx.set_controls(Controls::NONE);
                if self.leg_ticks >= n {
                    self.next_leg(ctx, "waited");
                }
            }
            Leg::Goto { x, z, mode, .. } => {
                let (dist, err) = self.steer(ctx, x, z);
                if dist < 0.6 {
                    ctx.set_controls(Controls::NONE);
                    self.next_leg(ctx, "arrived");
                } else if self.leg_ticks > 1200 {
                    ctx.set_controls(Controls::NONE);
                    log::warn!("leg {} timed out ({dist:.2} blocks left)", self.leg);
                    self.next_leg(ctx, "timed out");
                } else {
                    // Turn first when far off, then move while still turning.
                    let mut c = if err > 60.0 {
                        Controls::NONE
                    } else {
                        Self::controls(mode)
                    };
                    // Below the floor (in the pool): swim up and climb out.
                    if ctx.pose().pos.y < 3.9 {
                        c.jump = true;
                    }
                    ctx.set_controls(c);
                }
            }
            Leg::Push {
                x, z, mode, ticks, ..
            } => {
                self.steer(ctx, x, z);
                ctx.set_controls(Self::controls(mode));
                if self.leg_ticks >= ticks {
                    ctx.set_controls(Controls::NONE);
                    self.next_leg(ctx, "pushed");
                }
            }
            Leg::Circle {
                ticks,
                yaw_per_tick,
            } => {
                let yaw = ctx.pose().yaw;
                ctx.set_aim(yaw + yaw_per_tick, self.pitch);
                ctx.set_controls(Controls {
                    forward: true,
                    left: yaw_per_tick > 0.0,
                    right: yaw_per_tick < 0.0,
                    ..Controls::NONE
                });
                if self.leg_ticks >= ticks {
                    ctx.set_controls(Controls::NONE);
                    self.next_leg(ctx, "circled");
                }
            }
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
    let port: u16 = flag(&mut args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(8095);
    let addr = args
        .first()
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:25618".into());
    let name = args.get(1).cloned().unwrap_or_else(|| "ViewerBot".into());
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(7);
    let minutes: f64 = args.get(3).and_then(|m| m.parse().ok()).unwrap_or(5.0);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let viewer = Viewer::new(
            ViewerConfig::new(&name)
                .port(port)
                .idle(Duration::from_secs(u64::MAX / 4)),
        );
        let at = viewer.start().await?;
        log::info!("bot screen: http://{at}/");
        let mut rec = record.as_ref().map(|d| FeedRecorder::start(&viewer, d));
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let legs = lap(&mut rng);
        let course = Course {
            rng,
            legs,
            leg: 0,
            leg_ticks: 0,
            laps: 0,
            expect_teleport: 40,
            hold: 0,
            alerts: 0,
            pitch: 10.0,
            started: Instant::now(),
            limit: Some(Duration::from_secs_f64(minutes * 60.0)),
            trace: None,
        };
        let client = Client::new(addr.parse()?, GameVersion::V1_8_9, Account::offline(&name)?)
            .world_config(ghast_viewer::world_config());
        let mut bot = viewer.watch(course);
        let end = client.run(&mut bot).await;
        log::info!("session ended: {end:?}; {} laps", bot.inner().laps);
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
