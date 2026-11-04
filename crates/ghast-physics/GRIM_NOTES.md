# Grim vs vanilla: notes for a vanilla-exact client

Grim (GrimAnticheat/Grim, GPL-3) is the anticheat on our lab servers. ghast-physics reproduces
vanilla bit for bit (oracle-verified), so a Grim flag on a ghast bot is either a client-feed bug
or a place where Grim's model differs from vanilla. These notes describe Grim's behaviour **in
our own words**; no Grim code is copied. Read from the public source at tag `v2.3.73`
(commit `879d566c05bc6b03918c5f24bb5da1a209352b99`), the version on our labs. Paths below are
relative to `common/src/main/java/ac/grim/grimac/` in that tree.

Status: source reading only. Every "suspect" below still needs a lab experiment (plan at the end)
before we change any behaviour.

## How Grim judges a movement (relevant parts)

- It re-simulates each tick from its own copy of the player state and a set of candidate
  velocities (knockback / no knockback, jump / no jump, …), picks the candidate whose collided
  result is closest to the movement the client claims, and flags the offset (`Simulation`) when
  even the best candidate is too far away (`predictionengine/MovementCheckRunner.java`,
  `predictionengine/predictions/PredictionEngine.java`).
- Collision with step-up is its own re-implementation (`utils/nmsutil/Collisions.java`, the
  `collide` method). For 1.8–1.20 clients it follows the legacy two-path step-up (raise over the
  swept box / raise from the start box, keep the one that goes further, then come down by the
  step amount). It tries several axis orders and keeps the one closest to the claim.
- The step-up condition matches vanilla's "on ground before the move, or a downward move that was
  clipped" (Grim also accepts "client velocity Y < 0" and a 0.03-proximity heuristic as
  downward). So **Grim's collide function would step up in our landing case** if it ran it on
  the right state. The difference must come from the state it runs on, or from extra rules
  around it.
- Ground status: after the prediction, `GroundSpoof` compares the client's claimed on-ground flag
  with Grim's computed one (`checks/impl/prediction/GroundSpoof.java`). Grim computes it as
  "vertical collision and requested dy < 0", with exemptions (`player/GrimPlayer.java`,
  `exemptOnGround`) and special handling of 0.03 ticks.
- Packets without a position (the client moved less than 0.03 blocks, so vanilla omits it):
  `events/packets/CheckManagerListener.java` (`handleFlying`). For 1.8 clients Grim assumes no
  ticks are ever skipped (`player/GrimPlayer.java`, `canSkipTicks()` is false below 1.9). It
  re-runs the tick against the *unchanged* claimed position and absorbs the hidden movement
  with a "0.03" uncertainty (`predictionengine/PointThreeEstimator.java`,
  `UncertaintyHandler.lastPointThree`). If such a packet flips on-ground, Grim accepts the new
  ground state only when a 0.03-wide probe could reach ground and vertical velocity is ≤ 0.06.
  Otherwise it forces a resync (setback).

## Case 1: 1.8.9 landing step-up with a look-only packet (`Simulation`)

What vanilla does (oracle-verified, `tests/fixtures/lab_soulsand_restart_1_8_9.jsonl.gz` and
the `landing_step_*` scenarios): falling onto soul sand (top y + 0.875) into a grass edge
(y + 1.0), the landing tick steps up and ends on the grass. Net motion that tick is about 0.02
up and 0.02 sideways, under the 0.03 resend threshold, so the client sends only a look (or
flying) packet with on-ground true. Its state really is on top of the grass.

What Grim most likely does (from the code; to be confirmed):
1. On the look-only tick, Grim keeps the last claimed position (in the air above the soul sand)
   and treats the tick as "moved < 0.03". The step-up itself (0.125 high, more than 0.03) is not
   among the movements the 0.03 handling covers, so Grim's state stays "below the ledge".
2. On the next position packet the client is already on the grass. Grim predicts from its own
   state, where the player is still at the soul sand, and the gap is the step plus the walk.
   The reported offsets (~0.1587) fit "Grim chose no step-up".
3. **Chain of setbacks:** right after Grim's own setback teleport, Grim *forbids stepping* for
   one tick on purpose (`Collisions.java`, `disallowStepping` when the required setback has
   completed exactly one tick; an anti-exploit for high-ping step abuse). A vanilla client
   standing at the same ledge steps up again on that tick, so every tick after the setback
   flags again with the same offset. That matches the client's report of "a setback every
   tick".

`Client::force_step_position` (client crate, opt-in) sends a position packet on ticks whose feet
height changed but stayed under the threshold. Grim then sees the stepped position and the case
disappears (0 setbacks in 16 min on the lab). **Narrower option:** force the position only when
the tick's move was a step-up, i.e. the feet went *up* and the player is on ground after the
move (`on_ground && pos.y > sent.y`), and leave falling / descending sub-0.03 ticks vanilla. That
is the exact class of tick Grim cannot reconstruct. Still to check: whether Grim also needs the
position on the tick right after a setback, given the forbidden step on that tick.

## Case 2: 1.16.5 walking on slime, on-ground alternating (`GroundSpoof`)

Vanilla 1.16.5 (oracle-verified, 1,120-tick lab recording `client_slime/seed1`): walking on
slime, every landing bounces (vy reflected), so the on-ground flag alternates nearly every tick.
In that recording 140 of 141 flips come with a position packet, so the "0.03 ground flip" rule
above is not the cause.

Grim pieces involved (to be confirmed by experiment):
- Grim applies the slime bounce only when it identifies the block *under* the player as slime
  through its supporting-block lookup (`MovementTicker.java` around the collision code;
  `utils/nmsutil/BlockProperties.java`, `getOnPos` with the 1.20-style main supporting block).
  It also uses its server-side sneak state for "bypass bounce".
- Grim adds slime "uncertainty" (`UncertaintyHandler.isSteppingOnSlime`,
  `this/nextTickSlimeBlockUncertainty`) that widens the accepted Y but **does not** relax
  `GroundSpoof`, which compares booleans exactly.
- Suspects, in order:
  (a) Grim's best candidate on the up-tick has dy < 0 with a vertical collision, so it says
      "on ground" while vanilla is airborne after the bounce;
  (b) its supporting-block lookup picks a neighbouring non-slime block at block edges, so it
      computes "landed without bounce";
  (c) the sneak / bypass state differs by a tick.

**Possible compat tweak (proposal, needs evidence first):** it cannot fix a boolean disagreement
by sending more positions. Two options: (1) behaviour-level: avoid walking on slime, as now;
(2) report the ground flag Grim expects on the specific up-ticks, which is a deliberate
non-vanilla packet and only worth it if (a) is confirmed and the rule is simple. No tweak until
the lab log below says which tick and which Grim candidate disagree.

## Other places Grim deviates from vanilla that a vanilla-exact bot could hit

| # | Where (Grim) | Deviation | Conditions a bot can hit |
|---|---|---|---|
| 1 | `Collisions.collide`, `disallowStepping` | no step-up on the first tick after a Grim setback | any setback next to a step / ledge (case 1 chain) |
| 2 | `CheckManagerListener.handleFlying` | a non-position packet that flips on-ground is accepted only if a 0.06-high probe can reach ground within 0.03 and vy ≤ 0.06; otherwise force resync | tiny bounces (slime, bed), leaving / landing on very thin ledges with < 0.03 movement |
| 3 | `PointThreeEstimator` / `UncertaintyHandler` | hidden < 0.03 movement modelled as bounded uncertainty, not simulated | step-ups (case 1), sub-0.03 landings on partial blocks |
| 4 | `Collisions.maybeBackOffFromEdge` | 1.8–1.10 sneak edge probe uses a drop of 1 − 1e-7 instead of exactly 1.0 | sneaking at a ledge whose floor is exactly one block lower (boundary only) |
| 5 | `GrimPlayer.wouldCollisionResultFlagGroundSpoof` / `MovementTicker` | if the candidate's dy is exactly −1e-7 and the result is between −1e-7 and 0, Grim keeps its previous ground flag | standing still on ground with sub-epsilon motion (rare) |
| 6 | `MovementTicker` | speed factor / bounce block taken from the 1.20-style supporting block | edges between slime / soul sand / honey and other blocks (case 2 suspect b) |
| 7 | `Collisions.collide` | several axis orders tried, the closest to the claim wins | none: this only widens what Grim accepts |

Rows 1–3 are confirmed by reading and explain the observed flags. Rows 4–6 are candidates:
they matter only if a lab run shows them.

## Evidence plan

1. **Lab logs with ground truth:** run the movement course with Grim's verbose / debug output on
   (per-tick predicted vs actual offset, chosen candidate, on-ground) and ghast's recording on
   (`Client::record`). Align by tick; for every flag, have the oracle replay the recording from
   ghast's state with its box (bit-exact restarts). If ghast == vanilla, the flag is Grim's and
   the debug line names the candidate.
2. **Case 1 narrow tweak:** implement the "step-up only" variant behind the same opt-in and rerun
   the soul-sand course 16 min. Pass = 0 setbacks.
3. **Case 2:** the slime course with Grim debug. Classify each `GroundSpoof` tick as suspect a /
   b / c above; for (b), repeat with slime surrounded by slime.
4. **Rows 4–6:** a short scripted lab course per row (sneak at a 1-block drop, sub-0.03 stop on a
   carpet, slime / honey edges). The oracle replays each recording to prove the bot was vanilla.
