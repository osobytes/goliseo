// Pose LOD policy tests (#394/#400).
//
// The module under test is deliberately unwired — see pose_lod.ts's header
// for why its performance justification does not necessarily hold on this
// stack. The tests come along regardless: the policy encodes real decisions
// about what may be degraded, and those are worth pinning whether or not it
// is switched on.

import { describe, expect, it } from "vitest";
import type { PlayerRenderOptions } from "../player_render_options.ts";
import {
  FULL_RATE_HEIGHT_PX,
  PoseLodScheduler,
  RELAXED_POSE,
  REDUCED_INTERVAL,
  due,
  interval,
} from "./pose_lod.ts";

const SMALL = FULL_RATE_HEIGHT_PX - 1;

function optsFor(
  id: string | undefined,
  extra: Partial<PlayerRenderOptions> = {},
): PlayerRenderOptions {
  return {
    is_keeper: false,
    controlled: false,
    ...(id !== undefined ? { pose: { id, priority: 0, source: "test" } } : {}),
    ...extra,
  };
}

describe("pose_lod.interval", () => {
  it("holds every id in the gait family, and the set is exhaustive", () => {
    // Pinning the set exhaustively is the point: a dropped or mistyped entry
    // silently stops degrading a pose, or starts degrading one it should not.
    expect([...RELAXED_POSE].sort()).toEqual([
      "contain",
      "fatigue",
      "keeper_shuffle",
      "kick_follow",
      "locomotion",
      "run_telegraph",
      "settle",
    ]);
    for (const id of RELAXED_POSE) {
      expect(interval(optsFor(id), SMALL)).toBe(REDUCED_INTERVAL);
    }
  });

  it("holds a character with no pose id at all", () => {
    expect(interval(optsFor(undefined), SMALL)).toBe(REDUCED_INTERVAL);
  });

  it("refreshes every frame once the character is tall on screen", () => {
    expect(interval(optsFor("locomotion"), FULL_RATE_HEIGHT_PX + 1)).toBe(1);
  });

  it("never holds the controlled player", () => {
    expect(interval(optsFor("locomotion", { controlled: true }), SMALL)).toBe(1);
  });

  it("never holds a simulation-timer-driven action", () => {
    // Holding one of these desynchronises the body from the sim's own ramp.
    const active: ReadonlyArray<Partial<PlayerRenderOptions>> = [
      { dive: 0.5 },
      { aerial: 0.5 },
      { throw: 0.5 },
      { windup: 0.5 },
      { holding: true },
    ];
    for (const extra of active) {
      expect(interval(optsFor("locomotion", extra), SMALL)).toBe(1);
    }
  });

  it("never holds an unknown pose id", () => {
    // Conservative by construction: an id nobody listed cannot be degraded.
    for (const id of ["dive_left", "knockback", "combat_stance", "brand_new_pose"]) {
      expect(interval(optsFor(id), SMALL)).toBe(1);
    }
  });
});

describe("pose_lod.due", () => {
  it("is always due at full rate", () => {
    for (let tick = 0; tick < 8; tick += 1) {
      expect(due(tick, 0, 1)).toBe(true);
    }
  });

  it("alternates at interval 2", () => {
    for (let tick = 0; tick < 8; tick += 1) {
      expect(due(tick, 0, 2)).toBe(tick % 2 === 0);
    }
  });

  it("puts a stagger of 1 on the opposite frame", () => {
    // This is what turns ten characters refreshing together into five per
    // frame — the mean is unchanged, the p95 is halved.
    for (let tick = 0; tick < 8; tick += 1) {
      expect(due(tick, 0, 2)).toBe(!due(tick, 1, 2));
    }
  });
});

describe("PoseLodScheduler.step", () => {
  it("always refreshes a brand-new entry, whatever the schedule says", () => {
    // Rows that were never written must never reach the GPU.
    const scheduler = new PoseLodScheduler();
    const key = {};
    expect(scheduler.step(key, 2).refresh).toBe(true);
  });

  it("staggers consecutive characters onto opposite frames", () => {
    const scheduler = new PoseLodScheduler();
    const a = scheduler.step({}, 2).entry;
    const b = scheduler.step({}, 2).entry;
    expect(due(0, a.stagger, 2)).toBe(!due(0, b.stagger, 2));
  });

  it("follows the schedule once the entry is fresh", () => {
    const scheduler = new PoseLodScheduler();
    const key = {};
    scheduler.step(key, 2);
    for (let i = 0; i < 10; i += 1) {
      const { entry, refresh } = scheduler.step(key, 2);
      expect(refresh).toBe(due(entry.tick, entry.stagger, 2));
    }
  });

  it("refreshes immediately when the interval drops back to full rate", () => {
    // interval() returns 1 the moment a held character starts a dive; that must
    // take effect on the very next draw, not on the next scheduled slot.
    const scheduler = new PoseLodScheduler();
    const key = {};
    scheduler.step(key, 2);
    let heldFrames = 0;
    while (heldFrames < 4 && scheduler.step(key, 2).refresh) {
      heldFrames += 1;
    }
    expect(scheduler.step(key, 1).refresh).toBe(true);
  });

  it("advances each character's counter independently", () => {
    const scheduler = new PoseLodScheduler();
    const a = {};
    const b = {};
    scheduler.step(a, 2);
    scheduler.step(a, 2);
    scheduler.step(b, 2);
    expect(scheduler.step(a, 2).entry.tick).toBe(2);
    expect(scheduler.step(b, 2).entry.tick).toBe(1);
  });
});
