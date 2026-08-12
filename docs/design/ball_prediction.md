# Ball forward prediction

Status: implemented in `rust/crates/gc-sim/src/ball_prediction.rs`
(service) and `rust/crates/gc-sim/src/ball_flight.rs` (the shared free-flight
step). No production consumer yet — the keeper rework and the passing rework
are the first two, and both were blocked on this existing.

## The question this answers

Three upcoming systems need to know something about the ball's future:

- keeper save resolution needs the ball's true contact point along its flight
  path, and the time it gets there;
- pass leading needs travel time to a candidate reception point;
- interception and leave-the-line logic need "can this player physically
  arrive before the ball does", answered the same way for every asker.

Without one shared answer, each of them grows its own extrapolation. That is
the failure mode worth naming: a keeper diving to where a hand-rolled
quadratic said the ball would be, while the real step function applied drag
and a bounce, saves shots that would have missed and concedes shots it should
have reached. Two implementations of the physics disagree the day one of them
changes, and nothing tells you which one the player is watching.

## Simulate, don't solve

The service answers by cloning the live ball into a scratch world containing
only the ball and the ground plane, and stepping that world with **the live
simulation's own ball update function**. That is `ball_flight::step`, which
`match::update_ball` calls for the live loose ball and the predictor calls for
its scratch ticks. Extracted, not copied — the extraction is the design.

`tests/ball_prediction.rs::the_scratch_world_reproduces_the_live_sims_own_ball_path_bit_for_bit`
is what keeps that true. It predicts a whole trajectory from one live state,
then steps the *live simulation* through the same ticks and requires the two
to agree bit for bit — not within a tolerance, which a second implementation
could pass by luck. The fixture asserts per tick that nothing touched the
ball, because a contact would make the live path legitimately diverge from a
ball-and-ground-plane scratch world and the case would then be asserting the
wrong thing quietly.

What the scratch world therefore does **not** model: players. A predicted path
ignores the body block, keeper save or aerial contact that may end it early,
and for an owned ball it is the trajectory the ball would take if released
now, not the path a dribbler will carry it along.

## Queries

All queries read a time-ordered sample buffer, interpolating linearly between
the two bracketing samples. A time that lands on a sample returns that sample
untouched: `a + (b - a) * 1.0` is not bit-identically `b`, and sample points
are contractually exact.

| query | answers |
| --- | --- |
| `position_at_time(t)` | ball state `t` seconds from now |
| `state_after_distance(d)` | first state at cumulative path length `d` |
| `time_to_cross_plane(plane)` / `time_to_height(h)` | first crossing |
| `reachable_before_arrival(player, point, t)` | yes/no plus margin |

Every one of them returns an explicit `None` when it cannot resolve inside
the horizon or the remaining budget. Callers must handle it.

`reachable_before_arrival` uses the current constant-speed kinematic model:
straight line, top speed from a standing start. It is therefore *optimistic*
about the player. When the locomotion rework lands (momentum, separate accel
and decel) this must adopt the same acceleration profile or it will
systematically over-promise.

## Lazy horizon, hard budget, and the fallback

The buffer extends only as far as the furthest query asked this tick, capped
at `predict.max_horizon`. A per-tick step budget bounds scratch ticks across
**all** callers, so consumers cannot each claim a fresh allowance. Exhausting
it never stalls the frame:

- authoritative queries return `None`;
- the tolerant flavor (`estimate_position_at_time`) degrades to a cheap
  closed-form ballistic estimate.

The fallback being non-authoritative is structural, not a warning in a doc
comment. Its only output type is `BallEstimate`; the authoritative queries
return `BallSample`, and there is no conversion between them. A save verdict
or pass-release solution written against `BallSample` cannot be handed a
fallback value without a compile error.

## Cache, invalidation, and rollback

The buffer is keyed by `(tick_seq, ball_generation)`.

`ball_generation` bumps on **any** external change to the ball — the live
physics step, a kick, a deflection, a possession pickup, a restart placement.
It is derived by fingerprinting the live ball and the arena and bumping when
the fingerprint moves, rather than by a counter added at each of the forty-odd
ball-assignment sites in `match.rs`. That choice is deliberate: a call site
someone forgets to add is a cache that silently serves pre-mutation samples,
whereas a fingerprint covers every mutation path — including paths added
later — by construction. `tests/ball_prediction.rs` asserts both halves: an
enumerated table of mutation shapes, and a real 900-tick match that requires
the generation to move *whenever* the live ball moved, over whatever paths
that match happened to exercise.

`tick_seq` bumps once per live tick via `begin_tick`. It is monotonic and
never rewinds, so a buffer built on a mispredicted timeline can never be
served on the corrected one — the first query after a rollback restore always
rebuilds.

The sample buffer is derived data and lives on the service, not on
`MatchState`. It is therefore excluded from snapshots and from the state hash
*structurally* — there is no exclusion list to maintain and nothing to forget.
Two peers with identical sim state and different query histories hash
identically because there is nothing of this service in the hashed shape at
all.

The scratch step consumes **zero randomness**, and its signature is the proof:
it is handed a ball and an arena and has no access to `MatchState::rng`. If
future ball physics wants a random element it must be resolved into ball state
at impulse time, never drawn during flight — otherwise every prediction spends
stream entries the live sim never sees and resimulation diverges.

## Why the default budget is 120

Every query issued during a resimulated tick is re-issued, so the real cost of
the service is `step_budget × rollback_depth` in the worst frame, not
`step_budget`.

The retained rollback window is measured rather than guessed.
`gc_data::omp2_rollback_validation::DATA.budgets.snapshot_count` authors 31
retained boundaries, and `gc-sim/tests/snapshot_headroom.rs` steps a real
session past a full ring and asserts the *measured* `retained_boundaries`
equals that 31 (#476). A correction older than the ring is rejected as
`LateInputUnrecoverable` rather than resimulated, so 31 is a hard ceiling on
rollback depth, not a typical value.

The default therefore admits a worst frame of `120 × 31 = 3,720` scratch ball
steps. A scratch step is `ball_flight::step` alone — no players, no AI, no
combat, no collision resolution — and those 3,720 sit beside the 31 *full*
ticks that same worst frame already resimulates. 120 is also exactly one
full-horizon rebuild per live tick at `predict.max_horizon = 2.0` s and 60 Hz,
which is the honest unit to reason in: the default buys every consumer,
together, one complete two-second trajectory per tick.

This is a ceiling chosen against a measured window, **not** a measurement of
demand. The consumers that will spend it do not exist yet, so there is no
realistic per-tick query mix to measure; the budget is deliberately generous
for that reason. Re-deriving it from measured consumption before enabling it
under ranked latency is an explicit follow-up.

`tests/ball_prediction.rs::the_default_budget_covers_one_full_horizon_rebuild_and_a_bounded_worst_frame`
pins both numbers this argument rests on, so the prose above cannot drift away
from the code or from the authored rollback budget it cites.

## Tunables

Landed as plain constants and a `BallPredictionConfig`, not registry entries:
the declarative tunable registry is a separate, parallel change and this
service should not pre-empt its shape.

| knob | constant | default |
| --- | --- | --- |
| `predict.max_horizon` | `MAX_HORIZON_DEFAULT` | 2.0 s |
| `predict.step_budget` | `STEP_BUDGET_DEFAULT` | 120 ticks |
| `predict.sample_stride` | `SAMPLE_STRIDE_DEFAULT` | 1 tick |
| `predict.fallback_gravity_only` | `FALLBACK_GRAVITY_ONLY_DEFAULT` | on |

## Telemetry

`BallPredictionTelemetry` carries `predict.budget_exhaustions` and the counts
behind `predict.fallback_ratio`, rendered by `ball_prediction::marker` as a
`GC_PREDICT|...` line. `tests/ball_prediction.rs` prints it from the rollback
case, so any run that exercises the service reports whether callers are
starving each other.

These counters live on the service rather than in `sim::metrics`. That is a
deliberate, disclosed limitation: with no production consumer, there is no
match-long query stream to aggregate, and wiring a metric channel for a
counter that is structurally zero would be the kind of gate that measures
nothing. Aggregating it into match telemetry belongs with the first real
consumer.
