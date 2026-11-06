/**
 * Armour on bipeds (ArmorRenderer): BiPedModel(1.0F) for the helmet, chestplate and boots on
 * models/armor/<material>_layer_1, BiPedModel(0.5F) for the leggings on _layer_2; leather is drawn in its
 * dye colour with the uncoloured _overlay on top. The armour parts take the wearer's part matrices
 * (BiPedModel.copy copies the angles and rotation points), so they follow every pose.
 * Pure data (no GL), like mobs.ts.
 */
import type { Box, PartDef } from './mobs.js';

export const ARMOUR_MATERIALS = ['leather', 'chainmail', 'iron', 'diamond', 'gold'];

/** Leather's colour without a display.color tag (ArmorItem.getColor: 10511680). */
export const LEATHER_DEFAULT = 0xa06540;

/** Item id → [material, slot 1..4 as in entity_equipment (boots, leggings, chestplate, helmet)], or null. */
export function armourPiece(id: number): [string, number] | null {
  // 298..317: leather, chainmail, iron, diamond, gold; each helmet, chestplate, leggings, boots
  if (id < 298 || id > 317) return null;
  const k = id - 298;
  return [ARMOUR_MATERIALS[Math.floor(k / 4)], 4 - (k % 4)];
}

/** The texture of a piece: layer 2 for leggings, `overlay` for leather's uncoloured second pass. */
export function armourTexture(material: string, slot: number, overlay = false): string {
  return `models/armor/${material}_layer_${slot === 2 ? 2 : 1}${overlay ? '_overlay' : ''}`;
}

/** Parts each slot shows (ArmorRenderer.setVisible). */
export const ARMOUR_PARTS: Record<number, string[]> = {
  1: ['rightLeg', 'leftLeg'],
  2: ['body', 'rightLeg', 'leftLeg'],
  3: ['body', 'rightArm', 'leftArm'],
  4: ['head'],
};

/**
 * BiPedModel(inflate) on a 64×32 texture, the head with its headwear (inflate + 0.5) in one part. Only the boxes
 * matter: the wearer's matrices place them. The zombie villager's helmet sits 2 px higher on its taller head
 * (ZombieVillagerEntityModel(f, 0, true)).
 */
export function armourParts(inflate: number, villagerHead = false): Record<string, PartDef> {
  const f = inflate, at: [number, number, number] = [0, 0, 0];
  const head: Box[] = villagerHead
    ? [[0, 0, -4, -10, -4, 8, 8, 8, f]]
    : [[0, 0, -4, -8, -4, 8, 8, 8, f], [32, 0, -4, -8, -4, 8, 8, 8, f + 0.5]];
  return {
    head: { boxes: head, pivot: at },
    body: { boxes: [[16, 16, -4, 0, -2, 8, 12, 4, f]], pivot: at },
    rightArm: { boxes: [[40, 16, -3, -2, -2, 4, 12, 4, f]], pivot: at },
    leftArm: { boxes: [[40, 16, -1, -2, -2, 4, 12, 4, f, true]], pivot: at },
    rightLeg: { boxes: [[0, 16, -2, 0, -2, 4, 12, 4, f]], pivot: at },
    leftLeg: { boxes: [[0, 16, -2, 0, -2, 4, 12, 4, f, true]], pivot: at },
  };
}

/** Models that wear armour (BipedEntityRenderer / PlayerEntityRenderer / ArmorStandEntityRenderer / GiantEntityRenderer add ArmorRenderer). */
export const WEARS_ARMOUR = new Set(['player', 'Zombie', 'PigZombie', 'Skeleton', 'Giant', 'ArmorStand']);

/** Every armour texture (the server packs them: images.json). */
export function armourTextures(): string[] {
  const out: string[] = [];
  for (const m of ARMOUR_MATERIALS) for (const slot of [1, 2]) {
    out.push(armourTexture(m, slot));
    if (m === 'leather') out.push(armourTexture(m, slot, true));
  }
  return out;
}

/** 0xRRGGBB → tint. */
export const rgb = (c: number): [number, number, number] => [((c >> 16) & 255) / 255, ((c >> 8) & 255) / 255, (c & 255) / 255];

/**
 * A block on the head: the snow golem's pumpkin (SnowGolemPumpkinFeatureRenderer), or a block item in the helmet slot
 * (HeadFeatureRenderer: a pumpkin on a player). Block id and meta (pumpkins face north, the model's front), or null.
 */
export function headBlock(name: string, armour?: ReadonlyArray<readonly [number, number, number, number]>): [number, number] | null {
  if (name === 'SnowMan') return [86, 2];
  const helmet = armour?.find(a => a[0] === 4);
  if (helmet && helmet[1] > 0 && helmet[1] < 256) return [helmet[1], helmet[1] === 86 || helmet[1] === 91 ? 2 : helmet[2]];
  return null;
}

/**
 * Where that block sits, relative to the head's pivot in blocks (y up, the head's own axes): SnowGolemPumpkinFeatureRenderer
 * moves it 0.34375 up, HeadFeatureRenderer 0.25, both at 0.625 size, centred.
 */
export function headBlockPlace(name: string): { up: number; scale: number } {
  return { up: name === 'SnowMan' ? 0.34375 : 0.25, scale: 0.625 };
}
