// The in-match pause menu, including the two-press restart confirmation.
// See AGENTS.md §9 for the pure/impure seam; this screen needs no injected
// content.

import { focus, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";

export interface PauseScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly focus: string;
  readonly confirmRestart: boolean;
}

export type PauseAction = { readonly go: string };

function newState(viewport: { readonly w: number; readonly h: number }): PauseScreenState {
  return { viewport, focus: "resume", confirmRestart: false };
}

function layout(state: PauseScreenState): Layout {
  const widgets: Widget[] = [
    {
      id: "title",
      kind: "title",
      text: "MATCH PAUSED",
      rect: { x: 0, y: 92, w: state.viewport.w, h: 40 },
      data: { align: "center" },
    },
  ];
  const labels: readonly [string, string][] = [
    ["resume", "RESUME"],
    ["controls", "CONTROLS"],
    ["settings", "SETTINGS"],
    ["restart", state.confirmRestart ? "CONFIRM RESTART" : "RESTART MATCH"],
    ["main_menu", "MAIN MENU"],
  ];
  labels.forEach(([id, text], i) => {
    widgets.push({
      id,
      kind: "button",
      text,
      focused: state.focus === id,
      rect: { x: 350, y: 170 + i * 56, w: 260, h: 44 },
    });
  });
  return widgets;
}

function update(state: PauseScreenState, event: FocusEvent): readonly [PauseScreenState, PauseAction | undefined] {
  const currentLayout = layout(state);
  const nextFocus = focus.navigate(currentLayout, state.focus, event) ?? state.focus;
  const confirmation = state.confirmRestart && nextFocus === "restart";
  if (event.kind === "action" && (event.action === "back" || event.action === "pause")) {
    return [{ viewport: state.viewport, focus: nextFocus, confirmRestart: false }, { go: "resume" }];
  }
  const id = focus.activated(currentLayout, nextFocus, event);
  let next: PauseScreenState = { viewport: state.viewport, focus: id ?? nextFocus, confirmRestart: confirmation };
  if (id === "restart" && !state.confirmRestart) {
    next = { ...next, confirmRestart: true };
    return [next, undefined];
  } else if (id === "restart") {
    next = { ...next, confirmRestart: false };
    return [next, { go: "restart" }];
  } else if (id !== null) {
    next = { ...next, confirmRestart: false };
    return [next, { go: id }];
  }
  return [next, undefined];
}

export const pause = { newState, layout, update };

type Widget = Layout[number];
