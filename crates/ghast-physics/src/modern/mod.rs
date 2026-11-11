//! The modern client player movement (26.1 and later), one core generic over a [`Rules`] row.
//!
//! Same calling convention as the older version modules: state, input and world view in; new
//! state and the tick's movement packets out. Pure and deterministic. A version is a type implementing
//! [`Rules`] (its data tables and constants); the core is monomorphised per version, with no
//! run-time version branch.
//!
//! | rule set | version | status |
//! |---|---|---|
//! | [`V26_1`] | 26.1 (protocol 775) | bit-exact against the vanilla oracle |
//!
//! Unloaded chunks are handled exactly as the 26.1 client does: gravity stays normal (the client
//! reports every chunk as present), collision skips unloaded chunks, and the fluid sweep sees no
//! fluid when a chunk around the box is unloaded. The caller is responsible for not ticking the
//! player before its level is loaded (26.1's `player_loaded` gate: the client freezes the player
//! until the chunks around it have arrived); the project rule "never treat unloaded chunks as air"
//! is enforced there, one layer up.
//!
//! Not modelled (see the crate README): elytra, riptide, vehicles, entity push, pistons, block
//! entities. Known gap: an opening shulker box's collision reaches past its cube (its lid moves
//! with the block entity); the tables have the closed box, and the client push-out probe relies on
//! every suffocating large shape staying inside its cube (pinned by the `ghast-data` test
//! `suffocating_large_shapes_stay_in_their_cube`).

mod attr;
pub(crate) mod collide;
mod fluid;
mod inside;
pub(crate) mod math;
mod player;
mod rules;

pub use attr::{
    Attribute, MAX_MODIFIERS, Modifier, ModifierId, SPRINT_AMOUNT, identifier_hash,
    java_string_hash, sprinting_id,
};
pub use player::{
    AttributeKind, Attributes, ClientInput, ClientOptions, CollisionContext, Dimensions, Effects,
    LevelInfo, PlayerState, Pose, SentState, UseEffects, no_block_collision, on_climbable, tick,
};
pub use rules::{Rules, V26_1};

use crate::input::{Packets, TickInput};
use crate::world::BlockView;

/// Options of the modern rule sets (none yet).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Options {}

impl crate::Ruleset for V26_1 {
    type BlockState = ghast_data::v26_1::BlockState;
    type Player = PlayerState;
    type Options = Options;

    #[inline]
    fn tick_with<W: BlockView<State = Self::BlockState>>(
        player: &mut PlayerState,
        input: &TickInput,
        world: &W,
        _options: &Options,
        out: &mut Packets,
    ) {
        tick::<V26_1, W>(player, input, world, out);
    }
}

/// A [`crate::MemoryWorld`]-like in-memory world for modern versions (build height `-64..320`).
pub type MemoryWorld26 = ModernMemoryWorld<ghast_data::v26_1::BlockState>;

/// An in-memory world with a configurable build height: explicitly loaded chunk columns,
/// everything else unloaded. For tests, benches and offline planning.
#[derive(Clone, Debug)]
pub struct ModernMemoryWorld<S> {
    columns: std::collections::HashMap<(i32, i32), Box<[u16]>>,
    loaded_rects: Vec<[i32; 4]>,
    decode: fn(u16) -> S,
    min_y: i32,
    height: i32,
    border: crate::WorldBorder,
}

impl<S: Copy> ModernMemoryWorld<S> {
    /// An empty world; `decode` turns a stored raw value into a state (raw 0 must be air).
    #[must_use]
    pub fn new(decode: fn(u16) -> S, min_y: i32, height: i32) -> Self {
        ModernMemoryWorld {
            columns: std::collections::HashMap::new(),
            loaded_rects: Vec::new(),
            decode,
            min_y,
            height,
            border: crate::WorldBorder::default(),
        }
    }

    /// Marks the inclusive chunk rectangle `[cx0, cz0] ..= [cx1, cz1]` as loaded.
    pub fn load_chunks(&mut self, cx0: i32, cz0: i32, cx1: i32, cz1: i32) {
        self.loaded_rects.push([cx0, cz0, cx1, cz1]);
    }

    /// Sets the world border.
    pub fn set_border(&mut self, b: crate::WorldBorder) {
        self.border = b;
    }

    /// Stores a raw block value (positions outside the build height are ignored).
    pub fn set_raw(&mut self, x: i32, y: i32, z: i32, raw: u16) {
        if y < self.min_y || y >= self.min_y + self.height {
            return;
        }
        let len = (16 * 16 * self.height) as usize;
        let col = self
            .columns
            .entry((x >> 4, z >> 4))
            .or_insert_with(|| vec![0u16; len].into_boxed_slice());
        col[Self::index(x, y - self.min_y, z)] = raw;
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
    fn index(x: i32, ry: i32, z: i32) -> usize {
        ((ry as usize) << 8) | (((z & 15) as usize) << 4) | ((x & 15) as usize)
    }

    #[inline]
    fn loaded(&self, cx: i32, cz: i32) -> bool {
        self.loaded_rects
            .iter()
            .any(|r| cx >= r[0] && cx <= r[2] && cz >= r[1] && cz <= r[3])
    }
}

impl<S: Copy> BlockView for ModernMemoryWorld<S> {
    type State = S;

    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> S {
        let decode = self.decode;
        let (cx, cz) = (x >> 4, z >> 4);
        let ry = y - self.min_y;
        if ry < 0 || ry >= self.height || !self.loaded(cx, cz) {
            return decode(0);
        }
        match self.columns.get(&(cx, cz)) {
            Some(col) => decode(col[Self::index(x, ry, z)]),
            None => decode(0),
        }
    }

    #[inline]
    fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.loaded(chunk_x, chunk_z)
    }

    fn world_border(&self) -> crate::WorldBorder {
        self.border
    }
}

/// An empty 26.1 world (overworld build height); unknown ids decode to air.
#[must_use]
pub fn memory_world_26_1() -> MemoryWorld26 {
    ModernMemoryWorld::new(
        |raw| {
            ghast_data::v26_1::BlockState::from_id(raw)
                .unwrap_or(ghast_data::v26_1::BlockState::AIR)
        },
        -64,
        384,
    )
}
