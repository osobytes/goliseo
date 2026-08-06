import { describe, expect, it } from "vitest";
import { hit } from "./hit.ts";
import type { Layout } from "./types.ts";

const layout: Layout = [
  { id: "a", rect: { x: 0, y: 0, w: 100, h: 50 } },
  { id: "b", rect: { x: 0, y: 0, w: 40, h: 40 } }, // overlaps a, drawn later
  { id: "c", rect: { x: 200, y: 200, w: 30, h: 30 } },
];

describe("hit.at", () => {
  it("returns the topmost widget under the point", () => {
    expect(hit.at(layout, 10, 10)).toBe("b"); // b is later in the list -> on top
  });

  it("returns the lower widget where only it covers the point", () => {
    expect(hit.at(layout, 80, 10)).toBe("a");
  });

  it("returns null when nothing is hit", () => {
    expect(hit.at(layout, 500, 500)).toBeNull();
  });
});

describe("hit.find", () => {
  it("finds a widget by id", () => {
    expect(hit.find(layout, "c")?.id).toBe("c");
  });

  it("returns null for an unknown id", () => {
    expect(hit.find(layout, "zzz")).toBeNull();
  });
});
