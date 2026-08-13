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
const NATIVE_FINAL_HASH = "1fc71b997dab5d34";
const NATIVE_SEQUENCE_DIGEST = "5fe43d81cd0ce20a";

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
  //      final 1fc71b997dab5d34, sequence 5fe43d81cd0ce20a, both targets. So
  //      these are plain `it` now, because they pass.
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
  it("ends the match in exactly the state the native build ends it in", () => {
    expect(runAiDrivenEvidence().final_hash).toBe(NATIVE_FINAL_HASH);
  });

  it("matches the native build tick for tick, not merely at the final whistle", () => {
    expect(runAiDrivenEvidence().sequence_digest).toBe(NATIVE_SEQUENCE_DIGEST);
  });
});
