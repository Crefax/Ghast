/// <reference lib="dom" />
/**
 * Entities: players (and zombies/skeletons/pigmen) as Minecraft's biped with their own skins (skins.ts; slim arms, capes), pigs/cows/sheep
 * and creepers with their own textures, the other mobs from mobs.ts and armour stands, minecarts and boats
 * from objects.ts, armour over bipeds (armour.ts), babies; anything else as a box of its size; dropped items as spinning
 * sprites (blocks as small cubes); nametags; the held item. ModelBox / TexturedQuad are reproduced
 * from 1.8 so the skins map exactly; parts are drawn with MC's transform chain (rotation point, then
 * Z/Y/X) under a matrix that converts MC model space (pixels, y down, x mirrored) to blocks.
 */
import { compile, uniforms, MODEL_FS, MODEL_VS, type Uniforms } from './gl.js';
import { mat4, multiply, modelMatrix, type Mat4 } from './math.js';
import { texture2d } from './textures.js';
import type { Baked } from './bake.js';
import { SkinTextures } from './skins.js';
import { BANNER, BANNER_PATTERNS, FLEECE, LIVING_BABY, LIVING_SCALE, MOBS, SHEEP_FUR, SKULLS, babyGroup, bannerPlace, dyeColor, posesOf, skullPlace, type Baby, type MobDef } from './mobs.js';
import { ARMOUR_PARTS, LEATHER_DEFAULT, WEARS_ARMOUR, armourParts, armourPiece, armourTexture, rgb } from './armour.js';
import { OBJECT_MODELS } from './objects.js';
import { HURT_OVERLAY, bipedPose, deathAngle, heldPose, rotate, translate, type BipedPart } from './anim.js';

const FLOATS = 7; // pos xyz, uv + layer, shade

/** A MC ModelBox as quads in the part's own space (pixels). */
function box(out: number[], texU: number, texV: number, x: number, y: number, z: number, w: number, h: number, d: number, inflate: number, tw: number, th: number, mirror = false): void {
  let x0 = x - inflate, y0 = y - inflate, z0 = z - inflate;
  let x1 = x + w + inflate;
  const y1 = y + h + inflate, z1 = z + d + inflate;
  if (mirror) { const t = x1; x1 = x0; x0 = t; }
  const v7 = [x0, y0, z0], v = [x1, y0, z0], v1 = [x1, y1, z0], v2 = [x0, y1, z0];
  const v3 = [x0, y0, z1], v4 = [x1, y0, z1], v5 = [x1, y1, z1], v6 = [x0, y1, z1];
  const quads: Array<[number[][], number, number, number, number, number]> = [
    [[v4, v, v1, v5], texU + d + w, texV + d, texU + d + w + d, texV + d + h, 0.6],
    [[v7, v3, v6, v2], texU, texV + d, texU + d, texV + d + h, 0.6],
    [[v4, v3, v7, v], texU + d, texV, texU + d + w, texV + d, 1.0],
    [[v1, v2, v6, v5], texU + d + w, texV + d, texU + d + w + w, texV, 0.5],
    [[v, v7, v2, v1], texU + d, texV + d, texU + d + w, texV + d + h, 0.8],
    [[v3, v4, v5, v6], texU + d + w + d, texV + d, texU + d + w + d + w, texV + d + h, 0.8],
  ];
  for (const [vs, u1, vv1, u2, vv2, shade] of quads) {
    const uv = [[u2, vv1], [u1, vv1], [u1, vv2], [u2, vv2]];
    const order = mirror ? [3, 2, 1, 0] : [0, 1, 2, 3];
    const tri = [0, 1, 2, 0, 2, 3];
    for (const t of tri) {
      const k = order[t];
      out.push(vs[k][0], vs[k][1], vs[k][2], uv[k][0] / tw, uv[k][1] / th, 0, shade);
    }
  }
}

interface Part { buf: WebGLBuffer; vao: WebGLVertexArrayObject; count: number; px: number; py: number; pz: number; rx: number; ry: number; rz: number }
/** `mob`: posed by mobs.ts (parents, own animation); the others by the biped/quadruped rules in drawLiving. */
interface ModelDef { parts: Record<string, Part>; tex: WebGLTexture | null; mob?: MobDef }

/** Mobs that hold their arms out (AbstractZombieModel; 1.8's SkeletonEntityModel is one too). */
const ZOMBIE_ARMS = new Set(['Zombie', 'PigZombie', 'Skeleton', 'Giant']);
/** Mobs that fall further when they die (method_5771). */
const DEATH_MAX: Record<string, number> = { Spider: 180, CaveSpider: 180 };

export interface EntityState {
  id: number; kind: number; name: string; label: string; width: number; height: number; item: number; meta: number;
  x: number; y: number; z: number; yaw: number; pitch: number; headYaw: number;
  /** Limb swing phase and amount (walk animation). */
  swing: number; swingAmt: number; px: number; pz: number; bodyYaw: number; sneak?: boolean; spin: number;
  /** Arm swing progress 0..1 (hitting/using), undefined when not swinging. */
  arm?: number;
  /** SPAWN variant: slime size, villager profession, sheep fleece, horse type << 8 | colour, skeleton type... (protocol mobVariant). */
  variant?: number;
  /** Players (SKIN): served skin and cape paths ('' / undefined: Steve, or Alex when slim), 3 px arms. */
  skin?: string; slim?: boolean; cape?: string;
  /** The cape's lagging position (PlayerEntity.prevCape*) and when it was last stepped. */
  chase?: [number, number, number, number];
  /** Animation state (anim.ts): red hurt overlay, deathTime with the partial tick (0 alive), using the held item, riding, in a bed. */
  hurt?: boolean; death?: number; using?: boolean; riding?: boolean; sleeping?: boolean;
  /** LOOKS: a baby, the armour worn ([slot, item, meta, leather rgb]), an armour stand's pose (degrees). */
  baby?: boolean;
  armour?: Array<[number, number, number, number]>;
  pose?: number[];
}

export class EntityRenderer {
  private readonly prog: WebGLProgram;
  private readonly u: Uniforms;
  private readonly models = new Map<string, ModelDef>();
  private readonly tags = new Map<string, { tex: WebGLTexture; w: number; h: number }>();
  private readonly quad: { vao: WebGLVertexArrayObject; buf: WebGLBuffer };
  private readonly cubeCache = new Map<string, { vao: WebGLVertexArrayObject; buf: WebGLBuffer; count: number }>();
  private readonly m = mat4();
  private readonly tmp = mat4();
  private readonly skins = new Map<string, WebGLTexture | null>();
  /** Brightness setting 0..1 (see GAMMA_STEPS), applied from the next begin(). */
  gamma = 0;
  /** Skull heads (SkullEntityModel on 64×32 skins, SkullOverlayEntityModel with the hat on 64×64) and the banner parts. */
  private readonly skullParts = new Map<number, Part[]>();
  private readonly bannerParts: Record<string, Part> = {};
  private readonly bannerTex = new Map<string, WebGLTexture>();
  /** Downloaded player skins and capes by served path. */
  readonly playerSkins: SkinTextures;
  private cape: Part | null = null;
  /** Armour boxes: BiPedModel(1.0F) for slots 1, 3, 4 and (0.5F) for the leggings; `v`: the zombie villager's helmet. */
  private readonly armourModels: Record<string, Record<string, Part>> = {};

  constructor(private readonly gl: WebGL2RenderingContext, private readonly images: Map<string, ImageBitmap>, private readonly array: WebGLTexture) {
    this.prog = compile(gl, MODEL_VS, MODEL_FS);
    this.u = uniforms(gl, this.prog, ['u_viewProj', 'u_model', 'u_tex2d', 'u_texArr', 'u_useArray', 'u_tint', 'u_overlay', 'u_light', 'u_fogColor', 'u_fog', 'u_daylight', 'u_gamma']);
    // unit billboard quad (-0.5..0.5) for sprites and tags
    const q: number[] = [];
    const corners = [[-0.5, -0.5, 0, 1], [0.5, -0.5, 1, 1], [0.5, 0.5, 1, 0], [-0.5, -0.5, 0, 1], [0.5, 0.5, 1, 0], [-0.5, 0.5, 0, 0]];
    for (const [x, y, u, v] of corners) q.push(x, y, 0, u, v, 0, 1);
    this.quad = this.vao(new Float32Array(q));
    this.playerSkins = new SkinTextures(gl);
    this.buildModels();
  }

  private vao(data: Float32Array): { vao: WebGLVertexArrayObject; buf: WebGLBuffer } {
    const gl = this.gl;
    const vao = gl.createVertexArray()!, buf = gl.createBuffer()!;
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, FLOATS * 4, 0);
    gl.enableVertexAttribArray(1); gl.vertexAttribPointer(1, 3, gl.FLOAT, false, FLOATS * 4, 12);
    gl.enableVertexAttribArray(2); gl.vertexAttribPointer(2, 1, gl.FLOAT, false, FLOATS * 4, 24);
    gl.bindVertexArray(null);
    return { vao, buf };
  }

  private part(boxes: Array<[number, number, number, number, number, number, number, number, number?, boolean?]>, tw: number, th: number, pivot: [number, number, number], rot: [number, number, number] = [0, 0, 0]): Part {
    const data: number[] = [];
    for (const [u, v, x, y, z, w, h, d, inflate, mirror] of boxes) box(data, u, v, x, y, z, w, h, d, inflate ?? 0, tw, th, mirror);
    const { vao, buf } = this.vao(new Float32Array(data));
    return { vao, buf, count: data.length / FLOATS, px: pivot[0], py: pivot[1], pz: pivot[2], rx: rot[0], ry: rot[1], rz: rot[2] };
  }

  private buildModels(): void {
    const gl = this.gl;
    const tex = (name: string) => { const img = this.images.get(name); return img ? texture2d(gl, img) : null; };
    // PlayerEntityModel (64×64 skins: own left limbs and the outer layer) and BiPedModel (64×32: mirrored limbs)
    const steve = tex('entity/steve');
    if (steve) this.models.set('player', { tex: steve, parts: this.biped(64, 64, true) });
    if (steve) this.models.set('player#slim', { tex: this.skin('entity/alex') ?? steve, parts: this.biped(64, 64, true, false, true) });
    // PlayerEntityModel.cloak on the 64×32 cape texture
    this.cape = this.part([[0, 0, -5, 0, -1, 10, 16, 1]], 64, 32, [0, 0, 0]);
    const zombieImg = this.images.get('entity/zombie/zombie');
    const zombie = tex('entity/zombie/zombie');
    // AbstractZombieModel is a BiPedModel on a 64×64 texture: mirrored left limbs (the lower half of zombie.png is empty)
    if (zombie && zombieImg) this.models.set('Zombie', { tex: zombie, parts: this.biped(64, zombieImg.height, false) });
    const skeleton = tex('entity/skeleton/skeleton');
    if (skeleton) this.models.set('Skeleton', { tex: skeleton, parts: this.biped(64, 32, false, true) });
    /** QuadruPedEntityModel and its subclasses: head/body boxes and pivots as in 1.8, legs of `legH` px at x ±legX, z legZ0/legZ1. */
    const quadruped = (name: string, image: string, legH: number, head: number[], headPivot: number[], body: number[], bodyPivot: number[], legX: number, legZ0: number, legZ1: number) => {
      const t = tex(image);
      const img = this.images.get(image);
      if (!t || !img) return;
      const tw = 64, th = img.height, y = 24 - legH;
      const leg = (x: number, z: number) => this.part([[0, 16, -2, 0, -2, 4, legH, 4]], tw, th, [x, y, z]);
      this.models.set(name, {
        tex: t,
        parts: {
          head: this.part([[0, 0, head[0], head[1], head[2], head[3], head[4], head[5]]], tw, th, headPivot as [number, number, number]),
          body: this.part([[body[0], body[1], body[2], body[3], body[4], body[5], body[6], body[7]]], tw, th, bodyPivot as [number, number, number], [Math.PI / 2, 0, 0]),
          leg1: leg(-legX, legZ0), leg2: leg(legX, legZ0), leg3: leg(-legX, legZ1), leg4: leg(legX, legZ1),
        },
      });
    };
    const pigman = this.images.get('entity/zombie_pigman');
    if (pigman) this.models.set('PigZombie', { tex: tex('entity/zombie_pigman'), parts: this.biped(64, pigman.height, false) });
    const giant = this.models.get('Zombie');
    if (giant) this.models.set('Giant', giant);
    // ZombieVillagerEntityModel: the biped with the villager's head and nose (64×64, at v 32)
    const zv = tex('entity/zombie/zombie_villager');
    if (zv) {
      const parts = this.biped(64, 64, false);
      parts.head = this.part([[0, 32, -4, -10, -4, 8, 10, 8], [24, 32, -1, -3, -6, 2, 4, 2]], 64, 64, [0, 0, 0]);
      this.models.set('Zombie#villager', { tex: zv, parts });
    }
    for (const [key, inflate, villager] of [['1', 1, false], ['2', 0.5, false], ['1v', 1, true], ['2v', 0.5, true]] as Array<[string, number, boolean]>) {
      const parts: Record<string, Part> = {};
      for (const [k, p] of Object.entries(armourParts(inflate, villager))) parts[k] = this.part(p.boxes, 64, 32, p.pivot);
      this.armourModels[key] = parts;
    }
    this.skullParts.set(32, [this.part([[0, 0, -4, -8, -4, 8, 8, 8]], 64, 32, [0, 0, 0])]);
    this.skullParts.set(64, [this.part([[0, 0, -4, -8, -4, 8, 8, 8]], 64, 64, [0, 0, 0]), this.part([[32, 0, -4, -8, -4, 8, 8, 8, 0.25]], 64, 64, [0, 0, 0])]);
    for (const [k, p] of Object.entries(BANNER.parts)) this.bannerParts[k] = this.part(p.boxes, BANNER.tw, BANNER.th, p.pivot, p.rot);
    for (const [name, def] of [...Object.entries(MOBS), ...Object.entries(OBJECT_MODELS), ['Sheep#fur', SHEEP_FUR] as [string, MobDef]]) {
      const parts: Record<string, Part> = {};
      for (const [k, p] of Object.entries(def.parts)) parts[k] = this.part(p.boxes, def.tw, def.th, p.pivot, p.rot);
      this.models.set(name, { tex: null, parts, mob: def });
    }
    quadruped('Pig', 'entity/pig/pig', 6, [-4, -4, -8, 8, 8, 8], [0, 12, -6], [28, 8, -5, -10, -7, 10, 16, 8], [0, 11, 2], 3, 7, -5);
    quadruped('Cow', 'entity/cow/cow', 12, [-4, -4, -6, 8, 8, 6], [0, 4, -8], [18, 4, -6, -10, -7, 12, 18, 10], [0, 5, 2], 4, 7, -6);
    quadruped('MushroomCow', 'entity/cow/mooshroom', 12, [-4, -4, -6, 8, 8, 6], [0, 4, -8], [18, 4, -6, -10, -7, 12, 18, 10], [0, 5, 2], 4, 7, -6);
    quadruped('Sheep', 'entity/sheep/sheep', 12, [-3, -4, -6, 6, 6, 8], [0, 6, -8], [28, 8, -4, -10, -7, 8, 16, 6], [0, 5, 2], 3, 7, -5);
    const creeper = tex('entity/creeper/creeper');
    if (creeper) {
      this.models.set('Creeper', {
        tex: creeper, parts: {
          head: this.part([[0, 0, -4, -8, -4, 8, 8, 8]], 64, 32, [0, 6, 0]),
          body: this.part([[16, 16, -4, 0, -2, 8, 12, 4]], 64, 32, [0, 6, 0]),
          leg1: this.part([[0, 16, -2, 0, -2, 4, 6, 4]], 64, 32, [-2, 18, 4]),
          leg2: this.part([[0, 16, -2, 0, -2, 4, 6, 4]], 64, 32, [2, 18, 4]),
          leg3: this.part([[0, 16, -2, 0, -2, 4, 6, 4]], 64, 32, [-2, 18, -4]),
          leg4: this.part([[0, 16, -2, 0, -2, 4, 6, 4]], 64, 32, [2, 18, -4]),
        },
      });
    }
  }

  /** PlayerEntityModel (64×64: own left limbs + outer layer; `slim`: Alex's 3 px arms), BiPedModel (64×32: mirrored left limbs), SkeletonEntityModel (thin limbs). */
  private biped(tw: number, th: number, modern: boolean, thin = false, slim = false): Record<string, Part> {
    type B = [number, number, number, number, number, number, number, number, number?, boolean?];
    const limb = (u: number, v: number, x: number, y: number, mirror = false): B => (thin ? [u, v, -1, y, -1, 2, 12, 2, 0, mirror] : [u, v, x, y, -2, 4, 12, 4, 0, mirror]);
    const head: B[] = [[0, 0, -4, -8, -4, 8, 8, 8], [32, 0, -4, -8, -4, 8, 8, 8, 0.5]];
    if (modern) {
      // slim arms: 3 px wide, the pivot half a pixel lower (PlayerEntityModel thinArms)
      const aw = slim ? 3 : 4, ax = slim ? -2 : -3, ay = slim ? 2.5 : 2;
      return {
        head: this.part(head, tw, th, [0, 0, 0]),
        body: this.part([[16, 16, -4, 0, -2, 8, 12, 4], [16, 32, -4, 0, -2, 8, 12, 4, 0.25]], tw, th, [0, 0, 0]),
        rightArm: this.part([[40, 16, ax, -2, -2, aw, 12, 4], [40, 32, ax, -2, -2, aw, 12, 4, 0.25]], tw, th, [-5, ay, 0]),
        leftArm: this.part([[32, 48, -1, -2, -2, aw, 12, 4], [48, 48, -1, -2, -2, aw, 12, 4, 0.25]], tw, th, [5, ay, 0]),
        rightLeg: this.part([[0, 16, -2, 0, -2, 4, 12, 4], [0, 32, -2, 0, -2, 4, 12, 4, 0.25]], tw, th, [-1.9, 12, 0]),
        leftLeg: this.part([[16, 48, -2, 0, -2, 4, 12, 4], [0, 48, -2, 0, -2, 4, 12, 4, 0.25]], tw, th, [1.9, 12, 0]),
      };
    }
    return {
      head: this.part(head, tw, th, [0, 0, 0]),
      body: this.part([[16, 16, -4, 0, -2, 8, 12, 4]], tw, th, [0, 0, 0]),
      rightArm: this.part([limb(40, 16, -3, -2)], tw, th, [-5, 2, 0]),
      leftArm: this.part([limb(40, 16, -1, -2, true)], tw, th, [5, 2, 0]),
      rightLeg: this.part([limb(0, 16, -2, 0)], tw, th, [thin ? -2 : -1.9, 12, 0]),
      leftLeg: this.part([limb(0, 16, -2, 0, true)], tw, th, [thin ? 2 : 1.9, 12, 0]),
    };
  }

  begin(viewProj: Mat4, fogColor: [number, number, number], fog: [number, number], daylight: number): void {
    const gl = this.gl;
    gl.useProgram(this.prog);
    gl.uniformMatrix4fv(this.u.u_viewProj, false, viewProj);
    gl.uniform3f(this.u.u_fogColor, fogColor[0], fogColor[1], fogColor[2]);
    gl.uniform2f(this.u.u_fog, fog[0], fog[1]);
    gl.uniform1f(this.u.u_daylight, daylight);
    gl.uniform1f(this.u.u_gamma, this.gamma);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
    gl.uniform4f(this.u.u_overlay, 0, 0, 0, 0);
    gl.uniform2f(this.u.u_light, 1, 0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, this.array);
    gl.uniform1i(this.u.u_texArr, 1);
    gl.uniform1i(this.u.u_tex2d, 0);
    gl.disable(gl.CULL_FACE);
  }

  setLight(sky: number, block: number): void { this.gl.uniform2f(this.u.u_light, sky, block); }

  /** A 2D texture of a packed image by name, made on first use. */
  private skin(name: string): WebGLTexture | null {
    if (this.skins.has(name)) return this.skins.get(name)!;
    const img = this.images.get(name);
    const t = img ? texture2d(this.gl, img) : null;
    this.skins.set(name, t);
    return t;
  }

  /**
   * One mob/player at camera-relative (x, y, z). Returns false when there is no model (the caller draws a box).
   */
  drawLiving(e: EntityState, x: number, y: number, z: number, hand?: Mat4, age = 0, head?: Mat4): boolean {
    const villager = e.name === 'Zombie' && e.variant === 1 && e.kind !== 1;
    const def = this.models.get(e.kind === 1 ? (e.slim ? 'player#slim' : 'player') : villager ? 'Zombie#villager' : e.name) ?? this.models.get(e.name);
    if (!def) return false;
    if (def.mob) return this.drawMob(def, def.mob, e, x, y, z, age, 1, 1, 1, hand, head);
    const gl = this.gl;
    gl.uniform1f(this.u.u_useArray, 0);
    gl.activeTexture(gl.TEXTURE0);
    // a wither skeleton (skeleton type 1) is the same model on its own skin
    const tex = e.kind === 1 ? this.playerSkins.get(e.skin) ?? def.tex
      : e.name === 'Skeleton' && e.variant === 1 ? this.skin('entity/skeleton/wither_skeleton') ?? def.tex : def.tex;
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
    this.overlay(e);
    // entity → world: translate, body yaw, the corpse's fall, then MC model space (pixels, y down, x mirrored, feet at 24)
    const world = this.corpse(modelMatrix(mat4(), x, y + (e.sneak ? -0.125 : 0), z, e.bodyYaw, 0, LIVING_SCALE[e.name] ?? 1), e);
    const conv = mat4();
    conv[0] = -1 / 16; conv[5] = -1 / 16; conv[10] = 1 / 16; conv[13] = 1.5;
    const base = multiply(mat4(), world, conv);
    const baby = e.baby && e.kind !== 1 ? LIVING_BABY[e.name] : undefined;
    const mats = e.armour?.length || head ? new Map<string, Mat4>() : null;
    const swing = Math.cos(e.swing * 0.6662) * 1.4 * e.swingAmt;
    const netHeadYaw = e.sleeping ? 0 : e.bodyYaw - e.headYaw;
    // bipeds: BiPedModel / AbstractZombieModel (anim.ts); quadrupeds below
    const biped = def.parts.rightArm && def.parts.leftLeg ? bipedPose({
      limb: e.swing, limbAmt: e.swingAmt, age, headYaw: normalize(netHeadYaw), pitch: -e.pitch, swing: e.arm ?? 0,
      sneak: e.sneak, riding: e.riding, zombie: ZOMBIE_ARMS.has(e.name) && e.kind !== 1, ...(e.kind === 1 ? heldPose(e.item, !!e.using) : {}),
    }, def.parts.leftLeg.px) : null;
    for (const [name, p] of Object.entries(def.parts)) {
      let rx = p.rx, ry = p.ry, rz = p.rz, px = p.px, py = p.py, pz = p.pz;
      const bp = biped?.[name as BipedPart];
      if (bp) ({ rx, ry, rz, px, py, pz } = bp);
      else if (name === 'head') { ry = normalize(netHeadYaw); rx = -e.pitch; }
      else if (name === 'leg1' || name === 'leg4') rx = swing;
      else if (name === 'leg2' || name === 'leg3') rx = -swing;
      const m = partMatrix(this.tmp, px, py, pz, rx, ry, rz);
      multiply(this.m, baby ? groupBase(base, baby, name) : base, m);
      mats?.set(name, this.m.slice() as Mat4);
      if (hand && name === 'rightArm') {
        // the hand: the arm's end in MC space, back to blocks with the flips undone
        const t = mat4(); t[12] = -1; t[13] = 10; t[14] = -1;
        multiply(hand, this.m, t);
        const f = mat4(); f[0] = -16; f[5] = -16; f[10] = 16;
        multiply(hand, hand, f);
      }
      gl.uniformMatrix4fv(this.u.u_model, false, this.m);
      gl.bindVertexArray(p.vao);
      gl.drawArrays(gl.TRIANGLES, 0, p.count);
    }
    gl.bindVertexArray(null);
    if (e.kind === 1 && e.cape) this.drawCape(e, base);
    if (mats) this.drawArmour(e, mats, villager, head);
    // the sheep's fleece over the shorn skin, unless sheared (variant bit 16)
    const fur = e.name === 'Sheep' && !((e.variant ?? 0) & 16) ? this.models.get('Sheep#fur') : undefined;
    if (fur?.mob) { const c = FLEECE[(e.variant ?? 0) & 15]; this.drawMob(fur, fur.mob, e, x, y, z, age, c[0], c[1], c[2]); }
    this.gl.uniform4f(this.u.u_overlay, 0, 0, 0, 0);
    return true;
  }

  /** The red overlay of a hurt or dying entity (off again after it is drawn). */
  private overlay(e: EntityState): void {
    const o = e.hurt ? HURT_OVERLAY : null;
    this.gl.uniform4f(this.u.u_overlay, o ? o[0] : 0, o ? o[1] : 0, o ? o[2] : 0, o ? o[3] : 0);
  }

  /** method_5777: a dying entity falls on its side about its own z axis; a sleeping one lies on its back on the bed. */
  private corpse(world: Mat4, e: EntityState): Mat4 {
    if (e.death) rotate(world, deathAngle(e.death, DEATH_MAX[e.name] ?? 90) * 180 / Math.PI, 0, 0, 1);
    else if (e.sleeping) {
      // the bed packet puts the player at the head block's corner: the head on the pillow (mattress 9/16 high), on its
      // back, the feet 1.8 towards the foot (renderOffset); which way is the bed's: the body yaw stands in for it
      world[12] += 0.5; world[13] += 0.5625 + 0.125; world[14] += 0.5;
      translate(world, 0, 0, -1.8);
      rotate(world, 90, 1, 0, 0);
    }
    return world;
  }

  /**
   * ArmorRenderer over a biped whose part matrices are `mats` (and `head` gets the head's matrix in blocks,
   * for a block worn on the head): each piece on its parts, leather in its colour plus the overlay.
   */
  private drawArmour(e: EntityState, mats: Map<string, Mat4>, villager: boolean, head?: Mat4): void {
    const hm = mats.get('head');
    if (head && hm) { const f = mat4(); f[0] = -16; f[5] = -16; f[10] = 16; multiply(head, hm, f); }
    if (!e.armour?.length || !WEARS_ARMOUR.has(e.kind === 1 ? 'player' : e.name)) return;
    const gl = this.gl;
    gl.uniform1f(this.u.u_useArray, 0);
    gl.activeTexture(gl.TEXTURE0);
    for (const [slot, item, , color] of e.armour) {
      const piece = armourPiece(item);
      if (!piece || piece[1] !== slot) continue;
      const parts = this.armourModels[`${slot === 2 ? 2 : 1}${villager ? 'v' : ''}`];
      const passes: Array<[string, [number, number, number]]> = piece[0] === 'leather'
        ? [[armourTexture('leather', slot), rgb(color === 0xffffffff ? LEATHER_DEFAULT : color)], [armourTexture('leather', slot, true), [1, 1, 1]]]
        : [[armourTexture(piece[0], slot), [1, 1, 1]]];
      for (const [texName, c] of passes) {
        const tex = this.skin(texName);
        if (!tex) continue;
        gl.bindTexture(gl.TEXTURE_2D, tex);
        gl.uniform4f(this.u.u_tint, c[0], c[1], c[2], 1);
        for (const name of ARMOUR_PARTS[slot]) {
          const m = mats.get(name), p = parts[name];
          if (!m || !p) continue;
          gl.uniformMatrix4fv(this.u.u_model, false, m);
          gl.bindVertexArray(p.vao);
          gl.drawArrays(gl.TRIANGLES, 0, p.count);
        }
      }
    }
    gl.bindVertexArray(null);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
  }

  /** A mobs.ts model: poses from posesOf, children in their parent's space, see-through parts last and blended. */
  private drawMob(def: ModelDef, mob: MobDef, e: EntityState, x: number, y: number, z: number, age: number, r: number, g: number, b: number, hand?: Mat4, head?: Mat4): boolean {
    const v = e.variant ?? 0;
    const tex = this.skin(typeof mob.texture === 'string' ? mob.texture : mob.texture(v));
    if (!tex) return false;
    const gl = this.gl;
    gl.uniform1f(this.u.u_useArray, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.uniform4f(this.u.u_tint, r, g, b, 1);
    this.overlay(e);
    // a baby replaces the model's scale with its child transforms
    const baby = e.baby ? mob.baby : undefined;
    const scale = baby ? 1 : typeof mob.scale === 'function' ? mob.scale(v) : mob.scale ?? 1;
    const world = this.corpse(modelMatrix(mat4(), x, y + (mob.lift ?? 0), z, e.bodyYaw, 0, scale), e);
    const conv = mat4();
    conv[0] = -1 / 16; conv[5] = -1 / 16; conv[10] = 1 / 16; conv[13] = 1.5;
    const base = multiply(mat4(), world, conv);
    const poses = posesOf(mob, { swing: e.swing, swingAmt: e.swingAmt, headYaw: normalize(e.bodyYaw - e.headYaw), pitch: -e.pitch, age, variant: v, rot: e.pose });
    const hidden = mob.hidden?.(v);
    const done = new Map<string, Mat4>();
    const later: Array<[Mat4, Part]> = [];
    const drawn: Array<[Mat4, Part]> = [];
    for (const [name, pd] of Object.entries(mob.parts)) {
      const p = def.parts[name], o = poses[name];
      const parentM = pd.parent ? done.get(pd.parent) ?? base : baby ? groupBase(base, baby, name) : base;
      const m = multiply(mat4(), parentM, partMatrix(this.tmp, o.px, o.py, o.pz, o.rx, o.ry, o.rz));
      done.set(name, m);
      if (hand && name === 'rightArm') {
        // as the biped's hand: the arm's end, back to blocks with the flips undone
        const t = mat4(); t[12] = -1; t[13] = 10; t[14] = -1;
        multiply(hand, m, t);
        const f = mat4(); f[0] = -16; f[5] = -16; f[10] = 16;
        multiply(hand, hand, f);
      }
      if (hidden?.includes(name)) continue;
      if (pd.translucent) { later.push([m, p]); continue; }
      drawn.push([m, p]);
      gl.uniformMatrix4fv(this.u.u_model, false, m);
      gl.bindVertexArray(p.vao);
      gl.drawArrays(gl.TRIANGLES, 0, p.count);
    }
    if (later.length) {
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      for (const [m, p] of later) {
        gl.uniformMatrix4fv(this.u.u_model, false, m);
        gl.bindVertexArray(p.vao);
        gl.drawArrays(gl.TRIANGLES, 0, p.count);
      }
      gl.disable(gl.BLEND);
    }
    // layers with the same boxes (a tame wolf's collar)
    for (const [texName, c] of mob.layers?.(v) ?? []) {
      const lt = this.skin(texName);
      if (!lt) continue;
      gl.bindTexture(gl.TEXTURE_2D, lt);
      gl.uniform4f(this.u.u_tint, c[0], c[1], c[2], 1);
      for (const [m, p] of drawn) {
        gl.uniformMatrix4fv(this.u.u_model, false, m);
        gl.bindVertexArray(p.vao);
        gl.drawArrays(gl.TRIANGLES, 0, p.count);
      }
    }
    gl.bindVertexArray(null);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
    if (e.armour?.length || head) this.drawArmour(e, done, false, head);
    gl.uniform4f(this.u.u_overlay, 0, 0, 0, 0);
    return true;
  }

  /**
   * CapeFeatureRenderer: behind the body, swung back by how far the lagging chase position trails the player
   * (forward lag × 100 / 2 + 6°), sideways by the side lag, up when falling, more when sneaking.
   */
  private drawCape(e: EntityState, base: Mat4): void {
    const tex = this.playerSkins.get(e.cape, true);
    if (!tex || !this.cape) return;
    const now = performance.now();
    const c = e.chase ?? (e.chase = [e.x, e.y, e.z, now]);
    // chasingPos += (pos − chasingPos) × 0.25 per tick, at the frame rate; a big jump (teleport) snaps it
    const k = 1 - Math.pow(0.75, Math.min(20, (now - c[3]) / 50));
    if (Math.abs(e.x - c[0]) + Math.abs(e.y - c[1]) + Math.abs(e.z - c[2]) > 10) { c[0] = e.x; c[1] = e.y; c[2] = e.z; }
    c[0] += (e.x - c[0]) * k; c[1] += (e.y - c[1]) * k; c[2] += (e.z - c[2]) * k; c[3] = now;
    const dx = c[0] - e.x, dy = c[1] - e.y, dz = c[2] - e.z;
    // the model looks along (−sin bodyYaw, −cos bodyYaw) (wire yaw); right is (cos, −sin)
    const fx = -Math.sin(e.bodyYaw), fz = -Math.cos(e.bodyYaw);
    const back = Math.min(150, Math.max(0, -(dx * fx + dz * fz) * 100));
    const side = (dx * Math.cos(e.bodyYaw) - dz * Math.sin(e.bodyYaw)) * 100;
    let up = Math.max(-6, Math.min(32, dy * 10)) + Math.sin(e.swing * 0.9) * 32 * 0.1 * e.swingAmt;
    if (e.sneak) up += 25;
    const deg = Math.PI / 180;
    const gl = this.gl;
    gl.bindTexture(gl.TEXTURE_2D, tex);
    // MC space: 2 px behind the neck, rotate X (back), Z and −Y (side), then 180° about Y
    const m = multiply(mat4(), base, partMatrix(this.tmp, 0, 0, 2, (6 + back / 2 + up) * deg, 0, 0));
    multiply(m, m, partMatrix(this.tmp, 0, 0, 0, 0, 0, (side / 2) * deg));
    multiply(m, m, partMatrix(this.tmp, 0, 0, 0, 0, (-side / 2) * deg + Math.PI, 0));
    gl.uniformMatrix4fv(this.u.u_model, false, m);
    gl.bindVertexArray(this.cape.vao);
    gl.drawArrays(gl.TRIANGLES, 0, this.cape.count);
    gl.bindVertexArray(null);
  }

  /** The bare right arm in first person (an empty hand) in view space: `m` from anim.ts fpArmMatrix (MC pixels at the pivot); the bot's own skin. */
  drawArmFirstPerson(m: Mat4, self?: EntityState): void {
    const def = this.models.get(self?.slim ? 'player#slim' : 'player');
    const p = def?.parts.rightArm;
    if (!def || !p) return;
    const gl = this.gl;
    gl.uniform1f(this.u.u_useArray, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.playerSkins.get(self?.skin) ?? def.tex);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
    gl.uniformMatrix4fv(this.u.u_model, false, m);
    gl.bindVertexArray(p.vao);
    gl.drawArrays(gl.TRIANGLES, 0, p.count);
    gl.bindVertexArray(null);
  }

  /** A skull (block 144) at camera-relative block corner (x, y, z): its type's head, floor or wall; a player skull in its owner's skin. */
  drawSkull(skull: number, meta: number, rot: number, x: number, y: number, z: number, skin?: string): void {
    const [image, th] = SKULLS[skull] ?? SKULLS[0];
    const tex = (skull === 3 ? this.playerSkins.get(skin) : null) ?? this.skin(image);
    const parts = this.skullParts.get(th);
    if (!tex || !parts) return;
    const gl = this.gl;
    gl.uniform1f(this.u.u_useArray, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
    const at = skullPlace(meta, rot);
    const m = mat4(); m[12] = x + at.x; m[13] = y + at.y; m[14] = z + at.z;
    const conv = mat4(); conv[0] = -1 / 16; conv[5] = -1 / 16; conv[10] = 1 / 16;
    multiply(m, m, conv);
    multiply(m, m, partMatrix(this.tmp, 0, 0, 0, 0, at.ry, 0));
    gl.uniformMatrix4fv(this.u.u_model, false, m);
    for (const p of parts) { gl.bindVertexArray(p.vao); gl.drawArrays(gl.TRIANGLES, 0, p.count); }
    gl.bindVertexArray(null);
  }

  /** A banner (block 176 standing / 177 on a wall) with its base colour and patterns; the cloth sways with the world time. */
  drawBanner(standing: boolean, meta: number, base: number, patterns: Array<[string, number]>, x: number, y: number, z: number, sway: number): void {
    const tex = this.bannerTexture(base, patterns);
    if (!tex) return;
    const gl = this.gl;
    gl.uniform1f(this.u.u_useArray, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
    const at = bannerPlace(standing, meta);
    const f = 2 / 3;
    const world = modelMatrix(mat4(), x + at.x, y + at.y, z + at.z, at.yaw, 0, 1);
    const conv = mat4(); conv[0] = f / 16; conv[5] = -f / 16; conv[10] = -f / 16;
    multiply(world, world, conv);
    for (const [name, p] of Object.entries(this.bannerParts)) {
      if (name === 'stand' && !at.pole) continue;
      const rx = name === 'slate' ? (-0.0125 + 0.01 * Math.cos(sway * Math.PI * 0.02)) * Math.PI : 0;
      multiply(this.m, world, partMatrix(this.tmp, p.px, p.py, p.pz, rx, 0, 0));
      gl.uniformMatrix4fv(this.u.u_model, false, this.m);
      gl.bindVertexArray(p.vao);
      gl.drawArrays(gl.TRIANGLES, 0, p.count);
    }
    gl.bindVertexArray(null);
  }

  /**
   * ColorMaskTexture: banner_base, then the base colour and every pattern: where a mask has
   * alpha, the base image's pixel times the dye colour, with the mask's red channel as alpha.
   */
  private bannerTexture(base: number, patterns: Array<[string, number]>): WebGLTexture | null {
    const layers: Array<[string, number]> = [['b', base], ...patterns];
    const key = layers.map(([p, c]) => `${p}:${c}`).join(',');
    const known = this.bannerTex.get(key);
    if (known) return known;
    const baseImg = this.images.get('entity/banner_base');
    if (!baseImg) return null;
    const w = baseImg.width, h = baseImg.height;
    const read = (img: ImageBitmap): Uint8ClampedArray => {
      const c = document.createElement('canvas'); c.width = w; c.height = h;
      const ctx = c.getContext('2d', { willReadFrequently: true })!;
      ctx.drawImage(img, 0, 0, w, h);
      return ctx.getImageData(0, 0, w, h).data;
    };
    const src = read(baseImg);
    const out = new Uint8ClampedArray(src);
    for (const [code, dye] of layers) {
      const file = BANNER_PATTERNS.find(p => p[0] === code)?.[1];
      const img = file ? this.images.get(`entity/banner/${file}`) : undefined;
      if (!img) continue;
      const mask = read(img);
      const [r, g, b] = dyeColor(dye);
      for (let i = 0; i < out.length; i += 4) {
        if (!mask[i + 3]) continue;
        const a = mask[i] / 255;
        out[i] = out[i] * (1 - a) + src[i] * r * a;
        out[i + 1] = out[i + 1] * (1 - a) + src[i + 1] * g * a;
        out[i + 2] = out[i + 2] * (1 - a) + src[i + 2] * b * a;
        out[i + 3] = Math.max(out[i + 3], mask[i]);
      }
    }
    const c = document.createElement('canvas'); c.width = w; c.height = h;
    c.getContext('2d')!.putImageData(new ImageData(out, w, h), 0, 0);
    const tex = texture2d(this.gl, c);
    if (this.bannerTex.size > 64) this.bannerTex.clear();
    this.bannerTex.set(key, tex);
    return tex;
  }

  /** A box of the entity's size for mobs without a model, tinted per name. */
  drawBox(e: EntityState, x: number, y: number, z: number, cube: { vao: WebGLVertexArrayObject; count: number } | null): void {
    if (!cube) return;
    const gl = this.gl;
    const c = colorFor(e.name);
    gl.uniform1f(this.u.u_useArray, 1);
    gl.uniform4f(this.u.u_tint, c[0], c[1], c[2], 1);
    const m = modelMatrix(this.m, x, y, z, e.yaw, 0, 1);
    // scale to the hitbox: the cube is 1×1×1 centred at x/z, from y 0
    const s = mat4(); s[0] = e.width || 0.6; s[5] = e.height || 1; s[10] = e.width || 0.6;
    multiply(m, m, s);
    gl.uniformMatrix4fv(this.u.u_model, false, m);
    gl.bindVertexArray(cube.vao);
    gl.drawArrays(gl.TRIANGLES, 0, cube.count);
    gl.bindVertexArray(null);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
  }

  /** A block model as a small mesh (dropped block items, held blocks, the fallback box). */
  blockMesh(key: string, baked: Baked | null, base: Uint16Array): { vao: WebGLVertexArrayObject; count: number } | null {
    let c = this.cubeCache.get(key);
    if (c) return c;
    if (!baked) return null;
    const data: number[] = [];
    const shadeOf = [0.5, 1, 0.8, 0.8, 0.6, 0.6];
    for (const q of baked.quads) {
      const tri = [0, 1, 2, 0, 2, 3];
      for (const k of tri) data.push(q.pos[k * 3] - 0.5, q.pos[k * 3 + 1], q.pos[k * 3 + 2] - 0.5, q.uv[k * 2], q.uv[k * 2 + 1], base[q.layer], q.dir >= 0 ? shadeOf[q.dir] : 1);
    }
    const v = this.vao(new Float32Array(data));
    c = { vao: v.vao, buf: v.buf, count: data.length / FLOATS };
    this.cubeCache.set(key, c);
    return c;
  }

  /** A block model's quads all on one texture layer (the block-break crack over that block's shape). */
  crackMesh(key: string, baked: Baked | null, layer: number): { vao: WebGLVertexArrayObject; count: number } | null {
    let c = this.cubeCache.get(key);
    if (c) return c;
    if (!baked || !baked.quads.length) return null;
    const data: number[] = [];
    for (const q of baked.quads) {
      // a hair outside the block (and its faces' own direction) so it wins the depth test with the polygon offset
      for (const k of [0, 1, 2, 0, 2, 3]) {
        const x = q.pos[k * 3], y = q.pos[k * 3 + 1], z = q.pos[k * 3 + 2];
        data.push((x - 0.5) * 1.002, (y - 0.5) * 1.002 + 0.5, (z - 0.5) * 1.002, q.uv[k * 2], q.uv[k * 2 + 1], layer, 1);
      }
    }
    const v = this.vao(new Float32Array(data));
    c = { vao: v.vao, buf: v.buf, count: data.length / FLOATS };
    if (this.cubeCache.size > 512) this.cubeCache.clear();
    this.cubeCache.set(key, c);
    return c;
  }

  /** Cracks multiply what is behind them (vanilla: DST_COLOR, SRC_COLOR: 0.5 grey leaves it as it is). */
  drawCrack(mesh: { vao: WebGLVertexArrayObject; count: number }, x: number, y: number, z: number): void {
    const gl = this.gl;
    gl.uniform1f(this.u.u_useArray, 1);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
    gl.uniform2f(this.u.u_light, 1, 1);
    const m = mat4(); m[12] = x + 0.5; m[13] = y; m[14] = z + 0.5;
    gl.uniformMatrix4fv(this.u.u_model, false, m);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.DST_COLOR, gl.SRC_COLOR);
    gl.enable(gl.POLYGON_OFFSET_FILL);
    gl.polygonOffset(-3, -3);
    gl.depthMask(false);
    gl.bindVertexArray(mesh.vao);
    gl.drawArrays(gl.TRIANGLES, 0, mesh.count);
    gl.bindVertexArray(null);
    gl.depthMask(true);
    gl.disable(gl.POLYGON_OFFSET_FILL);
    gl.disable(gl.BLEND);
    gl.uniform2f(this.u.u_light, 1, 0);
  }

  /** A flat item with depth: front and back faces plus a side quad on every edge of an opaque pixel. */
  itemMesh(key: string, layer: number, mask: Uint8Array): { vao: WebGLVertexArrayObject; count: number } {
    let c = this.cubeCache.get(key);
    if (c) return c;
    const d: number[] = [];
    const t = 1 / 32, p = 1 / 16;
    const quad = (a: number[], b: number[], cc: number[], dd: number[], uv: number[][], shade: number) => {
      for (const [v, k] of [[a, 0], [b, 1], [cc, 2], [a, 0], [cc, 2], [dd, 3]] as Array<[number[], number]>) d.push(v[0], v[1], v[2], uv[k][0], uv[k][1], layer, shade);
    };
    const full = [[0, 1], [1, 1], [1, 0], [0, 0]];
    quad([-0.5, -0.5, t], [0.5, -0.5, t], [0.5, 0.5, t], [-0.5, 0.5, t], full, 1);
    quad([0.5, -0.5, -t], [-0.5, -0.5, -t], [-0.5, 0.5, -t], [0.5, 0.5, -t], [[1, 1], [0, 1], [0, 0], [1, 0]], 0.8);
    const at = (i: number, j: number) => (i >= 0 && i < 16 && j >= 0 && j < 16 ? mask[i + j * 16] : 0);
    for (let j = 0; j < 16; j++) for (let i = 0; i < 16; i++) {
      if (!at(i, j)) continue;
      const x0 = -0.5 + i * p, x1 = x0 + p, y1 = 0.5 - j * p, y0 = y1 - p;
      const c0 = [(i + 0.5) / 16, (j + 0.5) / 16];
      const uv = [c0, c0, c0, c0];
      if (!at(i - 1, j)) quad([x0, y0, -t], [x0, y0, t], [x0, y1, t], [x0, y1, -t], uv, 0.6);
      if (!at(i + 1, j)) quad([x1, y0, t], [x1, y0, -t], [x1, y1, -t], [x1, y1, t], uv, 0.6);
      if (!at(i, j - 1)) quad([x0, y1, t], [x1, y1, t], [x1, y1, -t], [x0, y1, -t], uv, 1);
      if (!at(i, j + 1)) quad([x0, y0, -t], [x1, y0, -t], [x1, y0, t], [x0, y0, t], uv, 0.5);
    }
    const v = this.vao(new Float32Array(d));
    c = { vao: v.vao, buf: v.buf, count: d.length / FLOATS };
    this.cubeCache.set(key, c);
    return c;
  }

  /** Item sprite (a flat icon from the texture array) or a block mesh, spinning and bobbing. */
  drawItem(x: number, y: number, z: number, spin: number, layer: number, mesh: { vao: WebGLVertexArrayObject; count: number } | null, scale = 0.5, isBlock = true): void {
    const gl = this.gl;
    gl.uniform1f(this.u.u_useArray, 1);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
    if (mesh) {
      modelMatrix(this.m, x, y + (isBlock ? 0 : scale * 0.5), z, spin, 0, isBlock ? scale * 0.5 : scale);
      gl.uniformMatrix4fv(this.u.u_model, false, this.m);
      gl.bindVertexArray(mesh.vao);
      gl.drawArrays(gl.TRIANGLES, 0, mesh.count);
    } else {
      modelMatrix(this.m, x, y + scale * 0.5, z, spin, 0, scale);
      gl.uniformMatrix4fv(this.u.u_model, false, this.m);
      this.drawQuadLayer(layer);
    }
    gl.bindVertexArray(null);
  }

  private quadLayer = new Map<number, { vao: WebGLVertexArrayObject; buf: WebGLBuffer }>();
  private drawQuadLayer(layer: number): void {
    let q = this.quadLayer.get(layer);
    if (!q) {
      const d: number[] = [];
      const corners = [[-0.5, -0.5, 0, 1], [0.5, -0.5, 1, 1], [0.5, 0.5, 1, 0], [-0.5, -0.5, 0, 1], [0.5, 0.5, 1, 0], [-0.5, 0.5, 0, 0]];
      for (const [x, y, u, v] of corners) d.push(x, y, 0, u, v, layer, 1);
      q = this.vao(new Float32Array(d));
      this.quadLayer.set(layer, q);
    }
    this.gl.bindVertexArray(q.vao);
    this.gl.drawArrays(this.gl.TRIANGLES, 0, 6);
  }

  /** Item drawn in a given matrix (held item in first person). */
  drawItemMatrix(m: Mat4, layer: number, mesh: { vao: WebGLVertexArrayObject; count: number } | null): void {
    const gl = this.gl;
    gl.uniform1f(this.u.u_useArray, 1);
    gl.uniformMatrix4fv(this.u.u_model, false, m);
    if (mesh) { gl.bindVertexArray(mesh.vao); gl.drawArrays(gl.TRIANGLES, 0, mesh.count); }
    else this.drawQuadLayer(layer);
    gl.bindVertexArray(null);
  }

  /** A vertex array in camera-relative coordinates with a 2D texture (clouds), blended, no depth writes. */
  drawTextured(vao: WebGLVertexArrayObject, count: number, tex: WebGLTexture, tint: [number, number, number, number], fog: [number, number]): void {
    const gl = this.gl;
    gl.uniform1f(this.u.u_useArray, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.uniform4f(this.u.u_tint, tint[0], tint[1], tint[2], tint[3]);
    gl.uniform2f(this.u.u_light, 1, 0);
    gl.uniform2f(this.u.u_fog, fog[0], fog[1]);
    gl.uniformMatrix4fv(this.u.u_model, false, mat4());
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.depthMask(false);
    gl.bindVertexArray(vao);
    gl.drawArrays(gl.TRIANGLES, 0, count);
    gl.bindVertexArray(null);
    gl.depthMask(true);
    gl.disable(gl.BLEND);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
  }

  /** A dynamic vertex array in this renderer's layout (pos, uv + layer, shade). */
  dynamic(): { vao: WebGLVertexArrayObject; buf: WebGLBuffer } { return this.vao(new Float32Array(FLOATS * 6)); }

  setViewProj(viewProj: Mat4): void { this.gl.uniformMatrix4fv(this.u.u_viewProj, false, viewProj); }

  /** Text on a sign board: a quad of `w`×`h` blocks in the given (camera-relative) matrix, depth tested. */
  drawSignText(tex: WebGLTexture, m: Mat4, sky: number): void {
    const gl = this.gl;
    gl.uniform1f(this.u.u_useArray, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
    gl.uniform2f(this.u.u_light, sky, 0);
    gl.uniformMatrix4fv(this.u.u_model, false, m);
    gl.bindVertexArray(this.quad.vao);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    gl.bindVertexArray(null);
  }

  /** Nametag billboard: facing the camera (camYaw/camPitch), constant world size, through walls unless `through` is false (a sneaking player's). */
  drawTag(text: string, x: number, y: number, z: number, camYaw: number, camPitch: number, through = true): void {
    const gl = this.gl;
    let t = this.tags.get(text);
    if (!t) {
      const c = document.createElement('canvas');
      const ctx = c.getContext('2d')!;
      const font = '600 28px system-ui, -apple-system, "Segoe UI", sans-serif';
      ctx.font = font;
      const w = Math.ceil(ctx.measureText(text).width) + 16, h = 40;
      c.width = w; c.height = h;
      ctx.fillStyle = 'rgba(0,0,0,0.42)';
      ctx.fillRect(0, 0, w, h);
      ctx.font = font;
      ctx.fillStyle = '#fff';
      ctx.textBaseline = 'middle';
      ctx.fillText(text, 8, h / 2 + 1);
      t = { tex: texture2d(gl, c), w, h };
      if (this.tags.size > 64) this.tags.clear();
      this.tags.set(text, t);
    }
    gl.uniform1f(this.u.u_useArray, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, t.tex);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
    gl.uniform2f(this.u.u_light, 1, 1);
    const k = 0.32 / 40;
    modelMatrix(this.m, x, y, z, camYaw, camPitch, 1);
    const s = mat4(); s[0] = t.w * k; s[5] = t.h * k;
    multiply(this.m, this.m, s);
    gl.uniformMatrix4fv(this.u.u_model, false, this.m);
    if (through) gl.disable(gl.DEPTH_TEST);
    else gl.uniform4f(this.u.u_tint, 1, 1, 1, 0.55);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.bindVertexArray(this.quad.vao);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    gl.bindVertexArray(null);
    gl.disable(gl.BLEND);
    gl.enable(gl.DEPTH_TEST);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
  }

  private orbQuads: Array<{ vao: WebGLVertexArrayObject; buf: WebGLBuffer }> = [];
  /** An XP orb (ExperienceOrbEntityRenderer): its frame of experience_orb.png, 0.3 blocks, facing the camera, in its pulsing colour. */
  drawOrb(x: number, y: number, z: number, frame: number, color: [number, number, number], camYaw: number, camPitch: number): void {
    const tex = this.skin('entity/experience_orb');
    if (!tex) return;
    const gl = this.gl;
    let q = this.orbQuads[frame];
    if (!q) {
      const u0 = (frame % 4) / 4, v0 = Math.floor(frame / 4) / 4, d: number[] = [];
      const corners = [[-0.5, -0.25, 0, 1], [0.5, -0.25, 1, 1], [0.5, 0.75, 1, 0], [-0.5, -0.25, 0, 1], [0.5, 0.75, 1, 0], [-0.5, 0.75, 0, 0]];
      for (const [cx, cy, u, v] of corners) d.push(cx, cy, 0, u0 + u / 4, v0 + v / 4, 0, 1);
      q = this.orbQuads[frame] = this.vao(new Float32Array(d));
    }
    gl.uniform1f(this.u.u_useArray, 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.uniform4f(this.u.u_tint, color[0], color[1], color[2], 1);
    gl.uniform2f(this.u.u_light, 1, 1);
    modelMatrix(this.m, x, y + 0.1, z, camYaw, camPitch, 0.3);
    gl.uniformMatrix4fv(this.u.u_model, false, this.m);
    gl.bindVertexArray(q.vao);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    gl.bindVertexArray(null);
    gl.uniform4f(this.u.u_tint, 1, 1, 1, 1);
    gl.uniform2f(this.u.u_light, 1, 0);
  }
}

/** The base matrix of a baby's top-level part: scaled, then moved in the scaled space (BiPedModel/QuadruPedEntityModel child). */
function groupBase(base: Mat4, baby: Baby, part: string): Mat4 {
  const [s, tx, ty, tz] = babyGroup(baby, part)!;
  const g = mat4(); g[0] = s; g[5] = s; g[10] = s; g[12] = s * tx; g[13] = s * ty; g[14] = s * tz;
  return multiply(mat4(), base, g);
}

/** MC ModelPart: translate to the rotation point, rotate Z, Y, X. */
function partMatrix(out: Mat4, px: number, py: number, pz: number, rx: number, ry: number, rz: number): Mat4 {
  const t = mat4(); t[12] = px; t[13] = py; t[14] = pz;
  const z = mat4(); const cz = Math.cos(rz), sz = Math.sin(rz); z[0] = cz; z[1] = sz; z[4] = -sz; z[5] = cz;
  const y = mat4(); const cy = Math.cos(ry), sy = Math.sin(ry); y[0] = cy; y[2] = -sy; y[8] = sy; y[10] = cy;
  const x = mat4(); const cx = Math.cos(rx), sx = Math.sin(rx); x[5] = cx; x[6] = sx; x[9] = -sx; x[10] = cx;
  multiply(out, t, z); multiply(out, out, y); multiply(out, out, x);
  return out;
}

function normalize(a: number): number {
  let d = a % (Math.PI * 2);
  if (d > Math.PI) d -= Math.PI * 2;
  if (d < -Math.PI) d += Math.PI * 2;
  return Math.max(-1.3, Math.min(1.3, d));
}

function colorFor(name: string): [number, number, number] {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) | 0;
  const hue = ((h >>> 0) % 360) / 360;
  const f = (n: number) => { const k = (n + hue * 12) % 12; return 0.6 - 0.3 * Math.max(-1, Math.min(k - 3, 9 - k, 1)); };
  return [f(0), f(8), f(4)];
}
