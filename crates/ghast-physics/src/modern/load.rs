//! The 26.1 level-load gate: after login and every respawn the client player does not tick until
//! the client reports the level loaded (`ServerboundPlayerLoadedPacket`).
//!
//! Vanilla (`LevelLoadTracker`, `ClientPacketListener.tick` / `hasClientLoaded`,
//! `LocalPlayer.tick`), proven by the oracle's `joins.jsonl.gz`:
//! - `handleLogin` / `handleRespawn` start waiting for the server; the server's game event
//!   `LEVEL_CHUNKS_LOAD_START` (13) moves the client on to waiting for the player's chunk.
//! - Every client tick, in `MultiPlayerGameMode.tick` (before the entities tick), the tracker
//!   checks: the 30 s timeout since the wait started ran out, or the player's feet block is outside
//!   the build height, or the player is a spectator or dead, or the level renderer has compiled the
//!   player's section. The first tick that holds sends `player_loaded` at once (no close delay in
//!   multiplayer) and the player ticks normally in the same tick.
//! - Until then `LocalPlayer.tick` does nothing: no physics, no movement packets, no input; only
//!   the entity tick counter runs, and every tick still ends with `client_tick_end`. Position
//!   packets are answered as usual (teleport confirm + position/look with on-ground false).
//! - The loading screen holds the mouse: the player does not turn before the tick after the gate.
//!
//! A headless client has no renderer: "section compiled" is the caller's decision (e.g. the
//! player's chunk column and its neighbours received). The gate order on the wire is
//! `player_loaded`, then the first movement packet of that tick, then `client_tick_end`.

use super::player::{PlayerState, tick};
use super::rules::Rules;
use crate::input::{Packet, Packets, TickInput};
use crate::world::BlockView;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Stage {
    WaitingForServer,
    WaitingForChunk,
    Loaded,
}

/// The client's level-load state (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LoadGate {
    stage: Stage,
    timed_out: bool,
    dead: bool,
}

impl Default for LoadGate {
    fn default() -> Self {
        LoadGate::waiting()
    }
}

impl LoadGate {
    /// Waiting for the server (after `login` / `respawn`).
    #[must_use]
    pub const fn waiting() -> Self {
        LoadGate {
            stage: Stage::WaitingForServer,
            timed_out: false,
            dead: false,
        }
    }

    /// Already loaded.
    #[must_use]
    pub const fn loaded() -> Self {
        LoadGate {
            stage: Stage::Loaded,
            timed_out: false,
            dead: false,
        }
    }

    /// The server's `game_event` 13 (`LEVEL_CHUNKS_LOAD_START`).
    pub fn level_chunks_load_start(&mut self) {
        if self.stage == Stage::WaitingForServer {
            self.stage = Stage::WaitingForChunk;
        }
    }

    /// 30 s have passed since [`LoadGate::waiting`] (the client's wall-clock timeout; it only lets
    /// the player in once the server's load start arrived).
    pub fn time_out(&mut self) {
        self.timed_out = true;
    }

    /// Whether the player is alive (`isAlive()`: health above 0, from `set_health`). A dead
    /// player does not wait for its section. Alive by default.
    pub fn player_alive(&mut self, alive: bool) {
        self.dead = !alive;
    }

    /// `hasClientLoaded()`.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        self.stage == Stage::Loaded
    }

    /// The gate's part of the client tick (`LevelLoadTracker.tickClientLoad` + `isLevelReady`),
    /// run after the tick's packets and before the player ticks. `section_compiled`: the
    /// player's section counts as rendered. Returns `true` on the tick the client sends
    /// `player_loaded`.
    pub fn tick(&mut self, p: &PlayerState, section_compiled: bool) -> bool {
        if self.stage != Stage::WaitingForChunk {
            return false;
        }
        let y = p.pos[1].floor();
        let outside =
            y < f64::from(p.level.min_y) || y >= f64::from(p.level.min_y + p.level.height);
        if self.timed_out || outside || p.spectator || self.dead || section_compiled {
            self.stage = Stage::Loaded;
            return true;
        }
        false
    }

    /// One whole client tick of the player behind the gate: the gate tick (pushes
    /// [`Packet::PlayerLoaded`] on the gate tick), then [`tick`] if loaded (without the mouse turn
    /// on the gate tick itself), else only the entity tick counter. `client_tick_end` is the
    /// caller's, after this.
    pub fn step<R: Rules, W: BlockView<State = R::State>>(
        &mut self,
        p: &mut PlayerState,
        input: &TickInput,
        world: &W,
        section_compiled: bool,
        out: &mut Packets,
    ) {
        let was_loaded = self.is_loaded();
        if self.tick(p, section_compiled) {
            out.push(Packet::PlayerLoaded);
        }
        if !self.is_loaded() {
            p.age += 1;
            return;
        }
        if was_loaded {
            tick::<R, W>(p, input, world, out);
        } else {
            let still = TickInput {
                yaw: p.yaw,
                pitch: p.pitch,
                ..*input
            };
            tick::<R, W>(p, &still, world, out);
        }
    }
}
