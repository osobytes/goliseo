//! Execution coverage for the OMP-2 rollback validation campaign.
//!
//! ## What runs here, and what runs elsewhere (#469)
//!
//! `rollback_validation::new_campaign` accepts five suites. Before #469 only
//! `LateWindow` was ever constructed by a test, so `Native`, `BrowserFull`,
//! `BrowserStress` and `Soak` were entry points nothing executed — a plan
//! that is built and validated is not a plan that has been run, and the
//! difference is the whole point of AGENTS.md §9.
//!
//! Four of the five now execute here, and the fifth is a strict subset of
//! one that does:
//!
//! - `LateWindow` — the 30/31-tick boundary pair, unchanged. Per PR.
//! - `BrowserStress` — despite the name, the plan is pure CPU: the nine
//!   authored game moments (tackle, shot, goal, kickoff, aerial,
//!   keeper_action, possession_change, repeated_rollback, full_time) at the
//!   stress profile, plus the combat determinism fixture and the four
//!   combat-load fixtures, for one network seed. `Native` embeds exactly
//!   this case list once per authored seed, so running it here runs the
//!   scenario layer of the native matrix on every PR. It is NOT browser
//!   evidence — running these cases in a real browser is #472's.
//! - `Native` — the complete matrix, including the twelve 7,201-tick
//!   `complete_fixture` runs (four profiles x three seeds) that
//!   `BrowserStress` does not carry. Those dominate the runtime, so the
//!   full-matrix test is `#[ignore]`d and driven by
//!   `scripts/check_rollback_native.sh`; see that script and
//!   `scripts/check.sh`'s header for the measured numbers and where it runs.
//! - `Soak` — same treatment, same script. Read its test's doc comment
//!   before treating it as soak evidence: it proves the suite runs and
//!   converges, and measures no memory at all.
//! - `BrowserFull` is the one suite with no test of its own. Its plan is a
//!   strict subset of `Native`'s — `full-{profile}-{seed}` and
//!   `combat-{profile}-{seed}` for `browser_full_profiles`, all of which
//!   `Native` already builds and this file already executes — so a separate
//!   in-process run would assert nothing new. What is genuinely missing is
//!   running those cases IN A BROWSER, which is #472's.

use gc_sim::rollback_lab::RollbackLabStatus;
use gc_sim::rollback_validation::{
    self, RollbackValidationCompletedCase, RollbackValidationOptions, RollbackValidationResult,
    RollbackValidationSuite,
};
use std::collections::BTreeSet;

/// Authoritative input rows advanced per `step_campaign` call. The campaign
/// is a state machine a browser drives one logical tick per frame; a native
/// runner has no such constraint, so this only trades call overhead against
/// step granularity and changes no outcome.
const STEP_TICKS: i64 = 64;

/// Drive `suite` to completion, returning its result and every completed
/// case in order.
fn run_to_completion(
    suite: RollbackValidationSuite,
    options: RollbackValidationOptions,
) -> (
    RollbackValidationResult,
    Vec<RollbackValidationCompletedCase>,
) {
    let mut campaign = rollback_validation::new_campaign(suite, options);
    let mut completed = Vec::new();
    loop {
        let (result, row) = rollback_validation::step_campaign(&mut campaign, STEP_TICKS);
        if let Some(row) = row {
            completed.push(row);
        }
        if let Some(result) = result {
            return (result, completed);
        }
    }
}

/// The acceptance contract every non-terminal case must satisfy: the
/// impaired client reproduced the no-delay authority exactly, and the case
/// actually observed the game moment it claims to cover.
///
/// `scenario_pass` is asserted separately from `accepted` on purpose. A case
/// whose window silently stopped containing its tackle would still converge
/// — it would just be validating nothing — and `accepted` folds both signals
/// into one boolean, so a bare `assert!(accepted)` cannot tell the two
/// apart in the failure message.
fn assert_reconverged(case: &RollbackValidationCompletedCase) {
    assert!(
        !case.expected_failure,
        "{}: no case in this suite is an expected terminal",
        case.id
    );
    assert!(
        case.scenario_pass,
        "{}: converged without ever observing the '{}' moment it claims to cover -- the window or fixture has gone stale",
        case.id, case.scenario
    );
    assert_eq!(
        case.result.status,
        RollbackLabStatus::Converged,
        "{}: {}",
        case.id,
        rollback_validation::case_marker(case)
    );
    assert!(
        case.result.success,
        "{}: {}",
        case.id,
        rollback_validation::case_marker(case)
    );
    assert!(
        !case.hidden_progress,
        "{}: terminal probe found hidden progress",
        case.id
    );
    assert!(
        case.accepted,
        "{}: {}",
        case.id,
        rollback_validation::case_marker(case)
    );
    assert_eq!(
        case.result.client_final_hash, case.result.reference_final_hash,
        "{}: client and reference final hashes disagree",
        case.id
    );
}

/// Every scenario id the authored registry expects one campaign over a
/// single network seed to cover: the nine game moments, the combat
/// determinism fixture, and the four combat-load fixtures.
fn expected_scenario_ids() -> BTreeSet<String> {
    let config = rollback_validation::config();
    let mut ids: BTreeSet<String> = config
        .scenarios
        .iter()
        .map(|scenario| scenario.id.to_string())
        .collect();
    ids.insert("combat".to_string());
    for fixture in config.combat_load_fixtures {
        ids.insert(fixture.scenario.to_string());
    }
    ids
}

fn observed_scenario_ids(completed: &[RollbackValidationCompletedCase]) -> BTreeSet<String> {
    completed.iter().map(|case| case.scenario.clone()).collect()
}

#[test]
fn pins_the_required_scenario_and_runtime_matrix_config_only() {
    let config = rollback_validation::config();
    assert_eq!(config.fixture_seed, 19);
    assert_eq!(config.source_pattern, "LRRRRRRR");
    assert_eq!(config.network_seeds.len(), 3);
    assert_eq!(config.network_seeds[0], 2001);
    assert_eq!(config.network_seeds[2], 2003);
    assert_eq!(config.full_profiles.len(), 4);
    assert_eq!(config.scenarios.len(), 9);
    assert_eq!(config.soak_network_seeds.len(), 5);
    assert_eq!(config.soak_network_seeds[0], 2001);
    assert_eq!(config.soak_network_seeds[1], 2002);
    assert_eq!(config.soak_network_seeds[2], 2003);
    assert_eq!(config.soak_network_seeds[3], 2001);
    assert_eq!(config.soak_network_seeds[4], 2002);
    assert_eq!(config.soak_samples.len(), 5);
    assert_eq!(config.combat_fixture.id, "omp2-combat-rollback-v1");
    assert_eq!(config.combat_fixture.frame_count, 80);
    assert_eq!(config.soak_samples[0], "warmup");
    assert_eq!(config.soak_samples[1], "120");
    assert_eq!(config.soak_samples[2], "360");
    assert_eq!(config.soak_samples[3], "600");
    assert_eq!(config.soak_samples[4], "final");
    assert_eq!(config.budgets.snapshot_count, 31);
    // The one place the retained-storage gates are spelled as literals.
    // #209 raised both a 128-KiB step; the 256-KiB gap between them is
    // deliberate and pinned.
    assert_eq!(config.budgets.snapshot_bytes, 896 * 1024);
    assert_eq!(config.budgets.history_bytes, 1152 * 1024);
    assert_eq!(
        config.budgets.history_bytes - config.budgets.snapshot_bytes,
        256 * 1024
    );
    assert!((config.budgets.memory_growth_ratio - 0.10).abs() < f64::EPSILON);
    assert_eq!(rollback_validation::profile_digest(), "5fbf1e0d51a6f4d5");
}

#[test]
fn accepts_delay_thirty_and_classifies_delay_thirty_one_as_the_explicit_terminal() {
    let (result, completed) = run_to_completion(
        RollbackValidationSuite::LateWindow,
        RollbackValidationOptions::default(),
    );

    assert!(result.success);
    assert_eq!(result.case_count, 2);
    assert_eq!(completed.len(), 2);
    assert_eq!(completed[0].id, "delay-30");
    assert!(completed[0].result.success);
    assert_eq!(completed[0].result.metrics.max_rollback_depth, 30);
    assert_eq!(completed[1].id, "delay-31");
    assert!(completed[1].accepted);
    assert!(completed[1].expected_failure);
    assert_eq!(
        completed[1].result.status,
        RollbackLabStatus::LateInputUnrecoverable
    );
    assert_eq!(completed[1].result.late_input_tick, Some(0));
    assert!(!completed[1].hidden_progress);
    assert!(rollback_validation::case_marker(&completed[1]).contains("expected_failure=1"));
    assert!(
        rollback_validation::result_marker(&result).starts_with("GC_ROLLBACK_VALIDATION|result|")
    );
}

/// The per-PR half of #469: every authored game moment, every combat
/// fixture, executed under the stress profile for every authored network
/// seed. This is the native matrix's scenario layer, case for case — the
/// twelve 7,201-tick `complete_fixture` runs it omits are what
/// `executes_the_complete_native_matrix` below adds.
#[test]
fn executes_every_authored_scenario_under_the_stress_profile_for_every_seed() {
    let config = rollback_validation::config();
    let expected_scenarios = expected_scenario_ids();
    let expected_cases = (config.scenarios.len() + 1 + config.combat_load_fixtures.len()) as i64;
    let mut total_rollbacks = 0i64;

    for &network_seed in config.network_seeds {
        let (result, completed) = run_to_completion(
            RollbackValidationSuite::BrowserStress,
            RollbackValidationOptions {
                profile_name: Some(config.stress_profile.to_string()),
                network_seed: Some(network_seed),
                measure: None,
            },
        );

        assert_eq!(
            result.case_count,
            expected_cases,
            "seed {network_seed}: {}",
            rollback_validation::result_marker(&result)
        );
        assert_eq!(completed.len() as i64, expected_cases);
        for case in &completed {
            assert_reconverged(case);
            assert_eq!(case.result.network_seed, network_seed, "{}", case.id);
            assert_eq!(case.result.profile, config.stress_profile, "{}", case.id);
            total_rollbacks += case.result.metrics.rollback_count;
        }
        assert_eq!(
            observed_scenario_ids(&completed),
            expected_scenarios,
            "seed {network_seed}: the executed case list no longer covers every authored scenario"
        );
        assert!(
            result.success,
            "seed {network_seed}: {}",
            rollback_validation::result_marker(&result)
        );
    }

    // A matrix in which nothing ever rolled back would satisfy every
    // assertion above while proving nothing about rollback. The stress
    // profile exists precisely to force corrections.
    assert!(
        total_rollbacks > 0,
        "the stress-profile matrix completed without a single rollback -- it is not exercising the client under impairment"
    );
}

/// The complete `RollbackValidationSuite::Native` matrix, executed.
///
/// `#[ignore]`d for runtime, not for doubt. Measured locally in a debug
/// build: 325.9s for all 66 cases, of which the twelve 7,201-tick
/// `complete_fixture` runs are 310.2s — 95% of the total — and the other 54
/// cases are 11.2s. Those twelve are the only thing this test adds over
/// `executes_every_authored_scenario_under_the_stress_profile_for_every_seed`
/// above, which runs on every PR in 12.4s.
///
/// `cargo test --workspace` — the per-PR gate in `scripts/check.sh` — does
/// not run `#[ignore]`d tests, so this one is run by
/// `scripts/check_rollback_native.sh`, which `.github/workflows/ci.yml`
/// invokes as an on-demand (`workflow_dispatch`) job. An ignored test
/// nothing invokes would be the same defect #469 is about, one layer down.
#[test]
#[ignore = "the full native matrix takes minutes; run scripts/check_rollback_native.sh"]
fn executes_the_complete_native_matrix() {
    let config = rollback_validation::config();
    let profiles = config.full_profiles.len();
    let seeds = config.network_seeds.len();
    // 24 profile x seed cases (`complete_fixture` + `combat` per pair), then
    // the stress scenario layer once per seed.
    let expected_cases = (profiles * seeds * 2
        + seeds * (config.scenarios.len() + 1 + config.combat_load_fixtures.len()))
        as i64;

    let (result, completed) = run_to_completion(
        RollbackValidationSuite::Native,
        RollbackValidationOptions::default(),
    );

    assert_eq!(
        result.case_count,
        expected_cases,
        "{}",
        rollback_validation::result_marker(&result)
    );
    assert_eq!(completed.len() as i64, expected_cases);

    for case in &completed {
        assert_reconverged(case);
    }

    // Every authored profile/seed pair ran the complete 7,201-tick fixture.
    let full_pairs: BTreeSet<(String, i64)> = completed
        .iter()
        .filter(|case| case.scenario == "complete_fixture")
        .map(|case| (case.result.profile.clone(), case.result.network_seed))
        .collect();
    assert_eq!(full_pairs.len(), profiles * seeds);
    for profile in config.full_profiles {
        for &network_seed in config.network_seeds {
            assert!(
                full_pairs.contains(&((*profile).to_string(), network_seed)),
                "the native matrix never ran the complete fixture for {profile}/{network_seed}"
            );
        }
    }

    // Every authored game moment ran, on every authored seed.
    let expected_scenarios = expected_scenario_ids();
    let mut observed = observed_scenario_ids(&completed);
    assert!(
        observed.remove("complete_fixture"),
        "the native matrix ran no complete-fixture case"
    );
    assert_eq!(observed, expected_scenarios);
    for scenario in config.scenarios {
        let seeds_seen: BTreeSet<i64> = completed
            .iter()
            .filter(|case| case.scenario == scenario.id)
            .map(|case| case.result.network_seed)
            .collect();
        assert_eq!(
            seeds_seen.len(),
            seeds,
            "scenario '{}' did not run on every authored network seed",
            scenario.id
        );
    }

    assert!(
        completed
            .iter()
            .map(|case| case.result.metrics.rollback_count)
            .sum::<i64>()
            > 0,
        "the native matrix completed without a single rollback"
    );
    assert!(
        result.success,
        "{}",
        rollback_validation::result_marker(&result)
    );
    // The campaign's own terminator, printed so a runner (and
    // scripts/check_rollback_native.sh) can read the verdict back out of the
    // log independently of this process's exit status -- AGENTS.md §9's
    // "never trust one signal".
    println!("{}", rollback_validation::result_marker(&result));
}

/// The `RollbackValidationSuite::Soak` matrix, executed — the fifth and last
/// suite that nothing constructed before #469.
///
/// BE CLEAR ABOUT WHAT THIS DOES AND DOES NOT PROVE. The suite's ten cases
/// are the complete fixture and the combat fixture at the `playable`
/// profile over `soak_network_seeds` (2001, 2002, 2003, 2001, 2002 — the
/// last two repeats). Behaviourally that is a subset of what
/// `executes_the_complete_native_matrix` already runs for `playable`, so
/// this adds no new convergence assertion; what it adds is that the suite
/// is constructed and run at all, and that its authored sample points still
/// land on the plan in order.
///
/// The property the word "soak" names — retained memory not growing across
/// forced-GC checkpoints — is a RUNTIME measurement this crate cannot take:
/// `soak_samples` reaches a completed case as a label and nothing here
/// weighs anything. That gap is #472's, not this test's, and this test must
/// not be read as covering it.
///
/// Measured locally in a debug build: 140.7s, effectively all of it the five
/// 7,201-tick cases. Same reasoning as the native matrix above — too slow
/// for the per-PR gate, so `#[ignore]`d and run by
/// `scripts/check_rollback_native.sh`.
#[test]
#[ignore = "the soak matrix takes minutes; run scripts/check_rollback_native.sh"]
fn executes_the_complete_soak_matrix() {
    let config = rollback_validation::config();
    let expected_cases = (config.soak_network_seeds.len() * 2) as i64;

    let (result, completed) = run_to_completion(
        RollbackValidationSuite::Soak,
        RollbackValidationOptions::default(),
    );

    assert_eq!(
        result.case_count,
        expected_cases,
        "{}",
        rollback_validation::result_marker(&result)
    );
    for case in &completed {
        assert_reconverged(case);
        assert_eq!(case.result.profile, "playable", "{}", case.id);
    }

    // Every authored seed ran, in the authored order, and every authored
    // sample point landed on a complete-fixture case.
    let seeds: Vec<i64> = completed
        .iter()
        .filter(|case| case.scenario == "complete_fixture")
        .map(|case| case.result.network_seed)
        .collect();
    assert_eq!(seeds, config.soak_network_seeds.to_vec());
    let samples: Vec<String> = completed
        .iter()
        .filter_map(|case| case.sample.clone())
        .collect();
    assert_eq!(
        samples,
        config
            .soak_samples
            .iter()
            .map(|sample| (*sample).to_string())
            .collect::<Vec<String>>()
    );

    assert!(
        result.success,
        "{}",
        rollback_validation::result_marker(&result)
    );
    println!("{}", rollback_validation::result_marker(&result));
}
