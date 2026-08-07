// Exercises the two binding strategies gc-wasm uses (see
// `v2/rust/crates/gc-wasm/src/lib.rs`'s doc): the wasm-bindgen session
// lifecycle, and the raw per-frame render path reading the SAME
// `WebAssembly.Instance`'s linear memory `Session` lives in.
//
// Requires `pnpm --filter @gc/wasm build` to have run first.

import { describe, expect, it } from "vitest";

import { loadSimHost } from "./index.ts";

/** A canonical, all-neutral `gc_sim::input_frame` wire for `tick`. Field
 * order and comma-separated `move_x,move_y,held,edges` shape per
 * `gc_sim::input_frame::decode`. */
function neutralWire(tick: number): string {
  const slot = "0,0,0,0";
  return ["2", String(tick), ...Array.from({ length: 8 }, () => slot)].join("|");
}

describe("Session lifecycle", () => {
  it("rejects an unknown team id", () => {
    const { Session } = loadSimHost();
    expect(() => new Session("not-a-team", "orion", 1, 20, 3)).toThrow();
  });

  it("constructs, steps, and reports state", () => {
    const { Session } = loadSimHost();
    const session = new Session("nebula", "orion", 7, 20, 3);
    try {
      expect(session.inputTick).toBe(0);
      expect(session.finished).toBe(false);
      expect(session.scoreHome).toBe(0);
      expect(session.scoreAway).toBe(0);

      session.step(neutralWire(0));

      expect(session.inputTick).toBe(1);
      expect(session.snapshotHash()).toMatch(/^[0-9a-f]+$/);
    } finally {
      session.free();
    }
  });

  it("rejects a malformed input wire", () => {
    const { Session } = loadSimHost();
    const session = new Session("nebula", "orion", 7, 20, 3);
    try {
      expect(() => session.step("not a wire")).toThrow();
    } finally {
      session.free();
    }
  });

  it("exposes the match-constant roster once, not per frame", () => {
    const { Session } = loadSimHost();
    const session = new Session("nebula", "orion", 7, 20, 3);
    try {
      expect(session.rosterNumeric()).toBeInstanceOf(Float64Array);
      expect(session.rosterNumeric().length).toBeGreaterThan(0);
      expect(session.rosterIdsAndNames()).toContain("ozzo");
    } finally {
      session.free();
    }
  });

  it("accepts an explicit home formation override, and still constructs without one", () => {
    // Before this parameter existed, `Session::new` hard-coded
    // `home_formation: None` -- nothing on `@gc/wasm`'s surface could
    // select a formation at all (see `crates/gc-wasm/src/session.rs`'s
    // `Session::new` doc). This does not validate `"2-1-1"` against
    // `gc_data::formations::ALL` itself (that stays a caller/screen
    // responsibility per that same doc), only proves the parameter reaches
    // construction without erroring either way.
    const { Session } = loadSimHost();
    const withFormation = new Session("nebula", "orion", 7, 20, 3, "2-1-1");
    const withoutFormation = new Session("nebula", "orion", 7, 20, 3);
    try {
      expect(withFormation.inputTick).toBe(0);
      expect(withoutFormation.inputTick).toBe(0);
    } finally {
      withFormation.free();
      withoutFormation.free();
    }
  });
});

describe("the raw per-frame render path", () => {
  it("builds a frame and reads it back as a zero-copy Float64Array view", () => {
    const host = loadSimHost();
    const session = new host.Session("nebula", "orion", 7, 20, 3);
    try {
      session.step(neutralWire(0));

      const frame = host.buildRenderFrame(session.handle);
      expect(frame).not.toBeNull();
      expect(frame).toBeInstanceOf(Float64Array);
      // Header word 0 is gc_render::frame_buffer::MAGIC (0x474F_4C46).
      expect(frame?.[0]).toBe(0x474f_4c46);
    } finally {
      session.free();
    }
  });

  it("returns null for a handle naming no live session", () => {
    const host = loadSimHost();
    expect(host.buildRenderFrame(999_999)).toBeNull();
  });

  it("invalidates the handle once the session is freed", () => {
    const host = loadSimHost();
    const session = new host.Session("nebula", "orion", 7, 20, 3);
    const handle = session.handle;
    session.free();
    expect(host.buildRenderFrame(handle)).toBeNull();
  });
});
