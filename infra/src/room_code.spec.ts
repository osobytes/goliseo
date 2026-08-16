import { describe, expect, it } from "vitest";

import {
  ROOM_CODE_ALPHABET,
  ROOM_CODE_LENGTH,
  generateRoomCode,
  isValidRoomCode,
} from "./room_code.ts";

describe("generateRoomCode", () => {
  it("produces a code of the fixed length, drawn from the alphabet", () => {
    const bytes = new Uint8Array([1, 2, 3, 4, 5, 6]);
    const code = generateRoomCode(bytes);
    expect(code).toHaveLength(ROOM_CODE_LENGTH);
    for (const ch of code) {
      expect(ROOM_CODE_ALPHABET.includes(ch)).toBe(true);
    }
  });

  it("is deterministic given the same random bytes", () => {
    const bytes = new Uint8Array([200, 5, 91, 254, 0, 33]);
    expect(generateRoomCode(bytes)).toBe(generateRoomCode(bytes));
  });

  it("uses extra bytes beyond ROOM_CODE_LENGTH without complaint", () => {
    const bytes = new Uint8Array([9, 9, 9, 9, 9, 9, 9, 9, 9, 9]);
    expect(generateRoomCode(bytes)).toHaveLength(ROOM_CODE_LENGTH);
  });

  it("throws when handed fewer random bytes than the code needs", () => {
    expect(() => generateRoomCode(new Uint8Array([1, 2, 3]))).toThrow();
  });

  it("never produces the excluded ambiguous letters I, L, O, U", () => {
    // Exhaust every possible byte value at every position once; the
    // alphabet excludes I/L/O/U by construction, so a byte-driven mapping
    // can never select them either.
    for (let b = 0; b < 256; b += 1) {
      const code = generateRoomCode(new Uint8Array(ROOM_CODE_LENGTH).fill(b));
      expect(code).not.toMatch(/[ILOU]/);
    }
  });
});

describe("isValidRoomCode", () => {
  it("accepts a code generateRoomCode could have produced", () => {
    expect(isValidRoomCode(generateRoomCode(new Uint8Array([1, 2, 3, 4, 5, 6])))).toBe(true);
  });

  it("rejects the wrong length", () => {
    expect(isValidRoomCode("ABC12")).toBe(false);
    expect(isValidRoomCode("ABC1234")).toBe(false);
    expect(isValidRoomCode("")).toBe(false);
  });

  it("rejects lowercase", () => {
    expect(isValidRoomCode("abc123")).toBe(false);
  });

  it("rejects the excluded ambiguous letters", () => {
    expect(isValidRoomCode("I2345L")).toBe(false);
    expect(isValidRoomCode("ABCDEO")).toBe(false);
    expect(isValidRoomCode("ABCDEU")).toBe(false);
  });

  it("rejects non-alphabet punctuation", () => {
    expect(isValidRoomCode("AB-123")).toBe(false);
  });
});
