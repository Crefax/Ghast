//! The 1.16.5 (protocol 754) world: paletted sections, `map_chunk`, `unload_chunk`, block changes
//! (1.16.2+ section-position `multi_block_change`), explosions and the client's view window.
//!
//! ## Section format (vanilla `ChunkSection.fromPacket` / `PalettedContainer.fromPacket`)
//!
//! Per section in the mask (ascending Y): `i16` non-air count (ignored, recomputed), signed byte
//! `bits`, then
//!
//! - `bits <= 4`: array palette with 4-bit indices, at most 16 entries;
//! - `5..=8`: hash palette with `bits`-bit indices;
//! - otherwise: no palette, global ids with [`GLOBAL_BITS`] bits (vanilla ignores the sent value);
//!
//! palette = VarInt count + VarInt global ids, then the data as a VarInt-prefixed `i64` array
//! in the 1.16 compact layout (values never span words, see [`crate::packed`]). Palette ids that
//! are not block states, and indices past the palette, read as air (vanilla's palettes return
//! null there and the container substitutes its default, air). A data array whose length differs
//! from what `bits` requires is rejected (vanilla would silently keep stale storage).
//!
//! Heightmaps, block entities and light (`update_light`) are not stored.

use ghast_data::v1_16_5::{Block, BlockState, STATE_COUNT};
use ghast_protocol::v1_16_5::play::clientbound::{
    self as cb, BlockChange, ClientboundPlayPacket, Explosion, Login, MapChunk, MultiBlockChange,
    Respawn, UnloadChunk, UpdateViewDistance, UpdateViewPosition,
};

use crate::border::{Border, LerpMath, border_packet};
use crate::column::{Biomes, Column, ColumnMap, MAX_Y, SECTIONS};
use crate::packed::{unpack_be, words_needed};
use crate::section::{SECTION_VOLUME, SectionBuilder};
use crate::{Applied, ChunkPos, WorldConfig, WorldError};

/// Bits per block of the global palette: `ceil(log2(STATE_COUNT))` (15 for 1.16.5).
pub const GLOBAL_BITS: u32 = usize::BITS - (STATE_COUNT - 1).leading_zeros();

/// Clientbound play packet ids this world consumes (for a `PacketFilter`).
pub const PACKET_IDS: [i32; 10] = [
    cb::id::LOGIN,
    cb::id::RESPAWN,
    cb::id::MAP_CHUNK,
    cb::id::UNLOAD_CHUNK,
    cb::id::BLOCK_CHANGE,
    cb::id::MULTI_BLOCK_CHANGE,
    cb::id::EXPLOSION,
    cb::id::WORLD_BORDER,
    cb::id::UPDATE_VIEW_POSITION,
    cb::id::UPDATE_VIEW_DISTANCE,
];

/// The client's chunk window (vanilla `ClientChunkManager`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ViewWindow {
    distance: Option<i32>,
    center: Option<(i32, i32)>,
}

impl ViewWindow {
    /// Vanilla `getChunkMapRadius`: `max(2, view distance) + 3`.
    fn radius(self) -> Option<(i64, i64, i64)> {
        let d = self.distance?;
        let (x, z) = self.center?;
        Some((i64::from(d.max(2)) + 3, i64::from(x), i64::from(z)))
    }

    fn contains(self, cx: i32, cz: i32) -> bool {
        match self.radius() {
            Some((r, x, z)) => (i64::from(cx) - x).abs() <= r && (i64::from(cz) - z).abs() <= r,
            None => true,
        }
    }
}

/// The client-side world of a 1.16.5 connection.
pub struct World {
    columns: ColumnMap,
    config: WorldConfig,
    world_name: String,
    view: ViewWindow,
    builder: SectionBuilder,
    border: Border,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World")
            .field("world_name", &self.world_name)
            .field("view", &self.view)
            .field("columns", &self.columns.len())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// `void_air`: what vanilla 1.16 returns outside the build height and in unloaded columns.
#[inline]
fn void_air() -> BlockState {
    Block::VOID_AIR.default_state()
}

impl World {
    /// An empty world (nothing loaded, no view window yet).
    #[must_use]
    pub fn new(config: WorldConfig) -> Self {
        World {
            columns: ColumnMap::default(),
            config,
            world_name: String::new(),
            view: ViewWindow::default(),
            builder: SectionBuilder::new(STATE_COUNT),
            border: Border::new(LerpMath::Double),
        }
    }

    /// The block state at a position, or `None` when its column is not loaded. Positions
    /// outside `0..256` in a loaded column are `void_air`, as in vanilla.
    #[inline]
    #[must_use]
    pub fn block_state(&self, x: i32, y: i32, z: i32) -> Option<BlockState> {
        let c = self.columns.get(x >> 4, z >> 4)?;
        Some(if (0..MAX_Y).contains(&y) {
            state(c.raw(x, y, z))
        } else {
            void_air()
        })
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
    pub fn columns(&self) -> &ColumnMap {
        &self.columns
    }

    /// Current world (dimension) name, e.g. `minecraft:overworld`.
    #[must_use]
    pub fn world_name(&self) -> &str {
        &self.world_name
    }

    /// View distance from `login` / `update_view_distance`.
    #[must_use]
    pub fn view_distance(&self) -> Option<i32> {
        self.view.distance
    }

    /// Chunk window centre from `update_view_position`.
    #[must_use]
    pub fn view_center(&self) -> Option<ChunkPos> {
        self.view.center.map(|(x, z)| ChunkPos::new(x, z))
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
    /// column is not loaded or `y` is outside `0..256`.
    pub fn set_block_state(&mut self, x: i32, y: i32, z: i32, s: BlockState) -> bool {
        set_block(&mut self.columns, x, y, z, s.id())
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
            ClientboundPlayPacket::UpdateViewPosition(p) => Ok(self.apply_view_position(p)),
            ClientboundPlayPacket::UpdateViewDistance(p) => Ok(self.apply_view_distance(p)),
            ClientboundPlayPacket::WorldBorder(p) => Ok(self.apply_world_border(p)),
            _ => Ok(Applied::NotWorldPacket),
        }
    }

    /// Join game: a new client world (everything unloaded, window reset).
    pub fn apply_login(&mut self, p: &Login) -> Applied {
        self.columns.clear();
        self.border = Border::new(LerpMath::Double);
        self.world_name = p.world_name.to_string();
        self.view = ViewWindow {
            distance: Some(p.view_distance),
            center: None,
        };
        Applied::Changed
    }

    /// Respawn: vanilla replaces the world only when the world name differs; a respawn into the
    /// same world keeps every column.
    pub fn apply_respawn(&mut self, p: &Respawn) -> Applied {
        if *p.world_name == *self.world_name {
            return Applied::Ignored;
        }
        self.columns.clear();
        self.border = Border::new(LerpMath::Double);
        self.world_name = p.world_name.to_string();
        self.view.center = None;
        Applied::Changed
    }

    /// `update_view_position`: moves the window; columns outside it are dropped.
    pub fn apply_view_position(&mut self, p: &UpdateViewPosition) -> Applied {
        self.view.center = Some((p.chunk_x, p.chunk_z));
        self.purge_outside_window()
    }

    /// `update_view_distance`: resizes the window; columns outside it are dropped.
    pub fn apply_view_distance(&mut self, p: &UpdateViewDistance) -> Applied {
        self.view.distance = Some(p.view_distance);
        self.purge_outside_window()
    }

    fn purge_outside_window(&mut self) -> Applied {
        let before = self.columns.len();
        let view = self.view;
        if view.radius().is_some() {
            self.columns.retain(|p| view.contains(p.x, p.z));
        }
        if self.columns.len() == before {
            Applied::Ignored
        } else {
            Applied::Changed
        }
    }

    /// `unload_chunk`.
    pub fn apply_unload_chunk(&mut self, p: &UnloadChunk) -> Applied {
        if self.columns.remove(p.chunk_x, p.chunk_z) {
            Applied::Changed
        } else {
            Applied::Ignored
        }
    }

    /// `map_chunk`. Ignored outside the view window (once the window is known) and, for a
    /// partial chunk, when the column is not loaded. A full chunk replaces the column (sections
    /// outside the mask become air); a partial chunk replaces only the masked sections.
    ///
    /// # Errors
    /// Malformed section data (the column is unloaded).
    pub fn apply_map_chunk(&mut self, p: &MapChunk) -> Result<Applied, WorldError> {
        self.apply_chunk_data(
            p.x,
            p.z,
            p.ground_up,
            p.bit_map,
            p.biomes.as_deref(),
            &p.chunk_data,
        )
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
        biomes: Option<&[i32]>,
        data: &[u8],
    ) -> Result<Applied, WorldError> {
        if !self.view.contains(x, z) {
            return Ok(Applied::Ignored);
        }
        if !full && !self.columns.contains(x, z) {
            return Ok(Applied::Ignored);
        }
        let mut sections: [Option<crate::Section>; SECTIONS] = Default::default();
        if let Err(e) = decode_sections(&mut self.builder, x, z, mask, data, &mut sections) {
            self.columns.remove(x, z);
            return Err(e);
        }
        if full {
            let biomes = match biomes {
                Some(b) if self.config.biomes && b.len() == 1024 => {
                    let mut out = Box::new([0u16; 1024]);
                    for (o, &v) in out.iter_mut().zip(b) {
                        *o = u16::try_from(v).unwrap_or(u16::MAX);
                    }
                    Biomes::Cells(out)
                }
                _ => Biomes::None,
            };
            self.columns.insert(
                x,
                z,
                Column {
                    sections,
                    biomes,
                    light: None,
                },
            );
        } else if let Some(col) = self.columns.get_mut(x, z) {
            for (j, s) in sections.iter_mut().enumerate() {
                if mask & (1 << j) != 0 {
                    col.sections[j] = s.take();
                }
            }
        }
        Ok(Applied::Changed)
    }

    /// `block_change`. Unknown state ids read as air; unloaded columns are ignored.
    pub fn apply_block_change(&mut self, p: &BlockChange) -> Applied {
        changed(set_block(
            &mut self.columns,
            p.location.x,
            p.location.y,
            p.location.z,
            raw_from_wire(i64::from(p.r#type)),
        ))
    }

    /// `multi_block_change` (1.16.2+: section position + `state << 12 | x << 8 | z << 4 | y`).
    pub fn apply_multi_block_change(&mut self, p: &MultiBlockChange) -> Applied {
        let c = &p.chunk_coordinates;
        // 20-bit signed section Y.
        let sy = ((c.y << 12) as i32) >> 12;
        let Some(col) = self.columns.get_mut(c.x, c.z) else {
            return Applied::Ignored;
        };
        for &r in &p.records {
            let local = (r & 0xFFF) as i32;
            let y = sy.wrapping_mul(16).wrapping_add(local & 15);
            col.set_raw(
                (local >> 8) & 15,
                y,
                (local >> 4) & 15,
                raw_from_wire(((r as u64) >> 12) as i64),
            );
        }
        Applied::Changed
    }

    /// `explosion`: every listed block that is not air (`air`, `cave_air`, `void_air`) becomes
    /// air. Offsets are relative to the floored explosion position.
    pub fn apply_explosion(&mut self, p: &Explosion) -> Applied {
        let (bx, by, bz) = (floor(p.x), floor(p.y), floor(p.z));
        let mut any = false;
        for o in &p.affected_block_offsets {
            let (x, y, z) = (
                bx.wrapping_add(i32::from(o.x)),
                by.wrapping_add(i32::from(o.y)),
                bz.wrapping_add(i32::from(o.z)),
            );
            if self
                .columns
                .raw(x, y, z)
                .is_some_and(|s| !state(s).is_air())
            {
                any |= set_block(&mut self.columns, x, y, z, 0);
            }
        }
        changed(any)
    }
}

/// Java `MathHelper.floor(float)`.
fn floor(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 { i - 1 } else { i }
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

/// `Block.STATE_IDS.get` with null as air.
#[inline]
fn raw_from_wire(v: i64) -> u16 {
    u16::try_from(v)
        .ok()
        .filter(|&r| usize::from(r) < STATE_COUNT)
        .unwrap_or(0)
}

fn set_block(columns: &mut ColumnMap, x: i32, y: i32, z: i32, raw: u16) -> bool {
    if !(0..MAX_Y).contains(&y) {
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

/// Decodes the masked sections of `data` into `out` (sections outside the mask stay `None`).
/// Mask bits above section 15 are ignored, as in vanilla; trailing bytes are ignored.
fn decode_sections(
    builder: &mut SectionBuilder,
    x: i32,
    z: i32,
    mask: i32,
    data: &[u8],
    out: &mut [Option<crate::Section>; SECTIONS],
) -> Result<(), WorldError> {
    let mut c = Cursor { buf: data };
    for (j, slot) in out.iter_mut().enumerate() {
        if mask & (1 << j) == 0 {
            continue;
        }
        let err = |reason| WorldError::Section {
            pos: ChunkPos::new(x, z),
            section: j as u8,
            reason,
        };
        *slot = decode_section(builder, &mut c).map_err(err)?;
    }
    Ok(())
}

fn decode_section(
    builder: &mut SectionBuilder,
    c: &mut Cursor<'_>,
) -> Result<Option<crate::Section>, &'static str> {
    const EOF: &str = "truncated";
    c.take(2).ok_or(EOF)?; // non-air count, recomputed
    let sent = c.u8().ok_or(EOF)? as i8;
    let bits: u32 = match sent {
        i8::MIN..=4 => 4,
        5..=8 => sent as u32,
        _ => GLOBAL_BITS,
    };
    let mut remap = [0u16; 256];
    let mut remap_len = 0;
    if bits <= 8 {
        let len = c.varint().ok_or(EOF)?;
        let len = usize::try_from(len).map_err(|_| "negative palette length")?;
        if bits == 4 && len > 16 {
            return Err("array palette longer than 16");
        }
        if len > c.buf.len() {
            return Err(EOF);
        }
        for i in 0..len {
            let id = c.varint().ok_or(EOF)?;
            if let Some(r) = remap.get_mut(i) {
                *r = raw_from_wire(i64::from(id));
            }
        }
        remap_len = len.min(256);
    }
    let words = c.varint().ok_or(EOF)?;
    let need = words_needed(bits, SECTION_VOLUME);
    if usize::try_from(words).ok() != Some(need) {
        return Err("data array length does not match bits per block");
    }
    let data = c.take(need * 8).ok_or(EOF)?;
    if bits <= 8 {
        Ok(builder.build_packed(&remap[..remap_len], bits, data))
    } else {
        let mut states = [0u16; SECTION_VOLUME];
        unpack_be(bits, data, SECTION_VOLUME, |i, v| {
            states[i] = raw_from_wire(i64::from(v));
        });
        Ok(builder.build(&states))
    }
}

#[cfg(feature = "physics")]
impl ghast_physics::BlockView for World {
    type State = BlockState;

    /// Vanilla semantics: unloaded columns (an empty chunk) and positions outside the build
    /// height read as `void_air`.
    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> BlockState {
        self.block_state(x, y, z).unwrap_or(void_air())
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
