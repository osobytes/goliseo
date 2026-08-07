// Exercises `SimHostPort` end to end over the real compiled `@gc/wasm`
// artifact (requires `pnpm --filter @gc/wasm build` to have run first, same
// precondition `packages/wasm/src/session.spec.ts` documents).

import { describe, expect, it } from "vitest";
import { loadSimHost } from "@gc/wasm";
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
  // parameter this wave, and `sim_host.ts`'s `SimHostOptions.combatEnabled`
  // now threads it through (see that file's `SessionConstructorWithCombat`
  // doc for the temporary local type widening this needs, pending
  // `packages/wasm/src/types.ts`'s own `SimSessionConstructor` catching up).
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

  it("rejects an out-of-range localSlot", () => {
    expect(() => createSimHost(HOME, AWAY, 7, 20, 3, { localSlot: 0 })).toThrow();
    expect(() => createSimHost(HOME, AWAY, 7, 20, 3, { localSlot: 9 })).toThrow();
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

      const first = rawHost.buildRenderFrame(session.handle);
      expect(first).not.toBeNull();
      const firstBuffer = first?.buffer;

      rawHost.memory.grow(4);

      const second = rawHost.buildRenderFrame(session.handle);
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
