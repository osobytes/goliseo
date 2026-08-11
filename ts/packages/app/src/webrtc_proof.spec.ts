import { describe, expect, it } from "vitest";
import {
  MAX_INPUT_BYTES,
  newDiagnostics,
  newHandshake,
  newInput,
  recordReceived,
  recordRtt,
  recordSent,
  summary,
  validateHandshake,
} from "./webrtc_proof.ts";

describe("WebRTC proof contract", () => {
  it("accepts matching handshakes and rejects useful mismatches", () => {
    const handshakeResult = newHandshake("host", "build-123");
    expect(handshakeResult.ok).toBe(true);
    if (!handshakeResult.ok) {
      throw new Error("unreachable");
    }
    const handshake = handshakeResult.value;
    const accepted = validateHandshake(handshake, 1, "build-123", "host");
    expect(accepted.ok).toBe(true);

    const versionMismatch = validateHandshake(handshake, 2, "build-123", "host");
    expect(!versionMismatch.ok && versionMismatch.error.code).toBe("unsupported_version");
    const buildMismatch = validateHandshake(handshake, 1, "build-999", "host");
    expect(!buildMismatch.ok && buildMismatch.error.code).toBe("build_mismatch");
    const roleMismatch = validateHandshake(handshake, 1, "build-123", "guest");
    expect(!roleMismatch.ok && roleMismatch.error.code).toBe("role_mismatch");
  });

  it("bounds input history and payload size", () => {
    const input = newInput(4, 10, "state", [4, 5, 6]);
    expect(input.ok).toBe(true);
    if (input.ok) {
      expect(input.value.tick).toBe(10);
    }

    const oversized = newInput(5, 11, "x".repeat(MAX_INPUT_BYTES + 1), []);
    expect(oversized.ok).toBe(false);
    expect(!oversized.ok && oversized.error.code).toBe("input_too_large");

    const tooMuchHistory = newInput(6, 12, "state", [1, 2, 3, 4, 5, 6, 7]);
    expect(tooMuchHistory.ok).toBe(false);
    expect(!tooMuchHistory.ok && tooMuchHistory.error.code).toBe("malformed");
  });

  it("reports gaps, recovery history, reordering, and latency", () => {
    const diagnostics = newDiagnostics(0);
    recordSent(diagnostics, 0, 2);
    recordSent(diagnostics, 0.1, 3);

    const first = newInput(1, 1, "a", []);
    const second = newInput(3, 3, "c", [2]);
    const third = newInput(2, 2, "b", []);
    if (!first.ok || !second.ok || !third.ok) {
      throw new Error("unreachable");
    }
    recordReceived(diagnostics, 0.2, first.value);
    recordReceived(diagnostics, 0.3, second.value);
    recordReceived(diagnostics, 0.4, third.value);
    recordRtt(diagnostics, 20);
    recordRtt(diagnostics, 30);

    const result = summary(diagnostics, 1);
    expect(result.sent).toBe(2);
    expect(result.received).toBe(3);
    expect(result.uniqueTicks).toBe(3);
    expect(result.historyRetransmits).toBe(1);
    expect(result.sequenceGaps).toBe(1);
    expect(result.outOfOrder).toBe(1);
    expect(result.maxQueueDepth).toBe(3);
    expect(result.rttP50Ms).toBe(20);
    expect(result.rttP95Ms).toBe(30);
    expect(result.jitterMaxMs).toBe(10);
  });
});
