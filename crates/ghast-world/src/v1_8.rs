//! The 1.8.x (protocol 47) world: `id << 4 | meta` sections, `map_chunk`, `map_chunk_bulk`,
//! block changes and explosions.
//!
//! ## Chunk data format (vanilla `Chunk.fillChunk`)
//!
//! For a section mask with `n` bits set, the data is, in this order:
//! `n × 4096` little-endian `u16` block states (sections in ascending Y), `n × 2048` bytes of block
//! light, `n × 2048` bytes of sky light (only in dimensions with sky), and 256 biome bytes when
//! the chunk is ground-up. Vanilla takes the sky-light presence from the client's dimension; this
//! decoder takes it from the data length when that is unambiguous (exactly one of the two layouts
//! fits) and from the dimension otherwise, so a server/client dimension mismatch cannot shift the
//! biome bytes.

use ghast_data::v1_8::{BLOCK_COUNT, BlockState};
use ghast_protocol::v1_8::play::clientbound::{
    self as cb, BlockChange, ClientboundPlayPacket, Explosion, Login, MapChunk, MapChunkBulk,
    MultiBlockChange, Respawn,
};

use crate::border::{Border, LerpMath, border_packet};
use crate::column::{Biomes, Column, ColumnMap, MAX_Y, SECTIONS, SectionLight};
use crate::section::{SECTION_VOLUME, SectionBuilder};
use crate::{Applied, ChunkPos, WorldConfig, WorldError};

/// Bytes of block states per section.
const BLOCK_BYTES: usize = 4096 * 2;
/// Bytes of one light array per section.
const LIGHT_BYTES: usize = 2048;
/// Biome bytes of a ground-up chunk.
const BIOME_BYTES: usize = 256;

/// Every canonical 1.8 state is `< 4096`.
const STATE_SPACE: usize = 4096;
const _: () = assert!(BLOCK_COUNT * 16 <= STATE_SPACE);

/// Clientbound play packet ids this world consumes (for a `PacketFilter`).
pub const PACKET_IDS: [i32; 8] = [
    cb::id::LOGIN,
    cb::id::RESPAWN,
    cb::id::MAP_CHUNK,
    cb::id::MAP_CHUNK_BULK,
    cb::id::BLOCK_CHANGE,
    cb::id::MULTI_BLOCK_CHANGE,
    cb::id::EXPLOSION,
    cb::id::WORLD_BORDER,
];

/// Vanilla's `World.isValid` horizontal limit.
const MAX_XZ: i32 = 30_000_000;

/// The client-side world of a 1.8 connection.
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

    /// A loaded column (read blocks of one column without re-hashing:
    /// `BlockState::from_raw_vanilla(column.raw(x, y, z))`).
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

    /// Sets a block locally (client-side prediction, as vanilla does when it breaks or places a
    /// block before the server confirms). Returns `false` and does nothing when the column is not
    /// loaded or the position is outside the world.
    pub fn set_block_state(&mut self, x: i32, y: i32, z: i32, s: BlockState) -> bool {
        set_block(
            &mut self.columns,
            x,
            y,
            z,
            BlockState::from_raw_vanilla(s.raw()).raw(),
        )
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

    /// Unloads everything (keeps the dimension).
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
            ClientboundPlayPacket::MapChunkBulk(p) => self.apply_map_chunk_bulk(p),
            ClientboundPlayPacket::BlockChange(p) => Ok(self.apply_block_change(p)),
            ClientboundPlayPacket::MultiBlockChange(p) => Ok(self.apply_multi_block_change(p)),
            ClientboundPlayPacket::Explosion(p) => Ok(self.apply_explosion(p)),
            ClientboundPlayPacket::WorldBorder(p) => Ok(self.apply_world_border(p)),
            _ => Ok(Applied::NotWorldPacket),
        }
    }

    /// Join game: vanilla creates a new client world, so everything is unloaded.
    pub fn apply_login(&mut self, p: &Login) -> Applied {
        self.columns.clear();
        self.border = Border::new(LerpMath::Float);
        self.dimension = i32::from(p.dimension);
        Applied::Changed
    }

    /// Respawn: vanilla 1.8 replaces the world only when the dimension changes; a respawn into
    /// the same dimension keeps every loaded column.
    pub fn apply_respawn(&mut self, p: &Respawn) -> Applied {
        if p.dimension == self.dimension {
            return Applied::Ignored;
        }
        self.columns.clear();
        self.border = Border::new(LerpMath::Float);
        self.dimension = p.dimension;
        Applied::Changed
    }

    /// `map_chunk`: ground-up with an empty mask unloads the column; ground-up otherwise replaces
    /// it; a partial chunk replaces the masked sections of a loaded column and is ignored for an
    /// unloaded one.
    ///
    /// # Errors
    /// Data too short for the mask (the column is unloaded).
    pub fn apply_map_chunk(&mut self, p: &MapChunk) -> Result<Applied, WorldError> {
        self.apply_chunk_data(p.x, p.z, p.ground_up, p.bit_map, &p.chunk_data)
    }

    /// [`World::apply_map_chunk`] without the packet struct (`data` = the packet's chunk data).
    ///
    /// # Errors
    /// Data too short for the mask (the column is unloaded).
    pub fn apply_chunk_data(
        &mut self,
        x: i32,
        z: i32,
        ground_up: bool,
        mask: u16,
        data: &[u8],
    ) -> Result<Applied, WorldError> {
        if ground_up && mask == 0 {
            let had = self.columns.remove(x, z);
            return Ok(if had {
                Applied::Changed
            } else {
                Applied::Ignored
            });
        }
        if !ground_up && !self.columns.contains(x, z) {
            return Ok(Applied::Ignored);
        }
        let has_sky = self.dimension == 0;
        match decode(
            &mut self.builder,
            self.config,
            x,
            z,
            ground_up,
            mask,
            data,
            has_sky,
        ) {
            Ok(d) => {
                self.commit(x, z, ground_up, d);
                Ok(Applied::Changed)
            }
            Err(e) => {
                self.columns.remove(x, z);
                Err(e)
            }
        }
    }

    /// `map_chunk_bulk`: every listed column is loaded ground-up (an empty mask loads an empty
    /// column, unlike `map_chunk`). The packet's sky-light flag splits the data.
    ///
    /// # Errors
    /// The data does not cover every column (nothing is applied, as vanilla fails the packet).
    pub fn apply_map_chunk_bulk(&mut self, p: &MapChunkBulk) -> Result<Applied, WorldError> {
        let per = |mask: u16| {
            let n = mask.count_ones() as usize;
            n * (BLOCK_BYTES + LIGHT_BYTES + if p.sky_light_sent { LIGHT_BYTES } else { 0 })
                + BIOME_BYTES
        };
        let need: usize = p.meta.iter().map(|m| per(m.bit_map)).sum();
        if p.data.len() < need {
            return Err(WorldError::BulkTruncated {
                len: p.data.len(),
                need,
            });
        }
        let mut off = 0;
        for m in &p.meta {
            let len = per(m.bit_map);
            let slice = &p.data[off..off + len];
            off += len;
            let d = decode(
                &mut self.builder,
                self.config,
                m.x,
                m.z,
                true,
                m.bit_map,
                slice,
                p.sky_light_sent,
            )?;
            self.commit(m.x, m.z, true, d);
        }
        Ok(Applied::Changed)
    }

    /// `block_change`. Unknown state values read as air (vanilla maps them through its state id
    /// map); unloaded columns and positions outside the world are ignored.
    pub fn apply_block_change(&mut self, p: &BlockChange) -> Applied {
        let s = raw_from_wire(p.r#type);
        changed(set_block(
            &mut self.columns,
            p.location.x,
            p.location.y,
            p.location.z,
            s,
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
            col.set_raw(x, i32::from(r.y), z, raw_from_wire(r.block_id));
        }
        Applied::Changed
    }

    /// `explosion`: every listed non-air block becomes air. Offsets are relative to the
    /// explosion position truncated towards zero (vanilla 1.8 `(int) x`).
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
            if self.columns.raw(x, y, z).is_some_and(|s| s >> 4 != 0) {
                any |= set_block(&mut self.columns, x, y, z, 0);
            }
        }
        changed(any)
    }

    fn commit(&mut self, x: i32, z: i32, ground_up: bool, d: Decoded) {
        if ground_up {
            let col = Column {
                sections: d.sections,
                biomes: d.biomes,
                light: self.config.light.then(|| Box::new(d.light)),
            };
            self.columns.insert(x, z, col);
            return;
        }
        let Some(col) = self.columns.get_mut(x, z) else {
            return;
        };
        let light = if self.config.light {
            Some(col.light.get_or_insert_with(Default::default))
        } else {
            None
        };
        let (mut sections, mut new_light) = (d.sections, d.light);
        let mut light = light;
        for j in 0..SECTIONS {
            if d.mask & (1 << j) != 0 {
                col.sections[j] = sections[j].take();
                if let Some(l) = light.as_deref_mut() {
                    l[j] = new_light[j].take();
                }
            }
        }
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

/// Vanilla's `Block.BLOCK_STATE_IDS.getByValue` with null as air.
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
            c.set_raw(x, y, z, raw);
            true
        }
        None => false,
    }
}

/// A decoded chunk packet, ready to commit.
struct Decoded {
    mask: u16,
    sections: [Option<crate::Section>; SECTIONS],
    light: [Option<SectionLight>; SECTIONS],
    biomes: Biomes,
}

#[allow(clippy::too_many_arguments)]
fn decode(
    builder: &mut SectionBuilder,
    config: WorldConfig,
    x: i32,
    z: i32,
    ground_up: bool,
    mask: u16,
    data: &[u8],
    dimension_has_sky: bool,
) -> Result<Decoded, WorldError> {
    let n = mask.count_ones() as usize;
    let blocks = n * BLOCK_BYTES;
    let light = n * LIGHT_BYTES;
    let no_sky = blocks + light + if ground_up { BIOME_BYTES } else { 0 };
    let with_sky = no_sky + light;
    let len = data.len();
    let sky = if n == 0 && len >= no_sky {
        false
    } else if n != 0 && len == with_sky {
        true
    } else if n != 0 && len == no_sky {
        false
    } else if dimension_has_sky && len >= with_sky {
        true
    } else if !dimension_has_sky && len >= no_sky {
        false
    } else {
        return Err(WorldError::Truncated {
            pos: ChunkPos::new(x, z),
            len,
            need: if dimension_has_sky { with_sky } else { no_sky },
        });
    };

    let mut out = Decoded {
        mask,
        sections: Default::default(),
        light: Default::default(),
        biomes: Biomes::None,
    };
    let mut k = 0;
    for j in 0..SECTIONS {
        if mask & (1 << j) == 0 {
            continue;
        }
        let mut states = [0u16; SECTION_VOLUME];
        let b = &data[k * BLOCK_BYTES..(k + 1) * BLOCK_BYTES];
        for (s, pair) in states.iter_mut().zip(b.chunks_exact(2)) {
            *s = BlockState::from_raw_vanilla(u16::from_le_bytes([pair[0], pair[1]])).raw();
        }
        out.sections[j] = builder.build(&states);
        if config.light {
            let bl = blocks + k * LIGHT_BYTES;
            let block = nibbles(&data[bl..bl + LIGHT_BYTES]);
            let sky = sky.then(|| {
                let sl = blocks + light + k * LIGHT_BYTES;
                nibbles(&data[sl..sl + LIGHT_BYTES])
            });
            out.light[j] = Some(SectionLight { block, sky });
        }
        k += 1;
    }
    if ground_up && config.biomes {
        let off = blocks + light + if sky { light } else { 0 };
        let mut b = Box::new([0u8; BIOME_BYTES]);
        b.copy_from_slice(&data[off..off + BIOME_BYTES]);
        out.biomes = Biomes::Legacy(b);
    }
    Ok(out)
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
