// Unlike ice_config.spec.ts (pure), this exercises the one impure seam
// #550 adds: a same-origin fetch that must degrade silently on every
// failure shape a plain static host (no such Worker at all), a
// misconfigured one, or a flaky one can produce. `globalThis.fetch` is
// stubbed per case rather than hitting a network.

import { afterEach, describe, expect, it, vi } from "vitest";
import { fetchTurnCredentials } from "./turn_credentials.ts";

function stubFetch(impl: (url: RequestInfo | URL, init?: RequestInit) => Promise<Response>): void {
  vi.stubGlobal("fetch", impl);
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("fetchTurnCredentials", () => {
  it("returns the served iceServers on a valid 2xx JSON response", async () => {
    stubFetch(() =>
      Promise.resolve(
        new Response(
          JSON.stringify({
            iceServers: [{ urls: "turn:turn.example.com:3478", username: "u", credential: "c" }],
          }),
          { status: 200 },
        ),
      ),
    );
    const servers = await fetchTurnCredentials();
    expect(servers).toEqual([
      { urls: "turn:turn.example.com:3478", username: "u", credential: "c" },
    ]);
  });

  it("degrades to undefined when the endpoint is absent (404, a plain static host)", async () => {
    stubFetch(() => Promise.resolve(new Response("not found", { status: 404 })));
    await expect(fetchTurnCredentials()).resolves.toBeUndefined();
  });

  it("degrades to undefined on a server error", async () => {
    stubFetch(() => Promise.resolve(new Response("boom", { status: 500 })));
    await expect(fetchTurnCredentials()).resolves.toBeUndefined();
  });

  it("degrades to undefined when the network call itself rejects", async () => {
    stubFetch(() => Promise.reject(new Error("network unreachable")));
    await expect(fetchTurnCredentials()).resolves.toBeUndefined();
  });

  it("degrades to undefined on a non-JSON body", async () => {
    stubFetch(() => Promise.resolve(new Response("<html>not json</html>", { status: 200 })));
    await expect(fetchTurnCredentials()).resolves.toBeUndefined();
  });

  it("degrades to undefined when the JSON body has no iceServers field", async () => {
    stubFetch(() => Promise.resolve(new Response(JSON.stringify({}), { status: 200 })));
    await expect(fetchTurnCredentials()).resolves.toBeUndefined();
  });

  it("degrades to undefined when iceServers is not an array of RTCIceServer-shaped entries", async () => {
    stubFetch(() =>
      Promise.resolve(
        new Response(JSON.stringify({ iceServers: [{ notUrls: 1 }] }), { status: 200 }),
      ),
    );
    await expect(fetchTurnCredentials()).resolves.toBeUndefined();
  });

  it("requests the documented same-origin path with same-origin credentials", async () => {
    let requestedUrl: string | undefined;
    let requestedInit: RequestInit | undefined;
    stubFetch((url, init) => {
      // `fetchTurnCredentials` always calls `fetch` with a plain string
      // path (`turn_credentials.ts`'s own `TURN_CREDENTIALS_PATH`), never a
      // `Request`/`URL` object -- asserted here rather than widened to
      // handle those too, since a stringified `Request` is not meaningful.
      if (typeof url !== "string") {
        throw new Error("expected fetchTurnCredentials to call fetch with a plain string URL");
      }
      requestedUrl = url;
      requestedInit = init;
      return Promise.resolve(new Response(JSON.stringify({ iceServers: [] }), { status: 200 }));
    });
    await fetchTurnCredentials();
    expect(requestedUrl).toBe("/api/turn-credentials");
    expect(requestedInit?.credentials).toBe("same-origin");
  });
});
