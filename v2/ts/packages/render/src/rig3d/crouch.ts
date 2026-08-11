// The knee bend that lets this rig settle (#445).
//
// ---------------------------------------------------------------------------
// WHY A ROOT DROP COULD NEVER DO THIS
// ---------------------------------------------------------------------------
//
// `action_pose.ts`'s `ATTITUDES` authors a crouch for five poses -- `settle`,
// `contain`, `fatigue`, `keeper_set`, `keeper_ready_low` -- as a downward
// translation of the rig root. A root translation moves the WHOLE rig, feet
// included, and this rig has no IK to put a slid foot back, so a drop of `d` is
// `d` of foot penetration exactly. #444 floored that translation against the
// rig's ground clearance, which on RIG_MEDIUM is zero, so grounding the drop
// and deleting it became the same operation and all five poses rendered at full
// standing height. `keeper_set` and `keeper_ready_low` became the same held
// pose.
//
// The authored magnitudes were never the problem and are NOT re-tuned here.
// What was missing is the thing that ABSORBS them: limb work. This module is
// that, and it is the pattern `clips.ts`'s SWING already demonstrates -- SWING
// drops its root 32 mm mid-lunge and does not penetrate, because its own thigh
// and shin keys raise the feet by 121 mm.
//
// ---------------------------------------------------------------------------
// THE SHAPE, AND WHY IT IS EXACT RATHER THAN TUNED
// ---------------------------------------------------------------------------
//
// A crouch is: hips down by `d`, feet where they were. On a leg of two
// segments hanging straight down, folding the thigh FORWARD by `c`, the knee
// BACKWARD by `2c` and the ankle forward by `c` again leaves the shank at `+c`
// from vertical, the foot back at its rest ORIENTATION (the three angles sum to
// zero, so the sole stays parallel to the turf) and the ankle at
//
//     upperleg * cos(c) + lowerleg * cos(c)
//
// below the hip instead of `upperleg + lowerleg`. So the foot RISES by
// `L * (1 - cos c)` for `L = upperleg + lowerleg`, and the drop that exactly
// cancels it is the same quantity. Inverting:
//
//     c = acos(1 - drop / L)
//
// which is why no angle in this file is a tuned constant. Give it a drop and it
// returns the fold that pays for it; `crouch.spec.ts` measures the residual
// through the REAL skeleton rather than trusting the derivation, and it is
// under a tenth of a millimetre.
//
// Two honest approximations in that closed form, both measured rather than
// asserted:
//
//   * THE 2 DEGREE THIGH ROLL. `skeleton.ts` gives each thigh a `rest` roll of
//     +/-2 degrees about z, so the leg does not hang exactly along -Y and the
//     shin's fold is about an axis tilted by the same 2 degrees. The error is
//     `1 - cos(2 deg)` = 0.06% of the drop: 0.11 mm at the ankle joint on the
//     deepest crouch here, and +0.09 mm at the rendered SOLE, which is the one
//     that decides ground contact and which lands ABOVE where a standing
//     character's sole does rather than below it. Both are measured --
//     `crouch.spec.ts` at the joint, `ground_contact.spec.ts` through the real
//     mesh, where the direction is pinned as well as the size.
//   * THE FEET TRAVEL FORWARD. `upperleg` and `lowerleg` differ, so a
//     symmetric fold leaves the ankle `(upperleg - lowerleg) * sin(c)` ahead of
//     where it was: 27 mm at the deepest crouch. That is knees-over-toes, which
//     is what a braced crouch looks like; the alternative is a two-variable
//     solve that keeps the ankle plumb, and it buys a numeric solver for 27 mm
//     of a shape nobody is measuring.
//
// ---------------------------------------------------------------------------
// UNITS: `move` UNITS, NOT WORLD METRES
// ---------------------------------------------------------------------------
//
// `skeleton.apply` multiplies `pose.move` by the rig's motion_scale (0.88) and
// does NOT scale rotations. So a drop authored in `move` units becomes
// `drop * motion_scale` of world travel while the fold's rise is already
// world metres, and the two only cancel if the conversion happens exactly once.
// It happens in `angleFor` below, which is the only place in this file that
// crosses between the two. This is the same trap #436 records and #444's
// `groundedRootY` divides by; it is the reason a spec here measures the planted
// foot rather than the authored number.
//
// Everything this module takes and returns is in `move` units, which is the
// unit `action_pose.attitudeDrop` reports and the unit `pose.move` carries.
//
// ONE RIG, AGAIN. Like `action_pose.ts`, this reads `RIG_MEDIUM` at module load
// while `skeleton.apply` uses the `Rig` it is handed. They agree because there
// is exactly one `RigProportions` (see `proportions.ts`), and this file carries
// the same disclosure that file's MOTION_SCALE note does: a second rig has to
// pass its own proportions in, not read a different constant.

import { quat, type Quat } from "@gc/core";
import * as proportions from "./proportions.ts";
import type { Pose } from "./skeleton.ts";

const RIG = proportions.RIG_MEDIUM;

/**
 * Hip-to-ANKLE reach with the leg straight, in metres: `upperleg + lowerleg`,
 * which is where the bone chain ends. NOT hip-to-sole -- the boot hangs below
 * the ankle and is nowhere in this sum.
 *
 * Hip-to-ankle is nonetheless the right quantity to fold against, and the
 * reason is the third angle. The fold's three local rotations sum to zero, so
 * the `foot` bone comes back to its REST ORIENTATION and the sole keeps its
 * exact offset from the ankle -- an unrotated rigid attachment. The sole then
 * translates by whatever the ankle translates by, and the boot's own depth
 * cancels out of the arithmetic rather than being neglected by it. Drop the
 * ankle's compensation and that stops holding at once: the sole would swing
 * about the ankle instead of riding it, and the toe would dig in by tens of
 * millimetres (measured: 40 mm at the deepest crouch).
 */
const LEG_REACH = RIG.seg.upperleg + RIG.seg.lowerleg;

/** The rig's `move` -> world scale, the same one `skeleton.apply` applies. */
const MOTION_SCALE = RIG.motion_scale;

/** The bones a crouch drives. `root` carries the drop; the rest carry the fold. */
export const BONES: readonly string[] = [
  "root",
  "thigh.L",
  "thigh.R",
  "shin.L",
  "shin.R",
  "foot.L",
  "foot.R",
];

/**
 * The thigh fold, in DEGREES, that raises the foot by `drop` of world travel.
 *
 * `drop` is in `move` units and positive means down, matching
 * `action_pose.attitudeDrop`. The knee takes twice this and the ankle takes it
 * back again, so the sole stays parallel to the pitch (see the header).
 *
 * The clamp is domain protection and nothing else: a drop deeper than the leg
 * is long has no fold that pays for it, and `acos` would return NaN rather than
 * saying so. No authored attitude is near it -- the deepest is 73.6 mm against
 * a 660 mm leg -- so the clamp has never been reached and the pose it would
 * produce (a leg folded flat) is not a shape anything here intends.
 */
export function angleFor(drop: number): number {
  const world = drop * MOTION_SCALE;
  const cosine = Math.max(-1, Math.min(1, 1 - world / LEG_REACH));
  return (Math.acos(cosine) * 180) / Math.PI;
}

function xRotation(degrees: number): Quat {
  return quat.fromEuler((degrees * Math.PI) / 180, 0, 0);
}

/**
 * The crouch overlay for one authored drop, or `null` when there is no crouch.
 *
 * A SPARSE overlay, composed onto the resolved pose by `clips.compose` rather
 * than layered over it: it names the six bones it drives and nothing else, so
 * the stride, the stance and the arms it says nothing about are untouched. That
 * is the whole reason it needs no bone mask -- `clips.layer`'s mask exists to
 * decide which bones go to REST when the overlay is silent about them, and an
 * additive overlay has no such case.
 *
 * NEGATIVE x SWINGS A LIMB FORWARD and a knee only bends backward, so the thigh
 * and ankle angles are negative and the shin's is positive -- the same
 * convention (and the same signs) as SWING's own lunge keys. See `clips.ts`'s
 * header.
 */
export function poseFor(drop: number): Pose | null {
  if (drop <= 0) {
    return null;
  }
  const c = angleFor(drop);
  const thigh = xRotation(-c);
  const shin = xRotation(2 * c);
  const foot = xRotation(-c);
  return {
    rot: {
      "thigh.L": thigh,
      "thigh.R": thigh,
      "shin.L": shin,
      "shin.R": shin,
      "foot.L": foot,
      "foot.R": foot,
    },
    // The drop the fold has just paid for. Authored on `root`, the one bone
    // whose translation is world-space, and left for `clips.compose` to ADD to
    // whatever the gait wrote -- so the run's vertical bob survives the crouch
    // exactly as `action_pose.apply`'s attitude branch intends it to.
    move: { root: [0, -drop, 0] },
  };
}
