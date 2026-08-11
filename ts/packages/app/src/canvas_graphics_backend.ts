// A Canvas2D implementation of `@gc/ui`'s `GraphicsBackend` -- the interface
// `draw.ts` declares but does not itself implement (that package's own
// header notes wiring one up is out of scope for it): this file is that
// implementation, the thing that makes the app shell actually render in a
// browser. Every menu screen (title, squad, formation, tactic, pause,
// settings, credits, help, result, the fake match fixture) draws through
// `@gc/ui`'s `draw.layout`, which draws through nothing but this interface
// -- without an implementation, none of those screens produce a single
// pixel.
//
// Scope: a straightforward, readable mapping onto `CanvasRenderingContext2D`.
// Not pixel-tuned against any reference build -- there is no baseline to
// diff against (the asset/font pipeline is out of this milestone's scope) -- so
// font family is a generic system stack and `printf`'s word-wrap is a plain
// greedy line-break, not exact per-glyph metrics. Colors, layout rects, and
// draw order all come straight from `@gc/ui`'s pure `layout()`/`draw.ts`,
// which this file does not reinterpret.

import type { Dimensions, DrawMode, GraphicsBackend, TextAlign } from "@gc/ui";
import { theme, type FontKind } from "@gc/ui";

const FONT_FAMILY = "system-ui, sans-serif";

function toCssColor(r: number, g: number, b: number, a: number): string {
  const byte = (c: number): number => Math.max(0, Math.min(255, Math.round(c * 255)));
  return `rgba(${byte(r)}, ${byte(g)}, ${byte(b)}, ${a})`;
}

/** Greedy word-wrap: the longest prefix of words that still fits `maxWidth`, repeated. */
function wrapLines(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string[] {
  if (maxWidth <= 0) {
    return [text];
  }
  const words = text.split(/\s+/).filter((w) => w.length > 0);
  if (words.length === 0) {
    return [""];
  }
  const lines: string[] = [];
  let current = "";
  for (const word of words) {
    const candidate = current === "" ? word : `${current} ${word}`;
    if (ctx.measureText(candidate).width > maxWidth && current !== "") {
      lines.push(current);
      current = word;
    } else {
      current = candidate;
    }
  }
  if (current !== "") {
    lines.push(current);
  }
  return lines;
}

export class CanvasGraphicsBackend implements GraphicsBackend {
  private readonly ctx: CanvasRenderingContext2D;
  private fontSize = theme.fonts.body;

  constructor(ctx: CanvasRenderingContext2D) {
    this.ctx = ctx;
    this.ctx.textBaseline = "top";
    this.setFont("body");
  }

  getDimensions(): Dimensions {
    return { width: this.ctx.canvas.width, height: this.ctx.canvas.height };
  }

  setColor(r: number, g: number, b: number, a = 1): void {
    const color = toCssColor(r, g, b, a);
    this.ctx.fillStyle = color;
    this.ctx.strokeStyle = color;
  }

  setLineWidth(width: number): void {
    this.ctx.lineWidth = width;
  }

  private roundedRectPath(x: number, y: number, w: number, h: number, rx = 0, ry = 0): void {
    const radius = Math.max(0, Math.min(rx, ry, Math.abs(w) / 2, Math.abs(h) / 2));
    this.ctx.beginPath();
    if (radius <= 0) {
      this.ctx.rect(x, y, w, h);
      return;
    }
    this.ctx.moveTo(x + radius, y);
    this.ctx.arcTo(x + w, y, x + w, y + h, radius);
    this.ctx.arcTo(x + w, y + h, x, y + h, radius);
    this.ctx.arcTo(x, y + h, x, y, radius);
    this.ctx.arcTo(x, y, x + w, y, radius);
    this.ctx.closePath();
  }

  rectangle(mode: DrawMode, x: number, y: number, w: number, h: number, rx = 0, ry = 0): void {
    this.roundedRectPath(x, y, w, h, rx, ry);
    if (mode === "fill") {
      this.ctx.fill();
    } else {
      this.ctx.stroke();
    }
  }

  circle(mode: DrawMode, x: number, y: number, radius: number): void {
    this.ctx.beginPath();
    this.ctx.arc(x, y, radius, 0, Math.PI * 2);
    if (mode === "fill") {
      this.ctx.fill();
    } else {
      this.ctx.stroke();
    }
  }

  ellipse(mode: DrawMode, x: number, y: number, radiusX: number, radiusY: number): void {
    this.ctx.beginPath();
    this.ctx.ellipse(x, y, Math.abs(radiusX), Math.abs(radiusY), 0, 0, Math.PI * 2);
    if (mode === "fill") {
      this.ctx.fill();
    } else {
      this.ctx.stroke();
    }
  }

  polygon(mode: DrawMode, points: readonly number[]): void {
    if (points.length < 2) {
      return;
    }
    this.ctx.beginPath();
    this.ctx.moveTo(points[0] ?? 0, points[1] ?? 0);
    for (let i = 2; i + 1 < points.length; i += 2) {
      this.ctx.lineTo(points[i] ?? 0, points[i + 1] ?? 0);
    }
    this.ctx.closePath();
    if (mode === "fill") {
      this.ctx.fill();
    } else {
      this.ctx.stroke();
    }
  }

  line(x1: number, y1: number, x2: number, y2: number): void {
    this.ctx.beginPath();
    this.ctx.moveTo(x1, y1);
    this.ctx.lineTo(x2, y2);
    this.ctx.stroke();
  }

  print(text: string, x: number, y: number): void {
    this.ctx.fillText(text, x, y);
  }

  printf(text: string, x: number, y: number, wrapWidth: number, align: TextAlign): void {
    const lines = wrapLines(this.ctx, text, wrapWidth);
    const lineHeight = this.fontSize * 1.25;
    lines.forEach((line, index) => {
      const lineWidth = this.ctx.measureText(line).width;
      let lineX = x;
      if (align === "center") {
        lineX = x + (wrapWidth - lineWidth) / 2;
      } else if (align === "right") {
        lineX = x + wrapWidth - lineWidth;
      }
      this.ctx.fillText(line, lineX, y + index * lineHeight);
    });
  }

  push(): void {
    this.ctx.save();
  }

  pop(): void {
    this.ctx.restore();
  }

  translate(x: number, y: number): void {
    this.ctx.translate(x, y);
  }

  scale(sx: number, sy: number): void {
    this.ctx.scale(sx, sy);
  }

  setFont(kind: FontKind): void {
    this.fontSize = theme.fonts[kind];
    this.ctx.font = `${this.fontSize}px ${FONT_FAMILY}`;
  }
}
