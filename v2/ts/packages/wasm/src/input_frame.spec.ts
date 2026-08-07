// Exercises `crates/gc-wasm/src/input_frame_bridge.rs`'s wasm-bindgen
// surface against the real compiled artifact, under node -- not just the
// Rust crate's own native `cargo test` (which cannot reach a `Result<_,
// JsValue>` error path at all off wasm32; see that module's test comment).
//
// Requires `pnpm --filter @gc/wasm build` to have run first.

import { describe, expect, it } from "vitest";

import { loadSimHost } from "./index.ts";

describe("inputFrame bridge", () => {
  it("neutralSample is the canonical all-zero wire", () => {
    const host = loadSimHost();
    expect(host.inputFrameNeutralSample()).toBe("7f7f0000");
  });

  it("newSample builds a validated sample and round-trips through decodeSampleJson", () => {
    const host = loadSimHost();
    const wire = host.inputFrameNewSample(10, -10, 3, 1);
    expect(wire).toMatch(/^[0-9a-f]{8}$/);

    const decoded = JSON.parse(host.inputFrameDecodeSampleJson(wire)) as {
      move_x: number;
      move_y: number;
      held: number;
      edges: number;
    };
    expect(decoded).toEqual({ move_x: 10, move_y: -10, held: 3, edges: 1 });
  });

  it("newSample with no overrides matches neutralSample", () => {
    const host = loadSimHost();
    expect(host.inputFrameNewSample()).toBe(host.inputFrameNeutralSample());
  });

  it("newSample throws (does not abort) on an out-of-range axis", () => {
    const host = loadSimHost();
    expect(() => host.inputFrameNewSample(1000)).toThrow();
  });

  it("newSample throws on an invalid equipment held/edge combination", () => {
    const host = loadSimHost();
    // `equipment_pressed` (edge bit 32) without `equipment` held and
    // without `equipment_released` is the exact invalid combination
    // `gc_sim::input_frame::validate_sample` rejects.
    const edgeBits = JSON.parse(host.inputFrameEdgeBitsJson()) as Record<string, number>;
    expect(() => host.inputFrameNewSample(0, 0, 0, edgeBits.equipment_pressed)).toThrow();
  });

  it("decodeSampleJson throws on a malformed wire", () => {
    const host = loadSimHost();
    expect(() => host.inputFrameDecodeSampleJson("not-a-wire")).toThrow();
  });

  it("EDGE_BITS/held bits match the canonical wire names", () => {
    const host = loadSimHost();
    const edgeBits = JSON.parse(host.inputFrameEdgeBitsJson()) as Record<string, number>;
    expect(edgeBits).toEqual({
      shoot: 1,
      pass: 2,
      switch: 4,
      dash: 8,
      dodge: 16,
      equipment_pressed: 32,
      equipment_released: 64,
    });

    const heldBits = JSON.parse(host.inputFrameHeldBitsJson()) as Record<string, number>;
    expect(heldBits).toEqual({
      shoot: 1,
      pass: 2,
      sprint: 4,
      jockey: 8,
      lob: 16,
      aerial_strike: 32,
      aerial_acrobatic: 64,
      equipment: 128,
    });
  });

  it("constantsJson reports the canonical eight-slot bound", () => {
    const host = loadSimHost();
    const constants = JSON.parse(host.inputFrameConstantsJson()) as { slot_count: number };
    expect(constants.slot_count).toBe(8);
  });

  it("quantizeAxis clamps and rounds like the native sim", () => {
    const host = loadSimHost();
    expect(host.inputFrameQuantizeAxis(1)).toBe(127);
    expect(host.inputFrameQuantizeAxis(-1)).toBe(-127);
    expect(host.inputFrameQuantizeAxis(0)).toBe(0);
  });

  it("quantizeAxis throws on a non-finite axis", () => {
    const host = loadSimHost();
    expect(() => host.inputFrameQuantizeAxis(Number.NaN)).toThrow();
  });
});
