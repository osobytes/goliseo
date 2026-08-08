// Does the COMPILED WASM MODULE reproduce the Lua reference match?
//
// `determinism.spec.ts` beside this asks the same question for the OMP-1
// campaign -- an IDLE match, where the local player never presses anything.
// This asks it for a match in which every player, including the one on the
// human-input branch, is AI-driven: shooting, charging a shot and releasing
// it, passing, lofting, dashing, dodging, jockeying, sprinting, and the
// input-frame quantisation that is lossless until someone actually steers.
//
// The two digests below are NOT copied from a Rust run. `crates/gc-sim/tests/
// ai_driven_evidence.rs` derives them from the captured output of real Lua
// (`session_ai_driven_lua_reference.txt`) and asserts that the native Rust
// replay reproduces them; this file asserts the same of the wasm build. So a
// green result here means the code a browser executes reproduces LUA, which is
// the only claim worth making.

import { describe, expect, it } from "vitest";
import { loadSimHost } from "./index.ts";

const runAiDrivenEvidence = () => loadSimHost().runAiDrivenEvidence();

/** Derived from the Lua capture -- see `crates/gc-sim/tests/ai_driven_evidence.rs`. */
const LUA_FINAL_HASH = "5254dcc8efde305b";
const LUA_SEQUENCE_DIGEST = "5278f4a48da4800a";

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

  it("ends the match in exactly the state Lua ends it in", () => {
    // This one PASSES. The final state agrees to the bit -- which is what
    // makes the failure below so easy to miss, and why a sequence digest
    // exists at all.
    expect(runAiDrivenEvidence().final_hash).toBe(LUA_FINAL_HASH);
  });

  // ---------------------------------------------------------------------
  // KNOWN DIVERGENCE -- wasm vs native, first observed 2026-08-07.
  //
  // The wasm build's tick SEQUENCE does not match Lua's, while its final
  // state does. Measured, and reproducible:
  //
  //   Lua fixture   final 5254dcc8efde305b   sequence 5278f4a48da4800a
  //   native Rust   final 5254dcc8efde305b   sequence 5278f4a48da4800a
  //   wasm          final 5254dcc8efde305b   sequence 17998dc0e72d8510
  //
  // The wasm result is stable across repeated runs, so this is not
  // nondeterminism -- it is the same source compiled for a different target
  // taking a different path and reconverging. Bisecting prefix digests
  // (`runAiDrivenEvidenceTo`) puts the FIRST divergent tick at 96: ticks 1-95
  // agree exactly, tick 96 does not.
  //
  // This is exactly the shape `v2/tools/lua_reference/README.md` warns about:
  // "a divergence which self-corrects a tick later is still a desync". For an
  // offline match it costs a few frames of different-looking play. For an
  // online one it is a desync, because two peers on different builds would
  // not agree -- which is the entire premise the netcode rests on.
  //
  // Pinned as `it.fails` rather than deleted or loosened: this flips green the
  // moment the divergence is fixed, and fails loudly if someone "fixes" it by
  // changing the expected digests instead. The OMP-1 idle campaign never
  // caught this because an idle match does not reach the code paths involved.
  // ---------------------------------------------------------------------
  it.fails("matches Lua tick for tick, not merely at the final whistle", () => {
    expect(runAiDrivenEvidence().sequence_digest).toBe(LUA_SEQUENCE_DIGEST);
  });
});
