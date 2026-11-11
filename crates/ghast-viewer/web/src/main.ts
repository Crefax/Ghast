/// <reference lib="dom" />
/**
 * The viewer page. Wires the socket (net.ts), the mesher worker, the terrain and entity
 * renderers, the camera and the HUD. Everything here only draws: the page never sends input to the bot.
 *
 * Latency probe: every POSE carries the server's clock at the tick; the first frame drawn after it
 * arrives records (estimated server time at that frame − tick time). The overlay shows its average
 * and p95, the network RTT, the pose buffer delay, fps and bandwidth.
 */
import { decode, NEEDS_NEIGHBOURS } from './shared/blocks18.js';
import type { ModelData } from './shared/model-data.js';
import { EFLAG, EVENT, KIND, LOOK, POSE, S, SELF, SKIN_FLAG, Reader, readExplosion, readJson, readLooks, readParticles, type Looks, type Tile } from './shared/protocol.js';
import {
  HURT_TICKS, PICKUP_TICKS, DEATH_TICKS, advance, advanceFirstPerson, bobMatrix, firstPerson, fpArmMatrix, fpItemMatrix, lidDegrees, lidHinge, living, livingAt, mountOffset,
  orbColor, orbFrame, pickupAt, rotate, startSwing, stepFirstPerson, stepLid, translate, useAction, type Living,
} from './anim.js';
import { headBlock, headBlockPlace } from './armour.js';
import { minecartBlock } from './objects.js';
import { bake, type Baked } from './bake.js';
import { CameraControl, type Mode, type View } from './camera.js';
import { EntityRenderer, type EntityState } from './entities.js';
import { GAMMA_STEPS, QuadIndex, SKY_FS, SKY_VS, compile, uniforms } from './gl.js';
import { Hud, type Slot } from './hud.js';
import { type EntityText, type Items, entityTag, gameLine } from './itemtext.js';
import { Lightning } from './lightning.js';
import { boxVisible, clamp, frustum, mat4, modelMatrix, multiply, perspective, viewFromYawPitch } from './math.js';
import { Net, clientNow } from './net.js';
import { ParticleRenderer, ParticleSystem } from './particles.js';
import { Terrain, type FrameParams } from './terrain.js';
import { biomeColors, loadPack, texture2d, type Pack } from './textures.js';
import { Track } from './track.js';
import { precipitation, rainDaylight, rainSky, weatherQuads, type Climate } from './weather.js';
import { ReplayUi } from './replay.js';

declare global { interface Window { __VIEWER: { data: string; pack: string; worker: string }; __viewerStats?: () => unknown } }

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const params = new URLSearchParams(location.search);
const initialMode: Mode = params.get('mode') === 'tp' ? 'tp' : params.get('mode') === 'free' ? 'free' : 'fp';

const stateBox = $('state'), info = $('info'), debugBox = $('debug'), statusLine = $('status');
const LOADING_WORLD = 'Loading the world…', CONNECTING = 'Connecting…', NOT_IN_GAME = 'The bot is not in game';
function show(text: string): void { stateBox.textContent = text; stateBox.hidden = !text; }

async function main(): Promise<void> {
  const canvas = $<HTMLCanvasElement>('view');
  const maybeGl = canvas.getContext('webgl2', { antialias: params.get('aa') !== '0', alpha: false, powerPreference: 'high-performance', preserveDrawingBuffer: false });
  if (!maybeGl) { show('This browser does not support WebGL2'); return; }
  if (!('DecompressionStream' in window)) { show('This browser is too old (on iPhone: iOS 16.4 or later is needed)'); return; }
  const gl: WebGL2RenderingContext = maybeGl;
  show('Loading textures…');
  const [data, pack] = await (async () => {
    const d = await (await fetch(window.__VIEWER.data)).json() as ModelData;
    const p = await loadPack(gl, d, window.__VIEWER.pack, (f) => show(`Loading textures… ${Math.round(f * 100)}%`));
    return [d, p] as [ModelData, Pack];
  })();
  const colors = biomeColors(data, pack.images);

  const worker = new Worker(window.__VIEWER.worker);
  worker.postMessage({ type: 'init', data, layerAlpha: pack.alpha, colors, base: pack.base });

  const index = new QuadIndex(gl);
  const terrain = new Terrain(gl, index, pack.array);
  terrain.skip = new Set((params.get('skip') ?? '').split(',').filter(Boolean));
  const ents = new EntityRenderer(gl, pack.images, pack.array);
  const skyProg = compile(gl, SKY_VS, SKY_FS);
  const skyU = uniforms(gl, skyProg, ['u_invViewProj', 'u_skyTop', 'u_fogColor', 'u_sunDir', 'u_daylight']);
  const skyVao = gl.createVertexArray()!;
  {
    gl.bindVertexArray(skyVao);
    const b = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, b);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);
  }

  // clouds: 1.8's clouds.png, one pixel = 12 blocks, at y 128, drifting east
  const cloudImg = pack.images.get('environment/clouds');
  let cloudTex: WebGLTexture | null = null;
  if (cloudImg) {
    cloudTex = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, cloudTex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, cloudImg);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.REPEAT);
  }

  // ---- block / item models on the main thread (dropped items, held item, HUD icons) ----
  const names = { name: (id: number) => data.blocks[id]?.name };
  const facts = { solid: (id: number) => !!data.blocks[id]?.solidHint };
  const bakedFor = new Map<string, Baked | null>();
  function blockBaked(id: number, meta: number): Baked | null {
    const k = `${id}:${meta}`;
    if (bakedFor.has(k)) return bakedFor.get(k)!;
    const b = data.blocks[id];
    let out: Baked | null = null;
    if (b && !b.fluid) {
      // items keep their variant in the damage value; slabs/logs etc. decode like placed blocks
      const key = decode(b.name, meta & 15, () => 0, names, facts);
      const refs = b.variants[key] ?? b.variants.normal ?? Object.values(b.variants)[0];
      if (refs?.length) out = bake(data.models[refs[0].m], { x: 0, y: 0, uvlock: false });
    }
    bakedFor.set(k, out);
    return out;
  }
  const itemLayer = (id: number, meta: number): number | null => {
    const it = (data.items as any)[`${id}:${meta}`] ?? data.items[id];
    return it ? it.layer : null;
  };
  const hud = new Hud($('hud'), pack.images, (id, meta) => {
    if (id >= 256) { const l = itemLayer(id, meta); return l === null ? null : { flat: pack.icons[l] }; }
    const baked = blockBaked(id, meta);
    if (!baked || !baked.quads.length) return null;
    const faceOf = (dir: number) => baked.quads.find(q => q.dir === dir) ?? null;
    if (baked.fullCube || baked.quads.length >= 6 && !NEEDS_NEIGHBOURS.has(data.blocks[id]?.name ?? '')) {
      const top = faceOf(1), south = faceOf(3), east = faceOf(5);
      const tinted = (q: typeof top) => (q && q.tint >= 0 ? 0x79c05a : undefined);
      // grass: the top is tinted, the sides carry a tinted overlay we leave out in the icon
      return { cube: { top: top ? pack.icons[top.layer] : null, left: south ? pack.icons[south.layer] : null, right: east ? pack.icons[east.layer] : null, tintTop: tinted(top), tintSide: undefined } };
    }
    return { flat: pack.icons[baked.quads[0].layer] };
  });
  hud.textTarget = $('items');

  // ---- particles and lightning ----
  const isTouch = matchMedia('(pointer: coarse)').matches;
  /** Particle texture slice and whether it takes the biome colour, by block state. */
  const particleTex = new Map<number, { layer: number; tint: 0 | 1 | 2 } | null>();
  const particles = new ParticleSystem(isTouch ? 1500 : 4000, {
    block(state, x, _y, z) {
      let t = particleTex.get(state);
      if (t === undefined) {
        t = null;
        const b = data.blocks[state >> 4];
        if (b?.fluid) t = { layer: pack.base[data.layers.findIndex(l => l.name === `blocks/${b.fluid}_still`)] ?? 0, tint: 0 };
        else if (b) {
          const key = decode(b.name, state & 15, () => 0, names, facts);
          const refs = b.variants[key] ?? b.variants.normal ?? Object.values(b.variants)[0];
          const model = refs?.length ? data.models[refs[0].m] : null;
          const faces = model ? model.el.flatMap(e => Object.values(e.faces)) : [];
          const layer = model?.p ?? faces[0]?.t;
          // BlockDustParticle takes the block's colour multiplier (leaves, tall grass...), the grass block excepted
          const tinted = state >> 4 !== 2 && faces.some(f => f?.t === layer && (f?.tint ?? -1) >= 0);
          if (layer !== undefined) t = { layer: pack.base[layer], tint: !tinted ? 0 : [18, 161, 106].includes(state >> 4) ? 2 : 1 };
        }
        particleTex.set(state, t);
      }
      if (!t) return null;
      if (!t.tint) return { layer: t.layer, color: 0xffffff };
      const col = heights.get(`${Math.floor(x / 16)},${Math.floor(z / 16)}`);
      const biome = col ? col.biomes[(x & 15) + 16 * (z & 15)] : 1;
      return { layer: t.layer, color: (t.tint === 2 ? colors.foliage : colors.grass)[biome] };
    },
    item(id, meta) {
      if (id < 256) return this.block((id << 4) | meta, 0, 0, 0)?.layer ?? null;
      const l = itemLayer(id, meta);
      return l === null ? null : pack.base[l];
    },
  });
  // ?fx=0 off, 1 decreased, 2 all; phones start decreased; slow devices turn it down (see the render loop)
  const fxParam = params.get('fx');
  particles.level = fxParam === null ? (isTouch ? 1 : 2) : Math.max(0, Math.min(2, Number(fxParam) || 0));
  const fxTexture = (name: string) => { const img = pack.images.get(name); return img ? texture2d(gl, img) : null; };
  const particleGl = new ParticleRenderer(gl, pack.array, fxTexture('particle/particles'), fxTexture('entity/explosion'));
  const lightning = new Lightning();
  /** ms of the 20 Hz particle clock not ticked yet. */
  let fxClock = 0;
  const fxStats = { drawn: 0 };

  // ---- state from the server ----
  const bot = new Track(100, 70);
  /** `life`: the per-tick animation state (anim.ts); `gone`: removed while dying (drawn until the fall ends); `vehicle`: ridden entity. */
  type Ent = { s: EntityState; track: Track; life: Living; flags: number; vehicle: number; gone?: boolean; collected?: boolean };
  const entities = new Map<number, Ent>();
  /** Items and orbs flying to their collector (COLLECT), 3 ticks each. */
  const pickups: Array<{ s: EntityState; from: [number, number, number]; to: number; at: number }> = [];
  /** The bot's own LOOKS (its armour in third person). */
  let selfLooks: Looks | null = null;
  let account = '', inGame = false, status = '', viewDistance = 8;
  /** 26.1: the header suffix (game version, assets) and the real entity text by id (`ENTITY`). */
  let game = '';
  /** 26.1: world y = page y + worldMinY (the page shows 256 rows of a taller world). */
  let worldMinY = 0;
  const entityText = new Map<number, EntityText>();
  let health = 20, food = 20;
  let time = { at: 0, tod: 6000 };
  let lastPoseFrame = -1, lastPoseT = 0;
  const lags: number[] = []; // arrival lag (server ms) of poses, for the buffer delay
  const latency: number[] = []; // tick → first frame
  let seenPoseVersion = 0;

  const camera = new CameraControl(canvas, initialMode);
  // ?replay=<name>: a recording instead of the live view (the server plays it back on the same socket)
  const replayName = params.get('replay');
  const replayQuery = replayName ? `&replay=${encodeURIComponent(replayName)}&t=${Number(params.get('t')) || 0}&speed=${Number(params.get('speed')) || 1}` : '';
  const net = new Net(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}${location.pathname.replace(/[^/]*$/, '')}ws?mode=${initialMode}${replayQuery}`,
    (type, r, frameAt) => onMessage(type, r, frameAt),
    (ok, why) => {
      if (ok) show(LOADING_WORLD);
      else { show(why ? `Connection closed: ${why}` : 'Connection lost, reconnecting…'); }
    });

  /** Sign text by column; textures by text. */
  const signs = new Map<string, Array<{ x: number; y: number; z: number; state: number; lines: string[] }>>();
  const signTex = new Map<string, WebGLTexture>();
  function signTexture(lines: string[]): WebGLTexture {
    const key = lines.join('\n');
    let t = signTex.get(key);
    if (t) return t;
    const c = document.createElement('canvas');
    c.width = 256; c.height = 128;
    const ctx = c.getContext('2d')!;
    ctx.fillStyle = '#000';
    ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
    lines.forEach((l, i) => {
      let size = 24;
      ctx.font = `600 ${size}px ui-monospace, Menlo, Consolas, monospace`;
      while (size > 12 && ctx.measureText(l).width > 240) { size -= 2; ctx.font = `600 ${size}px ui-monospace, Menlo, Consolas, monospace`; }
      ctx.fillText(l, 128, 18 + i * 30);
    });
    t = texture2d(gl, c);
    if (signTex.size > 256) signTex.clear();
    signTex.set(key, t);
    return t;
  }
  function drawSigns(cam: View): void {
    const m = mat4(), s = mat4();
    for (const list of signs.values()) for (const sg of list) {
      const rx = sg.x + 0.5 - cam.x, rz = sg.z + 0.5 - cam.z;
      if (rx * rx + rz * rz > 32 * 32) continue;
      const standing = sg.state >> 4 === 63;
      let yaw: number, cy: number, out: number;
      if (standing) { yaw = -((sg.state & 15) * Math.PI) / 8; cy = 13.33 / 16; out = 0.67 / 16 + 0.004; }
      else {
        const f = sg.state & 7; // 2 north, 3 south, 4 west, 5 east
        yaw = f === 2 ? Math.PI : f === 4 ? -Math.PI / 2 : f === 5 ? Math.PI / 2 : 0;
        cy = 8.33 / 16; out = -(6.67 / 16) + 0.004;
      }
      // the front faces along (sin yaw, cos yaw); the text sits just in front of the board
      const nx = Math.sin(yaw), nz = Math.cos(yaw);
      modelMatrix(m, rx + nx * out, sg.y + cy - cam.y, rz + nz * out, yaw, 0, 1);
      s[0] = 0.94; s[5] = 0.47;
      multiply(m, m, s);
      ents.drawSignText(signTexture(sg.lines), m, 1);
    }
  }

  /** Skulls and banners by column (the worker matched them to their blocks). */
  const tiles = new Map<string, Array<Tile & { state: number }>>();
  function drawTiles(cam: View, serverNow: number): void {
    const ticks = serverNow / 50;
    for (const list of tiles.values()) for (const t of list) {
      const rx = t.x - cam.x, ry = t.y - cam.y, rz = t.z - cam.z;
      if (rx * rx + rz * rz > 48 * 48) continue;
      if (t.kind === 'skull') ents.drawSkull(t.skull, t.state & 15, t.rot, rx, ry, rz, t.skin);
      else if (t.kind === 'banner') ents.drawBanner(t.state >> 4 === 176, t.state & 15, t.base, t.patterns, rx, ry, rz, t.x * 7 + t.y * 9 + t.z * 13 + ticks);
    }
  }

  function clearWorld(): void {
    signs.clear();
    lids.clear();
    tiles.clear();
    heights.clear();
    cracks.length = 0;
    particles.clear();
    lightning.clear();
    worker.postMessage({ type: 'reset' });
    terrain.clear();
    entities.clear();
    entityText.clear();
    bot.clear();
  }

  function onMessage(type: number, r: Reader, frameAt: number): void {
    switch (type) {
      case S.HELLO: { const h = readJson(r); account = h.account; viewDistance = h.viewDistance ?? 8; game = gameLine(h); worldMinY = h.worldMinY ?? 0; hud.setItemText(null); entityText.clear(); break; }
      case S.ITEMS: hud.setItemText(readJson<Items>(r)); break;
      case S.ENTITY: { const t = readJson<EntityText>(r); entityText.set(t.id, t); break; }
      case S.STATE: {
        const st = readJson(r);
        const was = inGame;
        inGame = !!st.inGame; status = st.status ?? ''; account = st.account || account;
        if (st.game) { game = gameLine(st); worldMinY = st.worldMinY ?? worldMinY; }
        if (!inGame) { show(st.message || NOT_IN_GAME); clearWorld(); }
        // back in game: the not-in-game message (any text the server sent) or a connection notice gives way
        else if (!was || stateBox.textContent === CONNECTING || stateBox.textContent?.startsWith('Connection')) show(LOADING_WORLD);
        statusLine.textContent = status;
        break;
      }
      case S.RESET: clearWorld(); break;
      case S.REPLAY: { const cursor = r.f64(), duration = r.f64(), speed = r.f32(), start = r.f64(); replayUi.update(cursor, duration, speed, start); break; }
      case S.CHUNK: {
        const cx = r.i32(), cz = r.i32(), deflated = r.u8() === 1;
        const body = r.rest().slice().buffer;
        worker.postMessage({ type: 'chunk', cx, cz, deflated, body }, [body]);
        break;
      }
      case S.UNLOAD: { const cx = r.i32(), cz = r.i32(); worker.postMessage({ type: 'unload', cx, cz }); break; }
      case S.BLOCKS: {
        const n = r.u16();
        const list: number[] = [];
        for (let i = 0; i < n; i++) list.push(r.i32(), r.u8(), r.i32(), r.u16());
        worker.postMessage({ type: 'blocks', list });
        break;
      }
      case S.POSE: {
        const t = r.f64(); r.u32();
        const x = r.f64(), y = r.f64(), z = r.f64(), yaw = r.f32(), pitch = r.f32(), flags = r.u8();
        bot.push({ t, x, y, z, yaw, pitch, headYaw: yaw, flags });
        if (flags & POSE.SWING) startSwing(selfLife);
        lastPoseFrame = frameAt; lastPoseT = t;
        lags.push(net.serverNow(frameAt) - t);
        if (lags.length > 40) lags.shift();
        if (stateBox.textContent === LOADING_WORLD && terrain.count > 0) show('');
        break;
      }
      case S.SPAWN: {
        const id = r.u32(), kind = r.u8(), name = r.str(), label = r.str();
        const x = r.f64(), y = r.f64(), z = r.f64(), yaw = r.f32(), pitch = r.f32(), headYaw = r.f32(), width = r.f32(), height = r.f32();
        const item = r.u16(), meta = r.u16();
        const variant = r.pos + 2 <= r.end ? r.u16() : 0;
        const track = new Track(150, 0);
        const t = net.serverNow();
        track.push({ t, x, y, z, yaw, pitch, headYaw, flags: 0 });
        // a re-spawn (a new variant) keeps the animation state
        const old = entities.get(id);
        entities.set(id, { s: { id, kind, name, label, width, height, item, meta, variant, x, y, z, yaw, pitch, headYaw, swing: 0, swingAmt: 0, px: x, pz: z, bodyYaw: yaw, spin: Math.random() * 6 }, track,
          life: old?.life ?? living(x, z, yaw), flags: old?.flags ?? 0, vehicle: old?.vehicle ?? 0 });
        break;
      }
      case S.MOVES: {
        const n = r.u16();
        const t = lastPoseFrame === frameAt ? lastPoseT : net.serverNow();
        for (let i = 0; i < n; i++) {
          const id = r.u32(), x = r.f64(), y = r.f64(), z = r.f64(), yaw = r.f32(), pitch = r.f32(), headYaw = r.f32();
          entities.get(id)?.track.push({ t, x, y, z, yaw, pitch, headYaw, flags: 0 });
        }
        break;
      }
      case S.REMOVE: {
        // a dying entity finishes its fall first (the server removes it at deathTime 20, give or take a tick)
        const n = r.u16();
        for (let i = 0; i < n; i++) { const id = r.u32(), e = entities.get(id); entityText.delete(id); if (e && e.life.death > 0 && e.life.death < DEATH_TICKS) e.gone = true; else entities.delete(id); }
        break;
      }
      case S.EVENTS: {
        const n = r.u16();
        for (let i = 0; i < n; i++) {
          const id = r.u32(), ev = r.u8();
          const life = id === SELF ? selfLife : entities.get(id)?.life;
          if (!life) continue;
          if (ev === EVENT.SWING) startSwing(life);
          else if (ev === EVENT.HURT) { life.hurt = HURT_TICKS; if (id === SELF) fpView.hurt = HURT_TICKS; }
          else if (ev === EVENT.DEATH && !life.death) life.death = 1;
        }
        break;
      }
      case S.ESTATE: {
        const n = r.u16();
        for (let i = 0; i < n; i++) { const id = r.u32(), flags = r.u8(), vehicle = r.u32(); const e = entities.get(id); if (e) { e.flags = flags; e.vehicle = vehicle; } }
        break;
      }
      case S.CHESTS: {
        const n = r.u16(), open: Array<[number, number, number, boolean]> = [];
        for (let i = 0; i < n; i++) {
          const x = r.i32(), y = r.u8(), z = r.i32(), viewers = r.u8(), k = `${x},${y},${z}`;
          let l = lids.get(k);
          if (!l && viewers) { l = { x, y, z, viewers, angle: 0, prev: 0, state: 0, mesh: null }; lids.set(k, l); open.push([x, y, z, true]); }
          if (l) l.viewers = viewers;
        }
        // the worker leaves the lid out of the terrain from now on and posts its quads
        if (open.length) worker.postMessage({ type: 'lids', list: open });
        break;
      }
      case S.SKIN: {
        const id = r.u32(), flags = r.u8(), skin = r.str(), cape = r.str();
        const s = id === SELF ? selfState : entities.get(id)?.s;
        if (s) { s.skin = skin; s.cape = cape; s.slim = (flags & SKIN_FLAG.SLIM) !== 0; }
        break;
      }
      case S.COLLECT: {
        const id = r.u32(), to = r.u32(), e = entities.get(id);
        if (e && !e.collected) { e.collected = true; pickups.push({ s: { ...e.s }, from: [e.s.x, e.s.y, e.s.z], to, at: performance.now() }); }
        break;
      }
      case S.EQUIP: { const id = r.u32(), item = r.u16(), meta = r.u16(); const e = entities.get(id); if (e) { e.s.item = item; e.s.meta = meta; } break; }
      case S.LOOKS: {
        const l = readLooks(r);
        if (l.id === SELF) { selfLooks = l; break; }
        const e = entities.get(l.id)?.s;
        if (e) { e.baby = (l.flags & LOOK.BABY) !== 0; e.armour = l.armour; e.pose = l.pose; }
        break;
      }
      case S.INVENTORY: {
        const held = r.u8();
        const slots: Slot[] = [];
        for (let i = 0; i < 9; i++) slots.push({ id: r.u16(), meta: r.u16(), count: r.u8() });
        hud.setInventory(held, slots);
        heldItem = slots[held] ?? null;
        break;
      }
      case S.VITALS: {
        health = r.f32(); food = r.u8();
        const saturation = r.f32(), level = r.u8(), progress = r.f32();
        hud.setVitals({ health, food, saturation, level, progress });
        break;
      }
      case S.TIME: { const at = r.f64(); r.f64(); const tod = r.f64(); time = { at, tod }; break; }
      case S.WEATHER: { rain = r.f32(); thunder = r.f32(); break; }
      case S.PARTICLES: for (const m of readParticles(r)) particles.fromServer(m); break;
      case S.EXPLOSION: particles.explosion(readExplosion(r)); break;
      case S.LIGHTNING: lightning.add(r.f64(), r.f64(), r.f64()); break;
      case S.CRACKS: {
        cracks.length = 0;
        const n = r.u16();
        for (let i = 0; i < n; i++) cracks.push({ x: r.i32(), y: r.u8(), z: r.i32(), state: r.u16(), stage: r.u8() });
        break;
      }
    }
  }
  let heldItem: Slot | null = null;
  /** Rain and thunder strength (WEATHER), eased towards so a change fades like vanilla's. */
  let rain = 0, thunder = 0, rainShown = 0, thunderShown = 0;
  const heights = new Map<string, { top: Uint8Array; biomes: Uint8Array }>();
  const climate: Climate = new Map(data.biomes.map(([id, t, r]) => [id, [t, r] as [number, number]]));
  const weatherTex = (name: string): WebGLTexture | null => {
    const img = pack.images.get(name);
    if (!img) return null;
    const t = texture2d(gl, img);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.REPEAT);
    return t;
  };
  const rainTex = weatherTex('environment/rain'), snowTex = weatherTex('environment/snow');
  const WEATHER_R = 10;
  const rainData = new Float32Array((2 * WEATHER_R + 1) ** 2 * 6 * 7), snowData = new Float32Array(rainData.length);
  type Dyn = { vao: WebGLVertexArrayObject; buf: WebGLBuffer };
  let rainVao: Dyn | null = null, snowVao: Dyn | null = null;
  function drawWeather(cam: View, ticks: number, daylight: number, fog: [number, number]): void {
    const n = weatherQuads(rainData, snowData, heights, climate, cam, WEATHER_R, ticks);
    const b = Math.max(0.25, daylight);
    for (const [count, buf, tex, alpha] of [[n.rain, rainData, rainTex, 0.75], [n.snow, snowData, snowTex, 0.9]] as const) {
      if (!count || !tex) continue;
      const v = buf === rainData ? (rainVao ??= ents.dynamic()) : (snowVao ??= ents.dynamic());
      gl.bindBuffer(gl.ARRAY_BUFFER, v.buf);
      gl.bufferData(gl.ARRAY_BUFFER, buf.subarray(0, count), gl.DYNAMIC_DRAW);
      ents.drawTextured(v.vao, count / 7, tex, [b, b, b, alpha * rainShown], fog);
    }
  }
  /** Blocks being broken (CRACKS): drawn with destroy_stage_<stage> over the block's model. */
  const cracks: Array<{ x: number; y: number; z: number; state: number; stage: number }> = [];
  const crackLayers = Array.from({ length: 10 }, (_, i) => data.layers.findIndex(l => l.name === `blocks/destroy_stage_${i}`));
  function drawCracks(cam: View): void {
    for (const c of cracks) {
      const layer = crackLayers[c.stage];
      if (layer < 0) continue;
      const id = c.state >> 4, meta = c.state & 15;
      const baked = blockBaked(id, meta) ?? blockBaked(1, 0);
      const mesh = ents.crackMesh(`crack${c.stage}:${baked === blockBaked(1, 0) ? 'cube' : c.state}`, baked, pack.base[layer]);
      if (mesh) ents.drawCrack(mesh, c.x - cam.x, c.y - cam.y, c.z - cam.z);
    }
  }
  /** Chest lids: block_action's viewers, ChestBlockEntity's animationAngle per tick, the lid's quads from the worker. */
  const lids = new Map<string, { x: number; y: number; z: number; viewers: number; angle: number; prev: number; state: number; mesh: { vao: WebGLVertexArrayObject; count: number } | null }>();
  let lidAcc = 0;
  function drawLids(cam: View, dt: number): void {
    lidAcc = Math.min(500, lidAcc + dt * 1000);
    const closed: Array<[number, number, number, boolean]> = [];
    for (; lidAcc >= 50; lidAcc -= 50) for (const [k, l] of lids) {
      l.prev = l.angle; l.angle = stepLid(l.angle, l.viewers > 0);
      // shut: back into the terrain
      if (!l.viewers && !l.angle && !l.prev) { lids.delete(k); closed.push([l.x, l.y, l.z, false]); }
    }
    if (closed.length) worker.postMessage({ type: 'lids', list: closed });
    const m = mat4();
    for (const l of lids.values()) {
      if (!l.mesh) continue;
      const h = lidHinge(l.state & 15);
      m.set(mat4());
      translate(m, l.x - cam.x + 0.5 + h.x, l.y - cam.y + 9 / 16, l.z - cam.z + 0.5 + h.z);
      rotate(m, h.sign * lidDegrees(l.prev, l.angle, lidAcc / 50), h.ax, 0, h.az);
      translate(m, -h.x, -9 / 16, -h.z);
      ents.drawItemMatrix(m, 0, l.mesh);
    }
  }

  /** The bot's own living state (arm swing, hurt) and first-person view state (bobbing, hand), stepped per tick. */
  const selfLife = living(0, 0, 0);
  const fpView = firstPerson();

  worker.onmessage = (e) => {
    const m = e.data;
    if (m.type === 'mesh') terrain.upload(m.cx, m.sy, m.cz, [m.solid, m.cutout, m.translucent]);
    else if (m.type === 'drop') { terrain.drop(m.cx, m.cz); heights.delete(`${m.cx},${m.cz}`); }
    else if (m.type === 'signs') { if (m.signs.length) signs.set(`${m.cx},${m.cz}`, m.signs); else signs.delete(`${m.cx},${m.cz}`); }
    else if (m.type === 'heights') heights.set(`${m.cx},${m.cz}`, { top: m.top, biomes: m.biomes });
    else if (m.type === 'tiles') { if (m.tiles.length) tiles.set(`${m.cx},${m.cz}`, m.tiles); else tiles.delete(`${m.cx},${m.cz}`); }
    else if (m.type === 'fxbox') { particles.box.set(m.ox, m.oy, m.oz, m.light, m.shape); particles.emitters = m.emitters; }
    else if (m.type === 'broken') { const l: number[] = m.list; for (let i = 0; i < l.length; i += 4) particles.breakBlock(l[i], l[i + 1], l[i + 2], l[i + 3]); }
    else if (m.type === 'stats') workerStats = m;
    else if (m.type === 'lid') {
      const l = lids.get(`${m.x},${m.y},${m.z}`);
      if (l && m.quads?.length) { l.state = m.state; l.mesh = ents.blockMesh(`lid${m.x},${m.y},${m.z}:${m.state}:${m.quads.length}:${m.quads[0].pos[0]}`, { quads: m.quads, ao: false, fullCube: false }, pack.base); }
      else if (l) l.mesh = null;
    }
    else if (m.type === 'error') console.error('mesher:', m.message);
  };
  let workerStats: { meshed: number; meshMs: number; columns: number } = { meshed: 0, meshMs: 0, columns: 0 };
  worker.postMessage({ type: 'fx', on: particles.level > 0 });

  const replayUi = new ReplayUi(b => net.send(b), replayName);

  // ---- UI: modes and the debug overlay ----
  // ?embed=1: the page sits in a host page's frame that has its own first/third person controls (it reloads the
  // frame with another ?mode=); only the free camera chip stays
  if (params.get('embed') === '1') document.body.classList.add('embedded');
  const modeButtons = document.querySelectorAll<HTMLButtonElement>('[data-mode]');
  const markMode = () => modeButtons.forEach(b => b.classList.toggle('on', b.dataset.mode === camera.mode));
  let lastView: View = { x: 0, y: 80, z: 0, yaw: 0, pitch: 0 };
  modeButtons.forEach(b => b.addEventListener('click', () => {
    const m = b.dataset.mode as Mode;
    // a second tap on Free goes back to the bot's view (the only way back when embedded)
    camera.setMode(m === 'free' && camera.mode === 'free' ? (initialMode === 'free' ? 'fp' : initialMode) : m, lastView);
    markMode();
  }));
  camera.onModeChange = () => { markMode(); $('cross').hidden = camera.mode !== 'fp'; };
  // ?cam=x,y,z,yaw,pitch: a fixed free camera (screenshots, tests)
  const fixed = params.get('cam')?.split(',').map(Number);
  if (fixed?.length === 5 && fixed.every(Number.isFinite)) camera.setMode('free', { x: fixed[0], y: fixed[1], z: fixed[2], yaw: fixed[3], pitch: fixed[4] });
  markMode();
  $('cross').hidden = camera.mode !== 'fp';
  let showDebug = params.get('debug') === '1';
  try { if (localStorage.getItem('viewer-debug') === '1') showDebug = true; } catch { /* private mode */ }
  debugBox.hidden = !showDebug;
  $('dbg').addEventListener('click', () => {
    showDebug = !showDebug; debugBox.hidden = !showDebug;
    try { localStorage.setItem('viewer-debug', showDebug ? '1' : '0'); } catch { /* private mode */ }
  });

  // brightness chip: vanilla's gamma setting, remembered per browser
  let gammaStep = 0;
  try { gammaStep = Math.max(0, Math.min(GAMMA_STEPS.length - 1, Number(localStorage.getItem('viewer-gamma') ?? 0) || 0)); } catch { /* private mode */ }
  const brightButton = $('bright');
  // the label hides on a narrow phone unless embedded (the mode chips need the room); the title says it
  const markBright = () => {
    $('bright-label').textContent = ` ${GAMMA_STEPS[gammaStep].label}`;
    brightButton.title = `Brightness: ${GAMMA_STEPS[gammaStep].label} (the game's gamma setting; tap to change)`;
    ents.gamma = GAMMA_STEPS[gammaStep].gamma;
  };
  markBright();
  brightButton.addEventListener('click', () => {
    gammaStep = (gammaStep + 1) % GAMMA_STEPS.length;
    markBright();
    try { localStorage.setItem('viewer-gamma', String(gammaStep)); } catch { /* private mode */ }
  });

  // ---- render loop ----
  let resScale = 1;
  const dprCap = isTouch ? 2 : 2;
  const resize = () => {
    const dpr = Math.min(window.devicePixelRatio || 1, dprCap) * resScale;
    const w = Math.max(1, Math.round(canvas.clientWidth * dpr)), h = Math.max(1, Math.round(canvas.clientHeight * dpr));
    if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
  };
  const proj = mat4(), view = mat4(), viewProj = mat4(), inv = mat4(), bobM = mat4();
  let planes: Float32Array = new Float32Array(24);
  let lastFrame = performance.now();
  const frameTimes: number[] = [];
  let lastCenter = '';
  let lowSince = 0, highSince = 0, slowSince = 0;

  function daylightAt(serverNow: number): { daylight: number; angle: number } {
    const tod = time.tod >= 0 ? time.tod + (serverNow - time.at) / 50 : -time.tod;
    let f = (((tod % 24000) + 24000) % 24000) / 24000 - 0.25;
    if (f < 0) f += 1;
    if (f > 1) f -= 1;
    const f2 = f;
    f = 1 - (Math.cos(f * Math.PI) + 1) / 2;
    f = f2 + (f - f2) / 3;
    const b = clamp(Math.cos(f * Math.PI * 2) * 2 + 0.5, 0, 1);
    return { daylight: 0.2 + 0.8 * b, angle: f };
  }

  const jsTimes: number[] = [];
  const prof = { opaque: 0, translucent: 0, frames: 0 };
  function frame(now: number): void {
    requestAnimationFrame(frame);
    const js0 = performance.now();
    prof.frames++;
    try { draw(now); } finally { jsTimes.push(performance.now() - js0); if (jsTimes.length > 120) jsTimes.shift(); }
  }

  function draw(now: number): void {
    const dt = Math.min(0.1, (now - lastFrame) / 1000);
    frameTimes.push(now - lastFrame);
    if (frameTimes.length > 120) frameTimes.shift();
    lastFrame = now;
    resize();
    hud.draw();

    const serverNow = net.serverNow();
    // pose buffer delay: the 90th percentile of how late poses arrive, a few ms of margin
    const sortedLags = [...lags].sort((a, b) => a - b);
    const lagP90 = sortedLags.length ? sortedLags[Math.floor(sortedLags.length * 0.8)] : 20;
    const delay = clamp(lagP90 + 6, 4, 120);
    const p = bot.at(serverNow - delay, now);
    if (bot.version !== seenPoseVersion) {
      const newest = bot.newest();
      if (newest) { latency.push(serverNow - newest.t); if (latency.length > 120) latency.shift(); }
      seenPoseVersion = bot.version;
    }

    // rain fades in and out over a few seconds (the server's level steps are 0.01 a tick)
    rainShown += (rain - rainShown) * Math.min(1, dt * 0.5); thunderShown += (thunder - thunderShown) * Math.min(1, dt * 0.5);
    const sun = daylightAt(serverNow);
    const angle = sun.angle;
    let daylight = 0.2 + (sun.daylight - 0.2) * rainDaylight(rainShown, thunderShown);
    const sunDir: [number, number, number] = [-Math.sin(angle * Math.PI * 2) * 0, Math.cos(angle * Math.PI * 2), -Math.sin(angle * Math.PI * 2)];
    // vanilla-ish sky (plains) and fog colours, darkened by the night, grey under rain
    const skyTop = rainSky([0.47 * daylight * daylight, 0.65 * daylight * daylight, 1.0 * daylight], rainShown, thunderShown);
    const fogColor = rainSky([0.75 * daylight, 0.85 * daylight, 1.0 * daylight], rainShown * 0.67, thunderShown * 0.67);
    // a lightning strike: the sky towards (0.8, 0.8, 1) and full daylight in the lightmap (World.method_3631, updateLightmap)
    const flash = lightning.flash(fxClock / 50);
    if (flash > 0) {
      for (let i = 0; i < 3; i++) { const w = i === 2 ? 1 : 0.8; skyTop[i] = skyTop[i] * (1 - flash) + w * flash; fogColor[i] = fogColor[i] * (1 - flash / 2) + w * flash / 2; }
      daylight = 1;
    }
    const fogEnd = Math.max(32, (viewDistance - 1) * 16), fog: [number, number] = [fogEnd * 0.6, fogEnd];

    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(fogColor[0], fogColor[1], fogColor[2], 1);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);

    let eye: View | null = null;
    // the bot's own per-tick state: arm swing and hurt (third person), bobbing, hand lag and equip dip (first person)
    let selfPt = 0, fpt = 0;
    if (p && inGame) {
      const sneak = (p.flags & POSE.SNEAK) !== 0;
      eye = { x: p.x, y: p.y + (sneak ? 1.54 : 1.62), z: p.z, yaw: p.yaw, pitch: p.pitch };
      const held = heldItem?.id ? `${heldItem.id}:${heldItem.meta}:${heldItem.count}` : '';
      const using = (p.flags & POSE.USE) !== 0 && !!heldItem?.id;
      selfPt = advance(selfLife, dt * 1000, p.x, p.z, p.yaw);
      fpt = advanceFirstPerson(fpView, dt * 1000, () => stepFirstPerson(fpView, p.x, p.y, p.z, (p.flags & POSE.GROUND) !== 0, p.yaw, p.pitch, held, using));
    }
    const cam = camera.update(dt, eye ?? lastView);
    lastView = cam;
    const aspect = canvas.width / canvas.height;
    perspective(proj, (70 * Math.PI) / 180, aspect, 0.05, 512);
    viewFromYawPitch(view, cam.yaw, cam.pitch);
    // first person: vanilla's view bobbing and hurt tilt in front of the camera's rotation
    const bob = camera.mode === 'fp' && eye ? bobMatrix(bobM, fpView, fpt) : null;
    if (bob) multiply(view, bob, view);
    multiply(viewProj, proj, view);
    planes = frustum(viewProj, planes);
    invert(inv, viewProj);

    // sky
    gl.disable(gl.DEPTH_TEST);
    gl.useProgram(skyProg);
    gl.uniformMatrix4fv(skyU.u_invViewProj, false, inv);
    gl.uniform3f(skyU.u_skyTop, skyTop[0], skyTop[1], skyTop[2]);
    gl.uniform3f(skyU.u_fogColor, fogColor[0], fogColor[1], fogColor[2]);
    gl.uniform3f(skyU.u_sunDir, sunDir[0], sunDir[1], sunDir[2]);
    gl.uniform1f(skyU.u_daylight, daylight);
    gl.bindVertexArray(skyVao);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.bindVertexArray(null);
    gl.enable(gl.DEPTH_TEST);
    gl.depthFunc(gl.LEQUAL);

    if (inGame && eye) {
      const center = `${Math.floor(cam.x / 4)},${Math.floor(cam.y / 4)},${Math.floor(cam.z / 4)}`;
      if (center !== lastCenter) { lastCenter = center; worker.postMessage({ type: 'center', x: cam.x, y: cam.y, z: cam.z }); }
      const fp: FrameParams = { viewProj, planes, cam: [cam.x, cam.y, cam.z], fogColor, fog, daylight, ticks: serverNow / 50, gamma: ents.gamma };
      const tt0 = performance.now();
      const vis = terrain.drawOpaque(fp);
      prof.opaque += performance.now() - tt0;

      // entities (camera-relative): positions first (riders sit on their vehicle), then each one's tick state and its model
      ents.begin(viewProj, fogColor, fog, daylight);
      const entT = serverNow - delay - 100;
      const boxMesh = ents.blockMesh('box', blockBaked(35, 0), pack.base);
      for (const { s, track } of entities.values()) {
        const st = track.at(entT, now);
        if (!st) continue;
        s.x = st.x; s.y = st.y; s.z = st.z; s.yaw = st.yaw; s.pitch = st.pitch; s.headYaw = st.headYaw;
      }
      for (const e of entities.values()) {
        const v = e.vehicle ? entities.get(e.vehicle - 1) : undefined;
        if (v) { e.s.x = v.s.x; e.s.z = v.s.z; e.s.y = v.s.y + mountOffset(v.s.name, v.s.height, e.s.kind === KIND.PLAYER); }
      }
      for (const [id, e] of entities) {
        const { s, life } = e;
        const pt = advance(life, dt * 1000, s.x, s.z, s.yaw);
        if (e.gone && life.death >= DEATH_TICKS) { entities.delete(id); continue; }
        if (e.collected || e.flags & EFLAG.INVISIBLE) continue;
        const a = livingAt(life, pt);
        s.swing = a.limb; s.swingAmt = a.limbAmt; s.bodyYaw = a.body; s.arm = a.swing; s.hurt = a.hurt; s.death = a.death;
        s.sneak = (e.flags & EFLAG.SNEAK) !== 0; s.using = (e.flags & EFLAG.USE) !== 0; s.sleeping = (e.flags & EFLAG.SLEEP) !== 0; s.riding = e.vehicle > 0;
        const v = e.vehicle ? entities.get(e.vehicle - 1) : undefined;
        if (v) s.bodyYaw = v.s.bodyYaw;
        const rx = s.x - cam.x, ry = s.y - cam.y, rz = s.z - cam.z;
        if (rx * rx + rz * rz > fogEnd * fogEnd) continue;
        if (!boxVisible(planes, rx - 2, ry - 1, rz - 2, rx + 2, ry + 3, rz + 2)) continue;
        if (s.kind === KIND.ITEM) { drawDrop(s, rx, ry, rz, now, dt); continue; }
        if (s.kind === KIND.ORB) { ents.drawOrb(rx, ry, rz, orbFrame(s.meta), orbColor(a.age), cam.yaw, cam.pitch); continue; }
        // falling blocks and primed TNT: their block, full size
        if ((s.name === 'FallingSand' || s.name === 'PrimedTnt') && s.item) {
          const mesh = ents.blockMesh(`b${s.item}:${s.meta}`, blockBaked(s.item, s.meta), pack.base);
          if (mesh) { ents.drawItemMatrix(modelMatrix(mat4(), rx, ry, rz, 0, 0, 0.98), 0, mesh); continue; }
        }
        const hand = mat4(), head = mat4();
        const worn = headBlock(s.name, s.armour);
        if (!ents.drawLiving(s, rx, ry, rz, hand, serverNow / 50, worn ? head : undefined)) { ents.drawBox(s, rx, ry, rz, boxMesh); continue; }
        if (s.item) drawInHand(hand, s.item, s.meta);
        if (worn) drawOnHead(head, s.name, worn);
        const cart = minecartBlock(s.name, s.variant ?? 0);
        if (cart) {
          // MinecartEntityRenderer: the block 0.75 ×, centred, turned 90° from the cart
          const mesh = ents.blockMesh(`b${cart.id}:${cart.meta}`, blockBaked(cart.id, cart.meta), pack.base);
          if (mesh) ents.drawItemMatrix(modelMatrix(mat4(), rx, ry + cart.dy, rz, s.bodyYaw + Math.PI / 2, 0, cart.scale), 0, mesh);
        }
      }
      // collected items and orbs fly to the collector's feet + 0.5 (ItemPickupParticle)
      for (let i = pickups.length - 1; i >= 0; i--) {
        const pk = pickups[i], ticks = (now - pk.at) / 50;
        const to = pk.to === SELF ? p : entities.get(pk.to)?.s;
        if (ticks >= PICKUP_TICKS || !to) { pickups.splice(i, 1); continue; }
        const [x, y, z] = pickupAt(pk.from, [to.x, to.y, to.z], ticks);
        if (pk.s.kind === KIND.ORB) ents.drawOrb(x - cam.x, y - cam.y, z - cam.z, orbFrame(pk.s.meta), orbColor(serverNow / 50), cam.yaw, cam.pitch);
        else drawDrop(pk.s, x - cam.x, y - cam.y, z - cam.z, now, dt);
      }
      // the bot itself (third person and free camera)
      if (camera.mode !== 'fp' && p) {
        const a = livingAt(selfLife, selfPt);
        selfState.swing = a.limb; selfState.swingAmt = a.limbAmt; selfState.bodyYaw = a.body; selfState.arm = a.swing; selfState.hurt = a.hurt;
        selfState.x = p.x; selfState.y = p.y; selfState.z = p.z;
        selfState.headYaw = p.yaw; selfState.pitch = p.pitch;
        selfState.sneak = (p.flags & POSE.SNEAK) !== 0;
        selfState.using = (p.flags & POSE.USE) !== 0;
        selfState.riding = (p.flags & POSE.RIDE) !== 0;
        selfState.item = heldItem?.id ?? 0;
        selfState.armour = selfLooks?.armour;
        const hand = mat4(), head = mat4();
        const worn = headBlock('player', selfState.armour);
        ents.drawLiving(selfState, p.x - cam.x, p.y - cam.y, p.z - cam.z, hand, serverNow / 50, worn ? head : undefined);
        if (heldItem?.id) drawInHand(hand, heldItem.id, heldItem.meta);
        if (worn) drawOnHead(head, 'player', worn);
      }
      if (lids.size) drawLids(cam, dt);
      if (signs.size) drawSigns(cam);
      if (tiles.size) drawTiles(cam, serverNow);
      if (cracks.length) drawCracks(cam);
      if (cloudTex) drawClouds(cam, serverNow, daylight, fogColor, fogEnd);
      // particles after the entity renderer's draws (they bind their own program), before water and glass
      tickFx(cam, dt);
      if (particles.count) {
        particleGl.gamma = ents.gamma;
        fxStats.drawn = particleGl.draw(particles, viewProj, view, cam, fxClock / 50, fogColor, fog, daylight);
      } else fxStats.drawn = 0;
      const tt1 = performance.now();
      terrain.drawTranslucent(fp, vis);
      prof.translucent += performance.now() - tt1;
      lightning.draw(gl, viewProj, cam);
      if (rainShown > 0.02 && heights.size) { ents.begin(viewProj, fogColor, fog, daylight); drawWeather(cam, serverNow / 50, daylight, fog); }
      // nametags over everything
      ents.begin(viewProj, fogColor, fog, daylight);
      for (const { s, flags, collected } of entities.values()) {
        // 26.1: entities drawn with a stand-in model (or item) carry their real name
        const standIn = s.kind !== KIND.PLAYER ? entityText.get(s.id) : undefined;
        const tag = standIn ? entityTag(standIn) : null;
        if (tag && !collected && !(flags & EFLAG.INVISIBLE)) {
          const rx = s.x - cam.x, ry = s.y - cam.y, rz = s.z - cam.z;
          if (rx * rx + rz * rz <= 32 * 32) ents.drawTag(tag, rx, ry + (s.height || 0.5) + 0.35, rz, cam.yaw, cam.pitch, false);
          continue;
        }
        if (s.kind !== KIND.PLAYER || !s.label || collected || flags & EFLAG.INVISIBLE) continue;
        const rx = s.x - cam.x, ry = s.y - cam.y, rz = s.z - cam.z;
        if (rx * rx + rz * rz > 64 * 64) continue;
        // a sneaking player's name sits 0.25 lower, faint, and not through walls (method_10208)
        ents.drawTag(s.label, rx, ry + (s.height || 1.8) + 0.45 - (s.sneak ? 0.25 : 0), rz, cam.yaw, cam.pitch, !s.sneak);
      }
      // like vanilla, no nametag over yourself in third person; in free camera it helps find the bot
      if (camera.mode === 'free' && p && account) ents.drawTag(account, p.x - cam.x, p.y - cam.y + 2.25, p.z - cam.z, cam.yaw, cam.pitch);
      // held item in first person
      if (camera.mode === 'fp' && p) drawHand(p.yaw, p.pitch, livingAt(selfLife, selfPt).swing ?? 0, fpt, bob);
    }

    // adaptive resolution: hold 60 fps on phones
    const fps = 1000 / avg(frameTimes.slice(-30));
    // not while the world streams in (uploads and texture decoding make those frames slow anyway)
    if (terrain.uploadsAt > now - 1500) { lowSince = 0; highSince = 0; }
    else if (fps < 50) { highSince = 0; if (!lowSince) lowSince = now; else if (now - lowSince > 2000 && resScale > 0.5) { resScale = Math.max(0.5, resScale - 0.15); lowSince = now; } }
    else if (fps > 58) { lowSince = 0; if (!highSince) highSince = now; else if (now - highSince > 5000 && resScale < 1) { resScale = Math.min(1, resScale + 0.1); highSince = now; } }
    else { lowSince = 0; highSince = 0; }
    // still slow at the lowest resolution: fewer particles, then none
    if (particles.level > 0 && resScale <= 0.5 && fps < 25 && terrain.uploadsAt < now - 1500) {
      if (!slowSince) slowSince = now;
      else if (now - slowSince > 5000) { particles.level--; slowSince = now; if (!particles.level) { particles.clear(); worker.postMessage({ type: 'fx', on: false }); } }
    } else slowSince = 0;

    if (p) {
      info.textContent = `${account}${game}`;
      $('coords').textContent = `XYZ ${p.x.toFixed(1)} ${(p.y + worldMinY).toFixed(1)} ${p.z.toFixed(1)}`;
    }
    if (showDebug && Math.floor(now / 250) !== lastDebug) { lastDebug = Math.floor(now / 250); updateDebug(fps, delay, lagP90); }
  }
  let lastDebug = 0;

  /** The 20 Hz particle clock: display ticks, rain splashes, dig hits, then every particle and bolt. */
  function tickFx(cam: View, dt: number): void {
    fxClock += dt * 1000;
    for (let n = 0; fxClock >= 50; n++) {
      fxClock -= 50;
      if (n >= 3) { fxClock %= 50; break; }
      particles.setCamera(cam.x, cam.y, cam.z);
      if (particles.level) {
        particles.ambient();
        if (rainShown > 0.02) particles.rain(rainShown, rainTop);
        // a hit fragment a tick on the face of every block being dug that looks at the camera
        for (const c of cracks) {
          const dx = cam.x - c.x - 0.5, dy = cam.y - c.y - 0.5, dz = cam.z - c.z - 0.5, ax = Math.abs(dx), ay = Math.abs(dy), az = Math.abs(dz);
          const face = ay >= ax && ay >= az ? (dy > 0 ? 1 : 0) : ax >= az ? (dx > 0 ? 5 : 4) : (dz > 0 ? 3 : 2);
          particles.hitBlock(c.x, c.y, c.z, c.state, face);
        }
      }
      particles.tick();
      lightning.tick();
    }
  }
  /** The precipitation height where it rains (not snows) at a block column, else −1. */
  const rainTop = (x: number, z: number): number => {
    const col = heights.get(`${Math.floor(x / 16)},${Math.floor(z / 16)}`);
    if (!col) return -1;
    const i = (x & 15) + 16 * (z & 15), top = col.top[i];
    return precipitation(climate, col.biomes[i], top) === 'rain' ? top : -1;
  };
  const selfState: EntityState = { id: -1, kind: KIND.PLAYER, name: 'player', label: '', width: 0.6, height: 1.8, item: 0, meta: 0, x: 0, y: 0, z: 0, yaw: 0, pitch: 0, headYaw: 0, swing: 0, swingAmt: 0, px: 0, pz: 0, bodyYaw: 0, spin: 0 };

  let cloudVao: { vao: WebGLVertexArrayObject; buf: WebGLBuffer } | null = null;
  const cloudData = new Float32Array(7 * 6);
  function drawClouds(cam: View, serverNow: number, daylight: number, fogColor: [number, number, number], fogEnd: number): void {
    cloudVao ??= ents.dynamic();
    const R = 384, y = 128.33 - cam.y;
    const scale = 1 / (12 * 256);
    const drift = ((serverNow / 50) * 0.03) % (12 * 256); // blocks per tick, as vanilla (mod one period: float uvs)
    // uvs near 0: the camera's position within one texture period
    const px = ((cam.x + drift) % 3072 + 3072) % 3072, pz = (cam.z % 3072 + 3072) % 3072;
    const u0 = (px - R) * scale, v0 = (pz - R) * scale, u1 = (px + R) * scale, v1 = (pz + R) * scale;
    const c = [[-R, -R, u0, v0], [R, -R, u1, v0], [R, R, u1, v1], [-R, -R, u0, v0], [R, R, u1, v1], [-R, R, u0, v1]];
    c.forEach(([x, z, u, v], i) => cloudData.set([x, y, z, u, v, 0, 1], i * 7));
    gl.bindBuffer(gl.ARRAY_BUFFER, cloudVao.buf);
    gl.bufferData(gl.ARRAY_BUFFER, cloudData, gl.DYNAMIC_DRAW);
    const b = Math.max(0.15, daylight);
    ents.drawTextured(cloudVao.vao, 6, cloudTex!, [b, b, b * 1.02, 0.8], [fogEnd * 1.2, R]);
    void fogColor;
  }

  /** A block on a head (the snow golem's pumpkin, a block in the helmet slot): `head` is the head's matrix in blocks. */
  function drawOnHead(head: Float32Array, name: string, [id, meta]: [number, number]): void {
    const mesh = ents.blockMesh(`b${id}:${meta}`, blockBaked(id, meta), pack.base);
    if (!mesh) return;
    const at = headBlockPlace(name);
    // the block mesh stands on y 0: centre it on the point `up` above the head's pivot
    const m = modelMatrix(mat4(), 0, at.up - at.scale / 2, 0, 0, 0, at.scale);
    ents.drawItemMatrix(multiply(mat4(), head, m), 0, mesh);
  }

  /** An item in a model's right hand (third person): sprites stand along the arm, blocks sit small in the fist. */
  function drawInHand(hand: Float32Array, id: number, meta: number): void {
    const block = id < 256;
    const layer = block ? null : itemLayer(id, meta);
    const mesh = block ? ents.blockMesh(`b${id}:${meta}`, blockBaked(id, meta), pack.base) : null;
    if (!mesh && layer === null) return;
    const m = block ? modelMatrix(mat4(), 0, -0.12, -0.05, 0.6, 0, 0.3) : modelMatrix(mat4(), 0, -0.05, -0.2, Math.PI / 2, -Math.PI / 2 + 0.6, 0.55);
    ents.drawItemMatrix(multiply(mat4(), hand, m), layer !== null ? pack.base[layer] : 0, mesh ?? extruded(layer));
  }

  /**
   * First person: the item the hand holds (it changes at the bottom of the equip dip) or the bare arm, as
   * HeldItemRenderer places them (anim.ts), under the same bobbing as the view and its own projection so it never
   * clips into blocks.
   */
  function drawHand(yaw: number, pitch: number, swing: number, pt: number, bob: Float32Array | null): void {
    gl.clear(gl.DEPTH_BUFFER_BIT);
    ents.begin(perspective(mat4(), (70 * Math.PI) / 180, canvas.width / canvas.height, 0.01, 10), [0, 0, 0], [1000, 1001], 1);
    const [id, meta] = fpView.shown ? fpView.shown.split(':').map(Number) : [0, 0];
    const m = mat4();
    if (!id) {
      fpArmMatrix(m, fpView, pt, yaw, pitch, swing);
      if (bob) multiply(m, bob, m);
      ents.drawArmFirstPerson(m, selfState);
      return;
    }
    const block = id < 256;
    fpItemMatrix(m, fpView, pt, yaw, pitch, swing, block, useAction(id), fpView.use >= 0 ? fpView.use + pt : -1);
    if (bob) multiply(m, bob, m);
    const layer = block ? null : itemLayer(id, meta);
    const mesh = block ? ents.blockMesh(`b${id}:${meta}`, blockBaked(id, meta), pack.base) : null;
    if (mesh || layer !== null) ents.drawItemMatrix(m, layer !== null ? pack.base[layer] : 0, mesh ?? extruded(layer));
  }

  /** A dropped item: spinning and bobbing, a block as a small cube, an item as its extruded icon. */
  function drawDrop(s: EntityState, rx: number, ry: number, rz: number, now: number, dt: number): void {
    s.spin += dt * 1.2;
    const bob = Math.sin(now / 400 + s.id) * 0.06 + 0.12;
    const layer = s.item >= 256 ? itemLayer(s.item, s.meta) : null;
    const mesh = s.item > 0 && s.item < 256 ? ents.blockMesh(`b${s.item}:${s.meta}`, blockBaked(s.item, s.meta), pack.base) : null;
    if (mesh || layer !== null) ents.drawItem(rx, ry + bob, rz, s.spin, layer !== null ? pack.base[layer] : 0, mesh ?? extruded(layer), mesh ? 0.5 : 0.45, !!mesh);
  }

  /** ItemModelGenerator: the icon with a pixel-deep edge (from its alpha), as a mesh. */
  const alphaCanvas = document.createElement('canvas');
  alphaCanvas.width = 16; alphaCanvas.height = 16;
  const alphaCtx = alphaCanvas.getContext('2d', { willReadFrequently: true })!;
  function extruded(layer: number | null): { vao: WebGLVertexArrayObject; count: number } | null {
    if (layer === null) return null;
    const icon = pack.icons[layer];
    if (!icon) return null;
    alphaCtx.clearRect(0, 0, 16, 16);
    alphaCtx.drawImage(icon, 0, 0, 16, 16);
    const px = alphaCtx.getImageData(0, 0, 16, 16).data;
    const mask = new Uint8Array(256);
    for (let i = 0; i < 256; i++) mask[i] = px[i * 4 + 3] > 127 ? 1 : 0;
    return ents.itemMesh(`i${layer}`, pack.base[layer], mask);
  }

  function updateDebug(fps: number, delay: number, lag: number): void {
    const ns = net.stats();
    const lat = [...latency].sort((a, b) => a - b);
    const ft = [...frameTimes].sort((a, b) => a - b);
    debugBox.textContent = [
      `FPS ${fps.toFixed(0)}  frame p95 ${(ft[Math.floor(ft.length * 0.95)] ?? 0).toFixed(1)} ms  resolution ${canvas.width}×${canvas.height}`,
      `latency (tick→frame) avg ${avg(lat).toFixed(0)} ms  p95 ${(lat[Math.floor(lat.length * 0.95)] ?? 0).toFixed(0)} ms`,
      `RTT ${ns.rtt.toFixed(0)} ms  arrival ${lag.toFixed(0)} ms  buffer ${delay.toFixed(0)} ms`,
      `net ${(ns.bytesPerSec / 1024).toFixed(1)} KB/s  total ${(ns.total / 1048576).toFixed(2)} MB`,
      `sections ${terrain.count}  drawn ${terrain.drawn}  quads ${(terrain.quads / 1000).toFixed(0)}k  GPU ${(terrain.gpuBytes / 1048576).toFixed(1)} MB`,
      `mesh ${workerStats.meshed} (${workerStats.meshed ? (workerStats.meshMs / workerStats.meshed).toFixed(1) : 0} ms)  columns ${workerStats.columns}  entities ${entities.size}`,
      `particles ${particles.count} (level ${particles.level})  emitters ${particles.emitters.length / 4}`,
    ].join('\n');
  }

  (window as any).__viewerReset = () => { latency.length = 0; frameTimes.length = 0; prof.opaque = prof.translucent = prof.frames = 0; };
  // the entities' animation state (bench checks and debugging)
  (window as any).__viewerEntities = () => [...entities.values()].map(({ s, life, flags, vehicle }) => ({ id: s.id, name: s.name, yaw: s.yaw, headYaw: s.headYaw, body: s.bodyYaw, hurt: life.hurt, death: life.death, swing: s.arm, flags, vehicle }));
  window.__viewerStats = () => {
    const lat = [...latency].sort((a, b) => a - b);
    const ft = [...frameTimes].sort((a, b) => a - b);
    return {
      fps: 1000 / avg(frameTimes.slice(-60)), frameP95: ft[Math.floor(ft.length * 0.95)] ?? 0, jsMs: avg(jsTimes),
      latencyAvg: avg(lat), latencyP95: lat[Math.floor(lat.length * 0.95)] ?? 0, rtt: net.rtt,
      bytes: net.total, sections: terrain.count, quads: terrain.quads, draws: terrain.drawn, gpuBytes: terrain.gpuBytes,
      meshed: workerStats.meshed, meshMs: workerStats.meshMs, columns: workerStats.columns, entities: entities.size,
      width: canvas.width, height: canvas.height, inGame, mode: camera.mode, signs: [...signs.values()].reduce((n, l) => n + l.length, 0),
      opaqueMs: prof.opaque / Math.max(1, prof.frames), translucentMs: prof.translucent / Math.max(1, prof.frames),
      particles: particles.count, particlesDrawn: fxStats.drawn, fxLevel: particles.level, emitters: particles.emitters.length / 4, bolts: lightning.bolts.length,
    };
  };

  net.connect();
  requestAnimationFrame(frame);
  show(CONNECTING);
}

const avg = (a: number[]) => (a.length ? a.reduce((x, y) => x + y, 0) / a.length : 0);

/** 4×4 inverse (general). */
function invert(out: Float32Array, m: Float32Array): Float32Array {
  const a00 = m[0], a01 = m[1], a02 = m[2], a03 = m[3], a10 = m[4], a11 = m[5], a12 = m[6], a13 = m[7];
  const a20 = m[8], a21 = m[9], a22 = m[10], a23 = m[11], a30 = m[12], a31 = m[13], a32 = m[14], a33 = m[15];
  const b00 = a00 * a11 - a01 * a10, b01 = a00 * a12 - a02 * a10, b02 = a00 * a13 - a03 * a10, b03 = a01 * a12 - a02 * a11;
  const b04 = a01 * a13 - a03 * a11, b05 = a02 * a13 - a03 * a12, b06 = a20 * a31 - a21 * a30, b07 = a20 * a32 - a22 * a30;
  const b08 = a20 * a33 - a23 * a30, b09 = a21 * a32 - a22 * a31, b10 = a21 * a33 - a23 * a31, b11 = a22 * a33 - a23 * a32;
  let det = b00 * b11 - b01 * b10 + b02 * b09 + b03 * b08 - b04 * b07 + b05 * b06;
  if (!det) return out;
  det = 1 / det;
  out[0] = (a11 * b11 - a12 * b10 + a13 * b09) * det; out[1] = (a02 * b10 - a01 * b11 - a03 * b09) * det;
  out[2] = (a31 * b05 - a32 * b04 + a33 * b03) * det; out[3] = (a22 * b04 - a21 * b05 - a23 * b03) * det;
  out[4] = (a12 * b08 - a10 * b11 - a13 * b07) * det; out[5] = (a00 * b11 - a02 * b08 + a03 * b07) * det;
  out[6] = (a32 * b02 - a30 * b05 - a33 * b01) * det; out[7] = (a20 * b05 - a22 * b02 + a23 * b01) * det;
  out[8] = (a10 * b10 - a11 * b08 + a13 * b06) * det; out[9] = (a01 * b08 - a00 * b10 - a03 * b06) * det;
  out[10] = (a30 * b04 - a31 * b02 + a33 * b00) * det; out[11] = (a21 * b02 - a20 * b04 - a23 * b00) * det;
  out[12] = (a11 * b07 - a10 * b09 - a12 * b06) * det; out[13] = (a00 * b09 - a01 * b07 + a02 * b06) * det;
  out[14] = (a31 * b01 - a30 * b03 - a32 * b00) * det; out[15] = (a20 * b03 - a21 * b01 + a22 * b00) * det;
  return out;
}

void main().catch((e) => { show(`The viewer could not start: ${(e as Error).message}`); console.error(e); });
