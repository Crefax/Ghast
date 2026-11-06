/**
 * Block models → quads, as Minecraft 1.8's BakedQuadFactory does: element rotation (with rescale), the
 * variant's x/y rotation in 90° steps, uvlock, per-face uv rotation, cullfaces rotated with the model.
 * Positions are block-local (0..1), uvs in texture tiles (0..1 = one 16×16 layer).
 */
import type { Element, Model, VariantRef } from './shared/model-data.js';

export const DIRS = ['down', 'up', 'north', 'south', 'west', 'east'] as const;
export type DirName = typeof DIRS[number];
export const DIR_VEC: ReadonlyArray<[number, number, number]> = [[0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]];
/** Vanilla face shading (Block.getAmbientOcclusionLightLevel aside): down .5, up 1, north/south .8, west/east .6. */
export const DIR_SHADE = [0.5, 1, 0.8, 0.8, 0.6, 0.6];

export interface Quad {
  pos: Float32Array; // 4 × xyz
  uv: Float32Array; // 4 × uv
  layer: number;
  dir: number;
  cull: number;
  tint: number;
  shade: boolean;
  /** The quad lies on the block's face plane (smooth light from the neighbour, culling applies). */
  boundary: boolean;
}
export interface Baked { quads: Quad[]; ao: boolean; fullCube: boolean }

const dirIndex = (d: string | undefined): number => (d ? DIRS.indexOf(d as DirName) : -1);

/** Vertex corners per face (CubeFace), as [x, y, z] picks: 0 = min, 1 = max. */
const CORNERS: number[][][] = [
  [[0, 0, 1], [0, 0, 0], [1, 0, 0], [1, 0, 1]], // down
  [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]], // up
  [[1, 1, 0], [1, 0, 0], [0, 0, 0], [0, 1, 0]], // north
  [[0, 1, 1], [0, 0, 1], [1, 0, 1], [1, 1, 1]], // south
  [[0, 1, 0], [0, 0, 0], [0, 0, 1], [0, 1, 1]], // west
  [[1, 1, 1], [1, 0, 1], [1, 0, 0], [1, 1, 0]], // east
];

/** ModelElement default uvs (16ths) from the element bounds. */
export function defaultUv(dir: number, from: ArrayLike<number>, to: ArrayLike<number>): number[] {
  switch (dir) {
    case 0: return [from[0], 16 - to[2], to[0], 16 - from[2]];
    case 1: return [from[0], from[2], to[0], to[2]];
    case 2: return [16 - to[0], 16 - to[1], 16 - from[0], 16 - from[1]];
    case 3: return [from[0], 16 - to[1], to[0], 16 - from[1]];
    case 4: return [from[2], 16 - to[1], to[2], 16 - from[1]];
    default: return [16 - to[2], 16 - to[1], 16 - from[2], 16 - from[1]];
  }
}

function rotX(v: number[], steps: number): void {
  for (let i = 0; i < steps; i++) { const y = v[1], z = v[2]; v[1] = z; v[2] = -y; }
}
function rotY(v: number[], steps: number): void {
  for (let i = 0; i < steps; i++) { const x = v[0], z = v[2]; v[0] = -z; v[2] = x; }
}
function rotateDir(d: number, xs: number, ys: number): number {
  if (d < 0) return d;
  const v = [...DIR_VEC[d]];
  rotX(v, xs); rotY(v, ys);
  return DIR_VEC.findIndex(w => w[0] === Math.round(v[0]) && w[1] === Math.round(v[1]) && w[2] === Math.round(v[2]));
}

export function bake(model: Model, ref: Pick<VariantRef, 'x' | 'y' | 'uvlock'>): Baked {
  const xs = (((ref.x ?? 0) / 90) % 4 + 4) % 4, ys = (((ref.y ?? 0) / 90) % 4 + 4) % 4;
  const quads: Quad[] = [];
  for (const e of model.el) for (let d = 0; d < 6; d++) {
    const f = e.faces[DIRS[d]];
    if (!f) continue;
    quads.push(bakeFace(e, d, f, xs, ys, !!ref.uvlock));
  }
  // full cubes: every element is the whole block with culled faces (grass = a cube plus its tinted side overlay)
  const fullCube = model.el.length > 0 && model.el.every(e => e.from.every(v => v === 0) && e.to.every(v => v === 16) && !e.rot
    && Object.values(e.faces).every(f => f?.cull !== undefined)) && DIRS.every(d => model.el[0].faces[d]);
  return { quads, ao: model.ao, fullCube };
}

function bakeFace(e: Element, d: number, f: NonNullable<Element['faces'][DirName]>, xs: number, ys: number, uvlock: boolean): Quad {
  const from = e.from, to = e.to;
  const pos = new Float32Array(12);
  const corners = CORNERS[d];
  for (let i = 0; i < 4; i++) {
    const c = corners[i];
    const p = [c[0] ? to[0] : from[0], c[1] ? to[1] : from[1], c[2] ? to[2] : from[2]];
    if (e.rot) rotateElement(p, e.rot);
    // variant rotation about the block centre
    const v = [p[0] / 16 - 0.5, p[1] / 16 - 0.5, p[2] / 16 - 0.5];
    rotX(v, xs); rotY(v, ys);
    pos[i * 3] = v[0] + 0.5; pos[i * 3 + 1] = v[1] + 0.5; pos[i * 3 + 2] = v[2] + 0.5;
  }
  const dir = rotateDir(d, xs, ys);
  let uvs = f.uv ?? defaultUv(d, from, to);
  if (uvlock && (xs || ys) && !e.rot) {
    // world-aligned uvs: the default mapping of the rotated face
    const mn = [Infinity, Infinity, Infinity], mx = [-Infinity, -Infinity, -Infinity];
    for (let i = 0; i < 4; i++) for (let a = 0; a < 3; a++) { mn[a] = Math.min(mn[a], pos[i * 3 + a] * 16); mx[a] = Math.max(mx[a], pos[i * 3 + a] * 16); }
    uvs = defaultUv(dir, mn, mx);
  }
  // getVertexU/V with the face rotation: vertex i takes corner (i + r/90) % 4 of [u1 v1, u1 v2, u2 v2, u2 v1]
  const uv = new Float32Array(8);
  const r = ((f.r ?? 0) / 90) | 0;
  // uvlock re-derived the uvs for the new direction: pick corners of that direction's vertex order
  const order = uvlock && (xs || ys) && !e.rot ? reorderForDir(pos, dir) : null;
  for (let i = 0; i < 4; i++) {
    const k = (i + r) % 4;
    uv[i * 2] = (k === 0 || k === 1 ? uvs[0] : uvs[2]) / 16;
    uv[i * 2 + 1] = (k === 0 || k === 3 ? uvs[1] : uvs[3]) / 16;
  }
  if (order) {
    // pos was produced in the original face's corner order; match uvs to the rotated face's canonical corners
    const fixed = new Float32Array(8);
    for (let i = 0; i < 4; i++) { fixed[i * 2] = uv[order[i] * 2]; fixed[i * 2 + 1] = uv[order[i] * 2 + 1]; }
    uv.set(fixed);
  }
  let boundary = false;
  if (!e.rot && dir >= 0) {
    const [dx, dy, dz] = DIR_VEC[dir];
    const axis = dx ? 0 : dy ? 1 : 2, side = dx + dy + dz > 0 ? 1 : 0;
    boundary = [0, 1, 2, 3].every(i => Math.abs(pos[i * 3 + axis] - side) < 1e-4);
  }
  return { pos, uv, layer: f.t, dir, cull: rotateDir(f.cull ? DIRS.indexOf(f.cull) : -1, xs, ys), tint: f.tint ?? -1, shade: e.shade !== false, boundary };
}

/** For each vertex of `pos`, which canonical corner (CubeFace order of `dir`) it is. */
function reorderForDir(pos: Float32Array, dir: number): number[] {
  const mn = [Infinity, Infinity, Infinity], mx = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < 4; i++) for (let a = 0; a < 3; a++) { mn[a] = Math.min(mn[a], pos[i * 3 + a]); mx[a] = Math.max(mx[a], pos[i * 3 + a]); }
  const out: number[] = [];
  for (let i = 0; i < 4; i++) {
    let best = 0, bestD = Infinity;
    for (let k = 0; k < 4; k++) {
      const c = CORNERS[dir][k];
      let dd = 0;
      for (let a = 0; a < 3; a++) dd += Math.abs(pos[i * 3 + a] - (c[a] ? mx[a] : mn[a]));
      if (dd < bestD) { bestD = dd; best = k; }
    }
    out.push(best);
  }
  return out;
}

function rotateElement(p: number[], rot: NonNullable<Element['rot']>): void {
  const a = (rot.angle * Math.PI) / 180, c = Math.cos(a), s = Math.sin(a);
  const o = rot.o;
  const x = p[0] - o[0], y = p[1] - o[1], z = p[2] - o[2];
  let nx = x, ny = y, nz = z;
  if (rot.axis === 'x') { ny = y * c - z * s; nz = y * s + z * c; }
  else if (rot.axis === 'y') { nx = x * c + z * s; nz = -x * s + z * c; }
  else { nx = x * c - y * s; ny = x * s + y * c; }
  if (rot.rescale) {
    const k = 1 / Math.cos(Math.abs(a));
    if (rot.axis !== 'x') nx *= k;
    if (rot.axis !== 'y') ny *= k;
    if (rot.axis !== 'z') nz *= k;
  }
  p[0] = nx + o[0]; p[1] = ny + o[1]; p[2] = nz + o[2];
}
