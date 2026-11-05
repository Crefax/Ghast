# ghast-world

Chunk storage for the ghast Minecraft Java Edition client library: decodes the chunk and
block-change packets of 1.8.9 (protocol 47) and 1.16.5 (protocol 754) into compact sections with
O(1) block lookup, and keeps loaded and unloaded columns explicitly apart.

| module | feature | chunk format |
|---|---|---|
| `v1_8` | `v1_8` | `id << 4 \| meta` `u16` arrays, light arrays, 256 biome bytes |
| `v1_16_5` | `v1_16_5` | paletted sections, 1.16 compact long arrays, 1024 biome ints |
| — | `physics` (default) | `ghast_physics::BlockView` for both worlds |

```rust
use ghast_world::{Applied, WorldConfig, v1_8::World};

let mut world = World::new(WorldConfig::default());
// for every decoded clientbound play packet:
// match world.handle(&packet) { Ok(Applied::NotWorldPacket) => ..., Err(e) => log(e), _ => {} }
assert_eq!(world.block_state(0, 64, 0), None); // not loaded: unknown, never air
```

`v1_8::PACKET_IDS` / `v1_16_5::PACKET_IDS` list the packets the world consumes (for a
`PacketFilter`). Each packet also has its own `apply_*` method, and `apply_chunk_data` takes the
raw fields for callers that do not build packet structs.

## Public API

- `v1_8::World`, `v1_16_5::World`: `new(WorldConfig)`, `handle(&ClientboundPlayPacket)`,
  `apply_login / apply_respawn / apply_map_chunk / apply_chunk_data / apply_map_chunk_bulk (1.8) /
  apply_unload_chunk (1.16) / apply_block_change / apply_multi_block_change / apply_explosion /
  apply_world_border / apply_view_position, apply_view_distance (1.16)`;
  queries `block_state(x, y, z) -> Option<BlockState>` (`None` = column not loaded),
  `is_column_loaded(cx, cz)`, `column(cx, cz) -> Option<&Column>`, `columns() -> &ColumnMap`,
  `border()`, `dimension()` (1.8) / `world_name()`, `view_distance()`, `view_center()` (1.16),
  `heap_bytes()`; mutation `set_block_state` (client-side prediction), `advance_time(ms)`
  (moves a lerping world border; call once per tick with 50), `clear()`.
- `Column`: `raw(x, y, z) -> u16` (column-local x/z low bits, world y; no hashing),
  `set_raw`, `section(i)`, `biomes()`, `block_light` / `sky_light` (1.8, when kept),
  `section_count()`, `heap_bytes()`.
- `ColumnMap`: version-independent map of loaded columns (`get`, `raw`, `contains`, `iter`, ...).
- `Section` (`Uniform` / indexed / direct, see below), `block_index(x, y, z)`.
- `Border`: centre, `size()` (interpolated), `target_size()`, warning fields.
- `WorldConfig` (`#[non_exhaustive]`; build it with
  `WorldConfig::default().with_biomes(..).with_light(..).with_block_entities(..)`): `biomes`
  (default true), `light` (default false), `block_entities` (default false). `World::packet_ids()` gives the packet ids for a configuration
  (`PACKET_IDS` plus `BLOCK_ENTITY_IDS` when block entities are on).
- Block entities (opt-in): `Column::block_entities()` returns `([x, y, z] column-local,
  &BlockEntity)` pairs, plus `Column::block_entity(x, y, z)` and `World::block_entity(x, y, z)`.
  `BlockEntity` is either `Nbt { action, nbt }` (a raw NBT copy) or `Sign([lines; 4])` (1.8,
  raw chat JSON).
- `Applied::{Changed, Ignored, NotWorldPacket}`; `WorldError` (malformed data).
- `packed::{unpack_be, pack, words_needed}`: the 1.16+ compact long array layout.

## Representation

Columns live in a `HashMap` keyed by the packed chunk position with a one-multiply hasher. A
column is 16 optional sections (all-air sections are not stored). A section is one of:

| variant | when | heap | lookup |
|---|---|---|---|
| `Uniform(state)` | one non-air state everywhere | 0 | 1 match |
| indexed | ≤ 256 distinct states | 4.6 KiB (`u8` per block + 256-entry palette) | 2 loads, no bounds checks |
| direct | > 256 distinct states | 8 KiB (`u16` per block) | 1 load |

States are stored as raw `u16` already mapped the way vanilla reads them, so a lookup never
validates anything. Block changes go uniform → indexed → (palette compaction) → direct, and a
section that becomes all air is dropped. 1.16.5 has the same 0..256 height, so both versions
share the 16-section column; 1.17+ variable heights will need the section count per world.

A possible further step (not done): 4-bit indices for sections with ≤ 16 states would halve the
memory of typical terrain sections at the cost of a shift per lookup.

## Vanilla behaviour reproduced

1.8.9 (checked against the client's `ClientPlayNetworkHandler` / `Chunk.method_3895` (unnamed in Legacy Yarn) / `ExplosionS2CPacket`):

- Ground-up `map_chunk` with an empty mask unloads the column; otherwise ground-up replaces the
  whole column (unmasked sections become air). A partial chunk replaces only the masked sections
  and is ignored when the column is not loaded (vanilla writes into its shared empty chunk).
- `map_chunk_bulk` loads every listed column ground-up, also with an empty mask (an empty loaded
  column, not an unload). The packet's sky-light flag splits the data.
- Sky light presence: vanilla uses the client's dimension; this decoder uses the data length when
  exactly one layout fits (so the biome bytes cannot shift on a dimension mismatch), else the
  dimension (overworld = sky). Trailing bytes are ignored, as in vanilla.
- Unknown ids and unregistered metas (`stone:7`) read as air (`BlockState::from_raw_vanilla`),
  in chunks and in block changes (vanilla's state id map returns null there).
- Block changes in unloaded columns, at `y` outside `0..256` or `|x|, |z| >= 30,000,000` are ignored.
- `explosion` offsets are relative to `(int) x` (truncation towards zero); listed non-air
  blocks become air.
- Join game clears the world; `respawn` clears it **only when the dimension changes**.
- Light is stored only with `WorldConfig::light` and is never recomputed after block changes.

1.16.5 (checked with javap against the official 1.16.5 client jar: the client chunk manager,
`WorldChunk.loadFromPacket`, `PalettedContainer`, the explosion packet, respawn handling and
`World.getBlockState` / the empty chunk):

- Section header `bits` is a signed byte: `<= 4` → 4-bit array palette (max 16 entries, more is
  rejected), `5..=8` → hash palette, otherwise global ids with 15 bits whatever was sent.
- Palette ids that are not states and indices past the palette read as air (vanilla's container
  default). A data array whose length does not match the bits is rejected (vanilla would silently
  keep stale storage). Mask bits above 15 and trailing bytes are ignored.
- Full chunk = new column contents (biomes kept when exactly 1024 ids); partial chunk only for a
  loaded column; an empty-mask full chunk is an empty loaded column; only `unload_chunk` unloads.
- View window: radius `max(2, view distance) + 3` around the `update_view_position` centre. Once
  both are known, chunks outside are ignored and columns outside are dropped when the window
  moves or shrinks. Until the first centre arrives every chunk is accepted (vanilla starts at
  0, 0; conforming servers send the centre before chunks, so this only matters for odd servers).
- `multi_block_change` uses the 1.16.2+ format (`state << 12 | x << 8 | z << 4 | y`, 20-bit signed
  section Y). Explosion offsets are relative to `floor(x)`; `air`, `cave_air`, `void_air` stay.
- `block_state` returns `void_air` for `y` outside `0..256` in loaded columns, like vanilla.
- Respawn clears the world only when the world name changes. Heightmaps, block entities and
  `update_light` are not stored.

World border (both versions): all six `world_border` actions; a lerp is advanced only by
`advance_time(ms)` (deterministic); 1.8 computes progress in `float`, 1.16 in `double`. Login and
world changes reset it to the vanilla default (centre 0, 0, size 6e7).

## Block entities (opt-in)

With `WorldConfig::block_entities` the world keeps block entity data as the server sends it.
Off, the packets are not in `packet_ids()`, nothing is parsed, and block changes do no extra
work. The only cost is an empty `Vec` (24 bytes) per column.

- **1.8:** `update_sign` (four raw chat JSON lines) and `tile_entity_data` (raw NBT + action).
  1.8 chunks carry no block entities. A ground-up chunk is a new vanilla chunk, so its block
  entities are dropped; a partial chunk keeps them, as vanilla does.
- **1.16.5:** the `map_chunk` block entity list (each compound goes to the position in its
  `x` / `y` / `z` ints; entries without them are skipped) and `tile_entity_data`. A partial chunk
  drops the block entities of the sections it replaces, as vanilla does.
- **Block changes** remove the block entity at that position when the block *type* changes
  (1.8: block id; 1.16: block), and keep it for a state change of the same block, as vanilla
  does.
  - This applies to `block_change`, `multi_block_change`, explosions and `set_block_state`.
- Data for unloaded columns and `y` outside `0..256` is ignored.
- Vanilla applies an update only when a block entity of the right kind already exists at that
  position. ghast stores whatever is sent, so a viewer sees the server's data.

## Unloaded columns and physics

`block_state` returns `None` for unloaded columns: behaviour code must never treat that as air. The `ghast_physics::BlockView` implementation is the one place that maps
unloaded to air (1.8) / `void_air` (1.16), because the vanilla client's empty chunk does exactly
that and the physics must reproduce it; `is_chunk_loaded(cx, cz)` drives the 1.8.9 unloaded-column
gravity rule (the physics truncates `(int) x >> 4` itself). Neighbour-dependent shapes (fences,
panes, walls, stairs, doors, chests) are resolved by the physics from neighbouring states read
through `BlockView::block`; the world keeps no connection state.

## Errors

Malformed chunk data never panics and always returns a `WorldError` for logging.

- 1.8 `map_chunk` shorter than its mask: the world ends up exactly as the vanilla client leaves
  it.
  - Vanilla first creates the empty column for a ground-up chunk, then `Chunk.fillChunk` copies
    block pairs in order until it runs past the array. The exception is logged by the client's
    task runner and play goes on.
  - So a ground-up chunk leaves a **loaded** column. Its blocks are filled in order up to the last
    complete little-endian pair; the rest of the failing section keeps what it had (air in a new
    section), and later sections are untouched.
  - Light and biome arrays are copied only whole and only while every earlier array was complete
    (`System.arraycopy` checks bounds first).
  - Seen on a real server: an End-dimension auth lobby sends a ground-up chunk with mask 1 and only
    the 256 biome bytes. Vanilla, and now ghast, keep the column loaded, with the first 128 blocks
    of section 0 decoded from those bytes.
- A short `map_chunk_bulk` applies nothing: vanilla fails it while decoding the packet.
- A malformed 1.16 section unloads the column. Vanilla only stores a new column after decoding
  succeeds; for an already loaded column it would keep a half-updated chunk, which we do not
  reproduce.

## Tests

```sh
cargo test -p ghast-world
cargo bench -p ghast-world
```

- `src/**`: section representation transitions and palette compaction, compact-array known
  answers (hand-computed words for bits 4, 5, 14, 15; padding ignored) and round trips for
  every width 1..=32, word counts per width, world border actions and float-vs-double lerp.
- `tests/v1_8.rs`: decode round trips against a test-side encoder of the vanilla layout (random
  sections with few / > 256 / garbage states, light, biomes, all three dimensions), sky-light
  inference, unregistered states, load / partial / unload semantics, truncation, bulk (incl.
  empty-mask column and truncated bulk), block changes, multi block change, explosion truncation,
  login / respawn rules, world border lerp, physics view.
- `tests/v1_16_5.rs`: every header bits value 0..=8 (array/hash palettes) and 9..=127 (global),
  wrong-length data rejected, signed header byte, hand-built known-answer words for bits
  4..=8 and 15 (which block each bit field lands in), out-of-palette indices / non-state ids /
  oversized array palette / oversized hash palette / trailing bytes, full vs partial, view window,
  section-position multi block change (incl. negative section Y), explosion floor, respawn.
- Block entities: signs, tile data with actions, same-block vs other-block changes, ground-up
  vs partial chunks (both versions), 1.16 chunk lists (including entries without coordinates),
  the off switch; a proptest of random sign / tile / block-change / partial-chunk sequences with
  junk NBT never panics and keeps one entry per position.
- `tests/robustness.rs`: proptest random and mutated chunk data for both versions and random
  bulk packets never panic; a failed 1.16 decode leaves the column unloaded; a property test
  compares truncated 1.8 chunks with a direct model of vanilla `fillChunk`; a model check
  of 600k random block changes across all representations.
- Hook for real captures: put clean lab packets (uncompressed `VarInt id + body`, one per file)
  into `tests/captured/1.8.9/*.bin` or `tests/captured/1.16.5/*.bin`; `captured_packets` applies
  them all and fails on any error. No such files are committed yet.

## Benchmarks

x86-64 WSL2 dev machine shared with other builds (runs varied by up to ~30 %),
`cargo bench -p ghast-world` (release, fat LTO). Synthetic overworld
terrain column = 5 non-empty sections (bedrock, stone with ores/caves/water, dirt, grass, trees).

| | time |
|---|---|
| decode 1.8 terrain column (61.7 KB of chunk data) | 30–43 µs |
| decode 1.8 full 16-section column (random mix) | 97–118 µs |
| decode 1.16.5 terrain column (10.3 KB) | 30–32 µs |
| decode 1.16.5 full 16-section column | 87–94 µs |
| `block_state`, 3×4×3 box pattern (physics-like) | ~4 ns / block |
| `block_state`, random positions over 441 columns (cache misses) | 10–12 ns / block |
| `Column::raw` (column already fetched) | ~1.5 ns / block |

Memory, 441 loaded terrain columns (view distance 10): 1.8 ≈ 10 MiB (≈ 23 KiB per column),
1.8 with light ≈ 19 MiB, 1.16.5 ≈ 11 MiB (≈ 25 KiB per column incl. 2 KiB biomes).
A Pi 5 is expected to be roughly 2× slower per operation; not measured there yet.
