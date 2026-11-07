# ghast

A headless Minecraft: Java Edition client / bot library in Rust.

- **Old and popular versions first:** 1.8.9 (protocol 47), 1.12.2 (protocol 340) and 1.16.5
  (protocol 754), in one workspace with a version-agnostic bot API on top. A bot written once
  runs on all three.
- **Vanilla-exact physics, proven:** player movement is compared bit for bit with the real
  vanilla client code, run headless as an *oracle*. Not "close enough".
- **Light:** packets a bot does not ask for are dropped before they are decoded; one bot is one
  thread and a few MB of memory.

Status: early (0.2, on crates.io). APIs will still change; see CONTRIBUTING.md for the versioning policy.

## Numbers

| | ghast | |
|---|---|---|
| AFK bot on a Raspberry Pi 5, 300 s, side by side with a mineflayer 4.39 / Node 22 AFK bot on the same 1.8 server | **0.33 %** of one core, **4.1 MB** RSS, 1 thread | mineflayer: 2.19 %, 120 MB RSS, 7 threads |
| Player physics vs the vanilla oracle (randomized fuzz, 1.8.9, 1.12.2 and 1.16.5) | **0 diffs** over 9.32 million ticks | positions, velocities, flags and packets compared by their bits |
| Window clicks vs vanilla's client-side prediction | **0 diffs** over ~154,000 random clicks per version | incl. crafting results |
| Protocol codecs | byte-exact against node-minecraft-protocol | both directions, every packet of both versions round-trips |
| Lab sessions against Paper + [Grim](https://github.com/GrimAnticheat/Grim) anticheat (30 min mixed walk / sprint / jump / dig / place / clicks, per version) | **0 Grim flags** | see `crates/ghast-client/README.md` for the two documented Grim-vs-vanilla corner cases |

All measurements are dated and described with their method in the crate READMEs and
`lab/README.md`.

## Crates

| crate | what |
|---|---|
| [`ghast`](crates/ghast) | facade: re-exports the bot API and the lower layers (`cargo add ghast`) |
| [`ghast-client`](crates/ghast-client) | connection, fixed 50 ms tick loop, `Behavior` / `TickContext` bot API |
| [`ghast-protocol`](crates/ghast-protocol) | generated per-version packets, framing, compression, encryption, VarInt, NBT, chat |
| [`ghast-auth`](crates/ghast-auth) | offline identities, SOCKS5 (auth, remote DNS), login encryption, Microsoft accounts |
| [`ghast-data`](crates/ghast-data) | blocks, block states, collision shapes, items, entities (generated from minecraft-data) |
| [`ghast-world`](crates/ghast-world) | chunk storage, block lookup, explicit loaded / unloaded columns |
| [`ghast-physics`](crates/ghast-physics) | pure, deterministic player movement (no I/O, no clock) |
| [`ghast-inventory`](crates/ghast-inventory) | windows, vanilla click prediction, the window confirm protocol, lazy-NBT stacks |
| [`ghast-entity`](crates/ghast-entity) | entity tracking (opt-in), vanilla server positions, metadata |
| [`ghast-ext`](crates/ghast-ext) | sequential async helpers on the tick model: look, dig, place, equip, containers, chat, events, `goto` (facade feature `ext`) |
| [`ghast-pathfinder`](crates/ghast-pathfinder) | A* path planning with physics-measured costs, a path follower, every move checked in the vanilla-exact physics (facade feature `pathfinder`) |
| [`ghast-viewer`](crates/ghast-viewer) | bot screen: a live 3D view of the bot's world in the browser, recordings, dash cam, replays; textures are fetched from Mojang's client jar at run time (facade feature `viewer`) |

Dependencies point one way: the protocol crate knows nothing about the layers above it, so it
can be used on its own.

## Quick start

```sh
cargo run -p ghast --release --example afk -- \
    --host localhost --port 25565 --username AfkBot --version 1.8.9
# options: --proxy socks5h://user:pass@host:1080  --policy except-local|require|prefer|direct
#          --minutes N  --reconnect  -v
```

A behaviour is a type with a `tick`:

```rust,no_run
use ghast::{Account, Behavior, Client, GameVersion, Interests, TickContext};

struct Afk;

impl Behavior for Afk {
    fn interests(&self) -> Interests { Interests::CHAT }
    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat { println!("{}", m.plain()); }
        if ctx.is_dead() { ctx.respawn(); }
    }
}

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let client = Client::new("localhost:25565".parse()?, GameVersion::V1_8_9, Account::offline("Bot")?);
let end = client.run(&mut Afk).await?;
# Ok(()) }
```

Other examples in `crates/ghast/examples/`: `chat`, `dig_place`, `use_item`, `attack`,
`inventory_probe`, `movement_course`, `soak`, and `goto` (`--features ext,pathfinder`: walks
planned paths with the async helpers).

## Features per version

| | 1.8.9 | 1.12.2 | 1.16.5 |
|---|---|---|---|
| login: offline, online (Microsoft, feature `microsoft`), compression, encryption, SOCKS5 | yes | yes | yes |
| keep-alive, teleports, respawn, chat, vanilla join packets | yes | yes | yes |
| chunks and block changes | yes | yes | yes |
| player physics (walk, sprint, sneak, jump, fly, water / lava, ladders, cobwebs, soul sand, ice, slime, step-up, effects) | oracle-verified | oracle-verified (+ levitation, bed bounce, sneaking size) | oracle-verified |
| crosshair raytrace, dig / place / use items | oracle-verified rays | yes (oracle selection boxes; neighbour-dependent and stairs rays approximated) | yes (ray boxes approximated) |
| inventory and container clicks, crafting results | oracle-verified | oracle-verified (results come from the server, as in vanilla 1.12) | oracle-verified |
| entities: tracking, crosshair, attack / interact | yes | yes | yes |
| mouse rotation in whole mouse counts (sensitivity GCD) | yes | yes | yes |
| path finding and following (`ghast-pathfinder`) | lab-tested, 0 Grim flags | not yet | simulation-tested |
| bot screen in the browser, recordings and replays (`ghast-viewer`) | yes | not yet | not yet |
| movement recording / trace replay (`Client::record`) | yes | not yet | yes |

Planned: 1.21 / latest, one version at a time, each verified the same way; 1.9 – 1.12.1 as small
deltas of the 1.12.2 family on demand. Not
planned for now: a game client with its own window (`ghast-viewer` only draws the bot's view
in a browser page, read-only).

## How verification works

- **The oracle** (`oracle/`, Java) downloads the vanilla client jar from Mojang's servers and
  Yarn mappings at run time, loads the real movement / collision / click code headless on a
  fake world, and writes what it did as JSON-lines **golden vectors**.
- **The vectors are committed** (`tests/golden/<version>/`), so plain `cargo test` replays them
  against the Rust crates without Java or any jar. A mismatch prints the first differing tick
  and field.
- Big randomized runs (millions of ticks, tens of thousands of click sequences) are generated
  by the oracle on demand and replayed the same way.
- Protocol codecs are differential-tested against node-minecraft-protocol; tick and packet
  order are checked with packet-trace replays and live lab servers running Grim.

See [ARCHITECTURE.md](ARCHITECTURE.md) and [oracle/README.md](oracle/README.md).

## Legal

- This repository contains **no Mojang code or assets**: no jars, no decompiled source. The
  oracle downloads the vanilla jar from Mojang at run time into a git-ignored cache, as
  Fabric Loom does. The Rust crates are an independent implementation.
- Names in code and comments come from [Yarn](https://github.com/FabricMC/yarn) /
  [Legacy Yarn](https://github.com/Legacy-Fabric/yarn) (CC0), fetched at run time.
- Packet and game-data tables are generated from
  [PrismarineJS minecraft-data](https://github.com/PrismarineJS/minecraft-data) (MIT); see
  `crates/ghast-protocol/src/generated/NOTICE` and `crates/ghast-data/src/generated/NOTICE`.
- Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft.
  "Minecraft" is a trademark of Mojang Synergies AB.

Use bots only where the server's rules allow them.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above,
without any additional terms or conditions.
