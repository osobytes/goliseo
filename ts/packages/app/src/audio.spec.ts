// Coverage for `audio.ts`'s own control flow.
//
// Broader coverage of match/rollback/combat-feedback audio behavior belongs
// to `@gc/screens`'s match screen, `sim`'s match logic, and
// `@gc/presentation`'s `combat_feedback` -- not this package's to claim.
// `rollback_validation.spec.ts` exercises `Audio.consumeConfirmed`
// indirectly through `observeImpairedStep`; this file adds direct coverage
// of the "HEADLESS CONTRACT" (audio.ts's header) and the confirmed-cue
// bookkeeping that contract has to keep working even with no
// `AudioBackend` wired up, plus the two-way `setMuted`/`toggleMute` and
// `consumeCombat` paths no other spec in this package reaches.

import { describe, expect, it } from "vitest";
import { Audio, type CombatFeedbackPort, type RollbackWrappedEvent } from "./audio.ts";

function audioOf(value: unknown): { readonly audio?: string } {
  const audio = (value as { readonly audio?: string }).audio;
  return audio !== undefined ? { audio } : {};
}

const NOOP_COMBAT_FEEDBACK: CombatFeedbackPort = {
  link: (payload) => ({ disposition: audioOf(payload) }),
  disposition: (event) => audioOf(event),
};

function matchEvent(id: string, kind: string): RollbackWrappedEvent {
  return { id, domain: "match/pass", payload: { kind } };
}

describe("audio (headless contract)", () => {
  it("no-ops cleanly without a backend, but still counts confirmed cues", () => {
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    audio.load(); // must not throw with no backend
    audio.tick(1 / 60); // must not throw with no backend

    const consumed = audio.consumeConfirmed(matchEvent("evt-1", "pass"));
    expect(consumed).toBe(true);
    expect(audio.confirmedCueCounts()).toEqual({ pass: 1 });
  });

  it("deduplicates confirmed events by id", () => {
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    const event = matchEvent("evt-1", "shot");
    expect(audio.consumeConfirmed(event)).toBe(true);
    expect(audio.consumeConfirmed(event)).toBe(false);
    expect(audio.confirmedCueCounts()).toEqual({ shot: 1 });
  });

  it("routes suppressed playback to its own counter instead of the confirmed one", () => {
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    audio.consumeConfirmed(matchEvent("evt-1", "tackle"), true);
    expect(audio.confirmedCueCounts()).toEqual({});
    expect(audio.suppressedConfirmedCueCounts()).toEqual({ tackle: 1 });
  });

  it("recognizes lifecycle domains as goal/kickoff/full_time cues", () => {
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    audio.consumeConfirmed({ id: "g", domain: "lifecycle/goal", payload: {} });
    audio.consumeConfirmed({ id: "k", domain: "lifecycle/kickoff", payload: {} });
    audio.consumeConfirmed({ id: "f", domain: "lifecycle/full_time", payload: {} });
    expect(audio.confirmedCueCounts()).toEqual({ goal: 1, kickoff: 1, full_time: 1 });
  });

  it("resolves combat/ domain cues through the injected CombatFeedbackPort", () => {
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    const consumed = audio.consumeConfirmed({
      id: "c1",
      domain: "combat/hit",
      payload: { audio: "combat_hit" },
    });
    expect(consumed).toBe(true);
    expect(audio.confirmedCueCounts()).toEqual({ combat_hit: 1 });
  });

  it("ignores confirmed events whose cue cannot be resolved", () => {
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    const consumed = audio.consumeConfirmed(matchEvent("evt-1", "not_a_cue"));
    expect(consumed).toBe(true); // still consumed/deduplicated, just silent
    expect(audio.confirmedCueCounts()).toEqual({});
  });

  it("rejects a malformed confirmed event", () => {
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    expect(() => audio.consumeConfirmed({} as unknown as RollbackWrappedEvent)).toThrow();
  });

  it("toggles mute and reports the new state", () => {
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    expect(audio.setMuted(true)).toBe(true);
    expect(audio.toggleMute()).toBe(false);
    expect(audio.toggleMute()).toBe(true);
  });

  it("consumes a batch of combat feedback events without a backend", () => {
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    expect(() =>
      audio.consumeCombat([{ audio: "combat_commit" } as unknown as { kind?: string }]),
    ).not.toThrow();
  });

  it("resets confirmed-id, cue, and score-edge bookkeeping for a new match", () => {
    const audio = new Audio(NOOP_COMBAT_FEEDBACK);
    audio.consumeConfirmed(matchEvent("evt-1", "pass"));
    expect(audio.confirmedCueCounts()).toEqual({ pass: 1 });
    audio.reset();
    expect(audio.confirmedCueCounts()).toEqual({});
    // The id is no longer "seen" post-reset, so the same id can confirm again.
    expect(audio.consumeConfirmed(matchEvent("evt-1", "pass"))).toBe(true);
  });
});
