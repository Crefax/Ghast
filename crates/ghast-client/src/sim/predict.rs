//! The client's own records of the blocks it changed ahead of the server, and how the server's
//! answers settle them.
//!
//! - 1.19+ (26.x), `BlockStatePredictionHandler`: every block the client changes while it predicts
//!   (`startPredicting`, one sequence number per predicting packet) remembers the server's state
//!   from before. Block updates for such a position only replace the remembered state; the
//!   server's `block_changed_ack(sequence)` ends every prediction up to that number and puts the
//!   remembered state back into the world (a refused break is restored even when the server sends
//!   no block update, which it often does not).
//! - 1.16.5, `ClientPlayerInteractionManager.unacknowledgedPlayerActions`: every dig action
//!   packet remembers the player's position; the server's `acknowledge_player_digging` answers
//!   each one with the block's state and whether it was approved.
//!
//! Both are empty almost always (the server answers within a round trip): the callers test
//! [`Sequenced::is_empty`] / nothing at all on the hot path.

use ghast_protocol::BlockPos;

/// One prediction of [`Sequenced`] (`ServerVerifiedState`).
#[cfg(ghast_modern)]
#[derive(Clone, Copy, Debug)]
struct Verified<S> {
    pos: BlockPos,
    /// The sequence of the last prediction that changed this block.
    sequence: i32,
    /// The server's state as far as the client knows it.
    state: S,
    /// The player's position when the block was first predicted.
    player: [f64; 3],
}

/// `BlockStatePredictionHandler` of one client level (26.x).
#[cfg(ghast_modern)]
#[derive(Clone, Debug)]
pub(crate) struct Sequenced<S> {
    /// Vanilla keeps a hash map; the list is short (one entry per block broken in the last
    /// round trip).
    entries: Vec<Verified<S>>,
    /// `lastTeleportSequence`: the sequence when the last server teleport arrived.
    last_teleport: i32,
}

#[cfg(ghast_modern)]
impl<S> Default for Sequenced<S> {
    fn default() -> Self {
        Sequenced {
            entries: Vec::new(),
            last_teleport: -1,
        }
    }
}

#[cfg(ghast_modern)]
impl<S: Copy> Sequenced<S> {
    /// No prediction is waiting for the server.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `retainKnownServerState`: the client changed `pos` (from `old`) in prediction
    /// `sequence`. A block predicted before keeps its first server state and player position
    /// and takes the new sequence.
    pub(crate) fn retain(&mut self, pos: BlockPos, old: S, sequence: i32, player: [f64; 3]) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.pos == pos) {
            e.sequence = sequence;
        } else {
            self.entries.push(Verified {
                pos,
                sequence,
                state: old,
                player,
            });
        }
    }

    /// `updateKnownServerState`: a server block update for a predicted position replaces the
    /// remembered state (`true`: the world must not take it now).
    pub(crate) fn update(&mut self, pos: BlockPos, state: S) -> bool {
        match self.entries.iter_mut().find(|e| e.pos == pos) {
            Some(e) => {
                e.state = state;
                true
            }
            None => false,
        }
    }

    /// `onTeleport`: a server teleport arrived while `current` was the last sequence sent.
    pub(crate) fn on_teleport(&mut self, current: i32) {
        self.last_teleport = current;
    }

    /// `endPredictionsUpTo(ack)`: removes every prediction with a sequence up to `ack` and
    /// hands each to `sync` (`ClientLevel.syncBlockState`) with its remembered state and, unless
    /// a teleport arrived after `ack` was sent, the player position to return to.
    pub(crate) fn end_up_to(
        &mut self,
        ack: i32,
        mut sync: impl FnMut(BlockPos, S, Option<[f64; 3]>),
    ) {
        let reset = self.last_teleport < ack;
        self.entries.retain(|e| {
            if e.sequence <= ack {
                sync(e.pos, e.state, reset.then_some(e.player));
                false
            } else {
                true
            }
        });
    }

    /// A new client level (a new handler).
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.last_teleport = -1;
    }
}

/// `ClientPlayerInteractionManager.unacknowledgedPlayerActions` (1.16.5): (position, dig
/// action) → the player's position when the action was sent, in sending order.
#[cfg(feature = "v1_16_5")]
#[derive(Clone, Debug, Default)]
pub(crate) struct Unacked {
    entries: Vec<(BlockPos, u8, [f64; 3])>,
}

#[cfg(feature = "v1_16_5")]
impl Unacked {
    /// Vanilla drops the oldest entries from this size on (after each answer).
    const LIMIT: usize = 50;

    /// `sendPlayerAction`: `put` (an entry for the same position and action keeps its place and
    /// takes the new player position).
    pub(crate) fn put(&mut self, pos: BlockPos, action: u8, player: [f64; 3]) {
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| e.0 == pos && e.1 == action)
        {
            e.2 = player;
        } else {
            self.entries.push((pos, action, player));
        }
    }

    /// The answer's `remove(Pair.of(pos, action))`.
    pub(crate) fn remove(&mut self, pos: BlockPos, action: u8) -> Option<[f64; 3]> {
        let i = self
            .entries
            .iter()
            .position(|e| e.0 == pos && e.1 == action)?;
        Some(self.entries.remove(i).2)
    }

    /// The size limit after an answer ("Too many unacked block actions").
    pub(crate) fn trim(&mut self) {
        if self.entries.len() >= Self::LIMIT {
            let n = self.entries.len() + 1 - Self::LIMIT;
            log::debug!("too many unacknowledged block actions: dropping {n}");
            self.entries.drain(..n);
        }
    }

    /// A new interaction manager (join game).
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// 1.16.5 `Entity.collidesWithStateAtPos`: the collision shape (context-free cells, unshifted)
/// placed at `pos` overlaps the player's box (`VoxelShapes.matchesAnywhere(…, AND)`). Not exact
/// for bamboo (model offset) and scaffolding (context shape); 26.x uses the physics' own
/// context-aware query.
#[cfg(feature = "v1_16_5")]
pub(crate) fn shape_overlaps(
    shape: ghast_physics::voxel::VoxelShape<'_>,
    pos: BlockPos,
    bb: &ghast_physics::math::Aabb,
) -> bool {
    shape.intersects(
        bb,
        Some([f64::from(pos.x), f64::from(pos.y), f64::from(pos.z)]),
    )
}

#[cfg(all(test, feature = "v1_16_5"))]
mod tests_unacked {
    use super::*;

    /// `put` keeps the first place of a (position, action) pair; `trim` leaves 49.
    #[test]
    fn unacked_order_and_limit() {
        let mut u = Unacked::default();
        let p = |i| BlockPos::new(i, 0, 0);
        u.put(p(1), 0, [1.0; 3]);
        u.put(p(1), 2, [2.0; 3]);
        u.put(p(1), 0, [3.0; 3]);
        assert_eq!(u.len(), 2);
        assert_eq!(u.remove(p(1), 0), Some([3.0; 3]));
        assert_eq!(u.remove(p(1), 0), None);
        for i in 0..60 {
            u.put(p(i), 1, [f64::from(i); 3]);
        }
        u.trim();
        assert_eq!(u.len(), 49);
        // The oldest went first.
        assert_eq!(u.remove(p(1), 2), None);
        assert_eq!(u.remove(p(10), 1), None);
        assert_eq!(u.remove(p(11), 1), Some([11.0; 3]));
    }
}

#[cfg(all(test, ghast_modern))]
mod tests {
    use super::*;

    #[test]
    fn sequenced_ends_up_to_the_ack() {
        let mut s = Sequenced::<u16>::default();
        let (a, b) = (BlockPos::new(1, 2, 3), BlockPos::new(4, 5, 6));
        s.retain(a, 1, 1, [0.0; 3]);
        s.retain(b, 1, 2, [0.0; 3]);
        // Predicted again: the first server state stays, the sequence moves.
        s.retain(a, 9, 3, [1.0; 3]);
        assert!(s.update(b, 7));
        assert!(!s.update(BlockPos::new(0, 0, 0), 7));
        let mut got = Vec::new();
        s.end_up_to(2, |p, st, pl| got.push((p, st, pl)));
        assert_eq!(got, [(b, 7, Some([0.0; 3]))]);
        s.on_teleport(3);
        got.clear();
        s.end_up_to(3, |p, st, pl| got.push((p, st, pl)));
        // A teleport at sequence 3: no position reset for acks up to 3.
        assert_eq!(got, [(a, 1, None)]);
        assert!(s.is_empty());
    }
}
