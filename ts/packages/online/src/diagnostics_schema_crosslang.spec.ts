// Cross-language guards for the diagnostics content digest.
//
// diagnostics_schema is a canonical serializer with a versioned digest
// (`fnv1a64/v1`). It is consumed here AND by desync_package in Rust, and a
// desync package is evidence peers exchange — so a digest computed in Rust on
// one client must equal one computed here on another. See ARCHITECTURE.md §1.2.
//
// Both defects these tests pin were real and shipped in the first port:
//   1. `toPrecision(17)` was used for `%.17g`. It pads trailing zeros where
//      `%g` strips them, so 480.75 hashed differently in each language — and
//      so did the integer 100.
//   2. `TextEncoder` UTF-8-encoded the payload. Lua strings are raw bytes, so
//      the single byte 0xff became the two bytes c3 bf.

import { describe, expect, it } from "vitest";
import { fnv1a64Hash, formatG17 } from "./diagnostics_schema.ts";

describe("formatG17 reproduces Lua's %.17g", () => {
  // Captured from the original Lua implementation, under headless LÖVE:
  //   print(string.format("%.17g", v))
  const cases: ReadonlyArray<readonly [number, string]> = [
    [0, "0"],
    [1, "1"],
    [-1, "-1"],
    [100, "100"],
    [480.75, "480.75"],
    [-3.5, "-3.5"],
    [0.1, "0.10000000000000001"],
    [1 / 3, "0.33333333333333331"],
    [1e-4, "0.0001"],
    [1e-5, "1.0000000000000001e-05"],
    [1e5, "100000"],
    [1e17, "1e+17"],
    [1e21, "1e+21"],
    [9007199254740991, "9007199254740991"],
    [-9007199254740991, "-9007199254740991"],
    [0.5, "0.5"],
    [1234.5678, "1234.5678"],
    [-0.0009765625, "-0.0009765625"],
    [1.7976931348623157e308, "1.7976931348623157e+308"],
    [5e-324, "4.9406564584124654e-324"],
  ];

  for (const [value, expected] of cases) {
    it(`formats ${expected}`, () => {
      expect(formatG17(value)).toBe(expected);
    });
  }

  it("strips trailing zeros where toPrecision would pad them", () => {
    // The exact regression: these two disagreed across languages.
    expect(formatG17(480.75)).toBe("480.75");
    expect((480.75).toPrecision(17)).toBe("480.75000000000000");
    expect(formatG17(100)).toBe("100");
    expect((100).toPrecision(17)).toBe("100.00000000000000");
  });
});

describe("fnv1a64Hash treats its input as raw bytes, not UTF-8", () => {
  // The canonical published FNV-1a-64 vectors, originally pinned by the Lua
  // implementation's spec/core/fnv1a64_spec.lua and still checked by
  // crates/gc-core/tests/fnv1a64.rs.
  it("matches the published vectors", () => {
    expect(fnv1a64Hash("")).toBe("cbf29ce484222325");
    expect(fnv1a64Hash("a")).toBe("af63dc4c8601ec8c");
    expect(fnv1a64Hash("foobar")).toBe("85944171f73967e8");
    expect(fnv1a64Hash("hello")).toBe("a430d84680aabd0b");
  });

  it("hashes every byte value 0-255 the same as Lua", () => {
    // The Lua implementation's spec/core/fnv1a64_spec.lua built this the
    // same way, with string.char(0..255).
    let all = "";
    for (let byte = 0; byte <= 255; byte += 1) {
      all += String.fromCharCode(byte);
    }
    expect(fnv1a64Hash(all)).toBe("4242dc5249c33625");
  });

  it("does not conflate byte 0xff with U+00FF's UTF-8 encoding", () => {
    const oneRawByte = String.fromCharCode(0xff);
    expect(oneRawByte.length).toBe(1);
    // Under the old TextEncoder path this hashed the two bytes c3 bf instead.
    expect(fnv1a64Hash(oneRawByte)).not.toBe(fnv1a64Hash("Ã¿"));
  });
});
