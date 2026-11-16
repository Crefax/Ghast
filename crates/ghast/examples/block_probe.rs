//! Lab probe for the client's own predictions: one action, then what the bot sees for a while.
//!
//! The action comes from the environment (the lab scripts set it):
//!
//! - `PROBE_MODE=dig`: break the block at `PROBE_AT` with the left button (its `PROBE_FACE`).
//! - `PROBE_MODE=place`: right-click `PROBE_FACE` of `PROBE_AT` with hotbar slot 0 (`place`).
//! - `PROBE_MODE=drop` / `swap`: drop one item of hotbar slot 0 / swap the hands (1.9+).
//!
//! The bot waits `PROBE_WAIT` ticks after joining (default 100, time for the script's `/tp`),
//! acts, watches the block (dig: `PROBE_AT`, place: the block next to the face) or the hands
//! for `PROBE_SETTLE` ticks (default 60) after the action and prints every change with its tick
//! counted from the first try (`ACT@t<n>`: the action was written on tick n; for a dig, the tick
//! after the finish went out):
//!
//! ```text
//! PROBE Place: t1 minecraft:air, ACT@t4, t5 minecraft:stone, t6 minecraft:air | final minecraft:air
//! ```
//!
//! ```text
//! PROBE_MODE=place PROBE_AT=2,3,0 cargo run -p ghast --release --example block_probe -- \
//!     --host 127.0.0.1 --port 25626 --username GhastProbe --version 26.1 --minutes 1
//! ```

mod support;

use ghast::{
    Behavior, BlockPos, Buttons, Controls, Face, GameVersion, Interests, TickContext, Vec3,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Dig,
    Place,
    Drop,
    Swap,
}

struct Probe {
    mode: Mode,
    at: BlockPos,
    face: Face,
    wait: u64,
    settle: u64,
    /// The tick of the action.
    acted: Option<u64>,
    /// Ticks spent trying to act.
    trying: u64,
    /// Give up after this many (`PROBE_TRIES`, default 400): "could not act".
    tries: u64,
    broken_before: u64,
    last: Option<String>,
    changes: Vec<String>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Yaw / pitch from `eyes` to `p` (vanilla convention).
fn look_at(eyes: Vec3, p: Vec3) -> (f32, f32) {
    let (dx, dy, dz) = (p.x - eyes.x, p.y - eyes.y, p.z - eyes.z);
    let yaw = (-dx).atan2(dz).to_degrees();
    let pitch = -(dy.atan2((dx * dx + dz * dz).sqrt())).to_degrees();
    (yaw as f32, pitch as f32)
}

/// The middle of a block face.
fn face_centre(p: BlockPos, f: Face) -> Vec3 {
    let n = f.offset(p);
    Vec3::new(
        f64::from(p.x) + 0.5 + f64::from(n.x - p.x) * 0.5,
        f64::from(p.y) + 0.5 + f64::from(n.y - p.y) * 0.5,
        f64::from(p.z) + 0.5 + f64::from(n.z - p.z) * 0.5,
    )
}

/// A block state as its name (the version's own data).
fn state_name(v: GameVersion, raw: Option<u32>) -> String {
    use ghast::data;
    let Some(raw) = raw else {
        return "unloaded".into();
    };
    let s = raw as u16;
    match v {
        GameVersion::V1_8_9 => data::v1_8::BlockState::from_raw(s)
            .map_or(format!("raw {raw}"), |b| {
                format!("{}:{}", b.block().name(), b.meta())
            }),
        GameVersion::V1_12_2 => data::v1_12::BlockState::from_raw(s)
            .map_or(format!("raw {raw}"), |b| {
                format!("{}:{}", b.block().name(), b.meta())
            }),
        GameVersion::V1_16_5 => data::v1_16_5::BlockState::from_id(s)
            .map_or(format!("id {raw}"), |b| b.block().name().to_string()),
        GameVersion::V26_2 => {
            data::v26_2::BlockState::from_id(s).map_or(format!("id {raw}"), |b| b.to_string())
        }
        _ => data::v26_1::BlockState::from_id(s).map_or(format!("id {raw}"), |b| b.to_string()),
    }
}

impl Probe {
    /// What is watched after the action.
    fn watched(&self, ctx: &TickContext<'_>) -> String {
        match self.mode {
            Mode::Dig | Mode::Place => {
                let p = if self.mode == Mode::Dig {
                    self.at
                } else {
                    self.face.offset(self.at)
                };
                state_name(ctx.version(), ctx.world().raw_block(p.x, p.y, p.z))
            }
            Mode::Drop | Mode::Swap => {
                let inv = ctx.inventory();
                let s = |x: Option<&ghast::inventory::Stack>| {
                    x.map_or("empty".to_string(), |s| format!("{}x{}", s.id(), s.count()))
                };
                format!("main {} off {}", s(inv.held()), s(inv.offhand()))
            }
        }
    }
}

impl Behavior for Probe {
    fn name(&self) -> &str {
        "block-probe"
    }

    fn interests(&self) -> Interests {
        Interests::CHAT
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        ctx.set_controls(Controls::NONE);
        let tick = ctx.tick();
        if tick < self.wait {
            // Hotbar slot 0 (the server may have restored another one at the join).
            if tick + 20 == self.wait {
                ctx.select_slot(0);
            }
            return;
        }
        let eyes = ctx.eyes(ctx.pose());
        // Every change from the first try on, by tick since the first try.
        let now = self.watched(ctx);
        if self.last.as_ref() != Some(&now) {
            self.changes.push(format!("t{} {now}", self.trying));
            self.last = Some(now);
        }
        if let Some(t0) = self.acted {
            if self.mode == Mode::Dig {
                ctx.set_buttons(Buttons::default());
            }
            self.trying += 1;
            if tick - t0 >= self.settle {
                println!(
                    "PROBE {:?}: {} | final {}",
                    self.mode,
                    self.changes.join(", "),
                    self.last.as_deref().unwrap_or("?")
                );
                ctx.disconnect("probe done");
            }
            return;
        }
        self.trying += 1;
        if self.trying > self.tries {
            println!(
                "PROBE {:?}: could not act | final {}",
                self.mode,
                self.watched(ctx)
            );
            ctx.disconnect("probe failed");
            return;
        }
        let before = self.watched(ctx);
        match self.mode {
            Mode::Dig => {
                if self.trying == 1 {
                    self.broken_before = ctx.broken();
                    log::info!("before: {before}");
                }
                let (yaw, pitch) = look_at(eyes, face_centre(self.at, self.face));
                ctx.set_aim(yaw, pitch);
                let on_it = ctx.target().is_some_and(|t| t.pos == self.at);
                ctx.set_buttons(Buttons {
                    left: on_it,
                    right: false,
                });
                if ctx.broken() > self.broken_before {
                    ctx.set_buttons(Buttons::default());
                    self.acted = Some(tick);
                }
            }
            Mode::Place => {
                if self.trying == 1 {
                    log::info!("before: {before}");
                }
                let hit = face_centre(self.at, self.face);
                let (yaw, pitch) = look_at(eyes, hit);
                ctx.set_aim(yaw, pitch);
                if self.trying > 3 && ctx.place(self.at, self.face, hit) {
                    self.acted = Some(tick);
                }
            }
            Mode::Drop => {
                log::info!("before: {before}");
                if ctx.drop_held(false) {
                    self.acted = Some(tick);
                }
            }
            Mode::Swap => {
                log::info!("before: {before}");
                if ctx.swap_hands() {
                    self.acted = Some(tick);
                }
            }
        }
        if self.acted.is_some() {
            self.changes.push(format!("ACT@t{}", self.trying));
        }
    }
}

fn main() {
    let opts = support::Options::parse();
    support::init_logger(opts.verbose);
    let mode = match env("PROBE_MODE").as_deref() {
        Some("place") => Mode::Place,
        Some("drop") => Mode::Drop,
        Some("swap") => Mode::Swap,
        _ => Mode::Dig,
    };
    let at: Vec<i32> = env("PROBE_AT")
        .unwrap_or_else(|| "1,3,0".into())
        .split(',')
        .map(|x| x.trim().parse().expect("PROBE_AT x,y,z"))
        .collect();
    let face = match env("PROBE_FACE").as_deref() {
        Some("down") => Face::Down,
        Some("north") => Face::North,
        Some("south") => Face::South,
        Some("west") => Face::West,
        Some("east") => Face::East,
        _ => Face::Up,
    };
    let num = |n: &str, d: u64| env(n).and_then(|v| v.parse().ok()).unwrap_or(d);
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
        let mut b = Probe {
            mode,
            at: BlockPos::new(at[0], at[1], at[2]),
            face,
            wait: num("PROBE_WAIT", 100),
            settle: num("PROBE_SETTLE", 60),
            acted: None,
            trying: 0,
            tries: num("PROBE_TRIES", 400),
            broken_before: 0,
            last: None,
            changes: Vec::new(),
        };
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        match client.run_until(&mut b, shutdown).await {
            Ok(e) => log::info!("session ended: {e:?}"),
            Err(e) => log::error!("session failed: {e}"),
        }
    });
}
