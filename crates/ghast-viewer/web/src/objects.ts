/**
 * Object models (1.8 spawn_entity objects, named by the server): armour stands, minecarts and boats,
 * as mobs.ts definitions so the same renderer draws them. Boxes from ArmorStandEntityModel, MinecartEntityModel and
 * BoatEntityModel; `lift` puts the model's origin where MinecartEntityRenderer (0.375) and BoatEntityRenderer (0.25) translate it.
 */
import { armourTextures } from './armour.js';
import { bipedBaby, type MobDef, type PartDef } from './mobs.js';

const PI = Math.PI;
const D = PI / 180;

/** ArmorStandEntityModel (64×64): stick head, shoulders, arms (variant flag 4), legs, the hip sticks and base plate (flag 8 hides it). */
const ARMOR_STAND: MobDef = {
  texture: 'entity/armorstand/wood', tw: 64, th: 64,
  parts: {
    head: { boxes: [[0, 0, -1, -7, -1, 2, 7, 2]], pivot: [0, 1, 0] },
    body: { boxes: [[0, 26, -6, 0, -1.5, 12, 3, 3]], pivot: [0, 0, 0] },
    rightArm: { boxes: [[24, 0, -2, -2, -1, 2, 12, 2]], pivot: [-5, 2, 0] },
    leftArm: { boxes: [[32, 16, 0, -2, -1, 2, 12, 2, 0, true]], pivot: [5, 2, 0] },
    rightLeg: { boxes: [[8, 0, -1, 0, -1, 2, 11, 2]], pivot: [-1.9, 12, 0] },
    leftLeg: { boxes: [[40, 16, -1, 0, -1, 2, 11, 2, 0, true]], pivot: [1.9, 12, 0] },
    standRightSide: { boxes: [[16, 0, -3, 3, -1, 2, 7, 2]], pivot: [0, 0, 0] },
    standLeftSide: { boxes: [[48, 16, 1, 3, -1, 2, 7, 2]], pivot: [0, 0, 0] },
    standWaist: { boxes: [[0, 48, -4, 10, -1, 8, 2, 2]], pivot: [0, 0, 0] },
    standBase: { boxes: [[0, 32, -6, 11, -6, 12, 1, 12]], pivot: [0, 12, 0] },
  },
  // a small armour stand is a child biped (ArmorStandEntity.isBaby = isSmall)
  baby: bipedBaby(),
  hidden: (v) => [...(v & 4 ? [] : ['rightArm', 'leftArm']), ...(v & 8 ? ['standBase'] : [])],
  animate: (p, poses) => {
    // the pose from metadata 11..16 in degrees (LOOKS); the hip sticks follow the body
    const r = p.rot;
    if (!r) return;
    const set = (name: string, i: number) => Object.assign(poses[name], { rx: r[i] * D, ry: r[i + 1] * D, rz: r[i + 2] * D });
    set('head', 0); set('body', 3); set('leftArm', 6); set('rightArm', 9); set('leftLeg', 12); set('rightLeg', 15);
    for (const k of ['standRightSide', 'standLeftSide', 'standWaist']) set(k, 3);
  },
};

/** MinecartEntityModel (64×32): the floor and four sides of a 20×16 tub, long along x; MinecartEntityRenderer translates 0.375 up. */
function minecart(): MobDef {
  const side = (px: number, pz: number, ry: number): PartDef => ({ boxes: [[0, 0, -8, -9, -1, 16, 8, 2]], pivot: [px, 4, pz], rot: [0, ry, 0] });
  return {
    texture: 'entity/minecart', tw: 64, th: 32, lift: 0.375 - 1.5,
    parts: {
      bottom: { boxes: [[0, 10, -10, -8, -1, 20, 16, 2]], pivot: [0, 4, 0], rot: [PI / 2, 0, 0] },
      // the inner floor (MinecartEntityModel.render: pivotY = 4 − (−0.1))
      floor: { boxes: [[44, 10, -9, -7, -1, 18, 14, 1]], pivot: [0, 4.1, 0], rot: [-PI / 2, 0, 0] },
      back: side(-9, 0, (PI * 3) / 2), front: side(9, 0, PI / 2), left: side(0, -7, PI), right: side(0, 7, 0),
    },
  };
}

/** BoatEntityModel (64×32): a 24×16 bottom and four sides; BoatEntityRenderer translates 0.25 up. */
function boat(): MobDef {
  const side = (px: number, pz: number, ry: number): PartDef => ({ boxes: [[0, 0, -10, -7, -1, 20, 6, 2]], pivot: [px, 4, pz], rot: [0, ry, 0] });
  return {
    texture: 'entity/boat', tw: 64, th: 32, lift: 0.25 - 1.5,
    parts: {
      bottom: { boxes: [[0, 8, -12, -8, -3, 24, 16, 4]], pivot: [0, 4, 0], rot: [PI / 2, 0, 0] },
      back: side(-11, 0, (PI * 3) / 2), front: side(11, 0, PI / 2), left: side(0, -9, PI), right: side(0, 9, 0),
    },
  };
}

const CART = minecart();
export const OBJECT_MODELS: Record<string, MobDef> = {
  ArmorStand: ARMOR_STAND,
  Boat: boat(),
  ...Object.fromEntries(['MinecartRideable', 'MinecartChest', 'MinecartFurnace', 'MinecartTNT', 'MinecartSpawner', 'MinecartHopper', 'MinecartCommandBlock'].map(n => [n, CART])),
};

/**
 * The block in a minecart (MinecartEntityRenderer): its SPAWN variant (id << 4 | meta), drawn 0.75 × centred over the
 * cart's origin, its bottom (offset − 8) / 16 × 0.75 blocks from there (offset 6, hoppers 1), turned 90°.
 */
export function minecartBlock(name: string, variant: number): { id: number; meta: number; dy: number; scale: number } | null {
  if (!name.startsWith('Minecart') || !variant) return null;
  const offset = name === 'MinecartHopper' ? 1 : 6;
  return { id: variant >> 4, meta: variant & 15, dy: 0.375 + ((offset - 8) / 16) * 0.75, scale: 0.75 };
}

/** Every texture the object models and the armour layers can ask for (the server packs them: images.json). */
export function objectTextures(): string[] {
  return [...new Set([...Object.values(OBJECT_MODELS).map(d => d.texture as string), ...armourTextures()])];
}
