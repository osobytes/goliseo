// #517's seeded native-vs-wasm differential corpus, wasm-side self-consistency.
//
// THE ACTUAL native-vs-wasm comparison -- the point of #517 -- lives in
// `scripts/check_wasm_native_corpus.mjs`, run from `scripts/check.sh` (gate
// 9b). It is not duplicated here as a vitest spec with pinned digests the
// way `ai_driven.spec.ts`/`determinism.spec.ts` compare against a frozen
// reference: this corpus is compared LIVE, against a fresh native run, on
// every gate invocation, which needs a `cargo test` child process outside
// vitest's own hermetic model (see that script's header for why the
// comparison is live rather than pinned).
//
// What THIS file gates instead: that the compiled wasm module's own
// `corpusScenarios()`/`runCorpusScenario()` surface behaves the way the
// differential script assumes it does -- deterministic within this target,
// producing the right shapes, and reachable through vitest's own `toBe`
// assertions rather than only through the bash gate's raw `node -e` probe
// (the same "never trust one signal" reasoning `determinism.spec.ts` and
// `check_determinism_terminator` split between them). If wasm itself were
// nondeterministic, a wasm-vs-native comparison could not tell "the targets
// disagree" from "this call got unlucky" -- this is the wasm-side half of
// that precondition; `every_scenario_is_deterministic_across_two_independent_native_runs`
// in `crates/gc-sim/tests/wasm_native_corpus.rs` is the native-side half.

import { describe, expect, it } from "vitest";
import { loadSimHost } from "./index.ts";

describe("gc-wasm corpus surface (#517)", () => {
  it("corpusScenarios() returns the real corpus, not an empty or truncated one", () => {
    const scenarios = loadSimHost().corpusScenarios();
    expect(scenarios.length).toBeGreaterThanOrEqual(8);
    // Every scenario needs a unique, non-empty id -- runCorpusScenario's
    // caller (the differential script) keys its native comparison by this.
    const ids = new Set(scenarios.map((s) => s.id));
    expect(ids.size).toBe(scenarios.length);
    for (const scenario of scenarios) {
      expect(scenario.id.length).toBeGreaterThan(0);
      expect(scenario.ticks).toBeGreaterThan(0);
    }
    // At least one scenario must enable combat -- otherwise
    // `combat.rs`/`combat_feasibility.rs`'s arc-cosine sites (#517's site 3)
    // are structurally unreachable by this corpus, exactly the gap
    // `gc_sim::wasm_native_corpus::CORPUS`'s own doc explains.
    expect(scenarios.some((s) => s.combat_enabled)).toBe(true);
  });

  it("runCorpusScenario() is deterministic across two independent calls, inside this compiled module", () => {
    const host = loadSimHost();
    const scenarios = host.corpusScenarios();
    for (const scenario of scenarios) {
      // A short prefix of each scenario, not its full length: this proves
      // the property (wasm-side determinism), not a specific tick budget --
      // running every scenario at full length twice here would duplicate
      // the differential script's own cost for no extra evidence.
      const ticks = Math.min(scenario.ticks, 50);
      const first = host.runCorpusScenario(
        scenario.id,
        scenario.match_seed,
        scenario.bot_seed,
        ticks,
        scenario.combat_enabled,
      );
      const second = host.runCorpusScenario(
        scenario.id,
        scenario.match_seed,
        scenario.bot_seed,
        ticks,
        scenario.combat_enabled,
      );
      expect(
        second.tick_hashes,
        `${scenario.id} was not deterministic across two wasm calls`,
      ).toEqual(first.tick_hashes);
      expect(second.final_hash).toBe(first.final_hash);
      expect(second.sequence_digest).toBe(first.sequence_digest);
    }
  });

  it("runCorpusScenario() returns one tick_hash per tick, ticks 0 through `ticks` inclusive", () => {
    const host = loadSimHost();
    const scenario = host.corpusScenarios()[0];
    if (!scenario) {
      throw new Error("corpusScenarios() returned no scenarios");
    }
    const ticks = Math.min(scenario.ticks, 30);
    const result = host.runCorpusScenario(
      scenario.id,
      scenario.match_seed,
      scenario.bot_seed,
      ticks,
      scenario.combat_enabled,
    );
    expect(result.ticks).toBe(ticks);
    expect(result.tick_hashes.length).toBe(ticks + 1);
    expect(result.tick_hashes[result.tick_hashes.length - 1]).toBe(result.final_hash);
    expect(result.scenario_id).toBe(scenario.id);
  });
});
