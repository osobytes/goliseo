// The whole-body action overlay is pure geometry, so its sign conventions can
// be pinned without a window. That matters more here than usual: clips.ts
// records that an early pose leaned backwards because a sign was assumed
// rather than checked, and every value in action_pose.ts depends on the same
// orientation rules (+Y up, facing +Z, their own right at -X).

import { describe, expect, it } from "vitest";
import * as actionPose from "./action_pose.ts";
import type { ActionPoseOptions, XY } from "./action_pose.ts";

// Facing up the pitch (+y). With that facing the character's own LEFT is +X,
// which is pitch +x, so a dive toward +x is a dive to their left.
const FACING_UP: XY = { x: 0, y: 1 };
const LEFT: XY = { x: 1, y: 0 };
const RIGHT: XY = { x: -1, y: 0 };

function opts(id: string | undefined, extra?: Partial<ActionPoseOptions>): ActionPoseOptions {
  const out: ActionPoseOptions = { facing: FACING_UP, ...extra };
  return id ? { ...out, pose: { id } } : out;
}

describe("rigged whole-body action poses", () => {
  it("leaves an ordinary run to the locomotion blend", () => {
    expect(actionPose.forOptions(opts("locomotion"))).toBeNull();
    expect(actionPose.forOptions(opts(undefined))).toBeNull();
  });

  it("tips a keeper toward the side they dive to", () => {
    const left = actionPose.forOptions(opts("keeper_dive", { dive: 1, dive_dir: LEFT }));
    expect(left).not.toBeNull();
    if (!left) return;
    // Their left is +X, and the head (at +Y) only reaches +X on a NEGATIVE
    // z rotation. Getting this backwards throws the keeper the wrong way.
    expect(left.rot.root?.[2], "diving left must roll negative about z").toBeLessThan(0);
    expect(left.move.root?.[0], "diving left must travel toward +X").toBeGreaterThan(0);
  });

  it("mirrors the dive exactly", () => {
    const left = actionPose.forOptions(opts("keeper_dive", { dive: 1, dive_dir: LEFT }));
    const right = actionPose.forOptions(opts("keeper_dive", { dive: 1, dive_dir: RIGHT }));
    expect(left).not.toBeNull();
    expect(right).not.toBeNull();
    if (!left || !right) return;
    expect(left.rot.root?.[2]).toBe(-(right.rot.root?.[2] ?? Number.NaN));
    expect(left.move.root?.[0]).toBe(-(right.move.root?.[0] ?? Number.NaN));
  });

  it("keeps the save families distinguishable by commitment", () => {
    function reach(id: string, dive?: number): number {
      const pose = actionPose.forOptions(opts(id, { dive: dive ?? 1, dive_dir: LEFT }));
      expect(pose).not.toBeNull();
      return Math.abs(pose?.rot.root?.[2] ?? Number.NaN);
    }
    // Spread stays compact, central corrects, dive commits, stretch is the
    // full lunge and a tip reaches just past it. If any two of these swap,
    // two different keeper decisions start looking like the same save.
    expect(reach("keeper_spread")).toBeLessThan(reach("keeper_central"));
    expect(reach("keeper_central")).toBeLessThan(reach("keeper_dive"));
    expect(reach("keeper_dive")).toBeLessThan(reach("keeper_stretch"));
    expect(reach("keeper_stretch")).toBeLessThan(reach("keeper_tip"));
  });

  it("holds the full-stretch silhouette even early in the dive", () => {
    const early = actionPose.forOptions(opts("keeper_stretch", { dive: 0.05, dive_dir: LEFT }));
    const late = actionPose.forOptions(opts("keeper_stretch", { dive: 1, dive_dir: LEFT }));
    expect(early).not.toBeNull();
    expect(late).not.toBeNull();
    if (!early || !late) return;
    // Floored at 0.82: a stretch that eased in from nothing would read as a
    // spread for the first half of the save.
    const earlyAbs = Math.abs(early.rot.root?.[2] ?? 0);
    const lateAbs = Math.abs(late.rot.root?.[2] ?? 0);
    expect(earlyAbs).toBeGreaterThan(0.8 * lateAbs);
  });

  it("plays a tip at full reach regardless of the dive timer", () => {
    const a = actionPose.forOptions(opts("keeper_tip", { dive: 0, dive_dir: LEFT }));
    const b = actionPose.forOptions(opts("keeper_tip", { dive: 1, dive_dir: LEFT }));
    expect(a).not.toBeNull();
    expect(b).not.toBeNull();
    expect(a?.rot.root?.[2]).toBe(b?.rot.root?.[2]);
  });

  it("needs a direction before it will throw a keeper anywhere", () => {
    expect(actionPose.forOptions(opts("keeper_dive", { dive: 1 }))).toBeNull();
  });

  it("takes a bicycle kick over backwards and off the ground", () => {
    const pose = actionPose.forOptions(opts("aerial_bicycle", { aerial: 1, aerial_style: "bicycle", aerial_jump: 1 }));
    expect(pose).not.toBeNull();
    if (!pose) return;
    // Negative x pitches the head behind the hips. Positive would be a dive
    // onto the face, which is not a bicycle kick.
    expect(pose.rot.root?.[0], "a bicycle must rotate backwards").toBeLessThan(0);
    expect(pose.move.root?.[1], "a bicycle must leave the ground").toBeGreaterThan(0);
  });

  it("lifts a non-bicycle aerial without rotating it", () => {
    const pose = actionPose.forOptions(opts("aerial_action", { aerial: 1, aerial_style: "chest_control", aerial_jump: 1 }));
    expect(pose).not.toBeNull();
    if (!pose) return;
    expect(pose.rot.root).toBeUndefined();
    expect(pose.move.root?.[1]).toBeGreaterThan(0);
  });

  it("lifts further off a jump than off a standing reach", () => {
    function lift(jump: number): number {
      const pose = actionPose.forOptions(opts("aerial_action", { aerial: 1, aerial_style: "leg_control", aerial_jump: jump }));
      expect(pose).not.toBeNull();
      return pose?.move.root?.[1] ?? Number.NaN;
    }
    expect(lift(1)).toBeGreaterThan(lift(0));
  });

  it("separates a knockback from a stagger", () => {
    const knock = actionPose.forOptions(opts("combat_knockback"));
    const stagger = actionPose.forOptions(opts("combat_stagger"));
    expect(knock).not.toBeNull();
    expect(stagger).not.toBeNull();
    if (!knock || !stagger) return;
    // Both go backwards; the knockback is driven off the feet and the
    // stagger is a rocked-back beat, so they must not read as one thing.
    const knockX = knock.rot.root?.[0] ?? 0;
    const staggerX = stagger.rot.root?.[0] ?? 0;
    expect(knockX < 0 && staggerX < 0).toBe(true);
    expect(Math.abs(knockX)).toBeGreaterThan(Math.abs(staggerX));
    expect(knock.move.root?.[1], "a knockback leaves the ground").toBeGreaterThan(0);
    expect(stagger.move.root?.[1], "a stagger settles into it").toBeLessThan(0);
  });

  it("tips a stumble away from the committed direction", () => {
    const pose = actionPose.forOptions(opts("stumble"));
    expect(pose).not.toBeNull();
    if (!pose) return;
    expect(pose.rot.root?.[0]).toBeLessThan(0);
    expect(pose.move.root?.[2], "a stumble falls behind the challenge").toBeLessThan(0);
  });

  it("gets a keeper up on the side they landed on", () => {
    const left = actionPose.forOptions(opts("keeper_get_up", { dive_dir: LEFT }));
    const right = actionPose.forOptions(opts("keeper_get_up", { dive_dir: RIGHT }));
    expect(left).not.toBeNull();
    expect(right).not.toBeNull();
    expect(left?.rot.root?.[2]).toBe(-(right?.rot.root?.[2] ?? Number.NaN));
  });
});

// #430: the held body attitudes recovered from the 2.5D billboard deleted in
// #418. These are postures rather than actions, and the distinction is what
// most of this suite is about -- see `action_pose.ts`'s TWO KINDS OF ROOT
// TRANSFORM note. Degrees are not pinned: they fall out of a documented
// conversion from the billboard's own constants and re-tuning them by eye at
// match-camera scale is a legitimate follow-up, not a regression. Signs,
// ordering and the composition rules ARE pinned, because those are the things
// a future edit can silently get backwards.
describe("held body attitudes", () => {
  const FORWARD = ["kick_follow", "run_telegraph"];
  const BACKWARD = ["contain", "fatigue"];
  const CROUCHED = ["settle", "contain", "fatigue", "keeper_set", "keeper_ready_low"];

  it("is not an action: the balance-lean gate must keep seeing `null`", () => {
    for (const id of [...FORWARD, ...BACKWARD, ...CROUCHED]) {
      expect(actionPose.forOptions(opts(id)), id).toBeNull();
      expect(actionPose.attitudeFor(opts(id)), id).not.toBeNull();
    }
    expect(actionPose.attitudeFor(opts("locomotion"))).toBeNull();
    expect(actionPose.attitudeFor(opts(undefined))).toBeNull();
    // The gap that stays a gap: a slide is a whole-body ground action, not a
    // root transform, and guessing at one would be worse than showing none.
    expect(actionPose.attitudeFor(opts("slide"))).toBeNull();
  });

  it("leans a follow-through and a telegraph forward, and a contain and a sag back", () => {
    for (const id of FORWARD) {
      expect(actionPose.attitudeFor(opts(id))?.rot.root?.[0] ?? 0, id).toBeGreaterThan(0);
    }
    for (const id of BACKWARD) {
      expect(actionPose.attitudeFor(opts(id))?.rot.root?.[0] ?? 0, id).toBeLessThan(0);
    }
  });

  // The billboard wrote `actionLean = fx * r * k`, projecting "forward" onto
  // the SCREEN's x axis, so a player running straight up the pitch (fx = 0)
  // did not lean at all. The rig is already yawed to `facing`, so forward is
  // local +Z and the facing term does not survive the port.
  it("leans by the same amount whichever way the player is facing", () => {
    const up = actionPose.attitudeFor(opts("run_telegraph", { facing: { x: 0, y: 1 } }));
    const across = actionPose.attitudeFor(opts("run_telegraph", { facing: { x: 1, y: 0 } }));
    const none = actionPose.attitudeFor({ pose: { id: "run_telegraph" } });
    expect(up?.rot.root).toEqual(across?.rot.root);
    expect(up?.rot.root).toEqual(none?.rot.root);
  });

  it("crouches rather than rises, and keeps the billboard's ordering", () => {
    const drop = (id: string) => -(actionPose.attitudeFor(opts(id))?.move.root?.[1] ?? 0);
    for (const id of CROUCHED) {
      expect(drop(id), id).toBeGreaterThan(0);
    }
    // 0.32r > 0.3r > 0.26r > 0.18r > 0.16r, straight off the deleted renderer.
    expect(drop("keeper_ready_low")).toBeGreaterThan(drop("settle"));
    expect(drop("settle")).toBeGreaterThan(drop("contain"));
    expect(drop("contain")).toBeGreaterThan(drop("keeper_set"));
    expect(drop("keeper_set")).toBeGreaterThan(drop("fatigue"));
    // A follow-through is a lean and nothing else.
    expect(actionPose.attitudeFor(opts("kick_follow"))?.move.root).toBeUndefined();
  });

  it("keeps the lean ordering the billboard had: a telegraph commits harder than a follow-through", () => {
    const tilt = (id: string) => Math.abs(actionPose.attitudeFor(opts(id))?.rot.root?.[0] ?? 0);
    expect(tilt("run_telegraph")).toBeGreaterThan(tilt("kick_follow"));
    expect(tilt("contain")).toBeGreaterThan(tilt("fatigue"));
  });

  // COMPOSES, where an action ASSIGNS. The locomotion clips write the run's
  // vertical bob to `move.root`; a crouch that assigned there would flatten it
  // and leave a settling player gliding.
  it("adds its crouch to whatever the gait already resolved, instead of replacing it", () => {
    const bob = 0.05;
    const pose = actionPose.apply({ rot: {}, move: { root: [0, bob, 0] } }, opts("settle"));
    const drop = actionPose.attitudeFor(opts("settle"))?.move.root?.[1] ?? 0;
    expect(pose.move["root"]?.[1]).toBeCloseTo(bob + drop, 12);
    expect(drop).toBeLessThan(0);
  });

  it("lets an action win outright when a pose id would claim both", () => {
    // No pose id is in both tables today, so this pins the rule rather than a
    // live case: `apply` returns after the action branch.
    const dive = opts("keeper_dive", { dive: 1, dive_dir: LEFT });
    const posed = actionPose.apply({ rot: {}, move: { root: [0, 0.05, 0] } }, dive);
    expect(posed.move["root"]).toEqual(actionPose.forOptions(dive)?.move.root);
  });

  it("leaves a plain run completely untouched", () => {
    const pose = actionPose.apply({ rot: {}, move: {} }, opts("locomotion"));
    expect(pose).toEqual({ rot: {}, move: {} });
  });
});
