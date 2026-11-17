//! Other entities around the 1.12.2 player: the ones it collides with (boats, live shulkers) and
//! the ones whose client tick pushes it (living entities, other players, boats).
//!
//! # As the 1.12.2 client does it
//!
//! * **Collision.** `World.doesBoxCollide` adds the box of every entity whose `getBox` is not
//!   null (a boat; a shulker while alive; minecarts return null) to the block boxes of the
//!   query, for the player's move, step-up, sneak edge guard and the water / lava jump-out
//!   test. Pass those boxes, as they are when the player ticks, to
//!   [`tick_in`](super::tick_in). [`collides`] says which entity types count.
//! * **Pushing.** `LivingEntity.tickMovement` ends with `tickCramming`, which pushes every entity
//!   `EntityPredicate.method_13025` lets through; on the client that is only the local player
//!   (`isMainPlayer`). Other players push from `OtherClientPlayerEntity.tickMovement` the same
//!   way, a boat's tick pushes what is in its box grown by 0.2 horizontally and shrunk by 0.01
//!   vertically when the player's feet are not above the boat's bottom
//!   (`BoatEntity.pushAwayFrom`). Bats and armor stands never push the player, parrots skip
//!   players, the ender dragon has its own movement tick. The player's own `tickCramming` finds
//!   nothing on the client. Call [`push_by`] for each pushing entity, in the client's entity tick
//!   order, with its position and box after its tick (entities spawned after the player tick
//!   after it). [`push_kind`] says which entity types push.
//! * **Lily pads.** A boat's tick breaks the lily pads its box touches in the client's world
//!   ([`lily_pads_broken_by_boat`]).
//! * Team collision rules (`teams` `collisionRule`) and a spectator pusher (`noClip`) are the
//!   caller's: skip what they forbid.

// Vanilla's negated float comparisons are kept as written: they decide the NaN cases.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::player::{PlayerState, is_climbing};
use crate::math::{Aabb, BoxOps};
use crate::world::BlockView;
use ghast_data::v1_12::BlockState;

/// How an entity's client tick pushes the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PushKind {
    /// `LivingEntity.tickCramming` (and `OtherClientPlayerEntity.tickMovement`): the player
    /// overlaps the entity's box and is pushed away from its position.
    Living,
    /// `BoatEntity.tick`: the player overlaps the boat's box grown by 0.2 horizontally and shrunk
    /// by 0.01 vertically, and its feet are not above the boat's bottom.
    Boat,
}

/// An entity whose tick may push the player: its position and box after its tick.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Pusher {
    /// Position (x and z decide the direction).
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

/// The push rule of a 1.12 entity type (registry path, `"player"` for other players), or `None`
/// when its client tick never pushes the player: living entities push except bats, parrots,
/// armor stands and the ender dragon; boats push; everything else (minecarts, items,
/// projectiles, …) does not. `living` is `instanceof LivingEntity` of the type.
#[must_use]
pub fn push_kind(name: &str, living: bool) -> Option<PushKind> {
    match name {
        "boat" => Some(PushKind::Boat),
        "bat" | "parrot" | "armor_stand" | "ender_dragon" => None,
        "player" => Some(PushKind::Living),
        _ if living => Some(PushKind::Living),
        _ => None,
    }
}

/// `true` for the 1.12 entity types the player collides with (`getBox` not null): boats, and
/// shulkers while alive (the caller checks the health).
#[must_use]
pub fn collides(name: &str) -> bool {
    matches!(name, "boat" | "shulker")
}

/// `true` when an entity of type `name` (registry path, `"player"` for other players) blocks the
/// boat the local player rides (`World.doesBoxCollide` for the boat: the entity's `getBox`, a
/// boat's or a live shulker's, and the boat's `getHardCollisionBox`, the box of every
/// `isPushable` entity). That is boats and minecarts (every kind); living entities while alive and not
/// climbing ([`entity_climbing`]), except bats and armor stands (never), parrots (always, also
/// dying or climbing) and horses, donkeys, mules and llamas (only without passengers). Items, rockets
/// and the other objects do not. `living` is `instanceof LivingEntity` of the type; spectator
/// players are skipped by the caller (`EXCEPT_SPECTATOR`), as is the boat's own passenger.
#[must_use]
pub fn blocks_boat(
    name: &str,
    living: bool,
    alive: bool,
    climbing: bool,
    has_passengers: bool,
) -> bool {
    match name {
        "boat" => true,
        n if n.ends_with("minecart") => true,
        "bat" | "armor_stand" => false,
        "parrot" => true,
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" | "llama" => {
            !has_passengers
        }
        _ => living && alive && !climbing,
    }
}

/// `LivingEntity.isClimbing` of another entity at `pos` whose box starts at `min_y`: a ladder or
/// vine at its feet, or an open trapdoor above a ladder with the same facing.
#[must_use]
pub fn entity_climbing<W: BlockView<State = BlockState>>(
    world: &W,
    pos: [f64; 3],
    min_y: f64,
) -> bool {
    super::player::climbing_at(
        world,
        crate::math::floor(pos[0]),
        crate::math::floor(min_y),
        crate::math::floor(pos[2]),
    )
}

/// The lily pads a boat's client tick breaks: `BoatEntity.tick` ends with
/// `Entity.checkBlockCollision`, and `LilyPadBlock.onEntityCollision` removes a lily pad a boat
/// touches from the client's world at once (the server breaks it too). Appends the position of
/// every lily pad in the blocks the boat's box `bb`, shrunk by 0.001, overlaps (call after the
/// boat's interpolation step, then set those blocks to air).
pub fn lily_pads_broken_by_boat<W: BlockView<State = BlockState>>(
    world: &W,
    bb: &Aabb,
    out: &mut Vec<[i32; 3]>,
) {
    use crate::math::floor;
    let (x0, y0, z0) = (
        floor(bb.min_x + 0.001),
        floor(bb.min_y + 0.001),
        floor(bb.min_z + 0.001),
    );
    let (x1, y1, z1) = (
        floor(bb.max_x - 0.001),
        floor(bb.max_y - 0.001),
        floor(bb.max_z - 0.001),
    );
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                if super::blocks::state_at(world, x, y, z).block().id() == LILY_PAD {
                    out.push([x, y, z]);
                }
            }
        }
    }
}

/// The lily pad block id.
const LILY_PAD: u8 = 111;

/// The push of one entity's client tick on the player (`Entity.pushAwayFrom` through the
/// entity's push rule). Returns `true` when the player's velocity changed. The player is pushed
/// when it is not a spectator (`EXCEPT_SPECTATOR`, `noClip`) and not on a ladder or vine
/// (`isPushable`; vanilla also: alive, not riding with the pusher, no passengers — the caller
/// does not call this then).
pub fn push_by<W: BlockView<State = BlockState>>(
    p: &mut PlayerState,
    world: &W,
    other: &Pusher,
) -> bool {
    let area = match other.kind {
        PushKind::Living => other.bb,
        PushKind::Boat => other
            .bb
            .grow(f64::from(0.2f32), f64::from(-0.01f32), f64::from(0.2f32)),
    };
    if !p.bb.intersects(&area) {
        return false;
    }
    if p.spectator || p.no_clip || is_climbing(p, world) {
        return false;
    }
    // Entity.pushAwayFrom: the living pusher pushes the player away from itself
    // (`player.pushAwayFrom(mob)`), the boat pushes the player (`boat.pushAwayFrom(player)`)
    // once its feet are not above the boat's bottom.
    let (dx, dz) = match other.kind {
        PushKind::Living => (other.pos[0] - p.pos[0], other.pos[2] - p.pos[2]),
        PushKind::Boat => {
            if !(p.bb.min_y <= other.bb.min_y) {
                return false;
            }
            (p.pos[0] - other.pos[0], p.pos[2] - other.pos[2])
        }
    };
    let Some((x, z)) = push_away(dx, dz) else {
        return false;
    };
    match other.kind {
        PushKind::Living => {
            p.vel[0] += -x;
            p.vel[2] += -z;
        }
        PushKind::Boat => {
            p.vel[0] += x;
            p.vel[2] += z;
        }
    }
    // Entity.addVelocity marks the velocity as changed.
    p.velocity_dirty = true;
    true
}

/// The shared arithmetic of `Entity.pushAwayFrom`: the offset `(dx, dz)` from the entity that
/// moves to the other one, scaled to the push; `None` closer than 0.01 on both axes. 1.12's
/// `MathHelper.sqrt(double)` rounds the root to `float`.
fn push_away(mut dx: f64, mut dz: f64) -> Option<(f64, f64)> {
    let mut f = abs_max(dx, dz);
    if !(f >= f64::from(0.01f32)) {
        return None;
    }
    f = f64::from(f.sqrt() as f32);
    dx /= f;
    dz /= f;
    let mut g = 1.0 / f;
    if g > 1.0 {
        g = 1.0;
    }
    dx *= g;
    dz *= g;
    dx *= f64::from(0.05f32);
    dz *= f64::from(0.05f32);
    // `* (1.0F - pushSpeedReduction)`: the reduction is 0 for every entity, the factor 1.
    Some((dx, dz))
}

/// `MathHelper.absMax`.
fn abs_max(a: f64, b: f64) -> f64 {
    let a = if a < 0.0 { -a } else { a };
    let b = if b < 0.0 { -b } else { b };
    if a > b { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_kinds() {
        assert_eq!(push_kind("zombie", true), Some(PushKind::Living));
        assert_eq!(push_kind("player", false), Some(PushKind::Living));
        assert_eq!(push_kind("boat", false), Some(PushKind::Boat));
        for n in ["bat", "parrot", "armor_stand", "ender_dragon"] {
            assert_eq!(push_kind(n, true), None, "{n}");
        }
        assert_eq!(push_kind("minecart", false), None);
        assert!(collides("boat") && collides("shulker") && !collides("minecart"));
    }
}
