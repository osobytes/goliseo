// Exercises the two binding strategies gc-wasm uses (see
// `rust/crates/gc-wasm/src/lib.rs`'s doc): the wasm-bindgen session
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

      const frame = host.buildRenderFrame(session.handle, 0);
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
    expect(host.buildRenderFrame(999_999, 0)).toBeNull();
  });

  it("invalidates the handle once the session is freed", () => {
    const host = loadSimHost();
    const session = new host.Session("nebula", "orion", 7, 20, 3);
    const handle = session.handle;
    session.free();
    expect(host.buildRenderFrame(handle, 0)).toBeNull();
  });
});

describe("a session drives a live match, not a permanent scoreless stalemate", () => {
  // `gc_render::frame_buffer::encode`'s header word index 8 is
  // `event_count` (the header layout `session.spec.ts`'s own MAGIC check
  // above already reads word 0 from). Reading it directly off the raw
  // Float64Array header is cheaper than decoding the whole frame, and is
  // enough to prove a tick produced discrete match events.
  const EVENT_COUNT_HEADER_INDEX = 8;

  // Regression test for this wave's bug: `Session::step` used to advance
  // the simulation directly off the raw `input_frame` wire it received,
  // which only ever carries a real sample for the local `home_1` slot --
  // every other one of the eight canonical slots stayed the wire's literal
  // neutral row forever (see `crates/gc-wasm/src/session.rs`'s module doc,
  // "Slot mode has no legacy-input fallback"). A match driven entirely
  // through this package's compiled artifact -- the actual surface a
  // browser loads -- must still be live even when the local player never
  // touches an input: some tick along the way must report a discrete match
  // event, or the match must not end 0-0. Before the fix this held on
  // every seed: `finished === true`, `scoreHome === 0`, `scoreAway === 0`,
  // and every single tick's `event_count` header word was `0`.
  it("reaches a non-zero event count or a non-zero score over a full match on an idle local wire", () => {
    const host = loadSimHost();
    for (const seed of [1, 17, 42, 120]) {
      // `99` mirrors `gc_sim::r#match::NO_GOAL_LIMIT`: a cap high enough
      // that a two-minute match never hits it, so `finished` is driven by
      // the clock, exactly like an ordinary browser match.
      const session = new host.Session("nebula", "orion", seed, 120, 99);
      let totalEvents = 0;
      try {
        while (!session.finished) {
          session.step(neutralWire(session.inputTick));
          const frame = host.buildRenderFrame(session.handle, 0);
          expect(frame).not.toBeNull();
          totalEvents += frame?.[EVENT_COUNT_HEADER_INDEX] ?? 0;
        }
        expect(
          totalEvents > 0 || session.scoreHome > 0 || session.scoreAway > 0,
          `seed ${seed}: a full match produced zero events and a 0-0 score -- ` +
            "every non-local slot is still receiving permanent neutral input",
        ).toBe(true);
      } finally {
        session.free();
      }
    }
  });
});
