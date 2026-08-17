// draw.ts's one testable property: where it puts text.
//
// Everything else in this module is colour and geometry that only a real
// display can judge, but vertical placement is arithmetic over
// `GraphicsBackend.measureText`, and it was wrong for a long time (#408): a
// short button centred its text with `rect.y + rect.h / 2 - 7`, where the `7`
// was half a line height of a font this build does not ship. Cards used a flat
// `rect.y + 10` and labels a bare `rect.y`, so neither knew how many lines the
// text would wrap to. A recording backend with known metrics pins all three.

import { describe, expect, it } from "vitest";
import { draw } from "./draw.ts";
import type { GraphicsBackend, TextBlockMetrics } from "./graphics_backend.ts";
import type { Layout, TextAlign } from "./types.ts";

const VIEWPORT = { w: 960, h: 540 };
/** The double's font: one line is 13 tall, lines advance 16.25, ~7px per char. */
const GLYPH_H = 13;
const LINE_H = 16.25;
const CHAR_W = 7;

interface PrintfCall {
  readonly text: string;
  readonly x: number;
  readonly y: number;
  readonly wrapWidth: number;
  readonly align: TextAlign;
}

function recordingBackend(): { backend: GraphicsBackend; calls: PrintfCall[] } {
  const calls: PrintfCall[] = [];
  const noop = (): void => {};
  const measureText = (text: string, wrapWidth: number): TextBlockMetrics => {
    const lines = wrapWidth > 0 ? Math.max(1, Math.ceil((text.length * CHAR_W) / wrapWidth)) : 1;
    return { lines, lineHeight: LINE_H, height: (lines - 1) * LINE_H + GLYPH_H };
  };
  const backend: GraphicsBackend = {
    getDimensions: () => ({ width: VIEWPORT.w, height: VIEWPORT.h }),
    setColor: noop,
    setLineWidth: noop,
    rectangle: noop,
    circle: noop,
    ellipse: noop,
    polygon: noop,
    line: noop,
    measureText,
    print: noop,
    printf: (text, x, y, wrapWidth, align) => {
      calls.push({ text, x, y, wrapWidth, align });
    },
    push: noop,
    pop: noop,
    translate: noop,
    scale: noop,
    setFont: noop,
  };
  return { backend, calls };
}

function drawnAs(layout: Layout, text: string): PrintfCall {
  const { backend, calls } = recordingBackend();
  draw.layout(backend, layout, VIEWPORT);
  const call = calls.find((c) => c.text === text);
  expect(call, `"${text}" was never drawn`).toBeDefined();
  if (!call) {
    throw new Error("unreachable");
  }
  return call;
}

/** Where a block of `lines` lines lands when centred in a rect of height `h`. */
function centredTop(y: number, h: number, lines: number): number {
  return y + (h - ((lines - 1) * LINE_H + GLYPH_H)) / 2;
}

describe("draw.layout: vertical text placement", () => {
  it("centres a single line of button text on measured metrics, not a magic offset", () => {
    const rect = { x: 100, y: 200, w: 300, h: 42 };
    const call = drawnAs([{ id: "b", kind: "button", text: "KICK OFF", rect }], "KICK OFF");

    expect(call.y).toBeCloseTo(centredTop(rect.y, rect.h, 1), 5);
    // The rule this replaced would have put it here, ~1.6px high.
    expect(call.y).not.toBeCloseTo(rect.y + rect.h / 2 - 7, 5);
  });

  it("centres a two-line card, rather than pinning it to a fixed top padding", () => {
    // 60 chars at 7px in a 300-wide card (14 left inset, 12 right) wraps to 2.
    const text = "Zyro Vex FORWARD PAC 8 STR 6 TEC 7 STA 5 MEN 2 and more here";
    const rect = { x: 62, y: 88, w: 300, h: 44 };
    const call = drawnAs([{ id: "c", kind: "card", text, rect }], text);

    expect(call.wrapWidth).toBe(rect.w - 14 - 12);
    const lines = Math.ceil((text.length * CHAR_W) / call.wrapWidth);
    expect(lines, "fixture should wrap to more than one line").toBeGreaterThan(1);
    expect(call.y).toBeCloseTo(centredTop(rect.y, rect.h, lines), 5);
    expect(call.y).not.toBeCloseTo(rect.y + 10, 5);
  });

  it("centres a label in its box instead of pinning it to the top edge", () => {
    const rect = { x: 0, y: 470, w: 960, h: 20 };
    const call = drawnAs(
      [{ id: "l", kind: "label", text: "PRESS ENTER", rect, data: { align: "center" } }],
      "PRESS ENTER",
    );

    expect(call.y).toBeCloseTo(centredTop(rect.y, rect.h, 1), 5);
    expect(call.align).toBe("center");
  });

  it("keeps text inside its box for every widget kind that carries text", () => {
    const layout: Layout = [
      { id: "button", kind: "button", text: "SETTINGS", rect: { x: 10, y: 10, w: 200, h: 40 } },
      { id: "card", kind: "card", text: "Ozzo KEEPER", rect: { x: 10, y: 60, w: 300, h: 44 } },
      { id: "label", kind: "label", text: "WAITING", rect: { x: 10, y: 120, w: 300, h: 20 } },
      { id: "title", kind: "title", text: "LOBBY", rect: { x: 10, y: 150, w: 300, h: 32 } },
      { id: "eyebrow", kind: "eyebrow", text: "FULL TIME", rect: { x: 10, y: 190, w: 300, h: 18 } },
    ];
    const { backend, calls } = recordingBackend();
    draw.layout(backend, layout, VIEWPORT);

    for (const widget of layout) {
      const rect = widget.rect;
      const call = calls.find((c) => c.text === widget.text);
      expect(call, `${widget.id} was never drawn`).toBeDefined();
      if (!call || !rect) {
        continue;
      }
      const lines = Math.max(1, Math.ceil((call.text.length * CHAR_W) / call.wrapWidth));
      const bottom = call.y + (lines - 1) * LINE_H + GLYPH_H;
      expect(call.y, `${widget.id} starts above its box`).toBeGreaterThanOrEqual(rect.y);
      expect(bottom, `${widget.id} runs past the bottom of its box`).toBeLessThanOrEqual(
        rect.y + rect.h,
      );
    }
  });

  it("never lifts text above its box when the text is taller than the box", () => {
    // An overflowing string must clip downward, not float up out of the panel
    // and collide with whatever sits above it.
    const rect = { x: 0, y: 100, w: 120, h: 16 };
    const text = "a very long overflowing string that cannot possibly fit in this small box";
    const call = drawnAs([{ id: "o", kind: "label", text, rect }], text);

    expect(call.y).toBeGreaterThanOrEqual(rect.y);
  });
});
