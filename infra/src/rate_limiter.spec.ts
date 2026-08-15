import { describe, expect, it } from "vitest";

import { tryConsume } from "./rate_limiter.ts";

const POLICY = { limit: 3, windowMs: 1000 };

describe("tryConsume", () => {
  it("starts a fresh window and admits when state is null", () => {
    const result = tryConsume(null, 0, POLICY);
    expect(result).toEqual({ ok: true, value: { windowStartMs: 0, count: 1 } });
  });

  it("admits repeatedly up to the limit within the window", () => {
    let state = tryConsume(null, 0, POLICY);
    expect(state.ok).toBe(true);
    for (let i = 1; i < POLICY.limit; i += 1) {
      if (!state.ok) throw new Error("unexpected rejection");
      state = tryConsume(state.value, 100 * i, POLICY);
      expect(state.ok).toBe(true);
    }
    if (!state.ok) throw new Error("unexpected rejection");
    expect(state.value.count).toBe(POLICY.limit);
  });

  it("rejects once the limit is spent within the window", () => {
    let state = tryConsume(null, 0, POLICY);
    for (let i = 1; i < POLICY.limit; i += 1) {
      if (!state.ok) throw new Error("unexpected rejection");
      state = tryConsume(state.value, i, POLICY);
    }
    if (!state.ok) throw new Error("unexpected rejection");
    const overflow = tryConsume(state.value, POLICY.limit, POLICY);
    expect(overflow).toEqual({ ok: false, error: "rate_limited" });
  });

  it("starts a new window once windowMs has elapsed, resetting the count", () => {
    let state = tryConsume(null, 0, POLICY);
    for (let i = 1; i < POLICY.limit; i += 1) {
      if (!state.ok) throw new Error("unexpected rejection");
      state = tryConsume(state.value, i, POLICY);
    }
    if (!state.ok) throw new Error("unexpected rejection");
    expect(state.value.count).toBe(POLICY.limit);

    const afterWindow = tryConsume(state.value, POLICY.windowMs + 1, POLICY);
    expect(afterWindow).toEqual({
      ok: true,
      value: { windowStartMs: POLICY.windowMs + 1, count: 1 },
    });
  });

  it("treats an elapsed window as elapsed at the exact boundary", () => {
    const state = tryConsume(null, 0, POLICY);
    if (!state.ok) throw new Error("unexpected rejection");
    const atBoundary = tryConsume(state.value, POLICY.windowMs, POLICY);
    expect(atBoundary).toEqual({ ok: true, value: { windowStartMs: POLICY.windowMs, count: 1 } });
  });
});
