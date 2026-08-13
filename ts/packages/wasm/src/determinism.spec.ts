// THE ACCEPTANCE TEST for this wave: `gc_sim::determinism_evidence::verify`
// run from inside the compiled wasm module, under node (vitest's default
// environment — see `ts/vitest.config.ts`), must reproduce exactly the
// digests JavaScriptCore, V8, SpiderMonkey and node already agreed on for
// the frozen 7,201-tick OMP-1 fixture
// (`crates/gc-data/src/omp1_determinism.rs`'s pinned
// `expected_final_hash`/`expected_sequence_digest`, and
// `crates/gc-sim/tests/determinism_evidence.rs`'s native-build assertion of
// the same two values). If these drift, that is a real finding about the
// wasm build, not a flaky test — do not "fix" it by changing the expected
// values here.
//
// Requires `pnpm --filter @gc/wasm build` to have run first so
// `dist/pkg/gc_wasm.cjs` exists.

import { describe, expect, it } from "vitest";

import { loadSimHost } from "./index.ts";

const EXPECTED_FINAL_HASH = "bfbb106aea5480f8";
const EXPECTED_SEQUENCE_DIGEST = "0bfd0ed355f87322";
// The behaviors the recording must still exercise. `scripts/check.sh` pins
// the same string independently as EXPECTED_COVERAGE.
const EXPECTED_COVERAGE = "tackle,aerial,keeper,full_time";

describe("determinism evidence, run inside the compiled wasm module", () => {
  // Why the explicit 30_000 timeout on the `it` below: 7,201 ticks (twice —
  // verify() runs an independent fresh comparison replay too) inside wasm is
  // measured at ~6s on this machine, well above vitest's 5s default. Wasm has
  // no JS-crossing per tick here (the whole campaign runs inside one
  // wasm-bindgen call), so this is real compute time, not marshalling
  // overhead.
  //
  // The note lives here rather than beside the argument it explains because
  // prettier mangles a comment in that position — it rotates the lines into
  // the closing brace and is not even idempotent about it.
  it("reproduces the frozen OMP-1 fixture's pinned digests exactly", () => {
    const host = loadSimHost();
    const result = host.runDeterminismEvidence();

    expect(result.final_hash).toBe(EXPECTED_FINAL_HASH);
    expect(result.sequence_digest).toBe(EXPECTED_SEQUENCE_DIGEST);

    // Same fixture facts the native `cargo test` asserts
    // (`crates/gc-sim/tests/determinism_evidence.rs`), pinned here too so a
    // fixture that stopped being this fixture — not just the two headline
    // digests — is caught.
    expect(result.ticks).toBe(7201);
    expect(result.boundaries).toBe(7202);

    // Replaces the score/outcome assertions that stood here until issue #505
    // split the campaign's claims by what they prove. "A tackle, a catch, a
    // header and a full time occurred" is what makes this recording evidence
    // of anything, and it survives a deliberate gameplay rework. "The score
    // was 1-0" does not, and gating on it foreclosed every queued one.
    expect(result.coverage).toBe(EXPECTED_COVERAGE);

    // Reported, never asserted (#505). Logged so a drifted scoreline is still
    // visible to whoever reads the run — vitest prints console output for a
    // passing test, which is the point: a demoted assertion that prints
    // nothing is a deleted assertion. `scripts/check.sh`'s determinism gate
    // prints the same field independently, from the same compiled module but
    // outside vitest (AGENTS.md §9: never trust one signal).
    console.log(
      `OMP-1 reported, not gated: score=${result.score_home}-${result.score_away} ` +
        `outcome=${result.outcome} drift=${result.behavioral_drift}`,
    );
    // The one thing asserted about the report: that there IS one. An empty
    // string means the reporting path broke; "none" means it ran and agreed.
    expect(result.behavioral_drift.length).toBeGreaterThan(0);
  }, 30_000);
});
