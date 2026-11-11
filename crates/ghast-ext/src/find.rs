//! Blocks and entities: naming raw states, searching the loaded world, entity snapshots.

use ghast_client::entity::{EntityClass, EntityType};
use ghast_client::{BlockPos, GameVersion, TickContext, Vec3};

use crate::{Bot, Error};

/// A block state of the session's version, named with ghast-data.
///
/// ```
/// use ghast_client::GameVersion;
/// use ghast_ext::Block;
///
/// let ore = Block::new(GameVersion::V1_8_9, 56 << 4); // 1.8 diamond ore
/// assert_eq!(ore.name(), "diamond_ore");
/// assert!(ore.is_ore());
/// assert!(Block::new(GameVersion::V1_8_9, 0).is_air());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Block {
    version: GameVersion,
    raw: u32,
}

impl Block {
    /// The block for a raw state of `version` (1.8 `id << 4 | meta`, 1.16 the global
    /// palette id), as [`ghast_client::WorldView::raw_block`] returns it.
    ///
    /// ```
    /// # use ghast_client::GameVersion;
    /// let b = ghast_ext::Block::new(GameVersion::V1_8_9, 1 << 4);
    /// assert_eq!(b.name(), "stone");
    /// ```
    #[must_use]
    pub const fn new(version: GameVersion, raw: u32) -> Self {
        Block { version, raw }
    }

    /// The raw state.
    ///
    /// ```
    /// # use ghast_client::GameVersion;
    /// assert_eq!(ghast_ext::Block::new(GameVersion::V1_8_9, 17).raw(), 17);
    /// ```
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.raw
    }

    /// The registry name without namespace (`"stone"`, `"oak_log"`; 1.8: `"log"`), or `""`
    /// for an id this version does not know.
    ///
    /// ```
    /// # use ghast_client::GameVersion;
    /// assert_eq!(ghast_ext::Block::new(GameVersion::V1_8_9, 17 << 4).name(), "log");
    /// ```
    #[must_use]
    pub fn name(self) -> &'static str {
        match self.version {
            #[cfg(feature = "v1_8")]
            GameVersion::V1_8_9 => u8::try_from(self.raw >> 4)
                .ok()
                .and_then(ghast_data::v1_8::Block::from_id)
                .map_or("", ghast_data::v1_8::Block::name),
            #[cfg(feature = "v1_12_2")]
            GameVersion::V1_12_2 => u8::try_from(self.raw >> 4)
                .ok()
                .and_then(ghast_data::v1_12::Block::from_id)
                .map_or("", ghast_data::v1_12::Block::name),
            #[cfg(feature = "v1_16_5")]
            GameVersion::V1_16_5 => u16::try_from(self.raw)
                .ok()
                .and_then(ghast_data::v1_16_5::BlockState::from_id)
                .map_or("", |s| s.block().name()),
            #[cfg(feature = "v26_1")]
            GameVersion::V26_1 => u16::try_from(self.raw)
                .ok()
                .and_then(ghast_data::v26_1::BlockState::from_id)
                .map_or("", |s| s.block().name()),
            _ => "",
        }
    }

    /// The 1.8 / 1.12 metadata (`raw & 15`); 0 on 1.16 (use ghast-data's properties there).
    ///
    /// ```
    /// # use ghast_client::GameVersion;
    /// assert_eq!(ghast_ext::Block::new(GameVersion::V1_8_9, 59 << 4 | 7).meta(), 7); // ripe wheat
    /// ```
    #[must_use]
    pub fn meta(self) -> u8 {
        match self.version {
            GameVersion::V1_8_9 | GameVersion::V1_12_2 => (self.raw & 15) as u8,
            _ => 0,
        }
    }

    /// Air (also cave / void air on 1.16).
    ///
    /// ```
    /// # use ghast_client::GameVersion;
    /// assert!(ghast_ext::Block::new(GameVersion::V1_16_5, 0).is_air());
    /// ```
    #[must_use]
    pub fn is_air(self) -> bool {
        matches!(self.name(), "air" | "cave_air" | "void_air")
    }

    /// An ore (`*_ore`, nether quartz / gold ore, ancient debris; not the lit redstone ore
    /// variant's name difference on 1.8: `lit_redstone_ore` counts too).
    ///
    /// ```
    /// # use ghast_client::GameVersion;
    /// assert!(ghast_ext::Block::new(GameVersion::V1_8_9, 15 << 4).is_ore()); // iron ore
    /// ```
    #[must_use]
    pub fn is_ore(self) -> bool {
        let n = self.name();
        n.ends_with("_ore") || n == "ancient_debris"
    }

    /// A log / stem / hyphae block (tree trunks).
    ///
    /// ```
    /// # use ghast_client::GameVersion;
    /// assert!(ghast_ext::Block::new(GameVersion::V1_8_9, 17 << 4).is_log());
    /// ```
    #[must_use]
    pub fn is_log(self) -> bool {
        let n = self.name();
        n == "log"
            || n == "log2"
            || n.ends_with("_log")
            || n.ends_with("_stem")
            || n.ends_with("_hyphae")
            || n.ends_with("_wood")
    }

    /// Water or lava.
    ///
    /// ```
    /// # use ghast_client::GameVersion;
    /// assert!(ghast_ext::Block::new(GameVersion::V1_8_9, 9 << 4).is_liquid());
    /// ```
    #[must_use]
    pub fn is_liquid(self) -> bool {
        matches!(
            self.name(),
            "water" | "flowing_water" | "lava" | "flowing_lava"
        )
    }
}

/// What [`Bot::find_blocks`] looks for: a name, several names, or a predicate.
#[non_exhaustive]
pub enum BlockMatch {
    /// One registry name (`"diamond_ore"`, `"minecraft:chest"`).
    Name(String),
    /// Any of these names.
    Names(Vec<String>),
    /// A predicate on the block.
    Pred(Box<dyn Fn(Block) -> bool + Send + Sync>),
}

impl std::fmt::Debug for BlockMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockMatch::Name(n) => f.debug_tuple("Name").field(n).finish(),
            BlockMatch::Names(n) => f.debug_tuple("Names").field(n).finish(),
            BlockMatch::Pred(_) => f.write_str("Pred(..)"),
        }
    }
}

impl BlockMatch {
    fn matches(&self, b: Block) -> bool {
        match self {
            BlockMatch::Name(n) => strip(n) == b.name(),
            BlockMatch::Names(ns) => ns.iter().any(|n| strip(n) == b.name()),
            BlockMatch::Pred(p) => p(b),
        }
    }
}

fn strip(n: &str) -> &str {
    n.strip_prefix("minecraft:").unwrap_or(n)
}

impl From<&str> for BlockMatch {
    fn from(n: &str) -> Self {
        BlockMatch::Name(n.to_owned())
    }
}

impl From<String> for BlockMatch {
    fn from(n: String) -> Self {
        BlockMatch::Name(n)
    }
}

impl From<&[&str]> for BlockMatch {
    fn from(ns: &[&str]) -> Self {
        BlockMatch::Names(ns.iter().map(|s| (*s).to_owned()).collect())
    }
}

impl<const N: usize> From<[&str; N]> for BlockMatch {
    fn from(ns: [&str; N]) -> Self {
        BlockMatch::Names(ns.iter().map(|s| (*s).to_owned()).collect())
    }
}

impl<F: Fn(Block) -> bool + Send + Sync + 'static> From<F> for BlockMatch {
    fn from(f: F) -> Self {
        BlockMatch::Pred(Box::new(f))
    }
}

/// Matching blocks within `max_distance` of `center` (block centres, Euclidean), nearest
/// first, at most `count`. Unloaded columns are skipped; all-air sections are skipped
/// unless air matches. The predicate runs once per distinct state.
pub(crate) fn find_blocks(
    ctx: &TickContext<'_>,
    m: &BlockMatch,
    center: Vec3,
    max_distance: f64,
    count: usize,
) -> Vec<BlockPos> {
    let v = ctx.version();
    let w = ctx.world();
    let r = max_distance.max(0.0);
    let r2 = r * r;
    // Per raw state: 0 unknown, 1 no, 2 yes.
    let mut memo = vec![0u8; 1 << 16];
    let mut test = |raw: u32| -> bool {
        let i = (raw as usize) & 0xffff;
        if memo[i] == 0 {
            memo[i] = if m.matches(Block::new(v, raw)) { 2 } else { 1 };
        }
        memo[i] == 2
    };
    let air_matches = test(0);
    let (min_y, max_y) = (
        ((center.y - r).floor() as i32).max(0),
        ((center.y + r).floor() as i32).min(255),
    );
    let (x0, x1) = ((center.x - r).floor() as i32, (center.x + r).floor() as i32);
    let (z0, z1) = ((center.z - r).floor() as i32, (center.z + r).floor() as i32);
    let mut found: Vec<(f64, BlockPos)> = Vec::new();
    for cx in (x0 >> 4)..=(x1 >> 4) {
        for cz in (z0 >> 4)..=(z1 >> 4) {
            if !w.is_column_loaded(cx, cz) {
                continue;
            }
            let col = w.column(cx, cz);
            for sy in (min_y >> 4)..=(max_y >> 4) {
                let section = col.map(|c| c.section(sy as usize));
                if matches!(section, Some(None)) && !air_matches {
                    continue; // all air
                }
                for y in (sy << 4).max(min_y)..=((sy << 4) + 15).min(max_y) {
                    let dy = f64::from(y) + 0.5 - center.y;
                    for z in ((cz << 4).max(z0))..=(((cz << 4) + 15).min(z1)) {
                        let dz = f64::from(z) + 0.5 - center.z;
                        for x in ((cx << 4).max(x0))..=(((cx << 4) + 15).min(x1)) {
                            let dx = f64::from(x) + 0.5 - center.x;
                            let d2 = dx * dx + dy * dy + dz * dz;
                            if d2 > r2 {
                                continue;
                            }
                            let raw = match col {
                                Some(c) => u32::from(c.raw(x, y, z)),
                                None => match w.raw_block(x, y, z) {
                                    Some(r) => r,
                                    None => continue,
                                },
                            };
                            if test(raw) {
                                found.push((d2, BlockPos::new(x, y, z)));
                            }
                        }
                    }
                }
            }
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.truncate(count);
    found.into_iter().map(|(_, p)| p).collect()
}

/// A snapshot of a tracked entity.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EntityInfo {
    /// Entity id.
    pub id: i32,
    /// Registry / minecraft-data name (`"player"`, `"zombie"`, `"item"`, `"arrow"`, ...;
    /// 1.8 names as minecraft-data has them), `""` when unknown.
    pub name: &'static str,
    /// Broad class.
    pub class: EntityClass,
    /// Position (the server's, feet).
    pub pos: Vec3,
    /// A player's tab-list name ([`ghast_client::Interests::PLAYER_LIST`]).
    pub player_name: Option<String>,
    /// An item entity's stack: item id and count.
    pub item: Option<(i32, i32)>,
}

/// The name of an entity type (1.8 and 1.12 share the legacy mob / object id spaces but not
/// the names).
fn entity_name(version: GameVersion, ty: EntityType) -> &'static str {
    let info: Option<&'static ghast_data::EntityInfo> = match (ty, version) {
        (EntityType::Player, _) => return "player",
        #[cfg(feature = "v26_1")]
        (EntityType::Registry(_), GameVersion::V26_1) => ghast_client::entity::v26_1::info(ty),
        #[cfg(feature = "v1_16_5")]
        (EntityType::Registry(_), _) => ghast_client::entity::v1_16_5::info(ty),
        #[cfg(feature = "v1_12_2")]
        (_, GameVersion::V1_12_2) => ghast_client::entity::v1_12::info(ty),
        #[cfg(feature = "v1_8")]
        (_, GameVersion::V1_8_9) => ghast_client::entity::v1_8::info(ty),
        _ => None,
    };
    info.map_or("", |i| i.name)
}

pub(crate) fn entity_info(ctx: &TickContext<'_>, id: i32) -> Option<EntityInfo> {
    let es = ctx.entities()?;
    let e = es.get(id)?;
    let [x, y, z] = e.position();
    Some(EntityInfo {
        id,
        name: entity_name(ctx.version(), e.entity_type()),
        class: e.class(),
        pos: Vec3::new(x, y, z),
        player_name: es.player_name(id).map(str::to_owned),
        item: e.item().map(|i| (i.id, i32::from(i.count))),
    })
}

impl Bot {
    /// The block at `pos`; `None` when its column is not loaded (never air by guess).
    ///
    /// ```no_run
    /// use ghast_client::BlockPos;
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// if let Some(b) = bot.block_at(BlockPos::new(0, 63, 0)).await? {
    ///     println!("{}", b.name());
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn block_at(&self, pos: BlockPos) -> Result<Option<Block>, Error> {
        self.with(move |ctx| {
            let v = ctx.version();
            ctx.world()
                .raw_block(pos.x, pos.y, pos.z)
                .map(|raw| Block::new(v, raw))
        })
        .await
    }

    /// Up to `count` blocks matching `what` within `max_distance` of the eyes, nearest first
    /// (loaded columns only). `what` is a name, an array of names or a predicate on
    /// [`Block`]. The search runs inside one tick: keep `max_distance` modest (32 blocks is
    /// ~270k positions; all-air sections are skipped).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let ores = bot.find_blocks(|b: ghast_ext::Block| b.is_ore(), 32.0, 10).await?;
    /// let chests = bot.find_blocks("chest", 16.0, 1).await?;
    /// let logs = bot.find_blocks(["log", "log2"], 24.0, 64).await?;
    /// # Ok(()) }
    /// ```
    pub async fn find_blocks(
        &self,
        what: impl Into<BlockMatch>,
        max_distance: f64,
        count: usize,
    ) -> Result<Vec<BlockPos>, Error> {
        let m = what.into();
        self.with(move |ctx| {
            let eyes = ctx.eyes(ctx.pose());
            find_blocks(ctx, &m, eyes, max_distance, count)
        })
        .await
    }

    /// The nearest block matching `what` within `max_distance` (see [`Bot::find_blocks`]).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// if let Some(chest) = bot.find_block("chest", 16.0).await? { /* ... */ }
    /// # Ok(()) }
    /// ```
    pub async fn find_block(
        &self,
        what: impl Into<BlockMatch>,
        max_distance: f64,
    ) -> Result<Option<BlockPos>, Error> {
        Ok(self
            .find_blocks(what, max_distance, 1)
            .await?
            .into_iter()
            .next())
    }

    /// Tracked entities (not the bot) within `radius` of the bot matching `pred`, nearest
    /// first. Needs [`ghast_client::Interests::ENTITIES`] ([`crate::Ext::interests`]);
    /// empty without it.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let zombies = bot.find_entities(|e| e.name == "zombie", 16.0).await?;
    /// # Ok(()) }
    /// ```
    pub async fn find_entities<F>(&self, pred: F, radius: f64) -> Result<Vec<EntityInfo>, Error>
    where
        F: Fn(&EntityInfo) -> bool + Send + 'static,
    {
        self.with(move |ctx| {
            let me = ctx.pose().pos;
            let Some(es) = ctx.entities() else {
                return Vec::new();
            };
            let self_id = es.self_id();
            let ids: Vec<i32> = es
                .within([me.x, me.y, me.z], radius)
                .map(|e| e.id())
                .filter(|id| Some(*id) != self_id)
                .collect();
            let mut v: Vec<(f64, EntityInfo)> = ids
                .into_iter()
                .filter_map(|id| entity_info(ctx, id))
                .filter(|e| pred(e))
                .map(|e| {
                    let d = (e.pos.x - me.x).powi(2)
                        + (e.pos.y - me.y).powi(2)
                        + (e.pos.z - me.z).powi(2);
                    (d, e)
                })
                .collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
            v.into_iter().map(|(_, e)| e).collect()
        })
        .await
    }

    /// The nearest tracked entity within `radius` matching `pred` (see
    /// [`Bot::find_entities`]).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let player = bot.nearest_entity(|e| e.name == "player", 32.0).await?;
    /// # Ok(()) }
    /// ```
    pub async fn nearest_entity<F>(&self, pred: F, radius: f64) -> Result<Option<EntityInfo>, Error>
    where
        F: Fn(&EntityInfo) -> bool + Send + 'static,
    {
        Ok(self.find_entities(pred, radius).await?.into_iter().next())
    }

    /// A tracked entity by id.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot, id: i32) -> Result<(), ghast_ext::Error> {
    /// if let Some(e) = bot.entity(id).await? { println!("{:?}", e.pos); }
    /// # Ok(()) }
    /// ```
    pub async fn entity(&self, id: i32) -> Result<Option<EntityInfo>, Error> {
        self.with(move |ctx| entity_info(ctx, id)).await
    }

    /// The player entity with this tab-list name (needs [`ghast_client::Interests::ENTITIES`]
    /// and [`ghast_client::Interests::PLAYER_LIST`]).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// if let Some(p) = bot.player("Steve").await? { println!("Steve at {:?}", p.pos); }
    /// # Ok(()) }
    /// ```
    pub async fn player(&self, name: &str) -> Result<Option<EntityInfo>, Error> {
        let name = name.to_owned();
        self.with(move |ctx| {
            let es = ctx.entities()?;
            let id = es
                .iter()
                .filter(|e| e.is_player())
                .find(|e| {
                    es.player_name(e.id())
                        .is_some_and(|n| n.eq_ignore_ascii_case(&name))
                })?
                .id();
            entity_info(ctx, id)
        })
        .await
    }
}
