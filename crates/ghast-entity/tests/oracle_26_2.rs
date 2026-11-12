//! The 26.2 tracker against the real client: replays `tests/golden/26.2/entity_tracking.jsonl.gz`
//! (oracle `EntityTrackDump`: random entity packets handled by the real `ClientPacketListener`,
//! entity state recorded at pick time every tick) and compares every entity bit for bit:
//! position, rotation, head rotation, box, on-ground, pickable, chunk ticking, riding links, item
//! counts, and the tab list. Each case runs with metadata tracking on and off.
//!
//! `ORACLE_TRACKING=<file>` replays another dump (e.g. a large one under `oracle/cache/26.2/`).

mod common;

use std::io::Read as _;
use std::path::PathBuf;

use bytes::Bytes;
use common::W;
use ghast_data::v26_2 as data_v;
use ghast_entity::v26_2::{self as ver, Tracker};
use ghast_entity::{Config, Entity, EntityType};
use ghast_protocol::v26_2::play::clientbound::id;
use serde_json::Value;

/// The golden dump replayed by default.
const GOLDEN: &str = "../../tests/golden/26.2/entity_tracking.jsonl.gz";
/// Known tick-order misses (passengers are not placed on their vehicles).
const ORDER_RATCHET: usize = 150;

fn login(player: i32) -> W {
    W::new()
        .i32(player)
        .bool(false)
        .varint(1)
        .str("minecraft:overworld")
        .varint(20)
        .varint(12)
        .varint(12)
        .bool(false)
        .bool(true)
        .bool(false)
        // spawn info: dimension type, dimension, seed, game mode, previous, debug, flat, death,
        // portal cooldown, sea level
        .varint(0)
        .str("minecraft:overworld")
        .i64(0)
        .i8(0)
        .u8(0xFF)
        .bool(false)
        .bool(false)
        .bool(false)
        .varint(0)
        .varint(63)
        // online mode (new in 26.2), enforces secure chat
        .bool(false)
        .bool(false)
}

/// The dump's world for `Level.noCollision`: the 7 x 7 stone floor at y 60 (no collidable
/// entities), through ghast-physics' block collision query.
fn world() -> ghast_physics::modern::MemoryWorld262 {
    let mut w = ghast_physics::modern::memory_world_26_2();
    w.load_chunks(-15, -15, 15, 15);
    w.fill_raw([-3, 60, -3], [3, 60, 3], 1); // stone
    w
}

fn no_collision(
    w: &ghast_physics::modern::MemoryWorld262,
) -> impl Fn(&Entity, [f64; 6]) -> bool + '_ {
    use ghast_physics::modern::{CollisionContext, LevelInfo, V26_2, no_block_collision};
    move |e, bb| {
        let ctx = CollisionContext {
            feet_y: e.position()[1],
            ..CollisionContext::default()
        };
        no_block_collision::<V26_2, _>(w, &LevelInfo::default(), e.position(), bb, &ctx)
    }
}

include!("oracle_body/tracking.rs");
