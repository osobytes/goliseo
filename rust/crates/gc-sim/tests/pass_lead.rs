//! Tests for `gc_sim::pass_lead` — the lead solver — and for the locomotion
//! arrival model (`gc_sim::locomotion::time_to_reach`) its admissibility test
//! is built on.
//!
//! The load-bearing case here is
//! [`the_flat_cap_model_over_promises_and_the_locomotion_model_does_not`]:
//! the issue that specified this work asks for `required speed <= cap *
//! tolerance`, and that test is the measurement showing why that test would
//! have shipped a bug. It is not a demonstration that the new model is
//! *nicer* — it is a demonstration that the old one says yes to a point the
//! body provably does not reach.

use gc_core::vec2::Vec2;
use gc_sim::fixed_clock;
use gc_sim::locomotion::{self, Kinematics, ReachBody};
use gc_sim::r#match::{self as sim_match, NewMatchOptions};
use gc_sim::match_snapshot::{MatchPlayer, MatchState, PitchSize};
use gc_sim::pass_lead::{self, LeadKnobs};
use gc_sim::tuning::Tuning;

const DT: f64 = fixed_clock::TICK_SECONDS;
/// The possession radius `r#match` passes as the arrival capture distance.
const CAPTURE: f64 = 22.0;

fn new_state() -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula is authored");
    let away = gc_data::teams::get("orion").expect("orion is authored");
    sim_match::new(NewMatchOptions {
        home,
        away,
        field: PitchSize { w: 960.0, h: 540.0 },
        home_formation: None,
        tactic: None,
        away_tactic: None,
        duration: Some(120.0),
        max_goals: Some(99),
        seed: Some(733.0),
        players_by_id: None,
        species_by_id: None,
        showcase_players_by_id: None,
        human_controlled: None,
        input_ownership: None,
    })
}

/// A receiver at `pos` running at `vel`, taken from the real fixture so the
/// stats and radius are an authored player's rather than invented.
fn runner(state: &MatchState, pos: Vec2, vel: Vec2) -> MatchPlayer {
    let mut p = state
        .players
        .iter()
        .find(|p| !p.is_keeper)
        .expect("the fixture has outfielders")
        .clone();
    p.pos = pos;
    p.vel = vel;
    p.run_vel = vel;
    p.facing = if vel.length() > 0.0 {
        vel.normalized()
    } else {
        Vec2::new(1.0, 0.0)
    };
    p.sprinting = false;
    p
}

fn body_of(p: &MatchPlayer, tune: &Tuning) -> ReachBody {
    ReachBody {
        pos: p.pos,
        k: Kinematics {
            run_vel: p.run_vel,
            facing: p.facing,
        },
        base_speed: p.move_speed,
        stats: locomotion::stats(p.move_speed, p.strength, p.dribble, tune),
        sprinting: p.sprinting,
    }
}

// ---------------------------------------------------------------------
// the fixed candidate set
// ---------------------------------------------------------------------

#[test]
fn the_candidate_set_is_fixed_ascending_and_bounded() {
    let tune = Tuning::new();
    let knobs = LeadKnobs::of(&tune);
    let times = pass_lead::candidate_lead_times(&knobs);
    assert_eq!(times.len(), knobs.steps);
    assert!(knobs.steps <= pass_lead::MAX_CANDIDATES);
    for pair in times.windows(2) {
        assert!(pair[1] > pair[0], "the set must be ascending: {times:?}");
    }
    assert_eq!(times.first().copied(), Some(knobs.time_min));
    assert_eq!(times.last().copied(), Some(knobs.time_max));
    // Fixed, not a convergence loop: the same knobs give the same set.
    assert_eq!(times, pass_lead::candidate_lead_times(&knobs));
}

#[test]
fn the_candidate_count_is_capped_below_the_step_knob_fence() {
    let mut tune = Tuning::new();
    tune.set(pass_lead::STEPS_KNOB, 999.0);
    assert_eq!(
        LeadKnobs::of(&tune).steps,
        pass_lead::MAX_CANDIDATES,
        "the per-release query burst must stay bounded whatever a sweep authors"
    );
    tune.set(pass_lead::STEPS_KNOB, 0.0);
    assert_eq!(LeadKnobs::of(&tune).steps, 1, "and never zero");
    tune.set(pass_lead::STEPS_KNOB, 4.999);
    assert_eq!(
        LeadKnobs::of(&tune).steps,
        4,
        "truncation, not rounding: a sweep must not buy a fifth query at 4.999"
    );
}

#[test]
fn an_inverted_lead_time_range_still_produces_an_ascending_set() {
    let mut tune = Tuning::new();
    tune.set(pass_lead::TIME_MIN_KNOB, 0.4);
    tune.set(pass_lead::TIME_MAX_KNOB, 0.3);
    let times = pass_lead::candidate_lead_times(&LeadKnobs::of(&tune));
    for pair in times.windows(2) {
        assert!(pair[1] >= pair[0], "{times:?}");
    }
}

// ---------------------------------------------------------------------
// the arrival model
// ---------------------------------------------------------------------

#[test]
fn a_body_already_on_the_point_arrives_at_zero() {
    let tune = Tuning::new();
    let state = new_state();
    let p = runner(&state, Vec2::new(400.0, 270.0), Vec2::new(0.0, 0.0));
    assert_eq!(
        locomotion::time_to_reach(&body_of(&p, &tune), p.pos, CAPTURE, 1.0, DT, &tune),
        Some(0.0)
    );
}

#[test]
fn the_arrival_model_returns_none_when_the_horizon_is_too_short() {
    let tune = Tuning::new();
    let state = new_state();
    let p = runner(&state, Vec2::new(100.0, 270.0), Vec2::new(0.0, 0.0));
    let far = Vec2::new(800.0, 270.0);
    assert_eq!(
        locomotion::time_to_reach(&body_of(&p, &tune), far, CAPTURE, 0.2, DT, &tune),
        None
    );
    assert!(
        locomotion::time_to_reach(&body_of(&p, &tune), far, CAPTURE, 5.0, DT, &tune).is_some(),
        "and yes with enough time, or the case above proves nothing"
    );
}

/// Momentum reaches the model: a body already running at the point beats a
/// body starting from rest, and a body running *away* from it is slowest of
/// the three because it has to brake through zero first.
///
/// A flat speed cap cannot tell these three apart at all — they have the same
/// `move_speed` and the same distance.
#[test]
fn the_arrival_model_is_sensitive_to_the_bodys_current_velocity() {
    let tune = Tuning::new();
    let state = new_state();
    let start = Vec2::new(300.0, 270.0);
    let target = Vec2::new(600.0, 270.0);
    let speed = state.players[1].move_speed;

    let toward = runner(&state, start, Vec2::new(speed, 0.0));
    let rest = runner(&state, start, Vec2::new(0.0, 0.0));
    let away = runner(&state, start, Vec2::new(-speed, 0.0));

    let t_toward =
        locomotion::time_to_reach(&body_of(&toward, &tune), target, CAPTURE, 6.0, DT, &tune)
            .expect("a body already running at the point arrives");
    let t_rest = locomotion::time_to_reach(&body_of(&rest, &tune), target, CAPTURE, 6.0, DT, &tune)
        .expect("a body from rest arrives eventually");
    let t_away = locomotion::time_to_reach(&body_of(&away, &tune), target, CAPTURE, 6.0, DT, &tune)
        .expect("a body running away arrives eventually");

    assert!(
        t_toward < t_rest,
        "momentum toward the point must help: {t_toward} vs {t_rest}"
    );
    assert!(
        t_rest < t_away,
        "a reversal must cost more than a standing start: {t_rest} vs {t_away}"
    );
}

/// **The measurement that replaced the issue's step 4, against the shipped
/// flat-cap model itself.**
///
/// The issue specifies `required speed <= receiver's cap * lead_tolerance`.
/// That is exactly what `BallPredictor::reachable_before_arrival` computes —
/// straight line, top speed from a standing start, no acceleration profile —
/// so this compares the two models directly rather than against a hand-rolled
/// restatement of one of them.
///
/// Ten of the twenty-five (angle, distance, window) cases swept while writing
/// this test have the flat model saying **yes** and the body, stepped through
/// the real locomotion primitive, **not arriving**. Four are pinned here, one
/// per window length, including a purely lateral redirect at 90 degrees —
/// the over-promise is not a reversal-only curiosity.
///
/// This is the whole justification for #491's arrival-model decision,
/// standing as a test so it cannot quietly stop being true. It is not a claim
/// that the new model is nicer: it is a demonstration that the old one admits
/// points the body provably does not reach.
#[test]
fn the_flat_cap_model_over_promises_where_the_locomotion_model_refuses() {
    let tune = Tuning::new();
    let state = new_state();
    let predictor = pass_lead::release_predictor();
    let start = Vec2::new(480.0, 270.0);
    let speed = state.players[1].move_speed;
    // Running due east at full pace, as an off-ball receiver in a run is.
    let receiver = runner(&state, start, Vec2::new(speed, 0.0));
    let body = body_of(&receiver, &tune);

    // (degrees off the current run, px away, ball travel window in seconds)
    let cases = [
        (180.0_f64, 60.0, 0.4),
        (180.0, 140.0, 1.0),
        (135.0, 100.0, 0.6),
        (90.0, 180.0, 1.2),
    ];
    for (degrees, distance, t_ball) in cases {
        let radians = degrees.to_radians();
        let target = start.add(Vec2::new(radians.cos(), radians.sin()).scale(distance));
        let flat = predictor.reachable_before_arrival(&receiver, target, t_ball);
        assert!(
            flat.reachable,
            "the flat-cap model must ADMIT {degrees} deg / {distance} px / {t_ball} s, or \
             the comparison is vacuous: it arrives at {:.3} s",
            flat.arrival
        );
        assert_eq!(
            locomotion::time_to_reach(&body, target, CAPTURE, t_ball, DT, &tune),
            None,
            "the body does not reach {degrees} deg / {distance} px inside {t_ball} s, but \
             the flat cap promised it at {:.3} s",
            flat.arrival
        );
    }

    // And the locomotion model is not simply refusing everything: the same
    // body reaches every one of those points given enough time.
    for (degrees, distance, _) in cases {
        let radians = degrees.to_radians();
        let target = start.add(Vec2::new(radians.cos(), radians.sin()).scale(distance));
        assert!(
            locomotion::time_to_reach(&body, target, CAPTURE, 6.0, DT, &tune).is_some(),
            "{degrees} deg / {distance} px is unreachable at ANY horizon, which would make \
             the refusal above meaningless"
        );
    }
}

// ---------------------------------------------------------------------
// the solver
// ---------------------------------------------------------------------

#[test]
fn a_stationary_receiver_is_not_led() {
    let tune = Tuning::new();
    let state = new_state();
    let mut predictor = pass_lead::release_predictor();
    let receiver = runner(&state, Vec2::new(500.0, 270.0), Vec2::new(0.0, 0.0));
    assert_eq!(
        pass_lead::solve(
            &state,
            &mut predictor,
            Vec2::new(300.0, 270.0),
            &receiver,
            CAPTURE,
            1.0,
            &tune,
        ),
        None,
        "the speed floor means a standing receiver gets the ball to their feet"
    );
    assert_eq!(
        predictor.telemetry().answers,
        0,
        "and the release burst is not even issued: no candidate is evaluated"
    );
}

#[test]
fn a_running_receiver_is_led_ahead_of_their_feet() {
    let tune = Tuning::new();
    let state = new_state();
    let mut predictor = pass_lead::release_predictor();
    let receiver = runner(&state, Vec2::new(500.0, 270.0), Vec2::new(0.0, 240.0));
    let solution = pass_lead::solve(
        &state,
        &mut predictor,
        Vec2::new(300.0, 270.0),
        &receiver,
        CAPTURE,
        1.0,
        &tune,
    )
    .expect("a receiver running across the pass lane is exactly the lead case");
    assert!(
        solution.point.y > receiver.pos.y,
        "the reception point must be ahead along the run: {:?}",
        solution.point
    );
    assert!(solution.lead_time > 0.0);
    assert!(
        solution.reach_time <= solution.travel_time * LeadKnobs::of(&tune).tolerance,
        "the winning candidate must satisfy the admissibility test it was chosen by"
    );
}

/// The per-release query burst IS the candidate count, which is the budget
/// claim the issue asks to be confirmed.
#[test]
fn the_release_burst_is_exactly_the_candidate_count() {
    let tune = Tuning::new();
    let state = new_state();
    let receiver = runner(&state, Vec2::new(500.0, 270.0), Vec2::new(0.0, 240.0));
    let expected = LeadKnobs::of(&tune).steps as i64;
    let mut predictor = pass_lead::release_predictor();
    let _ = pass_lead::solve(
        &state,
        &mut predictor,
        Vec2::new(300.0, 270.0),
        &receiver,
        CAPTURE,
        1.0,
        &tune,
    );
    assert_eq!(
        predictor.telemetry().answers,
        expected,
        "one prediction query per candidate lead time, and not one more"
    );
    assert_eq!(
        predictor.telemetry().budget_exhaustions,
        0,
        "a whole release burst must fit inside one tick's default step budget"
    );
    // And it moves with the knob rather than being a coincidence of 5.
    let mut fewer = Tuning::new();
    fewer.set(pass_lead::STEPS_KNOB, 3.0);
    let mut predictor = pass_lead::release_predictor();
    let _ = pass_lead::solve(
        &state,
        &mut predictor,
        Vec2::new(300.0, 270.0),
        &receiver,
        CAPTURE,
        1.0,
        &fewer,
    );
    assert_eq!(predictor.telemetry().answers, 3);
}

#[test]
fn tightening_the_tolerance_shortens_or_removes_the_lead() {
    let state = new_state();
    let receiver = runner(&state, Vec2::new(500.0, 270.0), Vec2::new(0.0, 260.0));
    let passer = Vec2::new(280.0, 270.0);

    let mut solved = Vec::new();
    for tolerance in [1.4_f64, 1.0, 0.7, 0.5] {
        let mut tune = Tuning::new();
        tune.set(pass_lead::TOLERANCE_KNOB, tolerance);
        let mut predictor = pass_lead::release_predictor();
        let lead = pass_lead::solve(
            &state,
            &mut predictor,
            passer,
            &receiver,
            CAPTURE,
            1.0,
            &tune,
        );
        solved.push(lead.map_or(0.0, |s| s.lead_time));
    }
    for pair in solved.windows(2) {
        assert!(
            pair[1] <= pair[0],
            "demanding more slack must never lengthen the lead: {solved:?}"
        );
    }
    assert!(
        solved[0] > solved[3],
        "the tolerance knob must actually reach the solution: {solved:?}"
    );
}

#[test]
fn the_solver_is_deterministic_and_draws_no_rng() {
    let tune = Tuning::new();
    let state = new_state();
    let receiver = runner(&state, Vec2::new(520.0, 300.0), Vec2::new(-90.0, 200.0));
    let passer = Vec2::new(260.0, 240.0);
    let mut first_predictor = pass_lead::release_predictor();
    let first = pass_lead::solve(
        &state,
        &mut first_predictor,
        passer,
        &receiver,
        CAPTURE,
        1.0,
        &tune,
    );
    let before = state.rng;
    let mut second_predictor = pass_lead::release_predictor();
    let second = pass_lead::solve(
        &state,
        &mut second_predictor,
        passer,
        &receiver,
        CAPTURE,
        1.0,
        &tune,
    );
    assert_eq!(first, second, "same inputs, bit-identical solution");
    assert_eq!(
        state.rng, before,
        "the solver must not consume the rng stream"
    );
}

/// The reception point never leaves the pitch, however fast the receiver is
/// running at the touchline — otherwise the ball is led out of play.
#[test]
fn a_lead_point_is_clamped_to_the_pitch() {
    let tune = Tuning::new();
    let state = new_state();
    let mut predictor = pass_lead::release_predictor();
    let receiver = runner(&state, Vec2::new(500.0, 500.0), Vec2::new(0.0, 900.0));
    let solution = pass_lead::solve(
        &state,
        &mut predictor,
        Vec2::new(300.0, 300.0),
        &receiver,
        CAPTURE,
        1.0,
        &tune,
    );
    if let Some(solution) = solution {
        assert!(
            solution.point.y <= state.field.h,
            "led off the pitch: {:?}",
            solution.point
        );
    }
}
