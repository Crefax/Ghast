//! 1.12.2: the chunk store and the vanilla-exact physics.

use ghast_protocol::{BlockPos, RawPacket};

use super::*;
use crate::common::{Abilities, Controls, Outbound, Pose, Teleport, Vec3};
use crate::hands::{BlockQuery, HeldItem};

// ---- 1.12.2: ghast-physics --------------------------------------------------------------

/// 1.12 blocks whose client `Block.onUse` answers a right click: the 1.8 list plus the 1.9 – 1.12
/// additions (command blocks, shulker boxes, structure block).
fn activates_1_12(id: u8) -> bool {
    ACTIVATED_1_8.contains(&id) || matches!(id, 137 | 210 | 211 | 219..=234 | 255)
}

/// The 1.12 ray box of a block: the oracle's selection bounds for static shapes; for shapes that
/// follow the neighbours the union of the actual collision boxes (an approximation of vanilla's
/// per-state outline, as for 1.16.5).
fn ray_1_12(w: &ghast_world::v1_12::World, x: i32, y: i32, z: i32) -> Option<[f64; 6]> {
    let s = w.block_state(x, y, z)?;
    let sel = s.selection();
    if !sel.can_collide {
        return None;
    }
    if sel.dependency == ghast_data::ShapeDependency::None {
        let b = sel.bounds;
        return Some([b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z]);
    }
    let local: Vec<ghast_data::Aabb> = ghast_physics::v1_12::collision_boxes(w, x, y, z)
        .iter()
        .map(|b| {
            let (fx, fy, fz) = (f64::from(x), f64::from(y), f64::from(z));
            ghast_data::Aabb::new(
                b.min_x - fx,
                b.min_y - fy,
                b.min_z - fz,
                b.max_x - fx,
                b.max_y - fy,
                b.max_z - fz,
            )
        })
        .collect();
    union(&local).or_else(|| {
        let b = sel.bounds;
        Some([b.min_x, b.min_y, b.min_z, b.max_x, b.max_y, b.max_z])
    })
}

/// 1.12.2: the chunk store and the vanilla-exact physics.
pub(crate) struct Sim12 {
    world: ghast_world::v1_12::World,
    player: ghast_physics::v1_12::PlayerState,
    packets: ghast_physics::Packets,
    force_step_position: ForceStepPosition,
    entities: Option<ghast_entity::v1_12::Tracker>,
    /// Block changes since the last drain (position, version raw state), when collected.
    updates: BlockUpdates,
    /// The session's last join and respawn packets: a tracker started mid-session replays
    /// them to learn our entity id and dimension.
    session: SessionPackets,
    /// The world keeps block entities.
    block_entities: bool,
    /// With entity tracking: the client side of the entities that push the player or are solid
    /// for it, and the teams' collision rules.
    near: Option<Box<Near12>>,
    /// The player's scoreboard name (team rules).
    profile_name: Box<str>,
    /// Entities push the player (`Client::entity_push`).
    entity_push: bool,
    /// Movement state beyond the player: gliding.
    ext: ghast_physics::v1_12::PlayerExt,
    /// Gliding: the server's flag 7 and the firework rockets attached to us.
    glide: Box<Glide12>,
    /// The extension state can act (an elytra, gliding, rockets): the tick takes
    /// [`Sim12::step_ext`] and the rockets tick after it.
    glide_on: bool,
    /// Movement recording ([`crate::Client::record`]).
    rec: Option<crate::record::Recorder>,
    /// The boat we ride and control (`set_passengers` naming us, with entity tracking).
    ride: Option<Box<Ride12>>,
    /// The answer to a server packet that goes out at once (`vehicle_move`).
    reply: Option<Outbound>,
}

/// The vehicle the player rides: its entity id and its client-side state.
#[derive(Debug)]
struct Ride12 {
    id: i32,
    seat: Seat12,
}

/// The boat the client moves, or the minecart the server moves.
#[derive(Debug)]
enum Seat12 {
    Boat(ghast_physics::v1_12::Boat),
    Cart(ghast_physics::v1_12::Minecart),
}

/// What the 1.12.2 glide reads beyond the physics state, decoded once an elytra was seen in the
/// chest slot: our shared flags (the server starts and ends the glide) and the rockets attached
/// to us.
#[derive(Debug, Default)]
struct Glide12 {
    /// Our entity data, rocket spawns and entity removals are read.
    watch: bool,
    /// The packet filter must change ([`Sim::take_filter_dirty`]).
    filter_dirty: bool,
    /// Our entity id (the tick's).
    own_id: i32,
    /// Firework rockets spawned but not (yet) known as attached to us: id, spawn height.
    pending: Vec<(i32, f64)>,
    /// The rockets attached to us, in the client world's entity list order (they tick after
    /// us): id, height (the spawn height, then ours after each of its ticks).
    rockets: Vec<(i32, f64)>,
}

/// The 1.12.2 spawn-object type of a firework rocket.
const FIREWORK_ROCKET_12: i8 = 76;

impl std::fmt::Debug for Sim12 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sim12")
            .field("world", &self.world)
            .field("pos", &self.player.pos)
            .finish_non_exhaustive()
    }
}

impl Sim12 {
    /// Test hook: sets one block.
    #[cfg(all(test, feature = "v1_8", feature = "v1_12_2", feature = "v1_16_5"))]
    pub(crate) fn test_set_block(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        st: ghast_data::v1_12::BlockState,
    ) {
        self.world.set_block_state(x, y, z, st);
    }

    /// Test hook: the player state (from an oracle record).
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) fn test_set_player(&mut self, p: ghast_physics::v1_12::PlayerState) {
        self.player = p;
    }

    /// Test hook: the player's scoreboard name (team rules).
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) fn test_set_profile_name(&mut self, name: &str) {
        self.profile_name = name.into();
    }

    /// Test hook: our flags and rockets are read from now on (as after a jump press in the air
    /// with an elytra on).
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) fn test_watch_glide(&mut self) {
        self.glide.watch = true;
        self.glide.own_id = 1;
    }

    /// Test hook: the bits of our shared flags byte movement reads.
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) fn test_shared_flags_1_12(&self) -> u8 {
        self.ext.shared_flags(&self.player)
    }

    pub(crate) fn new() -> Self {
        Sim12 {
            world: ghast_world::v1_12::World::new(config()),
            player: fresh_player_1_12(),
            packets: ghast_physics::Packets::new(),
            force_step_position: ForceStepPosition::Off,
            entities: None,
            updates: BlockUpdates::default(),
            session: SessionPackets::default(),
            block_entities: false,
            near: None,
            profile_name: "".into(),
            entity_push: true,
            ext: ghast_physics::v1_12::PlayerExt::new(),
            glide: Box::default(),
            glide_on: false,
            rec: None,
            ride: None,
            reply: None,
        }
    }

    /// A sim with options (Grim compatibility, entities, world contents, movement recording).
    pub(crate) fn with(opts: &SimOptions) -> Self {
        let mut s = Self::new();
        s.force_step_position = opts.force_step_position;
        s.entities = opts.entities.map(ghast_entity::v1_12::Tracker::new);
        s.near = s.entities.is_some().then(Box::default);
        if let Some(n) = &opts.profile_name {
            s.profile_name = n.as_str().into();
        }
        if let Some(c) = opts.world {
            s.world = ghast_world::v1_12::World::new(c);
            s.block_entities = c.block_entities;
        }
        if opts.shared_chunks.is_some() {
            log::debug!("shared chunks are not used on 1.12.2; every section stays owned");
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

impl Sim12 {
    /// Recomputes [`Sim12::glide_on`].
    fn update_glide(&mut self) {
        self.glide_on = !self.ext.is_inert() || !self.glide.rockets.is_empty();
    }

    /// The tick with the extension state (gliding, rockets): out of line, so the plain tick
    /// stays as it was.
    #[cold]
    #[inline(never)]
    fn step_ext(&mut self, pose: &Pose, controls: Controls, using_item: bool) {
        let input = ghast_physics::TickInput {
            keys: keys(controls),
            yaw: pose.yaw,
            pitch: pose.pitch,
            using_item,
        };
        let solids: &[ghast_physics::math::Aabb] =
            match (self.near.as_deref_mut(), self.entities.as_ref()) {
                (Some(n), Some(t)) if !n.idle() => {
                    n.solids(t, &self.world, &self.player);
                    &n.solids
                }
                _ => &[],
            };
        ghast_physics::v1_12::tick_ext(
            &mut self.player,
            &mut self.ext,
            &input,
            &self.world,
            &ghast_physics::v1_12::Options::default(),
            solids,
            &mut self.packets,
        );
    }

    /// The attached rockets' ticks after ours (in the world's entity list after the entities
    /// spawned before them; the pushes of other entities come first here).
    #[cold]
    #[inline(never)]
    fn tick_rockets(&mut self, pose: &mut Pose) {
        for r in &mut self.glide.rockets {
            ghast_physics::v1_12::rocket_boost(&mut self.player, &self.ext);
            // Entity.baseTick: below y = -64 the rocket is removed, after this tick.
            if r.1 < -64.0 {
                r.0 = i32::MIN;
            }
            r.1 = self.player.pos[1];
        }
        self.glide.rockets.retain(|r| r.0 != i32::MIN);
        self.update_glide();
        let p = &self.player;
        pose.motion = Vec3::new(p.vel[0], p.vel[1], p.vel[2]);
    }

    /// A rocket spawn, removal or entity data, or our own entity data: decoded only when it
    /// can concern us.
    #[cold]
    #[inline(never)]
    fn glide_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        use ghast_protocol::defs::p340::types::EntityMetadataValue as V;
        use ghast_protocol::v1_12_2::play::clientbound as cb;
        let g = &mut self.glide;
        match raw.id {
            cb::id::SPAWN_ENTITY => {
                let p: cb::SpawnEntity = raw.decode_as()?;
                g.pending.retain(|r| r.0 != p.entity_id);
                g.rockets.retain(|r| r.0 != p.entity_id);
                if p.r#type == FIREWORK_ROCKET_12 {
                    if g.pending.len() >= 32 {
                        g.pending.remove(0);
                    }
                    g.pending.push((p.entity_id, p.y));
                }
            }
            cb::id::ENTITY_DESTROY => {
                let p: cb::EntityDestroy = raw.decode_as()?;
                for id in &p.entity_ids {
                    g.pending.retain(|r| r.0 != *id);
                    g.rockets.retain(|r| r.0 != *id);
                }
            }
            cb::id::ENTITY_METADATA => {
                let mut r = ghast_protocol::Reader::new(&raw.body);
                let id = r.read_varint()?;
                let own = id == g.own_id;
                let pending = g.pending.iter().position(|r| r.0 == id);
                if !own && pending.is_none() {
                    return Ok(());
                }
                let p: cb::EntityMetadata = raw.decode_as()?;
                for e in &p.metadata {
                    match (e.key, &e.value) {
                        // The shared flags: sprinting, fall flying.
                        (0, V::Byte(b)) if own => {
                            self.ext.apply_shared_flags(&mut self.player, *b as u8);
                        }
                        // A rocket's shooter (an entity id, 0 for none).
                        (7, V::VarInt(v)) if !own => {
                            if let Some(k) = pending {
                                let (rid, y) = g.pending.remove(k);
                                if *v == g.own_id {
                                    g.rockets.push((rid, y));
                                    log::debug!("firework rocket {rid} attached to us");
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        self.update_glide();
        Ok(())
    }

    /// The player's tick next to tracked entities: with the boxes of the boats and shulkers
    /// around it. Out of line, so the tick without entities stays the plain one.
    #[cold]
    #[inline(never)]
    fn step_near(&mut self, pose: &Pose, controls: Controls, using_item: bool) {
        let input = ghast_physics::TickInput {
            keys: keys(controls),
            yaw: pose.yaw,
            pitch: pose.pitch,
            using_item,
        };
        let (Some(n), Some(t)) = (self.near.as_deref_mut(), self.entities.as_ref()) else {
            return;
        };
        n.solids(t, &self.world, &self.player);
        ghast_physics::v1_12::tick_in(
            &mut self.player,
            &input,
            &self.world,
            &ghast_physics::v1_12::Options::default(),
            &n.solids,
            &mut self.packets,
        );
    }

    /// A tick in the boat we control (`ghast_physics::v1_12::tick_boat`), at the boat's place
    /// in the client world's entity list: the tracked entities before it step first (their
    /// interpolation, the lily pads their boats break), then the boat collides with the
    /// entities around it as they are then and the passenger with the boats and shulkers.
    /// Their pushes on the passenger are not applied (its motion is zeroed before its next
    /// tick, they change no packet).
    #[cold]
    #[inline(never)]
    fn step_ride(&mut self, pose: &Pose, controls: Controls) {
        let Some(r) = self.ride.as_deref_mut() else {
            return;
        };
        let id = r.id;
        let Seat12::Boat(boat) = &mut r.seat else {
            return;
        };
        let input = ghast_physics::TickInput {
            keys: keys(controls),
            yaw: pose.yaw,
            pitch: pose.pitch,
            // The passenger tick has no item slowdown.
            using_item: false,
        };
        let (solids, boat_solids): (&[_], &[_]) =
            match (self.near.as_deref_mut(), self.entities.as_ref()) {
                (Some(n), Some(t)) => {
                    if let Some(&k) = n.index.get(&id) {
                        let seq = n.list[k as usize].seq;
                        n.tick(t, &self.world, &self.player.bb, false, "", 0..seq);
                        for &[x, y, z] in &n.lily_pads {
                            self.world
                                .set_block_state(x, y, z, ghast_data::v1_12::BlockState::AIR);
                        }
                    }
                    n.solids(t, &self.world, &self.player);
                    // As far as the boat can move this tick (its velocity grows by at most
                    // the paddle and the flow push before the move).
                    let b = &*boat;
                    let reach = |v: f64| v.abs() + 3.0;
                    n.boat_solids(
                        t,
                        &self.world,
                        &b.bb,
                        [reach(b.vel[0]), reach(b.vel[1]), reach(b.vel[2])],
                    );
                    (&n.solids, &n.boat_solids)
                }
                _ => (&[], &[]),
            };
        let still = ghast_physics::v1_12::tick_boat(
            &mut self.player,
            &mut self.ext,
            boat,
            solids,
            boat_solids,
            &input,
            &self.world,
            &mut self.packets,
        );
        for &[x, y, z] in boat.broken_lily_pads() {
            self.world
                .set_block_state(x, y, z, ghast_data::v1_12::BlockState::AIR);
        }
        let (pos, bb) = (boat.pos, boat.bb);
        if let Some(n) = self.near.as_deref_mut()
            && let Some(c) = n.get_mut(id)
        {
            // The entry follows the client's boat (its interpolation stays where it was).
            c.follow(pos, bb);
        }
        if !still {
            log::debug!("boat {id} fell out of the world");
            self.leave_boat();
        }
    }

    /// A tick in the minecart we sit in (`ghast_physics::v1_12::tick_minecart`), at the cart's
    /// place in the client world's entity list (the entities before it step first).
    #[cold]
    #[inline(never)]
    fn step_cart(&mut self, pose: &Pose, controls: Controls) {
        let Some(r) = self.ride.as_deref_mut() else {
            return;
        };
        let id = r.id;
        let Seat12::Cart(cart) = &mut r.seat else {
            return;
        };
        let input = ghast_physics::TickInput {
            keys: keys(controls),
            yaw: pose.yaw,
            pitch: pose.pitch,
            // The passenger tick has no item slowdown.
            using_item: false,
        };
        let solids: &[_] = match (self.near.as_deref_mut(), self.entities.as_ref()) {
            (Some(n), Some(t)) => {
                if let Some(&k) = n.index.get(&id) {
                    let seq = n.list[k as usize].seq;
                    n.tick(t, &self.world, &self.player.bb, false, "", 0..seq);
                    for &[x, y, z] in &n.lily_pads {
                        self.world
                            .set_block_state(x, y, z, ghast_data::v1_12::BlockState::AIR);
                    }
                }
                n.solids(t, &self.world, &self.player);
                &n.solids
            }
            _ => &[],
        };
        ghast_physics::v1_12::tick_minecart(
            &mut self.player,
            &mut self.ext,
            cart,
            solids,
            &input,
            &self.world,
            &mut self.packets,
        );
        let (pos, bb) = (cart.pos, cart.bb);
        if let Some(n) = self.near.as_deref_mut()
            && let Some(c) = n.get_mut(id)
        {
            c.follow(pos, bb);
        }
    }

    /// The server's moves of the minecart we sit in (`onEntityPosition` / `onEntityUpdate`; the
    /// entity list follows it, see [`Near12::ridden`]).
    #[cold]
    #[inline(never)]
    fn cart_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        use ghast_protocol::v1_12_2::play::clientbound as cb;
        let Some(Ride12 {
            id,
            seat: Seat12::Cart(c),
        }) = self.ride.as_deref_mut()
        else {
            return Ok(());
        };
        let id = *id;
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
                    self.leave_boat();
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The player is off the boat it drove: the boat stays where it is on the client.
    fn leave_boat(&mut self) {
        self.ride = None;
        if let Some(n) = self.near.as_deref_mut() {
            n.ridden = None;
        }
    }

    /// Our entity id: the tracker's, else the join packet's.
    fn own_id(&self) -> i32 {
        self.entities
            .as_ref()
            .and_then(|t| t.self_id())
            .unwrap_or(self.glide.own_id)
    }

    /// `set_passengers` (boarding / leaving a boat) and `vehicle_move` (the server's correction
    /// of the boat we control: `onVehicleMove` applies it and answers with the boat's position at
    /// once).
    #[cold]
    #[inline(never)]
    fn vehicle_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        use ghast_protocol::v1_12_2::play::clientbound as cb;
        if raw.id == cb::id::VEHICLE_MOVE {
            if let Some(Ride12 {
                id: rid,
                seat: Seat12::Boat(boat),
            }) = self.ride.as_deref_mut()
            {
                let rid = *rid;
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
                let (id, pos, bb) = (rid, boat.pos, boat.bb);
                if let Some(n) = self.near.as_deref_mut()
                    && let Some(c) = n.get_mut(id)
                {
                    c.follow(pos, bb);
                }
            }
            return Ok(());
        }
        let p: cb::SetPassengers = raw.decode_as()?;
        let on = p.passengers.contains(&self.own_id());
        let riding_it = self.ride.as_ref().is_some_and(|r| r.id == p.entity_id);
        if on && !riding_it {
            // A minecart: at its client position (the server's when it is not followed), with
            // the server's position of it for the relative moves.
            let cart = self.entities.as_ref().and_then(|t| {
                let e = t.get(p.entity_id)?;
                if e.entity_type() != ghast_entity::EntityType::LegacyObject(10) {
                    return None;
                }
                let pos = self
                    .near
                    .as_deref()
                    .and_then(|n| n.index.get(&p.entity_id).map(|&k| n.list[k as usize].pos))
                    .unwrap_or(e.position());
                let mut c = ghast_physics::v1_12::Minecart::new(pos, e.yaw(), e.pitch());
                c.tracked = e.position().map(|v| (v * 4096.0).floor() as i64);
                Some(c)
            });
            if let Some(cart) = cart {
                log::debug!("in minecart {}", p.entity_id);
                self.ride = Some(Box::new(Ride12 {
                    id: p.entity_id,
                    seat: Seat12::Cart(cart),
                }));
                if let Some(n) = self.near.as_deref_mut() {
                    n.ridden = Some(p.entity_id);
                }
                return Ok(());
            }
            // startRiding a boat the client knows: at its client position, with the rotation the
            // server last sent. Other vehicles are not simulated (the walking tick goes on).
            let boat = self.entities.as_ref().and_then(|t| {
                let e = t.get(p.entity_id)?;
                if e.entity_type() != ghast_entity::EntityType::LegacyObject(1) {
                    return None;
                }
                let pos = self
                    .near
                    .as_deref()
                    .and_then(|n| n.index.get(&p.entity_id).map(|&k| n.list[k as usize].pos))
                    .unwrap_or(e.position());
                Some(ghast_physics::v1_12::Boat::new(pos, e.yaw(), 0.0))
            });
            match boat {
                Some(boat) => {
                    log::debug!("riding boat {}", p.entity_id);
                    self.ride = Some(Box::new(Ride12 {
                        id: p.entity_id,
                        seat: Seat12::Boat(boat),
                    }));
                    if let Some(n) = self.near.as_deref_mut() {
                        n.ridden = Some(p.entity_id);
                    }
                }
                None => log::debug!(
                    "riding {} (not simulated: no entity tracking or not a boat)",
                    p.entity_id
                ),
            }
        } else if !on && riding_it {
            log::debug!("left boat {}", p.entity_id);
            self.leave_boat();
        }
        Ok(())
    }

    /// `World.tickEntities` after the player: the entities interpolate, the boats break lily
    /// pads, then they push the player. In a boat we control, only the entities after the boat
    /// in the entity list (the ones before it ticked before it, [`Sim12::step_ride`]).
    #[cold]
    #[inline(never)]
    fn tick_near(&mut self, pose: &mut Pose) {
        let (Some(n), Some(t)) = (self.near.as_deref_mut(), self.entities.as_ref()) else {
            return;
        };
        // The destroyed boats have left their chunks' entity lists.
        n.ghosts.clear();
        if n.list.is_empty() {
            return;
        }
        let seqs = match n.ridden.and_then(|id| n.index.get(&id)) {
            Some(&k) => n.list[k as usize].seq + 1..u64::MAX,
            None => 0..u64::MAX,
        };
        let p = &mut self.player;
        // Only a ticking player is in its chunk's entity list, where the pushers find it; the
        // bot riding a vehicle the client does not move or carrying something is not pushed
        // (not modelled). In the boat we control, the entities after the boat push the
        // passenger: its next tick in the boat zeroes that motion, a walking one (the server
        // took it off in between) starts with it.
        let riding_ours = n.ridden.is_some();
        let pushing = self.entity_push
            && p.added_to_chunk
            && !p.removed
            && !t.me().is_some_and(|m| {
                (m.vehicle().is_some() && !riding_ours) || !m.passengers().is_empty()
            });
        n.tick(t, &self.world, &p.bb, pushing, &self.profile_name, seqs);
        let mut changed = false;
        for (_, pusher) in &n.pushers {
            changed |= ghast_physics::v1_12::push_by(p, &self.world, pusher);
        }
        if changed {
            pose.motion = Vec3::new(p.vel[0], p.vel[1], p.vel[2]);
        }
        // A boat's tick breaks the lily pads it touches in the client's world.
        for &[x, y, z] in &n.lily_pads {
            self.world
                .set_block_state(x, y, z, ghast_data::v1_12::BlockState::AIR);
        }
    }
}

/// A fresh 1.12 client player: not yet in a chunk (the first tick with its column loaded adds
/// it, the next one moves it), as vanilla.
fn fresh_player_1_12() -> ghast_physics::v1_12::PlayerState {
    let mut p = ghast_physics::v1_12::PlayerState::new([0.0; 3], 0.0, 0.0);
    p.added_to_chunk = false;
    p
}

impl WorldView for Sim12 {
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
        Some(BlockQuery {
            exact: Some(crate::hands::ExactDig::V1_12),
            air: b.material().kind == ghast_data::v1_12::MaterialKind::Air,
            water: matches!(b.id(), 8 | 9),
            block: u16::from(b.id()),
            hardness: info.hardness.unwrap_or(-1.0),
            dig_material: info.material,
            harvest_tools: info.harvest_tools,
            selection: ray_1_12(&self.world, x, y, z),
            stairs: None,
            activates: activates_1_12(b.id()),
        })
    }

    fn crosshair(
        &self,
        eyes: Vec3,
        (yaw, pitch): (f32, f32),
        _look: Vec3,
        reach: f64,
    ) -> Option<crate::BlockHit> {
        // The oracle-verified 1.12 pick (ghast_physics::pick::v1_12: vanilla's ray shapes,
        // Entity.rayTrace with the float look vector).
        use ghast_physics::pick::v1_12 as pick;
        let eye = [eyes.x, eyes.y, eyes.z];
        let hit = pick::pick(&self.world, eye, pick::look_vector(yaw, pitch), reach)?;
        block_hit(&hit, eye)
    }
}

impl Sim for Sim12 {
    fn crosshair_target(&self, q: &PickQuery<'_>) -> Crosshair {
        // The oracle-verified GameRenderer.updateTargetedEntity: block ray at the game
        // mode's reach, then the entity ray.
        use ghast_physics::pick::v1_12 as pick;
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
            use ghast_protocol::v1_12_2::play::clientbound::id;
            self.session.note(raw, id::LOGIN, id::RESPAWN);
            if self.glide.watch
                && matches!(
                    raw.id,
                    id::SPAWN_ENTITY | id::ENTITY_DESTROY | id::ENTITY_METADATA
                )
            {
                self.glide_packet(raw)?;
            }
        }
        if let Some(t) = &mut self.entities
            && t.wants(raw.id)
        {
            t.apply(raw)?;
            if let Some(n) = &mut self.near {
                n.packet(t, raw);
            }
        }
        use ghast_protocol::v1_12_2::play::clientbound as cb;
        if matches!(raw.id, cb::id::SET_PASSENGERS | cb::id::VEHICLE_MOVE) {
            return self.vehicle_packet(raw);
        }
        if matches!(
            raw.id,
            cb::id::ENTITY_TELEPORT
                | cb::id::REL_ENTITY_MOVE
                | cb::id::ENTITY_MOVE_LOOK
                | cb::id::ENTITY_LOOK
                | cb::id::ENTITY
                | cb::id::ENTITY_DESTROY
        ) && matches!(
            self.ride.as_deref(),
            Some(Ride12 {
                seat: Seat12::Cart(_),
                ..
            })
        ) {
            self.cart_packet(raw)?;
        }
        if raw.id == cb::id::TEAMS {
            if let Some(n) = &mut self.near
                && !n
                    .teams
                    .apply(&raw.body, super::teams::TeamLayout::Legacy1_12)
            {
                log::debug!("bad teams packet ({} B)", raw.body.len());
            }
            return Ok(());
        }
        if let Some(n) = &mut self.near
            && matches!(
                raw.id,
                cb::id::MAP_CHUNK
                    | cb::id::BLOCK_CHANGE
                    | cb::id::MULTI_BLOCK_CHANGE
                    | cb::id::LOGIN
                    | cb::id::RESPAWN
            )
        {
            n.world_version = n.world_version.wrapping_add(1);
        }
        let w = &mut self.world;
        match raw.id {
            cb::id::LOGIN => {
                self.ride = None;
                let p: cb::Login = raw.decode_as()?;
                // Our entity id (our entity data names us by it).
                self.glide.own_id = p.entity_id;
                w.apply_login(&p);
            }
            cb::id::RESPAWN => {
                self.ride = None;
                w.apply_respawn(&raw.decode_as()?);
            }
            cb::id::MAP_CHUNK => {
                w.apply_map_chunk(&raw.decode_as()?)?;
            }
            cb::id::UNLOAD_CHUNK => {
                w.apply_unload_chunk(&raw.decode_as()?);
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
            cb::id::TILE_ENTITY_DATA if self.block_entities => {
                w.apply_tile_entity_data(&raw.decode_as()?);
            }
            _ => {}
        }
        Ok(())
    }

    fn reset_player(&mut self) {
        let abilities = self.player.abilities;
        let food = self.player.food_level;
        self.player = fresh_player_1_12();
        self.player.abilities = abilities;
        self.player.food_level = food;
        // A new client player: not gliding, no rockets, not riding.
        self.leave_boat();
        self.ext.fall_flying = false;
        self.glide.rockets.clear();
        self.glide.pending.clear();
        self.update_glide();
    }

    fn teleport(&mut self, pose: &mut Pose, t: &Teleport) {
        // The engine keeps the look (its rotation may have moved it since the last tick).
        self.player.yaw = pose.yaw;
        self.player.pitch = pose.pitch;
        let _reply =
            self.player
                .apply_teleport([t.x, t.y, t.z], t.yaw, t.pitch, rel_flags(t.flags));
        if let Some(r) = &mut self.rec {
            let p = &self.player;
            r.teleport(
                [t.x, t.y, t.z],
                t.yaw,
                t.pitch,
                t.flags,
                t.teleport_id,
                p.pos,
                p.yaw,
                p.pitch,
            );
        }
        sync_pose_12(pose, &self.player);
    }

    fn abilities(&mut self, a: Abilities) {
        let a = physics_abilities(a);
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
        e.levitation = effects.get(&25).map(|a| i32::from(*a));
    }

    fn boots(&mut self, depth_strider: u8, _soul_speed: u8) {
        // Soul speed does not exist in 1.12.
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
        use ghast_protocol::v1_12_2::play::clientbound as cb;
        let mut ids: Vec<i32> = self.world.packet_ids().collect();
        // Riding: boarding and the server's boat corrections.
        ids.extend([cb::id::SET_PASSENGERS, cb::id::VEHICLE_MOVE]);
        if self.glide.watch {
            // Gliding: the server's flag 7 (our entity data) and the rockets it attaches to us.
            ids.extend([
                cb::id::SPAWN_ENTITY,
                cb::id::ENTITY_DESTROY,
                cb::id::ENTITY_METADATA,
            ]);
        }
        ids
    }

    fn take_filter_dirty(&mut self) -> bool {
        std::mem::take(&mut self.glide.filter_dirty)
    }

    fn mount_rules(&self, entity_id: i32) -> Option<ghast_inventory::MountRules> {
        // HorseScreenHandler: the saddle slot asks canBeSaddled (every mount but llamas), the
        // armor slot isArmor (horse armor for horses, carpets for llamas): the type decides.
        use ghast_inventory::{MountArmor, MountRules};
        let e = self.entities.as_ref()?.get(entity_id)?;
        Some(match ghast_entity::v1_12::info(e.entity_type())?.name {
            "horse" => MountRules::new(true, MountArmor::HorseArmor),
            "donkey" | "mule" | "zombie_horse" | "skeleton_horse" => {
                MountRules::new(true, MountArmor::None)
            }
            "llama" => MountRules::new(false, MountArmor::Carpet),
            _ => return None,
        })
    }

    fn take_reply(&mut self) -> Option<Outbound> {
        self.reply.take()
    }

    fn vehicle(&self) -> Option<crate::common::Vehicle> {
        let r = self.ride.as_deref()?;
        let b = match &r.seat {
            Seat12::Boat(b) => b,
            Seat12::Cart(c) => {
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

    #[inline(always)]
    fn modern_inputs(&self) -> bool {
        // The elytra is read on jump presses in the air (the request needs that).
        !self.player.on_ground
    }

    fn pre_tick(&mut self, inv: &ghast_inventory::Inventory, t: &crate::sim::PreTick) -> bool {
        if t.controls.jump {
            // ElytraItem.isUsable: damage below 431 (elytra, item 443).
            self.ext.elytra = inv.armor(1).is_some_and(|s| {
                s.id() == 443 && s.damage_value(ghast_inventory::Version::V1_12) < 432 - 1
            });
            if self.ext.elytra && !self.glide.watch {
                self.glide.watch = true;
                self.glide.filter_dirty = true;
            }
            self.update_glide();
        }
        true
    }

    fn gliding(&self) -> bool {
        self.ext.fall_flying
    }

    fn entity_config(&self) -> Option<ghast_entity::Config> {
        self.entities.as_ref().map(|t| t.config())
    }

    fn set_entities(&mut self, config: Option<ghast_entity::Config>) {
        if self.entity_config() == config {
            return;
        }
        self.entities = config.map(|c| {
            let mut t = ghast_entity::v1_12::Tracker::new(c);
            self.session.replay(|r| {
                if let Err(e) = t.apply(r) {
                    log::debug!("entity tracker: session packet 0x{:02x}: {e}", r.id);
                }
            });
            t
        });
        // The tracker starts empty (entities spawned before are known once the server resends
        // them), and teams sent before are not known (all pushes allowed).
        self.near = self.entities.is_some().then(Box::default);
    }

    fn entity_packet_ids(&self) -> Vec<i32> {
        use ghast_protocol::v1_12_2::play::clientbound as cb;
        self.entities
            .as_ref()
            .map(|t| t.packet_ids().chain([cb::id::TEAMS]).collect())
            .unwrap_or_default()
    }

    fn tick_entities(&mut self, pose: &mut Pose) {
        if self.near.is_some() {
            self.tick_near(pose);
        }
        if self.glide_on && !self.glide.rockets.is_empty() {
            self.tick_rockets(pose);
        }
    }

    fn set_entity_push(&mut self, on: bool) {
        self.entity_push = on;
    }

    fn predict_air(&mut self, x: i32, y: i32, z: i32) {
        self.world
            .set_block_state(x, y, z, ghast_data::v1_12::BlockState::AIR);
    }

    fn place_sent(&mut self, p: &crate::sim::predict::PlaceSent<'_>) {
        // 1.12.2: single-state full-cube blocks with a plain block item (from the 1.12.2 classes:
        // no block-state properties, no own placement rules); the redstone ores light up when
        // clicked and the listed blocks open a screen or react to the click.
        const PLAIN: &[u16] = &[
            4, 7, 13, 14, 15, 16, 20, 21, 22, 25, 41, 42, 45, 47, 48, 49, 52, 56, 57, 58, 73, 79,
            80, 82, 87, 89, 103, 112, 121, 123, 129, 133, 138, 152, 153, 165, 166, 169, 172, 173,
            174, 201, 206, 213, 214, 215,
        ];
        const ACTIVATED: &[u16] = &[
            23, 25, 58, 61, 62, 73, 74, 84, 137, 138, 145, 158, 210, 211, 219, 220, 221, 222, 223,
            224, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 255,
        ];
        use ghast_data::v1_12::BlockState;
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
            return;
        };
        if let Some(s) = BlockState::from_raw(raw) {
            log::debug!("predicted {} at {at:?}", s.block().name());
            self.world.set_block_state(at.x, at.y, at.z, s);
        }
    }

    fn place_suppressed(&self, pos: BlockPos, face: crate::common::Face, held: u16) -> bool {
        use ghast_data::v1_12::{Block, BlockState};
        // Anvils and slab items (stone, wooden, red sandstone, purpur): not decided.
        const UNDECIDED: &[u16] = &[44, 126, 145, 182, 205];
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
            // World.hasEntityIn (spectators left out by getEntitiesIn): the entities whose
            // client position the sim keeps (with entity tracking). Not decided: the others
            // (minecarts, armor stands, bats, parrots, TNT, falling blocks, riders).
            |cube| match (self.near.as_deref(), self.entities.as_ref()) {
                (Some(n), Some(t)) => n.blocks_building(t, &self.world, cube),
                _ => false,
            },
        )
    }

    #[cfg(test)]
    fn set_player_pos(&mut self, p: Vec3) {
        self.player.set_position(p.x, p.y, p.z);
    }

    fn held_item(&self, id: u16, count: u8, efficiency: u8) -> HeldItem {
        held_item_facts(DataSet::V1_12, id, count, efficiency)
    }

    fn step(
        &mut self,
        pose: &mut Pose,
        controls: Controls,
        using_item: bool,
        entity_id: i32,
        out: &mut Vec<Outbound>,
    ) {
        self.packets.clear();
        if let Some(r) = &mut self.rec {
            // From the first tick the player really ticks: until its column is there it is not
            // added to the world (no move, no packet), which the replay's setup cannot express.
            if !r.started() && self.player.added_to_chunk {
                r.start_1_12(&self.player);
            }
            let w = &self.world;
            r.observe(self.player.pos, |x, y, z| {
                w.is_column_loaded(x >> 4, z >> 4)
                    .then(|| w.block_state(x, y, z).map_or(0, |s| u32::from(s.raw())))
            });
            if self.ride.is_some() {
                r.note_once("riding not representable");
            } else if self.glide_on {
                r.note_once("gliding not representable");
            } else if self.near.as_deref().is_some_and(|n| !n.idle()) {
                r.note_once("entity pushes and collisions not representable");
            }
        }
        let riding = self.ride.is_some();
        if riding {
            if matches!(
                self.ride.as_deref(),
                Some(Ride12 {
                    seat: Seat12::Cart(_),
                    ..
                })
            ) {
                self.step_cart(pose, controls);
            } else {
                self.step_ride(pose, controls);
            }
        } else if self.glide_on {
            self.step_ext(pose, controls, using_item);
        } else if self.near.as_deref().is_some_and(|n| !n.idle()) {
            self.step_near(pose, controls, using_item);
        } else {
            let input = ghast_physics::TickInput {
                keys: keys(controls),
                yaw: pose.yaw,
                pitch: pose.pitch,
                using_item,
            };
            ghast_physics::v1_12::tick(&mut self.player, &input, &self.world, &mut self.packets);
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
            let input = ghast_physics::TickInput {
                keys: keys(controls),
                yaw: pose.yaw,
                pitch: pose.pitch,
                using_item,
            };
            r.tick(
                &input,
                &crate::record::state_1_12(&self.player),
                &self.packets,
            );
        }
        push_packets(&self.packets, entity_id, out);
        sync_pose_12(pose, &self.player);
        // A moving world border advances with the client's clock.
        self.world.advance_time(50);
    }
}

fn sync_pose_12(pose: &mut Pose, p: &ghast_physics::v1_12::PlayerState) {
    pose.pos = Vec3::new(p.pos[0], p.bb.min_y, p.pos[2]);
    pose.motion = Vec3::new(p.vel[0], p.vel[1], p.vel[2]);
    pose.yaw = p.yaw;
    pose.pitch = p.pitch;
    pose.on_ground = p.on_ground;
    pose.flying = p.abilities.flying;
    pose.sneaking = p.sneaking();
    pose.sprinting = p.sprinting;
}

// ---- 1.12.2: entities pushing the player, entities it collides with --------------------------

/// What a tracked entity does to the 1.12 player on the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class12 {
    /// A living entity: `tickCramming` pushes the player; interpolates over 3 ticks.
    Living,
    /// Another player: pushes like a living entity unless it is a spectator.
    Player,
    /// A boat: pushes from its tick, solid for the player; interpolates over 10 ticks.
    Boat,
    /// A shulker: pushes like a living entity, solid while alive; never interpolates (the
    /// client snaps it to its attached block).
    Shulker,
    /// A minecart (every kind): only blocks a boat the player rides; interpolates over 5 ticks.
    Cart,
    /// A parrot: only blocks a boat the player rides; interpolates over 3 ticks.
    Parrot,
}

impl Class12 {
    /// Only matters for a boat the player rides (not pushing, not solid for the player).
    fn boat_only(self) -> bool {
        matches!(self, Class12::Cart | Class12::Parrot)
    }
}

/// The 1.12 class of a tracked entity, `None` when it neither pushes nor collides nor blocks a
/// boat: bats, armor stands, the ender dragon, items, projectiles, …
fn class_12(e: &ghast_entity::Entity) -> Option<Class12> {
    use ghast_entity::EntityType as T;
    match e.entity_type() {
        T::Player => Some(Class12::Player),
        // Spawn-object type 1.
        T::LegacyObject(1) => Some(Class12::Boat),
        T::LegacyObject(10) => Some(Class12::Cart),
        t @ T::LegacyMob(_) => match ghast_entity::v1_12::info(t)?.name {
            "shulker" => Some(Class12::Shulker),
            "parrot" => Some(Class12::Parrot),
            "bat" | "armor_stand" | "ender_dragon" => None,
            _ => Some(Class12::Living),
        },
        _ => None,
    }
}

/// How far from the player boats break lily pads in the client's world here (vanilla: every
/// ticking boat; the server breaks them as well and sends the change).
const LILY_PAD_REACH: f64 = 16.0;

/// The client side of one entity that pushes the player or is solid for it.
#[derive(Clone, Debug)]
struct Client12 {
    id: i32,
    /// Order in the client's entity list (spawn order).
    seq: u64,
    class: Class12,
    /// Client position (`x`, `y`, `z` after the interpolation steps so far; a shulker's is
    /// the server's, see [`Client12::placed`]).
    pos: [f64; 3],
    /// Interpolation target and remaining steps (`serverX`… / `bodyTrackingIncrements`, the
    /// boat's own fields).
    target: [f64; 3],
    steps: u8,
    /// Size (from the tracker at spawn and after each metadata packet).
    dims: ghast_data::EntityDimensions,
    /// Position and box as the client has them now (from `pos`).
    at: [f64; 3],
    bb: ghast_physics::math::Aabb,
    /// A player's profile name from its tab-list entry at spawn (`OtherClientPlayerEntity`
    /// keeps that profile: its scoreboard name for the team rules).
    name: Option<Box<str>>,
    /// A boat: the box and world version of its last lily pad scan (a boat that has not moved
    /// in an unchanged world breaks nothing new).
    scanned: Option<(ghast_physics::math::Aabb, u32)>,
}

impl Client12 {
    /// Recomputes [`Client12::at`] / [`Client12::bb`] from the position.
    fn placed(&mut self) {
        if self.class == Class12::Shulker {
            // ShulkerEntity.tick snaps it to its attached block (the server keeps that block
            // under the position it sends): the unit cube there.
            let [x, y, z] = self.pos;
            let (x, y, z) = (x.floor() + 0.5, y.floor(), z.floor() + 0.5);
            self.at = [x, y, z];
            self.bb =
                ghast_physics::math::Aabb::new(x - 0.5, y, z - 0.5, x + 0.5, y + 1.0, z + 0.5);
        } else {
            self.at = self.pos;
            self.bb = self
                .dims
                .bounding_box(self.pos[0], self.pos[1], self.pos[2]);
        }
    }

    /// The boat the client moves: its position and box as the move left them (the box is
    /// not the one its position would give).
    fn follow(&mut self, pos: [f64; 3], bb: ghast_physics::math::Aabb) {
        self.pos = pos;
        self.at = pos;
        self.bb = bb;
    }

    /// `updateTrackedPositionAndAngles` with the client's step count: 10 for boats, 3 else.
    /// A shulker ignores it; the client snaps it to its attached block, which the server keeps
    /// under the position it sends (so it follows that position here).
    fn interpolate_to(&mut self, target: [f64; 3]) {
        match self.class {
            Class12::Shulker => {
                self.pos = target;
                self.target = target;
                self.placed();
            }
            Class12::Boat => {
                self.target = target;
                self.steps = 10;
            }
            Class12::Living | Class12::Player | Class12::Parrot => {
                self.target = target;
                self.steps = 3;
            }
            // AbstractMinecartEntity.updateTrackedPositionAndAngles: the packet's 3 steps + 2.
            Class12::Cart => {
                self.target = target;
                self.steps = 5;
            }
        }
    }

    /// One interpolation step of the entity's client tick.
    #[inline]
    fn step(&mut self) {
        if self.steps > 0 {
            let n = f64::from(self.steps);
            for k in 0..3 {
                self.pos[k] += (self.target[k] - self.pos[k]) / n;
            }
            self.steps -= 1;
            self.placed();
        }
    }
}

/// `true` when box `b` is farther than `r` from the box `p`.
#[inline]
fn far(b: &ghast_physics::math::Aabb, p: &ghast_physics::math::Aabb, r: [f64; 3]) -> bool {
    b.max_x < p.min_x - r[0]
        || b.min_x > p.max_x + r[0]
        || b.max_y < p.min_y - r[1]
        || b.min_y > p.max_y + r[1]
        || b.max_z < p.min_z - r[2]
        || b.min_z > p.max_z + r[2]
}

/// 1.12 entities around the player as the client moves them: positions after the client's
/// interpolation (the tracker keeps the server's), pushes after the player's tick
/// (`World.tickEntities`: entities spawned after the player tick after it) and the boxes it
/// collides with (`World.doesBoxCollide`). Riding (the bot or the other entity) is not
/// modelled: such entities neither push nor collide here.
#[derive(Debug, Default)]
pub(crate) struct Near12 {
    /// Every tracked entity that pushes or collides (dense; `index` finds one by id).
    list: Vec<Client12>,
    index: std::collections::HashMap<i32, u32>,
    seq: u64,
    /// Boats in `list`.
    boats: usize,
    /// Entries in `list` that only matter for a ridden boat ([`Class12::boat_only`]).
    boat_only: usize,
    /// Boats and minecarts destroyed since the player's last tick: vanilla's world keeps them in
    /// their chunk's entity list until its entity loop reaches them, after the player moved.
    ghosts: Vec<Ghost12>,
    /// The boat the player rides and the client moves ([`Sim12`]'s): its entry follows the
    /// client's boat, the server's moves of it are ignored (`onEntityPosition` /
    /// `onEntityUpdate` skip an entity the client controls) and it is left out of the boxes.
    pub(super) ridden: Option<i32>,
    /// Scratch: the boxes the ridden boat collides with.
    boat_solids: Vec<ghast_physics::math::Aabb>,
    /// Scratch: this tick's solid boxes, pushers and broken lily pads.
    solids: Vec<ghast_physics::math::Aabb>,
    pushers: Vec<(u64, ghast_physics::v1_12::Pusher)>,
    lily_pads: Vec<[i32; 3]>,
    /// Bumped by every block packet (lily pads may have appeared).
    pub(super) world_version: u32,
    /// The teams' collision rules (`teams` packets).
    pub(super) teams: super::teams::Teams,
}

/// A destroyed boat or minecart still in its chunk's entity list (see [`Near12::ghosts`]).
#[derive(Clone, Copy, Debug)]
struct Ghost12 {
    /// Its place in the entity list (it leaves the chunk when the loop reaches it).
    seq: u64,
    bb: ghast_physics::math::Aabb,
    /// A minecart (blocks a ridden boat only), else a boat.
    cart: bool,
}

/// The first VarInt of a body (the entity id of spawn and move packets).
fn first_varint(body: &[u8]) -> Option<i32> {
    let mut v: u32 = 0;
    for (k, &b) in body.iter().take(5).enumerate() {
        v |= u32::from(b & 0x7f) << (7 * k);
        if b & 0x80 == 0 {
            return Some(v as i32);
        }
    }
    None
}

impl Near12 {
    /// Nothing that pushes the walking player or is solid for it.
    fn idle(&self) -> bool {
        self.list.len() == self.boat_only && self.ghosts.iter().all(|g| g.cart)
    }

    fn get_mut(&mut self, id: i32) -> Option<&mut Client12> {
        let &k = self.index.get(&id)?;
        self.list.get_mut(k as usize)
    }

    /// Follows one entity packet the tracker has just applied.
    fn packet(&mut self, t: &ghast_entity::v1_12::Tracker, raw: &RawPacket) {
        use ghast_protocol::v1_12_2::play::clientbound::id;
        match raw.id {
            id::LOGIN | id::RESPAWN => {
                self.list.clear();
                self.index.clear();
                self.boats = 0;
                self.boat_only = 0;
                self.ghosts.clear();
                self.ridden = None;
                if raw.id == id::LOGIN {
                    // A new play phase: a new client scoreboard.
                    self.teams.clear();
                }
            }
            id::SPAWN_ENTITY | id::SPAWN_ENTITY_LIVING | id::NAMED_ENTITY_SPAWN => {
                let Some(eid) = first_varint(&raw.body) else {
                    return;
                };
                // ClientWorld.addEntity: an entity with the same id leaves the world first.
                self.remove(eid);
                if let Some(e) = t.get(eid)
                    && let Some(class) = class_12(e)
                    && let Some(dims) = e.dimensions()
                {
                    self.seq += 1;
                    self.boats += usize::from(class == Class12::Boat);
                    self.boat_only += usize::from(class.boat_only());
                    let pos = e.position();
                    let mut c = Client12 {
                        id: eid,
                        seq: self.seq,
                        class,
                        pos,
                        target: pos,
                        steps: 0,
                        dims,
                        at: pos,
                        bb: ghast_physics::math::Aabb::FULL,
                        name: t.player_name(eid).map(Into::into),
                        scanned: None,
                    };
                    c.placed();
                    self.index.insert(eid, self.list.len() as u32);
                    self.list.push(c);
                }
            }
            id::ENTITY_TELEPORT => {
                let Some(eid) = first_varint(&raw.body) else {
                    return;
                };
                if self.ridden == Some(eid) {
                    return;
                }
                if let (Some(c), Some(e)) = (self.get_mut(eid), t.get(eid)) {
                    let d = e.position();
                    // onEntityPosition: a teleport within 1/32 (1/64 vertically) of the client
                    // position keeps it and ends the interpolation.
                    #[allow(clippy::neg_cmp_op_on_partial_ord)]
                    let tiny = !((c.pos[0] - d[0]).abs() >= 0.03125)
                        && !((c.pos[1] - d[1]).abs() >= 0.015625)
                        && !((c.pos[2] - d[2]).abs() >= 0.03125);
                    if tiny && c.class != Class12::Shulker {
                        c.target = c.pos;
                        c.steps = 0;
                    } else {
                        c.interpolate_to(d);
                    }
                }
            }
            id::REL_ENTITY_MOVE | id::ENTITY_MOVE_LOOK | id::ENTITY_LOOK | id::ENTITY => {
                // onEntityUpdate: towards the tracked position (1/4096 grid on every axis).
                let Some(eid) = first_varint(&raw.body) else {
                    return;
                };
                if self.ridden == Some(eid) {
                    return;
                }
                if let (Some(c), Some(e)) = (self.get_mut(eid), t.get(eid)) {
                    c.interpolate_to(e.position());
                }
            }
            id::ENTITY_METADATA => {
                // Size variants (babies, slime sizes) follow the metadata.
                let Some(eid) = first_varint(&raw.body) else {
                    return;
                };
                if let (Some(c), Some(e)) = (self.get_mut(eid), t.get(eid))
                    && let Some(d) = e.dimensions()
                    && d != c.dims
                {
                    c.dims = d;
                    c.placed();
                }
            }
            id::ENTITY_DESTROY => {
                let mut r = ghast_protocol::Reader::new(&raw.body);
                let Ok(n) = r.read_varint() else { return };
                for _ in 0..n.max(0) {
                    let Ok(eid) = r.read_varint() else { return };
                    self.remove(eid);
                }
            }
            _ => {}
        }
    }

    fn remove(&mut self, eid: i32) {
        let Some(k) = self.index.remove(&eid) else {
            return;
        };
        let c = self.list.swap_remove(k as usize);
        if let Some(moved) = self.list.get(k as usize) {
            self.index.insert(moved.id, k);
        }
        if c.class == Class12::Boat {
            self.boats -= 1;
        }
        self.boat_only -= usize::from(c.class.boat_only());
        if matches!(c.class, Class12::Boat | Class12::Cart) {
            self.ghosts.push(Ghost12 {
                seq: c.seq,
                bb: c.bb,
                cart: c.class == Class12::Cart,
            });
        }
    }

    /// The entity takes part (its column is loaded, it rides nothing and carries nothing).
    fn present(
        t: &ghast_entity::v1_12::Tracker,
        world: &ghast_world::v1_12::World,
        c: &Client12,
    ) -> bool {
        let Some(e) = t.get(c.id) else { return false };
        e.vehicle().is_none()
            && e.passengers().is_empty()
            && world.is_column_loaded((c.at[0].floor() as i32) >> 4, (c.at[2].floor() as i32) >> 4)
    }

    /// `World.hasEntityIn(box)` over these entities: one of them (not riding, not a spectator
    /// player) overlaps `cube` at its client position (`Box.intersects`).
    fn blocks_building(
        &self,
        t: &ghast_entity::v1_12::Tracker,
        world: &ghast_world::v1_12::World,
        cube: &ghast_physics::math::Aabb,
    ) -> bool {
        self.list.iter().any(|c| {
            !c.class.boat_only()
                && crate::sim::predict::boxes_intersect(&c.bb, cube)
                && Self::present(t, world, c)
                && !(c.class == Class12::Player
                    && t.get(c.id)
                        .is_some_and(|e| t.tab_game_mode(&e.uuid()) == Some(3)))
        })
    }

    /// This tick's boxes the player collides with: boats and live shulkers within reach of any
    /// move of the tick, and the boats destroyed since the last tick. In the boat the client
    /// moves ([`Near12::ridden`], the passenger's tick at the boat's place in the entity list):
    /// the same without that boat, and only the destroyed boats after it in the list.
    fn solids(
        &mut self,
        t: &ghast_entity::v1_12::Tracker,
        world: &ghast_world::v1_12::World,
        p: &ghast_physics::v1_12::PlayerState,
    ) {
        self.solids.clear();
        let own = self
            .ridden
            .and_then(|id| self.index.get(&id).map(|&k| (id, k)));
        let after = own.map_or(0, |(_, k)| self.list[k as usize].seq);
        let riding = own.is_none() && t.me().is_some_and(|m| m.vehicle().is_some());
        let reach = |v: f64| v.abs() + 3.0;
        let r = [reach(p.vel[0]), reach(p.vel[1]), reach(p.vel[2])];
        for c in &self.list {
            if !matches!(c.class, Class12::Boat | Class12::Shulker)
                || own.is_some_and(|(id, _)| id == c.id)
                || far(&c.bb, &p.bb, r)
                || riding
                || !Self::present(t, world, c)
            {
                continue;
            }
            // ShulkerEntity.getBox: only while alive.
            if c.class == Class12::Shulker
                && !t
                    .get(c.id)
                    .and_then(ghast_entity::v1_12::health)
                    .is_none_or(|h| h > 0.0)
            {
                continue;
            }
            self.solids.push(c.bb);
        }
        self.solids.extend(
            self.ghosts
                .iter()
                .filter(|g| !g.cart && g.seq > after)
                .map(|g| g.bb),
        );
    }

    /// The boxes the ridden boat collides with when it ticks (`World.doesBoxCollide` for the
    /// boat, `v1_12::blocks_boat`): the entities within reach of its move (`bb` grown by
    /// `reach`) whose column is loaded, at their client positions (the ones before the boat in
    /// the entity list have stepped this tick), and the destroyed boats and minecarts after it
    /// in the list. Left out: the boat itself, entities riding it with the player, spectator
    /// players, and entities that ride something (their seat is not followed here).
    fn boat_solids(
        &mut self,
        t: &ghast_entity::v1_12::Tracker,
        world: &ghast_world::v1_12::World,
        bb: &ghast_physics::math::Aabb,
        reach: [f64; 3],
    ) {
        self.boat_solids.clear();
        let Some(own) = self.ridden else { return };
        let after = self
            .index
            .get(&own)
            .map_or(0, |&k| self.list[k as usize].seq);
        for c in &self.list {
            if c.id == own || far(&c.bb, bb, reach) {
                continue;
            }
            let Some(e) = t.get(c.id) else { continue };
            if e.vehicle().is_some()
                || !world
                    .is_column_loaded((c.at[0].floor() as i32) >> 4, (c.at[2].floor() as i32) >> 4)
            {
                continue;
            }
            let alive = || ghast_entity::v1_12::health(e).is_none_or(|h| h > 0.0);
            let blocks = match c.class {
                Class12::Boat => true,
                Class12::Cart => true,
                Class12::Parrot => true,
                Class12::Shulker => alive(),
                Class12::Player => {
                    t.tab_game_mode(&e.uuid()) != Some(3)
                        && alive()
                        && !ghast_physics::v1_12::entity_climbing(world, c.at, c.bb.min_y)
                }
                Class12::Living => {
                    let name = ghast_entity::v1_12::info(e.entity_type()).map_or("", |i| i.name);
                    ghast_physics::v1_12::blocks_boat(
                        name,
                        true,
                        alive(),
                        ghast_physics::v1_12::entity_climbing(world, c.at, c.bb.min_y),
                        !e.passengers().is_empty(),
                    )
                }
            };
            if blocks {
                self.boat_solids.push(c.bb);
            }
        }
        self.boat_solids.extend(
            self.ghosts
                .iter()
                .filter(|g| g.seq > after && !far(&g.bb, bb, reach))
                .map(|g| g.bb),
        );
    }

    /// The entities' client ticks after the player's: one interpolation step each, the lily
    /// pads the boats near the player break (`lily_pads_broken_by_boat`; farther ones are left
    /// to the server), and the entities whose tick pushes the player, in tick order, into
    /// `pushers` (when `pushing`).
    fn tick(
        &mut self,
        t: &ghast_entity::v1_12::Tracker,
        world: &ghast_world::v1_12::World,
        p: &ghast_physics::math::Aabb,
        pushing: bool,
        profile_name: &str,
        seqs: std::ops::Range<u64>,
    ) {
        use ghast_physics::v1_12::{PushKind, Pusher};
        self.pushers.clear();
        self.lily_pads.clear();
        let version = self.world_version;
        let ridden = self.ridden;
        for c in &mut self.list {
            // The boat the client moves ticks with the player (the sim's).
            if !seqs.contains(&c.seq) || ridden == Some(c.id) {
                continue;
            }
            c.step();
            if c.class == Class12::Boat
                && c.scanned != Some((c.bb, version))
                && !far(&c.bb, p, [LILY_PAD_REACH; 3])
                && Self::present(t, world, c)
            {
                ghast_physics::v1_12::lily_pads_broken_by_boat(world, &c.bb, &mut self.lily_pads);
                c.scanned = Some((c.bb, version));
            }
            // Boats reach 0.2 farther; nothing else can touch the player.
            if !pushing
                || c.class.boat_only()
                || far(&c.bb, p, [0.25; 3])
                || !Self::present(t, world, c)
            {
                continue;
            }
            let Some(e) = t.get(c.id) else { continue };
            // A spectator player has noClip (set in its tick) and pushes nothing.
            if c.class == Class12::Player && t.tab_game_mode(&e.uuid()) == Some(3) {
                continue;
            }
            // EntityPredicate.method_13025: the pusher's and the player's team rules.
            if self.teams.any_rule() {
                let mut buf = [0u8; 36];
                let name: &str = match &c.name {
                    Some(n) => n,
                    None => e.uuid().hyphenated().encode_lower(&mut buf),
                };
                if !self.teams.may_push(name, profile_name) {
                    continue;
                }
            }
            let kind = if c.class == Class12::Boat {
                PushKind::Boat
            } else {
                PushKind::Living
            };
            self.pushers.push((c.seq, Pusher::new(c.at, c.bb, kind)));
        }
        self.pushers.sort_unstable_by_key(|x| x.0);
    }
}
