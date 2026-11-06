//! Following a path from a `ghast_client::Behavior`: plan, follow, replan.

use std::collections::HashSet;

use ghast_client::{BlockPos, Buttons, Controls, Face, TickContext};

use crate::control::{Controller, ControllerConfig, Progress};
use crate::goal::Goal;
use crate::physics::Observation;
use crate::plan::{Pathfinder, PlanError};

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
}

impl Default for FollowConfig {
    fn default() -> Self {
        FollowConfig {
            controller: ControllerConfig::default(),
            max_replans: 8,
            verify: true,
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
            partial_end: None,
            place_wait: 0,
            digging: false,
            plans: 0,
        }
    }

    /// Sets a new goal (planned on the next [`Follower::tick`]).
    pub fn goto(&mut self, goal: Goal) {
        self.goal = Some(goal);
        self.ctrl = None;
        self.status = FollowStatus::Moving;
        self.replans = 0;
        self.banned.clear();
        self.partial_end = None;
    }

    /// Drops the goal and releases the keys and buttons (in this tick).
    pub fn stop(&mut self, ctx: &mut TickContext<'_>) {
        self.goal = None;
        self.ctrl = None;
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
        self.status = FollowStatus::Failed(e);
        self.release(ctx);
        self.status
    }

    fn plan(&mut self, ctx: &TickContext<'_>) -> Result<Controller, PlanError> {
        let goal = self.goal.as_ref().expect("planning without a goal");
        let p = ctx.pose().pos;
        let start = Pathfinder::start_block([p.x, p.y, p.z]);
        let version = crate::Version::from(ctx.version());
        self.plans += 1;
        let path = if self.cfg.verify {
            let mut banned = self.banned.clone();
            let (path, _) = self.pf.plan_verified_avoiding(
                version,
                ctx.world(),
                start,
                goal,
                &mut banned,
                3,
                &self.cfg.controller,
            )?;
            path
        } else {
            self.pf
                .plan_avoiding(version, ctx.world(), start, goal, &self.banned)?
        };
        log::debug!(
            "pathfinder: {} steps, cost {:.0} ticks, {} nodes{}",
            path.steps.len(),
            path.cost,
            path.expanded,
            if path.partial { ", partial" } else { "" }
        );
        Ok(Controller::new(path, self.cfg.controller.clone()))
    }

    /// One tick: plans when needed, then aims and sets the keys (and digs / places). Returns
    /// the status after this tick.
    pub fn tick(&mut self, ctx: &mut TickContext<'_>) -> FollowStatus {
        if self.goal.is_none() || !matches!(self.status, FollowStatus::Moving) {
            return self.status;
        }
        if !ctx.events().teleports.is_empty() || ctx.events().respawned {
            self.ctrl = None;
        }
        if ctx.is_dead() {
            self.release(ctx);
            return self.status;
        }
        if self.ctrl.is_none() {
            match self.plan(ctx) {
                Ok(c) => self.ctrl = Some(c),
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
