// Tier-2 tests for the character animator: the composition of the three
// mixer layers and #445's crouch layer, the crossfade, and the user-facing claim #425's
// asset-agnostic slice makes -- that a set of poses which used to render as a
// plain jog now have their own silhouette.
//
// Headless (no GL, no window): the animator only ever produces a sparse
// `{rot, move}` pose. What is NOT verified here, and cannot be from a test
// runner, is whether each silhouette READS as the thing it is named after; see
// the PR's note on the visual pass this slice does not include.

import { beforeEach, describe, expect, it } from "vitest";
import type { Quat } from "@gc/core";
import * as actionPose from "./action_pose.ts";
import { poseFor, reset, basePose } from "./animator.ts";
import type { AnimatorOptions, AnimatorView } from "./animator.ts";
import { POSE_ACTIONS } from "./pose_table.ts";

const RUNNING: AnimatorView = { speed: 260, gait: 0.31 };
const STANDING: AnimatorView = { speed: 0, gait: 0 };

function opts(overrides: Partial<AnimatorOptions> = {}): AnimatorOptions {
  return overrides;
}

function withPose(id: string, overrides: Partial<AnimatorOptions> = {}): AnimatorOptions {
  return { pose: { id }, ...overrides };
}

// Largest absolute component difference across every bone either pose touches.
//
// ROTATION ONLY, deliberately. A quaternion component and a translation in
// metres are not commensurable, so folding both into one `worst` would make
// the 0.05 threshold below mean two different things depending on which bone
// won. `rootDrop` is the translation half, used where a pose's whole reading
// IS a translation (a crouch has no rotation at all).
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

// How much lower one pose puts the rig root than another, in metres. Positive
// means `b` is the more crouched of the two.
function rootDrop(a: actionPose.MutablePose, b: actionPose.MutablePose): number {
  return (a.move["root"]?.[1] ?? 0) - (b.move["root"]?.[1] ?? 0);
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

  // The other half of the explicit table: entries with no action AND no root
  // overlay must be indistinguishable from plain locomotion, not accidentally
  // different from it. Two are supposed to look like locomotion, and `slide`
  // is the one remaining gap -- a whole-body ground action no root transform
  // approximates, blocked on #423/#424 rather than merely unscheduled. The
  // point of the table is that the difference is written down.
  it.each(["keeper_shuffle", "keeper_ready_tall", "slide"])(
    "leaves %s on the locomotion base exactly as `locomotion` itself is",
    (id) => {
      expect(POSE_ACTIONS[id as keyof typeof POSE_ACTIONS].action).toBeNull();
      const plain = poseFor(freshId(), RUNNING, withPose("locomotion"), 4);
      const posed = poseFor(freshId(), RUNNING, withPose(id), 4);
      expect(delta(plain, posed)).toBe(0);
      expect(rootDrop(plain, posed)).toBe(0);
    },
  );

  // #430's headline, and the direct replacement for the five ids this suite
  // used to pin at `delta === 0`. Each of these is a posture the simulation
  // selects every match and that NO renderer has shown since #418 deleted the
  // billboard's `actionLean`/`stanceDrop` vocabulary.
  //
  // The threshold is not the 0.05 the stance tests above use, and that is a
  // statement about what an attitude IS rather than a weaker test. A stance
  // clip rotates a shoulder through tens of degrees; a whole-figure lean at
  // the billboard's own constants is a few degrees of ROOT tilt, which moves
  // every part of the body at once and reads out of proportion to its
  // quaternion magnitude. Inventing a bigger number to clear a threshold
  // borrowed from limb animation would be tuning by test. So this asserts the
  // three things that are actually claimed -- it is not zero, it is in the
  // right DIRECTION, and the poses stay ordered as the billboard had them --
  // and leaves "does it read" to the visual pass, which is the only honest
  // judge of it.
  const FORWARD = ["kick_follow", "run_telegraph"] as const;
  const BACKWARD = ["contain", "fatigue"] as const;

  // EACH COMPONENT ON ITS OWN, never their sum. `contain` and `fatigue` carry
  // both a lean and a crouch, and the crouch is the larger of the two -- so a
  // combined metric would let the crouch hold the assertion up while the lean
  // shrank to nothing, which is precisely the regression worth catching for
  // `fatigue` (see `action_pose.ts`'s note on it). Asserting the halves
  // separately means the smallest quantity in the table has to clear the floor
  // by itself; `action_pose.spec.ts` then pins its magnitude as an exact ratio
  // against `contain`, which a floor cannot do.
  it.each([...FORWARD, ...BACKWARD])("gives %s a body attitude instead of plain locomotion", (id) => {
    const plain = poseFor(freshId(), RUNNING, withPose("locomotion"), 4);
    const posed = poseFor(freshId(), RUNNING, withPose(id), 4);
    expect(delta(plain, posed), `${id}: lean`).toBeGreaterThan(0.01);
  });

  // THE CROUCH HALF, AND WHY IT ASSERTS THE WHOLE DROP AGAIN (#445).
  //
  // This assertion has been round the loop twice, and both turns belong in
  // front of a reader.
  //
  // It began as the same `it.each` as the lean above, asserting `rootDrop >
  // 0.01` for every id that crouches. It passed while those crouches drove the
  // character's ankles under the turf: the rig plants exactly on the pitch
  // plane and has no IK, so a root drop ON ITS OWN is penetration and nothing
  // else (`action_pose.ts`'s GROUND CONTACT section, and
  // `ground_contact.spec.ts`, which measures it on real geometry). #444 floored
  // the overlay's drop, and this became `toBe(0)` -- the honest pin for a rig
  // that could not absorb a crouch.
  //
  // #445 gave it one. `rig3d/crouch.ts` folds the thigh, knee and ankle so the
  // feet rise by the drop before it is spent, and writes the drop itself.
  // That translation is NOT floored, and does not need to be, for the same
  // reason `clips.SWING`'s own 32 mm root drop never was: its limbs pay for it
  // (see `ground_contact.spec.ts`'s "leaves a clip's own root motion alone").
  //
  // WHAT KEEPS THIS FROM BEING A ROUND TRIP BACK TO THE DEFECT is the second
  // half. A drop with no fold under it is #439 exactly, so the knee has to be
  // bent here too -- and `ground_contact.spec.ts` measures on the real geometry
  // that the two cancel at the sole to a fraction of a millimetre.
  it.each(["settle", "contain", "fatigue", "keeper_set", "keeper_ready_low"])(
    "gives %s its authored crouch, with a knee bend under it",
    (id) => {
      const authored = actionPose.attitudeDrop({ pose: { id } });
      expect(authored, `${id}: the table must still author a crouch`).toBeGreaterThan(0.03);
      const plain = poseFor(freshId(), RUNNING, withPose("locomotion"), 4);
      const posed = poseFor(freshId(), RUNNING, withPose(id), 4);
      expect(rootDrop(plain, posed), `${id}: the crouch reaches the root in full`).toBeCloseTo(authored, 12);
      // The limbs that pay for it. Negative x swings a thigh FORWARD and a knee
      // only bends backward, so the two move in opposite directions -- the sign
      // convention `clips.ts` documents and SWING's own lunge keys follow.
      for (const side of ["L", "R"]) {
        expect(posed.rot[`thigh.${side}`]?.[0] ?? 0, `${id}: thigh.${side} folds forward`).toBeLessThan(
          plain.rot[`thigh.${side}`]?.[0] ?? 0,
        );
        expect(posed.rot[`shin.${side}`]?.[0] ?? 0, `${id}: shin.${side} bends backward`).toBeGreaterThan(
          plain.rot[`shin.${side}`]?.[0] ?? 0,
        );
      }
    },
  );

  // A pose that authors no crouch must not acquire one, which is what stops
  // the layer above from being "every character is now slightly bent".
  it.each(["locomotion", "kick_follow", "run_telegraph", "keeper_ready_tall"])(
    "leaves %s standing at full height",
    (id) => {
      expect(actionPose.attitudeDrop({ pose: { id } }), `${id} authors no crouch`).toBe(0);
      const plain = poseFor(freshId(), RUNNING, withPose("locomotion"), 4);
      const posed = poseFor(freshId(), RUNNING, withPose(id), 4);
      expect(rootDrop(plain, posed), `${id}: nothing settles`).toBe(0);
      expect(posed.rot["thigh.L"], `${id}: no fold`).toEqual(plain.rot["thigh.L"]);
    },
  );

  // The crouch EASES in rather than snapping, because it is a held posture
  // rather than a committed action -- the same argument `pose_table.ts` makes
  // for a stance's crossfade. A character's FIRST observed frame still commits
  // outright, for the reason the stance crossfade does: easing every player
  // down at kickoff, and again after every rollback re-seed, would be a visible
  // tic bought with nothing.
  it("eases a crouch in over a stance's crossfade, after committing on the first frame", () => {
    const authored = actionPose.attitudeDrop({ pose: { id: "keeper_ready_low" } });
    // The comparison has to be taken at the SAME instant: the idle clip writes
    // its own weight shift to `move.root`, so a baseline sampled at t = 0 would
    // fold that shift into every later reading.
    const tall = (t: number) => poseFor(freshId(), STANDING, withPose("locomotion"), t);
    const first = freshId();
    expect(
      rootDrop(tall(0), poseFor(first, STANDING, withPose("keeper_ready_low"), 0)),
      "a character's first frame commits to its crouch",
    ).toBeCloseTo(authored, 12);

    // A character already standing tall, then asked to crouch: partway there
    // after one frame, all the way there once the fade has run.
    const second = freshId();
    poseFor(second, STANDING, withPose("locomotion"), 0);
    const partial = rootDrop(tall(0.02), poseFor(second, STANDING, withPose("keeper_ready_low"), 0.02));
    expect(partial, "not there yet").toBeLessThan(authored);
    expect(partial, "but on the way").toBeGreaterThan(0);
    // Stepped at 20 ms rather than jumped: `MAX_FRAME_DT` deliberately refuses
    // to integrate more than a 10 fps frame in one go, so a single leap to
    // t = 0.4 would measure the clamp instead of the ramp.
    let settled = 0;
    let t = 0.02;
    for (let i = 0; i < 20; i += 1) {
      t += 0.02;
      settled = rootDrop(tall(t), poseFor(second, STANDING, withPose("keeper_ready_low"), t));
    }
    expect(settled, "and fully settled once the fade has run").toBeCloseTo(authored, 12);
  });

  it("leans a follow-through and a telegraph forward, and a contain and a fatigue back", () => {
    const plain = poseFor(freshId(), RUNNING, withPose("locomotion"), 4);
    // Positive rot.x tips FORWARD onto the face (action_pose.ts's orientation
    // note), and the root quaternion's x component carries its sign.
    for (const id of FORWARD) {
      const posed = poseFor(freshId(), RUNNING, withPose(id), 4);
      expect(posed.rot["root"]?.[0] ?? 0, id).toBeGreaterThan(0);
    }
    for (const id of BACKWARD) {
      const posed = poseFor(freshId(), RUNNING, withPose(id), 4);
      expect(posed.rot["root"]?.[0] ?? 0, id).toBeLessThan(0);
    }
    expect(plain.rot["root"]).toBeUndefined();
  });

  it("keeps the billboard's ordering: a telegraph commits harder than a follow-through, a contain than a sag", () => {
    const at = (id: string) => Math.abs(poseFor(freshId(), RUNNING, withPose(id), 4).rot["root"]?.[0] ?? 0);
    expect(at("run_telegraph")).toBeGreaterThan(at("kick_follow"));
    expect(at("contain")).toBeGreaterThan(at("fatigue"));
  });

  // The ordering, asserted on the RESOLVED pose again (#445). Between #444 and
  // #445 it was unobservable there -- every crouch resolved to the same
  // grounded zero -- and was pinned on the authored table instead. It is a live
  // quantity once more, so both are asserted: the table orders the two, and the
  // rig shows the order. `action_pose.spec.ts` pins the full five-deep chain
  // and the exact fatigue/contain ratio.
  it.each([
    ["settle", "contain"],
    ["keeper_ready_low", "keeper_set"],
  ])("settles %s deeper than %s, on the table and on the rig", (deeper, shallower) => {
    const drop = (id: string) => -(actionPose.attitudeFor({ pose: { id } })?.move.root?.[1] ?? 0);
    expect(drop(deeper)).toBeGreaterThan(drop(shallower));
    expect(drop(shallower)).toBeGreaterThan(0);

    const tall = poseFor(freshId(), STANDING, withPose("locomotion"), 4);
    const resolved = (id: string) => rootDrop(tall, poseFor(freshId(), STANDING, withPose(id), 4));
    expect(resolved(deeper), `${deeper} renders lower than ${shallower}`).toBeGreaterThan(resolved(shallower));
  });

  // The attitude COMPOSES onto the run's vertical bob rather than replacing
  // it. The locomotion clips write that bob to `move.root`, so an attitude
  // that assigned there would flatten it and leave a settling player gliding
  // -- and so would a ground contact that floored the SUM instead of the
  // attitude's own contribution (#439). This is the test that tells those
  // apart: the bob survives a grounded crouch intact.
  it("keeps the run's vertical bob under a crouch", () => {
    const bobbing = poseFor(freshId(), { speed: 260, gait: 0.15 }, withPose("settle"), 4);
    const contact = poseFor(freshId(), { speed: 260, gait: 0 }, withPose("settle"), 4);
    expect(bobbing.move["root"]?.[1] ?? 0).toBeGreaterThan(contact.move["root"]?.[1] ?? 0);
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

describe("rig3d/animator.basePose", () => {
  it("changes with speed and with gait phase, independently", () => {
    const slow = basePose({ speed: 40, gait: 0.25 }, 0);
    const fast = basePose({ speed: 380, gait: 0.25 }, 0);
    const shifted = basePose({ speed: 380, gait: 0.75 }, 0);
    expect(delta(slow, fast)).toBeGreaterThan(0.05);
    expect(delta(fast, shifted)).toBeGreaterThan(0.05);
  });

  it("keeps the idle clip inside its own keyframes however long the clock has been running", () => {
    // `MixerLayer` advances by zero, so nothing wraps the pinned phase for it.
    // An unwrapped `now * IDLE_RATE` would walk past the idle clip's last key
    // after nine seconds and freeze there for the rest of the match.
    const early = basePose({ speed: 0, gait: 0 }, 1.0);
    const late = basePose({ speed: 0, gait: 0 }, 1.0 + 3.4 / 0.35);
    expect(delta(early, late)).toBeLessThan(1e-6);
    const midCycle = basePose({ speed: 0, gait: 0 }, 1.0 + 3.4 / 0.35 / 2);
    expect(delta(early, midCycle)).toBeGreaterThan(0.005);
  });
});
