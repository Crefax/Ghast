# Porting a mineflayer-style bot to ghast

A guide for moving a tick-driven behaviour from a TypeScript bot built on mineflayer (written
against a `TickContext`-style contract: one `tick(ctx)` call per game tick, actions and intent
recorded on the context) to a Rust bot built on `ghast`. The contract is the same (see
`PARITY.md`): tick model, the order of actions against the movement packet,
the rotation algorithm, and the refusal rules for place / dig / clicks. Most of a port is
mechanical. The items marked *different* below are where the code shape changes.

Everything here is from `ghast` (the facade re-exports `ghast-client`):

```rust
use ghast::{
    Behavior, BlockPos, Buttons, ClickRefused, ClickTicket, Controls, Face, Interests,
    TickContext, Vec3,
};
use ghast::inventory::{Click, ClickOutcome};
use ghast::entity::EntityClass;
```

## 1. The behaviour skeleton

```ts
// TypeScript
export class Wheat implements Behavior {
  readonly name = 'wheat';
  running = true;
  start(ctx: TickContext) { ... }
  tick(ctx: TickContext) { ... }
  stop(reason: string) { this.running = false; }
}
```

```rust
// ghast
pub struct Wheat { running: bool, /* state */ }

impl Behavior for Wheat {
    fn name(&self) -> &str { "wheat" }
    // Packet groups to decode; everything else is dropped before decoding (CPU).
    fn interests(&self) -> Interests { Interests::CHAT | Interests::BLOCK_UPDATES | Interests::ENTITIES }
    fn start(&mut self, ctx: &mut TickContext<'_>) { /* ... */ }
    fn tick(&mut self, ctx: &mut TickContext<'_>) { /* ... */ }
    fn running(&self) -> bool { self.running }
    fn stop(&mut self, _reason: &str) { self.running = false; }
}
```

- **First tick (different).** The TypeScript contract calls only `start` on the first ready tick. ghast
  calls `start` and then `tick` on that same tick, and both see the same events. If `start`
  already acts (aims, clicks), return early from that first `tick`.
- **Interests (ghast only).** Ask for what the behaviour reads. Chat needs `CHAT`. Block
  updates need `BLOCK_UPDATES`. Items and players need `ENTITIES`, and player names also
  need `PLAYER_LIST` (the tab list is the most expensive packet on busy servers).
- `running() == false` ends the behaviour. The player keeps standing and answering the
  server, like the TypeScript contract's `running` gate.
- A panic in `tick` stops only the behaviour (`stop("panic")`), never the connection or
  another bot.
- One `Behavior` value can outlive a session: each new session calls `start` again, so reset
  per-session state there.

## 2. `TickContext` mapping

Done = same name and meaning (snake_case). Only the rows that change are listed here; the
full table is in `PARITY.md`.

| TypeScript | ghast |
|---|---|
| `ctx.tick`, `ctx.pose`, `ctx.sent`, `ctx.canFly` | `ctx.tick()`, `ctx.pose()`, `ctx.sent()`, `ctx.can_fly()` |
| `pose.x / y / z`, `pose.motionX …` | `pose.pos.x …`, `pose.motion.x …` |
| `ctx.eyes()` / `ctx.eyes(p)` | `ctx.eyes(ctx.pose())` / `ctx.eyes(&p)` |
| `ctx.world.block(x, y, z)` | `ctx.world().raw_block(x, y, z)` → see §3.2 |
| `ctx.world.items()` / `players()` | `ctx.entities()` → see §3.3 |
| `ctx.inventory.slot(i)` | `ctx.inventory().window0_slot(i)` |
| `ctx.inventory.heldSlot` / `cursor` / `clickPending` | `ctx.held_slot()` / `ctx.inventory().cursor()` / `ctx.inventory().pending()` |
| `ctx.window` (id, type, title, size, slot) | `ctx.inventory().window()` (`id()`, `kind()`, `title()`), `container_len()`, `slot(i)` |
| `ctx.guiOpen` | `ctx.inventory().gui_open()` |
| `ctx.events.blockUpdates` | `ctx.events().block_updates` (`Interests::BLOCK_UPDATES`) |
| `ctx.events.inventoryChanged` | `ctx.events().inventory.player_changed` |
| `ctx.events.teleported` / `chat` | `ctx.events().teleported` / `ctx.events().chat` (§3.4) |
| `ctx.selectSlot(n)` | `ctx.select_slot(n)` |
| `ctx.place(support, face, hit)` / `ctx.dig(pos, face)` | `ctx.place(support, face, hit)` / `ctx.dig(pos, face)` (`face: Face`) |
| `await ctx.clickWindow(slot, button, mode)` | `ctx.click(Click::…)` + result on a later tick (§3.5) |
| `ctx.openInventory()` / `closeWindow()` | `ctx.open_inventory()` / `ctx.close_window()` (return `Result`) |
| `ctx.setAim(yaw, pitch)` | `ctx.set_aim(yaw, pitch)` |
| `ctx.setControls({...})` | `ctx.set_controls(Controls {...})` (§3.1) |
| `ctx.setButtons({left, right})` | `ctx.set_buttons(Buttons { left, right })` |
| `ctx.target()` | `ctx.target()` (block) / `ctx.target_entity()` (§3.6) |
| `ctx.mining`, `ctx.broken` | `ctx.mining()`, `ctx.broken()` |
| `ctx.setFlying(b)` | `ctx.set_flying(b)` |
| `ctx.status(s)` / `ctx.chat(s)` | `ctx.status(s)` / `ctx.chat(s)` |
| raw packet listeners (mineflayer `bot._client.on(...)`) | `Behavior::raw_packets` + `ctx.events().raw` (§3.9) |
| the core's last-inbound time (lag guard) | `ctx.last_inbound()` |
| a mirrored rotation step + RotationPlace hold | `ctx.planned_look()` |

Faces keep their wire numbers (`Face::from_id(1) == Some(Face::Up)`, `face.id()`).

## 3. The *different* items, before / after

### 3.1 Controls: numbers → keys

The TypeScript controls are vanilla `Input` values: the behaviour itself multiplies by 0.3
while sneaking. ghast takes keys, and the physics applies vanilla's ×0.3 (sneak) and ×0.98
itself.

```ts
const k = sneak ? 0.3 : 1;
ctx.setControls({ forward: 1 * k, strafe: -1 * k, jump: false, sneak, sprint: true });
```

```rust
ctx.set_controls(Controls {
    forward: true,   // forward > 0
    back: false,     // forward < 0
    left: false,     // strafe > 0 (vanilla: +strafe = left)
    right: true,     // strafe < 0
    jump: false,
    sneak,           // no manual ×0.3
    sprint: true,
});
```

Fractional values other than ±1 / ±0.3 have no key equivalent. A behaviour that steers with
analog strafe must turn (`set_aim`) instead. The lab trace replay reproduced every recorded tick with
keys.

Controls stay set until changed, as in the TypeScript contract. `Controls::NONE` releases everything.

### 3.2 Blocks: `{id, meta}` → raw state

```ts
const b = ctx.world.block(x, y, z);           // null = not loaded
if (b && b.id === 59 && b.meta === 7) ripe();
```

```rust
// 1.8: raw = id << 4 | meta. None = column not loaded (never treat it as air).
if let Some(raw) = ctx.world().raw_block(x, y, z) {
    let (id, meta) = (raw >> 4, raw & 15);
    if id == 59 && meta == 7 { ripe(); }
}
```

On 1.16.5 the raw value is the global block-state id. Use `ghast::data` (`v1_16_5`) to name
states rather than hard-coding numbers. For physics and tool facts (hardness, collision,
replaceable, water) use `ctx.world().block_query(x, y, z)`.

Block updates: the TypeScript contract keeps only S23/S22 near the bot. ghast gives all of them in
arrival order, so filter by distance yourself.

```rust
for (pos, raw) in &ctx.events().block_updates {
    let me = ctx.pose().pos;
    let d2 = (f64::from(pos.x) + 0.5 - me.x).powi(2) + (f64::from(pos.z) + 0.5 - me.z).powi(2);
    if d2 <= 64.0 { self.field.update(*pos, *raw); }
}
// More than 4096 between two ticks: the rest are counted here. Rescan the area if this is > 0.
if ctx.events().block_updates_dropped > 0 { self.rescan = true; }
```

### 3.3 Items and players: `world.items()` / `players()` → `entities()`

```ts
for (const it of ctx.world.items()) if (dist(it.pos, me) < 2 && it.item?.id === 296) pickUp(it);
```

```rust
// Needs Interests::ENTITIES (player names also need Interests::PLAYER_LIST).
if let Some(es) = ctx.entities() {
    let me = ctx.pose().pos;
    for e in es.near([me.x, me.y, me.z], 2.0) {
        if e.class() == EntityClass::Item && e.item().is_some_and(|i| i.id == 296) {
            self.pick_up(e.id(), e.position());
        }
    }
    let someone_close = es.iter().any(|e| e.is_player() && Some(e.id()) != es.self_id());
}
```

`es.player_name(id)` gives a player's tab-list name.

### 3.4 Chat: strings → `ChatMessage`

```rust
for m in &ctx.events().chat {
    let text = m.plain();          // the § codes are stripped; raw JSON in the message
    if text.contains("Grim") { /* ... */ }
}
```

`events().chat_dropped > 0` means more than 512 messages piled up while the behaviour was not
ticking.

### 3.5 Window clicks: promise → ticket

The TypeScript contract awaits `clickWindow`. ghast is tick-driven: `click` writes the packet now and
returns a ticket, and the outcome arrives in a later tick's events.

```ts
const ok = await ctx.clickWindow(36, 0, 0);
if (!ok) retry();
```

```rust
// state: Option<ClickTicket>
match self.pending {
    None => match ctx.click(Click::Pickup { slot: 36, right: false }) {
        Ok(t) => self.pending = Some(t),
        // Not sent: turning, another click in flight, same-tick order, resync. Try next tick.
        Err(ClickRefused::Turning | ClickRefused::Inventory(_)) => {}
    },
    Some(t) => {
        for r in &ctx.events().inventory.clicks {
            if t.matches(r) {
                self.pending = None;
                match r.outcome {
                    ClickOutcome::Accepted => { /* next step */ }
                    ClickOutcome::Rejected | ClickOutcome::TimedOut => { /* retry / rescan */ }
                }
            }
        }
    }
}
```

| `clickWindow` mode / button | ghast `Click` |
|---|---|
| 0, button 0 / 1 | `Pickup { slot, right: false / true }` (`slot: ghast::inventory::OUTSIDE` drops the cursor) |
| 1 | `QuickMove { slot, right }` |
| 2, button n | `Swap { slot, hotbar: n }` |
| 4, button 0 / 1 | `Drop { slot, all: false / true }` |
| 6 | `PickupAll { slot }` |

Clicks go to the open window (container, or window 0 after `open_inventory()`). The
"click only while still" rule is enforced by the core: `ClickRefused::Turning`.

### 3.6 Crosshair: one call → two

```ts
const t = ctx.target();
if (t?.type === 'block') ...; else if (t?.type === 'player') ...
```

```rust
if let Some(hit) = ctx.target() { /* hit.pos, hit.face, hit.hit (Vec3), hit.distance */ }
if let Some(e) = ctx.target_entity() { /* e.id: any pickable entity, vanilla GameRenderer.updateTargetedEntity */ }
```

Both are computed from the **sent** pose, which is what the server judges. Aim first, then
act on a later tick, exactly as in the TypeScript contract.

### 3.7 Place and the held count

The TypeScript contract decrements the held count for every block item it places. ghast predicts only
1.8 planting (seeds, carrot and potato on farmland, nether wart on soul sand, air above),
which is exactly vanilla's `SeedItem` check. Other blocks wait for the server's set_slot.
A behaviour that counts its stock should read `ctx.inventory()` and not keep its own counter.

### 3.8 Flight

`set_flying(true)` needs the server's "may fly" and must be called in the air: vanilla ends
flight on landing (it sends the `abilities` packet itself), which is why a guard like
`if (!pose.flying && !pose.onGround) ctx.setFlying(true)` is needed.

### 3.9 Raw packets, lag, the look the physics will use

Guards that listen to raw packets (mineflayer `bot._client.on(...)`) name the ids they want;
the frames arrive undecoded in the next tick's events. Packets the core consumes itself are
included, and the core still handles them as before.

```rust
use std::borrow::Cow;
use ghast::protocol::v1_8::play::clientbound::{self as cb, id};

fn raw_packets(&self, v: GameVersion) -> Cow<'static, [i32]> {
    match v {
        GameVersion::V1_8_9 => Cow::Borrowed(&[id::PLAYER_INFO, id::TITLE, id::POSITION]),
        _ => Cow::Borrowed(&[]),
    }
}

fn tick(&mut self, ctx: &mut TickContext<'_>) {
    for r in &ctx.events().raw {
        // r.at: when the frame was read (e.g. a tab-list add + remove within 100 ms).
        let raw = ghast::protocol::RawPacket::new(r.id, r.body.clone());
        if r.id == id::PLAYER_INFO {
            if let Ok(p) = raw.decode_as::<cb::PlayerInfo>() { /* ... */ }
        }
    }
    // Lag: how long since the last inbound bytes (e.g. > 1.5 s before a setback).
    let quiet = ctx.last_inbound().map(|t| t.elapsed());
    // The yaw / pitch this tick's movement step will use (after set_aim, including a turn
    // held by hold_turn_on_action): pick W/A/S/D from it instead of mirroring the rotation.
    let (yaw, _pitch) = ctx.planned_look();
}
```

## 4. Grim settings

| setting | library default | recommended with Grim | why |
|---|---|---|---|
| `Client::hold_turn_on_action` | on | on | Grim RotationPlace re-checks a dig / place with the next look. It only delays a turn. |
| `Client::force_step_position_mode` | `Off` (vanilla) | `StepUpOnly` | Grim 2.3.73 sets back a vanilla landing step-up under 0.03 (soul sand → grass); see `crates/ghast-physics/GRIM_NOTES.md` |
| `Client::sensitivity` | 0.5 | the account's options.txt value | rotation steps are multiples of its GCD; keep it stable per account |
| `Client::silent_hold` | 1.5 s | 1.5 s | freeze the behaviour when a proxy stalls |

- `StepUpOnly` sends the position only on a step-up tick: on ground after the move, with
  the feet higher. Falls and descents stay vanilla.
- `Always` (`force_step_position(true)`) is the variant measured at 0 setbacks in 16 min.
- Check `StepUpOnly` on the lab (`lab/alerts.sh 1.8 --count`) before using it on a real
  server. If it misses a case, use `Always`.
- Either way, keep behaviours off soul-sand edges and slime where possible. 1.16.5 slime
  walking is a separate Grim GroundSpoof case.

## 5. Runner: one process per bot

Running one bot per process means bots can never crash together. Use a `current_thread`
runtime (one thread, ~4 MB RSS idle) and a reconnect loop. Supervise the processes with
systemd (e.g. a `bot@<name>.service` template) or any process supervisor. (Many bots in one
process on a multi-threaded runtime work as well; the session future is `Send`.)

```rust
use std::time::{Duration, Instant};

use ghast::connect::{Connector, ProxyPolicy, Socks5Proxy, TargetAddr};
use ghast::{Account, Client, ForceStepPosition, GameVersion, SessionEnd};

fn main() {
    let cfg = config::load(std::env::args().nth(1).expect("usage: bot <config.json>"));
    logger::init(&cfg.name); // mask passwords in logged commands
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    std::process::exit(rt.block_on(run(cfg)));
}

async fn run(cfg: config::Bot) -> i32 {
    // Through a SOCKS5 proxy: socks5h:// = remote DNS, user:pass supported.
    let proxy: Socks5Proxy = match cfg.proxy.parse() {
        Ok(p) => p,
        Err(e) => {
            log::error!("proxy: {e}");
            return 2;
        }
    };
    let client = Client::new(
        TargetAddr::new(&cfg.host, cfg.port).expect("address"),
        GameVersion::V1_8_9,
        Account::offline(&cfg.username).expect("name"),
    )
    .connector(
        Connector::new()
            .with_optional_proxy(Some(proxy))
            .with_policy(ProxyPolicy::RequireProxy), // never fall back to a direct connection
    )
    .sensitivity(cfg.sensitivity)
    .hold_turn_on_action(true)
    .force_step_position_mode(ForceStepPosition::StepUpOnly);

    let mut behaviour = behaviours::create(&cfg); // Box<dyn Behavior> works too
    let (min, max) = (Duration::from_secs(5), Duration::from_secs(120));
    let mut backoff = min;
    loop {
        let started = Instant::now();
        let shutdown = async {
            // SIGTERM (systemd stop) and Ctrl-C: tokio "signal" feature.
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM handler");
            tokio::select! { _ = term.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        };
        match client.run_until(&mut behaviour, shutdown).await {
            Ok(SessionEnd::Shutdown) => return 0,
            Ok(SessionEnd::Quit(reason)) => {
                log::info!("quit: {reason}");
                return 0;
            }
            Ok(SessionEnd::Kicked { reason, .. }) => log::warn!("kicked: {reason}"),
            Ok(SessionEnd::ServerClosed) => log::warn!("server closed the connection"),
            Err(e) => log::warn!("session failed: {e}"),
        }
        // Reconnect with backoff (5 s .. 120 s): reset after a session that lasted.
        if started.elapsed() > Duration::from_secs(300) {
            backoff = min;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(max);
    }
}
```

- `Box<dyn Behavior>` implements `Behavior`, so a runner can pick the behaviour from the
  config at run time.
- Stats, remote control and server-specific handling belong in the application. They read
  the same `TickContext` (wrap the behaviour in a behaviour of your own that ticks it).
- For a Raspberry Pi 5, cross-compile: `cargo build --release --target
  aarch64-unknown-linux-gnu` with `-C target-cpu=cortex-a76` (see `lab/build-pi.sh`).

## 6. Checking a port

1. **Lab first.** Run the ported behaviour on a local lab server (see `lab/README.md`) with a
   throwaway account. Then `lab/alerts.sh 1.8 --count --since HH:MM` must print nothing.
2. **Replay.** Record a packet trace of the original behaviour on the lab. Then check that
   ghast's engine produces the same packets: `GHAST_TRACE=… cargo test -p ghast-client
   --release --lib lab_trace_replay -- --nocapture`. The replay world is the wheat field;
   other layouts need their own `field_block`.
3. **Real servers last**, and only within their rules.
