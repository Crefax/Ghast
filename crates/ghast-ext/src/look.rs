//! Looking: GCD-correct turning through the client's rotation, and choosing a visible
//! point of a block face.

use ghast_client::hands::{self, BlockHit};
use ghast_client::{BlockPos, Face, TickContext, Vec3};

use crate::{Bot, Error};

/// Ticks a turn may take before [`Error::Timeout`] (a 180° turn takes about 13).
const LOOK_TICKS: u64 = 60;

/// Yaw / pitch (degrees, vanilla convention) that look from `eyes` at `p`.
///
/// ```
/// use ghast_client::Vec3;
/// let (yaw, pitch) = ghast_ext::yaw_pitch(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
/// assert_eq!((yaw, pitch), (0.0, 0.0)); // +Z is yaw 0
/// let (yaw, _) = ghast_ext::yaw_pitch(Vec3::new(0.0, 0.0, 0.0), Vec3::new(-1.0, 0.0, 0.0));
/// assert_eq!(yaw, 90.0); // -X is yaw 90
/// ```
#[must_use]
pub fn yaw_pitch(eyes: Vec3, p: Vec3) -> (f32, f32) {
    let (dx, dy, dz) = (p.x - eyes.x, p.y - eyes.y, p.z - eyes.z);
    let yaw = (-dx).atan2(dz).to_degrees();
    let pitch = -(dy.atan2((dx * dx + dz * dz).sqrt())).to_degrees();
    (yaw as f32, pitch as f32)
}

/// The unit normal of a face.
pub(crate) fn normal(f: Face) -> (f64, f64, f64) {
    match f {
        Face::Down => (0.0, -1.0, 0.0),
        Face::Up => (0.0, 1.0, 0.0),
        Face::North => (0.0, 0.0, -1.0),
        Face::South => (0.0, 0.0, 1.0),
        Face::West => (-1.0, 0.0, 0.0),
        Face::East => (1.0, 0.0, 0.0),
    }
}

pub(crate) const FACES: [Face; 6] = [
    Face::Up,
    Face::North,
    Face::South,
    Face::West,
    Face::East,
    Face::Down,
];

/// The crosshair from `eyes` along (`yaw`, `pitch`), as the client computes it.
pub(crate) fn ray(ctx: &TickContext<'_>, eyes: Vec3, yaw: f32, pitch: f32) -> Option<BlockHit> {
    let w = ctx.world();
    let blocks = |x: i32, y: i32, z: i32| w.block_query(x, y, z);
    hands::pick(&blocks, eyes, hands::look_vector(yaw, pitch), hands::REACH)
}

/// A point to aim at: the point, its face and the look.
pub(crate) type AimPoint = (Vec3, Face, f32, f32);

/// Points on `pos`'s faces (only `want` when given) that the crosshair from the current
/// eyes hits on exactly that block and face, best first: faces turned toward the eyes most,
/// each face's centre first, then points nearer its edges. At most `max` points.
pub(crate) fn aim_points(
    ctx: &TickContext<'_>,
    pos: BlockPos,
    want: Option<Face>,
    max: usize,
) -> Vec<AimPoint> {
    let mut out = Vec::new();
    let eyes = ctx.eyes(ctx.pose());
    let Some(b) = ctx.world().block_query(pos.x, pos.y, pos.z) else {
        return out;
    };
    // The block's ray box (local), or the full cube.
    let s = b.selection.unwrap_or([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
    let (bx, by, bz) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
    let mut faces: Vec<(f64, Face)> = FACES
        .iter()
        .filter(|f| want.is_none_or(|w| w == **f))
        .filter_map(|&f| {
            let n = normal(f);
            // The face plane's coordinate along its normal.
            let c = (
                bx + (s[0] + s[3]) / 2.0,
                by + (s[1] + s[4]) / 2.0,
                bz + (s[2] + s[5]) / 2.0,
            );
            let plane = (
                if n.0 > 0.0 {
                    bx + s[3]
                } else if n.0 < 0.0 {
                    bx + s[0]
                } else {
                    c.0
                },
                if n.1 > 0.0 {
                    by + s[4]
                } else if n.1 < 0.0 {
                    by + s[1]
                } else {
                    c.1
                },
                if n.2 > 0.0 {
                    bz + s[5]
                } else if n.2 < 0.0 {
                    bz + s[2]
                } else {
                    c.2
                },
            );
            let d = (eyes.x - plane.0) * n.0 + (eyes.y - plane.1) * n.1 + (eyes.z - plane.2) * n.2;
            (d > 0.0).then_some((d, f))
        })
        .collect();
    faces.sort_by(|a, b| b.0.total_cmp(&a.0));
    // Points on the face: centre, then a ring toward the edges.
    const OFFSETS: [(f64, f64); 9] = [
        (0.5, 0.5),
        (0.3, 0.5),
        (0.7, 0.5),
        (0.5, 0.3),
        (0.5, 0.7),
        (0.2, 0.2),
        (0.8, 0.2),
        (0.2, 0.8),
        (0.8, 0.8),
    ];
    for (_, f) in faces {
        let n = normal(f);
        for (u, v) in OFFSETS {
            let lerp = |lo: f64, hi: f64, t: f64| lo + (hi - lo) * t;
            let p = match f {
                Face::Up | Face::Down => Vec3::new(
                    bx + lerp(s[0], s[3], u),
                    by + if n.1 > 0.0 { s[4] } else { s[1] },
                    bz + lerp(s[2], s[5], v),
                ),
                Face::North | Face::South => Vec3::new(
                    bx + lerp(s[0], s[3], u),
                    by + lerp(s[1], s[4], v),
                    bz + if n.2 > 0.0 { s[5] } else { s[2] },
                ),
                Face::West | Face::East => Vec3::new(
                    bx + if n.0 > 0.0 { s[3] } else { s[0] },
                    by + lerp(s[1], s[4], v),
                    bz + lerp(s[2], s[5], u),
                ),
            };
            let (yaw, pitch) = yaw_pitch(eyes, p);
            if ray(ctx, eyes, yaw, pitch).is_some_and(|h| h.pos == pos && h.face == f) {
                out.push((p, f, yaw, pitch));
                if out.len() >= max {
                    return out;
                }
            }
        }
    }
    out
}

/// One tick of a turn toward (`yaw`, `pitch`): sets the aim and says whether the look has
/// arrived (the rotation will not move any more and the server has this look).
pub(crate) fn step_toward(ctx: &mut TickContext<'_>, yaw: f32, pitch: f32) -> bool {
    ctx.set_aim(yaw, pitch);
    let p = *ctx.pose();
    let s = *ctx.sent();
    ctx.planned_look() == (p.yaw, p.pitch) && s.yaw == p.yaw && s.pitch == p.pitch
}

impl Bot {
    /// Turns to look at `point` in whole mouse counts (sensitivity GCD, at most 14° yaw /
    /// 10.5° pitch per tick, like a player's mouse) and returns once the server has the final
    /// look. The eyes are re-read every tick, so it also works while moving.
    ///
    /// ```no_run
    /// use ghast_client::Vec3;
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.look_at(Vec3::new(10.5, 65.0, -3.5)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn look_at(&self, point: Vec3) -> Result<(), Error> {
        let mark = self.mark();
        for _ in 0..LOOK_TICKS {
            let done = self
                .with(move |ctx| {
                    let (yaw, pitch) = yaw_pitch(ctx.eyes(ctx.pose()), point);
                    step_toward(ctx, yaw, pitch)
                })
                .await?;
            if done {
                return Ok(());
            }
            self.next_tick().await;
            self.check(mark)?;
        }
        Err(Error::Timeout)
    }

    /// Turns to an absolute yaw / pitch (degrees; pitch clamped to ±89.5 like the client)
    /// and returns once the server has the final look.
    ///
    /// ```no_run
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// bot.look(90.0, 0.0).await?; // face -X
    /// # Ok(()) }
    /// ```
    pub async fn look(&self, yaw: f32, pitch: f32) -> Result<(), Error> {
        let mark = self.mark();
        for _ in 0..LOOK_TICKS {
            if self.with(move |ctx| step_toward(ctx, yaw, pitch)).await? {
                return Ok(());
            }
            self.next_tick().await;
            self.check(mark)?;
        }
        Err(Error::Timeout)
    }

    /// Turns until the crosshair (as the server judges it, from the sent pose) is on `pos`
    /// (on `face`, if given) and returns the hit. Fails with [`Error::OutOfReach`] when no
    /// visible point of the block is within reach (4.5).
    ///
    /// ```no_run
    /// use ghast_client::BlockPos;
    /// # async fn f(bot: ghast_ext::Bot) -> Result<(), ghast_ext::Error> {
    /// let hit = bot.look_at_block(BlockPos::new(2, 63, 0), None).await?;
    /// println!("on face {:?} at {:?}", hit.face, hit.hit);
    /// # Ok(()) }
    /// ```
    pub async fn look_at_block(
        &self,
        pos: BlockPos,
        face: Option<Face>,
    ) -> Result<BlockHit, Error> {
        let mark = self.mark();
        let points = self
            .with(move |ctx| {
                if ctx.world().raw_block(pos.x, pos.y, pos.z).is_none() {
                    return Err(Error::NotLoaded(pos));
                }
                Ok(aim_points(ctx, pos, face, 6))
            })
            .await??;
        // Try the points in turn: the look lands on a multiple of the mouse step, which can
        // miss a thin face's centre.
        for (_, _, yaw, pitch) in points {
            for _ in 0..LOOK_TICKS {
                let r = self
                    .with(move |ctx| {
                        let arrived = step_toward(ctx, yaw, pitch);
                        let on = ctx
                            .target()
                            .filter(|t| t.pos == pos && face.is_none_or(|f| f == t.face));
                        (arrived, on)
                    })
                    .await?;
                match r {
                    (true, Some(hit)) => return Ok(hit),
                    (true, None) => break,
                    (false, _) => {}
                }
                self.next_tick().await;
                self.check(mark)?;
            }
        }
        Err(Error::OutOfReach(pos))
    }
}
