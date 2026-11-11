/**
 * The wire format (shared/protocol.ts): chunk columns (palette widths, light, empty sections, signs,
 * tiles), framing, strings, readers over sub-arrays.
 */
import zlib from 'node:zlib';
import { describe, expect, it } from 'vitest';
import { C, Reader, S, Writer, decodeColumn, encodeColumn, messages, nibble } from '../src/shared/protocol.js';

/** Deterministic PRNG (mulberry32). */
function rng(seed: number): () => number {
  return () => {
    seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** A 1.8 section buffer: 4096 × u16 LE states, block light, sky light. */
function sectionData(states: (i: number) => number, sky: (i: number) => number, block: (i: number) => number, withSky = true): Uint8Array {
  const d = new Uint8Array(withSky ? 12288 : 10240);
  const dv = new DataView(d.buffer);
  for (let i = 0; i < 4096; i++) dv.setUint16(i * 2, states(i), true);
  for (let i = 0; i < 4096; i++) {
    const b = block(i) & 15, s = sky(i) & 15;
    d[8192 + (i >> 1)] |= i & 1 ? b << 4 : b;
    if (withSky) d[10240 + (i >> 1)] |= i & 1 ? s << 4 : s;
  }
  return d;
}
/** Section index of x, y, z (y within the section). */
const at = (x: number, y: number, z: number) => x + 16 * (z + 16 * y);

describe('protocol: columns', () => {
  it('round-trips a column: palette + bit packing, light, empty sections left out', () => {
    // section 0: bedrock, dirt, farmland 7 under wheat of varying age; sky light from y 4 up. Section 2: one lit stone.
    const s0 = sectionData((i) => {
      const y = i >> 8, x = i & 15, z = (i >> 4) & 15;
      return y === 0 ? 7 << 4 : y < 3 ? 3 << 4 : y === 3 ? (60 << 4) | 7 : y === 4 ? (59 << 4) | ((x + z) % 8) : 0;
    }, (i) => ((i >> 8) >= 4 ? 15 : 0), () => 0);
    const s2 = sectionData((i) => (i === at(3, 8, 5) ? 1 << 4 : 0), () => 15, (i) => (i === at(3, 8, 5) ? 7 : 0));
    const biomes = new Uint8Array(256); biomes[2 + 16 * 2] = 4;
    const sections: Array<Uint8Array | null> = new Array(16).fill(null);
    sections[0] = s0; sections[2] = s2;
    const body = encodeColumn(sections, biomes, true);
    expect(body.length).toBeLessThan(8000);
    const col = decodeColumn(1, -2, body);
    expect(col.sections.map(s => s.y)).toEqual([0, 2]);
    const a = col.sections[0];
    expect(a.states[at(5, 4, 6)]).toBe((59 << 4) | ((5 + 6) % 8));
    expect(a.states[0]).toBe(7 << 4);
    expect(nibble(a.sky!, at(5, 4, 6))).toBe(15);
    expect(nibble(a.sky!, at(5, 2, 6))).toBe(0);
    const b = col.sections[1];
    expect(b.states[at(3, 8, 5)]).toBe(1 << 4);
    expect(nibble(b.block!, at(3, 8, 5))).toBe(7);
    expect(b.skyFull).toBe(true);
    expect(b.sky).toBeNull();
    expect(col.biomes[2 + 16 * 2]).toBe(4);
    // deflated it is tiny (what goes over the wire)
    expect(zlib.deflateRawSync(body).length).toBeLessThan(700);
  });

  it('round-trips every palette width (0, 1, 2, 4, 8, 16 bits)', () => {
    const r = rng(7);
    for (const n of [1, 2, 3, 4, 5, 16, 17, 256, 257, 900]) {
      const palette = Array.from({ length: n }, (_, k) => ((k % 4000) + 1) << 4 | (k % 16));
      const states = Array.from({ length: 4096 }, (_, i) => palette[i < n ? i : Math.floor(r() * n)]);
      const d = sectionData((i) => states[i], () => 15, () => 0);
      const col = decodeColumn(0, 0, encodeColumn([d], new Uint8Array(256), true));
      expect(col.sections).toHaveLength(1);
      expect(Array.from(col.sections[0].states)).toEqual(states);
    }
  });

  it('keeps light exact: full sky / dark sky / mixed, block light, no-sky dimensions and short buffers', () => {
    const r = rng(11);
    const sky = Array.from({ length: 4096 }, () => Math.floor(r() * 16));
    const blk = Array.from({ length: 4096 }, () => Math.floor(r() * 16));
    const stone = () => 1 << 4;
    const mixed = sectionData(stone, (i) => sky[i], (i) => blk[i]);
    const full = sectionData(stone, () => 15, () => 0);
    const dark = sectionData(stone, () => 0, () => 0);
    const col = decodeColumn(0, 0, encodeColumn([mixed, full, dark], new Uint8Array(256), true));
    const [a, b, c] = col.sections;
    for (let i = 0; i < 4096; i += 37) { expect(nibble(a.sky!, i)).toBe(sky[i]); expect(nibble(a.block!, i)).toBe(blk[i]); }
    expect(b.sky).toBeNull(); expect(b.skyFull).toBe(true); expect(b.block).toBeNull();
    expect(c.sky).toBeNull(); expect(c.skyFull).toBe(false);
    const nether = decodeColumn(0, 0, encodeColumn([full], new Uint8Array(256), false));
    expect(nether.hasSky).toBe(false);
    expect(nether.sections[0].sky).toBeNull(); expect(nether.sections[0].skyFull).toBe(false);
    const short = decodeColumn(0, 0, encodeColumn([sectionData(stone, () => 0, (i) => blk[i], false)], new Uint8Array(256), true));
    expect(short.sections[0].sky).toBeNull();
    expect(nibble(short.sections[0].block!, 100)).toBe(blk[100]);
  });

  it('leaves out all-air sections, keeps the y of the others, and pads short biome arrays', () => {
    const r = rng(3);
    const sections: Array<Uint8Array | null> = new Array(16).fill(null);
    const ys = [0, 5, 15];
    for (const y of ys) sections[y] = sectionData(() => (Math.floor(r() * 3) + 1) << 4, () => 15, () => 0);
    sections[7] = new Uint8Array(12288);
    const col = decodeColumn(-3, 9, encodeColumn(sections, new Uint8Array(10), true));
    expect(col.sections.map((s) => s.y)).toEqual(ys);
    expect([col.cx, col.cz]).toEqual([-3, 9]);
    expect(col.biomes).toHaveLength(256);
  });

  it('carries sign text (UTF-8) with the column', () => {
    const lines = ['Shop', '', 'café ⛏', ''];
    const body = encodeColumn(new Array(16).fill(null), new Uint8Array(256), true, [{ x: 3, y: 70, z: 9, lines }]);
    expect(decodeColumn(0, 0, body).signs).toEqual([{ x: 3, y: 70, z: 9, lines }]);
  });
});

describe('protocol: frames', () => {
  it('frames carry several typed messages; a truncated one throws', () => {
    const w = new Writer(4);
    let a = w.begin(S.POSE); w.f64(123.5).u32(7); w.end(a);
    a = w.begin(S.REMOVE); w.u16(1).u32(99); w.end(a);
    a = w.begin(S.STATE); w.str('x'); w.end(a);
    const got = [...messages(w.done())].map(([t, r]) => [t, r.end - r.pos]);
    expect(got).toEqual([[S.POSE, 12], [S.REMOVE, 6], [S.STATE, 3]]);
    expect(() => [...messages(w.done().subarray(0, 10))]).toThrow(/truncated/);
  });

  it('writes UTF-8 strings by byte length and grows the writer across big payloads', () => {
    const w = new Writer(2);
    const a = w.begin(S.SPAWN);
    w.u32(1).str('Ünïcødé_ğüşıöİ ⛏').f64(-1.25).str('');
    const big = new Uint8Array(20_000).map((_, i) => i & 255);
    w.bytes(big);
    w.end(a);
    const [[type, rd]] = [...messages(w.done())];
    expect(type).toBe(S.SPAWN);
    expect(rd.u32()).toBe(1);
    expect(rd.str()).toBe('Ünïcødé_ğüşıöİ ⛏');
    expect(rd.f64()).toBe(-1.25);
    expect(rd.str()).toBe('');
    expect(rd.rest()).toEqual(big);
  });

  it('a reader over a sub-array reads from its offset (frames inside larger buffers)', () => {
    const w = new Writer(16);
    w.u8(9).i32(-123456).f32(0.5);
    const inner = w.done();
    const outer = new Uint8Array(inner.length + 7);
    outer.set(inner, 7);
    const r = new Reader(outer.subarray(7));
    expect([r.u8(), r.i32(), r.f32()]).toEqual([9, -123456, 0.5]);
  });

  it('message type numbers are fixed (the Rust server uses the same)', () => {
    expect(S).toEqual({
      HELLO: 1, STATE: 2, CHUNK: 3, UNLOAD: 4, BLOCKS: 5, POSE: 6, SPAWN: 7, MOVES: 8, REMOVE: 9, EQUIP: 10, INVENTORY: 11, VITALS: 12,
      TIME: 13, PONG: 14, RESET: 15, CRACKS: 16, WEATHER: 17, EVENTS: 18, ESTATE: 19, COLLECT: 20, CHESTS: 21, SKIN: 22, LOOKS: 23,
      PARTICLES: 24, EXPLOSION: 25, LIGHTNING: 26, REPLAY: 27, ITEMS: 28, ENTITY: 29,
    });
    expect(C).toEqual({ PING: 1, ACK: 2, REPLAY: 3 });
  });
});
