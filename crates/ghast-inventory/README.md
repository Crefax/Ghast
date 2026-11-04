# ghast-inventory

Inventory and container windows for the ghast client library: the player inventory, cursor and
open window as the vanilla client holds them, **vanilla's client-side click prediction** and the
window confirm protocol, for **1.8.9** (protocol 47) and **1.16.5** (protocol 754). Pure and
sync: packets in, packets out; no I/O, no clock.

```rust
use ghast_inventory::{Click, Inventory, Outgoing, Version};

let mut inv = Inventory::new(Version::V1_8);
// inbound, on arrival: Some(..) is a confirm answer to write at once
// if let Some(out) = inv.apply_v1_8(&packet) { writer.queue(&out.to_v1_8())?; }
inv.tick();                                  // once per client tick
let open = inv.open_inventory();             // 1.8: Ok(Some(OpenInventory)) = C16
inv.tick();
match inv.click(Click::QuickMove { slot: 36, right: false }) {
    Ok(out) => { /* write out.to_v1_8() before this tick's movement packet */ }
    Err(e) => { /* Pending, Resyncing, SameTick, NoScreen, InvalidSlot, Unpredictable, ... */ }
}
```

## Model

- `Inventory` mirrors vanilla's data: `PlayerInventory` (0–8 hotbar, 9–35 main, 36–39 armor
  feet→head, 40 offhand), the 2×2 crafting grid and result, the cursor, and the open window's own
  slots. Screen slot numbers are vanilla's (`window0_slot`, `slot`, `slot_of_player`).
- Inbound handling follows `ClientPlayNetworkHandler`: `window_items` for window 0 or the open
  window; `set_slot` -1 = cursor, 1.16.5 -2 = `PlayerInventory` index, window 0 slots 36–44
  always reach window 0, other slots only the current screen (window 0 slot 10 while a chest is
  open is dropped, like vanilla); `close_window` ignores its id; window ids are compared after
  the packet's own (un)signed byte read; `login` / `respawn` reset everything (new player entity,
  action counters back to 0).
- 1.8 keeps zero-count stacks (vanilla 1.8 does); 1.16.5 treats them and air as empty.
- Item stacks keep NBT raw (`RawNbt`); `window_items` never parses NBT. Equality compares bytes,
  then (only if they differ) the parsed trees like Java's `NbtCompound.equals`. NBT is parsed only
  for the few rules that read it (`Unbreakable`, 1.16 `Damage`, binding curse) and only when that
  rule is reached.

## Clicks

`click(Click)` runs the version's `onSlotClick` on a scratch copy, then commits it, increments the
screen's action counter (vanilla increments before the click; the first click of a container is
action 1) and returns the `window_click` with the stack vanilla's `onSlotClick` returned (the
server compares it with its own result and rejects the click on a mismatch).

| `Click` | mode | button | clicked item |
|---|---|---|---|
| `Pickup { slot, right }` (slot -999 = drop cursor / one) | 0 | 0 / 1 | slot before the click |
| `QuickMove { slot, right }` (shift) | 1 | 0 / 1 | 1.8: first transfer's stack; 1.16.5: last transfer that left the same item in the slot (EMPTY when the slot emptied) |
| `Swap { slot, hotbar }` (number keys; 1.16.5 `40` = offhand key) | 2 | 0–8, 40 | none |
| `Drop { slot, all }` (Q / ctrl+Q; slot -999 = GUI click outside with empty cursor) | 4 | 0 / 1 | none |
| `PickupAll { slot }` (double click) | 6 | 0 | none |

Reproduced per version: stack limits (incl. 16-stacks, armor slots 1), 1.8 `hasSubtypes`
damage rules (the 41 item ids read off the vanilla registry), 1.8 PICKUP needing equal damage, 1.8
`insertItem` ignoring slot rules vs 1.16.5 checking `canInsert` and slot limits, the quick-move
retry loops, 1.8's number-key swap with a container slot (the hotbar item goes into the
inventory via `insertStack`), 1.16.5 swap limits, 1.16.5 `insertStack` preferring the selected
slot and the offhand, armor validity (1.8 armor ids + pumpkin/skull for the helmet; 1.16.5
preferred equipment slot incl. elytra, carved pumpkin, heads, shield → offhand), binding curse,
shulker boxes, double-click's two passes.

### Crafting

- **1.8.9**: like the vanilla client, ghast computes the result itself (`src/crafting` over `ghast_data::v1_8::recipes()`): the 373
  recipes in the client's match order (shaped with mirroring, shapeless, and the special recipes
  armor dye, fireworks (rocket, star, fade), book cloning, map cloning, repairing), recomputed
  exactly when vanilla does — whenever a grid slot is *set or taken from*, not on in-place count
  changes, and after `window_items` / grid `set_slot` (so the client's result wins over the
  server's). Built result NBT is written in Java `HashMap` key order (byte-identical packets).
  Banner recipes and map upscaling (which needs the map's state) make the result unknown.
- **1.16.5**: the server sends results; the client keeps the recipes from `declare_recipes`
  ([`RecipeBook`], `apply_v1_16_5` handles the packet) and uses them for the remainders of a taken
  result. After a grid change the result is unknown until the server's `set_slot`.
- Taking a result (click, shift-click — 1.8 repeats while the result stays the same item —,
  number key, Q) consumes one of each ingredient and hands back the recipe's remainders
  (buckets, 1.16.5 glass bottles; book cloning keeps the written book).

**Refused instead of guessed** (`ClickError::Unpredictable`, nothing sent, nothing changed):
shift-click from the player part of a furnace (smelting recipes, fuel), any interaction that
reads a fuel / brewing / enchanting / anvil / beacon / trading / horse-equipment slot, a
crafting-result click while the result is unknown, taking a result whose grid matches no recipe
ghast models (1.8 banners, map upscaling; 1.16.5 special recipes, a written book or banner in the
grid, or no recipe at all — vanilla then aliases the grid stacks).

### Window types

| ghast `WindowKind` | 1.8 `inventoryType` | 1.16.5 menu id | container slots | shift-click |
|---|---|---|---|---|
| `Chest { rows }` | `minecraft:chest`, `minecraft:container`, unknown types with slots (`slotCount / 9` rows) | 0–5 (`generic_9x1..6`) | rows × 9 | yes |
| `Generic3x3` | `minecraft:dispenser`, `minecraft:dropper` | 6 | 9 | yes |
| `Hopper` | `minecraft:hopper` | 15 | 5 | yes |
| `ShulkerBox` | — | 19 | 27 | yes |
| `Crafting` | `minecraft:crafting_table` (0 slots) | 11 | 10 (result + 3×3) | yes (1.16: into the grid first), not from the result |
| `Furnace` | `minecraft:furnace` | 13, 9 (blast), 21 (smoker) | 3 | from furnace slots only |
| `BrewingStand`, `Enchanting`, `Anvil`, `Beacon`, `Merchant`, `Horse`, `Grindstone`, `Lectern`, `Loom`, `Smithing`, `Cartography`, `Stonecutter` | as vanilla | as vanilla | as vanilla | no (player-part clicks of other modes work) |

### Confirm protocol and timing

- **One click in flight**: further clicks get `Pending` until the confirm with the click's action
  number arrives (`ClickOutcome::Accepted`), or `TimedOut` after 40 ticks (`set_click_timeout`).
- **Reject → resync**: on a rejected confirm the server marks the window, sends `window_items`
  and the cursor; clicks get `Resyncing` until that `window_items` arrives. The answer to every
  rejected confirm (window 0 or the open window; Grim's pings included) is
  `on_transaction`'s return value — vanilla's `accepted = true` echo, produced exactly once.
  Route every confirm through it and answer nowhere else (answer S32 once).
- **Per-tick packet order** (call `tick()` at each tick start): at most one click per tick
  (Grim PacketOrderA flags pickup and shift-click in one tick); no click or close in the tick
  `open_inventory` sent the 1.8 C16 and no C16 after a click/close in the same tick (PacketOrderK).
- Clicks in window 0 need the player GUI open (`open_inventory`; vanilla cannot click without
  it). `close()` drops the cursor client-side and on 1.8 also empties the 2×2 grid, like vanilla.
- **Only click while still**: the server and anticheats judge clicks against the last sent pose;
  the client (TickContext) must not click while turning.

## Tests

```sh
cargo test -p ghast-inventory
```

- `tests/clicks.rs`: every click mode on window 0, chest, 3×3, crafting table, furnace, shulker
  box, both versions, against expectations traced by hand through the vanilla 1.8.9 and 1.16.5
  `onSlotClick` / `transferSlot` / `insertItem` / `insertStack` code; the confirm protocol
  (pending, same-tick guard, reject → resync, foreign confirms, timeout), screen handling and
  `set_slot` routing.
- `tests/oracle_clicks.rs`: replays vanilla's own click handling (oracle `clicks` command,
  `oracle/src/mc_<ver>/…/ClickDump.java`: the real `ClientPlayerInteractionManager.clickSlot` on real
  screen handlers — window 0, chests of 1–6 rows, 3×3, hopper, crafting table, furnace, 1.16.5
  shulker box — with random contents incl. NBT, damaged / over-stacked / zero-count stacks, random
  cursor, armor and selected slot, 1–10 random clicks of modes 0/1/2/4/6 per case). For every click
  ghast predicts, the encoded `window_click` body must equal vanilla's bytes and every slot, the
  cursor and the player inventory must equal vanilla's state afterwards; a click ghast refuses ends
  the case (vanilla used an action number ghast did not). Committed: `tests/golden/<ver>/clicks_seed1.jsonl.gz`
  (3000 cases). Big runs: `oracle/run.sh clicks --seed 2 --count 30000` then
  `GHAST_CLICK_VECTORS_1_8=$PWD/oracle/cache/1.8.9/clicks/clicks-seed2-n30000.jsonl.gz cargo test --release -p ghast-inventory --test oracle_clicks`.
  Crafting vectors (`clicks --craft true`, `clicks_craft_seed1.jsonl.gz`): grids filled from
  real recipes and the special recipes' ingredients, 40 % of the clicks on the result; 1.16.5 with
  the vanilla data pack's recipes synced through a real `declare_recipes` packet (its bytes head
  the file and are decoded by ghast-protocol) and the server's result emulated after each grid
  change. **25 000 cases per version: 1.8.9 131 542 predicted clicks (11 207 taking a result), 0
  differences, all byte-identical; 1.16.5 138 224 (6 313 taking a result), 0 differences.**
  **2026-11-04, 30 000 cases per version: 1.8.9 154 347 predicted clicks, 1.16.5 154 584, 0
  differences** (1.8.9 packets byte-identical; on 1.16.5 vanilla's own `NbtCompound.copy` reorders
  compound keys, so NBT is compared as trees there — as the server does). Refused: ~1.5 %, furnace
  shift-clicks from the player part and clicks on unknown crafting results. The oracle found three
  vanilla details the hand-derived tests missed: `SimpleInventory.setStack` caps a container slot at
  64 (the excess of an over-stacked stack is lost), 1.8 double click keeps a zero-count cursor, and
  1.16.5 `insertStack` into an empty slot uses AIR's stack limit (64) for the new zero-count stack.
- `tests/robustness.rs`: proptest sequences of random packets (wild ids, counts -3..130,
  oversized/short `window_items`, random window types) and clicks never panic; 1.16.5 never
  stores empty stacks; clicks that only move items (all but drops) conserve every item.
- `examples/live_clicks.rs` (lab tool, not run by `cargo test`): a minimal standing-still client
  opens a chest with a real block interaction, sends 15 clicks of every mode in the chest and 15 in
  window 0 (armor, crafting grid, 16-stacks, number keys, Q, double click, outside), then reopens the
  chest and compares the server's `window_items` with the prediction.
  **2026-11-04, local WSL labs (Paper 1.8.8 / Paper 1.16.5, both + Grim 2.3.73), account
  GhastBot: 1.8 30/30 clicks accepted, 0 differing slots; 1.16.5 30/30 accepted, 0 differing
  slots (incl. 1.16.5 shift-clicks carrying EMPTY); 0 Grim flags on both.**
  ```sh
  lab/console.sh 1.8 'setblock 0 4 2 chest 2 replace {Items:[{Slot:0b,id:"minecraft:stone",Count:64b},{Slot:1b,id:"minecraft:dirt",Count:5b},{Slot:2b,id:"minecraft:wool",Damage:1s,Count:10b},{Slot:3b,id:"minecraft:planks",Count:12b}]}'
  # after the bot joined: gamemode survival, clear, tp 0.5 4 0.5 0 0, give stone 30, dirt 60, diamond_helmet 1, ender_pearl 10
  cargo run -p ghast-inventory --example live_clicks -- --version 1.8 --port 25618 --user GhastBot
  ```
  (1.16.5: `minecraft:chest[facing=north]{Items:[... orange_wool, oak_planks ...]}`, `--version 1.16 --port 25616`.)
- The same script through the full client (`crates/ghast/examples/inventory_probe.rs`: TickContext
  `click` / `open_inventory` / `close_window`, `place` on the chest after a GCD-stepped aim, the
  client's real movement stream), account GhastBot3, 2026-11-04: **1.8.9 30/30 accepted, 0
  differing slots; 1.16.5 30/30, 0 differing; 0 Grim flags on both.**
  `cargo run -p ghast --example inventory_probe -- --host 127.0.0.1 --port 25618 --username GhastBot3 --version 1.8.9`
- `src/**`: NBT tree equality (key order, root name, duplicates, NaN), NBT rule helpers, item
  facts per version, window type tables.
