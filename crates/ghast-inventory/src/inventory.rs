//! The inventory state machine: what the vanilla client holds (player inventory, cursor, the
//! open window), fed by the server's window packets, plus click intents with vanilla's
//! prediction and the window confirm ("transaction") protocol.

use bytestring::ByteString;

use crate::crafting::RecipeBook;
use crate::rules::Version;
use crate::sim::{Layout, Loc, Model, OUTSIDE, Screen, Sim};
use crate::stack::Stack;

/// Whether `l` is a crafting grid slot of the layout's screen.
fn is_grid(lay: Layout, l: Loc) -> bool {
    match (lay.screen, l) {
        (Screen::Player, Loc::Craft(_)) => true,
        (Screen::Container(WindowKind::Crafting), Loc::Container(k)) => (1..=9).contains(&k),
        _ => false,
    }
}
use crate::window::{SlotKind, WindowKind};

/// Ticks a click may wait for its confirm before it is given up ([`ClickOutcome::TimedOut`]).
pub const DEFAULT_CLICK_TIMEOUT_TICKS: u32 = 40;

/// A click intent: the vanilla GUI gestures a bot needs. Slot numbers are the open screen's
/// (window 0 when only the player inventory is open); [`OUTSIDE`] (-999) is a click outside the
/// window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Click {
    /// Mode 0: left click (`right = false`: pick up / put down / swap the whole stack) or right
    /// click (pick up half / put down one). On [`OUTSIDE`] it drops the cursor stack (left) or
    /// one item of it (right).
    Pickup {
        /// Slot, or [`OUTSIDE`].
        slot: i16,
        /// Right mouse button.
        right: bool,
    },
    /// Mode 1: shift-click (move the stack to the other part of the window).
    QuickMove {
        /// Slot.
        slot: i16,
        /// Right mouse button (same effect).
        right: bool,
    },
    /// Mode 2: number key over a slot: swap it with hotbar slot `hotbar` (0..=8), or on 1.16.5
    /// `40` (the swap-hands key) with the offhand.
    Swap {
        /// Slot.
        slot: i16,
        /// Hotbar index 0..=8, or 40 (offhand, 1.16.5 only).
        hotbar: u8,
    },
    /// Mode 4: drop key over a slot (`all = false`: one item, Q; `all = true`: the stack,
    /// ctrl+Q). With an empty cursor and [`OUTSIDE`] this is what the vanilla GUI sends for a
    /// click outside the window (no effect).
    Drop {
        /// Slot, or [`OUTSIDE`].
        slot: i16,
        /// Whole stack.
        all: bool,
    },
    /// Mode 6: double click: gather items like the cursor stack into it.
    PickupAll {
        /// Slot (normally the now empty slot the stack was picked up from).
        slot: i16,
    },
}

impl Click {
    /// The wire `(slot, button, mode)`.
    #[must_use]
    pub fn wire(self) -> (i16, u8, u8) {
        match self {
            Click::Pickup { slot, right } => (slot, u8::from(right), 0),
            Click::QuickMove { slot, right } => (slot, u8::from(right), 1),
            Click::Swap { slot, hotbar } => (slot, hotbar, 2),
            Click::Drop { slot, all } => (slot, u8::from(all), 4),
            Click::PickupAll { slot } => (slot, 0, 6),
        }
    }
}

/// Why a click (or open / close) was refused. Nothing was sent and nothing changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClickError {
    /// The previous click's confirm has not arrived yet (one click in flight).
    Pending,
    /// The server rejected a click and its full window resync has not arrived yet.
    Resyncing,
    /// Nothing is open: open the player inventory ([`Inventory::open_inventory`]) or wait for a
    /// container window.
    NoScreen,
    /// A container window is open (the player inventory cannot be opened over it).
    WindowOpen,
    /// Same-tick packet order vanilla never produces (and Grim's PacketOrder checks flag): a
    /// second click this tick, a click / close in the tick the inventory was opened, or the
    /// other way round. Retry next tick.
    SameTick,
    /// The slot does not exist in the open screen (or `OUTSIDE` where not allowed).
    InvalidSlot,
    /// Invalid hotbar index for [`Click::Swap`].
    InvalidButton,
    /// The result depends on data ghast does not model (recipes, fuel, brewing, trading,
    /// enchanting, an unknown crafting result); sending it would desynchronise the inventory.
    Unpredictable,
}

impl std::fmt::Display for ClickError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ClickError::Pending => "a click is waiting for its confirm",
            ClickError::Resyncing => "waiting for the window resync after a rejected click",
            ClickError::NoScreen => "no inventory screen is open",
            ClickError::WindowOpen => "a container window is open",
            ClickError::SameTick => "not allowed in the same tick",
            ClickError::InvalidSlot => "invalid slot",
            ClickError::InvalidButton => "invalid button",
            ClickError::Unpredictable => "click outcome cannot be predicted",
        })
    }
}

impl std::error::Error for ClickError {}

/// A window click as sent (`window_click`; 1.8 `ClickWindowC2SPacket`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClickPacket {
    /// Window id (0 = player inventory).
    pub window_id: u8,
    /// Slot, or -999.
    pub slot: i16,
    /// Mouse button / hotbar key.
    pub button: u8,
    /// Click mode.
    pub mode: u8,
    /// Action number: the window's counter, incremented before each click.
    pub action: i16,
    /// What vanilla's `onSlotClick` returned (the server compares it with its own result).
    pub item: Option<Stack>,
}

/// A packet the inventory wants sent. Write it right away, in call order (before the tick's
/// movement packet when it comes from a behaviour action).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    /// `window_click`.
    Click(ClickPacket),
    /// `transaction` answer to a rejected confirm (always `accepted = true`, like vanilla).
    Confirm {
        /// Window id as received.
        window_id: i8,
        /// Action number as received.
        action: i16,
    },
    /// `close_window` with this id (0 = player inventory).
    CloseWindow(u8),
    /// 1.8 only: `client_command` "open inventory achievement" (vanilla sends it when E opens
    /// the inventory).
    OpenInventory,
}

/// What became of a click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClickOutcome {
    /// The server agreed with the prediction.
    Accepted,
    /// The server disagreed; it resends the window (the inventory waits for it).
    Rejected,
    /// No confirm within the timeout (the server may not have had that window open).
    TimedOut,
}

/// A finished click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClickResult {
    /// Window id the click was sent for.
    pub window_id: i32,
    /// Its action number.
    pub action: i16,
    /// The outcome.
    pub outcome: ClickOutcome,
}

/// Changes since the last [`Inventory::drain_events`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Events {
    /// Something in the player's inventory (window 0, incl. crafting grid) changed.
    pub player_changed: bool,
    /// Something in the open window's own slots changed.
    pub window_changed: bool,
    /// The cursor stack changed.
    pub cursor_changed: bool,
    /// The server opened a window (its id).
    pub opened: Option<i32>,
    /// The open window was closed by the server.
    pub closed: bool,
    /// Clicks that were confirmed, rejected or timed out, in order.
    pub clicks: Vec<ClickResult>,
}

/// An open container window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    id: i32,
    kind: WindowKind,
    title: ByteString,
    action: i16,
    properties: Vec<(i16, i16)>,
}

impl Window {
    /// Window id (`syncId`).
    #[must_use]
    pub fn id(&self) -> i32 {
        self.id
    }
    /// Its type.
    #[must_use]
    pub fn kind(&self) -> WindowKind {
        self.kind
    }
    /// The title exactly as sent: a JSON chat component on both versions.
    #[must_use]
    pub fn title(&self) -> &ByteString {
        &self.title
    }
    /// A window property (`craft_progress_bar`: furnace progress, enchanting seeds, …).
    #[must_use]
    pub fn property(&self, id: i16) -> Option<i16> {
        self.properties.iter().find(|p| p.0 == id).map(|p| p.1)
    }
}

/// Which screen the client has open.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ScreenState {
    None,
    Inventory,
    Window(Window),
}

#[derive(Debug, Clone, Copy)]
struct PendingClick {
    window_id: i32,
    action: i16,
    age: u32,
}

#[derive(Debug, Clone, Copy, Default)]
struct TickFlags {
    clicked: bool,
    opened: bool,
    closed: bool,
}

/// The client's inventory: player inventory, cursor, the open window, click prediction and
/// the confirm protocol. Pure and deterministic: packets in, packets out, no I/O or clock
/// ([`tick`](Inventory::tick) counts time).
#[derive(Debug, Clone)]
pub struct Inventory {
    version: Version,
    m: Model,
    screen: ScreenState,
    player_action: i16,
    pending: Option<PendingClick>,
    resync: Option<(i32, u32)>,
    flags: TickFlags,
    timeout: u32,
    events: Events,
    book: RecipeBook,
}

impl Inventory {
    /// An empty inventory (a fresh player entity).
    #[must_use]
    pub fn new(version: Version) -> Inventory {
        Inventory {
            version,
            m: Model::default(),
            screen: ScreenState::None,
            player_action: 0,
            pending: None,
            resync: None,
            flags: TickFlags::default(),
            timeout: DEFAULT_CLICK_TIMEOUT_TICKS,
            events: Events::default(),
            book: RecipeBook::default(),
        }
    }

    /// Sets how many ticks a click may wait for its confirm.
    pub fn set_click_timeout(&mut self, ticks: u32) {
        self.timeout = ticks.max(1);
    }

    /// The protocol version.
    #[must_use]
    pub fn version(&self) -> Version {
        self.version
    }

    /// A new player entity (join, respawn, dimension change): everything is cleared,
    /// including the action counters; the server resends the inventory.
    /// The selected hotbar slot restarts at 0 too (a new `PlayerInventory`).
    pub fn reset(&mut self) {
        // the recipe book belongs to the connection, not to the player entity
        *self = Inventory {
            timeout: self.timeout,
            book: std::mem::take(&mut self.book),
            ..Inventory::new(self.version)
        };
        self.events.player_changed = true;
    }

    /// Call once at the start of every client tick: resets the per-tick packet-order guard
    /// and ages the pending click.
    pub fn tick(&mut self) {
        self.flags = TickFlags::default();
        if let Some(p) = &mut self.pending {
            p.age += 1;
            if p.age >= self.timeout {
                self.events.clicks.push(ClickResult {
                    window_id: p.window_id,
                    action: p.action,
                    outcome: ClickOutcome::TimedOut,
                });
                self.pending = None;
            }
        }
        if let Some((_, age)) = &mut self.resync {
            *age += 1;
            if *age >= self.timeout {
                self.resync = None;
            }
        }
    }

    /// Takes the accumulated change flags and click results.
    pub fn drain_events(&mut self) -> Events {
        std::mem::take(&mut self.events)
    }

    // ------------------------------------------------------------------ reading

    /// The cursor ("carried") stack.
    #[must_use]
    pub fn cursor(&self) -> Option<&Stack> {
        self.m.cursor.as_ref()
    }

    /// `PlayerInventory` slot by vanilla index: 0..=8 hotbar, 9..=35 main, 36..=39 armor
    /// (feet, legs, chest, head), 40 offhand (1.12 / 1.16.5).
    #[must_use]
    pub fn player(&self, index: usize) -> Option<&Stack> {
        self.m.player.get(index)?.as_ref()
    }

    /// Hotbar slot 0..=8.
    #[must_use]
    pub fn hotbar(&self, k: usize) -> Option<&Stack> {
        if k < 9 { self.player(k) } else { None }
    }

    /// The selected hotbar slot.
    #[must_use]
    pub fn selected(&self) -> u8 {
        self.m.selected
    }

    /// The stack in the selected hotbar slot.
    #[must_use]
    pub fn held(&self) -> Option<&Stack> {
        self.player(usize::from(self.m.selected))
    }

    /// Armor piece: 0 head, 1 chest, 2 legs, 3 feet.
    #[must_use]
    pub fn armor(&self, a: usize) -> Option<&Stack> {
        if a < 4 { self.player(39 - a) } else { None }
    }

    /// The offhand stack (1.12 / 1.16.5).
    #[must_use]
    pub fn offhand(&self) -> Option<&Stack> {
        self.player(40)
    }

    /// The open container window, if any.
    #[must_use]
    pub fn window(&self) -> Option<&Window> {
        match &self.screen {
            ScreenState::Window(w) => Some(w),
            _ => None,
        }
    }

    /// Whether a GUI is open (a container window or the player inventory).
    #[must_use]
    pub fn gui_open(&self) -> bool {
        self.screen != ScreenState::None
    }

    /// Whether the player inventory GUI is open (no container window).
    #[must_use]
    pub fn inventory_open(&self) -> bool {
        self.screen == ScreenState::Inventory
    }

    fn layout(&self) -> Layout {
        let screen = match &self.screen {
            ScreenState::Window(w) => Screen::Container(w.kind),
            _ => Screen::Player,
        };
        Layout::new(self.version, screen)
    }

    /// Number of slots of the current screen (window 0 when no container is open: 45 on 1.8,
    /// 46 on 1.12 / 1.16.5).
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.layout().len
    }

    /// Container slots of the open window (before the player's 36), 0 for window 0.
    #[must_use]
    pub fn container_len(&self) -> usize {
        self.layout().n
    }

    /// Slot `i` of the current screen (window 0 layout when no container is open).
    #[must_use]
    pub fn slot(&self, i: usize) -> Option<&Stack> {
        let l = self.layout().loc(i)?;
        self.m.at(l).as_ref()
    }

    /// Slot `i` of window 0 (`PlayerScreenHandler` numbering), whatever is open.
    #[must_use]
    pub fn window0_slot(&self, i: usize) -> Option<&Stack> {
        let l = Layout::new(self.version, Screen::Player).loc(i)?;
        self.m.at(l).as_ref()
    }

    /// Kind of slot `i` of the current screen.
    #[must_use]
    pub fn slot_kind(&self, i: usize) -> Option<SlotKind> {
        let lay = self.layout();
        (i < lay.len).then(|| lay.kind(i))
    }

    /// The current screen's slot that shows `PlayerInventory` index `k` (e.g. hotbar 0 is
    /// window 0 slot 36 and slot 54 of a 3-row chest).
    #[must_use]
    pub fn slot_of_player(&self, k: usize) -> Option<i16> {
        self.layout().slot_of_player(k).map(|i| i as i16)
    }

    /// First slot of the current screen whose stack matches.
    pub fn find(&self, mut pred: impl FnMut(&Stack) -> bool) -> Option<i16> {
        let lay = self.layout();
        (0..lay.len)
            .find(|&i| {
                lay.loc(i)
                    .and_then(|l| self.m.at(l).as_ref())
                    .is_some_and(&mut pred)
            })
            .map(|i| i as i16)
    }

    /// Total count of matching stacks in the player inventory (hotbar, main, armor, offhand).
    pub fn count(&self, mut pred: impl FnMut(&Stack) -> bool) -> i32 {
        self.m
            .player
            .iter()
            .flatten()
            .filter(|s| pred(s))
            .map(|s| s.count)
            .sum()
    }

    /// Whether the current screen's crafting result (window 0 slot 0, crafting table slot 0)
    /// is what the vanilla client shows. The 1.8 client computes results from its recipe book
    /// (ghast does not) and the server never sends them except in `window_items`; on 1.16.5 the
    /// server sends each new result as `set_slot`. The 1.12 client never computes results: the
    /// slot always holds what the server last sent (taking a result whose grid changed since is
    /// refused, its remainders depend on the recipes). While unknown, result clicks are
    /// [`ClickError::Unpredictable`].
    #[must_use]
    pub fn result_known(&self) -> bool {
        match &self.screen {
            ScreenState::Window(w) if w.kind == WindowKind::Crafting => {
                self.m.container_result_known
            }
            ScreenState::Window(_) => true,
            _ => self.m.craft_result_known,
        }
    }

    /// Whether the current screen's crafting result belongs to the grid as it is now. 1.12: the
    /// server sent it and the grid has not changed since (a click that changes the grid leaves
    /// the old result in the slot until the server's `set_slot`; taking such a stale result is
    /// [`ClickError::Unpredictable`]). 1.8 / 1.16.5: the same as
    /// [`result_known`](Inventory::result_known).
    #[must_use]
    pub fn result_current(&self) -> bool {
        if self.version != Version::V1_12 {
            return self.result_known();
        }
        match &self.screen {
            ScreenState::Window(w) if w.kind == WindowKind::Crafting => self.m.container_fresh,
            ScreenState::Window(_) => true,
            _ => self.m.craft_fresh,
        }
    }

    /// Whether a click waits for its confirm.
    #[must_use]
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether the inventory waits for the server's resync after a rejected click.
    #[must_use]
    pub fn resyncing(&self) -> bool {
        self.resync.is_some()
    }

    /// Whether [`click`](Inventory::click) would currently refuse for a reason other than the
    /// click itself.
    pub fn can_click(&self) -> Result<(), ClickError> {
        if self.pending.is_some() {
            return Err(ClickError::Pending);
        }
        if self.resync.is_some() {
            return Err(ClickError::Resyncing);
        }
        if self.flags.clicked || self.flags.opened {
            return Err(ClickError::SameTick);
        }
        if self.screen == ScreenState::None {
            return Err(ClickError::NoScreen);
        }
        Ok(())
    }

    // ------------------------------------------------------------------ actions

    /// Simulates a click exactly like the vanilla client (`ClientPlayerInteractionManager.clickSlot`:
    /// next action number, `onSlotClick`, packet with its return value) and returns the packet
    /// to send now. The prediction is applied immediately; the click is pending until its
    /// confirm arrives.
    ///
    /// Bots should click only while not turning (the server and anticheats see clicks against
    /// the last sent rotation; vanilla players do not move the camera with a GUI open).
    pub fn click(&mut self, click: Click) -> Result<Outgoing, ClickError> {
        self.can_click()?;
        let lay = self.layout();
        let (slot, button, mode) = click.wire();
        let in_range = usize::try_from(slot).is_ok_and(|i| i < lay.len);
        let slot_ok = match click {
            Click::Pickup { .. } | Click::Drop { .. } => in_range || slot == OUTSIDE,
            Click::QuickMove { .. } | Click::Swap { .. } | Click::PickupAll { .. } => in_range,
        };
        if !slot_ok {
            return Err(ClickError::InvalidSlot);
        }
        if let Click::Swap { hotbar, .. } = click {
            let ok = hotbar < 9 || (hotbar == 40 && self.version == Version::V1_16_5);
            if !ok {
                return Err(ClickError::InvalidButton);
            }
        }
        let mut m = self.m.clone();
        let item = {
            let mut sim = Sim {
                v: self.version,
                lay,
                m: &mut m,
                book: &self.book,
            };
            sim.click(slot, button, mode)
                .map_err(|_| ClickError::Unpredictable)?
        };
        self.commit(m, lay);
        let (window_id, action) = match &mut self.screen {
            ScreenState::Window(w) => {
                w.action = w.action.wrapping_add(1);
                (w.id, w.action)
            }
            _ => {
                self.player_action = self.player_action.wrapping_add(1);
                (0, self.player_action)
            }
        };
        self.pending = Some(PendingClick {
            window_id,
            action,
            age: 0,
        });
        self.flags.clicked = true;
        Ok(Outgoing::Click(ClickPacket {
            window_id: window_id as u8,
            slot,
            button,
            mode,
            action,
            item,
        }))
    }

    /// Replaces the model with the simulated one, records which parts changed and marks a
    /// crafting result unknown when its grid changed.
    fn commit(&mut self, new: Model, lay: Layout) {
        let old = std::mem::replace(&mut self.m, new);
        if old.player != self.m.player || old.craft != self.m.craft {
            self.events.player_changed = true;
        }
        if old.craft_result != self.m.craft_result {
            self.events.player_changed = true;
        }
        if old.container != self.m.container {
            self.events.window_changed = true;
        }
        if old.cursor != self.m.cursor {
            self.events.cursor_changed = true;
        }
        // 1.8: the simulation recomputed the result itself; 1.16.5: the server sends it; 1.12:
        // the client keeps the result it has, which no longer belongs to a changed grid.
        if self.version == Version::V1_8 {
            return;
        }
        if self.version == Version::V1_12 {
            if old.craft != self.m.craft {
                self.m.craft_fresh = false;
            }
            if lay.screen == Screen::Container(WindowKind::Crafting)
                && old.container.get(1..10) != self.m.container.get(1..10)
            {
                self.m.container_fresh = false;
            }
            return;
        }
        if old.craft != self.m.craft {
            self.m.craft_result_known = false;
        }
        if lay.screen == Screen::Container(WindowKind::Crafting)
            && old.container.get(1..10) != self.m.container.get(1..10)
        {
            self.m.container_result_known = false;
        }
    }

    /// Opens the player inventory GUI (key E). Returns the packet to send (1.8: the "open
    /// inventory achievement" client command; 1.12 / 1.16.5: nothing). No-op when already open.
    pub fn open_inventory(&mut self) -> Result<Option<Outgoing>, ClickError> {
        match self.screen {
            ScreenState::Inventory => return Ok(None),
            ScreenState::Window(_) => return Err(ClickError::WindowOpen),
            ScreenState::None => {}
        }
        if self.flags.clicked || self.flags.closed {
            return Err(ClickError::SameTick);
        }
        self.screen = ScreenState::Inventory;
        self.flags.opened = true;
        Ok((self.version == Version::V1_8).then_some(Outgoing::OpenInventory))
    }

    /// Closes the open GUI (Escape / E): `close_window` with its id. The client drops the
    /// cursor stack (the server throws it) and, for the 1.8 player inventory, empties the
    /// crafting grid (the server drops it).
    pub fn close(&mut self) -> Result<Outgoing, ClickError> {
        if self.screen == ScreenState::None {
            return Err(ClickError::NoScreen);
        }
        if self.flags.opened {
            return Err(ClickError::SameTick);
        }
        let id = match &self.screen {
            ScreenState::Window(w) => w.id as u8,
            _ => 0,
        };
        self.close_screen();
        self.flags.closed = true;
        Ok(Outgoing::CloseWindow(id))
    }

    /// The client side of closing the current screen handler (`closeScreen` + `close`).
    fn close_screen(&mut self) {
        if self.m.cursor.take().is_some() {
            self.events.cursor_changed = true;
        }
        match std::mem::replace(&mut self.screen, ScreenState::None) {
            ScreenState::Inventory => {
                if self.version == Version::V1_8 {
                    // PlayerScreenHandler.close: grid dropped, result cleared (also on the client)
                    if self.m.craft.iter().any(Option::is_some) {
                        self.events.player_changed = true;
                    }
                    self.m.craft = Default::default();
                }
                self.m.craft_result = None;
                self.m.craft_result_known = self.version != Version::V1_16_5;
                self.m.craft_fresh = false;
            }
            ScreenState::Window(_) => {
                self.m.container.clear();
                self.m.container_result_known = true;
                self.m.container_fresh = true;
            }
            ScreenState::None => {}
        }
        if let Some((w, _)) = self.resync
            && w != 0
        {
            self.resync = None;
        }
    }

    /// Client-side use of the held stack (no packet): `n` items off the selected hotbar stack,
    /// the slot emptied when nothing is left. Vanilla does this when a right click places a block
    /// or plants seeds (1.8 `BlockItem` / `SeedItem.use`, 1.16.5 `BlockItem.place`) or
    /// when eating finishes, before the server's `set_slot`; call it then (not in creative,
    /// where vanilla keeps the count). Returns whether there was a stack.
    pub fn consume_held(&mut self, n: u8) -> bool {
        self.consume(usize::from(self.m.selected.min(8)), n)
    }

    /// [`consume_held`](Inventory::consume_held) for the offhand stack (1.12 / 1.16.5; no-op on
    /// 1.8).
    pub fn consume_offhand(&mut self, n: u8) -> bool {
        self.version != Version::V1_8 && self.consume(40, n)
    }

    fn consume(&mut self, k: usize, n: u8) -> bool {
        let slot = &mut self.m.player[k];
        let Some(s) = slot.as_mut() else {
            return false;
        };
        s.count -= i32::from(n);
        if s.count <= 0 {
            *slot = None;
        }
        self.events.player_changed = true;
        true
    }

    /// Selects a hotbar slot (the behaviour's `select_slot`; vanilla's insertStack prefers it).
    pub fn select(&mut self, slot: u8) {
        if slot < 9 {
            self.m.selected = slot;
        }
    }

    // ------------------------------------------------------------------ inbound

    /// `held_item_slot` from the server.
    pub fn on_held_slot(&mut self, slot: i8) {
        if (0..9).contains(&slot) {
            self.m.selected = slot as u8;
        }
    }

    /// `open_window`: the server opened a window of this kind (`None`: a type the vanilla
    /// client has no screen for; ignored). Any open screen is closed client-side first.
    pub fn on_open_window(&mut self, id: i32, kind: Option<WindowKind>, title: ByteString) {
        let Some(kind) = kind else { return };
        if self.screen != ScreenState::None {
            self.close_screen();
        }
        let n = kind.container_len(self.version);
        self.m.container = vec![None; n];
        self.m.container_result_known = true;
        self.m.container_fresh = true;
        self.screen = ScreenState::Window(Window {
            id,
            kind,
            title,
            action: 0,
            properties: Vec::new(),
        });
        self.events.opened = Some(id);
        self.events.window_changed = true;
    }

    /// `close_window` from the server (closes whatever is open, whatever the id).
    pub fn on_close_window(&mut self) {
        if self.screen != ScreenState::None {
            let was_window = matches!(self.screen, ScreenState::Window(_));
            self.close_screen();
            if was_window {
                self.events.closed = true;
            }
        }
    }

    fn open_id(&self) -> i32 {
        match &self.screen {
            ScreenState::Window(w) => w.id,
            _ => 0,
        }
    }

    /// `window_items`: replaces the window's slots (`window_id` as received, unsigned byte).
    pub fn on_window_items(
        &mut self,
        window_id: i32,
        items: impl IntoIterator<Item = Option<Stack>>,
    ) {
        let lay = if window_id == 0 {
            Layout::new(self.version, Screen::Player)
        } else if window_id == self.open_id() {
            self.layout()
        } else {
            return;
        };
        let mut player = false;
        let mut window = false;
        let mut touched_grid = false;
        for (i, s) in items.into_iter().enumerate() {
            let Some(l) = lay.loc(i) else { break };
            touched_grid |= is_grid(lay, l);
            let mut s = self.normalize(s);
            if lay.clamps(l) {
                crate::sim::clamp(&mut s);
            }
            let slot = self.m.at_mut(l);
            if *slot != s {
                *slot = s;
                match l {
                    Loc::Container(_) => window = true,
                    _ => player = true,
                }
            }
        }
        if lay.screen == Screen::Player {
            self.m.craft_result_known = true;
            self.m.craft_fresh = true;
        } else {
            self.m.container_result_known = true;
            self.m.container_fresh = true;
        }
        // 1.8: setting the grid slots made the client compute the result itself
        if self.version == Version::V1_8 && touched_grid {
            crate::sim::recompute_1_8(&mut self.m, lay);
        }
        self.events.player_changed |= player;
        self.events.window_changed |= window;
        if self.resync.is_some_and(|(w, _)| w == window_id) {
            self.resync = None;
        }
    }

    /// `set_slot` (`window_id` as received, signed byte): -1 sets the cursor, 1.12 / 1.16.5 -2 a
    /// `PlayerInventory` index, window 0 slots 36..45 always reach window 0, others only the
    /// current screen.
    pub fn on_set_slot(&mut self, window_id: i32, slot: i16, item: Option<Stack>) {
        let item = self.normalize(item);
        if window_id == -1 {
            if self.m.cursor != item {
                self.m.cursor = item;
                self.events.cursor_changed = true;
            }
            return;
        }
        let loc = if window_id == -2 && self.version != Version::V1_8 {
            usize::try_from(slot)
                .ok()
                .filter(|&k| k < 41)
                .map(Loc::Player)
        } else if window_id == 0 && (36..45).contains(&slot) {
            Layout::new(self.version, Screen::Player).loc(slot as usize)
        } else if window_id == self.open_id() {
            usize::try_from(slot)
                .ok()
                .and_then(|i| self.layout().loc(i))
        } else {
            None
        };
        let Some(l) = loc else { return };
        let mut item = item;
        if self.layout().clamps(l) {
            crate::sim::clamp(&mut item);
        }
        let crafting_table = matches!(
            self.layout().screen,
            Screen::Container(WindowKind::Crafting)
        );
        match l {
            Loc::CraftResult => {
                self.m.craft_result_known = true;
                self.m.craft_fresh = true;
            }
            Loc::Container(0) if crafting_table => {
                self.m.container_result_known = true;
                self.m.container_fresh = true;
            }
            // 1.12: a grid slot changed under the result the client holds
            Loc::Craft(_) if self.version == Version::V1_12 => self.m.craft_fresh = false,
            Loc::Container(1..=9) if crafting_table && self.version == Version::V1_12 => {
                self.m.container_fresh = false;
            }
            _ => {}
        }
        let s = self.m.at_mut(l);
        if *s != item {
            *s = item;
            match l {
                Loc::Container(_) => self.events.window_changed = true,
                _ => self.events.player_changed = true,
            }
        }
        if self.version == Version::V1_8 {
            // the slot's screen: window 0 for its hotbar slots, else the current one
            let lay = if window_id == 0 && (36..45).contains(&slot) {
                Layout::new(self.version, Screen::Player)
            } else {
                self.layout()
            };
            if is_grid(lay, l) {
                crate::sim::recompute_1_8(&mut self.m, lay);
            }
        }
    }

    /// The crafting recipes the server declared (1.16.5; 1.8 clients know theirs).
    #[must_use]
    pub fn recipes(&self) -> &RecipeBook {
        &self.book
    }

    /// 1.16.5 `declare_recipes`: replaces the client's crafting recipes (used for the remainders
    /// of a taken crafting result).
    pub fn on_declare_recipes(&mut self, book: RecipeBook) {
        self.book = book;
    }

    /// `craft_progress_bar` (window property).
    pub fn on_window_property(&mut self, window_id: i32, property: i16, value: i16) {
        if let ScreenState::Window(w) = &mut self.screen
            && w.id == window_id
        {
            match w.properties.iter_mut().find(|p| p.0 == property) {
                Some(p) => p.1 = value,
                None => w.properties.push((property, value)),
            }
        }
    }

    /// `transaction` (window confirm) from the server. Returns the answer to send now, exactly
    /// when vanilla answers (not accepted, and the window is 0 or the open one); resolves the
    /// pending click when it is its confirm. Route every confirm through here and answer
    /// nowhere else, so each is answered exactly once.
    pub fn on_transaction(
        &mut self,
        window_id: i8,
        action: i16,
        accepted: bool,
    ) -> Option<Outgoing> {
        let w = i32::from(window_id);
        if let Some(p) = self.pending
            && p.window_id == w
            && p.action == action
        {
            self.pending = None;
            let outcome = if accepted {
                ClickOutcome::Accepted
            } else {
                self.resync = Some((w, 0));
                ClickOutcome::Rejected
            };
            self.events.clicks.push(ClickResult {
                window_id: w,
                action,
                outcome,
            });
        }
        let known = w == 0 || w == self.open_id();
        (known && !accepted).then_some(Outgoing::Confirm { window_id, action })
    }

    /// 1.12 / 1.16.5 drop empty stacks (1.16.5 also has no damage field); 1.8 keeps zero-count
    /// stacks.
    fn normalize(&self, s: Option<Stack>) -> Option<Stack> {
        match self.version {
            Version::V1_8 => s,
            Version::V1_12 => s.filter(|s| s.count > 0 && s.id != 0),
            Version::V1_16_5 => s
                .filter(|s| s.count > 0 && s.id != 0)
                .map(|s| Stack { damage: 0, ..s }),
        }
    }
}
