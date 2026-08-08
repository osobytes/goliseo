import { mat4, quat } from "@gc/core";
import { describe, expect, it } from "vitest";
import { RIG_MEDIUM } from "./proportions.ts";
import * as skeleton from "./skeleton.ts";

const RIG = RIG_MEDIUM;

describe("rig3d skeleton", () => {
  it("lists every bone parent-before-child", () => {
    const seen = new Set<string>();
    for (const bone of skeleton.bones(RIG)) {
      if (bone.parent !== null) {
        expect(seen.has(bone.parent), `${bone.name} precedes its parent`).toBe(true);
      }
      seen.add(bone.name);
    }
  });

  it("stands the rig on the ground with feet below the hips", () => {
    const rig = skeleton.newRig(RIG);
    const [, hipsY] = skeleton.jointPosition(rig, "hips");
    const [, footY] = skeleton.jointPosition(rig, "foot.R");
    expect(footY, "foot should sit below the hips").toBeLessThan(hipsY);
    expect(footY, `foot should not start below the ground, got ${footY}`).toBeGreaterThanOrEqual(0);
  });

  it("mirrors left and right limbs about the centre line", () => {
    const rig = skeleton.newRig(RIG);
    const [rx] = skeleton.jointPosition(rig, "hand.R");
    const [lx] = skeleton.jointPosition(rig, "hand.L");
    expect(rx).toBeCloseTo(-lx, 9);
  });

  it("applies a bone's rest rotation even with an empty pose", () => {
    // socket_hand carries a 145 degree rest roll; a weapon hangs off it, so
    // losing the rest rotation would point every blade the wrong way.
    const rig = skeleton.newRig(RIG);
    const socket = rig.world["socket_hand.R"];
    const hand = rig.world["hand.R"];
    expect(socket).toBeDefined();
    expect(hand).toBeDefined();
    if (!socket || !hand) {
      throw new Error("unreachable");
    }
    const [sx, sy, sz] = mat4.transformDirection(socket, 0, 1, 0);
    const [hx, hy, hz] = mat4.transformDirection(hand, 0, 1, 0);
    const dot = sx * hx + sy * hy + sz * hz;
    expect(dot, "socket should be rolled well away from the hand axis").toBeLessThan(0.5);
  });

  it("propagates a parent rotation to its children", () => {
    const rig = skeleton.newRig(RIG);
    const before = skeleton.jointPosition(rig, "hand.R")[1];
    skeleton.apply(rig, {
      rot: { "upper_arm.R": quat.fromEuler((-90 * Math.PI) / 180, 0, 0) },
      move: {},
    });
    const after = skeleton.jointPosition(rig, "hand.R")[1];
    expect(after, "raising the upper arm must lift the hand").toBeGreaterThan(before + 0.05);
  });
});

// #337 slice 2: bone index contract. `boneRows` / `ROWS_PER_BONE` -- the
// three-rows-per-bone matrix packing that only existed to fit LÖVE's custom
// shader inside GLSL ES 1.00's 128 vertex-uniform-vector floor -- are not
// ported: three.js's own `Skeleton` uploads bone matrices as a `DataTexture`,
// not a hand-packed uniform array, so that specific budget arithmetic no
// longer applies. See geometry.ts's header and this package's port report.
describe("rig3d bone index contract (#337 slice 2)", () => {
  it("indexes every bone densely from 0", () => {
    const bones = skeleton.bones(RIG);
    const index = skeleton.boneIndex(RIG);
    expect(skeleton.boneCount(RIG)).toBe(bones.length);
    const seen = new Set<number>();
    bones.forEach((bone, i) => {
      expect(index[bone.name], bone.name).toBe(i);
      const value = index[bone.name];
      expect(value).toBeDefined();
      if (value !== undefined) {
        expect(seen.has(value), `bone index must be unique: ${bone.name}`).toBe(false);
        seen.add(value);
      }
    });
  });

  it.skip(
    "flattens the posed skeleton into three rows per bone, in index order -- " +
      "dropped: boneRows/ROWS_PER_BONE packed bone matrices into a hand-rolled " +
      "uniform array to fit LÖVE's GLSL ES 1.00 budget (renderer.lua, out of " +
      "this port's scope). three.js's Skeleton uploads bone matrices via a " +
      "DataTexture instead, so this packing scheme has no three.js equivalent.",
    () => {
      // Intentionally not ported; see skip reason above.
    },
  );

  it.skip(
    "drops only the fourth row, which is always exactly (0, 0, 0, 1) -- " +
      "dropped for the same reason as the row-packing test above.",
    () => {
      // Intentionally not ported; see skip reason above.
    },
  );

  it.skip(
    "refills a reused row buffer rather than reallocating it -- dropped for " +
      "the same reason: boneRows itself was not ported.",
    () => {
      // Intentionally not ported; see skip reason above.
    },
  );
});

describe("rig3d vertex uniform budget (#337 slice 2)", () => {
  it.skip(
    "fits the GLSL ES 1.00 guaranteed floor of 128 vertex uniform vectors -- " +
      "this pinned a constraint specific to LÖVE's hand-written shader " +
      "(renderer.lua, out of this port's scope, replaced by three.js's " +
      "MeshStandardMaterial / SkinnedMesh pipeline, which does not upload bone " +
      "matrices as vertex-shader uniforms at all).",
    () => {
      // Intentionally not ported; see skip reason above.
    },
  );
});
