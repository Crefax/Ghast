# ghast oracle — vanilla movement as ground truth

A small Java harness that runs the **real** Minecraft client movement code headless on a fake world and
writes what it did as golden vectors. `ghast-physics` must reproduce those vectors bit for bit; the
committed vectors let plain `cargo test` check that without Java.

Nothing from Mojang is in this repository. On first use the oracle downloads the vanilla client jar
(Mojang launcher manifest, SHA-1 checked), its libraries, and Legacy Fabric's **Legacy Yarn** mappings
(CC0) into the git-ignored `oracle/cache/`, and remaps the jar to Yarn names there. The harness is our
own code that *calls* vanilla; only Yarn / intermediary names appear in it.

## Quick start

Requirements: a JDK 17+ (`JAVA_HOME` or `java` on `PATH`; the committed vectors were made with JDK 21,
and a JDK 17 run reproduced them) and, on the first run, network access to piston-meta.mojang.com,
libraries.minecraft.net, maven.legacyfabric.net, maven.fabricmc.net and Maven Central (~25 MB, cached).

```sh
oracle/run.sh            # fetch + remap (first run only) + build + regenerate tests/golden/1.8.9/
oracle/run.sh verify     # regenerate into a temp dir, compare content with tests/golden/1.8.9/
oracle/run.sh selftest   # determinism: two runs, reverse order, read-back replay must all match
```

`oracle/run.sh golden --out DIR` writes the whole golden set into `DIR` instead of `tests/golden/<mc>/`
(every version; give an absolute path: the JVM runs in `oracle/`, a relative `DIR` is resolved there).
Before 0.4.1 `run.sh golden` dropped its arguments and always overwrote `tests/golden/<mc>/`.

`run.sh` uses `./gradlew` (Gradle 8.5, downloaded by the wrapper) unless `GRADLE=/path/to/gradle` is
set. Other commands (`oracle/run.sh help` lists them):

| Command | What it does |
|---|---|
| `fuzz --seed S --count N --ticks T [--use-item true] [--edges true] [--out F]` | N random cases (streamed; default `cache/1.8.9/fuzz/…jsonl.gz`). ~20k ticks/s. `--use-item true` adds random spans of the `use` key (cases get a `_use` suffix; everything else is the same case). `--extras true` (1.8.9, 1.12.2; 1.16.5 see its section) additionally sprinkles server speed modifiers (ops 0/1/2), depth strider boots and spectator mode from a separate random stream (cases get an `_x` suffix). `--palette edges` (or `--edges true`; every version) switches to the edge generator: strips of partial-height blocks (soul sand, slabs, snow layers, farmland, carpets, stairs, trapdoors, …) next to each other, to full blocks and to one-block steps, crossed with mostly forward + jump input, so landings and step-ups at small height differences are frequent (cases `edge_s<seed>_<i>`). |
| `scenarios [--only NAME] [--out F]` | the hand-written scenarios |
| `replay --in F [--out F]` | read cases in the golden format (header + tick `input`s) and run them in vanilla |
| `boxes [--out F]` | the collision box dump |
| `boxes-far [--out F]` | the far-position collision box dump |
| `selection [--out F]` / `rays [--seed --worlds --rays --out]` | crosshair data: 1.8.9 / 1.12.2 ray bounds and raytrace vectors; 1.16.5 / 26.1 outline + interaction shapes per state and pick vectors with entities (see "Crosshair pick (1.16.5 and 26.1)") |
| `voxels [--out F]` | 1.16.5 voxel grids of all collision shapes (`MC=1.16.5`) |
| `clicks [--seed S --count N --out F]` | window click vectors (`ClickDump`, both versions): N random cases of a screen handler + contents + 1–10 clicks through the real `ClientPlayerInteractionManager.clickSlot`; default `cache/<mc>/clicks/…`. The golden set has 3000 cases (`tests/golden/<mc>/clicks_seed1.jsonl.gz`), replayed by `crates/ghast-inventory/tests/oracle_clicks.rs`. |
| `recipes [--out F]` | 1.8.9: the client's crafting recipes in match order (`RecipeDump`) → `tests/golden/1.8.9/recipes.jsonl.gz` |
| `list` | names and descriptions of the hand-written scenarios |
| `compare DIR_A DIR_B` | compare decompressed content of golden files |
| `coverage` | regenerate `YARN_COVERAGE.md` |
| `decompile` | decompile the Yarn jar into `cache/1.8.9/decompiled/` for local reading (never commit it) |

`gradle printClasspath` prints the harness runtime classpath (for an IDE or a profiler run).

`replay` is the way for other tools (the Rust tests, a case minimiser) to ask vanilla about their own
cases: write a header line and tick lines with only `input`, and the oracle returns the full records.

## Layout

```
oracle/
  run.sh                     entry point
  build.gradle               source sets: setup (fetch/remap), common (version-agnostic), mc_1_8_9
  versions/1.8.9.properties  what to download for 1.8.9 (manifest, mapping coordinates, harness)
  versions/1.8.9.coverage    class list + notes for YARN_COVERAGE.md
  src/setup/                 Setup (download, SHA-1, tiny-remapper), Coverage (report generator)
  src/common/                Scenario model, JSON writer (bit-exact floats), SplitMix64 RNG
  src/mc_1_8_9/              the 1.8.9 harness (Harness, Recorder, Scenarios, Fuzz, CollisionDump, …)
  src/mc_1_12_2/             the 1.12.2 harness (same structure as 1.8.9, see "1.12.2" below)
  src/mc_1_16_5/             the 1.16.5 harness
  src/modern/                the shared modern (26.x) harness, compiled once per modern version (see
                             "Modern versions: one shared harness")
  src/mc_26_1/, src/mc_26_2/ the per-version shims of the modern harness (`Compat`: calls whose shape
                             differs between versions)
  cache/                     (git-ignored) jars, libraries, mappings, remapped jar, fuzz output
tests/golden/1.8.9/
  scenarios.jsonl.gz         hand-written scenarios
  fuzz_seed1.jsonl.gz        100 random cases x 60 ticks (seed 1)
  collision_boxes.jsonl.gz   collision boxes for every block id/meta + neighbour configurations
  collision_boxes_far.jsonl.gz  the alone/settled boxes again at 3 far positions, absolute coordinates
  fuzz_edges_seed1.jsonl.gz  100 edge-mode cases x 60 ticks (partial-height blocks, landings, step-ups)
  selection_boxes.jsonl.gz   ray-test bounds + canCollide for every block id/meta (crosshair)
  raytrace.jsonl.gz          crosshair raytrace vectors (50 worlds x 40 viewpoints)
  entity_sizes.jsonl.gz      entity widths / heights / eye heights / boxes per spawn path + metadata variants
  entity_tracking.jsonl.gz   (26.1) random entity packets through the real client, entity state per tick
  joins.jsonl.gz             (26.1) joins through the level-load gate (delayed load)
  net_crypto.jsonl.gz        (26.1) login encryption: session hash, hello / key packets, AES/CFB8
  chat_signing.jsonl.gz      (26.1) signed chat: session update, chat / command packets, acks
```

The version is a parameter (`MC=1.16.5 oracle/run.sh …` → `-Pmc=1.16.5`): `Setup` and `Coverage` are
version-agnostic, a version needs `versions/<mc>.properties` + `versions/<mc>.coverage` and its own
harness source set (`src/mc_x_y_z`) compiled against that version's named jar. Implemented: 1.8.9,
1.16.5 (see "1.16.5" below), 1.12.2 (see "1.12.2" below), 26.1 (see "26.1" below) and 26.2 (see
"26.2" below). The modern versions share one harness tree instead of one copy each (next section).

## Modern versions: one shared harness

`src/modern/` (package `ghast.oracle.modern`) is the whole harness for 26.x: every generator and
dump. `versions/<mc>.properties` names it with `harness.shared=modern`, and `harness.sourceSet`
names the version's shim set (`src/mc_26_1/`, `src/mc_26_2/`). Gradle compiles the shared tree plus
that one shim set against that version's jar, so each version is checked by `javac` against its own
classes. The shim is one class, `Compat`, with the same static methods in every version: the calls
whose shape changed (moved or renamed classes, changed constructors, stand-ins for subclassed
client systems). Constant holders that move between versions are avoided altogether: the harness
looks entity types and items up by id (`Reg`), which reaches the same objects in every version.
`harness.label` lets a patch release run against an existing row (26.1.1 and 26.1.2 write `26.1`
headers and `verify` compares them with `tests/golden/26.1/`).

The restructuring was checked by regenerating every 26.1 golden file: all 28 are content-identical
to the ones the per-version 26.1 harness wrote, and `selftest` passes.

## How a run works (1.8.9)

What is real vanilla code:
- `ClientWorld` with its `ClientChunkProvider`; chunk columns are loaded with the real
  `ClientWorld.handleChunk` and filled by writing raw `id << 4 | meta` values into `ChunkSection`s, i.e.
  exactly what chunk data packets decode to (a meta a block does not use decodes to **air**).
- `ClientPlayerEntity`, created by the real `ClientPlayerInteractionManager` the way
  `MinecraftClient.connect` does it (`createPlayer`, `flipPlayer`, `afterSpawn`, `spawnEntity`,
  `KeyboardInput`, `copyAbilities`, camera entity, `setGameMode`).
- Input is a set of held keys, fed through `KeyBinding.setKeyPressed` (what the LWJGL keyboard loop
  calls) and read by the real `KeyboardInput`. So sprint double-tap, creative double-tap-jump flight
  toggle, sneak scaling etc. are vanilla.
- Each tick calls the real `World.tickEntities()`, which runs the chunk bookkeeping and
  `ClientPlayerEntity.tick()`: `tickMovement` (sprint/fly toggles, pushOutOfBlocks, jump, travel with
  friction / water / lava / ladders, `Entity.move` with collisions, step-up, sneak edge clipping, block
  contact effects — web, soul sand, slime) and then `sendMovementPackets`.

What is faked (and why it does not change movement):
- `MinecraftClient` is allocated without its constructor; only `options`, `world`, `player`,
  `interactionManager`, `cameraEntity`, `soundManager` (a silent subclass) and the static instance are
  set. The tick only touches those.
- `ClientPlayNetworkHandler` is the real class with `sendPacket` overridden to capture packets.
- Server packets used for setup and events are applied the way their client handlers do it:
  position/look (no relative flags: velocity zeroed, `updatePositionAndAngles`), abilities, entity
  velocity (`setVelocityClient(v / 8000)`), movement speed attribute base value (entity properties),
  status effects (`addStatusEffect`; on the client this does **not** add attribute modifiers — speed
  effects reach the client only as attribute modifiers, jump boost / blindness are read directly).

Order inside one tick (matches `MinecraftClient.tick`): scenario events (inbound packets) → key states
→ yaw/pitch (the renderer applies mouse look between ticks) → `world.tickEntities()` → record state and
the packets sent during the tick.

Setup: after the position packet the oracle does the chunk bookkeeping an earlier tick would have done
(`checkChunk(player, false)`), so tick 1 is a normal tick. Tick 0 in the output is the state after
setup; **initialise the Rust state from the tick 0 record**, not from the header.

Block bounds: 1.8 blocks keep their shape in mutable per-`Block` fields. The harness snapshots them
after bootstrap and restores them before every run (and every collision-dump query), so each case sees
fresh-JVM state no matter what ran before. See "History-dependent shapes" below for the three blocks
where that state decides the collision box.

### Item use

If any tick of a case holds the `use` key, the player gets a diamond sword (item 276) in hotbar slot 0
at setup. Before the world tick (where `MinecraftClient.tick` handles the use key): a rising edge while
not using calls the real `ClientPlayerInteractionManager.interactItem` (sends `block_place`; the sword
starts blocking for 72000 ticks), a falling edge while using calls `stopUsingItem` (sends `block_dig`
status 5). While using, `tickMovement` scales the input by 0.2, resets the sprint double-tap timer and
blocks sprint starts; forward 0.2 < 0.8 also stops an active sprint.

## History-dependent shapes (anvil, chest, trapped chest)

In 1.8.9 the collision of anvils (145), chests (54) and trapped chests (146) uses the block's mutable
bounds as they were left by the last shape update of *any* block of that type (`setBoundingBox(world,
pos)`, run e.g. by the crosshair raytrace); collision itself never recomputes them. Without a shape
update the bounds are the constructor ones: anvil = full cube, chest = 1/16-inset single chest even
next to another chest.

**Policy: ghast models the "settled" shape** — the bounds the block's own update sets from its state
and neighbours (anvil by orientation, chest by double-chest connection). Grim, the judge on real
servers, models these blocks by state/connection, and a real client has almost always refreshed the
bounds of nearby blocks.

In `collision_boxes.jsonl.gz` lines for these blocks carry `shape_source`:
- `"settled"` (config `settled`): `setBoundingBox(world, pos)` is called right before the collision
  query — every meta alone, and chests with a chest of each meta on each side (plus a chest of the other
  kind, which does not connect). **These are the reference for ghast.**
- `"constructor"` (configs `alone`, `chest_neighbour`): no shape update, i.e. what the oracle's movement
  harness would see. Kept for reference; not the ghast policy.

The movement harness does not settle bounds before each tick, so anvils and chests are left out of the
fuzz palette and of the hand-written scenarios.

Not covered (port these from elsewhere or add them later):
- `ClientPlayerInteractionManager.tick` (held-item sync, block breaking), depth strider (no
  inventory), riding, sleeping, portals, nausea. Item use is covered only as the `use` key with a
  sword (blocking, never finishes); eating/drinking (which end via a server packet) is not.
- Other entities (no entity collision boxes or pushing), server corrections. (Spectator mode works: the
  harness builds the tab-list entry `isSpectator` reads, like the player list packet would.)
- `ClientWorld.tick()` (time, weather, lighting, random display ticks) — no effect on movement.

## Golden file format

JSON lines, UTF-8. `.gz` files are plain gzip (`zcat` to read). A file is a sequence of cases; each
case is one `scenario` line followed by its `tick` lines (`t` = 0, 1, …, N). The collision dump has its
own line types (below).

### Floats

Every `f64`/`f32` value is a two-element array `[decimal, "hex"]`. `hex` is the raw IEEE bits
(`f64::from_bits(u64::from_str_radix(hex, 16))`, 16 digits; `f32::from_bits`, 8 digits) and is
authoritative; `decimal` is Java's shortest round-trip form for humans and `null` for NaN/∞. Integers
and booleans are plain JSON.

### `scenario` line

| key | meaning |
|---|---|
| `type` | `"scenario"` |
| `format` | `1` |
| `version` | `"1.8.9"` |
| `name`, `description` | identifier and text |
| `outside` | `"air"`: every chunk in `loaded_chunks` is loaded, all unlisted blocks are air; `"unloaded"`: only the listed chunk columns are loaded |
| `loaded_chunks` | list of inclusive chunk rectangles `[cx0, cz0, cx1, cz1]`; columns outside are **not loaded** (an `EmptyChunk`: no blocks, no collision; see quirks) |
| `fills` | world edits in order: `[x0, y0, z0, x1, y1, z1, id, meta]`, inclusive, later fills overwrite earlier ones; every other position is air |
| `setup` | the inputs that built the initial state: `game_mode` (`survival`/`creative`/`adventure`), `pos` (f64×3, feet), `yaw`, `pitch` (f32), `vel` (f64×3), `on_ground`, `fall_distance`, `sprinting`, `flying`, `allow_flying` (optional, else from game mode), `fly_speed`, `walk_speed`, `movement_speed_base` (f64), `food_level`, `effects` `[[id, amplifier], …]`; optional (1.8.9 and 1.16.5) `speed_modifiers` `[[amount (f64), operation 0/1/2], …]` (applied like the entity properties packet: base value set, all modifiers replaced, one UUID per modifier; the client's own sprint modifier is added on top while sprinting) and `depth_strider` (diamond boots with that level in the armor slot); optional `bb` (f64×6: the exact box set after the position packet instead of the one derived from `pos` — use it to restart a recorded state bit for bit, since the box carries rounding history the position does not) and, 1.16.5 only, optional `pose` (set before `bb`) |
| `ticks` | number of tick records after tick 0 |

### `tick` line

| key | meaning |
|---|---|
| `type`, `t` | `"tick"`, tick number (0 = after setup) |
| `input` | absent for `t = 0`. `keys`: comma-separated held keys from `forward,back,left,right,jump,sneak,sprint,use` (empty string = none; `use` = use key held while the crosshair hits nothing, see "Item use"); `yaw`, `pitch` (f32) set before the tick; optional `events` (applied first, in order): `{"type":"velocity","x","y","z"}` (raw 1/8000 units, ints), `{"type":"teleport","pos":[f64×3],"yaw","pitch","flags"?}` (`flags` = the packet's relative-flags byte: bit 0 x, 1 y, 2 z, 3 yaw, 4 pitch; set bits make the value relative, absent/0 = absolute), `{"type":"abilities","invulnerable","flying","allow_flying","creative","fly_speed","walk_speed"}`, `{"type":"speed_attribute","base":f64,"modifiers":[["<uuid>", f64, op], …]}` (entity properties packet for the movement speed: base set, ALL modifiers replaced in that order — if the server's list lacks the sprint modifier `662a6b8d-da3e-4c1c-8813-96ea6097278d`, the sprint bonus is gone until sprinting restarts, as in vanilla), `{"type":"effects","effects":[[id, amplifier], …]}` (the full active effect set after an add/remove effect packet: missing ids are removed, listed ones added/updated) |
| `state` | player state **after** the tick (below) |
| `packets` | packets the client sent during the tick, in order (below) |

`state` fields (Yarn name of the vanilla field in parentheses where it is not obvious):

| key | type | meaning |
|---|---|---|
| `pos` | f64×3 | `x`, `y`, `z` (feet). After a move these are derived from the box: x = (minX+maxX)/2, y = minY, z = (minZ+maxZ)/2 |
| `bb` | f64×6 | bounding box `minX, minY, minZ, maxX, maxY, maxZ` — the real position state |
| `vel` | f64×3 | `velocityX/Y/Z` after the tick |
| `yaw`, `pitch` | f32 | |
| `on_ground`, `horizontal_collision`, `vertical_collision`, `colliding` | bool | results of the last move (`colliding` = either) |
| `velocity_dirty` | bool | set by jumps/knockback; informational |
| `fall_distance` | f32 | |
| `in_web` | bool | set by cobweb contact after the move, consumed by the next move |
| `touching_water` | bool | (`touchingWater`) |
| `touching_lava`, `climbing` | bool | `isTouchingLava()`, `isClimbing()` evaluated after the tick |
| `sprinting`, `sneaking` | bool | `isSprinting()`, `isSneaking()` (sneak = held sneak key) |
| `outside_world_border` | bool | entity flag maintained by the collision query (world border handling) |
| `no_clip`, `step_height` | bool, f32 | constant here (false, 0.6) |
| `flying`, `allow_flying`, `creative`, `invulnerable`, `fly_speed`, `walk_speed` | | `PlayerAbilities` |
| `jump_cooldown` | int | (`jumpingCooldown`) |
| `jumping` | bool | jump input copied into the entity this tick |
| `forward_speed`, `sideways_speed` | f32 | entity input after the ×0.98 of travel |
| `flying_speed` | f32 | (`flyingSpeed`) air acceleration for the **next** tick (0.02, or 0.026 sprinting) |
| `movement_speed` | f32 | `LivingEntity.movementSpeed` field (set from the attribute after travel; players read the attribute directly) |
| `movement_speed_attr` | f64 | movement speed attribute value incl. modifiers (sprint ×1.3) |
| `fly_toggle_timer` | int | (`abilityResyncCountdown`) creative double-tap window |
| `sprint_toggle_timer` | int | (`field_1763`) sprint double-tap window |
| `sprinting_ticks_left` | int | (`ticksSinceSprintingChanged`) counts down from 600 while sprinting |
| `food_level`, `ticks_alive` | int | |
| `using_item` | bool | `isUsingItem()`; **only present in cases that hold the `use` key** |
| `input` | object | `KeyboardInput` after this tick: `forward`, `sideways` (f32, sneak-scaled), `jump`, `sneak`. The next tick compares against these for the toggles |
| `sent` | object | what the server was last told (`sendMovementPackets` bookkeeping): `pos` (f64×3: x, bb.minY, z), `yaw`, `pitch`, `sprinting`, `sneaking`, `ticks_since_pos` (position forced at ≥ 20) |

`packets` entries, named like minecraft-data's 1.8 `play.toServer` packets:
`{"name":"flying","on_ground"}`, `{"name":"position","pos","on_ground"}`, `{"name":"look","yaw","pitch","on_ground"}`,
`{"name":"position_look","pos","yaw","pitch","on_ground"}` (`pos` y is the box minY),
`{"name":"entity_action","action":id,"action_name"}` (0 start_sneaking, 1 stop_sneaking, 3 start_sprinting, 4 stop_sprinting),
`{"name":"abilities","invulnerable","flying","allow_flying","creative","fly_speed","walk_speed"}`,
`{"name":"block_place","location":[-1,-1,-1],"direction":255,"item":276}` (start using the held item),
`{"name":"block_dig","status":5,"status_name":"release_use_item"}` (stop using). A
`teleport` event also records the `position_look` reply that the client's position handler sends.

### `collision_boxes.jsonl.gz`

`{"type":"block", "id", "name", "slipperiness" (f32), "full_cube", "normal_block", "full_block",
"render_as_normal_block", "has_collision", "material_blocks_movement", "material_fluid",
"material_solid", "material_opaque", "material_replaceable", "climbable"}` — one per registered id
(Yarn semantics: `full_cube` = `Block.isFullCube()` = opaque material, normal render, not a power
source; `normal_block` = `Block.isNormalBlock()`; see YARN_COVERAGE.md).

`{"type":"boxes", "id", "meta", "config", "neighbours", "state", "boxes", "exact", "history_dependent"?,
"history_metas"?}` — the boxes `Block.appendCollisionBoxes` adds for the raw value `id << 4 | meta`
placed at (0, 64, 0), with the listed neighbours `[dx, dy, dz, id, meta]` (everything else air).
`state` is the decoded block state string, `null` when the meta decodes to air. `boxes` are relative to
the block position (f64×6 each, `[minX, minY, minZ, maxX, maxY, maxZ]`); `exact` = adding the position
back reproduces vanilla's absolute values bit for bit (true for every line). `config` = `alone` (all
id/meta pairs, with the history check), `stone_sides`, `one_side`, `self_*` (fences, panes, bars,
walls), `stair_neighbour`, `settled` (anvils and chests, see below), `door_lower_with_upper` / `door_upper_with_lower`, `chest_neighbour`,
`gate_between_walls`. `history_dependent` means the result changes after the same block computed its
shape elsewhere (another meta, alone or next to a copy of itself): anvils, chests, trapped chests; their
lines also carry `shape_source` (`constructor` / `settled`, see "History-dependent shapes").

Note: the box values are floats widened to double (`0.1F` → `0.10000000149011612`), and vanilla adds
them to the integer block position in double precision.

### `collision_boxes_far.jsonl.gz`

Some blocks build their collision box in **float** arithmetic on the block position
(`pos.getX() + 0.0625F` is an int + float sum), so far from the origin the box is not
`(double) pos + local shape`. This file repeats the `alone` lines (and the `settled` lines of anvils
and chests) at three far reference positions: (29999000, 200, -29999000), (-1234567, 5, 7654321) and
(-16777217, 100, -16777219) (just past 2^24, where floats no longer hold every integer).

`{"type":"boxes_abs", "id", "meta", "config" ("alone"/"settled"), "pos": [x, y, z], "state", "boxes",
"same_as_local", "shape_source"?}` — `boxes` are **absolute** (f64×6 each, `[decimal, "hex"]`);
`same_as_local` = every coordinate equals the local shape from `collision_boxes.jsonl.gz` added to `pos` in
double precision, bit for bit.

Result: `same_as_local` is false only for **cactus (81), cake (92) and all fence gates (107, 183–187)**,
at all three positions; e.g. a cactus at x = 29999000 has minX = maxX = 29999000.0 (the 0.0625 inset is
lost in float). Soul sand and snow layers also use float arithmetic, but only on y, which is exact for
y ≤ 255, so they always match. Reproduce the float sums exactly in Rust for those blocks:
`(pos.x as f32 + 0.0625f32) as f64` etc.

### Landing step-up cases (1.8.9 `scenarios.jsonl.gz`)

`landing_*` scenarios cover the 1.8.9 step-up that runs while landing (`Entity.move`: step-up when x/z
was clipped and the entity was on ground **or** the move's dy was clipped while falling): soul sand
into grass (bunny hop, sprint-jump, walking off 1/2/3-block ledges), 1..7 snow layers into an edge at
most 0.5 higher, farmland (collides as a full block in 1.8.9) into a slab-topped block, carpet into a
slab, slabs into a full block. For the `landing_*` variants after `landing_step_carpet_ledge`, the
spawn offset, yaw (0..30°), strip width and arrival (walk / sprint / hop) were chosen by running
vanilla until the case contains a landing step-up; the result is an ordinary fixed scenario.

### `selection_boxes.jsonl.gz` and `raytrace.jsonl.gz` (1.8.9, crosshair)

`{"type":"selection", "id", "meta", "config", "neighbours", "state", "bounds", "can_collide",
"can_collide_liquid", "has_collision_box"}` — `bounds` (f64×6, block-local) are the block's bounds
right after `setBoundingBox(world, pos)`, which is what `Block.rayTrace` tests (and what the outline
box `getSelectionBox` adds to the position); `can_collide` / `can_collide_liquid` =
`Block.canCollide(state, false / true)` — `World.rayTrace` skips blocks for which it is false (air,
fire, liquids without the liquid flag, portals, …); `has_collision_box` matters for the
`ignoreBlockWithoutBoundingBox` flag. Configs: every id/meta `alone`, plus the neighbour sets of
`collision_boxes.jsonl.gz` (fences, panes, bars, walls, stairs, doors, chests) and tripwire over stone.
Stairs are special: their ray test runs 8 passes over half-cube sub-boxes (stateful fields of the
block, reset between runs), so use the raytrace vectors for them rather than `bounds`.

`raytrace.jsonl.gz`: 50 worlds (the `fuzz_seed1` terrain generator, header = the usual `scenario`
line) with 40 viewpoints each: `{"type":"ray", "k", "feet" (f64×3), "yaw", "pitch", "head_yaw"
(f32), "sneaking", "eye_height" (f32), "reach" (4.5 survival / 5.0 creative), "eyes", "look"
(f64×3), "pick", "ray_fff"}`. `pick` is the real crosshair pick `Entity.rayTrace(reach, 1.0F)`
(= `World.rayTrace(eyes, eyes + look * reach, false, false, true)`, which also reports the last
non-collidable block on a miss); `ray_fff` is `World.rayTrace(…, false, false, false)`. Results:
`{"type": "block" | "miss" | "null", "block": [x, y, z], "side", "hit": f64×3}`. Note: at tick delta
1.0 the look vector of a living entity uses `pitch` and **`headYaw`** (the client's tick-time pick runs
before the world tick, so `headYaw` is the previous tick's yaw); `eyes` = feet + 1.62 (− 0.08 sneaking).

### `entity_sizes.jsonl.gz` (1.8.9 and 1.16.5)

Entity dimensions as the client sees them. Common fields of `{"type":"entity_size", …}`: "id", "name",
"variant", "width", "height", "eye_height" (f32), "box" (f64×6); `{"type":"entity_size", …, "error"}` marks
something the client could not construct (none at present).

**1.8.9** — every entity is built the way the client builds it from its spawn packet, at feet
(0.5, 64, 0.5) or block (0, 64, 0):
- `"spawn":"mob"` (spawn_mob): `EntityType.createInstanceFromRawId(type)` + `updatePositionAndAngles`, as in
  `ClientPlayNetworkHandler.onMobSpawn`. One line per non-abstract living registry id (Pig = 90 included;
  "Mob" 48 / "Monster" 49 are abstract and never spawned).
- `"spawn":"object"` (spawn_entity): the per-type constructors of `onEntitySpawn`, with `"object_type"`
  (packet type: 1 boat, 2 item, 10 minecart, 50 TNT, … 90 fishing bobber) and `"data"` (packet data: the
  minecart type 0..6, the item frame facing, the potion value, …; -1 for the fishing bobber, whose data
  is its owner's entity id). These differ from the registry
  constructors (e.g. the XP orb is 0.5 here, 0.25 via the registry). Item frames have one line per
  facing (`"facing"`, `"block_pos"`).
- `"spawn":"xp_orb"`, `"spawn":"painting"` (one line per motive × facing: `"motive"`, `"facing"`,
  `"facing_id"`, `"block_pos"`; the box depends on both, width/height stay 0.5),
  `"spawn":"global"` (lightning, not in the registry), `"spawn":"player"` (another player,
  `OtherClientPlayerEntity`), `"spawn":"client_player"`.
- "id" is the registry id (`EntityType` class → id map), -1 when the entity has none (lightning,
  fishing bobber, players). "pos" (f64×3) is the entity position after spawning.
- "collision_border" = `getTargetingMargin` (the 0.1 growth used when picking entities).

Variants start from a freshly spawned entity; metadata goes through `DataTracker.writeUpdatedEntries`
(what the entity metadata packet does, including the entity's tracked-data hook). `"applied"` says when
the client changes the size: `metadata` (immediately), `next_tick` (the size is set in a client tick
method — passive babies in `tickMovement`, wither skeleton in `tickMovement`, armor stand marker in
`tick()` — and the dump runs that step), `input` (client player sneak), `server` (not a client path).
- `baby`: passive mobs incl. horses (metadata 12 = -1). Zombies / zombie pigmen (metadata 12 = 1): **the
  1.8.9 client never resizes them** — a baby zombie keeps the adult 0.6 × 1.95 box (only the eye height
  changes); `baby_server` (`setBaby(true)`) is the server's 0.3 × 0.975. `villager` (zombie metadata 13):
  no size change.
- `wither` (skeleton type 1: 0.72 × 2.535). A type-0 skeleton on the client is 0.6 × **1.8** (the client
  never applies the server's 0.6 × 1.95). `elder` (guardian flags bit 2: 1.9975). `size_1..4` (slime /
  magma cube metadata 16; `default` is the constructor size before any metadata). Armor stand `small`
  (halves only the eye height), `marker` / `small_marker` (0 × 0). Player `sneaking` (eye height only).
- Horse types (metadata 19) and zombie villagers do not change the size in 1.8.9; horse types have no
  lines.
- **Boxes after a size change are not centred**: `Entity.setBounds` keeps the box's min corner, so a
  resized entity's box runs from the old min corner until its next `updatePosition` (any move or
  interpolation step) recentres it. The dumped "box" is the state right after the change; recentre with
  "pos" ± width / 2 if you need the steady state. Some objects already spawn off-centre: small fireballs
  and wither skulls are positioned with the 1.0 fireball box and shrunk to 0.3125 afterwards, so their
  box starts at pos - 0.5.

**1.16.5** — `{"type":"entity_type", "id", "name", "width", "height", "fixed"}` (the type's registered
dimensions) for every type, including those without a client instance (player, fishing bobber,
lightning; the client player is measured separately as "minecraft:player (client player)"), and
`entity_size` lines with "box" at feet (0.5, 64, 0.5), "targeting_margin", "pose", "fixed",
"poses": {pose: [width, height, eye_height]}. Variants: `baby` (passive mobs via the age setter; zombies,
piglins, … via `setBaby`), `size_1..4` (slime, magma cube), armor stand `small` / `marker` /
`small_marker`.

## Determinism and verification

- All outputs are written atomically (a `.<name>.partial` file in the same directory, then renamed), so
  a test run reading `tests/golden/` never sees a half-written file. Keep anything that must survive
  a session (replay cases, big fuzz runs) under `oracle/cache/…`, not in a temporary directory.
- `selftest`: the golden set generated twice in one JVM; every hand-written and golden-fuzz case rerun
  in reverse order after the collision dump has disturbed block state; and the written files read back
  with `replay` — all must reproduce identical content. Last result: pass.
- `verify`: a fresh JVM regenerates the golden set and compares it with `tests/golden/1.8.9/`.
- Compare *decompressed* content across machines: gzip bytes may differ between zlib builds.

## 1.16.5 (second version)

Run everything with `MC=1.16.5`, e.g. `MC=1.16.5 oracle/run.sh`, `MC=1.16.5 oracle/run.sh verify`,
`MC=1.16.5 oracle/run.sh fuzz --seed 7 --count 1000`. Mappings: Fabric **Yarn** `1.16.5+build.10`
(CC0, mergedv2). Mojang's official mappings are not used. Name report: `YARN_COVERAGE_1.16.5.md`.
Output: `tests/golden/1.16.5/` — `scenarios.jsonl.gz` (61 cases), `fuzz_seed1.jsonl.gz` and
`fuzz_edges_seed1.jsonl.gz` (100 x 60 each), `collision_shapes.jsonl.gz`, `collision_shapes_far.jsonl.gz`,
`voxel_shapes.jsonl.gz`.

Setup note: some 1.16.5 classes carry parameter-annotation tables that do not match their descriptor
and javac refuses to compile against them, so `prepareVanilla` drops parameter annotations from the
remapped jar (`remap.stripParameterAnnotations`; nothing reads them at run time).

### How a 1.16.5 run works

More of the client is real than in 1.8.9:
- **Inbound packets are real.** Setup and events are encoded into bytes and decoded by the real packet
  classes, then given to the real `ClientPlayNetworkHandler` handlers: player abilities, entity
  properties (movement speed base + modifiers), health/food, status effects, equipment (boots),
  position/look (which sends the real `teleport_confirm` + `position_look` reply), entity velocity.
- **Chunks arrive as chunk data packets.** Each column is built like a server holds it, serialised with
  `ChunkDataS2CPacket` and decoded by `ClientChunkManager.loadChunkFromPacket` (palette, biomes).
- **Tags are vanilla.** The client gets block/fluid tags from the server; the oracle builds them with
  the vanilla tag loader from the data pack inside the jar and applies them like the tag packet does
  (ladders/vines/scaffolding climbable, water/lava fluid tags, soul speed blocks).
- **The connection captures everything sent**, also packets sent outside the handler's `sendPacket`.
- Faked: `MinecraftClient` (allocated; options, sound manager (silent), tutorial manager, thread,
  game renderer camera, world renderer (no-op), world, player, interaction manager, camera entity),
  `GameOptions` (allocated: its constructor creates key bindings through `InputUtil`, whose static
  initialiser loads the GLFW native library; the movement key bindings are allocated directly, sprint
  and sneak as hold-mode `StickyKeyBinding`s), and the tab-list entry (gives `isSpectator`).
- The player list entry is built for the game mode, so **spectator mode works** (no-clip flight).
- Item use (`use` key): a **shield** in the main hand (swords no longer block). Rising edge →
  `interactItem` → `use_item` packet; falling edge → `stopUsingItem` → `block_dig` status 5.
- `auto_jump` is a setup field (vanilla's option default is on; bots turn it off; the scenarios use
  off except `auto_jump`).
- Not covered: elytra / fall flying (the 1.16.5 client starts gliding itself — `checkFallFlying` sets
  the flag when an elytra is in the chest slot — but only the server clears it; no case equips one),
  riptide, boats and other vehicles, entity collisions, pistons pushing the player, a non-default or
  moving world border, the nether (lava pushes 0.007 instead of 0.0023333), powder snow (1.17),
  held-item packets, `ClientWorld.tick()`.

### 1.16.5 fuzz

`fuzz` takes the same options as 1.8.9 / 1.12.2; what they generate in 1.16.5:
- plain (`Fuzz.one`): random terrain from a palette of movement-relevant blocks, each in a random one
  of its states; pools, deep water, climbable walls (ladder / vine / scaffolding), ceilings; creative
  flight, initial velocity, low food, an op-2 speed modifier, jump boost / levitation / slow falling /
  dolphin's grace, soul speed / depth strider boots, auto-jump, velocity events.
- `--palette edges` (`Fuzz.edge`, cases `edge_s<seed>_<i>`): strips of partial-height 1.16 blocks
  (soul sand / soil, slabs, snow layers, farmland, grass paths, carpets, honey, slime, stairs,
  trapdoors, scaffolding, sea pickles, lanterns, lily pads, stonecutters, chests, campfires, bells,
  fences, walls, ice kinds, water, cobwebs, berry bushes, bamboo, …) next to each other and to one-block
  steps, mostly forward + jump input; now and then auto-jump, soul speed boots, jump boost and a low
  ceiling (stone, slabs or scaffolding three or four blocks up).
- `--extras true` (`Fuzz.withExtras`, the plain case plus a separate stream, cases `…_x`): speed
  modifiers of every operation, another movement speed base, depth strider / soul speed up to level 5,
  food 5–7, fly speeds, the client-read effects (jump boost, blindness, levitation, slow falling,
  dolphin's grace), spectator mode, a pool with a bubble column through the spawn, the whole case
  moved next to the world border (mostly just inside it, facing it), and inbound packets during the
  case: entity properties (sometimes echoing the client's sprint modifier), effect changes, abilities
  (a spectator may be sent `flying` false), teleports with random relative flags. Combines with
  `--use-item true` (spectators cannot use items: `interactItem` passes).

Big runs (git-ignored `oracle/cache/1.16.5/`, ~4.4 M ticks, ~1 GB; replayed by the Rust large-fuzz
test, all at 0 diffs):

```sh
for s in 11 12 13 14 15 16 17 18 19 20; do MC=1.16.5 oracle/run.sh fuzz --seed $s --count 2000 --ticks 100 --out "$PWD/oracle/cache/1.16.5/fuzz/physics_s$s.jsonl.gz"; done
MC=1.16.5 oracle/run.sh fuzz --seed 21 --count 2000 --ticks 100 --use-item true --out "$PWD/oracle/cache/1.16.5/fuzz/physics_use_s21.jsonl.gz"
for s in 31 32 33 34 35; do MC=1.16.5 oracle/run.sh fuzz --palette edges --seed $s --count 2000 --ticks 100 --out "$PWD/oracle/cache/1.16.5/fuzz-edges/edges_s$s.jsonl.gz"; done
for s in 41 42 43 44 45; do MC=1.16.5 oracle/run.sh fuzz --extras true --seed $s --count 2000 --ticks 100 --out "$PWD/oracle/cache/1.16.5/fuzz-extras/extras_s$s.jsonl.gz"; done
MC=1.16.5 oracle/run.sh fuzz --extras true --use-item true --seed 46 --count 2000 --ticks 100 --out "$PWD/oracle/cache/1.16.5/fuzz-extras/extras_use_s46.jsonl.gz"
```

### 1.16.5 format differences

- `fills` entries are `[x0, y0, z0, x1, y1, z1, state]` with the **global block state id**
  (`Block.STATE_IDS`, the id chunk/block packets use); the header adds `"states": {"id": "name[...]"}`
  for every id used.
- `setup` adds `speed_modifiers` (`[[amount, operation], …]` as an entity properties packet sends them),
  `auto_jump`, `soul_speed`, `depth_strider` (boots enchantment levels).
- Teleport events get ids 1, 2, … (the setup teleport is id 0).
- `state` fields: `pos`, `bb`, `vel`, `yaw`, `pitch`, `on_ground`, `horizontal_collision`,
  `vertical_collision`, `fall_distance`, `movement_multiplier` (f64×3, cobweb / berry bush slowdown for
  the next move), `pose` (`standing`/`crouching`/`swimming`/…), `width`, `height`, `eye_height`,
  `touching_water`, `submerged_in_water`, `water_height`, `lava_height` (f64), `in_lava`, `first_update`,
  `climbing`, `swimming`, `sprinting`, `sneaking`, `in_sneaking_pose`, `fall_flying`, `using_item`,
  `no_clip`, `step_height`, abilities (`flying`, `allow_flying`, `creative`, `invulnerable`, `fly_speed`,
  `walk_speed`), `jump_cooldown`, `jumping`, `forward_speed`, `sideways_speed`, `upward_speed`,
  `flying_speed`, `movement_speed`, `movement_speed_attr` (f64), `last_block_pos`,
  `fly_toggle_timer`, `sprint_toggle_timer` (`ticksLeftToDoubleTapSprint`),
  `ticks_since_sprinting_changed` (counts up), `auto_jump_enabled`, `ticks_to_next_autojump`,
  `food_level`, `age`, `velocity_modified`, `submerged_fluid` (eye fluid tag: `water`/`lava`/null),
  `input` (1.8.9 fields plus `pressing_forward/back/left/right`) and `sent` (adds `on_ground`).
- `packets` use minecraft-data 1.16.2 names: `flying`, `position`, `look`, `position_look` (`pos` y is
  the feet y), `entity_action` (0 press_shift_key, 1 release_shift_key, 3 start_sprinting,
  4 stop_sprinting, 8 start_fall_flying), `abilities` (`flying` only), `teleport_confirm`
  (`teleport_id`), `use_item` (`hand`), `block_dig` (`status`), `held_item_slot`.

### `collision_shapes.jsonl.gz` / `collision_shapes_far.jsonl.gz` (1.16.5)

- `{"type":"block", "name", "first_state", "states", "default_state", "slipperiness",
  "velocity_multiplier", "jump_velocity_multiplier", "collidable", "climbable", "soul_speed_block",
  "offset_type"}` for every block (f32 values as `[decimal, "hex"]`).
- `{"type":"shape", "id", "state", "boxes", "full_cube", "solid_block", "suffocates", "fluid"
  (null/"water"/"lava"), "fluid_level"?, "fluid_still"?, "fluid_height"?, "contexts"?}` for every one of
  the 17112 states, placed at (0, 64, 0): `boxes` = `getCollisionShape(world, pos, ShapeContext.absent())`
  as its box list (block-local).
- **Entity-context policy:** shapes are state-driven in 1.16, but the collision query passes the moving
  entity. The dump evaluates five player contexts — feet `below` (y - 1.8), `inside` (y + 0.5),
  `above` (y + 1), `above_descending`, `inside_descending` (descending = sneaking) — and adds
  `contexts: {name: boxes}` only for states where some context differs from the context-free shape.
  Result: only **scaffolding** (32 states) depends on the context (solid top only when the feet are
  above it and not sneaking; the bottom-flag "collision shape" when above). Fluids' "walk on fluid" shape
  is empty for players in every context.
- Far file: `{"type":"shape_abs","id","pos","boxes","same_as_local"}` — the shape offset to three far
  positions (as `BlockCollisionSpliterator` offsets it). Only **bamboo** (random per-position model
  offset) differs from local + pos. The 1.8.9 float-arithmetic blocks (cactus, cake, fence gates) are
  exact in 1.16 (VoxelShapes in double).

### `voxel_shapes.jsonl.gz` (1.16.5)

The collision code of 1.16 works on voxel grids (`VoxelShape.calculateMaxDistance` walks grid cells),
so this file records the grids themselves, deduplicated:

- `{"type":"block", "name", "first_state", "states", "default_state", "slipperiness",
  "velocity_multiplier", "jump_velocity_multiplier" (f32), "tags": {"climbable", "fences", "walls",
  "fence_gates", "trapdoors", "soul_speed_blocks", "fire"}}`.
- `{"type":"voxel_shape", "sid", "class", "sizes": [nx, ny, nz], "points_x", "points_y", "points_z",
  "filled", "empty", "boxes"}` — every distinct shape once (before the first state that uses it).
  `class` = `simple` (`SimpleVoxelShape`: points are i/n of `sizes`, coordinate index = floor(c * n)
  clamped) or `array` (`ArrayVoxelShape`: explicit points) or `sliced`; `points_*` = the shape's
  `getPointPositions(axis)` as f64 `[decimal, "hex"]` (n + 1 values); `filled` = one `0`/`1` per cell,
  cell (x, y, z) at index `(x * ny + y) * nz + z` (the `BitSetVoxelSet` order); `boxes` =
  `getBoundingBoxes()` for convenience.
- `{"type":"context", "name", "feet_dy", "feet_y" (f64), "block_y", "descending"}` — the player
  contexts used for `context_shapes`: `below` (dy -1.8), `inside` (0.5), `above` (1.0),
  `above_descending`, `inside_descending`, and `feet_<dy>` / `feet_<dy>_descending` for dy = -0.1,
  -2.0E-5, -1.0E-6, 0.0, 1.0E-6, 0.01, 0.05, 0.1, 0.124, 0.125, 0.13, 0.999, 0.99998, 0.999995, 1.0
  (feet y = 64 + dy in double; the block is at y = 64).
- `{"type":"state", "id", "state", "collision_shape": sid, "context_shapes"?: {context name: sid},
  "full_cube_identity", "empty_identity", "exceeds_cube", "should_suffocate", "is_air",
  "has_dynamic_bounds", "material", "material_blocks_movement", "material_liquid", "material_solid",
  "material_replaceable", "side_solid_full_square": [N, E, S, W], "side_solid_up", "side_solid_down",
  "fluid": null | {"kind", "level", "falling", "still", "height", "height_fluid_above"}}` for every
  state id. `collision_shape` is the context-free shape
  (`ShapeContext.absent()`); `context_shapes` appear only where a player context differs (scaffolding:
  feet `below` = y - 1.8, `inside` = y + 0.5, `above` = y + 1; `_descending` = sneaking; the held item
  only changes the outline shape). Scaffolding thresholds seen in the `feet_*` contexts: the
  bottom-flag shape (0..0.125) applies once the feet are above `y - 1.0E-5F` (empty at dy -2.0E-5,
  bottom shape from dy -1.0E-6 on); the full scaffolding shape applies only when the feet are above
  `y + 1 - 1.0E-5F` and the player is not descending (dy 0.99998 → bottom shape, 0.999995 → full).
  `empty_identity` = the shape object is `VoxelShapes.empty()`; `full_cube_identity` = the shape object is `VoxelShapes.fullCube()`
  (the fast path of `BlockCollisionSpliterator`); `has_dynamic_bounds` = no shape cache; `material` =
  the Yarn field name of the `Material` (`ice`, `dense_ice`, …); fluid heights are f32
  (`FluidState.getHeight` alone and with the same fluid above). Moving pistons and shulker boxes are
  dumped without block entities.

### 1.16.5 quirks (observed in the vectors)

- `ClientWorld.isChunkLoaded` is always true: the client player always ticks and the 1.8.9
  "unloaded column ⇒ velocityY = -0.1" rule never applies (`walk_into_unloaded`: normal gravity, no
  collision in the empty column).
- No idle `flying` packets: a bare on-ground packet is sent only when `on_ground` changed; position is
  still forced every 20 ticks.
- `teleport` → `teleport_confirm` then `position_look` with `on_ground` false, before the tick's own
  packets.
- Sneaking = crouching pose (height 1.5), input ×0.3 whenever `in_sneaking_pose` (also when the gap
  above is too low to stand). A 1-high space gives the swimming pose (height 0.6) on land.
- Soul speed: the client only removes soul sand's 0.4 velocity multiplier; the speed boost is a
  server-side attribute modifier (`soul_speed_server_boost` sends it like the server:
  `(double)(0.03F * (1 + 0.35F * level))`, operation 0).
- Default world border radius 29,999,984 (collides like a wall: `world_border`).
- Sliding down a honey wall caps the falling speed (`honey_wall_slide`); berry bushes / cobwebs set
  `movement_multiplier`, consumed by the next move.

## 1.12.2 (third version)

Run everything with `MC=1.12.2`, e.g. `MC=1.12.2 oracle/run.sh`, `MC=1.12.2 oracle/run.sh verify`,
`MC=1.12.2 oracle/run.sh fuzz --seed 7 --count 1000`. Mappings: Legacy Fabric **Legacy Yarn**
`1.12.2+build.604` (CC0, mergedv2). Name report: `YARN_COVERAGE_1.12.2.md` (1.12.2 Legacy Yarn leaves
much of the movement code unnamed: `travel` is `LivingEntity.method_2657`, the input acceleration is
`Entity.method_2492`; see its "Names to read carefully" table). Output: `tests/golden/1.12.2/` with the
same file names as 1.8.9 (`scenarios` 112 cases, `fuzz_seed1` and `fuzz_edges_seed1` 100 x 60,
`collision_boxes`, `collision_boxes_far`, `selection_boxes`, `raytrace`, `entity_sizes`, `recipes`,
`clicks_seed1`, `clicks_craft_seed1` 3000 cases each) plus the 1.12-only `face_shapes`.

Setup: the remapped jar needs `remap.stripParameterAnnotations` (javac rejects some inner-class
constructor annotation tables), `remap.fixPackageAccess` and `remap.publicProtected` (protected members
become public: after moving the single official package into Yarn packages, protected calls on another
instance from a subclass in a different package fail verification, e.g. horses at bootstrap).
`Bootstrap.initialize` redirects `System.out` / `System.err` into its logger; the harness restores them.

### How a 1.12.2 run works

The harness is the 1.8.9 one (same structure, same scenario model) with these differences:
- Blocks are written as states: a raw `id << 4 | meta` value goes through `Block.BLOCK_STATES.fromId`
  and becomes air when no state has it — exactly what the chunk palette does on the wire (global palette
  and section palettes both map an unknown id to air). The raw → state table is built from every state
  of every block (`id << 4 | getData(state)`, the last state per raw value wins); tripwire registers
  metas 0–14 only.
- 1.12 blocks keep no mutable bounds: there are no history-dependent shapes (anvils and chests are in
  the fuzz palette).
- The auto-jump option is off (`GameOptions.field_14902 = false`; vanilla's default is on, bots never
  auto-jump). `ClientPlayerEntity.field_14951`, the per-tick copy of that option, starts true and is only
  refreshed at the end of `sendMovementPackets`; the harness sets it to false at setup (as if an earlier
  tick had copied the option), otherwise the first tick of a case could still auto-jump. The tutorial manager is set (no tutorial step: no effect).
- Teleport events: the position handler's reply is recorded as `teleport_confirm` (ids 1, 2, … per
  case) followed by `position_look` (`on_ground` false), before the tick's own packets. Positions are
  clamped to ±3.0E7 and the pitch to ±90 by `updatePositionAndAngles`.
- Item use (`use` key): a **shield** in the main hand (swords no longer block). Rising edge →
  `interactItem` (`method_12234`, sends `use_item` with the hand); falling edge → `stopUsingItem`
  (`block_dig` status 5). While the hand is active the input is ×0.2.
- Effects are sent by id like 1.8.9; 25 = levitation (in the fuzz and the `levitation*` scenarios).
- Not covered: elytra / fall flying (needs the server flag), vehicles, entity collisions (boats,
  shulkers), frost walker, auto-jump on.

### 1.12.2 format differences (vs 1.8.9)

- Header `version` `"1.12.2"`; `fills` keep `[x0, y0, z0, x1, y1, z1, id, meta]` (1.12 ids 0–255).
- `state` adds `width`, `height` (f32; 0.6 × 1.65 while sneaking, see quirks) and `upward_speed`
  (Legacy Yarn `LivingEntity.forwardSpeed`, which is the vertical travel input, always 0);
  `forward_speed` is `field_16513` (the forward input). `sprinting_ticks_left` keeps its key but holds
  `ticksSinceSprintingChanged`, which **counts up** in 1.12 (reset to 0 by every sprint change).
  `using_item` (only in cases using the `use` key) is "a hand is active" (`method_13061`).
  `sent` adds `on_ground` (`field_13457`, the last on_ground sent).
- `packets` use minecraft-data 1.12.2 names: `flying`, `position`, `look`, `position_look`,
  `entity_action` (0 start_sneaking, 1 stop_sneaking, 3 start_sprinting, 4 stop_sprinting, 8
  start_fall_flying), `abilities`, `teleport_confirm` (`teleport_id`), `use_item` (`hand`: 0 main,
  1 off; Legacy Yarn calls the class `SwingHandC2SPacket`), `arm_animation` (`hand`), `held_item_slot`
  (`slot`), `block_place` (`location`, `direction`, `hand`), `block_dig` (`status`, `status_name`).
- `collision_boxes.jsonl.gz`: `{"type":"block", "id", "name", "slipperiness", "has_collision",
  "material" (Yarn `Material` field name), "material_blocks_movement", "material_fluid",
  "material_solid", "material_opaque", "material_replaceable", "climbable"}`; the per-state flags moved
  to the `"alone"` box lines: `full_cube` (`method_11730`), `normal_cube` (`method_11734`, the
  pushOutOfBlocks test), `block_normal_cube` (`method_11733`), `top_solid` (`method_11739`),
  `opaque_cube` (`isFullBoundsCubeForCulling`), `full_block`, `causes_suffocation` (`method_13763`),
  `material`. Every box line has `actual_state` (the neighbour-aware state) when registered. Boxes come
  from `BlockState.appendCollisionBoxes(world, pos, mask, list, player, false)` — what the world's
  collision query calls. Configs: `alone`, `stone_sides`, `one_side` (55 neighbour kinds, incl. slabs,
  stairs, trapdoors, shulker boxes, concrete: 1.12 connects fences / panes / walls by the neighbour's
  face shape), `self_south`, `self_north_south`, `self_all`, `self_north_south_stone_above`,
  `self_east_west_wall_above`, `stair_neighbour` (oak, stone, purpur stairs), `door_lower_with_upper` /
  `door_upper_with_lower` (oak, iron, spruce), `door_stack` (oak, iron, spruce: each of the 12 metas
  with each of the 12 metas of the same door, and of another door kind, directly above and directly
  below — the actual state reads the other half's decoded properties whenever the neighbour is the same
  door, whatever half it is), `chest_neighbour` / `chest_other_kind`,
  `gate_between_walls` / `gate_between_walls_ns`, `chorus_sides` (all 64 combinations of chorus plants
  around a chorus plant) and `chorus_one_side`. No `shape_source` / `history_*` fields.
- `collision_boxes_far.jsonl.gz`: `boxes_abs` lines for every id × meta `alone` (no "settled").
- `selection_boxes.jsonl.gz`: same schema as 1.8.9; `bounds` = the state's outline box (Legacy Yarn
  `BlockState.getCollisionBox(view, pos)`, which is NOT the collision box), `has_collision_box` =
  `method_11726(view, pos) != null`. `raytrace.jsonl.gz` as 1.8.9.
- `entity_sizes.jsonl.gz`: as 1.8.9, `name` is the registry identifier's path (`"zombie"`,
  `"armor_stand"`; lightning `"lightning_bolt"`, players `"player"`); object types of 1.12 (3 area effect
  cloud, 67 shulker bullet, 68 llama spit, 79 evoker fangs, 91 spectral arrow, 93 dragon fireball);
  variants `baby` (passive mobs and zombies — 1.9+ clients resize zombies on the metadata too), slime
  `size_1..4`, armor stand `small` / `marker` / `small_marker` (`next_tick`), players `sneaking`
  (1.65 high, applied by the next tick's size update).
- Clicks: `inv` has 41 entries (main 0–35, armor 36–39 feet first, offhand 40); window 0 has 46 slots
  (45 = offhand, clickable directly). Mode numbers are `ItemAction` ordinals (0 pickup, 1 quick move,
  2 swap, 4 throw, 6 pickup all). The 1.12 client **never computes crafting results itself**
  (`ScreenHandler.method_14205` only runs on the server); in `--craft` cases the harness puts the
  server's result (`RecipeDispatcher.matches`) into the result slot once after filling the grid, as the
  server's set_slot would; it is not recomputed after clicks (a vanilla client keeps the stale result
  until the server answers).
- `face_shapes.jsonl.gz` (`faces` command, 1.12.2 only): the face shape 1.12 fences / panes / walls
  connect by. `{"type":"faces_block", "id", "name", "fence_excluded", "wall_excluded", "pane_excluded"}`
  per registered block (the real static exclusion tests `FenceBlock.method_14325`,
  `WallBlock.method_14353`, `PaneBlock.method_14348`, called by reflection), then
  `{"type":"faces", "id", "meta", "config", "neighbours", "state", "faces":{"down", "up", "north",
  "south", "west", "east"}}` for every id × meta `alone` (air neighbours, same position as the collision
  dump) and for every `stair_neighbour` configuration of the collision dump (the centre stairs' faces).
  Values are the enum names of `BlockState.getRenderLayer(view, pos, direction)` (Legacy Yarn calls
  1.12's face-shape enum `BlockRenderLayer`: `SOLID`, `BOWL`, `CENTER_SMALL`, `MIDDLE_POLE_THIN`,
  `CENTER`, `MIDDLE_POLE`, `CENTER_BIG`, `MIDDLE_POLE_THICK`, `UNDEFINED`).
- `recipes.jsonl.gz`: `{"i", "name", "type":"shaped", "w", "h", "in":[[[id,data]…]…], "out":hex}`,
  `{"i", "name", "type":"shapeless", "in":…, "out":hex}`, `{"i", "name", "type":"special", "class"}` in
  `RecipeDispatcher.REGISTRY` order (the order `matches` tries); every ingredient is the list of stacks
  it accepts (empty list = empty cell), data 32767 = any.

### 1.12.2 quirks (observed in the vectors)

- No idle `flying` packets: a bare on-ground packet is sent only when `on_ground` changed
  (`idle_on_ground_only`); the position is forced every **20** ticks (the counter is incremented before
  the test; 1.8.9: 21).
- Player size: at the end of `PlayerEntity.tick` (after the move, before the packets) the size follows
  the sneak key: 0.6 × 1.65 sneaking, 0.6 × 1.8 standing, applied only if the new box is free
  (`sneak_low_ceiling`: sneaking fits under a 1.75 gap, releasing sneak there keeps the small box).
  The box is rebuilt from its min corner (`minX + width`, …), so the max corner gets new rounding.
- Sneak edge guard probes `stepHeight` (0.6) down instead of 1.0: a sneaking player walks down a 0.5
  drop (`sneak_edge_small_drop`).
- Velocity cut-off 0.003 (1.8.9: 0.005); the input acceleration (`method_2492`) converts the yaw with `yaw * 0.017453292F`.
- Above an unloaded column (`floor(x)`, `floor(z)`; 1.8.9 truncated) the client still ticks and gets
  `velocityY = -0.1` (`unloaded_negative_column`).
- Climbing also on an open trapdoor above a ladder with the same facing (`trapdoor_ladder_climb`).
- Beds bounce (`vy = -vy * 0.66F` unless sneaking, `bed_bounce`); levitation replaces gravity
  (`vy += (0.05 * (amp + 1) - vy) * 0.2`, `levitation`).
- Creative flight while sneaking divides the input by 0.3 again (`flying_sneak_horizontal`).
- Collision shapes: fences are a post plus one box per arm, panes/bars a post plus arms; ladders 3/16;
  farmland and grass paths 15/16; lily pads 1/16 inset, 0.09375 high; piston head arms 1.25 long.

Big fuzz runs (git-ignored `oracle/cache/1.12.2/`, replayed by the Rust large-fuzz test):

```sh
for s in 11 12 13 14 15; do MC=1.12.2 oracle/run.sh fuzz --seed $s --count 2000 --ticks 100 --out "$PWD/oracle/cache/1.12.2/fuzz/physics_s$s.jsonl.gz"; done
MC=1.12.2 oracle/run.sh fuzz --seed 21 --count 2000 --ticks 100 --use-item true --out "$PWD/oracle/cache/1.12.2/fuzz/physics_use_s21.jsonl.gz"
for s in 31 32 33 34 35; do MC=1.12.2 oracle/run.sh fuzz --edges true --seed $s --count 2000 --ticks 100 --out "$PWD/oracle/cache/1.12.2/fuzz-edges/edges_s$s.jsonl.gz"; done
for s in 41 42 43 44 45; do MC=1.12.2 oracle/run.sh fuzz --extras true --seed $s --count 2000 --ticks 100 --out "$PWD/oracle/cache/1.12.2/fuzz-extras/extras_s$s.jsonl.gz"; done
```

The 1.12.2 fuzz palette is the 1.8.9 one plus frosted ice, grass paths, magma, bone blocks, concrete
(powder), glazed terracotta, purpur blocks / slabs / stairs / pillars, observers, shulker boxes, end rods,
chorus plants / flowers, beds, anvils, chests, doors, more fences / gates, hay, barriers, structure voids,
open trapdoors above ladders, and the levitation effect.

## 26.1 (fourth version, protocol 775)

Run everything with `MC=26.1`, e.g. `MC=26.1 oracle/run.sh`, `MC=26.1 oracle/run.sh verify`,
`MC=26.1 oracle/run.sh selftest`, `MC=26.1 oracle/run.sh fuzz --seed 7 --count 1000 --ticks 100`.
Output: `tests/golden/26.1/` — `scenarios.jsonl.gz` (73 cases: the 61 of 1.16.5 plus 12 for 26.1
mechanics), `fuzz_seed1.jsonl.gz` (100 × 60), `collision_shapes.jsonl.gz`, `collision_shapes_far.jsonl.gz`,
`voxel_shapes.jsonl.gz`, `entity_sizes.jsonl.gz`. Commands: `golden`, `scenarios`, `fuzz` (plain and
`--use-item true`), `replay`, `boxes`, `boxes-far`, `voxels`, `entity-sizes`, `list`, `selftest`, `compare`,
`clicks` (see "Window clicks (26.1)"). `fuzz` takes `--palette edges` and `--extras true` (see "26.1 fuzz" below); the
golden set adds `fuzz_edges_seed1.jsonl.gz` and `fuzz_extras_seed1.jsonl.gz` (100 × 60 each).

Setup: **26.1 ships unobfuscated** — its version json has no `client_mappings` download and the classes
carry their real names (`net.minecraft.client.player.LocalPlayer`, …). There is nothing to remap:
`versions/26.1.properties` has no `mappings.repo`, `prepareVanilla` only fetches the jar and libraries
(SHA-1 checked) and the harness compiles against `client-official.jar` in the git-ignored cache. So the
harness is written against the jar's own names (it only calls vanilla; nothing from the jar is copied
into the repository). There is no name coverage report (`coverage` refuses). The jar is Java 25 class
files: `java.version=25` makes Gradle compile and run this harness with a Java 25 toolchain (detected,
or downloaded through the foojay resolver plugin in `settings.gradle`); Gradle itself and the other
versions keep the JDK Gradle runs on (17 or 21; Gradle 8.5 cannot run on 25).

### How a 26.1 run works

Real vanilla code:
- **Configuration phase.** The vanilla data pack inside the jar is loaded with the server's own loaders
  in `WorldLoader.load` order (static-registry tags, worldgen registries, dimension registries; the
  reloadable server resources — recipes, loot, advancements, functions — are not part of the
  configuration phase and are not loaded). The server's registry sync (`RegistrySynchronization.
  packRegistries`, no packs known to the client, so every element is sent in full) and tag sync
  (`TagNetworkSerialization`) are encoded into real `ClientboundRegistryDataPacket` /
  `ClientboundUpdateTagsPacket` bytes, decoded, and handed to the real `RegistryDataCollector`, which
  builds the client's registry access as `handleConfigurationFinished` does for a remote connection
  (this also binds the block / fluid / item / entity tags the movement code reads: climbable, water,
  lava, soul speed blocks, …). Dimension type, biomes, enchantments, damage types come from there.
- A real `ClientPacketListener` per run (constructed from a `CommonListenerCookie`), the movement part of
  its `handleLogin` (game mode object, `ClientLevel` with the overworld dimension type, `LocalPlayer`
  from `MultiPlayerGameMode.createPlayer`, `resetPos`, `addEntity`, `KeyboardInput`, `adjustPlayer`,
  camera entity, `setLocalMode`), and real inbound packets, each encoded with its `STREAM_CODEC` and
  decoded again before its real handler runs: player info (tab list entry: game mode, so spectator
  works), chunk cache centre, abilities, update attributes, health / food, mob effects, equipment,
  player position (`handleMovePlayer`, which sends the real `teleport_confirm` + `position_look`), entity
  motion.
- **Chunks** are built like a server holds them (`LevelChunk` sections, primed heightmaps), serialised as
  `ClientboundLevelChunkPacketData` and decoded by `ClientChunkCache.replaceWithPacketData`.
- Real `Options` (constructed on an empty directory: vanilla defaults — auto-jump off, sprint window 7,
  hold-to-sneak / sprint) and real `KeyMapping` / `ToggleKeyMapping`s. A key press is `setDown` plus a
  queued click, as the keyboard handler does.
- Each tick runs the movement part of `Minecraft.tick` in its order: inbound packets (events), keys,
  yaw / pitch (mouse look happens between ticks), `tickRateManager().tick()`, `MultiPlayerGameMode.tick`
  (held-item sync), the use-key part of `handleKeybinds` (release while using; queued clicks and the
  held key with the 4-tick right-click delay start `MultiPlayerGameMode.useItem` with the crosshair on
  nothing), `ClientLevel.tickEntities()` (the whole `LocalPlayer.tick`: input, sprint / fly toggles,
  pose, travel, collisions, block effects, movement packets, the player-input packet), then the
  `ServerboundClientTickEndPacket` that `Minecraft.tick` sends.

Faked, and why it does not change movement:
- `Minecraft` is allocated without its constructor (no window, GL, sound device). Set: the instance,
  game thread + packet processor (packet handlers check the thread), options, tutorial (real, never
  started), user properties / profile futures and offline services (read by the chat abilities of a new
  player and by the player info handler), a game renderer holding only a real `Camera` (camera entity,
  sound / particle distance), a `Gui` holding an allocated debug overlay (the packet listener's
  constructor asks it for a ping logger), and the stand-ins below. The tick paths that run only read
  these fields.
- Stand-ins (allocated subclasses whose reachable hooks are no-ops): sound manager, particle engine,
  level renderer, block entity render dispatcher, player social manager. None of them returns anything
  the movement code reads (the particle engine returns no particle; particles and sounds draw from the
  level / entity randoms, which are seeded with 0 instead of the time so every run is identical).
- The `Connection` is the real class with no channel; `send` captures every packet in order.
- The rest of `handleLogin` (level loading screen, debug renderer, chat session, telemetry, toasts) is
  not run. The level counts as loaded right after setup: the harness calls the listener's own
  `notifyPlayerLoaded` (sends `player_loaded`), so the player ticks from tick 1 — a real client keeps
  the player frozen until `LevelLoadTracker` reports the level ready.
- Light is not sent (no light packet): no movement code reads light.

Weakest points (where it could differ from a real session): (1) the login is a hand-picked subset of
`handleLogin` — a future change that puts movement state into one of the skipped calls would be missed;
(2) attributes, depth strider and soul speed reach a 26.1 client only as server-sent attribute
modifiers; the harness sends what a server would send at setup (soul speed's *location-dependent*
modifiers are server-side and only appear when a scenario lists them in `attribute_modifiers`); (3) the
server always sends the full registry contents (no known packs) — a real vanilla server with a vanilla
client sends references to the client's own copy of the same data pack, which gives the same values;
(4) `Minecraft.tick` steps that are skipped (gui, screen, `ClientLevel.tick` time / weather, particles,
`gameRenderer.tick`) were read and found not to touch the player's movement state.

### 26.1 format differences

- Header: `fills` are `[x0, y0, z0, x1, y1, z1, state]` with the **26.1** global block state id;
  `states` names every state id a fill uses (also fills that later fills overwrite). `setup` adds
  `attribute_bases` (`{attribute id: base}`, e.g. `minecraft:step_height`, `minecraft:gravity`,
  `minecraft:scale`) and `attribute_modifiers` (`[[attribute id, modifier id, amount, operation], …]`),
  sent in the setup attributes packet. `effects` are `[[26.1 registry id, amplifier, name], …]`
  (registry ids are 0-based: jump boost 7, blindness 14, levitation 24, slow falling 27, dolphin's grace
  29). `soul_speed` / `depth_strider`: diamond boots with that enchantment; the enchantment's
  `minecraft:attributes` effects (depth strider: `water_movement_efficiency`) are sent as the server
  sends them. Velocity events keep the 1/8000 units; 26.1 sends the vector as an `LpVec3` (lossy).
  Speed attribute events: modifier UUIDs become `oracle:<uuid>` identifiers (the sprint UUID is
  `minecraft:sprinting`).
- `state` fields: as 1.16.5 where the concept exists (`pos`, `bb`, `vel`, `yaw`, `pitch`, `on_ground`,
  `horizontal_collision`, `vertical_collision`, `velocity_modified` (`hurtMarked`), `movement_multiplier`
  (`stuckSpeedMultiplier`, **zero** when unset), `pose`, `width`, `height`, `eye_height`,
  `touching_water` (`isInWater`), `submerged_in_water` (`isUnderWater`), `submerged_fluid`,
  `water_height`, `lava_height`, `in_lava`, `first_update` (`firstTick`), `climbing` (`onClimbable`),
  `swimming`, `sprinting`, `sneaking` (shift key), `in_sneaking_pose` (`LocalPlayer.crouching`),
  `fall_flying`, `using_item`, `no_clip`, `step_height` (`maxUpStep`), abilities, `jump_cooldown`
  (`noJumpDelay`), `jumping`, `forward_speed` (`zza`), `sideways_speed` (`xxa`), `upward_speed` (`yya`),
  `flying_speed` (`Player.getFlyingSpeed()`, computed live in 26.1), `movement_speed` (`getSpeed()`),
  `movement_speed_attr`, `last_block_pos`, `fly_toggle_timer` (`jumpTriggerTime`), `sprint_toggle_timer`
  (`sprintTriggerTime`), `auto_jump_enabled`, `ticks_to_next_autojump` (`autoJumpTime`), `food_level`,
  `age` (`tickCount`)), with these changes: `fall_distance` is **f64** (26.1 keeps it in a double), no
  `ticks_since_sprinting_changed`; new: `minor_horizontal_collision`, `vertical_collision_below`,
  `supporting_block` (`mainSupportingBlockPos`, `[x, y, z]` or null), `on_ground_no_blocks`,
  `in_powder_snow`, `was_in_powder_snow`, `visually_crawling`, `attributes` (values of movement_speed,
  step_height, gravity, jump_strength, sneaking_speed, movement_efficiency, water_movement_efficiency,
  scale, safe_fall_distance). `input` is `{"move_vector": [x (left), y (forward)] f32, "keys": {forward,
  backward, left, right, jump, shift, sprint}}` (26.1's `ClientInput`); `sent` has `pos` (`xLast`,
  `yLast`, `zLast`), `yaw`, `pitch`, `on_ground`, `horizontal_collision`, `sprinting` (`wasSprinting`),
  `ticks_since_pos` (`positionReminder`) and `input` (`lastSentInput`).
- `packets` use minecraft-data 1.21.x names: `flying` / `position` / `look` / `position_look` (now with
  `horizontal_collision`), `entity_action` (`ServerboundPlayerCommandPacket`: the sprint actions;
  sneaking is no longer an entity action), `player_input` (the seven key flags, sent when they change),
  `tick_end` (every tick), `player_loaded`, `abilities` (`flying`), `teleport_confirm`, `use_item`
  (`hand`, `sequence`, `yaw`, `pitch`), `block_dig` (`status`, `sequence`), `held_item_slot`,
  `arm_animation`.
- `collision_shapes.jsonl.gz`: as 1.16.5; block lines have `offset` (the block has an offset function)
  instead of `offset_type`; contexts add `above_falling` / `inside_falling` (fall distance 3) and
  `above_leather_boots` / `above_leather_boots_descending` — powder snow reads both from the entity.
  Context-dependent states: scaffolding (32) and powder snow.
- `voxel_shapes.jsonl.gz`: as 1.16.5 (same `simple` / `array` / `sliced` class names for
  `CubeVoxelShape` / `ArrayVoxelShape` / `SliceShape`); 26.1 has no block materials, so the `material*`
  fields are replaced by `blocks_motion`, `liquid`, `solid`, `replaceable`, `has_offset`; context lines
  add `fall_distance`, `leather_boots`.
- `entity_sizes.jsonl.gz`: as 1.16.5; `entity_type` lines add `eye_height`; instances are created with
  `EntityType.create(level, EntitySpawnReason.LOAD)` (what the add-entity handler does). Every
  `entity_size` line carries `poses`: per `Pose` the instance's width, height, eye height and
  `fixed` after `setPose(pose)` (the cached dimensions the client refreshes on a pose update; some
  types look at their own pose, e.g. the digging warden). The `default` line adds `living`
  (`instanceof LivingEntity`), `interpolation` (steps of the instance's `InterpolationHandler`,
  `-1` = none: position packets snap the entity; `0` = displays), `pickable` (`isPickable()` of the
  fresh instance), and for mobs `mob`, `max_head_y_rot` and `body_control` (the simple class name of
  its `BodyRotationControl`). Variant lines add `meta`: the synched data entries
  that differ from the default instance, `[index, serializer id, value]` (the baby flag, slime size,
  armor stand client flags: the metadata that selects the variant).

### 26.1 fuzz

- plain (`Fuzz.one`): the 1.16.5 generator (same palette and random stream; `grass_path` is
  `dirt_path` and `chain` is `iron_chain` in 26.1) with 26.1 effect ids. Same-named block states of
  both versions have identical collision geometry (only the box decomposition of 938 states —
  stairs, beds, anvils, … — differs) and identical slipperiness / speed / jump factors.
- `--palette edges` (`Fuzz.edge`, cases `edge_s<seed>_<i>`): the 1.16.5 strips of partial-height
  blocks with the palette extended by 26.x blocks (powder snow, mud, pointed dripstone, big
  dripleaf, sculk sensors / shrieker, decorated pot, copper grate / chain / lantern / trapdoor,
  moss carpets, mangrove roots, amethyst, candles, lightning rod, heavy core, vault, trial spawner,
  azalea, dried ghast, shelves, creaking heart, cave vines, sniffer egg, copper golem statue, …);
  sometimes leather boots, a step height (0 – 2), a scale (0.25 – 2), a jump strength, auto-jump,
  soul speed, jump boost, a low ceiling (stone, slabs, scaffolding, powder snow).
- `--extras true` (`Fuzz.withExtras`, the plain case plus a separate stream, cases `…_x`): speed
  modifiers of every operation and another speed base; bases and modifiers of all three operations
  for step height, gravity, jump strength, sneaking speed, movement / water movement efficiency,
  scale (0.0625 – 16) and safe fall distance; depth strider / soul speed up to V; leather boots; with
  `--use-item true` an item with its own `use_effects` (can sprint, speed multiplier 0 – 1); the
  sprint window option (0 – 10); food 5 – 7; fly speeds; the client-read effects (jump boost,
  blindness, levitation, slow falling, dolphin's grace, weaving); spectator or adventure mode; 26.x
  partial blocks and powder snow around the spawn; a bubble-column pool; the case moved next to
  the world border; and inbound packets during the case: movement-speed attributes (sometimes
  echoing the sprint modifier), other attribute packets (also scale: the box is rebuilt at the end
  of that tick), effect changes, abilities, motion packets up to 3 blocks per tick (the
  fall-distance raycast of `Entity.move`), teleports with all nine relative flags (position,
  rotation, delta, rotate-delta).
- Header additions (only when set): `leather_boots`, `use_effects` `{can_sprint, speed_multiplier}`,
  `sprint_window`; event `{"type":"attribute", "attribute", "base", "modifiers": [[id, amount, op],
  …]}` (an attributes packet entry for any attribute).
- `replay` maps fill state ids through the header's `states` names, so a file written by 26.1 replays
  on a jar that numbers states differently (used for the family check, below).

Big runs (git-ignored `oracle/cache/26.1/`, replayed by the Rust large-fuzz test):

```sh
for s in 11 12 13 14 15 16 17 18; do MC=26.1 oracle/run.sh fuzz --seed $s --count 2000 --ticks 100 --out "$PWD/oracle/cache/26.1/fuzz/physics_s$s.jsonl.gz"; done
MC=26.1 oracle/run.sh fuzz --seed 22 --count 2000 --ticks 100 --use-item true --out "$PWD/oracle/cache/26.1/fuzz/physics_use_s22.jsonl.gz"
for s in 31 32 33 34 35; do MC=26.1 oracle/run.sh fuzz --palette edges --seed $s --count 2000 --ticks 100 --out "$PWD/oracle/cache/26.1/fuzz-edges/edges_s$s.jsonl.gz"; done
for s in 41 42 43 44 45; do MC=26.1 oracle/run.sh fuzz --extras true --seed $s --count 2000 --ticks 100 --out "$PWD/oracle/cache/26.1/fuzz-extras/extras_s$s.jsonl.gz"; done
MC=26.1 oracle/run.sh fuzz --extras true --use-item true --seed 46 --count 2000 --ticks 100 --out "$PWD/oracle/cache/26.1/fuzz-extras/extras_use_s46.jsonl.gz"
```

(`physics_s11` of the first 26.1 run holds 1600 cases, `physics_use_s21` 400.)

### 26.1 joins through the level-load gate (`joins.jsonl.gz`)

Scenarios with `setup.delayed_load: true` start like a real join: the harness does not call
`notifyPlayerLoaded`; `handleLogin`'s `LevelLoadTracker.startClientLoad` runs (the multiplayer
tracker of `ConnectScreen`, no close delay) and every step runs the level-load part of
`ClientPacketListener.tick` where `MultiPlayerGameMode.tick` calls it (after the tick's packets,
before the entities tick). Events drive it: `level_chunks_load_start` (the server's game event 13,
through the real `handleGameEvent`), `section_compiled` (what the stub level renderer's
`isSectionCompiledAndVisible` returns from then on: the real renderer compiles the player's section
some frames after its chunk arrived, which a scenario places at a tick), `load_timeout` (the 30 s
wall-clock deadline of the tracker's client state set to 0). While the level is not loaded the
harness does not turn the player (the loading screen holds the mouse; it closes during the gate
tick). Tick lines add `client_loaded` (`hasClientLoaded()`), and the first record keeps what the
client sent during setup (teleport confirm + position/look). Cases: feet on the ground, 1, 1.5 and
5 blocks above it, in water; section compiled 0, 1, 5, 20 ticks after the load start; compiled
before the load start; the timeout; a second position packet while loading; no load start; dead at
join (`setup.health` 0, the set-health packet's value: the tracker lets a dead player in without
the section).

### 26.1 teleports with delta movement (`teleports.jsonl.gz`)

26.1's position packet (`PositionMoveRotation`) carries a delta movement next to the position and
rotation, and nine relative bits (x, y, z, yaw, pitch, delta x / y / z, rotate delta). Teleport
events record it as `delta: [x, y, z]` (omitted when zero, so older files are unchanged).
`teleports.jsonl.gz` (`teleports`, in `golden`) holds 13 hand-written cases (delta under the bit
sets 0, 0x20, 0x40, 0x80, 0xE0, 0x100, 0x1E0, 0x118, 0x1FF, 0x07; entity motion before / after the
position packet in the same tick; rotate delta while falling) and 100 cases of
`Fuzz.withTeleports` (`fuzz --teleports true`): the plain case plus 1–5 position packets with
random bits (biased towards the delta bits) and deltas up to ±20, and entity-motion packets in the
same tick (either order) or the neighbouring ticks. Replayed by `golden_26_1.rs`
(`teleports_26_1`); cached `fuzz-teleports` seeds 51 and 52 (204k ticks) also at 0 diffs.

### net_crypto.jsonl.gz (26.1)

`MC=26.1 oracle/run.sh net-crypto` (`NetCryptoDump`): login encryption as
`ClientHandshakePacketListenerImpl.handleHello` does it, with the real `Crypt` and packet codecs.
One line per case (`type: "case"`): `server_id`, `should_authenticate`, the server's RSA key
(`server_public_der`, `server_private_pkcs8`, base64; 1024 bits like the vanilla server, every
sixth case 2048), `shared_secret` and `challenge` (hex), `server_hash`
(`new BigInteger(Crypt.digestData(…)).toString(16)`), `hello_body` (`ClientboundHelloPacket`
bytes), `key_body` (a `ServerboundKeyPacket` through its codec, encrypted with the same RSA PKCS#1
v1.5 cipher as `Crypt.encryptUsingKey` but with seeded padding; the dump also checks that the real
constructor's packet decrypts to the secret and the challenge; the reader decrypts with the private
key), and `encrypt` / `decrypt`: `[input hex, output hex]` chunks through one `Crypt.getCipher`
stream per direction (AES/CFB8). Deterministic (seeded `SHA1PRNG`s, `Rng` for the rest).
Read by `crates/ghast-client/src/version/v26_1_chat_golden.rs`.

### chat_signing.jsonl.gz (26.1)

`MC=26.1 oracle/run.sh chat-signing` (`ChatSigningDump`): signed chat with the real client
classes, mirroring `ClientPacketListener.setKeyPair` / `sendChat` / `sendCommand` /
`markMessageAsProcessed`; only `Instant.now()` and the salt are chosen by the dump. Lines:

- `type: "session"`: `profile_id`, `session_id`, the profile key pair (`private_pkcs8`,
  `public_spki`, base64; 2048 bits), `expires_at_ms`, `key_signature` (signed by a dump-made
  "services" key, `services_public_spki`), `session_update_body` (`ServerboundChatSessionUpdatePacket`).
- `type: "commands"`: `body`, the `ClientboundCommandsPacket` an operator gets from
  `Commands(DEDICATED)` (`fillUsableCommands` at level 4); the client dispatcher is built from it
  by `handleCommands`' node builder.
- `type: "chain"` (14): `index`, `signed` (the session's encoder or `Encoder.UNSIGNED`),
  `op_level` (entity event `24 + level` before the chain: selectors need 2), and `events`, each one
  of: `recv` (a 256-byte signature, base64) + `shown` → `addPending`, with `ack_body` when the
  `ServerboundChatAckPacket` follows (offset > 64); `chat` + `timestamp_ms` + `salt` + `body`
  (`ServerboundChatPacket` after `generateAndApplyUpdate` and the encoder); `command` +
  `signable` (`[name, value]` of `SignableCommand.of(dispatcher.parse(…))`) + `packet`
  (`chat_command` or `chat_command_signed`) + `body` (+ `timestamp_ms`, `salt` when signed).

Deterministic. Read by `crates/ghast-client/src/version/v26_1_chat_golden.rs` (every body
byte-identical).

### 26.1 movement mechanics beyond the plain player (`Ext`, `ExtCases`)

One golden file per mechanic, `tests/golden/26.1/<mechanic>.jsonl.gz` (hand-written cases + 100
golden fuzz cases, part of `golden` / `verify` / `selftest`); `MC=26.1 oracle/run.sh <mechanic>`
writes it, `ext-fuzz --mechanic <m> --seed S --count N --ticks T --out F` writes cached fuzz
(`oracle/cache/26.1/fuzz-<m>/`, replayed by the Rust large-fuzz test). A case uses this only when
it has a version setup (`setup.ext` in the header), so older files keep their bytes. Server packets
of these cases (all through the real `ClientPacketListener`):

- `{"type":"shared_flags","set","clear"}`: `set_entity_data` of the player's shared flags byte
  (the client's current byte with `clear` cleared and `set` set; bit 3 sprinting, 4 swimming, 7
  fall flying).
- `{"type":"equipment","slot","item","damage"?}`: `set_equipment` for the player.
- `{"type":"firework","id"}`: the server's rocket for a gliding player — a `FireworkRocketEntity`
  built with the server constructor (at the player, attached to it, owned by it), sent as
  `add_entity` (owner id as its data) + its non-default entity data. `{"type":"remove_entity","id"}`:
  `remove_entities`.

`setup.ext`: `chest` (`elytra`) and `chest_damage`, `hand` (the use-key item instead of the shield,
e.g. `firework_rocket`: never "in use"), `fall_flying` (flag 7 in the setup entity data),
`left_handed` (the player's main-hand entity data).

**`elytra`** (23 cases + 100 fuzz): start (jump in the air: `entity_action` `start_fall_flying`),
not on the ground / in water / with levitation / creative flight / a broken elytra (damage 431),
glide physics at every pitch, landing (the client keeps gliding until the server clears flag 7),
a ladder stops it client-side, vines (`#can_glide_through`) do not, walls, ceilings, slow
falling, rockets (one to three, used with the use key, attached to a player who does not glide,
crossing into an unloaded chunk with either arm), server sprint / swim / glide bits, joining
while gliding, the elytra taken off. Fuzz (`ExtCases.elytraFuzz`, cases `…_x_el`): the
`withExtras` case with an elytra (sometimes broken or missing), raised spawns, glide at the start,
jump presses, pitch changes, flag events, rockets and chest swaps. Quirks seen in the vectors:

- A rocket is an entity: it ticks only in a loaded chunk (a rocket spawned where the client has
  no chunk never boosts) and is discarded more than 64 blocks below the world.
- `Math.cos` (glide lift) is HotSpot x86-64's intrinsic, not the correctly rounded cosine:
  `jdk-cos --out F` writes it for every float in `[0, pi/2]` for the Rust table (see
  `crates/ghast-physics/src/modern/jdk_math.rs`).
- An attached rocket's ray that hits a big dripleaf tilts it on the client (`onProjectileHit`
  runs client-side); the elytra fuzz removes big dripleaf from its terrain because the physics
  does not model that block change.

**`riptide`** (12 cases + 100 fuzz): a trident with riptide I-III in the hand (`setup.ext.hand`
`trident`, `riptide`), the use key held 6-12 ticks and released (`block_dig`
`release_use_item`): in water, too short, on dry land (the use itself fails), in the rain on open
ground (`setup.ext.raining`: the server's start-raining and rain-level game events) and under a
roof, into a wall, straight up and back into the pool, the spin pose under a ceiling, riptide
then gliding. Event `{"type":"living_flags","set","clear"}`: `set_entity_data` of the player's
living entity flags (index 8) — the client's byte with the item use bits (1, 2) as the server
sees them (a server that cleared bit 1 would stop the client's use), then the masks (4: spin
attack). Fuzz (`ExtCases.riptideFuzz`, cases `…_x_rt`): the `withExtras` case, often a pool at
the spawn and rain, use spans of 3-25 ticks, pitch changes, spin flag events, sometimes an elytra.
Quirk: the launch reads the rotation the tick's mouse turning set (key handling runs after it).

**`entities`** (13 cases + 100 fuzz): other entities through real `add_entity` (event / setup
`{"type":"spawn","id","entity","pos","yaw"}`, spawned after the player so they tick after it),
`teleport_entity` (`{"type":"entity_move","id","pos","yaw","on_ground"}`; the client interpolates)
and `remove_entities`. Cases: a zombie overlapping the player, a crowd of four mobs, walking into
cows and an iron golem, armor stands / bats / parrots (they do not push players), boats and rafts
(collide like blocks, push from their own tick), shulkers to stand and jump on, minecarts, a
zombie walking through the player, a ladder (not pushable), a spectator, a boat moving away under
the player, mobs spawned and removed mid-case. Fuzz (`ExtCases.entitiesFuzz`, cases `…_x_en`):
the `withExtras` case with 1-6 entities of a mixed pool within two blocks, moved, spawned and
removed during the case. Tick records of these cases add `entities`: every ticking entity in the
client's tick order (the player as `{"id":1,"player":true}`), each with `type`, `pre` (box before
the player's tick), `post` (box after the entity tick), `pos`, `alive` and `query_order` (its
place in the level's own `getEntities` order around the player). Quirks:

- On the client only the local player is pushable; the other entities' ticks push it (mobs from
  `pushEntities` over their box, boats over their box grown by 0.2 / shrunk by 0.01 when the
  player's feet are not above the boat's bottom).
- Entity collision shapes come in the level's `getEntities` order (by section, then the order
  entities entered it), and that order is observable: `Shapes.collide` returns 0 once the
  remaining distance is below `1e-7`, so a shape that leaves a sub-`1e-7` gap decides only when it
  is the last one.
- The collision list found for the movement is reused for the step-up query, and auto-jump's
  obstacle search sees entity boxes before blocks.

**`nether`** (4 cases + 100 fuzz): the level built with the `the_nether` dimension type
(`setup.ext.dimension`; `Level.NETHER`): its `fast_lava` environment attribute (lava pushes with
0.007 instead of 0.0023), build height 0-255. Cases: still and flowing lava, a fall into lava,
the bottom and the top of the world. Fuzz (`ExtCases.netherFuzz`, cases `…_x_ne`): the
`withExtras` case in the nether with still and flowing lava pools around the spawn.

**`border`** (4 cases + 100 fuzz): real `set_border_center`, `set_border_size` and
`set_border_lerp_size` packets (events `border_center {x, z}`, `border_size {size}`,
`border_lerp {from, to, ticks}`, built from a server `WorldBorder`); cases with `setup.ext.border`
also run `ClientLevel.tick`'s border step after the entities. Cases: shrinking and growing around
the player, static changes, a player outside a growing border. Fuzz (`ExtCases.borderFuzz`, cases
`…_x_wb`): the `withExtras` case with a small border near the spawn, lerps of 0-120 ticks, size and
centre changes. Quirks:

- A moving border's collision uses its size before the last tick's step (`getMinX(0.0F)` lerps
  from the previous size).
- 26.1 clamps each bound to ±29,999,984 on both sides; a border whose block-rounded inside is
  thinner than `1e-7` (both bounds clamped together) collides as `Shapes.INFINITY`: an entity inside
  it is never stopped, but every "does the box fit" test fails.

**`boats`** (11 cases + 100 fuzz): a boat spawned by `add_entity` and boarded through the real
`set_passengers` packet (event `ride {vehicle, passengers}`; an empty list takes the player off);
server corrections are real `move_vehicle` packets (event `vehicle_move {pos, yaw, pitch}`). Mouse
turning of a passenger goes through `onPassengerTurned` (the harness sets the rotation, then the
vehicle clamps it, as `Entity.turn` does). Tick records add `vehicle`: the boat's position, motion,
rotation, box, flags, status / old status, turn speed, out-of-control counter, water level, land
friction, last vertical motion, paddle input and paddles. Packets add `vehicle_move` and
`steer_boat`. Cases: paddling on still water, a raft turning while the player looks around, a chest
boat on land, packed and blue ice, a drop into water, walls, flowing water, a boat held under water,
slabs and soul sand, another boat in the way, server corrections, sneaking and a dismount. Fuzz
(`ExtCases.boatsFuzz`, cases `…_bo`): pools, land patches (ice, soul sand, slime, honey, magma, sand,
lily pads), walls and water roofs around a boat of a random kind; random paddling keys, looks,
corrections, other boats and dismounts. Quirks:

- The boat ticks before its passenger and paddles with the keys the player held in the previous
  tick (`LocalPlayer.rideTick` hands them over after the passenger's tick).
- The paddle sound check calls `getStatus()` again: where a stroke's sound would play, the water
  level and the land friction are re-read (the land friction is then not halved that tick).
- A non-player entity's water current is normalized, not averaged (`Tracker.applyCurrentTo`).
- `Shapes.joinIsNotEmpty` merges points closer than `1e-7`: a block touching the boat's friction
  probe by less than that does not count.
- Every tick of the passenger runs the whole player tick (travel included) before the boat seats
  it; its own ground / collision flags go out in the rotation packet.
- Fixed with this mechanic: the first record of a case with entities listed the previous case's
  last entity tick (static lists); `entities.jsonl.gz` changed in its `t = 0` records only.

**`minecarts`** (5 cases + 100 fuzz): a minecart (all five kinds) on a rail line, boarded through
`set_passengers`, moved by real `teleport_entity` packets (the client interpolates it; it is not
the client's to move). Tick records add `vehicle` with the cart's position, rotation and box after
its tick and `authoritative: false`. Cases: keys and looks while sitting, a cart moved along the
rails, against a wall and through water, a dismount. Fuzz (`ExtCases.minecartsFuzz`, cases `…_mc`):
obstacles next to and above the rails (walls, water, cobwebs, slabs, soul sand, ladders), the cart
moved along and off the rails with random speeds and rotations, random keys and looks, dismounts.
The passenger's own move and collisions still run every tick (its flags go out in the rotation
packet); the seat is 0.1875 above the cart.

### 26.1.1 and 26.1.2 (same protocol 775): one physics row

`versions/26.1.1.properties` and `versions/26.1.2.properties` run the 26.1 harness on those jars.
Replaying the committed 26.1 goldens (scenarios, `fuzz_seed1`, `fuzz_edges_seed1`,
`fuzz_extras_seed1`) and the cached `physics_s11` + `physics_use_s21` (200k ticks) on 26.1.2, and the
goldens on 26.1.1, gave tick records identical to 26.1's, byte for byte (state and packets):

```sh
MC=26.1.2 oracle/run.sh replay --in "$PWD/tests/golden/26.1/fuzz_extras_seed1.jsonl.gz" --out /tmp/x.jsonl.gz
```

26.2 (protocol 776) and 26.3 (777) need harness changes first (the harness does not compile
against their jars: renderer / level extractor, entity size and authlib API changes).

### 26.1 quirks (observed in the vectors)

- Position packets are sent when the squared move exceeds `(2.0E-4)^2` (1.16.5: 9.0E-4) or every 20
  ticks; a bare on-ground packet when `on_ground` or `horizontal_collision` changed.
- Every tick ends with `tick_end`; key state changes produce `player_input` after the movement packet.
- The position handler sets the position but not `xLast` / `yLast` / `zLast`: the tick after a teleport
  sends a position again.
- The bounding box is derived from the position after every move (`setPos` → `makeBoundingBox`), not
  moved by the offset; differs from 1.16.5 in the last bit.
- Soul sand slows a player with soul speed boots unless the server sends `movement_efficiency`
  (`soul_speed_boots` vs `attr_movement_efficiency`).
- Unloaded columns: `ClientLevel.hasChunk` is always true, so gravity is normal everywhere
  (`travelInAir` never takes its `-0.1` branch on the client); block collision skips unloaded
  chunks (`getChunkForCollisions` is null there); and the fluid sweep (`EntityFluidInteraction`) sees
  **no fluid at all** when any chunk of the box's columns widened by one block is unloaded.
- Flying players are never on a climbable (`Player.onClimbable`) and are never stuck in cobwebs,
  berry bushes or powder snow (`Player.makeStuckInBlock`).
- Attribute modifiers of one operation are summed in the iteration order of a fastutil
  `Object2ObjectOpenHashMap` with its default 32 slots (`HashCommon.mix(Identifier.hashCode())`,
  linear probing, iterated from the last slot down); the order decides the last bit with several
  modifiers.
- A scale change rebuilds the box at the end of the tick that applied the attributes packet
  (`refreshDirtyAttributes` in `LivingEntity.tick`), from the position.

## Crosshair pick (1.16.5 and 26.1)

`SelectionDump` in both harnesses (`selection`, `rays`; both part of `golden`). Replayed by
`crates/ghast-physics/tests/pick.rs`; generated tables in `crates/ghast-data/src/generated/<v>/selection.rs`.

### `selection_shapes.jsonl.gz`

`voxel_shape` lines (own sid space, same fields as `voxel_shapes.jsonl.gz` plus `boxes` =
`toAabbs()` / `getBoundingBoxes()` in vanilla's order), then one `state` line per state:

| field | meaning |
|---|---|
| `outline` | sid of `getShape` / `getOutlineShape(level, pos, CollisionContext.of(player))`, empty hand, at (0, 64, 0) |
| `outline_empty_context` | only when the context-free outline differs (never so far; the pick always has the player's context) |
| `outline_holding_own_item` | only when it differs: the outline while the main hand holds the block's own item (scaffolding; 26.1 also light) |
| `interaction` | sid of `getInteractionShape` (26.1) / `getRaycastShape` (1.16.5) |
| `outline_base`, `offset_at_dump` | for outlines moved by the model offset (flowers, bamboo, 26.1 pointed dripstone, eyeblossoms, mangrove propagules): the unshifted shape (rebuilt from the `OffsetDoubleList` delegates) and the offset at the dump position |
| `position_independent` | the same shapes (or base) at a second position (3, 66, 5) |

Grass, ferns and tall plants have an offset model but an unshifted outline in both versions.

### `raytrace.jsonl.gz`

16 worlds × 200 viewpoints (seed 1). A `ray_world` line: `region` (inclusive box filled at random
density with random states: a uniform pick of all states except shulker boxes and moving pistons,
mixed with a list of partial / special shapes), `blocks` (`[x, y, z, state id]`, air omitted),
`states` (id → name, so readers can check ids), `entities` (4–21 client-side entities of varied
types: `id`, `type`, `bb`, `pick_radius`, `pickable` = the pick's predicate; listed in the level's
iteration order, which the pick follows). A `ray` line: `feet`, `yaw`, `pitch`, `pose`,
`eye_height`, `held` (main-hand item or null), `bb` (player box), `eyes`, `view`, and:

- 26.1: `old` (xo/yo/zo; the eye is `lerp(1, old, pos)`), `block_range` / `entity_range` (attribute
  values, randomised incl. 0 and the clamp at 64), `clip` = `Entity.pick(max(ranges), 1, false)`, and
  `pick` = `LocalPlayer.raycastHitResult(1.0F, player)` (what `Minecraft.pick` stores);
- 1.16.5: `reach` (random, incl. 4.5 / 5.0) with `pick` = `Entity.raycast(reach, 1.0F, false)`, and
  `creative` with `target` = the crosshair after the real `GameRenderer.updateTargetedEntity(1.0F)`
  (reach 4.5 / 5.0, entity reach 3 / 6).

Hit objects: `type` (`block` / `miss` / `entity`), `location`, and `block`, `face`, `inside` for
block results or `entity` (id) for entity results. About 45 % of the viewpoints aim at an entity
(15 % of those stand inside it); 25 % of entities overlap the previous one.

Cached sets: `rays --seed 2..6 --worlds 100 --rays 400 --out cache/<v>/rays/…` (200k rays per
version, ~75 MB each), replayed with `cargo test -p ghast-physics --features oracle-cache --test pick`.

Findings (both versions, bit-identical in the replay): the block traversal, `VoxelShape.clip`,
`AABB.clip` and the entity loop are the same code in 1.16.5 and 26.1; 26.1.2 gives the same
selection and raytrace dumps as 26.1. The 26.1 eye position interpolates even at partial tick 1
(`xo + 1 * (x - xo)`, not always `x`); 1.16.5 does not. A hit inside a shape's first test point is
`from + (to - from) * 0.001` facing against the ray. In 26.1 a hit beyond the block range (or an
entity beyond the entity range) becomes a miss at that point.

## `items.jsonl.gz` (26.1): item and dig facts

`ItemDump` (`items`, part of `golden`), from the live registries with the harness's real tag load.
`item` lines: `id`, `name`, `max_stack_size`, `max_damage`, `food` (`nutrition`, `saturation`,
`can_always_eat`), `block` (a `BlockItem`'s block), `tool` (the default `minecraft:tool`: `rules`
with `tag` or `blocks`, optional `speed` / `correct_for_drops`, `default_mining_speed`,
`damage_per_block`, `can_destroy_blocks_in_creative`), and the rule logic's result through the real
stack on every block's default state (`speed_blocks`: speed ≠ 1 per block id, `correct_blocks`).
`tag` lines: each block tag a tool rule names, resolved (`blocks` = ids). `block` lines: `id`,
`name`, `destroy_time`, `requires_correct_tool`, `differing_states` (states whose destroy time or
correct-tool flag differ from the default state: none in 26.1), `item` (`Block.asItem`).
Generated into `crates/ghast-data/src/generated/v26_1/items.rs`; cross-checked with
minecraft-data (ids, names, stack sizes, durabilities, hardness: all equal) and, in tests, with the
Mojang item-component report when `.cache/mojang/26.1.2` is present.

## `dig.jsonl.gz` (26.1): block breaking through the real client

`DigDump` (`dig [--seed --count]`, part of `golden`: 3000 cases, seed 1). Per case a random block
state (80 % with destroy time ≤ 5), held item (empty / a tool / any other item), and conditions:
`mining_efficiency`, `block_break_speed`, `submerged_mining_speed` attribute bases, haste / conduit
power / mining fatigue amplifiers (`-1` = none; added as `handleUpdateMobEffect` does), eyes in
water (water blocks around the player, fluid tracker updated), on ground, creative. Recorded:
`destroy_speed` (`Player.getDestroySpeed`), `correct_tool` (`hasCorrectToolForDrops`), `progress`
(`BlockState.getDestroyProgress`), and `first`: the dig driven like `Minecraft.handleKeybinds`
(`startDestroyBlock`, then `continueDestroyBlock` in the start tick and once per later tick, up to
2000 calls) with `result` (`instant` = the start packet broke it, `ticks` = `STOP_DESTROY_BLOCK`
went out on call `calls`, `none` = not within the limit) — and, for 15 % of the cases that
finished, a second dig of the same block right after (the client's 5-call destroy delay). Level
events (break particles / sounds) are no-ops in this dump (they need the renderer's models).
Replayed by `crates/ghast-data/src/v26_1/tests.rs` (`dig_matches_the_client`); a 40k-case run
(seed 2) also matched.

## Dig vectors (1.8.9, 1.12.2, 1.16.5)

`DigDump` in each harness (`dig-facts`, `dig [--seed --count]`, both in `golden`), the 26.1 dump's
counterpart through `ClientPlayerInteractionManager.attackBlock` (start; an instant break happens
here, without a stop packet) and `updateBlockBreakingProgress` (in the start tick and once per
later tick; `STOP_DESTROY_BLOCK` when the progress reaches 1, then a 5-call cooldown).

`dig_facts.jsonl.gz`: `block` lines (`id`, `name`, `strength` (1.8 / 1.12) or `hardness` (1.16.5),
`no_tool_needed` (1.8 / 1.12: the material's Legacy Yarn `doesBlockMovement`, MCP
`isToolNotRequired`) or `requires_tool` (1.16.5 `isToolRequired`), `differing_states` (1.12 / 1.16.5:
states where they differ from the default state; none)); `item` lines (`id`, `name`,
`speed_blocks` = blocks where the real stack's mining speed multiplier is not 1, `effective_blocks`
= blocks it harvests (`isEffectiveOn` / `method_11396` / `isSuitableFor`), `differing_states`).

`dig.jsonl.gz` (3000 cases each): block (1.8 / 1.12 `raw` = id << 4 | meta, 1.16.5 `state`),
`held` / `held_id`, `efficiency` level (enchanted stack), `haste`, `mining_fatigue` (1.16.5 also
`conduit_power`) amplifiers, `eye_in_water` (water blocks at the eyes; the game's own test),
`aqua_affinity` (enchanted helmet), `on_ground`, `creative`; `mining_speed`, `effective_tool`,
`progress` and `first` as in the 26.1 dump. Replayed by `crates/ghast-data/tests/oracle_dig.rs`.

## `hashed_stacks.jsonl.gz` (26.1): item stack wire bytes and the client's `HashedStack`

`HashDump` (`hashed-stacks`, in `golden`). Since 1.21.5 the container-click packet carries
`HashedStack`s instead of stacks: item holder, count, and a `HashedPatchMap` of the stack's
component patch: per added component a 32-bit hash (`ClientPacketListener`'s generator: the
component's codec encoded with CRC32C `HashOps` in the synced registry context), and the set of
removed component types. Lines:

- `registry`: `name`, `entries` (every registry the client holds, names in id order: the holder ids
  in the bytes resolve through these; enchantments, banner patterns, trim materials, variants, ...).
- `stack` (4000): `k`, `item`, `item_id`, `count` (1–5000), `wire` (hex of
  `ItemStack.OPTIONAL_STREAM_CODEC` as the server writes it), `added` (sorted by component name:
  `component`, `id` (data component type id), `hash` (signed int, as the packet carries it),
  `json` (the value through its codec with registry ops, for debugging)), `removed` (component type
  names), `hashed_wire` (hex of `HashedStack.STREAM_CODEC`, what the click packet would carry). The
  stack is decoded from `wire` first and hashed as the client would see it.
- `component_type` (every type): `id`, `name`, `transient` (no codec, never in a patch:
  creative_slot_lock, additional_trade_cost, map_post_processing), `values` (pool size) by source
  (`from_defaults` = some item's default value, `from_generic` = a generic JSON value or a registry
  entry name the codec accepts, `from_curated`), `cases_added`, `cases_removed`.

Coverage: the registry has 110 data component types; the 3 transient ones never appear in a
patch; each of the other 107 is added in at least 57 cases. 45 of them also appear as removed:
removals only exist for types some item has by default, because a patch that removes a type the
prototype lacks is normalised away when the stack is built (as on a real server), so the other 62
cannot be removed. Values are validated by a network round trip and a hash before use; patches the
codecs reject are skipped.

Holder-keyed enchantment maps: vanilla keeps `enchantments` / `stored_enchantments` in an
`Object2IntOpenHashMap` keyed by holders (identity hash), so the order a real server writes their
entries in depends on the JVM run — **a client must accept any entry order**. The `HashOps` hash is
order-independent (checked for every stack: the stack as built and as decoded, in different map
orders, hash the same). `wire` is written in vanilla's layout with those maps sorted by
enchantment registry id; for every stack without a multi-entry enchantment map it is
byte-identical to vanilla's own encoding (checked), and the bytes decode to an equal stack. The
vectors hold 1–5-entry enchantment maps and 1–4-entry stored enchantments. `hashed_wire` is
written with the added map / removed set in component type id order (vanilla's order is
arbitrary: IdentityHashMap / HashSet; the server compares contents); `json` has sorted keys. Two
separate JVM runs produce identical files. (Debug-stick maps: the curated several-entry values came
out of parsing + network round trip with one entry; the cause is not investigated, so only
single-entry debug-stick maps are in the vectors.)

## `entity_tracking.jsonl.gz` (26.1): entity packets through the real client

`EntityTrackDump`: per case a fresh login (client player id 1 on a 7×7 stone floor at y 60, chunks
loaded as in an `AIR` scenario), then for every tick random clientbound packets: `add_entity`
(zombies, pigs, villagers, armor stands, slimes, magma cubes, camels, goats, foxes, cows, chickens,
endermen, players, end crystals, floating items; ids 100–117, so ids get replaced), the three
`move_entity_*`, `entity_position_sync` (near and > 64 blocks), `teleport_entity` (random relative
sets and delta movement), `rotate_head`, `set_entity_motion`, `set_entity_data` (random values of the
entity's own entries, size entries more often; complex serializers resend the current value),
`update_attributes` (scale, movement speed, max health with ≤ 1 modifier per operation),
`remove_entities`, `take_item_entity`, `set_passengers`, `set_entity_link`, `player_info_update` /
`player_info_remove` (4 profiles), `forget_level_chunk` and chunk reloads. Each packet body is written
as wire bytes, decoded with the packet's own stream codec (no bytes may be left) and handled by the
real `ClientPacketListener`; then `Harness.Run.step` ticks the level (`ClientLevel.tickEntities`).

- `case` line: `case`, `seed`, `player_id`, `scale_attribute` (registry id of `minecraft:scale`),
  `loaded` (rectangles of loaded chunk columns, `[x0, z0, x1, z1]`).
- `tick` line: `packets` = `[name, body hex]` in handling order (`name` = the packet type's path;
  `level_chunk_with_light` carries only the chunk x and z as two big-endian ints, the column itself is
  an empty one), then `entities` = every entity but the client player, by id, as the client holds it
  **after the packets and before the tick** (when `Minecraft.tick` picks): `pos` (3 × f64 hex), `rot`
  (yRot, xRot f32 hex), `head` and `body` (living only: yHeadRot, yBodyRot), `ground`, `box` (6 × f64 hex), `pickable`, `spectator`,
  `ticking` (`isTickingEntity`), `rode` (was named as a passenger since its spawn: its position then
  comes from the vehicle and is not a tracker fact), `vehicle` (-1 = none), `passengers`, `count`
  (item entities), and `tab` = `[profile k, name, game mode id (-1 none), latency, has display name]`.
- Items get no-gravity metadata at spawn and never a velocity, so the client's own item physics leaves
  them where the packets put them. No collidable entities (boats, minecarts, shulkers) are spawned.

## `biome_climate.jsonl.gz` (26.1): biome precipitation, temperature and zoom

`BiomeClimateDump` (`biome-climate`), from the client's synced `minecraft:worldgen/biome` registry:

- `biome` line (one per registry entry): `index`, `name`, `has_precipitation`, `temperature` (base,
  f32 bits hex), `modifier` (`none` / `frozen`), `samples` = 40 × `[x, y, z, sea_level,
  temperature hex, precipitation]` with `Biome.getHeightAdjustedTemperature(pos, sea_level)` and
  `Biome.getPrecipitationAt(pos, sea_level)` (`none` / `rain` / `snow`); x, z far (±3·10⁷), mid
  (±5000) and near (±300), sea level 63 or random.
- `zoom` line (16 seeds, the first 0): `seed` (decimal string), `samples` = 200 × `[x, y, z, qx, qy,
  qz]`: the noise cell (quart position) a real `BiomeManager(source, seed).getBiome(pos)` asks its
  source for.

Not covered: the client's own biome lookup on a loaded column (quart y clamped to the dimension,
section index, cell index `y << 4 | z << 2 | x` within the section), `Level.isRainingAt`'s sky and
height-map checks.

## Crosshair pick (1.8.9, 1.12.2): `picks.jsonl.gz`

`PickDump` in both old harnesses (`picks [--seed --worlds --rays]`, in `golden`: 16 worlds × 200
picks). A `pick_world` line: `region`, `blocks` (`[x, y, z, raw]`, raw = id << 4 | meta, every
valid state, a share of them from a list of partial / neighbour-dependent blocks), `entities` (the
world's `getEntitiesIn` order: `id`, `class`, `bb`, `margin` (targeting margin), `pickable` =
`collides()` and not a spectator; entities of earlier worlds stay listed: `ClientWorld.removeEntity`
leaves them in the chunk lists, removed — end crystals still collide). A `pick` line: `feet`,
`yaw`, `pitch`, `head_yaw`, `sneaking`, `eye_height`, `creative`, `reach` (random, incl. 0 and 64),
`bb`, `eyes`, `look`, `raw` = `Entity.rayTrace(reach, 1.0F)`, `target` = `client.result` after the
real `GameRenderer.updateTargetedEntity(1.0F)` (game-mode reach 4.5 / 5.0, entities to 3 / 6;
`miss` with `side: null` = an entity hit beyond 3 blocks outside creative), `targeted_entity`.
About 45 % of the picks aim at an entity (15 % of those from inside it).

Replayed by `crates/ghast-physics/tests/pick_legacy.rs` (also the older `raytrace.jsonl.gz` of both
versions and every 1.12.2 `selection_boxes.jsonl.gz` entry); cached seeds 2–4 (100 worlds × 400
picks each, `cache/<v>/picks`) with `--features oracle-cache`. Findings: the block walk and box test
are the same code in 1.8.9 and 1.12.2; 1.8 looks with the head yaw, 1.12 with the body yaw;
`Vec3d.distanceTo` rounds to `float` (entity distances); 1.8 torches set their own ray bounds from
the facing (the table bounds are stale); the moving piston is never hit; 1.12 flowers move their
box by the model offset, vines take one face box (top when the block above has a solid bottom),
fence gates beside a wall are 0.8125 high, redstone wire bounds follow its actual connections.

## Notes for implementers

What a Rust implementation (`ghast-physics`) has to get right to match these vectors. Questions or
cases to run: use `replay` (above), or ask the oracle maintainer.

### Initialising and stepping

- **Initialise from the tick 0 record, not from the `scenario` header.** Tick 0 holds the state after
  setup, including derived values: `bb`, `movement_speed_attr`, `flying_speed`, timers, `sent`. The header
  only records how the oracle built that state.
- Then for every tick: apply `input.events` in order (inbound packets) → set the held keys → set
  `yaw`/`pitch` → run one client tick → compare with `state` and `packets`.
- The world is `fills` applied in order inside `loaded_chunks`; everything else in those chunks is air,
  everything outside them is an unloaded column (see quirks).

### State carried from one tick to the next (all in `state`)

| field | why it matters |
|---|---|
| `bb` | the real position state; after a move, pos = ((minX+maxX)/2, minY, (minZ+maxZ)/2) |
| `vel`, `on_ground` | `on_ground` from the previous tick gates jumping and the sprint double-tap |
| `horizontal_collision` | the previous tick's value stops sprinting before this tick's move |
| `in_web` | set by cobweb contact after a move, consumed (×0.25 / ×0.05 / ×0.25) at the start of the next move |
| `fall_distance`, `jump_cooldown` | |
| `flying_speed` | air acceleration for the **next** tick (set after travel: 0.02, ×1.3 sprinting) |
| `movement_speed_attr` | ground speed, read live (sprint modifier ×1.3 applies immediately) |
| `sprint_toggle_timer`, `fly_toggle_timer`, `sprinting_ticks_left` | double-tap windows (7 ticks) and the 600-tick sprint countdown |
| `input` | the previous tick's input; sprint and flight double-taps compare against it |
| `sent` | drives the movement packet choice |
| `outside_world_border` | changes how the collision query treats border positions |

### Packet rules (`packets`)

- Order in a tick: sprint change `entity_action`, sneak change `entity_action`, then **exactly one**
  movement packet.
- Position is included when the squared distance to `sent.pos` (x, bb.minY, z) is > 9.0E-4 or
  `ticks_since_pos` ≥ 20. Look is included when yaw or pitch differ from `sent`. Neither → `flying`.
- `abilities` is sent when the double-tap toggles flight and when landing ends creative flight.
- A `teleport` event also produces the client's `position_look` reply (`on_ground` false).

### Quirks (all observed in the vectors)

- **Unloaded chunks.** The client treats every chunk column as existing. An unloaded column is an
  empty chunk: no blocks, no collision boxes at all. Gravity is replaced when the column at
  `((int) x, (int) z)` (truncation, not floor) is not loaded: `velocityY = -0.1` while y > 0 (then ×0.98),
  instead of `-0.08`.
- **Late air speed.** Ground acceleration reads the speed attribute directly, so a sprint change takes
  effect immediately. Air acceleration uses `flying_speed`, which is set after travel, so a sprint change
  reaches it one tick late.
- **Double-taps** (sprint, creative flight) compare with the previous tick's `input`.
- **Collision.** `Entity.move` clips Y, then X, then Z (`Box.method_589` / `method_583` / `method_594`).
  Step-up (0.6) retries the move raised by the step height when x or z was clipped and the entity was on
  ground before the move or is landing in this move (dy was clipped and requested dy < 0).
  `on_ground = vertical_collision && requested dy < 0`.
- **Sneak edge.** While on ground and sneaking, x/z are reduced in 0.05 steps until the box moved
  down by 1 would still collide.
- **Climbing.** `climbing` checks the block at the feet (ladder or vine). While climbing: x/z clamped to
  ±0.15, fall distance reset, descent clamped to -0.15, no descent while sneaking. Horizontal collision
  while climbing sets `velocityY = 0.2` after the move.
- **Friction.** The block at (floor(x), floor(bb.minY) - 1, floor(z)): slipperiness × 0.91; ground
  acceleration factor 0.16277136 / f³.
- **pushOutOfBlocks** (client player) is tested at the four corners ±0.35 × width at minY + 0.5 and uses
  `World.method_8565` ("has a collision box whose average edge length is ≥ 1").
- **Coordinate clamp.** `PlayerEntity.tick` clamps x and z to ±29,999,999 (`world_border` scenario: the
  player stops at 29,999,999.0 with no collision flag and keeps its velocity).
- **World border.** Collision positions outside the world border are treated as stone while the entity
  is inside it (default border here: centre 0,0, size 6e7). Border packets are not modelled yet.
- **Block shapes** are floats widened to double (0.1F = 0.10000000149011612), added to the integer block
  position in double. An unused meta decodes to air (network path), not to a default state.
- **Anvils and chests:** use the `settled` shapes (see "History-dependent shapes").

### Yarn names that mislead

`LivingEntity.flyingSpeed` is the air acceleration; `initializeAttribute` is a plain getter;
`World.doesBoxCollide` returns the *list* of collision boxes; `ticksSinceSprintingChanged` counts down;
`abilityResyncCountdown` is the flight double-tap window; `field_1763` is the sprint double-tap window;
`Block.isFullCube` = opaque && normal render && not a power source, `isNormalBlock` = blocks movement &&
normal render. Full list: YARN_COVERAGE.md, "Names to read carefully".

## Window clicks (`clicks`, every version; 1.12.2 specifics in its section)

`ClickDump` reuses the movement harness' player (`Harness.Run`) and calls the real client click path:
`ClientPlayerInteractionManager.clickSlot` → `ScreenHandler.getNextActionId` + `onSlotClick` → the
captured `window_click` packet. Screen handlers are built the way the client's screens build them
(1.8.9: `ChestScreenHandler` on a `SimpleInventory`, `Generic3x3ScreenHandler`, `HopperScreenHandler`,
`CraftingScreenHandler` on the client world, `FurnaceScreenHandler` on a `ClientNetworkSyncedInventory`;
1.16.5: the `ScreenHandlerType` constructors for generic 9×1…9×6, 3×3, hopper, shulker box, crafting,
furnace) or the player's own `PlayerScreenHandler`. Per case (seed × index → SplitMix64): window type,
sync id, selected slot, random contents (items chosen for stack sizes, subtypes, armor, damage,
remainders, NBT; over-stacked and, on 1.8, zero-count stacks), cursor, armor/offhand, and 1–10
clicks of modes 0/1/2/4/6 (buttons as the GUI sends them, slot -999 for modes 0/4). Each line holds
the state before (window 0 slots, screen slots, cursor, player inventory, every stack as its wire
bytes) and per click the packet body (`write(PacketByteBuf)`) and the state after.

`--craft true` (golden `clicks_craft_seed1.jsonl.gz`): only window 0 and crafting tables, grids
filled from a random vanilla recipe (1.8.9 also the special recipes' ingredients), 40 % of the
clicks on the result slot and 30 % on the grid. The 1.8.9 client computes crafting results from its
own recipes (the vectors contain them). On 1.16.5 the vanilla data pack is loaded the way an
integrated server loads it (`ServerResourceManager.reload` with the `DefaultResourcePack`), its
recipes are sent to the client as a real `SynchronizeRecipesS2CPacket` (sorted by type and id:
`RecipeManager.values()` order depends on identity hash codes) whose bytes are the file's first line
`{"declare_recipes": hex}`, and after every click that changed the grid the server's result
(`getFirstMatch(CRAFTING)` → `craft`) is put into the result slot and recorded as `"server"`.
Plain 1.16.5 cases sync the recipes too (a real client always has them). 1.16.5 needs `remap.fixPackageAccess=true` (cross-package
protected calls such as `Slot.onTake` after remapping the single official package).

`recipes.jsonl.gz` lines: `{"i", "type":"shaped", "w", "h", "in":[[id,data]|null…], "out":hex,
"copy_nbt"}`, `{"i", "type":"shapeless", "in":[[id,data]…], "out":hex}`, `{"i", "type":"special",
"class", "size"}` (data 32767 = any).

Line schema: `{"case", "kind", "rows"` (1.8.9) / `"menu"` (1.16.5 menu type id, -1 = window 0)`, "sync",
"selected", "w0": [slot hex…], "screen": [slot hex…] | null, "cursor": hex, "inv": [40 (1.8.9) / 41
(1.16.5) slot hex], "clicks": [{"slot", "button", "mode", "packet": hex, "w0", "screen", "cursor",
"inv"}]}` — slot hex = `writeItemStack` bytes, packet hex = the body after the packet id. Real: the
click path, screen handlers, inventories, item stacks and NBT. Faked: nothing beyond the movement
harness (the client screens are not opened; the handler is put into `openScreenHandler` /
`currentScreenHandler` directly). `selftest` also reruns 500 click cases in reverse order on the
same player (no state leaks between cases).

## Window clicks (26.1): `clicks_seed1.jsonl.gz`

`ClickDump` (`MC=26.1 oracle/run.sh clicks [--seed N] [--count N] [--out FILE] [--best true]`, in
`golden` with 3000 cases; big runs go to `cache/26.1/clicks/`). The real click path of the 26.1
client: `MultiPlayerGameMode.handleContainerInput` → the menu's `clicked` (`doClick` with every
`ContainerInput`: `PICKUP`, `QUICK_MOVE`, `SWAP`, `CLONE`, `THROW`, `QUICK_CRAFT`, `PICKUP_ALL`)
→ the slots whose stack changed and the carried stack hashed into the
`ServerboundContainerClickPacket` it sends. Every packet the click makes the client send is
recorded (a drop also sends `swing`).

Real, per case: the menu, built as `MenuScreens.create` builds it (`MenuType.create(id,
inventory)` for all 25 `minecraft:menu` types, or window 0's `InventoryMenu`), or in 6 % a mount
screen as `handleMountScreenOpen` builds it (a client entity of a random mount type added to the
level — horse, donkey, mule, llama, trader llama, camel, skeleton / zombie horse, nautilus,
zombie nautilus; baby or not, tamed or not, a chest and llama strength at random — and its
`HorseInventoryMenu` / `NautilusInventoryMenu` over a `SimpleContainer` of its columns); contents, carried
stack and state id through a real `container_set_content` (window 0 and the menu); game mode
changes as a server sends them (game event, player info, abilities: creative cases enable
`CLONE` and clone drags, and lift the binding curse lock); the experience level
(`set_experience`: the anvil's cost check), the held slot, container data (crafter slot locks,
anvil cost, stonecutter / loom selection), a villager's offers through a real
`ClientboundMerchantOffersPacket`. Screen actions besides clicks, as the screens perform them: a
trade picked (`MerchantScreen.postButtonClick`: hint, `tryMoveItems`, `select_trade`), a loom
pattern button (`clickMenuButton` + `container_button_click`), an anvil rename (`AnvilScreen`'s
name field: `setItemName` + `rename_item`). Once per file the vanilla data pack's recipes are
loaded the way a server loads them and synced through a real `ClientboundUpdateRecipesPacket`
(its bytes are the first line: the furnace, smithing and stonecutter item sets come from it);
the item sets and stonecutter entries are written sorted (item id, recipe id) because the
server's own order depends on identity hash codes. Faked: `Minecraft.screen` is an allocated
`InventoryScreen` / `ContainerScreen` (never initialised): the click code only asks whether a
container screen is open. Window 0 in creative mode is the creative inventory screen in a real
client (`set_creative_mode_slot` instead of `container_click`); those cases always open a menu.

Stacks: random items chosen for their rules (stack sizes, armor and offhand, remainders,
bundles with contents, shulker boxes, fuels, brewing ingredients, beacon payments, banners,
dyes and patterns, maps, smithing templates), damage, over-stacked counts, one random component
change in 15 % (custom name, `max_stack_size`, enchantments incl. the binding curse, repair cost,
unbreakable, `equippable` incl. `allowed_entities`, custom data, damage, a removed default).
Menus get contents that fit their slots most of the time. Gestures: a single click of each mode
(buttons as the GUI sends them, slot -999 for outside), or a drag: start (-999), 1–6 slots, end
(-999), of type split / one each / clone, sometimes broken (no start or end, a type change, an
unrelated click in the middle).

Lines: `{"type":"update_recipes","packet":hex}`; `{"type":"registry_data","registry","packet":hex}` (the
enchantment and banner pattern registries as a server sends them, with data);
`{"type":"update_tags","packet":hex}` (the server's tags, sorted); `{"type":"registry","name","entries"}` (every
registry the client holds, ids in order); `{"type":"item","id","name","fuel",
"brewing_ingredient","brewing_fuel","beacon_payment","loom_dye_tag","loom_pattern_tag","banner",
"fits_container","has_dye","has_patterns","remainder"}` (the facts the slot rules read, after
the configuration phase; `remainder` = `defaultCraftingReminder` of the item alone, stack hex);
`{"type":"case","case","menu"` (registry id, -1 = window 0)`,"window"` (container id)`,
"creative","xp","selected","data"`(crafter: 10 values, anvil / loom / stonecutter: 1)`,"offers"`
(merchant: the packet hex)`,"mount"`(`{"type","type_id","columns","saddle","body"}`: entity type,
chest columns, `canUseSlot(SADDLE / BODY)`)`` + state `,"clicks":[{"slot","button","mode"`
(modes 100 select trade, 101 loom button, 102 rename with `"name"` sent)`,"packets":[hex…]` +
state `}]}`; a state also carries `"slots_data"` (the menu's data slots after the click). State: `"w0"` (46 window 0
slots), `"screen"` (the menu's slots or null), `"carried"`, `"inv"` (41 player inventory slots),
`"state_id"`, `"drag"` (`quickcraftStatus`). Stack hex: `ItemStack.OPTIONAL_STREAM_CODEC`'s
layout with the patch in component type id order and enchantment maps by registry id (the
in-memory order depends on how the stack was built). Packet hex: a tag byte (0
`container_click`, 1 `set_creative_mode_slot`, 2 any other packet followed by its type name, 3
`select_trade`, 4 `container_button_click`, 5 `rename_item`),
then the body: `container_click` exactly as `STREAM_CODEC` writes it (the changed-slot map in
its own `Int2ObjectOpenHashMap` order) except that each `HashedStack`'s added components and
removed types are in type id order (vanilla's `IdentityHashMap` / `HashSet` order is
arbitrary; the server compares contents) — every hand-written packet is checked to decode to
the client's own. `selftest` reruns 500 cases in reverse order on the same player; two JVM runs
write identical files.

Replayed by `crates/ghast-inventory/tests/oracle_clicks_26_1.rs`.

### The creative inventory screen: `clicks_creative_seed1.jsonl.gz`

`creative-clicks` (in `golden` with 1000 cases): window 0 in creative mode is the
`CreativeModeInventoryScreen`, which sends `set_creative_mode_slot` instead of
`container_click`. Real: the screen's `slotClicked` (the inventory tab: its `SlotWrapper`s
around window 0's slots 5–45, its own Q / drop handling, otherwise `inventoryMenu.clicked` +
`broadcastChanges`), its `CreativeInventoryListener` on the player's `InventoryMenu` (added when
the screen opens: one broadcast), `ClientPacketListener.handleContainerSetSlot` for server slots
while it is open (the client echoes them). Faked: the screen is allocated, with the fields its
click code reads set (minecraft, its `ItemPickerMenu`, the inventory tab, the destroy slot, an
empty mouse-action list). Per case the menu's `lastSlots` start empty or half known (earlier
broadcasts) and the player's drop spam throttle starts at 0 (cases never tick the player).
Lines: `{"type":"creative","case","selected","last":[46 stack hex]` + state `,"open":[packet
hex…],"events":[{"slot","button","mode","packets"` + state `} | {"server_slot","item",
"packets"` + state `}]}`; `set_creative_mode_slot` packets are tag 1 + short slot + the stack in
`OPTIONAL_UNTRUSTED_STREAM_CODEC`'s layout (each component value length-prefixed, components in
type id order), checked to decode to the client's packet.
