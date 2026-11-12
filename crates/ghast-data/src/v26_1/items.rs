//! 26.1 items and dig facts, from the jar (oracle `tests/golden/26.1/items.jsonl.gz`).
//!
//! Items: id ↔ name, the default max stack size and max damage, food, the block an item places
//! (`BlockItem`), and the default `minecraft:tool` component. These are the item type's
//! defaults; a server can change any component on a stack (ghast-inventory decodes stack
//! components).
//!
//! Dig facts: per block its destroy time and whether drops need the correct tool; a tool's speed
//! and "correct for drops" come from its rules, each testing a block set (a block tag as the jar
//! resolves it, or a direct block list). [`dig`](super::dig) turns these into vanilla's per-tick
//! destroy progress.

use super::g;
use super::{Block, BlockState};

/// A 26.1 item type (registry id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Item(pub(crate) u16);

pub use super::ItemData;

/// The `minecraft:food` component.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Food {
    /// Hunger points restored.
    pub nutrition: u8,
    /// Saturation restored.
    pub saturation: f32,
    /// Edible with a full hunger bar.
    pub can_always_eat: bool,
}

/// A set of blocks a tool rule tests (`HolderSet<Block>`): a tag or a direct list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockSet(pub(crate) u16);

impl BlockSet {
    /// `true` when the block is in the set.
    #[inline]
    #[must_use]
    pub fn contains(self, b: Block) -> bool {
        let w = &g::items::BLOCK_SETS[self.0 as usize];
        let i = usize::from(b.id());
        w[i / 64] & (1 << (i % 64)) != 0
    }

    /// The tag name (`"minecraft:mineable/pickaxe"`); `None` for a direct block list.
    #[must_use]
    pub fn tag(self) -> Option<&'static str> {
        let n = set_name(self.0 as usize)?;
        (!n.is_empty()).then_some(n)
    }

    /// The set of a block tag the tool rules use, by name (`"minecraft:mineable/axe"`).
    #[must_use]
    pub fn from_tag(name: &str) -> Option<BlockSet> {
        (0..g::items::BLOCK_SET_NAMES.len())
            .position(|i| set_name(i).is_some_and(|n| !n.is_empty() && n == name))
            .map(|i| BlockSet(i as u16))
    }
}

/// One rule of a tool component.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolRule {
    /// The blocks the rule applies to.
    pub blocks: BlockSet,
    /// Mining speed on those blocks.
    pub speed: Option<f32>,
    /// Whether the tool is correct for their drops.
    pub correct_for_drops: Option<bool>,
}

/// The `minecraft:tool` component (`Tool` record).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tool {
    /// Rules in order; the first matching one with the asked field set decides.
    pub rules: &'static [ToolRule],
    /// Speed when no rule with a speed matches.
    pub default_mining_speed: f32,
    /// Durability lost per block broken.
    pub damage_per_block: u32,
    /// Whether a creative player can break blocks while holding it.
    pub can_destroy_blocks_in_creative: bool,
}

impl Tool {
    /// `Tool.getMiningSpeed(state)`.
    #[must_use]
    pub fn mining_speed(&self, b: Block) -> f32 {
        for r in self.rules {
            if let Some(s) = r.speed
                && r.blocks.contains(b)
            {
                return s;
            }
        }
        self.default_mining_speed
    }

    /// `Tool.isCorrectForDrops(state)`.
    #[must_use]
    pub fn correct_for_drops(&self, b: Block) -> bool {
        for r in self.rules {
            if let Some(c) = r.correct_for_drops
                && r.blocks.contains(b)
            {
                return c;
            }
        }
        false
    }
}

impl Item {
    /// Air (the empty stack's item).
    pub const AIR: Item = Item(0);

    /// The item with this id, if it exists.
    #[must_use]
    pub fn from_id(id: u16) -> Option<Item> {
        (usize::from(id) < g::items::ITEMS.len()).then_some(Item(id))
    }

    /// Looks an item up by registry name (`"diamond_pickaxe"`, no namespace).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Item> {
        crate::common::lookup_name(g::items::ITEM_NAMES_TEXT, &g::items::ITEM_NAMES, name).map(Item)
    }

    /// Registry id.
    #[must_use]
    pub const fn id(self) -> u16 {
        self.0
    }

    /// Static data.
    #[must_use]
    pub fn data(self) -> &'static ItemData {
        &g::items::ITEMS[self.0 as usize]
    }

    /// Registry name.
    #[must_use]
    pub fn name(self) -> &'static str {
        super::item_name(self.0)
    }

    /// English display name.
    #[cfg(feature = "display-names")]
    #[must_use]
    pub fn display_name(self) -> &'static str {
        super::item_display_name(self.0)
    }

    /// Default max stack size.
    #[must_use]
    pub fn max_stack_size(self) -> u8 {
        self.data().max_stack_size
    }

    /// The block this item places (`BlockItem.getBlock`).
    #[must_use]
    pub fn block(self) -> Option<Block> {
        self.data().block.map(Block)
    }

    /// Default food component.
    #[must_use]
    pub fn food(self) -> Option<Food> {
        self.data().food
    }

    /// Default tool component.
    #[must_use]
    pub fn tool(self) -> Option<&'static Tool> {
        self.data().tool.map(|i| &g::items::TOOLS[i as usize])
    }

    /// Default destroy speed of this item on a block (`Item.getDestroySpeed`: the tool's speed,
    /// else 1).
    #[must_use]
    pub fn destroy_speed(self, b: Block) -> f32 {
        self.tool().map_or(1.0, |t| t.mining_speed(b))
    }

    /// Default "correct tool for drops" (`Item.isCorrectToolForDrops`).
    #[must_use]
    pub fn correct_for_drops(self, b: Block) -> bool {
        self.tool().is_some_and(|t| t.correct_for_drops(b))
    }

    /// All items in id order.
    pub fn all() -> impl ExactSizeIterator<Item = Item> {
        (0..g::items::ITEMS.len() as u16).map(Item)
    }
}

/// Food data of an item (default component).
#[must_use]
pub fn food(item: Item) -> Option<Food> {
    item.food()
}

impl Block {
    /// Destroy time (vanilla "hardness" / `getDestroySpeed`), from the jar; `-1.0` = unbreakable.
    #[must_use]
    pub fn destroy_time(self) -> f32 {
        g::items::BLOCK_DESTROY_TIME[self.0 as usize]
    }

    /// Drops need the correct tool (`requiresCorrectToolForDrops`).
    #[must_use]
    pub fn requires_correct_tool_for_drops(self) -> bool {
        let i = self.0 as usize;
        g::items::BLOCK_REQUIRES_TOOL[i / 64] & (1 << (i % 64)) != 0
    }

    /// The block's item form (`Block.asItem`; [`Item::AIR`] when it has none).
    #[must_use]
    pub fn item(self) -> Item {
        Item(g::items::BLOCK_ITEM[self.0 as usize])
    }
}

impl BlockState {
    /// [`Block::destroy_time`] (the same for every state in 26.1, checked by the generator).
    #[must_use]
    pub fn destroy_time(self) -> f32 {
        self.block().destroy_time()
    }

    /// [`Block::requires_correct_tool_for_drops`].
    #[must_use]
    pub fn requires_correct_tool_for_drops(self) -> bool {
        self.block().requires_correct_tool_for_drops()
    }
}

/// The name of block set `i` (empty for a direct block list).
fn set_name(i: usize) -> Option<&'static str> {
    let end = usize::from(*g::items::BLOCK_SET_NAMES.get(i)?);
    let start = if i == 0 {
        0
    } else {
        usize::from(g::items::BLOCK_SET_NAMES[i - 1])
    };
    g::items::BLOCK_SET_NAMES_TEXT.get(start..end)
}
