//! Hands: dig, place, activate blocks, use items.

use ghast_client::hands::BlockHit;
use ghast_client::{BlockPos, Buttons, Face, GameMode, TickContext};

use crate::{Bot, Error};

/// Releases the mouse buttons when a helper ends early (dropped, failed, cancelled).
struct ReleaseButtons(Option<Bot>);

impl ReleaseButtons {
    fn new(bot: &Bot) -> Self {
        ReleaseButtons(Some(bot.clone()))
    }

    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for ReleaseButtons {
    fn drop(&mut self) {
        if let Some(bot) = self.0.take() {
            bot.act(|ctx| ctx.set_buttons(Buttons::default()));
        }
    }
}

/// Options of [`Bot::dig_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DigOptions {
    /// Select the hotbar slot that digs fastest first (default on; only `select_slot`, no
    /// clicks). See [`Bot::equip_for_block`] to bring a tool from the main inventory.
    pub best_tool: bool,
    /// The face to dig from (default: the most visible one).
    pub face: Option<Face>,
    /// Give up after this many ticks (default: twice the vanilla dig time + 2 s).
    pub timeout: Option<u64>,
}

impl Default for DigOptions {
    fn default() -> Self {
        DigOptions {
            best_tool: true,
            face: None,
            timeout: None,
        }
    }
}

impl DigOptions {
    /// Whether to select the fastest hotbar tool first.
    ///
    /// ```
    /// let o = ghast_ext::DigOptions::default().best_tool(false);
    /// ```
    #[must_use]
    pub const fn best_tool(mut self, on: bool) -> Self {
        self.best_tool = on;
        self
    }

    /// The face to dig from.
    ///
    /// ```
    /// let o = ghast_ext::DigOptions::default().face(ghast_client::Face::Up);
    /// ```
    #[must_use]
    pub const fn face(mut self, face: Face) -> Self {
        self.face = Some(face);
        self
    }

    /// Time limit in ticks.
    ///
    /// ```
    /// let o = ghast_ext::DigOptions::default().timeout(400);
    /// ```
    #[must_use]
    pub const fn timeout(mut self, ticks: u64) -> Self {
        self.timeout = Some(ticks);
        self
    }
}

/// The hotbar slot that digs `pos` fastest (the held slot on a tie) and its speed.
pub(crate) fn best_hotbar_tool(ctx: &TickContext<'_>, pos: BlockPos) -> (u8, f32) {
    let held = ctx.held_slot();
    let speed = |k: u8| {
        ctx.dig_speed(pos, ctx.inventory().hotbar(usize::from(k)))
            .unwrap_or(0.0)
    };
    let mut best = (held, speed(held));
    for k in 0..9u8 {
        let s = speed(k);
        if s > best.1 {
            best = (k, s);
        }
    }
    best
}

/// Ticks to break at `speed` progress per tick (vanilla sums f32 progress up to 1).
fn dig_ticks(speed: f32) -> u64 {
    if speed <= 0.0 {
        return u64::MAX;
    }
    let mut p = 0.0f32;
    let mut n = 0u64;
    while p < 1.0 && n < 100_000 {
        p += speed;
        n += 1;
    }
    n
}

impl Bot {
    /// Digs the block at `pos`: selects the fastest hotbar tool, turns until the server
    /// has the crosshair on the block (aim first), then holds the left button for the
    /// vanilla dig time (tool, efficiency, haste / fatigue, water, not on ground) and
    /// releases it once the block is gone. The player should stand still meanwhile.
    ///
    /// Fails with [`Error::NothingToDig`] (air), [`Error::Unbreakable`],
    /// [`Error::OutOfReach`], [`Error::LostTarget`], [`Error::Teleported`] /
    /// [`Error::Died`] or [`Error::Timeout`]. The client predicts the broken block as air;
    /// if the server disagrees it sends the block back (see [`Bot::block_at`]).
    ///
    /// ```no_run
    /// use ghast_client::BlockPos;
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.dig(BlockPos::new(2, 64, 0)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn dig(&self, pos: BlockPos) -> Result<(), Error> {
        self.dig_with(pos, DigOptions::default()).await
    }

    /// [`Bot::dig`] with options.
    ///
    /// ```no_run
    /// use ghast_client::{BlockPos, Face};
    /// use ghast_ext::DigOptions;
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.dig_with(BlockPos::new(2, 64, 0), DigOptions::default().best_tool(false).face(Face::Up)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn dig_with(&self, pos: BlockPos, opts: DigOptions) -> Result<(), Error> {
        let mark = self.mark();
        // Check and pick the tool.
        let (original, ticks) = self
            .with(move |ctx| -> Result<(u32, u64), Error> {
                let raw = ctx
                    .world()
                    .raw_block(pos.x, pos.y, pos.z)
                    .ok_or(Error::NotLoaded(pos))?;
                let q = ctx
                    .world()
                    .block_query(pos.x, pos.y, pos.z)
                    .ok_or(Error::NotLoaded(pos))?;
                if q.air {
                    return Err(Error::NothingToDig(pos));
                }
                if ctx.game_mode() == GameMode::Creative {
                    return Ok((raw, 1));
                }
                if q.hardness < 0.0 {
                    return Err(Error::Unbreakable(pos));
                }
                let speed = if opts.best_tool {
                    let (slot, speed) = best_hotbar_tool(ctx, pos);
                    ctx.select_slot(slot);
                    speed
                } else {
                    ctx.dig_speed(pos, ctx.inventory().held()).unwrap_or(0.0)
                };
                if speed <= 0.0 {
                    return Err(Error::Unbreakable(pos));
                }
                Ok((raw, dig_ticks(speed)))
            })
            .await??;
        let limit = opts
            .timeout
            .unwrap_or_else(|| ticks.saturating_mul(2).saturating_add(40));
        // Aim first: the server judges the dig with the sent look.
        self.look_at_block(pos, opts.face).await?;
        // Hold left until the block changes.
        let release = ReleaseButtons::new(self);
        let start = self.tick();
        let mut off_ticks = 0u32;
        loop {
            let state = self
                .with(move |ctx| {
                    let now = ctx.world().raw_block(pos.x, pos.y, pos.z);
                    if now != Some(original) {
                        ctx.set_buttons(Buttons::default());
                        return Some(true);
                    }
                    let on = ctx.target().is_some_and(|t| t.pos == pos);
                    ctx.set_buttons(Buttons {
                        left: on,
                        right: false,
                    });
                    on.then_some(false)
                })
                .await?;
            match state {
                Some(true) => break,
                Some(false) => off_ticks = 0,
                None => {
                    off_ticks += 1;
                    if off_ticks > 10 {
                        return Err(Error::LostTarget(pos));
                    }
                }
            }
            if self.tick().saturating_sub(start) > limit {
                return Err(Error::Timeout);
            }
            self.next_tick().await;
            self.check(mark)?;
        }
        release.disarm();
        Ok(())
    }

    /// Places the held block against `against`'s `face` (the new block goes to
    /// `face.offset(against)`): turns until the server has the crosshair on that face, sends
    /// the right click at the crosshair's hit point, and waits until the block appears.
    ///
    /// Fails with [`Error::NoSupport`] (air), [`Error::Occupied`] (the target is not air /
    /// liquid, or the player stands in it), [`Error::WouldActivate`] (a chest, door, ... :
    /// the click would use it), [`Error::NothingHeld`], [`Error::OutOfReach`],
    /// [`Error::NotPlaced`] (the server refused) and the usual interrupts.
    ///
    /// ```no_run
    /// use ghast_client::{BlockPos, Face};
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.equip("cobblestone").await?;
    /// bot.place(BlockPos::new(2, 63, 0), Face::Up).await?; // on top of the block at y 63
    /// # Ok(()) }
    /// ```
    pub async fn place(&self, against: BlockPos, face: Face) -> Result<(), Error> {
        let mark = self.mark();
        let target = face.offset(against);
        let before = self
            .with(move |ctx| -> Result<Option<u32>, Error> {
                let w = ctx.world();
                let s = w
                    .block_query(against.x, against.y, against.z)
                    .ok_or(Error::NotLoaded(against))?;
                if s.air {
                    return Err(Error::NoSupport(against));
                }
                if s.activates && !ctx.pose().sneaking {
                    return Err(Error::WouldActivate(against));
                }
                let t = w
                    .block_query(target.x, target.y, target.z)
                    .ok_or(Error::NotLoaded(target))?;
                let raw = w.raw_block(target.x, target.y, target.z);
                let liquid = raw.is_some_and(|r| crate::Block::new(ctx.version(), r).is_liquid());
                if !t.air && !liquid {
                    return Err(Error::Occupied(target));
                }
                if ctx.inventory().held().is_none() {
                    return Err(Error::NothingHeld);
                }
                // The player's box (0.6 x 1.8) must not overlap the new block.
                let p = ctx.pose().pos;
                let (tx, ty, tz) = (
                    f64::from(target.x),
                    f64::from(target.y),
                    f64::from(target.z),
                );
                if p.x + 0.3 > tx
                    && p.x - 0.3 < tx + 1.0
                    && p.y + 1.8 > ty
                    && p.y < ty + 1.0
                    && p.z + 0.3 > tz
                    && p.z - 0.3 < tz + 1.0
                {
                    return Err(Error::Occupied(target));
                }
                Ok(raw)
            })
            .await??;
        let hit = self.look_at_block(against, Some(face)).await?;
        self.right_click(against, face, hit, mark).await?;
        // Wait for the server's block change.
        for _ in 0..20 {
            self.next_tick().await;
            self.check(mark)?;
            let now = self
                .with(move |ctx| ctx.world().raw_block(target.x, target.y, target.z))
                .await?;
            if now != before {
                return Ok(());
            }
        }
        Err(Error::NotPlaced(target))
    }

    /// Right-clicks `pos` (opens a chest, flips a lever, uses a door, ...): turns until the
    /// server has the crosshair on the block, then sends the click. Returns once it was sent;
    /// see [`Bot::open_container`] to wait for a window.
    ///
    /// ```no_run
    /// use ghast_client::BlockPos;
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.activate_block(BlockPos::new(5, 64, 5)).await?; // a lever
    /// # Ok(()) }
    /// ```
    pub async fn activate_block(&self, pos: BlockPos) -> Result<(), Error> {
        let mark = self.mark();
        let hit = self.look_at_block(pos, None).await?;
        self.right_click(pos, hit.face, hit, mark).await
    }

    /// Sends `place` once the sent crosshair is on (`pos`, `face`), retrying a few ticks.
    async fn right_click(
        &self,
        pos: BlockPos,
        face: Face,
        hit: BlockHit,
        mark: crate::executor::Mark,
    ) -> Result<(), Error> {
        for _ in 0..10 {
            let sent = self
                .with(move |ctx| {
                    // The hit point of the crosshair as the server will compute it.
                    let h = ctx
                        .target()
                        .filter(|t| t.pos == pos && t.face == face)
                        .map_or(hit.hit, |t| t.hit);
                    ctx.place(pos, face, h)
                })
                .await?;
            if sent {
                return Ok(());
            }
            self.next_tick().await;
            self.check(mark)?;
        }
        Err(Error::OutOfReach(pos))
    }

    /// Uses the held item until it is done: eats / drinks (food needs a non-full food bar;
    /// golden apples always), with the right button held until the server ends the use.
    /// Fails with [`Error::CannotUse`] when the use does not start. Look away from
    /// containers / doors first (the right click would use them; checked:
    /// [`Error::WouldActivate`]).
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// if bot.health().food < 18 {
    ///     bot.equip("bread").await?;
    ///     bot.consume().await?;
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn consume(&self) -> Result<(), Error> {
        let mark = self.mark();
        self.with(|ctx| -> Result<(), Error> {
            let held = ctx.inventory().held().ok_or(Error::NothingHeld)?;
            if !ctx.item_facts(held).is_some_and(|f| f.usable) {
                return Err(Error::CannotUse);
            }
            if let Some(t) = ctx.target()
                && ctx
                    .world()
                    .block_query(t.pos.x, t.pos.y, t.pos.z)
                    .is_some_and(|b| b.activates)
            {
                return Err(Error::WouldActivate(t.pos));
            }
            ctx.set_buttons(Buttons {
                left: false,
                right: true,
            });
            Ok(())
        })
        .await??;
        let release = ReleaseButtons::new(self);
        let mut started = false;
        for n in 0..100u32 {
            self.next_tick().await;
            self.check(mark)?;
            let using = self.with(|ctx| ctx.using_item()).await?;
            if using {
                started = true;
            } else if started {
                drop(release);
                return Ok(());
            } else if n >= 5 {
                return Err(Error::CannotUse);
            }
        }
        Err(Error::Timeout)
    }

    /// Starts using the held item (right button held: draw a bow, block with a sword /
    /// shield, start eating). Release it with [`Bot::deactivate_item`].
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.equip("bow").await?;
    /// bot.activate_item();
    /// bot.wait_ticks(20).await; // full draw
    /// bot.deactivate_item();      // shoot
    /// # Ok(()) }
    /// ```
    pub fn activate_item(&self) {
        self.act(|ctx| {
            ctx.set_buttons(Buttons {
                left: false,
                right: true,
            });
        });
    }

    /// Releases the mouse buttons (ends an item use, stops digging).
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// bot.deactivate_item();
    /// # }
    /// ```
    pub fn deactivate_item(&self) {
        self.act(|ctx| ctx.set_buttons(Buttons::default()));
    }

    /// Swings the arm.
    ///
    /// ```no_run
    /// # fn f(bot: ghast_ext::Bot) {
    /// bot.swing();
    /// # }
    /// ```
    pub fn swing(&self) {
        self.act(|ctx| ctx.swing());
    }

    /// Attacks entity `id`: turns until the server's crosshair is on it, then attacks
    /// (refused by the client unless it is). Needs [`ghast_client::Interests::ENTITIES`].
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// if let Some(z) = bot.nearest_entity(|e| e.name == "zombie", 4.0).await? {
    ///     bot.attack(z.id).await?;
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn attack(&self, id: i32) -> Result<(), Error> {
        let mark = self.mark();
        for _ in 0..60 {
            let r = self
                .with(move |ctx| -> Result<bool, Error> {
                    let es = ctx.entities().ok_or(Error::NoSuchEntity(id))?;
                    let e = es.get(id).ok_or(Error::NoSuchEntity(id))?;
                    // Aim at the middle of its box.
                    let [x, y, z] = e.position();
                    let h = e.size().map_or(0.9, |(_, h)| f64::from(h) / 2.0);
                    let (yaw, pitch) = crate::yaw_pitch(
                        ctx.eyes(ctx.pose()),
                        ghast_client::Vec3::new(x, y + h, z),
                    );
                    if ctx.target_entity().is_some_and(|t| t.id == id) {
                        return Ok(ctx.attack(id));
                    }
                    ctx.set_aim(yaw, pitch);
                    Ok(false)
                })
                .await??;
            if r {
                return Ok(());
            }
            self.next_tick().await;
            self.check(mark)?;
        }
        Err(Error::Timeout)
    }
}
