// Keyframed animation clips, authored directly in TypeScript.
//
// A clip is a list of keyframes sorted by time. Each keyframe holds a *sparse*
// pose: `rot` gives bone rotations in degrees, `move` gives additive
// translations. A bone a clip never mentions stays at its rest transform.
//
// Sign conventions (the warrior faces +Z):
//   rot.x  The sign depends on which way the bone POINTS, which is the single
//          easiest thing to get wrong here:
//            * Limbs hang DOWN (-Y), so negative x swings them FORWARD.
//              A knee only bends backward, so shin.x stays positive; an elbow
//              only bends forward, so forearm.x stays negative.
//            * The spine chain points UP (+Y), so the sign INVERTS:
//              POSITIVE x on spine / chest / neck leans the torso FORWARD.
//          Verified by rendering, not assumed -- an early charge pose leaned
//          backwards for exactly this reason.
//   rot.y negative -> twists toward his right;  positive -> toward his left.
//   rot.z          -> splays a limb away from the body's midline.
//
// `loop` and `root_motion` are properties of the CLIP, not assumptions baked
// into the player. Every clip here is in-place: simulation owns position,
// facing and contact, per #101.
//
// Channels are interpolated independently and eased with smoothstep, which is
// enough to keep a 4-6 key clip from looking like straight-line lerp. Euler
// angles are interpolated component-wise: fine here because no joint passes
// through a gimbal-locking orientation.

import { quat, type Quat } from "@gc/core";
import type { Pose } from "./skeleton.ts";

/** A rotation or translation authored in the clip's Euler/metres convention. */
export type EulerTriple = readonly [number, number, number];

interface RawKeyframe {
  readonly t: number;
  readonly rot?: Readonly<Record<string, EulerTriple>>;
  readonly move?: Readonly<Record<string, EulerTriple>>;
}

interface RawClip {
  readonly name: string;
  readonly loop: boolean;
  readonly root_motion: false;
  readonly duration: number;
  readonly stride?: number;
  readonly fallback?: string;
  readonly keys: readonly RawKeyframe[];
}

interface PreparedKeyframe {
  readonly t: number;
  readonly q: Readonly<Record<string, Quat>>;
  readonly move: Readonly<Record<string, EulerTriple>>;
}

/** A clip ready to sample: keyframes baked to quaternions, channels indexed. */
export interface Clip {
  readonly name: string;
  readonly loop: boolean;
  readonly duration: number;
  readonly stride?: number;
  readonly fallback?: string;
  readonly keys: readonly PreparedKeyframe[];
  readonly rotBones: ReadonlySet<string>;
  readonly moveBones: ReadonlySet<string>;
}

// Shallow-merges `overrides` over `base` into a fresh object, so each keyframe
// can state only what differs from the warrior's guard stance.
function stance(
  base: Readonly<Record<string, EulerTriple>>,
  overrides: Readonly<Record<string, EulerTriple>>,
): Record<string, EulerTriple> {
  return { ...base, ...overrides };
}

// The combat guard every clip returns to: elbows bent, sword hand forward,
// shield arm carrying the shield at his left side.
//
// Relaxed carry stance: arms hang and swing freely. This -- not GUARD -- is
// the base for locomotion. A player jogging with the ball is not holding a
// shield up across their chest, and building every clip on the guard is a
// large part of why running looked stiff.
const FREE: Readonly<Record<string, EulerTriple>> = {
  "upper_arm.R": [0, 0, 7],
  "forearm.R": [-20, 0, 0],
  "hand.R": [0, -8, 0],
  "upper_arm.L": [0, 0, 9],
  "forearm.L": [-20, 0, 0],
};

// Arm pose for a locomotion key. Arms counter-swing against the legs, and the
// elbow bends further the faster you go: reference walks sit near 30 degrees
// of elbow, runs near 90.
function arms(
  fwdL: number,
  elbowL: number,
  fwdR: number,
  elbowR: number,
): Record<string, EulerTriple> {
  return {
    "upper_arm.L": [fwdL, 0, 9],
    "forearm.L": [-elbowL, 0, 0],
    "upper_arm.R": [fwdR, 0, 7],
    "forearm.R": [-elbowR, 0, 0],
  };
}

const GUARD: Readonly<Record<string, EulerTriple>> = {
  "upper_arm.R": [-18, 0, 6],
  "forearm.R": [-72, 0, 0],
  "hand.R": [0, -8, 0],
  "upper_arm.L": [-12, 0, 18],
  "forearm.L": [-58, 0, 0],
  "thigh.R": [-4, 0, 0],
  "thigh.L": [6, 0, 0],
  "shin.L": [6, 0, 0],
};

// ---------------------------------------------------------------------------
// Clip 1: IDLE -- breathing and a slow weight shift from one foot to the other.
// ---------------------------------------------------------------------------
const IDLE_RAW: RawClip = {
  name: "idle",
  loop: true,
  root_motion: false,
  duration: 3.4,
  keys: [
    { t: 0.0, rot: stance(FREE, {}), move: {} },
    {
      t: 0.85,
      rot: stance(FREE, {
        spine: [-1.5, 0, 0],
        chest: [-2.5, 0, 0],
        head: [-2, 3, 0],
        hips: [0, 0, 1.5],
        "upper_arm.R": [-16, 0, 7],
        "forearm.R": [-70, 0, 0],
        "upper_arm.L": [-10, 0, 19],
      }),
      move: { root: [0.015, 0.012, 0] },
    },
    {
      t: 1.7,
      rot: stance(FREE, {
        spine: [0.5, 0, 0],
        chest: [1.0, 0, 0],
        head: [1.5, -5, 0],
      }),
      move: { root: [0, 0, 0] },
    },
    {
      t: 2.55,
      rot: stance(FREE, {
        spine: [-1.2, 0, 0],
        chest: [-2.0, 0, 0],
        head: [-1.5, -4, 0],
        hips: [0, 0, -1.5],
        "upper_arm.R": [-20, 0, 5],
        "forearm.R": [-74, 0, 0],
        "upper_arm.L": [-14, 0, 17],
      }),
      move: { root: [-0.015, 0.01, 0] },
    },
    { t: 3.4, rot: stance(FREE, {}), move: {} },
  ],
};

// Builds a leg pose for one locomotion key: the forward leg's four joints,
// then the trailing leg's, then anything else this key sets.
function step(
  thighF: number,
  shinF: number,
  footF: number,
  toeF: number,
  thighB: number,
  shinB: number,
  footB: number,
  toeB: number,
  extra?: Readonly<Record<string, EulerTriple>>,
): Record<string, EulerTriple> {
  return {
    "thigh.R": [thighF, 0, -2],
    "shin.R": [shinF, 0, 0],
    "foot.R": [footF, 0, 0],
    "toe.R": [toeF, 0, 0],
    "thigh.L": [thighB, 0, 2],
    "shin.L": [shinB, 0, 0],
    "foot.L": [footB, 0, 0],
    "toe.L": [toeB, 0, 0],
    ...(extra ?? {}),
  };
}

// Mirrors a pose left/right, so a cycle is authored once and the opposite
// step comes for free -- and cannot drift out of sync with its own mirror.
function mirror(pose: Readonly<Record<string, EulerTriple>>): Record<string, EulerTriple> {
  const out: Record<string, EulerTriple> = {};
  for (const [name, v] of Object.entries(pose)) {
    const match = /^(.*)\.([LR])$/.exec(name);
    // Y and Z are the twist and splay axes, so both flip with the side.
    const swapped = match ? `${match[1] ?? ""}.${match[2] === "R" ? "L" : "R"}` : name;
    out[swapped] = [v[0], -v[1], -v[2]];
  }
  return out;
}

// ---------------------------------------------------------------------------
// Clip 2: WALK
// ---------------------------------------------------------------------------
function walkContact(): Record<string, EulerTriple> {
  const pose = step(-26, 6, 10, -16, 22, 30, -16, -30, { hips: [0, -5, 2], chest: [3, 5, 0] });
  return { ...pose, ...arms(-24, 32, 16, 24) };
}

function walkPassing(): Record<string, EulerTriple> {
  const pose = step(6, 12, -6, -4, -10, 70, -16, -12, { hips: [0, 0, 0], chest: [3, 0, 0] });
  return { ...pose, ...arms(-8, 28, -2, 26) };
}

const WALK_RAW: RawClip = {
  name: "walk",
  loop: true,
  root_motion: false,
  stride: 130,
  duration: 0.8,
  keys: [
    { t: 0.0, rot: stance(FREE, walkContact()), move: { root: [0, 0, 0] } },
    { t: 0.2, rot: stance(FREE, walkPassing()), move: { root: [0, 0.036, 0] } },
    { t: 0.4, rot: stance(FREE, mirror(walkContact())), move: { root: [0, 0, 0] } },
    { t: 0.6, rot: stance(FREE, mirror(walkPassing())), move: { root: [0, 0.036, 0] } },
    { t: 0.8, rot: stance(FREE, walkContact()), move: { root: [0, 0, 0] } },
  ],
};

// ---------------------------------------------------------------------------
// Clip 3: RUN
// ---------------------------------------------------------------------------
function runContact(): Record<string, EulerTriple> {
  const pose = step(-42, 18, 14, -22, 34, 78, -24, -38, {
    hips: [0, -7, 3],
    spine: [9, 0, 0],
    chest: [4, 7, 0],
  });
  return { ...pose, ...arms(-44, 88, 32, 72) };
}

function runPassing(): Record<string, EulerTriple> {
  const pose = step(12, 34, -10, -8, -22, 118, -18, -16, {
    hips: [0, 0, 0],
    spine: [11, 0, 0],
    chest: [4, 0, 0],
  });
  return { ...pose, ...arms(-14, 82, 6, 80) };
}

const RUN_RAW: RawClip = {
  name: "run",
  loop: true,
  root_motion: false,
  stride: 285,
  duration: 0.6,
  keys: [
    { t: 0.0, rot: stance(FREE, runContact()), move: { root: [0, 0, 0] } },
    { t: 0.15, rot: stance(FREE, runPassing()), move: { root: [0, 0.058, 0] } },
    { t: 0.3, rot: stance(FREE, mirror(runContact())), move: { root: [0, 0, 0] } },
    { t: 0.45, rot: stance(FREE, mirror(runPassing())), move: { root: [0, 0.058, 0] } },
    { t: 0.6, rot: stance(FREE, runContact()), move: { root: [0, 0, 0] } },
  ],
};

// ---------------------------------------------------------------------------
// GUARD STANCE: a held defensive pose, layered over locomotion rather than
// baked into it.
// ---------------------------------------------------------------------------
const GUARD_STANCE_RAW: RawClip = {
  name: "guard_stance",
  loop: true,
  root_motion: false,
  duration: 1.6,
  keys: [
    { t: 0.0, rot: stance(GUARD, { chest: [2, -4, 0] }), move: {} },
    { t: 0.8, rot: stance(GUARD, { chest: [3, 4, 0] }), move: {} },
    { t: 1.6, rot: stance(GUARD, { chest: [2, -4, 0] }), move: {} },
  ],
};

// ---------------------------------------------------------------------------
// Clip 3: SWING -- an overhead light_melee strike, with a long recovery
// ---------------------------------------------------------------------------
const SWING_RAW: RawClip = {
  name: "swing",
  loop: false,
  root_motion: false,
  fallback: "idle",
  duration: 1.4,
  keys: [
    { t: 0.0, rot: stance(GUARD, {}), move: {} },
    {
      // windup: torso coils to his right, sword arm lifts overhead and back
      t: 0.32,
      rot: stance(GUARD, {
        spine: [-7, -10, 0],
        chest: [-4, -24, 0],
        head: [7, -12, 0],
        hips: [0, -8, 0],
        "upper_arm.R": [-125, 0, 16],
        "forearm.R": [-55, 0, 0],
        "hand.R": [0, -14, 0],
        "upper_arm.L": [-14, 0, 20],
        "forearm.L": [-58, 0, 0],
        "thigh.R": [8, 0, -2],
        "thigh.L": [-6, 0, 2],
      }),
      move: { root: [0, 0.02, -0.03] },
    },
    {
      // strike: everything uncoils forward into a braced lunge
      t: 0.5,
      rot: stance(GUARD, {
        spine: [15, 10, 0],
        chest: [10, 22, 0],
        head: [-11, 10, 0],
        hips: [0, 10, 0],
        "upper_arm.R": [-45, 0, 6],
        "forearm.R": [-15, 0, 0],
        "hand.R": [0, -4, 0],
        "upper_arm.L": [-18, 0, 20],
        "forearm.L": [-60, 0, 0],
        "thigh.L": [-30, 0, 2],
        "shin.L": [10, 0, 0],
        "thigh.R": [18, 0, -2],
        "shin.R": [24, 0, 0],
        "foot.R": [-14, 0, 0],
      }),
      move: { root: [0, -0.025, 0.05] },
    },
    {
      // follow-through
      t: 0.66,
      rot: stance(GUARD, {
        spine: [19, 12, 0],
        chest: [13, 26, 0],
        head: [-13, 12, 0],
        hips: [0, 12, 0],
        "upper_arm.R": [-22, 0, 4],
        "forearm.R": [-3, 0, 0],
        "upper_arm.L": [-16, 0, 22],
        "forearm.L": [-58, 0, 0],
        "thigh.L": [-32, 0, 2],
        "shin.L": [14, 0, 0],
        "thigh.R": [20, 0, -2],
        "shin.R": [28, 0, 0],
        "foot.R": [-16, 0, 0],
      }),
      move: { root: [0, -0.032, 0.06] },
    },
    {
      // settle back toward the guard
      t: 0.98,
      rot: stance(GUARD, {
        spine: [7, 4, 0],
        chest: [5, 10, 0],
        "upper_arm.R": [-30, 0, 6],
        "forearm.R": [-55, 0, 0],
        "thigh.L": [-10, 0, 2],
      }),
      move: { root: [0, -0.01, 0.02] },
    },
    { t: 1.4, rot: stance(GUARD, {}), move: {} },
  ],
};

// ---------------------------------------------------------------------------
// Clip 4: CHARGE -- a held attack pose, not a swing. Authored as an upper-body
// OVERLAY: it deliberately says nothing about the legs, so whatever
// locomotion is playing underneath keeps the stride.
// ---------------------------------------------------------------------------
const CHARGE_RAW: RawClip = {
  name: "charge",
  loop: true,
  root_motion: false,
  duration: 0.7,
  keys: [
    {
      t: 0.0,
      rot: {
        spine: [16, 0, 0],
        chest: [10, -6, 0],
        head: [-17, 4, 0],
        "upper_arm.R": [-66, 0, 10],
        "forearm.R": [-22, 0, 0],
        "hand.R": [0, -6, 0],
        "upper_arm.L": [-34, 0, 22],
        "forearm.L": [-74, 0, 0],
      },
      move: {},
    },
    {
      t: 0.35,
      rot: {
        spine: [19, 0, 0],
        chest: [12, 6, 0],
        head: [-20, -4, 0],
        "upper_arm.R": [-62, 0, 8],
        "forearm.R": [-26, 0, 0],
        "hand.R": [0, -6, 0],
        "upper_arm.L": [-30, 0, 24],
        "forearm.L": [-70, 0, 0],
      },
      move: {},
    },
    {
      t: 0.7,
      rot: {
        spine: [16, 0, 0],
        chest: [10, -6, 0],
        head: [-17, 4, 0],
        "upper_arm.R": [-66, 0, 10],
        "forearm.R": [-22, 0, 0],
        "hand.R": [0, -6, 0],
        "upper_arm.L": [-34, 0, 22],
        "forearm.L": [-74, 0, 0],
      },
      move: {},
    },
  ],
};

// ---------------------------------------------------------------------------
// Clip 5: KEEPER_GATHER -- both arms wrapped around a held ball
// ---------------------------------------------------------------------------
const KEEPER_GATHER_RAW: RawClip = {
  name: "keeper_gather",
  loop: true,
  root_motion: false,
  duration: 2.0,
  keys: [
    {
      t: 0.0,
      rot: {
        spine: [6, 0, 0],
        chest: [4, 0, 0],
        head: [6, 0, 0],
        "upper_arm.R": [-72, 0, 2],
        "forearm.R": [-58, 0, 0],
        "upper_arm.L": [-72, 0, 2],
        "forearm.L": [-58, 0, 0],
      },
      move: {},
    },
    {
      t: 1.0,
      rot: {
        spine: [8, 0, 0],
        chest: [5, 0, 0],
        head: [5, 0, 0],
        "upper_arm.R": [-75, 0, 2],
        "forearm.R": [-60, 0, 0],
        "upper_arm.L": [-75, 0, 2],
        "forearm.L": [-60, 0, 0],
      },
      move: {},
    },
    {
      t: 2.0,
      rot: {
        spine: [6, 0, 0],
        chest: [4, 0, 0],
        head: [6, 0, 0],
        "upper_arm.R": [-72, 0, 2],
        "forearm.R": [-58, 0, 0],
        "upper_arm.L": [-72, 0, 2],
        "forearm.L": [-58, 0, 0],
      },
      move: {},
    },
  ],
};

// ---------------------------------------------------------------------------
// Clip 6: KEEPER_SLING -- the overarm throw out
// ---------------------------------------------------------------------------
const KEEPER_SLING_RAW: RawClip = {
  name: "keeper_sling",
  loop: false,
  root_motion: false,
  fallback: "idle",
  duration: 0.5,
  keys: [
    {
      // cocked
      t: 0.0,
      rot: {
        spine: [-4, -12, 0],
        chest: [-6, -20, 0],
        head: [4, 6, 0],
        "upper_arm.R": [-140, 0, 14],
        "forearm.R": [-62, 0, 0],
        "upper_arm.L": [-62, 0, 12],
        "forearm.L": [-22, 0, 0],
      },
      move: {},
    },
    {
      // release: arm through the top, torso unwound and over the front foot
      t: 0.28,
      rot: {
        spine: [10, 6, 0],
        chest: [8, 12, 0],
        head: [6, 0, 0],
        "upper_arm.R": [-96, 0, 6],
        "forearm.R": [-10, 0, 0],
        "upper_arm.L": [-30, 0, 14],
        "forearm.L": [-34, 0, 0],
      },
      move: {},
    },
    {
      // follow-through, settling back toward the carry stance
      t: 0.5,
      rot: {
        spine: [8, 2, 0],
        chest: [6, 4, 0],
        head: [4, 0, 0],
        "upper_arm.R": [-44, 0, 8],
        "forearm.R": [-26, 0, 0],
        "upper_arm.L": [-20, 0, 12],
        "forearm.L": [-28, 0, 0],
      },
      move: {},
    },
  ],
};

// Precomputes the set of bones a clip touches, so the sampler knows which
// channels to emit without re-scanning every keyframe each frame. Also bakes
// authored Euler into quaternions once: everything downstream -- sampling,
// layering, crossfading -- works on quaternions; humans keep editing degrees
// above.
function prepare(raw: RawClip): Clip {
  const firstKey = raw.keys[0];
  if (!firstKey || firstKey.t !== 0) {
    throw new Error(`${raw.name}: first key must be at t = 0`);
  }
  // A one-shot has to say where playback lands when it finishes, or the
  // controller has nothing to fall back to.
  if (!raw.loop && !raw.fallback) {
    throw new Error(`${raw.name}: one-shot needs a fallback`);
  }
  const lastKey = raw.keys[raw.keys.length - 1];
  if (!lastKey || lastKey.t !== raw.duration) {
    throw new Error(`${raw.name}: last key must be at the duration`);
  }

  const rotBones = new Set<string>();
  const moveBones = new Set<string>();
  const keys: PreparedKeyframe[] = raw.keys.map((key) => {
    const q: Record<string, Quat> = {};
    for (const [name, e] of Object.entries(key.rot ?? {})) {
      q[name] = quat.fromEuler(
        (e[0] * Math.PI) / 180,
        (e[1] * Math.PI) / 180,
        (e[2] * Math.PI) / 180,
      );
      rotBones.add(name);
    }
    for (const name of Object.keys(key.move ?? {})) {
      moveBones.add(name);
    }
    return { t: key.t, q, move: key.move ?? {} };
  });

  return { ...raw, keys, rotBones, moveBones };
}

export const IDLE: Clip = prepare(IDLE_RAW);
export const WALK: Clip = prepare(WALK_RAW);
export const SWING: Clip = prepare(SWING_RAW);
export const ORDER: readonly Clip[] = [IDLE, WALK, SWING];
export const RUN: Clip = prepare(RUN_RAW);
export const GUARD_STANCE: Clip = prepare(GUARD_STANCE_RAW);
export const CHARGE: Clip = prepare(CHARGE_RAW);
export const KEEPER_GATHER: Clip = prepare(KEEPER_GATHER_RAW);
export const KEEPER_SLING: Clip = prepare(KEEPER_SLING_RAW);

const ZERO: EulerTriple = [0, 0, 0];
const IDENTITY: Quat = quat.identity();

function lerp3(a: EulerTriple, b: EulerTriple, u: number): EulerTriple {
  return [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u];
}

// Composes an overlay layer onto a base pose through a bone mask.
//
// Override semantics, which is what a masked layer means: for every bone the
// mask owns, the result comes from the OVERLAY -- and if the overlay clip
// never touches that bone, it goes to rest rather than leaking the base pose
// through. Anything outside the mask is untouched. `weight` fades the layer
// in and out, which is how an action blends over a run instead of popping
// onto it.
export function layer(base: Pose, overlay: Pose, mask: ReadonlySet<string>, weight: number): Pose {
  const rot: Record<string, Quat> = { ...base.rot };
  const move: Record<string, EulerTriple> = { ...base.move };

  for (const name of mask) {
    // Rotations blend on the short arc; translations are linear, which is
    // correct -- only rotation has the wrap-around problem.
    if (overlay.rot[name] || base.rot[name]) {
      rot[name] = quat.slerp(base.rot[name] ?? IDENTITY, overlay.rot[name] ?? IDENTITY, weight);
    }
    if (overlay.move[name] || base.move[name]) {
      move[name] = lerp3(base.move[name] ?? ZERO, overlay.move[name] ?? ZERO, weight);
    }
  }
  return { rot, move };
}

// Composes a sparse overlay ADDITIVELY onto a base pose: rotations
// pre-multiplied, translations added.
//
// THE OTHER HALF OF `layer`, AND THE DIFFERENCE IS NOT A DETAIL. `layer` is
// OVERRIDE: the mask decides which bones the overlay owns, and a bone it owns
// but never mentions goes to REST. That is right for a stance -- a guard is a
// complete statement about the arms. It is wrong for a MODIFIER: a crouch is
// something a body does WHILE running, so an override would have to restate the
// stride to keep it, and a sparse crouch masked over the legs would delete it.
//
// So this one has no mask at all, deliberately. The overlay's own key set is
// the mask, because there is no "the mask owns it and the overlay is silent"
// case to resolve. Bones the overlay does not name keep the base pose exactly.
//
// PRE-MULTIPLIED, matching `action_pose.apply`'s attitude branch: the overlay's
// rotation is applied in the PARENT's frame, so a fold added to a thigh is a
// fold of the whole leg from the hip rather than a re-expression inside
// whatever the stride resolved. Every rotation this composes is authored about
// a single axis and `quat.fromEuler` puts x outermost after y, so for the leg
// bones (which carry x and z only) the composition is exactly additive in the
// authored angle -- `crouch.spec.ts` measures that rather than assuming it.
//
// No weight parameter, and that is load-bearing rather than an omission. A
// weight would scale the rotation and the translation by the SAME factor, and
// they do not scale together: a fold of `w * c` raises the foot by
// `L * (1 - cos(w * c))`, which is quadratic in `w`, while `w * drop` is
// linear -- so any weight below 1 would sink the feet. `crouch.poseFor` takes
// the DEPTH instead and re-derives the fold from it, which stays exact at every
// value the depth ramps through.
export function compose(base: Pose, overlay: Pose): Pose {
  const rot: Record<string, Quat> = { ...base.rot };
  const move: Record<string, EulerTriple> = { ...base.move };
  for (const [name, q] of Object.entries(overlay.rot)) {
    const existing = base.rot[name];
    rot[name] = existing !== undefined ? quat.multiply(q, existing) : q;
  }
  for (const [name, m] of Object.entries(overlay.move)) {
    const existing = base.move[name] ?? ZERO;
    move[name] = [(existing[0] ?? 0) + m[0], (existing[1] ?? 0) + m[1], (existing[2] ?? 0) + m[2]];
  }
  return { rot, move };
}

// Samples a clip at `time`, wrapping so every clip loops seamlessly.
export function sample(clip: Clip, time: number): Pose {
  const t = time % clip.duration;
  const keys = clip.keys;

  let i = 0;
  while (i < keys.length - 2) {
    const next = keys[i + 1];
    if (!next || next.t > t) {
      break;
    }
    i += 1;
  }
  const a = keys[i];
  const b = keys[i + 1];
  if (!a || !b) {
    throw new Error(`${clip.name}: clip has no keyframes to sample`);
  }

  const span = b.t - a.t;
  let u = span > 1e-6 ? (t - a.t) / span : 0;
  u = u * u * (3 - 2 * u); // smoothstep ease in/out

  const rot: Record<string, Quat> = {};
  for (const name of clip.rotBones) {
    rot[name] = quat.slerp(a.q[name] ?? IDENTITY, b.q[name] ?? IDENTITY, u);
  }
  const move: Record<string, EulerTriple> = {};
  for (const name of clip.moveBones) {
    move[name] = lerp3(a.move[name] ?? ZERO, b.move[name] ?? ZERO, u);
  }
  return { rot, move };
}
