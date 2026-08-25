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
//
// # Inline code entry (#610)
//
// A friend's room code used to need a whole screen change to type: click
// "USE AN INVITE", land on the lobby's own guest composer, then type. #603
// already made that landing focused with no further click; this adds the
// other half -- an inline six-character composer right here, so typing a
// code never needs the "USE AN INVITE" click at all. It reuses
// `room_code_entry.ts`'s pure editing primitives (the exact ones the
// lobby's own composer uses) rather than a second copy, and typing an
// alphabet character ANYWHERE on this screen -- not just while the
// composer widget itself is focused -- redirects focus to it and feeds the
// keystroke through, so a player can simply start typing.
//
// A completed code is carried on the `"guest"` action as `code`, and
// `app.ts`'s routing forwards it as `OnlineLobbyOptions.presetRoomCode` --
// the SAME pre-fill-and-auto-submit path #598's join links already use, not
// a parallel one, so a bad code fails exactly the way a mistyped one in the
// lobby's own composer already does.

import { focus, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import { DEFAULT_MODE, MODES, type SessionMatchMode } from "./lobby_model.ts";
import {
  newRoomCodeEntry,
  roomCodeCursor,
  roomCodeCycle,
  roomCodeDisplay,
  roomCodeKey,
  roomCodeText,
  ROOM_CODE_ALPHABET,
  type RoomCodeEntry,
} from "./room_code_entry.ts";

/** The inline composer's own widget id -- deliberately distinct from
 * `lobby.ts`'s `ROOM_CODE_ENTRY_WIDGET`: different screen, different
 * layout, no shared identity needed beyond the editing primitives above. */
const CODE_ENTRY_WIDGET = "code_entry";

export interface MultiplayerScreenContext {
  readonly mode?: SessionMatchMode;
}

export interface MultiplayerScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly mode: SessionMatchMode;
  readonly focus: string;
  /** The inline room-code composer -- always present (this screen has no
   * "activate the composer" step to gate it behind, unlike the lobby's own
   * `room_entry`, which only exists once "JOIN WITH A ROOM CODE" is
   * chosen). */
  readonly code_entry: RoomCodeEntry;
}

// The lobby route no longer carries a preset manual role (`{go:"lobby",
// role:"host"|"guest"}`): that shape dispatched `lobby_model.ts`'s "role"
// command straight in `OnlineLobby`'s constructor, which locked in the
// manual-signaling role before the lobby ever rendered, and `lobby.ts`'s
// room-code buttons only render while no role is chosen -- so the
// room-code path this screen exists to reach (#552) was unreachable from
// here (#597). `intent` instead names which `room_pick` path the lobby
// should run immediately: the host sees a room code with no further
// clicks, and a guest lands on the code composer, already focused. Mode
// stays the host's own front-door choice, applied once the lobby's model
// legally accepts a "mode" command for a room-hosting attempt (see
// `online_lobby.ts`'s constructor).
export type MultiplayerAction =
  | { readonly go: "title" }
  | { readonly go: "lobby"; readonly intent: "host"; readonly mode: SessionMatchMode }
  | { readonly go: "lobby"; readonly intent: "guest" }
  // The inline composer's own completed code (#610) -- a friend's code
  // reaches the lobby the same way #598's join links already do, forwarded
  // as `OnlineLobbyOptions.presetRoomCode` by `app.ts`'s routing (this
  // module's header).
  | { readonly go: "lobby"; readonly intent: "guest"; readonly code: string };

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
  return {
    viewport,
    mode: context?.mode ?? DEFAULT_MODE,
    focus: "host",
    code_entry: newRoomCodeEntry(),
  };
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
  // The inline code composer (#610): typing a friend's code works right
  // here, no "USE AN INVITE" click needed first -- this module's header.
  widgets.push({
    id: "code_entry_hint",
    kind: "label",
    text: "GOT A CODE? TYPE IT:",
    rect: { x: JOIN_X, y: MODE_Y + 60, w: PANEL_W, h: 16 },
    data: { align: "left", tone: "muted", focusable: false },
  });
  widgets.push({
    id: CODE_ENTRY_WIDGET,
    kind: "button",
    text: roomCodeDisplay(state.code_entry),
    focused: state.focus === CODE_ENTRY_WIDGET,
    rect: { x: JOIN_X, y: MODE_Y + 78, w: PANEL_W, h: 36 },
    data: { align: "center" },
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
  // Typing a room-code character anywhere on this screen -- not only while
  // the composer itself is focused -- redirects focus to it and feeds the
  // keystroke through, so "just start typing" genuinely needs no prior
  // click (this module's header). A key outside the closed alphabet (and
  // every non-"key" event) falls through to the normal paths below exactly
  // as before.
  if (
    state.focus !== CODE_ENTRY_WIDGET &&
    event.kind === "key" &&
    event.pressed !== false &&
    event.key.length === 1 &&
    ROOM_CODE_ALPHABET.includes(event.key.toUpperCase())
  ) {
    return [
      {
        ...state,
        focus: CODE_ENTRY_WIDGET,
        code_entry: roomCodeKey(state.code_entry, event.key),
      },
      undefined,
    ];
  }
  // Once focused, the composer captures typing/cycling directly -- the
  // same up/down/left/right-means-cycle/move contract `lobby.ts`'s own
  // room-code composer uses (`ROOM_CODE_ENTRY_WIDGET`'s doc there).
  // Confirm/back/click fall through to the normal paths below.
  if (state.focus === CODE_ENTRY_WIDGET) {
    if (event.kind === "key" && event.pressed !== false) {
      return [{ ...state, code_entry: roomCodeKey(state.code_entry, event.key) }, undefined];
    }
    if (event.kind === "action") {
      if (event.action === "up") {
        return [{ ...state, code_entry: roomCodeCycle(state.code_entry, 1) }, undefined];
      }
      if (event.action === "down") {
        return [{ ...state, code_entry: roomCodeCycle(state.code_entry, -1) }, undefined];
      }
      if (event.action === "left") {
        return [{ ...state, code_entry: roomCodeCursor(state.code_entry, -1) }, undefined];
      }
      if (event.action === "right") {
        return [{ ...state, code_entry: roomCodeCursor(state.code_entry, 1) }, undefined];
      }
    }
  }
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
    return [next, { go: "lobby", intent: "host", mode: next.mode }];
  }
  if (id === "join") {
    return [next, { go: "lobby", intent: "guest" }];
  }
  if (id === CODE_ENTRY_WIDGET) {
    const code = roomCodeText(next.code_entry);
    // An incomplete code (activated by click/confirm before all six slots
    // are filled) simply stays on the composer -- there is nothing to
    // connect to yet, and the lobby's own composer already owns the
    // "explain why this failed" copy for a code that IS complete but wrong.
    return code !== undefined ? [next, { go: "lobby", intent: "guest", code }] : [next, undefined];
  }
  const mode = /^mode_(.+)$/.exec(id)?.[1];
  if (mode !== undefined && MODES.includes(mode as SessionMatchMode)) {
    return [{ ...next, mode: mode as SessionMatchMode }, undefined];
  }
  return [next, undefined];
}

export const multiplayer = { newState, layout, update };

type Widget = Layout[number];
