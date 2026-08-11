// Keyboard/gamepad focus navigation over a layout, built entirely on
// hit.ts's pure primitives.

import { hit } from "./hit.ts";
import type { FocusEvent, Layout, Widget } from "./types.ts";

function focusable(widget: Widget): boolean {
  if (widget.data?.focusable === false) {
    return false;
  }
  return widget.kind === "button" || widget.kind === "card";
}

function clickable(widget: Widget): boolean {
  if (widget.data && (widget.data.focusable === false || widget.data.disabled === true)) {
    return false;
  }
  return focusable(widget) || widget.kind === "formation_preview";
}

function ids(layout: Layout): readonly string[] {
  const result: string[] = [];
  for (const widget of layout) {
    if (focusable(widget) && widget.data?.disabled !== true) {
      result.push(widget.id);
    }
  }
  return result;
}

function ensure(layout: Layout, current: string | null): string | null {
  const list = ids(layout);
  for (const id of list) {
    if (id === current) {
      return current;
    }
  }
  return list[0] ?? null;
}

function move(layout: Layout, current: string | null, delta: number): string | null {
  const list = ids(layout);
  const n = list.length;
  if (n === 0) {
    return null;
  }
  let index = 0;
  for (let i = 0; i < n; i++) {
    if (list[i] === current) {
      index = i;
      break;
    }
  }
  const next = (((index + delta) % n) + n) % n;
  return list[next] ?? null;
}

function activated(layout: Layout, current: string | null, event: FocusEvent): string | null {
  if (event.kind === "click") {
    const id = hit.at(layout, event.x, event.y);
    const widget = id !== null ? hit.find(layout, id) : null;
    return widget && clickable(widget) ? id : null;
  } else if (event.kind === "action" && event.action === "confirm") {
    return ensure(layout, current);
  }
  return null;
}

function navigate(layout: Layout, current: string | null, event: FocusEvent): string | null {
  if (event.kind !== "action") {
    return ensure(layout, current);
  }
  if (event.action === "up" || event.action === "left") {
    return move(layout, current, -1);
  } else if (event.action === "down" || event.action === "right") {
    return move(layout, current, 1);
  }
  return ensure(layout, current);
}

export const focus = { ids, ensure, move, activated, navigate };
