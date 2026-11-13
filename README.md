# ghast

A headless Minecraft: Java Edition client / bot library in Rust.

- **Old and popular versions first, current ones too:** 1.8.9 (protocol 47), 1.12.2 (protocol
  340), 1.16.5 (protocol 754) and 26.1 – 26.1.2 (protocol 775), in one workspace with a
  version-agnostic bot API on top. A bot written once runs on all of them.
- **Vanilla-exact physics, proven:** player movement is compared bit for bit with the real
  vanilla client code, run headless as an *oracle*. Not "close enough".
- **Light:** packets a bot does not ask for are dropped before they are decoded; one bot is one
  thread and a few MB of memory.

Status: early (0.4, on crates.io). APIs will still change; see CONTRIBUTING.md for the versioning policy.

## Numbers

| | ghast | |
|---|---|---|
| Player physics vs the vanilla oracle (randomized fuzz, 1.8.9, 1.12.2 and 1.16.5) | **0 diffs** over 12.72 million ticks | positions, velocities, flags and packets compared by their bits |
| Player physics vs the vanilla 26.1 client (randomized fuzz incl. edges, attributes, item use, teleports, elytra, riptide, entity pushes, nether, world border, boats, minecarts) | **0 diffs** over 11.20 million ticks | 26.1.1 and 26.1.2 give identical records |
| Window clicks vs vanilla's client-side prediction | **0 diffs** over ~154,000 random clicks per version (1.8.9 – 1.16.5) and 237,292 on 26.1 (every menu, byte-identical packets) | incl. crafting results |
| Protocol codecs | byte-exact against node-minecraft-protocol | both directions, every packet of every version round-trips |
| Lab sessions against Paper + [Grim](https://github.com/GrimAnticheat/Grim) anticheat (30 min mixed walk / sprint / jump / dig / place / clicks, per version) | **0 Grim flags** | see `crates/ghast-client/README.md` for the three documented Grim-vs-vanilla corner cases |

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

## Versions

| game versions | protocol | status |
|---|---|---|
| 1.8 – 1.8.9 | 47 | oracle-verified, lab-tested (Paper 1.8.8 + Grim) |
| 1.12.2 | 340 | oracle-verified, lab-tested (Paper 1.12.2 + Grim) |
| 1.16.4 – 1.16.5 | 754 | oracle-verified, lab-tested (Paper 1.16.5 + Grim) |
| 26.1, 26.1.1, 26.1.2 | 775 | oracle-verified (physics, crosshair, block breaking), lab-tested (Paper 26.1.2 + Grim 2.3.74) |
| 26.2 | 776 | oracle-verified (physics incl. collision restitution, crosshair, block breaking, clicks), lab-tested (Paper 26.2 + Grim 2.3.74) |

*Oracle-verified*: the behaviour is replayed against the real client code of that version
(26.1, 26.1.1 and 26.1.2 share one protocol number and give identical oracle records, so they
are one row). *Lab-tested*: sessions on a local Paper server with the Grim anticheat, 0 flags.
Name the exact version you connect with (`GameVersion`, `"26.1.2".parse()`); versions sharing a
protocol are served by the same code.

## Features per version

| | 1.8.9 | 1.12.2 | 1.16.5 | 26.1 and 26.2 |
|---|---|---|---|---|
| login: offline, online (Microsoft, feature `microsoft`), compression, encryption, SOCKS5 | yes | yes | yes | offline and compression lab-tested; online mode lab-tested against a local stand-in of the account services (not with a real account) |
| configuration state (known packs, registries, resource packs, cookies, reconfiguration) | – | – | – | yes (reconfiguration lab-tested through a Velocity proxy) |
| keep-alive, teleports, respawn, chat, vanilla join packets | yes | yes | yes | yes (signed chat and commands oracle-verified; server transfers) |
| chunks and block changes | yes | yes | yes | yes |
| player physics (walk, sprint, sneak, jump, fly, water / lava, ladders, cobwebs, soul sand, ice, slime, step-up, effects) | oracle-verified | oracle-verified (+ levitation, bed bounce, sneaking size) | oracle-verified | oracle-verified (+ attributes, powder snow, supporting block, elytra and rockets, riptide, entity pushes, boats, minecarts; living mounts not yet) |
| crosshair raytrace, dig / place / use items | oracle-verified rays | oracle-verified rays | oracle-verified rays | oracle-verified rays |
| block breaking time (per tick progress, finish tick) | oracle-verified | oracle-verified | oracle-verified | oracle-verified |
| inventory and container clicks, crafting results | oracle-verified | oracle-verified (results come from the server, as in vanilla 1.12) | oracle-verified | oracle-verified (every menu, creative screen, anvil / grindstone / loom / trades) |
| entities: tracking, crosshair, attack / interact | yes | yes | yes | yes (tracking oracle-verified; collisions and pushes with entity tracking on) |
| mouse rotation in whole mouse counts (sensitivity GCD) | yes | yes | yes | yes |
| path finding and following (`ghast-pathfinder`) | lab-tested, 0 Grim flags | lab-tested, 0 Grim flags | lab-tested, 0 Grim flags | lab-tested, 0 Grim flags |
| bot screen in the browser, recordings and replays (`ghast-viewer`) | yes | not yet | not yet | live view (1.8.9 assets, look-alikes for newer blocks); recordings untested |
| movement recording / trace replay (`Client::record`) | yes | not yet | yes | not yet |

Planned: more versions from 1.8 up, family by family, each verified the same way. Not
planned for now: a game client with its own window (`ghast-viewer` only draws the bot's view
in a browser page, read-only).

## Cargo features

`ghast` (and `ghast-client`) compile every version by default. Each version is a feature, and
each one builds alone, so a bot that only ever joins one server version can drop the rest
(less compile time, a smaller binary):

| feature | |
|---|---|
| `v1_8`, `v1_12_2`, `v1_16_5`, `v26_1`, `v26_2` | the game versions (default: all) |
| `libdeflate` | packet compression with libdeflate (C, faster; default). Without it: pure Rust (flate2 + zlib-rs) |
| `display-names` | English display names of blocks, items, entities, … in `ghast::data` (default). Registry names are always there; off saves ~120–200 KB |
| `microsoft` | online-mode servers (Microsoft accounts) |
| `ext`, `pathfinder`, `viewer` | the optional crates (see above) |

```toml
ghast = { version = "0.4", default-features = false, features = ["v1_8", "libdeflate"] }  # no display names
```

A `GameVersion` whose feature is off still exists; running it returns
`ClientError::VersionNotEnabled`. The `afk` example, stripped, release profile below
(2026-11-11): 5.3 MB with every version; 1.9 MB with 1.8.9 only, 1.9 MB 1.12.2, 2.6 MB 1.16.5,
3.3 MB 26.1. 2026-11-13 (0.4.2 work): 7.9 MB with every version, 4.2 MB 26.1 only, 4.0 MB 26.2
only (each modern version is its own compiled copy of the modern client).

### Recommended release profile

A library cannot set your build profile. For small, fast bots (what the numbers above use):

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
strip = true
```

**Many bots, one process each: build without PIE.** The generated data tables (block, item and
shape tables of every version) hold pointers; in a position-independent executable the dynamic
loader patches them at start-up, so every process gets a private copy (~0.9 MB). A non-PIE
build has nothing to patch: the tables stay clean, file-backed and shared between all processes
running the binary. It costs address-space randomisation of the main binary (ASLR of the
libraries and the stack stays).

```sh
RUSTFLAGS="-C relocation-model=static" cargo build --release --target x86_64-unknown-linux-gnu
```

Pass `--target` explicitly so the flag does not reach build scripts and proc macros; it works
the same for `aarch64-unknown-linux-gnu` (also through `lab/build-pi.sh` with
`PI_RUSTFLAGS="-C target-cpu=cortex-a76 -C relocation-model=static"`). Idle AFK bot, x86-64,
2026-11-12: private dirty memory 2.3 → 1.4 MB (26.1 and 1.8.9), RSS 7.3 → 6.1 MB (26.1) and
7.2 → 5.6 MB (1.8.9).

**Only the versions you use.** Code for every version and mechanic is resident in each process
(file-backed, shared between processes running the same binary). A single-version build is
smaller: idle AFK RSS 2026-11-12 with every version 8.0 MB on 26.1 / 7.0 MB on 1.8.9; 26.1 only
6.4 MB, 1.8.9 only 5.0 MB (private dirty 1.8 / 1.6 MB).

## Migrating from 0.3

0.4 breaks the API once, for 26.1 and the shared infrastructure (details in
[CHANGELOG.md](CHANGELOG.md#040--2026-11-11)):

1. **Features.** Every version is a feature (default: all). With `default-features = false`,
   add the versions you use (`v1_8`, `v1_12_2`, `v1_16_5`, `v26_1`) and `display-names` if you
   read English display names from `ghast::data`.
2. **Inventory stacks.** `Stack` fields are private: `stack.id` → `stack.id()`, `stack.count`
   → `stack.count()`, `stack.damage` → `stack.damage()`, `stack.nbt` → `stack.nbt()`; build with
   `Stack::new(id, count, damage).with_nbt(nbt)`.
3. **Fallible constructors.** `Inventory::new(version)` returns a `Result`;
   `Outgoing::to_v1_8()` returns an `Option` (like the other versions' encoders).
4. **Events and results.** New fields in `MovePacket`, `Teleport`, `Inbound::Respawn`; build
   `ChatMessage` with `from_json` / `from_text`. More enums and output structs are
   `#[non_exhaustive]` (e.g. `Click`, `ClickOutcome`, `Applied`, `Events`): add `_ =>` arms to
   matches and read the structs' fields instead of destructuring them fully.
   `hands::pick_entity` is gone: use `TickContext::target` / `target_entity` (one crosshair
   pick for block and entity).
5. **Protocol.** Packet types are aliases of shared definitions; code that names, builds or
   matches them is unchanged. `ghast_protocol::Version` is sealed (it cannot be implemented
   outside ghast-protocol).
6. **Behaviour.** Block-breaking time and crosshair picks now follow each version's real
   client exactly; break times changed for some blocks on 1.8.9, 1.12.2 and 1.16.5 (listed in
   the changelog).

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
  [Legacy Yarn](https://github.com/Legacy-Fabric/yarn) (CC0), fetched at run time. 26.x jars
  are published unobfuscated with Mojang's own names, which are used for those versions.
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
