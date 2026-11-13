# ghast-client

Headless Minecraft Java Edition client for the ghast library: connection, login, the fixed
50 ms tick loop and a version-agnostic bot API. Supports **1.8.9** (protocol 47),
**1.12.2** (protocol 340), **1.16.5** (protocol 754), **26.1 – 26.1.2** (protocol 775) and **26.2**
(protocol 776). Each version is a cargo feature (`v1_8`, `v1_12_2`, `v1_16_5`, `v26_1`, `v26_2`;
default: all) and builds
alone; what a version can do is its `version::Capabilities` row (`GameVersion::caps()`), which
the core reads instead of comparing version numbers.

```rust,no_run
use ghast_client::connect::{Connector, ProxyPolicy};
use ghast_client::{Account, Behavior, Client, GameVersion, Interests, TickContext};

struct Afk;
impl Behavior for Afk {
    fn interests(&self) -> Interests { Interests::CHAT }
    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        for m in &ctx.events().chat { println!("{}", m.plain()); }
        if ctx.is_dead() { ctx.respawn(); }
    }
}

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let connector = Connector::new()
    .with_proxy("socks5h://user:pass@proxy:1080".parse()?)
    .with_policy(ProxyPolicy::RequireProxyExceptLocal);
let client = Client::new("mc.example.net:25565".parse()?, GameVersion::V1_8_9, Account::offline("Bot")?)
    .connector(connector);
let end = client.run(&mut Afk).await?; // Kicked / ServerClosed / Quit / Shutdown
# Ok(()) }
```

Examples (facade crate): `cargo run -p ghast --release --example afk -- --host … --port …
--username … --version 1.8.9|1.12.2|1.16.5 [--proxy socks5h://…] [--minutes N] [--reconnect]`, and
`--example chat` (stdin lines → chat).

## Layers

| module | what |
|---|---|
| `client` | `Client` (config + `run` / `run_until`), connect → handshake → login → play loop |
| `version` | `ClientVersion` (sealed) for `V1_8` / `V1_12_2` / `V1_16_5`: login and play packet mapping, packet filter, teleport rules, chat limit |
| `common` | version-agnostic `Inbound` / `Outbound` packets and the API types (`Pose`, `Controls`, `ChatMessage`, …) |
| `engine` | the deterministic core (no I/O, no clock): state, `Behavior`, `TickContext`, the tick |
| `sim` | per version: the `ghast-world` chunk store + the mover (`ghast-physics` per version); `WorldView` for behaviours |
| `movement` | vanilla movement-packet choice (`MovementSender`) and a stand-still mover (no longer used by the client; kept for tools) |
| `hands` | dig / place / use state machine, vanilla 1.8 `World.rayTrace` crosshair |
| `record` | movement recorder in the oracle's replay format (`Client::record`) |
| `rotation` | whole-mouse-count rotation (sensitivity GCD, Java float maths) |
| `interest` | `Interests`: optional packet groups a behaviour opts into |

Dispatch is compile-time: `Client::run` matches the run-time `GameVersion` once and runs a
session monomorphised for that version; the engine and the behaviour are version-free.

## Tick model (1.8.9 `MinecraftClient.tick`)

1. Inbound packets are applied **on arrival** (like vanilla's per-frame task queue, which
   runs before each tick). Keep-alives, teleports (1.16: `teleport_confirm` +
   `position_look`; 1.8: `position_look`, `on_ground = false`) and window confirms
   (`transaction` for window 0 or the open window, when not accepted) are answered at once.
   Before a tick the loop also drains whatever the socket already holds.
2. `Behavior::tick(ctx)` (`start` first on the first tick; not called again once
   `Behavior::running()` is `false`). Actions (`chat`, `select_slot`, `place`, `dig`,
   `attack`, clicks, `swing`, `respawn`) are written in call order before this tick's
   movement packet; `set_aim` / `set_controls` / `set_buttons` / `set_flying` only record
   intent.
3. Held-slot sync (`ClientPlayerInteractionManager.tick`) → key processing (hands: held
   buttons) → `set_flying`'s `abilities` → rotation step (only on ticks the behaviour aimed;
   held back for one tick when it would take the crosshair off a block dug or placed this
   tick, see below) → movement step →
   sprint `entity_action` → sneak `entity_action` → **exactly one** of `flying` / `position` /
   `look` / `position_look` (moved > 0.03, or the position refresh: 1.8 checks
   `ticksSinceLastPositionPacketSent >= 20` before incrementing it — every 21st tick — while 1.9+ increments
   first — every 20th tick; Grim's BadPacketsE flags a 21-tick gap on 1.16.5).
   `ctx.sent()` is the pose the server has.

The behaviour only runs once the player has its first server position and its chunk column
is loaded (1.16 vanilla does not tick the player outside a loaded column; on 1.8 we wait as
well). Nothing is sent before the first position packet.

The tick schedule is a tokio interval with `MissedTickBehavior::Skip`: after a stall the
missed ticks are skipped, never caught up (a burst of movement packets trips timer checks).

**Silent-server guard:** after `silent_hold` (1.5 s) without inbound bytes the behaviour is
frozen (keys released, events kept) while the vanilla movement packet continues; it resumes
when data arrives. After `read_timeout` (30 s, vanilla's) the session fails with
`ClientError::Timeout`.

**Robustness:** decoding never panics (ghast-protocol); a packet the core needs that does not
decode ends the session with `ClientError::Decode` (vanilla disconnects too); malformed chunk
data (e.g. a section mask the data does not cover) only drops that column, is logged
(rate-limited) and play goes on, as vanilla's task runner does — `GHAST_DUMP_BAD_CHUNK=<dir>`
writes the first rejected bodies there for a bug report; a behaviour
panic is caught, the behaviour is stopped and the player keeps standing; sessions share no
state, the session future is `Send` for `Send` behaviours, so many bots can run on one
multi-threaded runtime or one bot per process on a `current_thread` runtime (examples).

## Modern versions: one implementation, one row per version

The modern client (26.1 and later) is written once: `src/version/modern.rs` (the wire layer:
login, configuration, play packets, chat signing, command trees) and `src/sim/modern.rs` (world,
movement, vehicles, entities, digging) are mounted once per version by a small row module
(`version/v26_x.rs`, `sim/v26_x.rs`) that names the version's protocol, data, world, entity,
inventory and physics modules and its few true facts (entity data indices, effect ids, the team
packet layout, the play login's online-mode flag). Each version gets its own monomorphised copy:
no run-time version branches. Ids that versions renumber (entity types, attributes, items,
command parsers) are resolved by name. The engine replays of the oracle goldens
(`src/engine_modern_tests.rs`) run for every modern version. Adding a version: see
CONTRIBUTING.md ("Adding a modern version").

## 26.2 (protocol 776)

26.2 is the 26.1 row with its own modules. True deltas: the play login's `onlineMode` decides
whether a chat session starts (26.1: an encrypted connection); the team parameter order
(display name, prefix, suffix, visibility, collision rule, optional colour, options); 40
renumbered attributes, three of them new for movement (`bounciness`, `friction_modifier`,
`air_drag_modifier`: fed to the physics, the extended tick runs while any is not at its
default); collision restitution in the physics (`V26_2`: slime and bed bounces, signed zeros).
The login's session id is telemetry in the vanilla client and is not used; `spectator_action` is
never sent. Proven: every 26.1 engine replay also runs on `tests/golden/26.2/` (joins, teleports,
elytra, riptide, entities, border, nether, boats, minecarts, item use, entity pick,
`player_loaded`, signed chat, login encryption) plus the idle slime / bed drop scenarios
(8500 ticks to the end of the bounce tail), packet for packet.

## 26.1 (protocol 775)

Read from the decompiled 26.1 client (`Minecraft.tick`, `LocalPlayer.tick` / `sendPosition`,
`ClientPacketListener`, `ClientConfigurationPacketListenerImpl`, `MultiPlayerGameMode`):

- **Login → configuration → play.** After `login_success` the client sends
  `login_acknowledged`, its brand and client information (in the configuration state, not on
  join), answers keep-alives and pings, selects the server's `minecraft` packs of the 775 family
  as known (`core` and the feature packs; the server then sends vanilla registry entries without
  data), keeps the registries the world needs (dimension types with their height, biome count;
  chat types and enchantment names for the wire layer), answers cookie requests (stored cookies
  or none), accepts the code of conduct, reports optional resource packs declined and required
  ones accepted + loaded (`Client::accept_required_resource_packs`, nothing is downloaded), and
  enters play on `finish_configuration`.
- **Reconfiguration** (`start_configuration` in play, e.g. a proxy switching backends):
  like `handleConfigurationStart`, the pending chat acknowledgement goes out first, then
  `configuration_acknowledged`; the level is dropped (world columns, tracked entities, the
  player; cookies stay), the configuration state runs again with the new server's registries
  (a different dimension height takes effect with the next join), no ticks meanwhile.
- **Transfers** (`transfer`, configuration or play state): the session ends with
  `SessionEnd::Transferred { host, port }`; with `Client::follow_transfers(true)` the client
  logs in to the target with the transfer intent and the stored cookies inside the same
  `run` (the behaviour stops and starts again; at most 8 in a row).
- **Online mode**: encryption (also `shouldAuthenticate = false`: no session join, any
  account) and the session-server `join` with `Account::Microsoft`; the services' URLs are
  `ghast_auth::microsoft::Endpoints` (a local stand-in for tests: `Endpoints::with_base_url`).
- **Signed chat** (encrypted connections, `Client::signed_chat`, default on): the profile key
  pair is fetched once per `Client` (`player/certificates`, refreshed when due; or given with
  `Client::profile_keys`), each join starts a chat session (random id) announced with
  `chat_session_update` from the connection tick, and chat messages and the `minecraft:message`
  arguments of commands are signed (SHA256withRSA over the 26.1 signed body: link, salt,
  timestamp, content, last-seen signatures). Which commands are signed comes from a
  Brigadier-style parse over the server's `declare_commands` tree (kept raw, parsed only when a
  command is sent): selectors need permission level 2, arguments behind a redirect to the root
  (`execute … run say`) are not signed; such commands go out as `chat_command_signed` (without
  signatures when there is no chat session), the rest as `chat_command`. Offline connections do
  no signing work.
- **Tick order** (one client tick): held slot sync → `player_loaded` (once per level, from the
  connection tick inside `gameMode.tick`) → key handling: dig / place / use / attack, written
  immediately → `LocalPlayer.tick`: abilities (flight toggle), `player_input` when the keys
  changed, sprint `player_command`, the movement packet (`on_ground` and
  `horizontal_collision` flags; squared threshold 2e-4, a position at least every 20 ticks) →
  `tick_end`. So the server still judges a tick's actions with the previous tick's sent pose.
- **Level loading** (`ghast_physics::modern::LoadGate`, proven on the oracle's
  `tests/golden/26.1/joins.jsonl.gz`, which the engine replays packet for packet). The player
  does not tick until the level is loaded: after the server's `LEVEL_CHUNKS_LOAD_START` game
  event, once the player's section is "compiled" (spectators, dead players and feet outside the
  build height at once; after 30 s anyway). Until then each tick sends only `tick_end` (position
  packets are still answered; only the entity tick counter runs). Vanilla compiles the player's
  section once its column and the eight around it are loaded (`RenderSection.hasAllNeighbors`),
  in a frame after that tick, so the engine's "compiled" signal is **the nine columns loaded on
  two consecutive ticks**. The gate tick sends `player_loaded`, the first movement packet
  (position/look, on-ground false, no mouse turn on that tick) and `tick_end`. Opening the gate
  earlier (on the player's own column) drew Grim Simulation + GroundSpoof flags when the saved
  position floated above the ground (`lab/airborne-join.sh`). A respawn starts over (with the
  respawn packet's kept data: attributes, last input and sprint state).
- **Wire layer** (`Wire26_1`, per connection): block action sequence numbers (start and finish
  digging, `use_item_on`, `use_item` take a new one; abort and release send 0; reset per level),
  `use_item` with the current look, chat with the last-seen acknowledgement state (offset,
  20-bit set, checksum; `message_acknowledgement` after 64 pending signed messages), the chat
  session and command tree above, the permission level (entity events 24–28), cookies, chunk
  batch answers with vanilla's chunks-per-tick estimate.
- **Item use**: the use state, food gating and the movement slowdown come from the used stack's
  components (`ghast_inventory::v26_1::item_use`: duration, `use_effects`, `food`); the server's
  entity event 9 completes a use (the client never does it itself); clicks that drop a stack
  send their swings first.
- **Minecarts** (26.1): seated by the server's `set_passengers`; the cart moves as the server
  says, the player sits on it (`vehicle()` reports it with `controlled: false`). Living mounts
  (horses, pigs, striders, camels …) are not simulated yet: the client keeps its 0.4.0 walking
  physics while on one.
- **Boats** (26.1): boarded when the server's `set_passengers` names the player (right-click a
  boat with the crosshair on it, or `/ride`); `TickContext::vehicle` / `is_riding`. While riding,
  the movement keys paddle (the client moves the boat, as vanilla); sneak to get off. The own
  boat needs no entity tracking; collisions with other boats and mobs need
  `Interests::ENTITIES`.
- **Elytra and riptide** (`ghast_physics::modern::PlayerExt`): glide starts on a jump press in the
  air with a glider equipped (`TickContext::start_gliding`, `is_gliding`), the server's own shared
  and living entity flags, firework rockets attached to the player and riptide launches are applied
  as the real client does; the own-entity packets are decoded only for the player's id, attached
  rockets and mounts (no entity tracking needed). `use_item` carries the rotation of the tick's
  movement packet. Riptide in rain needs the biome to rain at the feet or head block
  (`Level.isRainingAt`): biomes are kept from the first time a riptide trident is held (columns
  that arrived before count as raining until the server resends them); turn on
  `WorldConfig::biomes` with `Client::world_config` to have them from the start. Sky light is not
  modelled (the column above must be free of motion-blocking blocks and fluids).
- **Physics** is `ghast_physics::modern` (`V26_1`), fed every movement attribute in packet order,
  effects (weaving too), abilities, teleports with all nine relative bits and their delta,
  `LpVec3` motion, explosions, server rotations (answered with a look packet), leather boots.
  **Digging** uses `ghast_data::v26_1::dig` (oracle-verified) with the server's
  `mining_efficiency`, `block_break_speed` and `submerged_mining_speed` attributes. The
  crosshair is the oracle-verified 26.1 pick (blocks and the tracked entities).
- **Entities** (opt-in, `Interests::ENTITIES`): `ghast_entity::v26_1::Tracker` gets every packet
  it wants (also the `map_chunk` / `unload_chunk` the world reads) and ticks once per client tick
  after the pick (interpolation, with the block / border collision test); `entities()`,
  `target_entity()` and `attack()` work as on the older versions. With it the player also
  collides with boats, rafts and shulkers and is pushed by the mobs and boats around it, as the
  real client does. **Without `Interests::ENTITIES` neither happens**: a bot that walks into a
  boat or through a crowd then moves differently from a vanilla client (an anticheat can flag
  it); turn the interest on where that can happen. Team collision rules (`never`,
  `pushOtherTeams`, `pushOwnTeam`) are followed; other players' teams need
  `Interests::PLAYER_LIST` (unit- and lab-tested, not oracle-proven). Pushes follow the
  client's entity tick order. Not modelled: open or scaled shulkers (the closed shell is used),
  the happy ghast.
- **Chat** arrives as NBT components: system chat, player chat (rendered with the vanilla
  pattern of its chat type: `<name> message`, whispers, `/say`, `/me`) and disguised chat.
- **Proven against the real 26.1 client** (oracle vectors, `cargo test`):
  `tests/golden/26.1/chat_signing.jsonl.gz` — `chat_session_update`, signed and unsigned
  `chat_message`, `chat_command` / `chat_command_signed` (which arguments are signed, the
  signatures, the bytes) and acknowledgements, byte-identical; `net_crypto.jsonl.gz` — session
  hash, the `hello` packet, the key packet both ways, AES/CFB8 both directions.
- **Not yet on 26.x**: incoming signed messages are not verified (a message vanilla would
  reject as invalid still enters the last-seen list), movement recording,
  `WorldView::column` (the 26.1 column type differs). Not verifiable without a real account:
  the real services' responses (`player/certificates` format and key sizes as documented,
  `join` errors) — the lab uses a local stand-in that follows authlib's parsing.

## Packet filter

The play-state filter is built per version from the core's needs plus the behaviour's
`Interests` (re-read every tick). Everything else — tab list, teams, scoreboards, particles,
sounds, entities, time, recipes, tags, commands — is dropped by the frame decoder before it is
decoded (after inflating only the id when compressed).

Core set: keep_alive, login, update_health, respawn, position, held_item_slot,
game_state_change, open/close window, transaction, abilities, kick_disconnect, the world
(`ghast_world::v1_*::PACKET_IDS`: chunks, block changes, explosion, world border; 1.8
map_chunk_bulk; 1.16 unload_chunk, update_view_position / distance), entity_velocity (1.8 knockback),
set_compression (1.8 play), combat_event (1.16 immediate respawn). `Interests::CHAT` adds chat.
`Interests::BLOCK_UPDATES` copies the block changes the world applies into
`TickEvents::block_updates` (position, raw state; at most 4096 between two behaviour ticks,
the rest counted in `block_updates_dropped`) — no extra packets, only the copy.

**Raw tap:** `Behavior::raw_packets(version)` names extra clientbound play packet ids; each such
frame — also one the core consumes itself (position, held slot, abilities, …) — is copied
undecoded (`Bytes` clone) into `TickEvents::raw` (`RawInbound { id, body, at }`, arrival
order, kept while the behaviour is on hold, at most 4096 per tick, the rest counted in
`raw_dropped`). The core's own handling does not change; a tapped id the core handles
optionally (chat) is then handled too. Read every tick; a change rebuilds the filter.

## Vanilla details reproduced

- Join: `settings` then the brand plugin message (`MC|Brand` / `minecraft:brand`, `vanilla`).
- Teleport: relative flags; absolute axes zero that motion; 1.8 `yaw % 360`, `pitch % 360`;
  1.16 x/z clamped to ±3e7, pitch clamped to ±90 then `% 360`. `lastReported*` untouched, so
  the next tick sends the position again.
- A fresh player (join / respawn) has `lastReported* = 0`.
- Server `held_item_slot` changes only the selection; the next sync echoes it (vanilla).
- Chat truncated to 100 (1.8) / 256 (1.16) UTF-16 units like the vanilla packet constructor.
- 1.16 immediate respawn (game state 11 / `combat_event` entity dead) respawns at once.
- Login plugin requests (1.16) are answered "not understood".

## Movement and world

- **1.8.9**: `ghast-physics` (bit-exact against the vanilla oracle) runs the movement step on
  the `ghast-world` chunk store and produces the tick's sprint / sneak / abilities packets and
  the movement packet. Server teleports, abilities, food (sprint), our `entity_velocity` and
  explosion pushes are fed to it; the world border (moving borders advanced 50 ms per tick)
  reaches the collision code through the world. `Controls` are the vanilla keys (W/A/S/D, jump, sneak,
  sprint).
- **1.12.2**: `ghast-physics::v1_12` (0 diffs against the 1.12.2 oracle) on the 1.12.2 chunk
  store. A fresh player is added to its chunk on the first tick its column is loaded and moves
  from the next one, as vanilla 1.9+ does; standing still sends no idle packets (only the
  position refresh every 20 ticks). Recording is not available on 1.12.2 yet.
- **1.16.5**: `ghast-physics::v1_16_5` (0 diffs against the 1.16.5 oracle) on the 1.16.5 chunk
  store, fed the same way (teleports with relative flags, abilities, food, velocity). Its packet
  rules are 1.16's: teleport confirm + `position_look`, no packet at all on idle ticks
  (`flying` only when on-ground changes), a position at least every 20 ticks.
- `ctx.world()` (`WorldView`): loaded columns and raw block states; `None` = not loaded.

## What the physics is fed

Teleports (relative flags), abilities, food, our entity velocity and explosion pushes, the
movement speed attribute (`update_attributes` base + server modifiers, the client's own sprint
modifier excluded; 1.8 physics takes only the base so far), status effects (jump boost,
blindness; 1.16 also levitation, slow falling, dolphin's grace), boots enchantments (1.16 depth
strider / soul speed from the armour NBT) and spectator mode (1.16). Haste / mining fatigue
(effects) and aqua affinity (helmet) feed the dig speed.

## Hands: dig, place, use (phase 3 groundwork)

`hands.rs` ports 1.8.9 `MinecraftClient.tick` key processing and
`ClientPlayerInteractionManager` (verified against recorded lab packet traces); it runs after the behaviour and the held-slot sync
and before the movement step, so its packets precede the tick's movement packet.

**Turn held on action ticks** (`Client::hold_turn_on_action`, on by default): Grim's RotationPlace re-checks a dig / place with the look of the movement packet
that follows it. When this tick's rotation step would take the crosshair off a block dug
(start / finish) or placed this tick — full-cube ray from the sent and the current feet,
standing or sneaking eyes, old or new yaw, 6 blocks — the look stays for one tick and the turn
continues the next time the behaviour aims (`ctx.held_turns()` counts them).

- `ctx.set_buttons(Buttons { left, right })`: left held mines the crosshair block —
  `block_dig` start (abort of a previous block), damage from `relative_hardness` (hardness,
  harvest tools, tool multiplier + efficiency, haste / fatigue, eyes in water, not on ground
  ÷5; f32 sums like vanilla), finish at ≥ 1, then 5 ticks of hit delay; an arm swing per
  click and per damage tick; letting go aborts. Right held: `block_place` on the face
  (activation or placing swings) every 4 ticks, else the item use packet; usable items (food,
  potion, milk, bow, sword) set `using_item` until right is released (`block_dig` release),
  the server's entity status 9, or a slot change. A broken block is predicted air.
- `ctx.dig(pos, face)` / `ctx.place(support, face, hit)`: refused unless
  `ctx.target()` — the ray from the **sent** eyes along the sent look, reach 4.5 — is on that
  block face; dig = start (+ instant break) then swing, place = `block_place` then swing.
- `ctx.target()`, `ctx.mining()`, `ctx.broken()`, `ctx.using_item()`.
- The crosshair is vanilla 1.8's `World.rayTrace(eyes, eyes + look × reach, false, false, true)`
  (block walk with the −1e−4 zero-fraction rule, per-block six-plane test, ties to the earlier
  plane; stairs: the eight half-cube passes, the hit farthest from the ray's end wins) with
  `Entity.getRotationVector` from the float sine table and eyes = feet + `1.62F` (− `0.08F`
  sneaking); ray shapes from `ghast_physics::v1_8::ray_shape` (ghast-data's oracle-verified
  `selection()` bounds plus the neighbour-dependent fences / panes / walls / doors / chests and
  the stairs passes). `tests/oracle_raytrace.rs`: all **2000** of the oracle's rays bit-exact
  (hit block, face, hit vector, miss block).

1.9+ rules (1.16.5): the left click digs first and swings after (a click on air is a miss); the
right click tries the main hand, then the off hand — per hand the block (activation unless
sneaking with an item; a block item places and swings), then the item itself (`use_item`), the
first success ending it, so empty hands send a `block_place` per hand; swords no longer block,
shields / bows / crossbows / tridents / potions / milk / food are used; the swing names its
hand.

Open: entities are not hit by the crosshair yet; the 1.16 attack cooldown only matters for entity attacks (not implemented);
1.16 ray boxes are approximated (union of collision boxes).

## Entities (ghast-entity, opt-in)

With `Interests::ENTITIES` (and `PLAYER_LIST` for the tab list) the session creates the
version's `ghast_entity` tracker; its packet ids join the filter, so bots without the interest
never decode entity packets. `ctx.entities()` is the store (`get`, `within`, `players`, …). The
interests can change mid-session: a tracker started late is empty but knows our own entity id
and dimension (it replays the session's join / respawn packets); switching off drops it.

- Crosshair: vanilla `GameRenderer.updateTargetedEntity` — entities that `collides()` (living,
  players, paintings, boats, minecarts, TNT, falling blocks, item frames, armor stands, leash
  knots, end crystals) whose box, grown by 0.1 on 1.8 (0 on 1.9+), the look ray hits first,
  within 3 blocks and closer than the block hit. It takes over the buttons: left attacks
  (`use_entity` attack; 1.8 swing first, 1.16 swing after and the attack cooldown resets), right
  interacts (interact-at, then interact, then the held item like vanilla's fall-through for
  plain entities; 1.16 per hand).
- `ctx.target_entity()` (from the sent pose), `ctx.attack(id)` (refused unless the sent
  crosshair is on it), `ctx.attack_strength(attack_speed)` (1.16 cooldown, 1.0 on 1.8).
- Entity boxes come from ghast-entity (minecraft-data sizes, server positions; vanilla picks
  the interpolated position). Entity interactions that vanilla handles client-side (villager
  trading, armor stands) are not predicted.

## Inventory (ghast-inventory)

Every window packet is fed to `ghast_inventory::Inventory` on arrival; the inventory answers
window confirms (exactly once — the engine does not). `ctx.inventory()`, `ctx.click(Click)`
(refused while turning or when the inventory refuses: one click in flight, same-tick order
rules, unpredictable results), `ctx.open_inventory()`, `ctx.close_window()`;
`ctx.events().inventory` carries change flags and click results. The held stack drives dig speed
(tool, efficiency from NBT) and placing; an open GUI releases the mouse buttons.

## Grim compatibility (`Client::force_step_position`, off by default, NOT vanilla)

Grim 2.3.73 sets back two vanilla-exact situations found by the movement course (oracle replays
of the exact recordings: ghast == vanilla, states and packets): (1) a landing step-up onto a
1/8-higher block (soul sand → grass) whose move stays under the 0.03-block resend threshold, so
vanilla sends only `look` / `flying` with on-ground true (both versions); (2) 1.16.5 walking on
slime (on-ground alternates every tick; GroundSpoof). With `force_step_position(true)` the client
sends the position on ticks whose feet height changed under the threshold (and updates its
sent bookkeeping): (1) disappears on both labs (16 min, 0 setbacks); (2) remains. The option
also works on 26.1 / 26.2 (a status-only movement packet whose feet height changed gets the
position).

(3) **26.2, idle drop of 20 blocks onto slime**: Grim 2.3.74 and 2.3.75 (alpha e457d55) flag
Simulation 0.0417 at the tail of the bounce, on every drop. The tick before, the player rests
6.2e-5 above the slime without a position change and sends `flying(on_ground)`; then it bounces.
The oracle scenario `slime_drop_20_long` proves ghast's packets equal the real 26.2 client's on
every tick (engine replay `restitution_drops_through_the_engine`). Lab drops of 8, 12 and 30
blocks onto slime and of 6 blocks onto a bed draw no flag. `force_step_position` cannot help (there is no moved
position to send); avoid long idle falls onto slime on Grim servers.

## Recording (`Client::record(path)`, 1.8.9 and 1.16.5)

Writes the session in the oracle's replay format when it ends: `setup` = the player at the
first movement step, with its exact bounding box (`bb`; 1.16.5 also `pose`), so replays need no
box rebuild; `loaded_chunks` / `fills` = every column loaded within 3 chunks of the player at any
step, with its blocks as first seen; one `tick` line per movement step with the input (keys,
yaw, pitch, using item) and the inbound events applied before it (teleports — raw packet values
with their `flags` byte, the reply the client sent in `ghast_reply` — our velocity, abilities), plus
ghast's own `state` and packets. Compare with `oracle/cache/replay_compare.py`. Block changes
after a column was first seen and explosion pushes are not representable (`ghast_note`).

## API stability (0.4)

Types the library fills in and may extend are `#[non_exhaustive]`: the events (`TickEvents`,
`RawInbound`, `TeleportEvent`), the pose and status types (`Pose`, `Health`, `Abilities`,
`ChatMessage`, `Teleport`, `JoinGame`), the crosshair results (`BlockHit`, `EntityHit`,
`RayResult`), and the enums `Inbound`, `Outbound`, `SessionEnd`, `ClientError`, `ClickRefused`,
`GameVersion`, `Account`, `ForceStepPosition`, ... (match them with a `_` arm). New Minecraft
versions arrive as new `GameVersion` variants in patch releases; `GameVersion::ALL` is a slice. Option structs
are built with methods: `ClientSettings::default().with_view_distance(4)`,
`WorldConfig::default().with_light(true)`. Plain inputs (`Controls`, `Buttons`, `Vec3`, the
hands' `BlockQuery` / `HeldItem` / `DigFactors`) stay open structs.

## Observers (viewers, recorders)

All read-only and free unless used:

- `Client::world_config(WorldConfig::default().with_light(true)…)`: what the world keeps
  besides block states (light, biomes, block entities; default: nothing extra).
- `ctx.world().column(cx, cz)` / `view.world().column(..)`: the stored `ghast::world::Column`
  (sections, light, biomes) for bulk reads, instead of one `raw_block` call per block.
- `ctx.swings()`: arm swings written so far (behaviour, actions and held buttons).
- `Behavior::after_tick(&TickView)`: called after every movement step (also while on hold or
  after the behaviour finished) with the sent pose, world, entities, inventory and events.

## Behaviour API reference and trace replay

`PARITY.md` describes the behaviour contract (`Behavior` + `TickContext`) member by member
and records the trace replay: wheat plant / harvest traces recorded on a local lab server by
a reference mineflayer-based bot, replayed through the engine, give the same packets in the
same order on every tick (`src/trace_replay.rs`). `PORTING.md` is the guide for porting a
mineflayer-style bot to ghast (before / after snippets, Grim settings, a one-process-per-bot
runner).

## Tests

```sh
cargo test -p ghast-client
```

- `src/engine.rs`: tick order (actions before the single movement packet, one per tick),
  join/teleport replies per version, standing packets (C06, then C03 with a C04 every 21st
  tick), silent guard, transactions, panicking behaviour, immediate respawn.
- `src/engine.rs` (parity): block-change events per version (1.16 section y sign), `set_flying`
  (needs "may fly", abilities before the move), `running() == false` stops the ticks, turn
  held on a place tick, click tickets.
- `src/engine.rs`: 1.8 planting uses the seed at once (not on stone, not for a block item).
- `src/trace_replay.rs`: trace replay of the lab fixtures in `tests/fixtures/traces/`
  (every tick identical), see `PARITY.md`.
- `src/movement.rs`, `src/rotation.rs`: packet choice, GCD steps, Java float maths.
- `src/sim.rs`: the 1.8 physics' teleport equals the documented 1.8 rules; standing on and
  walking over a chunk decoded by `ghast-world`.
- `tests/fake_server.rs`: an in-process fake server, every test for both versions: offline
  login with compression (and 1.8's mid-stream play compression), keep-alive, transaction,
  kick reason, login refusal, online mode without an account, server close (play and login),
  malformed packet, garbage compressed frame, read timeout, wire-level tick order, silent
  guard, clean shutdown, chat events, two bots on one multi-threaded runtime.
- `tests/cpu.rs` (ignored; `cargo test -p ghast-client --release --test cpu -- --ignored
  --nocapture`): one bot against a busy fake 1.8 server (per tick a Grim-style transaction, 40
  entity moves, 4 entity velocities; time and keep-alive): client thread CPU from
  `/proc/thread-self/stat`. WSL2 desktop, 2026-11-03: 50 ms CPU in 20 s = **0.25 % of one
  core** at ~900 inbound packets/s.

## Live results: 26.2 (local WSL lab, Paper 26.2 build 135 + Grim 2.3.74, 2026-11-12/13)

Every script the 26.1 lab passes, on 26.2 (`LAB_VER=26.2` / `<script> 26.2`):

| run | result |
|---|---|
| AFK 1 min and 5 min | 0 flags (5 min: CPU 0.41 % of a core, RSS 8.3 MB with every version compiled in) |
| `movement_course` 3 min, `dig_place`, `attack` | 0 flags |
| `airborne-join.sh 26.2 3` | 0 flags |
| `use-items.sh 26.2` (golden apple, shield, bow while walking) | 0 flags |
| `elytra.sh`, `riptide.sh` (water and rain) | 0 flags |
| `entities.sh` crowd, boat, teams (collisionRule never: the 26.2 team layout), horse, villager, border, cart | all PASS, 0 flags |
| `inventory_probe` (chest + clicks of every mode) | PASS, 0 flags |
| pathfinder lab course (corridor, maze, follow) | 0 failed legs, 0 setbacks, 0 flags |
| `transfer.sh` (2 transfers) | PASS, 0 flags |
| `proxy-switch.sh 4` (Velocity 3.5.1, two 26.2 backends, different heights) | 4/4, 0 flags, RSS growth 0 % |
| `online-26.1.sh` with `LAB_VER=26.2` (online mode, signed chat) | PASS (signed chat secure, unsigned control refused), 0 flags |
| idle drops onto slime (8, 12, 30 blocks) and a bed (6) | 0 flags |
| idle drop onto slime from 20 blocks | Grim Simulation 0.0417 every time: corner case (3) above, vanilla-exact |

## Live results: 26.1 (local WSL lab, Paper 26.1.2 build 74 + Grim 2.3.74, 2026-11-11)

Native 26.1 connections (no ViaVersion); Grim's profile shows the bot as `Version: 26.1`, brand
`vanilla`. The lab is a superflat world with the surface at y = 3 like the other labs
(`lab/templates/server-26.1.properties`).

| run | result |
|---|---|
| AFK 30 s, then 10.5 min (view distance 8) | 0 Grim flags, 0 kicks |
| `movement_course` 3 min (walk, sprint, sprint-jump, jump, sneak incl. an edge, slab, block, wall, fence, ice, packed ice, soul sand, slime, water) | 0 flags, 0 setbacks |
| `chat`: a chat line, `/msg`, `/say` | delivered (`[Not Secure]` = unsigned), whisper and `/say` rendered, no kick |
| `dig_place` 2 min (grass by hand, dirt placed back) | 94 broken, 94 placed, 0 flags |
| `inventory_probe` | PASS: chest opened, 30 clicks of every mode accepted, the server's resent contents equal the prediction (0 differing slots), 0 flags |
| `attack` 1 min next to an armor stand (entity tracking on), `attack --burst` on a breakable one | 83 attacks + 28 interacts, 0 flags; the burst run breaks the stand (2 hits, removed + item dropped) |
| `lab/airborne-join.sh 26.1 5` (log in 1 block above the ground, 5 times) | 0 flags (Grim 2.3.74 and 2.3.75-e457d55) |
| `lab/proxy-switch.sh 10`: Velocity 3.5.1 + two backends (overworld y -64.. and a datapack overworld y -128..), `/server` ten times while walking | 10/10 switches, the bot sees each backend's floor (-64 / -128), 0 flags on both backends, resident memory flat (6.0 → 4.6 MB over the switches) |
| `lab/online-26.1.sh`: online mode + `enforce-secure-profile` against a local account stand-in | encryption + session join, signed chat, `/say`, `/me`, `/msg` accepted as secure, 0 flags; control run with `--unsigned`: chat refused ("missing profile public key") |
| `lab/transfer.sh`: `/transfer` back to the same server twice (`--follow-transfers`) | both followed (3 logins), 0 flags |
| `lab/elytra.sh 4`: walk off a tower, glide with a turn, rockets, land | 233 ticks gliding, 3 rockets attached and boosting, landed, 0 flags |
| `lab/riptide.sh`: riptide III trident in the water strip, then on grass in the rain | 2 + 11 launches, 0 flags |
| `lab/use-items.sh 26.1`: golden apple, shield, bow while walking | 0 flags |
| `lab/entities.sh` crowd: walk through 6 NoAI cows into an oak boat, entity tracking on, then off | on: pushed aside and over the boat, 0 flags; off: walks straight through, also 0 flags (Grim 2.3.74 does not check entity collision or pushes) |
| `lab/entities.sh` horse / villager: sneak-click a tamed horse (saddle out and back), trade with a villager (select trade, take the result) | horse screen 2 clicks, merchant screen 1 click, all confirmed as predicted, 0 flags |
| `lab/entities.sh` teams: two bots walk through each other, without a team, then in a `collisionRule never` team | without: pushed apart; with: pass straight through (x unchanged); 0 flags both ways |
| `lab/entities.sh` boat: board an oak boat (right click with tracking, `/ride` without), paddle straight, left, right against the pool's edge, then from the ice strip onto grass, sneak off | 3 runs, boarded and dismounted each, 0 flags |
| `lab/entities.sh` cart: seated by `/ride`, the cart runs ~35 blocks along powered rails, sneak off; tracking on and off | 0 flags both |
| `lab/entities.sh` border: walk into a 24-wide border, keep walking while it shrinks to 10 | stops at the edge (rounded out to whole blocks, as vanilla), the shrinking border passes, border damage kills, respawn, stops at the new edge, 0 flags |
| `creative_probe` (creative screen: clone, place, Q, drop outside, ctrl+Q) | 6 clicks accepted, 0 corrections, 0 flags |

AFK cost, `lab/bench-afk.sh` (30 s warm-up, 300 s), x86-64 WSL, one bot:

| | CPU (one core) | worst 10 s | RSS | PSS | threads |
|---|---|---|---|---|---|
| 26.1, view distance 8 | 0.34 % | 0.50 % | 6.9 MB | 5.3 MB | 1 |
| 26.1, view distance 2 | 0.33 % | 0.45 % | 6.2 MB | 4.8 MB | 1 |
| 1.8.9 on the 1.8 lab (view distance 6), same machine | 0.34 % | 0.40 % | 6.5 MB | 5.2 MB | 1 |

## Live results: 1.12.2 (local WSL lab, Paper 1.12.2 build 1620 + Grim 2.3.73, 2026-11-07)

Account `GhastBot13`, release examples, each run under `timeout -s TERM`, ~33 min in total:

| run | duration | activity | Grim flags | setbacks |
|---|---|---|---|---|
| `afk` (measured with `lab/measure.sh`, 300 s) | 6.7 min | standing at the spawn; CPU 0.13 % of one core (max 10 s 0.30 %), RSS 5.5 MB, PSS 4.0 MB, 1 thread | 0 | 0 |
| `movement_course --seed 7` | 8 min | 4 laps: ice, packed ice, soul sand, slime, water, slab step-up, block jump, sneak edge, wall / fence pushes, GCD turning | 0 | 0 |
| `soak` | 12.7 min | 127 walks, 63 digs, 63 places, 84 clicks (all accepted) | 0 | 0 |
| `inventory_probe` | 0.5 min | chest opened by right click, 30 clicks of every mode accepted, server contents equal the prediction (`PASS`) | 0 | 0 |
| `dig_place` | 5 min | 238 broken, 238 placed | 0 | 0 |
| `chat` | 0.3 min | chat sent and received | 0 | 0 |

## Live results: 30-min mixed soaks (local WSL labs, 2026-11-04)

`crates/ghast/examples/soak.rs` (walk / sprint / sprint-jump / sneak on open grass, dig + place
with vanilla timing, inventory open / 4 clicks / close; `--grim-compat`, `--record`), 30 min per
version, final binary:

| | 1.8.9 (Paper 1.8.8 + Grim) | 1.16.5 (Paper 1.16.5 + Grim) |
|---|---|---|
| activity | 314 walks, 156 digs, 156 places, 208 clicks | 306 walks, 153 digs, 153 places, 204 clicks |
| Grim flags | 0 | 0 |
| setbacks | 1 — an external console `tp` of the account | 0 |
| click results | all accepted | all accepted |
| CPU (avg / max 10 s) | 0.29 % / 0.50 % | 0.31 % / 0.50 % |
| RSS max | 59 MB (recording buffered in memory then; now streamed to a `.part` file) | 54 MB (same) |

An earlier 1.16 soak exposed the speed-attribute bug (sprint modifier excluded → Grim
Simulation .002–.03); fixed before these runs. `attack` example (entity pick + attacks; 1.16 at
full attack strength): 1.8 villager 30+ attacks, 1.16 armor stand 50+ attacks, 0 flags.

## Live results: movement course, dig / place / use (local WSL labs, 2026-11-04)

`crates/ghast/examples/movement_course.rs` (ice, packed ice, soul sand, slime, water strips;
slab step-up, block jump, sneak edge, wall and fence pushes, circles; GCD turning; seeded):
1.8.9 ~30 min, 1.16.5 ~25 min. All flags are the two Grim-vs-vanilla cases above; with
`--grim-compat` 0 setbacks on both (1.16 slime GroundSpoof x2 per crossing remains).
`dig_place` (left button dig with vanilla timing, `place` back; 1.8: stone by hand 151 ticks,
1.16: dirt) and `use_item` (golden apple eaten while walking: 32 ticks, ended by entity status
9; sword blocking; ×0.2 input) : 0 Grim flags. Found on the way: 1.8 C08 must carry the held
stack (Grim BadPacketsU) — fixed; food only starts eating below a full bar — fixed.

## Live results (local WSL labs, 2026-11-03)

`lab/bench-afk.sh --warmup 30 --secs 600 -- target/.../release/examples/afk --host 127.0.0.1
--port 25618|25616 --username GhastBot2|GhastBot3 --version 1.8.9|1.16.5`, both bots at the same
time, standing at the superflat spawn:

| | 1.8.9 (Paper 1.8.8 + Grim 2.3.73) | 1.16.5 (Paper 1.16.5 + Grim 2.3.73) |
|---|---|---|
| connected | 630 s, no disconnect | 630 s, no disconnect |
| Grim flags | 0 | 0 (after the 20-tick position refresh fix; before it, BadPacketsE every second) |
| CPU (one core, 600 s avg / max 10 s) | 0.41 % / 0.69 % | 0.31 % / 0.86 % |
| RSS / PSS | 3.9 MB / 1.8 MB | 3.8 MB / 2.1 MB |
| threads | 1 | 1 |

