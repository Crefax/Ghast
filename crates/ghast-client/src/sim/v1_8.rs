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
        let w = &mut self.world;
        match raw.id {
            cb::id::LOGIN => {
                w.apply_login(&raw.decode_as()?);
            }
            cb::id::RESPAWN => {
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
        self.world.packet_ids().collect()
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
        ghast_physics::v1_8::tick(&mut self.player, &input, &self.world, &mut self.packets);
        if self.force_step_position != ForceStepPosition::Off {
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
