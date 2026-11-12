//! Biome climate (26.1): what `Biome.getPrecipitationAt` reads — the precipitation flag, the
//! base temperature, the frozen-ocean temperature modifier — and the height-adjusted temperature
//! with the game's fixed-seed temperature noises (`Biome.TEMPERATURE_NOISE`,
//! `FROZEN_TEMPERATURE_NOISE`, `BIOME_INFO_NOISE`: `PerlinSimplexNoise` over `SimplexNoise`
//! seeded by a `LegacyRandomSource`). Checked against the real classes:
//! `tests/golden/26.1/biome_climate.jsonl.gz`.

use std::sync::OnceLock;

/// `Biome.Precipitation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Precipitation {
    /// No precipitation.
    None,
    /// Rain.
    Rain,
    /// Snow.
    Snow,
}

/// A biome's `ClimateSettings` as precipitation reads them.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Climate {
    /// `has_precipitation`.
    pub has_precipitation: bool,
    /// `temperature`.
    pub temperature: f32,
    /// `temperature_modifier` is `frozen` (frozen oceans).
    pub frozen: bool,
}

/// The vanilla 26.1 biomes' climate (name without `minecraft:`, precipitation, temperature
/// bits, frozen modifier), as the real client holds them after a vanilla configuration.
const VANILLA: &[(&str, bool, u32, bool)] = &[
    ("badlands", false, 0x40000000, false),
    ("bamboo_jungle", true, 0x3f733333, false),
    ("basalt_deltas", false, 0x40000000, false),
    ("beach", true, 0x3f4ccccd, false),
    ("birch_forest", true, 0x3f19999a, false),
    ("cherry_grove", true, 0x3f000000, false),
    ("cold_ocean", true, 0x3f000000, false),
    ("crimson_forest", false, 0x40000000, false),
    ("dark_forest", true, 0x3f333333, false),
    ("deep_cold_ocean", true, 0x3f000000, false),
    ("deep_dark", true, 0x3f4ccccd, false),
    ("deep_frozen_ocean", true, 0x3f000000, true),
    ("deep_lukewarm_ocean", true, 0x3f000000, false),
    ("deep_ocean", true, 0x3f000000, false),
    ("desert", false, 0x40000000, false),
    ("dripstone_caves", true, 0x3f4ccccd, false),
    ("end_barrens", false, 0x3f000000, false),
    ("end_highlands", false, 0x3f000000, false),
    ("end_midlands", false, 0x3f000000, false),
    ("eroded_badlands", false, 0x40000000, false),
    ("flower_forest", true, 0x3f333333, false),
    ("forest", true, 0x3f333333, false),
    ("frozen_ocean", true, 0x0, true),
    ("frozen_peaks", true, 0xbf333333, false),
    ("frozen_river", true, 0x0, false),
    ("grove", true, 0xbe4ccccd, false),
    ("ice_spikes", true, 0x0, false),
    ("jagged_peaks", true, 0xbf333333, false),
    ("jungle", true, 0x3f733333, false),
    ("lukewarm_ocean", true, 0x3f000000, false),
    ("lush_caves", true, 0x3f000000, false),
    ("mangrove_swamp", true, 0x3f4ccccd, false),
    ("meadow", true, 0x3f000000, false),
    ("mushroom_fields", true, 0x3f666666, false),
    ("nether_wastes", false, 0x40000000, false),
    ("ocean", true, 0x3f000000, false),
    ("old_growth_birch_forest", true, 0x3f19999a, false),
    ("old_growth_pine_taiga", true, 0x3e99999a, false),
    ("old_growth_spruce_taiga", true, 0x3e800000, false),
    ("pale_garden", true, 0x3f333333, false),
    ("plains", true, 0x3f4ccccd, false),
    ("river", true, 0x3f000000, false),
    ("savanna", false, 0x40000000, false),
    ("savanna_plateau", false, 0x40000000, false),
    ("small_end_islands", false, 0x3f000000, false),
    ("snowy_beach", true, 0x3d4ccccd, false),
    ("snowy_plains", true, 0x0, false),
    ("snowy_slopes", true, 0xbe99999a, false),
    ("snowy_taiga", true, 0xbf000000, false),
    ("soul_sand_valley", false, 0x40000000, false),
    ("sparse_jungle", true, 0x3f733333, false),
    ("stony_peaks", true, 0x3f800000, false),
    ("stony_shore", true, 0x3e4ccccd, false),
    ("sunflower_plains", true, 0x3f4ccccd, false),
    ("swamp", true, 0x3f4ccccd, false),
    ("taiga", true, 0x3e800000, false),
    ("the_end", false, 0x3f000000, false),
    ("the_void", false, 0x3f000000, false),
    ("warm_ocean", true, 0x3f000000, false),
    ("warped_forest", false, 0x40000000, false),
    ("windswept_forest", true, 0x3e4ccccd, false),
    ("windswept_gravelly_hills", true, 0x3e4ccccd, false),
    ("windswept_hills", true, 0x3e4ccccd, false),
    ("windswept_savanna", false, 0x40000000, false),
    ("wooded_badlands", false, 0x40000000, false),
];

impl Climate {
    /// A climate from its parts.
    #[must_use]
    pub const fn new(has_precipitation: bool, temperature: f32, frozen: bool) -> Climate {
        Climate {
            has_precipitation,
            temperature,
            frozen,
        }
    }

    /// The vanilla climate of a biome (`minecraft:` optional), for registry entries the server
    /// sent without data (the vanilla known pack).
    #[must_use]
    pub fn vanilla(name: &str) -> Option<Climate> {
        let n = name.strip_prefix("minecraft:").unwrap_or(name);
        VANILLA
            .iter()
            .find(|e| e.0 == n)
            .map(|&(_, p, t, f)| Climate::new(p, f32::from_bits(t), f))
    }

    /// `Biome.getHeightAdjustedTemperature`.
    #[must_use]
    pub fn temperature_at(&self, x: i32, y: i32, z: i32, sea_level: i32) -> f32 {
        let mut t = self.temperature;
        if self.frozen {
            t = frozen_modifier(x, z, t);
        }
        let snow_level = sea_level + 17;
        if y > snow_level {
            // TEMPERATURE_NOISE.getValue(x / 8.0F, z / 8.0F, false) * 8.0, as a float.
            let (fx, fz) = (x as f32 / 8.0, z as f32 / 8.0);
            let v = (noises().temperature.value(f64::from(fx), f64::from(fz)) * 8.0) as f32;
            t - (v + y as f32 - snow_level as f32) * 0.05 / 40.0
        } else {
            t
        }
    }

    /// `Biome.getPrecipitationAt`.
    #[must_use]
    pub fn precipitation_at(&self, x: i32, y: i32, z: i32, sea_level: i32) -> Precipitation {
        if !self.has_precipitation {
            Precipitation::None
        } else if self.temperature_at(x, y, z, sea_level) >= 0.15 {
            Precipitation::Rain
        } else {
            Precipitation::Snow
        }
    }
}

/// `BiomeManager.getBiome`: the noise cell (quart position) whose biome the block at `x`, `y`,
/// `z` has, for the level's zoom seed (the spawn info's hashed seed).
#[must_use]
pub fn zoom_cell(seed: i64, x: i32, y: i32, z: i32) -> [i32; 3] {
    let (x, y, z) = (x - 2, y - 2, z - 2);
    let base = [x >> 2, y >> 2, z >> 2];
    let frac = [
        f64::from(x & 3) / 4.0,
        f64::from(y & 3) / 4.0,
        f64::from(z & 3) / 4.0,
    ];
    let mut best = 0;
    let mut dist = f64::INFINITY;
    for corner in 0..8 {
        let hi = [corner & 4 != 0, corner & 2 != 0, corner & 1 != 0];
        let c = |i: usize| base[i] + i32::from(hi[i]);
        let f = |i: usize| if hi[i] { frac[i] - 1.0 } else { frac[i] };
        let d = fiddled_distance(seed, c(0), c(1), c(2), f(0), f(1), f(2));
        if dist > d {
            best = corner;
            dist = d;
        }
    }
    [
        base[0] + i32::from(best & 4 != 0),
        base[1] + i32::from(best & 2 != 0),
        base[2] + i32::from(best & 1 != 0),
    ]
}

/// `LinearCongruentialGenerator.next`.
#[inline]
fn lcg(r: i64, c: i64) -> i64 {
    r.wrapping_mul(
        r.wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407),
    )
    .wrapping_add(c)
}

#[inline]
fn fiddle(r: i64) -> f64 {
    let d = f64::from((r >> 24).rem_euclid(1024) as i32) / 1024.0;
    (d - 0.5) * 0.9
}

fn fiddled_distance(seed: i64, x: i32, y: i32, z: i32, fx: f64, fy: f64, fz: f64) -> f64 {
    let (xl, yl, zl) = (i64::from(x), i64::from(y), i64::from(z));
    let mut r = lcg(seed, xl);
    r = lcg(r, yl);
    r = lcg(r, zl);
    r = lcg(r, xl);
    r = lcg(r, yl);
    r = lcg(r, zl);
    let dx = fiddle(r);
    r = lcg(r, seed);
    let dy = fiddle(r);
    r = lcg(r, seed);
    let dz = fiddle(r);
    let sq = |v: f64| v * v;
    sq(fz + dz) + sq(fy + dy) + sq(fx + dx)
}

/// `TemperatureModifier.FROZEN.modifyTemperature`.
fn frozen_modifier(x: i32, z: i32, temperature: f32) -> f32 {
    let n = noises();
    let (x, z) = (f64::from(x), f64::from(z));
    let large = n.frozen.value(x * 0.05, z * 0.05) * 7.0;
    let edge = n.biome_info.value(x * 0.2, z * 0.2);
    if large + edge < 0.3 && n.biome_info.value(x * 0.09, z * 0.09) < 0.8 {
        0.2
    } else {
        temperature
    }
}

struct Noises {
    temperature: PerlinSimplex,
    frozen: PerlinSimplex,
    biome_info: PerlinSimplex,
}

fn noises() -> &'static Noises {
    static N: OnceLock<Noises> = OnceLock::new();
    N.get_or_init(|| Noises {
        temperature: PerlinSimplex::new(1234, &[0]),
        frozen: PerlinSimplex::new(3456, &[-2, -1, 0]),
        biome_info: PerlinSimplex::new(2345, &[0]),
    })
}

/// `LegacyRandomSource` (`java.util.Random`'s generator).
struct Legacy(i64);

impl Legacy {
    const MUL: i64 = 0x5_DEEC_E66D;
    const MASK: i64 = (1 << 48) - 1;

    fn new(seed: i64) -> Legacy {
        Legacy((seed ^ Self::MUL) & Self::MASK)
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(Self::MUL).wrapping_add(0xB)) & Self::MASK;
        (self.0 >> (48 - bits)) as i32
    }

    fn next_int(&mut self, bound: i32) -> i32 {
        if bound & (bound - 1) == 0 {
            return ((i64::from(bound) * i64::from(self.next(31))) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }

    fn next_double(&mut self) -> f64 {
        let hi = i64::from(self.next(26)) << 27;
        let lo = i64::from(self.next(27));
        (hi + lo) as f64 * f64::from(f32::from_bits(0x2580_0000))
    }

    fn consume(&mut self, n: usize) {
        for _ in 0..n {
            self.next(32);
        }
    }
}

/// `SimplexNoise` (2D part).
struct Simplex {
    p: [u8; 256],
}

const GRADIENT: [[i32; 3]; 16] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
    [1, 1, 0],
    [0, -1, 1],
    [-1, 1, 0],
    [0, -1, -1],
];

impl Simplex {
    fn new(r: &mut Legacy) -> Simplex {
        // xo, yo, zo (unused by the 2D value without noise start).
        for _ in 0..3 {
            let _ = r.next_double();
        }
        let mut p = [0u8; 256];
        for (i, v) in p.iter_mut().enumerate() {
            *v = i as u8;
        }
        for i in 0..256 {
            let off = r.next_int(256 - i as i32) as usize;
            p.swap(i, off + i);
        }
        Simplex { p }
    }

    fn p(&self, x: i32) -> i32 {
        i32::from(self.p[(x & 0xFF) as usize])
    }

    fn corner(index: i32, x: f64, y: f64) -> f64 {
        let t0 = 0.5 - x * x - y * y - 0.0 * 0.0;
        if t0 < 0.0 {
            0.0
        } else {
            let t0 = t0 * t0;
            let g = GRADIENT[index as usize];
            t0 * t0 * (f64::from(g[0]) * x + f64::from(g[1]) * y + f64::from(g[2]) * 0.0)
        }
    }

    fn value(&self, xin: f64, yin: f64) -> f64 {
        let sqrt3 = 3.0f64.sqrt();
        let f2 = 0.5 * (sqrt3 - 1.0);
        let g2 = (3.0 - sqrt3) / 6.0;
        let s = (xin + yin) * f2;
        let i = (xin + s).floor() as i32;
        let j = (yin + s).floor() as i32;
        let t = f64::from(i.wrapping_add(j)) * g2;
        let x0 = xin - (f64::from(i) - t);
        let y0 = yin - (f64::from(j) - t);
        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = x0 - f64::from(i1) + g2;
        let y1 = y0 - f64::from(j1) + g2;
        let x2 = x0 - 1.0 + 2.0 * g2;
        let y2 = y0 - 1.0 + 2.0 * g2;
        let (ii, jj) = (i & 0xFF, j & 0xFF);
        let gi0 = self.p(ii + self.p(jj)) % 12;
        let gi1 = self.p(ii + i1 + self.p(jj + j1)) % 12;
        let gi2 = self.p(ii + 1 + self.p(jj + 1)) % 12;
        let n0 = Self::corner(gi0, x0, y0);
        let n1 = Self::corner(gi1, x1, y1);
        let n2 = Self::corner(gi2, x2, y2);
        70.0 * (n0 + n1 + n2)
    }
}

/// `PerlinSimplexNoise` without positive octaves (the three biome noises).
struct PerlinSimplex {
    levels: Vec<Option<Simplex>>,
    input_factor: f64,
    value_factor: f64,
}

impl PerlinSimplex {
    fn new(seed: i64, octaves: &[i32]) -> PerlinSimplex {
        let mut r = Legacy::new(seed);
        let low = -octaves[0];
        let high = *octaves.last().unwrap_or(&0);
        assert!(
            high <= 0,
            "positive octaves are not used by the biome noises"
        );
        let n = (low + high + 1) as usize;
        let zero = Simplex::new(&mut r);
        let mut levels: Vec<Option<Simplex>> = (0..n).map(|_| None).collect();
        if high >= 0 && (high as usize) < n && octaves.contains(&0) {
            levels[high as usize] = Some(zero);
        }
        for (i, level) in levels.iter_mut().enumerate().skip((high + 1) as usize) {
            if octaves.contains(&(high - i as i32)) {
                *level = Some(Simplex::new(&mut r));
            } else {
                r.consume(262);
            }
        }
        PerlinSimplex {
            levels,
            input_factor: 2f64.powi(high),
            value_factor: 1.0 / (2f64.powi(n as i32) - 1.0),
        }
    }

    fn value(&self, x: f64, y: f64) -> f64 {
        let (mut v, mut f, mut vf) = (0.0, self.input_factor, self.value_factor);
        for l in &self.levels {
            if let Some(s) = l {
                v += s.value(x * f, y * f) * vf;
            }
            f /= 2.0;
            vf *= 2.0;
        }
        v
    }
}
