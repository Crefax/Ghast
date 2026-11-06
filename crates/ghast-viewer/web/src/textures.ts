/// <reference lib="dom" />
/**
 * The texture pack (pack.<hash>.bin, built by the server) → one WebGL2 16×16 texture array (animated textures take one
 * slice per frame), the alpha class of every layer (opaque / cutout), the 2D images (skins, HUD, sky),
 * biome colours from the colour maps, and item icons for the HUD.
 */
import type { ModelData } from './shared/model-data.js';

export interface Pack {
  array: WebGLTexture;
  /** First GPU slice of each data layer. */
  base: Uint16Array;
  /** 0 opaque, 1 has see-through pixels. */
  alpha: Uint8Array;
  images: Map<string, ImageBitmap>;
  /** Frame 0 of each data layer, for HUD icons. */
  icons: Array<ImageBitmap | null>;
  slices: number;
}

export async function loadPack(gl: WebGL2RenderingContext, data: ModelData, url: string, onProgress?: (f: number) => void): Promise<Pack> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`doku paketi ${res.status}`);
  const buf = new Uint8Array(await res.arrayBuffer());
  const hlen = new DataView(buf.buffer).getUint32(0, true);
  const header = JSON.parse(new TextDecoder().decode(buf.subarray(4, 4 + hlen))) as { layers: Array<[number, number]>; images: Record<string, [number, number]> };
  const body = 4 + hlen;
  const blobs = new Map<number, Promise<ImageBitmap>>();
  const bitmap = (off: number, len: number): Promise<ImageBitmap> => {
    let p = blobs.get(off);
    if (!p) {
      p = createImageBitmap(new Blob([buf.subarray(body + off, body + off + len)], { type: 'image/png' }), { premultiplyAlpha: 'none', colorSpaceConversion: 'none' } as ImageBitmapOptions);
      blobs.set(off, p);
    }
    return p;
  };

  const maxSlices = gl.getParameter(gl.MAX_ARRAY_TEXTURE_LAYERS) as number;
  const base = new Uint16Array(data.layers.length);
  let slices = 0;
  const frames = data.layers.map(l => l.frames);
  // 32 px flow textures keep their resolution as 2×2 slices per frame (a block shows 16 px of them, like vanilla)
  const quads = data.layers.map(l => (isFlow(l.name) ? 4 : 1));
  const needed = frames.reduce((a, b, i) => a + b * quads[i], 0);
  const animate = needed <= maxSlices;
  for (let i = 0; i < data.layers.length; i++) {
    base[i] = slices;
    if (!animate) frames[i] = 1;
    slices += frames[i] * quads[i];
  }

  const tex = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D_ARRAY, tex);
  gl.texStorage3D(gl.TEXTURE_2D_ARRAY, 5, gl.RGBA8, 16, 16, slices);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);

  const alpha = new Uint8Array(data.layers.length);
  const icons: Array<ImageBitmap | null> = new Array(data.layers.length).fill(null);
  const probe = document.createElement('canvas');
  probe.width = 16; probe.height = 16;
  const pctx = probe.getContext('2d', { willReadFrequently: true })!;
  pctx.imageSmoothingEnabled = false;
  const strip = document.createElement('canvas');
  const sctx = strip.getContext('2d')!;

  let done = 0;
  await Promise.all(data.layers.map(async (l, i) => {
    const [off, len] = header.layers[i];
    const img = await bitmap(off, len);
    const n = frames[i];
    const fw = l.crop ? l.crop[2] : img.width;
    const fh = l.crop ? l.crop[3] : (l.frames > 1 ? img.width : img.height);
    if (quads[i] === 4 && img.width === 32) {
      // frame f, quadrant (qx, qy) → slice base + 4 f + qx + 2 qy, nearest (no scaling)
      strip.width = 16; strip.height = 16 * 4 * n;
      sctx.clearRect(0, 0, 16, strip.height);
      for (let f = 0; f < n; f++) for (let q = 0; q < 4; q++) sctx.drawImage(img, (q & 1) * 16, f * 32 + (q >> 1) * 16, 16, 16, 0, (f * 4 + q) * 16, 16, 16);
      gl.pixelStorei(gl.UNPACK_IMAGE_HEIGHT, 16);
      gl.texSubImage3D(gl.TEXTURE_2D_ARRAY, 0, 0, 0, base[i], 16, 16, 4 * n, gl.RGBA, gl.UNSIGNED_BYTE, strip);
    } else if (!l.crop && img.width === 16 && (n === 1 ? img.height >= 16 : img.height >= 16 * n)) {
      gl.pixelStorei(gl.UNPACK_IMAGE_HEIGHT, 16);
      // the PNG straight in (no canvas: keeps the colour of transparent pixels, no premultiply round trip)
      if (img.height === 16 * n) gl.texSubImage3D(gl.TEXTURE_2D_ARRAY, 0, 0, 0, base[i], 16, 16, n, gl.RGBA, gl.UNSIGNED_BYTE, img);
      else {
        strip.width = 16; strip.height = 16 * n;
        sctx.clearRect(0, 0, 16, 16 * n);
        sctx.drawImage(img, 0, 0, 16, 16 * n, 0, 0, 16, 16 * n);
        gl.texSubImage3D(gl.TEXTURE_2D_ARRAY, 0, 0, 0, base[i], 16, 16, n, gl.RGBA, gl.UNSIGNED_BYTE, strip);
      }
    } else {
      // other sizes (32 px flow textures, 32 px items) and crops of entity textures: scale into 16×16 frames
      strip.width = 16; strip.height = 16 * n;
      sctx.imageSmoothingEnabled = fw > 16;
      sctx.clearRect(0, 0, 16, 16 * n);
      for (let f = 0; f < n; f++) {
        const sx = l.crop ? l.crop[0] : 0, sy = l.crop ? l.crop[1] : f * fh;
        sctx.drawImage(img, sx, sy, fw, fh, 0, f * 16, 16, 16);
      }
      gl.pixelStorei(gl.UNPACK_IMAGE_HEIGHT, 16);
      gl.texSubImage3D(gl.TEXTURE_2D_ARRAY, 0, 0, 0, base[i], 16, 16, n, gl.RGBA, gl.UNSIGNED_BYTE, strip);
    }
    // alpha class and the HUD icon from frame 0
    pctx.clearRect(0, 0, 16, 16);
    if (l.crop) pctx.drawImage(img, l.crop[0], l.crop[1], fw, fh, 0, 0, 16, 16);
    else pctx.drawImage(img, 0, 0, fw, Math.min(fh, img.height), 0, 0, 16, 16);
    const px = pctx.getImageData(0, 0, 16, 16).data;
    let see = 0;
    for (let k = 3; k < px.length; k += 4) if (px[k] < 250) { see = 1; break; }
    alpha[i] = see;
    if (l.name.startsWith('items/') || l.name.startsWith('blocks/')) icons[i] = await createImageBitmap(probe);
    if (++done % 64 === 0) onProgress?.(done / data.layers.length);
  }));
  gl.pixelStorei(gl.UNPACK_IMAGE_HEIGHT, 0);
  gl.generateMipmap(gl.TEXTURE_2D_ARRAY);
  gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MIN_FILTER, gl.NEAREST_MIPMAP_LINEAR);
  gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_S, gl.REPEAT);
  gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_WRAP_T, gl.REPEAT);
  gl.texParameteri(gl.TEXTURE_2D_ARRAY, gl.TEXTURE_MAX_LEVEL, 4);
  // no anisotropic filtering: on D3D (ANGLE on Windows) it turns the magnification linear and blurs the pixel art

  const images = new Map<string, ImageBitmap>();
  await Promise.all(Object.entries(header.images).map(async ([name, [off, len]]) => { images.set(name, await bitmap(off, len)); }));
  return { array: tex, base, alpha, images, icons, slices };
}

/** Flowing water and lava: 32 px textures sampled in 2×2 slices (the mesher flags their quads, see TERRAIN_FS). */
export const isFlow = (name: string): boolean => name === 'blocks/water_flow' || name === 'blocks/lava_flow';

/** A 2D texture from an image (entity skins), nearest filtered. */
export function texture2d(gl: WebGL2RenderingContext, img: TexImageSource): WebGLTexture {
  const t = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, t);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, img);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  return t;
}

/** Grass, foliage and water colours per biome id (GrassColors/FoliageColors + the 1.8 special cases). */
export function biomeColors(data: ModelData, images: Map<string, ImageBitmap>): { grass: Uint32Array; foliage: Uint32Array; water: Uint32Array } {
  const read = (name: string): Uint8ClampedArray | null => {
    const img = images.get(name);
    if (!img) return null;
    const c = document.createElement('canvas');
    c.width = img.width; c.height = img.height;
    const ctx = c.getContext('2d', { willReadFrequently: true })!;
    ctx.drawImage(img, 0, 0);
    return ctx.getImageData(0, 0, img.width, img.height).data;
  };
  const gmap = read('colormap/grass'), fmap = read('colormap/foliage');
  const grass = new Uint32Array(256).fill(0x79c05a), foliage = new Uint32Array(256).fill(0x59ae30), water = new Uint32Array(256).fill(0xffffff);
  const at = (map: Uint8ClampedArray | null, t: number, r: number, fallback: number): number => {
    if (!map) return fallback;
    const temp = Math.max(0, Math.min(1, t)), rain = Math.max(0, Math.min(1, r)) * temp;
    const x = Math.floor((1 - temp) * 255), y = Math.floor((1 - rain) * 255);
    const i = (y * 256 + x) * 4;
    return (map[i] << 16) | (map[i + 1] << 8) | map[i + 2];
  };
  for (const [id, t, r] of data.biomes) {
    if (id < 0 || id > 255) continue;
    grass[id] = at(gmap, t, r, grass[id]);
    foliage[id] = at(fmap, t, r, foliage[id]);
    if (id === 6 || id === 134) { grass[id] = 0x6a7039; foliage[id] = 0x6a7039; water[id] = 0xe0ffae; }
    if (id === 29 || id === 157) grass[id] = (((grass[id] & 0xfefefe) + 0x28340a) >> 1);
    if ((id >= 37 && id <= 39) || (id >= 165 && id <= 167)) { grass[id] = 0x90814d; foliage[id] = 0x9e814d; }
  }
  return { grass, foliage, water };
}
