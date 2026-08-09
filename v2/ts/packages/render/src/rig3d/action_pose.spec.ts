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

// #436: the translations leave this file in METRES, not in rig heights.
//
// Signs and orderings were pinned above from the day the poses landed, and all
// of them passed while every dive, aerial and reaction travelled 64% of its
// authored distance -- the four translation sites divided by `RADII_PER_HEIGHT`
// (6, radii per rig height) where the metres `skeleton.apply` adds onto bone
// offsets need `* RIG_HEIGHT_METRES / 6`. A relative pin cannot catch that,
// because scaling every distance by one factor preserves every ordering. So
// these are ABSOLUTE, and they are the only tests here that are.
//
// Each assertion is made twice on purpose. `metres(k)` ties the pose to the
// file's single conversion, which fails if a call site stops using it; the
// literal metre value ties the conversion itself to the rig, which fails if
// `metres` is redefined so the first check passes vacuously. The short value
// each pose used to produce is written down next to it, so a revert reads as a
// named regression rather than as an unexplained number.
describe("root translations in metres", () => {
  // 1.5676 m / 6 radii. Written out rather than imported so that a change to
  // the rig's proportions is a deliberate edit here, not a silent re-scale.
  const METRE_PER_RADIUS = 0.26126;

  it("crosses from radii to metres through the rig's height, not through 6", () => {
    expect(actionPose.metres(1)).toBeCloseTo(METRE_PER_RADIUS, 4);
    // The rig is 6 radii tall by construction, so this round-trips to its
    // standing height. If this reads 1.0 the conversion is back to rig heights.
    expect(actionPose.metres(6)).toBeCloseTo(1.5676, 3);
  });

  it("throws a keeper the full 1.6r of their dive", () => {
    const pose = actionPose.forOptions(opts("keeper_dive", { dive: 1, dive_dir: LEFT }));
    const travel = pose?.move.root?.[0] ?? Number.NaN;
    expect(travel).toBeCloseTo(actionPose.metres(1.6), 12);
    // 0.418 m, not the 0.267 m (1.6 / 6) this moved before #436.
    expect(travel).toBeCloseTo(0.418, 3);
  });

  it("reaches every save family in metres, keeper_tip the furthest", () => {
    const travel = (id: string) =>
      actionPose.forOptions(opts(id, { dive: 1, dive_dir: LEFT }))?.move.root?.[0] ?? Number.NaN;
    // SAVES' own radii: spread 0.65, central 0.95, stretch 1.9, tip 2.2.
    expect(travel("keeper_spread")).toBeCloseTo(actionPose.metres(0.65), 12);
    expect(travel("keeper_central")).toBeCloseTo(actionPose.metres(0.95), 12);
    expect(travel("keeper_stretch")).toBeCloseTo(actionPose.metres(1.9), 12);
    expect(travel("keeper_tip")).toBeCloseTo(actionPose.metres(2.2), 12);
    expect(travel("keeper_tip")).toBeCloseTo(0.5748, 4);
  });

  it("lifts a bicycle kick 2r off the turf, which is a third of the figure", () => {
    const pose = actionPose.forOptions(opts("aerial_bicycle", { aerial: 1, aerial_style: "bicycle", aerial_jump: 1 }));
    const lift = pose?.move.root?.[1] ?? Number.NaN;
    // 0.35 + 1.65 * jump, at jump = 1.
    expect(lift).toBeCloseTo(actionPose.metres(2.0), 12);
    // 0.523 m, not the 0.333 m this rose before #436. Before the rig's own
    // motion_scale (0.88), which `skeleton.apply` applies afterwards.
    expect(lift).toBeCloseTo(0.5225, 4);
  });

  it("drives a knockback off the feet and settles a stagger, both in metres", () => {
    const lift = actionPose.forOptions(opts("combat_knockback"))?.move.root?.[1] ?? Number.NaN;
    const settle = actionPose.forOptions(opts("combat_stagger"))?.move.root?.[1] ?? Number.NaN;
    expect(lift).toBeCloseTo(actionPose.metres(0.45), 12);
    expect(lift).toBeCloseTo(0.1176, 4);
    expect(settle).toBeCloseTo(actionPose.metres(-0.28), 12);
  });

  it("drops a stumble back behind the challenge in metres", () => {
    const pose = actionPose.forOptions(opts("stumble"));
    expect(pose?.move.root?.[1] ?? Number.NaN).toBeCloseTo(actionPose.metres(0.12), 12);
    expect(pose?.move.root?.[2] ?? Number.NaN).toBeCloseTo(actionPose.metres(-0.35), 12);
    expect(pose?.move.root?.[2] ?? Number.NaN).toBeCloseTo(-0.0914, 4);
  });

  it("gets a keeper up off the ground they landed on, in metres", () => {
    const pose = actionPose.forOptions(opts("keeper_get_up", { dive_dir: LEFT }));
    expect(pose?.move.root?.[1] ?? Number.NaN).toBeCloseTo(actionPose.metres(-0.18), 12);
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

  // EXACT RATIOS, not floors, and specifically for the reason `ATTITUDES`'
  // `fatigue` note records: fatigue's lean is the smallest quantity anything
  // here asserts (~2 degrees), so an ordering test passes on it trivially and
  // a magnitude floor it shares with a crouch lets the crouch carry it. These
  // are the pins that fail if somebody halves it.
  //
  // A ratio is the right shape because it is SCALE-FREE. Every lean here is
  // `asin(k * METRES_PER_RADIUS / LEAN_REFERENCE_HEIGHT)`, so `sin(theta)` is
  // exactly proportional to the billboard's `k` and the ratio of two poses'
  // sines is exactly the ratio of their constants -- independent of the rig's
  // height and of the reference point. So this survives an honest re-tune of
  // the mapping (which the file invites) and fails a per-pose magnitude drift
  // (which it does not). What a scale-free pin CANNOT see is the whole mapping
  // being wrong at once, which is exactly what #436 was; the absolute pins in
  // "root translations in metres" above are there for that half.
  it("keeps fatigue's magnitude tied to contain's, so a shrunk sag cannot pass on the crouch's coat-tails", () => {
    const sinLean = (id: string) => Math.sin(((actionPose.attitudeFor(opts(id))?.rot.root?.[0] ?? 0) * Math.PI) / 180);
    const drop = (id: string) => -(actionPose.attitudeFor(opts(id))?.move.root?.[1] ?? 0);
    // Billboard: fatigue -0.12r against contain -0.3r, and 0.16r against 0.26r.
    expect(sinLean("fatigue") / sinLean("contain")).toBeCloseTo(0.12 / 0.3, 12);
    expect(drop("fatigue") / drop("contain")).toBeCloseTo(0.16 / 0.26, 12);
    // ...and the same relation holds for the forward pair, so the ratio pin is
    // a property of the table rather than one hand-checked pose.
    expect(sinLean("kick_follow") / sinLean("run_telegraph")).toBeCloseTo(0.28 / 0.5, 12);
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
    // SYNTHETIC, and worth saying so plainly: `SAVES`/`TIPS` and `ATTITUDES`
    // have disjoint keys, so no pose id can reach a real conflict and this
    // cannot prove precedence under one. What it does prove is that the code
    // path exists and behaves -- `apply` returns after the action branch, so
    // the assigning semantics win and the composing ones never run. It becomes
    // a real test the day the two tables overlap; until then it is the
    // executable form of a documented rule, which beats a comment alone.
    const dive = opts("keeper_dive", { dive: 1, dive_dir: LEFT });
    const posed = actionPose.apply({ rot: {}, move: { root: [0, 0.05, 0] } }, dive);
    expect(posed.move["root"]).toEqual(actionPose.forOptions(dive)?.move.root);
  });

  it("leaves a plain run completely untouched", () => {
    const pose = actionPose.apply({ rot: {}, move: {} }, opts("locomotion"));
    expect(pose).toEqual({ rot: {}, move: {} });
  });
});
