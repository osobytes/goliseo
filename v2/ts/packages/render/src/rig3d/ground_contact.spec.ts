// Ground contact (#439): the property a player actually experiences.
//
// WHY THIS FILE EXISTS RATHER THAN A CLAMP TEST IN `action_pose.spec.ts`.
// AGENTS.md §9 draws the line this suite is built on: asserting that a clamp
// function clamps proves the clamp, not the game. #439 was a defect nobody
// could see in the pose tables -- `action_pose.ts` computed the drop
// correctly, wrote the number down in its own comment, and called 84 mm "a few
// millimetres" in the same sentence. Every existing unit pin passed while
// seven poses drove the character's ankles under the turf.
//
// So everything here goes through the REAL path: `body.accumulate` builds the
// real character geometry, `animator.poseFor` resolves the real pose (the
// locomotion blend, the stance layer, the root overlay), `skeleton.apply`
// evaluates the real bone transforms, and the assertion is on the lowest
// RENDERED VERTEX -- the same point a player sees disappear into the pitch.
// Nothing is re-derived: if the rig, the clips, the blend or the overlay
// change, this measures the change.
//
// The pitch plane is y = 0 in rig-local metres: `skeleton.ts` puts `root` at
// [0, 0, 0] and `player_renderer_3d.ts` hands the posed bones to three.js in
// that space, so a vertex at negative y is a vertex below the turf.

import { describe, expect, it } from "vitest";
import { mat4 } from "@gc/core";
import * as actionPose from "./action_pose.ts";
import * as animator from "./animator.ts";
import * as body from "./body.ts";
import * as clips from "./clips.ts";
import { RIG_MEDIUM } from "./proportions.ts";
import * as skeleton from "./skeleton.ts";
import * as themes from "./themes.ts";

const RIG = RIG_MEDIUM;

// `view_state.ts`'s own speeds, restated rather than imported: this file is in
// rig3d, which points only upward (see `animator.ts`'s layer-boundary note).
// `pose_table.locomotionBlend` is what reads them, and `pose_table.spec.ts`
// pins these exact two values against `view_state.ts`.
const WALK_SPEED = 150;
const RUN_SPEED = 400;

// Everything below is compared against millimetres, so the tolerance has to be
// far finer than the effect. A tenth of a millimetre is ~0.006% of the
// character's height and ~1/700th of the smallest sink #439 lists.
const MM = 0.001;
const TOLERANCE = 0.0001;

const THEME = themes.LIST[0];
const FIGURE = themes.FIGURES[0];
if (THEME === undefined || FIGURE === undefined) {
  throw new Error("ground_contact.spec.ts: themes.LIST/FIGURES must not be empty");
}
// One character, built once: `accumulate` is pure and the geometry never
// changes, only the bone transforms it rides.
const [MESH] = body.accumulate(RIG, THEME, FIGURE);
const BONE_ORDER = skeleton.bones(RIG).map((b) => b.name);

/**
 * The lowest rendered point of a posed character, in metres, and its bone.
 *
 * `skip` narrows it to part of the figure -- used to separate the keeper's own
 * body from the shield and sword hanging off the socket bones, which a hard
 * root roll swings deeper than anything anatomical (see SAVE_RESIDUALS).
 */
function lowestRendered(rig: skeleton.Rig, skip?: (bone: string) => boolean): { y: number; bone: string } {
  let y = Infinity;
  let bone = "";
  for (const vertex of MESH.verts) {
    const name = BONE_ORDER[vertex.bone];
    if (name !== undefined && skip?.(name) === true) {
      continue;
    }
    const world = name !== undefined ? rig.world[name] : undefined;
    if (world === undefined) {
      throw new Error(`ground_contact.spec.ts: vertex on unknown bone index ${vertex.bone}`);
    }
    const p = mat4.transformPoint(world, vertex.position[0], vertex.position[1], vertex.position[2]);
    if (p[1] < y) {
      y = p[1];
      bone = name ?? "";
    }
  }
  return { y, bone };
}

interface Frame {
  readonly speed: number;
  readonly gait: number;
  readonly now: number;
}

// A stride sampled finely enough that a pose cannot slip between two phases,
// at a standstill, at a walk and at a full run.
const PHASES = 24;
function frames(): Frame[] {
  const out: Frame[] = [];
  for (const speed of [0, WALK_SPEED, RUN_SPEED]) {
    for (let i = 0; i < PHASES; i += 1) {
      out.push({ speed, gait: i / PHASES, now: i * 0.037 });
    }
  }
  return out;
}

let poseSeq = 0;
/**
 * One real frame for one pose id, evaluated onto the rig.
 *
 * A fresh `playerId` per call on purpose: `animator.poseFor` keeps a per-player
 * crossfade, and a shared id would make one sample depend on the one before
 * it. Every sample here is a character seen on its own first frame, which is
 * also the frame that commits to its stance outright.
 */
function poseOnRig(
  rig: skeleton.Rig,
  id: string | undefined,
  frame: Frame,
  extra?: Partial<actionPose.ActionPoseOptions>,
): actionPose.MutablePose {
  poseSeq += 1;
  const opts =
    id === undefined ? {} : { pose: { id }, dive_dir: { x: 1, y: 0 }, facing: { x: 0, y: 1 }, ...extra };
  const pose = animator.poseFor(`gc_${String(poseSeq)}`, { speed: frame.speed, gait: frame.gait }, opts, frame.now);
  skeleton.apply(rig, pose);
  return pose;
}

/** The seven poses #439 names: every entry in `TIPS`/`ATTITUDES` with a drop. */
const DROP_POSES = [
  "keeper_ready_low",
  "settle",
  "combat_stagger",
  "contain",
  "keeper_get_up",
  "keeper_set",
  "fatigue",
] as const;

/** Poses whose root overlay carries a rotation as well as (or instead of) a drop. */
const ROTATING = new Set(["combat_stagger", "keeper_get_up", "contain", "fatigue"]);

describe("ground contact: the rig's own clearance", () => {
  // The number the whole fix rests on. If this ever goes positive the rig has
  // grown real clearance and `GROUND_CLEARANCE_METRES` can be raised
  // deliberately; if it goes sharply negative the rig itself has sunk and this
  // is where you find out.
  it("plants a standing character on the plane, with no clearance to spend", () => {
    const rig = skeleton.newRig(RIG);
    const lowest = lowestRendered(rig);
    expect(lowest.bone, "the lowest point of a standing character is a boot").toMatch(/^(foot|toe)\./);
    expect(lowest.y, "the rig is authored to plant, not to hover").toBeLessThanOrEqual(0);
    expect(lowest.y, "and it plants within a millimetre or two of the plane").toBeGreaterThan(-2 * MM);
  });

  // `action_pose.apply` floors the OVERLAY's downward component rather than the
  // sum, which is only conservative if the other summand never goes down. It
  // does not: the walk and run clips bottom out at exactly 0 on their contact
  // keys. Pinned here because the floor's correctness depends on it.
  it("never lets the locomotion blend write a downward root translation", () => {
    const rig = skeleton.newRig(RIG);
    let lowestBob = Infinity;
    for (const frame of frames()) {
      const pose = poseOnRig(rig, undefined, frame);
      lowestBob = Math.min(lowestBob, pose.move["root"]?.[1] ?? 0);
    }
    expect(lowestBob, "the gait's bob is a rise, never a sink").toBeGreaterThanOrEqual(0);
  });

  // The other half of that reasoning, and the reason the budget is measured on
  // a STANDING figure: a moving one is nowhere near the plane. Loose bounds on
  // purpose -- the exact peak is a clip-tuning detail, but "a walking figure is
  // never near the turf and a running one is sometimes a hand's width above it"
  // is the fact a per-frame budget would have been read off, and it is what
  // makes such a budget vary by more than the drops it was meant to bound.
  it("floats a moving figure far off the plane, which is why the budget is measured standing", () => {
    const rig = skeleton.newRig(RIG);
    const peak: Record<string, { min: number; max: number }> = {};
    for (const frame of frames()) {
      const key = frame.speed === 0 ? "idle" : frame.speed === WALK_SPEED ? "walk" : "run";
      poseOnRig(rig, undefined, frame);
      const y = lowestRendered(rig).y;
      const seen = peak[key] ?? { min: Infinity, max: -Infinity };
      peak[key] = { min: Math.min(seen.min, y), max: Math.max(seen.max, y) };
    }
    expect(peak["idle"]?.min, "a standing figure is on the plane").toBeGreaterThan(-2 * MM);
    expect(peak["idle"]?.max, "and stays there").toBeLessThan(5 * MM);
    expect(peak["walk"]?.min, "a walk never brings the boots back to the turf").toBeGreaterThan(10 * MM);
    expect(peak["run"]?.min, "nor does a run").toBeGreaterThan(15 * MM);
    expect(peak["run"]?.max, "and a run peaks a hand's width up").toBeGreaterThan(100 * MM);
  });
});

describe("ground contact: downward root translations (#439)", () => {
  // THE PROPERTY, stated exactly. A downward root translation may not lower
  // the character past the ground clearance the rig has -- which on RIG_MEDIUM
  // is none. Measured against the same frame with the overlay's translation
  // removed, so the pose's own rotation (which this fix deliberately does not
  // correct) cannot mask a regression in the translation.
  it("costs the character no depth it did not already have, in every pose and at every phase", () => {
    const rig = skeleton.newRig(RIG);
    for (const id of DROP_POSES) {
      for (const frame of frames()) {
        const posed = poseOnRig(rig, id, frame);
        const withTranslation = lowestRendered(rig).y;

        // The same frame, with the root translation channel emptied. Every
        // rotation, every clip, every blend weight is identical.
        const root = posed.move["root"];
        posed.move["root"] = [root?.[0] ?? 0, 0, root?.[2] ?? 0];
        skeleton.apply(rig, posed);
        const withoutTranslation = lowestRendered(rig).y;

        expect(
          withTranslation,
          `${id} at speed ${String(frame.speed)} phase ${frame.gait.toFixed(2)}: the root translation must not push the character below the plane`,
        ).toBeGreaterThan(withoutTranslation - TOLERANCE);
      }
    }
  });

  // The absolute form of the same claim, for the poses whose overlay is a drop
  // and nothing else. These are the ones that can be checked against the pitch
  // itself rather than against a baseline.
  it("keeps a drop-only pose on the turf it stands on", () => {
    const rig = skeleton.newRig(RIG);
    const standing = { speed: 0, gait: 0, now: 0 };
    for (const id of DROP_POSES) {
      if (ROTATING.has(id)) {
        continue;
      }
      poseOnRig(rig, id, standing);
      const lowest = lowestRendered(rig).y;
      expect(lowest, `${id} must not stand in a hole`).toBeGreaterThan(-2 * MM);
      // The sinks #439 measured, so a revert is a named regression rather than
      // an unexplained number: keeper_ready_low 74 mm, settle 69 mm,
      // keeper_set 41 mm.
      expect(lowest, `${id} used to sink tens of millimetres`).toBeGreaterThan(-10 * MM);
    }
  });

  // THE COST, PINNED RATHER THAN DESCRIBED. `pose_table.ts` says
  // `keeper_ready_low` "is LOW because of the attitude -- without it the two
  // stances differed only in crossfade duration", and grounding the drop takes
  // exactly that attitude away. So the two keeper stances now resolve to the
  // same held pose, and this says so in the suite instead of in a comment
  // nobody re-reads: it is the disclosure, and it goes red the moment a knee
  // bend gives `keeper_ready_low` its silhouette back -- which is the point at
  // which someone should be looking at this again.
  it("collapses keeper_set and keeper_ready_low into the same held pose", () => {
    const standing = { speed: 0, gait: 0, now: 0 };
    const rigA = skeleton.newRig(RIG);
    const rigB = skeleton.newRig(RIG);
    const set = poseOnRig(rigA, "keeper_set", standing);
    const readyLow = poseOnRig(rigB, "keeper_ready_low", standing);
    expect(readyLow, "the two stances differ only in the drop, which is now floored").toEqual(set);
    expect(lowestRendered(rigB).y, "and they render identically").toBeCloseTo(lowestRendered(rigA).y, 9);
    // Not a claim that the TABLE stopped distinguishing them: it still does,
    // and the day the rig can absorb a crouch they part again.
    const authored = actionPose.attitudeFor({ pose: { id: "keeper_ready_low" } })?.move.root?.[1] ?? 0;
    const authoredSet = actionPose.attitudeFor({ pose: { id: "keeper_set" } })?.move.root?.[1] ?? 0;
    expect(authored, "ATTITUDES still authors the deeper crouch").toBeLessThan(authoredSet);
  });

  // Upward is not this fix's business: an aerial, a knockback, a save and a
  // stumble all mean to leave the ground, and a floor that touched them would
  // be a new defect rather than a fix for an old one.
  it("leaves every upward and lateral root translation exactly as authored", () => {
    const cases: { id: string; extra?: Partial<actionPose.ActionPoseOptions> }[] = [
      { id: "aerial_bicycle", extra: { aerial: 1, aerial_style: "bicycle", aerial_jump: 1 } },
      { id: "aerial_action", extra: { aerial: 1, aerial_style: "chest_control", aerial_jump: 0.4 } },
      { id: "combat_knockback" },
      { id: "stumble" },
      { id: "keeper_dive", extra: { dive: 1 } },
      { id: "keeper_stretch", extra: { dive: 1 } },
    ];
    for (const c of cases) {
      const opts: actionPose.ActionPoseOptions = {
        pose: { id: c.id },
        dive_dir: { x: 1, y: 0 },
        facing: { x: 0, y: 1 },
        ...c.extra,
      };
      const authored = actionPose.forOptions(opts)?.move.root;
      const applied = actionPose.apply({ rot: {}, move: {} }, opts).move["root"];
      expect(applied, `${c.id} must reach the skeleton exactly as authored`).toEqual(authored);
    }
  });

  // A clip may drop the root, and SWING does -- 32 mm at its deepest lunge key,
  // which is 28 mm of world travel once `skeleton.apply` has applied the rig's
  // motion_scale, since that multiply is uniform and does not care whether a
  // clip or an overlay wrote the value.
  // It does not penetrate, because its own thigh and shin keys raise the feet
  // by more than it lowers the hips, which is what limb work buys and what the
  // root-only overlay cannot do. Pinned so that "the clips are fine" stays a
  // measurement rather than a memory.
  it("leaves a clip's own root motion alone, because its limbs pay for it", () => {
    const rig = skeleton.newRig(RIG);
    let deepestKey = 0;
    for (const key of clips.SWING.keys) {
      deepestKey = Math.min(deepestKey, key.move["root"]?.[1] ?? 0);
    }
    expect(deepestKey, "SWING really does translate the root downward").toBeLessThan(-20 * MM);
    // The authored key and the world travel it becomes, both measured, so the
    // two numbers in the comments above cannot drift apart from each other.
    expect(deepestKey * RIG.motion_scale, "and 32 mm authored is 28 mm travelled").toBeGreaterThan(-30 * MM);
    expect(deepestKey * RIG.motion_scale, "and 32 mm authored is 28 mm travelled").toBeLessThan(-26 * MM);

    let lowest = Infinity;
    for (let i = 0; i <= 40; i += 1) {
      // `combat_active` is the pose id `pose_table.ts` maps to SWING.
      poseOnRig(rig, "combat_active", { speed: 0, gait: 0, now: (i / 40) * clips.SWING.duration });
      lowest = Math.min(lowest, lowestRendered(rig).y);
    }
    expect(lowest, "a swing's lunge stays on the pitch on its own merits").toBeGreaterThan(-2 * MM);
  });
});

// THE DISCLOSED GAP (#439's "not in scope"). Root ROTATIONS move feet too, and
// this fix does not correct them: a tilt about a root at y = 0 swings the far
// boot below the plane, and undoing that means deciding what a keeper dive
// should look like once its pivot moves to the pitch -- a visual question, not
// a unit one.
//
// Measured here rather than described, for two reasons. It keeps the gap
// honest, and it makes it a REGRESSION TEST: these are floors, so the residual
// cannot quietly deepen while everyone assumes #439 dealt with it.
describe("ground contact: what root rotations still do (out of scope, measured)", () => {
  const rig = skeleton.newRig(RIG);
  const standing = { speed: 0, gait: 0, now: 0 };

  // Pose -> the depth its ROTATION alone leaves, in millimetres, once the drop
  // is grounded. Each is a FLOOR: the standing measurement rounded up to the
  // next millimetre, so the assertion reads "no deeper than this".
  //
  // Everything in THIS table takes no amount: `TIPS` and `ATTITUDES` are
  // constants, so one measurement is the whole story. The saves are not, and
  // they are pinned separately below -- see the note there, which is the more
  // important half of this block.
  const RESIDUAL_MM: Readonly<Record<string, number>> = {
    keeper_get_up: 50,
    run_telegraph: 24,
    kick_follow: 13,
    combat_stagger: 11,
    contain: 8,
    fatigue: 4,
  };

  for (const [id, mm] of Object.entries(RESIDUAL_MM)) {
    it(`${id}: a root tilt still reaches ${String(mm)} mm below the plane`, () => {
      poseOnRig(rig, id, standing);
      const lowest = lowestRendered(rig).y;
      expect(lowest, `${id}'s rotation must not be deepening`).toBeGreaterThan(-mm * MM);
      expect(lowest, `${id}'s residual is a rotation, so it does not vanish`).toBeLessThan(0);
    });
  }

  // THE SAVES ARE SCALED BY AN AMOUNT, AND A PIN THAT FORGETS TO PASS ONE
  // MEASURES NOTHING.
  //
  // `save()` computes `amount = spec.fixed ?? clamp(opts.dive ?? 0, 0, 1)` and
  // multiplies BOTH the roll and the travel by it. Omit `dive` and
  // `keeper_dive` resolves to a 0-degree roll and a 0 m slide -- the standing
  // rig, -1.2 mm, which sails under any floor you care to write. The first
  // version of this block did exactly that and pinned "111 mm" against a pose
  // that was not happening. It would have passed with the dive maths deleted.
  //
  // So every entry here names its amount and the table is indexed BY amount,
  // which also states the thing a single number cannot: THE RESIDUAL IS NOT A
  // CONSTANT. `keeper_dive` spans 57 mm to 412 mm across the amounts a real
  // dive passes through, and 111 mm -- the figure this PR first disclosed -- is
  // the halfway point, not the ceiling.
  //
  // The ceiling is not hypothetical either. `gc-render`'s frame builder pushes
  // `eased(dive_timer, DIVE_EASE)` = `min(timer / 0.3, 1)`, and
  // `KEEPER_DIVE_DURATION` is 0.32 s, so every dive STARTS at a saturated 1.0
  // and eases down. The deepest row below is the opening frame of every save
  // in the game, not a corner case.
  //
  // BODY AND PROPS ARE REPORTED SEPARATELY because they are different facts. A
  // 72-degree root roll swings the whole arm chain, and the shield hanging off
  // `socket_shield.L` reaches more than twice as deep as any part of the
  // keeper: at full dive, 192 mm of body and 412 mm of shield. Conflating them
  // would let a shield fix look like a body fix.
  const PROP = (bone: string): boolean => bone.startsWith("socket_");

  /** id, the `dive` passed in, and the floors: body-only, then including props. */
  const SAVE_RESIDUALS: readonly { id: string; dive: number; body: number; props: number }[] = [
    // Amount-driven: no `fixed`, no `floor`, so these scale from nothing.
    { id: "keeper_dive", dive: 0.25, body: 57, props: 57 },
    { id: "keeper_dive", dive: 0.5, body: 111, props: 111 },
    { id: "keeper_dive", dive: 1, body: 192, props: 412 },
    { id: "keeper_spread", dive: 1, body: 88, props: 88 },
    { id: "keeper_central", dive: 1, body: 141, props: 141 },
    // `floor: 0.82`, so this one is already 82% deep with no amount at all.
    { id: "keeper_stretch", dive: 0, body: 171, props: 294 },
    { id: "keeper_stretch", dive: 1, body: 279, props: 502 },
    // `fixed: 1`, so the amount is ignored entirely and it is always this deep.
    { id: "keeper_tip", dive: 0, body: 362, props: 595 },
  ];

  for (const row of SAVE_RESIDUALS) {
    it(`${row.id} at dive ${String(row.dive)}: ${String(row.body)} mm of body, ${String(row.props)} mm with props`, () => {
      poseOnRig(rig, row.id, standing, { dive: row.dive });
      const body = lowestRendered(rig, PROP).y;
      const all = lowestRendered(rig).y;
      expect(body, `${row.id}'s body residual must not be deepening`).toBeGreaterThan(-row.body * MM);
      expect(all, `${row.id}'s residual including props must not be deepening`).toBeGreaterThan(-row.props * MM);
      expect(all, `${row.id} at dive ${String(row.dive)} is a rotation, so it does not vanish`).toBeLessThan(-2 * MM);
    });
  }

  // The guard that makes the table above non-vacuous, stated as a property
  // rather than left to whoever writes the next row: if a save's amount ever
  // stops reaching the pose, these floors go back to measuring a standing rig
  // and every one of them passes. `keeper_tip` is `fixed` and `keeper_stretch`
  // is floored, so they are deep with no amount at all; the other three must
  // be at the standing baseline without one, and far past it with one.
  it("proves each save's residual is driven by its amount, not by the rig standing still", () => {
    const baseline = lowestRendered(skeleton.newRig(RIG)).y;
    for (const id of ["keeper_dive", "keeper_spread", "keeper_central"]) {
      poseOnRig(rig, id, standing, { dive: 0 });
      expect(lowestRendered(rig).y, `${id} with no dive amount is just a standing character`).toBeCloseTo(baseline, 6);
      poseOnRig(rig, id, standing, { dive: 1 });
      expect(lowestRendered(rig).y, `${id} at full dive must be far past standing`).toBeLessThan(baseline - 50 * MM);
    }
    for (const id of ["keeper_stretch", "keeper_tip"]) {
      poseOnRig(rig, id, standing, { dive: 0 });
      expect(lowestRendered(rig).y, `${id} is fixed or floored, so it is deep with no amount`).toBeLessThan(baseline - 50 * MM);
    }
  });
});
