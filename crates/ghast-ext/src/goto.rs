//! Walking along planned paths (feature `pathfinder`).

use ghast_client::{Buttons, Controls};
use ghast_pathfinder::{
    Config, FollowConfig, FollowError, FollowStatus, Follower, Goal, Pathfinder,
};

use crate::{Bot, Error};

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
    /// Planning runs inside the tick: a long detour through rough terrain can take tens of
    /// milliseconds (see the ghast-pathfinder README); keep goals within a few hundred blocks.
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
}
