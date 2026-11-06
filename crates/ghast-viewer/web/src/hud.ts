/// <reference lib="dom" />
/**
 * HUD over the 3D view: Minecraft's hotbar (widgets.png) with item icons and counts, hearts and food
 * (icons.png), the XP bar, the crosshair, the status/coordinate lines and the debug overlay.
 * Everything is drawn with nearest-neighbour scaling in one canvas, redrawn only when it changes.
 */
import type { Baked } from './bake.js';

export interface Slot { id: number; meta: number; count: number }
export interface Vitals { health: number; food: number; saturation: number; level: number; progress: number }

/** Isometric (top / south / east) or flat icon of an item. */
export type IconSource = (id: number, meta: number) => { flat?: ImageBitmap | null; cube?: { top: ImageBitmap | null; left: ImageBitmap | null; right: ImageBitmap | null; tintTop?: number; tintSide?: number } } | null;

export class Hud {
  private readonly canvas: HTMLCanvasElement;
  private readonly ctx: CanvasRenderingContext2D;
  private slots: Slot[] = [];
  private held = 0;
  private vitals: Vitals | null = null;
  private dirty = true;
  private scale = 2;
  private readonly iconCache = new Map<string, HTMLCanvasElement | null>();
  visible = true;

  constructor(parent: HTMLElement, private readonly images: Map<string, ImageBitmap>, private readonly icon: IconSource) {
    this.canvas = document.createElement('canvas');
    this.canvas.id = 'hud-bar';
    parent.appendChild(this.canvas);
    this.ctx = this.canvas.getContext('2d')!;
  }

  setInventory(held: number, slots: Slot[]): void { this.held = held; this.slots = slots; this.dirty = true; }
  setVitals(v: Vitals): void { this.vitals = v; this.dirty = true; }
  invalidate(): void { this.dirty = true; }

  /** Redraws when something changed (or the window size did). */
  draw(): void {
    const w = window.innerWidth;
    const scale = w < 400 ? 2 : w < 900 ? 2 : 3;
    const dpr = Math.min(window.devicePixelRatio || 1, 3);
    if (scale !== this.scale) { this.scale = scale; this.dirty = true; }
    if (!this.dirty) return;
    this.dirty = false;
    const W = 182, H = 22 + 7 + 11;
    const c = this.canvas, ctx = this.ctx;
    c.style.width = `${W * scale}px`; c.style.height = `${H * scale}px`;
    c.width = Math.round(W * scale * dpr); c.height = Math.round(H * scale * dpr);
    ctx.setTransform(scale * dpr, 0, 0, scale * dpr, 0, 0);
    ctx.imageSmoothingEnabled = false;
    ctx.clearRect(0, 0, W, H);
    if (!this.visible) return;
    const widgets = this.images.get('gui/widgets'), icons = this.images.get('gui/icons');
    const top = H - 22;
    if (widgets) {
      ctx.drawImage(widgets, 0, 0, 182, 22, 0, top, 182, 22);
      ctx.drawImage(widgets, 0, 22, 24, 24, -1 + this.held * 20, top - 1, 24, 24);
    }
    for (let i = 0; i < 9; i++) {
      const s = this.slots[i];
      if (!s || !s.id) continue;
      const img = this.iconFor(s.id, s.meta);
      const x = 3 + i * 20, y = top + 3;
      if (img) ctx.drawImage(img, x, y, 16, 16);
      if (s.count > 1) {
        ctx.font = '600 8px ui-monospace, Menlo, Consolas, monospace';
        ctx.textAlign = 'right'; ctx.textBaseline = 'alphabetic';
        ctx.fillStyle = '#3f3f3f'; ctx.fillText(String(s.count), x + 17.5, y + 16.5);
        ctx.fillStyle = '#fff'; ctx.fillText(String(s.count), x + 17, y + 16);
      }
    }
    const v = this.vitals;
    if (icons && v) {
      // XP bar
      const xy = top - 7;
      ctx.drawImage(icons, 0, 64, 182, 5, 0, xy, 182, 5);
      const fill = Math.round(Math.max(0, Math.min(1, v.progress)) * 183);
      if (fill > 0) ctx.drawImage(icons, 0, 69, fill, 5, 0, xy, fill, 5);
      if (v.level > 0) {
        ctx.font = '700 8px ui-monospace, Menlo, Consolas, monospace';
        ctx.textAlign = 'center';
        ctx.fillStyle = '#000'; ctx.fillText(String(v.level), 91 + 0.6, xy + 0.6);
        ctx.fillStyle = '#80ff20'; ctx.fillText(String(v.level), 91, xy);
      }
      // hearts left, food right (from the right edge leftwards)
      const hy = top - 7 - 10;
      const hp = Math.max(0, Math.round(v.health)), food = Math.max(0, Math.round(v.food));
      for (let i = 0; i < 10; i++) {
        const x = i * 8;
        ctx.drawImage(icons, 16, 0, 9, 9, x, hy, 9, 9);
        if (i * 2 + 1 < hp) ctx.drawImage(icons, 52, 0, 9, 9, x, hy, 9, 9);
        else if (i * 2 + 1 === hp) ctx.drawImage(icons, 61, 0, 9, 9, x, hy, 9, 9);
      }
      for (let i = 0; i < 10; i++) {
        const x = 182 - 9 - i * 8;
        ctx.drawImage(icons, 16, 27, 9, 9, x, hy, 9, 9);
        if (i * 2 + 1 < food) ctx.drawImage(icons, 52, 27, 9, 9, x, hy, 9, 9);
        else if (i * 2 + 1 === food) ctx.drawImage(icons, 61, 27, 9, 9, x, hy, 9, 9);
      }
    }
  }

  /** 16×16 icon of an item (cached): its own texture, or the block as a small isometric cube. */
  iconFor(id: number, meta: number): HTMLCanvasElement | null {
    const k = `${id}:${meta}`;
    if (this.iconCache.has(k)) return this.iconCache.get(k)!;
    const src = this.icon(id, meta);
    let out: HTMLCanvasElement | null = null;
    if (src?.flat) {
      out = document.createElement('canvas'); out.width = 16; out.height = 16;
      const ctx = out.getContext('2d')!;
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(src.flat, 0, 0, 16, 16);
    } else if (src?.cube) {
      out = isoCube(src.cube);
    }
    this.iconCache.set(k, out);
    return out;
  }
}

function face(img: ImageBitmap | null, shade: number, tint?: number): HTMLCanvasElement {
  const c = document.createElement('canvas'); c.width = 16; c.height = 16;
  const ctx = c.getContext('2d')!;
  ctx.imageSmoothingEnabled = false;
  if (!img) return c;
  ctx.drawImage(img, 0, 0, 16, 16);
  ctx.globalCompositeOperation = 'source-atop';
  if (tint !== undefined) {
    ctx.globalCompositeOperation = 'multiply';
    ctx.fillStyle = `#${tint.toString(16).padStart(6, '0')}`;
    ctx.fillRect(0, 0, 16, 16);
    ctx.globalCompositeOperation = 'destination-in';
    ctx.drawImage(img, 0, 0, 16, 16);
    ctx.globalCompositeOperation = 'source-atop';
  }
  if (shade < 1) { ctx.fillStyle = `rgba(0,0,0,${1 - shade})`; ctx.fillRect(0, 0, 16, 16); }
  return c;
}

/** The inventory look of a block: top, left (south) and right (east) faces. */
function isoCube(c: NonNullable<NonNullable<ReturnType<IconSource>>['cube']>): HTMLCanvasElement {
  const S = 4; // draw at 64×64 for smooth edges, scaled down on use
  const out = document.createElement('canvas'); out.width = 16 * S; out.height = 16 * S;
  const ctx = out.getContext('2d')!;
  ctx.imageSmoothingEnabled = false;
  const k = S;
  ctx.setTransform(0.5 * k, 0.25 * k, -0.5 * k, 0.25 * k, 8 * k, 0.4 * k);
  ctx.drawImage(face(c.top, 1, c.tintTop), 0, 0);
  ctx.setTransform(0.5 * k, 0.25 * k, 0, 0.53 * k, 0, 4.4 * k);
  ctx.drawImage(face(c.left, 0.8, c.tintSide), 0, 0);
  ctx.setTransform(0.5 * k, -0.25 * k, 0, 0.53 * k, 8 * k, 8.4 * k);
  ctx.drawImage(face(c.right, 0.6, c.tintSide), 0, 0);
  return out;
}

export type { Baked };
