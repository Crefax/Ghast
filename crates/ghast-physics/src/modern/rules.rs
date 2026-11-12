//! The rule-set seam of the modern core: what differs between modern versions (data tables and
//! constants). Everything is an associated item, so a version is monomorphised into the core with
//! no run-time branch.

/// The data types every modern version shares (`ghast_data::v26_1` and `ghast_data::v26_2`
/// re-export the same types; whichever is compiled in).
#[cfg(feature = "v26_1")]
pub(crate) use ghast_data::v26_1 as shared;
#[cfg(all(feature = "v26_2", not(feature = "v26_1")))]
pub(crate) use ghast_data::v26_2 as shared;

use shared::{StatePhys, VoxelShapeRef};

/// One modern version's rules.
///
/// The packed per-state record ([`StatePhys`]) and the voxel shapes are the types every modern
/// version shares. The per-block movement facts are read through [`Rules::friction`],
/// [`Rules::speed_factor`], [`Rules::jump_factor`], [`Rules::block_flags`], [`Rules::offset`] and
/// [`Rules::block_name`]; with the `v26_1` feature they default to the fields of
/// [`Rules::block`] (the 26.1 block record).
pub trait Rules: 'static {
    /// The version's block state (global palette id).
    type State: Copy + PartialEq;

    /// Protocol number (informational).
    const PROTOCOL: i32;

    /// Air (what the client reads outside the build height and in unloaded columns).
    const AIR: Self::State;

    /// Packed per-state movement record.
    fn phys(s: Self::State) -> StatePhys;

    /// The state's block as a 26.1 block record (the layout of `ghast_data::v26_1`). The core
    /// reads the per-block facts through the methods below; a later rule set's record is the one
    /// of the 26.1 block of the same name (see that rule set).
    #[cfg(feature = "v26_1")]
    fn block(s: Self::State) -> &'static ghast_data::v26_1::BlockData;

    /// `Block.getFriction`.
    #[cfg(feature = "v26_1")]
    #[inline]
    fn friction(s: Self::State) -> f32 {
        Self::block(s).friction
    }
    /// `Block.getFriction`.
    #[cfg(not(feature = "v26_1"))]
    fn friction(s: Self::State) -> f32;

    /// `Block.getSpeedFactor`.
    #[cfg(feature = "v26_1")]
    #[inline]
    fn speed_factor(s: Self::State) -> f32 {
        Self::block(s).speed_factor
    }
    /// `Block.getSpeedFactor`.
    #[cfg(not(feature = "v26_1"))]
    fn speed_factor(s: Self::State) -> f32;

    /// `Block.getJumpFactor`.
    #[cfg(feature = "v26_1")]
    #[inline]
    fn jump_factor(s: Self::State) -> f32 {
        Self::block(s).jump_factor
    }
    /// `Block.getJumpFactor`.
    #[cfg(not(feature = "v26_1"))]
    fn jump_factor(s: Self::State) -> f32;

    /// The block's flag bits (`block_flags`: tags and block classes).
    #[cfg(feature = "v26_1")]
    #[inline]
    fn block_flags(s: Self::State) -> u32 {
        Self::block(s).flags
    }
    /// The block's flag bits (`block_flags`: tags and block classes).
    #[cfg(not(feature = "v26_1"))]
    fn block_flags(s: Self::State) -> u32;

    /// The block's model offset (`(max horizontal, max vertical, xyz)`), if it has one.
    #[cfg(feature = "v26_1")]
    #[inline]
    fn offset(s: Self::State) -> Option<(f32, f32, bool)> {
        Self::block(s).offset
    }
    /// The block's model offset (`(max horizontal, max vertical, xyz)`), if it has one.
    #[cfg(not(feature = "v26_1"))]
    fn offset(s: Self::State) -> Option<(f32, f32, bool)>;

    /// The block's registry name without namespace.
    #[cfg(feature = "v26_1")]
    #[inline]
    fn block_name(s: Self::State) -> &'static str {
        Self::block(s).name
    }
    /// The block's registry name without namespace.
    #[cfg(not(feature = "v26_1"))]
    fn block_name(s: Self::State) -> &'static str;

    /// A collision voxel shape by index.
    fn voxel(i: u16) -> &'static VoxelShapeRef;

    /// Index of `Shapes.block()`.
    const FULL_CUBE_VOXEL: u16;
    /// Index of scaffolding's bottom shape.
    const SCAFFOLDING_BOTTOM_VOXEL: u16;
    /// Index of powder snow's falling shape.
    const POWDER_SNOW_FALLING_VOXEL: u16;

    /// `LocalPlayer.sendPosition`: a position is sent when the squared move exceeds this.
    const POSITION_THRESHOLD_SQ: f64;
    /// Ticks after which a position is sent anyway (`POSITION_REMINDER_INTERVAL`).
    const POSITION_REMINDER: i32;
    /// Players zero horizontal velocity when its squared length is below this.
    const HORIZONTAL_CUTOFF_SQ: f64;

    /// 26.2's collision response (`Entity.restituteMovementAfterCollisions`) instead of 26.1's
    /// (`setDeltaMovement` with zeroed axes + `Block.updateEntityMovementAfterFallOn`): every
    /// colliding axis is reflected with a restitution (the `bounciness` attribute, on landing at
    /// least the block's bounce restitution), with gravity and drag compensated for the part of
    /// the tick after the impact; with no restitution the axis becomes a signed zero
    /// (`-v * 0`). The movement attributes `friction_modifier`, `air_drag_modifier` and
    /// `bounciness` ([`MovementAttributes`](super::MovementAttributes)) exist from 26.2 on.
    const RESTITUTION: bool = false;
}

/// Minecraft 26.1 (protocol 775).
#[cfg(feature = "v26_1")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct V26_1;

#[cfg(feature = "v26_1")]
impl Rules for V26_1 {
    type State = ghast_data::v26_1::BlockState;

    const PROTOCOL: i32 = 775;
    const AIR: Self::State = ghast_data::v26_1::BlockState::AIR;

    #[inline]
    fn phys(s: Self::State) -> StatePhys {
        s.phys()
    }

    #[inline]
    fn block(s: Self::State) -> &'static ghast_data::v26_1::BlockData {
        s.block().data()
    }

    #[inline]
    fn voxel(i: u16) -> &'static VoxelShapeRef {
        &ghast_data::v26_1::voxel_shapes()[usize::from(i)]
    }

    const FULL_CUBE_VOXEL: u16 = ghast_data::v26_1::FULL_CUBE_VOXEL;
    const SCAFFOLDING_BOTTOM_VOXEL: u16 = ghast_data::v26_1::SCAFFOLDING_BOTTOM_VOXEL;
    const POWDER_SNOW_FALLING_VOXEL: u16 = ghast_data::v26_1::POWDER_SNOW_FALLING_VOXEL;

    const POSITION_THRESHOLD_SQ: f64 = 2.0e-4 * 2.0e-4;
    const POSITION_REMINDER: i32 = 20;
    const HORIZONTAL_CUTOFF_SQ: f64 = 9.0e-6;
}

/// Minecraft 26.2 (protocol 776): the 26.1 rules with 26.2's collision restitution
/// ([`Rules::RESTITUTION`]) over the 26.2 tables.
#[cfg(feature = "v26_2")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct V26_2;

#[cfg(feature = "v26_2")]
impl Rules for V26_2 {
    type State = ghast_data::v26_2::BlockState;

    const PROTOCOL: i32 = 776;
    const AIR: Self::State = ghast_data::v26_2::BlockState::AIR;

    #[inline]
    fn phys(s: Self::State) -> StatePhys {
        s.phys()
    }

    /// The record of the 26.1 block with the same name, stone for a block 26.1 does not have.
    /// The core does not read it: the per-block facts below come from the 26.2 tables.
    #[cfg(feature = "v26_1")]
    fn block(s: Self::State) -> &'static ghast_data::v26_1::BlockData {
        let stone = ghast_data::v26_1::BlockState::from_id(1).map(|s| s.block());
        match ghast_data::v26_1::Block::from_name(s.block().name()).or(stone) {
            Some(b) => b.data(),
            None => ghast_data::v26_1::BlockState::AIR.block().data(),
        }
    }

    #[inline]
    fn friction(s: Self::State) -> f32 {
        s.block().data().friction
    }

    #[inline]
    fn speed_factor(s: Self::State) -> f32 {
        s.block().data().speed_factor
    }

    #[inline]
    fn jump_factor(s: Self::State) -> f32 {
        s.block().data().jump_factor
    }

    #[inline]
    fn block_flags(s: Self::State) -> u32 {
        s.block().data().flags
    }

    #[inline]
    fn offset(s: Self::State) -> Option<(f32, f32, bool)> {
        s.block().data().offset
    }

    #[inline]
    fn block_name(s: Self::State) -> &'static str {
        s.block().name()
    }

    #[inline]
    fn voxel(i: u16) -> &'static VoxelShapeRef {
        &ghast_data::v26_2::voxel_shapes()[usize::from(i)]
    }

    const FULL_CUBE_VOXEL: u16 = ghast_data::v26_2::FULL_CUBE_VOXEL;
    const SCAFFOLDING_BOTTOM_VOXEL: u16 = ghast_data::v26_2::SCAFFOLDING_BOTTOM_VOXEL;
    const POWDER_SNOW_FALLING_VOXEL: u16 = ghast_data::v26_2::POWDER_SNOW_FALLING_VOXEL;

    const POSITION_THRESHOLD_SQ: f64 = 2.0e-4 * 2.0e-4;
    const POSITION_REMINDER: i32 = 20;
    const HORIZONTAL_CUTOFF_SQ: f64 = 9.0e-6;
    const RESTITUTION: bool = true;
}
