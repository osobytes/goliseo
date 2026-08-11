//! Port of `spec/sim/lever_metrics_spec.lua`.

use gc_data::{tactics, teams, tuning_presets};
use gc_sim::headless::{BatchOpts, HeadlessBot};
use gc_sim::lever_metrics::{self, LeverLivenessResult};

fn preset_blob(id: &str) -> &'static str {
    tuning_presets::get(id)
        .unwrap_or_else(|| panic!("missing preset: {id}"))
        .blob
}

fn seed_set(n: i64) -> Vec<f64> {
    (1..=n).map(|i| i as f64).collect()
}

fn assert_same_result(a: &LeverLivenessResult, b: &LeverLivenessResult) {
    assert!((a.dwin_pts - b.dwin_pts).abs() < 1e-12);
    assert_eq!(a.win_in_band, b.win_in_band);
    assert_eq!(a.passes, b.passes);
    assert_eq!(a.metric_deltas.len(), b.metric_deltas.len());
    assert_eq!(a.moved_metrics.len(), b.moved_metrics.len());
    for (delta, other) in a.metric_deltas.iter().zip(b.metric_deltas.iter()) {
        assert_eq!(delta.key, other.key);
        assert_eq!(delta.n, other.n);
        assert!((delta.band_widths - other.band_widths).abs() < 1e-12);
    }
}

#[test]
fn lever_liveness_rejects_an_identical_fixture_placebo() {
    let candidate_a = preset_blob("candidate_a");
    let nebula = teams::get("nebula").unwrap();
    let orion = teams::get("orion").unwrap();
    let balanced = tactics::get("balanced").unwrap();
    let base = BatchOpts {
        home: Some(nebula),
        away: Some(orion),
        away_tactic: Some(balanced),
        bot: Some(HeadlessBot::None),
        tuning_blob: Some(candidate_a),
        ..Default::default()
    };
    let seeds = seed_set(12);
    let result = lever_metrics::lever_liveness(&base, &base, &seeds);
    assert!((result.dwin_pts - 0.0).abs() < 1e-12);
    assert_eq!(result.moved_metrics.len(), 0);
    assert!(!result.passes);
}

#[test]
fn lever_liveness_registers_outcome_and_banded_metric_movement_for_a_real_tactic_lever() {
    let candidate_a = preset_blob("candidate_a");
    let nebula = teams::get("nebula").unwrap();
    let orion = teams::get("orion").unwrap();
    let balanced = tactics::get("balanced").unwrap();
    let press_high = tactics::get("press_high").unwrap();
    let counter = tactics::get("counter").unwrap();
    let press = BatchOpts {
        home: Some(nebula),
        away: Some(orion),
        tactic: Some(press_high),
        away_tactic: Some(balanced),
        bot: Some(HeadlessBot::None),
        tuning_blob: Some(candidate_a),
        ..Default::default()
    };
    let counter_opts = BatchOpts {
        home: Some(nebula),
        away: Some(orion),
        tactic: Some(counter),
        away_tactic: Some(balanced),
        bot: Some(HeadlessBot::None),
        tuning_blob: Some(candidate_a),
        ..Default::default()
    };
    // Sixty full matches per option keep the discrete win-rate assertion
    // stable when gameplay tuning changes a handful of seed outcomes.
    let seeds = seed_set(60);
    let result = lever_metrics::lever_liveness(&press, &counter_opts, &seeds);
    assert!(
        result.dwin_pts.abs() > 0.0,
        "different tactics must move home win share"
    );
    assert!(
        !result.moved_metrics.is_empty(),
        "different tactics must move a banded metric"
    );
}

#[test]
fn lever_liveness_repeats_deterministically_on_the_same_common_seed_set() {
    let candidate_a = preset_blob("candidate_a");
    let nebula = teams::get("nebula").unwrap();
    let orion = teams::get("orion").unwrap();
    let balanced = tactics::get("balanced").unwrap();
    let press_high = tactics::get("press_high").unwrap();
    let counter = tactics::get("counter").unwrap();
    let press = BatchOpts {
        home: Some(nebula),
        away: Some(orion),
        tactic: Some(press_high),
        away_tactic: Some(balanced),
        bot: Some(HeadlessBot::None),
        tuning_blob: Some(candidate_a),
        ..Default::default()
    };
    let counter_opts = BatchOpts {
        home: Some(nebula),
        away: Some(orion),
        tactic: Some(counter),
        away_tactic: Some(balanced),
        bot: Some(HeadlessBot::None),
        tuning_blob: Some(candidate_a),
        ..Default::default()
    };
    let seeds = seed_set(16);
    let a = lever_metrics::lever_liveness(&press, &counter_opts, &seeds);
    let b = lever_metrics::lever_liveness(&press, &counter_opts, &seeds);
    assert_same_result(&a, &b);
}
