//! 26.1 creative inventory screen check: in creative mode (give the bot a few stacks first,
//! e.g. `lab/console.sh 26.1 "gamemode creative GhastBot9" "give GhastBot9 stone 64"`), opens
//! the inventory and clones a stack (middle click), places it, drops one item (Q), drops a
//! picked-up stack by clicking outside, drops a whole stack (ctrl+Q), closes, and counts the server's window 0 corrections (`container_set_slot` / `container_set_content`
//! for window 0 after the first click). Prints `PASS` with 0 corrections.
//!
//! ```text
//! cargo run -p ghast --release --example creative_probe -- --host 127.0.0.1 --port 25626 \
//!     --username GhastBot9 --version 26.1
//! ```

mod support;

use std::borrow::Cow;

use ghast::inventory::Click;
use ghast::protocol::v26_1::play::clientbound as cb;
use ghast::{Behavior, GameVersion, Interests, TickContext};

struct Probe {
    step: usize,
    wait: u32,
    clicking: bool,
    corrections: usize,
    sent: usize,
    refused: usize,
    done_at: Option<u64>,
}

impl Probe {
    fn steps(&self) -> &'static [(&'static str, Click)] {
        &[
            ("clone 36 (middle click)", Click::Clone { slot: 36 }),
            (
                "place into 40",
                Click::Pickup {
                    slot: 40,
                    right: false,
                },
            ),
            (
                "drop one of 36 (Q)",
                Click::Drop {
                    slot: 36,
                    all: false,
                },
            ),
            (
                "pick up 37",
                Click::Pickup {
                    slot: 37,
                    right: false,
                },
            ),
            (
                "drop it outside",
                Click::Pickup {
                    slot: ghast::inventory::OUTSIDE,
                    right: false,
                },
            ),
            (
                "drop stack 40 (ctrl+Q)",
                Click::Drop {
                    slot: 40,
                    all: true,
                },
            ),
        ]
    }
}

impl Behavior for Probe {
    fn name(&self) -> &str {
        "creative_probe"
    }

    fn interests(&self) -> Interests {
        Interests::CHAT
    }

    fn raw_packets(&self, _version: GameVersion) -> Cow<'static, [i32]> {
        Cow::Borrowed(&[cb::id::SET_SLOT, cb::id::WINDOW_ITEMS])
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for r in &ctx.events().raw {
            // window id 0 (varint 0) first: a window 0 correction after we started clicking.
            if self.clicking && r.body.first() == Some(&0) {
                self.corrections += 1;
                log::info!("correction: packet 0x{:02x} ({} B)", r.id, r.body.len());
            }
        }
        if let Some(t) = self.done_at {
            if ctx.tick() > t + 60 {
                let pass = self.corrections == 0 && self.refused == 0 && self.sent > 0;
                log::info!(
                    "{}: {} clicks sent, {} refused, {} window 0 corrections",
                    if pass { "PASS" } else { "FAIL" },
                    self.sent,
                    self.refused,
                    self.corrections
                );
                ctx.disconnect("probe done");
            }
            return;
        }
        if ctx.tick() < 100 {
            return; // level load, the console's give
        }
        if self.wait > 0 {
            self.wait -= 1;
            return;
        }
        if self.step == 0 && !ctx.inventory().inventory_open() {
            if let Err(e) = ctx.open_inventory() {
                log::warn!("open inventory: {e:?}");
            }
            self.wait = 5;
            return;
        }
        let steps = self.steps();
        if self.step < steps.len() {
            let (what, click) = steps[self.step].clone();
            self.clicking = true;
            match ctx.click(click) {
                Ok(_) => {
                    self.sent += 1;
                    log::info!("click: {what}");
                }
                Err(e) => {
                    self.refused += 1;
                    log::warn!("click {what} refused: {e:?}");
                }
            }
            self.step += 1;
            self.wait = 10;
            return;
        }
        let _ = ctx.close_window();
        self.done_at = Some(ctx.tick());
    }
}

fn main() {
    let opts = support::Options::parse();
    support::init_logger(opts.verbose);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let ok = rt.block_on(async {
        let client = opts.client().expect("client");
        let mut p = Probe {
            step: 0,
            wait: 0,
            clicking: false,
            corrections: 0,
            sent: 0,
            refused: 0,
            done_at: None,
        };
        match client.run(&mut p).await {
            Ok(e) => log::info!("session ended: {e:?}"),
            Err(e) => log::error!("session failed: {e}"),
        }
        p.corrections == 0 && p.refused == 0 && p.sent > 0
    });
    std::process::exit(if ok { 0 } else { 1 });
}
