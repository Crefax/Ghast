# Changelog

All crates share one version number.

Versioning: until 1.0, a **patch** release (0.y.Z) adds features and fixes without breaking the
public API; a **minor** release (0.Y.0) may break it and says how to migrate. 1.0 comes only when
the API has been stable for a while and the 1.0 criteria in CONTRIBUTING.md are met.

## 0.3.0 — 2026-11-07

Minecraft 1.12.2 is the third supported version (the same bot runs on 1.8.9, 1.12.2 and
1.16.5), the path finder jumps gaps and solves mazes, and the public enums and tables that grow
with every Minecraft version are now open-ended, so that future versions (1.9 – 1.12.1, 1.13+,
1.21.x, ...) can be added in patch releases without breaking anyone.

### Breaking changes / migration

- **`#[non_exhaustive]` enums.** A `match` on one of these outside its crate needs a `_ =>`
  arm (building values is unchanged):
  - versions and protocol: `ghast_inventory::Version`, `ghast_protocol::State` (1.20.2 adds a
    configuration state); `GameVersion` and `ghast_pathfinder::Version` already were;
  - block shapes and data: `ghast_data::{ShapeDependency, NeighbourShape, DigMaterial,
    VariantApplied}`;
  - world: `ghast_world::{Biomes, BlockEntity, WorldError}`;
  - physics: `ghast_physics::Packet` (newer versions send movement input differently);
  - entities: `ghast_entity::{EntityType, EntityClass, MetaValue, Event}`;
  - inventory: `ghast_inventory::{SlotKind, ClickError, Outgoing}`;
  - path finding: `ghast_pathfinder::{Goal, MoveKind, NodeKind}`;
  - ext: `ghast_ext::{BlockMatch, ItemMatch}`;
  - viewer: `ghast_viewer::{tiles::Entry, protocol::TileKind, feed::Notice, replay::ReplayCmd}`;
  - auth errors: `ghast_auth::{AddrError, ConnectError, CryptoError, ProxyUrlError,
    Socks5Error}`, `ghast_auth::microsoft::{Stage, MicrosoftError, CacheError}`.
- **`#[non_exhaustive]` data records** (read their fields as before; they cannot be built with a
  struct literal or destructured without `..` outside `ghast-data`): `ghast_data::{BlockInfo,
  BlockPhysics, Collision, ItemInfo, EntityInfo, BiomeInfo, EnchantmentInfo, FoodInfo,
  EffectInfo}`.
- **New variants**: `GameVersion::V1_12_2`, `ghast_inventory::Version::V1_12`,
  `ghast_pathfinder::Version::V1_12`, `ghast_data::NeighbourShape::ChorusPlant`. The version
  enums are ordered oldest first, so `V1_16_5 as isize` changed; do not rely on such casts.
- **`GameVersion::ALL` is a slice** (`&'static [GameVersion]`, was `[GameVersion; 2]`):
  `for v in GameVersion::ALL` becomes `for &v in GameVersion::ALL`; `.iter()`, `.len()` and
  `.contains(..)` work as before. Its length changes with every added version.
- **Packet id lists are slices** (`&'static [i32]`, were fixed-size arrays):
  `ghast_world::v1_*::{PACKET_IDS, BLOCK_ENTITY_IDS}`, `ghast_entity::v1_*::{CORE_IDS,
  METADATA_IDS, EQUIPMENT_IDS, EFFECT_IDS, PLAYER_LIST_IDS}`. `filter.add(&CORE_IDS)` becomes
  `filter.add(CORE_IDS)`, `for id in PACKET_IDS` becomes `for &id in PACKET_IDS`.
- Behaviour: `GameVersion::V1_12_2` now plans with the 1.12 rules in `ghast-pathfinder` (it used
  1.16.5 blocks and physics); a search out of nodes now returns a partial path instead of
  `PlanError::NoPath` (see `Search::exhausted`).

### ghast-world
- 1.8: a ground-up `map_chunk` whose data is only the 256 biome bytes (sent by some servers
  for an empty column, with section 0 still in the mask) loads an empty column with those
  biomes, without an error. It used to replay vanilla's failing `fillChunk` (128 block states
  read from the biome bytes, `WorldError::Truncated`, a warning in the client log); dropping
  those client-side garbage states is a deliberate difference from vanilla, the server never
  has them. Real truncations stay errors with vanilla's column-kept semantics. 1.12.2 (256
  biome bytes) and 1.16.5 (no section data) handle the equivalent packet the same way. A
  ground-up chunk with an empty mask still unloads the column on 1.8.
- The client's "bad world packet" warning no longer claims the column was unloaded (1.8 keeps
  what vanilla keeps).

### Minecraft 1.12.2 (protocol 340)
A third supported version, end to end: the same bot runs unchanged on 1.8.9, 1.12.2 and 1.16.5
(`GameVersion::V1_12_2`, `--version 1.12.2` / `340`). Structured as the first member of a
1.9 – 1.12 family (shared chunk format, physics rules and pre-flattening block ids), so 1.9 –
1.12.1 can follow as small deltas.

- **ghast-protocol**: `v1_12_2` (feature `v1_12_2`), generated from minecraft-data; byte-exact
  against node-minecraft-protocol in both directions.
- **ghast-data**: `v1_12` (feature `v1_12`): blocks, items, entities, biomes, effects,
  enchantments, foods. Every collision box, per-state shape flag (normal cube, full cube, …),
  material, ray-test bound and 1.12 face shape (what fences, walls and panes connect to) comes
  from the vanilla 1.12.2 oracle. `NeighbourShape::ChorusPlant` is new.
- **ghast-world**: `v1_12::World`: 1.9 – 1.12 paletted sections (values spanning longs, the
  13-bit global palette), light, biomes, block entities, `unload_chunk`, block changes,
  explosions, world border.
- **ghast-physics**: `v1_12`: vanilla-exact 1.12.2 player movement, 0 diffs against the oracle
  (122 scenarios, the committed fuzz, and 4.16 M ticks of randomized fuzz). What changed since
  1.8.9 is in the crate README.
- **ghast-inventory**: `Version::V1_12`: vanilla 1.12.2 click prediction (offhand slot 45,
  shield shift-click, damage-value stacking, server-side crafting results), 0 diffs against
  the oracle's click vectors.
- **ghast-entity**: `v1_12::Tracker` with vanilla 1.12.2 position fixed-point rules, 1.9+
  metadata, passengers, one-slot equipment.
- **ghast-client**: `GameVersion::V1_12_2`, `GameVersion::combat_1_9()`; teleport confirms,
  1.9+ hands (swing after the click, off hand), attack cooldown, no idle on-ground packets.
- **ghast-ext**: block / item / entity names for 1.12.2.
- **oracle**: `MC=1.12.2 oracle/run.sh …` (vanilla 1.12.2 + Legacy Yarn 1.12.2+build.604,
  fetched at run time), golden vectors in `tests/golden/1.12.2/`.
- **lab**: `lab/start.sh 1.12` (Paper 1.12.2 build 1620 + GrimAC 2.3.73, Java 17, port 25612).

### ghast-pathfinder
- 1.12.2: `Version::V1_12` (`GameVersion::V1_12_2` maps to it; it used to plan with 1.16.5
  blocks and physics): block classes from the 1.12 data, move costs and the gap-jump table
  measured in the 1.12 physics, the whole validation suite run on it.
- Gap jumps (parkour) over 1–3 blocks on the level, onto a block one higher (also over 3) or
  one or two lower, and over 4 when landing two lower; from and onto 1×1 pillars, no run-up
  block needed. Each jump is offered only where the vanilla-exact physics lands it: a table
  measured on canonical terrain with the controller's exact key policy from 153 start states
  (`src/jump_table.rs`, regenerated by an ignored test), with every cell the body sweeps
  checked free in the world (and, up to the landing, nothing within 0.3 under the body and
  no soul sand / honey half a block under the feet, whose velocity multiplier 1.16 applies
  even in the air); before each jump the controller runs it once more in the physics from
  its actual state, jumps only if it lands, and then replays exactly those keys.
- Mazes: the default node budget is 250 000 (was 100 000), enough for all terrain a client
  loads at view distance 10; search records are half the size and the block cache faster
  (~25 % more nodes per second). A search out of nodes with nothing closer than the start
  returns a partial path to its most promising frontier node instead of `NoPath`.
- `Search::exhausted()`: whether the search expanded everything reachable (a partial result
  is then final for the loaded terrain).
- `Follower`: an unreachable goal (e.g. a flying player) ends after one search, at the block
  closest to it, with `NoPath` (no second full search). Partial-path replans count against
  `max_replans` unless they get closer to the goal, so a capped search cannot wander forever.
- Controller: brakes before a wall instead of sliding along it at corners of narrow
  corridors.

### ghast-ext
- `Bot::follow_path` / `follow_path_with`: follow an entity along planned paths (mazes, gaps,
  walls); `Bot::follow` stays the straight-line walker.

## 0.2.3 — 2026-11-06

Additive: one new crate, behind a facade feature (the default build is unchanged).

### ghast-viewer (new)
- Bot screen: a live 3D view of the bot's world in the browser (WebGL2: terrain, entities,
  players with skins and armour, particles, weather, the HUD) in first person, third person or
  a free camera. Read-only: it watches `after_tick` and never writes to the game connection.
- Costs nothing while nobody watches; per-bot HTTP / WebSocket server on loopback by default,
  optional token, idle stop.
- Recordings (`FeedRecorder`), a dash cam that saves the last minutes on demand (`DashCam`),
  and replays on the same page.
- Textures and models are built at run time from the vanilla client jar, downloaded from
  Mojang's servers into a git-ignored cache (feature `download`, on by default) or read from a
  local copy. No game assets are in the repository or the crate.
- Only 1.8.9 sessions are drawn for now.
- The built web client (`web/dist`) is committed, so `cargo build` needs no Node.

### ghast
- New feature `viewer`: re-exports `ghast-viewer` as `ghast::viewer`. Not enabled by default.

## 0.2.2 — 2026-11-06

Additive: two new crates, behind facade features (the default build is unchanged).

### ghast-pathfinder (new)
- A* over feet blocks with move costs measured in `ghast-physics` per version: walks (step-ups
  and drops up to 0.6: slabs, soul sand), diagonals, jumps up, drops (fall-damage free by
  default, deeper into water), 1- and 2-block gap jumps, swimming, ladders / vines (also
  onto a ladder from above), opt-in digging and hole filling. Unloaded blocks are never air:
  goals beyond them give partial paths.
- `Controller`: per-tick aim and keys with physics look-ahead (last-moment edge jumps,
  landing control, never stepping off the floor where the path does not go); `simulate`
  runs a path in the vanilla-exact physics, turning like the client; `plan_verified` plans
  around moves the simulation fails.
- `Follower` (feature `client`): plans, follows and replans from `Behavior::tick` through
  `TickContext` only.
- Validated on fake worlds for both versions (every move type, 96 fixed random terrains);
  1.8 lab run: 63 legs, 0 Grim flags. Planning: ~0.3 ms for 100 blocks of open ground,
  ~57 ms for a 193-step detour through rough terrain.
- Incremental planning: `Pathfinder::search` returns a resumable `Search` whose `step(world,
  Budget)` expands at most a node / time budget; `Simulation` runs `simulate` in slices. Both
  give exactly the one-shot results. The `Follower` plans and verifies across ticks within
  `FollowConfig::budget` (default 2000 nodes / 5 ms per tick) and `verify_ticks`, standing
  still meanwhile; a 193-step detour plans in 14 ticks of at most ~5 ms instead of one 57 ms
  tick. `Follower::is_planning`, `Follower::plan_ticks`.

### ghast-ext (new)
- `Ext`, a `Behavior` with a tick-driven executor: write bots as sequential async code
  (`Bot::with` runs a closure in the tick, before the movement packet). Helpers for looking,
  digging, placing, using items, attacking, equipping, window clicks, containers, block /
  entity search, chat, commands, events and walking; `goto` with ghast-pathfinder (feature
  `pathfinder`, default on).

### ghast-client
- `TickContext::item_facts(&Stack)`: the hands' version facts of any stack (block item,
  food, usable, tool speeds).
- `TickContext::dig_speed(pos, Option<&Stack>)`: the left button's per-tick break progress
  of a block with a given stack (tool, efficiency, haste / fatigue, water, on ground).

### ghast
- Features `ext` (`ghast::ext`) and `pathfinder` (`ghast::pathfinder`); example `goto`.

## 0.2.1 — 2026-11-05

Additive fixes: public types that downstream crates could not name are now exported.

### ghast-client
- Fix: `TickView` was not exported, so no crate outside ghast-client could implement
  `Behavior::after_tick`. It is now `ghast_client::TickView` (and `ghast::TickView`).
- Fix: `TeleportEvent` (the element type of `TickEvents::teleports`) is now exported.
- `TickView::dimension()` and `TickView::using_item()`, as on `TickContext`.

### ghast-world
- `Indexed` and `Direct`, the boxed section storages inside `Section`, are now exported
  (opaque types).

### All crates
- The `unnameable_types` lint is enabled (warn; CI denies warnings), so a public type that
  cannot be named from outside its crate fails the build.

## 0.2.0 — 2026-11-05

Breaking: public configuration and event structs are now `#[non_exhaustive]`; build the
configuration structs with `Default` + `with_*` builders (`WorldConfig`, `ghast_entity::Config`,
`ClientSettings`). Future fields will not be breaking changes.

### ghast-client
- Observers: `Behavior::after_tick(&TickView)` runs after every movement step with a read-only
  view (the pose sent this tick, world, entities, inventory, events).
- `WorldView::column(cx, cz)` returns a whole stored column; `ghast::world` is re-exported.
- `Client::world_config(..)` opts into light, biomes and block entities.
- `TickContext::swings()` counts every arm swing sent.
- `Interests::ENTITIES` / `PLAYER_LIST` can change mid-session; new `Interests::PLAYER_SKINS`.

### ghast-world
- Opt-in block-entity store (`WorldConfig::with_block_entities`): signs and tile entity data on
  1.8, chunk block entities and tile entity data on 1.16.5; `Column::block_entities()`.

### ghast-entity
- `PlayerInfo::textures` (skin property), opt-in via `Config::with_skins`.

## 0.1.3 — 2026-11-05

### ghast-client
- A malformed chunk packet no longer ends the session: the error is logged (rate-limited) and
  play continues, as the vanilla client does. `ClientError::World` is no longer returned.
- Opt-in `GHAST_DUMP_BAD_CHUNK=<dir>` writes the first rejected chunk bodies for bug reports.

### ghast-world
- A truncated 1.8 `map_chunk` now leaves the world as vanilla's chunk fill does: a ground-up
  chunk keeps a loaded column, blocks are filled up to the last complete pair, light and biome
  arrays only when whole. (Seen on a real server's login lobby: mask 1 with only the 256 biome
  bytes.)

## 0.1.2 — 2026-11-05

### ghast-client
- Fix: after `TickContext::close_window()` the client still treated the server-opened window
  as open, so held mouse buttons stayed blocked (no digging or placing) for the rest of the
  session. Closing a window on the client side now clears it, and the buttons follow the
  inventory's own screen state, as in vanilla.

## 0.1.1 — 2026-11-05

### ghast-client
- `Behavior::raw_packets`: receive selected inbound frames undecoded in `TickEvents::raw`
  (capped per tick, overflow counted in `raw_dropped`).
- `TickEvents::teleports`: every server position packet in arrival order, with the pose that
  was sent just before it and the resolved pose after it.
- `TickContext::last_inbound()` and `TickContext::planned_look()`.
- Fix: holding the left button on an instantly breakable block could send two `START_DIGGING`
  packets for the same position in one tick (flagged by Grim as AirLiquidBreak); the block is
  now predicted as air immediately, as vanilla does.

### All crates
- Version bump only.

## 0.1.0 — 2026-11-04

First release: 1.8.9 and 1.16.5 protocol, login (offline, Microsoft), SOCKS5, chunks,
entities, vanilla-exact player physics, window click and crafting prediction,
dig / place / use, and the fixed-tick bot API.
