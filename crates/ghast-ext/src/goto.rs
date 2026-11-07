//! Walking along planned paths (feature `pathfinder`).

use ghast_client::{Buttons, Controls, Vec3};
use ghast_pathfinder::{
    Config, FollowConfig, FollowError, FollowStatus, Follower, Goal, Pathfinder,
};

use crate::{Bot, Error};

/// State of [`Bot::follow_path`] between ticks.
struct Chase {
    follower: Follower,
    /// The entity's feet block the current plan goes to.
    target: Option<[i32; 3]>,
    /// The entity's feet block when following last failed (out of reach): wait until it moves.
    failed_at: Option<[i32; 3]>,
}

/// Feet block of a position.
fn feet_block(p: Vec3) -> [i32; 3] {
    Pathfinder::start_block([p.x, p.y, p.z])
}

/// Largest horizontal or vertical block distance.
fn blocks_apart(a: [i32; 3], b: [i32; 3]) -> i32 {
    (a[0] - b[0])
        .abs()
        .max((a[1] - b[1]).abs())
        .max((a[2] - b[2]).abs())
}

/// Releases the keys and buttons when a walk ends early.
struct Release(Option<Bot>);

impl Drop for Release {
    fn drop(&mut self) {
        if let Some(bot) = self.0.take() {
            bot.act(|ctx| {
                ctx.set_controls(Controls::NONE);
                if ctx.buttons().left {
                    ctx.set_buttons(Buttons::default());
                }
            });
        }
    }
}

impl Bot {
    /// Walks to `goal` along a path planned by [`ghast_pathfinder`] with the default
    /// [`Config`] (sprinting, gap jumps, swimming, ladders; no digging or placing) and
    /// followed by its [`Follower`]: every plan is first run through the vanilla-exact
    /// physics, moves the simulation fails are planned around, and the follower replans when
    /// it gets stuck or leaves the path.
    ///
    /// Fails with [`Error::NoPath`] when no path exists (or the start is not loaded),
    /// [`Error::Stuck`] after too many replans, and the usual interrupts (teleport / death,
    /// see [`crate::Ext::interrupt_on_teleport`]). Unloaded terrain is never treated as air:
    /// a goal beyond it is approached through partial paths. Dropping the future releases
    /// the keys.
    ///
    /// A long plan is spread over several ticks (by default at most 2000 nodes and 5 ms of
    /// planning per tick, see [`FollowConfig::budget`]); the bot stands still meanwhile and
    /// the client keeps sending its movement packet every tick.
    ///
    /// ```no_run
    /// use ghast_ext::pathfinder::Goal;
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.goto(Goal::Block([20, 64, -5])).await?;
    /// bot.goto(Goal::Near { pos: [0, 64, 0], radius: 3.0 }).await?;
    /// # Ok(()) }
    /// ```
    pub async fn goto(&self, goal: Goal) -> Result<(), Error> {
        let follower = Follower::new(Pathfinder::new(Config::default()), FollowConfig::default());
        self.goto_with(follower, goal).await
    }

    /// [`Bot::goto`] with a configured [`Follower`] (planner options such as digging or
    /// placing, controller options, replan budget).
    ///
    /// ```no_run
    /// use ghast_ext::pathfinder::{Config, DigCost, FollowConfig, Follower, Goal, Pathfinder};
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let pf = Pathfinder::new(Config::default().with_dig(Some(DigCost::default())));
    /// let follower = Follower::new(pf, FollowConfig::default());
    /// bot.goto_with(follower, Goal::Xz { x: 100, z: 40 }).await?;
    /// # Ok(()) }
    /// ```
    pub async fn goto_with(&self, mut follower: Follower, goal: Goal) -> Result<(), Error> {
        let mark = self.mark();
        let mut release = Release(Some(self.clone()));
        follower.goto(goal);
        loop {
            let (f, status) = self
                .with(move |ctx| {
                    let mut f = follower;
                    let s = f.tick(ctx);
                    (f, s)
                })
                .await?;
            follower = f;
            match status {
                FollowStatus::Arrived => {
                    release.0 = None;
                    return Ok(());
                }
                FollowStatus::Failed(FollowError::Plan(e)) => {
                    log::debug!("goto: {e}");
                    return Err(Error::NoPath);
                }
                FollowStatus::Failed(_) => return Err(Error::Stuck),
                _ => {}
            }
            self.next_tick().await;
            self.check(mark)?;
        }
    }

    /// Follows entity `id` along planned paths (the planner and follower of [`Bot::goto`]),
    /// staying within `range` blocks of it: through mazes, over gaps, around walls (unlike
    /// [`Bot::follow`], which walks in a straight line). The bot plans again when the entity
    /// has moved more than `range` (at least 2) blocks from where the current path goes, at
    /// most one plan at a time; within range it stands and looks at the entity. When the
    /// entity is out of reach (flying, across a gap too wide), the bot walks to the reachable
    /// block closest to it and waits there until the entity moves.
    ///
    /// Runs until the entity is gone ([`Error::NoSuchEntity`]) or the task is cancelled (e.g.
    /// with [`Bot::timeout`] or [`crate::JoinHandle::abort`]). Needs
    /// [`ghast_client::Interests::ENTITIES`]. Dropping the future releases the keys.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// if let Some(p) = bot.player("Steve").await? {
    ///     let _ = bot.timeout(20 * 60, bot.follow_path(p.id, 2.0)).await; // one minute
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn follow_path(&self, id: i32, range: f64) -> Result<(), Error> {
        let follower = Follower::new(Pathfinder::new(Config::default()), FollowConfig::default());
        self.follow_path_with(follower, id, range).await
    }

    /// [`Bot::follow_path`] with a configured [`Follower`].
    pub async fn follow_path_with(
        &self,
        follower: Follower,
        id: i32,
        range: f64,
    ) -> Result<(), Error> {
        let mark = self.mark();
        let _release = Release(Some(self.clone()));
        let mut chase = Chase {
            follower,
            target: None,
            failed_at: None,
        };
        let retarget = (range.ceil() as i32).max(2);
        loop {
            chase = self
                .with(move |ctx| -> Result<Chase, Error> {
                    let mut c = chase;
                    let e = crate::find::entity_info(ctx, id).ok_or(Error::NoSuchEntity(id))?;
                    let goal_block = feet_block(e.pos);
                    let me = ctx.pose().pos;
                    let (dx, dy, dz) = (me.x - e.pos.x, me.y - e.pos.y, me.z - e.pos.z);
                    // Within range, or on a block the goal accepts (block centres).
                    let goal = Goal::Near {
                        pos: goal_block,
                        radius: range,
                    };
                    let near = (dx * dx + dy * dy + dz * dz).sqrt() <= range
                        || goal.is_reached(feet_block(me));
                    let moving = c.follower.status() == FollowStatus::Moving;
                    if near && !moving {
                        // Within range: watch it.
                        let (yaw, pitch) = crate::yaw_pitch(
                            ctx.eyes(ctx.pose()),
                            Vec3::new(e.pos.x, e.pos.y + 1.6, e.pos.z),
                        );
                        ctx.set_aim(yaw, pitch);
                        c.target = None;
                        return Ok(c);
                    }
                    let stale = c
                        .target
                        .is_none_or(|t| blocks_apart(t, goal_block) > retarget);
                    let waiting = c
                        .failed_at
                        .is_some_and(|f| blocks_apart(f, goal_block) <= 1);
                    if stale
                        && !waiting
                        && (!c.follower.is_planning() || c.target.is_none())
                        && (!near || c.target.is_none())
                    {
                        c.follower.goto(goal);
                        c.target = Some(goal_block);
                        c.failed_at = None;
                    }
                    if c.follower.status() == FollowStatus::Moving {
                        match c.follower.tick(ctx) {
                            FollowStatus::Failed(err) => {
                                log::debug!("follow_path: {err}");
                                c.failed_at = Some(goal_block);
                                c.target = None;
                            }
                            FollowStatus::Arrived => c.target = None,
                            _ => {}
                        }
                    }
                    Ok(c)
                })
                .await??;
            self.next_tick().await;
            self.check(mark)?;
        }
    }
}
