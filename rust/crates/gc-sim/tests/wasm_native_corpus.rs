//! Native half of #517's seeded native-vs-wasm differential.
//!
//! ## What this file gates
//!
//! [`gc_sim::wasm_native_corpus::CORPUS`]'s scenarios must (a) actually
//! reach the sites the module doc claims they reach — empirically, not
//! merely by argument, see [`every_targeted_site_is_reached_by_the_corpus`]
//! — and (b) be deterministic on THIS side before a comparison against the
//! wasm side can mean anything at all, see
//! [`every_scenario_is_deterministic_across_two_independent_native_runs`].
//!
//! Neither of these needs the wasm module. The actual cross-target
//! comparison — the point of #517 — is
//! `scripts/check_wasm_native_corpus.mjs`, run from `scripts/check.sh`. It
//! drives the compiled wasm module through the SAME [`CORPUS`] (read via
//! `gc-wasm`'s `corpusScenarios()` export, so there is exactly one scenario
//! list, not a second copy that could drift) and diffs each scenario's
//! per-tick hashes against [`dump_corpus_tick_hashes`]'s native output.
//!
//! ## Why the cross-target comparison is not pinned here
//!
//! [`crate::ai_driven_evidence`]'s single fixture pins a `final_hash`/
//! `sequence_digest` pair once and compares both targets against it forever.
//! That works for one frozen scenario. A CORPUS exists specifically to be
//! read on a red run ("which scenario, which tick"), and pinning eight
//! final/sequence digests here would only prove "native still agrees with a
//! number captured once" — not "native agrees with THIS wasm build, right
//! now," which is the actual property #517 is about. So instead of a pinned
//! table, [`dump_corpus_tick_hashes`] below is a `--ignored` printer:
//! `scripts/check_wasm_native_corpus.mjs` runs it fresh on every gate
//! invocation and diffs its output against a live wasm run, tick for tick.
//! No fixture to go stale, and no re-record ceremony (contrast
//! `gc_sim::determinism_evidence`'s OMP-1 fixture, or
//! `session_ai_driven_lua_reference.txt`) — because unlike those, nothing
//! here is a frozen historical recording; it is the current source, run
//! twice.
use gc_sim::tuning::Tuning;
use gc_sim::wasm_native_corpus::{CORPUS, run_scenario};

/// Every #517 site this corpus targets must be empirically observed by at
/// least one scenario — see `gc_sim::wasm_native_corpus::CORPUS`'s own
/// module doc table for the full site-by-scenario mapping this backs up.
///
/// This is deliberately a hard gate, not merely a comment: if a future
/// gameplay change makes one of these sites unreachable by every scenario in
/// the corpus (a formation change that stops producing aerial duels, an AI
/// change that stops attempting saves in range), this test goes red with the
/// specific site name, rather than the corpus silently stopping being
/// evidence for a site it once covered.
#[test]
fn every_targeted_site_is_reached_by_the_corpus() {
    let tune = Tuning::new();
    let mut any_touch = false;
    let mut any_pass = false;
    let mut any_aerial = false;
    let mut any_keeper_save = false;
    let mut any_combat_commit = false;
    let mut combat_scenarios_ran = 0;

    for scenario in CORPUS {
        let run = run_scenario(&tune, scenario);
        any_touch |= run.coverage.touch;
        any_pass |= run.coverage.pass;
        any_aerial |= run.coverage.aerial;
        any_keeper_save |= run.coverage.keeper_save;
        if scenario.combat_enabled {
            combat_scenarios_ran += 1;
            any_combat_commit |= run.coverage.combat_commit;
        }
        // Sites 1/2/5/6/7a are structural: any scenario with live AI play
        // reaches them (see the module doc table), so require the two
        // cheaply observable proxies on EVERY scenario, not just the union.
        assert!(
            run.coverage.touch,
            "{}: no dribble touch observed -- site 1 (match.rs update_ball) unreached",
            scenario.id
        );
        assert!(
            run.coverage.pass,
            "{}: no pass observed -- sites 2/5/7a unreached",
            scenario.id
        );
    }

    assert!(combat_scenarios_ran > 0, "no scenario enables combat");
    assert!(
        any_touch,
        "no scenario in the corpus produced a dribble touch (site 1)"
    );
    assert!(
        any_pass,
        "no scenario in the corpus produced a pass (sites 2/5/7a)"
    );
    assert!(
        any_aerial,
        "no scenario in the corpus produced an aerial contact -- site 4 (aerial.rs) unreached"
    );
    assert!(
        any_keeper_save,
        "no scenario in the corpus produced a keeper save -- site 7b (match.rs attempt_save) unreached"
    );
    assert!(
        any_combat_commit,
        "no combat-enabled scenario produced a combat commit -- site 3 (combat.rs / \
         combat_feasibility.rs) ran its geometry checks but never confirmed a target, \
         which is weaker evidence than intended"
    );
}

/// Two independent native runs of every corpus scenario, at its full length,
/// must agree tick for tick. This is a precondition, not the claim #517 is
/// about: if native itself were nondeterministic, a wasm-vs-native
/// comparison could not distinguish "the targets disagree" from "this run
/// got unlucky."
#[test]
fn every_scenario_is_deterministic_across_two_independent_native_runs() {
    let tune = Tuning::new();
    for scenario in CORPUS {
        let first = run_scenario(&tune, scenario);
        let second = run_scenario(&tune, scenario);
        assert_eq!(
            first.tick_hashes, second.tick_hashes,
            "{}: two native runs of the same scenario disagreed",
            scenario.id
        );
    }
}

/// Print one line per tick, per scenario, in `scripts/check_wasm_native_corpus.mjs`'s
/// expected format: `GC_CORPUS_TICK|<scenario_id>|<tick>|<hash>`. `--ignored`
/// because this is a data source for that script, not an assertion —
/// exactly like `gc_sim::determinism_evidence`'s recorder tests, it never
/// runs inside an ordinary `cargo test`.
///
/// Run with:
/// `cargo test -p gc-sim --test wasm_native_corpus -- --ignored --nocapture dump_corpus_tick_hashes`
#[test]
#[ignore]
fn dump_corpus_tick_hashes() {
    let tune = Tuning::new();
    for scenario in CORPUS {
        let run = run_scenario(&tune, scenario);
        for (tick, hash) in run.tick_hashes.iter().enumerate() {
            println!("GC_CORPUS_TICK|{}|{tick}|{hash}", run.scenario_id);
        }
        println!(
            "GC_CORPUS_DONE|{}|{}|{}|{}",
            run.scenario_id, run.ticks, run.final_hash, run.sequence_digest
        );
    }
}
