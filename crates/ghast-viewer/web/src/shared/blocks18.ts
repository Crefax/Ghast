/**
 * Minecraft 1.8 block states for the viewer: (id, metadata) + neighbours → the variant key of the
 * 1.8 blockstates JSON (vanilla stateFromData + getBlockState). Used by the page's mesher worker; the
 * server's data builder must emit the same keys (docs/protocol.md). No Node or DOM imports.
 *
 * Keys are "prop=value,…" with the property names sorted (the JSON's order). Blocks with a variant in
 * the metadata get a "v=<n>," prefix instead of their variant property; VARIANTS says how the data
 * builder makes those entries.
 */

export const COLORS = ['white', 'orange', 'magenta', 'light_blue', 'yellow', 'lime', 'pink', 'gray', 'silver', 'cyan', 'purple', 'blue', 'brown', 'green', 'red', 'black'];
export const WOODS = ['oak', 'spruce', 'birch', 'jungle', 'acacia', 'dark_oak'];

/** Direction.fromHorizontal(i) and Direction.getById(i). */
export const HORIZONTAL = ['south', 'west', 'north', 'east'];
export const FRONT = ['down', 'up', 'north', 'south', 'west', 'east'];

/**
 * Blocks with a variant in the metadata: per variant either the model to use (the entry replaces the
 * whole block: "v=<n>,normal"), or a function renaming the models of the JSON's own variants (the
 * entries become "v=<n>,<original key>").
 */
export type VariantRule = string | ((model: string) => string);
export const VARIANTS: Record<string, { mask: number; rules: VariantRule[] }> = {
  stone: { mask: 7, rules: ['stone', 'granite', 'granite_smooth', 'diorite', 'diorite_smooth', 'andesite', 'andesite_smooth'] },
  dirt: { mask: 3, rules: ['dirt', 'coarse_dirt', 'podzol'] },
  planks: { mask: 7, rules: WOODS.map(w => `${w}_planks`) },
  sapling: { mask: 7, rules: WOODS.map(w => (m: string) => m.replace(/^oak_/, `${w}_`)) },
  sand: { mask: 1, rules: ['sand', 'red_sand'] },
  log: { mask: 3, rules: WOODS.slice(0, 4).map(w => (m: string) => m.replace(/^oak_/, `${w}_`)) },
  log2: { mask: 1, rules: ['acacia', 'dark_oak'].map(w => (m: string) => m.replace(/^acacia_/, `${w}_`)) },
  leaves: { mask: 3, rules: WOODS.slice(0, 4).map(w => `${w}_leaves`) },
  leaves2: { mask: 1, rules: ['acacia_leaves', 'dark_oak_leaves'] },
  sandstone: { mask: 3, rules: ['sandstone_normal', 'sandstone_chiseled', 'sandstone_smooth'] },
  red_sandstone: { mask: 3, rules: ['red_sandstone_normal', 'red_sandstone_chiseled', 'red_sandstone_smooth'] },
  tallgrass: { mask: 3, rules: ['dead_bush', 'tall_grass', 'fern'] },
  wool: { mask: 15, rules: COLORS.map(c => `${c}_wool`) },
  red_flower: { mask: 15, rules: ['poppy', 'orchid', 'allium', 'houstonia', 'red_tulip', 'orange_tulip', 'white_tulip', 'pink_tulip', 'daisy'] },
  double_stone_slab: { mask: 7, rules: ['double_stone', 'sandstone_normal', 'oak_planks', 'cobblestone', 'brick', 'stonebrick_normal', 'nether_brick', 'quartz_normal'] },
  stone_slab: { mask: 7, rules: ['stone', 'sandstone', 'oak', 'cobblestone', 'brick', 'stone_brick', 'nether_brick', 'quartz'].map(v => (m: string) => m.replace(/_stone$/, `_${v}`)) },
  double_wooden_slab: { mask: 7, rules: WOODS.map(w => `${w}_planks`) },
  wooden_slab: { mask: 7, rules: WOODS.map(w => (m: string) => m.replace(/_oak$/, `_${w}`)) },
  stained_glass: { mask: 15, rules: COLORS.map(c => `glass_${c}`) },
  stained_glass_pane: { mask: 15, rules: COLORS.map(c => (m: string) => m.replace(/^white_/, `${c}_`)) },
  stained_hardened_clay: { mask: 15, rules: COLORS.map(c => `hardened_clay_${c}`) },
  carpet: { mask: 15, rules: COLORS.map(c => `carpet_${c}`) },
  monster_egg: { mask: 7, rules: ['stone', 'cobblestone', 'stonebrick_normal', 'stonebrick_mossy', 'stonebrick_cracked', 'stonebrick_chiseled'] },
  stonebrick: { mask: 3, rules: ['stonebrick_normal', 'stonebrick_mossy', 'stonebrick_cracked', 'stonebrick_chiseled'] },
  prismarine: { mask: 3, rules: ['prismarine_rough', 'prismarine_bricks', 'prismarine_dark'] },
  cobblestone_wall: { mask: 1, rules: [(m: string) => m, (m: string) => m.replace(/^cobblestone_wall_/, 'mossy_wall_')] },
  double_plant: { mask: 7, rules: ['sunflower', 'syringa', 'grass', 'fern', 'rose', 'paeonia'].map(v => (m: string) => m.replace('sunflower', v)) },
};

/** Neighbour lookup: the state id (id << 4 | meta) at an offset, 0 for air or not loaded. */
export type Neighbour = (dx: number, dy: number, dz: number) => number;

/** Block names by id (from the page's model data). */
export interface BlockNames { name(id: number): string | undefined }

const key = (p: Record<string, string | number | boolean>): string =>
  Object.keys(p).sort().map(k => `${k}=${p[k]}`).join(',');

const OFFSETS: Record<string, [number, number, number]> = {
  north: [0, 0, -1], south: [0, 0, 1], west: [-1, 0, 0], east: [1, 0, 0], up: [0, 1, 0], down: [0, -1, 0],
};
const OPPOSITE: Record<string, string> = { north: 'south', south: 'north', west: 'east', east: 'west', up: 'down', down: 'up' };
const CW: Record<string, string> = { north: 'east', east: 'south', south: 'west', west: 'north' };
const CCW: Record<string, string> = { north: 'west', west: 'south', south: 'east', east: 'north' };

/**
 * Per-id facts the decoder needs about neighbours (a full opaque cube; fence family...). Filled by the
 * page from its model data; the defaults make every non-air block solid.
 */
export interface BlockFacts {
  /** An opaque full cube: fences, panes and walls connect to it, vines hang on it. */
  solid(id: number): boolean;
}

export interface Decoded { name: string; key: string }

const STAIRS = new Set(['oak_stairs', 'stone_stairs', 'brick_stairs', 'stone_brick_stairs', 'nether_brick_stairs', 'sandstone_stairs',
  'spruce_stairs', 'birch_stairs', 'jungle_stairs', 'quartz_stairs', 'acacia_stairs', 'dark_oak_stairs', 'red_sandstone_stairs']);
const FENCES = new Set(['fence', 'spruce_fence', 'birch_fence', 'jungle_fence', 'dark_oak_fence', 'acacia_fence']);
const GATES = new Set(['fence_gate', 'spruce_fence_gate', 'birch_fence_gate', 'jungle_fence_gate', 'dark_oak_fence_gate', 'acacia_fence_gate']);
const DOORS = new Set(['wooden_door', 'iron_door', 'spruce_door', 'birch_door', 'jungle_door', 'acacia_door', 'dark_oak_door']);
const REDSTONE_SOURCES = new Set(['redstone_wire', 'redstone_torch', 'unlit_redstone_torch', 'lever', 'stone_button', 'wooden_button', 'redstone_block',
  'stone_pressure_plate', 'wooden_pressure_plate', 'light_weighted_pressure_plate', 'heavy_weighted_pressure_plate', 'daylight_detector',
  'daylight_detector_inverted', 'detector_rail', 'tripwire_hook', 'unpowered_comparator', 'powered_comparator', 'unpowered_repeater', 'powered_repeater']);

/** Blocks whose key depends on neighbours (the mesher cannot cache them per state id). */
export const NEEDS_NEIGHBOURS = new Set<string>([...STAIRS, ...FENCES, ...GATES, ...DOORS, 'nether_brick_fence', 'glass_pane', 'stained_glass_pane',
  'iron_bars', 'cobblestone_wall', 'grass', 'mycelium', 'vine', 'tripwire', 'tripwire_hook', 'redstone_wire', 'pumpkin_stem', 'melon_stem',
  'double_plant', 'fire', 'unpowered_repeater', 'powered_repeater', 'chest', 'trapped_chest']);

/**
 * The blockstates key of a block. `names` maps neighbour ids to names; `facts` answers "solid".
 */
export function decode(name: string, meta: number, nb: Neighbour, names: BlockNames, facts: BlockFacts): string {
  const nName = (dx: number, dy: number, dz: number): string => names.name(nb(dx, dy, dz) >> 4) ?? 'air';
  const nMeta = (dx: number, dy: number, dz: number): number => nb(dx, dy, dz) & 15;
  const solidAt = (dx: number, dy: number, dz: number): boolean => { const s = nb(dx, dy, dz); return s !== 0 && facts.solid(s >> 4); };
  const at = (dir: string): [number, number, number] => OFFSETS[dir];
  const v = VARIANTS[name];

  if (v) {
    // the upper half of a double plant has its variant in the lower half
    const i = (name === 'double_plant' && meta & 8 ? nMeta(0, -1, 0) : meta) & v.mask;
    const rule = v.rules[i] ?? v.rules[0];
    const base = baseKey(name, meta, nb, nName, nMeta, solidAt, at);
    return `v=${v.rules[i] !== undefined ? i : 0},${typeof rule === 'string' ? 'normal' : base}`;
  }
  return baseKey(name, meta, nb, nName, nMeta, solidAt, at);
}

function baseKey(
  name: string, meta: number, nb: Neighbour,
  nName: (dx: number, dy: number, dz: number) => string, nMeta: (dx: number, dy: number, dz: number) => number,
  solidAt: (dx: number, dy: number, dz: number) => boolean, at: (dir: string) => [number, number, number],
): string {
  if (STAIRS.has(name)) return stairs(name, meta, nName, nMeta, at);
  if (FENCES.has(name) || name === 'nether_brick_fence') {
    const c = (dir: string) => {
      const n = nName(...at(dir));
      return (name === 'nether_brick_fence' ? n === name : FENCES.has(n)) || GATES.has(n) || solidAt(...at(dir));
    };
    return key({ east: c('east'), north: c('north'), south: c('south'), west: c('west') });
  }
  if (GATES.has(name)) {
    const facing = HORIZONTAL[meta & 3];
    const side = facing === 'north' || facing === 'south' ? ['east', 'west'] : ['north', 'south'];
    const inWall = side.some(d => nName(...at(d)) === 'cobblestone_wall');
    return key({ facing, in_wall: inWall, open: (meta & 4) !== 0 });
  }
  if (DOORS.has(name)) {
    const upper = (meta & 8) !== 0;
    const lowerMeta = upper ? nMeta(0, -1, 0) : meta;
    const upperMeta = upper ? meta : nMeta(0, 1, 0);
    return key({
      facing: CCW[HORIZONTAL[lowerMeta & 3]], half: upper ? 'upper' : 'lower',
      hinge: (upperMeta & 1) ? 'right' : 'left', open: (lowerMeta & 4) !== 0,
    });
  }
  switch (name) {
    case 'glass_pane': case 'stained_glass_pane': case 'iron_bars': {
      const c = (dir: string) => {
        const n = nName(...at(dir));
        return n === name || n === 'glass' || n === 'stained_glass' || n === 'glass_pane' || n === 'stained_glass_pane' || solidAt(...at(dir));
      };
      return key({ east: c('east'), north: c('north'), south: c('south'), west: c('west') });
    }
    case 'cobblestone_wall': {
      const c = (dir: string) => { const n = nName(...at(dir)); return n === 'cobblestone_wall' || GATES.has(n) || solidAt(...at(dir)); };
      const n = c('north'), s = c('south'), e = c('east'), w = c('west');
      const straight = (n && s && !e && !w) || (!n && !s && e && w);
      return key({ east: e, north: n, south: s, up: !straight || nName(0, 1, 0) !== 'air', west: w });
    }
    case 'grass': case 'mycelium': {
      const above = nName(0, 1, 0);
      return key({ snowy: above === 'snow' || above === 'snow_layer' });
    }
    case 'dirt': return 'normal';
    case 'sponge': return key({ wet: (meta & 1) !== 0 });
    case 'log': case 'log2': return key({ axis: ['y', 'x', 'z', 'none'][(meta >> 2) & 3] });
    case 'hay_block': return key({ axis: ['y', 'x', 'z', 'y'][(meta >> 2) & 3] });
    case 'quartz_block': return 'normal';
    case 'sapling': return key({ stage: (meta >> 3) & 1 });
    case 'stone_slab': case 'wooden_slab': case 'stone_slab2': return key({ half: meta & 8 ? 'top' : 'bottom' });
    case 'double_stone_slab': case 'double_stone_slab2': return meta & 8 ? 'all' : 'normal';
    case 'double_plant': {
      const upper = (meta & 8) !== 0;
      return key({ half: upper ? 'upper' : 'lower' });
    }
    case 'wheat': case 'carrots': case 'potatoes': return key({ age: meta & 7 });
    case 'nether_wart': return key({ age: meta & 3 });
    case 'farmland': return key({ moisture: meta & 7 });
    case 'cactus': case 'reeds': return 'normal';
    case 'pumpkin': case 'lit_pumpkin': return key({ facing: HORIZONTAL[meta & 3] });
    case 'pumpkin_stem': case 'melon_stem': {
      const age = meta & 7;
      let facing = 'up';
      if (age === 7) {
        const fruit = name === 'pumpkin_stem' ? 'pumpkin' : 'melon_block';
        for (const d of ['west', 'east', 'north', 'south']) if (nName(...at(d)) === fruit) { facing = d; break; }
      }
      return key({ age, facing });
    }
    case 'torch': case 'redstone_torch': case 'unlit_redstone_torch':
      return key({ facing: ['up', 'east', 'west', 'south', 'north', 'up'][meta & 7] ?? 'up' });
    case 'standing_sign': return key({ rotation: meta & 15 });
    case 'ladder': case 'furnace': case 'lit_furnace': case 'wall_sign': case 'ender_chest': {
      const f = FRONT[meta & 7];
      return key({ facing: f === 'down' || f === 'up' ? 'north' : f });
    }
    case 'chest': case 'trapped_chest': {
      // a double chest: the same chest on the left (seen from the front) or right; each half draws its side
      const f = FRONT[meta & 7];
      const facing = !f || f === 'down' || f === 'up' ? 'north' : f;
      const type = nName(...at(CW[facing])) === name ? 'left' : nName(...at(CCW[facing])) === name ? 'right' : '';
      return key(type ? { facing, type } : { facing });
    }
    case 'dispenser': case 'dropper': return key({ facing: FRONT[meta & 7] ?? 'north' });
    case 'hopper': { const f = FRONT[meta & 7]; return key({ facing: f === 'up' ? 'down' : f ?? 'down' }); }
    case 'piston': case 'sticky_piston': return key({ extended: (meta & 8) !== 0, facing: FRONT[meta & 7] ?? 'up' });
    case 'piston_head': return key({ facing: FRONT[meta & 7] ?? 'up', short: false, type: meta & 8 ? 'sticky' : 'normal' });
    case 'bed': return key({ facing: HORIZONTAL[meta & 3], part: meta & 8 ? 'head' : 'foot' });
    case 'rail': return key({ shape: ['north_south', 'east_west', 'ascending_east', 'ascending_west', 'ascending_north', 'ascending_south', 'south_east', 'south_west', 'north_west', 'north_east'][meta] ?? 'north_south' });
    case 'golden_rail': case 'detector_rail': case 'activator_rail':
      return key({ powered: (meta & 8) !== 0, shape: ['north_south', 'east_west', 'ascending_east', 'ascending_west', 'ascending_north', 'ascending_south'][meta & 7] ?? 'north_south' });
    case 'lever': return key({ facing: ['down_x', 'east', 'west', 'south', 'north', 'up_z', 'up_x', 'down_z'][meta & 7], powered: (meta & 8) !== 0 });
    case 'stone_button': case 'wooden_button': return key({ facing: ['down', 'east', 'west', 'south', 'north', 'up'][meta & 7] ?? 'up', powered: (meta & 8) !== 0 });
    case 'stone_pressure_plate': case 'wooden_pressure_plate': return key({ powered: meta === 1 });
    case 'light_weighted_pressure_plate': case 'heavy_weighted_pressure_plate': case 'daylight_detector': case 'daylight_detector_inverted':
      return key({ power: meta });
    case 'snow_layer': return key({ layers: (meta & 7) + 1 });
    case 'cake': return key({ bites: Math.min(6, meta) });
    case 'unpowered_repeater': case 'powered_repeater': {
      return key({ delay: (meta >> 2) + 1, facing: HORIZONTAL[meta & 3], locked: false });
    }
    case 'unpowered_comparator': case 'powered_comparator':
      return key({ facing: HORIZONTAL[meta & 3], mode: meta & 4 ? 'subtract' : 'compare', powered: (meta & 8) !== 0 });
    case 'portal': return key({ axis: (meta & 3) === 2 ? 'z' : 'x' });
    case 'vine': {
      return key({ east: (meta & 8) !== 0, north: (meta & 4) !== 0, south: (meta & 1) !== 0, up: solidAt(0, 1, 0), west: (meta & 2) !== 0 });
    }
    case 'cocoa': return key({ age: Math.min(2, meta >> 2), facing: HORIZONTAL[meta & 3] });
    case 'trapdoor': case 'iron_trapdoor':
      return key({ facing: ['north', 'south', 'west', 'east'][meta & 3], half: meta & 8 ? 'top' : 'bottom', open: (meta & 4) !== 0 });
    case 'tripwire_hook':
      return key({ attached: (meta & 4) !== 0, facing: HORIZONTAL[meta & 3], powered: (meta & 8) !== 0, suspended: !solidAt(0, -1, 0) });
    case 'tripwire': {
      const c = (dir: string) => {
        const [dx, dy, dz] = at(dir);
        const n = nName(dx, dy, dz);
        if (n === 'tripwire') return true;
        // a hook connects when it faces back at the wire
        return n === 'tripwire_hook' && HORIZONTAL[nMeta(dx, dy, dz) & 3] === OPPOSITE[dir];
      };
      return key({ attached: (meta & 4) !== 0, east: c('east'), north: c('north'), south: c('south'), suspended: (meta & 2) !== 0, west: c('west') });
    }
    case 'redstone_wire': {
      const side = (dir: string): string => {
        const [dx, , dz] = at(dir);
        const n = nName(dx, 0, dz);
        if (REDSTONE_SOURCES.has(n)) return 'side';
        if (!solidAt(0, 1, 0) && solidAt(dx, 0, dz) && nName(dx, 1, dz) === 'redstone_wire') return 'up';
        if (!solidAt(dx, 0, dz) && nName(dx, -1, dz) === 'redstone_wire') return 'side';
        return 'none';
      };
      return key({ east: side('east'), north: side('north'), south: side('south'), west: side('west') });
    }
    case 'fire': return key({ alt: false, east: false, flip: false, north: false, south: false, upper: 0, west: false });
    case 'brewing_stand': return key({ has_bottle_0: (meta & 1) !== 0, has_bottle_1: (meta & 2) !== 0, has_bottle_2: (meta & 4) !== 0 });
    case 'cauldron': return key({ level: Math.min(3, meta) });
    case 'end_portal_frame': return key({ eye: (meta & 4) !== 0, facing: HORIZONTAL[meta & 3] });
    case 'anvil': return key({ damage: Math.min(2, meta >> 2), facing: HORIZONTAL[meta & 3] });
    case 'brown_mushroom_block': case 'red_mushroom_block': {
      const v = ['all_inside', 'north_west', 'north', 'north_east', 'west', 'center', 'east', 'south_west', 'south', 'south_east', 'stem', 'all_inside', 'all_inside', 'all_inside', 'all_outside', 'all_stem'][meta];
      return key({ variant: v });
    }
    case 'flower_pot': return key({ contents: 'empty' }); // the contents come from the tile entity (potContents)
    case 'flowing_water': case 'water': case 'flowing_lava': case 'lava': return '';
    default: return 'normal';
  }
}

/** Vanilla 1.8 StairsBlock.getBlockState: straight, or inner/outer corners from the stairs behind/in front. */
function stairs(name: string, meta: number, nName: (dx: number, dy: number, dz: number) => string,
  nMeta: (dx: number, dy: number, dz: number) => number, at: (dir: string) => [number, number, number]): string {
  const facing = ['east', 'west', 'south', 'north'][meta & 3];
  const half = meta & 4 ? 'top' : 'bottom';
  const isStairs = (d: [number, number, number]) => STAIRS.has(nName(...d));
  const facingOf = (d: [number, number, number]) => ['east', 'west', 'south', 'north'][nMeta(...d) & 3];
  const halfOf = (d: [number, number, number]) => (nMeta(...d) & 4 ? 'top' : 'bottom');
  let shape = 'straight';
  // behind (the side the high step faces): an outer corner
  const back = at(facing);
  if (isStairs(back) && halfOf(back) === half) {
    const f = facingOf(back);
    if (f === CCW[facing] && !sameStairs(at(OPPOSITE[f]), facing, half)) shape = 'outer_left';
    else if (f === CW[facing] && !sameStairs(at(OPPOSITE[f]), facing, half)) shape = 'outer_right';
  }
  if (shape === 'straight') {
    const front = at(OPPOSITE[facing]);
    if (isStairs(front) && halfOf(front) === half) {
      const f = facingOf(front);
      if (f === CCW[facing] && !sameStairs(at(f), facing, half)) shape = 'inner_left';
      else if (f === CW[facing] && !sameStairs(at(f), facing, half)) shape = 'inner_right';
    }
  }
  return key({ facing, half, shape });

  function sameStairs(d: [number, number, number], f: string, h: string): boolean {
    return isStairs(d) && facingOf(d) === f && halfOf(d) === h;
  }
}

/** Water and lava height of a meta (AbstractFluidBlock.getHeightPercent): 0..1 of the block, 8+ = falling = full. */
export function fluidHeight(meta: number): number {
  if (meta >= 8) return 1;
  return 1 - (meta + 1) / 9;
}

/**
 * The flower pot's blockstates "contents" from its tile entity (FlowerPotBlock.getBlockState): the
 * item's block name (with or without "minecraft:") and its data value. Unknown plants: empty.
 */
export function potContents(item: string, data: number): string {
  const name = item.replace(/^minecraft:/, '');
  switch (name) {
    case 'red_flower': return ['rose', 'blue_orchid', 'allium', 'houstonia', 'red_tulip', 'orange_tulip', 'white_tulip', 'pink_tulip', 'oxeye_daisy'][data] ?? 'empty';
    case 'yellow_flower': return 'dandelion';
    case 'sapling': return WOODS[data & 7] ? `${WOODS[data & 7]}_sapling` : 'empty';
    case 'tallgrass': return data === 2 ? 'fern' : 'empty';
    case 'deadbush': return 'dead_bush';
    case 'cactus': return 'cactus';
    case 'red_mushroom': return 'mushroom_red';
    case 'brown_mushroom': return 'mushroom_brown';
    default: return 'empty';
  }
}
