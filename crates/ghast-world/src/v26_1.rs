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

use bytes::Bytes;
use ghast_data::v26_1::{Block, BlockState, STATE_COUNT};
use ghast_protocol::RawPacket;
use ghast_protocol::codec::Reader;
use ghast_protocol::nbt::{AnonNbt, Tag};
use ghast_protocol::v26_1::configuration::clientbound::RegistryData;
use ghast_protocol::v26_1::play::clientbound::{
    self as cb, BlockChange, ChunkBiomes, ClientboundPlayPacket, InitializeWorldBorder, Login,
    MapChunk, MultiBlockChange, Respawn, TileEntityData, UnloadChunk, UpdateViewDistance,
    UpdateViewPosition,
};

use crate::border::{Border, BorderPacket, LerpMath};
use crate::column::{BlockEntity, ColumnMap};
use crate::paletted::{Builder, Section, WireBlocks, block_index};
use crate::{Applied, ChunkPos, WorldConfig, WorldError};

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
    sections: Box<[Section]>,
    biomes: Option<Box<[BiomeSection]>>,
    /// Block entities keyed by [`be_key`] (only with [`WorldConfig::block_entities`]).
    block_entities: Vec<(u32, BlockEntity)>,
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
            Some(s) if ry >= 0 => s.get(block_index(x, ry, z)),
            _ => 0,
        }
    }

    /// Biomes per section (only with [`WorldConfig::biomes`]).
    #[must_use]
    pub fn biomes(&self) -> Option<&[BiomeSection]> {
        self.biomes.as_deref()
    }

    /// Block entities as `([x, ry, z], data)` with `ry = y - min_y`, in arrival order. Empty
    /// unless [`WorldConfig::block_entities`] is on. [`BlockEntity::Nbt`]'s `action` is the
    /// block entity type's registry id (`u8::MAX` if it does not fit).
    pub fn block_entities(&self) -> impl Iterator<Item = ([i32; 3], &BlockEntity)> {
        self.block_entities.iter().map(|(k, b)| {
            (
                [(k & 15) as i32, (k >> 8) as i32, ((k >> 4) & 15) as i32],
                b,
            )
        })
    }

    fn put_block_entity(&mut self, key: u32, be: BlockEntity) {
        match self.block_entities.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = be,
            None => self.block_entities.push((key, be)),
        }
    }

    /// Heap bytes owned by this column.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        let sections = self.sections.len() * size_of::<Section>()
            + self.sections.iter().map(Section::heap_bytes).sum::<usize>();
        let biomes = self.biomes.as_ref().map_or(0, |b| {
            b.len() * size_of::<BiomeSection>()
                + b.iter()
                    .map(|s| match s {
                        BiomeSection::Single(_) => 0,
                        BiomeSection::Cells(_) => 128,
                    })
                    .sum::<usize>()
        });
        let be = self.block_entities.capacity() * size_of::<(u32, BlockEntity)>()
            + self
                .block_entities
                .iter()
                .map(|(_, b)| match b {
                    BlockEntity::Nbt { nbt, .. } => nbt.len(),
                    BlockEntity::Sign(l) => l.iter().map(|s| s.len()).sum(),
                })
                .sum::<usize>();
        sections + biomes + be
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
        }
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
    #[inline]
    #[must_use]
    pub fn block_state(&self, x: i32, y: i32, z: i32) -> Option<BlockState> {
        let c = self.columns.get(x >> 4, z >> 4)?;
        let min_y = self.dim.as_ref().map_or(0, |d| d.kind.min_y);
        Some(read(c, x, y.wrapping_sub(min_y), z))
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
        self.columns.clear();
    }

    /// Turns [`WorldConfig::biomes`] on or off at run time. On: columns that arrive (and
    /// `chunks_biomes` updates) from now on keep their biomes; columns already loaded have none
    /// until the server resends them. Off: every stored biome is dropped.
    pub fn set_biomes(&mut self, on: bool) {
        self.config.biomes = on;
        if !on {
            for (_, col) in self.columns.iter_mut() {
                col.biomes = None;
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
        let biomes = self.columns.get(qx >> 2, qz >> 2)?.biomes.as_deref()?;
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
            min_y: self.dim.as_ref().map_or(0, |d| d.kind.min_y),
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
        self.columns.clear();
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
        self.columns.retain(|p| view.contains(p.x, p.z));
        changed(self.columns.len() != before)
    }

    /// `forget_level_chunk`.
    pub fn apply_unload_chunk(&mut self, p: &UnloadChunk) -> Applied {
        changed(self.columns.remove(p.chunk_x, p.chunk_z))
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
        let n = (height >> 4) as usize;
        let biome_bits = ceil_log2(self.registries.biome_count.max(1));
        let biome_count = self.registries.biome_count;
        let keep_biomes = self.config.biomes;
        let mut r = Reader::new(data);
        let mut sections = Vec::with_capacity(n);
        let mut biomes = keep_biomes.then(|| Vec::with_capacity(n));
        for j in 0..n {
            let err = |reason| WorldError::Section {
                pos: ChunkPos::new(x, z),
                section: j.min(255) as u8,
                reason,
            };
            let s = read_section(&mut self.builder, &mut r).map_err(err);
            let s = match s {
                Ok(s) => s,
                Err(e) => {
                    self.columns.remove(x, z);
                    return Err(e);
                }
            };
            sections.push(s);
            let b = read_biomes(&mut r, biome_bits, biome_count).map_err(err);
            match b {
                Ok(b) => {
                    if let Some(v) = &mut biomes {
                        v.push(b);
                    }
                }
                Err(e) => {
                    self.columns.remove(x, z);
                    return Err(e);
                }
            }
        }
        let mut col = Column {
            sections: sections.into_boxed_slice(),
            biomes: biomes.map(Vec::into_boxed_slice),
            block_entities: Vec::new(),
        };
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
                        nbt: nbt.as_bytes().clone(),
                    },
                );
            }
        }
        self.columns.insert(x, z, col);
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
                        col.biomes = None;
                        return Err(WorldError::Section {
                            pos: ChunkPos::new(cx, cz),
                            section: j.min(255) as u8,
                            reason,
                        });
                    }
                }
            }
            col.biomes = Some(v.into_boxed_slice());
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
            .block_entities
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
                nbt: nbt.as_bytes().clone(),
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
        let Some(s) = col.sections.get_mut((ry >> 4) as usize) else {
            return false;
        };
        let old = s.set(block_index(x, ry, z), raw);
        if !col.block_entities.is_empty() && !same_block(old, raw) {
            let k = be_key(x, ry, z);
            col.block_entities.retain(|(key, _)| *key != k);
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

/// Block read inside a column (`ry = y - min_y`): void air outside the height.
#[inline]
fn read(c: &Column, x: i32, ry: i32, z: i32) -> BlockState {
    match c.sections.get((ry >> 4) as usize) {
        Some(s) if ry >= 0 => state(s.get(block_index(x, ry, z))),
        _ => void_air(),
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
            let data = r.read_bytes(longs * 8).map_err(|_| EOF)?;
            b.build(&WireBlocks {
                storage_bits: storage,
                palette: Some(&pal[..n.min(256)]),
                data: &data,
            })
        }
        _ => {
            let longs = 4096usize.div_ceil((64 / GLOBAL_BITS) as usize);
            let data = r.read_bytes(longs * 8).map_err(|_| EOF)?;
            b.build(&WireBlocks {
                storage_bits: GLOBAL_BITS,
                palette: None,
                data: &data,
            })
        }
    }
}

/// Reads one biome container (`count` = biome registry size, 0 = unknown: ids not checked).
fn read_biomes(
    r: &mut Reader<'_>,
    global_bits: u32,
    count: usize,
) -> Result<BiomeSection, &'static str> {
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
            return Ok(BiomeSection::Single(id));
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
    let data = r.read_bytes(longs * 8).map_err(|_| EOF)?;
    let mut cells = Box::new([0u16; 64]);
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
        return Ok(BiomeSection::Single(cells[0]));
    }
    Ok(BiomeSection::Cells(cells))
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
    #[inline]
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
    #[inline]
    #[must_use]
    pub fn block_state(&self, x: i32, y: i32, z: i32) -> Option<BlockState> {
        let c = self.column(x >> 4, z >> 4)?;
        Some(read(c, x, y.wrapping_sub(self.min_y), z))
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
        #[inline]
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

        #[inline]
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
