//! 26.1 (protocol 775) entity tracking.
//!
//! The 26.1 client no longer keeps a separate "server position": an entity *is* at
//! `Entity.position()`, and what a position packet does depends on the type
//! (`ghast_data::v26_1::EntityTypeData::interpolation`, from the jar):
//!
//! - **Interpolated** types (living entities, boats, minecarts, experience orbs, fishing bobbers)
//!   get a target; [`Tracker::tick`] moves them a third of the remaining way per tick
//!   (`InterpolationHandler`: `Mth.lerp` / `Mth.rotLerp` in doubles, the target follows position
//!   changes made between steps unless the moved box would collide, see [`Tracker::tick_with`]).
//!   Head rotations of living entities are lerped over 3 ticks the same way.
//! - **Other** types (items, projectiles, end crystals, ...) move at once.
//!
//! Packet coordinates: spawns and `entity_position_sync` set the `VecDeltaCodec` base (doubles);
//! a relative move decodes `round(base * 4096) + delta` per non-zero axis, divided by 4096
//! (`Math.round`, half up; an axis with delta 0 keeps the base exactly). `teleport_entity` does
//! not touch the base; its relative axes are relative to the interpolation target. Syncs farther
//! than 64 blocks, and syncs of entities in chunks the client has not loaded, snap. Spawn
//! rotations follow `recreateFromPacket` (living: `absSnapTo` with x / z clamped to ±3e7).
//!
//! **Position at pick time.** Vanilla's `Minecraft.tick` runs the crosshair pick *before*
//! `ClientLevel.tickEntities`, after the tick's packets: apply this tick's packets, pick with
//! [`Entity::position`] / [`Entity::bounding_box`], then call [`Tracker::tick`]. Proven against
//! the real client by `tests/golden/26.1/entity_tracking.jsonl.gz` (oracle `EntityTrackDump`).
//!
//! Not modelled: the client-side physics of non-interpolated entities between packets (items,
//! projectiles, experience orbs move on their own in vanilla), passenger positions (vanilla places
//! riders on their vehicle every tick), vehicles the bot controls other than boats (they ignore
//! position packets in vanilla), and the client-side decay of velocities: [`Entity::velocity`] is
//! the last velocity the server sent (a relative teleport adds to it without `ROTATE_DELTA`).
//!
//! Every packet is parsed by hand from the body; movement packets touch no allocator, and with
//! [`Config::metadata`] off metadata is walked without allocating (the size keys, sleeping
//! position and item counts are always applied).

use std::ops::Deref;

use bytes::Bytes;
use ghast_data::v26_1::entities::{self as data, EntityTypeData, type_flags};
use ghast_protocol::nbt::AnonNbt;
use ghast_protocol::types::LpVec3;
use ghast_protocol::types::slot::v775::ItemStack;
use ghast_protocol::v26_1::play::clientbound::{Login, Respawn, id};
use ghast_protocol::{Decode, DecodeError, RawPacket, Reader};

use crate::Applied;
use crate::common::{IdMask, R, angle, for_each_varint, own_str, skip_string};
use crate::common::{remove_effect, set_effect};
use crate::entity::{Effect, Entity, EntityClass, EntityType, Item, MetaValue};
use crate::store::{Config, Entities, Event, Fast, PlayerInfo};

/// Spawn, movement, destroy, riding, pickup, status, metadata, attribute packets, chunk loads
/// (only their coordinates are read: an entity in an unloaded chunk does not tick), plus `login`
/// / `respawn`.
pub const CORE_IDS: &[i32] = &[
    id::LOGIN,
    id::RESPAWN,
    id::SPAWN_ENTITY,
    id::ENTITY_VELOCITY,
    id::ENTITY_DESTROY,
    id::REL_ENTITY_MOVE,
    id::ENTITY_LOOK,
    id::ENTITY_MOVE_LOOK,
    id::SYNC_ENTITY_POSITION,
    id::ENTITY_TELEPORT,
    id::ENTITY_HEAD_ROTATION,
    id::ENTITY_STATUS,
    id::ATTACH_ENTITY,
    id::SET_PASSENGERS,
    id::COLLECT,
    id::ENTITY_METADATA,
    id::ENTITY_UPDATE_ATTRIBUTES,
    id::MAP_CHUNK,
    id::UNLOAD_CHUNK,
];
/// `entity_metadata` (always in [`CORE_IDS`]: poses, baby flags, sizes, sleeping positions and
/// item counts change boxes and positions).
pub const METADATA_IDS: &[i32] = &[id::ENTITY_METADATA];
/// `entity_equipment`.
pub const EQUIPMENT_IDS: &[i32] = &[id::ENTITY_EQUIPMENT];
/// `entity_effect`, `remove_entity_effect`.
pub const EFFECT_IDS: &[i32] = &[id::ENTITY_EFFECT, id::REMOVE_ENTITY_EFFECT];
/// `player_info` (update), `player_remove`. Player spawns need the tab list: vanilla skips a
/// player spawn whose profile is not in it (with [`Config::player_list`] off every spawn is kept).
pub const PLAYER_LIST_IDS: &[i32] = &[id::PLAYER_INFO, id::PLAYER_REMOVE];

/// Index of the fox flags byte (`Fox.DATA_FLAGS_ID`; 0x20 = sleeping).
const FOX_FLAGS: u8 = 19;

/// Registry id of `minecraft:scale` (checked against the jar by the oracle test).
pub const SCALE_ATTRIBUTE: i32 = 25;

/// 26.1 metadata indices shared by every entity / living entity.
pub mod meta {
    /// Entity flags byte (0x01 on fire, 0x02 crouching, 0x08 sprinting, 0x10 swimming,
    /// 0x20 invisible, 0x40 glowing, 0x80 fall flying).
    pub const FLAGS: u8 = 0;
    /// Air (VarInt).
    pub const AIR: u8 = 1;
    /// Custom name (optional component).
    pub const CUSTOM_NAME: u8 = 2;
    /// No gravity.
    pub const NO_GRAVITY: u8 = 5;
    /// Pose.
    pub const POSE: u8 = 6;
    /// Living: hand states.
    pub const LIVING_FLAGS: u8 = 8;
    /// Living: health (float).
    pub const HEALTH: u8 = 9;
    /// Living: sleeping position (optional block position; setting it moves the entity onto
    /// the bed: `x + 0.5, y + 0.6875, z + 0.5`).
    pub const SLEEPING_POS: u8 = 14;
    /// Item entity: the stack.
    pub const ITEM: u8 = 8;
}

/// Bits of `State::flags`.
mod st {
    /// `previousTickPosition` / `previousTickRot` are set.
    pub const HAS_PREV: u8 = 1;
    /// The scale changed: vanilla refreshes the dimensions at the end of the entity's next tick
    /// (`refreshDirtyAttributes`).
    pub const SCALE_DIRTY: u8 = 2;
    /// Nothing changes in a tick until a packet or a movement disturbs the entity.
    pub const SETTLED: u8 = 4;
    /// Living: a sleeping position is set (`isSleeping`: the tick zeroes xRot).
    pub const SLEEPING: u8 = 8;
    /// The baby flag.
    pub const BABY: u8 = 16;
    /// `dims` holds the cached dimensions.
    pub const HAS_DIMS: u8 = 32;
}

/// The 26.1 client state of an entity (`InterpolationHandler`, `VecDeltaCodec`, size keys),
/// boxed so entities of the other versions do not carry it.
#[derive(Clone, Debug)]
pub(crate) struct State {
    pub(crate) td: Option<&'static EntityTypeData>,
    /// Interpolation target and its rotation `(yRot, xRot)`.
    target: [f64; 3],
    /// `previousTickPosition` (valid with [`st::HAS_PREV`]).
    prev: [f64; 3],
    target_rot: [f32; 2],
    /// `previousTickRot` (`(xRot, yRot)`, vanilla's `Vec2` order).
    prev_rot: [f32; 2],
    /// Vanilla's cached `dimensions` (valid with [`st::HAS_DIMS`]; refreshed on size metadata, at
    /// spawn, and after a scale change at the end of the next tick).
    dims: ghast_data::EntityDimensions,
    head_target: f32,
    /// `yBodyRot`, and the mob `BodyRotationControl` state.
    body_yaw: f32,
    last_stable_head: f32,
    /// `getScale()` (sanitized `minecraft:scale` attribute).
    scale: f32,
    slime_size: i32,
    /// Item entities: the stack size (0 = empty / unknown).
    item_count: i32,
    /// Players: the tab list object the entity cached (`AbstractClientPlayer.getPlayerInfo`,
    /// 0 = none yet) and its game mode once it left the tab list.
    info_gen: u32,
    frozen_game_mode: i8,
    /// Remaining interpolation steps (0 = none active).
    steps: u8,
    head_steps: u8,
    /// `headStableTime`, saturated at 30 (vanilla only compares it with 10 and 20).
    head_stable_time: u8,
    pose: u8,
    stand_flags: u8,
    /// Foxes: their own flags byte (`Fox.isSleeping` = bit 0x20 instead of the bed position).
    fox_flags: u8,
    /// [`st`] bits.
    flags: u8,
}

impl State {
    const EMPTY: State = State {
        td: None,
        target: [0.0; 3],
        prev: [0.0; 3],
        target_rot: [0.0; 2],
        prev_rot: [0.0; 2],
        dims: ghast_data::EntityDimensions {
            width: 0.0,
            height: 0.0,
            eye_height: 0.0,
        },
        head_target: 0.0,
        body_yaw: 0.0,
        last_stable_head: 0.0,
        scale: 1.0,
        slime_size: 1,
        item_count: 0,
        info_gen: 0,
        frozen_game_mode: 0,
        steps: 0,
        head_steps: 0,
        head_stable_time: 0,
        pose: 0,
        stand_flags: 0,
        fox_flags: 0,
        flags: 0,
    };

    #[inline]
    fn flag(&self, f: u8) -> bool {
        self.flags & f != 0
    }

    #[inline]
    fn set_flag(&mut self, f: u8, on: bool) {
        if on {
            self.flags |= f;
        } else {
            self.flags &= !f;
        }
    }

    #[inline]
    fn dims(&self) -> Option<ghast_data::EntityDimensions> {
        self.flag(st::HAS_DIMS).then_some(self.dims)
    }

    #[inline]
    fn set_dims(&mut self, d: Option<ghast_data::EntityDimensions>) {
        self.set_flag(st::HAS_DIMS, d.is_some());
        if let Some(d) = d {
            self.dims = d;
        }
    }
}

/// The boxed 26.1 state of an entity: empty (one pointer) for the other versions; reads of an
/// empty one see the defaults, the first write allocates.
#[derive(Clone, Debug, Default)]
pub(crate) struct Ext(Option<Box<State>>);

static EMPTY_STATE: State = State::EMPTY;

impl Deref for Ext {
    type Target = State;
    #[inline]
    fn deref(&self) -> &State {
        self.0.as_deref().unwrap_or(&EMPTY_STATE)
    }
}

impl std::ops::DerefMut for Ext {
    #[inline]
    fn deref_mut(&mut self) -> &mut State {
        self.0.get_or_insert_with(|| Box::new(State::EMPTY))
    }
}

/// The bot's entity view on a 26.1 connection. Dereferences to the shared [`Entities`].
#[derive(Clone, Debug)]
pub struct Tracker {
    store: Entities,
    dimension: String,
    wanted: IdMask,
    /// Loaded chunk columns (`x & 0xffffffff | z << 32`).
    chunks: Fast<u64, ()>,
    /// Ids held in vanilla's tick list by a spawn the entity lookup refused (duplicate UUID): the
    /// tick list is keyed by id, so `isTickingEntity` says yes for any entity with that id until
    /// a real one with it starts ticking.
    ghosts: Fast<i32, ()>,
}

impl Deref for Tracker {
    type Target = Entities;
    fn deref(&self) -> &Entities {
        &self.store
    }
}

#[inline]
fn chunk_key(x: i32, z: i32) -> u64 {
    u64::from(x as u32) | (u64::from(z as u32) << 32)
}

/// `Mth.floor(double)`: `(int) v`, minus one when that is above `v` (no libm `floor` call).
#[inline]
fn mth_floor(v: f64) -> i32 {
    let i = v as i32;
    if v < f64::from(i) { i - 1 } else { i }
}

/// `Mth.floor` of a coordinate, then the chunk.
#[inline]
fn chunk_of(p: [f64; 3]) -> u64 {
    chunk_key(mth_floor(p[0]) >> 4, mth_floor(p[2]) >> 4)
}

/// Java `Math.round(double)` (half up, saturating, NaN = 0), without a libm `floor` call.
#[inline]
fn jround(x: f64) -> i64 {
    if x.is_nan() || x.abs() >= 4.5e15 {
        // Integral already (or NaN / out of range): the saturating cast is Java's result.
        return x as i64;
    }
    let t = x as i64;
    let f = if x < t as f64 { t - 1 } else { t };
    if x - f as f64 >= 0.5 { f + 1 } else { f }
}

/// The `VecDeltaCodec` base (the last absolute packet position), kept inline in the entity's
/// version-specific `traced` slot (as `f64` bits) so a move needs no second cache line for it.
#[inline]
fn base(e: &Entity) -> [f64; 3] {
    e.traced.map(|b| f64::from_bits(b as u64))
}

#[inline]
fn set_base(e: &mut Entity, p: [f64; 3]) {
    e.traced = p.map(|c| c.to_bits() as i64);
}

/// `VecDeltaCodec.decode`.
#[inline]
fn decode_delta(base: [f64; 3], d: [i16; 3]) -> [f64; 3] {
    if d == [0; 3] {
        return base;
    }
    let ax = |b: f64, d: i16| {
        if d == 0 {
            b
        } else {
            jround(b * 4096.0).wrapping_add(i64::from(d)) as f64 / 4096.0
        }
    };
    [ax(base[0], d[0]), ax(base[1], d[1]), ax(base[2], d[2])]
}

/// `Float.equals` / `Double.compare == 0` (bits, NaN equal to NaN).
#[inline]
fn same_f32(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

#[inline]
fn same_f64(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

/// `Mth.wrapDegrees(double)`.
#[inline]
fn wrap_degrees(a: f64) -> f64 {
    let mut r = a % 360.0;
    if r >= 180.0 {
        r -= 360.0;
    }
    if r < -180.0 {
        r += 360.0;
    }
    r
}

/// `Entity.setYRot` (non-finite values are discarded).
#[inline]
fn set_yrot(e: &mut Entity, y: f32) {
    if y.is_finite() {
        e.yaw = y;
    }
}

/// `Entity.setXRot`: `clamp(x % 360, -90, 90)`.
#[inline]
fn set_xrot(e: &mut Entity, x: f32) {
    if x.is_finite() {
        e.pitch = (x % 360.0).clamp(-90.0, 90.0);
    }
}

#[inline]
fn living(e: &Entity) -> bool {
    e.m.td.is_some_and(EntityTypeData::is_living)
}

/// `snapTo(pos, yRot, xRot)`.
fn snap_to(e: &mut Entity, p: [f64; 3], y: f32, x: f32) {
    e.pos = p;
    set_yrot(e, y);
    set_xrot(e, x);
}

/// `Entity.moveOrInterpolateTo(position?, yRot?, xRot?)`.
#[inline]
fn move_or_interpolate(e: &mut Entity, p: Option<[f64; 3]>, y: Option<f32>, x: Option<f32>) {
    // One borrow of the boxed state for the whole update.
    let m = &mut *e.m;
    match m.td.and_then(|t| t.interpolation) {
        Some(n) => {
            let active = m.steps > 0;
            let p = p.unwrap_or(if active { m.target } else { e.pos });
            let y = y.unwrap_or(if active { m.target_rot[0] } else { e.yaw });
            let x = x.unwrap_or(if active { m.target_rot[1] } else { e.pitch });
            if n == 0 {
                m.steps = 0;
                m.set_flag(st::HAS_PREV, false);
                snap_to(e, p, y, x);
            } else if !active
                || !same_f32(m.target_rot[0], y)
                || !same_f32(m.target_rot[1], x)
                || !(0..3).all(|i| same_f64(m.target[i], p[i]))
            {
                m.steps = n;
                m.target = p;
                m.target_rot = [y, x];
                m.prev = e.pos;
                m.prev_rot = [e.pitch, e.yaw];
                m.set_flag(st::HAS_PREV, true);
            }
        }
        None => {
            if let Some(p) = p {
                e.pos = p;
            }
            if let Some(y) = y {
                set_yrot(e, y % 360.0);
            }
            if let Some(x) = x {
                set_xrot(e, x % 360.0);
            }
        }
    }
}

/// `true` when the bot (`me`) steers `e` (a boat whose first passenger is the bot).
#[inline]
fn controlled(me: Option<i32>, e: &Entity) -> bool {
    me.is_some()
        && e.passengers.first().copied() == me
        && e.m
            .td
            .is_some_and(|t| t.name.ends_with("_boat") || t.name.ends_with("_raft"))
}

impl Tracker {
    /// An empty tracker.
    #[must_use]
    pub fn new(config: Config) -> Self {
        let mut wanted = IdMask::default();
        wanted.add(CORE_IDS);
        if config.equipment {
            wanted.add(EQUIPMENT_IDS);
        }
        if config.effects {
            wanted.add(EFFECT_IDS);
        }
        if config.player_list {
            wanted.add(PLAYER_LIST_IDS);
        }
        Tracker {
            store: Entities {
                era: crate::entity::Era::V26,
                ..Entities::new(config)
            },
            dimension: String::new(),
            wanted,
            chunks: Fast::default(),
            ghosts: Fast::default(),
        }
    }

    /// The shared store, mutable for [`Entities::drain_events`].
    pub fn store_mut(&mut self) -> &mut Entities {
        &mut self.store
    }

    /// `true` when this tracker wants packet `id` (for a `PacketFilter`).
    #[inline]
    #[must_use]
    pub fn wants(&self, id: i32) -> bool {
        self.wanted.contains(id)
    }

    /// Every packet id this tracker wants.
    pub fn packet_ids(&self) -> impl Iterator<Item = i32> + '_ {
        self.wanted.iter()
    }

    /// `true` when the chunk column holding `p` is loaded (its entities tick).
    #[must_use]
    pub fn chunk_loaded(&self, p: [f64; 3]) -> bool {
        self.chunks.contains_key(&chunk_of(p))
    }

    /// A chunk column became loaded (what a `map_chunk` packet does here; for a tracker started
    /// while chunks are already loaded): its entities tick again.
    pub fn mark_column_loaded(&mut self, x: i32, z: i32) {
        let k = chunk_key(x, z);
        self.chunks.insert(k, ());
        if !self.ghosts.is_empty() {
            // startTicking: the column's entities take their ids' tick list slots.
            for e in self.store.map.values() {
                if chunk_of(e.pos) == k {
                    self.ghosts.remove(&e.id);
                }
            }
        }
    }

    /// A chunk column was unloaded (`unload_chunk`): its entities stop ticking.
    pub fn mark_column_unloaded(&mut self, x: i32, z: i32) {
        self.chunks.remove(&chunk_key(x, z));
    }

    /// `ClientLevel.isTickingEntity`: players always tick, other entities while their chunk
    /// column is loaded.
    #[must_use]
    pub fn is_ticking(&self, e: &Entity) -> bool {
        e.is_player()
            || self.chunks.contains_key(&chunk_of(e.pos))
            || self.ghosts.contains_key(&e.id)
    }

    /// Whether vanilla's crosshair pick can hit `e`: its type is pickable (`isPickable`), it is
    /// not an armor stand marker, and not a spectator (players: tab list game mode 3).
    #[must_use]
    pub fn pickable(&self, e: &Entity) -> bool {
        let Some(t) = e.m.td else { return false };
        if !t.is_pickable() {
            return false;
        }
        if t.flags_meta.is_some() && e.m.stand_flags & 0x10 != 0 {
            return false;
        }
        !(e.is_player() && game_mode(&self.store.players, e) == Some(3))
    }

    /// One client tick of the tracked entities (`ClientLevel.tickEntities`): every entity in a
    /// loaded chunk takes one interpolation step and one head-rotation step. Call it once per
    /// client tick, **after** the tick's crosshair pick (see the module docs). Assumes the
    /// moved interpolation target never collides; use [`Tracker::tick_with`] for vanilla's check.
    pub fn tick(&mut self) {
        self.tick_with(|_, _| true);
    }

    /// [`Tracker::tick`] with vanilla's collision test: when an entity moved since its last
    /// interpolation step (a snap, a teleport, a bed position), the step first moves the target
    /// along, but only if `no_collision(entity, box)` says the entity's box at the moved target
    /// (`[min_x, min_y, min_z, max_x, max_y, max_z]`) is free (`Level.noCollision`: blocks,
    /// collidable entities, world border).
    pub fn tick_with(&mut self, mut no_collision: impl FnMut(&Entity, [f64; 6]) -> bool) {
        let me = self.store.self_id;
        let chunks = &self.chunks;
        // Vehicles whose first passenger is a mob (their body control skips the idle turn).
        let mut carrying: Vec<i32> = Vec::new();
        for e in self.store.map.values() {
            if let Some(v) = e.vehicle
                && e.m.td.is_some_and(|t| t.flags & type_flags::MOB != 0)
                && self
                    .store
                    .map
                    .get(&v)
                    .is_some_and(|v| v.passengers.first() == Some(&e.id))
            {
                carrying.push(v);
            }
        }
        let mut discard: Vec<i32> = Vec::new();
        let players = &self.store.players;
        for e in self.store.map.values_mut() {
            if e.m.flag(st::SETTLED) || Some(e.id) == me {
                continue;
            }
            if !e.is_player() && !chunks.contains_key(&chunk_of(e.pos)) {
                continue;
            }
            if !self.ghosts.is_empty() {
                self.ghosts.remove(&e.id);
            }
            if e.ty == EntityType::Registry(data::ITEM) && e.m.item_count <= 0 {
                // ItemEntity.tick: an empty stack discards the entity.
                discard.push(e.id);
                continue;
            }
            if e.is_player() {
                if e.m.info_gen == 0
                    && let Some(p) = players.get(&e.uuid)
                {
                    e.m.info_gen = p.generation;
                }
                // Player.tick: spectators and passengers are never on the ground.
                if e.vehicle.is_some() || game_mode(players, e) == Some(3) {
                    e.on_ground = false;
                }
            }
            let c = !carrying.is_empty() && carrying.contains(&e.id);
            tick_entity(e, c, &mut no_collision);
        }
        for id in discard {
            self.store.remove(id);
        }
    }

    /// Applies one clientbound play packet.
    ///
    /// # Errors
    /// A malformed body (multi-entry packets may be partly applied).
    pub fn apply(&mut self, raw: &RawPacket) -> Result<Applied, DecodeError> {
        self.apply_body(raw.id, &raw.body)
    }

    /// [`Tracker::apply`] with the packet id and body.
    ///
    /// # Errors
    /// See [`Tracker::apply`].
    pub fn apply_body(&mut self, packet_id: i32, body: &Bytes) -> Result<Applied, DecodeError> {
        if !self.wants(packet_id) {
            return Ok(Applied::NotEntityPacket);
        }
        let r = &mut Reader::new(body);
        let ticking = (&self.chunks, &self.ghosts);
        let s = &mut self.store;
        let changed = match packet_id {
            id::REL_ENTITY_MOVE => relative(s, r, true, false)?,
            id::ENTITY_MOVE_LOOK => relative(s, r, true, true)?,
            id::ENTITY_LOOK => relative(s, r, false, true)?,
            id::SYNC_ENTITY_POSITION => sync(s, r, ticking)?,
            id::ENTITY_TELEPORT => teleport(s, r)?,
            id::ENTITY_VELOCITY => {
                let eid = r.read_varint()?;
                let v = LpVec3::read(r)?.to_vec3();
                match s.map.get_mut(&eid) {
                    Some(e) => {
                        e.velocity = v;
                        s.event(Event::Velocity(eid));
                        true
                    }
                    None => false,
                }
            }
            id::ENTITY_HEAD_ROTATION => {
                let eid = r.read_varint()?;
                let b = r.read_i8()?;
                match s.map.get_mut(&eid) {
                    // `lerpHeadTo(yRot, 3)`; a no-op for non-living entities.
                    Some(e) if living(e) => {
                        e.m.head_target = angle(b);
                        e.m.head_steps = 3;
                        e.m.set_flag(st::SETTLED, false);
                        true
                    }
                    _ => false,
                }
            }
            id::ENTITY_DESTROY => {
                let mut any = false;
                for_each_varint(r, |eid| any |= s.remove(eid))?;
                any
            }
            id::ENTITY_STATUS => {
                let eid = r.read_i32()?;
                let status = r.read_i8()?;
                let known = s.map.contains_key(&eid);
                if known {
                    s.event(Event::Status { id: eid, status });
                }
                known
            }
            id::SPAWN_ENTITY => {
                let (applied, slot) = spawn(s, r, &self.chunks)?;
                match slot {
                    TickSlot::Ghost(id) => {
                        self.ghosts.insert(id, ());
                    }
                    TickSlot::Taken(id) if !self.ghosts.is_empty() => {
                        self.ghosts.remove(&id);
                    }
                    _ => {}
                }
                return Ok(applied);
            }
            id::ATTACH_ENTITY => {
                let eid = r.read_i32()?;
                let holder = r.read_i32()?;
                s.leash(eid, (holder != 0).then_some(holder))
            }
            id::SET_PASSENGERS => {
                let vehicle = r.read_varint()?;
                let start = r.position();
                for_each_varint(r, |_| {})?;
                let mut again = Reader::new(body);
                again.skip(start)?;
                if !s.map.contains_key(&vehicle) {
                    return Ok(Applied::Ignored);
                }
                let n = again.read_varint()?.max(0);
                let ids = (0..n).map_while(|_| again.read_varint().ok());
                let r = s.set_passengers(vehicle, ids);
                // Riding changes the body turn of mob vehicles: let every entity tick again.
                for e in s.map.values_mut() {
                    e.m.set_flag(st::SETTLED, false);
                }
                r
            }
            id::COLLECT => {
                let collected = r.read_varint()?;
                let collector = r.read_varint()?;
                let amount = r.read_varint()?;
                match s.map.get_mut(&collected) {
                    None => false,
                    Some(e) => {
                        let t = match e.ty {
                            EntityType::Registry(t) => t,
                            _ => u16::MAX,
                        };
                        let gone = match t {
                            data::ITEM => {
                                if e.m.item_count > 0 {
                                    e.m.item_count = e.m.item_count.wrapping_sub(amount);
                                    shrink_stack(e, amount);
                                }
                                e.m.item_count <= 0
                            }
                            data::EXPERIENCE_ORB => false,
                            _ => true,
                        };
                        s.event(Event::Collected {
                            collected,
                            collector,
                        });
                        if gone {
                            s.remove(collected);
                        }
                        true
                    }
                }
            }
            id::ENTITY_METADATA => metadata(s, r)?,
            id::ENTITY_UPDATE_ATTRIBUTES => attributes(s, r)?,
            id::ENTITY_EQUIPMENT => {
                let eid = r.read_varint()?;
                let mut list: [Option<(usize, Option<Item>)>; 8] = Default::default();
                let mut n = 0;
                loop {
                    let b = r.read_u8()?;
                    let item = ItemStack::read(r)?.map(|i| Item {
                        id: i.id,
                        count: i.count.clamp(0, 127) as i8,
                        damage: 0,
                        nbt: None,
                    });
                    if n < list.len() {
                        list[n] = Some((usize::from(b & 0x7F), item));
                        n += 1;
                    }
                    if b & 0x80 == 0 {
                        break;
                    }
                }
                match s.map.get_mut(&eid) {
                    Some(e) => {
                        let eq = e.equipment.get_or_insert_with(Default::default);
                        for (slot, item) in list.into_iter().flatten() {
                            if let Some(dst) = eq.get_mut(slot) {
                                *dst = item;
                            }
                        }
                        s.event(Event::Equipment(eid));
                        true
                    }
                    None => false,
                }
            }
            id::ENTITY_EFFECT => {
                let eid = r.read_varint()?;
                let effect = Effect {
                    id: r.read_varint()? as u8,
                    amplifier: r.read_varint()?.clamp(-128, 127) as i8,
                    duration: r.read_varint()?,
                    flags: r.read_u8()?,
                };
                set_effect(s, eid, effect)
            }
            id::REMOVE_ENTITY_EFFECT => {
                let eid = r.read_varint()?;
                let eff = r.read_varint()? as u8;
                remove_effect(s, eid, eff)
            }
            id::PLAYER_INFO => {
                player_info(s, r)?;
                // Game modes feed Player.tick (spectators leave the ground).
                for e in s.map.values_mut() {
                    if e.is_player() {
                        e.m.set_flag(st::SETTLED, false);
                    }
                }
                true
            }
            id::PLAYER_REMOVE => {
                let n = r.read_varint()?;
                let n = r.check_count(i64::from(n), 16)?;
                for _ in 0..n {
                    let uuid = r.read_uuid()?;
                    if let Some(p) = s.players.remove(&uuid) {
                        // Player entities holding this tab list object keep it, frozen.
                        for e in s.map.values_mut() {
                            if e.m.info_gen == p.generation {
                                e.m.frozen_game_mode = p.game_mode as i8;
                            }
                        }
                    }
                }
                s.event(Event::PlayerList);
                true
            }
            id::MAP_CHUNK => {
                let (x, z) = (r.read_i32()?, r.read_i32()?);
                self.mark_column_loaded(x, z);
                return Ok(Applied::Ignored);
            }
            id::UNLOAD_CHUNK => {
                let k = r.read_i64()? as u64;
                self.chunks.remove(&k);
                return Ok(Applied::Ignored);
            }
            id::LOGIN => {
                let p = Login::decode(r)?;
                s.clear();
                s.set_self(p.entity_id);
                self.chunks.clear();
                self.ghosts.clear();
                self.dimension = p.world_state.name.to_string();
                true
            }
            id::RESPAWN => {
                let p = Respawn::decode(r)?;
                // A new client player: effects and equipment reset; metadata is copied only with
                // the keep-entity-data flag (2).
                if let Some(me) = s.self_id.and_then(|i| s.map.get_mut(&i)) {
                    me.effects.clear();
                    me.equipment = None;
                    if p.copy_metadata & 2 == 0 {
                        me.meta.clear();
                    }
                }
                if *p.world_state.name != *self.dimension {
                    s.clear();
                    self.chunks.clear();
                    self.ghosts.clear();
                    self.dimension = p.world_state.name.to_string();
                }
                true
            }
            _ => return Ok(Applied::NotEntityPacket),
        };
        Ok(if changed {
            Applied::Changed
        } else {
            Applied::Ignored
        })
    }
}

/// `Mth.wrapDegrees(float)`.
#[inline]
fn wrap_f32(a: f32) -> f32 {
    let mut r = a % 360.0;
    if r >= 180.0 {
        r -= 360.0;
    }
    if r < -180.0 {
        r += 360.0;
    }
    r
}

/// `Mth.clamp(float)`: `v < min ? min : Math.min(v, max)`.
#[inline]
fn jclamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v.is_nan() {
        v
    } else if v == 0.0 && max == 0.0 {
        if v.is_sign_negative() || max.is_sign_negative() {
            -0.0
        } else {
            0.0
        }
    } else if v <= max {
        v
    } else {
        max
    }
}

/// `Mth.rotateIfNecessary`.
#[inline]
fn rotate_if_necessary(base: f32, target: f32, max: f32) -> f32 {
    target - jclamp(wrap_f32(target - base), -max, max)
}

/// One `ClientLevel.tickNonPassenger` of a 26.1 entity, as far as it changes tracked state:
/// dirty attributes (`baseTick`), the interpolation and head-rotation steps (`aiStep`), the mob
/// body / head turn (`tickHeadTurn`: `BodyRotationControl.clientTick`).
fn tick_entity(
    e: &mut Entity,
    carrying_mob: bool,
    no_collision: &mut impl FnMut(&Entity, [f64; 6]) -> bool,
) {
    let old = e.pos;
    if e.m.steps > 0 {
        step(e, no_collision);
    }
    if e.m.head_steps > 0 {
        let a = 1.0 / f64::from(e.m.head_steps);
        let from = f64::from(e.head_yaw);
        e.head_yaw = (from + a * wrap_degrees(f64::from(e.m.head_target) - from)) as f32;
        e.m.head_steps -= 1;
    }
    let Some(t) = e.m.td else {
        e.m.set_flag(st::SETTLED, true);
        return;
    };
    let mut settled =
        e.m.steps == 0 && e.m.head_steps == 0 && !(t.id == data::ITEM && e.m.item_count <= 0);
    if t.flags & (type_flags::MOB | type_flags::DEFAULT_BODY)
        == (type_flags::MOB | type_flags::DEFAULT_BODY)
    {
        let max = f32::from(t.max_head_y_rot);
        let (xd, zd) = (e.pos[0] - old[0], e.pos[2] - old[2]);
        let m = &mut e.m;
        if xd * xd + zd * zd > f64::from(2.500_000_3e-7_f32) {
            m.body_yaw = e.yaw;
            e.head_yaw = rotate_if_necessary(e.head_yaw, m.body_yaw, max);
            m.last_stable_head = e.head_yaw;
            m.head_stable_time = 0;
            settled = false;
        } else if carrying_mob {
            // Idle turning resumes once the mob passenger is gone: keep ticking.
            settled = false;
        } else {
            if (e.head_yaw - m.last_stable_head).abs() > 15.0 {
                m.head_stable_time = 0;
                m.last_stable_head = e.head_yaw;
                m.body_yaw = rotate_if_necessary(m.body_yaw, e.head_yaw, max);
                settled = false;
            } else {
                m.head_stable_time = m.head_stable_time.saturating_add(1).min(30);
                if m.head_stable_time > 10 {
                    let since = m.head_stable_time - 10;
                    let frac = (since as f32 / 10.0).clamp(0.0, 1.0);
                    let angle = max * (1.0 - frac);
                    m.body_yaw = rotate_if_necessary(m.body_yaw, e.head_yaw, angle);
                }
                settled &= m.head_stable_time >= 20 && m.body_yaw.to_bits() == e.head_yaw.to_bits();
            }
        }
    } else if t.flags_meta.is_some() {
        // ArmorStand.tickHeadTurn: the body follows yRot.
        e.m.body_yaw = e.yaw;
    }
    let sleeping = if t.name == "fox" {
        e.m.fox_flags & 0x20 != 0
    } else {
        e.m.flag(st::SLEEPING)
    };
    if sleeping {
        set_xrot(e, 0.0);
    }
    // refreshDirtyAttributes, at the end of LivingEntity.tick.
    if e.m.flag(st::SCALE_DIRTY) {
        e.m.set_flag(st::SCALE_DIRTY, false);
        {
            let d = compute_dims(e);
            e.m.set_dims(d);
        }
    }
    e.m.set_flag(st::SETTLED, settled);
}

/// The game mode a player entity sees (`AbstractClientPlayer.getPlayerInfo` caches the tab list
/// object on first use; it stays frozen once the server removes the entry).
fn game_mode(players: &Fast<uuid::Uuid, PlayerInfo>, e: &Entity) -> Option<i32> {
    let cur = players.get(&e.uuid);
    if e.m.info_gen == 0 {
        return cur.map(|p| p.game_mode);
    }
    match cur {
        Some(p) if p.generation == e.m.info_gen => Some(p.game_mode),
        _ => Some(i32::from(e.m.frozen_game_mode)),
    }
}

/// One `InterpolationHandler.interpolate()` step.
fn step(e: &mut Entity, no_collision: &mut impl FnMut(&Entity, [f64; 6]) -> bool) {
    let alpha = 1.0 / f64::from(e.m.steps);
    if e.m.flag(st::HAS_PREV) {
        let d = [
            e.pos[0] - e.m.prev[0],
            e.pos[1] - e.m.prev[1],
            e.pos[2] - e.m.prev[2],
        ];
        let t = e.m.target;
        let moved = [t[0] + d[0], t[1] + d[1], t[2] + d[2]];
        // Adding +0.0 changes nothing (but -0.0): skip the collision test then.
        let noop = (0..3).all(|i| moved[i].to_bits() == t[i].to_bits());
        if !noop {
            let bb = dimensions(e).map(|dm| {
                let b = dm.bounding_box(moved[0], moved[1], moved[2]);
                [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]
            });
            if bb.is_none_or(|bb| no_collision(&*e, bb)) {
                e.m.target = moved;
            }
        }
        // previousTickRot = (xRot, yRot).
        e.m.target_rot[0] += e.yaw - e.m.prev_rot[1];
        e.m.target_rot[1] += e.pitch - e.m.prev_rot[0];
    }
    let t = e.m.target;
    let lerp = |a: f64, b: f64| a + alpha * (b - a);
    let p = [
        lerp(e.pos[0], t[0]),
        lerp(e.pos[1], t[1]),
        lerp(e.pos[2], t[2]),
    ];
    let y0 = f64::from(e.yaw);
    let ny = (y0 + alpha * wrap_degrees(f64::from(e.m.target_rot[0]) - y0)) as f32;
    let nx = lerp(f64::from(e.pitch), f64::from(e.m.target_rot[1])) as f32;
    e.pos = p;
    // setRot: setYRot(y % 360), setXRot(x % 360).
    set_yrot(e, ny % 360.0);
    set_xrot(e, nx % 360.0);
    e.m.steps -= 1;
    e.m.prev = p;
    e.m.prev_rot = [e.pitch, e.yaw];
    e.m.set_flag(st::HAS_PREV, true);
}

/// What a spawn did to vanilla's id-keyed tick list.
enum TickSlot {
    None,
    /// A refused (duplicate UUID) spawn that still entered the tick list.
    Ghost(i32),
    /// A ticking spawn took its id's slot.
    Taken(i32),
}

fn spawn(s: &mut Entities, r: &mut Reader<'_>, chunks: &Fast<u64, ()>) -> R<(Applied, TickSlot)> {
    let eid = r.read_varint()?;
    let uuid = r.read_uuid()?;
    let t = r.read_varint()?;
    let pos = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
    let v = LpVec3::read(r)?.to_vec3();
    let pitch = angle(r.read_i8()?);
    let yaw = angle(r.read_i8()?);
    let head = angle(r.read_i8()?);
    let data = r.read_varint()?;
    let Some(td) = u16::try_from(t).ok().and_then(data::entity_type) else {
        return Ok((Applied::Ignored, TickSlot::None));
    };
    let ty = if td.id == data::PLAYER {
        if s.config.player_list && !s.players.contains_key(&uuid) {
            // "Server attempted to add player prior to sending player info".
            return Ok((Applied::Ignored, TickSlot::None));
        }
        EntityType::Player
    } else {
        EntityType::Registry(td.id)
    };
    let mut e = Entity::new(eid, ty);
    e.living = td.is_living();
    e.uuid = uuid;
    e.m.td = Some(td);
    set_base(&mut e, pos);
    e.data = data;
    e.velocity = v;
    if td.is_living() {
        // LivingEntity.recreateFromPacket: absSnapTo, head = body = the head byte.
        e.pos = [
            pos[0].clamp(-3.0e7, 3.0e7),
            pos[1],
            pos[2].clamp(-3.0e7, 3.0e7),
        ];
        set_yrot(&mut e, yaw % 360.0);
        set_xrot(&mut e, pitch.clamp(-90.0, 90.0) % 360.0);
        e.head_yaw = head;
        e.m.body_yaw = if td.name == "shulker" { 0.0 } else { head };
        if td.name == "warden" && data == 1 {
            e.m.pose = 13; // emerging
        }
    } else {
        snap_to(&mut e, pos, yaw, pitch);
        e.head_yaw = e.yaw;
    }
    let d = compute_dims(&e);
    e.m.set_dims(d);
    // ClientLevel.addEntity: the old entity with this id goes first; then the entity lookup
    // refuses a UUID that another entity already has ("Duplicate entity UUID").
    let removed = s.remove(eid);
    if s.map.values().any(|o| o.uuid == uuid && o.id != eid) {
        // The rest of TransientEntitySectionManager.addEntity still runs: a ticking entity
        // enters the tick list under its id.
        let ticks = e.is_player() || chunks.contains_key(&chunk_of(e.pos));
        let a = if removed {
            Applied::Changed
        } else {
            Applied::Ignored
        };
        return Ok((
            a,
            if ticks {
                TickSlot::Ghost(eid)
            } else {
                TickSlot::None
            },
        ));
    }
    let ticks = e.is_player() || chunks.contains_key(&chunk_of(e.pos));
    s.spawn(e);
    Ok((
        Applied::Changed,
        if ticks {
            TickSlot::Taken(eid)
        } else {
            TickSlot::None
        },
    ))
}

#[inline]
fn relative(s: &mut Entities, r: &mut Reader<'_>, pos: bool, look: bool) -> R<bool> {
    let eid = r.read_varint()?;
    let d = if pos {
        [r.read_i16()?, r.read_i16()?, r.read_i16()?]
    } else {
        [0; 3]
    };
    let rot = if look {
        Some((angle(r.read_i8()?), angle(r.read_i8()?)))
    } else {
        None
    };
    let on_ground = r.read_bool()?;
    let me = s.self_id;
    if Some(eid) == me {
        return Ok(false);
    }
    let Some(e) = s.map.get_mut(&eid) else {
        return Ok(false);
    };
    let ctl = controlled(me, e);
    let m = &mut *e.m;
    if ctl {
        set_base(e, decode_delta(base(e), d));
        return Ok(true);
    }
    m.set_flag(st::SETTLED, false);
    if pos {
        let p = decode_delta(base(e), d);
        set_base(e, p);
        move_or_interpolate(e, Some(p), rot.map(|r| r.0), rot.map(|r| r.1));
    } else if let Some((y, x)) = rot {
        move_or_interpolate(e, None, Some(y), Some(x));
    }
    e.on_ground = on_ground;
    s.event(Event::Moved(eid));
    Ok(true)
}

/// `PositionMoveRotation.STREAM_CODEC`: position, delta movement, yRot, xRot.
fn pmr(r: &mut Reader<'_>) -> R<([f64; 3], [f64; 3], f32, f32)> {
    Ok((
        [r.read_f64()?, r.read_f64()?, r.read_f64()?],
        [r.read_f64()?, r.read_f64()?, r.read_f64()?],
        r.read_f32()?,
        r.read_f32()?,
    ))
}

fn dist_sqr(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

fn sync(
    s: &mut Entities,
    r: &mut Reader<'_>,
    (chunks, ghosts): (&Fast<u64, ()>, &Fast<i32, ()>),
) -> R<bool> {
    let eid = r.read_varint()?;
    let (p, _, y, x) = pmr(r)?;
    let on_ground = r.read_bool()?;
    let me = s.self_id;
    if Some(eid) == me {
        return Ok(false);
    }
    let Some(e) = s.map.get_mut(&eid) else {
        return Ok(false);
    };
    let ctl = controlled(me, e);
    set_base(e, p);
    e.m.set_flag(st::SETTLED, false);
    if !ctl {
        let far = dist_sqr(e.pos, p) > 4096.0;
        let ticking =
            e.is_player() || chunks.contains_key(&chunk_of(e.pos)) || ghosts.contains_key(&eid);
        if ticking && !far {
            move_or_interpolate(e, Some(p), Some(y), Some(x));
        } else {
            snap_to(e, p, y, x);
        }
        e.on_ground = on_ground;
    }
    s.event(Event::Moved(eid));
    Ok(true)
}

fn teleport(s: &mut Entities, r: &mut Reader<'_>) -> R<bool> {
    let eid = r.read_varint()?;
    let (cp, cd, cy, cx) = pmr(r)?;
    let flags = r.read_i32()?;
    let on_ground = r.read_bool()?;
    if Some(eid) == s.self_id {
        return Ok(false);
    }
    let Some(e) = s.map.get_mut(&eid) else {
        return Ok(false);
    };
    let has = |bit: u32| flags & (1 << bit) != 0;
    e.m.set_flag(st::SETTLED, false);
    // PositionMoveRotation.of: the interpolation target while interpolating.
    let active = e.m.steps > 0;
    let (sp, sy, sx) = if active {
        (e.m.target, e.m.target_rot[0], e.m.target_rot[1])
    } else {
        (e.pos, e.yaw, e.pitch)
    };
    let p = [
        if has(0) { sp[0] } else { 0.0 } + cp[0],
        if has(1) { sp[1] } else { 0.0 } + cp[1],
        if has(2) { sp[2] } else { 0.0 } + cp[2],
    ];
    let y = if has(3) { sy } else { 0.0 } + cy;
    let x = (if has(4) { sx } else { 0.0 } + cx).clamp(-90.0, 90.0);
    let v = [
        if has(5) { e.velocity[0] + cd[0] } else { cd[0] },
        if has(6) { e.velocity[1] + cd[1] } else { cd[1] },
        if has(7) { e.velocity[2] + cd[2] } else { cd[2] },
    ];
    // Interpolated unless a non-ticking vehicle the bot controls gets an absolute teleport;
    // that case (and a jump over 64 blocks) sets the values directly.
    let far = dist_sqr(sp, p) > 4096.0;
    if far {
        e.pos = p;
        set_yrot(e, y);
        set_xrot(e, x);
    } else {
        move_or_interpolate(e, Some(p), Some(y), Some(x));
    }
    e.velocity = v;
    e.on_ground = on_ground;
    s.event(Event::Moved(eid));
    Ok(true)
}

/// The value of one metadata entry: stored (owned) when wanted, else skipped. Size keys are
/// returned through `keys`.
struct Keys {
    pose: Option<u8>,
    baby: Option<bool>,
    slime: Option<i32>,
    stand: Option<u8>,
    fox: Option<u8>,
    bed: Option<Option<[i32; 3]>>,
    count: Option<i32>,
    stack: Option<Option<ItemStack>>,
}

fn block_pos(v: i64) -> [i32; 3] {
    // BlockPos.of(long): x 26 bits, z 26 bits, y 12 bits.
    [
        (v >> 38) as i32,
        ((v << 52) >> 52) as i32,
        ((v << 26) >> 38) as i32,
    ]
}

/// Copies `[start, end)` of the packet out (stored values never pin a packet buffer).
fn owned(r: &Reader<'_>, start: usize) -> Bytes {
    Bytes::copy_from_slice(&r.bytes_since(start))
}

fn read_value(
    r: &mut Reader<'_>,
    index: u8,
    ty: u32,
    td: &EntityTypeData,
    keep: bool,
    keys: &mut Keys,
) -> R<Option<MetaValue>> {
    let live = td.is_living();
    let v = match ty {
        0 => {
            let b = r.read_i8()?;
            if td.flags_meta == Some(index) {
                keys.stand = Some(b as u8);
            }
            if index == FOX_FLAGS && td.name == "fox" {
                keys.fox = Some(b as u8);
            }
            MetaValue::Byte(b)
        }
        1 => {
            let n = r.read_varint()?;
            if td.size_meta == Some(index) {
                keys.slime = Some(n);
            }
            MetaValue::VarInt(n)
        }
        2 => MetaValue::Long(r.read_varlong()?),
        3 => MetaValue::Float(r.read_f32()?),
        4 => {
            if !keep {
                skip_string(r)?;
                return Ok(None);
            }
            MetaValue::String(own_str(r)?)
        }
        5 => {
            let start = r.position();
            AnonNbt::read(r)?;
            if !keep {
                return Ok(None);
            }
            MetaValue::Component(owned(r, start))
        }
        6 => {
            if r.read_bool()? {
                let start = r.position();
                AnonNbt::read(r)?;
                if !keep {
                    return Ok(None);
                }
                MetaValue::OptComponent(Some(owned(r, start)))
            } else {
                MetaValue::OptComponent(None)
            }
        }
        7 => {
            let start = r.position();
            let s = ItemStack::read(r)?;
            let is_item = td.id == data::ITEM && index == meta::ITEM;
            if is_item {
                keys.count = Some(s.as_ref().map_or(0, |s| s.count));
            }
            if !keep && !is_item {
                return Ok(None);
            }
            // Re-read from an owned copy: the stored stack must not pin the packet buffer.
            let copy = owned(r, start);
            let s = ItemStack::read(&mut Reader::new(&copy))?;
            if is_item {
                keys.stack = Some(s.clone());
            }
            if !keep {
                return Ok(None);
            }
            MetaValue::Stack(s)
        }
        8 => {
            let b = r.read_bool()?;
            if td.baby_meta == Some(index) {
                keys.baby = Some(b);
            }
            MetaValue::Bool(b)
        }
        9 => MetaValue::Rotation([r.read_f32()?, r.read_f32()?, r.read_f32()?]),
        10 => MetaValue::BlockPos(block_pos(r.read_i64()?)),
        11 => {
            let p = if r.read_bool()? {
                Some(block_pos(r.read_i64()?))
            } else {
                None
            };
            if live && index == meta::SLEEPING_POS {
                keys.bed = Some(p);
            }
            MetaValue::OptBlockPos(p)
        }
        12 => MetaValue::Direction(r.read_varint()?),
        13 => MetaValue::OptUuid(if r.read_bool()? {
            Some(r.read_uuid()?)
        } else {
            None
        }),
        14 => MetaValue::BlockState(r.read_varint()?),
        18 => MetaValue::VillagerData([r.read_varint()?, r.read_varint()?, r.read_varint()?]),
        19 => MetaValue::OptVarInt(r.read_varint()?),
        20 => {
            let p = r.read_varint()?;
            if index == meta::POSE {
                // Pose.BY_ID: out-of-range ids read as standing.
                keys.pose = Some(if (0..18).contains(&p) { p as u8 } else { 0 });
            }
            MetaValue::Pose(p)
        }
        21..=32 | 35..=38 | 42 => MetaValue::Enum(ty as u8, r.read_varint()?),
        39 => MetaValue::Vector3([r.read_f32()?, r.read_f32()?, r.read_f32()?]),
        40 => MetaValue::Quaternion([r.read_f32()?, r.read_f32()?, r.read_f32()?, r.read_f32()?]),
        15 | 16 | 17 | 33 | 34 | 41 => {
            let start = r.position();
            skip_complex(r, ty)?;
            if !keep {
                return Ok(None);
            }
            MetaValue::Raw(ty as u8, owned(r, start))
        }
        _ => return Err(DecodeError::Invalid("unknown metadata serializer")),
    };
    Ok(keep.then_some(v))
}

/// Skips the values with a complex layout using the generated decoders.
fn skip_complex(r: &mut Reader<'_>, ty: u32) -> R<()> {
    use ghast_protocol::v26_1::types::{
        EntityMetadataPaintingVariant, GlobalPos, Particle, ResolvableProfile,
    };
    match ty {
        15 => {
            r.read_varint()?;
        }
        16 => {
            Particle::decode(r)?;
        }
        17 => {
            let n = r.read_varint()?;
            let n = r.check_count(i64::from(n), 1)?;
            for _ in 0..n {
                Particle::decode(r)?;
            }
        }
        33 => {
            if r.read_bool()? {
                GlobalPos::decode(r)?;
            }
        }
        34 => {
            // Holder: registry id + 1, or 0 and the variant inline.
            if r.read_varint()? == 0 {
                EntityMetadataPaintingVariant::decode(r)?;
            }
        }
        41 => {
            ResolvableProfile::decode(r)?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn metadata(s: &mut Entities, r: &mut Reader<'_>) -> R<bool> {
    let eid = r.read_varint()?;
    let Some(e) = s.map.get(&eid) else {
        return Ok(false);
    };
    let Some(td) = e.m.td.or_else(|| data::entity_type(data::PLAYER)) else {
        return Ok(false);
    };
    let keep = s.config.metadata;
    let mut keys = Keys {
        pose: None,
        baby: None,
        slime: None,
        stand: None,
        fox: None,
        bed: None,
        count: None,
        stack: None,
    };
    let mut scratch = std::mem::take(&mut s.scratch);
    scratch.clear();
    let res = (|| -> R<()> {
        loop {
            let index = r.read_u8()?;
            if index == 0xFF {
                return Ok(());
            }
            let ty = r.read_varint()? as u32;
            if let Some(v) = read_value(r, index, ty, td, keep, &mut keys)? {
                scratch.push((index, v));
            }
        }
    })();
    if res.is_ok()
        && let Some(e) = s.map.get_mut(&eid)
    {
        for (i, v) in scratch.drain(..) {
            e.meta.set(i, v);
        }
        e.m.set_flag(st::SETTLED, false);
        let resize = keys.pose.is_some()
            || keys.baby.is_some()
            || keys.stand.is_some()
            || keys.slime.is_some();
        if let Some(p) = keys.pose {
            e.m.pose = p;
        }
        if let Some(b) = keys.baby {
            e.m.set_flag(st::BABY, b);
        }
        if let Some(f) = keys.stand {
            e.m.stand_flags = f;
        }
        if let Some(f) = keys.fox {
            e.m.fox_flags = f;
        }
        if let Some(n) = keys.slime {
            e.m.slime_size = n;
            // Slime.onSyncedDataUpdated: setYRot(yHeadRot), yBodyRot = yHeadRot.
            set_yrot(e, e.head_yaw);
            e.m.body_yaw = e.head_yaw;
        }
        if resize {
            {
                let d = compute_dims(e);
                e.m.set_dims(d);
            }
        }
        if let Some(b) = keys.bed {
            e.m.set_flag(st::SLEEPING, b.is_some());
            if let Some(b) = b {
                // LivingEntity.setPosToBed.
                e.pos = [
                    f64::from(b[0]) + 0.5,
                    f64::from(b[1]) + 0.6875,
                    f64::from(b[2]) + 0.5,
                ];
            }
        }
        if let Some(c) = keys.count {
            e.m.item_count = c;
            if !keep && let Some(st) = keys.stack.take() {
                e.meta.set(meta::ITEM, MetaValue::Stack(st));
            }
        }
        s.event(Event::Metadata(eid));
    }
    s.scratch = scratch;
    res?;
    Ok(true)
}

fn attributes(s: &mut Entities, r: &mut Reader<'_>) -> R<bool> {
    let eid = r.read_varint()?;
    let n = r.read_varint()?;
    let n = r.check_count(i64::from(n), 10)?;
    let mut scale = None;
    for _ in 0..n {
        let attr = r.read_varint()?;
        let base = r.read_f64()?;
        let m = r.read_varint()?;
        let m = r.check_count(i64::from(m), 10)?;
        let (mut add, mut mul_base, mut mul_total) = (Vec::new(), Vec::new(), Vec::new());
        let want = attr == SCALE_ATTRIBUTE;
        for _ in 0..m {
            skip_string(r)?;
            let amount = r.read_f64()?;
            let op = r.read_varint()?;
            if want {
                match op {
                    0 => add.push(amount),
                    1 => mul_base.push(amount),
                    _ => mul_total.push(amount),
                }
            }
        }
        if want {
            // AttributeInstance.calculateValue (modifiers in packet order), then the
            // RangedAttribute clamp [0.0625, 16].
            let mut b = base;
            for a in &add {
                b += a;
            }
            let mut v = b;
            for a in &mul_base {
                v += b * a;
            }
            for a in &mul_total {
                v *= 1.0 + a;
            }
            let v = if v.is_nan() {
                0.0625
            } else {
                v.clamp(0.0625, 16.0)
            };
            scale = Some(v as f32);
        }
    }
    let Some(e) = s.map.get_mut(&eid) else {
        return Ok(false);
    };
    if !living(e) {
        return Ok(false);
    }
    if let Some(v) = scale {
        e.m.scale = match e.m.td.map(|t| t.name) {
            Some("ender_dragon") => 1.0,
            Some("happy_ghast") => v.min(1.0),
            _ => v,
        };
        e.m.set_flag(st::SCALE_DIRTY, true);
        e.m.set_flag(st::SETTLED, false);
    }
    Ok(true)
}

fn shrink_stack(e: &mut Entity, amount: i32) {
    if let Some(MetaValue::Stack(Some(st))) = e
        .meta
        .entries_mut()
        .find(|(i, _)| *i == meta::ITEM)
        .map(|(_, v)| v)
    {
        st.count = st.count.wrapping_sub(amount);
    }
}

/// 26.1 `player_info` (update): an action bitset (enum order), then per entry the fields of every
/// set action. `ADD_PLAYER` only adds unknown profiles (`putIfAbsent`); the other actions apply
/// to the entry the client has.
fn player_info(s: &mut Entities, r: &mut Reader<'_>) -> R<()> {
    let actions = r.read_u8()?;
    let n = r.read_varint()?;
    let n = r.check_count(i64::from(n), 16)?;
    let has = |bit: u8| actions & (1 << bit) != 0;
    for _ in 0..n {
        let uuid = r.read_uuid()?;
        if has(0) {
            let name = own_str(r)?;
            let props = r.read_varint()?;
            let props = r.check_count(i64::from(props), 3)?;
            let mut textures = None;
            for _ in 0..props {
                if s.config.skins {
                    let key = r.read_string()?;
                    let value = r.read_string()?;
                    if &*key == "textures" {
                        textures = Some(Box::<str>::from(&*value));
                    }
                } else {
                    skip_string(r)?;
                    skip_string(r)?;
                }
                if r.read_bool()? {
                    skip_string(r)?;
                }
            }
            let next = s.generations.wrapping_add(1).max(1);
            let fresh = !s.players.contains_key(&uuid);
            if fresh {
                s.generations = next;
            }
            s.players.entry(uuid).or_insert(PlayerInfo {
                name,
                game_mode: 0,
                ping: 0,
                display_name: None,
                textures,
                display_name_nbt: None,
                listed: false,
                list_order: 0,
                generation: next,
            });
        }
        if has(1) && r.read_bool()? {
            // Chat session: id, key expiry, key bytes, key signature.
            r.read_uuid()?;
            r.read_i64()?;
            let k = r.read_len_varint(r.remaining())?;
            r.skip(k)?;
            let k = r.read_len_varint(r.remaining())?;
            r.skip(k)?;
        }
        // GameType.byId: out-of-range ids read as survival.
        let gm = if has(2) {
            Some(
                Some(r.read_varint()?)
                    .filter(|g| (0..4).contains(g))
                    .unwrap_or(0),
            )
        } else {
            None
        };
        let listed = if has(3) { Some(r.read_bool()?) } else { None };
        let ping = if has(4) { Some(r.read_varint()?) } else { None };
        let display = if has(5) {
            Some(if r.read_bool()? {
                let start = r.position();
                AnonNbt::read(r)?;
                Some(owned(r, start))
            } else {
                None
            })
        } else {
            None
        };
        let order = if has(6) { Some(r.read_varint()?) } else { None };
        if has(7) {
            r.read_bool()?;
        }
        if let Some(p) = s.players.get_mut(&uuid) {
            if let Some(g) = gm {
                p.game_mode = g;
            }
            if let Some(l) = listed {
                p.listed = l;
            }
            if let Some(l) = ping {
                p.ping = l;
            }
            if let Some(d) = display {
                p.display_name_nbt = d;
            }
            if let Some(o) = order {
                p.list_order = o;
            }
        }
    }
    s.event(Event::PlayerList);
    Ok(())
}

/// The packet coordinates of a 26.1 entity (`VecDeltaCodec` base: the last spawn, relative move
/// or position sync; teleports do not change it).
#[must_use]
pub fn server_position(e: &Entity) -> [f64; 3] {
    base(e)
}

/// The interpolation target and its `(yRot, xRot)` while an interpolation is active.
#[must_use]
pub fn interpolation_target(e: &Entity) -> Option<([f64; 3], [f32; 2])> {
    (e.m.steps > 0).then_some((e.m.target, e.m.target_rot))
}

/// The type facts from ghast-data.
#[must_use]
pub fn entity_type(e: &Entity) -> Option<&'static EntityTypeData> {
    e.m.td
}

/// Pick radius (`getPickRadius`, 0 for every 26.1 type).
#[must_use]
pub fn pick_radius(e: &Entity) -> f32 {
    e.m.td
        .and_then(|t| t.size)
        .map_or(0.0, |s| s.targeting_margin)
}

/// `yBodyRot` of mobs with the default body control and armor stands (exact); other entities
/// keep their spawn value (players turn their body from movement, not modelled).
#[must_use]
pub fn body_yaw(e: &Entity) -> f32 {
    e.m.body_yaw
}

/// The pose id (`Pose`: 0 standing, 2 sleeping, 3 swimming, 5 crouching, 10 sitting, ...).
#[must_use]
pub fn pose(e: &Entity) -> u8 {
    e.m.pose
}

/// The `minecraft:scale` value the client uses for the entity's size.
#[must_use]
pub fn scale(e: &Entity) -> f32 {
    e.m.scale
}

/// Stack size of an item entity (0 before its stack arrived).
#[must_use]
pub fn item_count(e: &Entity) -> i32 {
    e.m.item_count
}

/// The stack of an item entity (always kept for item entities).
#[must_use]
pub fn item_stack(e: &Entity) -> Option<&ItemStack> {
    match e.meta.get(meta::ITEM)? {
        MetaValue::Stack(s) if e.ty == EntityType::Registry(data::ITEM) => s.as_ref(),
        _ => None,
    }
}

/// 26.1 health of a living entity (with [`Config::metadata`]).
#[must_use]
pub fn health(e: &Entity) -> Option<f32> {
    e.metadata().float(meta::HEALTH)
}

/// 26.1 entity flags byte (0 when unknown).
#[must_use]
pub fn flags(e: &Entity) -> u8 {
    e.metadata().byte(meta::FLAGS).unwrap_or(0) as u8
}

/// 26.1 custom name (raw network NBT text component).
#[must_use]
pub fn custom_name(e: &Entity) -> Option<&Bytes> {
    match e.metadata().get(meta::CUSTOM_NAME)? {
        MetaValue::OptComponent(Some(b)) => Some(b),
        _ => None,
    }
}

/// Static type information from ghast-data.
#[must_use]
pub fn info(t: EntityType) -> Option<&'static ghast_data::EntityInfo> {
    match t {
        EntityType::Registry(id) => ghast_data::v26_1::entity(id),
        EntityType::Player => ghast_data::v26_1::entity(data::PLAYER),
        _ => None,
    }
}

pub(crate) fn class(e: &Entity) -> EntityClass {
    match e.ty {
        EntityType::Player => EntityClass::Player,
        EntityType::Registry(data::ITEM) => EntityClass::Item,
        EntityType::Registry(data::EXPERIENCE_ORB) => EntityClass::ExperienceOrb,
        EntityType::Registry(data::PAINTING) => EntityClass::Painting,
        _ if e.living => EntityClass::Mob,
        _ => EntityClass::Object,
    }
}

pub(crate) fn vanilla_size(e: &Entity) -> Option<&'static ghast_data::VanillaEntitySize> {
    e.m.td.and_then(|t| t.size)
}

/// The client's cached dimensions (`Entity.dimensions`).
pub(crate) fn dimensions(e: &Entity) -> Option<ghast_data::EntityDimensions> {
    e.m.dims()
}

/// `getDimensions(getPose())`: variant (baby flag, slime size, armor stand flags), pose, scale.
fn compute_dims(e: &Entity) -> Option<ghast_data::EntityDimensions> {
    let t = e.m.td?;
    let m = &e.m;
    let (variant, slime) = if t.size_meta.is_some() {
        ("default", Some(m.slime_size))
    } else if t.flags_meta.is_some() {
        let f = m.stand_flags;
        let v = match (f & 0x01 != 0, f & 0x10 != 0) {
            (true, true) => "small_marker",
            (false, true) => "marker",
            (true, false) => "small",
            _ => "default",
        };
        (v, None)
    } else if t.baby_meta.is_some() && m.flag(st::BABY) {
        ("baby", None)
    } else {
        ("default", None)
    };
    let d = t.dimensions(variant, slime, i32::from(m.pose))?;
    if t.is_living() && m.pose != 2 {
        let fixed = t.flags & type_flags::FIXED != 0
            || variant.ends_with("marker")
            || t.fixed_poses & (1 << m.pose) != 0;
        return Some(EntityTypeData::scaled(d, m.scale, fixed));
    }
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_round_and_delta_codec() {
        assert_eq!(jround(2.5), 3);
        assert_eq!(jround(-2.5), -2);
        assert_eq!(jround(-2.500_000_1), -3);
        assert_eq!(jround(0.499_999_999_999_999_94), 0);
        assert_eq!(jround(f64::NAN), 0);
        assert_eq!(jround(-0.5), 0);
        assert_eq!(jround(-0.500_000_000_000_000_1), -1);
        assert_eq!(jround(1e300), i64::MAX);
        assert_eq!(jround(-4.5e15 - 1.0), -4_500_000_000_000_001);
        for i in -2000..2000 {
            let x = f64::from(i) * 0.137_5 + 1e-3 * f64::from(i % 7);
            assert_eq!(jround(x), (x + 0.5).floor() as i64, "{x}");
            assert_eq!(mth_floor(x), x.floor() as i32, "{x}");
        }
        let b = [1.0 / 3.0, -7.25, 1e-9];
        assert_eq!(decode_delta(b, [0; 3]), b);
        let d = decode_delta(b, [4096, 0, -1]);
        assert_eq!(d[0], (1365.0 + 4096.0) / 4096.0);
        assert_eq!(d[1].to_bits(), b[1].to_bits());
        assert_eq!(d[2], -1.0 / 4096.0);
    }

    #[test]
    fn block_pos_unpacks() {
        let pack = |x: i64, y: i64, z: i64| {
            ((x & 0x3FF_FFFF) << 38) | ((z & 0x3FF_FFFF) << 12) | (y & 0xFFF)
        };
        assert_eq!(block_pos(pack(-5, 70, 123)), [-5, 70, 123]);
        assert_eq!(
            block_pos(pack(30_000_000, -64, -30_000_000)),
            [30_000_000, -64, -30_000_000]
        );
    }
}
