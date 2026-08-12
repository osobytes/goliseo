//! Tests for `gc_sim::knob_contract` — the knob-moves-metric contract.
//!
//! Two of these matter more than the rest, and they are a pair:
//!
//! - [`knob_contract_passes_for_a_wired_knob`] is the contract's one shipped
//!   example, using an already-wired tunable (`AI_SHOOT_RANGE`).
//! - [`knob_contract_goes_red_for_a_decoration_knob`] is its demonstration
//!   that it can fail, using `REPLAY_SLOWMO` — a registered, swept knob that
//!   no simulation code reads. AGENTS.md §9 requires every gate to ship a
//!   demonstration it can go red, and a contract that cannot fail for a knob
//!   that moves nothing would be worth nothing: the issue that asked for this
//!   helper opened on the finding that dead knobs already exist here.

use gc_sim::knob_contract::{self, KnobMoveOpts, Perturb};

// Short matches and a modest seed set: enough for a wired knob to clear its
// measured noise floor, cheap enough for a per-feature test. A feature whose
// knob is subtler raises either number.
const DURATION: Option<f64> = Some(30.0);

fn seeds(n: usize) -> Vec<f64> {
    (0..n).map(|i| 20_001.0 + i as f64).collect()
}

#[test]
fn knob_contract_passes_for_a_wired_knob() {
    // `AI_SHOOT_RANGE` against `longest_drought_s`: shooting from further out
    // shortens the gaps between chances, which is a claim about the metric's
    // direction, not just its magnitude. Deterministic — the same seeds and
    // duration produce the same numbers on every run, so this cannot flake;
    // the seed count is what buys margin over the measured noise floor.
    let seeds = seeds(48);
    let outcome = knob_contract::assert_moves(&KnobMoveOpts {
        knob: "AI_SHOOT_RANGE",
        metric: "longest_drought_s",
        seeds: &seeds,
        duration: DURATION,
        perturbation: None,
        direction: Some(Perturb::Up),
    });
    assert!(outcome.moved, "{}", outcome.report);
    assert!(
        outcome.delta.abs() > outcome.threshold,
        "the verdict must be the measurement, not a flag: {}",
        outcome.report
    );
    assert!(
        outcome.noise.sd > 0.0 && outcome.noise.standard_error > 0.0,
        "the noise floor must be measured, not assumed: {}",
        outcome.report
    );
    assert!(
        outcome.delta < 0.0,
        "shooting from further out must SHORTEN the drought, not merely move it: {}",
        outcome.report
    );
    assert!(
        outcome.report.contains("WIRED"),
        "the report must say what it concluded: {}",
        outcome.report
    );
}

#[test]
fn knob_contract_goes_red_for_a_decoration_knob() {
    // `REPLAY_SLOWMO` is registered, appears in the F1 panel, and is swept —
    // and no `gc-sim` code reads it (`packages/render/src/replay.ts` restates
    // the value on the TypeScript side). Perturbing it therefore cannot change
    // a single match, so the paired delta is exactly zero and the contract
    // must refuse it.
    let seeds = seeds(8);
    let outcome = knob_contract::knob_moves_metric(&KnobMoveOpts {
        knob: "REPLAY_SLOWMO",
        metric: "goals_total",
        seeds: &seeds,
        duration: DURATION,
        perturbation: None,
        direction: Some(Perturb::Up),
    });
    assert!(
        !outcome.moved,
        "a knob no simulation code reads must fail the contract: {}",
        outcome.report
    );
    assert_eq!(
        outcome.delta, 0.0,
        "a knob nothing reads must produce a bit-identical match: {}",
        outcome.report
    );
    assert!(
        outcome.report.contains("DECORATION"),
        "the report must name the failure: {}",
        outcome.report
    );

    // And the assertion helper a feature test would call actually panics.
    let result = std::panic::catch_unwind(|| {
        knob_contract::assert_moves(&KnobMoveOpts {
            knob: "REPLAY_SLOWMO",
            metric: "goals_total",
            seeds: &seeds,
            duration: DURATION,
            perturbation: None,
            direction: Some(Perturb::Up),
        })
    });
    let err = result.expect_err("assert_moves must panic for a decoration knob");
    let message = err
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_else(|| (*err.downcast_ref::<&str>().unwrap_or(&"")).to_string());
    assert!(
        message.contains("REPLAY_SLOWMO") && message.contains("DECORATION"),
        "the panic must be the report, so a failing review reads why: {message}"
    );
}

#[test]
fn knob_contract_measures_a_noise_floor_rather_than_assuming_one() {
    let seeds = seeds(16);
    let floor = knob_contract::noise_floor("goals_total", &seeds, DURATION);
    assert_eq!(floor.n, seeds.len());
    assert!(floor.sd >= 0.0);
    assert!(
        (floor.standard_error - floor.sd / (floor.n as f64).sqrt()).abs() < 1e-12,
        "the threshold's basis is the standard error of the mean"
    );

    // Two runs of the same measurement agree exactly: the simulation is
    // deterministic, so the "noise" being measured is seed-to-seed spread, not
    // run-to-run variation. That distinction is why the helper thresholds on
    // dispersion rather than repeating a batch.
    let again = knob_contract::noise_floor("goals_total", &seeds, DURATION);
    assert_eq!(floor, again);
}

#[test]
fn knob_contract_panics_on_an_unregistered_knob_or_metric() {
    let two = seeds(2);
    let knob = std::panic::catch_unwind(|| {
        knob_contract::knob_moves_metric(&KnobMoveOpts {
            knob: "NOT_A_KNOB",
            metric: "goals_total",
            seeds: &two,
            duration: DURATION,
            perturbation: None,
            direction: None,
        })
    });
    assert!(knob.is_err(), "an unregistered knob is a programmer error");
    let metric =
        std::panic::catch_unwind(|| knob_contract::noise_floor("not_a_metric", &two, DURATION));
    assert!(
        metric.is_err(),
        "an unregistered metric is a programmer error"
    );
}

/// The noise-floor pilot, run by hand rather than on every CI run.
///
/// `cargo test -p gc-sim --test knob_contract -- --ignored --nocapture` prints
/// every registered metric's mean, per-match standard deviation and standard
/// error at default knobs, over 60 full-length matches. Ignored because it
/// costs minutes, not because the number is optional: it is the evidence
/// behind `knob_contract`'s claim that its threshold is measured, and the PR
/// that introduced this module records one run of it.
#[test]
#[ignore = "noise-floor pilot: minutes, run by hand"]
fn noise_floor_pilot_reports_per_metric_variance_at_defaults() {
    let seeds: Vec<f64> = (0..60).map(|i| 20_001.0 + i as f64).collect();
    println!(
        "{:<22} {:>4} {:>10} {:>10} {:>10}",
        "metric", "n", "mean", "sd", "se"
    );
    for id in gc_sim::metric_registry::shipped().ids() {
        let floor = knob_contract::noise_floor(id, &seeds, None);
        println!(
            "{:<22} {:>4} {:>10.4} {:>10.4} {:>10.4}",
            id, floor.n, floor.mean, floor.sd, floor.standard_error
        );
    }
}
