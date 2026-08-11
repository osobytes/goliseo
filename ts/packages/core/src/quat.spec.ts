import { describe, expect, it } from "vitest";
import { quat, type Quat } from "./quat.ts";
import { mat4, type Mat4 } from "./mat4.ts";

function expectNear(actual: number, expected: number, eps = 1e-6): void {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(eps);
}

// Written with literal indices (rather than a `for` loop over a computed
// index) so every element access stays a known `number` under
// `noUncheckedIndexedAccess` -- no assertions, no `any`.
function maxDiff(a: Mat4, b: Mat4): number {
  return Math.max(
    Math.abs(a[0] - b[0]),
    Math.abs(a[1] - b[1]),
    Math.abs(a[2] - b[2]),
    Math.abs(a[3] - b[3]),
    Math.abs(a[4] - b[4]),
    Math.abs(a[5] - b[5]),
    Math.abs(a[6] - b[6]),
    Math.abs(a[7] - b[7]),
    Math.abs(a[8] - b[8]),
    Math.abs(a[9] - b[9]),
    Math.abs(a[10] - b[10]),
    Math.abs(a[11] - b[11]),
    Math.abs(a[12] - b[12]),
    Math.abs(a[13] - b[13]),
    Math.abs(a[14] - b[14]),
    Math.abs(a[15] - b[15]),
  );
}

function yawOf(m: Mat4): number {
  return (Math.atan2(m[2], m[10]) * 180) / Math.PI;
}

describe("quat", () => {
  it("fromEuler agrees with mat4.euler on the same convention", () => {
    // The two must stay interchangeable: clips are authored in Euler and
    // baked to quaternions, and any divergence in the Y*X*Z order would
    // silently rotate every bone in the game.
    const cases: readonly [number, number, number][] = [
      [-125, 0, 16],
      [20, -5, -2],
      [8, -24, 0],
      [46, 30, -12],
      [-16, 12, 40],
    ];
    for (const [ex, ey, ez] of cases) {
      const rx = (ex * Math.PI) / 180;
      const ry = (ey * Math.PI) / 180;
      const rz = (ez * Math.PI) / 180;
      const fromEuler = mat4.transform(0, 0, 0, rx, ry, rz);
      const fromQuat = quat.toMat4(quat.fromEuler(rx, ry, rz), 0, 0, 0);
      const diff = maxDiff(fromEuler, fromQuat);
      expect(diff, `euler ${ex},${ey},${ez} diverged by ${diff}`).toBeLessThan(1e-9);
    }
  });

  it("slerps across the +/-180 seam the short way", () => {
    // Component-wise Euler interpolation takes the 340 degree route here.
    const a = quat.fromEuler(0, (-170 * Math.PI) / 180, 0);
    const b = quat.fromEuler(0, (170 * Math.PI) / 180, 0);
    const mid = yawOf(quat.toMat4(quat.slerp(a, b, 0.5), 0, 0, 0));
    expectNear(Math.abs(mid), 180, 0.001);
  });

  it("slerp endpoints are exact", () => {
    const a = quat.fromEuler((-40 * Math.PI) / 180, 0, (12 * Math.PI) / 180);
    const b = quat.fromEuler((70 * Math.PI) / 180, (30 * Math.PI) / 180, 0);
    expect(
      maxDiff(quat.toMat4(quat.slerp(a, b, 0), 0, 0, 0), quat.toMat4(a, 0, 0, 0)),
    ).toBeLessThan(1e-9);
    expect(
      maxDiff(quat.toMat4(quat.slerp(a, b, 1), 0, 0, 0), quat.toMat4(b, 0, 0, 0)),
    ).toBeLessThan(1e-9);
  });

  it("slerp stays on the unit sphere", () => {
    const a = quat.fromEuler((-125 * Math.PI) / 180, 0, (16 * Math.PI) / 180);
    const b = quat.fromEuler((46 * Math.PI) / 180, (30 * Math.PI) / 180, 0);
    for (let i = 0; i <= 10; i++) {
      const q: Quat = quat.slerp(a, b, i / 10);
      const len = Math.sqrt(q[0] ** 2 + q[1] ** 2 + q[2] ** 2 + q[3] ** 2);
      expectNear(len, 1, 1e-9);
    }
  });

  it("multiply composes: applying b then a", () => {
    const a = quat.fromEuler((30 * Math.PI) / 180, 0, 0);
    const b = quat.fromEuler(0, (45 * Math.PI) / 180, 0);
    const composed = quat.toMat4(quat.multiply(a, b), 0, 0, 0);
    const expected = mat4.multiply(quat.toMat4(a, 0, 0, 0), quat.toMat4(b, 0, 0, 0));
    expect(maxDiff(composed, expected), "quat multiply must match matrix multiply").toBeLessThan(
      1e-9,
    );
  });

  it("normalize survives a degenerate quaternion", () => {
    const q = quat.normalize([0, 0, 0, 0]);
    expectNear(q[3], 1, 1e-12);
  });

  it("toMat4 carries the translation", () => {
    const m = quat.toMat4(quat.identity(), 1.5, -2.5, 3.5);
    expectNear(m[3], 1.5, 1e-12);
    expectNear(m[7], -2.5, 1e-12);
    expectNear(m[11], 3.5, 1e-12);
  });
});
