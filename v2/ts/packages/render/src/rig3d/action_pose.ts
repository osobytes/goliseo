// Whole-body action poses, as a sparse overlay on the rig root.
//
// These are the poses the 2.5D renderer expressed as transforms of the whole
// billboard rather than as limb animation: a keeper's dive, a bicycle kick, a
// knockback, a stumble. It is worth being precise about why they are ported as
// ROOT TRANSFORMS and not as clips.
//
// Every one of them is continuous. A dive is not a fixed silhouette played
// back; it is the body rotating through an angle as `dive` runs 0 -> 1 against
// the simulation's own timer. The same is true of the aerial lift, the wind-up
// and the tip. A keyframe clip would have to re-derive that ramp and would
// then disagree with the timer driving it, whereas a parameterised transform
// reads the timer directly -- which is what the 2.5D renderer did, and why its
// poses stayed in step with the sim.
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
// were written in, and converted once on the way out. That keeps the numbers
// directly comparable to the renderer they came from.
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
}

// One player radius, in metres. The rig is HEIGHT_IN_RADII * 2 radii tall by
// construction, so this is the conversion and it needs no camera state.
const RADII_PER_HEIGHT = 6.0;

// The same conversion, carried all the way into metres.
//
// `RADII_PER_HEIGHT` on its own turns radii into RIG HEIGHTS; the numbers that
// reach the skeleton are metres (`proportions.RigSegments` is metres, and
// `skeleton.apply` adds `pose.move` straight onto a bone offset). The existing
// SAVES/TIPS translations above divide by `RADII_PER_HEIGHT` alone and are
// therefore a factor of the rig's height short of the radii they are labelled
// with -- left exactly as they are, because re-scaling a dive is a re-tune of
// poses a visual pass has already signed off on and this issue is not that.
// The attitudes below use the honest conversion and say so.
//
// The conversion itself is `player_renderer_3d.ts`'s TORSO LEAN block, not a
// new claim: `ppmForRadius` sets ppm = r * HEIGHT_IN_RADII * 2 / height, so
// `k * r` on-screen pixels is `k * height / RADII_PER_HEIGHT` metres for any
// k and any r -- the `r` cancels, which is why a billboard constant written in
// projected pixels converts to rig metres with no camera state at all.
const RIG_HEIGHT_METRES = proportions.height(proportions.RIG_MEDIUM);
const METRES_PER_RADIUS = RIG_HEIGHT_METRES / RADII_PER_HEIGHT;

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
    return null;
  }

  const pose = empty();
  // Head toward the dive side: +X needs a negative z (see the note above).
  pose.rot.root = [0, 0, -sign * spec.angle * amount];
  pose.move.root = [(sign * spec.travel * amount) / RADII_PER_HEIGHT, 0, 0];
  return pose;
}

// Aerials use the ground point for sorting and the shadow but lift the body.
// A bicycle also rotates it back into a readable overhead silhouette; the
// other styles are a lift with the limbs posed by their own clip.
function aerial(poseId: string | undefined, opts: ActionPoseOptions): RootPose | null {
  const isAerial =
    poseId === "aerial_bicycle" || poseId === "aerial_action" || (poseId === undefined && (opts.aerial ?? 0) > 0);
  if (!(isAerial && opts.aerial_style)) {
    return null;
  }

  const amount = clamp(opts.aerial ?? 0, 0, 1);
  const lift = (0.35 + 1.65 * (opts.aerial_jump ?? 0)) * amount;
  const pose = empty();
  pose.move.root = [0, lift / RADII_PER_HEIGHT, 0];
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
  keeper_get_up: { pitch: 0, drop: 0.18 },
};

function tip(poseId: string | undefined, opts: ActionPoseOptions): RootPose | null {
  // A failed challenge tips the body away from the direction it committed to.
  // Steeper than a combat stagger and pivoted off the trailing heel, so the
  // two recoveries never read as the same thing.
  if (poseId === "stumble") {
    const pose = empty();
    pose.rot.root = [-24, 0, 0];
    pose.move.root = [0, 0.12 / RADII_PER_HEIGHT, -0.35 / RADII_PER_HEIGHT];
    return pose;
  }

  const spec = TIPS[poseId ?? ""];
  if (!spec) {
    return null;
  }

  const pose = empty();
  if (poseId === "keeper_get_up") {
    // Still leaning on the hand they landed on, so this keeps the dive side.
    const sign = lateralSign(opts.dive_dir, opts.facing);
    pose.rot.root = [0, 0, -sign * 16];
  } else {
    pose.rot.root = [spec.pitch, 0, 0];
  }
  const dy = (spec.lift ?? 0) - (spec.drop ?? 0);
  if (dy !== 0) {
    pose.move.root = [0, dy / RADII_PER_HEIGHT, 0];
  }
  return pose;
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
    const displacement = lean * METRES_PER_RADIUS;
    const radians = Math.asin(clamp(displacement / LEAN_REFERENCE_HEIGHT, -1, 1));
    pose.rot.root = [(radians * 180) / Math.PI, 0, 0];
  }
  const drop = spec.drop ?? 0;
  if (drop !== 0) {
    // The billboard raised hipY/shoulderY/headY and left footY alone, so its
    // crouch compressed the legs. The rig has no IK to fold a leg under a
    // dropped pelvis, so the whole root settles instead and the feet settle
    // with it -- a few millimetres into the turf at these magnitudes (the
    // largest here, `keeper_ready_low`, is 0.084 m before the rig's own
    // motion_scale, against a 1.57 m figure). Named rather than hidden: a
    // knee-bent crouch is limb work and belongs with the clip set.
    pose.move.root = [0, -drop * METRES_PER_RADIUS, 0];
  }
  return pose;
}

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
export function apply(pose: MutablePose, opts: ActionPoseOptions): MutablePose {
  const action = forOptions(opts);
  if (action) {
    for (const [bone, r] of Object.entries(action.rot)) {
      pose.rot[bone] = quat.fromEuler((r[0] * Math.PI) / 180, (r[1] * Math.PI) / 180, (r[2] * Math.PI) / 180);
    }
    for (const [bone, m] of Object.entries(action.move)) {
      pose.move[bone] = m;
    }
    return pose;
  }

  const held = attitudeFor(opts);
  if (!held) {
    return pose;
  }
  for (const [bone, r] of Object.entries(held.rot)) {
    const q = quat.fromEuler((r[0] * Math.PI) / 180, (r[1] * Math.PI) / 180, (r[2] * Math.PI) / 180);
    const existing = pose.rot[bone];
    // Pre-multiplied, matching `player_renderer_3d.ts`'s `applyLean`: the
    // attitude is applied in the PARENT's frame, so it tips the whole rig
    // rather than being re-expressed inside whatever the clips resolved.
    pose.rot[bone] = existing !== undefined ? quat.multiply(q, existing) : q;
  }
  for (const [bone, m] of Object.entries(held.move)) {
    const existing = pose.move[bone] ?? [0, 0, 0];
    pose.move[bone] = [(existing[0] ?? 0) + m[0], (existing[1] ?? 0) + m[1], (existing[2] ?? 0) + m[2]];
  }
  return pose;
}
