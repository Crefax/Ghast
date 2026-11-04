# Behaviour API reference

ghast-client exposes a tick-driven behaviour contract (`Behavior` + `TickContext`). The same
contract is used by a reference TypeScript bot built on mineflayer, so behaviours written
against it can be ported member by member (see `PORTING.md`). This file lists every member of
that contract (camelCase names below are the TypeScript side), its ghast equivalent, and the
trace replay that checks the packets.

Status: **done** (same meaning), **different** (same capability, other shape — the port is
mechanical), **app** (deliberately not in the library: application or runner specific).

## Tick model

| contract | ghast | status |
|---|---|---|
| inbound applied on arrival; S32 not accepted → C0F at once; S08 → C06 at once | same (`Engine::handle`, ghast-inventory confirms) | done |
| `behaviour.start` on the first ready tick, `tick` while `running` | `start` then `tick` on the first ready tick; `tick` while `running()` | different: the reference calls only `start` on the first tick |
| actions written immediately in call order (C09 C08 C0A …) | same | done |
| hands tick (held buttons) → C09 sync → C13 → rotation → physics → C0B sprint → C0B sneak → one C03–C06 | C09 sync → hands → C13 → rotation → physics → … | done (the sync moved before the key processing like vanilla's `ClientPlayerInteractionManager.tick`; the hands' own clicks sync first in both, so the wire order is the same) |
| rotation held when the post-look would leave a block dug / placed this tick (Grim RotationPlace) | same check (`Client::hold_turn_on_action`, on by default) | done |
| silent server > 1.5 s: behaviour frozen, keys released, vanilla movement continues | same (`Client::silent_hold`) | done |
| player column not loaded / no position yet: no tick | same | done |

## `TickContext`

| contract | ghast `TickContext` | status |
|---|---|---|
| `tick` | `tick()` | done |
| `pose` (x y z, yaw pitch, onGround, flying, sneaking, motion) | `pose()` → `Pose { pos, motion, yaw, pitch, on_ground, flying, sneaking, sprinting }` | done |
| `sent` | `sent()` | done |
| `world.block(x, y, z)` → `{id, meta}` or null when unloaded | `world().raw_block(x, y, z)` → `Option<u32>` (1.8 `id << 4 \| meta`; `None` = not loaded); `world().block_query(..)` for physics / tool facts | different (raw state) |
| `world.items()` (dropped items) | `entities()` (with `Interests::ENTITIES`): `iter()` / `near(pos, r)` filtered by `class() == EntityClass::Item`, `item()` | different |
| `world.players()` | `entities()`: `is_player()`, `player_name(id)` (names need `Interests::PLAYER_LIST`) | different |
| `inventory.slot(i)` (window 0) | `inventory().window0_slot(i)`; `player(k)`, `hotbar(k)`, `held()` | done |
| `inventory.heldSlot` | `held_slot()` | done |
| `inventory.cursor` | `inventory().cursor()` | done |
| `inventory.clickPending` | `inventory().pending()` | done |
| `canFly` | `can_fly()` | done |
| `events.blockUpdates` (S23/S22 in arrival order) | `events().block_updates` — `(BlockPos, raw state)` in arrival order, with `Interests::BLOCK_UPDATES`; overflow counted in `block_updates_dropped` | done (the reference keeps only blocks near the bot, ghast all — filter by distance) |
| `events.inventoryChanged` | `events().inventory.player_changed` (+ `window_changed`, `cursor_changed`, `opened`, `closed`) | done |
| `events.teleported` | `events().teleported` | done |
| `events.chat` (strings) | `events().chat` (`ChatMessage`: `plain()`, position, raw JSON), with `Interests::CHAT` | different |
| `eyes(pose?)` | `eyes(&pose)` | done |
| `selectSlot(hotbar)` | `select_slot(slot)` | done |
| `place(support, face, hit)` — refused unless the sent crosshair is on that face; the held count drops at once | `place(support, face, hit)` — same rule, vanilla `World.rayTrace` (stairs, slabs, reach 4.5); 1.8 planting (seeds, carrot, potato on farmland, nether wart on soul sand, air above) uses the item at once like vanilla `SeedItem.use`, other blocks wait for the server's set_slot | different (the reference decrements for every block item; a wrong local decrement is never corrected by a vanilla server, so ghast predicts only what it can check exactly) |
| `dig(pos, face)` — instant break C07 start + C0A, same look check | `dig(pos, face)` — C07 start + swing; a non-instant block keeps breaking with `set_buttons(left)` | done |
| `clickWindow(slot, button, mode, windowId?)` → `Promise<boolean>` | `click(Click::{Pickup, QuickMove, Swap, Drop, PickupAll})` → `Result<ClickTicket, ClickRefused>`; the outcome arrives in `events().inventory.clicks` (`ClickTicket::matches`) | different (no promise: tick-driven; refused while turning — "click only while still") |
| `window` (id, type, title, size, slot) | `inventory().window()` (`id()`, `kind()`, `title()`), `container_len()`, `slot(i)` | done |
| `guiOpen` | `inventory().gui_open()` | done |
| `chat(text)` | `chat(text)` (truncated to the version limit) | done |
| `openInventory()` | `open_inventory()` | done |
| `closeWindow()` | `close_window()` | done |
| `setAim(yaw, pitch)` (mouse counts, 14° / 10.5° a tick) | `set_aim(yaw, pitch)` (same algorithm, same budgets) | done |
| `setControls({forward, strafe, jump, sneak, sprint})` (numbers, ×0.3 when sneaking) | `set_controls(Controls { forward, back, left, right, jump, sneak, sprint })` (keys; the physics applies ×0.3 and ×0.98 like vanilla `Input`) | different |
| `setButtons({left, right})` | `set_buttons(Buttons { left, right })` | done |
| `target()` (block within 4.5 / player within 3, from the sent pose) | `target()` (block) and `target_entity()` (any pickable entity, vanilla `GameRenderer.updateTargetedEntity`) | different (two calls) |
| `mining` | `mining()` → `(pos, progress)` | done |
| `broken` | `broken()` | done |
| `setFlying(flying)` | `set_flying(flying)` — `abilities` before the move, only with "may fly" | done |
| `status(text)` | `status(text)` | done |

ghast has more on the context: `version`, `entity_id`, `health`, `is_dead`, `game_mode`,
`abilities`, `dimension`, `open_window`, `controls`, `aim`, `buttons`, `respawn`, `swing`,
`attack`, `attack_strength`, `using_item`, `disconnect`, `held_turns`, and `events()` adds
`joined`, `respawned`, `died`, `health_changed`, `game_mode_changed`, `chat_dropped`.

## `Behavior`

| contract | ghast `Behavior` | status |
|---|---|---|
| `name` | `name()` | done |
| `start(ctx)` | `start(ctx)` | done (see the tick model row) |
| `tick(ctx)` | `tick(ctx)` | done |
| `stop(reason)` | `stop(reason)` | done |
| `running` | `running()` (default `true`; `false` = done, the player keeps standing) | done |
| — | `interests()` (packet groups to decode) | ghast only |

Left to the application (not in the library): tick hooks, remote control, pausable
behaviours, stats / timelines, reconnect policy, TPS estimates, server-specific handling.

## Trace replay

`src/trace_replay.rs` (part of `cargo test`, ~4 s in the debug profile) replays packet traces
recorded by the reference bot through the engine — by default the two committed lab traces in
`tests/fixtures/traces/` (wheat plant and harvest with a throwaway account on a local lab
server; no private data):

1. **Input inference** — the trace has packets, not keys: a beam search over the 1.8 physics
   finds the keys (and `set_flying` calls) whose C0B / C13 / C03–C06 match each tick.
2. **Engine replay** — a full `Engine` (world, inventory, hands, rotation controller, physics)
   runs a behaviour that repeats the trace's actions (`select_slot`, `place`, `dig`, `chat`,
   `set_flying`), aims with `set_aim` and holds the inferred keys; every tick's outbound
   packets are compared with the trace: order and content (C08 position / face / cursor bytes
   / held stack, C07, C09, C0A, C0B, C13, looks bit for bit, positions within 1e-4).

The world is the lab wheat field plus the trace's block changes; inbound S08 / S23 / S22 /
S2F / S09 / S39 are applied before their tick.

```sh
cargo test -p ghast-client --lib lab_trace_replay -- --nocapture       # the fixtures
GHAST_TRACE=a.jsonl.gz:b.jsonl.gz \
  cargo test -p ghast-client --release --lib lab_trace_replay -- --nocapture   # other traces
```

Only clean traces from a local lab server with throwaway accounts belong in the repository;
traces from real servers contain account names, coordinates and server details.

Results (2026-11-04, local Paper 1.8.8 + Grim lab, fresh traces):

| trace | ticks | identical ticks | compared packets | notes |
|---|---|---|---|---|
| plant (field A, 512 seeds) | 4483 | **4483** | C03 2635, C04 157, C06 1691, C08 512, C0A 512, C09 7, C0B 2, C13 1 | positions within 4.7e-6 |
| harvest (field B) | 4482 | **4482** | C06 4188, C05 288, C07 331, C08 330, C0A 661, C0B 88, C03 4, C04 2, C13 1 | 27 ticks with reference positions off vanilla (below) |

Every `place` / `dig` the trace made was accepted by ghast's crosshair check; every turn but
one was reproduced by ghast's own rotation controller (`set_aim`; one tick at |yaw| ≈ 470°
was set directly).

**The reference bot's physics is not vanilla in two places** (the replay accepts these as
position deviations; ghast follows vanilla, which the oracle proves):
- Its flight model uses `Math.sin` / `Math.cos` instead of vanilla's 65536-entry sine table:
  flight positions differ by ~1e-6 a tick.
- Flying through water: vanilla moves a flying player in water with the water rules on top
  of creative flight — the water jump (+0.04 motion Y while jump is held,
  `LivingEntity.tickMovement`) and the water travel (0.02 acceleration, ×0.8 drag);
  the reference flight model has neither. Harvest t592, t742, t786, t878 differ by exactly
  0.04 in y; the following ticks by 0.007–0.028, consistent with the missing water travel.
