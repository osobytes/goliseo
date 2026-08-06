// Ported from spec/screens/match_rollback_lab_spec.lua.
//
// Unblocker: every case drives the development rollback laboratory
// (`sim.rollback_playable_lab`) end to end -- real `sim.match` physics,
// real multi-tick network simulation, a stubbed `love.keyboard`, and (for
// the tier-3 cases) a real `game.screen_stack`. All of it is Rust-owned
// (`crates/gc-sim`; v2/README.md §2) with no wasm bridge this milestone, or
// depends on packages (`@gc/render`, `@gc/app`) this one does not declare
// as dependencies. `match.ts` ports only the rollback-consumption seam
// (`consume_rollback_event_diff`/`consume_confirmed_step`/
// `consume_confirmed_lifecycle`), which `combat_feedback_rollback.spec.ts`
// exercises for real; the laboratory construction, the fixed-clock
// handoff, and the render-smoothing/replay-gait behaviour this file
// covers are all out of scope this milestone -- see match.ts's header.

import { describe, it } from "vitest";

describe.skip("match screen rollback laboratory (tier 2) [needs wasm-compiled gc-sim + gc-netcode's rollback laboratory]", () => {
  it.skip("constructs the combat companion for an explicit rollback playtest", () => {});
  it.skip("requires rollback snapshot combat presence to match the explicit opt-in", () => {});
  it.skip("is an explicit development-only slot-mode option", () => {});
  it.skip("retains a zero-tick edge and consumes it exactly once", () => {});
  it.skip("captures a complete equipment tap before the next render update", () => {});
  it.skip("uses one fixed clock and aggregates multi-tick edges, holds, and corrections", () => {});
  it.skip("updates live player view state from the displayed rollback client", () => {});
  it.skip("clears rollback handoff batches before paused and terminal early returns", () => {});
  it.skip("preserves fixed-clock overload dropping and contiguous transport ticks", () => {});
  it.skip("live R replaces all rollback and presentation-owned state", () => {});
  it.skip("clears smoothing at kickoff, full time, and stack teardown", () => {});
  it.skip("keeps actual goal replay gait coherent and clears smoothing on both exits", () => {});
  it.skip("draws only from the cached debug model without mutating either match", () => {});
});

describe.skip("playable rollback ScreenStack flow (tier 3) [needs wasm-compiled gc-sim + gc-netcode + @gc/app's screen stack]", () => {
  it.skip("converges under the checked-in playable profile with pinned seeds", () => {});
  it.skip("reconciles a rollback goal through confirmed replay and result completion", () => {});
});
