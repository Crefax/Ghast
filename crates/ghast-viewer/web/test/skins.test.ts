/** Player skins on the page (skins.ts): the legacy 64×32 → 64×64 conversion and the served path format. */
import { describe, expect, it } from 'vitest';
import { normalizeSkin, SKIN_PATH } from '../src/skins.js';

describe('skin paths', () => {
  it('accepts only content-addressed skins/<32 hex>.png paths', () => {
    expect(SKIN_PATH.test('skins/0123456789abcdef0123456789abcdef.png')).toBe(true);
    expect(SKIN_PATH.test('skins/../secret.png')).toBe(false);
    expect(SKIN_PATH.test('http://example.com/skins/0123456789abcdef0123456789abcdef.png')).toBe(false);
  });
});

describe('skins on the page', () => {
  const px = (a: Uint8ClampedArray, x: number, y: number) => Array.from(a.subarray((y * 64 + x) * 4, (y * 64 + x) * 4 + 4));

  it('turns a legacy 64×32 skin into 64×64 like DownloadedSkinParser: mirrored left limbs, opaque base, no outer layer', () => {
    const src = new Uint8ClampedArray(64 * 32 * 4);
    for (let y = 0; y < 32; y++) for (let x = 0; x < 64; x++) {
      const i = (y * 64 + x) * 4;
      src[i] = x * 4; src[i + 1] = y * 8; src[i + 2] = 7; src[i + 3] = 100; // semi-transparent everywhere
    }
    const out = normalizeSkin(src, 64, 32);
    expect(out.length).toBe(64 * 64 * 4);
    // the right leg's front (4..8, 20..32) mirrored into the left leg's front (20..24, 52..64)
    expect(px(out, 20, 52).slice(0, 3)).toEqual(px(out, 7, 20).slice(0, 3));
    expect(px(out, 23, 60).slice(0, 3)).toEqual(px(out, 4, 28).slice(0, 3));
    // the right arm's top (44..48, 16..20) into the left arm's top (36..40, 48..52), mirrored
    expect(px(out, 36, 48).slice(0, 3)).toEqual(px(out, 47, 16).slice(0, 3));
    // base layer opaque, the outer layer of the new half empty, a hat with see-through pixels kept
    expect(px(out, 10, 10)[3]).toBe(255);
    expect(px(out, 20, 20)[3]).toBe(255);
    expect(px(out, 20, 60)[3]).toBe(255);
    expect(px(out, 4, 36)[3]).toBe(0);
    expect(px(out, 40, 5)[3]).toBe(100);
    // an all-opaque hat (old skins painted it solid) is cleared
    const solid = new Uint8ClampedArray(src); for (let i = 3; i < solid.length; i += 4) solid[i] = 255;
    expect(px(normalizeSkin(solid, 64, 32), 40, 5)[3]).toBe(0);
  });

  it('keeps a 64×64 skin\'s own left limbs and outer layer', () => {
    const src = new Uint8ClampedArray(64 * 64 * 4);
    for (let i = 0; i < src.length; i += 4) { src[i] = (i / 4) % 251; src[i + 3] = 255; }
    src[(40 * 64 + 4) * 4 + 3] = 0; // a hole in the pants
    const out = normalizeSkin(src, 64, 64);
    expect(px(out, 20, 52)).toEqual(px(src, 20, 52));
    expect(px(out, 4, 40)[3]).toBe(0);
    // a hat region that is all opaque is cleared on 64×64 skins too
    expect(px(out, 40, 5)[3]).toBe(0);
  });
});
