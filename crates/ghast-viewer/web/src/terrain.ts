/// <reference lib="dom" />
/** Section meshes on the GPU and the terrain passes (solid front to back, cutout, translucent back to front). */
import { compile, uniforms, TERRAIN_FS, TERRAIN_VS, type QuadIndex, type Uniforms } from './gl.js';
import { boxVisible } from './math.js';
import { VERTEX_BYTES } from './mesher.js';

interface PassGL { vao: WebGLVertexArrayObject; buf: WebGLBuffer; quads: number }
interface SectionGL { cx: number; sy: number; cz: number; passes: Array<PassGL | null> }

export interface FrameParams {
  viewProj: Float32Array;
  planes: Float32Array;
  cam: [number, number, number];
  fogColor: [number, number, number];
  fog: [number, number];
  daylight: number;
  ticks: number;
  /** Brightness setting 0 (Moody) .. 1 (Bright). */
  gamma?: number;
}

export class Terrain {
  private readonly sections = new Map<string, SectionGL>();
  private readonly prog: WebGLProgram;
  private readonly u: Uniforms;
  /** Draw calls and quads of the last frame (debug overlay). */
  drawn = 0;
  quads = 0;
  gpuBytes = 0;
  /** Debugging (?skip=solid,cutout,translucent): passes left out. */
  skip = new Set<string>();
  /** performance.now() of the last upload (the adaptive resolution waits for loading to settle). */
  uploadsAt = 0;

  constructor(private readonly gl: WebGL2RenderingContext, private readonly index: QuadIndex, private readonly array: WebGLTexture) {
    this.prog = compile(gl, TERRAIN_VS, TERRAIN_FS);
    this.u = uniforms(gl, this.prog, ['u_viewProj', 'u_origin', 'u_ticks', 'u_tex', 'u_fogColor', 'u_fog', 'u_cutout', 'u_alpha', 'u_daylight', 'u_camMod', 'u_gamma']);
  }

  get count(): number { return this.sections.size; }

  upload(cx: number, sy: number, cz: number, passes: Array<ArrayBuffer | null>): void {
    this.uploadsAt = performance.now();
    const k = `${cx},${sy},${cz}`;
    let s = this.sections.get(k);
    if (!s) { s = { cx, sy, cz, passes: [null, null, null] }; this.sections.set(k, s); }
    for (let p = 0; p < 3; p++) {
      const data = passes[p];
      const old = s.passes[p];
      if (!data || !data.byteLength) { if (old) this.free(old); s.passes[p] = null; continue; }
      const pass = old ?? this.createPass();
      const gl = this.gl;
      gl.bindBuffer(gl.ARRAY_BUFFER, pass.buf);
      if (old) this.gpuBytes -= old.quads * 4 * VERTEX_BYTES;
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      pass.quads = data.byteLength / VERTEX_BYTES / 4;
      this.gpuBytes += data.byteLength;
      this.index.ensure(pass.quads);
      s.passes[p] = pass;
    }
    if (s.passes.every(p => !p)) this.sections.delete(k);
  }

  drop(cx: number, cz: number): void {
    for (const [k, s] of this.sections) {
      if (s.cx !== cx || s.cz !== cz) continue;
      for (const p of s.passes) if (p) this.free(p);
      this.sections.delete(k);
    }
  }

  clear(): void {
    for (const s of this.sections.values()) for (const p of s.passes) if (p) this.free(p);
    this.sections.clear();
  }

  private createPass(): PassGL {
    const gl = this.gl;
    const vao = gl.createVertexArray()!;
    const buf = gl.createBuffer()!;
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.index.buffer);
    // integer attributes (uvec in the shader) and one normalized colour: D3D has no "int read as float" vertex
    // formats (nor 3-component 16-bit ones), so anything else gets converted on the CPU on every draw there
    gl.enableVertexAttribArray(0); gl.vertexAttribIPointer(0, 4, gl.UNSIGNED_SHORT, VERTEX_BYTES, 0);
    gl.enableVertexAttribArray(2); gl.vertexAttribIPointer(2, 2, gl.UNSIGNED_SHORT, VERTEX_BYTES, 8);
    gl.enableVertexAttribArray(3); gl.vertexAttribPointer(3, 4, gl.UNSIGNED_BYTE, true, VERTEX_BYTES, 12);
    gl.enableVertexAttribArray(4); gl.vertexAttribIPointer(4, 4, gl.UNSIGNED_BYTE, VERTEX_BYTES, 16);
    gl.bindVertexArray(null);
    return { vao, buf, quads: 0 };
  }

  private free(p: PassGL): void {
    this.gpuBytes -= p.quads * 4 * VERTEX_BYTES;
    this.gl.deleteBuffer(p.buf);
    this.gl.deleteVertexArray(p.vao);
  }

  private visible(f: FrameParams): Array<{ s: SectionGL; d: number; ox: number; oy: number; oz: number }> {
    const out: Array<{ s: SectionGL; d: number; ox: number; oy: number; oz: number }> = [];
    const [camX, camY, camZ] = f.cam;
    const far = f.fog[1] + 24;
    for (const s of this.sections.values()) {
      const ox = s.cx * 16 - camX, oy = s.sy * 16 - camY, oz = s.cz * 16 - camZ;
      const dx = ox + 8, dy = oy + 8, dz = oz + 8;
      if (Math.abs(dx) > far || Math.abs(dz) > far) continue;
      if (!boxVisible(f.planes, ox, oy - 1, oz, ox + 16, oy + 17, oz + 16)) continue;
      out.push({ s, d: dx * dx + dy * dy + dz * dz, ox, oy, oz });
    }
    return out;
  }

  /** Solid and cutout passes; returns the visible list for the translucent pass. */
  drawOpaque(f: FrameParams): ReturnType<Terrain['visible']> {
    const gl = this.gl;
    const vis = this.visible(f).sort((a, b) => a.d - b.d);
    gl.useProgram(this.prog);
    this.common(f);
    this.drawn = 0; this.quads = 0;
    gl.disable(gl.BLEND);
    gl.depthMask(true);
    gl.enable(gl.CULL_FACE);
    for (const pass of [0, 1]) {
      if (this.skip.has(pass === 0 ? 'solid' : 'cutout')) continue;
      gl.uniform1f(this.u.u_cutout, pass);
      gl.uniform1f(this.u.u_alpha, 0);
      // cutout plants are seen from both sides
      if (pass === 1) gl.disable(gl.CULL_FACE);
      for (const v of vis) {
        const p = v.s.passes[pass];
        if (!p) continue;
        gl.uniform3f(this.u.u_origin, v.ox, v.oy, v.oz);
        gl.bindVertexArray(p.vao);
        gl.drawElements(gl.TRIANGLES, p.quads * 6, gl.UNSIGNED_INT, 0);
        this.drawn++; this.quads += p.quads;
      }
    }
    gl.enable(gl.CULL_FACE);
    gl.bindVertexArray(null);
    return vis;
  }

  drawTranslucent(f: FrameParams, vis: ReturnType<Terrain['visible']>): void {
    if (this.skip.has('translucent')) return;
    const gl = this.gl;
    gl.useProgram(this.prog);
    this.common(f);
    gl.uniform1f(this.u.u_cutout, 0);
    gl.uniform1f(this.u.u_alpha, 1);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
    gl.depthMask(false);
    gl.enable(gl.CULL_FACE);
    for (let i = vis.length - 1; i >= 0; i--) {
      const v = vis[i];
      const p = v.s.passes[2];
      if (!p) continue;
      gl.uniform3f(this.u.u_origin, v.ox, v.oy, v.oz);
      gl.bindVertexArray(p.vao);
      gl.drawElements(gl.TRIANGLES, p.quads * 6, gl.UNSIGNED_INT, 0);
      this.drawn++; this.quads += p.quads;
    }
    gl.bindVertexArray(null);
    gl.depthMask(true);
    gl.disable(gl.BLEND);
  }

  private common(f: FrameParams): void {
    const gl = this.gl;
    gl.uniformMatrix4fv(this.u.u_viewProj, false, f.viewProj);
    gl.uniform1f(this.u.u_ticks, f.ticks);
    // world position for per-block hashes, wrapped to keep floats exact (a seam every 4096 blocks)
    const m = (v: number) => v - Math.floor(v / 4096) * 4096;
    gl.uniform3f(this.u.u_camMod, m(f.cam[0]), m(f.cam[1]), m(f.cam[2]));
    gl.uniform1f(this.u.u_daylight, f.daylight);
    gl.uniform1f(this.u.u_gamma, f.gamma ?? 0);
    gl.uniform3f(this.u.u_fogColor, f.fogColor[0], f.fogColor[1], f.fogColor[2]);
    gl.uniform2f(this.u.u_fog, f.fog[0], f.fog[1]);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D_ARRAY, this.array);
    gl.uniform1i(this.u.u_tex, 0);
  }
}
