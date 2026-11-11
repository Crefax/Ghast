//! Following a path from a `ghast_client::Behavior`: plan, follow, replan.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use ghast_client::{BlockPos, Buttons, Controls, Face, TickContext};
use ghast_physics::Keys;

use crate::Version;
use crate::control::{Controller, ControllerConfig, Progress};
use crate::goal::Goal;
use crate::physics::Observation;
use crate::plan::{Budget, Path, Pathfinder, PlanError, Search};
use crate::sim::{SimOutcome, Simulation};
use crate::world::BlockSource;

/// What the follower reads and does in one tick. Implemented for ghast-client's
/// [`TickContext`]; the tests drive the same follower with a simulated player.
pub(crate) trait Io {
    /// The world type.
    type World<'w>: BlockSource + ?Sized
    where
        Self: 'w;
    /// The world as the client knows it.
    fn world(&self) -> &Self::World<'_>;
    /// The planner's version (`None`: not supported by the planner, e.g. 26.1).
    fn version(&self) -> Option<Version>;
    /// The player.
    fn observation(&self) -> Observation;
    /// The server moved the player this tick (teleport, respawn).
    fn moved_by_server(&self) -> bool;
    /// Dead.
    fn is_dead(&self) -> bool;
    /// Aim target.
    fn set_aim(&mut self, yaw: f32, pitch: f32);
    /// The look this tick's movement uses (after turning toward the aim).
    fn planned_look(&self) -> (f32, f32);
    /// Movement keys from now on.
    fn set_keys(&mut self, keys: Keys);
    /// The left button is held.
    fn left_button(&self) -> bool;
    /// Holds or releases the left button (and the right one).
    fn set_left_button(&mut self, on: bool);
    /// The crosshair is on this block (as the server judges it).
    fn targets(&self, block: [i32; 3]) -> bool;
    /// Right-clicks the top face of `against` with the held item if the crosshair is on it;
    /// `true` when the place was sent.
    fn place_on_top(&mut self, against: [i32; 3]) -> bool;
    /// 26.1: the live client player (attributes, effects, abilities, level).
    #[cfg(feature = "v26_1")]
    fn modern_player(&self) -> Option<&ghast_physics::modern::PlayerState> {
        None
    }
    /// The build height `(min_y, height)` of the player's dimension, when known.
    fn build_height(&self) -> Option<(i32, i32)> {
        #[cfg(feature = "v26_1")]
        if let Some(p) = self.modern_player() {
            return Some((p.level.min_y, p.level.height));
        }
        None
    }
}

/// The client's world with the dimension's build height (unknown below it).
struct Height<'a, W: ?Sized> {
    world: &'a W,
    height: Option<(i32, i32)>,
}

impl<W: BlockSource + ?Sized> BlockSource for Height<'_, W> {
    #[inline]
    fn raw_block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        if let Some((lo, h)) = self.height
            && (y < lo || y >= lo + h)
        {
            return None;
        }
        self.world.raw_block(x, y, z)
    }

    #[inline]
    fn is_column_loaded(&self, chunk_x: i32, chunk_z: i32) -> bool {
        self.world.is_column_loaded(chunk_x, chunk_z)
    }

    fn build_height(&self) -> Option<(i32, i32)> {
        self.height.or_else(|| self.world.build_height())
    }
}

/// [`Height`] over an [`Io`]'s world.
fn world_of<I: Io + ?Sized>(ctx: &I) -> Height<'_, I::World<'_>> {
    Height {
        world: ctx.world(),
        height: ctx.build_height(),
    }
}

impl Io for TickContext<'_> {
    type World<'w>
        = dyn ghast_client::WorldView + 'w
    where
        Self: 'w;

    fn world(&self) -> &Self::World<'_> {
        TickContext::world(self)
    }

    fn version(&self) -> Option<Version> {
        Version::from_game(TickContext::version(self))
    }

    fn observation(&self) -> Observation {
        let pose = *self.pose();
        Observation {
            pos: [pose.pos.x, pose.pos.y, pose.pos.z],
            vel: [pose.motion.x, pose.motion.y, pose.motion.z],
            on_ground: pose.on_ground,
            yaw: pose.yaw,
            pitch: pose.pitch,
            sprinting: pose.sprinting,
            food: self.health().food,
        }
    }

    fn moved_by_server(&self) -> bool {
        !self.events().teleports.is_empty() || self.events().respawned
    }

    fn is_dead(&self) -> bool {
        TickContext::is_dead(self)
    }

    fn set_aim(&mut self, yaw: f32, pitch: f32) {
        TickContext::set_aim(self, yaw, pitch);
    }

    fn planned_look(&self) -> (f32, f32) {
        TickContext::planned_look(self)
    }

    fn set_keys(&mut self, k: Keys) {
        self.set_controls(Controls {
            forward: k.forward,
            back: k.back,
            left: k.left,
            right: k.right,
            jump: k.jump,
            sneak: k.sneak,
            sprint: k.sprint,
        });
    }

    fn left_button(&self) -> bool {
        self.buttons().left
    }

    fn set_left_button(&mut self, on: bool) {
        self.set_buttons(Buttons {
            left: on,
            right: false,
        });
    }

    fn targets(&self, block: [i32; 3]) -> bool {
        let pos = BlockPos::new(block[0], block[1], block[2]);
        self.target().is_some_and(|t| t.pos == pos)
    }

    fn place_on_top(&mut self, against: [i32; 3]) -> bool {
        let against = BlockPos::new(against[0], against[1], against[2]);
        match self
            .target()
            .filter(|t| t.pos == against && t.face == Face::Up)
        {
            Some(hit) => self.place(against, Face::Up, hit.hit),
            None => false,
        }
    }
    #[cfg(feature = "v26_1")]
    fn modern_player(&self) -> Option<&ghast_physics::modern::PlayerState> {
        TickContext::modern_player(self)
    }
}

/// Options of the [`Follower`]. Build with [`FollowConfig::default`] and the `with_*`
/// methods.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FollowConfig {
    /// The controller's options.
    pub controller: ControllerConfig,
    /// Replans after a stuck / off-path move or a blocked partial path before giving up.
    /// Default 8. Teleports replan without counting.
    pub max_replans: u32,
    /// Simulate each plan in `ghast-physics` before moving and plan around moves the
    /// simulation fails (see [`Pathfinder::plan_verified`]). Default on.
    pub verify: bool,
    /// Planning work per game tick: the search expands at most `budget.nodes` nodes and
    /// planning plus verification stop after `budget.time`. Default 2000 nodes and 5 ms
    /// (about 4 ms of A* on a desktop core), so a long plan spreads over several ticks
    /// while the player stands still.
    pub budget: Budget,
    /// Most simulated ticks of the verification per game tick (also bounded by
    /// `budget.time`). Default 400.
    pub verify_ticks: u32,
}

impl Default for FollowConfig {
    fn default() -> Self {
        FollowConfig {
            controller: ControllerConfig::default(),
            max_replans: 8,
            verify: true,
            budget: Budget::nodes(2000).with_time(Duration::from_millis(5)),
            verify_ticks: 400,
        }
    }
}

impl FollowConfig {
    /// The controller's options.
    #[must_use]
    pub fn with_controller(mut self, c: ControllerConfig) -> Self {
        self.controller = c;
        self
    }

    /// Replan budget.
    #[must_use]
    pub fn with_max_replans(mut self, n: u32) -> Self {
        self.max_replans = n;
        self
    }

    /// Simulated verification of plans on or off.
    #[must_use]
    pub fn with_verify(mut self, on: bool) -> Self {
        self.verify = on;
        self
    }

    /// Planning work per game tick ([`Budget::UNLIMITED`]: plan in one tick).
    #[must_use]
    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    /// Simulated verification ticks per game tick.
    #[must_use]
    pub fn with_verify_ticks(mut self, ticks: u32) -> Self {
        self.verify_ticks = ticks.max(1);
        self
    }
}

/// Why following failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FollowError {
    /// Planning failed.
    Plan(PlanError),
    /// The replan budget ([`FollowConfig::max_replans`]) is used up.
    GaveUp,
    /// The planner does not support this game version (26.1) or its feature is off.
    UnsupportedVersion,
}

impl std::fmt::Display for FollowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FollowError::Plan(e) => write!(f, "planning failed: {e}"),
            FollowError::GaveUp => f.write_str("gave up after too many replans"),
            FollowError::UnsupportedVersion => {
                f.write_str("the path finder does not support this game version")
            }
        }
    }
}

impl std::error::Error for FollowError {}

/// What the [`Follower`] is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FollowStatus {
    /// No goal.
    Idle,
    /// Walking (or digging / placing) toward the goal.
    Moving,
    /// The goal is reached; keys released.
    Arrived,
    /// Following failed; keys released.
    Failed(FollowError),
}

/// Plans and follows a path from a behaviour's tick: call [`Follower::tick`] once per tick from
/// `Behavior::tick`. It sets the aim, the movement keys and (for digs) the left button
/// through the [`TickContext`], so the client's rules hold: rotation in whole mouse counts,
/// actions before the tick's single movement packet, digs / places only with the sent
/// crosshair on the block.
///
/// It replans after a teleport, when the controller gets stuck or leaves the path (that move
/// is avoided from then on), and at the end of a partial path (goal beyond loaded terrain).
/// Digging and placing follow [`crate::Config::dig`] / [`crate::Config::place`]; for places
/// the behaviour keeps a block in the hand.
///
/// ```no_run
/// use ghast_client::{Behavior, TickContext};
/// use ghast_pathfinder::{Config, FollowConfig, FollowStatus, Follower, Goal, Pathfinder};
///
/// struct GoThere(Follower);
///
/// impl Behavior for GoThere {
///     fn tick(&mut self, ctx: &mut TickContext<'_>) {
///         match self.0.tick(ctx) {
///             FollowStatus::Arrived => ctx.disconnect("arrived"),
///             FollowStatus::Failed(e) => ctx.disconnect(format!("failed: {e}")),
///             _ => {}
///         }
///     }
/// }
///
/// let mut f = Follower::new(Pathfinder::new(Config::default()), FollowConfig::default());
/// f.goto(Goal::Block([20, 64, 5]));
/// let mut behaviour = GoThere(f);
/// ```
#[derive(Clone, Debug)]
pub struct Follower {
    pf: Pathfinder,
    cfg: FollowConfig,
    goal: Option<Goal>,
    ctrl: Option<Controller>,
    status: FollowStatus,
    replans: u32,
    banned: HashSet<([i32; 3], [i32; 3])>,
    /// Closest a partial path has come to the goal so far (no progress beyond it = blocked).
    partial_best: f64,
    /// The path being followed is partial and final: no path to the goal exists through the
    /// loaded terrain (the search was exhausted), it ends as close to the goal as possible.
    unreachable: bool,
    /// Ticks to wait after a place was sent.
    place_wait: u32,
    digging: bool,
    plans: u64,
    /// A plan in progress (spread over ticks).
    planning: Option<Planning>,
    /// Ticks the current (or last) plan took.
    plan_ticks: u32,
    /// Whether the goal lay in loaded terrain when the current plan started.
    goal_loaded: Option<bool>,
}

/// A plan in progress: the search, then the simulated check.
#[derive(Clone, Debug)]
enum Planning {
    Search {
        search: Box<Search>,
        /// Moves the verification failed (on top of [`Follower::banned`]).
        bans: HashSet<([i32; 3], [i32; 3])>,
        attempt: u32,
        /// The previous attempt's path (followed if a later search fails) and whether its
        /// search was exhausted.
        last: Option<(Path, bool)>,
    },
    Verify {
        sim: Box<Simulation>,
        bans: HashSet<([i32; 3], [i32; 3])>,
        attempt: u32,
        /// The search of the path was exhausted.
        exhausted: bool,
    },
}

/// The verification of `path` with the live player when the client has one (26.1).
fn new_sim<I: Io + ?Sized>(ctx: &I, path: &Path, cfg: &ControllerConfig) -> Simulation {
    #[cfg(feature = "v26_1")]
    if let Some(p) = ctx.modern_player() {
        return Simulation::with_modern_player(path, cfg, p);
    }
    let _ = ctx;
    Simulation::new(path, cfg)
}

/// Plans per goal and replan: the first one and two repairs after failed simulations.
const VERIFY_ATTEMPTS: u32 = 3;

/// Whether the goal's place lies in loaded terrain (goals without one: no).
fn goal_loaded<W: BlockSource + ?Sized>(goal: &Goal, world: &W) -> bool {
    match goal {
        Goal::Block(p) | Goal::Near { pos: p, .. } => world.is_column_loaded(p[0] >> 4, p[2] >> 4),
        Goal::Xz { x, z } => world.is_column_loaded(x >> 4, z >> 4),
        Goal::Y(_) => false,
        Goal::AnyOf(gs) => !gs.is_empty() && gs.iter().all(|g| goal_loaded(g, world)),
    }
}

impl Follower {
    /// A follower without a goal.
    #[must_use]
    pub fn new(pathfinder: Pathfinder, cfg: FollowConfig) -> Self {
        Follower {
            pf: pathfinder,
            cfg,
            goal: None,
            ctrl: None,
            status: FollowStatus::Idle,
            replans: 0,
            banned: HashSet::new(),
            partial_best: f64::INFINITY,
            unreachable: false,
            place_wait: 0,
            digging: false,
            plans: 0,
            planning: None,
            plan_ticks: 0,
            goal_loaded: None,
        }
    }

    /// Sets a new goal (planned on the next [`Follower::tick`]).
    pub fn goto(&mut self, goal: Goal) {
        self.goal = Some(goal);
        self.ctrl = None;
        self.planning = None;
        self.status = FollowStatus::Moving;
        self.replans = 0;
        self.banned.clear();
        self.partial_best = f64::INFINITY;
        self.unreachable = false;
    }

    /// Drops the goal and releases the keys and buttons (in this tick).
    pub fn stop(&mut self, ctx: &mut TickContext<'_>) {
        self.goal = None;
        self.ctrl = None;
        self.planning = None;
        self.status = FollowStatus::Idle;
        self.release(ctx);
    }

    /// The goal.
    #[must_use]
    pub fn goal(&self) -> Option<&Goal> {
        self.goal.as_ref()
    }

    /// The current state.
    #[must_use]
    pub fn status(&self) -> FollowStatus {
        self.status
    }

    /// The path being followed.
    #[must_use]
    pub fn path(&self) -> Option<&crate::Path> {
        self.ctrl.as_ref().map(Controller::path)
    }

    /// Plans made since the follower was created (for logs and tests).
    #[must_use]
    pub fn plans(&self) -> u64 {
        self.plans
    }

    /// A plan is in progress: the player stands still (keys released, no turning) and the
    /// client keeps sending its normal movement packet every tick.
    #[must_use]
    pub fn is_planning(&self) -> bool {
        self.planning.is_some()
    }

    /// Ticks the current (or the last) plan took, verification included (1 = planned within
    /// the tick it started).
    #[must_use]
    pub fn plan_ticks(&self) -> u32 {
        self.plan_ticks
    }

    /// The planner.
    #[must_use]
    pub fn pathfinder(&self) -> &Pathfinder {
        &self.pf
    }

    fn release<I: Io + ?Sized>(&mut self, ctx: &mut I) {
        ctx.set_keys(Keys::NONE);
        if self.digging || ctx.left_button() {
            ctx.set_left_button(false);
        }
        self.digging = false;
    }

    fn fail<I: Io + ?Sized>(&mut self, ctx: &mut I, e: FollowError) -> FollowStatus {
        self.ctrl = None;
        self.planning = None;
        self.status = FollowStatus::Failed(e);
        self.release(ctx);
        self.status
    }

    /// Starts a plan from the player's feet block.
    fn start_plan<I: Io + ?Sized>(&mut self, ctx: &I, version: Version) {
        let goal = self.goal.clone().expect("planning without a goal");
        let start = Pathfinder::start_block(ctx.observation().pos);
        self.plans += 1;
        self.plan_ticks = 0;
        self.planning = Some(Planning::Search {
            search: Box::new(self.pf.search(version, start, goal, self.banned.clone())),
            bans: HashSet::new(),
            attempt: 0,
            last: None,
        });
    }

    /// This tick's share of the plan in progress: the controller once the path is ready.
    fn work<I: Io + ?Sized>(&mut self, ctx: &I) -> Result<Option<Controller>, PlanError> {
        self.plan_ticks += 1;
        let t0 = Instant::now();
        let left = |budget: Option<Duration>| budget.map(|b| b.saturating_sub(t0.elapsed()));
        loop {
            if let Some(max) = self.cfg.budget.time
                && t0.elapsed() >= max
            {
                return Ok(None);
            }
            let Some(phase) = self.planning.take() else {
                return Ok(None);
            };
            match phase {
                Planning::Search {
                    mut search,
                    bans,
                    attempt,
                    last,
                } => {
                    let budget = Budget {
                        nodes: self.cfg.budget.nodes,
                        time: left(self.cfg.budget.time),
                    };
                    match search.step(&world_of(ctx), budget) {
                        None => {
                            self.planning = Some(Planning::Search {
                                search,
                                bans,
                                attempt,
                                last,
                            });
                            return Ok(None);
                        }
                        Some(Err(e)) => match last {
                            // A repair failed: follow the previous path after all.
                            Some((p, ex)) => return Ok(Some(self.ready(p, ex))),
                            None => return Err(e),
                        },
                        Some(Ok(path)) => {
                            let exhausted = search.exhausted();
                            if !self.cfg.verify {
                                return Ok(Some(self.ready(path, exhausted)));
                            }
                            let sim = new_sim(ctx, &path, &self.cfg.controller);
                            self.planning = Some(Planning::Verify {
                                sim: Box::new(sim),
                                bans,
                                attempt,
                                exhausted,
                            });
                        }
                    }
                }
                Planning::Verify {
                    mut sim,
                    mut bans,
                    attempt,
                    exhausted,
                } => {
                    let Some((outcome, reached)) = sim
                        .step(
                            &world_of(ctx),
                            self.cfg.verify_ticks,
                            left(self.cfg.budget.time),
                        )
                        .map(|r| (r.outcome, r.reached))
                    else {
                        self.planning = Some(Planning::Verify {
                            sim,
                            bans,
                            attempt,
                            exhausted,
                        });
                        return Ok(None);
                    };
                    let path = sim.path();
                    if outcome == SimOutcome::Arrived {
                        let path = path.clone();
                        return Ok(Some(self.ready(path, exhausted)));
                    }
                    // Plan around the move the simulation failed.
                    let k = (reached + 1).min(path.steps.len() - 1);
                    let m = (path.steps[k.saturating_sub(1)].pos, path.steps[k].pos);
                    log::debug!("pathfinder: simulation {outcome:?} on move {m:?}");
                    if k == 0 || attempt + 1 >= VERIFY_ATTEMPTS || !bans.insert(m) {
                        let path = path.clone();
                        return Ok(Some(self.ready(path, exhausted)));
                    }
                    let goal = self.goal.clone().expect("planning without a goal");
                    let mut all = self.banned.clone();
                    all.extend(bans.iter().copied());
                    let search = self.pf.search(path.version, path.steps[0].pos, goal, all);
                    self.planning = Some(Planning::Search {
                        search: Box::new(search),
                        bans,
                        attempt: attempt + 1,
                        last: Some((path.clone(), exhausted)),
                    });
                }
            }
        }
    }

    fn ready(&mut self, path: Path, exhausted: bool) -> Controller {
        // An exhausted search knows every block reachable through the loaded terrain: if the
        // goal lies in loaded terrain too, its partial path is as close as one gets.
        self.unreachable = path.partial && exhausted && self.goal_loaded.unwrap_or(false);
        log::debug!(
            "pathfinder: {} steps, cost {:.0} ticks, {} nodes{}, planned in {} ticks",
            path.steps.len(),
            path.cost,
            path.expanded,
            if self.unreachable {
                ", partial (goal unreachable)"
            } else if path.partial {
                ", partial"
            } else {
                ""
            },
            self.plan_ticks
        );
        self.planning = None;
        Controller::new(path, self.cfg.controller.clone())
    }

    /// One tick: plans when needed, then aims and sets the keys (and digs / places). Returns
    /// the status after this tick.
    pub fn tick(&mut self, ctx: &mut TickContext<'_>) -> FollowStatus {
        self.step(ctx)
    }

    /// [`Follower::tick`] over any [`Io`].
    pub(crate) fn step<I: Io + ?Sized>(&mut self, ctx: &mut I) -> FollowStatus {
        if self.goal.is_none() || !matches!(self.status, FollowStatus::Moving) {
            return self.status;
        }
        if ctx.moved_by_server() {
            self.ctrl = None;
            self.planning = None;
        }
        if ctx.is_dead() {
            self.release(ctx);
            return self.status;
        }
        if self.ctrl.is_none() {
            if self.planning.is_none() {
                let Some(version) = ctx.version() else {
                    return self.fail(ctx, FollowError::UnsupportedVersion);
                };
                self.goal_loaded = self.goal.as_ref().map(|g| goal_loaded(g, ctx.world()));
                self.start_plan(ctx, version);
            }
            match self.work(ctx) {
                Ok(Some(c)) => self.ctrl = Some(c),
                Ok(None) => {
                    // Still planning: stand still; the movement packet goes out as usual.
                    self.release(ctx);
                    return self.status;
                }
                Err(e) => return self.fail(ctx, FollowError::Plan(e)),
            }
        }
        let obs = ctx.observation();
        let ctrl = self.ctrl.as_mut().expect("planned above");
        #[cfg(feature = "v26_1")]
        if let Some(p) = ctx.modern_player() {
            ctrl.set_modern_player(p);
        }
        let (yaw, pitch) = ctrl.aim(&obs, &world_of(ctx));
        let done = matches!(
            ctrl.progress(),
            Progress::Arrived | Progress::Stuck | Progress::OffPath
        );
        if !done {
            ctx.set_aim(yaw, pitch);
        }
        let look = ctx.planned_look();
        let cmd = ctrl.keys(&obs, look, &world_of(ctx));
        match ctrl.progress() {
            Progress::Arrived => {
                let path = ctrl.path();
                if !path.partial {
                    self.release(ctx);
                    self.status = FollowStatus::Arrived;
                    return self.status;
                }
                if self.unreachable {
                    // As close as it gets: no path exists through the loaded terrain.
                    return self.fail(ctx, FollowError::Plan(PlanError::NoPath));
                }
                // The end of a partial path: plan the rest from here. Count it against the
                // replan budget unless it got closer to the goal than any partial path before.
                let goal = self.goal.as_ref().expect("following without a goal");
                let d = path
                    .steps
                    .last()
                    .map_or(f64::INFINITY, |s| goal.distance(s.pos));
                if d < self.partial_best - 0.5 {
                    self.partial_best = d;
                } else {
                    self.replans += 1;
                }
                self.ctrl = None;
                self.release(ctx);
                if self.replans > self.cfg.max_replans {
                    return self.fail(ctx, FollowError::GaveUp);
                }
                return self.status;
            }
            Progress::Stuck | Progress::OffPath => {
                if let Some(m) = ctrl.current_move() {
                    log::debug!(
                        "pathfinder: {:?} on move {m:?}, replanning",
                        ctrl.progress()
                    );
                    self.banned.insert(m);
                }
                self.ctrl = None;
                self.replans += 1;
                self.release(ctx);
                if self.replans > self.cfg.max_replans {
                    return self.fail(ctx, FollowError::GaveUp);
                }
                return self.status;
            }
            Progress::Moving | Progress::Working => {}
        }
        ctx.set_keys(cmd.keys);
        if let Some(d) = cmd.dig {
            let on = ctx.targets(d);
            ctx.set_left_button(on);
            self.digging = true;
        } else if self.digging {
            ctx.set_left_button(false);
            self.digging = false;
        }
        if let Some(p) = cmd.place {
            if self.place_wait > 0 {
                self.place_wait -= 1;
            } else if ctx.place_on_top(p.against) {
                self.place_wait = 10;
            }
        } else {
            self.place_wait = 0;
        }
        self.status
    }
}

#[cfg(all(test, feature = "v1_8", feature = "v1_12_2", feature = "v1_16_5"))]
pub(crate) mod tests {
    //! The follower driven tick by tick in the physics (a simulated client): mazes, unreachable
    //! goals, gap jumps.

    use std::time::{Duration, Instant};

    use ghast_physics::Keys;

    use super::{FollowConfig, FollowError, FollowStatus, Follower, Io};
    use crate::physics::{Observation, P12, P16, P18, P26, Physics, SourceView};
    use crate::plan::{Config, Pathfinder, PlanError};
    use crate::sim::Turner;
    use crate::world::BlockSource;
    use crate::{Goal, Version};

    /// A simulated client: the player in `ghast-physics`, turning in whole mouse counts.
    pub(crate) struct SimIo<'w, P: Physics, W: BlockSource> {
        pub(crate) world: &'w W,
        pub(crate) state: P::State,
        turner: Turner,
        aim: (f32, f32),
        keys: Keys,
    }

    impl<'w, P: Physics, W: BlockSource> SimIo<'w, P, W> {
        pub(crate) fn new(world: &'w W, feet: [f64; 3]) -> Self {
            let mut state = P::new_state(feet, 0.0, 0.0);
            let view = SourceView::<P, W>::new(world);
            for _ in 0..3 {
                P::tick(&mut state, Keys::NONE, 0.0, 0.0, &view);
            }
            SimIo {
                world,
                state,
                turner: Turner::default(),
                aim: (0.0, 0.0),
                keys: Keys::NONE,
            }
        }

        /// Runs this tick's movement with what the follower asked for.
        pub(crate) fn advance(&mut self) {
            let o = P::observe(&self.state);
            let look = self.turner.step(o.yaw, o.pitch, self.aim);
            let view = SourceView::<P, W>::new(self.world);
            P::tick(&mut self.state, self.keys, look.0, look.1, &view);
        }

        pub(crate) fn feet(&self) -> [f64; 3] {
            P::observe(&self.state).pos
        }
    }

    impl<P: Physics, W: BlockSource> Io for SimIo<'_, P, W> {
        type World<'a>
            = W
        where
            Self: 'a;

        fn world(&self) -> &W {
            self.world
        }

        fn version(&self) -> Option<Version> {
            Some(P::VERSION)
        }

        fn observation(&self) -> Observation {
            let mut o = P::observe(&self.state);
            o.food = 20;
            o
        }

        fn moved_by_server(&self) -> bool {
            false
        }

        fn is_dead(&self) -> bool {
            false
        }

        fn set_aim(&mut self, yaw: f32, pitch: f32) {
            self.aim = (yaw, pitch);
        }

        fn planned_look(&self) -> (f32, f32) {
            let o = P::observe(&self.state);
            let mut t = self.turner;
            t.step(o.yaw, o.pitch, self.aim)
        }

        fn set_keys(&mut self, keys: Keys) {
            self.keys = keys;
        }

        fn left_button(&self) -> bool {
            false
        }

        fn set_left_button(&mut self, _: bool) {}

        fn targets(&self, _: [i32; 3]) -> bool {
            false
        }

        fn place_on_top(&mut self, _: [i32; 3]) -> bool {
            false
        }

        fn modern_player(&self) -> Option<&ghast_physics::modern::PlayerState> {
            P::as_modern(&self.state)
        }
    }

    /// How a drive went.
    #[derive(Debug)]
    pub(crate) struct Drive {
        pub(crate) status: FollowStatus,
        pub(crate) ticks: u32,
        /// Ticks the follower stood still planning.
        pub(crate) planning_ticks: u32,
        /// The longest follower tick (wall clock).
        pub(crate) worst_tick: Duration,
        pub(crate) plans: u64,
        pub(crate) end: [f64; 3],
    }

    /// Drives `follower` to `goal` from standing at `feet`, at most `max` ticks.
    pub(crate) fn drive<P: Physics, W: BlockSource>(
        world: &W,
        feet: [f64; 3],
        mut follower: Follower,
        goal: Goal,
        max: u32,
    ) -> Drive {
        let mut io = SimIo::<P, W>::new(world, feet);
        follower.goto(goal);
        let mut d = Drive {
            status: FollowStatus::Moving,
            ticks: 0,
            planning_ticks: 0,
            worst_tick: Duration::ZERO,
            plans: 0,
            end: feet,
        };
        while d.ticks < max {
            let t0 = Instant::now();
            d.status = follower.step(&mut io);
            // The first tick of a process also measures the move costs once.
            if d.ticks > 0 {
                d.worst_tick = d.worst_tick.max(t0.elapsed());
            }
            if follower.is_planning() {
                d.planning_ticks += 1;
            }
            d.ticks += 1;
            if d.status != FollowStatus::Moving {
                break;
            }
            io.advance();
        }
        d.plans = follower.plans();
        d.end = io.feet();
        d
    }

    const STONE18: u16 = 1 << 4;

    fn rng(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    /// A perfect maze (recursive backtracker) of `n` × `n` cells with `h`-high stone walls on
    /// a stone floor at y = 63; grid (2n + 1)², corner (ox, oz). Cell (i, j) has its feet
    /// block at (ox + 2i + 1, 64, oz + 2j + 1). Entrances: west of cell (0, 0), east of
    /// cell (n - 1, n - 1).
    pub(crate) fn maze<B: Copy>(
        w: &mut ghast_physics::MemoryWorld<B>,
        (ox, oz): (i32, i32),
        n: i32,
        h: i32,
        seed: u64,
    ) {
        let size = 2 * n + 1;
        w.fill_raw(
            [ox, 64, oz],
            [ox + size - 1, 63 + h, oz + size - 1],
            STONE18,
        );
        let carve = |w: &mut ghast_physics::MemoryWorld<B>, gx: i32, gz: i32| {
            w.fill_raw([ox + gx, 64, oz + gz], [ox + gx, 63 + h, oz + gz], 0);
        };
        let mut s = seed | 1;
        let mut seen = vec![false; (n * n) as usize];
        let mut stack = vec![(0, 0)];
        seen[0] = true;
        carve(w, 1, 1);
        while let Some(&(cx, cz)) = stack.last() {
            let nb: Vec<(i32, i32)> = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .map(|(dx, dz)| (cx + dx, cz + dz))
                .filter(|&(x, z)| x >= 0 && z >= 0 && x < n && z < n && !seen[(z * n + x) as usize])
                .collect();
            if nb.is_empty() {
                stack.pop();
                continue;
            }
            let (nx, nz) = nb[(rng(&mut s) % nb.len() as u64) as usize];
            seen[(nz * n + nx) as usize] = true;
            carve(w, 2 * nx + 1, 2 * nz + 1);
            carve(w, cx + nx + 1, cz + nz + 1);
            stack.push((nx, nz));
        }
        carve(w, 0, 1);
        carve(w, size - 1, size - 2);
    }

    /// A flat stone world over chunks -r..=r.
    pub(crate) fn flat(r: i32) -> ghast_physics::v1_8::MemoryWorld {
        flat_in(ghast_physics::v1_8::memory_world(), r)
    }

    /// [`flat`] in a world of any version with 1.8 raw ids (1.8 and 1.12).
    pub(crate) fn flat_in<B: Copy>(
        mut w: ghast_physics::MemoryWorld<B>,
        r: i32,
    ) -> ghast_physics::MemoryWorld<B> {
        w.load_chunks(-r, -r, r, r);
        w.fill_raw(
            [-16 * r, 63, -16 * r],
            [16 * r + 15, 63, 16 * r + 15],
            STONE18,
        );
        w
    }

    fn follower() -> Follower {
        Follower::new(Pathfinder::new(Config::default()), FollowConfig::default())
    }

    /// Mazes of 15, 31 and 63 blocks: the bot starts outside, the goal is the centre cell,
    /// whose way through the maze is many times the straight distance.
    #[test]
    fn mazes_are_solved() {
        for (n, seed) in [(7, 3u64), (15, 5), (31, 7)] {
            let mut w = flat(6);
            maze(&mut w, (-n, -n), n, 2, seed);
            let centre = [
                if n % 2 == 1 { 0 } else { 1 },
                64,
                if n % 2 == 1 { 0 } else { 1 },
            ];
            let from = [f64::from(-n - 4) + 0.5, 64.0, f64::from(n + 4) + 0.5];
            let d = drive::<P18, _>(
                &w,
                from,
                follower(),
                Goal::Near {
                    pos: centre,
                    radius: 1.0,
                },
                20_000,
            );
            eprintln!("maze {}: {d:?}", 2 * n + 1);
            assert_eq!(d.status, FollowStatus::Arrived, "maze {}", 2 * n + 1);
            let end = Pathfinder::start_block(d.end);
            assert!(
                Goal::Near {
                    pos: centre,
                    radius: 1.0
                }
                .is_reached(end)
            );
        }
    }

    /// Unreachable goals end cleanly: the bot walks to the reachable block closest to the
    /// goal, then fails with `NoPath`, after a single search (no second full search from the
    /// closest block).
    #[test]
    fn unreachable_goals_end_after_one_search() {
        // A goal in the air (a flying player) and a goal sealed inside a maze.
        let w = flat(4);
        let d = drive::<P18, _>(
            &w,
            [-20.5, 64.0, 3.5],
            follower(),
            Goal::Near {
                pos: [10, 70, -4],
                radius: 2.0,
            },
            3000,
        );
        assert_eq!(
            d.status,
            FollowStatus::Failed(FollowError::Plan(PlanError::NoPath)),
            "{d:?}"
        );
        assert_eq!(d.plans, 1, "{d:?}");
        let end = Pathfinder::start_block(d.end);
        assert!((end[0] - 10).abs() <= 1 && (end[2] + 4).abs() <= 1, "{d:?}");
        let mut w = flat(4);
        maze(&mut w, (-15, -15), 15, 2, 11);
        w.fill_raw([-2, 64, -2], [2, 65, 2], STONE18);
        w.fill_raw([0, 64, 0], [0, 65, 0], 0);
        let d = drive::<P18, _>(
            &w,
            [-18.5, 64.0, 18.5],
            follower(),
            Goal::Block([0, 64, 0]),
            20_000,
        );
        assert_eq!(
            d.status,
            FollowStatus::Failed(FollowError::Plan(PlanError::NoPath)),
            "{d:?}"
        );
        assert_eq!(d.plans, 1, "{d:?}");
    }

    /// A node budget far below what the maze needs: partial paths into dead ends must not make
    /// the bot wander (and search) forever.
    #[test]
    fn capped_searches_give_up() {
        let mut w = flat(4);
        maze(&mut w, (-31, -31), 31, 2, 13);
        let f = Follower::new(
            Pathfinder::new(Config::default().with_max_nodes(1500)),
            FollowConfig::default(),
        );
        let d = drive::<P18, _>(&w, [-34.5, 64.0, -29.5], f, Goal::Block([0, 64, 0]), 40_000);
        eprintln!("{d:?}");
        assert_ne!(d.status, FollowStatus::Moving, "still wandering: {d:?}");
        assert!(d.plans <= 20, "{d:?}");
    }

    /// The per-tick work stays in the budget on the biggest maze. The wall-clock check runs
    /// only with `GHAST_TIMING` set, in release builds (debug builds are ~10 times slower,
    /// and a loaded machine stretches any tick).
    #[test]
    fn per_tick_budget_on_a_big_maze() {
        let mut w = flat(10);
        maze(&mut w, (-63, -63), 63, 2, 17);
        let budget = FollowConfig::default().budget;
        let d = drive::<P18, _>(
            &w,
            [-67.5, 64.0, 67.5],
            follower(),
            Goal::Near {
                pos: [1, 64, 1],
                radius: 1.0,
            },
            40_000,
        );
        eprintln!("maze 127, view distance 10: {d:?}");
        assert_eq!(d.status, FollowStatus::Arrived, "{d:?}");
        if !cfg!(debug_assertions) && std::env::var_os("GHAST_TIMING").is_some() {
            let max = budget.time.unwrap() + Duration::from_millis(3);
            assert!(d.worst_tick <= max, "{d:?}");
        }
        let _ = P16::VERSION;
    }

    /// The parkour corridor of `lab/pathfinder-course.sh` (superflat: grass y 3, feet y 4),
    /// east and back west, driven by the follower.
    #[test]
    fn the_lab_parkour_corridor() {
        let w = corridor(ghast_physics::v1_8::memory_world());
        run_corridor::<P18, _>(&w);
        let w = corridor(ghast_physics::v1_12::memory_world());
        run_corridor::<P12, _>(&w);
    }

    /// The 1.12 rules through mazes and an unreachable goal as well.
    #[test]
    fn mazes_on_1_12() {
        let n = 15;
        let mut w = flat_in(ghast_physics::v1_12::memory_world(), 6);
        maze(&mut w, (-n, -n), n, 2, 5);
        let from = [f64::from(-n - 4) + 0.5, 64.0, f64::from(n + 4) + 0.5];
        let goal = Goal::Near {
            pos: [1, 64, 1],
            radius: 1.0,
        };
        let d = drive::<P12, _>(&w, from, follower(), goal, 20_000);
        assert_eq!(d.status, FollowStatus::Arrived, "{d:?}");
        let air = Goal::Near {
            pos: [1, 70, 1],
            radius: 2.0,
        };
        let d = drive::<P12, _>(&w, from, follower(), air, 20_000);
        assert_eq!(
            d.status,
            FollowStatus::Failed(FollowError::Plan(PlanError::NoPath)),
            "{d:?}"
        );
        assert_eq!(d.plans, 1, "{d:?}");
    }

    fn corridor<B: Copy>(mut w: ghast_physics::MemoryWorld<B>) -> ghast_physics::MemoryWorld<B> {
        w.load_chunks(0, -2, 9, 2);
        w.fill_raw([0, 0, -32], [159, 0, 47], 7 << 4);
        w.fill_raw([0, 1, -32], [159, 2, 47], 3 << 4);
        w.fill_raw([0, 3, -32], [159, 3, 47], 2 << 4);
        for z in [-2, 2] {
            w.fill_raw([60, 1, z], [123, 7, z], STONE18);
        }
        w.fill_raw([123, 4, -2], [123, 7, 2], STONE18);
        let trench = |w: &mut ghast_physics::MemoryWorld<B>, a: i32, b: i32| {
            w.fill_raw([a, 1, -1], [b, 3, 1], 0);
        };
        trench(&mut w, 66, 66);
        trench(&mut w, 71, 72);
        trench(&mut w, 77, 79);
        trench(&mut w, 84, 84);
        w.fill_raw([85, 4, -1], [89, 4, 1], STONE18);
        trench(&mut w, 90, 91);
        trench(&mut w, 96, 111);
        for (x, top) in [(98, 3), (100, 3), (104, 3), (107, 4), (109, 3)] {
            w.fill_raw([x, 1, 0], [x, top, 0], STONE18);
        }
        w
    }

    /// A 26.1 block's state id.
    fn id26(spec: &str) -> u16 {
        ghast_data::v26_1::BlockState::parse(spec)
            .unwrap_or_else(|| panic!("{spec}"))
            .id()
    }

    /// The lab corridor in a 26.1 world (same layout, 26.1 ids).
    fn corridor_26() -> ghast_physics::modern::MemoryWorld26 {
        let mut w = ghast_physics::modern::memory_world_26_1();
        let stone = id26("stone");
        w.load_chunks(0, -2, 9, 2);
        w.fill_raw([0, 0, -32], [159, 0, 47], id26("bedrock"));
        w.fill_raw([0, 1, -32], [159, 2, 47], id26("dirt"));
        w.fill_raw([0, 3, -32], [159, 3, 47], id26("grass_block"));
        for z in [-2, 2] {
            w.fill_raw([60, 1, z], [123, 7, z], stone);
        }
        w.fill_raw([123, 4, -2], [123, 7, 2], stone);
        let trench = |w: &mut ghast_physics::modern::MemoryWorld26, a: i32, b: i32| {
            w.fill_raw([a, 1, -1], [b, 3, 1], 0);
        };
        trench(&mut w, 66, 66);
        trench(&mut w, 71, 72);
        trench(&mut w, 77, 79);
        trench(&mut w, 84, 84);
        w.fill_raw([85, 4, -1], [89, 4, 1], stone);
        trench(&mut w, 90, 91);
        trench(&mut w, 96, 111);
        for (x, top) in [(98, 3), (100, 3), (104, 3), (107, 4), (109, 3)] {
            w.fill_raw([x, 1, 0], [x, top, 0], stone);
        }
        w
    }

    /// The parkour corridor driven by the follower on 26.1.
    #[test]
    fn the_lab_parkour_corridor_26_1() {
        run_corridor::<P26, _>(&corridor_26());
    }

    /// 26.1: a maze (in negative Y, under the overworld's old floor) and an unreachable goal.
    #[test]
    fn mazes_on_26_1() {
        let n = 15;
        let (y, stone) = (-40, id26("stone"));
        let mut w = ghast_physics::modern::memory_world_26_1();
        w.load_chunks(-6, -6, 6, 6);
        w.fill_raw([-96, y - 1, -96], [111, y - 1, 111], stone);
        let size = 2 * n + 1;
        let (ox, oz) = (-n, -n);
        w.fill_raw([ox, y, oz], [ox + size - 1, y + 1, oz + size - 1], stone);
        let mut s = 5u64 | 1;
        let mut seen = vec![false; (n * n) as usize];
        let mut stack = vec![(0, 0)];
        seen[0] = true;
        let carve = |w: &mut ghast_physics::modern::MemoryWorld26, gx: i32, gz: i32| {
            w.fill_raw([ox + gx, y, oz + gz], [ox + gx, y + 1, oz + gz], 0);
        };
        carve(&mut w, 1, 1);
        while let Some(&(cx, cz)) = stack.last() {
            let nb: Vec<(i32, i32)> = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .map(|(dx, dz)| (cx + dx, cz + dz))
                .filter(|&(x, z)| x >= 0 && z >= 0 && x < n && z < n && !seen[(z * n + x) as usize])
                .collect();
            if nb.is_empty() {
                stack.pop();
                continue;
            }
            let (nx, nz) = nb[(rng(&mut s) % nb.len() as u64) as usize];
            seen[(nz * n + nx) as usize] = true;
            carve(&mut w, 2 * nx + 1, 2 * nz + 1);
            carve(&mut w, cx + nx + 1, cz + nz + 1);
            stack.push((nx, nz));
        }
        carve(&mut w, 0, 1);
        carve(&mut w, size - 1, size - 2);
        let from = [
            f64::from(-n - 4) + 0.5,
            f64::from(y),
            f64::from(n + 4) + 0.5,
        ];
        let goal = Goal::Near {
            pos: [1, y, 1],
            radius: 1.0,
        };
        let d = drive::<P26, _>(&w, from, follower(), goal, 20_000);
        eprintln!("26.1 maze 31: {d:?}");
        assert_eq!(d.status, FollowStatus::Arrived, "{d:?}");
        let air = Goal::Near {
            pos: [1, y + 6, 1],
            radius: 2.0,
        };
        let d = drive::<P26, _>(&w, from, follower(), air, 20_000);
        assert_eq!(
            d.status,
            FollowStatus::Failed(FollowError::Plan(PlanError::NoPath)),
            "{d:?}"
        );
        assert_eq!(d.plans, 1, "{d:?}");
    }

    /// The live attributes drive the look-ahead and the verification: with jump strength 0
    /// (a server attribute) a one-block step cannot be climbed, and the follower plans around
    /// it instead of jumping in place forever.
    #[test]
    fn live_attributes_26_1() {
        use ghast_physics::modern::AttributeKind;
        let stone = id26("stone");
        let mut w = ghast_physics::modern::memory_world_26_1();
        w.load_chunks(-2, -2, 2, 2);
        w.fill_raw([-32, 63, -32], [31, 63, 31], stone);
        // A wall one block high across the way, with a gap at z = 6.
        w.fill_raw([4, 64, -32], [4, 64, 31], stone);
        w.set_raw(4, 64, 6, 0);
        let goal = Goal::Block([8, 64, 0]);
        // Default attributes: straight over the step.
        let d = drive::<P26, _>(&w, [0.5, 64.0, 0.5], follower(), goal.clone(), 2000);
        assert_eq!(d.status, FollowStatus::Arrived, "{d:?}");
        // No jumping: around through the gap.
        let mut io = SimIo::<P26, _>::new(&w, [0.5, 64.0, 0.5]);
        io.state
            .apply_attribute(AttributeKind::JumpStrength, 0.0, &[]);
        let mut f = follower();
        f.goto(goal);
        let mut through_gap = false;
        let mut status = FollowStatus::Moving;
        for _ in 0..3000 {
            status = f.step(&mut io);
            if status != FollowStatus::Moving {
                break;
            }
            io.advance();
            let p = io.feet();
            through_gap |= p[0].floor() as i32 == 4 && p[2].floor() as i32 == 6;
        }
        assert_eq!(status, FollowStatus::Arrived, "{:?}", io.feet());
        assert!(through_gap, "the step was not avoided");
    }

    fn run_corridor<P: Physics, W: BlockSource>(w: &W) {
        let d = drive::<P, _>(
            w,
            [62.5, 4.0, 0.5],
            follower(),
            Goal::Xz { x: 119, z: 0 },
            4000,
        );
        eprintln!("{:?} east: {d:?}", P::VERSION);
        assert_eq!(d.status, FollowStatus::Arrived, "{d:?}");
        let d = drive::<P, _>(w, d.end, follower(), Goal::Xz { x: 62, z: 0 }, 4000);
        eprintln!("{:?} west: {d:?}", P::VERSION);
        assert_eq!(d.status, FollowStatus::Arrived, "{d:?}");
    }
}
