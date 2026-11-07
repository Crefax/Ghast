# Architecture

This document describes how ghast is put together and why. Crate-level details live in each
crate's README.

## Goals

1. **Old and popular versions first.** 1.8.9 first, then 1.16.5 and 1.12.2 (the first member
   of a 1.9 – 1.12 family); 1.21 / latest later. One repository, multi-version by design.
2. **Vanilla-exact behaviour, proven.** Movement, collisions, the crosshair and window clicks
   are checked bit for bit against the real vanilla code running as an oracle.
3. **Light.** CPU first (many bots on a small machine such as a Raspberry Pi 5), then
   robustness, then memory.

## Workspace layout

```
crates/
  ghast-protocol    packets (generated per version), framing, compression, encryption,
                    VarInt, NBT, chat components. No game logic.
  ghast-data        blocks, items, collision shapes, entities (generated from minecraft-data)
  ghast-auth        offline identities, Microsoft accounts, SOCKS5, login encryption
  ghast-world       chunk storage, block states, loaded / unloaded columns
  ghast-physics     pure and deterministic movement: (state, input, world view) -> new state
  ghast-inventory   windows, click prediction, the window confirm protocol, item stacks
  ghast-entity      entity tracking (opt-in)
  ghast-client      connection, fixed 50 ms tick loop, Behavior / TickContext bot API
  ghast             facade re-exporting the rest
codegen/            minecraft-data -> Rust generators (packets, game data); not published
oracle/             Java harness that runs the real vanilla code (see oracle/README.md)
tests/golden/       oracle output (golden vectors), committed
lab/                local test servers, measurement and cross-build scripts
```

One workspace, many crates, one version number for all of them (lockstep releases). Each
crate is published on its own, so a user can depend on only the protocol crate.
Dependencies point one way: the protocol crate knows nothing about the layers above it.

## Design rules

- **Physics is a pure function.** `ghast-physics` has no I/O, no async and no clock. The oracle
  tests, trace replays and look-ahead planning all call the same code.
- **Async I/O, sync game logic.** The network side runs on tokio; each bot has its own
  deterministic tick loop in fixed 50 ms steps. The engine in `ghast-client` (state, behaviour,
  tick) does no I/O and can be driven by tests directly.
- **Two-layer versioning.** At the bottom, version-specific packet types are generated per
  version. On top, a version-agnostic "common" layer of inbound events and outbound actions.
  The bot API only sees the common layer, so one behaviour runs on 1.8.9, 1.12.2 and 1.16.5.
  Version dispatch happens once per session and is monomorphised: version differences are
  checked at compile time.
- **Decode only what is needed.** The play-state packet filter is built from the core's needs
  plus the behaviour's `Interests`. Everything else (tab list, teams, particles, sounds,
  entities unless asked for, …) is dropped by the frame decoder before it is decoded; item NBT
  is parsed lazily.
- **The process model is the application's choice.** The library works with many bots in one
  process (the session future is `Send`) and with one bot per process on a `current_thread`
  runtime, where bots can never crash together. A panicking behaviour stops only itself.
- **Never treat unloaded chunks as air** in behaviour-facing APIs (`None` = not loaded). Only
  the physics' world view maps unloaded columns the way the vanilla client does.

## Tick model

The tick mirrors 1.8.9 `MinecraftClient.tick` (and the corresponding 1.16.5 order), verified with
packet traces:

1. Inbound packets are applied on arrival (keep-alives, teleports and window confirms are
   answered at once).
2. `Behavior::tick(ctx)`. **Actions** (`select_slot`, `place`, `dig`, `attack`, clicks, chat,
   swing) are written immediately, in call order, **before** this tick's movement packet.
   `set_aim`, `set_controls`, `set_buttons` and `set_flying` only record intent.
3. Held-slot sync → key processing (held buttons) → abilities → rotation step (whole mouse
   counts, sensitivity GCD, Java `float` maths) → physics step → sprint / sneak entity actions
   → **exactly one** movement packet.

So the server judges a tick's actions with the **previous** tick's sent pose. A behaviour aims
first and acts on a later tick; `ctx.sent()` exposes the pose the server has, and the crosshair
checks for `dig` / `place` use it.

Other rules the client enforces:
- Window clicks only while the look is still, one click in flight at a time; every window
  confirm is answered exactly once.
- Silent-server guard: after 1.5 s without inbound data the behaviour is frozen (keys
  released) while the vanilla movement packet continues.

## The oracle

`oracle/` is a small Java program that loads the real vanilla client classes headless with a
fake world whose blocks come from the test case, runs player movement (and collision boxes,
raytraces, window clicks, entity sizes), and writes JSON-lines golden vectors. The Rust tests
replay those vectors and print the first differing tick and field on a mismatch.

- The vanilla jar is downloaded at run time from Mojang's servers (SHA-1 checked) and remapped
  to Yarn names in a git-ignored cache. Nothing from Mojang enters the repository.
- Golden vectors are plain numbers and are committed, so `cargo test` needs no Java.
- Randomized fuzz runs (millions of ticks) are generated on demand and replayed by
  feature-gated tests.
- Live anticheats such as Grim mirror vanilla, so vanilla-exact is the right target. Where a
  specific anticheat version disagrees with vanilla, the deviation is opt-in and documented
  (`Client::force_step_position`, `crates/ghast-physics/GRIM_NOTES.md`).

## Multi-version plan

- **Protocol:** generated from PrismarineJS minecraft-data `protocol.json` per version; a new
  version is mostly new data plus regeneration.
- **World and data formats** have known breaks that need adapter layers: the 1.13 flattening
  (block ids), the 1.18 world height, the 1.20.2 configuration state between login and play,
  1.20.5 item components instead of NBT.
- **Physics** changes in 1.9 (combat and movement details), 1.13–1.14 (swimming, sneaking) and
  small ones in 1.21. The oracle works for every version (download that version's jar).
- Versions are added one at a time, each verified the same way. The latest version changes
  every few months; supporting it is ongoing work, so no more versions are promised than are
  tested.

## Rules for contributors

See [CONTRIBUTING.md](CONTRIBUTING.md). In short: no Mojang code or files, our own
implementation (read vanilla to understand the behaviour, then write it in our own
structure), Yarn names, and no private data in traces or fixtures.
