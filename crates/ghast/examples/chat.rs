//! Chat bot: prints the server's chat and sends every line typed on stdin.
//!
//! ```text
//! cargo run -p ghast --example chat -- --host localhost --port 25565 --username ChatBot --version 1.16.5
//! ```

mod support;

use std::sync::mpsc::{Receiver, TryRecvError, channel};

use ghast::{Behavior, Interests, TickContext};
use tokio::io::{AsyncBufReadExt, BufReader};

struct Chat {
    lines: Receiver<String>,
}

impl Behavior for Chat {
    fn name(&self) -> &str {
        "chat"
    }

    fn interests(&self) -> Interests {
        Interests::CHAT
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat {
            println!("{}", m.plain());
        }
        // At most one line per tick, like typing.
        match self.lines.try_recv() {
            Ok(line) if !line.trim().is_empty() => ctx.chat(line),
            Ok(_) | Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => ctx.disconnect("stdin closed"),
        }
        if ctx.is_dead() {
            ctx.respawn();
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
        let (tx, rx) = channel();
        tokio::spawn(async move {
            let mut stdin = BufReader::new(tokio::io::stdin()).lines();
            while let Ok(Some(line)) = stdin.next_line().await {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut chat = Chat { lines: rx };
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        match client.run_until(&mut chat, shutdown).await {
            Ok(e) => log::info!("session ended: {e:?}"),
            Err(e) => log::error!("session failed: {e}"),
        }
    });
}
