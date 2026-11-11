# ghast-pathfinder

Path planning and path following for the ghast Minecraft Java Edition bot library, 1.8.9,
1.12.2, 1.16.5 and 26.1. The planner is an A* over feet blocks; the follower drives the vanilla-exact
`ghast-physics` player. Every move type is checked by running it in that physics, both in
the tests and (by default) before the bot moves.

```rust
use ghast_pathfinder::{Config, ControllerConfig, Goal, Pathfinder, SimOutcome, Version, simulate};

let mut world = ghast_physics::v1_8::memory_world();
world.load_chunks(-2, -2, 2, 2);
world.fill_raw([-20, 63, -20], [20, 63, 20], 1 << 4); // stone floor
world.fill_raw([3, 64, -5], [3, 64, 5], 1 << 4); // a step

let pf = Pathfinder::new(Config::default());
let path = pf.plan(Version::V1_8, &world, [0, 64, 0], &Goal::Block([8, 64, 0])).unwrap();
let run = simulate(&path, &world, &ControllerConfig::default());
assert_eq!(run.outcome, SimOutcome::Arrived);
```

From a bot, use `Follower` (feature `client`, on by default) in `Behavior::tick`, or
`ghast-ext`'s `Bot::goto`.

Versions: one cargo feature each, `v1_8`, `v1_12_2`, `v1_16_5`, `v26_1` (default: all; they
forward to ghast-data, ghast-physics and, with `client`, ghast-client).

26.1 (`Version::V26_1`) runs on the modern physics core (`ghast_physics::modern`, rule set
`V26_1`) and the 26.1 block tables:

- **Live player.** 26.1 servers send movement as attributes (movement speed, step height, jump
  strength, gravity, scale, ...). The controller's look-ahead, the pre-jump check and the
  verification simulate with the live client player when they have it
  (`Controller::set_modern_player`, `Simulation::with_modern_player`; the `Follower` passes
  `TickContext::modern_player` every tick), vanilla defaults otherwise. The move set follows
  the attributes at plan start (`Search::with_modern_player`): step-ups up to the step height
  (at most vanilla's 0.6), jump-ups only as high as the jump reaches (jump strength, jump
  boost, gravity), gap jumps only with vanilla jump, gravity, movement speed and scale.
  Approximate: move costs stay vanilla's (a faster or slower player gets the same route); a
  higher step height is not used beyond 0.6; a scaled body (scale ≠ 1) still plans with the
  1.8-block clearance (the verification catches what does not fit); attributes that change
  during the walk are seen by the controller, not by the plan.
- **Build height.** From the dimension (`BlockSource::build_height`; the `Follower` takes it
  from the live player): negative Y works, below the bottom is unknown (never entered), above
  the top is air.
- **Blocks.** Classified from the jar's movement records: powder snow, scaffolding, cobwebs,
  berry bushes, fire and bubble columns are never entered; magma, campfires, pointed dripstone,
  lava cauldrons, big dripleaves and sculk sensors / shriekers are never stood on; mud (14/16),
  copper grates, moss and the like are ordinary floors; bamboo, pointed dripstone, shulker boxes
  and moving pistons are irregular obstacles.

## Moves

| move | what | cost (ticks) |
|---|---|---|
| `Walk` | to a cardinal neighbour, step-ups up to 0.6 (slabs, soul sand, carpets) and drops up to 0.6 | measured walk / sprint speed |
| `Diagonal` | both side columns clear **and floored** (a stray box never drops into a hole) | √2 × straight |
| `Ascend` | jump onto a block 0.6–1.0 higher, with headroom for the arc | measured standing jump |
| `Descend` | walk off an edge and fall, at most `max_fall` (3: no fall damage) or `max_water_fall` into deep water | measured fall table |
| `Parkour { gap }` | jump along an axis over 1–3 blocks on the level, onto a block one higher, or one or two lower (4 when two lower); from and onto 1×1 pillars | measured jump |
| `Swim` | in water, up, down, out onto land | measured swim speeds |
| `ClimbUp` / `ClimbDown` / `ClimbExit` | ladders and vines, also onto a ladder from above | measured climb speeds |
| dig / place (opt-in) | tunnel through blocks (`Config::with_dig`), fill one-deep holes (`Config::with_place`) | `DigCost`, place cost |

Costs are not guesses: `Costs::of(version)` measures them once per version by running the
physics on canonical terrain. Unknown (unloaded) blocks are never treated as air: they are
impassable, and a goal beyond them gives a partial path that ends at the loaded edge.
Lava, fire, cobwebs, berry bushes, portals, magma / campfires / cactus as floors are avoided;
doors, fence gates, scaffolding and other shapes the planner does not model are treated as
solid.

## Gap jumps

A jump is offered only where the vanilla-exact physics lands it, checked twice:

- **Planning** reads a table (`src/jump_table.rs`) measured in `ghast-physics` on canonical
  terrain: only the take-off and landing blocks exist, so the landing is a 1×1 pillar. Each
  run follows the controller's exact key policy (stand still on the take-off block, hold
  forward and maybe sprint along the axis with the yaw fixed, jump at the edge or at once,
  release the keys in the air at the tick that lands closest to the centre) from 153 start
  states (±0.2 along, ±0.15 across the axis, ±0.5° yaw). A mode is kept only if every start
  lands on the block and comes to rest on it without touching anything else. The planner
  then requires every cell the body swept in any of those runs to be free (no collision,
  fluid, ladder or hazard), up to the landing nothing within 0.3 under the body (it would
  catch the player early), and no soul sand or honey half a block under the feet (1.16
  applies their velocity multiplier from there, even in the air), so along the jump the
  world is, to the physics, the canonical terrain. Take-off and landing need full-height
  floors with ordinary friction (no ice, slime, soul sand, slabs).
- **Before jumping**, standing still on the take-off block, the controller runs the jump in
  the physics from its actual state in the actual world and jumps only if that lands;
  otherwise it reports `Stuck` and the follower plans around the jump. It then replays
  exactly the keys of that run (ground ticks, jump tick, air ticks with the keys held).

Jumps in the table (all three versions; some need sprinting, some jump at once from the
centre):

| gap | one up | level | one down | two down |
|---|---|---|---|---|
| 1 | ✓ | ✓ | ✓ | ✓ |
| 2 | ✓ | ✓ | ✓ | ✓ |
| 3 | ✓ | ✓ | ✓ | ✓ |
| 4 | — | — | — | ✓ |

The table is regenerated (after a physics or controller change) with
`GHAST_UPDATE_JUMPS=1 cargo test --release -p ghast-pathfinder --lib -- --ignored jump_table`;
a normal test run re-checks every entry from the centre and two corners of the envelope.

## Mazes and unreachable goals

A* with an admissible heuristic must expand every block whose estimate is below the real
cost, and in a maze the real way is many times the straight line: the search floods the
open terrain around the maze before it goes deep. The node budget (`Config::max_nodes`,
default 250 000) covers all terrain a client loads at view distance 10 on a flat world; the
follower spreads the search over ticks (2000 nodes / 5 ms per tick by default). A search out
of nodes returns a partial path; the follower walks it and plans again, and gives up after
`max_replans` partial paths that do not get closer to the goal.

When the goal cannot be reached through the loaded terrain (a flying player, a sealed room),
the search expands everything reachable (`Search::exhausted`): the follower walks to the
reachable block closest to the goal and then fails with `NoPath`, after that single search.

## API

- `Pathfinder::new(Config)`; `plan(version, &world, from, &goal)`,
  `plan_avoiding(.., &banned_moves)`, `plan_verified(.., attempts)` (plan, simulate, plan
  around the move the simulation failed), `Pathfinder::start_block(feet_pos)`.
- Incremental: `Pathfinder::search(..) -> Search` with `Search::step(&world, Budget)` (at most
  `Budget::nodes(n)` expansions, optionally `.with_time(d)`), `Search::exhausted`, and `Simulation::new(&path,
  &cfg)` with `Simulation::step(&world, ticks, time)`. Slices only pause the same algorithm:
  the results equal `plan` / `simulate` on the same world (tested).
- `Config` (`with_sprint`, `with_parkour`, `with_max_fall`, `with_swim`, `with_climb`,
  `with_dig(Some(DigCost))`, `with_place(Some(cost))`, `with_max_nodes`,
  `with_heuristic_weight`); `Goal::{Block, Near, Xz, Y, AnyOf}`.
- `Path { steps: Vec<Step>, cost, partial, expanded, version }`; `Step { pos, feet_y, kind,
  node, dig, place }`.
- `BlockSource`: raw block states by position (`None` = unknown). Implemented for
  ghast-client's `WorldView`, the ghast-world worlds and ghast-physics' `MemoryWorld`s.
- `Controller` (pure: observation in, aim + keys out; physics look-ahead for edge jumps,
  landings and never stepping off where the path does not go), `ControllerConfig`,
  `Command`, `Progress`.
- `simulate(&path, &world, &ControllerConfig) -> SimReport { outcome, ticks, reached, end,
  collisions, first_collision, max_fall, trace }`. The simulator turns in whole mouse counts
  exactly like ghast-client (a test compares it with the client's rotation).
- `Follower` (feature `client`): `goto(goal)`, `tick(&mut TickContext) -> FollowStatus`,
  `stop`, `path`, `status`; plans (verified by default), follows, replans after teleports,
  stuck / off-path moves (that move is avoided from then on) and at the end of partial
  paths. Planning and verification are spread over ticks within `FollowConfig::budget`
  (default 2000 nodes and 5 ms per tick) and `verify_ticks` (400 simulated ticks); while a
  plan is in progress (`is_planning`) the player stands still with the keys released and no
  turn, and the client sends its normal movement packet every tick. It acts only through the `TickContext`: aim in whole mouse counts, keys, the left
  button for digs, `place` for places, so the tick model of ghast-client holds (actions
  before the tick's single movement packet).

## Validation

`tests/validation.rs`, for all three versions, plans on fake worlds and runs each path through
the physics with the controller, asserting the goal block is reached with no unexpected
horizontal collision and no fall damage: flat runs (straight, diagonal, no sprint), a wall
with one opening, glass corridors, staircases up, drops of up to 3, slabs / soul sand
steps, a fence row (gone around), 1- and 2-block gaps over lava (and none without parkour),
a pool (swum across; impossible without swimming), a ladder up and down a cliff, a tunnel
dug through a dirt wall, a trench filled with blocks, a goal beyond unloaded chunks
(partial path), all goal kinds, verified plans, and 96 fixed random rough terrains
(heights 0–3 per block, slabs, soul sand, holes). An ignored proptest fuzzes more terrains:
`cargo test --release -p ghast-pathfinder -- --ignored`.

`tests/parkour.rs`: trenches 1, 2 and 3 wide (4 deep, no way around), one up and one down,
up onto a one-wide ledge and down again, stepping stones (1×1 pillars over lava) with gaps of
1, 2, 3 and one up, all four directions, a jump between the glass walls of a closed corridor,
a superflat ditch (jumped, not climbed through), and the jumps that must not happen: 4 wide,
4 wide and one up, under a low ceiling, without the option.

`tests/maze.rs` and the follower's unit tests: generated perfect mazes 15, 31 and 63 blocks
wide (corner to corner, and from outside to the centre, whose way is 3–6 times the straight
line), one-shot and in 2000-node slices (the same path), run in the physics; a 127-block maze
in terrain loaded at view distance 10 driven by the `Follower` itself through a simulated
client; a sealed cell and a goal in the air (one search, then `NoPath` at the closest
block); a node budget far too small (ends instead of wandering).

## Performance

`cargo bench -p ghast-pathfinder` (x86_64 WSL, release, 1.8.9, 2026-11-06):

| case | steps | nodes | plan | simulate |
|---|---|---|---|---|
| flat, 100 blocks straight | 101 | 100 | 0.28 ms | 2.2 ms (359 ticks) |
| flat, 50 × 50 diagonal (~71 blocks) | 51 | 50 | 0.12 ms | 2.0 ms (254 ticks) |
| rolling terrain with a long wall, 80 × 60 (detour) | 193 | 27,091 | 57 ms | 24 ms (1,708 ticks) |

About 400–480 k nodes/s. The controller costs ~6–14 µs per tick (physics look-ahead
included).

Per game tick with the follower's default budget (2000 nodes, 5 ms; verification 400
simulated ticks, 5 ms):

| case | search | verification |
|---|---|---|
| flat, 100 blocks straight | 1 tick, 0.34 ms | 1 tick, 2.1 ms |
| flat, ~71-block diagonal | 1 tick, 0.22 ms | 1 tick, 2.3 ms |
| rolling terrain detour (27,091 nodes) | 14 ticks, worst 4.6 ms (a 2000-node slice: 3.7 ms) | 5 ticks, worst 5.3 ms |

So the longest planning work in one tick stays near 5 ms (the clock is read every 64 nodes /
16 simulated ticks); the detour costs ~19 ticks (~1 s) of standing still before the bot moves.

Mazes, driven by the `Follower` with the default budget (x86_64 WSL, release, 1.8.9,
2026-11-07; the bot stands still while it plans):

| maze, loaded terrain | nodes | planned in | worst tick |
|---|---|---|---|
| 31 blocks, view distance 6 | ~12 k | 3 ticks | 5.1 ms |
| 63 blocks, view distance 6 | ~40 k | 19 ticks | 5.2 ms |
| 127 blocks, view distance 10 | ~110 k | 67 ticks | 6.1 ms |

Before this release the last one ran out of its 100 000 nodes and followed partial paths into
dead ends next to the goal.

## Live check

1.8 lab (Paper 1.8.8 + Grim 2.3.73, local, 2026-11-06): the `goto` example of the `ghast`
crate (`--features ext,pathfinder`) walked the test pad's step-up course, wall, fence,
water pool, soul sand, slime, ice and random points: 36 legs in 5 min (with
`--grim-compat`) and 27 legs in 3 min (without), **0 Grim flags, 0 setbacks**. A later run
added a pocket of walls built with `/fill` (a ~60-block way out, 931 nodes) planned with 250
nodes per tick: planned in 4 ticks standing still, then walked out; 23 legs in 3 min, 0 Grim
flags.

Gap jumps and mazes (1.8 lab, Paper 1.8.8 + Grim 2.3.73, local, 2026-11-07,
`lab/pathfinder-course.sh` + `ghast-ext`'s `lab_course` example, vanilla step behaviour):
a closed corridor with trenches 1, 2 and 3 wide (3 deep), a gap one up and one down, and
stepping stones on 1×1 pillars (gaps 2, 1, 3, 2 up, 1 down, 2), run east and west; a
31-block maze (centre, out, centre again); then `Bot::follow_path` after an armor stand moved
through the maze every 15 s, once hanging out of reach in the air (the bot waited under it
after one search). Five sessions, 27 legs, **0 failed, 0 Grim flags, 0 setbacks**; the
corridor takes ~15 s each way.

The same course on the 1.12 lab (Paper 1.12.2 + Grim 2.3.73, local, 2026-11-07, `GhastBot12`,
`... lab_course ... 1.12.2`): two sessions, 16 legs (the corridor 5 times each way, the maze
three times) and `follow_path` after the armor stand, **0 failed, 0 Grim flags, 0 setbacks**.

26.1 lab (Paper 26.1.2 + Grim 2.3.74, local, 2026-11-12, `GhastBot12`,
`lab/pathfinder-course.sh 26.1 build`, `... lab_course -- 127.0.0.1 25626 GhastBot12 640 2 26.1`):
the corridor twice each way (16–17 s per leg), the maze centre, out and back in, then 150 s of
`follow_path` after the armor stand: **7 of 7 legs ok, 0 Grim flags, 0 setbacks**.

The same session on 2026-11-12 on the 1.8 (Paper 1.8.8 + Grim 2.3.73), 1.12 (Paper 1.12.2 +
Grim 2.3.73) and 1.16 (Paper 1.16.5 + Grim 2.3.73) labs: 7 of 7 legs each, 0 Grim flags, 0
setbacks; corridor legs 15.1–15.6 s (1.8), 16.1–16.4 s (1.12), 16.2–16.4 s (1.16), maze legs
27.5–34.2 s on all four versions.

## Known limits

- About 1 in 1000 fuzzed rough terrains still ends `Stuck` / `OffPath` in the simulation
  (walking down onto a slab next to holes; none on a gap jump in 3000 terrains). `plan_verified` and the follower
  plan around such moves.
- Gap jumps go along the four axes only (no diagonal jumps), from and onto full-height
  floors with ordinary friction; sprint jumps need food above 6 (the controller reports
  `Stuck` otherwise and the follower plans around them).
- Not modelled: doors and gates (solid), trapdoors, entities in the way, flight, boats,
  1.16 sprint-swimming, speed / slowness effects in the costs.
- Digging and
  placing are validated in the
  simulation only (no live run yet).

Licensed under MIT OR Apache-2.0.
