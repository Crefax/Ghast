//! Planner and simulator speed: `cargo bench -p ghast-pathfinder`.
//!
//! Worlds: a flat stone plain, and rolling terrain (heights 0..=3, slabs, soul sand, holes)
//! with a wall that forces a detour. Paths of 60-100 blocks. The same worlds in 1.8.9, 1.16.5
//! and 26.1 (`plan/<case>/<version>`, ...).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use ghast_pathfinder::{
    BlockSource, Budget, Config, ControllerConfig, FollowConfig, Goal, Pathfinder, Simulation,
    Version, simulate,
};

/// A world of one version.
enum World {
    V18(ghast_physics::v1_8::MemoryWorld),
    V16(ghast_physics::v1_16_5::MemoryWorld),
    #[cfg(feature = "v26_1")]
    V26(ghast_physics::modern::MemoryWorld26),
}

impl BlockSource for World {
    #[inline]
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        match self {
            World::V18(w) => w.raw_block(x, y, z),
            World::V16(w) => w.raw_block(x, y, z),
            #[cfg(feature = "v26_1")]
            World::V26(w) => w.raw_block(x, y, z),
        }
    }

    fn is_column_loaded(&self, cx: i32, cz: i32) -> bool {
        match self {
            World::V18(w) => w.is_column_loaded(cx, cz),
            World::V16(w) => w.is_column_loaded(cx, cz),
            #[cfg(feature = "v26_1")]
            World::V26(w) => w.is_column_loaded(cx, cz),
        }
    }
}

/// Stone, bottom slab and soul sand of a version.
fn ids(v: Version) -> (u16, u16, u16) {
    match v {
        Version::V1_8 => (1 << 4, 44 << 4, 88 << 4),
        #[cfg(feature = "v26_1")]
        Version::V26_1 => {
            use ghast_data::v26_1::BlockState;
            let id = |s: &str| BlockState::parse(s).unwrap().id();
            (
                id("stone"),
                id("smooth_stone_slab[type=bottom]"),
                id("soul_sand"),
            )
        }
        _ => {
            use ghast_data::v1_16_5::Block;
            (
                Block::STONE.default_state().id(),
                Block::SMOOTH_STONE_SLAB.default_state().id(),
                Block::SOUL_SAND.default_state().id(),
            )
        }
    }
}

impl World {
    fn new(v: Version) -> World {
        match v {
            Version::V1_8 => World::V18(ghast_physics::v1_8::memory_world()),
            #[cfg(feature = "v26_1")]
            Version::V26_1 => World::V26(ghast_physics::modern::memory_world_26_1()),
            _ => World::V16(ghast_physics::v1_16_5::memory_world()),
        }
    }

    fn version(&self) -> Version {
        match self {
            World::V18(_) => Version::V1_8,
            World::V16(_) => Version::V1_16_5,
            #[cfg(feature = "v26_1")]
            World::V26(_) => Version::V26_1,
        }
    }

    fn load(&mut self, r: i32) {
        match self {
            World::V18(w) => w.load_chunks(-r, -r, r, r),
            World::V16(w) => w.load_chunks(-r, -r, r, r),
            #[cfg(feature = "v26_1")]
            World::V26(w) => w.load_chunks(-r, -r, r, r),
        }
    }

    fn fill(&mut self, a: [i32; 3], b: [i32; 3], raw: u16) {
        match self {
            World::V18(w) => w.fill_raw(a, b, raw),
            World::V16(w) => w.fill_raw(a, b, raw),
            #[cfg(feature = "v26_1")]
            World::V26(w) => w.fill_raw(a, b, raw),
        }
    }

    fn set(&mut self, x: i32, y: i32, z: i32, raw: u16) {
        self.fill([x, y, z], [x, y, z], raw);
    }

    fn plan(&self, pf: &Pathfinder, to: [i32; 3]) -> ghast_pathfinder::Path {
        pf.plan(self.version(), self, [0, 64, 0], &Goal::Block(to))
            .unwrap()
    }
}

fn flat(v: Version) -> World {
    let mut w = World::new(v);
    w.load(8);
    w.fill([-128, 63, -128], [143, 63, 143], ids(v).0);
    w
}

fn rolling(v: Version) -> World {
    let (stone, slab, soul) = ids(v);
    let mut w = flat(v);
    let mut s = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for x in -40..=110 {
        for z in -40..=110 {
            let r = rnd();
            let h = (r % 3) as i32;
            if h > 0 {
                w.fill([x, 64, z], [x, 63 + h, z], stone);
            }
            match (r >> 8) % 20 {
                0 => w.set(x, 64 + h, z, slab),
                1 => w.set(x, 63 + h, z, soul),
                2 if h == 0 => w.set(x, 63, z, 0),
                _ => {}
            }
        }
    }
    // A long wall with one gap.
    w.fill([40, 64, -40], [40, 70, 110], stone);
    w.fill([40, 64, 90], [40, 70, 91], 0);
    for p in [[0, 64, 0], [80, 64, 60]] {
        w.set(p[0], 63, p[2], stone);
        w.fill(p, [p[0], 72, p[2]], 0);
    }
    w
}

fn versions() -> Vec<(Version, &'static str)> {
    vec![
        (Version::V1_8, "1.8.9"),
        (Version::V1_16_5, "1.16.5"),
        #[cfg(feature = "v26_1")]
        (Version::V26_1, "26.1"),
    ]
}

fn bench(c: &mut Criterion) {
    let pf = Pathfinder::new(Config::default());
    for (v, vname) in versions() {
        let flat = flat(v);
        let rolling = rolling(v);
        // Warm the per-version tables and measured costs.
        let _ = flat.plan(&pf, [3, 64, 0]);
        let cases: [(&str, &World, [i32; 3]); 3] = [
            ("flat_100_straight", &flat, [100, 64, 0]),
            ("flat_70_diagonal", &flat, [50, 64, 50]),
            ("rolling_detour", &rolling, [80, 64, 60]),
        ];
        for (name, w, to) in cases {
            let p = w.plan(&pf, to);
            let t = std::time::Instant::now();
            let n = 20;
            for _ in 0..n {
                black_box(w.plan(&pf, to));
            }
            let per = t.elapsed().as_secs_f64() / f64::from(n);
            eprintln!(
                "{vname} {name}: {} steps, {} nodes expanded, {:.2} ms per plan, {:.0} k nodes/s, \
                 partial {}",
                p.steps.len(),
                p.expanded,
                per * 1e3,
                p.expanded as f64 / per / 1e3,
                p.partial
            );
            c.bench_function(&format!("plan/{name}/{vname}"), |b| {
                b.iter(|| pf.plan(v, black_box(w), [0, 64, 0], &Goal::Block(to)))
            });
            // Per game tick with the follower's default budget.
            let fc = FollowConfig::default();
            let mut search = pf.search(v, [0, 64, 0], Goal::Block(to), Default::default());
            let (mut slices, mut worst) = (0, std::time::Duration::ZERO);
            loop {
                let t = std::time::Instant::now();
                let r = search.step(w, fc.budget);
                worst = worst.max(t.elapsed());
                slices += 1;
                if r.is_some() {
                    break;
                }
            }
            let mut sim = Simulation::new(&p, &fc.controller);
            let (mut sim_slices, mut sim_worst) = (0, std::time::Duration::ZERO);
            loop {
                let t = std::time::Instant::now();
                let r = sim.step(w, fc.verify_ticks, fc.budget.time).is_some();
                sim_worst = sim_worst.max(t.elapsed());
                sim_slices += 1;
                if r {
                    break;
                }
            }
            eprintln!(
                "{vname} {name}: follower budget {:?}: search {slices} ticks (worst {:.2} ms), \
                 verification {sim_slices} ticks (worst {:.2} ms)",
                fc.budget,
                worst.as_secs_f64() * 1e3,
                sim_worst.as_secs_f64() * 1e3
            );
            c.bench_function(&format!("search_slice_2000/{name}/{vname}"), |b| {
                b.iter(|| {
                    let mut s = pf.search(v, [0, 64, 0], Goal::Block(to), Default::default());
                    s.step(black_box(w), Budget::nodes(2000))
                })
            });
            let cfg = ControllerConfig::default();
            let r = simulate(&p, w, &cfg);
            eprintln!(
                "{vname} {name}: simulated {:?} in {} ticks",
                r.outcome, r.ticks
            );
            c.bench_function(&format!("simulate/{name}/{vname}"), |b| {
                b.iter(|| simulate(black_box(&p), w, &cfg))
            });
        }
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
