//! 26.1 sessions on the page: 26.1 blocks, items and entity types mapped onto the 1.8 assets
//! the page draws with (the shared modern mapping, see the module items). Exact where 1.8 has
//! the block or item (pre-flattening `id:meta` from minecraft-data's legacy table, matched
//! property by property); blocks and items added later become a look-alike (new woods → old
//! woods, deepslate → stone, copper → orange terracotta, concrete → stained clay, any stair /
//! slab / fence / door ... → a 1.8 one of the same shape); anything else degrades to stone (a
//! full cube), a stone slab (a low shape), water / lava (a fluid) or air (no collision).
//!
//! The page's world is 256 blocks high; [`Window`] picks the 256 rows of the dimension around
//! the bot that are shown (`y` on the page = world `y − lo`).

use ghast_data::v26_1 as d26;
use ghast_entity::v26_1 as ent;

/// The inventory version of this mapping.
const INV: ghast_inventory::Version = ghast_inventory::Version::V26_1;

#[allow(clippy::duplicate_mod)]
#[path = "modern_map.rs"]
mod imp;

pub use crate::window::Window;
pub use imp::{
    block, block_is_stand_in, entity_is_stand_in, entity_name, entity_text, item, item_drawn,
    stack_text,
};

/// This version's mapping, for the session code.
pub(crate) const MAP: crate::ModernMap = crate::ModernMap {
    block: imp::block,
    item: imp::item,
    entity_text: imp::entity_text,
    stack_text: |st, names| imp::stack_text(st, names),
    entity_name: imp::entity_name,
    info: ent::info,
    item_stack: ent::item_stack,
    flags: ent::flags,
    inventory: INV,
    game: "26.1",
};
