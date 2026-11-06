/**
 * Mob models beyond the player, zombie, skeleton, pig, cow, sheep and creeper of entities.ts: the boxes,
 * texture offsets and rotation points of the 1.8 Model* classes, and their poses (setAngles).
 * Pure data and maths (no GL), so the tests check every box against its texture and the poses.
 *
 * Boxes are ModelBox arguments: [texU, texV, x, y, z, w, h, d, inflate?, mirror?] in the part's own
 * space (pixels, y down). A part with a `parent` is drawn in its parent's space (ModelPart.add).
 */

export type Box = [number, number, number, number, number, number, number, number, number?, boolean?];
export type V3 = [number, number, number];

export interface PartDef {
  boxes: Box[];
  pivot: V3;
  rot?: V3;
  parent?: string;
  /** Turns with the head (yaw, and pitch unless 'yaw'). */
  look?: boolean | 'yaw';
  /** Walk swing about x: +1 / -1 for the two diagonal leg pairs (0.6662 rad per block, like the quadrupeds). */
  walk?: number;
  /** Drawn blended after the rest (the slime's outer gel). */
  translucent?: boolean;
}

export interface Pose { px: number; py: number; pz: number; rx: number; ry: number; rz: number }

export interface PoseIn {
  /** Limb swing phase and amount (walk animation). */
  swing: number; swingAmt: number;
  /** Head yaw relative to the body, and pitch (radians, MC model space). */
  headYaw: number; pitch: number;
  /** Game ticks (idle animations: blaze rods, wings, tentacles). */
  age: number;
  variant: number;
  /** Armour stand rotations in degrees (LOOKS pose: head, body, left arm, right arm, left leg, right leg). */
  rot?: ArrayLike<number>;
}

/**
 * Child rendering (BiPedModel / QuadruPedEntityModel child): the `head` parts drawn scaled by hs and moved by ht
 * (pixels, in the scaled space: GlStateManager.scale then translate), every other top-level part by bs / bt.
 * Replaces the model's `scale`.
 */
export interface Baby { head: string[]; hs: number; ht: V3; bs: number; bt: V3 }

/** BiPedModel: head ¾ size 16 px down, the rest half size on the ground. */
export const bipedBaby = (head = ['head']): Baby => ({ head, hs: 0.75, ht: [0, 16, 0], bs: 0.5, bt: [0, 24, 0] });
/** QuadruPedEntityModel: the head full size, moved down y and back z (field_1514 8 / field_1515 4 unless the model sets them). */
export const quadBaby = (y = 8, z = 4, head = ['head']): Baby => ({ head, hs: 1, ht: [0, y, z], bs: 0.5, bt: [0, 24, 0] });
/** Whole-model scaling with the feet kept on the ground (scale s, then 24·(1 − s)/s px down). */
export const wholeBaby = (s: number): Baby => ({ head: [], hs: s, ht: [0, (24 * (1 - s)) / s, 0], bs: s, bt: [0, (24 * (1 - s)) / s, 0] });

export interface MobDef {
  /** Texture image name, or one per variant (villager professions, horse colours, slime sizes keep one). */
  texture: string | ((variant: number) => string);
  tw: number; th: number;
  parts: Record<string, PartDef>;
  /** Model scale (cave spider 0.7), or per variant (slime size). */
  scale?: number | ((variant: number) => number);
  /** Blocks to lift the model (models drawn below their feet in vanilla's renderer, the squid). */
  lift?: number;
  animate?: (p: PoseIn, poses: Record<string, Pose>) => void;
  baby?: Baby;
  /** Layers drawn over the model with the same boxes and poses (a tame wolf's dyed collar): texture and tint. */
  layers?: (variant: number) => Array<[string, [number, number, number]]>;
  /** Parts a variant does not draw (an armour stand's arms and base plate). */
  hidden?: (variant: number) => string[];
}

const PI = Math.PI;

/** SpiderEntityModel: head, neck, body and eight legs splayed out (setAngles' rest pose), legs scuttle when walking. */
function spider(texture: string, scale?: number): MobDef {
  const parts: Record<string, PartDef> = {
    head: { boxes: [[32, 4, -4, -4, -8, 8, 8, 8]], pivot: [0, 15, -3], look: true },
    neck: { boxes: [[0, 0, -3, -3, -3, 6, 6, 6]], pivot: [0, 15, 0] },
    body: { boxes: [[0, 12, -5, -4, -6, 10, 8, 12]], pivot: [0, 15, 9] },
  };
  const q = PI / 4, f2 = 0.3926991;
  const z = [-q, q, -q * 0.74, q * 0.74, -q * 0.74, q * 0.74, -q, q];
  const y = [f2 * 2, -f2 * 2, f2, -f2, -f2, f2, -f2 * 2, f2 * 2];
  const pz = [2, 2, 1, 1, 0, 0, -1, -1];
  for (let i = 0; i < 8; i++) {
    const right = i % 2 === 0;
    parts[`sleg${i + 1}`] = { boxes: [[18, 0, right ? -15 : -1, -1, -1, 16, 2, 2]], pivot: [right ? -4 : 4, 15, pz[i]], rot: [0, y[i], z[i]] };
  }
  return {
    texture, tw: 64, th: 32, parts, scale,
    animate: (p, poses) => {
      // SpiderEntityModel: legs swing forward/back in pairs (y) and lift (z)
      const s = p.swing * 0.6662 * 2, a = p.swingAmt;
      const fy = [-(Math.cos(s) * 0.4), -(Math.cos(s + PI) * 0.4), -(Math.cos(s + PI / 2) * 0.4), -(Math.cos(s + PI * 1.5) * 0.4)];
      const fz = [Math.abs(Math.sin(p.swing * 0.6662) * 0.4), Math.abs(Math.sin(p.swing * 0.6662 + PI) * 0.4),
        Math.abs(Math.sin(p.swing * 0.6662 + PI / 2) * 0.4), Math.abs(Math.sin(p.swing * 0.6662 + PI * 1.5) * 0.4)];
      for (let i = 0; i < 4; i++) {
        const l = poses[`sleg${i * 2 + 1}`], r = poses[`sleg${i * 2 + 2}`];
        l.ry += fy[i] * a; r.ry -= fy[i] * a;
        l.rz += fz[i] * a; r.rz -= fz[i] * a;
      }
    },
  };
}

/** VillagerEntityModel (64×64; the witch uses the same boxes on a 64×128 texture plus her hat). */
function villagerParts(): Record<string, PartDef> {
  return {
    head: { boxes: [[0, 0, -4, -10, -4, 8, 10, 8]], pivot: [0, 0, 0], look: true },
    nose: { boxes: [[24, 0, -1, -1, -6, 2, 4, 2]], pivot: [0, -2, 0], parent: 'head' },
    body: { boxes: [[16, 20, -4, 0, -3, 8, 12, 6], [0, 38, -4, 0, -3, 8, 18, 6, 0.5]], pivot: [0, 0, 0] },
    // crossed arms: two arms and the band between them, held forward (setAngles: y 3, z -1, x -0.75)
    arms: { boxes: [[44, 22, -8, -2, -2, 4, 8, 4], [44, 22, 4, -2, -2, 4, 8, 4], [40, 38, -4, 2, -2, 8, 4, 4]], pivot: [0, 3, -1], rot: [-0.75, 0, 0] },
    rightLeg: { boxes: [[0, 22, -2, 0, -2, 4, 12, 4]], pivot: [-2, 12, 0] },
    leftLeg: { boxes: [[0, 22, -2, 0, -2, 4, 12, 4, 0, true]], pivot: [2, 12, 0] },
  };
}

const VILLAGER_SKINS = ['entity/villager/farmer', 'entity/villager/librarian', 'entity/villager/priest', 'entity/villager/smith', 'entity/villager/butcher'];
const HORSE_COLORS = ['white', 'creamy', 'chestnut', 'brown', 'black', 'gray', 'darkbrown'];

/** Horse texture by variant (SPAWN variant: type << 8 | colour). */
export function horseTexture(variant: number): string {
  const type = variant >> 8;
  if (type === 1) return 'entity/horse/donkey';
  if (type === 2) return 'entity/horse/mule';
  if (type === 3) return 'entity/horse/horse_zombie';
  if (type === 4) return 'entity/horse/horse_skeleton';
  return `entity/horse/horse_${HORSE_COLORS[variant & 0xff] ?? 'brown'}`;
}

/** The horse's legs: upper leg, shin and hoof; shin and hoof hang from the leg (vanilla moves their pivots with it). */
type Cuboid = [number, number, number, number, number, number];
function horseLeg(out: Record<string, PartDef>, name: string, u: number, upper: Cuboid, shin: Cuboid, hoof: Cuboid, x: number, z: number, walk: number): void {
  out[name] = { boxes: [[u, 29, ...upper]], pivot: [x, 9, z], walk };
  out[`${name}Shin`] = { boxes: [[u, upper[4] === 8 ? 41 : 43, ...shin]], pivot: [0, 7, 0], parent: name };
  out[`${name}Hoof`] = { boxes: [[u, 51, ...hoof]], pivot: [0, 7, 0], parent: name };
}

function horse(): MobDef {
  const neck = 0.5235988;
  const parts: Record<string, PartDef> = {
    body: { boxes: [[0, 34, -5, -8, -19, 10, 10, 24]], pivot: [0, 11, 9] },
    tailBase: { boxes: [[44, 0, -1, -1, 0, 2, 2, 3]], pivot: [0, 3, 14], rot: [-1.134464, 0, 0] },
    tailMiddle: { boxes: [[38, 7, -1.5, -2, 3, 3, 4, 7]], pivot: [0, 3, 14], rot: [-1.134464, 0, 0] },
    tailTip: { boxes: [[24, 3, -1.5, -4.5, 9, 3, 4, 7]], pivot: [0, 3, 14], rot: [-1.40215, 0, 0] },
    neck: { boxes: [[0, 12, -2.05, -9.8, -2, 4, 14, 8]], pivot: [0, 4, -10], rot: [neck, 0, 0], look: 'yaw' },
    mane: { boxes: [[58, 0, -1, -11.5, 5, 2, 16, 4]], pivot: [0, 4, -10], rot: [neck, 0, 0], look: 'yaw' },
    head: { boxes: [[0, 0, -2.5, -10, -1.5, 5, 5, 7]], pivot: [0, 4, -10], rot: [neck, 0, 0], look: 'yaw' },
    upperMouth: { boxes: [[24, 18, -2, -10, -7, 4, 3, 6]], pivot: [0, -0.05, 0], parent: 'head' },
    lowerMouth: { boxes: [[24, 27, -2, -7, -6.5, 4, 2, 5]], pivot: [0, 0, 0], parent: 'head' },
    leftEar: { boxes: [[0, 0, 0.45, -12, 4, 2, 3, 1]], pivot: [0, 4, -10], rot: [neck, 0, 0], look: 'yaw' },
    rightEar: { boxes: [[0, 0, -2.45, -12, 4, 2, 3, 1]], pivot: [0, 4, -10], rot: [neck, 0, 0], look: 'yaw' },
  };
  // [x, y, z, w, h, d] of the upper leg, shin (relative to the knee) and hoof
  horseLeg(parts, 'backLeftLeg', 78, [-2.5, -2, -2.5, 4, 9, 5], [-2, 0, -1.5, 3, 5, 3], [-2.5, 5.1, -2, 4, 3, 4], 4, 11, -1);
  horseLeg(parts, 'backRightLeg', 96, [-1.5, -2, -2.5, 4, 9, 5], [-1, 0, -1.5, 3, 5, 3], [-1.5, 5.1, -2, 4, 3, 4], -4, 11, 1);
  horseLeg(parts, 'frontLeftLeg', 44, [-1.9, -1, -2.1, 3, 8, 4], [-1.9, 0, -1.6, 3, 5, 3], [-2.4, 5.1, -2.1, 4, 3, 4], 4, -8, 1);
  horseLeg(parts, 'frontRightLeg', 60, [-1.1, -1, -2.1, 3, 8, 4], [-1.1, 0, -1.6, 3, 5, 3], [-1.6, 5.1, -2.1, 4, 3, 4], -4, -8, -1);
  // HorseBaseEntityRenderer shrinks foals as a whole (simplified: half size)
  return { texture: horseTexture, tw: 128, th: 128, parts, baby: wholeBaby(0.5) };
}

function blaze(): MobDef {
  const parts: Record<string, PartDef> = { head: { boxes: [[0, 0, -4, -4, -4, 8, 8, 8]], pivot: [0, 0, 0], look: true } };
  for (let i = 0; i < 12; i++) parts[`rod${i}`] = { boxes: [[0, 16, 0, 0, 0, 2, 8, 2]], pivot: [0, 0, 0] };
  return {
    texture: 'entity/blaze', tw: 64, th: 32, parts,
    animate: (p, poses) => {
      // BlazeEntityModel: three rings of four rods circling at different heights and speeds
      const t = p.age;
      let f = t * PI * -0.1;
      for (let i = 0; i < 4; i++, f++) Object.assign(poses[`rod${i}`], { py: -2 + Math.cos((i * 2 + t) * 0.25), px: Math.cos(f) * 9, pz: Math.sin(f) * 9 });
      f = PI / 4 + t * PI * 0.03;
      for (let i = 4; i < 8; i++, f++) Object.assign(poses[`rod${i}`], { py: 2 + Math.cos((i * 2 + t) * 0.25), px: Math.cos(f) * 7, pz: Math.sin(f) * 7 });
      f = 0.47123894 + t * PI * -0.05;
      for (let i = 8; i < 12; i++, f++) Object.assign(poses[`rod${i}`], { py: 11 + Math.cos((i * 1.5 + t) * 0.5), px: Math.cos(f) * 5, pz: Math.sin(f) * 5 });
    },
  };
}

function squid(): MobDef {
  const parts: Record<string, PartDef> = { body: { boxes: [[0, 0, -6, -8, -6, 12, 16, 12]], pivot: [0, 8, 0] } };
  for (let j = 0; j < 8; j++) {
    const d = (j * PI * 2) / 8;
    parts[`tentacle${j}`] = { boxes: [[48, 0, -1, 0, -1, 2, 18, 2]], pivot: [Math.cos(d) * 5, 15, Math.sin(d) * 5], rot: [0, (j * PI * -2) / 8 + PI / 2, 0] };
  }
  return {
    texture: 'entity/squid', tw: 64, th: 32, parts, lift: 0.7,
    animate: (p, poses) => { const a = 0.35 + Math.sin(p.age * 0.15) * 0.25; for (let j = 0; j < 8; j++) poses[`tentacle${j}`].rx = a; },
  };
}

/** GhastEntityModel: a 16 px cube and nine tentacles of Random(1660) lengths, drawn 0.6 blocks lower, at 4.5 × (GhastEntityRenderer). */
function ghast(): MobDef {
  const parts: Record<string, PartDef> = { body: { boxes: [[0, 0, -8, -8, -8, 16, 16, 16]], pivot: [0, 8 + 9.6, 0] } };
  // new Random(1660L).nextInt(7) + 8, nine times
  const len = [8, 13, 9, 11, 11, 10, 12, 9, 12];
  for (let i = 0; i < 9; i++) {
    const x = ((i % 3) - (Math.floor(i / 3) % 2) * 0.5 + 0.25 - 1) * 5, z = (Math.floor(i / 3) - 1) * 5;
    parts[`tentacle${i}`] = { boxes: [[0, 0, -1, 0, -1, 2, len[i], 2]], pivot: [x, 15 + 9.6, z] };
  }
  return {
    texture: (v) => (v ? 'entity/ghast/ghast_shooting' : 'entity/ghast/ghast'), tw: 64, th: 32, parts, scale: 4.5,
    animate: (p, poses) => { for (let i = 0; i < 9; i++) poses[`tentacle${i}`].rx = 0.2 * Math.sin(p.age * 0.3 + i) + 0.4; },
  };
}

/** MagmaCubeEntityModel: eight 1 px slices (they spread when it jumps; at rest stacked) around a core, scaled by the size. */
function magmaCube(): MobDef {
  const parts: Record<string, PartDef> = {};
  for (let i = 0; i < 8; i++) {
    const [u, v] = i === 2 ? [24, 10] : i === 3 ? [24, 19] : [0, i];
    parts[`segment${i}`] = { boxes: [[u, v, -4, 16 + i, -4, 8, 1, 8]], pivot: [0, 0, 0] };
  }
  parts.core = { boxes: [[0, 16, -2, 18, -2, 4, 4, 4]], pivot: [0, 0, 0] };
  return { texture: 'entity/slime/magmacube', tw: 64, th: 32, parts, scale: (v) => Math.max(1, v || 1) };
}

/**
 * SilverfishEntityModel / EndermiteEntityModel: body segments [w, h, d] in a row along z, each swaying sideways with the
 * age (yaw `turn`·π·(1 + |i − 2|), offset `shift`·π·|i − 2|). The silverfish adds three fins over segments 2, 4, 1.
 */
function segmented(size: number[][], tex: number[][], texture: string, turn: number, shift: number): MobDef {
  const parts: Record<string, PartDef> = {};
  const zs: number[] = [];
  let z = -3.5;
  for (let i = 0; i < size.length; i++) {
    const [w, h, d] = size[i];
    parts[`part${i}`] = { boxes: [[tex[i][0], tex[i][1], w * -0.5, 0, d * -0.5, w, h, d]], pivot: [0, 24 - h, z] };
    zs.push(z);
    if (i < size.length - 1) z += (d + size[i + 1][2]) * 0.5;
  }
  const fins = size.length === 7;
  if (fins) {
    parts.fin0 = { boxes: [[20, 0, -5, 0, size[2][2] * -0.5, 10, 8, size[2][2]]], pivot: [0, 16, zs[2]] };
    parts.fin1 = { boxes: [[20, 11, -3, 0, size[4][2] * -0.5, 6, 4, size[4][2]]], pivot: [0, 20, zs[4]] };
    parts.fin2 = { boxes: [[20, 18, -3, 0, size[4][2] * -0.5, 6, 5, size[1][2]]], pivot: [0, 19, zs[1]] };
  }
  return {
    texture, tw: 64, th: 32, parts,
    animate: (p, poses) => {
      for (let i = 0; i < size.length; i++) {
        const a = p.age * 0.9 + i * 0.15 * PI;
        poses[`part${i}`].ry = Math.cos(a) * PI * turn * (1 + Math.abs(i - 2));
        poses[`part${i}`].px = Math.sin(a) * PI * shift * Math.abs(i - 2);
      }
      if (!fins) return;
      poses.fin0.ry = poses.part2.ry;
      poses.fin1.ry = poses.part4.ry; poses.fin1.px = poses.part4.px;
      poses.fin2.ry = poses.part1.ry; poses.fin2.px = poses.part1.px;
    },
  };
}

/** BatEntityModel (BatEntityRenderer: 0.35 ×): flying, wings flapping and bobbing, or hanging upside down (variant 1). */
function bat(): MobDef {
  return {
    texture: 'entity/bat', tw: 64, th: 64, scale: 0.35,
    parts: {
      head: { boxes: [[0, 0, -3, -3, -3, 6, 6, 6]], pivot: [0, 0, 0], look: true },
      rightEar: { boxes: [[24, 0, -4, -6, -2, 3, 4, 1]], pivot: [0, 0, 0], parent: 'head' },
      leftEar: { boxes: [[24, 0, 1, -6, -2, 3, 4, 1, 0, true]], pivot: [0, 0, 0], parent: 'head' },
      body: { boxes: [[0, 16, -3, 4, -3, 6, 12, 6], [0, 34, -5, 16, 0, 10, 6, 1]], pivot: [0, 0, 0] },
      rightWing: { boxes: [[42, 0, -12, 1, 1.5, 10, 16, 1]], pivot: [0, 0, 0], parent: 'body' },
      outerRightWing: { boxes: [[24, 16, -8, 1, 0, 8, 12, 1]], pivot: [-12, 1, 1.5], parent: 'rightWing' },
      leftWing: { boxes: [[42, 0, 2, 1, 1.5, 10, 16, 1, 0, true]], pivot: [0, 0, 0], parent: 'body' },
      outerLeftWing: { boxes: [[24, 16, 0, 1, 0, 8, 12, 1, 0, true]], pivot: [12, 1, 1.5], parent: 'leftWing' },
    },
    animate: (p, poses) => {
      const { head, body, rightWing: rw, leftWing: lw, outerRightWing: orw, outerLeftWing: olw } = poses;
      if (p.variant & 1) {
        // hanging: head and body turned over, wings folded round the body; BatEntityRenderer moves it 0.1 down
        const down = (0.1 * 16) / 0.35;
        Object.assign(head, { py: -2 + down, ry: PI - p.headYaw, rz: PI });
        Object.assign(body, { py: down, rx: PI });
        Object.assign(rw, { px: -3, pz: 3, rx: -0.15707964, ry: -PI * 0.4 });
        Object.assign(lw, { px: 3, pz: 3, rx: -0.15707964, ry: PI * 0.4 });
        orw.ry = -1.7278761; olw.ry = 1.7278761;
        return;
      }
      // flying: bobbing (BatEntityRenderer.method_5777), body tilted forward, wings beating
      const bob = (-Math.cos(p.age * 0.3) * 0.1 * 16) / 0.35;
      head.py += bob; body.py += bob;
      body.rx = PI / 4 + Math.cos(p.age * 0.1) * 0.15;
      rw.ry = Math.cos(p.age * 1.3) * PI * 0.25; lw.ry = -rw.ry;
      orw.ry = rw.ry * 0.5; olw.ry = -rw.ry * 0.5;
    },
  };
}

const OCELOTS = ['entity/cat/ocelot', 'entity/cat/black', 'entity/cat/red', 'entity/cat/siamese'];

/** OcelotEntityModel, walking (state 1); cats (types 1..3, always tame) are drawn at 0.8 (CatEntityRenderer). */
function ocelot(): MobDef {
  return {
    texture: (v) => OCELOTS[v & 3], tw: 64, th: 32, scale: (v) => ((v & 3) ? 0.8 : 1),
    parts: {
      head: { boxes: [[0, 0, -2.5, -2, -3, 5, 4, 5], [0, 24, -1.5, 0, -4, 3, 2, 2], [0, 10, -2, -3, 0, 1, 1, 2], [6, 10, 1, -3, 0, 1, 1, 2]], pivot: [0, 15, -9], look: true },
      body: { boxes: [[20, 0, -2, 3, -8, 4, 16, 6]], pivot: [0, 12, -10], rot: [PI / 2, 0, 0] },
      tail: { boxes: [[0, 15, -0.5, 0, 0, 1, 8, 1]], pivot: [0, 15, 8], rot: [0.9, 0, 0] },
      tail2: { boxes: [[4, 15, -0.5, 0, 0, 1, 8, 1]], pivot: [0, 20, 14], rot: [1.7278761, 0, 0] },
      backLeftLeg: { boxes: [[8, 13, -1, 0, 1, 2, 6, 2]], pivot: [1.1, 18, 5] },
      backRightLeg: { boxes: [[8, 13, -1, 0, 1, 2, 6, 2]], pivot: [-1.1, 18, 5] },
      frontLeftLeg: { boxes: [[40, 0, -1, 0, 0, 2, 10, 2]], pivot: [1.2, 13.8, -5] },
      frontRightLeg: { boxes: [[40, 0, -1, 0, 0, 2, 10, 2]], pivot: [-1.2, 13.8, -5] },
    },
    baby: { head: ['head'], hs: 0.75, ht: [0, 10, 4], bs: 0.5, bt: [0, 24, 0] },
    animate: (p, poses) => {
      const s = p.swing * 0.6662, a = p.swingAmt;
      poses.backLeftLeg.rx = Math.cos(s) * a;
      poses.backRightLeg.rx = Math.cos(s + 0.3) * a;
      poses.frontLeftLeg.rx = Math.cos(s + PI + 0.3) * a;
      poses.frontRightLeg.rx = Math.cos(s + PI) * a;
      poses.tail2.rx = 1.7278761 + (PI / 4) * Math.cos(p.swing) * a;
    },
  };
}

const RABBITS = ['brown', 'white', 'black', 'white_splotched', 'gold', 'salt'];

/** RabbitEntityModel: drawn at 0.6 (RabbitEntityModel.render); the jump poses are left out. Type 99 is the killer bunny. */
function rabbit(): MobDef {
  const m = true;
  return {
    texture: (v) => `entity/rabbit/${v === 99 ? 'caerbannog' : RABBITS[v] ?? 'brown'}`, tw: 64, th: 32, scale: 0.6,
    parts: {
      leftFoot: { boxes: [[26, 24, -1, 5.5, -3.7, 2, 1, 7, 0, m]], pivot: [3, 17.5, 3.7] },
      rightFoot: { boxes: [[8, 24, -1, 5.5, -3.7, 2, 1, 7, 0, m]], pivot: [-3, 17.5, 3.7] },
      leftThigh: { boxes: [[30, 15, -1, 0, 0, 2, 4, 5, 0, m]], pivot: [3, 17.5, 3.7], rot: [-0.34906584, 0, 0] },
      rightThigh: { boxes: [[16, 15, -1, 0, 0, 2, 4, 5, 0, m]], pivot: [-3, 17.5, 3.7], rot: [-0.34906584, 0, 0] },
      body: { boxes: [[0, 0, -3, -2, -10, 6, 5, 10, 0, m]], pivot: [0, 19, 8], rot: [-0.34906584, 0, 0] },
      leftArm: { boxes: [[8, 15, -1, 0, -1, 2, 7, 2, 0, m]], pivot: [3, 17, -1], rot: [-0.17453292, 0, 0] },
      rightArm: { boxes: [[0, 15, -1, 0, -1, 2, 7, 2, 0, m]], pivot: [-3, 17, -1], rot: [-0.17453292, 0, 0] },
      head: { boxes: [[32, 0, -2.5, -4, -5, 5, 4, 5, 0, m]], pivot: [0, 16, -1], look: true },
      rightEar: { boxes: [[52, 0, -2.5, -9, -1, 2, 5, 1, 0, m]], pivot: [0, 16, -1], rot: [0, -0.2617994, 0], look: true },
      leftEar: { boxes: [[58, 0, 0.5, -9, -1, 2, 5, 1, 0, m]], pivot: [0, 16, -1], rot: [0, 0.2617994, 0], look: true },
      tail: { boxes: [[52, 6, -1.5, -1.5, 0, 3, 3, 2, 0, m]], pivot: [0, 20, 7], rot: [-0.3490659, 0, 0] },
      nose: { boxes: [[32, 9, -0.5, -2.5, -5.5, 1, 1, 1, 0, m]], pivot: [0, 16, -1], look: true },
    },
    baby: { head: ['head', 'rightEar', 'leftEar', 'nose'], hs: 0.56666666, ht: [0, 22, 2], bs: 0.4, bt: [0, 36, 0] },
  };
}

/** SnowmanEntityModel (64×64, boxes inflated −0.5); the pumpkin on its head is a block (SnowGolemPumpkinFeatureRenderer, see armour.ts). */
function snowGolem(): MobDef {
  return {
    texture: 'entity/snowman', tw: 64, th: 64,
    parts: {
      head: { boxes: [[0, 0, -4, -8, -4, 8, 8, 8, -0.5]], pivot: [0, 4, 0], look: true },
      body: { boxes: [[0, 16, -5, -10, -5, 10, 10, 10, -0.5]], pivot: [0, 13, 0] },
      bottomBody: { boxes: [[0, 36, -6, -12, -6, 12, 12, 12, -0.5]], pivot: [0, 24, 0] },
      rightHand: { boxes: [[32, 0, -1, 0, -1, 12, 2, 2, -0.5]], pivot: [5, 6, 0], rot: [0, 0, 1] },
      leftHand: { boxes: [[32, 0, -1, 0, -1, 12, 2, 2, -0.5]], pivot: [-5, 6, 0], rot: [0, PI, -1] },
    },
    animate: (p, poses) => {
      // the upper body turns a quarter of the head's turn, the stick arms with it
      const y = p.headYaw * 0.25, sy = Math.sin(y), cy = Math.cos(y);
      poses.body.ry = y;
      Object.assign(poses.rightHand, { ry: y, px: cy * 5, pz: -sy * 5 });
      Object.assign(poses.leftHand, { ry: PI + y, px: -cy * 5, pz: sy * 5 });
    },
  };
}

/** GuardianEntityModel (64×64): body turned with the look, twelve spikes (out), the eye, a three-part tail; elders at 2.35 ×. */
function guardian(): MobDef {
  const parts: Record<string, PartDef> = {
    body: {
      boxes: [[0, 0, -6, 10, -8, 12, 12, 16], [0, 28, -8, 10, -6, 2, 12, 12], [0, 28, 6, 10, -6, 2, 12, 12, 0, true], [16, 40, -6, 8, -6, 12, 2, 12], [16, 40, -6, 22, -6, 12, 2, 12]],
      pivot: [0, 0, 0], look: true,
    },
  };
  const rx = [1.75, 0.25, 0, 0, 0.5, 0.5, 0.5, 0.5, 1.25, 0.75, 0, 0];
  const ry = [0, 0, 0, 0, 0.25, 1.75, 1.25, 0.75, 0, 0, 0, 0];
  const rz = [0, 0, 0.25, 1.75, 0, 0, 0, 0, 0, 0, 0.75, 1.25];
  const px = [0, 0, 8, -8, -8, 8, 8, -8, 0, 0, 8, -8];
  const py = [-8, -8, -8, -8, 0, 0, 0, 0, 8, 8, 8, 8];
  const pz = [8, -8, 0, 0, -8, -8, 8, 8, 8, -8, 0, 0];
  for (let i = 0; i < 12; i++) parts[`spine${i}`] = { boxes: [[0, 0, -1, -4.5, -1, 2, 9, 2]], pivot: [px[i], 16 + py[i], pz[i]], rot: [PI * rx[i], PI * ry[i], PI * rz[i]], parent: 'body' };
  parts.eye = { boxes: [[8, 0, -1, 15, 0, 2, 2, 1]], pivot: [0, 0, -8.25], parent: 'body' };
  parts.tail0 = { boxes: [[40, 0, -2, 14, 7, 4, 4, 8]], pivot: [0, 0, 0], parent: 'body' };
  parts.tail1 = { boxes: [[0, 54, 0, 14, 0, 3, 3, 7]], pivot: [-1.5, 0.5, 14], parent: 'tail0' };
  parts.tail2 = { boxes: [[41, 32, 0, 14, 0, 2, 2, 6], [25, 19, 1, 10.5, 3, 1, 9, 9]], pivot: [0.5, 0.5, 6], parent: 'tail1' };
  return {
    texture: (v) => (v ? 'entity/guardian_elder' : 'entity/guardian'), tw: 64, th: 64, parts, scale: (v) => (v ? 2.35 : 1),
    animate: (p, poses) => {
      for (let i = 0; i < 12; i++) {
        const k = 1 + Math.cos(p.age * 1.5 + i) * 0.01;
        Object.assign(poses[`spine${i}`], { px: px[i] * k, py: 16 + py[i] * k, pz: pz[i] * k });
      }
      const t = Math.sin(p.age * 0.15);
      poses.tail0.ry = t * PI * 0.05; poses.tail1.ry = t * PI * 0.1; poses.tail2.ry = t * PI * 0.15;
    },
  };
}

/** WitherEntityModel (64×64, WitherEntityRenderer: 2 ×): shoulders, the ribcage and tail breathing, three heads (the side ones look ahead). */
function wither(): MobDef {
  return {
    texture: 'entity/wither/wither', tw: 64, th: 64, scale: 2,
    parts: {
      shoulders: { boxes: [[0, 16, -10, 3.9, -0.5, 20, 3, 3]], pivot: [0, 0, 0] },
      ribs: { boxes: [[0, 22, 0, 0, 0, 3, 10, 3], [24, 22, -4, 1.5, 0.5, 11, 2, 2], [24, 22, -4, 4, 0.5, 11, 2, 2], [24, 22, -4, 6.5, 0.5, 11, 2, 2]], pivot: [-2, 6.9, -0.5] },
      tail: { boxes: [[12, 22, 0, 0, 0, 3, 6, 3]], pivot: [-2, 16.9, -0.5] },
      head: { boxes: [[0, 0, -4, -4, -4, 8, 8, 8]], pivot: [0, 0, 0], look: true },
      head2: { boxes: [[32, 0, -4, -4, -4, 6, 6, 6]], pivot: [-8, 4, 0] },
      head3: { boxes: [[32, 0, -4, -4, -4, 6, 6, 6]], pivot: [10, 4, 0] },
    },
    animate: (p, poses) => {
      const f = Math.cos(p.age * 0.1);
      const r = (0.065 + 0.05 * f) * PI;
      poses.ribs.rx = r;
      Object.assign(poses.tail, { py: 6.9 + Math.cos(r) * 10, pz: -0.5 + Math.sin(r) * 10, rx: (0.265 + 0.1 * f) * PI });
    },
  };
}

/** Mirror image of a part (the dragon's left wing and legs: EnderDragonModel draws them again under scale(−1, 1, 1)). */
function mirrored(p: PartDef, parent?: string): PartDef {
  const r = p.rot ?? [0, 0, 0];
  return { ...p, boxes: p.boxes.map(([u, v, x, y, z, w, h, d, inf, m]) => [u, v, -(x + w), y, z, w, h, d, inf ?? 0, !m] as Box), pivot: [-p.pivot[0], p.pivot[1], p.pivot[2]], rot: [r[0], -r[1], -r[2]], parent };
}

/**
 * EnderDragonModel (256×256), hovering straight (every movement offset equal): five neck segments, the head and
 * jaw, the body, both wings and legs, twelve tail segments, wings beating with the age. The root holds
 * EnderDragonEntityRenderer's turn (−yaw, not 180 − yaw), its 1 block forward and the model's own (0, f1 − 2, −3) blocks.
 */
function dragon(): MobDef {
  const neck: Box[] = [[192, 104, -5, -5, -5, 10, 10, 10], [48, 0, -1, -9, -3, 2, 4, 6]];
  const parts: Record<string, PartDef> = { root: { boxes: [], pivot: [0, -31.7, 32], rot: [0, PI, 0] } };
  for (let i = 0; i < 5; i++) parts[`neck${i}`] = { boxes: neck, pivot: [0, 0, 0], parent: 'root' };
  parts.head = {
    boxes: [[176, 44, -6, -1, -24, 12, 5, 16], [112, 30, -8, -8, -10, 16, 16, 16], [0, 0, -5, -12, -4, 2, 4, 6, 0, true], [112, 0, -5, -3, -22, 2, 2, 4, 0, true],
      [0, 0, 3, -12, -4, 2, 4, 6], [112, 0, 3, -3, -22, 2, 2, 4]],
    pivot: [0, 0, 0], parent: 'root',
  };
  parts.jaw = { boxes: [[176, 65, -6, 0, -16, 12, 4, 16]], pivot: [0, 4, -8], parent: 'head' };
  parts.body = { boxes: [[0, 0, -12, 0, -16, 24, 24, 64], [220, 53, -1, -6, -10, 2, 6, 12], [220, 53, -1, -6, 10, 2, 6, 12], [220, 53, -1, -6, 30, 2, 6, 12]], pivot: [0, 4, 8], parent: 'root' };
  const right: Record<string, PartDef> = {
    wing: { boxes: [[112, 88, -56, -4, -4, 56, 8, 8], [-56, 88, -56, 0, 2, 56, 0, 56]], pivot: [-12, 5, 2], parent: 'root' },
    wingTip: { boxes: [[112, 136, -56, -2, -2, 56, 4, 4], [-56, 144, -56, 0, 2, 56, 0, 56]], pivot: [-56, 0, 0], parent: 'wing' },
    frontLeg: { boxes: [[112, 104, -4, -4, -4, 8, 24, 8]], pivot: [-12, 20, 2], parent: 'root' },
    frontLegTip: { boxes: [[226, 138, -3, -1, -3, 6, 24, 6]], pivot: [0, 20, -1], parent: 'frontLeg' },
    frontFoot: { boxes: [[144, 104, -4, 0, -12, 8, 4, 16]], pivot: [0, 23, 0], parent: 'frontLegTip' },
    rearLeg: { boxes: [[0, 0, -8, -4, -8, 16, 32, 16]], pivot: [-16, 16, 42], parent: 'root' },
    rearLegTip: { boxes: [[196, 0, -6, -2, 0, 12, 32, 12]], pivot: [0, 32, -4], parent: 'rearLeg' },
    rearFoot: { boxes: [[112, 0, -9, 0, -20, 18, 6, 24]], pivot: [0, 31, 4], parent: 'rearLegTip' },
  };
  for (const [k, p] of Object.entries(right)) parts[`r${k}`] = { ...p, parent: p.parent === 'root' ? 'root' : `r${p.parent}` };
  for (const [k, p] of Object.entries(right)) parts[`l${k}`] = mirrored(p, p.parent === 'root' ? 'root' : `l${p.parent}`);
  for (let i = 0; i < 12; i++) parts[`tail${i}`] = { boxes: neck, pivot: [0, 0, 0], parent: 'root' };
  return {
    texture: 'entity/enderdragon/dragon', tw: 256, th: 256, parts,
    animate: (p, poses) => {
      const f = p.age * 0.02, f8 = f * PI * 2;
      poses.jaw.rx = (Math.sin(f8) + 1) * 0.2;
      let f1 = Math.sin(f8 - 1) + 1;
      f1 = (f1 * f1 + f1 * 2) * 0.05;
      // the neck: each segment 10 px further along its own pitch
      let y = 20, z = -12, x = 0;
      for (let i = 0; i < 5; i++) {
        const rx = Math.cos(i * 0.45 + f8) * 0.15;
        Object.assign(poses[`neck${i}`], { px: x, py: y, pz: z, rx });
        y += Math.sin(rx) * 10; z -= Math.cos(rx) * 10;
      }
      Object.assign(poses.head, { px: x, py: y, pz: z });
      for (const side of ['r', 'l']) {
        const m = side === 'r' ? 1 : -1;
        Object.assign(poses[`${side}wing`], { rx: 0.125 - Math.cos(f8) * 0.2, ry: 0.25 * m, rz: (Math.sin(f8) + 0.125) * 0.8 * m });
        poses[`${side}wingTip`].rz = -(Math.sin(f8 + 2) + 0.5) * 0.75 * m;
        poses[`${side}rearLeg`].rx = 1 + f1 * 0.1;
        poses[`${side}rearLegTip`].rx = 0.5 + f1 * 0.1;
        poses[`${side}rearFoot`].rx = 0.75 + f1 * 0.1;
        poses[`${side}frontLeg`].rx = 1.3 + f1 * 0.1;
        poses[`${side}frontLegTip`].rx = -0.5 - f1 * 0.1;
        poses[`${side}frontFoot`].rx = 0.75 + f1 * 0.1;
      }
      // the tail, turned round (180°), waving
      let f10 = 0;
      y = 10; z = 60; x = 0;
      for (let k = 0; k < 12; k++) {
        f10 += Math.sin(k * 0.45 + f8) * 0.05;
        Object.assign(poses[`tail${k}`], { px: x, py: y, pz: z, rx: f10, ry: PI });
        y += Math.sin(f10) * 10; z -= Math.cos(PI) * Math.cos(f10) * 10;
      }
    },
  };
}

/** Sheep fleece colours (SheepEntity.COLORS, from the dye colours). */
export const FLEECE: Array<[number, number, number]> = [
  [1, 1, 1], [0.85, 0.5, 0.2], [0.7, 0.3, 0.85], [0.4, 0.6, 0.85], [0.9, 0.9, 0.2], [0.5, 0.8, 0.1], [0.95, 0.5, 0.65], [0.3, 0.3, 0.3],
  [0.6, 0.6, 0.6], [0.3, 0.5, 0.6], [0.5, 0.25, 0.7], [0.2, 0.3, 0.7], [0.4, 0.3, 0.2], [0.4, 0.5, 0.2], [0.6, 0.2, 0.2], [0.1, 0.1, 0.1],
];

export const MOBS: Record<string, MobDef> = {
  Spider: spider('entity/spider/spider'),
  CaveSpider: spider('entity/spider/cave_spider', 0.7),
  Chicken: {
    texture: 'entity/chicken', tw: 64, th: 32,
    parts: {
      head: { boxes: [[0, 0, -2, -6, -2, 4, 6, 3]], pivot: [0, 15, -4], look: true },
      bill: { boxes: [[14, 0, -2, -4, -4, 4, 2, 2]], pivot: [0, 0, 0], parent: 'head' },
      chin: { boxes: [[14, 4, -1, -2, -3, 2, 2, 2]], pivot: [0, 0, 0], parent: 'head' },
      body: { boxes: [[0, 9, -3, -4, -3, 6, 8, 6]], pivot: [0, 16, 0], rot: [PI / 2, 0, 0] },
      rightLeg: { boxes: [[26, 0, -1, 0, -3, 3, 5, 3]], pivot: [-2, 19, 1], walk: 1 },
      leftLeg: { boxes: [[26, 0, -1, 0, -3, 3, 5, 3]], pivot: [1, 19, 1], walk: -1 },
      rightWing: { boxes: [[24, 13, 0, 0, -3, 1, 4, 6]], pivot: [-4, 13, 0] },
      leftWing: { boxes: [[24, 13, -1, 0, -3, 1, 4, 6]], pivot: [4, 13, 0] },
    },
    baby: quadBaby(5, 2),
  },
  Enderman: {
    // EndermanEntityModel: a biped 14 px higher with 30 px thin limbs (setAngles moves the pivots)
    texture: 'entity/enderman/enderman', tw: 64, th: 32,
    parts: {
      head: { boxes: [[0, 0, -4, -8, -4, 8, 8, 8]], pivot: [0, -13, 0], look: true },
      headwear: { boxes: [[0, 16, -4, -8, -4, 8, 8, 8, -0.5]], pivot: [0, -13, 0], look: true },
      body: { boxes: [[32, 16, -4, 0, -2, 8, 12, 4]], pivot: [0, -14, 0] },
      // BiPedModel's swing halved (arms swing at 1/1.4 of the legs, against them)
      rightArm: { boxes: [[56, 0, -1, -2, -1, 2, 30, 2]], pivot: [-5, -12, 0], walk: -0.36 },
      leftArm: { boxes: [[56, 0, -1, -2, -1, 2, 30, 2, 0, true]], pivot: [5, -12, 0], walk: 0.36 },
      rightLeg: { boxes: [[56, 0, -1, 0, -1, 2, 30, 2]], pivot: [-2, -5, 0], walk: 0.5 },
      leftLeg: { boxes: [[56, 0, -1, 0, -1, 2, 30, 2, 0, true]], pivot: [2, -5, 0], walk: -0.5 },
    },
  },
  // VillagerEntityRenderer / WitchEntityRenderer draw them at 0.9375, a child villager at half of that
  Villager: { texture: (v) => VILLAGER_SKINS[v] ?? 'entity/villager/villager', tw: 64, th: 64, parts: villagerParts(), scale: 0.9375, baby: wholeBaby(0.46875) },
  Witch: {
    texture: 'entity/witch', tw: 64, th: 128, scale: 0.9375,
    parts: {
      ...villagerParts(),
      mole: { boxes: [[0, 0, 0, 3, -6.75, 1, 1, 1, -0.25]], pivot: [0, -2, 0], parent: 'nose' },
      hat: { boxes: [[0, 64, 0, 0, 0, 10, 2, 10]], pivot: [-5, -10.03125, -5], parent: 'head' },
      hat2: { boxes: [[0, 76, 0, 0, 0, 7, 4, 7]], pivot: [1.75, -4, 2], rot: [-0.05235988, 0, 0.02617994], parent: 'hat' },
      hat3: { boxes: [[0, 87, 0, 0, 0, 4, 4, 4]], pivot: [1.75, -4, 2], rot: [-0.10471976, 0, 0.05235988], parent: 'hat2' },
      hat4: { boxes: [[0, 95, 0, 0, 0, 1, 2, 1, 0.25]], pivot: [1.75, -2, 2], rot: [-0.20943952, 0, 0.10471976], parent: 'hat3' },
    },
  },
  Slime: {
    // SlimeEntityModel(16) inside (core, eyes, mouth), SlimeEntityModel(0) as the see-through gel; scaled by the size
    texture: 'entity/slime/slime', tw: 64, th: 32, scale: (v) => Math.max(1, v || 1),
    parts: {
      inner: { boxes: [[0, 16, -3, 17, -3, 6, 6, 6]], pivot: [0, 0, 0] },
      rightEye: { boxes: [[32, 0, -3.25, 18, -3.5, 2, 2, 2]], pivot: [0, 0, 0] },
      leftEye: { boxes: [[32, 4, 1.25, 18, -3.5, 2, 2, 2]], pivot: [0, 0, 0] },
      mouth: { boxes: [[32, 8, 0, 21, -3.5, 1, 1, 1]], pivot: [0, 0, 0] },
      outer: { boxes: [[0, 0, -4, 16, -4, 8, 8, 8]], pivot: [0, 0, 0], translucent: true },
    },
  },
  Wolf: {
    // variant: 1 sitting, 2 angry, 4 tame, collar dye << 4 (WolfEntityRenderer, WolfCollarFeatureRenderer)
    texture: (v) => (v & 4 ? 'entity/wolf/wolf_tame' : v & 2 ? 'entity/wolf/wolf_angry' : 'entity/wolf/wolf'), tw: 64, th: 32,
    layers: (v) => (v & 4 ? [['entity/wolf/wolf_collar', dyeColor(v >> 4)]] : []),
    baby: quadBaby(5, 2),
    parts: {
      head: { boxes: [[0, 0, -3, -3, -2, 6, 6, 4], [16, 14, -3, -5, 0, 2, 2, 1], [16, 14, 1, -5, 0, 2, 2, 1], [0, 10, -1.5, 0, -5, 3, 3, 4]], pivot: [-1, 13.5, -7], look: true },
      body: { boxes: [[18, 14, -4, -2, -3, 6, 9, 6]], pivot: [0, 14, 2], rot: [PI / 2, 0, 0] },
      mane: { boxes: [[21, 0, -4, -3, -3, 8, 6, 7]], pivot: [-1, 14, -3], rot: [PI / 2, 0, 0] },
      leg1: { boxes: [[0, 18, -1, 0, -1, 2, 8, 2]], pivot: [-2.5, 16, 7], walk: 1 },
      leg2: { boxes: [[0, 18, -1, 0, -1, 2, 8, 2]], pivot: [0.5, 16, 7], walk: -1 },
      leg3: { boxes: [[0, 18, -1, 0, -1, 2, 8, 2]], pivot: [-2.5, 16, -4], walk: -1 },
      leg4: { boxes: [[0, 18, -1, 0, -1, 2, 8, 2]], pivot: [0.5, 16, -4], walk: 1 },
      // the tail's angle of a wild wolf (WolfEntity.method_2882: π/5)
      tail: { boxes: [[9, 18, -1, 0, -1, 2, 8, 2]], pivot: [-1, 12, 8], rot: [PI / 5, 0, 0] },
    },
    animate: (p, poses) => {
      // method_2882: angry 1.54, tame by health (full health assumed), wild π/5
      if (p.variant & 2) poses.tail.rx = 1.5393804;
      else if (p.variant & 4) poses.tail.rx = 0.55 * PI;
      if (!(p.variant & 1)) return;
      // WolfEntityModel.animateModel, sitting
      Object.assign(poses.mane, { px: -1, py: 16, pz: -3, rx: (PI * 2) / 5, ry: 0 });
      Object.assign(poses.body, { px: 0, py: 18, pz: 0, rx: PI / 4 });
      Object.assign(poses.tail, { px: -1, py: 21, pz: 6 });
      Object.assign(poses.leg1, { px: -2.5, py: 22, pz: 2, rx: (PI * 3) / 2 });
      Object.assign(poses.leg2, { px: 0.5, py: 22, pz: 2, rx: (PI * 3) / 2 });
      Object.assign(poses.leg3, { px: -2.49, py: 17, pz: -4, rx: 5.811947 });
      Object.assign(poses.leg4, { px: 0.51, py: 17, pz: -4, rx: 5.811947 });
    },
  },
  EntityHorse: horse(),
  VillagerGolem: {
    texture: 'entity/iron_golem', tw: 128, th: 128,
    parts: {
      head: { boxes: [[0, 0, -4, -12, -5.5, 8, 10, 8], [24, 0, -1, -5, -7.5, 2, 4, 2]], pivot: [0, -7, -2], look: true },
      body: { boxes: [[0, 40, -9, -2, -6, 18, 12, 11], [0, 70, -4.5, 10, -3, 9, 5, 6, 0.5]], pivot: [0, -7, 0] },
      rightArm: { boxes: [[60, 21, -13, -2.5, -3, 4, 30, 6]], pivot: [0, -7, 0], walk: 0.6 },
      leftArm: { boxes: [[60, 58, 9, -2.5, -3, 4, 30, 6]], pivot: [0, -7, 0], walk: -0.6 },
      leftLeg: { boxes: [[37, 0, -3.5, -3, -3, 6, 16, 5]], pivot: [-4, 11, 0], walk: -0.75 },
      rightLeg: { boxes: [[60, 0, -3.5, -3, -3, 6, 16, 5, 0, true]], pivot: [5, 11, 0], walk: 0.75 },
    },
  },
  Blaze: blaze(),
  Squid: squid(),
  Ghast: ghast(),
  LavaSlime: magmaCube(),
  Silverfish: segmented([[3, 2, 2], [4, 3, 2], [6, 4, 3], [3, 3, 3], [2, 2, 3], [2, 1, 2], [1, 1, 2]], [[0, 0], [0, 4], [0, 9], [0, 16], [0, 22], [11, 0], [13, 4]], 'entity/silverfish', 0.05, 0.2),
  Endermite: segmented([[4, 3, 2], [6, 4, 5], [3, 3, 1], [1, 2, 1]], [[0, 0], [0, 5], [0, 14], [0, 18]], 'entity/endermite', 0.01, 0.1),
  Bat: bat(),
  Ozelot: ocelot(),
  Rabbit: rabbit(),
  SnowMan: snowGolem(),
  Guardian: guardian(),
  WitherBoss: wither(),
  EnderDragon: dragon(),
};

/** The sheep's fleece (SheepWoolEntityModel), drawn over the shorn skin tinted with the fleece colour unless sheared. */
export const SHEEP_FUR: MobDef = {
  texture: 'entity/sheep/sheep_fur', tw: 64, th: 32,
  parts: {
    head: { boxes: [[0, 0, -3, -4, -4, 6, 6, 6, 0.6]], pivot: [0, 6, -8], look: true },
    body: { boxes: [[28, 8, -4, -10, -7, 8, 16, 6, 1.75]], pivot: [0, 5, 2], rot: [PI / 2, 0, 0] },
    leg1: { boxes: [[0, 16, -2, 0, -2, 4, 6, 4, 0.5]], pivot: [-3, 12, 7], walk: 1 },
    leg2: { boxes: [[0, 16, -2, 0, -2, 4, 6, 4, 0.5]], pivot: [3, 12, 7], walk: -1 },
    leg3: { boxes: [[0, 16, -2, 0, -2, 4, 6, 4, 0.5]], pivot: [-3, 12, -5], walk: -1 },
    leg4: { boxes: [[0, 16, -2, 0, -2, 4, 6, 4, 0.5]], pivot: [3, 12, -5], walk: 1 },
  },
};

/** Child rendering of the entities.ts models (bipeds and quadrupeds: PigEntityModel sets field_1514 4). */
export const LIVING_BABY: Record<string, Baby> = {
  Zombie: bipedBaby(), PigZombie: bipedBaby(), Pig: quadBaby(4), Cow: quadBaby(), MushroomCow: quadBaby(), Sheep: quadBaby(),
};
SHEEP_FUR.baby = quadBaby();
/** Whole-model scale of the entities.ts models (GiantEntityRenderer: a zombie at 6 ×). */
export const LIVING_SCALE: Record<string, number> = { Giant: 6 };

/** A baby's extra transform for a top-level part (scale, then translate in pixels), or null when it is not a baby. */
export function babyGroup(baby: Baby | undefined, part: string): [number, number, number, number] | null {
  if (!baby) return null;
  const head = baby.head.includes(part);
  const t = head ? baby.ht : baby.bt;
  return [head ? baby.hs : baby.bs, t[0], t[1], t[2]];
}

/** Every texture the mob models can ask for (the server packs them: images.json). */
export function mobTextures(): string[] {
  const out = new Set<string>();
  for (const d of [...Object.values(MOBS), SHEEP_FUR]) {
    if (typeof d.texture === 'string') out.add(d.texture);
  }
  for (const v of VILLAGER_SKINS) out.add(v);
  out.add('entity/villager/villager');
  for (let t = 0; t <= 4; t++) for (let c = 0; c < (t ? 1 : HORSE_COLORS.length); c++) out.add(horseTexture((t << 8) | c));
  out.add('entity/skeleton/wither_skeleton');
  out.add('entity/zombie_pigman');
  out.add('entity/cow/mooshroom');
  out.add('entity/zombie/zombie_villager');
  for (const t of ['wolf_tame', 'wolf_angry', 'wolf_collar']) out.add(`entity/wolf/${t}`);
  for (const t of [...OCELOTS, ...RABBITS.map(r => `entity/rabbit/${r}`), 'entity/rabbit/caerbannog', 'entity/ghast/ghast', 'entity/ghast/ghast_shooting', 'entity/guardian', 'entity/guardian_elder']) out.add(t);
  return [...out];
}

/**
 * The pose of every part: the rest pose, head look, walk swing, then the mob's own animation.
 * Children keep their own pose; the renderer multiplies them into their parent's matrix.
 */
export function posesOf(def: MobDef, p: PoseIn): Record<string, Pose> {
  const out: Record<string, Pose> = {};
  const swing = Math.cos(p.swing * 0.6662) * 1.4 * p.swingAmt;
  for (const [name, part] of Object.entries(def.parts)) {
    const r = part.rot ?? [0, 0, 0];
    const pose: Pose = { px: part.pivot[0], py: part.pivot[1], pz: part.pivot[2], rx: r[0], ry: r[1], rz: r[2] };
    if (part.look) { pose.ry += p.headYaw; if (part.look !== 'yaw') pose.rx += p.pitch; }
    if (part.walk) pose.rx += swing * part.walk;
    out[name] = pose;
  }
  def.animate?.(p, out);
  return out;
}

/** Highest texture coordinate a box reads (u, v): for the tests' "fits the texture" check. */
export function boxUvExtent(b: Box): [number, number] {
  const [u, v, , , , w, h, d] = b;
  return [u + 2 * (d + w), v + d + h];
}

// ---- tile entities drawn like entities: skulls and banners ----------------------------------------

/** Skull textures by SkullType (0 skeleton, 1 wither, 2 zombie, 3 player, 4 creeper) and their height: 64-high skins get the hat layer. */
export const SKULLS: Array<[string, number]> = [
  ['entity/skeleton/skeleton', 32], ['entity/skeleton/wither_skeleton', 32], ['entity/zombie/zombie', 64], ['entity/steve', 64], ['entity/creeper/creeper', 32],
];

/**
 * Where a skull sits (SkullBlockEntityRenderer): on the floor (block meta 1) at the block centre turned by
 * Rot × 22.5°, or on a wall (meta 2..5 facing north/south/west/east) 4 px up with its back on the wall.
 * Block-relative position of the head's bottom centre and the head's y rotation (ModelPart.posY) (MC model space,
 * under the same y/x flip as entities: 0 faces north).
 */
export function skullPlace(meta: number, rot: number): { x: number; y: number; z: number; ry: number } {
  switch (meta & 7) {
    case 2: return { x: 0.5, y: 0.25, z: 0.74, ry: 0 };
    case 3: return { x: 0.5, y: 0.25, z: 0.26, ry: PI };
    case 4: return { x: 0.74, y: 0.25, z: 0.5, ry: PI * 1.5 };
    case 5: return { x: 0.26, y: 0.25, z: 0.5, ry: PI / 2 };
    default: return { x: 0.5, y: 0, z: 0.5, ry: (rot & 15) * PI / 8 };
  }
}

/** BannerPattern: NBT code → texture under entity/banner/. */
export const BANNER_PATTERNS: Array<[string, string]> = [
  ['b', 'base'], ['bl', 'square_bottom_left'], ['br', 'square_bottom_right'], ['tl', 'square_top_left'], ['tr', 'square_top_right'],
  ['bs', 'stripe_bottom'], ['ts', 'stripe_top'], ['ls', 'stripe_left'], ['rs', 'stripe_right'], ['cs', 'stripe_center'], ['ms', 'stripe_middle'],
  ['drs', 'stripe_downright'], ['dls', 'stripe_downleft'], ['ss', 'small_stripes'], ['cr', 'cross'], ['sc', 'straight_cross'],
  ['bt', 'triangle_bottom'], ['tt', 'triangle_top'], ['bts', 'triangles_bottom'], ['tts', 'triangles_top'], ['ld', 'diagonal_left'],
  ['rd', 'diagonal_up_right'], ['lud', 'diagonal_up_left'], ['rud', 'diagonal_right'], ['mc', 'circle'], ['mr', 'rhombus'],
  ['vh', 'half_vertical'], ['hh', 'half_horizontal'], ['vhr', 'half_vertical_right'], ['hhb', 'half_horizontal_bottom'], ['bo', 'border'],
  ['cbo', 'curly_border'], ['cre', 'creeper'], ['gra', 'gradient'], ['gru', 'gradient_up'], ['bri', 'bricks'], ['sku', 'skull'],
  ['flo', 'flower'], ['moj', 'mojang'],
];

/** BannerBlockEntityModel (64×64): the cloth (BannerBlockEntityRenderer moves it to y -32), the pole (standing only), the bar. */
export const BANNER: MobDef = {
  texture: 'entity/banner_base', tw: 64, th: 64,
  parts: {
    slate: { boxes: [[0, 0, -10, 0, -2, 20, 40, 1]], pivot: [0, -32, 0] },
    stand: { boxes: [[44, 0, -1, -30, -1, 2, 42, 2]], pivot: [0, 0, 0] },
    top: { boxes: [[0, 42, -10, -32, -1, 20, 2, 2]], pivot: [0, 0, 0] },
  },
};

/**
 * Where a banner sits (BannerBlockEntityRenderer): standing (block 176, meta = rotation) on a pole from the
 * block centre 0.5 up, or on a wall (block 177, meta 2..5 facing north/south/west/east) hung from its bar
 * against the wall. Block-relative pivot and yaw (rotate(-angle) about y); the model is drawn at 2/3
 * size with y and z flipped, the pole only when standing.
 */
export function bannerPlace(standing: boolean, meta: number): { x: number; y: number; z: number; yaw: number; pole: boolean } {
  const f = 2 / 3;
  if (standing) return { x: 0.5, y: 0.75 * f, z: 0.5, yaw: -(meta & 15) * PI / 8, pole: true };
  const yaw = meta === 2 ? PI : meta === 4 ? -PI / 2 : meta === 5 ? PI / 2 : 0;
  // translate(0, -0.3125, -0.4375) after the turn: down and towards the wall
  return { x: 0.5 - Math.sin(yaw) * 0.4375, y: -0.25 * f - 0.3125, z: 0.5 - Math.cos(yaw) * 0.4375, yaw, pole: false };
}

/** Dye damage (NBT Base / Color, 0 black .. 15 white) → fleece colour. */
export const dyeColor = (damage: number): [number, number, number] => FLEECE[15 - (damage & 15)];
