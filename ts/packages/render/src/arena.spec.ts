import { describe, expect, it } from "vitest";
import {
  backdropCommands,
  frameCommands,
  type ArenaColors,
  type ArenaThemeColors,
} from "./arena.ts";

const colors: ArenaColors = {
  floor_color: [0.025, 0.16, 0.17],
  rail_color: [0.25, 0.88, 1.0],
  highlight_color: [1.0, 0.66, 0.24],
};
const theme: ArenaThemeColors = { zenith: [0.01, 0.008, 0.03], text: [0.91, 0.96, 1.0] };
const viewport = { w: 1280, h: 720 };

describe("arena.backdropCommands", () => {
  it("fills the whole viewport with the zenith colour first", () => {
    const commands = backdropCommands(colors, viewport, theme);
    const first = commands[0];
    if (first === undefined || first.kind !== "rect") {
      throw new Error("expected the first command to be the backdrop fill");
    }
    expect(first.mode).toBe("fill");
    expect(first.w).toBe(viewport.w);
    expect(first.h).toBe(viewport.h);
    expect(first.color).toEqual(theme.zenith);
  });

  it("draws exactly twelve stars, matching the fixed constellation", () => {
    const commands = backdropCommands(colors, viewport, theme);
    const stars = commands.filter((c) => c.kind === "circle" && c.color === theme.text);
    expect(stars).toHaveLength(12);
  });

  it("draws eight ribbon markers split evenly between rail and highlight colour", () => {
    const commands = backdropCommands(colors, viewport, theme);
    const rail = commands.filter(
      (c) => c.kind === "rect" && c.color === colors.rail_color && c.rx === 3,
    );
    const highlight = commands.filter(
      (c) => c.kind === "rect" && c.color === colors.highlight_color && c.rx === 3,
    );
    expect(rail).toHaveLength(4);
    expect(highlight).toHaveLength(4);
  });
});

describe("arena.frameCommands", () => {
  const corners = { ax: 10, ay: 20, bx: 100, by: 20, cx: 100, cy: 200, dx: 10, dy: 200 };

  it("draws four corner brackets, two per colour", () => {
    const commands = frameCommands(colors, corners);
    const rail = commands.filter((c) => c.kind === "line" && c.color === colors.rail_color);
    const highlight = commands.filter(
      (c) => c.kind === "line" && c.color === colors.highlight_color,
    );
    expect(rail).toHaveLength(2);
    expect(highlight).toHaveLength(2);
  });

  it("brightens and widens the brackets with pulse", () => {
    const still = frameCommands(colors, corners, 0);
    const pulsing = frameCommands(colors, corners, 1);
    const stillLine = still[0];
    const pulsingLine = pulsing[0];
    if (
      stillLine === undefined ||
      pulsingLine === undefined ||
      stillLine.kind !== "line" ||
      pulsingLine.kind !== "line"
    ) {
      throw new Error("expected line commands");
    }
    expect(pulsingLine.alpha ?? 0).toBeGreaterThan(stillLine.alpha ?? 0);
    expect(pulsingLine.lineWidth ?? 0).toBeGreaterThan(stillLine.lineWidth ?? 0);
  });

  it("anchors each bracket at its own corner", () => {
    const commands = frameCommands(colors, corners);
    const first = commands[0];
    if (first === undefined || first.kind !== "line") {
      throw new Error("expected a line command");
    }
    expect(first.points[0]).toBe(corners.ax);
    expect(first.points[1]).toBe(corners.ay);
  });
});
