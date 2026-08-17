// The presentation-agnostic drawing surface `draw.ts` and `tuning_panel.ts`
// render through.
//
// No rendering engine backs this interface yet, and wiring one — a three.js
// or Canvas2D implementation of this interface — is deliberately out of
// scope for this milestone: the glue that makes a playable browser build is
// a separate milestone, and this package does not build it or block on it.
// This interface exists so `draw.ts` can be written
// now and implemented later without another pass over its logic; nothing in
// this package implements it.

import type { FontKind } from "./theme.ts";

export type DrawMode = "fill" | "line";
export type TextAlign = "left" | "center" | "right";

export interface Dimensions {
  readonly width: number;
  readonly height: number;
}

/**
 * What a block of wrapped text will actually occupy in the current font.
 *
 * This exists so `draw.ts` can place text vertically from real metrics
 * instead of a magic constant. The rule it replaced —
 * `rect.y + rect.h / 2 - 7` — was a faithful port of LÖVE's, where the `7`
 * was half a line height of a font this build does not ship, so it landed
 * text high in every box (#408). Only the backend can measure, so only the
 * backend can answer this.
 */
export interface TextBlockMetrics {
  /** How many lines `printf` will wrap the text into at the same width. */
  readonly lines: number;
  /** Line-to-line advance. */
  readonly lineHeight: number;
  /**
   * Total drawn height: the top of the first line to the bottom of the last.
   * Deliberately the only vertical number a caller needs — centring is
   * `rect.y + (rect.h - height) / 2`, with no font arithmetic at the call
   * site to get subtly wrong a second time.
   */
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
  /** `points` is a flat `[x0, y0, x1, y1, ...]` list. */
  polygon(mode: DrawMode, points: readonly number[]): void;
  /**
   * Measure `text` as `printf` would wrap it at `wrapWidth`, in the font
   * `setFont` last selected. Pure: it must not draw.
   */
  measureText(text: string, wrapWidth: number): TextBlockMetrics;
  line(x1: number, y1: number, x2: number, y2: number): void;
  print(text: string, x: number, y: number): void;
  /** `y` is the TOP of the first line (backends render top-baselined). */
  printf(text: string, x: number, y: number, wrapWidth: number, align: TextAlign): void;
  push(): void;
  pop(): void;
  translate(x: number, y: number): void;
  scale(sx: number, sy: number): void;
  /** Selects a themed font. The backend owns font creation/caching. */
  setFont(kind: FontKind): void;
}
