//! The 26.1 (protocol 775, 26.1 – 26.1.2) world: dimension heights from the configuration-phase
//! registries, compact paletted sections ([`crate::paletted`]), `level_chunk_with_light`
//! without its light data, `forget_level_chunk`, block and section updates, biomes, block
//! entities, chunk batches, the world border and the client's chunk window.
//!
//! ## Dimensions
//!
//! The build height is never assumed: the `login` / `respawn` packets name a `dimension_type`
//! registry entry by index, and [`World::apply_registry_data`] (fed with every configuration
//! `registry_data` packet) keeps that registry's `min_y` / `height`. A server sends an entry's
//! data only for packs the client did not declare as known; entries of the vanilla pack without
//! data use vanilla's values ([`DimensionType::vanilla`]). The biome registry's size gives the
//! width of global biome ids.
//!
//! ## Chunk format (vanilla `LevelChunkSection.read` / `PalettedContainer.read`)
//!
//! One section per 16 rows of the dimension, bottom to top: `i16` non-empty count, `i16` fluid
//! count (both ignored), the block container, the biome container. A container is a signed byte
//! `bits`, the palette, then exactly as many longs as the in-memory storage has:
//!
//! - blocks: `0` single value; `1..=4` linear palette (at most 16 ids) with 4-bit storage;
//!   `5..=8` hash palette with `bits`-bit storage; anything else global ids with
//!   `ceil(log2(state count))` bits (15), whatever the sent value;
//! - biomes: `0` single value; `1..=3` linear palette with `bits`-bit storage; anything else
//!   global ids with `ceil(log2(biome count))` bits.
//!
//! Palette ids that are not registry entries are an error (vanilla's `byIdOrThrow`), as are
//! stored indices past the palette (vanilla throws when such a block is read). An error unloads
//! the column. Trailing bytes are ignored. Heightmaps and light are not stored.
//!
//! ## Reads
//!
//! [`World::block_state`] hashes the column position. Hot loops (the physics) should take a
//! [`View`] around the player once per tick: a 3×3 column snapshot with O(1) reads and no
//! hashing inside it.

use super::data::{Block, BlockState, STATE_COUNT};
use super::proto::configuration::clientbound::RegistryData;
use super::proto::play::clientbound::{
    self as cb, BlockChange, ChunkBiomes, ClientboundPlayPacket, InitializeWorldBorder, Login,
    MapChunk, MultiBlockChange, Respawn, TileEntityData, UnloadChunk, UpdateViewDistance,
    UpdateViewPosition,
};
use bytes::Bytes;
use ghast_protocol::RawPacket;
use ghast_protocol::codec::Reader;
use ghast_protocol::nbt::{AnonNbt, Tag};

use crate::border::{Border, BorderPacket, LerpMath};
use crate::column::{BlockEntity, ColumnMap};
use std::sync::Arc;

use crate::paletted::{Builder, Ident, Section, WireBlocks, block_index};
use crate::{Applied, ChunkPos, SharedChunks, WorldConfig, WorldError};

/// Bits per block of the global palette: `ceil(log2(STATE_COUNT))` (15 for 26.1).
pub const GLOBAL_BITS: u32 = ceil_log2(STATE_COUNT);

/// Vanilla `Mth.ceillog2`.
const fn ceil_log2(n: usize) -> u32 {
    if n <= 1 {
        0
    } else {
        usize::BITS - (n - 1).leading_zeros()
    }
}

/// Clientbound play packet ids this world consumes (for a `PacketFilter`).
pub const PACKET_IDS: &[i32] = &[
    cb::id::LOGIN,
    cb::id::RESPAWN,
    cb::id::MAP_CHUNK,
    cb::id::UNLOAD_CHUNK,
    cb::id::BLOCK_CHANGE,
    cb::id::MULTI_BLOCK_CHANGE,
    cb::id::CHUNK_BIOMES,
    cb::id::CHUNK_BATCH_START,
    cb::id::CHUNK_BATCH_FINISHED,
    cb::id::UPDATE_VIEW_POSITION,
    cb::id::UPDATE_VIEW_DISTANCE,
    cb::id::INITIALIZE_WORLD_BORDER,
    cb::id::WORLD_BORDER_CENTER,
    cb::id::WORLD_BORDER_LERP_SIZE,
    cb::id::WORLD_BORDER_SIZE,
    cb::id::WORLD_BORDER_WARNING_DELAY,
    cb::id::WORLD_BORDER_WARNING_REACH,
];

/// Extra packet ids with [`WorldConfig::block_entities`] (chunk block entities come with
/// `map_chunk`).
pub const BLOCK_ENTITY_IDS: &[i32] = &[cb::id::TILE_ENTITY_DATA];

/// One `dimension_type` registry entry (the fields a client uses).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DimensionType {
    /// Registry key, e.g. `minecraft:overworld`.
    pub name: String,
    /// Lowest block Y (a multiple of 16).
    pub min_y: i32,
    /// Number of block rows (a multiple of 16, at least 16).
    pub height: i32,
    /// `logical_height`.
    pub logical_height: i32,
    /// `has_skylight`.
    pub has_skylight: bool,
    /// `has_ceiling`.
    pub has_ceiling: bool,
    /// Environment attribute `minecraft:gameplay/fast_lava` (the nether).
    pub fast_lava: bool,
    /// The entry's data as sent (`None` for a known-pack entry without data).
    pub nbt: Option<AnonNbt>,
}

impl DimensionType {
    /// Vanilla's built-in dimension types (the values the vanilla data pack defines), used for
    /// registry entries the server sent without data.
    #[must_use]
    pub fn vanilla(name: &str) -> Option<DimensionType> {
        let (min_y, height, logical, sky, ceiling, fast_lava) = match name {
            "minecraft:overworld" => (-64, 384, 384, true, false, false),
            "minecraft:overworld_caves" => (-64, 384, 384, true, true, false),
            "minecraft:the_nether" => (0, 256, 128, false, true, true),
            "minecraft:the_end" => (0, 256, 256, true, false, false),
            _ => return None,
        };
        Some(DimensionType {
            name: name.to_string(),
            min_y,
            height,
            logical_height: logical,
            has_skylight: sky,
            has_ceiling: ceiling,
            fast_lava,
            nbt: None,
        })
    }

    /// Reads an entry's NBT (vanilla `DimensionType` network codec); `None` when required
    /// fields are missing or invalid (vanilla rejects such a registry).
    #[must_use]
    pub fn from_nbt(name: &str, nbt: &AnonNbt) -> Option<DimensionType> {
        let tag = nbt.parse().ok()??;
        let c = tag.as_compound()?;
        let int = |k: &str| match c.get(k) {
            Some(Tag::Int(v)) => Some(*v),
            Some(Tag::Short(v)) => Some(i32::from(*v)),
            Some(Tag::Byte(v)) => Some(i32::from(*v)),
            _ => None,
        };
        let flag = |k: &str| matches!(c.get(k), Some(Tag::Byte(v)) if *v != 0);
        let min_y = int("min_y")?;
        let height = int("height")?;
        let logical_height = int("logical_height").unwrap_or(height);
        // Vanilla's checks (DimensionType constructor).
        if height < 16
            || height % 16 != 0
            || min_y % 16 != 0
            || i64::from(min_y) + i64::from(height) > 2032
            || min_y < -2032
        {
            return None;
        }
        let fast_lava = match c.get("attributes") {
            Some(Tag::Compound(a)) => {
                matches!(a.get("minecraft:gameplay/fast_lava"), Some(Tag::Byte(v)) if *v != 0)
            }
            _ => false,
        };
        Some(DimensionType {
            name: name.to_string(),
            min_y,
            height,
            logical_height,
            has_skylight: flag("has_skylight"),
            has_ceiling: flag("has_ceiling"),
            fast_lava,
            nbt: Some(nbt.clone()),
        })
    }
}

/// A biome registry entry's climate (`has_precipitation`, `temperature`,
/// `temperature_modifier`); `None` when the data is malformed.
fn climate_from_nbt(nbt: &AnonNbt) -> Option<crate::climate::Climate> {
    let tag = nbt.parse().ok()??;
    let c = tag.as_compound()?;
    let has_precipitation = match c.get("has_precipitation")? {
        Tag::Byte(v) => *v != 0,
        _ => return None,
    };
    let temperature = match c.get("temperature")? {
        Tag::Float(v) => *v,
        Tag::Double(v) => *v as f32,
        _ => return None,
    };
    let frozen = match c.get("temperature_modifier") {
        None => false,
        Some(Tag::String(s)) => match &**s {
            "none" | "minecraft:none" => false,
            "frozen" | "minecraft:frozen" => true,
            _ => return None,
        },
        Some(_) => return None,
    };
    Some(crate::climate::Climate::new(
        has_precipitation,
        temperature,
        frozen,
    ))
}

/// What the world keeps from the configuration-phase registries.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Registries {
    /// `minecraft:dimension_type` in registry order (`None` = an entry without data that is
    /// not a vanilla dimension type, or invalid data).
    pub dimension_types: Vec<Option<DimensionType>>,
    /// Number of `minecraft:worldgen/biome` entries (0 until received).
    pub biome_count: usize,
    /// The climate of each `minecraft:worldgen/biome` entry in registry order (from its data,
    /// or vanilla's for an entry sent without data; `None` when neither is known).
    pub biome_climates: Vec<Option<crate::climate::Climate>>,
}

/// The dimension the player is in.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Dimension {
    /// World name from `login` / `respawn` (e.g. `minecraft:overworld`).
    pub name: String,
    /// Index into the `dimension_type` registry.
    pub type_index: i32,
    /// The dimension type.
    pub kind: DimensionType,
}

/// A loaded 26.1 chunk column.
#[derive(Clone, Debug, Default)]
pub struct Column {
    /// The sections, behind the one pointer a read follows. With a [`SharedChunks`] store the
    /// array is shared with the other worlds that hold the same sections (it is never written
    /// while another handle exists).
    sections: Arc<[Section]>,
    /// What most columns of a bot do not have (kept out of the column table's slots).
    cold: Option<Box<Cold>>,
}

/// The rarely present parts of a [`Column`].
#[derive(Clone, Debug, Default)]
struct Cold {
    biomes: Option<Box<[BiomeSection]>>,
    /// Block entities keyed by [`be_key`] (only with [`WorldConfig::block_entities`]).
    block_entities: Vec<(u32, BlockEntity)>,
}

/// Exclusive access to a column's section array: the array itself when this is its only
/// handle (always, for a world that never had a store), else a copy of it first.
fn sections_mut(sections: &mut Arc<[Section]>) -> &mut [Section] {
    Arc::make_mut(sections)
}

/// Biomes of one section as sent (registry ids of `minecraft:worldgen/biome`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BiomeSection {
    /// One biome for the whole section.
    Single(u16),
    /// One biome per 4×4×4 cell, index `y << 4 | z << 2 | x` (cell coordinates).
    Cells(Box<[u16; 64]>),
}

impl BiomeSection {
    /// The biome of cell `index` (`0..64`).
    #[must_use]
    pub fn get(&self, index: usize) -> u16 {
        match self {
            BiomeSection::Single(b) => *b,
            BiomeSection::Cells(c) => c[index & 63],
        }
    }
}

/// Column-local key of a block: `(y - min_y) << 8 | z << 4 | x`.
#[inline]
fn be_key(x: i32, ry: i32, z: i32) -> u32 {
    ((ry as u32) << 8) | (((z & 15) as u32) << 4) | ((x & 15) as u32)
}

impl Column {
    /// The sections, bottom to top (`(y - min_y) >> 4`).
    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Raw state at column-local `x`, `z` (low four bits) and `ry = y - min_y`; air outside
    /// the column's height.
    #[inline]
    #[must_use]
    pub fn raw_rel(&self, x: i32, ry: i32, z: i32) -> u16 {
        match self.sections.get((ry >> 4) as usize) {
            Some(s) if ry >= 0 => s.get_at(x, ry, z),
            _ => 0,
        }
    }

    /// Biomes per section (only with [`WorldConfig::biomes`]).
    #[must_use]
    pub fn biomes(&self) -> Option<&[BiomeSection]> {
        self.cold.as_ref()?.biomes.as_deref()
    }

    fn stored_block_entities(&self) -> &[(u32, BlockEntity)] {
        self.cold.as_ref().map_or(&[], |c| &c.block_entities)
    }

    fn has_block_entities(&self) -> bool {
        self.cold
            .as_ref()
            .is_some_and(|c| !c.block_entities.is_empty())
    }

    fn set_stored_biomes(&mut self, biomes: Option<Box<[BiomeSection]>>) {
        match (&mut self.cold, biomes) {
            (Some(c), b) => c.biomes = b,
            (None, None) => {}
            (cold @ None, b) => {
                *cold = Some(Box::new(Cold {
                    biomes: b,
                    block_entities: Vec::new(),
                }));
            }
        }
    }

    /// Block entities as `([x, ry, z], data)` with `ry = y - min_y`, in arrival order. Empty
    /// unless [`WorldConfig::block_entities`] is on. [`BlockEntity::Nbt`]'s `action` is the
    /// block entity type's registry id (`u8::MAX` if it does not fit).
    pub fn block_entities(&self) -> impl Iterator<Item = ([i32; 3], &BlockEntity)> {
        self.stored_block_entities().iter().map(|(k, b)| {
            (
                [(k & 15) as i32, (k >> 8) as i32, ((k >> 4) & 15) as i32],
                b,
            )
        })
    }

    fn put_block_entity(&mut self, key: u32, be: BlockEntity) {
        let list = &mut self.cold.get_or_insert_default().block_entities;
        match list.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = be,
            None => list.push((key, be)),
        }
    }

    /// Heap bytes owned by this column. A section array shared through a [`SharedChunks`]
    /// store counts 0 here, like shared section storage (the store counts both once).
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        let sections = if Arc::weak_count(&self.sections) == 0 {
            self.sections.len() * size_of::<Section>()
                + self.sections.iter().map(Section::heap_bytes).sum::<usize>()
        } else {
            0
        };
        let cold = self.cold.as_ref().map_or(0, |_| size_of::<Cold>());
        let biomes = self.biomes().map_or(0, |b| {
            size_of_val(b)
                + b.iter()
                    .map(|s| match s {
                        BiomeSection::Single(_) => 0,
                        BiomeSection::Cells(_) => 128,
                    })
                    .sum::<usize>()
        });
        let be = self
            .cold
            .as_ref()
            .map_or(0, |c| c.block_entities.capacity())
            * size_of::<(u32, BlockEntity)>()
            + self
                .stored_block_entities()
                .iter()
                .map(|(_, b)| match b {
                    BlockEntity::Nbt { nbt, .. } => nbt.len(),
                    BlockEntity::Sign(l) => l.iter().map(|s| s.len()).sum(),
                })
                .sum::<usize>();
        sections + cold + biomes + be
    }
}

/// The client's chunk window (vanilla `ClientChunkCache.Storage`): chunks more than
/// `max(2, view distance) + 3` columns from the centre are not stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ViewWindow {
    radius: i32,
    center: (i32, i32),
}

impl ViewWindow {
    fn new(view_distance: i32) -> Self {
        ViewWindow {
            radius: view_distance.max(2).saturating_add(3),
            center: (0, 0),
        }
    }

    fn contains(self, cx: i32, cz: i32) -> bool {
        let r = i64::from(self.radius);
        (i64::from(cx) - i64::from(self.center.0)).abs() <= r
            && (i64::from(cz) - i64::from(self.center.1)).abs() <= r
    }
}

/// Vanilla's `ChunkBatchSizeCalculator`: how many chunks per tick the client asks for after
/// each chunk batch. Time is passed in (nanoseconds from any fixed origin), so it stays pure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChunkBatchSize {
    aggregated_nanos_per_chunk: f64,
    old_samples_weight: i32,
    batch_start_nanos: i64,
}

impl Default for ChunkBatchSize {
    fn default() -> Self {
        ChunkBatchSize {
            aggregated_nanos_per_chunk: 2_000_000.0,
            old_samples_weight: 1,
            batch_start_nanos: 0,
        }
    }
}

impl ChunkBatchSize {
    /// `chunk_batch_start` arrived at `now_nanos`.
    pub fn on_batch_start(&mut self, now_nanos: i64) {
        self.batch_start_nanos = now_nanos;
    }

    /// `chunk_batch_finished` with `batch_size` chunks arrived at `now_nanos`; returns the
    /// `chunks_per_tick` value of the `chunk_batch_received` answer the client must send.
    pub fn on_batch_finished(&mut self, batch_size: i32, now_nanos: i64) -> f32 {
        if batch_size > 0 {
            let duration = now_nanos.wrapping_sub(self.batch_start_nanos) as f64;
            let per = duration / f64::from(batch_size);
            let lo = self.aggregated_nanos_per_chunk / 3.0;
            let hi = self.aggregated_nanos_per_chunk * 3.0;
            // Mth.clamp(double): lower bound first, as Java's `value < min ? min : Math.min(value, max)`.
            let clamped = if per < lo { lo } else { per.min(hi) };
            let w = f64::from(self.old_samples_weight);
            self.aggregated_nanos_per_chunk =
                (self.aggregated_nanos_per_chunk * w + clamped) / (w + 1.0);
            self.old_samples_weight = (self.old_samples_weight + 1).min(49);
        }
        self.desired_chunks_per_tick()
    }

    /// `getDesiredChunksPerTick`.
    #[must_use]
    pub fn desired_chunks_per_tick(&self) -> f32 {
        (7_000_000.0 / self.aggregated_nanos_per_chunk) as f32
    }
}

/// The client-side world of a 26.1 connection.
pub struct World {
    columns: ColumnMap<Column>,
    config: WorldConfig,
    registries: Registries,
    dim: Option<Dimension>,
    view: ViewWindow,
    builder: Builder,
    border: Border,
    /// The border as the 26.1 client's collision reads it (tick-based lerp, one tick behind).
    #[cfg(feature = "physics")]
    collision_border: ghast_physics::modern::BorderState,
    batch: ChunkBatchSize,
    /// `BiomeManager.biomeZoomSeed` (the login / respawn hashed seed).
    biome_zoom_seed: i64,
    /// `Level.getSeaLevel()` (login / respawn).
    sea_level: i32,
    /// Where non-uniform sections are shared with other worlds ([`World::set_shared_chunks`]).
    shared: Option<SharedChunks>,
    /// Lowest block Y of the current dimension (0 without one), for the read path.
    min_y: i32,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World")
            .field("dimension", &self.dim.as_ref().map(|d| &d.name))
            .field("view", &self.view)
            .field("columns", &self.columns.len())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

#[inline]
fn void_air() -> BlockState {
    Block::VOID_AIR.default_state()
}

#[inline]
fn state(raw: u16) -> BlockState {
    BlockState::from_id(raw).unwrap_or(BlockState::AIR)
}

#[inline]
fn changed(b: bool) -> Applied {
    if b {
        Applied::Changed
    } else {
        Applied::Ignored
    }
}

/// `Block.BLOCK_STATE_REGISTRY.byId` with unknown ids as air (block updates; vanilla reads
/// `byIdOrThrow` there too, but a bad block update must not unload anything).
#[inline]
fn raw_from_wire(v: i64) -> u16 {
    u16::try_from(v)
        .ok()
        .filter(|&r| usize::from(r) < STATE_COUNT)
        .unwrap_or(0)
}

/// Same block type (the condition under which vanilla keeps a block entity).
fn same_block(a: u16, b: u16) -> bool {
    match (BlockState::from_id(a), BlockState::from_id(b)) {
        (Some(a), Some(b)) => a.block() == b.block(),
        _ => a == b,
    }
}

impl World {
    /// An empty world (no dimension yet: nothing can load before `login`).
    #[must_use]
    pub fn new(config: WorldConfig) -> Self {
        World {
            columns: ColumnMap::default(),
            config,
            registries: Registries::default(),
            dim: None,
            view: ViewWindow::new(2),
            builder: Builder::new(STATE_COUNT),
            border: Border::new(LerpMath::Double),
            #[cfg(feature = "physics")]
            collision_border: ghast_physics::modern::BorderState::new(),
            batch: ChunkBatchSize::default(),
            biome_zoom_seed: 0,
            sea_level: 63,
            shared: None,
            min_y: 0,
        }
    }

    /// Shares the block storage of the sections this world decodes from now on through
    /// `cache` (see [`SharedChunks`]), or stops sharing (`None`). Columns already loaded keep
    /// the storage they have. Without a store (the default) sections are owned as before.
    pub fn set_shared_chunks(&mut self, cache: Option<SharedChunks>) {
        self.shared = cache;
    }

    /// The store this world shares sections through, if any.
    #[must_use]
    pub fn shared_chunks(&self) -> Option<&SharedChunks> {
        self.shared.as_ref()
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> WorldConfig {
        self.config
    }

    /// The registries received during configuration.
    #[must_use]
    pub fn registries(&self) -> &Registries {
        &self.registries
    }

    /// The current dimension (`None` before `login`, or when the server named a dimension
    /// type the registries do not describe).
    #[must_use]
    pub fn dimension(&self) -> Option<&Dimension> {
        self.dim.as_ref()
    }

    /// Lowest block Y of the current dimension.
    #[must_use]
    pub fn min_y(&self) -> Option<i32> {
        self.dim.as_ref().map(|d| d.kind.min_y)
    }

    /// Number of block rows of the current dimension.
    #[must_use]
    pub fn height(&self) -> Option<i32> {
        self.dim.as_ref().map(|d| d.kind.height)
    }

    /// The block state at a position, or `None` when its column is not loaded. Positions
    /// outside the build height in a loaded column are `void_air`, as in vanilla.
    #[inline(always)]
    #[must_use]
    pub fn block_state(&self, x: i32, y: i32, z: i32) -> Option<BlockState> {
        let c = self.columns.get(x >> 4, z >> 4)?;
        Some(read(&c.sections, x, y.wrapping_sub(self.min_y), z))
    }

    /// `true` when the column `(chunk_x, chunk_z)` is loaded.
    #[inline]
    #[must_use]
    pub fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.columns.contains(chunk_x, chunk_z)
    }

    /// A loaded column.
    #[must_use]
    pub fn column(&self, chunk_x: i32, chunk_z: i32) -> Option<&Column> {
        self.columns.get(chunk_x, chunk_z)
    }

    /// All loaded columns.
    #[must_use]
    pub fn columns(&self) -> &ColumnMap<Column> {
        &self.columns
    }

    /// View distance from `login` / `set_chunk_cache_radius` as the window radius
    /// (`max(2, distance) + 3`).
    #[must_use]
    pub fn window_radius(&self) -> i32 {
        self.view.radius
    }

    /// Chunk window centre (`set_chunk_cache_center`; `(0, 0)` until one arrives).
    #[must_use]
    pub fn view_center(&self) -> ChunkPos {
        ChunkPos::new(self.view.center.0, self.view.center.1)
    }

    /// The world border.
    #[must_use]
    pub fn border(&self) -> &Border {
        &self.border
    }

    /// The chunk batch rate calculator (see [`World::on_chunk_batch_finished`]).
    #[must_use]
    pub fn chunk_batch(&self) -> &ChunkBatchSize {
        &self.batch
    }

    /// Approximate heap bytes used by the loaded columns.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.columns.table_bytes()
            + self
                .columns
                .iter()
                .map(|(_, c)| c.heap_bytes())
                .sum::<usize>()
    }

    /// Advances time-dependent world state (a moving border) by one tick; the client calls
    /// this once per tick.
    pub fn tick(&mut self) {
        self.border.advance(50);
        #[cfg(feature = "physics")]
        self.collision_border.tick();
    }

    /// Unloads everything (keeps the registries and the dimension).
    pub fn clear(&mut self) {
        match &self.shared {
            None => self.columns.clear(),
            Some(cache) => {
                for col in self.columns.drain() {
                    cache.release_column(col.sections);
                }
            }
        }
    }

    /// Drops a column this world held (through the store, which then forgets a section array
    /// nobody holds any more).
    fn release(&self, col: Option<Column>) {
        if let (Some(cache), Some(col)) = (&self.shared, col) {
            cache.release_column(col.sections);
        }
    }

    /// Turns [`WorldConfig::biomes`] on or off at run time. On: columns that arrive (and
    /// `chunks_biomes` updates) from now on keep their biomes; columns already loaded have none
    /// until the server resends them. Off: every stored biome is dropped.
    pub fn set_biomes(&mut self, on: bool) {
        self.config.biomes = on;
        if !on {
            for (_, col) in self.columns.iter_mut() {
                col.set_stored_biomes(None);
            }
        }
    }

    /// `BiomeManager.biomeZoomSeed`: the hashed seed of the last login / dimension change.
    #[must_use]
    pub fn biome_zoom_seed(&self) -> i64 {
        self.biome_zoom_seed
    }

    /// `Level.getSeaLevel()`: the sea level of the last login / dimension change.
    #[must_use]
    pub fn sea_level(&self) -> i32 {
        self.sea_level
    }

    /// `Level.getBiome(pos)`: the `minecraft:worldgen/biome` registry id of the block at `x`,
    /// `y`, `z` (the zoomed noise cell, `BiomeManager`), or `None` when the cell's column is not
    /// loaded or was loaded without biomes ([`WorldConfig::biomes`], [`World::set_biomes`]).
    /// Vanilla reads plains in a column it does not have.
    #[must_use]
    pub fn biome_at(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        let d = self.dim.as_ref()?;
        let [qx, qy, qz] = crate::climate::zoom_cell(self.biome_zoom_seed, x, y, z);
        let biomes = self.columns.get(qx >> 2, qz >> 2)?.biomes()?;
        let lo = d.kind.min_y >> 2;
        let hi = lo + (d.kind.height >> 2) - 1;
        let qy = qy.clamp(lo, hi);
        let section = biomes.get((((qy << 2) - d.kind.min_y) >> 4) as usize)?;
        Some(section.get((((qy & 3) << 4) | ((qz & 3) << 2) | (qx & 3)) as usize))
    }

    /// The climate of biome registry id `id` (from the synced registry; vanilla's for entries
    /// sent without data).
    #[must_use]
    pub fn biome_climate(&self, id: u16) -> Option<crate::climate::Climate> {
        self.registries
            .biome_climates
            .get(usize::from(id))
            .copied()
            .flatten()
    }

    /// `Biome.getPrecipitationAt(pos, seaLevel)` of the biome at a block position, `None` when
    /// the biome or its climate is unknown ([`World::biome_at`]). `Level.isRainingAt` also needs
    /// rain, sky access and the motion-blocking height map, which this does not check.
    #[must_use]
    pub fn precipitation_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<crate::climate::Precipitation> {
        let c = self.biome_climate(self.biome_at(x, y, z)?)?;
        Some(c.precipitation_at(x, y, z, self.sea_level))
    }

    /// The packet ids this world consumes with its configuration (for a `PacketFilter`).
    pub fn packet_ids(&self) -> impl Iterator<Item = i32> + use<> {
        let extra: &'static [i32] = if self.config.block_entities {
            BLOCK_ENTITY_IDS
        } else {
            &[]
        };
        PACKET_IDS.iter().chain(extra).copied()
    }

    /// A 3×3-column view around block `(x, z)` for many reads in a small box (the physics):
    /// reads inside it need no hashing. Valid until the world changes.
    #[must_use]
    pub fn view(&self, x: i32, z: i32) -> View<'_> {
        let (cx, cz) = (x >> 4, z >> 4);
        let mut cols = [None; 9];
        for (i, c) in cols.iter_mut().enumerate() {
            *c = self
                .columns
                .get(cx - 1 + (i % 3) as i32, cz - 1 + (i / 3) as i32);
        }
        View {
            world: self,
            cx0: cx - 1,
            cz0: cz - 1,
            min_y: self.min_y,
            cols,
        }
    }

    // ------------------------------------------------------------------ configuration

    /// Applies a configuration `registry_data` packet (every one must be routed here, also
    /// after a reconfiguration).
    pub fn apply_registry_data(&mut self, p: &RegistryData) -> Applied {
        self.apply_registry(&p.id, p.entries.iter().map(|e| (&*e.key, e.value.as_ref())))
    }

    /// [`World::apply_registry_data`] without the packet struct: registry id and
    /// `(key, data)` entries in order.
    pub fn apply_registry<'a>(
        &mut self,
        id: &str,
        entries: impl ExactSizeIterator<Item = (&'a str, Option<&'a AnonNbt>)>,
    ) -> Applied {
        match id {
            "minecraft:dimension_type" => {
                self.registries.dimension_types = entries
                    .map(|(k, v)| match v {
                        Some(nbt) => DimensionType::from_nbt(k, nbt),
                        None => DimensionType::vanilla(k),
                    })
                    .collect();
                Applied::Changed
            }
            "minecraft:worldgen/biome" => {
                self.registries.biome_count = entries.len();
                self.registries.biome_climates = entries
                    .map(|(k, v)| match v {
                        Some(nbt) => climate_from_nbt(nbt),
                        None => crate::climate::Climate::vanilla(k),
                    })
                    .collect();
                Applied::Changed
            }
            _ => Applied::NotWorldPacket,
        }
    }

    // ------------------------------------------------------------------ play

    /// Applies any world-relevant packet; other packets return [`Applied::NotWorldPacket`].
    ///
    /// `chunk_batch_finished` needs the arrival time: use [`World::on_chunk_batch_finished`]
    /// (here it is applied with time 0 and its answer is dropped).
    ///
    /// # Errors
    /// Malformed chunk data (the affected column is unloaded).
    pub fn handle(&mut self, packet: &ClientboundPlayPacket) -> Result<Applied, WorldError> {
        use ClientboundPlayPacket as P;
        match packet {
            P::Login(p) => Ok(self.apply_login(p)),
            P::Respawn(p) => Ok(self.apply_respawn(p)),
            P::MapChunk(p) => self.apply_map_chunk(p),
            P::UnloadChunk(p) => Ok(self.apply_unload_chunk(p)),
            P::BlockChange(p) => Ok(self.apply_block_change(p)),
            P::MultiBlockChange(p) => Ok(self.apply_multi_block_change(p)),
            P::ChunkBiomes(p) => self.apply_chunk_biomes(p),
            P::UpdateViewPosition(p) => Ok(self.apply_view_position(p)),
            P::UpdateViewDistance(p) => Ok(self.apply_view_distance(p)),
            P::ChunkBatchStart(_) => Ok(self.batch_start()),
            P::ChunkBatchFinished(p) => Ok(self.batch_finished(p.batch_size)),
            P::InitializeWorldBorder(p) => Ok(self.apply_initialize_border(p)),
            P::WorldBorderCenter(p) => Ok(self.apply_border_center(p)),
            P::WorldBorderLerpSize(p) => Ok(self.apply_border_lerp_size(p)),
            P::WorldBorderSize(p) => Ok(self.apply_border_size(p)),
            P::WorldBorderWarningDelay(p) => Ok(self.apply_border_warning_delay(p)),
            P::WorldBorderWarningReach(p) => Ok(self.apply_border_warning_reach(p)),
            P::TileEntityData(p) if self.config.block_entities => {
                Ok(self.apply_tile_entity_data(p))
            }
            _ => Ok(Applied::NotWorldPacket),
        }
    }

    /// Applies a world packet straight from its raw body. `map_chunk` is decoded here without
    /// its heightmaps and light (the cheap path: nothing after the block entities is read);
    /// other ids are decoded as their own packet type (no whole-protocol decoder is linked).
    ///
    /// # Errors
    /// A malformed packet or malformed chunk data.
    pub fn handle_raw(&mut self, raw: &RawPacket) -> Result<Applied, WorldError> {
        macro_rules! decode {
            ($t:ty) => {
                &raw.decode_as::<$t>().map_err(WorldError::Decode)?
            };
        }
        match raw.id {
            cb::id::MAP_CHUNK => self.apply_map_chunk_body(&raw.body),
            cb::id::LOGIN => Ok(self.apply_login(decode!(Login))),
            cb::id::RESPAWN => Ok(self.apply_respawn(decode!(Respawn))),
            cb::id::UNLOAD_CHUNK => Ok(self.apply_unload_chunk(decode!(UnloadChunk))),
            cb::id::BLOCK_CHANGE => Ok(self.apply_block_change(decode!(BlockChange))),
            cb::id::MULTI_BLOCK_CHANGE => {
                Ok(self.apply_multi_block_change(decode!(MultiBlockChange)))
            }
            cb::id::CHUNK_BIOMES => self.apply_chunk_biomes(decode!(ChunkBiomes)),
            cb::id::UPDATE_VIEW_POSITION => {
                Ok(self.apply_view_position(decode!(UpdateViewPosition)))
            }
            cb::id::UPDATE_VIEW_DISTANCE => {
                Ok(self.apply_view_distance(decode!(UpdateViewDistance)))
            }
            cb::id::CHUNK_BATCH_START => {
                let _: &cb::ChunkBatchStart = decode!(cb::ChunkBatchStart);
                Ok(self.batch_start())
            }
            cb::id::CHUNK_BATCH_FINISHED => {
                Ok(self.batch_finished(decode!(cb::ChunkBatchFinished).batch_size))
            }
            cb::id::INITIALIZE_WORLD_BORDER => {
                Ok(self.apply_initialize_border(decode!(InitializeWorldBorder)))
            }
            cb::id::WORLD_BORDER_CENTER => {
                Ok(self.apply_border_center(decode!(cb::WorldBorderCenter)))
            }
            cb::id::WORLD_BORDER_LERP_SIZE => {
                Ok(self.apply_border_lerp_size(decode!(cb::WorldBorderLerpSize)))
            }
            cb::id::WORLD_BORDER_SIZE => Ok(self.apply_border_size(decode!(cb::WorldBorderSize))),
            cb::id::WORLD_BORDER_WARNING_DELAY => {
                Ok(self.apply_border_warning_delay(decode!(cb::WorldBorderWarningDelay)))
            }
            cb::id::WORLD_BORDER_WARNING_REACH => {
                Ok(self.apply_border_warning_reach(decode!(cb::WorldBorderWarningReach)))
            }
            cb::id::TILE_ENTITY_DATA if self.config.block_entities => {
                Ok(self.apply_tile_entity_data(decode!(TileEntityData)))
            }
            _ => Ok(Applied::NotWorldPacket),
        }
    }

    fn batch_start(&mut self) -> Applied {
        self.on_chunk_batch_start(0);
        Applied::Changed
    }

    fn batch_finished(&mut self, size: i32) -> Applied {
        self.on_chunk_batch_finished(size, 0);
        Applied::Changed
    }

    /// `set_border_center`.
    pub fn apply_border_center(&mut self, p: &cb::WorldBorderCenter) -> Applied {
        #[cfg(feature = "physics")]
        self.collision_border.set_center(p.x, p.z);
        self.apply_border(&BorderPacket {
            action: 2,
            x: Some(p.x),
            z: Some(p.z),
            ..BorderPacket::EMPTY
        })
    }

    /// `set_border_lerp_size` (lerp time in ticks).
    pub fn apply_border_lerp_size(&mut self, p: &cb::WorldBorderLerpSize) -> Applied {
        #[cfg(feature = "physics")]
        self.collision_border
            .lerp_size(p.old_diameter, p.new_diameter, p.speed);
        self.apply_border(&BorderPacket {
            action: 1,
            old_diameter: Some(p.old_diameter),
            new_diameter: Some(p.new_diameter),
            speed_ms: Some(p.speed.saturating_mul(50)),
            ..BorderPacket::EMPTY
        })
    }

    /// `set_border_size`.
    pub fn apply_border_size(&mut self, p: &cb::WorldBorderSize) -> Applied {
        #[cfg(feature = "physics")]
        self.collision_border.set_size(p.diameter);
        self.apply_border(&BorderPacket {
            action: 0,
            diameter: Some(p.diameter),
            ..BorderPacket::EMPTY
        })
    }

    /// `set_border_warning_delay`.
    pub fn apply_border_warning_delay(&mut self, p: &cb::WorldBorderWarningDelay) -> Applied {
        self.apply_border(&BorderPacket {
            action: 4,
            warning_time: Some(p.warning_time),
            ..BorderPacket::EMPTY
        })
    }

    /// `set_border_warning_distance`.
    pub fn apply_border_warning_reach(&mut self, p: &cb::WorldBorderWarningReach) -> Applied {
        self.apply_border(&BorderPacket {
            action: 5,
            warning_blocks: Some(p.warning_blocks),
            ..BorderPacket::EMPTY
        })
    }

    fn apply_border(&mut self, p: &BorderPacket) -> Applied {
        changed(self.border.apply(p))
    }

    /// `initialize_border` (lerp time in ticks).
    pub fn apply_initialize_border(&mut self, p: &InitializeWorldBorder) -> Applied {
        // ClientPacketListener.handleInitializeBorder.
        #[cfg(feature = "physics")]
        {
            let b = &mut self.collision_border;
            b.set_center(p.x, p.z);
            if p.speed > 0 {
                b.lerp_size(p.old_diameter, p.new_diameter, p.speed);
            } else {
                b.set_size(p.new_diameter);
            }
        }
        self.apply_border(&BorderPacket {
            action: 3,
            x: Some(p.x),
            z: Some(p.z),
            old_diameter: Some(p.old_diameter),
            new_diameter: Some(p.new_diameter),
            speed_ms: Some(p.speed.saturating_mul(50)),
            portal_boundary: Some(p.portal_teleport_boundary),
            warning_time: Some(p.warning_time),
            warning_blocks: Some(p.warning_blocks),
            diameter: None,
        })
    }

    /// `chunk_batch_start` at `now_nanos`.
    pub fn on_chunk_batch_start(&mut self, now_nanos: i64) {
        self.batch.on_batch_start(now_nanos);
    }

    /// `chunk_batch_finished` at `now_nanos`: returns the `chunks_per_tick` the client must
    /// answer with (`chunk_batch_received`), as vanilla computes it.
    pub fn on_chunk_batch_finished(&mut self, batch_size: i32, now_nanos: i64) -> f32 {
        self.batch.on_batch_finished(batch_size, now_nanos)
    }

    fn enter(
        &mut self,
        name: &str,
        type_index: i32,
        view_distance: Option<i32>,
        (seed, sea_level): (i64, i32),
    ) -> Applied {
        self.clear();
        #[cfg(feature = "physics")]
        {
            self.collision_border = ghast_physics::modern::BorderState::new();
        }
        self.biome_zoom_seed = seed;
        self.sea_level = sea_level;
        self.border = Border::new(LerpMath::Double);
        let radius_distance = view_distance.unwrap_or(self.view.radius - 3);
        self.view = ViewWindow::new(radius_distance);
        let kind = usize::try_from(type_index)
            .ok()
            .and_then(|i| self.registries.dimension_types.get(i))
            .cloned()
            .flatten();
        self.dim = kind.map(|kind| Dimension {
            name: name.to_string(),
            type_index,
            kind,
        });
        self.min_y = self.dim.as_ref().map_or(0, |d| d.kind.min_y);
        Applied::Changed
    }

    /// Enters a dimension without a `login` packet (tests, offline planning): a new client
    /// level like [`World::apply_login`]; `type_index` names a `dimension_type` registry entry.
    pub fn enter_dimension(&mut self, name: &str, type_index: i32, view_distance: i32) -> Applied {
        self.enter(name, type_index, Some(view_distance), (0, 63))
    }

    /// [`World::enter_dimension`] with the spawn info's hashed seed (the biome zoom seed) and
    /// sea level.
    pub fn enter_dimension_with(
        &mut self,
        name: &str,
        type_index: i32,
        view_distance: i32,
        hashed_seed: i64,
        sea_level: i32,
    ) -> Applied {
        self.enter(
            name,
            type_index,
            Some(view_distance),
            (hashed_seed, sea_level),
        )
    }

    /// Join game: a new client level (everything unloaded, window centred on 0, 0).
    pub fn apply_login(&mut self, p: &Login) -> Applied {
        self.enter(
            &p.world_state.name,
            p.world_state.dimension,
            Some(p.view_distance),
            (p.world_state.hashed_seed, p.world_state.sea_level),
        )
    }

    /// Respawn: vanilla creates a new level only when the dimension (world name) changes.
    pub fn apply_respawn(&mut self, p: &Respawn) -> Applied {
        if self
            .dim
            .as_ref()
            .is_some_and(|d| d.name == *p.world_state.name)
        {
            return Applied::Ignored;
        }
        self.enter(
            &p.world_state.name,
            p.world_state.dimension,
            None,
            (p.world_state.hashed_seed, p.world_state.sea_level),
        )
    }

    /// `set_chunk_cache_center`: moves the window; columns outside it are dropped.
    pub fn apply_view_position(&mut self, p: &UpdateViewPosition) -> Applied {
        self.view.center = (p.chunk_x, p.chunk_z);
        self.purge_outside_window()
    }

    /// `set_chunk_cache_radius`: resizes the window; columns outside it are dropped.
    pub fn apply_view_distance(&mut self, p: &UpdateViewDistance) -> Applied {
        let center = self.view.center;
        self.view = ViewWindow::new(p.view_distance);
        self.view.center = center;
        self.purge_outside_window()
    }

    fn purge_outside_window(&mut self) -> Applied {
        let before = self.columns.len();
        let view = self.view;
        match &self.shared {
            None => self.columns.retain(|p| view.contains(p.x, p.z)),
            Some(cache) => {
                for col in self.columns.extract(|p| view.contains(p.x, p.z)) {
                    cache.release_column(col.sections);
                }
            }
        }
        changed(self.columns.len() != before)
    }

    /// `forget_level_chunk`.
    pub fn apply_unload_chunk(&mut self, p: &UnloadChunk) -> Applied {
        let col = self.columns.take(p.chunk_x, p.chunk_z);
        let was_loaded = col.is_some();
        self.release(col);
        changed(was_loaded)
    }

    /// `level_chunk_with_light` (decoded with the generated type; [`World::handle_raw`] skips
    /// the light data instead of decoding it).
    ///
    /// # Errors
    /// Malformed section data (the column is unloaded).
    pub fn apply_map_chunk(&mut self, p: &MapChunk) -> Result<Applied, WorldError> {
        let bes = p
            .block_entities
            .iter()
            .map(|b| (b.x, b.y, b.z, b.r#type, b.nbt_data.clone()));
        self.apply_chunk(p.x, p.z, &p.chunk_data, bes)
    }

    /// `level_chunk_with_light` from its raw body: block data and block entities only.
    ///
    /// # Errors
    /// A truncated packet or malformed section data (the column is unloaded).
    pub fn apply_map_chunk_body(&mut self, body: &Bytes) -> Result<Applied, WorldError> {
        let mut r = Reader::new(body);
        let head = (|| {
            let x = r.read_i32()?;
            let z = r.read_i32()?;
            let n = r.read_varint()?;
            for _ in 0..n {
                r.read_varint()?;
                let longs = r.read_varint()?;
                let longs = usize::try_from(longs)
                    .map_err(|_| ghast_protocol::DecodeError::Invalid("heightmap length"))?;
                r.skip(longs.saturating_mul(8))?;
            }
            let data = r.read_byte_array()?;
            Ok::<_, ghast_protocol::DecodeError>((x, z, data))
        })();
        let (x, z, data) = head.map_err(WorldError::Decode)?;
        if !self.config.block_entities {
            return self.apply_chunk(x, z, &data, std::iter::empty());
        }
        let mut bes = Vec::new();
        let list = (|| {
            let n = r.read_varint()?;
            for _ in 0..n {
                let xz = r.read_u8()?;
                let y = r.read_i16()?;
                let ty = r.read_varint()?;
                let nbt = AnonNbt::read_optional(&mut r)?;
                bes.push((xz >> 4, y, xz & 15, ty, nbt));
            }
            Ok::<_, ghast_protocol::DecodeError>(())
        })();
        list.map_err(WorldError::Decode)?;
        self.apply_chunk(x, z, &data, bes.into_iter())
    }

    fn apply_chunk(
        &mut self,
        x: i32,
        z: i32,
        data: &Bytes,
        block_entities: impl Iterator<Item = (u8, i16, u8, i32, Option<AnonNbt>)>,
    ) -> Result<Applied, WorldError> {
        let Some((min_y, height)) = self.dim.as_ref().map(|d| (d.kind.min_y, d.kind.height)) else {
            return Ok(Applied::Ignored);
        };
        if !self.view.contains(x, z) {
            return Ok(Applied::Ignored);
        }
        let layout = Layout {
            sections: (height >> 4) as usize,
            biome_bits: ceil_log2(self.registries.biome_count.max(1)),
            biome_count: self.registries.biome_count,
            keep_biomes: self.config.biomes,
        };
        let decoded = match &self.shared {
            None => decode_sections(&mut self.builder, data, &layout),
            Some(cache) => decode_sections_shared(&mut self.builder, cache, data, &layout),
        };
        let (sections, biomes) = match decoded {
            Ok(d) => d,
            Err((j, reason)) => {
                let old = self.columns.take(x, z);
                self.release(old);
                return Err(WorldError::Section {
                    pos: ChunkPos::new(x, z),
                    section: j.min(255) as u8,
                    reason,
                });
            }
        };
        let mut col = Column {
            sections,
            cold: None,
        };
        col.set_stored_biomes(biomes.map(Vec::into_boxed_slice));
        if self.config.block_entities {
            for (bx, y, bz, ty, nbt) in block_entities {
                let ry = i32::from(y) - min_y;
                let Some(nbt) = nbt.filter(|n| !n.is_end()) else {
                    continue;
                };
                if !(0..height).contains(&ry) {
                    continue;
                }
                col.put_block_entity(
                    be_key(i32::from(bx), ry, i32::from(bz)),
                    BlockEntity::Nbt {
                        action: u8::try_from(ty).unwrap_or(u8::MAX),
                        // A copy of its own: a slice would keep the whole packet buffer alive.
                        nbt: Bytes::copy_from_slice(nbt.as_bytes()),
                    },
                );
            }
        }
        let old = self.columns.replace(x, z, col);
        self.release(old);
        Ok(Applied::Changed)
    }

    /// `chunks_biomes`: replaces the biomes of loaded columns (ignored without
    /// [`WorldConfig::biomes`]).
    ///
    /// # Errors
    /// Malformed biome data (the column's biomes are dropped).
    pub fn apply_chunk_biomes(&mut self, p: &ChunkBiomes) -> Result<Applied, WorldError> {
        if !self.config.biomes {
            return Ok(Applied::Ignored);
        }
        let Some(height) = self.height() else {
            return Ok(Applied::Ignored);
        };
        let n = (height >> 4) as usize;
        let bits = ceil_log2(self.registries.biome_count.max(1));
        let count = self.registries.biome_count;
        let mut any = false;
        for e in &p.biomes {
            let (cx, cz) = (e.position.x, e.position.z);
            if !self.view.contains(cx, cz) {
                continue;
            }
            let Some(col) = self.columns.get_mut(cx, cz) else {
                continue;
            };
            let mut r = Reader::new(&e.data);
            let mut v = Vec::with_capacity(n);
            for j in 0..n {
                match read_biomes(&mut r, bits, count) {
                    Ok(b) => v.push(b),
                    Err(reason) => {
                        col.set_stored_biomes(None);
                        return Err(WorldError::Section {
                            pos: ChunkPos::new(cx, cz),
                            section: j.min(255) as u8,
                            reason,
                        });
                    }
                }
            }
            col.set_stored_biomes(Some(v.into_boxed_slice()));
            any = true;
        }
        Ok(changed(any))
    }

    /// The block entity at a position (`None` when absent or the column is not loaded).
    #[must_use]
    pub fn block_entity(&self, x: i32, y: i32, z: i32) -> Option<&BlockEntity> {
        let ry = y.checked_sub(self.min_y()?)?;
        let k = be_key(x, ry, z);
        self.columns
            .get(x >> 4, z >> 4)?
            .stored_block_entities()
            .iter()
            .find(|(key, _)| *key == k)
            .map(|(_, b)| b)
    }

    /// `block_entity_data` (with [`WorldConfig::block_entities`]): stored for a loaded column
    /// whatever the block is (vanilla only loads it into an existing block entity of that type).
    pub fn apply_tile_entity_data(&mut self, p: &TileEntityData) -> Applied {
        if !self.config.block_entities {
            return Applied::Ignored;
        }
        let Some((min_y, height)) = self.dim.as_ref().map(|d| (d.kind.min_y, d.kind.height)) else {
            return Applied::Ignored;
        };
        let l = p.location;
        let ry = l.y.wrapping_sub(min_y);
        let Some(nbt) = p.nbt_data.as_ref().filter(|n| !n.is_end()) else {
            return Applied::Ignored;
        };
        if !(0..height).contains(&ry) {
            return Applied::Ignored;
        }
        let Some(col) = self.columns.get_mut(l.x >> 4, l.z >> 4) else {
            return Applied::Ignored;
        };
        col.put_block_entity(
            be_key(l.x, ry, l.z),
            BlockEntity::Nbt {
                action: u8::try_from(p.action).unwrap_or(u8::MAX),
                // A copy of its own: a slice would keep the whole packet buffer alive.
                nbt: Bytes::copy_from_slice(nbt.as_bytes()),
            },
        );
        Applied::Changed
    }

    /// Sets a block locally (client-side prediction). Returns `false` and does nothing when
    /// the column is not loaded or `y` is outside the build height.
    pub fn set_block_state(&mut self, x: i32, y: i32, z: i32, s: BlockState) -> bool {
        self.set_raw(x, y, z, s.id())
    }

    fn set_raw(&mut self, x: i32, y: i32, z: i32, raw: u16) -> bool {
        let Some(min_y) = self.min_y() else {
            return false;
        };
        let Some(col) = self.columns.get_mut(x >> 4, z >> 4) else {
            return false;
        };
        let ry = y.wrapping_sub(min_y);
        if ry < 0 {
            return false;
        }
        let j = (ry >> 4) as usize;
        let i = block_index(x, ry, z);
        let old = match &self.shared {
            None => {
                let Some(s) = sections_mut(&mut col.sections).get_mut(j) else {
                    return false;
                };
                s.set(i, raw)
            }
            Some(cache) => {
                let Some(s) = col.sections.get(j) else {
                    return false;
                };
                let old = s.get(i);
                // A section of the store moves to the shared section with this change; a
                // section this world owns is changed in place.
                if old != raw
                    && !cache.update(
                        super::PROTOCOL,
                        &mut col.sections,
                        j,
                        std::iter::once((i, raw)),
                        None,
                    )
                {
                    sections_mut(&mut col.sections)[j].set(i, raw);
                }
                old
            }
        };
        if col.has_block_entities() && !same_block(old, raw) {
            let k = be_key(x, ry, z);
            if let Some(c) = &mut col.cold {
                c.block_entities.retain(|(key, _)| *key != k);
            }
        }
        true
    }

    /// `block_update`. Unknown state ids read as air; unloaded columns are ignored.
    pub fn apply_block_change(&mut self, p: &BlockChange) -> Applied {
        changed(self.set_raw(
            p.location.x,
            p.location.y,
            p.location.z,
            raw_from_wire(i64::from(p.r#type)),
        ))
    }

    /// `section_blocks_update` (section position + `state << 12 | x << 8 | z << 4 | y`).
    pub fn apply_multi_block_change(&mut self, p: &MultiBlockChange) -> Applied {
        let c = &p.chunk_coordinates;
        let sy = (c.y << 12) >> 12;
        if !self.columns.contains(c.x, c.z) {
            return Applied::Ignored;
        }
        if self.shared.is_some() && self.update_section_shared(c.x, sy, c.z, &p.records) {
            return changed(!p.records.is_empty());
        }
        let mut any = false;
        for &r in &p.records {
            let local = (r & 0xFFF) as i32;
            let y = sy.wrapping_mul(16).wrapping_add(local & 15);
            any |= self.set_raw(
                c.x.wrapping_mul(16) | ((local >> 8) & 15),
                y,
                c.z.wrapping_mul(16) | ((local >> 4) & 15),
                raw_from_wire(((r as u64) >> 12) as i64),
            );
        }
        changed(any)
    }
}

impl World {
    /// `section_blocks_update` for a section of the store: every record in one step, so that
    /// the worlds that receive the same packet end up sharing one section again (and a world
    /// copies the section at most once per packet). `false` when this is not possible (the
    /// section is outside the column or owned by this world): the caller then applies the
    /// records one by one.
    fn update_section_shared(&mut self, cx: i32, sy: i32, cz: i32, records: &[i64]) -> bool {
        let (Some(cache), Some(min_y)) = (&self.shared, self.min_y()) else {
            return false;
        };
        // `ry = sy * 16 + local y - min_y` is in one section for every record only when the
        // dimension starts on a section boundary (vanilla requires it).
        let base = sy.wrapping_mul(16).wrapping_sub(min_y);
        if min_y & 15 != 0 || base < 0 {
            return false;
        }
        let Some(col) = self.columns.get_mut(cx, cz) else {
            return false;
        };
        let j = (base >> 4) as usize;
        if j >= col.sections.len() {
            return false;
        }
        let recs = records.iter().map(|&r| {
            let local = (r & 0xFFF) as i32;
            (
                block_index((local >> 8) & 15, local & 15, (local >> 4) & 15),
                raw_from_wire(((r as u64) >> 12) as i64),
            )
        });
        let Column { sections, cold } = col;
        let has_block_entities = cold.as_ref().is_some_and(|c| !c.block_entities.is_empty());
        let mut drop_block_entity = |i: usize, old: u16, new: u16| {
            if !same_block(old, new)
                && let Some(c) = cold
            {
                // `be_key` of the block: its row above `min_y`, then z and x as in the index.
                let k = ((base as u32 + (i >> 8) as u32) << 8) | (i & 0xFF) as u32;
                c.block_entities.retain(|(key, _)| *key != k);
            }
        };
        cache.update(
            super::PROTOCOL,
            sections,
            j,
            recs,
            has_block_entities.then_some(&mut drop_block_entity as &mut dyn FnMut(usize, u16, u16)),
        )
    }
}

impl Drop for World {
    fn drop(&mut self) {
        // Hands the section arrays back to the store (see `SharedChunks::release_column`).
        self.clear();
    }
}

/// How a chunk packet's section data is laid out (from the dimension and the registries).
struct Layout {
    sections: usize,
    biome_bits: u32,
    biome_count: usize,
    keep_biomes: bool,
}

/// A column's sections and (when kept) biomes, or the first malformed section and why.
type Decoded = Result<(Arc<[Section]>, Option<Vec<BiomeSection>>), (usize, &'static str)>;

/// Decodes a chunk packet's sections, in order.
fn decode_sections(b: &mut Builder, data: &Bytes, l: &Layout) -> Decoded {
    let mut r = Reader::new(data);
    let mut biomes = l.keep_biomes.then(|| Vec::with_capacity(l.sections));
    let mut err = None;
    // Collected straight into the array (one allocation); after an error the rest is filler.
    let sections: Arc<[Section]> = (0..l.sections)
        .map(|j| {
            if err.is_some() {
                return Section::Uniform(0);
            }
            let step = read_section(b, &mut r).and_then(|s| {
                match &mut biomes {
                    Some(v) => v.push(read_biomes(&mut r, l.biome_bits, l.biome_count)?),
                    None => skip_biomes(&mut r, l.biome_bits, l.biome_count)?,
                }
                Ok(s)
            });
            match step {
                Ok(s) => s,
                Err(reason) => {
                    err = Some((j, reason));
                    Section::Uniform(0)
                }
            }
        })
        .collect();
    match err {
        Some(e) => Err(e),
        None => Ok((sections, biomes)),
    }
}

/// [`decode_sections`] through a store. A first pass over the packet finds every section's
/// identity without decoding it (a single-value container's state, or the key of the bytes of
/// any other container). When a world already holds a column with exactly these sections, its
/// section array is taken: one lookup for the whole column. Otherwise each section is taken
/// from the store or decoded and stored, and the new array is stored too.
///
/// Errors are those of [`decode_sections`]: anything the first pass cannot read sends the
/// packet there, and a section that fails to decode in the second pass is the first malformed
/// one (the first pass read everything else).
fn decode_sections_shared(
    b: &mut Builder,
    cache: &SharedChunks,
    data: &Bytes,
    l: &Layout,
) -> Decoded {
    let mut r = Reader::new(data);
    let mut biomes = l.keep_biomes.then(|| Vec::with_capacity(l.sections));
    // (identity, offset of the section in `data`).
    let mut wire: Vec<(Ident, usize)> = Vec::with_capacity(l.sections);
    for _ in 0..l.sections {
        let at = r.position();
        let ident = match shareable_len(&r) {
            Some(len) => {
                let key = cache.key(super::PROTOCOL, &r.rest_slice()[..len]);
                r.skip(len).ok().map(|()| Ident::Keyed(key))
            }
            None => match read_section(b, &mut r) {
                Ok(Section::Uniform(u)) => Some(Ident::Uniform(u)),
                _ => None,
            },
        };
        let biome = match &mut biomes {
            Some(v) => read_biomes(&mut r, l.biome_bits, l.biome_count).map(|b| v.push(b)),
            None => skip_biomes(&mut r, l.biome_bits, l.biome_count),
        };
        let (Some(ident), Ok(())) = (ident, biome) else {
            // Malformed: decode in order for the exact error (nothing is shared then).
            let (sections, biomes) = decode_sections(b, data, l)?;
            return Ok((cache.intern_column(sections), biomes));
        };
        wire.push((ident, at));
    }
    if let Some(sections) = cache.find_column(wire.iter().map(|w| w.0)) {
        return Ok((sections, biomes));
    }
    let mut err = None;
    let sections: Arc<[Section]> = wire
        .iter()
        .enumerate()
        .map(|(j, &(ident, at))| {
            let key = match ident {
                Ident::Uniform(u) => return Section::Uniform(u),
                Ident::Keyed(key) => key,
            };
            if err.is_some() {
                return Section::Uniform(0);
            }
            if let Some(s) = cache.get(key) {
                return s;
            }
            let mut r = Reader::new(data);
            let s = r
                .skip(at)
                .map_err(|_| "truncated")
                .and_then(|()| read_section(b, &mut r));
            match s {
                Ok(s) => cache.insert(key, s),
                Err(reason) => {
                    err = Some((j, reason));
                    Section::Uniform(0)
                }
            }
        })
        .collect();
    match err {
        Some(e) => Err(e),
        None => Ok((cache.intern_column(sections), biomes)),
    }
}

/// Block read inside a column (`ry = y - min_y`): void air outside the height. The read
/// chain (`View::block` → `View::block_state` → `read` → `Section::get`) is always inlined so
/// the physics' collision loops keep it inline (engine bench, instructions per tick).
#[inline(always)]
fn read(sections: &[Section], x: i32, ry: i32, z: i32) -> BlockState {
    // One unsigned compare covers both ends: a negative `ry` is at least 2^31 as `u32`, so
    // its section index is at least 2^27, more sections than a dimension can have (its height
    // is an `i32`).
    match sections.get((ry as u32 >> 4) as usize) {
        Some(s) => state(s.get_at(x, ry, z)),
        None => void_air(),
    }
}

/// Reads one section's block container (and the counts before it).
fn read_section(b: &mut Builder, r: &mut Reader<'_>) -> Result<Section, &'static str> {
    const EOF: &str = "truncated";
    r.read_i16().map_err(|_| EOF)?;
    r.read_i16().map_err(|_| EOF)?;
    let bits = r.read_i8().map_err(|_| EOF)?;
    match bits {
        0 => {
            let id = r.read_varint().map_err(|_| EOF)?;
            let id = u16::try_from(id)
                .ok()
                .filter(|&v| usize::from(v) < STATE_COUNT)
                .ok_or("palette id is not a block state")?;
            Ok(Section::Uniform(id))
        }
        1..=8 => {
            let storage = if bits <= 4 { 4u32 } else { bits as u32 };
            let n = r.read_varint().map_err(|_| EOF)?;
            let n = usize::try_from(n).map_err(|_| "negative palette length")?;
            if bits <= 4 && n > 16 {
                return Err("linear palette longer than 16");
            }
            if n > r.remaining() {
                return Err(EOF);
            }
            let mut pal = [0u16; 256];
            for i in 0..n {
                let id = r.read_varint().map_err(|_| EOF)?;
                let id = u16::try_from(id)
                    .ok()
                    .filter(|&v| usize::from(v) < STATE_COUNT)
                    .ok_or("palette id is not a block state")?;
                if let Some(p) = pal.get_mut(i) {
                    *p = id;
                }
            }
            let longs = 4096usize.div_ceil((64 / storage) as usize);
            let data = r.take(longs * 8).map_err(|_| EOF)?;
            b.build(&WireBlocks {
                storage_bits: storage,
                palette: Some(&pal[..n.min(256)]),
                data,
            })
        }
        _ => {
            let longs = 4096usize.div_ceil((64 / GLOBAL_BITS) as usize);
            let data = r.take(longs * 8).map_err(|_| EOF)?;
            b.build(&WireBlocks {
                storage_bits: GLOBAL_BITS,
                palette: None,
                data,
            })
        }
    }
}

/// Length of the section at the reader (counts and block container) when it is worth sharing:
/// `None` for a single-value container (nothing on the heap) and for anything malformed (the
/// decoder then reports the error). Ids are not checked here: a store only holds sections
/// whose identical bytes decoded without error for the same protocol.
fn shareable_len(r: &Reader<'_>) -> Option<usize> {
    let mut p = r.clone();
    let start = p.position();
    p.skip(4).ok()?;
    let bits = p.read_i8().ok()?;
    let longs = match bits {
        0 => return None,
        1..=8 => {
            let n = usize::try_from(p.read_varint().ok()?).ok()?;
            if (bits <= 4 && n > 16) || n > p.remaining() {
                return None;
            }
            for _ in 0..n {
                p.read_varint().ok()?;
            }
            let storage = if bits <= 4 { 4 } else { bits as usize };
            4096usize.div_ceil(64 / storage)
        }
        _ => 4096usize.div_ceil((64 / GLOBAL_BITS) as usize),
    };
    p.skip(longs * 8).ok()?;
    Some(p.position() - start)
}

/// Reads one biome container (`count` = biome registry size, 0 = unknown: ids not checked).
fn read_biomes(
    r: &mut Reader<'_>,
    global_bits: u32,
    count: usize,
) -> Result<BiomeSection, &'static str> {
    let mut cells = [0u16; 64];
    Ok(match read_biome_cells(r, global_bits, count, &mut cells)? {
        Some(single) => BiomeSection::Single(single),
        None => BiomeSection::Cells(Box::new(cells)),
    })
}

/// Reads and checks one biome container without keeping it (the same errors as
/// [`read_biomes`], no allocation).
fn skip_biomes(r: &mut Reader<'_>, global_bits: u32, count: usize) -> Result<(), &'static str> {
    read_biome_cells(r, global_bits, count, &mut [0u16; 64]).map(|_| ())
}

/// Reads one biome container into `cells`; `Some(biome)` when the whole section has one biome
/// (`cells` is then unspecified).
fn read_biome_cells(
    r: &mut Reader<'_>,
    global_bits: u32,
    count: usize,
    cells: &mut [u16; 64],
) -> Result<Option<u16>, &'static str> {
    const EOF: &str = "truncated";
    let check = |id: i32| -> Result<u16, &'static str> {
        u16::try_from(id)
            .ok()
            .filter(|&v| count == 0 || usize::from(v) < count)
            .ok_or("palette id is not a biome")
    };
    let bits = r.read_i8().map_err(|_| EOF)?;
    let (storage, palette) = match bits {
        0 => {
            let id = check(r.read_varint().map_err(|_| EOF)?)?;
            return Ok(Some(id));
        }
        1..=3 => {
            let n = r.read_varint().map_err(|_| EOF)?;
            let n = usize::try_from(n).map_err(|_| "negative palette length")?;
            if n > 1 << bits {
                return Err("biome palette too long");
            }
            let mut pal = [0u16; 8];
            for p in pal.iter_mut().take(n) {
                *p = check(r.read_varint().map_err(|_| EOF)?)?;
            }
            (bits as u32, Some((pal, n)))
        }
        _ => (global_bits.max(1), None),
    };
    let per = (64 / storage) as usize;
    let longs = 64usize.div_ceil(per);
    let data = r.take(longs * 8).map_err(|_| EOF)?;
    let mask = (1u64 << storage) - 1;
    let mut i = 0;
    for w in data.as_chunks::<8>().0 {
        let mut x = u64::from_be_bytes(*w);
        for _ in 0..per {
            if i == 64 {
                break;
            }
            let v = (x & mask) as usize;
            cells[i] = match &palette {
                Some((pal, n)) => {
                    if v >= *n {
                        return Err("biome index past the palette");
                    }
                    pal[v]
                }
                None => check(v as i32)?,
            };
            x >>= storage;
            i += 1;
        }
    }
    if cells.iter().all(|&c| c == cells[0]) {
        return Ok(Some(cells[0]));
    }
    Ok(None)
}

/// A 3×3-column snapshot of a [`World`] for fast reads in a small box (see [`World::view`]).
/// Reads outside the snapshot fall back to the world's hash map.
#[derive(Clone, Copy)]
pub struct View<'w> {
    world: &'w World,
    cx0: i32,
    cz0: i32,
    min_y: i32,
    cols: [Option<&'w Column>; 9],
}

impl std::fmt::Debug for View<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("View")
            .field("chunk", &(self.cx0 + 1, self.cz0 + 1))
            .finish_non_exhaustive()
    }
}

impl<'w> View<'w> {
    #[inline(always)]
    fn column(&self, cx: i32, cz: i32) -> Option<&'w Column> {
        let dx = cx.wrapping_sub(self.cx0) as u32;
        let dz = cz.wrapping_sub(self.cz0) as u32;
        if dx < 3 && dz < 3 {
            self.cols[(dz * 3 + dx) as usize]
        } else {
            self.world.columns.get(cx, cz)
        }
    }

    /// Like [`World::block_state`].
    #[inline(always)]
    #[must_use]
    pub fn block_state(&self, x: i32, y: i32, z: i32) -> Option<BlockState> {
        let c = self.column(x >> 4, z >> 4)?;
        Some(read(&c.sections, x, y.wrapping_sub(self.min_y), z))
    }

    /// Like [`World::is_column_loaded`].
    #[inline]
    #[must_use]
    pub fn is_column_loaded(&self, cx: i32, cz: i32) -> bool {
        self.column(cx, cz).is_some()
    }

    /// The world the view was taken from.
    #[must_use]
    pub fn world(&self) -> &'w World {
        self.world
    }
}

#[cfg(feature = "physics")]
mod physics {
    use super::{BlockState, View, World, void_air};

    /// The border collision reads (`WorldBorder.getMinX()` etc.: a moving border at its size
    /// before the last tick).
    fn border(w: &World) -> ghast_physics::WorldBorder {
        w.collision_border.collision_border()
    }

    impl ghast_physics::BlockView for World {
        type State = BlockState;

        /// Vanilla semantics: unloaded columns (the client's empty chunk) and positions outside
        /// the build height read as `void_air`.
        #[inline(always)]
        fn block(&self, x: i32, y: i32, z: i32) -> BlockState {
            self.block_state(x, y, z).unwrap_or_else(void_air)
        }

        #[inline]
        fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
            self.columns.contains(chunk_x, chunk_z)
        }

        fn world_border(&self) -> ghast_physics::WorldBorder {
            border(self)
        }
    }

    impl ghast_physics::BlockView for View<'_> {
        type State = BlockState;

        #[inline(always)]
        fn block(&self, x: i32, y: i32, z: i32) -> BlockState {
            self.block_state(x, y, z).unwrap_or_else(void_air)
        }

        #[inline]
        fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
            self.is_column_loaded(chunk_x, chunk_z)
        }

        fn world_border(&self) -> ghast_physics::WorldBorder {
            border(self.world)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_bits() {
        assert_eq!(GLOBAL_BITS, 15);
        assert_eq!(ceil_log2(65), 7);
        assert_eq!(ceil_log2(64), 6);
        assert_eq!(ceil_log2(1), 0);
    }

    /// An overworld world (vanilla registry entry, min_y -64, 24 sections).
    fn overworld() -> World {
        let mut w = World::new(WorldConfig::default());
        w.apply_registry(
            "minecraft:dimension_type",
            [("minecraft:overworld", None)].into_iter(),
        );
        w.enter_dimension("minecraft:overworld", 0, 8);
        w
    }

    /// A chunk body: section 7 (y 48..64) has `top` in its top row (y 63) and `rest` below,
    /// as a 4-bit linear palette; every other section is single-value air.
    fn chunk(cx: i32, cz: i32, top: u16, rest: u16) -> Bytes {
        use bytes::BufMut;
        use ghast_protocol::codec::McWrite;
        let mut d = bytes::BytesMut::new();
        for s in 0..24 {
            d.put_i16(if s == 7 { 4096 } else { 0 });
            d.put_i16(0);
            if s == 7 {
                d.put_u8(4);
                d.put_varint(2);
                d.put_varint(i32::from(rest));
                d.put_varint(i32::from(top));
                for w in 0..256 {
                    d.put_u64(if w >= 240 { 0x1111_1111_1111_1111 } else { 0 });
                }
            } else {
                d.put_u8(0);
                d.put_varint(0);
            }
            d.put_u8(0);
            d.put_varint(0);
        }
        let mut b = bytes::BytesMut::new();
        b.put_i32(cx);
        b.put_i32(cz);
        b.put_varint(0);
        b.put_varint(d.len() as i32);
        b.put_slice(&d);
        b.put_varint(0);
        b.freeze()
    }

    fn sec(w: &World, cx: i32, cz: i32) -> &Section {
        &w.column(cx, cz).unwrap().sections()[7]
    }

    fn same_storage(a: &Section, b: &Section) -> bool {
        use crate::paletted::Shared;
        match (a, b) {
            (Section::Shared1(x), Section::Shared1(y)) => Shared::ptr_eq(x, y),
            (Section::Shared2(x), Section::Shared2(y)) => Shared::ptr_eq(x, y),
            (Section::Shared4(x), Section::Shared4(y)) => Shared::ptr_eq(x, y),
            (Section::Shared8(x), Section::Shared8(y)) => Shared::ptr_eq(x, y),
            (Section::SharedDirect(x), Section::SharedDirect(y)) => Shared::ptr_eq(x, y),
            _ => false,
        }
    }

    /// A chunk body like [`chunk`] with any linear palette in section 7: block `i` has
    /// `palette[index(i)]`.
    fn chunk_with(cx: i32, cz: i32, palette: &[u16], index: impl Fn(usize) -> usize) -> Bytes {
        use bytes::BufMut;
        use ghast_protocol::codec::McWrite;
        assert!((2..=16).contains(&palette.len()));
        let mut d = bytes::BytesMut::new();
        for s in 0..24 {
            d.put_i16(if s == 7 { 4096 } else { 0 });
            d.put_i16(0);
            if s == 7 {
                d.put_u8(4);
                d.put_varint(palette.len() as i32);
                for &p in palette {
                    d.put_varint(i32::from(p));
                }
                for w in 0..256 {
                    let mut x = 0u64;
                    for k in 0..16 {
                        x |= (index(w * 16 + k) as u64 & 15) << (4 * k);
                    }
                    d.put_u64(x);
                }
            } else {
                d.put_u8(0);
                d.put_varint(0);
            }
            d.put_u8(0);
            d.put_varint(0);
        }
        let mut b = bytes::BytesMut::new();
        b.put_i32(cx);
        b.put_i32(cz);
        b.put_varint(0);
        b.put_varint(d.len() as i32);
        b.put_slice(&d);
        b.put_varint(0);
        b.freeze()
    }

    fn array(w: &World, cx: i32, cz: i32) -> &Arc<[Section]> {
        &w.column(cx, cz).unwrap().sections
    }

    /// A server `block_update`.
    fn update(w: &mut World, x: i32, y: i32, z: i32, raw: u16) -> Applied {
        w.apply_block_change(&BlockChange {
            location: ghast_protocol::BlockPos { x, y, z },
            r#type: i32::from(raw),
        })
    }

    /// A server `section_blocks_update` for section `sy` of column `(cx, cz)`: records are
    /// `(x, y, z, state)` with section-local coordinates.
    fn update_many(
        w: &mut World,
        (cx, sy, cz): (i32, i32, i32),
        recs: &[(i32, i32, i32, u16)],
    ) -> Applied {
        w.apply_multi_block_change(&MultiBlockChange {
            chunk_coordinates: cb::MultiBlockChangeChunkCoordinates {
                x: cx,
                z: cz,
                y: sy,
            },
            records: recs
                .iter()
                .map(|&(x, y, z, s)| (i64::from(s) << 12) | i64::from((x << 8) | (z << 4) | y))
                .collect(),
        })
    }

    /// Every loaded column, block and block entity of `w` is what `plain` has.
    fn assert_same(plain: &World, w: &World) {
        assert_eq!(plain.columns().len(), w.columns().len());
        for (p, c) in plain.columns().iter() {
            let d = w.column(p.x, p.z).expect("same columns");
            assert_eq!(c.sections().len(), d.sections().len());
            for (j, (a, b)) in c.sections().iter().zip(d.sections()).enumerate() {
                if let (Section::Uniform(x), Section::Uniform(y)) = (a, b) {
                    assert_eq!(x, y, "{p:?} section {j}");
                    continue;
                }
                for i in 0..4096 {
                    assert_eq!(a.get(i), b.get(i), "{p:?} section {j} block {i}");
                }
            }
            let bes = |c: &Column| {
                let mut v: Vec<_> = c.block_entities().map(|(pos, _)| pos).collect();
                v.sort_unstable();
                v
            };
            assert_eq!(bes(c), bes(d), "{p:?} block entities");
        }
    }

    const STONE: u16 = 1;
    const DIRT: u16 = 10;

    #[test]
    fn worlds_share_one_section_and_one_array() {
        let cache = SharedChunks::new();
        let (mut a, mut b, mut plain) = (overworld(), overworld(), overworld());
        a.set_shared_chunks(Some(cache.clone()));
        b.set_shared_chunks(Some(cache.clone()));
        let body = chunk(0, 0, STONE, DIRT);
        for w in [&mut a, &mut b, &mut plain] {
            assert_eq!(w.apply_map_chunk_body(&body).unwrap(), Applied::Changed);
        }
        assert!(sec(&a, 0, 0).is_shared() && !sec(&plain, 0, 0).is_shared());
        assert!(same_storage(sec(&a, 0, 0), sec(&b, 0, 0)), "decoded once");
        assert!(Arc::ptr_eq(array(&a, 0, 0), array(&b, 0, 0)), "one array");
        assert_eq!((cache.entries(), cache.sections()), (1, 1));
        assert_eq!((cache.column_entries(), cache.columns()), (1, 1));
        // The one array holds the storage; both worlds hold the array.
        assert!(matches!(sec(&a, 0, 0), Section::Shared1(s) if s.holders() == 1));
        assert_eq!(Arc::strong_count(array(&a, 0, 0)), 2);
        // Uniform sections are never stored.
        assert!(!a.column(0, 0).unwrap().sections()[0].is_shared());
        for (x, z) in [(1, 1), (5, 9)] {
            for w in [&a, &b] {
                assert_eq!(w.block_state(x, 63, z).unwrap().id(), STONE);
                assert_eq!(w.block_state(x, 62, z).unwrap().id(), DIRT);
            }
        }
        // a predicts a change b never gets: a moves to another section, b reads the old one.
        assert!(a.set_block_state(1, 63, 1, BlockState::AIR));
        assert!(sec(&a, 0, 0).is_shared() && sec(&b, 0, 0).is_shared());
        assert!(!same_storage(sec(&a, 0, 0), sec(&b, 0, 0)));
        assert!(!Arc::ptr_eq(array(&a, 0, 0), array(&b, 0, 0)));
        assert_eq!(a.block_state(1, 63, 1), Some(BlockState::AIR));
        assert_eq!(b.block_state(1, 63, 1).unwrap().id(), STONE);
        assert_eq!((cache.entries(), cache.sections()), (2, 2));
        assert_eq!((cache.column_entries(), cache.columns()), (2, 2));
        // A change to the same state touches nothing.
        let before = Arc::as_ptr(array(&b, 0, 0));
        assert!(b.set_block_state(5, 63, 5, BlockState::from_id(STONE).unwrap()));
        assert_eq!(Arc::as_ptr(array(&b, 0, 0)), before);
        assert_eq!((cache.entries(), cache.column_entries()), (2, 2));
        // b changes another block: nobody holds the decoded section any more.
        assert!(b.set_block_state(2, 62, 2, BlockState::AIR));
        assert_eq!((cache.entries(), cache.sections()), (2, 2));
        assert_eq!((cache.column_entries(), cache.columns()), (2, 2));
        assert_eq!(a.block_state(2, 62, 2).unwrap().id(), DIRT);
        assert_eq!(b.block_state(2, 62, 2), Some(BlockState::AIR));
        assert_eq!(b.block_state(1, 63, 1).unwrap().id(), STONE);
        // Both read like the plain world except for their own edits.
        for i in 0..4096 {
            let (x, y, z) = (i & 15, 48 + (i >> 8), (i >> 4) & 15);
            let p = plain.block_state(x, y, z);
            if (x, y, z) != (1, 63, 1) {
                assert_eq!(a.block_state(x, y, z), p);
            }
            if (x, y, z) != (2, 62, 2) {
                assert_eq!(b.block_state(x, y, z), p);
            }
        }
        // Each gets the other's change: the same content again, so one section and one array.
        assert!(a.set_block_state(2, 62, 2, BlockState::AIR));
        assert!(b.set_block_state(1, 63, 1, BlockState::AIR));
        assert!(same_storage(sec(&a, 0, 0), sec(&b, 0, 0)));
        assert!(Arc::ptr_eq(array(&a, 0, 0), array(&b, 0, 0)));
        assert_eq!((cache.entries(), cache.column_entries()), (1, 1));
        drop((a, b));
        assert_eq!((cache.entries(), cache.column_entries()), (0, 0));
    }

    #[test]
    fn the_same_updates_keep_one_copy() {
        let cache = SharedChunks::new();
        let mut plain = overworld();
        let mut ws = [overworld(), overworld(), overworld()];
        for w in &mut ws {
            w.set_shared_chunks(Some(cache.clone()));
        }
        let body = chunk(0, 0, STONE, DIRT);
        plain.apply_map_chunk_body(&body).unwrap();
        for w in &mut ws {
            w.apply_map_chunk_body(&body).unwrap();
        }
        // Single updates, a section update, updates in the uniform sections above and below,
        // states that grow the palette past 2, 4 and 16 entries, and a change back.
        type Step = Box<dyn Fn(&mut World) -> Applied>;
        let mut steps: Vec<Step> = vec![
            Box::new(|w| update(w, 3, 63, 3, 0)),
            Box::new(|w| update(w, 3, 62, 3, 7)),
            Box::new(|w| update_many(w, (0, 3, 0), &[(1, 2, 3, 9), (4, 15, 6, 0), (1, 2, 4, 9)])),
            Box::new(|w| update(w, 8, 64, 8, STONE)),
            Box::new(|w| update_many(w, (0, 2, 0), &[(0, 0, 0, 5), (15, 15, 15, 6)])),
            Box::new(|w| update(w, 3, 63, 3, STONE)),
        ];
        for k in 0..40u16 {
            steps.push(Box::new(move |w| {
                update(w, i32::from(k % 16), 50, i32::from(k / 16), 100 + k)
            }));
        }
        steps.push(Box::new(|w| {
            let recs: Vec<_> = (0..300)
                .map(|k| (k % 16, 5, (k / 16) % 16, 400 + k as u16))
                .collect();
            update_many(w, (0, 3, 0), &recs)
        }));
        let mut widths = std::collections::BTreeSet::new();
        widths.insert(sec(&ws[0], 0, 0).bits());
        for step in &steps {
            let r = step(&mut plain);
            assert_eq!(r, Applied::Changed);
            for w in &mut ws {
                assert_eq!(step(w), r);
            }
            for w in &ws {
                assert_same(&plain, w);
                assert!(Arc::ptr_eq(array(w, 0, 0), array(&ws[0], 0, 0)));
            }
            for j in 6..=8 {
                let s = &array(&ws[0], 0, 0)[j];
                assert!(
                    s.is_shared() || matches!(s, Section::Uniform(_)),
                    "{j}: {s:?}"
                );
            }
            widths.insert(sec(&ws[0], 0, 0).bits());
            // One section per changed place, one array: nothing left of the steps before.
            assert_eq!(cache.entries(), cache.sections());
            assert!(cache.sections() <= 3, "{}", cache.sections());
            assert_eq!((cache.column_entries(), cache.columns()), (1, 1));
        }
        assert_eq!(widths.into_iter().collect::<Vec<_>>(), [1, 2, 4, 8, 16]);
        drop(ws);
        assert_eq!((cache.entries(), cache.column_entries()), (0, 0));
    }

    #[test]
    fn updates_in_another_order_are_still_exact() {
        let cache = SharedChunks::new();
        let (mut a, mut b) = (overworld(), overworld());
        let (mut pa, mut pb) = (overworld(), overworld());
        a.set_shared_chunks(Some(cache.clone()));
        b.set_shared_chunks(Some(cache.clone()));
        let body = chunk(0, 0, STONE, DIRT);
        for w in [&mut a, &mut b, &mut pa, &mut pb] {
            w.apply_map_chunk_body(&body).unwrap();
        }
        // Different blocks in a different order: different sections on the way, the same
        // content at the end.
        for w in [&mut a, &mut pa] {
            update(w, 1, 60, 1, 7);
            update(w, 2, 60, 2, 8);
        }
        update(&mut b, 2, 60, 2, 8);
        update(&mut pb, 2, 60, 2, 8);
        assert!(!same_storage(sec(&a, 0, 0), sec(&b, 0, 0)));
        assert_same(&pb, &b);
        update(&mut b, 1, 60, 1, 7);
        update(&mut pb, 1, 60, 1, 7);
        assert_same(&pa, &a);
        assert_same(&pb, &b);
        assert!(same_storage(sec(&a, 0, 0), sec(&b, 0, 0)));
        assert!(Arc::ptr_eq(array(&a, 0, 0), array(&b, 0, 0)));
        // The same block in a different order: each world keeps what its own stream says.
        for (w, first, second) in [(&mut a, 20, 21), (&mut pa, 20, 21)] {
            update(w, 9, 61, 9, first);
            update(w, 9, 61, 9, second);
        }
        for (w, first, second) in [(&mut b, 21, 20), (&mut pb, 21, 20)] {
            update(w, 9, 61, 9, first);
            update(w, 9, 61, 9, second);
        }
        assert_eq!(a.block_state(9, 61, 9).unwrap().id(), 21);
        assert_eq!(b.block_state(9, 61, 9).unwrap().id(), 20);
        assert!(!same_storage(sec(&a, 0, 0), sec(&b, 0, 0)));
        assert_same(&pa, &a);
        assert_same(&pb, &b);
        // A world predicts a change; the other only learns it from the server later.
        assert!(a.set_block_state(9, 61, 9, BlockState::AIR));
        assert!(pa.set_block_state(9, 61, 9, BlockState::AIR));
        assert_eq!(b.block_state(9, 61, 9).unwrap().id(), 20);
        assert_same(&pb, &b);
        for w in [&mut a, &mut b, &mut pa, &mut pb] {
            update(w, 9, 61, 9, 0);
        }
        assert_same(&pa, &a);
        assert_same(&pb, &b);
        assert!(same_storage(sec(&a, 0, 0), sec(&b, 0, 0)));
        assert_eq!((cache.entries(), cache.column_entries()), (1, 1));
    }

    #[test]
    fn a_sole_holder_changes_its_storage_in_place_under_a_new_key() {
        let cache = SharedChunks::new();
        let (mut a, mut b, mut plain) = (overworld(), overworld(), overworld());
        a.set_shared_chunks(Some(cache.clone()));
        b.set_shared_chunks(Some(cache.clone()));
        let body = chunk(0, 0, STONE, DIRT);
        a.apply_map_chunk_body(&body).unwrap();
        plain.apply_map_chunk_body(&body).unwrap();
        let storage = |w: &World| match sec(w, 0, 0) {
            Section::Shared1(s) => std::ptr::from_ref::<crate::paletted::Packed1>(s).cast::<u8>(),
            Section::Shared2(s) => std::ptr::from_ref::<crate::paletted::Packed2>(s).cast::<u8>(),
            other => panic!("{other:?}"),
        };
        let (before, before_array) = (storage(&a), Arc::as_ptr(array(&a, 0, 0)));
        update(&mut a, 4, 60, 4, STONE);
        update(&mut plain, 4, 60, 4, STONE);
        assert_eq!(storage(&a), before, "changed in place");
        assert_eq!(Arc::as_ptr(array(&a, 0, 0)), before_array);
        assert_eq!((cache.entries(), cache.column_entries()), (1, 1));
        assert_same(&plain, &a);
        // The changed storage is not the decoded section any more: a world that now decodes
        // the same bytes must not get it.
        b.apply_map_chunk_body(&body).unwrap();
        assert!(!same_storage(sec(&a, 0, 0), sec(&b, 0, 0)));
        assert!(!Arc::ptr_eq(array(&a, 0, 0), array(&b, 0, 0)));
        assert_eq!(b.block_state(4, 60, 4).unwrap().id(), DIRT);
        assert_eq!(a.block_state(4, 60, 4).unwrap().id(), STONE);
        assert_eq!((cache.entries(), cache.column_entries()), (2, 2));
        // It is found by its content: b gets the same update and joins a.
        update(&mut b, 4, 60, 4, STONE);
        assert!(same_storage(sec(&a, 0, 0), sec(&b, 0, 0)));
        assert!(Arc::ptr_eq(array(&a, 0, 0), array(&b, 0, 0)));
        assert_eq!((cache.entries(), cache.column_entries()), (1, 1));
        // Held by two worlds now: a change by one must not show in the other.
        let shared = storage(&a);
        update(&mut a, 5, 60, 5, STONE);
        assert_ne!(storage(&a), shared);
        assert_eq!(storage(&b), shared);
        assert_eq!(b.block_state(5, 60, 5).unwrap().id(), DIRT);
        // A full palette (2 entries, a third state) cannot change in place: a new section.
        let before = storage(&a);
        update(&mut a, 6, 60, 6, 33);
        update(&mut plain, 5, 60, 5, STONE);
        update(&mut plain, 6, 60, 6, 33);
        assert_ne!(storage(&a), before);
        assert_same(&plain, &a);
        assert_eq!((cache.entries(), cache.column_entries()), (2, 2));
        drop((a, b));
        assert_eq!((cache.entries(), cache.column_entries()), (0, 0));
    }

    #[test]
    fn sections_a_world_owns_stay_its_own() {
        let cache = SharedChunks::new();
        let other = SharedChunks::new();
        let body = chunk(0, 0, STONE, DIRT);
        // Decoded without a store: owned, and still owned after a change with a store.
        let mut a = overworld();
        a.apply_map_chunk_body(&body).unwrap();
        a.set_shared_chunks(Some(cache.clone()));
        update(&mut a, 1, 60, 1, STONE);
        assert!(!sec(&a, 0, 0).is_shared());
        assert_eq!(a.block_state(1, 60, 1).unwrap().id(), STONE);
        assert_eq!((cache.entries(), cache.column_entries()), (0, 0));
        // A uniform section of that column is the store's to share; the array is not.
        update(&mut a, 1, 70, 1, STONE);
        assert!(array(&a, 0, 0)[8].is_shared());
        assert_eq!((cache.entries(), cache.column_entries()), (1, 0));
        // Shared through another store: copied on the first change, as without a store, and
        // the worlds of that store do not see it.
        let (mut b, mut c) = (overworld(), overworld());
        b.set_shared_chunks(Some(other.clone()));
        c.set_shared_chunks(Some(other.clone()));
        b.apply_map_chunk_body(&body).unwrap();
        c.apply_map_chunk_body(&body).unwrap();
        b.set_shared_chunks(Some(cache.clone()));
        update(&mut b, 2, 60, 2, STONE);
        assert!(!sec(&b, 0, 0).is_shared() && sec(&c, 0, 0).is_shared());
        assert_eq!(b.block_state(2, 60, 2).unwrap().id(), STONE);
        assert_eq!(c.block_state(2, 60, 2).unwrap().id(), DIRT);
        assert_eq!((other.entries(), other.sections()), (1, 1));
        // And with no store at all.
        c.set_shared_chunks(None);
        let mut d = overworld();
        d.set_shared_chunks(Some(other.clone()));
        d.apply_map_chunk_body(&body).unwrap();
        assert!(Arc::ptr_eq(array(&c, 0, 0), array(&d, 0, 0)));
        update(&mut c, 3, 60, 3, STONE);
        assert!(!sec(&c, 0, 0).is_shared() && sec(&d, 0, 0).is_shared());
        assert_eq!(d.block_state(3, 60, 3).unwrap().id(), DIRT);
        assert_eq!(c.block_state(3, 60, 3).unwrap().id(), STONE);
        // A cloned column (two handles to an unshared array) is copied before a write.
        let mut e = overworld();
        e.apply_map_chunk_body(&body).unwrap();
        let copy = e.column(0, 0).unwrap().clone();
        update(&mut e, 4, 60, 4, STONE);
        assert_eq!(copy.sections()[7].get(block_index(4, 60 + 64, 4)), DIRT);
        assert_eq!(e.block_state(4, 60, 4).unwrap().id(), STONE);
    }

    #[test]
    fn section_updates_drop_block_entities_like_an_unshared_world() {
        let cache = SharedChunks::new();
        let config = WorldConfig::default().with_block_entities(true);
        let make = |shared: bool| {
            let mut w = World::new(config);
            w.apply_registry(
                "minecraft:dimension_type",
                [("minecraft:overworld", None)].into_iter(),
            );
            w.enter_dimension("minecraft:overworld", 0, 8);
            w.set_shared_chunks(shared.then(|| cache.clone()));
            w.apply_map_chunk_body(&chunk(0, 0, STONE, DIRT)).unwrap();
            for (x, y, z) in [(1, 50, 1), (2, 50, 2), (3, 50, 3), (4, 70, 4)] {
                let r = w.apply_tile_entity_data(&TileEntityData {
                    location: ghast_protocol::BlockPos { x, y, z },
                    action: 1,
                    nbt_data: Some(AnonNbt::from_bytes(Bytes::from_static(&[10, 0])).unwrap()),
                });
                assert_eq!(r, Applied::Changed);
            }
            w
        };
        let (mut plain, mut a, mut b) = (make(false), make(true), make(true));
        // (1, 50, 1): another block; (2, 50, 2): the same state (kept); (3, 50, 3): changed
        // and changed back in one packet (dropped, as the records apply in order).
        let recs = [
            (1, 2, 1, 5),
            (2, 2, 2, DIRT),
            (3, 2, 3, 5),
            (3, 2, 3, DIRT),
            (9, 9, 9, 6),
        ];
        for w in [&mut plain, &mut a, &mut b] {
            assert_eq!(update_many(w, (0, 3, 0), &recs), Applied::Changed);
            assert_eq!(update(w, 4, 70, 4, STONE), Applied::Changed);
        }
        assert_eq!(plain.column(0, 0).unwrap().block_entities().count(), 1);
        assert!(plain.block_entity(2, 50, 2).is_some());
        assert_same(&plain, &a);
        assert_same(&plain, &b);
        assert!(same_storage(sec(&a, 0, 0), sec(&b, 0, 0)));
        // Records that leave the section as it was change nothing.
        let before = Arc::as_ptr(array(&a, 0, 0));
        let recs = [(7, 7, 7, 5), (7, 7, 7, DIRT)];
        for w in [&mut plain, &mut a] {
            assert_eq!(update_many(w, (0, 3, 0), &recs), Applied::Changed);
        }
        assert_eq!(Arc::as_ptr(array(&a, 0, 0)), before);
        assert_same(&plain, &a);
        // Outside the column: ignored, as without a store.
        for w in [&mut plain, &mut a] {
            assert_eq!(update_many(w, (0, 40, 0), &recs), Applied::Ignored);
            assert_eq!(update_many(w, (0, -9, 0), &recs), Applied::Ignored);
            assert_eq!(update_many(w, (0, 3, 0), &[]), Applied::Ignored);
            assert_eq!(update_many(w, (5, 3, 0), &recs), Applied::Ignored);
        }
    }

    #[test]
    fn block_entity_data_is_copied_out_of_the_packet() {
        let config = WorldConfig::default().with_block_entities(true);
        let mut w = World::new(config);
        w.apply_registry(
            "minecraft:dimension_type",
            [("minecraft:overworld", None)].into_iter(),
        );
        w.enter_dimension("minecraft:overworld", 0, 8);
        // A chunk with one block entity at (3, 50, 4): the list replaces the empty one.
        let mut body = chunk(0, 0, STONE, DIRT).to_vec();
        body.pop();
        body.extend_from_slice(&[1, (3 << 4) | 4, 0, 50, 7, 10, 0]);
        let body = Bytes::from(body);
        w.apply_map_chunk_body(&body).unwrap();
        // A `block_entity_data` packet whose NBT is a slice of a larger read buffer.
        let mut buffer = vec![0u8; 4096];
        buffer[1000..1002].copy_from_slice(&[10, 0]);
        let buffer = Bytes::from(buffer);
        let r = w.apply_tile_entity_data(&TileEntityData {
            location: ghast_protocol::BlockPos { x: 5, y: 51, z: 6 },
            action: 1,
            nbt_data: Some(AnonNbt::from_bytes(buffer.slice(1000..1002)).unwrap()),
        });
        assert_eq!(r, Applied::Changed);
        let outside = |held: &Bytes, of: &Bytes| {
            let (p, start) = (held.as_ptr() as usize, of.as_ptr() as usize);
            p < start || p >= start + of.len()
        };
        for (pos, source) in [((3, 50, 4), &body), ((5, 51, 6), &buffer)] {
            let Some(BlockEntity::Nbt { nbt, .. }) = w.block_entity(pos.0, pos.1, pos.2) else {
                panic!("no block entity at {pos:?}");
            };
            assert_eq!(&nbt[..], &[10, 0]);
            assert!(outside(nbt, source), "{pos:?} holds the packet buffer");
        }
    }

    /// xorshift64*.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        fn below(&mut self, n: u64) -> u64 {
            (self.next() >> 33) % n
        }
    }

    /// One random thing a server (or the bot) does to a world.
    #[derive(Debug, Clone)]
    enum Op {
        Chunk(i32, u8),
        Unload(i32),
        Update(i32, i32, i32, u16),
        Many(i32, i32, Vec<(i32, i32, i32, u16)>),
        BlockEntity(i32, i32, i32),
    }

    impl Op {
        fn random(r: &mut Rng) -> Op {
            // Few places and few states, so that worlds meet again; states up to 40 so that
            // palettes grow through every width but direct.
            let state = |r: &mut Rng| match r.below(4) {
                0 => 0,
                1 => [STONE, DIRT][r.below(2) as usize],
                _ => r.below(40) as u16,
            };
            let cx = r.below(2) as i32;
            match r.below(20) {
                0 => Op::Chunk(cx, r.below(4) as u8),
                1 => Op::Unload(cx),
                2 => Op::BlockEntity(cx * 16 + r.below(3) as i32, 50, r.below(3) as i32),
                3..=9 => {
                    let most = if r.below(8) == 0 { 400 } else { 6 };
                    let n = 1 + r.below(most) as usize;
                    let side = if r.below(3) == 0 { 2 } else { 16 };
                    let recs = (0..n)
                        .map(|_| {
                            (
                                r.below(side) as i32,
                                r.below(side) as i32,
                                r.below(side) as i32,
                                state(r),
                            )
                        })
                        .collect();
                    Op::Many(cx, 1 + r.below(4) as i32, recs)
                }
                _ => Op::Update(
                    cx * 16 + r.below(3) as i32,
                    30 + r.below(50) as i32,
                    r.below(3) as i32,
                    state(r),
                ),
            }
        }

        fn apply(&self, w: &mut World) -> Result<Applied, WorldError> {
            Ok(match self {
                Op::Chunk(cx, 0) => w.apply_map_chunk_body(&chunk(*cx, 0, STONE, DIRT))?,
                Op::Chunk(cx, 1) => w.apply_map_chunk_body(&chunk(*cx, 0, DIRT, STONE))?,
                Op::Chunk(cx, 2) => {
                    let palette: Vec<u16> = (0..16).map(|k| 50 + k).collect();
                    w.apply_map_chunk_body(&chunk_with(*cx, 0, &palette, |i| (i * 7) % 16))?
                }
                Op::Chunk(cx, _) => {
                    // A palette entry past the palette: an error, the column unloads.
                    w.apply_map_chunk_body(&chunk_with(*cx, 0, &[STONE, DIRT], |i| i % 3))?
                }
                Op::Unload(cx) => w.apply_unload_chunk(&UnloadChunk {
                    chunk_z: 0,
                    chunk_x: *cx,
                }),
                Op::Update(x, y, z, s) => update(w, *x, *y, *z, *s),
                Op::Many(cx, sy, recs) => update_many(w, (*cx, *sy, 0), recs),
                Op::BlockEntity(x, y, z) => w.apply_tile_entity_data(&TileEntityData {
                    location: ghast_protocol::BlockPos {
                        x: *x,
                        y: *y,
                        z: *z,
                    },
                    action: 1,
                    nbt_data: Some(AnonNbt::from_bytes(Bytes::from_static(&[10, 0])).unwrap()),
                }),
            })
        }
    }

    fn bot(cache: Option<&SharedChunks>) -> World {
        let mut w = World::new(WorldConfig::default().with_block_entities(true));
        w.apply_registry(
            "minecraft:dimension_type",
            [("minecraft:overworld", None)].into_iter(),
        );
        w.enter_dimension("minecraft:overworld", 0, 8);
        w.set_shared_chunks(cache.cloned());
        w
    }

    /// Random chunks, block updates, section updates, unloads, block entities and store
    /// changes on several worlds of one store, each next to a world without a store that gets
    /// the same packets: after every step each sharing world reads exactly what its unshared
    /// twin reads, whatever the other worlds did.
    #[test]
    fn random_packets_read_like_unshared_worlds() {
        let (cache, other) = (SharedChunks::new(), SharedChunks::new());
        let mut r = Rng(0x9e37_79b9_7f4a_7c15);
        let mut shared: Vec<World> = (0..4).map(|_| bot(Some(&cache))).collect();
        let mut plain: Vec<World> = (0..4).map(|_| bot(None)).collect();
        let (mut met, mut most) = (0usize, 0usize);
        for step in 0..2500 {
            let op = Op::random(&mut r);
            // Mostly every world (the same server), sometimes only some (a prediction, a
            // packet the others get later or never).
            let mask = if r.below(4) == 0 { r.below(16) } else { 15 };
            for k in 0..4 {
                if mask & (1 << k) == 0 {
                    continue;
                }
                let want = op.apply(&mut plain[k]).map_err(|e| e.to_string());
                let got = op.apply(&mut shared[k]).map_err(|e| e.to_string());
                assert_eq!(got, want, "step {step} world {k}: {op:?}");
            }
            if r.below(200) == 0 {
                let k = r.below(4) as usize;
                let to = [None, Some(&cache), Some(&other)][r.below(3) as usize];
                shared[k].set_shared_chunks(to.cloned());
            }
            for k in 0..4 {
                assert_same(&plain[k], &shared[k]);
            }
            for k in 1..4 {
                if let (Some(a), Some(b)) = (shared[0].column(0, 0), shared[k].column(0, 0)) {
                    met += usize::from(Arc::ptr_eq(&a.sections, &b.sections));
                }
            }
            most = most.max(cache.sections());
            assert!(cache.sections() <= 4 * 2 * 5, "{}", cache.sections());
        }
        assert!(met > 300 && most > 3, "{met} {most}");
        drop(shared);
        // A world that changed stores cannot tell its old store about the arrays it drops:
        // those entries are dead (nothing is held) until the table's next sweep.
        assert_eq!((cache.entries(), cache.columns()), (0, 0));
        assert_eq!((other.entries(), other.columns()), (0, 0));
        assert!(cache.column_entries() + other.column_entries() <= 8);
    }

    /// The same on four threads at once (one sharing world and its unshared twin per thread,
    /// one store): racing lookups, in-place changes and last drops never show a world
    /// anything but its own packets.
    #[test]
    fn random_packets_on_threads_read_like_unshared_worlds() {
        let cache = SharedChunks::new();
        std::thread::scope(|s| {
            for t in 0..4u64 {
                let cache = cache.clone();
                s.spawn(move || {
                    // Two threads replay one stream (they meet all the time), two their own.
                    let mut r = Rng(0x1234_5678_9abc_def1 + t / 2);
                    let (mut shared, mut plain) = (bot(Some(&cache)), bot(None));
                    for step in 0..4000 {
                        let op = Op::random(&mut r);
                        let want = op.apply(&mut plain).map_err(|e| e.to_string());
                        let got = op.apply(&mut shared).map_err(|e| e.to_string());
                        assert_eq!(got, want, "thread {t} step {step}: {op:?}");
                        if step % 16 == 0 {
                            assert_same(&plain, &shared);
                        }
                    }
                    assert_same(&plain, &shared);
                });
            }
        });
        assert_eq!(cache.entries(), 0);
        assert_eq!(cache.columns(), 0);
    }

    #[test]
    fn unload_frees_the_entry() {
        let cache = SharedChunks::new();
        let (mut a, mut b) = (overworld(), overworld());
        a.set_shared_chunks(Some(cache.clone()));
        b.set_shared_chunks(Some(cache.clone()));
        for w in [&mut a, &mut b] {
            w.apply_map_chunk_body(&chunk(0, 0, STONE, DIRT)).unwrap();
            w.apply_map_chunk_body(&chunk(1, 0, DIRT, STONE)).unwrap();
        }
        assert_eq!(cache.entries(), 2);
        assert!(cache.heap_bytes() > 2 * 512);
        let unload = |w: &mut World, x| {
            w.apply_unload_chunk(&UnloadChunk {
                chunk_z: 0,
                chunk_x: x,
            })
        };
        assert_eq!(unload(&mut a, 0), Applied::Changed);
        assert_eq!(cache.entries(), 2, "b still holds (0, 0)");
        assert_eq!(unload(&mut b, 0), Applied::Changed);
        assert_eq!(cache.entries(), 1);
        b.clear();
        assert_eq!(cache.entries(), 1, "a still holds (1, 0)");
        drop(a);
        assert_eq!((cache.entries(), cache.sections()), (0, 0));
        // Worlds outlive the store handle they were given.
        let mut c = overworld();
        c.set_shared_chunks(Some(SharedChunks::new()));
        c.apply_map_chunk_body(&chunk(0, 0, STONE, DIRT)).unwrap();
        c.set_shared_chunks(None);
        assert!(sec(&c, 0, 0).is_shared());
        assert_eq!(c.block_state(3, 63, 3).unwrap().id(), STONE);
        drop(c);
    }

    #[test]
    fn a_later_full_chunk_replaces_shared_sections() {
        let cache = SharedChunks::new();
        let (mut a, mut b) = (overworld(), overworld());
        a.set_shared_chunks(Some(cache.clone()));
        b.set_shared_chunks(Some(cache.clone()));
        a.apply_map_chunk_body(&chunk(0, 0, STONE, DIRT)).unwrap();
        b.apply_map_chunk_body(&chunk(0, 0, STONE, DIRT)).unwrap();
        // a gets new contents for the same column; b keeps the old ones.
        a.apply_map_chunk_body(&chunk(0, 0, DIRT, STONE)).unwrap();
        assert_eq!(cache.entries(), 2);
        assert_eq!(a.block_state(4, 63, 4).unwrap().id(), DIRT);
        assert_eq!(b.block_state(4, 63, 4).unwrap().id(), STONE);
        assert!(!same_storage(sec(&a, 0, 0), sec(&b, 0, 0)));
        // The same contents again: a's own entry is reused, b's old one dies.
        b.apply_map_chunk_body(&chunk(0, 0, DIRT, STONE)).unwrap();
        assert!(same_storage(sec(&a, 0, 0), sec(&b, 0, 0)));
        assert_eq!(cache.entries(), 1);
        // Same bytes at another column position share too (the key is the section bytes).
        b.apply_map_chunk_body(&chunk(3, -2, DIRT, STONE)).unwrap();
        assert!(same_storage(sec(&a, 0, 0), sec(&b, 3, -2)));
        assert_eq!(cache.entries(), 1);
    }

    #[test]
    fn malformed_sections_are_not_stored() {
        let cache = SharedChunks::new();
        let mut a = overworld();
        a.set_shared_chunks(Some(cache.clone()));
        // Palette entry 1 used but the palette has one entry: an error, nothing stored.
        let mut body = chunk(0, 0, STONE, DIRT).to_vec();
        let at = body
            .windows(4)
            .position(|w| w == [4, 2, DIRT as u8, STONE as u8])
            .unwrap();
        body[at + 1] = 1;
        body.remove(at + 3);
        let n = body.len();
        let err = a.apply_map_chunk_body(&Bytes::from(body));
        assert!(err.is_err(), "{n} {err:?}");
        assert!(a.column(0, 0).is_none());
        assert_eq!(cache.entries(), 0);
    }

    #[test]
    fn batch_size_matches_vanilla_formula() {
        let mut b = ChunkBatchSize::default();
        assert_eq!(b.desired_chunks_per_tick(), 3.5);
        b.on_batch_start(0);
        // 10 chunks in 1 ms: 100 µs per chunk, clamped to 2e6 / 3.
        let v = b.on_batch_finished(10, 1_000_000);
        let agg = (2_000_000.0 + 2_000_000.0 / 3.0) / 2.0;
        assert_eq!(v, (7_000_000.0 / agg) as f32);
        // empty batches change nothing
        assert_eq!(b.on_batch_finished(0, 5), v);
    }
}
