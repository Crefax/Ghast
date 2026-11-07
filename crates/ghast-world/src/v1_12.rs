//! The 1.12.2 (protocol 340) world: paletted `id << 4 | meta` sections, `map_chunk`,
//! `unload_chunk`, block changes and explosions. The first member of the 1.9 – 1.12 chunk family
//! (same section format since 1.9; 1.9.4 added the block entity list to `map_chunk`).
//!
//! ## Chunk data format (vanilla 1.12 `Chunk.method_3895` / `PaletteContainer.read`)
//!
//! For every section in the mask, in ascending Y:
//!
//! - one byte `bits`; `<= 4` reads as 4 with a linear palette of up to 16 entries, `5..=8` a hash
//!   palette with `bits`-bit indices, anything larger the global palette with [`GLOBAL_BITS`] bits
//!   (vanilla ignores the sent value);
//! - the palette: VarInt length + VarInt raw states (the global palette still sends a VarInt,
//!   normally 0, which is read and ignored);
//! - the data: a VarInt-prefixed `i64` array in the 1.9 – 1.15 layout, values may span two words
//!   ([`crate::packed::unpack_spanning_be`]);
//! - 2048 bytes of block light, and 2048 bytes of sky light in the overworld only.
//!
//! A ground-up chunk then carries 256 biome bytes. Raw states that are not registered (unknown
//! ids, metas the block never produces) and indices past the palette read as air, as vanilla's
//! palettes return null there and the container substitutes air. A data array whose length does
//! not match `bits` is rejected (vanilla would keep the old storage) and the column is unloaded.

use ghast_data::v1_12::BlockState;
use ghast_protocol::v1_12_2::play::clientbound::{
    self as cb, BlockChange, ClientboundPlayPacket, Explosion, Login, MapChunk, MultiBlockChange,
    Respawn, TileEntityData, UnloadChunk,
};

use crate::border::{Border, LerpMath, border_packet};
use crate::column::{Biomes, BlockEntity, Column, ColumnMap, MAX_Y, SECTIONS, SectionLight};
use crate::packed::{unpack_spanning_be, words_needed_spanning};
use crate::section::{SECTION_VOLUME, SectionBuilder};
use crate::{Applied, ChunkPos, WorldConfig, WorldError};

/// Bits per block of the 1.12.2 global palette (`log2DeBruijn` of the 5365 registered states).
pub const GLOBAL_BITS: u32 = 13;

/// Raw states are `< 4096`.
const STATE_SPACE: usize = 4096;
/// Bytes of one light array per section.
const LIGHT_BYTES: usize = 2048;
/// Biome bytes of a ground-up chunk.
const BIOME_BYTES: usize = 256;

/// Clientbound play packet ids this world consumes (for a `PacketFilter`).
pub const PACKET_IDS: [i32; 8] = [
    cb::id::LOGIN,
    cb::id::RESPAWN,
    cb::id::MAP_CHUNK,
    cb::id::UNLOAD_CHUNK,
    cb::id::BLOCK_CHANGE,
    cb::id::MULTI_BLOCK_CHANGE,
    cb::id::EXPLOSION,
    cb::id::WORLD_BORDER,
];

/// Extra packet ids with [`WorldConfig::block_entities`] (chunk block entities come with
/// `map_chunk`).
pub const BLOCK_ENTITY_IDS: [i32; 1] = [cb::id::TILE_ENTITY_DATA];

/// Same block id (any meta): the condition under which vanilla keeps a block entity.
fn same_block(a: u16, b: u16) -> bool {
    a >> 4 == b >> 4
}

/// Vanilla's `World.isValidPos` horizontal limit.
const MAX_XZ: i32 = 30_000_000;

/// The client-side world of a 1.12.2 connection.
pub struct World {
    columns: ColumnMap,
    config: WorldConfig,
    dimension: i32,
    builder: SectionBuilder,
    border: Border,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World")
            .field("dimension", &self.dimension)
            .field("columns", &self.columns.len())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl World {
    /// An empty world (nothing loaded, overworld).
    #[must_use]
    pub fn new(config: WorldConfig) -> Self {
        World {
            columns: ColumnMap::default(),
            config,
            dimension: 0,
            builder: SectionBuilder::new(STATE_SPACE),
            border: Border::new(LerpMath::Float),
        }
    }

    /// The block state at a position, or `None` when its column is not loaded. Positions
    /// outside `0..256` in a loaded column are air.
    #[inline]
    #[must_use]
    pub fn block_state(&self, x: i32, y: i32, z: i32) -> Option<BlockState> {
        self.columns.raw(x, y, z).map(state)
    }

    /// `true` when the column `(chunk_x, chunk_z)` is loaded.
    #[inline]
    #[must_use]
    pub fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.columns.contains(chunk_x, chunk_z)
    }

    /// A loaded column (`BlockState::from_raw_vanilla(column.raw(x, y, z))`).
    #[must_use]
    pub fn column(&self, chunk_x: i32, chunk_z: i32) -> Option<&Column> {
        self.columns.get(chunk_x, chunk_z)
    }

    /// All loaded columns.
    #[must_use]
    pub fn columns(&self) -> &ColumnMap {
        &self.columns
    }

    /// Current dimension (-1 nether, 0 overworld, 1 end).
    #[must_use]
    pub fn dimension(&self) -> i32 {
        self.dimension
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> WorldConfig {
        self.config
    }

    /// Approximate heap bytes used by the loaded columns.
    #[must_use]
    pub fn heap_bytes(&self) -> usize {
        self.columns.heap_bytes()
    }

    /// Sets a block locally (client-side prediction). Returns `false` and does nothing when the
    /// column is not loaded or the position is outside the world.
    pub fn set_block_state(&mut self, x: i32, y: i32, z: i32, s: BlockState) -> bool {
        set_block(&mut self.columns, x, y, z, s.raw())
    }

    /// The world border.
    #[must_use]
    pub fn border(&self) -> &Border {
        &self.border
    }

    /// Advances time-dependent world state (a moving world border) by `ms` milliseconds; the
    /// client calls this once per tick with 50.
    pub fn advance_time(&mut self, ms: i64) {
        self.border.advance(ms);
    }

    /// `world_border`.
    pub fn apply_world_border(&mut self, p: &cb::WorldBorder) -> Applied {
        changed(self.border.apply(&border_packet!(p)))
    }

    /// Unloads everything.
    pub fn clear(&mut self) {
        self.columns.clear();
    }

    /// Applies any world-relevant packet; other packets return [`Applied::NotWorldPacket`].
    ///
    /// # Errors
    /// Malformed chunk data (the affected column is unloaded).
    pub fn handle(&mut self, packet: &ClientboundPlayPacket) -> Result<Applied, WorldError> {
        match packet {
            ClientboundPlayPacket::Login(p) => Ok(self.apply_login(p)),
            ClientboundPlayPacket::Respawn(p) => Ok(self.apply_respawn(p)),
            ClientboundPlayPacket::MapChunk(p) => self.apply_map_chunk(p),
            ClientboundPlayPacket::UnloadChunk(p) => Ok(self.apply_unload_chunk(p)),
            ClientboundPlayPacket::BlockChange(p) => Ok(self.apply_block_change(p)),
            ClientboundPlayPacket::MultiBlockChange(p) => Ok(self.apply_multi_block_change(p)),
            ClientboundPlayPacket::Explosion(p) => Ok(self.apply_explosion(p)),
            ClientboundPlayPacket::WorldBorder(p) => Ok(self.apply_world_border(p)),
            ClientboundPlayPacket::TileEntityData(p) if self.config.block_entities => {
                Ok(self.apply_tile_entity_data(p))
            }
            _ => Ok(Applied::NotWorldPacket),
        }
    }

    /// The packet ids this world consumes with its configuration (for a `PacketFilter`).
    pub fn packet_ids(&self) -> impl Iterator<Item = i32> + use<> {
        let extra: &'static [i32] = if self.config.block_entities {
            &BLOCK_ENTITY_IDS
        } else {
            &[]
        };
        PACKET_IDS.into_iter().chain(extra.iter().copied())
    }

    /// Join game: vanilla creates a new client world, so everything is unloaded.
    pub fn apply_login(&mut self, p: &Login) -> Applied {
        self.columns.clear();
        self.border = Border::new(LerpMath::Float);
        self.dimension = p.dimension;
        Applied::Changed
    }

    /// Respawn: vanilla replaces the world only when the dimension changes; a respawn into the
    /// same dimension keeps every loaded column.
    pub fn apply_respawn(&mut self, p: &Respawn) -> Applied {
        if p.dimension == self.dimension {
            return Applied::Ignored;
        }
        self.columns.clear();
        self.border = Border::new(LerpMath::Float);
        self.dimension = p.dimension;
        Applied::Changed
    }

    /// `unload_chunk`.
    pub fn apply_unload_chunk(&mut self, p: &UnloadChunk) -> Applied {
        changed(self.columns.remove(p.chunk_x, p.chunk_z))
    }

    /// `map_chunk`. A ground-up chunk replaces the column (sections outside the mask become
    /// air); a partial chunk replaces only the masked sections of a loaded column and is ignored
    /// for an unloaded one (vanilla writes it into the shared empty chunk, which always reads as
    /// air).
    ///
    /// # Errors
    /// Malformed section data (the column is unloaded).
    pub fn apply_map_chunk(&mut self, p: &MapChunk) -> Result<Applied, WorldError> {
        let r = self.apply_chunk_data(p.x, p.z, p.ground_up, p.bit_map, &p.chunk_data)?;
        if r == Applied::Changed && self.config.block_entities {
            self.apply_block_entities(&p.block_entities);
        }
        Ok(r)
    }

    /// Stores a chunk's block entity list (with [`WorldConfig::block_entities`]): each compound
    /// goes to the loaded column containing its `x` / `y` / `z` ints; entries without them are
    /// skipped.
    pub fn apply_block_entities(&mut self, list: &[ghast_protocol::nbt::RawNbt]) {
        use ghast_protocol::nbt::Tag;
        for nbt in list {
            let Ok(Some(tree)) = nbt.parse() else {
                continue;
            };
            let Tag::Compound(c) = &tree.tag else {
                continue;
            };
            let int = |k: &str| match c.get(k) {
                Some(Tag::Int(v)) => Some(*v),
                _ => None,
            };
            let (Some(x), Some(y), Some(z)) = (int("x"), int("y"), int("z")) else {
                continue;
            };
            crate::column::put_nbt(
                &mut self.columns,
                ghast_protocol::BlockPos { x, y, z },
                0,
                Some(nbt),
            );
        }
    }

    /// The block entity at a position (`None` when absent or the column is not loaded).
    #[must_use]
    pub fn block_entity(&self, x: i32, y: i32, z: i32) -> Option<&BlockEntity> {
        self.columns.get(x >> 4, z >> 4)?.block_entity(x, y, z)
    }

    /// `tile_entity_data` (with [`WorldConfig::block_entities`]).
    pub fn apply_tile_entity_data(&mut self, p: &TileEntityData) -> Applied {
        if !self.config.block_entities {
            return Applied::Ignored;
        }
        crate::column::put_nbt(&mut self.columns, p.location, p.action, p.nbt_data.as_ref())
    }

    /// [`World::apply_map_chunk`] without the packet struct.
    ///
    /// # Errors
    /// Malformed section data (the column is unloaded).
    pub fn apply_chunk_data(
        &mut self,
        x: i32,
        z: i32,
        full: bool,
        mask: i32,
        data: &[u8],
    ) -> Result<Applied, WorldError> {
        if !full && !self.columns.contains(x, z) {
            return Ok(Applied::Ignored);
        }
        let has_sky = self.dimension == 0;
        let d = match decode(
            &mut self.builder,
            self.config,
            x,
            z,
            full,
            mask,
            data,
            has_sky,
        ) {
            Ok(d) => d,
            Err(e) => {
                self.columns.remove(x, z);
                return Err(e);
            }
        };
        if full {
            self.columns.insert(
                x,
                z,
                Column {
                    sections: d.sections,
                    biomes: d.biomes,
                    light: self.config.light.then(|| Box::new(d.light)),
                    block_entities: Vec::new(),
                },
            );
        } else if let Some(col) = self.columns.get_mut(x, z) {
            // Vanilla resets the block entities of the replaced sections.
            col.remove_block_entities_in(mask as u32 & 0xFFFF);
            let (mut sections, mut light) = (d.sections, d.light);
            let keep_light = self.config.light;
            let col_light = if keep_light {
                Some(col.light.get_or_insert_with(Default::default))
            } else {
                None
            };
            let mut col_light = col_light;
            for j in 0..SECTIONS {
                if mask & (1 << j) != 0 {
                    col.sections[j] = sections[j].take();
                    if let Some(l) = col_light.as_deref_mut() {
                        l[j] = light[j].take();
                    }
                }
            }
        }
        Ok(Applied::Changed)
    }

    /// `block_change`. Unregistered states read as air; unloaded columns and positions outside
    /// the world are ignored.
    pub fn apply_block_change(&mut self, p: &BlockChange) -> Applied {
        changed(set_block(
            &mut self.columns,
            p.location.x,
            p.location.y,
            p.location.z,
            raw_from_wire(p.r#type),
        ))
    }

    /// `multi_block_change` (records `x << 4 | z`, `y`, state).
    pub fn apply_multi_block_change(&mut self, p: &MultiBlockChange) -> Applied {
        let Some(col) = self.columns.get_mut(p.chunk_x, p.chunk_z) else {
            return Applied::Ignored;
        };
        for r in &p.records {
            let x = i32::from(r.horizontal_pos >> 4);
            let z = i32::from(r.horizontal_pos & 15);
            col.set_raw_with(x, i32::from(r.y), z, raw_from_wire(r.block_id), same_block);
        }
        Applied::Changed
    }

    /// `explosion`: every listed block whose material is not air becomes air. Offsets are
    /// relative to the explosion position truncated towards zero (vanilla `(int) x`).
    pub fn apply_explosion(&mut self, p: &Explosion) -> Applied {
        let bx = f64::from(p.x) as i32;
        let by = f64::from(p.y) as i32;
        let bz = f64::from(p.z) as i32;
        let mut any = false;
        for o in &p.affected_block_offsets {
            let (x, y, z) = (
                bx.wrapping_add(i32::from(o.x)),
                by.wrapping_add(i32::from(o.y)),
                bz.wrapping_add(i32::from(o.z)),
            );
            if self.columns.raw(x, y, z).is_some_and(|s| {
                state(s).block().material().kind != ghast_data::v1_12::MaterialKind::Air
            }) {
                any |= set_block(&mut self.columns, x, y, z, 0);
            }
        }
        changed(any)
    }
}

#[inline]
fn state(raw: u16) -> BlockState {
    BlockState::from_raw(raw).unwrap_or(BlockState::AIR)
}

#[inline]
fn changed(b: bool) -> Applied {
    if b {
        Applied::Changed
    } else {
        Applied::Ignored
    }
}

/// `Block.BLOCK_STATES.fromId` with null as air.
#[inline]
fn raw_from_wire(v: i32) -> u16 {
    u16::try_from(v).map_or(0, |r| BlockState::from_raw_vanilla(r).raw())
}

fn set_block(columns: &mut ColumnMap, x: i32, y: i32, z: i32, raw: u16) -> bool {
    if !(0..MAX_Y).contains(&y)
        || !(-MAX_XZ..MAX_XZ).contains(&x)
        || !(-MAX_XZ..MAX_XZ).contains(&z)
    {
        return false;
    }
    match columns.get_mut(x >> 4, z >> 4) {
        Some(c) => {
            c.set_raw_with(x, y, z, raw, same_block);
            true
        }
        None => false,
    }
}

/// A decoded chunk packet, ready to commit.
struct Decoded {
    sections: [Option<crate::Section>; SECTIONS],
    light: [Option<SectionLight>; SECTIONS],
    biomes: Biomes,
}

/// Minimal cursor over section data.
struct Cursor<'a> {
    buf: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Option<u8> {
        let (&b, rest) = self.buf.split_first()?;
        self.buf = rest;
        Some(b)
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.buf.len() < n {
            return None;
        }
        let (a, b) = self.buf.split_at(n);
        self.buf = b;
        Some(a)
    }

    fn varint(&mut self) -> Option<i32> {
        let mut v: u32 = 0;
        for i in 0..5 {
            let b = self.u8()?;
            v |= u32::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                return Some(v as i32);
            }
        }
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn decode(
    builder: &mut SectionBuilder,
    config: WorldConfig,
    x: i32,
    z: i32,
    full: bool,
    mask: i32,
    data: &[u8],
    has_sky: bool,
) -> Result<Decoded, WorldError> {
    let mut out = Decoded {
        sections: Default::default(),
        light: Default::default(),
        biomes: Biomes::None,
    };
    let mut c = Cursor { buf: data };
    for j in 0..SECTIONS {
        if mask & (1 << j) == 0 {
            continue;
        }
        let err = |reason| WorldError::Section {
            pos: ChunkPos::new(x, z),
            section: j as u8,
            reason,
        };
        out.sections[j] = decode_section(builder, &mut c).map_err(err)?;
        let block = c.take(LIGHT_BYTES).ok_or("truncated light").map_err(err)?;
        let sky = if has_sky {
            Some(
                c.take(LIGHT_BYTES)
                    .ok_or("truncated sky light")
                    .map_err(err)?,
            )
        } else {
            None
        };
        if config.light {
            out.light[j] = Some(SectionLight {
                block: nibbles(block),
                sky: sky.map(nibbles),
            });
        }
    }
    if full {
        let b = c.take(BIOME_BYTES).ok_or(WorldError::Truncated {
            pos: ChunkPos::new(x, z),
            len: data.len(),
            need: data.len() - c.buf.len() + BIOME_BYTES,
        })?;
        if config.biomes {
            let mut bx = Box::new([0u8; BIOME_BYTES]);
            bx.copy_from_slice(b);
            out.biomes = Biomes::Legacy(bx);
        }
    }
    Ok(out)
}

fn decode_section(
    builder: &mut SectionBuilder,
    c: &mut Cursor<'_>,
) -> Result<Option<crate::Section>, &'static str> {
    const EOF: &str = "truncated";
    let sent = c.u8().ok_or(EOF)? as i8;
    let bits: u32 = match sent {
        i8::MIN..=4 => 4,
        5..=8 => sent as u32,
        _ => GLOBAL_BITS,
    };
    let mut remap = [0u16; 256];
    let mut remap_len = 0;
    let len = c.varint().ok_or(EOF)?;
    if bits <= 8 {
        let len = usize::try_from(len).map_err(|_| "negative palette length")?;
        if bits == 4 && len > 16 {
            return Err("linear palette longer than 16");
        }
        if len > c.buf.len() {
            return Err(EOF);
        }
        for i in 0..len {
            let id = c.varint().ok_or(EOF)?;
            if let Some(r) = remap.get_mut(i) {
                *r = raw_from_wire(id);
            }
        }
        remap_len = len.min(256);
    }
    let words = c.varint().ok_or(EOF)?;
    let need = words_needed_spanning(bits, SECTION_VOLUME);
    if usize::try_from(words).ok() != Some(need) {
        return Err("data array length does not match bits per block");
    }
    let data = c.take(need * 8).ok_or(EOF)?;
    if bits <= 8 {
        Ok(builder.build_packed_layout(&remap[..remap_len], bits, data, true))
    } else {
        let mut states = [0u16; SECTION_VOLUME];
        unpack_spanning_be(bits, data, SECTION_VOLUME, |i, v| {
            states[i] = raw_from_wire(v as i32);
        });
        Ok(builder.build(&states))
    }
}

fn nibbles(s: &[u8]) -> Box<[u8; LIGHT_BYTES]> {
    let mut b = Box::new([0u8; LIGHT_BYTES]);
    b.copy_from_slice(s);
    b
}

#[cfg(feature = "physics")]
impl ghast_physics::BlockView for World {
    type State = BlockState;

    /// Vanilla semantics: an unloaded column is an empty chunk, so it reads as air.
    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> BlockState {
        self.columns.raw(x, y, z).map_or(BlockState::AIR, state)
    }

    #[inline]
    fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.columns.contains(chunk_x, chunk_z)
    }

    fn world_border(&self) -> ghast_physics::WorldBorder {
        let (center_x, center_z) = self.border.center();
        ghast_physics::WorldBorder {
            center_x,
            center_z,
            size: self.border.size(),
        }
    }
}
