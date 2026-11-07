//! Items: matching by name, equipping, window clicks, containers.

use std::sync::Arc;

use ghast_client::inventory::{Click, ClickError, ClickOutcome, Stack, WindowKind};
use ghast_client::{BlockPos, ClickRefused, GameVersion, TickContext};

use crate::{Bot, Error};

/// Ticks a click may wait for its turn (still look, previous confirm, same-tick rules).
const CLICK_WAIT: u64 = 100;

/// What an item helper looks for: a registry name, an item id of the session's version,
/// or a predicate on the stack.
#[derive(Clone)]
#[non_exhaustive]
pub enum ItemMatch {
    /// A registry name (`"diamond_pickaxe"`, `"minecraft:bread"`), resolved per version.
    Name(String),
    /// An item id of the session's version.
    Id(i32),
    /// A predicate.
    Pred(Arc<dyn Fn(&Stack) -> bool + Send + Sync>),
}

impl std::fmt::Debug for ItemMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemMatch::Name(n) => f.debug_tuple("Name").field(n).finish(),
            ItemMatch::Id(i) => f.debug_tuple("Id").field(i).finish(),
            ItemMatch::Pred(_) => f.write_str("Pred(..)"),
        }
    }
}

impl std::fmt::Display for ItemMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemMatch::Name(n) => f.write_str(n),
            ItemMatch::Id(i) => write!(f, "item id {i}"),
            ItemMatch::Pred(_) => f.write_str("matching item"),
        }
    }
}

impl From<&str> for ItemMatch {
    fn from(n: &str) -> Self {
        ItemMatch::Name(n.to_owned())
    }
}

impl From<String> for ItemMatch {
    fn from(n: String) -> Self {
        ItemMatch::Name(n)
    }
}

impl From<i32> for ItemMatch {
    fn from(id: i32) -> Self {
        ItemMatch::Id(id)
    }
}

impl<F: Fn(&Stack) -> bool + Send + Sync + 'static> From<F> for ItemMatch {
    fn from(f: F) -> Self {
        ItemMatch::Pred(Arc::new(f))
    }
}

/// The item id of a registry name in `version`.
///
/// ```
/// use ghast_client::GameVersion;
/// assert_eq!(ghast_ext::item_id(GameVersion::V1_8_9, "diamond_pickaxe"), Some(278));
/// assert!(ghast_ext::item_id(GameVersion::V1_16_5, "minecraft:diamond_pickaxe").is_some());
/// assert_eq!(ghast_ext::item_id(GameVersion::V1_8_9, "no_such_item"), None);
/// ```
#[must_use]
pub fn item_id(version: GameVersion, name: &str) -> Option<i32> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    match version {
        GameVersion::V1_8_9 => ghast_data::v1_8::Item::from_name(n).map(|i| i32::from(i.id())),
        GameVersion::V1_12_2 => ghast_data::v1_12::Item::from_name(n).map(|i| i32::from(i.id())),
        _ => ghast_data::v1_16_5::Item::from_name(n).map(|i| i32::from(i.id())),
    }
}

/// The registry name of an item id in `version` (`""` when unknown).
///
/// ```
/// use ghast_client::GameVersion;
/// assert_eq!(ghast_ext::item_name(GameVersion::V1_8_9, 278), "diamond_pickaxe");
/// ```
#[must_use]
pub fn item_name(version: GameVersion, id: i32) -> &'static str {
    let Ok(id) = u16::try_from(id) else {
        return "";
    };
    match version {
        GameVersion::V1_8_9 => ghast_data::v1_8::Item::from_id(id).map_or("", |i| i.info().name),
        GameVersion::V1_12_2 => ghast_data::v1_12::Item::from_id(id).map_or("", |i| i.info().name),
        _ => ghast_data::v1_16_5::Item::from_id(id).map_or("", |i| i.info().name),
    }
}

/// A resolved [`ItemMatch`].
#[derive(Clone)]
pub(crate) struct Matcher(ItemMatchInner);

#[derive(Clone)]
enum ItemMatchInner {
    Id(i32),
    Pred(Arc<dyn Fn(&Stack) -> bool + Send + Sync>),
}

impl Matcher {
    pub(crate) fn resolve(m: &ItemMatch, version: GameVersion) -> Result<Matcher, Error> {
        Ok(Matcher(match m {
            ItemMatch::Name(n) => ItemMatchInner::Id(
                item_id(version, n).ok_or_else(|| Error::UnknownItem(n.clone()))?,
            ),
            ItemMatch::Id(i) => ItemMatchInner::Id(*i),
            ItemMatch::Pred(p) => ItemMatchInner::Pred(p.clone()),
        }))
    }

    pub(crate) fn matches(&self, s: &Stack) -> bool {
        if s.count <= 0 && s.id <= 0 {
            return false;
        }
        match &self.0 {
            ItemMatchInner::Id(i) => s.id == *i,
            ItemMatchInner::Pred(p) => p(s),
        }
    }
}

/// An open container window ([`Bot::open_container`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Container {
    /// Window id.
    pub id: i32,
    /// Window kind.
    pub kind: WindowKind,
    /// Number of container slots (before the player's 36).
    pub slots: usize,
}

/// The `PlayerInventory` index (0–8 hotbar, 9–35 main) of the first matching stack in the
/// hotbar or main inventory, hotbar first.
fn find_player(ctx: &TickContext<'_>, m: &Matcher) -> Option<usize> {
    (0..36).find(|&k| ctx.inventory().player(k).is_some_and(|s| m.matches(s)))
}

impl Bot {
    /// The window-0 slots / counts are read in the tick: total count of matching items in
    /// the player inventory (hotbar, main, armour, off hand).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let n = bot.count("cobblestone").await?;
    /// # Ok(()) }
    /// ```
    pub async fn count(&self, item: impl Into<ItemMatch>) -> Result<i32, Error> {
        let item = item.into();
        self.with(move |ctx| {
            let m = Matcher::resolve(&item, ctx.version())?;
            Ok(ctx.inventory().count(|s| m.matches(s)))
        })
        .await?
    }

    /// The `PlayerInventory` index (0–8 hotbar, 9–35 main) of the first matching stack,
    /// hotbar first, and a copy of the stack.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// if let Some((index, stack)) = bot.find_item("bread").await? {
    ///     println!("{} bread in slot {index}", stack.count);
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn find_item(
        &self,
        item: impl Into<ItemMatch>,
    ) -> Result<Option<(usize, Stack)>, Error> {
        let item = item.into();
        self.with(move |ctx| {
            let m = Matcher::resolve(&item, ctx.version())?;
            Ok(
                find_player(ctx, &m)
                    .and_then(|k| ctx.inventory().player(k).map(|s| (k, s.clone()))),
            )
        })
        .await?
    }

    /// The held stack (a copy).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let held = bot.held_item().await?;
    /// # Ok(()) }
    /// ```
    pub async fn held_item(&self) -> Result<Option<Stack>, Error> {
        self.with(|ctx| ctx.inventory().held().cloned()).await
    }

    /// Selects hotbar slot `slot` (0 … 8) in this tick.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// bot.select_slot(2);
    /// # }
    /// ```
    pub fn select_slot(&self, slot: u8) {
        self.act(move |ctx| ctx.select_slot(slot));
    }

    /// Holds a matching item: selects it when it is in the hotbar; otherwise swaps it from
    /// the main inventory into the hotbar (an empty hotbar slot, else the held one) with a
    /// number-key click in the inventory screen (opened and closed again unless a container
    /// is open), then selects it. Returns the hotbar slot.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.equip("diamond_pickaxe").await?;
    /// bot.equip(|s: &ghast_client::inventory::Stack| s.id == 278).await?;
    /// # Ok(()) }
    /// ```
    pub async fn equip(&self, item: impl Into<ItemMatch>) -> Result<u8, Error> {
        let item = item.into();
        let label = item.to_string();
        let found = self
            .with(move |ctx| -> Result<Option<(usize, u8)>, Error> {
                let m = Matcher::resolve(&item, ctx.version())?;
                let Some(k) = find_player(ctx, &m) else {
                    return Ok(None);
                };
                if k < 9 {
                    ctx.select_slot(k as u8);
                    return Ok(Some((k, k as u8)));
                }
                let inv = ctx.inventory();
                let hotbar = (0..9u8)
                    .find(|h| inv.hotbar(usize::from(*h)).is_none())
                    .unwrap_or(ctx.held_slot());
                Ok(Some((k, hotbar)))
            })
            .await??;
        let Some((index, hotbar)) = found else {
            return Err(Error::NoSuchItem(label));
        };
        if index < 9 {
            return Ok(hotbar);
        }
        self.swap_to_hotbar(index, hotbar).await?;
        self.select_slot(hotbar);
        self.next_tick().await;
        Ok(hotbar)
    }

    /// Selects the hotbar tool that digs `pos` fastest, first bringing a faster one from the
    /// main inventory into the hotbar if there is one. Returns the hotbar slot.
    ///
    /// ```no_run
    /// use ghast_client::BlockPos;
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let p = BlockPos::new(1, 60, 1);
    /// bot.equip_for_block(p).await?;
    /// bot.dig(p).await?;
    /// # Ok(()) }
    /// ```
    pub async fn equip_for_block(&self, pos: BlockPos) -> Result<u8, Error> {
        let plan = self
            .with(move |ctx| -> Result<(u8, Option<usize>), Error> {
                if ctx.world().raw_block(pos.x, pos.y, pos.z).is_none() {
                    return Err(Error::NotLoaded(pos));
                }
                let (slot, hot) = crate::actions::best_hotbar_tool(ctx, pos);
                let main = (9..36)
                    .filter_map(|k| {
                        let s = ctx.dig_speed(pos, ctx.inventory().player(k))?;
                        (s > hot).then_some((k, s))
                    })
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(k, _)| k);
                if main.is_none() {
                    ctx.select_slot(slot);
                }
                Ok((slot, main))
            })
            .await??;
        match plan {
            (slot, None) => Ok(slot),
            (_, Some(k)) => {
                let hotbar = self.with(|ctx| ctx.held_slot()).await?;
                self.swap_to_hotbar(k, hotbar).await?;
                self.select_slot(hotbar);
                self.next_tick().await;
                Ok(hotbar)
            }
        }
    }

    /// Swaps `PlayerInventory` index `index` with hotbar slot `hotbar` (number-key click).
    /// Uses the open container if there is one, else opens the inventory screen for it and
    /// closes it again.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.swap_to_hotbar(20, 0).await?; // main slot 20 <-> hotbar 0
    /// # Ok(()) }
    /// ```
    pub async fn swap_to_hotbar(&self, index: usize, hotbar: u8) -> Result<(), Error> {
        let opened = self.ensure_screen().await?;
        let r = async {
            let slot = self
                .with(move |ctx| ctx.inventory().slot_of_player(index))
                .await?
                .ok_or(Error::Click(ClickError::InvalidSlot))?;
            self.click(Click::Swap { slot, hotbar }).await
        }
        .await;
        if opened {
            self.close_window().await?;
        }
        r
    }

    /// Drops the stack at `PlayerInventory` index `index` (all of it, or one with
    /// `all = false`), with a Q click in the inventory screen.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.drop_slot(9, true).await?;
    /// # Ok(()) }
    /// ```
    pub async fn drop_slot(&self, index: usize, all: bool) -> Result<(), Error> {
        let opened = self.ensure_screen().await?;
        let r = async {
            let slot = self
                .with(move |ctx| ctx.inventory().slot_of_player(index))
                .await?
                .ok_or(Error::Click(ClickError::InvalidSlot))?;
            self.click(Click::Drop { slot, all }).await
        }
        .await;
        if opened {
            self.close_window().await?;
        }
        r
    }

    /// Drops every matching stack from the hotbar and main inventory; returns how many items
    /// were dropped.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let n = bot.toss("rotten_flesh").await?;
    /// # Ok(()) }
    /// ```
    pub async fn toss(&self, item: impl Into<ItemMatch>) -> Result<i32, Error> {
        let item = item.into();
        let m = {
            let item = item.clone();
            self.with(move |ctx| Matcher::resolve(&item, ctx.version()))
                .await??
        };
        let opened = self.ensure_screen().await?;
        let r = async {
            let mut dropped = 0;
            loop {
                let m2 = m.clone();
                let next = self
                    .with(move |ctx| {
                        let k = find_player(ctx, &m2)?;
                        let n = ctx.inventory().player(k)?.count;
                        Some((ctx.inventory().slot_of_player(k)?, n))
                    })
                    .await?;
                let Some((slot, n)) = next else { break };
                self.click(Click::Drop { slot, all: true }).await?;
                dropped += n;
            }
            Ok(dropped)
        }
        .await;
        if opened {
            self.close_window().await?;
        }
        r
    }

    /// Opens the player inventory screen unless a GUI is open; `true` if it opened it.
    async fn ensure_screen(&self) -> Result<bool, Error> {
        let need = self.with(|ctx| !ctx.inventory().gui_open()).await?;
        if !need {
            return Ok(false);
        }
        for _ in 0..CLICK_WAIT {
            match self.with(|ctx| ctx.open_inventory()).await? {
                Ok(()) => {
                    // No click in the tick the 1.8 "open inventory" went out.
                    self.next_tick().await;
                    return Ok(true);
                }
                Err(ClickRefused::Inventory(ClickError::SameTick | ClickError::Pending)) => {}
                Err(ClickRefused::Inventory(e)) => return Err(Error::Click(e)),
                Err(_) => {}
            }
            self.next_tick().await;
        }
        Err(Error::Timeout)
    }

    /// Sends a window click in the open screen and waits for the server's confirm. Retries
    /// (one tick at a time) while the click must wait: the look is turning (clicks only
    /// while still), another click is in flight, or the same-tick packet rules forbid it.
    /// Fails with [`Error::Click`] for refusals that will not go away (no screen, invalid
    /// slot, unpredictable result), [`Error::ClickRejected`] or [`Error::ClickTimedOut`].
    ///
    /// ```no_run
    /// use ghast_client::inventory::Click;
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.click(Click::QuickMove { slot: 0, right: false }).await?; // shift-click slot 0
    /// # Ok(()) }
    /// ```
    pub async fn click(&self, click: Click) -> Result<(), Error> {
        let start = self.tick();
        let ticket = loop {
            match self.with(move |ctx| ctx.click(click)).await? {
                Ok(t) => break t,
                Err(ClickRefused::Inventory(
                    e @ (ClickError::NoScreen
                    | ClickError::InvalidSlot
                    | ClickError::InvalidButton
                    | ClickError::Unpredictable
                    | ClickError::WindowOpen),
                )) => return Err(Error::Click(e)),
                Err(_) => {}
            }
            if self.tick().saturating_sub(start) > CLICK_WAIT {
                return Err(Error::Timeout);
            }
            self.next_tick().await;
        };
        // The confirm arrives in a later tick's events (the inventory times out after 40).
        loop {
            self.next_tick().await;
            let r = self
                .with(move |ctx| {
                    ctx.events()
                        .inventory
                        .clicks
                        .iter()
                        .find(|r| ticket.matches(r))
                        .map(|r| r.outcome)
                })
                .await?;
            match r {
                Some(ClickOutcome::Accepted) => return Ok(()),
                Some(ClickOutcome::Rejected) => return Err(Error::ClickRejected),
                Some(_) => return Err(Error::ClickTimedOut),
                None => {}
            }
            if self.tick().saturating_sub(start) > CLICK_WAIT + 60 {
                return Err(Error::ClickTimedOut);
            }
        }
    }

    /// Closes the open GUI (container or inventory screen).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.close_window().await?;
    /// # Ok(()) }
    /// ```
    pub async fn close_window(&self) -> Result<(), Error> {
        for _ in 0..CLICK_WAIT {
            match self
                .with(|ctx| {
                    if !ctx.inventory().gui_open() {
                        return Ok(());
                    }
                    ctx.close_window()
                })
                .await?
            {
                Ok(()) => return Ok(()),
                Err(ClickRefused::Inventory(ClickError::SameTick | ClickError::Pending)) => {}
                Err(ClickRefused::Inventory(e)) => return Err(Error::Click(e)),
                Err(_) => {}
            }
            self.next_tick().await;
        }
        Err(Error::Timeout)
    }

    /// Right-clicks the container at `pos` (chest, furnace, ...) and waits for its window.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// if let Some(pos) = bot.find_block("chest", 5.0).await? {
    ///     let chest = bot.open_container(pos).await?;
    ///     bot.deposit("cobblestone", None).await?;
    ///     bot.close_window().await?;
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn open_container(&self, pos: BlockPos) -> Result<Container, Error> {
        let mark = self.mark();
        if self.with(|ctx| ctx.inventory().gui_open()).await? {
            self.close_window().await?;
        }
        self.activate_block(pos).await?;
        for _ in 0..40 {
            self.next_tick().await;
            self.check(mark)?;
            let w = self
                .with(|ctx| {
                    let inv = ctx.inventory();
                    inv.window().map(|w| Container {
                        id: w.id(),
                        kind: w.kind(),
                        slots: inv.container_len(),
                    })
                })
                .await?;
            if let Some(c) = w {
                // The contents come right after the open packet.
                self.next_tick().await;
                return Ok(c);
            }
        }
        Err(Error::Timeout)
    }

    /// The open container's slots (index, stack copy), container part only.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// for (slot, stack) in bot.container_items().await? {
    ///     println!("{slot}: {} x{}", stack.id, stack.count);
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn container_items(&self) -> Result<Vec<(usize, Stack)>, Error> {
        self.with(|ctx| {
            let inv = ctx.inventory();
            if inv.window().is_none() {
                return Err(Error::NoWindow);
            }
            Ok((0..inv.container_len())
                .filter_map(|i| inv.slot(i).map(|s| (i, s.clone())))
                .collect())
        })
        .await?
    }

    /// Shift-clicks matching stacks from the player part of the open container into it,
    /// whole stacks, until at least `max` items moved (`None`: all). Returns how many items
    /// left the player inventory (it stops when the container is full).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let moved = bot.deposit("cobblestone", Some(128)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn deposit(
        &self,
        item: impl Into<ItemMatch>,
        max: Option<i32>,
    ) -> Result<i32, Error> {
        self.transfer(item.into(), max, true).await
    }

    /// Shift-clicks matching stacks from the open container into the player inventory,
    /// whole stacks, until at least `max` items moved (`None`: all). Returns how many items
    /// arrived.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let got = bot.withdraw("bread", Some(16)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn withdraw(
        &self,
        item: impl Into<ItemMatch>,
        max: Option<i32>,
    ) -> Result<i32, Error> {
        self.transfer(item.into(), max, false).await
    }

    async fn transfer(
        &self,
        item: ItemMatch,
        max: Option<i32>,
        deposit: bool,
    ) -> Result<i32, Error> {
        let m = self
            .with(move |ctx| -> Result<(Matcher, i32), Error> {
                let m = Matcher::resolve(&item, ctx.version())?;
                if ctx.inventory().window().is_none() {
                    return Err(Error::NoWindow);
                }
                let n = ctx.inventory().count(|s| m.matches(s));
                Ok((m, n))
            })
            .await??;
        let (m, start) = m;
        let mut moved = 0;
        let mut tried: Vec<i16> = Vec::new();
        while max.is_none_or(|x| moved < x) {
            let m2 = m.clone();
            let skip = tried.clone();
            let slot = self
                .with(move |ctx| -> Result<Option<i16>, Error> {
                    let inv = ctx.inventory();
                    if inv.window().is_none() {
                        return Err(Error::NoWindow);
                    }
                    let n = inv.container_len();
                    let range = if deposit { n..inv.slot_count() } else { 0..n };
                    Ok(range
                        .map(|i| i as i16)
                        .filter(|i| !skip.contains(i))
                        .find(|&i| inv.slot(i as usize).is_some_and(|s| m2.matches(s))))
                })
                .await??;
            let Some(slot) = slot else { break };
            self.click(Click::QuickMove { slot, right: false }).await?;
            tried.push(slot);
            let m3 = m.clone();
            let now = self
                .with(move |ctx| ctx.inventory().count(|s| m3.matches(s)))
                .await?;
            let delta = if deposit { start - now } else { now - start };
            if delta == moved {
                break; // nothing moved: full
            }
            moved = delta;
        }
        Ok(moved)
    }
}
