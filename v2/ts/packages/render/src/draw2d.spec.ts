// New tests for draw2d.ts's pure half (see that file's header). Not a port
// of a Lua spec -- this is new shared infrastructure this package's port
// needed, so there is no Lua original to mirror.

import { describe, expect, it } from "vitest";
import { DrawList, rotateAround, type PivotTransform } from "./draw2d.ts";

function near(actual: number, expected: number, eps = 1e-9): void {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(eps);
}

describe("rotateAround", () => {
  it("leaves the pivot itself fixed under any rotation", () => {
    const t: PivotTransform = { pivotX: 10, pivotY: 20, angle: Math.PI / 3, offsetX: 0, offsetY: 0 };
    const [x, y] = rotateAround(t, 10, 20);
    near(x, 10);
    near(y, 20);
  });

  it("rotates a point 90 degrees around the pivot", () => {
    // (pivot + (1, 0)) rotated 90deg CCW-in-math-convention becomes (pivot + (0, 1)).
    const t: PivotTransform = { pivotX: 0, pivotY: 0, angle: Math.PI / 2, offsetX: 0, offsetY: 0 };
    const [x, y] = rotateAround(t, 1, 0);
    near(x, 0);
    near(y, 1);
  });

  it("applies the offset after rotation, matching push/translate/rotate/translate composition order", () => {
    const t: PivotTransform = { pivotX: 5, pivotY: 5, angle: 0, offsetX: 3, offsetY: -2 };
    const [x, y] = rotateAround(t, 5, 5);
    near(x, 8);
    near(y, 3);
  });

  it("matches the dive lunge transform's shape: rotate about the feet, then travel along the dive axis", () => {
    // Mirrors player_renderer.lua's push(); translate(sx + sign*r*travel*amount, gy);
    // rotate(angle); translate(-sx, -gy) for a keeper_spread dive to the right.
    const sx = 100;
    const gy = 200;
    const r = 12;
    const sign = 1;
    const travel = 0.65;
    const amount = 1;
    const angle = ((28 * Math.PI) / 180) * amount;
    const t: PivotTransform = { pivotX: sx, pivotY: gy, angle, offsetX: sign * r * travel * amount, offsetY: 0 };
    // A point directly "above" the feet (head, at -r on y) should end up
    // rotated toward the dive direction and translated along it.
    const [x, y] = rotateAround(t, sx, gy - r);
    const dy = -r; // (gy - r) - gy
    const c = Math.cos(angle);
    const s = Math.sin(angle);
    near(x, sx + sign * r * travel * amount + -dy * s);
    near(y, gy + dy * c);
  });
});

describe("DrawList", () => {
  it("records commands in call order, matching a sequence of love.graphics calls", () => {
    const dl = new DrawList();
    dl.rect("fill", 0, 0, 10, 10, [1, 0, 0]);
    dl.circle("fill", 5, 5, 3, [0, 1, 0]);
    dl.line([0, 0, 10, 10], [0, 0, 1]);
    expect(dl.commands.map((c) => c.kind)).toEqual(["rect", "circle", "line"]);
  });

  it("omits optional fields entirely rather than writing them as undefined (exactOptionalPropertyTypes)", () => {
    const dl = new DrawList();
    dl.circle("fill", 0, 0, 1, [1, 1, 1]);
    const command = dl.commands[0];
    expect(command).toBeDefined();
    expect(command && "alpha" in command).toBe(false);
  });

  it("transforms a circle's centre under a PivotTransform but keeps its radius", () => {
    const dl = new DrawList();
    const t: PivotTransform = { pivotX: 0, pivotY: 0, angle: Math.PI / 2, offsetX: 0, offsetY: 0 };
    dl.circle("fill", 1, 0, 7, [1, 1, 1], undefined, t);
    const command = dl.commands[0];
    if (command === undefined || command.kind !== "circle") {
      throw new Error("expected a circle command");
    }
    near(command.x, 0);
    near(command.y, 1);
    expect(command.r).toBe(7);
  });

  it("degrades a rotated rect to its four transformed corners as a polygon", () => {
    const dl = new DrawList();
    const t: PivotTransform = { pivotX: 0, pivotY: 0, angle: Math.PI / 2, offsetX: 0, offsetY: 0 };
    dl.rect("fill", 0, 0, 2, 4, [1, 1, 1], undefined, t);
    const command = dl.commands[0];
    if (command === undefined || command.kind !== "polygon") {
      throw new Error("expected a rotated rect to degrade to a polygon");
    }
    expect(command.points.length).toBe(8);
  });

  it("rotates an arc's angle range along with its centre", () => {
    const dl = new DrawList();
    const t: PivotTransform = { pivotX: 0, pivotY: 0, angle: Math.PI / 4, offsetX: 0, offsetY: 0 };
    dl.arc("fill", 0, 0, 5, 0, Math.PI, [1, 1, 1], undefined, t);
    const command = dl.commands[0];
    if (command === undefined || command.kind !== "arc") {
      throw new Error("expected an arc command");
    }
    near(command.angle1, Math.PI / 4);
    near(command.angle2, Math.PI / 4 + Math.PI);
  });

  it("extend() concatenates another command list in order", () => {
    const a = new DrawList();
    a.circle("fill", 0, 0, 1, [1, 0, 0]);
    const b = new DrawList();
    b.circle("fill", 1, 1, 1, [0, 1, 0]);
    b.extend(a.commands);
    expect(b.commands.map((c) => (c.kind === "circle" ? c.x : undefined))).toEqual([1, 0]);
  });
});
