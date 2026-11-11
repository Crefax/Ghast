# ghast-ext

Sequential, mineflayer-style async helpers for ghast bots. ghast's bot API is a per-tick
state machine (`Behavior::tick`): exact and cheap, but every multi-step job becomes an
explicit state enum. ghast-ext lets you write the same job as straight-line async code
without giving up the tick model.

Versions: one cargo feature each, `v1_8`, `v1_12_2`, `v1_16_5`, `v26_1` (default: all, plus
`pathfinder`), forwarded to ghast-client, ghast-data and the path finder. `goto` works on every
version (1.8.9, 1.12.2, 1.16.5, 26.1). Item and block names and ids follow the connection's version,
26.1 included.

```rust,no_run
use ghast_client::{Account, BlockPos, Client, Face, GameVersion, Interests};
use ghast_ext::{Ext, pathfinder::Goal};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let mut bot = Ext::new(|bot| async move {
    bot.goto(Goal::Block([10, 64, 3])).await?;  // planned, physics-checked path
    let below = BlockPos::new(10, 63, 4);
    bot.dig(below).await?;                       // best hotbar tool, vanilla dig time
    bot.equip("cobblestone").await?;
    bot.place(BlockPos::new(10, 62, 4), Face::Up).await?;
    let m = bot.command("/balance", |m| m.plain().contains("Balance"), 100).await?;
    println!("{}", m.plain());
    Ok(())
})
.interests(Interests::CHAT);

let client = Client::new("localhost:25565".parse()?, GameVersion::V1_8_9, Account::offline("Bot")?);
client.run(&mut bot).await?;
# Ok(()) }
```

## How it works

`Ext` is a `Behavior` with a tiny single-threaded executor inside. Every tick it polls the
tasks that are ready; a task reaches the game only through `Bot::with`, which hands a closure
to the executor to run with that tick's `TickContext` and resumes the task in the same tick.
So every action is written inside `Behavior::tick`, before the tick's single movement packet,
in call order; dig / place are refused unless the **sent** crosshair is on the block (the
helpers aim first and act on a later tick); clicks wait while turning or while another click
is in flight; rotation is whole mouse counts. Nothing runs between ticks, the silent-server
guard holds everything, and there is no `unsafe`.

Helpers fail with `Error` when the world changes under them (teleport / setback, death), and
dropping a helper's future releases the keys and buttons it held. One task should own the
look and the movement keys at a time.

## API

- Executor: `Ext::new(main)` with `name`, `interests`, `when_done`, `auto_respawn`,
  `interrupt_on_teleport`, `bot`, `result`; `Bot::with`, `act`, `wait_ticks`, `next_tick`,
  `sleep`, `timeout`, `wait_until`, `spawn` (`JoinHandle`: await, `abort`, `is_finished`),
  `require`; snapshot reads `tick`, `version`, `pose`, `sent`, `eyes`, `health`, `is_dead`,
  `game_mode`, `held_slot`, `open_window`, `entity_id`, `dimension`.
- Looking: `look`, `look_at`, `look_at_block`, `yaw_pitch`.
- Hands: `dig`, `dig_with(DigOptions)`, `place`, `activate_block`, `consume`,
  `activate_item`, `deactivate_item`, `swing`, `attack`.
- Items: `count`, `find_item`, `held_item`, `select_slot`, `equip`, `equip_for_block`,
  `swap_to_hotbar`, `drop_slot`, `toss`, `click`, `close_window`, `open_container`,
  `container_items`, `deposit`, `withdraw`; `ItemMatch` (name, id or predicate), `item_id`,
  `item_name`.
- World: `block_at`, `find_blocks`, `find_block` (`BlockMatch`: name, names or predicate;
  `Block` names raw states), `find_entities`, `nearest_entity`, `entity`, `player`.
- Chat and events: `chat`, `command`, `wait_for_chat`, `wait_for`, `events` (`Event`
  stream), `respawn`, `disconnect`.
- Moving: `walk_straight_to`, `follow` (straight line), `set_controls`; `goto` / `goto_with`,
  `follow_path` / `follow_path_with` (feature
  `pathfinder`, on by default: ghast-pathfinder's planner and follower, re-exported as
  `ghast_ext::pathfinder`).

## Tests

`tests/helpers.rs` runs every helper group against an in-process 1.8 fake server
(`tests/support`) with a real login, real 50 ms ticks and real packets: look (server sees
the final look), look-at-block (face, out of reach, not loaded), dig (start / finish packets,
the swing and the movement packet after the start in the same tick, refusals), place and
equip from the hotbar (refusals), equip from the main inventory (number-key swap click),
toss, chest withdraw / deposit (shift clicks), chat / commands / events / teleports,
teleport interrupting a walk (keys released), straight walking, block and entity search,
the executor (ticks, same-tick `with`, timeouts, spawn / abort / panics, `act` order) and
`goto` around a wall.

Licensed under MIT OR Apache-2.0.
