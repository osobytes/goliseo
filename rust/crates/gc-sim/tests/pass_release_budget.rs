//! #491's budget acceptance criterion: *"release-tick query burst measured
//! within the prediction budget under rollback"*.
//!
//! ## Why the real matrix and not a hand-driven substitute
//!
//! #486 asked for the same evidence for the prediction service itself and got
//! a hand-driven `rollback_session` peer, because there was no way to inject
//! per-tick work into a real campaign. #495 (`e2ae786`) added the
//! `rollback_lab::RollbackLabOptions::observer` seam, and
//! `tests/rollback_lab_query_observer.rs` is the general-purpose query-heavy
//! consumer run through it. This file is the **pass-shaped** one: instead of
//! a synthetic mix of every query kind, the observer issues exactly what
//! `r#match::release_pass` issues — a `pass_lead::solve` burst against a
//! freshly-sized `pass_lead::release_predictor` — against the campaign's own
//! live state, over the authored scenario matrix, every profile, every seed.
//!
//! ## It over-states the load on purpose
//!
//! A real tick releases **at most one** pass. The observer here solves a
//! burst for **every home outfielder as receiver, on every single tick** — so
//! the measured worst tick is an upper bound with a wide margin, not a
//! typical one. If that fits, one release per tick fits.
//!
//! ## Two claims, and they are different
//!
//! 1. **Purity.** Issuing the burst leaves every retained-boundary hash, the
//!    final hashes, the rollback count and the correction count identical.
//!    That is the determinism claim: a release-tick solve cannot leak into
//!    simulation state, so a resimulated release re-solves rather than
//!    remembers.
//! 2. **Budget.** No burst exhausts its predictor's budget, and the measured
//!    per-burst scratch-step cost is reported. The budget it must fit is
//!    `pass_lead::release_predictor`'s, which is derived from the burst
//!    rather than borrowed from the shared per-tick ceiling — see that
//!    function's doc for why the shared default was measured and rejected.

use gc_core::vec2::Vec2;
use gc_data::omp2_rollback_validation::Omp2RollbackScenario;
use gc_sim::ball_prediction::BallPredictionTelemetry;
use gc_sim::input_tape::InputTape;
use gc_sim::match_snapshot::{self, MatchState, Team};
use gc_sim::pass_lead;
use gc_sim::rollback_lab::{self, RollbackLabObserver, RollbackLabOptions, RollbackLabRunState};
use gc_sim::rollback_session;
use gc_sim::rollback_validation;
use gc_sim::tuning::Tuning;
use std::cell::RefCell;
use std::rc::Rc;

/// The possession radius `r#match::release_pass` passes as the arrival
/// capture distance.
const CAPTURE: f64 = 22.0;

/// The worst single burst observed across the whole matrix.
#[derive(Clone, Copy, Debug, Default)]
struct BurstCost {
    queries: i64,
    scratch_steps: i64,
    exhaustions: i64,
    bursts: i64,
    solved: i64,
}

impl BurstCost {
    fn fold(&mut self, other: &BurstCost) {
        self.queries = self.queries.max(other.queries);
        self.scratch_steps = self.scratch_steps.max(other.scratch_steps);
        self.exhaustions += other.exhaustions;
        self.bursts += other.bursts;
        self.solved += other.solved;
    }
}

/// One release burst, exactly as `release_pass` issues it: a fresh
/// release-sized predictor, one `solve` against the live state.
fn release_burst(
    state: &MatchState,
    passer: Vec2,
    receiver_index: usize,
    tune: &Tuning,
) -> BurstCost {
    let mut predictor = pass_lead::release_predictor();
    let solved = pass_lead::solve(
        state,
        &mut predictor,
        passer,
        &state.players[receiver_index],
        CAPTURE,
        1.0,
        tune,
    );
    let BallPredictionTelemetry {
        answers,
        scratch_steps,
        budget_exhaustions,
        ..
    } = predictor.telemetry();
    BurstCost {
        queries: answers,
        scratch_steps,
        exhaustions: budget_exhaustions,
        bursts: 1,
        solved: i64::from(solved.is_some()),
    }
}

/// Every home outfielder as a receiver, from the ball's current position —
/// deliberately more load than any real tick produces.
fn tick_bursts(state: &MatchState, tune: &Tuning) -> BurstCost {
    let mut worst = BurstCost::default();
    for index in 0..state.players.len() {
        if state.players[index].team != Team::Home || state.players[index].is_keeper {
            continue;
        }
        worst.fold(&release_burst(state, state.ball, index, tune));
    }
    worst
}

fn boundary_hash(state: &RollbackLabRunState) -> String {
    match_snapshot::hash_canonical(&rollback_session::current_snapshot(&state.session))
}

struct CaseRun {
    result: rollback_lab::RollbackLabResult,
    hashes: Vec<String>,
    cost: BurstCost,
}

fn run_case(
    tape: InputTape,
    profile_name: &str,
    network_seed: i64,
    attach_bursts: bool,
) -> CaseRun {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let cost: Rc<RefCell<BurstCost>> = Rc::new(RefCell::new(BurstCost::default()));

    let observer: RollbackLabObserver = if attach_bursts {
        let log = Rc::clone(&log);
        let cost = Rc::clone(&cost);
        Box::new(move |_tick, state: &RollbackLabRunState| {
            let tune = Tuning::new();
            let tick_cost = tick_bursts(rollback_session::state(&state.session), &tune);
            cost.borrow_mut().fold(&tick_cost);
            log.borrow_mut().push(boundary_hash(state));
        })
    } else {
        // The quiet run records the SAME hash the queried run records, so the
        // observer's own read-only bookkeeping is not itself the difference.
        let log = Rc::clone(&log);
        Box::new(move |_tick, state: &RollbackLabRunState| {
            log.borrow_mut().push(boundary_hash(state));
        })
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
    let cost = *cost.borrow();
    CaseRun {
        result,
        hashes,
        cost,
    }
}

fn all_scenario_tapes(tune: &Tuning) -> Vec<(&'static Omp2RollbackScenario, InputTape)> {
    rollback_validation::config()
        .scenarios
        .iter()
        .map(|scenario| (scenario, rollback_validation::scenario_tape(scenario, tune)))
        .collect()
}

#[test]
fn the_release_burst_fits_its_budget_and_changes_no_hash_across_the_matrix() {
    let tune = Tuning::new();
    let config = rollback_validation::config();
    let scenario_tapes = all_scenario_tapes(&tune);
    let candidates = pass_lead::LeadKnobs::of(&tune).steps as i64;

    let mut worst = BurstCost::default();
    let mut cases_checked = 0i64;
    let mut total_rollbacks = 0i64;

    for &profile_name in config.full_profiles {
        for &network_seed in config.network_seeds {
            for (scenario, tape) in &scenario_tapes {
                let label = format!("{profile_name}/{network_seed}/{}", scenario.id);
                let quiet = run_case(tape.clone(), profile_name, network_seed, false);
                let busy = run_case(tape.clone(), profile_name, network_seed, true);

                assert!(quiet.result.success, "{label}: quiet run did not converge");
                assert!(busy.result.success, "{label}: burst run did not converge");
                assert!(
                    !quiet.hashes.is_empty(),
                    "{label}: the observer never fired -- the seam is not wired"
                );
                assert_eq!(
                    quiet.hashes, busy.hashes,
                    "{label}: the per-tick retained-boundary hash sequence diverged when release \
                     bursts were issued -- the lead solver leaked into simulation state"
                );
                assert_eq!(
                    quiet.result.client_final_hash, busy.result.client_final_hash,
                    "{label}: final client hash changed"
                );
                assert_eq!(
                    quiet.result.reference_final_hash, busy.result.reference_final_hash,
                    "{label}: final reference hash changed"
                );
                assert_eq!(
                    quiet.result.metrics.rollback_count, busy.result.metrics.rollback_count,
                    "{label}: rollback count changed"
                );
                assert_eq!(
                    quiet.result.metrics.correction_count, busy.result.metrics.correction_count,
                    "{label}: correction count changed"
                );
                assert_eq!(
                    busy.cost.exhaustions, 0,
                    "{label}: a release burst exhausted its own predictor's budget, which \
                     silently drops the LONGEST leads and biases the solver short"
                );
                assert!(
                    busy.cost.queries <= candidates,
                    "{label}: a burst issued {} queries against a {candidates}-candidate set",
                    busy.cost.queries
                );

                worst.fold(&busy.cost);
                total_rollbacks += busy.result.metrics.rollback_count;
                cases_checked += 1;
            }
        }
    }

    let expected_cases =
        (config.full_profiles.len() * config.network_seeds.len() * config.scenarios.len()) as i64;
    assert_eq!(cases_checked, expected_cases);
    assert!(
        total_rollbacks > 0,
        "no case rolled back at all, so this says nothing about resimulation"
    );
    assert!(
        worst.solved > 0,
        "no burst in the entire matrix produced a lead, so the cost measured is the cost of \
         refusing early -- the budget claim would be vacuous"
    );

    // The measurement the issue asked to be reported, not merely asserted.
    let budget = pass_lead::release_predictor().budget_remaining();
    println!(
        "GC_PASS_BUDGET|cases={cases_checked}|bursts={}|solved={}|rollbacks={total_rollbacks}\
         |max_queries={}|max_scratch_steps={}|release_budget={budget}|exhaustions={}",
        worst.bursts, worst.solved, worst.queries, worst.scratch_steps, worst.exhaustions
    );
    assert!(
        worst.scratch_steps <= budget,
        "the worst burst spent {} scratch ticks against a {budget} budget",
        worst.scratch_steps
    );
}

/// AGENTS.md §9: every gate ships a demonstration that it can go red.
///
/// The assertion above that no burst exhausts its budget is only worth
/// anything if exhaustion is *reachable* and *visible*. This starves a
/// predictor deliberately — a handful of scratch ticks instead of the
/// release-sized budget — and requires the exact failure the matrix run
/// asserts against: `budget_exhaustions` rises, and the solve silently
/// degrades to fewer answered candidates.
///
/// It also pins the degradation's SHAPE, which is why
/// `pass_lead::release_predictor` exists at all: a starved burst does not
/// merely cost less, it loses the LONGEST candidates first (they are the ones
/// whose ball travels furthest), so a solver running out of budget biases
/// itself toward short leads while still looking like it works.
#[test]
fn a_starved_predictor_exhausts_its_budget_and_loses_the_long_candidates() {
    use gc_sim::ball_prediction::{BallPredictionConfig, BallPredictor};
    use gc_sim::r#match::{self as sim_match, NewMatchOptions};
    use gc_sim::match_snapshot::PitchSize;

    let tune = Tuning::new();
    let mut state = sim_match::new(NewMatchOptions {
        home: gc_data::teams::get("nebula").expect("nebula is authored"),
        away: gc_data::teams::get("orion").expect("orion is authored"),
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
    });
    // A receiver running hard across a long pass lane: several candidates,
    // each needing real scratch ticks.
    let receiver = state
        .players
        .iter()
        .position(|p| !p.is_keeper)
        .expect("the fixture has outfielders");
    state.players[receiver].pos = Vec2::new(620.0, 200.0);
    state.players[receiver].run_vel = Vec2::new(0.0, 240.0);
    state.players[receiver].vel = state.players[receiver].run_vel;
    state.players[receiver].facing = Vec2::new(0.0, 1.0);
    let passer = Vec2::new(200.0, 260.0);

    let healthy = release_burst(&state, passer, receiver, &tune);
    assert_eq!(
        healthy.exhaustions, 0,
        "the control burst must fit, or the starved one proves nothing"
    );
    assert!(
        healthy.queries > 1,
        "the control burst must issue candidates"
    );

    let mut starved = BallPredictor::new(BallPredictionConfig {
        step_budget: 12,
        ..BallPredictionConfig::default()
    });
    let solution = pass_lead::solve(
        &state,
        &mut starved,
        passer,
        &state.players[receiver],
        CAPTURE,
        1.0,
        &tune,
    );
    let telemetry = starved.telemetry();
    assert!(
        telemetry.budget_exhaustions > 0,
        "a 12-tick budget must exhaust against a multi-candidate burst, or the matrix \
         assertion is unfalsifiable: {telemetry:?}"
    );
    assert!(
        telemetry.scratch_steps <= 12,
        "a starved predictor must actually stop spending: {telemetry:?}"
    );
    // The degradation's shape: every candidate that could not be answered is
    // dropped, so at most the shortest survives -- and here none does.
    assert!(
        solution.is_none_or(|s| s.lead_time
            < healthy_lead(&state, passer, receiver, &tune).unwrap_or(f64::INFINITY)),
        "a starved solve must not return a longer lead than a funded one"
    );
}

fn healthy_lead(state: &MatchState, passer: Vec2, receiver: usize, tune: &Tuning) -> Option<f64> {
    let mut predictor = pass_lead::release_predictor();
    pass_lead::solve(
        state,
        &mut predictor,
        passer,
        &state.players[receiver],
        CAPTURE,
        1.0,
        tune,
    )
    .map(|s| s.lead_time)
}
