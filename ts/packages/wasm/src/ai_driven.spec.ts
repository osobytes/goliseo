// Does the COMPILED WASM MODULE reproduce the frozen AI-driven reference
// match?
//
// `determinism.spec.ts` beside this asks the same question for the OMP-1
// campaign -- an IDLE match, where the local player never presses anything.
// This asks it for a match in which every player, including the one on the
// human-input branch, is AI-driven: shooting, charging a shot and releasing
// it, passing, lofting, dashing, dodging, jockeying, sprinting, and the
// input-frame quantisation that is lossless until someone actually steers.
//
// The two digests below are NOT copied from a Rust run. `crates/gc-sim/tests/
// ai_driven_evidence.rs` derives them from `session_ai_driven_lua_reference.txt`,
// a capture of the original Lua implementation's output made before that
// implementation was removed from this repository, and asserts that the
// native Rust replay reproduces them; this file asserts the same of the
// wasm build. So a green result here means the code a browser executes
// reproduces that frozen reference, which is the only claim worth making.
//
// Both digest assertions currently sit behind `it.fails` -- the wasm build does
// NOT reproduce that frozen reference on this scenario. See the block above
// them for the measurements, the bisection and issue #405.

import { describe, expect, it } from "vitest";
import { loadSimHost } from "./index.ts";

const runAiDrivenEvidence = () => loadSimHost().runAiDrivenEvidence();

/** Derived from a historical capture of the original Lua implementation's
 * output, made before it was removed from this repository -- frozen, and
 * cannot be regenerated. See `crates/gc-sim/tests/ai_driven_evidence.rs`. */
// Renamed from LUA_* under #520: these are no longer Lua's digests. They are
// `gc-sim`'s `ai_driven_evidence::EXPECTED_*`, derived by
// `crates/gc-sim/tests/ai_driven_evidence.rs` from a baseline recorded from
// the Rust build, and re-recorded in the same commit as this file. What they
// still gate is the claim that never depended on Lua and is the urgent one
// today: the COMPILED WASM module and the native build produce the same bits
// from the same source. See #517.
//
// Re-recorded for the 2026-08-25 pitch re-dimensioning (960x540 -> 1648x927,
// docs/design/fun_metrics.md's drift log), alongside `session_ai_driven_baseline.txt`
// and `ai_driven_evidence.rs`'s own `EXPECTED_*` constants, in the same commit.
// The frozen input frames are untouched, but the match they replay against is
// a different-sized pitch from tick 0, so this scenario's whole trajectory
// moves, the same reason `determinism.spec.ts`'s OMP-1 pair moved together
// this round.
//
// Re-recorded a second time the same day: LOCO_PACE_REF_HI settled at 280
// (down from the 300 in place when the pair above was first captured), so
// the reference match's whole trajectory moves again.
//
// Re-recorded a third time the same day: the passing knobs were rescaled for
// the 1648x927 pitch (`PASS_RANGE_MIN/MAX`, `PASS_ELIGIBLE_MIN/MAX`,
// `PASS_ARRIVE_PACE`, `PASS_SPEED_MIN/MAX`, `PASS_ANGULAR_WEIGHT` -- see
// `gc_data::tunables`'s own dated note and `gc-sim/tests/passing.rs`). Two
// defects had shipped with the un-rescaled knobs: passes clamped at the
// reach ceiling and died short far more often on the bigger pitch, and the
// receiver-scoring angular term lost enough authority relative to distance
// that a teammate directly behind the passer could outscore one dead on aim.
// A bot-driven match passes constantly, so this scenario's whole trajectory
// moves again, from the first AI pass on -- confirmed against the fixture:
// the recorded baseline and this native replay agree bit for bit from tick 0
// through 29 and diverge starting tick 30.
//
// CHECKED AGAIN, NOT RE-RECORDED, for the #622 follow-up (owner-approved):
// the half-plane aim gate in `gc_sim::passing::select_receiver` ("never
// opposite the aim" is now structural, rejected before scoring, rather than
// arbitrated by weight), `PASS_ANGULAR_WEIGHT` settling at 180 (down from
// the 240 the row above shipped), and deflection-aware lane risk in
// `gc_sim::ai::pass_intercept` (a fast ground pass a body merely reaches
// BLOCKING position for is now a cut lane too, not only a slow one a body
// collects -- `ai::VERSION` 1 -> 2; see `gc_data::tunables`'s own
// 2026-08-25 note and `gc-sim/tests/knob_contract.rs`'s
// `the_shipped_passing_defaults_land_inside_their_proposed_bands` for the
// honest, still-unsettled `pass_aim_error` band verdict this redesign
// produces in general). Each of those three is exactly the class of change
// #405's note above warns moves this scenario's whole trajectory, so it was
// checked rather than assumed: reverting `passing.rs`, `ai.rs` and
// `match.rs`'s `pass_intercept` call site to their committed form while
// leaving every tunable at its current (rescaled) default reproduces the
// IDENTICAL final/sequence pair below, and `record_session_ai_driven_baseline`
// re-run against the full redesign is byte-for-byte identical to the
// checked-in fixture over all 7,201 rows. So this frozen bot-vs-bot match
// never puts a candidate in the half-plane behind the aim, never turns on a
// score tie the weight would break, and never drives a ball fast enough for
// the deflection branch to fire -- the redesign is real, and
// `knob_contract.rs`'s own measurement finds its effect elsewhere, but this
// specific scenario doesn't exercise it. Nothing here moved, and the
// digests, the fixture and `ai_driven_evidence.rs`'s constants are
// deliberately left untouched rather than re-recorded to a value that would
// happen to match -- see this file's own rule against a pinned value moved
// without a stated reason.
//
// THE DISCRIMINATING MEASUREMENT THIS FILE'S OWN HEADER DEMANDS WAS RUN
// FIRST, because a moved digest here is exactly as consistent with #405/#517
// (wasm and native disagreeing) as with a knob-driven trajectory shift --
// wasm, `node -e` against the freshly built `dist/pkg/gc_wasm.cjs`
// (`runAiDrivenEvidence()`, the same export `loadSimHost().runAiDrivenEvidence`
// wraps): final `36d1f260e2b1c9b4`, sequence `54a0f25ab32d86f8`; native, via
// `cargo test -p gc-sim --test ai_driven_evidence`
// (`ai_driven_evidence::EXPECTED_FINAL_HASH`/`EXPECTED_SEQUENCE_DIGEST`), the
// same two. They AGREE, so this is not #517. Re-run identically after the
// #622 follow-up above, against a freshly rebuilt `dist/pkg/gc_wasm.cjs`:
// same wasm pair, same native pair. They still AGREE.
//
// Re-recorded 2026-08-26 for the merge of #628 (the keeper races winnable
// loose balls; the engagement geometry catches up with the futsal box) with
// main's pass-reception/first-touch/juke rework: a bot-driven match passes,
// dribbles, receives and jukes constantly, so either change alone -- let
// alone both landing on this branch together -- is exactly the class of
// change #405's note above warns moves this scenario's whole trajectory.
// Discriminating measurement re-run before these two lines moved -- wasm,
// `node -e` against the freshly rebuilt `dist/pkg/gc_wasm.cjs`
// (`runAiDrivenEvidence()`): final `0291700ae05c7a77`, sequence
// `a471ab55610efef3`; native, via `cargo test -p gc-sim --test
// ai_driven_evidence` (`ai_driven_evidence::EXPECTED_FINAL_HASH`/
// `EXPECTED_SEQUENCE_DIGEST`), the same two. They AGREE, so this is not
// #517 -- the constants above were simply stale (they still carried main's
// pre-merge values).
const NATIVE_FINAL_HASH = "0291700ae05c7a77";
const NATIVE_SEQUENCE_DIGEST = "a471ab55610efef3";

describe("the compiled wasm module against the AI-driven Lua reference", () => {
  it("replays the scenario it claims to, and plays it", () => {
    const evidence = runAiDrivenEvidence();
    expect(evidence.fixture_id).toBe("session_ai_driven/v1");
    expect(evidence.ticks).toBe(7200);
    expect(evidence.rows).toBe(7201);
    // A regression that reduced the bot to an idle player would still produce
    // stable digests, and would silently turn this back into the AFK scenario
    // it exists to replace.
    expect(evidence.score_home + evidence.score_away).toBeGreaterThan(0);
  });

  // ---------------------------------------------------------------------
  // KNOWN DIVERGENCE -- wasm vs native/Lua. Issue #405. First observed
  // 2026-08-07; re-measured 2026-08-10 under #450.
  //
  // The wasm build takes a different path from tick 96 of this scenario. The
  // SOURCE is identical and native Rust agrees with Lua bit for bit
  // (`crates/gc-sim/tests/session_ai_driven_differential.rs`), so this is the
  // same source compiled for a different target -- stable across repeated
  // runs, so not nondeterminism.
  //
  // 2026-08-07, before #450:
  //
  //   Lua fixture   final 5254dcc8efde305b   sequence 5278f4a48da4800a
  //   native Rust   final 5254dcc8efde305b   sequence 5278f4a48da4800a
  //   wasm          final 5254dcc8efde305b   sequence 17998dc0e72d8510
  //
  // The final states happened to AGREE then, which is what made the sequence
  // failure so easy to miss. #450 changed the simulation, and the coincidence
  // expired. 2026-08-10, on this tree:
  //
  //   Lua fixture   final 628d7fc71238dec6   sequence 29bbbc0f32b78dfa
  //   native Rust   final 628d7fc71238dec6   sequence 29bbbc0f32b78dfa
  //   wasm          final 20ff634062e96578   sequence d7b87ee152cf2ce8
  //
  // THE DEFECT DID NOT MOVE, AND #450 DID NOT CAUSE IT. Bisecting prefix
  // digests (`runAiDrivenEvidenceTo`) puts the first divergent tick at 96, the
  // same tick as before, with the same self-correcting shape:
  //
  //   to 95    wasm 0ce5b1fafb0f40e5   native 0ce5b1fafb0f40e5   agree
  //   to 96    wasm 95a53203fce97fec   native eb150bcf724c0406   DIVERGE
  //   to 97    wasm 531f22a8c957a5a4   native 531f22a8c957a5a4   reconverged
  //   to 100   wasm b8630c1136c7b0cf   native b8630c1136c7b0cf   agree
  //
  // #450's first behavioural change is at tick 3099, 3,003 ticks later. What
  // changed at full time is only that the tick-96 perturbation no longer
  // happens to reconverge before the whistle. Both builds still finish 0-1.
  //
  // This is exactly the shape `tools/lua_reference/README.md` warns about:
  // "a divergence which self-corrects a tick later is still a desync". For an
  // offline match it costs a few frames of different-looking play. For an
  // online one it is a desync, because two peers on different builds would
  // not agree -- which is the entire premise the netcode rests on.
  //
  // Both are pinned as `it.fails` rather than deleted or loosened: each flips
  // green the moment #405 is fixed, and each fails loudly if someone "fixes"
  // it by changing the expected digests instead. The OMP-1 idle campaign never
  // caught this because an idle match does not reach the code paths involved --
  // and OMP-1 still passes in wasm, on the refreshed #450 contract, which is
  // why the divergence is specific to this scenario rather than general.
  // ---------------------------------------------------------------------
  // ---------------------------------------------------------------------
  // #520 + #488 UPDATE, 2026-08-12. READ THIS BEFORE CONCLUDING #405 IS FIXED.
  //
  // These two were `it.fails` against the Lua digests. Two things changed at
  // once, and conflating them would be the expensive mistake:
  //
  //   1. #520 retired the Lua behavioral vector, so "the state Lua ends in"
  //      is no longer a claim this repository makes. The constants are now
  //      the native Rust build's, re-recorded in the same commit.
  //   2. MEASURED ON THIS BUILD, wasm and native agree exactly --
  //      final 64b8ad7d35ab1c39, sequence 6c6d9581eac53f8c, both targets. So
  //      these are plain `it` now, because they pass. Re-measured after
  //      merging #501, which moves this scenario: both targets moved together
  //      and still agree, which is the property these two lines gate.
  //
  // THAT IS NOT EVIDENCE THAT #405 IS FIXED, and nobody should close it on
  // the strength of these two lines going green. #405's divergence at tick 96
  // of this scenario was never a constant defect: it is two libms
  // disagreeing occasionally (#517), so WHICH transcendental calls happen,
  // with which arguments, moves with the trajectory -- and #488 changed the
  // trajectory of every body on the pitch. The same measurement during this
  // PR's development caught the divergence at OMP-1 boundary 12, then 7006,
  // then not at all, across three drafts of one module. It is latent, not
  // gone.
  //
  // What these two now gate is still worth having, and is stronger than an
  // `it.fails`: any future divergence between the two targets on this
  // scenario turns them red instead of quietly satisfying an expected-fail.
  // ---------------------------------------------------------------------
  // ---------------------------------------------------------------------
  // #531 PHASE 2 RE-MEASUREMENT, 2026-08-14. Re-recorded because phase 2
  // moved the gameplay AI's pass/throw decisions onto the same `MatchInput`
  // charge-and-release seam a human uses (#531), which shifts every RNG draw
  // downstream of the first AI pass and therefore this scenario's whole
  // trajectory from that tick on -- see `gc-sim/tests/ai_driven_evidence.rs`.
  // Expected this run to plausibly re-expose #405's tick-96 divergence, since
  // #517 says WHICH transcendental calls happen moves with the trajectory.
  // MEASURED ON THIS BUILD: wasm and native still agree exactly -- final
  // d146d1cc4f359ca7, sequence abe72518de86a606, both targets, both native
  // `cargo test -p gc-sim --test ai_driven_evidence` and a direct
  // `runAiDrivenEvidence()` call against the freshly built wasm module. So
  // these stay plain `it`, not reverted to `it.fails`; #405 remains open and
  // latent, not fixed, for the same reason the note above gives.
  // ---------------------------------------------------------------------
  // ---------------------------------------------------------------------
  // #517 MECHANICAL SITE CONVERSION RE-MEASUREMENT, 2026-08-14. Re-recorded
  // because this PR converts the nine MECHANICALLY-REPLACEABLE sites (the
  // dribble-touch and AI-outfield-error `cos`/`sin` in `match.rs`, the
  // aerial-contact `cos`/`sin` in `aerial.rs`, the aim-noise `cos`/`sin` in
  // `bot.rs`, and the support-triangle/combat-arc constants) to
  // `gc_core::deterministic_math::cos_sin` or a precomputed value, which is
  // exactly the class of change #405's note above warns moves this
  // scenario's whole trajectory. MEASURED ON THIS BUILD: wasm and native
  // still agree exactly -- final eca1c4cbe8cbaf40, sequence
  // 74bcbe5d31dd15c0, both targets, both native `cargo test -p gc-sim --test
  // ai_driven_evidence` and this spec against the freshly built wasm module.
  // These stay plain `it`. This is also the direct, best-available evidence
  // that converting these nine sites did not merely move where #517's own
  // corpus differential (`scripts/check_wasm_native_corpus.mjs`) looks: this
  // scenario is the one #405 originally caught wasm/native disagreeing on,
  // and it still agrees after the conversion.
  // ---------------------------------------------------------------------
  // ---------------------------------------------------------------------
  // #489 COMMITTED-ACTIONS RE-MEASUREMENT, 2026-08-15. Re-recorded because
  // #489 adds `MatchPlayer::action` (match_snapshot::VERSION 12 -> 13, see
  // `gc_sim::action_slot`) and replaces the standing-poke tackle's
  // instant-resolve `attempt_steals` branch with a real charge/execute/
  // recover state machine (`r#match::advance_tackle_actions`) -- exactly
  // the class of change #405's note above warns reshapes this scenario's
  // whole trajectory from the first tackle attempt on. See
  // `gc-sim/tests/ai_driven_evidence.rs` for the native-side re-record.
  // ---------------------------------------------------------------------
  it("ends the match in exactly the state the native build ends it in", () => {
    expect(runAiDrivenEvidence().final_hash).toBe(NATIVE_FINAL_HASH);
  });

  it("matches the native build tick for tick, not merely at the final whistle", () => {
    expect(runAiDrivenEvidence().sequence_digest).toBe(NATIVE_SEQUENCE_DIGEST);
  });
});
