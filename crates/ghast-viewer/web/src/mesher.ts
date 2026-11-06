/**
 * Section mesher (runs in the worker; pure, so the tests drive it directly). One 16³ section with a
 * one-block border (18³ arrays) → vertex streams for the solid, cutout and translucent passes.
 *
 * - block states decoded the 1.8 way (blocks18.ts), models baked once per variant (bake.ts)
 * - faces culled against full opaque cubes (and glass/ice against themselves)
 * - smooth lighting + ambient occlusion per vertex (vanilla's 4-sample corner average, opaque = 0.2),
 *   the quad split along the brighter diagonal
 * - greedy merging of full-cube faces whose four corners are identical (flat lit areas)
 * - biome grass/foliage colours blended over 3×3 columns, fixed colours for spruce/birch/lily pads,
 *   stem age and redstone power colours
 * - water/lava surfaces with vanilla corner heights and flow direction; water in the translucent pass
 *
 * Vertex (20 bytes): u16 x, y, z ((p + 1) * 2048, section-local), u16 layer, u16 u, v ((uv + 16) * 256),
 * u8 r, g, b, a (tint × shade × AO), u8 sky, block (0..255), frames (bit 7: turn the texture per block), frametime.
 */
import { decode, NEEDS_NEIGHBOURS, fluidHeight, type BlockFacts, type BlockNames } from './shared/blocks18.js';
import type { BlockData, ModelData, VariantRef } from './shared/model-data.js';
import { bake, DIR_SHADE, DIR_VEC, type Baked, type Quad } from './bake.js';

export const VERTEX_BYTES = 20;
export const P = 18; // padded edge
export const pidx = (x: number, y: number, z: number): number => (x + 1) + P * ((z + 1) + P * (y + 1));

export interface SectionInput {
  states: Uint16Array; // P³
  sky: Uint8Array; // P³, 0..15
  blk: Uint8Array; // P³, 0..15
  biomes: Uint8Array; // P² (x + 1) + P * (z + 1)
  /** World coordinates of the section's corner (random model variants). */
  wx: number; wy: number; wz: number;
  /** Variant keys from tile entities (a flower pot's contents) by padded index, over the decoded key. */
  extra?: Map<number, string>;
  /** Chests (padded index) whose lid and latch the page draws opening: left out here. */
  open?: Set<number>;
}
export interface SectionMesh { solid: Uint8Array; cutout: Uint8Array; translucent: Uint8Array }

export class VertexBuffer {
  buf: ArrayBuffer;
  u8: Uint8Array;
  u16: Uint16Array;
  n = 0;
  constructor(cap = 4096) { this.buf = new ArrayBuffer(cap * VERTEX_BYTES); this.u8 = new Uint8Array(this.buf); this.u16 = new Uint16Array(this.buf); }
  private grow(): void {
    const next = new ArrayBuffer(this.buf.byteLength * 2);
    new Uint8Array(next).set(this.u8);
    this.buf = next; this.u8 = new Uint8Array(next); this.u16 = new Uint16Array(next);
  }
  push(x: number, y: number, z: number, layer: number, u: number, v: number, r: number, g: number, b: number, sky: number, blk: number, frames: number, ft: number): void {
    if ((this.n + 1) * VERTEX_BYTES > this.buf.byteLength) this.grow();
    const o = this.n * VERTEX_BYTES, h = o >> 1;
    this.u16[h] = Math.round((x + 1) * 2048); this.u16[h + 1] = Math.round((y + 1) * 2048); this.u16[h + 2] = Math.round((z + 1) * 2048);
    this.u16[h + 3] = layer;
    this.u16[h + 4] = Math.max(0, Math.min(65535, Math.round((u + 16) * 256))); this.u16[h + 5] = Math.max(0, Math.min(65535, Math.round((v + 16) * 256)));
    this.u8[o + 12] = r; this.u8[o + 13] = g; this.u8[o + 14] = b; this.u8[o + 15] = 255;
    this.u8[o + 16] = sky; this.u8[o + 17] = blk; this.u8[o + 18] = frames; this.u8[o + 19] = ft;
    this.n++;
  }
  done(): Uint8Array { return this.u8.slice(0, this.n * VERTEX_BYTES); }
  reset(): void { this.n = 0; }
}

const enum Kind { Unresolved = 0, None = 1, Static = 2, Dynamic = 3, Fluid = 4 }

interface Variant { baked: Baked; w: number; ref: VariantRef }
/** Variant lists that differ only in a y rotation of one model (grass, dirt, sand...): drawn as the first, the shader turns the top/bottom texture per block. */
const ROT_ONLY = new WeakSet<Variant[]>();

/** Block ids with useNeighbourLight in 1.8: farmland, slabs, stairs. */
const NEIGHBOUR_LIGHT = new Uint8Array(256);
for (const id of [60, 44, 126, 182, 53, 67, 108, 109, 114, 128, 134, 135, 136, 156, 163, 164, 180]) NEIGHBOUR_LIGHT[id] = 1;

/** Blocks that cull faces against their own kind. */
const SELF_CULL = new Set(['glass', 'stained_glass', 'ice', 'packed_ice', 'slime']);

export interface Colors { grass: Uint32Array; foliage: Uint32Array; water: Uint32Array }

export class Mesher {
  private readonly kind = new Uint8Array(65536);
  private readonly solidArr = new Uint8Array(65536); // 1 = full opaque cube
  private readonly staticVariants: Array<Variant[] | undefined> = [];
  private readonly bakedCache = new Map<string, Baked>();
  private readonly dynamicCache = new Map<string, Variant[]>();
  private readonly names: BlockNames;
  private readonly facts: BlockFacts;
  private readonly vb = [new VertexBuffer(), new VertexBuffer(), new VertexBuffer()];
  // per-section scratch
  private grassCol = new Uint32Array(256);
  private foliageCol = new Uint32Array(256);
  private waterCol = new Uint32Array(256);

  /**
   * @param layerAlpha per data layer: 0 opaque, 1 has see-through pixels (cutout pass)
   * @param base per data layer: its first slice in the GPU texture array (animated layers take one per frame)
   */
  constructor(private readonly data: ModelData, private readonly layerAlpha: Uint8Array, private colors: Colors, private readonly base: Uint16Array) {
    this.names = { name: (id: number) => data.blocks[id]?.name };
    this.facts = { solid: (id: number) => !!data.blocks[id]?.solidHint && !data.blocks[id]?.translucent };
  }

  setColors(c: Colors): void { this.colors = c; }

  /** Full opaque cube (culls neighbours, darkens AO). */
  solid(state: number): boolean {
    if (this.kind[state] === Kind.Unresolved) this.resolve(state);
    return this.solidArr[state] === 1;
  }

  private block(state: number): BlockData | null { return this.data.blocks[state >> 4] ?? null; }

  private variantsFor(b: BlockData, key: string): Variant[] {
    let refs: VariantRef[] | undefined = b.variants[key];
    if (!refs) refs = b.variants.normal ?? b.variants[''] ?? Object.values(b.variants)[0];
    if (!refs) return [];
    const out = refs.map(r => ({ baked: this.baked(r), w: r.w ?? 1, ref: r }));
    if (out.length > 1 && out.every(v => v.ref.m === out[0].ref.m && !v.ref.x && !v.ref.uvlock)) {
      out.sort((a, b) => (a.ref.y ?? 0) - (b.ref.y ?? 0));
      ROT_ONLY.add(out);
    }
    return out;
  }

  private baked(r: VariantRef): Baked {
    const k = `${r.m}:${r.x ?? 0}:${r.y ?? 0}:${r.uvlock ? 1 : 0}`;
    let b = this.bakedCache.get(k);
    if (!b) { b = bake(this.data.models[r.m], r); this.bakedCache.set(k, b); }
    return b;
  }

  private resolve(state: number): void {
    const b = this.block(state);
    if (!b || state >> 4 === 0) { this.kind[state] = Kind.None; return; }
    if (b.fluid) { this.kind[state] = Kind.Fluid; this.solidArr[state] = 0; return; }
    const key = decode(b.name, state & 15, () => 0, this.names, this.facts);
    const vs = this.variantsFor(b, key);
    if (!vs.length) { this.kind[state] = Kind.None; return; }
    this.kind[state] = NEEDS_NEIGHBOURS.has(b.name) ? Kind.Dynamic : Kind.Static;
    this.staticVariants[state] = vs;
    // opaque full cube: each side has an opaque face (overlays with see-through pixels on top are fine)
    const q0 = vs[0].baked.quads;
    const full = vs[0].baked.fullCube && !b.translucent && [0, 1, 2, 3, 4, 5].every(d => q0.some(q => q.dir === d && this.layerAlpha[q.layer] === 0))
      && !b.name.startsWith('leaves');
    this.solidArr[state] = full ? 1 : 0;
  }

  // ---- meshing --------------------------------------------------------------------------------------

  mesh(s: SectionInput): SectionMesh {
    for (const v of this.vb) v.reset();
    this.biomeColors(s);
    const st = s.states;
    // which cells are full cubes (greedy candidates) handled after the per-block pass
    const greedyFaces: GreedyCell[][] = [[], [], [], [], [], []];
    for (let y = 0; y < 16; y++) for (let z = 0; z < 16; z++) for (let x = 0; x < 16; x++) {
      const i = pidx(x, y, z);
      const state = st[i];
      if (state === 0) continue;
      if (this.kind[state] === Kind.Unresolved) this.resolve(state);
      const k = this.kind[state];
      if (k === Kind.None) continue;
      if (k === Kind.Fluid) { this.fluid(s, x, y, z, state); continue; }
      const b = this.block(state)!;
      const at = this.variantAt(s, x, y, z);
      if (!at) continue;
      const { v, vs, rot } = at;
      if (v.baked.fullCube && this.solidArr[state]) {
        this.cube(s, x, y, z, state, v.baked, greedyFaces, vs.length > 1 && !rot ? vs.indexOf(v) : 0, rot);
        continue;
      }
      this.model(s, x, y, z, state, b, s.open?.has(i) ? lidless(v.baked) : v.baked);
    }
    this.greedy(greedyFaces);
    return { solid: this.vb[0].done(), cutout: this.vb[1].done(), translucent: this.vb[2].done() };
  }

  private biomeColors(s: SectionInput): void {
    const c = this.colors;
    for (let z = 0; z < 16; z++) for (let x = 0; x < 16; x++) {
      let gr = 0, gg = 0, gb = 0, fr = 0, fg = 0, fb = 0, wr = 0, wg = 0, wb = 0;
      for (let dz = -1; dz <= 1; dz++) for (let dx = -1; dx <= 1; dx++) {
        const bio = s.biomes[(x + dx + 1) + P * (z + dz + 1)];
        const g = c.grass[bio], f = c.foliage[bio], w = c.water[bio];
        gr += g >> 16 & 255; gg += g >> 8 & 255; gb += g & 255;
        fr += f >> 16 & 255; fg += f >> 8 & 255; fb += f & 255;
        wr += w >> 16 & 255; wg += w >> 8 & 255; wb += w & 255;
      }
      const k = x + z * 16;
      this.grassCol[k] = ((gr / 9) << 16) | ((gg / 9) << 8) | (gb / 9);
      this.foliageCol[k] = ((fr / 9) << 16) | ((fg / 9) << 8) | (fb / 9);
      this.waterCol[k] = ((wr / 9) << 16) | ((wg / 9) << 8) | (wb / 9);
    }
  }

  /** The colour multiplier of a tinted face (BlockColors in later versions). */
  private tint(b: BlockData, state: number, x: number, z: number): number {
    const k = x + z * 16;
    switch (b.name) {
      case 'grass': case 'tallgrass': case 'double_plant': case 'reeds': case 'flower_pot': return this.grassCol[k]; // the potted fern is the only tinted pot
      case 'leaves': { const v = state & 3; return v === 1 ? 0x619961 : v === 2 ? 0x80a755 : this.foliageCol[k]; }
      case 'leaves2': case 'vine': return this.foliageCol[k];
      case 'waterlily': return 0x208030;
      case 'pumpkin_stem': case 'melon_stem': { const a = state & 7; return ((a * 32) << 16) | ((255 - a * 8) << 8) | (a * 4); }
      case 'redstone_wire': {
        const f = (state & 15) / 15;
        const r = f * 0.6 + (f > 0 ? 0.4 : 0.3), g = Math.max(0, f * f * 0.7 - 0.5), bb = Math.max(0, f * f * 0.6 - 0.7);
        return (Math.round(r * 255) << 16) | (Math.round(g * 255) << 8) | Math.round(bb * 255);
      }
      default: return 0xffffff;
    }
  }

  /** Neighbour culls the face: a full opaque cube, or the same see-through block (glass, ice). */
  private culled(s: SectionInput, x: number, y: number, z: number, dir: number, self: number): boolean {
    const [dx, dy, dz] = DIR_VEC[dir];
    const n = s.states[pidx(x + dx, y + dy, z + dz)];
    if (n === 0) return false;
    if (this.solid(n)) return true;
    if (n >> 4 === self >> 4) {
      const name = this.data.blocks[self >> 4]?.name ?? '';
      if (SELF_CULL.has(name)) return name !== 'stained_glass' || n === self;
    }
    return false;
  }

  /** A full cube: per face corner light/AO; uniform faces go to the greedy pass, the rest straight out. */
  private cube(s: SectionInput, x: number, y: number, z: number, state: number, baked: Baked, greedy: GreedyCell[][], variant: number, rot: boolean): void {
    const b = this.block(state)!;
    for (const q of baked.quads) {
      if (q.cull >= 0 && this.culled(s, x, y, z, q.cull, state)) continue;
      const lit = this.smooth(s, x, y, z, q, true);
      const color = this.faceColor(b, state, q, x, z, baked.ao);
      const pass = this.passOf(b, q.layer);
      if (pass === 0 && lit.uniform) {
        greedy[q.dir].push({ x, y, z, q, color, ao: lit.ao[0], sky: lit.sky[0], blk: lit.blk[0], rnd: rot && q.dir < 2, variant });
        continue;
      }
      this.emit(pass, x, y, z, q, color, lit, rot && q.dir < 2);
    }
  }

  /** The model variant of the block at (x, y, z): tile entity keys, neighbour-dependent states, random variants. */
  private variantAt(s: SectionInput, x: number, y: number, z: number): { v: Variant; vs: Variant[]; rot: boolean } | null {
    const st = s.states, i = pidx(x, y, z), state = st[i];
    if (this.kind[state] === Kind.Unresolved) this.resolve(state);
    const k = this.kind[state];
    if (k === Kind.None || k === Kind.Fluid) return null;
    let vs = this.staticVariants[state]!;
    const b = this.block(state)!;
    const ex = s.extra?.get(i);
    if (ex !== undefined) {
      let cached = this.dynamicCache.get(`${state}|${ex}`);
      if (!cached) { cached = this.variantsFor(b, ex); this.dynamicCache.set(`${state}|${ex}`, cached); }
      vs = cached;
    } else if (k === Kind.Dynamic) {
      const key = decode(b.name, state & 15, (dx, dy, dz) => st[pidx(x + dx, y + dy, z + dz)], this.names, this.facts);
      let cached = this.dynamicCache.get(`${state}|${key}`);
      if (!cached) { cached = this.variantsFor(b, key); this.dynamicCache.set(`${state}|${key}`, cached); }
      vs = cached;
    }
    if (!vs.length) return null;
    const rot = ROT_ONLY.has(vs);
    return { v: vs.length > 1 && !rot ? pick(vs, s.wx + x, s.wy + y, s.wz + z) : vs[0], vs, rot };
  }

  /** The lid and latch quads of the chest at (x, y, z) of a section (block-local), for the page to draw opening. */
  lidOf(s: SectionInput, x: number, y: number, z: number): Quad[] {
    const at = this.variantAt(s, x, y, z);
    return at ? at.v.baked.quads.filter(isLid) : [];
  }

  private model(s: SectionInput, x: number, y: number, z: number, state: number, b: BlockData, baked: Baked): void {
    for (const q of baked.quads) {
      if (q.cull >= 0 && this.culled(s, x, y, z, q.cull, state)) continue;
      // inner faces (farmland/slab tops) take the light in front of them too: those blocks have none of their own
      const lit = baked.ao && isAxisAligned(q) ? this.smooth(s, x, y, z, q, true) : this.flat(s, x, y, z, q);
      this.emit(this.passOf(b, q.layer), x, y, z, q, this.faceColor(b, state, q, x, z, baked.ao), lit);
    }
  }

  private passOf(b: BlockData, layer: number): number {
    if (b.translucent) return 2;
    return this.layerAlpha[layer] ? 1 : 0;
  }

  private faceColor(b: BlockData, state: number, q: Quad, x: number, z: number, _ao: boolean): number {
    const t = q.tint >= 0 ? this.tint(b, state, x, z) : 0xffffff;
    const shade = q.shade && q.dir >= 0 ? DIR_SHADE[q.dir] : 1;
    if (shade === 1) return t;
    return (Math.round((t >> 16 & 255) * shade) << 16) | (Math.round((t >> 8 & 255) * shade) << 8) | Math.round((t & 255) * shade);
  }

  /**
   * Vanilla smooth lighting: per vertex, the 4 blocks in front of it (the face's neighbour and its
   * sideways/corner neighbours). Opaque samples weigh 0.2 for AO and lend the centre's light.
   * `outside`: sample the plane in front of the face (boundary faces) or the block's own plane (inner faces).
   */
  private smooth(s: SectionInput, x: number, y: number, z: number, q: Quad, outside: boolean): Lit {
    const n = DIR_VEC[q.dir];
    const cx = x + (outside ? n[0] : 0), cy = y + (outside ? n[1] : 0), cz = z + (outside ? n[2] : 0);
    const axis = n[0] ? 0 : n[1] ? 1 : 2;
    const ta = axis === 0 ? 1 : 0, tb = axis === 2 ? 1 : 2;
    const ci = pidx(cx, cy, cz);
    const cSky = this.light(s, s.sky, ci), cBlk = this.light(s, s.blk, ci);
    const lit: Lit = { ao: [0, 0, 0, 0], sky: [0, 0, 0, 0], blk: [0, 0, 0, 0], uniform: true };
    for (let v = 0; v < 4; v++) {
      const pa = q.pos[v * 3 + ta], pb = q.pos[v * 3 + tb];
      const sa = pa > 0.5 ? 1 : -1, sb = pb > 0.5 ? 1 : -1;
      const o1 = [cx, cy, cz]; o1[ta] += sa;
      const o2 = [cx, cy, cz]; o2[tb] += sb;
      const o3 = [cx, cy, cz]; o3[ta] += sa; o3[tb] += sb;
      const i1 = pidx(o1[0], o1[1], o1[2]), i2 = pidx(o2[0], o2[1], o2[2]), i3 = pidx(o3[0], o3[1], o3[2]);
      const s1 = this.solid(s.states[i1]), s2 = this.solid(s.states[i2]);
      const s3 = s1 && s2 ? true : this.solid(s.states[i3]);
      const ao = (1 + (s1 ? 0.2 : 1) + (s2 ? 0.2 : 1) + (s3 ? 0.2 : 1)) / 4;
      const sky = (cSky + (s1 ? cSky : this.light(s, s.sky, i1)) + (s2 ? cSky : this.light(s, s.sky, i2)) + (s3 ? cSky : this.light(s, s.sky, i3))) / 4;
      const blk = (cBlk + (s1 ? cBlk : this.light(s, s.blk, i1)) + (s2 ? cBlk : this.light(s, s.blk, i2)) + (s3 ? cBlk : this.light(s, s.blk, i3))) / 4;
      lit.ao[v] = Math.round(ao * 255);
      lit.sky[v] = Math.round(sky * 17);
      lit.blk[v] = Math.round(blk * 17);
      if (v > 0 && (lit.ao[v] !== lit.ao[0] || lit.sky[v] !== lit.sky[0] || lit.blk[v] !== lit.blk[0])) lit.uniform = false;
    }
    return lit;
  }

  /**
   * The light value at a padded index. Farmland, slabs and stairs store 0 (they block light) but are not
   * full cubes: like vanilla's useNeighbourLight they show the brightest of their six neighbours.
   */
  private light(s: SectionInput, arr: Uint8Array, i: number): number {
    const id = s.states[i] >> 4;
    if (!NEIGHBOUR_LIGHT[id]) return arr[i];
    const x = i % P, z = Math.floor(i / P) % P, y = Math.floor(i / (P * P));
    let v = arr[i];
    if (x > 0) v = Math.max(v, arr[i - 1]);
    if (x < P - 1) v = Math.max(v, arr[i + 1]);
    if (z > 0) v = Math.max(v, arr[i - P]);
    if (z < P - 1) v = Math.max(v, arr[i + P]);
    if (y > 0) v = Math.max(v, arr[i - P * P]);
    if (y < P - 1) v = Math.max(v, arr[i + P * P]);
    return v;
  }

  /** Plants and other see-through models: the light of the block itself, no AO. */
  private flat(s: SectionInput, x: number, y: number, z: number, _q: Quad): Lit {
    const i = pidx(x, y, z);
    let sky = this.light(s, s.sky, i), blk = this.light(s, s.blk, i);
    if (this.solid(s.states[i])) { const a = pidx(x, y + 1, z); sky = this.light(s, s.sky, a); blk = this.light(s, s.blk, a); }
    const sk = sky * 17, bl = blk * 17;
    return { ao: [255, 255, 255, 255], sky: [sk, sk, sk, sk], blk: [bl, bl, bl, bl], uniform: true };
  }

  /** `rnd`: the shader turns the texture per block (random variant rotation, see ROT_ONLY). */
  private emit(pass: number, x: number, y: number, z: number, q: Quad, color: number, lit: Lit, rnd = false): void {
    const vb = this.vb[pass];
    const r = color >> 16 & 255, g = color >> 8 & 255, bb = color & 255;
    const li = this.data.layers[q.layer];
    const frames = Math.min(127, li?.frames ?? 1) | (rnd ? 128 : 0), ft = Math.min(255, li?.frametime ?? 1);
    // split along the brighter diagonal (no AO streaks)
    const w = (v: number) => lit.ao[v] + lit.sky[v] * 0.5 + lit.blk[v] * 0.5;
    const start = w(0) + w(2) < w(1) + w(3) ? 1 : 0;
    for (let k = 0; k < 4; k++) {
      const v = (k + start) & 3;
      const a = lit.ao[v] / 255;
      vb.push(x + q.pos[v * 3], y + q.pos[v * 3 + 1], z + q.pos[v * 3 + 2], this.base[q.layer], q.uv[v * 2], q.uv[v * 2 + 1],
        Math.round(r * a), Math.round(g * a), Math.round(bb * a), lit.sky[v], lit.blk[v], frames, ft);
    }
  }

  /** Merges uniform full-cube faces into rectangles per direction and slice (identical faces only). */
  private greedy(faces: GreedyCell[][]): void {
    const vb = this.vb[0];
    for (let d = 0; d < 6; d++) {
      const list = faces[d];
      if (!list.length) continue;
      const n = DIR_VEC[d];
      const axis = n[0] ? 0 : n[1] ? 1 : 2;
      const ta = axis === 0 ? 1 : 0, tb = axis === 2 ? 1 : 2;
      // slice → 16×16 grid of cells
      const slices = new Map<number, Array<GreedyCell | undefined>>();
      for (const c of list) {
        const p = [c.x, c.y, c.z];
        let grid = slices.get(p[axis]);
        if (!grid) { grid = new Array(256); slices.set(p[axis], grid); }
        grid[p[ta] + p[tb] * 16] = c;
      }
      const same = (a: GreedyCell | undefined, b: GreedyCell): boolean => !!a && a.q.layer === b.q.layer && a.color === b.color && a.ao === b.ao
        && a.sky === b.sky && a.blk === b.blk && a.variant === b.variant && a.q === b.q && a.rnd === b.rnd;
      for (const [, grid] of slices) {
        for (let j = 0; j < 16; j++) for (let i = 0; i < 16; i++) {
          const c = grid[i + j * 16];
          if (!c) continue;
          let w = 1;
          while (i + w < 16 && same(grid[i + w + j * 16], c)) w++;
          let h = 1;
          outer: while (j + h < 16) {
            for (let k = 0; k < w; k++) if (!same(grid[i + k + (j + h) * 16], c)) break outer;
            h++;
          }
          for (let jj = 0; jj < h; jj++) for (let ii = 0; ii < w; ii++) grid[i + ii + (j + jj) * 16] = undefined;
          // stretch the unit quad: corners at max along ta/tb move by w-1/h-1, uvs extrapolate (REPEAT)
          const q = c.q;
          const li = this.data.layers[q.layer];
          const frames = Math.min(127, li?.frames ?? 1) | (c.rnd ? 128 : 0), ft = Math.min(255, li?.frametime ?? 1);
          const r = c.color >> 16 & 255, g = c.color >> 8 & 255, bb = c.color & 255, a = c.ao / 255;
          const du = uvGradient(q, ta), dv = uvGradient(q, tb);
          for (let v = 0; v < 4; v++) {
            const p = [q.pos[v * 3], q.pos[v * 3 + 1], q.pos[v * 3 + 2]];
            const ea = p[ta] > 0.5 ? w - 1 : 0, eb = p[tb] > 0.5 ? h - 1 : 0;
            p[ta] += ea; p[tb] += eb;
            const u = q.uv[v * 2] + du[0] * ea + dv[0] * eb, vv = q.uv[v * 2 + 1] + du[1] * ea + dv[1] * eb;
            vb.push(c.x + p[0], c.y + p[1], c.z + p[2], this.base[q.layer], u, vv, Math.round(r * a), Math.round(g * a), Math.round(bb * a), c.sky, c.blk, frames, ft);
          }
        }
      }
    }
  }

  // ---- fluids --------------------------------------------------------------------------------------

  private fluid(s: SectionInput, x: number, y: number, z: number, state: number): void {
    const b = this.block(state)!;
    const water = b.fluid === 'water';
    const same = (n: number) => { const nb = this.data.blocks[n >> 4]; return !!nb && nb.fluid === b.fluid; };
    const at = (dx: number, dy: number, dz: number) => s.states[pidx(x + dx, y + dy, z + dz)];
    const above = same(at(0, 1, 0));
    // vanilla getFluidHeight at a block corner (cx, cz ∈ {0, 1})
    const corner = (cx: number, cz: number): number => {
      let f = 0, n = 0;
      for (let j = 0; j < 4; j++) {
        const dx = cx - (j & 1), dz = cz - ((j >> 1) & 1);
        if (same(at(dx, 1, dz))) return 1;
        const st = at(dx, 0, dz);
        if (!same(st)) { if (!this.solid(st)) { f += 1; n++; } }
        else {
          const m = st & 15;
          const pct = 1 - fluidHeight(m);
          if (m >= 8 || m === 0) { f += pct * 10; n += 10; }
          f += pct; n++;
        }
      }
      return 1 - f / Math.max(1, n);
    };
    const h00 = above ? 1 : corner(0, 0), h01 = above ? 1 : corner(0, 1), h11 = above ? 1 : corner(1, 1), h10 = above ? 1 : corner(1, 0);
    const pass = water ? 2 : 0;
    const still = this.layerIndex(water ? 'blocks/water_still' : 'blocks/lava_still');
    const flow = this.layerIndex(water ? 'blocks/water_flow' : 'blocks/lava_flow');
    const tint = water ? this.waterCol[x + z * 16] : 0xffffff;
    const i = pidx(x, y, z), ia = pidx(x, y + 1, z);
    const sky = Math.max(s.sky[i], s.sky[ia]) * 17, blk = water ? Math.max(s.blk[i], s.blk[ia]) * 17 : 255;
    const vb = this.vb[pass];
    const put = (layer: number, verts: number[][], shade: number) => {
      const li = this.data.layers[layer];
      const r = Math.round((tint >> 16 & 255) * shade), g = Math.round((tint >> 8 & 255) * shade), bl = Math.round((tint & 255) * shade);
      // flow textures: frames | 128 makes the shader pick one of the frame's 2×2 slices by the uv (16 px per unit)
      const frames = Math.min(127, li?.frames ?? 1) | (layer === flow && (li?.frames ?? 1) > 1 ? 128 : 0);
      for (const v of verts) vb.push(x + v[0], y + v[1], z + v[2], this.base[layer], v[3], v[4], r, g, bl, sky, blk, frames, li?.frametime ?? 1);
    };
    const e = 0.001;
    if (!above) {
      const dir = flowAngle(state, at, same, (st) => !!this.data.blocks[st >> 4]?.collides);
      if (dir === null) {
        put(still, [[0, h00 - e, 0, 0, 0], [0, h01 - e, 1, 0, 1], [1, h11 - e, 1, 1, 1], [1, h10 - e, 0, 1, 0]], 1);
      } else {
        // FluidRenderer: the flow texture turned to the flow direction, 16 of its 32 px around the centre (uv unit = 16 px)
        const s19 = Math.sin(dir) * 0.5, c20 = Math.cos(dir) * 0.5;
        put(flow, [
          [0, h00 - e, 0, 1 - c20 - s19, 1 - c20 + s19], [0, h01 - e, 1, 1 - c20 + s19, 1 + c20 + s19],
          [1, h11 - e, 1, 1 + c20 + s19, 1 + c20 - s19], [1, h10 - e, 0, 1 + c20 - s19, 1 - c20 - s19],
        ], 1);
      }
      // seen from below (inside water): the same surface, reversed
      if (water) put(still, [[1, h10 - e, 0, 1, 0], [1, h11 - e, 1, 1, 1], [0, h01 - e, 1, 0, 1], [0, h00 - e, 0, 0, 0]], 1);
    }
    const below = at(0, -1, 0);
    if (!same(below) && !this.solid(below)) put(still, [[0, e, 1, 0, 1], [0, e, 0, 0, 0], [1, e, 0, 1, 0], [1, e, 1, 1, 1]], 0.5);
    // sides: [dir, corner a, corner b] with heights; quads from the bottom to the surface
    const sides: Array<[number, number, number, number, number, number, number, number]> = [
      // dx, dz, x0, z0, x1, z1, hA, hB
      [0, -1, 1, 0, 0, 0, h10, h00],
      [0, 1, 0, 1, 1, 1, h01, h11],
      [-1, 0, 0, 0, 0, 1, h00, h01],
      [1, 0, 1, 1, 1, 0, h11, h10],
    ];
    for (const [dx, dz, x0, z0, x1, z1, ha, hb] of sides) {
      const n = at(dx, 0, dz);
      if (same(n) || this.solid(n)) continue;
      const shade = dx ? 0.6 : 0.8;
      put(flow, [[x0, ha, z0, 0, 1 - ha], [x0, 0, z0, 0, 1], [x1, 0, z1, 1, 1], [x1, hb, z1, 1, 1 - hb]], shade);
      if (water) put(flow, [[x1, hb, z1, 1, 1 - hb], [x1, 0, z1, 1, 1], [x0, 0, z0, 0, 1], [x0, ha, z0, 0, 1 - ha]], shade);
    }
  }

  private layerIdx = new Map<string, number>();
  private layerIndex(name: string): number {
    let i = this.layerIdx.get(name);
    if (i === undefined) { i = Math.max(0, this.data.layers.findIndex(l => l.name === name)); this.layerIdx.set(name, i); }
    return i;
  }
}

/**
 * AbstractFluidBlock.getFluidVec → getDirection: the sum over the four sides of (their level − ours),
 * a side that is open and not liquid counting the liquid below it (level − (ours − 8)); the angle
 * atan2(z, x) − π/2 of that vector, or null when it is zero (the still texture).
 */
export function flowAngle(state: number, at: (dx: number, dy: number, dz: number) => number, same: (st: number) => boolean, blocks: (st: number) => boolean): number | null {
  const decay = (st: number) => (same(st) ? ((st & 15) >= 8 ? 0 : st & 15) : -1);
  const self = decay(state);
  let vx = 0, vz = 0;
  for (const [dx, dz] of [[0, -1], [1, 0], [0, 1], [-1, 0]]) {
    const n = at(dx, 0, dz);
    let j = decay(n);
    if (j < 0) {
      if (blocks(n)) continue;
      j = decay(at(dx, -1, dz));
      if (j >= 0) { const k = j - (self - 8); vx += dx * k; vz += dz * k; }
    } else { const k = j - self; vx += dx * k; vz += dz * k; }
  }
  if (vx === 0 && vz === 0) return null;
  return Math.atan2(vz, vx) - Math.PI / 2;
}

interface Lit { ao: number[]; sky: number[]; blk: number[]; uniform: boolean }
interface GreedyCell { x: number; y: number; z: number; q: Quad; color: number; ao: number; sky: number; blk: number; rnd: boolean; variant: number }

/** A chest's lid (y 9..14) and latch (y 7..11): every quad above 7/16; the base reaches down to 0. */
function isLid(q: Quad): boolean { return Math.min(q.pos[1], q.pos[4], q.pos[7], q.pos[10]) >= 7 / 16 - 1e-4; }
const lidlessCache = new WeakMap<Baked, Baked>();
function lidless(b: Baked): Baked {
  let out = lidlessCache.get(b);
  if (!out) { out = { ...b, quads: b.quads.filter(q => !isLid(q)) }; lidlessCache.set(b, out); }
  return out;
}

function isAxisAligned(q: Quad): boolean {
  if (q.dir < 0) return false;
  const n = DIR_VEC[q.dir];
  const axis = n[0] ? 0 : n[1] ? 1 : 2;
  const v0 = q.pos[axis];
  return Math.abs(q.pos[3 + axis] - v0) < 1e-4 && Math.abs(q.pos[6 + axis] - v0) < 1e-4 && Math.abs(q.pos[9 + axis] - v0) < 1e-4;
}

/** d(uv)/d(position along axis) of a unit quad: from two vertices that differ only along that axis. */
function uvGradient(q: Quad, axis: number): [number, number] {
  for (let a = 0; a < 4; a++) for (let b = 0; b < 4; b++) {
    if (a === b) continue;
    const dp = q.pos[b * 3 + axis] - q.pos[a * 3 + axis];
    if (Math.abs(dp) < 0.5) continue;
    let other = true;
    for (let k = 0; k < 3; k++) if (k !== axis && Math.abs(q.pos[b * 3 + k] - q.pos[a * 3 + k]) > 1e-4) other = false;
    if (!other) continue;
    return [(q.uv[b * 2] - q.uv[a * 2]) / dp, (q.uv[b * 2 + 1] - q.uv[a * 2 + 1]) / dp];
  }
  return [0, 0];
}

/** MathHelper.hashCode-like hash → weighted variant. */
function pick(vs: Variant[], x: number, y: number, z: number): Variant {
  let h = Math.imul(x, 3129871) ^ Math.imul(z, 116129781) ^ y;
  h = Math.imul(Math.imul(h, h), 42317861) + Math.imul(h, 11);
  let total = 0;
  for (const v of vs) total += v.w;
  let r = Math.abs(h >> 16) % total;
  for (const v of vs) { r -= v.w; if (r < 0) return v; }
  return vs[0];
}
