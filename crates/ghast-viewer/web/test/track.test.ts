/** Pose samples → a smooth state (track.ts) and the replay control message (replay.ts). */
import { describe, expect, it } from 'vitest';
import { Track, type PoseSample } from '../src/track.js';
import { replayControl, span } from '../src/replay.js';
import { C } from '../src/shared/protocol.js';

const sample = (t: number, x: number, yaw = 0): PoseSample => ({ t, x, y: 64, z: 0, yaw, pitch: 0, headYaw: yaw, flags: 0 });

describe('track', () => {
  it('interpolates between samples and keeps the flags of the nearer one', () => {
    const tr = new Track(100, 0);
    tr.push({ ...sample(0, 0), flags: 1 }); tr.push({ ...sample(50, 1), flags: 2 });
    expect(tr.raw(25)!.x).toBeCloseTo(0.5);
    expect(tr.raw(10)!.flags).toBe(1);
    expect(tr.raw(40)!.flags).toBe(2);
    expect(tr.raw(-10)!.x).toBe(0); // before the first sample: held
  });

  it('extrapolates past the newest sample, capped, and not across a long gap', () => {
    const tr = new Track(100, 0);
    tr.push(sample(0, 0)); tr.push(sample(50, 1));
    expect(tr.raw(75)!.x).toBeCloseTo(1.5);
    expect(tr.raw(1000)!.x).toBeCloseTo(1 + 100 / 50); // at most 100 ms ahead
    const gap = new Track(100, 0);
    gap.push(sample(0, 0)); gap.push(sample(600, 1));
    expect(gap.raw(650)!.x).toBe(1);
  });

  it('turns the shortest way round (yaw wraps)', () => {
    const tr = new Track(100, 0);
    tr.push(sample(0, 0, Math.PI - 0.1)); tr.push(sample(50, 0, -Math.PI + 0.1));
    const mid = tr.raw(25)!.yaw;
    expect(Math.abs(Math.abs(mid) - Math.PI)).toBeLessThan(1e-6);
  });

  it('ignores older samples, replaces one of the same time, keeps at most 40', () => {
    const tr = new Track();
    tr.push(sample(10, 1)); tr.push(sample(5, 9));
    expect(tr.samples).toHaveLength(1);
    tr.push(sample(10, 2));
    expect(tr.newest()!.x).toBe(2);
    for (let i = 0; i < 100; i++) tr.push(sample(20 + i, i));
    expect(tr.samples).toHaveLength(40);
  });

  it('fades a misprediction out instead of snapping, but snaps on a teleport', () => {
    const tr = new Track(100, 60);
    tr.push(sample(0, 0)); tr.push(sample(50, 1));
    expect(tr.at(50, 1000)!.x).toBeCloseTo(1);
    tr.push(sample(100, 1.2)); // the target moves on from where the picture is (1)
    const shown = tr.at(100, 1016)!.x;
    expect(shown).toBeGreaterThan(1); // still near the old picture...
    expect(shown).toBeLessThan(1.1); // ...not jumped to the target
    let x = shown;
    for (let t = 1032; t < 1600; t += 16) x = tr.at(100, t)!.x;
    expect(x).toBeCloseTo(1.2, 3);
    tr.push(sample(150, 50)); // a teleport
    expect(tr.at(150, 1616)!.x).toBeCloseTo(50);
  });
});

describe('replay control', () => {
  it('writes C.REPLAY: f64 speed, f64 seek', () => {
    const v = new DataView(replayControl(4, 12_500));
    expect(v.byteLength).toBe(17);
    expect([v.getUint8(0), v.getFloat64(1, true), v.getFloat64(9, true)]).toEqual([C.REPLAY, 4, 12_500]);
    const keep = new DataView(replayControl(Number.NaN, -1));
    expect(Number.isNaN(keep.getFloat64(1, true))).toBe(true);
    expect(keep.getFloat64(9, true)).toBe(-1);
  });

  it('formats spans as m:ss', () => {
    expect([span(0), span(59_400), span(61_000), span(3_600_000), span(-5)]).toEqual(['0:00', '0:59', '1:01', '60:00', '0:00']);
  });
});
