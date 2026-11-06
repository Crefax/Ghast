/**
 * The page's vanilla 1.8 animation maths (anim.ts): body yaw, limb swing, hurt/death, arm swing, BiPedModel
 * poses, view bobbing, the first-person hand, pickups, orbs, chest lids.
 */
import { describe, expect, it } from 'vitest';
import {
  advance, bipedPose, bobMatrix, deathAngle, firstPerson, fpArmMatrix, fpItemMatrix, heldPose, living, livingAt, orbColor, orbFrame,
  lidDegrees, lidHinge, pickupAt, startSwing, stepFirstPerson, stepLid, stepLiving, useAction, wrap, DEATH_TICKS, HURT_TICKS,
} from '../src/anim.js';

const deg = (r: number) => (r * 180) / Math.PI;

describe('animation maths (vanilla 1.8)', () => {
  it('turns the body towards the walk, keeps it within 75° of the look, and swings the limbs with the speed', () => {
    const l = living(0, 0, 0);
    // walking east (+x) while looking north (yaw 0): the body eases 30 % towards the walk (yaw −π/2) and is pulled
    // 20 % back past 50° off the look: it settles where b = 0.8 (0.7 b − 27°), at −49.1°
    for (let i = 0; i < 40; i++) stepLiving(l, (i + 1) * 0.2, 0, 0);
    expect(deg(wrap(l.body))).toBeCloseTo(-21.6 / 0.44, 1);
    expect(l.limbAmt).toBeGreaterThan(0.75);
    expect(l.limbAmt).toBeLessThanOrEqual(0.8 + 1e-9);
    // standing: the amount decays, the body stays put while the look turns less than 75°
    const body = l.body;
    for (let i = 0; i < 40; i++) stepLiving(l, 8, 0, 0.3);
    expect(l.limbAmt).toBeLessThan(0.01);
    expect(Math.abs(wrap(l.body - body))).toBeLessThan(0.5);
    // looking 120° away: the body follows to stay within 75°
    for (let i = 0; i < 5; i++) stepLiving(l, 8, 0, 2.5);
    expect(Math.abs(deg(wrap(2.5 - l.body)))).toBeLessThanOrEqual(75.01);
  });

  it('counts hurt down from 10 and death up to 20; the corpse falls 90° (spiders 180°)', () => {
    const l = living(0, 0, 0);
    l.hurt = HURT_TICKS;
    expect(livingAt(l, 0).hurt).toBe(true);
    for (let i = 0; i < 10; i++) stepLiving(l, 0, 0, 0);
    expect(livingAt(l, 0).hurt).toBe(false);
    l.death = 1;
    for (let i = 0; i < 30; i++) stepLiving(l, 0, 0, 0);
    expect(l.death).toBe(DEATH_TICKS);
    expect(livingAt(l, 0.5).hurt).toBe(true); // red while dying
    expect(deathAngle(0)).toBe(0);
    expect(deathAngle(1)).toBe(0);
    expect(deg(deathAngle(20))).toBeCloseTo(90, 5);
    expect(deg(deathAngle(20, 180))).toBeCloseTo(180, 5);
    expect(deg(deathAngle(5))).toBeCloseTo(Math.sqrt((4 / 20) * 1.6) * 90, 5);
  });

  it('swings the arm over 6 ticks, running on to 1 in the last; a swing in its first half is not restarted', () => {
    const l = living(0, 0, 0);
    expect(livingAt(l, 0).swing).toBeUndefined();
    startSwing(l);
    const seen: number[] = [];
    for (let i = 0; i < 8; i++) { stepLiving(l, 0, 0, 0); seen.push(livingAt(l, 0.5).swing ?? -1); }
    // the first tick is at 0 (int −1 → 0), then 1/6 a tick; the last tick runs from 5/6 on to 1
    expect(seen[0]).toBe(-1);
    for (let i = 2; i <= 6; i++) expect(seen[i]).toBeGreaterThan(seen[i - 1]);
    expect(seen[6]).toBeCloseTo((5 / 6 + 1) / 2, 5);
    expect(seen[7]).toBe(-1);
    startSwing(l);
    stepLiving(l, 0, 0, 0); stepLiving(l, 0, 0, 0);
    const t = l.swingT;
    startSwing(l); // first half: keeps going
    expect(l.swingT).toBe(t);
  });

  it('steps whole ticks of page time and renders the partial tick', () => {
    const l = living(0, 0, 0);
    expect(advance(l, 120, 0, 0, 0)).toBeCloseTo(0.4, 5);
    expect(l.ticks).toBe(2);
    expect(advance(l, 5000, 0, 0, 0)).toBeCloseTo(0, 5); // away for 5 s: one tick, not a hundred
    expect(l.ticks).toBe(3);
  });

  it('poses BiPedModel: idle sway, held item, blocking, sneak, riding, bow and zombie arms', () => {
    const rest = { limb: 0, limbAmt: 0, age: 0, headYaw: 0, pitch: 0, swing: 0 };
    const idle = bipedPose(rest);
    expect(idle.rightArm.rz).toBeCloseTo(0.1, 6); // cos(0) × 0.05 + 0.05
    expect(idle.leftArm.rz).toBeCloseTo(-0.1, 6);
    expect([idle.rightArm.px, idle.rightArm.pz, idle.leftLeg.py, idle.leftLeg.pz]).toEqual([-5, 0, 12, 0.1]);
    expect(bipedPose({ ...rest, held: 1 }).rightArm.rx).toBeCloseTo(-Math.PI / 10, 6);
    const block = bipedPose({ ...rest, ...heldPose(276, true) });
    expect(block.rightArm.rx).toBeCloseTo(-Math.PI * 3 / 10, 6);
    expect(block.rightArm.ry).toBeCloseTo(-0.5235988, 6);
    const sneak = bipedPose({ ...rest, sneak: true });
    expect([sneak.body.rx, sneak.head.py, sneak.rightLeg.py, sneak.rightLeg.pz]).toEqual([0.5, 1, 9, 4]);
    expect(sneak.rightArm.rx).toBeCloseTo(0.4, 6);
    const ride = bipedPose({ ...rest, riding: true });
    expect(ride.rightLeg.rx).toBeCloseTo(-Math.PI * 2 / 5, 6);
    expect([ride.rightLeg.ry, ride.leftLeg.ry]).toEqual([Math.PI / 10, -Math.PI / 10]);
    const bow = bipedPose({ ...rest, ...heldPose(261, true), pitch: 0.2 });
    expect(bow.rightArm.rx).toBeCloseTo(-Math.PI / 2 + 0.2, 6);
    expect(bow.leftArm.ry).toBeCloseTo(0.5, 6);
    const zombie = bipedPose({ ...rest, zombie: true });
    expect(zombie.rightArm.rx).toBeCloseTo(-Math.PI / 2, 6);
    expect(zombie.leftArm.ry).toBeCloseTo(0.1, 6);
    // walking: the legs swing against each other, the arms against the legs
    const walk = bipedPose({ ...rest, limb: 1, limbAmt: 1 });
    expect(walk.rightLeg.rx).toBeCloseTo(-walk.leftLeg.rx, 6);
    expect(Math.sign(walk.rightArm.rx)).toBe(-Math.sign(walk.rightLeg.rx));
    // the swing turns the body and pulls the arm forward
    const hit = bipedPose({ ...rest, swing: 0.5 });
    expect(hit.body.ry).not.toBe(0);
    expect(hit.rightArm.rx).toBeLessThan(-0.7);
    expect(useAction(297)).toBe('eat');
    expect(heldPose(297, true)).toEqual({ held: 1, bow: false });
    expect(heldPose(0, true)).toEqual({ held: 0, bow: false });
  });

  it('bobs the view only while walking on the ground, tilts it when hurt, dips the hand when the held item changes', () => {
    const f = firstPerson();
    for (let i = 0; i < 20; i++) stepFirstPerson(f, 0, 4, 0, true, 0, 0, '276:0:1', false);
    const still = bobMatrix(new Float32Array(16), f, 0.5);
    expect([...still]).toEqual([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
    for (let i = 0; i < 20; i++) stepFirstPerson(f, (i + 1) * 0.2158, 4, 0, true, 0, 0, '276:0:1', false);
    expect(f.bob).toBeCloseTo(0.1, 2);
    const walking = bobMatrix(new Float32Array(16), f, 0.3);
    expect(Math.abs(walking[13]) + Math.abs(walking[12])).toBeGreaterThan(0.001);
    f.hurt = HURT_TICKS - 2; // the tilt peaks a couple of ticks after the hit
    f.bob = f.prevBob = 0;
    const hurt = bobMatrix(new Float32Array(16), f, 0.2, false);
    expect(Math.abs(hurt[1])).toBeGreaterThan(0.1); // rolled about z
    // a new item: down by 0.4 a tick, swapped below 0.1, back up
    const eq: number[] = [];
    for (let i = 0; i < 6; i++) { stepFirstPerson(f, 4, 4, 0, true, 0, 0, '297:0:1', false); eq.push(Math.round(f.equip * 10) / 10); }
    expect(eq).toEqual([0.6, 0.2, 0, 0.4, 0.8, 1]);
    expect(f.shown).toBe('297:0:1');
    // eating counts ticks in use
    stepFirstPerson(f, 4, 4, 0, true, 0, 0, '297:0:1', true);
    stepFirstPerson(f, 4, 4, 0, true, 0, 0, '297:0:1', true);
    expect(f.use).toBe(1);
  });

  it('places the first-person item and arm low right of the crosshair, the item lifted to the mouth when eating', () => {
    const f = firstPerson();
    stepFirstPerson(f, 0, 4, 0, true, 0, 0, '276:0:1', false);
    const at = (m: Float32Array) => [m[12], m[13], m[14]];
    const [x, y, z] = at(fpItemMatrix(new Float32Array(16), f, 0, 0, 0, 0, false, 'block', -1));
    expect(x).toBeGreaterThan(0.3); expect(y).toBeLessThan(-0.2); expect(z).toBeLessThan(-0.4);
    const eat = at(fpItemMatrix(new Float32Array(16), f, 0, 0, 0, 0, false, 'eat', 10));
    expect(eat[0]).toBeLessThan(x); // towards the middle
    const arm = at(fpArmMatrix(new Float32Array(16), f, 0, 0, 0, 0));
    expect(arm[0]).toBeGreaterThan(0); expect(arm[1]).toBeLessThan(0); expect(arm[2]).toBeLessThan(0);
    for (const v of fpItemMatrix(new Float32Array(16), f, 0.5, 1, -0.4, 0.6, true, 'none', -1)) expect(Number.isFinite(v)).toBe(true);
  });

  it('flies pickups to the collector in 3 ticks (eased), picks orb frames by value and pulses their colour', () => {
    expect(pickupAt([0, 0, 0], [4, 1, 0], 0)).toEqual([0, 0, 0]);
    expect(pickupAt([0, 0, 0], [4, 1, 0], 3)).toEqual([4, 1.5, 0]);
    expect(pickupAt([0, 0, 0], [4, 1, 0], 1.5)[0]).toBeCloseTo(1, 6); // (½)² of the way
    expect([1, 2, 3, 6, 7, 16, 17, 36, 37, 2476, 2477, 30000].map(orbFrame)).toEqual([0, 0, 1, 1, 2, 2, 3, 3, 4, 9, 10, 10]);
    const c = orbColor(0);
    expect(c[1]).toBe(1);
    expect(c[0]).toBeCloseTo(0.5, 6);
    expect(orbColor(Math.PI)[0]).not.toBeCloseTo(c[0], 2);
  });

  it('opens a chest lid 0.1 a tick, eased out to 90°, on the hinge at its back', () => {
    let a = 0;
    const seen: number[] = [];
    for (let i = 0; i < 12; i++) { a = stepLid(a, true); seen.push(a); }
    expect(seen[9]).toBeCloseTo(1, 6);
    expect(seen[11]).toBe(1);
    expect(lidDegrees(0, 0, 0.5)).toBe(0);
    expect(lidDegrees(1, 1, 0)).toBeCloseTo(90, 6);
    expect(lidDegrees(0, 0.1, 1)).toBeCloseTo((1 - 0.9 ** 3) * 90, 6); // fast at first
    expect(stepLid(0.05, false)).toBe(0);
    // the front is the meta's facing; the hinge is the opposite edge and lifting turns the front up
    for (const [meta, front] of [[2, [0, -1]], [3, [0, 1]], [4, [-1, 0]], [5, [1, 0]]] as Array<[number, [number, number]]>) {
      const h = lidHinge(meta);
      expect([Math.sign(h.x), Math.sign(h.z)]).toEqual([-front[0] || 0, -front[1] || 0].map(v => Math.sign(v)));
      // a point at the front edge (relative to the hinge), turned by +sign about the axis, goes up
      const rx = front[0] * 14 / 16, rz = front[1] * 14 / 16, t = (h.sign * Math.PI) / 4;
      const up = h.ax ? -rz * Math.sin(t) : rx * Math.sin(t);
      expect(up).toBeGreaterThan(0);
    }
  });
});
