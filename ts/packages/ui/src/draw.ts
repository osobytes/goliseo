// game/ui/draw.lua — impure rendering of pure UI layouts. Screens own state
// and layout (see hit.ts/focus.ts); this file owns the shared product look
// and every draw call in non-match UI. It is the one impure module in this
// package: everything here targets `GraphicsBackend` (graphics_backend.ts)
// rather than `love.graphics` directly, since no browser implementation of
// that interface exists yet (see graphics_backend.ts's header comment).
//
// The Lua original also detects a headless/stub backend missing
// push/pop/translate/scale before using them (`can_transform`). That was a
// defense against partial LÖVE stubs in tests; `GraphicsBackend` is a full
// contract instead; a test double just needs to implement every method
// (as no-ops if it likes), so that check is dropped here.

import { invariant } from "./assert.ts";
import type { GraphicsBackend } from "./graphics_backend.ts";
import { motion } from "./motion.ts";
import { theme } from "./theme.ts";
import type { FontKind } from "./theme.ts";
import type { Anchor, Layout, Rect, RgbColor, TextAlign, Widget } from "./types.ts";
import { viewport } from "./viewport.ts";

const COLORS = theme.colors;
const PREVIEW_INSET = 7;
const PREVIEW_DOT_RADIUS = 4;

// [fractional x, fractional y, kind]. kind 2 stars are bigger and cyan; the
// same number is reused as both the "is special" flag and the circle radius,
// exactly as the Lua original does.
type StarPoint = readonly [x: number, y: number, kind: 1 | 2];

const STAR_POINTS: readonly StarPoint[] = [
  [0.06, 0.12, 1],
  [0.13, 0.78, 1],
  [0.21, 0.28, 2],
  [0.28, 0.9, 1],
  [0.36, 0.16, 1],
  [0.44, 0.72, 1],
  [0.52, 0.08, 2],
  [0.61, 0.86, 1],
  [0.68, 0.23, 1],
  [0.75, 0.67, 2],
  [0.84, 0.14, 1],
  [0.91, 0.82, 1],
  [0.96, 0.38, 1],
];

function setColor(backend: GraphicsBackend, color: RgbColor, alpha = 1): void {
  backend.setColor(color[0], color[1], color[2], alpha);
}

function requireRect(widget: Widget): Rect {
  invariant(widget.rect !== undefined, `widget "${widget.id}" has no rect to draw`);
  return widget.rect;
}

function panelFill(backend: GraphicsBackend, rect: Rect, color: RgbColor, alpha = 1): void {
  setColor(backend, color, alpha);
  backend.rectangle("fill", rect.x, rect.y, rect.w, rect.h, theme.radius, theme.radius);
}

function panelLine(
  backend: GraphicsBackend,
  rect: Rect,
  color: RgbColor,
  alpha = 1,
  width: number = theme.borderWidth,
): void {
  backend.setLineWidth(width);
  setColor(backend, color, alpha);
  backend.rectangle("line", rect.x, rect.y, rect.w, rect.h, theme.radius, theme.radius);
}

function drawFocus(backend: GraphicsBackend, widget: Widget): void {
  if (!widget.focused || !widget.rect) {
    return;
  }
  const rect = widget.rect;
  panelLine(backend, rect, COLORS.cyan, 1, theme.focusWidth);
  setColor(backend, COLORS.cyan, 0.95);
  backend.polygon("fill", [
    rect.x - 10,
    rect.y + rect.h / 2,
    rect.x - 4,
    rect.y + rect.h / 2 - 5,
    rect.x - 4,
    rect.y + rect.h / 2 + 5,
  ]);
}

function drawButton(backend: GraphicsBackend, widget: Widget): void {
  const rect = requireRect(widget);
  const disabled = widget.data?.disabled === true;
  let fill: RgbColor = widget.selected ? COLORS.panelSelected : COLORS.panelRaised;
  if (disabled) {
    fill = COLORS.disabled;
  } else if (widget.focused) {
    fill = COLORS.panelSelected;
  }
  panelFill(backend, rect, fill, disabled ? 0.6 : 1);
  panelLine(backend, rect, widget.focused ? COLORS.cyan : COLORS.border, disabled ? 0.35 : 0.8);
  drawFocus(backend, widget);

  if (widget.text !== undefined) {
    backend.setFont("body");
    setColor(backend, disabled ? COLORS.textMuted : COLORS.text, disabled ? 0.55 : 1);
    const align: TextAlign = widget.data?.align ?? "center";
    const inset = align === "left" ? 16 : 0;
    const textY = rect.h > 50 ? rect.y + 10 : rect.y + rect.h / 2 - 7;
    backend.printf(widget.text, rect.x + inset, textY, rect.w - inset * 2, align);
  }
}

function drawSpeciesMark(
  backend: GraphicsBackend,
  shape: string | undefined,
  x: number,
  y: number,
  color: RgbColor,
  size = 8,
): void {
  setColor(backend, color);
  if (shape === "broad") {
    backend.rectangle("fill", x - size, y - size * 0.75, size * 2, size * 1.5, 3, 3);
  } else if (shape === "angular") {
    backend.polygon("fill", [
      x,
      y - size * 1.1,
      x + size,
      y + size * 0.9,
      x - size,
      y + size * 0.9,
    ]);
  } else if (shape === "cluster") {
    backend.circle("fill", x - size * 0.55, y + size * 0.35, size * 0.55);
    backend.circle("fill", x + size * 0.55, y + size * 0.35, size * 0.55);
    backend.circle("fill", x, y - size * 0.55, size * 0.55);
  } else {
    backend.circle("fill", x, y, size);
  }
}

function drawCard(backend: GraphicsBackend, widget: Widget): void {
  const rect = requireRect(widget);
  const data = widget.data ?? {};
  const accent = data.accent ?? COLORS.border;
  const fill = widget.selected ? COLORS.panelSelected : COLORS.panel;
  panelFill(backend, rect, fill);
  panelLine(backend, rect, widget.focused ? COLORS.cyan : COLORS.borderSoft, 0.9);
  setColor(backend, accent);
  backend.rectangle("fill", rect.x, rect.y, 5, rect.h, theme.radius, theme.radius);

  let textInset = 14;
  if (data.speciesShape !== undefined) {
    drawSpeciesMark(backend, data.speciesShape, rect.x + 24, rect.y + 24, accent);
    textInset = 44;
  }
  if (data.locked === true) {
    backend.setFont("eyebrow");
    setColor(backend, COLORS.amber);
    backend.printf("LOCKED", rect.x + rect.w - 70, rect.y + 9, 56, "right");
  }
  if (widget.selected === true) {
    setColor(backend, COLORS.cyan);
    backend.circle("fill", rect.x + rect.w - 13, rect.y + rect.h - 13, 4);
  }
  if (widget.text !== undefined) {
    backend.setFont("body");
    setColor(backend, COLORS.text);
    backend.printf(
      widget.text,
      rect.x + textInset,
      rect.y + 10,
      rect.w - textInset - 12,
      data.align ?? "left",
    );
  }
  drawFocus(backend, widget);
}

function drawLabel(backend: GraphicsBackend, widget: Widget): void {
  const rect = requireRect(widget);
  const data = widget.data ?? {};
  let fontKind: FontKind = "body";
  let color: RgbColor = COLORS.text;
  if (widget.kind === "hero_title") {
    fontKind = "hero";
    color = COLORS.text;
  } else if (widget.kind === "title") {
    fontKind = "title";
    color = COLORS.text;
  } else if (widget.kind === "eyebrow") {
    fontKind = "eyebrow";
    color = COLORS.cyan;
  } else if (data.tone === "muted") {
    color = COLORS.textMuted;
  }

  backend.setFont(fontKind);
  if (widget.kind === "hero_title") {
    setColor(backend, COLORS.cyan, 0.22);
    backend.printf(widget.text ?? "", rect.x + 2, rect.y + 3, rect.w, data.align ?? "left");
  }
  setColor(backend, color);
  backend.printf(widget.text ?? "", rect.x, rect.y, rect.w, data.align ?? "left");
}

function anchorPosition(rect: Rect, anchor: Anchor): readonly [x: number, y: number] {
  const innerW = rect.w - PREVIEW_INSET * 2;
  const innerH = rect.h - PREVIEW_INSET * 2;
  return [rect.x + PREVIEW_INSET + anchor.x * innerW, rect.y + PREVIEW_INSET + anchor.y * innerH];
}

function drawFormationPreview(backend: GraphicsBackend, widget: Widget): void {
  const rect = requireRect(widget);
  const data = widget.data;
  invariant(
    data?.keeper !== undefined && data.outfield !== undefined,
    `formation_preview widget "${widget.id}" is missing keeper/outfield data`,
  );
  panelFill(backend, rect, widget.selected ? COLORS.panelSelected : COLORS.panelRaised);
  panelLine(backend, rect, widget.selected ? COLORS.cyan : COLORS.border, 0.85);

  const pitch: Rect = {
    x: rect.x + PREVIEW_INSET,
    y: rect.y + PREVIEW_INSET,
    w: rect.w - PREVIEW_INSET * 2,
    h: rect.h - PREVIEW_INSET * 2,
  };
  setColor(backend, COLORS.pitch);
  backend.rectangle("fill", pitch.x, pitch.y, pitch.w, pitch.h, 3, 3);
  setColor(backend, COLORS.border, 0.55);
  backend.rectangle("line", pitch.x, pitch.y, pitch.w, pitch.h, 3, 3);
  backend.line(pitch.x + pitch.w / 2, pitch.y, pitch.x + pitch.w / 2, pitch.y + pitch.h);

  const markers = data.markers ?? [];
  const [keeperX, keeperY] = anchorPosition(rect, data.keeper);
  const keeperMarker = markers[0];
  drawSpeciesMark(
    backend,
    keeperMarker?.shape ?? "round",
    keeperX,
    keeperY,
    keeperMarker?.color ?? COLORS.keeper,
    PREVIEW_DOT_RADIUS,
  );

  data.outfield.forEach((anchor, i) => {
    const [x, y] = anchorPosition(rect, anchor);
    const marker = markers[i + 1];
    drawSpeciesMark(
      backend,
      marker?.shape ?? "round",
      x,
      y,
      marker?.color ?? COLORS.amber,
      PREVIEW_DOT_RADIUS,
    );
  });
}

function drawBackdrop(backend: GraphicsBackend, width: number, height: number): void {
  setColor(backend, COLORS.space);
  backend.rectangle("fill", 0, 0, width, height);
  setColor(backend, COLORS.nebula, 0.22);
  backend.ellipse("fill", width * 0.16, height * 0.18, width * 0.34, height * 0.22);
  backend.ellipse("fill", width * 0.86, height * 0.82, width * 0.28, height * 0.2);
  for (const [sx, sy, kind] of STAR_POINTS) {
    setColor(backend, kind === 2 ? COLORS.cyan : COLORS.text, kind === 2 ? 0.7 : 0.38);
    backend.circle("fill", sx * width, sy * height, kind);
  }
  setColor(backend, COLORS.borderSoft, 0.25);
  backend.line(0, height - 20, width, height - 20);
}

/** Render a layout into its virtual viewport, letterboxed into the current window. */
function layout(
  backend: GraphicsBackend,
  widgets: Layout,
  viewportSize?: { readonly w: number; readonly h: number },
  transition?: number,
): void {
  const dims = backend.getDimensions();
  const base = viewportSize ?? { w: 960, h: 540 };
  const transform = viewport.create(dims.width, dims.height, base.w, base.h);

  setColor(backend, COLORS.void);
  backend.rectangle("fill", 0, 0, dims.width, dims.height);
  backend.push();
  backend.translate(transform.offsetX, transform.offsetY);
  backend.scale(transform.scale, transform.scale);

  drawBackdrop(backend, base.w, base.h);
  for (const widget of widgets) {
    if (widget.kind === "button") {
      drawButton(backend, widget);
    } else if (widget.kind === "card") {
      drawCard(backend, widget);
    } else if (widget.kind === "formation_preview") {
      drawFormationPreview(backend, widget);
    } else {
      drawLabel(backend, widget);
    }
  }
  if (transition !== undefined && transition < 1) {
    const [wipeX, wipeW] = motion.wipe(transition, base.w);
    setColor(backend, COLORS.void, 0.96);
    backend.rectangle("fill", wipeX, 0, wipeW, base.h);
    setColor(backend, COLORS.cyan, 0.75);
    backend.rectangle("fill", wipeX - 2, 0, 2, base.h);
  }

  backend.pop();
}

export const draw = { layout };
