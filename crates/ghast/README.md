# ghast

A headless Minecraft Java Edition client / bot library in Rust (1.8.9 and 1.16.5), built for
running many light bots: packets nobody needs are dropped before they are decoded, the tick
loop mirrors vanilla's order exactly, and movement is checked against the real game.

This crate re-exports the bot API of `ghast-client` at the top level and the lower layers as
`ghast::protocol`, `ghast::auth` and `ghast::data`. Optional features: `ext` (`ghast::ext`,
sequential async helpers), `pathfinder` (`ghast::pathfinder`, physics-validated path finding;
with `ext` also `Bot::goto`), `viewer` (`ghast::viewer`, a bot screen in the browser with
recordings; textures are fetched from Mojang's client jar at run time), `microsoft`
(online-mode accounts).

```sh
cargo run -p ghast --release --example afk -- --host localhost --port 25565 --username AfkBot --version 1.8.9
cargo run -p ghast --example chat -- --host localhost --port 25565 --username ChatBot --version 1.16.5
```

Example options: `--proxy socks5h://user:pass@host:1080`, `--policy except-local|require|prefer|direct`
(default `except-local`: local addresses direct, everything else must use the proxy),
`--minutes N`, `--reconnect`, `-v`.

Licensed under MIT OR Apache-2.0.
