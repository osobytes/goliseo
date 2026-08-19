// Tier 1 (pure logic) coverage for `room_signaling.ts` -- parsing/encoding
// against `infra/src/room_durable_object.ts`'s own documented wire shapes,
// with no WebSocket and no Worker involved. See that file's module doc and
// this module's header for the exact frames asserted below.

import { describe, expect, it } from "vitest";
import {
  ROOM_CODE_ALPHABET,
  ROOM_CODE_LENGTH,
  ROOM_SIGNAL_HOST_PATH,
  classifyClose,
  encodeHostSignal,
  isRoomCodeShaped,
  parseServerFrame,
  roomSignalJoinPath,
} from "./room_signaling.ts";

describe("room_signaling: isRoomCodeShaped", () => {
  it("accepts a code drawn from the alphabet at the fixed length", () => {
    expect(isRoomCodeShaped("A3F9K2")).toBe(true);
  });

  it("rejects the wrong length", () => {
    expect(isRoomCodeShaped("A3F9K")).toBe(false);
    expect(isRoomCodeShaped("A3F9K22")).toBe(false);
    expect(isRoomCodeShaped("")).toBe(false);
  });

  it("rejects a character outside the alphabet, including the excluded look-alikes", () => {
    // Crockford's alphabet excludes I, L, O, U -- see this module's header.
    expect(isRoomCodeShaped("AIF9K2")).toBe(false);
    expect(isRoomCodeShaped("A3F9K!")).toBe(false);
    expect(isRoomCodeShaped("a3f9k2")).toBe(false); // uppercase-only, by construction
  });

  it("mirrors infra/src/room_code.ts's own alphabet and length exactly", () => {
    expect(ROOM_CODE_ALPHABET).toBe("0123456789ABCDEFGHJKMNPQRSTVWXYZ");
    expect(ROOM_CODE_LENGTH).toBe(6);
  });
});

describe("room_signaling: paths", () => {
  it("names the fixed host-signaling route", () => {
    expect(ROOM_SIGNAL_HOST_PATH).toBe("/signal/host");
  });

  it("builds a join path carrying the code as a query parameter", () => {
    expect(roomSignalJoinPath("A3F9K2")).toBe("/signal/join?code=A3F9K2");
  });

  it("percent-encodes a code that needs it", () => {
    expect(roomSignalJoinPath("A B&C")).toBe("/signal/join?code=A%20B%26C");
  });
});

describe("room_signaling: parseServerFrame", () => {
  it("parses a host's created frame", () => {
    const result = parseServerFrame('{"type":"created","code":"A3F9K2"}');
    expect(result).toEqual({ ok: true, value: { type: "created", code: "A3F9K2" } });
  });

  it("parses a guest's joined frame", () => {
    const result = parseServerFrame('{"type":"joined","code":"A3F9K2"}');
    expect(result).toEqual({ ok: true, value: { type: "joined", code: "A3F9K2" } });
  });

  it("parses guest_joined and guest_left, host-only frames", () => {
    expect(parseServerFrame('{"type":"guest_joined","guestId":"g-1"}')).toEqual({
      ok: true,
      value: { type: "guest_joined", guestId: "g-1" },
    });
    expect(parseServerFrame('{"type":"guest_left","guestId":"g-1"}')).toEqual({
      ok: true,
      value: { type: "guest_left", guestId: "g-1" },
    });
  });

  it("parses a signal frame relayed from a guest, decoding it exactly once", () => {
    // Exactly `infra/src/room_durable_object.ts`'s own documented "DO ->
    // host" shape: `body` is a JSON STRING value holding the guest's exact
    // original text -- here a JSON-looking string, deliberately, to prove
    // this module does NOT parse it a second time (see this file's header).
    const guestBlob = '{"type":"answer","sdp":"v=0..."}';
    const wire = JSON.stringify({ type: "signal", from: "g-1", body: guestBlob });
    const result = parseServerFrame(wire);
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.value).toEqual({ type: "signal", from: "g-1", body: guestBlob });
      // One JSON.parse recovered the guest's exact original string -- not
      // an object, not the string parsed again.
      expect(typeof (result.value as { body: unknown }).body).toBe("string");
    }
  });

  it("parses a signal frame relayed from the host, whose body is not re-stringified", () => {
    const wire = '{"type":"signal","from":"host","body":"offer-blob-text"}';
    expect(parseServerFrame(wire)).toEqual({
      ok: true,
      value: { type: "signal", from: "host", body: "offer-blob-text" },
    });
  });

  it("parses an error frame", () => {
    expect(parseServerFrame('{"type":"error","error":"room_not_open"}')).toEqual({
      ok: true,
      value: { type: "error", error: "room_not_open" },
    });
  });

  it("parses each in-band admission-failure reason (#599) as an error frame", () => {
    for (const reason of [
      "room_not_found",
      "room_full",
      "room_expired",
      "room_closed",
      "host_already_claimed",
      "already_joined",
    ]) {
      expect(parseServerFrame(JSON.stringify({ type: "error", error: reason }))).toEqual({
        ok: true,
        value: { type: "error", error: reason },
      });
    }
  });

  it("parses a host_left frame, with no other fields", () => {
    expect(parseServerFrame('{"type":"host_left"}')).toEqual({
      ok: true,
      value: { type: "host_left" },
    });
  });

  it("rejects invalid JSON", () => {
    expect(parseServerFrame("not json")).toEqual({ ok: false, error: "malformed_frame" });
  });

  it("rejects a JSON value that is not an object", () => {
    expect(parseServerFrame("42")).toEqual({ ok: false, error: "malformed_frame" });
    expect(parseServerFrame("null")).toEqual({ ok: false, error: "malformed_frame" });
    expect(parseServerFrame('"a string"')).toEqual({ ok: false, error: "malformed_frame" });
  });

  it("rejects an unrecognized type", () => {
    expect(parseServerFrame('{"type":"mystery"}')).toEqual({ ok: false, error: "malformed_frame" });
  });

  it("rejects a signal frame whose body is an unrecognized object shape -- see this file's header", () => {
    // The Durable Object's own wire contract allows a host to send ANY
    // JSON value as `body`, and #601's `v: 1` slot envelope is now one
    // recognized object shape (`resolveSignalBody`'s own doc) -- but an
    // object that is neither a string nor that envelope is still a
    // protocol violation, never guessed at. This is also, precisely, what
    // an OLD (pre-#601) `parseServerFrame` does to ANY object-shaped body,
    // including a NEW host's `v: 1` envelope: its `typeof body !== "string"`
    // check does not inspect shape at all, so a stale guest bundle
    // receiving a slotted offer fails exactly this way -- a visible
    // `"malformed_frame"` failure, not a hang. See this file's header,
    // "The slot envelope (#601)", for the full compatibility argument.
    const wire = '{"type":"signal","from":"host","body":{"nested":true}}';
    expect(parseServerFrame(wire)).toEqual({ ok: false, error: "malformed_frame" });
  });

  it("rejects a slot envelope with no signal payload to recover", () => {
    // A `v: 1` envelope with a missing/mistyped `payload` has no signal to
    // deliver at all -- unlike a missing `slot` (below), there is nothing
    // sensible to degrade to, so this fails the frame.
    const noPayload = '{"type":"signal","from":"host","body":{"v":1,"slot":"guest_2"}}';
    expect(parseServerFrame(noPayload)).toEqual({ ok: false, error: "malformed_frame" });
    const wrongVersion =
      '{"type":"signal","from":"host","body":{"v":2,"slot":"guest_2","payload":"offer"}}';
    expect(parseServerFrame(wrongVersion)).toEqual({ ok: false, error: "malformed_frame" });
  });

  it("parses a host's slotted offer, recovering both the payload and the slot", () => {
    const wire = JSON.stringify({
      type: "signal",
      from: "host",
      body: { v: 1, slot: "guest_2", payload: "offer-blob-text" },
    });
    expect(parseServerFrame(wire)).toEqual({
      ok: true,
      value: { type: "signal", from: "host", body: "offer-blob-text", slot: "guest_2" },
    });
  });

  it("parses a well-formed envelope missing a slot, falling back gracefully (#601)", () => {
    // Deploy-window skew: an old host's envelope-less signal already takes
    // the plain-string path above. This is the OTHER graceful-fallback
    // shape -- a well-formed `v: 1` envelope that simply has nothing to
    // report for `slot` (e.g. the manual-flow-style identity path) -- and
    // it still resolves to a usable signal, just without one. `lobby_model.ts`'s
    // `roomPeerSignal` is what actually falls back to the guest's existing
    // default identity when `slot` is absent -- this module's job stops at
    // "parses cleanly, `slot` omitted".
    const wire = JSON.stringify({
      type: "signal",
      from: "host",
      body: { v: 1, payload: "offer-blob-text" },
    });
    expect(parseServerFrame(wire)).toEqual({
      ok: true,
      value: { type: "signal", from: "host", body: "offer-blob-text" },
    });
  });

  it("rejects a frame missing a required field", () => {
    expect(parseServerFrame('{"type":"created"}')).toEqual({ ok: false, error: "malformed_frame" });
    expect(parseServerFrame('{"type":"signal","from":"host"}')).toEqual({
      ok: false,
      error: "malformed_frame",
    });
  });
});

describe("room_signaling: encodeHostSignal", () => {
  it("encodes the { to, body } envelope infra/src/room_durable_object.ts documents", () => {
    expect(encodeHostSignal("g-1", "offer-blob-text")).toBe(
      JSON.stringify({ to: "g-1", body: "offer-blob-text" }),
    );
  });

  it("round-trips through JSON.parse to exactly the fields a host frame needs", () => {
    const wire = encodeHostSignal("g-2", "another blob");
    const parsed = JSON.parse(wire) as { readonly to: string; readonly body: string };
    expect(parsed.to).toBe("g-2");
    expect(parsed.body).toBe("another blob");
  });

  it("wraps the signal in a v:1 slot envelope when a slot is given (#601)", () => {
    const wire = encodeHostSignal("g-2", "offer-blob-text", "guest_2");
    expect(wire).toBe(
      JSON.stringify({ to: "g-2", body: { v: 1, slot: "guest_2", payload: "offer-blob-text" } }),
    );
  });

  it("round-trips a slotted signal through parseServerFrame end to end", () => {
    const wire = encodeHostSignal("g-2", "offer-blob-text", "guest_2");
    // Simulates the Durable Object's own relay: `to` is consumed for
    // routing, `body` is forwarded exactly as sent (`room_durable_object.ts`'s
    // own doc) inside a fresh `{type:"signal", from:"host", body}` frame.
    const forwarded = JSON.stringify({
      type: "signal",
      from: "host",
      body: (JSON.parse(wire) as { readonly body: unknown }).body,
    });
    expect(parseServerFrame(forwarded)).toEqual({
      ok: true,
      value: { type: "signal", from: "host", body: "offer-blob-text", slot: "guest_2" },
    });
  });
});

describe("room_signaling: classifyClose", () => {
  it("classifies a close before readiness as a handshake failure", () => {
    expect(classifyClose(false)).toEqual({ reason: "handshake_failed" });
  });

  it("classifies a close after readiness as a lost connection", () => {
    expect(classifyClose(true)).toEqual({ reason: "connection_lost" });
  });
});
