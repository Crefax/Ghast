# ghast-inventory

Inventory and container windows for the ghast client library: the player inventory, cursor and
open window as the vanilla client holds them, **vanilla's client-side click prediction** and the
window confirm protocol, for **1.8.9** (protocol 47), **1.12.2** (protocol 340; the click rules
hold for 1.12 – 1.12.2), **1.16.5** (protocol 754) and **26.1** (protocol 775). Pure and
sync: packets in, packets out; no I/O, no clock.

```rust
use ghast_inventory::{Click, Inventory, Outgoing, Version};

let mut inv = Inventory::new(Version::V1_8)?; // Err when the `v1_8` feature is off
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
  window; `set_slot` -1 = cursor, 1.12 / 1.16.5 -2 = `PlayerInventory` index, window 0 slots 36–44
  always reach window 0, other slots only the current screen (window 0 slot 10 while a chest is
  open is dropped, like vanilla); `close_window` ignores its id; window ids are compared after
  the packet's own (un)signed byte read; `login` / `respawn` reset everything (new player entity,
  action counters back to 0).
- 1.8 keeps zero-count stacks (vanilla 1.8 does); 1.12 and 1.16.5 treat them and air as empty
  (1.12 stacks keep their damage value; 1.16.5 has none on the wire).
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
| `QuickMove { slot, right }` (shift) | 1 | 0 / 1 | 1.8: first transfer's stack; 1.12 / 1.16.5: last transfer that left the same item (1.12: and damage) in the slot (EMPTY when the slot emptied) |
| `Swap { slot, hotbar }` (number keys; 1.16.5 `40` = offhand key, not on 1.12) | 2 | 0–8, 40 | none |
| `Drop { slot, all }` (Q / ctrl+Q; slot -999 = GUI click outside with empty cursor) | 4 | 0 / 1 | none |
| `PickupAll { slot }` (double click) | 6 | 0 | none |

Reproduced per version: stack limits (incl. 16-stacks, armor slots 1), 1.8 damage-variant
rules (the 41 item ids read off the vanilla registry), 1.8 PICKUP needing equal damage, 1.8
`insertItem` ignoring slot rules vs 1.16.5 checking `canInsert` and slot limits, the quick-move
retry loops, 1.8's number-key swap with a container slot (the hotbar item goes into the
inventory via `insertStack`), 1.16.5 swap limits, 1.16.5 `insertStack` preferring the selected
slot and the offhand, armor validity (1.8 armor ids + pumpkin/skull for the helmet; 1.16.5
preferred equipment slot incl. elytra, carved pumpkin, heads, shield → offhand), binding curse,
shulker boxes, double-click's two passes. **1.12.2** combines the two: 1.8-style damage
variants and damage comparisons (42 item ids off the 1.12.2 registry), 1.16-style empty stacks,
`insertItem` checking `canInsert` and slot limits, the quick-move loop, `insertStack` preferring
the selected slot and the offhand, and number-key swaps with the hotbar only; window 0 has the
offhand slot 45; the preferred equipment slot sends pumpkins and skulls to the helmet, elytra
to the chest and the shield to the offhand; binding curse is the `ench` id 10.

### Crafting

- **1.8.9**: like the vanilla client, ghast computes the result itself (`src/crafting` over `ghast_data::v1_8::recipes()`): the 373
  recipes in the client's match order (shaped with mirroring, shapeless, and the special recipes
  armor dye, fireworks (rocket, star, fade), book cloning, map cloning, repairing), recomputed
  exactly when vanilla does — whenever a grid slot is *set or taken from*, not on in-place count
  changes, and after `window_items` / grid `set_slot` (so the client's result wins over the
  server's). Built result NBT is written in Java `HashMap` key order (byte-identical packets).
  Banner recipes and map upscaling (which needs the map's state) make the result unknown.
- **1.12.2**: the client never computes results (vanilla computes them on the server only): the
  result slot holds what the server last sent, also after the grid changed (a stale result until
  the server's `set_slot`). `result_current()` says whether the result belongs to the grid as it
  is now. Taking a current result consumes the grid and hands back the remainders every 1.12
  recipe uses (buckets, a glass bottle for dragon's breath, the written book of book cloning);
  taking a stale result (its remainders depend on which recipe matches now) and grids with an
  NBT banner are refused as unpredictable.
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

1.12.2 `open_window` uses the 1.8 type strings plus `minecraft:shulker_box`
(`WindowKind::from_v1_12_2`); the brewing stand has 5 slots there.

| ghast `WindowKind` | 1.8 `inventoryType` | 1.16.5 menu id | container slots | shift-click |
|---|---|---|---|---|
| `Chest { rows }` | `minecraft:chest`, `minecraft:container`, unknown types with slots (`slotCount / 9` rows) | 0–5 (`generic_9x1..6`) | rows × 9 | yes |
| `Generic3x3` | `minecraft:dispenser`, `minecraft:dropper` | 6 | 9 | yes |
| `Hopper` | `minecraft:hopper` | 15 | 5 | yes |
| `ShulkerBox` | — (1.12: `minecraft:shulker_box`) | 19 | 27 | yes |
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

## 26.1

- Stacks keep their data component patch raw (`ItemData::Components`); `apply_raw_v26_1`
  reads `container_set_content` / `container_set_slot` / `set_cursor_item` /
  `set_player_inventory` with a structural skip (no component is decoded). Item accessors
  read one component on demand, over the item defaults from Mojang's 26.1.2 reports
  (`src/generated/items_26_1.rs`, regenerated with `examples/gen_items_26_1.rs`).
- Clicks: there is no confirm. The client sends the slots its prediction changed and the
  cursor as hashed stacks (vanilla `HashedStack`: crc32c `HashOps` over each component's
  codec); the server applies the click and resends only slots whose hash differs. ghast hashes
  every component kind a client can receive (107 of 110; the other three never reach a
  client), from a data-driven schema table (`src/v26_1_hash.rs`). Proven against the real
  client's hashes of 4000 stacks (`tests/golden/26.1/hashed_stacks.jsonl.gz`, all
  byte-identical) and live: 129 stacks, one kind each, drew no correction on a vanilla 26.1.2
  server. Data-driven registry names come from the configuration phase
  (`set_registries_26_1`). A component ghast cannot hash (an unsent registry, an inline holder
  other than a sound event, a block predicate with an `nbt` test) gets a hash the server
  corrects.
- Prediction: vanilla 26.1's own click handling (`src/sim26.rs`: `AbstractContainerMenu.doClick`
  with every click mode, each menu's slot rules and quick move, the client-side result updates)
  for window 0, all 25 menus a server opens with `open_screen` and the mount screens of
  `horse_screen_open` (`on_mount_screen_26_1`). The results the client computes itself are
  computed too: the anvil (`createResult`: repair, combining, enchantment compatibility, cost,
  renaming), the grindstone (merging, removing all but curses), the loom (selectable patterns,
  the result of the selected one) and villager trades (the selected or first matching offer,
  its payment). A 26.1 click is never refused: one ghast cannot predict (missing data) is sent
  unpredicted ([`ContainerClick::predicted`]) and the server's corrections bring the result. The packet lists the changed slots in the real
  client's `Int2ObjectOpenHashMap` order; `ContainerClick::swings` is how many arm swings the
  client sends first (every dropped stack swings).
- Server data the rules read: the furnace / blast furnace / smoker inputs, smithing slots and
  stonecutter inputs from `update_recipes` (`declare_recipes`), creative mode from `abilities`,
  the experience level from `experience` (the anvil), container data (anvil cost, crafter
  locks, loom selection), the offers of `merchant_offers`, the tags of `update_tags`, all read by
  `apply_raw_v26_1`; the configuration state's `update_tags` and `registry_data` go to
  `on_update_tags_26_1` / `on_registry_data_26_1` (the enchantment registry; a server that sends
  it without data uses vanilla's definitions, `src/generated/enchantments_26_1.rs`). Recipes,
  tags and enchantments are kept as raw bytes and parsed on the first click that needs them. Fuels, brewing ingredients and fuel, beacon payments,
  loom dyes and patterns, crafting remainders and "fits inside a bundle / shulker box" are
  item facts of the real client after configuration (`src/generated/menus_26_1.rs`, from the
  click vectors).
- Screen actions that are not clicks: `anvil_rename_26_1(name)` (`rename_item`, the result
  recomputed), `select_loom_pattern_26_1(index)` (`container_button_click`),
  `select_trade_26_1(index)` (`select_trade`, payment moved into the slots like the screen).
- Item use facts for the client engine: `v26_1::item_use(&stack)` (animation, duration,
  `use_effects` speed multiplier and sprinting, consumable, food).

| 26.1 | predicted (proven by the click vectors) | sent unpredicted |
|---|---|---|
| clicks | pickup, quick move, number keys, Q / ctrl+Q, double click, drags (split, one each, creative clone), creative middle click, clicks outside | — |
| window 0, chests, dispenser / dropper, hopper, shulker box, lectern, crafter | everything (shulker boxes refused by shulker box slots; crafter: locked slots) | — |
| crafting table, window 0 grid | everything; taking a result uses up the grid and leaves the items' remainders (the result itself is what the server sent) | — |
| furnace, blast furnace, smoker | input / fuel rules, quick moves by the server's item sets, a bucket as fuel | — |
| brewing stand, enchanting table, beacon | slot rules and quick moves | — |
| smithing table, stonecutter, cartography table | slot rules; the client's own result updates (none: no client recipes / map data) | — |
| grindstone | everything: merged / disenchanted results, taking them (experience is the server's) | — |
| loom | everything: pattern buttons, results, taking them | — |
| anvil | everything: results, costs, renames, taking them (experience level from `experience`) | — |
| villager trading | everything: trade selection, results by offer, taking them (uses and stock) | — |
| horse, donkey, mule, llama, trader llama, camel, skeleton / zombie horse, nautilus, zombie nautilus | everything: saddle and body armor slots by the entity's rules, chest columns | — |
| bundles | insert into / take out of a bundle (carried or in the slot), nested bundles, weights | a bundle whose contents do not read |
| creative mode | clone, clone drags, armor with the binding curse comes off, overflow vanishes; window 0 is the creative inventory screen (below) | — |

**Creative inventory screen** (window 0 opened in creative mode, `creative_screen_26_1()`): the
real client sends no `container_click` there but `set_creative_mode_slot` for every window 0
slot that differs from the menu's last broadcast (`Outgoing::Creative(Vec<CreativePacket>)`,
encode each with `CreativePacket::to_v26_1`): opening the screen, every click (Q and drops are
the screen's own: `swing`, the dropped stack as slot -1, the slot), and every server
`container_set_slot` while it is open, which the client echoes back
(`take_creative_echo_26_1`). Drops are throttled like `LocalPlayer.dropSpamThrottler`. Only
the inventory tab is modelled (the item tabs are a GUI for picking items; a bot sets a slot
directly).

Not covered: the creative screen's item tabs, a server that changes tags or recipe item sets
mid-session (the item facts are vanilla's), a selected bundle item (the bundle scroll wheel).

## Tests

```sh
cargo test -p ghast-inventory
```

- `tests/clicks.rs`: every click mode on window 0, chest, 3×3, crafting table, furnace, shulker
  box, all three versions, against expectations traced by hand through the vanilla 1.8.9 and 1.16.5
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
- 1.12.2 (2026-11-07, committed vectors, 3000 cases each): `clicks_seed1` 15 769 clicks, 15 599
  predicted, all byte-identical, 0 differences, 170 refused (furnace shift-clicks and fuel);
  `clicks_craft_seed1` 15 262 clicks, 14 797 predicted, all byte-identical, 0 differences, 465
  refused (taking a stale result: the oracle emulates the server's result only once per case,
  so most grid changes leave a stale result there).
- `tests/oracle_clicks_26_1.rs`: replays the 26.1 click vectors (oracle `clicks` with `MC=26.1`,
  `oracle/src/mc_26_1/…/ClickDump.java`: the real client's `handleContainerInput` on all 25
  menus and window 0, contents through real `container_set_content` packets, recipes through a
  real `update_recipes`, survival and creative, every click mode incl. valid and broken drags).
  For every click ghast predicts, the encoded `container_click` body must equal the client's
  bytes, the client must have sent as many arm swings, and every slot, the carried stack and the
  player inventory must equal the client's afterwards (compared by item, count and component
  hashes, as the server compares them). **Committed `tests/golden/26.1/clicks_seed1.jsonl.gz`
  (3000 cases, incl. 142 mount screens): 23 963 clicks and screen actions, all predicted and
  byte-identical, 0 differences. 30 000 cases (seed 2, 2026-11-12): 237 292, all predicted and
  byte-identical, 0 differences.** The vectors carry the server's tags, enchantment registry and
  item facts, the villager offers, the anvil names sent and the data slots after each click. Big runs: `MC=26.1 oracle/run.sh clicks --seed 2 --count 30000` then
  `GHAST_CLICK_VECTORS_26_1=$PWD/oracle/cache/26.1/clicks/clicks-seed2-n30000.jsonl.gz cargo test --release -p ghast-inventory --test oracle_clicks_26_1 -- --nocapture`.
- `tests/oracle_clicks_26_1.rs` `golden_26_1_creative_screen_matches_the_real_client`
  (`tests/golden/26.1/clicks_creative_seed1.jsonl.gz`, oracle `creative-clicks`: the real
  `CreativeModeInventoryScreen.slotClicked` with its listener, random earlier broadcasts,
  server slots in between): 1000 cases, 7756 events, 19 381 packets, all byte-identical, 0
  differences.
- `tests/v26_1.rs` `menus_session_replays` (`fixtures/menus_26_1_2.bin.zz`): `examples/live_clicks_26_1.rs`
  with `MENUS=1` on a vanilla 26.1.2 server (2026-11-12): drags, bundles, a crafting table
  (remainders), a furnace, an anvil (sword + enchanted book combined, renamed, taken), a
  grindstone (two damaged swords merged), a villager (two trades selected and taken), a shulker
  box and creative clicks — 69 clicks, all predicted, and 3 screen actions; the server
  corrected none (every slot compared after each click and action). The capture holds the
  configuration's tags and registry data; the replay sends every click and action byte for
  byte. The driver gives the items, places the blocks and summons the villager around the
  player (`forceload add` first: right after a join `setblock` reports the position as not
  loaded; a player saved dead is respawned, a dead player gets no chunks or entities).
- `tests/robustness.rs`: proptest sequences of random packets (wild ids, counts -3..130,
  oversized/short `window_items`, random window types) and clicks never panic; 1.12 and 1.16.5 never
  store empty stacks; clicks that only move items (all but drops) conserve every item.
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
  (1.16.5: `minecraft:chest[facing=north]{Items:[... orange_wool, oak_planks ...]}`, `--version 1.16 --port 25616`;
  1.12.2: `--version 1.12` with the 1.8 block syntax.)
- The same script through the full client (`crates/ghast/examples/inventory_probe.rs`: TickContext
  `click` / `open_inventory` / `close_window`, `place` on the chest after a GCD-stepped aim, the
  client's real movement stream), account GhastBot3, 2026-11-04: **1.8.9 30/30 accepted, 0
  differing slots; 1.16.5 30/30, 0 differing; 0 Grim flags on both.**
  `cargo run -p ghast --example inventory_probe -- --host 127.0.0.1 --port 25618 --username GhastBot3 --version 1.8.9`
- `src/**`: NBT tree equality (key order, root name, duplicates, NaN), NBT rule helpers, item
  facts per version, window type tables.
