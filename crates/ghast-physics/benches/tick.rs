//! Cost of one player tick (`cargo bench -p ghast-physics`): 1.8.9 (`v1_8_tick`), 1.16.5
//! (`v1_16_5_tick`) and the modern core with 26.1 (`modern_tick`).
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

/// A world shaped like `ghast-world`'s: columns in a hash map with a cheap integer hasher,
/// 16-high sections that are uniform or palette-indexed (one byte per block), states decoded from
/// raw ids. Built from another world's region; every other column is unloaded.
mod column_world {
    use ghast_physics::BlockView;
    use std::collections::HashMap;
    use std::hash::{BuildHasherDefault, Hasher};

    #[derive(Default)]
    pub struct ChunkHasher(u64);

    impl Hasher for ChunkHasher {
        #[inline]
        fn finish(&self) -> u64 {
            let h = (self.0 ^ (self.0 >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            h ^ (h >> 32)
        }

        #[inline]
        fn write(&mut self, bytes: &[u8]) {
            for &b in bytes {
                self.0 = (self.0 << 8) | u64::from(b);
            }
        }

        #[inline]
        fn write_u64(&mut self, v: u64) {
            self.0 = v;
        }
    }

    enum Section {
        Uniform(u16),
        Indexed {
            palette: Vec<u16>,
            idx: Box<[u8; 4096]>,
        },
    }

    impl Section {
        #[inline]
        fn get(&self, i: usize) -> u16 {
            match self {
                Section::Uniform(s) => *s,
                Section::Indexed { palette, idx } => palette[usize::from(idx[i & 4095])],
            }
        }
    }

    pub struct ColumnWorld<S> {
        map: HashMap<u64, Box<[Option<Section>]>, BuildHasherDefault<ChunkHasher>>,
        min_y: i32,
        sections: usize,
        decode: fn(u16) -> S,
    }

    #[inline]
    fn key(x: i32, z: i32) -> u64 {
        u64::from(x as u32) | (u64::from(z as u32) << 32)
    }

    impl<S: Copy> ColumnWorld<S> {
        /// Copies the chunk rectangle `[cx0, cz0] ..= [cx1, cz1]` of `w` (states encoded with `raw`).
        pub fn of<W: BlockView<State = S>>(
            w: &W,
            c0: [i32; 2],
            c1: [i32; 2],
            min_y: i32,
            sections: usize,
            raw: fn(S) -> u16,
            decode: fn(u16) -> S,
        ) -> Self {
            let mut map: HashMap<u64, Box<[Option<Section>]>, BuildHasherDefault<ChunkHasher>> =
                HashMap::default();
            for cx in c0[0]..=c1[0] {
                for cz in c0[1]..=c1[1] {
                    let mut col = Vec::with_capacity(sections);
                    for sy in 0..sections {
                        let mut ids = [0u16; 4096];
                        for (i, id) in ids.iter_mut().enumerate() {
                            let (x, z, y) = (i & 15, (i >> 4) & 15, i >> 8);
                            *id = raw(w.block(
                                cx * 16 + x as i32,
                                min_y + (sy * 16 + y) as i32,
                                cz * 16 + z as i32,
                            ));
                        }
                        let mut palette: Vec<u16> = ids.to_vec();
                        palette.sort_unstable();
                        palette.dedup();
                        col.push(if palette == [0] {
                            None
                        } else if palette.len() == 1 {
                            Some(Section::Uniform(palette[0]))
                        } else {
                            let mut idx = Box::new([0u8; 4096]);
                            for (i, id) in ids.iter().enumerate() {
                                idx[i] = palette.binary_search(id).unwrap() as u8;
                            }
                            Some(Section::Indexed { palette, idx })
                        });
                    }
                    map.insert(key(cx, cz), col.into_boxed_slice());
                }
            }
            ColumnWorld {
                map,
                min_y,
                sections,
                decode,
            }
        }
    }

    impl<S: Copy> BlockView for ColumnWorld<S> {
        type State = S;

        #[inline]
        fn block(&self, x: i32, y: i32, z: i32) -> S {
            let Some(col) = self.map.get(&key(x >> 4, z >> 4)) else {
                return (self.decode)(0);
            };
            let sy = (y - self.min_y) >> 4;
            if sy < 0 || sy as usize >= self.sections {
                return (self.decode)(0);
            }
            let raw = match &col[sy as usize] {
                Some(s) => s.get((((y & 15) << 8) | ((z & 15) << 4) | (x & 15)) as usize),
                None => 0,
            };
            (self.decode)(raw)
        }

        #[inline]
        fn is_chunk_loaded(&self, cx: i32, cz: i32) -> bool {
            self.map.contains_key(&key(cx, cz))
        }
    }
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

    /// A dense copy of the region x, z in -32..32, y in 0..128 (every chunk loaded).
    struct Dense16 {
        blocks: Vec<ghast_data::v1_16_5::BlockState>,
    }

    impl Dense16 {
        fn of(w: &MemoryWorld) -> Dense16 {
            use ghast_physics::BlockView;
            let mut blocks = Vec::with_capacity(64 * 128 * 64);
            for y in 0..128 {
                for z in -32..32 {
                    for x in -32..32 {
                        blocks.push(w.block(x, y, z));
                    }
                }
            }
            Dense16 { blocks }
        }
    }

    impl ghast_physics::BlockView for Dense16 {
        type State = ghast_data::v1_16_5::BlockState;

        #[inline]
        fn block(&self, x: i32, y: i32, z: i32) -> Self::State {
            let (lx, lz) = (x + 32, z + 32);
            if !(0..64).contains(&lx) || !(0..64).contains(&lz) || !(0..128).contains(&y) {
                return ghast_data::v1_16_5::BlockState::AIR;
            }
            self.blocks[((y * 64 + lz) * 64 + lx) as usize]
        }

        #[inline]
        fn is_chunk_loaded(&self, _: i32, _: i32) -> bool {
            true
        }
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
            let cw = super::column_world::ColumnWorld::of(
                &w,
                [-4, -4],
                [3, 3],
                0,
                16,
                |s: ghast_data::v1_16_5::BlockState| s.id(),
                |r| {
                    ghast_data::v1_16_5::BlockState::from_id(r)
                        .unwrap_or(ghast_data::v1_16_5::BlockState::AIR)
                },
            );
            g.bench_function(format!("{name}/column_world"), |b| {
                let mut out = Packets::new();
                b.iter(|| {
                    let mut p = start;
                    for input in &inputs {
                        out.clear();
                        v1_16_5::tick(&mut p, input, black_box(&cw), &mut out);
                    }
                    black_box((p, out.len()))
                });
            });
            let dense = Dense16::of(&w);
            g.bench_function(format!("{name}/dense_array"), |b| {
                let mut out = Packets::new();
                b.iter(|| {
                    let mut p = start;
                    for input in &inputs {
                        out.clear();
                        v1_16_5::tick(&mut p, input, black_box(&dense), &mut out);
                    }
                    black_box((p, out.len()))
                });
            });
        }
        g.finish();
    }
}

mod modern26 {
    //! The modern core (26.1 rules): the same three shapes of work as `v1_16_5_tick`, in the same
    //! kind of hash-map world, plus a dense-array world that isolates the physics from the world.
    use super::{Criterion, TICKS, Throughput, black_box};
    use ghast_data::v26_1::BlockState;
    use ghast_physics::modern::{self, MemoryWorld26, PlayerState, V26_1};
    use ghast_physics::{BlockView, Keys, Packets, TickInput};

    fn st(spec: &str) -> u16 {
        BlockState::parse(spec)
            .unwrap_or_else(|| panic!("{spec}"))
            .id()
    }

    fn warm(w: &MemoryWorld26, mut p: PlayerState, input: &TickInput, n: usize) -> PlayerState {
        let mut out = Packets::new();
        for _ in 0..n {
            out.clear();
            modern::tick::<V26_1, _>(&mut p, input, w, &mut out);
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

    fn flat() -> (MemoryWorld26, PlayerState, Vec<TickInput>) {
        let mut w = modern::memory_world_26_1();
        w.load_chunks(-4, -4, 4, 4);
        w.fill_raw([-64, 63, -64], [63, 63, 63], st("stone"));
        let input = forward(-45.0);
        let p = warm(
            &w,
            PlayerState::new([0.5, 64.0, 0.5], -45.0, 0.0),
            &input,
            10,
        );
        (w, p, vec![input; TICKS])
    }

    fn busy() -> (MemoryWorld26, PlayerState, Vec<TickInput>) {
        let mut w = modern::memory_world_26_1();
        w.load_chunks(-2, -2, 2, 2);
        w.fill_raw([-8, 62, -8], [8, 63, 8], st("stone"));
        w.fill_raw([-8, 63, -8], [-1, 63, 8], 0);
        w.fill_raw(
            [3, 64, -3],
            [3, 64, 3],
            st("oak_fence[north=true,south=true]"),
        );
        w.fill_raw(
            [3, 65, -3],
            [3, 65, 3],
            st("iron_bars[north=true,south=true]"),
        );
        w.fill_raw(
            [0, 64, 3],
            [2, 64, 3],
            st("cobblestone_wall[east=low,west=low]"),
        );
        w.set_raw(2, 64, 2, st("oak_stairs[facing=south,shape=outer_left]"));
        w.set_raw(1, 64, 2, st("oak_stairs[facing=east]"));
        w.set_raw(2, 64, 1, st("smooth_stone_slab"));
        w.set_raw(0, 64, 1, st("glass_pane[north=true,south=true]"));
        w.fill_raw([-2, 67, -2], [4, 67, 4], st("smooth_stone_slab[type=top]"));
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

    fn swim() -> (MemoryWorld26, PlayerState, Vec<TickInput>) {
        let mut w = modern::memory_world_26_1();
        w.load_chunks(-2, -2, 2, 2);
        w.fill_raw([-16, 55, -16], [15, 55, 15], st("stone"));
        w.fill_raw([-16, 56, -16], [15, 63, 15], st("water"));
        w.fill_raw([-3, 56, -3], [3, 58, 3], st("kelp"));
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

    /// A dense copy of a world region (cheapest lookup), every chunk loaded.
    struct Dense {
        origin: [i32; 3],
        side: [i32; 3],
        blocks: Vec<BlockState>,
    }

    impl Dense {
        fn of(w: &MemoryWorld26, origin: [i32; 3], side: [i32; 3]) -> Dense {
            let mut blocks = Vec::with_capacity((side[0] * side[1] * side[2]) as usize);
            for y in 0..side[1] {
                for z in 0..side[2] {
                    for x in 0..side[0] {
                        blocks.push(w.block(origin[0] + x, origin[1] + y, origin[2] + z));
                    }
                }
            }
            Dense {
                origin,
                side,
                blocks,
            }
        }
    }

    impl BlockView for Dense {
        type State = BlockState;

        #[inline]
        fn block(&self, x: i32, y: i32, z: i32) -> BlockState {
            let (lx, ly, lz) = (x - self.origin[0], y - self.origin[1], z - self.origin[2]);
            if lx < 0
                || ly < 0
                || lz < 0
                || lx >= self.side[0]
                || ly >= self.side[1]
                || lz >= self.side[2]
            {
                return BlockState::AIR;
            }
            self.blocks[((ly * self.side[2] + lz) * self.side[0] + lx) as usize]
        }

        #[inline]
        fn is_chunk_loaded(&self, _: i32, _: i32) -> bool {
            true
        }
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
                modern::tick::<V26_1, _>(&mut p, input, black_box(w), &mut out);
            }
            black_box((p.pos, out.len()))
        });
    }

    pub fn bench(c: &mut Criterion) {
        let mut g = c.benchmark_group("modern_tick");
        g.throughput(Throughput::Elements(TICKS as u64));
        for (name, (w, start, inputs)) in [
            ("walk_flat", flat()),
            ("busy_collision", busy()),
            ("swim", swim()),
        ] {
            g.bench_function(format!("{name}/memory_world"), |b| {
                run(b, &w, start, &inputs)
            });
            let cw = super::column_world::ColumnWorld::of(
                &w,
                [-4, -4],
                [3, 3],
                -64,
                24,
                |s: BlockState| s.id(),
                |r| BlockState::from_id(r).unwrap_or(BlockState::AIR),
            );
            g.bench_function(format!("{name}/column_world"), |b| {
                run(b, &cw, start, &inputs)
            });
            let dense = Dense::of(&w, [-32, 0, -32], [64, 128, 64]);
            g.bench_function(format!("{name}/dense_array"), |b| {
                run(b, &dense, start, &inputs)
            });
        }
        g.finish();
    }
}

criterion_group!(benches, bench, modern::bench, modern26::bench);
criterion_main!(benches);
