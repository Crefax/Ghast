//! Shared helpers of the examples: command-line options and a tiny stderr logger.

#![allow(dead_code)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ghast::connect::{Connector, ProxyPolicy, Socks5Proxy, TargetAddr};
use ghast::{Account, Client, GameVersion};

/// Options shared by the examples.
pub struct Options {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub version: GameVersion,
    pub proxy: Option<Socks5Proxy>,
    pub policy: ProxyPolicy,
    pub minutes: Option<f64>,
    pub reconnect: bool,
    pub verbose: bool,
    pub seed: u64,
    pub sensitivity: Option<f32>,
    pub record: Option<String>,
    pub grim_compat: bool,
    /// `--grim-compat-up`: the narrower step-up-only rule.
    pub grim_compat_up: bool,
    /// `--view-distance`: the client's view distance setting.
    pub view_distance: Option<i8>,
}

pub const USAGE: &str = "options:
  --host <host>         server host (default localhost)
  --port <port>         server port (default 25565)
  --username <name>     offline username (default GhastBot)
  --version <v>         1.8.9 | 1.12.2 | 1.16.5 | 26.1 (default 1.8.9)
  --proxy <url>         socks5h://[user:pass@]host:port
  --policy <p>          except-local (default) | require | prefer | direct
  --minutes <m>         quit after m minutes
  --reconnect           reconnect after the session ends
  --seed <n>            random seed (examples that use one)
  --sensitivity <s>     mouse sensitivity 0..1 (default 0.5)
  --record <file>       record movement in the oracle replay format (1.8.9)
  --grim-compat         send the position on sub-0.03 step-ups (not vanilla)
  --view-distance <n>   client view distance in chunks (default 8)
  -v, --verbose         debug logging";

impl Options {
    /// Parses `std::env::args`; prints usage and exits on errors.
    pub fn parse() -> Options {
        match Self::try_parse(std::env::args().skip(1)) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("{e}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    fn try_parse(args: impl Iterator<Item = String>) -> Result<Options, String> {
        let mut o = Options {
            host: "localhost".into(),
            port: 25565,
            username: "GhastBot".into(),
            version: GameVersion::V1_8_9,
            proxy: None,
            policy: ProxyPolicy::RequireProxyExceptLocal,
            minutes: None,
            reconnect: false,
            verbose: false,
            seed: 1,
            sensitivity: None,
            record: None,
            grim_compat: false,
            grim_compat_up: false,
            view_distance: None,
        };
        let mut args = args.peekable();
        while let Some(a) = args.next() {
            let mut val = || args.next().ok_or_else(|| format!("{a} needs a value"));
            match a.as_str() {
                "--host" => o.host = val()?,
                "--port" => o.port = val()?.parse().map_err(|e| format!("--port: {e}"))?,
                "--username" => o.username = val()?,
                "--version" => o.version = val()?.parse().map_err(|e| format!("{e}"))?,
                "--proxy" => o.proxy = Some(val()?.parse().map_err(|e| format!("--proxy: {e}"))?),
                "--policy" => {
                    o.policy = match val()?.as_str() {
                        "except-local" => ProxyPolicy::RequireProxyExceptLocal,
                        "require" => ProxyPolicy::RequireProxy,
                        "prefer" => ProxyPolicy::PreferProxy,
                        "direct" => ProxyPolicy::custom(|_| ghast::connect::Route::Direct),
                        p => return Err(format!("unknown policy {p}")),
                    }
                }
                "--minutes" => {
                    o.minutes = Some(val()?.parse().map_err(|e| format!("--minutes: {e}"))?)
                }
                "--reconnect" => o.reconnect = true,
                "-v" | "--verbose" => o.verbose = true,
                "--record" => o.record = Some(val()?),
                "--grim-compat" => o.grim_compat = true,
                "--grim-compat-up" => o.grim_compat_up = true,
                "--seed" => o.seed = val()?.parse().map_err(|e| format!("--seed: {e}"))?,
                "--view-distance" => {
                    o.view_distance = Some(
                        val()?
                            .parse()
                            .map_err(|e| format!("--view-distance: {e}"))?,
                    );
                }
                "--sensitivity" => {
                    o.sensitivity =
                        Some(val()?.parse().map_err(|e| format!("--sensitivity: {e}"))?);
                }
                "-h" | "--help" => return Err("ghast example".into()),
                _ => return Err(format!("unknown option {a}")),
            }
        }
        Ok(o)
    }

    /// The client for these options.
    pub fn client(&self) -> Result<Client, Box<dyn std::error::Error>> {
        let address = TargetAddr::new(&self.host, self.port)?;
        let connector = Connector::new()
            .with_optional_proxy(self.proxy.clone())
            .with_policy(self.policy.clone());
        let mut c = Client::new(address, self.version, Account::offline(&self.username)?)
            .connector(connector);
        if let Some(s) = self.sensitivity {
            c = c.sensitivity(s);
        }
        if let Some(v) = self.view_distance {
            c = c.settings(ghast::ClientSettings::default().with_view_distance(v));
        }
        if let Some(r) = &self.record {
            c = c.record(r);
        }
        c = c.force_step_position(self.grim_compat);
        if self.grim_compat_up {
            c = c.force_step_position_mode(ghast::ForceStepPosition::StepUpOnly);
        }
        Ok(c)
    }

    /// The run-time limit, if any.
    pub fn limit(&self) -> Option<Duration> {
        self.minutes.map(|m| Duration::from_secs_f64(m * 60.0))
    }
}

/// Logs to stderr: `HH:MM:SS.mmm LEVEL message`.
struct Logger {
    level: log::LevelFilter,
}

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= self.level
    }

    fn log(&self, r: &log::Record<'_>) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let s = (ms / 1000) % 86_400;
        eprintln!(
            "{:02}:{:02}:{:02}.{:03} {:5} {}",
            s / 3600,
            (s / 60) % 60,
            s % 60,
            ms % 1000,
            r.level(),
            r.args()
        );
    }

    fn flush(&self) {}
}

/// Installs the logger (info, or debug with `verbose`).
pub fn init_logger(verbose: bool) {
    let level = if verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    let logger = Box::leak(Box::new(Logger { level }));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}
