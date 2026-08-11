// Tier-1 tests for the knee bend (#445).
//
// The claim this module makes is a geometric one -- "the pelvis goes down by
// the authored drop and the soles stay where they are" -- so most of what is
// below is measured against the REAL skeleton rather than against the closed
// form that produced it. `ground_contact.spec.ts` makes the same measurement
// through the real character MESH, which is the one a player sees; this file
// isolates the joints, so a failure here says the derivation is wrong and a
// failure there says the geometry riding it is.

import { describe, expect, it } from "vitest";
import * as actionPose from "./action_pose.ts";
import * as clips from "./clips.ts";
import * as crouch from "./crouch.ts";
import { RIG_MEDIUM } from "./proportions.ts";
import * as skeleton from "./skeleton.ts";

const RIG = RIG_MEDIUM;
const MM = 0.001;
const LEG_REACH = RIG.seg.upperleg + RIG.seg.lowerleg;

/** Every pose id that authors a crouch, deepest first. */
const CROUCHES = ["keeper_ready_low", "settle", "contain", "keeper_set", "fatigue"] as const;

function drop(id: string): number {
  return actionPose.attitudeDrop({ pose: { id } });
}

function degreesOf(pose: skeleton.Pose, bone: string): number {
  const q = pose.rot[bone];
  if (q === undefined) {
    throw new Error(`crouch.spec.ts: no rotation on ${bone}`);
  }
  // Every rotation here is about x alone, so the half-angle is in the x
  // component and the sign comes with it.
  return (2 * Math.atan2(q[0], q[3]) * 180) / Math.PI;
}

describe("rig3d/crouch", () => {
  it("authors a crouch for exactly the five ATTITUDES entries that ask for one", () => {
    for (const id of CROUCHES) {
      expect(drop(id), `${id} authors a crouch`).toBeGreaterThan(30 * MM);
      expect(crouch.poseFor(drop(id)), `${id} gets a fold`).not.toBeNull();
    }
    for (const id of ["locomotion", "kick_follow", "run_telegraph", "keeper_dive"]) {
      expect(drop(id), `${id} authors none`).toBe(0);
      expect(crouch.poseFor(drop(id)), `${id} gets no fold`).toBeNull();
    }
  });

  // THE INVERSION, checked against the forward form rather than against a
  // table of angles. `angleFor` exists to answer "what fold raises the foot by
  // this much"; the forward form is `L * (1 - cos c)`, and the only way the two
  // disagree is if the `move`-units-to-world conversion is dropped or applied
  // twice -- which is #436's trap and the one thing this file is really for.
  it("returns the fold whose foot rise is the drop, in world metres", () => {
    for (const id of CROUCHES) {
      const c = (crouch.angleFor(drop(id)) * Math.PI) / 180;
      expect(LEG_REACH * (1 - Math.cos(c)), `${id}`).toBeCloseTo(drop(id) * RIG.motion_scale, 12);
    }
  });

  it("folds deeper for a deeper drop, and not at all for none", () => {
    const angles = CROUCHES.map((id) => crouch.angleFor(drop(id)));
    for (let i = 1; i < angles.length; i += 1) {
      expect(angles[i], `${CROUCHES[i]} is shallower than ${CROUCHES[i - 1]}`).toBeLessThan(
        angles[i - 1] ?? 0,
      );
    }
    expect(crouch.angleFor(0), "no drop, no fold").toBe(0);
    // The deepest crouch in the game, so a reader knows the scale of it: about
    // 27 degrees of thigh, which is 55 of knee.
    expect(angles[0]).toBeGreaterThan(25);
    expect(angles[0]).toBeLessThan(30);
  });

  // THE SIGNS, which are the easiest thing to get backwards on this rig and
  // the reason `clips.ts` opens with a note about them. Limbs hang DOWN, so
  // negative x swings them FORWARD; a knee only bends backward, so the shin's
  // angle is positive. SWING's lunge keys are the precedent
  // (`thigh.R: [20,...]`, `shin.R: [28,...]`).
  it("swings the thigh forward, the knee backward, and the ankle forward again", () => {
    const pose = crouch.poseFor(drop("keeper_ready_low"));
    if (pose === null) {
      throw new Error("crouch.spec.ts: the deepest attitude must produce a fold");
    }
    const c = crouch.angleFor(drop("keeper_ready_low"));
    for (const side of ["L", "R"]) {
      expect(degreesOf(pose, `thigh.${side}`), "thigh forward").toBeCloseTo(-c, 9);
      expect(degreesOf(pose, `shin.${side}`), "knee backward, twice as far").toBeCloseTo(2 * c, 9);
      expect(degreesOf(pose, `foot.${side}`), "ankle forward again").toBeCloseTo(-c, 9);
      // The three sum to zero, which is what keeps the sole parallel to the
      // pitch instead of digging a toe or a heel in.
      expect(
        degreesOf(pose, `thigh.${side}`) +
          degreesOf(pose, `shin.${side}`) +
          degreesOf(pose, `foot.${side}`),
        "the sole stays flat",
      ).toBeCloseTo(0, 9);
    }
    expect(pose.move["root"]?.[1], "and the root spends exactly the drop").toBeCloseTo(
      -drop("keeper_ready_low"),
      12,
    );
    expect(Object.keys(pose.rot).sort().concat("root").sort(), "the bones it names").toEqual(
      [...crouch.BONES].sort(),
    );
  });

  // MEASURED THROUGH THE REAL SKELETON, which is the only thing that makes the
  // closed form above trustworthy: `skeleton.apply` composes the pose with each
  // bone's REST rotation, and the thigh carries a 2 degree roll the derivation
  // does not model.
  it("lowers the hips by the drop and leaves the ankles where they were", () => {
    const rest = skeleton.newRig(RIG);
    const restHips = skeleton.jointPosition(rest, "hips")[1];
    const restFoot = skeleton.jointPosition(rest, "foot.R")[1];

    for (const id of CROUCHES) {
      const pose = crouch.poseFor(drop(id));
      if (pose === null) {
        throw new Error(`crouch.spec.ts: ${id} must produce a fold`);
      }
      const rig = skeleton.newRig(RIG);
      skeleton.apply(rig, pose);
      const world = drop(id) * RIG.motion_scale;
      expect(
        restHips - skeleton.jointPosition(rig, "hips")[1],
        `${id}: the pelvis settles`,
      ).toBeCloseTo(world, 12);
      const ankle = skeleton.jointPosition(rig, "foot.R")[1] - restFoot;
      // Within a tenth of a millimetre of where it started, in either
      // direction: what is left is the 2 degree thigh roll the closed form does
      // not model, 0.06% of the drop.
      //
      // A BAND RATHER THAN A FLOOR, deliberately. The SOLE -- which is what
      // gets rendered and what a player sees meet the turf -- lands ABOVE its
      // rest height for every one of these, and `ground_contact.spec.ts` pins
      // that direction on the real mesh. The ankle JOINT is a different point
      // on a foot whose frame the same roll tilts, so its sign is not the
      // sole's and pretending otherwise here would be pinning a coincidence.
      expect(Math.abs(ankle), `${id}: the ankle stays put`).toBeLessThan(0.2 * MM);
    }
  });

  // The feet travel FORWARD, because `upperleg` and `lowerleg` differ. Stated
  // as a pin rather than left in a comment: it is knees-over-toes, it is what a
  // braced crouch looks like, and if it ever grew to something a reader would
  // call a step then the symmetric fold has stopped being the right shape.
  it("puts the knees over the toes, by centimetres rather than by a stride", () => {
    const rest = skeleton.newRig(RIG);
    const restZ = skeleton.jointPosition(rest, "foot.R")[2];
    const pose = crouch.poseFor(drop("keeper_ready_low"));
    if (pose === null) {
      throw new Error("crouch.spec.ts: the deepest attitude must produce a fold");
    }
    const rig = skeleton.newRig(RIG);
    skeleton.apply(rig, pose);
    const forward = skeleton.jointPosition(rig, "foot.R")[2] - restZ;
    expect(forward, "the character faces +z, and the ankle goes that way").toBeGreaterThan(20 * MM);
    expect(forward, "by rather less than a foot's length").toBeLessThan(RIG.form.foot_len);
  });

  // COMPOSED ONTO A STRIDE, NOT INSTEAD OF ONE. This is the property that
  // decides the whole mechanism: `clips.layer` would have had to own the leg
  // bones to write any of them, which sends the ones it does not mention to
  // rest and deletes the gait. `clips.compose` adds, so a running character
  // keeps every degree of their stride and gains the fold on top.
  it("adds to whatever the gait resolved rather than replacing it", () => {
    const striding = clips.sample(clips.RUN, 0.2);
    const pose = crouch.poseFor(drop("settle"));
    if (pose === null) {
      throw new Error("crouch.spec.ts: settle must produce a fold");
    }
    const c = crouch.angleFor(drop("settle"));
    const composed = clips.compose(striding, pose);

    // Exactly additive in the authored angle: every leg key in the locomotion
    // clips is about x and z only, and `quat.fromEuler` puts x outermost, so
    // pre-multiplying an x rotation adds to the x angle and leaves z alone.
    expect(degreesOf(composed, "thigh.R"), "the stride's thigh, folded further").toBeCloseTo(
      degreesOf(striding, "thigh.R") - c,
      9,
    );
    expect(degreesOf(composed, "shin.R"), "the stride's knee, bent further").toBeCloseTo(
      degreesOf(striding, "shin.R") + 2 * c,
      9,
    );
    // Non-vacuous: the stride really is doing something for the fold to be
    // added to, rather than this being a fold composed onto nothing.
    expect(
      Math.abs(degreesOf(striding, "thigh.R")),
      "the run really swings the leg",
    ).toBeGreaterThan(5);

    // And the bones the crouch says nothing about are the stride's, untouched.
    for (const bone of ["spine", "chest", "upper_arm.R", "forearm.L"]) {
      expect(composed.rot[bone], bone).toBe(striding.rot[bone]);
    }
    // The gait's own vertical bob survives, added to rather than overwritten.
    expect(composed.move["root"]?.[1] ?? 0, "the bob and the drop, summed").toBeCloseTo(
      (striding.move["root"]?.[1] ?? 0) - drop("settle"),
      12,
    );
  });
});
