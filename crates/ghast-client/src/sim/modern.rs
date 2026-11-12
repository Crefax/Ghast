//! The world and movement of the modern versions (26.1+): the chunk store of `ghast-world` and
//! the modern physics (`ghast_physics::modern`, bit-exact against each version's client),
//! written once and mounted per version by `sim/v26_x.rs` (the parent module, `super`), which
//! supplies the version's crates (`data`, `world`, `entity`, `inv`, `proto`, `pick`), its
//! physics rule set (`Rules`) and its tables. Entity type ids are resolved by name from the
//! version's data set ([`EntityIds`]).

use super::Rules;
use super::data::BlockState;
use ghast_physics::modern::{self, AttributeKind, ModifierId};
use ghast_protocol::{BlockPos, RawPacket};

use crate::common::{Abilities, AttributeUpdate, Controls, Outbound, Pose, Teleport, Vec3};
use crate::hands::{BlockQuery, HeldItem};
use crate::sim::*;

/// A modern version's chunk store and vanilla-exact modern physics.
pub(crate) struct ModernSim {
    world: super::world::World,
    player: modern::PlayerState,
    packets: ghast_physics::Packets,
    /// Block changes since the last drain (position, state id), when collected.
    updates: BlockUpdates,
    /// `mining_efficiency`, `block_break_speed`, `submerged_mining_speed` (digging).
    mining: [modern::Attribute; 3],
    /// Conduit power amplifier (digging).
    conduit_power: Option<i32>,
    /// `block_interaction_range`, `entity_interaction_range` (the crosshair).
    ranges: [modern::Attribute; 2],
    /// `Entity.xo/yo/zo`: the position at the start of the last tick (the pick's eye lerps
    /// from it).
    old_pos: [f64; 3],
    /// The entity tracker, when entities are tracked.
    entities: Option<super::entity::Tracker>,
    /// Join / respawn packets, replayed into a tracker started mid-session.
    session: SessionPackets,
    /// The level-load gate (`player_loaded`), oracle-proven on `joins.jsonl.gz`.
    gate: modern::LoadGate,
    /// `use_effects` of the item in use (see [`ModernSim::use_effects`]).
    use_effects: Option<(bool, f32)>,
    /// Movement state beyond `player`: gliding, attached rockets (and later mechanics).
    ext: modern::PlayerExt,
    /// What the client knows about entities that drive `ext` (own id, rockets).
    own: OwnEntity,
    /// `Level.rainLevel` from the weather game events (1 START_RAINING, 2 STOP_RAINING,
    /// 7 RAIN_LEVEL_CHANGE); the level rains above 0.2.
    rain_level: f32,
    /// The extension state can act this tick (gliding, a glider to start a glide, rockets, the
    /// riptide spin or counter, a launch made before the tick): set wherever `ext` gains such a
    /// state, recomputed after each extended tick. One flag keeps the plain tick's path lean.
    ext_active: bool,
    /// Riptide strength of the trident in the main / off hand (`held_use_inputs`).
    riptide: [f32; 2],
    /// With entity tracking: the boxes the player collides with this tick and the entities
    /// that push it after its tick (buffers kept between ticks).
    around: Around,
    /// With entity tracking: the scoreboard teams' collision rules (`set_player_team`).
    teams: Option<Box<crate::sim::teams::Teams>>,
    /// The player's scoreboard name (its team membership).
    profile_name: Box<str>,
    /// The vehicle the player rides and controls (`set_passengers` naming us).
    riding: Option<Riding>,
    /// Without entity tracking: a tracker of the vehicles only (boats and minecarts the player
    /// may ride), fed their spawn, move and passenger packets.
    vehicles: Option<Box<super::entity::Tracker>>,
    /// The boat the player left: it stays where the player drove it (the client moved it; the
    /// server sends no move for it) until the server moves it.
    parked: Option<Parked>,
    /// The answer to a server packet that goes out at once (`move_vehicle`).
    reply: Option<Outbound>,
    /// 26.2+: a movement attribute of `ext` (friction / air drag modifier, bounciness) is not
    /// at its default, so the extended tick runs (the plain tick reads the defaults).
    movement_attributes: bool,
    /// The packets the sim reads changed (boats appeared / are gone): rebuild the filter.
    filter_dirty: bool,
}

/// A vehicle the player rides.
#[derive(Debug)]
enum Riding {
    /// A boat or raft: client-authoritative (`modern::tick_boat`).
    Boat { id: i32, boat: Box<modern::Boat> },
    /// A minecart: the server moves it, the player sits on it (`modern::tick_passenger`).
    Seat { id: i32 },
}

impl Riding {
    fn id(&self) -> i32 {
        match self {
            Riding::Boat { id, .. } | Riding::Seat { id } => *id,
        }
    }
}

/// A boat the player drove and left.
#[derive(Debug, Clone, Copy)]
struct Parked {
    id: i32,
    pos: [f64; 3],
    bb: [f64; 6],
    /// The tracker's position of it when the player left (a different one: the server moved it).
    tracked: Option<[f64; 3]>,
}

impl Parked {
    /// The parked position and box when `e` is the parked boat and the server has not moved it.
    fn of(this: Option<&Parked>, e: &ghast_entity::Entity) -> Option<([f64; 3], [f64; 6])> {
        let p = this?;
        (p.id == e.id() && p.tracked == Some(e.position())).then_some((p.pos, p.bb))
    }
}

/// The vehicle kinds the player can ride with modelled physics.
fn vehicle_kind(name: &str) -> Option<VehicleKind> {
    if let Some(k) = modern::BoatKind::from_entity_name(name) {
        Some(VehicleKind::Boat(k))
    } else if name.ends_with("minecart") {
        Some(VehicleKind::Cart)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
enum VehicleKind {
    Boat(modern::BoatKind),
    Cart,
}

/// Entity collision and pushing around the player (only with entity tracking).
#[derive(Debug, Default)]
struct Around {
    /// Collidable entity boxes (`canBeCollidedWith`), in `getEntities` section order.
    boxes: Vec<ghast_physics::math::Aabb>,
    /// Pushers of this tick: order key, push.
    pushers: Vec<(i64, modern::Pusher)>,
    /// The player ticked since the last entity tick (pushes come after its tick).
    player_ticked: bool,
    /// The entities near the player that tick before it in the client's tick list (ticked
    /// early: their boxes are post-tick, their pushes come before the player's tick).
    pre: Vec<i32>,
}

/// Per entity type: the player collides with it (the version's `collides`).
const COLLIDES: u8 = 1;
/// Per entity type: its tick pushes the player like a living entity.
const PUSH_LIVING: u8 = 2;
/// Per entity type: its tick pushes the player like a boat.
const PUSH_BOAT: u8 = 4;

/// [`COLLIDES`] / [`PUSH_LIVING`] / [`PUSH_BOAT`] by entity type id.
fn entity_bits(e: &ghast_entity::Entity) -> u8 {
    static BITS: std::sync::OnceLock<Box<[u8]>> = std::sync::OnceLock::new();
    let bits = BITS.get_or_init(|| {
        let types = super::data::entity_types();
        let mut v = vec![
            0u8;
            types
                .iter()
                .map(|t| usize::from(t.id) + 1)
                .max()
                .unwrap_or(0)
        ];
        for t in types {
            let mut b = 0;
            if super::collides(t.name) {
                b |= COLLIDES;
            }
            match super::push_kind(t.name, t.is_living()) {
                Some(modern::PushKind::Living) => b |= PUSH_LIVING,
                Some(modern::PushKind::Boat) => b |= PUSH_BOAT,
                _ => {}
            }
            v[usize::from(t.id)] = b;
        }
        v.into_boxed_slice()
    });
    super::entity::entity_type(e).map_or(0, |t| bits.get(usize::from(t.id)).copied().unwrap_or(0))
}

/// The boxes the player may collide with this tick (`canBeCollidedWith`: boats, rafts, living
/// shulkers; its own vehicle excluded), and while riding the boxes its boat collides with
/// (`canVehicleCollide`: those and every living entity, not the player), near enough for any
/// move of this tick, each in `getEntities` section order. Only with entity tracking.
#[inline(never)]
fn collect_colliders(
    t: Option<&super::entity::Tracker>,
    parked: Option<&Parked>,
    p: &modern::PlayerState,
    vehicle: Option<i32>,
    boxes: &mut Vec<ghast_physics::math::Aabb>,
    boat_boxes: &mut Vec<ghast_physics::math::Aabb>,
) {
    boxes.clear();
    boat_boxes.clear();
    // Farther than any move of one tick can reach (the physics filters exactly); a boat moves
    // up to ~2 blocks a tick on ice.
    let extra = if vehicle.is_some() { 4.0 } else { 3.0 };
    let reach = |v: f64| v.abs() + extra;
    let (rx, ry, rz) = (reach(p.vel[0]), reach(p.vel[1]), reach(p.vel[2]));
    let mut found: Vec<((i32, u64), [f64; 6], bool)> = Vec::new();
    // The boat the player left, where it drove it (without the tracker it is the only one).
    if let Some(k) = parked
        && t.is_none_or(|t| t.get(k.id).is_none())
    {
        found.push((section_order(k.pos), k.bb, true));
    }
    for e in t.into_iter().flat_map(|t| t.iter()) {
        let bits = entity_bits(e);
        let living =
            vehicle.is_some() && super::entity::entity_type(e).is_some_and(|t| t.is_living());
        if bits & COLLIDES == 0 && !living {
            continue;
        }
        if Some(e.id()) == vehicle {
            continue;
        }
        let Some((pos, b)) = Parked::of(parked, e).or_else(|| entity_box(e)) else {
            continue;
        };
        if b[3] < p.bb.min_x - rx
            || b[0] > p.bb.max_x + rx
            || b[4] < p.bb.min_y - ry
            || b[1] > p.bb.max_y + ry
            || b[5] < p.bb.min_z - rz
            || b[2] > p.bb.max_z + rz
        {
            continue;
        }
        // isAlive: a dying shulker (health 0) no longer collides.
        if super::entity::health(e).is_some_and(|h| h <= 0.0) {
            continue;
        }
        found.push((section_order(pos), b, bits & COLLIDES != 0));
    }
    // getEntities walks sections by x, then z / y.
    found.sort_by_key(|f| f.0);
    for (_, b, collides) in found {
        if collides {
            boxes.push(aabb(b));
        }
        if vehicle.is_some() {
            boat_boxes.push(aabb(b));
        }
    }
}

/// `Level.noCollision` without the entity part, for a tracked entity's moved interpolation
/// target: free of blocks and the border.
fn block_free(
    world: &super::world::World,
    level: &modern::LevelInfo,
    e: &ghast_entity::Entity,
    bb: [f64; 6],
) -> bool {
    let pos = e.position();
    modern::no_block_collision::<Rules, _>(
        world,
        level,
        pos,
        bb,
        &modern::CollisionContext {
            feet_y: pos[1],
            ..Default::default()
        },
    )
}

/// `SectionPos` of a position, as `getEntities` orders sections: x, then z, then y.
fn section_order(pos: [f64; 3]) -> (i32, u64) {
    let b = |v: f64| (v.floor() as i32) >> 4;
    let (x, y, z) = (b(pos[0]), b(pos[1]), b(pos[2]));
    (
        x,
        (u64::from(z as u32 & 0x3F_FFFF) << 20) | u64::from(y as u32 & 0xF_FFFF),
    )
}

/// Position and box of an entity, with the shulker's own rules where the tracker has no box (its
/// size follows the peek): `Shulker.setPos` snaps to the block (`floor(x) + 0.5`,
/// `floor(y + 0.5)`, `floor(z) + 0.5`) and the closed shell is the unit cube there
/// (`Shulker.makeBoundingBox` at peek 0, scale 1; an open shell, taller toward its attach face,
/// and a scaled shulker are not modelled).
fn entity_box(e: &ghast_entity::Entity) -> Option<([f64; 3], [f64; 6])> {
    if let Some(b) = e.bounding_box() {
        return Some((e.position(), b));
    }
    let t = super::entity::entity_type(e)?;
    (t.name == "shulker" && e.vehicle().is_none()).then(|| {
        let [x, y, z] = e.position();
        let (x, y, z) = (x.floor() + 0.5, (y + 0.5).floor(), z.floor() + 0.5);
        ([x, y, z], [x - 0.5, y, z - 0.5, x + 0.5, y + 1.0, z + 0.5])
    })
}

fn aabb(b: [f64; 6]) -> ghast_physics::math::Aabb {
    ghast_physics::math::Aabb::new(b[0], b[1], b[2], b[3], b[4], b[5])
}

/// The entity packets the modern movement reads without entity tracking: the player's own
/// shared flags and main arm, and firework rockets attached to it. Only those are decoded: a
/// packet is decoded past its entity id only when the id is ours or a rocket's.
#[derive(Debug, Default)]
struct OwnEntity {
    /// Our entity id (from the login packet).
    id: i32,
    /// The main arm per our own entity data (`DATA_PLAYER_MAIN_HAND`), else the client option.
    main_arm_left: bool,
    /// Firework rockets spawned but not (yet) known as attached to us: id, spawn position.
    rockets: Vec<(i32, [f64; 3])>,
    /// Mountable animals with an inventory screen (horses, llamas, nautili …): what their
    /// screen reads off them (`canUseSlot(SADDLE / BODY)`).
    mounts: Vec<Mount>,
}

/// A mount entity's screen facts.
#[derive(Debug, Clone, Copy)]
struct Mount {
    id: i32,
    /// `minecraft:entity_type` registry id.
    ty: i32,
    /// `AgeableMob.DATA_BABY_ID`.
    baby: bool,
    /// Index 18: `AbstractHorse.DATA_ID_FLAGS` (tamed = 2) or `TamableAnimal.DATA_FLAGS_ID`
    /// (tame = 4).
    flags: u8,
}

/// The entity type ids the modern movement reads, resolved by registry name from the version's
/// data set (each version numbers its entity types itself).
#[derive(Debug, Clone, Copy)]
struct EntityIds {
    /// Entity types whose screen is a mount screen: horse, donkey, mule, skeleton / zombie
    /// horse, camel, camel husk (`AbstractHorse`).
    horses: [i32; 7],
    /// llama, trader llama (`Llama`).
    llamas: [i32; 2],
    /// nautilus, zombie nautilus (`AbstractNautilus`).
    nautili: [i32; 2],
    /// `minecraft:firework_rocket`.
    firework_rocket: i32,
}

impl EntityIds {
    /// The ids of this version (`-1`: a type the version does not have).
    fn get() -> &'static EntityIds {
        static IDS: std::sync::OnceLock<EntityIds> = std::sync::OnceLock::new();
        IDS.get_or_init(|| {
            let id = |name: &str| super::data::entity_by_name(name).map_or(-1, |t| i32::from(t.id));
            EntityIds {
                horses: [
                    "horse",
                    "donkey",
                    "mule",
                    "skeleton_horse",
                    "zombie_horse",
                    "camel",
                    "camel_husk",
                ]
                .map(id),
                llamas: ["llama", "trader_llama"].map(id),
                nautili: ["nautilus", "zombie_nautilus"].map(id),
                firework_rocket: id("firework_rocket"),
            }
        })
    }

    #[cfg(test)]
    fn tuple(&self) -> ([i32; 7], [i32; 2], [i32; 2], i32) {
        (self.horses, self.llamas, self.nautili, self.firework_rocket)
    }

    fn is_mount(&self, ty: i32) -> bool {
        self.horses.contains(&ty) || self.llamas.contains(&ty) || self.nautili.contains(&ty)
    }
}

/// Mounts remembered at most (nearest first is not known: the oldest are dropped).
const MAX_MOUNTS: usize = 128;

impl Mount {
    /// `canUseSlot(SADDLE)`, `canUseSlot(BODY)` (alive: the entity is still in the level).
    fn slots(&self, ids: &EntityIds) -> (bool, bool) {
        if ids.llamas.contains(&self.ty) {
            (true, true)
        } else if ids.nautili.contains(&self.ty) {
            let ok = !self.baby && self.flags & 4 != 0;
            (ok, ok)
        } else {
            (!self.baby && self.flags & 2 != 0, true)
        }
    }
}

// Entity data indices of the version (read from its real classes).
use super::meta::{
    ATTACHED_TO_TARGET as DATA_ATTACHED_TO_TARGET, BABY as DATA_BABY,
    LIVING_FLAGS as DATA_LIVING_FLAGS, MOUNT_FLAGS as DATA_MOUNT_FLAGS,
    PLAYER_MAIN_HAND as DATA_PLAYER_MAIN_HAND, SHARED_FLAGS as DATA_SHARED_FLAGS,
};
/// Rockets spawned near us that may still get attached (a bound against a flood of rockets).
const MAX_PENDING_ROCKETS: usize = 32;

/// The interaction ranges with vanilla's defaults and ranges.
fn range_attributes() -> [modern::Attribute; 2] {
    [
        modern::Attribute::new(4.5, 0.0, 64.0),
        modern::Attribute::new(3.0, 0.0, 64.0),
    ]
}

/// The mining attributes with vanilla's defaults and ranges.
fn mining_attributes() -> [modern::Attribute; 3] {
    [
        modern::Attribute::new(0.0, 0.0, 1024.0),
        modern::Attribute::new(1.0, 0.0, 1024.0),
        modern::Attribute::new(0.2, 0.0, 20.0),
    ]
}

impl std::fmt::Debug for ModernSim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModernSim")
            .field("world", &self.world)
            .field("pos", &self.player.pos)
            .finish_non_exhaustive()
    }
}

/// `Player.canGlide` equipment part: some equipment slot holds a stack that can glide using that
/// slot (`LivingEntity.canGlideUsing`: the `glider` component, `equippable` naming that slot,
/// the next point of damage will not break it).
fn glider_equipped(inv: &ghast_inventory::Inventory) -> bool {
    use super::inv::{Equip, can_glide_using};
    let slots = [
        (inv.held(), Equip::MainHand),
        (inv.offhand(), Equip::Offhand),
        (inv.armor(0), Equip::Head),
        (inv.armor(1), Equip::Chest),
        (inv.armor(2), Equip::Legs),
        (inv.armor(3), Equip::Feet),
    ];
    slots
        .into_iter()
        .any(|(s, slot)| s.is_some_and(|s| can_glide_using(s, slot)))
}

impl ModernSim {
    /// Something in `ext` acts next tick (see [`ModernSim::ext_active`]).
    #[inline]
    fn ext_wanted(&self) -> bool {
        let x = &self.ext;
        x.fall_flying
            || x.glider
            || x.rocket_count() > 0
            || x.spin_attack
            || x.auto_spin_ticks > 0
            || self.movement_attributes
    }

    /// `set_entity_data`, `add_entity`, `remove_entities` for our own entity and attached
    /// rockets (everything else is read only up to the entity id, or the entity type).
    fn own_entity_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        use super::proto::play::clientbound as cb;
        use super::proto::types::EntityMetadataEntryValue as V;
        let mut r = ghast_protocol::Reader::new(&raw.body);
        match raw.id {
            cb::id::ENTITY_METADATA => {
                let id = r.read_varint()?;
                let rocket = self.own.rockets.iter().position(|&(rid, _)| rid == id);
                let mount = self.own.mounts.iter().position(|m| m.id == id);
                if id != self.own.id && rocket.is_none() && mount.is_none() {
                    return Ok(());
                }
                let p: cb::EntityMetadata = raw.decode_as()?;
                for e in &p.metadata {
                    match (&e.value, e.key) {
                        (V::Byte(b), DATA_SHARED_FLAGS) if id == self.own.id => {
                            self.ext.apply_shared_flags(&mut self.player, *b as u8);
                            self.ext_active |= self.ext.fall_flying;
                        }
                        (V::Byte(b), DATA_LIVING_FLAGS) if id == self.own.id => {
                            self.ext.apply_living_flags(*b as u8);
                            self.ext_active |= self.ext.spin_attack;
                        }
                        (V::HumanoidArm(a), DATA_PLAYER_MAIN_HAND) if id == self.own.id => {
                            // HumanoidArm: LEFT 0, RIGHT 1.
                            self.own.main_arm_left = a.0 == 0;
                        }
                        (V::Boolean(b), DATA_BABY) if mount.is_some() => {
                            self.own.mounts[mount.unwrap_or(0)].baby = *b;
                        }
                        (V::Byte(f), DATA_MOUNT_FLAGS) if mount.is_some() => {
                            self.own.mounts[mount.unwrap_or(0)].flags = *f as u8;
                        }
                        (V::OptionalUnsignedInt(v), DATA_ATTACHED_TO_TARGET) => {
                            if let Some(k) = rocket
                                && *v - 1 == self.own.id
                            {
                                let (rid, pos) = self.own.rockets.swap_remove(k);
                                let ok = self.ext.attach_rocket(rid, pos);
                                self.ext_active |= ok;
                                log::debug!("firework rocket {rid} attached to us ({ok})");
                            }
                        }
                        _ => {}
                    }
                }
            }
            cb::id::SPAWN_ENTITY => {
                let id = r.read_varint()?;
                r.skip(16)?; // uuid
                let ty = r.read_varint()?;
                // Vehicles the player may ride (the full tracker knows them when it runs).
                if self.entities.is_none()
                    && u16::try_from(ty)
                        .ok()
                        .and_then(super::data::entity_type)
                        .and_then(|t| vehicle_kind(t.name))
                        .is_some()
                {
                    self.vehicle_tracker_mut().apply(raw)?;
                    return Ok(());
                }
                let ids = EntityIds::get();
                if ids.is_mount(ty) {
                    self.own.mounts.retain(|m| m.id != id);
                    if self.own.mounts.len() >= MAX_MOUNTS {
                        self.own.mounts.remove(0);
                    }
                    self.own.mounts.push(Mount {
                        id,
                        ty,
                        baby: false,
                        flags: 0,
                    });
                    return Ok(());
                }
                if ty != ids.firework_rocket {
                    return Ok(());
                }
                let p: cb::SpawnEntity = raw.decode_as()?;
                if self.own.rockets.len() >= MAX_PENDING_ROCKETS {
                    self.own.rockets.remove(0);
                }
                self.own.rockets.push((id, [p.x, p.y, p.z]));
            }
            cb::id::ENTITY_DESTROY => {
                if self.own.rockets.is_empty()
                    && self.ext.rocket_count() == 0
                    && self.own.mounts.is_empty()
                    && self.vehicles.is_none()
                    && self.riding.is_none()
                {
                    return Ok(());
                }
                let p: cb::EntityDestroy = raw.decode_as()?;
                if let Some(v) = &mut self.vehicles {
                    v.apply(raw)?;
                }
                for id in p.entity_ids {
                    self.own.rockets.retain(|&(rid, _)| rid != id);
                    self.own.mounts.retain(|m| m.id != id);
                    self.ext.remove_rocket(id);
                    // The vehicle is gone: its passenger is off (removePassenger).
                    if self.riding.as_ref().is_some_and(|v| v.id() == id) {
                        self.riding = None;
                    }
                    if self.parked.is_some_and(|p| p.id == id) {
                        self.parked = None;
                    }
                }
                if self.vehicles.as_ref().is_some_and(|v| v.is_empty()) {
                    self.vehicles = None;
                    self.filter_dirty = true;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The vehicles-only tracker, started on the first vehicle seen without entity tracking
    /// (from the session's join packets and the loaded columns, like a tracker started late).
    fn vehicle_tracker_mut(&mut self) -> &mut super::entity::Tracker {
        if self.vehicles.is_none() {
            let mut t = super::entity::Tracker::new(
                ghast_entity::Config::default()
                    .with_metadata(false)
                    .with_equipment(false)
                    .with_effects(false)
                    .with_events(false),
            );
            self.session.replay(|r| {
                let _ = t.apply(r);
            });
            for (c, _) in self.world.columns().iter() {
                t.mark_column_loaded(c.x, c.z);
            }
            self.vehicles = Some(Box::new(t));
            self.filter_dirty = true;
        }
        self.vehicles.as_mut().unwrap()
    }

    /// `ClientLevel.tickEntities` up to the player: the entities near it that tick before it
    /// (before its root vehicle when riding) tick now, and their pushes act before its tick.
    #[inline(never)]
    fn tick_before_player(&mut self, pose: &mut Pose) {
        let Some(t) = &mut self.entities else { return };
        let me = self.riding.as_ref().map_or(self.own.id, Riding::id);
        let Some(me_seq) = t.tick_seq(me) else { return };
        let p = &self.player;
        let reach = |v: f64| v.abs() + 4.0;
        let (rx, ry, rz) = (reach(p.vel[0]), reach(p.vel[1]), reach(p.vel[2]));
        let pre = &mut self.around.pre;
        pre.clear();
        let mut found: Vec<(u64, i32)> = Vec::new();
        for e in t.iter() {
            let [x, y, z] = e.position();
            if (x - p.pos[0]).abs() > rx + 2.0
                || (y - p.pos[1]).abs() > ry + 2.0
                || (z - p.pos[2]).abs() > rz + 2.0
            {
                continue;
            }
            if entity_bits(e) & (COLLIDES | PUSH_LIVING | PUSH_BOAT) == 0 && self.riding.is_none() {
                continue;
            }
            let Some(seq) = t.tick_seq(e.id()).filter(|&q| q < me_seq) else {
                continue;
            };
            found.push((seq, e.id()));
        }
        if found.is_empty() {
            return;
        }
        found.sort_unstable();
        let (world, level) = (&self.world, &self.player.level);
        for &(_, id) in &found {
            t.tick_early(id, |e, bb| block_free(world, level, e, bb));
            pre.push(id);
        }
        if self.riding.is_none() {
            self.push_player(pose, true);
        }
    }

    /// The parked boat ends when the server moved it (its tracked position changed) or it is
    /// gone.
    fn check_parked(&mut self) {
        if let Some(k) = self.parked {
            let now = self
                .vehicle_tracker()
                .and_then(|t| t.get(k.id))
                .map(|e| e.position());
            if now != k.tracked {
                self.parked = None;
            }
        }
    }

    /// The tracker that knows the vehicles: the full one, else the vehicles-only one.
    fn vehicle_tracker(&self) -> Option<&super::entity::Tracker> {
        self.entities.as_ref().or(self.vehicles.as_deref())
    }

    /// `set_passengers` and `move_vehicle` (the server's vehicle correction).
    fn vehicle_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        use super::proto::play::clientbound as cb;
        if raw.id == cb::id::VEHICLE_MOVE {
            // handleMoveVehicle: only for the vehicle this client controls; answered at once.
            if let Some(Riding::Boat { boat, .. }) = &mut self.riding {
                let p: cb::VehicleMove = raw.decode_as()?;
                if let ghast_physics::Packet::MoveVehicle {
                    pos,
                    yaw,
                    pitch,
                    on_ground,
                } = boat.server_move([p.x, p.y, p.z], p.yaw, p.pitch)
                {
                    self.reply = Some(Outbound::VehicleMove {
                        pos: Vec3::new(pos[0], pos[1], pos[2]),
                        yaw,
                        pitch,
                        on_ground,
                    });
                }
            }
            return Ok(());
        }
        if let Some(v) = &mut self.vehicles {
            v.apply(raw)?;
        }
        let mut r = ghast_protocol::Reader::new(&raw.body);
        let vehicle = r.read_varint()?;
        let riding_it = self.riding.as_ref().is_some_and(|v| v.id() == vehicle);
        let Some(t) = self.vehicle_tracker() else {
            return Ok(());
        };
        if !riding_it && t.get(vehicle).is_none() {
            return Ok(());
        }
        let p: cb::SetPassengers = raw.decode_as()?;
        let on = p.passengers.contains(&self.own.id);
        if on && !riding_it {
            // startRiding, on a vehicle the client knows (position and rotation as it holds
            // them). A vehicle without modelled physics (living mounts) keeps the walking
            // player, as before.
            let ride = t.get(vehicle).and_then(|e| {
                let kind = super::entity::entity_type(e).and_then(|t| vehicle_kind(t.name))?;
                Some(match kind {
                    VehicleKind::Boat(k) => Riding::Boat {
                        id: vehicle,
                        boat: Box::new(modern::Boat::new(k, e.position(), e.yaw(), e.pitch())),
                    },
                    VehicleKind::Cart => Riding::Seat { id: vehicle },
                })
            });
            if let Some(ride) = ride {
                self.parked = None;
                match &ride {
                    Riding::Boat { boat, .. } => boat.board(&mut self.player),
                    Riding::Seat { .. } => modern::Seat::board(&mut self.player),
                }
                log::debug!("riding {vehicle}");
                self.riding = Some(ride);
            }
        } else if !on && riding_it {
            // Off the vehicle (the server teleports the player afterwards).
            log::debug!("left vehicle {vehicle}");
            if let Some(Riding::Boat { id, boat }) = self.riding.take() {
                let b = boat.bounding_box();
                let pos = boat.pos();
                self.parked = Some(Parked {
                    id,
                    pos,
                    bb: [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z],
                    tracked: self
                        .vehicle_tracker()
                        .and_then(|t| t.get(id))
                        .map(|e| e.position()),
                });
            }
            // The extended tick runs once more (what the ride left in `ext` is read there).
            self.ext_active = true;
        }
        Ok(())
    }

    /// The `use_effects` of the stack in use (`LivingEntity.getUseItem`), set by the engine
    /// each tick from `super::inv::item_use`; the component default without one.
    fn use_effects(&self) -> modern::UseEffects {
        self.use_effects.map_or_else(
            modern::UseEffects::default,
            |(can_sprint, speed_multiplier)| modern::UseEffects {
                can_sprint,
                speed_multiplier,
            },
        )
    }

    #[cfg(all(test, feature = "v1_8", feature = "v1_16_5"))]
    pub(crate) fn use_effects_now(&self) -> Option<(bool, f32)> {
        self.use_effects
    }

    /// The inventory-derived inputs of `ext` (see [`Sim::pre_tick`]): kept out of line, they
    /// matter only on jump presses and while rockets are attached.
    #[cold]
    #[inline(never)]
    fn ext_inputs(&mut self, inv: &ghast_inventory::Inventory, controls: Controls) {
        // The glider is read only when a glide starts (a jump press in the air).
        if controls.jump {
            self.ext.glider = glider_equipped(inv);
            self.ext_active |= self.ext.glider;
        }
        // Entity.getHandHoldingItemAngle(FIREWORK_ROCKET): the main arm, or the other one when
        // only the off hand holds a rocket. Read only while rockets are attached.
        if self.ext.rocket_count() > 0 {
            let rocket = |s: Option<&ghast_inventory::Stack>| {
                s.is_some_and(|s| super::inv::item_name(s.id()) == Some("firework_rocket"))
            };
            let off_only = rocket(inv.offhand()) && !rocket(inv.held());
            self.ext.rocket_arm_left = self.own.main_arm_left != off_only;
        }
    }

    /// `isInWaterOrRain` with the level's rain (biome precipitation is not known: every biome
    /// counts as rainy).
    /// `Entity.isInWaterOrRain`. `Level.isRainingAt` per position (feet and head): rain level
    /// above 0.2, the biome rains there (`Biome.getPrecipitationAt`; an unknown biome counts as
    /// raining: biomes are kept only once a riptide trident was held, or with
    /// `WorldConfig::biomes`), and nothing above blocks motion or holds fluid (the sky check
    /// of the physics; sky light itself is not modelled).
    fn wet(&self) -> bool {
        let p = &self.player;
        let (bx, bz) = (p.pos[0].floor() as i32, p.pos[2].floor() as i32);
        let view = self.world.view(bx, bz);
        let raining = self.rain_level > 0.2;
        if p.touching_water || !raining {
            return modern::in_water_or_rain::<Rules, _>(p, &view, raining);
        }
        let rains = |y: f64| {
            self.world
                .precipitation_at(bx, y.floor() as i32, bz)
                .is_none_or(|r| r == ghast_world::climate::Precipitation::Rain)
        };
        // in_water_or_rain looks at the feet and the head block: keep only the raining ones.
        let mut q = *p;
        match (rains(p.pos[1]), rains(p.bb.max_y)) {
            (false, false) => return false,
            (true, true) => {}
            (true, false) => q.bb.max_y = q.pos[1],
            (false, true) => q.pos[1] = q.bb.max_y,
        }
        modern::in_water_or_rain::<Rules, _>(&q, &view, true)
    }

    /// `releaseUsingItem` → `TridentItem.releaseUsing` with riptide: the launch, before the tick,
    /// along the tick's look (the mouse turned before the key handling).
    fn release_use(
        &mut self,
        inv: &ghast_inventory::Inventory,
        (off, id, ticks): (bool, u16, u32),
        look: (f32, f32),
    ) {
        let stack = if off { inv.offhand() } else { inv.held() };
        let Some(s) = stack.filter(|s| i64::from(s.id()) == i64::from(id)) else {
            return;
        };
        let strength = self.riptide[usize::from(off)];
        if ticks < 10 || strength <= 0.0 || super::inv::next_damage_will_break(s) {
            return;
        }
        let (yaw, pitch) = (self.player.yaw, self.player.pitch);
        self.player.yaw = look.0;
        self.player.pitch = look.1;
        if !self.wet() {
            self.player.yaw = yaw;
            self.player.pitch = pitch;
            return;
        }
        let p = &mut self.player;
        let view = self
            .world
            .view(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
        modern::riptide::<Rules, _>(p, &mut self.ext, &view, strength);
        self.ext_active = true;
    }

    /// The pushes of the entities that ticked after the player (`LivingEntity.pushEntities`,
    /// `AbstractBoat.tick`), each with its box after its own tick.
    #[inline(never)]
    fn push_player(&mut self, pose: &mut Pose, before: bool) {
        let p = &self.player;
        let a = &mut self.around;
        a.pushers.clear();
        let parked = self.parked.as_ref();
        // The boat the player left pushes from where it is (without the tracker, only it).
        if let Some(k) = parked
            && !before
            && self.entities.as_ref().is_none_or(|t| t.get(k.id).is_none())
        {
            a.pushers.push((
                i64::from(k.id),
                modern::Pusher::new(k.pos, aabb(k.bb), modern::PushKind::Boat),
            ));
        }
        for e in self.entities.iter().flat_map(|t| t.iter()) {
            let t = self.entities.as_ref().unwrap();
            // Those that tick before the player pushed it before its tick.
            if before != (!a.pre.is_empty() && a.pre.contains(&e.id())) {
                continue;
            }
            let bits = entity_bits(e);
            if bits & (PUSH_LIVING | PUSH_BOAT) == 0 {
                continue;
            }
            let Some((pos, b)) = Parked::of(parked, e).or_else(|| entity_box(e)) else {
                continue;
            };
            // Boats reach 0.2 farther; nothing else can touch the player.
            let m = 0.25;
            if b[3] < p.bb.min_x - m
                || b[0] > p.bb.max_x + m
                || b[4] < p.bb.min_y - m
                || b[1] > p.bb.max_y + m
                || b[5] < p.bb.min_z - m
                || b[2] > p.bb.max_z + m
            {
                continue;
            }
            // Only ticking entities push (their column is loaded).
            if !t.is_ticking(e) {
                continue;
            }
            // EntitySelector.pushableBy: the pusher's and the player's team collision rules.
            if let Some(teams) = self.teams.as_deref().filter(|t| t.any_rule()) {
                let mut buf = [0u8; 36];
                let name = match t.player_name(e.id()) {
                    Some(n) if e.is_player() => n,
                    _ => e.uuid().hyphenated().encode_lower(&mut buf),
                };
                if !teams.may_push(name, &self.profile_name) {
                    continue;
                }
            }
            let kind = if bits & PUSH_BOAT != 0 {
                modern::PushKind::Boat
            } else {
                modern::PushKind::Living
            };
            // In the client's tick order.
            let seq = t.tick_seq(e.id()).map_or(i64::MAX, |q| q as i64);
            a.pushers
                .push((seq, modern::Pusher::new(pos, aabb(b), kind)));
        }
        if a.pushers.is_empty() {
            return;
        }
        a.pushers.sort_by_key(|x| x.0);
        let p = &mut self.player;
        let view = self
            .world
            .view(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
        for (_, pusher) in &a.pushers {
            modern::push_by::<Rules, _>(p, &self.ext, &view, pusher);
        }
        pose.motion = Vec3::new(p.vel[0], p.vel[1], p.vel[2]);
    }

    pub(crate) fn new() -> Self {
        ModernSim {
            world: super::world::World::new(config()),
            player: modern::PlayerState::new([0.0; 3], 0.0, 0.0),
            packets: ghast_physics::Packets::new(),
            updates: BlockUpdates::default(),
            mining: mining_attributes(),
            conduit_power: None,
            ranges: range_attributes(),
            old_pos: [0.0; 3],
            entities: None,
            use_effects: None,
            ext: modern::PlayerExt::default(),
            own: OwnEntity::default(),
            rain_level: 0.0,
            ext_active: false,
            riptide: [0.0; 2],
            around: Around::default(),
            teams: None,
            profile_name: "".into(),
            riding: None,
            vehicles: None,
            parked: None,
            reply: None,
            movement_attributes: false,
            filter_dirty: false,
            session: SessionPackets::default(),
            gate: modern::LoadGate::waiting(),
        }
    }

    /// A sim with options (world contents, entity tracking). Recording and the Grim step
    /// compatibility are not available on the modern versions yet.
    pub(crate) fn with(opts: &SimOptions) -> Self {
        let mut s = Self::new();
        if let Some(c) = opts.world {
            s.world = super::world::World::new(c);
        }
        if let Some(p) = opts.record.as_deref() {
            log::warn!(
                "movement recording is not supported on {}; not recording to {}",
                super::GAME,
                p.display()
            );
        }
        s.entities = opts.entities.map(super::entity::Tracker::new);
        s.teams = s.entities.is_some().then(Box::default);
        if let Some(n) = &opts.profile_name {
            s.profile_name = n.as_str().into();
        }
        if opts.force_step_position != ForceStepPosition::Off {
            log::warn!("force_step_position has no effect on {}", super::GAME);
        }
        s
    }

    /// Test hook: a world in the vanilla nether (registries as with known packs).
    #[cfg(all(test, feature = "v1_8", feature = "v1_12_2", feature = "v1_16_5"))]
    pub(crate) fn test_nether() -> Self {
        let mut s = Self::new();
        s.world.apply_registry(
            "minecraft:dimension_type",
            [
                ("minecraft:overworld", None),
                ("minecraft:the_nether", None),
            ]
            .into_iter(),
        );
        s.world.apply_registry(
            "minecraft:worldgen/biome",
            [("minecraft:plains", None)].into_iter(),
        );
        s.world.enter_dimension("minecraft:the_nether", 1, 8);
        s.sync_level();
        s
    }

    /// Test hook: a world in the vanilla overworld (registries as with known packs).
    #[cfg(test)]
    pub(crate) fn test_overworld() -> Self {
        let mut s = Self::new();
        s.world.apply_registry(
            "minecraft:dimension_type",
            [("minecraft:overworld", None)].into_iter(),
        );
        s.world.apply_registry(
            "minecraft:worldgen/biome",
            [("minecraft:plains", None)].into_iter(),
        );
        s.world.enter_dimension("minecraft:overworld", 0, 8);
        s.sync_level();
        s
    }

    /// The level facts movement reads, from the world's dimension.
    fn sync_level(&mut self) {
        if let Some(d) = self.world.dimension() {
            self.player.level = modern::LevelInfo {
                min_y: d.kind.min_y,
                height: d.kind.height,
                fast_lava: d.kind.fast_lava,
            };
        }
    }

    fn state(&self, x: i32, y: i32, z: i32) -> Option<BlockState> {
        self.world.block_state(x, y, z)
    }
}

/// The union of a shape's boxes as a ray box (block-local).
fn outline_box(s: BlockState) -> Option<[f64; 6]> {
    let shape = s.outline_shape();
    if shape.empty {
        return None;
    }
    union(shape.boxes)
}

impl WorldView for ModernSim {
    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.world.is_column_loaded(chunk_x, chunk_z)
    }

    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.state(x, y, z).map(|s| u32::from(s.id()))
    }

    fn entities(&self) -> Option<&ghast_entity::Entities> {
        self.entities.as_deref()
    }

    fn loaded_columns(&self) -> usize {
        self.world.columns().len()
    }

    fn build_height(&self) -> Option<(i32, i32)> {
        self.world
            .dimension()
            .map(|d| (d.kind.min_y, d.kind.height))
    }

    fn block_query(&self, x: i32, y: i32, z: i32) -> Option<BlockQuery> {
        let s = self.state(x, y, z)?;
        let d = s.block().data();
        let hardness = if d.diggable {
            d.hardness.unwrap_or(-1.0)
        } else {
            -1.0
        };
        Some(BlockQuery {
            exact: Some(super::exact_dig(crate::hands::ModernDig {
                state: s.id(),
                mining_efficiency: self.mining[0].value(),
                block_break_speed: self.mining[1].value(),
                submerged_mining_speed: self.mining[2].value(),
                conduit_power: self.conduit_power,
            })),
            air: s.is_air(),
            water: d.name == "water",
            block: s.block().default_state().id(),
            hardness,
            // The modern destroy progress replaces tool groups and harvest lists.
            dig_material: None,
            harvest_tools: &[],
            selection: outline_box(s),
            stairs: None,
            activates: activates_1_16(d.name),
        })
    }

    fn crosshair(
        &self,
        eyes: Vec3,
        (yaw, pitch): (f32, f32),
        _look: Vec3,
        reach: f64,
    ) -> Option<crate::BlockHit> {
        use super::pick;
        // The modern look vector (`calculateViewVector`, double-index sine table).
        let view = pick::view_vector(yaw, pitch);
        let eye = [eyes.x, eyes.y, eyes.z];
        // Survival reach on both rays: the entity range (3) is shorter, the block range wins.
        let hit = pick::pick(&self.world, eye, view, reach, 3.0, None);
        block_hit(&hit, eye)
    }
}

impl Sim for ModernSim {
    fn crosshair_now(
        &self,
        _eyes: Vec3,
        (yaw, pitch): (f32, f32),
        _look: Vec3,
        held: Option<u16>,
    ) -> Option<crate::BlockHit> {
        use super::pick;
        let p = &self.player;
        // Minecraft.pick at partial tick 1: the eye lerps from the old position.
        let eye = pick::eye_position(p.pos, self.old_pos, p.dims.eye_height);
        let held_block = held
            .and_then(super::data::Item::from_id)
            .and_then(super::data::Item::block);
        let hit = pick::pick(
            &self.world,
            eye,
            pick::view_vector(yaw, pitch),
            self.ranges[0].value(),
            self.ranges[1].value(),
            held_block,
        );
        block_hit(&hit, eye)
    }

    fn world_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        use super::proto::play::clientbound as cb;
        self.session.note(raw, cb::id::LOGIN, cb::id::RESPAWN);
        match raw.id {
            cb::id::ENTITY_METADATA | cb::id::SPAWN_ENTITY | cb::id::ENTITY_DESTROY => {
                self.own_entity_packet(raw)?;
            }
            cb::id::SET_PASSENGERS | cb::id::VEHICLE_MOVE => self.vehicle_packet(raw)?,
            cb::id::ENTITY_TELEPORT
            | cb::id::SYNC_ENTITY_POSITION
            | cb::id::REL_ENTITY_MOVE
            | cb::id::ENTITY_MOVE_LOOK
            | cb::id::ENTITY_LOOK
                if self.vehicles.is_some() =>
            {
                if let Some(v) = &mut self.vehicles {
                    v.apply(raw)?;
                }
            }
            cb::id::RESPAWN => {
                self.riding = None;
                self.parked = None;
            }
            cb::id::LOGIN => {
                self.riding = None;
                self.parked = None;
                if self.vehicles.take().is_some() {
                    self.filter_dirty = true;
                }
                self.own.id = raw.decode_as::<cb::Login>()?.entity_id;
                // A new play phase: a new client scoreboard.
                if let Some(t) = &mut self.teams {
                    t.clear();
                }
                self.own.rockets.clear();
                self.own.mounts.clear();
            }
            _ => {}
        }
        if raw.id == cb::id::TEAMS {
            if let Some(t) = &mut self.teams
                && !t.apply(&raw.body, super::TEAM_LAYOUT)
            {
                log::debug!("bad set_player_team packet ({} B)", raw.body.len());
            }
            return Ok(());
        }
        // The tracker also reads map_chunk / unload_chunk (which columns tick): the world gets
        // the same packets below.
        if let Some(t) = &mut self.entities
            && t.wants(raw.id)
        {
            t.apply(raw)?;
        }
        // The vehicles-only tracker follows the level and its columns (which entities tick).
        if let Some(v) = &mut self.vehicles
            && matches!(
                raw.id,
                cb::id::LOGIN | cb::id::RESPAWN | cb::id::MAP_CHUNK | cb::id::UNLOAD_CHUNK
            )
        {
            v.apply(raw)?;
        }
        match raw.id {
            // Batch timing needs the arrival time: the engine routes these (Inbound).
            cb::id::CHUNK_BATCH_START | cb::id::CHUNK_BATCH_FINISHED => return Ok(()),
            cb::id::BLOCK_CHANGE if self.updates.on => {
                let p: cb::BlockChange = raw.decode_as()?;
                self.updates.push(p.location, p.r#type as u32);
                self.world.apply_block_change(&p);
                return Ok(());
            }
            cb::id::MULTI_BLOCK_CHANGE if self.updates.on => {
                let p: cb::MultiBlockChange = raw.decode_as()?;
                let c = &p.chunk_coordinates;
                let sy = ((c.y << 12) as i32) >> 12;
                for &r in &p.records {
                    let local = (r & 0xFFF) as i32;
                    let pos = BlockPos::new(
                        c.x * 16 + ((local >> 8) & 15),
                        sy.wrapping_mul(16).wrapping_add(local & 15),
                        c.z * 16 + ((local >> 4) & 15),
                    );
                    self.updates.push(pos, ((r as u64) >> 12) as u32);
                }
                self.world.apply_multi_block_change(&p);
                return Ok(());
            }
            _ => {}
        }
        // Decode only this world packet: the packet-set enum's decode_raw (behind
        // World::handle_raw) would link every decoder of the version into the binary.
        use super::proto::play::clientbound::ClientboundPlayPacket as P;
        use ghast_world::WorldError;
        let d = |r: Result<P, ghast_protocol::DecodeError>| r.map_err(WorldError::Decode);
        let applied = if raw.id == cb::id::MAP_CHUNK {
            self.world.apply_map_chunk_body(&raw.body)
        } else {
            let p = match raw.id {
                cb::id::LOGIN => d(raw.decode_as().map(P::Login))?,
                cb::id::RESPAWN => d(raw.decode_as().map(P::Respawn))?,
                cb::id::UNLOAD_CHUNK => d(raw.decode_as().map(P::UnloadChunk))?,
                cb::id::BLOCK_CHANGE => d(raw.decode_as().map(P::BlockChange))?,
                cb::id::MULTI_BLOCK_CHANGE => d(raw.decode_as().map(P::MultiBlockChange))?,
                cb::id::CHUNK_BIOMES => d(raw.decode_as().map(P::ChunkBiomes))?,
                cb::id::UPDATE_VIEW_POSITION => d(raw.decode_as().map(P::UpdateViewPosition))?,
                cb::id::UPDATE_VIEW_DISTANCE => d(raw.decode_as().map(P::UpdateViewDistance))?,
                cb::id::INITIALIZE_WORLD_BORDER => {
                    d(raw.decode_as().map(P::InitializeWorldBorder))?
                }
                cb::id::WORLD_BORDER_CENTER => d(raw.decode_as().map(P::WorldBorderCenter))?,
                cb::id::WORLD_BORDER_LERP_SIZE => d(raw.decode_as().map(P::WorldBorderLerpSize))?,
                cb::id::WORLD_BORDER_SIZE => d(raw.decode_as().map(P::WorldBorderSize))?,
                cb::id::WORLD_BORDER_WARNING_DELAY => {
                    d(raw.decode_as().map(P::WorldBorderWarningDelay))?
                }
                cb::id::WORLD_BORDER_WARNING_REACH => {
                    d(raw.decode_as().map(P::WorldBorderWarningReach))?
                }
                cb::id::TILE_ENTITY_DATA => d(raw.decode_as().map(P::TileEntityData))?,
                _ => return Ok(()),
            };
            self.world.handle(&p)
        };
        if matches!(raw.id, cb::id::LOGIN | cb::id::RESPAWN) {
            self.sync_level();
        }
        applied?;
        Ok(())
    }

    fn registry_data(&mut self, raw: &RawPacket) {
        match raw.decode_as::<super::proto::configuration::clientbound::RegistryData>() {
            Ok(p) => {
                self.world.apply_registry_data(&p);
            }
            Err(e) => log::warn!("bad registry_data packet: {e}"),
        }
    }

    fn modern_inputs(&self) -> bool {
        true
    }

    fn held_use_inputs(&mut self, inv: &mut ghast_inventory::Inventory) {
        // EnchantmentHelper.getTridentSpinAttackStrength through the synced registry (None:
        // not computable -> no riptide). Only tridents are looked at.
        let trident = |s: Option<&ghast_inventory::Stack>| {
            s.filter(|s| super::inv::item_name(s.id()) == Some("trident"))
                .cloned()
        };
        let hands = [trident(inv.held()), trident(inv.offhand())];
        for (k, s) in hands.into_iter().enumerate() {
            self.riptide[k] = s
                .and_then(|s| super::inv_trident_strength(inv, &s))
                .unwrap_or(0.0);
        }
        // Riptide works in rain only where the biome rains: keep biomes from now on.
        if self.riptide.iter().any(|&r| r > 0.0) && !self.world.config().biomes {
            self.world.set_biomes(true);
        }
    }

    fn pre_tick_wanted(&self) -> bool {
        self.ext.rocket_count() > 0
    }

    fn item_use_facts(
        &self,
        stack: &ghast_inventory::Stack,
        off_hand: bool,
        mut held: HeldItem,
    ) -> HeldItem {
        // getUseDuration > 0; food only below a full bar unless can_always_eat.
        if let Some(u) = super::inv::item_use(stack) {
            held.usable = u.duration.is_none_or(|d| d > 0);
            held.food = u.food.is_some();
            held.always_edible = u.food.is_some_and(|f| f.can_always_eat);
        }
        // FireworkRocketItem.use: SUCCESS (the server spawns the boosting rocket) only while
        // gliding; otherwise PASS.
        // TridentItem.use: with riptide only in water or rain (else FAIL: no use state).
        if self.riptide[usize::from(off_hand)] > 0.0
            && super::inv::item_name(stack.id()) == Some("trident")
        {
            held.usable = held.usable && self.wet();
        }
        held.swing_on_use =
            self.ext.fall_flying && super::inv::item_name(stack.id()) == Some("firework_rocket");
        held
    }

    fn pre_tick(&mut self, inv: &ghast_inventory::Inventory, t: &crate::sim::PreTick) -> bool {
        if t.controls.jump || self.ext.rocket_count() > 0 {
            self.ext_inputs(inv, t.controls);
        }
        if let Some(released) = t.released {
            self.release_use(inv, released, t.look);
        }
        self.use_effects = None;
        let Some((off, id)) = t.using else {
            return true;
        };
        // updatingUsingItem: the used hand must still hold the same item (useItem follows it).
        let s = if off { inv.offhand() } else { inv.held() };
        let Some(s) = s.filter(|s| i64::from(s.id()) == i64::from(id)) else {
            return false;
        };
        self.use_effects = super::inv::item_use(s).map(|u| (u.can_sprint, u.speed_multiplier));
        true
    }

    fn gliding(&self) -> bool {
        self.ext.fall_flying
    }

    fn weather(&mut self, reason: u8, value: f32) {
        // ClientPacketListener.handleGameEvent: START_RAINING sets the rain level to 0 (the
        // server then raises it with RAIN_LEVEL_CHANGE), STOP_RAINING to 1, RAIN_LEVEL_CHANGE
        // to the value.
        self.rain_level = match reason {
            1 => 0.0,
            2 => 1.0,
            _ => value,
        };
    }

    fn mount_info(&self, entity_id: i32) -> Option<(i32, bool, bool)> {
        let m = self.own.mounts.iter().find(|m| m.id == entity_id)?;
        let (saddle, body) = m.slots(EntityIds::get());
        Some((m.ty, saddle, body))
    }

    #[cfg(all(test, feature = "v1_8", feature = "v1_16_5"))]
    fn test_set_modern(&mut self, p: modern::PlayerState, x: modern::PlayerExt) {
        let level = self.player.level;
        self.player = p;
        self.player.level = level;
        self.ext = x;
        self.ext_active = true;
        self.old_pos = p.pos;
        self.gate = modern::LoadGate::loaded();
    }

    #[cfg(all(test, feature = "v1_8", feature = "v1_16_5"))]
    fn test_shared_flags(&self) -> u8 {
        self.ext.shared_flags(&self.player)
    }

    #[cfg(all(test, feature = "v1_8", feature = "v1_16_5"))]
    fn test_set_own_id(&mut self, id: i32) {
        self.own.id = id;
    }

    fn set_main_arm_left(&mut self, left: bool) {
        self.own.main_arm_left = left;
    }

    fn drop_level(&mut self) {
        // ClientPacketListener.handleConfigurationStart → clearClientLevel: no level, no
        // entities until the next login (which enters a new level from the new registries).
        self.world.clear();
        self.session = SessionPackets::default();
        self.updates.list.clear();
        self.updates.dropped = 0;
        if let Some(t) = &mut self.entities {
            *t = super::entity::Tracker::new(t.config());
        }
        self.reset_player();
    }

    fn reset_player(&mut self) {
        // A fresh LocalPlayer; abilities and food are sent again by the server, the level
        // stays (attributes are re-sent too).
        let abilities = self.player.abilities;
        let food = self.player.food_level;
        let level = self.player.level;
        self.player = modern::PlayerState::new([0.0; 3], 0.0, 0.0);
        // A new LocalPlayer: not gliding, no rockets attached, default attributes.
        self.ext = modern::PlayerExt::default();
        self.ext_active = false;
        self.movement_attributes = false;
        self.player.abilities = abilities;
        self.player.food_level = food;
        self.player.level = level;
        self.mining = mining_attributes();
        self.ranges = range_attributes();
        self.old_pos = [0.0; 3];
        // handleLogin / handleRespawn: wait for the level again.
        self.gate = modern::LoadGate::waiting();
        self.sync_level();
    }

    fn respawn_player(&mut self, keep: u8) {
        // ClientPacketListener.handleRespawn: KEEP_ATTRIBUTE_MODIFIERS (1) keeps the attributes,
        // KEEP_ENTITY_DATA (2) the last sent input and sprinting.
        let old = self.player;
        let old_mining = self.mining;
        let old_ranges = self.ranges;
        let old_moves = (self.ext.attributes, self.movement_attributes);
        self.reset_player();
        if keep & 1 != 0 {
            self.player.attributes = old.attributes;
            (self.ext.attributes, self.movement_attributes) = old_moves;
            self.ext_active |= self.movement_attributes;
            self.mining = old_mining;
            self.ranges = old_ranges;
        }
        if keep & 2 != 0 {
            self.player.sent.input = old.sent.input;
            self.player.sent.sprinting = old.sent.sprinting;
        }
    }

    fn teleport(&mut self, pose: &mut Pose, t: &Teleport) {
        // The engine keeps the look (its rotation may have moved it since the last tick).
        self.player.yaw = pose.yaw;
        self.player.pitch = pose.pitch;
        // handleMovePlayer: the old position follows the teleport (relative axes keep their
        // offset from it).
        let before = self.player.pos;
        let _reply = self.player.apply_teleport(
            [t.x, t.y, t.z],
            [t.delta.x, t.delta.y, t.delta.z],
            t.yaw,
            t.pitch,
            t.relative,
        );
        let now = self.player.pos;
        for (i, old) in self.old_pos.iter_mut().enumerate() {
            *old = if t.relative & (1 << i) != 0 {
                *old + (now[i] - before[i])
            } else {
                now[i]
            };
        }
        sync_pose_26(pose, &self.player);
    }

    fn abilities(&mut self, a: Abilities) {
        self.player.apply_abilities(physics_abilities(a));
    }

    fn food(&mut self, food: i32) {
        self.player.food_level = food;
    }

    fn velocity(&mut self, v: Vec3, add: bool) {
        let p = &mut self.player;
        if add {
            p.apply_velocity([p.vel[0] + v.x, p.vel[1] + v.y, p.vel[2] + v.z]);
        } else {
            p.apply_velocity([v.x, v.y, v.z]);
        }
    }

    fn movement_speed(&mut self, base: f64, modifiers: &[(u128, f64, u8)]) {
        // Modern attributes arrive as [`Sim::attributes`].
        let _ = (base, modifiers);
    }

    fn attributes(&mut self, attributes: &[AttributeUpdate]) {
        for a in attributes {
            let mods: Vec<(ModifierId, f64, u8)> = a
                .modifiers
                .iter()
                .map(|(id, amount, op)| (ModifierId::new(id), *amount, *op as u8))
                .collect();
            let mining = match a.name {
                "mining_efficiency" => Some(0),
                "block_break_speed" => Some(1),
                "submerged_mining_speed" => Some(2),
                _ => None,
            };
            let range = match a.name {
                "block_interaction_range" => Some(0),
                "entity_interaction_range" => Some(1),
                _ => None,
            };
            if let Some(m) = self.ext.attributes.get_mut(a.name) {
                m.apply_packet(a.base, &mods);
                let d = modern::MovementAttributes::default();
                let x = &self.ext.attributes;
                self.movement_attributes = x.friction_modifier.value()
                    != d.friction_modifier.value()
                    || x.air_drag_modifier.value() != d.air_drag_modifier.value()
                    || x.bounciness.value() != d.bounciness.value();
                self.ext_active |= self.movement_attributes;
            } else if let Some(i) = mining {
                self.mining[i].apply_packet(a.base, &mods);
            } else if let Some(i) = range {
                self.ranges[i].apply_packet(a.base, &mods);
            } else if let Some(kind) = AttributeKind::from_name(a.name) {
                self.player.apply_attribute(kind, a.base, &mods);
            }
        }
    }

    fn status_effects(&mut self, effects: &std::collections::BTreeMap<i8, i8>) {
        // The engine keeps classic 1-based effect numbers (the version's `minecraft:mob_effect`
        // registry id + 1).
        use super::effects as fx;
        let e = &mut self.player.effects;
        e.jump_boost = effects.get(&fx::JUMP_BOOST).map(|a| i32::from(*a));
        e.blindness = effects.contains_key(&fx::BLINDNESS);
        e.levitation = effects.get(&fx::LEVITATION).map(|a| i32::from(*a));
        e.slow_falling = effects.contains_key(&fx::SLOW_FALLING);
        e.dolphins_grace = effects.contains_key(&fx::DOLPHINS_GRACE);
        e.weaving = effects.contains_key(&fx::WEAVING);
        self.conduit_power = effects.get(&fx::CONDUIT_POWER).map(|a| i32::from(*a));
    }

    fn boots(&mut self, depth_strider: u8, soul_speed: u8) {
        // Depth strider and soul speed arrive as attributes.
        let _ = (depth_strider, soul_speed);
    }

    fn feet_item(&mut self, id: Option<i32>) {
        let leather = super::data::Item::from_name("leather_boots");
        self.player.leather_boots = id
            .and_then(|i| u16::try_from(i).ok())
            .and_then(super::data::Item::from_id)
            == leather;
    }

    fn spectator(&mut self, on: bool) {
        self.player.spectator = on;
    }

    fn modern_player(&self) -> Option<&modern::PlayerState> {
        Some(&self.player)
    }

    fn set_flying(&mut self, flying: bool, out: &mut Vec<Outbound>) {
        let a = &mut self.player.abilities;
        if a.flying == flying || (flying && !a.allow_flying) {
            return;
        }
        a.flying = flying;
        let a = *a;
        out.push(Outbound::Abilities {
            flags: abilities_flags(&a),
            flying_speed: a.fly_speed,
            walking_speed: a.walk_speed,
        });
    }

    fn collect_block_updates(&mut self, on: bool) {
        self.updates.on = on;
        if !on {
            self.updates.list.clear();
        }
    }

    fn drain_block_updates(&mut self, out: &mut Vec<(BlockPos, u32)>) -> usize {
        out.append(&mut self.updates.list);
        std::mem::take(&mut self.updates.dropped)
    }

    fn world_packet_ids(&self) -> Vec<i32> {
        use super::proto::play::clientbound as cb;
        // Our own entity data and attached rockets (gliding), decoded only when they concern us.
        let own = [
            cb::id::ENTITY_METADATA,
            cb::id::SPAWN_ENTITY,
            cb::id::ENTITY_DESTROY,
            // The vehicle the player rides.
            cb::id::SET_PASSENGERS,
            cb::id::VEHICLE_MOVE,
        ];
        // Moves of the boats the player may board, only while some are known without the
        // tracker (entity moves are frequent: not decoded otherwise).
        let boats: &[i32] = if self.entities.is_none() && self.vehicles.is_some() {
            &[
                cb::id::ENTITY_TELEPORT,
                cb::id::SYNC_ENTITY_POSITION,
                cb::id::REL_ENTITY_MOVE,
                cb::id::ENTITY_MOVE_LOOK,
                cb::id::ENTITY_LOOK,
            ]
        } else {
            &[]
        };
        self.world
            .packet_ids()
            .chain(own)
            .chain(boats.iter().copied())
            .collect()
    }

    fn take_reply(&mut self) -> Option<Outbound> {
        self.reply.take()
    }

    fn take_filter_dirty(&mut self) -> bool {
        std::mem::take(&mut self.filter_dirty)
    }

    fn vehicle(&self) -> Option<crate::common::Vehicle> {
        match self.riding.as_ref()? {
            Riding::Seat { id } => {
                let e = self.vehicle_tracker()?.get(*id)?;
                let [x, y, z] = e.position();
                Some(crate::common::Vehicle {
                    entity_id: *id,
                    pos: Vec3::new(x, y, z),
                    yaw: e.yaw(),
                    pitch: e.pitch(),
                    on_ground: e.on_ground(),
                    controlled: false,
                })
            }
            Riding::Boat { id, boat } => {
                let [x, y, z] = boat.pos();
                Some(crate::common::Vehicle {
                    entity_id: *id,
                    pos: Vec3::new(x, y, z),
                    yaw: boat.yaw(),
                    pitch: boat.pitch(),
                    on_ground: boat.on_ground(),
                    controlled: true,
                })
            }
        }
    }

    fn entity_config(&self) -> Option<ghast_entity::Config> {
        self.entities.as_ref().map(|t| t.config())
    }

    fn set_entities(&mut self, config: Option<ghast_entity::Config>) {
        if self.entity_config() == config {
            return;
        }
        self.entities = config.map(|c| {
            let mut t = super::entity::Tracker::new(c);
            self.session.replay(|r| {
                if let Err(e) = t.apply(r) {
                    log::debug!("entity tracker: session packet 0x{:02x}: {e}", r.id);
                }
            });
            // The columns already loaded tick their entities.
            for (c, _) in self.world.columns().iter() {
                t.mark_column_loaded(c.x, c.z);
            }
            t
        });
        // Teams sent before tracking started are not known (all pushes then allowed).
        self.teams = self.entities.is_some().then(Box::default);
        // The tracker knows the boats from now on (it starts empty: boats spawned before are
        // known again once the server resends them).
        if self.entities.is_some() && self.vehicles.take().is_some() {
            self.filter_dirty = true;
        }
    }

    fn entity_packet_ids(&self) -> Vec<i32> {
        self.entities
            .as_ref()
            .map(|t| {
                t.packet_ids()
                    .chain([super::proto::play::clientbound::id::TEAMS])
                    .collect()
            })
            .unwrap_or_default()
    }

    fn pick_margin(&self, e: &ghast_entity::Entity) -> Option<f32> {
        let t = self.entities.as_ref()?;
        t.pickable(e).then(|| super::entity::pick_radius(e))
    }

    fn level_load_start(&mut self) {
        self.gate.level_chunks_load_start();
    }

    fn player_alive(&mut self, alive: bool) {
        self.gate.player_alive(alive);
    }

    fn load_gate_tick(&mut self, section_compiled: bool, timed_out: bool) -> bool {
        if timed_out {
            self.gate.time_out();
        }
        self.gate.tick(&self.player, section_compiled)
    }

    fn tick_unloaded(&mut self) {
        // LoadGate::step before the gate: only the entity tick counter.
        self.player.age += 1;
    }

    fn tick_entities(&mut self, pose: &mut Pose) {
        // ClientLevel.tickEntities; a moved interpolation target must be free of blocks and
        // the border (Level.noCollision without the entity part).
        let (world, level) = (&self.world, &self.player.level);
        if let Some(t) = &mut self.entities {
            t.tick_with(|e, bb| block_free(world, level, e, bb));
        }
        if let Some(v) = &mut self.vehicles {
            v.tick_with(|e, bb| block_free(world, level, e, bb));
        }
        self.check_parked();
        if std::mem::take(&mut self.around.player_ticked)
            && (self.entities.is_some() || self.parked.is_some())
        {
            self.push_player(pose, false);
        }
        self.around.pre.clear();
        // ClientLevel.tick: the world border steps after the entities (the player included).
        self.world.tick();
    }

    fn crosshair_target(&self, q: &PickQuery<'_>) -> Crosshair {
        // The oracle-verified LocalPlayer.pick: block ray to the longer range, then the
        // entity ray, each filtered to its range attribute.
        use super::pick;
        let held_block = q
            .held
            .and_then(super::data::Item::from_id)
            .and_then(super::data::Item::block);
        let eye = if q.now {
            let p = &self.player;
            pick::eye_position(p.pos, self.old_pos, p.dims.eye_height)
        } else {
            [q.eyes.x, q.eyes.y, q.eyes.z]
        };
        let view = pick::view_vector(q.rotation.0, q.rotation.1);
        let (block_range, entity_range) = (self.ranges[0].value(), self.ranges[1].value());
        if q.entities.is_empty() {
            // pick_target without entities is the block pick.
            let hit = pick::pick(
                &self.world,
                eye,
                view,
                block_range,
                entity_range,
                held_block,
            );
            return Crosshair::Block(block_hit(&hit, eye));
        }
        let t = pick::pick_target(
            &self.world,
            &q.player_bb,
            eye,
            view,
            block_range,
            entity_range,
            held_block,
            q.entities,
        );
        modern_crosshair(t, eye)
    }

    fn predict_air(&mut self, x: i32, y: i32, z: i32) {
        self.world.set_block_state(x, y, z, BlockState::AIR);
    }

    fn held_item(&self, id: u16, count: u8, efficiency: u8) -> HeldItem {
        held_item_facts(id, count, efficiency)
    }

    fn chunk_batch_start(&mut self, now_nanos: i64) {
        self.world.on_chunk_batch_start(now_nanos);
    }

    fn chunk_batch_finished(&mut self, batch_size: i32, now_nanos: i64) -> f32 {
        self.world.on_chunk_batch_finished(batch_size, now_nanos)
    }

    fn step(
        &mut self,
        pose: &mut Pose,
        controls: Controls,
        using_item: bool,
        entity_id: i32,
        out: &mut Vec<Outbound>,
    ) {
        let input = ghast_physics::TickInput {
            keys: keys(controls),
            yaw: pose.yaw,
            pitch: pose.pitch,
            using_item,
        };
        self.packets.clear();
        self.player.use_effects = if using_item {
            self.use_effects()
        } else {
            modern::UseEffects::default()
        };
        // Entity.setOldPosAndRot at the start of the entity tick.
        self.old_pos = self.player.pos;
        if self.entities.is_some() {
            self.tick_before_player(pose);
        }
        if let Some(Riding::Seat { id }) = self.riding {
            // The vehicle ticks before its passenger: its position after this tick.
            let (world, level) = (&self.world, &self.player.level);
            let t = match (&mut self.entities, &mut self.vehicles) {
                (Some(t), _) => Some(t),
                (None, Some(v)) => Some(&mut **v),
                _ => None,
            };
            let pos = t.and_then(|t| t.tick_early(id, |e, bb| block_free(world, level, e, bb)));
            if let Some(pos) = pos {
                if self.entities.is_some() {
                    collect_colliders(
                        self.entities.as_ref(),
                        None,
                        &self.player,
                        Some(id),
                        &mut self.around.boxes,
                        &mut Vec::new(),
                    );
                } else {
                    self.around.boxes.clear();
                }
                let p = &mut self.player;
                let view = self
                    .world
                    .view(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
                let around = modern::Surroundings::new().with_entities(&self.around.boxes);
                let seat = modern::Seat::new(pos, modern::Seat::MINECART);
                modern::tick_passenger::<Rules, _>(
                    p,
                    &mut self.ext,
                    &around,
                    &seat,
                    &input,
                    &view,
                    &mut self.packets,
                );
                push_packets(&self.packets, entity_id, out);
                sync_pose_26(pose, &self.player);
                return;
            }
            // The vehicle is not known any more: walk on as before.
            self.riding = None;
        }
        if let Some(Riding::Boat { id, boat }) = &mut self.riding {
            // The boat ticks with its passenger (tickNonPassenger -> tickPassenger).
            let id = *id;
            let mut boat_boxes = Vec::new();
            if self.entities.is_some() {
                collect_colliders(
                    self.entities.as_ref(),
                    None,
                    &self.player,
                    Some(id),
                    &mut self.around.boxes,
                    &mut boat_boxes,
                );
            } else {
                self.around.boxes.clear();
            }
            let p = &mut self.player;
            let view = self
                .world
                .view(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
            let around = modern::Surroundings::new().with_entities(&self.around.boxes);
            modern::tick_boat::<Rules, _>(
                boat,
                p,
                &mut self.ext,
                &around,
                &boat_boxes,
                &input,
                &view,
                &mut self.packets,
            );
            push_packets(&self.packets, entity_id, out);
            sync_pose_26(pose, &self.player);
            return;
        }
        if self.entities.is_some() || self.parked.is_some() {
            collect_colliders(
                self.entities.as_ref(),
                self.parked.as_ref(),
                &self.player,
                None,
                &mut self.around.boxes,
                &mut Vec::new(),
            );
        } else {
            self.around.boxes.clear();
        }
        let p = &mut self.player;
        let view = self
            .world
            .view(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
        self.around.player_ticked = true;
        if !self.around.boxes.is_empty() {
            // Entities to collide with: the extended tick with them (tick_in).
            let around = modern::Surroundings::new().with_entities(&self.around.boxes);
            modern::tick_in::<Rules, _>(
                p,
                &mut self.ext,
                &around,
                &input,
                &view,
                &mut self.packets,
            );
            self.ext_active = self.ext_wanted();
        } else if self.ext_active {
            modern::tick_ext::<Rules, _>(p, &mut self.ext, &input, &view, &mut self.packets);
            self.ext_active = self.ext_wanted();
        } else {
            // Nothing of the extension state can act (no glide, no glider to start one, no
            // rocket): the plain tick, which is tick_ext with a default state, folded.
            modern::tick::<Rules, _>(p, &input, &view, &mut self.packets);
        }
        push_packets(&self.packets, entity_id, out);
        sync_pose_26(pose, &self.player);
    }
}

fn sync_pose_26(pose: &mut Pose, p: &modern::PlayerState) {
    pose.pos = Vec3::new(p.pos[0], p.pos[1], p.pos[2]);
    pose.motion = Vec3::new(p.vel[0], p.vel[1], p.vel[2]);
    pose.yaw = p.yaw;
    pose.pitch = p.pitch;
    pose.on_ground = p.on_ground;
    pose.flying = p.abilities.flying;
    pose.sneaking = p.shift();
    pose.sprinting = p.sprinting;
}

/// [`held_item_facts`] of this version's data set.
pub(crate) fn held_item_facts(id: u16, count: u8, efficiency: u8) -> HeldItem {
    let item = super::data::Item::from_id(id);
    let name = item.map_or("", super::data::Item::name);
    let food = item.and_then(super::data::Item::food);
    HeldItem {
        id,
        count,
        efficiency,
        places_block: item.is_some_and(|i| i.block().is_some()),
        usable: food.is_some()
            || matches!(
                name,
                "potion"
                    | "milk_bucket"
                    | "honey_bottle"
                    | "bow"
                    | "crossbow"
                    | "shield"
                    | "trident"
                    | "spyglass"
                    | "goat_horn"
            ),
        food: food.is_some(),
        always_edible: food.is_some_and(|f| f.can_always_eat),
        // The modern destroy progress reads the item itself.
        tool_speed: None,
        modern_item: item.map(|_| id),
        swing_on_use: false,
    }
}

/// `BlockBehaviour.getDestroyProgress` of this version (oracle-verified in its `data::dig`).
pub(crate) fn dig_progress(
    m: &crate::hands::ModernDig,
    held: Option<&HeldItem>,
    f: crate::hands::DigFactors,
) -> f32 {
    use super::data::{BlockState, Item, dig};
    let Some(state) = BlockState::from_id(m.state) else {
        return 0.0;
    };
    let item = held.and_then(|h| h.modern_item).and_then(Item::from_id);
    let c = dig::DigConditions {
        mining_efficiency: m.mining_efficiency,
        block_break_speed: m.block_break_speed,
        submerged_mining_speed: m.submerged_mining_speed,
        haste: (f.haste > 0).then(|| i32::from(f.haste) - 1),
        conduit_power: m.conduit_power,
        mining_fatigue: (f.fatigue > 0).then(|| i32::from(f.fatigue) - 1),
        eye_in_water: f.in_water,
        on_ground: f.on_ground,
    };
    dig::destroy_progress_with(state, item, &c)
}

/// The resolved entity ids (the per-version table tests).
#[cfg(test)]
pub(super) fn entity_ids_for_test() -> ([i32; 7], [i32; 2], [i32; 2], i32) {
    EntityIds::get().tuple()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine's teleport path (`ModernSim::teleport`, fed from the decoded `Teleport`) lands
    /// exactly where `modern::PlayerState::apply_teleport` does — the function the oracle's
    /// teleport goldens (`tests/golden/26.1/teleports.jsonl.gz`) prove — for random
    /// relative bits, deltas, rotations and prior motion.
    #[test]
    fn teleport_matches_the_proven_physics_call() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..2000 {
            let mut sim = ModernSim::new();
            let start = [rnd() * 64.0 - 32.0, rnd() * 100.0, rnd() * 64.0 - 32.0];
            sim.player.set_pos(start[0], start[1], start[2]);
            sim.player.vel = [rnd() - 0.5, rnd() - 0.5, rnd() - 0.5];
            sim.player.yaw = (rnd() * 720.0 - 360.0) as f32;
            sim.player.pitch = (rnd() * 180.0 - 90.0) as f32;
            let mut pose = Pose::default();
            sync_pose_26(&mut pose, &sim.player);
            let mut direct = sim.player;
            let relative = (rnd() * 512.0) as u32 & 0x1ff;
            let t = Teleport {
                x: rnd() * 40.0 - 20.0,
                y: rnd() * 40.0 - 20.0,
                z: rnd() * 40.0 - 20.0,
                yaw: (rnd() * 720.0 - 360.0) as f32,
                pitch: (rnd() * 200.0 - 100.0) as f32,
                flags: (relative & 0x1f) as i8,
                teleport_id: Some(1),
                relative,
                delta: Vec3::new(
                    rnd() * 40.0 - 20.0,
                    rnd() * 40.0 - 20.0,
                    rnd() * 40.0 - 20.0,
                ),
            };
            sim.teleport(&mut pose, &t);
            let _ = direct.apply_teleport(
                [t.x, t.y, t.z],
                [t.delta.x, t.delta.y, t.delta.z],
                t.yaw,
                t.pitch,
                relative,
            );
            assert_eq!(sim.player, direct, "{t:?}");
            assert_eq!(
                pose.pos,
                Vec3::new(direct.pos[0], direct.pos[1], direct.pos[2])
            );
            assert_eq!((pose.yaw, pose.pitch), (direct.yaw, direct.pitch));
        }
    }
}
