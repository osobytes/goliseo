// Pure hit-testing over a layout. A layout is just an ordered list of
// widgets; later widgets are "on top", so hit-testing scans back-to-front.
// This is the core of AGENTS.md §9's pure/impure seam: no DOM, no three.js,
// no globals, so every screen spec can run headless.

import type { Layout, Rect, Widget } from "./types.ts";

function inside(r: Rect, x: number, y: number): boolean {
  return x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h;
}

/** Id of the topmost widget containing (x, y), or `null`. */
function at(layout: Layout, x: number, y: number): string | null {
  for (let i = layout.length - 1; i >= 0; i--) {
    const w = layout[i];
    if (w?.rect && inside(w.rect, x, y)) {
      return w.id;
    }
  }
  return null;
}

function find(layout: Layout, id: string): Widget | null {
  for (const w of layout) {
    if (w.id === id) {
      return w;
    }
  }
  return null;
}

export const hit = { at, find };
