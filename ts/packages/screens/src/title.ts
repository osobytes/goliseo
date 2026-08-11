// The main title/menu screen. See AGENTS.md §9 for the pure/impure seam;
// this screen needs no injected content.

import { focus, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";

export interface TitleScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly focus: string;
}

export type TitleAction = { readonly go: string };

function newState(viewport: { readonly w: number; readonly h: number }): TitleScreenState {
  return { viewport, focus: "play" };
}

function layout(state: TitleScreenState): Layout {
  const widgets: Widget[] = [
    {
      id: "brand",
      kind: "eyebrow",
      text: "INTERGALACTIC 5v5",
      rect: { x: 0, y: 72, w: state.viewport.w, h: 22 },
      data: { align: "center" },
    },
    {
      id: "title",
      kind: "hero_title",
      text: "GOLISEO",
      rect: { x: 0, y: 104, w: state.viewport.w, h: 58 },
      data: { align: "center" },
    },
    {
      id: "tagline",
      kind: "label",
      text: "PICK THE FIVE  •  SET THE SHAPE  •  PLAY THE PLAN",
      rect: { x: 0, y: 174, w: state.viewport.w, h: 24 },
      data: { align: "center", tone: "muted" },
    },
  ];

  const labels: readonly [string, string][] = [
    ["play", "PLAY SHOWCASE"],
    ["combat_prototype", "COMBAT PROTOTYPE"],
    ["help", "HOW TO PLAY"],
    ["settings", "SETTINGS"],
    ["credits", "CREDITS"],
    ["online_lobby", "ONLINE LOBBY (DEV)"],
    ["quit", "QUIT"],
  ];
  labels.forEach(([id, text], i) => {
    widgets.push({
      id,
      kind: "button",
      text,
      focused: state.focus === id,
      rect: { x: 350, y: 210 + i * 46, w: 260, h: 40 },
    });
  });
  return widgets;
}

function update(state: TitleScreenState, event: FocusEvent): readonly [TitleScreenState, TitleAction | undefined] {
  const currentLayout = layout(state);
  const nextFocus = focus.navigate(currentLayout, state.focus, event) ?? state.focus;
  if (event.kind === "action" && event.action === "back") {
    return [{ viewport: state.viewport, focus: nextFocus }, { go: "quit" }];
  }
  const id = focus.activated(currentLayout, nextFocus, event);
  const nextState: TitleScreenState = { viewport: state.viewport, focus: id ?? nextFocus };
  return [nextState, id !== null ? { go: id } : undefined];
}

export const title = { newState, layout, update };

type Widget = Layout[number];
