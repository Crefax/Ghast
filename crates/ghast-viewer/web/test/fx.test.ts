/**
 * Particles, explosions and lightning: the PARTICLES / EXPLOSION messages, the particle box (fxbox.ts),
 * the pool and the vanilla motions (particles.ts) and the bolt (lightning.ts: java.util.Random,
 * LightningEntityRenderer's shape, the strike cycle).
 */
import { describe, expect, it } from 'vitest';
import { FX_EVENT, messages, Reader, readExplosion, readParticles, S, writeExplosion, writeParticles, Writer, type ParticleMsg } from '../src/shared/protocol.js';
import type { BlockData } from '../src/shared/model-data.js';
import { BOX_X, BOX_Y, BOX_Z, emits, FxBox, LAVA, shapeOf, WATER } from '../src/fxbox.js';
import { INSTANCE, K, ParticleSystem, PT } from '../src/particles.js';
import { Bolt, boltTriangles, JavaRandom, Lightning } from '../src/lightning.js';

const msg = (frame: Uint8Array): Array<[number, Reader]> => [...messages(frame)];

describe('fx protocol', () => {
  it('round-trips PARTICLES and EXPLOSION (negative offsets, large coordinates)', () => {
    const list: ParticleMsg[] = [
      { id: 34, x: 1e6 + 0.25, y: 64.5, z: -30000.75, ox: 0.5, oy: 0.25, oz: 0, speed: 0.125, count: 7, data: 0 },
      { id: 37, x: 0, y: 0, z: 0, ox: 0, oy: 0, oz: 0, speed: 0, count: 0, data: (1 << 4) | 3 },
    ];
    const w = new Writer();
    writeParticles(w, list);
    writeExplosion(w, { x: 10.5, y: 64, z: -3.5, radius: 4, blocks: [[-4, 0, 3], [1, -2, -1]] });
    const [[t1, r1], [t2, r2]] = msg(w.done());
    expect(t1).toBe(S.PARTICLES);
    expect(readParticles(r1)).toEqual(list);
    expect(t2).toBe(S.EXPLOSION);
    expect(readExplosion(r2)).toEqual({ x: 10.5, y: 64, z: -3.5, radius: 4, blocks: [[-4, 0, 3], [1, -2, -1]] });
  });

});

// a tiny block table: air, stone (collides), water, lava, a slab, torches, snow layers
const blocks: Array<BlockData | null> = new Array(256).fill(null);
const def = (id: number, name: string, extra: Partial<BlockData> = {}) => { blocks[id] = { name, variants: {}, solidHint: false, ...extra }; };
def(1, 'stone', { collides: true, solidHint: true });
def(9, 'water', { fluid: 'water' });
def(11, 'lava', { fluid: 'lava' });
def(44, 'stone_slab');
def(50, 'torch');
def(55, 'redstone_wire');
def(78, 'snow_layer');

/** A box at the origin with a stone floor at y = 3 (the top at 4) and sky light above. */
function floorBox(): FxBox {
  const box = new FxBox();
  const light = new Uint8Array(BOX_X * BOX_Y * BOX_Z), shape = new Uint8Array(light.length);
  for (let y = 0; y < BOX_Y; y++) for (let z = 0; z < BOX_Z; z++) for (let x = 0; x < BOX_X; x++) {
    const i = x + BOX_X * (z + BOX_Z * y);
    if (y <= 3) shape[i] = 16; else light[i] = 15 << 4;
  }
  box.set(0, 0, 0, light, shape);
  return box;
}

describe('particle box', () => {
  it('collision shapes, liquids and which blocks emit', () => {
    expect(shapeOf(0, blocks)).toBe(0);
    expect(shapeOf(1 << 4, blocks)).toBe(16);
    expect(shapeOf(44 << 4, blocks)).toBe(8);
    expect(shapeOf((44 << 4) | 8, blocks)).toBe(16);
    expect(shapeOf((78 << 4) | 2, blocks)).toBe(6);
    expect(shapeOf(9 << 4, blocks)).toBe(WATER);
    expect(shapeOf(11 << 4, blocks)).toBe(LAVA);
    expect(shapeOf(50 << 4, blocks)).toBe(0);
    const column = (states: Record<number, number>) => (dy: number) => states[dy] ?? 0;
    expect(emits(50 << 4, column({}), blocks)).toBe(true);
    expect(emits(55 << 4, column({}), blocks)).toBe(false); // unpowered wire
    expect(emits((55 << 4) | 9, column({}), blocks)).toBe(true);
    expect(emits(11 << 4, column({}), blocks)).toBe(true); // lava under air pops
    expect(emits(11 << 4, column({ 1: 1 << 4 }), blocks)).toBe(false);
    expect(emits(9 << 4, column({ [-1]: 1 << 4 }), blocks)).toBe(true); // water over a ceiling with air below drips
    expect(emits(9 << 4, column({ [-1]: 1 << 4, [-2]: 1 << 4 }), blocks)).toBe(false);
    expect(emits(1 << 4, column({}), blocks)).toBe(false);
  });

  it('light at a point; a full block takes its brightest neighbour; outside is open sky', () => {
    const box = floorBox();
    expect(box.lightAt(5.5, 4.2, 5.5)).toBe(15 << 4);
    expect(box.lightAt(5.5, 3.5, 5.5)).toBe(15 << 4); // inside the floor: from the block above
    expect(box.lightAt(5.5, 1.5, 5.5)).toBe(0);
    expect(box.lightAt(-50, 4, 5, 15)).toBe(15 << 4);
    expect(box.blocked(5.5, 3.99, 5.5)).toBe(true);
    expect(box.blocked(5.5, 4.01, 5.5)).toBe(false);
    box.setShape(5, 3, 5, 0);
    expect(box.blocked(5.5, 3.5, 5.5)).toBe(false);
  });
});

/** A seeded Math.random stand-in (mulberry32). */
function seeded(seed = 1): () => number {
  let a = seed;
  return () => { a = (a + 0x6d2b79f5) | 0; let t = Math.imul(a ^ (a >>> 15), 1 | a); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
}
const sources = { block: (state: number) => (state ? { layer: 7, color: 0xffffff } : null), item: () => 3 };

function system(limit = 4000): ParticleSystem {
  const s = new ParticleSystem(limit, sources, seeded(7));
  s.box.set(0, 0, 0, floorBox().light!, floorBox().shape!);
  s.setCamera(10, 5, 10);
  return s;
}

describe('particles', () => {
  it('a broken block bursts into 64 fragments of its texture that fall, land on the floor and expire', () => {
    const s = system();
    s.breakBlock(10, 4, 10, 1 << 4);
    expect(s.count).toBe(64);
    expect(s.pool.slice(0, 64).every(p => p.kind === K.DIGGING && p.layer === 7)).toBe(true);
    let landed = 0;
    for (let t = 0; t < 200 && s.count; t++) {
      s.tick();
      for (let i = 0; i < s.count; i++) {
        const p = s.pool[i];
        expect(p.y).toBeGreaterThanOrEqual(4 - 1e-9); // never through the floor
        if (p.ground) landed++;
      }
    }
    expect(landed).toBeGreaterThan(0);
    expect(s.count).toBe(0);
  });

  it('the pool is fixed: spawns past the limit are dropped, dead particles leave their objects for reuse', () => {
    const s = system(100);
    for (let i = 0; i < 150; i++) s.spawn(PT.SMOKE, 10, 6, 10);
    expect(s.count).toBe(100);
    const objects = new Set(s.pool);
    for (let t = 0; t < 400 && s.count; t++) s.tick();
    expect(s.count).toBe(0);
    for (let i = 0; i < 50; i++) s.spawn(PT.FLAME, 10, 6, 10);
    expect(s.pool.slice(0, 50).every(p => objects.has(p))).toBe(true);
    expect(s.pool.length).toBe(100);
  });

  it('vanilla motions: smoke rises, a water drip falls and splashes, a bubble dies out of water, far spawns are dropped', () => {
    const s = system();
    s.spawn(PT.SMOKE, 10.5, 5, 10.5);
    const smoke = s.pool[0];
    for (let t = 0; t < 4; t++) s.tick();
    expect(smoke.y).toBeGreaterThan(5);
    s.count = 0;
    s.spawn(PT.DRIP_WATER, 10.5, 8, 10.5);
    let splashed = false;
    for (let t = 0; t < 200 && !splashed; t++) { s.tick(); splashed = s.pool.slice(0, s.count).some(p => p.kind === K.RAIN); }
    expect(splashed).toBe(true);
    s.count = 0;
    s.spawn(PT.BUBBLE, 10.5, 6, 10.5);
    s.tick();
    expect(s.count).toBe(0);
    s.spawn(PT.SMOKE, 100, 5, 100);
    expect(s.count).toBe(0);
    s.level = 0;
    s.spawn(PT.SMOKE, 10, 5, 10);
    s.breakBlock(10, 4, 10, 1 << 4);
    expect(s.count).toBe(0);
  });

  it('server particles: count 0 is one particle moving at speed × offset; otherwise count of them; events', () => {
    const s = system();
    s.fromServer({ id: PT.REDDUST, x: 10, y: 5, z: 10, ox: 0, oy: 1, oz: 0, speed: 1, count: 0, data: 0 });
    expect(s.count).toBe(1);
    expect(s.pool[0].kind).toBe(K.REDDUST);
    expect(s.pool[0].r).toBeGreaterThan(0.4); // red 0 means 1
    expect(s.pool[0].g).toBeGreaterThan(0.4); // green from the offset
    s.count = 0;
    s.fromServer({ id: 34, x: 10, y: 5, z: 10, ox: 0.5, oy: 0.5, oz: 0.5, speed: 0, count: 5, data: 0 });
    expect(s.count).toBe(5);
    s.count = 0;
    s.fromServer({ id: FX_EVENT + 4, x: 10, y: 4, z: 10, ox: 0, oy: 0, oz: 0, speed: 0, count: 1, data: 0 });
    expect(s.count).toBe(40); // spawner: 20 × smoke + flame
    s.count = 0;
    s.explosion({ x: 10, y: 5, z: 10, radius: 3, blocks: [[0, 0, 1], [1, 0, 0]] });
    expect(s.pool.slice(0, s.count).map(p => p.kind).sort()).toEqual([K.SMOKE, K.EXPLODE, K.SMOKE, K.EXPLODE, K.HUGE].sort());
    s.tick();
    expect(s.pool.slice(0, s.count).filter(p => p.kind === K.LARGE_EXPLODE).length).toBe(6); // the huge one emits 6 a tick
  });

  it('display ticks: a torch next to the camera gets ~0.244 calls a tick (smoke + flame each), lava pops, drips', () => {
    const s = system();
    s.emitters = new Int32Array([10, 5, 10, 50 << 4]);
    let spawned = 0;
    for (let t = 0; t < 1000; t++) { s.count = 0; s.ambient(); spawned += s.count; }
    expect(spawned / 2 / 1000).toBeGreaterThan(0.2);
    expect(spawned / 2 / 1000).toBeLessThan(0.29);
    s.count = 0;
    s.emitters = new Int32Array([30, 5, 30, 50 << 4]); // 20 blocks away: never
    for (let t = 0; t < 100; t++) s.ambient();
    expect(s.count).toBe(0);
  });

  it('rain splashes land on the surfaces near the camera (droplets on blocks, smoke on lava)', () => {
    const s = system();
    s.rain(1, () => 4);
    expect(s.count).toBeGreaterThan(50);
    expect(s.pool.slice(0, s.count).every(p => p.kind === K.RAIN && Math.abs(p.y - 4.1) < 1e-9)).toBe(true);
    s.count = 0;
    s.rain(1, () => -1);
    expect(s.count).toBe(0);
  });

  it('fill writes camera-relative interpolated instances with the uv of particles.png or the block piece', () => {
    const s = system();
    s.spawn(PT.FLAME, 10.5, 6, 10.5);
    s.breakBlock(12, 4, 12, 1 << 4);
    s.tick();
    const out = new Float32Array(100 * INSTANCE);
    const n = s.fill(out, 10, 5, 10, 0.5);
    expect(n).toBe(65);
    const flame = s.pool.findIndex(p => p.kind === K.FLAME);
    const p = s.pool[flame], o = flame * INSTANCE;
    expect(out[o]).toBeCloseTo(p.px + (p.x - p.px) * 0.5 - 10, 5);
    expect(out[o + 4]).toBeCloseTo(0); expect(out[o + 5]).toBeCloseTo(3 / 16); // index 48: column 0, row 3
    expect(out[o + 12]).toBe(-1);
    expect(out[o + 13]).toBe(1); // full bright
    const frag = s.pool.findIndex(q => q.kind === K.DIGGING);
    expect(out[frag * INSTANCE + 12]).toBe(7);
    expect(out[frag * INSTANCE + 6]).toBe(0.25);
  });
});

describe('lightning', () => {
  it('java.util.Random gives the JDK sequence', () => {
    expect(new JavaRandom(42n).nextLong()).toBe(-5025562857975149833n);
    const r = new JavaRandom(0n);
    const ints = Array.from({ length: 5 }, () => r.nextInt(11));
    expect(ints.every(v => v >= 0 && v < 11)).toBe(true);
    expect(new JavaRandom(0n).nextInt(16)).toBe(new JavaRandom(0n).nextInt(16));
  });

  it('the bolt ends at its block (x + 0.5) and is 128 blocks high, 4 × 14 segments', () => {
    const tri = boltTriangles(123456789n);
    expect(tri.length).toBe(4 * 14 * 8 * 3 * 3);
    let top = 0;
    for (let i = 0; i < tri.length; i += 3) {
      top = Math.max(top, tri[i + 1]);
      if (tri[i + 1] === 0) { expect(Math.abs(tri[i] - 0.5)).toBeLessThan(0.8); expect(Math.abs(tri[i + 2] - 0.5)).toBeLessThan(0.8); }
    }
    expect(top).toBe(128);
  });

  it('strikes 1..3 times, flashes the sky while striking, then goes', () => {
    const l = new Lightning();
    l.add(0, 64, 0);
    let flashed = 0, ticks = 0;
    for (; ticks < 100 && l.bolts.length; ticks++) { l.tick(); if (l.flash(0) > 0) flashed++; }
    expect(flashed).toBeGreaterThan(0);
    expect(l.bolts.length).toBe(0);
    for (let t = 0; t < 3; t++) l.tick();
    expect(l.flash(0)).toBe(0);
    const b = new Bolt(0, 0, 0, () => 0.99);
    expect(b.living).toBe(3);
  });
});
