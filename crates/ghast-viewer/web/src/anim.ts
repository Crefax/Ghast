/**
 * Animation maths of the page, as vanilla 1.8 does it (no GL, so the tests check it):
 *  - the per-tick living state of every entity (LivingEntity.tick: bodyYaw turning with the walk
 *    and lagging the look by at most 75°, field_6750, hurtTime, deathTime, the arm swing of 6 ticks),
 *  - BiPedModel / AbstractZombieModel.setAngles (idle arm sway, held item, blocking, bow, sneak, riding, swing),
 *  - LivingEntityRenderer's corpse rotation and red hurt overlay,
 *  - first person: GameRenderer's view bobbing and hurt tilt, HeldItemRenderer's hand (equip dip, swing, sway behind
 *    the look, eating, blocking, drawing a bow), the collect animation and XP orbs.
 * Angles on the wire are radians (yaw 0 looks to −z, +yaw turns left; see protocol.ts); MC degrees only inside the matrices.
 */
import { mat4, multiply, type Mat4 } from './math.js';

const PI = Math.PI;
const DEG = PI / 180;

/** Angle difference wrapped to −π..π (MathHelper.wrapDegrees). */
export function wrap(a: number): number {
  let d = a % (PI * 2);
  if (d >= PI) d -= PI * 2;
  if (d < -PI) d += PI * 2;
  return d;
}

// ---- the living state, one step per game tick -------------------------------------------------------------

export interface Living {
  /** Ticks stepped (ageInTicks) and ms into the next one. */
  ticks: number; acc: number;
  x: number; z: number;
  /** bodyYaw (wire radians) this tick and the last. */
  body: number; prevBody: number;
  limb: number; limbAmt: number; prevLimbAmt: number;
  /** hurtTime: 10 on a hit, down to 0 (the red overlay while > 0). */
  hurt: number;
  /** deathTime: 1 on death, up to 20 (then the entity is gone). */
  death: number;
  /** handSwingTicks 0..5 while `swingOn` (6 ticks), handSwingProgress this tick and the last. */
  swingT: number; swingOn: boolean; prevSwing: number; swing: number;
}

export const SWING_TICKS = 6, HURT_TICKS = 10, DEATH_TICKS = 20;

export function living(x: number, z: number, yaw: number): Living {
  return { ticks: 0, acc: 0, x, z, body: yaw, prevBody: yaw, limb: 0, limbAmt: 0, prevLimbAmt: 0, hurt: 0, death: 0, swingT: 0, swingOn: false, prevSwing: 0, swing: 0 };
}

/** LivingEntity.swingHand: restarts unless the swing is in its first half. */
export function startSwing(l: Living): void {
  if (!l.swingOn || l.swingT >= SWING_TICKS / 2 || l.swingT < 0) { l.swingT = -1; l.swingOn = true; }
}

/** One tick at the entity's position (x, z) with its look yaw. */
export function stepLiving(l: Living, x: number, z: number, yaw: number): void {
  const dx = x - l.x, dz = z - l.z;
  l.x = x; l.z = z;
  const d2 = dx * dx + dz * dz;
  // the body faces the walk (atan2 of the motion), or the look while swinging
  let target = l.body;
  if (d2 > 0.0025000002) target = Math.atan2(-dx, -dz);
  if (l.swing > 0) target = yaw;
  const before = l.body;
  // LivingEntity.turnHead: ease 30 % towards it, then keep within 75° of the look (and pull a little past 50°)
  l.body += wrap(target - l.body) * 0.3;
  let f1 = wrap(yaw - l.body);
  const max = 75 * DEG;
  if (f1 < -max) f1 = -max;
  if (f1 >= max) f1 = max;
  l.body = yaw - f1;
  if (f1 * f1 > (50 * DEG) ** 2) l.body += f1 * 0.2;
  l.prevBody = l.body - wrap(l.body - before);
  // limb swing (travel): 4 per block, amount eases at 40 %
  l.prevLimbAmt = l.limbAmt;
  l.limbAmt += (Math.min(1, Math.sqrt(d2) * 4) - l.limbAmt) * 0.4;
  l.limb += l.limbAmt;
  if (l.hurt > 0) l.hurt--;
  if (l.death > 0 && l.death < DEATH_TICKS) l.death++;
  // tickHandSwing
  l.prevSwing = l.swing;
  if (l.swingOn) { l.swingT++; if (l.swingT >= SWING_TICKS) { l.swingT = 0; l.swingOn = false; } }
  else l.swingT = 0;
  l.swing = l.swingT / SWING_TICKS;
  l.ticks++;
}

/** Steps whole ticks for `dtMs` of page time; returns the partial tick (0..1) to render at. */
export function advance(l: Living, dtMs: number, x: number, z: number, yaw: number): number {
  l.acc += dtMs;
  // a page that was away does not replay minutes of ticks
  if (l.acc > 500) l.acc = 50;
  while (l.acc >= 50) { l.acc -= 50; stepLiving(l, x, z, yaw); }
  return l.acc / 50;
}

/** What the renderer needs at partial tick pt (vanilla's interpolations). */
export function livingAt(l: Living, pt: number): { body: number; limb: number; limbAmt: number; swing: number | undefined; age: number; hurt: boolean; death: number } {
  // getHandSwingProgress: the last tick of a swing runs on to 1
  let d = l.swing - l.prevSwing;
  if (d < 0) d += 1;
  const swing = l.prevSwing + d * pt;
  return {
    body: l.prevBody + wrap(l.body - l.prevBody) * pt,
    limb: l.limb - l.limbAmt * (1 - pt),
    limbAmt: l.prevLimbAmt + (l.limbAmt - l.prevLimbAmt) * pt,
    swing: swing > 0 && (l.swingOn || l.prevSwing > 0) ? swing : undefined,
    age: l.ticks + pt,
    hurt: l.hurt > 0 || l.death > 0,
    death: l.death > 0 ? l.death + pt : 0,
  };
}

/** LivingEntityRenderer.method_5777: the fall about the body's z axis (radians); 90° for most, 180° for spiders. */
export function deathAngle(deathTime: number, max = 90): number {
  if (deathTime <= 0) return 0;
  const f = Math.sqrt(Math.max(0, ((deathTime - 1) / DEATH_TICKS) * 1.6));
  return Math.min(1, f) * max * DEG;
}

/** LivingEntityRenderer.method_10252: red at 30 % over a hurt or dying entity (mixed in before the light). */
export const HURT_OVERLAY: [number, number, number, number] = [1, 0, 0, 0.3];

// ---- BiPedModel / AbstractZombieModel ---------------------------------------------------------------------------------

export interface Pose { px: number; py: number; pz: number; rx: number; ry: number; rz: number }

export interface BipedIn {
  /** field_6750, field_6749, ageInTicks. */
  limb: number; limbAmt: number; age: number;
  /** Head yaw relative to the body and pitch, MC model radians. */
  headYaw: number; pitch: number;
  /** handSwingProgress 0..1 (0 when idle). */
  swing: number;
  sneak?: boolean; riding?: boolean;
  /** rightArmPose: 0 empty, 1 an item, 3 blocking with a sword. */
  held?: number;
  /** aiming (a player drawing a bow). */
  bow?: boolean;
  /** AbstractZombieModel's arms held forward (zombies, pigmen, and 1.8's skeletons). */
  zombie?: boolean;
}

export type BipedPart = 'head' | 'body' | 'rightArm' | 'leftArm' | 'rightLeg' | 'leftLeg';

/** BiPedModel.setAngles (+ AbstractZombieModel's arms): every part's pivot and rotation. `legX`: the legs' x pivot. */
export function bipedPose(p: BipedIn, legX = 1.9): Record<BipedPart, Pose> {
  const P = (px: number, py: number, pz: number): Pose => ({ px, py, pz, rx: 0, ry: 0, rz: 0 });
  const head = P(0, 0, 0), body = P(0, 0, 0), ra = P(-5, 2, 0), la = P(5, 2, 0), rl = P(-legX, 12, 0.1), ll = P(legX, 12, 0.1);
  const c = 0.6662;
  head.ry = p.headYaw; head.rx = p.pitch;
  ra.rx = Math.cos(p.limb * c + PI) * 2 * p.limbAmt * 0.5;
  la.rx = Math.cos(p.limb * c) * 2 * p.limbAmt * 0.5;
  rl.rx = Math.cos(p.limb * c) * 1.4 * p.limbAmt;
  ll.rx = Math.cos(p.limb * c + PI) * 1.4 * p.limbAmt;
  if (p.riding) {
    ra.rx += -PI / 5; la.rx += -PI / 5;
    rl.rx = ll.rx = -PI * 2 / 5;
    rl.ry = PI / 10; ll.ry = -PI / 10;
  }
  const held = p.held ?? 0;
  if (held === 1 || held === 3) ra.rx = ra.rx * 0.5 - (PI / 10) * held;
  if (held === 3) ra.ry = -0.5235988;
  // the swing turns the body and carries the arms' pivots round with it (runs at 0 too: pivots reset)
  const s = p.swing;
  body.ry = Math.sin(Math.sqrt(s) * PI * 2) * 0.2;
  ra.pz = Math.sin(body.ry) * 5; ra.px = -Math.cos(body.ry) * 5;
  la.pz = -Math.sin(body.ry) * 5; la.px = Math.cos(body.ry) * 5;
  ra.ry += body.ry; la.ry += body.ry; la.rx += body.ry;
  if (s > 0) {
    let f = 1 - s; f *= f; f *= f; f = 1 - f;
    const f1 = Math.sin(f * PI);
    const f2 = Math.sin(s * PI) * -(head.rx - 0.7) * 0.75;
    ra.rx -= f1 * 1.2 + f2;
    ra.ry += body.ry * 2;
    ra.rz += Math.sin(s * PI) * -0.4;
  }
  if (p.sneak) {
    body.rx = 0.5;
    ra.rx += 0.4; la.rx += 0.4;
    rl.pz = ll.pz = 4; rl.py = ll.py = 9;
    head.py = 1;
  }
  // idle breathing sway
  const sway = (a: Pose, b: Pose) => {
    a.rz += Math.cos(p.age * 0.09) * 0.05 + 0.05; b.rz -= Math.cos(p.age * 0.09) * 0.05 + 0.05;
    a.rx += Math.sin(p.age * 0.067) * 0.05; b.rx -= Math.sin(p.age * 0.067) * 0.05;
  };
  sway(ra, la);
  if (p.bow) {
    ra.rz = la.rz = 0;
    ra.ry = -0.1 + head.ry; la.ry = 0.1 + head.ry + 0.4;
    ra.rx = la.rx = -PI / 2 + head.rx;
    sway(ra, la);
  }
  if (p.zombie) {
    const f = Math.sin(s * PI), f1 = Math.sin((1 - (1 - s) * (1 - s)) * PI);
    ra.rz = la.rz = 0;
    ra.ry = -(0.1 - f * 0.6); la.ry = 0.1 - f * 0.6;
    ra.rx = la.rx = -PI / 2;
    ra.rx -= f * 1.2 - f1 * 0.4; la.rx -= f * 1.2 - f1 * 0.4;
    sway(ra, la);
  }
  return { head, body, rightArm: ra, leftArm: la, rightLeg: rl, leftLeg: ll };
}

/** UseAction of a held item: swords block, the bow draws, food and potions are eaten/drunk. */
export function useAction(id: number): 'block' | 'bow' | 'eat' | 'none' {
  if (id === 267 || id === 268 || id === 272 || id === 276 || id === 283) return 'block';
  if (id === 261) return 'bow';
  if (FOOD.has(id) || id === 373 || id === 335) return 'eat';
  return 'none';
}
const FOOD = new Set([260, 282, 297, 319, 320, 322, 349, 350, 357, 360, 363, 364, 365, 366, 367, 375, 391, 392, 393, 394, 396, 400, 411, 412, 413, 423, 424]);

/** PlayerEntityRenderer.setModelPose: rightArmPose and aiming of a player with `item`, using it or not. */
export function heldPose(item: number, using: boolean): { held: number; bow: boolean } {
  if (!item) return { held: 0, bow: false };
  const a = using ? useAction(item) : 'none';
  return { held: a === 'block' ? 3 : 1, bow: a === 'bow' };
}

/** Getting on a vehicle: getMountedHeightOffset of the vehicle plus the rider's getYOffset (1.8). */
export function mountOffset(vehicle: string, vehicleHeight: number, riderIsPlayer: boolean): number {
  const v = vehicle === 'Boat' ? -0.3 : vehicle === 'MinecartRideable' || vehicle === 'Minecart' ? 0 : vehicle === 'EntityHorse' ? vehicleHeight * 0.75 - 0.1 : vehicleHeight * 0.75;
  return v + (riderIsPlayer ? -0.35 : 0);
}

// ---- matrices with GlStateManager's semantics (post-multiplied, degrees) ------------------------------------

export function translate(m: Mat4, x: number, y: number, z: number): Mat4 {
  m[12] += m[0] * x + m[4] * y + m[8] * z; m[13] += m[1] * x + m[5] * y + m[9] * z;
  m[14] += m[2] * x + m[6] * y + m[10] * z; m[15] += m[3] * x + m[7] * y + m[11] * z;
  return m;
}
export function rotate(m: Mat4, deg: number, ax: number, ay: number, az: number): Mat4 {
  if (!deg) return m;
  const r = mat4(), c = Math.cos(deg * DEG), s = Math.sin(deg * DEG), t = 1 - c;
  r[0] = t * ax * ax + c; r[1] = t * ax * ay + s * az; r[2] = t * ax * az - s * ay;
  r[4] = t * ax * ay - s * az; r[5] = t * ay * ay + c; r[6] = t * ay * az + s * ax;
  r[8] = t * ax * az + s * ay; r[9] = t * ay * az - s * ax; r[10] = t * az * az + c;
  return multiply(m, m, r);
}
export function scale(m: Mat4, x: number, y: number, z: number): Mat4 {
  for (let i = 0; i < 4; i++) { m[i] *= x; m[4 + i] *= y; m[8 + i] *= z; }
  return m;
}

// ---- first person ---------------------------------------------------------------------------------------------

/** The bot's own per-tick view state: walked distance and bob (PlayerEntity), arm lag (ClientPlayerEntity), the equip dip (HeldItemRenderer). */
export interface FirstPerson {
  acc: number; x: number; y: number; z: number; init: boolean;
  walked: number; prevWalked: number;
  bob: number; prevBob: number; tilt: number; prevTilt: number;
  /** renderPitch / renderYaw, MC degrees. */
  armPitch: number; prevArmPitch: number; armYaw: number; prevArmYaw: number;
  /** equipProgress: 1 shown, dips to 0 and back when the held item changes; `shown` is what the hand holds. */
  equip: number; prevEquip: number; shown: string;
  /** hurtTime of the bot (the camera tilt). */
  hurt: number;
  /** Ticks the held item has been used (eating, blocking, the bow), −1 when not. */
  use: number;
}

export function firstPerson(): FirstPerson {
  return { acc: 0, x: 0, y: 0, z: 0, init: false, walked: 0, prevWalked: 0, bob: 0, prevBob: 0, tilt: 0, prevTilt: 0, armPitch: 0, prevArmPitch: 0, armYaw: 0, prevArmYaw: 0, equip: 1, prevEquip: 1, shown: '', hurt: 0, use: -1 };
}

/** One game tick: pose in wire units (radians, pitch > 0 up), the held stack's key, whether it is in use. */
export function stepFirstPerson(f: FirstPerson, x: number, y: number, z: number, onGround: boolean, yaw: number, pitch: number, held: string, using: boolean): void {
  const mcPitch = -pitch / DEG, mcYaw = -yaw / DEG;
  if (!f.init) { f.init = true; f.x = x; f.y = y; f.z = z; f.armPitch = f.prevArmPitch = mcPitch; f.armYaw = f.prevArmYaw = mcYaw; f.shown = held; }
  const dx = x - f.x, dy = y - f.y, dz = z - f.z;
  f.x = x; f.y = y; f.z = z;
  const moved = Math.sqrt(dx * dx + dz * dz);
  if (moved > 4) return; // a teleport
  f.prevWalked = f.walked; f.walked += moved * 0.6;
  // tickMovement: cameraYaw from the horizontal motion (after ground friction), cameraPitch from falling
  let b = Math.min(0.1, moved * 0.546), t = Math.atan(-dy * 0.2) * 15;
  if (!onGround) b = 0; else t = 0;
  f.prevBob = f.bob; f.bob += (b - f.bob) * 0.4;
  f.prevTilt = f.tilt; f.tilt += (t - f.tilt) * 0.8;
  f.prevArmPitch = f.armPitch; f.prevArmYaw = f.armYaw;
  f.armPitch += (mcPitch - f.armPitch) * 0.5;
  f.armYaw += wrap((mcYaw - f.armYaw) * DEG) / DEG * 0.5;
  // updateHeldItems: down 0.4 a tick while the item differs, the new one comes up from below 0.1
  f.prevEquip = f.equip;
  f.equip += Math.max(-0.4, Math.min(0.4, (held === f.shown ? 1 : 0) - f.equip));
  if (f.equip < 0.1) f.shown = held;
  if (f.hurt > 0) f.hurt--;
  f.use = using ? f.use + 1 : -1;
}

export function advanceFirstPerson(f: FirstPerson, dtMs: number, step: () => void): number {
  f.acc += dtMs;
  if (f.acc > 500) f.acc = 50;
  while (f.acc >= 50) { f.acc -= 50; step(); }
  return f.acc / 50;
}

/** GameRenderer.bobViewWhenHurt + bobView: a view-space matrix to put before the camera's rotation. */
export function bobMatrix(out: Mat4, f: FirstPerson, pt: number, bobbing = true): Mat4 {
  out.set(mat4());
  if (f.hurt > 0) {
    const h = Math.sin(((f.hurt - pt) / HURT_TICKS) ** 4 * PI);
    rotate(out, -h * 14, 0, 0, 1);
  }
  if (!bobbing) return out;
  const d = f.walked - f.prevWalked;
  const w = -(f.walked + d * pt);
  const b = f.prevBob + (f.bob - f.prevBob) * pt, t = f.prevTilt + (f.tilt - f.prevTilt) * pt;
  translate(out, Math.sin(w * PI) * b * 0.5, -Math.abs(Math.cos(w * PI) * b), 0);
  rotate(out, Math.sin(w * PI) * b * 3, 0, 0, 1);
  rotate(out, Math.abs(Math.cos(w * PI - 0.2) * b) * 5, 1, 0, 0);
  rotate(out, t, 1, 0, 0);
  return out;
}

/** applyPlayerRotation: the hand trails the look by half a tick's turn (×0.1 degrees). */
function sway(m: Mat4, f: FirstPerson, pt: number, yaw: number, pitch: number): void {
  const ap = f.prevArmPitch + (f.armPitch - f.prevArmPitch) * pt;
  const ay = f.prevArmYaw + wrap((f.armYaw - f.prevArmYaw) * DEG) / DEG * pt;
  rotate(m, (-pitch / DEG - ap) * 0.1, 1, 0, 0);
  rotate(m, (wrap((-yaw / DEG - ay) * DEG) / DEG) * 0.1, 0, 1, 0);
}

/** 1 − the interpolated equipProgress: how far the hand is down (0 = up). */
const dip = (f: FirstPerson, pt: number) => 1 - (f.prevEquip + (f.equip - f.prevEquip) * pt);

/** applyEquipAndSwingOffset: the item's place right of the crosshair, the dip, and the swing's turn. */
function transformItem(m: Mat4, equip: number, swing: number): void {
  translate(m, 0.56, -0.52, -0.72);
  translate(m, 0, equip * -0.6, 0);
  rotate(m, 45, 0, 1, 0);
  const a = Math.sin(swing * swing * PI), b = Math.sin(Math.sqrt(swing) * PI);
  rotate(m, a * -20, 0, 1, 0);
  rotate(m, b * -20, 0, 0, 1);
  rotate(m, b * -80, 1, 0, 0);
  scale(m, 0.4, 0.4, 0.4);
}

/**
 * The held item in view space (HeldItemRenderer.renderArmHoldingItem), for this renderer's meshes: flat items
 * centred on the origin, blocks standing on y 0. `handheld`: tools and swords (item/handheld's display is the
 * same as item/generated's in first person). `use`: ticks in use (with the partial tick), −1 when not.
 */
export function fpItemMatrix(out: Mat4, f: FirstPerson, pt: number, yaw: number, pitch: number, swing: number, block: boolean, action: 'block' | 'bow' | 'eat' | 'none', use: number): Mat4 {
  out.set(mat4());
  sway(out, f, pt, yaw, pitch);
  const e = dip(f, pt);
  if (use >= 0 && action !== 'none') {
    if (action === 'eat') {
      // applyEatOrDrinkTransformation: up and down to the mouth over 32 ticks
      const left = 32 - use, k = left / 32;
      let bob = Math.abs(Math.cos((left / 4) * PI) * 0.1);
      if (k >= 0.8) bob = 0;
      translate(out, 0, bob, 0);
      const g = 1 - Math.pow(Math.max(0, k), 27);
      translate(out, g * 0.6, g * -0.5, 0);
      rotate(out, g * 90, 0, 1, 0); rotate(out, g * 10, 1, 0, 0); rotate(out, g * 30, 0, 0, 1);
      transformItem(out, e, 0);
    } else if (action === 'block') {
      transformItem(out, e, 0);
      translate(out, -0.5, 0.2, 0); rotate(out, 30, 0, 1, 0); rotate(out, -80, 1, 0, 0); rotate(out, 60, 0, 1, 0);
    } else {
      transformItem(out, e, 0);
      rotate(out, -18, 0, 0, 1); rotate(out, -12, 0, 1, 0); rotate(out, -8, 1, 0, 0);
      translate(out, -0.9, 0.2, 0);
      let k = use / 20; k = Math.min(1, (k * k + k * 2) / 3);
      if (k > 0.1) translate(out, 0, Math.sin((use - 0.1) * 1.3) * (k - 0.1) * 0.01, 0);
      translate(out, 0, 0, k * 0.1);
      scale(out, 1, 1, 1 + k * 0.2);
    }
  } else {
    // translateSwingProgress: the swing's push forward and in
    const r = Math.sqrt(swing) * PI;
    translate(out, -0.4 * Math.sin(r), 0.2 * Math.sin(r * 2), -0.2 * Math.sin(swing * PI));
    transformItem(out, e, swing);
  }
  // ItemRenderer.preRenderGuiItemModel doubles flat (not gui3d) models; the model's firstperson display (item/generated and
  // handheld: rotation 0 −135 25, translation 0 4 2, scale 1.7; blocks: none), then the half scale around the centre.
  // (1.9's firstperson_righthand, scale 0.68 and rotation 0 −90 25, is this chain folded up.)
  if (!block) { scale(out, 2, 2, 2); translate(out, 0, 4 / 16, 2 / 16); rotate(out, -135, 0, 1, 0); rotate(out, 25, 0, 0, 1); scale(out, 1.7, 1.7, 1.7); }
  scale(out, 0.5, 0.5, 0.5);
  if (block) translate(out, 0, -0.5, 0);
  return out;
}

/** renderArm: the bare right arm in view space, in MC model pixels at the arm's pivot (the part's boxes go on top). */
export function fpArmMatrix(out: Mat4, f: FirstPerson, pt: number, yaw: number, pitch: number, swing: number): Mat4 {
  out.set(mat4());
  sway(out, f, pt, yaw, pitch);
  const e = dip(f, pt), r = Math.sqrt(swing) * PI;
  translate(out, -0.3 * Math.sin(r), 0.4 * Math.sin(r * 2), -0.4 * Math.sin(swing * PI));
  translate(out, 0.64000005, -0.6, -0.72);
  translate(out, 0, e * -0.6, 0);
  rotate(out, 45, 0, 1, 0);
  rotate(out, Math.sin(r) * 70, 0, 1, 0);
  rotate(out, Math.sin(swing * swing * PI) * -20, 0, 0, 1);
  translate(out, -1, 3.6, 3.5);
  rotate(out, 120, 0, 0, 1); rotate(out, 200, 1, 0, 0); rotate(out, -135, 0, 1, 0);
  translate(out, 5.6, 0, 0);
  // renderRightArm: ModelPart.render(0.0625) at the arm's rest pose (pivot −5 2 0, z 0.1 from the idle sway at age 0)
  scale(out, 1 / 16, 1 / 16, 1 / 16);
  translate(out, -5, 2, 0);
  rotate(out, 0.1 / DEG, 0, 0, 1);
  return out;
}

// ---- items picked up, XP orbs -----------------------------------------------------------------------------------

/** ItemPickupParticle: 3 ticks from where the item lay to the collector's feet + 0.5, eased in (f²). */
export const PICKUP_TICKS = 3;
export function pickupAt(from: [number, number, number], to: [number, number, number], ticks: number): [number, number, number] {
  let f = Math.min(1, Math.max(0, ticks / PICKUP_TICKS));
  f *= f;
  return [from[0] + (to[0] - from[0]) * f, from[1] + (to[1] + 0.5 - from[1]) * f, from[2] + (to[2] - from[2]) * f];
}

/** ExperienceOrbEntity.getOrbSize: the frame (of 4 × 4 in experience_orb.png) by the orb's value. */
export function orbFrame(xp: number): number {
  const steps = [3, 7, 17, 37, 73, 149, 307, 617, 1237, 2477];
  let i = 0;
  while (i < steps.length && xp >= steps[i]) i++;
  return i;
}

/** ExperienceOrbEntityRenderer's colour: red and blue pulse against full green (renderTicks is the orb's age in ticks). */
export function orbColor(age: number): [number, number, number] {
  const f = age / 2;
  return [(Math.sin(f) + 1) * 0.5, 1, (Math.sin(f + 4.1887903) + 1) * 0.1];
}

// ---- chest lids --------------------------------------------------------------------------------------------------

/** ChestBlockEntity.tick: animationAngle rises 0.1 a tick while someone has it open and falls back after. */
export function stepLid(angle: number, open: boolean): number {
  return Math.max(0, Math.min(1, angle + (open ? 0.1 : -0.1)));
}

/** ChestBlockEntityRenderer: the lid's angle eased out (1 − (1 − a)³) × 90°, at partial tick pt. */
export function lidDegrees(prev: number, angle: number, pt: number): number {
  const a = 1 - (prev + (angle - prev) * pt);
  return (1 - a * a * a) * 90;
}

/**
 * The hinge of a chest lid by the chest's meta (its front: 2 north, 3 south, 4 west, 5 east), block-centred:
 * the back edge at the lid's bottom (y 9/16), the axis, and the turn's sign that lifts the front.
 */
export function lidHinge(meta: number): { x: number; z: number; ax: number; az: number; sign: number } {
  switch (meta & 7) {
    case 3: return { x: 0, z: -7 / 16, ax: 1, az: 0, sign: -1 };
    case 4: return { x: 7 / 16, z: 0, ax: 0, az: 1, sign: -1 };
    case 5: return { x: -7 / 16, z: 0, ax: 0, az: 1, sign: 1 };
    default: return { x: 0, z: 7 / 16, ax: 1, az: 0, sign: 1 };
  }
}
