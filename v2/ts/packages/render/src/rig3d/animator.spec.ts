// Tier-2 tests for the character animator: the composition of the three
// mixer layers, the crossfade, the lean, and the user-facing claim #425's
// asset-agnostic slice makes -- that a set of poses which used to render as a
// plain jog now have their own silhouette.
//
// Headless (no GL, no window): the animator only ever produces a sparse
// `{rot, move}` pose. What is NOT verified here, and cannot be from a test
// runner, is whether each silhouette READS as the thing it is named after; see
// the PR's note on the visual pass this slice does not include.

import { beforeEach, describe, expect, it } from "vitest";
import { quat, type Quat } from "@gc/core";
import * as actionPose from "./action_pose.ts";
import { poseFor, reset, applyLean, basePose, LEAN_ROOT_DEGREES } from "./animator.ts";
import type { AnimatorOptions, AnimatorView } from "./animator.ts";
import { POSE_ACTIONS } from "./pose_table.ts";

const RUNNING: AnimatorView = { speed: 260, gait: 0.31, lean: 0 };
const STANDING: AnimatorView = { speed: 0, gait: 0, lean: 0 };

function opts(overrides: Partial<AnimatorOptions> = {}): AnimatorOptions {
  return overrides;
}

function withPose(id: string, overrides: Partial<AnimatorOptions> = {}): AnimatorOptions {
  return { pose: { id }, ...overrides };
}

// Largest absolute component difference across every bone either pose touches.
function delta(a: actionPose.MutablePose, b: actionPose.MutablePose): number {
  let worst = 0;
  for (const bone of new Set([...Object.keys(a.rot), ...Object.keys(b.rot)])) {
    const qa: Quat = a.rot[bone] ?? [0, 0, 0, 1];
    const qb: Quat = b.rot[bone] ?? [0, 0, 0, 1];
    const dot = qa[0] * qb[0] + qa[1] * qb[1] + qa[2] * qb[2] + qa[3] * qb[3];
    const sign = dot < 0 ? -1 : 1;
    for (let i = 0; i < 4; i += 1) {
      worst = Math.max(worst, Math.abs((qa[i] ?? 0) - sign * (qb[i] ?? 0)));
    }
  }
  return worst;
}

let nextId = 0;
function freshId(): string {
  nextId += 1;
  return `p${nextId}`;
}

beforeEach(() => {
  reset();
});

describe("rig3d/animator.poseFor", () => {
  it("is deterministic for the same character, inputs and time", () => {
    const id = freshId();
    const a = poseFor(id, RUNNING, withPose("combat_guard"), 1.23);
    const b = poseFor(id, RUNNING, withPose("combat_guard"), 1.23);
    expect(delta(a, b)).toBe(0);
  });

  it("animates two characters independently -- posing one does not disturb the other", () => {
    const guard = freshId();
    const plain = freshId();
    const plainAlone = poseFor(plain, RUNNING, withPose("locomotion"), 2);
    poseFor(guard, RUNNING, withPose("combat_guard"), 2);
    const plainAgain = poseFor(plain, RUNNING, withPose("locomotion"), 2);
    expect(delta(plainAlone, plainAgain)).toBe(0);
  });

  // THE HEADLINE. Each of these rendered as a plain jog before #425 -- their
  // pose ids reached neither `POSE_CLIP` nor `action_pose.forOptions`.
  it.each([
    ["keeper_grab", {}],
    ["keeper_throw", { throw: 0.6 }],
    ["keeper_punt", { throw: 0.6 }],
    ["keeper_set", {}],
    ["keeper_ready_low", {}],
    ["soccer_windup", { windup: 0.4 }],
    ["tackle", {}],
  ])("gives %s a silhouette of its own instead of plain locomotion", (id, extra) => {
    const plain = poseFor(freshId(), RUNNING, withPose("locomotion"), 3);
    const posed = poseFor(freshId(), RUNNING, withPose(id, extra as Partial<AnimatorOptions>), 3);
    expect(delta(plain, posed)).toBeGreaterThan(0.05);
  });

  // Previously `combat_windup` shared the guard stance with `aim` and
  // `recovery`, so a player about to strike looked exactly like one holding
  // guard. `clips.SWING`'s windup key was authored for this and had no caller.
  it("distinguishes a combat windup from the guard stance it used to share", () => {
    const guard = poseFor(freshId(), RUNNING, withPose("combat_guard"), 3);
    const windup = poseFor(freshId(), RUNNING, withPose("combat_windup", { windup: 0.5 }), 3);
    expect(delta(guard, windup)).toBeGreaterThan(0.05);
  });

  it("holds a windup's coiled key while its timer runs and its contact key once it expires", () => {
    const coiled = poseFor(freshId(), RUNNING, withPose("soccer_windup", { windup: 0.5 }), 3);
    const contact = poseFor(freshId(), RUNNING, withPose("soccer_windup", { windup: 0 }), 3);
    expect(delta(coiled, contact)).toBeGreaterThan(0.05);
  });

  // The other half of the explicit table: entries with no action of their own
  // must be indistinguishable from plain locomotion, not accidentally
  // different from it. That covers both the poses that are SUPPOSED to look
  // like locomotion and the ones recorded as gaps (`slide`, `contain`,
  // `fatigue`, ...) -- the point of the table is that the difference is
  // written down, not that the two render differently today.
  it.each([
    "keeper_shuffle",
    "keeper_ready_tall",
    "kick_follow",
    "settle",
    "run_telegraph",
    "contain",
    "fatigue",
    "slide",
  ])("leaves %s on the locomotion base exactly as `locomotion` itself is", (id) => {
    expect(POSE_ACTIONS[id as keyof typeof POSE_ACTIONS].action).toBeNull();
    const plain = poseFor(freshId(), RUNNING, withPose("locomotion"), 4);
    const posed = poseFor(freshId(), RUNNING, withPose(id), 4);
    expect(delta(plain, posed)).toBe(0);
  });

  it("leaves the root-overlay poses to action_pose.ts, which still reaches the root", () => {
    const grounded = poseFor(freshId(), RUNNING, withPose("locomotion"), 5);
    const diving = poseFor(freshId(), RUNNING, withPose("keeper_dive", { dive: 1, dive_dir: { x: 0, y: 1 }, facing: { x: 1, y: 0 } }), 5);
    expect(diving.rot["root"]).toBeDefined();
    expect(delta(grounded, diving)).toBeGreaterThan(0.05);
  });

  it("keeps the possession override outranking the pose id, as poseFor always had it", () => {
    // A keeper whose pose id says "guard" but whose hands are round the ball
    // must show the gather, not the guard.
    const guard = poseFor(freshId(), STANDING, withPose("combat_guard"), 6);
    const holding = poseFor(freshId(), STANDING, withPose("combat_guard", { holding: true }), 6);
    const gather = poseFor(freshId(), STANDING, withPose("keeper_grab"), 6);
    expect(delta(guard, holding)).toBeGreaterThan(0.05);
    // Not bit-identical: the two reach the same clip through different mixer
    // layers, whose baked tracks are Float32Array-backed.
    expect(delta(holding, gather)).toBeLessThan(1e-6);
  });
});

describe("rig3d/animator crossfade", () => {
  it("commits outright on a character's first observed frame, rather than easing in from nothing", () => {
    const snapped = poseFor(freshId(), RUNNING, withPose("combat_guard"), 0);
    const plain = poseFor(freshId(), RUNNING, withPose("locomotion"), 0);
    expect(delta(snapped, plain)).toBeGreaterThan(0.05);
  });

  it("eases a stance in over its own crossfade duration once the character is known", () => {
    const id = freshId();
    const crossfade = POSE_ACTIONS.combat_guard.crossfade;
    poseFor(id, RUNNING, withPose("locomotion"), 0);
    const quarter = poseFor(id, RUNNING, withPose("combat_guard"), crossfade / 4);
    const plain = poseFor(freshId(), RUNNING, withPose("locomotion"), crossfade / 4);
    const full = poseFor(freshId(), RUNNING, withPose("combat_guard"), crossfade / 4);
    const partial = delta(plain, quarter);
    expect(partial).toBeGreaterThan(0);
    expect(partial).toBeLessThan(delta(plain, full));
  });

  it("reaches the full stance once the crossfade has elapsed", () => {
    const id = freshId();
    const crossfade = POSE_ACTIONS.combat_guard.crossfade;
    // Real frames, not one long step: the frame clamp deliberately refuses to
    // integrate a delta longer than 0.1s (see `MAX_FRAME_DT`).
    const dt = 1 / 60;
    let t = 0;
    poseFor(id, RUNNING, withPose("locomotion"), t);
    for (let elapsed = 0; elapsed <= crossfade; elapsed += dt) {
      t += dt;
      poseFor(id, RUNNING, withPose("combat_guard"), t);
    }
    const arrived = poseFor(id, RUNNING, withPose("combat_guard"), t);
    const full = poseFor(freshId(), RUNNING, withPose("combat_guard"), t);
    expect(delta(arrived, full)).toBeLessThan(1e-9);
  });

  // A stance whose pose id is replaced by one with no action of its own has
  // nothing to fade IN, so the fade OUT has to keep the duration the outgoing
  // action was engaged with -- otherwise every stance would pop off.
  it("fades a stance out over the duration it was engaged with, rather than snapping", () => {
    const id = freshId();
    const crossfade = POSE_ACTIONS.combat_recovery.crossfade;
    poseFor(id, RUNNING, withPose("combat_recovery"), 0);
    const halfway = poseFor(id, RUNNING, withPose("locomotion"), crossfade / 2);
    const plain = poseFor(freshId(), RUNNING, withPose("locomotion"), crossfade / 2);
    expect(delta(plain, halfway)).toBeGreaterThan(0);
  });

  it("does not integrate a frame longer than the clamp, so a backgrounded tab cannot snap every stance", () => {
    const id = freshId();
    poseFor(id, RUNNING, withPose("locomotion"), 0);
    // 10 seconds of wall clock in one step; the clamp caps the integrated
    // delta well below `combat_recovery`'s crossfade, so this must still be
    // partial rather than complete.
    const jumped = poseFor(id, RUNNING, withPose("combat_recovery"), 10);
    const full = poseFor(freshId(), RUNNING, withPose("combat_recovery"), 10);
    expect(delta(jumped, full)).toBeGreaterThan(0);
  });

  it("forgets a character's stance on reset, so a fresh match does not inherit one", () => {
    const id = "stable-roster-slot";
    poseFor(id, RUNNING, withPose("combat_guard"), 0);
    reset();
    // Post-reset the id is unknown again, so its first frame snaps -- which is
    // observable as the stance being FULL rather than a quarter faded.
    const afterReset = poseFor(id, RUNNING, withPose("combat_guard"), 0.04);
    const full = poseFor(freshId(), RUNNING, withPose("combat_guard"), 0.04);
    expect(delta(afterReset, full)).toBeLessThan(1e-9);
  });
});

describe("rig3d/animator lean", () => {
  it("tilts the root and takes part of it back on the spine", () => {
    const upright: actionPose.MutablePose = { rot: {}, move: {} };
    const leaning = applyLean({ rot: {}, move: {} }, 1);
    expect(delta(upright, leaning)).toBeGreaterThan(0);
    const root = leaning.rot["root"];
    const spine = leaning.rot["spine"];
    if (root === undefined || spine === undefined) {
      throw new Error("expected a root and spine rotation");
    }
    // Full lean rolls the root by LEAN_ROOT_DEGREES about z; the spine takes
    // back a signed fraction of it, so the two roll opposite ways.
    const expected = quat.fromEuler(0, 0, (-LEAN_ROOT_DEGREES * Math.PI) / 180);
    expect(root[2]).toBeCloseTo(expected[2], 9);
    expect(Math.sign(spine[2])).toBe(-Math.sign(root[2]));
  });

  it("is a no-op at zero and mirrors itself either side of it", () => {
    const flat = applyLean({ rot: {}, move: {} }, 0);
    expect(flat.rot["root"]).toBeUndefined();
    const left = applyLean({ rot: {}, move: {} }, 0.6).rot["root"];
    const right = applyLean({ rot: {}, move: {} }, -0.6).rot["root"];
    if (left === undefined || right === undefined) {
      throw new Error("expected root rotations");
    }
    expect(left[2]).toBeCloseTo(-right[2], 12);
  });

  it("reaches the composed pose, so a running character actually leans", () => {
    const straight = poseFor(freshId(), { ...RUNNING, lean: 0 }, opts(), 7);
    const leaning = poseFor(freshId(), { ...RUNNING, lean: 0.9 }, opts(), 7);
    expect(delta(straight, leaning)).toBeGreaterThan(0.01);
  });

  // Applied BEFORE action_pose, so a committed whole-body action -- which sets
  // the root outright -- wins. A keeper mid-save is not also cornering.
  it("yields the root to a whole-body action rather than fighting it", () => {
    const dive: Partial<AnimatorOptions> = { dive: 1, dive_dir: { x: 0, y: 1 }, facing: { x: 1, y: 0 } };
    const straight = poseFor(freshId(), { ...RUNNING, lean: 0 }, withPose("keeper_dive", dive), 8);
    const leaning = poseFor(freshId(), { ...RUNNING, lean: 0.9 }, withPose("keeper_dive", dive), 8);
    expect(straight.rot["root"]).toEqual(leaning.rot["root"]);
  });
});

describe("rig3d/animator.basePose", () => {
  it("changes with speed and with gait phase, independently", () => {
    const slow = basePose({ speed: 40, gait: 0.25, lean: 0 }, 0);
    const fast = basePose({ speed: 380, gait: 0.25, lean: 0 }, 0);
    const shifted = basePose({ speed: 380, gait: 0.75, lean: 0 }, 0);
    expect(delta(slow, fast)).toBeGreaterThan(0.05);
    expect(delta(fast, shifted)).toBeGreaterThan(0.05);
  });

  it("keeps the idle clip inside its own keyframes however long the clock has been running", () => {
    // `MixerLayer` advances by zero, so nothing wraps the pinned phase for it.
    // An unwrapped `now * IDLE_RATE` would walk past the idle clip's last key
    // after nine seconds and freeze there for the rest of the match.
    const early = basePose({ speed: 0, gait: 0, lean: 0 }, 1.0);
    const late = basePose({ speed: 0, gait: 0, lean: 0 }, 1.0 + 3.4 / 0.35);
    expect(delta(early, late)).toBeLessThan(1e-6);
    const midCycle = basePose({ speed: 0, gait: 0, lean: 0 }, 1.0 + 3.4 / 0.35 / 2);
    expect(delta(early, midCycle)).toBeGreaterThan(0.005);
  });
});
