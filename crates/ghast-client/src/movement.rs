//! The per-tick movement packet (`onUpdateWalkingPlayer` in 1.8, `sendPlayerPacket` /
//! `onUpdateWalkingPlayer` in 1.16) and the stand-in mover used until `ghast-physics` is
//! integrated.

use crate::common::{Controls, EntityAction, MovePacket, Pose, Vec3};

/// Squared distance above which the position is resent (both 1.8 and 1.16).
const POSITION_EPSILON_SQ: f64 = 9.0e-4;
/// The position is resent at least every this many ticks.
const POSITION_REFRESH_TICKS: u32 = 20;

/// Chooses the tick's packets the way vanilla does: sprint change, sneak change, then exactly
/// one of `flying` / `position` / `look` / `position_look`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MovementSender {
    last_pos: Vec3,
    last_yaw: f32,
    last_pitch: f32,
    ticks_since_pos: u32,
    server_sprint: bool,
    server_sneak: bool,
    /// 1.9+: `positionUpdateTicks` is incremented *before* the check (a position at least
    /// every 20 ticks); 1.8 increments after it (every 21st tick).
    increment_first: bool,
}

/// The packets of one movement step, in send order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveStep {
    /// Sprint change (`entity_action`), sent first.
    pub sprint: Option<EntityAction>,
    /// Sneak change (`entity_action`), sent second.
    pub sneak: Option<EntityAction>,
    /// The movement packet.
    pub packet: MovePacket,
}

impl MovementSender {
    /// A fresh 1.8 client player: `lastReported*` are all zero, nothing sprinting or
    /// sneaking; the position is refreshed after 20 ticks without one (every 21st tick).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A fresh 1.9+ client player: like [`MovementSender::new`], but `positionUpdateTicks`
    /// is incremented before the check, so a position goes out at least every 20 ticks
    /// (Grim's BadPacketsE flags a 21-tick gap on these versions).
    #[must_use]
    pub fn new_1_9() -> Self {
        MovementSender {
            increment_first: true,
            ..Self::default()
        }
    }

    /// Ticks since the position was last sent.
    #[must_use]
    pub fn ticks_since_pos(&self) -> u32 {
        self.ticks_since_pos
    }

    /// One tick's packets for the pose after the physics step.
    pub fn update(&mut self, pose: &Pose) -> MoveStep {
        let sprint = (pose.sprinting != self.server_sprint).then(|| {
            self.server_sprint = pose.sprinting;
            if pose.sprinting {
                EntityAction::StartSprinting
            } else {
                EntityAction::StopSprinting
            }
        });
        let sneak = (pose.sneaking != self.server_sneak).then(|| {
            self.server_sneak = pose.sneaking;
            if pose.sneaking {
                EntityAction::StartSneaking
            } else {
                EntityAction::StopSneaking
            }
        });
        let dx = pose.pos.x - self.last_pos.x;
        let dy = pose.pos.y - self.last_pos.y;
        let dz = pose.pos.z - self.last_pos.z;
        // `(double)(rotationYaw - lastReportedYaw) != 0`: an f32 difference.
        let turned = pose.yaw - self.last_yaw != 0.0 || pose.pitch - self.last_pitch != 0.0;
        if self.increment_first {
            self.ticks_since_pos += 1;
        }
        let moved = dx * dx + dy * dy + dz * dz > POSITION_EPSILON_SQ
            || self.ticks_since_pos >= POSITION_REFRESH_TICKS;
        let on_ground = pose.on_ground;
        let packet = match (moved, turned) {
            (true, true) => MovePacket::PositionLook {
                pos: pose.pos,
                yaw: pose.yaw,
                pitch: pose.pitch,
                on_ground,
            },
            (true, false) => MovePacket::Position {
                pos: pose.pos,
                on_ground,
            },
            (false, true) => MovePacket::Look {
                yaw: pose.yaw,
                pitch: pose.pitch,
                on_ground,
            },
            (false, false) => MovePacket::Flying { on_ground },
        };
        if !self.increment_first {
            self.ticks_since_pos += 1;
        }
        if moved {
            self.last_pos = pose.pos;
            self.ticks_since_pos = 0;
        }
        if turned {
            self.last_yaw = pose.yaw;
            self.last_pitch = pose.pitch;
        }
        MoveStep {
            sprint,
            sneak,
            packet,
        }
    }
}

/// The stand-in mover used until `ghast-physics` is integrated: the player never moves.
///
/// It reproduces what the vanilla client computes for a player standing still on a floor
/// right after a (non-relative) teleport: the first tick moves by `motionY = 0` and is not on
/// ground; gravity then makes the next move collide with the floor, so every later tick is on
/// ground. It assumes the server put the player on a floor; without world data it cannot
/// know. Movement keys are ignored (only the sneak key is honoured, it does not move).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StandStill {
    ticks_since_teleport: u32,
}

impl StandStill {
    /// The server teleported the player.
    pub fn teleported(&mut self) {
        self.ticks_since_teleport = 0;
    }

    /// One tick: updates `on_ground`, `sneaking` and `sprinting`; position and look are kept.
    pub fn step(&mut self, pose: &mut Pose, controls: &Controls) {
        pose.on_ground = self.ticks_since_teleport > 0;
        self.ticks_since_teleport = self.ticks_since_teleport.saturating_add(1);
        pose.motion = Vec3::default();
        pose.sneaking = controls.sneak && !pose.flying;
        pose.sprinting = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f64, y: f64, z: f64, yaw: f32, pitch: f32) -> Pose {
        Pose {
            pos: Vec3::new(x, y, z),
            yaw,
            pitch,
            on_ground: true,
            ..Pose::default()
        }
    }

    #[test]
    fn packet_choice() {
        let mut s = MovementSender::new();
        // Fresh player: last reported is the origin, so the first packet carries everything.
        let p = pose(10.5, 64.0, -3.5, 90.0, 10.0);
        assert!(matches!(
            s.update(&p).packet,
            MovePacket::PositionLook { .. }
        ));
        // Standing still: 20 flying packets, then a position refresh (vanilla checks
        // `positionUpdateTicks >= 20` before incrementing it: a 21-tick period).
        for _ in 0..20 {
            assert!(matches!(
                s.update(&p).packet,
                MovePacket::Flying { on_ground: true }
            ));
        }
        assert!(matches!(s.update(&p).packet, MovePacket::Position { .. }));
        // A tiny move below the threshold is not sent; a look change is.
        let p2 = pose(10.51, 64.0, -3.5, 91.0, 10.0);
        assert!(matches!(s.update(&p2).packet, MovePacket::Look { .. }));
        let p3 = pose(10.6, 64.0, -3.5, 91.0, 10.0);
        assert!(matches!(s.update(&p3).packet, MovePacket::Position { .. }));
    }

    #[test]
    fn position_refresh_1_9() {
        let mut s = MovementSender::new_1_9();
        let p = pose(10.5, 64.0, -3.5, 90.0, 10.0);
        assert!(matches!(
            s.update(&p).packet,
            MovePacket::PositionLook { .. }
        ));
        for _ in 0..19 {
            assert!(matches!(s.update(&p).packet, MovePacket::Flying { .. }));
        }
        assert!(matches!(s.update(&p).packet, MovePacket::Position { .. }));
    }

    #[test]
    fn sneak_and_sprint_order() {
        let mut s = MovementSender::new();
        let mut p = pose(0.0, 0.0, 0.0, 0.0, 0.0);
        p.sneaking = true;
        p.sprinting = true;
        let st = s.update(&p);
        assert_eq!(st.sprint, Some(EntityAction::StartSprinting));
        assert_eq!(st.sneak, Some(EntityAction::StartSneaking));
        let st = s.update(&p);
        assert_eq!((st.sprint, st.sneak), (None, None));
        p.sneaking = false;
        assert_eq!(s.update(&p).sneak, Some(EntityAction::StopSneaking));
    }

    #[test]
    fn stand_still_ground() {
        let mut m = StandStill::default();
        let mut p = Pose::default();
        m.teleported();
        m.step(&mut p, &Controls::default());
        assert!(!p.on_ground);
        m.step(&mut p, &Controls::default());
        assert!(p.on_ground);
    }
}
