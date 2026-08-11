// Ported from spec/game/combat_feedback_rollback_spec.lua.
//
// The Lua spec drives a real `Match.new({ rollback_lab = ... })` screen
// against `game/render/effects.lua`, `game/audio.lua`, and
// `game/render/replay.lua`. `match.ts` ports only the rollback-consumption
// seam this milestone (see its header); those three modules do not exist in
// TypeScript yet (`effects`/`audio`) or are not a declared dependency of
// this package (`replay` -- `@gc/render`), so `match.ts` takes them as
// injected `EffectsPort`/`AudioPort`/`ReplayPort`.
//
// Every assertion below is ported as a real test against small
// hand-written fakes for those three ports (mirroring
// `match_presentation.spec.ts`'s `fakeRollbackEvents`/`fakeMatchDriver`):
// the fakes track event identity and cue counts, not real particle physics
// or DSP, which is exactly enough to prove `match.ts`'s own dedup/ledger
// logic -- the actual subject of this spec -- without reimplementing
// `effects.lua`/`audio.lua`. `combat_feedback` itself is real
// (`@gc/presentation`, a declared dependency), so every assertion that
// reads `combat_feedback.diagnostics(...)`/`combat_feedback.notice(...)`
// exercises the genuine module.
//
// `combat_feedback_fixture.lua`'s `state`/`combat`/occluder fields need
// `sim.match`/`sim.combat` (Rust) to build; this spec only ever reads
// `fixture.events`, five hand-built `RollbackWrappedCombatEvent` literals,
// which are ported verbatim below with no sim dependency at all.
//
// One adaptation: the Lua spec's last case reads `value.state.controlled`
// -- the real `Match` screen's own controlled-player index, which comes
// from `sim.match.new`'s default and has no equivalent on this port's
// scoped-down state (see match.ts's header). It is replaced with the
// fixture event's own `source_index`, which is the field `combat_feedback`
// actually keys a notice on -- see the inline comment at that assertion.

import { describe, expect, it } from "vitest";
import { combatFeedback } from "@gc/presentation";
import type { CombatEvent, RollbackEventDiff } from "@gc/presentation";
import {
  consumeConfirmedLifecycle,
  consumeConfirmedStep,
  consumeRollbackEventDiff,
  newMatchRollbackConsumerState,
  type AudioPort,
  type EffectsPort,
  type MatchRollbackConsumerPorts,
  type ReplayPort,
  type RollbackEventStep,
  type RollbackWrappedCombatEvent,
  type RollbackWrappedLifecycleEvent,
  type RollbackWrappedMatchEvent,
} from "./match.ts";

// --- spec/fixtures/crowded_combat_feedback.lua (-> game/presentation/combat_feedback_fixture.lua) ---
//
// Only `fixture.events` is ported -- see this file's header. `wrapped(id,
// ordinal, event)` inlined per literal.

function wrapped(id: string, ordinal: number, event: CombatEvent): RollbackWrappedCombatEvent {
  return { id, tick: event.tick, domain: `combat/${event.kind}/${event.source_sequence ?? 0}`, ordinal, payload: event };
}

const FIXTURE_BALL = { x: 480, y: 270 };

const fixtureEvents: readonly RollbackWrappedCombatEvent[] = [
  wrapped("crowded/commit/1", 1, {
    kind: "commit",
    tick: 20,
    family_id: "unarmed",
    source_index: 2,
    source_sequence: 1,
    x: 220,
    y: 190,
  }),
  wrapped("crowded/guarded/2", 2, {
    kind: "contact",
    tick: 20,
    family_id: "light_melee",
    source_index: 3,
    target_index: 7,
    source_sequence: 2,
    result: "guarded",
    x: 300,
    y: 230,
  }),
  wrapped("crowded/hit/3", 3, {
    kind: "contact",
    tick: 20,
    family_id: "ranged",
    source_index: 4,
    target_index: 8,
    source_sequence: 3,
    result: "hit",
    x: 306,
    y: 234,
  }),
  wrapped("crowded/spill/3", 4, {
    kind: "ball_spill",
    tick: 20,
    family_id: "ranged",
    source_index: 4,
    target_index: 8,
    source_sequence: 3,
    x: FIXTURE_BALL.x,
    y: FIXTURE_BALL.y,
  }),
  wrapped("crowded/forced/3", 5, {
    kind: "forced",
    tick: 20,
    family_id: "ranged",
    source_index: 4,
    target_index: 8,
    source_sequence: 3,
    x: 710,
    y: 360,
  }),
];

// ---------------------------------------------------------------------------
// Fake ports: event-identity and cue-count bookkeeping only. See this
// file's header.
// ---------------------------------------------------------------------------

class FakeEffects implements EffectsPort {
  private speculative = new Map<string, unknown>();
  private confirmed = new Set<string>();
  private particleCount = 0;

  reset(): void {
    this.speculative.clear();
    this.confirmed.clear();
    this.particleCount = 0;
  }
  resetVisuals(): void {
    this.particleCount = 0;
  }
  resetTrail(): void {
    /* no fake particle trail to clear */
  }
  applyEventDiff(diff: RollbackEventDiff): void {
    for (const revoked of diff.revoked) {
      this.speculative.delete(revoked.id);
    }
    for (const replacement of diff.replaced) {
      this.speculative.delete(replacement.before.id);
      this.speculative.set(replacement.after.id, replacement.after);
      this.particleCount += 1;
    }
    for (const added of diff.added) {
      this.speculative.set(added.id, added);
      this.particleCount += 1;
    }
  }
  discardEventDiff(): void {
    /* replay owns the screen: no live visual is spawned */
  }
  confirmEvent(event: RollbackWrappedMatchEvent | RollbackWrappedCombatEvent): void {
    this.speculative.delete(event.id);
    this.confirmed.add(event.id);
  }
  diagnostics(): { readonly particle_count: number; readonly speculative_ids: readonly string[]; readonly confirmed_ids: readonly string[] } {
    return {
      particle_count: this.particleCount,
      speculative_ids: [...this.speculative.keys()].sort(),
      confirmed_ids: [...this.confirmed.keys()].sort(),
    };
  }
}

class FakeAudio implements AudioPort {
  private seen = new Set<string>();
  private confirmedCounts = new Map<string, number>();
  private suppressedCounts = new Map<string, number>();

  reset(): void {
    this.seen.clear();
    this.confirmedCounts.clear();
    this.suppressedCounts.clear();
  }

  private cueFor(event: RollbackWrappedMatchEvent | RollbackWrappedCombatEvent | RollbackWrappedLifecycleEvent): string {
    const kind = (event.payload as { readonly kind: string }).kind;
    if (kind === "goal" || kind === "kickoff" || kind === "full_time") {
      return kind;
    }
    const link = combatFeedback.link(event.payload as CombatEvent, event.id);
    return link.disposition.audio ?? "unknown";
  }

  consumeConfirmed(
    event: RollbackWrappedMatchEvent | RollbackWrappedCombatEvent | RollbackWrappedLifecycleEvent,
    replayOwnsScreen?: boolean
  ): boolean {
    if (this.seen.has(event.id)) {
      return false;
    }
    this.seen.add(event.id);
    const cue = this.cueFor(event);
    const bucket = replayOwnsScreen === true ? this.suppressedCounts : this.confirmedCounts;
    bucket.set(cue, (bucket.get(cue) ?? 0) + 1);
    return true;
  }

  confirmedCueCounts(): Readonly<Record<string, number>> {
    return Object.fromEntries(this.confirmedCounts);
  }
  suppressedConfirmedCueCounts(): Readonly<Record<string, number>> {
    return Object.fromEntries(this.suppressedCounts);
  }
}

class FakeReplay implements ReplayPort {
  private recordedBoundaries = new Set<number>();
  private activeFlag = false;

  reset(): void {
    this.recordedBoundaries.clear();
    this.activeFlag = false;
  }
  recordBoundary(boundary: number): void {
    this.recordedBoundaries.add(boundary);
  }
  active(): boolean {
    return this.activeFlag;
  }
  startAt(_team: "home" | "away", tick: number): boolean {
    if (this.recordedBoundaries.has(tick)) {
      this.activeFlag = true;
      return true;
    }
    return false;
  }
}

function step(event: RollbackWrappedCombatEvent): RollbackEventStep {
  return {
    tick: event.tick,
    start_boundary: event.tick,
    end_boundary: event.tick + 1,
    state: { score: { home: 0, away: 0 }, time_left: 100, finished: false },
    match_events: [],
    combat_events: [event],
    lifecycle_events: [],
  };
}

function confirmedStep(
  tick: number,
  combatEvents: readonly RollbackWrappedCombatEvent[],
  lifecycleEvents: readonly RollbackWrappedLifecycleEvent[]
): RollbackEventStep {
  return {
    tick,
    start_boundary: tick,
    end_boundary: tick + 1,
    state: { score: { home: 0, away: 0 }, time_left: 100, finished: false },
    match_events: [],
    combat_events: combatEvents,
    lifecycle_events: lifecycleEvents,
  };
}

function goal(id: string, tick: number): RollbackWrappedLifecycleEvent {
  return {
    id,
    tick,
    domain: "lifecycle/goal",
    ordinal: 1,
    payload: { kind: "goal", team: "home", score: { home: 1, away: 0 } },
  };
}

describe("combat feedback rollback presentation", () => {
  it("revokes corrected-away effects and publishes the survivor exactly once", () => {
    const effects = new FakeEffects();
    const audio = new FakeAudio();
    const replay = new FakeReplay();
    const ports: MatchRollbackConsumerPorts = { effects, audio, replay };
    const value = newMatchRollbackConsumerState();
    const predicted = fixtureEvents[2];
    const corrected = fixtureEvents[1];
    if (predicted === undefined || corrected === undefined) {
      throw new Error("fixture missing events");
    }

    expect(consumeRollbackEventDiff(value, ports, true, { added: [predicted], revoked: [], replaced: [] })).toBe(true);
    expect(effects.diagnostics().particle_count).toBeGreaterThan(0);
    expect(
      consumeRollbackEventDiff(value, ports, true, { added: [], revoked: [], replaced: [{ before: predicted, after: corrected }] })
    ).toBe(true);
    const correctedCount = effects.diagnostics().particle_count;
    expect(correctedCount).toBeGreaterThan(0);

    expect(consumeConfirmedStep(value, ports, step(corrected))).toBe(1);
    const confirmedCount = effects.diagnostics().particle_count;
    expect(consumeConfirmedStep(value, ports, step(corrected))).toBe(0);
    expect(effects.diagnostics().particle_count).toBe(confirmedCount);
    expect(audio.confirmedCueCounts()["combat_guarded"]).toBe(1);
    expect(combatFeedback.diagnostics(value.combat_feedback).confirmed_count).toBe(1);
    // corrected-away result cannot publish audio
    expect(audio.confirmedCueCounts()["combat_hit"]).toBeUndefined();
  });

  it("suppresses live combat feedback while confirmed replay owns the screen", () => {
    const effects = new FakeEffects();
    const audio = new FakeAudio();
    const replay = new FakeReplay();
    const ports: MatchRollbackConsumerPorts = { effects, audio, replay };
    const value = newMatchRollbackConsumerState();
    for (let boundary = 1; boundary <= 40; boundary += 1) {
      replay.recordBoundary(boundary);
    }
    expect(consumeConfirmedLifecycle(value, ports, goal("fixture/goal", 40))).toBe(true);
    expect(replay.active()).toBe(true);

    const live = fixtureEvents[2];
    if (live === undefined) {
      throw new Error("fixture missing events");
    }
    expect(consumeRollbackEventDiff(value, ports, true, { added: [live], revoked: [], replaced: [] })).toBe(false);
    expect(effects.diagnostics().particle_count).toBe(0);
    expect(consumeConfirmedStep(value, ports, step(live))).toBe(1);
    expect(effects.diagnostics().particle_count).toBe(0);
    expect(combatFeedback.diagnostics(value.combat_feedback).notice_id).toBeUndefined();
    expect(combatFeedback.diagnostics(value.combat_feedback).impulse_id).toBeUndefined();
    expect(audio.confirmedCueCounts()["combat_hit"]).toBeUndefined();
    expect(audio.suppressedConfirmedCueCounts()["combat_hit"]).toBe(1);
  });

  it("preserves speculative identity while replay clears live visuals", () => {
    const effects = new FakeEffects();
    const audio = new FakeAudio();
    const replay = new FakeReplay();
    const ports: MatchRollbackConsumerPorts = { effects, audio, replay };
    const value = newMatchRollbackConsumerState();
    for (let boundary = 1; boundary <= 40; boundary += 1) {
      replay.recordBoundary(boundary);
    }
    const live = fixtureEvents[2];
    if (live === undefined) {
      throw new Error("fixture missing events");
    }
    expect(consumeRollbackEventDiff(value, ports, true, { added: [live], revoked: [], replaced: [] })).toBe(true);
    expect(effects.diagnostics().particle_count).toBeGreaterThan(0);
    expect(effects.diagnostics().speculative_ids[0]).toBe(live.id);

    expect(consumeConfirmedLifecycle(value, ports, goal("ledger/goal", 40))).toBe(true);
    const entered = effects.diagnostics();
    expect(replay.active()).toBe(true);
    expect(entered.particle_count).toBe(0);
    // replay entry preserves the identity ledger
    expect(entered.speculative_ids[0]).toBe(live.id);

    expect(consumeConfirmedStep(value, ports, step(live))).toBe(1);
    const confirmed = effects.diagnostics();
    expect(confirmed.particle_count).toBe(0);
    expect(confirmed.speculative_ids.length).toBe(0);
    expect(confirmed.confirmed_ids[0]).toBe(live.id);
    expect(consumeConfirmedStep(value, ports, step(live))).toBe(0);
    // duplicate confirmation stays suppressed
    expect(effects.diagnostics().particle_count).toBe(0);
  });

  it("lets lifecycle replay ownership suppress same-batch combat feedback", () => {
    const effects = new FakeEffects();
    const audio = new FakeAudio();
    const replay = new FakeReplay();
    const ports: MatchRollbackConsumerPorts = { effects, audio, replay };
    const value = newMatchRollbackConsumerState();
    for (let boundary = 1; boundary <= 40; boundary += 1) {
      replay.recordBoundary(boundary);
    }
    const live = fixtureEvents[2];
    if (live === undefined) {
      throw new Error("fixture missing events");
    }
    const batch = confirmedStep(40, [live], [goal("same-batch/goal", 40)]);

    expect(consumeConfirmedStep(value, ports, batch)).toBe(2);
    expect(replay.active()).toBe(true);
    expect(effects.diagnostics().particle_count).toBe(0);
    // Any controlled-player index reads no notice: the same-batch lifecycle
    // reset cleared them all.
    expect(combatFeedback.notice(value.combat_feedback, 0)).toBeNull();
    expect(audio.confirmedCueCounts()["combat_hit"]).toBeUndefined();
    expect(audio.suppressedConfirmedCueCounts()["combat_hit"]).toBe(1);
    expect(audio.confirmedCueCounts()["goal"]).toBe(1);
  });

  it("retains a controlled-player notice behind unrelated same-step feedback", () => {
    const effects = new FakeEffects();
    const audio = new FakeAudio();
    const replay = new FakeReplay();
    const ports: MatchRollbackConsumerPorts = { effects, audio, replay };
    const value = newMatchRollbackConsumerState();
    const controlled = fixtureEvents[0];
    const unrelated = fixtureEvents[2];
    if (controlled === undefined || unrelated === undefined) {
      throw new Error("fixture missing events");
    }

    expect(consumeConfirmedStep(value, ports, confirmedStep(controlled.tick, [controlled, unrelated], []))).toBe(2);

    // Substitutes for the Lua spec's `value.state.controlled` (the real
    // match's default controlled-player index, unavailable on this port's
    // scoped-down state -- see this file's header) with the field
    // `combat_feedback.notice` actually keys on: the commit event's own
    // `source_index`.
    const controlledIndex = controlled.payload.source_index;
    if (controlledIndex === undefined) {
      throw new Error("fixture commit event must carry a source_index");
    }
    const notice = combatFeedback.notice(value.combat_feedback, controlledIndex);
    expect(notice).not.toBeNull();
    expect(notice?.id).toBe(controlled.id);
    expect(notice?.text).toBe("EQUIPMENT COMMITTED");
    expect(combatFeedback.diagnostics(value.combat_feedback).notice_count).toBe(2);
  });
});
