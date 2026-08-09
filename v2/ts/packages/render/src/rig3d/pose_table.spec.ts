// Tier-1 tests (AGENTS.md §9) for the pose -> action table and the blend
// maths that drives it. No display, no GL, no three.js: `pose_table.ts` is
// pure data plus arithmetic, which is the whole point of it being a separate
// module from `mixer.ts`.
//
// The load-bearing test in this file is `"has an explicit entry for every one
// of the 32 wire pose ids"`. #425 exists because 20 of the 32 fell through to
// a silent `"idle"`; a table that quietly loses an entry again would restore
// exactly that defect, and TypeScript's exhaustiveness check on
// `Record<PlayerPoseId, ...>` only catches it if `PlayerPoseId` itself is
// still complete -- so the id list below is spelled out independently rather
// than derived from the union it is meant to check.

import { describe, expect, it } from "vitest";
import { viewState } from "../view_state.ts";
import * as clips from "./clips.ts";
import {
  PHASE_BY_ACTION,
  POSE_ACTIONS,
  actionForPose,
  advanceCrossfade,
  animatedPoseIds,
  crossfadeTotal,
  locomotionBlend,
  locomotionTimeScale,
  newCrossfade,
  snapCrossfade,
  strideFor,
  type PoseActionEntry,
} from "./pose_table.ts";

// Spelled out, not derived: see this file's header.
const WIRE_POSE_IDS = [
  "keeper_grab",
  "keeper_throw",
  "keeper_punt",
  "keeper_tip",
  "keeper_spread",
  "keeper_central",
  "keeper_stretch",
  "keeper_dive",
  "keeper_get_up",
  "keeper_set",
  "keeper_ready_low",
  "keeper_shuffle",
  "keeper_ready_tall",
  "aerial_bicycle",
  "aerial_action",
  "combat_knockback",
  "combat_stagger",
  "combat_guard",
  "combat_active",
  "combat_windup",
  "combat_aim",
  "combat_recovery",
  "soccer_windup",
  "slide",
  "tackle",
  "stumble",
  "kick_follow",
  "settle",
  "run_telegraph",
  "contain",
  "fatigue",
  "locomotion",
] as const;

describe("rig3d/pose_table.POSE_ACTIONS", () => {
  it("has an explicit entry for every one of the 32 wire pose ids", () => {
    expect(WIRE_POSE_IDS).toHaveLength(32);
    expect(Object.keys(POSE_ACTIONS).sort()).toEqual([...WIRE_POSE_IDS].sort());
  });

  it("states exactly one of `action` or `fallback` per entry, with the dependent fields agreeing", () => {
    for (const id of WIRE_POSE_IDS) {
      const entry: PoseActionEntry = POSE_ACTIONS[id];
      const hasAction = entry.action !== null;
      expect(entry.fallback === null, `${id}: action and fallback must be mutually exclusive`).toBe(hasAction);
      expect(entry.mask !== null, `${id}: mask is meaningful iff there is an action`).toBe(hasAction);
      expect(entry.phase !== null, `${id}: phase is meaningful iff there is an action`).toBe(hasAction);
      expect(entry.crossfade > 0, `${id}: crossfade is meaningful iff there is an action`).toBe(hasAction);
      expect(entry.note.length, `${id}: every entry must say why it reads the way it does`).toBeGreaterThan(0);
    }
  });

  // The whole point of the rewrite: a pose with no action of its own must
  // record WHICH of the three unrelated situations it is in. `POSE_CLIP`'s
  // `"idle"` covered all three at once, which is why the third was invisible.
  it("distinguishes the three reasons a pose has no action of its own", () => {
    const byFallback = (want: string) => WIRE_POSE_IDS.filter((id) => POSE_ACTIONS[id].fallback === want);
    expect(byFallback("root_overlay")).toEqual([
      "keeper_tip",
      "keeper_spread",
      "keeper_central",
      "keeper_stretch",
      "keeper_dive",
      "keeper_get_up",
      "aerial_bicycle",
      "aerial_action",
      "combat_knockback",
      "combat_stagger",
      "stumble",
      // #430: the five body attitudes recovered from the deleted billboard.
      // They are root overlays for the same reason a dive is -- a whole-figure
      // lean and a crouch are transforms of the root, not limb animation --
      // but they are HELD postures rather than timed actions, which is what
      // `action_pose.ts` keeps out of `forOptions`.
      "kick_follow",
      "settle",
      "run_telegraph",
      "contain",
      "fatigue",
    ]);
    expect(byFallback("locomotion_base")).toEqual(["keeper_shuffle", "keeper_ready_tall", "locomotion"]);
    // THE REAL GAP, singular since #430. A slide is a whole-body ground action
    // that no root transform approximates, so it is blocked on #423's asset
    // set and #424's pipeline rather than merely unscheduled. If this list
    // grows, somebody added a pose id without deciding how it animates and
    // this test says so.
    expect(byFallback("unanimated")).toEqual(["slide"]);
  });

  // The user-facing claim of #425's asset-agnostic slice: these are the poses
  // that used to render as a plain jog and now have a distinct silhouette.
  it("animates the poses that previously fell through to idle/gait", () => {
    expect(animatedPoseIds()).toEqual([
      "keeper_grab",
      "keeper_throw",
      "keeper_punt",
      "keeper_set",
      "keeper_ready_low",
      "combat_guard",
      "combat_active",
      "combat_windup",
      "combat_aim",
      "combat_recovery",
      "soccer_windup",
      "tackle",
    ]);
  });

  it("names a phase source consistent with the action it selects", () => {
    for (const id of WIRE_POSE_IDS) {
      const entry = POSE_ACTIONS[id];
      if (entry.action !== null) {
        expect(entry.phase, `${id}`).toBe(PHASE_BY_ACTION[entry.action]);
      }
    }
  });

  it("selects a one-shot loop mode exactly for the two one-shot clips", () => {
    const oneShot = new Set(
      [clips.SWING, clips.KEEPER_SLING].filter((clip) => !clip.loop).map((clip) => clip.name),
    );
    expect(oneShot).toEqual(new Set(["swing", "keeper_sling"]));
    for (const id of WIRE_POSE_IDS) {
      const entry = POSE_ACTIONS[id];
      if (entry.action !== null) {
        expect(entry.loop === "clamp", `${id}`).toBe(oneShot.has(entry.action));
      }
    }
  });

  it("resolves an unknown or missing pose id to locomotion's own entry, not to an accident", () => {
    expect(actionForPose(undefined)).toBe(POSE_ACTIONS.locomotion);
    expect(actionForPose("not_a_pose_id")).toBe(POSE_ACTIONS.locomotion);
    expect(actionForPose("locomotion").fallback).toBe("locomotion_base");
  });
});

describe("rig3d/pose_table.locomotionBlend", () => {
  it("mirrors view_state.ts's own speed thresholds and strides", () => {
    // `pose_table.ts` declares these locally to stay free of view_state.ts's
    // mutable module state; this is what keeps the two copies equal.
    expect(locomotionBlend(0)).toEqual({ idle: 1, walk: 0, run: 0 });
    expect(locomotionBlend(viewState.WALK_SPEED)).toEqual({ idle: 0, walk: 1, run: 0 });
    expect(locomotionBlend(viewState.RUN_SPEED)).toEqual({ idle: 0, walk: 0, run: 1 });
    expect(strideFor(0)).toBe(viewState.WALK_STRIDE);
    expect(strideFor(viewState.RUN_SPEED)).toBe(viewState.RUN_STRIDE);
  });

  it("sums to exactly 1 at every speed, so the mixer never blends toward rest", () => {
    for (let speed = 0; speed <= viewState.MAX_DISPLAY_SPEED; speed += 7) {
      const blend = locomotionBlend(speed);
      expect(blend.idle + blend.walk + blend.run).toBeCloseTo(1, 12);
    }
  });

  // The weights exist to reproduce `poseFor`'s two nested `clips.layer` calls
  // once three.js accumulates them at `weight / cumulativeWeight`; these are
  // the two inner fractions that has to produce.
  it("reproduces poseFor's walkMix/runMix as three.js's cumulative-weight fractions", () => {
    for (let speed = 0; speed <= viewState.MAX_DISPLAY_SPEED; speed += 13) {
      const walkMix = Math.min(speed / viewState.WALK_SPEED, 1);
      const runMix = Math.max(0, Math.min((speed - viewState.WALK_SPEED) / (viewState.RUN_SPEED - viewState.WALK_SPEED), 1));
      const { idle, walk, run } = locomotionBlend(speed);
      if (idle + walk > 0) {
        expect(walk / (idle + walk)).toBeCloseTo(walkMix, 9);
      }
      expect(run / (idle + walk + run)).toBeCloseTo(runMix, 9);
    }
  });

  // Below WALK_SPEED the run weight is zero and above it the idle weight is,
  // so the blend is only ever two-way. That is what makes the nested-slerp
  // equivalence above EXACT rather than approximate -- a genuine three-way
  // quaternion blend would not be associative.
  //
  // NOT A STRUCTURAL GUARANTEE. It holds only while `RUN_SPEED > WALK_SPEED >
  // 0`, which nothing in the type system or the arithmetic enforces: reorder
  // the two thresholds and `runMix` starts leaving zero before `walkMix`
  // saturates, all three weights go positive together, and the exactness the
  // parity suite relies on quietly becomes an approximation. This test is the
  // only thing standing between that edit and a silent accuracy loss, so it
  // asserts the precondition explicitly before sweeping the consequence.
  it("is never a genuine three-way blend", () => {
    // The precondition the two-way property depends on, stated rather than
    // assumed -- see this test's comment.
    expect(viewState.WALK_SPEED).toBeGreaterThan(0);
    expect(viewState.RUN_SPEED).toBeGreaterThan(viewState.WALK_SPEED);
    for (let speed = 0; speed <= viewState.MAX_DISPLAY_SPEED; speed += 3) {
      const { idle, walk, run } = locomotionBlend(speed);
      const nonZero = [idle, walk, run].filter((w) => w > 0);
      expect(nonZero.length, `speed ${speed}`).toBeLessThanOrEqual(2);
    }
  });
});

describe("rig3d/pose_table.locomotionTimeScale", () => {
  // The animator pins `.time` to `gait * duration` instead of integrating a
  // clock; `locomotionTimeScale` must therefore be exactly the derivative of
  // that pinning, or an action's own `timeScale` would fight the anchor.
  it("is the exact derivative of the gait-driven phase it anchors against", () => {
    const dt = 1 / 240;
    for (const speed of [0, 40, 150, 260, 400, 480]) {
      const gaitStep = (speed * dt) / strideFor(speed);
      for (const duration of [clips.WALK.duration, clips.RUN.duration]) {
        const phaseStep = gaitStep * duration;
        expect(locomotionTimeScale(speed, duration) * dt).toBeCloseTo(phaseStep, 12);
      }
    }
  });

  it("is zero at a standstill, so a stationary character's feet do not slide", () => {
    expect(locomotionTimeScale(0, clips.WALK.duration)).toBe(0);
  });
});

describe("rig3d/pose_table.advanceCrossfade", () => {
  it("reaches full weight after exactly the stated crossfade, in equal linear steps", () => {
    let state = newCrossfade();
    const crossfade = 0.2;
    const dt = 0.05;
    const seen: number[] = [];
    for (let i = 0; i < 4; i += 1) {
      state = advanceCrossfade(state, "guard_stance", dt, crossfade);
      seen.push(state.weights["guard_stance"] ?? 0);
    }
    expect(seen).toEqual([0.25, 0.5, 0.75, 1]);
  });

  it("keeps a two-action swap summing to exactly 1, so the composite never dips or overshoots", () => {
    let state = snapCrossfade("guard_stance");
    for (let i = 0; i < 3; i += 1) {
      state = advanceCrossfade(state, "charge", 0.04, 0.16);
      expect(crossfadeTotal(state)).toBeCloseTo(1, 12);
    }
  });

  it("drops an action once it has finished fading out, rather than keeping a zero forever", () => {
    let state = snapCrossfade("charge");
    state = advanceCrossfade(state, null, 1, 0.16);
    expect(state.weights).toEqual({});
    expect(crossfadeTotal(state)).toBe(0);
  });

  it("snaps when the crossfade duration is zero or the frame is longer than it", () => {
    expect(advanceCrossfade(newCrossfade(), "swing", 0.016, 0).weights).toEqual({ swing: 1 });
    expect(advanceCrossfade(newCrossfade(), "swing", 1, 0.06).weights).toEqual({ swing: 1 });
  });

  it("never runs backwards on a non-monotonic or zero frame delta", () => {
    const start = advanceCrossfade(newCrossfade(), "guard_stance", 0.08, 0.16);
    expect(advanceCrossfade(start, "guard_stance", -5, 0.16)).toEqual(start);
    expect(advanceCrossfade(start, "guard_stance", 0, 0.16)).toEqual(start);
  });
});
