# ghast-data

Static Minecraft Java Edition game data for the ghast client library: blocks, block states,
collision shapes, physics properties, items, entities, biomes, enchantments, foods, status effects
and tool speed multipliers.

| module | feature | version | block state model |
|---|---|---|---|
| `v1_8` | `v1_8` | 1.8.x (protocol 47) | `id << 4 \| meta` (`u16`, wire / chunk value) |
| `v1_12` | `v1_12` | 1.12.x (protocol 335 – 340) | `id << 4 \| meta`; every shape, flag and face shape from the 1.12.2 oracle |
| `v1_16_5` | `v1_16_5` | 1.16.5 (protocol 754) | global palette id with properties |

Both features are on by default; disable the ones you do not need.

```rust
use ghast_data::v1_8::{Block, BlockState};
use ghast_data::ShapeDependency;

let s = BlockState::from_raw(44 << 4 | 8).unwrap(); // top stone slab
assert_eq!(s.shape()[0].min_y, 0.5);
assert_eq!(Block::ICE.physics().slipperiness, 0.98);
assert!(matches!(Block::FENCE.shape_dependency(), ShapeDependency::Neighbours(_)));

use ghast_data::v1_16_5::{Block as B, Property};
let stairs = B::OAK_STAIRS.state_with(&[(Property::Facing, "east")]).unwrap();
assert_eq!(stairs.get(Property::Half), Some("bottom"));
println!("{stairs}"); // minecraft:oak_stairs[facing=east,half=bottom,shape=straight,waterlogged=false]
```

## Design

- Everything is generated into `src/generated/` by `codegen/data` and **committed**; building needs
  no network and no JSON parsing at run time. Tables are `static` arrays: id lookups are indexing,
  name lookups binary-search a sorted `(name, id)` array (no `phf` dependency; names are not a hot
  path).
- Hot-path tables are separate dense arrays: per state a shape index (`u8` in 1.8, `u16` in 1.16)
  into a deduplicated shape list, per block a `ShapeDependency`, and in 1.16 per state the block id
  and a fluid byte. `BlockState::collision()` is two (1.8) or three (1.16) array reads.
- Neighbour-, entity- or block-entity-dependent shapes are never presented as static:
  `Collision::dependency` says what else is needed, `Collision::boxes` is then only the
  default/isolated shape.
- Physics facts minecraft-data lacks (slipperiness, soul sand / honey speed factor, jump factor,
  cobweb/berry-bush stuck multipliers, climbable, liquids, slime/bed bounce) are maintained by hand
  in `codegen/data/src/facts.rs`, keyed by block name (codegen fails on names that do not exist).
  They are *unverified* until oracle vectors cover them.

## Regenerating

```sh
cargo run -p ghast-codegen-data            # downloads minecraft-data 3.119.0 into /.cache if needed
cargo run -p ghast-codegen-data -- --check --offline   # CI: fail if src/generated is stale
```

The npm tarball is verified against its pinned SHA-512 integrity. The generator prints notes for
data quirks it worked around (listed below).

## Data quirks found in minecraft-data 3.119.0

- 1.12: minecraft-data names two blocks with their 1.13 names (`light_gray_shulker_box`,
  `light_gray_glazed_terracotta`; vanilla 1.12.2 says `silver_*`); the tables keep
  minecraft-data's names. Ids 253 and 254 are not blocks (`Block::from_id` is `None`, they
  read as air). Several entity sizes differ from the oracle and are fixed in `facts.rs`.
- 1.16.5 `blockCollisionShapes` resolve to `pc/1.16.1` while `blocks` resolve to `pc/1.16.2`; the
  per-block state counts were checked and match (incl. chain `axis`, lantern `waterlogged`).
- 1.16.5 `foods.json` (`pc/1.16.1`) uses 1.16.1 item ids, which differ from the 1.16.2 item ids
  used for 1.16.5 (e.g. apple 576 vs 573). Foods are resolved **by name**.
- 1.8 `foods.json` lists block ids `carrots` (141) and `potatoes` (142) instead of the items
  `carrot`/`potato` (skipped), lists `melon_block` and `cake` (kept as listed) and lacks `melon`.
- 1.16 data gives unbreakable blocks (bedrock, barrier, ...) `hardness: 0` with
  `diggable: false`; 1.8 data uses `hardness: null`. Check `BlockInfo::diggable`.
- minecraft-data's block `material` is a tool-speed group, not vanilla's `Material`.
- Entity sizes are not float-exact (e.g. 1.8 giant `3.5999999999999996`, vanilla computes
  `0.6f * 6.0f`).

## 1.8.9 collision vs the vanilla oracle

`tests/oracle_collision.rs` compares the 1.8 tables bit for bit with
`tests/golden/1.8.9/collision_boxes.jsonl.gz` (every id × meta alone, plus neighbour configurations)
and the oracle's per-block slipperiness / climbable / fluid flags. All isolated shapes, including the
isolated shape of every neighbour-dependent block, match.

minecraft-data's 1.8 shapes are partly 1.9+ shapes. The generator fixes them with explicit,
oracle-cited overrides in `codegen/data/src/facts.rs` (`V1_8_SHAPE_OVERRIDES`):

| block | minecraft-data | vanilla 1.8.9 (oracle) |
|---|---|---|
| farmland | 15/16 high | full cube |
| ladder (metas 2–5) | 3/16 thick | 2/16 thick |
| snow_layer meta 0 | no box | zero-height box at y = 0 |
| waterlily | 1/16-inset, 0.09375 high | full width, 0.015625 high |
| piston_head | arm extends 0.25 outside the block | arm ends at the edge; horizontal arms 0.5 wide |
| glass_pane, stained_glass_pane, iron_bars (isolated) | 1/8 centre post | full-length cross |
| doors, upper half alone (metas 9–11) | per-meta facing | default facing (0.1875 at x = 0) |

Unregistered states: vanilla reads an `id << 4 | meta` that no block state produces as air
(1428 such states had a minecraft-data shape). The registered set comes from the oracle dump's
`"state": null` lines; those states have no collision, `BlockState::is_registered` reports them and
`BlockState::from_raw_vanilla` maps them (and unknown ids) to air.

Anvils and chests use mutable per-block bounds in 1.8. Following the oracle's policy
(`oracle/README.md`, "History-dependent shapes") ghast models the *settled* shape: anvil by facing
(static), chests by double-chest connection (`ShapeDependency::Neighbours(Chest)`).

Moving pistons (`piston_extension`, id 36) take their box from the piston block entity:
`ShapeDependency::BlockEntity`.

### Float world coordinates

A few 1.8 blocks build their collision box with `float` arithmetic on the block position
(`(float) pos.getX() + 0.0625F`), found by scanning every block class's `getCollisionBox` /
`appendCollisionBoxes` bytecode: cactus, cake, all fence gates (closed), and soul sand / snow
layers (only `maxY`, exact for y ≤ 255). `Collision::world_coords` carries one `WorldCoord` rule per
bound and `Collision::world_box(i, x, y, z)` / `world_boxes` reproduce vanilla's absolute boxes bit
for bit, including the min/max ordering of vanilla's `Box` constructor (a cake below x = -2^24 ends up
with swapped bounds). For every other block `world_box` equals `Aabb::at` (double addition).
`tests/oracle_collision.rs` checks all states at three far positions from
`tests/golden/1.8.9/collision_boxes_far.jsonl.gz`.

## Entity sizes

`v1_8::vanilla_entity_size(name)` / `v1_16_5::vanilla_entity_size(name)` return the oracle's
measurements (`tests/golden/<ver>/entity_sizes.jsonl.gz`): every entity spawned the way the client
spawns it, plus the metadata-dependent variants (`baby`, `wither`, `elder`, `size_1..4`, armor stand
`small` / `marker`, 1.16 player poses) with when the client applies them (`VariantApplied`).
`slime_dimensions(n)` reproduces vanilla's slime / magma cube formula for any size, and
`EntityDimensions::bounding_box` builds vanilla's centred box bit for bit. `EntityInfo::width` /
`height` are the vanilla spawn sizes; minecraft-data differences are fixed in `facts.rs`
(`V1_8_ENTITY_SIZE_FIXES`, `V1_16_5_ENTITY_SIZE_FIXES`) and checked against the dumps by codegen.
Notable 1.8.9 client behaviour: baby zombies keep the adult box (`baby_server` is the server's size),
a plain skeleton is 0.6 x 1.8, and the wither skeleton size arrives on the next client tick.
