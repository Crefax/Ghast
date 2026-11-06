/**
 * The shape of the block/item data file the server builds (data.<hash>.json, see docs/protocol.md):
 * resolved block models, texture layers, item icons and biome colours. Types only; the server side
 * (Rust) writes this format, the page and its mesher worker read it.
 */

export type Dir = 'down' | 'up' | 'north' | 'south' | 'west' | 'east';

/** One model face: `t` is the texture layer, `cull` the face that hides it, `r` the UV rotation, `tint` the tint index. */
export interface Face { uv?: number[]; t: number; cull?: Dir; r?: number; tint?: number }
export interface Element { from: number[]; to: number[]; rot?: { o: number[]; axis: string; angle: number; rescale?: boolean }; shade?: boolean; faces: Partial<Record<Dir, Face>> }
/** `p`: the layer of the model's particle texture (block break and dig particles). */
export interface Model { ao: boolean; el: Element[]; p?: number }
/** A blockstate variant: model index, rotations, uvlock and the random weight. */
export interface VariantRef { m: number; x?: number; y?: number; uvlock?: boolean; w?: number }
/** `collides`: a full collision box (stops rain and snow, with fluids). */
export interface BlockData { name: string; variants: Record<string, VariantRef[]>; fluid?: 'water' | 'lava'; solidHint: boolean; translucent?: boolean; collides?: true }
/** A texture layer: animation frames, the frame time in ticks, an optional crop [u, v, w, h] of the source image `src`. */
export interface LayerInfo { name: string; frames: number; frametime: number; crop?: number[]; src?: string }
export interface ModelData {
  layers: LayerInfo[];
  models: Model[];
  /** By block id (0..4095); null: unknown id. */
  blocks: Array<BlockData | null>;
  items: Record<number, { name: string; layer: number }>;
  /** By biome id: [biome id, temperature, rainfall] (the list from minecraft-data). */
  biomes: Array<[number, number, number]>;
  /** Images (paths below textures/, no extension) in the pack besides the block layers. */
  images: string[];
}
