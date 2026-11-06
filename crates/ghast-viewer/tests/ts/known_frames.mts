// Known-answer frames from the TypeScript encoder (web/src/shared/protocol.ts), for the Rust tests
// in tests/known_frames.rs. Regenerate with:
//   node --experimental-transform-types tests/ts/known_frames.mts > tests/fixtures/ts_frames.json
// Inputs are synthetic; the same cases are rebuilt in Rust and must give the same bytes.
import { S, Writer, encodeColumn, mobVariant, writeExplosion, writeLooks, writeParticles, NO_COLOR, SELF } from '../../web/src/shared/protocol.ts';

const hex = (b: Uint8Array) => Buffer.from(b).toString('hex');
const out: Record<string, string> = {};

// one deterministic section: states, block light, sky light (prismarine layout)
function section(seed: number, kinds: number, light: 'none' | 'data' | 'full'): Uint8Array {
  const d = new Uint8Array(12288);
  const v = new DataView(d.buffer);
  let x = seed;
  for (let i = 0; i < 4096; i++) {
    x = (Math.imul(x, 1103515245) + 12345) >>> 0;
    v.setUint16(i * 2, (((x >>> 16) % kinds) + 1) << 4 | (i & 3), true);
  }
  if (light === 'data') for (let i = 0; i < 2048; i++) { d[8192 + i] = (i * 7) & 255; d[10240 + i] = 255 - (i & 15); }
  if (light === 'full') d.fill(0xff, 10240);
  return d;
}
const biomes = new Uint8Array(256).map((_, i) => i % 7);
const sections: Array<Uint8Array | null> = new Array(16).fill(null);
sections[0] = section(1, 1, 'full');
sections[1] = section(2, 2, 'data');
sections[3] = section(3, 4, 'none');
sections[4] = section(4, 9, 'full');
sections[5] = section(5, 100, 'data');
sections[6] = section(6, 300, 'full');
sections[7] = new Uint8Array(12288); // all air: left out
out.column = hex(encodeColumn(sections, biomes, true,
  [{ x: 1, y: 64, z: 2, lines: ['Hello', '', 'ünï', 'x'.repeat(70)] }],
  [
    { x: 3, y: 65, z: 4, kind: 'skull', skull: 3, rot: 9, skin: 'skins/abc.png' },
    { x: 5, y: 66, z: 6, kind: 'banner', base: 14, patterns: [['bri', 1], ['cr', 15]] },
    { x: 7, y: 67, z: 8, kind: 'pot', contents: 'red_tulip' },
  ]));
out.columnNoSky = hex(encodeColumn([section(9, 3, 'data'), ...new Array(15).fill(null)], new Uint8Array(0), false));

let w = new Writer(64);
writeLooks(w, 42, { flags: 1, armour: [[1, 301, 0, 0xa06540], [4, 310, 0, NO_COLOR]] });
writeLooks(w, SELF, { flags: 0, armour: [], pose: [1, 2, 3, -10, 0, -10, -15, 0, 10, -1, 0, -1, 1, 0, 1, 0.1, 0.2, 0.3] });
out.looks = hex(w.done());

w = new Writer(64);
writeParticles(w, [{ id: 37, x: 1.5, y: 64.1, z: -3.3, ox: 0.1, oy: 0.2, oz: 0.3, speed: 0.05, count: 12, data: (1 << 4) | 2 },
  { id: 104, x: -100.25, y: 5, z: 1e6, ox: 0, oy: 0, oz: 0, speed: 0, count: 1, data: 7 }]);
writeExplosion(w, { x: 0.5, y: 70.3, z: 0.5, radius: 4, blocks: [[1, -1, 0], [-128, 127, 3]] });
out.fx = hex(w.done());

// the inline messages of feed.ts / stream.ts / record.ts, with the same field order
w = new Writer(64);
let at = w.begin(S.POSE); w.f64(1730000000123.5).u32(4567).f64(100.5).f64(64).f64(-200.25).f32(3.14159265).f32(-0.5).u8(1 | 8 | 16).f32(0.1).f32(-0.0784).f32(0); w.end(at);
at = w.begin(S.SPAWN); w.u32(12).u8(3).str('PigZombie').str('Zombie Pigman').f64(1.5).f64(2.5).f64(3.5).f32(1).f32(2).f32(3).f32(0.6).f32(1.95).u16(283).u16(0).u16(0); w.end(at);
at = w.begin(S.MOVES); w.u16(2); w.u32(12).f64(1).f64(2).f64(3).f32(0.5).f32(0.25).f32(0.125); w.u32(13).f64(-1).f64(-2).f64(-3).f32(6.2).f32(-1.5).f32(6.2); w.end(at);
at = w.begin(S.REMOVE); w.u16(1).u32(12); w.end(at);
at = w.begin(S.BLOCKS); w.u16(2); w.i32(-17).u8(64).i32(33).u16((1 << 4) | 0); w.i32(5).u8(255).i32(-1).u16(0); w.end(at);
at = w.begin(S.EQUIP); w.u32(13).u16(276).u16(0); w.end(at);
at = w.begin(S.INVENTORY); w.u8(3); for (let i = 0; i < 9; i++) w.u16(i ? 0 : 278).u16(i === 2 ? 5 : 0).u8(i ? 0 : 1); w.end(at);
at = w.begin(S.VITALS); w.f32(19.5).u8(17).f32(2.5).u8(30).f32(0.42); w.end(at);
at = w.begin(S.TIME); w.f64(1730000000999).f64(123456789).f64(-6000); w.end(at);
at = w.begin(S.PONG); w.f64(55.5).f64(1730000001000); w.end(at);
at = w.begin(S.RESET); w.end(at);
at = w.begin(S.UNLOAD); w.i32(-4).i32(9); w.end(at);
at = w.begin(S.CRACKS); w.u16(1); w.i32(10).u8(70).i32(-10).u16((56 << 4)).u8(4); w.end(at);
at = w.begin(S.WEATHER); w.f32(0.7).f32(0); w.end(at);
at = w.begin(S.EVENTS); w.u16(2); w.u32(SELF).u8(0); w.u32(13).u8(1); w.end(at);
at = w.begin(S.ESTATE); w.u16(1); w.u32(13).u8(2 | 4).u32(15); w.end(at);
at = w.begin(S.COLLECT); w.u32(99).u32(SELF); w.end(at);
at = w.begin(S.CHESTS); w.u16(1); w.i32(1).u8(64).i32(2).u8(1); w.end(at);
at = w.begin(S.SKIN); w.u32(13).u8(1).str('skins/0123.png').str(''); w.end(at);
at = w.begin(S.LIGHTNING); w.f64(1).f64(2).f64(3); w.end(at);
at = w.begin(S.REPLAY); w.f64(1500).f64(60000).f32(2).f64(1730000000000); w.end(at);
at = w.begin(S.CHUNK); w.i32(-1).i32(2).u8(1).bytes(new Uint8Array([1, 2, 3])); w.end(at);
out.inline = hex(w.done());

// mobVariant over a table of names and metadata
const cases: Array<[string, Record<number, unknown>]> = [
  ['Slime', { 16: 4 }], ['LavaSlime', { 16: 300 }], ['Villager', { 16: 2 }], ['Sheep', { 16: 16 | 5 }], ['Sheep', { 16: 255 }],
  ['EntityHorse', { 19: 3, 20: 0x102 }], ['Skeleton', { 13: 1 }], ['Zombie', { 13: 1 }], ['Wolf', { 16: 4 | 1, 20: 14 }],
  ['Ozelot', { 18: 2 }], ['Rabbit', { 18: 99 }], ['Rabbit', { 18: 9 }], ['Bat', { 16: 1 }], ['Ghast', { 16: 0 }],
  ['Guardian', { 16: 4 }], ['Guardian', { 16: 2 }], ['ArmorStand', { 10: 0x3f }], ['MinecartChest', {}],
  ['MinecartFurnace', { 16: 1 }], ['MinecartRideable', { 22: 1, 20: 54 | (3 << 12) }], ['MinecartTNT', {}], ['Pig', {}],
  ['Slime', { 16: -2 }],
];
console.log(JSON.stringify({ ...out, variants: cases.map(([n, m]) => [n, Object.entries(m).map(([k, v]) => [Number(k), v]), mobVariant(n, m)]) }, null, 1));
