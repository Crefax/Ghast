//! Walking: a straight-line walker (open ground, no planning) and following an entity.

use ghast_client::{Controls, TickContext, Vec3};

use crate::{Bot, Error};

/// Releases the movement keys when a walker ends early.
struct ReleaseKeys(Option<Bot>);

impl Drop for ReleaseKeys {
    fn drop(&mut self) {
        if let Some(bot) = self.0.take() {
            bot.act(|ctx| ctx.set_controls(Controls::NONE));
        }
    }
}

/// Ticks without progress before [`Error::Stuck`].
const STUCK_TICKS: u32 = 60;

/// State of the straight walker between ticks.
#[derive(Debug, Clone, Copy, Default)]
struct Walk {
    last: Option<Vec3>,
    still: u32,
}

/// One walker tick toward `goal`: aims the yaw (keeps the pitch), presses forward, sprints
/// when far, jumps when pressing forward made no progress on the ground. `true` once
/// within `range` (horizontal) of the goal (keys released).
fn walk_step(
    ctx: &mut TickContext<'_>,
    w: &mut Walk,
    goal: Vec3,
    range: f64,
    sprint: bool,
) -> bool {
    let p = *ctx.pose();
    let (dx, dz) = (goal.x - p.pos.x, goal.z - p.pos.z);
    let d = (dx * dx + dz * dz).sqrt();
    if d <= range {
        ctx.set_controls(Controls::NONE);
        return true;
    }
    let yaw = (-dx).atan2(dz).to_degrees() as f32;
    ctx.set_aim(yaw, p.pitch);
    let moved = w.last.map_or(1.0, |l| {
        ((p.pos.x - l.x).powi(2) + (p.pos.z - l.z).powi(2)).sqrt()
    });
    w.still = if moved < 0.01 { w.still + 1 } else { 0 };
    w.last = Some(p.pos);
    let jump = p.on_ground && w.still >= 2;
    ctx.set_controls(Controls {
        forward: true,
        sprint: sprint && d > 3.0,
        jump,
        ..Controls::NONE
    });
    false
}

impl Bot {
    /// Walks in a straight line until within `range` blocks (horizontally) of `goal`:
    /// turns toward it (whole mouse counts), holds forward (sprinting while more than 3
    /// blocks away), and jumps when blocked. No path planning: for open ground. Fails with
    /// [`Error::Stuck`] after 3 s without progress, or when teleported / dead.
    ///
    /// ```no_run
    /// use ghast_client::Vec3;
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.walk_straight_to(Vec3::new(10.5, 64.0, 3.5), 0.5).await?;
    /// # Ok(()) }
    /// ```
    pub async fn walk_straight_to(&self, goal: Vec3, range: f64) -> Result<(), Error> {
        let mark = self.mark();
        let mut keys = ReleaseKeys(Some(self.clone()));
        let mut w = Walk::default();
        loop {
            let (done, ww) = self
                .with(move |ctx| {
                    let mut w = w;
                    let done = walk_step(ctx, &mut w, goal, range, true);
                    (done, w)
                })
                .await?;
            w = ww;
            if done {
                keys.0 = None;
                return Ok(());
            }
            if w.still > STUCK_TICKS {
                return Err(Error::Stuck);
            }
            self.next_tick().await;
            self.check(mark)?;
        }
    }

    /// Follows entity `id`, staying within `range` blocks of it, with the straight walker
    /// (see [`Bot::walk_straight_to`]): for open ground. To follow through mazes, over gaps
    /// and around walls use `Bot::follow_path` (feature `pathfinder`). Runs until the entity is gone
    /// ([`Error::NoSuchEntity`]), the walker is stuck, or the task is cancelled (e.g. with
    /// [`Bot::timeout`] or [`crate::JoinHandle::abort`]). Needs
    /// [`ghast_client::Interests::ENTITIES`].
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// if let Some(p) = bot.player("Steve").await? {
    ///     let _ = bot.timeout(20 * 60, bot.follow(p.id, 2.0)).await; // one minute
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn follow(&self, id: i32, range: f64) -> Result<(), Error> {
        let mark = self.mark();
        let _keys = ReleaseKeys(Some(self.clone()));
        let mut w = Walk::default();
        loop {
            let r = self
                .with(move |ctx| -> Result<Walk, Error> {
                    let e = crate::find::entity_info(ctx, id).ok_or(Error::NoSuchEntity(id))?;
                    let mut w = w;
                    if walk_step(ctx, &mut w, e.pos, range, true) {
                        // Arrived: keep watching (look at it).
                        let (yaw, pitch) = crate::yaw_pitch(
                            ctx.eyes(ctx.pose()),
                            Vec3::new(e.pos.x, e.pos.y + 1.6, e.pos.z),
                        );
                        ctx.set_aim(yaw, pitch);
                        w = Walk::default();
                    }
                    Ok(w)
                })
                .await??;
            w = r;
            if w.still > STUCK_TICKS {
                return Err(Error::Stuck);
            }
            self.next_tick().await;
            self.check(mark)?;
        }
    }

    /// Sets the movement keys (kept until changed) in this tick.
    ///
    /// ```no_run
    /// use ghast_client::Controls;
    /// # async fn f(bot: ghast_ext::Bot) {
    /// bot.set_controls(Controls { forward: true, jump: true, ..Controls::NONE });
    /// bot.wait_ticks(10).await;
    /// bot.set_controls(Controls::NONE);
    /// # }
    /// ```
    pub fn set_controls(&self, controls: Controls) {
        self.act(move |ctx| ctx.set_controls(controls));
    }
}
