// Pure unit tests for the byte <-> "binary string" convention this package
// uses at its wasm boundary — no wasm artifact required.

import { describe, expect, it } from "vitest";

import { bytesFromByteString, byteStringFromBytes } from "./binary_string.ts";

describe("byteStringFromBytes / bytesFromByteString", () => {
  it("round-trips every byte value 0..255", () => {
    const bytes = new Uint8Array(256);
    for (let i = 0; i < 256; i += 1) {
      bytes[i] = i;
    }
    const roundTripped = bytesFromByteString(byteStringFromBytes(bytes));
    expect(roundTripped).toEqual(bytes);
  });

  it("round-trips an empty buffer", () => {
    expect(bytesFromByteString(byteStringFromBytes(new Uint8Array(0)))).toEqual(new Uint8Array(0));
  });

  it("round-trips a buffer larger than the internal chunk size", () => {
    const bytes = new Uint8Array(10_000);
    for (let i = 0; i < bytes.length; i += 1) {
      bytes[i] = i % 256;
    }
    expect(bytesFromByteString(byteStringFromBytes(bytes))).toEqual(bytes);
  });

  it("never attempts UTF-8 interpretation: a lone continuation byte survives", () => {
    // 0x80 is not valid standalone UTF-8; a `TextDecoder`/`TextEncoder`
    // round trip would replace it with U+FFFD and corrupt it. This is
    // exactly the failure mode the module doc warns about.
    const bytes = new Uint8Array([0x80, 0xff, 0x00, 0x41]);
    expect(bytesFromByteString(byteStringFromBytes(bytes))).toEqual(bytes);
  });

  it("throws on a code unit outside 0..255", () => {
    expect(() => bytesFromByteString("caféሴ")).toThrow();
  });
});
