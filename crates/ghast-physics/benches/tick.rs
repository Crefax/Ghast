//! Cost of one player tick (`cargo bench -p ghast-physics`): 1.8.9 (`v1_8_tick`) and 1.16.5
//! (`v1_16_5_tick`).
//!
//! Each iteration replays a fixed 20-tick input sequence from the same start state, so the
//! numbers are per 20 ticks (criterion reports the per-element time with `Throughput`).

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use ghast_data::v1_8::BlockState;
use ghast_physics::v1_8::{self, PlayerState};
use ghast_physics::{BlockView, Keys, Packets, TickInput};
use std::hint::black_box;

/// A dense array over a fixed region: the cheapest possible block lookup, to separate the cost
/// of the physics from the cost of the world representation.
struct DenseWorld {
    origin: [i32; 2],
    side: i32,
    blocks: Vec<BlockState>,
}

impl DenseWorld {
    fn copy_of(w: &v1_8::MemoryWorld, origin: [i32; 2], side: i32) -> DenseWorld {
        let mut blocks = Vec::with_capacity((side * side * 256) as usize);
        for y in 0..256 {
            for z in 0..side {
                for x in 0..side {
                    blocks.push(w.block(origin[0] + x, y, origin[1] + z));
                }
            }
        }
        DenseWorld {
            origin,
            side,
            blocks,
        }
    }
}

impl BlockView for DenseWorld {
    type State = BlockState;

    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> BlockState {
        let (lx, lz) = (x - self.origin[0], z - self.origin[1]);
        if lx < 0 || lz < 0 || lx >= self.side || lz >= self.side {
            return BlockState::AIR;
        }
        self.blocks[((y * self.side + lz) * self.side + lx) as usize]
    }

    #[inline]
    fn is_chunk_loaded(&self, _: i32, _: i32) -> bool {
        true
    }
}

const TICKS: usize = 20;

fn raw(id: u16, meta: u16) -> u16 {
    (id << 4) | meta
}

/// Flat stone, player walking forward on it (warmed up so it is on the ground and moving).
fn flat() -> (v1_8::MemoryWorld, PlayerState, Vec<TickInput>) {
    let mut w = v1_8::memory_world();
    w.load_chunks(-4, -4, 4, 4);
    w.fill_raw([-64, 63, -64], [63, 63, 63], raw(1, 0));
    let input = TickInput {
        keys: Keys {
            forward: true,
            ..Keys::NONE
        },
        yaw: -45.0,
        pitch: 0.0,
        using_item: false,
    };
    let mut p = PlayerState::new([0.5, 64.0, 0.5], -45.0, 0.0);
    let mut out = Packets::new();
    for _ in 0..10 {
        out.clear();
        v1_8::tick(&mut p, &input, &w, &mut out);
    }
    (w, p, vec![input; TICKS])
}

/// A cluttered corner: sneaking on a ledge edge (edge-guard probing every tick) while pushing into
/// fences, panes, a wall, stairs and slabs (neighbour-dependent shapes, step-up attempts), with a
/// ceiling of slabs above, jumping every few ticks.
fn busy() -> (v1_8::MemoryWorld, PlayerState, Vec<TickInput>) {
    let mut w = v1_8::memory_world();
    w.load_chunks(-2, -2, 2, 2);
    w.fill_raw([-8, 62, -8], [8, 63, 8], raw(1, 0));
    // A 1-block-wide ledge along x = 0..=2, open drop to the west.
    w.fill_raw([-8, 63, -8], [-1, 63, 8], raw(0, 0));
    w.fill_raw([3, 64, -3], [3, 64, 3], raw(85, 0)); // fence row
    w.fill_raw([3, 65, -3], [3, 65, 3], raw(101, 0)); // iron bars above
    w.fill_raw([0, 64, 3], [2, 64, 3], raw(139, 0)); // wall
    w.set_raw(2, 64, 2, raw(53, 2)); // stairs, two of them forming a corner
    w.set_raw(1, 64, 2, raw(53, 0));
    w.set_raw(2, 64, 1, raw(44, 0)); // bottom slab
    w.set_raw(0, 64, 1, raw(102, 0)); // glass pane
    w.fill_raw([-2, 67, -2], [4, 67, 4], raw(44, 8)); // top slab ceiling
    let mut inputs = Vec::with_capacity(TICKS);
    for t in 0..TICKS {
        inputs.push(TickInput {
            keys: Keys {
                forward: true,
                left: t % 3 == 0,
                sneak: t % 7 != 6,
                jump: t % 5 == 4,
                ..Keys::NONE
            },
            yaw: -60.0 + (t as f32) * 1.5,
            pitch: 10.0,
            using_item: false,
        });
    }
    let mut p = PlayerState::new([1.5, 64.0, 1.5], -60.0, 10.0);
    let mut out = Packets::new();
    for _ in 0..5 {
        out.clear();
        v1_8::tick(&mut p, &inputs[0], &w, &mut out);
    }
    (w, p, inputs)
}

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("v1_8_tick");
    g.throughput(Throughput::Elements(TICKS as u64));
    for (name, (w, start, inputs)) in [("walk_flat", flat()), ("busy_collision", busy())] {
        g.bench_function(format!("{name}/memory_world"), |b| {
            run(b, &w, start, &inputs)
        });
        let dense = DenseWorld::copy_of(&w, [-32, -32], 64);
        g.bench_function(format!("{name}/dense_array"), |b| {
            run(b, &dense, start, &inputs);
        });
    }
    g.finish();
}

fn run<W: BlockView<State = BlockState>>(
    b: &mut criterion::Bencher<'_>,
    w: &W,
    start: PlayerState,
    inputs: &[TickInput],
) {
    let mut out = Packets::new();
    b.iter(|| {
        let mut p = start;
        for input in inputs {
            out.clear();
            v1_8::tick(&mut p, input, black_box(w), &mut out);
        }
        black_box((p, out.len()))
    });
}

mod modern {
    //! 1.16.5: the same two shapes of work, plus a swimming case (fluid pushing, pose changes).
    use super::{Criterion, TICKS, Throughput, black_box};
    use ghast_data::v1_16_5::{Block, Property};
    use ghast_physics::v1_16_5::{self, MemoryWorld, PlayerState};
    use ghast_physics::{Keys, Packets, TickInput};

    fn st(name: &str, props: &[(&str, &str)]) -> u16 {
        let b = Block::from_name(name).unwrap();
        let props: Vec<(Property, &str)> = props
            .iter()
            .map(|&(k, v)| (Property::from_name(k).unwrap(), v))
            .collect();
        b.state_with(&props).unwrap().id()
    }

    fn warm(w: &MemoryWorld, mut p: PlayerState, input: &TickInput, n: usize) -> PlayerState {
        let mut out = Packets::new();
        for _ in 0..n {
            out.clear();
            v1_16_5::tick(&mut p, input, w, &mut out);
        }
        p
    }

    fn forward(yaw: f32) -> TickInput {
        TickInput {
            keys: Keys {
                forward: true,
                ..Keys::NONE
            },
            yaw,
            pitch: 0.0,
            using_item: false,
        }
    }

    fn flat() -> (MemoryWorld, PlayerState, Vec<TickInput>) {
        let mut w = v1_16_5::memory_world();
        w.load_chunks(-4, -4, 4, 4);
        w.fill_raw([-64, 63, -64], [63, 63, 63], st("stone", &[]));
        let input = forward(-45.0);
        let p = warm(
            &w,
            PlayerState::new([0.5, 64.0, 0.5], -45.0, 0.0),
            &input,
            10,
        );
        (w, p, vec![input; TICKS])
    }

    fn busy() -> (MemoryWorld, PlayerState, Vec<TickInput>) {
        let mut w = v1_16_5::memory_world();
        w.load_chunks(-2, -2, 2, 2);
        w.fill_raw([-8, 62, -8], [8, 63, 8], st("stone", &[]));
        w.fill_raw([-8, 63, -8], [-1, 63, 8], 0);
        w.fill_raw(
            [3, 64, -3],
            [3, 64, 3],
            st("oak_fence", &[("north", "true"), ("south", "true")]),
        );
        w.fill_raw(
            [3, 65, -3],
            [3, 65, 3],
            st("iron_bars", &[("north", "true"), ("south", "true")]),
        );
        w.fill_raw(
            [0, 64, 3],
            [2, 64, 3],
            st("cobblestone_wall", &[("east", "low"), ("west", "low")]),
        );
        w.set_raw(
            2,
            64,
            2,
            st(
                "oak_stairs",
                &[("facing", "south"), ("shape", "outer_left")],
            ),
        );
        w.set_raw(1, 64, 2, st("oak_stairs", &[("facing", "east")]));
        w.set_raw(2, 64, 1, st("smooth_stone_slab", &[]));
        w.set_raw(
            0,
            64,
            1,
            st("glass_pane", &[("north", "true"), ("south", "true")]),
        );
        w.fill_raw(
            [-2, 67, -2],
            [4, 67, 4],
            st("smooth_stone_slab", &[("type", "top")]),
        );
        let mut inputs = Vec::with_capacity(TICKS);
        for t in 0..TICKS {
            inputs.push(TickInput {
                keys: Keys {
                    forward: true,
                    left: t % 3 == 0,
                    sneak: t % 7 != 6,
                    jump: t % 5 == 4,
                    ..Keys::NONE
                },
                yaw: -60.0 + (t as f32) * 1.5,
                pitch: 10.0,
                using_item: false,
            });
        }
        let p = warm(
            &w,
            PlayerState::new([1.5, 64.0, 1.5], -60.0, 10.0),
            &inputs[0],
            5,
        );
        (w, p, inputs)
    }

    fn swim() -> (MemoryWorld, PlayerState, Vec<TickInput>) {
        let mut w = v1_16_5::memory_world();
        w.load_chunks(-2, -2, 2, 2);
        w.fill_raw([-16, 55, -16], [15, 55, 15], st("stone", &[]));
        w.fill_raw([-16, 56, -16], [15, 63, 15], st("water", &[]));
        w.fill_raw([-3, 56, -3], [3, 58, 3], st("kelp", &[]));
        let input = TickInput {
            keys: Keys {
                forward: true,
                sprint: true,
                ..Keys::NONE
            },
            yaw: 30.0,
            pitch: 20.0,
            using_item: false,
        };
        let p = warm(
            &w,
            PlayerState::new([0.5, 60.0, 0.5], 30.0, 20.0),
            &input,
            10,
        );
        (w, p, vec![input; TICKS])
    }

    pub fn bench(c: &mut Criterion) {
        let mut g = c.benchmark_group("v1_16_5_tick");
        g.throughput(Throughput::Elements(TICKS as u64));
        for (name, (w, start, inputs)) in [
            ("walk_flat", flat()),
            ("busy_collision", busy()),
            ("swim", swim()),
        ] {
            g.bench_function(format!("{name}/memory_world"), |b| {
                let mut out = Packets::new();
                b.iter(|| {
                    let mut p = start;
                    for input in &inputs {
                        out.clear();
                        v1_16_5::tick(&mut p, input, black_box(&w), &mut out);
                    }
                    black_box((p, out.len()))
                });
            });
        }
        g.finish();
    }
}

criterion_group!(benches, bench, modern::bench);
criterion_main!(benches);
