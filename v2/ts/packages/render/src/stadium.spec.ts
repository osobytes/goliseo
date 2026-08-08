// New tests for stadium.ts. Mirrors arena.spec.ts/scene.spec.ts's style:
// no GL context (vitest's default "node" environment, see draw2d.ts's file
// header), so everything asserted here is object-graph shape, uniform
// object identity/mutation, geometry bounds and determinism -- never a
// rendered pixel.

import { describe, expect, it, vi } from "vitest";
import * as THREE from "three";
import { Stadium, type StadiumOptions } from "./stadium.ts";
import { computeStadiumLayout } from "./stadium_layout.ts";
import { CENTER_CIRCLE_RADIUS } from "./stadium_pitch_surface.ts";
import type { RenderFrameField } from "./pitch.ts";
import type { RGB } from "./draw2d.ts";
import type { NumberUniform } from "./stadium_types.ts";

const FIELD: RenderFrameField = {
  w: 960,
  h: 540,
  penalty_box_depth: 95,
  penalty_box_h: 200,
  crossbar_h: 70,
  goal_home: { x: -30, y: 215, w: 30, h: 110 },
  goal_away: { x: 960, y: 215, w: 30, h: 110 },
};

const HOME_COLOR: RGB = [0.2, 0.55, 1.0];
const AWAY_COLOR: RGB = [1.0, 0.42, 0.18];

function baseOptions(overrides: Partial<StadiumOptions> = {}): StadiumOptions {
  return { field: FIELD, home_color: HOME_COLOR, away_color: AWAY_COLOR, ...overrides };
}

function findByName(root: THREE.Object3D, name: string): THREE.Object3D | undefined {
  let found: THREE.Object3D | undefined;
  root.traverse((obj) => {
    if (obj.name === name && found === undefined) {
      found = obj;
    }
  });
  return found;
}

function collectInstancedMeshes(root: THREE.Object3D): THREE.InstancedMesh[] {
  const meshes: THREE.InstancedMesh[] = [];
  root.traverse((obj) => {
    if (obj instanceof THREE.InstancedMesh) {
      meshes.push(obj);
    }
  });
  return meshes;
}

function timeUniformsOf(stadium: Stadium): readonly NumberUniform[] {
  const uniforms = stadium.group.userData["stadiumTimeUniforms"];
  if (!Array.isArray(uniforms)) {
    throw new Error("expected stadiumTimeUniforms on group.userData");
  }
  return uniforms as NumberUniform[];
}

function excitementUniformsOf(stadium: Stadium): readonly NumberUniform[] {
  const uniforms = stadium.group.userData["stadiumExcitementUniforms"];
  if (!Array.isArray(uniforms)) {
    throw new Error("expected stadiumExcitementUniforms on group.userData");
  }
  return uniforms as NumberUniform[];
}

describe("Stadium sub-groups", () => {
  it("has every named sub-group directly under the stadium group", () => {
    const stadium = new Stadium(baseOptions());
    const names = stadium.group.children.map((c) => c.name);
    expect(names).toContain("bowl");
    expect(names).toContain("crowd");
    expect(names).toContain("sky");
    expect(names).toContain("braziers");
    expect(names).toContain("floodlights");
    expect(names).toContain("banners");
    expect(names).toContain("apron");
    expect(names).toContain("pitch_surface");
    expect(names).toContain("goals");
    expect(names).toContain("lighting");
  });

  it("names the stadium's own root group", () => {
    const stadium = new Stadium(baseOptions());
    expect(stadium.group.name).toBe("stadium");
  });
});

describe("Stadium crowd", () => {
  it("high quality seats between 9,000 and 14,000 spectators across two InstancedMeshes", () => {
    const stadium = new Stadium(baseOptions({ quality: "high" }));
    const crowd = findByName(stadium.group, "crowd");
    if (crowd === undefined) {
      throw new Error("expected a crowd sub-group");
    }
    const meshes = collectInstancedMeshes(crowd);
    expect(meshes).toHaveLength(2);
    const total = meshes.reduce((sum, m) => sum + m.count, 0);
    expect(total).toBeGreaterThanOrEqual(9000);
    expect(total).toBeLessThanOrEqual(14000);
  });

  it("low quality seats around 2,500 spectators in one InstancedMesh", () => {
    const stadium = new Stadium(baseOptions({ quality: "low" }));
    const crowd = findByName(stadium.group, "crowd");
    if (crowd === undefined) {
      throw new Error("expected a crowd sub-group");
    }
    const meshes = collectInstancedMeshes(crowd);
    expect(meshes).toHaveLength(1);
    expect(meshes[0]?.count).toBe(2500);
  });

  it("places every crowd instance inside the bowl's outer ellipse and outside the pitch rect", () => {
    const stadium = new Stadium(baseOptions({ quality: "low" }));
    const layout = computeStadiumLayout(FIELD, "low");
    const crowd = findByName(stadium.group, "crowd");
    if (crowd === undefined) {
      throw new Error("expected a crowd sub-group");
    }
    const meshes = collectInstancedMeshes(crowd);
    const matrix = new THREE.Matrix4();
    const position = new THREE.Vector3();
    const scale = new THREE.Vector3();
    const quaternion = new THREE.Quaternion();
    let checked = 0;
    for (const mesh of meshes) {
      for (let i = 0; i < mesh.count; i += 1) {
        mesh.getMatrixAt(i, matrix);
        matrix.decompose(position, quaternion, scale);
        const nx = (position.x - layout.cx) / layout.bowlOuterRx;
        const nz = (position.z - layout.cz) / layout.bowlOuterRz;
        expect(nx * nx + nz * nz).toBeLessThanOrEqual(1.0001);
        const insideRect = position.x >= 0 && position.x <= FIELD.w && position.z >= 0 && position.z <= FIELD.h;
        expect(insideRect).toBe(false);
        checked += 1;
      }
    }
    expect(checked).toBe(2500);
  });
});

describe("Stadium pitch surface", () => {
  it("mirrors the field's own w/h/penalty-box values in its marking uniforms, and pins the circle radius constant", () => {
    const stadium = new Stadium(baseOptions());
    const quad = findByName(stadium.group, "pitch_surface_quad");
    if (!(quad instanceof THREE.Mesh) || Array.isArray(quad.material) || !(quad.material instanceof THREE.ShaderMaterial)) {
      throw new Error("expected the pitch surface quad to be a ShaderMaterial mesh");
    }
    const uniforms = quad.material.uniforms;
    expect(uniforms["uFieldSize"]?.value.x).toBe(FIELD.w);
    expect(uniforms["uFieldSize"]?.value.y).toBe(FIELD.h);
    expect(uniforms["uPenaltyBox"]?.value.x).toBe(FIELD.penalty_box_depth);
    expect(uniforms["uPenaltyBox"]?.value.y).toBe(FIELD.penalty_box_h);
    expect(uniforms["uCircleRadius"]?.value).toBe(CENTER_CIRCLE_RADIUS);
    expect(uniforms["uCircleRadius"]?.value).toBe(70);
  });
});

describe("Stadium.update", () => {
  it("advances every registered uTime uniform (sky/crowd/flames/banners among them) to now", () => {
    const stadium = new Stadium(baseOptions());
    const uniforms = timeUniformsOf(stadium);
    // crowd + sky + braziers(flames) + banners + apron + pitch_surface.
    expect(uniforms.length).toBeGreaterThanOrEqual(6);
    for (const u of uniforms) {
      expect(u.value).toBe(0);
    }
    stadium.update(12.5);
    for (const u of uniforms) {
      expect(u.value).toBe(12.5);
    }
  });
});

describe("Stadium.setExcitement", () => {
  it("clamps to [0, 1] and writes the crowd's excitement uniform", () => {
    const stadium = new Stadium(baseOptions());
    const uniforms = excitementUniformsOf(stadium);
    expect(uniforms.length).toBeGreaterThanOrEqual(1);

    stadium.setExcitement(0.4);
    for (const u of uniforms) {
      expect(u.value).toBe(0.4);
    }

    stadium.setExcitement(5);
    for (const u of uniforms) {
      expect(u.value).toBe(1);
    }

    stadium.setExcitement(-3);
    for (const u of uniforms) {
      expect(u.value).toBe(0);
    }
  });
});

describe("Stadium goals", () => {
  it("tags each goal group and positions the home goal's frame at the rect + mouth line x=0", () => {
    const stadium = new Stadium(baseOptions());
    const goals = findByName(stadium.group, "goals");
    if (goals === undefined) {
      throw new Error("expected a goals sub-group");
    }
    const home = goals.children.find((c) => c.userData["stadiumGoal"] === "home");
    const away = goals.children.find((c) => c.userData["stadiumGoal"] === "away");
    if (home === undefined || away === undefined) {
      throw new Error("expected both home and away goals to be tagged");
    }
    const homeFrame = findByName(home, "goal_frame");
    if (!(homeFrame instanceof THREE.Mesh)) {
      throw new Error("expected a goal_frame mesh");
    }
    homeFrame.geometry.computeBoundingBox();
    const box = homeFrame.geometry.boundingBox;
    if (box === null) {
      throw new Error("expected a bounding box");
    }
    // Mouth at x=0, net box back at x=-30 (FIELD.goal_home), some post radius
    // padding either side (POST_RADIUS 3.5 / BACK_POST_RADIUS 2.2, creative
    // polish pass -- see stadium_goals.ts).
    expect(box.min.x).toBeGreaterThanOrEqual(-34);
    expect(box.max.x).toBeLessThanOrEqual(4);
    expect(box.min.z).toBeGreaterThanOrEqual(FIELD.goal_home.y - 4);
    expect(box.max.z).toBeLessThanOrEqual(FIELD.goal_home.y + FIELD.goal_home.h + 4);
  });

  it("positions the away goal's frame at the rect + mouth line x=field.w", () => {
    const stadium = new Stadium(baseOptions());
    const goals = findByName(stadium.group, "goals");
    if (goals === undefined) {
      throw new Error("expected a goals sub-group");
    }
    const away = goals.children.find((c) => c.userData["stadiumGoal"] === "away");
    if (away === undefined) {
      throw new Error("expected the away goal to be tagged");
    }
    const awayFrame = findByName(away, "goal_frame");
    if (!(awayFrame instanceof THREE.Mesh)) {
      throw new Error("expected a goal_frame mesh");
    }
    awayFrame.geometry.computeBoundingBox();
    const box = awayFrame.geometry.boundingBox;
    if (box === null) {
      throw new Error("expected a bounding box");
    }
    expect(box.min.x).toBeGreaterThanOrEqual(FIELD.w - 4);
    expect(box.max.x).toBeLessThanOrEqual(FIELD.goal_away.x + FIELD.goal_away.w + 4);
  });
});

describe("Stadium.dispose", () => {
  it("disposes every geometry and material under the group", () => {
    const stadium = new Stadium(baseOptions({ quality: "low" }));
    let meshCount = 0;
    stadium.group.traverse((obj) => {
      if (obj instanceof THREE.Mesh || obj instanceof THREE.Line || obj instanceof THREE.Points) {
        meshCount += 1;
      }
    });
    expect(meshCount).toBeGreaterThan(10);

    const geometryDispose = vi.spyOn(THREE.BufferGeometry.prototype, "dispose");
    const materialDispose = vi.spyOn(THREE.Material.prototype, "dispose");
    stadium.dispose();
    expect(geometryDispose).toHaveBeenCalledTimes(meshCount);
    expect(materialDispose.mock.calls.length).toBeGreaterThanOrEqual(meshCount);
    geometryDispose.mockRestore();
    materialDispose.mockRestore();
  });

  it("does not throw when called twice", () => {
    const stadium = new Stadium(baseOptions({ quality: "low" }));
    stadium.dispose();
    expect(() => stadium.dispose()).not.toThrow();
  });
});

describe("Stadium determinism", () => {
  it("produces byte-identical InstancedMesh matrices across two constructions with the same options", () => {
    const a = new Stadium(baseOptions());
    const b = new Stadium(baseOptions());
    const meshesA = collectInstancedMeshes(a.group);
    const meshesB = collectInstancedMeshes(b.group);
    expect(meshesA.length).toBe(meshesB.length);
    for (let i = 0; i < meshesA.length; i += 1) {
      const meshA = meshesA[i];
      const meshB = meshesB[i];
      if (meshA === undefined || meshB === undefined) {
        throw new Error("mismatched mesh list");
      }
      expect(Array.from(meshA.instanceMatrix.array)).toEqual(Array.from(meshB.instanceMatrix.array));
    }
  });
});

describe("Stadium performance budget", () => {
  // This task's own budget: "constructible in <150ms on a laptop,
  // update(now) < 0.2ms CPU". CI hardware varies, so the assertions below
  // use a generous multiple of that target (a regression guard, not a tight
  // pin) rather than the literal number -- the point is to catch an
  // accidental per-frame allocation or an O(n^2) construction path, not to
  // fail on ordinary machine noise.
  it("constructs a high-quality stadium well within budget", () => {
    const t0 = performance.now();
    const stadium = new Stadium(baseOptions({ quality: "high" }));
    const elapsed = performance.now() - t0;
    expect(stadium.group.children.length).toBeGreaterThan(0);
    expect(elapsed).toBeLessThan(1000);
  });

  it("update(now) is cheap -- uniform writes only, no per-call allocation blowing up the average", () => {
    const stadium = new Stadium(baseOptions({ quality: "high" }));
    const iterations = 500;
    const t0 = performance.now();
    for (let i = 0; i < iterations; i += 1) {
      stadium.update(i * 0.016);
    }
    const avg = (performance.now() - t0) / iterations;
    expect(avg).toBeLessThan(5);
  });
});
