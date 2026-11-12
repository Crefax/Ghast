//! Other entities around the player: the ones it collides with (boats, shulkers) and the ones
//! whose client tick pushes it (mobs, boats).
//!
//! # As the 26.1 client does it
//!
//! * **Collision.** In its own move the player collides with the boxes of entities that
//!   `canBeCollidedWith` it (boats and rafts always, a living shulker, an adult happy ghast under
//!   conditions) like with blocks: entity boxes first, then the border, then blocks, also for the
//!   step-up and for every "does the box fit" test (poses, sneaking at edges). Pass the boxes as
//!   they are when the player ticks in [`Surroundings::entities`] to [`tick_in`](super::tick_in).
//! * **Pushing.** On the client only the local player is pushable (`EntitySelector.pushableBy`
//!   rejects every other entity there), and the player's own push finds nothing: it is the other
//!   entities' client ticks that push it. A mob's `aiStep` ends with `pushEntities` over its own
//!   box; a boat's tick pushes what is in its box grown by 0.2 horizontally (and shrunk by 0.01
//!   vertically) when the player's feet are not above the boat's bottom. Call [`push_by`] for
//!   each such entity, in the client's entity tick order, with its box after its tick (entities
//!   added after the player tick after it; the player is added first at a login).
//!   [`push_kind_26_1`] says which entity types push.
//! * Team collision rules (`set_player_team` `collisionRule`) are the caller's: with a rule other
//!   than `always` on either team, skip what it forbids.

// Vanilla's negated float comparisons are kept as written: they decide the NaN cases.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use super::ext::PlayerExt;
use super::math::{V3, intersects};
use super::player::{PlayerState, on_climbable_ext};
use super::rules::Rules;
use crate::math::Aabb;
use crate::world::BlockView;

/// What else is around the player for one tick. Default: nothing.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct Surroundings<'a> {
    /// Boxes of the entities the player collides with, as they are when the player ticks.
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
    /// `LivingEntity.pushEntities`: the player overlaps the entity's box.
    Living,
    /// `AbstractBoat.tick`: the player overlaps the boat's box grown by 0.2 horizontally and
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

/// The push rule of a 26.1 entity type (registry name without namespace) for the local player,
/// or `None` when its client tick never pushes the player: living entities except bats, parrots
/// and armor stands push; boats and rafts push; everything else (minecarts, items, projectiles,
/// …) does not. `living` is `instanceof LivingEntity` of the type
/// (`ghast_data::v26_1::entity_type(id).is_living()`).
#[must_use]
pub fn push_kind_26_1(name: &str, living: bool) -> Option<PushKind> {
    if is_boat(name) {
        return Some(PushKind::Boat);
    }
    match name {
        "bat" | "parrot" | "armor_stand" => None,
        _ if living => Some(PushKind::Living),
        _ => None,
    }
}

/// `true` for the 26.1 entity types the player always collides with (`canBeCollidedWith`):
/// boats and rafts, and a shulker while it is alive. (An adult happy ghast collides only under
/// conditions: with a player standing at or above its top, or while it keeps still after a
/// rider got off; the caller decides that one.)
#[must_use]
pub fn collides_26_1(name: &str) -> bool {
    is_boat(name) || name == "shulker"
}

fn is_boat(name: &str) -> bool {
    name.ends_with("_boat") || name.ends_with("_raft")
}

/// The push of one entity's client tick on the player (`Entity.push(Entity)` through the entity's
/// push rule). Returns `true` when the player's velocity changed. The player is pushable when it
/// is not a spectator and not on a climbable (vanilla also: alive, not riding something it is
/// the vehicle of; the caller does not call this for a dead player).
pub fn push_by<R: Rules, W: BlockView<State = R::State>>(
    p: &mut PlayerState,
    x: &PlayerExt,
    world: &W,
    other: &Pusher,
) -> bool {
    let area = match other.kind {
        PushKind::Living => other.bb,
        PushKind::Boat => super::math::inflate(
            &other.bb,
            f64::from(0.2f32),
            f64::from(-0.01f32),
            f64::from(0.2f32),
        ),
    };
    let b = p.bb;
    if !intersects(
        &b, area.min_x, area.min_y, area.min_z, area.max_x, area.max_y, area.max_z,
    ) {
        return false;
    }
    // EntitySelector.pushableBy: no spectators, isPushable (alive, not a spectator, not on a
    // climbable).
    if p.spectator || on_climbable_ext::<R, W>(p, x, world) {
        return false;
    }
    if other.kind == PushKind::Boat && !(b.min_y <= other.bb.min_y) {
        return false;
    }
    // Entity.push(entity): the player moves away from the other entity.
    let mut xa = p.pos[0] - other.pos[0];
    let mut za = p.pos[2] - other.pos[2];
    let mut dd = xa.abs().max(za.abs());
    if !(dd >= f64::from(0.01f32)) {
        return false;
    }
    dd = dd.sqrt();
    xa /= dd;
    za /= dd;
    let pow = (1.0 / dd).min(1.0);
    xa *= pow;
    za *= pow;
    xa *= f64::from(0.05f32);
    za *= f64::from(0.05f32);
    if !(xa.is_finite() && za.is_finite()) {
        return false;
    }
    let v = V3::of(p.vel).add(V3::new(xa, 0.0, za));
    if v.is_finite() {
        p.vel = v.arr();
    }
    true
}
