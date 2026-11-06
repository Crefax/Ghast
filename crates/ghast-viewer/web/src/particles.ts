/// <reference lib="dom" />
/**
 * Particles (1.8 ParticleManager): a fixed pool ticked at 20 Hz like vanilla (drawn between ticks by
 * interpolation), one instanced billboard draw, lit by the world light of the particle's block (FxBox).
 * Motions, lifetimes, sizes and particles.png indices follow the 1.8 Particle classes; block fragments
 * (BlockDustParticle) are a 4×4 px piece of the block's particle texture in the array.
 * Sources: blocks broken and dug at, world_particles and world_event effects, explosions, rain splashes
 * and the randomDisplayTick of blocks near the camera (torches, fire, lava, drips, furnaces, portals, redstone).
 */
import { FX_EVENT, type ExplosionMsg, type ParticleMsg } from './shared/protocol.js';
import { FxBox, LAVA, WATER } from './fxbox.js';
import { compile, uniforms, LIGHT, type Uniforms } from './gl.js';

/** Particle kinds (the Particle subclass they behave like). */
export const K = {
  DIGGING: 1, SMOKE: 2, EXPLODE: 3, FLAME: 4, LAVA: 5, DRIP: 6, RAIN: 7, BUBBLE: 8, CRIT: 9, HEART: 10, AURA: 11, NOTE: 12,
  PORTAL: 13, REDDUST: 14, SPELL: 15, CLOUD: 16, LARGE_EXPLODE: 17, HUGE: 18, ENCHANT: 19, SUSPEND: 20,
} as const;

/** 1.8 ParticleType ids the page spawns. */
export const PT = {
  EXPLODE: 0, LARGE_EXPLODE: 1, HUGE_EXPLOSION: 2, BUBBLE: 4, SPLASH: 5, CRIT: 9, MAGIC_CRIT: 10, SMOKE: 11, LARGE_SMOKE: 12,
  DRIP_WATER: 18, DRIP_LAVA: 19, HAPPY_VILLAGER: 21, PORTAL: 24, FLAME: 26, LAVA: 27, REDDUST: 30, DROPLET: 39, BLOCK_CRACK: 37,
} as const;

export class Particle {
  kind = 0; x = 0; y = 0; z = 0; px = 0; py = 0; pz = 0; vx = 0; vy = 0; vz = 0;
  age = 0; life = 0; scale = 1; base = 1; r = 1; g = 1; b = 1; a = 1; grav = 0;
  /** particles.png index; with layer ≥ 0 the array slice and a 4×4 px piece at (tu, tv) quarters instead. */
  tex = 0; layer = -1; tu = 0; tv = 0;
  ground = false; collide = true; bright = false; light = 0xf0;
  /** Kind-specific: drip bob timer, spell base index, snow shovel gravity. (ox, oy, oz): portal/enchant origin. */
  n = 0; ox = 0; oy = 0; oz = 0; lava = false;
}

export interface FxSources {
  /** The array slice of a block state's particle texture and its colour at a position (null: no particles). */
  block(state: number, x: number, y: number, z: number): { layer: number; color: number } | null;
  /** The array slice of an item's icon (iconcrack, slime, snowball). */
  item(id: number, meta: number): number | null;
}

/** Floats per instance: pos xyz + half size, uv rect, rgba, layer + sky + block light + pad. */
export const INSTANCE = 16;
/** Spawns further from the camera than this are dropped (vanilla: 16 blocks). */
const RANGE = 32;
const TWO_PI = Math.PI * 2;

export class ParticleSystem {
  readonly pool: Particle[] = [];
  count = 0;
  /** 2 all, 1 decreased (a third of the spawns and half the rain skipped, like vanilla's setting), 0 off. */
  level = 2;
  readonly box = new FxBox();
  /** Blocks with a randomDisplayTick near the camera (x, y, z, state)*, from the worker. */
  emitters: Int32Array = new Int32Array(0);
  private cx = 0; private cy = 0; private cz = 0;

  constructor(public limit: number, private readonly src: FxSources, private readonly R: () => number = Math.random) {}

  /** A gaussian (Random.nextGaussian). */
  private gauss(): number {
    const u = this.R() || 1e-9, v = this.R();
    return Math.sqrt(-2 * Math.log(u)) * Math.cos(TWO_PI * v);
  }

  clear(): void { this.count = 0; this.emitters = new Int32Array(0); this.box.clear(); }

  /** Particle(world, x, y, z[, xs, ys, zs]): the defaults, and with `motion` the random push every particle gets. */
  add(kind: number, x: number, y: number, z: number, xs = 0, ys = 0, zs = 0, motion = true): Particle | null {
    if (this.count >= this.limit) return null;
    const R = this.R;
    const p = this.pool[this.count] ?? (this.pool[this.count] = new Particle());
    this.count++;
    p.kind = kind; p.x = p.px = x; p.y = p.py = y; p.z = p.pz = z;
    p.age = 0; p.life = (4 / (R() * 0.9 + 0.1)) | 0; p.scale = p.base = (R() * 0.5 + 0.5) * 2;
    p.r = p.g = p.b = p.a = 1; p.grav = 0; p.tex = 0; p.layer = -1; p.tu = R() * 3; p.tv = R() * 3;
    p.ground = false; p.collide = true; p.bright = false; p.n = 0; p.lava = false;
    if (motion) {
      const mx = xs + (R() * 2 - 1) * 0.4, my = ys + (R() * 2 - 1) * 0.4, mz = zs + (R() * 2 - 1) * 0.4;
      const f = (R() + R() + 1) * 0.15, f1 = Math.sqrt(mx * mx + my * my + mz * mz) || 1;
      p.vx = (mx / f1) * f * 0.4; p.vy = (my / f1) * f * 0.4 + 0.1; p.vz = (mz / f1) * f * 0.4;
    } else { p.vx = 0; p.vy = 0; p.vz = 0; }
    p.light = this.box.lightAt(x, y, z);
    return p;
  }

  /** ClientWorld.addParticle → the 1.8 particle factories. `near`: skip the range check. */
  spawn(id: number, x: number, y: number, z: number, xs = 0, ys = 0, zs = 0, data = 0, near = false): void {
    if (!this.level) return;
    const R = this.R;
    if (!near) {
      const dx = x - this.cx, dy = y - this.cy, dz = z - this.cz;
      if (dx * dx + dy * dy + dz * dz > RANGE * RANGE) return;
      if (this.level === 1 && R() < 1 / 3) return;
    }
    let p: Particle | null;
    switch (id) {
      case 0: // explode
        if (!(p = this.add(K.EXPLODE, x, y, z, xs, ys, zs))) return;
        p.vx = xs + (R() * 2 - 1) * 0.05; p.vy = ys + (R() * 2 - 1) * 0.05; p.vz = zs + (R() * 2 - 1) * 0.05;
        p.r = p.g = p.b = R() * 0.3 + 0.7; p.scale = R() * R() * 6 + 1; p.life = ((16 / (R() * 0.8 + 0.2)) | 0) + 2;
        return;
      case 1: // largeexplode (explosion.png, full bright)
        if (!(p = this.add(K.LARGE_EXPLODE, x, y, z, 0, 0, 0, false))) return;
        p.life = 6 + ((R() * 4) | 0); p.r = p.g = p.b = R() * 0.6 + 0.4; p.base = 2 * (1 - xs * 0.5); p.bright = true; p.collide = false;
        return;
      case 2: // hugeexplosion: an emitter of large ones for 8 ticks
        if (!(p = this.add(K.HUGE, x, y, z, 0, 0, 0, false))) return;
        p.life = 8; p.collide = false;
        return;
      case 3: case 13: case 14: case 15: case 16: case 17: { // fireworksSpark (approximated), spells
        if (!(p = this.add(K.SPELL, x, y, z, xs, ys, zs))) return;
        p.vy *= 0.2;
        if (xs === 0 && zs === 0) { p.vx *= 0.1; p.vz *= 0.1; }
        p.scale *= 0.75; p.life = (8 / (R() * 0.8 + 0.2)) | 0;
        p.n = id === 3 ? 160 : id === 14 || id === 17 ? 144 : 128;
        if (id === 15 || id === 16) { p.r = xs; p.g = ys; p.b = zs; if (id === 16) p.a = 0.15; }
        if (id === 17) { const f = R() * 0.5 + 0.35; p.r = f; p.g = 0; p.b = f; }
        p.tex = p.n + 7;
        return;
      }
      case 4: // bubble
        if (!(p = this.add(K.BUBBLE, x, y, z, xs, ys, zs))) return;
        p.tex = 32; p.scale *= R() * 0.6 + 0.2; p.life = (8 / (R() * 0.8 + 0.2)) | 0;
        p.vx = xs * 0.2 + (R() * 2 - 1) * 0.02; p.vy = ys * 0.2 + (R() * 2 - 1) * 0.02; p.vz = zs * 0.2 + (R() * 2 - 1) * 0.02;
        return;
      case 5: case 6: case 39: // splash, wake, droplet (RainSplashParticle)
        if (!(p = this.add(K.RAIN, x, y, z))) return;
        p.vx *= 0.3; p.vy = R() * 0.2 + 0.1; p.vz *= 0.3;
        p.tex = 19 + ((R() * 4) | 0); p.grav = 0.06; p.life = (8 / (R() * 0.8 + 0.2)) | 0;
        if (id !== 39) {
          p.grav = 0.04; p.tex++;
          if (ys === 0 && (xs !== 0 || zs !== 0)) { p.vx = xs; p.vy = ys + 0.1; p.vz = zs; }
        }
        return;
      case 7: // suspended (in water)
        if (!(p = this.add(K.SUSPEND, x, y - 0.125, z, 0, 0, 0, false))) return;
        p.r = 0.4; p.g = 0.4; p.b = 0.7; p.scale *= R() * 0.6 + 0.2; p.life = (16 / (R() * 0.8 + 0.2)) | 0;
        return;
      case 9: case 10: // crit, magicCrit
        if (!(p = this.add(K.CRIT, x, y, z))) return;
        p.vx = p.vx * 0.1 + xs * 0.4; p.vy = p.vy * 0.1 + ys * 0.4; p.vz = p.vz * 0.1 + zs * 0.4;
        p.r = p.g = p.b = R() * 0.3 + 0.6; p.base = p.scale * 0.75; p.life = (6 / (R() * 0.8 + 0.6)) | 0; p.tex = 65;
        if (id === 10) { p.r *= 0.3; p.g *= 0.8; p.tex = 66; }
        return;
      case 11: case 12: case 32: { // smoke, largesmoke, snowshovel
        const mul = id === 12 ? 2.5 : 1;
        if (!(p = this.add(id === 32 ? K.CLOUD : K.SMOKE, x, y, z))) return;
        p.vx = p.vx * 0.1 + xs; p.vy = p.vy * 0.1 + ys; p.vz = p.vz * 0.1 + zs;
        p.r = p.g = p.b = id === 32 ? 1 - R() * 0.3 : R() * 0.3;
        p.base = p.scale * 0.75 * mul; p.life = (((8 / (R() * 0.8 + 0.2)) | 0) * mul) | 0; p.tex = 7;
        if (id === 32) p.n = 1;
        return;
      }
      case 18: case 19: // dripWater, dripLava
        if (!(p = this.add(K.DRIP, x, y, z, 0, 0, 0, false))) return;
        p.lava = id === 19; p.bright = p.lava;
        p.r = p.lava ? 1 : 0; p.g = 0; p.b = p.lava ? 0 : 1;
        p.tex = 113; p.grav = 0.06; p.n = 40; p.life = (64 / (R() * 0.8 + 0.2)) | 0;
        return;
      case 20: case 34: // angryVillager, heart
        if (!(p = this.add(K.HEART, x, id === 20 ? y + 0.5 : y, z))) return;
        p.vx *= 0.01; p.vy = p.vy * 0.01 + 0.1; p.vz *= 0.01;
        p.base = p.scale * 0.75 * 2; p.life = 16; p.tex = id === 20 ? 81 : 80;
        return;
      case 8: case 21: case 22: { // depthsuspend, happyVillager, townaura (VillageParticle)
        if (!(p = this.add(K.AURA, x, y, z, xs, ys, zs))) return;
        const f = R() * 0.1 + 0.2;
        p.r = p.g = p.b = f; p.scale *= R() * 0.6 + 0.5;
        p.vx *= 0.02; p.vy *= 0.02; p.vz *= 0.02; p.life = (20 / (R() * 0.8 + 0.2)) | 0; p.collide = false;
        if (id === 21) { p.tex = 82; p.r = p.g = p.b = 1; }
        return;
      }
      case 23: // note: xs is the note / 24
        if (!(p = this.add(K.NOTE, x, y, z))) return;
        p.vx *= 0.01; p.vy = p.vy * 0.01 + 0.2; p.vz *= 0.01;
        p.r = Math.sin(xs * TWO_PI) * 0.65 + 0.35; p.g = Math.sin((xs + 1 / 3) * TWO_PI) * 0.65 + 0.35; p.b = Math.sin((xs + 2 / 3) * TWO_PI) * 0.65 + 0.35;
        p.base = p.scale * 0.75 * 2; p.life = 6; p.tex = 64;
        return;
      case 24: { // portal: flies from its origin and back
        if (!(p = this.add(K.PORTAL, x, y, z))) return;
        p.vx = xs; p.vy = ys; p.vz = zs; p.ox = x; p.oy = y; p.oz = z;
        const f = R() * 0.6 + 0.4;
        p.scale = p.base = R() * 0.2 + 0.5; p.r = f * 0.9; p.g = f * 0.3; p.b = f;
        p.life = ((R() * 10) | 0) + 40; p.collide = false; p.tex = (R() * 8) | 0;
        return;
      }
      case 25: { // enchantmenttable: glyphs flying into the table
        if (!(p = this.add(K.ENCHANT, x + xs, y + ys, z + zs, 0, 0, 0, false))) return;
        p.vx = xs; p.vy = ys; p.vz = zs; p.ox = x; p.oy = y; p.oz = z;
        const f = R() * 0.6 + 0.4;
        p.scale = R() * 0.5 + 0.2; p.r = p.g = 0.9 * f; p.b = f;
        p.life = ((R() * 10) | 0) + 30; p.collide = false; p.tex = (R() * 26 + 1 + 224) | 0;
        return;
      }
      case 26: // flame
        if (!(p = this.add(K.FLAME, x, y, z, xs, ys, zs))) return;
        p.vx = p.vx * 0.01 + xs; p.vy = p.vy * 0.01 + ys; p.vz = p.vz * 0.01 + zs;
        p.x = p.px = x + (R() - R()) * 0.05; p.y = p.py = y + (R() - R()) * 0.05; p.z = p.pz = z + (R() - R()) * 0.05;
        p.base = p.scale; p.life = ((8 / (R() * 0.8 + 0.2)) | 0) + 4; p.collide = false; p.tex = 48; p.bright = true;
        return;
      case 27: // lava pop
        if (!(p = this.add(K.LAVA, x, y, z))) return;
        p.vx *= 0.8; p.vy = R() * 0.4 + 0.05; p.vz *= 0.8;
        p.scale *= R() * 2 + 0.2; p.base = p.scale; p.life = (16 / (R() * 0.8 + 0.2)) | 0; p.tex = 49; p.bright = true;
        return;
      case 29: // cloud
        if (!(p = this.add(K.CLOUD, x, y, z))) return;
        p.vx = p.vx * 0.1 + xs; p.vy = p.vy * 0.1 + ys; p.vz = p.vz * 0.1 + zs;
        p.r = p.g = p.b = 1 - R() * 0.3; p.base = p.scale * 0.75 * 2.5; p.life = ((8 / (R() * 0.8 + 0.3)) * 2.5) | 0; p.tex = 7;
        return;
      case 30: { // reddust: the speeds are the colour (red 0 means 1)
        if (!(p = this.add(K.REDDUST, x, y, z))) return;
        p.vx *= 0.1; p.vy *= 0.1; p.vz *= 0.1;
        const f = R() * 0.4 + 0.6;
        p.r = (R() * 0.2 + 0.8) * (xs || 1) * f; p.g = (R() * 0.2 + 0.8) * ys * f; p.b = (R() * 0.2 + 0.8) * zs * f;
        p.base = p.scale * 0.75; p.life = (8 / (R() * 0.8 + 0.2)) | 0; p.tex = 7;
        return;
      }
      case 31: case 33: case 36: { // snowballpoof, slime, iconcrack (SnowballParticle): a piece of the item's icon
        const layer = this.src.item(id === 31 ? 332 : id === 33 ? 341 : data >> 4, id === 36 ? data & 15 : 0);
        if (layer === null || !(p = this.add(K.DIGGING, x, y, z))) return;
        if (id === 36) { p.vx = p.vx * 0.1 + xs; p.vy = p.vy * 0.1 + ys; p.vz = p.vz * 0.1 + zs; }
        p.layer = layer; p.grav = 1; p.scale /= 2;
        return;
      }
      case 37: case 38: { // blockcrack, blockdust (BlockDustParticle / BlockParticle)
        const t = this.src.block(data, Math.floor(x), Math.floor(y), Math.floor(z));
        if (!t || !(p = this.add(K.DIGGING, x, y, z, xs, ys, zs))) return;
        if (id === 38) { p.vx = xs; p.vy = ys; p.vz = zs; }
        p.layer = t.layer; p.grav = 1; p.scale /= 2;
        p.r = 0.6 * ((t.color >> 16) & 255) / 255; p.g = 0.6 * ((t.color >> 8) & 255) / 255; p.b = 0.6 * (t.color & 255) / 255;
        return;
      }
    }
  }

  /** ClientPlayNetworkHandler.onParticle (count 0: one particle with speed × offset as its motion) and the world_event effects. */
  fromServer(m: ParticleMsg): void {
    if (m.id >= FX_EVENT) { this.event(m.id - FX_EVENT, Math.floor(m.x), Math.floor(m.y), Math.floor(m.z), m.data); return; }
    if (m.count === 0) { this.spawn(m.id, m.x, m.y, m.z, m.speed * m.ox, m.speed * m.oy, m.speed * m.oz, m.data); return; }
    const n = Math.min(m.count, 256);
    for (let i = 0; i < n; i++) {
      const g = () => this.gauss();
      this.spawn(m.id, m.x + g() * m.ox, m.y + g() * m.oy, m.z + g() * m.oz, g() * m.speed, g() * m.speed, g() * m.speed, m.data);
    }
  }

  /** WorldRenderer.processWorldEvent: 2000 dispenser smoke (data: direction), 2004 spawner flames, 2005 bonemeal (data: count). */
  private event(e: number, x: number, y: number, z: number, data: number): void {
    const R = this.R;
    if (e === 0) {
      const i = (data % 3) - 1, j = (((data / 3) | 0) % 3) - 1;
      const cx = x + i * 0.6 + 0.5, cy = y + 0.5, cz = z + j * 0.6 + 0.5;
      for (let k = 0; k < 10; k++) {
        const s = R() * 0.2 + 0.01;
        this.spawn(PT.SMOKE, cx + i * 0.01 + (R() - 0.5) * j * 0.5, cy + (R() - 0.5) * 0.5, cz + j * 0.01 + (R() - 0.5) * i * 0.5,
          i * s + this.gauss() * 0.01, -0.03 + this.gauss() * 0.01, j * s + this.gauss() * 0.01);
      }
    } else if (e === 4) {
      for (let k = 0; k < 20; k++) {
        const px = x + 0.5 + (R() - 0.5) * 2, py = y + 0.5 + (R() - 0.5) * 2, pz = z + 0.5 + (R() - 0.5) * 2;
        this.spawn(PT.SMOKE, px, py, pz); this.spawn(PT.FLAME, px, py, pz);
      }
    } else if (e === 5) {
      for (let k = 0, n = data || 15; k < n; k++) {
        this.spawn(PT.HAPPY_VILLAGER, x + R(), y + R(), z + R(), this.gauss() * 0.02, this.gauss() * 0.02, this.gauss() * 0.02);
      }
    }
  }

  /** ParticleManager.addBlockBreakParticles: 4×4×4 fragments pushed out from the centre. */
  breakBlock(x: number, y: number, z: number, state: number): void {
    if (!this.level) return;
    const t = this.src.block(state, x, y, z);
    if (!t) return;
    // the block is air now, whatever the box still says (the worker resends it shortly)
    this.box.setShape(x, y, z, 0);
    for (let j = 0; j < 4; j++) for (let k = 0; k < 4; k++) for (let l = 0; l < 4; l++) {
      const dx = (j + 0.5) / 4, dy = (k + 0.5) / 4, dz = (l + 0.5) / 4;
      this.digging(t, x + dx, y + dy, z + dz, dx - 0.5, dy - 0.5, dz - 0.5, 1, 1);
    }
  }

  /** ParticleManager.addBlockBreakingParticles: one small, slow fragment on the face being dug (0 down, 1 up, 2 north, 3 south, 4 west, 5 east). */
  hitBlock(x: number, y: number, z: number, state: number, face: number): void {
    if (!this.level) return;
    const t = this.src.block(state, x, y, z);
    if (!t) return;
    const R = this.R, f = 0.1, s = this.box.shapeAt(x, y, z), top = s > 0 && s < 16 ? s / 16 : 1;
    let px = x + R() * (1 - f * 2) + f, py = y + R() * (top - f * 2) + f, pz = z + R() * (1 - f * 2) + f;
    if (face === 0) py = y - f; else if (face === 1) py = y + top + f;
    else if (face === 2) pz = z - f; else if (face === 3) pz = z + 1 + f;
    else if (face === 4) px = x - f; else px = x + 1 + f;
    this.digging(t, px, py, pz, 0, 0, 0, 0.2, 0.6);
  }

  private digging(t: { layer: number; color: number }, x: number, y: number, z: number, xs: number, ys: number, zs: number, speed: number, size: number): void {
    const p = this.add(K.DIGGING, x, y, z, xs, ys, zs);
    if (!p) return;
    p.layer = t.layer; p.grav = 1; p.scale = (p.scale / 2) * size;
    p.r = 0.6 * ((t.color >> 16) & 255) / 255; p.g = 0.6 * ((t.color >> 8) & 255) / 255; p.b = 0.6 * (t.color & 255) / 255;
    if (speed !== 1) { p.vx *= speed; p.vy = (p.vy - 0.1) * speed + 0.1; p.vz *= speed; }
  }

  /** Explosion.affectWorld on the client: a large (or huge) explosion, then a puff and smoke from every affected block. */
  explosion(e: ExplosionMsg): void {
    const R = this.R;
    this.spawn(e.radius >= 2 ? PT.HUGE_EXPLOSION : PT.LARGE_EXPLODE, e.x, e.y, e.z, 1, 0, 0, 0, true);
    const bx = Math.floor(e.x), by = Math.floor(e.y), bz = Math.floor(e.z);
    for (const [ox, oy, oz] of e.blocks.slice(0, 300)) {
      const x = bx + ox + R(), y = by + oy + R(), z = bz + oz + R();
      let dx = x - e.x, dy = y - e.y, dz = z - e.z;
      const d = Math.sqrt(dx * dx + dy * dy + dz * dz) || 1;
      dx /= d; dy /= d; dz /= d;
      const v = (0.5 / (d / e.radius + 0.1)) * (R() * R() + 0.3);
      dx *= v; dy *= v; dz *= v;
      this.spawn(PT.EXPLODE, (x + e.x) / 2, (y + e.y) / 2, (z + e.z) / 2, dx, dy, dz);
      this.spawn(PT.SMOKE, x, y, z, dx, dy, dz);
    }
  }

  /**
   * GameRenderer.tickRainSplashing: up to 100 · rain² drops a tick on the surfaces within 10 blocks
   * (smoke on lava). `top(x, z)`: the precipitation height where it rains there, else −1.
   */
  rain(strength: number, top: (x: number, z: number) => number): void {
    if (!this.level || strength <= 0) return;
    const R = this.R;
    let n = (100 * strength * strength) | 0;
    if (this.level === 1) n >>= 1;
    const bx = Math.floor(this.cx), by = Math.floor(this.cy), bz = Math.floor(this.cz);
    for (let i = 0; i < n; i++) {
      const x = bx + ((R() * 10) | 0) - ((R() * 10) | 0), z = bz + ((R() * 10) | 0) - ((R() * 10) | 0);
      const t = top(x, z);
      if (t <= 0 || t > by + 10 || t < by - 10) continue;
      const f1 = R(), f2 = R(), s = this.box.shapeAt(x, t - 1, z);
      if (s === LAVA) this.spawn(PT.SMOKE, x + f1, t + 0.1, z + f2, 0, 0, 0, 0, true);
      else this.spawn(PT.DROPLET, x + f1, t - 1 + 0.1 + (s > 0 && s < 16 ? s / 16 : 1), z + f2, 0, 0, 0, 0, true);
    }
  }

  /**
   * ClientWorld.spawnRandomParticles: 1000 random blocks a tick at offsets nextInt(16) − nextInt(16) get a
   * randomDisplayTick. Here every listed emitter gets its expected share, 1000 · Π (16 − |d|) / 256.
   */
  ambient(): void {
    if (!this.level) return;
    const e = this.emitters, R = this.R;
    const bx = Math.floor(this.cx), by = Math.floor(this.cy), bz = Math.floor(this.cz);
    for (let i = 0; i < e.length; i += 4) {
      const x = e[i], y = e[i + 1], z = e[i + 2], st = e[i + 3];
      const dx = Math.abs(x - bx), dy = Math.abs(y - by), dz = Math.abs(z - bz);
      if (dx > 15 || dy > 15 || dz > 15) continue;
      // a mob spawner's flames come from its tile entity every tick while a player is near
      if (st >> 4 === 52) { this.display(x, y, z, st); continue; }
      const want = (1000 * (16 - dx) * (16 - dy) * (16 - dz)) / 16777216;
      for (let calls = Math.floor(want) + (R() < want % 1 ? 1 : 0); calls > 0; calls--) this.display(x, y, z, st);
    }
  }

  /** Block.randomDisplayTick of the blocks the worker lists (fxbox.ts EMITTERS). */
  display(x: number, y: number, z: number, state: number): void {
    const R = this.R, id = state >> 4, meta = state & 15;
    switch (id) {
      case 50: case 76: { // torch (smoke + flame), lit redstone torch (red dust); meta 1..4 on a wall: east, west, south, north
        const ox = meta === 1 ? -1 : meta === 2 ? 1 : 0, oz = meta === 3 ? -1 : meta === 4 ? 1 : 0, wall = ox || oz;
        if (id === 50) {
          const px = x + 0.5 + 0.27 * ox, py = y + 0.7 + (wall ? 0.22 : 0), pz = z + 0.5 + 0.27 * oz;
          this.spawn(PT.SMOKE, px, py, pz); this.spawn(PT.FLAME, px, py, pz);
        } else {
          const px = x + 0.5 + (R() - 0.5) * 0.2 + 0.27 * ox, py = y + 0.7 + (R() - 0.5) * 0.2 + (wall ? 0.22 : 0), pz = z + 0.5 + (R() - 0.5) * 0.2 + 0.27 * oz;
          this.spawn(PT.REDDUST, px, py, pz);
        }
        return;
      }
      case 51: // fire on a block
        for (let i = 0; i < 3; i++) this.spawn(PT.LARGE_SMOKE, x + R(), y + R() * 0.5 + 0.5, z + R());
        return;
      case 8: case 9: case 10: case 11: { // liquids: lava pops on the surface, drips under a ceiling
        const lava = id >= 10;
        if (lava && this.box.shapeAt(x, y + 1, z) === 0 && R() < 0.01) this.spawn(PT.LAVA, x + R(), y + 1, z + R());
        const below = this.box.shapeAt(x, y - 1, z), below2 = this.box.shapeAt(x, y - 2, z);
        if (R() < 0.1 && below === 16 && below2 < 16) this.spawn(lava ? PT.DRIP_LAVA : PT.DRIP_WATER, x + R(), y - 1.05, z + R());
        return;
      }
      case 62: { // lit furnace: smoke and flame in front (meta 2 north, 3 south, 4 west, 5 east)
        const py = y + (R() * 6) / 16, d = R() * 0.6 - 0.3;
        const [px, pz] = meta === 4 ? [x + 0.5 - 0.52, z + 0.5 + d] : meta === 5 ? [x + 0.5 + 0.52, z + 0.5 + d] : meta === 2 ? [x + 0.5 + d, z + 0.5 - 0.52] : [x + 0.5 + d, z + 0.5 + 0.52];
        this.spawn(PT.SMOKE, px, py, pz); this.spawn(PT.FLAME, px, py, pz);
        return;
      }
      case 90: // nether portal (meta 1: along x, else along z)
        for (let i = 0; i < 4; i++) {
          let px = x + R(), pz = z + R(), vx = (R() - 0.5) * 0.5, vz = (R() - 0.5) * 0.5;
          const py = y + R(), vy = (R() - 0.5) * 0.5, j = ((R() * 2) | 0) * 2 - 1;
          if (meta === 2) { px = x + 0.5 + 0.25 * j; vx = R() * 2 * j; } else { pz = z + 0.5 + 0.25 * j; vz = R() * 2 * j; }
          this.spawn(PT.PORTAL, px, py, pz, vx, vy, vz);
        }
        return;
      case 130: // ender chest
        for (let i = 0; i < 3; i++) {
          const j = ((R() * 2) | 0) * 2 - 1, k = ((R() * 2) | 0) * 2 - 1;
          this.spawn(PT.PORTAL, x + 0.5 + 0.25 * j, y + R(), z + 0.5 + 0.25 * k, R() * j, (R() - 0.5) * 0.125, R() * k);
        }
        return;
      case 74: { // lit redstone ore: sparkles on the faces not covered
        const d = 0.0625;
        for (let i = 0; i < 6; i++) {
          let px = x + R(), py = y + R(), pz = z + R();
          if (i === 0 && this.box.shapeAt(x, y + 1, z) !== 16) py = y + 1 + d;
          if (i === 1 && this.box.shapeAt(x, y - 1, z) !== 16) py = y - d;
          if (i === 2 && this.box.shapeAt(x, y, z + 1) !== 16) pz = z + 1 + d;
          if (i === 3 && this.box.shapeAt(x, y, z - 1) !== 16) pz = z - d;
          if (i === 4 && this.box.shapeAt(x + 1, y, z) !== 16) px = x + 1 + d;
          if (i === 5 && this.box.shapeAt(x - 1, y, z) !== 16) px = x - d;
          if (px < x || px > x + 1 || py < y || py > y + 1 || pz < z || pz > z + 1) this.spawn(PT.REDDUST, px, py, pz);
        }
        return;
      }
      case 55: { // powered redstone wire: red dust in the power's colour
        const f = meta / 15;
        this.spawn(PT.REDDUST, x + 0.5 + (R() - 0.5) * 0.2, y + 0.0625, z + 0.5 + (R() - 0.5) * 0.2, f * 0.6 + 0.4, Math.max(0, f * f * 0.7 - 0.5), Math.max(0, f * f * 0.6 - 0.7));
        return;
      }
      case 52: { // mob spawner
        const px = x + R(), py = y + R(), pz = z + R();
        this.spawn(PT.SMOKE, px, py, pz); this.spawn(PT.FLAME, px, py, pz);
      }
    }
  }

  /** The camera (spawns further than RANGE are dropped; rain and display ticks centre on it). */
  setCamera(x: number, y: number, z: number): void { this.cx = x; this.cy = y; this.cz = z; }

  /** One 20 Hz tick of every particle. */
  tick(): void {
    for (let i = 0; i < this.count;) {
      const p = this.pool[i];
      if (this.update(p)) { i++; continue; }
      // swap the last one in (the pool keeps its objects)
      const last = this.count - 1;
      this.pool[i] = this.pool[last]; this.pool[last] = p;
      this.count = last;
    }
  }

  private update(p: Particle): boolean {
    p.px = p.x; p.py = p.y; p.pz = p.z;
    const R = this.R;
    switch (p.kind) {
      case K.DIGGING:
        if (p.age++ >= p.life) return false;
        p.vy -= 0.04 * p.grav;
        this.move(p); this.drag(p, 0.98);
        break;
      case K.SMOKE: case K.REDDUST: case K.SPELL: case K.CLOUD:
        if (p.age++ >= p.life) return false;
        p.tex = (p.kind === K.SPELL ? p.n : 0) + 7 - ((p.age * 8) / p.life | 0);
        if (p.kind === K.SMOKE || p.kind === K.SPELL) p.vy += 0.004;
        if (p.kind === K.CLOUD && p.n) p.vy -= 0.03;
        this.move(p);
        if (p.kind !== K.CLOUD && p.y === p.py) { p.vx *= 1.1; p.vz *= 1.1; }
        this.drag(p, 0.96);
        break;
      case K.EXPLODE:
        if (p.age++ >= p.life) return false;
        p.tex = 7 - ((p.age * 8) / p.life | 0);
        p.vy += 0.004;
        this.move(p); this.drag(p, 0.9);
        break;
      case K.FLAME:
        if (p.age++ >= p.life) return false;
        this.move(p); this.drag(p, 0.96);
        break;
      case K.LAVA:
        if (p.age++ >= p.life) return false;
        if (R() > p.age / p.life) this.spawn(PT.SMOKE, p.x, p.y, p.z, p.vx, p.vy, p.vz, 0, true);
        p.vy -= 0.03;
        this.move(p); this.drag(p, 0.999);
        break;
      case K.DRIP:
        if (p.lava) { p.g = 16 / (40 - p.n + 16); p.b = 4 / (40 - p.n + 8); } else { p.r = 0.2; p.g = 0.3; p.b = 1; }
        p.vy -= p.grav;
        if (p.n-- > 0) { p.vx *= 0.02; p.vy *= 0.02; p.vz *= 0.02; p.tex = 113; } else p.tex = 112;
        this.move(p); this.drag(p, 0.98, false);
        if (p.life-- <= 0) return false;
        if (p.ground) {
          if (!p.lava) { this.spawn(PT.SPLASH, p.x, p.y, p.z, 0, 0, 0, 0, true); return false; }
          p.tex = 114; p.vx *= 0.7; p.vz *= 0.7;
        }
        if (this.inLiquid(p)) return false;
        break;
      case K.RAIN:
        p.vy -= p.grav;
        this.move(p); this.drag(p, 0.98, false);
        if (p.life-- <= 0) return false;
        if (p.ground) { if (R() < 0.5) return false; p.vx *= 0.7; p.vz *= 0.7; }
        if (this.inLiquid(p)) return false;
        break;
      case K.BUBBLE:
        p.vy += 0.002;
        this.move(p); this.drag(p, 0.85, false);
        if (this.box.shapeAt(p.x, p.y, p.z) !== WATER) return false;
        if (p.life-- <= 0) return false;
        break;
      case K.SUSPEND:
        if (this.box.shapeAt(p.x, p.y, p.z) !== WATER) return false;
        if (p.life-- <= 0) return false;
        break;
      case K.CRIT:
        if (p.age++ >= p.life) return false;
        this.move(p);
        p.g *= 0.96; p.b *= 0.9;
        p.vx *= 0.7; p.vy *= 0.7; p.vz *= 0.7; p.vy -= 0.02;
        if (p.ground) { p.vx *= 0.7; p.vz *= 0.7; }
        break;
      case K.HEART: case K.NOTE:
        if (p.age++ >= p.life) return false;
        this.move(p);
        if (p.y === p.py) { p.vx *= 1.1; p.vz *= 1.1; }
        this.drag(p, p.kind === K.HEART ? 0.86 : 0.66);
        break;
      case K.AURA:
        this.move(p); this.drag(p, 0.99, false);
        if (p.life-- <= 0) return false;
        break;
      case K.PORTAL: {
        let f = p.age / p.life;
        f = 1 - (-f + f * f * 2);
        p.x = p.ox + p.vx * f; p.y = p.oy + p.vy * f + (1 - f); p.z = p.oz + p.vz * f;
        if (p.age++ >= p.life) return false;
        break;
      }
      case K.ENCHANT: {
        const f = 1 - p.age / p.life;
        let f1 = 1 - f; f1 *= f1; f1 *= f1;
        p.x = p.ox + p.vx * f; p.y = p.oy + p.vy * f - f1 * 1.2; p.z = p.oz + p.vz * f;
        if (p.age++ >= p.life) return false;
        break;
      }
      case K.LARGE_EXPLODE:
        if (++p.age >= p.life) return false;
        break;
      case K.HUGE:
        for (let i = 0; i < 6; i++) {
          this.spawn(PT.LARGE_EXPLODE, p.x + (R() - R()) * 4, p.y + (R() - R()) * 4, p.z + (R() - R()) * 4, p.age / p.life, 0, 0, 0, true);
        }
        if (++p.age >= p.life) return false;
        break;
      default: return false;
    }
    if (!p.bright) p.light = this.box.lightAt(p.x, p.y, p.z);
    return true;
  }

  /** motion *= k; on the ground x and z lose another 30 % (Particle.tick). */
  private drag(p: Particle, k: number, ground = true): void {
    p.vx *= k; p.vy *= k; p.vz *= k;
    if (ground && p.ground) { p.vx *= 0.7; p.vz *= 0.7; }
  }

  private inLiquid(p: Particle): boolean {
    const s = this.box.shapeAt(p.x, p.y, p.z);
    return s === WATER || s === LAVA;
  }

  /** Entity.move against the box's shapes: y, then x, then z; a blocked axis loses its motion. */
  private move(p: Particle): void {
    const box = this.box;
    p.ground = false;
    if (!p.collide || !box.shape) { p.x += p.vx; p.y += p.vy; p.z += p.vz; return; }
    let ny = p.y + p.vy;
    if (p.vy < 0) {
      const s = box.shapeAt(p.x, ny, p.z);
      if (s > 0 && s <= 16 && ny - Math.floor(ny) < s / 16) { ny = Math.floor(ny) + s / 16; p.ground = true; p.vy = 0; }
    } else if (p.vy > 0 && box.blocked(p.x, ny + 0.02, p.z)) { ny = p.y; p.vy = 0; }
    p.y = ny;
    const nx = p.x + p.vx;
    if (box.blocked(nx, p.y + 0.01, p.z)) p.vx = 0; else p.x = nx;
    const nz = p.z + p.vz;
    if (box.blocked(p.x, p.y + 0.01, nz)) p.vz = 0; else p.z = nz;
  }

  /** Instances (camera-relative, interpolated by `pt` 0..1 between ticks); returns how many. */
  fill(out: Float32Array, camX: number, camY: number, camZ: number, pt: number): number {
    let n = 0;
    const max = Math.floor(out.length / INSTANCE);
    for (let i = 0; i < this.count && n < max; i++) {
      const p = this.pool[i];
      const t = (p.age + pt) / Math.max(1, p.life);
      let half: number, u0 = 0, v0 = 0, du = 0, layer = p.layer;
      switch (p.kind) {
        case K.HUGE: continue;
        case K.SMOKE: case K.CLOUD: case K.CRIT: case K.HEART: case K.NOTE: case K.REDDUST: half = 0.1 * p.base * Math.min(1, Math.max(0, t * 32)); break;
        case K.FLAME: half = 0.1 * p.base * (1 - t * t * 0.5); break;
        case K.LAVA: half = 0.1 * p.base * (1 - t * t); break;
        case K.PORTAL: { const f = 1 - t; half = 0.1 * p.base * (1 - f * f); break; }
        case K.LARGE_EXPLODE: {
          const frame = ((p.age + pt) * 15 / p.life) | 0;
          if (frame > 15) continue;
          half = p.base; layer = -2;
          u0 = (frame & 3) / 4; v0 = (frame >> 2) / 4; du = 0.25;
          break;
        }
        default: half = 0.1 * p.scale;
      }
      if (layer >= 0) { u0 = p.tu / 4; v0 = p.tv / 4; du = 0.25; }
      else if (layer === -1) { u0 = (p.tex & 15) / 16; v0 = (p.tex >> 4) / 16; du = 0.0624375; }
      const o = n * INSTANCE;
      out[o] = p.px + (p.x - p.px) * pt - camX; out[o + 1] = p.py + (p.y - p.py) * pt - camY; out[o + 2] = p.pz + (p.z - p.pz) * pt - camZ; out[o + 3] = half;
      out[o + 4] = u0; out[o + 5] = v0; out[o + 6] = du; out[o + 7] = du;
      out[o + 8] = p.r; out[o + 9] = p.g; out[o + 10] = p.b; out[o + 11] = p.a;
      out[o + 12] = layer; out[o + 13] = p.bright ? 1 : (p.light >> 4) / 15; out[o + 14] = p.bright ? 1 : (p.light & 15) / 15; out[o + 15] = 0;
      n++;
    }
    return n;
  }
}

// ---- drawing ---------------------------------------------------------------------------------------

const VS = `#version 300 es
precision highp float;
layout(location=0) in vec2 a_corner;
layout(location=1) in vec4 a_posSize;
layout(location=2) in vec4 a_uv;
layout(location=3) in vec4 a_color;
layout(location=4) in vec4 a_layerLight;
uniform mat4 u_viewProj;
uniform vec3 u_right;
uniform vec3 u_up;
out vec2 v_uv;
flat out float v_layer;
out vec4 v_color;
out vec2 v_light;
out float v_dist;
void main() {
  vec3 p = a_posSize.xyz + (u_right * a_corner.x + u_up * a_corner.y) * a_posSize.w;
  gl_Position = u_viewProj * vec4(p, 1.0);
  v_uv = a_uv.xy + vec2(a_corner.x * 0.5 + 0.5, 0.5 - a_corner.y * 0.5) * a_uv.zw;
  v_layer = a_layerLight.x;
  v_color = a_color;
  v_light = a_layerLight.yz;
  v_dist = length(p);
}`;

const FS = `#version 300 es
precision highp float;
precision highp sampler2DArray;
uniform sampler2D u_parts;
uniform sampler2D u_expl;
uniform sampler2DArray u_arr;
uniform vec3 u_fogColor;
uniform vec2 u_fog;
in vec2 v_uv;
flat in float v_layer;
in vec4 v_color;
in vec2 v_light;
in float v_dist;
out vec4 o;
${LIGHT}
void main() {
  vec4 t = v_layer < -1.5 ? texture(u_expl, v_uv) : v_layer < -0.5 ? texture(u_parts, v_uv) : texture(u_arr, vec3(v_uv, v_layer));
  if (t.a * v_color.a < 0.05) discard;
  vec3 c = t.rgb * v_color.rgb * lightOf(v_light.x, v_light.y);
  float f = clamp((v_dist - u_fog.x) / (u_fog.y - u_fog.x), 0.0, 1.0);
  o = vec4(mix(c, u_fogColor, f), t.a * v_color.a);
}`;

/** The pool as one instanced draw: particles.png, explosion.png and block pieces from the texture array. */
export class ParticleRenderer {
  private readonly prog: WebGLProgram;
  private readonly u: Uniforms;
  private readonly vao: WebGLVertexArrayObject;
  private readonly inst: WebGLBuffer;
  private data: Float32Array;
  private capacity = 0;
  gamma = 0;

  constructor(private readonly gl: WebGL2RenderingContext, private readonly array: WebGLTexture, private readonly parts: WebGLTexture | null, private readonly expl: WebGLTexture | null) {
    this.prog = compile(gl, VS, FS);
    this.u = uniforms(gl, this.prog, ['u_viewProj', 'u_right', 'u_up', 'u_parts', 'u_expl', 'u_arr', 'u_fogColor', 'u_fog', 'u_daylight', 'u_gamma']);
    this.vao = gl.createVertexArray()!;
    gl.bindVertexArray(this.vao);
    const corners = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, corners);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    this.inst = gl.createBuffer()!;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.inst);
    for (let a = 0; a < 4; a++) {
      gl.enableVertexAttribArray(1 + a);
      gl.vertexAttribPointer(1 + a, 4, gl.FLOAT, false, INSTANCE * 4, a * 16);
      gl.vertexAttribDivisor(1 + a, 1);
    }
    gl.bindVertexArray(null);
    this.data = new Float32Array(0);
  }

  /** Draws the pool; `view` is the camera rotation (rows: right, up). Blended, depth tested and written. */
  draw(sys: ParticleSystem, viewProj: Float32Array, view: Float32Array, cam: { x: number; y: number; z: number }, pt: number,
    fogColor: [number, number, number], fog: [number, number], daylight: number): number {
    if (!sys.count || !this.parts) return 0;
    const gl = this.gl;
    if (this.data.length < sys.count * INSTANCE) this.data = new Float32Array(Math.max(256, sys.limit) * INSTANCE);
    const n = sys.fill(this.data, cam.x, cam.y, cam.z, pt);
    if (!n) return 0;
    gl.useProgram(this.prog);
    gl.uniformMatrix4fv(this.u.u_viewProj, false, viewProj);
    gl.uniform3f(this.u.u_right, view[0], view[4], view[8]);
    gl.uniform3f(this.u.u_up, view[1], view[5], view[9]);
    gl.uniform3f(this.u.u_fogColor, fogColor[0], fogColor[1], fogColor[2]);
    gl.uniform2f(this.u.u_fog, fog[0], fog[1]);
    gl.uniform1f(this.u.u_daylight, daylight);
    gl.uniform1f(this.u.u_gamma, this.gamma);
    gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, this.parts);
    gl.activeTexture(gl.TEXTURE2); gl.bindTexture(gl.TEXTURE_2D, this.expl ?? this.parts);
    gl.activeTexture(gl.TEXTURE1); gl.bindTexture(gl.TEXTURE_2D_ARRAY, this.array);
    gl.uniform1i(this.u.u_parts, 0); gl.uniform1i(this.u.u_arr, 1); gl.uniform1i(this.u.u_expl, 2);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.inst);
    if (this.capacity < this.data.length) { gl.bufferData(gl.ARRAY_BUFFER, this.data.byteLength, gl.DYNAMIC_DRAW); this.capacity = this.data.length; }
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, this.data, 0, n * INSTANCE);
    gl.disable(gl.CULL_FACE);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.bindVertexArray(this.vao);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, n);
    gl.bindVertexArray(null);
    gl.disable(gl.BLEND);
    gl.activeTexture(gl.TEXTURE0);
    return n;
  }
}
