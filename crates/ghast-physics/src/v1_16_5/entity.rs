//! Other entities around the 1.16.5 player: the ones it collides with (boats, shulkers) and the
//! ones whose client tick pushes it (mobs, other players, boats).
//!
//! # As the 1.16.5 client does it
//!
//! * **Collision.** In its own move the player collides with the boxes of the entities it
//!   `collidesWith` (`isCollidable`: boats, and shulkers while alive) like with blocks, also for
//!   the step-up and for every "is this space empty" test (pose changes, sneaking at edges,
//!   jumping out of water). Pass the boxes as they are when the player ticks in
//!   [`Surroundings::entities`] to [`tick_in`](super::tick_in).
//! * **Pushing.** On the client only the local player is pushable (`EntityPredicates.canBePushedBy`
//!   lets no other entity through there), and the player's own push finds nothing: it is the
//!   other entities' client ticks that push it. A living entity's `tickMovement` ends with
//!   `tickCramming` over its own box (another player's `OtherClientPlayerEntity.tickMovement`
//!   too); a boat's tick pushes what is in its box grown by 0.2 horizontally (and shrunk by 0.01
//!   vertically) when the player's feet are not above the boat's bottom. Call [`push_by`] for
//!   each such entity in the client's entity tick order (the iteration order of the client
//!   world's entity map), with its position and box after its own tick. [`push_kind`] says
//!   which entity types push.
//! * Team collision rules (`TeamS2CPacket` `collisionRule`) are the caller's: with a rule other
//!   than `always` on either team, skip what it forbids.

// Vanilla's negated float comparisons are kept as written: they decide the NaN cases.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use ghast_data::v1_16_5::BlockState;

use super::player::{PlayerState, is_climbing};
use crate::math::{Aabb, java_max, java_min};
use crate::voxel::EPSILON;
use crate::world::BlockView;

/// What else is around the player for one tick. Default: nothing.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct Surroundings<'a> {
    /// Boxes of the entities the player collides with, as they are when the player ticks
    /// (any order; vanilla's `getOtherEntities` order only matters at the `1e-7` tolerance).
    pub entities: &'a [Aabb],
}

impl<'a> Surroundings<'a> {
    /// Nothing around.
    #[must_use]
    pub fn new() -> Surroundings<'a> {
        Surroundings::default()
    }

    /// With these collidable entity boxes.
    #[must_use]
    pub fn with_entities(mut self, entities: &'a [Aabb]) -> Surroundings<'a> {
        self.entities = entities;
        self
    }
}

/// How an entity's client tick pushes the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PushKind {
    /// `LivingEntity.tickCramming`: the player overlaps the entity's box.
    Living,
    /// `BoatEntity.tick`: the player overlaps the boat's box grown by 0.2 horizontally and
    /// shrunk by 0.01 vertically, and its feet are not above the boat's bottom.
    Boat,
}

/// An entity whose tick may push the player: its position and box after its tick.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Pusher {
    /// Position (`getX()`, `getZ()` decide the direction).
    pub pos: [f64; 3],
    /// Bounding box.
    pub bb: Aabb,
    /// Push rule of its type.
    pub kind: PushKind,
}

impl Pusher {
    /// An entity at `pos` with box `bb` pushing by `kind`.
    #[must_use]
    pub fn new(pos: [f64; 3], bb: Aabb, kind: PushKind) -> Pusher {
        Pusher { pos, bb, kind }
    }
}

/// The push rule of a 1.16.5 entity type (registry name without namespace) for the local player,
/// or `None` when its client tick never pushes the player. `living` is `instanceof
/// LivingEntity` (in 1.16.5: spawned with the living-entity spawn packet, or a player).
///
/// Living entities push, except bats (`tickCramming` does nothing), armor stands (they push
/// only minecarts), parrots (`pushAway` skips players), the ender dragon (its own
/// `tickMovement`) and vexes (no clip while they tick); boats push; nothing else does
/// (minecarts, items, projectiles, ...).
#[must_use]
pub fn push_kind(name: &str, living: bool) -> Option<PushKind> {
    match name {
        "boat" => Some(PushKind::Boat),
        "bat" | "armor_stand" | "parrot" | "ender_dragon" | "vex" => None,
        _ if living => Some(PushKind::Living),
        _ => None,
    }
}

/// `true` for the 1.16.5 entity types the player collides with (`isCollidable`): boats, and a
/// shulker while it is alive (the caller checks that).
#[must_use]
pub fn collides(name: &str) -> bool {
    matches!(name, "boat" | "shulker")
}

/// 1.16 `Box.intersects` (strict, positive overlap).
#[inline]
fn overlaps(a: &Aabb, b: &Aabb) -> bool {
    a.min_x < b.max_x
        && a.max_x > b.min_x
        && a.min_y < b.max_y
        && a.max_y > b.min_y
        && a.min_z < b.max_z
        && a.max_z > b.min_z
}

/// The push of one entity's client tick on the player (`Entity.pushAwayFrom` through the
/// entity's push rule). Returns `true` when the player's velocity changed. The player is
/// pushable when it is not a spectator and not climbing (vanilla also: alive, not riding with
/// the pusher, not sleeping; the caller does not call this then).
///
/// `filed` is where the client world files the player in its chunk entity sections: its
/// position after its last tick (a teleport since does not move it). The pusher finds the
/// player through `World.getOtherEntities`, which only looks in the sections around the pushing
/// area: an entity that ticks before the player does not find a player teleported since its
/// last tick from far away. For the pushes after the player's tick, `filed` is its position.
pub fn push_by<W: BlockView<State = BlockState>>(
    p: &mut PlayerState,
    filed: [f64; 3],
    world: &W,
    other: &Pusher,
) -> bool {
    let area = match other.kind {
        PushKind::Living => other.bb,
        PushKind::Boat => {
            // Box.expand(0.2F, -0.01F, 0.2F).
            let (h, v) = (f64::from(0.2f32), f64::from(-0.01f32));
            super::collide::bx(
                other.bb.min_x - h,
                other.bb.min_y - v,
                other.bb.min_z - h,
                other.bb.max_x + h,
                other.bb.max_y + v,
                other.bb.max_z + h,
            )
        }
    };
    if !overlaps(&p.bb, &area) || !query_reaches(&area, filed) {
        return false;
    }
    // canBePushedBy: no spectators; isPushable: alive, not a spectator, not climbing.
    if p.spectator || is_climbing(p, world) {
        return false;
    }
    // BoatEntity.pushAwayFrom: only what does not stand above the boat's bottom.
    if other.kind == PushKind::Boat && !(p.bb.min_y <= other.bb.min_y) {
        return false;
    }
    // Entity.pushAwayFrom: the player moves away from the other entity (the push speed
    // reduction is 0 for every 1.16.5 entity, a factor of exactly 1).
    let mut d = p.pos[0] - other.pos[0];
    let mut e = p.pos[2] - other.pos[2];
    let mut f = d.abs().max(e.abs());
    if !(f >= f64::from(0.01f32)) {
        return false;
    }
    // MathHelper.sqrt(double) rounds to float.
    f = f64::from(f.sqrt() as f32);
    d /= f;
    e /= f;
    let mut g = 1.0 / f;
    if g > 1.0 {
        g = 1.0;
    }
    d *= g;
    e *= g;
    d *= f64::from(0.05f32);
    e *= f64::from(0.05f32);
    // addVelocity: Vec3d.add (the y sum turns a -0.0 into 0.0).
    p.vel[0] += d;
    p.vel[1] += 0.0;
    p.vel[2] += e;
    true
}

/// `true` when `World.getOtherEntities` over `area` visits the chunk entity section that holds
/// an entity filed at `filed` (`WorldChunk.addEntity`: the section of its position, clamped to
/// the 16 sections): the chunks of the area grown by 2 blocks horizontally, the sections of it
/// grown by 2 blocks vertically.
#[must_use]
pub fn query_reaches(area: &Aabb, filed: [f64; 3]) -> bool {
    use crate::math::floor;
    let chunk = |v: f64| floor(v / 16.0);
    let section = |v: f64| floor(v / 16.0).clamp(0, 15);
    let (cx, cz) = (chunk(filed[0]), chunk(filed[2]));
    let sy = section(filed[1]);
    chunk(area.min_x - 2.0) <= cx
        && cx <= chunk(area.max_x + 2.0)
        && chunk(area.min_z - 2.0) <= cz
        && cz <= chunk(area.max_z + 2.0)
        && section(area.min_y - 2.0) <= sy
        && sy <= section(area.max_y + 2.0)
}

/// `Box.getAverageSideLength`.
#[inline]
pub(crate) fn average_side(b: &Aabb) -> f64 {
    let x = b.max_x - b.min_x;
    let y = b.max_y - b.min_y;
    let z = b.max_z - b.min_z;
    (x + y + z) / 3.0
}

/// `true` when an entity box is an entity collision of a query over `query`
/// (`EntityView.getEntityCollisions`: the query grown by `1e-7`, a strict overlap).
#[inline]
pub(crate) fn collision_hit(query: &Aabb, e: &Aabb) -> bool {
    if average_side(query) < EPSILON {
        return false;
    }
    let q = super::collide::expand(query, EPSILON);
    overlaps(&q, e)
}

#[inline]
fn lo(b: &Aabb, axis: usize) -> f64 {
    [b.min_x, b.min_y, b.min_z][axis]
}

#[inline]
fn hi(b: &Aabb, axis: usize) -> f64 {
    [b.max_x, b.max_y, b.max_z][axis]
}

/// `VoxelShape.calculateMaxDistance` of an entity box (`VoxelShapes.cuboid` of a box outside the
/// unit cube: one cell with explicit boundaries) for a box `b` moving `max` along `axis`.
#[inline]
pub(crate) fn cuboid_max_distance(s: &Aabb, axis: usize, b: &Aabb, max: f64) -> f64 {
    if max.abs() < EPSILON {
        return 0.0;
    }
    let (a2, a3) = ((axis + 1) % 3, (axis + 2) % 3);
    // The cell's layer covers the box on both other axes (the coordinate index search).
    let across = |a: usize| lo(b, a) + EPSILON < hi(s, a) && hi(b, a) - EPSILON >= lo(s, a);
    if !(across(a2) && across(a3)) {
        return max;
    }
    if max > 0.0 {
        if hi(b, axis) - EPSILON < lo(s, axis) {
            return java_min(max, lo(s, axis) - hi(b, axis));
        }
        max
    } else if max < 0.0 {
        if lo(b, axis) + EPSILON >= hi(s, axis) {
            return java_max(max, hi(s, axis) - lo(b, axis));
        }
        max
    } else {
        max
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::{Points, VoxelShape};

    /// The single-cell shortcut equals the generic voxel shape with explicit boundaries.
    #[test]
    fn cuboid_matches_the_generic_shape() {
        let mut seed = 0x1234_5678_9abc_def0_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..200_000 {
            let (x0, y0, z0) = (
                (next() - 0.5) * 6.0,
                (next() - 0.5) * 6.0 + 64.0,
                (next() - 0.5) * 6.0,
            );
            let s = Aabb::new(
                x0,
                y0,
                z0,
                x0 + next() * 2.0 + 0.01,
                y0 + next() * 2.0 + 0.01,
                z0 + next() * 2.0 + 0.01,
            );
            let (bx0, by0, bz0) = (
                (next() - 0.5) * 6.0,
                (next() - 0.5) * 6.0 + 64.0,
                (next() - 0.5) * 6.0,
            );
            let b = Aabb::new(bx0, by0, bz0, bx0 + 0.6, by0 + 1.8, bz0 + 0.6);
            let xs = [s.min_x, s.max_x];
            let ys = [s.min_y, s.max_y];
            let zs = [s.min_z, s.max_z];
            let shape = VoxelShape {
                axes: [
                    Points::Explicit(&xs),
                    Points::Explicit(&ys),
                    Points::Explicit(&zs),
                ],
                cells: &[1],
            };
            for axis in 0..3 {
                for max in [
                    (next() - 0.5) * 6.0,
                    -((next() - 0.5) * 6.0).abs(),
                    ((next() - 0.5) * 6.0).abs(),
                    0.0,
                    1e-8,
                    -1e-8,
                ] {
                    let want = shape.max_distance(axis, &b, max, None);
                    let got = cuboid_max_distance(&s, axis, &b, max);
                    assert_eq!(want.to_bits(), got.to_bits(), "{s:?} {b:?} {axis} {max}");
                }
            }
        }
    }
}
