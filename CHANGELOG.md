# Changelog

All crates share one version number.

Versioning: until 1.0, a **patch** release (0.y.Z) adds features and fixes without breaking the
public API; a **minor** release (0.Y.0) may break it and says how to migrate. 1.0 comes only when
the API has been stable for a while and the 1.0 criteria in CONTRIBUTING.md are met.

## 0.4.1 — unreleased

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
- **26.1 creative inventory screen**: window 0 opened in creative mode sends what the real
  screen sends (`Outgoing::Creative(Vec<CreativePacket>)`: `set_creative_mode_slot` for the
  slots that differ from the last broadcast, drops as slot -1 with the arm swing; echoes of
  server slots from `take_creative_echo_26_1`; the drop spam throttle), proven against the
  real screen (`tests/golden/26.1/clicks_creative_seed1.jsonl.gz`, 19 381 packets).
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
  names and a `standIn` flag; `HELLO` names the game (`game`, `drawnWith`, `worldMinY`). 1.8.9
  payloads are byte-identical to 0.4.0. `BotView::build_height` (defaulted).
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
- `oracle/run.sh golden --out DIR` now writes into `DIR` (it used to ignore the flag and overwrite
  `tests/golden/<mc>/`).

### Changed (behaviour)

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
  unpredicted; a 26.1 smithing table has 4 slots (template, base, addition, result), not 3;
  crafting grids, the crafter and result slots no longer cap stacks at 99 when the server
  sets them.

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
