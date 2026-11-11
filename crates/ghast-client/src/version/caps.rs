//! The cross-layer version row: what a protocol version can do, as facts the version-agnostic
//! core asks for instead of comparing version numbers.
//!
//! One constant row per [`GameVersion`]. Adding a version is adding a row (plus whatever is
//! really new); the core never names a version. Every field is a plain fact of the vanilla
//! client of that version.

use super::GameVersion;

/// The physics rule set a version runs on (`ghast-physics` module).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PhysicsRules {
    /// `ghast_physics::v1_8` (1.8 – 1.8.9).
    V1_8,
    /// `ghast_physics::v1_12` (1.12.2).
    V1_12,
    /// `ghast_physics::v1_16_5` (1.16.4 – 1.16.5).
    V1_16_5,
    /// `ghast_physics::modern` with the `V26_1` rules (26.1 – 26.1.2).
    Modern26_1,
}

/// The data tables of a version (`ghast-data` module).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DataSet {
    /// `ghast_data::v1_8`: numeric block / item ids with metadata.
    V1_8,
    /// `ghast_data::v1_12`: numeric block / item ids with metadata.
    V1_12,
    /// `ghast_data::v1_16_5`: flattened block states and item ids.
    V1_16_5,
    /// `ghast_data::v26_1`: flattened block states and item ids.
    V26_1,
}

/// How item stacks carry their extra data on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ItemModel {
    /// Numeric id, damage value and an optional NBT compound (1.8 – 1.12).
    LegacyNbt,
    /// Flattened id and an optional NBT compound (1.13 – 1.20.4).
    FlatNbt,
    /// Flattened id and data components (1.20.5+).
    Components,
}

/// What a protocol version can do: the facts the client core asks for.
///
/// Read one with [`GameVersion::caps`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Capabilities {
    /// The version this row describes.
    pub version: GameVersion,
    /// Protocol number.
    pub protocol: i32,
    /// Physics rule set.
    pub physics: PhysicsRules,
    /// Data tables.
    pub data: DataSet,
    /// How item stacks carry extra data.
    pub item_model: ItemModel,
    /// 1.9 combat: attack cooldown, swing sent after the click packet, the off hand.
    pub combat_1_9: bool,
    /// The crosshair pick skips entities with the player's root vehicle (1.9+), not only the
    /// vehicle the player sits on (1.8).
    pub entity_pick_root_vehicle: bool,
    /// Eye height above the feet while standing.
    pub eye_height: f64,
    /// Eye height while sneaking (1.54 in 1.8 – 1.12, the crouching pose's 1.27 since 1.14).
    pub sneak_eye_height: f64,
    /// Longest chat line the server accepts, in UTF-16 units.
    pub chat_limit: usize,
    /// Game event 11 ("immediate respawn" game rule) exists (1.15+).
    pub immediate_respawn_rule: bool,
    /// The client uses a seed item at once when planting on its soil (the 1.8 `SeedItem.use`
    /// prediction; later versions wait for the server's slot update).
    pub seed_planting_prediction: bool,
    /// Server teleports carry an id the client confirms (1.9+).
    pub teleport_confirm: bool,
    /// Window clicks are confirmed with transactions (1.8 – 1.16).
    pub window_confirm: bool,
    /// A configuration state sits between login and play (1.20.2+).
    pub configuration_state: bool,
    /// Sneaking is sent as `player_input` key state, not as an entity action (1.21.2+).
    pub sneak_via_player_input: bool,
    /// The client sends `tick_end` after every tick (1.21.2+).
    pub sends_tick_end: bool,
    /// The player does not tick until the client sends `player_loaded` (1.21.4+).
    pub player_loaded_gate: bool,
    /// Chunk batches are acknowledged with the desired chunks per tick (1.20.2+).
    pub chunk_batch_ack: bool,
    /// Dig / place / use packets carry a sequence number the server acknowledges (1.19+).
    pub block_sequence: bool,
    /// Chat packets carry a timestamp, salt and message acknowledgements (1.19.3+).
    pub chat_acknowledgements: bool,
}

const fn f(v: f32) -> f64 {
    v as f64
}

/// 1.8 – 1.8.9 (protocol 47).
pub(crate) const V1_8_9: Capabilities = Capabilities {
    version: GameVersion::V1_8_9,
    protocol: 47,
    physics: PhysicsRules::V1_8,
    data: DataSet::V1_8,
    item_model: ItemModel::LegacyNbt,
    combat_1_9: false,
    entity_pick_root_vehicle: false,
    eye_height: f(1.62),
    // `1.62F - 0.08F` in float.
    sneak_eye_height: f(1.62 - 0.08),
    chat_limit: 100,
    immediate_respawn_rule: false,
    seed_planting_prediction: true,
    teleport_confirm: false,
    window_confirm: true,
    configuration_state: false,
    sneak_via_player_input: false,
    sends_tick_end: false,
    player_loaded_gate: false,
    chunk_batch_ack: false,
    block_sequence: false,
    chat_acknowledgements: false,
};

/// 1.12.2 (protocol 340).
pub(crate) const V1_12_2: Capabilities = Capabilities {
    version: GameVersion::V1_12_2,
    protocol: 340,
    physics: PhysicsRules::V1_12,
    data: DataSet::V1_12,
    combat_1_9: true,
    entity_pick_root_vehicle: true,
    chat_limit: 256,
    seed_planting_prediction: false,
    teleport_confirm: true,
    ..V1_8_9
};

/// 1.16.4 – 1.16.5 (protocol 754).
pub(crate) const V1_16_5: Capabilities = Capabilities {
    version: GameVersion::V1_16_5,
    protocol: 754,
    physics: PhysicsRules::V1_16_5,
    data: DataSet::V1_16_5,
    item_model: ItemModel::FlatNbt,
    sneak_eye_height: f(1.27),
    immediate_respawn_rule: true,
    ..V1_12_2
};

/// 26.1 – 26.1.2 (protocol 775).
pub(crate) const V26_1: Capabilities = Capabilities {
    version: GameVersion::V26_1,
    protocol: 775,
    physics: PhysicsRules::Modern26_1,
    data: DataSet::V26_1,
    item_model: ItemModel::Components,
    window_confirm: false,
    configuration_state: true,
    sneak_via_player_input: true,
    sends_tick_end: true,
    player_loaded_gate: true,
    chunk_batch_ack: true,
    block_sequence: true,
    chat_acknowledgements: true,
    ..V1_16_5
};
