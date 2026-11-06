# ghast-viewer web client

The page the viewer server hands to the browser: a WebGL2 renderer of the bot's world (terrain, entities,
mobs, players with skins and armour, particles, weather, lightning, the HUD), a mesher in a Web Worker and the
replay controls. It only draws: nothing the page does reaches the bot or the Minecraft server.

The page talks to the server over one binary WebSocket. The format is in `src/shared/protocol.ts` and
`docs/protocol.md`. Block models and textures come from the server at run time (`data.<hash>.json`,
`pack.<hash>.bin`, built from the vanilla client jar the server downloads), so the repository holds no game
assets.

## Building

The built files in `dist/` are committed: the Rust crate embeds them, so `cargo build` works without Node.
Rebuild them after changing anything in `src/` or `page.html`:

```sh
cd web
npm ci              # once: esbuild, TypeScript, vitest (node_modules is git-ignored)
npm run build       # writes dist/page.html, app.js, worker.js, images.json
npm run check       # fails when dist/ does not match the sources (CI)
npm run typecheck
npm test
```

`node build.mjs --dev` builds unminified bundles with inline source maps for debugging (do not commit them).

| File | What it is |
|---|---|
| `dist/page.html` | the page; the server fills in `%APP%`, `%WORKER%`, `%DATA%` and `%PACK%` with the served file names |
| `dist/app.js` | the page script (`src/main.ts` and everything it imports) |
| `dist/worker.js` | the mesher worker (`src/mesher.worker.ts`) |
| `dist/images.json` | a JSON array of the entity, HUD, sky and particle textures the server must put in the pack (from `src/images.ts`) |

Some model tests check against the real 1.8 assets only when they are given:
`VIEWER_TEXTURES=<…/assets/minecraft/textures> VIEWER_DATA=<data.json> npm test`.

## Camera modes

- **1st person** (`fp`): the bot's own eyes, its hand and held item, view bobbing, the crosshair.
- **3rd person** (`tp`): orbits the bot. Drag to turn, wheel or pinch to zoom, double tap to get behind it again.
- **Free** (`free`): drag to look, WASD / arrow keys to move, Space / Shift up and down (left Ctrl + left Shift: 40 instead of 12 blocks a second, while sinking);
  on a touch screen two fingers move and pinch flies forward. Tapping Free again returns to the bot's view.

The ☀ chip cycles the brightness like the game's gamma setting (Moody, 50%, Bright); `i` shows frame rate,
latency, bandwidth and mesher statistics. Both are remembered per browser.

## URL parameters

| Parameter | Effect |
|---|---|
| `mode=fp\|tp\|free` | the camera mode to start in (default `fp`) |
| `cam=x,y,z,yaw,pitch` | a fixed free camera (screenshots, tests); yaw/pitch in radians, yaw 0 looks to −z |
| `replay=<name>` | watch a recording instead of the live bot (see below) |
| `t=<ms>`, `speed=<x>` | with `replay`: start position and playback speed |
| `embed=1` | for a host page that shows the viewer in a frame and has its own first/third person controls: only the Free chip is shown (the host switches modes by reloading the frame with another `mode`) |
| `debug=1` | open the statistics overlay |
| `fx=0\|1\|2` | particles off / reduced / all (default: all, reduced on touch devices; slow devices turn it down) |
| `aa=0` | no antialiasing |
| `skip=solid,cutout,translucent` | leave render passes out (debugging) |

For automated checks the page exposes `window.__viewerStats()` (frame and latency statistics, terrain and
mesher counts), `window.__viewerReset()` and `window.__viewerEntities()`.

## Recordings and replay

When the bot application records (the server writes recordings; when it records is the application's choice),
the **Recordings** chip lists them (`GET replays`, newest first). Picking one reloads the page with
`?replay=<name>`; the same renderer then plays the recording on its own WebSocket. The bar at the bottom plays
and pauses, seeks with the slider and switches the speed (1×, 2×, 4×, 8×, 16×); the server reports the
position a few times a second. **Live** goes back to the live view. A replay can be linked directly with
`?replay=<name>&t=<ms>&speed=<x>`.

## Browser support

WebGL2 and `DecompressionStream` are required: current Chrome, Edge, Firefox and Safari (iOS 16.4 or later).

## Source layout

| | |
|---|---|
| `src/main.ts` | wiring: socket, worker, renderers, camera, HUD, the render loop |
| `src/net.ts`, `src/track.ts` | the WebSocket (clock sync, flow-control ACKs, reconnect) and pose interpolation |
| `src/mesher.worker.ts`, `src/mesher.ts`, `src/bake.ts` | chunk columns → greedy-meshed sections with ambient occlusion and light |
| `src/terrain.ts`, `src/gl.ts`, `src/textures.ts` | terrain passes, shaders, the texture array |
| `src/entities.ts`, `src/mobs.ts`, `src/objects.ts`, `src/armour.ts`, `src/skins.ts`, `src/anim.ts` | entity models, poses and animation as the 1.8 client does them |
| `src/particles.ts`, `src/fxbox.ts`, `src/weather.ts`, `src/lightning.ts` | particles, rain and snow, lightning |
| `src/hud.ts`, `src/camera.ts`, `src/replay.ts` | the hotbar, health and food, camera modes, the replay controls |
| `src/shared/` | the wire protocol, 1.8 block state decoding and the data file types |

Comments name vanilla classes and methods with Legacy Yarn / Yarn names.
