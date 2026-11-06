/**
 * Mob, object and armour models (mobs.ts, objects.ts, armour.ts), tile entities in the column, flower pots,
 * chests, skulls and banners, LOOKS. Checks that need the real 1.8 assets run only when they are given:
 *   VIEWER_TEXTURES=<dir of assets/minecraft/textures>  the texture sizes the models expect
 *   VIEWER_DATA=<data.json the server built>            block models (flower pots, double chests, images)
 */
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import type { ModelData } from '../src/shared/model-data.js';
import { decode, potContents } from '../src/shared/blocks18.js';
import { decodeColumn, encodeColumn, mobVariant, messages, S, Writer, LOOK, NO_COLOR, readLooks, writeLooks, type Tile } from '../src/shared/protocol.js';
import { Mesher, P, pidx, VERTEX_BYTES, type SectionInput } from '../src/mesher.js';
import { BANNER, BANNER_PATTERNS, LIVING_BABY, MOBS, SHEEP_FUR, SKULLS, babyGroup, bannerPlace, boxUvExtent, horseTexture, mobTextures, posesOf, skullPlace, type PoseIn } from '../src/mobs.js';
import { OBJECT_MODELS, minecartBlock, objectTextures } from '../src/objects.js';
import { ARMOUR_PARTS, armourParts, armourPiece, armourTexture, armourTextures, headBlock } from '../src/armour.js';
import { pageImages } from '../src/images.js';

const TEXTURES = process.env.VIEWER_TEXTURES ?? '';
const DATA = process.env.VIEWER_DATA ?? '';
const png = (name: string): { w: number; h: number } | null => {
  const file = path.join(TEXTURES, `${name}.png`);
  if (!fs.existsSync(file)) return null;
  const b = fs.readFileSync(file);
  return { w: b.readUInt32BE(16), h: b.readUInt32BE(20) };
};

const rest: PoseIn = { swing: 0, swingAmt: 0, headYaw: 0, pitch: 0, age: 0, variant: 0 };

describe('mob models', () => {
  const all = { ...MOBS, ...OBJECT_MODELS, 'Sheep#fur': SHEEP_FUR };

  it.skipIf(!TEXTURES)('every box reads inside its texture, and the texture has the size the model expects', () => {
    for (const [name, def] of Object.entries(all)) {
      const tex = typeof def.texture === 'string' ? def.texture : def.texture(0);
      const size = png(tex);
      expect(size, `${name}: ${tex}.png`).not.toBeNull();
      expect([size!.w, size!.h], `${name} texture size`).toEqual([def.tw, def.th]);
      for (const [part, p] of Object.entries(def.parts)) {
        // MinecartEntityModel's inner floor reads past u 64 with its back face, which the bottom hides (vanilla too)
        if (def === OBJECT_MODELS.MinecartRideable && part === 'floor') continue;
        for (const b of p.boxes) {
          const [u, v] = boxUvExtent(b);
          expect(u, `${name}.${part} u`).toBeLessThanOrEqual(def.tw);
          expect(v, `${name}.${part} v`).toBeLessThanOrEqual(def.th);
        }
      }
    }
  });

  it('declares parents before their children', () => {
    for (const [name, def] of Object.entries(all)) {
      const seen = new Set<string>();
      for (const [part, p] of Object.entries(def.parts)) {
        if (p.parent) expect(seen.has(p.parent), `${name}.${part} → ${p.parent}`).toBe(true);
        seen.add(part);
      }
    }
  });

  it('lists every texture a model can ask for (variants too) in images.json', () => {
    const images = pageImages();
    for (const t of [...mobTextures(), ...objectTextures()]) {
      if (TEXTURES) expect(png(t), t).not.toBeNull();
      expect(images, t).toContain(t);
    }
    expect(horseTexture(2)).toBe('entity/horse/horse_chestnut');
    expect(horseTexture(1 << 8)).toBe('entity/horse/donkey');
    expect(horseTexture(4 << 8)).toBe('entity/horse/horse_skeleton');
    expect(typeof MOBS.Villager.texture === 'function' && MOBS.Villager.texture(3)).toBe('entity/villager/smith');
  });

  it('poses: the head looks, legs swing in opposite pairs, blaze rods circle, a big slime is bigger', () => {
    const looking = posesOf(MOBS.Wolf, { ...rest, headYaw: 0.5, pitch: -0.2 });
    expect(looking.head.ry).toBeCloseTo(0.5);
    expect(looking.head.rx).toBeCloseTo(-0.2);
    expect(looking.body.rx).toBeCloseTo(Math.PI / 2);
    const walking = posesOf(MOBS.Wolf, { ...rest, swing: 0, swingAmt: 1 });
    expect(walking.leg1.rx).toBeCloseTo(1.4);
    expect(walking.leg2.rx).toBeCloseTo(-1.4);
    expect(walking.leg4.rx).toBeCloseTo(walking.leg1.rx);
    // the horse turns its neck only sideways and keeps the shin under the knee (a child of the leg)
    const horse = posesOf(MOBS.EntityHorse, { ...rest, headYaw: 0.4, pitch: 0.5 });
    expect(horse.neck.rx).toBeCloseTo(0.5235988);
    expect(horse.neck.ry).toBeCloseTo(0.4);
    expect(MOBS.EntityHorse.parts.frontLeftLegShin.parent).toBe('frontLeftLeg');
    const a = posesOf(MOBS.Blaze, { ...rest, age: 0 }), b = posesOf(MOBS.Blaze, { ...rest, age: 10 });
    expect(Math.hypot(a.rod0.px, a.rod0.pz)).toBeCloseTo(9);
    expect(Math.hypot(a.rod8.px, a.rod8.pz)).toBeCloseTo(5);
    expect(a.rod0.px).not.toBeCloseTo(b.rod0.px);
    const slime = MOBS.Slime.scale as (v: number) => number;
    expect([slime(0), slime(1), slime(4)]).toEqual([1, 1, 4]);
    // spider legs spread out to both sides
    const sp = posesOf(MOBS.Spider, rest);
    expect(sp.sleg1.rz).toBeLessThan(0);
    expect(sp.sleg2.rz).toBeGreaterThan(0);
  });

  it('reads the variant from 1.8 entity metadata', () => {
    expect(mobVariant('Slime', { 16: 4 })).toBe(4);
    expect(mobVariant('LavaSlime', [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2])).toBe(2);
    expect(mobVariant('Villager', { 16: 4 })).toBe(4);
    expect(mobVariant('Sheep', { 16: 16 | 14 })).toBe(30);
    expect(mobVariant('EntityHorse', { 19: 1, 20: 3 | (2 << 8) })).toBe((1 << 8) | 3);
    expect(mobVariant('Skeleton', { 13: 1 })).toBe(1);
    expect(mobVariant('Pig', { 16: 1 })).toBe(0);
    expect(mobVariant('Zombie', { 13: 1 })).toBe(1);
    expect(mobVariant('Wolf', { 16: 4 | 1, 20: 4 })).toBe(5 | (4 << 4));
    expect(mobVariant('Ozelot', { 18: 3 })).toBe(3);
    expect(mobVariant('Rabbit', { 18: 99 })).toBe(99);
    expect(mobVariant('Bat', { 16: 1 })).toBe(1);
    expect(mobVariant('Guardian', { 16: 4 })).toBe(1);
    expect(mobVariant('Guardian', { 16: 2 })).toBe(0);
    expect(mobVariant('ArmorStand', { 10: 4 | 8 })).toBe(12);
    // minecarts: their class's block, a powered furnace lit, or the display tile (id | meta << 12) when byte 22 is set
    expect(mobVariant('MinecartRideable', {})).toBe(0);
    expect(mobVariant('MinecartHopper', {})).toBe(154 << 4);
    expect(mobVariant('MinecartFurnace', { 16: 1 })).toBe((62 << 4) | 2);
    expect(mobVariant('MinecartRideable', { 20: 35 | (14 << 12), 22: 1 })).toBe((35 << 4) | 14);
    expect(mobVariant('Slime', undefined)).toBe(0);
  });

  it('a SPAWN without the variant (older server) still reads: the page treats it as 0', () => {
    const w = new Writer();
    const at = w.begin(S.SPAWN);
    w.u32(5).u8(3).str('Slime').str('').f64(0).f64(0).f64(0).f32(0).f32(0).f32(0).f32(1).f32(1).u16(0).u16(0);
    w.end(at);
    const [[type, r]] = [...messages(w.done())];
    expect(type).toBe(S.SPAWN);
    r.u32(); r.u8(); r.str(); r.str(); r.f64(); r.f64(); r.f64(); r.f32(); r.f32(); r.f32(); r.f32(); r.f32(); r.u16(); r.u16();
    expect(r.pos + 2 <= r.end).toBe(false);
  });
});

let data: ModelData | null = null;
const modelData = (): ModelData => (data ??= JSON.parse(fs.readFileSync(DATA, 'utf8')) as ModelData);

describe('tile entities', () => {
  it('carry skulls, banners and flower pots with the column (and a column without them still decodes)', () => {
    const tiles: Tile[] = [
      { x: 1, y: 70, z: 2, kind: 'skull', skull: 3, rot: 9 },
      { x: 15, y: 4, z: 15, kind: 'banner', base: 1, patterns: [['cre', 0], ['bo', 15]] },
      { x: 0, y: 255, z: 0, kind: 'pot', contents: 'blue_orchid' },
    ];
    const body = encodeColumn([], new Uint8Array(256), true, [{ x: 3, y: 3, z: 3, lines: ['a', '', '', ''] }], tiles);
    const col = decodeColumn(0, 0, body);
    expect(col.signs).toHaveLength(1);
    expect(col.tiles).toEqual(tiles);
    // an older server ends after the signs (no tile count)
    const plain = encodeColumn([], new Uint8Array(256), true, [{ x: 3, y: 3, z: 3, lines: ['a', '', '', ''] }]);
    const old = decodeColumn(0, 0, plain.subarray(0, plain.length - 2));
    expect(old.signs).toHaveLength(1);
    expect(old.tiles).toEqual([]);
  });

  it('flower pot contents follow FlowerPotBlock.getBlockState', () => {
    expect(potContents('minecraft:red_flower', 0)).toBe('rose');
    expect(potContents('red_flower', 8)).toBe('oxeye_daisy');
    expect(potContents('yellow_flower', 0)).toBe('dandelion');
    expect(potContents('tallgrass', 2)).toBe('fern');
    expect(potContents('tallgrass', 1)).toBe('empty');
    expect(potContents('deadbush', 0)).toBe('dead_bush');
    expect(potContents('brown_mushroom', 0)).toBe('mushroom_brown');
    expect(potContents('stone', 0)).toBe('empty');
  });

  it.skipIf(!DATA)('every flower pot content has a model', () => {
    const pot = modelData().blocks[140]!;
    const all = [...['red_flower:0', 'red_flower:1', 'red_flower:2', 'red_flower:3', 'red_flower:4', 'red_flower:5', 'red_flower:6', 'red_flower:7', 'red_flower:8'],
      'yellow_flower:0', 'sapling:0', 'sapling:1', 'sapling:2', 'sapling:3', 'sapling:4', 'sapling:5', 'tallgrass:2', 'deadbush:0', 'cactus:0', 'red_mushroom:0', 'brown_mushroom:0'];
    for (const it of all) { const [n, d] = it.split(':'); expect(pot.variants[`contents=${potContents(n, Number(d))}`], it).toBeDefined(); }
  });

  it.skipIf(!DATA)('meshes a flower pot with its plant when the worker passes the contents', () => {
    const d = modelData();
    const m = new Mesher(d, new Uint8Array(d.layers.length), { grass: new Uint32Array(256).fill(0x79c05a), foliage: new Uint32Array(256), water: new Uint32Array(256) }, new Uint16Array(d.layers.length).map((_, i) => i));
    const states = new Uint16Array(P * P * P), sky = new Uint8Array(P * P * P).fill(15);
    states[pidx(8, 8, 8)] = 140 << 4;
    const input: SectionInput = { states, sky, blk: new Uint8Array(P * P * P), biomes: new Uint8Array(P * P), wx: 0, wy: 0, wz: 0 };
    const quads = (o: { solid: Uint8Array; cutout: Uint8Array; translucent: Uint8Array }) => (o.solid.length + o.cutout.length + o.translucent.length) / VERTEX_BYTES / 4;
    const empty = quads(m.mesh(input));
    const planted = quads(m.mesh({ ...input, extra: new Map([[pidx(8, 8, 8), 'contents=cactus']]) }));
    expect(empty).toBeGreaterThan(0);
    expect(planted).toBeGreaterThan(empty);
  });

  it('pairs chests into double chests from the neighbour on the left or right of the front', () => {
    const names = { name: (id: number) => ({ 54: 'chest', 146: 'trapped_chest', 130: 'ender_chest' } as Record<number, string>)[id] };
    const facts = { solid: () => false };
    const at = (dx: number, dz: number, id = 54, meta = 3) => (x: number, y: number, z: number) => (x === dx && y === 0 && z === dz ? (id << 4) | meta : 0);
    // facing south: the viewer looks north, west is on the right
    expect(decode('chest', 3, at(1, 0), names, facts)).toBe('facing=south,type=right');
    expect(decode('chest', 3, at(-1, 0), names, facts)).toBe('facing=south,type=left');
    expect(decode('chest', 2, at(1, 0, 54, 2), names, facts)).toBe('facing=north,type=left');
    expect(decode('chest', 5, at(0, 1, 54, 5), names, facts)).toBe('facing=east,type=left');
    // a trapped chest next to a normal one stays single; ender chests never pair
    expect(decode('chest', 3, at(1, 0, 146), names, facts)).toBe('facing=south');
    expect(decode('ender_chest', 3, at(1, 0, 130), names, facts)).toBe('facing=south');
    if (!DATA) return;
    const chest = modelData().blocks[54]!;
    for (const f of ['north', 'south', 'west', 'east']) for (const t of ['left', 'right']) expect(chest.variants[`facing=${f},type=${t}`]).toBeDefined();
    expect(modelData().blocks[146]!.variants['facing=west,type=right']).toBeDefined();
  });

  it('places skulls and banners like the 1.8 tile entity renderers, with every texture packed', () => {
    expect(skullPlace(1, 8)).toEqual({ x: 0.5, y: 0, z: 0.5, ry: Math.PI });
    expect(skullPlace(2, 0)).toMatchObject({ z: 0.74, y: 0.25 });
    expect(skullPlace(5, 0)).toMatchObject({ x: 0.26, ry: Math.PI / 2 });
    const wall = bannerPlace(false, 2);
    expect(wall.pole).toBe(false);
    expect(wall.z).toBeCloseTo(0.9375);
    expect(bannerPlace(false, 4).x).toBeCloseTo(0.9375);
    expect(bannerPlace(true, 4)).toMatchObject({ x: 0.5, z: 0.5, pole: true, yaw: -Math.PI / 2 });
    const images = pageImages();
    for (const [img] of SKULLS) expect(images).toContain(img);
    expect(images).toContain(BANNER.texture);
    for (const [, file] of BANNER_PATTERNS) expect(images, file).toContain(`entity/banner/${file}`);
    for (const p of Object.values(BANNER.parts)) for (const b of p.boxes) expect(boxUvExtent(b)[0]).toBeLessThanOrEqual(64);
  });
});

describe('new mob models, armour and objects', () => {
  it('models the mobs that were boxes, with their textures by variant', () => {
    for (const name of ['Ghast', 'LavaSlime', 'Silverfish', 'Endermite', 'Bat', 'Ozelot', 'Rabbit', 'SnowMan', 'Guardian', 'WitherBoss', 'EnderDragon']) expect(MOBS[name], name).toBeDefined();
    for (const name of ['ArmorStand', 'Boat', 'MinecartRideable', 'MinecartChest', 'MinecartHopper']) expect(OBJECT_MODELS[name], name).toBeDefined();
    const tex = (name: string, v: number) => { const t = MOBS[name].texture; return typeof t === 'string' ? t : t(v); };
    expect([0, 1, 2, 3].map(v => tex('Ozelot', v))).toEqual(['entity/cat/ocelot', 'entity/cat/black', 'entity/cat/red', 'entity/cat/siamese']);
    expect([tex('Rabbit', 3), tex('Rabbit', 99)]).toEqual(['entity/rabbit/white_splotched', 'entity/rabbit/caerbannog']);
    expect([tex('Wolf', 0), tex('Wolf', 2), tex('Wolf', 4)]).toEqual(['entity/wolf/wolf', 'entity/wolf/wolf_angry', 'entity/wolf/wolf_tame']);
    expect([tex('Ghast', 1), tex('Guardian', 1)]).toEqual(['entity/ghast/ghast_shooting', 'entity/guardian_elder']);
    // a tame wolf wears its collar in the dye colour (byte 20 is a dye damage: 1 red, 4 blue)
    expect(MOBS.Wolf.layers!(4 | (1 << 4))).toEqual([['entity/wolf/wolf_collar', [0.6, 0.2, 0.2]]]);
    expect(MOBS.Wolf.layers!(0)).toEqual([]);
    const scale = (name: string, v: number) => { const s = MOBS[name].scale; return typeof s === 'function' ? s(v) : s; };
    expect([scale('Ghast', 0), scale('Bat', 0), scale('Guardian', 1), scale('WitherBoss', 0), scale('Ozelot', 2), scale('Villager', 0)]).toEqual([4.5, 0.35, 2.35, 2, 0.8, 0.9375]);
  });

  it('poses: bats hang or fly, tentacles sway, a sitting wolf, the dragon\'s wings mirror, the snow golem\'s arms turn with its body', () => {
    const flying = posesOf(MOBS.Bat, { ...rest, age: 3 }), hanging = posesOf(MOBS.Bat, { ...rest, variant: 1 });
    expect(hanging.body.rx).toBeCloseTo(Math.PI);
    expect(hanging.head.rz).toBeCloseTo(Math.PI);
    expect(flying.body.rx).toBeGreaterThan(0.5);
    expect(flying.leftWing.ry).toBeCloseTo(-flying.rightWing.ry);
    expect(MOBS.Bat.parts.outerLeftWing.parent).toBe('leftWing');
    const g0 = posesOf(MOBS.Ghast, { ...rest, age: 0 }), g1 = posesOf(MOBS.Ghast, { ...rest, age: 5 });
    expect(g0.tentacle0.rx).not.toBeCloseTo(g1.tentacle0.rx);
    expect(MOBS.Ghast.parts.tentacle1.boxes[0][6]).toBe(13);
    const sitting = posesOf(MOBS.Wolf, { ...rest, variant: 1 });
    expect(sitting.body.rx).toBeCloseTo(Math.PI / 4);
    expect(sitting.leg1.py).toBe(22);
    const d = posesOf(MOBS.EnderDragon, { ...rest, age: 7 });
    expect(d.lwing.rz).toBeCloseTo(-d.rwing.rz);
    expect(MOBS.EnderDragon.parts.lwing.pivot[0]).toBe(-MOBS.EnderDragon.parts.rwing.pivot[0]);
    expect(MOBS.EnderDragon.parts.lwingTip.parent).toBe('lwing');
    const sg = posesOf(MOBS.SnowMan, { ...rest, headYaw: 0.8 });
    expect(sg.body.ry).toBeCloseTo(0.2);
    expect(sg.leftHand.ry).toBeCloseTo(Math.PI + 0.2);
    expect(Math.hypot(sg.rightHand.px, sg.rightHand.pz)).toBeCloseTo(5);
    // silverfish segments wiggle sideways with the age, the middle one least
    const sf = posesOf(MOBS.Silverfish, { ...rest, age: 4 });
    expect(sf.part2.px).toBeCloseTo(0);
    expect(Math.abs(sf.part6.px)).toBeGreaterThan(0);
  });

  it('armour stands: the pose from LOOKS in degrees, arms and base plate by flags, small ones are children', () => {
    const stand = OBJECT_MODELS.ArmorStand;
    const pose = [10, 0, 0, 0, 30, 0, -10, 0, -10, -90, 0, 10, -1, 0, -1, 1, 0, 1];
    const p = posesOf(stand, { ...rest, rot: pose });
    expect(p.head.rx).toBeCloseTo((10 * Math.PI) / 180);
    expect(p.rightArm.rx).toBeCloseTo(-Math.PI / 2);
    expect(p.standWaist.ry).toBeCloseTo((30 * Math.PI) / 180);
    expect(stand.hidden!(0)).toEqual(['rightArm', 'leftArm']);
    expect(stand.hidden!(4 | 8)).toEqual(['standBase']);
    expect(babyGroup(stand.baby, 'head')).toEqual([0.75, 0, 16, 0]);
    expect(babyGroup(stand.baby, 'standBase')).toEqual([0.5, 0, 24, 0]);
  });

  it('babies: biped heads ¾, quadruped heads full size moved, the rest half size on the ground', () => {
    expect(babyGroup(LIVING_BABY.Zombie, 'head')).toEqual([0.75, 0, 16, 0]);
    expect(babyGroup(LIVING_BABY.Pig, 'head')).toEqual([1, 0, 4, 4]);
    expect(babyGroup(LIVING_BABY.Cow, 'head')).toEqual([1, 0, 8, 4]);
    expect(babyGroup(LIVING_BABY.Cow, 'leg1')).toEqual([0.5, 0, 24, 0]);
    expect(babyGroup(MOBS.Wolf.baby, 'head')).toEqual([1, 0, 5, 2]);
    expect(babyGroup(MOBS.Rabbit.baby, 'leftEar')).toEqual([0.56666666, 0, 22, 2]);
    // a whole-model baby keeps its feet on the ground: 24 px stay 24 px
    const [s, , ty] = babyGroup(MOBS.Villager.baby, 'body')!;
    expect(s * (24 + ty)).toBeCloseTo(24);
    expect(babyGroup(undefined, 'head')).toBeNull();
  });

  it('armour: item ids to material and slot, the layer textures, BiPedModel(1.0F / 0.5F) boxes, blocks on heads', () => {
    expect(armourPiece(298)).toEqual(['leather', 4]);
    expect(armourPiece(307)).toEqual(['iron', 3]);
    expect(armourPiece(312)).toEqual(['diamond', 2]);
    expect(armourPiece(317)).toEqual(['gold', 1]);
    expect(armourPiece(276)).toBeNull();
    expect(armourTexture('diamond', 2)).toBe('models/armor/diamond_layer_2');
    expect(armourTexture('leather', 4, true)).toBe('models/armor/leather_layer_1_overlay');
    expect(armourTextures()).toHaveLength(12);
    expect(ARMOUR_PARTS[2]).toEqual(['body', 'rightLeg', 'leftLeg']);
    const helmet = armourParts(1).head.boxes, legs = armourParts(0.5).body.boxes;
    expect(helmet[0][8]).toBe(1);
    expect(helmet[1][8]).toBe(1.5);
    expect(legs[0][8]).toBe(0.5);
    expect(armourParts(1, true).head.boxes[0][3]).toBe(-10);
    for (const inflate of [1, 0.5]) for (const p of Object.values(armourParts(inflate))) for (const b of p.boxes) {
      const [u, v] = boxUvExtent(b);
      expect(u).toBeLessThanOrEqual(64); expect(v).toBeLessThanOrEqual(32);
    }
    expect(headBlock('SnowMan')).toEqual([86, 2]);
    expect(headBlock('player', [[4, 86, 0, NO_COLOR]])).toEqual([86, 2]);
    expect(headBlock('player', [[4, 310, 0, NO_COLOR]])).toBeNull();
  });

  it('LOOKS: what an entity wears and its size, written and read back', () => {
    const w = new Writer();
    const l = { flags: LOOK.BABY, armour: [[4, 298, 0, 0xa06540]] as Array<[number, number, number, number]>, pose: Array.from({ length: 18 }, (_, i) => i) };
    writeLooks(w, 77, l);
    const [[type, r]] = [...messages(w.done())];
    expect(type).toBe(S.LOOKS);
    expect(readLooks(r)).toEqual({ id: 77, ...l });
  });

  it('objects: minecart blocks', () => {
    expect(minecartBlock('MinecartChest', (54 << 4) | 2)).toMatchObject({ id: 54, meta: 2, scale: 0.75 });
    expect(minecartBlock('MinecartHopper', 154 << 4)!.dy).toBeCloseTo(0.375 - (7 / 16) * 0.75);
    expect(minecartBlock('MinecartRideable', 0)).toBeNull();
    expect(minecartBlock('Boat', 5)).toBeNull();
  });
});
