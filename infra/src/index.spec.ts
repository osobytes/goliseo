import { describe, expect, it, vi } from "vitest";

import { handleHostSignal, handleJoinSignal } from "./index.ts";

/**
 * A fake `Env` exposing only what these two handlers touch. Cast through
 * `unknown` rather than satisfying the full generated `Env` shape
 * (`RateLimit`/`DurableObjectNamespace` carry many members these handlers
 * never call) -- standard for testing Worker code against fakes.
 *
 * `isClaimedByHost` is the fake's stand-in for `RoomDurableObject`'s own RPC
 * method of the same name (`room_durable_object.ts`'s doc, "Collision probe
 * (RPC, not `fetch`)") -- `handleHostSignal` calls it directly on the stub,
 * never through `fetch`, before every real attempt. `doFetch` answers a real
 * `Upgrade: websocket` request with a stand-in successful upgrade. A real
 * 101 Switching Protocols response can only be constructed inside a Workers
 * runtime (with its `webSocket` property); Node's plain `Response`
 * constructor rejects the status code outright, so 200 stands in for it
 * here -- these tests assert routing/ordering, never the WS-upgrade shape
 * itself, which `wrangler dev` evidence covers instead.
 */
function fakeEnv(rateLimitSuccess: boolean, options: { readonly hostClaimed?: boolean } = {}) {
  const limit = vi.fn().mockResolvedValue({ success: rateLimitSuccess });
  const hostClaimed = options.hostClaimed ?? false;
  // Typed explicitly as `(request: Request) => ...` (rather than inferred
  // from an unused parameter, which the lint config rejects) so
  // `doFetch.mock.calls[N]?.[0]` below still resolves to `Request`.
  const doFetch = vi.fn<(request: Request) => Promise<Response>>(() =>
    Promise.resolve(new Response(null, { status: 200 })),
  );
  const isClaimedByHost = vi.fn(() => Promise.resolve(hostClaimed));
  const getByName = vi.fn().mockReturnValue({ fetch: doFetch, isClaimedByHost });
  const env = {
    SIGNAL_RATE_LIMITER: { limit },
    ROOM: { getByName },
  } as unknown as Env;
  return { env, limit, getByName, doFetch, isClaimedByHost };
}

// Round-2 council review, blocking finding 1: a non-upgrade request used to
// reach RoomDurableObject.fetch's own probe branch, answering "is this code
// claimed" to any caller for the cost of one rate-limit token -- an
// unauthenticated, internet-reachable room-existence oracle. Both handlers
// must now refuse a non-upgrade request outright, before EITHER the rate
// limiter or env.ROOM.getByName runs.
describe("the Upgrade-header gate (round-2 council review, blocking finding 1)", () => {
  it("handleHostSignal: rejects a non-upgrade request with 400 before touching the rate limiter or the DO", async () => {
    const { env, limit, getByName } = fakeEnv(true);
    const request = new Request("https://example.com/signal/host");

    const response = await handleHostSignal(request, env);

    expect(response.status).toBe(400);
    expect(limit).not.toHaveBeenCalled();
    expect(getByName).not.toHaveBeenCalled();
  });

  it("handleJoinSignal: rejects a non-upgrade GET with a valid code shape with 400, and the DO is never addressed", async () => {
    const { env, limit, getByName } = fakeEnv(true);
    // A VALID code shape on purpose -- this is exactly the request that used
    // to reach the room-existence oracle (a malformed code would already be
    // rejected downstream anyway; a valid one is the case that mattered).
    const request = new Request("https://example.com/signal/join?code=AB3D9H");

    const response = await handleJoinSignal(request, env);

    expect(response.status).toBe(400);
    expect(limit).not.toHaveBeenCalled();
    expect(getByName).not.toHaveBeenCalled();
  });

  it("handleJoinSignal: an Upgrade header present but not 'websocket' is still rejected", async () => {
    const { env, getByName } = fakeEnv(true);
    const request = new Request("https://example.com/signal/join?code=AB3D9H", {
      headers: { Upgrade: "h2c" },
    });

    const response = await handleJoinSignal(request, env);

    expect(response.status).toBe(400);
    expect(getByName).not.toHaveBeenCalled();
  });
});

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
    const { env, limit, getByName, doFetch, isClaimedByHost } = fakeEnv(true);
    const request = new Request("https://example.com/signal/host", {
      headers: { Upgrade: "websocket", "CF-Connecting-IP": "203.0.113.9" },
    });

    const response = await handleHostSignal(request, env);

    expect(limit).toHaveBeenCalledTimes(1);
    expect(getByName).toHaveBeenCalledTimes(1);
    // The collision probe (an RPC call, not an HTTP request) admits the
    // code, then the real upgrade request claims it -- exactly one of each.
    expect(isClaimedByHost).toHaveBeenCalledTimes(1);
    expect(doFetch).toHaveBeenCalledTimes(1);
    expect(doFetch.mock.calls[0]?.[0]?.headers.get("Upgrade")).toBe("websocket");
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

  // room_durable_object.ts's admission failures now complete the WebSocket
  // upgrade instead of returning a status this loop can read at all (its
  // own doc, "Admission failures") -- so the collision probe (the
  // isClaimedByHost RPC method) is the only remaining signal this loop has
  // for "try a different code", and it must never fall through to a real
  // upgrade attempt for a code the probe reports as already claimed.
  it("retries with a new code when the collision probe reports the code is already claimed", async () => {
    const { env, getByName, doFetch, isClaimedByHost } = fakeEnv(true, { hostClaimed: true });

    const response = await handleHostSignal(
      new Request("https://example.com/signal/host", { headers: { Upgrade: "websocket" } }),
      env,
    );

    // MAX_CODE_ATTEMPTS worth of probes, never a real upgrade attempt: every
    // probe in this test reports the code as claimed.
    expect(getByName).toHaveBeenCalledTimes(5);
    expect(isClaimedByHost).toHaveBeenCalledTimes(5);
    expect(doFetch).not.toHaveBeenCalled();
    expect(response.status).toBe(503);
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
