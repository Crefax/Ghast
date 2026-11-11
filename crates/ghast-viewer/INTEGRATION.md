# Integrating ghast-viewer into a bot

This guide is for apps built on [ghast](https://crates.io/crates/ghast). It shows how to give each
bot a bot screen, protect it with a token, and record what the bot saw: always, on demand, or with
a dash cam that saves the last minutes when something happens. The full example is
[`examples/dashcam_nearby.rs`](examples/dashcam_nearby.rs).

The viewer is read-only. It watches the bot's ticks and never writes to the game connection, and
the page can only send clock pings, flow-control ACKs and playback controls. 1.8.9 and 26.1
sessions are drawn; 26.1 through the 1.8.9 assets (see the README).

## 1. Wrap the behaviour

Use one `Viewer` per bot. Build the client with `ghast_viewer::world_config()` so ghast keeps
light, biomes and block entities. Without it, signs have no text, skulls, banners and flower pots
are missing, and shading is flat.

```rust
use ghast_client::{Account, Client, GameVersion};
use ghast_viewer::{Viewer, ViewerConfig};

let viewer = Viewer::new(ViewerConfig::new("ViewerBot").port(8090));
let client = Client::new(addr, GameVersion::V1_8_9, Account::offline("ViewerBot")?)
    .world_config(ghast_viewer::world_config());
let mut bot = viewer.watch(my_behaviour); // Watched<MyBehaviour>
viewer.start().await?;                    // serves http://127.0.0.1:8090/
client.run(&mut bot).await?;
```

`Watched<B>` passes everything through to your behaviour: `interests` (combined with the
viewer's), raw packets, `start`, `tick`, `running`, `after_tick` and `stop`. The viewer reads the
bot in `after_tick`, after the movement step, so the page shows the pose the server has. A
finished behaviour is not ticked any more, but the screen keeps showing the bot. Use
`bot.inner()` / `inner_mut()` to reach your behaviour.

Keep the same `Viewer` (and the same `Watched`, or `viewer.watch(..)` a new behaviour) across
reconnects. A new session resets the page by itself.

## 2. Ports, tokens and access

- **Port:** each bot needs its own port (`.port(..)`). With one bot per process, pass it in the
  bot's config. The default, port 0, picks a free one; `viewer.start()` returns the address.
- **Host:** the default is `127.0.0.1`. To reach the screen from elsewhere, either put a reverse
  proxy in front of it (it passes WebSockets) or bind `.host([0, 0, 0, 0].into())`, and **always set a
  token** when you do.
- **Token:** with `.token(..)`, every route needs it:
  - Open `/?token=<token>` once. The page sets an `HttpOnly; SameSite=Strict` cookie, so links
    without the token keep working afterwards.
  - Programs send `Authorization: Bearer <token>`.
  - Without the token, the answer is `401`. The comparison runs in constant time.
- **Routes:**
  - `/` is the page. `?mode=fp|tp|free` picks the camera and `?replay=<name>` plays a recording.
  - `/status` returns `{running, port, viewers, inGame, account}` for a dashboard or hub.
  - `/replays` lists the recordings.
- **Idle stop:** the server stops itself after `.idle(..)` (default 60 s) with no page
  open, and it costs nothing while stopped. Call `viewer.start().await` again when someone asks
  for the screen. It is idempotent and returns the running address.
- **Status line:** `viewer.set_status(status, message)` sets the line under the bot's name, and
  the message shown while the bot is not in a world (e.g. "reconnecting in 30 s").

## 3. Entities

`ViewerConfig::entities(EntityTracking::…)` controls when entities and skins are tracked:

| setting | cost | page |
|---|---|---|
| `Always` (default) | entity + tab-list tracking for the whole session | a page opened later shows everything at once |
| `WhileWatched` | none while nobody watches or records | entities appear as the server sends them again (when they move into view or respawn) |
| `Off` | none | no entities |

If your behaviour needs entities anyway (a guard that looks for players), it asks for them in its
own `interests`, and the viewer adds nothing on top.

## 4. Textures and the cache

Textures and models come from the vanilla 1.8.9 client jar, which is never shipped with this
crate. On first start the viewer downloads the jar from Mojang's servers (SHA-1 checked) into
`cache_dir`. It then builds the block data and texture pack (about 0.1 s) and caches them there.

- Use an **absolute, shared** `cache_dir` for all bots on a machine (the default
  `.cache/ghast-viewer` is relative to the working directory). Several bot processes may start at
  once; the download and the build are safe to run concurrently.
- Offline machines: `.client_jar(path)` points at a local copy. The download needs the default
  `download` feature.

## 5. Recordings

A recording is exactly what a page would have received:

- `<dir>/<name>.rec` holds the frames (gzip).
- `<dir>/<name>.json` holds the meta: account, start, end, sizes, frame count and an optional note.
- Names are the UTC start time, `YYYY-MM-DD_HH-MM[-n]`.

The page lists the recordings of `ViewerConfig::replay_dir` and plays them back with pause, speed
(1–16×) and seeking. Write recordings into that folder.

```rust
use ghast_viewer::FeedRecorder;

let mut rec = FeedRecorder::start(&viewer, &replay_dir); // on disk at once
// …
let meta = rec.stop(); // also on drop
```

- A recording may start before the bot joins. Its clock starts at the first in-game frame, so
  0:00 shows the bot in the world.
- A recording costs about what one watching page costs (section 8).
- A crash leaves a playable file: the data is flushed every 10 s, and only the `end` time is
  missing.
- A runaway recording stops at 400 MB of raw frames.

To record only part of the time (e.g. while the bot works in a certain area), start a
`FeedRecorder` when entering and `stop()` it when leaving. Check the condition every second or
so, not every tick.

## 6. Dash cam

`DashCam` keeps the last few minutes in memory and saves them on demand, plus a little more after
the trigger. This is the cheap way to keep evidence of rare events (a player came near, the bot
took damage, a staff message arrived) without recording all day.

```rust
use ghast_viewer::{DashCam, DashCamOptions};

let v = viewer.clone();
let cam = DashCam::start(&viewer, &replay_dir, DashCamOptions {
    active: Box::new(move || v.status().in_game /* && !paused */),
    every: Duration::from_secs(120), // a new in-memory segment this often
    life: Duration::from_secs(240),  // … each kept this long: ≥ 2 min before the trigger
    after: Duration::from_secs(60),  // keep recording this long after the trigger
    max_bytes: 256 << 20,            // folder limits, applied after each clip
    max_age: Duration::from_secs(7 * 86_400),
});
// later, on an event:
if let Some(name) = cam.clip("player near: Alex") { /* link /?replay=<name> */ }
```

- A clip covers at least `life − every` before the trigger and `after` after it. Right after
  buffering began it covers less, and with nothing buffered yet it starts at the trigger. A second
  `clip()` while one runs extends it and returns the same name, so triggers do not need a
  debounce for correctness, only to keep logs quiet.
- `clip()` writes the buffered minutes to disk, typically a few MB. Call it off the tick thread:
  have the behaviour send the note over a channel to a task that calls it inside
  `spawn_blocking`, as the example does.
- `active` gates the buffer. Outside it (not in a world, bot paused) nothing is buffered and
  nothing costs. While active, the dash cam costs about what a recording costs.
- **A trigger for "a player came near"** (from the example): every tick, look at
  `ctx.entities()` for player entities with a tab-list name (`player_name`). Skip yourself, your
  app's other bots and server NPCs (UUID version 2). The entity tracking range of the server
  (48 blocks for players by default) is how far a bot can see players at all.

## 7. Pruning

The dash cam prunes its folder after every clip (`max_bytes`, `max_age`, never the clip just
saved). For `FeedRecorder` recordings, call it yourself, e.g. when a recording stops and once at
start-up:

```rust
ghast_viewer::prune_recordings(&replay_dir, 512 << 20, Duration::from_secs(14 * 86_400), None);
```

It removes the oldest recordings until the folder fits both limits. Pass `Some(name)` to keep a
recording in progress.

## 8. Costs

Measured on x86-64 (WSL), 1.8 lab server, one bot process:

| state | CPU (one core) | RSS |
|---|---|---|
| bot standing, no page | 0.38 % | 10 MB |
| bot standing, one page | 0.46 % | 11 MB |
| bot moving (movement course), no page | 0.35 % | 11 MB |
| bot moving, one page | 0.67 % | 12 MB |
| bot moving, recording (no page) | 0.52 % | 12 MB |

These are 5-minute runs, measured over 4.5-minute windows. The bot without the viewer was not
measured here. A page of a moving bot gets about 1.5 KB/s,
20 poses a second plus the columns along the way. A recording of the same run took 215 KB
(about 2.6 MB an hour).

A dash cam costs about what a recording costs, while it is active.

While nobody watches and nothing records, the viewer builds no frames and copies no columns.
Its only work is keeping a few small things (weather, experience) and, with
`EntityTracking::Always`, the entity tracking. Column encoding runs off the tick thread, and each
column is encoded once for all pages. A page that falls behind gets its column stream paused
by the flow-control window and its per-tick extras dropped, never queued without bound.
