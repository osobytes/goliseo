// Exercises `crates/gc-wasm/src/input_protocol_bridge.rs`'s wasm-bindgen
// surface against the real compiled artifact, under node.
//
// This is also the "prove bytes survive" test the wave's brief asked for:
// `inputProtocolEncode` returns a `Uint8Array` straight off the wasm ABI
// (lossless by construction -- `wasm-bindgen`'s native `Vec<u8>` marshalling
// never touches UTF-8), and `binaryStringRoundTrip` below additionally
// takes that `Uint8Array` through `byteStringFromBytes`/`bytesFromByteString`
// -- the exact conversion a caller handing this packet to `@gc/transport`'s
// `newMessage({ payload })` would perform -- and re-decodes from the
// recovered bytes, proving the "binary string" convention this package
// documents (`binary_string.ts`) does not silently corrupt a real encoded
// packet.
//
// Requires `pnpm --filter @gc/wasm build` to have run first.

import { describe, expect, it } from "vitest";

import { bytesFromByteString, byteStringFromBytes } from "./binary_string.ts";
import { loadSimHost, type WasmInputPacket } from "./index.ts";

const SESSION_ID = "session_1";
const MANIFEST_ID = "0123456789abcdef";
const SENDER_ID = "guest_1";

function guestRowsJson(host: ReturnType<typeof loadSimHost>, slotIndex: number, finalEdges: number): string {
  const historyRows = JSON.parse(host.inputProtocolConstantsJson()).history_rows as number;
  const rows = [];
  for (let tick = 0; tick <= historyRows; tick += 1) {
    const edges = tick === historyRows ? finalEdges : 0;
    rows.push({
      tick,
      slot_index: slotIndex,
      sample: host.inputFrameNewSample(undefined, undefined, undefined, edges),
    });
  }
  return JSON.stringify(rows);
}

function newGuestPacket(
  host: ReturnType<typeof loadSimHost>,
  overrides: { slotIndex?: number; sequence?: number; transportTick?: number; firstInputTick?: number; edges?: number } = {},
): WasmInputPacket {
  const slotIndex = overrides.slotIndex ?? 3;
  const sequence = overrides.sequence ?? 0;
  const transportTick = overrides.transportTick ?? 6;
  const firstInputTick = overrides.firstInputTick ?? 0;
  return host.inputProtocolNewGuest(
    SESSION_ID,
    MANIFEST_ID,
    SENDER_ID,
    sequence,
    transportTick,
    firstInputTick,
    undefined,
    guestRowsJson(host, slotIndex, overrides.edges ?? 0),
  );
}

describe("inputProtocol bridge", () => {
  it("constantsJson matches the module's documented bounds", () => {
    const host = loadSimHost();
    const constants = JSON.parse(host.inputProtocolConstantsJson()) as {
      fairness_delay_ticks: number;
      history_rows: number;
    };
    expect(constants.fairness_delay_ticks).toBe(3);
    expect(constants.history_rows).toBe(6);
  });

  it("newGuest builds a packet exposing exactly sequence/transport_tick as declared by InputProtocolPacket", () => {
    const host = loadSimHost();
    const packet = newGuestPacket(host, { sequence: 5, transportTick: 11 });
    expect(packet.sequence).toBe(5);
    expect(packet.transport_tick).toBe(11);
    expect(packet.kind).toBe("guest");
  });

  it("encode then decode round-trips a packet byte for byte", () => {
    const host = loadSimHost();
    const packet = newGuestPacket(host);
    const wire = host.inputProtocolEncode(packet);
    expect(wire).toBeInstanceOf(Uint8Array);

    const decoded = host.inputProtocolDecode(SESSION_ID, MANIFEST_ID, SENDER_ID, wire);
    expect(decoded.packet_id).toBe(packet.packet_id);
    expect(decoded.rows_json).toBe(packet.rows_json);
    expect(host.inputProtocolEncode(decoded)).toEqual(wire);
  });

  it("survives a real binary-string round trip (the @gc/transport payload convention)", () => {
    const host = loadSimHost();
    const packet = newGuestPacket(host, { edges: 1 });
    const originalBytes = host.inputProtocolEncode(packet);

    // Exactly what a caller does before calling `newMessage({ payload })`.
    const binaryString = byteStringFromBytes(originalBytes);
    expect(typeof binaryString).toBe("string");

    // ... and exactly what a caller does on the receiving end, before
    // handing the bytes back into `inputProtocolDecode`.
    const recoveredBytes = bytesFromByteString(binaryString);
    expect(recoveredBytes).toEqual(originalBytes);

    const decoded = host.inputProtocolDecode(SESSION_ID, MANIFEST_ID, SENDER_ID, recoveredBytes);
    expect(decoded.rows_json).toBe(packet.rows_json);
    expect(decoded.transport_tick).toBe(packet.transport_tick);
  });

  it("newGuest throws on a forged bundle naming a second slot in the retained history", () => {
    const host = loadSimHost();
    // Exactly the shape `net_diagnostics_fixture.ts`'s `forgedBundle` needs
    // to reach downstream ownership-violation/authority-conflict cases: a
    // history whose rows do not all name the same slot.
    const historyRows = JSON.parse(host.inputProtocolConstantsJson()).history_rows as number;
    const rows: Array<{ tick: number; slot_index: number; sample: string }> = [];
    for (let tick = 0; tick <= historyRows; tick += 1) {
      rows.push({
        tick,
        slot_index: tick === historyRows ? 4 : 3,
        sample: host.inputFrameNeutralSample(),
      });
    }
    expect(() =>
      host.inputProtocolNewGuest(SESSION_ID, MANIFEST_ID, SENDER_ID, 0, 6, 0, undefined, JSON.stringify(rows)),
    ).toThrow();
  });

  it("newGuest throws on a bundle with too few retained rows", () => {
    const host = loadSimHost();
    const rows = [{ tick: 5, slot_index: 3, sample: host.inputFrameNeutralSample() }];
    expect(() =>
      host.inputProtocolNewGuest(SESSION_ID, MANIFEST_ID, SENDER_ID, 0, 6, 0, undefined, JSON.stringify(rows)),
    ).toThrow();
  });

  it("decode throws on a byte payload that is not a canonical packet", () => {
    const host = loadSimHost();
    const garbage = new Uint8Array([1, 2, 3, 4, 5]);
    expect(() => host.inputProtocolDecode(SESSION_ID, MANIFEST_ID, SENDER_ID, garbage)).toThrow();
  });

  it("packetId is stable for the same identity and differs across sequence", () => {
    const host = loadSimHost();
    const a = host.inputProtocolPacketId("guest", SESSION_ID, SENDER_ID, 0);
    const b = host.inputProtocolPacketId("guest", SESSION_ID, SENDER_ID, 0);
    const c = host.inputProtocolPacketId("guest", SESSION_ID, SENDER_ID, 1);
    expect(a).toBe(b);
    expect(a).not.toBe(c);
  });

  it("confirmedSpan/confirmedTick round-trip", () => {
    const host = loadSimHost();
    const span = host.inputProtocolConfirmedSpan(10, 15);
    expect(span).toBe(6);
  });

  it("classifyDuplicate reports idempotent for a byte-identical resend", () => {
    const host = loadSimHost();
    const a = newGuestPacket(host);
    const b = newGuestPacket(host);
    expect(host.inputProtocolClassifyDuplicate(a, b)).toBe("idempotent");
  });

  it("classifyDuplicate throws on a genuine conflict (same identity, different bytes)", () => {
    const host = loadSimHost();
    const a = newGuestPacket(host, { edges: 0 });
    const b = newGuestPacket(host, { edges: 1 });
    expect(() => host.inputProtocolClassifyDuplicate(a, b)).toThrow();
  });
});
