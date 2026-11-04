# ghast-auth

Authentication and connection setup for the [ghast](https://github.com/Crefax/Ghast)
Minecraft: Java Edition client library — everything that happens before the game protocol
takes over a socket:

- **Offline identities**: `offline::offline_uuid` derives the UUID vanilla and Bukkit / Paper
  servers assign to an offline player.
- **SOCKS5** (RFC 1928 / RFC 1929): username / password authentication, remote DNS only
  (`socks5h://`), and a `connect::Connector` that opens the game connection directly or
  through a proxy under a `connect::ProxyPolicy` (for example "proxy required except for
  local addresses").
- **Login encryption**: shared secret, RSA PKCS#1 v1.5 with the server's key, the
  session-server hash.
- **Microsoft accounts** (feature `microsoft`, off by default): OAuth device-code flow, Xbox
  Live, XSTS, Minecraft services, a token cache, and the session-server `join` call.

```rust,no_run
use ghast_auth::connect::{Connector, ProxyPolicy, TargetAddr};
use ghast_auth::socks5::Socks5Proxy;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let proxy: Socks5Proxy = "socks5h://user:secret@proxy.example:1080".parse()?;
let connector = Connector::new()
    .with_proxy(proxy)
    .with_policy(ProxyPolicy::RequireProxyExceptLocal);
let target: TargetAddr = "mc.example.net:25565".parse()?;
let stream = connector.connect(&target).await?; // tokio TcpStream, ready for the handshake
# drop(stream); Ok(()) }
```

Examples: `cargo run -p ghast-auth --example offline_socks5`, `cargo run -p ghast-auth
--features microsoft --example microsoft_device_code`.

Secrets (`SecretString`: proxy passwords, tokens) are never printed by `Debug` / `Display`.

Licensed under MIT OR Apache-2.0.
