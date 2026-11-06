//! The error type of the helpers.

use ghast_client::BlockPos;
use ghast_client::inventory::ClickError;

/// Why a helper gave up. Nothing was left half-done: buttons are released and a window the
/// helper opened is closed again.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The session ended (or the handle belongs to a session that ended).
    #[error("the session ended")]
    Disconnected,
    /// The helper did not finish within its time limit.
    #[error("timed out")]
    Timeout,
    /// The server teleported / set back the player while the helper was running.
    #[error("teleported by the server")]
    Teleported,
    /// The player died while the helper was running.
    #[error("the player died")]
    Died,
    /// The block's chunk column is not loaded (never treated as air).
    #[error("block {0:?} is not loaded")]
    NotLoaded(BlockPos),
    /// There is nothing to dig (air).
    #[error("nothing to dig at {0:?}")]
    NothingToDig(BlockPos),
    /// The block cannot be broken (bedrock, barriers, ...).
    #[error("block {0:?} cannot be broken")]
    Unbreakable(BlockPos),
    /// No face of the block can be reached with an unobstructed crosshair (reach 4.5).
    #[error("block {0:?} is out of reach or hidden")]
    OutOfReach(BlockPos),
    /// The crosshair left the block while digging (something moved in between).
    #[error("lost the crosshair on {0:?}")]
    LostTarget(BlockPos),
    /// The block to place into is occupied (not air or a liquid), or the player stands in it.
    #[error("{0:?} is occupied")]
    Occupied(BlockPos),
    /// The block to place against is air.
    #[error("nothing to place against at {0:?}")]
    NoSupport(BlockPos),
    /// The block placed against reacts to a right click (chest, door, ...): placing would
    /// activate it. Sneak first (`set_controls` with `sneak`) or pick another face.
    #[error("block {0:?} would be activated instead")]
    WouldActivate(BlockPos),
    /// The server did not put a block there (it refused the placement).
    #[error("no block appeared at {0:?}")]
    NotPlaced(BlockPos),
    /// Nothing in the hand.
    #[error("nothing held")]
    NothingHeld,
    /// No stack matches.
    #[error("no matching item: {0}")]
    NoSuchItem(String),
    /// An item name this version does not know.
    #[error("unknown item name {0:?}")]
    UnknownItem(String),
    /// A block name this version does not know.
    #[error("unknown block name {0:?}")]
    UnknownBlock(String),
    /// The helper needs an open container window.
    #[error("no container window is open")]
    NoWindow,
    /// The inventory refused the click for good (not a retryable refusal).
    #[error("click refused: {0}")]
    Click(ClickError),
    /// The server rejected the click (the inventory resyncs).
    #[error("the server rejected the click")]
    ClickRejected,
    /// No confirm arrived for the click.
    #[error("the click was not confirmed")]
    ClickTimedOut,
    /// The held item cannot be used now (not usable, full food bar, ...).
    #[error("the held item cannot be used now")]
    CannotUse,
    /// The task was aborted or its session ended before it finished.
    #[error("the task was cancelled")]
    Cancelled,
    /// The task panicked.
    #[error("the task panicked: {0}")]
    Panicked(String),
    /// The entity is not tracked (gone, or [`ghast_client::Interests::ENTITIES`] is off).
    #[error("entity {0} is not tracked")]
    NoSuchEntity(i32),
    /// No path to the goal.
    #[error("no path to the goal")]
    NoPath,
    /// The walker made no progress.
    #[error("stuck")]
    Stuck,
}
