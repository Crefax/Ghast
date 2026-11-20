//! 1.8.9: the chunk store and the vanilla-exact physics.

use ghast_protocol::{BlockPos, RawPacket};

use super::*;
use crate::common::{Abilities, Controls, Outbound, Pose, Teleport, Vec3};
use crate::hands::{BlockQuery, HeldItem};

/// Block-local `[min, max]` of a physics box.
fn arr(b: &ghast_physics::math::Aabb) -> [f64; 6] {
    [b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]
}

/// The 1.8 ray shape at a position (ghast-physics: vanilla's bounds after
/// `Block.setBoundingBox`, neighbour-dependent shapes and the stairs passes):
/// `(selection, stairs passes)`.
fn ray_1_8(w: &ghast_world::v1_8::World, x: i32, y: i32, z: i32) -> RayBox {
    use ghast_physics::v1_8::RayShape;
    match ghast_physics::v1_8::ray_shape(w, x, y, z, false) {
        RayShape::None => (None, None),
        RayShape::Bounds(b) => (Some(arr(&b)), None),
        RayShape::Stairs(p) => (None, Some(p.boxes().map(|b| b.as_ref().map(arr)))),
    }
}

/// A ray shape as the hands take it.
type RayBox = (Option<[f64; 6]>, Option<[Option<[f64; 6]>; 8]>);

// ---- 1.8.9: ghast-physics ---------------------------------------------------------------

/// 1.8.9: the chunk store and the vanilla-exact physics.
pub(crate) struct Sim18 {
    world: ghast_world::v1_8::World,
    pub(super) player: ghast_physics::v1_8::PlayerState,
    packets: ghast_physics::Packets,
    rec: Option<crate::record::Recorder>,
    force_step_position: ForceStepPosition,
    entities: Option<ghast_entity::v1_8::Tracker>,
    /// Block changes since the last drain (position, version raw state), when collected.
    updates: BlockUpdates,
    /// The session's last join and respawn packets: a tracker started mid-session replays
    /// them to learn our entity id and dimension.
    session: SessionPackets,
    /// The world keeps block entities (signs, tile entity NBT).
    block_entities: bool,
    /// The boat we ride (the 1.8 client simulates it; with entity tracking).
    ride: Option<Box<Ride18>>,
    /// Our entity id (join game).
    own_id: Option<i32>,
    /// The flags byte (entity data index 0) the server last sent for our entity.
    server_flags: Option<u8>,
    /// Test hook: the JVM's `Math.sin` / `Math.cos` of the next boat tick (argument bits, sine,
    /// cosine), from an oracle record.
    #[cfg(test)]
    trig: Vec<(u64, f64, f64)>,
}

/// Test hook: `Math.sin` / `Math.cos` from a table, the platform's for other arguments.
#[cfg(test)]
struct TableTrig<'a>(&'a [(u64, f64, f64)]);

#[cfg(test)]
impl ghast_physics::v1_8::JavaTrig for TableTrig<'_> {
    fn sin(&self, x: f64) -> f64 {
        self.0
            .iter()
            .find(|e| e.0 == x.to_bits())
            .map_or_else(|| x.sin(), |e| e.1)
    }
    fn cos(&self, x: f64) -> f64 {
        self.0
            .iter()
            .find(|e| e.0 == x.to_bits())
            .map_or_else(|| x.cos(), |e| e.2)
    }
}

/// The boat the player rides: its entity id and its client-side state.
#[derive(Debug)]
struct Ride18 {
    id: i32,
    seat: Seat18,
}

/// The boat the 1.8 client simulates, or the minecart the server moves.
#[derive(Debug)]
enum Seat18 {
    Boat(ghast_physics::v1_8::Boat),
    Cart(ghast_physics::v1_8::Minecart),
}

impl std::fmt::Debug for Sim18 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sim18")
            .field("world", &self.world)
            .field("pos", &self.player.pos)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
impl Sim18 {
    /// Test hook: the physics player.
    pub(crate) fn test_player(&self) -> &ghast_physics::v1_8::PlayerState {
        &self.player
    }

    /// Test hook: the JVM's sine / cosine values of the next boat ticks (argument bits, sine,
    /// cosine).
    #[cfg(all(feature = "v1_12_2", feature = "v1_16_5", feature = "v26_1"))]
    pub(crate) fn test_set_trig(&mut self, t: Vec<(u64, f64, f64)>) {
        self.trig = t;
    }

    /// Test hook: replaces the physics player and syncs `pose`.
    pub(crate) fn test_set_player(&mut self, pose: &mut Pose, p: ghast_physics::v1_8::PlayerState) {
        self.player = p;
        sync_pose(pose, &self.player);
    }

    /// Test hook: sets one block.
    #[cfg(all(test, feature = "v1_12_2", feature = "v1_16_5"))]
    pub(crate) fn test_set_block(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        s: ghast_data::v1_8::BlockState,
    ) {
        self.world.set_block_state(x, y, z, s);
    }

    /// Test hook: the chunk store.
    pub(crate) fn test_world(&self) -> &ghast_world::v1_8::World {
        &self.world
    }
}

impl Sim18 {
    pub(crate) fn new() -> Self {
        Sim18 {
            world: ghast_world::v1_8::World::new(config()),
            player: ghast_physics::v1_8::PlayerState::new([0.0; 3], 0.0, 0.0),
            packets: ghast_physics::Packets::new(),
            rec: None,
            force_step_position: ForceStepPosition::Off,
            entities: None,
            updates: BlockUpdates::default(),
            session: SessionPackets::default(),
            block_entities: false,
            ride: None,
            own_id: None,
            server_flags: None,
            #[cfg(test)]
            trig: Vec::new(),
        }
    }

    /// A sim with options (recording, Grim compatibility).
    pub(crate) fn with(opts: &SimOptions) -> Self {
        let mut s = Self::new();
        s.force_step_position = opts.force_step_position;
        s.entities = opts.entities.map(ghast_entity::v1_8::Tracker::new);
        if let Some(c) = opts.world {
            s.world = ghast_world::v1_8::World::new(c);
            s.block_entities = c.block_entities;
        }
        if opts.shared_chunks.is_some() {
            log::debug!("shared chunks are not used on 1.8.9; every section stays owned");
        }
        if let Some(p) = opts.record.as_deref() {
            match crate::record::Recorder::create(p) {
                Ok(r) => s.rec = Some(r),
                Err(e) => log::warn!("cannot record to {}: {e}", p.display()),
            }
        }
        s
    }
}

impl Sim18 {
    /// `entity_metadata`: only our own entity's flags byte (index 0) is read. Vanilla's data
    /// watcher writes it into the local player too (`NetHandlerPlayClient.handleEntityMetadata`):
    /// the sprint bit sets the flag `isSprinting()` reads (`Entity.setFlag(3)`, not
    /// `setSprinting`: the speed modifier stays until the client's own movement update changes
    /// it). Sneaking is the input's on the 1.8 client (`isSneaking()`), the other bits are not
    /// read by movement.
    fn own_metadata(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        use ghast_protocol::defs::p47::types::EntityMetadataValue as V;
        use ghast_protocol::v1_8::play::clientbound as cb;
        let Some(own) = self.own_id else {
            return Ok(());
        };
        if ghast_protocol::Reader::new(&raw.body).read_varint()? != own {
            return Ok(());
        }
        let p: cb::EntityMetadata = raw.decode_as()?;
        for e in &p.metadata {
            if let (0, V::Byte(b)) = (e.key, &e.value) {
                let flags = *b as u8;
                self.server_flags = Some(flags);
                self.player.sprinting = flags & 0x08 != 0;
                if let Some(r) = &mut self.rec {
                    r.note("own entity flags not representable");
                }
            }
        }
        Ok(())
    }

    /// The boat packets (with entity tracking): `attach_entity` naming us (boarding a tracked
    /// boat, or leaving it with vehicle -1), and while riding the server's teleport, relative
    /// moves, looks and velocity of that boat and its destruction (vanilla's
    /// `World.removeEntity` takes the rider off).
    #[cold]
    #[inline(never)]
    fn vehicle_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        use ghast_protocol::v1_8::play::clientbound as cb;
        let Some(t) = &self.entities else {
            return Ok(());
        };
        if raw.id == cb::id::ATTACH_ENTITY {
            let p: cb::AttachEntity = raw.decode_as()?;
            if p.leash || t.self_id() != Some(p.entity_id) {
                return Ok(());
            }
            if p.vehicle_id == -1 {
                if let Some(r) = self.ride.take() {
                    log::debug!("left boat {}", r.id);
                }
                return Ok(());
            }
            // onEntityAttach: a boat the client knows starts its physics; a minecart follows
            // the server. The client's interpolation of either at rest ends at the server's
            // last position.
            let seat = t.get(p.vehicle_id).and_then(|e| {
                let fixed = e.position().map(|v| (v * 32.0).round() as i32);
                let yaw = (e.yaw() * 256.0 / 360.0).round() as i32 as i8;
                let pitch = (e.pitch() * 256.0 / 360.0).round() as i32 as i8;
                match e.entity_type() {
                    ghast_entity::EntityType::LegacyObject(1) => Some(Seat18::Boat(
                        ghast_physics::v1_8::Boat::spawned(fixed, yaw, pitch),
                    )),
                    ghast_entity::EntityType::LegacyObject(10) => Some(Seat18::Cart(
                        ghast_physics::v1_8::Minecart::spawned(fixed, yaw, pitch),
                    )),
                    _ => None,
                }
            });
            match seat {
                Some(seat) => {
                    log::debug!("riding {}", p.vehicle_id);
                    self.ride = Some(Box::new(Ride18 {
                        id: p.vehicle_id,
                        seat,
                    }));
                }
                None => {
                    self.ride = None;
                    log::debug!("riding {} (not simulated: not a boat)", p.vehicle_id);
                }
            }
            return Ok(());
        }
        let Some(r) = self.ride.as_deref_mut() else {
            return Ok(());
        };
        if let Seat18::Cart(c) = &mut r.seat {
            let id = r.id;
            match raw.id {
                cb::id::ENTITY_TELEPORT => {
                    let p: cb::EntityTeleport = raw.decode_as()?;
                    if p.entity_id == id {
                        c.server_teleport([p.x, p.y, p.z], p.yaw, p.pitch);
                    }
                }
                cb::id::REL_ENTITY_MOVE => {
                    let p: cb::RelEntityMove = raw.decode_as()?;
                    if p.entity_id == id {
                        c.server_update([p.d_x, p.d_y, p.d_z], None);
                    }
                }
                cb::id::ENTITY_MOVE_LOOK => {
                    let p: cb::EntityMoveLook = raw.decode_as()?;
                    if p.entity_id == id {
                        c.server_update([p.d_x, p.d_y, p.d_z], Some((p.yaw, p.pitch)));
                    }
                }
                cb::id::ENTITY_LOOK => {
                    let p: cb::EntityLook = raw.decode_as()?;
                    if p.entity_id == id {
                        c.server_update([0; 3], Some((p.yaw, p.pitch)));
                    }
                }
                cb::id::ENTITY => {
                    let p: cb::Entity = raw.decode_as()?;
                    if p.entity_id == id {
                        c.server_update([0; 3], None);
                    }
                }
                cb::id::ENTITY_DESTROY => {
                    let p: cb::EntityDestroy = raw.decode_as()?;
                    if p.entity_ids.contains(&id) {
                        log::debug!("minecart {id} destroyed");
                        self.ride = None;
                    }
                }
                // A minecart's client motion does not move it.
                _ => {}
            }
            return Ok(());
        }
        let Seat18::Boat(b) = &mut r.seat else {
            return Ok(());
        };
        match raw.id {
            cb::id::ENTITY_TELEPORT => {
                let p: cb::EntityTeleport = raw.decode_as()?;
                if p.entity_id == r.id {
                    b.server_teleport([p.x, p.y, p.z], p.yaw, p.pitch, p.on_ground);
                }
            }
            cb::id::REL_ENTITY_MOVE => {
                let p: cb::RelEntityMove = raw.decode_as()?;
                if p.entity_id == r.id {
                    b.server_move([p.d_x, p.d_y, p.d_z], p.on_ground);
                }
            }
            cb::id::ENTITY_MOVE_LOOK => {
                let p: cb::EntityMoveLook = raw.decode_as()?;
                if p.entity_id == r.id {
                    b.server_move([p.d_x, p.d_y, p.d_z], p.on_ground);
                }
            }
            cb::id::ENTITY_LOOK => {
                let p: cb::EntityLook = raw.decode_as()?;
                if p.entity_id == r.id {
                    b.server_move([0; 3], p.on_ground);
                }
            }
            cb::id::ENTITY => {
                // The bare entity packet: no move, its ground flag is false.
                let p: cb::Entity = raw.decode_as()?;
                if p.entity_id == r.id {
                    b.server_move([0; 3], false);
                }
            }
            cb::id::ENTITY_VELOCITY => {
                let p: cb::EntityVelocity = raw.decode_as()?;
                if p.entity_id == r.id {
                    let v = p.velocity;
                    let c = |x: i16| f64::from(x) / 8000.0;
                    b.server_velocity([c(v.x), c(v.y), c(v.z)]);
                }
            }
            cb::id::ENTITY_DESTROY => {
                let p: cb::EntityDestroy = raw.decode_as()?;
                if p.entity_ids.contains(&r.id) {
                    log::debug!("boat {} destroyed", r.id);
                    self.ride = None;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// A tick in the boat the 1.8 client simulates (`ghast_physics::v1_8::tick_boat`).
    #[cold]
    #[inline(never)]
    fn step_ride(&mut self, input: &ghast_physics::TickInput) {
        let Some(r) = self.ride.as_deref_mut() else {
            return;
        };
        let id = r.id;
        let boat = match &mut r.seat {
            Seat18::Boat(b) => b,
            Seat18::Cart(c) => {
                // The minecart the server moves (`ghast_physics::v1_8::tick_minecart`).
                ghast_physics::v1_8::tick_minecart(
                    &mut self.player,
                    c,
                    input,
                    &self.world,
                    &ghast_physics::v1_8::Options::default(),
                    &mut self.packets,
                );
                return;
            }
        };
        #[cfg(test)]
        let still = ghast_physics::v1_8::tick_boat_with(
            &mut self.player,
            boat,
            input,
            &self.world,
            &ghast_physics::v1_8::Options::default(),
            &TableTrig(&self.trig),
            &mut self.packets,
        );
        #[cfg(not(test))]
        let still = ghast_physics::v1_8::tick_boat(
            &mut self.player,
            boat,
            input,
            &self.world,
            &ghast_physics::v1_8::Options::default(),
            &mut self.packets,
        );
        for &[x, y, z] in boat.broken_blocks() {
            self.world
                .set_block_state(x, y, z, ghast_data::v1_8::BlockState::AIR);
        }
        if !still {
            log::debug!("boat {id} fell out of the world");
            self.ride = None;
        }
    }
}

impl WorldView for Sim18 {
    fn column(&self, chunk_x: i32, chunk_z: i32) -> Option<&ghast_world::Column> {
        self.world.column(chunk_x, chunk_z)
    }

    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.world.is_column_loaded(chunk_x, chunk_z)
    }

    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.world.block_state(x, y, z).map(|s| u32::from(s.raw()))
    }

    fn entities(&self) -> Option<&ghast_entity::Entities> {
        self.entities.as_deref()
    }

    fn loaded_columns(&self) -> usize {
        self.world.columns().len()
    }

    fn block_query(&self, x: i32, y: i32, z: i32) -> Option<BlockQuery> {
        let s = self.world.block_state(x, y, z)?;
        let b = s.block();
        let info = b.info();
        let ray = ray_1_8(&self.world, x, y, z);
        Some(BlockQuery {
            exact: Some(crate::hands::ExactDig::V1_8),
            air: s.is_air(),
            water: matches!(b.id(), 8 | 9),
            block: u16::from(b.id()),
            hardness: info.hardness.unwrap_or(-1.0),
            dig_material: info.material,
            harvest_tools: info.harvest_tools,
            selection: ray.0,
            stairs: ray.1,
            activates: ACTIVATED_1_8.contains(&b.id()),
        })
    }

    fn crosshair(
        &self,
        eyes: Vec3,
        (yaw, pitch): (f32, f32),
        _look: Vec3,
        reach: f64,
    ) -> Option<crate::BlockHit> {
        // The oracle-verified 1.8 pick (ghast_physics::pick::v1_8: vanilla's ray shapes,
        // Entity.rayTrace with the float look vector).
        use ghast_physics::pick::v1_8 as pick;
        let eye = [eyes.x, eyes.y, eyes.z];
        let hit = pick::pick(&self.world, eye, pick::look_vector(yaw, pitch), reach)?;
        block_hit(&hit, eye)
    }
}

impl Sim for Sim18 {
    fn crosshair_target(&self, q: &PickQuery<'_>) -> Crosshair {
        // The oracle-verified GameRenderer.updateTargetedEntity: block ray at the game
        // mode's reach, then the entity ray.
        use ghast_physics::pick::v1_8 as pick;
        let eye = [q.eyes.x, q.eyes.y, q.eyes.z];
        let look = pick::look_vector(q.rotation.0, q.rotation.1);
        if q.entities.is_empty() {
            // pick_target without entities is its block pick (`target` returns it as is).
            let reach = if q.creative {
                pick::CREATIVE_REACH
            } else {
                pick::REACH
            };
            return Crosshair::Block(
                pick::pick(&self.world, eye, look, reach).and_then(|h| block_hit(&h, eye)),
            );
        }
        let t = pick::pick_target(&self.world, eye, look, q.creative, &q.player_bb, q.entities);
        legacy_crosshair(t, eye)
    }

    fn world_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        {
            use ghast_protocol::v1_8::play::clientbound::id;
            self.session.note(raw, id::LOGIN, id::RESPAWN);
        }
        if let Some(t) = &mut self.entities
            && t.wants(raw.id)
        {
            t.apply(raw)?;
        }
        use ghast_protocol::v1_8::play::clientbound as cb;
        if self.entities.is_some()
            && (raw.id == cb::id::ATTACH_ENTITY
                || (self.ride.is_some()
                    && matches!(
                        raw.id,
                        cb::id::ENTITY_TELEPORT
                            | cb::id::REL_ENTITY_MOVE
                            | cb::id::ENTITY_MOVE_LOOK
                            | cb::id::ENTITY_LOOK
                            | cb::id::ENTITY
                            | cb::id::ENTITY_VELOCITY
                            | cb::id::ENTITY_DESTROY
                    )))
        {
            return self.vehicle_packet(raw);
        }
        let w = &mut self.world;
        match raw.id {
            cb::id::LOGIN => {
                self.ride = None;
                let p: cb::Login = raw.decode_as()?;
                self.own_id = Some(p.entity_id);
                w.apply_login(&p);
            }
            cb::id::ENTITY_METADATA => return self.own_metadata(raw),
            cb::id::RESPAWN => {
                self.ride = None;
                w.apply_respawn(&raw.decode_as()?);
            }
            cb::id::MAP_CHUNK => {
                w.apply_map_chunk(&raw.decode_as()?)?;
            }
            cb::id::MAP_CHUNK_BULK => {
                w.apply_map_chunk_bulk(&raw.decode_as()?)?;
            }
            cb::id::BLOCK_CHANGE => {
                let p: cb::BlockChange = raw.decode_as()?;
                self.updates.push(p.location, p.r#type as u32);
                w.apply_block_change(&p);
            }
            cb::id::MULTI_BLOCK_CHANGE => {
                let p: cb::MultiBlockChange = raw.decode_as()?;
                for r in &p.records {
                    let x = p.chunk_x * 16 + i32::from(r.horizontal_pos >> 4);
                    let z = p.chunk_z * 16 + i32::from(r.horizontal_pos & 15);
                    self.updates
                        .push(BlockPos::new(x, i32::from(r.y), z), r.block_id as u32);
                }
                w.apply_multi_block_change(&p);
            }
            cb::id::EXPLOSION => {
                w.apply_explosion(&raw.decode_as()?);
            }
            cb::id::WORLD_BORDER => {
                w.apply_world_border(&raw.decode_as()?);
            }
            cb::id::UPDATE_SIGN if self.block_entities => {
                w.apply_update_sign(&raw.decode_as()?);
            }
            cb::id::TILE_ENTITY_DATA if self.block_entities => {
                w.apply_tile_entity_data(&raw.decode_as()?);
            }
            _ => {}
        }
        Ok(())
    }

    fn reset_player(&mut self) {
        // A fresh ClientPlayerEntity: at the origin, lastReported* zero; abilities and food are
        // re-sent by the server.
        let abilities = self.player.abilities;
        let food = self.player.food_level;
        self.player = ghast_physics::v1_8::PlayerState::new([0.0; 3], 0.0, 0.0);
        self.player.abilities = abilities;
        self.player.food_level = food;
        self.ride = None;
        self.server_flags = None;
    }

    fn mount_rules(&self, entity_id: i32) -> Option<ghast_inventory::MountRules> {
        // HorseScreenHandler: the saddle slot always takes a saddle; the armor slot horse armor
        // when the horse type (entity data 19) is 0.
        use ghast_inventory::{MountArmor, MountRules};
        let t = self.entities.as_ref()?;
        let e = t.get(entity_id)?;
        if ghast_entity::v1_8::info(e.entity_type())?.name != "EntityHorse" || !t.config().metadata
        {
            return None;
        }
        let horse = e.metadata().byte(19).unwrap_or(0) == 0;
        let armor = if horse {
            MountArmor::HorseArmor
        } else {
            MountArmor::None
        };
        Some(MountRules::new(true, armor))
    }

    fn vehicle(&self) -> Option<crate::common::Vehicle> {
        let r = self.ride.as_deref()?;
        let b = match &r.seat {
            Seat18::Boat(b) => b,
            Seat18::Cart(c) => {
                return Some(crate::common::Vehicle {
                    entity_id: r.id,
                    pos: Vec3::new(c.pos[0], c.pos[1], c.pos[2]),
                    yaw: c.yaw,
                    pitch: c.pitch,
                    on_ground: false,
                    controlled: false,
                });
            }
        };
        Some(crate::common::Vehicle {
            entity_id: r.id,
            pos: Vec3::new(b.pos[0], b.pos[1], b.pos[2]),
            yaw: b.yaw,
            pitch: b.pitch,
            on_ground: b.on_ground,
            controlled: true,
        })
    }

    fn teleport(&mut self, pose: &mut Pose, t: &Teleport) {
        use crate::common::teleport_flags as fl;
        let rel = [
            t.flags & fl::X != 0,
            t.flags & fl::Y != 0,
            t.flags & fl::Z != 0,
            t.flags & fl::YAW != 0,
            t.flags & fl::PITCH != 0,
        ];
        // The engine keeps the look (its rotation may have moved it since the last tick).
        self.player.yaw = pose.yaw;
        self.player.pitch = pose.pitch;
        let _reply = self
            .player
            .apply_teleport([t.x, t.y, t.z], t.yaw, t.pitch, rel);
        if let Some(r) = &mut self.rec {
            let p = &self.player;
            r.teleport(
                [t.x, t.y, t.z],
                t.yaw,
                t.pitch,
                t.flags,
                t.teleport_id,
                [p.pos[0], p.bb.min_y, p.pos[2]],
                p.yaw,
                p.pitch,
            );
        }
        sync_pose(pose, &self.player);
    }

    fn abilities(&mut self, a: Abilities) {
        let a = ghast_physics::Abilities {
            invulnerable: a.flags & 1 != 0,
            flying: a.flags & 2 != 0,
            allow_flying: a.flags & 4 != 0,
            creative: a.flags & 8 != 0,
            fly_speed: a.flying_speed,
            walk_speed: a.walking_speed,
        };
        if let Some(r) = &mut self.rec {
            r.abilities(&a);
        }
        self.player.apply_abilities(a);
    }

    fn food(&mut self, food: i32) {
        self.player.food_level = food;
    }

    fn velocity(&mut self, v: Vec3, add: bool) {
        if let Some(r) = &mut self.rec {
            if add {
                r.note("explosion push not representable");
            } else {
                r.velocity([
                    (v.x * 8000.0).round() as i32,
                    (v.y * 8000.0).round() as i32,
                    (v.z * 8000.0).round() as i32,
                ]);
            }
        }
        let p = &mut self.player;
        if add {
            p.vel[0] += v.x;
            p.vel[1] += v.y;
            p.vel[2] += v.z;
        } else {
            p.vel = [v.x, v.y, v.z];
        }
    }

    fn movement_speed(&mut self, base: f64, modifiers: &[(u128, f64, u8)]) {
        if let Some(r) = &mut self.rec {
            r.speed_attribute(base, modifiers);
        }
        self.player.apply_speed_attribute(base, modifiers);
    }

    fn status_effects(&mut self, effects: &std::collections::BTreeMap<i8, i8>) {
        if let Some(r) = &mut self.rec {
            r.effects(effects);
        }
        let e = &mut self.player.effects;
        e.jump_boost = effects.get(&8).map(|a| i32::from(*a));
        e.blindness = effects.contains_key(&15);
    }

    fn boots(&mut self, depth_strider: u8, _soul_speed: u8) {
        // Soul speed does not exist in 1.8.
        self.player.depth_strider = depth_strider;
    }

    fn spectator(&mut self, on: bool) {
        self.player.spectator = on;
    }

    fn set_flying(&mut self, flying: bool, out: &mut Vec<Outbound>) {
        let a = &mut self.player.abilities;
        if a.flying == flying || (flying && !a.allow_flying) {
            return;
        }
        a.flying = flying;
        let a = *a;
        if let Some(r) = &mut self.rec {
            r.abilities(&a);
        }
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
        let mut ids: Vec<i32> = self.world.packet_ids().collect();
        // Our own entity data (the flags byte), decoded only when it is ours.
        ids.push(ghast_protocol::v1_8::play::clientbound::id::ENTITY_METADATA);
        ids
    }

    #[cfg(all(test, ghast_modern, feature = "v1_8", feature = "v1_16_5"))]
    fn test_set_own_id(&mut self, id: i32) {
        self.own_id = Some(id);
    }

    fn attack_hurts(&self, e: &ghast_entity::Entity) -> bool {
        use ghast_entity::EntityType as T;
        // 1.8.9 `damage` on the client: `OtherClientPlayerEntity` always, boats and minecarts
        // (`world.isClient` → true), the end crystal, the ghast fireball (an attacker given).
        // LivingEntity is false on the client; paintings / item frames are skipped by
        // `handleAttack`; small fireballs and wither skulls are never damaged.
        match e.entity_type() {
            T::Player => true,
            T::LegacyObject(t) => matches!(t, 1 | 10 | 51 | 63),
            _ => false,
        }
    }

    fn attack_slow(&mut self, knockback: i32, sprint_bonus: bool) {
        let p = &mut self.player;
        if super::attack_knockback_applies(knockback, p.sprinting, sprint_bonus) {
            p.vel[0] *= 0.6;
            p.vel[2] *= 0.6;
            p.set_sprinting(false);
            if let Some(r) = &mut self.rec {
                r.note("attack slow-down not representable");
            }
        }
    }

    fn server_flags(&self) -> Option<u8> {
        self.server_flags
    }

    fn velocity_now(&self) -> Vec3 {
        let v = self.player.vel;
        Vec3::new(v[0], v[1], v[2])
    }

    fn collisions(&self) -> (bool, bool) {
        (
            self.player.horizontal_collision,
            self.player.vertical_collision,
        )
    }

    fn entity_config(&self) -> Option<ghast_entity::Config> {
        self.entities.as_ref().map(|t| t.config())
    }

    fn set_entities(&mut self, config: Option<ghast_entity::Config>) {
        if self.entity_config() == config {
            return;
        }
        self.entities = config.map(|c| {
            let mut t = ghast_entity::v1_8::Tracker::new(c);
            self.session.replay(|r| {
                if let Err(e) = t.apply(r) {
                    log::debug!("entity tracker: session packet 0x{:02x}: {e}", r.id);
                }
            });
            t
        });
    }

    fn entity_packet_ids(&self) -> Vec<i32> {
        self.entities
            .as_ref()
            .map(|t| t.packet_ids().collect())
            .unwrap_or_default()
    }

    fn predict_air(&mut self, x: i32, y: i32, z: i32) {
        self.world
            .set_block_state(x, y, z, ghast_data::v1_8::BlockState::AIR);
    }

    fn place_sent(&mut self, p: &crate::sim::predict::PlaceSent<'_>) -> bool {
        // 1.8.9: single-state full-cube blocks with a plain block item (from the 1.8.9 classes:
        // no block-state properties, no own placement rules); the redstone ores light up when
        // clicked and the listed blocks open a screen or react to the click.
        const PLAIN: &[u16] = &[
            4, 7, 13, 14, 15, 16, 20, 21, 22, 25, 41, 42, 45, 47, 48, 49, 52, 56, 57, 58, 73, 79,
            80, 82, 87, 89, 103, 112, 121, 123, 129, 133, 138, 152, 153, 165, 166, 169, 172, 173,
            174,
        ];
        const ACTIVATED: &[u16] = &[23, 25, 58, 61, 62, 73, 74, 84, 137, 138, 145, 158];
        use ghast_data::v1_8::BlockState;
        let w = &self.world;
        let unit = ghast_data::Aabb {
            min_x: 0.0,
            min_y: 0.0,
            min_z: 0.0,
            max_x: 1.0,
            max_y: 1.0,
            max_z: 1.0,
        };
        let Some((at, raw)) = crate::sim::predict::legacy_plain_placement(
            p,
            PLAIN,
            // logs (4 variants), acacia / dark oak logs (2), hay
            &[(17, 4), (162, 2), (170, 1)],
            ACTIVATED,
            |q| w.block_state(q.x, q.y, q.z).map(BlockState::raw),
            |raw| {
                BlockState::from_raw(raw).is_some_and(|s| {
                    s.block().shape_dependency() == ghast_data::ShapeDependency::None
                        && s.shape() == [unit]
                })
            },
            &self.player.bb,
            &ghast_physics::BlockView::world_border(&self.world),
        ) else {
            return false;
        };
        let Some(s) = BlockState::from_raw(raw) else {
            return false;
        };
        log::debug!("predicted {} at {at:?}", s.block().name());
        self.world.set_block_state(at.x, at.y, at.z, s);
        true
    }

    fn place_suppressed(&self, pos: BlockPos, face: crate::common::Face, held: u16) -> bool {
        use ghast_data::v1_8::{Block, BlockState};
        // Anvils and slab items (stone, wooden, red sandstone): not decided.
        const UNDECIDED: &[u16] = &[44, 126, 145, 182];
        let w = &self.world;
        let unit = ghast_data::Aabb {
            min_x: 0.0,
            min_y: 0.0,
            min_z: 0.0,
            max_x: 1.0,
            max_y: 1.0,
            max_z: 1.0,
        };
        crate::sim::predict::legacy_place_suppressed(
            pos,
            face,
            held,
            UNDECIDED,
            |q| w.block_state(q.x, q.y, q.z).map(BlockState::raw),
            |id| {
                u8::try_from(id)
                    .ok()
                    .and_then(Block::from_id)
                    .is_some_and(|b| b.material().replaceable)
            },
            |id| {
                let Some(b) = u8::try_from(id).ok().and_then(Block::from_id) else {
                    return false;
                };
                b.shape_dependency() == ghast_data::ShapeDependency::None
                    && (0..16u8).all(|m| {
                        let s = BlockState::new(b, m);
                        !s.is_registered() || s.shape() == [unit]
                    })
            },
            &self.player.bb,
            // 1.8 keeps no client positions of other entities (interpolation): not decided.
            |_| false,
        )
    }

    #[cfg(test)]
    fn set_player_pos(&mut self, p: Vec3) {
        self.player.set_position(p.x, p.y, p.z);
    }

    fn held_item(&self, id: u16, count: u8, efficiency: u8) -> HeldItem {
        held_item_facts(DataSet::V1_8, id, count, efficiency)
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
        if let Some(r) = &mut self.rec
            && !r.started()
        {
            r.start_1_8(&self.player);
        }
        if let Some(r) = &mut self.rec {
            let w = &self.world;
            r.observe(self.player.pos, |x, y, z| {
                w.is_column_loaded(x >> 4, z >> 4)
                    .then(|| w.block_state(x, y, z).map_or(0, |s| u32::from(s.raw())))
            });
        }
        let riding = self.ride.is_some();
        if riding {
            if let Some(r) = &mut self.rec {
                r.note_once("riding not representable");
            }
            self.step_ride(&input);
        } else {
            ghast_physics::v1_8::tick(&mut self.player, &input, &self.world, &mut self.packets);
        }
        if self.force_step_position != ForceStepPosition::Off && !riding {
            let p = &mut self.player;
            let cur = [p.pos[0], p.bb.min_y, p.pos[2]];
            force_position(
                &mut self.packets,
                cur,
                &mut p.sent.pos,
                &mut p.sent.ticks_since_pos,
                self.force_step_position,
            );
        }
        if let Some(r) = &mut self.rec {
            r.tick(
                &input,
                &crate::record::state_1_8(&self.player),
                &self.packets,
            );
        }
        push_packets(&self.packets, entity_id, out);
        sync_pose(pose, &self.player);
        // A moving world border advances with the client's clock.
        self.world.advance_time(50);
    }
}

fn sync_pose(pose: &mut Pose, p: &ghast_physics::v1_8::PlayerState) {
    pose.pos = Vec3::new(p.pos[0], p.bb.min_y, p.pos[2]);
    pose.motion = Vec3::new(p.vel[0], p.vel[1], p.vel[2]);
    pose.yaw = p.yaw;
    pose.pitch = p.pitch;
    pose.on_ground = p.on_ground;
    pose.flying = p.abilities.flying;
    pose.sneaking = p.sneaking();
    pose.sprinting = p.sprinting;
}
