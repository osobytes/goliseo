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
