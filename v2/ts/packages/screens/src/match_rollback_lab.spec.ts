// Ported from spec/screens/match_rollback_lab_spec.lua.
//
// Re-checked, not just trusted: a wasm bridge for `gc-sim` now exists
// (`@gc/wasm`, `MatchScreen`'s real `SimHostPort` game loop in match.ts),
// so "no wasm bridge this milestone" alone no longer explains this file's
// skip. What still blocks every case here, layered:
//
//   1. `MatchScreen` (match.ts) has no `rollback_lab` construction option
//      at all -- no `_rollback_lab`/`_rollback_debug`/`_rollback_outputs`/
//      `_rollback_corrections`/`_render_smoothing` fields, no `local_slot`/
//      `network_profile`/`profile_name` options. `sim.rollback_playable_lab`
//      IS ported to Rust (`crates/gc-sim/src/rollback_playable_lab.rs`), but
//      nothing threads a laboratory session through `SimHostPort` into this
//      screen -- see match.ts's own header ("the rollback laboratory" is
//      named as explicitly out of scope this milestone).
//   2. `SimHostPort`'s fixed contract (`planTicks`/`step`/
//      `cancelPlannedTicks`/`frame`/`roster`/`tick`/`dispose`) has no
//      rollback/correction surface to expose one through even if `Match`
//      grew the option -- it was designed for the single-peer offline game
//      loop, not a resimulating client.
//   3. The closest existing wasm rollback surface, `@gc/wasm`'s
//      `MatchDriverBridge` (`gc_netcode::match_driver` bound over
//      `gc_sim::rollback_events`), is mid-flux: it currently skips
//      correction/rollback batches rather than guess at `apply`'s interval
//      semantics (another agent's concurrent work this milestone). Even a
//      from-scratch lab construction on top of it could not produce a real
//      correction batch to assert on yet.
//   4. Presentation dependencies: `correction_smoothing.ts`/`view_state.ts`/
//      `bloom.ts` (`@gc/render`) and `replay.ts` (`@gc/render`) all exist in
//      TypeScript now, but `@gc/render` is still not a declared dependency
//      of `@gc/screens` (the same gap `match_screen.spec.ts`'s goal-replay
//      skip names). `tuning_panel.ts` (`@gc/ui`) is reachable -- `@gc/ui`
//      IS declared -- but is unused without the rest.
//   5. Tier 3 additionally needs `game.screen_stack`'s TS analog and
//      `real_match.ts`'s `RealMatch`, both of which exist (`@gc/app`'s
//      `screen_stack.ts`, `@gc/screens`' `real_match.ts`) but only compose
//      with a rollback-lab-capable `MatchScreen`, which per (1) does not
//      exist yet.
//
// `match.ts` ports only the rollback-consumption seam
// (`consumeRollbackEventDiff`/`consumeConfirmedStep`/
// `consumeConfirmedLifecycle`), which `combat_feedback_rollback.spec.ts`
// exercises for real (and `rollback_validation.spec.ts`'s
// "keeps confirmed lifecycle presentation idempotent at the screen
// boundary" case exercises again, against the real `Audio`); the
// laboratory construction, the fixed-clock handoff, and the render-
// smoothing/replay-gait behaviour this file covers remain out of scope
// this milestone.

import { describe, it } from "vitest";

describe.skip("match screen rollback laboratory (tier 2) [MatchScreen has no rollback_lab construction option; SimHostPort has no rollback/correction surface; see this file's header]", () => {
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

describe.skip("playable rollback ScreenStack flow (tier 3) [needs a rollback-lab-capable MatchScreen, which does not exist yet -- see this file's header, point 5]", () => {
  it.skip("converges under the checked-in playable profile with pinned seeds", () => {});
  it.skip("reconciles a rollback goal through confirmed replay and result completion", () => {});
});
