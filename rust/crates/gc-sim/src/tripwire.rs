//! Port of `sim/tripwire.lua`.
//!
//! Fun-signature tripwire: a fast, deterministic snapshot of the fun-proxy
//! metrics over a fixed seed set, compared against a checked-in baseline
//! (`data::fun_baseline`). Any sim change that moves the signature beyond
//! tolerance FAILS check.sh — the point is not to forbid drift but to force
//! the ritual: confirm the drift is intended, re-run the sim benchmark, log
//! the shift in `docs/design/fun_metrics.md`'s drift log, then refresh the
//! baseline.
//!
//! Pure module: measurement and comparison only. File reading/writing is a
//! caller concern (`sim/` does no I/O).

use crate::headless;
use indexmap::IndexMap;

/// Seeds in the snapshot. Small enough to keep the check quick, large enough
/// that the signature reflects play, not one match's quirks. The run is
/// deterministic per seed, so this is a snapshot, not a sample — tolerance
/// expresses "drift a human should acknowledge", not statistical noise.
pub const DEFAULT_N: i64 = 30;

/// The signature: the composite, every banded metric, and normalized
/// dribble diagnostics that make proxy-vs-team-AI tool parity visible
/// (means).
pub const TRACKED: &[&str] = &[
    "fun",
    "goals_total",
    "shots_per_goal",
    "save_rate",
    "pass_completion",
    "turnovers_per_min",
    "possession_balance",
    "longest_drought_s",
    "decided_late",
    "controlled_dribble_close_share",
    "controlled_dribble_sprint_share",
    "controlled_dribble_juke_share",
    "controlled_dribble_touches_per_min",
    "controlled_dribble_heavy_losses_per_min",
    "ai_dribble_close_share",
    "ai_dribble_sprint_share",
    "ai_dribble_juke_share",
    "ai_dribble_touches_per_min",
    "ai_dribble_heavy_losses_per_min",
];

// Allowed drift per metric: 5% of the baseline magnitude, floored so
// near-zero baselines don't demand impossible precision.
const REL_TOL: f64 = 0.05;
const ABS_TOL_FLOOR: f64 = 0.015;

fn tolerance(base: f64) -> f64 {
    (base.abs() * REL_TOL).max(ABS_TOL_FLOOR)
}

/// A fun-signature: one mean value per [`TRACKED`] metric name.
pub type Signature = IndexMap<&'static str, f64>;

/// Run the snapshot batch and flatten to `{metric = mean}`.
///
/// `n` defaults to [`DEFAULT_N`].
#[must_use]
pub fn measure(n: Option<i64>) -> (Signature, i64) {
    let n = n.unwrap_or(DEFAULT_N);
    let batch = headless::run_batch(&headless::BatchOpts {
        n: Some(n),
        ..Default::default()
    });
    let mut out = IndexMap::new();
    for &key in TRACKED {
        let mean = batch.agg.get(key).map_or(0.0, |a| a.mean);
        out.insert(key, mean);
    }
    (out, n)
}

/// One tracked metric's comparison against the baseline.
#[derive(Clone, Debug, PartialEq)]
pub struct TripwireRow {
    /// The metric name (one of [`TRACKED`]).
    pub key: &'static str,
    /// The checked-in baseline value.
    pub base: f64,
    /// The freshly measured value.
    pub cur: f64,
    /// `cur - base`.
    pub delta: f64,
    /// The allowed drift for this row.
    pub tol: f64,
    /// Whether `delta` is within `tol`.
    pub ok: bool,
}

/// Compare a measured signature against the baseline table. Keys missing
/// from either table read as `0.0`, mirroring the Lua original's
/// `table[key] or 0`.
#[must_use]
pub fn compare(baseline: &Signature, current: &Signature) -> (bool, Vec<TripwireRow>) {
    let mut ok = true;
    let mut rows = Vec::with_capacity(TRACKED.len());
    for &key in TRACKED {
        let base = baseline.get(key).copied().unwrap_or(0.0);
        let cur = current.get(key).copied().unwrap_or(0.0);
        let tol = tolerance(base);
        let row_ok = (cur - base).abs() <= tol;
        ok = ok && row_ok;
        rows.push(TripwireRow {
            key,
            base,
            cur,
            delta: cur - base,
            tol,
            ok: row_ok,
        });
    }
    (ok, rows)
}

/// Render a human-readable comparison report.
#[must_use]
pub fn report(rows: &[TripwireRow], ok: bool, n: i64) -> String {
    let mut lines = vec![
        format!("fun tripwire: {n} seeded matches vs data::fun_baseline"),
        format!(
            "{:<20} {:>9} {:>9} {:>9} {:>7}",
            "metric", "base", "now", "delta", "tol"
        ),
    ];
    for r in rows {
        lines.push(format!(
            "{:<20} {:>9.3} {:>9.3} {:>+9.3} {:>7.3}  {}",
            r.key,
            r.base,
            r.cur,
            r.delta,
            r.tol,
            if r.ok { "ok" } else { "DRIFT" }
        ));
    }
    if ok {
        lines.push("TRIPWIRE OK".to_string());
    } else {
        lines.push("TRIPWIRE DRIFT — if intended: re-run the sim benchmark, log the".to_string());
        lines
            .push("shift in docs/design/fun_metrics.md (drift log), then refresh with".to_string());
        lines.push("`tripwire write`.".to_string());
    }
    lines.join("\n")
}

/// Serialize a signature as loadable baseline source text (mirrors the Lua
/// `data/fun_baseline.lua` file format). Stable order, regenerable.
#[must_use]
pub fn serialize(current: &Signature, n: i64) -> String {
    let mut lines = vec![
        "-- Fun-signature tripwire baseline. REGENERATE, don't hand-edit.".to_string(),
        "-- ...and only after confirming the drift is intended, re-running".to_string(),
        "-- the sim benchmark, and logging the shift in the drift log of".to_string(),
        "-- docs/design/fun_metrics.md.".to_string(),
        "return {".to_string(),
        format!("    n = {n},"),
    ];
    for &key in TRACKED {
        let v = current.get(key).copied().unwrap_or(0.0);
        lines.push(format!("    {key} = {v:.6},"));
    }
    lines.push("}".to_string());
    lines.push(String::new());
    lines.join("\n")
}
