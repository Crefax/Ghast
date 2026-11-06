# Changelog

All crates share one version number.

Versioning: until 1.0, a **patch** release (0.y.Z) adds features and fixes without breaking the
public API; a **minor** release (0.Y.0) may break it and says how to migrate. 1.0 comes only when
the API has been stable for a while and the 1.0 criteria in CONTRIBUTING.md are met.

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
