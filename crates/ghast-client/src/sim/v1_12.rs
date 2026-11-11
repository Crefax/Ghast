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
}

impl std::fmt::Debug for Sim12 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sim12")
            .field("world", &self.world)
            .field("pos", &self.player.pos)
            .finish_non_exhaustive()
    }
}

impl Sim12 {
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
        }
    }

    /// A sim with options (Grim compatibility, entities, world contents). Movement recording is
    /// not available on 1.12.2 yet.
    pub(crate) fn with(opts: &SimOptions) -> Self {
        let mut s = Self::new();
        s.force_step_position = opts.force_step_position;
        s.entities = opts.entities.map(ghast_entity::v1_12::Tracker::new);
        if let Some(c) = opts.world {
            s.world = ghast_world::v1_12::World::new(c);
            s.block_entities = c.block_entities;
        }
        if let Some(p) = opts.record.as_deref() {
            log::warn!(
                "movement recording is not supported on 1.12.2; not recording to {}",
                p.display()
            );
        }
        s
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
}

impl Sim for Sim12 {
    fn world_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        {
            use ghast_protocol::v1_12_2::play::clientbound::id;
            self.session.note(raw, id::LOGIN, id::RESPAWN);
        }
        if let Some(t) = &mut self.entities
            && t.wants(raw.id)
        {
            t.apply(raw)?;
        }
        use ghast_protocol::v1_12_2::play::clientbound as cb;
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
    }

    fn teleport(&mut self, pose: &mut Pose, t: &Teleport) {
        // The engine keeps the look (its rotation may have moved it since the last tick).
        self.player.yaw = pose.yaw;
        self.player.pitch = pose.pitch;
        let _reply =
            self.player
                .apply_teleport([t.x, t.y, t.z], t.yaw, t.pitch, rel_flags(t.flags));
        sync_pose_12(pose, &self.player);
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
            p.vel[0] += v.x;
            p.vel[1] += v.y;
            p.vel[2] += v.z;
        } else {
            p.vel = [v.x, v.y, v.z];
        }
    }

    fn movement_speed(&mut self, base: f64, modifiers: &[(u128, f64, u8)]) {
        self.player.apply_speed_attribute(base, modifiers);
    }

    fn status_effects(&mut self, effects: &std::collections::BTreeMap<i8, i8>) {
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
            let mut t = ghast_entity::v1_12::Tracker::new(c);
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
            .set_block_state(x, y, z, ghast_data::v1_12::BlockState::AIR);
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
        let input = ghast_physics::TickInput {
            keys: keys(controls),
            yaw: pose.yaw,
            pitch: pose.pitch,
            using_item,
        };
        self.packets.clear();
        ghast_physics::v1_12::tick(&mut self.player, &input, &self.world, &mut self.packets);
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
