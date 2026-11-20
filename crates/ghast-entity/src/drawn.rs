//! What the vanilla client draws and picks of the other entities, and a few per-entity clocks
//! (opt-in: `track_client_motion` of each version's tracker).
//!
//! * **1.8.9 and 1.12.2.** A position packet does not move a living entity at once: it sets an
//!   interpolation target and a step count (`updateTrackedPositionAndAngles(.., 3, ..)`), and
//!   each client tick (`LivingEntity.tickMovement`, `OtherClientPlayerEntity.tickMovement`)
//!   moves the entity `1 / steps` of the remaining way and counts down. The crosshair pick
//!   (`GameRenderer.updateTargetedEntity`, at the start of the tick, before the entities tick)
//!   reads that position. Every relative-move, look and bare `entity` packet restarts the 3
//!   steps (a look-only packet towards the current target, rotation-less ones keep the drawn
//!   rotation as the target rotation). A teleport to within 1/32 (y: 1/64) of the drawn position
//!   freezes it there (1.8: 3 steps towards the drawn position, 1.12: 0 steps). Spawns place the
//!   entity at once. Both versions tick every entity of the client world every tick (their
//!   client chunk check passes for any column). A 1.12 shulker sits on its block (the server's
//!   position snapped to the block centre).
//! * **1.16.5** keeps its own client view (`client16`); the drawn position and box are copied
//!   from it.
//! * **26.x** entities *are* the drawn state ([`crate::Entity::position`]); only the clocks live
//!   here.
//!
//! Not modelled (drawn = the server position at once, as before): non-living entities of 1.8 /
//! 1.12 (boats and minecarts interpolate with their own rules; 1.8's base entity is pushed up out
//! of blocks it would overlap), riders (vanilla places them on their vehicle) and the client's
//! own removal of a dead entity after its death animation.

use crate::entity::{Entity, EntityType, Era};

/// How an entity's drawn position follows the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lerp {
    /// `OtherClientPlayerEntity` (yaw wrapped with two loops).
    Player,
    /// `LivingEntity` (yaw wrapped with `MathHelper.wrapDegrees`).
    Living,
    /// 1.12 shulker: the server position's block centre, set by its tick.
    Shulker,
    /// The drawn state is copied from another client view (1.16.5) or is the entity itself
    /// (26.x): only the clocks are kept.
    External,
}

/// The client-side state of one entity (boxed: only living entities with the client view on
/// carry it).
#[derive(Clone, Debug)]
pub(crate) struct Client {
    pub(crate) lerp: Lerp,
    /// Drawn position (`x`, `y`, `z`).
    pub(crate) pos: [f64; 3],
    /// Interpolation target (`serverX` …).
    pub(crate) target: [f64; 3],
    /// Drawn `(yaw, pitch)`.
    pub(crate) rot: [f32; 2],
    /// Target `(yaw, pitch)` (vanilla keeps them as doubles holding float values).
    pub(crate) target_rot: [f32; 2],
    /// Remaining interpolation steps (`bodyTrackingIncrements`).
    pub(crate) steps: u8,
    /// 1.16.5: the client box copied from the client view.
    pub(crate) bb: Option<[f64; 6]>,
    /// Tick the entity spawned.
    pub(crate) spawned: u64,
    /// Tick of the last server position change.
    pub(crate) moved: u64,
    /// Server position at the last change before the tick of [`Client::moved`], and its tick.
    pub(crate) from: [f64; 3],
    pub(crate) from_tick: u64,
    /// Server position after the last change.
    pub(crate) to: [f64; 3],
    /// Tick of the last hurt status / damage event.
    pub(crate) hurt: Option<u64>,
}

/// The legacy (1.8 / 1.12) interpolation kind of `e` (`None`: not interpolated, drawn at the
/// server position).
pub(crate) fn legacy_lerp(e: &Entity) -> Option<Lerp> {
    match e.ty {
        EntityType::Player => Some(Lerp::Player),
        // 1.12 shulker (registry id 69).
        EntityType::LegacyMob(69) if e.era == Era::V1_12 => Some(Lerp::Shulker),
        EntityType::LegacyMob(_) => Some(Lerp::Living),
        // The armor stand is a living entity spawned as object 78.
        EntityType::LegacyObject(78) => Some(Lerp::Living),
        _ => None,
    }
}

impl Client {
    /// A freshly spawned entity: drawn where the server put it. 1.12.2 player and mob spawns
    /// go through `updatePositionAndAngles`, which clamps x / z to ±3e7 and the pitch to ±90
    /// (objects such as the armor stand set their rotation directly).
    pub(crate) fn new(e: &Entity, lerp: Lerp, now: u64) -> Self {
        let (mut pos, mut pitch) = (e.pos, e.pitch);
        if e.era == Era::V1_12 && !matches!(e.ty, EntityType::LegacyObject(_)) {
            // MathHelper.clamp: NaN passes through, as with `clamp`.
            pos = [
                pos[0].clamp(-3.0e7, 3.0e7),
                pos[1],
                pos[2].clamp(-3.0e7, 3.0e7),
            ];
            pitch = pitch.clamp(-90.0, 90.0);
        }
        Client {
            lerp,
            pos,
            target: pos,
            rot: [e.yaw, pitch],
            target_rot: [e.yaw, pitch],
            steps: 0,
            bb: None,
            spawned: now,
            moved: now,
            from: e.pos,
            from_tick: now,
            to: e.pos,
            hurt: None,
        }
    }

    /// The server position became `p` at tick `now` (updates the estimated velocity).
    #[inline]
    pub(crate) fn server_moved(&mut self, p: [f64; 3], now: u64) {
        if p == self.to {
            return;
        }
        if now != self.moved {
            self.from = self.to;
            self.from_tick = self.moved;
        }
        self.to = p;
        self.moved = now;
    }

    /// The interpolating-count change after an update that found it `idle` (no steps left).
    #[inline]
    fn lerp_delta(&self, idle: bool) -> i32 {
        i32::from(self.steps > 0) - i32::from(!idle)
    }

    /// Blocks per tick between the last two server positions.
    pub(crate) fn velocity(&self) -> [f64; 3] {
        let dt = self.moved.saturating_sub(self.from_tick).max(1) as f64;
        [
            (self.to[0] - self.from[0]) / dt,
            (self.to[1] - self.from[1]) / dt,
            (self.to[2] - self.from[2]) / dt,
        ]
    }

    /// `updateTrackedPositionAndAngles(target, rot, steps)`.
    #[inline]
    fn track(&mut self, target: [f64; 3], rot: [f32; 2], steps: u8) {
        self.target = target;
        self.target_rot = rot;
        self.steps = steps;
    }

    /// One client tick (`tickMovement`'s interpolation step; a shulker's block snap). Returns
    /// `true` when the interpolation ended in this step.
    #[inline]
    pub(crate) fn step(&mut self) -> bool {
        match self.lerp {
            Lerp::Player | Lerp::Living if self.steps > 0 => {
                let n = f64::from(self.steps);
                let p = [
                    self.pos[0] + (self.target[0] - self.pos[0]) / n,
                    self.pos[1] + (self.target[1] - self.pos[1]) / n,
                    self.pos[2] + (self.target[2] - self.pos[2]) / n,
                ];
                let mut g = f64::from(self.target_rot[0]) - f64::from(self.rot[0]);
                if self.lerp == Lerp::Player {
                    while g < -180.0 {
                        g += 360.0;
                    }
                    while g >= 180.0 {
                        g -= 360.0;
                    }
                } else {
                    g = wrap_degrees(g);
                }
                let yaw = (f64::from(self.rot[0]) + g / n) as f32;
                let pitch = (f64::from(self.rot[1])
                    + (f64::from(self.target_rot[1]) - f64::from(self.rot[1])) / n)
                    as f32;
                self.steps -= 1;
                self.pos = p;
                // setRotation: `% 360.0F`.
                self.rot = [yaw % 360.0, pitch % 360.0];
                self.steps == 0
            }
            Lerp::Shulker if self.steps > 0 => {
                let [x, y, z] = self.to;
                self.pos = [x.floor() + 0.5, y.floor(), z.floor() + 0.5];
                self.steps = 0;
                true
            }
            _ => false,
        }
    }
}

/// `MathHelper.wrapDegrees(double)` (1.8.9 / 1.12.2).
#[inline]
fn wrap_degrees(d: f64) -> f64 {
    let mut d = d % 360.0;
    if d >= 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d
}

/// A relative move, look or bare `entity` packet was applied to `e` (its server position and,
/// with `rotated`, its rotation are the new ones): `onEntityUpdate`'s
/// `updateTrackedPositionAndAngles(server position, rotation, 3)`, where a packet without a
/// rotation passes the drawn rotation.
///
/// Returns the change of the store's count of interpolating entities (+1 when it starts, −1
/// when it stops, else 0): a tick without any visits nothing.
#[inline]
#[must_use]
pub(crate) fn relative(e: &mut Entity, rotated: bool, now: u64) -> i32 {
    let (pos, rot) = (e.pos, [e.yaw, e.pitch]);
    let Some(c) = e.client.as_deref_mut() else {
        return 0;
    };
    let idle = c.steps == 0;
    c.server_moved(pos, now);
    match c.lerp {
        Lerp::Player | Lerp::Living => {
            let rot = if rotated { rot } else { c.rot };
            c.track(pos, rot, 3);
        }
        // The shulker's next tick snaps it onto the new position's block.
        Lerp::Shulker => c.steps = 1,
        Lerp::External => {}
    }
    c.lerp_delta(idle)
}

/// A teleport was applied to `e`: `onEntityPosition` keeps the drawn position when the new one
/// is within 1/32 (y: 1/64) of it, then `updateTrackedPositionAndAngles(.., 3)` (1.8) or
/// `(.., 0)` for the kept position (1.12).
///
/// Returns the change of the store's count of interpolating entities (see [`relative`]).
#[inline]
#[must_use]
pub(crate) fn teleport(e: &mut Entity, now: u64) -> i32 {
    let (pos, rot, era) = (e.pos, [e.yaw, e.pitch], e.era);
    let Some(c) = e.client.as_deref_mut() else {
        return 0;
    };
    let idle = c.steps == 0;
    c.server_moved(pos, now);
    match c.lerp {
        Lerp::Player | Lerp::Living => {}
        Lerp::Shulker => {
            c.steps = 1;
            return c.lerp_delta(idle);
        }
        Lerp::External => return 0,
    }
    let d = c.pos;
    // `!(a >= t)` as vanilla writes it: NaN counts as near.
    let far = |a: f64, t: f64| a.partial_cmp(&t).is_some_and(|o| o.is_ge());
    let near = !far((d[0] - pos[0]).abs(), 0.031_25)
        && !far((d[1] - pos[1]).abs(), 0.015_625)
        && !far((d[2] - pos[2]).abs(), 0.031_25);
    if near {
        let steps = if era == Era::V1_8 { 3 } else { 0 };
        c.track(d, rot, steps);
    } else {
        c.track(pos, rot, 3);
    }
    c.lerp_delta(idle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(pos: [f64; 3]) -> Entity {
        let mut e = Entity::new(7, EntityType::Player);
        e.pos = pos;
        e.client = Some(Box::new(Client::new(&e, Lerp::Player, 0)));
        e
    }

    fn drawn(e: &Entity) -> [f64; 3] {
        e.client.as_ref().unwrap().pos
    }

    fn tick(e: &mut Entity) {
        let _ = e.client.as_deref_mut().unwrap().step();
    }

    /// Vanilla's three steps: 1/3, then 1/2 of the rest, then the rest; a second packet restarts
    /// the count from the drawn position.
    #[test]
    fn three_steps_and_restart() {
        let mut e = player([0.0, 64.0, 0.0]);
        e.pos = [3.0, 64.0, 0.0];
        let _ = relative(&mut e, false, 1);
        assert_eq!(
            drawn(&e),
            [0.0, 64.0, 0.0],
            "packets do not move the drawn entity"
        );
        tick(&mut e);
        assert_eq!(drawn(&e)[0], 1.0);
        e.pos = [6.0, 64.0, 0.0];
        let _ = relative(&mut e, false, 2);
        tick(&mut e);
        // 1 + (6 - 1) / 3
        assert_eq!(drawn(&e)[0], 1.0 + 5.0 / 3.0);
        tick(&mut e);
        let x = 1.0 + 5.0 / 3.0;
        assert_eq!(drawn(&e)[0], x + (6.0 - x) / 2.0);
        tick(&mut e);
        assert_eq!(drawn(&e)[0], 6.0);
        tick(&mut e);
        assert_eq!(drawn(&e)[0], 6.0);
        let c = e.client.as_ref().unwrap();
        assert_eq!((c.spawned, c.moved, c.from_tick), (0, 2, 1));
        assert_eq!(c.velocity(), [3.0, 0.0, 0.0]);
    }

    /// A look-only packet restarts the 3 steps towards the same target.
    #[test]
    fn look_restarts_steps() {
        let mut e = player([0.0, 64.0, 0.0]);
        e.pos = [3.0, 64.0, 0.0];
        let _ = relative(&mut e, false, 1);
        tick(&mut e);
        e.yaw = 90.0;
        let _ = relative(&mut e, true, 2);
        tick(&mut e);
        assert_eq!(drawn(&e)[0], 1.0 + 2.0 / 3.0);
        let c = e.client.as_ref().unwrap();
        assert_eq!(c.rot[0], 30.0);
        assert_eq!(c.moved, 1, "a look is not a position update");
    }

    /// Teleports near the drawn position freeze it (1.8: 3 steps there, 1.12: 0).
    #[test]
    fn near_teleport_freezes() {
        for (era, steps) in [(Era::V1_8, 3), (Era::V1_12, 0)] {
            let mut e = player([0.0, 64.0, 0.0]);
            e.era = era;
            e.pos = [0.03, 64.0, 0.0];
            let _ = teleport(&mut e, 1);
            let c = e.client.as_ref().unwrap();
            assert_eq!((c.target, c.steps), ([0.0, 64.0, 0.0], steps));
            e.pos = [0.0, 64.015_625, 0.0];
            let _ = teleport(&mut e, 2);
            assert_eq!(e.client.as_ref().unwrap().target, [0.0, 64.015_625, 0.0]);
        }
    }

    /// Yaw takes the short way round; players wrap with loops, living entities with
    /// `wrapDegrees`, and `setRotation` keeps the result below 360 in magnitude.
    #[test]
    fn yaw_wraps() {
        let mut e = player([0.0; 3]);
        e.client.as_deref_mut().unwrap().rot = [170.0, 0.0];
        e.yaw = -170.0;
        let _ = relative(&mut e, true, 1);
        tick(&mut e);
        let y = e.client.as_ref().unwrap().rot[0];
        assert_eq!(y, (170.0f64 + 20.0 / 3.0) as f32);
        for _ in 0..2 {
            tick(&mut e);
        }
        assert_eq!(e.client.as_ref().unwrap().rot[0], 190.0);
        assert_eq!(wrap_degrees(540.0), -180.0);
        assert_eq!(wrap_degrees(-190.0), 170.0);
    }
}
