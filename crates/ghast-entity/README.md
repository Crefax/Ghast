# ghast-entity

Entity tracking for the ghast Minecraft Java Edition client library, 1.8.9 (protocol 47),
1.12.2 (protocol 340), 1.16.5 (protocol 754) and 26.1 – 26.1.2 (protocol 775). It keeps what the vanilla client knows about every entity the server
announced, parsed by hand straight from the packet bodies.

```rust
use ghast_entity::{Applied, Config, v1_8::Tracker};

let mut entities = Tracker::new(Config::default());
// for every clientbound play packet (RawPacket): skip it before decoding if not wanted
// if entities.wants(raw.id) { entities.apply(&raw)?; }
for e in entities.within([0.5, 64.0, 0.5], 6.0) {
    let [x, y, z] = e.position();
    // e.entity_type(), e.yaw(), e.velocity(), e.vehicle(), v1_8::health(e), ...
}
```

## API

- `v1_8::Tracker`, `v1_12::Tracker`, `v1_16_5::Tracker`, `v26_1::Tracker`:
  - setup and packets: `new(Config)`, `apply(&RawPacket)` / `apply_body(id, &Bytes)` returning
    `Result<Applied, DecodeError>`;
  - packet filter: `wants(id)`, `packet_ids()`;
  - store access: `store_mut()` (for `drain_events`).

  All deref to `Entities`:
  - lookups: `get(id)`, `iter()` (skips the bot), `within(pos, r)`, `len()`;
  - the bot: `me()`, `self_id()`;
  - tab list: `player_info(&uuid)`, `players()`, `player_name(id)`;
  - other: `drain_events()`, `config()`, `clear()`.
- `Entity` accessors:
  - identity: `id()`, `uuid()`, `entity_type()`, `is_player()`;
  - position: `position()` (server position), `fixed_position()` (1.8: 1/32 ints);
  - motion: `yaw()`, `pitch()`, `head_yaw()` (degrees), `velocity()`, `on_ground()`, `data()`;
  - links: `vehicle()`, `passengers()`, `leash_holder()`;
  - state: `metadata()`, `equipment(slot)`, `effects()` / `effect(id)`.
- `EntityType`: `Player`, `LegacyMob(u8)`, `LegacyObject(u8)`, `ExperienceOrb`, `Painting` (1.8
  and 1.12: mobs by entity registry id, objects by spawn-object type), `Registry(u16)` (1.16).
  Static info (name, size) via `v1_8::info` / `v1_12::info` / `v1_16_5::info` from ghast-data.
- Version-free helpers on `Entity`: `class()` (`EntityClass::{Player, Item, ExperienceOrb, Mob,
  Object, Painting}`), `item()` (stack of an item entity), `dimensions()` / `size()` / `bounding_box()` (vanilla, see below);
  `Entities::near` is an alias of `within`.
- Per-version helpers: `health`, `item`, `flags`, `custom_name`, `meta::*` indices.
- `Config` (`#[non_exhaustive]`; build it with `Config::default().with_metadata(..)` etc.):
  `metadata`, `equipment`, `effects`, `player_list`, `events`, `skins`. Everything is on by
  default except `events` and `skins`. `PlayerInfo` is `#[non_exhaustive]` too (read-only). With `skins`, `PlayerInfo::textures` keeps the base64
  `"textures"` profile property (skin / cape URLs) of tab-list entries. `Event` covers spawned / removed / moved / velocity / status / metadata /
  equipment / effects / mounted / collected / player list.
- Packet id lists per version: `CORE_IDS`, `METADATA_IDS`, `EQUIPMENT_IDS`, `EFFECT_IDS`,
  `PLAYER_LIST_IDS`.

The bot's own entity (id from `login`) has an entry with metadata, effects, equipment and riding
state, but **no position**: the client owns the bot's position, and movement packets for its id
are ignored.

## Vanilla behaviour reproduced

Checked with javap against the local 1.8.9 client (`ClientPlayNetworkHandler`, `EntitySpawnS2CPacket`),
the official 1.16.5 client jar (`ClientPlayNetworkHandler`, `EntityS2CPacket`), and the 1.12.2
client read with Legacy Yarn names in the oracle's git-ignored cache. Only these conclusions are
in this repository; no Mojang-derived files are.

| | 1.8.9 | 1.12.2 | 1.16.5 |
|---|---|---|---|
| server position | `int` 1/32 (`serverPos`); `pos = serverPos / 32.0` | spawn / teleport doubles, after a move `tracked / 4096.0` | packet coordinates (`f64`) |
| spawn / teleport | sets `serverPos` | sets the exact doubles and `tracked = floor(x*4096)` | sets the exact doubles |
| relative move | `serverPos += byte delta` | `tracked += short delta`, **every** axis becomes `tracked / 4096` | per axis: delta 0 keeps the double, else `(floor(x*4096) + d) / 4096` |
| bare `entity` packet | `on_ground = false` | `on_ground = false`, position snaps to `tracked / 4096` | `on_ground = false` |
| angles | `(float)(b * 360) / 256.0F` | same | same |
| velocity packet | `v / 8000.0` (double) | same | same |
| mob spawn velocity | `(double)((float) v / 8000.0F)` | same | same |
| object spawn velocity | read only when data **> 0** | always sent, applied when the (partly reset) data is > 0; llama spit always | always present |
| pickup (`collect`) | removes the entity | removes the entity (the count only feeds the animation) | item: stack shrinks, removed when empty; others removed |
| riding | `attach_entity`; one rider; an unknown vehicle id dismounts | `set_passengers`, `attach_entity` = leash of a mob (unknown holder = none) | `set_passengers` (eject all, then mount), `attach_entity` = leash (0 = none) |
| login | clears everything | clears everything | clears everything |
| respawn | clears on a dimension change; bot: effects, metadata, equipment reset | clears on a dimension change; bot: effects, equipment reset, metadata kept | clears on a world-name change; bot: effects reset, metadata kept |

In both versions a spawn with an existing id replaces it, and unknown types spawn nothing (1.8: also
the abstract mob ids 48/49 and object ids vanilla does not create). Moves for unknown ids are
ignored, and removing an entity detaches its vehicle, passenger and leash links. 1.8 and 1.12
lightning is a weather effect, not tracked by id. 1.12 fishing bobbers spawn only when the
owner (object data) is a known player, and effects are kept for living entities only.

The 1.12 / 1.16 rule "a vehicle the local player controls ignores position packets"
(`canPassengerSteer` / `isControlledByLocalInstance`) is modelled only for boats (the bot is the
first passenger); in 1.12 the tracked 1/4096 value keeps following the packets meanwhile.
Horses, pigs and striders are not modelled.

The rendered (interpolated) position is not computed: vanilla moves towards the server position
over 3 ticks, and the 1.8 teleport keeps the rendered position when it is within 1/32. A bot
wants the server position.

Entity sizes (`Entity::dimensions`, `size`, `bounding_box`) use ghast-data's vanilla size
tables, which are checked bit for bit against the oracle. The bounding box is the bit-exact
vanilla centred box at the server position. On top of the type's spawn size, the client-side
variants come from metadata:
- 1.8: slime / magma cube size, wither skeleton, elder guardian, baby ageable mobs, baby zombies
  (the 1.8 client keeps the adult box), small / marker armor stands;
- 1.12: slime / magma cube size (VarInt at 12), baby flag (boolean at 12: ageable mobs and
  zombies), armor stand flags (byte at 11), sneaking players (0.6 × 1.65);
- 1.16: slime / magma cube size, baby flag (piglin index 16, else 15), armor stand flags, and
  the player pose table.

Variants vanilla applies on the next client tick are applied immediately, and the old-min-corner
quirk after a resize is not modelled. Hanging entities (paintings, item frames, leash knots) have
no box here: vanilla builds it from facing and motive. Variants need `Config::metadata`.

## 26.1: positions as the client holds them, proven against the real client

The 26.1 client has no separate server position: an entity is where `Entity.position()` says,
and position packets either set an interpolation target (living entities, boats, minecarts,
experience orbs, fishing bobbers: 3 steps) or move the entity at once (everything else; the
type facts come from the jar via ghast-data). `v26_1::Tracker::tick()` runs the client's entity
tick: interpolation (`InterpolationHandler`, doubles, the target follows position changes made
between steps unless the moved box collides: `tick_with(no_collision)` takes vanilla's
`Level.noCollision`), head-rotation lerps, the mob `BodyRotationControl` (body and head yaw),
sleeping (xRot 0), deferred scale-attribute resizes, players leaving the ground as spectators or
passengers, empty item entities being discarded. Entities in unloaded chunk columns do not tick
(the tracker reads the chunk coordinates of `map_chunk` / `unload_chunk`); players always do.

Call order per client tick (vanilla's `Minecraft.tick`): apply the tick's packets, pick with
`Entity::position()` / `bounding_box()` (+ `v26_1::pick_radius`, `Tracker::pickable`), then
`tick()`. Packet coordinates (`VecDeltaCodec` base, `Math.round(x * 4096)` + delta) are
`v26_1::server_position`.

Also modelled: spawn rotations (`absSnapTo` for living entities), syncs over 64 blocks and of
non-ticking entities snapping, teleports relative to the interpolation target, `Math.round`
half-up, the duplicate-UUID refusal of the entity lookup (and the id it leaves in the tick list),
players needing a tab list entry to spawn, `putIfAbsent` tab entries and the tab entry a player
entity keeps after removal, sleeping positions moving the entity onto the bed, the fox's own
sleeping flag, slime resizes turning the body, item pickups shrinking the stack. Sizes: baby,
slime / magma cube size, armor stand small / marker, every pose (sleeping, player poses, camel
sitting, digging warden, ...) and the `minecraft:scale` attribute (modifiers in packet order;
vanilla sums several per operation in hash order), with fixed dimensions not scaled.

**Proof:** the oracle's `EntityTrackDump` sends random entity packets as wire bytes through the
real `ClientPacketListener` and records every entity at pick time (position, rotation, head and
body yaw, box, on-ground, pickable, ticking, riding links, item counts, tab list).
`tests/oracle_26_1.rs` replays `tests/golden/26.1/entity_tracking.jsonl.gz` (24 cases x 72
ticks) with metadata tracking on and off at 0 diffs; a 500 x 100-tick run
(`ORACLE_TRACKING=oracle/cache/26.1/tracking/...`): 0 diffs over 8.66 M compared fields.
`tests/v26_1.rs` runs the vanilla 26.1.2 capture (39 spawns) through the tracker.

Not modelled on 26.1: client-side physics of non-interpolated entities between packets (items,
projectiles and experience orbs move on their own), passenger positions (placed on the vehicle
every tick), controlled vehicles other than boats, body / head turning of camels, creakings,
happy ghasts, phantoms, shulkers, armadillos (own body controls) and players' body yaw,
velocity decay (`velocity()` is the last value sent), boxes of hanging entities, shulkers,
interactions and wind charges (`bounding_box()` is `None`), phantom size, pufferfish puff state,
display interpolation durations. Equipment items carry id and count only (no components);
item entity stacks keep their components (`v26_1::item_stack`).

Not tracked yet (1.8 – 1.16): `update_attributes`, `entity_status` state (only an event), and the 1.16
`entity_sound_effect` / `camera`.

**Found in passing:** minecraft-data (and so the generated `ghast_protocol::v1_8` `spawn_entity`)
reads the object velocity when `objectData != 0`, while vanilla reads it only when `> 0`. This
crate parses that packet itself and follows vanilla.

## Performance

- Hand parsing from the body with the protocol's bounds-checked `Reader`. Movement, look,
  teleport, velocity, head rotation, status and destroy packets allocate nothing. Metadata is
  parsed into a reused scratch buffer and committed only when the whole packet is valid. Scalar
  updates overwrite in place; only strings, items and NBT allocate, as private copies, so
  nothing pins a packet buffer.
- Lookups: one `HashMap<i32, Entity>` probe with a one-multiply hasher.
- `wants(id)` is a 256-bit mask check, so the client can drop unwanted packets before decoding.
  With metadata, equipment, effects and the tab list off, only the movement core is parsed. In
  1.16 metadata is still read for item entities, whose stack the pickup rule needs.
- `player_info` skips profile properties (skins) without allocating; with `Config::skins` only
  the `"textures"` value is copied (one allocation per added player).

Bench (`cargo bench -p ghast-entity`, x86-64 WSL): a busy stream of 10,000 packets over 300
entities (35 % rel move, 20 % move+look, 15 % look, 10 % head rotation, 7 % velocity, 6 % teleport,
5 % metadata, 2 % status):

| | per packet |
|---|---|
| 1.8 tracker, everything on | 25 ns (40 M packets/s) |
| 1.8 generated full decode alone (no tracking), for comparison | 29 ns |
| 1.16.5 tracker, everything on | 26-29 ns (shared machine) |
| 26.1 tracker, everything on (same mix, 26.1 formats; syncs instead of teleports) | 40 ns |
| 1.16.5 / 26.1 relative move alone | 22 ns / 39 ns |
| 26.1 `tick()`, 300 interpolating zombies / 300 settled | 40 ns / 7 ns per entity |
| `wants()` filter check | 0.8 ns |

Where 26.1 spends the extra time per move (measured by removing parts): about 13 ns is the
client's interpolation bookkeeping on every position packet (compare with the running target,
store target, rotation and the previous position / rotation, as `InterpolationHandler.interpolateTo`
does), about 7 ns the second cache line of the boxed 26.1 state. Syncs read 64 bytes instead of
1.16's 34-byte teleport and look up the chunk. LpVec3 velocities cost what 1.16's shorts do.

Memory: `size_of::<Entity>()` is 232 bytes with every version compiled in (as in 0.3.0) and 224
without `v26_1`; a 26.1 entity adds one 120-byte heap block (interpolation target and previous
step, head / body turn, cached dimensions, size keys; the `VecDeltaCodec` base lives inline in
the entity's version slot). The hash map slot adds the key.

## Tests

```sh
cargo test -p ghast-entity
cargo bench -p ghast-entity
```

- `tests/v1_8.rs`, `tests/v1_12.rs`, `tests/v1_16_5.rs`: known answers for every packet type,
  from hand-built bodies:
  - fixed-point positions and float-vs-double velocity cases;
  - the data > 0 velocity rule;
  - all 14 1.12 and 19 1.16 metadata serializers;
  - riding, leash and pickup rules;
  - the controlled boat;
  - login / respawn rules;
  - the tab list (properties skipped);
  - configuration filters.
- `tests/robustness.rs`, every version:
  - every wanted packet id × 300 seeds of the protocol's `testgen` values (encoded by the
    generated codecs, which are checked against node-minecraft-protocol) is accepted;
  - metadata is equal to the generated codec's decoding for 500 seeds;
  - proptest: random bodies, mutated / truncated generated packets, and random
    spawn / destroy / move / mount / pickup sequences never panic and keep vehicle / passenger
    links consistent.
- Live (`tools/live_check.py`, needs the local labs, see `lab/README.md`):
  - setup: `examples/live_probe.rs` connects as `GhastBot3`. The script summons armor stands,
    a bat, an item, an XP orb, a minecart, a boat and an armor stand riding a boat. It moves two
    armor stands with 30 small random teleports each, lets the bat fly and then freezes it.
  - check: it compares the tracked server positions with the server's own (`entitydata` /
    `data get entity`).
  - result 2026-11-04: **1.8: 10/10 exact** (`floor(x*32)/32`), **1.16.5: 9/9 exact**
    (`floor(x*4096)/4096` or the exact double); riding links correct; the 1.16 item was
    picked up by the bot and is gone on both sides.
  - lab note: `spawn-animals=false` removes summoned animals and golems at once, so the live
    check uses non-animal entities.
