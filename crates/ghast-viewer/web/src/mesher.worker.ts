/**
 * Mesher worker: keeps the columns the page received (inflated and decoded here, off the main thread),
 * re-meshes dirty sections nearest to the camera first and hands the vertex buffers back as transferables.
 *
 * main → worker  init {data, layerAlpha, colors, base} | chunk {cx, cz, deflated, body} | unload {cx, cz}
 *                | blocks {list: [x, y, z, state]*} | center {x, y, z} | reset | fx {on}
 * worker → main  mesh {cx, sy, cz, solid, cutout, translucent} | drop {cx, cz} | signs {cx, cz, signs} | tiles {cx, cz, tiles}
 *                | heights {cx, cz, top, biomes} (rain and snow) | stats {...}
 *                | fxbox {ox, oy, oz, light, shape, emitters: [x, y, z, state]*} (particles, see fxbox.ts)
 *                | broken {list: [x, y, z, state]*} blocks that turned to air (break particles)
 */
import { decodeColumn } from './shared/protocol.js';
import type { ModelData } from './shared/model-data.js';
import { Mesher, P, pidx, type Colors } from './mesher.js';
import { BOX_X, BOX_Y, BOX_Z, emits, EMITTERS, shapeOf } from './fxbox.js';
import { precipitationHeights } from './weather.js';

declare const self: { postMessage(m: unknown, transfer?: Transferable[]): void; onmessage: ((e: MessageEvent) => void) | null };

interface WSection { states: Uint16Array; sky: Uint8Array; blk: Uint8Array }
interface WSign { x: number; y: number; z: number; state: number; lines: string[] }
/** A skull or banner for the page (world position, the block state says floor/wall and which way). */
type WTile = { x: number; y: number; z: number; state: number } & ({ kind: 'skull'; skull: number; rot: number; skin?: string } | { kind: 'banner'; base: number; patterns: Array<[string, number]> });
interface WColumn {
  cx: number; cz: number; sections: Array<WSection | null>; biomes: Uint8Array; hasSky: boolean; signs?: WSign[];
  tiles?: WTile[];
  /** Flower pot contents by column-local index x + 16 * (z + 16 * y). */
  pots?: Map<number, string>;
  /** Precipitation heights (x + 16 z) as last posted. */
  top?: Uint8Array;
}

const SKULL = 144, STANDING_BANNER = 176, WALL_BANNER = 177, FLOWER_POT = 140;
const CHESTS = new Set([54, 130, 146]);
const TILE_BLOCKS = new Set([SKULL, STANDING_BANNER, WALL_BANNER]);

const columns = new Map<string, WColumn>();
/** Chests the page draws the lid of (opening, open or closing), by world "x,y,z": meshed without it, its quads posted. */
const lids = new Map<string, [number, number, number]>();
const dirty = new Set<string>();
let mesher: Mesher | null = null;
let data: ModelData | null = null;
let center = [0, 64, 0];
let chain: Promise<void> = Promise.resolve();
let scheduled = false;
let meshed = 0, meshMs = 0;

const ck = (cx: number, cz: number) => `${cx},${cz}`;

function expand(nibbles: Uint8Array | null, fill: number): Uint8Array {
  const out = new Uint8Array(4096);
  if (!nibbles) { out.fill(fill); return out; }
  for (let i = 0; i < 2048; i++) { const b = nibbles[i]; out[i * 2] = b & 15; out[i * 2 + 1] = b >> 4; }
  return out;
}

async function inflate(body: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([body as BlobPart]).stream().pipeThrough(new (globalThis as any).DecompressionStream('deflate-raw'));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

function markColumn(cx: number, cz: number, withNeighbours: boolean): void {
  for (let dx = -1; dx <= 1; dx++) for (let dz = -1; dz <= 1; dz++) {
    if (!withNeighbours && (dx || dz)) continue;
    if (dx && dz) continue;
    const c = columns.get(ck(cx + dx, cz + dz));
    if (!c) continue;
    for (let sy = 0; sy < 16; sy++) if (c.sections[sy]) dirty.add(`${cx + dx},${sy},${cz + dz}`);
  }
}

async function handle(m: any): Promise<void> {
  switch (m.type) {
    case 'init':
      data = m.data as ModelData;
      mesher = new Mesher(data, m.layerAlpha as Uint8Array, m.colors as Colors, m.base as Uint16Array);
      break;
    case 'colors':
      mesher?.setColors(m.colors);
      for (const c of columns.values()) markColumn(c.cx, c.cz, false);
      break;
    case 'chunk': {
      const body = m.deflated ? await inflate(new Uint8Array(m.body)) : new Uint8Array(m.body);
      const col = decodeColumn(m.cx, m.cz, body);
      const sections: Array<WSection | null> = new Array(16).fill(null);
      for (const s of col.sections) sections[s.y] = { states: s.states, sky: expand(s.sky, s.skyFull ? 15 : 0), blk: expand(s.block, 0) };
      const prev = columns.get(ck(m.cx, m.cz));
      columns.set(ck(m.cx, m.cz), { cx: m.cx, cz: m.cz, sections, biomes: col.biomes, hasSky: col.hasSky });
      // sections that vanished (all air now) must drop their mesh
      if (prev) for (let sy = 0; sy < 16; sy++) if (prev.sections[sy] && !sections[sy]) self.postMessage({ type: 'mesh', cx: m.cx, sy, cz: m.cz, solid: null, cutout: null, translucent: null });
      markColumn(m.cx, m.cz, true);
      // sign text for the page (it draws the lines on the boards); the state says standing/wall and which way
      const signs = col.signs.map((sg) => {
        const sec = sections[sg.y >> 4];
        const state = sec ? sec.states[sg.x + 16 * (sg.z + 16 * (sg.y & 15))] : 0;
        return { x: m.cx * 16 + sg.x, y: sg.y, z: m.cz * 16 + sg.z, state, lines: sg.lines };
      }).filter(sg => sg.state >> 4 === 63 || sg.state >> 4 === 68);
      const wc = columns.get(ck(m.cx, m.cz))!;
      wc.signs = signs;
      self.postMessage({ type: 'signs', cx: m.cx, cz: m.cz, signs });
      // skulls and banners for the page, flower pot contents for the mesher
      const stateAt = (x: number, y: number, z: number) => { const sec = sections[y >> 4]; return sec ? sec.states[x + 16 * (z + 16 * (y & 15))] : 0; };
      wc.tiles = [];
      wc.pots = new Map();
      for (const t of col.tiles) {
        const state = stateAt(t.x, t.y, t.z);
        if (t.kind === 'pot') { if (state >> 4 === FLOWER_POT) wc.pots.set(t.x + 16 * (t.z + 16 * t.y), `contents=${t.contents}`); continue; }
        if (!TILE_BLOCKS.has(state >> 4) || (t.kind === 'skull') !== (state >> 4 === SKULL)) continue;
        wc.tiles.push({ ...t, x: m.cx * 16 + t.x, z: m.cz * 16 + t.z, state } as WTile);
      }
      self.postMessage({ type: 'tiles', cx: m.cx, cz: m.cz, tiles: wc.tiles });
      postHeights(wc);
      break;
    }
    case 'unload':
      columns.delete(ck(m.cx, m.cz));
      for (let sy = 0; sy < 16; sy++) dirty.delete(`${m.cx},${sy},${m.cz}`);
      self.postMessage({ type: 'drop', cx: m.cx, cz: m.cz });
      self.postMessage({ type: 'signs', cx: m.cx, cz: m.cz, signs: [] });
      self.postMessage({ type: 'tiles', cx: m.cx, cz: m.cz, tiles: [] });
      markColumn(m.cx, m.cz, true);
      break;
    case 'blocks': {
      const l: number[] = m.list;
      const broken: number[] = [];
      for (let i = 0; i < l.length; i += 4) {
        const before = setBlock(l[i], l[i + 1], l[i + 2], l[i + 3]);
        // dug out (not liquids or fire going out); a burst of them (an explosion, a big change) breaks quietly
        if (!l[i + 3] && before && !data?.blocks[before >> 4]?.fluid && before >> 4 !== 51) broken.push(l[i], l[i + 1], l[i + 2], before);
      }
      if (broken.length && broken.length <= 4 * 4 && fxOn) self.postMessage({ type: 'broken', list: broken });
      if (fxOn) boxSoon();
      break;
    }
    case 'center': center = [m.x, m.y, m.z]; if (fxOn) boxSoon(0); break;
    case 'fx': fxOn = !!m.on; if (fxOn) boxSoon(0); break;
    case 'lids':
      for (const [x, y, z, on] of m.list as Array<[number, number, number, boolean]>) {
        const k = `${x},${y},${z}`;
        if (on) lids.set(k, [x, y, z]); else { lids.delete(k); self.postMessage({ type: 'lid', x, y, z, quads: null }); }
        if (y >= 0 && y < 256) dirty.add(`${Math.floor(x / 16)},${y >> 4},${Math.floor(z / 16)}`);
      }
      break;
    case 'reset':
      for (const c of columns.values()) self.postMessage({ type: 'drop', cx: c.cx, cz: c.cz });
      columns.clear(); dirty.clear(); lids.clear();
      break;
  }
  if (m.type === 'chunk' && fxOn && Math.abs(m.cx * 16 + 8 - center[0]) < 40 && Math.abs(m.cz * 16 + 8 - center[2]) < 40) boxSoon();
  schedule();
}

// ---- the particle box around the camera ------------------------------------------------------------

let fxOn = false;
let boxTimer: ReturnType<typeof setTimeout> | null = null;
/** Rebuilds the box after `ms` (changes within that time share one rebuild). */
function boxSoon(ms = 150): void {
  if (boxTimer && ms > 0) return;
  if (boxTimer) clearTimeout(boxTimer);
  boxTimer = setTimeout(() => { boxTimer = null; chain = chain.then(postBox); }, ms);
}

function postBox(): void {
  if (!data || !fxOn) return;
  const blocks = data.blocks;
  const ox = Math.floor(center[0]) - BOX_X / 2, oy = Math.floor(center[1]) - BOX_Y / 2, oz = Math.floor(center[2]) - BOX_Z / 2;
  const light = new Uint8Array(BOX_X * BOX_Y * BOX_Z), shape = new Uint8Array(light.length);
  const emitters: number[] = [];
  for (let bz = 0; bz < BOX_Z; bz++) for (let bx = 0; bx < BOX_X; bx++) {
    const x = ox + bx, z = oz + bz, cx = Math.floor(x / 16), cz = Math.floor(z / 16);
    const c = columns.get(ck(cx, cz));
    const li = (x - cx * 16) + 16 * (z - cz * 16);
    for (let by = 0; by < BOX_Y; by++) {
      const y = oy + by, i = bx + BOX_X * (bz + BOX_Z * by);
      const s = c && y >= 0 && y < 256 ? c.sections[y >> 4] : null;
      if (!s) { light[i] = (c ? (c.hasSky ? 15 : 0) : 15) << 4; continue; }
      const j = li + 256 * (y & 15), st = s.states[j];
      light[i] = (s.sky[j] << 4) | s.blk[j];
      shape[i] = shapeOf(st, blocks);
      if (EMITTERS.has(st >> 4) && emitters.length < 4 * 2048 && emits(st, (dy) => stateAt(x, y + dy, z), blocks)) emitters.push(x, y, z, st);
    }
  }
  const em = new Int32Array(emitters);
  self.postMessage({ type: 'fxbox', ox, oy, oz, light, shape, emitters: em }, [light.buffer, shape.buffer, em.buffer]);
}

function stateAt(x: number, y: number, z: number): number {
  const c = cell(x, y, z);
  return c ? c.s.states[c.i] : 0;
}

/** Sets a block; returns the state it had. */
function setBlock(x: number, y: number, z: number, state: number): number {
  if (y < 0 || y > 255) return 0;
  const cx = Math.floor(x / 16), cz = Math.floor(z / 16), sy = y >> 4;
  const c = columns.get(ck(cx, cz));
  if (!c) return 0;
  let s = c.sections[sy];
  if (!s) {
    if (state === 0) return 0;
    s = { states: new Uint16Array(4096), sky: new Uint8Array(4096).fill(c.hasSky ? 15 : 0), blk: new Uint8Array(4096) };
    c.sections[sy] = s;
  }
  const lx = x - cx * 16, ly = y & 15, lz = z - cz * 16;
  const before = s.states[lx + 16 * (lz + 16 * ly)];
  s.states[lx + 16 * (lz + 16 * ly)] = state;
  // a sign broken or replaced: its text goes (new text comes with the column the server re-sends)
  if ((before >> 4 === 63 || before >> 4 === 68) && before !== state && c.signs?.length) {
    c.signs = c.signs.filter(sg => sg.x !== x || sg.y !== y || sg.z !== z);
    self.postMessage({ type: 'signs', cx, cz, signs: c.signs });
  }
  // the same for skulls, banners and flower pots (their new data comes with the re-sent column)
  if ((TILE_BLOCKS.has(before >> 4) || before >> 4 === FLOWER_POT) && before !== state) {
    c.pots?.delete(lx + 16 * (lz + 16 * y));
    if (c.tiles?.some(t => t.x === x && t.y === y && t.z === z)) {
      c.tiles = c.tiles.filter(t => t.x !== x || t.y !== y || t.z !== z);
      self.postMessage({ type: 'tiles', cx, cz, tiles: c.tiles });
    }
  }
  // the rain stops at the top block: a change at or above it moves it
  if (y + 1 >= (c.top?.[lx + 16 * lz] ?? 0)) postHeights(c);
  // the server does not send light with block changes: relight around when the opacity changed
  if (mesher && opacity(before) !== opacity(state)) relight(x, y, z, c.hasSky);
  dirty.add(`${cx},${sy},${cz}`);
  // border blocks change the neighbour's faces and light samples
  if (lx === 0) dirtyIf(cx - 1, sy, cz);
  if (lx === 15) dirtyIf(cx + 1, sy, cz);
  if (lz === 0) dirtyIf(cx, sy, cz - 1);
  if (lz === 15) dirtyIf(cx, sy, cz + 1);
  if (ly === 0 && sy > 0) dirtyIf(cx, sy - 1, cz);
  if (ly === 15 && sy < 15) dirtyIf(cx, sy + 1, cz);
  return before;
}

/** Light opacity like Block.getOpacity: full cubes stop light, leaves/water/ice dim it. */
function opacity(state: number): number {
  if (!state) return 0;
  if (mesher!.solid(state)) return 15;
  const id = state >> 4;
  return id === 18 || id === 161 || id === 8 || id === 9 || id === 79 || id === 30 ? 1 : 0;
}

interface Cell { s: WSection; i: number }
function cell(x: number, y: number, z: number): Cell | null {
  if (y < 0 || y > 255) return null;
  const cx = Math.floor(x / 16), cz = Math.floor(z / 16);
  const s = columns.get(ck(cx, cz))?.sections[y >> 4];
  return s ? { s, i: (x - cx * 16) + 16 * ((z - cz * 16) + 16 * (y & 15)) } : null;
}
const skyAt = (x: number, y: number, z: number): number => { if (y > 255) return 15; const c = cell(x, y, z); return c ? c.s.sky[c.i] : 15; };

/**
 * A local light update (vanilla's World.method_8568, simplified): sky light straight down the changed column,
 * then both lights spread back in from the surroundings (a block at most 14 away) and a removed light
 * path darkens what it fed. Sections without data (all air) count as open sky and are not written.
 */
function relight(x: number, y: number, z: number, hasSky: boolean): void {
  const R = 15;
  const touched = new Set<string>();
  const mark = (wx: number, wy: number, wz: number) => touched.add(`${Math.floor(wx / 16)},${wy >> 4},${Math.floor(wz / 16)}`);
  // 1. reset the box to "unknown" (0) where it is not straight sky, keeping fixed sources (lava, torches by state)
  const x0 = x - R, x1 = x + R, z0 = z - R, z1 = z + R, y0 = Math.max(0, y - R), y1 = Math.min(255, y + R);
  const queue: number[] = [];
  for (let wx = x0; wx <= x1; wx++) for (let wz = z0; wz <= z1; wz++) {
    // straight sky from above the box down
    let sky = hasSky ? skyAt(wx, y1 + 1, wz) : 0;
    for (let wy = y1; wy >= y0; wy--) {
      const c = cell(wx, wy, wz);
      if (!c) { sky = hasSky ? 15 : 0; continue; }
      const op = opacity(c.s.states[c.i]);
      sky = op >= 15 ? 0 : Math.max(0, sky - op);
      const edge = wx === x0 || wx === x1 || wz === z0 || wz === z1 || wy === y0 || wy === y1;
      const before = c.s.sky[c.i];
      // the box edge keeps its light (it feeds the inside); inside, only the straight sky is known now
      const now = edge ? Math.max(before, sky) : sky;
      if (now !== before) { c.s.sky[c.i] = now; mark(wx, wy, wz); }
      if (now > 1) queue.push(wx, wy, wz);
      const bl = blockLightSource(c.s.states[c.i]);
      if (!edge && c.s.blk[c.i] !== bl) { c.s.blk[c.i] = bl; mark(wx, wy, wz); }
      if (c.s.blk[c.i] > 1) queue.push(wx, wy, wz);
    }
  }
  // 2. spread both lights (BFS, -1 per step, opacity on entry)
  const dirs = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];
  for (let q = 0; q < queue.length; q += 3) {
    const wx = queue[q], wy = queue[q + 1], wz = queue[q + 2];
    const c = cell(wx, wy, wz);
    if (!c) continue;
    const sl = c.s.sky[c.i], bl = c.s.blk[c.i];
    for (const [dx, dy, dz] of dirs) {
      const nx = wx + dx, ny = wy + dy, nz = wz + dz;
      if (nx < x0 || nx > x1 || nz < z0 || nz > z1 || ny < y0 || ny > y1) continue;
      const n = cell(nx, ny, nz);
      if (!n) continue;
      const op = Math.max(1, opacity(n.s.states[n.i]));
      if (op >= 15) continue;
      let grew = false;
      if (sl - op > n.s.sky[n.i]) { n.s.sky[n.i] = sl - op; grew = true; }
      if (bl - op > n.s.blk[n.i]) { n.s.blk[n.i] = bl - op; grew = true; }
      if (grew) { mark(nx, ny, nz); queue.push(nx, ny, nz); }
    }
  }
  for (const k of touched) {
    const [cx, sy, cz] = k.split(',').map(Number);
    dirtyIf(cx, sy, cz);
  }
}

/** Block light a block emits (Block.lightLevel, 1.8). */
function blockLightSource(state: number): number {
  switch (state >> 4) {
    case 10: case 11: case 51: case 89: case 91: case 119: case 124: case 138: case 169: return 15;
    case 50: return 14;
    case 62: case 74: case 76: case 94: return id9(state);
    case 90: return 11;
    case 130: return 7;
    case 39: case 122: return 1;
    default: return 0;
  }
}
const id9 = (state: number) => ({ 62: 13, 74: 9, 76: 7, 94: 9 } as Record<number, number>)[state >> 4] ?? 0;

/** Rain and snow heights of a column for the page, when they changed. */
function postHeights(c: WColumn): void {
  if (!data) return;
  const stops = (st: number) => { const b = data!.blocks[st >> 4]; return !!b && (!!b.collides || !!b.fluid); };
  const top = precipitationHeights(c.sections, stops);
  if (c.top && c.top.every((v, i) => v === top[i])) return;
  c.top = top;
  self.postMessage({ type: 'heights', cx: c.cx, cz: c.cz, top: top.slice(), biomes: c.biomes.slice() });
}

function dirtyIf(cx: number, sy: number, cz: number): void {
  if (columns.get(ck(cx, cz))?.sections[sy]) dirty.add(`${cx},${sy},${cz}`);
}

// ---- meshing loop --------------------------------------------------------------------------------

const pad = { states: new Uint16Array(P * P * P), sky: new Uint8Array(P * P * P), blk: new Uint8Array(P * P * P), biomes: new Uint8Array(P * P) };
const BEDROCK = 7 << 4;

const extra = new Map<number, string>();
const open = new Set<number>();

function fill(cx: number, sy: number, cz: number): boolean {
  const near: Array<WSection | null | undefined> = new Array(27);
  const cols: Array<WColumn | undefined> = new Array(9);
  for (let dz = -1; dz <= 1; dz++) for (let dx = -1; dx <= 1; dx++) {
    const c = columns.get(ck(cx + dx, cz + dz));
    cols[(dx + 1) + (dz + 1) * 3] = c;
    for (let dy = -1; dy <= 1; dy++) {
      const y = sy + dy;
      near[(dx + 1) + 3 * ((dz + 1) + 3 * (dy + 1))] = c ? (y < 0 || y > 15 ? null : c.sections[y]) : undefined;
    }
  }
  const self = near[13];
  if (!self) return false;
  for (let y = -1; y <= 16; y++) {
    const dy = y < 0 ? -1 : y > 15 ? 1 : 0, ly = y & 15;
    for (let z = -1; z <= 16; z++) {
      const dz = z < 0 ? -1 : z > 15 ? 1 : 0, lz = z & 15;
      for (let x = -1; x <= 16; x++) {
        const dx = x < 0 ? -1 : x > 15 ? 1 : 0, lx = x & 15;
        const s = near[(dx + 1) + 3 * ((dz + 1) + 3 * (dy + 1))];
        const i = pidx(x, y, z);
        if (s) {
          const j = lx + 16 * (lz + 16 * ly);
          pad.states[i] = s.states[j]; pad.sky[i] = s.sky[j]; pad.blk[i] = s.blk[j];
        } else {
          const col = cols[(dx + 1) + (dz + 1) * 3];
          const below = sy + dy < 0;
          pad.states[i] = below ? BEDROCK : 0;
          pad.sky[i] = below ? 0 : !col || col.hasSky ? 15 : 0;
          pad.blk[i] = 0;
        }
      }
    }
  }
  for (let z = -1; z <= 16; z++) for (let x = -1; x <= 16; x++) {
    const dx = x < 0 ? -1 : x > 15 ? 1 : 0, dz = z < 0 ? -1 : z > 15 ? 1 : 0;
    const c = cols[(dx + 1) + (dz + 1) * 3] ?? cols[4]!;
    pad.biomes[(x + 1) + P * (z + 1)] = c.biomes[(x & 15) + 16 * (z & 15)];
  }
  extra.clear();
  for (const [k, contents] of cols[4]!.pots ?? []) {
    const y = k >> 8;
    if (y >> 4 === sy) extra.set(pidx(k & 15, y & 15, (k >> 4) & 15), contents);
  }
  open.clear();
  for (const [x, y, z] of lids.values()) {
    if (Math.floor(x / 16) !== cx || y >> 4 !== sy || Math.floor(z / 16) !== cz) continue;
    const i = pidx(x & 15, y & 15, z & 15);
    if (CHESTS.has(pad.states[i] >> 4)) open.add(i);
  }
  return true;
}

function schedule(): void {
  if (scheduled || !mesher || !dirty.size) return;
  scheduled = true;
  setTimeout(run, 0);
}

function run(): void {
  scheduled = false;
  if (!mesher) return;
  const t0 = performance.now();
  // nearest dirty sections first (by section centre)
  const order = [...dirty].map((k) => {
    const [cx, sy, cz] = k.split(',').map(Number);
    const d = Math.hypot(cx * 16 + 8 - center[0], (sy * 16 + 8 - center[1]) * 1.5, cz * 16 + 8 - center[2]);
    return { k, cx, sy, cz, d };
  }).sort((a, b) => a.d - b.d);
  for (const s of order) {
    if (performance.now() - t0 > 12) break;
    dirty.delete(s.k);
    if (!fill(s.cx, s.sy, s.cz)) { self.postMessage({ type: 'mesh', cx: s.cx, sy: s.sy, cz: s.cz, solid: null, cutout: null, translucent: null }); continue; }
    const m0 = performance.now();
    const input = { ...pad, wx: s.cx * 16, wy: s.sy * 16, wz: s.cz * 16, extra, open };
    const mesh = mesher.mesh(input);
    // the lids left out: the page draws them on their hinge
    for (const i of open) {
      const lx = i % P - 1, lz = Math.floor(i / P) % P - 1, ly = Math.floor(i / (P * P)) - 1;
      self.postMessage({ type: 'lid', x: s.cx * 16 + lx, y: s.sy * 16 + ly, z: s.cz * 16 + lz, state: pad.states[i], quads: mesher.lidOf(input, lx, ly, lz) });
    }
    meshMs += performance.now() - m0; meshed++;
    const out = (b: Uint8Array) => (b.length ? b.buffer : null);
    const msg = { type: 'mesh', cx: s.cx, sy: s.sy, cz: s.cz, solid: out(mesh.solid), cutout: out(mesh.cutout), translucent: out(mesh.translucent) };
    self.postMessage(msg, [msg.solid, msg.cutout, msg.translucent].filter(Boolean) as ArrayBuffer[]);
  }
  if (dirty.size) schedule();
  else self.postMessage({ type: 'stats', meshed, meshMs: Math.round(meshMs), columns: columns.size });
}

self.onmessage = (e: MessageEvent) => {
  const m = e.data;
  chain = chain.then(() => handle(m)).catch((err) => { self.postMessage({ type: 'error', message: String(err?.message ?? err) }); });
};
