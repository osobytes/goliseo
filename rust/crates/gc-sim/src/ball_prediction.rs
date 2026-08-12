//! Forward prediction of the ball's trajectory: one authoritative answer to
//! "where will the ball be, and when does it get there".
//!
//! ## Why a service rather than a formula
//!
//! Keeper saves need the ball's true contact point along its flight path.
//! Pass leading needs travel time to a candidate reception point.
//! Interception and leave-the-line logic need "can this player physically
//! arrive before the ball does" answered the same way for every asker.
//! Without one shared answer each consumer grows its own extrapolation, and
//! an ad-hoc extrapolation drifts from what the physics actually does: a
//! keeper that dives to where a hand-rolled quadratic said the ball would
//! be, while the real step applied drag and a bounce, saves shots that would
//! have missed and concedes shots it should have reached.
//!
//! So this service answers by **simulating, not by solving**. It clones the
//! live ball into a scratch world containing only the ball and the ground
//! plane, and advances that world with [`crate::ball_flight::step`] — the
//! identical function the live simulation runs on a loose ball, not a copy
//! of it. Because the scratch world runs the real step function, its answers
//! cannot drift from what the physics will do.
//!
//! ## Determinism
//!
//! - **No randomness.** [`crate::ball_flight::step`] has no access to
//!   `MatchState::rng`; the scratch world therefore consumes zero stream
//!   entries. If it did, a peer that predicted a different number of times
//!   would resimulate a different match.
//! - **No live mutation.** Every query takes `&MatchState`. The borrow
//!   checker is the proof: a query cannot write simulation state, so query
//!   history cannot enter a snapshot, a state hash, or a peer comparison.
//! - **Nothing retained across a rollback.** The load-bearing invariant is
//!   *fingerprint purity*: a prediction is a pure function of the ball state
//!   and the arena, and the cache is keyed on a bit-exact fingerprint of
//!   exactly those inputs. A restore rewinds the ball, the fingerprint moves,
//!   and the buffer is rebuilt — so a buffer built on a mispredicted timeline
//!   cannot be served on the corrected one even if the corrected timeline
//!   re-enters the same tick number. `tick_seq` does *not* carry that
//!   argument; it bounds the per-tick budget window, and forcing a rebuild at
//!   each tick boundary is a consequence of that rather than the safety
//!   property. Purity is what makes the cache correct; `tick_seq` is what
//!   makes the budget mean "per tick".
//!
//! ## The sample buffer is derived data
//!
//! It is not part of [`MatchState`], so it is structurally excluded from
//! snapshots and from the state hash rather than excluded by a list someone
//! has to maintain. Two peers with identical sim state and different query
//! histories hash identically because there is nothing of this service in
//! the hashed shape at all.
//!
//! ## Cache and invalidation
//!
//! The buffer is keyed by `(tick_seq, ball_generation)`.
//!
//! `ball_generation` is bumped by **any** change to the live ball's state —
//! the live physics step, a kick, a deflection, a possession pickup, a
//! restart placement. It is derived by fingerprinting the live ball
//! (position, velocity, height, vertical velocity, spin, owner) and the
//! arena, and bumping when the fingerprint differs from the last one
//! observed. That is deliberately not a counter sprinkled across the forty
//! ball-assignment sites in `crate::r#match`: a call site that someone
//! forgets to add is a cache that silently serves pre-mutation samples,
//! whereas a fingerprint covers every mutation path — including ones added
//! later — by construction. A mutation that restores the ball to a
//! bit-identical state does not bump, and correctly so: the prediction is a
//! pure function of ball state and arena, so the buffer is still the right
//! answer.
//!
//! `tick_seq` is bumped by [`BallPredictor::begin_tick`], called once per
//! live tick by whoever owns the predictor. Its job is the per-tick step
//! budget's boundary; forcing the first query of each tick to rebuild falls
//! out of that. It is not what makes a post-rollback query safe — the
//! fingerprint is (see "Determinism" above).
//!
//! ## Budget, horizon, and the fallback
//!
//! The buffer extends lazily, only as far as the furthest query asked this
//! tick, capped at [`BallPredictionConfig::max_horizon`]. A per-tick step
//! budget bounds how many scratch ticks may be executed in one live tick
//! **across all callers**. Exhausting it never stalls the frame:
//!
//! - **Authoritative queries** ([`BallPredictor::position_at_time`] and the
//!   other three kinds) return an explicit `None`. Callers must handle it.
//! - **Tolerant queries** ([`BallPredictor::estimate_position_at_time`])
//!   fall back to a cheap closed-form ballistic estimate.
//!
//! The fallback is *never* an authoritative answer, and that is structural
//! rather than documented: it can only ever be returned as a
//! [`BallEstimate`], a type no authoritative entry point returns and which
//! carries no conversion into [`BallSample`]. A save or pass-release path
//! written against [`BallSample`] cannot be handed a fallback value by
//! accident, because it would not type-check.
//!
//! ## What this service does *not* model
//!
//! The scratch world holds the ball and the ground plane. It does not model
//! players, so a predicted path ignores the body block, keeper save or
//! aerial contact that may end it early. It predicts the ball's **free
//! flight**: for an owned ball that is the trajectory the ball would take if
//! released now, not the path a dribbler will carry it along.

use crate::ball_flight::{self, BALL_RADIUS, BallArena, BallFlight, GRAVITY};
use crate::fixed_clock;
use crate::match_snapshot::{MatchPlayer, MatchState};
use gc_core::fnv1a64::Fnv1a64State;
use gc_core::vec2::Vec2;

/// Furthest future the buffer may cover, in seconds (`predict.max_horizon`).
///
/// Two seconds at the canonical 60 Hz tick is 120 scratch ticks: long enough
/// to cover a cleared ball's whole flight and a cross-pitch pass, short
/// enough that the worst frame stays bounded (see [`STEP_BUDGET_DEFAULT`]).
pub const MAX_HORIZON_DEFAULT: f64 = 2.0;

/// Scratch ticks executable per live tick, across all callers
/// (`predict.step_budget`).
///
/// **Why 120, justified against the retained rollback window.** Every query
/// issued during a resimulated tick is re-issued, so the real cost of this
/// service is `step_budget × rollback_depth` in the worst frame, not
/// `step_budget`. The project's retained rollback window is measured, not
/// guessed: `Omp2RollbackBudgets::snapshot_count` authors 31 retained
/// boundaries, and `gc-sim/tests/snapshot_headroom.rs` steps a real session
/// past a full ring and asserts the measured `retained_boundaries` equals
/// that 31 — a correction older than the ring is rejected as
/// `LateInputUnrecoverable` rather than resimulated. So 31 is the hard
/// ceiling on rollback depth, and the worst frame this default admits is
/// `120 × 31 = 3,720` scratch ball steps.
///
/// A scratch step is [`crate::ball_flight::step`] alone: no players, no AI,
/// no combat, no collision resolution. It is a small fraction of a full live
/// tick, and 3,720 of them sit beside the 31 *full* ticks that same worst
/// frame already resimulates. 120 also equals exactly one full-horizon
/// rebuild per live tick at [`MAX_HORIZON_DEFAULT`], which is the honest
/// unit to reason in: this default buys every consumer, together, one
/// complete 2-second trajectory per tick.
///
/// This is a *ceiling chosen against a measured window*, not a measurement
/// of demand — the keeper and passing consumers that will spend it do not
/// exist yet, so there is no realistic per-tick query mix to measure. The
/// budget is deliberately generous for that reason; re-deriving it from
/// measured consumption before enabling it under ranked latency is the
/// follow-up the issue calls for.
pub const STEP_BUDGET_DEFAULT: i64 = 120;

/// Scratch ticks per stored sample (`predict.sample_stride`). One stores
/// every tick, so sample points are exact and interpolation only ever
/// happens between adjacent ticks.
pub const SAMPLE_STRIDE_DEFAULT: i64 = 1;

/// Whether the closed-form fallback models gravity only, ignoring the
/// bounce (`predict.fallback_gravity_only`).
pub const FALLBACK_GRAVITY_ONLY_DEFAULT: bool = true;

/// How many bounces the non-gravity-only fallback chains before giving up
/// and settling the ball. Bounded so the "cheap" flavor stays cheap.
const FALLBACK_MAX_BOUNCES: i64 = 4;

/// Tolerance for float time comparisons, in seconds. Two orders of
/// magnitude below one 60 Hz tick.
const TIME_EPSILON: f64 = 1e-9;

/// The service's tunables. Registered as sim-affecting scalars when the
/// declarative tunable registry lands; plain values until then.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallPredictionConfig {
    /// Furthest future the buffer may cover, in seconds.
    pub max_horizon: f64,
    /// Scratch ticks executable per live tick, across all callers.
    pub step_budget: i64,
    /// Scratch ticks per stored sample.
    pub sample_stride: i64,
    /// Whether the closed-form fallback ignores the bounce.
    pub fallback_gravity_only: bool,
    /// The fixed tick the scratch world steps with. Must equal
    /// [`crate::fixed_clock::TICK_SECONDS`] — [`BallPredictor::new`] asserts
    /// it — because sample points are contractually bit-exact against the
    /// live sim, which only holds when both step the same tick length.
    pub dt: f64,
}

impl Default for BallPredictionConfig {
    fn default() -> Self {
        BallPredictionConfig {
            max_horizon: MAX_HORIZON_DEFAULT,
            step_budget: STEP_BUDGET_DEFAULT,
            sample_stride: SAMPLE_STRIDE_DEFAULT,
            fallback_gravity_only: FALLBACK_GRAVITY_ONLY_DEFAULT,
            dt: fixed_clock::TICK_SECONDS,
        }
    }
}

/// One sampled ball state along the predicted path.
///
/// Produced only by the scratch world. The closed-form fallback cannot
/// produce one — see [`BallEstimate`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallSample {
    /// Seconds from now.
    pub time: f64,
    /// Ground position.
    pub pos: Vec2,
    /// Horizontal velocity, px/s.
    pub vel: Vec2,
    /// Height above the pitch.
    pub z: f64,
    /// Vertical velocity, px/s.
    pub vz: f64,
    /// Cumulative ground path length travelled to reach this sample, px.
    pub path: f64,
}

/// Where a [`BallEstimate`] came from. Drives `predict.fallback_ratio`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BallEstimateSource {
    /// Read off the real sample buffer.
    Sampled,
    /// Produced by the closed-form fallback.
    Fallback,
}

/// A **non-authoritative** ball estimate.
///
/// This is the return type of the tolerant query flavor, and the only type
/// the closed-form fallback can produce. It is deliberately not a
/// [`BallSample`] and there is no conversion between them: authoritative
/// resolution paths (a save verdict, a pass release solution) are written
/// against [`BallSample`], so a fallback answer cannot reach them without a
/// compile error.
///
/// Accuracy contract: "cheap and wrong-ish". Constant-velocity ground
/// travel, gravity for an airborne ball, no drag, no walls, and bounces only
/// when [`BallPredictionConfig::fallback_gravity_only`] is off. It ignores
/// spin entirely, and if spin gains further gameplay effects later this
/// flavor degrades further while the sampled flavor inherits them for free.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallEstimate {
    /// Seconds from now.
    pub time: f64,
    /// Estimated ground position.
    pub pos: Vec2,
    /// Estimated height above the pitch.
    pub z: f64,
    /// Whether this came off the sample buffer or the closed form.
    pub source: BallEstimateSource,
}

/// A ground axis, for [`BallPlane`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BallAxis {
    /// The pitch's long axis (goal to goal).
    X,
    /// The pitch's short axis (touchline to touchline).
    Y,
}

/// A vertical plane perpendicular to a ground axis — a goal line, an
/// offside-style reference line, a keeper's line of engagement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallPlane {
    /// Which ground axis the plane is perpendicular to.
    pub axis: BallAxis,
    /// The plane's coordinate on that axis.
    pub coord: f64,
}

/// The answer to "can this player reach `point` before the ball arrives".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BallReach {
    /// Whether the player's best case beats the ball.
    pub reachable: bool,
    /// The player's best-case arrival time at the point, in seconds.
    pub arrival: f64,
    /// `t_arrival - arrival`: seconds to spare (positive) or short
    /// (negative).
    pub margin: f64,
}

/// Why an extension stopped short.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExtendStop {
    /// The buffer reached [`BallPredictionConfig::max_horizon`].
    Horizon,
    /// This live tick's step budget is spent.
    Budget,
}

/// Harness-visible counters. Not fun-score metrics: these say whether the
/// service is starving its callers, not whether the game is any good.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BallPredictionTelemetry {
    /// `predict.budget_exhaustions`: queries denied an extension because
    /// this live tick's step budget was spent.
    pub budget_exhaustions: i64,
    /// Queries answered, of any flavor.
    pub answers: i64,
    /// Answers served by the closed-form fallback.
    pub fallback_answers: i64,
    /// Scratch ticks executed.
    pub scratch_steps: i64,
    /// Times the buffer was discarded and re-cloned.
    pub rebuilds: i64,
}

impl BallPredictionTelemetry {
    /// `predict.fallback_ratio`: fallback answers over total answers. Zero
    /// when nothing has been asked.
    #[must_use]
    pub fn fallback_ratio(&self) -> f64 {
        if self.answers == 0 {
            0.0
        } else {
            self.fallback_answers as f64 / self.answers as f64
        }
    }
}

/// This telemetry as one collision-safe marker line, for harness logs.
#[must_use]
pub fn marker(telemetry: &BallPredictionTelemetry) -> String {
    format!(
        "GC_PREDICT|budget_exhaustions={}|fallback_ratio={:.6}|answers={}|fallback_answers={}\
         |scratch_steps={}|rebuilds={}",
        telemetry.budget_exhaustions,
        telemetry.fallback_ratio(),
        telemetry.answers,
        telemetry.fallback_answers,
        telemetry.scratch_steps,
        telemetry.rebuilds
    )
}

/// The forward-prediction service. One instance per match state.
///
/// Queries take `&MatchState` and `&mut self`: the mutable borrow is the
/// *cache's*, never the simulation's.
#[derive(Clone, Debug)]
pub struct BallPredictor {
    config: BallPredictionConfig,
    tick_seq: u64,
    generation: u64,
    fingerprint: Option<u64>,
    key: Option<(u64, u64)>,
    arena: Option<BallArena>,
    samples: Vec<BallSample>,
    cursor: BallFlight,
    cursor_steps: i64,
    cursor_path: f64,
    budget_left: i64,
    telemetry: BallPredictionTelemetry,
}

impl Default for BallPredictor {
    fn default() -> Self {
        Self::new(BallPredictionConfig::default())
    }
}

impl BallPredictor {
    /// A predictor with the given tunables and a full first-tick budget.
    ///
    /// # Panics
    ///
    /// Panics when `dt` is not [`crate::fixed_clock::TICK_SECONDS`], when
    /// `sample_stride` is below one tick, or when the horizon or budget is
    /// negative — producer invariants, not recoverable input.
    #[must_use]
    pub fn new(config: BallPredictionConfig) -> Self {
        // Not merely "positive". Sample points are contractually bit-exact
        // against the live sim, and that is only true when the scratch world
        // steps the *same* tick length the live sim does. A predictor
        // configured with any other `dt` would keep answering, silently, with
        // samples that no live tick will ever land on.
        assert!(
            config.dt == fixed_clock::TICK_SECONDS,
            "prediction dt must be the canonical fixed tick"
        );
        assert!(
            config.sample_stride >= 1,
            "prediction sample stride must be at least one tick"
        );
        assert!(
            config.max_horizon >= 0.0,
            "prediction horizon must not be negative"
        );
        assert!(
            config.step_budget >= 0,
            "prediction step budget must not be negative"
        );
        BallPredictor {
            config,
            tick_seq: 0,
            generation: 0,
            fingerprint: None,
            key: None,
            arena: None,
            samples: Vec::new(),
            cursor: BallFlight {
                pos: Vec2::new(0.0, 0.0),
                vel: Vec2::new(0.0, 0.0),
                z: 0.0,
                vz: 0.0,
                spin: 0.0,
            },
            cursor_steps: 0,
            cursor_path: 0.0,
            budget_left: config.step_budget,
            telemetry: BallPredictionTelemetry::default(),
        }
    }

    /// Open a new live tick: refill the step budget and move the cache key
    /// forward so the first query of this tick rebuilds.
    ///
    /// Call once per live tick, before the tick's consumers run. Skipping it
    /// cannot serve stale samples — the generation fingerprint still catches
    /// every ball mutation — but it does let one tick's callers spend a
    /// previous tick's leftover budget, so the per-tick bound stops meaning
    /// what it says.
    pub fn begin_tick(&mut self) {
        self.tick_seq += 1;
        self.budget_left = self.config.step_budget;
    }

    /// This predictor's tunables.
    #[must_use]
    pub fn config(&self) -> BallPredictionConfig {
        self.config
    }

    /// Replace the tunables. Discards the buffer: samples produced under a
    /// different stride or `dt` are not comparable with new ones.
    ///
    /// # Panics
    ///
    /// Panics on the same producer invariants as [`BallPredictor::new`].
    pub fn set_config(&mut self, config: BallPredictionConfig) {
        let telemetry = self.telemetry;
        let tick_seq = self.tick_seq;
        let generation = self.generation;
        *self = Self::new(config);
        self.telemetry = telemetry;
        self.tick_seq = tick_seq;
        self.generation = generation;
    }

    /// The harness-visible counters.
    #[must_use]
    pub fn telemetry(&self) -> BallPredictionTelemetry {
        self.telemetry
    }

    /// Zero the counters, keeping the cache. For a harness that reports per
    /// match rather than per process.
    pub fn reset_telemetry(&mut self) {
        self.telemetry = BallPredictionTelemetry::default();
    }

    /// Scratch ticks still executable this live tick.
    #[must_use]
    pub fn budget_remaining(&self) -> i64 {
        self.budget_left
    }

    /// The current ball generation. Bumps on every observed ball mutation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Samples currently buffered, including the `t = 0` clone.
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    // -----------------------------------------------------------------
    // Authoritative queries. Never fall back; unresolvable is `None`.
    // -----------------------------------------------------------------

    /// The ball's state at `time` seconds from now, interpolated linearly
    /// between the two bracketing samples.
    ///
    /// `None` when `time` is negative, beyond the horizon, or beyond what
    /// this tick's remaining budget can reach.
    pub fn position_at_time(&mut self, state: &MatchState, time: f64) -> Option<BallSample> {
        self.sync(state);
        self.telemetry.answers += 1;
        if time < 0.0 {
            return None;
        }
        loop {
            if let Some(last) = self.samples.last()
                && last.time + TIME_EPSILON >= time
            {
                return self.interpolate_at(time);
            }
            self.extend_one().ok()?;
        }
    }

    /// The first state at which the ball's cumulative ground path length
    /// reaches `distance`.
    ///
    /// `None` when the ball never travels that far inside the horizon, or
    /// the budget ran out first.
    pub fn state_after_distance(
        &mut self,
        state: &MatchState,
        distance: f64,
    ) -> Option<BallSample> {
        self.sync(state);
        self.telemetry.answers += 1;
        if distance < 0.0 {
            return None;
        }
        let mut index = 0usize;
        loop {
            while index + 1 < self.samples.len() {
                let a = self.samples[index];
                let b = self.samples[index + 1];
                if b.path >= distance {
                    if a.path >= distance {
                        return Some(a);
                    }
                    let span = b.path - a.path;
                    let t = if span > 0.0 {
                        (distance - a.path) / span
                    } else {
                        0.0
                    };
                    return Some(lerp_sample(&a, &b, t));
                }
                index += 1;
            }
            if let Some(first) = self.samples.first()
                && first.path >= distance
            {
                return Some(*first);
            }
            self.extend_one().ok()?;
        }
    }

    /// The time at which the ball first crosses `plane`, or `None` if it
    /// does not inside the horizon (or the budget ran out first).
    ///
    /// A ball already exactly on the plane crosses at `0.0`.
    pub fn time_to_cross_plane(&mut self, state: &MatchState, plane: BallPlane) -> Option<f64> {
        self.sync(state);
        self.telemetry.answers += 1;
        self.first_crossing(|sample| axis_value(sample, plane.axis) - plane.coord)
    }

    /// The time at which the ball's height first reaches `height`, or `None`
    /// if it does not inside the horizon (or the budget ran out first).
    pub fn time_to_height(&mut self, state: &MatchState, height: f64) -> Option<f64> {
        self.sync(state);
        self.telemetry.answers += 1;
        self.first_crossing(|sample| sample.z - height)
    }

    /// Whether `player` can reach `point` before the ball gets there.
    ///
    /// Pure kinematics over the player's speed cap — it reads no samples and
    /// spends no budget, so it is `&self`. The model is deliberately the
    /// current constant-speed one: straight line, top speed from a standing
    /// start, no acceleration profile. It is therefore *optimistic* about
    /// the player. Once the locomotion rework lands (momentum, separate
    /// accel and decel), this must adopt the same acceleration profile or it
    /// will systematically over-promise.
    #[must_use]
    pub fn reachable_before_arrival(
        &self,
        player: &MatchPlayer,
        point: Vec2,
        t_arrival: f64,
    ) -> BallReach {
        let distance = (player.pos.dist(point) - player.radius - BALL_RADIUS).max(0.0);
        let speed = player.move_speed;
        let arrival = if speed > 0.0 {
            distance / speed
        } else {
            f64::INFINITY
        };
        let margin = t_arrival - arrival;
        BallReach {
            reachable: margin >= 0.0,
            arrival,
            margin,
        }
    }

    // -----------------------------------------------------------------
    // Tolerant query flavor. Degrades to the closed form; never
    // authoritative.
    // -----------------------------------------------------------------

    /// A **non-authoritative** estimate of the ball's position at `time`.
    ///
    /// Prefers the real sample buffer; falls back to the closed form when
    /// the horizon or this tick's budget puts `time` out of reach. For AI
    /// heuristic scoring and coarse "is anything even nearby" pre-filters —
    /// callers that would rather have a wrong number than no number. Never
    /// for a save verdict or a pass release solution: those are written
    /// against [`BallSample`], which this cannot produce.
    pub fn estimate_position_at_time(&mut self, state: &MatchState, time: f64) -> BallEstimate {
        self.sync(state);
        self.telemetry.answers += 1;
        let clamped = time.max(0.0);
        loop {
            if let Some(last) = self.samples.last()
                && last.time + TIME_EPSILON >= clamped
            {
                if let Some(sample) = self.interpolate_at(clamped) {
                    return BallEstimate {
                        time: clamped,
                        pos: sample.pos,
                        z: sample.z,
                        source: BallEstimateSource::Sampled,
                    };
                }
                break;
            }
            if self.extend_one().is_err() {
                break;
            }
        }
        self.telemetry.fallback_answers += 1;
        fallback_estimate(&BallFlight::of(state), clamped, self.config)
    }

    // -----------------------------------------------------------------
    // Cache, scratch world, budget.
    // -----------------------------------------------------------------

    /// Fingerprint the live ball, bump the generation on any change, and
    /// rebuild the buffer when the key no longer matches.
    fn sync(&mut self, state: &MatchState) {
        let fingerprint = ball_fingerprint(state);
        if self.fingerprint != Some(fingerprint) {
            self.generation += 1;
            self.fingerprint = Some(fingerprint);
        }
        let key = (self.tick_seq, self.generation);
        if self.key != Some(key) {
            self.rebuild(state);
            self.key = Some(key);
        }
    }

    /// Discard the buffer and re-clone the live ball into the scratch world.
    fn rebuild(&mut self, state: &MatchState) {
        self.cursor = BallFlight::of(state);
        self.arena = Some(BallArena::of(state));
        self.cursor_steps = 0;
        self.cursor_path = 0.0;
        self.samples.clear();
        self.samples.push(BallSample {
            time: 0.0,
            pos: self.cursor.pos,
            vel: self.cursor.vel,
            z: self.cursor.z,
            vz: self.cursor.vz,
            path: 0.0,
        });
        self.telemetry.rebuilds += 1;
    }

    /// Seconds the scratch cursor has advanced.
    fn cursor_time(&self) -> f64 {
        self.cursor_steps as f64 * self.config.dt
    }

    /// Advance the scratch world exactly one tick, spending one unit of
    /// budget. `Err` when the horizon or the budget stops it — in which case
    /// the cursor is flushed into the buffer first, so the last sample is
    /// always as far as this tick could see.
    fn extend_one(&mut self) -> Result<(), ExtendStop> {
        if self.cursor_time() + self.config.dt > self.config.max_horizon + TIME_EPSILON {
            self.flush_tail();
            return Err(ExtendStop::Horizon);
        }
        if self.budget_left <= 0 {
            self.flush_tail();
            self.telemetry.budget_exhaustions += 1;
            return Err(ExtendStop::Budget);
        }
        let arena = self.arena.expect("a synced predictor has cloned an arena");
        let before = self.cursor.pos;
        ball_flight::step(&mut self.cursor, &arena, self.config.dt);
        self.cursor_steps += 1;
        self.budget_left -= 1;
        self.telemetry.scratch_steps += 1;
        self.cursor_path += self.cursor.pos.dist(before);
        if self.cursor_steps % self.config.sample_stride == 0 {
            self.push_cursor();
        }
        Ok(())
    }

    /// Store the cursor as a sample if the stride has not already.
    fn flush_tail(&mut self) {
        let cursor_time = self.cursor_time();
        let stored = self.samples.last().map_or(-1.0, |sample| sample.time);
        if stored + TIME_EPSILON < cursor_time {
            self.push_cursor();
        }
    }

    fn push_cursor(&mut self) {
        self.samples.push(BallSample {
            time: self.cursor_time(),
            pos: self.cursor.pos,
            vel: self.cursor.vel,
            z: self.cursor.z,
            vz: self.cursor.vz,
            path: self.cursor_path,
        });
    }

    /// Linear interpolation between the two samples bracketing `time`.
    ///
    /// A time that lands **on** a sample returns that sample untouched
    /// rather than a blend with weight one. That is not a micro-optimization:
    /// `a + (b - a) * 1.0` is not bit-identically `b` in IEEE 754, and the
    /// accuracy contract says sample points are exact.
    fn interpolate_at(&self, time: f64) -> Option<BallSample> {
        let first = *self.samples.first()?;
        if time <= first.time + TIME_EPSILON {
            return Some(first);
        }
        for window in self.samples.windows(2) {
            let (a, b) = (window[0], window[1]);
            if time <= b.time + TIME_EPSILON {
                if (time - a.time).abs() <= TIME_EPSILON {
                    return Some(a);
                }
                if (time - b.time).abs() <= TIME_EPSILON {
                    return Some(b);
                }
                let span = b.time - a.time;
                let t = if span > 0.0 {
                    (time - a.time) / span
                } else {
                    0.0
                };
                return Some(lerp_sample(&a, &b, t));
            }
        }
        None
    }

    /// The time at which `signed` first changes sign (or touches zero),
    /// extending the buffer until it does.
    fn first_crossing<F>(&mut self, signed: F) -> Option<f64>
    where
        F: Fn(&BallSample) -> f64,
    {
        let mut index = 0usize;
        loop {
            if index == 0
                && let Some(first) = self.samples.first()
                && signed(first) == 0.0
            {
                return Some(first.time);
            }
            while index + 1 < self.samples.len() {
                let a = self.samples[index];
                let b = self.samples[index + 1];
                let (sa, sb) = (signed(&a), signed(&b));
                if sb == 0.0 {
                    return Some(b.time);
                }
                if (sa < 0.0) != (sb < 0.0) {
                    let span = sb - sa;
                    let t = if span == 0.0 { 0.0 } else { -sa / span };
                    return Some(a.time + (b.time - a.time) * t);
                }
                index += 1;
            }
            self.extend_one().ok()?;
        }
    }
}

/// Linear blend between two samples, `t` in `0..=1`. The endpoints are
/// returned exactly rather than blended, for the reason
/// [`BallPredictor::interpolate_at`] documents.
fn lerp_sample(a: &BallSample, b: &BallSample, t: f64) -> BallSample {
    if t <= 0.0 {
        return *a;
    }
    if t >= 1.0 {
        return *b;
    }
    BallSample {
        time: a.time + (b.time - a.time) * t,
        pos: a.pos.add(b.pos.sub(a.pos).scale(t)),
        vel: a.vel.add(b.vel.sub(a.vel).scale(t)),
        z: a.z + (b.z - a.z) * t,
        vz: a.vz + (b.vz - a.vz) * t,
        path: a.path + (b.path - a.path) * t,
    }
}

/// The ground coordinate a [`BallPlane`] measures against.
fn axis_value(sample: &BallSample, axis: BallAxis) -> f64 {
    match axis {
        BallAxis::X => sample.pos.x,
        BallAxis::Y => sample.pos.y,
    }
}

/// A bit-exact fingerprint of everything the scratch world clones: the ball,
/// the arena, and the owner (a possession pickup is a ball mutation even on
/// the tick it does not move the ball).
///
/// # This must cover every input the scratch step reads
///
/// The cache is correct because a prediction is a pure function of the values
/// hashed here. That holds today only because every *other* input to
/// [`crate::ball_flight::step`] — friction, air drag, gravity, restitution,
/// spin decay, the settle threshold, the cage ceiling, the net damping — is a
/// hardcoded `const`, and a constant cannot change mid-match.
///
/// **A requirement on whoever migrates those constants to the declarative
/// tunable registry:** the moment any of them becomes registry-backed sim
/// state, its live value must be folded into this fingerprint. Otherwise a
/// mid-match tuning change leaves the ball itself bit-identical, the
/// fingerprint unmoved, and the buffer serving samples produced under the old
/// physics — a silent wrong answer with no failing test, because the ball
/// state the fingerprint watches genuinely did not change. The knobs this
/// service owns (horizon, budget, stride, fallback flavor) do not need
/// hashing: they change what the buffer *covers*, not what the physics
/// computes, and [`BallPredictor::set_config`] already discards the buffer.
fn ball_fingerprint(state: &MatchState) -> u64 {
    let mut hasher = Fnv1a64State::new();
    for value in [
        state.ball.x,
        state.ball.y,
        state.ball_vel.x,
        state.ball_vel.y,
        state.ball_z,
        state.ball_vz,
        state.ball_spin,
        state.field.w,
        state.field.h,
        state.goal_home.x,
        state.goal_home.y,
        state.goal_home.w,
        state.goal_home.h,
        state.goal_away.x,
        state.goal_away.y,
        state.goal_away.w,
        state.goal_away.h,
    ] {
        hasher.update(&value.to_bits().to_be_bytes());
    }
    hasher.update(&state.owner.unwrap_or(0).to_be_bytes());
    u64::from_str_radix(&hasher.hex(), 16).expect("fnv1a64 renders 16 hex digits")
}

/// The closed-form ballistic estimate: constant-velocity ground travel,
/// gravity for an airborne ball, no drag and no walls. Bounces only when
/// [`BallPredictionConfig::fallback_gravity_only`] is off, and then only
/// [`FALLBACK_MAX_BOUNCES`] times.
///
/// A free function, not a method, so it is obvious that it reads no cache
/// and spends no budget.
#[must_use]
pub fn fallback_estimate(
    ball: &BallFlight,
    time: f64,
    config: BallPredictionConfig,
) -> BallEstimate {
    let time = time.max(0.0);
    let pos = ball.pos.add(ball.vel.scale(time));
    let z = if config.fallback_gravity_only {
        (ball.z + ball.vz * time - 0.5 * GRAVITY * time * time).max(0.0)
    } else {
        bouncing_height(ball.z, ball.vz, time)
    };
    BallEstimate {
        time,
        pos,
        z,
        source: BallEstimateSource::Fallback,
    }
}

/// Height after `time` under gravity with a bounded chain of restitution
/// bounces.
fn bouncing_height(z0: f64, vz0: f64, time: f64) -> f64 {
    let mut z = z0;
    let mut vz = vz0;
    let mut left = time;
    for _ in 0..FALLBACK_MAX_BOUNCES {
        // Time until this arc lands: solve 0 = z + vz*t - g/2 t^2.
        let discriminant = vz * vz + 2.0 * GRAVITY * z;
        if discriminant <= 0.0 {
            break;
        }
        let landing = (vz + discriminant.sqrt()) / GRAVITY;
        if landing >= left || landing <= 0.0 {
            return (z + vz * left - 0.5 * GRAVITY * left * left).max(0.0);
        }
        let impact = vz - GRAVITY * landing;
        left -= landing;
        z = 0.0;
        vz = -impact * ball_flight::BOUNCE;
        if vz <= 0.0 {
            break;
        }
    }
    0.0
}
