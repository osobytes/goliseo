// The multiplayer front door: host or guest, decided before the lobby opens.
//
// There was no front door. The title screen jumped straight at a developer
// lobby that threw before it drew, and the lobby itself had to be both the
// role picker and the seating chart — which is what made it read as a control
// panel rather than a screen.
//
// Host and Join need genuinely different next screens, so the decision moves
// here. This is also the one screen that has to say out loud what kind of
// connection this is: peer-to-peer, no server, no matchmaking. Players will
// otherwise wait for a queue that does not exist.
//
// `MODES` and `SessionMatchMode` come from `lobby_model.ts` in this same
// package, so the mode list is the model's, not a second copy.

import { focus, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import { DEFAULT_MODE, MODES, type SessionMatchMode } from "./lobby_model.ts";

export interface MultiplayerScreenContext {
  readonly mode?: SessionMatchMode;
}

export interface MultiplayerScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly mode: SessionMatchMode;
  readonly focus: string;
}

export type MultiplayerAction =
  | { readonly go: "title" }
  | { readonly go: "lobby"; readonly role: "host"; readonly mode: SessionMatchMode }
  | { readonly go: "lobby"; readonly role: "guest" };

const PANEL_Y = 196;
const PANEL_H = 118;
const PANEL_W = 280;
const HOST_X = 170;
const JOIN_X = 510;
const MODE_Y = 324;
const MODE_W = 88;
const MODE_STEP = 96;

function newState(
  viewport: { readonly w: number; readonly h: number },
  context?: MultiplayerScreenContext,
): MultiplayerScreenState {
  return { viewport, mode: context?.mode ?? DEFAULT_MODE, focus: "host" };
}

function layout(state: MultiplayerScreenState): Layout {
  const widgets: Widget[] = [
    {
      id: "brand",
      kind: "eyebrow",
      text: "PEER TO PEER  •  NO SERVER",
      rect: { x: 0, y: 96, w: state.viewport.w, h: 22 },
      data: { align: "center", focusable: false },
    },
    {
      id: "title",
      kind: "title",
      text: "PLAY SOMEONE",
      rect: { x: 0, y: 126, w: state.viewport.w, h: 40 },
      data: { align: "center", focusable: false },
    },
    {
      id: "host",
      kind: "card",
      text: "OPEN A LOBBY\nYou pick the size, then send an invite. Empty seats fill with bots.",
      focused: state.focus === "host",
      rect: { x: HOST_X, y: PANEL_Y, w: PANEL_W, h: PANEL_H },
      data: { align: "left" },
    },
    {
      id: "join",
      kind: "card",
      text: "USE AN INVITE\nPaste the code a friend sent you. You take a seat in their lobby.",
      focused: state.focus === "join",
      rect: { x: JOIN_X, y: PANEL_Y, w: PANEL_W, h: PANEL_H },
      data: { align: "left" },
    },
  ];

  MODES.forEach((mode, i) => {
    widgets.push({
      id: `mode_${mode}`,
      kind: "button",
      text: mode.toUpperCase(),
      selected: state.mode === mode,
      focused: state.focus === `mode_${mode}`,
      rect: { x: HOST_X + i * MODE_STEP, y: MODE_Y, w: MODE_W, h: 30 },
    });
  });

  widgets.push({
    id: "size_hint",
    kind: "label",
    text: "LOBBY SIZE",
    rect: { x: HOST_X, y: MODE_Y + 34, w: PANEL_W, h: 18 },
    data: { align: "left", tone: "muted", focusable: false },
  });
  widgets.push({
    id: "note",
    kind: "label",
    text: "Matches connect directly between the two browsers. Nothing is stored, and the session ends when you both leave.",
    rect: { x: JOIN_X, y: MODE_Y, w: PANEL_W, h: 56 },
    data: { align: "left", tone: "muted", focusable: false },
  });
  widgets.push({
    id: "back",
    kind: "button",
    text: "BACK",
    focused: state.focus === "back",
    rect: { x: 62, y: 464, w: 140, h: 40 },
  });
  widgets.push({
    id: "hint",
    kind: "label",
    text: "WEBRTC  •  ROLLBACK NETCODE",
    rect: { x: 500, y: 474, w: 398, h: 20 },
    data: { align: "right", tone: "muted", focusable: false },
  });
  return widgets;
}

function update(
  state: MultiplayerScreenState,
  event: FocusEvent,
): readonly [MultiplayerScreenState, MultiplayerAction | undefined] {
  const currentLayout = layout(state);
  let next: MultiplayerScreenState = {
    ...state,
    focus: focus.navigate(currentLayout, state.focus, event) ?? state.focus,
  };
  if (event.kind === "action" && event.action === "back") {
    return [next, { go: "title" }];
  }
  const id = focus.activated(currentLayout, next.focus, event);
  if (id === null) {
    return [next, undefined];
  }
  next = { ...next, focus: id };

  if (id === "back") {
    return [next, { go: "title" }];
  }
  if (id === "host") {
    return [next, { go: "lobby", role: "host", mode: next.mode }];
  }
  if (id === "join") {
    return [next, { go: "lobby", role: "guest" }];
  }
  const mode = /^mode_(.+)$/.exec(id)?.[1];
  if (mode !== undefined && MODES.includes(mode as SessionMatchMode)) {
    return [{ ...next, mode: mode as SessionMatchMode }, undefined];
  }
  return [next, undefined];
}

export const multiplayer = { newState, layout, update };

type Widget = Layout[number];
