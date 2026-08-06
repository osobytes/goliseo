// Ported from spec/game/online_match_presentation_spec.lua.
//
// Every assertion in the Lua original is about a claim only a *live*
// rollback session can make: peers converging on one snapshot hash through
// a real correction, a real combat encounter being revoked and replaced by
// a resimulation, a real driver reaching `completed` at full time. All of
// that runs through `game.online.match_driver` and `sim.rollback_events`,
// both Rust-owned (`crates/gc-sim` / `crates/gc-netcode`; v2/README.md
// §2.1), driven by `spec/fixtures/online_match_session.lua` and
// `spec/support/online_combat_phases.lua`, neither of which has a TS
// equivalent either.
//
// Faking any of that in TypeScript would mean re-implementing rollback
// scheduling and combat resimulation here purely to make a test fixture
// realistic -- exactly the thing v2/README.md §2.1 names
// `match_presentation.lua` as "the one to watch" over. So every case below
// is ported as `it.skip`, matching the precedent in
// `packages/ui/src/tuning_panel.spec.ts`'s `describe.skip("tuning presets
// data", ...)`.
//
// # Status as of the `gc-wasm` `MatchDriverBridge` landing (commit 87d53b3)
//
// A wasm bridge over `game.online.match_driver` now exists
// (`crates/gc-wasm/src/match_driver_bridge.rs`, `@gc/wasm`'s
// `MatchDriverBridge`), but it does not close this file's gap, for three
// independent reasons -- any one of them alone would still block every
// case below:
//
//   1. `@gc/online`'s `package.json` does not declare `@gc/wasm` as a
//      dependency, so `import("@gc/wasm")` does not resolve here under
//      pnpm's strict workspace linking (confirmed directly: a probe import
//      in this package throws "Cannot find package '@gc/wasm'" even though
//      `packages/wasm/dist/pkg/gc_wasm.cjs` is built and present on disk).
//      This port does not own package manifests and was told not to edit
//      them; see this port's final report for the request to whoever does.
//   2. Even wired up, `MatchDriverBridge` exposes only the opaque,
//      aggregate `advance()` step plus read-only diagnostics/JSON dumps.
//      It has no wasm-bindgen surface matching the granular primitives
//      `RollbackEventsPort`/`MatchDriverPort` need to drive `consume()`
//      honestly: no `create(initialSnapshot, maxUnconfirmedTicks)`, no
//      `apply(from, through, steps)`, no `confirm(tick)`, no
//      `snapshot(boundaryTick)`. `match_driver_bridge.rs` does all of that
//      *inside* `advance()`, on the Rust side, and never hands the pieces
//      out -- by its own module doc's design (`MatchSnapshot` is
//      deliberately never serialized to JS).
//   3. Even a hypothetical raw `apply` would not save the 11 of these 13
//      cases whose claim is specifically about a *correction*: all seven
//      "keeps feedback honest through a correction during <phase>" cases,
//      "replaces the speculative tail on a correction...", "never
//      publishes a combat cue a correction took away", "agrees between
//      peers on every confirmed boundary" (bursty delivery, period 4), and
//      "publishes the lifecycle exactly once through full time" (bursty,
//      period 6) all require a real rollback/correction batch.
//      `match_driver_bridge.rs`'s own doc states plainly that its
//      rollback-event feed "simply does not attempt" that case: a
//      correction is reported as `rollback_events_fed: false` and skipped,
//      "and once skipped the feed cannot resynchronize", rather than
//      guess at `rollback_events::apply`'s replaced-interval contract. Only
//      the first two cases below ("publishes each confirmed event exactly
//      once under clean delivery", "tracks the driver's own confirmation
//      ceiling") use clean, non-bursty delivery and would not hit this
//      specific limit -- but both are still blocked by gaps 1 and 2 above.
//
// Re-port once (a) `@gc/wasm` is a declared dependency of `@gc/online` and
// (b) a `RollbackEventsPort`/`MatchDriverPort` backed by real, granular
// wasm-bindgen primitives exists -- and, for the 11 correction-dependent
// cases, once the bridge's rollback-event feed handles the correction case
// rather than skipping it.
//
// What *is* ported below, in the second describe block, is coverage of
// `match_presentation.ts`'s own control flow -- the append/correction
// split, the unconfirmed-window propagation, the confirmation-ceiling
// clamp -- against small hand-written fake ports. A fake real enough to
// reach "a correction happened during a ball spill" would be a rollback
// implementation; a fake that returns scripted, literal `RollbackEventDiff`
// values in response to `apply`/`confirm` calls is not, and it is enough to
// prove `consume`'s own branching is correct.

import { describe, expect, it } from "vitest";
import {
  consume,
  diagnostics,
  newOnlineMatchPresentation,
  status,
  type MatchDriverBatch,
  type MatchDriverPort,
  type MatchPresentationPorts,
  type RollbackApplyResult,
  type RollbackEventDiff,
  type RollbackEventStep,
  type RollbackEventsDiagnostics,
  type RollbackEventsPort,
  type RollbackEventsStatus,
  type RollbackTickOutput,
  type SnapshotLookup,
} from "./match_presentation.ts";

describe.skip("online match presentation (blocked: @gc/wasm not a declared dependency of @gc/online, and MatchDriverBridge exposes no raw snapshot/apply/confirm primitives or a correction-batch feed -- see the file header comment)", () => {
  it.skip("publishes each confirmed event exactly once under clean delivery", () => {});
  it.skip("tracks the driver's own confirmation ceiling", () => {});
  it.skip("replaces the speculative tail on a correction and never re-publishes it", () => {});
  it.skip("agrees between peers on every confirmed boundary it presented", () => {});
  it.skip("keeps feedback honest through a correction during windup", () => {});
  it.skip("keeps feedback honest through a correction during guard", () => {});
  it.skip("keeps feedback honest through a correction during contact", () => {});
  it.skip("keeps feedback honest through a correction during projectile_flight", () => {});
  it.skip("keeps feedback honest through a correction during stagger", () => {});
  it.skip("keeps feedback honest through a correction during ball_spill", () => {});
  it.skip("keeps feedback honest through a correction during immunity_expiry", () => {});
  it.skip("never publishes a combat cue a correction took away", () => {});
  it.skip("publishes the lifecycle exactly once through full time", () => {});
});

// ---------------------------------------------------------------------------
// Fake ports: scripted, literal responses only -- no rollback logic.
// ---------------------------------------------------------------------------

interface FakeTimeline {
  readonly maxUnconfirmedTicks: number;
  applied: Array<{ readonly from: number; readonly through: number; readonly count: number }>;
  confirmedTick: number;
  status: RollbackEventsStatus;
  /** When set, the next `apply` call fails with this status instead of succeeding. */
  failNextApply: boolean;
}

function fakeRollbackEvents(): RollbackEventsPort<FakeTimeline, number> {
  return {
    create(_initialSnapshot, maxUnconfirmedTicks): FakeTimeline {
      return { maxUnconfirmedTicks, applied: [], confirmedTick: -1, status: "active", failNextApply: false };
    },
    apply(timeline, from, through, steps): RollbackApplyResult {
      if (timeline.failNextApply) {
        return { ok: false, error: { message: "fake window exceeded", code: "unconfirmed_window_exceeded" } };
      }
      timeline.applied.push({ from, through, count: steps.length });
      const diff: RollbackEventDiff = {
        added: steps.map((step, offset) => ({
          id: `evt_${from + offset}`,
          tick: step.output.tick,
          domain: "match/test",
          ordinal: from + offset,
          payload: null,
        })),
        revoked: [],
        replaced: [],
      };
      return { ok: true, value: diff };
    },
    confirm(timeline, confirmedOutputTick): readonly RollbackEventStep[] {
      const steps: RollbackEventStep[] = [];
      for (let tick = timeline.confirmedTick + 1; tick <= confirmedOutputTick; tick += 1) {
        steps.push({
          tick,
          start_boundary: tick,
          end_boundary: tick + 1,
          state: { score: { home: 0, away: 0 }, time_left: 0, finished: false },
          match_events: [],
          lifecycle_events: [],
        });
      }
      timeline.confirmedTick = Math.max(timeline.confirmedTick, confirmedOutputTick);
      return steps;
    },
    diagnostics(timeline): RollbackEventsDiagnostics {
      return {
        status: timeline.status,
        confirmed_tick: timeline.confirmedTick,
        confirmed_boundary: timeline.confirmedTick + 1,
        max_unconfirmed_ticks: timeline.maxUnconfirmedTicks,
        retained_step_count: timeline.applied.length,
        retained_event_count: timeline.applied.reduce((sum, entry) => sum + entry.count, 0),
      };
    },
  };
}

interface FakeDriver {
  confirmedOutputTick: number;
}

function fakeMatchDriver(): MatchDriverPort<FakeDriver, number> {
  return {
    snapshot(_driver, boundaryTick): SnapshotLookup<number> {
      return { status: "present", tick: boundaryTick, snapshot: boundaryTick };
    },
    diagnostics(driver): { readonly confirmed_output_tick: number } {
      return { confirmed_output_tick: driver.confirmedOutputTick };
    },
  };
}

function output(tick: number): RollbackTickOutput {
  return { tick, end_boundary: tick + 1 };
}

function batch(outputs: readonly RollbackTickOutput[]): MatchDriverBatch {
  return { outputs };
}

describe("match presentation (pure control flow, fake ports)", () => {
  it("starts inactive-applied and active", () => {
    const rollbackEvents = fakeRollbackEvents();
    const presentation = newOnlineMatchPresentation(rollbackEvents, 0, 100, 30);
    expect(presentation.first).toBe(100);
    expect(presentation.applied).toBe(-1);
    expect(status(presentation)).toBe("active");
    expect(diagnostics(presentation, rollbackEvents).status).toBe("active");
  });

  it("appends forward outputs in order without producing a correction", () => {
    const rollbackEvents = fakeRollbackEvents();
    const matchDriver = fakeMatchDriver();
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = { rollbackEvents, matchDriver };
    const presentation = newOnlineMatchPresentation(rollbackEvents, 0, 0, 30);
    const driver: FakeDriver = { confirmedOutputTick: -1 };

    const result = consume(presentation, ports, driver, batch([output(0), output(1)]));

    expect(result.status).toBe("active");
    expect(result.corrections.length).toBe(0);
    expect(result.outputs.map((entry) => entry.tick)).toEqual([0, 1]);
    expect(result.event_diffs.length).toBe(2);
    expect(presentation.applied).toBe(1);
  });

  it("treats a tick at or below the applied ceiling as a correction and replaces the tail", () => {
    const rollbackEvents = fakeRollbackEvents();
    const matchDriver = fakeMatchDriver();
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = { rollbackEvents, matchDriver };
    const presentation = newOnlineMatchPresentation(rollbackEvents, 0, 0, 30);
    const driver: FakeDriver = { confirmedOutputTick: -1 };

    consume(presentation, ports, driver, batch([output(0), output(1)]));
    expect(presentation.applied).toBe(1);

    // A corrected replay of tick 1, followed by a fresh tick 2 in the same
    // batch -- the correction must not swallow the fresh append past what
    // it is replacing.
    const result = consume(presentation, ports, driver, batch([output(1), output(2)]));

    expect(result.corrections.length).toBe(1);
    expect(result.corrections[0]).toMatchObject({
      causal_tick: 1,
      replaced_from_tick: 1,
      replaced_through_tick: 1,
      corrected_from_tick: 1,
      corrected_through_tick: 1,
    });
    // The correction's own tick (1) is not re-appended to `outputs` (it was
    // already pushed once, ahead of the branch split); the fresh tick 2 is
    // appended as an ordinary forward output afterwards.
    expect(result.outputs.map((entry) => entry.tick)).toEqual([1, 2]);
    expect(presentation.applied).toBe(2);
  });

  it("stops at unconfirmed_window_exceeded and never resumes", () => {
    const rollbackEvents = fakeRollbackEvents();
    const matchDriver = fakeMatchDriver();
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = { rollbackEvents, matchDriver };
    const presentation = newOnlineMatchPresentation(rollbackEvents, 0, 0, 30);
    const driver: FakeDriver = { confirmedOutputTick: -1 };
    presentation.events.failNextApply = true;

    const first = consume(presentation, ports, driver, batch([output(0)]));
    expect(first.status).toBe("unconfirmed_window_exceeded");
    expect(status(presentation)).toBe("unconfirmed_window_exceeded");
    expect(presentation.applied).toBe(-1);

    // A later call does no further work: the timeline already gave up.
    const second = consume(presentation, ports, driver, batch([output(0), output(1)]));
    expect(second.status).toBe("unconfirmed_window_exceeded");
    expect(second.outputs.length).toBe(0);
  });

  it("clamps confirmation to what has actually been applied this batch", () => {
    const rollbackEvents = fakeRollbackEvents();
    const matchDriver = fakeMatchDriver();
    const ports: MatchPresentationPorts<FakeTimeline, number, FakeDriver> = { rollbackEvents, matchDriver };
    const presentation = newOnlineMatchPresentation(rollbackEvents, 0, 0, 30);
    // The driver's confirmation ceiling is far ahead of anything presented
    // yet -- confirmation must not claim ticks the timeline never applied.
    const driver: FakeDriver = { confirmedOutputTick: 50 };

    const result = consume(presentation, ports, driver, batch([output(0), output(1)]));

    expect(result.confirmed_steps.map((step) => step.tick)).toEqual([0, 1]);
    expect(presentation.applied).toBe(1);
  });
});
