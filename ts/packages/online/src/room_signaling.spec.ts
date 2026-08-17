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

  it("rejects a signal frame whose body is not a string -- see this file's header", () => {
    // The Durable Object's own wire contract allows a host to send a
    // non-string `body`, but this module (and every caller past it) only
    // ever handles a string signal -- a non-string body is a protocol
    // violation here, never coerced.
    const wire = '{"type":"signal","from":"host","body":{"nested":true}}';
    expect(parseServerFrame(wire)).toEqual({ ok: false, error: "malformed_frame" });
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
});

describe("room_signaling: classifyClose", () => {
  it("classifies a close before readiness as a handshake failure", () => {
    expect(classifyClose(false)).toEqual({ reason: "handshake_failed" });
  });

  it("classifies a close after readiness as a lost connection", () => {
    expect(classifyClose(true)).toEqual({ reason: "connection_lost" });
  });
});
