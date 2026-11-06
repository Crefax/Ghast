//! Following a path from a `ghast_client::Behavior`: plan, follow, replan.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use ghast_client::{BlockPos, Buttons, Controls, Face, TickContext};

use crate::control::{Controller, ControllerConfig, Progress};
use crate::goal::Goal;
use crate::physics::Observation;
use crate::plan::{Budget, Path, Pathfinder, PlanError, Search};
use crate::sim::{SimOutcome, Simulation};

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
}

impl std::fmt::Display for FollowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FollowError::Plan(e) => write!(f, "planning failed: {e}"),
            FollowError::GaveUp => f.write_str("gave up after too many replans"),
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
    /// Where the last partial path ended (no progress beyond it = blocked).
    partial_end: Option<[i32; 3]>,
    /// Ticks to wait after a place was sent.
    place_wait: u32,
    digging: bool,
    plans: u64,
    /// A plan in progress (spread over ticks).
    planning: Option<Planning>,
    /// Ticks the current (or last) plan took.
    plan_ticks: u32,
}

/// A plan in progress: the search, then the simulated check.
#[derive(Clone, Debug)]
enum Planning {
    Search {
        search: Box<Search>,
        /// Moves the verification failed (on top of [`Follower::banned`]).
        bans: HashSet<([i32; 3], [i32; 3])>,
        attempt: u32,
        /// The previous attempt's path (followed if a later search fails).
        last: Option<Path>,
    },
    Verify {
        sim: Box<Simulation>,
        bans: HashSet<([i32; 3], [i32; 3])>,
        attempt: u32,
    },
}

/// Plans per goal and replan: the first one and two repairs after failed simulations.
const VERIFY_ATTEMPTS: u32 = 3;

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
            partial_end: None,
            place_wait: 0,
            digging: false,
            plans: 0,
            planning: None,
            plan_ticks: 0,
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
        self.partial_end = None;
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

    fn release(&mut self, ctx: &mut TickContext<'_>) {
        ctx.set_controls(Controls::NONE);
        if self.digging || ctx.buttons().left {
            ctx.set_buttons(Buttons::default());
        }
        self.digging = false;
    }

    fn fail(&mut self, ctx: &mut TickContext<'_>, e: FollowError) -> FollowStatus {
        self.ctrl = None;
        self.planning = None;
        self.status = FollowStatus::Failed(e);
        self.release(ctx);
        self.status
    }

    /// Starts a plan from the player's feet block.
    fn start_plan(&mut self, ctx: &TickContext<'_>) {
        let goal = self.goal.clone().expect("planning without a goal");
        let p = ctx.pose().pos;
        let start = Pathfinder::start_block([p.x, p.y, p.z]);
        let version = crate::Version::from(ctx.version());
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
    fn work(&mut self, ctx: &TickContext<'_>) -> Result<Option<Controller>, PlanError> {
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
                    match search.step(ctx.world(), budget) {
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
                            Some(p) => return Ok(Some(self.ready(p))),
                            None => return Err(e),
                        },
                        Some(Ok(path)) => {
                            if !self.cfg.verify {
                                return Ok(Some(self.ready(path)));
                            }
                            let sim = Simulation::new(&path, &self.cfg.controller);
                            self.planning = Some(Planning::Verify {
                                sim: Box::new(sim),
                                bans,
                                attempt,
                            });
                        }
                    }
                }
                Planning::Verify {
                    mut sim,
                    mut bans,
                    attempt,
                } => {
                    let Some((outcome, reached)) = sim
                        .step(
                            ctx.world(),
                            self.cfg.verify_ticks,
                            left(self.cfg.budget.time),
                        )
                        .map(|r| (r.outcome, r.reached))
                    else {
                        self.planning = Some(Planning::Verify { sim, bans, attempt });
                        return Ok(None);
                    };
                    let path = sim.path();
                    if outcome == SimOutcome::Arrived {
                        let path = path.clone();
                        return Ok(Some(self.ready(path)));
                    }
                    // Plan around the move the simulation failed.
                    let k = (reached + 1).min(path.steps.len() - 1);
                    let m = (path.steps[k.saturating_sub(1)].pos, path.steps[k].pos);
                    log::debug!("pathfinder: simulation {outcome:?} on move {m:?}");
                    if k == 0 || attempt + 1 >= VERIFY_ATTEMPTS || !bans.insert(m) {
                        let path = path.clone();
                        return Ok(Some(self.ready(path)));
                    }
                    let goal = self.goal.clone().expect("planning without a goal");
                    let mut all = self.banned.clone();
                    all.extend(bans.iter().copied());
                    let search = self.pf.search(path.version, path.steps[0].pos, goal, all);
                    self.planning = Some(Planning::Search {
                        search: Box::new(search),
                        bans,
                        attempt: attempt + 1,
                        last: Some(path.clone()),
                    });
                }
            }
        }
    }

    fn ready(&mut self, path: Path) -> Controller {
        log::debug!(
            "pathfinder: {} steps, cost {:.0} ticks, {} nodes{}, planned in {} ticks",
            path.steps.len(),
            path.cost,
            path.expanded,
            if path.partial { ", partial" } else { "" },
            self.plan_ticks
        );
        self.planning = None;
        Controller::new(path, self.cfg.controller.clone())
    }

    /// One tick: plans when needed, then aims and sets the keys (and digs / places). Returns
    /// the status after this tick.
    pub fn tick(&mut self, ctx: &mut TickContext<'_>) -> FollowStatus {
        if self.goal.is_none() || !matches!(self.status, FollowStatus::Moving) {
            return self.status;
        }
        if !ctx.events().teleports.is_empty() || ctx.events().respawned {
            self.ctrl = None;
            self.planning = None;
        }
        if ctx.is_dead() {
            self.release(ctx);
            return self.status;
        }
        if self.ctrl.is_none() {
            if self.planning.is_none() {
                self.start_plan(ctx);
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
        let pose = *ctx.pose();
        let obs = Observation {
            pos: [pose.pos.x, pose.pos.y, pose.pos.z],
            vel: [pose.motion.x, pose.motion.y, pose.motion.z],
            on_ground: pose.on_ground,
            yaw: pose.yaw,
            pitch: pose.pitch,
            sprinting: pose.sprinting,
            food: ctx.health().food,
        };
        let ctrl = self.ctrl.as_mut().expect("planned above");
        let (yaw, pitch) = ctrl.aim(&obs, ctx.world());
        let done = matches!(
            ctrl.progress(),
            Progress::Arrived | Progress::Stuck | Progress::OffPath
        );
        if !done {
            ctx.set_aim(yaw, pitch);
        }
        let look = ctx.planned_look();
        let cmd = ctrl.keys(&obs, look, ctx.world());
        match ctrl.progress() {
            Progress::Arrived => {
                let path = ctrl.path();
                if !path.partial {
                    self.release(ctx);
                    self.status = FollowStatus::Arrived;
                    return self.status;
                }
                // The end of a partial path: plan the rest from here.
                let end = path.steps.last().map(|s| s.pos);
                if end.is_some() && end == self.partial_end {
                    self.replans += 1;
                }
                self.partial_end = end;
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
        let k = cmd.keys;
        ctx.set_controls(Controls {
            forward: k.forward,
            back: k.back,
            left: k.left,
            right: k.right,
            jump: k.jump,
            sneak: k.sneak,
            sprint: k.sprint,
        });
        if let Some(d) = cmd.dig {
            let pos = BlockPos::new(d[0], d[1], d[2]);
            let on = ctx.target().is_some_and(|t| t.pos == pos);
            ctx.set_buttons(Buttons {
                left: on,
                right: false,
            });
            self.digging = true;
        } else if self.digging {
            ctx.set_buttons(Buttons::default());
            self.digging = false;
        }
        if let Some(p) = cmd.place {
            if self.place_wait > 0 {
                self.place_wait -= 1;
            } else {
                let against = BlockPos::new(p.against[0], p.against[1], p.against[2]);
                if let Some(hit) = ctx
                    .target()
                    .filter(|t| t.pos == against && t.face == Face::Up)
                    && ctx.place(against, Face::Up, hit.hit)
                {
                    self.place_wait = 10;
                }
            }
        } else {
            self.place_wait = 0;
        }
        self.status
    }
}
