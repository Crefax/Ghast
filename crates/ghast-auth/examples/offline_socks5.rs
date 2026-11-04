//! Offline identity + SOCKS5 connection.
//!
//! ```text
//! cargo run -p ghast-auth --example offline_socks5 -- <username> <host[:port]> [socks5h://user:pass@proxy:port]
//! ```
//!
//! Prints the offline UUID, then opens a TCP connection to the server (through the proxy when one
//! is given) under `ProxyPolicy::RequireProxyExceptLocal`: LAN/loopback servers may be reached
//! directly, anything else only through the proxy. The connection is closed right away; a real
//! client would now send the handshake packet.

use std::process::ExitCode;

use ghast_auth::connect::{Connector, DEFAULT_MINECRAFT_PORT, ProxyPolicy, TargetAddr};
use ghast_auth::offline::OfflineProfile;
use ghast_auth::socks5::Socks5Proxy;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: offline_socks5 <username> <host[:port]> [socks5h://user:pass@proxy:port]";
    let name = args.next().ok_or(usage)?;
    let server = args.next().ok_or(usage)?;
    let proxy = args.next().map(|p| p.parse::<Socks5Proxy>()).transpose()?;

    let profile = OfflineProfile::new(name)?;
    println!(
        "username {} -> offline UUID {}",
        profile.name(),
        profile.uuid()
    );

    let target = TargetAddr::parse_with_default_port(&server, DEFAULT_MINECRAFT_PORT)?;
    let connector = Connector::new()
        .with_optional_proxy(proxy)
        .with_policy(ProxyPolicy::RequireProxyExceptLocal);
    let via = match connector.plan(&target) {
        Some(true) => "through the proxy",
        Some(false) => "directly",
        None => "nowhere (a proxy is required for this target)",
    };
    println!("connecting to {target} {via}");

    let stream = connector.connect(&target).await?;
    println!(
        "connected: local {} -> peer {} (TCP_NODELAY = {})",
        stream.local_addr()?,
        stream.peer_addr()?,
        stream.nodelay()?
    );
    Ok(())
}
