import { describe, expect, it } from "vitest";
import { mat4, type Mat4 } from "./mat4.ts";

function expectNear(actual: number, expected: number, eps = 1e-6): void {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(eps);
}

function nearVec(
  ax: number,
  ay: number,
  az: number,
  bx: number,
  by: number,
  bz: number,
  tol = 1e-9,
): void {
  expectNear(ax, bx, tol);
  expectNear(ay, by, tol);
  expectNear(az, bz, tol);
}

// Element-wise closeness across a full Mat4. Written with literal indices
// (rather than a `for` loop over a computed index) so every element access
// stays a known `number` under `noUncheckedIndexedAccess` -- no assertions.
function expectMatNear(actual: Mat4, expected: Mat4, tol = 1e-9): void {
  expectNear(actual[0], expected[0], tol);
  expectNear(actual[1], expected[1], tol);
  expectNear(actual[2], expected[2], tol);
  expectNear(actual[3], expected[3], tol);
  expectNear(actual[4], expected[4], tol);
  expectNear(actual[5], expected[5], tol);
  expectNear(actual[6], expected[6], tol);
  expectNear(actual[7], expected[7], tol);
  expectNear(actual[8], expected[8], tol);
  expectNear(actual[9], expected[9], tol);
  expectNear(actual[10], expected[10], tol);
  expectNear(actual[11], expected[11], tol);
  expectNear(actual[12], expected[12], tol);
  expectNear(actual[13], expected[13], tol);
  expectNear(actual[14], expected[14], tol);
  expectNear(actual[15], expected[15], tol);
}

describe("mat4", () => {
  it("stores translation row-major, in m[3], m[7], m[11]", () => {
    // Row-major is not cosmetic: it is what the Lua original's LÖVE shader
    // uniform expects, and this port preserves the layout so quat.toMat4
    // stays interchangeable. Getting it wrong sends a transposed matrix with
    // no error.
    const m = mat4.translation(1, 2, 3);
    expect(m[3]).toBe(1);
    expect(m[7]).toBe(2);
    expect(m[11]).toBe(3);
  });

  it("multiply applies the right-hand matrix first", () => {
    const move = mat4.translation(10, 0, 0);
    const spin = mat4.rotationY(Math.PI / 2);
    // move * spin: rotate, then translate along world x.
    const [x, y, z] = mat4.transformPoint(mat4.multiply(move, spin), 0, 0, 1);
    nearVec(x, y, z, 11, 0, 0, 1e-9);
    // spin * move: translate, then rotate the whole thing.
    const [x2, y2, z2] = mat4.transformPoint(mat4.multiply(spin, move), 0, 0, 1);
    nearVec(x2, y2, z2, 1, 0, -10, 1e-9);
  });

  it("rotations turn the expected way", () => {
    let [x, y, z] = mat4.transformPoint(mat4.rotationY(Math.PI / 2), 0, 0, 1);
    nearVec(x, y, z, 1, 0, 0, 1e-9);
    [x, y, z] = mat4.transformPoint(mat4.rotationX(Math.PI / 2), 0, 1, 0);
    nearVec(x, y, z, 0, 0, 1, 1e-9);
    [x, y, z] = mat4.transformPoint(mat4.rotationZ(Math.PI / 2), 1, 0, 0);
    nearVec(x, y, z, 0, 1, 0, 1e-9);
  });

  it("transformDirection ignores translation", () => {
    const m = mat4.multiply(mat4.translation(100, 100, 100), mat4.rotationY(Math.PI / 2));
    const [x, y, z] = mat4.transformDirection(m, 0, 0, 1);
    nearVec(x, y, z, 1, 0, 0, 1e-9);
  });

  it("lookAt puts the target in front of the eye", () => {
    const view = mat4.lookAt([0, 0, 5], [0, 0, 0]);
    // The view basis has -z forward, so a target ahead lands at negative z.
    const [, , z] = mat4.transformPoint(view, 0, 0, 0);
    expect(z, `target should be down the negative z axis, got ${z}`).toBeLessThan(0);
    // The eye maps to the origin of view space.
    const [ex, ey, ez] = mat4.transformPoint(view, 0, 0, 5);
    nearVec(ex, ey, ez, 0, 0, 0, 1e-9);
  });

  it("perspective maps the near and far planes to the clip range", () => {
    const nearPlane = 1;
    const farPlane = 100;
    const p = mat4.perspective(60, 16 / 9, nearPlane, farPlane);
    const ndcZ = (depth: number): number => {
      // A point `depth` in front of the camera sits at z = -depth.
      const z = p[8] * 0 + p[10] * -depth + p[11];
      const w = p[12] * 0 + p[14] * -depth + p[15];
      return z / w;
    };
    expectNear(ndcZ(nearPlane), -1, 1e-6);
    expectNear(ndcZ(farPlane), 1, 1e-6);
  });

  it("uniformScale keeps direction and scales length", () => {
    const [x, y, z] = mat4.transformDirection(mat4.uniformScale(3), 0, 2, 0);
    nearVec(x, y, z, 0, 6, 0, 1e-9);
  });

  it("chain composes left to right", () => {
    const direct = mat4.multiply(mat4.translation(1, 0, 0), mat4.rotationY(Math.PI));
    const chained = mat4.chain(mat4.translation(1, 0, 0), mat4.rotationY(Math.PI));
    expectMatNear(chained, direct, 1e-12);
  });
});
