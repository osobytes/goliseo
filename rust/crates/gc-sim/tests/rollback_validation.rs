//! Test coverage for the assertions in `spec/sim/rollback_validation_spec.lua`.
//!
//! See `rollback_validation.rs`'s module doc comment: `new_campaign`/
//! `step_campaign` implement every suite, but this file only exercises the
//! cheap, self-contained assertions from the Lua spec's first case
//! (`config()`, `profile_digest()`) and the full second case
//! (`"late-window"`, which the Lua spec itself keeps to two small cases).
//! The `"browser-stress"` campaign-construction assertions from the first
//! Lua case (`#browser.cases == 14`, scenario/fixture coverage) are not
//! exercised here — building that plan alone constructs and validates
//! several multi-hundred-tick tapes, which did not fit this pass's time
//! budget; report this as an open gap.

use gc_sim::rollback_validation::{self, RollbackValidationOptions, RollbackValidationSuite};

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
    //
    // These four assertions pin the authored values against transcription
    // error. They are *not* the gate, and must never be mistaken for one:
    // they compare data to its own literals (#470). What measures a real
    // retained window against these budgets is
    // `tests/snapshot_headroom.rs`.
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
    let mut campaign = rollback_validation::new_campaign(
        RollbackValidationSuite::LateWindow,
        RollbackValidationOptions::default(),
    );
    let mut completed = Vec::new();
    let result = loop {
        let (result, row) = rollback_validation::step_campaign(&mut campaign, 4);
        if let Some(row) = row {
            completed.push(row);
        }
        if let Some(result) = result {
            break result;
        }
    };

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
        gc_sim::rollback_lab::RollbackLabStatus::LateInputUnrecoverable
    );
    assert_eq!(completed[1].result.late_input_tick, Some(0));
    assert!(!completed[1].hidden_progress);
    assert!(rollback_validation::case_marker(&completed[1]).contains("expected_failure=1"));
    assert!(
        rollback_validation::result_marker(&result).starts_with("GC_ROLLBACK_VALIDATION|result|")
    );
}
