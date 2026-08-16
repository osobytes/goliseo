// Impure rendering of pure UI layouts. Screens own state and layout (see
// hit.ts/focus.ts); this file owns the shared product look and every draw
// call in non-match UI. It is the one impure module in this package:
// everything here targets `GraphicsBackend` (graphics_backend.ts) rather
// than a concrete rendering engine directly, since no browser implementation
// of that interface exists yet (see graphics_backend.ts's header comment).
//
// An earlier implementation also detected a headless/stub backend missing
// push/pop/translate/scale before using them. That was a defense against
// partial stub backends in tests; `GraphicsBackend` is a full contract
// instead; a test double just needs to implement every method (as no-ops if
// it likes), so that check is dropped here.

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

// --- backdrop geometry, in virtual (960x540) pixels -------------------------
//
// The menu backdrop is the coliseum seen from the pitch: a row of arches
// along the upper bowl, an amber fascia line under them, and brazier light
// pooling on the arena floor. It replaces a starfield of thirteen hardcoded
// stars that predated the building.

/** Vertical bands the sky gradient is quantized into. The backend has no gradient primitive. */
const SKY_BANDS = 16;
/** The arcade occupies the top of the frame; arches are cut into it. */
const ARCADE_BOTTOM = 68;
/** One pier + one arch opening. */
const ARCH_PITCH = 58;
const ARCH_WIDTH = 30;
const ARCH_INSET = 14;
/** The pitch ellipse: centre, then radii. Its top arc sweeps the lower frame. */
const FLOOR_CX = 480;
const FLOOR_CY = 511;
const FLOOR_RX = 380;
const FLOOR_RY = 75;
/** The brazier pool, centred below the frame so only its upper half shows. */
const BRAZIER_CY = 637;
const BRAZIER_RX = 1152;
const BRAZIER_RY = 421;
/** Steps in a soft radial pool. More is smoother and costs one ellipse each. */
const GLOW_STEPS = 6;

function setColor(backend: GraphicsBackend, color: RgbColor, alpha = 1): void {
  backend.setColor(color[0], color[1], color[2], alpha);
}

/**
 * Draw `text` inside `rect`, vertically centred on measured metrics.
 *
 * This replaced three different rules that all got it wrong in their own way:
 * `rect.y + rect.h / 2 - 7` for a short button (a LÖVE-calibrated half-line
 * height for a font this build does not ship — #408), a flat `rect.y + 10`
 * for a card, and a bare `rect.y` for a label. None of them knew how many
 * lines the text would wrap to, so multi-line text hung out of its box while
 * single-line text sat high in it.
 *
 * Returns the height the text actually occupied, so a caller can tell whether
 * it fitted.
 */
function printCentred(
  backend: GraphicsBackend,
  text: string,
  rect: Rect,
  align: TextAlign,
  insetLeft: number,
  insetRight: number = insetLeft,
): number {
  const width = Math.max(0, rect.w - insetLeft - insetRight);
  const metrics = backend.measureText(text, width);
  const top = rect.y + Math.max(0, (rect.h - metrics.height) / 2);
  backend.printf(text, rect.x + insetLeft, top, width, align);
  return metrics.height;
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
    printCentred(backend, widget.text, rect, align, align === "left" ? 16 : 0);
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
    // Selection is amber; only focus is cyan. See theme.ts's header.
    setColor(backend, COLORS.amber);
    backend.circle("fill", rect.x + rect.w - 13, rect.y + rect.h - 13, 4);
  }
  if (widget.text !== undefined) {
    backend.setFont("body");
    setColor(backend, COLORS.text);
    printCentred(backend, widget.text, rect, data.align ?? "left", textInset, 12);
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
    color = COLORS.amber;
  } else if (data.tone === "muted") {
    color = COLORS.textMuted;
  }

  backend.setFont(fontKind);
  const text = widget.text ?? "";
  const align = data.align ?? "left";
  if (widget.kind === "hero_title") {
    // A brazier-lit halo behind the title, not a cyan one. Offset from the
    // centred position, so the halo tracks the glyphs rather than the box.
    setColor(backend, COLORS.amber, 0.22);
    printCentred(backend, text, { ...rect, x: rect.x + 2, y: rect.y + 3 }, align, 0);
  }
  setColor(backend, color);
  printCentred(backend, text, rect, align, 0);
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
  panelLine(backend, rect, widget.selected ? COLORS.amber : COLORS.border, 0.85);

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

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function mixColor(a: RgbColor, b: RgbColor, t: number): RgbColor {
  return [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)];
}

/**
 * A soft radial pool, approximated as nested ellipses of decreasing alpha
 * because `GraphicsBackend` has no gradient primitive. Alpha falls off
 * linearly rather than smoothly; at these sizes the banding is not visible.
 */
function glow(
  backend: GraphicsBackend,
  cx: number,
  cy: number,
  rx: number,
  ry: number,
  color: RgbColor,
  peakAlpha: number,
): void {
  for (let i = GLOW_STEPS; i >= 1; i -= 1) {
    const t = i / GLOW_STEPS;
    setColor(backend, color, (peakAlpha / GLOW_STEPS) * (1 - t + 1 / GLOW_STEPS));
    backend.ellipse("fill", cx, cy, rx * t, ry * t);
  }
}

/** The sky, quantized into horizontal bands: zenith at the top, horizon at the bottom. */
function drawSky(backend: GraphicsBackend, width: number, height: number): void {
  const bandHeight = height / SKY_BANDS;
  for (let i = 0; i < SKY_BANDS; i += 1) {
    const t = i / (SKY_BANDS - 1);
    // Zenith -> mid over the top third, mid -> horizon over the rest, matching
    // stadium_sky.ts's two-stage mix.
    const color =
      t < 0.35
        ? mixColor(COLORS.zenith, COLORS.skyMid, t / 0.35)
        : mixColor(COLORS.skyMid, COLORS.sky, (t - 0.35) / 0.65);
    setColor(backend, color);
    // Bands overlap by a pixel so no seam shows at fractional scales.
    backend.rectangle("fill", 0, i * bandHeight, width, bandHeight + 1);
  }
}

/** The upper bowl: stone piers with lit arch openings between them. */
function drawArcade(backend: GraphicsBackend, width: number): void {
  for (let x = ARCH_INSET; x < width; x += ARCH_PITCH) {
    // The arch is a rounded opening: a rectangle capped by a semicircle, drawn
    // as one rounded rectangle since only its top corners read at this size.
    setColor(backend, COLORS.sand, 0.13);
    backend.rectangle("fill", x, 0, ARCH_WIDTH, ARCADE_BOTTOM, ARCH_WIDTH / 2, ARCH_WIDTH / 2);
  }
  // The fascia trim under the arcade — a thin accent line, not a light source,
  // matching stadium_bowl.ts's own note about staying below the bloom threshold.
  setColor(backend, COLORS.amber, 0.3);
  backend.rectangle("fill", 0, ARCADE_BOTTOM, width, 2);
}

/** Brazier light pooling on the arena floor, and the pitch ellipse it lands on. */
function drawFloor(backend: GraphicsBackend, width: number, height: number): void {
  glow(backend, width / 2, BRAZIER_CY, BRAZIER_RX, BRAZIER_RY, COLORS.amber, 0.2);
  glow(backend, FLOOR_CX, FLOOR_CY - FLOOR_RY, FLOOR_RX, FLOOR_RY, COLORS.keeper, 0.07);
  setColor(backend, COLORS.keeper, 0.16);
  backend.setLineWidth(1);
  backend.ellipse("line", FLOOR_CX, FLOOR_CY, FLOOR_RX, FLOOR_RY);
  setColor(backend, COLORS.borderSoft, 0.3);
  backend.line(0, height - 20, width, height - 20);
}

function drawBackdrop(backend: GraphicsBackend, width: number, height: number): void {
  drawSky(backend, width, height);
  drawArcade(backend, width);
  drawFloor(backend, width, height);
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

  setColor(backend, COLORS.zenith);
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
    setColor(backend, COLORS.zenith, 0.96);
    backend.rectangle("fill", wipeX, 0, wipeW, base.h);
    setColor(backend, COLORS.amber, 0.75);
    backend.rectangle("fill", wipeX - 2, 0, 2, base.h);
  }

  backend.pop();
}

export const draw = { layout };
