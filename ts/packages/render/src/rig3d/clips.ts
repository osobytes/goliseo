// Keyframed animation clips, authored directly in TypeScript.
//
// A clip is a list of keyframes sorted by time. Each keyframe holds a *sparse*
// pose: `rot` gives bone rotations in degrees, `move` gives additive
// translations. A bone a clip never mentions stays at its rest transform.
//
// Key schedules are PER BONE, not per clip (#580). A keyframe is DENSE by
// default: it speaks for every bone the clip touches, and a bone it does not
// name heads to rest at that time -- which is what every key meant before
// #580, so an all-dense clip behaves exactly as it always did. A keyframe
// marked `sparse: true` speaks ONLY for the bones it names: every other bone
// interpolates between its own neighbouring keys as if the sparse key did not
// exist. That is what lets one limb carry a refinement key -- a follow-through
// on the striking arm, a toe-off on the legs -- without re-keying every other
// bone at that time. `prepare` flattens the authored keys into one key track
// per bone, and `sample` walks each bone's own track.
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
// Channels are interpolated independently, and each SEGMENT chooses its own
// easing via the keyframe's `ease` (defaulting to `smooth`, the smoothstep
// every segment used to get unconditionally -- see `ease` for why that default
// was the whole problem when it was the only option). Since #580 a segment is
// a BONE-LOCAL concept: it runs between one bone's own neighbouring keys, and
// the `ease` on a sparse key governs only the named bones' segments starting
// there. Euler angles are interpolated component-wise: fine here because no
// joint passes through a gimbal-locking orientation.

import { quat, type Quat } from "@gc/core";
import type { Pose } from "./skeleton.ts";

/** A rotation or translation authored in the clip's Euler/metres convention. */
export type EulerTriple = readonly [number, number, number];

/**
 * How one segment distributes its motion in time. Named for what the BODY
 * does across the segment, not for the CSS easing of the same shape -- the two
 * conventions use "in" and "out" in opposite senses and this rig has been
 * bitten by sign conventions before (see this file's header).
 *
 * - `smooth` — eased at both ends: `u*u*(3-2u)`. Zero velocity leaving the
 *   first pose AND arriving at the second.
 * - `linear` — constant velocity, `u`.
 * - `accel` — leaves slowly, arrives at full speed: `u*u`. The strike shape.
 * - `decel` — leaves at full speed, settles into the arrival: `u*(2-u)`. The
 *   push-off and follow-through shape.
 */
export type Easing = "smooth" | "linear" | "accel" | "decel";

/**
 * Easing for one segment, optionally split by channel kind.
 *
 * WHY THE SPLIT EXISTS, because a single curve per segment looks like it ought
 * to be enough: in locomotion the limbs and the body's vertical bounce want
 * OPPOSITE timing over the very same segment. A planted foot has to sweep
 * backward at a CONSTANT rate -- that is what "planted" means, and any easing
 * at all makes it skid at one end of the segment -- while the body's rise and
 * fall between the same two keys is ballistic, fast off the ground and slow at
 * the apex. Rotations carry the legs and `move` carries the root, so splitting
 * by channel kind resolves it exactly, with no extra keyframes.
 */
export type SegmentEasing = Easing | { readonly rot?: Easing; readonly move?: Easing };

/** One authored keyframe. Exported for specs; clips are authored in this file. */
export interface RawKeyframe {
  readonly t: number;
  readonly rot?: Readonly<Record<string, EulerTriple>>;
  readonly move?: Readonly<Record<string, EulerTriple>>;
  /**
   * Easing of the segment that STARTS at this key (so the last key's is never
   * read). Omitted means `smooth`, which is what every segment did
   * unconditionally before #574. On a sparse key this governs only the named
   * bones' segments -- the segment is bone-local (#580).
   */
  readonly ease?: SegmentEasing;
  /**
   * A sparse key speaks ONLY for the bones it names: every other bone
   * interpolates between its own neighbouring keys as if this key did not
   * exist. Omitted, the key is DENSE and speaks for every bone the clip
   * touches -- a bone it does not name heads to rest at this time, exactly as
   * every key behaved before #580. Sparse keys are what make keyframe density
   * a per-BONE choice: a refinement key on one limb no longer forces every
   * other bone to be re-keyed at its time. The first and last keys of a clip
   * must be dense, so every bone's own track spans the whole clip.
   */
  readonly sparse?: true;
}

// NO `stride` FIELD, deliberately (#574). The walk and run clips used to carry
// one (130 and 285), it was read by no live code, and it disagreed with the
// authoritative stride in `pose_table.ts` / `view_state.ts` (100 and 185) --
// so the one number a reader would most reasonably trust for "how far does a
// step cover" was both dead and wrong. The stride belongs to the gait, not to
// the clip: `view_state.ts` derives phase from distance actually travelled,
// and `pose_table.strideFor` is the single authority.
/** One authored clip. Exported for specs; clips are authored in this file. */
export interface RawClip {
  readonly name: string;
  readonly loop: boolean;
  readonly root_motion: false;
  readonly duration: number;
  readonly fallback?: string;
  readonly keys: readonly RawKeyframe[];
}

interface PreparedKeyframe {
  readonly t: number;
  readonly q: Readonly<Record<string, Quat>>;
  readonly move: Readonly<Record<string, EulerTriple>>;
  /** Easing of the rotation channels over the segment starting here. */
  readonly easeRot: Easing;
  /** Easing of the translation channels over the segment starting here. */
  readonly easeMove: Easing;
  /** Whether this key spoke only for the bones it names. See `RawKeyframe`. */
  readonly sparse: boolean;
}

// One key on one bone's own rotation track: the flattened form `sample` walks.
interface RotTrackKey {
  readonly t: number;
  readonly q: Quat;
  /** Easing of THIS BONE's segment starting here (the last key's is never read). */
  readonly ease: Easing;
}

// One key on one bone's own translation track.
interface MoveTrackKey {
  readonly t: number;
  readonly v: EulerTriple;
  /** Easing of THIS BONE's segment starting here (the last key's is never read). */
  readonly ease: Easing;
}

/** A clip ready to sample: keyframes baked to quaternions, channels indexed. */
export interface Clip {
  readonly name: string;
  readonly loop: boolean;
  readonly duration: number;
  readonly fallback?: string;
  /**
   * The prepared keyframes, whole. `sample` never reads these -- it walks
   * `rotTracks`/`moveTracks` (#580) -- but the characterization specs do: the
   * frozen pre-#580 sampler that pins bit-identical backwards compatibility
   * needs the clip-wide keyframe view, and the crouch-fold sweep audits keys
   * as authored. Drop this field and those specs lose their subject.
   */
  readonly keys: readonly PreparedKeyframe[];
  readonly rotBones: ReadonlySet<string>;
  readonly moveBones: ReadonlySet<string>;
  /**
   * Per-bone rotation key schedules (#580): for each bone in `rotBones`, its
   * own keys in time order. Dense keyframes contribute an entry for every
   * bone (rest where unnamed); sparse keyframes only for the bones they name.
   */
  readonly rotTracks: Readonly<Record<string, readonly RotTrackKey[]>>;
  /** Per-bone translation key schedules; same construction as `rotTracks`. */
  readonly moveTracks: Readonly<Record<string, readonly MoveTrackKey[]>>;
}

// Hoisted above the module-level `prepare` calls below: `prepare` reads both
// when it fills a dense key's unnamed bones, so declaring them after the
// exports would be a temporal-dead-zone crash at module load.
const ZERO: EulerTriple = [0, 0, 0];
const IDENTITY: Quat = quat.identity();

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

// A walk VAULTS: the body is lowest at contact (double support, legs split)
// and highest at mid-stance, rising over a near-straight support leg. That is
// the opposite of a run's bounce, and both are authored explicitly below
// rather than left to look similar (#574).
//
// Easing is split by channel, and the split is the point (see `SegmentEasing`).
// The LEGS run linear the whole way round: a planted foot has to sweep backward
// at a constant rate, and any easing on the rotations skids it at one end of
// the segment or the other. The ROOT vaults: `decel` climbing to the apex (the
// rise runs out of energy at the top, as a vault does) and `accel` falling off
// it, so the body drops onto the next plant with real downward speed.
//
// Both feet share one set of rotation channels, so this cannot be authored
// per-foot: by mirror symmetry every segment is one foot's stance and the
// other's swing at the same time. Linear serves the planted one, which is the
// foot the eye is judging.
const WALK_RAW: RawClip = {
  name: "walk",
  loop: true,
  root_motion: false,
  duration: 0.8,
  keys: [
    {
      t: 0.0,
      rot: stance(FREE, walkContact()),
      move: { root: [0, 0, 0] },
      ease: { rot: "linear", move: "decel" },
    },
    {
      t: 0.2,
      rot: stance(FREE, walkPassing()),
      move: { root: [0, 0.036, 0] },
      ease: { rot: "linear", move: "accel" },
    },
    {
      t: 0.4,
      rot: stance(FREE, mirror(walkContact())),
      move: { root: [0, 0, 0] },
      ease: { rot: "linear", move: "decel" },
    },
    {
      t: 0.6,
      rot: stance(FREE, mirror(walkPassing())),
      move: { root: [0, 0.036, 0] },
      ease: { rot: "linear", move: "accel" },
    },
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

// A run BOUNCES, and its vertical phase is the INVERSE of the walk's above --
// which is the bug this authoring fixes (#574). The `runContact` split pose is
// the airborne moment (both legs extended, no foot down yet), so it is the
// HIGH point; `runPassing` is mid-stance, where the support knee is loaded and
// the body is compressed, so it is the LOW point. These heights used to be the
// walk's way round, which gave the run a vault instead of a bounce: no impact
// bottom, no airborne top, and a sprint that read as a hurried walk.
//
// The swap is mean-neutral -- two keys of each per cycle, the same 0.058
// amplitude -- so nothing about the character's average standing height moves,
// and the low point stays at 0 rather than going negative into `ground.ts`'s
// penetration lift.
//
// Easing splits by channel exactly as the walk's does and for the same reason.
// The ROOT gets the bouncing-ball shape, which is what a run is: `accel` down
// off the airborne apex onto the foot (it lands with real downward speed --
// that is the impact) and `decel` off the compressed mid-stance, the push-off
// being an impulse that is spent by the time it reaches the apex. The LEGS run
// linear so the planted foot holds a constant backward sweep.
//
// Under the old unconditional smoothstep every one of these segments came to a
// dead stop at BOTH ends, on every channel: the run's legs stopped four times a
// second, which is the texture that read as slow motion.
const RUN_RAW: RawClip = {
  name: "run",
  loop: true,
  root_motion: false,
  duration: 0.6,
  keys: [
    {
      t: 0.0,
      rot: stance(FREE, runContact()),
      move: { root: [0, 0.058, 0] },
      ease: { rot: "linear", move: "accel" },
    },
    {
      t: 0.15,
      rot: stance(FREE, runPassing()),
      move: { root: [0, 0, 0] },
      ease: { rot: "linear", move: "decel" },
    },
    {
      t: 0.3,
      rot: stance(FREE, mirror(runContact())),
      move: { root: [0, 0.058, 0] },
      ease: { rot: "linear", move: "accel" },
    },
    {
      t: 0.45,
      rot: stance(FREE, mirror(runPassing())),
      move: { root: [0, 0, 0] },
      ease: { rot: "linear", move: "decel" },
    },
    { t: 0.6, rot: stance(FREE, runContact()), move: { root: [0, 0.058, 0] } },
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
// The easing here is the whole shape of a strike, and it is deliberately NOT
// uniform (#574). A convincing attack is three different speeds: a coil that
// arrives and settles (`decel` into the windup key), a delivery that starts
// from that stillness and arrives at maximum velocity (`accel` into the strike
// key -- the one segment in this file where a dead stop at the destination
// would destroy the entire read), and a follow-through that carries the
// momentum out and spends it (`decel`, then `smooth` through the settle).
//
// Snap is a RATIO, not a rate: what makes the strike land is that it is fast
// relative to the coil either side of it, which is why speeding the whole clip
// up would not have bought it and would have read as fast-forward instead.
const SWING_RAW: RawClip = {
  name: "swing",
  loop: false,
  root_motion: false,
  fallback: "idle",
  duration: 1.4,
  keys: [
    { t: 0.0, rot: stance(GUARD, {}), move: {}, ease: "decel" },
    {
      // windup: torso coils to his right, sword arm lifts overhead and back
      ease: "accel",
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
      ease: "decel",
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

/**
 * Precomputes the set of bones a clip touches, bakes authored Euler into
 * quaternions once (everything downstream -- sampling, layering, crossfading
 * -- works on quaternions; humans keep editing degrees above), and flattens
 * the keyframes into one key track per bone so `sample` can interpolate each
 * bone between ITS OWN neighbouring keys (#580).
 *
 * Exported for specs, which need to build clips with per-bone schedules
 * without shipping them; production clips are authored in this file.
 */
export function prepare(raw: RawClip): Clip {
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
  if (raw.keys.length < 2) {
    throw new Error(`${raw.name}: a clip needs at least two keys`);
  }
  // Sparse keys refine an existing schedule; the schedule's ENDS have to be
  // dense so every bone's own track spans the whole clip -- otherwise a bone
  // named only in sparse keys would have no neighbour at the edges, and a
  // looping clip would pop at the wrap.
  if (firstKey.sparse || lastKey.sparse) {
    throw new Error(`${raw.name}: first and last keys must be dense (not sparse)`);
  }
  // Sorted keys were always assumed; now they are load-bearing (a sparse key
  // slotted at the wrong time would corrupt every named bone's track), so the
  // assumption is checked. Programmer error, so it fails loud.
  for (let i = 1; i < raw.keys.length; i += 1) {
    const prev = raw.keys[i - 1];
    const cur = raw.keys[i];
    if (prev && cur && cur.t <= prev.t) {
      throw new Error(`${raw.name}: keys must be in strictly increasing time order`);
    }
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
    const spec = key.ease ?? "smooth";
    const easeRot = typeof spec === "string" ? spec : (spec.rot ?? "smooth");
    const easeMove = typeof spec === "string" ? spec : (spec.move ?? "smooth");
    return { t: key.t, q, move: key.move ?? {}, easeRot, easeMove, sparse: key.sparse === true };
  });

  // Flatten into per-bone tracks. A dense key contributes an entry for EVERY
  // bone the clip touches -- rest where it names none, which is exactly what
  // `sample` used to synthesise at sampling time, so an all-dense clip's
  // tracks reproduce the old behaviour value for value. A sparse key
  // contributes entries only for the bones it names.
  const rotTracks: Record<string, readonly RotTrackKey[]> = {};
  for (const bone of rotBones) {
    const track: RotTrackKey[] = [];
    for (const key of keys) {
      const q = key.q[bone];
      if (key.sparse && q === undefined) {
        continue;
      }
      track.push({ t: key.t, q: q ?? IDENTITY, ease: key.easeRot });
    }
    rotTracks[bone] = track;
  }
  const moveTracks: Record<string, readonly MoveTrackKey[]> = {};
  for (const bone of moveBones) {
    const track: MoveTrackKey[] = [];
    for (const key of keys) {
      const v = key.move[bone];
      if (key.sparse && v === undefined) {
        continue;
      }
      track.push({ t: key.t, v: v ?? ZERO, ease: key.easeMove });
    }
    moveTracks[bone] = track;
  }

  return { ...raw, keys, rotBones, moveBones, rotTracks, moveTracks };
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

// Remaps a segment's normalised progress. See `Easing` for the four shapes.
//
// WHY THIS IS A CHOICE PER SEGMENT AND NOT A CONSTANT (#574). Every segment of
// every channel used to be smoothstepped unconditionally, right here. The
// argument for that -- in this file's header, and it is a real argument -- was
// that a 4-6 key clip interpolated linearly reads as straight-line lerp. True,
// but it answers the wrong question: lerp's failure is POPPINESS at the keys,
// and smoothstep bought the fix by driving angular velocity to ZERO AT EVERY
// KEY. Every joint of every player came to a complete stop four times a second
// during a run. That is the texture players described as slow motion, and it
// is why "make the animations faster" was the wrong instinct -- the clips were
// not too long, they were full of stops.
//
// The rule this replaces it with: a pose the body PASSES THROUGH (a foot
// contact, a strike) keeps its velocity, and a pose the body ARRIVES AT (an
// apex, a settle) eases. `smooth` stays the default so an unannotated
// keyframe behaves exactly as it did before this change.
function ease(u: number, kind: Easing): number {
  switch (kind) {
    case "linear":
      return u;
    case "accel":
      return u * u;
    case "decel":
      return u * (2 - u);
    case "smooth":
      return u * u * (3 - 2 * u);
  }
}

// Finds the segment of one bone's track containing `t`: the pair of
// neighbouring keys around it and the normalised (un-eased) progress between
// them. The scan is the exact shape the old whole-clip sampler used, applied
// per track: the last segment absorbs any `t` at or past its start, so a
// sample exactly on the final key lands at u = 1 of the segment before it.
function segmentAt<K extends { readonly t: number }>(
  track: readonly K[],
  t: number,
): readonly [K, K, number] | undefined {
  let i = 0;
  while (i < track.length - 2) {
    const next = track[i + 1];
    if (!next || next.t > t) {
      break;
    }
    i += 1;
  }
  const a = track[i];
  const b = track[i + 1];
  if (!a || !b) {
    return undefined;
  }
  const span = b.t - a.t;
  const u = span > 1e-6 ? (t - a.t) / span : 0;
  return [a, b, u];
}

// Samples a clip at `time`, wrapping so every clip loops seamlessly.
//
// Each bone interpolates between ITS OWN neighbouring keys (#580): the
// segment -- and therefore the easing -- is bone-local, so a sparse
// refinement key on one limb bends that limb's timing and nobody else's.
// For an all-dense clip every track shares the clip's key times and this
// reproduces the old whole-clip sampler value for value.
export function sample(clip: Clip, time: number): Pose {
  const t = time % clip.duration;

  const rot: Record<string, Quat> = {};
  for (const name of clip.rotBones) {
    const seg = segmentAt(clip.rotTracks[name] ?? [], t);
    if (!seg) {
      throw new Error(`${clip.name}: ${name} has no keyframes to sample`);
    }
    const [a, b, u] = seg;
    rot[name] = quat.slerp(a.q, b.q, ease(u, a.ease));
  }
  const move: Record<string, EulerTriple> = {};
  for (const name of clip.moveBones) {
    const seg = segmentAt(clip.moveTracks[name] ?? [], t);
    if (!seg) {
      throw new Error(`${clip.name}: ${name} has no keyframes to sample`);
    }
    const [a, b, u] = seg;
    move[name] = lerp3(a.v, b.v, ease(u, a.ease));
  }
  return { rot, move };
}
