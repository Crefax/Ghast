//! Lab tool: 26.1 click prediction and click hashes against a live vanilla 26.1.x server.
//!
//! ```text
//! cargo run --release -p ghast-inventory --example live_clicks_26_1 -- 127.0.0.1:25565 [name] [capture.bin.zz]
//! ```
//!
//! With a capture path, the configuration registries, the inventory packets of both directions
//! and nothing else are written there (record format of `ghast-protocol/examples/login_26_1.rs`)
//! for `tests/v26_1.rs`.
//!
//! Logs in offline, waits for the console to give the player items (see `lab` notes in the
//! crate README), then runs click steps in window 0 and in a chest next to the player. After
//! each click the server sends `container_set_slot` / `set_cursor_item` only for slots whose
//! hash differs from its own result, so every correction it sends is a misprediction or a hash
//! ghast got wrong; the step's line reports them.

use ghast_inventory::Version;
use ghast_protocol::v26_1 as proto;

const V: Version = Version::V26_1;
const PROTOCOL: i32 = 775;
/// `minecraft:villager` in the 26.1 entity type registry.
const VILLAGER: i32 = 139;

include!("common/live_clicks_modern.rs");
