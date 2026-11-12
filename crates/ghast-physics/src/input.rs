//! Version-independent per-tick input and the movement packets a tick produces.

/// Movement keys held during a tick (what the keyboard state would be in vanilla).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Keys {
    /// Walk forward (W).
    pub forward: bool,
    /// Walk backward (S).
    pub back: bool,
    /// Strafe left (A).
    pub left: bool,
    /// Strafe right (D).
    pub right: bool,
    /// Jump (space): jumps, swims up, flies up, double-tap toggles creative flight.
    pub jump: bool,
    /// Sneak (shift).
    pub sneak: bool,
    /// Sprint key (ctrl).
    pub sprint: bool,
}

impl Keys {
    /// No key held.
    pub const NONE: Keys = Keys {
        forward: false,
        back: false,
        left: false,
        right: false,
        jump: false,
        sneak: false,
        sprint: false,
    };
}

/// Everything the player controls for one tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TickInput {
    /// Held keys.
    pub keys: Keys,
    /// Yaw set before the tick (vanilla applies mouse look between ticks). For packets to pass
    /// rotation checks it should be a whole number of mouse steps from the last value; see
    /// [`crate::rotation`].
    pub yaw: f32,
    /// Pitch set before the tick, `-90..=90`.
    pub pitch: f32,
    /// An item is in use (eating, drinking, blocking with a sword, drawing a bow): movement input
    /// is scaled by 0.2, sprinting cannot start and the sprint double-tap window resets. The
    /// use / release packets are the client's; this is only the movement effect.
    pub using_item: bool,
}

/// The player abilities as both directions of the abilities packet carry them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Abilities {
    /// Takes no damage.
    pub invulnerable: bool,
    /// Currently flying.
    pub flying: bool,
    /// May fly (creative / spectator, or granted by the server).
    pub allow_flying: bool,
    /// Creative mode (instant break etc.; no effect on movement).
    pub creative: bool,
    /// Flying speed (default 0.05).
    pub fly_speed: f32,
    /// Walking speed (default 0.1; the server turns it into the movement speed attribute).
    pub walk_speed: f32,
}

impl Default for Abilities {
    fn default() -> Self {
        Abilities {
            invulnerable: false,
            flying: false,
            allow_flying: false,
            creative: false,
            fly_speed: 0.05,
            walk_speed: 0.1,
        }
    }
}

/// A movement-related packet the client sends, in the order the tick produced it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Packet {
    /// Entity action "start sneaking".
    StartSneaking,
    /// Entity action "stop sneaking".
    StopSneaking,
    /// Entity action "start sprinting".
    StartSprinting,
    /// Entity action "stop sprinting".
    StopSprinting,
    /// Abilities update (sent when the client toggles flight).
    Abilities(Abilities),
    /// Ground flag only.
    Flying {
        /// On ground.
        on_ground: bool,
    },
    /// Position (`y` is the bounding box bottom).
    Position {
        /// X.
        x: f64,
        /// Feet Y (bounding box minimum).
        y: f64,
        /// Z.
        z: f64,
        /// On ground.
        on_ground: bool,
    },
    /// Rotation.
    Look {
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// On ground.
        on_ground: bool,
    },
    /// A modern (26.x) movement packet: position and / or rotation (both absent = status only),
    /// on-ground and horizontal-collision flags.
    Move {
        /// Position (feet), when included.
        pos: Option<[f64; 3]>,
        /// `(yaw, pitch)`, when included.
        look: Option<(f32, f32)>,
        /// On ground.
        on_ground: bool,
        /// Horizontal collision.
        horizontal_collision: bool,
    },
    /// The modern `player_input` packet: the key state, sent when it changes.
    PlayerInput(Keys),
    /// The 26.1 `player_loaded` packet: the client has loaded the level (see
    /// `modern::LoadGate`).
    PlayerLoaded,
    /// The modern `player_command` `START_FALL_FLYING`: the client started gliding (see
    /// `modern::PlayerExt`).
    StartFallFlying,
    /// The modern `vehicle_move` packet: the position and rotation of the vehicle the player
    /// controls (see `modern::Boat`).
    MoveVehicle {
        /// Position.
        pos: [f64; 3],
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// The vehicle's on-ground flag.
        on_ground: bool,
    },
    /// The modern `steer_boat` (paddle) packet: the boat's paddle states, sent every boat tick.
    PaddleBoat {
        /// Left paddle.
        left: bool,
        /// Right paddle.
        right: bool,
    },
    /// Position and rotation.
    PositionLook {
        /// X.
        x: f64,
        /// Feet Y (bounding box minimum).
        y: f64,
        /// Z.
        z: f64,
        /// Yaw.
        yaw: f32,
        /// Pitch.
        pitch: f32,
        /// On ground.
        on_ground: bool,
    },
}

/// Capacity of [`Packets`]: a tick sends at most two abilities updates, two entity actions and
/// one movement packet.
const MAX_PACKETS: usize = 8;

/// The packets of one tick, in send order. Fixed capacity, no allocation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Packets {
    buf: [Packet; MAX_PACKETS],
    len: usize,
}

impl Default for Packets {
    fn default() -> Self {
        Packets {
            buf: [Packet::Flying { on_ground: false }; MAX_PACKETS],
            len: 0,
        }
    }
}

impl Packets {
    /// An empty list.
    #[must_use]
    pub fn new() -> Self {
        Packets::default()
    }

    /// Removes all packets.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Appends a packet.
    ///
    /// # Panics
    /// When more than the fixed capacity is pushed (a tick never does).
    pub fn push(&mut self, p: Packet) {
        assert!(self.len < MAX_PACKETS, "packet list overflow");
        self.buf[self.len] = p;
        self.len += 1;
    }

    /// The packets in send order.
    #[must_use]
    pub fn as_slice(&self) -> &[Packet] {
        &self.buf[..self.len]
    }

    /// Number of packets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// `true` when empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<'a> IntoIterator for &'a Packets {
    type Item = &'a Packet;
    type IntoIter = std::slice::Iter<'a, Packet>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}
