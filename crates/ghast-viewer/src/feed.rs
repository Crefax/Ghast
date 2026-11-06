//! The world feed: turns what the bot knows (pose, world, entities, inventory, a few raw packets)
//! into protocol frames, once per client tick, shared by every page and recording.
//!
//! Cost model: nothing runs while nobody watches (the hub does not call the feed). While someone
//! watches, the feed builds ONE frame per tick (pose, entity moves, block changes, spawns …) and
//! hands the same bytes to every stream; chunk columns are encoded once (deflated off the tick
//! thread) and cached until a block in them changes, so a second viewer or a reload costs no
//! encoding.
//!
//! The feed only reads. It keeps a mirror of what the page was told (entities with their last
//! sent position, the last inventory, vitals, …), so a page that connects gets a snapshot without
//! waiting for a tick.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use ghast_client::{GameVersion, Health, Pose, RawInbound};
use ghast_entity::{Entities, Entity, EntityClass, Item, Metadata};
use ghast_inventory::Stack;
use ghast_protocol::BlockPos;
use ghast_protocol::v1_8::play::clientbound as cb;

use crate::entities::{
    display_name, estate_flags, kind_of, leather_color_nbt, looks_of, name_of, object_info,
    pitch_rad, size_of_entity, stack_of, variant_of, yaw_rad,
};
use crate::protocol::{
    Explosion, FX_EVENT, Looks, NO_COLOR, PROTOCOL, Particle, SELF, SectionIn, Sky, Writer, event,
    pose, s, skin_flag, write_explosion, write_looks, write_particles,
};
use crate::skins::{SkinStore, Textures, slim_by_uuid};
use crate::tiles::column_tiles;

/// Wall clock in ms since the Unix epoch (the page's clock sync compares against it).
#[must_use]
pub fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// What the feed reads from the bot during a tick. Implemented for ghast's `TickContext`; tests
/// implement it with a fake.
pub trait BotView {
    /// Client tick counter.
    fn tick(&self) -> u64;
    /// Protocol version of the session.
    fn version(&self) -> GameVersion;
    /// The bot's pose (feet position, look in degrees, motion per tick, flags).
    fn pose(&self) -> Pose;
    /// The bot's entity id.
    fn entity_id(&self) -> i32;
    /// Health, food, saturation.
    fn health(&self) -> Health;
    /// Selected hotbar slot.
    fn held_slot(&self) -> u8;
    /// Hotbar slot `0..9`.
    fn hotbar(&self, k: usize) -> Option<&Stack>;
    /// Armour: 0 head, 1 chest, 2 legs, 3 feet.
    fn armor(&self, a: usize) -> Option<&Stack>;
    /// The block being broken and its progress `0..=1`.
    fn mining(&self) -> Option<(BlockPos, f32)>;
    /// Blocks broken so far.
    fn broken(&self) -> u64;
    /// Arm swings sent so far, when the client counts them.
    fn swings(&self) -> Option<u64> {
        None
    }
    /// Using the held item.
    fn using_item(&self) -> bool;
    /// Dimension name as the client keeps it.
    fn dimension(&self) -> &str;
    /// Whether a column is loaded.
    fn is_column_loaded(&self, cx: i32, cz: i32) -> bool;
    /// Raw block state (1.8 `id << 4 | meta`); `None` when not loaded.
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32>;
    /// Direct column access, when the client offers it (faster than `raw_block`, with light
    /// and biomes when the world keeps them).
    fn column(&self, cx: i32, cz: i32) -> Option<&ghast_world::Column> {
        let _ = (cx, cz);
        None
    }
    /// Tracked entities (`None` without entity tracking).
    fn entities(&self) -> Option<&Entities>;
    /// Block changes since the last tick.
    fn block_updates(&self) -> &[(BlockPos, u32)];
    /// Raw packets copied for the viewer since the last tick.
    fn raw(&self) -> &[RawInbound];
    /// A join game happened since the last tick.
    fn joined(&self) -> bool;
    /// A respawn happened since the last tick.
    fn respawned(&self) -> bool;
}

/// 1.8 clientbound packet ids the feed reads while someone watches.
pub const RAW_WATCHED_1_8: &[i32] = &[
    cb::id::MAP_CHUNK,
    cb::id::MAP_CHUNK_BULK,
    cb::id::UPDATE_SIGN,
    cb::id::TILE_ENTITY_DATA,
    cb::id::BLOCK_BREAK_ANIMATION,
    cb::id::ANIMATION,
    cb::id::ENTITY_STATUS,
    cb::id::BLOCK_ACTION,
    cb::id::COLLECT,
    cb::id::WORLD_PARTICLES,
    cb::id::WORLD_EVENT,
    cb::id::EXPLOSION,
    cb::id::SPAWN_ENTITY_WEATHER,
    cb::id::UPDATE_TIME,
    cb::id::GAME_STATE_CHANGE,
    cb::id::BED,
    cb::id::EXPERIENCE,
];

/// 1.8 packet ids read even while nobody watches (rare, small packets with lasting state:
/// weather and experience).
pub const RAW_ALWAYS_1_8: &[i32] = &[cb::id::GAME_STATE_CHANGE, cb::id::EXPERIENCE];

/// What a page stream must do besides sending the tick frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// The bot changed or left the game: drop everything, a snapshot follows.
    Reset,
    /// A column was (re)sent or changed under a cached encoding: send it again.
    ColumnChanged(i32, i32),
    /// The server unloaded a column: the page drops it too.
    ColumnUnloaded(i32, i32),
}

/// The output of one tick.
#[derive(Debug, Default)]
pub struct TickFrames {
    /// Must reach every page (spawns, blocks, inventory …).
    pub reliable: Option<Bytes>,
    /// May be dropped by a congested page (pose, moves, particles).
    pub volatile: Option<Bytes>,
    pub notices: Vec<Notice>,
}

/// Block light and sky light nibbles of a section (`None`: all 0 / no sky).
pub type SectionLight = (Option<Box<[u8; 2048]>>, Option<Box<[u8; 2048]>>);

/// A column whose body is to be encoded and deflated off the tick thread.
pub struct ColumnJob {
    pub cx: i32,
    pub cz: i32,
    pub version: u32,
    pub states: Vec<Option<Box<[u16; 4096]>>>,
    pub light: Vec<Option<SectionLight>>,
    pub biomes: Option<Box<[u8; 256]>>,
    pub has_sky: bool,
    pub signs: Vec<crate::protocol::SignText>,
    pub tiles: Vec<crate::protocol::Tile>,
}

impl ColumnJob {
    /// The `CHUNK` message (column body deflated).
    #[must_use]
    pub fn encode(&self) -> Bytes {
        let mut sections: [Option<SectionIn<'_>>; 16] = [None; 16];
        for (y, st) in self.states.iter().enumerate().take(16) {
            let Some(st) = st else { continue };
            let light = self.light.get(y).and_then(Option::as_ref);
            sections[y] = Some(SectionIn {
                states: st,
                block_light: light.and_then(|l| l.0.as_deref()),
                sky: match light {
                    None => Sky::Full,
                    Some((_, None)) => Sky::Dark,
                    Some((_, Some(sky))) => Sky::Data(sky),
                },
            });
        }
        let body = crate::protocol::encode_column(
            &sections,
            self.biomes.as_deref(),
            self.has_sky,
            &self.signs,
            &self.tiles,
        );
        let z = deflate_raw(&body);
        let mut w = Writer::with_capacity(z.len() + 16);
        let at = w.begin(s::CHUNK);
        w.i32(self.cx).i32(self.cz).u8(1).bytes(&z);
        w.end(at);
        Bytes::from(w.into_vec())
    }
}

fn deflate_raw(b: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut e = flate2::write::DeflateEncoder::new(
        Vec::with_capacity(b.len() / 4 + 64),
        flate2::Compression::new(6),
    );
    // writing to a Vec cannot fail
    let _ = e.write_all(b);
    e.finish().unwrap_or_default()
}

/// A column's state in the cache.
#[derive(Debug, Clone)]
pub enum ColumnSlot {
    /// The current encoding.
    Ready(Bytes),
    /// Being encoded (or asked for; it is encoded at the next tick).
    Pending,
    /// Not loaded.
    Missing,
}

#[derive(Debug, Clone)]
enum Cached {
    Requested(u32),
    Encoding(u32),
    Done(u32, Bytes),
}

/// What the page was told about an entity, and what it was derived from.
#[derive(Debug, Clone)]
struct Known {
    seen: u64,
    pos: [f64; 3],
    rot: [f32; 3],
    item: (u16, u16),
    variant: u16,
    looks: Option<Looks>,
    estate: u32,
    meta: Metadata,
    equip: [Option<Item>; 5],
    vehicle: Option<i32>,
    skin: Option<String>,
    /// The SPAWN message (rewritten with the last position for a snapshot).
    kind: u8,
    name: String,
    label: String,
    size: (f32, f32),
    player: bool,
    uuid: u128,
}

/// Feed options.
#[derive(Debug, Clone)]
pub struct FeedConfig {
    /// Shown on the page.
    pub account: String,
    /// Columns sent around the bot: a (2d − 1)² square.
    pub view_distance: i32,
    /// No time budget for copying columns (tests: the same columns every run).
    pub unbudgeted: bool,
}

/// The feed. See the module docs.
pub struct Feed {
    cfg: FeedConfig,
    skins: Arc<SkinStore>,
    in_game: bool,
    /// A session is running (ticks arrive) even when the feed has not bound to it yet.
    live: bool,
    message: String,
    status: String,
    last_state: String,
    self_id: i32,
    dimension: String,
    has_sky: bool,
    center: Option<(i32, i32)>,
    pose: Option<Pose>,
    reliable: Writer,
    // columns
    loaded: HashSet<(i32, i32)>,
    versions: HashMap<(i32, i32), u32>,
    cache: HashMap<(i32, i32), Cached>,
    requests: VecDeque<(i32, i32)>,
    skull_waits: HashMap<String, Vec<(i32, i32)>>,
    /// Columns whose content changed since the last tick (pages are told then).
    changed_cols: HashSet<(i32, i32)>,
    // per tick
    blocks: Vec<(i32, u8, i32, u16)>,
    moved: Vec<i32>,
    known: HashMap<i32, Known>,
    ticks: u64,
    last_pose: Option<(Pose, u8)>,
    last_pose_at: f64,
    pose_msg: Option<Vec<u8>>,
    last_inv: Option<Vec<u8>>,
    last_self_looks: Option<Vec<u8>>,
    last_vitals: Option<Vec<u8>>,
    last_cracks: Option<Vec<u8>>,
    last_cracks_key: String,
    last_weather: Option<Vec<u8>>,
    last_weather_key: String,
    time_msg: Option<Vec<u8>>,
    swings: Option<u64>,
    broken: u64,
    breaking: HashMap<i32, (i32, i32, i32, u8)>,
    events: Vec<(u32, u8)>,
    estate_dirty: HashSet<i32>,
    sleeping: HashSet<i32>,
    chests: HashMap<(i32, i32, i32), u8>,
    chest_dirty: Vec<(i32, i32, i32, u8)>,
    particles: Vec<Particle>,
    fx: Writer,
    // lasting state from raw packets
    time: Option<(f64, f64, f64)>,
    raining: bool,
    rain_level: f32,
    thunder_level: f32,
    xp: (i32, f32),
    /// Profile textures by UUID: the raw property (to see changes) and its parse.
    profiles: HashMap<u128, (Box<str>, Textures)>,
    skin_sent: HashMap<u32, String>,
    skin_waits: HashSet<String>,
    /// Bytes produced (tick frames once, columns once).
    pub encoded_bytes: u64,
}

const MAX_ENCODE_PER_TICK: usize = 8;
/// Time a tick may spend copying columns for encoding.
const ENCODE_BUDGET_US: u128 = 4000;

impl Feed {
    /// A feed for one bot.
    #[must_use]
    pub fn new(cfg: FeedConfig, skins: Arc<SkinStore>) -> Self {
        Feed {
            cfg,
            skins,
            in_game: false,
            live: false,
            message: String::new(),
            status: String::new(),
            last_state: String::new(),
            self_id: i32::MIN,
            dimension: String::new(),
            has_sky: true,
            center: None,
            pose: None,
            reliable: Writer::with_capacity(1024),
            loaded: HashSet::new(),
            versions: HashMap::new(),
            cache: HashMap::new(),
            requests: VecDeque::new(),
            skull_waits: HashMap::new(),
            changed_cols: HashSet::new(),
            blocks: Vec::new(),
            moved: Vec::new(),
            known: HashMap::new(),
            ticks: 0,
            last_pose: None,
            last_pose_at: 0.0,
            pose_msg: None,
            last_inv: None,
            last_self_looks: None,
            last_vitals: None,
            last_cracks: None,
            last_cracks_key: String::new(),
            last_weather: None,
            last_weather_key: String::new(),
            time_msg: None,
            swings: None,
            broken: 0,
            breaking: HashMap::new(),
            events: Vec::new(),
            estate_dirty: HashSet::new(),
            sleeping: HashSet::new(),
            chests: HashMap::new(),
            chest_dirty: Vec::new(),
            particles: Vec::new(),
            fx: Writer::default(),
            time: None,
            raining: false,
            rain_level: 0.0,
            thunder_level: 0.0,
            xp: (0, 0.0),
            profiles: HashMap::new(),
            skin_sent: HashMap::new(),
            skin_waits: HashSet::new(),
            encoded_bytes: 0,
        }
    }

    /// The configuration.
    #[must_use]
    pub fn config(&self) -> &FeedConfig {
        &self.cfg
    }

    /// Whether the bot is in a world.
    #[must_use]
    pub fn in_game(&self) -> bool {
        self.in_game
    }

    /// The bot is in a world: bound (somebody watches) or seen in the unwatched ticks.
    #[must_use]
    pub fn session_in_game(&self) -> bool {
        self.in_game || self.live
    }

    /// The bot's chunk, `None` when not in game.
    #[must_use]
    pub fn center(&self) -> Option<(i32, i32)> {
        self.center
    }

    /// Whether a column is loaded (within the view range).
    #[must_use]
    pub fn has_column(&self, cx: i32, cz: i32) -> bool {
        self.loaded.contains(&(cx, cz))
    }

    /// Changes the status line shown on the page (sent with the next tick, or at once when
    /// not in game).
    pub fn set_status(&mut self, status: &str, message: &str) {
        self.status = status.to_string();
        self.message = message.to_string();
        self.write_state_if_changed();
    }

    fn state_json(&self) -> serde_json::Value {
        let message = if self.in_game || self.live {
            String::new()
        } else if self.message.is_empty() {
            "The bot is not in game".to_string()
        } else {
            self.message.clone()
        };
        serde_json::json!({
            "inGame": self.in_game || self.live,
            "account": self.cfg.account,
            "message": message,
            "status": self.status,
        })
    }

    fn write_state_if_changed(&mut self) {
        let j = self.state_json();
        let key = j.to_string();
        if key != self.last_state {
            self.last_state = key;
            self.reliable.json(s::STATE, &j);
        }
    }

    /// The `HELLO` frame for a new page.
    #[must_use]
    pub fn hello(&self, mode: &str) -> Bytes {
        let mut w = Writer::with_capacity(256);
        w.json(
            s::HELLO,
            &serde_json::json!({
                "protocol": PROTOCOL,
                "version": "1.8.8",
                "account": self.cfg.account,
                "mode": mode,
                "viewDistance": self.cfg.view_distance,
                "now": now_ms(),
            }),
        );
        Bytes::from(w.into_vec())
    }

    /// Takes reliable messages written outside a tick (state changes while not in game).
    pub fn take_reliable(&mut self) -> Option<Bytes> {
        (!self.reliable.is_empty()).then(|| Bytes::from(self.reliable.take()))
    }

    /// Ends the session: the next frames say "not in game".
    pub fn session_ended(&mut self, message: &str) -> TickFrames {
        let mut f = TickFrames::default();
        if self.in_game {
            self.unbind();
            f.notices.push(Notice::Reset);
        }
        self.live = false;
        self.message = message.to_string();
        self.write_state_if_changed();
        f.reliable = self.take_reliable();
        f
    }

    fn unbind(&mut self) {
        self.in_game = false;
        self.center = None;
        self.pose = None;
        self.loaded.clear();
        self.versions.clear();
        self.cache.clear();
        self.requests.clear();
        self.skull_waits.clear();
        self.profiles.clear();
        self.changed_cols.clear();
        self.blocks.clear();
        self.moved.clear();
        self.known.clear();
        self.last_pose = None;
        self.pose_msg = None;
        self.last_cracks = None;
        self.last_cracks_key.clear();
        self.breaking.clear();
        self.events.clear();
        self.estate_dirty.clear();
        self.sleeping.clear();
        self.chests.clear();
        self.chest_dirty.clear();
        self.particles.clear();
        self.fx = Writer::default();
        self.skin_sent.clear();
        self.skin_waits.clear();
    }

    /// Forgets everything (nobody watches any more).
    pub fn idle(&mut self) {
        let status = std::mem::take(&mut self.status);
        let message = std::mem::take(&mut self.message);
        self.unbind();
        self.last_inv = None;
        self.last_self_looks = None;
        self.last_vitals = None;
        self.last_weather = None;
        self.last_weather_key.clear();
        self.last_state.clear();
        self.reliable = Writer::with_capacity(1024);
        self.status = status;
        self.message = message;
    }

    // ---- raw packets that hold lasting state, read even while nobody watches -----------------

    /// Applies the raw packets of a tick while nobody watches (state only, no frames).
    pub fn observe(&mut self, v: &dyn BotView) {
        self.live = v.version() == GameVersion::V1_8_9;
        if !self.live {
            return;
        }
        for p in v.raw() {
            self.lasting(p);
        }
        // encodings only matter while someone watches
        self.changed_cols.clear();
        self.versions.clear();
        self.cache.clear();
    }

    /// The lasting-state part of a raw packet; returns false for packets it does not handle.
    fn lasting(&mut self, p: &RawInbound) -> bool {
        let mut r = ghast_protocol::Reader::new(&p.body);
        use ghast_protocol::Decode as _;
        match p.id {
            cb::id::GAME_STATE_CHANGE => {
                if let Ok(g) = cb::GameStateChange::decode(&mut r) {
                    match g.reason.0 {
                        1 => self.raining = true,
                        2 => self.raining = false,
                        7 => {
                            let was = self.rain_level > 0.0;
                            self.rain_level = g.game_mode;
                            if was != (g.game_mode > 0.0) {
                                self.raining = g.game_mode > 0.0;
                            }
                        }
                        8 => self.thunder_level = g.game_mode,
                        _ => {}
                    }
                }
            }
            cb::id::EXPERIENCE => {
                if let Ok(x) = cb::Experience::decode(&mut r) {
                    self.xp = (x.level, x.experience_bar);
                }
            }
            cb::id::UPDATE_SIGN | cb::id::TILE_ENTITY_DATA => {
                // the column carries its block entities: encode it again
                let at = if p.id == cb::id::UPDATE_SIGN {
                    cb::UpdateSign::decode(&mut r).map(|u| u.location)
                } else {
                    cb::TileEntityData::decode(&mut r).map(|t| t.location)
                };
                if let Ok(l) = at {
                    self.column_dirty(l.x >> 4, l.z >> 4);
                }
            }
            cb::id::MAP_CHUNK => {
                if let Ok(c) = cb::MapChunk::decode(&mut r) {
                    self.column_dirty(c.x, c.z);
                }
            }
            cb::id::MAP_CHUNK_BULK => {
                if let Ok(c) = cb::MapChunkBulk::decode(&mut r) {
                    for m in &c.meta {
                        self.column_dirty(m.x, m.z);
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// A column's content changed: forget its encoding (pages are told at the next tick).
    fn column_dirty(&mut self, cx: i32, cz: i32) {
        *self.versions.entry((cx, cz)).or_insert(0) += 1;
        self.cache.remove(&(cx, cz));
        self.changed_cols.insert((cx, cz));
    }

    // ---- the tick ------------------------------------------------------------------------------

    /// One client tick while somebody watches.
    pub fn tick(&mut self, v: &dyn BotView) -> TickFrames {
        let mut out = TickFrames::default();
        if v.version() != GameVersion::V1_8_9 {
            // only 1.8 is drawn so far: say so instead of showing a wrong world
            self.message = "Only 1.8 sessions can be shown".into();
            self.live = false;
            if self.in_game {
                self.unbind();
                out.notices.push(Notice::Reset);
            }
            self.write_state_if_changed();
            out.reliable = self.take_reliable();
            return out;
        }
        if !self.in_game || v.joined() || v.entity_id() != self.self_id {
            if self.in_game {
                self.unbind();
            }
            out.notices.push(Notice::Reset);
            self.bind(v);
        } else if v.respawned() && v.dimension() != self.dimension {
            self.unbind();
            out.notices.push(Notice::Reset);
            self.bind(v);
        }
        self.write_state_if_changed();
        self.ticks += 1;
        let p = v.pose();
        self.pose = Some(p);
        let c = (
            (p.pos.x.floor() as i64 >> 4) as i32,
            (p.pos.z.floor() as i64 >> 4) as i32,
        );
        self.center = Some(c);

        for raw in v.raw() {
            if !self.lasting(raw) {
                self.fx_packet(raw, v);
            }
        }
        for &(pos, state) in v.block_updates() {
            self.block(pos.x, pos.y, pos.z, state);
        }
        self.scan_columns(v, &mut out.notices);
        for k in self.changed_cols.drain() {
            if self.loaded.contains(&k) {
                out.notices.push(Notice::ColumnChanged(k.0, k.1));
            }
        }
        self.poll_skins(v.entities(), &mut out.notices);
        self.entities(v);
        self.close_blocks();
        self.close_animation(v);
        self.cracks(v, false);
        if self.ticks.is_multiple_of(10) {
            self.skin_self(v.entities());
            self.inventory(v, false);
            self.self_looks(v, false);
            self.vitals(v, false);
        }
        if self.ticks.is_multiple_of(20) {
            self.time_message();
        }
        if self.ticks.is_multiple_of(5) {
            self.weather(false);
        }
        if self.ticks.is_multiple_of(200) {
            self.prune_far();
        }
        out.volatile = self.volatile_frame(v);
        out.reliable = self.take_reliable();
        self.encoded_bytes += out.reliable.as_ref().map_or(0, |b| b.len() as u64)
            + out.volatile.as_ref().map_or(0, |b| b.len() as u64);
        out
    }

    fn bind(&mut self, v: &dyn BotView) {
        self.in_game = true;
        self.live = true;
        self.self_id = v.entity_id();
        self.dimension = v.dimension().to_string();
        let d = self.dimension.as_str();
        self.has_sky = !(matches!(d, "-1" | "1") || d.contains("nether") || d.contains("the_end"));
        self.last_pose = None;
        self.last_inv = None;
        self.last_vitals = None;
        self.last_self_looks = None;
        self.swings = v.swings();
        self.broken = v.broken();
        self.inventory(v, true);
        self.self_looks(v, true);
        self.vitals(v, true);
        self.skin_self(v.entities());
    }

    fn scan_columns(&mut self, v: &dyn BotView, notices: &mut Vec<Notice>) {
        let Some((bx, bz)) = self.center else { return };
        let vd = self.cfg.view_distance;
        let mut now = HashSet::with_capacity(self.loaded.len() + 8);
        for dx in -vd + 1..vd {
            for dz in -vd + 1..vd {
                let k = (bx + dx, bz + dz);
                if v.is_column_loaded(k.0, k.1) {
                    now.insert(k);
                    if !self.loaded.contains(&k) {
                        // the encoding of an earlier copy (if any) is stale
                        *self.versions.entry(k).or_insert(0) += 1;
                        self.cache.remove(&k);
                        self.changed_cols.remove(&k);
                        notices.push(Notice::ColumnChanged(k.0, k.1));
                    }
                }
            }
        }
        for &k in &self.loaded {
            if !now.contains(&k) && (k.0 - bx).abs() < vd && (k.1 - bz).abs() < vd {
                self.cache.remove(&k);
                notices.push(Notice::ColumnUnloaded(k.0, k.1));
            }
        }
        self.loaded = now;
    }

    /// The cache slot of a column, asking for an encoding when there is none.
    pub fn column(&mut self, cx: i32, cz: i32) -> ColumnSlot {
        let k = (cx, cz);
        if !self.loaded.contains(&k) {
            return ColumnSlot::Missing;
        }
        let version = self.versions.get(&k).copied().unwrap_or(0);
        match self.cache.get(&k) {
            Some(Cached::Done(v, f)) if *v == version => ColumnSlot::Ready(f.clone()),
            Some(Cached::Requested(v) | Cached::Encoding(v)) if *v == version => {
                ColumnSlot::Pending
            }
            _ => {
                self.cache.insert(k, Cached::Requested(version));
                self.requests.push_back(k);
                ColumnSlot::Pending
            }
        }
    }

    /// Whether `frame` is still the current encoding of the column.
    #[must_use]
    pub fn is_current(&self, cx: i32, cz: i32, frame: &Bytes) -> bool {
        let version = self.versions.get(&(cx, cz)).copied().unwrap_or(0);
        matches!(self.cache.get(&(cx, cz)), Some(Cached::Done(v, f)) if *v == version && f.as_ptr() == frame.as_ptr())
    }

    /// Copies the requested columns out of the world (within the tick's time budget) for
    /// encoding.
    pub fn take_jobs(&mut self, v: &dyn BotView) -> Vec<ColumnJob> {
        let mut jobs = Vec::new();
        let t0 = std::time::Instant::now();
        while let Some(k) = self.requests.pop_front() {
            let version = self.versions.get(&k).copied().unwrap_or(0);
            if !matches!(self.cache.get(&k), Some(Cached::Requested(r)) if *r == version) {
                continue;
            }
            if !self.loaded.contains(&k) {
                self.cache.remove(&k);
                continue;
            }
            jobs.push(self.snapshot_column(v, k.0, k.1, version));
            self.cache.insert(k, Cached::Encoding(version));
            if jobs.len() >= MAX_ENCODE_PER_TICK
                || (!self.cfg.unbudgeted && t0.elapsed().as_micros() > ENCODE_BUDGET_US)
            {
                break;
            }
        }
        jobs
    }

    /// An encoding finished: keeps it when the column did not change meanwhile (else asks again).
    /// Returns whether pages may have something new to send.
    pub fn finish(&mut self, cx: i32, cz: i32, version: u32, frame: Bytes) -> bool {
        let k = (cx, cz);
        let current = self.versions.get(&k).copied().unwrap_or(0);
        match self.cache.get(&k) {
            Some(Cached::Encoding(v)) if *v == version && version == current => {
                self.encoded_bytes += frame.len() as u64;
                self.cache.insert(k, Cached::Done(version, frame));
                true
            }
            Some(Cached::Encoding(v)) if *v == version => {
                // changed while encoding: again at the next tick
                if self.loaded.contains(&k) {
                    self.cache.insert(k, Cached::Requested(current));
                    self.requests.push_back(k);
                } else {
                    self.cache.remove(&k);
                }
                false
            }
            _ => false,
        }
    }

    fn snapshot_column(&mut self, v: &dyn BotView, cx: i32, cz: i32, version: u32) -> ColumnJob {
        let mut states: Vec<Option<Box<[u16; 4096]>>> = vec![None; 16];
        let mut light = vec![None; 16];
        let mut biomes = None;
        if let Some(col) = v.column(cx, cz) {
            for (y, slot) in states.iter_mut().enumerate() {
                let Some(sec) = col.section(y) else { continue };
                let mut st = Box::new([0u16; 4096]);
                sec.for_each(|i, s| st[i] = s);
                *slot = Some(st);
            }
            if let ghast_world::Biomes::Legacy(b) = col.biomes() {
                biomes = Some(b.clone());
            }
            let base_y = 0;
            for (y, l) in light.iter_mut().enumerate() {
                if states[y].is_none() {
                    continue;
                }
                let yy = base_y + (y as i32) * 16;
                if col.block_light(0, yy, 0).is_none() {
                    continue;
                }
                let mut block = Box::new([0u8; 2048]);
                let mut sky = self.has_sky.then(|| Box::new([0u8; 2048]));
                for i in 0..4096usize {
                    let (x, yi, z) = ((i & 15) as i32, (i >> 8) as i32, ((i >> 4) & 15) as i32);
                    let b = col.block_light(x, yy + yi, z).unwrap_or(0);
                    block[i >> 1] |= if i & 1 == 0 { b } else { b << 4 };
                    if let Some(sky) = sky.as_mut() {
                        let s = col.sky_light(x, yy + yi, z).unwrap_or(15);
                        sky[i >> 1] |= if i & 1 == 0 { s } else { s << 4 };
                    }
                }
                *l = Some((Some(block), sky));
            }
        } else {
            let (x0, z0) = (cx * 16, cz * 16);
            for (sy, slot) in states.iter_mut().enumerate() {
                let mut st = Box::new([0u16; 4096]);
                let mut any = false;
                for (i, s) in st.iter_mut().enumerate() {
                    let (x, y, z) = (
                        x0 + (i & 15) as i32,
                        (sy * 16 + (i >> 8)) as i32,
                        z0 + ((i >> 4) & 15) as i32,
                    );
                    let raw = v.raw_block(x, y, z).unwrap_or(0) as u16;
                    any |= raw != 0;
                    *s = raw;
                }
                if any {
                    *slot = Some(st);
                }
            }
        }
        let skins = Arc::clone(&self.skins);
        let mut waits = Vec::new();
        let block_name = |id: i32| {
            u8::try_from(id)
                .ok()
                .and_then(ghast_data::v1_8::Block::from_id)
                .map(ghast_data::v1_8::Block::name)
        };
        let (signs, tiles) = match v.column(cx, cz) {
            Some(col) => column_tiles(col, block_name, |url| {
                if let Some(p) = skins.ready(url) {
                    return p;
                }
                if skins.load(url, false) {
                    waits.push(url.to_string());
                }
                String::new()
            }),
            None => (Vec::new(), Vec::new()),
        };
        for u in waits {
            let list = self.skull_waits.entry(u).or_default();
            if !list.contains(&(cx, cz)) {
                list.push((cx, cz));
            }
        }
        ColumnJob {
            cx,
            cz,
            version,
            states,
            light,
            biomes,
            has_sky: self.has_sky,
            signs,
            tiles,
        }
    }

    /// Forgets cached encodings of columns no page can see any more.
    fn prune_far(&mut self) {
        let Some((bx, bz)) = self.center else { return };
        let far = |k: &(i32, i32)| (k.0 - bx).abs().max((k.1 - bz).abs()) > 13;
        self.cache.retain(|k, _| !far(k));
        self.versions.retain(|k, _| !far(k));
    }

    // ---- blocks ---------------------------------------------------------------------------------

    fn block(&mut self, x: i32, y: i32, z: i32, state: u32) {
        // a block that changed is not being broken any more
        self.breaking
            .retain(|_, c| !(c.0 == x && c.1 == y && c.2 == z));
        let Ok(yy) = u8::try_from(y) else { return };
        self.blocks.push((x, yy, z, state as u16));
        let k = (x >> 4, z >> 4);
        *self.versions.entry(k).or_insert(0) += 1;
        self.cache.remove(&k);
    }

    fn close_blocks(&mut self) {
        if self.blocks.is_empty() {
            return;
        }
        for part in self.blocks.chunks(0xffff) {
            let at = self.reliable.begin(s::BLOCKS);
            self.reliable.u16(part.len() as u16);
            for &(x, y, z, st) in part {
                self.reliable.i32(x).u8(y).i32(z).u16(st);
            }
            self.reliable.end(at);
        }
        self.blocks.clear();
    }

    // ---- entities ------------------------------------------------------------------------------

    fn entities(&mut self, v: &dyn BotView) {
        let Some(ents) = v.entities() else {
            return;
        };
        let stamp = self.ticks;
        let me = v.entity_id();
        for e in ents.iter() {
            if e.id() == me {
                continue;
            }
            let id = e.id();
            let mut respawn = false;
            match self.known.get_mut(&id) {
                None => {
                    self.spawn(e, ents, stamp);
                    self.estate_dirty.insert(id);
                }
                Some(k) => {
                    k.seen = stamp;
                    let p = e.position();
                    let rot = [e.yaw(), e.pitch(), e.head_yaw()];
                    if p != k.pos || rot != k.rot {
                        k.pos = p;
                        k.rot = rot;
                        self.moved.push(id);
                    }
                    let equip_changed = (0..5).any(|i| e.equipment(i) != k.equip[i].as_ref());
                    if equip_changed || e.metadata() != &k.meta {
                        k.meta = e.metadata().clone();
                        for i in 0..5 {
                            k.equip[i] = e.equipment(i).cloned();
                        }
                        let variant = variant_of(e);
                        if variant != k.variant {
                            // a new variant (slime size, sheared sheep …): spawn it again
                            respawn = true;
                        } else {
                            let looks = looks_of(e);
                            if looks != k.looks {
                                write_looks(
                                    &mut self.reliable,
                                    id as u32,
                                    looks.as_ref().unwrap_or(&Looks::default()),
                                );
                                k.looks = looks;
                            }
                            let st = stack_of(e);
                            if st != k.item {
                                k.item = st;
                                let at = self.reliable.begin(s::EQUIP);
                                self.reliable.u32(id as u32).u16(st.0).u16(st.1);
                                self.reliable.end(at);
                            }
                        }
                        self.estate_dirty.insert(id);
                    }
                    if e.vehicle() != k.vehicle {
                        k.vehicle = e.vehicle();
                        self.estate_dirty.insert(id);
                    }
                    if k.player {
                        let uuid = k.uuid;
                        self.refresh_profile(uuid, ents.player_info(&e.uuid()));
                        let key = self.skin_key(uuid);
                        let Some(k) = self.known.get_mut(&id) else {
                            continue;
                        };
                        if Some(&key) != k.skin.as_ref() {
                            k.skin = Some(key.clone());
                            self.write_skin(id as u32, &key);
                        }
                    }
                }
            }
            if respawn {
                self.spawn(e, ents, stamp);
            }
        }
        let gone: Vec<i32> = self
            .known
            .iter()
            .filter(|(_, k)| k.seen != stamp)
            .map(|(id, _)| *id)
            .collect();
        if !gone.is_empty() {
            let at = self.reliable.begin(s::REMOVE);
            self.reliable.u16(gone.len().min(0xffff) as u16);
            for id in gone.iter().take(0xffff) {
                self.reliable.u32(*id as u32);
            }
            self.reliable.end(at);
            for id in gone {
                self.known.remove(&id);
                self.moved.retain(|m| *m != id);
                self.estate_dirty.remove(&id);
                self.sleeping.remove(&id);
                self.skin_sent.remove(&(id as u32));
            }
        }
    }

    fn spawn(&mut self, e: &Entity, ents: &Entities, stamp: u64) {
        let id = e.id();
        let player = e.class() == EntityClass::Player;
        let label = if player {
            ents.player_info(&e.uuid())
                .map(|p| p.name.to_string())
                .unwrap_or_default()
        } else if object_info(e).is_some() {
            String::new()
        } else {
            display_name(e).to_string()
        };
        let looks = looks_of(e);
        let mut k = Known {
            seen: stamp,
            pos: e.position(),
            rot: [e.yaw(), e.pitch(), e.head_yaw()],
            item: stack_of(e),
            variant: variant_of(e),
            looks,
            estate: self.known.get(&id).map_or(0, |k| k.estate),
            meta: e.metadata().clone(),
            equip: std::array::from_fn(|i| e.equipment(i).cloned()),
            vehicle: e.vehicle(),
            skin: None,
            kind: kind_of(e),
            name: name_of(e),
            label,
            size: size_of_entity(e),
            player,
            uuid: e.uuid().as_u128(),
        };
        write_spawn(&mut self.reliable, id as u32, &k);
        if player {
            self.refresh_profile(k.uuid, ents.player_info(&e.uuid()));
            let key = self.skin_key(k.uuid);
            self.write_skin(id as u32, &key);
            k.skin = Some(key);
        }
        self.known.insert(id, k);
    }

    /// `flags|skin|cape` of a profile (the paths '' until downloaded; downloads are started).
    fn skin_key(&mut self, uuid: u128) -> String {
        let (textures, slim_default) = match self.profiles.get(&uuid) {
            Some((_, t)) => (Some(t.clone()), slim_by_uuid(uuid)),
            None => (None, slim_by_uuid(uuid)),
        };
        self.textures_key(textures.as_ref(), slim_default)
    }

    fn textures_key(&mut self, t: Option<&Textures>, slim_default: bool) -> String {
        let slim = match t {
            Some(t) if t.skin.is_some() => t.slim,
            _ => slim_default,
        };
        let mut path = |url: Option<&String>, cape: bool| -> String {
            let Some(url) = url else { return String::new() };
            if let Some(p) = self.skins.ready(url) {
                return p;
            }
            if self.skins.load(url, cape) {
                self.skin_waits.insert(url.clone());
            }
            String::new()
        };
        let skin = path(t.and_then(|t| t.skin.as_ref()), false);
        let cape = path(t.and_then(|t| t.cape.as_ref()), true);
        format!("{}|{skin}|{cape}", if slim { skin_flag::SLIM } else { 0 })
    }

    fn write_skin(&mut self, id: u32, key: &str) {
        self.skin_sent.insert(id, key.to_string());
        write_skin_msg(&mut self.reliable, id, key);
    }

    /// Refreshes the profile textures of `uuid` from the tab list (parsed only on change).
    fn refresh_profile(&mut self, uuid: u128, info: Option<&ghast_entity::PlayerInfo>) {
        let raw = info.and_then(|i| i.textures.as_deref());
        match raw {
            Some(r) => {
                if self.profiles.get(&uuid).is_none_or(|(old, _)| **old != *r) {
                    let t = crate::skins::parse_textures(r).unwrap_or_default();
                    self.profiles.insert(uuid, (r.into(), t));
                }
            }
            None => {
                self.profiles.remove(&uuid);
            }
        }
    }

    fn skin_self(&mut self, ents: Option<&Entities>) {
        let name = self.cfg.account.clone();
        let mine = ents.and_then(|e| {
            e.players()
                .find(|(_, p)| *p.name == *name)
                .map(|(u, p)| (u.as_u128(), p.clone()))
        });
        if let Some((u, p)) = &mine {
            self.refresh_profile(*u, Some(p));
        }
        let key = match mine {
            Some((u, _)) => self.skin_key(u),
            None => self.textures_key(None, false),
        };
        if self.skin_sent.get(&SELF) != Some(&key) {
            self.write_skin(SELF, &key);
        }
    }

    /// Downloads that finished: SKIN again for the players using them, columns with skulls.
    fn poll_skins(&mut self, ents: Option<&Entities>, notices: &mut Vec<Notice>) {
        if !self.skin_waits.is_empty() {
            let done: Vec<String> = self
                .skin_waits
                .iter()
                .filter(|u| !self.skins.loading(u))
                .cloned()
                .collect();
            if !done.is_empty() {
                for u in &done {
                    self.skin_waits.remove(u);
                }
                let players: Vec<(i32, u128)> = self
                    .known
                    .iter()
                    .filter(|(_, k)| k.player)
                    .map(|(id, k)| (*id, k.uuid))
                    .collect();
                for (id, uuid) in players {
                    let key = self.skin_key(uuid);
                    if let Some(k) = self.known.get_mut(&id)
                        && k.skin.as_ref() != Some(&key)
                    {
                        k.skin = Some(key.clone());
                        self.write_skin(id as u32, &key);
                    }
                }
                self.skin_self(ents);
            }
        }
        if !self.skull_waits.is_empty() {
            let done: Vec<String> = self
                .skull_waits
                .keys()
                .filter(|u| !self.skins.loading(u))
                .cloned()
                .collect();
            for u in done {
                let got = self.skins.ready(&u).is_some();
                for (cx, cz) in self.skull_waits.remove(&u).unwrap_or_default() {
                    if got && self.loaded.contains(&(cx, cz)) {
                        *self.versions.entry((cx, cz)).or_insert(0) += 1;
                        self.cache.remove(&(cx, cz));
                        notices.push(Notice::ColumnChanged(cx, cz));
                    }
                }
            }
        }
    }

    // ---- raw packets with effects (only while watched) ---------------------------------------

    fn near(&self, x: f64, y: f64, z: f64) -> bool {
        let Some(p) = self.pose else { return false };
        (x + y + z).is_finite()
            && (x - p.pos.x).abs() < 64.0
            && (y - p.pos.y).abs() < 64.0
            && (z - p.pos.z).abs() < 64.0
    }

    fn push_particle(&mut self, m: Particle) {
        if self.particles.len() < 64 && self.near(m.x, m.y, m.z) {
            self.particles.push(m);
        }
    }

    fn event(&mut self, id: i32, ev: u8) {
        if id == self.self_id {
            self.events.push((SELF, ev));
        } else if self.known.contains_key(&id) {
            self.events.push((id as u32, ev));
        }
    }

    fn fx_packet(&mut self, p: &RawInbound, _v: &dyn BotView) {
        use ghast_protocol::Decode as _;
        let mut r = ghast_protocol::Reader::new(&p.body);
        match p.id {
            cb::id::ANIMATION => {
                if let Ok(a) = cb::Animation::decode(&mut r) {
                    match a.animation {
                        0 => self.event(a.entity_id, event::SWING),
                        1 => self.event(a.entity_id, event::HURT),
                        2 if self.sleeping.remove(&a.entity_id) => {
                            self.estate_dirty.insert(a.entity_id);
                        }
                        _ => {}
                    }
                }
            }
            cb::id::ENTITY_STATUS => {
                if let Ok(st) = cb::EntityStatus::decode(&mut r) {
                    match st.entity_status {
                        2 => self.event(st.entity_id, event::HURT),
                        3 => self.event(st.entity_id, event::DEATH),
                        _ => {}
                    }
                }
            }
            cb::id::BED => {
                if let Ok(b) = cb::Bed::decode(&mut r) {
                    self.sleeping.insert(b.entity_id);
                    self.estate_dirty.insert(b.entity_id);
                    if self.known.contains_key(&b.entity_id) {
                        self.moved.push(b.entity_id);
                    }
                }
            }
            cb::id::COLLECT => {
                if let Ok(c) = cb::Collect::decode(&mut r)
                    && self.known.contains_key(&c.collected_entity_id)
                {
                    let to = if c.collector_entity_id == self.self_id {
                        SELF
                    } else {
                        c.collector_entity_id as u32
                    };
                    let at = self.reliable.begin(s::COLLECT);
                    self.reliable.u32(c.collected_entity_id as u32).u32(to);
                    self.reliable.end(at);
                }
            }
            cb::id::BLOCK_ACTION => {
                if let Ok(b) = cb::BlockAction::decode(&mut r) {
                    // chest, trapped chest, ender chest: action 1 carries the viewer count
                    if b.byte1 == 1 && matches!(b.block_id, 54 | 130 | 146) {
                        let l = b.location;
                        let n = b.byte2;
                        let k = (l.x, l.y, l.z);
                        if self.chests.get(&k).copied().unwrap_or(0) != n {
                            if n > 0 {
                                self.chests.insert(k, n);
                            } else {
                                self.chests.remove(&k);
                            }
                            self.chest_dirty.push((l.x, l.y, l.z, n));
                        }
                    }
                }
            }
            cb::id::BLOCK_BREAK_ANIMATION => {
                if let Ok(b) = cb::BlockBreakAnimation::decode(&mut r) {
                    let l = b.location;
                    if (0..=9).contains(&b.destroy_stage) {
                        self.breaking
                            .insert(b.entity_id, (l.x, l.y, l.z, b.destroy_stage as u8));
                    } else {
                        self.breaking.remove(&b.entity_id);
                    }
                }
            }
            cb::id::WORLD_PARTICLES => {
                if let Ok(w) = cb::WorldParticles::decode(&mut r) {
                    let d: &[i32] = match &w.data {
                        cb::WorldParticlesData::Case36(d)
                        | cb::WorldParticlesData::Case37(d)
                        | cb::WorldParticlesData::Case38(d) => d,
                        cb::WorldParticlesData::Other => &[],
                    };
                    let d0 = d.first().copied().unwrap_or(0);
                    let data = match w.particle_id {
                        37 | 38 => ((d0 & 4095) << 4) | ((d0 >> 12) & 15),
                        36 => (d0 << 4) | (d.get(1).copied().unwrap_or(0) & 15),
                        _ => 0,
                    };
                    self.push_particle(Particle {
                        id: (w.particle_id & 255) as u8,
                        x: f64::from(w.x),
                        y: f64::from(w.y),
                        z: f64::from(w.z),
                        ox: w.offset_x,
                        oy: w.offset_y,
                        oz: w.offset_z,
                        speed: w.particle_data,
                        count: w.particles.clamp(0, 65535) as u16,
                        data: (data & 0xffff) as u16,
                    });
                }
            }
            cb::id::WORLD_EVENT => {
                if let Ok(e) = cb::WorldEvent::decode(&mut r)
                    && matches!(e.effect_id, 2000 | 2004 | 2005)
                {
                    let l = e.location;
                    self.push_particle(Particle {
                        id: FX_EVENT + (e.effect_id - 2000) as u8,
                        x: f64::from(l.x),
                        y: f64::from(l.y),
                        z: f64::from(l.z),
                        ox: 0.0,
                        oy: 0.0,
                        oz: 0.0,
                        speed: 0.0,
                        count: 1,
                        data: (e.data & 0xffff) as u16,
                    });
                }
            }
            cb::id::EXPLOSION => {
                if let Ok(e) = cb::Explosion::decode(&mut r) {
                    let (x, y, z) = (f64::from(e.x), f64::from(e.y), f64::from(e.z));
                    if self.near(x, y, z) {
                        let blocks = e
                            .affected_block_offsets
                            .iter()
                            .map(|b| [b.x, b.y, b.z])
                            .collect();
                        write_explosion(
                            &mut self.fx,
                            &Explosion {
                                x,
                                y,
                                z,
                                radius: e.radius,
                                blocks,
                            },
                        );
                    }
                }
            }
            cb::id::SPAWN_ENTITY_WEATHER => {
                if let Ok(w) = cb::SpawnEntityWeather::decode(&mut r)
                    && w.r#type == 1
                {
                    let at = self.fx.begin(s::LIGHTNING);
                    self.fx
                        .f64(f64::from(w.x) / 32.0)
                        .f64(f64::from(w.y) / 32.0)
                        .f64(f64::from(w.z) / 32.0);
                    self.fx.end(at);
                }
            }
            cb::id::UPDATE_TIME => {
                if let Ok(t) = cb::UpdateTime::decode(&mut r) {
                    let first = self.time.is_none();
                    self.time = Some((now_ms(), t.age as f64, t.time as f64));
                    if first {
                        self.time_message();
                    }
                }
            }
            _ => {}
        }
    }

    // ---- animation, state, chests --------------------------------------------------------------

    fn estate_of(&self, e: &Entity) -> u32 {
        let flags = estate_flags(e, self.sleeping.contains(&e.id()));
        let vehicle = e.vehicle().map_or(0, |v| v as u32 + 1);
        u32::from(flags) | (vehicle << 8)
    }

    fn close_animation(&mut self, v: &dyn BotView) {
        if !self.estate_dirty.is_empty() {
            let mut list = Vec::new();
            if let Some(ents) = v.entities() {
                let dirty: Vec<i32> = self.estate_dirty.drain().collect();
                for id in dirty {
                    let Some(e) = ents.get(id) else { continue };
                    let value = self.estate_of(e);
                    let Some(k) = self.known.get_mut(&id) else {
                        continue;
                    };
                    if value != k.estate {
                        k.estate = value;
                        list.push((id as u32, value));
                    }
                }
            }
            self.estate_dirty.clear();
            if !list.is_empty() {
                write_states(&mut self.reliable, &list);
            }
        }
        if !self.events.is_empty() {
            let n = self.events.len().min(0xffff);
            let at = self.reliable.begin(s::EVENTS);
            self.reliable.u16(n as u16);
            for &(id, ev) in &self.events[..n] {
                self.reliable.u32(id).u8(ev);
            }
            self.reliable.end(at);
            self.events.clear();
        }
        if !self.chest_dirty.is_empty() {
            let list = std::mem::take(&mut self.chest_dirty);
            write_chests(&mut self.reliable, &list);
        }
    }

    // ---- the bot ----------------------------------------------------------------------------

    fn volatile_frame(&mut self, v: &dyn BotView) -> Option<Bytes> {
        let mut w = Writer::with_capacity(256);
        let p = v.pose();
        let swung = match v.swings() {
            Some(n) => {
                let s = self.swings != Some(n);
                self.swings = Some(n);
                s
            }
            None => {
                let b = v.broken();
                let s = v.mining().is_some() || b != self.broken;
                self.broken = b;
                s
            }
        };
        let riding = v
            .entities()
            .and_then(Entities::me)
            .and_then(Entity::vehicle)
            .is_some();
        let flags = (if p.on_ground { pose::GROUND } else { 0 })
            | (if p.flying { pose::FLYING } else { 0 })
            | (if p.sneaking { pose::SNEAK } else { 0 })
            | (if p.sprinting { pose::SPRINT } else { 0 })
            | (if swung { pose::SWING } else { 0 })
            | (if v.using_item() { pose::USE } else { 0 })
            | (if riding { pose::RIDE } else { 0 });
        let t = now_ms();
        let same = self.last_pose.is_some_and(|(lp, lf)| {
            lf == flags && lp.pos == p.pos && lp.yaw == p.yaw && lp.pitch == p.pitch
        });
        if !same || t - self.last_pose_at > 1000.0 {
            self.last_pose = Some((p, flags));
            self.last_pose_at = t;
            let mut m = Writer::with_capacity(64);
            let at = m.begin(s::POSE);
            m.f64(t)
                .u32(v.tick() as u32)
                .f64(p.pos.x)
                .f64(p.pos.y)
                .f64(p.pos.z)
                .f32(yaw_rad(p.yaw))
                .f32(pitch_rad(p.pitch))
                .u8(flags)
                .f32(p.motion.x as f32)
                .f32(p.motion.y as f32)
                .f32(p.motion.z as f32);
            m.end(at);
            w.bytes(m.as_slice());
            // the snapshot's pose never carries a one-off swing
            let mut keep = m.into_vec();
            keep[5 + 8 + 4 + 24 + 8] &= !pose::SWING;
            self.pose_msg = Some(keep);
        }
        if !self.particles.is_empty() {
            write_particles(&mut w, &self.particles);
            self.particles.clear();
        }
        if !self.fx.is_empty() {
            w.bytes(self.fx.as_slice());
            self.fx = Writer::default();
        }
        if !self.moved.is_empty() {
            self.moved.sort_unstable();
            self.moved.dedup();
            let ids = std::mem::take(&mut self.moved);
            let at = w.begin(s::MOVES);
            let n = ids.len().min(0xffff);
            w.u16(n as u16);
            for id in &ids[..n] {
                if let Some(k) = self.known.get(id) {
                    write_move(&mut w, *id as u32, k);
                } else {
                    write_move_zero(&mut w, *id as u32);
                }
            }
            w.end(at);
        }
        (!w.is_empty()).then(|| Bytes::from(w.into_vec()))
    }

    /// `MOVES` of every known entity (for a page that dropped volatile frames).
    #[must_use]
    pub fn all_moves(&self) -> Option<Bytes> {
        if !self.in_game || self.known.is_empty() {
            return None;
        }
        let mut w = Writer::with_capacity(16 + self.known.len() * 40);
        let at = w.begin(s::MOVES);
        let n = self.known.len().min(0xffff);
        w.u16(n as u16);
        for (id, k) in self.known.iter().take(n) {
            write_move(&mut w, *id as u32, k);
        }
        w.end(at);
        Some(Bytes::from(w.into_vec()))
    }

    fn inventory(&mut self, v: &dyn BotView, force: bool) {
        let mut m = Writer::with_capacity(64);
        let at = m.begin(s::INVENTORY);
        m.u8(v.held_slot());
        for k in 0..9 {
            match v.hotbar(k).filter(|s| s.id > 0 && s.count > 0) {
                Some(st) => m
                    .u16(st.id as u16)
                    .u16(st.damage as u16)
                    .u8(st.count.clamp(0, 255) as u8),
                None => m.u16(0).u16(0).u8(0),
            };
        }
        m.end(at);
        let m = m.into_vec();
        if force || self.last_inv.as_ref() != Some(&m) {
            self.reliable.bytes(&m);
            self.last_inv = Some(m);
        }
    }

    fn self_looks(&mut self, v: &dyn BotView, force: bool) {
        let mut armour = Vec::new();
        for slot in 1..=4u8 {
            if let Some(st) = v.armor(4 - usize::from(slot)).filter(|s| s.id > 0) {
                let color = st.nbt.as_ref().map_or(NO_COLOR, leather_color_nbt);
                armour.push((slot, st.id as u16, st.damage as u16, color));
            }
        }
        let mut m = Writer::with_capacity(64);
        write_looks(
            &mut m,
            SELF,
            &Looks {
                flags: 0,
                armour,
                pose: None,
            },
        );
        let m = m.into_vec();
        if force || self.last_self_looks.as_ref() != Some(&m) {
            self.reliable.bytes(&m);
            self.last_self_looks = Some(m);
        }
    }

    fn vitals(&mut self, v: &dyn BotView, force: bool) {
        let h = v.health();
        let mut m = Writer::with_capacity(32);
        let at = m.begin(s::VITALS);
        m.f32(h.health)
            .u8(h.food.clamp(0, 255) as u8)
            .f32(h.saturation)
            .u8(self.xp.0.clamp(0, 255) as u8)
            .f32(self.xp.1);
        m.end(at);
        let m = m.into_vec();
        if force || self.last_vitals.as_ref() != Some(&m) {
            self.reliable.bytes(&m);
            self.last_vitals = Some(m);
        }
    }

    fn cracks(&mut self, v: &dyn BotView, force: bool) {
        let mut list: Vec<(i32, i32, i32, u8)> = self.breaking.values().copied().collect();
        list.sort_unstable();
        if let Some((pos, progress)) = v.mining() {
            let stage = (progress * 10.0).floor() as i32 - 1;
            if stage >= 0 {
                list.push((pos.x, pos.y, pos.z, stage.min(9) as u8));
            }
        }
        let key: String = list
            .iter()
            .map(|c| format!("{},{},{},{}|", c.0, c.1, c.2, c.3))
            .collect();
        if !force && key == self.last_cracks_key {
            return;
        }
        self.last_cracks_key = key;
        let mut m = Writer::with_capacity(16 + list.len() * 12);
        let at = m.begin(s::CRACKS);
        m.u16(list.len().min(0xffff) as u16);
        for &(x, y, z, stage) in list.iter().take(0xffff) {
            let state = v.raw_block(x, y, z).unwrap_or(0) as u16;
            m.i32(x)
                .u8(y.clamp(0, 255) as u8)
                .i32(z)
                .u16(state)
                .u8(stage);
        }
        m.end(at);
        let m = m.into_vec();
        self.reliable.bytes(&m);
        self.last_cracks = (!list.is_empty()).then_some(m);
    }

    fn weather(&mut self, force: bool) {
        let rain = if self.rain_level > 0.0 {
            self.rain_level
        } else if self.raining {
            1.0
        } else {
            0.0
        }
        .clamp(0.0, 1.0);
        let thunder = if rain > 0.0 {
            self.thunder_level.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let key = format!("{rain:.2},{thunder:.2}");
        if !force && key == self.last_weather_key {
            return;
        }
        self.last_weather_key = key;
        let mut m = Writer::with_capacity(16);
        let at = m.begin(s::WEATHER);
        m.f32(rain).f32(thunder);
        m.end(at);
        let m = m.into_vec();
        self.reliable.bytes(&m);
        self.last_weather = Some(m);
    }

    fn time_message(&mut self) {
        let Some((at_ms, age, tod)) = self.time else {
            return;
        };
        let mut m = Writer::with_capacity(32);
        let at = m.begin(s::TIME);
        m.f64(at_ms).f64(age).f64(tod);
        m.end(at);
        let m = m.into_vec();
        self.reliable.bytes(&m);
        self.time_msg = Some(m);
    }

    /// Everything a new page needs besides the columns: state, entities, inventory, vitals,
    /// time, weather, cracks, the pose.
    #[must_use]
    pub fn snapshot(&mut self) -> Bytes {
        // the next tick sends a fresh pose
        self.last_pose = None;
        let mut w = Writer::with_capacity(4096);
        w.json(s::STATE, &self.state_json());
        if self.in_game {
            let mut states = Vec::new();
            for (id, k) in &self.known {
                write_spawn(&mut w, *id as u32, k);
                if let Some(key) = &k.skin {
                    write_skin_msg(&mut w, *id as u32, key);
                }
                if k.estate != 0 {
                    states.push((*id as u32, k.estate));
                }
            }
            if let Some(key) = self.skin_sent.get(&SELF) {
                write_skin_msg(&mut w, SELF, key);
            }
            if !states.is_empty() {
                write_states(&mut w, &states);
            }
            if !self.chests.is_empty() {
                let list: Vec<_> = self
                    .chests
                    .iter()
                    .map(|(&(x, y, z), &n)| (x, y, z, n))
                    .collect();
                write_chests(&mut w, &list);
            }
            for m in [
                &self.last_inv,
                &self.last_self_looks,
                &self.last_vitals,
                &self.time_msg,
                &self.last_cracks,
                &self.pose_msg,
            ]
            .into_iter()
            .flatten()
            {
                w.bytes(m);
            }
            match &self.last_weather {
                Some(m) => w.bytes(m),
                None => {
                    let at = w.begin(s::WEATHER);
                    w.f32(0.0).f32(0.0);
                    w.end(at);
                    &mut w
                }
            };
        }
        Bytes::from(w.into_vec())
    }
}

fn write_spawn(w: &mut Writer, id: u32, k: &Known) {
    let at = w.begin(s::SPAWN);
    w.u32(id).u8(k.kind).str(&k.name).str(&k.label);
    w.f64(k.pos[0]).f64(k.pos[1]).f64(k.pos[2]);
    w.f32(yaw_rad(k.rot[0]))
        .f32(pitch_rad(k.rot[1]))
        .f32(yaw_rad(k.rot[2]));
    w.f32(k.size.0)
        .f32(k.size.1)
        .u16(k.item.0)
        .u16(k.item.1)
        .u16(k.variant);
    w.end(at);
    if let Some(l) = &k.looks {
        write_looks(w, id, l);
    }
}

fn write_move(w: &mut Writer, id: u32, k: &Known) {
    w.u32(id).f64(k.pos[0]).f64(k.pos[1]).f64(k.pos[2]);
    w.f32(yaw_rad(k.rot[0]))
        .f32(pitch_rad(k.rot[1]))
        .f32(yaw_rad(k.rot[2]));
}

fn write_move_zero(w: &mut Writer, id: u32) {
    w.u32(id)
        .f64(0.0)
        .f64(0.0)
        .f64(0.0)
        .f32(0.0)
        .f32(0.0)
        .f32(0.0);
}

fn write_skin_msg(w: &mut Writer, id: u32, key: &str) {
    let mut parts = key.splitn(3, '|');
    let flags: u8 = parts.next().and_then(|f| f.parse().ok()).unwrap_or(0);
    let skin = parts.next().unwrap_or("");
    let cape = parts.next().unwrap_or("");
    let at = w.begin(s::SKIN);
    w.u32(id).u8(flags).str(skin).str(cape);
    w.end(at);
}

fn write_states(w: &mut Writer, list: &[(u32, u32)]) {
    let n = list.len().min(0xffff);
    let at = w.begin(s::ESTATE);
    w.u16(n as u16);
    for &(id, v) in &list[..n] {
        w.u32(id).u8((v & 255) as u8).u32(v >> 8);
    }
    w.end(at);
}

fn write_chests(w: &mut Writer, list: &[(i32, i32, i32, u8)]) {
    let n = list.len().min(0xffff);
    let at = w.begin(s::CHESTS);
    w.u16(n as u16);
    for &(x, y, z, v) in &list[..n] {
        w.i32(x).u8(y.clamp(0, 255) as u8).i32(z).u8(v);
    }
    w.end(at);
}
