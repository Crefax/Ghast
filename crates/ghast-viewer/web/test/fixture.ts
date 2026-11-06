/**
 * A tiny hand-made ModelData for tests that must run without the real 1.8 assets: a few full cubes, glass,
 * a crop and water. Variant keys come from decode() itself, so every metadata of a block has its model.
 */
import { decode, type BlockFacts, type BlockNames } from '../src/shared/blocks18.js';
import type { BlockData, Dir, Element, Face, Model, ModelData } from '../src/shared/model-data.js';
import { Mesher } from '../src/mesher.js';

const DIRS: Dir[] = ['down', 'up', 'north', 'south', 'west', 'east'];

export const LAYERS = ['blocks/stone', 'blocks/planks_oak', 'blocks/glass', 'blocks/wheat_stage_7', 'blocks/water_still', 'blocks/water_flow', 'blocks/dirt', 'blocks/farmland_wet'];
const layer = (name: string): number => LAYERS.indexOf(`blocks/${name}`);

function cube(tex: string): Model {
  const faces: Partial<Record<Dir, Face>> = {};
  for (const d of DIRS) faces[d] = { t: layer(tex), uv: [0, 0, 16, 16], cull: d };
  return { ao: true, el: [{ from: [0, 0, 0], to: [16, 16, 16], faces }], p: layer(tex) };
}

/** The crop model: four planes, two along x and two along z, seen from both sides. */
function crop(tex: string): Model {
  const t = layer(tex);
  const f = (): Face => ({ t, uv: [0, 0, 16, 16] });
  const el: Element[] = [
    { from: [4, -1, 0], to: [4, 15, 16], shade: false, faces: { west: f(), east: f() } },
    { from: [12, -1, 0], to: [12, 15, 16], shade: false, faces: { west: f(), east: f() } },
    { from: [0, -1, 4], to: [16, 15, 4], shade: false, faces: { north: f(), south: f() } },
    { from: [0, -1, 12], to: [16, 15, 12], shade: false, faces: { north: f(), south: f() } },
  ];
  return { ao: false, el, p: t };
}

/** Farmland: a 15 px high block with its own top. */
function farmland(): Model {
  const faces: Partial<Record<Dir, Face>> = {};
  for (const d of DIRS) faces[d] = { t: layer(d === 'up' ? 'farmland_wet' : 'dirt'), uv: [0, d === 'up' || d === 'down' ? 0 : 1, 16, 16], ...(d === 'up' ? {} : { cull: d }) };
  return { ao: true, el: [{ from: [0, 0, 0], to: [16, 15, 16], faces }] };
}

export const NAMES: Record<number, string> = {
  1: 'stone', 3: 'dirt', 5: 'planks', 9: 'water', 20: 'glass', 59: 'wheat', 60: 'farmland',
  54: 'chest', 85: 'fence', 86: 'pumpkin', 64: 'wooden_door', 78: 'snow_layer', 130: 'ender_chest', 131: 'tripwire_hook', 132: 'tripwire',
  146: 'trapped_chest', 175: 'double_plant',
};
export const names: BlockNames = { name: (id) => NAMES[id] };
export const facts: BlockFacts = { solid: (id) => id === 1 || id === 3 || id === 5 };

export function fixtureData(): ModelData {
  const models: Model[] = [cube('stone'), cube('planks_oak'), cube('glass'), crop('wheat_stage_7'), cube('dirt'), farmland()];
  const blocks: Array<BlockData | null> = new Array(4096).fill(null);
  const def = (id: number, model: number, extra: Partial<BlockData> = {}) => {
    const variants: BlockData['variants'] = {};
    for (let meta = 0; meta < 16; meta++) variants[decode(NAMES[id], meta, () => 0, names, facts)] = [{ m: model }];
    blocks[id] = { name: NAMES[id], variants, solidHint: model !== 3, ...extra };
  };
  def(1, 0, { collides: true }); def(3, 4, { collides: true }); def(5, 1, { collides: true }); def(20, 2); def(59, 3); def(60, 5, { solidHint: false });
  blocks[9] = { name: 'water', variants: {}, fluid: 'water', solidHint: false, translucent: true };
  return {
    layers: LAYERS.map(name => ({ name, frames: 1, frametime: 1 })),
    models, blocks, items: {}, biomes: [[1, 0.8, 0.4]], images: [],
  };
}

/** A mesher over the fixture: glass and wheat are see-through (cutout), grass colours green. */
export function fixtureMesher(d = fixtureData()): Mesher {
  const alpha = new Uint8Array(d.layers.length);
  d.layers.forEach((l, i) => { if (/wheat|glass/.test(l.name)) alpha[i] = 1; });
  const base = new Uint16Array(d.layers.length).map((_, i) => i);
  const col = new Uint32Array(256).fill(0x79c05a);
  return new Mesher(d, alpha, { grass: col, foliage: col, water: new Uint32Array(256).fill(0xffffff) }, base);
}
