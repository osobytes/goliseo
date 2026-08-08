//! Port of `spec/sim/tripwire_spec.lua`.

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
    assert!(rep.contains("tripwire write"), "and how to refresh");
}

#[test]
fn tripwire_serializes_a_loadable_baseline_covering_every_tracked_metric() {
    let sig = signature(None, None);
    let chunk = tripwire::serialize(&sig, 30);

    // A tiny loader for the generated Lua-table-literal text: pull out
    // `key = value,` lines and `n = value,`, matching what `love`'s Lua
    // parser would see when it loads this file.
    let mut loaded: IndexMap<String, f64> = IndexMap::new();
    let mut n: Option<i64> = None;
    for line in chunk.lines() {
        let line = line.trim().trim_end_matches(',');
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if key == "n" {
            n = value.parse::<i64>().ok();
        } else if let Ok(v) = value.parse::<f64>() {
            loaded.insert(key.to_string(), v);
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
}
