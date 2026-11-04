# ghast-entity

Entity tracking for the ghast Minecraft Java Edition client library, 1.8.9 (protocol 47) and
1.16.5 (protocol 754). It keeps what the vanilla client knows about every entity the server
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

- `v1_8::Tracker`, `v1_16_5::Tracker`:
  - setup and packets: `new(Config)`, `apply(&RawPacket)` / `apply_body(id, &Bytes)` returning
    `Result<Applied, DecodeError>`;
  - packet filter: `wants(id)`, `packet_ids()`;
  - store access: `store_mut()` (for `drain_events`).

  Both deref to `Entities`:
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
- `EntityType`: `Player`, `LegacyMob(u8)`, `LegacyObject(u8)`, `ExperienceOrb`, `Painting` (1.8),
  `Registry(u16)` (1.16). Static info (name, size) via `v1_8::info` / `v1_16_5::info` from
  ghast-data.
- Version-free helpers on `Entity`: `class()` (`EntityClass::{Player, Item, ExperienceOrb, Mob,
  Object, Painting}`), `item()` (stack of an item entity), `dimensions()` / `size()` / `bounding_box()` (vanilla, see below);
  `Entities::near` is an alias of `within`.
- Per-version helpers: `health`, `item`, `flags`, `custom_name`, `meta::*` indices.
- `Config { metadata, equipment, effects, player_list, events }`: everything is on by default
  except `events`. `Event` covers spawned / removed / moved / velocity / status / metadata /
  equipment / effects / mounted / collected / player list.
- Packet id lists per version: `CORE_IDS`, `METADATA_IDS`, `EQUIPMENT_IDS`, `EFFECT_IDS`,
  `PLAYER_LIST_IDS`.

The bot's own entity (id from `login`) has an entry with metadata, effects, equipment and riding
state, but **no position**: the client owns the bot's position, and movement packets for its id
are ignored.

## Vanilla behaviour reproduced

Checked with javap against the local 1.8.9 client (`ClientPlayNetworkHandler`, `EntitySpawnS2CPacket`)
and the official 1.16.5 client jar (`ClientPlayNetworkHandler`, `EntityS2CPacket`). Only
these conclusions are in this repository; no Mojang-derived files are.

| | 1.8.9 | 1.16.5 |
|---|---|---|
| server position | `int` 1/32 (`serverPos`); `pos = serverPos / 32.0` | packet coordinates (`f64`) |
| spawn / teleport | sets `serverPos` | sets the exact doubles |
| relative move | `serverPos += byte delta` | per axis: delta 0 keeps the double, else `(floor(x*4096) + d) / 4096` |
| bare `entity` packet | `on_ground = false` | `on_ground = false` |
| angles | `(float)(b * 360) / 256.0F` | same |
| velocity packet | `v / 8000.0` (double) | same |
| mob spawn velocity | `(double)((float) v / 8000.0F)` | same |
| object spawn velocity | read only when data **> 0** | always present |
| pickup (`collect`) | removes the entity | item: stack shrinks, removed when empty; others removed |
| riding | `attach_entity`; one rider; an unknown vehicle id dismounts | `set_passengers` (eject all, then mount), `attach_entity` = leash (0 = none) |
| login | clears everything | clears everything |
| respawn | clears on a dimension change; bot: effects, metadata, equipment reset | clears on a world-name change; bot: effects reset, metadata kept |

In both versions a spawn with an existing id replaces it, and unknown types spawn nothing (1.8: also
the abstract mob ids 48/49 and object ids vanilla does not create). Moves for unknown ids are
ignored, and removing an entity detaches its vehicle, passenger and leash links. 1.8 lightning
is a weather effect, not tracked by id.

The 1.16 rule "a vehicle the local player controls ignores position packets"
(`isControlledByLocalInstance`) is modelled only for boats (the bot is the first passenger).
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
- 1.16: slime / magma cube size, baby flag (piglin index 16, else 15), armor stand flags, and
  the player pose table.

Variants vanilla applies on the next client tick are applied immediately, and the old-min-corner
quirk after a resize is not modelled. Hanging entities (paintings, item frames, leash knots) have
no box here: vanilla builds it from facing and motive. Variants need `Config::metadata`.

Not tracked yet: `update_attributes`, `entity_status` state (only an event), and the 1.16
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
- `player_info` skips profile properties (skins) without allocating.

Bench (`cargo bench -p ghast-entity`, x86-64 WSL): a busy stream of 10,000 packets over 300
entities (35 % rel move, 20 % move+look, 15 % look, 10 % head rotation, 7 % velocity, 6 % teleport,
5 % metadata, 2 % status):

| | per packet |
|---|---|
| 1.8 tracker, everything on | 25 ns (40 M packets/s) |
| 1.8 generated full decode alone (no tracking), for comparison | 29 ns |
| 1.16.5 tracker, everything on | 26-29 ns (shared machine) |
| `wants()` filter check | 0.8 ns |

## Tests

```sh
cargo test -p ghast-entity
cargo bench -p ghast-entity
```

- `tests/v1_8.rs`, `tests/v1_16_5.rs`: known answers for every packet type, from hand-built bodies:
  - fixed-point positions and float-vs-double velocity cases;
  - the data > 0 velocity rule;
  - all 19 1.16 metadata serializers;
  - riding, leash and pickup rules;
  - the controlled boat;
  - login / respawn rules;
  - the tab list (properties skipped);
  - configuration filters.
- `tests/robustness.rs`, both versions:
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
