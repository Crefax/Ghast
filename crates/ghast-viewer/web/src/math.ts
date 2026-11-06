/** Column-major 4×4 matrices (Float32Array(16)) and the few vector helpers the renderer needs. */
export type Mat4 = Float32Array<ArrayBufferLike>;

export const mat4 = (): Mat4 => { const m = new Float32Array(16); m[0] = m[5] = m[10] = m[15] = 1; return m; };

export function perspective(out: Mat4, fovy: number, aspect: number, near: number, far: number): Mat4 {
  const f = 1 / Math.tan(fovy / 2), nf = 1 / (near - far);
  out.fill(0);
  out[0] = f / aspect; out[5] = f; out[10] = (far + near) * nf; out[11] = -1; out[14] = 2 * far * near * nf;
  return out;
}

export function multiply(out: Mat4, a: Mat4, b: Mat4): Mat4 {
  const r = new Float32Array(16);
  for (let i = 0; i < 4; i++) for (let j = 0; j < 4; j++) {
    r[j * 4 + i] = a[i] * b[j * 4] + a[4 + i] * b[j * 4 + 1] + a[8 + i] * b[j * 4 + 2] + a[12 + i] * b[j * 4 + 3];
  }
  out.set(r);
  return out;
}

/**
 * View matrix of a camera at the origin (positions are camera-relative) looking along yaw/pitch in
 * The wire's convention (protocol.ts): yaw 0 looks to -z, yaw π/2 to -x; pitch > 0 looks up.
 */
export function viewFromYawPitch(out: Mat4, yaw: number, pitch: number): Mat4 {
  // rotate the world by -pitch around x, then by -yaw... built directly: rows are right, up, -forward
  const cy = Math.cos(yaw), sy = Math.sin(yaw), cp = Math.cos(pitch), sp = Math.sin(pitch);
  const fx = -sy * cp, fy = sp, fz = -cy * cp;
  const rx = cy, ry = 0, rz = -sy; // right = forward × up (normalized for yaw only)
  const ux = ry * fz - rz * fy, uy = rz * fx - rx * fz, uz = rx * fy - ry * fx;
  out[0] = rx; out[4] = ry; out[8] = rz; out[12] = 0;
  out[1] = ux; out[5] = uy; out[9] = uz; out[13] = 0;
  out[2] = -fx; out[6] = -fy; out[10] = -fz; out[14] = 0;
  out[3] = 0; out[7] = 0; out[11] = 0; out[15] = 1;
  return out;
}

export function forward(yaw: number, pitch: number): [number, number, number] {
  const cp = Math.cos(pitch);
  return [-Math.sin(yaw) * cp, Math.sin(pitch), -Math.cos(yaw) * cp];
}

/** Model matrix: translate, rotate about y (yaw, wire convention), then x (pitch), then scale. */
export function modelMatrix(out: Mat4, x: number, y: number, z: number, yaw: number, pitch = 0, scale = 1): Mat4 {
  const cy = Math.cos(yaw), sy = Math.sin(yaw), cp = Math.cos(pitch), sp = Math.sin(pitch);
  // R = Ry(yaw) * Rx(pitch)
  out[0] = cy * scale; out[1] = 0; out[2] = -sy * scale; out[3] = 0;
  out[4] = sy * sp * scale; out[5] = cp * scale; out[6] = cy * sp * scale; out[7] = 0;
  out[8] = sy * cp * scale; out[9] = -sp * scale; out[10] = cy * cp * scale; out[11] = 0;
  out[12] = x; out[13] = y; out[14] = z; out[15] = 1;
  return out;
}

/** Frustum planes (a, b, c, d) × 6 from a view-projection matrix. */
export function frustum(m: Mat4, out: Float32Array = new Float32Array(24)): Float32Array {
  const p = (i: number, a: number, b: number, c: number, d: number) => {
    const l = Math.hypot(a, b, c) || 1;
    out[i * 4] = a / l; out[i * 4 + 1] = b / l; out[i * 4 + 2] = c / l; out[i * 4 + 3] = d / l;
  };
  p(0, m[3] + m[0], m[7] + m[4], m[11] + m[8], m[15] + m[12]);
  p(1, m[3] - m[0], m[7] - m[4], m[11] - m[8], m[15] - m[12]);
  p(2, m[3] + m[1], m[7] + m[5], m[11] + m[9], m[15] + m[13]);
  p(3, m[3] - m[1], m[7] - m[5], m[11] - m[9], m[15] - m[13]);
  p(4, m[3] + m[2], m[7] + m[6], m[11] + m[10], m[15] + m[14]);
  p(5, m[3] - m[2], m[7] - m[6], m[11] - m[10], m[15] - m[14]);
  return out;
}

/** False when the box is entirely outside one plane. */
export function boxVisible(planes: Float32Array<ArrayBufferLike>, x0: number, y0: number, z0: number, x1: number, y1: number, z1: number): boolean {
  for (let i = 0; i < 6; i++) {
    const a = planes[i * 4], b = planes[i * 4 + 1], c = planes[i * 4 + 2], d = planes[i * 4 + 3];
    const x = a > 0 ? x1 : x0, y = b > 0 ? y1 : y0, z = c > 0 ? z1 : z0;
    if (a * x + b * y + c * z + d < 0) return false;
  }
  return true;
}

export const lerp = (a: number, b: number, t: number): number => a + (b - a) * t;

/** Shortest-way angle interpolation. */
export function lerpAngle(a: number, b: number, t: number): number {
  let d = (b - a) % (Math.PI * 2);
  if (d > Math.PI) d -= Math.PI * 2;
  if (d < -Math.PI) d += Math.PI * 2;
  return a + d * t;
}

export const clamp = (v: number, lo: number, hi: number): number => (v < lo ? lo : v > hi ? hi : v);
