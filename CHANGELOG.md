# Changelog

All crates share one version number.

Versioning: until 1.0, a **patch** release (0.y.Z) adds features and fixes without breaking the
public API; a **minor** release (0.Y.0) may break it and says how to migrate. 1.0 comes only when
the API has been stable for a while and the 1.0 criteria in CONTRIBUTING.md are met.

## 0.4.2 — 2026-11-13

Minecraft **26.2 (protocol 776)**; 26.2 is the only game version with protocol 776 so far. Additive
only: every 0.4.1 API stays.

### Added

- **Minecraft 26.2 protocol (776)** in `ghast-protocol`: module `v26_2`, feature `v26_2` (on by
  default), version table row 776 → 26.2. minecraft-data has no 26.2 data yet, so the packets are
  generated from its 26.1 data plus a reviewed delta (`codegen/protocol/delta/776.json`: each
  change names the vanilla 26.2 code it follows): the session id after the login profile,
  `onlineMode` in the play `login`, the new team parameter order with an optional team colour,
  `spectator_action` (replaces `spectate_entity`), the geyser particles, the
  `sulfur_cube_content` component, and the 26.2 particle, component, attribute and command
  parser ids. 487 of the 573 definitions are shared with older protocols. Packet ids are checked
  against the 26.2 server's packet report; a capture from a vanilla 26.2 server (status, login,
  configuration, play) decodes strictly and re-encodes byte-identically
  (`tests/capture_26_2.rs`); example `login_26_2`.
- `slot::v775::ComponentPatch` / `ItemStack` also hold 26.2 stacks: `read_776`, `decode_776`,
  `decode_all_776`, `from_parts_776`, `ComponentPatch::protocol`.
- **26.2 data** in `ghast-data` (`v26_2`, feature `v26_2`, on by default): blocks, states,
  properties, collision and crosshair shapes, items, tools, dig facts and entities from the real
  26.2 jar (oracle dumps in `tests/golden/26.2/`); display fields minecraft-data lacks for 26.2
  are its 26.1 values by name plus a reviewed table for the new blocks, items and the sulfur
  cube. Same API as `v26_1`; the movement types (`StatePhys`, `BlockKind`, `block_flags`,
  `state_flags`, `VoxelShapeRef`, `VoxelClass`) are one type shared by both, and so are the shape
  tables. 26.2 `BlockData` / `ItemData` rows hold no pointers (no `name` / `display_name`
  fields); `Block::name` / `display_name` and `Item::name` / `display_name` read them, and are
  new on `v26_1` too.
- **26.2 world** in `ghast-world` (`v26_2`, feature `v26_2`): the 26.1 world over the 26.2 block
  states and packets; checked against a vanilla 26.2 server (every chunk of a capture, 1576
  block probes confirmed by the server).
- **26.2 movement** in `ghast-physics`: rule set `modern::V26_2` (feature `v26_2`, on by
  default) over `ghast_data::v26_2`, bit-exact against the vanilla 26.2 oracle (all 26.2 movement
  goldens; 16.2 M cached fuzz ticks at 0 diffs, incl. 1.0 M of restitution fuzz; 3200 golden
  crosshair rays identical). 26.2 is the 26.1 row plus one structural
  change behind the new defaulted `Rules::RESTITUTION`: `Entity.restituteMovementAfterCollisions`
  (colliding axes reflected by the `bounciness` attribute, landing on slime (1) / beds (0.75,
  ×0.8 for a boat), gravity and drag compensation, no bounce when sneaking or on honey) and the
  `friction_modifier` / `air_drag_modifier` attributes. New: `modern::MovementAttributes`
  (`PlayerExt::attributes`, `get_mut(name)`; read by the extended ticks), `MemoryWorld262`,
  `memory_world_26_2`, per-field `Rules` methods (`friction`, `speed_factor`, `jump_factor`,
  `block_flags`, `offset`, `block_name`; defaulted from `Rules::block`), `pick::v26_2`.
- **Oracle**: one shared modern harness (`oracle/src/modern`, formerly `oracle/src/mc_26_1`)
  compiled per version with a small per-version shim (`Compat`); the 26.1 goldens it produces are
  content-identical to 0.4.1's (the 26.1 `entities` golden gains one case,
  `push_large_scale_section_reach`, for the boat-push fix below). `tests/golden/26.2/` (29 files, incl. `fuzz_restitution_seed1`;
  `scenarios` has 108 scenarios, among them 20 restitution and movement-attribute scenarios and
  15 idle drops onto slime (6 – 30 blocks) and a bed).
- **26.2 entity tracking** in `ghast-entity`: `v26_2::Tracker` (feature `v26_2`, on by default),
  the 26.1 tracker source compiled a second time over `ghast_protocol::v26_2` /
  `ghast_data::v26_2` (26.2 entity ids, the sulfur cube, sizes, `SCALE_ATTRIBUTE` = 30, the 776
  slot codec); same API as `v26_1`. Replays `tests/golden/26.2/entity_tracking.jsonl.gz` with
  every field equal (1728 ticks, 7266 packets, 305,600 fields; like 26.1, the tick order differs
  only around riding entities: 3306 of 3456 ticks equal) and the vanilla 26.2 capture.
- **26.2 inventory** in `ghast-inventory` (`Version::V26_2`, feature `v26_2`, on by default):
  click prediction for every menu, creative screen, hashed click stacks (incl. the new
  `sulfur_cube_content`), item facts from the 26.2 reports. Proven like 26.1: the 26.2 click
  vectors (23,963 golden and 237,292 cached clicks) and creative vectors (19,259 packets)
  byte-identical, 0 differences; the real client's hashes of 4,000 stacks (108 of 111 component kinds) exact. One modern
  implementation over per-version tables: components are handled as canonical kinds mapped to
  each version's wire ids at compile time.
- `ghast-inventory` version-neutral modern API (dispatches on the inventory's version):
  `Inventory::{apply_raw_modern, encode_modern, encode_creative_modern, on_open_screen_modern,
  on_registry_data_modern, on_update_tags_modern, on_merchant_offers_modern,
  on_mount_screen_modern, set_registries_modern, registries_modern, set_creative_modern,
  anvil_rename_modern, select_loom_pattern_modern, select_trade_modern, …}`,
  `Window::menu_modern`, `Version::is_modern`, and module `modern` (functions taking a
  `Version`: `item_name`, `item_id`, `defaults`, `component_id`, `component_name`, `menu_name`,
  `window_kind`, `item_use`, `component_hash`, `hash_covers`, `packet_ids`, …). The 0.4.1
  `_26_1` methods and `v26_1` functions stay and are aliases.
- **Minecraft 26.2 in the client** (`ghast-client`, `ghast`): `GameVersion::V26_2` (`"26.2"`,
  `"776"`), feature `v26_2` (on by default, forwarded by the facade, `ghast-ext`,
  `ghast-pathfinder` and `ghast-viewer`), capability row = 26.1's with the `V26_2` physics and
  the 26.2 data. The modern client is now written once and compiled per version (a small row
  module per version; no run-time version branches), and ids that versions renumber are
  resolved by name. 26.2 deltas: the play login's `onlineMode` decides whether a chat session
  starts; the new team parameter order; the movement attributes `bounciness`,
  `friction_modifier`, `air_drag_modifier`. Every 26.1 engine replay of the oracle goldens also
  runs on 26.2 (joins, teleports, elytra, riptide, entities, border, nether, boats, minecarts,
  item use, entity pick, `player_loaded`, signed chat, login encryption), plus the idle slime /
  bed drop scenarios, packet for packet. Lab: every 26.1 lab script passes on a Paper 26.2 +
  Grim 2.3.74 lab with 0 flags (one new documented Grim corner case, below).
- `ghast-pathfinder`: `Version::V26_2` (its jump table measured with the 26.2 physics);
  `ghast-viewer`: draws 26.2 sessions (module `v26_2`, feature `v26_2`); `ghast-ext`: 26.2 block,
  item and entity names.
- `Client::force_step_position` (Grim compatibility, off by default) now also works on 26.1
  and 26.2.
- Lab: a 26.2 lab (Paper 26.2 build 135, Grim 2.3.74, `127.0.0.1:25634`); the 26.x scripts take
  `LAB_VER=26.2` (proxy switch, online mode, entities, elytra, riptide, transfers).

### Changed

- **Build cost of every version.** The modern client, physics, inventory and tracker are one
  source compiled once per modern version, so the default (every-version) build carries two
  modern copies. Stripped `afk` example, release profile, x86-64, 2026-11-13: every version
  7.94 MB (0.4.1: 6.25 MB), 26.1 only 4.16 MB (0.4.1: 4.13 MB), 26.2 only 4.00 MB. Idle AFK bot
  (RSS / private dirty): every-version build 8.56 / 2.26 MB on 26.1 (0.4.1: 8.08 / 2.21 MB; the
  difference is file-backed code pages) and 8.98 / 2.28 MB on 26.2; single-version builds
  6.66 / 1.85 MB (26.1) and 6.50 / 1.68 MB (26.2). A bot that joins one server version should
  build only that version (`default-features = false, features = ["v26_2", …]`; see the README's
  Cargo features).
- `ghast_protocol::defs::p776` is `#[doc(hidden)]`: its shared definitions are not a stability
  promise; name 26.2 packets through `ghast_protocol::v26_2`. Switch enums and id newtypes of
  definition modules newer than the last published protocol (775) are `#[non_exhaustive]` (id
  newtypes gain `new()`).
- `ghast-physics`: `ModernMemoryWorld` decodes block states when they are stored, not on every
  read (a second modern row stopped the decode function pointer from being folded; the plain
  26.1 tick on the memory world is now about 1 % cheaper than in 0.4.1).

### Fixed

Both are behaviour corrections of the modern core, so they change **26.1** as well as 26.2; all
26.1 goldens and the 12.2 M cached 26.1 fuzz ticks still replay with 0 diffs.

- `ghast-physics` modern core (26.1 and 26.2): a boat pushes the player only when its push query
  reaches the player's entity section (`Level.getEntities` visits the sections of the query box
  grown by 2 blocks horizontally and 4 downwards); a scaled-up player overlapping a boat far from
  its own section is not pushed. Mob pushes are unaffected (the client tests the local player's
  box directly). New `modern::section_query_reaches` for callers filtering entity collision boxes.
  Oracle case `push_large_scale_section_reach` (26.1 and 26.2 `entities` goldens).
- `ghast-physics` modern core (26.1 and 26.2): a zero movement input still adds `Vec3.ZERO` to
  the velocity (`moveRelative`), which turns `-0.0` into `+0.0` as vanilla does. No 26.1 vector
  could show it: 26.1 never leaves a signed zero in a horizontal velocity before an input-less
  tick (its collisions zero the axis with `0.0`); 26.2's restitution does.

### Known approximations

- **Fluid shape cache: ghast behaves as a freshly started client.** Vanilla's
  `FlowingFluid.getShape` caches one shape per fluid state for the whole process, built with the
  height of the first block it was asked about (water sources have amount 8, so a source first
  clipped with water above keeps a full-block shape for every later source). Only ray clips
  with a fluid mode read it: on the client the fall-distance raycast of `Entity.move` (the client
  `fallDistance`) and the item-use ray of buckets, bottles, lily pads and other place-on-water
  items, boat items and spawn eggs. In a long-lived real client whose cache is "sticky", such a
  ray grazing the top ninth of a water source cell hits where a fresh client misses, which can
  change the item-use outcome (swing packet, predicted block and held item) and the fall
  distance. `ghast-physics` computes the fluid height per query; the oracle starts every case
  with empty caches (three 26.2 cached teleport cases had picked up a full-block source shape
  from earlier cases in the same JVM).

### Known Grim corner cases

Three situations in which Grim flags movement that the oracle proves identical to the real
client's (details in `crates/ghast-client/README.md`):

1. A landing step-up onto a 1/8-higher block (soul sand → grass) whose move stays under the
   position resend threshold (found on the 1.8.9 and 1.16.5 labs;
   `Client::force_step_position(true)`, now also on 26.1 / 26.2, avoids it).
2. 1.16.5 walking on slime (on-ground alternates every tick; GroundSpoof).
3. **New, 26.2:** an idle 20-block drop onto slime. Grim 2.3.74 (and the 2.3.75 alpha) flag
   Simulation 0.0417 at the tail of the bounce, where the vanilla client rests 6.2e-5 above the
   slime and sends only `flying(on_ground)` for a tick. ghast's packets equal the real 26.2
   client's on every tick of that drop (oracle scenario `slime_drop_20_long`, replayed through
   the engine); other drop heights (6 – 30 blocks) and beds draw no flag.
   `force_step_position` cannot help (the position did not change).

## 0.4.1 — 2026-11-12

### Added

- **Signed chat on 26.1** (online mode): `ghast_auth::chat` (`ProfileKeys`,
  `SignedMessageChain`: SHA256withRSA over the 26.1 signed body),
  `MicrosoftAuth::profile_keys` (`player/certificates`; `with_certificates_url`,
  `certificates_url`, `Stage::ProfileKeys`), `Client::signed_chat` (default on) and
  `Client::profile_keys`, re-exported `ghast::ProfileKeys`. On encrypted connections each join
  starts a chat session (`chat_session_update` from the connection tick), chat messages and the
  `minecraft:message` arguments of commands are signed; the key pair is cached per `Client` and
  refreshed when due. Which commands are signable comes from the server's command tree
  (`declare_commands`, kept raw and parsed only when a command is sent). Proven byte-identical
  (signatures included) against the real client: `tests/golden/26.1/chat_signing.jsonl.gz`.
- **Server transfers** (1.20.5+ `transfer`, configuration and play state):
  `SessionEnd::Transferred { host, port }`, `Client::follow_transfers` (opt-in: log in to the
  target with the transfer intent and the stored cookies, at most 8 in a row),
  `Inbound::Transfer`.
- `LoginInbound::EncryptionRequestUnauthenticated`: 26.1 encryption requests with
  `shouldAuthenticate = false` are answered without the session-server join (any account).
- `Outbound::ChatSessionUpdate`.
- Oracle dumps `net-crypto` and `chat-signing` (26.1): `tests/golden/26.1/net_crypto.jsonl.gz`
  (session hash, `hello`, key packet, AES/CFB8) and `chat_signing.jsonl.gz`.
- Lab: `proxy-switch.sh` (Velocity + two 26.1 backends with different overworld heights,
  repeated `/server` switches), `online-26.1.sh` + `fake-session-server.py` (online mode and
  `enforce-secure-profile` against a local account stand-in), `transfer.sh`; examples
  `server_switch`, `online_chat` (feature `microsoft`), option `--follow-transfers`.
- CI runs the protocol differential tests against node-minecraft-protocol (`setup-node`,
  `npm ci`); `GHAST_REQUIRE_NODE=1` makes a missing harness fail instead of skip.
- **26.1 click prediction for every menu**, proven against the real client (oracle `clicks`,
  `tests/golden/26.1/clicks_seed1.jsonl.gz`: all 25 `open_screen` menus, the mount screens
  and window 0, survival and creative, every click mode, screen actions; 237 292 clicks and
  actions of a 30 000-case run, all predicted and byte-identical): drags (`Click::QuickCraft`
  with `DragStage` / `DragKind`), creative middle-click (`Click::Clone`), bundles, crafting
  results taken with their remainders, furnace / smoker / blast furnace input and fuel rules
  (from the server's `update_recipes` item sets), brewing stand, enchanting table, beacon,
  smithing, stonecutter, cartography table, crafter (locked slots), lectern, hopper,
  dispenser, shulker box, and the results the client computes itself: the anvil (repair,
  combining, enchantment compatibility, cost, renaming), the grindstone, the loom (selectable
  patterns) and villager trades (offers, payment, uses and stock).
- Mount inventories (`horse_screen_open`): `Inventory::on_mount_screen_26_1(window_id,
  columns, entity_type, can_saddle, can_body)` — horse, donkey, mule, llama, trader llama,
  camel, skeleton / zombie horse, nautilus, zombie nautilus.
- Screen actions: `Inventory::anvil_rename_26_1` (`Outgoing::RenameItem`),
  `select_loom_pattern_26_1` (`Outgoing::ButtonClick`), `select_trade_26_1`
  (`Outgoing::SelectTrade`; the payment moved like the villager screen does).
  Server data for them: `on_update_tags_26_1`, `on_registry_data_26_1` (configuration state),
  `on_merchant_offers_26_1`; `apply_raw_v26_1` reads play `update_tags` and `merchant_offers`
  (`PACKET_IDS_26_1`). Recipes, tags, the enchantment registry and offers stay raw bytes until
  a click needs them; vanilla's enchantment definitions are built in
  (`src/generated/enchantments_26_1.rs`) for servers that send the registry by known pack.
- `ContainerClick::swings`: the arm swings the real client sends before a click that drops a
  stack (send them first). `WindowKind::Crafter` (the crafter screen opens on 26.1).
  `Window::menu_26_1`. `Inventory::{on_open_screen_26_1, on_recipes_26_1, set_creative_26_1,
  set_experience_level_26_1}`; `apply_raw_v26_1` also reads `declare_recipes`, `abilities`
  and `experience` (now in `PACKET_IDS_26_1`).
- `v26_1::item_use(&Stack) -> Option<ItemUse>`: use animation and duration, `use_effects`
  (speed multiplier, can sprint), `consumable`, `food` of a 26.1 stack (checked against the
  real client's components in `hashed_stacks.jsonl.gz`).
- `v26_1::{glider, can_glide_using, next_damage_will_break}`: the slot a glider (the `glider`
  component, the elytra by default) works from and `LivingEntity.canGlideUsing` (not when the
  next damage breaks it).
- `Inventory::enchantment_name_26_1(id)`: an enchantment's registry name (`minecraft:sharpness`).
- `Inventory::trident_spin_attack_strength_26_1(&Stack) -> Option<f32>`
  (`EnchantmentHelper.getTridentSpinAttackStrength`): the `trident_spin_attack_strength` effect
  of the stack's enchantments as the server's enchantment registry defines it (vanilla's
  definition for entries sent without data; level-based values and value effects evaluated
  like the game).
- `v26_1::item_name(id)`: an item's registry name without the item defaults table. The 26.1
  item, registry-name and enchantment tables ghast reads are plain read-only data now (strings
  as spans of one text, no pointers): about 205 KB less relocated data per process
  (`.data.rel.ro` of the `afk` example 1 053 560 -> 843 896 B). `v26_1::defaults` builds its
  table on the first call.
- **26.1 creative inventory screen**: window 0 opened in creative mode sends what the real
  screen sends (`Outgoing::Creative(Vec<CreativePacket>)`: `set_creative_mode_slot` for the
  slots that differ from the last broadcast, drops as slot -1 with the arm swing; echoes of
  server slots from `take_creative_echo_26_1`; the drop spam throttle), proven against the
  real screen (`tests/golden/26.1/clicks_creative_seed1.jsonl.gz`, 19 259 packets).
- Oracle dumps `clicks` and `creative-clicks` (26.1) and `examples/gen_menus_26_1.rs` (the
  menus' item facts from the click vectors).
- **Path finding on 26.1** (`ghast-pathfinder` feature `v26_1`, default on, forwarded by
  `ghast-ext` and the facade): `Version::V26_1` on the modern physics core. Block classes come
  from the jar's 26.1 movement records (powder snow, scaffolding, cobwebs, berry bushes, fire
  never entered; magma, campfires, pointed dripstone, lava cauldrons, big dripleaves, sculk
  sensors / shriekers never stood on); move costs and the gap-jump table are measured with
  `modern::tick`. The look-ahead, the pre-jump check and the verification simulate with the
  live player (server attributes such as step height, jump strength, movement speed and scale,
  effects, abilities): `Controller::set_modern_player`, `Simulation::with_modern_player`; the
  `Follower` feeds `TickContext::modern_player` every tick. Build height from the dimension
  (negative Y; below the bottom is unknown): `BlockSource::build_height` (defaulted). The
  move set follows the live attributes at plan start (`Search::with_modern_player`, used by
  the `Follower`): step height, jump strength / jump boost / gravity (jump-ups up to the
  jump's apex), gap jumps only with vanilla jump, gravity, speed and scale. On 26.1 only: a
  walk braking before a wall without progress for 10 ticks accepts the graze (the exact
  look-ahead could otherwise alternate forward and back at a corridor's mouth); 1.8.9 –
  1.16.5 plans and simulated runs are identical to 0.4.0 (209 cases compared).
  `Bot::goto` works on 26.1. Live: the lab parkour corridor and maze on Paper 26.1.2 + Grim
  2.3.74, 0 flags, 0 setbacks.
- `ghast_client::TickContext::modern_player` (26.1 movement state) and
  `WorldView::build_height` (defaulted; 26.1 from the dimension).
- **Bot screen for 26.1** (`ghast-viewer` feature `v26_1`, default on; the facade's `v26_1`
  forwards it): a 26.1 world is drawn with the 1.8.9 assets — blocks and items mapped onto
  their 1.8 states (exact where 1.8 has them, look-alikes for newer ones, by shape otherwise;
  `ghast_viewer::v26_1`), the 256 rows around the bot shown, entities from the 26.1 tracker,
  the hotbar from the component item model. Text stays the truth: new messages `ITEMS` (28) and
  `ENTITY` (29) carry the real 26.1 ids, English names, count, damage, enchantments, custom
  names (from `Inventory::enchantment_name_26_1`) and a `standIn` flag; `HELLO` and `STATE`
  name the game (`game`, `drawnWith`, `worldMinY`; replays keep them). The page shows it: the
  header ("26.1 (drawn with 1.8.9 assets)", world Y), the held item and armour, hotbar tooltips
  (name, count, enchantments, durability, id), a ≈ mark on stand-in icons, durability bars from
  the real damage, name tags over entities and dropped items drawn with a stand-in. 1.8.9
  payloads and page behaviour are unchanged (byte-identical frames to 0.4.0).
  `BotView::{build_height, enchantment_name}` (defaulted). The crate's `include` patterns are
  anchored (a local `web/node_modules` no longer leaks into the package).
  `afk_viewer --version 26.1`.
- `ghast_physics::modern::ModernMemoryWorld::{min_y, height}`.
- **26.1 gliding (elytra) and firework rockets**, proven against the real client (oracle
  `elytra`, `tests/golden/26.1/elytra.jsonl.gz`; 1 M cached fuzz ticks, 0 diffs):
  `ghast_physics::modern::PlayerExt` (glide flag, usable glider, attached rockets, rocket arm)
  with `tick_ext` / `LoadGate::step_ext`, `PlayerExt::apply_shared_flags` (the server's shared
  entity flags: sprinting, swimming, gliding), `attach_rocket` / `remove_rocket`,
  `on_climbable_ext`, `Packet::StartFallFlying`. The glide uses `Math.cos` exactly as the
  HotSpot x86-64 JVM computes it (`modern::jdk_math`, a generated exception table).
  `ghast_data::v26_1::block_flags::CAN_GLIDE_THROUGH`.
- **26.1 riptide**, proven against the real client (oracle `riptide`,
  `tests/golden/26.1/riptide.jsonl.gz`; 1 M cached fuzz ticks, 0 diffs): `modern::riptide` (the
  launch on releasing the trident), `modern::in_water_or_rain`, `PlayerExt::{spin_attack,
  auto_spin_ticks, apply_living_flags, living_flags}` (spin counter and pose).
- **26.1 entity collision and pushing**, proven against the real client (oracle `entities`,
  `tests/golden/26.1/entities.jsonl.gz`; 1 M cached fuzz ticks, 0 diffs): `modern::Surroundings`
  with `tick_in` / `LoadGate::step_in` (boxes of collidable entities: in collisions, step-up,
  pose and edge tests, auto-jump), `modern::push_by` (`Pusher`, `PushKind`), `push_kind_26_1`,
  `collides_26_1`, `MAX_ROCKETS`.
- **26.1 nether movement** proven against the real client (oracle `nether`,
  `tests/golden/26.1/nether.jsonl.gz`; 1 M cached fuzz ticks, 0 diffs): fast lava flow
  (`LevelInfo::fast_lava`) and the 0-255 build height.
- **26.1 world border, moving and set**, proven against the real client (oracle `border`,
  `tests/golden/26.1/border.jsonl.gz`; 1 M cached fuzz ticks, 0 diffs): `modern::BorderState`
  (the client's tick-based lerp; `collision_border` lags it by a tick as vanilla's collision does).
- **26.1 boats the player rides and controls**, proven against the real client (oracle `boats`,
  `tests/golden/26.1/boats.jsonl.gz`, 112 cases; 1 M cached fuzz ticks, 0 diffs): `modern::Boat`
  (`BoatKind`, `BoatStatus`, `board`, `server_move`) and `modern::tick_boat` — the boat's tick
  (status, fluid pushing, floating, paddling with the previous tick's keys, collisions with blocks
  and entities, block effects), the player's tick as a passenger and its seat on the boat, with
  the `steer_boat`, rotation-only movement and `vehicle_move` packets in vanilla order.
  `Packet::{MoveVehicle, PaddleBoat}`.
- **26.1 riding a minecart** (a vehicle the server moves), proven against the real client (oracle
  `minecarts`, `tests/golden/26.1/minecarts.jsonl.gz`, 105 cases; 1 M cached fuzz ticks, 0 diffs):
  `modern::Seat` + `modern::tick_passenger` — the passenger's tick given the vehicle's position after
  its own tick (rotation-only movement packet, no `vehicle_move`), then its seat.
- `ghast-entity` 26.1 `Tracker::{tick_seq, tick_order}`: the client's entity tick order
  (`ClientLevel.tickingEntities`, also the insertion order `getEntities` lists within a section):
  entities enter when they spawn into, move into or have their column loaded, leave when removed or
  unloaded; the local player first at login / dimension change, last at a same-dimension respawn.
  Checked against the real client's tick list in `entity_tracking.jsonl.gz` (new `tick_order`
  field): 3,362 of 3,456 ticks equal; the rest differ because the tracker does not place passengers
  on their vehicles. The full membership re-check runs only after packets were applied or when
  an entity's interpolation moved it into another column; `Tracker::tick_early` ticks an entity
  ahead of the rest (a vehicle before its passenger, entities that tick before the player).
- **26.1 elytra in the client**: `TickContext::is_gliding` / `start_gliding`; the client glides
  with the oracle-proven `modern::PlayerExt` (glide start on a jump press in the air with a glider
  equipped, `START_FALL_FLYING`), applies its own shared flags from the server's entity data and
  the boost of firework rockets attached to it (decoded without entity tracking, only for our own
  id and rockets), and swings when a rocket is used while gliding. `EntityAction::StartFallFlying`.
  13 oracle elytra cases replay through the engine packet for packet; `lab/elytra.sh`.
- **26.1 riptide in the client**: releasing a riptide trident after at least 10 ticks in water
  or rain launches the player like the real client (oracle-proven `modern::riptide`, along the
  tick's look); a riptide trident has no use state out of water and rain; the rain comes from the
  weather game events, the spin pose from the server's living entity flags. 11 oracle riptide cases
  replay through the engine; `lab/riptide.sh`. Rain counts only where the biome rains (see the
  biome climate entry). Not known: touching an entity while spinning.
- **26.1 biome climate** (`ghast_world::climate`, oracle `biome-climate`,
  `tests/golden/26.1/biome_climate.jsonl.gz`): `Biome.getPrecipitationAt` with the height-adjusted
  temperature (the fixed-seed temperature noises, the frozen-ocean modifier, the snow line), the
  vanilla climate of every 26.1 biome, and `BiomeManager`'s zoom (`zoom_cell`); all positions and
  zoomed cells identical to the real classes. The 26.1 `World` keeps each biome registry entry's
  climate (from its data, or vanilla's for known-pack entries), the spawn info's hashed seed and sea
  level, and answers `biome_at`, `biome_climate`, `precipitation_at`; `World::set_biomes` turns
  biome keeping on or off at run time. The client keeps biomes once a riptide trident is held
  (columns loaded before that count as raining until resent); `WorldConfig::biomes` keeps them from
  the start.
- **26.1 inventory routing** for every predicted menu: configuration registry and tag bodies to the
  inventory (kept raw until a screen needs them), mount screens with the entity's type and saddle /
  body slots (from a minimal mount lookup that works without entity tracking),
  `TickContext::anvil_rename` / `select_loom_pattern` / `select_trade`.
- **26.1 boats in the client** (oracle-proven `modern::tick_boat`): `set_passengers` naming the
  player boards a boat or raft (a right click on it with the crosshair on it, or the server's
  `/ride`); while riding, the client moves the boat with the movement keys and sends
  `steer_boat`, the look, `move_vehicle` (and `player_input` / sprint changes) in vanilla order;
  the server's `move_vehicle` correction is answered at once; sneaking dismounts (server-side).
  Works without entity tracking for the bot's own boat (boats are known from their spawn and
  move packets, which are decoded only while some boat is known); other boats and mobs collide
  with it only with `Interests::ENTITIES`. The boat the player left stays where it was driven.
  `TickContext::vehicle` / `is_riding`, `Vehicle`, `Outbound::VehicleMove` / `SteerBoat`. 12
  hand-written and 100 fuzz cases of `tests/golden/26.1/boats.jsonl.gz` replay through the
  engine (72 of them also without entity tracking).
- **26.1 minecarts in the client** (oracle-proven `modern::tick_passenger`): `set_passengers`
  naming the player on a minecart seats it; each tick the cart ticks first (its interpolation
  of the server's moves, `Tracker::tick_early`), then the passenger on its seat, sending
  `player_input` and the look as vanilla. Without entity tracking a small tracker of the
  vehicles only (boats and minecarts) keeps the cart's position; it replaces the boats'
  spawn-position bookkeeping. Riding anything else (living mounts) keeps the walking player
  of 0.4.0 (no mount physics before 0.4.2). 5 hand-written and 100 fuzz cases of
  `tests/golden/26.1/minecarts.jsonl.gz` replay through the engine, with and without tracking.
- **26.1 entity collision and pushing in the client** (with `Interests::ENTITIES` only): the
  player collides with boats, rafts and living shulkers (`modern::tick_in`, boxes in
  `getEntities` section order), and after its tick the tracked mobs and boats push it
  (`modern::push_by`, each with its box after its own tick), as the real client does. 13
  hand-written and 9 fuzz cases of `tests/golden/26.1/entities.jsonl.gz` replay through the
  engine packet for packet. Without entity tracking the bot neither collides with nor is pushed
  by entities (no cost; a server-side anticheat may then flag walking into a boat or a crowd).
  Team collision rules (`set_player_team` `collisionRule`, decoded only with entity tracking:
  the team name, rule and members) decide each push like `EntitySelector.pushableBy`; other
  players' teams need `Interests::PLAYER_LIST` (their scoreboard names); the team rules are
  unit-tested against the decompiled predicate and lab-tested, not oracle-proven. Pushes and
  collision boxes follow the client's tick order (`Tracker::tick_seq`): entities that tick before
  the player (or before the vehicle it rides) tick first and push before its tick. Not
  modelled: an open or scaled shulker's box (the closed shell is used), the happy ghast; tick
  order of passengers (the tracker does not place them on their vehicles).

### Changed (behaviour)

- 26.1 world border collision follows the real client: the border's lerp advances once per
  client tick after the entities (`modern::BorderState` in the 26.1 `World`), and collision reads
  the size from before the last step; it used to move with wall-clock time. Oracle `border` and
  `nether` cases (4 + 43 fuzz each) replay through the engine.
- 26.1 `start_configuration` now follows `handleConfigurationStart`: a pending chat
  acknowledgement is sent before `configuration_acknowledged`, and the level (world columns,
  tracked entities) is dropped right away instead of at the next join.
- 26.1 commands whose parse reaches a `minecraft:message` argument (`/msg`, `/say`, `/me`,
  `/teammsg` …) are sent as `chat_command_signed` like vanilla (without signatures when there is
  no chat session, e.g. offline mode); other commands stay `chat_command`.
- A server transfer ends the session (`SessionEnd::Transferred`) instead of being ignored.
- 26.1 item use: whether an item has a use state comes from its components (`item_use`:
  duration > 0, food only below a full bar unless `can_always_eat`), the item's `use_effects`
  (`can_sprint`, `speed_multiplier`) drive the movement while using, and the use stops when the
  used hand no longer holds that item. The client writes the swings of a dropping click
  (`ContainerClick::swings`) before the `container_click`, and lets `update_recipes`,
  `abilities` and `experience` through to the inventory.
- 26.1 clicks: the changed slots are written in the real client's map order and each hashed
  stack's components in type id order (both are maps the server compares by content); bundle
  clicks, crafting results and the other menus above are predicted instead of sent
  unpredicted.
- The plain 26.1 tick (`modern::tick`) compiles to the code it had in 0.4.0 again: the extensions
  are a compile-time type (instructions per tick at or below 0.4.0's in the tick benchmarks).

### Fixed

- 26.1: the player kept still in a column that is not loaded; since the level-load gate the
  real client ticks it there too (the blocks read as void air).
- 26.1: `use_item` carried the previous tick's look instead of the rotation of the tick's
  movement packet (what the real client sends); Grim flags it as BadPacketsJ when an item is used
  while turning. 1.8.9 / 1.12.2 / 1.16.5 use packets carry no rotation.
- 26.1 world border collision: each bound is clamped to ±29,999,984 on both sides, and a border
  clamped to a sliver collides as `Shapes.INFINITY` (it blocked movement before).
- 26.1: a smithing table has 4 slots (template, base, addition, result), not 3; crafting grids,
  the crafter and result slots no longer cap stacks at 99 when the server sets them.
- `oracle/run.sh golden --out DIR` now writes into `DIR` (it used to ignore the flag and overwrite
  `tests/golden/<mc>/`).

### Known approximations

- Team collision rules: unit-tested against the decompiled predicate and lab-tested, not
  oracle-proven. Other players' teams need `Interests::PLAYER_LIST`.
- Shulkers collide as a closed shell (an open lid or a scaled shulker is not modelled); the
  happy ghast does not collide.
- Living mounts (horses, pigs, striders, camels …) are not simulated: the client keeps its walking
  physics while on one (0.4.2).
- The tick order of passengers (the tracker does not place them on their vehicles).
- Biome lookup inside a loaded column and sky light for rain are not oracle-proven (the climate,
  temperature noise and zoom are).
- Incoming signed chat messages are not verified; the online-mode path is tested against a local
  stand-in of the account services, not real accounts.
- 26.1 riptide: touching an entity while spinning is not modelled.

### Memory

Idle AFK bot, x86-64, release profile, 2026-11-12 (kB; 0.4.0 → 0.4.1, two runs each):

| build | VmRSS | private dirty |
|---|---|---|
| all versions, on 26.1 | 7,296 → 8,028–8,140 | 2,292–2,328 → 2,216–2,220 |
| all versions, on 1.8.9 | 6,912–7,168 → 6,912–7,296 | 2,452 → 2,316 |
| 26.1 only (`--no-default-features --features v26_1,libdeflate`) | — → 6,436–6,444 | — → 1,788–1,792 |
| 1.8.9 only (`--features v1_8,libdeflate`) | — → 4,992 | — → 1,616 |

Per-process private memory is lower than in 0.4.0: the inventory and data tables no longer hold
load-time relocations, the idle heap keeps less. Resident shared code is higher (~0.9 MB on 26.1
with every version compiled in: the code of this release's mechanics); it is file-backed and
shared between processes running the same binary, and a single-version build avoids it. A
non-PIE build (README) lowers private memory further (about 1.4 MB).

## 0.4.0 — 2026-11-11

Minecraft 26.1 (protocol 775) is the fourth supported version, built on a shared
infrastructure that is meant to carry every later version: one generated definition per packet
layout shared across versions, a version table, capability rows in the client instead of
version checks, and one cargo feature per version in every crate. Block-breaking time and the
crosshair pick now come from tables and code proven against each version's real client, which
changes packet timing on several blocks (see *Behaviour changes*). The item model is shared:
a stack carries legacy NBT or 26.1 data components behind one type.

### Breaking changes / migration

See *Migrating from 0.3* in the README for a short checklist; the full list:

- **Cargo features per version** in `ghast`, `ghast-client`, `ghast-protocol`, `ghast-data`,
  `ghast-world`, `ghast-physics`, `ghast-entity` and `ghast-inventory` (default: every
  version). With `default-features = false`, enable the versions you use: `v1_8`, `v1_12_2`,
  `v1_16_5`, `v26_1` on `ghast` / `ghast-client` / `ghast-protocol`; `v1_8`, `v1_12`,
  `v1_16_5`, `v26_1` on the data / world / physics / entity / inventory crates. A
  `GameVersion` whose feature is off returns `ClientError::VersionNotEnabled`.
  `ghast-ext` (`v1_8`, `v1_12_2`, `v1_16_5`, `v26_1`) and `ghast-pathfinder` (`v1_8`, `v1_12_2`,
  `v1_16_5`) have them too (default: all); each publishable crate builds on its own with any one
  version (checked by `scripts/feature-matrix.sh` in CI).
- **ghast-pathfinder `Version`**: the variants exist only with their feature, and
  `From<GameVersion>` is replaced by `Version::from_game(v) -> Option<Version>` (it mapped 26.1
  to 1.16.5); the `Follower` fails with the new `FollowError::UnsupportedVersion` on 26.1.
- **ghast-ext** item / block / entity names and ids on 26.1 use the 26.1 tables (they used the
  1.16.5 ones).
- **`display-names` feature** (default on) in `ghast-data`, `ghast-client` and `ghast`: the
  English display-name fields of the data records exist only with it. Registry names are
  always there. Builds with `default-features = false` that read display names must enable it.
- **ghast-inventory `Stack`**: the fields are private. Read them with `id()`, `count()`,
  `damage()`, `data()` / `nbt()`; build stacks with `Stack::new(id, count, damage)` and
  `with_nbt`, `with_damage`, `with_count`, `with_data`. The NBT field became
  `data: ItemData` (`ItemData::None`, `ItemData::Nbt(RawNbt)`, and with `v26_1`
  `ItemData::Components(..)`); `stack.nbt()` still returns `Option<&RawNbt>`.
  `stacks_equal` / `nbt_eq` are unchanged.
- **`Inventory::new` returns `Result<Inventory, VersionNotEnabled>`** (it panicked for a
  version whose feature is off): `Inventory::new(v)?` or `.expect(..)`.
- **`Outgoing::to_v1_8` returns `Option`** like `to_v1_12_2` / `to_v1_16_5`; `None` for the new
  26.1 `Outgoing::ContainerClick`.
- **ghast-protocol packet types are type aliases** of shared generic definitions
  (`v1_8::play::serverbound::WindowClick` = `defs::p47::play::serverbound::WindowClick<47>`).
  Struct literals, patterns, `decode_as::<cb::X>()` and `write_packet(&sb::X { .. })` work
  unchanged; docs and error messages show the `defs::…` path. A nested or named type that is
  identical in two versions is now one Rust type, so a downstream trait implemented for both
  would conflict.
- **`ghast_protocol::Version` is sealed** (only ghast-protocol's version markers implement it,
  so later versions can add items without a break) and has new items (`INFO`,
  `ConfigurationClientbound`, `ConfigurationServerbound`); `State::Configuration` is new, so
  `State::Play as u8` changed (do not rely on such casts).
- **ghast-client**: `MovePacket` gained `horizontal_collision` in every variant and is
  `#[non_exhaustive]`; `Teleport` gained `relative` and `delta` (and `Default`);
  `Inbound::Respawn` gained `keep`; new `Inbound` / `Outbound` variants and the new
  `ConfigInbound`, `ConfigOutbound`, `KnownPack`, `AttributeUpdate`,
  `LoginInbound::CookieRequest`; `ChatMessage` has a private field (build it with
  `ChatMessage::from_json` / `from_text`); `BlockQuery` / `HeldItem` gained crate-private fields;
  `WorldView` gained a defaulted `crosshair` method. The sealed `ClientVersion` trait changed
  (not implementable outside the crate). `hands::pick_entity` is gone (the crosshair is one
  block-and-entity pick now, `TickContext::target` / `target_entity`, built on
  `ghast_physics::pick::<version>::pick_target`), and so is `Capabilities::entity_pick_border`.
- **`#[non_exhaustive]`** so that these types can grow in patch releases. A `match` on one of
  the enums needs a `_ =>` arm; the structs are read as before but cannot be built with a
  struct literal outside their crate:
  - enums the library returns or that will grow: `ghast_inventory::{Click, ClickOutcome}`,
    `ghast_world::Applied`, `ghast_inventory::{ItemText, EnchantmentId}`,
    `ghast_world::v26_1::BiomeSection`, `ghast_world::paletted::Section`;
  - output-only structs: `ghast_inventory::{ClickPacket, Events, ContainerClick, Enchantment,
    VersionNotEnabled}`, `ghast_world::{Border, v26_1::{Registries, Dimension,
    DimensionType}}`, `ghast_protocol::frame::DecoderStats`,
    `ghast_client::version::Capabilities`, `ghast_protocol::versions::ProtocolVersion`.
  - `ghast_inventory::ClickResult` is `#[non_exhaustive]`; build one with the new
    `ClickResult::new(window_id, action, outcome)` (tests and replays).
- New variants in already open enums: `GameVersion::V26_1`, `ghast_inventory::Version::V26_1`,
  `ghast_protocol::State::Configuration`, `Outgoing::ContainerClick`.

### Behaviour changes

- **Block-breaking time is oracle-proven on every version.** The client computes per-tick
  destroy progress with each version's own tables and code, taken from the vanilla jar and
  replayed bit for bit against the real client (`ghast_data::{v1_8, v1_12, v1_16_5,
  v26_1}::dig`); 0.3 used minecraft-data's hardness / tool tables, which are wrong for some
  blocks. Changed break times (empty hand, diamond tools, shears):
  - 1.8.9 (25 block / tool pairs): mushroom blocks are no longer instant (6 ticks); cactus with
    an axe or sword is slower; snow layers (20 → 10 ticks) and lily pads (instant) are faster.
  - 1.12.2 (187 pairs): were too fast in 0.3 — shulker boxes without a pickaxe (60 → 200
    ticks), double stone slabs and non-oak fence gates (were instant), mushroom blocks, cactus
    with an axe, leaves with an axe, hardened clay with a shovel, grass paths; the moving piston
    is unbreakable. Were too slow — a pickaxe on purpur, end bricks, magma, red nether brick,
    bone blocks, observers, shulker boxes and hardened clay; ladders and chorus with an axe;
    leaves with shears; snow layers.
  - 1.16.5 (233 pairs): were too fast in 0.3 — stone-type stairs and slabs, the blackstone
    family, bells and stonecutters without a pickaxe (45 → 150 ticks by hand), wooden slabs and
    pressure plates with a pickaxe, the conduit (12 → 90), leaves with an axe. Were too slow —
    a pickaxe on shulker boxes, purpur, bone blocks, blue ice and blackstone (45–84 → 6–11
    ticks); conduit power is now applied.
  A break that used to finish too early could trip an anticheat's fast-break check; these now
  match vanilla.
- **Crosshair picks are oracle-proven on every version** (`ghast_physics::pick`): the block
  and the face / cursor position sent with `use_item_on` / `block_dig` follow vanilla's ray
  shapes. Changed against 0.3: 1.8.9 — the moving piston (`piston_extension`) is no longer
  targetable; 1.12.2 — stairs (each part box), flowers (model offset), connected redstone wire
  and the moving piston; 1.16.5 — every non-cube block now uses vanilla's outline and
  interaction shapes instead of a union box (full cubes unchanged). `ghast_physics::pick` also
  picks entities with vanilla's entity loop (all four versions, oracle-proven), and the client
  uses one pick for block and entity like `GameRenderer.updateTargetedEntity`: an entity hit
  beyond 3 blocks but before the block is now a miss (it used to dig the block behind it), and
  entity boxes grow by their type's targeting margin (`Entity::targeting_margin`). The pick is
  computed only on ticks with a mouse button down or when a behaviour asks for the target, at
  the same point and from the same state as before, so packets do not change.
- **Disconnect**: quitting half-closes the socket and drains it like vanilla, so servers log
  "Disconnected" instead of "Connection reset by peer".

### Minecraft 26.1 (protocol 775)

26.1, 26.1.1 and 26.1.2 (same protocol, identical oracle records): `GameVersion::V26_1`,
`--version 26.1.2`. Lab: Paper 26.1.2 + Grim 2.3.74, 0 flags (AFK, movement course, chat,
dig + place, chest clicks).

- **ghast-protocol**: `v26_1` (feature `v26_1`), all five states including configuration
  (known packs, registry data, resource packs, cookies; reconfiguration not tested live),
  257 packets; data
  components in slots, read lazily (`types::slot::v775`: zero-copy component patch, structural
  skip); `nbt::AnonNbt`, `types::{Holder, HolderSet, LpVec3, chunk}`; the version table
  `ghast_protocol::versions`. Checked against a live server capture (every packet decodes
  strictly and re-encodes identically) and Mojang's packet report.
- **ghast-data**: `v26_1`: block states, voxel shapes, movement records, items, dig facts and
  entity sizes from the 26.1 jar; property maps checked against Mojang's block report.
- **ghast-world**: `v26_1::World`: dimension heights from the configuration registries,
  power-of-two packed sections (about the wire size in memory), chunk batches with vanilla's
  batch-size calculator, block entities, the world border; decoded worlds checked block by
  block against a second decoder and against a live server.
- **ghast-physics**: one modern physics core with the 26.1 rule set (attributes, powder snow,
  supporting block, teleports with delta movement), 0 diffs against the 26.1 oracle.
- **ghast-inventory**: windows and clicks with hashed stacks (vanilla `HashedStack`), every
  component kind a client can receive hashed like the real client (4000 oracle stacks
  byte-identical; a live session of 129 stacks drew no correction); click results by settle
  ticks / corrections (26.1 has no confirm); item accessors over Mojang's item defaults.
- **ghast-client**: 26.1 sessions end to end (configuration, chunk batch acks, player input,
  tick end, block sequences), the oracle-verified pick with vanilla's eye lerp and interaction
  range attributes, entity tracking with entity picks and attacks. The level-load gate
  (`player_loaded`) is `ghast_physics::modern::LoadGate`, replayed against the oracle's 27 joins (incl. joining dead);
  it opens once the player's column and the eight around it have been loaded for two ticks
  (vanilla's first section compile needs them). Opening it on the player's own column, as an
  earlier build did, drew Grim flags when logging in above the ground.
- Not yet on 26.1: path finding, the viewer, signed chat, online mode (untested).

### Added in the other crates

- **ghast-entity**: `v26_1::Tracker` (feature `v26_1`): every 26.1 entity packet parsed by hand,
  the client's entity tick (interpolation steps, head lerps, body rotation, deferred resizes,
  chunk ticking), passengers, tab list; replayed against the real `ClientPacketListener` at
  0 diffs. `Tracker::mark_column_loaded` / `mark_column_unloaded` for a tracker started late,
  `Entity::targeting_margin`. Entities of the older versions stay 232 bytes with 26.1 compiled
  in.
- **ghast-data**: per-version `dig` modules (`v1_8`, `v1_12`, `v1_16_5`, `v26_1`): destroy
  progress from the jar's block and tool tables; selection / outline / interaction shapes for
  the crosshair (`v1_8::Block::selection`, 26.1 `outline_shape`, `interaction_shape`, …); 26.1
  posed entity dimensions; the `display-names` feature.
- **ghast-physics**: `pick` (crosshair rays and entity picks for every version), `modern` (the
  26.1 player core, `modern::LoadGate` for the level-load gate, `modern::no_block_collision`),
  one feature per version.

### Shared infrastructure

- **ghast-protocol codegen** emits each packet layout once and shares it between versions with
  the same layout (−20 % generated code before 26.1); a JSON-pointer patch layer corrects
  minecraft-data where it disagrees with vanilla (each patch asserts the value it replaces, so
  a stale patch fails the build).
- **ghast-client** reads a capability row per version (`GameVersion::caps()`) instead of
  comparing versions; engine cost per tick unchanged.
- **Item model** (`ghast_inventory::ItemData`): one `Stack` for NBT and components; accessors
  (`item_name`, `max_stack_size`, `max_damage`, `damage_value`, `unbreakable`, `custom_name`,
  `lore`, `enchantments`, `potion`, …) answer per version.

### Performance

- 1.8 / 1.12 `window_items` compares each slot before converting it (−17 % instructions on a
  90-slot shop window), and the 1.8 crafting-grid result is memoised per grid (a player-window
  `window_items` 142k → 9k instructions).
- The client no longer links decoders and encoders of packets it never handles (−5 % binary).
- `afk` example, stripped, release profile: 5.3 MB with every version; 1.9 MB with 1.8.9 only,
  1.9 MB 1.12.2, 2.6 MB 1.16.5, 3.3 MB 26.1.

### Verification (at release time)

- Cached oracle fuzz: 16.9 M ticks, 0 diffs (1.8.9 4.16 M, 1.12.2 4.16 M, 1.16.5 4.40 M,
  26.1 4.20 M); crosshair: 400 000 cached rays (1.16.5, 26.1) and 240 000 cached picks (1.8.9,
  1.12.2) bit-identical; click vectors for 1.8.9 / 1.12.2 / 1.16.5 with 0 diffs; 26.1 entity
  tracking against the real client: 51 728 ticks (211 362 packets, 8.96 M fields) equal; the
  27 oracle joins through the level-load gate bit-identical; 26.1 click hashes: 4000 oracle
  stacks byte-identical.

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
