/// <reference lib="dom" />
/**
 * Lightning (1.8 LightningBoltEntity + LightningEntityRenderer): a bolt lives a few ticks, strikes 1..3
 * times with a new shape each time, and flashes the sky while it strikes (World.field_4553).
 * The shape is vanilla's: java.util.Random seeded with the bolt's vertex seed, an 8-segment zig-zag
 * 128 blocks high with two short branches, drawn 4 times wider and wider, additive.
 */
import { compile, uniforms, type Uniforms } from './gl.js';

const MULT = 0x5deece66dn, MASK = (1n << 48n) - 1n;

/** java.util.Random (the bolt's shape must come from the same sequence as vanilla's). */
export class JavaRandom {
  private seed: bigint;
  constructor(seed: bigint) { this.seed = (seed ^ MULT) & MASK; }
  private next(bits: number): number {
    this.seed = (this.seed * MULT + 0xbn) & MASK;
    return Number(BigInt.asIntN(32, this.seed >> BigInt(48 - bits)));
  }
  nextInt(bound: number): number {
    if ((bound & -bound) === bound) return Number((BigInt(bound) * BigInt(this.next(31))) >> 31n);
    let bits: number, val: number;
    do { bits = this.next(31); val = bits % bound; } while (((bits - val + (bound - 1)) | 0) < 0);
    return val;
  }
  nextLong(): bigint { return BigInt.asIntN(64, (BigInt(this.next(32)) << 32n) + BigInt(this.next(32))); }
}

/** LightningEntityRenderer's triangles (x, y, z relative to the bolt's block corner + 0.5), from its vertex seed. */
export function boltTriangles(vertex: bigint): Float32Array {
  const ax = new Array<number>(8), az = new Array<number>(8);
  const random = new JavaRandom(vertex);
  let d0 = 0, d1 = 0;
  for (let i = 7; i >= 0; i--) { ax[i] = d0; az[i] = d1; d0 += random.nextInt(11) - 5; d1 += random.nextInt(11) - 5; }
  const out: number[] = [];
  for (let k1 = 0; k1 < 4; k1++) {
    const random1 = new JavaRandom(vertex);
    for (let j = 0; j < 3; j++) {
      const k = j > 0 ? 7 - j : 7, l = j > 0 ? k - 2 : 0;
      let d2 = ax[k] - d0, d3 = az[k] - d1;
      for (let i1 = k; i1 >= l; i1--) {
        const d4 = d2, d5 = d3;
        if (j === 0) { d2 += random1.nextInt(11) - 5; d3 += random1.nextInt(11) - 5; }
        else { d2 += random1.nextInt(31) - 15; d3 += random1.nextInt(31) - 15; }
        let d6 = 0.1 + k1 * 0.2, d7 = 0.1 + k1 * 0.2;
        if (j === 0) { d6 *= i1 * 0.1 + 1; d7 *= (i1 - 1) * 0.1 + 1; }
        // a triangle strip around the segment (5 pairs: the square's corners and back to the first)
        const strip: number[][] = [];
        for (let j1 = 0; j1 < 5; j1++) {
          let d8 = 0.5 - d6, d9 = 0.5 - d6, d10 = 0.5 - d7, d11 = 0.5 - d7;
          if (j1 === 1 || j1 === 2) { d8 += d6 * 2; d10 += d7 * 2; }
          if (j1 === 2 || j1 === 3) { d9 += d6 * 2; d11 += d7 * 2; }
          strip.push([d10 + d2, i1 * 16, d11 + d3], [d8 + d4, (i1 + 1) * 16, d9 + d5]);
        }
        for (let t = 0; t + 2 < strip.length; t++) out.push(...strip[t], ...strip[t + 1], ...strip[t + 2]);
      }
    }
  }
  return new Float32Array(out);
}

const randomLong = (): bigint => BigInt.asIntN(64, (BigInt(Math.floor(Math.random() * 2 ** 32)) << 32n) | BigInt(Math.floor(Math.random() * 2 ** 32)));

/** LightningBoltEntity's life: strikes while state ≥ 0, restrikes after a random pause, then goes. */
export class Bolt {
  state = 2;
  living: number;
  vertex: bigint;
  constructor(public x: number, public y: number, public z: number, private readonly R: () => number = Math.random, vertex = randomLong()) {
    this.living = ((R() * 3) | 0) + 1;
    this.vertex = vertex;
  }
  /** One tick; false when the bolt is gone. */
  tick(): boolean {
    this.state--;
    if (this.state < 0) {
      if (this.living === 0) return false;
      if (this.state < -((this.R() * 10) | 0)) { this.living--; this.state = 1; this.vertex = randomLong(); }
    }
    return true;
  }
  get striking(): boolean { return this.state >= 0; }
}

const VS = `#version 300 es
precision highp float;
layout(location=0) in vec3 a_pos;
uniform mat4 u_viewProj;
uniform vec3 u_offset;
void main() { gl_Position = u_viewProj * vec4(a_pos + u_offset, 1.0); }`;
const FS = `#version 300 es
precision highp float;
out vec4 o;
void main() { o = vec4(0.45, 0.45, 0.5, 0.3); }`;

/** Bolts and the sky flash (field_4553: 2 ticks after the last strike). */
export class Lightning {
  bolts: Bolt[] = [];
  /** World.field_4553. */
  last = 0;
  private prog: WebGLProgram | null = null;
  private u: Uniforms = {};
  private vao: WebGLVertexArrayObject | null = null;
  private buf: WebGLBuffer | null = null;
  private shapes = new Map<Bolt, { vertex: bigint; count: number; data: Float32Array }>();

  add(x: number, y: number, z: number): void { if (this.bolts.length < 16) this.bolts.push(new Bolt(x, y, z)); }
  clear(): void { this.bolts = []; this.shapes.clear(); this.last = 0; }

  tick(): void {
    if (this.last > 0) this.last--;
    this.bolts = this.bolts.filter((b) => {
      const alive = b.tick();
      if (alive && b.striking) this.last = 2;
      if (!alive) this.shapes.delete(b);
      return alive;
    });
  }

  /** The sky flash 0..0.45 at partial tick `pt` (World.method_3631 mixes towards 0.8, 0.8, 1 by it). */
  flash(pt: number): number { return this.last > 0 ? Math.min(1, this.last - pt) * 0.45 : 0; }

  draw(gl: WebGL2RenderingContext, viewProj: Float32Array, cam: { x: number; y: number; z: number }): void {
    if (!this.bolts.length) return;
    if (!this.prog) {
      this.prog = compile(gl, VS, FS);
      this.u = uniforms(gl, this.prog, ['u_viewProj', 'u_offset']);
      this.vao = gl.createVertexArray()!; this.buf = gl.createBuffer()!;
      gl.bindVertexArray(this.vao);
      gl.bindBuffer(gl.ARRAY_BUFFER, this.buf);
      gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
      gl.bindVertexArray(null);
    }
    gl.useProgram(this.prog);
    gl.uniformMatrix4fv(this.u.u_viewProj, false, viewProj);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE);
    gl.depthMask(false);
    gl.disable(gl.CULL_FACE);
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buf);
    for (const b of this.bolts) {
      let s = this.shapes.get(b);
      if (!s || s.vertex !== b.vertex) { const data = boltTriangles(b.vertex); s = { vertex: b.vertex, count: data.length / 3, data }; this.shapes.set(b, s); }
      gl.bufferData(gl.ARRAY_BUFFER, s.data, gl.STREAM_DRAW);
      gl.uniform3f(this.u.u_offset, b.x - cam.x, b.y - cam.y, b.z - cam.z);
      gl.drawArrays(gl.TRIANGLES, 0, s.count);
    }
    gl.bindVertexArray(null);
    gl.depthMask(true);
    gl.disable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  }
}
