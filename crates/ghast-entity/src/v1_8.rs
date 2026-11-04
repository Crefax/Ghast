//! 1.8.x (protocol 47) entity tracking.
//!
//! Positions follow the vanilla client exactly: every entity keeps the server position as
//! `int`s in 1/32 blocks (`serverPosX`); spawns and teleports set it, relative moves add the
//! signed byte deltas, and the position is `serverPos / 32.0` (double division). Velocities are
//! `v / 8000.0` (double) except at mob spawn, where vanilla divides in `float`
//! (`(float) v / 8000.0F`). Angles are `(float)(b * 360) / 256.0F`.
//!
//! Every packet is parsed by hand straight from the packet body: the hot ones (moves, looks,
//! teleports, velocity, head rotation, destroy) touch no allocator.

use std::ops::Deref;

use bytes::Bytes;
use ghast_data::EntityKind;
use ghast_protocol::types::PositionLayout;
use ghast_protocol::types::slot::legacy::Slot;
use ghast_protocol::v1_8::play::clientbound::id;
use ghast_protocol::{BlockPos, Decode, DecodeError, RawPacket, Reader};

use crate::Applied;
use crate::common::{
    IdMask, R, angle, for_each_varint, legacy_item, own_str, player_info, skip_string,
};
use crate::entity::{EQUIPMENT_SLOTS, Effect, Entity, EntityType, Item, MetaValue};
use crate::store::{Config, Entities, Event};

/// Spawn, movement, destroy, riding, pickup, status packets (always needed for tracking),
/// plus `login` / `respawn` (world changes).
pub const CORE_IDS: [i32; 19] = [
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
    id::COLLECT,
    id::SPAWN_ENTITY_WEATHER,
];
/// `entity_metadata`.
pub const METADATA_IDS: [i32; 1] = [id::ENTITY_METADATA];
/// `entity_equipment`.
pub const EQUIPMENT_IDS: [i32; 1] = [id::ENTITY_EQUIPMENT];
/// `entity_effect`, `remove_entity_effect`.
pub const EFFECT_IDS: [i32; 2] = [id::ENTITY_EFFECT, id::REMOVE_ENTITY_EFFECT];
/// `player_info`.
pub const PLAYER_LIST_IDS: [i32; 1] = [id::PLAYER_INFO];

/// 1.8 metadata indices.
pub mod meta {
    /// Entity flags byte (0x01 on fire, 0x02 sneaking, 0x08 sprinting, 0x10 eating/using,
    /// 0x20 invisible).
    pub const FLAGS: u8 = 0;
    /// Air (short).
    pub const AIR: u8 = 1;
    /// Custom name (string).
    pub const CUSTOM_NAME: u8 = 2;
    /// Custom name visible (byte).
    pub const CUSTOM_NAME_VISIBLE: u8 = 3;
    /// Living: health (float).
    pub const HEALTH: u8 = 6;
    /// Item entity: the item stack.
    pub const ITEM: u8 = 10;
}

/// The bot's entity view on a 1.8 connection. Dereferences to the shared [`Entities`] queries.
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
        wanted.add(&CORE_IDS);
        if config.metadata {
            wanted.add(&METADATA_IDS);
        }
        if config.equipment {
            wanted.add(&EQUIPMENT_IDS);
        }
        if config.effects {
            wanted.add(&EFFECT_IDS);
        }
        if config.player_list {
            wanted.add(&PLAYER_LIST_IDS);
        }
        Tracker {
            store: Entities::new(config),
            dimension: 0,
            wanted,
        }
    }

    /// The shared store (also reachable through `Deref`), mutable for [`Entities::drain_events`].
    pub fn store_mut(&mut self) -> &mut Entities {
        &mut self.store
    }

    /// `true` when this tracker wants packet `id` with its configuration (for a `PacketFilter`).
    #[must_use]
    #[inline]
    pub fn wants(&self, id: i32) -> bool {
        self.wanted.contains(id)
    }

    /// Every packet id this tracker wants with its configuration.
    pub fn packet_ids(&self) -> impl Iterator<Item = i32> + '_ {
        self.wanted.iter()
    }

    /// Applies one clientbound play packet (only the body is read; nothing is allocated for
    /// movement packets).
    ///
    /// # Errors
    /// A malformed packet body (the packet's effect may be partly applied for multi-entry
    /// packets such as `entity_destroy` / `player_info`; single-entity packets are atomic).
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
            id::NAMED_ENTITY_SPAWN => spawn_player(s, r)?,
            id::SPAWN_ENTITY => spawn_object(s, r)?,
            id::SPAWN_ENTITY_LIVING => spawn_mob(s, r)?,
            id::SPAWN_ENTITY_EXPERIENCE_ORB => {
                let mut e = Entity::new(r.read_varint()?, EntityType::ExperienceOrb);
                set_fixed(&mut e, [r.read_i32()?, r.read_i32()?, r.read_i32()?]);
                e.data = i32::from(r.read_i16()?);
                s.spawn(e);
                true
            }
            id::SPAWN_ENTITY_PAINTING => {
                let eid = r.read_varint()?;
                skip_string(r)?; // title
                let p = BlockPos::read(r, PositionLayout::Xyz)?;
                let dir = r.read_u8()?;
                let mut e = Entity::new(eid, EntityType::Painting);
                e.pos = [f64::from(p.x), f64::from(p.y), f64::from(p.z)];
                e.data = i32::from(dir);
                s.spawn(e);
                true
            }
            // Lightning is a weather effect in 1.8, not an entity in the id map.
            id::SPAWN_ENTITY_WEATHER => false,
            id::ATTACH_ENTITY => {
                let eid = r.read_i32()?;
                let vid = r.read_i32()?;
                let leash = r.read_u8()? != 0;
                let target = s.map.contains_key(&vid).then_some(vid);
                if leash {
                    s.leash(eid, target)
                } else {
                    // An unknown vehicle id dismounts, as in vanilla.
                    s.mount(eid, target, true)
                }
            }
            id::COLLECT => {
                let collected = r.read_varint()?;
                let collector = r.read_varint()?;
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
                let slot = r.read_i16()?;
                let item = legacy_item(Slot::decode(r)?);
                match s.map.get_mut(&eid) {
                    Some(e) if (0..5).contains(&slot) => {
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
                    flags: u8::from(r.read_bool()?),
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
                let eid = r.read_i32()?;
                let _game_mode = r.read_u8()?;
                let dim = r.read_i8()?;
                s.clear();
                s.set_self(eid);
                self.dimension = i32::from(dim);
                true
            }
            id::RESPAWN => {
                let dim = r.read_i32()?;
                // A new player entity: effects and metadata start empty.
                if let Some(me) = s.self_id.and_then(|i| s.map.get_mut(&i)) {
                    me.effects.clear();
                    me.meta.clear();
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

/// 1.8 health of a living entity (`None` without metadata).
#[must_use]
pub fn health(e: &Entity) -> Option<f32> {
    e.metadata().float(meta::HEALTH)
}

/// 1.8 item of an item entity.
#[must_use]
pub fn item(e: &Entity) -> Option<&Item> {
    e.metadata().item(meta::ITEM).flatten()
}

/// 1.8 entity flags byte (0 when unknown).
#[must_use]
pub fn flags(e: &Entity) -> u8 {
    e.metadata().byte(meta::FLAGS).unwrap_or(0) as u8
}

/// 1.8 custom name.
#[must_use]
pub fn custom_name(e: &Entity) -> Option<&str> {
    match e.metadata().get(meta::CUSTOM_NAME)? {
        MetaValue::String(s) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// Static type information from ghast-data.
#[must_use]
pub fn info(ty: EntityType) -> Option<&'static ghast_data::EntityInfo> {
    match ty {
        EntityType::LegacyMob(id) => ghast_data::v1_8::entity(EntityKind::Mob, u16::from(id)),
        EntityType::LegacyObject(id) => ghast_data::v1_8::entity(EntityKind::Object, u16::from(id)),
        _ => None,
    }
}

#[inline]
fn set_fixed(e: &mut Entity, p: [i32; 3]) {
    e.fixed = p;
    e.pos = p.map(|c| f64::from(c) / 32.0);
}

#[inline]
fn vel3(r: &mut Reader<'_>) -> R<[i16; 3]> {
    Ok([r.read_i16()?, r.read_i16()?, r.read_i16()?])
}

/// `entity`, `rel_entity_move`, `entity_look`, `entity_move_look` (vanilla
/// `onEntityUpdate`: the bare `entity` packet also sets `on_ground = false`).
#[inline]
fn relative(s: &mut Entities, r: &mut Reader<'_>, pos: bool, look: bool) -> R<bool> {
    let eid = r.read_varint()?;
    let d = if pos {
        [r.read_i8()?, r.read_i8()?, r.read_i8()?]
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
    let Some(e) = s.map.get_mut(&eid) else {
        return Ok(false);
    };
    let f = [
        e.fixed[0].wrapping_add(i32::from(d[0])),
        e.fixed[1].wrapping_add(i32::from(d[1])),
        e.fixed[2].wrapping_add(i32::from(d[2])),
    ];
    set_fixed(e, f);
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
    let p = [r.read_i32()?, r.read_i32()?, r.read_i32()?];
    let (y, pi) = (r.read_i8()?, r.read_i8()?);
    let on_ground = r.read_bool()?;
    if Some(eid) == s.self_id {
        return Ok(false);
    }
    let Some(e) = s.map.get_mut(&eid) else {
        return Ok(false);
    };
    set_fixed(e, p);
    e.yaw = angle(y);
    e.pitch = angle(pi);
    e.on_ground = on_ground;
    s.event(Event::Moved(eid));
    Ok(true)
}

fn spawn_player(s: &mut Entities, r: &mut Reader<'_>) -> R<bool> {
    let mut e = Entity::new(r.read_varint()?, EntityType::Player);
    e.uuid = r.read_uuid()?;
    set_fixed(&mut e, [r.read_i32()?, r.read_i32()?, r.read_i32()?]);
    e.yaw = angle(r.read_i8()?);
    e.pitch = angle(r.read_i8()?);
    e.head_yaw = e.yaw;
    let held = r.read_i16()?;
    if s.config.equipment && held != 0 {
        let mut eq: Box<[Option<Item>; EQUIPMENT_SLOTS]> = Box::default();
        eq[0] = Some(Item {
            id: i32::from(held),
            count: 1,
            damage: 0,
            nbt: None,
        });
        e.equipment = Some(eq);
    }
    spawn_metadata(s, r, e)
}

fn spawn_object(s: &mut Entities, r: &mut Reader<'_>) -> R<bool> {
    let eid = r.read_varint()?;
    let ty = r.read_i8()? as u8;
    let p = [r.read_i32()?, r.read_i32()?, r.read_i32()?];
    let pitch = r.read_i8()?;
    let yaw = r.read_i8()?;
    let data = r.read_i32()?;
    // Vanilla reads (and applies) the velocity only when data > 0 (not != 0).
    let v = if data > 0 { Some(vel3(r)?) } else { None };
    if !matches!(
        ty,
        1 | 2 | 10 | 50 | 51 | 60..=66 | 70..=73 | 75..=78 | 90
    ) {
        return Ok(false); // vanilla creates no entity for other ids
    }
    let mut e = Entity::new(eid, EntityType::LegacyObject(ty));
    set_fixed(&mut e, p);
    e.pitch = angle(pitch);
    e.yaw = angle(yaw);
    e.head_yaw = e.yaw;
    e.data = data;
    if let Some(v) = v {
        e.velocity = v.map(|c| f64::from(c) / 8000.0);
    }
    s.spawn(e);
    Ok(true)
}

fn spawn_mob(s: &mut Entities, r: &mut Reader<'_>) -> R<bool> {
    let eid = r.read_varint()?;
    let ty = r.read_u8()?;
    let p = [r.read_i32()?, r.read_i32()?, r.read_i32()?];
    let yaw = r.read_i8()?;
    let pitch = r.read_i8()?;
    let head = r.read_i8()?;
    let v = vel3(r)?;
    // Unknown and abstract ids (48 "Mob", 49 "Monster") spawn nothing in vanilla.
    if ty == 48 || ty == 49 || ghast_data::v1_8::entity(EntityKind::Mob, u16::from(ty)).is_none() {
        return Ok(false);
    }
    let mut e = Entity::new(eid, EntityType::LegacyMob(ty));
    set_fixed(&mut e, p);
    e.yaw = angle(yaw);
    e.pitch = angle(pitch);
    e.head_yaw = angle(head);
    // Mob spawn divides in float: (double)((float) v / 8000.0F).
    e.velocity = v.map(|c| f64::from(f32::from(c) / 8000.0));
    spawn_metadata(s, r, e)
}

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

/// 1.8 metadata: `type << 5 | index` header bytes, `0x7F` terminator.
fn read_metadata(r: &mut Reader<'_>, out: &mut Vec<(u8, MetaValue)>) -> R<()> {
    loop {
        let h = r.read_u8()?;
        if h == 0x7F {
            return Ok(());
        }
        let v = match h >> 5 {
            0 => MetaValue::Byte(r.read_i8()?),
            1 => MetaValue::Short(r.read_i16()?),
            2 => MetaValue::Int(r.read_i32()?),
            3 => MetaValue::Float(r.read_f32()?),
            4 => MetaValue::String(own_str(r)?),
            5 => MetaValue::Item(legacy_item(Slot::decode(r)?)),
            6 => MetaValue::BlockPos([r.read_i32()?, r.read_i32()?, r.read_i32()?]),
            _ => MetaValue::Rotation([r.read_f32()?, r.read_f32()?, r.read_f32()?]),
        };
        out.push((h & 0x1F, v));
    }
}

pub(crate) fn set_effect(s: &mut Entities, eid: i32, effect: Effect) -> bool {
    match s.map.get_mut(&eid) {
        Some(e) => {
            match e.effects.iter_mut().find(|x| x.id == effect.id) {
                Some(x) => *x = effect,
                None => e.effects.push(effect),
            }
            s.event(Event::Effects(eid));
            true
        }
        None => false,
    }
}

pub(crate) fn remove_effect(s: &mut Entities, eid: i32, effect: u8) -> bool {
    match s.map.get_mut(&eid) {
        Some(e) => {
            e.effects.retain(|x| x.id != effect);
            s.event(Event::Effects(eid));
            true
        }
        None => false,
    }
}

/// The vanilla 1.8.9 size entry for an entity (`None` for paintings and hanging objects).
pub(crate) fn vanilla_size(e: &Entity) -> Option<&'static ghast_data::VanillaEntitySize> {
    let all = ghast_data::v1_8::vanilla_entity_sizes();
    match e.ty {
        EntityType::Player => ghast_data::v1_8::vanilla_entity_size("Player"),
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

/// 1.8.9 dimensions with the metadata variants the client applies.
pub(crate) fn dimensions(
    e: &Entity,
    vs: &'static ghast_data::VanillaEntitySize,
) -> Option<ghast_data::EntityDimensions> {
    let base = vs.default_dimensions()?;
    let m = &e.meta;
    let pick = |name: &str| Some(vs.variant(name).map_or(base, |v| v.dimensions));
    match e.ty {
        // Slime / magma cube: size byte at 16.
        EntityType::LegacyMob(55 | 62) => match m.byte(16) {
            Some(n) => Some(ghast_data::v1_8::slime_dimensions(i32::from(n))),
            None => Some(base),
        },
        // Skeleton type byte at 13: 1 = wither skeleton.
        EntityType::LegacyMob(51) if m.byte(13) == Some(1) => pick("wither"),
        // Guardian flags int at 16: 0x04 = elder.
        EntityType::LegacyMob(68) => match m.get(16) {
            Some(MetaValue::Int(f)) if f & 0x04 != 0 => pick("elder"),
            _ => Some(base),
        },
        // Zombie / zombie pigman child byte at 12 (1 = baby; the client keeps the adult box).
        EntityType::LegacyMob(54 | 57) if m.byte(12) == Some(1) => Some(vs.baby().unwrap_or(base)),
        // Ageable mobs: age byte at 12, negative = baby.
        EntityType::LegacyMob(_) if m.byte(12).is_some_and(|a| a < 0) => {
            Some(vs.baby().unwrap_or(base))
        }
        // Armor stand flags byte at 10: 0x01 small, 0x10 marker.
        EntityType::LegacyObject(78) => {
            let f = m.byte(10).unwrap_or(0) as u8;
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
