# ghast-pathfinder

Path planning and path following for the ghast Minecraft Java Edition bot library, 1.8.9 and
1.16.5. The planner is an A* over feet blocks; the follower drives the vanilla-exact
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

## Moves

| move | what | cost (ticks) |
|---|---|---|
| `Walk` | to a cardinal neighbour, step-ups up to 0.6 (slabs, soul sand, carpets) and drops up to 0.6 | measured walk / sprint speed |
| `Diagonal` | both side columns clear **and floored** (a stray box never drops into a hole) | √2 × straight |
| `Ascend` | jump onto a block 0.6–1.0 higher, with headroom for the arc | measured standing jump |
| `Descend` | walk off an edge and fall, at most `max_fall` (3: no fall damage) or `max_water_fall` into deep water | measured fall table |
| `Parkour { gap }` | jump over a 1-block gap (walking), or a 2-block gap with a sprint run-up from the block behind | measured jump |
| `Swim` | in water, up, down, out onto land | measured swim speeds |
| `ClimbUp` / `ClimbDown` / `ClimbExit` | ladders and vines, also onto a ladder from above | measured climb speeds |
| dig / place (opt-in) | tunnel through blocks (`Config::with_dig`), fill one-deep holes (`Config::with_place`) | `DigCost`, place cost |

Costs are not guesses: `Costs::of(version)` measures them once per version by running the
physics on canonical terrain. Unknown (unloaded) blocks are never treated as air: they are
impassable, and a goal beyond them gives a partial path that ends at the loaded edge.
Lava, fire, cobwebs, berry bushes, portals, magma / campfires / cactus as floors are avoided;
doors, fence gates, scaffolding and other shapes the planner does not model are treated as
solid.

## API

- `Pathfinder::new(Config)`; `plan(version, &world, from, &goal)`,
  `plan_avoiding(.., &banned_moves)`, `plan_verified(.., attempts)` (plan, simulate, plan
  around the move the simulation failed), `Pathfinder::start_block(feet_pos)`.
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
  paths. It acts only through the `TickContext`: aim in whole mouse counts, keys, the left
  button for digs, `place` for places, so the tick model of ghast-client holds (actions
  before the tick's single movement packet).

## Validation

`tests/validation.rs`, for both versions, plans on fake worlds and runs each path through
the physics with the controller, asserting the goal block is reached with no unexpected
horizontal collision and no fall damage: flat runs (straight, diagonal, no sprint), a wall
with one opening, glass corridors, staircases up, drops of up to 3, slabs / soul sand
steps, a fence row (gone around), 1- and 2-block gaps over lava (and none without parkour),
a pool (swum across; impossible without swimming), a ladder up and down a cliff, a tunnel
dug through a dirt wall, a trench filled with blocks, a goal beyond unloaded chunks
(partial path), all goal kinds, verified plans, and 96 fixed random rough terrains
(heights 0–3 per block, slabs, soul sand, holes). An ignored proptest fuzzes more terrains:
`cargo test --release -p ghast-pathfinder -- --ignored`.

## Performance

`cargo bench -p ghast-pathfinder` (x86_64 WSL, release, 1.8.9, 2026-11-06):

| case | steps | nodes | plan | simulate |
|---|---|---|---|---|
| flat, 100 blocks straight | 101 | 100 | 0.28 ms | 2.2 ms (359 ticks) |
| flat, 50 × 50 diagonal (~71 blocks) | 51 | 50 | 0.12 ms | 2.0 ms (254 ticks) |
| rolling terrain with a long wall, 80 × 60 (detour) | 193 | 27,091 | 57 ms | 24 ms (1,708 ticks) |

About 400–480 k nodes/s. The controller costs ~6–14 µs per tick (physics look-ahead
included). Planning runs where it is called; a long detour through rough terrain can exceed
one 50 ms tick, so plan nearer goals, cap `Config::max_nodes`, or raise
`with_heuristic_weight` for long trips.

## Live check

1.8 lab (Paper 1.8.8 + Grim 2.3.73, local, 2026-11-06): the `goto` example of the `ghast`
crate (`--features ext,pathfinder`) walked the test pad's step-up course, wall, fence,
water pool, soul sand, slime, ice and random points: 36 legs in 5 min (with
`--grim-compat`) and 27 legs in 3 min (without), **0 Grim flags, 0 setbacks**.

## Known limits

- About 1 in 300 fuzzed rough terrains still ends `Stuck` / `OffPath` in the simulation
  (awkward lines of slabs and soul sand between holes). `plan_verified` and the follower
  plan around such moves.
- Not modelled: doors and gates (solid), trapdoors, entities in the way, flight, boats,
  1.16 sprint-swimming, speed / slowness effects in the costs.
- 1.16.5 is validated in the simulation only; digging and placing are validated in the
  simulation only (no live run yet).

Licensed under MIT OR Apache-2.0.
