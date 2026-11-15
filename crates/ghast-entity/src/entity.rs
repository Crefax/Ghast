//! Tracked entity state.

use bytes::Bytes;
use uuid::Uuid;

/// What kind of entity this is, in the id space of the packet that spawned it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EntityType {
    /// A player (`named_entity_spawn`).
    Player,
    /// 1.8 – 1.12 mob id (`spawn_entity_living`; the entity registry id), e.g. 54 = zombie. See
    /// `ghast_data::v1_8::entity(EntityKind::Mob, id)` / `ghast_data::v1_12::entity(..)`.
    LegacyMob(u8),
    /// 1.8 – 1.12 object type (`spawn_entity`), e.g. 2 = item, 1 = boat. See
    /// `ghast_data::v1_8::entity(EntityKind::Object, id)` / `ghast_data::v1_12::entity(..)`.
    LegacyObject(u8),
    /// 1.8 – 1.12 experience orb (`spawn_entity_experience_orb`; orb value in [`Entity::data`]).
    ExperienceOrb,
    /// 1.8 – 1.12 painting (`spawn_entity_painting`).
    Painting,
    /// 1.13+ entity registry id (1.16.5: every spawn packet except players), see
    /// `ghast_data::v1_16_5::entity`.
    Registry(u16),
}

/// Version-independent classification of an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EntityClass {
    /// A player.
    Player,
    /// A dropped item (stack: [`Entity::item`]).
    Item,
    /// An experience orb (value: [`Entity::data`]).
    ExperienceOrb,
    /// A living non-player entity (spawned with the mob-spawn packet).
    Mob,
    /// Any other spawned object (boats, minecarts, arrows, falling blocks, armor stands in 1.8, ...).
    Object,
    /// Paintings.
    Painting,
}

/// An item stack (equipment, item entities, metadata).
///
/// `damage` is the 1.8 damage/metadata value (always 0 in 1.13+, where damage lives in NBT).
/// `nbt` is a private copy of the raw NBT bytes (never a view into a packet buffer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// Item id.
    pub id: i32,
    /// Stack size.
    pub count: i8,
    /// 1.8 damage / metadata.
    pub damage: i16,
    /// Raw NBT (`None` = no tag).
    pub nbt: Option<Bytes>,
}

/// An active status effect as the server reported it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    /// Effect id (1 = speed, 8 = jump boost, ...).
    pub id: u8,
    /// Amplifier (level - 1).
    pub amplifier: i8,
    /// Duration in ticks at the time of the packet (not counted down here).
    pub duration: i32,
    /// 1.8: bit 0 = hide particles. 1.12: bit 0 ambient, bit 1 show particles. 1.16: bit 0 ambient,
    /// bit 1 show particles, bit 2 show icon.
    pub flags: u8,
}

/// One entity metadata value (union of the 1.8, 1.12.2 and 1.16.5 serializer types; variants
/// marked 1.16 also occur in 1.12 where 1.12 has that serializer).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MetaValue {
    /// Byte.
    Byte(i8),
    /// 1.8 short.
    Short(i16),
    /// 1.8 int.
    Int(i32),
    /// 1.16 VarInt.
    VarInt(i32),
    /// Float.
    Float(f32),
    /// String (1.8) / string (1.16).
    String(Box<str>),
    /// 1.16 chat component (raw JSON).
    Chat(Box<str>),
    /// 1.16 optional chat component (raw JSON).
    OptChat(Option<Box<str>>),
    /// Item stack (`None` = empty).
    Item(Option<Item>),
    /// 1.16 boolean.
    Bool(bool),
    /// Rotation (three floats).
    Rotation([f32; 3]),
    /// Block position.
    BlockPos([i32; 3]),
    /// 1.16 optional block position.
    OptBlockPos(Option<[i32; 3]>),
    /// 1.16 direction.
    Direction(i32),
    /// 1.16 optional UUID.
    OptUuid(Option<Uuid>),
    /// 1.16 optional block state (0 = none).
    BlockState(i32),
    /// 1.16 NBT (raw copy).
    Nbt(Bytes),
    /// 1.16 particle (only the particle type id is kept).
    Particle(i32),
    /// 1.16 villager data (type, profession, level).
    VillagerData([i32; 3]),
    /// 1.16 optional VarInt (0 = none, else value + 1).
    OptVarInt(i32),
    /// 1.16 / 26.1 pose.
    Pose(i32),
    /// 26.1 long (VarLong).
    Long(i64),
    /// 26.1 text component (the raw network NBT, copied).
    Component(Bytes),
    /// 26.1 optional text component (raw network NBT).
    OptComponent(Option<Bytes>),
    /// 26.1 item stack (`None` = empty; component patch copied out of the packet).
    #[cfg(any(feature = "v26_1", feature = "v26_2"))]
    Stack(Option<ghast_protocol::types::slot::v775::ItemStack>),
    /// 26.1 registry or enum value: `(serializer id, value)` (cat / cow / wolf / frog / pig /
    /// chicken / zombie nautilus variants and sound variants, sniffer / armadillo / copper golem
    /// states, humanoid arm).
    Enum(u8, i32),
    /// 26.1 vector (three floats).
    Vector3([f32; 3]),
    /// 26.1 quaternion (four floats).
    Quaternion([f32; 4]),
    /// 26.1 value kept as its raw wire bytes, copied: `(serializer id, bytes)` (optional block
    /// state, particle, particle list, optional global position, painting variant, resolvable
    /// profile).
    Raw(u8, Bytes),
}

/// The metadata of one entity: `(index, value)` pairs, at most one per index, in arrival order.
///
/// Scalar updates overwrite in place (no allocation). The pairs live in one exact-size heap
/// block (no spare capacity): an entity typically has a handful of entries, set once at spawn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metadata {
    entries: Box<[(u8, MetaValue)]>,
}

impl Metadata {
    /// The value at `index`.
    #[must_use]
    pub fn get(&self, index: u8) -> Option<&MetaValue> {
        self.entries
            .iter()
            .find(|(i, _)| *i == index)
            .map(|(_, v)| v)
    }

    /// Sets the value at `index`.
    pub fn set(&mut self, index: u8, value: MetaValue) {
        match self.entries.iter_mut().find(|(i, _)| *i == index) {
            Some(slot) => slot.1 = value,
            None => self.push_new(index, value),
        }
    }

    /// Appends a new index (the block grows by exactly one entry).
    #[cold]
    #[inline(never)]
    fn push_new(&mut self, index: u8, value: MetaValue) {
        let mut v = Vec::with_capacity(self.entries.len() + 1);
        v.extend(std::mem::take(&mut self.entries));
        v.push((index, value));
        self.entries = v.into_boxed_slice();
    }

    /// Applies `src` (drained) in order, as [`Metadata::set`] would one by one, skipping indices
    /// `keep` refuses; grows the block at most once, to the exact size.
    #[inline]
    pub(crate) fn set_from(&mut self, src: &mut Vec<(u8, MetaValue)>, keep: impl Fn(u8) -> bool) {
        let mut it = src.drain(..);
        // Overwrites in place until the first index this entity does not have yet.
        while let Some((i, v)) = it.next() {
            if !keep(i) {
                continue;
            }
            match self.entries.iter_mut().find(|(k, _)| *k == i) {
                Some(slot) => slot.1 = v,
                None => return self.grow_from((i, v), it, keep),
            }
        }
    }

    /// The rest of [`Metadata::set_from`] once a new index occurs: one exact-size block.
    #[cold]
    #[inline(never)]
    fn grow_from(
        &mut self,
        first: (u8, MetaValue),
        rest: std::vec::Drain<'_, (u8, MetaValue)>,
        keep: impl Fn(u8) -> bool,
    ) {
        let tail = rest.as_slice();
        let mut new = 1;
        for (j, (i, _)) in tail.iter().enumerate() {
            if keep(*i)
                && *i != first.0
                && !self.entries.iter().any(|(k, _)| k == i)
                && !tail[..j].iter().any(|(k, _)| k == i)
            {
                new += 1;
            }
        }
        let mut out = Vec::with_capacity(self.entries.len() + new);
        out.extend(std::mem::take(&mut self.entries));
        out.push(first);
        for (i, v) in rest {
            if !keep(i) {
                continue;
            }
            match out.iter_mut().find(|(k, _)| *k == i) {
                Some(slot) => slot.1 = v,
                None => out.push((i, v)),
            }
        }
        debug_assert_eq!(out.len(), out.capacity());
        self.entries = out.into_boxed_slice();
    }

    /// All entries.
    pub fn iter(&self) -> impl Iterator<Item = (u8, &MetaValue)> {
        self.entries.iter().map(|(i, v)| (*i, v))
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when nothing has been received.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Byte at `index` (`Byte` values only).
    #[must_use]
    pub fn byte(&self, index: u8) -> Option<i8> {
        match self.get(index)? {
            MetaValue::Byte(b) => Some(*b),
            _ => None,
        }
    }

    /// Float at `index`.
    #[must_use]
    pub fn float(&self, index: u8) -> Option<f32> {
        match self.get(index)? {
            MetaValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Item at `index` (`Some(None)` = an empty stack was sent).
    #[must_use]
    pub fn item(&self, index: u8) -> Option<Option<&Item>> {
        match self.get(index)? {
            MetaValue::Item(i) => Some(i.as_ref()),
            _ => None,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries = Box::default();
    }

    pub(crate) fn entries_mut(&mut self) -> impl Iterator<Item = &mut (u8, MetaValue)> {
        self.entries.iter_mut()
    }

    /// Heap bytes held (for the size tests).
    #[cfg(test)]
    pub(crate) fn heap_bytes(&self) -> usize {
        std::mem::size_of_val::<[(u8, MetaValue)]>(&self.entries)
    }
}

/// The rarely set parts of an entity (passengers, leash, effects), behind one pointer: most
/// entities never have any of them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Links {
    /// 26.x players: the profile name the entity was created with, once its tab-list entry is
    /// gone (vanilla's player keeps its `GameProfile`).
    pub(crate) profile_name: Option<Box<str>>,
    pub(crate) leash_holder: Option<i32>,
    pub(crate) passengers: Vec<i32>,
    pub(crate) effects: Vec<Effect>,
}

/// The protocol family an entity was tracked with (selects metadata indices and size rules).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Era {
    /// 1.8.x.
    #[default]
    V1_8,
    /// 1.9 – 1.12 (implemented: 1.12.2).
    V1_12,
    /// 1.16.5.
    V1_16,
    /// 26.1.
    V26,
    /// 26.2 (the same modern state as 26.1, with 26.2's ids and sizes).
    V26_2,
}

/// 1.16 registry ids used by [`Entity::class`].
const ITEM_1_16: u16 = 37;
const XP_ORB_1_16: u16 = 24;
const PAINTING_1_16: u16 = 55;

/// Number of equipment slots kept (1.16: main hand, off hand, feet, legs, chest, head;
/// 1.8: held, boots, leggings, chestplate, helmet in slots 0..=4).
pub const EQUIPMENT_SLOTS: usize = 6;

/// A tracked entity, as the vanilla client knows it from the server's packets.
#[derive(Clone, Debug)]
pub struct Entity {
    pub(crate) id: i32,
    pub(crate) uuid: Uuid,
    pub(crate) ty: EntityType,
    pub(crate) living: bool,
    /// The tracker's protocol family.
    pub(crate) era: Era,
    /// Vanilla size table entry (resolved at spawn).
    pub(crate) sizer: Option<&'static ghast_data::VanillaEntitySize>,
    pub(crate) pos: [f64; 3],
    /// Per version: 1.8 the server position in 1/32 blocks (`serverPos`, i32 values); 1.9 – 1.12
    /// the tracked position in 1/4096 blocks (`lfloor(x * 4096)`, plus moves); 26.1 the
    /// `VecDeltaCodec` base (`f64` bits).
    pub(crate) traced: [i64; 3],
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) head_yaw: f32,
    pub(crate) velocity: [f64; 3],
    pub(crate) on_ground: bool,
    pub(crate) data: i32,
    pub(crate) meta: Metadata,
    pub(crate) equipment: Option<Box<[Option<Item>; EQUIPMENT_SLOTS]>>,
    /// The vehicle's id (valid with `rides`; an `Option<i32>` would take 8 bytes here).
    pub(crate) vehicle_id: i32,
    pub(crate) rides: bool,
    /// Passengers, leash and effects (`None` = none of them).
    pub(crate) links: Option<Box<Links>>,
    /// 26.1 client state (codec base, interpolation, size keys), boxed: one pointer for the
    /// other versions.
    #[cfg(any(feature = "v26_1", feature = "v26_2"))]
    pub(crate) m: crate::modern_state::Ext,
}

impl Entity {
    pub(crate) fn new(id: i32, ty: EntityType) -> Self {
        Entity {
            id,
            uuid: Uuid::nil(),
            ty,
            living: matches!(ty, EntityType::Player | EntityType::LegacyMob(_)),
            era: Era::V1_8,
            sizer: None,
            pos: [0.0; 3],
            traced: [0; 3],
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            velocity: [0.0; 3],
            on_ground: false,
            data: 0,
            meta: Metadata::default(),
            equipment: None,
            links: None,
            vehicle_id: 0,
            rides: false,
            #[cfg(any(feature = "v26_1", feature = "v26_2"))]
            m: crate::modern_state::Ext::default(),
        }
    }

    /// The links, allocated on first use.
    #[inline]
    pub(crate) fn links_mut(&mut self) -> &mut Links {
        self.links.get_or_insert_with(Box::default)
    }

    /// Sets the vehicle (dismounting allocates nothing).
    #[inline]
    pub(crate) fn set_vehicle(&mut self, v: Option<i32>) {
        self.rides = v.is_some();
        self.vehicle_id = v.unwrap_or(0);
    }

    /// Takes the vehicle.
    #[inline]
    pub(crate) fn take_vehicle(&mut self) -> Option<i32> {
        let v = self.vehicle();
        self.set_vehicle(None);
        v
    }

    /// Sets the leash holder.
    #[inline]
    pub(crate) fn set_leash_holder(&mut self, h: Option<i32>) {
        match (&mut self.links, h) {
            (Some(l), h) => l.leash_holder = h,
            (None, None) => {}
            (None, Some(_)) => self.links_mut().leash_holder = h,
        }
        self.tidy_links();
    }

    /// Takes the passenger list (no allocation when there is none).
    #[inline]
    pub(crate) fn take_passengers(&mut self) -> Vec<i32> {
        let p = self
            .links
            .as_mut()
            .map(|l| std::mem::take(&mut l.passengers))
            .unwrap_or_default();
        self.tidy_links();
        p
    }

    /// Removes `id` from the passengers.
    #[inline]
    pub(crate) fn remove_passenger(&mut self, id: i32) {
        if let Some(l) = &mut self.links {
            l.passengers.retain(|&p| p != id);
        }
        self.tidy_links();
    }

    /// Replaces the passenger list.
    #[inline]
    pub(crate) fn set_passengers(&mut self, p: Vec<i32>) {
        if !p.is_empty() || self.links.is_some() {
            self.links_mut().passengers = p;
        }
        self.tidy_links();
    }

    /// Adds a passenger.
    #[inline]
    pub(crate) fn push_passenger(&mut self, id: i32) {
        self.links_mut().passengers.push(id);
    }

    /// Clears the effects.
    #[inline]
    pub(crate) fn clear_effects(&mut self) {
        if let Some(l) = &mut self.links {
            l.effects = Vec::new();
        }
        self.tidy_links();
    }

    /// Frees the links once they are all empty.
    #[inline]
    pub(crate) fn tidy_links(&mut self) {
        if self.links.as_ref().is_some_and(|l| {
            l.leash_holder.is_none()
                && l.profile_name.is_none()
                && l.passengers.is_empty()
                && l.effects.is_empty()
        }) {
            self.links = None;
        }
    }

    /// Heap bytes this entity holds (metadata block, equipment, links with their buffers, the
    /// modern state; for the size tests).
    #[cfg(test)]
    pub(crate) fn heap_bytes(&self) -> usize {
        let mut n = self.meta.heap_bytes();
        if self.equipment.is_some() {
            n += std::mem::size_of::<[Option<Item>; EQUIPMENT_SLOTS]>();
        }
        if let Some(l) = &self.links {
            n += std::mem::size_of::<Links>()
                + l.passengers.capacity() * 4
                + l.effects.capacity() * std::mem::size_of::<Effect>();
        }
        #[cfg(any(feature = "v26_1", feature = "v26_2"))]
        if self.m.0.is_some() {
            n += std::mem::size_of::<crate::modern_state::State>();
        }
        n
    }

    /// Network entity id.
    #[must_use]
    pub fn id(&self) -> i32 {
        self.id
    }

    /// UUID (nil for 1.8 entities other than players).
    #[must_use]
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Type.
    #[must_use]
    pub fn entity_type(&self) -> EntityType {
        self.ty
    }

    /// Version-independent class.
    #[must_use]
    pub fn class(&self) -> EntityClass {
        #[cfg(feature = "v26_1")]
        if self.era == Era::V26 {
            return crate::v26_1::class(self);
        }
        #[cfg(feature = "v26_2")]
        if self.era == Era::V26_2 {
            return crate::v26_2::class(self);
        }
        match self.ty {
            EntityType::Player => EntityClass::Player,
            EntityType::LegacyObject(2) | EntityType::Registry(ITEM_1_16) => EntityClass::Item,
            EntityType::ExperienceOrb | EntityType::Registry(XP_ORB_1_16) => {
                EntityClass::ExperienceOrb
            }
            EntityType::Painting | EntityType::Registry(PAINTING_1_16) => EntityClass::Painting,
            _ if self.living => EntityClass::Mob,
            _ => EntityClass::Object,
        }
    }

    /// The stack of an item entity (from metadata; `None` before the server sent it or when
    /// metadata is not tracked; 1.16 always keeps it for item entities). Always `None` on 26.1,
    /// whose stacks carry data components: see `v26_1::item_stack` / `v26_1::item_count`.
    #[must_use]
    pub fn item(&self) -> Option<&Item> {
        if matches!(self.era, Era::V26 | Era::V26_2) {
            return None;
        }
        let index = match self.ty {
            EntityType::LegacyObject(2) if self.era == Era::V1_12 => 6,
            EntityType::LegacyObject(2) => 10,
            EntityType::Registry(ITEM_1_16) => 7,
            _ => return None,
        };
        self.meta.item(index).flatten()
    }

    /// Vanilla client-side dimensions: the type's size from ghast-data (checked against the
    /// oracle), with the variant the client applies from metadata (baby, slime size, wither
    /// skeleton, elder guardian, small / marker armor stand, 1.16 player pose). `None` for
    /// hanging entities (paintings, item frames, leash knots), whose box comes from facing.
    ///
    /// Variants vanilla applies on the entity's next client tick are applied immediately, and
    /// the old-min-corner quirk after a resize is not modelled.
    #[must_use]
    pub fn dimensions(&self) -> Option<ghast_data::EntityDimensions> {
        let vs = self.sizer?;
        match self.era {
            #[cfg(feature = "v1_8")]
            Era::V1_8 => crate::v1_8::dimensions(self, vs),
            #[cfg(feature = "v1_12")]
            Era::V1_12 => crate::v1_12::dimensions(self, vs),
            #[cfg(feature = "v1_16_5")]
            Era::V1_16 => crate::v1_16_5::dimensions(self, vs),
            #[cfg(feature = "v26_1")]
            Era::V26 => {
                let _ = vs;
                crate::v26_1::dimensions(self)
            }
            #[cfg(feature = "v26_2")]
            Era::V26_2 => {
                let _ = vs;
                crate::v26_2::dimensions(self)
            }
            // A tracker only stamps eras of versions compiled in.
            #[allow(unreachable_patterns)]
            _ => None,
        }
    }

    /// How much the crosshair pick grows this entity's box (vanilla `getCollisionBorderSize` /
    /// `getTargetingMargin`, from the type's size table: 0.1 for most 1.8 types, 0 since 1.9,
    /// 1.0 for fireballs); 0 when the type has no size entry.
    #[must_use]
    pub fn targeting_margin(&self) -> f32 {
        self.sizer.map_or(0.0, |s| s.targeting_margin)
    }

    /// `(width, height)` of [`Entity::dimensions`].
    #[must_use]
    pub fn size(&self) -> Option<(f32, f32)> {
        self.dimensions().map(|d| (d.width, d.height))
    }

    /// Vanilla bounding box at the server position, `[min_x, min_y, min_z, max_x, max_y,
    /// max_z]` (bit-exact `EntityDimensions::bounding_box`).
    #[must_use]
    pub fn bounding_box(&self) -> Option<[f64; 6]> {
        let [x, y, z] = self.pos;
        let b = self.dimensions()?.bounding_box(x, y, z);
        Some([b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z])
    }

    /// `true` for players.
    #[must_use]
    pub fn is_player(&self) -> bool {
        self.ty == EntityType::Player
    }

    /// The **server position**: exactly the coordinates the vanilla client tracks from the
    /// packets (1.8: `serverPos / 32.0`; 1.12: spawn / teleport coordinates, after a relative
    /// move the tracked 1/4096 position; 1.16: the packet coordinates, rel moves applied in
    /// 1/4096 fixed point). The rendered position interpolates towards this over 3 ticks in
    /// vanilla; a bot does not need that.
    ///
    /// **26.1** is different: this is the entity's *current* position as the client holds it
    /// (`Entity.position()`): position packets of interpolated entities (living entities, boats,
    /// minecarts) set a target the entity reaches over 3 calls of `v26_1::Tracker::tick`, others
    /// move at once. That is the position vanilla's crosshair pick reads at tick time (with
    /// [`Entity::bounding_box`]); the packet coordinates are `v26_1::server_position`.
    #[must_use]
    pub fn position(&self) -> [f64; 3] {
        self.pos
    }

    /// 1.8 only: the raw server position in 1/32 blocks (`[0; 3]` in 1.12 and 1.16).
    #[must_use]
    pub fn fixed_position(&self) -> [i32; 3] {
        if self.era == Era::V1_8 {
            self.traced.map(|t| t as i32)
        } else {
            [0; 3]
        }
    }

    /// Body yaw in degrees as vanilla computes it from the angle byte (`b * 360 / 256f`).
    #[must_use]
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Pitch in degrees.
    #[must_use]
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// Head yaw in degrees (from `entity_head_rotation`; mobs also at spawn).
    #[must_use]
    pub fn head_yaw(&self) -> f32 {
        self.head_yaw
    }

    /// Last velocity the server sent (blocks per tick), from the spawn packet or
    /// `entity_velocity`. Not integrated over time.
    #[must_use]
    pub fn velocity(&self) -> [f64; 3] {
        self.velocity
    }

    /// On-ground flag of the last movement packet.
    #[must_use]
    pub fn on_ground(&self) -> bool {
        self.on_ground
    }

    /// Spawn data: object data (1.8 objects / 1.16 `spawn_entity`), experience orb value,
    /// painting direction.
    #[must_use]
    pub fn data(&self) -> i32 {
        self.data
    }

    /// Entity this one rides.
    #[must_use]
    pub fn vehicle(&self) -> Option<i32> {
        self.rides.then_some(self.vehicle_id)
    }

    /// Entities riding this one (1.8: at most one).
    #[must_use]
    pub fn passengers(&self) -> &[i32] {
        self.links.as_ref().map_or(&[], |l| &l.passengers)
    }

    /// Entity holding this one's leash.
    #[must_use]
    pub fn leash_holder(&self) -> Option<i32> {
        self.links.as_ref().and_then(|l| l.leash_holder)
    }

    /// Metadata (empty unless metadata tracking is enabled).
    #[must_use]
    pub fn metadata(&self) -> &Metadata {
        &self.meta
    }

    /// Equipment slot (see [`EQUIPMENT_SLOTS`]); `None` if never sent or empty.
    #[must_use]
    pub fn equipment(&self, slot: usize) -> Option<&Item> {
        self.equipment.as_ref()?.get(slot)?.as_ref()
    }

    /// Active effects as last reported.
    #[must_use]
    pub fn effects(&self) -> &[Effect] {
        self.links.as_ref().map_or(&[], |l| &l.effects)
    }

    /// The effect with this id, if active.
    #[must_use]
    pub fn effect(&self, id: u8) -> Option<&Effect> {
        self.effects().iter().find(|e| e.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `set_from` is `set` applied in order: overwrites, new indices appended once, the last
    /// duplicate wins, refused indices are skipped; the block is exact-size.
    #[test]
    fn metadata_set_from_matches_set() {
        let batches: [&[(u8, i8)]; 4] = [
            &[(0, 1), (9, 2), (0, 3)],
            &[(9, 4)],
            &[(5, 5), (2, 6), (5, 7), (9, 8)],
            &[(7, 9), (3, 10)],
        ];
        let keep = |i: u8| i != 3;
        let (mut a, mut b) = (Metadata::default(), Metadata::default());
        for batch in batches {
            let mut src: Vec<(u8, MetaValue)> = batch
                .iter()
                .map(|&(i, v)| (i, MetaValue::Byte(v)))
                .collect();
            for (i, v) in src.clone() {
                if keep(i) {
                    a.set(i, v);
                }
            }
            b.set_from(&mut src, keep);
            assert!(src.is_empty());
            assert_eq!(a, b);
        }
        let got: Vec<(u8, i8)> = b
            .iter()
            .map(|(i, v)| match v {
                MetaValue::Byte(x) => (i, *x),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(got, [(0, 3), (9, 8), (5, 7), (2, 6), (7, 9)]);
        assert_eq!(b.heap_bytes(), 5 * std::mem::size_of::<(u8, MetaValue)>());
        b.clear();
        assert!(b.is_empty() && b.heap_bytes() == 0);
    }

    #[test]
    fn links_are_freed_when_empty() {
        let mut e = Entity::new(1, EntityType::Player);
        e.set_vehicle(None);
        e.set_leash_holder(None);
        e.clear_effects();
        assert!(e.links.is_none());
        e.set_vehicle(Some(2));
        e.push_passenger(3);
        assert_eq!((e.vehicle(), e.passengers()), (Some(2), &[3][..]));
        e.set_vehicle(None);
        e.remove_passenger(3);
        assert!(e.links.is_none());
        assert_eq!(e.take_passengers(), Vec::<i32>::new());
    }
}
