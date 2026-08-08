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

import { quat, type Quat } from "@gc/core";
import type { EulerTriple } from "./clips.ts";

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

// The root overlay for one player's action, or null when they are simply
// running and the locomotion blend is the whole story.
//
// Order matters: a keeper diving with the ball is diving first, and an aerial
// beats a reaction, exactly as the 2.5D renderer's early returns had it.
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
export function apply(pose: MutablePose, opts: ActionPoseOptions): MutablePose {
  const action = forOptions(opts);
  if (!action) {
    return pose;
  }
  for (const [bone, r] of Object.entries(action.rot)) {
    pose.rot[bone] = quat.fromEuler((r[0] * Math.PI) / 180, (r[1] * Math.PI) / 180, (r[2] * Math.PI) / 180);
  }
  for (const [bone, m] of Object.entries(action.move)) {
    pose.move[bone] = m;
  }
  return pose;
}
