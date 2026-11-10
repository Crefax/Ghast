//! What the physics needs to know about the world, and a simple in-memory implementation.

use std::collections::HashMap;

/// Read access to the blocks around a player, implemented by the world crate.
///
/// The physics only calls [`BlockView::block`] for valid positions: `|x|, |z| < 30_000_000` and
/// `0 <= y < 256` (vanilla returns air outside that range without asking the chunk). The
/// implementation must
///
/// - return the state **as vanilla decodes it** from chunk data: for 1.8 that is
///   `BlockState::from_raw_vanilla` (unknown ids and unregistered metas read as air);
/// - return air for positions in columns that are not loaded (vanilla's client uses an empty chunk
///   there — never "solid", never an error).
pub trait BlockView {
    /// Block state type of the protocol version (`ghast_data::v1_8::BlockState` for 1.8).
    type State: Copy;

    /// The block state at a valid position (air when the column is not loaded).
    fn block(&self, x: i32, y: i32, z: i32) -> Self::State;

    /// `true` when the chunk column `(chunk_x, chunk_z)` has been received from the server.
    fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool;

    /// The current world border (default: vanilla's initial border, centre 0, 0, size 6e7).
    fn world_border(&self) -> WorldBorder {
        WorldBorder::default()
    }
}

impl<W: BlockView + ?Sized> BlockView for &W {
    type State = W::State;

    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> Self::State {
        (**self).block(x, y, z)
    }

    #[inline]
    fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        (**self).is_chunk_loaded(chunk_x, chunk_z)
    }

    #[inline]
    fn world_border(&self) -> WorldBorder {
        (**self).world_border()
    }
}

/// The world border as the collision code sees it (centre and current diameter).
///
/// Vanilla clamps each bound to ±29,999,984 (`maxWorldBorderRadius`). The default (centre 0, 0,
/// size 6e7) is what a client has before any border packet arrives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldBorder {
    /// Centre X.
    pub center_x: f64,
    /// Centre Z.
    pub center_z: f64,
    /// Current diameter (a moving border is not modelled: pass its current size).
    pub size: f64,
}

impl Default for WorldBorder {
    fn default() -> Self {
        WorldBorder {
            center_x: 0.0,
            center_z: 0.0,
            size: 6.0e7,
        }
    }
}

/// Vanilla's clamp for each border bound.
const MAX_BORDER_RADIUS: f64 = 29_999_984.0;

impl WorldBorder {
    /// West bound (minimum X).
    #[must_use]
    pub fn west(&self) -> f64 {
        let v = self.center_x - self.size / 2.0;
        if v < -MAX_BORDER_RADIUS {
            -MAX_BORDER_RADIUS
        } else {
            v
        }
    }

    /// East bound (maximum X).
    #[must_use]
    pub fn east(&self) -> f64 {
        let v = self.center_x + self.size / 2.0;
        if v > MAX_BORDER_RADIUS {
            MAX_BORDER_RADIUS
        } else {
            v
        }
    }

    /// North bound (minimum Z).
    #[must_use]
    pub fn north(&self) -> f64 {
        let v = self.center_z - self.size / 2.0;
        if v < -MAX_BORDER_RADIUS {
            -MAX_BORDER_RADIUS
        } else {
            v
        }
    }

    /// South bound (maximum Z).
    #[must_use]
    pub fn south(&self) -> f64 {
        let v = self.center_z + self.size / 2.0;
        if v > MAX_BORDER_RADIUS {
            MAX_BORDER_RADIUS
        } else {
            v
        }
    }

    /// `true` when the block column at `(x, z)` is (partly) inside the border.
    #[must_use]
    pub fn contains_block(&self, x: i32, z: i32) -> bool {
        f64::from(x) + 1.0 > self.west()
            && f64::from(x) < self.east()
            && f64::from(z) + 1.0 > self.north()
            && f64::from(z) < self.south()
    }
}

/// Number of blocks in one 16×256×16 chunk column.
const COLUMN_LEN: usize = 16 * 256 * 16;

/// A small in-memory world: explicitly loaded chunk columns, everything else unloaded.
///
/// Meant for tests, benches and offline planning. Blocks are stored per column in a dense array
/// of raw values (`u16`), so lookups are a hash of the column plus an index.
#[derive(Clone, Debug)]
pub struct MemoryWorld<S> {
    columns: HashMap<(i32, i32), Box<[u16]>>,
    loaded_rects: Vec<[i32; 4]>,
    decode: fn(u16) -> S,
}

impl<S: Copy> MemoryWorld<S> {
    /// An empty world where nothing is loaded. `decode` turns a stored raw value into a state
    /// (for 1.8: `BlockState::from_raw_vanilla`); raw 0 must decode to air.
    #[must_use]
    pub fn new(decode: fn(u16) -> S) -> Self {
        MemoryWorld {
            columns: HashMap::new(),
            loaded_rects: Vec::new(),
            decode,
        }
    }

    /// Marks the inclusive chunk rectangle `[cx0, cz0] ..= [cx1, cz1]` as loaded (air until
    /// blocks are set).
    pub fn load_chunks(&mut self, cx0: i32, cz0: i32, cx1: i32, cz1: i32) {
        self.loaded_rects.push([cx0, cz0, cx1, cz1]);
    }

    /// Stores a raw block value. Positions outside `0..256` in Y are ignored. Setting a block in
    /// a column that is not loaded stores it but it stays invisible until the column is loaded.
    pub fn set_raw(&mut self, x: i32, y: i32, z: i32, raw: u16) {
        if !(0..256).contains(&y) {
            return;
        }
        let col = self
            .columns
            .entry((x >> 4, z >> 4))
            .or_insert_with(|| vec![0u16; COLUMN_LEN].into_boxed_slice());
        col[Self::index(x, y, z)] = raw;
    }

    /// Fills the inclusive box with a raw value.
    pub fn fill_raw(&mut self, from: [i32; 3], to: [i32; 3], raw: u16) {
        for x in from[0]..=to[0] {
            for z in from[2]..=to[2] {
                for y in from[1]..=to[1] {
                    self.set_raw(x, y, z, raw);
                }
            }
        }
    }

    #[inline]
    fn index(x: i32, y: i32, z: i32) -> usize {
        ((y as usize) << 8) | (((z & 15) as usize) << 4) | ((x & 15) as usize)
    }

    #[inline]
    fn loaded(&self, cx: i32, cz: i32) -> bool {
        self.loaded_rects
            .iter()
            .any(|r| cx >= r[0] && cx <= r[2] && cz >= r[1] && cz <= r[3])
    }
}

impl<S: Copy> BlockView for MemoryWorld<S> {
    type State = S;

    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> S {
        let decode = self.decode;
        let (cx, cz) = (x >> 4, z >> 4);
        if !self.loaded(cx, cz) {
            return decode(0);
        }
        match self.columns.get(&(cx, cz)) {
            Some(col) => decode(col[Self::index(x, y, z)]),
            None => decode(0),
        }
    }

    #[inline]
    fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.loaded(chunk_x, chunk_z)
    }
}

/// Side of the per-tick block cache cube.
#[cfg_attr(
    not(any(
        feature = "v1_8",
        feature = "v1_12",
        feature = "v1_16_5",
        feature = "v26_1"
    )),
    allow(dead_code)
)]
const CACHE_SIDE: i32 = 8;

/// A per-tick memo of block lookups in an 8×8×8 cube around the player.
///
/// One tick asks for the same few dozen positions many times (pose checks, push-out probes, the
/// collision sweeps, step-up retries); the cache turns the repeats into an array read. Anything
/// outside the cube goes straight to the world. Purely an optimisation: results are identical.
#[cfg_attr(
    not(any(
        feature = "v1_8",
        feature = "v1_12",
        feature = "v1_16_5",
        feature = "v26_1"
    )),
    allow(dead_code)
)]
pub(crate) struct TickCache<'w, W: BlockView> {
    world: &'w W,
    origin: [i32; 3],
    /// Chunk-loaded answers for the 3×3 chunks around the origin's chunk (0 unknown, 1 no, 2 yes).
    loaded: [std::cell::Cell<u8>; 9],
    chunk: [i32; 2],
    cells: [std::cell::Cell<Option<W::State>>; (CACHE_SIDE * CACHE_SIDE * CACHE_SIDE) as usize],
}

#[cfg_attr(
    not(any(
        feature = "v1_8",
        feature = "v1_12",
        feature = "v1_16_5",
        feature = "v26_1"
    )),
    allow(dead_code)
)]
impl<'w, W: BlockView> TickCache<'w, W> {
    /// A cache centred on block `(x, y, z)`.
    pub(crate) fn new(world: &'w W, x: i32, y: i32, z: i32) -> Self {
        let h = CACHE_SIDE / 2;
        TickCache {
            world,
            origin: [x - h, y - h + 1, z - h],
            loaded: [const { std::cell::Cell::new(0) }; 9],
            chunk: [x >> 4, z >> 4],
            cells: [const { std::cell::Cell::new(None) };
                (CACHE_SIDE * CACHE_SIDE * CACHE_SIDE) as usize],
        }
    }
}

impl<W: BlockView> BlockView for TickCache<'_, W> {
    type State = W::State;

    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> W::State {
        let (dx, dy, dz) = (
            x.wrapping_sub(self.origin[0]) as u32,
            y.wrapping_sub(self.origin[1]) as u32,
            z.wrapping_sub(self.origin[2]) as u32,
        );
        let side = CACHE_SIDE as u32;
        if dx >= side || dy >= side || dz >= side {
            return self.world.block(x, y, z);
        }
        let cell = &self.cells[((dy * side + dz) * side + dx) as usize];
        match cell.get() {
            Some(s) => s,
            None => {
                let s = self.world.block(x, y, z);
                cell.set(Some(s));
                s
            }
        }
    }

    #[inline]
    fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        let (dx, dz) = (
            chunk_x.wrapping_sub(self.chunk[0]).wrapping_add(1) as u32,
            chunk_z.wrapping_sub(self.chunk[1]).wrapping_add(1) as u32,
        );
        if dx >= 3 || dz >= 3 {
            return self.world.is_chunk_loaded(chunk_x, chunk_z);
        }
        let cell = &self.loaded[(dz * 3 + dx) as usize];
        match cell.get() {
            0 => {
                let l = self.world.is_chunk_loaded(chunk_x, chunk_z);
                cell.set(if l { 2 } else { 1 });
                l
            }
            v => v == 2,
        }
    }

    #[inline]
    fn world_border(&self) -> WorldBorder {
        self.world.world_border()
    }
}
