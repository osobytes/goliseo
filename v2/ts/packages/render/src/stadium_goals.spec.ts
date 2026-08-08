// New tests for stadium_goals.ts. No GL context needed -- everything here is
// geometry bounding boxes and object-graph shape (mirrors stadium.spec.ts's
// own style).

import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { buildGoal } from "./stadium_goals.ts";
import type { Rect } from "./pitch.ts";
import type { RGB } from "./draw2d.ts";

const HOME_RECT: Rect = { x: -30, y: 215, w: 30, h: 110 };
const AWAY_RECT: Rect = { x: 960, y: 215, w: 30, h: 110 };
const CROSSBAR_H = 70;
const TEAM_COLOR: RGB = [0.2, 0.55, 1.0];

function findByName(root: THREE.Object3D, name: string): THREE.Object3D | undefined {
  let found: THREE.Object3D | undefined;
  root.traverse((obj) => {
    if (obj.name === name && found === undefined) {
      found = obj;
    }
  });
  return found;
}

describe("buildGoal", () => {
  it("names the returned group 'goal' and does not set the caller's userData tag itself", () => {
    const goal = buildGoal(HOME_RECT, CROSSBAR_H, 0, TEAM_COLOR);
    expect(goal.name).toBe("goal");
    expect(goal.userData["stadiumGoal"]).toBeUndefined();
  });

  it("builds exactly three meshes per goal: frame, trim, net", () => {
    const goal = buildGoal(HOME_RECT, CROSSBAR_H, 0, TEAM_COLOR);
    expect(findByName(goal, "goal_frame")).toBeInstanceOf(THREE.Mesh);
    expect(findByName(goal, "goal_trim")).toBeInstanceOf(THREE.Mesh);
    expect(findByName(goal, "goal_net")).toBeInstanceOf(THREE.Mesh);
    let meshCount = 0;
    goal.traverse((obj) => {
      if (obj instanceof THREE.Mesh) {
        meshCount += 1;
      }
    });
    expect(meshCount).toBe(3);
  });

  it("places the home goal's posts at the mouth line (x=0) and its net back at the rect's far edge (x=-30)", () => {
    const goal = buildGoal(HOME_RECT, CROSSBAR_H, 0, TEAM_COLOR);
    const frame = findByName(goal, "goal_frame");
    if (!(frame instanceof THREE.Mesh)) {
      throw new Error("expected goal_frame");
    }
    frame.geometry.computeBoundingBox();
    const box = frame.geometry.boundingBox;
    if (box === null) {
      throw new Error("expected a bounding box");
    }
    // Front posts (radius 3.5, creative polish pass -- see stadium_goals.ts's
    // POST_RADIUS) at x=0, back posts (radius 2.2, BACK_POST_RADIUS) at x=-30.
    expect(box.max.x).toBeGreaterThanOrEqual(0);
    expect(box.max.x).toBeLessThanOrEqual(4);
    expect(box.min.x).toBeGreaterThanOrEqual(-33);
    expect(box.min.x).toBeLessThanOrEqual(-30);
    expect(box.min.z).toBeGreaterThanOrEqual(HOME_RECT.y - 4);
    expect(box.max.z).toBeLessThanOrEqual(HOME_RECT.y + HOME_RECT.h + 4);
    // Crossbar height.
    expect(box.max.y).toBeGreaterThanOrEqual(CROSSBAR_H);
    expect(box.max.y).toBeLessThanOrEqual(CROSSBAR_H + 4);
  });

  it("places the away goal's posts at the mouth line (x=960) and its net back at the rect's far edge (x=990)", () => {
    const goal = buildGoal(AWAY_RECT, CROSSBAR_H, 960, TEAM_COLOR);
    const frame = findByName(goal, "goal_frame");
    if (!(frame instanceof THREE.Mesh)) {
      throw new Error("expected goal_frame");
    }
    frame.geometry.computeBoundingBox();
    const box = frame.geometry.boundingBox;
    if (box === null) {
      throw new Error("expected a bounding box");
    }
    expect(box.min.x).toBeGreaterThanOrEqual(956);
    expect(box.min.x).toBeLessThanOrEqual(960);
    expect(box.max.x).toBeGreaterThanOrEqual(990);
    expect(box.max.x).toBeLessThanOrEqual(993);
  });

  it("tints the trim and net with the given team colour", () => {
    const red: RGB = [0.9, 0.1, 0.1];
    const goal = buildGoal(HOME_RECT, CROSSBAR_H, 0, red);
    const trim = findByName(goal, "goal_trim");
    if (!(trim instanceof THREE.Mesh) || Array.isArray(trim.material) || !(trim.material instanceof THREE.MeshStandardMaterial)) {
      throw new Error("expected goal_trim to be a MeshStandardMaterial mesh");
    }
    expect(trim.material.color.r).toBeCloseTo(red[0], 5);
    expect(trim.material.color.g).toBeCloseTo(red[1], 5);
    expect(trim.material.color.b).toBeCloseTo(red[2], 5);
    expect(trim.material.emissiveIntensity).toBeGreaterThan(0.7);

    const net = findByName(goal, "goal_net");
    if (!(net instanceof THREE.Mesh) || Array.isArray(net.material) || !(net.material instanceof THREE.ShaderMaterial)) {
      throw new Error("expected goal_net to be a ShaderMaterial mesh");
    }
    const teamColorUniform = net.material.uniforms["uTeamColor"]?.value;
    expect(teamColorUniform).toBeInstanceOf(THREE.Color);
    expect((teamColorUniform as THREE.Color).r).toBeCloseTo(red[0], 5);
  });

  it("builds two independent goals with no shared geometry instances", () => {
    const home = buildGoal(HOME_RECT, CROSSBAR_H, 0, TEAM_COLOR);
    const away = buildGoal(AWAY_RECT, CROSSBAR_H, 960, TEAM_COLOR);
    const homeFrame = findByName(home, "goal_frame");
    const awayFrame = findByName(away, "goal_frame");
    if (!(homeFrame instanceof THREE.Mesh) || !(awayFrame instanceof THREE.Mesh)) {
      throw new Error("expected both frames");
    }
    expect(homeFrame.geometry).not.toBe(awayFrame.geometry);
  });

  it("disposal: every mesh's geometry and material can be disposed without throwing", () => {
    const goal = buildGoal(HOME_RECT, CROSSBAR_H, 0, TEAM_COLOR);
    expect(() => {
      goal.traverse((obj) => {
        if (obj instanceof THREE.Mesh) {
          obj.geometry.dispose();
          const materials = Array.isArray(obj.material) ? obj.material : [obj.material];
          for (const m of materials) {
            m.dispose();
          }
        }
      });
    }).not.toThrow();
  });
});
