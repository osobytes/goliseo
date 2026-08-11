import { describe, expect, it } from "vitest";
import * as clips from "./clips.ts";
import * as masks from "./masks.ts";

// Quaternion components, read with a runtime bounds check: `Quat` is a fixed
// 4-tuple, but indexing it with a non-literal loop variable widens to
// `number | undefined` under `noUncheckedIndexedAccess`.
function comp(v: readonly number[], i: number): number {
  const x = v[i];
  if (x === undefined) {
    throw new Error(`index ${i} out of range`);
  }
  return x;
}

describe("rig3d clips", () => {
  it("every clip loops: the last keyframe matches the first", () => {
    // Looping clips only. KEEPER_SLING is a one-shot with a `fallback`, so
    // its last keyframe is a follow-through and must NOT match its first.
    const looping = [clips.ORDER[0], clips.ORDER[1], clips.RUN, clips.CHARGE, clips.KEEPER_GATHER];
    for (const clip of looping) {
      expect(clip).toBeDefined();
      if (!clip) continue;
      expect(clip.loop, `${clip.name} is listed here but does not declare loop`).toBe(true);
      const first = clips.sample(clip, 0);
      const last = clips.sample(clip, clip.duration - 1e-6);
      for (const [bone, q] of Object.entries(first.rot)) {
        const other = last.rot[bone];
        expect(other, `${clip.name}: ${bone} missing at loop point`).toBeDefined();
        if (!other) continue;
        for (let i = 0; i < 4; i++) {
          expect(comp(q, i)).toBeCloseTo(comp(other, i), 3);
        }
      }
    }
  });

  it("sampling wraps rather than running off the end", () => {
    const walk = clips.ORDER[1];
    expect(walk).toBeDefined();
    if (!walk) return;
    const a = clips.sample(walk, 0.1);
    const b = clips.sample(walk, walk.duration + 0.1);
    for (const [bone, q] of Object.entries(a.rot)) {
      const other = b.rot[bone];
      expect(other).toBeDefined();
      if (!other) continue;
      for (let i = 0; i < 4; i++) {
        expect(comp(q, i)).toBeCloseTo(comp(other, i), 9);
      }
    }
  });

  it("layer leaves bones outside the mask untouched", () => {
    const walk = clips.ORDER[1];
    expect(walk).toBeDefined();
    if (!walk) return;
    const base = clips.sample(walk, 0.2);
    const overlay = clips.sample(clips.CHARGE, 0.1);
    const out = clips.layer(base, overlay, masks.UPPER_BODY, 1);
    // Legs are not in UPPER_BODY, so they must survive verbatim.
    for (const bone of ["thigh.R", "shin.R", "foot.L", "toe.L"]) {
      const baseQ = base.rot[bone];
      if (baseQ) {
        const outQ = out.rot[bone];
        expect(outQ).toBeDefined();
        if (!outQ) continue;
        for (let i = 0; i < 4; i++) {
          expect(comp(outQ, i)).toBeCloseTo(comp(baseQ, i), 12);
        }
      }
    }
  });

  it("layer at weight 0 is the base pose", () => {
    const walk = clips.ORDER[1];
    expect(walk).toBeDefined();
    if (!walk) return;
    const base = clips.sample(walk, 0.3);
    const overlay = clips.sample(clips.CHARGE, 0.2);
    const out = clips.layer(base, overlay, masks.UPPER_BODY, 0);
    for (const [bone, q] of Object.entries(base.rot)) {
      const outQ = out.rot[bone];
      expect(outQ).toBeDefined();
      if (!outQ) continue;
      for (let i = 0; i < 4; i++) {
        expect(comp(outQ, i)).toBeCloseTo(comp(q, i), 9);
      }
    }
  });

  it("layer at weight 1 takes the overlay on masked bones", () => {
    const walk = clips.ORDER[1];
    expect(walk).toBeDefined();
    if (!walk) return;
    const base = clips.sample(walk, 0.3);
    const overlay = clips.sample(clips.CHARGE, 0.2);
    const out = clips.layer(base, overlay, masks.UPPER_BODY, 1);
    for (const [bone, q] of Object.entries(overlay.rot)) {
      if (masks.UPPER_BODY.has(bone)) {
        const outQ = out.rot[bone];
        expect(outQ).toBeDefined();
        if (!outQ) continue;
        for (let i = 0; i < 4; i++) {
          expect(comp(outQ, i)).toBeCloseTo(comp(q, i), 9);
        }
      }
    }
  });

  it("masks include the sockets attached to the hands they cover", () => {
    // A socket left out of the mask keeps the base layer's transform while
    // the arm follows the overlay, and the weapon detaches from the fist.
    for (const mask of [masks.UPPER_BODY, masks.ARMS]) {
      expect(mask.has("hand.R") && mask.has("socket_hand.R"), "socket must accompany its hand").toBe(true);
      expect(mask.has("hand.L") && mask.has("socket_hand.L"), "socket must accompany its hand").toBe(true);
    }
  });
});
