//! The wave's acceptance surface: run
//! [`gc_sim::determinism_evidence::verify`] from inside the compiled wasm
//! module and hand its digests back to JS.
//!
//! This is control-surface binding (called once, not per frame), so it uses
//! `wasm-bindgen` rather than [`crate::render_export`]'s raw pointer
//! convention.

use gc_sim::determinism_evidence::{self, Outcome};
use gc_sim::tuning::Tuning;
use wasm_bindgen::prelude::*;

/// The result of one [`run_determinism_evidence`] call, exposed to JS as a
/// plain object with getters. Every field mirrors
/// [`gc_sim::determinism_evidence::DeterminismEvidenceResult`]; `outcome` is
/// rendered to its wire string (`"home"` / `"away"` / `"draw"`) since
/// `wasm-bindgen` has no direct binding for a bare Rust enum.
#[wasm_bindgen]
pub struct DeterminismEvidence {
    #[wasm_bindgen(getter_with_clone)]
    /// The fixture's stable identity.
    pub fixture_id: String,
    /// Ticks simulated.
    pub ticks: f64,
    /// Boundaries recorded (`ticks + 1`).
    pub boundaries: f64,
    #[wasm_bindgen(getter_with_clone)]
    /// The final canonical snapshot hash, lowercase hex.
    pub final_hash: String,
    #[wasm_bindgen(getter_with_clone)]
    /// The FNV-1a-64 digest over every boundary hash in sequence, lowercase
    /// hex.
    pub sequence_digest: String,
    /// Final home score.
    pub score_home: f64,
    /// Final away score.
    pub score_away: f64,
    #[wasm_bindgen(getter_with_clone)]
    /// `"home"`, `"away"` or `"draw"`.
    pub outcome: String,
    /// Encoded byte length of the final canonical snapshot.
    pub snapshot_bytes: f64,
}

/// Run a complete OMP-1 determinism campaign against the frozen 7,201-tick
/// fixture, exactly as `gc_sim::determinism_evidence::verify` does natively,
/// and return its evidence. The caller compares `final_hash` and
/// `sequence_digest` against the pinned native-build digests; this function
/// makes no claim about what they should be — see this crate's report for
/// what was actually observed under wasm.
///
/// # Errors
///
/// Returns a `JsValue` (a `String`) if the campaign's own internal
/// consistency checks fail — see
/// [`gc_sim::determinism_evidence::verify`]'s doc.
#[wasm_bindgen(js_name = runDeterminismEvidence)]
pub fn run_determinism_evidence() -> Result<DeterminismEvidence, JsValue> {
    let tune = Tuning::new();
    let result = determinism_evidence::verify(&tune).map_err(|err| JsValue::from_str(&err))?;
    let outcome = match result.outcome {
        Outcome::Home => "home",
        Outcome::Away => "away",
        Outcome::Draw => "draw",
    };
    Ok(DeterminismEvidence {
        fixture_id: result.fixture_id,
        ticks: result.ticks as f64,
        boundaries: result.boundaries as f64,
        final_hash: result.final_hash,
        sequence_digest: result.sequence_digest,
        score_home: result.score_home as f64,
        score_away: result.score_away as f64,
        outcome: outcome.to_string(),
        snapshot_bytes: result.snapshot_bytes as f64,
    })
}
