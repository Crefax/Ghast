//! Live inventory check through the full client: opens a chest by right-clicking it, sends
//! clicks of every mode in the chest and in the player inventory, then reopens the chest and
//! compares the server's resent slots with ghast-inventory's prediction.
//!
//! ```text
//! lab/console.sh 1.8 'setblock 0 4 2 chest 2 replace {Items:[...]}'   # see ghast-inventory's README
//! cargo run -p ghast --example inventory_probe -- --host 127.0.0.1 --port 25618 \
//!     --username GhastBot3 --version 1.8.9
//! ```
//! Prints `PASS` when every click was accepted and the server's contents equal the prediction.

mod support;

use ghast::inventory::{Click, ClickError, ClickOutcome, OUTSIDE, Stack};
use ghast::{Behavior, BlockPos, ClickRefused, Face, TickContext};

const CHEST: BlockPos = BlockPos::new(0, 4, 2);
/// Pitch looking from (0.5, 5.62, 0.5) at the chest's north face: 245 mouse counts at
/// sensitivity 0.5.
const PITCH: f32 = 36.75;

#[derive(Debug, Clone, Copy)]
enum Step {
    Wait(u32),
    Aim,
    UseChest,
    WaitWindow,
    OpenInv,
    Close,
    Click(Click),
    Snapshot,
    Verify { player_only: bool },
    Done,
}

fn script() -> Vec<Step> {
    use Step::*;
    use ghast::inventory::Click::*;
    vec![
        Wait(40),
        Aim,
        Wait(3),
        UseChest,
        WaitWindow,
        Wait(3),
        Step::Click(Pickup {
            slot: 0,
            right: false,
        }),
        Step::Click(Pickup {
            slot: 30,
            right: false,
        }),
        Step::Click(Pickup {
            slot: 1,
            right: true,
        }),
        Step::Click(Pickup {
            slot: OUTSIDE,
            right: true,
        }),
        Step::Click(Pickup {
            slot: 1,
            right: false,
        }),
        Step::Click(QuickMove {
            slot: 2,
            right: false,
        }),
        Step::Click(QuickMove {
            slot: 30,
            right: false,
        }),
        Step::Click(Swap { slot: 3, hotbar: 0 }),
        Step::Click(Swap { slot: 0, hotbar: 1 }),
        Step::Click(Drop {
            slot: 1,
            all: false,
        }),
        Step::Click(Pickup {
            slot: 54,
            right: false,
        }),
        Step::Click(PickupAll { slot: 54 }),
        Step::Click(Pickup {
            slot: 54,
            right: false,
        }),
        Step::Click(QuickMove {
            slot: 54,
            right: true,
        }),
        Step::Click(Drop {
            slot: OUTSIDE,
            all: false,
        }),
        Snapshot,
        Close,
        Wait(5),
        UseChest,
        WaitWindow,
        Wait(3),
        Verify { player_only: false },
        Close,
        Wait(5),
        OpenInv,
        Wait(2),
        Step::Click(QuickMove {
            slot: 38,
            right: false,
        }),
        Step::Click(QuickMove {
            slot: 39,
            right: false,
        }),
        Step::Click(Pickup {
            slot: 9,
            right: false,
        }),
        Step::Click(Pickup {
            slot: 1,
            right: true,
        }),
        Step::Click(Pickup {
            slot: 2,
            right: true,
        }),
        Step::Click(Pickup {
            slot: 1,
            right: false,
        }),
        Step::Click(Pickup {
            slot: 1,
            right: false,
        }),
        Step::Click(Pickup {
            slot: 2,
            right: false,
        }),
        Step::Click(QuickMove {
            slot: 2,
            right: false,
        }),
        Step::Click(Swap {
            slot: 44,
            hotbar: 6,
        }),
        Step::Click(QuickMove {
            slot: 5,
            right: false,
        }),
        Step::Click(Drop {
            slot: 42,
            all: false,
        }),
        Step::Click(Pickup {
            slot: 42,
            right: false,
        }),
        Step::Click(PickupAll { slot: 42 }),
        Step::Click(Pickup {
            slot: 15,
            right: false,
        }),
        Snapshot,
        Close,
        Wait(5),
        UseChest,
        WaitWindow,
        Wait(3),
        Verify { player_only: true },
        Close,
        Wait(10),
        Done,
    ]
}

struct Probe {
    script: Vec<Step>,
    pc: usize,
    wait: u32,
    /// A click was sent and its confirm is outstanding.
    awaiting: bool,
    opened: bool,
    snapshot: Vec<Option<Stack>>,
    sent: u32,
    accepted: u32,
    failures: Vec<String>,
}

fn screen(ctx: &TickContext<'_>) -> Vec<Option<Stack>> {
    let inv = ctx.inventory();
    (0..inv.slot_count())
        .map(|i| inv.slot(i).cloned())
        .collect()
}

fn key(s: &Option<Stack>) -> Option<(i32, i32, i16, bool)> {
    s.as_ref()
        .map(|s| (s.id(), s.count(), s.damage(), s.nbt().is_some()))
}

impl Probe {
    fn finish(&mut self, ctx: &mut TickContext<'_>) {
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
        ctx.disconnect("probe done");
    }
}

impl Behavior for Probe {
    fn name(&self) -> &str {
        "inventory_probe"
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
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
        if self.wait > 0 {
            self.wait -= 1;
            return;
        }
        let Some(step) = self.script.get(self.pc).copied() else {
            return;
        };
        let mut advance = true;
        match step {
            Step::Wait(n) => self.wait = n,
            Step::Aim => {
                if (ctx.sent().pitch - PITCH).abs() > 0.01 || ctx.sent().yaw.abs() > 0.01 {
                    ctx.set_aim(0.0, PITCH);
                    advance = false;
                }
            }
            Step::UseChest => {
                self.opened = false;
                match ctx.target() {
                    Some(t) if t.pos == CHEST && t.face == Face::North => {
                        ctx.place(CHEST, Face::North, t.hit);
                    }
                    t => {
                        self.failures
                            .push(format!("not looking at the chest: {t:?}"));
                        self.finish(ctx);
                        self.pc = self.script.len();
                        return;
                    }
                }
            }
            Step::WaitWindow => advance = self.opened && !ctx.inventory().resyncing(),
            Step::OpenInv => advance = ctx.open_inventory().is_ok(),
            Step::Close => {
                advance = !self.awaiting && ctx.close_window().is_ok();
            }
            Step::Click(c) => {
                if self.awaiting {
                    advance = false;
                } else {
                    match ctx.click(c) {
                        Ok(_) => {
                            self.sent += 1;
                            self.awaiting = true;
                            println!("  click {c:?}");
                        }
                        Err(
                            ClickRefused::Turning
                            | ClickRefused::Inventory(
                                ClickError::Pending | ClickError::SameTick | ClickError::Resyncing,
                            ),
                        ) => advance = false,
                        Err(e) => self.failures.push(format!("{c:?} refused: {e}")),
                    }
                }
            }
            Step::Snapshot => {
                if self.awaiting {
                    advance = false;
                } else {
                    self.snapshot = screen(ctx);
                    if ctx.inventory().cursor().is_some() {
                        self.failures.push("cursor not empty at snapshot".into());
                    }
                }
            }
            Step::Verify { player_only } => {
                let now = screen(ctx);
                let n = ctx.inventory().container_len();
                let pairs: Vec<(usize, usize)> = if player_only {
                    (0..36).map(|i| (9 + i, n + i)).collect()
                } else {
                    (0..now.len()).map(|i| (i, i)).collect()
                };
                let mut bad = 0;
                for (a, b) in pairs {
                    let x = key(self.snapshot.get(a).unwrap_or(&None));
                    let y = key(now.get(b).unwrap_or(&None));
                    if x != y {
                        bad += 1;
                        self.failures
                            .push(format!("slot {b}: predicted {x:?}, server {y:?}"));
                    }
                }
                println!("  verify: {bad} differing slots");
            }
            Step::Done => {
                self.finish(ctx);
            }
        }
        if advance {
            self.pc += 1;
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
        let mut probe = Probe {
            script: script(),
            pc: 0,
            wait: 0,
            awaiting: false,
            opened: false,
            snapshot: Vec::new(),
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
