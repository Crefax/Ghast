/**
 * Timestamped samples → a smooth state at any time: interpolation between samples, linear
 * extrapolation past the newest (capped), and an error offset that absorbs mispredictions over
 * `smoothMs` instead of snapping.
 */
import { lerp, lerpAngle } from './math.js';

export interface PoseSample { t: number; x: number; y: number; z: number; yaw: number; pitch: number; headYaw: number; flags: number }

export class Track {
  readonly samples: PoseSample[] = [];
  /** Bumps on every new sample (the latency probe and the error smoothing look at it). */
  version = 0;
  private err = { x: 0, y: 0, z: 0, yaw: 0, pitch: 0 };
  private shown: PoseSample | null = null;
  private seenVersion = 0;
  private lastAt = 0;

  constructor(private readonly maxExtrapolateMs = 100, private readonly smoothMs = 60) {}

  push(s: PoseSample): void {
    const last = this.samples[this.samples.length - 1];
    if (last && s.t <= last.t) {
      if (s.t === last.t) this.samples[this.samples.length - 1] = s;
      else return;
    } else this.samples.push(s);
    if (this.samples.length > 40) this.samples.shift();
    this.version++;
  }

  newest(): PoseSample | undefined { return this.samples[this.samples.length - 1]; }

  clear(): void { this.samples.length = 0; this.shown = null; this.err = { x: 0, y: 0, z: 0, yaw: 0, pitch: 0 }; }

  /** Raw state at time t (no smoothing). */
  raw(t: number): PoseSample | null {
    const ss = this.samples;
    if (!ss.length) return null;
    const last = ss[ss.length - 1];
    if (t >= last.t) {
      const prev = ss[ss.length - 2];
      if (!prev || last.t - prev.t > 500 || last.t === prev.t) return { ...last, t };
      const dt = Math.min(t - last.t, this.maxExtrapolateMs);
      const k = dt / (last.t - prev.t);
      // positions run on; rotation only half way (bots turn in steps that stop)
      return {
        t, x: last.x + (last.x - prev.x) * k, y: last.y + (last.y - prev.y) * k, z: last.z + (last.z - prev.z) * k,
        yaw: lerpAngle(last.yaw, lerpAngle(prev.yaw, last.yaw, 1 + k), 0.5), pitch: last.pitch, headYaw: last.headYaw, flags: last.flags,
      };
    }
    let i = ss.length - 1;
    while (i > 0 && ss[i - 1].t > t) i--;
    if (i === 0) return { ...ss[0], t };
    const a = ss[i - 1], b = ss[i];
    const f = (t - a.t) / Math.max(1e-6, b.t - a.t);
    return {
      t, x: lerp(a.x, b.x, f), y: lerp(a.y, b.y, f), z: lerp(a.z, b.z, f),
      yaw: lerpAngle(a.yaw, b.yaw, f), pitch: lerp(a.pitch, b.pitch, f), headYaw: lerpAngle(a.headYaw, b.headYaw, f), flags: f < 0.5 ? a.flags : b.flags,
    };
  }

  /** Smoothed state for a frame at time t (`now` is the wall clock in ms, for the error decay). */
  at(t: number, now: number): PoseSample | null {
    const target = this.raw(t);
    if (!target) return null;
    const dt = this.lastAt ? Math.min(100, now - this.lastAt) : 0;
    this.lastAt = now;
    if (this.shown && this.version !== this.seenVersion) {
      // a new sample moved the target: keep the picture where it was and let the difference fade out
      const e = this.err;
      e.x = this.shown.x - target.x; e.y = this.shown.y - target.y; e.z = this.shown.z - target.z;
      e.yaw = lerpAngle(0, this.shown.yaw - target.yaw, 1); e.pitch = this.shown.pitch - target.pitch;
      if (Math.abs(e.x) + Math.abs(e.y) + Math.abs(e.z) > 4) { e.x = e.y = e.z = 0; e.yaw = 0; e.pitch = 0; } // teleport
    }
    this.seenVersion = this.version;
    const k = this.smoothMs > 0 ? Math.exp(-dt / this.smoothMs) : 0;
    const e = this.err;
    e.x *= k; e.y *= k; e.z *= k; e.yaw *= k; e.pitch *= k;
    const out = { ...target, x: target.x + e.x, y: target.y + e.y, z: target.z + e.z, yaw: target.yaw + e.yaw, pitch: target.pitch + e.pitch };
    this.shown = out;
    return out;
  }
}
