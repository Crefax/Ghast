# ghast-viewer

A bot screen for bots built on [ghast](https://crates.io/crates/ghast): a live 3D view of the
bot's world in the browser (first person, third person or a free camera), plus recordings, a
dash cam and replays on the same page.

- **Read-only.** The viewer wraps your behaviour and reads the bot in `after_tick`; it never
  writes to the game connection. The page can only send clock pings, flow-control ACKs and
  playback controls.
- **Free while nobody watches.** No frames are built and no columns copied until a page is
  open; the server stops itself after an idle timeout.
- **One per bot.** Each bot serves its own page on its own port, on `127.0.0.1` by default,
  optionally protected by a token.
- **No game assets in the crate.** Textures and block models are built at run time from the
  vanilla client jar, downloaded from Mojang's servers into a local `.cache/ghast-viewer`
  (feature `download`, on by default), or read from a local copy (`ViewerConfig::client_jar`).
- 1.8.9 and 26.1 sessions are drawn (26.1 with the `v26_1` feature, on by default).
  The page draws with the 1.8.9 assets: a 26.1 world is mapped onto them (blocks and items
  1.8 has keep their state, newer ones become a look-alike, see `ghast_viewer::v26_1`), and
  the page shows the 256 rows of the dimension around the bot (it moves the window and redraws
  when the bot leaves it). Entities are listed from the 26.1 tracker (mob models where 1.8 has
  the mob, else a box of the entity's size; dropped items show their stack); other entities'
  held items and armour, chest lids, particles, weather, experience and other players' block
  cracks are 1.8-only.

## Use

Through the facade crate:

```sh
cargo add ghast --features viewer     # ghast::viewer
```

or on its own: `cargo add ghast-viewer`.

```rust,ignore
use ghast::viewer::{Viewer, ViewerConfig};
use ghast::{Account, Client, GameVersion, Idle};

let viewer = Viewer::new(ViewerConfig::new("Bot").port(8090));
let addr = viewer.start().await?;
println!("open http://{addr}/");
let client = Client::new("127.0.0.1:25565".parse()?, GameVersion::V1_8_9, Account::offline("Bot")?)
    .world_config(ghast::viewer::world_config());
let mut behaviour = viewer.watch(Idle);
client.run(&mut behaviour).await?;
```

The integration guide ([INTEGRATION.md](INTEGRATION.md)) covers ports, tokens, recordings, the
dash cam and costs. The wire protocol between server and page is in
[docs/protocol.md](docs/protocol.md).

## Examples

```sh
cargo run -p ghast-viewer --release --example afk_viewer -- 127.0.0.1:25565 ViewerBot 8090
cargo run -p ghast-viewer --release --example dashcam_nearby -- 127.0.0.1:25565 ViewerBot 8090 <token> replays
cargo run -p ghast-viewer --example dump_assets -- --download assets-out
```

## Web client

The page (TypeScript, WebGL2) lives in `web/`; its built files in `web/dist/` are committed and
embedded in the crate, so `cargo build` needs no Node. See [web/README.md](web/README.md) to
rebuild or test it.

## Licence

MIT OR Apache-2.0.
