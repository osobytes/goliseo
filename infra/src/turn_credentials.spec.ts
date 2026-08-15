import { describe, expect, it } from "vitest";

import { fetchIceServers, isSameOrigin, isTurnCredentialsResponse } from "./turn_credentials.ts";

const CREDENTIALS = { keyId: "test-key-id", apiToken: "test-token" };

describe("isSameOrigin", () => {
  it("accepts a matching origin", () => {
    expect(
      isSameOrigin("https://play.example.com/api/turn-credentials", "https://play.example.com"),
    ).toBe(true);
  });

  it("accepts a matching origin that also carries a path (browsers never send one, but the check should not depend on that)", () => {
    expect(
      isSameOrigin("https://play.example.com/api/turn-credentials", "https://play.example.com/"),
    ).toBe(true);
  });

  it("rejects a different origin", () => {
    expect(
      isSameOrigin("https://play.example.com/api/turn-credentials", "https://evil.example.com"),
    ).toBe(false);
  });

  it("rejects a missing Origin header", () => {
    expect(isSameOrigin("https://play.example.com/api/turn-credentials", null)).toBe(false);
  });

  it("rejects a malformed Origin header", () => {
    expect(isSameOrigin("https://play.example.com/api/turn-credentials", "not-a-url")).toBe(false);
  });

  it("treats scheme and port as part of the origin", () => {
    expect(
      isSameOrigin("https://play.example.com/api/turn-credentials", "http://play.example.com"),
    ).toBe(false);
    expect(
      isSameOrigin(
        "https://play.example.com/api/turn-credentials",
        "https://play.example.com:8443",
      ),
    ).toBe(false);
  });
});

describe("isTurnCredentialsResponse", () => {
  it("accepts a well-shaped response", () => {
    expect(
      isTurnCredentialsResponse({
        iceServers: [
          { urls: "stun:stun.cloudflare.com:3478" },
          { urls: ["turn:turn.cloudflare.com:3478?transport=udp"], username: "u", credential: "c" },
        ],
      }),
    ).toBe(true);
  });

  it("rejects a missing iceServers field", () => {
    expect(isTurnCredentialsResponse({})).toBe(false);
  });

  it("rejects a non-array iceServers field", () => {
    expect(isTurnCredentialsResponse({ iceServers: "nope" })).toBe(false);
  });

  it("rejects an entry with a non-string/array urls field", () => {
    expect(isTurnCredentialsResponse({ iceServers: [{ urls: 5 }] })).toBe(false);
  });

  it("rejects an entry with a non-string username", () => {
    expect(isTurnCredentialsResponse({ iceServers: [{ urls: "x", username: 5 }] })).toBe(false);
  });

  it("rejects a non-object body", () => {
    expect(isTurnCredentialsResponse(null)).toBe(false);
    expect(isTurnCredentialsResponse("iceServers")).toBe(false);
  });
});

describe("fetchIceServers", () => {
  it("posts the ttl and bearer token, and returns the parsed body", async () => {
    let capturedUrl: string | undefined;
    let capturedInit: RequestInit | undefined;
    const fakeFetch: typeof fetch = (input, init) => {
      capturedUrl = new Request(input, init).url;
      capturedInit = init;
      return Promise.resolve(
        new Response(JSON.stringify({ iceServers: [{ urls: "stun:stun.cloudflare.com:3478" }] }), {
          status: 201,
        }),
      );
    };

    const result = await fetchIceServers(CREDENTIALS, 600, fakeFetch);
    expect(result).toEqual({
      ok: true,
      value: { iceServers: [{ urls: "stun:stun.cloudflare.com:3478" }] },
    });
    expect(capturedUrl).toBe(
      "https://rtc.live.cloudflare.com/v1/turn/keys/test-key-id/credentials/generate-ice-servers",
    );
    expect(capturedInit?.method).toBe("POST");
    const headers = new Headers(capturedInit?.headers);
    expect(headers.get("authorization")).toBe("Bearer test-token");
    const body = capturedInit?.body;
    if (typeof body !== "string") {
      throw new Error("expected a string request body");
    }
    expect(JSON.parse(body)).toEqual({ ttl: 600 });
  });

  it("returns an error when the API responds with a non-2xx status", async () => {
    const fakeFetch: typeof fetch = () => Promise.resolve(new Response("nope", { status: 401 }));
    const result = await fetchIceServers(CREDENTIALS, 600, fakeFetch);
    expect(result).toEqual({ ok: false, error: "turn_api_error_401" });
  });

  it("returns an error when the network call itself fails", async () => {
    const fakeFetch: typeof fetch = () => {
      throw new Error("network down");
    };
    const result = await fetchIceServers(CREDENTIALS, 600, fakeFetch);
    expect(result).toEqual({ ok: false, error: "turn_api_unreachable" });
  });

  it("returns an error when the body is not valid JSON", async () => {
    const fakeFetch: typeof fetch = () =>
      Promise.resolve(new Response("not json", { status: 200 }));
    const result = await fetchIceServers(CREDENTIALS, 600, fakeFetch);
    expect(result).toEqual({ ok: false, error: "turn_api_invalid_json" });
  });

  it("returns an error when the body has the wrong shape", async () => {
    const fakeFetch: typeof fetch = () =>
      Promise.resolve(new Response(JSON.stringify({ nope: true }), { status: 200 }));
    const result = await fetchIceServers(CREDENTIALS, 600, fakeFetch);
    expect(result).toEqual({ ok: false, error: "turn_api_unexpected_shape" });
  });
});
