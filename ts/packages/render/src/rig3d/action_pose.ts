// Whole-body action poses, as a sparse overlay on the rig root.
//
// These are whole-body poses -- a keeper's dive, a bicycle kick, a knockback, a
// stumble -- and it is worth being precise about why they are ROOT TRANSFORMS
// and not clips.
//
// Every one of them is continuous. A dive is not a fixed silhouette played
// back; it is the body rotating through an angle as `dive` runs 0 -> 1 against
// the simulation's own timer. The same is true of the aerial lift, the wind-up
// and the tip. A keyframe clip would have to re-derive that ramp and would
// then disagree with the timer driving it, whereas a parameterised transform
// reads the timer directly, and so stays in step with the sim.
//
// So the numbers below are not new. They are the constants the 2.5D renderer
// already had, tuned against the real game, moved onto the rig. Authored clips
// for these actions remain #101/#102's job; this is what keeps them legible in
// the meantime.
//
// ORIENTATION, since every sign here depends on it (skeleton.ts):
//   +Y is up, the character faces +Z, and because the frame is right-handed
//   their own RIGHT is at -X.
//   * rot.z rotates +X toward +Y, so a POSITIVE z tips the body toward their
//     own right (the head, at +Y, swings to -X).
//   * rot.x rotates +Y toward +Z, so a POSITIVE x tips the body FORWARD onto
//     its face, and a negative x tips it backward.
//   * move on `root` is a translation in the character's own frame before the
//     draw-time yaw, so move.x is sideways and move.z is along their facing.
//     Like every clip translation it rides the rig's motion_scale.
//
// Distances are expressed in PLAYER RADII, the same unit the 2.5D constants
// were written in, and converted once on the way out -- through `metres()`,
// which is the only radii-to-metres crossing in the file (#436). That keeps
// the numbers directly comparable to the renderer they came from.
//
// No IK here: every pose is a transform of the rig ROOT bone, not a limb
// solve. rig3d has no two-bone IK anywhere in this milestone's source files.
//
// TWO KINDS OF ROOT TRANSFORM (#430)
//
// Everything above describes an ACTION: a dive, an aerial, a knockback. An
// action is a discrete event with its own timer, it OWNS the body while it
// runs, and `forOptions` is the predicate for "an action owns the body this
// frame" -- `player_renderer_3d.ts` gates the balance lean on exactly that.
//
// `ATTITUDES` below is the other kind: a held POSTURE. A player who is
// containing, tiring, settling a ball or following through on a shot is still
// running, still balancing, still on the locomotion blend -- the attitude is a
// modifier on top of all of that, not a replacement for it. That difference is
// mechanical, not editorial, and it shows up in three places:
//
//   * attitudes are NOT part of `forOptions`, so a running player who is
//     containing or tiring keeps their velocity-derived torso lean. Folding
//     them into that predicate would silently switch the lean off for four of
//     the commonest poses on the pitch;
//   * `apply` COMPOSES an attitude onto the resolved pose (pre-multiplying the
//     root rotation, adding the root translation) instead of assigning it, so
//     the run's vertical bob -- which the locomotion clips write to
//     `move.root` -- survives a crouch instead of being overwritten; and
//   * an action beats an attitude if a pose id ever claims both. Today the two
//     tables have disjoint keys, so the ordering is a stated rule rather than a
//     live case.

import { quat, type Quat } from "@gc/core";
import type { EulerTriple } from "./clips.ts";
import * as proportions from "./proportions.ts";

/** The mutable pose shape `skeleton.apply` consumes: quaternion rotations. */
export interface MutablePose {
  readonly rot: Record<string, Quat>;
  readonly move: Record<string, EulerTriple>;
}

/** A pitch-space direction or position: `{x, y}`, structurally compatible with `Vec2`. */
export interface XY {
  readonly x: number;
  readonly y: number;
}

/** A sparse root-only overlay, in the authoring convention (degrees, metres). */
export interface RootPose {
  readonly rot: Readonly<Record<string, EulerTriple>>;
  readonly move: Readonly<Record<string, EulerTriple>>;
}

/** Which whole-body action, if any, is driving the root this frame. */
export interface PoseId {
  readonly id?: string;
}

/** Inputs the whole-body action poses read. Every field is optional. */
export interface ActionPoseOptions {
  readonly pose?: PoseId;
  readonly dive?: number;
  readonly dive_dir?: XY;
  readonly facing?: XY;
  readonly aerial?: number;
  readonly aerial_style?: string;
  readonly aerial_jump?: number;
  /**
   * Ticks remaining in the current forced (stagger/knockback) disable
   * window -- `gc_sim::combat_snapshot::CombatPlayerState.forced_ticks`,
   * mirrored one hop through `gc_render::player_pose::CombatPoseSample`'s
   * field of the same name. Only read when `pose.id` is
   * `"combat_knockback"` or `"combat_stagger"`, and only by [`apply`]'s hit-
   * reaction envelope (see its own doc) -- `forOptions`/`tip` never read it,
   * so the AUTHORED static tilt in `TIPS` stays the single source of truth
   * for the pose's magnitude; the envelope only ever scales it.
   *
   * `undefined` -- not `0` -- is "no live tick count for this frame", which
   * is every caller today: this field is not yet threaded from a live
   * `RenderFrame`, which does not carry it (see
   * `gc_render::frame::FrameCombatModel`'s own doc on why not). `apply`'s
   * hit-reaction envelope does NOT simply go unscaled when this is
   * `undefined` -- it falls back to a wall-clock approximation instead, so
   * the curve is live today rather than dormant until that wire extension
   * lands. See `hitReaction`'s own doc for the fallback's shape and why it
   * degrades gracefully. `0` means "the window is ending this tick", which
   * is a real, meaningfully different frame from "no data" and is handled
   * by the EXACT path, not the fallback.
   */
  readonly forced_ticks?: number;
}

// ONE UNIT, ONE CONVERSION (#436).
//
// Every distance in this file is authored in PLAYER RADII and every distance
// that leaves it is in METRES, because `proportions.RigSegments` is metres and
// `skeleton.apply` adds `pose.move` straight onto a bone offset. `metres()`
// below is the only place that crossing happens.
//
// It is a function rather than a comment because the comment did not hold.
// Until #436 this file carried a second constant, `RADII_PER_HEIGHT = 6.0`,
// which turns radii into RIG HEIGHTS rather than metres -- a plausible-looking
// divisor sitting one line above the honest conversion, guarded only by a note
// saying which one to use. Four translation sites used it: `save`, `aerial`
// and `tip` twice. Every keeper dive, tip, get-up, knockback, stagger, bicycle
// kick, header and volley therefore travelled a factor of the rig's own height
// (~1.57x) short of the radii it was labelled with -- `keeper_dive` moved
// 0.267 m where its 1.6r says 0.418 m. That name no longer exists, so the
// short idiom is now a compile error rather than a thing to remember.
//
// The rig is HEIGHT_IN_RADII * 2 = 6 radii tall by construction, which is why
// the divisor is 6 and why no camera state is needed. That identity is
// `player_renderer_3d.ts`'s TORSO LEAN block, not a new claim: `ppmForRadius`
// sets ppm = r * HEIGHT_IN_RADII * 2 / height, so `k * r` on-screen pixels is
// `k * height / 6` metres for any k and any r -- the `r` cancels, which is why
// a billboard constant written in projected pixels converts to rig metres with
// no camera state at all.
const RIG_HEIGHT_METRES = proportions.height(proportions.RIG_MEDIUM);
const METRES_PER_RADIUS = RIG_HEIGHT_METRES / 6.0;

/**
 * One distance in player radii, in the metres `skeleton.apply` consumes.
 *
 * Exported so a sibling cannot re-derive the crossing and get a different
 * answer, and so the specs can pin a travel against the conversion rather than
 * against a hand-copied decimal.
 */
export function metres(radii: number): number {
  return radii * METRES_PER_RADIUS;
}

// Where a whole-figure lean is read off, in metres above the ground.
//
// The billboard TRANSLATED the whole figure sideways by `actionLean` pixels --
// hips, shoulders, head and feet together -- which a billboard can afford and
// a rig cannot: the feet are placed by the skeleton and there is no IK to put
// a slid foot back. So the displacement arrives as a rotation of the root,
// whose pivot is the ground between the feet (`skeleton.ts` puts `root` at
// [0, 0, 0]). A rotation moves a point at height h by h * sin(theta) rather
// than moving every point equally, so "the same amount of lean" only becomes
// well defined once a height is named -- the same argument
// `player_renderer_3d.ts`'s TORSO LEAN block makes for `view.lean`.
//
// It names the HEAD, because a torso tilt leaves the lower body alone and the
// head is the only part that visibly moves. This is the other case: the root
// tilt moves the whole figure, so the honest single match point is where the
// mass is, and that is the hips -- 0.8 m, 51% of standing height, which is the
// usual band for a standing human's centre of mass. Taken from the rig rather
// than written down, so a proportions change carries it.
const LEAN_REFERENCE_HEIGHT = proportions.RIG_MEDIUM.seg.hips_y;

// ---------------------------------------------------------------------------
// GROUND CONTACT (#439)
// ---------------------------------------------------------------------------
//
// A root translation moves the WHOLE rig, feet included, and there is no IK
// here to put a slid foot back (see this file's "No IK here" note). Upward is
// fine -- an aerial, a knockback and a stumble mean to leave the ground. A
// DOWNWARD translation is not: it drives the feet through the pitch, and it
// did, in seven places across `TIPS` and `ATTITUDES`, by 3.7 cm to 7.4 cm.
//
// WHY THE SINK EQUALS THE DROP. The rig is authored to plant exactly on the
// pitch plane: measured through the real `body.accumulate` geometry and the
// real `skeleton.apply`, the lowest rendered point of a STANDING character is
// -1.2 mm -- the boot's outer sole corner, a millimetre into the turf because
// the thigh carries a 2 degree rest roll. There is no clearance to spend, so a
// drop of d puts the sole d below the turf and nothing absorbs any of it.
// `ground_contact.spec.ts` measures that number rather than trusting this
// comment, and fails if the rig ever changes underneath it.
//
// THE FIX IS A FLOOR ON THE OVERLAY'S DOWNWARD COMPONENT, applied in `apply`
// below -- the one place every root overlay reaches a real pose, so a pose
// authored tomorrow is covered by construction rather than by remembering to
// call something. Three properties make that floor exact rather than
// approximate:
//
//   * `root` HAS NO PARENT, and `skeleton.apply` builds its local transform as
//     `quat.toMat4(rot, offset + move * motion_scale)` -- the translation is
//     the matrix's own column, with nothing above it to re-orient. So a metre
//     of authored `move.root.y` is a metre of WORLD travel for every vertex in
//     the character, whatever the rest of the pose is doing. A floor in metres
//     is therefore a floor in penetration, with no second evaluation pass and
//     no per-frame cost at all.
//   * IT IS EXPRESSED ON THE AUTHORING SIDE OF `motion_scale`, which is the
//     mistake #436 was: `skeleton.apply` multiplies `move` by the rig's
//     motion_scale (0.88), so a budget in world metres becomes a budget in
//     `move` units only after dividing by it. `groundedRootY` does that
//     division, and its spec pins it with a deliberately non-zero budget so
//     the arithmetic is exercised rather than multiplied away by today's zero.
//   * IT CLAMPS THE OVERLAY, NOT THE TOTAL. The locomotion clips write the
//     run's vertical bob to `move.root` too, and `apply` COMPOSES an attitude
//     onto that rather than assigning (see this file's ASSIGN FOR AN ACTION
//     note). Clamping the sum would eat the bob and leave a settling player
//     gliding, which is the exact failure that composition exists to avoid.
//     Clamping the overlay is conservative with respect to the sum because no
//     locomotion blend ever writes a NEGATIVE root y -- the walk and run clips
//     bottom out at exactly 0 on their contact keys -- which
//     `ground_contact.spec.ts` pins rather than assumes.
//
// WHAT THIS DELIBERATELY DOES NOT COVER, because an undisclosed gap is worse
// than a disclosed one:
//
//   * ROOT ROTATIONS -- now `rig3d/ground.ts`'s (#446), not an open gap.
//     Every attitude `lean` is a root tilt, `combat_stagger` pitches -8
//     degrees, `keeper_get_up` rolls 16 and the saves roll up to 84, and a
//     tilt about a root at y = 0 swings the far boot below the plane just as
//     surely as a drop does. Measured residuals at a standstill, with the drop
//     already grounded: 50 mm keeper_get_up, 24 mm run_telegraph, 13 mm
//     kick_follow, 11 mm combat_stagger, 8 mm contain, 4 mm fatigue, and
//     192-595 mm across the keeper saves.
//
//     THAT IS NOT FIXED HERE, and cannot be: how far a rotation drives a limb
//     under the turf depends on WHERE THE LIMB IS, and the overlay is authored
//     before any clip has posed one. Measured, a closed form in this file
//     using a single rig-derived half-extent fits the three saves whose lowest
//     point is a boot and under-lifts `keeper_stretch` by 92 mm and
//     `keeper_tip` by 172 mm, whose lowest point is a stance clip's
//     outstretched hand. So the correction is a LIFT measured off the posed
//     rig, in `ground.poseAndGround`, which every display path calls in place
//     of `skeleton.apply`. See that file for why a lift rather than a moved
//     pivot, and for the cost of measuring it.
//
//     THE TWO CORRECTIONS ARE NOT REDUNDANT even though both end up in
//     `move.root.y`. This one is constant and applied before evaluation; that
//     one necessarily reads the posed rig every frame. Apply a per-frame
//     correction to a CROUCH and it tracks the gait's own vertical bob --
//     absorbed at the top of the stride, fully applied at the bottom -- which
//     is a wobble at stride frequency on poses whose whole job is to read as a
//     held posture, and is exactly what the GROUND CLEARANCE note below
//     rejects a per-frame budget for. Drops are floored here; what the
//     rotation leaves is lifted there. `ground_contact.spec.ts` drives the
//     interaction rather than asserting it.
//   * CLIP-AUTHORED ROOT MOTION. `clips.ts`'s SWING authors a 32 mm root drop
//     mid-lunge -- 28 mm of world travel once `skeleton.apply` has applied the
//     same motion_scale this file divides by three paragraphs up, because that
//     multiply is uniform and does not care that a clip wrote the value. It
//     does NOT penetrate, because its own thigh and shin keys raise the feet
//     by more than that. That is the legitimate way to settle a body on this
//     rig, it is limb work, and clamping it would flatten a pose that is
//     already correct. Measured and pinned too.
//
// WHAT IT COST THE POSES, AND WHO PAYS IT NOW (#445). On RIG_MEDIUM the budget
// below is ZERO, so a grounded drop is no drop, and between #444 and #445 that
// meant `settle`, `keeper_ready_low` and `keeper_set` changed no silhouette at
// all while `contain` and `fatigue` kept their lean and lost their crouch --
// two collisions with it, both pinned as equalities in the specs rather than
// left as prose: `keeper_set` and `keeper_ready_low` became THE SAME HELD POSE
// (`pose_table.ts`: `keeper_ready_low` "is LOW because of the attitude"), and
// `settle` became plain locomotion (its own entry calls the drop "the whole
// read").
//
// That was never a re-tune and never a deletion. The constants in `ATTITUDES`
// are unchanged, and the thing that absorbs them is a knee bend -- limb work,
// which `attitudeFor` has said belongs with the clip set since #430, and which
// `rig3d/crouch.ts` now supplies. A crouching pose folds the thigh, knee and
// ankle so the pelvis settles by exactly the authored drop while the soles stay
// on the turf, measured through the real geometry at under a tenth of a
// millimetre of residue.
//
// THE FLOOR BELOW IS STILL LOAD-BEARING, and is not made redundant by that.
// The two write to the same channel from opposite ends and mean different
// things: this floor removes a root translation that would drag the feet down,
// and `crouch.ts` adds one that the limbs have already paid for. Delete the
// floor and the authored drop arrives TWICE -- once as the crouch's own,
// grounded, `move.root`, and once as an ungrounded overlay on top of it, which
// is the original sink with a knee bend in front of it.
//
// The ground clearance a downward root translation may spend, in metres of
// world travel. Zero for RIG_MEDIUM, and not a placeholder: the rig plants at
// the plane with a millimetre to spare on the wrong side (see above). It is a
// named constant with a measured spec rather than an inlined `0` so that a rig
// which ever DOES stand clear of the turf -- a thicker sole, a higher ankle --
// raises it deliberately, with the measurement in front of whoever does it.
//
// Deliberately NOT the clearance of a MOVING character. The walk and run clips
// float the whole figure well above the plane for the entire cycle (measured
// over 24 phases: never below +14 mm walking or +21 mm running, peaking at
// +137 mm mid-run -- pinned as bounds in `ground_contact.spec.ts`), so
// a per-frame budget read off the posed rig would let a settle sink its full
// 7 cm at one phase of the stride and none of it at another -- a 5 cm vertical
// wobble at stride frequency, added to poses whose whole job is to read as a
// held posture. A standing figure is the bound because these poses can and do
// occur at a standstill.
const GROUND_CLEARANCE_METRES = 0;

// One rig, one motion_scale -- the same single-rig assumption `metres()` above
// already rests on (`proportions.ts`: "there is one rig here").
//
// THE COUPLING, NAMED SO IT IS NOT A SURPRISE LATER. This is read from
// `RIG_MEDIUM` at module load, while `skeleton.apply` divides by
// `rig.style.motion_scale` from the `Rig` it is handed. They agree today
// because there is exactly one `RigProportions` and one construction site, and
// they would silently disagree the moment a second rig or a per-species
// motion_scale appears: the floor would then be expressed in one rig's units
// and applied in another's, and the error would be the ratio between them --
// a wrong-by-a-few-millimetres bug, which is the hardest kind to see here.
// `action_pose.ts` has no access to the real `Rig` (the overlay is authored
// before a rig is chosen), so fixing it means passing the scale in, not
// reading a different constant. Deliberately not done now: a second rig is the
// event that should force it, and doing it early would add a parameter to
// every caller for a case that does not exist.
const MOTION_SCALE = proportions.RIG_MEDIUM.motion_scale;

// The only bone the floor applies to, and the only bone this file's overlays
// ever translate. It is not a shorthand for "the first bone": `root` is the
// one bone whose translation is world-space, because it has no parent to
// re-orient it. A -Y translation on any other bone is down the PARENT's axes,
// which is not down the pitch, so flooring it would be arithmetic on two
// different quantities.
const ROOT = "root";

/**
 * One root translation's vertical component, floored so it cannot drive the
 * feet through the pitch. In `move` units (metres BEFORE `skeleton.apply`
 * multiplies by the rig's motion_scale), which is why the budget is divided by
 * it here.
 *
 * Upward travel is returned untouched: an aerial lift, a knockback, a save and
 * a stumble all mean to leave the ground.
 *
 * `clearanceMetres` is a parameter only so the spec can exercise the
 * conversion with a non-zero budget; every caller uses the rig's own.
 */
export function groundedRootY(
  y: number,
  clearanceMetres: number = GROUND_CLEARANCE_METRES,
): number {
  if (y >= 0) {
    return y;
  }
  // `0 - x` rather than `-x`: at a zero budget the unary form yields -0, which
  // `Object.is` (and therefore `toBe`) distinguishes from the 0 every other
  // path produces.
  const floor = 0 - clearanceMetres / MOTION_SCALE;
  return Math.max(y, floor);
}

interface SaveSpec {
  readonly angle: number;
  readonly travel: number;
  readonly floor?: number;
  readonly fixed?: number;
}

// Keeper saves reuse one body under bounded transforms, so the families stay
// distinguishable: spread stays compact, central corrects a short distance,
// stretch holds the full lunge, and a one-shot tip reaches just past it.
const SAVES: Readonly<Record<string, SaveSpec>> = {
  keeper_spread: { angle: 28, travel: 0.65 },
  keeper_central: { angle: 48, travel: 0.95 },
  keeper_stretch: { angle: 78, travel: 1.9, floor: 0.82 },
  keeper_tip: { angle: 84, travel: 2.2, fixed: 1 },
  keeper_dive: { angle: 72, travel: 1.6 },
};

function clamp(x: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, x));
}

function empty(): { rot: Record<string, EulerTriple>; move: Record<string, EulerTriple> } {
  return { rot: {}, move: {} };
}

// Which way a dive leans, in the character's own frame.
//
// The 2.5D renderer collapsed this to a screen-space sign, which is right
// often enough for a keeper facing up the pitch and wrong when they are not.
// On the rig the honest answer is available: project the dive direction onto
// the character's own left, which is +X once the draw-time yaw is applied.
//
// Returns +1 when the dive goes to the keeper's LEFT (toward +X), -1 to their
// right, and 0 when there is nothing to lean along.
//
// ZERO IS A REAL ANSWER, NOT A FAILURE (#451). Callers must not read it as
// "no pose": `gc-sim`'s `launch_dive` leaves `dive_dir` as the ZERO VECTOR
// whenever the intercept is under a pixel from where the keeper already
// stands, and this function then correctly reports that a save with no
// lateral component has no lateral component. That is a save the keeper takes
// at the BODY, and `bodySave` below is what it looks like. Treating the zero
// as "skip the overlay" is what left a keeper standing upright while the ball
// was saved beside them.
function lateralSign(diveDir: XY | undefined, facing: XY | undefined): number {
  if (!diveDir) {
    return 0;
  }
  // With no facing the pitch axes are the character's own, so their left is
  // pitch +x. Matches the 2.5D fallback of leaning along the dominant axis.
  const fx = facing ? facing.x : 1;
  const fy = facing ? facing.y : 0;
  // Character's local +X in pitch coordinates is (fy, -fx).
  const alongLeft = diveDir.x * fy - diveDir.y * fx;
  if (Math.abs(alongLeft) < 1e-6) {
    return 0;
  }
  return alongLeft > 0 ? 1 : -1;
}

// A save with NO lateral component: the keeper takes it at the body (#451).
//
// A near-straight shot leaves `gc-sim`'s `launch_dive` with `dive_dir` at the
// zero vector, because the point where the shot crosses the keeper's line is
// under a pixel from where the keeper already stands. There is genuinely no
// side to throw the body to, and inventing one would draw a lateral dive for a
// ball arriving down the middle -- a different wrong picture, which is why
// this is a pose rather than a sign.
//
// WHAT A KEEPER ACTUALLY DOES with a ball arriving at them is get behind it:
// the chest comes over the ball and the body commits FORWARD, down the line
// the shot is travelling, rather than sideways off it. So the transform is the
// lateral one turned onto the other axis -- `rot.x` forward instead of `rot.z`
// to the side (see ORIENTATION at the top of this file), and travel along
// `move.z`, the character's own facing, instead of across it. For a keeper
// that facing is the goal-line normal, which `gc_render::frame`'s
// `drawn_facing` publishes precisely so a leaning keeper has a stable
// reference (#449) -- so forward here is up the pitch, into the shot.
//
// THE MAGNITUDES ARE THE FAMILY'S OWN, deliberately not new constants. Each
// `SaveSpec` says how far THAT save commits the body; the lateral sign only
// ever answered which way. Reusing `angle` and `travel` keeps spread, central
// and dive as far apart from each other here as they are laterally -- pinned
// in `action_pose.spec.ts` on both axes -- and means a re-tune of the save
// families carries to the body save for free instead of drifting from it.
//
// `keeper_stretch` and `keeper_tip` reach this in principle and not in play: a
// full stretch is CLASSIFIED by a large lateral distance (`gc_sim::keeper`'s
// `save_style`), and a tip's `dive_dir` is synthesised as a unit lateral
// vector by `gc_render::frame` rather than read from the simulation. They are
// covered because the branch is per-family, not because they are expected.
//
// NO VERTICAL COMPONENT HERE, for exactly the reason the lateral branch has
// none: how far this pitch has to rise to keep a toe out of the turf depends
// on where the stance clip put the limbs, and `rig3d/ground.ts` measures that
// off the posed rig. It is the same lift, on a rotation about a different
// axis, and it needs no new code there -- `poseAndGround` scans the posed rig,
// not the overlay.
function bodySave(spec: SaveSpec, amount: number): RootPose {
  const pose = empty();
  // Positive x tips the body forward onto its face: over the ball, not off it.
  pose.rot.root = [spec.angle * amount, 0, 0];
  pose.move.root = [0, 0, metres(spec.travel * amount)];
  return pose;
}

// The keeper save families, plus the un-posed dive the renderer falls back to
// when no pose id is supplied.
function save(poseId: string | undefined, opts: ActionPoseOptions): RootPose | null {
  let spec = SAVES[poseId ?? ""];
  if (!spec && !(poseId === undefined && (opts.dive ?? 0) > 0)) {
    return null;
  }
  spec = spec ?? SAVES.keeper_dive;
  if (!spec || !opts.dive_dir) {
    return null;
  }

  let amount = spec.fixed ?? clamp(opts.dive ?? 0, 0, 1);
  if (spec.floor !== undefined) {
    amount = Math.max(amount, spec.floor);
  }
  const sign = lateralSign(opts.dive_dir, opts.facing);
  if (sign === 0) {
    return bodySave(spec, amount);
  }

  const pose = empty();
  // Head toward the dive side: +X needs a negative z (see the note above).
  pose.rot.root = [0, 0, -sign * spec.angle * amount];
  // LATERAL ONLY, AND THAT IS DELIBERATE (#446). A dive has a vertical
  // component too -- a keeper leaves the ground -- and it is NOT authored
  // here, because how far this roll has to rise to keep a limb out of the turf
  // depends on where the stance clip has put the limbs, which this file cannot
  // see. `rig3d/ground.ts` measures it off the posed rig instead. Writing a
  // `spec.lift` next to `spec.travel` would be five hand-tuned numbers that
  // silently stop matching the moment a keeper's clip changes.
  pose.move.root = [metres(sign * spec.travel * amount), 0, 0];
  return pose;
}

// Aerials use the ground point for sorting and the shadow but lift the body.
// A bicycle also rotates it back into a readable overhead silhouette; the
// other styles are a lift with the limbs posed by their own clip.
function aerial(poseId: string | undefined, opts: ActionPoseOptions): RootPose | null {
  const isAerial =
    poseId === "aerial_bicycle" ||
    poseId === "aerial_action" ||
    (poseId === undefined && (opts.aerial ?? 0) > 0);
  if (!(isAerial && opts.aerial_style)) {
    return null;
  }

  const amount = clamp(opts.aerial ?? 0, 0, 1);
  const lift = (0.35 + 1.65 * (opts.aerial_jump ?? 0)) * amount;
  const pose = empty();
  pose.move.root = [0, metres(lift), 0];
  if (opts.aerial_style === "bicycle") {
    // Over backwards, which is negative x: the head goes behind the hips.
    pose.rot.root = [-78 * amount, 0, 0];
  }
  return pose;
}

interface TipSpec {
  readonly pitch: number;
  readonly lift?: number;
  readonly drop?: number;
}

// Reactions and recoveries, all of them a tip about the character's own
// left-right axis. The angles are what separate them at a glance, so they are
// listed together rather than spread through the file.
const TIPS: Readonly<Record<string, TipSpec>> = {
  // Driven off their feet, away from whatever hit them.
  combat_knockback: { pitch: -68, lift: 0.45 },
  // A rocked-back beat, deliberately shallow so it never reads as knockback.
  combat_stagger: { pitch: -8, drop: 0.28 },
  // Back onto the feet after a save: shallower still, and to the dive side.
  // Its own tilt is `GET_UP_TILT` below rather than `pitch`, because which
  // AXIS it tilts about depends on which way the keeper went down.
  keeper_get_up: { pitch: 0, drop: 0.18 },
};

// How far a keeper is still tipped while pushing back up off the floor. One
// magnitude, two axes: rolled to the side they dived to, or pitched forward
// when they went down at the body and there was no side (#451).
const GET_UP_TILT = 16;

function tip(poseId: string | undefined, opts: ActionPoseOptions): RootPose | null {
  // A failed challenge tips the body away from the direction it committed to.
  // Steeper than a combat stagger and pivoted off the trailing heel, so the
  // two recoveries never read as the same thing.
  if (poseId === "stumble") {
    const pose = empty();
    pose.rot.root = [-24, 0, 0];
    pose.move.root = [0, metres(0.12), metres(-0.35)];
    return pose;
  }

  const spec = TIPS[poseId ?? ""];
  if (!spec) {
    return null;
  }

  const pose = empty();
  if (poseId === "keeper_get_up") {
    // Still leaning on the hand they landed on, so this keeps the dive side.
    // A keeper who went down with no lateral component landed on their FRONT,
    // so the same tilt is a forward one -- the `bodySave` argument, one beat
    // later and at the recovery's own shallower magnitude (#451). Without it
    // this pose resolved to a zero rotation and a drop the rig grounds away,
    // which is to say to nothing at all: a keeper snapping upright the instant
    // a body save ended.
    const sign = lateralSign(opts.dive_dir, opts.facing);
    pose.rot.root = sign === 0 ? [GET_UP_TILT, 0, 0] : [0, 0, -sign * GET_UP_TILT];
  } else {
    pose.rot.root = [spec.pitch, 0, 0];
  }
  const dy = (spec.lift ?? 0) - (spec.drop ?? 0);
  if (dy !== 0) {
    pose.move.root = [0, metres(dy), 0];
  }
  return pose;
}

// ---------------------------------------------------------------------------
// HIT-REACTION ENVELOPE (#564)
// ---------------------------------------------------------------------------
//
// THE BUG THIS REPLACES. `TIPS.combat_knockback`/`combat_stagger` above are a
// single STATIC tilt, held unchanged for the whole forced window and then
// gone in one frame the instant `forced_state` clears -- a hit reads as "the
// victim braces once, then is back up immediately", because nothing about
// the pose moves for the ~0.5s (<=30 ticks) `gc_sim` actually holds control
// away from them. `hitReactionMultiplier` below is what makes it move: a
// fast attack that slightly OVERSHOOTS the authored tilt (an impact, not a
// pose fading in), a settle to a REDUCED hold (absorbing the hit, not still
// reeling at the peak), and a recovery that ramps back toward upright over
// the window's OWN LAST FEW TICKS -- so the character is visibly getting up
// before control returns, not snapping upright the instant it does.
//
// TWO SEPARATE MECHANISMS, DELIBERATELY:
//
//   1. THE ENVELOPE ([`hitReactionMultiplier`]) scales `TIPS`' authored
//      magnitude while `forced_state` is active. Pure and stateless on its
//      own -- it takes `elapsedTicks`/`remainingTicks` as plain numbers so
//      its SHAPE is testable independent of how a caller tracks them.
//   2. THE LATCH AND THE BLEND-OUT (`hitReactionStates`, a per-slot `Map`)
//      are what a caller needs to find `elapsedTicks` in the first place --
//      `forced_ticks` only ever reports what remains, not the window's
//      original length -- and to fade the ROOT OVERLAY out over
//      `HIT_BLEND_OUT_SECONDS` once `forced_state` clears, instead of
//      letting it disappear in the one frame `forOptions` stops returning
//      it. This is the same shape `rig3d/animator.ts`'s own `states` map
//      keeps per character (see that file's `AnimatorState`) -- a plain
//      module-level `Map<string, ...>` keyed by a caller-supplied slot id,
//      because AGENTS.md's "no global mutable state" rule is a Rust rule
//      (`unsafe_code = "forbid"`; there is no such restriction here) and
//      because presentation-derived per-character state belongs in
//      `@gc/render`, not in the simulation this module reads from.
//
// WHY THIS DOES NOT TOUCH `forOptions`/`tip`. Both stay exactly as authored:
// the envelope is applied to their OUTPUT, in `apply`, rather than threaded
// into their signatures. That keeps every existing caller of `forOptions`
// (`ground_contact.spec.ts`'s exact-equality assertions against `TIPS`'s
// static geometry among them) reading the unscaled, authored pose, which is
// still the right thing for them to read: they are pinning the AUTHORED
// contract, not the runtime envelope on top of it.
//
// TODAY'S REACHABILITY, STATED RATHER THAN LEFT IMPLICIT. `apply`'s two new
// parameters (`slotId`, `now`) are OPTIONAL and every call site but
// `rig3d/animator.ts`'s omits them, so every one of them keeps today's exact
// static/instant behaviour -- see `apply`'s own doc. Where they ARE supplied
// (`animator.ts`, the only production call site), the curve is LIVE TODAY:
// `ActionPoseOptions.forced_ticks` is not yet populated by any production
// caller (`RenderFrame` does not carry it -- see that field's own doc), so
// `hitReaction` takes its WALL-CLOCK FALLBACK branch rather than its exact
// one, latching `now` and assuming `MAX_DISABLE_TICKS` as the window's
// length. See `hitReaction`'s own doc for the fallback's shape and why an
// early real recovery (a shorter window than the assumed cap) degrades
// gracefully through the same blend-out an exact, on-time one uses, rather
// than reading as a second snap. The EXACT path stays preferred and takes
// over the instant `forced_ticks` is threaded from a live `RenderFrame`,
// with no further change to this file.

/** Ticks the attack ramp takes to reach its overshoot peak. */
const HIT_ATTACK_TICKS = 4;
/** Ticks the peak then takes to settle down to the held magnitude. */
const HIT_SETTLE_TICKS = 3;
/** Ticks before the forced window ends that recovery starts -- the window
 * the character is visibly getting up in, before control returns. */
const HIT_RECOVER_TICKS = 8;
/** The attack's overshoot, as a multiple of `TIPS`' authored static tilt. */
const HIT_PEAK_MULT = 1.15;
/** What the overshoot settles to and holds at until recovery starts -- LESS
 * than the authored static magnitude, which is the "reduced hold" the task
 * asks for. */
const HIT_HOLD_MULT = 0.75;
/** How long the root overlay keeps fading out the LAST forced pose shown
 * after `forced_state` clears, instead of snapping to whatever the next
 * pose (usually locomotion) resolves to. */
const HIT_BLEND_OUT_SECONDS = 0.1;

/**
 * The hit-reaction envelope's shape, as a multiplier on `TIPS`' authored
 * static magnitude. Pure and stateless: `elapsedTicks`/`remainingTicks` are
 * plain numbers so the curve itself is testable independent of
 * `hitReactionStates`' latching below.
 *
 * THE RECOVER CHECK COMES FIRST, and that ordering is load-bearing rather
 * than incidental: a forced window shorter than
 * `HIT_ATTACK_TICKS + HIT_SETTLE_TICKS + HIT_RECOVER_TICKS` has no room for
 * a hold plateau at all, and checking `remainingTicks` first means a short
 * stagger recovers immediately -- proportionally, from wherever the attack
 * ramp would have left it -- rather than demanding attack/settle ticks the
 * window does not have before it is allowed to start recovering.
 */
export function hitReactionMultiplier(elapsedTicks: number, remainingTicks: number): number {
  const remaining = Math.max(0, remainingTicks);
  if (remaining <= HIT_RECOVER_TICKS) {
    return HIT_HOLD_MULT * clamp(remaining / HIT_RECOVER_TICKS, 0, 1);
  }
  const elapsed = Math.max(0, elapsedTicks);
  if (elapsed < HIT_ATTACK_TICKS) {
    return HIT_PEAK_MULT * (elapsed / HIT_ATTACK_TICKS);
  }
  const settling = elapsed - HIT_ATTACK_TICKS;
  if (settling < HIT_SETTLE_TICKS) {
    return HIT_PEAK_MULT + (HIT_HOLD_MULT - HIT_PEAK_MULT) * (settling / HIT_SETTLE_TICKS);
  }
  return HIT_HOLD_MULT;
}

/** Ticks per second the sim runs at. Used only by the wall-clock fallback
 * below (see `hitReaction`'s own doc) to convert elapsed real time into an
 * assumed tick count -- everywhere else in this file that cares about ticks
 * reads them straight from the sim via `ActionPoseOptions.forced_ticks`. */
const HIT_FALLBACK_TICKS_PER_SECOND = 60;
/** The forced window length the wall-clock fallback assumes: not any one
 * hit's actual duration (which the fallback has no way to know), but
 * `gc_sim::combat::MAX_DISABLE_TICKS` -- the CAP every real window is at
 * most as long as. See `hitReaction`'s own doc for why assuming the cap,
 * rather than guessing shorter, is the right bias. */
const HIT_FALLBACK_ASSUMED_TOTAL_TICKS = 30;

/** One character's hit-reaction latch/blend-out bookkeeping. */
interface HitReactionSlotState {
  /** The forced pose id this latch belongs to. A fresh forced pose -- either
   * the other one (stagger after a knockback or vice versa) or the same one
   * starting a new window after a full clear -- is detected by this
   * changing (together with `phase`, see below), which is what tells
   * `hitReaction` to re-latch outright rather than continue an old window. */
  poseId: "combat_knockback" | "combat_stagger";
  /** Whether this slot is currently SHOWING a forced pose or BLENDING OUT
   * of one. This is what makes `poseId` alone insufficient to decide
   * "continuing": without it, a fresh hit of the SAME forced pose id
   * arriving while an earlier hit's blend-out is still in flight would read
   * as a continuation of the OLD (already-finished) window and inherit its
   * stale latch, rather than starting its own ramp at zero. `"forced"` is
   * set every time the forced branch below runs; `"blending"` is set every
   * time the blend-out branch runs; `continuing` in `hitReaction` requires
   * BOTH the pose id to match AND `phase` to still be `"forced"`. */
  phase: "forced" | "blending";
  /** EXACT-TICKS LATCH (the preferred branch -- see `ActionPoseOptions
   * .forced_ticks`'s own doc): `forced_ticks` the FIRST frame this window
   * was observed at this magnitude or higher. `elapsedTicks` is derived
   * from it each frame (`latchedTotalTicks - forced_ticks`) rather than
   * counted independently, so a frame this module never saw (a dropped
   * frame, a rollback re-seed) cannot desync the two. Re-latched upward
   * rather than dropped if a later frame reports MORE remaining ticks than
   * the latch (a chained hit extending the same window), so elapsed never
   * goes negative. `undefined` while this slot is running the wall-clock
   * fallback instead (the two latches are mutually exclusive: a slot is in
   * exactly one mode per window, decided by whether `forced_ticks` was live
   * the frame the window was first observed). */
  latchedTotalTicks: number | undefined;
  /** WALL-CLOCK FALLBACK LATCH: `now` the FIRST frame this window was
   * observed with no live `forced_ticks` -- see `hitReaction`'s own doc.
   * `undefined` while this slot is running the exact-ticks latch instead. */
  latchedAt: number | undefined;
  /** The last root overlay this slot actually showed while forced, in
   * authoring units (degrees/metres) -- what a blend-out fades FROM once
   * `forced_state` clears. */
  lastForcedPose: RootPose;
  /** Wall time (seconds) the blend-out began, or `undefined` while this
   * slot is not currently blending out. */
  blendStartedAt: number | undefined;
}

const hitReactionStates = new Map<string, HitReactionSlotState>();

/**
 * Drops every character's hit-reaction latch and blend-out state.
 *
 * Not wired to any call site inside this package -- there is no scene-
 * discontinuity hook within this file's scope to wire it to (see
 * `rig3d/animator.ts`'s own `reset`, which the same caller that resets
 * animator state should call this alongside). Left un-called rather than
 * silently omitted is deliberate here: the cost of not calling it is a
 * self-correcting one (the next hit re-latches this slot outright), not a
 * lingering incorrect state.
 */
export function resetHitReactions(): void {
  hitReactionStates.clear();
}

function isForcedReactionPose(id: string | undefined): id is "combat_knockback" | "combat_stagger" {
  return id === "combat_knockback" || id === "combat_stagger";
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

// Euler-linear rather than slerped: the blend-out window is 100 ms and the
// angles involved are the same small magnitudes `TIPS` already authors, so
// the difference from a true slerp is not visible, and every other root
// transform in this file is already authored and composed in euler space
// (see `apply`'s own ASSIGN FOR AN ACTION / COMPOSE FOR AN ATTITUDE note).
// `null` reads as the zero transform, matching how `apply` already treats
// "no action pose" for the branch this feeds.
function blendRootPose(from: RootPose, to: RootPose | null, t: number): RootPose {
  const fr = from.rot.root ?? [0, 0, 0];
  const fm = from.move.root ?? [0, 0, 0];
  const tr = to?.rot.root ?? [0, 0, 0];
  const tm = to?.move.root ?? [0, 0, 0];
  return {
    rot: { root: [lerp(fr[0], tr[0], t), lerp(fr[1], tr[1], t), lerp(fr[2], tr[2], t)] },
    move: { root: [lerp(fm[0], tm[0], t), lerp(fm[1], tm[1], t), lerp(fm[2], tm[2], t)] },
  };
}

// Scales a root overlay's rotation and translation uniformly. Only ever
// called on `combat_knockback`/`combat_stagger`'s own output, whose `TIPS`
// entries author exactly one nonzero rotation axis and one nonzero
// translation axis (see `tip`), so scaling every component uniformly is
// scaling exactly the two authored numbers -- there is no third axis here
// for a uniform scale to corrupt.
function scaleRootPose(root: RootPose, mult: number): RootPose {
  const r = root.rot.root ?? [0, 0, 0];
  const m = root.move.root ?? [0, 0, 0];
  return {
    rot: { root: [r[0] * mult, r[1] * mult, r[2] * mult] },
    move: { root: [m[0] * mult, m[1] * mult, m[2] * mult] },
  };
}

/**
 * The hit-reaction seam `apply` calls into: shapes `action` while a forced
 * reaction pose is active, and blends the root overlay out over
 * `HIT_BLEND_OUT_SECONDS` for the frames right after it clears. Returns
 * `action` completely unchanged for every other pose id, so a player who
 * was never staggered or knocked back is not touched by any of this.
 *
 * TWO WAYS TO FIND `elapsedTicks`/`remainingTicks`, one preferred:
 *
 *   1. EXACT (`ActionPoseOptions.forced_ticks` live this frame): the sim's
 *      own remaining-tick count, latched and differenced exactly as before.
 *      Always used when available -- see that field's own doc for why a
 *      future wire extension needs no change here to take over.
 *   2. WALL-CLOCK FALLBACK (`forced_ticks` undefined -- today's actual
 *      production reachability, since `RenderFrame` does not carry it yet):
 *      latches `now` the first frame this slot shows `poseId`, and reads
 *      elapsed real time since, converted to ticks at
 *      `HIT_FALLBACK_TICKS_PER_SECOND` against an ASSUMED total of
 *      `HIT_FALLBACK_ASSUMED_TOTAL_TICKS` (the cap, `MAX_DISABLE_TICKS` --
 *      not any one hit's real, possibly shorter, length, which this branch
 *      has no way to know). Chosen deliberately over a shorter guess:
 *      assuming the cap means a real, shorter window is caught by the pose
 *      id reverting -- exactly what the BLEND-OUT branch below already
 *      handles -- rather than the curve finishing recovery early and
 *      holding upright while the player is still actually staggered.
 *
 *      THIS IS WHY THE APPROXIMATION DEGRADES GRACEFULLY RATHER THAN
 *      VISIBLY: a real window shorter than the assumed 30 ticks clears
 *      while the fallback curve is still mid-attack/hold, and the moment it
 *      does, this function's OTHER branch takes over and blends the
 *      in-flight tilt out over `HIT_BLEND_OUT_SECONDS` -- the same 100 ms
 *      fade an exact, on-time recovery would have used anyway. A viewer
 *      never sees a snap; they see a hit that recovers somewhat sooner than
 *      the full curve, which is the least-wrong reading available without
 *      the sim's own tick count.
 */
function hitReaction(
  slotId: string,
  opts: ActionPoseOptions,
  action: RootPose | null,
  now: number,
): RootPose | null {
  const poseId = opts.pose?.id;

  if (isForcedReactionPose(poseId) && action !== null) {
    const previous = hitReactionStates.get(slotId);
    // Both must hold: the pose id matching alone is not enough while a
    // PREVIOUS window's blend-out is still in flight -- see `phase`'s own
    // doc on `HitReactionSlotState`.
    const continuing = previous?.poseId === poseId && previous.phase === "forced";
    const forcedTicks = opts.forced_ticks;

    let elapsedTicks: number;
    let remainingTicks: number;
    let latchedTotalTicks: number | undefined;
    let latchedAt: number | undefined;

    if (forcedTicks !== undefined && forcedTicks >= 0) {
      // EXACT PATH (preferred). `0` is deliberately accepted here, not
      // treated like "no data": it is a real, meaningful frame (the window
      // ending THIS tick), and `hitReactionMultiplier` already resolves it
      // to exactly zero -- see `ActionPoseOptions.forced_ticks`'s own doc
      // on why `0` and `undefined` must not be conflated.
      const baseline = continuing ? (previous?.latchedTotalTicks ?? forcedTicks) : forcedTicks;
      latchedTotalTicks = Math.max(baseline, forcedTicks);
      elapsedTicks = latchedTotalTicks - forcedTicks;
      remainingTicks = forcedTicks;
    } else {
      // WALL-CLOCK FALLBACK. See this function's own doc above.
      latchedAt = continuing && previous?.latchedAt !== undefined ? previous.latchedAt : now;
      elapsedTicks = Math.max(0, now - latchedAt) * HIT_FALLBACK_TICKS_PER_SECOND;
      remainingTicks = Math.max(0, HIT_FALLBACK_ASSUMED_TOTAL_TICKS - elapsedTicks);
    }

    const shaped = scaleRootPose(action, hitReactionMultiplier(elapsedTicks, remainingTicks));
    hitReactionStates.set(slotId, {
      poseId,
      phase: "forced",
      latchedTotalTicks,
      latchedAt,
      lastForcedPose: shaped,
      blendStartedAt: undefined,
    });
    return shaped;
  }

  // Not (or no longer) a forced reaction pose this frame. Blend out of the
  // last one shown, if any, instead of snapping straight to whatever this
  // frame resolved to. Reached from BOTH latch modes above -- a real early
  // release under the wall-clock fallback lands here exactly like an
  // on-time exact-path recovery does.
  const state = hitReactionStates.get(slotId);
  if (state === undefined) {
    return action;
  }
  const startedAt = state.blendStartedAt ?? now;
  const t = clamp((now - startedAt) / HIT_BLEND_OUT_SECONDS, 0, 1);
  if (t >= 1) {
    hitReactionStates.delete(slotId);
    return action;
  }
  hitReactionStates.set(slotId, { ...state, phase: "blending", blendStartedAt: startedAt });
  return blendRootPose(state.lastForcedPose, action, t);
}

interface AttitudeSpec {
  /** Whole-figure lean in player radii: positive FORWARD, negative back. */
  readonly lean?: number;
  /** Crouch in player radii: how far the body settles toward the ground. */
  readonly drop?: number;
}

/**
 * Held body attitudes, in the 2.5D renderer's own units (#430).
 *
 * These are the `actionLean` / `stanceDrop` constants the billboard deleted in
 * #418 carried, recovered verbatim from
 * `git show 0333065^:v2/ts/packages/render/src/player_renderer.ts`. Each one is
 * a posture the simulation selects every match and that no renderer has shown
 * since: a containing defender, a tiring player, a first touch taken low, a
 * telegraphed break, a shot's follow-through. The pose ids were reaching the
 * renderer the whole time -- #428 observed `kick_follow` live on hardware GL --
 * and rendering as a plain jog.
 *
 * SIGN, which is the easy thing to get backwards. The billboard wrote
 * `actionLean = fx * r * k` with `fx = opts.facing?.x`, because it drew in
 * SCREEN space and had to project "forward" onto the screen's x axis itself --
 * with the side effect that a player running straight up the pitch (fx = 0)
 * did not lean at all, the same billboard limitation `lateralSign` fixes for
 * dives. The rig is already yawed to `facing`, so forward is local +Z and a
 * forward lean is simply a positive `rot.x` (see the ORIENTATION note above).
 * No facing term survives the port, and the poses now read from every angle.
 *
 * WHAT IS DELIBERATELY NOT HERE. Two billboard details are limb work, not root
 * work, and are left out rather than approximated: `fatigue`'s extra `slump`
 * (0.24r, which lowered the shoulders and head WITHOUT the hips -- a rounded
 * spine, not a crouch) and `settle`'s widened stance (`hipDx` 0.74r). Both want
 * a clip or a limb overlay; this file is root-only by charter. `slide` is out
 * for the same reason and at greater cost: it is a whole-body ground action
 * that no root transform approximates, so it stays an honest gap in
 * `pose_table.ts` for #423/#424 rather than becoming a standing player at a
 * strange angle.
 */
const ATTITUDES: Readonly<Record<string, AttitudeSpec>> = {
  // A shot's follow-through: the mass carries on past the ball.
  kick_follow: { lean: 0.28 },
  // A first touch is taken at a run, but taken LOW -- the drop is the whole
  // read, and it is what separated a settle from plain running.
  settle: { drop: 0.3 },
  // A body committing before the feet do. The hardest forward lean here, and
  // more than a change of speed.
  run_telegraph: { lean: 0.5 },
  // Contain holds its weight BACK: it shepherds, it does not commit. The
  // negative lean is the entire point -- a forward stance would read as an
  // attacking challenge, which is the opposite instruction.
  contain: { lean: -0.3, drop: 0.26 },
  // A spent player sags: weight behind them, knees gone.
  //
  // THE WEAKEST ENTRY IN THIS TABLE, on three independent counts. Recorded
  // together because each one alone is acceptable and the three together are
  // not, and because the next person should not have to rediscover them:
  //
  //   1. WEAKEST READ. -0.12r is about 2 degrees of whole-body tilt. The
  //      billboard's fatigue read did not come from that at all -- it came
  //      from its `slump`, 0.24r lowering the SHOULDERS AND HEAD with the hips
  //      unmoved, which is a rounded spine and therefore limb work this
  //      root-only file cannot express. What is here is the part of the
  //      billboard's reading that ports; the part that carried it does not.
  //   2. NEVER SEEN LIVE. A hardware-GL run of `tools/browser_match_harness`
  //      at seed 7, ~18000 ticks, tallied every other pose id in this table --
  //      `contain` 1789 slot-frames, `settle` 1180, `kick_follow` 469 -- and
  //      observed `fatigue` ZERO times. So this entry is verified in tests and
  //      in a static pose gallery, and NOT in a real match. Whether
  //      `gc_render::player_pose::select` reaches it at ordinary match length
  //      is an open question, not a settled one.
  //   3. THINNEST TEST MARGIN. Its lean is the smallest quantity any pin here
  //      asserts. `action_pose.spec.ts` therefore pins fatigue's magnitude as
  //      an exact RATIO against `contain` rather than only against a floor, so
  //      a halving of it fails rather than passing on the crouch's coat-tails.
  //
  // If fatigue turns out to matter on screen, the fix is the slump (a spine
  // clip), not a bigger root tilt: inflating this number past what the
  // billboard had would be inventing, and the poses would stop being ordered
  // the way the renderer they came from ordered them.
  fatigue: { lean: -0.12, drop: 0.16 },
  // The crouch halves of the two keeper ready stances. #429 gave both their
  // braced arms (`guard_stance` over the upper body); this is the part of the
  // billboard's reading that was still missing, and it composes with those
  // arms rather than replacing them.
  keeper_set: { drop: 0.18 },
  keeper_ready_low: { drop: 0.32 },
};

/**
 * The held posture for one pose id, or null when the pose has no attitude.
 *
 * Kept out of `forOptions` on purpose -- see this file's TWO KINDS OF ROOT
 * TRANSFORM note. An attitude is not an action, and the caller that asks
 * `forOptions` whether to suppress the balance lean must keep getting `null`
 * for a player who is merely containing or tiring.
 */
export function attitudeFor(opts: ActionPoseOptions): RootPose | null {
  const spec = ATTITUDES[opts.pose?.id ?? ""];
  if (!spec) {
    return null;
  }
  const pose = empty();
  const lean = spec.lean ?? 0;
  if (lean !== 0) {
    // Displacement at LEAN_REFERENCE_HEIGHT, turned into the root tilt that
    // produces it. Positive x tips forward onto the face.
    const displacement = metres(lean);
    const radians = Math.asin(clamp(displacement / LEAN_REFERENCE_HEIGHT, -1, 1));
    pose.rot.root = [(radians * 180) / Math.PI, 0, 0];
  }
  const drop = spec.drop ?? 0;
  if (drop !== 0) {
    // The billboard raised hipY/shoulderY/headY and left footY alone, so its
    // crouch compressed the legs. The rig has no IK to fold a leg under a
    // dropped pelvis, so the whole root settles instead and the feet settle
    // with it -- the WHOLE WAY into the turf at these magnitudes, because the
    // rig has no ground clearance to spend. The largest here,
    // `keeper_ready_low`, is 0.084 m before the rig's own motion_scale and
    // 0.074 m after it, against a 1.57 m figure: 4.7% of the character's
    // height, feet and ankles below the pitch.
    //
    // This comment used to call that "a few millimetres into the turf" in the
    // same sentence that quoted 0.084 m -- the number computed correctly and
    // the size of it mis-stated by a factor of twenty, which is why half of
    // #439 stayed invisible after being written down. The authored magnitude
    // is unchanged and correct; what was missing was the ground contact, and
    // `apply` floors this against the rig's clearance (see GROUND CONTACT
    // above).
    //
    // WHAT MAKES THE MAGNITUDE LIVE AGAIN is `rig3d/crouch.ts` (#445), which
    // reads the same number through `attitudeDrop` below and folds the legs
    // under it. So this branch keeps authoring the geometry the billboard had,
    // `apply` keeps refusing to let a root translation deliver it, and the
    // crouch layer delivers it as limb work -- one magnitude, authored once,
    // in this table.
    pose.move.root = [0, -metres(drop), 0];
  }
  return pose;
}

/**
 * How far one pose id's attitude settles the body, in `move` units (metres
 * BEFORE `skeleton.apply` multiplies by motion_scale). Zero for every pose that
 * authors no crouch.
 *
 * WHY THIS EXISTS SEPARATELY FROM `attitudeFor` (#445). The drop is the ONE
 * quantity in this table that a root translation cannot deliver: it moves the
 * feet with the body, so `apply` floors it to the rig's ground clearance, which
 * is zero. `rig3d/crouch.ts` delivers it instead, as a knee bend that lowers the
 * pelvis by exactly this while the soles stay on the turf, and `animator.ts`
 * reads it from here so that `ATTITUDES` stays the single place the magnitude
 * is authored. Positive means down, which is the sign a caller sizing a crouch
 * wants; `attitudeFor` keeps reporting the same number as a negative `move.root`
 * y, because that is the geometry it is authored as.
 */
export function attitudeDrop(opts: ActionPoseOptions): number {
  return metres(ATTITUDES[opts.pose?.id ?? ""]?.drop ?? 0);
}

/**
 * The deepest crouch any attitude authors, in `move` units.
 *
 * Derived from the table rather than written down, so it cannot fall out of
 * step with it. `animator.ts` uses it to set the rate a crouch eases in at, so
 * that the deepest one takes the stated time and shallower ones take
 * proportionally less -- the same linear-ramp reasoning `advanceCrossfade`
 * gives for stance weights.
 */
export const DEEPEST_ATTITUDE_DROP: number = Object.values(ATTITUDES).reduce(
  (deepest, spec) => Math.max(deepest, metres(spec.drop ?? 0)),
  0,
);

// The root overlay for one player's action, or null when they are simply
// running and the locomotion blend is the whole story.
//
// Order matters: a keeper diving with the ball is diving first, and an aerial
// beats a reaction, exactly as the 2.5D renderer's early returns had it.
//
// DELIBERATELY EXCLUDES `attitudeFor`. This is also the predicate
// `player_renderer_3d.ts` uses for "a whole-body action owns the body, so the
// velocity-derived balance lean would be a second reading of the same motion".
// A held attitude is the opposite case: the player is still running and still
// balancing, so `contain`, `fatigue`, `run_telegraph`, `kick_follow` and
// `settle` must keep their lean. See TWO KINDS OF ROOT TRANSFORM above.
export function forOptions(opts: ActionPoseOptions): RootPose | null {
  const poseId = opts.pose?.id;
  return save(poseId, opts) ?? aerial(poseId, opts) ?? tip(poseId, opts);
}

// Merges the overlay into an already-resolved pose, mutating and returning it.
//
// Kept separate from `forOptions` so the geometry stays pure and testable in
// the authoring format (degrees, as clips.ts uses), while this side owns the
// conversion into the quaternions skeleton.apply consumes.
//
// The overlay only ever touches `root`, so it composes with the gait rather
// than replacing it: a keeper who dives mid-stride keeps the stride.
//
// ASSIGN FOR AN ACTION, COMPOSE FOR AN ATTITUDE. An action is the whole story
// of what the body is doing, so it assigns -- a dive at 72 degrees is not 72
// degrees ON TOP OF anything. An attitude is a modifier on a body that is
// otherwise running normally, so it composes, and the difference is visible:
// the locomotion clips write the run's vertical bob to `move.root`, and a
// crouch that assigned there would flatten the bob and leave a settling player
// gliding. Actions run first and return, so a pose id that ever appeared in
// both tables would be an action; today the two are disjoint.
//
// GROUNDED ON THE WAY THROUGH. Both branches below floor the ROOT's downward
// translation against the rig's ground clearance -- see the GROUND CONTACT
// section above for why this is the right place for it, why it is the overlay
// and not the sum that gets floored, and what it deliberately leaves alone.
// `forOptions` and `attitudeFor` keep returning the AUTHORED geometry, so the
// tables stay readable and testable in the units they are written in; this is
// the boundary where a pose becomes something a skeleton will actually be
// posed with, and therefore the boundary where the pitch exists.
//
// `slotId`/`now` ARE OPTIONAL, AND THAT IS THE COMPATIBILITY SEAM (#564).
// They exist only to drive the HIT-REACTION ENVELOPE section above --
// `rig3d/animator.ts`'s `poseFor` is the one production caller with both a
// stable per-character id and a wall clock in scope, and is the only call
// site that supplies them. Every other caller (this file's own spec,
// `ground_contact.spec.ts`'s exact-equality assertions against `forOptions`'
// authored geometry, `crouch.spec.ts`, `animator.spec.ts`) omits both and
// gets EXACTLY today's behaviour: `hitReaction` is never reached, `action`
// passes through `forOptions`'s output unchanged, and a forced pose still
// assigns its static `TIPS` tilt outright. Adding the envelope therefore
// changes no existing caller's result.
export function apply(
  pose: MutablePose,
  opts: ActionPoseOptions,
  slotId?: string,
  now?: number,
): MutablePose {
  let action = forOptions(opts);
  if (slotId !== undefined && now !== undefined) {
    action = hitReaction(slotId, opts, action, now);
  }
  if (action) {
    for (const [bone, r] of Object.entries(action.rot)) {
      pose.rot[bone] = quat.fromEuler(
        (r[0] * Math.PI) / 180,
        (r[1] * Math.PI) / 180,
        (r[2] * Math.PI) / 180,
      );
    }
    for (const [bone, m] of Object.entries(action.move)) {
      pose.move[bone] = bone === ROOT ? [m[0], groundedRootY(m[1]), m[2]] : m;
    }
    return pose;
  }

  const held = attitudeFor(opts);
  if (!held) {
    return pose;
  }
  for (const [bone, r] of Object.entries(held.rot)) {
    const q = quat.fromEuler(
      (r[0] * Math.PI) / 180,
      (r[1] * Math.PI) / 180,
      (r[2] * Math.PI) / 180,
    );
    const existing = pose.rot[bone];
    // Pre-multiplied, matching `player_renderer_3d.ts`'s `applyLean`: the
    // attitude is applied in the PARENT's frame, so it tips the whole rig
    // rather than being re-expressed inside whatever the clips resolved.
    pose.rot[bone] = existing !== undefined ? quat.multiply(q, existing) : q;
  }
  for (const [bone, m] of Object.entries(held.move)) {
    const existing = pose.move[bone] ?? [0, 0, 0];
    // The attitude's own contribution is what gets floored, and it is floored
    // BEFORE it is added: the gait's bob is the clip's business and survives
    // untouched, exactly as composition (rather than assignment) intends.
    const dy = bone === ROOT ? groundedRootY(m[1]) : m[1];
    pose.move[bone] = [
      (existing[0] ?? 0) + m[0],
      (existing[1] ?? 0) + dy,
      (existing[2] ?? 0) + m[2],
    ];
  }
  return pose;
}
