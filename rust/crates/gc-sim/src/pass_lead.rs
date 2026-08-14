//! The lead solver: where to put a pass so it meets a moving receiver's
//! stride instead of their heels.
//!
//! Pure over the simulation — `state` is borrowed immutably, the only
//! external call is into [`crate::ball_prediction`] (also `sim/`), no RNG is
//! drawn, and the candidate set is fixed and iterated in a stable order. A
//! resimulated release therefore re-solves identically.
//!
//! ## The shape, and what changed from the issue that specified it
//!
//! The issue's step 4 is *"if required speed ≤ receiver's cap ×
//! `lead_tolerance`"*. **That test is now wrong**, and it is worth being
//! precise about why rather than quietly replacing it.
//!
//! It was written before #488 landed. #488 gave bodies momentum: separate
//! acceleration and deceleration, a bounded turn rate, a reversal that brakes
//! through zero, and per-context multipliers on all three
//! (`gc_data::locomotion`). A receiver therefore cannot reach its top speed
//! instantly, cannot redirect without an arc, and pays a full stop to turn
//! around. Comparing a *required average speed* against a *flat top-speed
//! cap* ignores every one of those, in one direction only: it systematically
//! **over-promises**. The solver would lead passes to points the receiver
//! provably cannot reach, and — because the ball would arrive at an empty
//! patch of grass with a teammate labouring toward it — the failure would
//! read as bad receiver AI rather than as a bad solver. #486's own risk list
//! predicted exactly this for `reachable_before_arrival`, which still uses
//! the constant-speed model.
//!
//! So admissibility is a **time** test against the real locomotion
//! primitive: [`crate::locomotion::time_to_reach`] steps `resolve` /
//! `profile` / `step` — the identical three functions every player's movement
//! runs through every tick — with a flat-out straight command at the
//! candidate point, and the candidate is admissible when the body gets there
//! within `t_ball × PASS_LEAD_TOLERANCE`. What that model still ignores is
//! enumerated on `time_to_reach` itself rather than implied here.
//!
//! `PASS_LEAD_TOLERANCE` keeps its name and its sense: it is how much of the
//! receiver's capability the solver is willing to demand. Below 1.0 it
//! requires slack (the receiver must arrive *early*), so fewer and shorter
//! leads survive; above 1.0 it will lead a ball the receiver has to chase.
//! The units moved from "fraction of a speed cap" to "fraction of the ball's
//! travel time" because the model did.
//!
//! ## One burst, at release only — and it does NOT fit the shared default
//!
//! Every prediction query this module issues happens at **release**, never
//! during preview: selection (`crate::passing`) needs only current positions,
//! so the per-frame preview path never touches the service. The cost of a
//! pass is therefore `candidate_count` queries once, multiplied by rollback
//! depth only on a release tick that is itself resimulated.
//!
//! The issue predicted this "fits the service budget by construction". It
//! does not, and that is a measurement rather than a guess:
//! `BallPredictionConfig`'s shipped `step_budget` is 120 scratch ticks,
//! chosen as *one full-horizon rebuild per live tick*, and a lead burst is
//! `candidate_count` **independent** trajectories rather than one — a
//! hypothetical launch cannot share the live ball's sample buffer, because it
//! is a different curve. `tests/pass_lead.rs` measured five candidates
//! spending far past 120 on an ordinary 200 px pass and silently losing the
//! longest leads to `budget_exhaustions`, which biases the solver toward
//! short leads exactly where it looks like it is working.
//!
//! So [`release_predictor`] hands the release path its own instance whose
//! budget is **derived from the burst it must serve** — `MAX_CANDIDATES`
//! full-horizon trajectories — rather than borrowing a per-tick ceiling sized
//! for a different consumer. That is honest about the cost instead of hiding
//! it behind a degradation nobody would see, and it is a per-release budget
//! rather than a per-tick one, so no other consumer's query count can change
//! a release's answer between an original tick and its resimulation.

use crate::ball_flight::BallFlight;
use crate::ball_prediction::{BallPredictionConfig, BallPredictor};
use crate::locomotion::{self, ReachBody};
use crate::match_snapshot::{MatchPlayer, MatchState};
use crate::passing;
use crate::tuning::Tuning;
use gc_core::vec2::Vec2;

/// Knob id: how much of the receiver's arrival capability a lead may demand,
/// as a fraction of the ball's travel time to the lead point.
pub const TOLERANCE_KNOB: &str = "PASS_LEAD_TOLERANCE";
/// Knob id: receiver speed below which a pass is not led at all, px/s.
pub const MIN_SPEED_KNOB: &str = "PASS_LEAD_MIN_SPEED";
/// Knob id: the shortest candidate lead time, seconds.
pub const TIME_MIN_KNOB: &str = "PASS_LEAD_TIME_MIN";
/// Knob id: the longest candidate lead time, seconds.
pub const TIME_MAX_KNOB: &str = "PASS_LEAD_TIME_MAX";
/// Knob id: how many candidate lead times the fixed set holds.
pub const STEPS_KNOB: &str = "PASS_LEAD_STEPS";

/// Hard ceiling on the candidate set, independent of `PASS_LEAD_STEPS`.
///
/// The per-release query burst is `candidate_count` prediction queries, and
/// the budget argument in the module doc is only as good as its bound. A
/// sweep that pushed `PASS_LEAD_STEPS` to its fence must not be able to
/// multiply the burst past what the rollback matrix was measured against.
pub const MAX_CANDIDATES: usize = 6;

/// A predictor sized for exactly one release burst.
///
/// The budget is `MAX_CANDIDATES` full-horizon trajectories: the worst case
/// in which every candidate's ball runs the whole horizon without covering
/// its distance. A real burst spends a fraction of that — see
/// `tests/pass_lead.rs` and the rollback-matrix confirmation in
/// `tests/pass_release_budget.rs` — but the ceiling is what bounds the frame,
/// so it is stated rather than assumed.
///
/// Everything else stays at [`BallPredictionConfig::default`], including the
/// horizon and the fixed tick.
#[must_use]
pub fn release_predictor() -> BallPredictor {
    let default = BallPredictionConfig::default();
    let full_horizon_ticks = (default.max_horizon / default.dt).ceil() as i64;
    BallPredictor::new(BallPredictionConfig {
        step_budget: MAX_CANDIDATES as i64 * full_horizon_ticks,
        ..default
    })
}

/// The five registered scalars the solver reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeadKnobs {
    /// Fraction of the ball's travel time the receiver's own arrival must
    /// fit inside.
    pub tolerance: f64,
    /// Receiver speed below which the pass is not led.
    pub min_speed: f64,
    /// Shortest candidate lead time, seconds.
    pub time_min: f64,
    /// Longest candidate lead time, seconds.
    pub time_max: f64,
    /// Candidate count, clamped to `1..=MAX_CANDIDATES`.
    pub steps: usize,
}

impl LeadKnobs {
    /// Read the live values out of `tune`.
    ///
    /// `PASS_LEAD_STEPS` is a registered tier-1 scalar like everything else —
    /// the registry, the sweep and the F1 panel are all `f64` — so it is
    /// truncated here rather than authored as an integer. Truncation, not
    /// rounding, so a sweep landing on `4.999` cannot silently buy a fifth
    /// query.
    ///
    /// The two lead-time ends overlap in their declared ranges, so
    /// `PASS_LEAD_TIME_MIN > PASS_LEAD_TIME_MAX` is authorable. The ceiling
    /// is raised to the floor rather than trusted, so the candidate set stays
    /// ascending at every knob combination a sweep may try.
    #[must_use]
    pub fn of(tune: &Tuning) -> Self {
        let steps = tune.value(STEPS_KNOB);
        let steps = if steps >= 1.0 { steps as usize } else { 1 };
        let time_min = tune.value(TIME_MIN_KNOB);
        LeadKnobs {
            tolerance: tune.value(TOLERANCE_KNOB),
            min_speed: tune.value(MIN_SPEED_KNOB),
            time_min,
            time_max: tune.value(TIME_MAX_KNOB).max(time_min),
            steps: steps.min(MAX_CANDIDATES),
        }
    }
}

/// The fixed candidate lead times, ascending.
///
/// A fixed set rather than a convergence loop, exactly as the issue
/// specifies: it bounds the per-release cost at `steps` queries and it keeps
/// evaluation order stable, which is what makes the tie-break below mean
/// anything on a peer that resimulates.
#[must_use]
pub fn candidate_lead_times(knobs: &LeadKnobs) -> Vec<f64> {
    if knobs.steps <= 1 {
        return vec![knobs.time_min];
    }
    let span = knobs.time_max - knobs.time_min;
    let last = (knobs.steps - 1) as f64;
    (0..knobs.steps)
        .map(|i| knobs.time_min + span * (i as f64 / last))
        .collect()
}

/// An admissible lead: where to aim, and the measurements that justified it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeadSolution {
    /// The reception point to aim at.
    pub point: Vec2,
    /// The candidate lead time that won.
    pub lead_time: f64,
    /// Launch speed for a pass to `point`, from the registered curve.
    pub speed: f64,
    /// Ball travel time to `point`, from the prediction service.
    pub travel_time: f64,
    /// The receiver's own modelled arrival time at `point`.
    pub reach_time: f64,
}

/// Solve for the best admissible lead, or `None` for the unled fallback.
///
/// `capture` is the radius at which the receiver counts as having arrived —
/// the caller's possession distance, taken as a parameter rather than
/// restated here so the two cannot drift apart. `speed_scale` is whatever
/// the caller multiplies the curve speed by before launch (today: the
/// passer's species link bonus); the solver must model the ball that will
/// actually be kicked, not the one the curve alone describes.
///
/// Returns `None` — meaning "pass to where the receiver is now" — when the
/// receiver is barely moving, when no candidate is admissible, or when the
/// prediction service cannot answer inside its horizon or this tick's
/// budget. All three are the same instruction to the caller and deliberately
/// share one representation: an unled pass is a real pass, not a failure.
#[must_use]
pub fn solve(
    state: &MatchState,
    predictor: &mut BallPredictor,
    passer_pos: Vec2,
    receiver: &MatchPlayer,
    capture: f64,
    speed_scale: f64,
    tune: &Tuning,
) -> Option<LeadSolution> {
    let knobs = LeadKnobs::of(tune);
    let run = receiver.run_vel;
    if run.length() < knobs.min_speed {
        return None;
    }
    let body = ReachBody {
        pos: receiver.pos,
        k: locomotion::Kinematics {
            run_vel: receiver.run_vel,
            facing: receiver.facing,
        },
        base_speed: receiver.move_speed,
        stats: locomotion::stats(
            receiver.move_speed,
            receiver.strength,
            receiver.dribble,
            tune,
        ),
        sprinting: receiver.sprinting,
    };
    let dt = predictor.config().dt;

    let mut best: Option<LeadSolution> = None;
    for lead_time in candidate_lead_times(&knobs) {
        // Clamp the reception point to the pitch: a lead that runs a
        // receiver through the touchline is a bug in the point, which is why
        // `time_to_reach` does not clamp the body.
        let point = clamp_to_pitch(
            state,
            receiver.radius,
            receiver.pos.add(run.scale(lead_time)),
        );
        let distance = passer_pos.dist(point);
        let direction = point.sub(passer_pos).normalized();
        if direction.x == 0.0 && direction.y == 0.0 {
            continue;
        }
        let speed = passing::speed_for(distance, tune) * speed_scale;
        if speed <= 0.0 {
            continue;
        }
        let launch = BallFlight {
            pos: passer_pos,
            vel: direction.scale(speed),
            z: 0.0,
            vz: 0.0,
            spin: 0.0,
        };
        let Some(sample) = predictor.launch_after_distance(state, launch, distance) else {
            continue;
        };
        let budget = sample.time * knobs.tolerance;
        let Some(reach_time) = locomotion::time_to_reach(&body, point, capture, budget, dt, tune)
        else {
            continue;
        };
        // The best admissible candidate is the one furthest into the run:
        // leading further ahead is the entire point, and slack is not a
        // virtue on its own. Strictly greater, so an exact duplicate lead
        // time (a degenerate `time_min == time_max` set) keeps the earlier
        // entry rather than the later one.
        if best.is_none_or(|b| lead_time > b.lead_time) {
            best = Some(LeadSolution {
                point,
                lead_time,
                speed,
                travel_time: sample.time,
                reach_time,
            });
        }
    }
    best
}

fn clamp_to_pitch(state: &MatchState, radius: f64, pos: Vec2) -> Vec2 {
    Vec2::new(
        pos.x.clamp(radius, state.field.w - radius),
        pos.y.clamp(radius, state.field.h - radius),
    )
}
