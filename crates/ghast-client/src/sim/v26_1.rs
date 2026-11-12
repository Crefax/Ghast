//! 26.1: the chunk store of `ghast-world` and the modern physics (`ghast_physics::modern` with
//! the `V26_1` rules, bit-exact against the 26.1 client).

use ghast_data::v26_1::BlockState;
use ghast_physics::modern::{self, AttributeKind, ModifierId, V26_1};
use ghast_protocol::{BlockPos, RawPacket};

use super::*;
use crate::common::{Abilities, AttributeUpdate, Controls, Outbound, Pose, Teleport, Vec3};
use crate::hands::{BlockQuery, HeldItem};

/// 26.1: the chunk store and the vanilla-exact modern physics.
pub(crate) struct Sim26 {
    world: ghast_world::v26_1::World,
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
    entities: Option<ghast_entity::v26_1::Tracker>,
    /// Join / respawn packets, replayed into a tracker started mid-session.
    session: SessionPackets,
    /// The level-load gate (`player_loaded`), oracle-proven on `joins.jsonl.gz`.
    gate: modern::LoadGate,
    /// `use_effects` of the item in use (see [`Sim26::use_effects`]).
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
}

/// Per 26.1 entity type: the player collides with it ([`modern::collides_26_1`]).
const COLLIDES: u8 = 1;
/// Per 26.1 entity type: its tick pushes the player like a living entity.
const PUSH_LIVING: u8 = 2;
/// Per 26.1 entity type: its tick pushes the player like a boat.
const PUSH_BOAT: u8 = 4;

/// [`COLLIDES`] / [`PUSH_LIVING`] / [`PUSH_BOAT`] by entity type id.
fn entity_bits(e: &ghast_entity::Entity) -> u8 {
    static BITS: std::sync::OnceLock<Box<[u8]>> = std::sync::OnceLock::new();
    let bits = BITS.get_or_init(|| {
        let types = ghast_data::v26_1::entity_types();
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
            if modern::collides_26_1(t.name) {
                b |= COLLIDES;
            }
            match modern::push_kind_26_1(t.name, t.is_living()) {
                Some(modern::PushKind::Living) => b |= PUSH_LIVING,
                Some(modern::PushKind::Boat) => b |= PUSH_BOAT,
                _ => {}
            }
            v[usize::from(t.id)] = b;
        }
        v.into_boxed_slice()
    });
    ghast_entity::v26_1::entity_type(e)
        .map_or(0, |t| bits.get(usize::from(t.id)).copied().unwrap_or(0))
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
    let t = ghast_entity::v26_1::entity_type(e)?;
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

/// Entity types whose screen is a mount screen (26.1 registry ids): horse, donkey, mule,
/// skeleton / zombie horse, camel, camel husk (`AbstractHorse`), llama, trader llama
/// (`Llama`), nautilus, zombie nautilus (`AbstractNautilus`).
const HORSES: [i32; 7] = [66, 36, 87, 116, 151, 19, 20];
const LLAMAS: [i32; 2] = [78, 134];
const NAUTILI: [i32; 2] = [88, 152];
const DATA_BABY: u8 = 16;
const DATA_MOUNT_FLAGS: u8 = 18;
/// Mounts remembered at most (nearest first is not known: the oldest are dropped).
const MAX_MOUNTS: usize = 128;

impl Mount {
    /// `canUseSlot(SADDLE)`, `canUseSlot(BODY)` (alive: the entity is still in the level).
    fn slots(&self) -> (bool, bool) {
        if LLAMAS.contains(&self.ty) {
            (true, true)
        } else if NAUTILI.contains(&self.ty) {
            let ok = !self.baby && self.flags & 4 != 0;
            (ok, ok)
        } else {
            (!self.baby && self.flags & 2 != 0, true)
        }
    }
}

/// Entity data indices (26.1, read from the real classes): `Entity.DATA_SHARED_FLAGS_ID`,
/// `FireworkRocketEntity.DATA_ATTACHED_TO_TARGET`, `Avatar.DATA_PLAYER_MAIN_HAND`.
const DATA_SHARED_FLAGS: u8 = 0;
/// `LivingEntity.DATA_LIVING_ENTITY_FLAGS` (bit value 4: the riptide spin).
const DATA_LIVING_FLAGS: u8 = 8;
const DATA_ATTACHED_TO_TARGET: u8 = 9;
const DATA_PLAYER_MAIN_HAND: u8 = 15;
/// `minecraft:firework_rocket` in the 26.1 entity type registry.
const FIREWORK_ROCKET: i32 = 53;
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

impl std::fmt::Debug for Sim26 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sim26")
            .field("world", &self.world)
            .field("pos", &self.player.pos)
            .finish_non_exhaustive()
    }
}

/// `Player.canGlide` equipment part: some equipment slot holds a stack that can glide using that
/// slot (`LivingEntity.canGlideUsing`: the `glider` component, `equippable` naming that slot,
/// the next point of damage will not break it).
fn glider_equipped(inv: &ghast_inventory::Inventory) -> bool {
    use ghast_inventory::v26_1::{Equip, can_glide_using};
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

impl Sim26 {
    /// `set_entity_data`, `add_entity`, `remove_entities` for our own entity and attached
    /// rockets (everything else is read only up to the entity id, or the entity type).
    fn own_entity_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        use ghast_protocol::v26_1::play::clientbound as cb;
        use ghast_protocol::v26_1::types::EntityMetadataEntryValue as V;
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
                if HORSES.contains(&ty) || LLAMAS.contains(&ty) || NAUTILI.contains(&ty) {
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
                if ty != FIREWORK_ROCKET {
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
                {
                    return Ok(());
                }
                let p: cb::EntityDestroy = raw.decode_as()?;
                for id in p.entity_ids {
                    self.own.rockets.retain(|&(rid, _)| rid != id);
                    self.own.mounts.retain(|m| m.id != id);
                    self.ext.remove_rocket(id);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The `use_effects` of the stack in use (`LivingEntity.getUseItem`), set by the engine
    /// each tick from `ghast_inventory::v26_1::item_use`; the component default without one.
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
                s.is_some_and(|s| {
                    ghast_inventory::v26_1::defaults(s.id())
                        .is_some_and(|d| d.name == "firework_rocket")
                })
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
            return modern::in_water_or_rain::<V26_1, _>(p, &view, raining);
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
        modern::in_water_or_rain::<V26_1, _>(&q, &view, true)
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
        if ticks < 10 || strength <= 0.0 || ghast_inventory::v26_1::next_damage_will_break(s) {
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
        modern::riptide::<V26_1, _>(p, &mut self.ext, &view, strength);
        self.ext_active = true;
    }

    /// The boxes of the entities the player may collide with this tick (`canBeCollidedWith`:
    /// boats, rafts, living shulkers), near enough for any move of this tick, in `getEntities`
    /// section order. Only with entity tracking.
    #[inline(never)]
    fn collect_colliders(&mut self) {
        let a = &mut self.around;
        a.boxes.clear();
        let Some(t) = &self.entities else { return };
        let p = &self.player;
        // Farther than any move of one tick can reach (the physics filters exactly).
        let reach = |v: f64| v.abs() + 3.0;
        let (rx, ry, rz) = (reach(p.vel[0]), reach(p.vel[1]), reach(p.vel[2]));
        let mut found: Vec<((i32, u64), [f64; 6])> = Vec::new();
        for e in t.iter() {
            if entity_bits(e) & COLLIDES == 0 {
                continue;
            }
            let Some((pos, b)) = entity_box(e) else {
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
            if ghast_entity::v26_1::health(e).is_some_and(|h| h <= 0.0) {
                continue;
            }
            found.push((section_order(pos), b));
        }
        // getEntities walks sections by x, then z / y (the order does not change the result:
        // each box only shortens the move).
        found.sort_by_key(|f| f.0);
        a.boxes.extend(found.into_iter().map(|(_, b)| aabb(b)));
    }

    /// The pushes of the entities that ticked after the player (`LivingEntity.pushEntities`,
    /// `AbstractBoat.tick`), each with its box after its own tick.
    #[inline(never)]
    fn push_player(&mut self, pose: &mut Pose) {
        let Some(t) = &self.entities else { return };
        let p = &self.player;
        let a = &mut self.around;
        a.pushers.clear();
        for e in t.iter() {
            let bits = entity_bits(e);
            if bits & (PUSH_LIVING | PUSH_BOAT) == 0 {
                continue;
            }
            let Some((pos, b)) = entity_box(e) else {
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
            let kind = if bits & PUSH_BOAT != 0 {
                modern::PushKind::Boat
            } else {
                modern::PushKind::Living
            };
            // Placeholder tick order: by entity id (vanilla: the tick list's insertion order).
            a.pushers
                .push((i64::from(e.id()), modern::Pusher::new(pos, aabb(b), kind)));
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
            modern::push_by::<V26_1, _>(p, &self.ext, &view, pusher);
        }
        pose.motion = Vec3::new(p.vel[0], p.vel[1], p.vel[2]);
    }

    pub(crate) fn new() -> Self {
        Sim26 {
            world: ghast_world::v26_1::World::new(config()),
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
            session: SessionPackets::default(),
            gate: modern::LoadGate::waiting(),
        }
    }

    /// A sim with options (world contents, entity tracking). Recording and the Grim step
    /// compatibility are not available on 26.1 yet.
    pub(crate) fn with(opts: &SimOptions) -> Self {
        let mut s = Self::new();
        if let Some(c) = opts.world {
            s.world = ghast_world::v26_1::World::new(c);
        }
        if let Some(p) = opts.record.as_deref() {
            log::warn!(
                "movement recording is not supported on 26.1; not recording to {}",
                p.display()
            );
        }
        s.entities = opts.entities.map(ghast_entity::v26_1::Tracker::new);
        if opts.force_step_position != ForceStepPosition::Off {
            log::warn!("force_step_position has no effect on 26.1");
        }
        s
    }

    /// Test hook: a world in the vanilla overworld (registries as with known packs).
    #[cfg(all(test, feature = "v1_8", feature = "v1_12_2", feature = "v1_16_5"))]
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

impl WorldView for Sim26 {
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
            exact: Some(crate::hands::ExactDig::V26_1(crate::hands::ModernDig {
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
        use ghast_physics::pick;
        // 26.1's own look vector (`calculateViewVector`, double-index sine table).
        let view = pick::v26_1::view_vector(yaw, pitch);
        let eye = [eyes.x, eyes.y, eyes.z];
        // Survival reach on both rays: the entity range (3) is shorter, the block range wins.
        let hit = pick::v26_1::pick(&self.world, eye, view, reach, 3.0, None);
        block_hit(&hit, eye)
    }
}

impl Sim for Sim26 {
    fn crosshair_now(
        &self,
        _eyes: Vec3,
        (yaw, pitch): (f32, f32),
        _look: Vec3,
        held: Option<u16>,
    ) -> Option<crate::BlockHit> {
        use ghast_physics::pick::v26_1 as pick;
        let p = &self.player;
        // Minecraft.pick at partial tick 1: the eye lerps from the old position.
        let eye = pick::eye_position(p.pos, self.old_pos, p.dims.eye_height);
        let held_block = held
            .and_then(ghast_data::v26_1::Item::from_id)
            .and_then(ghast_data::v26_1::Item::block);
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
        use ghast_protocol::v26_1::play::clientbound as cb;
        self.session.note(raw, cb::id::LOGIN, cb::id::RESPAWN);
        match raw.id {
            cb::id::ENTITY_METADATA | cb::id::SPAWN_ENTITY | cb::id::ENTITY_DESTROY => {
                self.own_entity_packet(raw)?;
            }
            cb::id::LOGIN => {
                self.own.id = raw.decode_as::<cb::Login>()?.entity_id;
                self.own.rockets.clear();
                self.own.mounts.clear();
            }
            _ => {}
        }
        // The tracker also reads map_chunk / unload_chunk (which columns tick): the world gets
        // the same packets below.
        if let Some(t) = &mut self.entities
            && t.wants(raw.id)
        {
            t.apply(raw)?;
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
        // World::handle_raw) would link every 26.1 decoder into the binary.
        use ghast_protocol::v26_1::play::clientbound::ClientboundPlayPacket as P;
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
        match raw.decode_as::<ghast_protocol::v26_1::configuration::clientbound::RegistryData>() {
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
            s.filter(|s| {
                ghast_inventory::v26_1::defaults(s.id()).is_some_and(|d| d.name == "trident")
            })
            .cloned()
        };
        let hands = [trident(inv.held()), trident(inv.offhand())];
        for (k, s) in hands.into_iter().enumerate() {
            self.riptide[k] = s
                .and_then(|s| inv.trident_spin_attack_strength_26_1(&s))
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
        if let Some(u) = ghast_inventory::v26_1::item_use(stack) {
            held.usable = u.duration.is_none_or(|d| d > 0);
            held.food = u.food.is_some();
            held.always_edible = u.food.is_some_and(|f| f.can_always_eat);
        }
        // FireworkRocketItem.use: SUCCESS (the server spawns the boosting rocket) only while
        // gliding; otherwise PASS.
        // TridentItem.use: with riptide only in water or rain (else FAIL: no use state).
        if self.riptide[usize::from(off_hand)] > 0.0
            && ghast_inventory::v26_1::defaults(stack.id()).is_some_and(|d| d.name == "trident")
        {
            held.usable = held.usable && self.wet();
        }
        held.swing_on_use = self.ext.fall_flying
            && ghast_inventory::v26_1::defaults(stack.id())
                .is_some_and(|d| d.name == "firework_rocket");
        held
    }

    fn pre_tick(&mut self, inv: &ghast_inventory::Inventory, t: &super::PreTick) -> bool {
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
        self.use_effects =
            ghast_inventory::v26_1::item_use(s).map(|u| (u.can_sprint, u.speed_multiplier));
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
        let (saddle, body) = m.slots();
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
            *t = ghast_entity::v26_1::Tracker::new(t.config());
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
        // A new LocalPlayer: not gliding, no rockets attached.
        self.ext = modern::PlayerExt::default();
        self.ext_active = false;
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
        self.reset_player();
        if keep & 1 != 0 {
            self.player.attributes = old.attributes;
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
        // 26.1 attributes arrive as [`Sim::attributes`].
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
            if let Some(i) = mining {
                self.mining[i].apply_packet(a.base, &mods);
            } else if let Some(i) = range {
                self.ranges[i].apply_packet(a.base, &mods);
            } else if let Some(kind) = AttributeKind::from_name(a.name) {
                self.player.apply_attribute(kind, a.base, &mods);
            }
        }
    }

    fn status_effects(&mut self, effects: &std::collections::BTreeMap<i8, i8>) {
        // The engine keeps classic 1-based effect numbers (26.1 registry id + 1).
        let e = &mut self.player.effects;
        e.jump_boost = effects.get(&8).map(|a| i32::from(*a));
        e.blindness = effects.contains_key(&15);
        e.levitation = effects.get(&25).map(|a| i32::from(*a));
        e.slow_falling = effects.contains_key(&28);
        e.dolphins_grace = effects.contains_key(&30);
        e.weaving = effects.contains_key(&37);
        self.conduit_power = effects.get(&29).map(|a| i32::from(*a));
    }

    fn boots(&mut self, depth_strider: u8, soul_speed: u8) {
        // 26.1 sends depth strider and soul speed as attributes.
        let _ = (depth_strider, soul_speed);
    }

    fn feet_item(&mut self, id: Option<i32>) {
        let leather = ghast_data::v26_1::Item::from_name("leather_boots");
        self.player.leather_boots = id
            .and_then(|i| u16::try_from(i).ok())
            .and_then(ghast_data::v26_1::Item::from_id)
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
        use ghast_protocol::v26_1::play::clientbound as cb;
        // Our own entity data and attached rockets (gliding), decoded only when they concern us.
        let own = [
            cb::id::ENTITY_METADATA,
            cb::id::SPAWN_ENTITY,
            cb::id::ENTITY_DESTROY,
        ];
        self.world.packet_ids().chain(own).collect()
    }

    fn entity_config(&self) -> Option<ghast_entity::Config> {
        self.entities.as_ref().map(|t| t.config())
    }

    fn set_entities(&mut self, config: Option<ghast_entity::Config>) {
        if self.entity_config() == config {
            return;
        }
        self.entities = config.map(|c| {
            let mut t = ghast_entity::v26_1::Tracker::new(c);
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
    }

    fn entity_packet_ids(&self) -> Vec<i32> {
        self.entities
            .as_ref()
            .map(|t| t.packet_ids().collect())
            .unwrap_or_default()
    }

    fn pick_margin(&self, e: &ghast_entity::Entity) -> Option<f32> {
        let t = self.entities.as_ref()?;
        t.pickable(e).then(|| ghast_entity::v26_1::pick_radius(e))
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
        if let Some(t) = &mut self.entities {
            let (world, level) = (&self.world, &self.player.level);
            t.tick_with(|e, bb| {
                let pos = e.position();
                modern::no_block_collision::<V26_1, _>(
                    world,
                    level,
                    pos,
                    bb,
                    &modern::CollisionContext {
                        feet_y: pos[1],
                        ..Default::default()
                    },
                )
            });
            if std::mem::take(&mut self.around.player_ticked) {
                self.push_player(pose);
            }
        }
    }

    fn crosshair_target(&self, q: &PickQuery<'_>) -> Crosshair {
        // The oracle-verified LocalPlayer.pick: block ray to the longer range, then the
        // entity ray, each filtered to its range attribute.
        use ghast_physics::pick::v26_1 as pick;
        let held_block = q
            .held
            .and_then(ghast_data::v26_1::Item::from_id)
            .and_then(ghast_data::v26_1::Item::block);
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
        held_item_facts(DataSet::V26_1, id, count, efficiency)
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
            self.collect_colliders();
        }
        let p = &mut self.player;
        let view = self
            .world
            .view(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
        self.around.player_ticked = true;
        if !self.around.boxes.is_empty() {
            // Entities to collide with: the extended tick with them (tick_in).
            let around = modern::Surroundings::new().with_entities(&self.around.boxes);
            modern::tick_in::<V26_1, _>(
                p,
                &mut self.ext,
                &around,
                &input,
                &view,
                &mut self.packets,
            );
            let x = &self.ext;
            self.ext_active = x.fall_flying
                || x.glider
                || x.rocket_count() > 0
                || x.spin_attack
                || x.auto_spin_ticks > 0;
        } else if self.ext_active {
            modern::tick_ext::<V26_1, _>(p, &mut self.ext, &input, &view, &mut self.packets);
            let x = &self.ext;
            self.ext_active = x.fall_flying
                || x.glider
                || x.rocket_count() > 0
                || x.spin_attack
                || x.auto_spin_ticks > 0;
        } else {
            // Nothing of the extension state can act (no glide, no glider to start one, no
            // rocket): the plain tick, which is tick_ext with a default state, folded.
            modern::tick::<V26_1, _>(p, &input, &view, &mut self.packets);
        }
        push_packets(&self.packets, entity_id, out);
        sync_pose_26(pose, &self.player);
        self.world.tick();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine's teleport path (`Sim26::teleport`, fed from the decoded `Teleport`) lands
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
            let mut sim = Sim26::new();
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
