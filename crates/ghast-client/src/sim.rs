//! Per-version world and player movement ("simulation"): the chunk store of `ghast-world` and
//! the movement step — `ghast-physics` for 1.8.9 (vanilla-exact), the stand-still mover for
//! 1.16.5 until its physics exists.
//!
//! The engine is generic over [`Sim`]; the behaviour sees the world through the
//! version-free [`WorldView`].

use ghast_protocol::{BlockPos, DecodeError, RawPacket};
use ghast_world::{WorldConfig, WorldError};

use crate::common::{
    Abilities, Controls, EntityAction, MovePacket, Outbound, Pose, Teleport, Vec3,
};
use crate::hands::{BlockQuery, HeldItem};

/// Why a world packet could not be applied.
#[derive(Debug)]
pub(crate) enum SimError {
    /// The packet itself did not decode.
    Decode(DecodeError),
    /// The chunk data inside it was malformed.
    World(WorldError),
}

impl From<DecodeError> for SimError {
    fn from(e: DecodeError) -> Self {
        SimError::Decode(e)
    }
}

impl From<WorldError> for SimError {
    fn from(e: WorldError) -> Self {
        SimError::World(e)
    }
}

/// Block access for behaviours, independent of the protocol version.
pub trait WorldView {
    /// Whether the chunk column is loaded (sent by the server and not unloaded).
    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool;

    /// The block state at a position as the version's raw id: the 1.8 `id << 4 | meta` or the
    /// 1.16 global palette id (decode it with `ghast-data`). `None` when the column is not
    /// loaded — never treat that as air.
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32>;

    /// Number of loaded columns.
    fn loaded_columns(&self) -> usize;

    /// The tracked entities (`None` unless the behaviour asked for [`crate::Interests::ENTITIES`]).
    fn entities(&self) -> Option<&ghast_entity::Entities>;

    /// Dig / place facts of a block (hardness, tool group, ray box); `None` when the column
    /// is not loaded.
    fn block_query(&self, x: i32, y: i32, z: i32) -> Option<BlockQuery>;
}

/// World + movement of one version.
/// Most block changes kept between two behaviour ticks (a chunk-sized burst is 65536).
const MAX_BLOCK_UPDATES: usize = 4096;

/// Block changes buffered for the behaviour.
#[derive(Debug, Default)]
struct BlockUpdates {
    on: bool,
    list: Vec<(BlockPos, u32)>,
    dropped: usize,
}

impl BlockUpdates {
    fn push(&mut self, pos: BlockPos, state: u32) {
        if self.on {
            if self.list.len() < MAX_BLOCK_UPDATES {
                self.list.push((pos, state));
            } else {
                self.dropped += 1;
            }
        }
    }
}

pub(crate) trait Sim: WorldView + std::fmt::Debug {
    /// Applies a world packet (chunks, block changes, join / respawn / view changes).
    /// Other packets are ignored.
    fn world_packet(&mut self, raw: &RawPacket) -> Result<(), SimError>;
    /// A new client player (join, respawn).
    fn reset_player(&mut self);
    /// A server teleport: updates the pose like vanilla (the engine sends the reply).
    fn teleport(&mut self, pose: &mut Pose, t: &Teleport);
    /// Server abilities.
    fn abilities(&mut self, a: Abilities);
    /// Food level (sprinting needs more than 6).
    fn food(&mut self, food: i32);
    /// Velocity from the server (entity velocity for our entity, explosion push), in blocks
    /// per tick; `add` adds to the current velocity (explosion) instead of setting it.
    fn velocity(&mut self, v: Vec3, add: bool);
    /// Our movement speed attribute: base and every modifier `(uuid, amount, operation)` in
    /// packet order (the server's list includes the sprint modifier while sprinting; vanilla
    /// replaces the client's set with it).
    fn movement_speed(&mut self, base: f64, modifiers: &[(u128, f64, u8)]);
    /// Our active status effects (id → amplifier).
    fn status_effects(&mut self, effects: &std::collections::BTreeMap<i8, i8>);
    /// Boots enchantments that change movement.
    fn boots(&mut self, depth_strider: u8, soul_speed: u8);
    /// Spectator game mode (no collision).
    fn spectator(&mut self, on: bool);
    /// One movement step: physics, then the sprint / sneak / abilities packets and exactly one
    /// movement packet appended to `out`. Updates `pose`.
    fn step(
        &mut self,
        pose: &mut Pose,
        controls: Controls,
        using_item: bool,
        entity_id: i32,
        out: &mut Vec<Outbound>,
    );
    /// Packet ids the entity tracker wants (empty without tracking).
    fn entity_packet_ids(&self) -> Vec<i32>;
    /// Client-side flight toggle: sets the physics' flying flag and appends the `abilities`
    /// packet when it changes (starting needs "may fly").
    fn set_flying(&mut self, flying: bool, out: &mut Vec<Outbound>);
    /// Whether block changes are collected for [`Sim::drain_block_updates`].
    fn collect_block_updates(&mut self, on: bool);
    /// Moves the block changes (position, version raw state) received since the last call
    /// into `out`; returns how many were dropped because the buffer was full.
    fn drain_block_updates(&mut self, out: &mut Vec<(BlockPos, u32)>) -> usize;
    /// The block was broken on the client (air until the server says otherwise).
    fn predict_air(&mut self, x: i32, y: i32, z: i32);
    /// A held item's version facts (tool speeds, block item, usable) from its id.
    fn held_item(&self, id: u16, count: u8, efficiency: u8) -> HeldItem;
}

/// Options of the per-version sim.
#[derive(Debug, Clone, Default)]
pub(crate) struct SimOptions {
    /// Write a movement recording there.
    pub record: Option<std::path::PathBuf>,
    /// Grim compatibility (not vanilla): send the position on ticks whose feet height changed
    /// although the move stayed under the 0.03 resend threshold.
    pub force_step_position: ForceStepPosition,
    /// Track entities (and the tab list).
    pub entities: Option<ghast_entity::Config>,
}

/// Grim compatibility (**not vanilla**): when to send the position on a tick whose move stayed
/// under vanilla's 0.03-block resend threshold. See `crates/ghast-physics/GRIM_NOTES.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ForceStepPosition {
    /// Vanilla: never.
    #[default]
    Off,
    /// Whenever the feet height changed since the last sent position (step-ups and landings).
    Always,
    /// Only step-ups: on ground after the move and the feet went up. Falls, landings and
    /// descents stay vanilla.
    StepUpOnly,
}

/// Grim compatibility: when the tick's movement packet carries no position but the feet height
/// changed since the last sent position (a step-up or landing under the 0.03 threshold),
/// upgrade it to `position` / `position_look` and update the sent bookkeeping.
fn force_position(
    packets: &mut ghast_physics::Packets,
    cur: [f64; 3],
    sent_pos: &mut [f64; 3],
    ticks_since_pos: &mut i32,
    mode: ForceStepPosition,
) {
    use ghast_physics::Packet as P;
    if mode == ForceStepPosition::Off || cur[1] == sent_pos[1] {
        return;
    }
    let Some(last) = packets.as_slice().last().copied() else {
        return;
    };
    if mode == ForceStepPosition::StepUpOnly {
        let on_ground = matches!(
            last,
            P::Flying { on_ground: true }
                | P::Look {
                    on_ground: true,
                    ..
                }
        );
        if !on_ground || cur[1] <= sent_pos[1] {
            return;
        }
    }
    let new = match last {
        P::Flying { on_ground } => P::Position {
            x: cur[0],
            y: cur[1],
            z: cur[2],
            on_ground,
        },
        P::Look {
            yaw,
            pitch,
            on_ground,
        } => P::PositionLook {
            x: cur[0],
            y: cur[1],
            z: cur[2],
            yaw,
            pitch,
            on_ground,
        },
        _ => return,
    };
    let mut out = ghast_physics::Packets::new();
    let n = packets.len();
    for p in &packets.as_slice()[..n - 1] {
        out.push(*p);
    }
    out.push(new);
    *packets = out;
    *sent_pos = cur;
    *ticks_since_pos = 0;
}

/// 1.16 blocks whose `onUse` succeeds for a plain right click (GUIs, doors,
/// switches, …). Name-based approximation of vanilla's per-block `use` overrides.
fn activates_1_16(name: &str) -> bool {
    const EXACT: &[&str] = &[
        "chest",
        "trapped_chest",
        "ender_chest",
        "crafting_table",
        "furnace",
        "blast_furnace",
        "smoker",
        "barrel",
        "anvil",
        "chipped_anvil",
        "damaged_anvil",
        "enchanting_table",
        "brewing_stand",
        "hopper",
        "dispenser",
        "dropper",
        "repeater",
        "comparator",
        "note_block",
        "jukebox",
        "cake",
        "loom",
        "stonecutter",
        "grindstone",
        "cartography_table",
        "smithing_table",
        "lectern",
        "bell",
        "beacon",
        "daylight_detector",
        "respawn_anchor",
        "composter",
        "lever",
        "command_block",
        "chain_command_block",
        "repeating_command_block",
        "structure_block",
        "jigsaw",
        "dragon_egg",
        "flower_pot",
        "sweet_berry_bush",
    ];
    EXACT.contains(&name)
        || name.ends_with("_shulker_box")
        || name == "shulker_box"
        || name.ends_with("_bed")
        || name.ends_with("_button")
        || name.ends_with("_fence_gate")
        || (name.ends_with("_door") && name != "iron_door")
        || (name.ends_with("_trapdoor") && name != "iron_trapdoor")
        || name.starts_with("potted_")
}

/// Union of the collision boxes as a ray box (`None` when there is none).
fn union(boxes: &[ghast_data::Aabb]) -> Option<[f64; 6]> {
    let mut u = [1.0_f64, 1.0, 1.0, 0.0, 0.0, 0.0];
    for b in boxes {
        u[0] = u[0].min(b.min_x);
        u[1] = u[1].min(b.min_y);
        u[2] = u[2].min(b.min_z);
        u[3] = u[3].max(b.max_x.min(1.0));
        u[4] = u[4].max(b.max_y.min(1.0));
        u[5] = u[5].max(b.max_z.min(1.0));
    }
    (!boxes.is_empty()).then_some(u)
}

/// 1.8 blocks whose client `Block.onUse` answers a right click.
const ACTIVATED_1_8: &[u8] = &[
    23, 25, 26, 54, 58, 61, 62, 64, 69, 71, 77, 84, 92, 93, 94, 96, 107, 116, 117, 118, 120, 130,
    138, 143, 145, 146, 149, 150, 151, 154, 158, 167, 178, 183, 184, 185, 186, 187, 193, 194, 195,
    196, 197,
];

/// 1.8 items used by holding right click besides food: potion, milk, bow, swords.
const USABLE_1_8: &[u16] = &[373, 335, 261, 267, 268, 272, 276, 283];

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

fn config() -> WorldConfig {
    // The bot needs block states only: no biomes, no light.
    WorldConfig {
        biomes: false,
        light: false,
    }
}

fn abilities_flags(a: &ghast_physics::Abilities) -> i8 {
    let mut f = 0;
    if a.invulnerable {
        f |= 1;
    }
    if a.flying {
        f |= 2;
    }
    if a.allow_flying {
        f |= 4;
    }
    if a.creative {
        f |= 8;
    }
    f
}

// ---- 1.8.9: ghast-physics ---------------------------------------------------------------

/// 1.8.9: the chunk store and the vanilla-exact physics.
pub(crate) struct Sim18 {
    world: ghast_world::v1_8::World,
    player: ghast_physics::v1_8::PlayerState,
    packets: ghast_physics::Packets,
    rec: Option<crate::record::Recorder>,
    force_step_position: ForceStepPosition,
    entities: Option<ghast_entity::v1_8::Tracker>,
    /// Block changes since the last drain (position, version raw state), when collected.
    updates: BlockUpdates,
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
        }
    }

    /// A sim with options (recording, Grim compatibility).
    pub(crate) fn with(opts: &SimOptions) -> Self {
        let mut s = Self::new();
        s.force_step_position = opts.force_step_position;
        s.entities = opts.entities.map(ghast_entity::v1_8::Tracker::new);
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
}

impl Sim for Sim18 {
    fn world_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
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
        HeldItem {
            id,
            count,
            efficiency,
            places_block: id < 256,
            usable: ghast_data::v1_8::food(id).is_some() || USABLE_1_8.contains(&id),
            food: ghast_data::v1_8::food(id).is_some(),
            // Golden apple.
            always_edible: id == 322,
            tool_speed: Some(ghast_data::v1_8::tool_multiplier),
        }
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

/// The physics' packets of a tick, in order, as wire packets.
fn push_packets(packets: &ghast_physics::Packets, entity_id: i32, out: &mut Vec<Outbound>) {
    for p in packets {
        use ghast_physics::Packet as P;
        let action = |action| Outbound::EntityAction { entity_id, action };
        out.push(match *p {
            P::StartSneaking => action(EntityAction::StartSneaking),
            P::StopSneaking => action(EntityAction::StopSneaking),
            P::StartSprinting => action(EntityAction::StartSprinting),
            P::StopSprinting => action(EntityAction::StopSprinting),
            P::Abilities(a) => Outbound::Abilities {
                flags: abilities_flags(&a),
                flying_speed: a.fly_speed,
                walking_speed: a.walk_speed,
            },
            P::Flying { on_ground } => Outbound::Move(MovePacket::Flying { on_ground }),
            P::Position { x, y, z, on_ground } => Outbound::Move(MovePacket::Position {
                pos: Vec3::new(x, y, z),
                on_ground,
            }),
            P::Look {
                yaw,
                pitch,
                on_ground,
            } => Outbound::Move(MovePacket::Look {
                yaw,
                pitch,
                on_ground,
            }),
            P::PositionLook {
                x,
                y,
                z,
                yaw,
                pitch,
                on_ground,
            } => Outbound::Move(MovePacket::PositionLook {
                pos: Vec3::new(x, y, z),
                yaw,
                pitch,
                on_ground,
            }),
        });
    }
}

fn physics_abilities(a: Abilities) -> ghast_physics::Abilities {
    ghast_physics::Abilities {
        invulnerable: a.flags & 1 != 0,
        flying: a.flags & 2 != 0,
        allow_flying: a.flags & 4 != 0,
        creative: a.flags & 8 != 0,
        fly_speed: a.flying_speed,
        walk_speed: a.walking_speed,
    }
}

fn keys(c: Controls) -> ghast_physics::Keys {
    ghast_physics::Keys {
        forward: c.forward,
        back: c.back,
        left: c.left,
        right: c.right,
        jump: c.jump,
        sneak: c.sneak,
        sprint: c.sprint,
    }
}

fn rel_flags(flags: i8) -> [bool; 5] {
    use crate::common::teleport_flags as fl;
    [
        flags & fl::X != 0,
        flags & fl::Y != 0,
        flags & fl::Z != 0,
        flags & fl::YAW != 0,
        flags & fl::PITCH != 0,
    ]
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
        }
    }

    /// A sim with options (recording, Grim compatibility).
    pub(crate) fn with(opts: &SimOptions) -> Self {
        let mut s = Self::new();
        s.force_step_position = opts.force_step_position;
        s.entities = opts.entities.map(ghast_entity::v1_16_5::Tracker::new);
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
}

impl Sim for Sim16 {
    fn world_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
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
        HeldItem {
            id,
            count,
            efficiency,
            places_block: ghast_data::v1_16_5::Item::from_id(id)
                .is_some_and(|i| ghast_data::v1_16_5::Block::from_name(i.info().name).is_some()),
            usable: ghast_data::v1_16_5::food(id).is_some()
                || ghast_data::v1_16_5::Item::from_id(id).is_some_and(|i| {
                    matches!(
                        i.info().name,
                        "potion"
                            | "milk_bucket"
                            | "honey_bottle"
                            | "bow"
                            | "crossbow"
                            | "shield"
                            | "trident"
                    )
                }),
            food: ghast_data::v1_16_5::food(id).is_some(),
            always_edible: ghast_data::v1_16_5::Item::from_id(id).is_some_and(|i| {
                matches!(
                    i.info().name,
                    "golden_apple" | "enchanted_golden_apple" | "chorus_fruit"
                )
            }),
            tool_speed: Some(ghast_data::v1_16_5::tool_multiplier),
        }
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

/// Chunk packets for tests.
#[cfg(test)]
pub(crate) mod test_chunks {
    use bytes::{BufMut, Bytes, BytesMut};
    use ghast_protocol::RawPacket;

    /// A 1.8 ground-up chunk with one section (y 48–63) whose top layer (y = 63) is stone.
    pub(crate) fn v1_8(x: i32, z: i32) -> RawPacket {
        let mut d = BytesMut::with_capacity(12_544);
        for i in 0..4096 {
            // Index = y << 8 | z << 4 | x; local y 15 is the top layer. Stone = 1 << 4.
            d.put_u16_le(if i >> 8 == 15 { 1 << 4 } else { 0 });
        }
        d.put_bytes(0, 2048); // block light
        d.put_bytes(0xff, 2048); // sky light
        d.put_bytes(1, 256); // biomes
        RawPacket::from_packet(&ghast_protocol::v1_8::play::clientbound::MapChunk {
            x,
            z,
            ground_up: true,
            bit_map: 1 << 3,
            chunk_data: d.freeze(),
        })
        .unwrap()
    }

    /// A 1.16.5 full chunk without sections (all air).
    pub(crate) fn v1_16_5(x: i32, z: i32) -> RawPacket {
        RawPacket::from_packet(&ghast_protocol::v1_16_5::play::clientbound::MapChunk {
            x,
            z,
            ground_up: true,
            bit_map: 0,
            heightmaps: ghast_protocol::nbt::RawNbt::from_bytes(Bytes::from_static(&[10, 0, 0, 0]))
                .unwrap(),
            biomes: Some(vec![1; 1024]),
            chunk_data: Bytes::new(),
            block_entities: Vec::new(),
        })
        .unwrap()
    }
}

#[cfg(test)]
impl WorldView for Box<dyn Sim> {
    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        (**self).is_column_loaded(chunk_x, chunk_z)
    }
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        (**self).raw_block(x, y, z)
    }
    fn entities(&self) -> Option<&ghast_entity::Entities> {
        (**self).entities()
    }
    fn loaded_columns(&self) -> usize {
        (**self).loaded_columns()
    }

    fn block_query(&self, x: i32, y: i32, z: i32) -> Option<BlockQuery> {
        (**self).block_query(x, y, z)
    }
}

#[cfg(test)]
impl Sim for Box<dyn Sim> {
    fn world_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        (**self).world_packet(raw)
    }
    fn reset_player(&mut self) {
        (**self).reset_player();
    }
    fn teleport(&mut self, pose: &mut Pose, t: &Teleport) {
        (**self).teleport(pose, t);
    }
    fn abilities(&mut self, a: Abilities) {
        (**self).abilities(a);
    }
    fn food(&mut self, food: i32) {
        (**self).food(food);
    }
    fn velocity(&mut self, v: Vec3, add: bool) {
        (**self).velocity(v, add);
    }
    fn step(
        &mut self,
        pose: &mut Pose,
        controls: Controls,
        using_item: bool,
        entity_id: i32,
        out: &mut Vec<Outbound>,
    ) {
        (**self).step(pose, controls, using_item, entity_id, out);
    }
    fn set_flying(&mut self, flying: bool, out: &mut Vec<Outbound>) {
        (**self).set_flying(flying, out);
    }
    fn collect_block_updates(&mut self, on: bool) {
        (**self).collect_block_updates(on);
    }
    fn drain_block_updates(&mut self, out: &mut Vec<(BlockPos, u32)>) -> usize {
        (**self).drain_block_updates(out)
    }
    fn entity_packet_ids(&self) -> Vec<i32> {
        (**self).entity_packet_ids()
    }
    fn predict_air(&mut self, x: i32, y: i32, z: i32) {
        (**self).predict_air(x, y, z);
    }
    fn movement_speed(&mut self, base: f64, modifiers: &[(u128, f64, u8)]) {
        (**self).movement_speed(base, modifiers);
    }
    fn status_effects(&mut self, effects: &std::collections::BTreeMap<i8, i8>) {
        (**self).status_effects(effects);
    }
    fn boots(&mut self, depth_strider: u8, soul_speed: u8) {
        (**self).boots(depth_strider, soul_speed);
    }
    fn spectator(&mut self, on: bool) {
        (**self).spectator(on);
    }
    fn held_item(&self, id: u16, count: u8, efficiency: u8) -> HeldItem {
        (**self).held_item(id, count, efficiency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::{ClientVersion, V1_8};

    /// The 1.8 physics' teleport matches the version's documented rules.
    #[test]
    fn teleport_rules_agree() {
        let cases = [
            (1.5, 64.0, -2.5, 450.0_f32, 100.0_f32, 0_i8),
            (0.25, -1.0, 0.5, -30.0, -95.0, 0x1f),
            (3.0, 70.0, 3.0, 720.5, 10.0, 0x08),
        ];
        for (x, y, z, yaw, pitch, flags) in cases {
            let t = Teleport {
                x,
                y,
                z,
                yaw,
                pitch,
                flags,
                teleport_id: None,
            };
            let start = Pose {
                pos: Vec3::new(10.0, 65.0, 10.0),
                yaw: 33.0,
                pitch: 5.0,
                motion: Vec3::new(0.1, -0.0784, 0.2),
                ..Pose::default()
            };
            let mut a = start;
            <V1_8 as ClientVersion>::apply_teleport(&mut a, &t);
            let mut sim = Sim18::new();
            let mut b = start;
            sim.player
                .set_position(start.pos.x, start.pos.y, start.pos.z);
            sim.player.vel = [start.motion.x, start.motion.y, start.motion.z];
            sim.teleport(&mut b, &t);
            assert_eq!(
                (a.pos, a.yaw, a.pitch, a.motion),
                (b.pos, b.yaw, b.pitch, b.motion),
                "{t:?}"
            );
        }
    }

    #[test]
    fn force_position_only_on_height_change() {
        use ghast_physics::Packet as P;
        let mut pk = ghast_physics::Packets::new();
        pk.push(P::StopSprinting);
        pk.push(P::Look {
            yaw: 1.0,
            pitch: 2.0,
            on_ground: true,
        });
        let mut sent = [1.0, 3.979, 8.3];
        let mut ticks = 3;
        // Same height: untouched.
        force_position(
            &mut pk,
            [1.01, 3.979, 8.3],
            &mut sent,
            &mut ticks,
            ForceStepPosition::Always,
        );
        assert!(matches!(pk.as_slice()[1], P::Look { .. }));
        // Stepped up: the look becomes a position_look, the bookkeeping follows.
        force_position(
            &mut pk,
            [1.01, 4.0, 8.28],
            &mut sent,
            &mut ticks,
            ForceStepPosition::Always,
        );
        assert_eq!(pk.as_slice()[0], P::StopSprinting);
        assert!(matches!(
            pk.as_slice()[1],
            P::PositionLook {
                y: 4.0,
                on_ground: true,
                ..
            }
        ));
        assert_eq!((sent, ticks), ([1.01, 4.0, 8.28], 0));
    }

    #[test]
    fn force_position_step_up_only() {
        use ghast_physics::Packet as P;
        let look = |on_ground| P::Look {
            yaw: 1.0,
            pitch: 2.0,
            on_ground,
        };
        let mode = ForceStepPosition::StepUpOnly;
        // Feet went down (a landing under the threshold): stays vanilla.
        let mut pk = ghast_physics::Packets::new();
        pk.push(look(true));
        let (mut sent, mut ticks) = ([1.0, 4.0, 8.3], 3);
        force_position(&mut pk, [1.01, 3.979, 8.3], &mut sent, &mut ticks, mode);
        assert!(matches!(pk.as_slice()[0], P::Look { .. }));
        // Went up but airborne: stays vanilla.
        force_position(&mut pk, [1.01, 4.01, 8.3], &mut sent, &mut ticks, mode);
        let mut air = ghast_physics::Packets::new();
        air.push(look(false));
        force_position(&mut air, [1.01, 4.01, 8.3], &mut sent, &mut ticks, mode);
        assert!(matches!(air.as_slice()[0], P::Look { .. }));
        // A step-up (up and on ground): forced.
        let mut up = ghast_physics::Packets::new();
        up.push(look(true));
        let (mut sent, mut ticks) = ([1.0, 3.979, 8.3], 3);
        force_position(&mut up, [1.01, 4.0, 8.28], &mut sent, &mut ticks, mode);
        assert!(matches!(up.as_slice()[0], P::PositionLook { y: 4.0, .. }));
        assert_eq!((sent, ticks), ([1.01, 4.0, 8.28], 0));
    }

    #[test]
    fn physics_stands_on_the_test_floor() {
        let mut sim = Sim18::new();
        sim.world_packet(&test_chunks::v1_8(0, 0)).unwrap();
        let mut pose = Pose::default();
        sim.teleport(
            &mut pose,
            &Teleport {
                x: 8.5,
                y: 64.0,
                z: 8.5,
                yaw: 0.0,
                pitch: 0.0,
                flags: 0,
                teleport_id: None,
            },
        );
        let mut out = Vec::new();
        for _ in 0..30 {
            sim.step(&mut pose, Controls::NONE, false, 1, &mut out);
        }
        assert_eq!(pose.pos, Vec3::new(8.5, 64.0, 8.5));
        assert!(pose.on_ground);
        // Walking forward moves the player.
        for _ in 0..10 {
            sim.step(
                &mut pose,
                Controls {
                    forward: true,
                    ..Controls::NONE
                },
                false,
                1,
                &mut out,
            );
        }
        assert!(pose.pos.z > 9.0, "{pose:?}");
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
