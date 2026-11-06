//! The deterministic client core: game state, the behaviour contract and the tick.
//!
//! No I/O and no clock here: the connection feeds decoded [`Inbound`] packets and the time
//! in, and takes the [`Outbound`] packets out, so the tick order can be tested without a
//! socket.
//!
//! # Tick model
//!
//! Mirrors 1.8.9 `MinecraftClient.tick` as recorded in vanilla-client packet traces (and is the same
//! in 1.16.5):
//!
//! 1. inbound packets that arrived since the last tick have been applied (on arrival, like
//!    vanilla's per-frame task queue; keep-alives, teleports and window confirms are
//!    answered then);
//! 2. [`Behavior::tick`] runs. Actions (`chat`, `select_slot`, `swing`, `respawn`, …) are
//!    written immediately, in call order, before this tick's movement packet;
//!    [`TickContext::set_aim`] / [`TickContext::set_controls`] only record intent;
//! 3. held slot sync → rotation step (whole mouse counts) → movement step → sprint change →
//!    sneak change → exactly one movement packet.
//!
//! The server therefore judges this tick's actions with the *previous* tick's sent pose
//! ([`TickContext::sent`]).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::common::{
    Abilities, ChatMessage, ClientSettings, Controls, GameMode, Health, Inbound, MovePacket,
    Outbound, Pose, Vec3,
};
use crate::hands::{self, BlockHit, Buttons, DigFactors, Hands, HandsIo};
use crate::interest::Interests;
use crate::rotation::Rotation;
use crate::sim::{Sim, WorldView};
use crate::version::GameVersion;

/// Pending chat messages kept for the behaviour; more are dropped (and counted) while the
/// behaviour is not ticking.
const MAX_PENDING_CHAT: usize = 512;

/// A bot behaviour: the code that decides what the player does each tick.
///
/// Behaviours never see packets or versions: they read a [`TickContext`] and call its
/// actions. A panic in a behaviour stops that behaviour (its [`Behavior::stop`] is called with
/// `"panic"`) but not the connection: the player keeps standing and answering the server.
pub trait Behavior {
    /// Name for logs.
    fn name(&self) -> &str {
        "behavior"
    }

    /// Optional packet groups this behaviour needs. Read every tick; a change updates the
    /// packet filter. Everything not asked for is dropped before it is decoded.
    fn interests(&self) -> Interests {
        Interests::NONE
    }

    /// Extra clientbound play packet ids (of `version`) to copy undecoded into
    /// [`TickEvents::raw`], including packets the core consumes itself (the core's handling
    /// does not change). Read every tick like [`Behavior::interests`]; a change updates the
    /// packet filter. Decoding is the behaviour's job (`ghast::protocol::v1_8::play::clientbound`
    /// / `v1_16_5`).
    fn raw_packets(&self, version: GameVersion) -> std::borrow::Cow<'static, [i32]> {
        let _ = version;
        std::borrow::Cow::Borrowed(&[])
    }

    /// Called on the first tick the player is in the world, before that tick's
    /// [`Behavior::tick`] (both see the same events).
    fn start(&mut self, ctx: &mut TickContext<'_>) {
        let _ = ctx;
    }

    /// Called every tick while the player is in the world and the server is answering.
    fn tick(&mut self, ctx: &mut TickContext<'_>);

    /// Whether the behaviour still wants ticks. Once it returns `false` the behaviour is done:
    /// it is not ticked again (the player keeps standing and answering the server) and
    /// [`Behavior::stop`] still runs when the session ends. Read after every start / tick.
    fn running(&self) -> bool {
        true
    }

    /// Called after every tick's movement step (also while the behaviour is on hold or has
    /// finished), with a read-only view of what the server now has: the pose sent this tick,
    /// the world, entities and inventory. For observers (a viewer, a recorder); it cannot act.
    /// Default: nothing.
    ///
    /// ```
    /// use ghast_client::{Behavior, TickContext, TickView};
    ///
    /// /// Logs where the server has the player, once a second.
    /// struct Tracker;
    ///
    /// impl Behavior for Tracker {
    ///     fn tick(&mut self, _ctx: &mut TickContext<'_>) {}
    ///
    ///     fn after_tick(&mut self, view: &TickView<'_>) {
    ///         if view.tick() % 20 == 0 {
    ///             let p = view.sent();
    ///             println!("{} {:?} in {}", view.tick(), p.pos, view.dimension());
    ///         }
    ///     }
    /// }
    /// ```
    fn after_tick(&mut self, view: &TickView<'_>) {
        let _ = view;
    }

    /// Called once when the session ends (or the behaviour panicked).
    fn stop(&mut self, reason: &str) {
        let _ = reason;
    }
}

/// A boxed behaviour is a behaviour (a runner can pick one from its config at run time).
impl<B: Behavior + ?Sized> Behavior for Box<B> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn interests(&self) -> Interests {
        (**self).interests()
    }
    fn raw_packets(&self, version: GameVersion) -> std::borrow::Cow<'static, [i32]> {
        (**self).raw_packets(version)
    }
    fn start(&mut self, ctx: &mut TickContext<'_>) {
        (**self).start(ctx);
    }
    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        (**self).tick(ctx);
    }
    fn running(&self) -> bool {
        (**self).running()
    }
    fn after_tick(&mut self, view: &TickView<'_>) {
        (**self).after_tick(view);
    }
    fn stop(&mut self, reason: &str) {
        (**self).stop(reason);
    }
}

/// Why [`TickContext::click`] (or opening / closing a window) sent nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClickRefused {
    /// The player is turning: click only while still.
    Turning,
    /// The inventory refused it.
    Inventory(ghast_inventory::ClickError),
}

impl std::fmt::Display for ClickRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClickRefused::Turning => f.write_str("the player is turning"),
            ClickRefused::Inventory(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ClickRefused {}

/// A click that was sent: match it with [`TickEvents::inventory`]'s `clicks` on a later tick
/// (window id and action number, as the server's confirm carries them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClickTicket {
    /// Window id the click was sent for.
    pub window_id: i32,
    /// Its action number.
    pub action: i16,
}

impl ClickTicket {
    /// Whether `r` is the result of this click.
    #[must_use]
    pub fn matches(&self, r: &ghast_inventory::ClickResult) -> bool {
        r.window_id == self.window_id && r.action == self.action
    }
}

/// A behaviour that does nothing (the player stands still).
#[derive(Debug, Clone, Copy, Default)]
pub struct Idle;

impl Behavior for Idle {
    fn name(&self) -> &str {
        "idle"
    }

    fn tick(&mut self, _ctx: &mut TickContext<'_>) {}
}

/// A clientbound play packet copied for the behaviour ([`Behavior::raw_packets`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RawInbound {
    /// Packet id (version-specific).
    pub id: i32,
    /// The body after the id, undecoded (a cheap reference-counted copy).
    pub body: bytes::Bytes,
    /// When the frame was decoded from the socket (frames read together share it).
    pub at: Instant,
}

/// A server teleport / correction (`position`) with the poses around it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct TeleportEvent {
    /// The packet's values as received: absolute or relative per
    /// [`crate::common::teleport_flags`] in `flags`.
    pub teleport: crate::common::Teleport,
    /// What the server had from us just before ([`TickContext::sent`] before the overwrite):
    /// compare with [`TeleportEvent::after`] to see what the server moved / turned us by.
    pub sent_before: Pose,
    /// The client pose just before (it may be ahead of `sent_before` within a tick).
    pub pose_before: Pose,
    /// The pose after applying it (relative fields resolved).
    pub after: Pose,
}

/// Most teleports kept between two behaviour ticks.
const MAX_PENDING_TELEPORTS: usize = 64;

/// Most raw packets kept between two behaviour ticks.
const MAX_PENDING_RAW: usize = 4096;

/// What happened since the behaviour's previous tick.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct TickEvents {
    /// Chat messages in arrival order (only with [`Interests::CHAT`]).
    pub chat: Vec<ChatMessage>,
    /// Chat messages dropped because too many piled up while the behaviour was not ticking.
    pub chat_dropped: usize,
    /// The server teleported / corrected the player.
    pub teleported: bool,
    /// Every teleport in arrival order, with the sent pose before it (at most 64 per tick).
    pub teleports: Vec<TeleportEvent>,
    /// A join game packet arrived (first join or a proxy server switch).
    pub joined: bool,
    /// The player respawned or changed dimension.
    pub respawned: bool,
    /// The player died (health reached 0).
    pub died: bool,
    /// Health, food or saturation changed.
    pub health_changed: bool,
    /// The game mode changed.
    pub game_mode_changed: bool,
    /// Inventory / window changes and click results (ghast-inventory).
    pub inventory: ghast_inventory::Events,
    /// Block changes in arrival order (only with [`Interests::BLOCK_UPDATES`]): position and
    /// the version's raw block state (1.8 `id << 4 | meta`, 1.16 the global palette id).
    /// The world already holds them when the behaviour ticks.
    pub block_updates: Vec<(ghast_protocol::BlockPos, u32)>,
    /// Block changes dropped because too many piled up between two behaviour ticks.
    pub block_updates_dropped: usize,
    /// The packets asked for with [`Behavior::raw_packets`], in arrival order (kept while the
    /// behaviour is on hold, like chat).
    pub raw: Vec<RawInbound>,
    /// Raw packets dropped because more than 4096 piled up between two behaviour ticks.
    pub raw_dropped: usize,
}

impl TickEvents {
    fn clear(&mut self) {
        self.chat.clear();
        self.chat_dropped = 0;
        self.teleported = false;
        self.teleports.clear();
        self.joined = false;
        self.respawned = false;
        self.died = false;
        self.health_changed = false;
        self.game_mode_changed = false;
        self.inventory = ghast_inventory::Events::default();
        self.block_updates.clear();
        self.block_updates_dropped = 0;
        self.raw.clear();
        self.raw_dropped = 0;
    }
}

/// Tunables of the core.
#[derive(Debug, Clone)]
pub(crate) struct CoreOptions {
    pub name: Arc<str>,
    pub version: GameVersion,
    pub settings: ClientSettings,
    pub brand: Option<String>,
    pub sensitivity: f32,
    pub silent_hold: Duration,
    pub inventory: ghast_inventory::Version,
    /// Keep the look still on a tick that dug / placed when the turn would take the
    /// crosshair off the clicked block (Grim RotationPlace re-checks with the next look).
    pub hold_turn_on_action: bool,
}

/// Client state shared by the connection and the behaviour context.
#[derive(Debug)]
pub(crate) struct Core {
    opts: CoreOptions,
    tick: u64,
    entity_id: i32,
    joined: bool,
    teleported_once: bool,
    ready: bool,
    pose: Pose,
    sent: Pose,
    rotation: Rotation,
    aimed: bool,
    controls: Controls,
    health: Health,
    dead: bool,
    abilities: Abilities,
    game_mode: GameMode,
    held_slot: u8,
    sent_slot: u8,
    open_window: i32,
    dimension: String,
    respawn_screen: bool,
    pending: TickEvents,
    current: TickEvents,
    out: Vec<Outbound>,
    last_inbound: Option<Instant>,
    silent: bool,
    quit: Option<String>,
    status: String,
    hands: Hands,
    buttons: Buttons,
    /// Inventory and windows (ghast-inventory).
    inv: ghast_inventory::Inventory,
    /// The look changed in the last movement step (window clicks need a still player).
    turned: bool,
    /// Our active status effects: id → amplifier.
    effects: std::collections::BTreeMap<i8, i8>,
    /// Aqua affinity on the helmet (dig speed under water).
    aqua_affinity: bool,
    /// Blocks broken on the client this tick (predicted air).
    broken_now: Vec<ghast_protocol::BlockPos>,
    /// Flight wanted by the behaviour this tick ([`TickContext::set_flying`]).
    want_flying: Option<bool>,
    /// Turns held back because they would have moved the crosshair off a clicked block.
    held_turns: u64,
    /// Arm swings (main and off hand) written in finished ticks.
    swings: u64,
    /// Test hook: this tick's look, set instead of a rotation step.
    #[cfg(test)]
    forced_look: Option<(f32, f32)>,
}

/// Behaviour bookkeeping of the engine.
#[derive(Debug, Default)]
struct BehaviorState {
    started: bool,
    failed: bool,
    /// The behaviour reported [`Behavior::running`] `false`.
    done: bool,
}

/// The core plus the behaviour's lifecycle.
#[derive(Debug)]
pub(crate) struct Engine<S> {
    pub core: Core,
    pub sim: S,
    state: BehaviorState,
}

impl Core {
    fn new(opts: CoreOptions) -> Self {
        let rotation = Rotation::new(opts.sensitivity);
        let inv = ghast_inventory::Inventory::new(opts.inventory);
        Core {
            opts,
            tick: 0,
            entity_id: 0,
            joined: false,
            teleported_once: false,
            ready: false,
            pose: Pose::default(),
            sent: Pose::default(),
            rotation,
            aimed: false,
            controls: Controls::default(),
            health: Health::default(),
            dead: false,
            abilities: Abilities::default(),
            game_mode: GameMode::default(),
            held_slot: 0,
            sent_slot: 0,
            open_window: 0,
            dimension: String::new(),
            respawn_screen: true,
            pending: TickEvents::default(),
            current: TickEvents::default(),
            out: Vec::with_capacity(16),
            last_inbound: None,
            silent: false,
            quit: None,
            status: String::new(),
            hands: Hands::default(),
            buttons: Buttons::default(),
            inv,
            turned: false,
            effects: std::collections::BTreeMap::new(),
            aqua_affinity: false,
            broken_now: Vec::new(),
            want_flying: None,
            held_turns: 0,
            swings: 0,
            #[cfg(test)]
            forced_look: None,
        }
    }

    /// A new client player entity (join, respawn): `lastReported*` zero, nothing held down,
    /// waiting for the server's position.
    fn reset_player(&mut self, why: &str) {
        self.teleported_once = false;
        self.ready = false;
        self.pose = Pose::default();
        self.sent = Pose::default();
        self.controls = Controls::default();
        self.aimed = false;
        self.open_window = 0;
        self.dead = false;
        self.hands.reset();
        self.buttons = Buttons::default();
        self.effects.clear();
        log::debug!(
            "[{}] t{} {why}: waiting for the server position",
            self.opts.name,
            self.tick
        );
    }

    fn column_loaded(&self, world: &dyn WorldView) -> bool {
        // BlockPos(posX, 0, posZ) floors the coordinates.
        let cx = (self.pose.pos.x.floor() as i64 >> 4) as i32;
        let cz = (self.pose.pos.z.floor() as i64 >> 4) as i32;
        world.is_column_loaded(cx, cz)
    }
}

/// The held stack's dig / use facts.
fn held_item<W: Sim + ?Sized>(
    inv: &ghast_inventory::Inventory,
    sim: &W,
) -> Option<hands::HeldItem> {
    let s = inv.held()?;
    let id = u16::try_from(s.id).ok()?;
    let count = u8::try_from(s.count.clamp(0, 255)).unwrap_or(0);
    Some(sim.held_item(id, count, efficiency(s)))
}

/// Whether rays hit the entity (`Entity.collides`): living entities, players,
/// paintings and the solid objects (boats, minecarts, TNT, falling blocks, item frames, armor
/// stands, leash knots, end crystals); not items, orbs, projectiles.
fn collidable(e: &ghast_entity::Entity) -> bool {
    use ghast_entity::{EntityClass, EntityType};
    match e.class() {
        EntityClass::Player | EntityClass::Mob | EntityClass::Painting => true,
        EntityClass::Item | EntityClass::ExperienceOrb => false,
        EntityClass::Object => match e.entity_type() {
            EntityType::LegacyObject(t) => matches!(t, 1 | 10 | 50 | 51 | 70 | 71 | 77 | 78),
            EntityType::Registry(_) => {
                ghast_entity::v1_16_5::info(e.entity_type()).is_some_and(|i| {
                    matches!(
                        i.name,
                        "boat"
                            | "tnt"
                            | "end_crystal"
                            | "falling_block"
                            | "item_frame"
                            | "leash_knot"
                            | "armor_stand"
                            | "shulker_bullet"
                    ) || i.name.contains("minecart")
                })
            }
            _ => false,
        },
    }
}

/// The entity under the crosshair from `eyes` along `look` (vanilla `GameRenderer.updateTargetedEntity`), if
/// entities are tracked.
fn entity_pick<W: WorldView + ?Sized>(
    w: &W,
    version: GameVersion,
    eyes: Vec3,
    look: Vec3,
    block_distance: Option<f64>,
) -> Option<hands::EntityHit> {
    let ents = w.entities()?;
    let riding = ents.me().and_then(ghast_entity::Entity::vehicle);
    // 1.8 grows entity boxes by the 0.1 collision border; 1.9+ does not.
    let border = if version == GameVersion::V1_8_9 {
        0.1
    } else {
        0.0
    };
    let cands = ents
        .within([eyes.x, eyes.y, eyes.z], 8.0)
        .filter(|e| collidable(e))
        .filter_map(|e| {
            let p = e.position();
            Some((e.id(), e.bounding_box()?, Vec3::new(p[0], p[1], p[2])))
        });
    hands::pick_entity(cands, eyes, look, block_distance, border, riding)
}

/// The off-hand stack's facts (1.9+).
fn off_held_item<W: Sim + ?Sized>(
    inv: &ghast_inventory::Inventory,
    sim: &W,
) -> Option<hands::HeldItem> {
    let s = inv.offhand()?;
    let id = u16::try_from(s.id).ok()?;
    let count = u8::try_from(s.count.clamp(0, 255)).unwrap_or(0);
    Some(sim.held_item(id, count, 0))
}

/// Efficiency level of a stack (see [`enchant_level`]).
fn efficiency(s: &ghast_inventory::Stack) -> u8 {
    enchant_level(s, 32, "minecraft:efficiency")
}

/// An enchantment level from the stack NBT: the 1.8 `ench` list with numeric ids, or the 1.13+
/// `Enchantments` list with names. Parsed only when the stack has NBT.
fn enchant_level(s: &ghast_inventory::Stack, id_1_8: i32, name: &str) -> u8 {
    use ghast_protocol::nbt::Tag;
    let Some(root) = s.nbt.as_ref().and_then(|n| n.parse().ok().flatten()) else {
        return 0;
    };
    let Some(c) = root.tag.as_compound() else {
        return 0;
    };
    let num = |t: Option<&Tag>| match t {
        Some(Tag::Short(v)) => i32::from(*v),
        Some(Tag::Int(v)) => *v,
        Some(Tag::Byte(v)) => i32::from(*v),
        _ => -1,
    };
    for key in ["ench", "Enchantments"] {
        if let Some(Tag::List(l)) = c.get(key) {
            for e in l.items.iter().filter_map(Tag::as_compound) {
                let hit = match e.get("id") {
                    Some(Tag::String(n)) => n == name,
                    other => num(other) == id_1_8,
                };
                if hit {
                    return u8::try_from(num(e.get("lvl")).clamp(0, 255)).unwrap_or(0);
                }
            }
        }
    }
    0
}

impl<S: Sim> Engine<S> {
    pub(crate) fn new(opts: CoreOptions, sim: S) -> Self {
        Engine {
            core: Core::new(opts),
            sim,
            state: BehaviorState::default(),
        }
    }

    /// Bytes arrived from the server (liveness for the silent-server guard).
    pub(crate) fn note_inbound(&mut self, now: Instant) {
        self.core.last_inbound = Some(now);
    }

    /// Time of the last inbound bytes.
    pub(crate) fn last_inbound(&self) -> Option<Instant> {
        self.core.last_inbound
    }

    /// The bot's name (for logs).
    pub(crate) fn name(&self) -> &str {
        &self.core.opts.name
    }

    /// Queues a raw packet copy for the behaviour's next tick.
    pub(crate) fn push_raw(&mut self, p: RawInbound) {
        let ev = &mut self.core.pending;
        if ev.raw.len() < MAX_PENDING_RAW {
            ev.raw.push(p);
        } else {
            ev.raw_dropped += 1;
        }
    }

    /// The raw packet ids the behaviour wants (none once it failed or finished).
    pub(crate) fn raw_packets<B: Behavior>(&self, b: &B) -> std::borrow::Cow<'static, [i32]> {
        if self.state.failed || self.state.done {
            std::borrow::Cow::Borrowed(&[])
        } else {
            b.raw_packets(self.core.opts.version)
        }
    }

    /// The inventory (the connection feeds it its packets).
    pub(crate) fn inventory_mut(&mut self) -> &mut ghast_inventory::Inventory {
        &mut self.core.inv
    }

    /// Outbound packets produced since the last call, in send order.
    pub(crate) fn outbound(&mut self) -> &mut Vec<Outbound> {
        &mut self.core.out
    }

    /// Set when the behaviour asked to disconnect.
    pub(crate) fn quit_requested(&self) -> Option<&str> {
        self.core.quit.as_deref()
    }

    pub(crate) fn core_name(&self) -> &str {
        &self.core.opts.name
    }

    pub(crate) fn tick_count(&self) -> u64 {
        self.core.tick
    }

    /// Applies one inbound packet (except disconnect and compression, which the connection
    /// handles).
    pub(crate) fn handle(&mut self, p: Inbound) {
        let c = &mut self.core;
        let sim = &mut self.sim;
        match p {
            Inbound::KeepAlive(id) => c.out.push(Outbound::KeepAlive(id)),
            Inbound::JoinGame(j) => {
                c.entity_id = j.entity_id;
                c.game_mode = j.game_mode;
                c.respawn_screen = j.respawn_screen;
                // A join always creates a new client world (the world itself was cleared by
                // the world packet routing).
                c.dimension = j.dimension;
                c.joined = true;
                c.pending.joined = true;
                c.health = Health::default();
                c.abilities = Abilities::default();
                c.reset_player("join game");
                sim.reset_player();
                sim.spectator(c.game_mode == GameMode::Spectator);
                // ClientPlayNetworkHandler.onGameJoin: settings, then the brand.
                c.out.push(Outbound::Settings(c.opts.settings.clone()));
                if let Some(brand) = &c.opts.brand {
                    c.out.push(Outbound::Brand(brand.clone()));
                }
                log::info!(
                    "[{}] joined: entity {} {:?} dimension {}",
                    c.opts.name,
                    c.entity_id,
                    c.game_mode,
                    c.dimension
                );
            }
            Inbound::Respawn {
                dimension,
                game_mode,
            } => {
                if dimension != c.dimension {
                    c.dimension = dimension;
                }
                c.game_mode = game_mode;
                c.pending.respawned = true;
                c.reset_player("respawn");
                sim.reset_player();
                sim.spectator(c.game_mode == GameMode::Spectator);
            }
            Inbound::Teleport(t) => {
                if c.teleported_once {
                    log::debug!(
                        "[{}] t{} teleport {:?} {:?} {:?} yaw {:?} pitch {:?} flags {}; we had sent \
                         {:?} {:?} {:?} yaw {:?} pitch {:?} ground {}",
                        c.opts.name,
                        c.tick,
                        t.x,
                        t.y,
                        t.z,
                        t.yaw,
                        t.pitch,
                        t.flags,
                        c.sent.pos.x,
                        c.sent.pos.y,
                        c.sent.pos.z,
                        c.sent.yaw,
                        c.sent.pitch,
                        c.sent.on_ground
                    );
                }
                let (sent_before, pose_before) = (c.sent, c.pose);
                sim.teleport(&mut c.pose, &t);
                if c.pending.teleports.len() < MAX_PENDING_TELEPORTS {
                    c.pending.teleports.push(TeleportEvent {
                        teleport: t,
                        sent_before,
                        pose_before,
                        after: c.pose,
                    });
                }
                if let Some(id) = t.teleport_id {
                    c.out.push(Outbound::TeleportConfirm(id));
                }
                c.out.push(Outbound::Move(MovePacket::PositionLook {
                    pos: c.pose.pos,
                    yaw: c.pose.yaw,
                    pitch: c.pose.pitch,
                    on_ground: false,
                }));
                // The server now has this pose; vanilla's lastReported* are not touched.
                c.sent = Pose {
                    on_ground: false,
                    ..c.pose
                };
                if !c.teleported_once {
                    log::debug!(
                        "[{}] t{} spawn position {:.3} {:.3} {:.3}",
                        c.opts.name,
                        c.tick,
                        c.pose.pos.x,
                        c.pose.pos.y,
                        c.pose.pos.z
                    );
                }
                c.teleported_once = true;
                c.pending.teleported = true;
            }
            Inbound::Chat(m) => {
                if c.pending.chat.len() < MAX_PENDING_CHAT {
                    c.pending.chat.push(m);
                } else {
                    c.pending.chat_dropped += 1;
                }
            }
            Inbound::Health(h) => {
                let was_dead = c.dead;
                c.health = h;
                sim.food(h.food);
                c.pending.health_changed = true;
                if h.health <= 0.0 && !was_dead {
                    c.dead = true;
                    c.pending.died = true;
                    log::info!("[{}] t{} died", c.opts.name, c.tick);
                }
            }
            Inbound::Died { entity_id } => {
                if entity_id == c.entity_id && !c.respawn_screen {
                    // 1.16 immediate respawn: the client respawns without a death screen.
                    c.out.push(Outbound::Respawn);
                }
            }
            Inbound::Abilities(a) => {
                c.abilities = a;
                c.pose.flying = a.flying();
                sim.abilities(a);
            }
            Inbound::HeldSlot(s) => {
                if (0..9).contains(&s) {
                    // Only the inventory's selection changes; the controller's copy does not,
                    // so the next sync echoes it (as vanilla does).
                    if c.held_slot != s as u8 {
                        c.hands.stop_using();
                    }
                    c.held_slot = s as u8;
                }
            }
            Inbound::GameState { reason, value } => match reason {
                3 => {
                    c.game_mode = GameMode::from_wire(value as u8);
                    sim.spectator(c.game_mode == GameMode::Spectator);
                    c.pending.game_mode_changed = true;
                }
                // 1.15+: "immediate respawn" gamerule (1 = no respawn screen).
                11 if c.opts.version != GameVersion::V1_8_9 => c.respawn_screen = value == 0.0,
                _ => {}
            },
            // Window confirms are answered by the inventory (exactly once).
            Inbound::Transaction { .. } => {}
            Inbound::OpenWindow(id) => c.open_window = id,
            Inbound::CloseWindow(_) => c.open_window = 0,
            Inbound::Velocity { entity_id, motion } => {
                if entity_id == c.entity_id {
                    sim.velocity(motion, false);
                }
            }
            Inbound::ExplosionPush(motion) => sim.velocity(motion, true),
            Inbound::MovementSpeed {
                entity_id,
                base,
                modifiers,
            } => {
                if entity_id == c.entity_id {
                    log::debug!(
                        "[{}] t{} movement speed base {base:?} modifiers {modifiers:?}",
                        c.opts.name,
                        c.tick
                    );
                    let mods: Vec<(u128, f64, u8)> = modifiers
                        .iter()
                        .map(|&(id, amount, op)| (id.as_u128(), amount, op as u8))
                        .collect();
                    sim.movement_speed(base, &mods);
                }
            }
            Inbound::Effect {
                entity_id,
                id,
                amplifier,
            } => {
                if entity_id == c.entity_id {
                    c.effects.insert(id, amplifier);
                    sim.status_effects(&c.effects);
                }
            }
            Inbound::EffectRemoved { entity_id, id } => {
                if entity_id == c.entity_id {
                    c.effects.remove(&id);
                    sim.status_effects(&c.effects);
                }
            }
            Inbound::EntityStatus { entity_id, status } => {
                // 9: the server finished our item use (eating, drinking).
                if entity_id == c.entity_id && status == 9 {
                    c.hands.stop_using();
                }
            }
            Inbound::Disconnect(_) | Inbound::SetCompression(_) => {}
        }
    }

    /// The interests of the behaviour (the packet filter follows them).
    pub(crate) fn interests<B: Behavior>(&self, b: &B) -> Interests {
        if self.state.failed {
            Interests::NONE
        } else {
            b.interests()
        }
    }

    /// Ends the behaviour (session over).
    pub(crate) fn stop<B: Behavior>(&mut self, b: &mut B, reason: &str) {
        if self.state.started && !self.state.failed {
            self.state.failed = true;
            let name = self.core.opts.name.clone();
            if catch_unwind(AssertUnwindSafe(|| b.stop(reason))).is_err() {
                log::error!("[{name}] behaviour {} panicked in stop", b.name());
            }
        }
    }

    /// One client tick at `now`.
    pub(crate) fn tick<B: Behavior>(&mut self, b: &mut B, now: Instant) {
        let c = &mut self.core;
        c.tick += 1;
        // Per-tick packet-order guard and click timeout of the inventory.
        c.inv.tick();
        let was_ready = c.ready;
        c.ready = c.teleported_once && c.column_loaded(&self.sim);
        if c.ready != was_ready {
            if c.ready {
                log::info!(
                    "[{}] t{} in the world at {:.3} {:.3} {:.3}",
                    c.opts.name,
                    c.tick,
                    c.pose.pos.x,
                    c.pose.pos.y,
                    c.pose.pos.z
                );
            } else {
                log::debug!(
                    "[{}] t{} player column not loaded: not ticking",
                    c.opts.name,
                    c.tick
                );
            }
        }
        if !c.ready {
            // The client player does not tick outside a loaded column (1.16 vanilla; on 1.8
            // the bot waits for its column as well).
            return;
        }

        let silent_for = c.last_inbound.map(|t| now.saturating_duration_since(t));
        let silent = silent_for.is_some_and(|d| d > c.opts.silent_hold);
        if silent != c.silent {
            c.silent = silent;
            if silent {
                log::warn!(
                    "[{}] t{} server silent for {} ms: behaviour on hold",
                    c.opts.name,
                    c.tick,
                    silent_for.unwrap_or_default().as_millis()
                );
            } else {
                log::info!(
                    "[{}] t{} server answering again: behaviour resumes",
                    c.opts.name,
                    c.tick
                );
            }
        }

        if silent {
            // Freeze: release the keys, keep only the vanilla movement packet.
            c.controls = Controls::default();
            c.aimed = false;
        } else if !self.state.failed && !self.state.done {
            std::mem::swap(&mut c.pending, &mut c.current);
            c.pending.clear();
            c.current.inventory = c.inv.drain_events();
            c.current.block_updates_dropped =
                self.sim.drain_block_updates(&mut c.current.block_updates);
            if c.current.inventory.player_changed {
                // Armour enchantments that change movement / digging.
                let ench = |a: usize, id: i32, name: &str| {
                    c.inv.armor(a).map_or(0, |s| enchant_level(s, id, name))
                };
                let depth = ench(3, 8, "minecraft:depth_strider");
                let soul = ench(3, -1, "minecraft:soul_speed");
                c.aqua_affinity = ench(0, 6, "minecraft:aqua_affinity") > 0;
                self.sim.boots(depth, soul);
            }
            let started = self.state.started;
            self.state.started = true;
            if !started {
                c.rotation.reset(c.pose.yaw, c.pose.pitch);
            }
            let name = c.opts.name.clone();
            let result = catch_unwind(AssertUnwindSafe(|| {
                let held = held_item(&c.inv, &self.sim);
                let mut ctx = TickContext {
                    core: c,
                    world: &self.sim,
                    held,
                };
                if !started {
                    b.start(&mut ctx);
                }
                b.tick(&mut ctx);
                b.running()
            }));
            match result {
                Ok(true) => {}
                Ok(false) => {
                    self.state.done = true;
                    log::info!("[{name}] behaviour {} finished", b.name());
                }
                Err(_) => {
                    log::error!("[{name}] behaviour {} panicked: stopped", b.name());
                    self.stop(b, "panic");
                }
            }
        }

        let c = &mut self.core;
        // ClientPlayerInteractionManager.tick (syncSelectedSlot) runs before the key
        // processing: a held slot the server changed is echoed before any click.
        if c.held_slot != c.sent_slot {
            c.sent_slot = c.held_slot;
            c.out.push(Outbound::HeldSlot(i16::from(c.sent_slot)));
        }
        // MinecraftClient.tick key processing (a GUI takes the mouse: buttons up).
        {
            // The screen as the inventory models it (vanilla: a window the client has no
            // screen for opens none; a window the client closed is gone at once).
            let gui = c.inv.gui_open();
            let eye = if c.pose.sneaking {
                c.opts.version.sneak_eye_height()
            } else {
                c.opts.version.eye_height()
            };
            let eyes = Vec3::new(c.pose.pos.x, c.pose.pos.y + eye, c.pose.pos.z);
            let sim = &self.sim;
            let blocks = |x: i32, y: i32, z: i32| sim.block_query(x, y, z);
            let look = hands::look_vector(c.pose.yaw, c.pose.pitch);
            let target = hands::pick(&blocks, eyes, look, hands::REACH);
            let entity = entity_pick(sim, c.opts.version, eyes, look, target.map(|t| t.distance));
            let in_water = blocks(
                eyes.x.floor() as i32,
                eyes.y.floor() as i32,
                eyes.z.floor() as i32,
            )
            .is_some_and(|b| b.water);
            let held = held_item(&c.inv, sim);
            let mut placed = false;
            let first = c.out.len();
            let mut io = HandsIo {
                blocks: &blocks,
                target,
                held,
                held_stack: c.inv.held().cloned(),
                food: c.health.food,
                modern: c.opts.version != GameVersion::V1_8_9,
                off_held: off_held_item(&c.inv, sim),
                off_stack: c.inv.offhand().cloned(),
                entity,
                factors: DigFactors {
                    on_ground: c.pose.on_ground,
                    in_water,
                    aqua_affinity: c.aqua_affinity,
                    haste: c.effects.get(&3).map_or(0, |a| (*a as u8).wrapping_add(1)),
                    fatigue: c.effects.get(&4).map_or(0, |a| (*a as u8).wrapping_add(1)),
                },
                sneaking: c.pose.sneaking,
                out: &mut c.out,
                broken: &mut c.broken_now,
                placed: &mut placed,
            };
            let (left, right) = (c.buttons.left && !gui, c.buttons.right && !gui);
            c.hands.tick(left, right, &mut io);
            // Other placements: the server's set_slot brings the new count of the held stack.
            let _ = placed;
            predict_planting(c, sim, first);
        }
        for p in c.broken_now.drain(..) {
            self.sim.predict_air(p.x, p.y, p.z);
        }
        // ClientPlayerEntity.tickMovement: a flight toggle is told to the server before the move.
        if let Some(f) = c.want_flying.take() {
            let first = c.out.len();
            self.sim.set_flying(f, &mut c.out);
            if c.out.len() > first {
                c.pose.flying = f;
            } else if f && !c.pose.flying {
                log::debug!(
                    "[{}] t{} set_flying(true) refused: flight not allowed",
                    c.opts.name,
                    c.tick
                );
            }
        }
        // Rotation runs only on ticks the behaviour aimed (budgets untouched otherwise).
        c.turned = false;
        #[cfg(test)]
        if let Some((yaw, pitch)) = c.forced_look.take() {
            c.turned = yaw != c.pose.yaw || pitch != c.pose.pitch;
            c.pose.yaw = yaw;
            c.pose.pitch = pitch;
            c.aimed = false;
        }
        if c.aimed {
            let (yaw, pitch) = c.rotation.step(c.pose.yaw, c.pose.pitch);
            if c.opts.hold_turn_on_action && !post_look_ok(c, yaw, pitch) {
                // Grim RotationPlace re-checks a dig / place with the look of the movement
                // packet that follows it: keep the mouse still on the clicked block this tick.
                c.held_turns += 1;
            } else {
                c.turned = yaw != c.pose.yaw || pitch != c.pose.pitch;
                c.pose.yaw = yaw;
                c.pose.pitch = pitch;
            }
            c.aimed = false;
        }
        let first = c.out.len();
        let using = c.hands.using_item();
        self.sim
            .step(&mut c.pose, c.controls, using, c.entity_id, &mut c.out);
        for o in &c.out[first..] {
            let Outbound::Move(m) = o else { continue };
            match *m {
                MovePacket::Position { pos, .. } => c.sent.pos = pos,
                MovePacket::Look { yaw, pitch, .. } => {
                    c.sent.yaw = yaw;
                    c.sent.pitch = pitch;
                }
                MovePacket::PositionLook {
                    pos, yaw, pitch, ..
                } => {
                    c.sent.pos = pos;
                    c.sent.yaw = yaw;
                    c.sent.pitch = pitch;
                }
                MovePacket::Flying { .. } => {}
            }
            if matches!(
                m,
                MovePacket::Position { .. } | MovePacket::PositionLook { .. }
            ) {
                c.sent.motion = c.pose.motion;
            }
        }
        c.sent.on_ground = c.pose.on_ground;
        c.sent.sneaking = c.pose.sneaking;
        c.sent.sprinting = c.pose.sprinting;
        c.sent.flying = c.pose.flying;
        c.swings += count_swings(&c.out);

        if self.state.started && !self.state.failed {
            let name = c.opts.name.clone();
            let view = TickView {
                core: &self.core,
                world: &self.sim,
            };
            if catch_unwind(AssertUnwindSafe(|| b.after_tick(&view))).is_err() {
                log::error!(
                    "[{name}] behaviour {} panicked in after_tick: stopped",
                    b.name()
                );
                self.stop(b, "panic");
            }
        }
    }
}

/// Arm swings (main or off hand) in a packet list.
fn count_swings(out: &[Outbound]) -> u64 {
    out.iter()
        .filter(|o| matches!(o, Outbound::Swing | Outbound::SwingOffHand))
        .count() as u64
}

/// A read-only view after the movement step ([`Behavior::after_tick`]).
pub struct TickView<'a> {
    core: &'a Core,
    world: &'a dyn WorldView,
}

impl std::fmt::Debug for TickView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TickView")
            .field("tick", &self.core.tick)
            .field("sent", &self.core.sent)
            .finish_non_exhaustive()
    }
}

impl TickView<'_> {
    /// Ticks since the session started.
    #[must_use]
    pub fn tick(&self) -> u64 {
        self.core.tick
    }

    /// The protocol version.
    #[must_use]
    pub fn version(&self) -> GameVersion {
        self.core.opts.version
    }

    /// The client pose after this tick's movement step.
    #[must_use]
    pub fn pose(&self) -> &Pose {
        &self.core.pose
    }

    /// What the server has after this tick's movement packet (position, look, flags).
    #[must_use]
    pub fn sent(&self) -> &Pose {
        &self.core.sent
    }

    /// The world.
    #[must_use]
    pub fn world(&self) -> &dyn WorldView {
        self.world
    }

    /// The tracked entities (with [`Interests::ENTITIES`]).
    #[must_use]
    pub fn entities(&self) -> Option<&ghast_entity::Entities> {
        self.world.entities()
    }

    /// The inventory and the open window.
    #[must_use]
    pub fn inventory(&self) -> &ghast_inventory::Inventory {
        &self.core.inv
    }

    /// What the behaviour saw this tick (the events of [`TickContext::events`]).
    #[must_use]
    pub fn events(&self) -> &TickEvents {
        &self.core.current
    }

    /// Our entity id.
    #[must_use]
    pub fn entity_id(&self) -> i32 {
        self.core.entity_id
    }

    /// Health, food and saturation.
    #[must_use]
    pub fn health(&self) -> Health {
        self.core.health
    }

    /// Game mode.
    #[must_use]
    pub fn game_mode(&self) -> GameMode {
        self.core.game_mode
    }

    /// The selected hotbar slot.
    #[must_use]
    pub fn held_slot(&self) -> u8 {
        self.core.held_slot
    }

    /// The block being broken and its progress 0 … 1.
    #[must_use]
    pub fn mining(&self) -> Option<(ghast_protocol::BlockPos, f32)> {
        self.core.hands.mining()
    }

    /// Arm swings sent so far (see [`TickContext::swings`]).
    #[must_use]
    pub fn swings(&self) -> u64 {
        self.core.swings
    }

    /// Blocks broken so far.
    #[must_use]
    pub fn broken(&self) -> u64 {
        self.core.hands.broken
    }

    /// The current dimension (`"0"` / `"-1"` / `"1"` on 1.8, the world name on 1.16), as
    /// [`TickContext::dimension`].
    #[must_use]
    pub fn dimension(&self) -> &str {
        &self.core.dimension
    }

    /// Using the held item (eating, blocking, drawing a bow): movement input is slowed. As
    /// [`TickContext::using_item`].
    #[must_use]
    pub fn using_item(&self) -> bool {
        self.core.hands.using_item()
    }
}

#[cfg(test)]
impl Core {
    /// Test hook: the held slot as both the client and the server have it (no sync packet).
    pub(crate) fn test_set_slot(&mut self, slot: u8) {
        self.held_slot = slot;
        self.sent_slot = slot;
        self.inv.select(slot);
    }

    /// Test hook: the pose and the sent pose.
    pub(crate) fn test_poses(&mut self) -> (&mut Pose, &mut Pose) {
        (&mut self.pose, &mut self.sent)
    }

    /// Test hook: turns held back by the post-look check.
    pub(crate) fn test_held_turns(&self) -> u64 {
        self.held_turns
    }
}

#[cfg(test)]
impl TickContext<'_> {
    /// Test hook: this tick's look without a rotation step.
    pub(crate) fn test_force_look(&mut self, yaw: f32, pitch: f32) {
        self.core.forced_look = Some((yaw, pitch));
    }

    /// Test hook: the rotation controller (to predict a step).
    pub(crate) fn test_rotation(&self) -> Rotation {
        self.core.rotation
    }
}

/// 1.8 `SeedItem.use` / `CropItem.use` on the client: planting on the top of
/// its soil with air above uses one item at once (the server's set_slot follows later; it
/// sends none when the count already agrees). Applied to the `block_place` packets written
/// from `first` on. Other placements wait for the server's set_slot: predicting them needs
/// vanilla's full placement rules, and a wrong prediction would never be corrected.
fn predict_planting(c: &mut Core, world: &dyn WorldView, first: usize) {
    if c.opts.version != GameVersion::V1_8_9 || c.game_mode == GameMode::Creative {
        return;
    }
    let mut uses = 0u8;
    for o in &c.out[first..] {
        let Outbound::Place {
            pos,
            face: crate::common::Face::Up,
            held: Some(held),
            off_hand: false,
            ..
        } = o
        else {
            continue;
        };
        // Seed item → its soil (wheat, pumpkin, melon seeds, carrot, potato: farmland;
        // nether wart: soul sand).
        let soil = match held.id {
            295 | 361 | 362 | 391 | 392 => 60,
            372 => 88,
            _ => continue,
        };
        let id = |y: i32| world.raw_block(pos.x, y, pos.z).map(|s| s >> 4);
        if held.count > i32::from(uses) && id(pos.y) == Some(soil) && id(pos.y + 1) == Some(0) {
            uses += 1;
        }
    }
    if uses > 0 {
        c.inv.consume_held(uses);
    }
}

/// Whether the look `yaw`/`pitch` still points at every block dug or placed this tick (its
/// full cube, from the sent and the current feet, standing or sneaking eyes, with the old or the
/// new yaw) — the post-look check that keeps Grim's RotationPlace check satisfied.
fn post_look_ok(c: &Core, yaw: f32, pitch: f32) -> bool {
    let looks = [
        hands::look_vector(c.pose.yaw, pitch),
        hands::look_vector(yaw, pitch),
    ];
    let heights = [
        c.opts.version.eye_height(),
        c.opts.version.sneak_eye_height(),
    ];
    c.out
        .iter()
        .filter_map(|o| match o {
            Outbound::Dig {
                status: crate::common::DigStatus::Start | crate::common::DigStatus::Finish,
                pos,
                ..
            }
            | Outbound::Place { pos, .. } => Some(*pos),
            _ => None,
        })
        .all(|t| {
            [c.sent.pos, c.pose.pos].iter().all(|o| {
                heights.iter().any(|h| {
                    let eyes = Vec3::new(o.x, o.y + *h, o.z);
                    looks
                        .iter()
                        .any(|l| hands::ray_hits_cube(eyes, *l, t, ACTION_LOOK_REACH))
                })
            })
        })
}

/// Reach of the post-look check (6 blocks).
const ACTION_LOOK_REACH: f64 = 6.0;

/// The behaviour's view of the client during one tick.
///
/// Reading is free; actions are written to the server in call order before this tick's
/// movement packet.
pub struct TickContext<'a> {
    core: &'a mut Core,
    world: &'a dyn WorldView,
    held: Option<hands::HeldItem>,
}

impl std::fmt::Debug for TickContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TickContext")
            .field("tick", &self.core.tick)
            .field("pose", &self.core.pose)
            .finish_non_exhaustive()
    }
}

impl TickContext<'_> {
    /// Ticks since the connection entered the play state.
    #[must_use]
    pub fn tick(&self) -> u64 {
        self.core.tick
    }

    /// The protocol version of this connection.
    #[must_use]
    pub fn version(&self) -> GameVersion {
        self.core.opts.version
    }

    /// The client pose now (start of the tick, before this tick's rotation and movement).
    #[must_use]
    pub fn pose(&self) -> &Pose {
        &self.core.pose
    }

    /// The last pose written to the server: what it judges this tick's actions against.
    #[must_use]
    pub fn sent(&self) -> &Pose {
        &self.core.sent
    }

    /// Eye position of `pose` (standing or sneaking eye height of this version).
    #[must_use]
    pub fn eyes(&self, pose: &Pose) -> Vec3 {
        let v = self.core.opts.version;
        pose.eyes(if pose.sneaking {
            v.sneak_eye_height()
        } else {
            v.eye_height()
        })
    }

    /// The world (loaded chunk columns and their blocks).
    #[must_use]
    pub fn world(&self) -> &dyn WorldView {
        self.world
    }

    /// What happened since the previous tick.
    #[must_use]
    pub fn events(&self) -> &TickEvents {
        &self.core.current
    }

    /// Our entity id.
    #[must_use]
    pub fn entity_id(&self) -> i32 {
        self.core.entity_id
    }

    /// Arm swings (main and off hand) written so far in the session: every `arm_animation`,
    /// whether from [`TickContext::swing`], an action (place, dig, attack) or the held buttons
    /// (digging swings every tick). Includes this tick's written so far.
    #[must_use]
    pub fn swings(&self) -> u64 {
        self.core.swings + count_swings(&self.core.out)
    }

    /// Health, food and saturation.
    #[must_use]
    pub fn health(&self) -> Health {
        self.core.health
    }

    /// The player is dead (waiting for [`TickContext::respawn`]).
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.core.dead
    }

    /// Game mode.
    #[must_use]
    pub fn game_mode(&self) -> GameMode {
        self.core.game_mode
    }

    /// Abilities as the server sent them.
    #[must_use]
    pub fn abilities(&self) -> Abilities {
        self.core.abilities
    }

    /// The server allows flight.
    #[must_use]
    pub fn can_fly(&self) -> bool {
        self.core.abilities.may_fly()
    }

    /// The selected hotbar slot (0 … 8).
    #[must_use]
    pub fn held_slot(&self) -> u8 {
        self.core.held_slot
    }

    /// Id of the open container window, if the server opened one.
    #[must_use]
    pub fn open_window(&self) -> Option<i32> {
        (self.core.open_window != 0).then_some(self.core.open_window)
    }

    /// The current dimension (`"0"` / `"-1"` / `"1"` on 1.8, the world name on 1.16).
    #[must_use]
    pub fn dimension(&self) -> &str {
        &self.core.dimension
    }

    /// The movement controls recorded for this tick.
    #[must_use]
    pub fn controls(&self) -> Controls {
        self.core.controls
    }

    /// The rotation target.
    #[must_use]
    pub fn aim(&self) -> (f32, f32) {
        self.core.rotation.target()
    }

    // ---- actions: written now, before this tick's movement packet ----

    /// Sends a chat line or command (truncated to the version's limit, 100 / 256 UTF-16
    /// units, like vanilla).
    pub fn chat(&mut self, text: impl Into<String>) {
        self.core.out.push(Outbound::Chat(text.into()));
    }

    /// Asks the server to respawn the player. Only sent while dead; returns whether it was.
    pub fn respawn(&mut self) -> bool {
        if self.core.dead {
            self.core.out.push(Outbound::Respawn);
            true
        } else {
            false
        }
    }

    /// Selects a hotbar slot (0 … 8); sends `held_item_slot` when it changes. Out-of-range
    /// slots are ignored.
    pub fn select_slot(&mut self, slot: u8) {
        if slot < 9 {
            if self.core.held_slot != slot {
                self.core.hands.stop_using();
                self.core.hands.ticks_since_swing = 0;
            }
            self.core.held_slot = slot;
            self.core.inv.select(slot);
            if self.core.sent_slot != slot {
                self.core.sent_slot = slot;
                self.core.out.push(Outbound::HeldSlot(i16::from(slot)));
            }
        }
    }

    /// Swings the arm (`arm_animation`).
    pub fn swing(&mut self) {
        self.core.out.push(Outbound::Swing);
    }

    /// Ends the session after this tick's packets are written.
    pub fn disconnect(&mut self, reason: impl Into<String>) {
        self.core.quit = Some(reason.into());
    }

    // ---- intent for this tick's movement step ----

    /// Rotation target for this tick: the client turns toward it in whole mouse counts
    /// (at most 14° yaw / 10.5° pitch per tick). Call it every tick you want to turn.
    pub fn set_aim(&mut self, yaw: f32, pitch: f32) {
        self.core.rotation.set_target(yaw, pitch);
        self.core.aimed = true;
    }

    /// Movement keys from now on (kept until changed); the physics turns them into movement
    /// like vanilla's `Input` (sneaking scales walking by 0.3).
    pub fn set_controls(&mut self, controls: Controls) {
        self.core.controls = controls;
    }

    /// The inventory: player slots, cursor, the open window, the pending click.
    #[must_use]
    pub fn inventory(&self) -> &ghast_inventory::Inventory {
        &self.core.inv
    }

    /// Clicks in the open screen (`window_click`, written now). Refused (nothing sent) while
    /// the player is turning (the look changed in the last movement step or an aim is set for
    /// this tick: clicks only while still) or when the inventory refuses it (one click in
    /// flight, same-tick packet order, unpredictable result, …). The result arrives in
    /// [`TickEvents::inventory`] on a later tick.
    pub fn click(&mut self, click: ghast_inventory::Click) -> Result<ClickTicket, ClickRefused> {
        if self.core.turned || self.core.aimed {
            return Err(ClickRefused::Turning);
        }
        let o = self
            .core
            .inv
            .click(click)
            .map_err(ClickRefused::Inventory)?;
        let ticket = match &o {
            ghast_inventory::Outgoing::Click(p) => ClickTicket {
                window_id: i32::from(p.window_id),
                action: p.action,
            },
            _ => ClickTicket {
                window_id: -1,
                action: -1,
            },
        };
        self.core.out.push(Outbound::Inventory(o));
        Ok(ticket)
    }

    /// Opens the player inventory GUI (1.8: the "open inventory" client command; 1.16: nothing
    /// is sent). Window 0 clicks need it open.
    pub fn open_inventory(&mut self) -> Result<(), ClickRefused> {
        if let Some(o) = self
            .core
            .inv
            .open_inventory()
            .map_err(ClickRefused::Inventory)?
        {
            self.core.out.push(Outbound::Inventory(o));
        }
        Ok(())
    }

    /// Closes the open GUI (`close_window` with its id; 0 for the player inventory).
    pub fn close_window(&mut self) -> Result<(), ClickRefused> {
        let o = self.core.inv.close().map_err(ClickRefused::Inventory)?;
        self.core.out.push(Outbound::Inventory(o));
        // Vanilla closeHandledScreen: the container ends on the client too.
        self.core.open_window = 0;
        Ok(())
    }

    /// Mouse buttons from now on (kept until changed; released by a join, a respawn or an
    /// open window). Left held mines what the crosshair is on, right held uses / places
    /// (vanilla's key processing, after the behaviour and before the movement packet).
    pub fn set_buttons(&mut self, buttons: Buttons) {
        self.core.buttons = buttons;
    }

    /// The buttons held.
    #[must_use]
    pub fn buttons(&self) -> Buttons {
        self.core.buttons
    }

    /// What the crosshair is on as the server judges it: the first block within reach 4.5
    /// along the **sent** look from the sent eyes.
    #[must_use]
    pub fn target(&self) -> Option<BlockHit> {
        let s = &self.core.sent;
        let v = self.core.opts.version;
        let eye = if s.sneaking {
            v.sneak_eye_height()
        } else {
            v.eye_height()
        };
        let world = self.world;
        let blocks = |x: i32, y: i32, z: i32| world.block_query(x, y, z);
        hands::pick(
            &blocks,
            Vec3::new(s.pos.x, s.pos.y + eye, s.pos.z),
            hands::look_vector(s.yaw, s.pitch),
            hands::REACH,
        )
    }

    /// The tracked entities (behaviours that ask for [`Interests::ENTITIES`]).
    #[must_use]
    pub fn entities(&self) -> Option<&ghast_entity::Entities> {
        self.world.entities()
    }

    /// The entity the crosshair is on as the server judges it (from the sent pose; vanilla's
    /// entity pick: reach 3, 1.8 boxes grown by 0.1, closer than the block hit).
    #[must_use]
    pub fn target_entity(&self) -> Option<hands::EntityHit> {
        let s = &self.core.sent;
        let v = self.core.opts.version;
        let eye = if s.sneaking {
            v.sneak_eye_height()
        } else {
            v.eye_height()
        };
        let eyes = Vec3::new(s.pos.x, s.pos.y + eye, s.pos.z);
        let look = hands::look_vector(s.yaw, s.pitch);
        let block = self.target().map(|t| t.distance);
        entity_pick(self.world, v, eyes, look, block)
    }

    /// Attacks `entity` (`use_entity` attack and the arm swing in the version's order; 1.16
    /// resets the attack cooldown). Refused (`false`, nothing sent) unless the sent crosshair is
    /// on that entity.
    pub fn attack(&mut self, entity: i32) -> bool {
        if self.target_entity().is_none_or(|e| e.id != entity) {
            return false;
        }
        let world = self.world;
        let blocks = |x: i32, y: i32, z: i32| world.block_query(x, y, z);
        let c = &mut *self.core;
        let modern = c.opts.version != GameVersion::V1_8_9;
        let mut placed = false;
        let mut io = HandsIo {
            blocks: &blocks,
            target: None,
            held: self.held,
            held_stack: None,
            food: c.health.food,
            modern,
            off_held: None,
            off_stack: None,
            entity: None,
            factors: DigFactors::default(),
            sneaking: c.pose.sneaking,
            out: &mut c.out,
            broken: &mut c.broken_now,
            placed: &mut placed,
        };
        if !modern {
            io.out.push(Outbound::Swing);
        }
        c.hands.attack(entity, &mut io);
        if modern {
            c.out.push(Outbound::Swing);
        }
        true
    }

    /// 1.16 attack strength 0 … 1 for an attack speed (4.0 for an empty hand, swords 1.6,
    /// axes 0.8–1.0); 1.0 on 1.8 (no cooldown).
    #[must_use]
    pub fn attack_strength(&self, attack_speed: f64) -> f32 {
        if self.core.opts.version == GameVersion::V1_8_9 {
            1.0
        } else {
            self.core.hands.attack_strength(attack_speed)
        }
    }

    /// The block being broken with the left button and its progress 0 … 1.
    #[must_use]
    pub fn mining(&self) -> Option<(ghast_protocol::BlockPos, f32)> {
        self.core.hands.mining()
    }

    /// Blocks broken so far (finish sent).
    #[must_use]
    pub fn broken(&self) -> u64 {
        self.core.hands.broken
    }

    /// Using the held item (eating, blocking, drawing a bow): movement input is slowed.
    #[must_use]
    pub fn using_item(&self) -> bool {
        self.core.hands.using_item()
    }

    /// The version facts of an item stack (tool speeds, block item, food, usable), as the
    /// hands use them for the held stack. `None` for ids outside the item id range.
    ///
    /// ```no_run
    /// # fn f(ctx: &ghast_client::TickContext<'_>) {
    /// if let Some(s) = ctx.inventory().hotbar(0) {
    ///     let food = ctx.item_facts(s).is_some_and(|f| f.food);
    /// }
    /// # }
    /// ```
    #[must_use]
    pub fn item_facts(&self, stack: &ghast_inventory::Stack) -> Option<hands::HeldItem> {
        let id = u16::try_from(stack.id).ok()?;
        let count = u8::try_from(stack.count.clamp(0, 255)).unwrap_or(0);
        Some(crate::sim::held_item_facts(
            self.core.opts.version,
            id,
            count,
            efficiency(stack),
        ))
    }

    /// Break progress per tick (vanilla `calcBlockBreakingData`) of the block at `pos` dug
    /// with `stack` (`None` = empty hand), with the factors the left button uses now: on
    /// ground, eyes in water, aqua affinity, haste / mining fatigue. The block breaks once the
    /// progress sums to 1, so it takes about `ceil(1 / speed)` ticks; 0 = unbreakable.
    /// Creative mode breaks every block at once and is not reflected here. `None` when the
    /// column is not loaded.
    ///
    /// ```no_run
    /// # fn f(ctx: &ghast_client::TickContext<'_>, pos: ghast_client::BlockPos) {
    /// // The hotbar slot that digs `pos` fastest.
    /// let best = (0..9u8).max_by(|a, b| {
    ///     let s = |k: u8| ctx.dig_speed(pos, ctx.inventory().hotbar(k.into())).unwrap_or(0.0);
    ///     s(*a).total_cmp(&s(*b))
    /// });
    /// # }
    /// ```
    #[must_use]
    pub fn dig_speed(
        &self,
        pos: ghast_protocol::BlockPos,
        stack: Option<&ghast_inventory::Stack>,
    ) -> Option<f32> {
        let b = self.world.block_query(pos.x, pos.y, pos.z)?;
        if b.air {
            return Some(0.0);
        }
        let c = &*self.core;
        let eyes = self.eyes(&c.pose);
        let in_water = self
            .world
            .block_query(
                eyes.x.floor() as i32,
                eyes.y.floor() as i32,
                eyes.z.floor() as i32,
            )
            .is_some_and(|b| b.water);
        let held = stack.and_then(|s| self.item_facts(s));
        let factors = DigFactors {
            on_ground: c.pose.on_ground,
            in_water,
            aqua_affinity: c.aqua_affinity,
            haste: c.effects.get(&3).map_or(0, |a| (*a as u8).wrapping_add(1)),
            fatigue: c.effects.get(&4).map_or(0, |a| (*a as u8).wrapping_add(1)),
        };
        Some(hands::relative_hardness(&b, held.as_ref(), factors))
    }

    /// Starts digging `pos` on `face` (`block_dig` start, then an arm swing), breaking it at
    /// once when it is instant for the held item. Refused (nothing sent, `false`) unless the
    /// crosshair from the sent pose is on exactly that block face.
    pub fn dig(&mut self, pos: ghast_protocol::BlockPos, face: crate::common::Face) -> bool {
        if !self
            .target()
            .is_some_and(|t| t.pos == pos && t.face == face)
        {
            return false;
        }
        let world = self.world;
        let blocks = |x: i32, y: i32, z: i32| world.block_query(x, y, z);
        let c = &mut *self.core;
        let mut placed = false;
        let mut io = HandsIo {
            blocks: &blocks,
            target: None,
            held: self.held,
            held_stack: c.inv.held().cloned(),
            food: c.health.food,
            modern: c.opts.version != GameVersion::V1_8_9,
            off_held: None,
            off_stack: None,
            entity: None,
            factors: DigFactors {
                on_ground: c.pose.on_ground,
                in_water: false,
                aqua_affinity: c.aqua_affinity,
                haste: c.effects.get(&3).map_or(0, |a| (*a as u8).wrapping_add(1)),
                fatigue: c.effects.get(&4).map_or(0, |a| (*a as u8).wrapping_add(1)),
            },
            sneaking: c.pose.sneaking,
            out: &mut c.out,
            broken: &mut c.broken_now,
            placed: &mut placed,
        };
        c.hands.click_block(pos, face, &mut io);
        c.out.push(Outbound::Swing);
        true
    }

    /// Right-clicks `support`'s `face` at the absolute point `hit` (`block_place`, then an arm
    /// swing): places the held block or activates the block. Refused (nothing sent, `false`)
    /// unless the crosshair from the sent pose is on exactly that block face.
    pub fn place(
        &mut self,
        support: ghast_protocol::BlockPos,
        face: crate::common::Face,
        hit: Vec3,
    ) -> bool {
        let Some(t) = self.target().filter(|t| t.pos == support && t.face == face) else {
            return false;
        };
        let world = self.world;
        let Some(b) = world.block_query(support.x, support.y, support.z) else {
            return false;
        };
        let blocks = |x: i32, y: i32, z: i32| world.block_query(x, y, z);
        let c = &mut *self.core;
        let mut placed = false;
        let first = c.out.len();
        let mut io = HandsIo {
            blocks: &blocks,
            target: None,
            held: self.held,
            held_stack: c.inv.held().cloned(),
            food: c.health.food,
            modern: c.opts.version != GameVersion::V1_8_9,
            off_held: None,
            off_stack: None,
            entity: None,
            factors: DigFactors::default(),
            sneaking: c.pose.sneaking,
            out: &mut c.out,
            broken: &mut c.broken_now,
            placed: &mut placed,
        };
        c.hands
            .right_click_block(BlockHit { hit, ..t }, &b, &mut io);
        let _ = placed;
        predict_planting(c, world, first);
        c.out.push(Outbound::Swing);
        true
    }

    /// When the last inbound bytes arrived (a gap before a setback means lag).
    #[must_use]
    pub fn last_inbound(&self) -> Option<Instant> {
        self.core.last_inbound
    }

    /// The yaw / pitch this tick's movement step will use, from this tick's
    /// [`TickContext::set_aim`] and the digs / places written so far (incl. the turn held by
    /// [`crate::Client::hold_turn_on_action`]). Computed on a copy: the rotation's carried
    /// budget does not change. Clicks of held buttons, which run after the behaviour, are not
    /// seen. Without an aim this tick: the current look.
    #[must_use]
    pub fn planned_look(&self) -> (f32, f32) {
        let c = &*self.core;
        let now = (c.pose.yaw, c.pose.pitch);
        #[cfg(test)]
        if let Some(l) = c.forced_look {
            return l;
        }
        if !c.aimed {
            return now;
        }
        let mut r = c.rotation;
        let (yaw, pitch) = r.step(c.pose.yaw, c.pose.pitch);
        if c.opts.hold_turn_on_action && !post_look_ok(c, yaw, pitch) {
            now
        } else {
            (yaw, pitch)
        }
    }

    /// Starts or stops creative-style flight: the `abilities` packet goes out before this
    /// tick's movement step, like a double-tapped jump (but without the jump). Starting needs
    /// the server's "may fly" ability; asking for the current state sends nothing.
    pub fn set_flying(&mut self, flying: bool) {
        self.core.want_flying = Some(flying);
    }

    /// Turns held back so far because they would have taken the crosshair off a block dug or
    /// placed on the same tick (see [`crate::Client::hold_turn_on_action`]).
    #[must_use]
    pub fn held_turns(&self) -> u64 {
        self.core.held_turns
    }

    /// Behaviour status line (logged when it changes).
    pub fn status(&mut self, text: &str) {
        if self.core.status != text {
            text.clone_into(&mut self.core.status);
            log::info!(
                "[{}] t{} status: {text}",
                self.core.opts.name,
                self.core.tick
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Teleport;
    use crate::common::{ChatPosition, EntityAction, JoinGame};
    use ghast_protocol::RawPacket;

    fn opts(version: GameVersion) -> CoreOptions {
        CoreOptions {
            name: "test".into(),
            version,
            settings: ClientSettings::default(),
            brand: Some("vanilla".into()),
            sensitivity: 0.5,
            silent_hold: Duration::from_millis(1500),
            hold_turn_on_action: true,
            inventory: match version {
                GameVersion::V1_8_9 => ghast_inventory::Version::V1_8,
                _ => ghast_inventory::Version::V1_16_5,
            },
        }
    }

    /// A 1.8.9 engine (physics) or a 1.16.5 one (stand-still mover).
    fn engine(version: GameVersion) -> Engine<Box<dyn Sim>> {
        let sim: Box<dyn Sim> = match version {
            GameVersion::V1_8_9 => Box::new(crate::sim::Sim18::new()),
            _ => Box::new(crate::sim::Sim16::new()),
        };
        Engine::new(opts(version), sim)
    }

    /// Loads the column of chunk (x, z) the way the connection does.
    fn load(e: &mut Engine<Box<dyn Sim>>, x: i32, z: i32) {
        let raw = match e.core.opts.version {
            GameVersion::V1_8_9 => crate::sim::test_chunks::v1_8(x, z),
            _ => crate::sim::test_chunks::v1_16_5(x, z),
        };
        e.sim.world_packet(&raw).unwrap();
    }

    fn join(e: &mut Engine<Box<dyn Sim>>) {
        e.handle(Inbound::JoinGame(JoinGame {
            entity_id: 7,
            game_mode: GameMode::Survival,
            dimension: "0".into(),
            hardcore: false,
            dimension_nbt: None,
            respawn_screen: true,
        }));
        load(e, 0, 0);
        e.handle(Inbound::Teleport(Teleport {
            x: 8.5,
            y: 64.0,
            z: 8.5,
            yaw: 90.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: None,
        }));
        e.outbound().clear();
    }

    /// Records the tick's packets as names.
    fn names(out: &mut Vec<Outbound>) -> Vec<&'static str> {
        let v = out
            .iter()
            .map(|o| match o {
                Outbound::Move(m) => m.trace_id(),
                Outbound::Chat(_) => "chat",
                Outbound::HeldSlot(_) => "held_slot",
                Outbound::Swing => "swing",
                Outbound::EntityAction { .. } => "entity_action",
                Outbound::KeepAlive(_) => "keep_alive",
                Outbound::Transaction { .. } => "transaction",
                Outbound::TeleportConfirm(_) => "teleport_confirm",
                Outbound::Respawn => "respawn",
                Outbound::Settings(_) => "settings",
                Outbound::Brand(_) => "brand",
                Outbound::Abilities { .. } => "abilities",
                Outbound::Dig { .. } => "block_dig",
                Outbound::Place { .. } | Outbound::UseItem { .. } => "block_place",
                Outbound::SwingOffHand => "swing_off",
                Outbound::UseEntity { .. } => "use_entity",
                Outbound::Inventory(_) => "inventory",
            })
            .collect();
        out.clear();
        v
    }

    struct Script<F: FnMut(&mut TickContext<'_>)>(F);
    impl<F: FnMut(&mut TickContext<'_>)> Behavior for Script<F> {
        fn tick(&mut self, ctx: &mut TickContext<'_>) {
            (self.0)(ctx);
        }
    }

    #[test]
    fn join_sends_settings_then_brand() {
        let mut e = engine(GameVersion::V1_8_9);
        e.handle(Inbound::JoinGame(JoinGame {
            entity_id: 1,
            game_mode: GameMode::Creative,
            dimension: "0".into(),
            hardcore: false,
            dimension_nbt: None,
            respawn_screen: true,
        }));
        assert_eq!(names(e.outbound()), ["settings", "brand"]);
    }

    #[test]
    fn nothing_before_the_first_teleport() {
        let mut e = engine(GameVersion::V1_8_9);
        let mut b = Idle;
        let t0 = Instant::now();
        load(&mut e, 0, 0);
        e.outbound().clear();
        for i in 0..5 {
            e.tick(&mut b, t0 + Duration::from_millis(50 * i));
        }
        assert!(e.outbound().is_empty());
    }

    #[test]
    fn teleport_reply_per_version() {
        let mut e = engine(GameVersion::V1_16_5);
        e.handle(Inbound::Teleport(Teleport {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            yaw: 450.0,
            pitch: 100.0,
            flags: 0,
            teleport_id: Some(5),
        }));
        let out = std::mem::take(e.outbound());
        assert_eq!(out[0], Outbound::TeleportConfirm(5));
        match out[1] {
            Outbound::Move(MovePacket::PositionLook {
                yaw,
                pitch,
                on_ground,
                ..
            }) => {
                assert_eq!((yaw, pitch, on_ground), (90.0, 90.0, false));
            }
            ref o => panic!("unexpected {o:?}"),
        }
        let mut e = engine(GameVersion::V1_8_9);
        e.handle(Inbound::Teleport(Teleport {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            yaw: 450.0,
            pitch: 100.0,
            flags: 0,
            teleport_id: None,
        }));
        let out = std::mem::take(e.outbound());
        assert_eq!(out.len(), 1);
        // 1.8 does not clamp the pitch.
        assert!(matches!(
            out[0],
            Outbound::Move(MovePacket::PositionLook {
                yaw: 90.0,
                pitch: 100.0,
                ..
            })
        ));
    }

    #[test]
    fn actions_precede_the_single_movement_packet() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            if ctx.tick() == 2 {
                ctx.select_slot(3);
                ctx.chat("hello");
                ctx.swing();
                ctx.set_controls(Controls {
                    sneak: true,
                    ..Controls::default()
                });
            }
        });
        e.tick(&mut b, t0);
        assert_eq!(names(e.outbound()), ["C06"]);
        e.tick(&mut b, t0);
        assert_eq!(
            names(e.outbound()),
            ["held_slot", "chat", "swing", "entity_action", "C03"]
        );
        for _ in 0..100 {
            e.tick(&mut b, t0);
            let n = names(e.outbound());
            assert_eq!(n.len(), 1, "one movement packet per tick: {n:?}");
        }
    }

    #[test]
    fn standing_bot_packets() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let mut b = Idle;
        let t0 = Instant::now();
        e.note_inbound(t0);
        // First tick: the position differs from the fresh player's lastReported (origin).
        e.tick(&mut b, t0);
        let out = std::mem::take(e.outbound());
        assert!(matches!(
            out[..],
            [Outbound::Move(MovePacket::PositionLook {
                on_ground: false,
                ..
            })]
        ));
        // Then flying (on ground) with a position refresh every 21st tick.
        let mut kinds = Vec::new();
        for _ in 0..42 {
            e.tick(&mut b, t0);
            kinds.extend(names(e.outbound()));
        }
        assert_eq!(kinds.iter().filter(|k| **k == "C04").count(), 2);
        assert_eq!(kinds.iter().filter(|k| **k == "C03").count(), 40);
        assert_eq!((kinds[20], kinds[41]), ("C04", "C04"));
        assert!(e.core.sent.on_ground);
    }

    #[test]
    fn silent_server_freezes_behaviour() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        let mut ticks = 0;
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            ticks += 1;
            ctx.set_controls(Controls {
                sneak: true,
                ..Controls::default()
            });
        });
        e.tick(&mut b, t0 + Duration::from_millis(100));
        e.tick(&mut b, t0 + Duration::from_millis(1600));
        // Frozen: keys released, so the sneak stops; still one movement packet.
        let n = names(e.outbound());
        assert_eq!(n, ["entity_action", "C06", "entity_action", "C03"]);
        e.note_inbound(t0 + Duration::from_millis(1700));
        e.tick(&mut b, t0 + Duration::from_millis(1700));
        let _ = b;
        assert_eq!(ticks, 2);
    }

    #[test]
    fn keepalive_answered_transactions_left_to_the_inventory() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        e.handle(Inbound::KeepAlive(42));
        // The engine never answers confirms: the inventory does (exactly once), see the
        // fake-server test.
        e.handle(Inbound::Transaction {
            window_id: 0,
            action: -5,
            accepted: false,
        });
        assert_eq!(std::mem::take(e.outbound()), [Outbound::KeepAlive(42)]);
    }

    #[test]
    fn panicking_behaviour_does_not_stop_the_tick() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let t0 = Instant::now();
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            if ctx.tick() == 2 {
                panic!("boom");
            }
        });
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        for _ in 0..5 {
            e.tick(&mut b, t0);
        }
        std::panic::set_hook(hook);
        assert!(e.state.failed);
        assert_eq!(names(e.outbound()).len(), 5);
    }

    #[test]
    fn chat_events_reach_the_behaviour_once() {
        let mut e = engine(GameVersion::V1_16_5);
        join(&mut e);
        e.handle(Inbound::Chat(ChatMessage {
            json: r#"{"text":"hi"}"#.into(),
            position: ChatPosition::System,
            sender: None,
        }));
        let mut seen = Vec::new();
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            seen.extend(ctx.events().chat.iter().map(ChatMessage::plain));
        });
        let t0 = Instant::now();
        e.tick(&mut b, t0);
        e.tick(&mut b, t0);
        let _ = b;
        assert_eq!(seen, ["hi"]);
    }

    #[test]
    fn immediate_respawn_1_16() {
        let mut e = engine(GameVersion::V1_16_5);
        join(&mut e);
        e.handle(Inbound::GameState {
            reason: 11,
            value: 1.0,
        });
        e.handle(Inbound::Died { entity_id: 7 });
        assert_eq!(e.outbound().as_slice(), [Outbound::Respawn]);
    }

    /// Left button on the floor: START, swings, FINISH, all before the tick's movement
    /// packet; the broken block is predicted air and the player falls into the hole.
    #[test]
    fn dig_with_the_left_button() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            ctx.set_aim(90.0, 89.5);
            let down = ctx.target().is_some_and(|t| {
                t.pos == ghast_protocol::BlockPos::new(8, 63, 8) && t.face == crate::Face::Up
            });
            ctx.set_buttons(Buttons {
                left: down,
                right: false,
            });
        });
        let mut ticks = Vec::new();
        for _ in 0..200 {
            e.tick(&mut b, t0);
            let n = names(e.outbound());
            assert_eq!(n.iter().filter(|x| x.starts_with('C')).count(), 1, "{n:?}");
            assert!(n.last().unwrap().starts_with('C'), "movement last: {n:?}");
            ticks.push(n);
        }
        let flat: Vec<&str> = ticks.iter().flatten().copied().collect();
        assert_eq!(
            flat.iter().filter(|x| **x == "block_dig").count(),
            2,
            "start + finish"
        );
        assert!(e.core.hands.broken == 1);
        // Stone by hand: not harvestable, 1 / 1.5 / 100 per tick -> 151 damage ticks.
        let start = ticks.iter().position(|t| t.contains(&"block_dig")).unwrap();
        let finish = ticks
            .iter()
            .rposition(|t| t.contains(&"block_dig"))
            .unwrap();
        assert_eq!(finish - start, 150);
        assert!(e.sim.block_query(8, 63, 8).unwrap().air);
        assert!(
            e.core.pose.pos.y < 64.0,
            "falls into the hole: {:?}",
            e.core.pose.pos
        );
    }

    /// `dig_speed` is the left button's per-tick progress; `item_facts` the hands' facts.
    #[test]
    fn dig_speed_and_item_facts() {
        // 1.16.5 facts by name (its test chunk is empty).
        let mut e = engine(GameVersion::V1_16_5);
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            let id = |n: &str| i32::from(ghast_data::v1_16_5::Item::from_name(n).unwrap().id());
            let bread = ctx.item_facts(&ghast_inventory::Stack::new(id("bread"), 3, 0));
            assert!(bread.is_some_and(|f| f.food && f.usable && !f.places_block && f.count == 3));
            let dirt = ctx.item_facts(&ghast_inventory::Stack::new(id("dirt"), 1, 0));
            assert!(dirt.is_some_and(|f| f.places_block && !f.food));
        });
        e.tick(&mut b, t0);

        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        e.note_inbound(t0);
        let got = std::cell::Cell::new(None);
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            let stone = ghast_protocol::BlockPos::new(8, 63, 8);
            let pick = ghast_inventory::Stack::new(278, 1, 0);
            let facts = ctx.item_facts(&pick).unwrap();
            assert!(!facts.places_block && !facts.food);
            got.set(Some((
                ctx.dig_speed(stone, None),
                ctx.dig_speed(stone, Some(&pick)),
                ctx.dig_speed(ghast_protocol::BlockPos::new(8, 64, 8), None),
                ctx.dig_speed(ghast_protocol::BlockPos::new(500, 63, 8), None),
            )));
        });
        // Land first (in the air the progress is ÷5).
        for _ in 0..40 {
            e.tick(&mut b, t0);
        }
        assert!(e.core.pose.on_ground, "{:?}", e.core.pose);
        let (hand, pick, air, unloaded) = got.get().expect("ticked");
        // Stone by hand: not harvestable, 1 / 1.5 / 100 per tick (the 150 ticks of
        // `dig_with_the_left_button`).
        assert_eq!(hand, Some(1.0 / 1.5 / 100.0));
        // Diamond pickaxe: speed 8, harvestable: 8 / 1.5 / 30.
        assert_eq!(pick, Some(8.0 / 1.5 / 30.0));
        assert_eq!(air, Some(0.0));
        assert_eq!(unloaded, None);
    }

    /// `place` is refused unless the sent look is on the face; then C08 + C0A before the
    /// movement packet.
    #[test]
    fn place_needs_the_sent_look() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        let floor = ghast_protocol::BlockPos::new(8, 63, 8);
        let mut results = Vec::new();
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            ctx.set_aim(90.0, 89.5);
            results.push(ctx.place(floor, crate::Face::Up, Vec3::new(8.5, 64.0, 8.5)));
        });
        // A stack of stone in the selected hotbar slot (window 0 slot 36).
        e.core
            .inv
            .on_set_slot(0, 36, Some(ghast_inventory::Stack::new(1, 64, 0)));
        let mut placed_tick = None;
        for i in 0..20 {
            e.tick(&mut b, t0);
            let n = names(e.outbound());
            if n.contains(&"block_place") {
                assert_eq!(n[..2], ["block_place", "swing"], "{n:?}");
                placed_tick.get_or_insert(i);
            }
        }
        let _ = b;
        let first = results.iter().position(|r| *r).unwrap();
        assert!(first > 3, "refused while still turning");
        assert_eq!(placed_tick, Some(first));
        assert_eq!(e.core.inv.held().map(|s| s.id), Some(1));
    }

    #[test]
    fn sneak_action_order() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            ctx.set_controls(Controls {
                sneak: true,
                ..Controls::default()
            });
        });
        e.tick(&mut b, Instant::now());
        let out = std::mem::take(e.outbound());
        assert_eq!(
            out[0],
            Outbound::EntityAction {
                entity_id: 7,
                action: EntityAction::StartSneaking
            }
        );
        assert!(matches!(out[1], Outbound::Move(_)));
    }

    /// Block changes reach the behaviour once, in arrival order, only when asked for.
    #[test]
    fn block_updates_per_version() {
        use ghast_protocol::BlockPos;
        for v in [GameVersion::V1_8_9, GameVersion::V1_16_5] {
            let mut e = engine(v);
            join(&mut e);
            e.sim.collect_block_updates(true);
            let t0 = Instant::now();
            e.note_inbound(t0);
            let raws = match v {
                GameVersion::V1_8_9 => {
                    use ghast_protocol::v1_8::play::clientbound as cb;
                    vec![
                        RawPacket::from_packet(&cb::BlockChange {
                            location: BlockPos::new(3, 63, 4),
                            r#type: 59 << 4 | 7,
                        })
                        .unwrap(),
                        RawPacket::from_packet(&cb::MultiBlockChange {
                            chunk_x: 0,
                            chunk_z: 0,
                            records: vec![cb::MultiBlockChangeRecords {
                                horizontal_pos: 5 << 4 | 6,
                                y: 62,
                                block_id: 60 << 4 | 7,
                            }],
                        })
                        .unwrap(),
                    ]
                }
                _ => {
                    use ghast_protocol::v1_16_5::play::clientbound as cb;
                    // Section y = -1 (20-bit two's complement), local x 5 z 6 y 14.
                    let local = (5 << 8 | 6 << 4 | 14) as i64;
                    vec![
                        RawPacket::from_packet(&cb::BlockChange {
                            location: BlockPos::new(3, 63, 4),
                            r#type: 1,
                        })
                        .unwrap(),
                        RawPacket::from_packet(&cb::MultiBlockChange {
                            chunk_coordinates: cb::MultiBlockChangeChunkCoordinates {
                                x: 0,
                                z: 0,
                                y: 0xF_FFFF,
                            },
                            not_trust_edges: false,
                            records: vec![9 << 12 | local],
                        })
                        .unwrap(),
                    ]
                }
            };
            for raw in &raws {
                e.sim.world_packet(raw).unwrap();
            }
            let mut seen = Vec::new();
            let mut b = Script(|ctx: &mut TickContext<'_>| {
                seen.push(ctx.events().block_updates.clone());
            });
            e.tick(&mut b, t0);
            e.tick(&mut b, t0);
            let _ = b;
            let want = match v {
                GameVersion::V1_8_9 => vec![
                    (BlockPos::new(3, 63, 4), 59 << 4 | 7),
                    (BlockPos::new(5, 62, 6), 60 << 4 | 7),
                ],
                _ => vec![(BlockPos::new(3, 63, 4), 1), (BlockPos::new(5, -2, 6), 9)],
            };
            assert_eq!(seen, [want, Vec::new()], "{v:?}");

            // Not collected when not asked for.
            e.sim.collect_block_updates(false);
            e.sim.world_packet(&raws[0]).unwrap();
            let mut n = usize::MAX;
            let mut b = Script(|ctx: &mut TickContext<'_>| n = ctx.events().block_updates.len());
            e.tick(&mut b, t0);
            let _ = b;
            assert_eq!(n, 0);
        }
    }

    /// `set_flying` sends the abilities packet before the move, only when allowed and changed.
    #[test]
    fn set_flying_needs_may_fly() {
        for v in [GameVersion::V1_8_9, GameVersion::V1_16_5] {
            let mut e = engine(v);
            join(&mut e);
            let t0 = Instant::now();
            e.note_inbound(t0);
            // In the air (flight ends on landing, like vanilla).
            e.handle(Inbound::Teleport(Teleport {
                x: 8.5,
                y: 70.0,
                z: 8.5,
                yaw: 90.0,
                pitch: 0.0,
                flags: 0,
                teleport_id: Some(2),
            }));
            e.outbound().clear();
            let mut b = Script(|ctx: &mut TickContext<'_>| ctx.set_flying(true));
            e.tick(&mut b, t0);
            assert!(!names(e.outbound()).contains(&"abilities"), "{v:?}");
            assert!(!e.core.pose.flying);
            e.handle(Inbound::Abilities(Abilities {
                flags: 4,
                ..Abilities::default()
            }));
            e.tick(&mut b, t0);
            let out = std::mem::take(e.outbound());
            assert_eq!(
                out[0],
                Outbound::Abilities {
                    flags: 6,
                    flying_speed: 0.05,
                    walking_speed: 0.1
                },
                "{v:?}"
            );
            assert!(matches!(out[1], Outbound::Move(_)), "{out:?}");
            assert!(e.core.pose.flying && e.core.sent.flying);
            // Already flying: nothing more.
            e.tick(&mut b, t0);
            assert!(!names(e.outbound()).contains(&"abilities"));
            let _ = b;
        }
    }

    /// A behaviour that reports `running() == false` is not ticked again.
    #[test]
    fn finished_behaviour_is_not_ticked() {
        struct Three(std::rc::Rc<std::cell::Cell<u32>>);
        impl Behavior for Three {
            fn tick(&mut self, _ctx: &mut TickContext<'_>) {
                self.0.set(self.0.get() + 1);
            }
            fn running(&self) -> bool {
                self.0.get() < 3
            }
        }
        let ticks = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        let mut b: Box<dyn Behavior> = Box::new(Three(ticks.clone()));
        for _ in 0..10 {
            e.tick(&mut b, t0);
            // The player keeps sending its movement packet.
            let out = std::mem::take(e.outbound());
            assert!(
                out.iter().any(|o| matches!(o, Outbound::Move(_))),
                "{out:?}"
            );
        }
        assert_eq!(
            ticks.get(),
            3,
            "not ticked after running() == false (through the Box)"
        );
    }

    /// A turn that would take the crosshair off a block placed this tick waits one tick.
    #[test]
    fn turn_held_on_a_place_tick() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        e.core
            .inv
            .on_set_slot(0, 36, Some(ghast_inventory::Stack::new(1, 64, 0)));
        let floor = ghast_protocol::BlockPos::new(8, 63, 8);
        let mut tick = 0;
        let mut placed_at = None;
        let mut pitches = Vec::new();
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            tick += 1;
            if placed_at.is_none() {
                ctx.set_aim(90.0, 89.5);
                if ctx.place(floor, crate::Face::Up, Vec3::new(8.5, 64.0, 8.5)) {
                    placed_at = Some(tick);
                    // Look up on the same tick: held back.
                    ctx.set_aim(90.0, -60.0);
                }
            } else {
                ctx.set_aim(90.0, -60.0);
            }
            pitches.push(ctx.pose().pitch);
        });
        for _ in 0..30 {
            e.tick(&mut b, t0);
            e.outbound().clear();
        }
        let _ = b;
        let p = placed_at.expect("placed");
        assert_eq!(e.core.held_turns, 1);
        // Pose at the start of the tick after the place: still the place look.
        assert_eq!(pitches[p], pitches[p - 1], "{pitches:?}");
        assert!(pitches[p + 1] < pitches[p], "{pitches:?}");
    }

    /// `click` returns the ticket its confirm carries.
    #[test]
    fn click_ticket_matches_the_confirm() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        e.core
            .inv
            .on_set_slot(0, 36, Some(ghast_inventory::Stack::new(1, 64, 0)));
        let mut ticket = None;
        let mut n = 0;
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            n += 1;
            if n == 1 {
                ctx.open_inventory().unwrap();
            } else if ticket.is_none() {
                ticket = ctx
                    .click(ghast_inventory::Click::Pickup {
                        slot: 36,
                        right: false,
                    })
                    .ok();
            }
        });
        for _ in 0..3 {
            e.tick(&mut b, t0);
        }
        let _ = b;
        let t = ticket.expect("click sent");
        assert_eq!(t.window_id, 0);
        assert!(t.matches(&ghast_inventory::ClickResult {
            window_id: 0,
            action: t.action,
            outcome: ghast_inventory::ClickOutcome::Accepted,
        }));
    }

    /// 1.8 planting uses the seed at once (vanilla `SeedItem.use` on the client); a
    /// stone block place waits for the server.
    #[test]
    fn planting_uses_the_seed_at_once() {
        use ghast_protocol::v1_8::play::clientbound as cb;
        for (soil, item, used) in [
            (60 << 4 | 7, 295, true),
            (1 << 4, 295, false),
            (60 << 4, 1, false),
        ] {
            let mut e = engine(GameVersion::V1_8_9);
            join(&mut e);
            let t0 = Instant::now();
            e.note_inbound(t0);
            let floor = ghast_protocol::BlockPos::new(8, 63, 8);
            let raw = RawPacket::from_packet(&cb::BlockChange {
                location: floor,
                r#type: soil,
            })
            .unwrap();
            e.sim.world_packet(&raw).unwrap();
            e.core
                .inv
                .on_set_slot(0, 36, Some(ghast_inventory::Stack::new(item, 64, 0)));
            let mut done = false;
            let mut b = Script(|ctx: &mut TickContext<'_>| {
                ctx.set_aim(90.0, 89.5);
                if !done {
                    done = ctx.place(floor, crate::Face::Up, Vec3::new(8.5, 64.0, 8.5));
                }
            });
            let mut sent = None;
            for _ in 0..20 {
                e.tick(&mut b, t0);
                for o in e.outbound().drain(..) {
                    if let Outbound::Place { held, .. } = o {
                        sent = held.map(|h| h.count);
                    }
                }
            }
            let _ = b;
            assert_eq!(sent, Some(64), "the packet carries the stack before use");
            let left = e.core.inv.held().map(|s| s.count);
            assert_eq!(
                left,
                Some(if used { 63 } else { 64 }),
                "soil {soil} item {item}"
            );
        }
    }

    /// `planned_look` is the look the movement step then uses: a plain turn, a turn held on a
    /// place tick, no aim; and it leaves the rotation's budget alone.
    #[test]
    fn planned_look_is_the_next_look() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        e.core
            .inv
            .on_set_slot(0, 36, Some(ghast_inventory::Stack::new(1, 64, 0)));
        let floor = ghast_protocol::BlockPos::new(8, 63, 8);
        let mut n = 0;
        let mut placed = false;
        let mut plans: Vec<(f32, f32)> = Vec::new();
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            n += 1;
            if n % 5 == 0 {
                // No aim this tick: the current look.
            } else if !placed {
                ctx.set_aim(90.0, 89.5);
                if ctx.place(floor, crate::Face::Up, Vec3::new(8.5, 64.0, 8.5)) {
                    placed = true;
                    ctx.set_aim(90.0, -60.0);
                }
            } else {
                ctx.set_aim(150.0, -60.0);
            }
            // Asking twice changes nothing.
            let p = ctx.planned_look();
            assert_eq!(p, ctx.planned_look());
            plans.push(p);
        });
        let mut looks = Vec::new();
        for _ in 0..40 {
            e.tick(&mut b, t0);
            e.outbound().clear();
            looks.push((e.core.pose.yaw, e.core.pose.pitch));
        }
        let _ = b;
        assert!(placed);
        assert_eq!(e.core.test_held_turns(), 1);
        assert_eq!(plans, looks);
    }

    /// A teleport event keeps the sent pose from before the overwrite, the raw values and the
    /// resolved pose.
    #[test]
    fn teleport_event_has_the_sent_pose_before() {
        for v in [GameVersion::V1_8_9, GameVersion::V1_16_5] {
            let mut e = engine(v);
            join(&mut e);
            let t0 = Instant::now();
            e.note_inbound(t0);
            let mut seen = Vec::new();
            let mut b = Script(|ctx: &mut TickContext<'_>| {
                seen.extend(ctx.events().teleports.iter().copied());
            });
            e.tick(&mut b, t0);
            let sent = e.core.sent;
            // Relative yaw +30, absolute position.
            let tp = Teleport {
                x: 10.5,
                y: 64.0,
                z: 8.5,
                yaw: 30.0,
                pitch: 0.0,
                flags: crate::common::teleport_flags::YAW,
                teleport_id: Some(9),
            };
            e.handle(Inbound::Teleport(tp));
            e.tick(&mut b, t0);
            e.tick(&mut b, t0);
            let _ = b;
            // The spawn position first (join), then ours, each once.
            assert_eq!(seen.len(), 2, "{v:?}");
            let ev = seen[1];
            assert_eq!(ev.teleport, tp);
            assert_eq!(ev.sent_before, sent);
            assert_eq!(ev.after.pos, Vec3::new(10.5, 64.0, 8.5));
            assert_eq!(ev.after.yaw, sent.yaw + 30.0, "{v:?}");
        }
    }

    /// A container the server opened and the behaviour closed (`close_window`) no longer
    /// holds the mouse: the left button digs again (both versions). While it is open the
    /// buttons are up.
    #[test]
    fn closed_window_frees_the_buttons() {
        for v in [GameVersion::V1_8_9, GameVersion::V1_16_5] {
            let mut e = engine(v);
            join(&mut e);
            let t0 = Instant::now();
            e.note_inbound(t0);
            if v == GameVersion::V1_16_5 {
                // The 1.16 test chunk is all air: put a stone floor under the player.
                use ghast_protocol::v1_16_5::play::clientbound as cb;
                let raw = RawPacket::from_packet(&cb::BlockChange {
                    location: ghast_protocol::BlockPos::new(8, 63, 8),
                    r#type: 1,
                })
                .unwrap();
                e.sim.world_packet(&raw).unwrap();
            }
            e.core.inv.on_open_window(
                5,
                Some(ghast_inventory::WindowKind::Chest { rows: 3 }),
                "Vault".into(),
            );
            e.handle(Inbound::OpenWindow(5));
            // Something on the cursor: vanilla's client-side close drops it (the server
            // throws it out), so the client forgets it too.
            e.core
                .inv
                .on_set_slot(-1, -1, Some(ghast_inventory::Stack::new(1, 3, 0)));
            let mut n = 0;
            let mut closed_at = None;
            let mut b = Script(|ctx: &mut TickContext<'_>| {
                n += 1;
                ctx.set_aim(90.0, 89.5);
                ctx.set_buttons(Buttons {
                    left: true,
                    right: false,
                });
                if n == 15 {
                    assert!(ctx.inventory().cursor().is_some());
                    ctx.close_window().unwrap();
                    assert_eq!(ctx.open_window(), None);
                    let inv = ctx.inventory();
                    assert!(!inv.gui_open() && inv.window().is_none() && inv.cursor().is_none());
                    closed_at = Some(n);
                }
            });
            let mut first_dig = None;
            for i in 1..=30 {
                e.tick(&mut b, t0);
                let out = std::mem::take(e.outbound());
                if first_dig.is_none()
                    && out.iter().any(|o| {
                        matches!(
                            o,
                            Outbound::Dig {
                                status: crate::common::DigStatus::Start,
                                ..
                            }
                        )
                    })
                {
                    first_dig = Some(i);
                }
            }
            let _ = b;
            assert_eq!(closed_at, Some(15));
            let d = first_dig.unwrap_or_else(|| panic!("{v:?}: no dig after the close"));
            // The hands run after the behaviour: the closing tick already digs.
            assert!(d >= 15, "{v:?}: dug at tick {d} with the window open");
        }
    }

    /// `swings` counts every arm swing: the behaviour's, an action's and the held button's.
    #[test]
    fn swings_count_every_arm_swing() {
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        let mut n = 0;
        let mut seen = Vec::new();
        let mut b = Script(|ctx: &mut TickContext<'_>| {
            n += 1;
            ctx.set_aim(90.0, 89.5);
            if n == 1 {
                ctx.swing();
                ctx.swing();
            }
            if n == 20 {
                // Hold the left button on the stone floor: a swing every tick while digging.
                ctx.set_buttons(Buttons {
                    left: true,
                    right: false,
                });
            }
            seen.push(ctx.swings());
        });
        let mut sent = 0u64;
        for _ in 0..30 {
            e.tick(&mut b, t0);
            sent += count_swings(&std::mem::take(e.outbound()));
        }
        let _ = b;
        assert_eq!(seen[0], 2, "this tick's own swings count at once");
        assert!(sent > 10, "the held button swings: {sent}");
        // The last reading misses only the hands' swing of the last tick.
        assert_eq!(*seen.last().unwrap() + 1, sent);
        assert_eq!(e.core.swings, sent);
    }

    /// `WorldView::column` gives the stored column; light and biomes only when configured.
    #[test]
    fn column_view_and_world_config() {
        for config in [
            None,
            Some(
                ghast_world::WorldConfig::default()
                    .with_biomes(true)
                    .with_light(true),
            ),
        ] {
            let sim = crate::sim::Sim18::with(&crate::sim::SimOptions {
                world: config,
                ..crate::sim::SimOptions::default()
            });
            let mut e = Engine::new(opts(GameVersion::V1_8_9), Box::new(sim) as Box<dyn Sim>);
            join(&mut e);
            let w: &dyn WorldView = &e.sim;
            let col = w.column(0, 0).expect("loaded");
            assert_eq!(u32::from(col.raw(8, 63, 8)), w.raw_block(8, 63, 8).unwrap());
            assert_eq!(col.raw(8, 63, 8), 1 << 4);
            assert!(w.column(5, 5).is_none());
            let light = col.sky_light(8, 63, 8);
            assert_eq!(light.is_some(), config.is_some(), "{config:?}");
            let biomes = !matches!(col.biomes(), ghast_world::Biomes::None);
            assert_eq!(biomes, config.is_some(), "{config:?}");
        }
    }

    /// `after_tick` runs after the movement step with the sent pose, also once the behaviour
    /// finished; a panic there stops the behaviour only.
    #[test]
    fn after_tick_sees_the_sent_pose() {
        struct Obs {
            n: u32,
            views: Vec<(u64, Pose, u64)>,
        }
        impl Behavior for Obs {
            fn tick(&mut self, ctx: &mut TickContext<'_>) {
                self.n += 1;
                ctx.set_aim(150.0, 10.0);
                ctx.set_controls(Controls {
                    forward: true,
                    ..Controls::default()
                });
            }
            fn running(&self) -> bool {
                self.n < 5
            }
            fn after_tick(&mut self, view: &TickView<'_>) {
                self.views.push((view.tick(), *view.sent(), view.swings()));
                assert!(view.world().is_column_loaded(0, 0));
                assert_eq!(view.version(), GameVersion::V1_8_9);
            }
        }
        let mut e = engine(GameVersion::V1_8_9);
        join(&mut e);
        let t0 = Instant::now();
        e.note_inbound(t0);
        let mut b = Obs {
            n: 0,
            views: Vec::new(),
        };
        let mut sent = Vec::new();
        for _ in 0..10 {
            e.tick(&mut b, t0);
            e.outbound().clear();
            sent.push(e.core.sent);
        }
        assert_eq!(b.n, 5);
        assert_eq!(b.views.len(), 10, "also after the behaviour finished");
        for (i, (_, p, _)) in b.views.iter().enumerate() {
            assert_eq!(*p, sent[i]);
        }
        assert_ne!(
            b.views[0].1.yaw, b.views[4].1.yaw,
            "the view follows the turn"
        );
    }

    /// With block entities on, a 1.8 sign update reaches the column (and the packet passes the
    /// filter); off by default.
    #[test]
    fn block_entities_follow_the_world_config() {
        use ghast_protocol::v1_8::play::clientbound as cb;
        for on in [false, true] {
            let sim = crate::sim::Sim18::with(&crate::sim::SimOptions {
                world: on.then(|| ghast_world::WorldConfig::default().with_block_entities(true)),
                ..crate::sim::SimOptions::default()
            });
            assert_eq!(sim.world_packet_ids().contains(&cb::id::UPDATE_SIGN), on);
            let mut e = Engine::new(opts(GameVersion::V1_8_9), Box::new(sim) as Box<dyn Sim>);
            join(&mut e);
            let raw = RawPacket::from_packet(&cb::UpdateSign {
                location: ghast_protocol::BlockPos::new(3, 64, 4),
                text1: "\"Hello\"".into(),
                text2: "\"\"".into(),
                text3: "\"\"".into(),
                text4: "\"\"".into(),
            })
            .unwrap();
            e.sim.world_packet(&raw).unwrap();
            let w: &dyn WorldView = &e.sim;
            let be = w.column(0, 0).unwrap().block_entity(3, 64, 4);
            assert_eq!(be.is_some(), on, "{be:?}");
        }
    }
}
