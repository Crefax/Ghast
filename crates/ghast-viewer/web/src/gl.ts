/** WebGL2 helpers: programs, the shared quad index buffer. */

export function compile(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const sh = (type: number, src: string) => {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(`shader: ${gl.getShaderInfoLog(s)}`);
    return s;
  };
  const p = gl.createProgram()!;
  gl.attachShader(p, sh(gl.VERTEX_SHADER, vs));
  gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(`program: ${gl.getProgramInfoLog(p)}`);
  return p;
}

export type Uniforms = Record<string, WebGLUniformLocation | null>;
export function uniforms(gl: WebGL2RenderingContext, p: WebGLProgram, names: string[]): Uniforms {
  const out: Uniforms = {};
  for (const n of names) out[n] = gl.getUniformLocation(p, n);
  return out;
}

/** Quads are 4 vertices (0,1,2 / 0,2,3): one index buffer for every mesh. */
export class QuadIndex {
  buffer: WebGLBuffer;
  quads = 0;
  constructor(private readonly gl: WebGL2RenderingContext) { this.buffer = gl.createBuffer()!; this.ensure(16384); }
  ensure(quads: number): void {
    if (quads <= this.quads) return;
    let n = Math.max(this.quads, 16384);
    while (n < quads) n *= 2;
    const idx = new Uint32Array(n * 6);
    for (let q = 0, i = 0; q < n; q++) {
      const v = q * 4;
      idx[i++] = v; idx[i++] = v + 1; idx[i++] = v + 2; idx[i++] = v; idx[i++] = v + 2; idx[i++] = v + 3;
    }
    const gl = this.gl;
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.buffer);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, idx, gl.STATIC_DRAW);
    this.quads = n;
  }
}

// ---- shaders -------------------------------------------------------------------------------------

/** The brightness setting (vanilla's gamma slider): Moody, half way, Bright; the page's chip cycles through them. */
export const GAMMA_STEPS: ReadonlyArray<{ gamma: number; label: string }> = [
  { gamma: 0, label: 'Moody' }, { gamma: 0.5, label: '50%' }, { gamma: 1, label: 'Bright' },
];

/** Shared lighting: vanilla's light curve, sky light scaled by the daylight, warm block light, the gamma setting. */
export const LIGHT = `
uniform float u_daylight;
uniform float u_gamma;
// GameRenderer.updateLightmap: the brightness table of the sky level, scaled by the sun (0.05..1)
vec3 lightOf(float sky, float block) {
  float bs = sky / (4.0 - 3.0 * sky) * (u_daylight * 0.95 + 0.05);
  float bb = block / (4.0 - 3.0 * block);
  vec3 skyCol = vec3(bs) * mix(vec3(0.55, 0.6, 0.85), vec3(1.0), u_daylight);
  vec3 blockCol = vec3(bb, bb * 0.88, bb * 0.75);
  vec3 c = max(max(skyCol, blockCol), vec3(0.045));
  // gamma: c·(1 − g) + (1 − (1 − c)⁴)·g lifts the dark end
  vec3 inv = 1.0 - c;
  return mix(c, 1.0 - inv * inv * inv * inv, u_gamma);
}
`;

export const TERRAIN_VS = `#version 300 es
precision highp float;
layout(location=0) in uvec4 a_posLayerI;
layout(location=2) in uvec2 a_uvI;
layout(location=3) in vec4 a_color;
layout(location=4) in uvec4 a_lightI;
uniform mat4 u_viewProj;
uniform vec3 u_origin;
uniform float u_ticks;
uniform vec3 u_camMod;
out vec3 v_uvl;
out vec3 v_wpos;
out float v_rnd;
out float v_quad;
out vec3 v_color;
out vec2 v_light;
out float v_dist;
void main() {
  vec4 a_posLayer = vec4(a_posLayerI);
  vec2 a_uv = vec2(a_uvI);
  vec4 a_light = vec4(a_lightI);
  vec3 p = a_posLayer.xyz / 2048.0 - 1.0 + u_origin;
  gl_Position = u_viewProj * vec4(p, 1.0);
  float frames = mod(a_light.z, 128.0);
  // flag 128: a random turn on still textures, the 2×2 slices of a 32 px flow texture on animated ones
  bool flag = a_light.z >= 127.5;
  v_rnd = flag && frames <= 1.0 ? 1.0 : 0.0;
  v_quad = flag && frames > 1.0 ? 1.0 : 0.0;
  v_wpos = p + u_camMod;
  float layer = a_posLayer.w;
  if (frames > 1.0) layer += mod(floor(u_ticks / max(a_light.w, 1.0)), frames) * (flag ? 4.0 : 1.0);
  v_uvl = vec3(a_uv / 256.0 - 16.0, layer);
  v_color = a_color.rgb;
  v_light = a_light.xy / 255.0;
  v_dist = length(p);
}`;

export const TERRAIN_FS = `#version 300 es
precision highp float;
precision highp sampler2DArray;
uniform sampler2DArray u_tex;
uniform vec3 u_fogColor;
uniform vec2 u_fog;
uniform float u_cutout;
uniform float u_alpha;
in vec3 v_uvl;
in vec3 v_color;
in vec2 v_light;
in float v_dist;
in vec3 v_wpos;
in float v_rnd;
in float v_quad;
out vec4 o;
${LIGHT}
void main() {
  vec4 t;
  if (v_rnd > 0.5) {
    // random 90° turn per block (vanilla's y-rotated variants), so merged faces keep the variety
    vec2 b = floor(v_wpos.xz + 0.0001);
    float h = fract(sin(dot(b, vec2(12.9898, 78.233))) * 43758.5453);
    vec2 f = fract(v_uvl.xy);
    int r = int(h * 4.0);
    vec2 g = r == 0 ? f : r == 1 ? vec2(1.0 - f.y, f.x) : r == 2 ? 1.0 - f : vec2(f.y, 1.0 - f.x);
    // 1 - 0 = 1.0 would wrap to the opposite edge: a line of wrong texels on block borders
    g = clamp(g, 0.0001, 0.9999);
    t = textureGrad(u_tex, vec3(g, v_uvl.z), dFdx(v_uvl.xy), dFdy(v_uvl.xy));
  } else if (v_quad > 0.5) {
    // a 32 px flow texture as 2×2 slices: the uv is in 16 px units, the quadrant picks the slice
    vec2 q = mod(floor(v_uvl.xy), 2.0);
    vec2 f = clamp(fract(v_uvl.xy), 0.0001, 0.9999);
    t = textureGrad(u_tex, vec3(f, v_uvl.z + q.x + 2.0 * q.y), dFdx(v_uvl.xy), dFdy(v_uvl.xy));
  } else t = texture(u_tex, v_uvl);
  if (u_cutout > 0.5 && t.a < 0.5) discard;
  vec3 c = t.rgb * v_color * lightOf(v_light.x, v_light.y);
  float f = clamp((v_dist - u_fog.x) / (u_fog.y - u_fog.x), 0.0, 1.0);
  o = vec4(mix(c, u_fogColor, f), u_alpha > 0.0 ? t.a * u_alpha : 1.0);
}`;

/** Entities and sprites: position, uv in a 2D texture or the layer array, a per-vertex shade. */
export const MODEL_VS = `#version 300 es
precision highp float;
layout(location=0) in vec3 a_pos;
layout(location=1) in vec3 a_uv;
layout(location=2) in float a_shade;
uniform mat4 u_viewProj;
uniform mat4 u_model;
out vec3 v_uv;
out float v_shade;
out float v_dist;
void main() {
  vec4 p = u_model * vec4(a_pos, 1.0);
  gl_Position = u_viewProj * p;
  v_uv = a_uv;
  v_shade = a_shade;
  v_dist = length(p.xyz);
}`;

export const MODEL_FS = `#version 300 es
precision highp float;
precision highp sampler2DArray;
uniform sampler2D u_tex2d;
uniform sampler2DArray u_texArr;
uniform float u_useArray;
uniform vec4 u_tint;
uniform vec4 u_overlay;
uniform vec2 u_light;
uniform vec3 u_fogColor;
uniform vec2 u_fog;
in vec3 v_uv;
in float v_shade;
in float v_dist;
out vec4 o;
${LIGHT}
void main() {
  vec4 t = u_useArray > 0.5 ? texture(u_texArr, v_uv) : texture(u_tex2d, v_uv.xy);
  if (t.a < 0.1) discard;
  // u_overlay: vanilla's hurt red, mixed in before the light (LivingEntityRenderer.method_10252)
  vec3 c = mix(t.rgb * u_tint.rgb * v_shade, u_overlay.rgb, u_overlay.a) * lightOf(u_light.x, u_light.y);
  float f = clamp((v_dist - u_fog.x) / (u_fog.y - u_fog.x), 0.0, 1.0);
  o = vec4(mix(c, u_fogColor, f), t.a * u_tint.a);
}`;

/** Screen-space textured quads (nametags): position in clip space supplied by the model matrix. */
export const SKY_VS = `#version 300 es
precision highp float;
layout(location=0) in vec2 a_pos;
uniform mat4 u_invViewProj;
out vec3 v_dir;
void main() {
  gl_Position = vec4(a_pos, 0.9999, 1.0);
  vec4 w = u_invViewProj * vec4(a_pos, 1.0, 1.0);
  v_dir = w.xyz / w.w;
}`;

export const SKY_FS = `#version 300 es
precision highp float;
uniform vec3 u_skyTop;
uniform vec3 u_fogColor;
uniform vec3 u_sunDir;
uniform float u_daylight;
in vec3 v_dir;
out vec4 o;
void main() {
  vec3 d = normalize(v_dir);
  float h = clamp(d.y, -1.0, 1.0);
  vec3 c = mix(u_fogColor, u_skyTop, smoothstep(-0.05, 0.45, h));
  // sun glow and disc
  float s = max(dot(d, u_sunDir), 0.0);
  c += vec3(1.0, 0.85, 0.55) * pow(s, 64.0) * 0.6 * u_daylight;
  float disc = smoothstep(0.9993, 0.9996, s);
  c = mix(c, vec3(1.0, 0.97, 0.85), disc * clamp(u_daylight * 1.5, 0.0, 1.0));
  // moon: the opposite side, a cool disc
  float m = max(dot(d, -u_sunDir), 0.0);
  c = mix(c, vec3(0.85, 0.88, 0.95), smoothstep(0.9994, 0.9997, m) * (1.0 - u_daylight));
  if (h < -0.02) c = mix(c, u_fogColor * 0.7, smoothstep(-0.02, -0.3, h));
  o = vec4(c, 1.0);
}`;
