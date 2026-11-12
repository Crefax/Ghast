//! Minecraft 26.1 entity types: ids, names, the client instance's dimensions per variant and pose,
//! and the per-type facts an entity tracker needs (living, interpolation, pickable, the synched
//! data indices that select a size variant).
//!
//! Everything but display names and categories comes from the real 26.1 jar (oracle dump
//! `tests/golden/26.1/entity_sizes.jsonl.gz`; minecraft-data `pc/26.1` is cross-checked for ids and
//! names at generation time).
//!
//! Dimensions as the client computes them ([`EntityTypeData::dimensions`]): the variant's size
//! (`"default"`, `"baby"`, slime / magma cube size `n` = the type's size scaled by `n`, armor stand
//! `"small"` / `"marker"` / `"small_marker"`), the pose's override where one exists (camel sitting,
//! goat long jumping, player poses, ...), a living entity's sleeping pose is
//! [`LIVING_SLEEPING`], and a living entity's `minecraft:scale` attribute scales the result unless
//! the dimensions are fixed. Not modelled (the default size is returned): phantom size,
//! pufferfish puff state; no box at all for the types flagged [`type_flags::NO_BOX`].

use super::g;
use crate::common::{EntityDimensions, EntityInfo, VanillaEntitySize, lookup_name};

/// Bits of [`EntityTypeData::flags`].
pub mod type_flags {
    /// `instanceof LivingEntity` (has attributes, a head rotation, the sleeping pose).
    pub const LIVING: u8 = 1;
    /// `isPickable()` of a fresh client instance (players also need to be non-spectators,
    /// armor stands non-markers).
    pub const PICKABLE: u8 = 2;
    /// The client box is not the centred dimensions box (hanging entities: from facing; shulker:
    /// attach face and peek; interaction: width / height metadata; wind charges: lowered box).
    /// Not modelled.
    pub const NO_BOX: u8 = 4;
    /// The type's registered dimensions are fixed (not scaled for babies / the scale attribute).
    pub const FIXED: u8 = 8;
    /// `instanceof Mob`: the client turns its body and head with a `BodyRotationControl`.
    pub const MOB: u8 = 16;
    /// The mob uses the default `BodyRotationControl` (not the camel / creaking / happy ghast /
    /// phantom / shulker / armadillo ones).
    pub const DEFAULT_BODY: u8 = 32;
}

/// `LivingEntity.SLEEPING_DIMENSIONS`: every living entity in the sleeping pose (never scaled).
pub const LIVING_SLEEPING: EntityDimensions = EntityDimensions {
    width: 0.2,
    height: 0.2,
    eye_height: 0.2,
};

/// Network id of `minecraft:player`.
pub const PLAYER: u16 = 155;
/// Network id of `minecraft:item`.
pub const ITEM: u16 = 71;
/// Network id of `minecraft:experience_orb`.
pub const EXPERIENCE_ORB: u16 = 49;
/// Network id of `minecraft:painting`.
pub const PAINTING: u16 = 93;
/// Network id of `minecraft:armor_stand`.
pub const ARMOR_STAND: u16 = 5;

/// Per-type client facts.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct EntityTypeData {
    /// Network (registry) id.
    pub id: u16,
    /// Registry path (`"zombie"`).
    pub name: &'static str,
    /// [`type_flags`].
    pub flags: u8,
    /// Steps of the client instance's interpolation handler: position packets set a target the
    /// entity moves to over that many ticks (`Some(3)`: living entities, boats, minecarts,
    /// experience orbs, fishing bobbers; `Some(0)`: displays, which snap unless metadata sets a
    /// duration). `None`: position packets move the entity at once.
    pub interpolation: Option<u8>,
    /// Mobs: `getMaxHeadYRot()` (how far the head may turn from the body; 0 for non-mobs).
    pub max_head_y_rot: u8,
    /// Bit `1 << pose id` set when that pose's dimensions are fixed (`EntityDimensions.fixed`:
    /// not scaled by the scale attribute; a player dying, a digging warden, sleeping).
    pub fixed_poses: u32,
    /// Synched data index of the boolean baby flag.
    pub baby_meta: Option<u8>,
    /// Synched data index of the slime / magma cube size (int).
    pub size_meta: Option<u8>,
    /// Synched data index of the armor stand client flags (byte: 0x01 small, 0x10 marker).
    pub flags_meta: Option<u8>,
    /// Measured client sizes (`None`: no client instance, e.g. lightning).
    pub size: Option<&'static VanillaEntitySize>,
}

impl EntityTypeData {
    /// `instanceof LivingEntity`.
    #[must_use]
    pub fn is_living(&self) -> bool {
        self.flags & type_flags::LIVING != 0
    }

    /// `isPickable()` of a fresh client instance.
    #[must_use]
    pub fn is_pickable(&self) -> bool {
        self.flags & type_flags::PICKABLE != 0
    }

    /// The vanilla client dimensions for a variant (`"default"`, `"baby"`, `"small"`, `"marker"`,
    /// `"small_marker"`; slime sizes via `slime_size`) and pose id, before the scale attribute
    /// (see [`EntityTypeData::scaled`]). `None` for [`type_flags::NO_BOX`] types and types
    /// without a client instance.
    #[must_use]
    pub fn dimensions(
        &self,
        variant: &str,
        slime_size: Option<i32>,
        pose: i32,
    ) -> Option<EntityDimensions> {
        if self.flags & type_flags::NO_BOX != 0 {
            return None;
        }
        let s = self.size?;
        if pose == 2 && self.is_living() {
            return Some(LIVING_SLEEPING);
        }
        let pose_name = POSES.get(pose as usize).copied().unwrap_or("standing");
        if let Some(n) = slime_size {
            // Slime.getDefaultDimensions: the type's size (= the size-1 default) scaled by the size,
            // whatever the pose.
            return Some(scale(s.default_dimensions()?, n as f32, false));
        }
        if variant == "default" {
            if let Some(d) = s.pose(pose_name) {
                return Some(d);
            }
            return s.default_dimensions();
        }
        let mut key = [0u8; 40];
        let k = join(&mut key, variant, pose_name);
        if let Some(d) = k.and_then(|k| s.pose(k)) {
            return Some(d);
        }
        s.variant(variant)
            .map(|v| v.dimensions)
            .or_else(|| s.default_dimensions())
    }

    /// `EntityDimensions.scale(f)` of a living entity's dimensions by its scale attribute
    /// (`fixed` dimensions and `f == 1` stay as they are).
    #[must_use]
    pub fn scaled(d: EntityDimensions, f: f32, fixed: bool) -> EntityDimensions {
        scale(d, f, fixed)
    }
}

fn scale(d: EntityDimensions, f: f32, fixed: bool) -> EntityDimensions {
    if fixed || f == 1.0 {
        d
    } else {
        EntityDimensions {
            width: d.width * f,
            height: d.height * f,
            eye_height: d.eye_height * f,
        }
    }
}

fn join<'a>(buf: &'a mut [u8; 40], a: &str, b: &str) -> Option<&'a str> {
    let n = a.len() + 1 + b.len();
    if n > buf.len() {
        return None;
    }
    buf[..a.len()].copy_from_slice(a.as_bytes());
    buf[a.len()] = b'/';
    buf[a.len() + 1..n].copy_from_slice(b.as_bytes());
    core::str::from_utf8(&buf[..n]).ok()
}

/// `Pose` names by id (out-of-range ids read as standing, as vanilla's `BY_ID` does).
pub const POSES: [&str; 18] = [
    "standing",
    "fall_flying",
    "sleeping",
    "swimming",
    "spin_attack",
    "crouching",
    "long_jumping",
    "dying",
    "croaking",
    "using_tongue",
    "sitting",
    "roaring",
    "sniffing",
    "emerging",
    "digging",
    "sliding",
    "shooting",
    "inhaling",
];

/// All entity types by id.
#[must_use]
pub fn entities() -> &'static [EntityInfo] {
    &g::entities::ENTITIES
}

/// The entity type with network id `id`.
#[must_use]
pub fn entity(id: u16) -> Option<&'static EntityInfo> {
    g::entities::ENTITIES.get(usize::from(id))
}

/// Entity type by registry path (`"zombie"`).
#[must_use]
pub fn entity_by_name(name: &str) -> Option<&'static EntityInfo> {
    lookup_name(
        g::entities::ENTITY_NAMES_TEXT,
        &g::entities::ENTITY_NAMES,
        name,
    )
    .map(|i| &g::entities::ENTITIES[usize::from(i)])
}

/// The client facts of type `id` (O(1)).
#[inline]
#[must_use]
pub fn entity_type(id: u16) -> Option<&'static EntityTypeData> {
    g::entities::TYPES.get(usize::from(id))
}

/// All per-type facts by id.
#[must_use]
pub fn entity_types() -> &'static [EntityTypeData] {
    &g::entities::TYPES
}

/// Measured client sizes, sorted by name.
#[must_use]
pub fn vanilla_entity_sizes() -> &'static [VanillaEntitySize] {
    &g::entities::VANILLA_ENTITY_SIZES
}

/// Measured client sizes by registry path.
#[must_use]
pub fn vanilla_entity_size(name: &str) -> Option<&'static VanillaEntitySize> {
    g::entities::VANILLA_ENTITY_SIZES
        .binary_search_by(|e| e.name.cmp(name))
        .ok()
        .map(|i| &g::entities::VANILLA_ENTITY_SIZES[i])
}
