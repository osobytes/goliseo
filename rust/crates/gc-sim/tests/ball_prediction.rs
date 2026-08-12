//! Tests for `gc_sim::ball_prediction`, the sim-side forward-prediction
//! service, and for the `gc_sim::ball_flight` extraction it is built on.
//!
//! ## What the accuracy test is actually for
//!
//! "The scratch world steps the same ball update function as the live sim"
//! is the design's whole argument, and it is exactly the kind of claim that
//! survives a refactor as a comment while stopping being true in the code.
//! So it is asserted the only way that cannot rot: predict a whole
//! trajectory from one live state, then step the *live simulation* through
//! those same ticks and require the two to agree **bit for bit** — not
//! within a tolerance, which a second implementation could also pass by
//! luck. A copied integrator would drift the first time either copy changed;
//! a `f64`-exact match every tick is only obtainable by running the same
//! code.
//!
//! The fixture parks all ten players on the far side of the pitch and asserts
//! per tick that the ball was never contacted (`owner` stays `None` and the
//! frame produces no events). That is not decoration: a contact would make
//! the live path legitimately diverge from a ball-and-ground-plane scratch
//! world, and the test would then be asserting the wrong thing quietly.

use gc_core::vec2::Vec2;
use gc_data::omp2_rollback_validation;
use gc_sim::ball_flight::{self, BallArena, BallFlight};
use gc_sim::ball_prediction::{
    self, BallAxis, BallEstimateSource, BallPlane, BallPredictionConfig, BallPredictor,
};
use gc_sim::fixed_clock;
use gc_sim::input_frame::{self, InputSample, InputSampleOptions};
use gc_sim::r#match::{self as sim_match, NewMatchOptions, StepInput};
use gc_sim::match_snapshot::{self, MatchInput, MatchSnapshot, MatchState, PitchSize};
use gc_sim::rollback_input_history::RollbackInputSource;
use gc_sim::rollback_session::{self, RollbackSession};
use gc_sim::tuning::Tuning;

const DT: f64 = fixed_clock::TICK_SECONDS;

// ---------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------

fn new_state(slot_mode: bool) -> MatchState {
    let home = gc_data::teams::get("nebula").expect("nebula is authored");
    let away = gc_data::teams::get("orion").expect("orion is authored");
    let ownership = if slot_mode {
        Some(sim_match::ownership_for_teams(home, away, None))
    } else {
        None
    };
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
        input_ownership: ownership,
    })
}

/// A loose ball nobody can reach: every player is parked along the bottom
/// touchline while the ball runs along the top one, airborne, so neither a
/// ground possession nor an aerial contact is geometrically available.
fn undisturbed_state() -> MatchState {
    let mut state = new_state(false);
    for (index, player) in state.players.iter_mut().enumerate() {
        let x = 60.0 + (index as f64) * 30.0;
        player.pos = Vec2::new(x, 500.0);
        player.anchor = Vec2::new(x, 500.0);
        player.vel = Vec2::new(0.0, 0.0);
        player.run_vel = Vec2::new(0.0, 0.0);
    }
    state.owner = None;
    state.ball = Vec2::new(220.0, 60.0);
    state.ball_vel = Vec2::new(520.0, -40.0);
    state.ball_z = 90.0;
    state.ball_vz = 120.0;
    state.ball_spin = 0.0;
    state.pickup_cd = 0.0;
    state
}

fn tuning() -> Tuning {
    Tuning::new()
}

fn step_live(state: &mut MatchState) {
    let tune = tuning();
    sim_match::step(
        state,
        DT,
        StepInput::Legacy(MatchInput::default()),
        None,
        &tune,
    );
}

fn pad_marks(state: &mut MatchState) {
    let n = state.players.len();
    state.marks.home.resize(n, None);
    state.marks.away.resize(n, None);
}

fn initial_snapshot() -> MatchSnapshot {
    let mut state = new_state(true);
    pad_marks(&mut state);
    match_snapshot::capture_owned(&state, None)
}

fn sources() -> [RollbackInputSource; 8] {
    [RollbackInputSource::Remote; 8]
}

fn sample(options: InputSampleOptions) -> InputSample {
    input_frame::new_sample(options).expect("valid sample")
}

/// A heavy, deterministic query mix: every query kind, every flavor, at a
/// spread of horizons. This is the "query-heavy consumer" the invalidation
/// and rollback cases need — the keeper and passing consumers that will
/// really issue these do not exist yet.
fn query_burst(predictor: &mut BallPredictor, state: &MatchState, salt: i64) {
    let horizon = predictor.config().max_horizon;
    for step in 0..6 {
        let time = horizon * (step as f64 + 1.0) / 6.0;
        let _ = predictor.position_at_time(state, time);
        let _ = predictor.estimate_position_at_time(state, time);
    }
    let _ = predictor.state_after_distance(
        state,
        40.0 * f64::from(u32::try_from(salt % 7).unwrap_or(0) + 1),
    );
    let _ = predictor.time_to_cross_plane(
        state,
        BallPlane {
            axis: BallAxis::X,
            coord: state.field.w / 2.0,
        },
    );
    let _ = predictor.time_to_cross_plane(
        state,
        BallPlane {
            axis: BallAxis::Y,
            coord: state.field.h / 2.0,
        },
    );
    let _ = predictor.time_to_height(state, 20.0);
    let point = Vec2::new(state.ball.x, state.ball.y);
    for player in &state.players {
        let _ = predictor.reachable_before_arrival(player, point, 0.5);
    }
}

// ---------------------------------------------------------------------
// Accuracy: the scratch world runs the live sim's own ball step
// ---------------------------------------------------------------------

#[test]
fn the_scratch_world_reproduces_the_live_sims_own_ball_path_bit_for_bit() {
    let mut state = undisturbed_state();
    let mut predictor = BallPredictor::default();
    predictor.begin_tick();

    let ticks = 40;
    let predicted: Vec<_> = (1..=ticks)
        .map(|tick| {
            predictor
                .position_at_time(&state, f64::from(tick) * DT)
                .expect("an undisturbed ball resolves inside the default horizon")
        })
        .collect();

    for tick in 1..=ticks {
        step_live(&mut state);
        assert!(
            state.owner.is_none() && state.events.is_empty(),
            "tick {tick}: the fixture's ball was contacted, so this case is no longer \
             comparing a free-flying ball against a ball-and-ground-plane scratch world"
        );
        let want = predicted[(tick - 1) as usize];
        assert_eq!(state.ball.x, want.pos.x, "tick {tick}: ball x");
        assert_eq!(state.ball.y, want.pos.y, "tick {tick}: ball y");
        assert_eq!(state.ball_vel.x, want.vel.x, "tick {tick}: ball vel x");
        assert_eq!(state.ball_vel.y, want.vel.y, "tick {tick}: ball vel y");
        assert_eq!(state.ball_z, want.z, "tick {tick}: ball z");
        assert_eq!(state.ball_vz, want.vz, "tick {tick}: ball vz");
    }
}

#[test]
fn interpolated_answers_between_sample_points_stay_close_to_the_stepped_path() {
    let state = undisturbed_state();
    let mut exact = BallPredictor::default();
    exact.begin_tick();
    let mut strided = BallPredictor::new(BallPredictionConfig {
        sample_stride: 4,
        ..BallPredictionConfig::default()
    });
    strided.begin_tick();

    for tick in 1..=40 {
        let time = f64::from(tick) * DT;
        let fine = exact.position_at_time(&state, time).expect("resolves");
        let coarse = strided.position_at_time(&state, time).expect("resolves");
        if tick % 4 == 0 {
            // A stored sample on both sides: identical scratch ticks, so
            // identical numbers.
            assert_eq!(coarse.pos.x, fine.pos.x, "tick {tick}: stored sample x");
            assert_eq!(coarse.pos.y, fine.pos.y, "tick {tick}: stored sample y");
            assert_eq!(coarse.z, fine.z, "tick {tick}: stored sample z");
        } else {
            let error = coarse.pos.dist(fine.pos);
            assert!(
                error < 2.0,
                "tick {tick}: interpolating across a four-tick stride drifted {error} px"
            );
        }
    }
    assert!(strided.sample_count() < exact.sample_count());
}

// ---------------------------------------------------------------------
// All four query kinds, and their explicit `none`
// ---------------------------------------------------------------------

#[test]
fn all_four_query_kinds_answer_a_resolvable_question() {
    let state = undisturbed_state();
    let mut predictor = BallPredictor::default();
    predictor.begin_tick();

    let at = predictor
        .position_at_time(&state, 0.25)
        .expect("a quarter second is inside the horizon");
    assert!((at.time - 0.25).abs() < 1e-9);
    assert!(at.pos.x > state.ball.x, "the ball is travelling +x");

    let after = predictor
        .state_after_distance(&state, 100.0)
        .expect("the ball covers 100 px inside the horizon");
    assert!(after.path >= 100.0);
    assert!(after.time > 0.0);

    let crossing = predictor
        .time_to_cross_plane(
            &state,
            BallPlane {
                axis: BallAxis::X,
                coord: 400.0,
            },
        )
        .expect("the ball crosses x = 400 inside the horizon");
    assert!(crossing > 0.0 && crossing < 2.0);
    let at_crossing = predictor
        .position_at_time(&state, crossing)
        .expect("resolves");
    assert!(
        (at_crossing.pos.x - 400.0).abs() < 6.0,
        "the reported crossing time should land on the plane"
    );

    let grounded = predictor
        .time_to_height(&state, 0.0)
        .expect("a ball thrown up comes down inside the horizon");
    assert!(grounded > 0.0);

    let reach = predictor.reachable_before_arrival(&state.players[0], state.ball, 10.0);
    assert!(
        reach.reachable,
        "ten seconds is plenty for any distance here"
    );
    assert!(reach.margin > 0.0);
    let tight = predictor.reachable_before_arrival(&state.players[0], state.ball, 0.01);
    assert!(!tight.reachable);
    assert!(tight.margin < 0.0);
    assert!((tight.arrival - reach.arrival).abs() < 1e-12);
}

#[test]
fn every_query_kind_returns_an_explicit_none_when_it_cannot_resolve() {
    let state = undisturbed_state();
    let mut predictor = BallPredictor::default();
    predictor.begin_tick();

    assert_eq!(predictor.position_at_time(&state, -1.0), None);
    assert_eq!(
        predictor.position_at_time(&state, predictor.config().max_horizon + 1.0),
        None,
        "beyond the horizon is an explicit none, never a clamped answer"
    );
    assert_eq!(predictor.state_after_distance(&state, -1.0), None);
    assert_eq!(
        predictor.state_after_distance(&state, 100_000.0),
        None,
        "the ball never travels a hundred metres of pitch inside two seconds"
    );
    assert_eq!(
        predictor.time_to_cross_plane(
            &state,
            BallPlane {
                axis: BallAxis::X,
                coord: -5_000.0,
            }
        ),
        None
    );
    assert_eq!(
        predictor.time_to_height(&state, 5_000.0),
        None,
        "the cage ceiling is 170 px; nothing reaches 5,000"
    );
}

// ---------------------------------------------------------------------
// Budget, fallback, and the knob-moves-metric contract
// ---------------------------------------------------------------------

#[test]
fn a_spent_budget_falls_back_for_tolerant_callers_and_refuses_authoritative_ones() {
    let state = undisturbed_state();
    let mut predictor = BallPredictor::new(BallPredictionConfig {
        step_budget: 0,
        ..BallPredictionConfig::default()
    });
    predictor.begin_tick();

    assert_eq!(
        predictor.position_at_time(&state, 0.5),
        None,
        "an authoritative query must never be answered by the fallback"
    );
    assert_eq!(predictor.state_after_distance(&state, 200.0), None);
    assert_eq!(
        predictor.time_to_cross_plane(
            &state,
            BallPlane {
                axis: BallAxis::X,
                coord: 400.0
            }
        ),
        None
    );
    assert_eq!(predictor.time_to_height(&state, 0.0), None);

    let estimate = predictor.estimate_position_at_time(&state, 0.5);
    assert_eq!(
        estimate.source,
        BallEstimateSource::Fallback,
        "a tolerant caller degrades rather than stalling"
    );
    assert!((estimate.time - 0.5).abs() < 1e-12);

    let telemetry = predictor.telemetry();
    assert!(telemetry.budget_exhaustions >= 5);
    assert_eq!(telemetry.scratch_steps, 0);
    assert!(telemetry.fallback_ratio() > 0.0);
    assert!(ball_prediction::marker(&telemetry).contains("budget_exhaustions="));
}

#[test]
fn the_budget_is_shared_across_all_callers_in_one_tick_and_refills_on_the_next() {
    let state = undisturbed_state();
    let mut predictor = BallPredictor::new(BallPredictionConfig {
        step_budget: 10,
        ..BallPredictionConfig::default()
    });
    predictor.begin_tick();

    // One caller spends the whole budget...
    let first = predictor.position_at_time(&state, 10.0 * DT);
    assert!(first.is_some());
    assert_eq!(predictor.budget_remaining(), 0);
    // ...so the next caller, in the same tick, is refused rather than
    // allowed its own fresh allowance.
    assert_eq!(predictor.position_at_time(&state, 20.0 * DT), None);
    assert_eq!(predictor.telemetry().budget_exhaustions, 1);

    predictor.begin_tick();
    assert_eq!(predictor.budget_remaining(), 10);
    assert!(predictor.position_at_time(&state, 10.0 * DT).is_some());
}

#[test]
fn lowering_the_step_budget_raises_the_fallback_ratio() {
    fn run(step_budget: i64) -> f64 {
        let mut state = undisturbed_state();
        let mut predictor = BallPredictor::new(BallPredictionConfig {
            step_budget,
            ..BallPredictionConfig::default()
        });
        for tick in 0..60 {
            predictor.begin_tick();
            query_burst(&mut predictor, &state, tick);
            step_live(&mut state);
        }
        predictor.telemetry().fallback_ratio()
    }

    let generous = run(ball_prediction::STEP_BUDGET_DEFAULT);
    let starved = run(4);
    assert_eq!(
        generous, 0.0,
        "the default budget covers a full horizon rebuild per tick"
    );
    assert!(
        starved > generous,
        "starving the budget must show up as fallback answers: {starved} vs {generous}"
    );
}

#[test]
fn the_default_budget_covers_one_full_horizon_rebuild_and_a_bounded_worst_frame() {
    // The written justification in `STEP_BUDGET_DEFAULT`'s doc comment
    // rests on two numbers. Pin both, so the prose cannot drift away from
    // the code or from the authored rollback budget it cites.
    let config = BallPredictionConfig::default();
    let horizon_ticks = (config.max_horizon / config.dt).round() as i64;
    assert_eq!(horizon_ticks, ball_prediction::STEP_BUDGET_DEFAULT);

    let retained_window = omp2_rollback_validation::DATA.budgets.snapshot_count;
    assert_eq!(
        retained_window, 31,
        "the budget justification is written against a 31-boundary retained window"
    );
    assert_eq!(
        ball_prediction::STEP_BUDGET_DEFAULT * retained_window,
        3_720
    );
}

// ---------------------------------------------------------------------
// Generation-based invalidation
// ---------------------------------------------------------------------

#[test]
fn a_mid_tick_ball_mutation_rebuilds_and_never_serves_pre_mutation_samples() {
    let mut state = undisturbed_state();
    let mut predictor = BallPredictor::default();
    predictor.begin_tick();

    let before = predictor
        .position_at_time(&state, 0.5)
        .expect("resolves")
        .pos;
    let generation_before = predictor.generation();

    // A kick resolved later in the same tick's ordering.
    state.ball_vel = Vec2::new(-600.0, 300.0);

    let after = predictor
        .position_at_time(&state, 0.5)
        .expect("resolves")
        .pos;
    assert!(
        predictor.generation() > generation_before,
        "an external mutation must bump the ball generation"
    );
    assert!(
        after.dist(before) > 100.0,
        "the post-kick answer must come from the post-kick ball, not the cached path"
    );

    // And it agrees with a predictor that only ever saw the new ball.
    let mut fresh = BallPredictor::default();
    fresh.begin_tick();
    let independent = fresh.position_at_time(&state, 0.5).expect("resolves");
    assert_eq!(after.x, independent.pos.x);
    assert_eq!(after.y, independent.pos.y);
}

#[test]
fn every_enumerated_external_ball_mutation_path_bumps_the_generation() {
    // One case per shape of external mutation the live sim performs.
    // Named for the sim path each stands in for, so a reader can check the
    // list against `crate::r#match` rather than trust it.
    type Mutation = fn(&mut MatchState);
    let mutations: Vec<(&str, Mutation)> = vec![
        ("free-flight step (position)", |s| {
            s.ball = s.ball.add(Vec2::new(1.0, 0.0));
        }),
        ("kick / shot release (velocity)", |s| {
            s.ball_vel = Vec2::new(-400.0, 25.0);
        }),
        ("lob / punt (height)", |s| s.ball_z += 15.0),
        ("bounce, save parry (vertical velocity)", |s| {
            s.ball_vz = -s.ball_vz - 10.0;
        }),
        ("curved strike (spin)", |s| s.ball_spin += 90.0),
        ("possession pickup (owner)", |s| s.owner = Some(3)),
        ("restart placement (teleport)", |s| {
            s.ball = Vec2::new(480.0, 270.0);
            s.ball_vel = Vec2::new(0.0, 0.0);
        }),
    ];

    for (label, mutate) in mutations {
        let mut state = undisturbed_state();
        let mut predictor = BallPredictor::default();
        predictor.begin_tick();
        let _ = predictor.position_at_time(&state, 0.5);
        let generation = predictor.generation();
        let rebuilds = predictor.telemetry().rebuilds;

        mutate(&mut state);
        let _ = predictor.position_at_time(&state, 0.5);

        assert!(
            predictor.generation() > generation,
            "{label}: the generation did not bump"
        );
        assert!(
            predictor.telemetry().rebuilds > rebuilds,
            "{label}: the buffer was not rebuilt"
        );
    }
}

/// The enumerated list above is an inventory, and an inventory can be
/// incomplete. This case takes the coverage question away from the list:
/// it plays a real match and requires that *whenever* the live ball's state
/// changed between two ticks, the predictor's generation changed too — over
/// whatever mutation paths that match actually exercised (touches, kicks,
/// blocks, saves, restarts), including ones added after this test was
/// written.
#[test]
fn a_real_match_never_changes_the_ball_without_bumping_the_generation() {
    let mut state = new_state(false);
    let mut predictor = BallPredictor::default();
    let mut observed_changes = 0;
    let mut previous: Option<(BallFlight, Option<i64>)> = None;

    for tick in 0..900 {
        predictor.begin_tick();
        let generation = predictor.generation();
        let _ = predictor.position_at_time(&state, 0.25);
        let now = (BallFlight::of(&state), state.owner);
        if let Some(before) = previous
            && before != now
        {
            observed_changes += 1;
            assert!(
                predictor.generation() > generation,
                "tick {tick}: the live ball changed and the generation did not"
            );
        }
        previous = Some(now);
        step_live(&mut state);
    }
    assert!(
        observed_changes > 200,
        "the fixture must actually move the ball around: only {observed_changes} changes seen"
    );
}

// ---------------------------------------------------------------------
// Determinism: no state, no snapshot, no hash, no RNG
// ---------------------------------------------------------------------

#[test]
fn queries_leave_the_simulation_state_and_its_hash_untouched() {
    let state = undisturbed_state();
    let before = state.clone();
    let before_hash = match_snapshot::hash_canonical(&match_snapshot::capture_owned(&state, None));

    let mut predictor = BallPredictor::default();
    for tick in 0..25 {
        predictor.begin_tick();
        query_burst(&mut predictor, &state, tick);
    }

    assert_eq!(
        state.rng, before.rng,
        "the scratch world drew from the seeded stream"
    );
    assert!(state == before, "a query mutated simulation state");
    assert_eq!(
        match_snapshot::hash_canonical(&match_snapshot::capture_owned(&state, None)),
        before_hash
    );
    assert!(
        predictor.telemetry().scratch_steps > 0,
        "nothing was predicted"
    );
    // Nothing of the service is in the snapshot's shape, so nothing of it
    // can be in the hash: the buffer is excluded structurally rather than
    // by a maintained exclusion list.
    let _: &MatchState = &state;
}

#[test]
fn two_peers_hash_identically_regardless_of_query_history() {
    let mut queried = undisturbed_state();
    let mut quiet = undisturbed_state();
    let mut predictor = BallPredictor::default();
    let mut hashes_match = 0;

    for tick in 0..180 {
        predictor.begin_tick();
        query_burst(&mut predictor, &queried, tick);
        if tick % 3 == 0 {
            // A second, differently configured consumer on the same peer:
            // a different query history again, still no state effect.
            let mut coarse = BallPredictor::new(BallPredictionConfig {
                step_budget: 7,
                sample_stride: 3,
                ..BallPredictionConfig::default()
            });
            coarse.begin_tick();
            query_burst(&mut coarse, &queried, tick);
        }
        step_live(&mut queried);
        step_live(&mut quiet);
        let a = match_snapshot::hash_canonical(&match_snapshot::capture_owned(&queried, None));
        let b = match_snapshot::hash_canonical(&match_snapshot::capture_owned(&quiet, None));
        assert_eq!(a, b, "tick {tick}: query history changed the state hash");
        hashes_match += 1;
    }
    assert_eq!(hashes_match, 180);
}

#[test]
fn the_scratch_step_is_the_shared_free_flight_function_and_draws_no_randomness() {
    // `ball_flight::step`'s signature is the structural proof: it is handed
    // a ball and an arena, and has no access to `MatchState::rng`. This
    // pins the behavioral half — repeated identical runs, and agreement
    // with what the predictor reports.
    let state = undisturbed_state();
    let arena = BallArena::of(&state);
    let mut a = BallFlight::of(&state);
    let mut b = BallFlight::of(&state);
    for _ in 0..90 {
        ball_flight::step(&mut a, &arena, DT);
        ball_flight::step(&mut b, &arena, DT);
    }
    assert_eq!(a, b);

    let mut predictor = BallPredictor::default();
    predictor.begin_tick();
    let sampled = predictor
        .position_at_time(&state, 90.0 * DT)
        .expect("resolves");
    assert_eq!(sampled.pos.x, a.pos.x);
    assert_eq!(sampled.pos.y, a.pos.y);
    assert_eq!(sampled.z, a.z);
}

// ---------------------------------------------------------------------
// Rollback
// ---------------------------------------------------------------------

fn drive_rollback_peer(mut queries: Option<&mut BallPredictor>) -> (Vec<String>, i64, i64) {
    let initial = initial_snapshot();
    let mut session: RollbackSession = rollback_session::new(&initial, sources(), None, None);
    let mut hashes = Vec::new();

    let nudged = sample(InputSampleOptions {
        move_x: Some(120),
        move_y: Some(-90),
        held: Some(input_frame::HELD_SPRINT),
        edges: Some(input_frame::EDGE_DASH),
    });

    for tick in 0..90 {
        rollback_session::step(&mut session).expect("step succeeds");
        if let Some(predictor) = queries.as_deref_mut() {
            predictor.begin_tick();
            query_burst(predictor, rollback_session::state(&session), tick);
        }
        // Late authority for a tick already predicted with neutral input:
        // this is what forces a real rollback and resimulation.
        if tick >= 4 && tick % 5 == 0 {
            let corrected = tick - 4;
            for slot in 1..=input_frame::SLOT_COUNT {
                rollback_session::add_authoritative(&mut session, corrected, slot, nudged)
                    .expect("authoritative row accepted");
            }
            let result = rollback_session::reconcile(&mut session, false);
            if result.changed
                && let Some(predictor) = queries.as_deref_mut()
            {
                // A resimulated tick re-issues its queries, against a
                // state restored from a retained boundary.
                predictor.begin_tick();
                query_burst(predictor, rollback_session::state(&session), tick);
            }
        }
        hashes.push(match_snapshot::hash_canonical(
            &rollback_session::current_snapshot(&session),
        ));
    }
    let diagnostics = rollback_session::diagnostics(&session);
    (
        hashes,
        diagnostics.rollback_count,
        diagnostics.resimulated_ticks,
    )
}

#[test]
fn a_query_heavy_rollback_peer_hashes_identically_to_a_query_free_peer() {
    let mut predictor = BallPredictor::default();
    let (queried, queried_rollbacks, queried_resims) = drive_rollback_peer(Some(&mut predictor));
    let (quiet, quiet_rollbacks, quiet_resims) = drive_rollback_peer(None);

    assert!(
        queried_rollbacks > 0 && queried_resims > 0,
        "the fixture must actually roll back and resimulate: {queried_rollbacks} rollbacks, \
         {queried_resims} resimulated ticks"
    );
    assert_eq!(queried_rollbacks, quiet_rollbacks);
    assert_eq!(queried_resims, quiet_resims);
    assert_eq!(
        queried, quiet,
        "issuing queries changed a rollback peer's boundary hashes"
    );

    let telemetry = predictor.telemetry();
    assert!(telemetry.answers > 0);
    // Harness-visible: the budget-exhaustion counter and the fallback ratio
    // are reported by any run that exercises the service.
    println!("{}", ball_prediction::marker(&telemetry));
}

#[test]
fn a_restored_boundary_rebuilds_rather_than_serving_the_mispredicted_timeline() {
    let initial = initial_snapshot();
    let mut session = rollback_session::new(&initial, sources(), None, None);
    let mut predictor = BallPredictor::default();

    let nudged = sample(InputSampleOptions {
        move_x: Some(127),
        held: Some(input_frame::HELD_SPRINT),
        ..Default::default()
    });
    for _ in 0..8 {
        rollback_session::step(&mut session).expect("step succeeds");
    }
    predictor.begin_tick();
    let mispredicted = predictor
        .position_at_time(rollback_session::state(&session), 0.5)
        .expect("resolves");
    let generation = predictor.generation();

    for slot in 1..=input_frame::SLOT_COUNT {
        rollback_session::add_authoritative(&mut session, 0, slot, nudged)
            .expect("authoritative row accepted");
    }
    let result = rollback_session::reconcile(&mut session, false);
    assert!(result.changed, "the correction must actually roll back");

    predictor.begin_tick();
    let corrected = predictor
        .position_at_time(rollback_session::state(&session), 0.5)
        .expect("resolves");
    assert!(
        predictor.generation() > generation,
        "the restored ball must be seen as a new generation"
    );
    assert!(
        corrected != mispredicted,
        "the corrected timeline was served the mispredicted timeline's samples"
    );
}
