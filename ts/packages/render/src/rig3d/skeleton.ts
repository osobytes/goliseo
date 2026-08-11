// The rigged-player skeleton: a concrete proposal for the bone contract #93
// has to pin down.
//
// Orientation convention:
//   +Y up, the character faces +Z, and because the frame is right-handed his
//   own RIGHT side is at -X. Bone names use his anatomical left/right.
//
// Bones are listed parent-before-child, so a single forward pass computes every
// world transform: world = parent.world * local.
//
// ---------------------------------------------------------------------------
// DESIGN RULES (load-bearing decisions, not style preferences)
// ---------------------------------------------------------------------------
//
// The plan is to retarget ~131 external CC0 clips (KayKit Rig_Medium) onto this
// skeleton. That single fact drives everything below:
//
// 1. CHAIN TOPOLOGY MIRRORS A STANDARD HUMANOID. Inserting a bone into an
//    existing chain changes how rotation distributes along it, so every
//    imported clip would need remapping and name/proportion-based retargeters
//    produce hunched or corkscrewed results. This is the classic retargeting
//    failure mode.
//
//    Consequence: NO third spine segment and no in-chain twist bones. Two spine
//    segments plus neck is enough for bicycle kicks, dives and staggers when
//    the arch is spread across hips + spine + chest + neck.
//
// 2. LEAF BONES ARE FREE. A bone with no children that no source clip targets
//    simply holds its rest pose. Toes and sockets cost nothing.
//
// 3. NAMES ARE A RETARGETING INTERFACE. `hips` (not `pelvis`) because
//    name-matchers look for "hips". `shoulder.L` IS the clavicle -- having both
//    a `clavicle` and a `shoulder` double-matches. `.L`/`.R` suffixes are what
//    Blender symmetry, Rigify, Mixamo converters and Auto-Rig Pro key on. The
//    `socket_` prefix marks non-deform bones so no auto-weighter touches them.
//
// Parts and gear ride a bone's world transform. There are no per-vertex skin
// WEIGHTS here -- one bone drives a vertex outright, which is the prototype's
// rigid arcade look.
//
// BONE INDEX CONTRACT: the order `bones()` returns IS the bone index order,
// 0-based. Vertices baked against one order and matrices resolved against
// another would silently render a scrambled character, so both sides go
// through `boneIndex` and never count for themselves.
//
// MECHANISM VS CONTENT: the skinning maths below -- computing a bone's world
// transform from its parent and evaluating a posed joint position -- is
// exactly what three.js's `Skeleton`/`Bone` classes do, and a rendering
// integration pass can host it there once this lands on stage. What lives
// here, deliberately, is the CONTENT: this game's specific bone names, parents,
// rest offsets and rest rotations, plus the pure pose-evaluation functions the
// rest of rig3d (action_pose.ts, clips.ts) depend on and which stay exactly as
// testable without any renderer. Deliberately absent: `boneRows` /
// `ROWS_PER_BONE`, the three-rows-per-bone matrix packing that only existed to
// fit a hand-written shader inside GLSL ES 1.00's 128 vertex-uniform-vector
// floor -- replaced by three.js's own bone-matrix upload path, which needs no
// such packing.

import { mat4, quat, type Mat4, type Quat } from "@gc/core";

/** A rest translation in the parent's space, in metres. */
export type Offset = readonly [number, number, number];
/** A rest rotation in degrees, added to whatever the animation asks for. */
export type RestRotation = readonly [number, number, number];

/** One bone definition, as authored (parent-before-child). */
export interface BoneDef {
  readonly name: string;
  readonly parent: string | null;
  readonly offset: Offset;
  readonly rest?: RestRotation;
}

/** A bone definition with its rest rotation baked to a quaternion. */
interface PreparedBoneDef extends BoneDef {
  readonly q_rest: Quat;
}

/** A pose to evaluate: sparse per-bone rotation (already a `Quat`) and translation. */
export interface Pose {
  readonly rot: Readonly<Record<string, Quat>>;
  readonly move: Readonly<Record<string, Offset>>;
}

/** A skeleton instance, posed: every bone's current world transform. */
export interface Rig {
  readonly style: RigLike;
  readonly defs: readonly PreparedBoneDef[];
  readonly world: Record<string, Mat4>;
  readonly byName: Readonly<Record<string, PreparedBoneDef>>;
  readonly order: readonly string[];
}

/** The subset of `proportions.RigProportions` the skeleton needs. */
export interface RigLike {
  readonly seg: {
    readonly hips_y: number;
    readonly spine: number;
    readonly chest: number;
    readonly neck: number;
    readonly head: number;
    readonly shoulder_x: number;
    readonly shoulder_y: number;
    readonly arm_x: number;
    readonly upperarm: number;
    readonly lowerarm: number;
    readonly thigh_x: number;
    readonly thigh_y: number;
    readonly upperleg: number;
    readonly lowerleg: number;
  };
  readonly form: {
    readonly hand_r: number;
    readonly foot_w: number;
    readonly foot_len: number;
    readonly arm_r: number;
    readonly torso_r: number;
  };
  readonly motion_scale?: number;
}

const RIGHT = -1;
const LEFT = 1;

function arm(s: RigLike["seg"], f: RigLike["form"], side: "L" | "R", sign: number): BoneDef[] {
  return [
    {
      name: `shoulder.${side}`,
      parent: "chest",
      offset: [sign * s.shoulder_x, s.shoulder_y, 0],
    },
    {
      name: `upper_arm.${side}`,
      parent: `shoulder.${side}`,
      offset: [sign * s.arm_x, -s.upperarm * 0.11, 0],
      rest: [0, 0, sign * 8],
    },
    {
      name: `forearm.${side}`,
      parent: `upper_arm.${side}`,
      offset: [0, -s.upperarm, 0],
    },
    { name: `hand.${side}`, parent: `forearm.${side}`, offset: [0, -s.lowerarm, 0] },
    // Grip socket, non-deform. Everything held in a fist hangs off this, so
    // the three light_melee items share one attachment id.
    {
      name: `socket_hand.${side}`,
      parent: `hand.${side}`,
      offset: [0, -f.hand_r * 1.05, f.hand_r * 0.1],
      rest: [145, 0, 0],
    },
  ];
}

function leg(s: RigLike["seg"], f: RigLike["form"], side: "L" | "R", sign: number): BoneDef[] {
  return [
    {
      name: `thigh.${side}`,
      parent: "hips",
      offset: [sign * s.thigh_x, s.thigh_y, 0],
      rest: [0, 0, sign * 2],
    },
    { name: `shin.${side}`, parent: `thigh.${side}`, offset: [0, -s.upperleg, 0] },
    { name: `foot.${side}`, parent: `shin.${side}`, offset: [0, -s.lowerleg, 0] },
    // Ball of the foot. Without it the foot is a rigid block: no roll through
    // a plant, no push-off on a keeper dive, no instep orientation for a kick
    // contact. Highest-value addition for a football game, and free because
    // it is a leaf.
    {
      name: `toe.${side}`,
      parent: `foot.${side}`,
      offset: [0, -f.foot_w * 0.34, f.foot_len * 0.4],
    },
  ];
}

/** Every bone this rig contract has, parent-before-child, 0-based order. */
export function bones(rig: RigLike): readonly BoneDef[] {
  const s = rig.seg;
  const f = rig.form;

  const out: BoneDef[] = [
    { name: "root", parent: null, offset: [0, 0, 0] },
    { name: "hips", parent: "root", offset: [0, s.hips_y, 0] },
    { name: "spine", parent: "hips", offset: [0, s.spine, 0] },
    { name: "chest", parent: "spine", offset: [0, s.chest, 0] },
    { name: "neck", parent: "chest", offset: [0, s.neck, 0] },
    { name: "head", parent: "neck", offset: [0, s.head, 0] },
  ];
  for (const group of [
    arm(s, f, "R", RIGHT),
    arm(s, f, "L", LEFT),
    leg(s, f, "R", RIGHT),
    leg(s, f, "L", LEFT),
  ]) {
    out.push(...group);
  }

  // Appended last: these hang off bones the groups above just created.
  out.push(
    // Strapped kit (shields) rides this rather than a matrix baked into the
    // body builder. The 70 degree rest roll is the negation of the left
    // forearm's guard-pose angle, which stands a shield upright.
    {
      name: "socket_shield.L",
      parent: "forearm.L",
      offset: [f.arm_r * 1.15, -s.lowerarm * 0.7, f.arm_r * 0.35],
      rest: [70, 0, 0],
    },
    // Held ball for keeper grab / carry / set. Parented to the chest so a
    // torso-driven clip carries it. Throw and punt reparent it to
    // socket_hand.R at runtime -- one socket cannot be in two hands.
    {
      name: "socket_ball",
      parent: "chest",
      offset: [0, s.chest * 0.34, f.torso_r * 1.45],
    },
  );
  return out;
}

// Bone name -> 0-based bone index, in the order `bones` lists them. Mesh
// builders bake this against a vertex; pure and independent of any posed
// instance, so geometry can be built long before a skeleton is instantiated.
export function boneIndex(rig: RigLike): Readonly<Record<string, number>> {
  const out: Record<string, number> = {};
  bones(rig).forEach((bone, i) => {
    out[bone.name] = i;
  });
  return out;
}

export function boneCount(rig: RigLike): number {
  return bones(rig).length;
}

const ZERO: Offset = [0, 0, 0];
const IDENTITY: Quat = quat.identity();

export function newRig(rig: RigLike): Rig {
  const defs = bones(rig);
  const byName: Record<string, PreparedBoneDef> = {};
  const order: string[] = [];
  const prepared: PreparedBoneDef[] = [];
  for (const def of defs) {
    if (def.parent !== null && !byName[def.parent]) {
      throw new Error(`parent must precede child: ${def.name}`);
    }
    const r = def.rest ?? [0, 0, 0];
    const q_rest = quat.fromEuler(
      (r[0] * Math.PI) / 180,
      (r[1] * Math.PI) / 180,
      (r[2] * Math.PI) / 180,
    );
    const withRest: PreparedBoneDef = { ...def, q_rest };
    byName[def.name] = withRest;
    prepared.push(withRest);
    order.push(def.name);
  }
  const out: Rig = { style: rig, defs: prepared, world: {}, byName, order };
  apply(out, { rot: {}, move: {} });
  return out;
}

/**
 * The same skeleton, standing `metres` higher off the pitch plane.
 *
 * WHY THIS IS A WHOLE-RIG PROPERTY AND NOT A POSE. The bone lengths above
 * describe a body; where that body's boots come to rest relative to y = 0 is a
 * property of the built character's GEOMETRY, which this file cannot see -- it
 * has no mesh, by charter. `rig3d/ground.ts` measures it once and hands the
 * answer here, so the offset is resolved at construction instead of being
 * re-measured sixty times a second. See that file's REST POSE section.
 *
 * It raises the ROOT's offset, which is the one bone whose translation is
 * world-space: `root` has no parent, so `apply` writes its local transform
 * straight into `world` and every descendant inherits that translation
 * unrotated. A metre here is a metre of world travel for every vertex in the
 * character, whatever the pose is doing -- the same property
 * `action_pose.ts`'s GROUND CONTACT note and `ground.poseAndGround` both rest
 * on.
 *
 * Returns a NEW rig; the input is untouched. It re-evaluates the rest pose, so
 * it is a construction-time call, not a per-frame one.
 *
 * NOT CANCELLED BY A BIND POSE, checked rather than assumed:
 * `player_renderer_3d.ts` builds its `THREE.Bone`s with an identity `matrix`
 * and never positions them, so `THREE.Skeleton`'s bone inverses are all
 * identity and the mesh's vertices are already bone-local. The skinning
 * transform is this rig's `world` outright, so raising it raises what is
 * drawn.
 */
export function raised(rig: Rig, metres: number): Rig {
  const defs = rig.defs.map((def) =>
    def.parent === null
      ? { ...def, offset: [def.offset[0], def.offset[1] + metres, def.offset[2]] as Offset }
      : def,
  );
  const byName: Record<string, PreparedBoneDef> = {};
  for (const def of defs) {
    byName[def.name] = def;
  }
  const out: Rig = { style: rig.style, defs, world: {}, byName, order: rig.order };
  apply(out, { rot: {}, move: {} });
  return out;
}

// Evaluates every bone's world transform for one pose. `pose.rot` carries
// already-baked quaternions and `pose.move` is an additive translation on top
// of the rest offset; both are sparse -- a bone the clip never mentions
// simply sits at rest.
//
// Clip translations are authored in metres for a full-size figure and scaled
// by the rig's motion_scale.
export function apply(rig: Rig, pose: Pose): void {
  const k = rig.style.motion_scale ?? 1;
  for (const bone of rig.defs) {
    const rot = pose.rot[bone.name] ?? IDENTITY;
    const move = pose.move[bone.name] ?? ZERO;
    const offset = bone.offset;

    // pose * rest, not a sum of angles. Every rest rotation in this rig is
    // about a single axis, so this is numerically identical to the old
    // summed-Euler form -- but it stays correct if a rest ever becomes
    // multi-axis, which summing would not.
    const localTf = quat.toMat4(
      quat.multiply(rot, bone.q_rest),
      offset[0] + move[0] * k,
      offset[1] + move[1] * k,
      offset[2] + move[2] * k,
    );

    if (bone.parent === null) {
      rig.world[bone.name] = localTf;
    } else {
      const parentWorld = rig.world[bone.parent];
      if (!parentWorld) {
        throw new Error(`parent must precede child: ${bone.name}`);
      }
      rig.world[bone.name] = mat4.multiply(parentWorld, localTf);
    }
  }
}

/** World-space position of a bone's origin. Used to place the drop shadow. */
export function jointPosition(rig: Rig, name: string): readonly [number, number, number] {
  const m = rig.world[name];
  if (!m) {
    throw new Error(`unknown bone: ${name}`);
  }
  return mat4.transformPoint(m, 0, 0, 0);
}
