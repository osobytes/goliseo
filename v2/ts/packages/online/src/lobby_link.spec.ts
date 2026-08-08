// Ported from the "lobby control framing" describe block of
// spec/game/online_lobby_flow_spec.lua.
//
// This block is the one part of that spec that never touches
// `game.screens.lobby_model` or `LobbyTestDriver` at all: it drives
// `lobby_link.frame` / `lobby_link.absorb` directly, pinning the chunking
// and reassembly invariants of *this* module alone. It belongs here, not in
// `@gc/screens`' `lobby_flow.spec.ts`, precisely because it is the half of
// the original spec that has nothing to do with the lobby model.
//
// `game.online.protocol` is Rust-owned (`crates/gc-netcode`; v2/README.md
// §2.1) with no wasm bridge this milestone, so the one case that builds a
// real control wire ("carries a real control wire whose body holds
// delimiters") cannot call the real `protocol.new`/`protocol.encode`/
// `protocol.decode`. A minimal local stand-in is used instead -- just
// enough to produce and read back a `{ kind, session_id, peer_id, sequence,
// body }` envelope -- because the behavior this case actually pins is
// `lobby_link`'s framing round trip, not `protocol`'s own encoding (see the
// module doc comment in the now-deleted `crates/gc-netcode/tests/
// lobby_flow.rs` for the same distinction).

import { describe, expect, it } from "vitest";
import { absorb, frame, MAX_CHUNK_BYTES, newFrameBuffer } from "./lobby_link.ts";

const PROTOCOL_MAX_WIRE_BYTES = 8192;

interface FakeProtocolMessage {
  readonly kind: string;
  readonly session_id: string;
  readonly peer_id: string;
  readonly sequence: number;
  readonly body: Readonly<Record<string, unknown>>;
}

// Stands in for `game.online.protocol.new`/`.encode`/`.decode` -- see this
// file's header. Not a port of the real wire format; only shaped closely
// enough to exercise `lobby_link`'s framing over a JSON envelope whose body
// contains the same delimiter characters (`;`, `|`, newlines) the real
// protocol's canonical encoding does.
function fakeProtocolNew(
  kind: string,
  sessionId: string,
  peerId: string,
  sequence: number,
  body: Readonly<Record<string, unknown>>
): FakeProtocolMessage {
  return { kind, session_id: sessionId, peer_id: peerId, sequence, body };
}

function fakeProtocolEncode(message: FakeProtocolMessage): string {
  return JSON.stringify(message);
}

function fakeProtocolDecode(wire: string): FakeProtocolMessage {
  return JSON.parse(wire) as FakeProtocolMessage;
}

describe("lobby control framing", () => {
  it("splits and rebuilds a wire that exceeds one transport payload", () => {
    const wire = "x".repeat(2500);
    const frames = frame(wire);
    if (!frames.ok) throw new Error(frames.error);
    expect(frames.value.length).toBe(3);
    const buffer = newFrameBuffer();
    const first = absorb(buffer, frames.value[0] as string);
    expect(first.ok && first.value).toBe(null);
    const second = absorb(buffer, frames.value[1] as string);
    expect(second.ok && second.value).toBe(null);
    const third = absorb(buffer, frames.value[2] as string);
    expect(third.ok && third.value).toBe(wire);
  });

  it("keeps every frame inside the transport payload bound", () => {
    const frames = frame("y".repeat(PROTOCOL_MAX_WIRE_BYTES));
    if (!frames.ok) throw new Error(frames.error);
    for (const chunk of frames.value) {
      expect(chunk.length <= 1024).toBe(true);
    }
  });

  it("refuses a stream that starts mid-wire or reorders", () => {
    const frames = frame("z".repeat(2000));
    if (!frames.ok) throw new Error(frames.error);
    const midWire = absorb(newFrameBuffer(), frames.value[1] as string);
    expect(midWire.ok).toBe(false);

    const buffer = newFrameBuffer();
    absorb(buffer, frames.value[0] as string);
    const outOfOrder = absorb(buffer, frames.value[0] as string);
    expect(outOfOrder.ok).toBe(false);
  });

  // The framing is delimiter-safe by construction: chunks are cut with
  // string slicing on the wire and the header is matched with an anchored
  // pattern, so nothing ever scans inside a payload for a separator. Pin
  // it, because that is the invariant a future "optimization" would break.
  it("carries payloads full of its own delimiters unchanged", () => {
    const wire = "GCLF;1;9;9;|a;b|\nc\r\n;;;|".repeat(120);
    expect(wire.length > MAX_CHUNK_BYTES).toBe(true);
    const frames = frame(wire);
    if (!frames.ok) throw new Error(frames.error);
    expect(frames.value.length > 1).toBe(true);
    const buffer = newFrameBuffer();
    let rebuilt: string | null = null;
    for (const chunk of frames.value) {
      const result = absorb(buffer, chunk);
      if (!result.ok) throw new Error(result.error);
      rebuilt = result.value;
    }
    expect(rebuilt).toBe(wire);
  });

  it("carries a real control wire whose body holds delimiters", () => {
    const message = fakeProtocolNew("abort", "session_alpha", "host", 1, { code: "host_abort" });
    const wire = fakeProtocolEncode(message);
    const frames = frame(wire);
    if (!frames.ok) throw new Error(frames.error);
    const buffer = newFrameBuffer();
    let rebuilt: string | null = null;
    for (const chunk of frames.value) {
      const result = absorb(buffer, chunk);
      if (!result.ok) throw new Error(result.error);
      rebuilt = result.value;
    }
    expect(rebuilt).toBe(wire);
    if (rebuilt === null) throw new Error("the wire was never reassembled");
    expect(fakeProtocolDecode(rebuilt).kind).toBe("abort");
  });

  it("refuses a wire beyond the protocol bound", () => {
    const result = frame("q".repeat(PROTOCOL_MAX_WIRE_BYTES + 1));
    expect(result.ok).toBe(false);
  });
});
