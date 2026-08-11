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
import * as ground from "./ground.ts";
import * as poseTable from "./pose_table.ts";
import * as presentationContent from "./presentation_content.ts";
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

/** Anything hanging off a socket bone: the sword and the shield. */
const PROP = (bone: string): boolean => bone.startsWith("socket_");

// The probes the renderer builds, from the same geometry, over the same bone
// order (`player_renderer_3d.ts`'s `build`).
const PROBES = ground.probesFrom(RIG, MESH.verts, BONE_ORDER);

// TWO RIGS, AND THE DIFFERENCE BETWEEN THEM IS THE POINT.
//
// `flatRig` is the skeleton as `proportions.ts` authors it -- what the game
// rendered before #446, and what every UNGROUNDED measurement in this file is
// taken against, so the figures #446 quotes stay the figures this suite pins.
// It plants 1.2 mm under the plane; the first test below measures exactly
// that, and it is where `PROBES.restLift` comes from.
//
// `groundedRig` is what `player_renderer_3d.ts`'s `build` actually poses:
// raised by that constant once, at construction, so a character who is doing
// nothing measures >= 0 and `poseAndGround`'s early-out fires instead of
// paying a second skeleton evaluation sixty times a second. Anything
// asserting what a PLAYER SEES goes through this one.
const flatRig = (): skeleton.Rig => skeleton.newRig(RIG);
const groundedRig = (): skeleton.Rig => skeleton.raised(skeleton.newRig(RIG), PROBES.restLift);

// The same character with nothing in its hands. This was written before #447
// as a projection of what a keeper WOULD be; it is now what a keeper IS.
const BODY_ONLY_PROBES = ground.probesFrom(
  RIG,
  MESH.verts.filter((v) => {
    const name = BONE_ORDER[v.bone];
    return name !== undefined && !PROP(name);
  }),
  BONE_ORDER,
);

// ---------------------------------------------------------------------------
// THE TWO FIGURES #447 MADE REAL
// ---------------------------------------------------------------------------
//
// Before #447 there was one character mesh in the game, built from
// `themes.LIST[0]`'s own authored loadout -- a tournament sword AND a heater
// shield -- and every player wore it, keepers included. That is why this file
// could measure "body" and "props" as two columns of ONE figure: there was no
// second figure to build.
//
// There is now. `body.accumulate` takes the loadout the player's authored
// `loadout_id` resolves to, and a keeper's is empty, so a keeper's mesh is
// genuinely a different mesh with no equipment vertices in it. The tables
// below are stated against whichever figure the pose belongs to:
//
//   * KEEPER_MESH  -- a keeper. No props at all, so `body` and `props` are
//                     the same measurement, which is exactly what #447
//                     predicted would happen to the `props` column.
//   * SHIELD_MESH  -- an outfield defender carrying `loadout_emberguard_shield`
//                     (brakka, drell and sela_dwin in `gc-data`). Still has a
//                     shield on `socket_shield.L`, which is what keeps the
//                     `combat_knockback` case below meaningful.
//   * MESH         -- the theme's own authored sword+shield figure, still what
//                     `draw`/`renderToSprite` and this file's non-#447 blocks
//                     render, and the figure every ungrounded number in the
//                     #439 blocks was measured against.
const KEEPER_MESH = body.accumulate(RIG, THEME, FIGURE, presentationContent.loadoutFor(undefined))[0];
const KEEPER_PROBES = ground.probesFrom(RIG, KEEPER_MESH.verts, BONE_ORDER);
const SHIELD_MESH = body.accumulate(RIG, THEME, FIGURE, presentationContent.loadoutFor("loadout_emberguard_shield"))[0];
const SHIELD_PROBES = ground.probesFrom(RIG, SHIELD_MESH.verts, BONE_ORDER);

/**
 * The lowest rendered point of a posed character, in metres, and its bone.
 *
 * `skip` narrows it to part of the figure -- used to separate a player's own
 * body from anything hanging off the socket bones, which a hard root roll
 * swings deeper than anything anatomical (see SAVE_RESIDUALS).
 *
 * `verts` picks WHICH figure is being measured (#447). It defaults to the
 * sword-and-shield `MESH` every block above this one was written against, so
 * those numbers keep meaning what they meant; the keeper blocks below pass
 * `KEEPER_MESH.verts` instead, because that is what a keeper now renders.
 */
function lowestRendered(
  rig: skeleton.Rig,
  skip?: (bone: string) => boolean,
  verts: readonly { readonly bone: number; readonly position: readonly [number, number, number] }[] = MESH.verts,
): { y: number; bone: string } {
  let y = Infinity;
  let bone = "";
  for (const vertex of verts) {
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
 * One real frame for one pose id, evaluated onto the rig -- WITHOUT #446's
 * ground lift.
 *
 * This is the path `player_renderer_3d.ts` used before #446 and is kept as
 * the baseline the lift is measured against: the #439 blocks below are about
 * `action_pose.apply`'s translation floor, which is upstream of the lift and
 * has to keep being testable on its own. Anything asserting what a PLAYER
 * SEES goes through `groundedOnRig` instead.
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

/**
 * The same frame through the REAL display path (#446): `animator.poseFor`
 * then `ground.poseAndGround`, which is what `characterMesh` and
 * `prepareCharacter` both call. Returns the pose and the lift in metres.
 *
 * One step of those two is missing here and is missing on purpose:
 * `player_renderer_3d.ts` also applies the velocity-derived torso lean
 * between the two calls, and that lives above rig3d (its magnitude is derived
 * from `ppmForRadius`, which is the renderer's). It cannot change any answer
 * in this file -- `applyLean` rotates `spine` and `chest` only, so it moves
 * nothing below the hips, and it is gated on `forOptions(opts) === null`, so
 * it never runs during a save at all. The grounding itself happens AFTER it
 * either way, so the product path is grounded against the lean; this suite
 * simply does not exercise a lean it cannot reach from here.
 */
function groundedOnRig(
  rig: skeleton.Rig,
  id: string | undefined,
  frame: Frame,
  extra?: Partial<actionPose.ActionPoseOptions>,
  probes: ground.GroundProbes = PROBES,
): { pose: actionPose.MutablePose; lift: number } {
  poseSeq += 1;
  const opts =
    id === undefined ? {} : { pose: { id }, dive_dir: { x: 1, y: 0 }, facing: { x: 0, y: 1 }, ...extra };
  const pose = animator.poseFor(`gc_${String(poseSeq)}`, { speed: frame.speed, gait: frame.gait }, opts, frame.now);
  const lift = ground.poseAndGround(rig, pose, probes);
  return { pose, lift };
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

/**
 * The five `ATTITUDES` entries whose drop is a CROUCH, and which #445 pays for
 * with a knee bend instead of leaving floored to nothing.
 *
 * The split matters to the assertions below. A floored root drop is not there
 * at all, so it can be checked by removing the translation channel and
 * confirming nothing moved. A crouch's translation IS there -- the limbs raised
 * the feet by exactly as much first -- so the same removal would leave a figure
 * standing on air, and the honest check is the one #445 is actually about:
 * the pelvis went down by the authored amount and the soles did not.
 */
const CROUCH_POSES = ["keeper_ready_low", "settle", "contain", "keeper_set", "fatigue"] as const;

/** The drops that are still root-only, and still floored away: `TIPS`'. */
const FLOORED_DROP_POSES = DROP_POSES.filter(
  (id) => !(CROUCH_POSES as readonly string[]).includes(id),
);

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
  //
  // NARROWED TO THE STILL-FLOORED DROPS BY #445, and the narrowing is the
  // point rather than a retreat from it. This baseline -- "the same frame with
  // `move.root.y` zeroed" -- is only a baseline while the translation is the
  // ONLY thing delivering the drop. A crouch's translation is paid for by a
  // knee bend that has already raised the feet by the same amount, so zeroing
  // it leaves a figure standing on air, and "with the translation is not lower
  // than without it" would be asserting the opposite of the fix. The five
  // crouches get the stronger, absolute property instead, two blocks below:
  // the pelvis drops by exactly what `ATTITUDES` authors and the soles stay on
  // the turf. `TIPS`' two drops are still root-only and still floored to
  // nothing, and they are what this measures.
  it("costs the character no depth it did not already have, in every pose and at every phase", () => {
    const rig = skeleton.newRig(RIG);
    expect(FLOORED_DROP_POSES, "`TIPS`' two drops, or this loop measures nothing").toEqual([
      "combat_stagger",
      "keeper_get_up",
    ]);
    for (const id of FLOORED_DROP_POSES) {
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

  // THE COST, PAID OFF (#445). This test used to assert the opposite, and the
  // history is the point of keeping it here rather than writing a new one
  // elsewhere.
  //
  // `pose_table.ts` says `keeper_ready_low` "is LOW because of the attitude --
  // without it the two stances differed only in crossfade duration", and #444's
  // floor took exactly that attitude away: from #444 to #445 the two keeper
  // stances resolved to DEEP-EQUAL poses and rendered to the same millimetre.
  // That equality was pinned deliberately, as a disclosure that would go red
  // the day a knee bend gave `keeper_ready_low` its silhouette back. This is
  // that day, so the equality is replaced by its opposite -- with a MAGNITUDE,
  // because "not equal" would also be satisfied by a micron.
  //
  // The second half is unchanged and is what keeps either version honest: it
  // says the difference comes from `ATTITUDES` still authoring one crouch
  // deeper than the other, not from the table having stopped distinguishing
  // them.
  it("renders keeper_set and keeper_ready_low at visibly different heights", () => {
    const standing = { speed: 0, gait: 0, now: 0 };
    const rigA = skeleton.newRig(RIG);
    const rigB = skeleton.newRig(RIG);
    poseOnRig(rigA, "keeper_set", standing);
    poseOnRig(rigB, "keeper_ready_low", standing);

    const setHips = skeleton.jointPosition(rigA, "hips")[1];
    const lowHips = skeleton.jointPosition(rigB, "hips")[1];
    const setHead = skeleton.jointPosition(rigA, "head")[1];
    const lowHead = skeleton.jointPosition(rigB, "head")[1];

    // 32 mm of pelvis, on a 1.57 m figure drawn ~26 px tall at match camera:
    // about half a pixel of separation between two stances that were the same
    // pose a release ago. The whole upper body carries it, so the head parts by
    // the same amount rather than the difference being a detail of the legs.
    expect(lowHips, "the LOW ready stance is the lower of the two").toBeLessThan(setHips);
    expect(setHips - lowHips, "and by tens of millimetres, not by a micron").toBeGreaterThan(30 * MM);
    expect(setHead - lowHead, "the head drops with the pelvis, so the silhouette differs").toBeCloseTo(
      setHips - lowHips,
      9,
    );

    // The separation is exactly what the table authors, not an artefact of the
    // fold: `crouch.ts` derives its angle FROM `attitudeDrop`, so a re-tune of
    // either entry moves this by the same amount.
    const authoredGap =
      (actionPose.attitudeDrop({ pose: { id: "keeper_ready_low" } }) -
        actionPose.attitudeDrop({ pose: { id: "keeper_set" } })) *
      RIG.motion_scale;
    expect(setHips - lowHips, "and it is the authored gap, in world metres").toBeCloseTo(authoredGap, 9);

    // Not a claim that the difference is the RIG's: it is the table's, and it
    // was the table's throughout the release in which nothing showed it.
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

// ---------------------------------------------------------------------------
// THE KNEE BEND (#445)
// ---------------------------------------------------------------------------
//
// #444 grounded the drop and disclosed, in this file, that five poses lost
// their crouch as a result -- because a root translation moves the feet with
// the body and this rig has no IK to plant them again. The fix is not a bigger
// translation and not a smaller floor: it is limb work, exactly the pattern
// SWING already demonstrates two tests above.
//
// So the property to measure is the one a player sees: THE PELVIS GOES DOWN AND
// THE SOLES DO NOT. Both halves are measured through the real geometry, and
// each is what makes the other non-vacuous -- a fold that raised the feet
// without dropping the hips is a player standing on tiptoe, and a drop without
// the fold is #439 again.
describe("ground contact: the crouch is limb work (#445)", () => {
  const standing = { speed: 0, gait: 0, now: 0 };

  /** The soles and the pelvis of a character doing nothing at all. */
  const rest = (): { sole: number; hips: number } => {
    const rig = skeleton.newRig(RIG);
    return { sole: lowestRendered(rig).y, hips: skeleton.jointPosition(rig, "hips")[1] };
  };

  // The three crouches that carry no lean, so the drop is the whole overlay and
  // the arithmetic is exact rather than "within the tilt".
  const PLUMB_CROUCHES = CROUCH_POSES.filter((id) => !ROTATING.has(id));

  it("settles the pelvis by exactly the drop ATTITUDES authors", () => {
    expect(PLUMB_CROUCHES, "the drop-only crouches, or this measures nothing").toEqual([
      "keeper_ready_low",
      "settle",
      "keeper_set",
    ]);
    const base = rest();
    for (const id of PLUMB_CROUCHES) {
      const rig = skeleton.newRig(RIG);
      poseOnRig(rig, id, standing);
      // The authored magnitude, converted the one way `skeleton.apply` converts
      // it. Not a copied decimal: re-tune `ATTITUDES` and this follows.
      const authored = actionPose.attitudeDrop({ pose: { id } }) * RIG.motion_scale;
      expect(authored, `${id} authors a crouch worth seeing`).toBeGreaterThan(30 * MM);
      expect(
        base.hips - skeleton.jointPosition(rig, "hips")[1],
        `${id}'s pelvis settles by its authored drop`,
      ).toBeCloseTo(authored, 9);
    }
  });

  it("leaves the soles on the turf while it does it", () => {
    const base = rest();
    for (const id of PLUMB_CROUCHES) {
      const rig = skeleton.newRig(RIG);
      poseOnRig(rig, id, standing);
      const sole = lowestRendered(rig);
      expect(sole.bone, `${id}'s lowest point is still a boot`).toMatch(/^(foot|toe)\./);
      // NEVER BELOW where a standing character's sole is: the fold is derived
      // from the drop rather than tuned against it, so the residue is the 2
      // degree rest roll on the thigh that the closed form does not model --
      // 0.06% of the drop, and in the direction of clearance.
      expect(sole.y, `${id} must not push a sole below where standing puts it`).toBeGreaterThanOrEqual(base.sole);
      expect(sole.y - base.sole, `${id}'s residue is a fraction of a millimetre`).toBeLessThan(0.2 * MM);
    }
  });

  // THE OTHER HALF, and the reason the two above are not satisfied by a pose
  // that does nothing. Remove the crouch's own root translation and the figure
  // must FLOAT by the drop -- which is precisely the measurement that says the
  // limbs, not the pitch, paid for it. It is also the exact baseline the #439
  // block deliberately no longer uses for these five poses, kept here with its
  // sign the right way round.
  it("floats the figure by the whole drop when its root translation is removed", () => {
    const base = rest();
    for (const id of PLUMB_CROUCHES) {
      const rig = skeleton.newRig(RIG);
      const posed = poseOnRig(rig, id, standing);
      const root = posed.move["root"];
      posed.move["root"] = [root?.[0] ?? 0, 0, root?.[2] ?? 0];
      skeleton.apply(rig, posed);
      const authored = actionPose.attitudeDrop({ pose: { id } }) * RIG.motion_scale;
      const float = lowestRendered(rig).y - base.sole;
      expect(
        float,
        `${id}'s fold raises the feet by at least the drop its root translation spends`,
      ).toBeGreaterThanOrEqual(authored);
      expect(float, `${id}'s fold overshoots by the 2 degree rest roll and no more`).toBeLessThan(
        authored + 0.2 * MM,
      );
    }
  });

  // The two crouches that ALSO lean. The lean is a root tilt, so the pelvis
  // travels a little further than the drop alone and the exact equality above
  // does not apply -- but the crouch must still be all there, and the tilt's
  // own residual must not have grown. Both figures are the ones `RESIDUAL_MM`
  // pins below, so the two blocks cannot drift apart.
  it("keeps the crouch under a leaning pose without deepening its tilt", () => {
    const base = rest();
    for (const id of ["contain", "fatigue"] as const) {
      const rig = skeleton.newRig(RIG);
      poseOnRig(rig, id, standing);
      const authored = actionPose.attitudeDrop({ pose: { id } }) * RIG.motion_scale;
      expect(authored, `${id} authors a crouch`).toBeGreaterThan(30 * MM);
      expect(
        base.hips - skeleton.jointPosition(rig, "hips")[1],
        `${id}'s pelvis settles at least its authored drop`,
      ).toBeGreaterThan(authored - TOLERANCE);
      expect(
        base.hips - skeleton.jointPosition(rig, "hips")[1],
        `${id}'s lean adds a few more millimetres and no more`,
      ).toBeLessThan(authored + 10 * MM);
    }
  });

  // THE COST PROPERTY, as a count rather than a depth -- the same shape as
  // "costs an idle, walking or running character no second skeleton
  // evaluation", and for the same reason. A crouch is a HELD posture, so a
  // per-frame lift on one would track the gait's own vertical bob and read as a
  // wobble at stride frequency; `action_pose.ts`'s GROUND CLEARANCE note
  // rejects a per-frame budget for exactly that. A keeper is the figure to
  // measure it on: keepers hold both of the deepest crouches and, since #447,
  // carry nothing.
  it("costs a crouching keeper no per-frame lift, at any phase or speed", () => {
    const rig = skeleton.raised(skeleton.newRig(RIG), KEEPER_PROBES.restLift);
    let lifted = 0;
    let samples = 0;
    for (const id of ["keeper_set", "keeper_ready_low"] as const) {
      for (const frame of frames()) {
        samples += 1;
        if (groundedOnRig(rig, id, frame, undefined, KEEPER_PROBES).lift > 0) {
          lifted += 1;
        }
      }
    }
    expect(samples, "72 frames each of two stances").toBe(144);
    expect(lifted, "a crouching keeper never needs the pitch to move for them").toBe(0);
  });

  // THE SAME PROPERTY FOR THE TWO CROUCHES THAT ALSO LEAN, which the block
  // above deliberately cannot cover: `contain` and `fatigue` carry a root tilt,
  // and a tilt DOES need a few millimetres of lift at a standstill -- 4 mm and
  // 2 mm, pinned by `RESIDUAL_MM` below, and not this fix's doing.
  //
  // A MOVING one needs none, and that is the claim worth having, because a
  // containing defender is a moving player and `contain` is the commonest pose
  // in the table. If the crouch ever started costing them a per-frame
  // correction, it would arrive as a wobble at stride frequency on the pose
  // that shows up most.
  it("costs a moving contain or fatigue no per-frame lift either", () => {
    const rig = groundedRig();
    let lifted = 0;
    let samples = 0;
    for (const id of ["contain", "fatigue"] as const) {
      for (const frame of frames()) {
        if (frame.speed === 0) {
          continue;
        }
        samples += 1;
        if (groundedOnRig(rig, id, frame).lift > 0) {
          lifted += 1;
        }
      }
    }
    expect(samples, "48 moving frames each of two poses").toBe(96);
    expect(lifted, "a moving lean-and-crouch never needs the pitch to move either").toBe(0);
  });

  // PARTIAL DEPTH, ON THE REAL MESH. Everything else in this file -- including
  // the ~7000-assertion sweep -- samples a character on their FIRST observed
  // frame, because every helper here uses a fresh player id and the animator
  // commits to its crouch outright on that frame. So every one of those
  // measurements is taken at FULL depth, and the ease-in is measured nowhere on
  // real geometry.
  //
  // Mid-transition is precisely where a weight-blended implementation sinks the
  // feet: a fold of `w * c` raises the foot by `L * (1 - cos(w * c))`, which is
  // QUADRATIC in `w`, while `w * drop` lowers it linearly, so any partial weight
  // under such a scheme puts the soles through the turf and the deficit peaks
  // near the middle of the ramp. `clips.compose` takes no weight and
  // `crouch.poseFor` re-derives the angle from the ramped DEPTH for exactly this
  // reason -- which is an argument until something measures it.
  //
  // Driven through one persistent player id at 60 fps, which is the only way to
  // reach a partial depth at all, and measured at a STANDSTILL: a moving figure
  // floats tens of millimetres clear (pinned at the top of this file) and would
  // absorb the failure this is looking for.
  // MEASURED AGAINST THE SAME INSTANT, NOT AGAINST THE REST POSE, and the first
  // version of this test got that wrong in a way worth recording: the idle clip
  // writes its own weight shift to `move.root`, so once `now` advances the
  // standing figure is no longer at rest height either. Comparing a crouch at
  // t = 0.33 s against a rest pose at t = 0 folds half a millimetre of idle bob
  // into a measurement whose whole subject is a tenth of one. The baseline has
  // to be the same character, at the same instant, not crouching.
  it("keeps the soles on the turf at every depth the ease-in passes through", () => {
    const rig = skeleton.newRig(RIG);
    const tallRig = skeleton.newRig(RIG);
    const base = rest();
    const authored = actionPose.attitudeDrop({ pose: { id: "keeper_ready_low" } });
    const player = "gc_ramp";
    // One frame of plain locomotion first, so the crouch has somewhere to ease
    // FROM. Without it the animator's first-frame commit skips the ramp
    // entirely and this measures the same full depth as everything else.
    animator.poseFor(player, { speed: 0, gait: 0 }, {}, 0);

    let partialSamples = 0;
    let deepest = 0;
    for (let i = 1; i <= 20; i += 1) {
      const now = i / 60;
      const pose = animator.poseFor(player, { speed: 0, gait: 0 }, { pose: { id: "keeper_ready_low" } }, now);
      const tall = animator.poseFor(`gc_tall_${String(i)}`, { speed: 0, gait: 0 }, {}, now);
      const depth = (tall.move["root"]?.[1] ?? 0) - (pose.move["root"]?.[1] ?? 0);
      skeleton.apply(rig, pose);
      skeleton.apply(tallRig, tall);
      const sole = lowestRendered(rig).y;
      const tallSole = lowestRendered(tallRig).y;
      const at = `${(1000 * depth).toFixed(1)} mm of ${(1000 * authored).toFixed(1)} mm`;
      expect(sole, `at ${at}, a sole is below where standing puts it`).toBeGreaterThanOrEqual(tallSole);
      expect(sole - tallSole, `at ${at}, a sole has lifted off`).toBeLessThan(0.2 * MM);
      // And in absolute terms, against the pitch itself.
      expect(sole, `at ${at}, a sole is through the turf`).toBeGreaterThanOrEqual(base.sole);
      // A sample genuinely in the middle of the ramp, where the quadratic
      // deficit a weight blend would produce is largest.
      if (depth > 0.2 * authored && depth < 0.8 * authored) {
        partialSamples += 1;
      }
      deepest = Math.max(deepest, depth);
    }
    // NON-VACUOUS IN BOTH DIRECTIONS: the ramp really passed through the middle
    // rather than snapping, and it really did arrive.
    expect(partialSamples, "the ease-in must actually be sampled part-way down").toBeGreaterThan(2);
    expect(deepest, "and must reach the authored depth by the end of it").toBeCloseTo(authored, 9);
  });

  // THE ONE PLACE IT DOES COST SOMETHING, disclosed with its size rather than
  // left to be found. A crouch lowers the WHOLE body, including anything a
  // player is carrying, and an outfielder's held weapon swings low at one phase
  // of a walk. Settling at a walk therefore dips the weapon under the turf and
  // #446's lift raises it -- a lift that varies across the stride, which is the
  // wobble shape the test above rules out for the body.
  //
  // It is bounded and it is small: 13 mm at its worst, on a 1.5676 m figure
  // drawn ~26 px tall at match camera, which is 0.22 px. The BODY is nowhere
  // near the plane at that phase, so nothing anatomical is being corrected.
  //
  // OVER EVERY LOADOUT, NOT THE ONE THAT HAPPENS TO DIP. Measured, only
  // `loadout_tournament_sword` reaches the turf at all -- `loadout_vector_blade`
  // is a nearly identical length (0.82 m against 0.84 m) and never touches it.
  // So a single hardcoded id would pin the ceiling against the one item that
  // exercises it and hold by luck for the rest: a longer polearm authored
  // tomorrow could sail past 13 mm with nothing to catch it. The loop is over
  // `LOADOUT_EQUIPMENT` itself, so a new loadout is under the ceiling by
  // construction rather than by somebody remembering. That matters more since
  // #460 made loadouts per-player.
  it("keeps every loadout's held item within a fifth of a pixel of the turf while settling", () => {
    const ids = Object.keys(presentationContent.LOADOUT_EQUIPMENT);
    expect(ids.length, "every authored loadout, or this sweep means nothing").toBeGreaterThanOrEqual(6);

    let deepestDip = 0;
    let dippingLoadouts = 0;
    for (const loadoutId of ids) {
      const figure = body.accumulate(RIG, THEME, FIGURE, presentationContent.loadoutFor(loadoutId))[0];
      if (figure === undefined) {
        throw new Error(`ground_contact.spec.ts: ${loadoutId} must build a figure`);
      }
      const probes = ground.probesFrom(RIG, figure.verts, BONE_ORDER);
      const rig = skeleton.raised(skeleton.newRig(RIG), probes.restLift);
      let maxLift = 0;
      let bodyLowest = Infinity;
      for (const frame of frames()) {
        maxLift = Math.max(maxLift, groundedOnRig(rig, "settle", frame, undefined, probes).lift);
        bodyLowest = Math.min(bodyLowest, lowestRendered(rig, PROP, figure.verts).y);
      }
      expect(
        maxLift,
        `${loadoutId}: a fifth of a pixel at match camera, not a boot's depth`,
      ).toBeLessThan(15 * MM);
      expect(
        bodyLowest,
        `${loadoutId}: the player's own body is never what is being lifted`,
      ).toBeGreaterThan(-TOLERANCE);
      if (maxLift > 0) {
        dippingLoadouts += 1;
      }
      deepestDip = Math.max(deepestDip, maxLift);
    }

    // NON-VACUOUS, and this is what a ceiling on its own cannot give: at least
    // one loadout has to actually reach the turf, or every bound above passes
    // on a sweep that is measuring nothing. The sword is that one today.
    expect(dippingLoadouts, "at least one authored loadout really does dip").toBeGreaterThan(0);
    expect(deepestDip, "and by millimetres, so the ceiling is a bound and not a formality").toBeGreaterThan(5 * MM);
  });
});

// ---------------------------------------------------------------------------
// ROOT ROTATIONS, GROUNDED (#446)
// ---------------------------------------------------------------------------
//
// #439 grounded root TRANSLATIONS and disclosed, in this file, that root
// ROTATIONS still swung limbs under the turf -- 4 mm on `fatigue`, 361 mm on
// `keeper_tip`, which is `fixed: 1` and therefore does it on every occurrence.
// The block that measured that gap lives on below, inverted: the same figures
// are now what the LIFT has to be worth, and every one of them is asserted
// twice.
//
//   * ON THE UNGROUNDED PATH (`poseOnRig`), the residual is still exactly
//     there. It has to be: #446 puts reducing the dive angles explicitly out
//     of scope, so a pose whose rotation stopped swinging its limbs down
//     would mean someone re-tuned the constants instead of supplying the
//     missing vertical. These keep the old floors verbatim.
//   * ON THE REAL PATH (`groundedOnRig`), nothing is below the plane -- and
//     nothing is meaningfully ABOVE it either, which is the half a floor
//     alone would not catch. A lift that cleared the turf by over-correcting
//     would be a keeper hovering, and that is the failure this fix is most
//     likely to have.
describe("ground contact: root rotations, grounded (#446)", () => {
  // The ungrounded figures are measured on the rig AS AUTHORED, so they stay
  // the numbers #446 quotes; the grounded ones on the rig the product poses.
  const flat = flatRig();
  const rig = groundedRig();
  const standing = { speed: 0, gait: 0, now: 0 };

  // The per-frame lift is what is left AFTER the rig's constant rest
  // correction, so the two together are the whole depth. Written out rather
  // than folded away because it is the arithmetic a reader will want to check.
  const perFrameLiftFor = (ungrounded: number): number => -(ungrounded + PROBES.restLift);

  // Pose -> the depth its ROTATION alone leaves when it is NOT grounded, in
  // millimetres, once the drop is floored. Each is a FLOOR on the ungrounded
  // path: "the rotation still reaches at least this far down, so the lift is
  // still doing this much work".
  const RESIDUAL_MM: Readonly<Record<string, number>> = {
    keeper_get_up: 50,
    run_telegraph: 24,
    kick_follow: 13,
    combat_stagger: 11,
    contain: 8,
    fatigue: 4,
  };

  for (const [id, mm] of Object.entries(RESIDUAL_MM)) {
    it(`${id}: a ${String(mm)} mm root tilt is lifted onto the plane, not clamped away`, () => {
      poseOnRig(flat, id, standing);
      const ungrounded = lowestRendered(flat).y;
      expect(ungrounded, `${id}'s rotation must not be deepening`).toBeGreaterThan(-mm * MM);
      expect(ungrounded, `${id}'s tilt is still authored, so the residual is still there`).toBeLessThan(0);

      const { lift } = groundedOnRig(rig, id, standing);
      const lowest = lowestRendered(rig).y;
      expect(lowest, `${id} must not stand in a hole`).toBeGreaterThan(-TOLERANCE);
      expect(lowest, `${id} must not hover either -- the lift is exact, not generous`).toBeLessThan(TOLERANCE);
      expect(lift, `${id}'s lift is the depth the rig's own rest correction does not cover`).toBeCloseTo(
        perFrameLiftFor(ungrounded),
        9,
      );
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
  // CONSTANT, and therefore neither is the lift. `keeper_dive` spans 57 mm to
  // 412 mm across the amounts a real dive passes through, and 111 mm -- the
  // figure #444 first disclosed -- is the halfway point, not the ceiling.
  //
  // The ceiling is not hypothetical either. `gc-render`'s frame builder pushes
  // `eased(dive_timer, DIVE_EASE)` = `min(timer / 0.3, 1)`, and
  // `KEEPER_DIVE_DURATION` is 0.32 s, so every dive STARTS at a saturated 1.0
  // and eases down. The deepest row below is the opening frame of every save
  // in the game, not a corner case.
  //
  // BODY AND PROPS STAY SEPARATE COLUMNS, and #447 has now moved one of them,
  // which is the change this table records.
  //
  // BEFORE: every player rendered `themes.LIST[0]`'s sword and shield, so a
  // 72-degree root roll swung the whole arm chain and took `socket_shield.L`
  // with it -- more than twice as deep as any part of the keeper (412 mm
  // against 192 on a full `keeper_dive`, 502 against 279, 595 against 362).
  // The shield BOUND the lift, so the keeper's own body floated clear of the
  // turf by the difference.
  //
  // AFTER: a keeper's mesh has no props in it at all, so the two columns are
  // the same measurement over the same figure and the `props` column equals
  // the `body` one throughout. The `body` figures are UNCHANGED -- #447 moved
  // no bone, retuned no angle and touched no clip; it removed geometry that
  // was never the keeper's. The prediction this file recorded before the fix
  // ("only the magnitude moves, and the pins that would change are the
  // `props` column of SAVE_RESIDUALS, which becomes equal to the `body`
  // column") is what landed, and this is it landed.
  //
  // Both columns are kept rather than collapsed to one, so the shape of the
  // table still says which quantity is which -- and so the day a keeper is
  // authored something to carry, there is a column to put it in.

  /**
   * id, the `dive` passed in, and the ungrounded depths of A KEEPER: body-only,
   * then including whatever props the keeper renders -- which since #447 is
   * nothing, so the two agree.
   */
  const SAVE_RESIDUALS: readonly { id: string; dive: number; body: number; props: number }[] = [
    // Amount-driven: no `fixed`, no `floor`, so these scale from nothing.
    { id: "keeper_dive", dive: 0.25, body: 57, props: 57 },
    { id: "keeper_dive", dive: 0.5, body: 111, props: 111 },
    { id: "keeper_dive", dive: 1, body: 192, props: 192 },
    { id: "keeper_spread", dive: 1, body: 88, props: 88 },
    { id: "keeper_central", dive: 1, body: 141, props: 141 },
    // `floor: 0.82`, so this one is already 82% deep with no amount at all.
    { id: "keeper_stretch", dive: 0, body: 171, props: 171 },
    { id: "keeper_stretch", dive: 1, body: 279, props: 279 },
    // `fixed: 1`, so the amount is ignored entirely and it is always this deep.
    { id: "keeper_tip", dive: 0, body: 362, props: 362 },
  ];

  for (const row of SAVE_RESIDUALS) {
    it(`${row.id} at dive ${String(row.dive)}: ${String(row.body)} mm of body and ${String(row.props)} mm with props, lifted clear`, () => {
      poseOnRig(flat, row.id, standing, { dive: row.dive });
      const ungroundedBody = lowestRendered(flat, PROP, KEEPER_MESH.verts).y;
      const ungroundedAll = lowestRendered(flat, undefined, KEEPER_MESH.verts).y;
      expect(ungroundedBody, `${row.id}'s body residual must not be deepening`).toBeGreaterThan(-row.body * MM);
      expect(ungroundedAll, `${row.id}'s residual including props must not be deepening`).toBeGreaterThan(-row.props * MM);
      expect(ungroundedAll, `${row.id} at dive ${String(row.dive)} is a rotation, so it does not vanish`).toBeLessThan(-2 * MM);
      // THE #447 CLAIM ITSELF, per row rather than in prose: for a keeper the
      // two columns are one measurement, because there is no prop to be the
      // deeper of the two. Non-vacuous by the `-2 mm` floor just above -- a
      // pose that stopped happening would fail there, not pass here.
      expect(ungroundedAll, `${row.id}: a keeper renders no props, so body and props are the same depth`).toBe(
        ungroundedBody,
      );

      const { lift } = groundedOnRig(rig, row.id, standing, { dive: row.dive }, KEEPER_PROBES);
      expect(lowestRendered(rig, undefined, KEEPER_MESH.verts).y, `${row.id} must not reach through the pitch`).toBeGreaterThan(-TOLERANCE);
      expect(lowestRendered(rig, undefined, KEEPER_MESH.verts).y, `${row.id} must not hover above it`).toBeLessThan(TOLERANCE);
      expect(lift, `${row.id}'s lift is exactly what it would otherwise sink`).toBeCloseTo(
        perFrameLiftFor(ungroundedAll),
        9,
      );
      // The lift is genuinely large: it is the missing vertical of a dive, not
      // a rounding correction.
      expect(lift, `${row.id}'s lift is worth tens of millimetres at least`).toBeGreaterThan(50 * MM);
    });
  }

  // THE BODY SAVE ROTATES ABOUT THE OTHER AXIS (#451), so the lift has to
  // reach it there too.
  //
  // Every row above dives to a SIDE, which is a roll about Z. A save with no
  // lateral component pitches FORWARD instead -- about X, at the same angle --
  // and #446's lift is measured off the posed rig rather than derived from the
  // overlay, so it covers a new axis with no new code. That is a claim about
  // `poseAndGround`, and this is where it gets checked rather than assumed.
  //
  // WHAT THE MEASUREMENT SAYS, in both directions. THE BODY GOES LESS DEEP
  // FORWARD THAN SIDEWAYS at every family -- 74 mm against 88 for a spread,
  // 118 against 141 for a central, 153 against 192 for a dive -- because a
  // forward pitch swings the figure over a foot that is already under it,
  // while a roll throws it off the side of both.
  //
  // Every millimetre figure in this block is rounded UP to the floor pinned
  // beside it, which is `SAVE_RESIDUALS`'s own convention rather than a new
  // one: prose and assertion are then the same number, and a reader comparing
  // the two never has to work out which way each was rounded. The lateral
  // spread and central are 87.3 and 140.3 measured, pinned and quoted at 88
  // and 141.
  //
  // THE PROPS USED TO GO DEEPER STILL, and by more than the body gained: 674
  // mm at `keeper_dive` against the lateral 412, 428 at a central against
  // 141, 166 at a spread against 88. That was the sword and shield the
  // renderer handed EVERY player including keepers, swung down and forward by
  // the same pitch. It BOUND the lift, so a body save floated higher than the
  // equivalent lateral one -- and this block recorded that rather than tuning
  // around it, on the grounds that shrinking the authored pitch to flatter a
  // prop the keeper should not have would be fixing #447 in the wrong file.
  //
  // #447 fixed it in the right file. A keeper's mesh has no props, so the
  // `props` column here is the `body` column too, and the hover it caused is
  // gone with it. Every `body` figure is unchanged.
  const BODY_SAVE_RESIDUALS: readonly { id: string; dive: number; body: number; props: number }[] = [
    { id: "keeper_spread", dive: 1, body: 74, props: 74 },
    { id: "keeper_central", dive: 1, body: 118, props: 118 },
    { id: "keeper_dive", dive: 1, body: 153, props: 153 },
    // The recovery from one, on the same axis and at its own shallower tilt.
    { id: "keeper_get_up", dive: 0, body: 42, props: 42 },
  ];

  for (const row of BODY_SAVE_RESIDUALS) {
    it(`${row.id} taken at the body: a forward pitch is lifted clear too`, () => {
      const straight = { dive: row.dive, dive_dir: { x: 0, y: 0 } };
      poseOnRig(flat, row.id, standing, straight);
      const ungroundedBody = lowestRendered(flat, PROP, KEEPER_MESH.verts).y;
      const ungroundedAll = lowestRendered(flat, undefined, KEEPER_MESH.verts).y;
      expect(ungroundedBody, `${row.id}'s body residual must not be deepening`).toBeGreaterThan(-row.body * MM);
      expect(ungroundedAll, `${row.id}'s residual including props must not be deepening`).toBeGreaterThan(
        -row.props * MM,
      );
      // Non-vacuous: the pitch is a real rotation, so it really does reach
      // below the plane. Without this the floors above would pass on a
      // standing rig, which is the mistake the lateral table records.
      expect(ungroundedAll, `${row.id} at the body is a rotation, so it does not vanish`).toBeLessThan(-2 * MM);
      expect(ungroundedAll, `${row.id}: a keeper renders no props, so body and props are the same depth`).toBe(
        ungroundedBody,
      );

      const { lift } = groundedOnRig(rig, row.id, standing, straight, KEEPER_PROBES);
      expect(lowestRendered(rig, undefined, KEEPER_MESH.verts).y, `${row.id} must not reach through the pitch`).toBeGreaterThan(-TOLERANCE);
      expect(lowestRendered(rig, undefined, KEEPER_MESH.verts).y, `${row.id} must not hover above it`).toBeLessThan(TOLERANCE);
      expect(lift, `${row.id}'s lift is exactly what it would otherwise sink`).toBeCloseTo(
        perFrameLiftFor(ungroundedAll),
        9,
      );
    });
  }

  it("keeps a body save shallower on the body than the dive it stands in for", () => {
    // The comparison the table above states in prose, asserted so it cannot
    // quietly invert: whatever the props do, the KEEPER is never driven deeper
    // by taking a save at the body than by throwing themselves at it.
    for (const id of ["keeper_spread", "keeper_central", "keeper_dive"]) {
      poseOnRig(flat, id, standing, { dive: 1, dive_dir: { x: 0, y: 0 } });
      const straight = lowestRendered(flat, PROP).y;
      // NON-VACUOUS, and this floor is the whole reason the comparison means
      // anything. A "shallower" test passes MOST easily when there is no pose
      // at all: skip the overlay -- which is exactly what this file's own
      // subject did before #451 -- and the rig sits at its ~-1.2 mm standing
      // baseline, shallower than any real dive depth, and the assertion below
      // sails through while proving nothing. The floor is the same `-2 mm`
      // guard the row loop above uses, and it is what makes this a statement
      // about the body save rather than about its absence.
      expect(straight, `${id} at the body must be a real rotation, not a standing rig`).toBeLessThan(-2 * MM);
      poseOnRig(flat, id, standing, { dive: 1 });
      const lateral = lowestRendered(flat, PROP).y;
      expect(straight, `${id} at the body must not sink past the lateral dive`).toBeGreaterThan(lateral);
    }
  });

  // The guard that makes the table above non-vacuous, stated as a property
  // rather than left to whoever writes the next row: if a save's amount ever
  // stops reaching the pose, these measurements go back to describing a
  // standing rig, whose lift is zero and whose depth is -1.2 mm -- and every
  // "must not be deepening" floor passes trivially. `keeper_tip` is `fixed`
  // and `keeper_stretch` is floored, so they are deep with no amount at all;
  // the other three must be at the standing baseline without one, and far
  // past it with one. Asserted on the LIFT as well as on the depth, because
  // the lift is the new quantity and a vacuous pin would leave it at zero.
  it("proves each save's lift is driven by its amount, not by the rig standing still", () => {
    // Measured on the KEEPER's own figure (#447), matching the two tables
    // above -- these are keeper poses, and the whole point of the fix is that
    // a keeper's depth is now their own.
    const keeperVerts = KEEPER_MESH.verts;
    const baseline = lowestRendered(flatRig(), undefined, keeperVerts).y;
    for (const id of ["keeper_dive", "keeper_spread", "keeper_central"]) {
      poseOnRig(flat, id, standing, { dive: 0 });
      expect(lowestRendered(flat, undefined, keeperVerts).y, `${id} with no dive amount is just a standing character`).toBeCloseTo(baseline, 6);
      // Exactly zero, not "small": a standing character is what the rig's own
      // rest correction already covers, so the per-frame lift has nothing left
      // to do and the second skeleton evaluation is not paid.
      expect(groundedOnRig(rig, id, standing, { dive: 0 }, KEEPER_PROBES).lift, `${id} with no dive amount needs no lift`).toBe(0);

      poseOnRig(flat, id, standing, { dive: 1 });
      expect(lowestRendered(flat, undefined, keeperVerts).y, `${id} at full dive must be far past standing`).toBeLessThan(baseline - 50 * MM);
      expect(groundedOnRig(rig, id, standing, { dive: 1 }, KEEPER_PROBES).lift, `${id} at full dive must be lifted far`).toBeGreaterThan(
        50 * MM,
      );
    }
    for (const id of ["keeper_stretch", "keeper_tip"]) {
      poseOnRig(flat, id, standing, { dive: 0 });
      expect(lowestRendered(flat, undefined, keeperVerts).y, `${id} is fixed or floored, so it is deep with no amount`).toBeLessThan(baseline - 50 * MM);
      expect(groundedOnRig(rig, id, standing, { dive: 0 }, KEEPER_PROBES).lift, `${id} is lifted with no amount too`).toBeGreaterThan(
        50 * MM,
      );
    }
  });

  // The property, over the whole reachable pose set rather than a hand-picked
  // table: at a standstill, at a walk and at a run, at every phase of the
  // stride, at four dive amounts. This is the assertion that makes "the next
  // authored tilt cannot reintroduce it" true rather than intended -- a pose
  // id added to `SAVES`, `TIPS` or `ATTITUDES` tomorrow is covered by
  // `pose_table.POSE_ACTIONS`' own key set, not by remembering to add a row.
  it("puts no rendered vertex through the pitch, for any pose, phase or amount", () => {
    const rigAll = groundedRig();
    const ids = [undefined, ...Object.keys(poseTable.POSE_ACTIONS)];
    for (const id of ids) {
      for (const dive of [0, 0.5, 1]) {
        for (const frame of frames()) {
          groundedOnRig(rigAll, id, frame, {
            dive,
            aerial: dive,
            aerial_style: "bicycle",
            aerial_jump: dive,
          });
          // `ground.lowestPoint` rather than this file's brute-force
          // `lowestRendered`: the two are pinned equal over exactly this pose
          // set in `ground.spec.ts`, and the sweep below is ~7000 frames.
          expect(
            ground.lowestPoint(rigAll, PROBES),
            `${id ?? "(no pose)"} at dive ${String(dive)}, speed ${String(frame.speed)}, phase ${frame.gait.toFixed(2)}`,
          ).toBeGreaterThan(-TOLERANCE);
        }
      }
    }
  });
});

describe("ground contact: what the lift is, exactly", () => {
  const standing = { speed: 0, gait: 0, now: 0 };

  // THE #436 TRAP, PINNED. `skeleton.apply` multiplies `move` by the rig's
  // motion_scale (0.88), so a lift measured in world metres has to be DIVIDED
  // by it on the way into `move.root`. Skipping that division clears 88% of
  // the penetration and leaves 12% -- 43 mm at a full `keeper_dive`, which is
  // deeper than five of the six poses in the residual table above and would
  // pass every "no longer 411 mm" assertion anyone would think to write.
  it("writes the lift in `move` units, not world metres", () => {
    const rig = groundedRig();
    const before = animator.poseFor("gc_scale", { speed: 0, gait: 0 }, { pose: { id: "keeper_tip" }, dive_dir: { x: 1, y: 0 }, facing: { x: 0, y: 1 }, dive: 1 }, 0);
    const beforeY = before.move["root"]?.[1] ?? 0;
    const lift = ground.poseAndGround(rig, before, PROBES);
    const afterY = before.move["root"]?.[1] ?? 0;
    expect(lift, "keeper_tip is the deepest pose in the game").toBeGreaterThan(500 * MM);
    expect((afterY - beforeY) * RIG.motion_scale, "the authored delta times motion_scale IS the world lift").toBeCloseTo(
      lift,
      12,
    );
    // Stated the other way too, so the direction of the division cannot be
    // read backwards: the number written down is LARGER than the world lift.
    expect(afterY - beforeY, "0.88 metres of travel per metre authored").toBeGreaterThan(lift);
  });

  // A rigid translation of a rig whose root has no parent cannot change which
  // vertex is lowest, so one correction is exact and a second finds nothing.
  // If this ever fails, the lift has stopped being a pure world translation
  // and the whole "no iteration needed" argument in `ground.ts` is void.
  it("needs exactly one correction, never two", () => {
    const rig = groundedRig();
    const { pose, lift } = groundedOnRig(rig, "keeper_dive", standing, { dive: 1 });
    expect(lift).toBeGreaterThan(50 * MM);
    expect(ground.poseAndGround(rig, pose, PROBES), "an already-grounded rig is already grounded").toBe(0);
  });

  // THE RIG'S OWN MILLIMETRE, AND WHERE IT IS PAID.
  //
  // The first test in this file measures a STANDING character's lowest point at
  // -1.2 mm: the boot's outer sole corner, a millimetre into the turf because
  // the thigh carries a 2 degree rest roll. #444 named that number and spent it
  // as "no clearance", which is what let it floor a drop at zero. An exact lift
  // does not get to be selective about which millimetre it is exact about, so
  // every character now plants ON the plane instead of a millimetre under it --
  // 0.02 px at match camera (derived once in `ground.ts`, alongside the figure
  // height and the ~26 px it divides into), invisible at any camera scale this
  // game uses, and in the direction of correct.
  //
  // WHAT THIS PAIR IS REALLY ABOUT is that it is a CONSTANT and is paid ONCE.
  // The idle clip never reaches below the identity rest pose -- measured over
  // 24 phases, the two agree to every digit -- so there is nothing per-frame
  // about the correction, and `probesFrom` measures it while `skeleton.raised`
  // applies it at build time. Left inside `poseAndGround` it would be
  // re-measured sixty times a second and every idle character on the pitch
  // would pay a second skeleton evaluation to be raised 0.02 px.
  // That regression is invisible in every pixel, so the second test asserts a
  // COUNT rather than a depth.
  //
  // The alternative was an epsilon -- do not bother lifting less than the
  // rig's own baseline -- and it is still rejected: a tolerance says "ignore
  // penetration under 1.2 mm" and hides a real 1.2 mm defect the day one
  // appears. Moving the rig hides nothing.
  it("carries the rig's own 1.2 mm as a rest correction, not as a per-frame lift", () => {
    const authored = flatRig();
    const authoredY = lowestRendered(authored).y;
    expect(authoredY, "the authored rig plants a millimetre under").toBeLessThan(0);
    expect(PROBES.restLift, "and `restLift` is exactly that, measured through the same scan").toBeCloseTo(
      -authoredY,
      12,
    );
    expect(PROBES.restLift, "which is the millimetre the first test in this file measures").toBeLessThan(2 * MM);
    expect(PROBES.restLift, "and it is real, not zero").toBeGreaterThan(MM);

    // Raised, the rest pose is ON the plane -- not below it. The residue is
    // ~1e-17 m of double arithmetic and its SIGN is what matters: negative
    // would mean the early-out stops firing and every idle character pays a
    // second evaluation again.
    const raised = groundedRig();
    expect(lowestRendered(raised).y, "and raised by it, the rig plants on the plane").toBeGreaterThanOrEqual(0);
    expect(lowestRendered(raised).y, "to within double precision").toBeLessThan(TOLERANCE);
  });

  // The cost property, as a deterministic count rather than a wall clock: a
  // character who is not penetrating must cost ONE skeleton evaluation, not
  // two. `poseAndGround`'s second `skeleton.apply` happens if and only if it
  // returns a non-zero lift, so the lift is the count here -- and
  // `ground.spec.ts` pins that equivalence against a genuinely counted
  // `skeleton.apply`, in both directions.
  it("costs an idle, walking or running character no second skeleton evaluation", () => {
    const rig = groundedRig();
    const authored = flatRig();
    let lifted = 0;
    for (const frame of frames()) {
      poseOnRig(authored, undefined, frame);
      const ungroundedY = lowestRendered(authored).y;
      const { lift } = groundedOnRig(rig, undefined, frame);
      expect(lift, "grounding only ever raises").toBeGreaterThanOrEqual(0);
      expect(lowestRendered(rig).y, "and it raises to the plane, never past it").toBeCloseTo(
        Math.max(ungroundedY + PROBES.restLift, 0),
        9,
      );
      if (lift > 0) {
        lifted += 1;
      }
    }
    // 72 frames: 24 phases each of idle, walk and run. Before the rest
    // correction moved to build time, 24 of them (every idle phase) took the
    // correction branch; on an unraised rig they do again, which is what makes
    // this able to go red.
    expect(lifted, "no phase of idle, a walk or a run needs a per-frame lift at all").toBe(0);
  });

  // A clip may drop the root, and SWING does. Its own limbs pay for it (pinned
  // above), so grounding must find nothing to do beyond the same sole -- if it
  // ever lifts a swing by more than that, the clip has broken, not the fix.
  it("leaves a clip's own root motion alone", () => {
    const rig = groundedRig();
    let maxLift = 0;
    for (let i = 0; i <= 40; i += 1) {
      const { lift } = groundedOnRig(rig, "combat_active", { speed: 0, gait: 0, now: (i / 40) * clips.SWING.duration });
      maxLift = Math.max(maxLift, lift);
    }
    // Exactly zero: SWING's 32 mm root drop is paid for by its own thigh and
    // shin keys, so the deepest it ever reaches is the rest pose's own sole --
    // which the rig already carries.
    expect(maxLift, "a swing's lunge stays on the pitch on its own merits").toBe(0);
  });
});

// ---------------------------------------------------------------------------
// THE FLOOR AND THE LIFT (#444 x #446)
// ---------------------------------------------------------------------------
//
// These two corrections write to the SAME channel -- `move.root.y` -- from
// opposite ends, so the question of whether one makes the other redundant has
// to be answered with a measurement rather than an opinion.
//
// It does not, and the reason is the gait. #444's floor removes an authored
// drop BEFORE the pose is evaluated, which is a constant correction: a settle
// is a settle at every phase of the stride. The lift is necessarily read off
// the posed rig every frame, and the posed rig's own height varies by more
// than a crouch is worth -- the run floats the whole figure between +21 mm and
// +137 mm (pinned at the top of this file). Delete the floor and a settling
// player's 69 mm drop is absorbed at one phase of the stride and lifted back at
// another, which is a vertical wobble at stride frequency applied to poses
// whose entire job is to read as a held posture. That is the exact failure
// `action_pose.ts`'s GROUND CLEARANCE note rejects a per-frame budget for, and
// the test below drives it rather than describing it.
//
// #445 SHARPENED THIS RATHER THAN RETIRING IT. `rig3d/crouch.ts` now delivers
// the same authored drop as limb work, and the overlay's copy of it is still
// floored -- so what the floor prevents today is the drop arriving TWICE, once
// paid for by a knee bend and once not. The unfloored figure the test measures
// is exactly that second, unpaid copy.
describe("ground contact: #444's floor and #446's lift compose", () => {
  it("clears the plane for every drop pose, at every phase and speed", () => {
    const rig = groundedRig();
    for (const id of DROP_POSES) {
      for (const frame of frames()) {
        groundedOnRig(rig, id, frame);
        expect(
          lowestRendered(rig).y,
          `${id} at speed ${String(frame.speed)} phase ${frame.gait.toFixed(2)}`,
        ).toBeGreaterThan(-TOLERANCE);
      }
    }
  });

  it("is why the floor is still load-bearing: without it the lift tracks the stride", () => {
    // Measured on a KEEPER's figure, which since #447 carries nothing: this
    // test is about what the RIG does under a doubled drop, and an outfielder's
    // held weapon has its own, separately pinned, 13 mm dip at one phase of a
    // settling walk (see the #445 block above). Letting that in here would put
    // a prop's depth inside a measurement of the body's.
    const rig = skeleton.raised(skeleton.newRig(RIG), KEEPER_PROBES.restLift);
    // `settle`'s crouch as ATTITUDES authors it, which `apply` floors to zero
    // on the overlay -- `rig3d/crouch.ts` is what delivers it, with the limbs
    // paying for it first.
    const authoredDrop = actionPose.attitudeFor({ pose: { id: "settle" } })?.move.root?.[1] ?? 0;
    expect(authoredDrop, "settle authors a real drop for the floor to remove").toBeLessThan(-50 * MM);

    let flooredMax = 0;
    let unflooredMin = Infinity;
    let unflooredMax = 0;
    for (const frame of frames()) {
      flooredMax = Math.max(flooredMax, groundedOnRig(rig, "settle", frame, undefined, KEEPER_PROBES).lift);

      // The same frame with the floor undone: the authored drop added back on
      // top of the crouch that has already spent it, which is the second,
      // unpaid copy the floor exists to refuse.
      const pose = animator.poseFor(`gc_unfloored_${String(frame.speed)}_${String(frame.gait)}`, { speed: frame.speed, gait: frame.gait }, { pose: { id: "settle" } }, frame.now);
      const root = pose.move["root"] ?? [0, 0, 0];
      pose.move["root"] = [root[0] ?? 0, (root[1] ?? 0) + authoredDrop, root[2] ?? 0];
      const lift = ground.poseAndGround(rig, pose, KEEPER_PROBES);
      unflooredMin = Math.min(unflooredMin, lift);
      unflooredMax = Math.max(unflooredMax, lift);
    }

    expect(flooredMax, "with the floor, a settling character never needs a lift at all").toBeLessThan(TOLERANCE);
    expect(
      unflooredMax - unflooredMin,
      "without it, the lift swings by tens of millimetres across the gait -- a wobble, not a posture",
    ).toBeGreaterThan(20 * MM);
  });

  // The lift only ever adds. It cannot reintroduce the sink the floor removed,
  // because a rig that is already clear is returned untouched (pinned above)
  // and a rig that is not is moved UP.
  it("never writes a downward root translation of its own", () => {
    const rig = groundedRig();
    let n = 0;
    for (const id of [...DROP_POSES, "keeper_dive", "keeper_tip", "combat_knockback"]) {
      for (const frame of frames()) {
        n += 1;
        const before = animator.poseFor(`gc_dir_${String(n)}`, { speed: frame.speed, gait: frame.gait }, { pose: { id }, dive_dir: { x: 1, y: 0 }, facing: { x: 0, y: 1 }, dive: 1 }, frame.now);
        const beforeY = before.move["root"]?.[1] ?? 0;
        ground.poseAndGround(rig, before, PROBES);
        const afterY = before.move["root"]?.[1] ?? 0;
        expect(afterY, `${id} must only ever be raised`).toBeGreaterThanOrEqual(beforeY);
      }
    }
  });
});

// ---------------------------------------------------------------------------
// EQUIPMENT (#447)
// ---------------------------------------------------------------------------
//
// This block was written BEFORE #447, against a renderer that built ONE
// memoised geometry for the whole game from `themes.LIST[0]` -- Medieval
// Fantasy, which carries a sword and a shield -- and a `pitch.ts` that passed
// it neither `loadout_id` nor `presentation_id`. Every keeper on screen dived
// holding a shield, despite `gc-data` asserting keepers have none. Its job
// then was to make sure #446's fix was correct in BOTH worlds and did not
// quietly depend on the defective one.
//
// #447 has landed, so its job now is to say what actually changed, and to
// keep the one case that must NOT change honest.
describe("ground contact: equipment (#447)", () => {
  const standing = { speed: 0, gait: 0, now: 0 };

  // THE ONE OUTFIELD CASE WHERE A BODY-ONLY MEASUREMENT LIES, AND IT SURVIVES
  // #447 UNCHANGED. Everywhere else in this file, ignoring the props
  // understates the depth. Here it inverts the answer: the body reads a
  // comfortable 43 mm CLEAR of the plane -- this pose lifts 0.45r on purpose
  // -- while the shield on `socket_shield.L`, swung by a 68-degree backward
  // pitch, dips below it. A table keyed on pose id and measured body-only
  // would have gone on passing while that margin eroded to nothing.
  //
  // MEASURED ON A REAL OUTFIELD FIGURE, not on the preview default: this is
  // `SHIELD_MESH`, the mesh a defender carrying `loadout_emberguard_shield`
  // now renders -- brakka, drell and sela_dwin in `gc-data`. #447 took the
  // shield away from keepers and from nobody else, so the case is not a
  // hypothetical about a figure that no longer exists.
  it("combat_knockback: an outfielder's body reads clear while their shield does not", () => {
    const flat = flatRig();
    poseOnRig(flat, "combat_knockback", standing);
    const bodyOnly = lowestRendered(flat, PROP, SHIELD_MESH.verts);
    const withProps = lowestRendered(flat, undefined, SHIELD_MESH.verts);
    expect(bodyOnly.y, "body-only, this pose looks entirely fine").toBeGreaterThan(40 * MM);
    expect(withProps.y, "and it is not: the shield is through the turf").toBeLessThan(0);
    expect(withProps.y, "though only just -- this is a small margin, not a gouge").toBeGreaterThan(-10 * MM);
    expect(withProps.bone, "and it is the shield that does it").toBe("socket_shield.L");

    const rig = groundedRig();
    const { lift } = groundedOnRig(rig, "combat_knockback", standing, undefined, SHIELD_PROBES);
    expect(lift, "the lift is driven by the shield, so it is small").toBeCloseTo(
      -(withProps.y + SHIELD_PROBES.restLift),
      9,
    );
    expect(lowestRendered(rig, undefined, SHIELD_MESH.verts).y, "and nothing is left below the plane").toBeGreaterThan(-TOLERANCE);
  });

  // THE KEEPER RULE AT THE MESH, which is the assertion #447 asks for and the
  // one this file is the right place for: everywhere else it is checked
  // against the part list or the loadout table, and here it is checked
  // against the VERTICES that get grounded, because a keeper who still had a
  // shield in their geometry would still be lifted by it no matter what any
  // table said.
  it("renders a keeper with no equipment vertices at all, and an outfielder with some", () => {
    const keeperSockets = new Set(
      KEEPER_MESH.verts.map((v) => BONE_ORDER[v.bone]).filter((name): name is string => name !== undefined && PROP(name)),
    );
    expect([...keeperSockets], "a keeper's mesh carries nothing on a socket bone").toEqual([]);
    // NON-VACUOUS, and this is the half that matters: the same scan over an
    // outfielder's mesh must find the shield, or the assertion above would
    // pass on a broken scan, a renamed bone convention, or an empty mesh.
    const shieldSockets = new Set(
      SHIELD_MESH.verts.map((v) => BONE_ORDER[v.bone]).filter((name): name is string => name !== undefined && PROP(name)),
    );
    expect([...shieldSockets], "an emberguard defender still has one").toEqual(["socket_shield.L"]);
    expect(KEEPER_MESH.verts.length).toBeLessThan(SHIELD_MESH.verts.length);
  });

  // WHAT #447 DID TO THE FIGURES ABOVE, measured rather than described: it
  // did not invalidate them, it halved the deep ones. Grounding is a
  // measurement over whatever is RENDERED, so once the shield stops being
  // rendered for keepers the same code lifts them by their own body's depth
  // instead of the shield's -- no change to `ground.ts` at all.
  //
  // Both the old and the new figure are asserted here, from the same rig, so
  // the size of the change is a measurement and not a memory: `PROBES` is
  // still the sword-and-shield figure (what a keeper used to render) and
  // `KEEPER_PROBES` is what a keeper renders now.
  it("lifts a keeper by their own hand, where it used to lift them by a shield", () => {
    // Both probe sets measure the same rest pose -- the lowest point of a
    // standing character is a BOOT, not a prop -- so the rig this poses is
    // correctly raised for either, and #447 did not move the rest correction.
    expect(BODY_ONLY_PROBES.restLift, "the rest pose's lowest point is a boot, with or without kit").toBe(
      PROBES.restLift,
    );
    // And the keeper's OWN mesh agrees with the filtered one to the bit,
    // which is what makes `BODY_ONLY_PROBES` -- written before the fix as a
    // projection -- a legitimate stand-in for the real thing throughout this
    // file.
    expect(KEEPER_PROBES.restLift, "building with an empty loadout is the same figure as filtering the props out").toBe(
      BODY_ONLY_PROBES.restLift,
    );

    const rig = groundedRig();
    const asKeeperRendersNow = groundedOnRig(rig, "keeper_dive", standing, { dive: 1 }, KEEPER_PROBES).lift;
    const asKeeperUsedToRender = groundedOnRig(rig, "keeper_dive", standing, { dive: 1 }, PROBES).lift;
    expect(asKeeperUsedToRender, "the old figure was lifted by the shield it should not have had").toBeGreaterThan(400 * MM);
    expect(asKeeperRendersNow, "a keeper's own deepest point is the hand, at roughly half that").toBeGreaterThan(180 * MM);
    expect(asKeeperRendersNow, "roughly half that").toBeLessThan(200 * MM);
    // The visible consequence, stated as a number rather than left for
    // somebody to notice on screen: before #447 a diving keeper's own body
    // cleared the turf by the difference, because the shield was what was
    // touching it. That hover is what this change removes.
    expect(asKeeperUsedToRender - asKeeperRendersNow, "the hover the shield's extra reach used to cause").toBeGreaterThan(
      200 * MM,
    );
  });
});
