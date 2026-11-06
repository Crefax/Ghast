/**
 * Block states (1.8 metadata + neighbours → blockstates keys) and the mesher on tiny worlds built from
 * the hand-made fixture (no real assets needed): culling, greedy merging, passes, ambient occlusion,
 * neighbour sections at the borders.
 */
import { describe, expect, it } from 'vitest';
import { decode } from '../src/shared/blocks18.js';
import { P, pidx, VERTEX_BYTES, type SectionInput } from '../src/mesher.js';
import { bake } from '../src/bake.js';
import { NAMES, facts, fixtureData, fixtureMesher, names } from './fixture.js';

/** decode() with neighbours from a small map of "dx,dy,dz" → [id, meta]. */
const dec = (name: string, meta: number, around: Record<string, [number, number]> = {}) =>
  decode(name, meta, (dx, dy, dz) => { const n = around[`${dx},${dy},${dz}`]; return n ? (n[0] << 4) | n[1] : 0; }, names, facts);

describe('block states (1.8 metadata → blockstates keys)', () => {
  it('decodes metadata properties', () => {
    expect(dec('wheat', 5)).toBe('age=5');
    expect(dec('farmland', 7)).toBe('moisture=7');
    expect(dec('log', 2 | 4)).toBe('v=2,axis=x');
    expect(dec('wool', 14)).toBe('v=14,normal');
    expect(dec('stone_slab', 8 | 3)).toBe('v=3,half=top');
    expect(dec('pumpkin', 1)).toBe('facing=west');
    expect(dec('torch', 5)).toBe('facing=up');
    expect(dec('cactus', 9)).toBe('normal');
  });

  it('computes the actual state from neighbours', () => {
    expect(dec('fence', 0, { '1,0,0': [85, 0], '0,0,-1': [1, 0] })).toBe('east=true,north=true,south=false,west=false');
    expect(dec('grass', 0, { '0,1,0': [78, 0] })).toBe('snowy=true');
    expect(dec('tripwire', 0, { '-1,0,0': [131, 3], '1,0,0': [132, 0] })).toBe('attached=false,east=true,north=false,south=false,suspended=false,west=true');
    expect(dec('tripwire', 0, { '-1,0,0': [131, 1] })).toContain('west=false');
    expect(dec('wooden_door', 8 | 1, { '0,-1,0': [64, 3 | 4] })).toBe('facing=north,half=upper,hinge=right,open=true');
    expect(dec('double_plant', 8, { '0,-1,0': [175, 2] })).toBe('v=2,half=upper');
    expect(dec('oak_stairs', 0)).toBe('facing=east,half=bottom,shape=straight');
    expect(dec('pumpkin_stem', 7, { '0,0,1': [86, 0] })).toBe('age=7,facing=south');
    expect(NAMES[86]).toBe('pumpkin');
  });
});

type TinyWorld = SectionInput & { set(x: number, y: number, z: number, id: number, meta?: number): void };
/** One section in open sky; set() stores light 0 in the block's cell, as the server does for opaque blocks. */
function world(): TinyWorld {
  const states = new Uint16Array(P * P * P), sky = new Uint8Array(P * P * P).fill(15), blk = new Uint8Array(P * P * P), biomes = new Uint8Array(P * P).fill(1);
  return { states, sky, blk, biomes, wx: 0, wy: 0, wz: 0, set(x, y, z, id, meta = 0) { const i = pidx(x, y, z); states[i] = (id << 4) | meta; sky[i] = 0; } };
}
const quads = (b: Uint8Array) => b.length / VERTEX_BYTES / 4;

interface Vert { x: number; y: number; z: number; r: number; sky: number; blk: number }
function quadsOf(buf: Uint8Array): Vert[][] {
  const u16 = new Uint16Array(buf.buffer, buf.byteOffset, buf.byteLength / 2);
  const vert = (v: number): Vert => { const o = v * VERTEX_BYTES; return { x: u16[o / 2] / 2048 - 1, y: u16[o / 2 + 1] / 2048 - 1, z: u16[o / 2 + 2] / 2048 - 1, r: buf[o + 12], sky: buf[o + 16], blk: buf[o + 17] }; };
  const out: Vert[][] = [];
  for (let q = 0; q < buf.length / VERTEX_BYTES / 4; q++) out.push([0, 1, 2, 3].map((k) => vert(q * 4 + k)));
  return out;
}
const near = (a: number, b: number) => Math.abs(a - b) < 1e-3;

describe('mesher', () => {
  it('culls hidden faces and merges flat faces (greedy)', () => {
    const m = fixtureMesher();
    const s = world();
    s.set(4, 4, 4, 5);
    expect(quads(m.mesh(s).solid)).toBe(6);
    s.set(5, 4, 4, 5);
    expect(quads(m.mesh(s).solid)).toBe(6); // the shared faces are gone, the other faces merge in pairs
    const f = world();
    for (let x = 0; x < 4; x++) for (let z = 0; z < 4; z++) f.set(x, 4, z, 5);
    expect(quads(m.mesh(f).solid)).toBe(2 + 4);
  });

  it('a block surrounded on all sides makes no faces; a 3×3×3 cube of stone makes 6 merged faces', () => {
    const m = fixtureMesher();
    const s = world();
    for (let x = 3; x < 6; x++) for (let y = 3; y < 6; y++) for (let z = 3; z < 6; z++) s.set(x, y, z, 1);
    expect(quads(m.mesh(s).solid)).toBe(6);
  });

  it('an empty section and a section of only air make no geometry', () => {
    const out = fixtureMesher().mesh(world());
    expect(out.solid.length + out.cutout.length + out.translucent.length).toBe(0);
  });

  it('culls against the padded neighbour section at every border', () => {
    const m = fixtureMesher();
    const s = world();
    s.set(0, 8, 8, 1);
    expect(quads(m.mesh(s).solid)).toBe(6);
    s.set(-1, 8, 8, 1); // the neighbour section's block at x = −1 hides the west face
    expect(quads(m.mesh(s).solid)).toBe(5);
    s.set(15, 0, 15, 1);
    s.set(15, -1, 15, 1); s.set(16, 0, 15, 1); s.set(15, 0, 16, 1);
    expect(quads(m.mesh(s).solid)).toBe(5 + 3);
  });

  it('puts crops in the cutout pass, water in the translucent pass, and skips faces between glass', () => {
    const m = fixtureMesher();
    const s = world();
    s.set(1, 4, 1, 59, 7);
    s.set(3, 4, 3, 9, 0);
    s.set(6, 4, 6, 20); s.set(7, 4, 6, 20);
    const out = m.mesh(s);
    expect(quads(out.cutout)).toBe(8 + 10); // wheat: 4 planes from both sides; two glass blocks: 10 faces
    expect(quads(out.translucent)).toBeGreaterThanOrEqual(6);
  });

  it('darkens corners next to solid blocks (ambient occlusion) and reads the light in front of a face', () => {
    const m = fixtureMesher();
    const s = world();
    s.set(4, 4, 4, 5); s.set(5, 5, 4, 5);
    const top = quadsOf(m.mesh(s).solid).find(vs => vs.every(v => near(v.y, 5) && v.x > 3.99 && v.x < 5.01 && v.z > 3.99 && v.z < 5.01));
    expect(top).toHaveLength(4);
    const east = top!.filter(v => near(v.x, 5)), west = top!.filter(v => near(v.x, 4));
    expect(Math.max(...east.map(v => v.r))).toBeLessThan(Math.min(...west.map(v => v.r)));
    expect(west.every(v => v.sky === 255)).toBe(true);
  });

  it('bakes the crop as four planes that never cull', () => {
    const d = fixtureData();
    const b = bake(d.models[3], {});
    expect(b.fullCube).toBe(false);
    expect(b.quads.every(q => q.cull === -1)).toBe(true);
    expect(bake(d.models[0], {}).fullCube).toBe(true);
  });
});
