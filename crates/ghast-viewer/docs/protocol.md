# Bot screen wire protocol (revision 1)

The page and the server talk over one WebSocket per page (`/ws`). Every message is binary and
little endian. The server side is `src/protocol.rs` (plus `src/feed.rs`, which writes the
messages); the page side is `web/src/shared/protocol.ts`. `tests/known_frames.rs` and the web
tests check that both sides encode the same bytes.

## Frames and messages

A WebSocket message (a *frame*) is a run of messages, each
`[u8 type][u32 length][payload]`. The server sends:

- one frame per client tick (the bot's pose, entity moves, block changes and other changes of
  that tick), only when something changed;
- one frame per chunk column.

Unknown types are skipped by their length, so new message types can be added without a new
revision. Type `0xff` is reserved for "skip me" (a replay blanks the recorded `HELLO` this way).

Types used below:

- `str` is a `u16` byte length followed by UTF-8.
- `json` is a whole payload of UTF-8 JSON.
- A block state is the 1.8 `id << 4 | meta`.
- Angles are radians. Yaw 0 looks to −z (north), and +yaw turns left (π/2 looks to −x). Pitch
  > 0 looks up. Conversion from vanilla degrees: `yaw = π − yaw°·π/180`, `pitch = −pitch°·π/180`.
- Times (`serverMs`) are the server's wall clock in ms. The page estimates the offset with
  `PING`/`PONG` and interpolates poses with it.
- `SELF` (`0xffffffff`) stands for the bot itself in `EVENTS`, `COLLECT`, `SKIN` and `LOOKS`.
  The bot's own entity is never spawned on the page.

## Server → page

| type | name | payload |
|---|---|---|
| 1 | HELLO | json `{protocol, version, account, mode, viewDistance, now, replay?}`. `replay` = `{name, start, duration}` on a recording's socket |
| 2 | STATE | json `{inGame, message, status, account}` |
| 3 | CHUNK | `i32 cx, i32 cz, u8 deflated`, then a column body (below), deflate-raw when `deflated = 1` |
| 4 | UNLOAD | `i32 cx, i32 cz` |
| 5 | BLOCKS | `u16 n × (i32 x, u8 y, i32 z, u16 state)` |
| 6 | POSE | `f64 serverMs, u32 tick, f64 x, y, z, f32 yaw, pitch, u8 flags, f32 vx, vy, vz`. Flags: 1 ground, 2 flying, 4 sneak, 8 sprint, 16 swung since the last pose, 32 using the held item, 64 riding |
| 7 | SPAWN | `u32 id, u8 kind, str name, str label, f64 x, y, z, f32 yaw, pitch, headYaw, f32 width, height, u16 item, u16 meta, u16 variant`. Kind: 0 other, 1 player, 2 item, 3 mob, 4 object, 5 experience orb (its value in `meta`). The variant is mob-specific (slime size, sheep colour, horse type …) |
| 8 | MOVES | `u16 n × (u32 id, f64 x, y, z, f32 yaw, pitch, headYaw)` |
| 9 | REMOVE | `u16 n × u32 id` |
| 10 | EQUIP | `u32 id, u16 item, u16 meta`: an entity's held item, or a dropped item's stack |
| 11 | INVENTORY | `u8 held, 9 × (u16 id, u16 meta, u8 count)`: the hotbar, id 0 = empty |
| 12 | VITALS | `f32 health, u8 food, f32 saturation, u8 xpLevel, f32 xpProgress` |
| 13 | TIME | `f64 serverMs, f64 worldAge, f64 timeOfDay` (0..24000, negative: the daylight cycle is off) |
| 14 | PONG | `f64 clientMs, f64 serverMs` |
| 15 | RESET | empty: drop every column and entity (a new session; a replay seeking back) |
| 16 | CRACKS | `u16 n × (i32 x, u8 y, i32 z, u16 state, u8 stage 0..9)`: every block being broken now (by the bot or others). Replaces the previous set |
| 17 | WEATHER | `f32 rain, f32 thunder` (0..1) |
| 18 | EVENTS | `u16 n × (u32 id, u8 event)`. Event: 0 arm swing, 1 hurt, 2 death |
| 19 | ESTATE | `u16 n × (u32 id, u8 flags, u32 vehicle id + 1)`. Flags: 1 fire, 2 sneak, 4 sleeping, 8 sprint, 16 using an item, 32 invisible. Vehicle 0 = none |
| 20 | COLLECT | `u32 collected id, u32 collector id`: an item or orb flies to its collector (a `REMOVE` follows) |
| 21 | CHESTS | `u16 n × (i32 x, u8 y, i32 z, u8 viewers)`: a chest lid is open while `viewers > 0` |
| 22 | SKIN | `u32 id, u8 flags, str skin, str cape`. Flag 1 = slim arms. Paths are relative to the page (`skins/<sha>.png`); '' = the default skin / no cape. Sent after the `SPAWN`, and again when a download finished or the profile changed |
| 23 | LOOKS | `u32 id, u8 flags, u8 n × (u8 slot 1..4 boots..helmet, u16 item, u16 meta, u32 leather rgb or 0xffffffff)`, then with flag 2 `18 × f32` degrees (armour stand head, body, arms, legs: x, y, z each). Flag 1 = baby / small. Sent after the `SPAWN` when not plain, and on every change |
| 24 | PARTICLES | `u16 n × (u8 id, f64 x, y, z, f32 ox, oy, oz, f32 speed, u16 count, u16 data)`. `id` is the 1.8 particle id, or `100 + (effect − 2000)` for world-event effects |
| 25 | EXPLOSION | `f64 x, y, z, f32 radius, u16 n × (i8 dx, dy, dz)` |
| 26 | LIGHTNING | `f64 x, y, z` |
| 27 | REPLAY | `f64 cursor ms, f64 duration ms, f32 speed, f64 start`: a recording's playback state, about 4 per second |

A page connects and gets `HELLO`, `STATE` and a snapshot: the inventory, vitals, time, weather,
every known entity with its `SKIN`/`LOOKS`, and the pose. The columns follow nearest first.
After that it gets tick frames.

## Page → server

The page cannot reach the game connection. Any other message, any text message, or a message
over 64 bytes closes the socket.

| type | name | payload |
|---|---|---|
| 1 | PING | `f64 clientMs`, answered with `PONG` |
| 2 | ACK | `f64` bytes received so far |
| 3 | REPLAY | `f64 speed` (0 = paused), `f64 seek ms` (−1 = none). Recordings only |

## Flow control

The page ACKs the bytes it has received. At most a *window* of column bytes (384 KiB by default)
is in flight unacknowledged, and at most four columns are being encoded for one page at once, so
a slow link never builds a queue that delays the pose.

The tick frame has two parts:

- The reliable part (blocks, spawns, removes, inventory …) is always sent.
- The volatile part (`POSE`, `MOVES`) is dropped while more than 256 KiB wait in the socket.
  Once the page catches up, it gets every entity's position again.

A page with more than 8 MiB waiting is disconnected.

## Column body

```text
u16 section mask (bit y: section y is present; all-air sections are left out)
u8  has sky
per present section, bottom up:
    u8  bits per block (0, 1, 2, 4, 8 or 16)
    u16 palette size, palette size × u16 state
    indices: 0 bits → none (one state); 16 bits → 4096 × u16;
             otherwise packed LSB first, 8 / bits per byte, in x + 16·(z + 16·y) order
    u8  light flags: 1 sky all 15, 2 sky all 0, 4 block light all 0
    2048 bytes sky light unless flag 1 or 2; 2048 bytes block light unless flag 4
    (nibbles in 1.8 order: low nibble = even index)
256 bytes biome ids (x + 16·z)
u16 signs   × (u8 x, u8 y, u8 z, 4 × str line)                  chunk-local x/z, plain text
u16 tiles   × (u8 x, u8 y, u8 z, u8 kind, …)
    kind 1 skull:  u8 type (0 skeleton … 4 creeper), u8 rotation 0..15, str skin path ('' = Steve)
    kind 2 banner: u8 base dye, u8 n × (str pattern, u8 dye)
    kind 3 pot:    str contents (the blockstates "contents" value)
```

The page lights missing sections as open sky.

## Recordings

A recording (`<name>.rec`) is a gzip stream of records `u32 ms, u32 length, frame`. `ms` counts
from the bot's first in-game frame; frames before it are stamped 0. The frames are exactly what a
page would have received, so playback rewrites only the clock fields of `POSE` and `TIME` onto
the playback clock, and blanks the recorded `HELLO`. Seeking compacts the frames up to the
target (the last state per block and entity, no one-off effects) and sends them at once. Seeking
backwards sends `RESET` first.
