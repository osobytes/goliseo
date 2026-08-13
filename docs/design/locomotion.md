# Design: Locomotion — momentum, turn arcs, and facing decoupled from movement

Supersedes the mechanism in `docs/design/momentum.md`, which stays as the
record of the first pass. That pass is where accel/decel separation and
`run_vel` come from; this one generalizes it.

Issue #488. This document covers **PR 1 of a stack**: the sim primitive.

## Why

A body that reverses on a frame has no positioning to exploit, fake or
punish. Any defender could mirror any attacker; any attacker could erase a
defender's position with a free 180° flick. Every downstream system that
wants a body to have weight — committed tackles, keeper repositioning,
leading a runner, a defensive contain stance — inherits that.

## Player-facing behavior

- **A reversal is a stop.** Turning back on yourself sheds your speed first
  and only then commits the other way. It is visibly a decision, and the
  faster you were going the more it costs.
- **A turn at pace is an arc, and you keep your pace through it.** The old
  helper nudged a velocity vector toward a desired one, so every hard turn
  shed speed as a side effect of cutting the chord. Now the heading rotates
  at a bounded rate while the speed scalar holds.
- **A turn at a standstill is free**, and turns get cheaper as you slow down.
- **Where you look is not where you move.** Facing rotates toward its own
  target at its own rate, gliding into the last few degrees rather than
  snapping. Jockeying sideways around a carrier is now a different profile
  from running sideways, without either being a special-cased verb.
- **Carrying costs you.** A carrier's top speed, acceleration and turn rate
  all differ from the same player empty-handed, and sprinting with the ball
  differs again.
- **Players differ in how they move, from their stats alone.** A fast,
  low-technique player is quick in a straight line and wide in the corners.
  Nobody authored that; it falls out of pace, strength and technique feeding
  acceleration, braking and turn rate respectively.

## Carry is a modifier, not a context — a departure from #488's model

**#488 enumerates seven mutually exclusive contexts:**

> `jog | run | sprint | carry | sprint_carry | strafe | backpedal`

There is no `carry_backpedal` in that list, and there cannot be one without
enumerating the cross product. Implemented literally, the model has a hole
with teeth: `resolve` has to answer *"carrying, or backing away?"* with a
single value, and whichever it answers, the other's kinematics are discarded.

The first implementation here checked the geometry first, so **a carrier who
backed off or shielded got the empty-handed profile.** Carry's reduced top
speed, acceleration and turn rate all vanished at precisely the moment a
carrier is shielding — the mechanic #488 says should make body position worth
something. The two profiles were bit-identical.

What ships treats them as what they are: two independent facts about a body —
*which way it is moving relative to where it looks*, and *whether it has the
ball* — composed multiplicatively.

```
top_speed = dir_ctx.top_mult * carry.top_mult * base_speed
accel     = dir_ctx.accel_mult * carry.accel_mult * base_accel * pace_curve(pace)
...
```

- **Direction contexts** (5): `jog`, `run`, `sprint`, `strafe`, `backpedal`.
- **Carry modes** (3): `empty`, `carry`, `sprint_carry`; `empty` composes as
  1.0 throughout.
- `backpedal x carry` and `strafe x sprint_carry` are now expressible. They
  were not before.

**No knob was added or removed.** `Run`'s multipliers are all `1.0`, so
`Carry`'s defaults are unchanged and `run x carry` reproduces exactly what the
peer-context model produced. `SprintCarry`'s four were re-derived by dividing
the old absolute values by `Sprint`'s, so `sprint x sprint_carry` reproduces
its old product too — their *meaning* changed from absolute to relative, and
their descriptions say so. Only the previously unreachable cases behave
differently, which is the entire point.

> Worth recording: the authored `SprintCarry` values were already within 1–5%
> of `Sprint x Carry` (`1.35 x 0.92 = 1.24` against an authored `1.18`, and so
> on down all four). The composition was implicit in the numbers before it was
> in the code, which is some evidence the peer-context enumeration was always
> the wrong shape rather than merely an incomplete one.

`shielding_costs_more_than_backing_off_empty_handed` is the mechanic in one
tier-1 assertion, and it checks top speed, acceleration *and* turn rate —
shielding should be a handling cost, not only a speed cost.

## The five ordered steps

`gc_sim::locomotion` owns all of it, purely: no RNG, no I/O, rates as
per-second quantities integrated over the fixed tick, so a rollback
resimulation reproduces every arc exactly.

1. **Resolve the direction context and the carry mode**, separately. The
   direction — `jog`, `run`, `sprint`, `strafe`, `backpedal` — comes from the
   commanded throttle, the sprint flag and the angle between commanded
   movement and the facing target. The carry mode comes from possession.
   Geometry decides the direction; it no longer decides the ball away.
2. **Derive the context's kinematics** from context multipliers × stat curves
   (below).
3. **Ease the speed scalar** toward the context target. Accel and decel are
   separate rates; a reversal sets the target to zero.
4. **Ease the movement heading** toward the command at the context's bounded
   angular rate, with a low-speed bonus.
5. **Ease facing** toward its own target: bounded rate above the ease-out
   threshold, a damped ease-out below it.

## Parametric derivation

```
top_speed = ctx.top_mult   * base_speed
accel     = ctx.accel_mult * base_accel * pace_curve(pace)
decel     = ctx.decel_mult * base_decel * strength_curve(strength)
turn_rate = ctx.turn_mult  * base_turn  * technique_curve(technique)
```

Context multipliers and curve control points are `gc-data`; the derivation is
a pure `gc-sim` function. There are no per-character tables.

**One deviation from the issue's formula.** It writes `top_speed = ...  *
base_speed * pace_curve(pace)`. Our `base_speed` is `MatchPlayer::move_speed`,
which `stats::move_speed` already computes as `BASE_MOVE + pace *
MOVE_PER_PACE` — a pace curve applied to a base speed. A second one would
count pace into top speed twice and restretch every authored player.

## Two properties that hold structurally, not by tuning

- **A reversal never flips sign.** Braking that would take the body below the
  snap speed lands it at exactly zero and pivots there. The velocity crosses
  the origin at every tick rate and every knob value, not merely at the
  shipped defaults.
- **Facing never overshoots, so it cannot oscillate.** The rotation step is
  clamped to the remaining angle, so the angle is monotonically non-increasing
  and cannot change sign. A floor under the ease-out rate stops the approach
  being asymptotic, so it also lands in finite time.

`gc-sim/tests/locomotion.rs` asserts both at the declared `min` and `max` of
every knob that touches them, because a rule that only holds at its defaults
is not a rule.

## No state-shape migration, and why the budgeted one was not needed

The issue budgeted for a breaking snapshot change: a speed scalar, an
independent facing and a context id added to `MatchPlayer`, with a state
version bump and a fixture migration. None of the three turned out to be
needed, and adding them would have introduced redundant state two peers can
disagree about:

- The speed scalar and heading **are** `run_vel.length()` and
  `run_vel.normalized()`. The only thing a separate heading adds is a
  direction at zero speed — and at zero speed the heading is free anyway.
- Facing is already a field.
- The context is a pure function of state and inputs. Persisting it would be
  two representations of one fact, and a rollback restore that disagreed with
  a fresh resolve would desync silently.

So `match_snapshot::VERSION`, the snapshot layout and the wire player layout
are untouched. `match_snapshot_case_a/b_lua_reference.txt` — the encoding
vectors that pin the shape — stay green, which is the check that the claim is
true rather than merely asserted.

Behavioral fixtures are a different matter: the *values* in every replay move,
because the simulation moves. See the PR for the full list.

## Determinism: the turn math may not touch libm

`sin`, `cos` and `atan2` are the three functions ARCHITECTURE.md §1 names as
implementation-approximated, and its wasm paragraph is arguing browser against
browser. Rust links a **different** libm for `wasm32-unknown-unknown` than a
native build uses, so the two disagree in the low bits. On a per-tick,
per-player path that is a native peer and a browser peer desyncing in the same
match — measured, not theorized: the first draft of this module reproduced its
own re-recorded OMP-1 boundary hashes natively and diverged inside the compiled
wasm module at boundary 12 of 7,202.

The second draft removed the transcendentals by bounding the rotation with
**chord** distance in a lerp-then-renormalize. That is deterministic, and it is
also **not a bounded angular rate** — the defect design review caught. Chord
`= 2 sin(theta/2)` tracks the angle only while the *remaining* angle is near
the step, not merely while the step is small, so the achieved rate collapses as
the remaining angle grows: at a nominal 18°/tick, 89% of nominal at 90°
remaining, 53% at 135°, **1.3% at 179°**. Facing has no reversal short-circuit,
so `FacingIntent::Toward` — the jockey/contain case this feature exists for —
asks for exactly those angles whenever play swings around a defender, whose
facing would have nearly frozen and then rushed to catch up.

What ships is an exact rotation on top of `gc_core::deterministic_math::cos_sin`,
which halves the angle to under 0.0625 rad (exact — a power-of-two scale),
evaluates truncated Maclaurin series in Horner form, applies the double-angle
identities back up, and renormalizes. Only `+ - * /` and `sqrt`, every one
correctly rounded by IEEE 754, in a fixed evaluation order: bit-identical across
targets by construction rather than by luck. It sits beside
`negative_log_one_minus`, which exists in that module for precisely the same
reason.

No inverse trig is needed anywhere, which is the part worth remembering:

- **Comparing two angles is comparing two cosines.** `cos` is monotone on
  `0..=pi`, so "the remaining angle is inside one step" is exactly
  `dot(from, to) >= cos(step)`. Clamping the step to `pi` keeps that monotone.
- **Which way to turn is the sign of the cross product**, and at the antipode,
  where it is zero and both arcs are equal, `>= 0` breaks the tie the same way
  on every peer.

`gc-sim`'s remaining ten `sin`/`cos` sites are **not** fixed by this and are a
live desync risk; removing this module's three moved the first wasm divergence
from boundary 12 to boundary 7006. That is #517, and `cos_sin` is the primitive
its direction 3 asks for.

> **The lesson for the next reader.** Every test in `tests/locomotion.rs` passed
> throughout the chord defect and none of them could have failed: an upper bound
> on the step, monotone convergence, and no oscillation are all satisfied by a
> rate that is far too *low*. `facing_turns_at_its_nominal_rate_from_any_remaining_angle`
> pins the achieved rate from below as well as above, across the whole domain.
> When a quantity has a declared unit, assert the unit.

## Tunables

`LOCO_*` in the `Locomotion` panel category, flat SCREAMING_SNAKE, plus the
already-shipped `MOVE_ACCEL` / `START_ACCEL` / `MOVE_DECEL` as the shared
bases and `SPRINT_MULT` as the sprint context's top-speed multiplier.

Turn rates are in `deg/s` and that unit is now literally true: the body turns
by exactly `rate * dt` per tick until the last partial step, which lands on the
target exactly. The three arc thresholds are authored as **cosines** rather than
degrees, because each is compared against a dot product every tick and a `cos()`
there would put a libm call back on the determinism path. `LOCO_TURN_EASE_DEG`
stays in degrees: the ease region is entered exactly (chord is monotone in the
angle, so the threshold comparison is equivalent), and the ramp *inside* it is
chord-proportional — a feel choice between two monotone 0→1 shapes, not an
approximation standing in for one.

> **Do not use dotted ids.** `Tuning::deserialize`'s parser accepts only
> `[A-Za-z0-9_]` in a key and skips malformed lines *without erroring*, and
> `knob_contract` perturbs a knob through exactly that parser. A knob named
> `loco.run.accel` would look correctly registered in the F1 panel and the
> sweep while every knob-moves-metric assertion reported it as decoration.
> `gc-sim/tests/locomotion.rs` round-trips every `LOCO_*` id through the blob
> so this cannot regress silently.

## Measuring it: `time_to_reverse`

The eight metrics that existed before this rework are all match **outcomes** —
goals, shots, possession, droughts. A kinematics change reaches an outcome
only through many layers of AI decision-making, and the consequence was
measured, not guessed: **44 of the 45 `LOCO_*` knobs reported `DECORATION`
against every one of them** at seed counts a per-PR gate can afford. Under
AGENTS.md §9 that is not a tuning problem to be swept away, it is the feature
failing the knob-contract rule. So the fix is a metric that measures what the
primitive actually claims.

`time_to_reverse` is the mean seconds a body takes to complete a 180° reversal:
armed when it is cruising at ≥80% of base speed, completed when it is back
above 50% heading within 15° of the opposite. Two details carry weight:

- **It requires the body to pass near a standstill.** That is the observable
  signature of the reversing branch — a commanded 180° always trips it,
  because the arc thresholds put anything past 120° there. A body that merely
  turned a long way round keeps its pace and never comes near zero. The first
  draft had no such gate and counted arcs too; braking is most of a real
  reversal and almost none of an arc, so folding the populations together
  diluted exactly the term the metric exists to measure.
- **It is detected from the trajectory, not from a stored command.**
  Persisting the commanded direction would add a `MatchPlayer` field, hence a
  snapshot version bump, hence state two peers can disagree about — the same
  argument that kept the context out of the snapshot. A reversal is fully
  observable without it.

It resolves an order of magnitude better than anything else registered,
because it is a mean over hundreds of events per match rather than a count of
roughly one and a half goals. Measured over 60 full-length matches:

| metric | mean | sd | se/mean |
| --- | --- | --- | --- |
| `time_to_reverse` | 0.482 | 0.026 | **0.7%** |
| `possession_balance` | 0.392 | 0.063 | 2.1% |
| `turnovers_per_min` | 3.82 | 1.19 | 4.0% |
| `goals_total` | 2.28 | 1.42 | 8.0% |
| `shots_per_goal` | 24.6 | 14.7 | 8.1% |

At the shipped defaults it measures **0.48 s**, inside #488's proposed
0.25–0.6 s band and nearer its slow edge. The issue calls that band a prior;
the measurement backs it.

### The §9 census — all 45 knobs, published in full

Every `LOCO_*` knob perturbed across its **whole declared range** against
`time_to_reverse`, 24 full-length matches, after the carry-composition fix.
**12 move it; 33 are unresolved.**

| knob | default → perturbed | delta (s) | threshold | verdict |
| --- | --- | --- | --- | --- |
| `LOCO_BACKPEDAL_ACCEL_MULT` | 0.7 → 2 | -0.0986 | 0.0179 | **WIRED** |
| `LOCO_PACE_CURVE_HI` | 1.2 → 2 | -0.0718 | 0.0168 | **WIRED** |
| `LOCO_PACE_REF_LO` | 100 → 200 | +0.0554 | 0.0196 | **WIRED** |
| `LOCO_STRAFE_ACCEL_MULT` | 0.85 → 2 | -0.0380 | 0.0181 | **WIRED** |
| `LOCO_PACE_REF_HI` | 240 → 320 | +0.0342 | 0.0166 | **WIRED** |
| `LOCO_DIR_SNAP_SPEED` | 6 → 60 | -0.0333 | 0.0190 | **WIRED** |
| `LOCO_RUN_ACCEL_MULT` | 1 → 2 | -0.0307 | 0.0177 | **WIRED** |
| `LOCO_STRAFE_ARC_COS` | 0.57 → 0.94 | +0.0289 | 0.0199 | **WIRED** |
| `LOCO_STRENGTH_CURVE_HI` | 1.2 → 2 | -0.0211 | 0.0142 | **WIRED** |
| `LOCO_BACKPEDAL_TOP_MULT` | 0.6 → 1.4 | +0.0210 | 0.0166 | **WIRED** |
| `LOCO_BACKPEDAL_ARC_COS` | -0.5 → 0 | +0.0206 | 0.0171 | **WIRED** |
| `LOCO_BACKPEDAL_DECEL_MULT` | 1.25 → 2.5 | -0.0174 | 0.0160 | **WIRED** |
| `LOCO_JOG_DECEL_MULT` | 1 → 2.5 | +0.0178 | 0.0210 | unresolved |
| `LOCO_PACE_CURVE_LO` | 0.8 → 1 | -0.0136 | 0.0179 | unresolved |
| `LOCO_CARRY_TOP_MULT` | 0.92 → 1.4 | +0.0133 | 0.0176 | unresolved |
| `LOCO_RUN_TOP_MULT` | 1 → 1.4 | -0.0123 | 0.0173 | unresolved |
| `LOCO_SPRINT_DECEL_MULT` | 0.8 → 2.5 | +0.0116 | 0.0188 | unresolved |
| `LOCO_BACKPEDAL_TURN_MULT` | 1.1 → 2.5 | +0.0112 | 0.0180 | unresolved |
| `LOCO_STRENGTH_CURVE_LO` | 0.8 → 1 | +0.0112 | 0.0177 | unresolved |
| `LOCO_SPRINT_CARRY_ACCEL_MULT` | 0.882 → 2 | +0.0101 | 0.0175 | unresolved |
| `LOCO_JOG_ACCEL_MULT` | 1 → 2 | +0.0100 | 0.0170 | unresolved |
| `LOCO_STRAFE_DECEL_MULT` | 1.15 → 2.5 | +0.0098 | 0.0146 | unresolved |
| `LOCO_CARRY_DECEL_MULT` | 1.05 → 2.5 | +0.0091 | 0.0192 | unresolved |
| `LOCO_STRAFE_TOP_MULT` | 0.75 → 1.4 | +0.0084 | 0.0184 | unresolved |
| `LOCO_RUN_TURN_MULT` | 1 → 2.5 | +0.0080 | 0.0207 | unresolved |
| `LOCO_SPRINT_CARRY_DECEL_MULT` | 1.0625 → 2.5 | +0.0077 | 0.0165 | unresolved |
| `LOCO_JOG_TOP_MULT` | 1 → 1.4 | +0.0074 | 0.0154 | unresolved |
| `LOCO_REVERSE_ARC_COS` | -0.5 → 0 | +0.0069 | 0.0165 | unresolved |
| `LOCO_JOG_THROTTLE` | 0.65 → 1 | +0.0067 | 0.0215 | unresolved |
| `LOCO_TURN_LOW_SPEED_BONUS` | 2 → 5 | +0.0062 | 0.0164 | unresolved |
| `LOCO_SPRINT_CARRY_TOP_MULT` | 0.874 → 1.1 | -0.0054 | 0.0207 | unresolved |
| `LOCO_SPRINT_ACCEL_MULT` | 0.85 → 2 | +0.0045 | 0.0178 | unresolved |
| `LOCO_SPRINT_CARRY_TURN_MULT` | 0.818 → 2.5 | +0.0042 | 0.0194 | unresolved |
| `LOCO_TECHNIQUE_CURVE_HI` | 1.2 → 2 | +0.0042 | 0.0170 | unresolved |
| `LOCO_TURN_EASE_DEG` | 20 → 45 | +0.0039 | 0.0211 | unresolved |
| `LOCO_JOG_TURN_MULT` | 1.6 → 2.5 | -0.0037 | 0.0189 | unresolved |
| `LOCO_TECHNIQUE_CURVE_LO` | 0.8 → 1 | +0.0022 | 0.0147 | unresolved |
| `LOCO_CARRY_TURN_MULT` | 0.8 → 2.5 | -0.0018 | 0.0164 | unresolved |
| `LOCO_SPRINT_TURN_MULT` | 0.55 → 2.5 | +0.0018 | 0.0179 | unresolved |
| `LOCO_CARRY_ACCEL_MULT` | 0.9 → 2 | +0.0017 | 0.0183 | unresolved |
| `LOCO_STRAFE_TURN_MULT` | 1.3 → 2.5 | +0.0014 | 0.0172 | unresolved |
| `LOCO_FACE_TURN_MULT` | 2 → 4 | +0.0008 | 0.0162 | unresolved |
| `LOCO_BASE_TURN` | 360 → 1080 | +0.0007 | 0.0183 | unresolved |
| `LOCO_FACE_EASE_FLOOR` | 0.2 → 1 | +0.0005 | 0.0168 | unresolved |
| `LOCO_RUN_DECEL_MULT` | 1 → 2.5 | +0.0005 | 0.0138 | unresolved |

**"Unresolved" is not "inert", and the distinction is load-bearing.** This
census resolves effects above roughly **0.017 s — about 3.5% of the 0.48 s
mean.** A knob with a real 0.010 s effect is invisible to it. Pruning the 33
was considered and rejected: #488 itself warns that a band set purely from the
harness "can certify sluggishness as balance", and deleting feel controls that
a cheap measurement cannot see would be that same mistake pointed the other
way. The defensible claim is that 12 knobs have a demonstrable effect at a
seed count a per-PR gate can afford — not that the rest do nothing.

The count went **down**, 15 → 12, when the composition fix landed: four knobs
became reachable and seven fell below threshold, because composing carry onto
the direction context changes which knobs a carrier's reversal passes through.

### The structural half: six decel knobs cannot move this metric

#488 specifies the pairing *"`LOCO_RUN_DECEL` up must lower
`time_to_reverse`"*. Across the knob's whole declared range that reports
`DECORATION` — **structurally wrong, not merely weak**, and it generalizes.

`resolve` tests the movement-versus-facing geometry *before* possession and
sprint, so from the first tick of a commanded reversal the body is moving
opposite the way it looks: it is **`Backpedal`**. Facing needs about 0.25 s to
come round against a reversal that takes about 0.48 s, so a reversal is
roughly half backpedal, then run. Essentially all the braking happens in that
first half.

Consequently **`LOCO_RUN_DECEL_MULT`, `LOCO_SPRINT_DECEL_MULT`,
`LOCO_JOG_DECEL_MULT`, `LOCO_CARRY_DECEL_MULT`, `LOCO_STRAFE_DECEL_MULT` and
`LOCO_SPRINT_CARRY_DECEL_MULT` cannot affect a reversal at all.** Only
`LOCO_BACKPEDAL_DECEL_MULT` can, and it is the knob the shipped braking
assertion perturbs. This was re-measured *after* the carry-composition fix
precisely because several of those knobs had been unreachable by construction
beforehand, and condemning them on the earlier run would have blamed knobs for
a defect. The fix did not rescue them: they are not broken, they are simply
not the brake a reversal uses.

Reach for `MOVE_DECEL` (the shared base every context multiplies) or
`LOCO_BACKPEDAL_DECEL_MULT` when tuning how hard a body brakes out of a run.

### One caution about `fun`

`fun` is a geometric mean over however many metrics extract a value, so
**adding a healthy metric raises it for every build**. `time_to_reverse` sits
inside its band and scores 1.0, which lifts a 9-metric `fun` above the
8-metric one arithmetically, with nothing about the game having improved.
`fun` is therefore not comparable across metric-set versions, which is exactly
what the AI baseline's `baseline_version` is for — and one more reason its
re-freeze has to be deliberate.

## Watch out for

Everything in `docs/design/momentum.md`'s "Watch out for" list still applies
verbatim, plus:

- **Specs that budget N frames for a distance.** Momentum makes closing slower.
  Prefer relaxing the budget over weakening the assertion — the two online
  combat scenarios this change moved (`stagger`, `vs_ranged_scrum`) kept every
  assertion and only got more steps.
- **Fixed-input replays diverge much harder than AI-driven matches.** The OMP-1
  fixture replays frozen inputs against new physics, so bodies end up
  somewhere else entirely: its tackle count goes 147 → 2. The AI-driven
  baseline over the same change stays recognizable (32 shots, 34 passes). A
  fixed-input tape measures "these inputs, this physics", and this changed the
  physics.
- **Render must not latch.** Lean and gait derive from current speed and the
  angle between facing and movement, every frame. A "started sprinting" event
  edge would strand a rolled-back sprint start.
