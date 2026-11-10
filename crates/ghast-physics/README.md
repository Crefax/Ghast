# ghast-physics

Vanilla-exact player movement for the ghast Minecraft Java Edition client library. A **pure,
deterministic function**: `(player state, tick input, world view) -> new state + the movement
packets of the tick`. No IO, no async, no clock, no mutable global state (the only static is
vanilla's constant 65536-entry sine table, built once).

| module | version | verification |
|---|---|---|
| `v1_8` | 1.8.9 (protocol 47) | bit-exact against the vanilla oracle (see below) |
| `v1_12` | 1.12.2 (protocol 340; the rules hold for every 1.12 release) | bit-exact against the vanilla oracle (see below) |
| `v1_16_5` | 1.16.5 (protocol 754) | bit-exact against the vanilla oracle (see below) |
| `modern` with `V26_1` (feature `v26_1`, default) | 26.1 (protocol 775) | bit-exact against the vanilla oracle (see "Modern core") |

## Use

```rust
use ghast_physics::{BlockView, Keys, Packets, TickInput, v1_8};

fn step<W: BlockView<State = v1_8::BlockState>>(
    player: &mut v1_8::PlayerState,
    world: &W,
) -> Packets {
    let mut out = Packets::new();
    let input = TickInput {
        keys: Keys { forward: true, sprint: true, ..Keys::NONE },
        yaw: player.yaw,
        pitch: player.pitch,
        using_item: false,
    };
    v1_8::tick(player, &input, world, &mut out);
    out // entity actions / abilities first, then exactly one movement packet
}
```

Per client tick the caller (ghast-client) does what `MinecraftClient.tick` does:

1. apply inbound packets that arrived since the last tick, in order:
   `PlayerState::apply_velocity_packet(x, y, z)` (raw 1/8000 units),
   `apply_teleport(pos, yaw, pitch, relative_flags)` (returns the `PositionLook` confirmation the
   client sends immediately; it does **not** touch `sent`, like vanilla),
   `apply_abilities(..)`, and plain field writes for the rest
   (`movement_speed_base` from entity properties, `food_level` from update health, `effects`
   for jump boost / blindness); `TickInput::using_item` is the client's item-use state (eating,
   blocking, bow) for this tick — the use / release packets themselves are sent by the client;
2. write this tick's actions (dig / place / click) — they are judged with `state.sent`, the pose
   the server has;
3. `v1_8::tick(&mut state, &input, &world, &mut out)` and send `out` in order.

`PlayerState` is `Copy`: clone it to plan ahead (look-ahead pathing, "where will I land") with the
same code path.

### The world: `BlockView`

```rust
pub trait BlockView {
    type State: Copy;                                    // v1_8::BlockState for 1.8
    fn block(&self, x: i32, y: i32, z: i32) -> Self::State; // only called for valid positions
    fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool;
    fn world_border(&self) -> WorldBorder { WorldBorder::default() }
}
```

Contract: `block` is only called with `|x|, |z| < 30_000_000` and `0 <= y < 256`; it must return
states as vanilla decodes chunk data (`BlockState::from_raw_vanilla`: unknown ids and unregistered
metas are air) and **air for columns that are not loaded** (vanilla's client uses an empty chunk:
no collision; the only other effect of an unloaded column is the gravity rule below).
`MemoryWorld` (`v1_8::memory_world()`) is a simple implementation for tests, benches and tools.

### Rotation helpers (`rotation`)

`mouse_multiplier(sensitivity)`, `mouse_step_degrees`, `turn(yaw, pitch, yaw_counts,
pitch_counts, multiplier)` reproduce the 1.8 renderer + entity angle update in whole mouse counts
(the "GCD" anti-cheats check), `wrap_degrees`. How many counts to move per tick (aim speed,
budgets) is client policy and stays out of this crate.

## Modern core (`modern`, 26.1)

One core for the modern versions, generic over a `modern::Rules` row (the version's block tables and
constants as associated items, monomorphised: no run-time version branch). `V26_1` is the first row.
The older modules stay as they are.

```rust
use ghast_physics::modern::{self, PlayerState, V26_1};
use ghast_physics::{Keys, Packets, TickInput};

let mut world = modern::memory_world_26_1();          // any BlockView<State = v26_1::BlockState>
world.load_chunks(-1, -1, 0, 0);
let stone = ghast_data::v26_1::BlockState::parse("stone").unwrap().id();
world.fill_raw([-16, 63, -16], [15, 63, 15], stone);
let mut p = PlayerState::new([0.5, 64.0, 0.5], 0.0, 0.0);
let mut out = Packets::new();
let input = TickInput { keys: Keys { forward: true, ..Keys::NONE }, yaw: 0.0, pitch: 0.0, using_item: false };
modern::tick::<V26_1, _>(&mut p, &input, &world, &mut out);
// out: sprint entity actions / abilities, then `Packet::PlayerInput` when the keys changed, then
// at most one `Packet::Move` (position and / or rotation, on-ground, horizontal collision).
// `tick_end` is sent by the client after every tick and is not in `out`.
```

What the client feeds in (26.x moved much of movement to server data):

- **Attributes**: every `update_attributes` entry for the nine movement attributes
  (`AttributeKind::from_name`) goes to `PlayerState::apply_attribute(kind, base, &modifiers)` in
  packet order (modifier ids via `ModifierId::new("namespace:path")`; the summation order of
  several modifiers follows vanilla's hash table, so the ids matter). Soul speed, depth strider,
  swift sneak, powder-snow frost and potion speed changes all arrive this way; the client never
  computes them. The client's own sprint modifier is handled by `set_sprinting`.
- **Motion**: `apply_velocity(v)` with the vector decoded from the packet's `LpVec3`.
- **Teleport**: `apply_teleport(pos, delta, yaw, pitch, relative_bits)` (all nine `Relative` bits);
  send the returned packet after `teleport_confirm`. It does not update `sent`, like vanilla.
- **Abilities**, **effects** (`Effects`: jump boost, blindness, levitation, slow falling,
  dolphin's grace, weaving), **food level**, **game mode** (`spectator`), **leather boots** in
  the feet slot (powder snow), the **item in use** (`TickInput::using_item` plus its
  `use_effects` component in `PlayerState::use_effects`), the client **options**
  (`auto_jump`, `sprint_window`) and the **level** (`LevelInfo`: build height, `fast_lava`).

World contract (`BlockView`): positions are only asked for inside the build height of
`LevelInfo`; horizontal coordinates can reach about ±30,000,100. Return air for unloaded columns
and answer `is_chunk_loaded` truthfully: collision skips unloaded chunks, and the fluid sweep sees
no fluid when any chunk around the player's box is unloaded (vanilla 26.1 client). Gravity is
normal over unloaded columns (the 26.1 client always reports chunks as present).

Verification: `tests/golden_26_1.rs` replays the committed 26.1 goldens (scenarios, `fuzz_seed1`,
`fuzz_edges_seed1`, `fuzz_extras_seed1`); the large-fuzz test replays `oracle/cache/26.1/` (4.0 M
ticks: plain, item use, edges, extras; commands in `oracle/README.md`, "26.1 fuzz"), 0 diffs.
26.1.1 and 26.1.2 give identical oracle records, so `V26_1` covers 26.1 – 26.1.2.

Cost per tick (`modern_tick`, 2026-11-11, machine shared with other builds): walk 3.3 µs hash-map
world / 2.4 µs dense world, busy collision 3.9 / 2.7 µs, swim 2.4 / 1.8 µs (1.16.5 on the same
scenarios: 5.4 / 3.2, 6.2 / 4.1, 3.3 / 2.2 µs).

Not modelled: elytra, riptide, vehicles, entity collision and push, pistons, block entities
(shulker box lids collide as closed boxes), a moving world border, the nether's `fast_lava` is
supported only as a flag (no oracle vector yet).

## Design decisions

- **Version rules are modules, selected at compile time.** `v1_8` has its own `PlayerState`,
  block rules and `tick`. `Ruleset` (implemented by `v1_8::V1_8`) lets the client be generic over
  versions with static dispatch; nothing on the hot path is dynamic. 1.16.5 will be a sibling
  module with its own state type (sneaking/swimming poses differ) behind the same trait.
- **Exactness first, structure our own.** Each step keeps vanilla's precision (`f32` vs `f64`,
  widening points, `int + float` producing a float, truncation vs floor, Java `Math.min/max`
  zero-sign rules, `Vec3d`'s `-0.0 -> 0.0`, NaN behaviour of negated comparisons). The code is
  organised in our own way (one context struct, shared sweep helpers, geometric stairs and door
  rules), not a transliteration.
- **Collision shapes:** static shapes come from `ghast-data`; this crate adds what a table cannot
  express: neighbour-dependent fences, panes, walls, stairs (outer/inner corners) and doors (each
  half reads the other half's metadata).
- **History-dependent shapes are an option.** 1.8.9 anvils, chests and trapped chests collide
  with whatever bounds the last shape update of any block of that type left behind.
  `v1_8::Options { history_shapes }` selects `HistoryShapes::Settled` (default, project policy:
  anvil by facing, chest extended towards the first same-kind chest north/south/west/east — what
  Grim models) or `Constructor` (anvil full cube, single-chest inset — what a fresh client and the
  oracle's movement harness have). `tick` uses the default; `tick_with` takes options. The golden
  and fuzz replays run with `Constructor`; the box-dump tests check `Settled` against every
  `"settled"` line and `Constructor` against the rest.
- **Block data from `ghast-data`:** static shapes are placed with `Collision::world_boxes`, which
  reproduces vanilla's `float` world-coordinate boxes (cactus, cake, fence gates; soul sand and
  snow only on y) and the sorting box constructor; per-block flags (full cube, full block, render
  as normal block, material identity and blocks-movement / solid / fluid / opaque) come from
  `Block::vanilla()` / `Block::material()`. A unit test checks the derived predicates (incl. the
  fence/wall connection rule) against the oracle dump.
- **Collision queries** fill a small inline box list (no allocation unless a query finds more than
  24 boxes); emptiness checks (sneak edge guard, liquid jump-out) stop at the first box. Every
  query updates the world-border flag exactly as vanilla's does.
- **Packets** come out as a fixed-capacity `Packets` list in send order.

## Vanilla behaviours worth knowing (all verified)

- Tick order: water contact (flow push 0.014) → lava halves fall distance → sprint timer →
  input (sneak ×0.3) → push out of blocks (4 probes at minY + 0.5) → sprint (double tap within 7
  ticks, sprint key, stop on forward < 0.8 / horizontal collision / food ≤ 6) → creative flight
  double tap → fly up/down impulse → velocity cut-off (|v| < 0.005) → jump (0.42, +0.1 per jump
  boost level, sprint +0.2 along yaw; water/lava +0.04) → input ×0.98 → travel → air acceleration
  for the next tick (0.02 / 0.026 sprinting: sprint changes reach the air one tick late) → landing
  ends flight → x/z clamp ±29,999,999 → packets.
- Travel: ground acceleration `attr * 0.16277136 / f³` with `f = slipperiness * 0.91` of the block
  at `(floor x, floor minY − 1, floor z)`; ladders/vines clamp to ±0.15, stop descent while
  sneaking, climb 0.2 on horizontal collision; water drag 0.8, lava 0.5, jump out at a ledge 0.3.
- Gravity −0.08 ×0.98; when the column at `((int) x, (int) z)` (truncation) is not loaded the
  player instead gets `velY = −0.1` (0 at y ≤ 0).
- Move: cobweb (×0.25 / ×0.05, velocity zeroed), sneak edge guard in 0.05 steps, clip Y→X→Z,
  step-up 0.6 (two candidate paths), slime bounce / slime walking ×(0.4 + |vy|·0.2), soul sand
  ×0.4 per contained block, fall distance, landing re-checks water.
- Outside the world border (default ±29,999,984) blocks collide as stone while the player is
  inside it.
- Movement packet: position when moved² > 9e-4 or every 20 ticks, rotation when the float
  difference is non-zero; sprint/sneak entity actions before it; flight toggles send abilities.

## 1.12.2 (`v1_12`): what changed since 1.8.9

`v1_12` has the structure of `v1_8` (same calling convention, `PlayerState` is `Copy`) and is the
first member of the 1.9 – 1.12 family. Everything below is verified against the vanilla 1.12.2
oracle (`tests/golden/1.12.2/`, `oracle/README.md` "1.12.2"):

- **Ticking**: the client only ticks a player its world has added to a chunk: a player spawned
  above a column that is not loaded neither moves nor sends anything until the column arrives
  (`PlayerState::added_to_chunk`); once added it keeps ticking over unloaded columns, falling
  at −0.1 there (the column test floors x / z; 1.8 truncated).
- **Constants**: velocity cut-off 0.003 (1.8: 0.005); input acceleration converts the yaw with
  one `float` multiply (`yaw * 0.017453292F`) and has a (zero) vertical component, which turns a
  `-0.0` vertical velocity into `+0.0`.
- **Sneaking**: the edge guard probes one step height (0.6) down instead of one block, so a
  sneaking player walks down half-block drops; after the move the box becomes 0.6 × 1.65 while
  sneaking (1.8 when not) if the new box is free, rebuilt from its minimum corner; the step
  effects (slime walking) are skipped when the player is sneaking on the ground *after* the move.
- **Flying**: sneaking no longer slows horizontal flight (the input is divided by 0.3 again);
  flight resets the fall distance.
- **Effects / blocks**: levitation replaces gravity (`vy += (0.05 · (amp + 1) − vy) · 0.2`); beds
  bounce (`vy = −vy · 0.66F` unless sneaking); an open trapdoor above a ladder with the same facing
  is climbable.
- **Collision query**: one block wider on x / z (corner columns and the top layer of edge columns
  skipped); no validity filter on columns, so the size check counts columns beyond ±30,000,000 as
  colliding. Axes with a zero offset are not clipped or moved.
- **Fluids**: every material query iterates `floor(min) .. ceil(max)`; falling water pulls down
  next to a solid *face* (by the 1.12 face shape, stairs and the attach exclusion list excepted).
- **Shapes** (`blocks.rs`): no history-dependent shapes; fences are a post plus one box per arm
  and connect by the neighbour's face shape (same material or a gate for fence posts), panes a
  post plus arms, walls one box; doors read the other half's *decoded* properties when it is the
  same door (a half whose metadata does not carry a property has the palette's hidden default:
  the last value — facing east, closed, right hinge); chests reach towards a chest of the same
  kind; chorus plants grow arms towards chorus blocks (and end stone below).
- **Packets**: a bare on-ground packet only when `on_ground` changed (no idle packets), the
  position refresh on the 20th tick; teleports clamp x / z to ±3e7 and the pitch to ±90 and are
  answered with `teleport_confirm` (sent by the client crate) then `position_look`.
- Auto-jump is not modelled (bots never auto-jump; the oracle runs with it off).

## Verification

```sh
export CARGO_TARGET_DIR=$PWD/target/physics
cargo test -p ghast-physics                      # no Java needed
cargo test --release -p ghast-physics --features oracle-cache --test fuzz_cache -- --nocapture
cargo bench -p ghast-physics
```

`cargo test` runs:

- `tests/golden.rs`: every committed vector — `tests/golden/1.8.9/scenarios.jsonl.gz` (80 cases,
  3,245 ticks, incl. 4 item-use cases) and `fuzz_seed1.jsonl.gz` (100 × 60 = 6,000 ticks).
  Interaction packets in the records (`block_place` / `block_dig` of item use) are the client
  crate's and are not compared. Every recorded state field
  (position, box, velocity, all flags, timers, speeds, input, `sent` bookkeeping, the derived
  `touching_lava` / `climbing`) and every packet is compared by its bits; the tick-0 record must
  round-trip too. A mismatch prints case, first differing tick, field path, expected and actual
  (hex + decimal). Result: **0 diffs**.
- `src/v1_8/tests.rs`: all 4,442 lines of `collision_boxes.jsonl.gz` (every id × meta alone and in
  the dumped neighbour configurations, the 184 "settled" lines with `Settled`, the rest with
  `Constructor`) produce exactly the oracle's boxes; all 9,648 `collision_boxes_far.jsonl.gz` lines
  (absolute boxes at three far positions, incl. just past 2^24) match too — this caught the
  float-built cake box whose rounded min exceeds its max there (vanilla's box constructor sorts);
  the block-flag table equals the dump's block lines.
- `math`: the sine table's bit hash equals the one a JDK builds.

Large fuzz (feature `oracle-cache`, reads the git-ignored `oracle/cache/1.8.9/fuzz/*.jsonl.gz` or
`$GHAST_FUZZ`):

```sh
for s in 11 12 13 14 15; do   # add --use-item true for item-use spans
  oracle/run.sh fuzz --seed $s --count 2000 --ticks 100 --out "$PWD/oracle/cache/1.8.9/fuzz/physics_s$s.jsonl.gz"
done
for s in 1 2 3 4 5 6; do   # wider palette: doors, chests, anvils, liquids, events, item use
  python3 crates/ghast-physics/tools/extra_fuzz.py --seed $s --count 2000 --ticks 80 \
      --out "$PWD/oracle/cache/1.8.9/fuzz-in/extra_s$s.jsonl.gz"
  oracle/run.sh replay --in "$PWD/oracle/cache/1.8.9/fuzz-in/extra_s$s.jsonl.gz" \
      --out "$PWD/oracle/cache/1.8.9/fuzz/extra_s$s.jsonl.gz"
done
```

Extras fuzz (server speed modifiers of every operation incl. negative ones, depth strider,
spectator no-clip): `oracle/run.sh fuzz --extras true --seed S --count 2000 --ticks 100 --out
"$PWD/oracle/cache/1.8.9/fuzz-extras/extras_sS.jsonl.gz"`; seeds 41-45 = 1,000,000 ticks, 0 diffs.

Edge fuzz (partial-height blocks next to each other and to full blocks, landing step-ups):
`oracle/run.sh fuzz --edges true --seed S --count 2000 --ticks 100 --out
"$PWD/oracle/cache/1.8.9/fuzz-edges/edges_sS.jsonl.gz"`; seeds 31-35 = 1,000,000 ticks, 0 diffs
(about 320 landing step-ups per 200k ticks). The committed `fuzz_edges_seed1.jsonl.gz` and the
`landing_step_*` scenarios cover the case a live Grim test raised.

Last run (2026-11-03): 24,000 cases, **2,160,000 ticks, 0 diffs** (oracle fuzz 1,000,000 ticks,
oracle fuzz with `--use-item true` 200,000 ticks (seed 21), extra fuzz 960,000 ticks; per 160k extra ticks about 14k in water with flow, 4.5k in lava, 7k in
cobweb, 21k flying, 1.8k outside the border, 1,100 teleports, 1,300 abilities and 2,400 velocity
events; seeds 4-6 add item use and origins at ±2^24).

1.16.5: `MC=1.16.5 oracle/run.sh fuzz --seed S --count 2000 --ticks 100 --out
"$PWD/oracle/cache/1.16.5/fuzz/physics_sS.jsonl.gz"`; seeds 11-15 = 1,000,000 ticks, 0 diffs (plus the
committed `tests/golden/1.16.5/` scenarios and `fuzz_seed1`, replayed by `tests/golden_1_16_5.rs`).

1.12.2: `tests/golden_1_12_2.rs` replays the committed `tests/golden/1.12.2/` scenarios (122 cases),
`fuzz_seed1` and `fuzz_edges_seed1`; `src/v1_12/tests.rs` checks every line of the 1.12.2 collision
dumps (6,083 box lines incl. neighbour configurations, 12,192 far lines) and the stairs face shapes.
Large fuzz: the same commands with `MC=1.12.2` and `oracle/cache/1.12.2/` (seeds 11-15, 21 with
`--use-item true`, extra 1-6 replayed from `fuzz-in/`, edges 31-35, extras 41-45) = 4,160,000 ticks,
0 diffs.

**Total randomized oracle fuzz: 9,320,000 ticks, 0 diffs** — 1.8.9 4,160,000 (the 2,160,000 above,
1,000,000 extras, 1,000,000 edges), 1.12.2 4,160,000 and 1.16.5 1,000,000.

## Performance

`cargo bench -p ghast-physics` (x86-64 WSL2, release + fat LTO), per tick, 20-tick sequences:

| case | `MemoryWorld` (hash per column) | dense array world |
|---|---|---|
| walk on flat stone | 1.2–1.5 µs | 0.43–0.67 µs |
| busy: sneak on a ledge edge into fences / panes / wall / stairs / slabs under a ceiling, jumping | 1.4–1.7 µs | ~0.54 µs |

(Ranges over several runs on a machine shared with other builds.)

With the real `ghast-world` worlds (palette sections, hashed columns; measured 2026-11-04 with a
small standalone harness that builds both worlds through their chunk-data API):

| case | 1.8.9 | 1.16.5 before | 1.16.5 after |
|---|---|---|---|
| walk on flat stone | ~0.55-0.58 µs | 6.2 µs | ~2.8 µs |
| busy collision case | ~0.65-0.69 µs | 8.3 µs | ~4.8 µs |

1.16.5 is dearer by design: vanilla's collision scan covers a one-block ring around every query
box (fences and walls reach into neighbouring cells), and a tick runs several queries (pose
checks, push-out probes, the sweep). What made it cheaper without changing a single result:
- a per-tick 8x8x8 block memo around the player and a 3x3 chunk-loaded memo (`TickCache`, both
  versions);
- a per-state collision summary byte (empty / full cube / exceeds cube / moving piston) that
  rejects most cells with one table read;
- the shapes of a sweep are collected once and reused for all three axes, as vanilla's reusable
  stream does;
- nested pose boxes (0.6 < 1.5 < 1.8 high, same feet) answer each other, so the pose checks
  usually cost one query, and the four push-out probes share per-column answers;
- the shape/box overlap test runs on a stack buffer and skips vanilla's redundant
  separation pre-check.

Even the 1.16.5 busy case at ~5 µs per tick is 0.01 % of a core per bot at 20 tps.

## Not covered yet (open)

- **Movement speed modifiers** beyond the sprint modifier (speed / slowness potions arrive as
  attribute modifiers; with several multiply-total modifiers vanilla's hash-set iteration order
  matters for the last bit). `movement_speed_base` covers the base value only.
- Depth strider, riding, sleeping, spectator, portals / nausea, entity collisions (boats,
  minecarts) and entity pushing, a moving world border, server-side corrections.
- Falling below y = −64 removes the player (vanilla stops ticking it): modelled as
  `PlayerState::removed`; a real server kills the player first.
