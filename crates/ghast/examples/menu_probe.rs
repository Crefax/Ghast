//! Live menu checks for the lab (1.8.9, 1.12.2, 1.16.5): opens one screen (a block in front of
//! the spawn pad, or the nearest entity), runs a click script and checks every click against
//! the server: the click's confirm must be "accepted", and the screen 4 ticks after the confirm
//! must still equal the prediction made when the click was sent (a `set_slot` that changes it
//! is a server correction). `v` closes and reopens the screen and compares the server's fresh
//! contents (and, in a villager screen, the trade uses) with the prediction.
//!
//! The scene comes from the environment:
//! * `GHAST_MENU_BLOCK=x,y,z`: right-click that block (the bot stands south of it at 0.5 4 0.5
//!   facing south, so use `0,4,2`; the aim is at its centre, `GHAST_MENU_AIM_Y=0.06` lower for
//!   a flat ray shape such as the 1.12 brewing stand's base), or
//!   `GHAST_MENU_ENTITY=1`: right-click the
//!   nearest tracked entity (`GHAST_MENU_SNEAK=1`: while sneaking, e.g. a tamed horse).
//! * `GHAST_MENU_STEPS`: space-separated steps. Slots are window slot numbers, or `hK` for the
//!   player's hotbar slot K and `iK` for main inventory slot K (row-major, 0..27) of the open
//!   screen:
//!   `pS` / `rS` pickup left / right, `qS` shift-click, `tN` select trade N, `bN` menu button
//!   N, `wN` wait N ticks, `v` verify by reopening.
//!
//! ```text
//! GHAST_MENU_BLOCK=0,4,2 GHAST_MENU_STEPS="qh0 q0 v" cargo run -p ghast --release \
//!     --example menu_probe -- --host 127.0.0.1 --port 25618 --username GhastBot3 --version 1.8.9
//! ```
//! `GHAST_MENU_RETURNS=1`: the screen gives its items back on close (enchanting table,
//! workstations); the reopen then checks the screen is empty and no item was lost.
//! `GHAST_MENU_LIVE=0,1`: container slots the server's own tick may change (a burning
//! furnace); their changes are only printed. `GHAST_MENU_RESULT=N`: the result slot (its
//! preview is not counted as an item).
//! `lab/menus.sh` sets the scenes up. Prints `PASS` when every check held.

mod support;

use ghast::inventory::{Click, ClickError, ClickOutcome, Stack};
use ghast::{Behavior, BlockPos, Buttons, ClickRefused, Controls, Interests, TickContext, Vec3};

#[derive(Debug, Clone, Copy)]
enum Slot {
    Abs(i16),
    Hotbar(i16),
    Main(i16),
}

#[derive(Debug, Clone, Copy)]
enum Step {
    Pickup(Slot, bool),
    Quick(Slot),
    Trade(i32),
    Button(i32),
    Wait(u32),
    Verify,
}

fn parse_slot(s: &str) -> Option<Slot> {
    if let Some(k) = s.strip_prefix('h') {
        k.parse().ok().map(Slot::Hotbar)
    } else if let Some(k) = s.strip_prefix('i') {
        k.parse().ok().map(Slot::Main)
    } else {
        s.parse().ok().map(Slot::Abs)
    }
}

fn parse_steps(s: &str) -> Result<Vec<Step>, String> {
    s.split_whitespace()
        .map(|t| {
            let (head, rest) = t.split_at(1);
            let bad = || format!("bad step {t:?}");
            Ok(match head {
                "p" => Step::Pickup(parse_slot(rest).ok_or_else(bad)?, false),
                "r" => Step::Pickup(parse_slot(rest).ok_or_else(bad)?, true),
                "q" => Step::Quick(parse_slot(rest).ok_or_else(bad)?),
                "t" => Step::Trade(rest.parse().map_err(|_| bad())?),
                "b" => Step::Button(rest.parse().map_err(|_| bad())?),
                "w" => Step::Wait(rest.parse().map_err(|_| bad())?),
                "v" if rest.is_empty() => Step::Verify,
                _ => return Err(bad()),
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for the world, then opening the screen.
    Open,
    /// The screen is open: running the steps.
    Run,
    /// A sent click waits for its confirm.
    Confirm,
    /// The confirm came: compare the screen after a few ticks.
    Settle(u32),
    /// `v`: closed, waiting before the reopen.
    Reopen(u32),
    Finished,
}

type Key = Option<(i32, i32, i16, bool)>;

fn key(s: Option<&Stack>) -> Key {
    s.map(|s| (s.id(), s.count(), s.damage(), s.nbt().is_some()))
}

fn screen(ctx: &TickContext<'_>) -> Vec<Key> {
    let inv = ctx.inventory();
    (0..inv.slot_count()).map(|i| key(inv.slot(i))).collect()
}

fn trade_uses(ctx: &TickContext<'_>) -> Option<Vec<i32>> {
    ctx.inventory()
        .trade_offers()
        .map(|o| o.iter().map(|t| t.uses).collect())
}

struct Probe {
    block: Option<BlockPos>,
    /// Aim height in the block (0..1; default the centre).
    aim_y: f64,
    sneak: bool,
    steps: Vec<Step>,
    pc: usize,
    phase: Phase,
    ticks: u32,
    wait: u32,
    opened: bool,
    /// Ticks since the last window open.
    open_for: u32,
    /// Ticks since the open attempt started (a click every 10 ticks until a screen opens).
    trying: u32,
    predicted: Vec<Key>,
    predicted_cursor: Key,
    predicted_trades: Option<Vec<i32>>,
    /// The screen returns its items to the player on close (crafting-like menus).
    returns: bool,
    /// Container slots the server's own tick may change (a burning furnace's fuel and input).
    live: Vec<usize>,
    /// The screen's result slot (a preview, not an item: left out of the item count).
    result: Option<usize>,
    /// Container slots of the screen before the verify's close.
    container_len: usize,
    sent: u32,
    accepted: u32,
    failures: Vec<String>,
}

impl Probe {
    fn slot(ctx: &TickContext<'_>, s: Slot) -> i16 {
        let n = i16::try_from(ctx.inventory().container_len()).unwrap_or(0);
        match s {
            Slot::Abs(i) => i,
            Slot::Main(k) => n + k,
            Slot::Hotbar(k) => n + 27 + k,
        }
    }

    fn fail(&mut self, ctx: &mut TickContext<'_>, why: String) {
        self.failures.push(why);
        self.finish(ctx);
    }

    fn finish(&mut self, ctx: &mut TickContext<'_>) {
        ctx.set_buttons(Buttons::default());
        let _ = ctx.close_window();
        println!(
            "clicks sent {}, accepted {}, failures {}",
            self.sent,
            self.accepted,
            self.failures.len()
        );
        for f in &self.failures {
            println!("  FAIL {f}");
        }
        if self.failures.is_empty() && self.sent == self.accepted {
            println!("PASS");
        }
        self.phase = Phase::Finished;
        ctx.disconnect("probe done");
    }

    /// Aims at the block's north face or the nearest entity and right-clicks it.
    fn open(&mut self, ctx: &mut TickContext<'_>) {
        self.trying += 1;
        let eyes = ctx.eyes(ctx.pose());
        let (point, on) = if let Some(b) = self.block {
            // The block's centre: thin shapes (a brewing stand's rod) are hit on their north face.
            let p = Vec3::new(
                f64::from(b.x) + 0.5,
                f64::from(b.y) + self.aim_y,
                f64::from(b.z) + 0.5,
            );
            let on = ctx.target().is_some_and(|t| t.pos == b);
            (p, on)
        } else {
            let me = ctx.entities().and_then(|e| e.self_id());
            let near = ctx.entities().and_then(|es| {
                es.iter()
                    .filter(|e| Some(e.id()) != me)
                    .filter_map(|e| Some((e.id(), e.bounding_box()?)))
                    .min_by(|a, b| {
                        let d = |b: &[f64; 6]| {
                            let c = [
                                (b[0] + b[3]) / 2.0,
                                (b[1] + b[4]) / 2.0,
                                (b[2] + b[5]) / 2.0,
                            ];
                            (c[0] - eyes.x).powi(2)
                                + (c[1] - eyes.y).powi(2)
                                + (c[2] - eyes.z).powi(2)
                        };
                        d(&a.1).total_cmp(&d(&b.1))
                    })
            });
            let Some((id, b)) = near else {
                if self.trying > 200 {
                    self.fail(ctx, "no entity to open".into());
                }
                return;
            };
            let p = Vec3::new(
                (b[0] + b[3]) / 2.0,
                (b[1] + b[4]) / 2.0,
                (b[2] + b[5]) / 2.0,
            );
            (p, ctx.target_entity().is_some_and(|e| e.id == id))
        };
        let (dx, dy, dz) = (point.x - eyes.x, point.y - eyes.y, point.z - eyes.z);
        let yaw = (-dx).atan2(dz).to_degrees() as f32;
        let mut pitch = -(dy.atan2((dx * dx + dz * dz).sqrt())).to_degrees() as f32;
        // A teleport after our aim leaves the server with its rotation (no look is resent):
        // aim a little off until the server has ours.
        let (sent, pose) = (*ctx.sent(), ctx.pose());
        if sent.yaw != pose.yaw || sent.pitch != pose.pitch {
            pitch += 0.5;
        }
        let cur = pose.yaw;
        ctx.set_aim(cur + ghast::rotation::wrap_degrees(yaw - cur), pitch);
        ctx.set_controls(Controls {
            sneak: self.sneak,
            ..Controls::NONE
        });
        if self.trying.is_multiple_of(40) {
            log::debug!(
                "opening: target {:?}, entity {:?}, aim {yaw:.2} {pitch:.2}",
                ctx.target().map(|t| (t.pos, t.face)),
                ctx.target_entity().map(|e| e.id)
            );
        }
        let click = on && self.trying.is_multiple_of(10);
        if let Some(b) = self.block {
            if click && let Some(t) = ctx.target() {
                ctx.place(b, t.face, t.hit);
            }
        } else {
            ctx.set_buttons(Buttons {
                left: false,
                right: click,
            });
        }
        if self.trying > 400 {
            self.fail(ctx, format!("screen never opened (on target: {on})"));
        }
    }

    /// A screen that gave its items back on close (the server moves them; the client does
    /// not predict that): its slots are empty after the reopen and every item is still there.
    fn compare_returned(&mut self, now: &[Key], cursor: Key) {
        use std::collections::BTreeMap;
        let n = self.container_len;
        let mut bad = 0;
        for (i, s) in now.iter().enumerate().take(n) {
            if s.is_some() {
                bad += 1;
                self.failures
                    .push(format!("reopened: slot {i} not returned on close: {s:?}"));
            }
        }
        let count = |v: &[Key]| {
            let mut m: BTreeMap<(i32, i16, bool), i32> = BTreeMap::new();
            for (id, c, d, t) in v.iter().flatten() {
                *m.entry((*id, *d, *t)).or_default() += c;
            }
            m
        };
        let mut before = self.predicted.clone();
        if let Some(r) = self.result
            && let Some(slot) = before.get_mut(r)
        {
            *slot = None;
        }
        let (before, after) = (count(&before), count(now));
        if before != after || cursor.is_some() {
            bad += 1;
            self.failures.push(format!(
                "reopened: items before the close {before:?}, after {after:?}, cursor {cursor:?}"
            ));
        }
        println!("  reopened (items returned on close): {bad} differences");
    }

    fn compare(&mut self, what: &str, now: &[Key], cursor: Key) {
        let mut bad = 0;
        for (i, (p, s)) in self.predicted.iter().zip(now).enumerate() {
            if p != s && self.live.contains(&i) {
                println!("  {what}: live slot {i} {p:?} -> {s:?} (the server's tick)");
            } else if p != s {
                bad += 1;
                self.failures
                    .push(format!("{what}: slot {i} predicted {p:?}, server {s:?}"));
            }
        }
        if self.predicted.len() != now.len() {
            self.failures.push(format!(
                "{what}: {} slots predicted, {} now",
                self.predicted.len(),
                now.len()
            ));
        }
        if self.predicted_cursor != cursor {
            bad += 1;
            self.failures.push(format!(
                "{what}: cursor predicted {:?}, server {cursor:?}",
                self.predicted_cursor
            ));
        }
        println!("  {what}: {bad} differing slots");
    }
}

impl Behavior for Probe {
    fn name(&self) -> &str {
        "menu_probe"
    }

    fn interests(&self) -> Interests {
        // Mount screens read the mount entity (its type and chest) from the tracker.
        Interests::ENTITIES | Interests::CHAT
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        self.ticks += 1;
        for m in &ctx.events().chat {
            let line = m.plain();
            if line.contains("Grim") || line.contains("failed") {
                self.failures.push(format!("chat alert: {line}"));
            }
        }
        let ev = ctx.events().inventory.clone();
        if ev.opened.is_some() {
            self.opened = true;
            self.open_for = 0;
        }
        self.open_for += 1;
        if ev.closed {
            self.opened = false;
        }
        for c in &ev.clicks {
            match c.outcome {
                ClickOutcome::Accepted => {
                    self.accepted += 1;
                    if self.phase == Phase::Confirm {
                        self.phase = Phase::Settle(4);
                    }
                }
                o => {
                    let why = format!(
                        "step {} {:?}: action {} in window {}: {o:?}",
                        self.pc,
                        self.steps.get(self.pc),
                        c.action,
                        c.window_id
                    );
                    self.fail(ctx, why);
                    return;
                }
            }
        }
        if self.phase == Phase::Finished {
            return;
        }
        if self.ticks < 60 {
            return;
        }
        if self.wait > 0 {
            self.wait -= 1;
            return;
        }
        match self.phase {
            Phase::Finished => {}
            Phase::Open => {
                if self.opened {
                    ctx.set_buttons(Buttons::default());
                    ctx.set_controls(Controls::NONE);
                    if !ctx.inventory().resyncing() {
                        println!(
                            "opened {:?} ({} slots), trades {:?}",
                            ctx.inventory().window().map(|w| w.kind()),
                            ctx.inventory().container_len(),
                            trade_uses(ctx)
                        );
                        self.phase = Phase::Run;
                        self.wait = 5;
                    }
                } else {
                    self.open(ctx);
                }
            }
            Phase::Reopen(n) => {
                if n > 0 {
                    self.phase = Phase::Reopen(n - 1);
                } else if self.opened && self.open_for < 5 {
                    // The window's contents follow its open packet: give them a few ticks.
                    ctx.set_buttons(Buttons::default());
                    ctx.set_controls(Controls::NONE);
                } else if self.opened && !ctx.inventory().resyncing() {
                    self.wait = 5;
                    self.pc += 1;
                    let now = screen(ctx);
                    let cursor = key(ctx.inventory().cursor());
                    if self.returns {
                        self.compare_returned(&now, cursor);
                    } else {
                        self.compare("reopened", &now, cursor);
                    }
                    if let Some(p) = self.predicted_trades.take() {
                        let t = trade_uses(ctx);
                        println!("  trade uses predicted {p:?}, server {t:?}");
                        if t.as_ref() != Some(&p) {
                            self.failures
                                .push(format!("trade uses predicted {p:?}, server {t:?}"));
                        }
                    }
                    self.phase = Phase::Run;
                } else {
                    self.open(ctx);
                }
            }
            Phase::Confirm => {}
            Phase::Settle(n) => {
                if n > 0 {
                    self.phase = Phase::Settle(n - 1);
                    return;
                }
                let now = screen(ctx);
                let cursor = key(ctx.inventory().cursor());
                let what = format!("after step {} {:?}", self.pc, self.steps[self.pc]);
                self.compare(&what, &now, cursor);
                self.pc += 1;
                self.phase = Phase::Run;
            }
            Phase::Run => {
                let Some(step) = self.steps.get(self.pc).copied() else {
                    self.finish(ctx);
                    return;
                };
                let click = match step {
                    Step::Pickup(s, right) => Some(Click::Pickup {
                        slot: Self::slot(ctx, s),
                        right,
                    }),
                    Step::Quick(s) => Some(Click::QuickMove {
                        slot: Self::slot(ctx, s),
                        right: false,
                    }),
                    Step::Trade(i) => {
                        if let Err(e) = ctx.select_trade(i) {
                            self.fail(ctx, format!("select_trade({i}) refused: {e}"));
                            return;
                        }
                        println!("  select trade {i}");
                        // No confirm for this one: compare after a few ticks.
                        self.predicted = screen(ctx);
                        self.predicted_cursor = key(ctx.inventory().cursor());
                        self.phase = Phase::Settle(4);
                        None
                    }
                    Step::Button(i) => {
                        if let Err(e) = ctx.click_button(i) {
                            self.fail(ctx, format!("button {i} refused: {e}"));
                            return;
                        }
                        println!("  button {i}");
                        self.predicted = screen(ctx);
                        self.predicted_cursor = key(ctx.inventory().cursor());
                        self.phase = Phase::Settle(4);
                        None
                    }
                    Step::Wait(n) => {
                        self.wait = n;
                        self.pc += 1;
                        None
                    }
                    Step::Verify => {
                        if ctx.inventory().cursor().is_some() {
                            self.fail(ctx, "cursor not empty at a verify".into());
                            return;
                        }
                        self.predicted = screen(ctx);
                        self.predicted_cursor = None;
                        self.predicted_trades = trade_uses(ctx);
                        self.container_len = ctx.inventory().container_len();
                        if let Err(e) = ctx.close_window() {
                            self.fail(ctx, format!("close refused: {e}"));
                            return;
                        }
                        println!("  verify: closed, reopening");
                        self.opened = false;
                        self.trying = 0;
                        self.phase = Phase::Reopen(10);
                        None
                    }
                };
                if let Some(c) = click {
                    match ctx.click(c) {
                        Ok(_) => {
                            self.sent += 1;
                            println!("  click {c:?}");
                            self.predicted = screen(ctx);
                            self.predicted_cursor = key(ctx.inventory().cursor());
                            self.phase = Phase::Confirm;
                        }
                        Err(
                            ClickRefused::Turning
                            | ClickRefused::Inventory(
                                ClickError::Pending | ClickError::SameTick | ClickError::Resyncing,
                            ),
                        ) => {}
                        Err(e) => self.fail(ctx, format!("step {} {c:?} refused: {e}", self.pc)),
                    }
                }
            }
        }
    }
}

fn main() {
    let opts = support::Options::parse();
    support::init_logger(opts.verbose);
    let block = std::env::var("GHAST_MENU_BLOCK").ok().map(|s| {
        let v: Vec<i32> = s
            .split(',')
            .map(|x| x.trim().parse().expect("GHAST_MENU_BLOCK=x,y,z"))
            .collect();
        assert_eq!(v.len(), 3, "GHAST_MENU_BLOCK=x,y,z");
        BlockPos::new(v[0], v[1], v[2])
    });
    if block.is_none() && std::env::var("GHAST_MENU_ENTITY").is_err() {
        eprintln!("set GHAST_MENU_BLOCK=x,y,z or GHAST_MENU_ENTITY=1");
        std::process::exit(2);
    }
    let steps = match parse_steps(&std::env::var("GHAST_MENU_STEPS").unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("GHAST_MENU_STEPS: {e}");
            std::process::exit(2);
        }
    };
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
            block,
            aim_y: std::env::var("GHAST_MENU_AIM_Y")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.5),
            sneak: std::env::var("GHAST_MENU_SNEAK").is_ok_and(|v| v == "1"),
            steps,
            pc: 0,
            phase: Phase::Open,
            ticks: 0,
            wait: 0,
            opened: false,
            open_for: 0,
            trying: 0,
            predicted: Vec::new(),
            predicted_cursor: None,
            predicted_trades: None,
            returns: std::env::var("GHAST_MENU_RETURNS").is_ok_and(|v| v == "1"),
            live: std::env::var("GHAST_MENU_LIVE")
                .unwrap_or_default()
                .split(',')
                .filter_map(|x| x.trim().parse().ok())
                .collect(),
            container_len: 0,
            result: std::env::var("GHAST_MENU_RESULT")
                .ok()
                .and_then(|v| v.parse().ok()),
            sent: 0,
            accepted: 0,
            failures: Vec::new(),
        };
        let end = client.run(&mut probe).await;
        log::info!("session ended: {end:?}");
        let ok = probe.failures.is_empty() && probe.sent == probe.accepted && probe.sent > 0;
        std::process::exit(if ok { 0 } else { 1 });
    });
}
