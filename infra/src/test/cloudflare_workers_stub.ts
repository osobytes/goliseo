//! A minimal stand-in for the `cloudflare:workers` built-in module,
//! resolved ONLY inside vitest (see `../../vitest.config.ts`'s alias).
//!
//! `index.ts` re-exports `RoomDurableObject` from `room_durable_object.ts`,
//! which imports the real `DurableObject` base class from
//! `cloudflare:workers` -- a module that only exists inside an actual
//! Workers/workerd runtime, never plain Node. Because importing anything
//! from `index.ts` pulls in that whole module graph, `index.spec.ts`
//! (which tests `handleHostSignal`/`handleJoinSignal`, neither of which
//! touches `RoomDurableObject`'s own behavior) would fail to even load
//! without this. `RoomDurableObject`'s actual behavior is exercised by
//! hand via `wrangler dev` -- see `vitest.config.ts`'s module doc.
export class DurableObject<Env = unknown> {
  public constructor(
    protected readonly ctx: unknown,
    protected readonly env: Env,
  ) {}
}
