// Ported from spec/game/rollback_validation_spec.lua.
//
// The Lua original builds its fixtures from `sim.match`/`sim.combat`/
// `sim.match_snapshot`/`sim.rollback_events` (Rust-owned, `crates/gc-sim`;
// v2/README.md §2) and `game.screens.match`. That header note used to say
// none of that reaches TypeScript this milestone -- stale by the time this
// task started: `@gc/presentation`'s `combatFeedback` (real audio/vfx
// disposition mapping) and `@gc/screens`'s `match.ts` (the rollback-
// consumption seam, `consumeConfirmedLifecycle`) are BOTH declared
// dependencies of this package already (see package.json) and are used for
// real below, not faked. Six of the Lua original's seven cases turn out not
// to need `sim.match`/`sim.rollback_events` at all once read closely: they
// build their `RollbackEventStep`/`RollbackEventDiff` fixtures by hand
// (`match_event`/`lifecycle_event`/`step`/`diff` below, transcribed from the
// Lua helpers of the same name) and drive `rollback_validation.ts`'s own
// `run`/`newAudit`/`finish` against them -- exactly the "small hand-written
// fakes" technique `combat_feedback_rollback.spec.ts` already established
// for this kind of port, just applied to `EffectsPort`/`ReplayPort` too
// (`trackingEffectsPort`/`recordingReplayPort` below) so the cross-checks
// those two fixtures exist to prove (`consumer_speculative_residue`,
// `replay_matched`) are genuine rather than trivially zero.
//
// The one case that remains skipped, "derives reference identities from raw
// campaign step inputs", is the one case that genuinely calls
// `sim.rollback_events.new/apply/confirm` -- a real Rust timeline
// (create/apply/confirm), not just the wire shapes it produces.
//
// # Re-audited against the current `@gc/wasm`
//
// The claim this comment used to make -- "`@gc/wasm` does not expose that
// surface directly, only the bundled `MatchDriverBridge`" -- is **stale**:
// `crates/gc-wasm/src/rollback_events_bridge.rs`'s `RollbackEventsTimeline`
// is exactly a standalone, directly-callable `create`/`apply`/`confirm`
// port (`match_presentation.spec.ts` builds a real `RollbackEventsPort`
// adapter over it, driven by a real two-peer `MatchDriverBridge` harness,
// for four of *its* thirteen cases).
//
// What is real and narrower: this specific case needs a `WasmMatchSnapshot`
// for a *hand-specified* game state (`final.owner = 2`, a `"shot"` event at
// the ball's exact position, `time_left` nudged by one tick) -- the Lua
// original builds it with `match_snapshot.capture(final)` on a hand-mutated
// `MatchState` table. `@gc/wasm` has no snapshot-construction entry point at
// all: the only ways to obtain a `WasmMatchSnapshot` are `SimSession
// .snapshotHandle()` (whatever state a real, actually-stepped session
// reached) and `MatchDriverBridge.initialSnapshotHandle`/`.snapshotLookup`
// (ditto, for a real driver) -- confirmed by reading `session.rs` and
// `match_driver_bridge.rs` end to end, neither exposes anything resembling
// "build a snapshot from these fields." Reaching this exact scenario for
// real would mean scripting genuine gameplay input precisely enough to
// force a real shot at a real, known position and tick -- deep
// gameplay-mechanics engineering this port has no other reason to carry,
// and guessing at it risks a flaky, non-representative test even if it
// happens to pass once. Left `it.skip`, for this narrower and different
// reason than the one this comment used to give.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import { combatFeedback as realCombatFeedback, type CombatEvent } from "@gc/presentation";
import {
  consumeConfirmedLifecycle,
  newMatchRollbackConsumerState,
  type AudioPort as ScreenAudioPort,
  type EffectsPort as ScreenEffectsPort,
  type ReplayPort as ScreenReplayPort,
  type RollbackWrappedLifecycleEvent,
} from "@gc/screens";
import { Audio, type CombatFeedbackPort } from "./audio.ts";
import type { ObservedMatchState } from "./match_observer.ts";
import {
  applyImpairedDiff,
  newAudit,
  observeImpairedStep,
  observeReferenceStep,
  finish,
  run,
  expectedReplayBoundaries,
  type EffectsPort,
  type MatchTeam,
  type ReplayPort,
  type ReplaySampleState,
  type RollbackConfirmedStateView,
  type RollbackEventDiff,
  type RollbackEventReplacement,
  type RollbackEventStep,
  type RollbackEventsPort,
  type MatchSnapshotPort,
  type RollbackValidationPorts,
  type RollbackValidationReplaySample,
  type RollbackValidationTraceRow,
  type RollbackWrappedEvent,
} from "./rollback_validation.ts";

describe("rollback validation (cross-boundary integration)", () => {
  // `RollbackEventsTimeline` (create/apply/confirm) is real now -- the
  // remaining blocker is narrower: no way to build a `WasmMatchSnapshot` for
  // this case's hand-specified game state. See this file's header.
  it.skip("derives reference identities from raw campaign step inputs", () => {});
});

type FakeState = ObservedMatchState & ReplaySampleState;

function fakeState(): FakeState {
  return {
    players: [
      { id: "home_1", team: "home", is_keeper: true },
      { id: "home_2", team: "home", is_keeper: false },
    ],
    events: [],
    score: { home: 0, away: 0 },
    input_tick: 0,
    ball: new Vec2(0, 0),
  };
}

const NOOP_COMBAT_FEEDBACK: CombatFeedbackPort = {
  link: () => ({ disposition: {} }),
  disposition: () => ({}),
};

/** `@gc/presentation`'s real `combatFeedback`, wrapped to satisfy the injected, payload-agnostic `CombatFeedbackPort` -- see this file's header. */
const REAL_COMBAT_FEEDBACK: CombatFeedbackPort = {
  link: (payload, stableId) => realCombatFeedback.link(payload as CombatEvent, stableId),
  disposition: (event) => realCombatFeedback.disposition(event as CombatEvent),
};

function fakePorts(): RollbackValidationPorts<FakeState, undefined, FakeState, unknown[]> {
  const matchSnapshot: MatchSnapshotPort<FakeState, undefined, FakeState> = {
    capture: (state) => state,
    restore: (snapshot) => snapshot,
    numberBytes: (value) => value.toFixed(6),
  };
  const rollbackEvents: RollbackEventsPort<unknown[], FakeState> = {
    create: () => [],
    apply: () => ({ ok: true, value: { added: [], revoked: [], replaced: [] } }),
    confirm: () => [],
  };
  const effects: EffectsPort = {
    reset: () => {},
    applyEventDiff: () => {},
    confirmEvent: () => {},
    diagnostics: () => ({ speculative_ids: [] }),
  };
  const replay: ReplayPort<FakeState, undefined> = {
    reset: () => {},
    capacity: () => 8,
    truncateFrom: () => {},
    recordBoundary: () => {},
    diagnostics: () => ({ count: 0, boundaries: [] }),
    boundarySample: () => undefined,
  };
  return {
    matchSnapshot,
    rollbackEvents,
    effects,
    replay,
    audio: new Audio(NOOP_COMBAT_FEEDBACK),
    combatFeedback: NOOP_COMBAT_FEEDBACK,
  };
}

function confirmedState(overrides: Partial<RollbackConfirmedStateView> = {}): RollbackConfirmedStateView {
  return { score: { home: 0, away: 0 }, ...overrides };
}

function passEvent(id: string, tick: number): RollbackWrappedEvent {
  return { id, tick, domain: "match/pass", ordinal: 0, payload: { kind: "pass" } };
}

describe("rollback validation's own control flow", () => {
  it("initializes a fresh audit with zeroed counters and every scenario at zero", () => {
    const ports = fakePorts();
    const audit = newAudit(ports, fakeState(), undefined);
    expect(audit.events.reference_unique).toBe(0);
    expect(audit.events.speculative_added).toBe(0);
    expect(audit.referenceObserverSteps).toBe(0);
    expect(audit.replayRecordCount).toBe(0);
    for (const scenario of Object.values(audit.scenarioCounts)) {
      expect(scenario).toBe(0);
    }
  });

  it("tracks speculative add/revoke/replace bookkeeping from an event diff", () => {
    const ports = fakePorts();
    const audit = newAudit(ports, fakeState(), undefined);
    const added = passEvent("evt-1", 1);
    applyImpairedDiff(ports.effects, audit, { added: [added], revoked: [], replaced: [] }, ports.matchSnapshot.numberBytes);
    expect(audit.events.speculative_added).toBe(1);
    expect(audit.speculativeIds.get("evt-1")).toBeDefined();

    applyImpairedDiff(ports.effects, audit, { added: [], revoked: [added], replaced: [] }, ports.matchSnapshot.numberBytes);
    expect(audit.events.speculative_revoked).toBe(1);
    expect(audit.events.speculative_unknown_revoked).toBe(0);
    expect(audit.speculativeIds.has("evt-1")).toBe(false);

    // Revoking an id the ledger never saw is flagged, not silently accepted.
    applyImpairedDiff(ports.effects, audit, { added: [], revoked: [added], replaced: [] }, ports.matchSnapshot.numberBytes);
    expect(audit.events.speculative_unknown_revoked).toBe(1);
  });

  it("finishes clean when reference and impaired observe the same confirmed step", () => {
    const ports = fakePorts();
    const audit = newAudit(ports, fakeState(), undefined);
    const step: RollbackEventStep = {
      tick: 0,
      state: confirmedState({ owner_team: "home" }),
      match_events: [passEvent("evt-1", 0)],
      lifecycle_events: [],
    };
    observeReferenceStep(ports.matchSnapshot.numberBytes, audit, step);
    applyImpairedDiff(ports.effects, audit, { added: [passEvent("evt-1", 0)], revoked: [], replaced: [] }, ports.matchSnapshot.numberBytes);
    observeImpairedStep({ numberBytes: ports.matchSnapshot.numberBytes, effects: ports.effects, audio: ports.audio }, audit, step);

    const report = finish(ports, audit, { home_team_id: "nebula", away_team_id: "orion" });
    expect(report.passed).toBe(true);
    expect(report.errors).toEqual([]);
    expect(report.events.missing_confirmed).toBe(0);
    expect(report.events.unexpected_confirmed).toBe(0);
  });

  it("reports missing confirmation when the impaired run never saw a reference event", () => {
    const ports = fakePorts();
    const audit = newAudit(ports, fakeState(), undefined);
    const step: RollbackEventStep = {
      tick: 0,
      state: confirmedState(),
      match_events: [passEvent("evt-only-reference", 0)],
      lifecycle_events: [],
    };
    observeReferenceStep(ports.matchSnapshot.numberBytes, audit, step);
    // Impaired side confirms an empty step -- never observes evt-only-reference.
    observeImpairedStep(
      { numberBytes: ports.matchSnapshot.numberBytes, effects: ports.effects, audio: ports.audio },
      audit,
      { tick: 0, state: confirmedState(), match_events: [], lifecycle_events: [] },
    );

    const report = finish(ports, audit, { home_team_id: "nebula", away_team_id: "orion" });
    expect(report.passed).toBe(false);
    expect(report.events.missing_confirmed).toBe(1);
    expect(report.errors).toContain("confirmed events are missing");
  });
});

// =============================================================================
// THE SIX PORTABLE INTEGRATION CASES -- see this file's header for why these
// six do not actually need `sim.match`/`sim.rollback_events`, only real
// `combatFeedback` (audio disposition) and hand-fixtured trace rows.
// =============================================================================

/** A speculative-id ledger that actually tracks add/revoke/replace/confirm, the same bookkeeping `game/render/effects.lua`'s real ledger performs -- so `consumer_speculative_residue` cross-checks something real rather than being trivially zero (`fakePorts`' own `EffectsPort` above, by contrast, is deliberately inert -- fine for the control-flow tests, not for these). */
function trackingEffectsPort(): EffectsPort {
  const speculative = new Set<string>();
  return {
    reset: () => speculative.clear(),
    applyEventDiff: (diff: RollbackEventDiff) => {
      for (const event of diff.revoked) {
        speculative.delete(event.id);
      }
      for (const replacement of diff.replaced as readonly RollbackEventReplacement[]) {
        speculative.delete(replacement.before.id);
        speculative.add(replacement.after.id);
      }
      for (const event of diff.added) {
        speculative.add(event.id);
      }
    },
    confirmEvent: (event: RollbackWrappedEvent) => {
      speculative.delete(event.id);
    },
    diagnostics: () => ({ speculative_ids: Array.from(speculative) }),
  };
}

/** A bounded, boundary-keyed ring buffer mirroring `game/render/replay.lua`'s real `record_boundary`/`truncate_from`/`diagnostics`/`boundary_sample` semantics (insert-or-replace by boundary, evict oldest past capacity, truncate drops everything from a boundary onward). */
function recordingReplayPort(capacity: number): ReplayPort<FakeState, undefined> & {
  truncateCount: number;
} {
  const buf: { boundary: number; sample: RollbackValidationReplaySample }[] = [];
  const port = {
    truncateCount: 0,
    reset(): void {
      buf.length = 0;
    },
    capacity(): number {
      return capacity;
    },
    truncateFrom(boundary: number): void {
      let index = buf.findIndex((frame) => frame.boundary >= boundary);
      if (index === -1) {
        index = buf.length;
      } else {
        buf.splice(index);
      }
      port.truncateCount += 1;
    },
    recordBoundary(boundary: number, state: FakeState): void {
      const sample: RollbackValidationReplaySample = {
        boundary,
        ball_x: state.ball.x,
        ball_y: state.ball.y,
        score_home: state.score.home,
        score_away: state.score.away,
      };
      const existing = buf.findIndex((frame) => frame.boundary === boundary);
      if (existing !== -1) {
        buf[existing] = { boundary, sample };
      } else {
        buf.push({ boundary, sample });
        buf.sort((a, b) => a.boundary - b.boundary);
      }
      while (buf.length > capacity) {
        buf.shift();
      }
    },
    diagnostics(): { count: number; boundaries: readonly number[] } {
      return { count: buf.length, boundaries: buf.map((frame) => frame.boundary) };
    },
    boundarySample(boundary: number): RollbackValidationReplaySample | undefined {
      return buf.find((frame) => frame.boundary === boundary)?.sample;
    },
  };
  return port;
}

function newState(): FakeState {
  return {
    players: [
      { id: "home_1", team: "home", is_keeper: false },
      { id: "home_2", team: "home", is_keeper: false },
      { id: "away_1", team: "away", is_keeper: true },
      { id: "away_2", team: "away", is_keeper: false },
    ],
    events: [],
    score: { home: 0, away: 0 },
    input_tick: 0,
    ball: new Vec2(480, 270),
  };
}

function replayState(source: FakeState, boundary: number, ballX: number): FakeState {
  const state = newState();
  state.input_tick = boundary;
  state.ball = new Vec2(ballX, source.ball.y);
  state.score.home = source.score.home;
  state.score.away = source.score.away;
  return state;
}

function matchEvent(id: string, tick: number, kind: string, player?: string): RollbackWrappedEvent {
  return {
    id,
    tick,
    domain: `match/${kind}`,
    ordinal: 1,
    payload: { kind, x: 100 + tick, y: 200 + tick, player },
  };
}

function lifecycleEvent(
  id: string,
  tick: number,
  kind: "goal" | "kickoff" | "full_time",
  homeScore: number,
): RollbackWrappedEvent {
  return {
    id,
    tick,
    domain: `lifecycle/${kind}`,
    ordinal: 1,
    payload: { kind, team: kind === "goal" ? "home" : undefined, score: { home: homeScore, away: 0 } },
  };
}

function step(
  tick: number,
  score: number,
  ownerTeam: MatchTeam | undefined,
  matchEvents: readonly RollbackWrappedEvent[],
  lifecycleEvents: readonly RollbackWrappedEvent[],
  combatEvents?: readonly RollbackWrappedEvent[],
): RollbackEventStep {
  return {
    tick,
    state: { score: { home: score, away: 0 }, ...(ownerTeam !== undefined ? { owner_team: ownerTeam } : {}) },
    match_events: matchEvents,
    lifecycle_events: lifecycleEvents,
    ...(combatEvents !== undefined ? { combat_events: combatEvents } : {}),
  };
}

function diff(
  added: readonly RollbackWrappedEvent[],
  revoked?: readonly RollbackWrappedEvent[],
  replaced?: readonly RollbackEventReplacement[],
): RollbackEventDiff {
  return { added, revoked: revoked ?? [], replaced: replaced ?? [] };
}

function integrationPorts(
  replayCapacity: number,
): RollbackValidationPorts<FakeState, unknown, FakeState, unknown[]> & { replay: ReturnType<typeof recordingReplayPort> } {
  const matchSnapshot: MatchSnapshotPort<FakeState, unknown, FakeState> = {
    capture: (state) => state,
    restore: (snapshot) => snapshot,
    numberBytes: (value) => value.toFixed(6),
  };
  const rollbackEvents: RollbackEventsPort<unknown[], FakeState> = {
    create: () => [],
    apply: () => ({ ok: true, value: { added: [], revoked: [], replaced: [] } }),
    confirm: () => [],
  };
  return {
    matchSnapshot,
    rollbackEvents,
    effects: trackingEffectsPort(),
    replay: recordingReplayPort(replayCapacity),
    audio: new Audio(REAL_COMBAT_FEEDBACK),
    combatFeedback: REAL_COMBAT_FEEDBACK,
  };
}

describe("rollback validation (fixture-driven integration -- see this file's header)", () => {
  it("seeds its event audit from an atomic combat boundary", () => {
    const ports = integrationPorts(8);
    const initial = newState();
    const report = run(ports, initial, [], {
      home_team_id: "nebula",
      away_team_id: "orion",
      // `initial_combat_state` is opaque to this module (TCombatState is
      // fully generic -- see rollback_validation.ts's header); a real
      // combat companion snapshot would thread through identically.
      initial_combat_state: { players: [], projectiles: [] },
      reference_final_state: initial,
      impaired_final_state: initial,
    });

    expect(report.passed, report.errors.join("; ")).toBe(true);
  });

  it("audits corrected presentation consumers against independent authority", () => {
    const ports = integrationPorts(8);
    const initial = newState();
    const homePlayer = "home_2";
    const awayPlayer = "away_2";
    const awayKeeper = "away_1";
    const tackle = matchEvent("tackle-0", 0, "tackle", homePlayer);
    const shot = matchEvent("shot-1", 1, "shot", homePlayer);
    const header = matchEvent("header-2", 2, "header", awayPlayer);
    const catchEvt = matchEvent("catch-3", 3, "catch", awayKeeper);
    const goal = lifecycleEvent("goal-4", 4, "goal", 1);
    const kickoff = lifecycleEvent("kickoff-4", 4, "kickoff", 1);
    const fullTime = lifecycleEvent("full-time-5", 5, "full_time", 1);
    const confirmed = [
      step(0, 0, "home", [tackle], []),
      step(1, 0, "home", [shot], []),
      step(2, 0, "away", [header], []),
      step(3, 0, "away", [catchEvt], []),
      step(4, 1, "home", [], [goal, kickoff]),
      step(5, 1, undefined, [], [fullTime]),
    ];
    const staleTackle = matchEvent("stale-tackle", 0, "touch", homePlayer);
    const revokedShot = matchEvent("revoked-shot", 1, "shot", awayPlayer);

    const trace: RollbackValidationTraceRow<FakeState, FakeState>[] = [];
    for (const value of confirmed) {
      trace.push({ kind: "reference_confirmed", step: value });
    }
    trace.push({ kind: "impaired_diff", diff: diff([staleTackle]) });
    trace.push({ kind: "impaired_diff", diff: diff([], undefined, [{ before: staleTackle, after: tackle }]) });
    trace.push({ kind: "impaired_confirmed", step: confirmed[0]! });
    trace.push({ kind: "impaired_confirmed", step: confirmed[0]! });
    trace.push({ kind: "impaired_diff", diff: diff([revokedShot]) });
    trace.push({ kind: "impaired_diff", diff: diff([], [revokedShot]) });
    for (let index = 1; index < confirmed.length; index += 1) {
      const value = confirmed[index]!;
      const added = [...value.match_events, ...value.lifecycle_events];
      trace.push({ kind: "impaired_diff", diff: diff(added) });
      trace.push({ kind: "confirmed", step: value });
    }

    trace.push({ kind: "replay_boundary", boundary: 0, state: replayState(initial, 0, 10) });
    trace.push({ kind: "replay_boundary", boundary: 1, state: replayState(initial, 1, 11) });
    trace.push({ kind: "replay_boundary", boundary: 2, state: replayState(initial, 2, 12) });
    trace.push({ kind: "replay_truncate", boundary: 1 });
    trace.push({ kind: "replay_boundary", boundary: 1, state: replayState(initial, 1, 101) });
    trace.push({ kind: "replay_boundary", boundary: 2, state: replayState(initial, 2, 102) });

    const finalReference = newState();
    finalReference.score.home = 1;
    const finalImpaired = newState();
    finalImpaired.score.home = 1;

    const report = run(ports, initial, trace, {
      home_team_id: "nebula",
      away_team_id: "orion",
      reference_final_state: finalReference,
      impaired_final_state: finalImpaired,
      seed: 77,
      expected_replay_boundaries: [0, 1, 2],
      expected_replay_samples: [
        { boundary: 1, ball_x: 101, ball_y: initial.ball.y, score_home: 0, score_away: 0 },
        { boundary: 2, ball_x: 102, ball_y: initial.ball.y, score_home: 0, score_away: 0 },
      ],
      expected_replay_truncate_count: 1,
      required_scenarios: ["possession", "tackle", "shot", "goal", "kickoff", "aerial", "keeper", "full_time"],
    });

    expect(report.passed, report.errors.join("; ")).toBe(true);
    expect(report.errors.length).toBe(0);
    expect(report.events.reference_unique).toBe(7);
    expect(report.events.impaired_unique).toBe(7);
    expect(report.events.duplicate_confirmed).toBeGreaterThan(0);
    expect(report.events.speculative_replaced).toBe(1);
    expect(report.events.speculative_revoked).toBe(1);
    expect(report.events.terminal_speculative_residue).toBe(0);
    expect(report.events.consumer_speculative_residue).toBe(0);
    expect(report.consumers.audio_cue_count).toBe(report.consumers.expected_audio_cue_count);
    expect(report.observer_matched).toBe(true);
    expect(report.result_matched).toBe(true);
    expect(report.replay_matched).toBe(true);
    expect(report.consumers.replay_record_count).toBe(5);
    expect(report.consumers.replay_truncate_count).toBe(1);
    expect(report.replay_boundaries[0]).toBe(0);
    expect(report.replay_boundaries[2]).toBe(2);
  });

  it("builds the authoritative bounded replay timeline", () => {
    // `@gc/render`'s real `replay.ts` capacity (`MAX_SECONDS * 60`); a
    // declared dependency of this package already (package.json) --
    // `expected_replay_boundaries`'s own signature takes capacity as an
    // explicit parameter rather than importing `replay.ts` directly (see
    // rollback_validation.ts's header on why `TState` stays narrow), so the
    // caller supplies the real constant.
    const REPLAY_CAPACITY = 480;
    const boundaries = expectedReplayBoundaries(REPLAY_CAPACITY, 500);
    expect(boundaries.length).toBe(480);
    expect(boundaries[0]).toBe(21);
    expect(boundaries[boundaries.length - 1]).toBe(500);
  });

  it("reports missing confirmation and terminal speculative residue", () => {
    const ports = integrationPorts(8);
    const initial = newState();
    const player = "home_2";
    const authority = matchEvent("authority-shot", 0, "shot", player);
    const residue = matchEvent("residue-shot", 1, "shot", player);
    const reference = step(0, 0, "home", [authority], []);
    const trace: RollbackValidationTraceRow<FakeState, FakeState>[] = [
      { kind: "reference_confirmed", step: reference },
      { kind: "impaired_diff", diff: diff([authority, residue]) },
    ];

    const report = run(ports, initial, trace, {
      home_team_id: "nebula",
      away_team_id: "orion",
      reference_final_state: initial,
      impaired_final_state: initial,
      required_scenarios: ["shot", "full_time"],
    });

    expect(report.passed).toBe(false);
    expect(report.events.missing_confirmed).toBe(1);
    expect(report.events.terminal_speculative_residue).toBe(2);
    expect(report.events.consumer_speculative_residue).toBe(2);
    expect(report.errors.length).toBeGreaterThanOrEqual(4);
  });

  it("audits confirmed combat audio against the closed feedback disposition", () => {
    const ports = integrationPorts(8);
    const initial = newState();
    const guarded: RollbackWrappedEvent = {
      id: "combat-guarded",
      tick: 0,
      domain: "combat/contact/1",
      ordinal: 1,
      payload: {
        kind: "contact",
        tick: 0,
        family_id: "light_melee",
        source_index: 2,
        target_index: 7,
        source_sequence: 1,
        result: "guarded",
        x: 500,
        y: 300,
      },
    };
    const confirmed = step(0, 0, undefined, [], [], [guarded]);
    const report = run(
      ports,
      initial,
      [
        { kind: "reference_confirmed", step: confirmed },
        { kind: "impaired_diff", diff: diff([guarded]) },
        { kind: "confirmed", step: confirmed },
      ],
      {
        home_team_id: "nebula",
        away_team_id: "orion",
        reference_final_state: initial,
        impaired_final_state: initial,
      },
    );

    expect(report.passed, report.errors.join("; ")).toBe(true);
    expect(report.consumers.expected_audio_cues["combat_guarded"]).toBe(1);
    expect(report.consumers.audio_cues["combat_guarded"]).toBe(1);
    expect(report.consumers.expected_audio_cue_count).toBe(1);
    expect(report.consumers.audio_cue_count).toBe(1);
  });

  it("keeps confirmed lifecycle presentation idempotent at the screen boundary", () => {
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    const state = newMatchRollbackConsumerState();
    const effects: ScreenEffectsPort = {
      reset: () => {},
      resetVisuals: () => {},
      resetTrail: () => {},
      applyEventDiff: () => {},
      discardEventDiff: () => {},
      confirmEvent: () => {},
    };
    const replay: ScreenReplayPort = {
      active: () => false,
      startAt: () => false,
    };
    const ports = { effects, audio: audio as ScreenAudioPort, replay };

    const goal: RollbackWrappedLifecycleEvent = {
      id: "screen-goal",
      tick: 10,
      domain: "lifecycle/goal",
      ordinal: 1,
      payload: { kind: "goal", team: "home", score: { home: 1, away: 0 } },
    };
    const kickoff: RollbackWrappedLifecycleEvent = {
      id: "screen-kickoff",
      tick: 10,
      domain: "lifecycle/kickoff",
      ordinal: 1,
      payload: { kind: "kickoff" },
    };
    const fullTime: RollbackWrappedLifecycleEvent = {
      id: "screen-full-time",
      tick: 11,
      domain: "lifecycle/full_time",
      ordinal: 1,
      payload: { kind: "full_time" },
    };

    expect(consumeConfirmedLifecycle(state, ports, goal)).toBe(true);
    expect(consumeConfirmedLifecycle(state, ports, goal)).toBe(false);
    expect(consumeConfirmedLifecycle(state, ports, kickoff)).toBe(true);
    expect(consumeConfirmedLifecycle(state, ports, kickoff)).toBe(false);
    expect(consumeConfirmedLifecycle(state, ports, fullTime)).toBe(true);
    expect(consumeConfirmedLifecycle(state, ports, fullTime)).toBe(false);
    // `Match:full_time_confirmed()`'s source-driven branch is out of scope
    // this milestone (`MatchScreen` has no `_source`, see match.ts's
    // header); its pure analog is this consumer state's own flag.
    expect(state.presentation_full_time).toBe(true);

    const cues = audio.confirmedCueCounts();
    expect(cues["goal"]).toBe(1);
    expect(cues["kickoff"]).toBe(1);
    expect(cues["full_time"]).toBe(1);
  });
});
