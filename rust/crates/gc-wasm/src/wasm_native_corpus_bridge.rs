//! Runs #517's seeded native-vs-wasm differential corpus
//! (`gc_sim::wasm_native_corpus`) from inside the compiled wasm module.
//!
//! [`crate::ai_driven`]/[`crate::determinism`] already prove the wasm build
//! reproduces ONE frozen scenario. This binding exists because #517's own
//! scoping comment found that a single scenario is not evidence about the
//! other twelve `sin`/`cos`/`exp`/`ln` sites it never happens to reach —
//! "which transcendental calls fire, with which arguments, moves with the
//! trajectory." `scripts/check_wasm_native_corpus.mjs` is the caller: it
//! reads [`corpus_scenarios`] so the scenario LIST has exactly one source of
//! truth (`gc_sim::wasm_native_corpus::CORPUS`, never a second copy
//! hardcoded in TypeScript), runs each through [`run_corpus_scenario`], and
//! diffs the result's `tick_hashes` against a fresh native run of the
//! identical scenario — see that script and
//! `crates/gc-sim/tests/wasm_native_corpus.rs`'s module doc for why the
//! comparison is live rather than pinned.
//!
//! Control-surface binding (called a handful of times per gate run, not per
//! frame), so this uses `wasm-bindgen` rather than
//! [`crate::render_export`]'s raw pointer convention — the same choice
//! [`crate::ai_driven`]/[`crate::determinism`] made for the same reason.

use gc_sim::tuning::Tuning;
use gc_sim::wasm_native_corpus::{self, CorpusScenario};
use wasm_bindgen::prelude::*;

/// One [`gc_sim::wasm_native_corpus::CorpusScenario`], exposed to JS so a
/// caller can drive the identical scenario [`run_corpus_scenario`] expects —
/// see [`corpus_scenarios`].
#[wasm_bindgen]
pub struct CorpusScenarioInfo {
    #[wasm_bindgen(getter_with_clone)]
    /// The scenario's stable identity.
    pub id: String,
    /// The match's own RNG seed.
    pub match_seed: f64,
    /// The local slot's bot seed.
    pub bot_seed: f64,
    /// Ticks to simulate.
    pub ticks: f64,
    /// Whether this scenario builds a combat companion.
    pub combat_enabled: bool,
}

/// The corpus's own scenario list, read directly from
/// [`gc_sim::wasm_native_corpus::CORPUS`] — so a caller drives the SAME
/// scenarios the native side defines, rather than maintaining a second,
/// driftable copy of scenario ids/seeds/tick counts in TypeScript. Per
/// AGENTS.md §8, this list is data a caller receives, not content it
/// restates.
#[wasm_bindgen(js_name = corpusScenarios)]
#[must_use]
pub fn corpus_scenarios() -> Vec<CorpusScenarioInfo> {
    wasm_native_corpus::CORPUS
        .iter()
        .map(|scenario| CorpusScenarioInfo {
            id: scenario.id.to_string(),
            match_seed: scenario.match_seed,
            bot_seed: scenario.bot_seed,
            ticks: scenario.ticks as f64,
            combat_enabled: scenario.combat_enabled,
        })
        .collect()
}

/// The result of one [`run_corpus_scenario`] call, exposed to JS as a plain
/// object with getters. Mirrors [`gc_sim::wasm_native_corpus::CorpusRun`].
#[wasm_bindgen]
pub struct CorpusRunResult {
    #[wasm_bindgen(getter_with_clone)]
    /// The scenario's own stable identity.
    pub scenario_id: String,
    /// Ticks simulated.
    pub ticks: f64,
    #[wasm_bindgen(getter_with_clone)]
    /// One FNV-1a-64 hash per row, tick 0 through `ticks` inclusive — see
    /// [`gc_sim::wasm_native_corpus::CorpusRun::tick_hashes`]'s doc for why
    /// this is a vector and not only a final/sequence digest: it is what
    /// lets a caller name the exact tick two runs diverged at, in a single
    /// pass, with no bisection needed.
    pub tick_hashes: Vec<String>,
    #[wasm_bindgen(getter_with_clone)]
    /// The final row's hash (`tick_hashes`'s last entry).
    pub final_hash: String,
    #[wasm_bindgen(getter_with_clone)]
    /// FNV-1a-64 folded over `tick_hashes` in order.
    pub sequence_digest: String,
    /// Final home score.
    pub score_home: f64,
    /// Final away score.
    pub score_away: f64,
    /// Whether this run observed a dribble touch (site 1).
    pub covered_touch: bool,
    /// Whether this run observed a shot.
    pub covered_shot: bool,
    /// Whether this run observed a pass (sites 2/5/7a).
    pub covered_pass: bool,
    /// Whether this run observed a pass reception.
    pub covered_reception: bool,
    /// Whether this run observed a tackle.
    pub covered_tackle: bool,
    /// Whether this run observed a gameplay-AI combat commit (site 3).
    pub covered_combat_commit: bool,
    /// Whether this run observed an aerial contact (site 4).
    pub covered_aerial: bool,
    /// Whether this run observed a keeper save (site 7b).
    pub covered_keeper_save: bool,
    /// The final tick's ball state: `[x, y, vel_x, vel_y, z, vz]` — see
    /// [`gc_sim::wasm_native_corpus::CorpusRun::last_row`]'s doc. Exists for
    /// ATTRIBUTION: a caller comparing two `runCorpusScenario` calls at
    /// `ticks` and `ticks - 1` on both targets can diff this (and
    /// `last_players`) to find which scalar first changed, rather than only
    /// knowing THAT the scenario diverged.
    #[wasm_bindgen(getter_with_clone)]
    pub last_ball: Vec<f64>,
    /// The final tick's ball owner (player index, or `-1`).
    pub last_owner: f64,
    /// The final tick's match RNG state.
    pub last_rng: f64,
    /// The final tick's player positions, flattened `[x0, y0, x1, y1, ...]`
    /// in roster order (ten players).
    #[wasm_bindgen(getter_with_clone)]
    pub last_players: Vec<f64>,
}

/// Run one corpus scenario — by its own parameters, not merely its id, so a
/// caller that already fetched [`corpus_scenarios`] does not need a second
/// lookup — inside this compiled wasm module and return its digested result.
///
/// # Panics
///
/// Panics if `ticks` is negative or not finite — the same invariant
/// [`gc_sim::wasm_native_corpus::run_scenario`]'s own `for tick in 1..=ticks`
/// loop assumes.
#[wasm_bindgen(js_name = runCorpusScenario)]
#[must_use]
pub fn run_corpus_scenario(
    id: &str,
    match_seed: f64,
    bot_seed: f64,
    ticks: f64,
    combat_enabled: bool,
) -> CorpusRunResult {
    assert!(
        ticks.is_finite() && ticks >= 0.0,
        "runCorpusScenario: ticks must be a finite, non-negative number"
    );
    let scenario = CorpusScenario {
        id: Box::leak(id.to_string().into_boxed_str()),
        match_seed,
        bot_seed,
        ticks: ticks as i64,
        combat_enabled,
    };
    let run = wasm_native_corpus::run_scenario(&Tuning::new(), &scenario);
    CorpusRunResult {
        scenario_id: run.scenario_id.to_string(),
        ticks: run.ticks as f64,
        tick_hashes: run.tick_hashes,
        final_hash: run.final_hash,
        sequence_digest: run.sequence_digest,
        score_home: run.score_home as f64,
        score_away: run.score_away as f64,
        covered_touch: run.coverage.touch,
        covered_shot: run.coverage.shot,
        covered_pass: run.coverage.pass,
        covered_reception: run.coverage.reception,
        covered_tackle: run.coverage.tackle,
        covered_combat_commit: run.coverage.combat_commit,
        covered_aerial: run.coverage.aerial,
        covered_keeper_save: run.coverage.keeper_save,
        last_ball: run.last_row.ball.to_vec(),
        last_owner: run.last_row.owner as f64,
        last_rng: f64::from(run.last_row.rng),
        last_players: run
            .last_row
            .players
            .iter()
            .flat_map(|&(x, y)| [x, y])
            .collect(),
    }
}
