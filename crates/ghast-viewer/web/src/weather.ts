/**
 * Rain and snow (GameRenderer.renderWeather, 1.8): around the camera, every column whose biome has
 * precipitation gets a vertical quad from its precipitation height (the top block that stops movement
 * or is liquid, +1) up, facing the camera, with rain.png falling fast or snow.png drifting. Cold biomes
 * (temperature below 0.15 at that height) snow. Pure maths: the page uploads the vertices.
 */

/** Columns' precipitation heights and biomes (from the mesher worker), by chunk "cx,cz": 256 bytes each, x + 16 z. */
export interface WeatherColumns { get(key: string): { top: Uint8Array; biomes: Uint8Array } | undefined }

/** Biome → [temperature, rainfall] (ModelData.biomes). */
export type Climate = Map<number, [number, number]>;

/** Biome.getTemperature: colder 0.05 per 30 blocks above y 64. */
export function temperatureAt(t: number, y: number): number {
  return y > 64 ? t - ((y - 64) * 0.05) / 30 : t;
}

/** 'rain', 'snow' or null (no precipitation: deserts, savannas, the nether). */
export function precipitation(climate: Climate, biome: number, y: number): 'rain' | 'snow' | null {
  const c = climate.get(biome);
  if (!c || c[1] <= 0) return null;
  return temperatureAt(c[0], y) < 0.15 ? 'snow' : 'rain';
}

const FLOATS = 7; // the entity renderer's vertex: pos xyz, uv + layer, shade

/**
 * Vertices (camera-relative) of the rain and snow quads around (cx, cy, cz) within `radius` blocks.
 * `ticks` drives the fall; returns how many floats of each buffer it filled.
 */
export function weatherQuads(rain: Float32Array, snow: Float32Array, cols: WeatherColumns, climate: Climate,
  cam: { x: number; y: number; z: number }, radius: number, ticks: number): { rain: number; snow: number } {
  const bx = Math.floor(cam.x), by = Math.floor(cam.y), bz = Math.floor(cam.z);
  let nr = 0, ns = 0;
  for (let z = bz - radius; z <= bz + radius; z++) for (let x = bx - radius; x <= bx + radius; x++) {
    const dx = x + 0.5 - cam.x, dz = z + 0.5 - cam.z;
    const d = Math.hypot(dx, dz);
    if (d > radius) continue;
    const col = cols.get(`${Math.floor(x / 16)},${Math.floor(z / 16)}`);
    if (!col) continue;
    const i = (x & 15) + 16 * (z & 15);
    const top = col.top[i];
    const y0 = Math.max(by - radius, top), y1 = Math.max(by + radius, top);
    if (y0 >= y1) continue;
    const kind = precipitation(climate, col.biomes[i], top);
    if (!kind) continue;
    // the quad faces the camera: across the direction to the column (vanilla's rainOffsetX / rainOffsetY)
    const ux = d > 1e-3 ? -dz / d * 0.5 : 0.5, uz = d > 1e-3 ? dx / d * 0.5 : 0;
    // a per-column offset so neighbouring columns do not fall in step (vanilla hashes x and z)
    const h = (x * x * 3121 + x * 45238971 + z * z * 418711 + z * 13761) & 31;
    const fall = kind === 'rain' ? ((ticks + h) % 32) / 32 * 3 : (ticks / 512 + h / 32) % 1;
    const side = kind === 'snow' ? (ticks * 0.01 + h * 0.37) % 1 : 0;
    const v0 = y0 * 0.25 + fall, v1 = y1 * 0.25 + fall;
    const out = kind === 'rain' ? rain : snow;
    let n = kind === 'rain' ? nr : ns;
    if (n + 6 * FLOATS > out.length) continue;
    const ax = dx - ux, az = dz - uz, bx2 = dx + ux, bz2 = dz + uz;
    const yy0 = y0 - cam.y, yy1 = y1 - cam.y;
    const corners: Array<[number, number, number, number, number]> = [
      [ax, yy0, az, side, v0], [bx2, yy0, bz2, side + 1, v0], [bx2, yy1, bz2, side + 1, v1],
      [ax, yy0, az, side, v0], [bx2, yy1, bz2, side + 1, v1], [ax, yy1, az, side, v1],
    ];
    for (const [px, py, pz, u, v] of corners) {
      out[n++] = px; out[n++] = py; out[n++] = pz; out[n++] = u; out[n++] = -v; out[n++] = 0; out[n++] = 1;
    }
    if (kind === 'rain') nr = n; else ns = n;
  }
  return { rain: nr, snow: ns };
}

/**
 * World.method_3631 / World.method_3597 under rain and thunder: the sky turns grey
 * (1 − 0.75 · strength mixed towards its luminance) and the daylight drops by up to 5/16 each.
 */
export function rainSky(c: [number, number, number], rain: number, thunder: number): [number, number, number] {
  let [r, g, b] = c;
  for (const s of [rain, thunder]) {
    if (s <= 0) continue;
    const grey = (r * 0.3 + g * 0.59 + b * 0.11) * 0.6, k = 1 - s * 0.75;
    r = r * k + grey * (1 - k); g = g * k + grey * (1 - k); b = b * k + grey * (1 - k);
  }
  return [r, g, b];
}
export const rainDaylight = (rain: number, thunder: number): number => (1 - (rain * 5) / 16) * (1 - (thunder * 5) / 16);

/** The precipitation height of every column (x + 16 z) of a column's sections: top block that stops movement or is liquid, + 1. */
export function precipitationHeights(sections: ReadonlyArray<{ states: Uint16Array } | null>, stops: (state: number) => boolean, out = new Uint8Array(256)): Uint8Array {
  for (let i = 0; i < 256; i++) {
    const x = i & 15, z = i >> 4;
    let top = 0;
    scan: for (let sy = 15; sy >= 0; sy--) {
      const s = sections[sy];
      if (!s) continue;
      for (let ly = 15; ly >= 0; ly--) {
        if (stops(s.states[x + 16 * (z + 16 * ly)])) { top = sy * 16 + ly + 1; break scan; }
      }
    }
    out[i] = Math.min(255, top);
  }
  return out;
}
