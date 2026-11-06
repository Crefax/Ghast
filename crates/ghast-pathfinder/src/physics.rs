//! A version-generic face over `ghast-physics`, used for costs, the follower's one-tick
//! look-ahead and path validation.

use ghast_physics::{BlockView, Keys, Packets, TickInput, WorldBorder};

use crate::Version;
use crate::world::BlockSource;

/// What the follower observes of the player (the client's `Pose`, version-free).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observation {
    /// Feet position.
    pub pos: [f64; 3],
    /// Motion per tick (as stored after the last tick).
    pub vel: [f64; 3],
    /// On ground.
    pub on_ground: bool,
    /// Yaw (degrees).
    pub yaw: f32,
    /// Pitch (degrees).
    pub pitch: f32,
    /// Sprinting.
    pub sprinting: bool,
    /// Food level (sprinting needs more than 6).
    pub food: i32,
}

/// One version's movement rules.
pub(crate) trait Physics: 'static {
    type State: Copy;
    type Block: Copy;
    const VERSION: Version;

    /// A player standing still at `pos` (settled: on ground if a floor is right below).
    fn new_state(pos: [f64; 3], yaw: f32, pitch: f32) -> Self::State;
    /// A state rebuilt from what the client exposes (approximate: timers are not known).
    fn from_observation(o: &Observation) -> Self::State;
    /// The observation of a state.
    fn observe(s: &Self::State) -> Observation;
    /// One tick.
    fn tick<W: BlockView<State = Self::Block>>(
        s: &mut Self::State,
        keys: Keys,
        yaw: f32,
        pitch: f32,
        world: &W,
    );
    /// Horizontal collision in the last tick.
    fn horizontal_collision(s: &Self::State) -> bool;
    /// Fall distance so far (reset on landing).
    fn fall_distance(s: &Self::State) -> f32;
    /// Decodes a raw state the way the world reads it.
    fn decode(raw: u32) -> Self::Block;
    /// Air.
    fn air() -> Self::Block;
}

pub(crate) struct P18;
pub(crate) struct P16;

impl Physics for P18 {
    type State = ghast_physics::v1_8::PlayerState;
    type Block = ghast_data::v1_8::BlockState;
    const VERSION: Version = Version::V1_8;

    fn new_state(pos: [f64; 3], yaw: f32, pitch: f32) -> Self::State {
        let mut s = ghast_physics::v1_8::PlayerState::new(pos, yaw, pitch);
        s.sent.pos = pos;
        s.sent.yaw = yaw;
        s.sent.pitch = pitch;
        s
    }

    fn from_observation(o: &Observation) -> Self::State {
        let mut s = Self::new_state(o.pos, o.yaw, o.pitch);
        s.vel = o.vel;
        s.on_ground = o.on_ground;
        // Through the setter: sprinting also carries the speed modifier.
        s.set_sprinting(o.sprinting);
        s.sent.sprinting = o.sprinting;
        s.food_level = o.food;
        s
    }

    fn observe(s: &Self::State) -> Observation {
        Observation {
            pos: [s.pos[0], s.bb.min_y, s.pos[2]],
            vel: s.vel,
            on_ground: s.on_ground,
            yaw: s.yaw,
            pitch: s.pitch,
            sprinting: s.sprinting,
            food: s.food_level,
        }
    }

    #[inline]
    fn tick<W: BlockView<State = Self::Block>>(
        s: &mut Self::State,
        keys: Keys,
        yaw: f32,
        pitch: f32,
        world: &W,
    ) {
        let mut out = Packets::new();
        let input = TickInput {
            keys,
            yaw,
            pitch,
            using_item: false,
        };
        ghast_physics::v1_8::tick(s, &input, world, &mut out);
    }

    fn horizontal_collision(s: &Self::State) -> bool {
        s.horizontal_collision
    }

    fn fall_distance(s: &Self::State) -> f32 {
        s.fall_distance
    }

    #[inline]
    fn decode(raw: u32) -> Self::Block {
        ghast_data::v1_8::BlockState::from_raw_vanilla(raw as u16)
    }

    fn air() -> Self::Block {
        ghast_data::v1_8::BlockState::AIR
    }
}

impl Physics for P16 {
    type State = ghast_physics::v1_16_5::PlayerState;
    type Block = ghast_data::v1_16_5::BlockState;
    const VERSION: Version = Version::V1_16_5;

    fn new_state(pos: [f64; 3], yaw: f32, pitch: f32) -> Self::State {
        let mut s = ghast_physics::v1_16_5::PlayerState::new(pos, yaw, pitch);
        s.sent.pos = pos;
        s.sent.yaw = yaw;
        s.sent.pitch = pitch;
        s.first_update = false;
        s.auto_jump_enabled = false;
        s
    }

    fn from_observation(o: &Observation) -> Self::State {
        let mut s = Self::new_state(o.pos, o.yaw, o.pitch);
        s.vel = o.vel;
        s.on_ground = o.on_ground;
        // Through the setter: sprinting also carries the speed modifier.
        s.set_sprinting(o.sprinting);
        s.sent.sprinting = o.sprinting;
        s.food_level = o.food;
        s
    }

    fn observe(s: &Self::State) -> Observation {
        Observation {
            pos: s.pos,
            vel: s.vel,
            on_ground: s.on_ground,
            yaw: s.yaw,
            pitch: s.pitch,
            sprinting: s.sprinting,
            food: s.food_level,
        }
    }

    #[inline]
    fn tick<W: BlockView<State = Self::Block>>(
        s: &mut Self::State,
        keys: Keys,
        yaw: f32,
        pitch: f32,
        world: &W,
    ) {
        let mut out = Packets::new();
        let input = TickInput {
            keys,
            yaw,
            pitch,
            using_item: false,
        };
        ghast_physics::v1_16_5::tick(s, &input, world, &mut out);
    }

    fn horizontal_collision(s: &Self::State) -> bool {
        s.horizontal_collision
    }

    fn fall_distance(s: &Self::State) -> f32 {
        s.fall_distance
    }

    #[inline]
    fn decode(raw: u32) -> Self::Block {
        ghast_data::v1_16_5::BlockState::from_id(raw as u16)
            .unwrap_or(ghast_data::v1_16_5::BlockState::AIR)
    }

    fn air() -> Self::Block {
        ghast_data::v1_16_5::BlockState::AIR
    }
}

/// A physics world view over a [`BlockSource`] (unknown reads as air, like the vanilla client),
/// with optional block overrides (simulated digs and places).
pub(crate) struct SourceView<'a, P: Physics, S: ?Sized> {
    pub(crate) src: &'a S,
    pub(crate) overrides: Vec<([i32; 3], u32)>,
    _p: std::marker::PhantomData<P>,
}

impl<'a, P: Physics, S: BlockSource + ?Sized> SourceView<'a, P, S> {
    pub(crate) fn new(src: &'a S) -> Self {
        Self::with_overrides(src, Vec::new())
    }

    /// A view with earlier overrides (a resumed simulation).
    pub(crate) fn with_overrides(src: &'a S, overrides: Vec<([i32; 3], u32)>) -> Self {
        SourceView {
            src,
            overrides,
            _p: std::marker::PhantomData,
        }
    }

    /// The raw state including overrides.
    pub(crate) fn raw(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        if let Some((_, r)) = self.overrides.iter().rev().find(|(p, _)| *p == [x, y, z]) {
            return Some(*r);
        }
        self.src.raw_block(x, y, z)
    }

    pub(crate) fn set(&mut self, p: [i32; 3], raw: u32) {
        self.overrides.push((p, raw));
    }
}

impl<P: Physics, S: BlockSource + ?Sized> BlockSource for SourceView<'_, P, S> {
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.raw(x, y, z)
    }

    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.src.is_column_loaded(chunk_x, chunk_z)
    }
}

impl<P: Physics, S: BlockSource + ?Sized> BlockView for SourceView<'_, P, S> {
    type State = P::Block;

    #[inline]
    fn block(&self, x: i32, y: i32, z: i32) -> P::Block {
        match self.raw(x, y, z) {
            Some(r) => P::decode(r),
            None => P::air(),
        }
    }

    #[inline]
    fn is_chunk_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.src.is_column_loaded(chunk_x, chunk_z)
    }

    fn world_border(&self) -> WorldBorder {
        WorldBorder::default()
    }
}
