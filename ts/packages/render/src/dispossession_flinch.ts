// Presentation-only dispossession flinch for the standing-poke tackle (#591).
//
// `win_ball` (`rust/crates/gc-sim/src/match.rs`) pops the ball loose and
// pushes a `MatchEventKind::Tackle` event naming the CHALLENGER, but only a
// SLIDE tackle sets the victim's `stun_timer` (`sliding` in `win_ball`) --
// the standing poke deliberately never slows or knocks down the player it
// robs. The result: the victim of a poke showed no reaction at all (a
// playtest report literally read "I never saw my player fall or be affected
// by a tackle"), even though `player_pose.rs` already has a `stumble` pose
// wired end to end for the slide's own knockdown.
//
// This module is the standing poke's presentation-only second producer for
// that SAME pose: it tracks who owned the ball just before a `tackle` event
// fires and opens a short window for them, mirroring `release_follow.ts`'s
// shape almost exactly (a per-id remaining-seconds map, aged then latched,
// reduced to a roster-slot bitmask for the wasm per-frame path). Nothing
// here enters a snapshot, a hash or a rollback resimulation, and nothing
// here ever touches `MatchPlayer::stun_timer` -- nothing here is simulation.
//
// ## Why "the owner just before this batch" and not the event itself
//
// The `Tackle` event names the CHALLENGER (`event.player`), not the victim,
// and by the time a render frame is built the sim's own `owner` is already
// `None` (`win_ball` clears it in the same tick that pushes the event) -- so
// there is no field on the event or the frame that already says who lost the
// ball. What the frame DOES carry, batch to batch, is `possession.owner`:
// the same "possession/last-owner" data #591 points at. This module remembers
// the owner id it was handed on the PREVIOUS call and, on seeing a `tackle`
// event, treats that remembered id as the victim -- exactly the owner a
// human watching the previous frame would have seen carrying the ball. It
// then overwrites the remembered id with whatever `update` was handed THIS
// call, ready for the next tackle.
//
// This is a per-render-frame heuristic, not a per-tick one: if several ticks
// land inside one render frame (rollback resimulation) and more than one
// tackle fires in that batch, every tackle in the batch resolves against the
// SAME remembered owner -- the one before the whole batch, not the one
// between two tackles inside it. That is the same granularity
// `release_follow.ts` already accepts for its own per-tick event batch, and
// a wrong or missing flinch here is a missed beat of presentation, never a
// wrong simulation result.

export interface DispossessionEvent {
  readonly kind: string;
  readonly player?: string;
}

/** Seconds a flinch stays visible after a standing-poke tackle takes the ball. Within the 0.25-0.4s window #591 asks for. */
const DURATION = 0.3;

const remaining = new Map<string, number>();

/** The ball owner's id as of the last `update` call -- see this file's header ("owner just before this batch"). `undefined` when nobody owned the ball. */
let lastOwnerId: string | undefined;

/** Presentation-only dispossession flinch window for the standing-poke tackle. See file header. */
export const dispossessionFlinch = {
  DURATION,

  // Age the open windows by the render dt, then latch a flinch for
  // whoever owned the ball just before THIS batch, for every `tackle` event
  // in it. Ageing first keeps a same-frame tackle at its full duration,
  // mirroring `release_follow.update`'s own ordering exactly. `ownerId` is
  // recorded AFTER resolving this call's tackles, so the next call still
  // sees the owner from before this one.
  update(events: readonly DispossessionEvent[], dt: number, ownerId: string | undefined): void {
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
      if (event.kind === "tackle" && lastOwnerId !== undefined && lastOwnerId !== event.player) {
        remaining.set(lastOwnerId, DURATION);
      }
    }
    lastOwnerId = ownerId;
  },

  active(id: string): boolean {
    return (remaining.get(id) ?? 0) > 0;
  },

  // Every open window as a plain id -> true map -- mirrors
  // `releaseFollow.windows()`'s own snapshot contract.
  windows(): Readonly<Record<string, true>> {
    const open: Record<string, true> = {};
    for (const [id, left] of remaining) {
      if (left > 0) {
        open[id] = true;
      }
    }
    return open;
  },

  // The same snapshot as `windows()`, as a ROSTER-SLOT BITMASK -- the shape
  // `gc_wasm::session::dispossessed_ids` resolves back into ids on the wasm
  // side, exactly parallel to `releaseFollow.slotMask` and
  // `gc_wasm::session::kick_follow_ids`. See that module's own doc for why a
  // bitmask crosses the boundary instead of the ids themselves, and why
  // slots past 32 are dropped rather than aliased.
  slotMask(rosterIds: readonly string[]): number {
    let mask = 0;
    const count = Math.min(rosterIds.length, 32);
    for (let index = 0; index < count; index += 1) {
      const id = rosterIds[index];
      if (id !== undefined && (remaining.get(id) ?? 0) > 0) {
        mask = (mask | (1 << index)) >>> 0;
      }
    }
    return mask;
  },

  // Drop every window AND the remembered owner (fresh match, kickoff,
  // correction reset, replay boundary) so a flinch can never survive the
  // timeline that produced it, and so a stale remembered owner from a
  // finished match can never be blamed for a tackle in the next one.
  reset(): void {
    remaining.clear();
    lastOwnerId = undefined;
  },
};
