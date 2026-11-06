/**
 * The world around the camera as the particles need it (pure, shared by the mesher worker and the page):
 * a box of light (sky << 4 | block) and collision shapes, and the blocks with a randomDisplayTick
 * (torches, fire, lava, drips...). The worker fills it from its columns; the page moves particles in it.
 */
import type { BlockData } from './shared/model-data.js';

/** Box size in blocks (x, y, z) around the camera: particles further out fly unlit by blocks and without collisions. */
export const BOX_X = 40, BOX_Y = 24, BOX_Z = 40;

/** Shapes: 0 open, 1..16 a box from the floor up to n/16, WATER, LAVA. */
export const WATER = 17, LAVA = 18;

const SLABS = new Set([44, 126, 182]);

/** The collision shape a particle meets in a block (a full-width box of some height, or a liquid). */
export function shapeOf(state: number, blocks: ReadonlyArray<BlockData | null>): number {
  if (!state) return 0;
  const id = state >> 4, b = blocks[id];
  if (!b) return 0;
  if (b.fluid) return b.fluid === 'water' ? WATER : LAVA;
  if (SLABS.has(id)) return state & 8 ? 16 : 8;
  switch (id) {
    case 78: return ((state & 7) + 1) * 2; // snow layers
    case 171: return 1; // carpet
    case 60: return 15; // farmland
    case 81: return 15; // cactus
    case 54: case 130: case 146: return 14; // chests
    case 96: case 167: return state & 4 ? 0 : state & 8 ? 16 : 3; // trapdoors: open ones stand at a side
  }
  return b.collides ? 16 : 0;
}

/** Block ids with a randomDisplayTick the page reproduces. */
export const EMITTERS = new Set([50, 76, 51, 10, 11, 8, 9, 62, 90, 74, 55, 52, 130]);

/**
 * Whether a block near the camera emits particles at all (the worker lists only these):
 * liquids only with air above (lava pops) or dripping through a ceiling, redstone wire only powered.
 * `at(dy)` is the state dy blocks above (negative: below).
 */
export function emits(state: number, at: (dy: number) => number, blocks: ReadonlyArray<BlockData | null>): boolean {
  const id = state >> 4;
  if (!EMITTERS.has(id)) return false;
  if (id === 55) return (state & 15) > 0;
  if (id >= 8 && id <= 11) {
    if (id >= 10 && at(1) === 0) return true;
    // AbstractFluidBlock: a drip under a solid block with room below it
    const below = blocks[at(-1) >> 4], below2 = at(-2);
    return !!below?.collides && !below.fluid && (below2 === 0 || (!blocks[below2 >> 4]?.collides && !blocks[below2 >> 4]?.fluid));
  }
  return true;
}

/** The page's view of the box the worker last sent. */
export class FxBox {
  ox = 0; oy = 0; oz = 0;
  light: Uint8Array | null = null;
  shape: Uint8Array | null = null;

  set(ox: number, oy: number, oz: number, light: Uint8Array, shape: Uint8Array): void {
    this.ox = ox; this.oy = oy; this.oz = oz; this.light = light; this.shape = shape;
  }
  clear(): void { this.light = null; this.shape = null; }

  private index(x: number, y: number, z: number): number {
    const bx = Math.floor(x) - this.ox, by = Math.floor(y) - this.oy, bz = Math.floor(z) - this.oz;
    if (bx < 0 || by < 0 || bz < 0 || bx >= BOX_X || by >= BOX_Y || bz >= BOX_Z) return -1;
    return bx + BOX_X * (bz + BOX_Z * by);
  }

  /** Overrides one block's shape (a block the page knows changed before the worker resends the box). */
  setShape(x: number, y: number, z: number, v: number): void {
    const i = this.shape ? this.index(x, y, z) : -1;
    if (i >= 0) this.shape![i] = v;
  }

  /** Shape of the block at a point (0 outside the box). */
  shapeAt(x: number, y: number, z: number): number {
    const i = this.shape ? this.index(x, y, z) : -1;
    return i < 0 ? 0 : this.shape![i];
  }

  /** True if the point is inside a block's collision box. */
  blocked(x: number, y: number, z: number): boolean {
    const s = this.shapeAt(x, y, z);
    return s > 0 && s <= 16 && y - Math.floor(y) < s / 16;
  }

  /**
   * Light at a point as sky << 4 | block (open sky outside the box). A point inside a full block (or a block
   * just broken, before the worker relit it) takes its brightest neighbour, like useNeighbourLight.
   */
  lightAt(x: number, y: number, z: number, sky = 15): number {
    const i = this.light ? this.index(x, y, z) : -1;
    if (i < 0) return sky << 4;
    // (a dark cell takes its neighbours too: a block broken a moment ago still has the light 0 of a solid one)
    if (this.shape![i] !== 16 && this.light![i]) return this.light![i];
    let s = 0, b = 0;
    for (const [dx, dy, dz] of NEIGHBOURS) {
      const j = this.index(x + dx, y + dy, z + dz);
      const v = j < 0 ? sky << 4 : this.light![j];
      s = Math.max(s, v >> 4); b = Math.max(b, v & 15);
    }
    return (s << 4) | b;
  }
}
const NEIGHBOURS = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];
