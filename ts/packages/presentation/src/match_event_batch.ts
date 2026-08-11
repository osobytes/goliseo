// Reduces a sequence of rollback event diffs down to the match events that
// survived every correction, in deterministic (sorted-id) order. Purely a
// read of already-decided rollback output — it never decides which events
// are correct, only which ones are still standing after the fact.

/** The rollback-wrapped envelope shared by match/combat/lifecycle events. */
export interface RollbackWrappedEvent<TPayload = unknown> {
  readonly id: string;
  readonly tick: number;
  readonly domain: string;
  readonly ordinal: number;
  readonly payload: TPayload;
}

/** A match event's payload shape — owned by the (Rust) sim, kept minimal here. */
export interface MatchEvent {
  readonly kind: string;
}

export type RollbackWrappedMatchEvent = RollbackWrappedEvent<MatchEvent>;

export interface RollbackEventReplacement {
  readonly before: RollbackWrappedEvent;
  readonly after: RollbackWrappedEvent;
}

export interface RollbackEventDiff {
  readonly added: readonly RollbackWrappedEvent[];
  readonly revoked: readonly RollbackWrappedEvent[];
  readonly replaced: readonly RollbackEventReplacement[];
}

const MATCH_DOMAIN_PREFIX = "match/";

function surviving(diffs: readonly RollbackEventDiff[]): MatchEvent[] {
  const frameMatchEvents = new Map<string, RollbackWrappedMatchEvent>();
  for (const diff of diffs) {
    for (const revoked of diff.revoked) {
      frameMatchEvents.delete(revoked.id);
    }
    for (const replacement of diff.replaced) {
      frameMatchEvents.delete(replacement.before.id);
      const after = replacement.after;
      if (after.domain.startsWith(MATCH_DOMAIN_PREFIX)) {
        frameMatchEvents.set(after.id, after as RollbackWrappedMatchEvent);
      }
    }
    for (const added of diff.added) {
      if (added.domain.startsWith(MATCH_DOMAIN_PREFIX)) {
        frameMatchEvents.set(added.id, added as RollbackWrappedMatchEvent);
      }
    }
  }

  const eventIds = [...frameMatchEvents.keys()].sort();

  const events: MatchEvent[] = [];
  for (const id of eventIds) {
    const wrapped = frameMatchEvents.get(id);
    if (wrapped === undefined) {
      throw new Error("match event batch lost a tracked event id");
    }
    events.push(wrapped.payload);
  }
  return events;
}

export const matchEventBatch = { surviving };
