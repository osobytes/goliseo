// The canvas state this backend depends on, and the way it loses it.
//
// `textBaseline` was set once in the constructor. That does not survive:
// assigning `canvas.width`/`height` resets the whole 2D context to its
// defaults, and `browser_main.ts`'s `resize()` does exactly that at boot and
// on every window resize. So every string rendered with its baseline — not its
// top — at `y`, about one ascent too high in its box, and `draw.ts`'s centring
// arithmetic was correct against a contract the renderer was not honouring.
//
// No layout spec could see this: the pure layer computes positions and never
// learns what the canvas did with them. The fake context below is the seam
// where it becomes visible.

import { describe, expect, it } from "vitest";
import { CanvasGraphicsBackend } from "./canvas_graphics_backend.ts";

interface FillTextCall {
  readonly text: string;
  readonly y: number;
  /** What `textBaseline` was at the moment this glyph run was drawn. */
  readonly baseline: string;
}

/**
 * The slice of `CanvasRenderingContext2D` this backend touches, plus a
 * `resetToDefaults()` that reproduces what assigning `canvas.width` does.
 */
function fakeContext(): {
  ctx: CanvasRenderingContext2D;
  calls: FillTextCall[];
  resetToDefaults: () => void;
} {
  const calls: FillTextCall[] = [];
  const noop = (): void => {};
  const state = { textBaseline: "alphabetic", font: "10px sans-serif" };
  const ctx = {
    canvas: { width: 1280, height: 720 },
    get textBaseline() {
      return state.textBaseline;
    },
    set textBaseline(value: string) {
      state.textBaseline = value;
    },
    get font() {
      return state.font;
    },
    set font(value: string) {
      state.font = value;
    },
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    measureText: (text: string) => ({
      width: text.length * 7,
      fontBoundingBoxAscent: 13,
      fontBoundingBoxDescent: 3,
    }),
    fillText: (text: string, _x: number, y: number) => {
      calls.push({ text, y, baseline: state.textBaseline });
    },
    beginPath: noop,
    closePath: noop,
    moveTo: noop,
    lineTo: noop,
    rect: noop,
    arc: noop,
    arcTo: noop,
    ellipse: noop,
    fill: noop,
    stroke: noop,
    save: noop,
    restore: noop,
    translate: noop,
    scale: noop,
    clearRect: noop,
  } as unknown as CanvasRenderingContext2D;

  return {
    ctx,
    calls,
    // Assigning canvas.width/height resets every context property.
    resetToDefaults: () => {
      state.textBaseline = "alphabetic";
      state.font = "10px sans-serif";
    },
  };
}

describe("CanvasGraphicsBackend text baseline", () => {
  it("draws top-baselined even after the canvas resize that resets the context", () => {
    const { ctx, calls, resetToDefaults } = fakeContext();
    const backend = new CanvasGraphicsBackend(ctx);

    // `browser_main.ts`'s resize(), which runs at boot and on every window
    // resize, long after this backend was constructed.
    resetToDefaults();

    backend.setFont("body");
    backend.printf("KICK OFF", 100, 200, 300, "center");

    expect(calls.length).toBe(1);
    expect(
      calls[0]?.baseline,
      "text drew with the default alphabetic baseline, so `y` was a baseline and not the top of the block",
    ).toBe("top");
  });

  it("top-baselines `print` too, not only `printf`", () => {
    const { ctx, calls, resetToDefaults } = fakeContext();
    const backend = new CanvasGraphicsBackend(ctx);
    resetToDefaults();

    backend.print("PAUSED", 10, 20);

    expect(calls[0]?.baseline).toBe("top");
  });

  it("keeps every line of a wrapped block on the same baseline setting", () => {
    const { ctx, calls, resetToDefaults } = fakeContext();
    const backend = new CanvasGraphicsBackend(ctx);
    resetToDefaults();

    backend.setFont("body");
    // 7px per char in the fake metrics, so this wraps at 140px.
    backend.printf("one two three four five six seven", 0, 0, 140, "left");

    expect(calls.length).toBeGreaterThan(1);
    for (const call of calls) {
      expect(call.baseline, `"${call.text}" drew on the wrong baseline`).toBe("top");
    }
  });

  it("advances each wrapped line by the line height, from the block top", () => {
    const { ctx, calls, resetToDefaults } = fakeContext();
    const backend = new CanvasGraphicsBackend(ctx);
    resetToDefaults();

    backend.setFont("body");
    backend.printf("one two three four five six seven", 0, 50, 140, "left");

    const lineHeight = 13 * 1.25;
    calls.forEach((call, index) => {
      expect(call.y).toBeCloseTo(50 + index * lineHeight, 5);
    });
  });

  it("reports a wrapped block's height from the top of the first line to the bottom of the last", () => {
    const { ctx } = fakeContext();
    const backend = new CanvasGraphicsBackend(ctx);
    backend.setFont("body");

    const single = backend.measureText("SHORT", 300);
    expect(single.lines).toBe(1);
    // One glyph box: ascent + descent.
    expect(single.height).toBeCloseTo(16, 5);

    const wrapped = backend.measureText("one two three four five six seven", 140);
    expect(wrapped.lines).toBeGreaterThan(1);
    expect(wrapped.height).toBeCloseTo((wrapped.lines - 1) * 13 * 1.25 + 16, 5);
  });
});
