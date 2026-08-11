// Ported from spec/game/match_event_batch_spec.lua.

import { describe, expect, it } from "vitest";
import { matchEventBatch, type RollbackEventDiff, type RollbackWrappedEvent } from "./match_event_batch.ts";

interface TipEvent {
  readonly kind: "tip";
  readonly x: number;
  readonly y: number;
  readonly player: string;
}

function tip(id: string, y: number): RollbackWrappedEvent<TipEvent> {
  return {
    id,
    tick: 12,
    domain: "match/tip",
    ordinal: 1,
    payload: { kind: "tip", x: 24, y, player: "home_keeper" },
  };
}

function emptyDiff(): RollbackEventDiff {
  return { added: [], revoked: [], replaced: [] };
}

describe("rollback match-event presentation batch", () => {
  it("does not present a tip added then revoked in one render update", () => {
    const event = tip("tip/old", 220);
    const added: RollbackEventDiff = { ...emptyDiff(), added: [event] };
    const revoked: RollbackEventDiff = { ...emptyDiff(), revoked: [event] };

    expect(matchEventBatch.surviving([added, revoked])).toHaveLength(0);
  });

  it("presents only the corrected tip after added then replaced", () => {
    const before = tip("tip/old", 220);
    const after = tip("tip/new", 320);
    const added: RollbackEventDiff = { ...emptyDiff(), added: [before] };
    const replaced: RollbackEventDiff = { ...emptyDiff(), replaced: [{ before, after }] };

    const events = matchEventBatch.surviving([added, replaced]);
    expect(events).toHaveLength(1);
    const [survivor] = events;
    expect(survivor).toBeDefined();
    expect((survivor as unknown as TipEvent).kind).toBe("tip");
    expect((survivor as unknown as TipEvent).y).toBe(320);
  });
});
