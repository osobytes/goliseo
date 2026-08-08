import { describe, expect, it } from "vitest";
import { Vec2 } from "./vec2.ts";

function expectNear(actual: number, expected: number, eps = 1e-6): void {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(eps);
}

describe("Vec2", () => {
  it("adds component-wise", () => {
    const v = new Vec2(1, 2).add(new Vec2(3, 4));
    expect(v.x).toBe(4);
    expect(v.y).toBe(6);
  });

  it("subtracts component-wise", () => {
    const v = new Vec2(5, 5).sub(new Vec2(2, 1));
    expect(v.x).toBe(3);
    expect(v.y).toBe(4);
  });

  it("scales", () => {
    const v = new Vec2(2, -3).scale(2);
    expect(v.x).toBe(4);
    expect(v.y).toBe(-6);
  });

  it("computes length", () => {
    expectNear(new Vec2(3, 4).length(), 5);
  });

  it("normalizes to unit length", () => {
    expectNear(new Vec2(0, 10).normalized().length(), 1);
    expect(new Vec2(0, 10).normalized().y).toBe(1);
  });

  it("normalizes the zero vector to zero (no NaN)", () => {
    const n = new Vec2(0, 0).normalized();
    expect(n.x).toBe(0);
    expect(n.y).toBe(0);
  });

  it("measures distance", () => {
    expectNear(new Vec2(0, 0).dist(new Vec2(3, 4)), 5);
  });
});
