//! The client state of a modern (26.x) entity, shared by every modern tracker instance
//! (`v26_1`, `v26_2`): one type, so the version-free [`crate::Entity`] can hold it.

use std::ops::Deref;

use ghast_data::modern::EntityTypeData;

/// Bits of `State::flags`.
pub(crate) mod st {
    /// `previousTickPosition` / `previousTickRot` are set.
    pub const HAS_PREV: u8 = 1;
    /// The scale changed: vanilla refreshes the dimensions at the end of the entity's next tick
    /// (`refreshDirtyAttributes`).
    pub const SCALE_DIRTY: u8 = 2;
    /// Nothing changes in a tick until a packet or a movement disturbs the entity.
    pub const SETTLED: u8 = 4;
    /// Living: a sleeping position is set (`isSleeping`: the tick zeroes xRot).
    pub const SLEEPING: u8 = 8;
    /// The baby flag.
    pub const BABY: u8 = 16;
    /// `dims` holds the cached dimensions.
    pub const HAS_DIMS: u8 = 32;
}

/// The modern client state of an entity (`InterpolationHandler`, `VecDeltaCodec`, size keys),
/// boxed so entities of the other versions do not carry it.
#[derive(Clone, Debug)]
pub(crate) struct State {
    pub(crate) td: Option<&'static EntityTypeData>,
    /// Interpolation target and its rotation `(yRot, xRot)`.
    pub(crate) target: [f64; 3],
    /// `previousTickPosition` (valid with [`st::HAS_PREV`]).
    pub(crate) prev: [f64; 3],
    pub(crate) target_rot: [f32; 2],
    /// `previousTickRot` (`(xRot, yRot)`, vanilla's `Vec2` order).
    pub(crate) prev_rot: [f32; 2],
    /// Vanilla's cached `dimensions` (valid with [`st::HAS_DIMS`]; refreshed on size metadata, at
    /// spawn, and after a scale change at the end of the next tick).
    pub(crate) dims: ghast_data::EntityDimensions,
    pub(crate) head_target: f32,
    /// `yBodyRot`, and the mob `BodyRotationControl` state.
    pub(crate) body_yaw: f32,
    pub(crate) last_stable_head: f32,
    /// `getScale()` (sanitized `minecraft:scale` attribute).
    pub(crate) scale: f32,
    pub(crate) slime_size: i32,
    /// Item entities: the stack size (0 = empty / unknown).
    pub(crate) item_count: i32,
    /// Players: the tab list object the entity cached (`AbstractClientPlayer.getPlayerInfo`,
    /// 0 = none yet) and its game mode once it left the tab list.
    pub(crate) info_gen: u32,
    pub(crate) frozen_game_mode: i8,
    /// Remaining interpolation steps (0 = none active).
    pub(crate) steps: u8,
    pub(crate) head_steps: u8,
    /// `headStableTime`, saturated at 30 (vanilla only compares it with 10 and 20).
    pub(crate) head_stable_time: u8,
    pub(crate) pose: u8,
    pub(crate) stand_flags: u8,
    /// Foxes: their own flags byte (`Fox.isSleeping` = bit 0x20 instead of the bed position).
    pub(crate) fox_flags: u8,
    /// [`st`] bits.
    pub(crate) flags: u8,
}

impl State {
    pub(crate) const EMPTY: State = State {
        td: None,
        target: [0.0; 3],
        prev: [0.0; 3],
        target_rot: [0.0; 2],
        prev_rot: [0.0; 2],
        dims: ghast_data::EntityDimensions {
            width: 0.0,
            height: 0.0,
            eye_height: 0.0,
        },
        head_target: 0.0,
        body_yaw: 0.0,
        last_stable_head: 0.0,
        scale: 1.0,
        slime_size: 1,
        item_count: 0,
        info_gen: 0,
        frozen_game_mode: 0,
        steps: 0,
        head_steps: 0,
        head_stable_time: 0,
        pose: 0,
        stand_flags: 0,
        fox_flags: 0,
        flags: 0,
    };

    #[inline]
    pub(crate) fn flag(&self, f: u8) -> bool {
        self.flags & f != 0
    }

    #[inline]
    pub(crate) fn set_flag(&mut self, f: u8, on: bool) {
        if on {
            self.flags |= f;
        } else {
            self.flags &= !f;
        }
    }

    #[inline]
    pub(crate) fn dims(&self) -> Option<ghast_data::EntityDimensions> {
        self.flag(st::HAS_DIMS).then_some(self.dims)
    }

    #[inline]
    pub(crate) fn set_dims(&mut self, d: Option<ghast_data::EntityDimensions>) {
        self.set_flag(st::HAS_DIMS, d.is_some());
        if let Some(d) = d {
            self.dims = d;
        }
    }
}

/// The boxed modern state of an entity: empty (one pointer) for the other versions; reads of an
/// empty one see the defaults, the first write allocates.
#[derive(Clone, Debug, Default)]
pub(crate) struct Ext(pub(crate) Option<Box<State>>);

static EMPTY_STATE: State = State::EMPTY;

impl Deref for Ext {
    type Target = State;
    #[inline]
    fn deref(&self) -> &State {
        self.0.as_deref().unwrap_or(&EMPTY_STATE)
    }
}

impl std::ops::DerefMut for Ext {
    #[inline]
    fn deref_mut(&mut self) -> &mut State {
        self.0.get_or_insert_with(|| Box::new(State::EMPTY))
    }
}
