//! Per-version world and player movement ("simulation"): the chunk store of `ghast-world` and
//! the vanilla-exact movement step of `ghast-physics`, one module per version.
//!
//! The engine is generic over [`Sim`]; the behaviour sees the world through the
//! version-free [`WorldView`].

use ghast_protocol::{BlockPos, DecodeError, RawPacket};
use ghast_world::{WorldConfig, WorldError};

use crate::common::{
    Abilities, Controls, EntityAction, MovePacket, Outbound, Pose, Teleport, Vec3,
};
use crate::hands::{BlockQuery, HeldItem};
use crate::version::DataSet;

#[cfg(feature = "v1_12_2")]
mod v1_12;
#[cfg(feature = "v1_16_5")]
mod v1_16_5;
#[cfg(feature = "v1_8")]
mod v1_8;
#[cfg(feature = "v26_1")]
mod v26_1;

#[cfg(feature = "v1_8")]
pub(crate) use v1_8::Sim18;
#[cfg(feature = "v1_12_2")]
pub(crate) use v1_12::Sim12;
#[cfg(feature = "v1_16_5")]
pub(crate) use v1_16_5::Sim16;
#[cfg(feature = "v26_1")]
pub(crate) use v26_1::Sim26;

/// The crosshair pick's inputs besides the world (vanilla `GameRenderer.updateTargetedEntity`).
#[cfg_attr(
    not(any(feature = "v1_8", feature = "v1_12_2", feature = "v1_16_5")),
    allow(dead_code)
)]
pub(crate) struct PickQuery<'a> {
    /// The eyes (feet + eye height of the pose judged).
    pub eyes: Vec3,
    /// `(yaw, pitch)` of that pose.
    pub rotation: (f32, f32),
    /// The generic look vector (`hands::look_vector`); versions use their own.
    pub look: Vec3,
    /// The main-hand item id (1.16.5+: a held block changes some interaction shapes).
    pub held: Option<u16>,
    /// Pick from the sim's own current state (this tick's key handling: 26.1 eye lerp and
    /// range attributes) instead of `eyes`.
    pub now: bool,
    /// Creative reach (blocks 5, entities 6, no 3-block entity limit).
    pub creative: bool,
    /// The player's bounding box (the entity search box grows from it).
    pub player_bb: ghast_physics::math::Aabb,
    /// The pickable entities near the eyes, in the tracker's order.
    pub entities: &'a [ghast_physics::pick::PickEntity],
}

/// The crosshair target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Crosshair {
    /// A block, or nothing (a miss, or an entity beyond the 3-block limit).
    Block(Option<crate::BlockHit>),
    /// An entity: index into [`PickQuery::entities`] and the point hit.
    Entity {
        /// Index into the query's entities.
        index: usize,
        /// The point on its (grown) box; the eyes when they are inside it.
        hit: Vec3,
    },
}

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

    /// The whole loaded column (sections, light / biomes when kept, see
    /// [`crate::Client::world_config`]) for bulk reads without a call per block. Block states
    /// are the version's raw ids, as in [`WorldView::raw_block`]. `None` when not loaded.
    fn column(&self, chunk_x: i32, chunk_z: i32) -> Option<&ghast_world::Column> {
        let _ = (chunk_x, chunk_z);
        None
    }

    /// The block the crosshair is on from `eyes` with the look `(yaw, pitch)` (`look` is its
    /// unit vector, [`crate::hands::look_vector`]) within `reach`, by the version's ray rules.
    /// `None` when nothing is hit.
    fn crosshair(
        &self,
        eyes: Vec3,
        rotation: (f32, f32),
        look: Vec3,
        reach: f64,
    ) -> Option<crate::BlockHit> {
        let _ = rotation;
        let blocks = |x: i32, y: i32, z: i32| self.block_query(x, y, z);
        crate::hands::pick(&blocks, eyes, look, reach)
    }
}

/// The join / respawn packets of the session (cheap `Bytes` copies).
#[cfg(any(
    feature = "v1_8",
    feature = "v1_12_2",
    feature = "v1_16_5",
    feature = "v26_1"
))]
#[derive(Debug, Default)]
pub(super) struct SessionPackets {
    login: Option<RawPacket>,
    respawn: Option<RawPacket>,
}

#[cfg(any(
    feature = "v1_8",
    feature = "v1_12_2",
    feature = "v1_16_5",
    feature = "v26_1"
))]
impl SessionPackets {
    fn note(&mut self, raw: &RawPacket, login: i32, respawn: i32) {
        if raw.id == login {
            self.login = Some(raw.clone());
            self.respawn = None;
        } else if raw.id == respawn {
            self.respawn = Some(raw.clone());
        }
    }

    /// Feeds them to a fresh tracker.
    fn replay(&self, mut apply: impl FnMut(&RawPacket)) {
        for r in [&self.login, &self.respawn].into_iter().flatten() {
            apply(r);
        }
    }
}

/// Most block changes kept between two behaviour ticks (a chunk-sized burst is 65536).
pub(super) const MAX_BLOCK_UPDATES: usize = 4096;

/// Block changes buffered for the behaviour.
#[derive(Debug, Default)]
pub(super) struct BlockUpdates {
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

/// World + movement of one version.
pub(crate) trait Sim: WorldView + std::fmt::Debug {
    /// Applies a world packet (chunks, block changes, join / respawn / view changes).
    /// Other packets are ignored.
    fn world_packet(&mut self, raw: &RawPacket) -> Result<(), SimError>;
    /// A new client player (join, respawn).
    fn reset_player(&mut self);
    /// Play → configuration (`start_configuration`): vanilla's `clearClientLevel` drops the
    /// level (chunks, entities) and the player until the next join.
    fn drop_level(&mut self) {
        self.reset_player();
    }
    /// A new client player after a respawn; `keep` is the respawn packet's kept-data bits
    /// (1.20.2+).
    fn respawn_player(&mut self, keep: u8) {
        let _ = keep;
        self.reset_player();
    }
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
    /// The item id in the feet slot (1.17+ powder snow: leather boots).
    fn feet_item(&mut self, id: Option<i32>) {
        let _ = id;
    }
    /// Spectator game mode (no collision).
    fn spectator(&mut self, on: bool);
    /// 26.1: the client player's movement state (see [`crate::TickContext::modern_player`]).
    #[cfg(feature = "v26_1")]
    fn modern_player(&self) -> Option<&ghast_physics::modern::PlayerState> {
        None
    }
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
    /// Packet ids the world consumes with its configuration.
    fn world_packet_ids(&self) -> Vec<i32>;
    /// The entity tracking configuration (`None`: not tracking).
    #[cfg_attr(
        not(any(feature = "v1_8", feature = "v1_12_2", feature = "v1_16_5")),
        allow(dead_code)
    )]
    fn entity_config(&self) -> Option<ghast_entity::Config>;
    /// Starts (empty, our entity id and dimension from the session's join / respawn), stops
    /// (frees the store) or reconfigures entity tracking mid-session.
    fn set_entities(&mut self, config: Option<ghast_entity::Config>);
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
    /// The crosshair target as vanilla picks it at the start of this tick (before the key
    /// handling) with the player's own eye and ranges; `eyes` / `look` are the generic
    /// approximation (feet + eye height, `hands::look_vector`). `held` is the main-hand item id.
    fn crosshair_now(
        &self,
        eyes: Vec3,
        rotation: (f32, f32),
        look: Vec3,
        held: Option<u16>,
    ) -> Option<crate::BlockHit> {
        let _ = held;
        self.crosshair(eyes, rotation, look, crate::hands::REACH)
    }
    /// Whether the crosshair pick can hit `e` and by how much its box grows (`None`: not
    /// pickable). Default: the engine's 1.8 – 1.16.5 classification.
    fn pick_margin(&self, e: &ghast_entity::Entity) -> Option<f32> {
        crate::engine::legacy_pick_margin(self.entities()?, e)
    }
    /// One client tick of the tracked entities (`ClientLevel.tickEntities`), after the tick's
    /// crosshair pick. Versions whose tracker has no client-side motion do nothing.
    fn tick_entities(&mut self) {}
    /// 1.21.4+ level-load gate: the server's `game_event` 13 (`LEVEL_CHUNKS_LOAD_START`).
    fn level_load_start(&mut self) {}
    /// 1.21.4+ level-load gate: the player is alive (`set_health` above 0).
    fn player_alive(&mut self, alive: bool) {
        let _ = alive;
    }
    /// 1.21.4+ level-load gate, once per tick before the key handling: `true` on the tick the
    /// client sends `player_loaded`. `section_compiled`: the engine's readiness signal;
    /// `timed_out`: the client's 30 s wait is over.
    fn load_gate_tick(&mut self, section_compiled: bool, timed_out: bool) -> bool {
        let _ = (section_compiled, timed_out);
        false
    }
    /// 1.21.4+: a tick behind the level-load gate (the player does not tick; only its entity
    /// tick counter runs).
    fn tick_unloaded(&mut self) {}
    /// The whole crosshair target (vanilla `GameRenderer.updateTargetedEntity`): the block
    /// pick unless one of `q.entities` is hit nearer. Versions without an entity pick yet
    /// pick blocks only.
    fn crosshair_target(&self, q: &PickQuery<'_>) -> Crosshair {
        Crosshair::Block(if q.now {
            self.crosshair_now(q.eyes, q.rotation, q.look, q.held)
        } else {
            self.crosshair(q.eyes, q.rotation, q.look, crate::hands::REACH)
        })
    }
    /// 1.20.5+ movement attributes of our entity, in packet order.
    fn attributes(&mut self, attributes: &[crate::common::AttributeUpdate]) {
        let _ = attributes;
    }
    /// `chunk_batch_start` arrived (1.20.2+), at `now_nanos` from any fixed origin.
    fn chunk_batch_start(&mut self, now_nanos: i64) {
        let _ = now_nanos;
    }
    /// `chunk_batch_finished` arrived: the chunks per tick to answer with (1.20.2+).
    fn chunk_batch_finished(&mut self, batch_size: i32, now_nanos: i64) -> f32 {
        let _ = (batch_size, now_nanos);
        // ChunkBatchSizeCalculator's initial value: 7 ms / 2 ms.
        3.5
    }
    /// A configuration-state `registry_data` packet (1.20.2+).
    fn registry_data(&mut self, raw: &RawPacket) {
        let _ = raw;
    }
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
    /// What the world keeps besides block states (default: nothing, see [`config`]).
    pub world: Option<WorldConfig>,
}

/// Grim compatibility (**not vanilla**): when to send the position on a tick whose move stayed
/// under vanilla's 0.03-block resend threshold. See `crates/ghast-physics/GRIM_NOTES.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
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
#[cfg(any(feature = "v1_8", feature = "v1_12_2", feature = "v1_16_5"))]
pub(super) fn force_position(
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
#[cfg(any(feature = "v1_16_5", feature = "v26_1"))]
pub(super) fn activates_1_16(name: &str) -> bool {
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

/// A legacy (1.8.9 / 1.12.2) crosshair target as the engine takes it.
#[cfg(any(feature = "v1_8", feature = "v1_12_2"))]
pub(super) fn legacy_crosshair(t: ghast_physics::pick::legacy::Target, eye: [f64; 3]) -> Crosshair {
    use ghast_physics::pick::legacy::Target;
    match t {
        Target::Block(h) => Crosshair::Block(block_hit(&h, eye)),
        Target::Entity(e) => Crosshair::Entity {
            index: e.index,
            hit: Vec3::new(e.location[0], e.location[1], e.location[2]),
        },
        Target::None | Target::TooFar { .. } => Crosshair::Block(None),
    }
}

/// A modern (1.16.5+) crosshair target as the engine takes it.
#[cfg(any(feature = "v1_16_5", feature = "v26_1"))]
pub(super) fn modern_crosshair(t: ghast_physics::pick::Target, eye: [f64; 3]) -> Crosshair {
    use ghast_physics::pick::Target;
    match t {
        Target::Block(h) => Crosshair::Block(block_hit(&h, eye)),
        Target::Entity(e) => Crosshair::Entity {
            index: e.index,
            hit: Vec3::new(e.location[0], e.location[1], e.location[2]),
        },
    }
}

/// A pick result as the hands take it (`None` for a miss).
pub(super) fn block_hit(
    hit: &ghast_physics::pick::BlockHit,
    eye: [f64; 3],
) -> Option<crate::BlockHit> {
    use ghast_physics::pick::Direction;
    if !hit.is_block() {
        return None;
    }
    let face = match hit.face {
        Direction::Down => crate::Face::Down,
        Direction::Up => crate::Face::Up,
        Direction::North => crate::Face::North,
        Direction::South => crate::Face::South,
        Direction::West => crate::Face::West,
        Direction::East => crate::Face::East,
    };
    let [lx, ly, lz] = hit.location;
    let distance = ((lx - eye[0]).powi(2) + (ly - eye[1]).powi(2) + (lz - eye[2]).powi(2)).sqrt();
    Some(crate::BlockHit {
        pos: BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
        face,
        hit: Vec3::new(lx, ly, lz),
        distance,
    })
}

/// Union of the collision boxes as a ray box (`None` when there is none).
#[cfg(any(feature = "v1_12_2", feature = "v1_16_5", feature = "v26_1"))]
pub(super) fn union(boxes: &[ghast_data::Aabb]) -> Option<[f64; 6]> {
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
#[cfg(any(feature = "v1_8", feature = "v1_12_2"))]
pub(super) const ACTIVATED_1_8: &[u8] = &[
    23, 25, 26, 54, 58, 61, 62, 64, 69, 71, 77, 84, 92, 93, 94, 96, 107, 116, 117, 118, 120, 130,
    138, 143, 145, 146, 149, 150, 151, 154, 158, 167, 178, 183, 184, 185, 186, 187, 193, 194, 195,
    196, 197,
];

#[cfg(feature = "v1_12_2")]
/// 1.12 items used by holding right click besides food: potion, milk, bow, shield, splash and
/// lingering potions are thrown (not held).
const USABLE_1_12: &[u16] = &[373, 335, 261, 442];

/// 1.8 items used by holding right click besides food: potion, milk, bow, swords.
#[cfg(feature = "v1_8")]
const USABLE_1_8: &[u16] = &[373, 335, 261, 267, 268, 272, 276, 283];

/// A held item's version facts (tool speeds, block item, usable) from its id.
pub(crate) fn held_item_facts(data: DataSet, id: u16, count: u8, efficiency: u8) -> HeldItem {
    match data {
        #[cfg(feature = "v1_8")]
        DataSet::V1_8 => {
            HeldItem {
                modern_item: None,
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
        #[cfg(feature = "v1_12_2")]
        DataSet::V1_12 => HeldItem {
            modern_item: None,
            id,
            count,
            efficiency,
            places_block: id < 256,
            usable: ghast_data::v1_12::food(id).is_some() || USABLE_1_12.contains(&id),
            food: ghast_data::v1_12::food(id).is_some(),
            // Golden apple, chorus fruit.
            always_edible: id == 322 || id == 432,
            tool_speed: Some(ghast_data::v1_12::tool_multiplier),
        },
        #[cfg(feature = "v1_16_5")]
        DataSet::V1_16_5 => HeldItem {
            modern_item: None,
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
        },
        #[cfg(feature = "v26_1")]
        DataSet::V26_1 => {
            let item = ghast_data::v26_1::Item::from_id(id);
            let name = item.map_or("", ghast_data::v26_1::Item::name);
            let food = item.and_then(ghast_data::v26_1::Item::food);
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
            }
        }
        // A data set this build does not include: an item without known facts.
        #[allow(unreachable_patterns)]
        _ => HeldItem {
            modern_item: None,
            id,
            count,
            efficiency,
            places_block: false,
            usable: false,
            food: false,
            always_edible: false,
            tool_speed: None,
        },
    }
}

pub(super) fn config() -> WorldConfig {
    // The bot needs block states only: no biomes, no light.
    WorldConfig::default()
        .with_biomes(false)
        .with_light(false)
        .with_block_entities(false)
}

pub(super) fn abilities_flags(a: &ghast_physics::Abilities) -> i8 {
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

/// The physics' packets of a tick, in order, as wire packets.
pub(super) fn push_packets(
    packets: &ghast_physics::Packets,
    entity_id: i32,
    out: &mut Vec<Outbound>,
) {
    for p in packets {
        use ghast_physics::Packet as P;
        let action = |action| Outbound::EntityAction { entity_id, action };
        let wire = match *p {
            P::StartSneaking => action(EntityAction::StartSneaking),
            P::StopSneaking => action(EntityAction::StopSneaking),
            P::StartSprinting => action(EntityAction::StartSprinting),
            P::StopSprinting => action(EntityAction::StopSprinting),
            P::Abilities(a) => Outbound::Abilities {
                flags: abilities_flags(&a),
                flying_speed: a.fly_speed,
                walking_speed: a.walk_speed,
            },
            P::Flying { on_ground } => Outbound::Move(MovePacket::Flying {
                horizontal_collision: false,
                on_ground,
            }),
            P::Position { x, y, z, on_ground } => Outbound::Move(MovePacket::Position {
                horizontal_collision: false,
                pos: Vec3::new(x, y, z),
                on_ground,
            }),
            P::Look {
                yaw,
                pitch,
                on_ground,
            } => Outbound::Move(MovePacket::Look {
                horizontal_collision: false,
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
                horizontal_collision: false,
                pos: Vec3::new(x, y, z),
                yaw,
                pitch,
                on_ground,
            }),
            // The modern physics (1.21.2+): one move packet with optional parts, held keys.
            P::Move {
                pos,
                look,
                on_ground,
                horizontal_collision,
            } => Outbound::Move(match (pos, look) {
                (Some([x, y, z]), Some((yaw, pitch))) => MovePacket::PositionLook {
                    pos: Vec3::new(x, y, z),
                    yaw,
                    pitch,
                    on_ground,
                    horizontal_collision,
                },
                (Some([x, y, z]), None) => MovePacket::Position {
                    pos: Vec3::new(x, y, z),
                    on_ground,
                    horizontal_collision,
                },
                (None, Some((yaw, pitch))) => MovePacket::Look {
                    yaw,
                    pitch,
                    on_ground,
                    horizontal_collision,
                },
                (None, None) => MovePacket::Flying {
                    on_ground,
                    horizontal_collision,
                },
            }),
            P::PlayerInput(k) => Outbound::PlayerInput(Controls {
                forward: k.forward,
                back: k.back,
                left: k.left,
                right: k.right,
                jump: k.jump,
                sneak: k.sneak,
                sprint: k.sprint,
            }),
            // A physics packet added later without a wire form here.
            _ => {
                debug_assert!(false, "physics packet without a wire form: {p:?}");
                continue;
            }
        };
        out.push(wire);
    }
}

#[cfg(any(feature = "v1_12_2", feature = "v1_16_5", feature = "v26_1"))]
pub(super) fn physics_abilities(a: Abilities) -> ghast_physics::Abilities {
    ghast_physics::Abilities {
        invulnerable: a.flags & 1 != 0,
        flying: a.flags & 2 != 0,
        allow_flying: a.flags & 4 != 0,
        creative: a.flags & 8 != 0,
        fly_speed: a.flying_speed,
        walk_speed: a.walking_speed,
    }
}

pub(super) fn keys(c: Controls) -> ghast_physics::Keys {
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

#[cfg(any(feature = "v1_12_2", feature = "v1_16_5"))]
pub(super) fn rel_flags(flags: i8) -> [bool; 5] {
    use crate::common::teleport_flags as fl;
    [
        flags & fl::X != 0,
        flags & fl::Y != 0,
        flags & fl::Z != 0,
        flags & fl::YAW != 0,
        flags & fl::PITCH != 0,
    ]
}

/// Chunk packets for tests.
#[cfg(test)]
#[allow(unused_imports, dead_code)] // each helper is used by the tests of its version
pub(crate) mod test_chunks {
    use bytes::{BufMut, Bytes, BytesMut};
    use ghast_protocol::RawPacket;

    #[cfg(feature = "v1_8")]
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

    #[cfg(feature = "v1_12_2")]
    /// A 1.12.2 ground-up chunk with one section (y 48–63) whose top layer (y = 63) is stone.
    pub(crate) fn v1_12_2(x: i32, z: i32) -> RawPacket {
        let mut d = BytesMut::new();
        d.put_u8(4); // bits per block: linear palette
        d.put_u8(2); // palette length (VarInt)
        d.put_u8(0); // air
        d.put_u8(1 << 4); // stone (one byte VarInt: 16)
        d.put_u8(0x80); // data length 256 (VarInt)
        d.put_u8(0x02);
        for w in 0..256 {
            // 16 values of 4 bits per word; words 240.. hold y = 15 (index >> 8 == 15).
            d.put_u64(if w >= 240 { 0x1111_1111_1111_1111 } else { 0 });
        }
        d.put_bytes(0, 2048); // block light
        d.put_bytes(0xff, 2048); // sky light
        d.put_bytes(1, 256); // biomes
        RawPacket::from_packet(&ghast_protocol::v1_12_2::play::clientbound::MapChunk {
            x,
            z,
            ground_up: true,
            bit_map: 1 << 3,
            chunk_data: d.freeze(),
            block_entities: Vec::new(),
        })
        .unwrap()
    }

    #[cfg(feature = "v26_1")]
    /// A 26.1 overworld chunk (y −64 … 319) whose layer y = 63 is stone, the rest air.
    pub(crate) fn v26_1(x: i32, z: i32) -> RawPacket {
        use ghast_protocol::McWrite;
        let stone = ghast_data::v26_1::BlockState::parse("stone").unwrap().id();
        let mut d = BytesMut::new();
        for section in 0..24 {
            if section == 7 {
                // y 48 … 63: a 4-bit linear palette (air, stone), the top layer stone.
                d.put_i16(256);
                d.put_i16(0);
                d.put_u8(4);
                d.put_varint(2);
                d.put_varint(0);
                d.put_varint(i32::from(stone));
                for w in 0..256 {
                    d.put_u64(if w >= 240 { 0x1111_1111_1111_1111 } else { 0 });
                }
            } else {
                d.put_i16(0);
                d.put_i16(0);
                d.put_u8(0);
                d.put_varint(0);
            }
            // Biomes: one value (registry id 0).
            d.put_u8(0);
            d.put_varint(0);
        }
        RawPacket::from_packet(&ghast_protocol::v26_1::play::clientbound::MapChunk {
            x,
            z,
            heightmaps: Vec::new(),
            chunk_data: d.freeze(),
            block_entities: Vec::new(),
            sky_light_mask: Vec::new(),
            block_light_mask: Vec::new(),
            empty_sky_light_mask: Vec::new(),
            empty_block_light_mask: Vec::new(),
            sky_light: Vec::new(),
            block_light: Vec::new(),
        })
        .unwrap()
    }

    #[cfg(feature = "v1_16_5")]
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
    fn column(&self, chunk_x: i32, chunk_z: i32) -> Option<&ghast_world::Column> {
        (**self).column(chunk_x, chunk_z)
    }
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

    fn crosshair(
        &self,
        eyes: Vec3,
        rotation: (f32, f32),
        look: Vec3,
        reach: f64,
    ) -> Option<crate::BlockHit> {
        (**self).crosshair(eyes, rotation, look, reach)
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
    fn respawn_player(&mut self, keep: u8) {
        (**self).respawn_player(keep);
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
    fn world_packet_ids(&self) -> Vec<i32> {
        (**self).world_packet_ids()
    }
    fn entity_config(&self) -> Option<ghast_entity::Config> {
        (**self).entity_config()
    }
    fn set_entities(&mut self, config: Option<ghast_entity::Config>) {
        (**self).set_entities(config);
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
    fn feet_item(&mut self, id: Option<i32>) {
        (**self).feet_item(id);
    }
    fn spectator(&mut self, on: bool) {
        (**self).spectator(on);
    }
    fn held_item(&self, id: u16, count: u8, efficiency: u8) -> HeldItem {
        (**self).held_item(id, count, efficiency)
    }
    fn crosshair_now(
        &self,
        eyes: Vec3,
        rotation: (f32, f32),
        look: Vec3,
        held: Option<u16>,
    ) -> Option<crate::BlockHit> {
        (**self).crosshair_now(eyes, rotation, look, held)
    }
    fn crosshair_target(&self, q: &PickQuery<'_>) -> Crosshair {
        (**self).crosshair_target(q)
    }
    fn pick_margin(&self, e: &ghast_entity::Entity) -> Option<f32> {
        (**self).pick_margin(e)
    }
    fn tick_entities(&mut self) {
        (**self).tick_entities();
    }
    fn level_load_start(&mut self) {
        (**self).level_load_start();
    }
    fn player_alive(&mut self, alive: bool) {
        (**self).player_alive(alive);
    }
    fn load_gate_tick(&mut self, section_compiled: bool, timed_out: bool) -> bool {
        (**self).load_gate_tick(section_compiled, timed_out)
    }
    fn tick_unloaded(&mut self) {
        (**self).tick_unloaded();
    }
    fn attributes(&mut self, attributes: &[crate::common::AttributeUpdate]) {
        (**self).attributes(attributes);
    }
    fn chunk_batch_start(&mut self, now_nanos: i64) {
        (**self).chunk_batch_start(now_nanos);
    }
    fn chunk_batch_finished(&mut self, batch_size: i32, now_nanos: i64) -> f32 {
        (**self).chunk_batch_finished(batch_size, now_nanos)
    }
    fn registry_data(&mut self, raw: &RawPacket) {
        (**self).registry_data(raw);
    }
}

#[cfg(all(test, feature = "v1_8"))]
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
                ..Default::default()
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
                ..Default::default()
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
