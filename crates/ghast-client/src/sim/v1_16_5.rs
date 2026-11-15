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
        }
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
        let w = &mut self.world;
        match raw.id {
            cb::id::LOGIN => {
                w.apply_login(&raw.decode_as()?);
                // A new world, a new scoreboard.
                if let Some(t) = &mut self.teams {
                    t.clear();
                }
            }
            cb::id::RESPAWN => {
                w.apply_respawn(&raw.decode_as()?);
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
        self.world.packet_ids().collect()
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
        if self.entities.is_some() {
            self.entities_around();
            let around =
                ghast_physics::v1_16_5::Surroundings::new().with_entities(&self.around.boxes);
            ghast_physics::v1_16_5::tick_in(
                &mut self.player,
                &around,
                &input,
                &self.world,
                &mut self.packets,
            );
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

fn sync_pose_16(pose: &mut Pose, p: &ghast_physics::v1_16_5::PlayerState) {
    pose.pos = Vec3::new(p.pos[0], p.pos[1], p.pos[2]);
    pose.motion = Vec3::new(p.vel[0], p.vel[1], p.vel[2]);
    pose.yaw = p.yaw;
    pose.pitch = p.pitch;
    pose.on_ground = p.on_ground;
    pose.flying = p.abilities.flying;
    pose.sneaking = p.sneaking();
    pose.sprinting = p.sprinting;
}
