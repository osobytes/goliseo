// Ported from spec/game/rollback_validation_spec.lua.
//
// Every case in the Lua original builds its fixture from `sim.match`,
// `sim.combat`, `sim.match_snapshot`, `sim.rollback_events` (Rust-owned,
// `crates/gc-sim`; v2/README.md §2), `data.teams` (Rust-owned,
// `crates/gc-data`), `game.screens.match` (not yet ported to `@gc/screens`
// -- this package's porting report), and drives real rollback resimulation.
// None of that exists in TypeScript this milestone, and faking it well
// enough to reach "a corrected combat encounter" would mean
// reimplementing rollback scheduling here -- exactly what v2/README.md
// §2.1 says must never happen on this side of the determinism line. Every
// case is therefore ported as `it.skip`, matching the precedent
// `@gc/online`'s `match_presentation.spec.ts` set for the same situation.
//
// What is ported below, in the second describe block, is coverage of
// `rollback_validation.ts`'s own control flow -- audit initialization,
// speculative add/revoke/replace bookkeeping, and `finish`'s error
// aggregation -- against small hand-written fake ports, the same technique
// `match_presentation.spec.ts` uses.

import { describe, expect, it } from "vitest";
import { Vec2 } from "@gc/core";
import { Audio, type CombatFeedbackPort } from "./audio.ts";
import type { ObservedMatchState } from "./match_observer.ts";
import {
  applyImpairedDiff,
  newAudit,
  observeImpairedStep,
  observeReferenceStep,
  finish,
  type EffectsPort,
  type ReplayPort,
  type ReplaySampleState,
  type RollbackConfirmedStateView,
  type RollbackEventStep,
  type RollbackEventsPort,
  type MatchSnapshotPort,
  type RollbackValidationPorts,
  type RollbackWrappedEvent,
} from "./rollback_validation.ts";

describe("rollback validation (cross-boundary integration)", () => {
  // Needs sim.match/sim.combat/sim.match_snapshot/sim.rollback_events
  // (Rust-owned) and game.screens.match (not yet ported) -- see this
  // file's header.
  it.skip("seeds its event audit from an atomic combat boundary", () => {});
  it.skip("audits corrected presentation consumers against independent authority", () => {});
  it.skip("builds the authoritative bounded replay timeline", () => {});
  it.skip("reports missing confirmation and terminal speculative residue", () => {});
  it.skip("derives reference identities from raw campaign step inputs", () => {});
  it.skip("audits confirmed combat audio against the closed feedback disposition", () => {});
  it.skip("keeps confirmed lifecycle presentation idempotent at the screen boundary", () => {});
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
