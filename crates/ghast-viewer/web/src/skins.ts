/// <reference lib="dom" />
/**
 * Player skins on the page: the PNGs the viewer server downloaded (served as
 * skins/<sha>.png next to the page), turned into 64×64 textures the way 1.8's DownloadedSkinParser does
 * it, so one PlayerEntityModel draws every skin: a legacy 64×32 skin gets its left limbs as mirrored copies of
 * the right ones and no outer layer, the base layer is made opaque and an all-opaque hat (old skins
 * painted it black) is cleared. Capes are used as they come (64×32).
 */
import { texture2d } from './textures.js';

export const SKIN_PATH = /^skins\/[0-9a-f]{32}\.png$/;

/** DownloadedSkinParser.parseSkin: RGBA of a 64×64 or 64×32 skin → 64×64 RGBA. */
export function normalizeSkin(src: ArrayLike<number>, w: number, h: number): Uint8ClampedArray<ArrayBuffer> {
  const out = new Uint8ClampedArray(64 * 64 * 4);
  const rows = Math.min(h, 64), cols = Math.min(w, 64);
  for (let y = 0; y < rows; y++) for (let x = 0; x < cols; x++) {
    const i = (y * w + x) * 4, o = (y * 64 + x) * 4;
    out[o] = src[i]; out[o + 1] = src[i + 1]; out[o + 2] = src[i + 2]; out[o + 3] = src[i + 3];
  }
  // Graphics.drawImage(img, dx1, dy1, dx2, dy2, sx1, sy1, sx2, sy2): dx1 > dx2 mirrors the copy
  const copy = (dx1: number, dy1: number, dx2: number, dy2: number, sx1: number, sy1: number, sx2: number, sy2: number) => {
    for (let j = 0; j < sy2 - sy1; j++) for (let i = 0; i < sx2 - sx1; i++) {
      const dx = dx1 < dx2 ? dx1 + i : dx1 - 1 - i, dy = dy1 + j;
      const s = ((sy1 + j) * 64 + sx1 + i) * 4, d = (dy * 64 + dx) * 4;
      for (let k = 0; k < 4; k++) out[d + k] = out[s + k];
    }
  };
  if (h === 32) {
    copy(24, 48, 20, 52, 4, 16, 8, 20); copy(28, 48, 24, 52, 8, 16, 12, 20);
    copy(20, 52, 16, 64, 8, 20, 12, 32); copy(24, 52, 20, 64, 4, 20, 8, 32);
    copy(28, 52, 24, 64, 0, 20, 4, 32); copy(32, 52, 28, 64, 12, 20, 16, 32);
    copy(40, 48, 36, 52, 44, 16, 48, 20); copy(44, 48, 40, 52, 48, 16, 52, 20);
    copy(36, 52, 32, 64, 48, 20, 52, 32); copy(40, 52, 36, 64, 44, 20, 48, 32);
    copy(44, 52, 40, 64, 40, 20, 44, 32); copy(48, 52, 44, 64, 52, 20, 56, 32);
  }
  const opaque = (x0: number, y0: number, x1: number, y1: number) => {
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) out[(y * 64 + x) * 4 + 3] = 255;
  };
  const transparent = (x0: number, y0: number, x1: number, y1: number) => {
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) if (out[(y * 64 + x) * 4 + 3] < 128) return;
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) out[(y * 64 + x) * 4 + 3] = 0;
  };
  opaque(0, 0, 32, 16); transparent(32, 0, 64, 32); opaque(0, 16, 64, 32); opaque(16, 48, 48, 64);
  return out;
}

/** Skin and cape textures by served path, loaded on first use; a small LRU (textures are freed). */
export class SkinTextures {
  private readonly tex = new Map<string, WebGLTexture | null>();

  constructor(private readonly gl: WebGL2RenderingContext, private readonly max = 48) {}

  /** The texture, or null while it loads (or when it failed: the caller draws the default). */
  get(path: string | undefined, cape = false): WebGLTexture | null {
    if (!path || !SKIN_PATH.test(path)) return null;
    if (this.tex.has(path)) {
      const t = this.tex.get(path)!;
      this.tex.delete(path); this.tex.set(path, t);
      return t;
    }
    this.tex.set(path, null);
    void this.load(path, cape).then((t) => {
      if (!this.tex.has(path)) { if (t) this.gl.deleteTexture(t); return; }
      this.tex.set(path, t);
      while (this.tex.size > this.max) {
        const [old, ot] = this.tex.entries().next().value as [string, WebGLTexture | null];
        this.tex.delete(old);
        if (ot) this.gl.deleteTexture(ot);
      }
    });
    return null;
  }

  private async load(path: string, cape: boolean): Promise<WebGLTexture | null> {
    try {
      const res = await fetch(path);
      if (!res.ok) return null;
      const img = await createImageBitmap(await res.blob(), { premultiplyAlpha: 'none', colorSpaceConversion: 'none' } as ImageBitmapOptions);
      if (img.width !== 64 || (img.height !== 64 && img.height !== 32)) return null;
      if (cape) return texture2d(this.gl, img);
      const c = document.createElement('canvas');
      c.width = img.width; c.height = img.height;
      const ctx = c.getContext('2d', { willReadFrequently: true })!;
      ctx.drawImage(img, 0, 0);
      const px = normalizeSkin(ctx.getImageData(0, 0, img.width, img.height).data, img.width, img.height);
      return texture2d(this.gl, new ImageData(px, 64, 64));
    } catch { return null; }
  }
}
