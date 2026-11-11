//! Version-independent entity store shared by the per-version trackers.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use uuid::Uuid;

use crate::entity::{Entity, EntityType, MetaValue};

/// What the tracker keeps. Everything off costs nothing beyond position tracking; packets for a
/// disabled part are not parsed (and the client should not even decode them, see the version
/// modules' packet id lists).
///
/// `#[non_exhaustive]`: build it from [`Config::default`] with the `with_*` methods:
///
/// ```
/// let c = ghast_entity::Config::default().with_equipment(false).with_skins(true);
/// assert!(c.metadata && !c.equipment && c.skins);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    /// Entity metadata (flags, health, item of item entities, names, ...). Default `true`.
    pub metadata: bool,
    /// Equipment. Default `true`.
    pub equipment: bool,
    /// Status effects. Default `true`.
    pub effects: bool,
    /// Tab list (`player_info`). Default `true`.
    pub player_list: bool,
    /// Queue [`Event`]s for [`Entities::drain_events`]. Default `false`.
    pub events: bool,
    /// Keep the base64 `"textures"` profile property (skin / cape URLs) of tab-list entries in
    /// [`PlayerInfo::textures`]. Default `false`: properties are skipped without allocating.
    pub skins: bool,
}

impl Config {
    /// Sets [`Config::metadata`].
    #[must_use]
    pub const fn with_metadata(mut self, on: bool) -> Self {
        self.metadata = on;
        self
    }

    /// Sets [`Config::equipment`].
    #[must_use]
    pub const fn with_equipment(mut self, on: bool) -> Self {
        self.equipment = on;
        self
    }

    /// Sets [`Config::effects`].
    #[must_use]
    pub const fn with_effects(mut self, on: bool) -> Self {
        self.effects = on;
        self
    }

    /// Sets [`Config::player_list`].
    #[must_use]
    pub const fn with_player_list(mut self, on: bool) -> Self {
        self.player_list = on;
        self
    }

    /// Sets [`Config::events`].
    #[must_use]
    pub const fn with_events(mut self, on: bool) -> Self {
        self.events = on;
        self
    }

    /// Sets [`Config::skins`].
    #[must_use]
    pub const fn with_skins(mut self, on: bool) -> Self {
        self.skins = on;
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            metadata: true,
            equipment: true,
            effects: true,
            player_list: true,
            events: false,
            skins: false,
        }
    }
}

/// A change notification (only queued with [`Config::events`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// A new entity (or a replacement for an existing id).
    Spawned(i32),
    /// An entity was removed (destroy, pickup, world change).
    Removed {
        /// Id.
        id: i32,
        /// Its type.
        ty: EntityType,
    },
    /// Server position or rotation changed (movement, teleport).
    Moved(i32),
    /// Velocity changed.
    Velocity(i32),
    /// `entity_status` (e.g. 2 hurt, 3 death).
    Status {
        /// Id.
        id: i32,
        /// Status byte.
        status: i8,
    },
    /// Metadata changed.
    Metadata(i32),
    /// Equipment changed.
    Equipment(i32),
    /// Effects changed.
    Effects(i32),
    /// Riding changed: `id` now rides `vehicle`.
    Mounted {
        /// Rider.
        id: i32,
        /// Vehicle (`None` = dismounted).
        vehicle: Option<i32>,
    },
    /// An item / orb / arrow was picked up (`collector` may be the bot).
    Collected {
        /// Picked-up entity.
        collected: i32,
        /// Collector.
        collector: i32,
    },
    /// The tab list changed.
    PlayerList,
}

/// A tab-list entry (`#[non_exhaustive]`: read-only for callers, fields may be added).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlayerInfo {
    /// Profile name.
    pub name: Box<str>,
    /// Game mode.
    pub game_mode: i32,
    /// Latency in ms.
    pub ping: i32,
    /// Display name (raw chat JSON), if set (1.8 – 1.16; see
    /// [`PlayerInfo::display_name_nbt`] for 26.1).
    pub display_name: Option<Box<str>>,
    /// The base64 `"textures"` profile property as sent (only with [`Config::skins`]; its
    /// signature is not kept). Decoded it is JSON with the skin and cape URLs.
    pub textures: Option<Box<str>>,
    /// 26.1: the display name as its raw network NBT text component, if set.
    pub display_name_nbt: Option<bytes::Bytes>,
    /// Shown in the tab list (26.1 `UPDATE_LISTED`; always `true` before 26.1).
    pub listed: bool,
    /// 26.1 tab list order (`UPDATE_LIST_ORDER`; 0 before).
    pub list_order: i32,
    /// Which tab list object this is (26.1 player entities keep the one they first saw).
    pub(crate) generation: u64,
}

#[derive(Default, Clone, Copy)]
pub(crate) struct IdHasher(u64);

impl Hasher for IdHasher {
    #[inline]
    fn finish(&self) -> u64 {
        let h = self.0.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^ (h >> 29)
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 << 8) | u64::from(b);
        }
    }

    #[inline]
    fn write_i32(&mut self, v: i32) {
        self.0 = u64::from(v as u32);
    }

    #[inline]
    fn write_u64(&mut self, v: u64) {
        self.0 = v;
    }

    #[inline]
    fn write_u128(&mut self, v: u128) {
        self.0 = (v as u64) ^ ((v >> 64) as u64);
    }
}

pub(crate) type Fast<K, V> = HashMap<K, V, BuildHasherDefault<IdHasher>>;

/// The tracked entities and tab list (queries are the same for every version).
#[derive(Clone, Debug, Default)]
pub struct Entities {
    pub(crate) map: Fast<i32, Entity>,
    pub(crate) self_id: Option<i32>,
    pub(crate) config: Config,
    pub(crate) events: Vec<Event>,
    pub(crate) players: Fast<Uuid, PlayerInfo>,
    pub(crate) scratch: Vec<(u8, MetaValue)>,
    /// Set once any leash link exists (skips the leash scan on removal otherwise).
    pub(crate) any_leash: bool,
    /// The tracker's protocol family (selects the vanilla size table).
    pub(crate) era: crate::entity::Era,
    /// Last tab list object generation handed out.
    pub(crate) generations: u64,
}

impl Entities {
    pub(crate) fn new(config: Config) -> Self {
        Entities {
            config,
            ..Default::default()
        }
    }

    /// The entity with this id (including the bot's own entry, see [`Entities::self_id`]).
    #[inline]
    #[must_use]
    pub fn get(&self, id: i32) -> Option<&Entity> {
        self.map.get(&id)
    }

    /// Number of tracked entities (including the bot's own entry).
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// `true` when nothing is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Other entities (the bot's own entry is skipped), arbitrary order.
    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        let me = self.self_id;
        self.map.values().filter(move |e| Some(e.id) != me)
    }

    /// The bot's own entity id (from `login`). Its entry carries metadata, effects, equipment
    /// and riding state, but **no position**: the client owns its own position.
    #[must_use]
    pub fn self_id(&self) -> Option<i32> {
        self.self_id
    }

    /// The bot's own entry.
    #[must_use]
    pub fn me(&self) -> Option<&Entity> {
        self.map.get(&self.self_id?)
    }

    /// Entities whose server position is within `radius` of `p` (linear scan).
    pub fn within(&self, p: [f64; 3], radius: f64) -> impl Iterator<Item = &Entity> {
        let r2 = radius * radius;
        self.iter().filter(move |e| {
            let d = [e.pos[0] - p[0], e.pos[1] - p[1], e.pos[2] - p[2]];
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= r2
        })
    }

    /// Alias of [`Entities::within`].
    pub fn near(&self, p: [f64; 3], radius: f64) -> impl Iterator<Item = &Entity> {
        self.within(p, radius)
    }

    /// The tab list.
    #[must_use]
    pub fn player_info(&self, uuid: &Uuid) -> Option<&PlayerInfo> {
        self.players.get(uuid)
    }

    /// All tab-list entries.
    pub fn players(&self) -> impl Iterator<Item = (&Uuid, &PlayerInfo)> {
        self.players.iter()
    }

    /// Name of a player entity (from the tab list).
    #[must_use]
    pub fn player_name(&self, id: i32) -> Option<&str> {
        let e = self.map.get(&id)?;
        e.is_player()
            .then(|| self.players.get(&e.uuid).map(|p| &*p.name))
            .flatten()
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> Config {
        self.config
    }

    /// Takes the queued events (keeps the buffer's capacity).
    pub fn drain_events(&mut self) -> std::vec::Drain<'_, Event> {
        self.events.drain(..)
    }

    /// Removes every entity (keeps the tab list and the bot's id).
    pub fn clear(&mut self) {
        if self.config.events {
            let me = self.self_id;
            self.events.extend(
                self.map
                    .values()
                    .filter(|e| Some(e.id) != me)
                    .map(|e| Event::Removed { id: e.id, ty: e.ty }),
            );
        }
        let me = self.self_id.and_then(|id| self.map.remove(&id));
        self.map.clear();
        if let Some(mut me) = me {
            me.vehicle = None;
            me.passengers.clear();
            me.leash_holder = None;
            self.map.insert(me.id, me);
        }
    }

    #[inline]
    pub(crate) fn event(&mut self, e: Event) {
        if self.config.events {
            self.events.push(e);
        }
    }

    pub(crate) fn set_self(&mut self, id: i32) {
        if let Some(old) = self.self_id {
            self.map.remove(&old);
        }
        self.self_id = Some(id);
        let mut me = Entity::new(id, EntityType::Player);
        self.resolve(&mut me);
        self.map.insert(id, me);
    }

    /// Stamps the protocol family and resolves the vanilla size entry.
    fn resolve(&self, e: &mut Entity) {
        use crate::entity::Era;
        e.era = self.era;
        e.sizer = match self.era {
            #[cfg(feature = "v1_8")]
            Era::V1_8 => crate::v1_8::vanilla_size(e),
            #[cfg(feature = "v1_12")]
            Era::V1_12 => crate::v1_12::vanilla_size(e),
            #[cfg(feature = "v1_16_5")]
            Era::V1_16 => crate::v1_16_5::vanilla_size(e),
            #[cfg(feature = "v26_1")]
            Era::V26 => crate::v26_1::vanilla_size(e),
            #[allow(unreachable_patterns)]
            _ => None,
        };
    }

    /// Adds an entity, replacing (and unlinking) any entity with the same id, as vanilla does.
    pub(crate) fn spawn(&mut self, mut e: Entity) {
        let id = e.id;
        self.resolve(&mut e);
        if Some(id) == self.self_id {
            return;
        }
        if self.map.contains_key(&id) {
            self.unlink(id);
        }
        self.map.insert(id, e);
        self.event(Event::Spawned(id));
    }

    pub(crate) fn remove(&mut self, id: i32) -> bool {
        if Some(id) == self.self_id || !self.map.contains_key(&id) {
            return false;
        }
        self.unlink(id);
        if let Some(e) = self.map.remove(&id) {
            self.event(Event::Removed { id, ty: e.ty });
        }
        true
    }

    /// Detaches `id` from its vehicle, passengers and leash links.
    fn unlink(&mut self, id: i32) {
        let Some(e) = self.map.get_mut(&id) else {
            return;
        };
        let vehicle = e.vehicle.take();
        let passengers = std::mem::take(&mut e.passengers);
        if let Some(v) = vehicle
            && let Some(v) = self.map.get_mut(&v)
        {
            v.passengers.retain(|&p| p != id);
        }
        for p in passengers {
            if let Some(p) = self.map.get_mut(&p)
                && p.vehicle == Some(id)
            {
                p.vehicle = None;
            }
        }
        if self.any_leash {
            for other in self.map.values_mut() {
                if other.leash_holder == Some(id) {
                    other.leash_holder = None;
                }
            }
        }
    }

    /// `true` when `vehicle` is `rider` or (indirectly) rides `rider`.
    fn rides(&self, vehicle: i32, rider: i32) -> bool {
        let mut cur = Some(vehicle);
        let mut guard = 0;
        while let Some(c) = cur {
            if c == rider {
                return true;
            }
            guard += 1;
            if guard > 64 {
                return true;
            }
            cur = self.map.get(&c).and_then(|e| e.vehicle);
        }
        false
    }

    /// Makes `rider` ride `vehicle` (or dismount with `None`). `single` = 1.8 semantics (one
    /// rider per vehicle: a previous rider is dismounted). Returns `false` when ignored.
    pub(crate) fn mount(&mut self, rider: i32, vehicle: Option<i32>, single: bool) -> bool {
        if !self.map.contains_key(&rider) {
            return false;
        }
        if let Some(v) = vehicle
            && (!self.map.contains_key(&v) || self.rides(v, rider))
        {
            return false;
        }
        // Leave the old vehicle.
        let old = self.map.get_mut(&rider).and_then(|e| e.vehicle.take());
        if let Some(o) = old
            && let Some(o) = self.map.get_mut(&o)
        {
            o.passengers.retain(|&p| p != rider);
        }
        if let Some(v) = vehicle {
            let prev: Vec<i32> = if single {
                self.map
                    .get_mut(&v)
                    .map(|e| std::mem::take(&mut e.passengers))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            for p in prev {
                if let Some(p) = self.map.get_mut(&p) {
                    p.vehicle = None;
                }
            }
            if let Some(ve) = self.map.get_mut(&v) {
                ve.passengers.push(rider);
            }
            if let Some(r) = self.map.get_mut(&rider) {
                r.vehicle = Some(v);
            }
        }
        self.event(Event::Mounted { id: rider, vehicle });
        true
    }

    /// 1.16 `set_passengers`: ejects every current passenger of `vehicle`, then mounts `ids`.
    pub(crate) fn set_passengers(&mut self, vehicle: i32, ids: impl Iterator<Item = i32>) -> bool {
        let Some(v) = self.map.get_mut(&vehicle) else {
            return false;
        };
        let old = std::mem::take(&mut v.passengers);
        for p in &old {
            if let Some(p) = self.map.get_mut(p)
                && p.vehicle == Some(vehicle)
            {
                p.vehicle = None;
            }
        }
        // Reuse the old buffer for the new list (no allocation in steady state).
        let mut old = old;
        old.clear();
        if let Some(v) = self.map.get_mut(&vehicle) {
            v.passengers = old;
        }
        for id in ids {
            self.mount(id, Some(vehicle), false);
        }
        true
    }

    pub(crate) fn leash(&mut self, id: i32, holder: Option<i32>) -> bool {
        match self.map.get_mut(&id) {
            Some(e) => {
                e.leash_holder = holder;
                self.any_leash |= holder.is_some();
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Entities {
        let mut s = Entities::new(Config {
            events: true,
            ..Config::default()
        });
        for id in 1..=4 {
            s.spawn(Entity::new(id, EntityType::Registry(6)));
        }
        s
    }

    /// Bytes per tracked entity (the map slot adds the 4-byte key, padded).
    #[test]
    fn entity_size() {
        let n = std::mem::size_of::<Entity>();
        eprintln!("size_of::<Entity>() = {n}");
        assert!(n <= if cfg!(feature = "v26_1") { 400 } else { 264 }, "{n}");
    }

    #[test]
    fn mount_single_and_multi() {
        let mut s = store();
        assert!(s.mount(1, Some(2), true));
        assert!(s.mount(3, Some(2), true)); // 1.8: replaces rider 1
        assert_eq!(s.get(2).unwrap().passengers(), &[3]);
        assert_eq!(s.get(1).unwrap().vehicle(), None);
        assert!(!s.mount(2, Some(3), true), "cycle");
        assert!(s.set_passengers(4, [1, 3].into_iter()));
        assert_eq!(s.get(4).unwrap().passengers(), &[1, 3]);
        assert_eq!(s.get(2).unwrap().passengers(), &[] as &[i32]);
        assert_eq!(s.get(3).unwrap().vehicle(), Some(4));
        s.remove(4);
        assert_eq!(s.get(1).unwrap().vehicle(), None);
        assert_eq!(s.get(3).unwrap().vehicle(), None);
    }

    #[test]
    fn self_entry_survives_clear() {
        let mut s = store();
        s.set_self(9);
        s.spawn(Entity::new(9, EntityType::Registry(1))); // ignored: own id
        assert_eq!(s.get(9).unwrap().entity_type(), EntityType::Player);
        assert!(!s.remove(9));
        s.drain_events().for_each(drop);
        s.clear();
        assert_eq!(s.len(), 1);
        assert!(s.me().is_some());
        assert_eq!(s.drain_events().count(), 4);
        assert_eq!(s.iter().count(), 0);
    }
}
