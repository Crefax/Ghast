//! The rule-set seam of the modern core: what differs between modern versions (data tables and
//! constants). Everything is an associated item, so a version is monomorphised into the core with
//! no run-time branch.

use ghast_data::v26_1::{BlockData, StatePhys, VoxelShapeRef};

/// One modern version's rules.
///
/// The block data types are the 26.1 data layout (`ghast_data::v26_1`); a later version with
/// the same layout provides its own tables through the same types.
pub trait Rules: 'static {
    /// The version's block state (global palette id).
    type State: Copy + PartialEq;

    /// Protocol number (informational).
    const PROTOCOL: i32;

    /// Air (what the client reads outside the build height and in unloaded columns).
    const AIR: Self::State;

    /// Packed per-state movement record.
    fn phys(s: Self::State) -> StatePhys;

    /// Per-block data of the state's block (friction, speed / jump factors, flags, kind).
    fn block(s: Self::State) -> &'static BlockData;

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
}

/// Minecraft 26.1 (protocol 775).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct V26_1;

impl Rules for V26_1 {
    type State = ghast_data::v26_1::BlockState;

    const PROTOCOL: i32 = 775;
    const AIR: Self::State = ghast_data::v26_1::BlockState::AIR;

    #[inline]
    fn phys(s: Self::State) -> StatePhys {
        s.phys()
    }

    #[inline]
    fn block(s: Self::State) -> &'static BlockData {
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
