//! Tests for `gc_sim::sweep`.
//!
//! There is no "restores the defaults after" assertion here: `crate::tuning`
//! has no mutable global singleton for a sweep to leave dirty — every
//! `sweep::evaluate` call builds a fresh, owned `Tuning` (see
//! `crate::sweep`'s module doc and `crate::tuning`'s own doc), so there is
//! nothing to restore and no equivalent assertion to make. This omission is
//! architectural, not a gap.

use gc_sim::sweep;
use gc_sim::tuning;
use indexmap::IndexMap;

#[test]
fn sweep_paired_delta_computes_the_mean_per_seed_difference_and_its_standard_error() {
    let d = sweep::paired_delta(&[0.2, 0.4, 0.6], &[0.3, 0.5, 0.7]);
    assert!((d.mean - 0.1).abs() < 1e-9);
    assert!(d.se.abs() < 1e-9, "constant shift has zero spread");

    let d2 = sweep::paired_delta(&[0.0, 0.0], &[0.1, 0.3]);
    assert!((d2.mean - 0.2).abs() < 1e-9);
    assert!(d2.se > 0.0);
}

#[test]
fn sweep_sensitivity_perturbs_a_knob_to_min_max() {
    let seeds = [1.0, 2.0];
    let keys: [&'static str; 1] = ["AI_SHOOT_RANGE"];
    let r = sweep::sensitivity(
        &sweep::SensitivityOpts {
            seeds: &seeds,
            keys: Some(&keys),
            duration: Some(10.0),
        },
        None,
    );
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0].key, "AI_SHOOT_RANGE");
    assert!(r.rows[0].impact >= 0.0);
    let report = sweep::sensitivity_report(&r);
    assert!(report.contains("AI_SHOOT_RANGE"));
}

#[test]
fn sweep_parse_blob_round_trips_the_tuning_blob_line_format_skipping_junk() {
    let o = sweep::parse_blob("AI_SHOOT_RANGE=300\nNOT_A_KNOB=5\ngarbage line\nJOCKEY_SLOW=0.6");
    assert_eq!(o.get("AI_SHOOT_RANGE"), Some(&300.0));
    assert_eq!(o.get("JOCKEY_SLOW"), Some(&0.6));
    assert!(o.get("NOT_A_KNOB").is_none());
}

#[test]
fn sweep_ascend_warm_starts_from_given_overrides_and_still_reports_delta_vs_defaults() {
    let keys: [&'static str; 1] = ["AI_SHOOT_RANGE"];
    let seeds = [1.0, 2.0];
    let mut start: IndexMap<&'static str, f64> = IndexMap::new();
    start.insert("AI_SHOOT_RANGE", 300.0);
    let r = sweep::ascend(
        &sweep::AscentOpts {
            keys: &keys,
            seeds: &seeds,
            levels: Some(2),
            passes: Some(1),
            duration: Some(10.0),
            start: Some(&start),
        },
        None,
    );
    assert!(
        r.overrides.contains_key("AI_SHOOT_RANGE"),
        "the start override is in play"
    );
}

#[test]
fn sweep_ascend_only_accepts_strict_improvements_and_reports_the_winning_blob() {
    let keys: [&'static str; 1] = ["AI_SHOOT_RANGE"];
    let seeds = [1.0, 2.0];
    let r = sweep::ascend(
        &sweep::AscentOpts {
            keys: &keys,
            seeds: &seeds,
            levels: Some(3),
            passes: Some(1),
            duration: Some(10.0),
            start: None,
        },
        None,
    );
    assert!(
        r.delta.mean >= 0.0,
        "greedy ascent never ends below its baseline"
    );
    for (&key, &v) in &r.overrides {
        let k = tuning::KNOBS
            .iter()
            .find(|k| k.key == key)
            .unwrap_or_else(|| panic!("{key} is a registered knob"));
        assert!(v >= k.min && v <= k.max, "{key} stays in range");
    }
}
