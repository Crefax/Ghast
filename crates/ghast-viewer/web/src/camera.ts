/// <reference lib="dom" />
/**
 * Camera modes: first person (the bot's eyes, exact), third person (orbit around the bot: drag turns,
 * wheel / pinch zooms, double tap puts it behind the bot again) and free (drag looks, WASD/Space/Shift
 * or two fingers move, pinch flies forward). Only the view changes: nothing goes to the bot.
 */
import { clamp, forward } from './math.js';

export type Mode = 'fp' | 'tp' | 'free';
export interface View { x: number; y: number; z: number; yaw: number; pitch: number }

export class CameraControl {
  mode: Mode;
  private orbit = { yaw: 0, pitch: -0.35, dist: 5, init: false };
  private free: View = { x: 0, y: 80, z: 0, yaw: 0, pitch: 0 };
  private keys = new Set<string>();
  private pointers = new Map<number, { x: number; y: number }>();
  private pinch = 0;
  private lastTap = 0;
  onModeChange: (m: Mode) => void = () => {};

  constructor(private readonly el: HTMLElement, mode: Mode) {
    this.mode = mode;
    el.addEventListener('pointerdown', (e) => this.down(e));
    el.addEventListener('pointermove', (e) => this.move(e));
    el.addEventListener('pointerup', (e) => this.up(e));
    el.addEventListener('pointercancel', (e) => this.up(e));
    el.addEventListener('wheel', (e) => { e.preventDefault(); this.zoom(Math.sign(e.deltaY) * 0.12); }, { passive: false });
    window.addEventListener('keydown', (e) => { if (!(e.target instanceof HTMLInputElement)) this.keys.add(e.code); });
    window.addEventListener('keyup', (e) => this.keys.delete(e.code));
    window.addEventListener('blur', () => this.keys.clear());
  }

  setMode(m: Mode, from?: View): void {
    if (m === this.mode) return;
    if (m === 'free' && from) this.free = { ...from };
    if (m === 'tp') this.orbit.init = false;
    this.mode = m;
    this.onModeChange(m);
  }

  private down(e: PointerEvent): void {
    this.el.setPointerCapture?.(e.pointerId);
    this.pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (this.pointers.size === 2) this.pinch = this.spread();
    const now = performance.now();
    if (this.pointers.size === 1 && now - this.lastTap < 300 && this.mode === 'tp') this.orbit.init = false;
    this.lastTap = now;
  }

  private up(e: PointerEvent): void {
    this.pointers.delete(e.pointerId);
    this.pinch = this.pointers.size === 2 ? this.spread() : 0;
  }

  private spread(): number {
    const [a, b] = [...this.pointers.values()];
    return Math.hypot(a.x - b.x, a.y - b.y);
  }

  private move(e: PointerEvent): void {
    const p = this.pointers.get(e.pointerId);
    if (!p) return;
    const dx = e.clientX - p.x, dy = e.clientY - p.y;
    p.x = e.clientX; p.y = e.clientY;
    const sens = 0.0055;
    if (this.pointers.size === 2) {
      const s = this.spread();
      if (this.pinch) {
        const d = s - this.pinch;
        if (this.mode === 'tp') this.zoom(-d * 0.004);
        else if (this.mode === 'free') this.fly(0, 0, d * 0.03);
      }
      this.pinch = s;
      // two-finger drag pans in free mode (half per finger)
      if (this.mode === 'free') this.fly(-dx * 0.01, dy * 0.01, 0);
      return;
    }
    if (this.mode === 'tp') {
      this.orbit.yaw -= dx * sens;
      this.orbit.pitch = clamp(this.orbit.pitch - dy * sens, -1.5, 1.5);
    } else if (this.mode === 'free') {
      this.free.yaw -= dx * sens;
      this.free.pitch = clamp(this.free.pitch - dy * sens, -1.55, 1.55);
    }
  }

  private zoom(f: number): void {
    if (this.mode === 'tp') this.orbit.dist = clamp(this.orbit.dist * (1 + f), 1.5, 40);
    else if (this.mode === 'free') this.fly(0, 0, -f * 20);
  }

  /** Move the free camera: right, up, forward (blocks). */
  private fly(r: number, u: number, f: number): void {
    const v = this.free;
    const [fx, fy, fz] = forward(v.yaw, v.pitch);
    const rx = Math.cos(v.yaw), rz = -Math.sin(v.yaw);
    v.x += fx * f + rx * r; v.y += fy * f + u; v.z += fz * f + rz * r;
  }

  /** The camera for this frame from the bot's (interpolated) eye position and look. */
  update(dt: number, eye: View): View {
    if (this.mode === 'fp') return eye;
    if (this.mode === 'tp') {
      if (!this.orbit.init) { this.orbit.yaw = eye.yaw; this.orbit.pitch = -0.35; this.orbit.init = true; }
      const [fx, fy, fz] = forward(this.orbit.yaw, this.orbit.pitch);
      const d = this.orbit.dist;
      return { x: eye.x - fx * d, y: eye.y - fy * d, z: eye.z - fz * d, yaw: this.orbit.yaw, pitch: this.orbit.pitch };
    }
    const speed = (this.keys.has('ShiftLeft') && this.keys.has('ControlLeft') ? 40 : 12) * dt;
    let f = 0, r = 0, u = 0;
    if (this.keys.has('KeyW') || this.keys.has('ArrowUp')) f += speed;
    if (this.keys.has('KeyS') || this.keys.has('ArrowDown')) f -= speed;
    if (this.keys.has('KeyD') || this.keys.has('ArrowRight')) r += speed;
    if (this.keys.has('KeyA') || this.keys.has('ArrowLeft')) r -= speed;
    if (this.keys.has('Space')) u += speed;
    if (this.keys.has('ShiftLeft')) u -= speed;
    if (f || r || u) this.fly(r, u, f);
    return { ...this.free };
  }
}
