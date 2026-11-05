# Changelog

All crates share one version number.

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
