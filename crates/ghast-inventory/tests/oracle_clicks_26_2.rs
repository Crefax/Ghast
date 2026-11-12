//! Replays the 26.2 click vectors of the oracle (`oracle/run.sh clicks` with `MC=26.2`,
//! `oracle/src/modern/…/ClickDump.java`): the real client's `handleContainerInput` on every
//! menu type a server opens, filled through real `container_set_content` packets, with every
//! `ContainerInput` (drags included), survival and creative mode.
//!
//! For every click ghast predicts, the encoded `container_click` body must equal the bytes the
//! real client sent (changed slots in its map order, hashed stacks, carried stack), the client
//! must have sent as many arm swings before it, and every slot, the carried stack and the
//! player inventory must equal the client's afterwards (compared as the server compares them:
//! item, count and component hashes). A click ghast sends unpredicted is counted, then the model
//! is resynchronised from the client's state (what the server's corrections do) and the replay
//! continues (skipping to the end of a drag in progress).
//!
//! Committed vectors: `tests/golden/26.2/clicks_seed1.jsonl.gz`. Bigger runs:
//! `GHAST_CLICK_VECTORS_26_2=path cargo test -p ghast-inventory --release --test oracle_clicks_26_2 -- --nocapture`.

#![cfg(feature = "v26_2")]

use ghast_inventory::Version;
use ghast_protocol::DecodeError;
use ghast_protocol::codec::Reader;
use ghast_protocol::types::slot::v775::ItemStack;

const V: Version = Version::V26_2;
const LABEL: &str = "26.2";
const CLICKS: &[u8] = include_bytes!("../../../tests/golden/26.2/clicks_seed1.jsonl.gz");
const CREATIVE: &[u8] = include_bytes!("../../../tests/golden/26.2/clicks_creative_seed1.jsonl.gz");
const ENV: &str = "GHAST_CLICK_VECTORS_26_2";
const DECLARE_RECIPES: i32 = ghast_protocol::v26_2::play::clientbound::id::DECLARE_RECIPES;
const SET_CREATIVE_SLOT: i32 = ghast_protocol::v26_2::play::serverbound::id::SET_CREATIVE_SLOT;

fn read_stack(r: &mut Reader<'_>) -> Result<Option<ItemStack>, DecodeError> {
    ItemStack::read_776(r)
}

include!("common/modern_clicks.rs");
