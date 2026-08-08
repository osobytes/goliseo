// Ported from game/render/release_follow.lua.
//
// Presentation-only follow-through window for outfield ball releases.
//
// A pass/cross/shot is a one-tick `MatchEvent`; the striking leg needs to
// stay extended for a beat after it. That beat is a *render* fact, so it
// lives here next to `view_state` instead of becoming persistent simulation
// state: nothing in `sim/` reads it, it never enters `MatchSnapshot`, and it
// therefore cannot move a hash, a replay, or a rollback resimulation.
//
// The window is fed from the same frame batch the renderer already draws
// from (`match_event_batch.surviving` under rollback, `MatchState.events`
// offline), so an event revoked before it reaches presentation never opens
// one. A window opened by an event that is revoked afterwards ages out on
// its own within `DURATION`, exactly like an already-spawned particle in
// `game/render/effects.lua`.
//
// A cancelled wind-up emits no release event, so it never shows
// follow-through.
//
// Boundary note: `MatchEvent` is a sim shape (v2/README.md rule 6.7,
// `sim/**` -> Rust `crates/gc-sim`). Only the fields this module reads
// (`kind`, `player`) are declared locally.

export interface ReleaseFollowEvent {
  readonly kind: string;
  readonly player?: string;
}

/** Seconds a release stays visible after the ball leaves the boot. */
const DURATION = 0.15;

const remaining = new Map<string, number>();

/** Presentation-only follow-through window for outfield ball releases. See file header. */
export const releaseFollow = {
  DURATION,

  // Age the open windows by the render dt, then latch any release in this
  // frame's batch. Ageing first keeps a same-frame release at its full
  // duration.
  update(events: readonly ReleaseFollowEvent[], dt: number): void {
    if (dt > 0) {
      for (const [id, left] of remaining) {
        const nextLeft = left - dt;
        if (nextLeft > 0) {
          remaining.set(id, nextLeft);
        } else {
          remaining.delete(id);
        }
      }
    }
    for (const event of events) {
      if ((event.kind === "shot" || event.kind === "pass") && event.player !== undefined) {
        remaining.set(event.player, DURATION);
      }
    }
  },

  active(id: string): boolean {
    return (remaining.get(id) ?? 0) > 0;
  },

  // Every open window as a plain id -> true map, the shape `render.frame`
  // takes as its `kick_follow` input. Handing the payload builder a snapshot
  // keeps this module's state renderer-owned: the builder never reaches
  // back in here.
  windows(): Readonly<Record<string, true>> {
    const open: Record<string, true> = {};
    for (const [id, left] of remaining) {
      if (left > 0) {
        open[id] = true;
      }
    }
    return open;
  },

  // Drop every window (fresh match, kickoff, correction reset, replay
  // boundary) so a follow-through can never survive the timeline that
  // produced it.
  reset(): void {
    remaining.clear();
  },
};
