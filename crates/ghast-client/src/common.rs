//! The version-agnostic layer: what the client core receives from and sends to the server,
//! independent of the protocol version.
//!
//! Each supported version (see [`crate::version`]) maps its generated packets to [`Inbound`]
//! and [`Outbound`] values. Only the packets the core or the behaviour needs are mapped; the
//! rest is dropped by the frame decoder's filter before it is decoded.

use bytes::Bytes;
use ghast_protocol::types::ByteString;
use uuid::Uuid;

/// A position or direction in world space.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec3 {
    /// East (+) / west (−).
    pub x: f64,
    /// Up (+) / down (−).
    pub y: f64,
    /// South (+) / north (−).
    pub z: f64,
}

impl Vec3 {
    /// A vector from its components.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Vec3 { x, y, z }
    }
}

/// The player's pose: feet position, look, motion and movement flags.
///
/// `yaw` / `pitch` are degrees in vanilla's convention (yaw is an unbounded `f32` like the
/// client's `rotationYaw`; 0 = south, 90 = west; pitch 90 = straight down).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pose {
    /// Feet position (bounding box minimum Y).
    pub pos: Vec3,
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// Motion per tick.
    pub motion: Vec3,
    /// On ground as the client computes it (and sends it).
    pub on_ground: bool,
    /// Creative / ability flight.
    pub flying: bool,
    /// Sneaking (as sent with `entity_action`).
    pub sneaking: bool,
    /// Sprinting (as sent with `entity_action`).
    pub sprinting: bool,
}

impl Pose {
    /// Eye position: feet + 1.62 (1.54 while sneaking in 1.8; 1.16.5 uses a crouching pose
    /// with eyes at 1.27, see [`crate::GameVersion::sneak_eye_height`]).
    #[must_use]
    pub fn eyes(&self, eye_height: f64) -> Vec3 {
        Vec3::new(self.pos.x, self.pos.y + eye_height, self.pos.z)
    }
}

/// Movement keys held for the next movement step (the keyboard state in vanilla).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Controls {
    /// Walk forward (W).
    pub forward: bool,
    /// Walk backward (S).
    pub back: bool,
    /// Strafe left (A).
    pub left: bool,
    /// Strafe right (D).
    pub right: bool,
    /// Jump (space).
    pub jump: bool,
    /// Sneak (shift).
    pub sneak: bool,
    /// Sprint key (ctrl).
    pub sprint: bool,
}

impl Controls {
    /// No key held.
    pub const NONE: Controls = Controls {
        forward: false,
        back: false,
        left: false,
        right: false,
        jump: false,
        sneak: false,
        sprint: false,
    };
}

/// Where a chat message is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatPosition {
    /// A player chat message.
    Chat,
    /// A system message (command feedback, server announcements).
    System,
    /// The action bar above the hotbar.
    ActionBar,
    /// A value this client does not know.
    Other(i8),
}

impl ChatPosition {
    /// From the wire byte (same values in every supported version).
    #[must_use]
    pub fn from_wire(v: i8) -> Self {
        match v {
            0 => Self::Chat,
            1 => Self::System,
            2 => Self::ActionBar,
            v => Self::Other(v),
        }
    }
}

/// A chat message from the server. The text stays raw JSON until asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// The chat component as the server sent it (JSON).
    pub json: ByteString,
    /// Where it is shown.
    pub position: ChatPosition,
    /// Sender UUID (1.16+; `None` on 1.8 and for system messages with the nil UUID).
    pub sender: Option<Uuid>,
}

impl ChatMessage {
    /// The message as plain text, with legacy `§` formatting codes kept.
    #[must_use]
    pub fn text(&self) -> String {
        ghast_protocol::chat::to_plain_text(&self.json)
    }

    /// The message as plain text without `§` formatting codes.
    #[must_use]
    pub fn plain(&self) -> String {
        ghast_protocol::chat::strip_formatting(&self.text())
    }
}

/// Converts a chat component (JSON) to plain text without formatting codes.
#[must_use]
pub fn chat_to_plain(json: &str) -> String {
    ghast_protocol::chat::strip_formatting(&ghast_protocol::chat::to_plain_text(json))
}

/// Game mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameMode {
    /// Survival.
    #[default]
    Survival,
    /// Creative.
    Creative,
    /// Adventure.
    Adventure,
    /// Spectator.
    Spectator,
    /// A value this client does not know.
    Other(u8),
}

impl GameMode {
    /// From the wire value (the hardcore bit 0x08 of 1.8 is ignored).
    #[must_use]
    pub fn from_wire(v: u8) -> Self {
        match v & 0x07 {
            0 => Self::Survival,
            1 => Self::Creative,
            2 => Self::Adventure,
            3 => Self::Spectator,
            _ => Self::Other(v),
        }
    }
}

/// Player abilities (`abilities` packet).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Abilities {
    /// Raw flags: 1 invulnerable, 2 flying, 4 may fly, 8 creative (instant break).
    pub flags: i8,
    /// Flying speed (0.05 by default).
    pub flying_speed: f32,
    /// Walking speed (0.1 by default).
    pub walking_speed: f32,
}

impl Default for Abilities {
    fn default() -> Self {
        Abilities {
            flags: 0,
            flying_speed: 0.05,
            walking_speed: 0.1,
        }
    }
}

impl Abilities {
    /// The server allows flight.
    #[must_use]
    pub fn may_fly(&self) -> bool {
        self.flags & 4 != 0
    }

    /// The server says the player is flying.
    #[must_use]
    pub fn flying(&self) -> bool {
        self.flags & 2 != 0
    }
}

/// Health, food and saturation (`update_health`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Health {
    /// Health points (20 = full; ≤ 0 = dead).
    pub health: f32,
    /// Food level 0 … 20.
    pub food: i32,
    /// Food saturation.
    pub saturation: f32,
}

impl Default for Health {
    fn default() -> Self {
        Health {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        }
    }
}

/// Relative-teleport flag bits of the server's position packet.
pub mod teleport_flags {
    /// X is relative.
    pub const X: i8 = 0x01;
    /// Y is relative.
    pub const Y: i8 = 0x02;
    /// Z is relative.
    pub const Z: i8 = 0x04;
    /// Yaw is relative.
    pub const YAW: i8 = 0x08;
    /// Pitch is relative.
    pub const PITCH: i8 = 0x10;
}

/// A server teleport (position packet), before it is applied to the pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Teleport {
    /// X (absolute or relative, see `flags`).
    pub x: f64,
    /// Y.
    pub y: f64,
    /// Z.
    pub z: f64,
    /// Yaw.
    pub yaw: f32,
    /// Pitch.
    pub pitch: f32,
    /// [`teleport_flags`] bits.
    pub flags: i8,
    /// Teleport id to confirm (1.9+).
    pub teleport_id: Option<i32>,
}

/// Login-state packets from the server, common to every version.
#[derive(Debug, Clone, PartialEq)]
pub enum LoginInbound {
    /// The server refused the login (chat JSON).
    Disconnect(ByteString),
    /// Online mode: the server asks for encryption.
    EncryptionRequest {
        /// Server id (usually empty) for the session hash.
        server_id: ByteString,
        /// The server's RSA public key (DER).
        public_key: Bytes,
        /// Token to encrypt and send back.
        verify_token: Bytes,
    },
    /// Compression starts with the next packet.
    Compress(i32),
    /// Login finished; the connection is in the play state.
    Success {
        /// The player's UUID as the server assigned it.
        uuid: Option<Uuid>,
        /// The player's name.
        username: ByteString,
    },
    /// A login plugin request (1.13+): answered with "not understood".
    PluginRequest {
        /// Message id to answer.
        message_id: i32,
        /// Channel name.
        channel: ByteString,
    },
}

/// Static information from the join game packet.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinGame {
    /// The player's entity id.
    pub entity_id: i32,
    /// Game mode.
    pub game_mode: GameMode,
    /// Dimension: the 1.8 id (−1 nether, 0 overworld, 1 end) as a string, or the 1.16 world
    /// name (e.g. `minecraft:overworld`).
    pub dimension: String,
    /// Hardcore world.
    pub hardcore: bool,
    /// 1.16: the dimension codec and dimension type NBT, kept raw (not parsed).
    pub dimension_nbt: Option<(ghast_protocol::nbt::RawNbt, ghast_protocol::nbt::RawNbt)>,
    /// 1.16: the respawn screen is shown (false = immediate respawn gamerule).
    pub respawn_screen: bool,
}

/// Clientbound play packets the core understands, common to every version.
#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    /// Keep-alive to echo.
    KeepAlive(i64),
    /// Join game (also sent again on a proxy server switch).
    JoinGame(JoinGame),
    /// Respawn / dimension change.
    Respawn {
        /// Dimension key (see [`JoinGame::dimension`]).
        dimension: String,
        /// Game mode after the respawn.
        game_mode: GameMode,
    },
    /// Server teleport / position correction.
    Teleport(Teleport),
    /// Chat message.
    Chat(ChatMessage),
    /// The server ends the session (chat JSON).
    Disconnect(ByteString),
    /// Health, food and saturation.
    Health(Health),
    /// Abilities.
    Abilities(Abilities),
    /// The server selects a hotbar slot.
    HeldSlot(i8),
    /// Game state change (`reason`, `value`), e.g. reason 3 = game mode.
    GameState {
        /// Reason id (same numbering in all supported versions).
        reason: u8,
        /// Value.
        value: f32,
    },
    /// Window confirm transaction.
    Transaction {
        /// Window id.
        window_id: i8,
        /// Action number.
        action: i16,
        /// Accepted.
        accepted: bool,
    },
    /// A window opened.
    OpenWindow(i32),
    /// The open window was closed by the server.
    CloseWindow(i32),
    /// Compression threshold change in the play state (1.8).
    SetCompression(i32),
    /// Entity velocity (blocks per tick; the wire's 1/8000 units converted).
    Velocity {
        /// The entity.
        entity_id: i32,
        /// Motion per tick.
        motion: Vec3,
    },
    /// The player motion an explosion adds.
    ExplosionPush(Vec3),
    /// Movement speed attribute of an entity (entity properties packet).
    MovementSpeed {
        /// The entity.
        entity_id: i32,
        /// Base value.
        base: f64,
        /// Modifiers `(uuid, amount, operation)` in packet order.
        modifiers: Vec<(Uuid, f64, i8)>,
    },
    /// A status effect was applied to an entity.
    Effect {
        /// The entity.
        entity_id: i32,
        /// Effect id.
        id: i8,
        /// Amplifier (level − 1).
        amplifier: i8,
    },
    /// A status effect ended.
    EffectRemoved {
        /// The entity.
        entity_id: i32,
        /// Effect id.
        id: i8,
    },
    /// Entity status (e.g. 9 = our item use finished).
    EntityStatus {
        /// The entity.
        entity_id: i32,
        /// Status id.
        status: i8,
    },
    /// An entity (the player, if it is our id) died (1.16 `combat_event` "entity dead").
    Died {
        /// The dead player's entity id.
        entity_id: i32,
    },
}

/// Sneak / sprint `entity_action` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityAction {
    /// Start sneaking.
    StartSneaking,
    /// Stop sneaking.
    StopSneaking,
    /// Start sprinting.
    StartSprinting,
    /// Stop sprinting.
    StopSprinting,
}

/// The one movement packet of a tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MovePacket {
    /// `flying` (C03): only on-ground.
    Flying {
        /// On ground.
        on_ground: bool,
    },
    /// `position` (C04).
    Position {
        /// Feet position.
        pos: Vec3,
        /// On ground.
        on_ground: bool,
    },
    /// `look` (C05).
    Look {
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// On ground.
        on_ground: bool,
    },
    /// `position_look` (C06).
    PositionLook {
        /// Feet position.
        pos: Vec3,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// On ground.
        on_ground: bool,
    },
}

impl MovePacket {
    /// The 1.8 trace name (`C03` … `C06`).
    #[must_use]
    pub fn trace_id(&self) -> &'static str {
        match self {
            Self::Flying { .. } => "C03",
            Self::Position { .. } => "C04",
            Self::Look { .. } => "C05",
            Self::PositionLook { .. } => "C06",
        }
    }
}

/// Client settings (`settings` packet), sent right after join game like vanilla.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientSettings {
    /// Locale, e.g. `en_US` (1.8) / `en_us` (1.16).
    pub locale: String,
    /// View distance in chunks.
    pub view_distance: i8,
    /// Chat visibility: 0 full, 1 commands only, 2 hidden.
    pub chat_mode: i8,
    /// Chat colours.
    pub chat_colors: bool,
    /// Displayed skin parts (bit mask; 0x7f = all).
    pub skin_parts: u8,
    /// Main hand (1.9+): 0 left, 1 right.
    pub main_hand: i32,
}

impl Default for ClientSettings {
    fn default() -> Self {
        ClientSettings {
            locale: "en_US".to_owned(),
            view_distance: 8,
            chat_mode: 0,
            chat_colors: true,
            skin_parts: 0x7f,
            main_hand: 1,
        }
    }
}

/// A block face, as in the dig / place packets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Face {
    /// −Y (0).
    Down,
    /// +Y (1).
    Up,
    /// −Z (2).
    North,
    /// +Z (3).
    South,
    /// −X (4).
    West,
    /// +X (5).
    East,
}

impl Face {
    /// The wire value 0 … 5.
    #[must_use]
    pub const fn id(self) -> i8 {
        match self {
            Face::Down => 0,
            Face::Up => 1,
            Face::North => 2,
            Face::South => 3,
            Face::West => 4,
            Face::East => 5,
        }
    }

    /// From the wire value.
    #[must_use]
    pub const fn from_id(id: i8) -> Option<Face> {
        Some(match id {
            0 => Face::Down,
            1 => Face::Up,
            2 => Face::North,
            3 => Face::South,
            4 => Face::West,
            5 => Face::East,
            _ => return None,
        })
    }

    /// The neighbouring block in this direction.
    #[must_use]
    pub const fn offset(self, p: ghast_protocol::BlockPos) -> ghast_protocol::BlockPos {
        let (x, y, z) = match self {
            Face::Down => (0, -1, 0),
            Face::Up => (0, 1, 0),
            Face::North => (0, 0, -1),
            Face::South => (0, 0, 1),
            Face::West => (-1, 0, 0),
            Face::East => (1, 0, 0),
        };
        ghast_protocol::BlockPos::new(p.x + x, p.y + y, p.z + z)
    }
}

/// The `use_entity` action.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EntityUse {
    /// Right click (0).
    Interact,
    /// Left click (1).
    Attack,
    /// Right click at a point relative to the entity position (2).
    InteractAt(Vec3),
}

/// `block_dig` status values used by the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DigStatus {
    /// Start destroying a block (0).
    Start,
    /// Abort destroying (1).
    Abort,
    /// Finish destroying (2).
    Finish,
    /// Release the used item: stop eating / blocking / drawing (5).
    ReleaseUseItem,
}

impl DigStatus {
    /// The wire value.
    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            DigStatus::Start => 0,
            DigStatus::Abort => 1,
            DigStatus::Finish => 2,
            DigStatus::ReleaseUseItem => 5,
        }
    }
}

/// Serverbound play packets the core writes, common to every version.
#[derive(Debug, Clone, PartialEq)]
pub enum Outbound {
    /// Keep-alive answer.
    KeepAlive(i64),
    /// Teleport confirm (1.9+; never produced for 1.8).
    TeleportConfirm(i32),
    /// The movement packet.
    Move(MovePacket),
    /// Sneak / sprint change.
    EntityAction {
        /// Our entity id.
        entity_id: i32,
        /// The action.
        action: EntityAction,
    },
    /// Chat line or command (already truncated to the version's limit).
    Chat(String),
    /// Respawn request (client command "perform respawn").
    Respawn,
    /// Window confirm transaction answer.
    Transaction {
        /// Window id.
        window_id: i8,
        /// Action number.
        action: i16,
        /// Accepted.
        accepted: bool,
    },
    /// Abilities (sent when the client toggles flight).
    Abilities {
        /// Flags (1 invulnerable, 2 flying, 4 may fly, 8 creative).
        flags: i8,
        /// Flying speed.
        flying_speed: f32,
        /// Walking speed.
        walking_speed: f32,
    },
    /// Select a hotbar slot (0 … 8).
    HeldSlot(i16),
    /// Arm swing (main hand).
    Swing,
    /// `block_dig`.
    Dig {
        /// Status.
        status: DigStatus,
        /// Block.
        pos: ghast_protocol::BlockPos,
        /// Face.
        face: Face,
    },
    /// `block_place` on a block face (1.8 with the held stack; 1.16 main hand).
    Place {
        /// The block clicked.
        pos: ghast_protocol::BlockPos,
        /// The face clicked.
        face: Face,
        /// Hit point relative to the block (floats, 0 … 1).
        cursor: Vec3,
        /// The held stack (1.8 sends it; vanilla servers / Grim compare it with the hand).
        held: Option<ghast_inventory::Stack>,
        /// 1.9+: the off hand.
        off_hand: bool,
    },
    /// Use the held item (1.8 `block_place` at −1/−1/−1 face 255 with the held stack; 1.16
    /// `use_item`).
    UseItem {
        /// The held stack.
        held: Option<ghast_inventory::Stack>,
        /// 1.9+: the off hand.
        off_hand: bool,
    },
    /// 1.9+: off-hand arm swing.
    SwingOffHand,
    /// `use_entity`: attack or interact with an entity.
    UseEntity {
        /// The entity.
        target: i32,
        /// What to do.
        action: EntityUse,
        /// 1.9+: the off hand (interactions).
        off_hand: bool,
        /// 1.16: the sneak key is held.
        sneaking: bool,
    },
    /// Client settings.
    Settings(ClientSettings),
    /// A packet of the inventory (window click, confirm answer, close, open inventory).
    Inventory(ghast_inventory::Outgoing),
    /// The client brand plugin message (`MC|Brand` on 1.8, `minecraft:brand` on 1.13+).
    Brand(String),
}
