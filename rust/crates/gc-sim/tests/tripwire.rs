//! Tests for `gc_sim::tripwire`.

use gc_sim::tripwire;
use indexmap::IndexMap;

/// A signature table covering every tracked metric, offset by `bump` on
/// `key`. Mirrors the Lua spec's local `signature` helper.
fn signature(bump: Option<f64>, key: Option<&str>) -> tripwire::Signature {
    let mut sig: IndexMap<&'static str, f64> = IndexMap::new();
    for (i, &k) in tripwire::TRACKED.iter().enumerate() {
        sig.insert(k, (i + 1) as f64 * 0.5);
    }
    if let Some(key) = key {
        let entry = sig.get_mut(key).expect("key is tracked");
        *entry += bump.unwrap_or(0.0);
    }
    sig
}

#[test]
fn tripwire_pins_normalized_controlled_vs_ai_dribble_diagnostics() {
    let tracked: std::collections::BTreeSet<&str> = tripwire::TRACKED.iter().copied().collect();
    assert!(tracked.contains("controlled_dribble_sprint_share"));
    assert!(tracked.contains("controlled_dribble_touches_per_min"));
    assert!(tracked.contains("ai_dribble_sprint_share"));
    assert!(tracked.contains("ai_dribble_touches_per_min"));
}

#[test]
fn tripwire_passes_when_the_signature_matches_the_baseline() {
    let sig = signature(None, None);
    let (ok, rows) = tripwire::compare(&sig, &sig);
    assert!(ok, "identical signatures pass");
    assert_eq!(rows.len(), tripwire::TRACKED.len());
    for r in &rows {
        assert!(r.ok, "{} row ok", r.key);
    }
}

#[test]
fn tripwire_tolerates_drift_inside_the_tolerance_band() {
    // fun baseline is 0.5 here; 5% = 0.025 tolerance.
    let (ok, _) = tripwire::compare(&signature(None, None), &signature(Some(0.02), Some("fun")));
    assert!(ok, "sub-tolerance drift passes");
}

#[test]
fn tripwire_fails_when_one_metric_drifts_beyond_tolerance() {
    let (ok, rows) = tripwire::compare(&signature(None, None), &signature(Some(0.1), Some("fun")));
    assert!(!ok, "a drifted metric fails the whole check");
    let mut drifted = 0;
    for r in &rows {
        if !r.ok {
            drifted += 1;
            assert_eq!(r.key, "fun");
        }
    }
    assert_eq!(drifted, 1, "only the drifted metric is flagged");
}

#[test]
fn tripwire_reports_drift_rows_and_refresh_instructions() {
    let (ok, rows) = tripwire::compare(&signature(None, None), &signature(Some(0.1), Some("fun")));
    let rep = tripwire::report(&rows, ok, 30);
    assert!(rep.contains("DRIFT"), "report names the drift");
    assert!(
        rep.contains("gc_sim::tripwire::serialize"),
        "and how to refresh"
    );
    assert!(
        rep.contains("fun_baseline.rs"),
        "names the file the refresh lands in"
    );
}

#[test]
fn tripwire_serializes_a_loadable_baseline_covering_every_tracked_metric() {
    let sig = signature(None, None);
    let chunk = tripwire::serialize(&sig, 30);

    // A tiny loader for the generated Rust struct-literal text: pull the
    // `pub const BASELINE: FunBaseline = FunBaseline { ... };` body out of
    // the emitted module source and read its `field: value,` rows. This is
    // what a human pasting the output over fun_baseline.rs is trusting to
    // parse as real Rust.
    let open = "FunBaseline {";
    let start = chunk.find(open).expect("emits a FunBaseline literal");
    let after_open = &chunk[start + open.len()..];
    let close = after_open.find("};").expect("literal is closed");
    let body = &after_open[..close];

    let mut loaded: IndexMap<String, f64> = IndexMap::new();
    let mut n: Option<i64> = None;
    for line in body.lines() {
        let line = line.trim().trim_end_matches(',');
        if line.is_empty() {
            continue;
        }
        let (key, value) = line.split_once(':').expect("`field: value` row");
        let key = key.trim();
        let value = value.trim();
        if key == "n" {
            n = value.parse::<i64>().ok();
        } else {
            loaded.insert(
                key.to_string(),
                value.parse::<f64>().expect("numeric field"),
            );
        }
    }

    assert_eq!(n, Some(30));
    for &k in tripwire::TRACKED {
        let got = *loaded.get(k).unwrap_or_else(|| panic!("{k} round-trips"));
        let want = sig[k];
        assert!(
            (got - want).abs() < 1e-5,
            "{k} round-trips: got {got}, want {want}"
        );
    }
    assert_eq!(
        loaded.len(),
        tripwire::TRACKED.len(),
        "no extra, unrecognized fields snuck in"
    );

    // "Loadable" means the emitted field names are real fields of
    // `gc_data::fun_baseline::FunBaseline`, not merely text shaped like
    // some. Assigning into an actual value through an exhaustive match on
    // field name is compile-checked realness: a name with no matching real
    // field has no arm to take, so it falls into the panicking catch-all.
    let mut baseline = gc_data::fun_baseline::FunBaseline {
        n: 0,
        fun: 0.0,
        goals_total: 0.0,
        shots_per_goal: 0.0,
        save_rate: 0.0,
        pass_completion: 0.0,
        turnovers_per_min: 0.0,
        possession_balance: 0.0,
        longest_drought_s: 0.0,
        decided_late: 0.0,
        controlled_dribble_close_share: 0.0,
        controlled_dribble_sprint_share: 0.0,
        controlled_dribble_juke_share: 0.0,
        controlled_dribble_touches_per_min: 0.0,
        controlled_dribble_heavy_losses_per_min: 0.0,
        ai_dribble_close_share: 0.0,
        ai_dribble_sprint_share: 0.0,
        ai_dribble_juke_share: 0.0,
        ai_dribble_touches_per_min: 0.0,
        ai_dribble_heavy_losses_per_min: 0.0,
    };
    baseline.n = n.expect("n present");
    for (key, value) in &loaded {
        match key.as_str() {
            "fun" => baseline.fun = *value,
            "goals_total" => baseline.goals_total = *value,
            "shots_per_goal" => baseline.shots_per_goal = *value,
            "save_rate" => baseline.save_rate = *value,
            "pass_completion" => baseline.pass_completion = *value,
            "turnovers_per_min" => baseline.turnovers_per_min = *value,
            "possession_balance" => baseline.possession_balance = *value,
            "longest_drought_s" => baseline.longest_drought_s = *value,
            "decided_late" => baseline.decided_late = *value,
            "controlled_dribble_close_share" => baseline.controlled_dribble_close_share = *value,
            "controlled_dribble_sprint_share" => {
                baseline.controlled_dribble_sprint_share = *value;
            }
            "controlled_dribble_juke_share" => baseline.controlled_dribble_juke_share = *value,
            "controlled_dribble_touches_per_min" => {
                baseline.controlled_dribble_touches_per_min = *value;
            }
            "controlled_dribble_heavy_losses_per_min" => {
                baseline.controlled_dribble_heavy_losses_per_min = *value;
            }
            "ai_dribble_close_share" => baseline.ai_dribble_close_share = *value,
            "ai_dribble_sprint_share" => baseline.ai_dribble_sprint_share = *value,
            "ai_dribble_juke_share" => baseline.ai_dribble_juke_share = *value,
            "ai_dribble_touches_per_min" => baseline.ai_dribble_touches_per_min = *value,
            "ai_dribble_heavy_losses_per_min" => {
                baseline.ai_dribble_heavy_losses_per_min = *value;
            }
            other => panic!("serialize emitted a field {other:?} that isn't real"),
        }
    }
    assert_eq!(baseline.n, 30);
}
