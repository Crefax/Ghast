//! Dialog bot (26.1+): prints every dialog the server shows, in the configuration and the play
//! state, and answers it with one button and the given input values.
//!
//! ```text
//! cargo run -p ghast --release --example dialog -- --host localhost --port 25565 \
//!     --username DialogBot --version 26.2 [--button 0] [--value password=secret] [--confirm] \
//!     [--minutes 2]
//! ```
//!
//! `--button N` presses button N (default 0; `exit` / `close` for the exit button / Escape),
//! `--value key=text` types into a text input (repeatable), `--confirm` accepts vanilla's
//! command confirmation.

mod support;

use std::time::{Duration, Instant};

use ghast::dialog::{Dialog, DialogAnswer, DialogEvent, InputKind, InputValue};
use ghast::{Behavior, Interests, TickContext};

struct Answers {
    button: String,
    values: Vec<(String, String)>,
    confirm: bool,
    started: Instant,
    limit: Option<Duration>,
}

impl Answers {
    fn answer(&self, d: &Dialog) -> DialogAnswer {
        let mut a = match self.button.as_str() {
            "exit" => DialogAnswer::exit(),
            "close" => DialogAnswer::close(),
            n => DialogAnswer::button(n.parse().unwrap_or(0)),
        };
        for (k, v) in &self.values {
            if d.inputs
                .iter()
                .any(|i| i.key == *k && matches!(i.kind, InputKind::Text { .. }))
            {
                a = a.with_value(k.clone(), InputValue::Text(v.clone()));
            }
        }
        a.confirm_commands(self.confirm)
    }
}

fn show(state: &str, d: &Dialog) {
    log::info!(
        "dialog ({state}): {:?} \"{}\" after {:?}, escape {}",
        d.kind,
        d.title,
        d.after_action,
        d.can_close_with_escape
    );
    for b in &d.body {
        log::info!("  body: {b:?}");
    }
    for i in &d.inputs {
        log::info!("  input {:?} \"{}\": {:?}", i.key, i.label, i.kind);
    }
    for (n, b) in d.buttons.iter().enumerate() {
        log::info!("  button {n} \"{}\": {:?}", b.label, b.action);
    }
    if let Some(b) = &d.exit {
        log::info!("  exit \"{}\": {:?}", b.label, b.action);
    }
}

impl Behavior for Answers {
    fn name(&self) -> &str {
        "dialog"
    }

    fn interests(&self) -> Interests {
        Interests::DIALOGS | Interests::CHAT
    }

    fn configuration_dialog(&mut self, dialog: &Dialog) -> Option<DialogAnswer> {
        show("configuration", dialog);
        Some(self.answer(dialog))
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat {
            log::info!("chat: {}", m.plain());
        }
        let events = ctx.events().dialogs.clone();
        for ev in events {
            match ev {
                DialogEvent::Shown(d) => {
                    show("play", &d);
                    match ctx.answer_dialog(&self.answer(&d)) {
                        Ok(out) => log::info!(
                            "answered: sent {:?}, open {:?}, waiting {}",
                            out.sent,
                            out.open.as_ref().map(|d| &d.title),
                            out.waiting
                        ),
                        Err(e) => log::warn!("answer refused: {e}"),
                    }
                }
                DialogEvent::Cleared => log::info!("dialog cleared by the server"),
                _ => {}
            }
        }
        if self.limit.is_some_and(|l| self.started.elapsed() >= l) {
            ctx.disconnect("time limit reached");
        }
    }
}

fn main() {
    // Own options first; the rest are the shared ones.
    let mut rest = Vec::new();
    let mut button = "0".to_owned();
    let mut values = Vec::new();
    let mut confirm = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--button" => button = args.next().unwrap_or_default(),
            "--value" => {
                if let Some((k, v)) = args.next().as_deref().and_then(|s| s.split_once('=')) {
                    values.push((k.to_owned(), v.to_owned()));
                }
            }
            "--confirm" => confirm = true,
            _ => rest.push(a),
        }
    }
    let opts = support::Options::parse_from(rest);
    support::init_logger(opts.verbose);
    let client = match opts.client() {
        Ok(c) => c,
        Err(e) => {
            log::error!("{e}");
            std::process::exit(2);
        }
    };
    let mut bot = Answers {
        button,
        values,
        confirm,
        started: Instant::now(),
        limit: opts.limit(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(async {
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        match client.run_until(&mut bot, shutdown).await {
            Ok(e) => log::info!("session ended: {e:?}"),
            Err(e) => log::error!("session failed: {e}"),
        }
    });
}
