//! 1.16.5: the chunk store and the vanilla-exact physics.

use ghast_protocol::{BlockPos, RawPacket};

use super::*;
use crate::common::{Abilities, Controls, Outbound, Pose, Teleport, Vec3};
use crate::hands::{BlockQuery, HeldItem};

// ---- 1.16.5: world + stand-still mover --------------------------------------------------

/// 1.16.5: the chunk store and the stand-still mover (no 1.16 physics yet).
pub(crate) struct Sim16 {
    world: ghast_world::v1_16_5::World,
    player: ghast_physics::v1_16_5::PlayerState,
    packets: ghast_physics::Packets,
    rec: Option<crate::record::Recorder>,
    force_step_position: ForceStepPosition,
    entities: Option<ghast_entity::v1_16_5::Tracker>,
    /// Block changes since the last drain (position, version raw state), when collected.
    updates: BlockUpdates,
    /// The session's last join and respawn packets: a tracker started mid-session replays
    /// them to learn our entity id and dimension.
    session: SessionPackets,
    /// The world keeps block entities (signs, tile entity NBT).
    block_entities: bool,
    /// Other entities push the player ([`Sim::set_entity_push`]).
    entity_push: bool,
    /// With entity tracking: the teams' collision rules (`teams` packets).
    teams: Option<Box<super::teams::Teams>>,
    /// The player's scoreboard name (team rules).
    profile_name: Box<str>,
    /// This tick's entity scratch (see [`Sim16::entities_around`]).
    around: Around16,
    /// Where the client world files the player: its position after its last tick.
    filed_pos: [f64; 3],
    /// Movement state beyond the player (the dimension type, riptide).
    ext: ghast_physics::v1_16_5::PlayerExt,
    /// The tick takes the slow path: entity tracking, an extension state that changes the
    /// movement or a use the riptide follows ([`Sim16::update_slow`]); a use that starts takes
    /// it too.
    slow: bool,
    /// Riptide: the tridents in the hands, the rain, the use (see [`Riptide16`]).
    rt: Riptide16,
    /// `ClientPlayerInteractionManager.unacknowledgedPlayerActions`: the dig actions the
    /// server has not answered yet.
    unacked: crate::sim::predict::Unacked,
    /// An answer moved the player back ([`Sim::take_snap`]).
    snapped: bool,
}

/// What the engine's look at the held items ([`Sim::item_use_facts`], through `&self`) leaves
/// for the tick. Boxed: interior mutability inline in [`Sim16`] would cost every `&Sim16`
/// borrow of the engine its no-alias guarantees.
#[derive(Debug, Default)]
struct HeldCells {
    /// Per hand (main, off): the riptide level of the trident there (0: none), whether that
    /// hand's use would start (`HeldItem::usable` and the food rule, as the hands decide), and
    /// the player's age when the engine looked (an empty hand is not looked at: an older entry
    /// is stale).
    hands: std::cell::Cell<[(u8, bool, i32); 2]>,
    /// A riptide trident was held: our entity data is read from now on (the spin flag).
    watch: std::cell::Cell<bool>,
    /// The packet filter must change ([`Sim::take_filter_dirty`]).
    filter_dirty: std::cell::Cell<bool>,
}

/// What the 1.16.5 riptide reads beyond the physics state.
#[derive(Debug)]
struct Riptide16 {
    /// See [`HeldCells`].
    held: Box<HeldCells>,
    /// The use in progress (riptide level of its hand), and the player ticks it lasted.
    using: Option<u8>,
    use_ticks: u32,
    /// `World.rainGradient` (game state packets).
    rain_level: f32,
    /// The dimension type has sky light (no rain without it).
    sky: bool,
    /// The world's hashed seed (biome zoom, `BiomeAccess`).
    seed: i64,
    /// Our entity id (the join packet).
    own_id: i32,
}

impl Default for Riptide16 {
    fn default() -> Self {
        Riptide16 {
            held: Box::default(),
            using: None,
            use_ticks: 0,
            rain_level: 0.0,
            sky: true,
            seed: 0,
            own_id: i32::MIN,
        }
    }
}

/// `VoronoiBiomeAccessType.getBiome`: the biome cell (quart position) whose biome the block at
/// `x`, `y`, `z` has, for the world's hashed seed. The client decodes the dimension type from
/// the join packet with `DimensionType.CODEC`, which always builds the 3D access type.
fn voronoi_cell(seed: i64, x: i32, y: i32, z: i32) -> [i32; 3] {
    // SeedMixer.mixSeed.
    let mix = |s: i64, salt: i64| {
        s.wrapping_mul(
            s.wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407),
        )
        .wrapping_add(salt)
    };
    let distribute = |s: i64| (f64::from((s >> 24).rem_euclid(1024) as i32) / 1024.0 - 0.5) * 0.9;
    let (x, y, z) = (x - 2, y - 2, z - 2);
    let base = [x >> 2, y >> 2, z >> 2];
    let frac = [
        f64::from(x & 3) / 4.0,
        f64::from(y & 3) / 4.0,
        f64::from(z & 3) / 4.0,
    ];
    let mut best = 0;
    let mut dist = 0.0;
    for o in 0..8 {
        let hi = [o & 4 != 0, o & 2 != 0, o & 1 != 0];
        let c: [i64; 3] = std::array::from_fn(|i| i64::from(base[i] + i32::from(hi[i])));
        let f: [f64; 3] = std::array::from_fn(|i| if hi[i] { frac[i] - 1.0 } else { frac[i] });
        let mut l = mix(seed, c[0]);
        l = mix(l, c[1]);
        l = mix(l, c[2]);
        l = mix(l, c[0]);
        l = mix(l, c[1]);
        l = mix(l, c[2]);
        let d = distribute(l);
        l = mix(l, seed);
        let e = distribute(l);
        l = mix(l, seed);
        let g = distribute(l);
        let sq = |v: f64| v * v;
        let v = sq(f[2] + g) + sq(f[1] + e) + sq(f[0] + d);
        if o == 0 || dist > v {
            best = o;
            dist = v;
        }
    }
    [
        base[0] + i32::from(best & 4 != 0),
        base[1] + i32::from(best & 2 != 0),
        base[2] + i32::from(best & 1 != 0),
    ]
}

/// A byte field of a 1.16.5 dimension type compound (`ultrawarm`, `has_skylight`).
fn dimension_flag(dimension: &ghast_protocol::nbt::RawNbt, name: &str) -> Option<bool> {
    use ghast_protocol::nbt::Tag;
    let Ok(Some(t)) = dimension.parse() else {
        return None;
    };
    match t.tag.as_compound().and_then(|c| c.get(name)) {
        Some(Tag::Byte(b)) => Some(*b != 0),
        _ => None,
    }
}

/// The riptide level (`Enchantments` list, `minecraft:riptide`) and the `Damage` of a 1.16.5
/// stack's NBT.
fn trident_nbt(s: &ghast_inventory::Stack) -> (u8, i32) {
    use ghast_protocol::nbt::Tag;
    let Some(root) = s.nbt().and_then(|n| n.parse().ok().flatten()) else {
        return (0, 0);
    };
    let Some(c) = root.tag.as_compound() else {
        return (0, 0);
    };
    let num = |t: Option<&Tag>| match t {
        Some(Tag::Short(v)) => i32::from(*v),
        Some(Tag::Int(v)) => *v,
        Some(Tag::Byte(v)) => i32::from(*v),
        _ => 0,
    };
    let mut level = 0;
    if let Some(Tag::List(l)) = c.get("Enchantments") {
        for e in l.items.iter().filter_map(Tag::as_compound) {
            if matches!(e.get("id"), Some(Tag::String(n)) if n == "minecraft:riptide") {
                level = u8::try_from(num(e.get("lvl")).clamp(0, 255)).unwrap_or(0);
                break;
            }
        }
    }
    (level, num(c.get("Damage")))
}

/// The entities around the player this tick.
#[derive(Debug, Default)]
struct Around16 {
    /// The boxes the player collides with.
    boxes: Vec<ghast_physics::math::Aabb>,
    /// The pushes of the entities that tick after the player, in tick order.
    after: Vec<(u32, ghast_physics::v1_16_5::Pusher)>,
    /// The pushes of the entities that tick before it.
    before: Vec<(u32, ghast_physics::v1_16_5::Pusher)>,
    /// The tracker's client view ticked this tick (in [`Sim::step`]).
    ticked: bool,
    /// What the riptide spin can touch this tick.
    others: Vec<ghast_physics::v1_16_5::OtherEntity>,
}

/// Per entity type: the player collides with it.
const COLLIDES: u8 = 1;
/// Per entity type: a living entity of this type pushes the player.
const PUSH_LIVING: u8 = 2;
/// Per entity type: its tick pushes the player like a boat.
const PUSH_BOAT: u8 = 4;

/// [`COLLIDES`] / [`PUSH_LIVING`] / [`PUSH_BOAT`] of a 1.16.5 entity (players push).
fn entity_bits(e: &ghast_entity::Entity) -> u8 {
    use ghast_physics::v1_16_5 as phys;
    static BITS: std::sync::OnceLock<Box<[u8]>> = std::sync::OnceLock::new();
    let bits = BITS.get_or_init(|| {
        let types = ghast_data::v1_16_5::entities();
        let mut v = vec![
            0u8;
            types
                .iter()
                .map(|t| usize::from(t.id) + 1)
                .max()
                .unwrap_or(0)
        ];
        for t in types {
            let mut b = 0;
            if phys::collides(t.name) {
                b |= COLLIDES;
            }
            match phys::push_kind(t.name, true) {
                Some(phys::PushKind::Boat) => b |= PUSH_BOAT,
                Some(phys::PushKind::Living) => b |= PUSH_LIVING,
                _ => {}
            }
            v[usize::from(t.id)] = b;
        }
        v.into_boxed_slice()
    });
    match e.entity_type() {
        ghast_entity::EntityType::Player => PUSH_LIVING,
        ghast_entity::EntityType::Registry(id) => {
            let b = bits.get(usize::from(id)).copied().unwrap_or(0);
            // Living pushes only for living entities (the living-entity spawn packet).
            if e.class() == ghast_entity::EntityClass::Mob {
                b
            } else {
                b & !PUSH_LIVING
            }
        }
        _ => 0,
    }
}

fn aabb(b: [f64; 6]) -> ghast_physics::math::Aabb {
    ghast_physics::math::Aabb::new(b[0], b[1], b[2], b[3], b[4], b[5])
}

/// `true` when box `b` comes within `m` of the player's box on every axis.
fn near(b: &[f64; 6], p: &ghast_physics::math::Aabb, m: [f64; 3]) -> bool {
    !(b[3] < p.min_x - m[0]
        || b[0] > p.max_x + m[0]
        || b[4] < p.min_y - m[1]
        || b[1] > p.max_y + m[1]
        || b[5] < p.min_z - m[2]
        || b[2] > p.max_z + m[2])
}

impl std::fmt::Debug for Sim16 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sim16")
            .field("world", &self.world)
            .finish_non_exhaustive()
    }
}

impl Sim16 {
    /// Test hook: sets one block.
    #[cfg(all(test, feature = "v1_8", feature = "v1_12_2", feature = "v1_16_5"))]
    pub(crate) fn test_set_block(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        st: ghast_data::v1_16_5::BlockState,
    ) {
        self.world.set_block_state(x, y, z, st);
    }

    pub(crate) fn new() -> Self {
        Sim16 {
            world: ghast_world::v1_16_5::World::new(config()),
            player: ghast_physics::v1_16_5::PlayerState::new([0.0; 3], 0.0, 0.0),
            packets: ghast_physics::Packets::new(),
            rec: None,
            force_step_position: ForceStepPosition::Off,
            entities: None,
            updates: BlockUpdates::default(),
            session: SessionPackets::default(),
            block_entities: false,
            entity_push: true,
            teams: None,
            profile_name: "".into(),
            around: Around16::default(),
            filed_pos: [0.0; 3],
            ext: ghast_physics::v1_16_5::PlayerExt::new(),
            slow: false,
            rt: Riptide16::default(),
            unacked: Default::default(),
            snapped: false,
        }
    }

    /// Whether the tick needs the slow path (entities, a non-default extension state, a use
    /// the riptide follows).
    fn update_slow(&mut self) {
        self.slow = self.entities.is_some() || !self.ext.is_inert() || self.rt.using.is_some();
    }

    /// The join / respawn packet's dimension type and seed (a new client world).
    fn new_world(&mut self, dimension: &ghast_protocol::nbt::RawNbt, seed: i64) {
        self.ext.ultrawarm = dimension_flag(dimension, "ultrawarm").unwrap_or(false);
        self.rt.sky = dimension_flag(dimension, "has_skylight").unwrap_or(true);
        self.rt.seed = seed;
        self.update_slow();
    }

    /// `World.hasRain(pos)` without the height map: the level rains (gradient above 0.2), the
    /// dimension has sky light and the biome at the block (`VoronoiBiomeAccessType`, the
    /// dimension type the client decodes from the packet) rains there: precipitation `rain`
    /// and a base temperature of at least 0.15. A biome the world does not keep counts as
    /// raining; vanilla reads plains in a column it does not have.
    fn rains_at(&self, x: i32, y: i32, z: i32) -> bool {
        // `getRainGradient > 0.2` (false for a NaN gradient too).
        let raining = self.rt.rain_level.partial_cmp(&0.2) == Some(std::cmp::Ordering::Greater);
        if !raining || !self.rt.sky {
            return false;
        }
        let [qx, qy, qz] = voronoi_cell(self.rt.seed, x, y, z);
        let id = match self.world.column(qx >> 2, qz >> 2) {
            None => Some(1),
            Some(c) => match c.biomes() {
                ghast_world::Biomes::Cells(b) => {
                    let i = (qy.clamp(0, 63) << 4 | (qz & 3) << 2 | (qx & 3)) as usize;
                    Some(b[i])
                }
                _ => None,
            },
        };
        let Some(id) = id else {
            return true;
        };
        ghast_data::v1_16_5::biome(id)
            .is_none_or(|b| b.precipitation == "rain" && b.temperature >= 0.15)
    }

    /// `isTouchingWaterOrRain()` of the player as it is now.
    fn wet(&self) -> bool {
        use ghast_physics::v1_16_5 as phys;
        let p = &self.player;
        if p.touching_water {
            return true;
        }
        let (bx, bz) = (p.pos[0].floor() as i32, p.pos[2].floor() as i32);
        let (feet, head) = (p.pos[1].floor() as i32, p.bb.max_y.floor() as i32);
        // The physics checks both blocks against one flag: keep only the raining ones.
        let mut q = *p;
        match (self.rains_at(bx, feet, bz), self.rains_at(bx, head, bz)) {
            (false, false) => return false,
            (true, true) => {}
            (true, false) => q.bb.max_y = q.pos[1],
            (false, true) => q.pos[1] = q.bb.max_y,
        }
        phys::in_water_or_rain(&q, &self.world, true)
    }

    /// The riptide part of the key handling, before the tick: follows the use of a riptide
    /// trident and launches on its release (`TridentItem.onStoppedUsing`).
    fn riptide_use(&mut self, input: &ghast_physics::TickInput, out: &[Outbound]) {
        let age = self.player.age;
        let hands = self
            .rt
            .held
            .hands
            .get()
            .map(|h| if h.2 == age { (h.0, h.1) } else { (0, false) });
        match (self.rt.using, input.using_item) {
            (None, true) => {
                // The hands start with the main hand, then the off hand.
                let hand = if hands[0].1 { hands[0] } else { hands[1] };
                self.rt.using = Some(hand.0);
                self.rt.use_ticks = 0;
            }
            (Some(level), false) => {
                self.rt.using = None;
                // A release (the hands sent `release_use_item`); a use cut short otherwise (a
                // slot change, ...) launches nothing.
                let released = out.iter().rev().any(|o| {
                    matches!(
                        o,
                        Outbound::Dig {
                            status: crate::common::DigStatus::ReleaseUseItem,
                            ..
                        }
                    )
                });
                if released && level > 0 && self.rt.use_ticks >= 10 && self.wet() {
                    let boxes = self.launch_boxes();
                    let around = ghast_physics::v1_16_5::Surroundings::new().with_entities(&boxes);
                    ghast_physics::v1_16_5::riptide(
                        &mut self.player,
                        &mut self.ext,
                        &around,
                        &self.world,
                        level,
                        input.yaw,
                        input.pitch,
                    );
                }
            }
            _ => {}
        }
    }

    /// The boxes of the entities the player collides with before any entity ticks (the key
    /// handling's launch move).
    fn launch_boxes(&self) -> Vec<ghast_physics::math::Aabb> {
        let Some(t) = self.entities.as_ref() else {
            return Vec::new();
        };
        let w = &self.world;
        t.iter()
            .filter(|e| entity_bits(e) & COLLIDES != 0 && t.client_alive(e.id()))
            .filter_map(|e| {
                let pos = t.client_position(e.id())?;
                w.is_column_loaded((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4)
                    .then(|| t.client_box(e.id()).map(aabb))
                    .flatten()
            })
            .collect()
    }

    /// Test hook: the extension state (the dimension type).
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) fn test_ext(&self) -> ghast_physics::v1_16_5::PlayerExt {
        self.ext
    }

    /// A tracker for this sim: with the vanilla client's own entity view (tick order,
    /// interpolated positions and boxes) that pushes and entity collisions read.
    fn tracker(c: ghast_entity::Config) -> ghast_entity::v1_16_5::Tracker {
        let mut t = ghast_entity::v1_16_5::Tracker::new(c);
        t.track_client_motion(true);
        t
    }

    /// Test hook: the player state (from an oracle record).
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) fn test_set_player(&mut self, p: ghast_physics::v1_16_5::PlayerState) {
        self.player = p;
        self.filed_pos = p.pos;
    }

    /// Test hook: the player's scoreboard name (team rules).
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) fn test_set_profile_name(&mut self, name: &str) {
        self.profile_name = name.into();
    }

    /// Test hook: a `login` (own entity id, the overworld) into the entity tracker only.
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) fn test_tracker_login(&mut self, own_id: i32) {
        use ghast_protocol::v1_16_5::play::clientbound::id;
        let Some(t) = &mut self.entities else { return };
        let mut b = Vec::new();
        b.extend_from_slice(&own_id.to_be_bytes());
        b.extend_from_slice(&[0, 0, 0xFF, 1]);
        let name = b"minecraft:overworld";
        b.push(name.len() as u8);
        b.extend_from_slice(name);
        b.extend_from_slice(&[10, 0, 0, 0, 10, 0, 0, 0]);
        b.push(name.len() as u8);
        b.extend_from_slice(name);
        b.extend_from_slice(&0i64.to_be_bytes());
        b.extend_from_slice(&[20, 10, 0, 1, 0, 0]);
        let raw = RawPacket {
            id: id::LOGIN,
            body: bytes::Bytes::from(b),
        };
        t.apply(&raw).unwrap();
    }

    /// Test hook: the client position and box the tracker keeps for entity `id`.
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) fn test_client_box(&self, id: i32) -> Option<([f64; 3], [f64; 6])> {
        let t = self.entities.as_ref()?;
        Some((t.client_position(id)?, t.client_box(id)?))
    }

    /// Test hook: the player state.
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) fn test_player(&self) -> &ghast_physics::v1_16_5::PlayerState {
        &self.player
    }

    /// Test hook: the client world's tick order the tracker keeps.
    #[cfg(all(
        test,
        feature = "v1_8",
        feature = "v1_12_2",
        feature = "v1_16_5",
        feature = "v26_1"
    ))]
    pub(crate) fn test_tick_order(&self) -> Vec<i32> {
        self.entities
            .as_ref()
            .map(|t| t.client_tick_order().collect())
            .unwrap_or_default()
    }

    /// [`Sim::tick_entities`] with a tracker (`ClientWorld.tickEntities`): the entities after
    /// the player push it after its tick.
    #[inline(never)]
    fn tick_tracked(&mut self, pose: &mut Pose) {
        if !std::mem::take(&mut self.around.ticked) {
            // The player did not tick: the other entities still do.
            let w = &self.world;
            if let Some(t) = &mut self.entities {
                t.tick_client(|x, z| w.is_column_loaded(x, z));
            }
            return;
        }
        if self.around.after.is_empty() {
            return;
        }
        let p = &mut self.player;
        for (_, pusher) in &self.around.after {
            ghast_physics::v1_16_5::push_by(p, self.filed_pos, &self.world, pusher);
        }
        self.around.after.clear();
        pose.motion = Vec3::new(p.vel[0], p.vel[1], p.vel[2]);
    }

    /// The entities around the player for this tick, as the 1.16.5 client's entity tick
    /// (`ClientWorld.tickEntities`) has them around the player's own tick: every tracked entity
    /// takes its interpolation step; the ones that tick before the player push it now, the
    /// boxes it collides with are those of the moment it ticks (after their tick for the
    /// entities before it, before it for the others), and the pushes of the entities after it
    /// are kept for [`Sim::tick_entities`].
    fn entities_around(&mut self) {
        let a = &mut self.around;
        a.boxes.clear();
        a.after.clear();
        a.before.clear();
        let Some(t) = self.entities.as_mut() else {
            return;
        };
        let w = &self.world;
        let loaded = |x: i32, z: i32| w.is_column_loaded(x, z);
        let p = &self.player;
        // Farther than any move of one tick can reach (the physics filters exactly).
        let reach = |v: f64| v.abs() + 3.0;
        let m = [reach(p.vel[0]), reach(p.vel[1]), reach(p.vel[2])];
        // The colliders that tick after the player: their boxes before their tick.
        let mut any = false;
        for e in t.iter() {
            let bits = entity_bits(e);
            if bits == 0 {
                continue;
            }
            any = true;
            if bits & COLLIDES == 0 || t.ticks_before_self(e.id()) {
                continue;
            }
            if let Some(b) = t.client_box(e.id())
                && near(&b, &p.bb, m)
                && t.client_alive(e.id())
                && t.client_position(e.id())
                    .is_some_and(|q| loaded((q[0].floor() as i32) >> 4, (q[2].floor() as i32) >> 4))
            {
                a.boxes.push(aabb(b));
            }
        }
        t.tick_client(loaded);
        a.ticked = true;
        if !any {
            return;
        }
        // The local player rides: no pushes, no entity collisions (not simulated).
        if let Some(me) = t.self_id()
            && t.iter().any(|e| e.passengers().contains(&me))
        {
            a.boxes.clear();
            return;
        }
        let own: &str = &self.profile_name;
        let teams = self.teams.as_deref().filter(|t| t.any_rule());
        for e in t.iter() {
            let bits = entity_bits(e);
            if bits == 0 {
                continue;
            }
            let id = e.id();
            let before = t.ticks_before_self(id);
            let (Some(b), Some(pos)) = (t.client_box(id), t.client_position(id)) else {
                continue;
            };
            if bits & COLLIDES != 0
                && before
                && near(&b, &p.bb, m)
                && t.client_alive(id)
                && loaded((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4)
            {
                a.boxes.push(aabb(b));
            }
            if !self.entity_push || bits & (PUSH_LIVING | PUSH_BOAT) == 0 {
                continue;
            }
            // Boats reach 0.2 farther; nothing else can touch the player (the ones after it
            // push the moved player: anything its move can reach).
            let margin = if before { [0.25; 3] } else { m };
            if !near(&b, &p.bb, margin) || !t.client_ticks(e, loaded) {
                continue;
            }
            // A spectator player has noClip and pushes nothing.
            if t.is_spectator(e) {
                continue;
            }
            // canBePushedBy: the pusher's and the player's team collision rules.
            if let Some(teams) = teams {
                let mut buf = [0u8; 36];
                let name = match t.scoreboard_name(e) {
                    Some(n) => n,
                    None => e.uuid().hyphenated().encode_lower(&mut buf),
                };
                if !teams.may_push(name, own) {
                    continue;
                }
            }
            let kind = if bits & PUSH_BOAT != 0 {
                ghast_physics::v1_16_5::PushKind::Boat
            } else {
                ghast_physics::v1_16_5::PushKind::Living
            };
            let rank = t.client_tick_rank(id).unwrap_or(u32::MAX);
            let pusher = ghast_physics::v1_16_5::Pusher::new(pos, aabb(b), kind);
            if before {
                a.before.push((rank, pusher));
            } else {
                a.after.push((rank, pusher));
            }
        }
        a.before.sort_by_key(|x| x.0);
        a.after.sort_by_key(|x| x.0);
        let p = &mut self.player;
        for (_, pusher) in &a.before {
            ghast_physics::v1_16_5::push_by(p, self.filed_pos, &self.world, pusher);
        }
    }

    /// `entity_metadata` for our own entity (read once a riptide trident was held): the living
    /// entity flags (index 7), whose bit 4 is the riptide spin.
    fn own_metadata(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        use ghast_protocol::defs::p754::types::EntityMetadataValue as V;
        use ghast_protocol::v1_16_5::play::clientbound as cb;
        let mut r = ghast_protocol::Reader::new(&raw.body);
        if r.read_varint()? != self.rt.own_id {
            return Ok(());
        }
        let p: cb::EntityMetadata = raw.decode_as()?;
        for e in &p.metadata {
            if let (7, V::Byte(b)) = (e.key, &e.value) {
                self.ext.apply_living_flags(*b as u8);
            }
        }
        self.update_slow();
        Ok(())
    }

    /// The other entities of the client world the riptide spin can touch, with their boxes
    /// when the player ticks (before their own tick for the ones that tick after it).
    fn spin_contacts(&mut self, before: bool) {
        let Some(t) = self.entities.as_ref() else {
            return;
        };
        let w = &self.world;
        for e in t.iter() {
            let id = e.id();
            if t.ticks_before_self(id) != before || t.is_spectator(e) {
                continue;
            }
            let (Some(b), Some(pos)) = (t.client_box(id), t.client_position(id)) else {
                continue;
            };
            if !w.is_column_loaded((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4) {
                continue;
            }
            let living = matches!(e.entity_type(), ghast_entity::EntityType::Player)
                || e.class() == ghast_entity::EntityClass::Mob;
            self.around
                .others
                .push(ghast_physics::v1_16_5::OtherEntity::new(aabb(b), living));
        }
    }

    /// The tick with entities around and / or the extension state.
    #[inline(never)]
    fn slow_tick(&mut self, input: &ghast_physics::TickInput, out: &[Outbound]) {
        // The key handling (before the entities tick): the riptide launch.
        if self.rt.using.is_some() || input.using_item {
            self.riptide_use(input, out);
        }
        self.around.others.clear();
        let spinning = self.ext.riptide_ticks > 0;
        if spinning {
            // The ones after the player, before the tracker ticks them.
            self.spin_contacts(false);
        }
        if self.entities.is_some() {
            self.entities_around();
        }
        if spinning {
            self.spin_contacts(true);
        }
        let around = ghast_physics::v1_16_5::Surroundings::new()
            .with_entities(&self.around.boxes)
            .with_others(&self.around.others);
        if self.ext.is_inert() {
            ghast_physics::v1_16_5::tick_in(
                &mut self.player,
                &around,
                input,
                &self.world,
                &mut self.packets,
            );
        } else {
            ghast_physics::v1_16_5::tick_ext(
                &mut self.player,
                &mut self.ext,
                &around,
                input,
                &self.world,
                &mut self.packets,
            );
        }
        if self.rt.using.is_some() {
            self.rt.use_ticks += 1;
        }
        self.update_slow();
    }

    /// A sim with options (recording, Grim compatibility).
    pub(crate) fn with(opts: &SimOptions) -> Self {
        let mut s = Self::new();
        s.force_step_position = opts.force_step_position;
        s.entities = opts.entities.map(Self::tracker);
        s.teams = s.entities.is_some().then(Box::default);
        if let Some(n) = &opts.profile_name {
            s.profile_name = n.as_str().into();
        }
        if let Some(c) = opts.world {
            s.world = ghast_world::v1_16_5::World::new(c);
            s.block_entities = c.block_entities;
        }
        if opts.shared_chunks.is_some() {
            log::debug!("shared chunks are not used on 1.16.5; every section stays owned");
        }
        s.update_slow();
        if let Some(p) = opts.record.as_deref() {
            match crate::record::Recorder::create(p) {
                Ok(r) => s.rec = Some(r),
                Err(e) => log::warn!("cannot record to {}: {e}", p.display()),
            }
        }
        s
    }
}

impl WorldView for Sim16 {
    fn column(&self, chunk_x: i32, chunk_z: i32) -> Option<&ghast_world::Column> {
        self.world.column(chunk_x, chunk_z)
    }

    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.world.is_column_loaded(chunk_x, chunk_z)
    }

    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        self.world.block_state(x, y, z).map(|s| u32::from(s.id()))
    }

    fn entities(&self) -> Option<&ghast_entity::Entities> {
        self.entities.as_deref()
    }

    fn loaded_columns(&self) -> usize {
        self.world.columns().len()
    }

    fn block_query(&self, x: i32, y: i32, z: i32) -> Option<BlockQuery> {
        let s = self.world.block_state(x, y, z)?;
        let info = s.block().info();
        let air = s.block().is_air();
        let passable = matches!(
            info.name,
            "water" | "lava" | "fire" | "nether_portal" | "end_portal"
        );
        Some(BlockQuery {
            exact: Some(crate::hands::ExactDig::V1_16_5),
            air,
            water: info.name == "water",
            block: s.block().id(),
            hardness: if info.diggable {
                info.hardness.unwrap_or(-1.0)
            } else {
                -1.0
            },
            dig_material: info.material,
            harvest_tools: info.harvest_tools,
            selection: if air || passable {
                None
            } else {
                Some(union(s.shape()).unwrap_or([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]))
            },
            stairs: None,
            activates: activates_1_16(info.name),
        })
    }

    fn crosshair(
        &self,
        eyes: Vec3,
        (yaw, pitch): (f32, f32),
        _look: Vec3,
        reach: f64,
    ) -> Option<crate::BlockHit> {
        // The oracle-verified 1.16.5 pick (outline shapes, model offsets, interaction shapes).
        use ghast_physics::pick::v1_16_5 as pick;
        let eye = [eyes.x, eyes.y, eyes.z];
        let hit = pick::pick(
            &self.world,
            eye,
            pick::rotation_vector(yaw, pitch),
            reach,
            None,
        );
        block_hit(&hit, eye)
    }
}

impl Sim for Sim16 {
    fn crosshair_target(&self, q: &PickQuery<'_>) -> Crosshair {
        // The oracle-verified GameRenderer.updateTargetedEntity (outline shapes, model
        // offsets, interaction shapes, then the entity ray).
        use ghast_physics::pick::v1_16_5 as pick;
        let held_block = q
            .held
            .and_then(ghast_data::v1_16_5::Item::from_id)
            .and_then(|i| ghast_data::v1_16_5::Block::from_name(i.info().name));
        let eye = [q.eyes.x, q.eyes.y, q.eyes.z];
        let look = pick::rotation_vector(q.rotation.0, q.rotation.1);
        if q.entities.is_empty() {
            // pick_target without entities is its block pick.
            let reach = if q.creative {
                pick::CREATIVE_REACH
            } else {
                pick::REACH
            };
            let hit = pick::pick(&self.world, eye, look, reach, held_block);
            return Crosshair::Block(block_hit(&hit, eye));
        }
        let t = pick::pick_target(
            &self.world,
            &q.player_bb,
            eye,
            look,
            q.creative,
            held_block,
            q.entities,
        );
        modern_crosshair(t, eye)
    }

    fn crosshair_now(
        &self,
        eyes: Vec3,
        (yaw, pitch): (f32, f32),
        _look: Vec3,
        held: Option<u16>,
    ) -> Option<crate::BlockHit> {
        use ghast_physics::pick::v1_16_5 as pick;
        let held_block = held
            .and_then(ghast_data::v1_16_5::Item::from_id)
            .and_then(|i| ghast_data::v1_16_5::Block::from_name(i.info().name));
        let eye = [eyes.x, eyes.y, eyes.z];
        let hit = pick::pick(
            &self.world,
            eye,
            pick::rotation_vector(yaw, pitch),
            crate::hands::REACH,
            held_block,
        );
        block_hit(&hit, eye)
    }

    fn world_packet(&mut self, raw: &RawPacket) -> Result<(), SimError> {
        {
            use ghast_protocol::v1_16_5::play::clientbound::id;
            self.session.note(raw, id::LOGIN, id::RESPAWN);
        }
        if let Some(t) = &mut self.entities
            && t.wants(raw.id)
        {
            t.apply(raw)?;
        }
        use ghast_protocol::v1_16_5::play::clientbound as cb;
        if raw.id == cb::id::TEAMS {
            if let Some(t) = &mut self.teams
                && !t.apply(&raw.body, super::teams::TeamLayout::Legacy1_16)
            {
                log::debug!("bad teams packet ({} B)", raw.body.len());
            }
            return Ok(());
        }
        if raw.id == cb::id::ENTITY_METADATA {
            return self.own_metadata(raw);
        }
        let w = &mut self.world;
        match raw.id {
            cb::id::LOGIN => {
                let p: cb::Login = raw.decode_as()?;
                // ClientPlayNetworkHandler.onGameJoin: the world gets the packet's dimension type.
                w.apply_login(&p);
                self.rt.own_id = p.entity_id;
                self.rt.rain_level = 0.0;
                self.new_world(&p.dimension, p.hashed_seed);
                // A new world, a new scoreboard.
                if let Some(t) = &mut self.teams {
                    t.clear();
                }
                // onGameJoin: a new interaction manager (respawns keep it).
                self.unacked.clear();
            }
            cb::id::RESPAWN => {
                let p: cb::Respawn = raw.decode_as()?;
                // onPlayerRespawn builds a new world (with the packet's dimension type, seed and
                // fresh weather) only when the world name changes.
                let new = *p.world_name != *w.world_name();
                w.apply_respawn(&p);
                if new {
                    self.rt.rain_level = 0.0;
                    self.new_world(&p.dimension, p.hashed_seed);
                }
            }
            cb::id::MAP_CHUNK => {
                w.apply_map_chunk(&raw.decode_as()?)?;
            }
            cb::id::UNLOAD_CHUNK => {
                w.apply_unload_chunk(&raw.decode_as()?);
            }
            cb::id::UPDATE_VIEW_POSITION => {
                w.apply_view_position(&raw.decode_as()?);
            }
            cb::id::UPDATE_VIEW_DISTANCE => {
                w.apply_view_distance(&raw.decode_as()?);
            }
            cb::id::BLOCK_CHANGE => {
                let p: cb::BlockChange = raw.decode_as()?;
                self.updates.push(p.location, p.r#type as u32);
                w.apply_block_change(&p);
            }
            cb::id::MULTI_BLOCK_CHANGE => {
                let p: cb::MultiBlockChange = raw.decode_as()?;
                let c = &p.chunk_coordinates;
                let sy = ((c.y << 12) as i32) >> 12;
                for &r in &p.records {
                    let local = (r & 0xFFF) as i32;
                    let pos = BlockPos::new(
                        c.x * 16 + ((local >> 8) & 15),
                        sy.wrapping_mul(16).wrapping_add(local & 15),
                        c.z * 16 + ((local >> 4) & 15),
                    );
                    self.updates.push(pos, ((r as u64) >> 12) as u32);
                }
                w.apply_multi_block_change(&p);
            }
            cb::id::EXPLOSION => {
                w.apply_explosion(&raw.decode_as()?);
            }
            cb::id::ACKNOWLEDGE_PLAYER_DIGGING => {
                self.action_answer(&raw.decode_as()?);
            }
            cb::id::WORLD_BORDER => {
                w.apply_world_border(&raw.decode_as()?);
            }
            cb::id::TILE_ENTITY_DATA if self.block_entities => {
                w.apply_tile_entity_data(&raw.decode_as()?);
            }
            _ => {}
        }
        Ok(())
    }

    fn reset_player(&mut self) {
        let abilities = self.player.abilities;
        let food = self.player.food_level;
        self.player = ghast_physics::v1_16_5::PlayerState::new([0.0; 3], 0.0, 0.0);
        self.filed_pos = [0.0; 3];
        self.player.abilities = abilities;
        self.player.food_level = food;
        // A new client player: no spin, no use.
        self.ext.spin_attack = false;
        self.ext.riptide_ticks = 0;
        self.rt.using = None;
        self.rt.held.hands.set([(0, false, -1); 2]);
        self.update_slow();
    }

    fn teleport(&mut self, pose: &mut Pose, t: &Teleport) {
        self.player.yaw = pose.yaw;
        self.player.pitch = pose.pitch;
        let _reply =
            self.player
                .apply_teleport([t.x, t.y, t.z], t.yaw, t.pitch, rel_flags(t.flags));
        if let Some(r) = &mut self.rec {
            let p = &self.player;
            r.teleport(
                [t.x, t.y, t.z],
                t.yaw,
                t.pitch,
                t.flags,
                t.teleport_id,
                p.pos,
                p.yaw,
                p.pitch,
            );
        }
        sync_pose_16(pose, &self.player);
    }

    fn abilities(&mut self, a: Abilities) {
        let a = physics_abilities(a);
        if let Some(r) = &mut self.rec {
            r.abilities(&a);
        }
        self.player.apply_abilities(a);
    }

    fn food(&mut self, food: i32) {
        self.player.food_level = food;
    }

    fn velocity(&mut self, v: Vec3, add: bool) {
        if let Some(r) = &mut self.rec {
            if add {
                r.note("explosion push not representable");
            } else {
                r.velocity([
                    (v.x * 8000.0).round() as i32,
                    (v.y * 8000.0).round() as i32,
                    (v.z * 8000.0).round() as i32,
                ]);
            }
        }
        let p = &mut self.player;
        if add {
            p.vel[0] += v.x;
            p.vel[1] += v.y;
            p.vel[2] += v.z;
        } else {
            p.vel = [v.x, v.y, v.z];
        }
    }

    fn movement_speed(&mut self, base: f64, modifiers: &[(u128, f64, u8)]) {
        if let Some(r) = &mut self.rec {
            r.speed_attribute(base, modifiers);
        }
        self.player.apply_speed_attribute(base, modifiers);
    }

    fn status_effects(&mut self, effects: &std::collections::BTreeMap<i8, i8>) {
        if let Some(r) = &mut self.rec {
            r.effects(effects);
        }
        let e = &mut self.player.effects;
        e.jump_boost = effects.get(&8).map(|a| i32::from(*a));
        e.blindness = effects.contains_key(&15);
        e.levitation = effects.get(&25).map(|a| i32::from(*a));
        e.slow_falling = effects.contains_key(&28);
        e.dolphins_grace = effects.contains_key(&30);
    }

    fn boots(&mut self, depth_strider: u8, soul_speed: u8) {
        self.player.enchantments.depth_strider = depth_strider;
        self.player.enchantments.soul_speed = soul_speed;
    }

    fn spectator(&mut self, on: bool) {
        self.player.spectator = on;
    }

    fn set_flying(&mut self, flying: bool, out: &mut Vec<Outbound>) {
        let a = &mut self.player.abilities;
        if a.flying == flying || (flying && !a.allow_flying) {
            return;
        }
        a.flying = flying;
        let a = *a;
        if let Some(r) = &mut self.rec {
            r.abilities(&a);
        }
        out.push(Outbound::Abilities {
            flags: abilities_flags(&a),
            flying_speed: a.fly_speed,
            walking_speed: a.walk_speed,
        });
    }

    fn collect_block_updates(&mut self, on: bool) {
        self.updates.on = on;
        if !on {
            self.updates.list.clear();
        }
    }

    fn drain_block_updates(&mut self, out: &mut Vec<(BlockPos, u32)>) -> usize {
        out.append(&mut self.updates.list);
        std::mem::take(&mut self.updates.dropped)
    }

    fn world_packet_ids(&self) -> Vec<i32> {
        use ghast_protocol::v1_16_5::play::clientbound as cb;
        let mut ids: Vec<i32> = self.world.packet_ids().collect();
        // The server's answers to the dig actions.
        ids.push(cb::id::ACKNOWLEDGE_PLAYER_DIGGING);
        // Our own entity data (the riptide spin flag), once a riptide trident was held.
        if self.rt.held.watch.get() {
            ids.push(cb::id::ENTITY_METADATA);
        }
        ids
    }

    fn take_filter_dirty(&mut self) -> bool {
        self.rt.held.filter_dirty.replace(false)
    }

    // Out of line: inlined, it makes `Engine::handle` (every inbound packet) dearer.
    #[inline(never)]
    fn weather(&mut self, reason: u8, value: f32) {
        // ClientPlayNetworkHandler.onGameStateChange: RAIN_STARTED sets the gradient to 0 (the
        // server raises it with RAIN_GRADIENT_CHANGED), RAIN_STOPPED to 1, the gradient change
        // to the value (clamped to 0..1, World.setRainGradient).
        self.rt.rain_level = match reason {
            1 => 0.0,
            2 => 1.0,
            _ => value.clamp(0.0, 1.0),
        };
    }

    // Out of line: the engine's key handling inlines it for every held stack.
    #[inline(never)]
    fn item_use_facts(
        &self,
        stack: &ghast_inventory::Stack,
        off_hand: bool,
        mut held: HeldItem,
    ) -> HeldItem {
        let trident = u16::try_from(stack.id())
            .ok()
            .and_then(ghast_data::v1_16_5::Item::from_id)
            .is_some_and(|i| i.info().name == "trident");
        let mut level = 0;
        if trident {
            // TridentItem.use: FAIL when the next point of damage breaks it, and with riptide
            // out of water and rain.
            let (lvl, damage) = trident_nbt(stack);
            level = lvl;
            held.usable = held.usable && damage < 250 - 1 && (lvl == 0 || self.wet());
            // A riptide trident: read our entity data (the spin flag) from now on.
            if lvl > 0 && !self.rt.held.watch.replace(true) {
                self.rt.held.filter_dirty.set(true);
            }
        }
        let starts =
            held.usable && (!held.food || held.always_edible || self.player.food_level < 20);
        let mut h = self.rt.held.hands.get();
        h[usize::from(off_hand)] = (level, starts, self.player.age);
        self.rt.held.hands.set(h);
        held
    }

    fn entity_config(&self) -> Option<ghast_entity::Config> {
        self.entities.as_ref().map(|t| t.config())
    }

    fn set_entities(&mut self, config: Option<ghast_entity::Config>) {
        if self.entity_config() == config {
            return;
        }
        // Teams sent before are not known (all pushes allowed).
        self.teams = config.is_some().then(Box::default);
        self.entities = config.map(|c| {
            let mut t = Self::tracker(c);
            self.session.replay(|r| {
                if let Err(e) = t.apply(r) {
                    log::debug!("entity tracker: session packet 0x{:02x}: {e}", r.id);
                }
            });
            t
        });
        self.update_slow();
    }

    fn entity_packet_ids(&self) -> Vec<i32> {
        use ghast_protocol::v1_16_5::play::clientbound as cb;
        self.entities
            .as_ref()
            .map(|t| t.packet_ids().chain([cb::id::TEAMS]).collect())
            .unwrap_or_default()
    }

    #[inline(always)]
    fn tick_entities(&mut self, pose: &mut Pose) {
        // Without a tracker nothing ticks (`around` is only filled with one).
        if self.entities.is_some() {
            self.tick_tracked(pose);
        }
    }

    fn set_entity_push(&mut self, on: bool) {
        self.entity_push = on;
    }

    fn predict_air(&mut self, x: i32, y: i32, z: i32) {
        self.world
            .set_block_state(x, y, z, ghast_data::v1_16_5::BlockState::AIR);
    }

    fn dig_sent(
        &mut self,
        pos: BlockPos,
        status: crate::common::DigStatus,
        broken: bool,
        _sequence: i32,
        player: Vec3,
    ) {
        use crate::common::DigStatus;
        // sendPlayerAction (start, abort, stop) records the action, then breakBlock.
        if matches!(
            status,
            DigStatus::Start | DigStatus::Abort | DigStatus::Finish
        ) {
            self.unacked
                .put(pos, status.id() as u8, [player.x, player.y, player.z]);
        }
        if broken {
            // breakBlock: the block's fluid (`FluidState.getBlockState`) takes its place: a
            // waterlogged block leaves a water source.
            use ghast_data::FluidKind;
            use ghast_data::v1_16_5::{Block, BlockState};
            let Some(old) = self.world.block_state(pos.x, pos.y, pos.z) else {
                return;
            };
            let left = match old.fluid().map(|f| f.kind) {
                Some(FluidKind::Water) => Block::WATER.default_state(),
                Some(FluidKind::Lava) => Block::LAVA.default_state(),
                None => BlockState::AIR,
            };
            self.world.set_block_state(pos.x, pos.y, pos.z, left);
        }
    }

    fn place_sent(&mut self, p: &crate::sim::predict::PlaceSent<'_>) {
        // interactBlock → BlockItem.place on the client world; the server's block updates for
        // the clicked block and its neighbour settle it (no records).
        if let Some((at, state)) = self.plain_placement(p) {
            self.world.set_block_state(at.x, at.y, at.z, state);
        }
    }

    #[cfg(test)]
    fn set_player_pos(&mut self, p: Vec3) {
        self.player.set_position(p.x, p.y, p.z);
    }

    #[inline(always)]
    fn records_digs(&self) -> bool {
        true
    }

    fn take_snap(&mut self) -> Option<Vec3> {
        if !std::mem::take(&mut self.snapped) {
            return None;
        }
        let p = self.player.pos;
        Some(Vec3::new(p[0], p[1], p[2]))
    }

    fn held_item(&self, id: u16, count: u8, efficiency: u8) -> HeldItem {
        held_item_facts(DataSet::V1_16_5, id, count, efficiency)
    }

    fn step(
        &mut self,
        pose: &mut Pose,
        controls: Controls,
        using_item: bool,
        entity_id: i32,
        out: &mut Vec<Outbound>,
    ) {
        let input = ghast_physics::TickInput {
            keys: keys(controls),
            yaw: pose.yaw,
            pitch: pose.pitch,
            using_item,
        };
        self.packets.clear();
        if let Some(r) = &mut self.rec
            && !r.started()
        {
            r.start_1_16(&self.player);
        }
        if let Some(r) = &mut self.rec {
            let w = &self.world;
            r.observe(self.player.pos, |x, y, z| {
                w.is_column_loaded(x >> 4, z >> 4)
                    .then(|| w.block_state(x, y, z).map_or(0, |s| u32::from(s.id())))
            });
        }
        if self.slow || using_item {
            self.slow_tick(&input, out);
        } else {
            ghast_physics::v1_16_5::tick(&mut self.player, &input, &self.world, &mut self.packets);
        }
        // ClientWorld.checkEntityChunkPos after the player's tick.
        self.filed_pos = self.player.pos;
        if self.force_step_position != ForceStepPosition::Off {
            let p = &mut self.player;
            let cur = p.pos;
            force_position(
                &mut self.packets,
                cur,
                &mut p.sent.pos,
                &mut p.sent.ticks_since_pos,
                self.force_step_position,
            );
        }
        if let Some(r) = &mut self.rec {
            r.tick(
                &input,
                &crate::record::state_1_16(&self.player),
                &self.packets,
            );
        }
        push_packets(&self.packets, entity_id, out);
        sync_pose_16(pose, &self.player);
        self.world.advance_time(50);
    }
}

impl Sim16 {
    /// `acknowledge_player_digging` (`ClientPlayerInteractionManager.processPlayerActionResponse`):
    /// the server's state of the block replaces the client's when they differ, unless the server
    /// approved a start the client recorded; the player returns to where it was when the action
    /// was sent if the block would hold it. Oldest records go from 50 on.
    #[cold]
    #[inline(never)]
    fn action_answer(
        &mut self,
        p: &ghast_protocol::v1_16_5::play::clientbound::AcknowledgePlayerDigging,
    ) {
        use ghast_data::v1_16_5::BlockState;
        let pos = p.location;
        let at = u8::try_from(p.status)
            .ok()
            .and_then(|a| self.unacked.remove(pos, a));
        let state = u16::try_from(p.block)
            .ok()
            .and_then(BlockState::from_id)
            .unwrap_or(BlockState::AIR);
        let now = self
            .world
            .block_state(pos.x, pos.y, pos.z)
            .unwrap_or(BlockState::AIR);
        let start = p.status == crate::common::DigStatus::Start.id();
        if now != state && (at.is_none() || !p.successful || !start) {
            self.world.set_block_state(pos.x, pos.y, pos.z, state);
            if let Some(at) = at
                && holds_player(state, pos, &self.player.bb)
            {
                // Entity.updatePosition: the position and the previous position.
                self.player.set_position(at[0], at[1], at[2]);
                self.snapped = true;
            }
        }
        self.unacked.trim();
    }
}

impl Sim16 {
    /// `ClientPlayerInteractionManager.interactBlock` → `BlockItem.place` on the client, when its
    /// result is known exactly ([`crate::sim::predict::PlaceSent`]; the same rules as 26.x,
    /// with this version's blocks): the position and the state placed.
    fn plain_placement(
        &self,
        p: &crate::sim::predict::PlaceSent<'_>,
    ) -> Option<(BlockPos, ghast_data::v1_16_5::BlockState)> {
        use crate::common::GameMode;
        use ghast_data::v1_16_5::{Block, Item};
        // ItemStack.useOnBlock: adventure needs CanPlaceOn; spectators never place.
        if !matches!(p.game_mode, GameMode::Survival | GameMode::Creative) {
            return None;
        }
        let stack = p.held?;
        // BlockStateTag / BlockEntityTag (BlockItem.placeFromTag).
        if stack.nbt().is_some() {
            return None;
        }
        let item = Item::from_id(u16::try_from(stack.id()).ok()?)?;
        let block = Block::from_name(item.info().name)?;
        let state = block.default_state();
        if block.layout().state_count != 1
            || !state.full_cube()
            // ConcretePowderBlock.getPlacementState: concrete next to water.
            || block.info().name.ends_with("_concrete_powder")
        {
            return None;
        }
        if !crate::sim::predict::border_contains(
            &ghast_physics::BlockView::world_border(&self.world),
            p.pos,
        ) {
            return None;
        }
        let clicked = self.world.block_state(p.pos.x, p.pos.y, p.pos.z)?;
        if !clicked.full_cube() {
            return None;
        }
        // AbstractBlock.onUse of the clicked block, unless sneaking with an item.
        if !(p.sneaking && p.any_item) && activated_full_cube_16(clicked.block().info().name) {
            return None;
        }
        let at = p.face.offset(p.pos);
        if !(0..256).contains(&at.y) {
            return None;
        }
        let there = self.world.block_state(at.x, at.y, at.z)?;
        let name = there.block().info().name;
        if !(there.is_air() || name == "water" || name == "lava") {
            return None;
        }
        // World.canPlace: the player's own box (other entities: the server's updates).
        if holds_player(state, at, &self.player.bb) {
            return None;
        }
        Some((at, state))
    }
}

/// Full-cube blocks whose `onUse` may accept a right click with a block item (from the 1.16.5
/// classes; beehives, pumpkins, TNT, redstone ore and fletching tables pass it on).
fn activated_full_cube_16(name: &str) -> bool {
    const LIST: &[&str] = &[
        "barrel",
        "beacon",
        "blast_furnace",
        "cartography_table",
        "chain_command_block",
        "command_block",
        "crafting_table",
        "dispenser",
        "dropper",
        "furnace",
        "jigsaw",
        "jukebox",
        "loom",
        "note_block",
        "repeating_command_block",
        "respawn_anchor",
        "smithing_table",
        "smoker",
        "structure_block",
    ];
    name.ends_with("shulker_box") || LIST.contains(&name)
}

/// 1.16.5 `Entity.collidesWithStateAtPos`: the state's collision shape at `pos` overlaps `bb`.
fn holds_player(
    state: ghast_data::v1_16_5::BlockState,
    pos: BlockPos,
    bb: &ghast_physics::math::Aabb,
) -> bool {
    use ghast_data::v1_16_5::VoxelClass;
    use ghast_physics::voxel::{Points, VoxelShape};
    let r = state.voxel_shape();
    let axis = |n: u8, pts: &'static [f64]| match r.class {
        VoxelClass::Simple => Points::Fractional(u32::from(n)),
        VoxelClass::Array => Points::Explicit(pts),
    };
    let shape = VoxelShape {
        axes: [
            axis(r.sizes[0], r.xs),
            axis(r.sizes[1], r.ys),
            axis(r.sizes[2], r.zs),
        ],
            log::debug!(
                "predicted the break of {} at {pos:?}: {}",
                old.block().name(),
                left.block().name()
            );
        cells: r.cells,
    };
    crate::sim::predict::shape_overlaps(shape, pos, bb)
}

fn sync_pose_16(pose: &mut Pose, p: &ghast_physics::v1_16_5::PlayerState) {
    pose.pos = Vec3::new(p.pos[0], p.pos[1], p.pos[2]);
    pose.motion = Vec3::new(p.vel[0], p.vel[1], p.vel[2]);
            log::debug!("predicted {} at {at:?}", state.block().name());
    pose.yaw = p.yaw;
    pose.pitch = p.pitch;
    pose.on_ground = p.on_ground;
    pose.flying = p.abilities.flying;
    pose.sneaking = p.sneaking();
    pose.sprinting = p.sprinting;
}

#[cfg(all(test, ghast_modern))]
mod tests {
    /// 1.16.5's `VoronoiBiomeAccessType` is the zoom of 26.1's `BiomeManager.getBiome` (whose
    /// port is checked against the 26.1 client): same cells over random seeds and positions.
    #[test]
    fn voronoi_cell_is_the_modern_zoom() {
        let mut r: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            r ^= r << 13;
            r ^= r >> 7;
            r ^= r << 17;
            r
        };
        for _ in 0..20_000 {
            let seed = next() as i64;
            let p = [next(), next(), next()].map(|v| (v as i32) >> 8);
            assert_eq!(
                super::voronoi_cell(seed, p[0], p[1], p[2]),
                ghast_world::climate::zoom_cell(seed, p[0], p[1], p[2]),
                "seed {seed} at {p:?}"
            );
        }
    }
}
            log::debug!(
                "dig answer {} ({}): {pos:?} {} back to the server's {}",
                p.status,
                if p.successful { "approved" } else { "refused" },
                now.block().name(),
                state.block().name()
            );
