// Screen-space backdrop: starfield, a distant planet, orbital rails and a
// pulsing frame around the pitch corners. Pure content -- three.js has no
// direct equivalent to draw onto, so `backdropCommands`/`frameCommands`
// return a `DrawCommand[]` (see draw2d.ts) that `draw` below paints into a
// group.
//
// Boundary notes (ARCHITECTURE.md §4 rule 6 and this package's package.json):
//   - `ArenaData` comes from Rust `crates/gc-data`. Only the three color
//     fields this module reads are declared locally.
//   - `theme.colors.zenith`/`theme.colors.text` would be `@gc/ui`, but
//     `@gc/render`'s package.json does not depend on `@gc/ui` (only
//     `@gc/core`/`@gc/presentation`/`three`), so the two colors this module
//     needs are threaded through as an explicit parameter instead, the same
//     §4 rule 6 injection pattern applies to Rust-owned content.

import * as THREE from "three";
import { DrawList, paint, type DrawCommand, type RGB } from "./draw2d.ts";

/** The slice of Rust `crates/gc-data`'s `ArenaData` this module reads. */
export interface ArenaColors {
  readonly floor_color: RGB;
  readonly rail_color: RGB;
  readonly highlight_color: RGB;
}

/** The slice of `theme.colors` this module reads. */
export interface ArenaThemeColors {
  readonly zenith: RGB;
  readonly text: RGB;
}

export interface ArenaViewport {
  readonly w: number;
  readonly h: number;
}

export interface ArenaCorners {
  readonly ax: number;
  readonly ay: number;
  readonly bx: number;
  readonly by: number;
  readonly cx: number;
  readonly cy: number;
  readonly dx: number;
  readonly dy: number;
}

// A fixed starfield: [xFrac, yFrac, radiusPx].
const STARS: readonly (readonly [number, number, number])[] = [
  [0.05, 0.09, 1],
  [0.12, 0.18, 2],
  [0.2, 0.06, 1],
  [0.29, 0.15, 1],
  [0.37, 0.05, 2],
  [0.46, 0.13, 1],
  [0.55, 0.07, 1],
  [0.64, 0.16, 2],
  [0.72, 0.05, 1],
  [0.81, 0.13, 1],
  [0.9, 0.07, 2],
  [0.96, 0.18, 1],
];

/** Starfield, planet, orbital rails and ribbon markers. */
export function backdropCommands(
  value: ArenaColors,
  viewport: ArenaViewport,
  theme: ArenaThemeColors,
): DrawCommand[] {
  const dl = new DrawList();

  dl.rect("fill", 0, 0, viewport.w, viewport.h, theme.zenith);

  for (const star of STARS) {
    dl.circle("fill", viewport.w * star[0], viewport.h * star[1], star[2], theme.text, {
      alpha: 0.35,
    });
  }

  const cx = viewport.w * 0.5;
  const cy = viewport.h * 0.205;
  dl.circle("fill", cx, cy, viewport.h * 0.075, value.highlight_color, { alpha: 0.12 });
  dl.circle("fill", cx, cy, viewport.h * 0.034, value.highlight_color, { alpha: 0.78 });
  const railLineWidth = Math.max(1, viewport.h / 270);
  dl.ellipse("line", cx, cy, viewport.w * 0.23, viewport.h * 0.068, value.highlight_color, {
    alpha: 0.7,
    lineWidth: railLineWidth,
  });
  dl.ellipse("line", cx, cy, viewport.w * 0.31, viewport.h * 0.09, value.rail_color, {
    alpha: 0.35,
    lineWidth: railLineWidth,
  });

  const ribbonY = viewport.h * 0.222;
  const ribbonLineWidth = Math.max(2, viewport.h / 180);
  dl.line([viewport.w * 0.07, ribbonY, viewport.w * 0.39, ribbonY], value.rail_color, {
    alpha: 0.62,
    lineWidth: ribbonLineWidth,
  });
  dl.line([viewport.w * 0.61, ribbonY, viewport.w * 0.93, ribbonY], value.highlight_color, {
    alpha: 0.62,
    lineWidth: ribbonLineWidth,
  });

  for (let i = 0; i <= 7; i += 1) {
    const x = viewport.w * (0.16 + i * 0.098);
    const color = i < 4 ? value.rail_color : value.highlight_color;
    dl.rect("fill", x, viewport.h * 0.202, viewport.w * 0.066, viewport.h * 0.028, color, {
      alpha: 0.18,
      rx: 3,
      ry: 3,
    });
  }

  return dl.commands;
}

/** Glowing corner brackets around the pitch's projected trapezoid. */
export function frameCommands(
  value: ArenaColors,
  corners: ArenaCorners,
  pulse?: number,
): DrawCommand[] {
  const dl = new DrawList();
  const p = pulse ?? 0;
  const glow = 0.58 + 0.34 * p;
  const lineWidth = 2 + 2 * p;

  dl.line([corners.ax, corners.ay, corners.ax - 10, corners.ay - 25], value.rail_color, {
    alpha: glow,
    lineWidth,
  });
  dl.line([corners.dx, corners.dy, corners.dx - 12, corners.dy + 16], value.rail_color, {
    alpha: glow,
    lineWidth,
  });
  dl.line([corners.bx, corners.by, corners.bx + 10, corners.by - 25], value.highlight_color, {
    alpha: glow,
    lineWidth,
  });
  dl.line([corners.cx, corners.cy, corners.cx + 12, corners.cy + 16], value.highlight_color, {
    alpha: glow,
    lineWidth,
  });

  return dl.commands;
}

/** Impure: paints the backdrop into `group`. Untested -- see this file's header. */
export function drawBackdrop(
  group: THREE.Group,
  value: ArenaColors,
  viewport: ArenaViewport,
  theme: ArenaThemeColors,
): void {
  paint(group, backdropCommands(value, viewport, theme));
}

/** Impure: paints the corner frame into `group`. Untested -- see this file's header. */
export function drawFrame(
  group: THREE.Group,
  value: ArenaColors,
  corners: ArenaCorners,
  pulse?: number,
): void {
  paint(group, frameCommands(value, corners, pulse));
}
