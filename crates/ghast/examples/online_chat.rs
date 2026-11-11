//! Online-mode lab bot: logs in to an online-mode server through a session service at a base
//! URL (a local stand-in such as `lab/fake-session-server.py`; never real credentials), then
//! chats, runs signable commands and walks. With signed chat on (the default) it fetches a
//! profile key pair from that service and signs every message and command argument.
//!
//! ```text
//! cargo run -p ghast --release --features microsoft --example online_chat -- \
//!     --host 127.0.0.1 --port 25627 --username GhastBot --version 26.1 \
//!     --session-base http://127.0.0.1:25700 [--unsigned] [--minutes 1]
//! ```
//!
//! The access token is `fake:<profile id>` and the profile id is derived from the name (what the
//! stand-in accepts). Prints every chat line it receives.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use ghast::auth::SecretString;
use ghast::auth::microsoft::{
    Endpoints, MicrosoftAuth, MicrosoftConfig, MinecraftProfile, MinecraftSession,
};
use ghast::{Account, Behavior, Client, Controls, Interests, TickContext};

struct Online {
    started: Instant,
    limit: Duration,
    joined_at: Option<u64>,
    step: usize,
    name: String,
}

const SAY: &[&str] = &[
    "hello from a signed chat session",
    "/say signed say command",
    "/me waves (signed)",
    "/msg {name} signed whisper to myself",
    "second plain chat line",
];

impl Behavior for Online {
    fn name(&self) -> &str {
        "online_chat"
    }

    fn interests(&self) -> Interests {
        Interests::CHAT
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat {
            log::info!("chat: {}", m.plain());
        }
        if ctx.events().joined || self.joined_at.is_none() {
            self.joined_at.get_or_insert(ctx.tick());
        }
        let since = ctx.tick() - self.joined_at.unwrap_or(0);
        // Walk back and forth between the messages.
        let t = since / 30;
        ctx.set_aim(90.0, 0.0);
        ctx.set_controls(Controls {
            forward: t.is_multiple_of(2),
            back: !t.is_multiple_of(2),
            ..Controls::NONE
        });
        if since >= 80 && since.is_multiple_of(40) && self.step < SAY.len() {
            let line = SAY[self.step].replace("{name}", &self.name);
            log::info!("sending: {line}");
            ctx.chat(line);
            self.step += 1;
        }
        if self.started.elapsed() >= self.limit {
            ctx.disconnect("time limit reached");
        }
    }
}

fn main() {
    let mut base = None;
    let mut signed = true;
    let mut rest = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--session-base" => base = args.next(),
            "--unsigned" => signed = false,
            _ => rest.push(a),
        }
    }
    let base = base.expect("--session-base <url>");
    let opts = support::Options::parse_from(rest);
    support::init_logger(opts.verbose);
    // The profile id: a name-based UUID like the offline one, of "lab:<name>" (so it differs
    // from the offline id of the same name).
    let id = ghast::auth::offline::offline_uuid(&format!("lab:{}", opts.username));
    let auth = MicrosoftAuth::new(
        MicrosoftConfig::live("ghast-lab").with_endpoints(Endpoints::with_base_url(&base)),
    )
    .expect("http client");
    let session = MinecraftSession {
        access_token: SecretString::new(format!("fake:{}", id.simple())),
        expires_at: SystemTime::now() + Duration::from_secs(3600),
        profile: MinecraftProfile {
            id,
            name: opts.username.clone(),
        },
    };
    let account = Account::Microsoft {
        session,
        auth: Arc::new(auth),
    };
    let address = ghast::connect::TargetAddr::new(&opts.host, opts.port).expect("address");
    let client = Client::new(address, opts.version, account)
        .connector(ghast::connect::Connector::new().with_policy(opts.policy.clone()))
        .signed_chat(signed);
    let mut bot = Online {
        started: Instant::now(),
        limit: opts.limit().unwrap_or(Duration::from_secs(40)),
        joined_at: None,
        step: 0,
        name: opts.username.clone(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let ok = rt.block_on(async {
        match client.run(&mut bot).await {
            Ok(e) => {
                log::info!("session ended: {e:?}");
                matches!(e, ghast::SessionEnd::Quit(_))
            }
            Err(e) => {
                log::error!("session failed: {e}");
                false
            }
        }
    });
    std::process::exit(if ok { 0 } else { 1 });
}
