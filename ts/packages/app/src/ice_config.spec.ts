import { describe, expect, it } from "vitest";
import {
  DEFAULT_STUN_SERVERS,
  forceRelayFromSearch,
  mergeIceServers,
  newIceConfig,
} from "./ice_config.ts";

const TURN_SERVER: RTCIceServer = {
  urls: "turn:turn.example.com:3478",
  username: "short-lived-user",
  credential: "short-lived-credential",
};

describe("mergeIceServers", () => {
  it("returns only the defaults when nothing was fetched", () => {
    expect(mergeIceServers(DEFAULT_STUN_SERVERS, undefined)).toBe(DEFAULT_STUN_SERVERS);
  });

  it("returns only the defaults when the endpoint answered with an empty list", () => {
    expect(mergeIceServers(DEFAULT_STUN_SERVERS, [])).toBe(DEFAULT_STUN_SERVERS);
  });

  it("appends fetched TURN servers after the STUN defaults", () => {
    const merged = mergeIceServers(DEFAULT_STUN_SERVERS, [TURN_SERVER]);
    expect(merged).toEqual([...DEFAULT_STUN_SERVERS, TURN_SERVER]);
  });
});

describe("newIceConfig", () => {
  it("degrades to STUN-only when the TURN fetch produced nothing (endpoint absent/errored)", () => {
    const config = newIceConfig(undefined, false);
    expect(config.iceServers).toEqual(DEFAULT_STUN_SERVERS);
    expect(config.iceTransportPolicy).toBeUndefined();
  });

  it("merges a served TURN response with the STUN defaults", () => {
    const config = newIceConfig([TURN_SERVER], false);
    expect(config.iceServers).toEqual([...DEFAULT_STUN_SERVERS, TURN_SERVER]);
  });

  it("leaves iceTransportPolicy unset when the relay toggle is off (direct path unaffected)", () => {
    const config = newIceConfig([TURN_SERVER], false);
    expect(config.iceTransportPolicy).toBeUndefined();
  });

  it("sets iceTransportPolicy to relay only when the toggle is explicitly on", () => {
    const config = newIceConfig([TURN_SERVER], true);
    expect(config.iceTransportPolicy).toBe("relay");
  });

  it("can force relay even with no TURN servers fetched (degrades to STUN-only relay, still off by default elsewhere)", () => {
    const config = newIceConfig(undefined, true);
    expect(config.iceServers).toEqual(DEFAULT_STUN_SERVERS);
    expect(config.iceTransportPolicy).toBe("relay");
  });
});

describe("forceRelayFromSearch", () => {
  it("is off for a plain URL with no query string", () => {
    expect(forceRelayFromSearch("")).toBe(false);
  });

  it("is off for an unrelated query parameter", () => {
    expect(forceRelayFromSearch("?debug=1")).toBe(false);
  });

  it("is off for a near-miss value", () => {
    expect(forceRelayFromSearch("?ice=relayed")).toBe(false);
  });

  it("turns on for ?ice=relay", () => {
    expect(forceRelayFromSearch("?ice=relay")).toBe(true);
  });

  it("turns on for ?ice=relay alongside other parameters", () => {
    expect(forceRelayFromSearch("?debug=1&ice=relay")).toBe(true);
  });
});
