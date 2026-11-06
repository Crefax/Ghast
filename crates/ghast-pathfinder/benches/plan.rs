//! Planner and simulator speed: `cargo bench -p ghast-pathfinder`.
//!
//! Worlds: a flat stone plain, and rolling terrain (heights 0..=3, slabs, soul sand, holes)
//! with a wall that forces a detour. Paths of 60-100 blocks.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use ghast_pathfinder::{Config, ControllerConfig, Goal, Pathfinder, Version, simulate};
use ghast_physics::v1_8::{MemoryWorld, memory_world};

fn flat() -> MemoryWorld {
    let mut w = memory_world();
    w.load_chunks(-8, -8, 8, 8);
    w.fill_raw([-128, 63, -128], [143, 63, 143], 1 << 4);
    w
}

fn rolling() -> MemoryWorld {
    let mut w = flat();
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
                w.fill_raw([x, 64, z], [x, 63 + h, z], 1 << 4);
            }
            match (r >> 8) % 20 {
                0 => w.set_raw(x, 64 + h, z, 44 << 4),
                1 => w.set_raw(x, 63 + h, z, 88 << 4),
                2 if h == 0 => w.set_raw(x, 63, z, 0),
                _ => {}
            }
        }
    }
    // A long wall with one gap.
    w.fill_raw([40, 64, -40], [40, 70, 110], 1 << 4);
    w.fill_raw([40, 64, 90], [40, 70, 91], 0);
    for p in [[0, 64, 0], [80, 64, 60]] {
        w.set_raw(p[0], 63, p[2], 1 << 4);
        w.fill_raw(p, [p[0], 72, p[2]], 0);
    }
    w
}

fn bench(c: &mut Criterion) {
    let pf = Pathfinder::new(Config::default());
    let flat = flat();
    let rolling = rolling();
    // Warm the per-version tables and measured costs.
    let _ = pf.plan(Version::V1_8, &flat, [0, 64, 0], &Goal::Block([3, 64, 0]));

    let cases: [(&str, &MemoryWorld, [i32; 3]); 3] = [
        ("flat_100_straight", &flat, [100, 64, 0]),
        ("flat_70_diagonal", &flat, [50, 64, 50]),
        ("rolling_detour", &rolling, [80, 64, 60]),
    ];
    for (name, w, to) in cases {
        let p = pf
            .plan(Version::V1_8, w, [0, 64, 0], &Goal::Block(to))
            .unwrap();
        let t = std::time::Instant::now();
        let n = 20;
        for _ in 0..n {
            black_box(
                pf.plan(Version::V1_8, w, [0, 64, 0], &Goal::Block(to))
                    .unwrap(),
            );
        }
        let per = t.elapsed().as_secs_f64() / f64::from(n);
        eprintln!(
            "{name}: {} steps, {} nodes expanded, {:.2} ms per plan, {:.0} k nodes/s, partial {}",
            p.steps.len(),
            p.expanded,
            per * 1e3,
            p.expanded as f64 / per / 1e3,
            p.partial
        );
        c.bench_function(&format!("plan/{name}"), |b| {
            b.iter(|| pf.plan(Version::V1_8, black_box(w), [0, 64, 0], &Goal::Block(to)))
        });
        let cfg = ControllerConfig::default();
        let r = simulate(&p, w, &cfg);
        eprintln!("{name}: simulated {:?} in {} ticks", r.outcome, r.ticks);
        c.bench_function(&format!("simulate/{name}"), |b| {
            b.iter(|| simulate(black_box(&p), w, &cfg))
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
