// Ported from spec/screens/match_rollback_lab_spec.lua's "playable rollback
// ScreenStack flow (tier 3)" describe block -- left `describe.skip`/
// `it.skip` in `@gc/screens` (packages/screens/src/match_rollback_lab.spec.ts)
// because both cases need `ScreenStack` (`@gc/app`'s `screen_stack.ts`)
// driving a real `MatchScreen`/`RealMatch` pair, and `@gc/screens` cannot
// depend on `@gc/app` -- the dependency runs the other way (v2/README.md
// §2/§9). `@gc/app` is the correct side of that edge; this file is where
// the cases now live.
//
// # Moving them here does not unblock them
//
// `ScreenStack` was necessary but was never sufficient. Both cases drive
// `Match.new({ rollback_lab = ... })` with a REAL `profile_name = "playable"`
// against the REAL `sim.rollback_playable_lab` (`crates/gc-sim/src/
// rollback_playable_lab.rs`) -- a deterministic, network-simulated, bot-
// driven local rollback harness that runs many ticks and reports
// convergence (`_rollback_debug.status` reaching `"converged"`, matched
// snapshot hashes, `rollback_count`/`resimulated_ticks` > 0).
//
// `@gc/screens`'s `match.ts` does not, and structurally cannot, compute any
// of that itself: `MatchScreen`'s rollback surface is `RollbackHostPort`, a
// package-local interface this package's own tier-2 cases (same source
// file) drive with a hand-written `FakeRollbackHost` specifically BECAUSE
// no `@gc/wasm` binding for `sim.rollback_playable_lab` exists yet (see
// that file's header, "STATUS" section). `_rollback_debug.status`/
// `.convergence`/`.rollback_count` etc. are a pure passthrough of whatever
// the host reports (`match.ts`'s `RollbackLabHostDebug` doc) -- `MatchScreen`
// never computes "converged" itself. `FakeRollbackHost` (as it stands in
// that package, and as it would have to be copied or extended here) never
// produces a `"converged"`/`"settling"` status at all; it only ever reports
// `"active"` or `"unconfirmed_window_exceeded"`. Making it report
// `"converged"` for this case would mean inventing convergence detection
// locally and asserting against that invention -- exactly the
// "asserting against your own fake" anti-pattern this port must not commit
// (the same principle `online_match_flow.spec.ts`, this package, names for
// its own still-skipped cases).
//
// The second case ("reconciles a rollback goal...") additionally needs the
// BASE (non-rollback) goal-replay feature -- `@gc/render`'s real `replay.ts`
// consuming raw `MatchState`, not `SimHostPort`'s presentation-derived
// `RenderFrame` -- the same gap `match_screen.spec.ts`'s own "goal replay"
// skip and this file's tier-2 skips both name.
//
// So both cases stay `it.skip`, moved to the package that could host them,
// with the CURRENT and complete blocker recorded: a missing
// `sim.rollback_playable_lab` wasm bridge (building one needs a new Rust/
// wasm export, outside every file this batch owns -- no Rust crate may be
// touched), plus, for the second case, the base goal-replay gap. Building
// that bridge is what would actually unblock these, not any TypeScript-side
// glue in either package.

import { describe, it } from "vitest";

describe.skip(
  "playable rollback ScreenStack flow (tier 3) [needs a sim.rollback_playable_lab wasm bridge -- @gc/screens' MatchScreen only ever reports what its injected RollbackHostPort tells it, and no host anywhere in this codebase can report a real convergence; see this file's header]",
  () => {
    it.skip("converges under the checked-in playable profile with pinned seeds", () => {});
    it.skip("reconciles a rollback goal through confirmed replay and result completion", () => {});
  },
);
