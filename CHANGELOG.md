# Changelog

All crates share one version number.

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
