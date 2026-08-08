// The presentation-agnostic drawing surface `draw.ts` and `tuning_panel.ts`
// render through, standing in for `love.graphics`.
//
// LÖVE's `love.graphics` calls have no TypeScript equivalent yet, and
// wiring one — a three.js or Canvas2D implementation of this interface — is
// deliberately out of scope for this milestone (v2/README.md §1: "the glue
// that makes a playable browser build ... is a separate milestone. Do not
// build it, and do not block on it."). This interface exists so `draw.ts`
// can be ported now and implemented later without another pass over its
// logic; nothing in this package implements it.

import type { FontKind } from "./theme.ts";

export type DrawMode = "fill" | "line";
export type TextAlign = "left" | "center" | "right";

export interface Dimensions {
  readonly width: number;
  readonly height: number;
}

export interface GraphicsBackend {
  /** Actual (pre-letterbox) drawable size, e.g. the canvas/window size in px. */
  getDimensions(): Dimensions;
  setColor(r: number, g: number, b: number, a?: number): void;
  setLineWidth(width: number): void;
  rectangle(
    mode: DrawMode,
    x: number,
    y: number,
    w: number,
    h: number,
    rx?: number,
    ry?: number,
  ): void;
  circle(mode: DrawMode, x: number, y: number, radius: number): void;
  ellipse(mode: DrawMode, x: number, y: number, radiusX: number, radiusY: number): void;
  /** `points` is a flat `[x0, y0, x1, y1, ...]` list, matching `love.graphics.polygon`. */
  polygon(mode: DrawMode, points: readonly number[]): void;
  line(x1: number, y1: number, x2: number, y2: number): void;
  print(text: string, x: number, y: number): void;
  printf(text: string, x: number, y: number, wrapWidth: number, align: TextAlign): void;
  push(): void;
  pop(): void;
  translate(x: number, y: number): void;
  scale(sx: number, sy: number): void;
  /** Selects a themed font. The backend owns font creation/caching (LÖVE's `newFont` + a cache table, folded in). */
  setFont(kind: FontKind): void;
}
