// Exercises `SimHostPort` end to end over the real compiled `@gc/wasm`
// artifact (requires `pnpm --filter @gc/wasm build` to have run first, same
// precondition `packages/wasm/src/session.spec.ts` documents).

import { afterEach, describe, expect, it } from "vitest";
import { loadSimHost } from "@gc/wasm";
import { releaseFollow } from "@gc/render";
import { createSimHost } from "./sim_host.ts";

const HOME = "nebula";
const AWAY = "orion";

function neutralSample(): { move_x: number; move_y: number; held: number; edges: number } {
  return { move_x: 0, move_y: 0, held: 0, edges: 0 };
}

describe("createSimHost", () => {
  it("constructs, steps, decodes frames, and advances its tick counter", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      expect(host.tick()).toBe(0);

      for (let i = 0; i < 5; i += 1) {
        host.step(neutralSample());
      }
      expect(host.tick()).toBe(5);

      const roster = host.roster();
      expect(roster.ids.length).toBeGreaterThan(0);
      expect(roster.teams.length).toBe(roster.ids.length);
      expect(roster.teams).toContain("home");
      expect(roster.teams).toContain("away");

      const frame = host.frame();
      expect(frame.field.w).toBeGreaterThan(0);
      expect(frame.field.h).toBeGreaterThan(0);
      expect(frame.players.count).toBeGreaterThan(0);
      expect(frame.players.x.length).toBe(frame.players.count);
      for (const x of frame.players.x) {
        expect(Number.isFinite(x)).toBe(true);
      }
      for (const y of frame.players.y) {
        expect(Number.isFinite(y)).toBe(true);
      }
      expect(Number.isFinite(frame.ball.x)).toBe(true);
      expect(Number.isFinite(frame.ball.y)).toBe(true);
      expect(frame.roster).toBe(roster);
    } finally {
      host.dispose();
    }
  });

  it("roster() decodes once and returns the same reused object on later calls", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      const first = host.roster();
      const second = host.roster();
      expect(first).toBe(second);
    } finally {
      host.dispose();
    }
  });

  it("frame() reuses the decoded object when called again without stepping", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      host.step(neutralSample());
      const first = host.frame();
      const second = host.frame();
      expect(first).toBe(second);

      host.step(neutralSample());
      const third = host.frame();
      expect(third).not.toBe(second);
    } finally {
      host.dispose();
    }
  });

  it("drives the configured local slot and leaves the rest neutral (rejects a malformed sample loudly)", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3, { localSlot: 3 });
    try {
      // `held` must fit an 8-bit mask (`gc_sim::input_frame::validate_sample`,
      // mirrored by `@gc/input`'s `validateSample`): this is expected,
      // recoverable misuse the port must fail loudly on, not swallow.
      expect(() => host.step({ move_x: 0, move_y: 0, held: 999, edges: 0 })).toThrow();
      // The session must not have advanced on a rejected sample.
      expect(host.tick()).toBe(0);

      host.step({ move_x: 50, move_y: -20, held: 0, edges: 0 });
      expect(host.tick()).toBe(1);
    } finally {
      host.dispose();
    }
  });

  // `crates/gc-wasm/src/session.rs`'s `Session::new` gained a `combat_enabled`
  // parameter, and `sim_host.ts`'s `SimHostOptions.combatEnabled` threads it
  // through directly -- `packages/wasm/src/types.ts`'s `SimSessionConstructor`
  // now declares this seventh parameter itself, so `sim_host.ts` calls the
  // real wasm constructor with no local type-widening cast (confirmed by
  // reading `types.ts` directly; the temporary cast that used to live there
  // is gone).
  // Neither `SimSession` nor `SimHostPort` exposes a getter for combat
  // presence or per-tick combat state (confirmed by reading `session.rs`
  // directly), so this is a construction/threading smoke test, not a
  // behavioral one -- the same narrower claim `@gc/screens`'s
  // `match_screen.spec.ts` and `@gc/app`'s `bootstrap.spec.ts` settle for.
  // It mirrors `crates/gc-wasm/src/session.rs`'s own
  // `combat_opt_in_tests` module: proving the seventh constructor argument
  // reaches the real wasm session without throwing or otherwise disturbing
  // stepping, for both `true` and omitted/`false`.
  it("threads combatEnabled to the real wasm session without disturbing stepping", () => {
    const withCombat = createSimHost(HOME, AWAY, 7, 20, 3, { combatEnabled: true });
    try {
      for (let i = 0; i < 5; i += 1) {
        withCombat.step(neutralSample());
      }
      expect(withCombat.tick()).toBe(5);
    } finally {
      withCombat.dispose();
    }

    const withoutCombat = createSimHost(HOME, AWAY, 7, 20, 3, { combatEnabled: false });
    try {
      withoutCombat.step(neutralSample());
      expect(withoutCombat.tick()).toBe(1);
    } finally {
      withoutCombat.dispose();
    }

    // Omitted must reproduce the exact prior behavior -- no `combatEnabled`
    // key at all, not merely `combatEnabled: false`.
    const omitted = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      omitted.step(neutralSample());
      expect(omitted.tick()).toBe(1);
    } finally {
      omitted.dispose();
    }
  });

  // `crates/gc-wasm/src/session.rs`'s `Session::new` grew `home_formation`/
  // `tactic`/`away_tactic`/`home_starter_ids` as its sixth/eighth/ninth/
  // tenth optional parameters this wave, and `SimHostOptions` (this file)
  // now threads all four through -- `packages/wasm/src/types.ts`'s
  // `SimSessionConstructor` declares them directly (confirmed by reading
  // `types.ts` and the regenerated `dist/pkg/gc_wasm.d.cts` after
  // rebuilding), so no local widening cast is needed here either.
  // `matchStateJson()` (also new this wave) is what makes this provable at
  // all: `press` is set at construction, from the tactic, with no stepping
  // required -- `crates/gc-sim/src/match.rs`'s `new` sets
  // `press: {home: home_tactic.press, away: away_tactic.press}` directly.
  it("threads tactic/awayTactic/homeFormation/homeStarterIds to the real wasm session, observable via matchStateJson", () => {
    const STARTERS = ["ozzo", "veil_nyx", "rok_tann", "mika_olu", "sela_dwin"];

    const balanced = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      const raw = balanced.matchStateJson?.();
      if (raw === undefined) {
        throw new Error("expected matchStateJson() on a WasmSimHost");
      }
      const state = JSON.parse(raw) as { readonly press: { readonly home: number; readonly away: number } };
      // Omitted tactic/awayTactic reproduce the exact prior behavior:
      // "balanced" on both sides.
      expect(state.press.home).toBe(1);
      expect(state.press.away).toBe(1);
    } finally {
      balanced.dispose();
    }

    const pressHigh = createSimHost(HOME, AWAY, 7, 20, 3, {
      homeFormation: "1-2-1",
      tactic: "press_high",
      awayTactic: "press_high",
      homeStarterIds: STARTERS,
    });
    try {
      const raw = pressHigh.matchStateJson?.();
      if (raw === undefined) {
        throw new Error("expected matchStateJson() on a WasmSimHost");
      }
      const state = JSON.parse(raw) as {
        readonly players: readonly { readonly id: string }[];
        readonly press: { readonly home: number; readonly away: number };
      };
      expect(state.press.home).toBe(2);
      expect(state.press.away).toBe(2);
      expect(state.players[0]?.id).toBe("ozzo");
      expect(state.players[1]?.id).toBe("veil_nyx");
    } finally {
      pressHigh.dispose();
    }
  });

  it("rejects an unknown tactic id and a malformed starting XI, without disturbing later construction", () => {
    expect(() => createSimHost(HOME, AWAY, 7, 20, 3, { tactic: "no-such-tactic" })).toThrow();
    expect(() => createSimHost(HOME, AWAY, 7, 20, 3, { homeStarterIds: ["not-a-keeper", "veil_nyx", "rok_tann", "mika_olu", "sela_dwin"] })).toThrow();
    expect(() =>
      createSimHost(HOME, AWAY, 7, 20, 3, {
        // A duplicate id.
        homeStarterIds: ["ozzo", "veil_nyx", "veil_nyx", "mika_olu", "sela_dwin"],
      }),
    ).toThrow();

    // A well-formed construction afterward still succeeds.
    const host = createSimHost(HOME, AWAY, 7, 20, 3, { tactic: "press_high" });
    host.dispose();
  });

  it("rejects an out-of-range localSlot", () => {
    expect(() => createSimHost(HOME, AWAY, 7, 20, 3, { localSlot: 0 })).toThrow();
    expect(() => createSimHost(HOME, AWAY, 7, 20, 3, { localSlot: 9 })).toThrow();
  });

  // `SimSession.combatEventsJson()` (new this wave, `crates/gc-wasm/src/
  // session.rs`) is the BASE (non-rollback) counterpart of what
  // `MatchDriverBridge`/`OnlineCombatPhasesBridge` already exposed --
  // "[]" with no combat companion or before the first `step`, replaced (not
  // accumulated) by every step after. This is a threading/shape smoke test,
  // the same narrower claim `sim_host.spec.ts`'s own `combatEnabled` test
  // above settles for: neither `SimSession` nor `SimHostPort` has any
  // getter proving a specific combat event actually fired on a given tick,
  // so this only proves the surface reaches this port and returns valid
  // JSON, not a specific event's content.
  it("exposes combatEventsJson(), replaced not accumulated, empty with no combat companion", () => {
    const withoutCombat = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      expect(withoutCombat.combatEventsJson?.()).toBe("[]");
      withoutCombat.step(neutralSample());
      // No combat companion at all -- stays "[]" regardless of stepping.
      expect(withoutCombat.combatEventsJson?.()).toBe("[]");
    } finally {
      withoutCombat.dispose();
    }

    const withCombat = createSimHost(HOME, AWAY, 7, 20, 3, { combatEnabled: true });
    try {
      // Before the first step, still "[]" -- see this method's doc.
      expect(withCombat.combatEventsJson?.()).toBe("[]");
      withCombat.step(neutralSample());
      const raw = withCombat.combatEventsJson?.();
      expect(raw).toBeDefined();
      expect(() => JSON.parse(raw as string)).not.toThrow();
      expect(Array.isArray(JSON.parse(raw as string))).toBe(true);
    } finally {
      withCombat.dispose();
    }
  });

  it("dispose() is safe to call twice, and use-after-dispose fails loudly", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    host.step(neutralSample());
    host.dispose();
    expect(() => host.dispose()).not.toThrow();

    expect(() => host.step(neutralSample())).toThrow(/use after dispose/);
    expect(() => host.frame()).toThrow(/use after dispose/);
    expect(() => host.roster()).toThrow(/use after dispose/);
    expect(() => host.tick()).toThrow(/use after dispose/);
  });

  it("survives wasm heap growth between two frame() reads (memory-view invalidation)", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      host.step(neutralSample());
      const before = host.frame();
      expect(Number.isFinite(before.ball.x)).toBe(true);
      expect(before.players.count).toBeGreaterThan(0);

      // Grow the SAME wasm instance's linear memory directly. This replaces
      // `WebAssembly.Memory`'s backing `ArrayBuffer` wholesale -- any
      // `Float64Array` view still pointing at the OLD buffer would now read
      // a detached/stale buffer. `loadSimHost()` is a per-process singleton
      // (see `@gc/wasm`'s `index.ts`), so this is the exact same instance
      // `createSimHost` above is driving.
      const rawHost = loadSimHost();
      const bufferBeforeGrowth = rawHost.memory.buffer;
      rawHost.memory.grow(4);
      expect(rawHost.memory.buffer).not.toBe(bufferBeforeGrowth);

      host.step(neutralSample());
      const after = host.frame();
      expect(Number.isFinite(after.ball.x)).toBe(true);
      expect(Number.isFinite(after.ball.y)).toBe(true);
      expect(after.players.count).toBe(before.players.count);
      expect(after.players.x.length).toBe(after.players.count);
      for (const x of after.players.x) {
        expect(Number.isFinite(x)).toBe(true);
      }
      // roster() decoded before growth must still read back correctly too --
      // it is cached, but re-derive it fresh here to prove the roster path
      // (which crosses via ordinary wasm-bindgen `Vec<f64>` copies, not a
      // raw memory view) is unaffected either way.
      expect(host.roster().ids.length).toBeGreaterThan(0);
    } finally {
      host.dispose();
    }
  });

  it("planTicks reports the fixed_clock tick count for a render dt, and step advances exactly that many times", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      // A sub-tick dt plans zero ticks -- gc_sim::fixed_clock::TICK_SECONDS
      // is 1/60; this render update banks less than one tick's worth.
      expect(host.planTicks(1 / 120)).toBe(0);
      expect(host.tick()).toBe(0);

      // The banked 1/120s plus another 1/120s crosses one tick.
      const ticks = host.planTicks(1 / 120);
      expect(ticks).toBe(1);
      for (let i = 0; i < ticks; i += 1) {
        host.step(neutralSample());
      }
      expect(host.tick()).toBe(ticks);
    } finally {
      host.dispose();
    }
  });

  it("cancelPlannedTicks resets the clock's carried-over accumulator", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      // Bank a fraction of a tick, then cancel -- mirrors MatchScreen
      // stopping a catch-up batch early because the match finished
      // mid-batch.
      expect(host.planTicks(1 / 120)).toBe(0);
      host.cancelPlannedTicks();
      // If cancelPlannedTicks had not zeroed the accumulator, the banked
      // 1/120s would combine with this call and plan a tick.
      expect(host.planTicks(1 / 120)).toBe(0);
    } finally {
      host.dispose();
    }
  });

  it("planTicks rejects a non-finite or negative dt (SimSession's wasm-bindgen error path, exercised here rather than natively -- see crates/gc-wasm/src/session.rs)", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      expect(() => host.planTicks(Number.NaN)).toThrow();
      expect(() => host.planTicks(-0.001)).toThrow();
    } finally {
      host.dispose();
    }
  });

  it("growing the heap directly between two raw buildRenderFrame calls still decodes (lower-level proof)", () => {
    const rawHost = loadSimHost();
    const session = new rawHost.Session(HOME, AWAY, 7, 20, 3);
    try {
      const wire = ["2", "0", ...Array.from({ length: 8 }, () => "0,0,0,0")].join("|");
      session.step(wire);

      const first = rawHost.buildRenderFrame(session.handle, 0);
      expect(first).not.toBeNull();
      const firstBuffer = first?.buffer;

      rawHost.memory.grow(4);

      const second = rawHost.buildRenderFrame(session.handle, 0);
      expect(second).not.toBeNull();
      // A view derived after growth must not still point at the pre-growth
      // buffer -- if it did, that would mean stale-view caching crept back
      // in somewhere on this path.
      expect(second?.buffer).not.toBe(firstBuffer);
      expect(second?.[0]).toBe(0x474f_4c46); // MAGIC
    } finally {
      session.free();
    }
  });
});

// The whole reason `frame()` reads `releaseFollow` at all. Before this
// wiring, BOTH production `RenderFrameOptions` construction sites built
// `{ roster, ..Default::default() }`, so `kick_follow` was always `None` and
// `PlayerPoseId::KickFollow` could never be selected in a real match no
// matter what the renderer's follow-through window said. These cases drive
// the full chain: renderer window -> roster-slot mask -> raw wasm export ->
// `gc_render::player_pose::select` -> decoded frame.
describe("createSimHost carries the renderer's kick_follow window into the built frame", () => {
  afterEach(() => {
    // Module-level state shared with every other consumer in this process.
    releaseFollow.reset();
  });

  function outfieldSlot(ids: readonly string[], isKeeper: readonly boolean[]): number {
    const slot = isKeeper.findIndex((keeper) => !keeper);
    expect(slot).toBeGreaterThanOrEqual(0);
    expect(ids[slot]).toBeDefined();
    return slot;
  }

  it("selects the kick_follow pose for a player with an open window, and only that player", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      host.step(neutralSample());
      const roster = host.roster();
      const slot = outfieldSlot(roster.ids, roster.is_keeper);

      expect(host.frame().players.pose_id[slot]).not.toBe("kick_follow");

      releaseFollow.update([{ kind: "shot", player: roster.ids[slot]! }], 0);
      const following = host.frame();
      expect(following.players.pose_id[slot]).toBe("kick_follow");
      for (let other = 0; other < following.players.count; other += 1) {
        if (other !== slot) {
          expect(following.players.pose_id[other]).not.toBe("kick_follow");
        }
      }
    } finally {
      host.dispose();
    }
  });

  it("drops the pose again once the window ages out, without the tick having moved", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      host.step(neutralSample());
      const roster = host.roster();
      const slot = outfieldSlot(roster.ids, roster.is_keeper);
      const tick = host.tick();

      releaseFollow.update([{ kind: "shot", player: roster.ids[slot]! }], 0);
      expect(host.frame().players.pose_id[slot]).toBe("kick_follow");

      releaseFollow.update([], releaseFollow.DURATION);
      // The per-tick frame cache must not serve the stale pose: the window
      // is part of its key precisely because it changes the frame while the
      // simulation stands still.
      expect(host.tick()).toBe(tick);
      expect(host.frame().players.pose_id[slot]).not.toBe("kick_follow");
    } finally {
      host.dispose();
    }
  });

  it("ignores a window whose id is not on this match's roster", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      host.step(neutralSample());
      releaseFollow.update([{ kind: "shot", player: "not_on_this_roster" }], 0);
      const frame = host.frame();
      for (let slot = 0; slot < frame.players.count; slot += 1) {
        expect(frame.players.pose_id[slot]).not.toBe("kick_follow");
      }
    } finally {
      host.dispose();
    }
  });
});

// REGRESSION: `frameBuffer.toRenderFrame` used to drop `events` on the floor.
// `decode` recovered them off the wire correctly, but the object handed to
// every consumer had no `events` field at all -- so
// `MatchScreen.appendObservedFrameEvents` was an unconditional early return
// in production and BOTH consumers of the per-tick batch were inert: the
// `game.match_observer` attribution feed and the release follow-through
// window. A fake host can never catch this; only a real decode can.
describe("createSimHost surfaces the wire's per-tick match events", () => {
  it("carries a decoded, well-shaped events block on every frame", () => {
    const host = createSimHost(HOME, AWAY, 7, 20, 3);
    try {
      host.step(neutralSample());
      const events = host.frame().events;
      expect(events).toBeDefined();
      expect(typeof events.count).toBe("number");
      expect(events.kind.length).toBe(events.count);
      expect(events.slot.length).toBe(events.count);
    } finally {
      host.dispose();
    }
  });

  it("actually reports events from a live match, attributed to a roster slot", () => {
    const host = createSimHost(HOME, AWAY, 7, 60, 3);
    try {
      const kinds = new Set<string>();
      let attributed = 0;
      // Long enough for the inline AI to produce touches/tackles/passes.
      for (let tick = 0; tick < 1800; tick += 1) {
        host.step(neutralSample());
        const events = host.frame().events;
        for (let index = 0; index < events.count; index += 1) {
          const kind = events.kind[index];
          if (kind !== undefined) {
            kinds.add(kind);
          }
          const slot = events.slot[index];
          if (slot !== undefined) {
            attributed += 1;
            expect(host.roster().ids[slot - 1]).toBeDefined();
          }
        }
      }
      expect(kinds.size, "a minute of real match produced no events at all").toBeGreaterThan(0);
      expect(attributed, "no event was attributed to a roster slot").toBeGreaterThan(0);
    } finally {
      host.dispose();
    }
  });
});
