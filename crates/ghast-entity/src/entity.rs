//! Tracked entity state.

use bytes::Bytes;
use uuid::Uuid;

/// What kind of entity this is, in the id space of the packet that spawned it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// 1.16 pose.
    Pose(i32),
}

/// The metadata of one entity: `(index, value)` pairs, at most one per index, in arrival order.
///
/// Scalar updates overwrite in place (no allocation).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metadata {
    entries: Vec<(u8, MetaValue)>,
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
            None => self.entries.push((index, value)),
        }
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
        self.entries.clear();
    }

    pub(crate) fn entries_mut(&mut self) -> impl Iterator<Item = &mut (u8, MetaValue)> {
        self.entries.iter_mut()
    }
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
    pub(crate) fixed: [i32; 3],
    /// 1.9 – 1.12: the tracked position in 1/4096 blocks (`lfloor(x * 4096)`, plus moves).
    pub(crate) traced: [i64; 3],
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) head_yaw: f32,
    pub(crate) velocity: [f64; 3],
    pub(crate) on_ground: bool,
    pub(crate) data: i32,
    pub(crate) vehicle: Option<i32>,
    pub(crate) passengers: Vec<i32>,
    pub(crate) leash_holder: Option<i32>,
    pub(crate) meta: Metadata,
    pub(crate) equipment: Option<Box<[Option<Item>; EQUIPMENT_SLOTS]>>,
    pub(crate) effects: Vec<Effect>,
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
            fixed: [0; 3],
            traced: [0; 3],
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            velocity: [0.0; 3],
            on_ground: false,
            data: 0,
            vehicle: None,
            passengers: Vec::new(),
            leash_holder: None,
            meta: Metadata::default(),
            equipment: None,
            effects: Vec::new(),
        }
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
    /// metadata is not tracked; 1.16 always keeps it for item entities).
    #[must_use]
    pub fn item(&self) -> Option<&Item> {
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
            Era::V1_8 => crate::v1_8::dimensions(self, vs),
            Era::V1_12 => crate::v1_12::dimensions(self, vs),
            Era::V1_16 => crate::v1_16_5::dimensions(self, vs),
        }
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
    #[must_use]
    pub fn position(&self) -> [f64; 3] {
        self.pos
    }

    /// 1.8 only: the raw server position in 1/32 blocks (`[0; 3]` in 1.12 and 1.16).
    #[must_use]
    pub fn fixed_position(&self) -> [i32; 3] {
        self.fixed
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
        self.vehicle
    }

    /// Entities riding this one (1.8: at most one).
    #[must_use]
    pub fn passengers(&self) -> &[i32] {
        &self.passengers
    }

    /// Entity holding this one's leash.
    #[must_use]
    pub fn leash_holder(&self) -> Option<i32> {
        self.leash_holder
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
        &self.effects
    }

    /// The effect with this id, if active.
    #[must_use]
    pub fn effect(&self, id: u8) -> Option<&Effect> {
        self.effects.iter().find(|e| e.id == id)
    }
}
