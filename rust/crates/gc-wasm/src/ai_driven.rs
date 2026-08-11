//! Run the AI-driven reference match from inside the compiled wasm module and
//! hand its digests back to JS.
//!
//! [`crate::determinism`] already does this for the OMP-1 campaign — an IDLE
//! match. This is its counterpart for a match in which every player, including
//! the one on the human-input branch, is driven by an AI: shooting, charging,
//! passing, lofting, dashing, dodging, jockeying, sprinting, and the lossy
//! input-frame quantisation that only does anything once a player actually
//! steers. See `gc_sim::ai_driven_evidence`.
//!
//! WHY THIS EXISTS AS A WASM ENTRY POINT AT ALL. `gc-sim`'s own differential
//! proves the SOURCE reproduces Lua. On 2026-08-07 that was true all day while
//! the browser rendered an obviously different game, because the browser was
//! executing a wasm artifact built before the fix — a build, not a source,
//! problem, and one no amount of native testing can see. Together with
//! `scripts/check_v2.sh`'s byte comparison between this module and the one the
//! app bundle ships, this closes that gap: the digests below are pinned to the
//! Lua capture, so a green check here means the code in the browser reproduces
//! Lua, not merely that some code somewhere did.
//!
//! Control-surface binding, called once rather than per frame, so it uses
//! `wasm-bindgen` rather than [`crate::render_export`]'s raw pointer
//! convention.

use gc_sim::ai_driven_evidence;
use gc_sim::tuning::Tuning;
use wasm_bindgen::prelude::*;

/// The result of one [`run_ai_driven_evidence`] call, exposed to JS as a plain
/// object with getters. Mirrors
/// [`gc_sim::ai_driven_evidence::AiDrivenEvidenceResult`]; the integer counts
/// cross as `f64` because that is the only number type the JS boundary has.
#[wasm_bindgen]
pub struct AiDrivenEvidence {
    #[wasm_bindgen(getter_with_clone)]
    /// The scenario's stable identity, so two different campaigns' digests
    /// cannot be compared and believed to agree.
    pub fixture_id: String,
    /// Ticks simulated.
    pub ticks: f64,
    /// Rows digested (`ticks + 1`; tick 0 is recorded before the first step).
    pub rows: f64,
    #[wasm_bindgen(getter_with_clone)]
    /// FNV-1a-64 over the final row's fields, lowercase hex.
    pub final_hash: String,
    #[wasm_bindgen(getter_with_clone)]
    /// FNV-1a-64 over every row in sequence, lowercase hex. Catches a
    /// divergence that self-corrects before the final tick, which
    /// `final_hash` alone would not.
    pub sequence_digest: String,
    /// Final home score.
    pub score_home: f64,
    /// Final away score.
    pub score_away: f64,
}

/// Replay the AI-driven reference match inside this wasm module and return its
/// digests.
///
/// The caller compares `final_hash` and `sequence_digest` against the
/// constants `gc-sim`'s `tests/ai_driven_evidence.rs` derives from the Lua
/// capture — this function makes no claim about what they should be, exactly
/// as [`crate::determinism::run_determinism_evidence`] makes none about the
/// OMP-1 pair.
#[wasm_bindgen(js_name = runAiDrivenEvidence)]
#[must_use]
pub fn run_ai_driven_evidence() -> AiDrivenEvidence {
    run_ai_driven_evidence_to(ai_driven_evidence::TICKS as f64)
}

/// [`run_ai_driven_evidence`], stopped after `ticks` ticks -- see
/// [`gc_sim::ai_driven_evidence::run_to`] for why bisection needs to cross the
/// wasm boundary rather than run only natively.
#[wasm_bindgen(js_name = runAiDrivenEvidenceTo)]
#[must_use]
pub fn run_ai_driven_evidence_to(ticks: f64) -> AiDrivenEvidence {
    let result = ai_driven_evidence::run_to(&Tuning::new(), ticks as i64);
    AiDrivenEvidence {
        fixture_id: result.fixture_id,
        ticks: result.ticks as f64,
        rows: result.rows as f64,
        final_hash: result.final_hash,
        sequence_digest: result.sequence_digest,
        score_home: result.score_home as f64,
        score_away: result.score_away as f64,
    }
}
