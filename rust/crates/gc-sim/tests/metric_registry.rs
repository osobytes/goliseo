//! Tests for `gc_sim::metric_registry`.
//!
//! The load-bearing one is
//! [`registered_metrics_score_bit_for_bit_like_the_pre_migration_fold`]: the
//! fun score is a geometric mean, floating-point multiplication is not
//! associative, and the whole migration is only safe if the registry folds the
//! same eight desirabilities in the same order. So the pre-migration
//! implementation is reproduced here VERBATIM — the private `BAND_*` table and
//! the `entries` array `crate::metrics::fun_score` used to carry — and the two
//! are compared on their raw bits, not with a tolerance.

use gc_data::tunables::Tier;
use gc_sim::metric_registry;
use gc_sim::metrics::{self, MatchMetrics};
use indexmap::IndexMap;

// ---------------------------------------------------------------------------
// The pre-migration implementation, verbatim from `metrics.rs` before #487.
// ---------------------------------------------------------------------------

const BAND_GOALS_TOTAL: [f64; 4] = [0.0, 2.0, 5.0, 8.0];
const BAND_SHOTS_PER_GOAL: [f64; 4] = [1.0, 2.5, 6.0, 25.0];
const BAND_SAVE_RATE: [f64; 4] = [0.15, 0.45, 0.75, 0.95];
const BAND_PASS_COMPLETION: [f64; 4] = [0.25, 0.55, 0.85, 1.001];
const BAND_TURNOVERS_PER_MIN: [f64; 4] = [0.3, 1.0, 5.0, 10.0];
const BAND_POSSESSION_BALANCE: [f64; 4] = [0.1, 0.35, 0.65, 0.9];
const BAND_LONGEST_DROUGHT_S: [f64; 4] = [-1.0, 0.0, 35.0, 80.0];
const BAND_DECIDED_LATE: [f64; 4] = [0.05, 0.4, 1.0, f64::INFINITY];

fn legacy_desirability(v: f64, band: [f64; 4]) -> f64 {
    let [zl, gl, gh, zh] = band;
    if v <= zl || v >= zh {
        return 0.0;
    }
    if v < gl {
        return (v - zl) / (gl - zl);
    }
    if v > gh {
        return (zh - v) / (zh - gh);
    }
    1.0
}

fn legacy_fun_score(m: &MatchMetrics) -> (f64, IndexMap<&'static str, f64>) {
    let entries: [(&'static str, [f64; 4], Option<f64>); 8] = [
        ("goals_total", BAND_GOALS_TOTAL, Some(m.goals_total as f64)),
        ("shots_per_goal", BAND_SHOTS_PER_GOAL, m.shots_per_goal),
        ("save_rate", BAND_SAVE_RATE, m.save_rate),
        ("pass_completion", BAND_PASS_COMPLETION, m.pass_completion),
        (
            "turnovers_per_min",
            BAND_TURNOVERS_PER_MIN,
            Some(m.turnovers_per_min),
        ),
        (
            "possession_balance",
            BAND_POSSESSION_BALANCE,
            m.possession_balance,
        ),
        (
            "longest_drought_s",
            BAND_LONGEST_DROUGHT_S,
            Some(m.longest_drought_s),
        ),
        ("decided_late", BAND_DECIDED_LATE, Some(m.decided_late)),
    ];

    let mut product = 1.0_f64;
    let mut n = 0_i32;
    let mut per = IndexMap::new();
    for (key, band, value) in entries {
        if let Some(v) = value {
            let d = legacy_desirability(v, band);
            per.insert(key, d);
            product *= d;
            n += 1;
        }
    }
    if n == 0 {
        return (0.0, per);
    }
    (product.powf(1.0 / f64::from(n)), per)
}

// ---------------------------------------------------------------------------

fn sample(index: usize) -> MatchMetrics {
    // Deliberately awkward numbers: values inside bands, on band edges,
    // outside them, and `None` denominators, so the comparison exercises every
    // branch of the fold rather than one comfortable match.
    let mut m = MatchMetrics {
        goals_total: (index as i64) % 9,
        turnovers_per_min: 0.37 + index as f64 * 0.61,
        longest_drought_s: 3.0 + index as f64 * 7.3,
        decided_late: 0.05 + index as f64 * 0.11,
        ..Default::default()
    };
    m.shots_per_goal = if index.is_multiple_of(5) {
        None
    } else {
        Some(1.0 + index as f64 * 2.7)
    };
    m.save_rate = if index.is_multiple_of(7) {
        None
    } else {
        Some(0.13 + index as f64 * 0.061)
    };
    m.pass_completion = if index.is_multiple_of(4) {
        None
    } else {
        Some(0.2 + index as f64 * 0.043)
    };
    m.possession_balance = Some(0.09 + index as f64 * 0.037);
    m
}

#[test]
fn registered_metrics_score_bit_for_bit_like_the_pre_migration_fold() {
    for index in 0..64 {
        let m = sample(index);
        let (legacy, legacy_per) = legacy_fun_score(&m);
        let (now, now_per) = metrics::fun_score(&m);
        assert_eq!(
            legacy.to_bits(),
            now.to_bits(),
            "fun score changed at sample {index}: {legacy} -> {now}"
        );
        assert_eq!(
            legacy_per.len(),
            now_per.len(),
            "a different number of metrics contributed at sample {index}"
        );
        for ((lk, lv), (nk, nv)) in legacy_per.iter().zip(now_per.iter()) {
            assert_eq!(lk, nk, "desirability key order changed at sample {index}");
            assert_eq!(
                lv.to_bits(),
                nv.to_bits(),
                "desirability for {lk} changed at sample {index}"
            );
        }
    }
}

#[test]
fn metric_registry_folds_in_the_authored_order() {
    assert_eq!(
        metric_registry::shipped().ids(),
        vec![
            "goals_total",
            "shots_per_goal",
            "save_rate",
            "pass_completion",
            "turnovers_per_min",
            "possession_balance",
            "longest_drought_s",
            "decided_late",
            "time_to_reverse",
        ]
    );
}

#[test]
fn metric_registry_is_the_only_band_table_left() {
    let reg = metric_registry::shipped();
    // The three places that used to carry a band table now agree by
    // construction; this pins the values themselves so a silent edit to one
    // authored row still has to be deliberate.
    assert_eq!(reg.band("goals_total"), Some([0.0, 2.0, 5.0, 8.0]));
    assert_eq!(reg.band("shots_per_goal"), Some([1.0, 2.5, 6.0, 25.0]));
    assert_eq!(reg.band("save_rate"), Some([0.15, 0.45, 0.75, 0.95]));
    assert_eq!(reg.band("pass_completion"), Some([0.25, 0.55, 0.85, 1.001]));
    assert_eq!(reg.band("turnovers_per_min"), Some([0.3, 1.0, 5.0, 10.0]));
    assert_eq!(reg.band("possession_balance"), Some([0.1, 0.35, 0.65, 0.9]));
    assert_eq!(reg.band("longest_drought_s"), Some([-1.0, 0.0, 35.0, 80.0]));
    assert_eq!(
        reg.band("decided_late"),
        Some([0.05, 0.4, 1.0, f64::INFINITY])
    );
    assert_eq!(reg.band("not_a_metric"), None);

    // `lever_metrics` used to keep these eight widths by hand.
    assert_eq!(reg.band_width("goals_total"), Some(3.0));
    assert_eq!(reg.band_width("longest_drought_s"), Some(35.0));
    assert_eq!(
        reg.band_width("decided_late"),
        Some(1.0 - 0.4),
        "the width must be computed from the band, not restated"
    );
}

#[test]
fn metric_registry_extracts_through_the_registered_function() {
    let reg = metric_registry::shipped();
    let mut m = MatchMetrics {
        goals_total: 3,
        ..Default::default()
    };
    m.save_rate = None;
    assert_eq!(reg.extract("goals_total", &m), Some(3.0));
    assert_eq!(
        reg.extract("save_rate", &m),
        None,
        "a missing denominator stays missing rather than becoming zero"
    );
    assert_eq!(reg.extract("not_a_metric", &m), None);
}

#[test]
fn metric_registry_panics_on_a_duplicate_registration() {
    let result = std::panic::catch_unwind(|| {
        let mut b = metric_registry::MetricRegistryBuilder::new();
        let one = metric_registry::shipped().metrics()[0];
        b.register_metrics("first", &[one]);
        b.register_metrics("second", &[one]);
        b.build()
    });
    let err = result.expect_err("a duplicate metric id must be a startup assertion failure");
    let message = err
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_else(|| (*err.downcast_ref::<&str>().unwrap_or(&"")).to_string());
    assert!(
        message.contains("duplicate metric id"),
        "unhelpful duplicate-id panic: {message}"
    );
}

#[test]
fn metric_bands_are_authored_content_and_are_not_tunables() {
    // A band is not a tier-1 scalar: a sweep varying its own scoring bands
    // would be optimising the ruler. The two registries are separate, and no
    // metric id is a registered tunable.
    let tunables = gc_sim::tunable_registry::shipped();
    for id in metric_registry::shipped().ids() {
        assert!(
            tunables.def(id).is_none(),
            "{id} is both a metric and a tunable"
        );
    }
    assert!(tunables.tier(Tier::Sim).len() >= 8);
}
