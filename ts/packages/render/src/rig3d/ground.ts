// Ground contact for a POSED rig: the lift a root rotation never had (#446).
//
// ---------------------------------------------------------------------------
// WHAT WAS MISSING
// ---------------------------------------------------------------------------
//
// `action_pose.save()` writes a rotation and a horizontal travel, and nothing
// else. For a dive to a side that is a roll about Z with LATERAL travel along
// X; since #451, for a save the simulation reports no lateral component for,
// it is a forward pitch about X with travel along Z instead. What both have in
// common is the part that matters here: the Y component of that translation is
// zero, for every save, at every amount -- while the rotation reaches 84
// degrees. So the rig, whose `root` sits on the pitch plane at [0, 0, 0],
// takes a standing body most of the way over and never rises. The down-side
// boot, then the hand, then anything strapped to the forearm sweeps under the
// turf.
//
// Measured through the real geometry on the LATERAL branch: 87 mm at
// `keeper_spread`, 140 mm at `keeper_central`, 192 mm at `keeper_dive`,
// 278 mm at `keeper_stretch`, 361 mm at `keeper_tip` -- body only, and roughly
// twice that counting the shield the render layer currently gives every keeper
// (#447). The body save trades those round: shallower on the body, deeper on
// the props. `ground_contact.spec.ts` carries and pins both sets, and NEITHER
// set is read by the code below -- see the next section for why the lift is
// measured off the posed rig rather than derived from any of them, which is
// also why a new rotation axis needed no change here.
//
// #439/#444 grounded downward root TRANSLATIONS by flooring the overlay's Y.
// That fix is exact and costs nothing per frame, and it does not reach this:
// a rotation moves feet just as surely as a drop does, and how far it moves
// them depends on where the limbs are, which the overlay cannot see.
//
// THE MISSING THING IS A LIFT, NOT A CLAMP. A diving keeper is airborne. The
// pose describes the rotation of the dive and omits its vertical component
// entirely, so the correction is to supply that component rather than to
// restrain the rotation (which #446 puts explicitly out of scope, for the
// same reason #439 did: the constants describe the intended motion).
//
// ---------------------------------------------------------------------------
// WHY THE LIFT IS MEASURED RATHER THAN DERIVED
// ---------------------------------------------------------------------------
//
// The tempting version of this lives next to #444's floor in
// `action_pose.apply`, as a closed form: a body of horizontal half-extent `r`
// rolled by `theta` about a pivot on the plane needs `r * |sin theta|` of
// lift. It was measured before it was rejected, and it does not work.
// Solving for `r` against the real geometry, pose by pose:
//
//     keeper_spread   28 deg   87 mm   ->  r = 0.186
//     keeper_central  48 deg  140 mm   ->  r = 0.189
//     keeper_dive     72 deg  192 mm   ->  r = 0.202
//     keeper_stretch  78 deg  278 mm   ->  r = 0.284
//     keeper_tip      84 deg  361 mm   ->  r = 0.363
//
// The first three agree, because their lowest point is a BOOT and a boot is
// where the rest pose puts it. The last two do not, because their lowest
// point is a HAND, and `pose_table.ts` gives those two saves stance clips
// that reach the arm out past the rest silhouette. A single rig-derived `r`
// therefore under-lifts `keeper_stretch` by 92 mm and `keeper_tip` by 172 mm
// -- limbs still through the pitch, in the two deepest poses in the game.
// A per-bone bounding box evaluated from the overlay fares worse in the other
// direction: measured, it over-lifts by up to 202 mm, which is a floating
// keeper.
//
// The lift depends on the POSED rig, so it is computed from the posed rig.
// That is why this file exists at all instead of six more lines in
// `action_pose.ts`.
//
// ---------------------------------------------------------------------------
// WHY NOT MOVE THE PIVOT INSTEAD
// ---------------------------------------------------------------------------
//
// #446 offers "pivot about the plane" as the alternative. The root is ALREADY
// on the plane (`skeleton.ts`: `root` is at [0, 0, 0]), so what that option
// actually means is pivoting about the down-side CONTACT POINT rather than
// about the midpoint between the feet. Rotating about a pivot at local x = p
// instead of at the origin contributes
//
//     dy = -p * sin(theta)          dx = p * (1 - cos(theta))
//
// The `dy` is this file's lift, arrived at from the other end. The `dx` is
// the difference between the two options, and it is not small: for
// `keeper_tip` the contact point that grazes the plane sits at p ~ 0.363 m
// and `cos 84 deg` is 0.105, so a pivot change would add 0.325 m of lateral
// travel to an authored 0.576 m -- a 56% change to a distance #436 corrected
// on purpose, arriving as a side effect of a ground-contact fix. A pure lift
// is the same correction with that side effect removed, which is why it is
// the one taken. The two are NOT the same transform expressed two ways.
//
// ---------------------------------------------------------------------------
// THE PROPERTY, AND WHAT IT IS NOT
// ---------------------------------------------------------------------------
//
// `poseAndGround` guarantees exactly one thing: no rendered vertex ends below
// the plane, for any pose, any clip, any amount. It buys that with a rigid
// vertical translation of the whole character, which is honest for a body
// that is genuinely airborne (a dive, a knockback) and is an approximation
// for a body that is merely leaning: the up-side boot rises with everything
// else instead of staying planted. Measured, that costs 120 mm of clearance
// on the up-side boot at `keeper_spread` and 69 mm at `keeper_get_up`, and
// NOTHING at all on the six lean-and-tip poses, whose root tilt is about X --
// there both boots share a contact point, so both stay on the turf.
//
// Planting the up-side foot while the down-side one lifts is limb work: a
// two-bone leg solve, which is #318 and is deliberately not here. This is the
// whole-body half of that problem, and it is the half that is 361 mm deep.
//
// ---------------------------------------------------------------------------
// THE REST POSE IS A CONSTANT, AND IS RESOLVED AT CONSTRUCTION
// ---------------------------------------------------------------------------
//
// The rig is authored to plant 1.2 mm UNDER the plane: the boot's outer sole
// corner, because the thigh carries a 2 degree rest roll. #444 measured that
// and spent it as "no clearance to spend", which is what let it floor a drop
// at zero.
//
// 1.231242 mm on a 1.5676 m figure, which is 0.02 px of the ~26 px a character
// occupies at the match camera scale #448's captures were taken at -- stated
// with its divisor because "sub-pixel" on its own is the kind of claim that
// drifts.
//
// It is a property of the REST POSE, not of any frame. Measured over 24 phases
// of the idle clip, the lowest rendered point never goes below the identity
// rest pose's own -1.231242 mm -- the two agree to every digit -- so there is
// nothing per-frame about it at all. Left to `poseAndGround` it would be
// re-measured sixty times a second, the early-out below would never fire for a
// standing character, and every idle player on the pitch would pay a doubled
// skeleton evaluation to be raised 0.02 px.
//
// So `probesFrom` measures it once, as `restLift`, and the built character's
// rig is raised by it (`skeleton.raised`, called from
// `player_renderer_3d.ts`'s `build`). Then a standing character measures >= 0,
// the early-out fires for real, and the second `skeleton.apply` is paid only
// when something is actually penetrating.
//
// NOT AN EPSILON, which is the thing this deliberately is not. A tolerance
// would say "ignore penetration under 1.2 mm" and would hide a real 1.2 mm
// defect the day one appears. This moves the RIG instead: the character still
// plants exactly on the plane, every measurement stays exact, and the number
// that changes is where the skeleton starts rather than what counts as zero.
//
// EXACT, AND PINNED AS SUCH. Raising by `restLift` leaves the rest pose at
// +1.4e-17 m rather than at 0 -- the residue of adding and subtracting the same
// double through two different matrix chains -- and that residue is POSITIVE,
// which is the side the early-out needs. Nothing here relies on it staying
// that small; what it relies on is it not being negative, and
// `ground_contact.spec.ts` asserts exactly that over the whole locomotion
// sweep. If a future rig lands it on the other side, the failure is a doubled
// evaluation caught by an apply-count test, never a character in the turf.
//
// ---------------------------------------------------------------------------
// COST
// ---------------------------------------------------------------------------
//
// Costs below are PER CHARACTER on purpose. How many characters render is not
// rig3d's fact and does not belong in this file -- it is `gc-sim`'s
// (`input_frame.rs`: `HOME_SLOT_COUNT` and `AWAY_SLOT_COUNT`, four outfield
// slots a side, plus a keeper each; `gc-data/formations.rs`: "small-sided 5v5
// (1 keeper + 4 outfield)"). Multiply there, not here.
//
// The exact answer is a minimum over every rendered vertex, and there are
// 14112 of them -- 16 to 19 us per character across runs, which is too much
// to spend on a property that binds for a handful of players at a time. So
// the scan is a
// branch and bound: each bone carries a precomputed bounding SPHERE, a sphere
// gives a lower bound on that bone's lowest vertex for one dot product, and
// bones are visited in increasing bound order and abandoned as soon as the
// bound cannot beat the best real vertex found. The answer is identical -- it
// is a bound, not an approximation -- and measured, it visits 1488 vertices in
// the worst case instead of 14112: 1.9 us per character. `ground.spec.ts`
// pins the equality against the brute-force scan so the pruning cannot go
// quietly wrong.
//
// THE PRUNE'S PRECONDITION, unstated until a reviewer asked for it: a bounding
// sphere of radius r bounds the bone's vertices in WORLD space only if the
// bone's world matrix has an orthonormal linear part. That holds today because
// `skeleton.apply` composes unit quaternions with translations and nothing
// else -- there is no scale anywhere in this rig. If per-player size variance
// ever introduces one, a bone scaled UP would have its sphere under-estimate
// how far its vertices reach, the prune would skip the bone that is actually
// lowest, and the lift would come back too SMALL: a silent under-lift, which
// is the failure direction that hides. `ground.spec.ts` pins the pruned scan
// equal to a brute-force minimum across the whole pose table, so the day a
// scale appears that suite goes red rather than the pitch quietly swallowing a
// boot again.
//
// THE SECOND `skeleton.apply` IS THE REST OF THE BILL when it happens, and
// resolving the rest pose at construction is what stops it happening to
// everyone. Measured per character, pose resolution included, against the same
// work before #446:
//
//                            before   after (raised)   after (UNRAISED rig)
//     idle                    7.3 us    10.0 us (1.4x)      15.6 us (2.1x)
//     running                 7.9 us     8.8 us (1.1x)      --
//     full keeper_dive        8.5 us    14.8 us (1.7x)      --
//
// READ THE RATIOS, NOT THE MICROSECONDS. These come from a Node microbenchmark
// over this module, not from the browser/three.js/GPU path the game actually
// ships on, so they are not a frame budget and the absolute figures do not
// survive being quoted as one. An independent re-run on the same machine
// reproduced the ordering and the story -- raised cheaper than unraised,
// keeper_dive heaviest -- while landing the idle-before at 8.3 us and the
// unraised idle ratio anywhere from 1.15x to 1.9x. The ratio is what the
// argument below rests on and the ratio is what reproduces; the third
// significant figure is noise, and the count tests, not these numbers, are
// what hold the property.
//
// The right-hand column is what this file looked like before the rest pose
// moved to build time: every standing character taking the correction branch
// and paying a second skeleton evaluation to be raised 0.02 px.
// `ground_contact.spec.ts` and `ground.spec.ts` both pin that it does not,
// the latter by counting `skeleton.apply` outright.
//
// Paid rather than optimised further: the obvious next step is to add the lift
// to each world matrix's Y translation instead of re-evaluating, which is
// exactly equivalent by the same argument `poseAndGround` makes below, but
// `Mat4` is a readonly tuple and rebuilding 26 of them by hand is a second
// copy of `skeleton.apply`'s contract living here. If this ever shows up in a
// profile, that is the change to make, with this comment as the reason it is
// safe.
//
// NO LOD EARLY-OUT, and whoever wires one in needs to know. `rig3d/pose_lod.ts`
// is ported but unwired -- nothing under `v2/ts` calls it -- so there is no
// interaction today. When #394's LOD is wired, `poseAndGround` will still do a
// full evaluation and a full scan for every character every frame, including
// held or distant ones whose pose did not change. The scan result is a pure
// function of the posed rig, so it caches exactly as the pose does; the place
// to skip it is wherever the LOD decides the pose is unchanged, not inside
// this function.

import type { MutablePose } from "./action_pose.ts";
import type { Vertex } from "./geometry.ts";
import * as skeleton from "./skeleton.ts";

/** One bone's rendered vertices, in bone-local metres, plus its bounding sphere. */
interface BoneProbe {
  readonly bone: string;
  /** Flat xyz triples. */
  readonly points: Float64Array;
  readonly cx: number;
  readonly cy: number;
  readonly cz: number;
  readonly radius: number;
}

/**
 * Precomputed ground probes for one character's geometry.
 *
 * Built once per built character, from the same `body.accumulate` output the
 * renderer uploads, so it cannot describe a different figure than the one on
 * screen.
 *
 * `bound` and `visited` are per-call scratch, not state: they are preallocated
 * here rather than per frame because `lowestPoint` is called once per
 * character per frame. Reusing them is safe for exactly the reason the shared
 * character mesh in `player_renderer_3d.ts` is safe -- one character is posed
 * at a time and JavaScript is single-threaded -- and it means the scan
 * allocates nothing.
 */
export interface GroundProbes {
  readonly probes: readonly BoneProbe[];
  readonly bound: Float64Array;
  readonly visited: Uint8Array;
  /**
   * How far this geometry's REST pose sits below the pitch plane, in metres,
   * and therefore how far the rig it rides has to be raised for a character
   * who is doing nothing to plant on it. Constant, measured once, applied via
   * `skeleton.raised` at build time -- see this file's REST POSE section.
   */
  readonly restLift: number;
}

/**
 * Groups a merged character's vertices by bone, fits each group a bounding
 * sphere, and measures the rest pose's own penetration. Pure; `verts` is
 * `body.accumulate`'s `PartBuilder.verts`, `boneOrder` is
 * `skeleton.bones(style).map(b => b.name)`, and `style` is the proportions
 * that geometry was built against.
 *
 * The sphere is centroid-and-farthest-vertex rather than a minimal enclosing
 * sphere: it only has to be a valid bound, and a looser one costs a few more
 * visited bones, never a wrong answer.
 */
export function probesFrom(
  style: skeleton.RigLike,
  verts: readonly Vertex[],
  boneOrder: readonly string[],
): GroundProbes {
  const grouped = new Map<string, number[]>();
  for (const vertex of verts) {
    const name = boneOrder[vertex.bone];
    if (name === undefined) {
      throw new Error(`rig3d/ground.ts: vertex on unknown bone index ${String(vertex.bone)}`);
    }
    let flat = grouped.get(name);
    if (flat === undefined) {
      flat = [];
      grouped.set(name, flat);
    }
    flat.push(vertex.position[0], vertex.position[1], vertex.position[2]);
  }

  const probes: BoneProbe[] = [];
  for (const [bone, flat] of grouped) {
    const points = Float64Array.from(flat);
    const n = points.length / 3;
    let cx = 0;
    let cy = 0;
    let cz = 0;
    for (let i = 0; i < points.length; i += 3) {
      cx += points[i] ?? 0;
      cy += points[i + 1] ?? 0;
      cz += points[i + 2] ?? 0;
    }
    cx /= n;
    cy /= n;
    cz /= n;
    let radius = 0;
    for (let i = 0; i < points.length; i += 3) {
      const dx = (points[i] ?? 0) - cx;
      const dy = (points[i + 1] ?? 0) - cy;
      const dz = (points[i + 2] ?? 0) - cz;
      const d = Math.sqrt(dx * dx + dy * dy + dz * dz);
      if (d > radius) {
        radius = d;
      }
    }
    probes.push({ bone, points, cx, cy, cz, radius });
  }

  // The rest pose, measured through the scan itself rather than through a
  // second code path: `newRig` evaluates the identity pose in its constructor,
  // so this is the same figure `ground_contact.spec.ts`'s first test measures
  // standing still. A rig that ever stands CLEAR of the plane needs no
  // correction, hence the `max(0, ...)`.
  const partial: GroundProbes = {
    probes,
    bound: new Float64Array(probes.length),
    visited: new Uint8Array(probes.length),
    restLift: 0,
  };
  return { ...partial, restLift: Math.max(0, -lowestPoint(skeleton.newRig(style), partial)) };
}

/**
 * The lowest rendered point of an already-posed rig, in metres above the
 * pitch plane. Negative means something is through the turf.
 *
 * Exact: the pruning below is a branch and bound over per-bone bounding
 * spheres, never an approximation of the minimum (see this file's COST note).
 *
 * `Mat4` is row-major (`@gc/core/mat4.ts`), so a vertex's world Y is
 * `m[4]x + m[5]y + m[6]z + m[7]` -- the row is hoisted out of each bone's
 * inner loop, which is most of why the scan is cheap.
 */
export function lowestPoint(rig: skeleton.Rig, probes: GroundProbes): number {
  const { probes: parts, bound, visited } = probes;
  visited.fill(0);
  for (let j = 0; j < parts.length; j += 1) {
    const part = parts[j];
    if (part === undefined) {
      // Unreachable for j < parts.length; written rather than skipped so a
      // stale bound from the previous call can never survive into this one.
      bound[j] = Infinity;
      continue;
    }
    const m = rig.world[part.bone];
    if (m === undefined) {
      throw new Error(`rig3d/ground.ts: posed rig has no bone ${part.bone}`);
    }
    bound[j] = m[4] * part.cx + m[5] * part.cy + m[6] * part.cz + m[7] - part.radius;
  }

  let best = Infinity;
  for (;;) {
    // The unvisited bone whose sphere reaches lowest. A linear search over
    // ~23 bones, run two to five times in practice -- cheaper than sorting,
    // and it allocates nothing.
    let next = -1;
    let nextBound = Infinity;
    for (let j = 0; j < parts.length; j += 1) {
      if (visited[j] === 0 && (bound[j] ?? Infinity) < nextBound) {
        next = j;
        nextBound = bound[j] ?? Infinity;
      }
    }
    if (next < 0 || nextBound >= best) {
      break;
    }
    visited[next] = 1;
    const part = parts[next];
    if (part === undefined) {
      break;
    }
    const m = rig.world[part.bone];
    if (m === undefined) {
      throw new Error(`rig3d/ground.ts: posed rig has no bone ${part.bone}`);
    }
    const ax = m[4];
    const ay = m[5];
    const az = m[6];
    const d = m[7];
    const points = part.points;
    for (let i = 0; i < points.length; i += 3) {
      const y = ax * (points[i] ?? 0) + ay * (points[i + 1] ?? 0) + az * (points[i + 2] ?? 0) + d;
      if (y < best) {
        best = y;
      }
    }
  }
  return best;
}

/**
 * Poses `rig` with `pose` and grounds it: if anything rendered would end up
 * below the pitch plane, the whole character rises by exactly that much.
 * Returns the lift applied, in world metres (0 when nothing penetrated).
 *
 * Call this instead of `skeleton.apply` wherever a character is posed for
 * display. It is `skeleton.apply` plus a measurement, and it mutates `pose`
 * so the lift is visible to whatever inspects the pose afterwards rather than
 * being hidden in the world matrices.
 *
 * `rig` is expected to have been raised by `probes.restLift` already
 * (`skeleton.raised`, at build time). That is what makes the early-out below
 * fire for a character who is merely standing, so the second `skeleton.apply`
 * is paid only by a character whose POSE is penetrating. Hand it an unraised
 * rig and the result is still correct -- every frame is simply lifted the
 * rig's own 1.2 mm here instead of once at construction, at the cost of a
 * doubled evaluation for every idle character. See this file's REST POSE
 * section, and `ground_contact.spec.ts`, which counts the evaluations.
 *
 * ONE CORRECTION IS EXACT, so there is no iteration and no convergence
 * question. `root` has no parent and `skeleton.apply` builds its local
 * transform as `quat.toMat4(rot, offset + move * motion_scale)`, so the
 * translation is the matrix's own column with nothing above it to re-orient:
 * a metre added to `move.root.y` is a metre of world travel for every vertex
 * in the character, whatever the rest of the pose is doing. This is the same
 * property `action_pose.ts`'s GROUND CONTACT note relies on for the floor,
 * used in the opposite direction.
 *
 * DIVIDED BY `motion_scale`, which is the #436 trap and the reason a lift is
 * not simply added: `skeleton.apply` multiplies `move` by the rig's
 * motion_scale (0.88), so a quantity in world metres becomes a quantity in
 * `move` units only after dividing by it. Adding the raw lift would clear
 * 88% of the penetration and leave the remaining 12% -- 43 mm at
 * `keeper_dive`, which is exactly the size of bug that survives a reviewer.
 *
 * COMPOSES WITH #444's FLOOR RATHER THAN REPLACING IT. The floor removes the
 * authored downward translation before the pose is ever evaluated; this
 * removes whatever the rotation leaves after that. They are not two attempts
 * at one job: the floor is a constant correction applied at authoring time,
 * while a lift necessarily reads the posed rig every frame, and a per-frame
 * correction applied to a CROUCH would track the gait's own vertical bob --
 * absorbed at the top of the stride, fully applied at the bottom -- and turn
 * a held posture into a wobble at stride frequency. That is precisely the
 * failure `action_pose.ts`'s GROUND CLEARANCE note rejects a per-frame budget
 * for, and it is why the floor is still what handles drops.
 * `ground_contact.spec.ts` drives that interaction directly.
 */
export function poseAndGround(rig: skeleton.Rig, pose: MutablePose, probes: GroundProbes): number {
  skeleton.apply(rig, pose);
  const lowest = lowestPoint(rig, probes);
  if (lowest >= 0) {
    return 0;
  }
  const lift = -lowest;
  const k = rig.style.motion_scale ?? 1;
  const root = pose.move["root"] ?? [0, 0, 0];
  pose.move["root"] = [root[0] ?? 0, (root[1] ?? 0) + lift / k, root[2] ?? 0];
  skeleton.apply(rig, pose);
  return lift;
}
