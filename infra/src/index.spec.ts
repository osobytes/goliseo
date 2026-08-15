import { describe, expect, it, vi } from "vitest";

import { handleHostSignal, handleJoinSignal } from "./index.ts";

/**
 * A fake `Env` exposing only what these two handlers touch. Cast through
 * `unknown` rather than satisfying the full generated `Env` shape
 * (`RateLimit`/`DurableObjectNamespace` carry many members these handlers
 * never call) -- standard for testing Worker code against fakes.
 */
function fakeEnv(rateLimitSuccess: boolean) {
  const limit = vi.fn().mockResolvedValue({ success: rateLimitSuccess });
  // A real 101 Switching Protocols response can only be constructed inside
  // a Workers runtime (with its `webSocket` property); Node's plain
  // Response constructor rejects the status code outright. 200 stands in
  // -- these tests assert routing/ordering, never the WS-upgrade shape
  // itself, which `wrangler dev` evidence covers instead.
  const doFetch = vi.fn().mockResolvedValue(new Response(null, { status: 200 }));
  const getByName = vi.fn().mockReturnValue({ fetch: doFetch });
  const env = {
    SIGNAL_RATE_LIMITER: { limit },
    ROOM: { getByName },
  } as unknown as Env;
  return { env, limit, getByName, doFetch };
}

describe("handleHostSignal", () => {
  it("returns 429 and never touches env.ROOM when the signal rate limit is exhausted", async () => {
    const { env, limit, getByName } = fakeEnv(false);
    const request = new Request("https://example.com/signal/host", {
      headers: { Upgrade: "websocket", "CF-Connecting-IP": "203.0.113.9" },
    });

    const response = await handleHostSignal(request, env);

    expect(response.status).toBe(429);
    expect(limit).toHaveBeenCalledWith({ key: "203.0.113.9" });
    // The blocking finding: a Durable Object must never be provisioned or
    // addressed before this check runs.
    expect(getByName).not.toHaveBeenCalled();
  });

  it("addresses a room only after the rate limit admits the request", async () => {
    const { env, limit, getByName, doFetch } = fakeEnv(true);
    const request = new Request("https://example.com/signal/host", {
      headers: { Upgrade: "websocket", "CF-Connecting-IP": "203.0.113.9" },
    });

    const response = await handleHostSignal(request, env);

    expect(limit).toHaveBeenCalledTimes(1);
    expect(getByName).toHaveBeenCalledTimes(1);
    expect(doFetch).toHaveBeenCalledTimes(1);
    expect(response.status).toBe(200);
  });

  it("falls back to a fixed rate-limit key when CF-Connecting-IP is absent", async () => {
    const { env, limit } = fakeEnv(false);
    const request = new Request("https://example.com/signal/host", {
      headers: { Upgrade: "websocket" },
    });

    await handleHostSignal(request, env);

    expect(limit).toHaveBeenCalledWith({ key: "unknown" });
  });
});

describe("handleJoinSignal", () => {
  it("returns 429 and never touches env.ROOM when the signal rate limit is exhausted, even for a malformed code", async () => {
    const { env, limit, getByName } = fakeEnv(false);
    const request = new Request("https://example.com/signal/join?code=not-a-real-code", {
      headers: { Upgrade: "websocket", "CF-Connecting-IP": "203.0.113.9" },
    });

    const response = await handleJoinSignal(request, env);

    expect(response.status).toBe(429);
    expect(limit).toHaveBeenCalledWith({ key: "203.0.113.9" });
    expect(getByName).not.toHaveBeenCalled();
  });

  it("still rejects an invalid code once the rate limit admits the request", async () => {
    const { env, getByName } = fakeEnv(true);
    const request = new Request("https://example.com/signal/join?code=nope", {
      headers: { Upgrade: "websocket" },
    });

    const response = await handleJoinSignal(request, env);

    expect(response.status).toBe(400);
    expect(getByName).not.toHaveBeenCalled();
  });

  it("addresses a room by its (uppercased) code once admitted", async () => {
    const { env, getByName, doFetch } = fakeEnv(true);
    const request = new Request("https://example.com/signal/join?code=ab3d9h", {
      headers: { Upgrade: "websocket" },
    });

    const response = await handleJoinSignal(request, env);

    expect(getByName).toHaveBeenCalledWith("AB3D9H");
    expect(doFetch).toHaveBeenCalledTimes(1);
    expect(response.status).toBe(200);
  });
});
