//! 1.16.5: the chunk store and the vanilla-exact physics.

use ghast_protocol::{BlockPos, RawPacket};

use super::*;
use crate::common::{Abilities, Controls, Outbound, Pose, Teleport, Vec3};
use crate::hands::{BlockQuery, HeldItem};

// ---- 1.16.5: world + stand-still mover --------------------------------------------------

/// 1.16.5: the chunk store and the stand-still mover (no 1.16 physics yet).
pub(crate) struct Sim16 {
    world: ghast_world::v1_16_5::World,
    player: ghast_physics::v1_16_5::PlayerState,
    packets: ghast_physics::Packets,
    rec: Option<crate::record::Recorder>,
    force_step_position: ForceStepPosition,
    entities: Option<ghast_entity::v1_16_5::Tracker>,
    /// Block changes since the last drain (position, version raw state), when collected.
    updates: BlockUpdates,
    /// The session's last join and respawn packets: a tracker started mid-session replays
    /// them to learn our entity id and dimension.
    session: SessionPackets,
    /// The world keeps block entities (signs, tile entity NBT).
    block_entities: bool,
}

impl std::fmt::Debug for Sim16 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sim16")
            .field("world", &self.world)
            .finish_non_exhaustive()
    }
}

impl Sim16 {
    pub(crate) fn new() -> Self {
        Sim16 {
            world: ghast_world::v1_16_5::World::new(config()),
            player: ghast_physics::v1_16_5::PlayerState::new([0.0; 3], 0.0, 0.0),
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
        s.entities = opts.entities.map(ghast_entity::v1_16_5::Tracker::new);
        if let Some(c) = opts.world {
            s.world = ghast_world::v1_16_5::World::new(c);
            s.block_entities = c.block_entities;
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

impl WorldView for Sim16 {
    fn column(&self, chunk_x: i32, chunk_z: i32) -> Option<&ghast_world::Column> {
        self.world.column(chunk_x, chunk_z)
    }

    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.world.is_column_loaded(chunk_x, chunk_z)
    }

    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.world.block_state(x, y, z).map(|s| u32::from(s.id()))
    }

    fn entities(&self) -> Option<&ghast_entity::Entities> {
        self.entities.as_deref()
    }

    fn loaded_columns(&self) -> usize {
        self.world.columns().len()
    }

    fn block_query(&self, x: i32, y: i32, z: i32) -> Option<BlockQuery> {
        let s = self.world.block_state(x, y, z)?;
        let info = s.block().info();
        let air = s.block().is_air();
        let passable = matches!(
            info.name,
            "water" | "lava" | "fire" | "nether_portal" | "end_portal"
        );
        Some(BlockQuery {
            modern: None,
            air,
            water: info.name == "water",
            block: s.block().id(),
            hardness: if info.diggable {
                info.hardness.unwrap_or(-1.0)
            } else {
                -1.0
            },
            dig_material: info.material,
            harvest_tools: info.harvest_tools,
            selection: if air || passable {
                None
            } else {
                Some(union(s.shape()).unwrap_or([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]))
            },
            stairs: None,
            activates: activates_1_16(info.name),
        })
    }

    fn crosshair(
        &self,
        eyes: Vec3,
        (yaw, pitch): (f32, f32),
        _look: Vec3,
        reach: f64,
    ) -> Option<crate::BlockHit> {
        // The oracle-verified 1.16.5 pick (outline shapes, model offsets, interaction shapes).
        use ghast_physics::pick::v1_16_5 as pick;
        let eye = [eyes.x, eyes.y, eyes.z];
        let hit = pick::pick(
            &self.world,
            eye,
            pick::rotation_vector(yaw, pitch),
            reach,
            None,
        );
        block_hit(&hit, eye)
    }
}

impl Sim for Sim16 {
    fn crosshair_now(
        &self,
        eyes: Vec3,
        (yaw, pitch): (f32, f32),
        _look: Vec3,
        held: Option<u16>,
    ) -> Option<crate::BlockHit> {
        use ghast_physics::pick::v1_16_5 as pick;
        let held_block = held
            .and_then(ghast_data::v1_16_5::Item::from_id)
            .and_then(|i| ghast_data::v1_16_5::Block::from_name(i.info().name));
        let eye = [eyes.x, eyes.y, eyes.z];
        let hit = pick::pick(
            &self.world,
            eye,
            pick::rotation_vector(yaw, pitch),
            crate::hands::REACH,
            held_block,
        );
        block_hit(&hit, eye)
    }

    fn world_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        {
            use ghast_protocol::v1_16_5::play::clientbound::id;
            self.session.note(raw, id::LOGIN, id::RESPAWN);
        }
        if let Some(t) = &mut self.entities
            && t.wants(raw.id)
        {
            t.apply(raw)?;
        }
        use ghast_protocol::v1_16_5::play::clientbound as cb;
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
            cb::id::UNLOAD_CHUNK => {
                w.apply_unload_chunk(&raw.decode_as()?);
            }
            cb::id::UPDATE_VIEW_POSITION => {
                w.apply_view_position(&raw.decode_as()?);
            }
            cb::id::UPDATE_VIEW_DISTANCE => {
                w.apply_view_distance(&raw.decode_as()?);
            }
            cb::id::BLOCK_CHANGE => {
                let p: cb::BlockChange = raw.decode_as()?;
                self.updates.push(p.location, p.r#type as u32);
                w.apply_block_change(&p);
            }
            cb::id::MULTI_BLOCK_CHANGE => {
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
        self.player = ghast_physics::v1_16_5::PlayerState::new([0.0; 3], 0.0, 0.0);
        self.player.abilities = abilities;
        self.player.food_level = food;
    }

    fn teleport(&mut self, pose: &mut Pose, t: &Teleport) {
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
        sync_pose_16(pose, &self.player);
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
        e.slow_falling = effects.contains_key(&28);
        e.dolphins_grace = effects.contains_key(&30);
    }

    fn boots(&mut self, depth_strider: u8, soul_speed: u8) {
        self.player.enchantments.depth_strider = depth_strider;
        self.player.enchantments.soul_speed = soul_speed;
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
            let mut t = ghast_entity::v1_16_5::Tracker::new(c);
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
            .set_block_state(x, y, z, ghast_data::v1_16_5::BlockState::AIR);
    }

    fn held_item(&self, id: u16, count: u8, efficiency: u8) -> HeldItem {
        held_item_facts(DataSet::V1_16_5, id, count, efficiency)
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
            r.start_1_16(&self.player);
        }
        if let Some(r) = &mut self.rec {
            let w = &self.world;
            r.observe(self.player.pos, |x, y, z| {
                w.is_column_loaded(x >> 4, z >> 4)
                    .then(|| w.block_state(x, y, z).map_or(0, |s| u32::from(s.id())))
            });
        }
        ghast_physics::v1_16_5::tick(&mut self.player, &input, &self.world, &mut self.packets);
        if self.force_step_position != ForceStepPosition::Off {
            let p = &mut self.player;
            let cur = p.pos;
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
                &crate::record::state_1_16(&self.player),
                &self.packets,
            );
        }
        push_packets(&self.packets, entity_id, out);
        sync_pose_16(pose, &self.player);
        self.world.advance_time(50);
    }
}

fn sync_pose_16(pose: &mut Pose, p: &ghast_physics::v1_16_5::PlayerState) {
    pose.pos = Vec3::new(p.pos[0], p.pos[1], p.pos[2]);
    pose.motion = Vec3::new(p.vel[0], p.vel[1], p.vel[2]);
    pose.yaw = p.yaw;
    pose.pitch = p.pitch;
    pose.on_ground = p.on_ground;
    pose.flying = p.abilities.flying;
    pose.sneaking = p.sneaking();
    pose.sprinting = p.sprinting;
}
