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
}

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

impl Sim26 {
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
        }
    }

    /// A sim with options (world contents). Recording, entity tracking and the Grim step
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
        if opts.entities.is_some() {
            log::warn!("entity tracking is not available on 26.1 yet");
        }
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
        None
    }

    fn loaded_columns(&self) -> usize {
        self.world.columns().len()
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

    fn reset_player(&mut self) {
        // A fresh LocalPlayer; abilities and food are sent again by the server, the level
        // stays (attributes are re-sent too).
        let abilities = self.player.abilities;
        let food = self.player.food_level;
        let level = self.player.level;
        self.player = modern::PlayerState::new([0.0; 3], 0.0, 0.0);
        self.player.abilities = abilities;
        self.player.food_level = food;
        self.player.level = level;
        self.mining = mining_attributes();
        self.ranges = range_attributes();
        self.old_pos = [0.0; 3];
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
        None
    }

    fn set_entities(&mut self, config: Option<ghast_entity::Config>) {
        let _ = config;
    }

    fn entity_packet_ids(&self) -> Vec<i32> {
        Vec::new()
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
        // Entity.setOldPosAndRot at the start of the entity tick.
        self.old_pos = self.player.pos;
        let p = &mut self.player;
        let view = self
            .world
            .view(p.pos[0].floor() as i32, p.pos[2].floor() as i32);
        modern::tick::<V26_1, _>(p, &input, &view, &mut self.packets);
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
