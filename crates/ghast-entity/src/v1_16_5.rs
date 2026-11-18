//! 1.16.5 (protocol 754) entity tracking.
//!
//! Positions follow the vanilla client exactly: each entity keeps its "packet coordinates"
//! (`f64`). Spawns and teleports set them to the sent doubles; a relative move with delta `d`
//! on an axis computes `floor(x * 4096) + d` and divides by 4096 (`(double) l / 4096.0`), an
//! axis with delta 0 keeps its exact value. Velocities are `v / 8000.0` (double) except at mob
//! spawn (`(float) v / 8000.0F`). Angles are `(float)(b * 360) / 256.0F`. A boat the bot steers
//! (the bot is its first passenger) ignores relative moves and keeps only teleported packet
//! coordinates, like vanilla's `isControlledByLocalInstance` (other controllable vehicles are not
//! modelled).
//!
//! Every packet is parsed by hand from the body; movement packets touch no allocator.

use std::ops::Deref;

use bytes::Bytes;
use ghast_protocol::types::PositionLayout;
use ghast_protocol::types::slot::modern::Slot;
use ghast_protocol::v1_16_5::play::clientbound::{Login, Respawn, id};
use ghast_protocol::v1_16_5::types::Particle;
use ghast_protocol::{BlockPos, Decode, DecodeError, RawPacket, Reader};

use crate::Applied;
use crate::common::{
    IdMask, R, angle, for_each_varint, modern_item, own_nbt, own_str, player_info,
};
use crate::common::{remove_effect, set_effect};
use crate::entity::{Effect, Entity, EntityType, Item, MetaValue};
use crate::store::{Config, Entities, Event};

/// Spawn, movement, destroy, riding, pickup, status packets, plus `login` / `respawn`.
pub const CORE_IDS: &[i32] = &[
    id::LOGIN,
    id::RESPAWN,
    id::NAMED_ENTITY_SPAWN,
    id::SPAWN_ENTITY,
    id::SPAWN_ENTITY_LIVING,
    id::SPAWN_ENTITY_PAINTING,
    id::SPAWN_ENTITY_EXPERIENCE_ORB,
    id::ENTITY_VELOCITY,
    id::ENTITY_DESTROY,
    id::ENTITY,
    id::REL_ENTITY_MOVE,
    id::ENTITY_LOOK,
    id::ENTITY_MOVE_LOOK,
    id::ENTITY_TELEPORT,
    id::ENTITY_HEAD_ROTATION,
    id::ENTITY_STATUS,
    id::ATTACH_ENTITY,
    id::SET_PASSENGERS,
    id::COLLECT,
    id::ENTITY_METADATA,
];
/// `entity_metadata`. It is also in [`CORE_IDS`]: item entities need their stack for pickup
/// counts, so with [`Config::metadata`] off it is still read for item entities (and skipped
/// after the entity id for everything else).
pub const METADATA_IDS: &[i32] = &[id::ENTITY_METADATA];
/// `entity_equipment`.
pub const EQUIPMENT_IDS: &[i32] = &[id::ENTITY_EQUIPMENT];
/// `entity_effect`, `remove_entity_effect`.
pub const EFFECT_IDS: &[i32] = &[id::ENTITY_EFFECT, id::REMOVE_ENTITY_EFFECT];
/// `player_info`.
pub const PLAYER_LIST_IDS: &[i32] = &[id::PLAYER_INFO];

/// Registry ids used by the tracker's rules.
mod ty {
    pub const BOAT: u16 = 6;
    pub const ITEM: u16 = 37;
}

/// 1.16.5 metadata indices.
pub mod meta {
    /// Entity flags byte (0x01 on fire, 0x02 crouching, 0x08 sprinting, 0x10 swimming,
    /// 0x20 invisible, 0x40 glowing, 0x80 elytra flying).
    pub const FLAGS: u8 = 0;
    /// Air (VarInt).
    pub const AIR: u8 = 1;
    /// Custom name (optional chat).
    pub const CUSTOM_NAME: u8 = 2;
    /// Custom name visible.
    pub const CUSTOM_NAME_VISIBLE: u8 = 3;
    /// Silent.
    pub const SILENT: u8 = 4;
    /// No gravity.
    pub const NO_GRAVITY: u8 = 5;
    /// Pose.
    pub const POSE: u8 = 6;
    /// Item entity: the item stack.
    pub const ITEM: u8 = 7;
    /// Living: hand states.
    pub const HAND_STATES: u8 = 7;
    /// Living: health (float).
    pub const HEALTH: u8 = 8;
}

/// The bot's entity view on a 1.16.5 connection. Dereferences to the shared [`Entities`].
#[derive(Clone, Debug)]
pub struct Tracker {
    store: Entities,
    world_name: String,
    wanted: IdMask,
    /// The client's own entity state ([`Tracker::track_client_motion`]).
    client: Option<Box<crate::client16::Client16>>,
}

impl Deref for Tracker {
    type Target = Entities;
    fn deref(&self) -> &Entities {
        &self.store
    }
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
        if config.reads_tab_list() {
            wanted.add(PLAYER_LIST_IDS);
        }
        Tracker {
            store: Entities {
                era: crate::entity::Era::V1_16,
                ..Entities::new(config)
            },
            world_name: String::new(),
            wanted,
            client: None,
        }
    }

    /// Keeps the vanilla client's own view of the entities from now on (off by default): the
    /// client world's tick order ([`Tracker::ticks_before_self`]), the positions the client
    /// interpolates to and the boxes it holds ([`Tracker::client_box`]), each player's profile
    /// name and the living entities' health. What the local player's pushes and entity
    /// collisions read (`ghast-physics` `v1_16_5::push_by`); see the `client16` notes in the
    /// crate docs for what is not modelled. Turn it on before the login packet: the tick order
    /// depends on every entity id the client world has held.
    pub fn track_client_motion(&mut self, on: bool) {
        if on == self.client.is_some() {
            return;
        }
        self.client = on.then(|| {
            let mut c = Box::<crate::client16::Client16>::default();
            c.new_world(self.store.self_id);
            for e in self.store.map.values() {
                c.map.insert(e.id);
                c.spawned(e, None);
            }
            c
        });
    }

    /// `true` while [`Tracker::track_client_motion`] is on.
    #[must_use]
    pub fn tracks_client_motion(&self) -> bool {
        self.client.is_some()
    }

    /// The position the vanilla client holds for entity `id` (interpolated; `None` without
    /// [`Tracker::track_client_motion`] or for types the client view does not keep: only living
    /// entities, boats and shulkers).
    #[must_use]
    pub fn client_position(&self, id: i32) -> Option<[f64; 3]> {
        self.client.as_ref()?.motion.get(&id).map(|m| m.pos)
    }

    /// The bounding box the vanilla client holds for entity `id` (see
    /// [`Tracker::client_position`]), `[min_x, min_y, min_z, max_x, max_y, max_z]`.
    #[must_use]
    pub fn client_box(&self, id: i32) -> Option<[f64; 6]> {
        self.client.as_ref()?.motion.get(&id).map(|m| m.bb)
    }

    /// `isAlive` as far as the client view knows it: `false` once the last health the server
    /// sent a living entity is not above 0 (`true` without the client view).
    #[must_use]
    pub fn client_alive(&self, id: i32) -> bool {
        self.client
            .as_ref()
            .and_then(|c| c.motion.get(&id))
            .is_none_or(|m| !m.dead)
    }

    /// `true` when entity `id` ticks before the bot's own player in the client world's tick
    /// order (needs [`Tracker::track_client_motion`]).
    #[must_use]
    pub fn ticks_before_self(&self, id: i32) -> bool {
        match (&self.client, self.store.self_id) {
            (Some(c), Some(me)) => c.map.before(id, me),
            _ => false,
        }
    }

    /// Entity `id`'s place in the client world's tick order as a sort key (smaller ticks
    /// earlier; `None` when the client world does not hold it or without
    /// [`Tracker::track_client_motion`]).
    #[must_use]
    pub fn client_tick_rank(&self, id: i32) -> Option<u32> {
        self.client.as_ref()?.map.rank(id)
    }

    /// The client world's tick order: every entity id the client world holds, the bot's own
    /// included (empty without [`Tracker::track_client_motion`]).
    pub fn client_tick_order(&self) -> impl Iterator<Item = i32> + '_ {
        self.client.iter().flat_map(|c| c.map.order())
    }

    /// One client tick of the client view (`ClientWorld.tickEntities`): every kept entity that
    /// ticks takes one interpolation step. `column_loaded(chunk_x, chunk_z)` says which columns
    /// the client has (an entity elsewhere does not tick; players always do). Passengers are
    /// left alone. Call it once per client tick, where the bot's world ticks its entities.
    pub fn tick_client(&mut self, column_loaded: impl Fn(i32, i32) -> bool) {
        let Some(c) = &mut self.client else { return };
        c.tick(&self.store, |e| {
            !e.rides && (e.is_player() || entity_ticks_at(e, &column_loaded))
        });
    }

    /// `true` when the client would tick `e` this tick: not riding, and a player or in a
    /// loaded column (`ClientChunkManager.shouldTickEntity` at its client position).
    #[must_use]
    pub fn client_ticks(&self, e: &Entity, column_loaded: impl Fn(i32, i32) -> bool) -> bool {
        !e.rides
            && (e.is_player()
                || match self.client_position(e.id) {
                    Some(p) => column_loaded(chunk(p[0]), chunk(p[2])),
                    None => entity_ticks_at(e, &column_loaded),
                })
    }

    /// `isSpectator` of a player entity: its tab-list game mode is spectator (needs the tab
    /// list or [`Config::player_names`]).
    #[must_use]
    pub fn is_spectator(&self, e: &Entity) -> bool {
        e.is_player() && self.store.tab_game_mode(&e.uuid) == Some(3)
    }

    /// `getEntityName`, what team rules match: a player's profile name (from the tab-list
    /// entry it spawned with; needs [`Tracker::track_client_motion`] and the tab list or
    /// [`Config::player_names`]), else `None`; vanilla uses the UUID string for other entities.
    #[must_use]
    pub fn scoreboard_name(&self, e: &Entity) -> Option<&str> {
        if !e.is_player() {
            return None;
        }
        self.client.as_ref()?.motion.get(&e.id)?.name.as_deref()
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
        let s = &mut self.store;
        let mut client = self.client.as_deref_mut();
        let changed = match packet_id {
            id::REL_ENTITY_MOVE => relative(s, client, r, true, false)?,
            id::ENTITY_MOVE_LOOK => relative(s, client, r, true, true)?,
            id::ENTITY_LOOK => relative(s, client, r, false, true)?,
            id::ENTITY => relative(s, client, r, false, false)?,
            id::ENTITY_TELEPORT => teleport(s, client, r)?,
            id::ENTITY_VELOCITY => {
                let eid = r.read_varint()?;
                let v = vel3(r)?;
                match s.map.get_mut(&eid) {
                    Some(e) => {
                        e.velocity = v.map(|c| f64::from(c) / 8000.0);
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
                    Some(e) => {
                        e.head_yaw = angle(b);
                        true
                    }
                    None => false,
                }
            }
            id::ENTITY_DESTROY => {
                let mut any = false;
                for_each_varint(r, |eid| {
                    if let Some(c) = client.as_deref_mut()
                        && Some(eid) != s.self_id
                    {
                        c.remove(eid);
                    }
                    any |= s.remove(eid);
                })?;
                any
            }
            id::ENTITY_STATUS => {
                let eid = r.read_i32()?;
                let status = r.read_i8()?;
                let keep = s.config.metadata;
                let known = match s.map.get_mut(&eid) {
                    Some(e) => {
                        // LivingEntity.handleEntityEvent(3): setHealth(0), die.
                        if status == 3 && e.living && keep {
                            e.meta.set(meta::HEALTH, MetaValue::Float(0.0));
                        }
                        true
                    }
                    None => false,
                };
                if known {
                    s.event(Event::Status { id: eid, status });
                }
                known
            }
            id::NAMED_ENTITY_SPAWN => {
                let mut e = Entity::new(r.read_varint()?, EntityType::Player);
                e.uuid = r.read_uuid()?;
                e.pos = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
                e.yaw = angle(r.read_i8()?);
                e.pitch = angle(r.read_i8()?);
                e.head_yaw = e.yaw;
                // The client builds the player from its tab-list entry (none: the spawn fails).
                let name = s.tab_name(&e.uuid).map(Box::<str>::from);
                let listed = name.is_some() || !s.config.reads_tab_list();
                let eid = e.id;
                s.spawn(e);
                if listed {
                    client_spawned(s, client, eid, name);
                }
                true
            }
            id::SPAWN_ENTITY => {
                let eid = r.read_varint()?;
                let uuid = r.read_uuid()?;
                let t = r.read_varint()?;
                let pos = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
                let pitch = r.read_i8()?;
                let yaw = r.read_i8()?;
                let data = r.read_i32()?;
                let v = vel3(r)?;
                let Some(t) = known_type(t) else {
                    return Ok(Applied::Ignored);
                };
                let mut e = Entity::new(eid, EntityType::Registry(t));
                e.uuid = uuid;
                e.pos = pos;
                e.pitch = angle(pitch);
                e.yaw = angle(yaw);
                e.head_yaw = e.yaw;
                e.data = data;
                e.velocity = v.map(|c| f64::from(c) / 8000.0);
                s.spawn(e);
                client_spawned(s, client, eid, None);
                true
            }
            id::SPAWN_ENTITY_LIVING => {
                let eid = r.read_varint()?;
                let uuid = r.read_uuid()?;
                let t = r.read_varint()?;
                let pos = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
                let yaw = r.read_i8()?;
                let pitch = r.read_i8()?;
                let head = r.read_i8()?;
                let v = vel3(r)?;
                let Some(t) = known_type(t) else {
                    return Ok(Applied::Ignored);
                };
                let mut e = Entity::new(eid, EntityType::Registry(t));
                e.living = true;
                e.uuid = uuid;
                e.pos = pos;
                e.yaw = angle(yaw);
                e.pitch = angle(pitch);
                e.head_yaw = angle(head);
                e.velocity = v.map(|c| f64::from(f32::from(c) / 8000.0));
                s.spawn(e);
                client_spawned(s, client, eid, None);
                true
            }
            id::SPAWN_ENTITY_EXPERIENCE_ORB => {
                let mut e = Entity::new(r.read_varint()?, EntityType::Registry(24));
                e.pos = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
                e.data = i32::from(r.read_i16()?);
                let eid = e.id;
                s.spawn(e);
                client_spawned(s, client, eid, None);
                true
            }
            id::SPAWN_ENTITY_PAINTING => {
                let eid = r.read_varint()?;
                let uuid = r.read_uuid()?;
                let _motive = r.read_varint()?;
                let p = BlockPos::read(r, PositionLayout::Xzy)?;
                let dir = r.read_u8()?;
                let mut e = Entity::new(eid, EntityType::Registry(55));
                e.uuid = uuid;
                e.pos = [f64::from(p.x), f64::from(p.y), f64::from(p.z)];
                e.data = i32::from(dir);
                s.spawn(e);
                client_spawned(s, client, eid, None);
                true
            }
            id::ATTACH_ENTITY => {
                let eid = r.read_i32()?;
                let holder = r.read_i32()?;
                s.leash(eid, (holder != 0).then_some(holder))
            }
            id::SET_PASSENGERS => {
                let vehicle = r.read_varint()?;
                // Validate the list first so a bad packet changes nothing.
                let start = r.position();
                for_each_varint(r, |_| {})?;
                let mut again = Reader::new(body);
                again.skip(start)?;
                if !s.map.contains_key(&vehicle) {
                    return Ok(Applied::Ignored);
                }
                let n = again.read_varint()?.max(0);
                let ids = (0..n).map_while(|_| again.read_varint().ok());
                s.set_passengers(vehicle, ids)
            }
            id::COLLECT => {
                let collected = r.read_varint()?;
                let collector = r.read_varint()?;
                let amount = r.read_varint()?;
                match s.map.get_mut(&collected) {
                    None => false,
                    Some(e) => {
                        let gone = if e.ty == EntityType::Registry(ty::ITEM) {
                            shrink_item(e, amount)
                        } else {
                            true
                        };
                        s.event(Event::Collected {
                            collected,
                            collector,
                        });
                        if gone {
                            s.remove(collected);
                            if let Some(c) = client {
                                c.remove(collected);
                            }
                        }
                        true
                    }
                }
            }
            id::ENTITY_METADATA => {
                let eid = r.read_varint()?;
                let wanted = match s.map.get(&eid) {
                    None => false,
                    // Without metadata tracking, item entities still need their stack for
                    // pickup counts.
                    Some(e) => s.config.metadata || e.ty == EntityType::Registry(ty::ITEM),
                };
                // The client view reads the size, health and attachment entries.
                let viewed = client.as_ref().is_some_and(|c| c.motion.contains_key(&eid));
                if !wanted && !viewed {
                    return Ok(Applied::Ignored);
                }
                let mut scratch = std::mem::take(&mut s.scratch);
                scratch.clear();
                let res = read_metadata(r, &mut scratch);
                if res.is_ok()
                    && viewed
                    && let Some(c) = client
                    && let Some(e) = s.map.get(&eid)
                {
                    c.data(e, &scratch);
                }
                if res.is_ok()
                    && wanted
                    && let Some(e) = s.map.get_mut(&eid)
                {
                    let all = s.config.metadata;
                    e.meta.set_from(&mut scratch, |i| all || i == meta::ITEM);
                    s.event(Event::Metadata(eid));
                }
                s.scratch = scratch;
                res?;
                true
            }
            id::ENTITY_EQUIPMENT => {
                let eid = r.read_varint()?;
                // Parse the whole list before touching the entity.
                let mut list: [Option<(usize, Option<Item>)>; 8] = Default::default();
                let mut n = 0;
                loop {
                    let b = r.read_u8()?;
                    let item = modern_item(Slot::decode(r)?);
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
                    id: r.read_i8()? as u8,
                    amplifier: r.read_i8()?,
                    duration: r.read_varint()?,
                    flags: r.read_i8()? as u8,
                };
                set_effect(s, eid, effect)
            }
            id::REMOVE_ENTITY_EFFECT => {
                let eid = r.read_varint()?;
                let eff = r.read_i8()? as u8;
                remove_effect(s, eid, eff)
            }
            id::PLAYER_INFO => {
                player_info(s, r)?;
                true
            }
            id::LOGIN => {
                let p = Login::decode(r)?;
                s.clear();
                s.set_self(p.entity_id);
                self.world_name = p.world_name.to_string();
                // A new client world holding the new player.
                if let Some(c) = client {
                    c.new_world(Some(p.entity_id));
                }
                true
            }
            id::RESPAWN => {
                let p = Respawn::decode(r)?;
                // A new player entity: effects reset, metadata is copied over by vanilla.
                if let Some(me) = s.self_id.and_then(|i| s.map.get_mut(&i)) {
                    me.clear_effects();
                    me.equipment = None;
                }
                if *p.world_name != *self.world_name {
                    s.clear();
                    self.world_name = p.world_name.to_string();
                    if let Some(c) = client {
                        c.new_world(s.self_id);
                    }
                } else if let (Some(c), Some(me)) = (client, s.self_id) {
                    // The same world: the new player replaces the old one (`addEntityPrivate`).
                    c.map.remove(me);
                    c.map.insert(me);
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

/// 1.16.5 health of a living entity.
#[must_use]
pub fn health(e: &Entity) -> Option<f32> {
    e.metadata().float(meta::HEALTH)
}

/// 1.16.5 item of an item entity.
#[must_use]
pub fn item(e: &Entity) -> Option<&Item> {
    e.metadata().item(meta::ITEM).flatten()
}

/// 1.16.5 entity flags byte (0 when unknown).
#[must_use]
pub fn flags(e: &Entity) -> u8 {
    e.metadata().byte(meta::FLAGS).unwrap_or(0) as u8
}

/// 1.16.5 custom name (raw chat JSON).
#[must_use]
pub fn custom_name(e: &Entity) -> Option<&str> {
    match e.metadata().get(meta::CUSTOM_NAME)? {
        MetaValue::OptChat(Some(s)) => Some(s),
        _ => None,
    }
}

/// Static type information from ghast-data (players: `player`).
#[must_use]
pub fn info(t: EntityType) -> Option<&'static ghast_data::EntityInfo> {
    let id = match t {
        EntityType::Registry(id) => id,
        EntityType::Player => 106,
        _ => return None,
    };
    ghast_data::v1_16_5::entities().iter().find(|e| e.id == id)
}

fn known_type(t: i32) -> Option<u16> {
    let t = u16::try_from(t).ok()?;
    ghast_data::v1_16_5::entities()
        .iter()
        .any(|e| e.id == t)
        .then_some(t)
}

/// Vanilla `ItemEntity` pickup: the stack shrinks by `amount`; the entity goes when empty
/// (an unknown stack counts as empty, like vanilla's default empty stack).
fn shrink_item(e: &mut Entity, amount: i32) -> bool {
    let Some(MetaValue::Item(Some(it))) = e
        .meta
        .entries_mut()
        .find(|(i, _)| *i == meta::ITEM)
        .map(|(_, v)| v)
    else {
        return true;
    };
    let left = i32::from(it.count) - amount.max(0);
    if left <= 0 {
        return true;
    }
    it.count = left as i8;
    false
}

/// `lfloor(x * 4096)`.
#[inline]
fn encode(x: f64) -> i64 {
    let d = x * 4096.0;
    let l = d as i64;
    if d < l as f64 { l - 1 } else { l }
}

#[inline]
fn axis(x: f64, d: i16) -> f64 {
    if d == 0 {
        x
    } else {
        encode(x).wrapping_add(i64::from(d)) as f64 / 4096.0
    }
}

#[inline]
fn vel3(r: &mut Reader<'_>) -> R<[i16; 3]> {
    Ok([r.read_i16()?, r.read_i16()?, r.read_i16()?])
}

/// `true` when the bot steers `vehicle` (boats only, see the module docs).
fn controlled(s: &Entities, e: &Entity) -> bool {
    e.ty == EntityType::Registry(ty::BOAT)
        && s.self_id.is_some()
        && e.passengers().first().copied() == s.self_id
}

/// The client view's side of a spawn: `addEntityPrivate` and the spawn position.
fn client_spawned(
    s: &Entities,
    client: Option<&mut crate::client16::Client16>,
    id: i32,
    name: Option<Box<str>>,
) {
    let Some(c) = client else { return };
    if Some(id) == s.self_id {
        return;
    }
    let Some(e) = s.map.get(&id) else { return };
    c.add(id);
    c.spawned(e, name);
}

#[inline]
fn relative(
    s: &mut Entities,
    client: Option<&mut crate::client16::Client16>,
    r: &mut Reader<'_>,
    pos: bool,
    look: bool,
) -> R<bool> {
    let eid = r.read_varint()?;
    let d = if pos {
        [r.read_i16()?, r.read_i16()?, r.read_i16()?]
    } else {
        [0; 3]
    };
    let rot = if look {
        Some((r.read_i8()?, r.read_i8()?))
    } else {
        None
    };
    let on_ground = if pos || look { r.read_bool()? } else { false };
    if Some(eid) == s.self_id {
        return Ok(false);
    }
    let Some(e) = s.map.get(&eid) else {
        return Ok(false);
    };
    if controlled(s, e) {
        return Ok(false);
    }
    let Some(e) = s.map.get_mut(&eid) else {
        return Ok(false);
    };
    if pos {
        e.pos = [
            axis(e.pos[0], d[0]),
            axis(e.pos[1], d[1]),
            axis(e.pos[2], d[2]),
        ];
    }
    if let Some((y, p)) = rot {
        e.yaw = angle(y);
        e.pitch = angle(p);
    }
    e.on_ground = on_ground;
    if let Some(c) = client {
        if pos {
            c.moved(eid, e.pos);
        } else if look {
            c.looked(eid);
        }
    }
    s.event(Event::Moved(eid));
    Ok(true)
}

fn teleport(
    s: &mut Entities,
    client: Option<&mut crate::client16::Client16>,
    r: &mut Reader<'_>,
) -> R<bool> {
    let eid = r.read_varint()?;
    let p = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
    let (y, pi) = (r.read_i8()?, r.read_i8()?);
    let on_ground = r.read_bool()?;
    if Some(eid) == s.self_id {
        return Ok(false);
    }
    let Some(e) = s.map.get(&eid) else {
        return Ok(false);
    };
    let ctl = controlled(s, e);
    let Some(e) = s.map.get_mut(&eid) else {
        return Ok(false);
    };
    // Packet coordinates are set even for a vehicle the bot controls.
    e.pos = p;
    if !ctl {
        e.yaw = angle(y);
        e.pitch = angle(pi);
        e.on_ground = on_ground;
        if let Some(c) = client {
            c.moved(eid, p);
        }
    }
    s.event(Event::Moved(eid));
    Ok(true)
}

/// 1.16 metadata: index byte (`0xFF` ends), VarInt serializer id, value.
fn read_metadata(r: &mut Reader<'_>, out: &mut Vec<(u8, MetaValue)>) -> R<()> {
    loop {
        let index = r.read_u8()?;
        if index == 0xFF {
            return Ok(());
        }
        let v = match r.read_varint()? {
            0 => MetaValue::Byte(r.read_i8()?),
            1 => MetaValue::VarInt(r.read_varint()?),
            2 => MetaValue::Float(r.read_f32()?),
            3 => MetaValue::String(own_str(r)?),
            4 => MetaValue::Chat(own_str(r)?),
            5 => MetaValue::OptChat(if r.read_bool()? {
                Some(own_str(r)?)
            } else {
                None
            }),
            6 => MetaValue::Item(modern_item(Slot::decode(r)?)),
            7 => MetaValue::Bool(r.read_bool()?),
            8 => MetaValue::Rotation([r.read_f32()?, r.read_f32()?, r.read_f32()?]),
            9 => MetaValue::BlockPos(pos(BlockPos::read(r, PositionLayout::Xzy)?)),
            10 => MetaValue::OptBlockPos(if r.read_bool()? {
                Some(pos(BlockPos::read(r, PositionLayout::Xzy)?))
            } else {
                None
            }),
            11 => MetaValue::Direction(r.read_varint()?),
            12 => MetaValue::OptUuid(if r.read_bool()? {
                Some(r.read_uuid()?)
            } else {
                None
            }),
            13 => MetaValue::BlockState(r.read_varint()?),
            14 => MetaValue::Nbt(
                own_nbt(Some(ghast_protocol::nbt::RawNbt::read(r)?)).unwrap_or_default(),
            ),
            15 => MetaValue::Particle(Particle::decode(r)?.particle_id),
            16 => MetaValue::VillagerData([r.read_varint()?, r.read_varint()?, r.read_varint()?]),
            17 => MetaValue::OptVarInt(r.read_varint()?),
            18 => MetaValue::Pose(r.read_varint()?),
            _ => return Err(DecodeError::Invalid("unknown metadata serializer")),
        };
        out.push((index, v));
    }
}

fn pos(p: BlockPos) -> [i32; 3] {
    [p.x, p.y, p.z]
}

#[inline]
fn chunk(v: f64) -> i32 {
    (v.floor() as i32) >> 4
}

fn entity_ticks_at(e: &Entity, column_loaded: &impl Fn(i32, i32) -> bool) -> bool {
    column_loaded(chunk(e.pos[0]), chunk(e.pos[2]))
}

/// The vanilla 1.16.5 size entry for an entity (`None` for hanging entities).
pub(crate) fn vanilla_size(e: &Entity) -> Option<&'static ghast_data::VanillaEntitySize> {
    let id = match e.ty {
        EntityType::Player => 106,
        EntityType::Registry(38 | 40 | 55) => return None, // item frame, leash knot, painting
        EntityType::Registry(id) => i32::from(id),
        _ => return None,
    };
    ghast_data::v1_16_5::vanilla_entity_sizes()
        .iter()
        .find(|v| v.registry_id == id)
}

/// 1.16.5 dimensions with the metadata variants the client applies.
pub(crate) fn dimensions(
    e: &Entity,
    vs: &'static ghast_data::VanillaEntitySize,
) -> Option<ghast_data::EntityDimensions> {
    let m = &e.meta;
    let pose = match m.get(meta::POSE) {
        Some(MetaValue::Pose(p)) => *p,
        _ => 0,
    };
    let slime = match m.get(15) {
        Some(MetaValue::VarInt(n)) => Some(*n),
        _ => None,
    };
    let idx = if e.ty == EntityType::Registry(60) {
        16
    } else {
        15
    };
    let baby = matches!(m.get(idx), Some(MetaValue::Bool(true)));
    dims_with(e.ty, vs, pose, slime, m.byte(14).unwrap_or(0) as u8, baby)
}

/// [`dimensions`] from the size data itself: the pose (players), the slime size (slimes and
/// magma cubes; `None` keeps the spawn size), the armor stand flags and the baby flag.
pub(crate) fn dims_with(
    ty: EntityType,
    vs: &'static ghast_data::VanillaEntitySize,
    pose: i32,
    slime: Option<i32>,
    stand: u8,
    baby: bool,
) -> Option<ghast_data::EntityDimensions> {
    use ghast_data::EntityDimensions;
    let base = vs.default_dimensions().or_else(|| {
        vs.registered.map(|(width, height)| EntityDimensions {
            width,
            height,
            eye_height: height * 0.85,
        })
    })?;
    let EntityType::Registry(id) = ty else {
        // Player: pose table.
        let name = [
            "standing",
            "fall_flying",
            "sleeping",
            "swimming",
            "spin_attack",
            "crouching",
            "dying",
        ]
        .get(pose as usize)
        .copied()
        .unwrap_or("standing");
        return Some(vs.pose(name).unwrap_or(base));
    };
    match id {
        // slime, magma cube: size VarInt at 15.
        75 | 44 => {
            if let Some(n) = slime {
                return Some(ghast_data::v1_16_5::slime_dimensions(n));
            }
        }
        // armor stand flags byte at 14: 0x01 small, 0x10 marker.
        1 => {
            let f = stand;
            let name = match (f & 0x01 != 0, f & 0x10 != 0) {
                (true, true) => "small_marker",
                (false, true) => "marker",
                (true, false) => "small",
                _ => "default",
            };
            return Some(vs.variant(name).map_or(base, |v| v.dimensions));
        }
        _ => {}
    }
    // Baby flag: Bool at 15 (ageable mobs, zombies), 16 for piglins.
    if baby && let Some(b) = vs.baby() {
        return Some(b);
    }
    Some(base)
}
