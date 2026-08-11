// Tier-2 tests for the character animator: the composition of the three
// mixer layers, the crossfade, and the user-facing claim #425's
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

  // THE CROUCH HALF, AND WHY IT NOW ASSERTS ZERO (#439).
  //
  // This used to be the same `it.each` as the lean above, `settle` included,
  // asserting `rootDrop > 0.01` for every id that crouches. It passed while
  // those crouches drove the character's ankles under the turf: the rig plants
  // exactly on the pitch plane and has no IK, so a root drop is penetration
  // and nothing else (`action_pose.ts`'s GROUND CONTACT section, and
  // `ground_contact.spec.ts`, which measures it on real geometry).
  // `action_pose.apply` now floors it, so the honest pin is that a crouch
  // reaches the resolved pose as ZERO.
  //
  // The coverage does not leave with the behaviour. The authored magnitudes
  // and their ordering are still pinned one layer down, on `attitudeFor` --
  // `action_pose.spec.ts`'s "crouches rather than rises, and keeps the
  // billboard's ordering" -- so shrinking or deleting a `drop` still fails.
  // What this asserts is the seam: the table still says crouch, and the rig
  // still stands on the pitch.
  it.each(["settle", "contain", "fatigue", "keeper_set", "keeper_ready_low"])(
    "grounds %s's crouch instead of sinking the character into the pitch",
    (id) => {
      const authored = actionPose.attitudeFor({ pose: { id } })?.move.root?.[1] ?? 0;
      expect(authored, `${id}: the table must still author a crouch`).toBeLessThan(0);
      const plain = poseFor(freshId(), RUNNING, withPose("locomotion"), 4);
      const posed = poseFor(freshId(), RUNNING, withPose(id), 4);
      expect(rootDrop(plain, posed), `${id}: the crouch must not reach the root`).toBe(0);
    },
  );

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

  // The ordering these two pairs used to assert on the RESOLVED pose is now
  // unobservable there -- every crouch resolves to the same grounded zero --
  // so it is asserted where it is still a live quantity, on the authored
  // table. Same claim, same pairs, one layer down; `action_pose.spec.ts` pins
  // the full five-deep chain and the exact fatigue/contain ratio.
  it.each([
    ["settle", "contain"],
    ["keeper_ready_low", "keeper_set"],
  ])("still authors %s as a deeper crouch than %s", (deeper, shallower) => {
    const drop = (id: string) => -(actionPose.attitudeFor({ pose: { id } })?.move.root?.[1] ?? 0);
    expect(drop(deeper)).toBeGreaterThan(drop(shallower));
    expect(drop(shallower)).toBeGreaterThan(0);
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
