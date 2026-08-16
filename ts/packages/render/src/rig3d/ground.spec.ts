// `rig3d/ground.ts`'s own properties.
//
// The GAME-FACING claims -- what a player sees, which poses reach through the
// pitch, what the lift is worth pose by pose -- are `ground_contact.spec.ts`'s,
// and deliberately stay there: AGENTS.md §9's line is that asserting a helper
// helps proves the helper, not the game. What is left for this file is the two
// things that would make that suite quietly wrong rather than red:
//
//   * the branch and bound is a BOUND, so its answer must equal a brute-force
//     minimum over every rendered vertex, always -- and `ground_contact.spec.ts`
//     leans on that equality to sweep ~7000 frames at a cost it could not
//     otherwise afford; and
//   * `probesFrom` must reject a vertex whose bone index it cannot resolve,
//     because a silently dropped bone is a part that can hang through the pitch
//     with every assertion in the suite still green.
//
// Plus one that is neither: HOW MANY TIMES `poseAndGround` EVALUATES THE
// SKELETON. That is a cost, not a picture, so it belongs to the helper rather
// than to the game -- but it is the property the whole rest-pose correction
// exists to protect, and it is invisible in every pixel. Counted here, because
// a count is deterministic and a stopwatch is a flake generator (AGENTS.md
// section 9: a gate must be able to go red, and it must go red for a reason
// and not for a busy CI machine).

import { beforeEach, describe, expect, it, vi } from "vitest";
import { mat4 } from "@gc/core";

// `skeleton.apply`, counted. The factory is hoisted above the imports, so the
// counter has to be hoisted with it; the wrapper delegates to the real module,
// so nothing else in this file behaves differently. Only calls that cross the
// MODULE boundary are counted -- `newRig` and `raised` call their own local
// `apply` and are invisible here, which is exactly right: what is being
// measured is what `poseAndGround` asks for.
const applies = vi.hoisted(() => ({ count: 0 }));
vi.mock("./skeleton.ts", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./skeleton.ts")>();
  return {
    ...actual,
    apply: (rig: Parameters<typeof actual.apply>[0], pose: Parameters<typeof actual.apply>[1]) => {
      applies.count += 1;
      actual.apply(rig, pose);
    },
  };
});
import * as actionPose from "./action_pose.ts";
import * as animator from "./animator.ts";
import * as body from "./body.ts";
import * as ground from "./ground.ts";
import * as poseTable from "./pose_table.ts";
import { RIG_MEDIUM } from "./proportions.ts";
import * as skeleton from "./skeleton.ts";
import * as themes from "./themes.ts";

const RIG = RIG_MEDIUM;
const THEME = themes.LIST[0];
const FIGURE = themes.FIGURES[0];
if (THEME === undefined || FIGURE === undefined) {
  throw new Error("ground.spec.ts: themes.LIST/FIGURES must not be empty");
}
const [MESH] = body.accumulate(RIG, THEME, FIGURE);
const BONE_ORDER = skeleton.bones(RIG).map((b) => b.name);
const PROBES = ground.probesFrom(RIG, MESH.verts, BONE_ORDER);

/** The minimum the pruning has to reproduce: every vertex, no shortcuts. */
function bruteForceLowest(rig: skeleton.Rig): number {
  let lowest = Infinity;
  for (const vertex of MESH.verts) {
    const name = BONE_ORDER[vertex.bone];
    const world = name !== undefined ? rig.world[name] : undefined;
    if (world === undefined) {
      throw new Error(`ground.spec.ts: vertex on unknown bone index ${String(vertex.bone)}`);
    }
    const p = mat4.transformPoint(
      world,
      vertex.position[0],
      vertex.position[1],
      vertex.position[2],
    );
    if (p[1] < lowest) {
      lowest = p[1];
    }
  }
  return lowest;
}

let seq = 0;
function posed(
  rig: skeleton.Rig,
  id: string | undefined,
  extra: Partial<actionPose.ActionPoseOptions>,
  now: number,
  speed: number,
  gait: number,
) {
  seq += 1;
  const opts =
    id === undefined
      ? {}
      : { pose: { id }, dive_dir: { x: 1, y: 0 }, facing: { x: 0, y: 1 }, ...extra };
  const pose = animator.poseFor(`g_${String(seq)}`, { speed, gait }, opts, now);
  skeleton.apply(rig, pose);
  return pose;
}

describe("rig3d/ground: the pruned scan is exact", () => {
  // Over every pose id the table can select, at three dive amounts, three
  // speeds and eight phases. The pruning depends on which bone's bounding
  // sphere happens to reach lowest, so it is only convincing across poses that
  // put different bones there -- a boot, a hand, a shield.
  it("agrees with a brute-force minimum over every rendered vertex", () => {
    const rig = skeleton.newRig(RIG);
    const ids: (string | undefined)[] = [undefined, ...Object.keys(poseTable.POSE_ACTIONS)];
    let checked = 0;
    for (const id of ids) {
      for (const dive of [0, 0.5, 1]) {
        for (const speed of [0, 90, 260]) {
          for (let i = 0; i < 8; i += 1) {
            posed(
              rig,
              id,
              { dive, aerial: dive, aerial_style: "bicycle", aerial_jump: dive },
              i * 0.037,
              speed,
              i / 8,
            );
            expect(
              ground.lowestPoint(rig, PROBES),
              `${id ?? "(no pose)"} dive ${String(dive)} speed ${String(speed)} phase ${String(i)}`,
            ).toBe(bruteForceLowest(rig));
            checked += 1;
          }
        }
      }
    }
    expect(checked, "the sweep must actually have covered the pose table").toBeGreaterThan(500);
  });

  // The pruning's stopping rule reads a per-call scratch buffer. If that
  // buffer were ever left dirty, the SECOND call on the same probes would
  // prune bones it had already visited and could return a value that is too
  // high -- a character that renders through the turf while the scan reports
  // it clear. Two poses in a row, alternating between a shield-lowest and a
  // boot-lowest one, is the shape that catches it.
  it("gives the same answer on repeated calls with the same probes", () => {
    const rig = skeleton.newRig(RIG);
    for (let round = 0; round < 3; round += 1) {
      posed(rig, "keeper_tip", { dive: 1 }, 0, 0, 0);
      const tip = ground.lowestPoint(rig, PROBES);
      expect(tip).toBe(bruteForceLowest(rig));
      posed(rig, "contain", {}, 0, 0, 0);
      const contain = ground.lowestPoint(rig, PROBES);
      expect(contain).toBe(bruteForceLowest(rig));
      expect(tip, "and the two really are different cases").toBeLessThan(contain - 0.1);
    }
  });
});

describe("rig3d/ground: probesFrom", () => {
  it("covers every bone the geometry actually uses", () => {
    const used = new Set<string>();
    for (const vertex of MESH.verts) {
      const name = BONE_ORDER[vertex.bone];
      if (name !== undefined) {
        used.add(name);
      }
    }
    expect(new Set(PROBES.probes.map((p) => p.bone))).toEqual(used);
    const total = PROBES.probes.reduce((n, p) => n + p.points.length / 3, 0);
    expect(total, "and every vertex, exactly once").toBe(MESH.verts.length);
  });

  // A vertex whose bone index is off the end of the skeleton is the failure
  // `skeleton.ts`'s BONE INDEX CONTRACT exists to prevent, and dropping it
  // silently here would hide a part from the ground scan -- the one place
  // where "renders fine, measures fine, hangs through the pitch" is possible.
  it("refuses a vertex on a bone the skeleton does not have", () => {
    const stray = {
      position: [0, 0, 0] as const,
      normal: [0, 1, 0] as const,
      paletteSlot: 0,
      bone: BONE_ORDER.length,
      material: "plain",
    };
    expect(() => {
      ground.probesFrom(RIG, [stray as unknown as (typeof MESH.verts)[number]], BONE_ORDER);
    }).toThrow(/unknown bone index/);
  });
});

describe("rig3d/ground: how many times poseAndGround evaluates the skeleton", () => {
  // THE REGRESSION THIS EXISTS FOR. The rig plants 1.2 mm under the plane at
  // rest, so before that constant moved to build time (`probesFrom`'s
  // `restLift` + `skeleton.raised`) EVERY idle character measured below zero,
  // took the correction branch, and paid a second `skeleton.apply` -- 60 times
  // a second, to be raised 0.02 px. Nothing about that is visible,
  // which is why it survived a review that checked the pictures.
  //
  // Deterministic, and it goes red both ways: raise the rig and an idle
  // character costs one evaluation; hand `poseAndGround` the UNRAISED rig and
  // the same character costs two. Both directions are asserted, so this cannot
  // pass by measuring nothing.
  const raised = (): skeleton.Rig => skeleton.raised(skeleton.newRig(RIG), PROBES.restLift);

  // #564's hit-reaction envelope makes `combat_knockback`/`combat_stagger`'s
  // magnitude depend on a per-slot latch (`action_pose.ts`'s
  // `hitReactionStates`), keyed by the slot id `appliesFor` mints fresh every
  // call. A stale latch from one test's slot id could never leak into
  // another's (`seq` only grows), but reset anyway so this describe block's
  // behaviour does not depend on that being true forever.
  beforeEach(() => {
    actionPose.resetHitReactions();
  });

  function appliesFor(
    rig: skeleton.Rig,
    id: string | undefined,
    extra: Partial<actionPose.ActionPoseOptions>,
    speed: number,
    gait: number,
  ) {
    seq += 1;
    const opts =
      id === undefined
        ? {}
        : { pose: { id }, dive_dir: { x: 1, y: 0 }, facing: { x: 0, y: 1 }, ...extra };
    const pose = animator.poseFor(`c_${String(seq)}`, { speed, gait }, opts, 0);
    applies.count = 0;
    const lift = ground.poseAndGround(rig, pose, PROBES);
    return { applies: applies.count, lift };
  }

  // Lands a forced reaction pose (`combat_knockback`/`combat_stagger`)
  // exactly on its hit-reaction HOLD plateau (elapsed ticks ==
  // HIT_ATTACK_TICKS + HIT_SETTLE_TICKS, comfortably short of the recovery
  // tail at HIT_RECOVER_TICKS) instead of a single call's elapsed-ZERO
  // instant, which is all `appliesFor` above can ever sample for one of
  // these two pose ids (see `action_pose.ts`'s own latch doc: the FIRST
  // frame a slot observes a forced window is always elapsed 0, whatever
  // `forced_ticks` value it is handed). This suite's whole point is a
  // DETERMINISTIC "this pose id genuinely penetrates" claim, and elapsed 0
  // is a multiplier of exactly zero -- no tilt, no penetration, no claim
  // left to test. Reached by threading the SAME slot id through several
  // consecutive `forced_ticks` values, the exact path `appliesFor` never
  // exercises, so this is deliberately its own helper rather than a mode of
  // `appliesFor`.
  function forcedReactionPoseAtHold(
    id: "combat_knockback" | "combat_stagger",
  ): actionPose.MutablePose {
    seq += 1;
    const slotId = `c_${String(seq)}`;
    const opts = { pose: { id }, dive_dir: { x: 1, y: 0 }, facing: { x: 0, y: 1 } };
    const holdElapsedTicks = 7; // HIT_ATTACK_TICKS (4) + HIT_SETTLE_TICKS (3)
    const total = 20; // comfortably past holdElapsedTicks + HIT_RECOVER_TICKS (8)
    let pose: actionPose.MutablePose | undefined;
    for (let elapsed = 0; elapsed <= holdElapsedTicks; elapsed += 1) {
      pose = animator.poseFor(
        slotId,
        { speed: 0, gait: 0 },
        { ...opts, forced_ticks: total - elapsed },
        elapsed / 60,
      );
    }
    if (pose === undefined) {
      throw new Error("ground.spec.ts: forcedReactionPoseAtHold produced no pose");
    }
    return pose;
  }

  it("evaluates once for an idle character, at every phase of the stride", () => {
    const rig = raised();
    for (let i = 0; i < 24; i += 1) {
      const r = appliesFor(rig, undefined, {}, 0, i / 24);
      expect(r.applies, `idle phase ${String(i)} must cost one skeleton evaluation`).toBe(1);
      expect(r.lift, "and no lift at all").toBe(0);
    }
  });

  it("evaluates once for a walking and a running character", () => {
    const rig = raised();
    for (const speed of [90, 260]) {
      for (let i = 0; i < 24; i += 1) {
        const r = appliesFor(rig, undefined, {}, speed, i / 24);
        expect(r.applies, `speed ${String(speed)} phase ${String(i)}`).toBe(1);
        expect(r.lift).toBe(0);
      }
    }
  });

  it("evaluates twice for a character whose pose is genuinely penetrating", () => {
    const rig = raised();
    for (const [id, dive] of [
      ["keeper_dive", 1],
      ["keeper_tip", 1],
      ["contain", 0],
    ] as const) {
      const r = appliesFor(rig, id, { dive }, 0, 0);
      expect(r.applies, `${id} penetrates, so it costs the correction`).toBe(2);
      expect(r.lift, `${id} really is lifted`).toBeGreaterThan(0);
    }

    // `combat_stagger`, landed at its hit-reaction HOLD plateau rather than
    // through `appliesFor` -- see `forcedReactionPoseAtHold`'s own doc on
    // why a single fresh-slot call can never sample anything but elapsed 0
    // (no tilt at all) for a forced reaction pose.
    const staggerPose = forcedReactionPoseAtHold("combat_stagger");
    applies.count = 0;
    const staggerLift = ground.poseAndGround(rig, staggerPose, PROBES);
    expect(applies.count, "combat_stagger penetrates, so it costs the correction").toBe(2);
    expect(staggerLift, "combat_stagger really is lifted").toBeGreaterThan(0);
  });

  // The other direction, which is what makes the three above non-vacuous: with
  // the rig's constant NOT resolved at construction, an idle character pays the
  // second evaluation. This is the state the fix moved away from, asserted so
  // that moving back is a failing test rather than a silent 1.9x on every
  // standing player.
  it("would evaluate twice for an idle character on an unraised rig", () => {
    const rig = skeleton.newRig(RIG);
    const r = appliesFor(rig, undefined, {}, 0, 0);
    expect(r.applies, "an unraised rig re-measures its own rest pose every frame").toBe(2);
    expect(r.lift, "and lifts by exactly the constant that should have been built in").toBeCloseTo(
      PROBES.restLift,
      12,
    );
  });
});
