//! Knob-space sweep and coordinate-ascent search over headless batches.
//!
//! Knob-space exploration over headless batches: per-knob sensitivity
//! sweeps and greedy coordinate ascent toward higher fun scores. Pure — no
//! I/O; long runs report progress through an injected `log` callback and
//! every result comes back as data + a formatted report string.
//!
//! Statistics: every config is evaluated on the SAME seed set (common
//! random numbers), so config effects are paired per seed — deltas are mean
//! paired difference +/- their standard error, not a comparison of noisy
//! means.
//!
//! ## Tuning is an owned value here too
//!
//! [`crate::tuning::Tuning`] is an explicit, owned value, not a global
//! singleton (see that module's doc, and `crate::headless`'s restated
//! version of the same point) — every [`evaluate`] call builds a fresh
//! `Tuning` deep in `crate::headless::run_match`, so there is no global left
//! to perturb or restore between configs (see `tests/sweep.rs` for the
//! coverage of that guarantee).
//!
//! ## Private-helper duplication
//!
//! [`crate::tuning::Tuning`]'s knob-line parser and `%.6g` formatter are
//! private to that module, and this module doesn't own it, so [`parse_blob`]
//! and `blob_of`'s formatter reimplement the same two small grammars here —
//! the same duplication `crate::headless`'s `band_for`/`format_g` already
//! document for the same reason.

use crate::headless;
use crate::metrics::MetricStats;
use crate::tunable_registry;
use crate::tuning::{self, Knob};
use indexmap::IndexMap;

/// One tuning config's evaluation: the aggregated batch and the raw
/// per-seed fun scores (seed order) paired deltas are computed from.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigEval {
    /// The serialized tuning blob this config ran with.
    pub blob: String,
    /// Per-metric distribution stats across the batch.
    pub agg: IndexMap<&'static str, MetricStats>,
    /// Per-seed fun scores, seed order.
    pub funs: Vec<f64>,
}

/// A paired-seed mean difference and its standard error.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PairedDelta {
    /// Mean per-seed difference (config - reference).
    pub mean: f64,
    /// Standard error of that mean.
    pub se: f64,
}

fn format_g(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs().log10().floor() as i32;
    let decimals = (5 - magnitude).clamp(0, 17) as usize;
    let mut s = format!("{value:.decimals$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

fn blob_of(overrides: &IndexMap<&'static str, f64>) -> String {
    let mut lines = Vec::new();
    // Registry order: stable blobs.
    for k in tuning::KNOBS.iter() {
        if let Some(&v) = overrides.get(k.key)
            && v != k.default
        {
            lines.push(format!("{}={}", k.key, format_g(v)));
        }
    }
    lines.join("\n")
}

/// Grammar mirrors `crate::tuning::Tuning::deserialize`'s line format:
/// `^([%w_]+)=([%-%d%.eE]+)$`.
fn parse_knob_line(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    let (key, rest) = (&line[..eq], &line[eq + 1..]);
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    if rest.is_empty()
        || !rest
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == 'e' || c == 'E')
    {
        return None;
    }
    Some((key, rest))
}

/// Parse a serialized tuning blob back into a knob-key -> value table (the
/// inverse of `blob_of`, same line format as [`crate::tuning`]). Unknown
/// keys are skipped.
#[must_use]
pub fn parse_blob(blob: &str) -> IndexMap<&'static str, f64> {
    let mut overrides = IndexMap::new();
    for line in blob.split(['\r', '\n']) {
        if line.is_empty() {
            continue;
        }
        if let Some((key, num)) = parse_knob_line(line)
            && let Ok(v) = num.parse::<f64>()
            && let Some(k) = tuning::KNOBS.iter().find(|k| k.key == key)
        {
            overrides.insert(k.key, v);
        }
    }
    overrides
}

/// Run one tuning config over `seeds` and fold it into a [`ConfigEval`].
/// `duration` shortens matches for tests; `None` is the real 120 s.
#[must_use]
pub fn evaluate(blob: &str, seeds: &[f64], duration: Option<f64>) -> ConfigEval {
    let batch = headless::run_batch(&headless::BatchOpts {
        seeds: Some(seeds),
        tuning_blob: Some(blob),
        duration,
        ..Default::default()
    });
    let funs = batch
        .matches
        .iter()
        .map(|r| r.metrics.fun.unwrap_or(0.0))
        .collect();
    ConfigEval {
        blob: blob.to_string(),
        agg: batch.agg,
        funs,
    }
}

/// Paired per-seed mean difference between `funs` and `reference`, plus its
/// standard error (`0` for `n <= 1`, where the spread is undefined).
#[must_use]
pub fn paired_delta(reference: &[f64], funs: &[f64]) -> PairedDelta {
    let n = funs.len();
    let mut diffs = Vec::with_capacity(n);
    let mut sum = 0.0;
    for i in 0..n {
        let d = funs[i] - reference[i];
        diffs.push(d);
        sum += d;
    }
    let mean = sum / n as f64;
    let mut var = 0.0;
    for &d in &diffs {
        var += (d - mean).powi(2);
    }
    let se = if n > 1 {
        (var / (n as f64 - 1.0) / n as f64).sqrt()
    } else {
        0.0
    };
    PairedDelta { mean, se }
}

/// One knob's sensitivity: paired deltas at its min and max, plus the goals
/// context each end ran with.
#[derive(Clone, Debug, PartialEq)]
pub struct SensitivityRow {
    /// The knob key.
    pub key: &'static str,
    /// Paired delta at the knob's min.
    pub lo_delta: PairedDelta,
    /// Paired delta at the knob's max.
    pub hi_delta: PairedDelta,
    /// Mean `goals_total` at min (context for the weak metric).
    pub lo_goals: f64,
    /// Mean `goals_total` at max.
    pub hi_goals: f64,
    /// `max(|lo_delta.mean|, |hi_delta.mean|)`: the ranking key.
    pub impact: f64,
}

/// A full sensitivity sweep's result.
#[derive(Clone, Debug, PartialEq)]
pub struct SensitivityResult {
    /// The default-knobs baseline evaluation.
    pub baseline: ConfigEval,
    /// Every swept knob's row, sorted by [`SensitivityRow::impact`],
    /// largest first.
    pub rows: Vec<SensitivityRow>,
}

/// [`sensitivity`]'s options.
#[derive(Debug)]
pub struct SensitivityOpts<'a> {
    /// Seeds every config runs on (common random numbers).
    pub seeds: &'a [f64],
    /// Knobs to sweep; defaults to every knob in registry order.
    pub keys: Option<&'a [&'static str]>,
    /// Shorter matches for tests; `None` = the real 120 s.
    pub duration: Option<f64>,
}

fn agg_mean(eval: &ConfigEval, key: &str) -> f64 {
    eval.agg
        .get(key)
        .unwrap_or_else(|| panic!("{key} is always aggregated by a headless batch"))
        .mean
}

/// Perturb every knob (or `opts.keys`) to its min and max, one at a time,
/// against a defaults baseline.
#[must_use]
pub fn sensitivity(
    opts: &SensitivityOpts<'_>,
    mut log: Option<&mut dyn FnMut(&str)>,
) -> SensitivityResult {
    let owned_keys: Vec<&'static str>;
    let keys: &[&'static str] = match opts.keys {
        Some(k) => k,
        None => {
            // Enumerated from the registry, never a list kept here: a knob a
            // feature registers is swept without an edit to this file, and
            // `sweepable_ids` is tier-1 only and id-sorted, so the sweep can
            // neither reach a presentation value nor depend on the order
            // features happened to register in.
            owned_keys = tunable_registry::shipped().sweepable_ids();
            &owned_keys
        }
    };

    if let Some(log) = log.as_deref_mut() {
        log(&format!(
            "sensitivity: baseline over {} seeds",
            opts.seeds.len()
        ));
    }
    let baseline = evaluate("", opts.seeds, opts.duration);

    let mut rows = Vec::with_capacity(keys.len());
    for (i, &key) in keys.iter().enumerate() {
        let k: &Knob = tuning::KNOBS
            .iter()
            .find(|k| k.key == key)
            .unwrap_or_else(|| panic!("unknown knob: {key}"));
        if let Some(log) = log.as_deref_mut() {
            log(&format!("sensitivity: {}/{} {}", i + 1, keys.len(), key));
        }
        let mut lo_over = IndexMap::new();
        lo_over.insert(key, k.min);
        let lo = evaluate(&blob_of(&lo_over), opts.seeds, opts.duration);
        let mut hi_over = IndexMap::new();
        hi_over.insert(key, k.max);
        let hi = evaluate(&blob_of(&hi_over), opts.seeds, opts.duration);
        let lo_d = paired_delta(&baseline.funs, &lo.funs);
        let hi_d = paired_delta(&baseline.funs, &hi.funs);
        rows.push(SensitivityRow {
            key,
            lo_delta: lo_d,
            hi_delta: hi_d,
            lo_goals: agg_mean(&lo, "goals_total"),
            hi_goals: agg_mean(&hi, "goals_total"),
            impact: lo_d.mean.abs().max(hi_d.mean.abs()),
        });
    }
    rows.sort_by(|a, b| {
        b.impact
            .partial_cmp(&a.impact)
            .expect("impact values are finite")
    });
    SensitivityResult { baseline, rows }
}

/// Render a human-readable sensitivity report.
#[must_use]
pub fn sensitivity_report(r: &SensitivityResult) -> String {
    let base_fun: f64 = r.baseline.funs.iter().sum::<f64>() / r.baseline.funs.len() as f64;
    let mut out = vec![
        format!(
            "sensitivity over {} seeds — baseline fun {:.3}, goals {:.2}",
            r.baseline.funs.len(),
            base_fun,
            agg_mean(&r.baseline, "goals_total")
        ),
        format!(
            "{:<22} {:>8} {:>8} | {:>8} {:>8} | {:>7} {:>7}",
            "knob (min..max)", "dFun@min", "+/-se", "dFun@max", "+/-se", "gls@min", "gls@max"
        ),
    ];
    for row in &r.rows {
        out.push(format!(
            "{:<22} {:>+8.3} {:>8.3} | {:>+8.3} {:>8.3} | {:>7.2} {:>7.2}",
            row.key,
            row.lo_delta.mean,
            row.lo_delta.se,
            row.hi_delta.mean,
            row.hi_delta.se,
            row.lo_goals,
            row.hi_goals
        ));
    }
    out.join("\n")
}

// Evenly spaced candidate values across a knob's range, snapped to its
// step.
fn level_values(k: &Knob, levels: i64) -> Vec<f64> {
    let mut vals: Vec<f64> = Vec::new();
    for i in 0..levels {
        let v = k.min + (k.max - k.min) * i as f64 / (levels - 1) as f64;
        let v = k.min + ((v - k.min) / k.step + 0.5).floor() * k.step;
        let v = v.max(k.min).min(k.max);
        if !vals.contains(&v) {
            vals.push(v);
        }
    }
    vals
}

/// [`ascend`]'s options.
#[derive(Debug)]
pub struct AscentOpts<'a> {
    /// Knobs to sweep, in the order tried each pass.
    pub keys: &'a [&'static str],
    /// Seeds every config runs on (common random numbers).
    pub seeds: &'a [f64],
    /// Values tried per knob per pass; defaults to 5.
    pub levels: Option<i64>,
    /// Coordinate-ascent passes; defaults to 2.
    pub passes: Option<i64>,
    /// Shorter matches for tests; `None` = the real 120 s.
    pub duration: Option<f64>,
    /// Warm-start overrides (e.g. a prior round's candidate).
    pub start: Option<&'a IndexMap<&'static str, f64>>,
}

/// A completed coordinate-ascent search.
#[derive(Clone, Debug, PartialEq)]
pub struct AscentResult {
    /// The winning non-default knob values.
    pub overrides: IndexMap<&'static str, f64>,
    /// The winning config's serialized blob.
    pub blob: String,
    /// The winning config's evaluation on the search seeds.
    pub eval: ConfigEval,
    /// Delta vs the default-knob baseline, search seeds.
    pub delta: PairedDelta,
    /// Accepted moves, in order.
    pub trace: Vec<String>,
}

/// Greedy coordinate ascent on mean fun: sweep each knob through `levels`
/// values, keep any strict improvement, repeat for `passes`. Deterministic
/// (fixed seeds -> same result every run) but greedy — it finds a good
/// ridge, not a proven optimum, and it can overfit the search seeds: always
/// re-check the result on held-out seeds ([`evaluate`] with fresh seeds).
/// `opts.start` warm-starts from known-good overrides; the reported delta
/// stays vs the DEFAULTS baseline either way.
///
/// # Panics
///
/// Panics if `opts.start` or `opts.keys` names an unregistered knob key.
#[must_use]
pub fn ascend(opts: &AscentOpts<'_>, mut log: Option<&mut dyn FnMut(&str)>) -> AscentResult {
    let levels = opts.levels.unwrap_or(5);
    let passes = opts.passes.unwrap_or(2);

    let baseline = evaluate("", opts.seeds, opts.duration);
    let mut overrides: IndexMap<&'static str, f64> = IndexMap::new();
    if let Some(start) = opts.start {
        for (&key, &v) in start {
            assert!(
                tuning::KNOBS.iter().any(|k| k.key == key),
                "unknown start knob: {key}"
            );
            overrides.insert(key, v);
        }
    }
    let mut best = if overrides.is_empty() {
        baseline.clone()
    } else {
        evaluate(&blob_of(&overrides), opts.seeds, opts.duration)
    };
    let mut best_mean: f64 = best.funs.iter().sum::<f64>() / best.funs.len() as f64;
    if let Some(log) = log.as_deref_mut() {
        log(&format!(
            "ascent: start fun {:.3} over {} seeds",
            best_mean,
            opts.seeds.len()
        ));
    }

    let mut trace = Vec::new();
    for pass in 1..=passes {
        for &key in opts.keys {
            let k: &Knob = tuning::KNOBS
                .iter()
                .find(|k| k.key == key)
                .unwrap_or_else(|| panic!("unknown knob: {key}"));
            let mut current = *overrides.get(key).unwrap_or(&k.default);
            for v in level_values(k, levels) {
                if v != current {
                    let mut trial: IndexMap<&'static str, f64> = IndexMap::new();
                    trial.insert(key, v);
                    for (&ok, &ov) in &overrides {
                        if ok != key {
                            trial.insert(ok, ov);
                        }
                    }
                    let eval = evaluate(&blob_of(&trial), opts.seeds, opts.duration);
                    let mean: f64 = eval.funs.iter().sum::<f64>() / eval.funs.len() as f64;
                    if mean > best_mean {
                        best = eval;
                        best_mean = mean;
                        overrides = trial;
                        current = v;
                        let mv = format!("pass {pass}: {key}={} -> fun {mean:.3}", format_g(v));
                        trace.push(mv.clone());
                        if let Some(log) = log.as_deref_mut() {
                            log(&format!("ascent: {mv}"));
                        }
                    }
                }
            }
        }
    }

    let delta = paired_delta(&baseline.funs, &best.funs);
    let blob = blob_of(&overrides);
    AscentResult {
        overrides,
        blob,
        eval: best,
        delta,
        trace,
    }
}
