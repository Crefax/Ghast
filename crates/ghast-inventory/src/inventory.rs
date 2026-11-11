//! The inventory state machine: what the vanilla client holds (player inventory, cursor, the
//! open window), fed by the server's window packets, plus click intents with vanilla's
//! prediction and the window confirm ("transaction") protocol.

use bytestring::ByteString;

use crate::crafting::RecipeBook;
use crate::rules::{Version, VersionNotEnabled};
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

/// Version tags for loops monomorphised per version ([`Inventory::on_window_items`]).
const V18: u8 = 0;
const V112: u8 = 1;
const V1165: u8 = 2;
const V261: u8 = 3;

/// One incoming `window_items` slot: [`Incoming::settle`] stores it (normalised and clamped
/// like vanilla's handler) and says whether the slot changed.
trait Incoming {
    fn settle<const V: u8>(self, slot: &mut Option<Stack>, clamp: bool) -> bool;
}

impl Incoming for Option<Stack> {
    #[inline(always)]
    fn settle<const V: u8>(self, slot: &mut Option<Stack>, clamp: bool) -> bool {
        let s = normalize_as::<V>(self);
        let s = if clamp { clamp_as::<V>(s) } else { s };
        if same_slot(slot, &s) {
            false
        } else {
            *slot = s;
            true
        }
    }
}

#[cfg(any(feature = "v1_8", feature = "v1_12"))]
impl Incoming for &ghast_protocol::types::slot::legacy::Slot {
    #[inline(always)]
    fn settle<const V: u8>(self, slot: &mut Option<Stack>, clamp: bool) -> bool {
        if legacy_same::<V>(slot, self, clamp) {
            return false;
        }
        crate::stack::from_legacy_slot(self.clone()).settle::<V>(slot, clamp);
        true
    }
}

/// Whether `stored` equals the 1.8 / 1.12 wire slot `w` after normalising and clamping, i.e.
/// what [`Incoming::settle`] of the converted slot would compare.
#[cfg(any(feature = "v1_8", feature = "v1_12"))]
#[inline(always)]
fn legacy_same<const V: u8>(
    stored: &Option<Stack>,
    w: &ghast_protocol::types::slot::legacy::Slot,
    clamp: bool,
) -> bool {
    let Some(ws) = &w.0 else {
        return stored.is_none();
    };
    let id = i32::from(ws.id);
    let mut count = i32::from(ws.count);
    if V != V18 && (count <= 0 || id == 0) {
        return stored.is_none();
    }
    if V == V261 || V == V1165 {
        // legacy slots never reach these versions (see `on_window_items_legacy`)
        return false;
    }
    if clamp && count > 64 {
        count = 64;
    }
    let Some(st) = stored else {
        return false;
    };
    st.id == id
        && st.count == count
        && st.damage == ws.damage
        && match (&st.data, &ws.nbt) {
            (crate::stack::ItemData::None, None) => true,
            (crate::stack::ItemData::Nbt(a), Some(b)) => a == b,
            _ => false,
        }
}

/// [`Inventory::normalize`] with the version as a constant.
#[inline(always)]
fn normalize_as<const V: u8>(s: Option<Stack>) -> Option<Stack> {
    if V == V18 {
        return s;
    }
    let mut s = s.filter(|s| s.count > 0 && s.id != 0);
    if V != V112
        && let Some(st) = &mut s
    {
        st.damage = 0;
    }
    s
}

/// [`crate::sim::clamp_v`] with the version as a constant.
#[inline(always)]
fn clamp_as<const V: u8>(mut s: Option<Stack>) -> Option<Stack> {
    if V == V261 {
        crate::sim::clamp_v(Version::V26_1, &mut s);
    } else {
        crate::sim::clamp(&mut s);
    }
    s
}

/// `a == b` for slots, written out so the common equal-NBT case stays a few compares and one
/// `memcmp` whatever variants `ItemData` has.
#[inline(always)]
fn same_slot(a: &Option<Stack>, b: &Option<Stack>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.id == b.id && a.count == b.count && a.damage == b.damage && a.data == b.data
        }
        _ => false,
    }
}

/// 26.1: ticks without a correcting container packet after which a click counts as accepted
/// (see [`Inventory::set_click_settle_ticks`]).
pub const DEFAULT_CLICK_SETTLE_TICKS: u32 = 4;

/// Ticks a click may wait for its confirm before it is given up ([`ClickOutcome::TimedOut`]).
pub const DEFAULT_CLICK_TIMEOUT_TICKS: u32 = 40;

/// A click intent: the vanilla GUI gestures a bot needs. Slot numbers are the open screen's
/// (window 0 when only the player inventory is open); [`OUTSIDE`] (-999) is a click outside the
/// window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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
    /// Mode 3: middle click in creative mode: a full stack of the slot's item onto the empty
    /// cursor (26.1; no effect outside creative mode).
    Clone {
        /// Slot.
        slot: i16,
    },
    /// Mode 5: one packet of a drag ("quick craft"): the GUI sends [`DragStage::Start`] (slot
    /// [`OUTSIDE`]), one [`DragStage::Slot`] per slot the mouse crosses, then
    /// [`DragStage::End`] (slot [`OUTSIDE`]), each its own click (26.1).
    QuickCraft {
        /// Slot ([`OUTSIDE`] for start and end).
        slot: i16,
        /// Which packet of the drag.
        stage: DragStage,
        /// How the carried stack is spread.
        kind: DragKind,
    },
}

/// The packet of a drag ([`Click::QuickCraft`]): vanilla's quick-craft header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DragStage {
    /// Mouse pressed over the screen (header 0).
    Start,
    /// The mouse crossed a slot (header 1).
    Slot,
    /// Mouse released: the stack is spread (header 2).
    End,
}

/// How a drag spreads the carried stack ([`Click::QuickCraft`]): vanilla's quick-craft type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DragKind {
    /// Left button: split evenly (type 0).
    Split,
    /// Right button: one item per slot (type 1).
    One,
    /// Middle button, creative mode: a full stack per slot (type 2).
    Clone,
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
            Click::Clone { slot } => (slot, 2, 3),
            Click::QuickCraft { slot, stage, kind } => {
                let header = match stage {
                    DragStage::Start => 0,
                    DragStage::Slot => 1,
                    DragStage::End => 2,
                };
                let ty = match kind {
                    DragKind::Split => 0,
                    DragKind::One => 1,
                    DragKind::Clone => 2,
                };
                (slot, header | ty << 2, 5)
            }
        }
    }
}

/// Why a click (or open / close) was refused. Nothing was sent and nothing changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
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
    /// 26.1 `container_click` (no confirm exists: the server applies every click and resends
    /// the slots whose hashes differ from its own result).
    ContainerClick(ContainerClick),
    /// 26.1 creative inventory screen (window 0 in creative mode): the packets the real
    /// client sends, in order (often none). See [`CreativePacket`].
    Creative(Vec<CreativePacket>),
}

/// One packet the 26.1 creative inventory screen sends (`Outgoing::Creative`): the server
/// trusts these stacks, so they are exactly what the real client sends.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CreativePacket {
    /// `swing` (main hand): a stack was dropped.
    Swing,
    /// `set_creative_mode_slot`: window 0 slot `slot` now holds `item` (`slot` -1: `item` is
    /// dropped).
    SetSlot {
        /// Window 0 slot, or -1 for a drop.
        slot: i16,
        /// The stack.
        item: Option<Stack>,
    },
}

/// A 26.1 click as sent: what vanilla `MultiPlayerGameMode.handleContainerInput` writes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContainerClick {
    /// Container id (0 = player inventory).
    pub window_id: i32,
    /// The screen's state id (from the last `container_set_content` / `container_set_slot`).
    pub state_id: i32,
    /// Slot, or -999.
    pub slot: i16,
    /// Mouse button / hotbar key.
    pub button: u8,
    /// Click mode (`ContainerInput`).
    pub mode: u8,
    /// Slots whose stack the prediction changed, with the new stack (vanilla compares every slot
    /// before and after the simulated click).
    pub changed: Vec<(i16, Option<Stack>)>,
    /// The cursor after the click.
    pub carried: Option<Stack>,
    /// `false` when ghast could not predict this click (see [`Inventory::click`]): nothing
    /// changed locally, `changed` is empty, and the server's corrections bring the result.
    pub predicted: bool,
    /// ghast's own number for the click (per screen, like the legacy action number, but never
    /// sent): the [`ClickResult::action`] that resolves it.
    pub action: i16,
    /// How many `swing` (main hand) packets the vanilla client sends right before this click:
    /// every stack the click drops (outside the window, the drop key, a remainder that does not
    /// fit) swings the arm. Send them first, then the click (0 when unpredicted).
    pub swings: u8,
}

/// What became of a click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
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
#[non_exhaustive]
pub struct ClickResult {
    /// Window id the click was sent for.
    pub window_id: i32,
    /// Its action number.
    pub action: i16,
    /// The outcome.
    pub outcome: ClickOutcome,
}

impl ClickResult {
    /// A result as [`Inventory::drain_events`] reports it (for tests and replays).
    #[must_use]
    pub fn new(window_id: i32, action: i16, outcome: ClickOutcome) -> ClickResult {
        ClickResult {
            window_id,
            action,
            outcome,
        }
    }
}

/// Changes since the last [`Inventory::drain_events`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
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
    /// 26.1 `minecraft:menu` registry id (-1 for other versions).
    menu: i32,
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
    /// 26.1: the `minecraft:menu` registry id of `open_screen` (`None` for other versions).
    #[must_use]
    pub fn menu_26_1(&self) -> Option<i32> {
        (self.menu >= 0).then_some(self.menu)
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
    settle: u32,
    events: Events,
    book: RecipeBook,
    /// 26.1 state ids: window 0 (`inventoryMenu`) and the open container.
    state_player: i32,
    state_window: i32,
    /// 26.1 registry names for click hashes and the binding curse.
    #[cfg(feature = "v26_1")]
    reg26: crate::v26_1::HashRegistries,
    /// 26.1: recipe item sets, game mode, experience level.
    #[cfg(feature = "v26_1")]
    w26: crate::sim26::World26,
    /// 26.1: window 0's and the open container's per-screen click state (drags, …).
    #[cfg(feature = "v26_1")]
    s26: [crate::sim26::Screen26; 2],
    /// 26.1: window 0's `lastSlots` (what its last `broadcastChanges` saw; the creative
    /// screen's listener sends every slot that differs).
    #[cfg(feature = "v26_1")]
    creative_last: Vec<Option<Stack>>,
    /// 26.1: the creative inventory screen is open (window 0 in creative mode).
    #[cfg(feature = "v26_1")]
    creative_screen: bool,
    /// 26.1: packets the creative screen sends in answer to server packets
    /// ([`Inventory::take_creative_echo_26_1`]).
    #[cfg(feature = "v26_1")]
    creative_echo: Vec<CreativePacket>,
    /// 26.1 `LocalPlayer.dropSpamThrottler`: +20 per creative drop, -1 per tick; drops are
    /// refused from 1280 (`canDropItems`).
    #[cfg(feature = "v26_1")]
    drop_throttle: i32,
}

impl Inventory {
    /// An empty inventory (a fresh player entity).
    ///
    /// # Errors
    /// [`VersionNotEnabled`] when this build of the crate lacks the version's feature
    /// ([`Version::is_enabled`]).
    pub fn new(version: Version) -> Result<Inventory, VersionNotEnabled> {
        if version.is_enabled() {
            Ok(Inventory::empty(version))
        } else {
            Err(VersionNotEnabled(version))
        }
    }

    fn empty(version: Version) -> Inventory {
        Inventory {
            version,
            m: Model::default(),
            screen: ScreenState::None,
            player_action: 0,
            pending: None,
            resync: None,
            flags: TickFlags::default(),
            timeout: DEFAULT_CLICK_TIMEOUT_TICKS,
            settle: DEFAULT_CLICK_SETTLE_TICKS,
            events: Events::default(),
            book: RecipeBook::default(),
            state_player: 0,
            state_window: 0,
            #[cfg(feature = "v26_1")]
            reg26: crate::v26_1::HashRegistries::default(),
            #[cfg(feature = "v26_1")]
            w26: crate::sim26::World26::default(),
            #[cfg(feature = "v26_1")]
            s26: Default::default(),
            #[cfg(feature = "v26_1")]
            creative_last: vec![None; 46],
            #[cfg(feature = "v26_1")]
            creative_screen: false,
            #[cfg(feature = "v26_1")]
            creative_echo: Vec::new(),
            #[cfg(feature = "v26_1")]
            drop_throttle: 0,
        }
    }

    /// Sets how many ticks a click may wait for its confirm.
    pub fn set_click_timeout(&mut self, ticks: u32) {
        self.timeout = ticks.max(1);
    }

    /// 26.1: how many ticks after a click without a correcting container packet make it
    /// [`ClickOutcome::Accepted`]. 26.1 has no confirm: the server applies the click when it
    /// receives it and answers in the same moment, with `container_set_slot` /
    /// `container_set_content` / `set_cursor_item` only for slots its result disagrees with, and
    /// with nothing when the client predicted right. So silence is the acceptance; set this to
    /// cover the round trip (`ceil(rtt / 50 ms) + 1`; default
    /// [`DEFAULT_CLICK_SETTLE_TICKS`]). A correction before that makes the click
    /// [`ClickOutcome::Rejected`] (already applied: no resync wait).
    pub fn set_click_settle_ticks(&mut self, ticks: u32) {
        self.settle = ticks.max(1);
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
        #[cfg(feature = "v26_1")]
        let reg26 = std::mem::take(&mut self.reg26);
        #[cfg(feature = "v26_1")]
        let w26 = std::mem::take(&mut self.w26);
        *self = Inventory {
            timeout: self.timeout,
            settle: self.settle,
            book: std::mem::take(&mut self.book),
            #[cfg(feature = "v26_1")]
            reg26,
            #[cfg(feature = "v26_1")]
            w26,
            ..Inventory::empty(self.version)
        };
        self.events.player_changed = true;
    }

    /// Call once at the start of every client tick: resets the per-tick packet-order guard
    /// and ages the pending click.
    pub fn tick(&mut self) {
        self.flags = TickFlags::default();
        #[cfg(feature = "v26_1")]
        if self.drop_throttle > 0 {
            self.drop_throttle -= 1;
        }
        if let Some(p) = &mut self.pending {
            p.age += 1;
            if self.version == Version::V26_1 && p.age >= self.settle {
                self.events.clicks.push(ClickResult {
                    window_id: p.window_id,
                    action: p.action,
                    outcome: ClickOutcome::Accepted,
                });
                self.pending = None;
            } else if p.age >= self.timeout {
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
            Click::QuickMove { .. }
            | Click::Swap { .. }
            | Click::PickupAll { .. }
            | Click::Clone { .. } => in_range,
            Click::QuickCraft { stage, .. } => match stage {
                DragStage::Slot => in_range,
                DragStage::Start | DragStage::End => in_range || slot == OUTSIDE,
            },
        };
        if !slot_ok {
            return Err(ClickError::InvalidSlot);
        }
        if matches!(click, Click::Clone { .. } | Click::QuickCraft { .. })
            && self.version != Version::V26_1
        {
            // the older versions' drags and creative clicks are not modelled
            return Err(ClickError::Unpredictable);
        }
        if let Click::Swap { hotbar, .. } = click {
            let ok = hotbar < 9
                || (hotbar == 40 && matches!(self.version, Version::V1_16_5 | Version::V26_1));
            if !ok {
                return Err(ClickError::InvalidButton);
            }
        }
        #[cfg(feature = "v26_1")]
        if self.version == Version::V26_1 {
            if self.creative_screen {
                return self.creative_click(slot, button, mode);
            }
            return Ok(self.click_26(slot, button, mode));
        }
        let mut m = self.m.clone();
        let simulated = {
            let mut sim = Sim {
                v: self.version,
                lay,
                m: &mut m,
                book: &self.book,
                binding_curse: self.binding_curse(),
            };
            sim.click(slot, button, mode)
        };
        let item = simulated.map_err(|_| ClickError::Unpredictable)?;
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

    fn binding_curse(&self) -> Option<i32> {
        #[cfg(feature = "v26_1")]
        {
            self.reg26
                .enchantments
                .iter()
                .position(|n| n == "minecraft:binding_curse")
                .map(|i| i as i32)
        }
        #[cfg(not(feature = "v26_1"))]
        None
    }

    /// 26.1: the registry names click hashes and the binding curse check need (the
    /// configuration phase's `minecraft:enchantment` keys in order). Without them, clicks with
    /// enchanted items send hashes the server corrects, and taking enchanted armor off is not
    /// predicted.
    #[cfg(feature = "v26_1")]
    pub fn set_registries_26_1(&mut self, reg: crate::v26_1::HashRegistries) {
        self.reg26 = reg;
    }

    /// 26.1: the registry names set with [`Inventory::set_registries_26_1`].
    #[cfg(feature = "v26_1")]
    #[must_use]
    pub fn registries_26_1(&self) -> &crate::v26_1::HashRegistries {
        &self.reg26
    }

    /// 26.1: the state id the next click of the current screen carries.
    #[must_use]
    pub fn state_id(&self) -> i32 {
        match &self.screen {
            ScreenState::Window(_) => self.state_window,
            _ => self.state_player,
        }
    }

    /// The 26.1 menu the click runs in.
    #[cfg(feature = "v26_1")]
    fn kind_26(&self) -> crate::sim26::Kind {
        match &self.screen {
            ScreenState::Window(w) => match (crate::sim26::Kind::from_menu(w.menu), w.kind) {
                (Some(k), _) => k,
                (None, WindowKind::Chest { rows }) => crate::sim26::Kind::Chest(rows),
                (None, _) => crate::sim26::Kind::Chest(0),
            },
            _ => crate::sim26::Kind::Player,
        }
    }

    /// The 26.1 click (`MultiPlayerGameMode.handleContainerInput`): vanilla predicts, then lists
    /// every slot whose stack changed (in the order of its `Int2ObjectOpenHashMap`) and the
    /// carried stack. A click ghast cannot predict is sent unpredicted (nothing changes locally;
    /// the server applies it and sends the true slots), never refused.
    #[cfg(feature = "v26_1")]
    fn click_26(&mut self, slot: i16, button: u8, mode: u8) -> Outgoing {
        let kind = self.kind_26();
        let w = usize::from(matches!(self.screen, ScreenState::Window(_)));
        let mut m = self.m.clone();
        let mut scr = self.s26[w].clone();
        self.w26.binding_curse = self.binding_curse();
        self.w26.can_drop = self.drop_throttle < 1280;
        let (result, swings) = {
            let mut sim = crate::sim26::Sim26 {
                kind,
                m: &mut m,
                s: &mut scr,
                w: &self.w26,
                swings: 0,
            };
            let r = sim.click(slot, button as i8, mode);
            (r, sim.swings)
        };
        let predicted = result.is_ok();
        let mut changed = Vec::new();
        let mut swings_out = 0;
        if predicted {
            let mut idx = Vec::new();
            for i in 0..kind.len() {
                let Some(l) = kind.loc(i) else { break };
                let (a, b) = (self.m.at(l), m.at(l));
                let same = match (a, b) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.count == b.count && a.id == b.id && a.data_eq(b),
                    _ => false,
                };
                if !same {
                    idx.push(i as i32);
                }
            }
            for i in crate::sim26::fastutil_order(&idx) {
                let l = kind.loc(i as usize).unwrap_or(Loc::Player(0));
                changed.push((i as i16, m.at(l).clone()));
            }
            let lay = self.layout();
            self.commit(m, lay);
            self.s26[w] = scr;
            swings_out = swings;
        } else {
            // the outcome is unknown, including how far a drag got: start over
            self.s26[w].drag.reset();
        }
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
        Outgoing::ContainerClick(ContainerClick {
            action,
            window_id,
            state_id: self.state_id(),
            slot,
            button,
            mode,
            changed,
            carried: self.m.cursor.clone(),
            predicted,
            swings: swings_out,
        })
    }

    /// 26.1: whether the player is in creative mode (instant build): middle-click clones,
    /// clone drags, armor with the binding curse comes off, overflow vanishes. Set from the
    /// `abilities` packet by [`Inventory::apply_raw_v26_1`] / [`Inventory::apply_v26_1`].
    #[cfg(feature = "v26_1")]
    pub fn set_creative_26_1(&mut self, creative: bool) {
        self.w26.creative = creative;
    }

    /// 26.1: the packets the creative inventory screen sent in answer to server packets since
    /// the last call (vanilla echoes every window 0 slot the server sets while the screen is
    /// open); send them right away.
    #[cfg(feature = "v26_1")]
    pub fn take_creative_echo_26_1(&mut self) -> Vec<CreativePacket> {
        std::mem::take(&mut self.creative_echo)
    }

    /// Whether the 26.1 creative inventory screen is open (window 0 opened in creative mode):
    /// clicks then return [`Outgoing::Creative`].
    #[cfg(feature = "v26_1")]
    #[must_use]
    pub fn creative_screen_26_1(&self) -> bool {
        self.creative_screen
    }

    /// `InventoryMenu.broadcastChanges` with the creative listener: every slot that differs
    /// from the last broadcast is sent.
    #[cfg(feature = "v26_1")]
    fn creative_broadcast(&mut self, out: &mut Vec<CreativePacket>) {
        let lay = Layout::new(Version::V26_1, Screen::Player);
        for i in 0..46 {
            let Some(l) = lay.loc(i) else { break };
            let cur = self.m.at(l);
            let same = match (&self.creative_last[i], cur) {
                (None, None) => true,
                (Some(a), Some(b)) => a.count == b.count && a.id == b.id && a.data_eq(b),
                _ => false,
            };
            if !same {
                self.creative_last[i] = cur.clone();
                out.push(CreativePacket::SetSlot {
                    slot: i as i16,
                    item: cur.clone(),
                });
            }
        }
    }

    /// A click in the creative inventory screen (`CreativeModeInventoryScreen.slotClicked`,
    /// inventory tab): Q / drops are the screen's own; everything else runs window 0's click
    /// and broadcasts. Refused as unpredictable when ghast cannot simulate it (the stacks it
    /// sends would be wrong).
    #[cfg(feature = "v26_1")]
    fn creative_click(&mut self, slot: i16, button: u8, mode: u8) -> Result<Outgoing, ClickError> {
        let mode = if slot == OUTSIDE && mode == 0 {
            4
        } else {
            mode
        };
        let lay = Layout::new(Version::V26_1, Screen::Player);
        let mut out = Vec::new();
        let idx = usize::try_from(slot).ok();
        if mode == 4 && self.drop_throttle >= 1280 {
            // `canDropItems()` is false: the screen ignores the click
            self.flags.clicked = true;
            return Ok(Outgoing::Creative(out));
        }
        if mode == 4 {
            if let Some(i) = idx
                && let Some(l) = lay.loc(i)
                && let Some(st) = self.m.at(l).clone()
            {
                // slot.remove(button == 0 ? 1 : max stack size): the crafting result is taken
                // whole, other slots split
                let n = if button == 0 {
                    1
                } else {
                    crate::v26_1::max_stack_size(&st)
                };
                let dropped = if l == Loc::CraftResult {
                    self.m.at_mut(l).take()
                } else {
                    let s = self.m.at_mut(l);
                    let k = n.min(st.count).max(0);
                    let d = (k > 0).then(|| st.with_count(k));
                    if let Some(x) = s {
                        x.count -= k;
                        if x.count <= 0 {
                            *s = None;
                        }
                    }
                    d
                };
                if dropped.is_some() {
                    self.drop_throttle += 20;
                    out.push(CreativePacket::Swing);
                    out.push(CreativePacket::SetSlot {
                        slot: -1,
                        item: dropped,
                    });
                }
                out.push(CreativePacket::SetSlot {
                    slot,
                    item: self.m.at(l).clone(),
                });
                self.events.player_changed = true;
                self.flags.clicked = true;
                return Ok(Outgoing::Creative(out));
            }
            if slot == OUTSIDE
                && let Some(c) = self.m.cursor.take()
            {
                self.drop_throttle += 20;
                out.push(CreativePacket::Swing);
                out.push(CreativePacket::SetSlot {
                    slot: -1,
                    item: Some(c),
                });
                self.events.cursor_changed = true;
                self.flags.clicked = true;
                return Ok(Outgoing::Creative(out));
            }
        }
        let mut m = self.m.clone();
        let mut scr = self.s26[0].clone();
        self.w26.binding_curse = self.binding_curse();
        self.w26.can_drop = self.drop_throttle < 1280;
        let (result, swings) = {
            let mut sim = crate::sim26::Sim26 {
                kind: crate::sim26::Kind::Player,
                m: &mut m,
                s: &mut scr,
                w: &self.w26,
                swings: 0,
            };
            let r = sim.click(slot, button as i8, mode);
            (r, sim.swings)
        };
        if result.is_err() {
            return Err(ClickError::Unpredictable);
        }
        for _ in 0..swings {
            out.push(CreativePacket::Swing);
        }
        self.commit(m, lay);
        self.s26[0] = scr;
        self.creative_broadcast(&mut out);
        self.flags.clicked = true;
        Ok(Outgoing::Creative(out))
    }

    /// 26.1: the player's experience level (the anvil's result can only be taken with enough);
    /// set from the `experience` packet by [`Inventory::apply_raw_v26_1`].
    #[cfg(feature = "v26_1")]
    pub fn set_experience_level_26_1(&mut self, level: i32) {
        self.w26.xp_level = Some(level);
    }

    /// 26.1 `update_recipes` (`declare_recipes`): the recipe item sets the furnace, smithing
    /// and stonecutter slot rules read (`(set name, item ids)`, e.g.
    /// `("minecraft:furnace_input", …)`) and the stonecutter recipes' inputs (`None` when one
    /// names a tag: shift-clicks into the stonecutter are then not predicted).
    #[cfg(feature = "v26_1")]
    pub fn on_recipes_26_1<'a>(
        &mut self,
        sets: impl IntoIterator<Item = (&'a str, &'a [i32])>,
        stonecutter_inputs: Option<Vec<i32>>,
    ) {
        let w = &mut self.w26;
        for v in [
            &mut w.furnace_input,
            &mut w.blast_input,
            &mut w.smoker_input,
            &mut w.smithing_base,
            &mut w.smithing_template,
            &mut w.smithing_addition,
        ] {
            v.clear();
        }
        for (name, ids) in sets {
            let target = match name.strip_prefix("minecraft:").unwrap_or(name) {
                "furnace_input" => &mut w.furnace_input,
                "blast_furnace_input" => &mut w.blast_input,
                "smoker_input" => &mut w.smoker_input,
                "smithing_base" => &mut w.smithing_base,
                "smithing_template" => &mut w.smithing_template,
                "smithing_addition" => &mut w.smithing_addition,
                _ => continue,
            };
            target.extend_from_slice(ids);
            target.sort_unstable();
            target.dedup();
        }
        w.cutter_inputs = stonecutter_inputs.map(|mut v| {
            v.sort_unstable();
            v.dedup();
            v
        });
    }

    /// 26.1 `open_screen`: the menu registry id (`minecraft:menu`) and the title. Menus the
    /// client has no screen for are ignored.
    #[cfg(feature = "v26_1")]
    pub fn on_open_screen_26_1(&mut self, id: i32, menu: i32, title: ByteString) {
        self.on_open_window(id, crate::v26_1::window_kind(menu), title);
        if let ScreenState::Window(w) = &mut self.screen
            && w.id == id
        {
            w.menu = menu;
        }
        self.s26[1] = Default::default();
    }

    /// 26.1: the client-side effects of setting the open menu's container slot `k` from a
    /// packet (vanilla runs the slot's `set` and the menu's `slotsChanged`).
    #[cfg(feature = "v26_1")]
    fn inbound_26_1(&mut self, k: usize, content: bool) {
        use crate::sim26::Kind;
        let kind = self.kind_26();
        match kind {
            Kind::Beacon => {
                if let Some(Some(s)) = self.m.container.get_mut(0)
                    && s.count > 1
                {
                    s.count = 1;
                }
            }
            Kind::Stonecutter if k == 0 => {
                let id = self
                    .m
                    .container
                    .first()
                    .and_then(|s| s.as_ref())
                    .map_or(0, |s| s.id);
                if id != self.s26[1].cutter_input {
                    self.s26[1].cutter_input = id;
                    // a content packet sets the result slot after the input
                    if !content && let Some(r) = self.m.container.get_mut(1) {
                        *r = None;
                    }
                }
            }
            Kind::Cartography if k <= 2 => {
                let c = &mut self.m.container;
                if c.len() >= 3 && c[2].is_some() && !(c[0].is_some() && c[1].is_some()) {
                    c[2] = None;
                }
            }
            _ => {}
        }
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
        #[cfg(feature = "v26_1")]
        if self.version == Version::V26_1 && self.w26.creative {
            // the creative screen adds its listener: one broadcast
            self.creative_screen = true;
            let mut out = Vec::new();
            self.creative_broadcast(&mut out);
            return Ok((!out.is_empty()).then_some(Outgoing::Creative(out)));
        }
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
        #[cfg(feature = "v26_1")]
        {
            self.creative_screen = false;
        }
        // 26.1: only the server empties the cursor (`removed` checks for a ServerPlayer); the
        // client's player inventory menu keeps its own, so closing window 0 keeps the cursor,
        // and closing a container returns to window 0's (empty) one.
        let keep_cursor = self.version == Version::V26_1 && self.screen == ScreenState::Inventory;
        if !keep_cursor && self.m.cursor.take().is_some() {
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
                self.m.craft_result_known =
                    !matches!(self.version, Version::V1_16_5 | Version::V26_1);
                self.m.craft_fresh = false;
            }
            ScreenState::Window(_) => {
                self.m.container.clear();
                self.m.container_result_known = true;
                self.m.container_fresh = true;
                #[cfg(feature = "v26_1")]
                {
                    self.s26[1] = Default::default();
                }
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
            menu: -1,
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
        // One loop per version: the per-slot normalising and clamping rules are constants
        // inside it, so the legacy hot path carries no version tests.
        let items = items.into_iter();
        match self.version {
            Version::V1_8 => self.window_items_as::<V18, _>(window_id, items),
            Version::V1_12 => self.window_items_as::<V112, _>(window_id, items),
            Version::V1_16_5 => self.window_items_as::<V1165, _>(window_id, items),
            Version::V26_1 => self.window_items_as::<V261, _>(window_id, items),
        }
    }

    /// `window_items` straight from the 1.8 / 1.12 wire slots: a slot equal to the stored stack
    /// is compared without being converted (no NBT refcount traffic for unchanged slots, the
    /// common case of a refreshed shop window).
    #[cfg(any(feature = "v1_8", feature = "v1_12"))]
    pub(crate) fn on_window_items_legacy(
        &mut self,
        window_id: i32,
        items: &[ghast_protocol::types::slot::legacy::Slot],
    ) {
        match self.version {
            Version::V1_8 => self.window_items_as::<V18, _>(window_id, items.iter()),
            Version::V1_12 => self.window_items_as::<V112, _>(window_id, items.iter()),
            // not reached: the legacy packets belong to 1.8 / 1.12 connections
            _ => self.on_window_items(
                window_id,
                items.iter().cloned().map(crate::stack::from_legacy_slot),
            ),
        }
    }

    #[inline]
    fn window_items_as<const V: u8, W: Incoming>(
        &mut self,
        window_id: i32,
        items: impl Iterator<Item = W>,
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
        for (i, s) in items.enumerate() {
            let Some(l) = lay.loc(i) else { break };
            touched_grid |= is_grid(lay, l);
            let clamps = if V == V261 {
                lay.clamps_26(l)
            } else {
                lay.clamps(l)
            };
            let slot = self.m.at_mut(l);
            if s.settle::<V>(slot, clamps) {
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
        if V == V18 && touched_grid {
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
            crate::sim::clamp_v(self.version, &mut item);
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

    /// 26.1: a container packet arrived while a click waits to settle: the server disagreed.
    fn correction_26_1(&mut self, window_id: Option<i32>) {
        if let Some(p) = self.pending
            && window_id.is_none_or(|w| w == p.window_id || w == 0)
        {
            self.pending = None;
            self.events.clicks.push(ClickResult {
                window_id: p.window_id,
                action: p.action,
                outcome: ClickOutcome::Rejected,
            });
        }
    }

    /// 26.1 `container_set_content` (vanilla `initializeContents`): window 0 or the open
    /// container's slots, its carried stack and its state id. Window 0's carried stack only
    /// shows while no container is open (the client keeps one per menu).
    pub fn on_window_items_26_1(
        &mut self,
        window_id: i32,
        state_id: i32,
        items: impl IntoIterator<Item = Option<Stack>>,
        carried: Option<Stack>,
    ) {
        let current = window_id == self.open_id();
        if window_id != 0 && !current {
            return;
        }
        self.correction_26_1(Some(window_id));
        self.on_window_items(window_id, items);
        if window_id == 0 {
            self.state_player = state_id;
        } else {
            self.state_window = state_id;
            #[cfg(feature = "v26_1")]
            {
                self.inbound_26_1(0, true);
                self.inbound_26_1(2, true);
            }
        }
        if current {
            self.on_set_cursor(carried);
        }
    }

    /// 26.1 `container_set_slot`: window 0 slots always reach window 0 (any slot), the open
    /// container's only that container; the menu's state id follows.
    pub fn on_set_slot_26_1(
        &mut self,
        window_id: i32,
        state_id: i32,
        slot: i16,
        item: Option<Stack>,
    ) {
        let item = self.normalize(item);
        let lay = if window_id == 0 {
            Layout::new(self.version, Screen::Player)
        } else if window_id == self.open_id() {
            self.layout()
        } else {
            return;
        };
        if window_id == 0 {
            self.state_player = state_id;
        } else {
            self.state_window = state_id;
        }
        self.correction_26_1(Some(window_id));
        let Some(l) = usize::try_from(slot).ok().and_then(|i| lay.loc(i)) else {
            return;
        };
        let mut item = item;
        if lay.clamps_26(l) {
            crate::sim::clamp_v(self.version, &mut item);
        }
        match l {
            Loc::CraftResult => self.m.craft_result_known = true,
            Loc::Container(0) if lay.screen == Screen::Container(WindowKind::Crafting) => {
                self.m.container_result_known = true;
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
        #[cfg(feature = "v26_1")]
        if let Loc::Container(k) = l
            && window_id != 0
        {
            self.inbound_26_1(k, false);
        }
        // the creative screen broadcasts window 0 after every server slot
        #[cfg(feature = "v26_1")]
        if window_id == 0 && self.creative_screen {
            let mut out = std::mem::take(&mut self.creative_echo);
            self.creative_broadcast(&mut out);
            self.creative_echo = out;
        }
    }

    /// 26.1 `set_cursor_item`: the open menu's carried stack.
    pub fn on_set_cursor(&mut self, item: Option<Stack>) {
        if self.version == Version::V26_1 {
            self.correction_26_1(None);
            // the creative screen keeps its own cursor
            #[cfg(feature = "v26_1")]
            if self.creative_screen {
                return;
            }
        }
        let item = self.normalize(item);
        if self.m.cursor != item {
            self.m.cursor = item;
            self.events.cursor_changed = true;
        }
    }

    /// 26.1 `set_player_inventory`: a `PlayerInventory` index (0..=40; the body armor and saddle
    /// slots 41 and 42 are not kept).
    pub fn on_set_player_inventory(&mut self, slot: i32, item: Option<Stack>) {
        self.correction_26_1(None);
        let item = self.normalize(item);
        let Some(k) = usize::try_from(slot).ok().filter(|&k| k < 41) else {
            return;
        };
        if self.m.player[k] != item {
            self.m.player[k] = item;
            self.events.player_changed = true;
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
            #[cfg(feature = "v26_1")]
            match w.menu {
                // anvil: the cost data slot
                8 if property == 0 => self.s26[1].anvil_cost = i32::from(value),
                // crafter: slot locks
                7 if (0..9).contains(&property) => {
                    let bit = 1u16 << property;
                    if value == 1 {
                        self.s26[1].crafter_locked |= bit;
                    } else {
                        self.s26[1].crafter_locked &= !bit;
                    }
                }
                _ => {}
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
    ///
    /// Works in place: moving an `Option<Stack>` through a `match` per slot cost the legacy
    /// `window_items` path measurably once `Stack` grew the 26.1 variant.
    #[inline]
    fn normalize(&self, mut s: Option<Stack>) -> Option<Stack> {
        self.normalize_in_place(&mut s);
        s
    }

    #[inline]
    fn normalize_in_place(&self, s: &mut Option<Stack>) {
        if self.version == Version::V1_8 {
            return;
        }
        if s.as_ref().is_some_and(|s| s.count <= 0 || s.id == 0) {
            *s = None;
        } else if self.version != Version::V1_12
            && let Some(st) = s
        {
            st.damage = 0;
        }
    }
}
