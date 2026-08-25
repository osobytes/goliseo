// Tier-2 tests for the ball in a keeper's hands: that there IS one, that it
// is the same object as the ball on the grass, and that the gather pose closes
// around it.
//
// WHY THIS IS A MEASUREMENT AND NOT AN INSPECTION, the same argument
// `rig3d/foot_contact.spec.ts` makes about the stride: "the hands hold the
// ball" is invisible in the authored keyframes -- they are angles in
// `rig3d/clips.ts`, and the ball is a socket offset in `rig3d/skeleton.ts` and
// a radius in `held_ball.ts`. Nothing compares the three. So everything below
// runs the real pose path (`animator.poseFor` -> `skeleton.apply` ->
// `skeleton.jointPosition`) and measures where the fists ended up relative to
// where the ball is drawn.
//
// It has teeth, demonstrated rather than claimed: the last test re-measures
// the angles this clip carried BEFORE the ball was restored and shows they
// fail the same predicate. A pose test that passes on any pose is decoration.
//
// Headless: that path is pure arithmetic, no GL and no window.

import { describe, expect, it } from "vitest";
import { quat } from "@gc/core";
import * as heldBall from "./held_ball.ts";
import { DEFAULT_PLAYER_RADIUS, metresPerWorldUnit, ppmForRadius } from "./player_renderer_3d.ts";
import * as animator from "./rig3d/animator.ts";
import * as clips from "./rig3d/clips.ts";
import * as masks from "./rig3d/masks.ts";
import * as proportions from "./rig3d/proportions.ts";
import * as skeleton from "./rig3d/skeleton.ts";
import type { PlayerRenderOptions } from "./player_render_options.ts";
import type { PlayerView } from "./view_state.ts";

const RIG = proportions.RIG_MEDIUM;
// Everything here goes through the shipping functions rather than restating
// their algebra: `proportions.height` for the rig, `metresPerWorldUnit` for the
// conversion, `heldBall.radiusMetres` for the ball. This file is a sibling of
// the renderer, so it may import them -- the "rig3d points only upward" rule
// `rig3d/foot_contact.spec.ts` works around does not apply here, and a spec
// that re-derives what it is checking cannot catch a drift in it.
const HEIGHT = proportions.height(RIG);
const BALL_R = heldBall.radiusMetres(metresPerWorldUnit(HEIGHT));
const HAND_R = RIG.form.hand_r;

const STILL: PlayerView = { px: 0, py: 0, speed: 0, phase: 0, gait: 0, lean: 0 };

interface Grip {
  /** Fist centre to ball centre, metres, per hand. */
  readonly reach: readonly [number, number];
  /** Fist-to-fist separation across the ball, metres. */
  readonly span: number;
  /** How far each fist sits to its own side of the ball's centre, metres. */
  readonly straddle: readonly [number, number];
  /** Fist height above the ball's centre, metres (negative is below). */
  readonly above: readonly [number, number];
}

type Point = readonly [number, number, number];

function dist(a: Point, b: Point): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

// `socket_hand.L/R` rather than `hand.L/R`: the fist MESH is built around the
// grip socket (`rig3d/body.ts` centres it within 6 mm of it), while the `hand`
// bone is the wrist. What has to be on the ball is the fist.
function gripOf(pose: skeleton.Pose): Grip {
  const rig = skeleton.newRig(RIG);
  skeleton.apply(rig, pose);
  const ball = skeleton.jointPosition(rig, heldBall.SOCKET);
  const l = skeleton.jointPosition(rig, "socket_hand.L");
  const r = skeleton.jointPosition(rig, "socket_hand.R");
  return {
    reach: [dist(l, ball), dist(r, ball)],
    span: dist(l, r),
    // The character's own left is +X (`skeleton.ts`'s orientation note).
    straddle: [l[0] - ball[0], ball[0] - r[0]],
    above: [l[1] - ball[1], r[1] - ball[1]],
  };
}

function gatherPose(now: number, overrides: Partial<PlayerRenderOptions> = {}): skeleton.Pose {
  const opts: PlayerRenderOptions = {
    is_keeper: true,
    controlled: false,
    holding: true,
    ...overrides,
  };
  // A fresh id per sample: `animator.poseFor` keeps per-character crossfade
  // state, and a measurement should not depend on what this file asked for
  // last.
  return animator.poseFor(`keeper-${String(now)}-${JSON.stringify(overrides)}`, STILL, opts, now);
}

// The claim, as one predicate so the red demonstration below can reuse it:
// both fists are ON the ball (their surfaces overlap it rather than merely
// reaching toward it), they are on OPPOSITE sides of it, and neither is above
// its middle. A cradle, not a juggle.
function holdsTheBall(grip: Grip): boolean {
  const touching = BALL_R + HAND_R;
  return (
    grip.reach.every((d) => d < touching) &&
    grip.straddle.every((d) => d > BALL_R * 0.5) &&
    grip.span > BALL_R * 2 &&
    grip.above.every((d) => d < BALL_R * 0.5)
  );
}

describe("held_ball: the ball is the ball on the grass", () => {
  // pitch.ts draws the loose ball at `heldBall.RADIUS * scale` pixels and this
  // ball at `radiusMetres` metres under a `ppmForRadius(r)` wrapper, with
  // `r = PLAYER_RADIUS * scale`. Both `scale` and the rig's height cancel, so
  // the two are the same number of pixels at any depth -- which is the whole
  // reason a keeper gathering the ball is not supposed to change its size.
  it("draws at exactly the loose ball's on-screen radius, at any projection scale", () => {
    for (const scale of [0.25, 1, 2.5, 7]) {
      // `r = PLAYER_RADIUS * scale` is pitch.ts's own `playerAnchor`, and
      // `ppmForRadius` is the pixels-per-metre pitch.ts scales the character
      // wrapper by -- the real functions, not a copy of their algebra.
      const ppm = ppmForRadius(DEFAULT_PLAYER_RADIUS * scale);
      expect(ppm, "the rigged pass must be available in this environment").toBeDefined();
      expect((ppm ?? 0) * BALL_R).toBeCloseTo(heldBall.RADIUS * scale, 10);
    }
  });

  it("is a plausible football at the rig's own scale", () => {
    // 0.22 m across, against a 1.57 m figure. Not a tuning knob -- a check
    // that `RADIUS`, the rig proportions and the world-unit conversion have
    // not drifted into describing a beach ball or a marble.
    expect(BALL_R * 2).toBeGreaterThan(0.19);
    expect(BALL_R * 2).toBeLessThan(0.26);
  });
});

describe("held_ball: the gather pose closes around it", () => {
  it("puts both fists on the ball, on opposite sides of it, throughout the carry", () => {
    // Across the whole clip, not one frame of it: the carry breathes, and a
    // hold that only reads at t = 0 is not a hold.
    for (let i = 0; i < 12; i += 1) {
      const now = (i / 12) * clips.KEEPER_GATHER.duration;
      const grip = gripOf(gatherPose(now));
      expect(holdsTheBall(grip), `t=${now.toFixed(2)}: ${JSON.stringify(grip)}`).toBe(true);
    }
  });

  it("holds it the same way when the pose id drives the clip instead of possession", () => {
    // `keeper_grab` reaches `keeper_gather` through `pose_table.ts`'s stance
    // layer as well as through the possession layer (see `animator.ts`), and
    // a keeper that catches the ball has both. The ball must be held on
    // either route.
    const grip = gripOf(gatherPose(0.4, { pose: { id: "keeper_grab" } }));
    expect(holdsTheBall(grip), JSON.stringify(grip)).toBe(true);
  });

  it("holds it symmetrically -- neither fist takes the ball for itself", () => {
    const grip = gripOf(gatherPose(0));
    expect(grip.reach[0]).toBeCloseTo(grip.reach[1], 3);
  });

  it("lets go when the keeper is not holding", () => {
    // The same predicate, on a keeper with no ball: an idle keeper whose arms
    // happened to satisfy it would make every test above vacuous.
    const empty: PlayerRenderOptions = { is_keeper: true, controlled: false };
    const idle = animator.poseFor("empty-handed", STILL, empty, 0);
    expect(holdsTheBall(gripOf(idle))).toBe(false);
  });

  // THE RED DEMONSTRATION (AGENTS.md §9). These are the angles `keeper_gather`
  // carried while nothing drew the ball at all. They are a perfectly readable
  // "arms out in front" pose and they fail the predicate above outright --
  // the fists land more than two ball diameters apart, which is what the
  // reported defect looked like once the ball was restored between them.
  it("would have failed on the angles authored before the ball was drawn", () => {
    const rot: Record<string, ReturnType<typeof quat.fromEuler>> = {};
    const deg = (x: number, y: number, z: number) =>
      quat.fromEuler((x * Math.PI) / 180, (y * Math.PI) / 180, (z * Math.PI) / 180);
    rot["spine"] = deg(6, 0, 0);
    rot["chest"] = deg(4, 0, 0);
    rot["head"] = deg(6, 0, 0);
    for (const side of ["L", "R"]) {
      rot[`upper_arm.${side}`] = deg(-72, 0, 2);
      rot[`forearm.${side}`] = deg(-58, 0, 0);
    }
    const before = clips.layer(clips.sample(clips.IDLE, 0), { rot, move: {} }, masks.UPPER_BODY, 1);
    const grip = gripOf(before);
    expect(holdsTheBall(grip)).toBe(false);
    expect(grip.span).toBeGreaterThan(BALL_R * 4);
  });
});
