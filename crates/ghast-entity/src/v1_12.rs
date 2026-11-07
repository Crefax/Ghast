//! 1.12.2 (protocol 340) entity tracking: the first member of the 1.9 – 1.12 family (the
//! packets and client rules below are shared by 1.9 – 1.12; only 1.12.2 is implemented).
//!
//! Positions follow the vanilla client exactly. Every entity keeps a tracked position in 1/4096
//! blocks (vanilla `trackedX` = `lfloor(x * 4096)`). Spawns and teleports set it from the sent
//! doubles and the position to those doubles; a relative move adds the `i16` deltas to it and
//! the position becomes `tracked / 4096.0` on **every** axis (unlike 1.16, an axis with delta 0
//! also snaps to the 1/4096 grid, and so does the bare `entity` packet). Velocities are
//! `v / 8000.0` (double) except at mob spawn (`(float) v / 8000.0F`); an object spawn applies its
//! velocity only when its (possibly reset) object data is positive. Angles are
//! `(float)(b * 360) / 256.0F`.
//!
//! As in 1.8, mobs and objects are separate id spaces: mobs are [`EntityType::LegacyMob`] with
//! the 1.12 entity registry id, objects [`EntityType::LegacyObject`] with the spawn-object type.
//! A boat the bot steers (the bot is its first passenger) ignores relative moves and teleports
//! for its position and rotation (vanilla `canPassengerSteer`; only the tracked value moves).
//! A pickup (`collect`) always removes the collected entity.
//!
//! Every packet is parsed by hand from the body; movement packets touch no allocator.

use std::ops::Deref;

use bytes::Bytes;
use ghast_data::EntityKind;
use ghast_protocol::types::PositionLayout;
use ghast_protocol::types::slot::legacy::Slot;
use ghast_protocol::v1_12_2::play::clientbound::id;
use ghast_protocol::{BlockPos, Decode, DecodeError, RawPacket, Reader};

use crate::Applied;
use crate::common::{
    IdMask, R, angle, for_each_varint, legacy_item, own_nbt, own_str, player_info, skip_string,
};
use crate::entity::{EQUIPMENT_SLOTS, Effect, Entity, EntityType, Era, Item, MetaValue};
use crate::store::{Config, Entities, Event};
use crate::v1_8::{remove_effect, set_effect};

/// Spawn, movement, destroy, riding, pickup, status packets, plus `login` / `respawn`.
pub const CORE_IDS: &[i32] = &[
    id::LOGIN,
    id::RESPAWN,
    id::NAMED_ENTITY_SPAWN,
    id::SPAWN_ENTITY,
    id::SPAWN_ENTITY_LIVING,
    id::SPAWN_ENTITY_PAINTING,
    id::SPAWN_ENTITY_EXPERIENCE_ORB,
    id::SPAWN_ENTITY_WEATHER,
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
];
/// `entity_metadata`.
pub const METADATA_IDS: &[i32] = &[id::ENTITY_METADATA];
/// `entity_equipment`.
pub const EQUIPMENT_IDS: &[i32] = &[id::ENTITY_EQUIPMENT];
/// `entity_effect`, `remove_entity_effect`.
pub const EFFECT_IDS: &[i32] = &[id::ENTITY_EFFECT, id::REMOVE_ENTITY_EFFECT];
/// `player_info`.
pub const PLAYER_LIST_IDS: &[i32] = &[id::PLAYER_INFO];

/// Object types and registry ids used by the tracker's rules.
mod ty {
    /// Spawn-object type of a boat.
    pub const BOAT_OBJECT: u8 = 1;
    /// Spawn-object type of a fishing bobber (object data: the owner's entity id).
    pub const FISHING_BOBBER_OBJECT: u8 = 90;
    /// Spawn-object type of a llama spit (its constructor takes the velocity).
    pub const LLAMA_SPIT_OBJECT: u8 = 68;
}

/// 1.12.2 metadata indices (vanilla data tracker registration order).
pub mod meta {
    /// Entity flags byte (0x01 on fire, 0x02 sneaking, 0x08 sprinting, 0x10 using an item,
    /// 0x20 invisible, 0x40 glowing, 0x80 elytra flying).
    pub const FLAGS: u8 = 0;
    /// Air (VarInt).
    pub const AIR: u8 = 1;
    /// Custom name (string).
    pub const CUSTOM_NAME: u8 = 2;
    /// Custom name visible (boolean).
    pub const CUSTOM_NAME_VISIBLE: u8 = 3;
    /// Silent (boolean).
    pub const SILENT: u8 = 4;
    /// No gravity (boolean).
    pub const NO_GRAVITY: u8 = 5;
    /// Item entity: the item stack.
    pub const ITEM: u8 = 6;
    /// Living: hand states byte (0x01 hand active, 0x02 off hand).
    pub const HAND_STATES: u8 = 6;
    /// Living: health (float).
    pub const HEALTH: u8 = 7;
    /// Mob: AI flags byte.
    pub const MOB_FLAGS: u8 = 11;
    /// Ageable mobs and zombies: baby (boolean); slimes: size (VarInt).
    pub const BABY: u8 = 12;
    /// Slime / magma cube size (VarInt).
    pub const SLIME_SIZE: u8 = 12;
    /// Armor stand flags byte (0x01 small, 0x04 arms, 0x08 no base plate, 0x10 marker).
    pub const ARMOR_STAND_FLAGS: u8 = 11;
}

/// The bot's entity view on a 1.12.2 connection. Dereferences to the shared [`Entities`].
#[derive(Clone, Debug)]
pub struct Tracker {
    store: Entities,
    dimension: i32,
    wanted: IdMask,
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
        if config.metadata {
            wanted.add(METADATA_IDS);
        }
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
                era: Era::V1_12,
                ..Entities::new(config)
            },
            dimension: 0,
            wanted,
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
        let changed = match packet_id {
            id::REL_ENTITY_MOVE => relative(s, r, true, false)?,
            id::ENTITY_MOVE_LOOK => relative(s, r, true, true)?,
            id::ENTITY_LOOK => relative(s, r, false, true)?,
            id::ENTITY => relative(s, r, false, false)?,
            id::ENTITY_TELEPORT => teleport(s, r)?,
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
            id::NAMED_ENTITY_SPAWN => {
                let mut e = Entity::new(r.read_varint()?, EntityType::Player);
                e.uuid = r.read_uuid()?;
                set_pos(&mut e, [r.read_f64()?, r.read_f64()?, r.read_f64()?]);
                e.yaw = angle(r.read_i8()?);
                e.pitch = angle(r.read_i8()?);
                e.head_yaw = e.yaw;
                spawn_metadata(s, r, e)?
            }
            id::SPAWN_ENTITY => spawn_object(s, r)?,
            id::SPAWN_ENTITY_LIVING => spawn_mob(s, r)?,
            id::SPAWN_ENTITY_EXPERIENCE_ORB => {
                let mut e = Entity::new(r.read_varint()?, EntityType::ExperienceOrb);
                set_pos(&mut e, [r.read_f64()?, r.read_f64()?, r.read_f64()?]);
                e.data = i32::from(r.read_i16()?);
                s.spawn(e);
                true
            }
            id::SPAWN_ENTITY_PAINTING => {
                let eid = r.read_varint()?;
                let uuid = r.read_uuid()?;
                skip_string(r)?; // title
                let p = BlockPos::read(r, PositionLayout::Xyz)?;
                let dir = r.read_u8()?;
                let mut e = Entity::new(eid, EntityType::Painting);
                e.uuid = uuid;
                e.pos = [f64::from(p.x), f64::from(p.y), f64::from(p.z)];
                e.data = i32::from(dir);
                s.spawn(e);
                true
            }
            // Lightning goes to the world's weather effects, not the entity id map.
            id::SPAWN_ENTITY_WEATHER => false,
            id::ATTACH_ENTITY => {
                let eid = r.read_i32()?;
                let holder = r.read_i32()?;
                // Only mobs can be leashed; an unknown holder detaches.
                match s.map.get(&eid) {
                    Some(e) if matches!(e.ty, EntityType::LegacyMob(_)) => {
                        let holder = s.map.contains_key(&holder).then_some(holder);
                        s.leash(eid, holder)
                    }
                    _ => false,
                }
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
                let _count = r.read_varint()?;
                let known = s.map.contains_key(&collected);
                if known {
                    s.event(Event::Collected {
                        collected,
                        collector,
                    });
                    s.remove(collected);
                }
                known
            }
            id::ENTITY_METADATA => {
                let eid = r.read_varint()?;
                if !s.map.contains_key(&eid) {
                    return Ok(Applied::Ignored);
                }
                let mut scratch = std::mem::take(&mut s.scratch);
                scratch.clear();
                let res = read_metadata(r, &mut scratch);
                if res.is_ok()
                    && let Some(e) = s.map.get_mut(&eid)
                {
                    for (i, v) in scratch.drain(..) {
                        e.meta.set(i, v);
                    }
                    s.event(Event::Metadata(eid));
                }
                s.scratch = scratch;
                res?;
                true
            }
            id::ENTITY_EQUIPMENT => {
                let eid = r.read_varint()?;
                let slot = r.read_varint()?;
                let item = legacy_item(Slot::decode(r)?);
                match s.map.get_mut(&eid) {
                    // 0 main hand, 1 off hand, 2 feet, 3 legs, 4 chest, 5 head.
                    Some(e) if (0..EQUIPMENT_SLOTS as i32).contains(&slot) => {
                        e.equipment.get_or_insert_with(Default::default)[slot as usize] = item;
                        s.event(Event::Equipment(eid));
                        true
                    }
                    _ => false,
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
                // Only living entities keep effects.
                match s.map.get(&eid) {
                    Some(e) if e.living => set_effect(s, eid, effect),
                    _ => false,
                }
            }
            id::REMOVE_ENTITY_EFFECT => {
                let eid = r.read_varint()?;
                let eff = r.read_i8()? as u8;
                match s.map.get(&eid) {
                    Some(e) if e.living => remove_effect(s, eid, eff),
                    _ => false,
                }
            }
            id::PLAYER_INFO => {
                player_info(s, r)?;
                true
            }
            id::LOGIN => {
                let eid = r.read_i32()?;
                let _game_mode = r.read_u8()?;
                let dim = r.read_i32()?;
                s.clear();
                s.set_self(eid);
                self.dimension = dim;
                true
            }
            id::RESPAWN => {
                let dim = r.read_i32()?;
                // A new player entity: effects and equipment start empty, the metadata is
                // copied over from the old one (as in 1.16).
                if let Some(me) = s.self_id.and_then(|i| s.map.get_mut(&i)) {
                    me.effects.clear();
                    me.equipment = None;
                }
                if dim != self.dimension {
                    s.clear();
                    self.dimension = dim;
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

/// 1.12.2 health of a living entity (`None` without metadata).
#[must_use]
pub fn health(e: &Entity) -> Option<f32> {
    e.metadata().float(meta::HEALTH)
}

/// 1.12.2 item of an item entity (`None` without metadata).
#[must_use]
pub fn item(e: &Entity) -> Option<&Item> {
    e.metadata().item(meta::ITEM).flatten()
}

/// 1.12.2 entity flags byte (0 when unknown).
#[must_use]
pub fn flags(e: &Entity) -> u8 {
    e.metadata().byte(meta::FLAGS).unwrap_or(0) as u8
}

/// 1.12.2 custom name (plain string; empty means none).
#[must_use]
pub fn custom_name(e: &Entity) -> Option<&str> {
    match e.metadata().get(meta::CUSTOM_NAME)? {
        MetaValue::String(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// Static type information from ghast-data (`ghast_data::v1_12`; registry paths such as
/// `"zombie"`, players: `None`).
#[must_use]
pub fn info(ty: EntityType) -> Option<&'static ghast_data::EntityInfo> {
    match ty {
        EntityType::LegacyMob(id) => ghast_data::v1_12::entity(EntityKind::Mob, u16::from(id)),
        EntityType::LegacyObject(id) => {
            ghast_data::v1_12::entity(EntityKind::Object, u16::from(id))
        }
        EntityType::ExperienceOrb => ghast_data::v1_12::entity(EntityKind::Mob, 2),
        EntityType::Painting => ghast_data::v1_12::entity(EntityKind::Mob, 9),
        _ => None,
    }
}

/// `lfloor(x * 4096)` (vanilla `EntityTracker` position encoding).
#[inline]
fn encode(x: f64) -> i64 {
    let d = x * 4096.0;
    let l = d as i64;
    if d < l as f64 { l.wrapping_sub(1) } else { l }
}

/// Spawn / teleport: the position is the sent doubles, the tracked value their encoding.
#[inline]
fn set_pos(e: &mut Entity, p: [f64; 3]) {
    e.pos = p;
    e.traced = p.map(encode);
}

#[inline]
fn vel3(r: &mut Reader<'_>) -> R<[i16; 3]> {
    Ok([r.read_i16()?, r.read_i16()?, r.read_i16()?])
}

/// `true` when the bot steers the entity (a boat whose first passenger is the bot).
fn controlled(s: &Entities, e: &Entity) -> bool {
    e.ty == EntityType::LegacyObject(ty::BOAT_OBJECT)
        && s.self_id.is_some()
        && e.passengers.first().copied() == s.self_id
}

/// `entity`, `rel_entity_move`, `entity_look`, `entity_move_look` (vanilla `onEntityUpdate`:
/// the tracked position moves by the deltas and the position becomes `tracked / 4096.0` on all
/// three axes; the bare `entity` packet also sets `on_ground = false`).
#[inline]
fn relative(s: &mut Entities, r: &mut Reader<'_>, pos: bool, look: bool) -> R<bool> {
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
    let ctl = controlled(s, e);
    let Some(e) = s.map.get_mut(&eid) else {
        return Ok(false);
    };
    for (t, d) in e.traced.iter_mut().zip(d) {
        *t = t.wrapping_add(i64::from(d));
    }
    if ctl {
        return Ok(false);
    }
    e.pos = e.traced.map(|t| t as f64 / 4096.0);
    if let Some((y, p)) = rot {
        e.yaw = angle(y);
        e.pitch = angle(p);
    }
    e.on_ground = on_ground;
    s.event(Event::Moved(eid));
    Ok(true)
}

fn teleport(s: &mut Entities, r: &mut Reader<'_>) -> R<bool> {
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
    if ctl {
        // Only the tracked value follows a vehicle the bot steers.
        e.traced = p.map(encode);
        return Ok(false);
    }
    set_pos(e, p);
    e.yaw = angle(y);
    e.pitch = angle(pi);
    e.on_ground = on_ground;
    s.event(Event::Moved(eid));
    Ok(true)
}

/// Object types the 1.12 client creates an entity for.
fn known_object(t: u8) -> bool {
    matches!(
        t,
        1 | 2 | 3 | 10 | 50 | 51 | 60..=68 | 70..=73 | 75..=79 | 90 | 91 | 93
    )
}

/// Object types whose object data vanilla resets to 0 before deciding about the velocity.
fn data_reset(t: u8) -> bool {
    matches!(t, 63 | 64 | 66 | 67 | 70 | 71 | 73 | 75 | 77 | 90 | 93)
}

fn spawn_object(s: &mut Entities, r: &mut Reader<'_>) -> R<bool> {
    let eid = r.read_varint()?;
    let uuid = r.read_uuid()?;
    let t = r.read_i8()? as u8;
    let p = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
    let pitch = r.read_i8()?;
    let yaw = r.read_i8()?;
    let data = r.read_i32()?;
    let v = vel3(r)?;
    if !known_object(t) {
        return Ok(false); // vanilla creates no entity for other types
    }
    // A fishing bobber needs its owner: a player the client knows.
    if t == ty::FISHING_BOBBER_OBJECT
        && !s.map.get(&data).is_some_and(|o| o.ty == EntityType::Player)
    {
        return Ok(false);
    }
    let mut e = Entity::new(eid, EntityType::LegacyObject(t));
    e.uuid = uuid;
    set_pos(&mut e, p);
    e.pitch = angle(pitch);
    e.yaw = angle(yaw);
    e.head_yaw = e.yaw;
    e.data = data;
    let effective = if data_reset(t) { 0 } else { data };
    if effective > 0 || t == ty::LLAMA_SPIT_OBJECT {
        e.velocity = v.map(|c| f64::from(c) / 8000.0);
    }
    s.spawn(e);
    Ok(true)
}

fn spawn_mob(s: &mut Entities, r: &mut Reader<'_>) -> R<bool> {
    let eid = r.read_varint()?;
    let uuid = r.read_uuid()?;
    let t = r.read_varint()?;
    let p = [r.read_f64()?, r.read_f64()?, r.read_f64()?];
    let yaw = r.read_i8()?;
    let pitch = r.read_i8()?;
    let head = r.read_i8()?;
    let v = vel3(r)?;
    // Unknown registry ids spawn nothing (vanilla logs "Skipping Entity").
    let Some(t) = u8::try_from(t)
        .ok()
        .filter(|&t| ghast_data::v1_12::entity(EntityKind::Mob, u16::from(t)).is_some())
    else {
        return Ok(false);
    };
    let mut e = Entity::new(eid, EntityType::LegacyMob(t));
    e.uuid = uuid;
    set_pos(&mut e, p);
    e.yaw = angle(yaw);
    e.pitch = angle(pitch);
    e.head_yaw = angle(head);
    // Mob spawn divides in float: (double)((float) v / 8000.0F).
    e.velocity = v.map(|c| f64::from(f32::from(c) / 8000.0));
    spawn_metadata(s, r, e)
}

/// Spawns that carry metadata (mobs, players): kept with [`Config::metadata`].
fn spawn_metadata(s: &mut Entities, r: &mut Reader<'_>, mut e: Entity) -> R<bool> {
    if s.config.metadata {
        let mut scratch = std::mem::take(&mut s.scratch);
        scratch.clear();
        let res = read_metadata(r, &mut scratch);
        for (i, v) in scratch.drain(..) {
            e.meta.set(i, v);
        }
        s.scratch = scratch;
        res?;
    }
    s.spawn(e);
    Ok(true)
}

/// 1.9 – 1.12 metadata: index byte (`0xFF` ends), VarInt serializer id, value.
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
            5 => MetaValue::Item(legacy_item(Slot::decode(r)?)),
            6 => MetaValue::Bool(r.read_bool()?),
            7 => MetaValue::Rotation([r.read_f32()?, r.read_f32()?, r.read_f32()?]),
            8 => MetaValue::BlockPos(pos(BlockPos::read(r, PositionLayout::Xyz)?)),
            9 => MetaValue::OptBlockPos(if r.read_bool()? {
                Some(pos(BlockPos::read(r, PositionLayout::Xyz)?))
            } else {
                None
            }),
            10 => MetaValue::Direction(r.read_varint()?),
            11 => MetaValue::OptUuid(if r.read_bool()? {
                Some(r.read_uuid()?)
            } else {
                None
            }),
            12 => MetaValue::BlockState(r.read_varint()?),
            13 => MetaValue::Nbt(
                own_nbt(Some(ghast_protocol::nbt::RawNbt::read(r)?)).unwrap_or_default(),
            ),
            _ => return Err(DecodeError::Invalid("unknown metadata serializer")),
        };
        out.push((index, v));
    }
}

fn pos(p: BlockPos) -> [i32; 3] {
    [p.x, p.y, p.z]
}

/// The vanilla 1.12.2 size entry for an entity (`None` for paintings and hanging objects).
pub(crate) fn vanilla_size(e: &Entity) -> Option<&'static ghast_data::VanillaEntitySize> {
    let all = ghast_data::v1_12::vanilla_entity_sizes();
    match e.ty {
        EntityType::Player => ghast_data::v1_12::vanilla_entity_size("player"),
        EntityType::LegacyMob(id) => all
            .iter()
            .find(|v| v.spawn == "mob" && v.registry_id == i32::from(id)),
        // Item frame and leash knot are hanging entities.
        EntityType::LegacyObject(71 | 77) => None,
        EntityType::LegacyObject(t) => {
            let t = i32::from(t);
            all.iter()
                .find(|v| v.spawn == "object" && v.object_types.contains(&(t, e.data)))
                .or_else(|| {
                    all.iter().find(|v| {
                        v.spawn == "object" && v.object_types.iter().any(|&(ty, _)| ty == t)
                    })
                })
        }
        EntityType::ExperienceOrb => all.iter().find(|v| v.spawn == "xp_orb"),
        _ => None,
    }
}

/// 1.12.2 dimensions with the metadata variants the client applies.
pub(crate) fn dimensions(
    e: &Entity,
    vs: &'static ghast_data::VanillaEntitySize,
) -> Option<ghast_data::EntityDimensions> {
    let base = vs.default_dimensions()?;
    let m = &e.meta;
    let pick = |name: &str| Some(vs.variant(name).map_or(base, |v| v.dimensions));
    match e.ty {
        // Players: 0.6 x 1.65 while sneaking (applied by the next size update).
        EntityType::Player if m.byte(meta::FLAGS).is_some_and(|f| f & 0x02 != 0) => {
            pick("sneaking")
        }
        // Slime / magma cube: size VarInt at 12.
        EntityType::LegacyMob(55 | 62) => match m.get(meta::SLIME_SIZE) {
            Some(MetaValue::VarInt(n)) => Some(ghast_data::v1_12::slime_dimensions(*n)),
            _ => Some(base),
        },
        // Ageable mobs and zombies: baby boolean at 12.
        EntityType::LegacyMob(_) if matches!(m.get(meta::BABY), Some(MetaValue::Bool(true))) => {
            Some(vs.baby().unwrap_or(base))
        }
        // Armor stand flags byte at 11: 0x01 small, 0x10 marker.
        EntityType::LegacyObject(78) => {
            let f = m.byte(meta::ARMOR_STAND_FLAGS).unwrap_or(0) as u8;
            match (f & 0x01 != 0, f & 0x10 != 0) {
                (true, true) => pick("small_marker"),
                (false, true) => pick("marker"),
                (true, false) => pick("small"),
                _ => Some(base),
            }
        }
        _ => Some(base),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_floors() {
        assert_eq!(encode(1.0), 4096);
        assert_eq!(encode(-0.0001), -1);
        assert_eq!(encode(0.5 / 4096.0), 0);
        assert_eq!(encode(-1.0), -4096);
    }

    #[test]
    fn item_object_id() {
        assert!(known_object(2));
        assert!(!known_object(4));
    }
}
