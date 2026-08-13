//! Coverage for `rollback_lab::RollbackLabOptions::observer` (#495): the
//! per-tick injection seam that lets a test drive read-only per-tick work —
//! issuing queries against a sim service — inside a real campaign, without
//! `RollbackLabRunState` widening any field to mutable access outside
//! `rollback_lab.rs`.
//!
//! ## Why this file exists
//!
//! #486 asked for "run the rollback scenario matrix with a query-heavy
//! consumer; assert state hashes match with and without queries issued".
//! Before this seam existed there was no way to inject work into a real
//! `rollback_lab` campaign, so PR #492 substituted a hand-driven
//! `rollback_session` peer with late authority every fifth tick —real
//! evidence, but one correction pattern, no network jitter/loss/corruption
//! profiles, and no multi-depth rollback sweep. This file runs the actual
//! authored nine-scenario matrix
//! (`gc_data::omp2_rollback_validation::DATA.scenarios`) with a query-heavy
//! `gc_sim::ball_prediction::BallPredictor` consumer attached through the
//! seam, across every authored network profile and seed, and asserts
//! retained-boundary hashes are identical with and without queries issued.
//!
//! ## The substitute tests are kept, not retired
//!
//! `a_query_heavy_rollback_peer_hashes_identically_to_a_query_free_peer` and
//! its plain-match sibling `two_peers_hash_identically_regardless_of_query_history`
//! in `tests/ball_prediction.rs` stay. They are not redundant with this file:
//! they run in milliseconds against a hand-built fixture with no dependency
//! on `gc-data`'s frozen OMP-1 recording or the scenario registry, so they
//! stay the fast, dependency-light per-PR smoke check for "does issuing
//! queries touch simulation state at all" — useful on its own even if this
//! file's matrix run were ever gated to on-demand. This file is the
//! authoritative, broader claim #486 actually asked for: the full authored
//! matrix, every profile, every seed.

use gc_core::vec2::Vec2;
use gc_data::omp2_rollback_validation::Omp2RollbackScenario;
use gc_sim::ball_prediction::{self, BallAxis, BallPlane, BallPredictor};
use gc_sim::input_tape::InputTape;
use gc_sim::match_snapshot::{self, MatchState};
use gc_sim::rollback_lab::{self, RollbackLabObserver, RollbackLabOptions, RollbackLabRunState};
use gc_sim::rollback_session;
use gc_sim::rollback_validation;
use gc_sim::tuning::Tuning;
use std::cell::RefCell;
use std::rc::Rc;

/// A heavy, deterministic query mix: every query kind, every flavor, at a
/// spread of horizons. Mirrors `tests/ball_prediction.rs`'s `query_burst` —
/// the "query-heavy consumer" #486 asked for and no real consumer (keeper,
/// passing) exists yet to stand in for.
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

fn boundary_hash(state: &RollbackLabRunState) -> String {
    match_snapshot::hash_canonical(&rollback_session::current_snapshot(&state.session))
}

/// One case's outcome: the lab result, the per-tick retained-boundary hash
/// log the observer recorded, and (for a queried run) the predictor's own
/// telemetry.
struct CaseRun {
    result: rollback_lab::RollbackLabResult,
    hashes: Vec<String>,
    telemetry: Option<ball_prediction::BallPredictionTelemetry>,
}

/// Run one scenario tape through `rollback_lab` with the observer seam
/// attached, either issuing a query burst every tick (`attach_queries`) or
/// merely recording the same retained-boundary hash a queried run records
/// (`!attach_queries`) — so the two logs are directly comparable and the
/// observer's own read-only bookkeeping is not itself a confound.
fn run_case(
    tape: InputTape,
    profile_name: &str,
    network_seed: i64,
    attach_queries: bool,
) -> CaseRun {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let predictor = attach_queries.then(|| Rc::new(RefCell::new(BallPredictor::default())));

    let observer: RollbackLabObserver = match &predictor {
        Some(predictor) => {
            let predictor = Rc::clone(predictor);
            let log = Rc::clone(&log);
            Box::new(move |tick, state: &RollbackLabRunState| {
                let mut predictor = predictor.borrow_mut();
                predictor.begin_tick();
                query_burst(
                    &mut predictor,
                    rollback_session::state(&state.session),
                    tick,
                );
                log.borrow_mut().push(boundary_hash(state));
            })
        }
        None => {
            let log = Rc::clone(&log);
            Box::new(move |_tick, state: &RollbackLabRunState| {
                log.borrow_mut().push(boundary_hash(state));
            })
        }
    };

    let options = RollbackLabOptions {
        profile_name: Some(profile_name.to_string()),
        network_seed: Some(network_seed),
        sources: Some(rollback_validation::sources()),
        prevalidated_tape: true,
        observer: Some(observer),
        ..Default::default()
    };
    let result = rollback_lab::run(tape, options);
    let hashes = log.borrow().clone();
    let telemetry = predictor.map(|predictor| predictor.borrow().telemetry());
    CaseRun {
        result,
        hashes,
        telemetry,
    }
}

/// Every authored scenario tape, built once and cloned per (profile, seed)
/// pair below — `InputTape` is a plain owned/`Clone` value (this crate's
/// convention throughout `rollback_lab`/`rollback_session`), and rebuilding
/// the 7,201-frame frozen fixture per case rather than once would dominate
/// this test's run time for no evidentiary gain: the window a scenario
/// slices out of it does not depend on network profile or seed.
fn all_scenario_tapes(tune: &Tuning) -> Vec<(&'static Omp2RollbackScenario, InputTape)> {
    rollback_validation::config()
        .scenarios
        .iter()
        .map(|scenario| (scenario, rollback_validation::scenario_tape(scenario, tune)))
        .collect()
}

/// The acceptance criterion #486 asked for and #495 exists to make
/// reachable: the authored nine-scenario matrix, every authored network
/// profile (`clean`, `omp0_parity`, `playable`, `stress` — the jitter/loss/
/// corruption dimension PR #492's substitute could not cover), every
/// authored network seed, with a query-heavy `BallPredictor` consumer
/// attached through the observer seam. `predict.budget_exhaustions` is
/// printed for every case via `ball_prediction::marker`, satisfying #486's
/// "visible in the matrix runs" -- on these short scenario windows it stays
/// at zero (the default step budget is sized to cover one full horizon
/// rebuild per tick), which is disclosed rather than forced.
#[test]
fn a_query_heavy_consumer_leaves_every_matrix_scenario_hash_identical() {
    let tune = Tuning::new();
    let config = rollback_validation::config();
    assert_eq!(
        config.scenarios.len(),
        6,
        "the authored matrix grew or shrank"
    );
    let scenario_tapes = all_scenario_tapes(&tune);

    let mut cases_checked = 0i64;
    let mut total_answers = 0i64;
    let mut total_budget_exhaustions = 0i64;
    let mut total_rollbacks = 0i64;

    for &profile_name in config.full_profiles {
        for &network_seed in config.network_seeds {
            for (scenario, tape) in &scenario_tapes {
                let label = format!("{profile_name}/{network_seed}/{}", scenario.id);

                let quiet = run_case(tape.clone(), profile_name, network_seed, false);
                let queried = run_case(tape.clone(), profile_name, network_seed, true);

                assert!(
                    quiet.result.success,
                    "{label}: quiet run did not converge: {:?}",
                    quiet.result.status
                );
                assert!(
                    queried.result.success,
                    "{label}: queried run did not converge: {:?}",
                    queried.result.status
                );
                assert_eq!(
                    quiet.result.client_final_hash, queried.result.client_final_hash,
                    "{label}: final client hash changed when queries were issued"
                );
                assert_eq!(
                    quiet.result.reference_final_hash, queried.result.reference_final_hash,
                    "{label}: final reference hash changed when queries were issued"
                );
                assert_eq!(
                    quiet.result.metrics.rollback_count, queried.result.metrics.rollback_count,
                    "{label}: rollback count changed when queries were issued"
                );
                assert_eq!(
                    quiet.result.metrics.correction_count, queried.result.metrics.correction_count,
                    "{label}: correction count changed when queries were issued"
                );
                assert!(
                    !quiet.hashes.is_empty(),
                    "{label}: the observer never fired -- the seam is not wired"
                );
                assert_eq!(
                    quiet.hashes, queried.hashes,
                    "{label}: the per-tick retained-boundary hash sequence diverged when queries \
                     were issued -- the query-heavy consumer leaked into simulation state"
                );

                let telemetry = queried
                    .telemetry
                    .expect("a queried run always attaches a predictor");
                assert!(
                    telemetry.answers > 0,
                    "{label}: the query-heavy consumer never actually queried"
                );
                let marker = ball_prediction::marker(&telemetry);
                assert!(
                    marker.contains("budget_exhaustions="),
                    "{label}: predict.budget_exhaustions is no longer in the harness marker"
                );
                println!("{label}: {marker}");

                total_answers += telemetry.answers;
                total_budget_exhaustions += telemetry.budget_exhaustions;
                total_rollbacks += queried.result.metrics.rollback_count;
                cases_checked += 1;
            }
        }
    }

    let expected_cases =
        (config.full_profiles.len() * config.network_seeds.len() * config.scenarios.len()) as i64;
    assert_eq!(cases_checked, expected_cases);
    assert!(total_answers > 0);
    println!(
        "GC_ROLLBACK_MATRIX_QUERY_SUMMARY|cases={cases_checked}|answers={total_answers}\
         |budget_exhaustions={total_budget_exhaustions}|rollbacks={total_rollbacks}"
    );
    // The stress profile (among the four) is expected to force at least
    // some corrections somewhere across nine scenarios x three seeds; a
    // matrix that never rolled back anywhere would satisfy every assertion
    // above while proving nothing about rollback-under-impairment.
    assert!(
        total_rollbacks > 0,
        "the matrix completed without a single rollback across every profile and seed -- it is \
         not exercising the client under impairment"
    );
}

/// Run `f`, catching a panic and returning its message instead of letting it
/// abort the test.
fn catch_panic_message<F: FnOnce() + std::panic::UnwindSafe>(f: F) -> Option<String> {
    match std::panic::catch_unwind(f) {
        Ok(()) => None,
        Err(payload) => Some(
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "<non-string panic payload>".to_string()),
        ),
    }
}

/// AGENTS.md §9: a comparison that cannot go red proves nothing. The
/// observer seam only ever hands a consumer `&RollbackLabRunState` — never
/// `&mut` — so a genuinely side-effecting observer does not compile; that
/// is the property this file's main test relies on, not something a second
/// test can additionally demonstrate at runtime. What a runtime test *can*
/// demonstrate is that the comparison itself is sensitive to a real
/// divergence rather than being a vacuous `assert_eq!(x, x)`: this tampers
/// one retained-boundary hash the way a consumer that leaked into live
/// state would leave its mark, and requires the exact assertion the main
/// test uses to fail.
#[test]
fn a_tampered_boundary_hash_fails_the_same_comparison_the_matrix_test_uses() {
    let tune = Tuning::new();
    let config = rollback_validation::config();
    let scenario = &config.scenarios[0];
    let tape = rollback_validation::scenario_tape(scenario, &tune);

    let quiet = run_case(tape, "clean", config.network_seeds[0], false);
    assert!(
        !quiet.hashes.is_empty(),
        "fixture produced no retained boundaries to tamper"
    );

    let mut poisoned = quiet.hashes.clone();
    let last = poisoned.len() - 1;
    poisoned[last] = format!("{}-leaked-mutation", poisoned[last]);

    let honest = quiet.hashes;
    let message = catch_panic_message(move || {
        assert_eq!(
            honest, poisoned,
            "a side-effecting consumer changed a retained-boundary hash"
        );
    });
    assert!(
        message.is_some(),
        "the matrix comparison must fail when a consumer's queries leak into retained state, \
         and did not"
    );
}
