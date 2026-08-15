// The pure screen for the manual-connect online lobby.
//
// `layout`, `hit`, and `update` touch no graphics, no transport, and no
// clock, so the whole lobby -- role choice, manual offer/answer exchange,
// mode choice, ownership, readiness, countdown, and every failure -- runs
// headlessly with zero display.
//
// One deviation from the simplest screens is deliberate: a transition may
// need side effects (open a peer, send a wire, write the clipboard). Those
// leave as data on `state.effects`, an ordered list the owning screen
// (`online_lobby.ts`) drains after each update. The transition itself stays
// pure.
//
// `lobby_model.ts`'s `LobbyModelPorts` (coordinator/protocol/... -- see its
// header) is threaded through embedded on `LobbyScreenState`, the same
// pattern `squad.ts` uses for `SquadContentData`.

import { focus, type Layout, type Widget } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import {
  MODES,
  command as lobbyCommand,
  newLobbyModel,
  view as lobbyView,
  type LobbyCommand,
  type LobbyEffect,
  type LobbyModel,
  type LobbyModelOptions,
  type LobbyModelPorts,
  type LobbyPreferenceView,
  type LobbySeatView,
  type LobbySignalRecord,
  type LobbySlotView,
  type LobbyView,
  type RoomCodeEntry,
  type SessionMatchMode,
} from "./lobby_model.ts";

export type { LobbyEffect } from "./lobby_model.ts";

/** The single focusable widget in the room-code composer sub-view -- see
 * `update()`'s own special-casing right below `layout()`. */
const ROOM_CODE_ENTRY_WIDGET = "room_code_slots";

export interface LobbyScreenContext {
  readonly model?: LobbyModel;
  readonly options?: LobbyModelOptions;
}

export interface LobbyScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly ports: LobbyModelPorts;
  readonly model: LobbyModel;
  readonly focus: string;
  /** Produced by the last update; drained by the owner. */
  readonly effects: readonly LobbyEffect[];
}

export type LobbyAction =
  { readonly go: "online_match"; readonly freeze: unknown } | { readonly go: "main_menu" };

const LEFT_X = 24;
const LEFT_W = 248;
const ROSTER_X = 300;
const ROSTER_W = 360;
const RIGHT_X = 676;
const RIGHT_W = 260;
const ROW_TOP = 68;
const ROW_STEP = 34;
const ROW_H = 30;

export function newState(
  viewport: { readonly w: number; readonly h: number },
  ports: LobbyModelPorts,
  context?: LobbyScreenContext,
): LobbyScreenState {
  return {
    viewport,
    ports,
    model: context?.model ?? newLobbyModel(ports, context?.options),
    focus: "role_host",
    effects: [],
  };
}

function slotText(slot: LobbySlotView): string {
  const owner = slot.owner ?? "unassigned";
  let driver: string;
  if (slot.driver === "pending") {
    driver = "pending";
  } else if (slot.owner_kind === "bot") {
    driver = "AI FILL";
  } else if (slot.driver === "human") {
    driver = "LIVE";
  } else {
    driver = "AI (OWNED)";
  }
  return `${slot.slot.toUpperCase()}  ${slot.player_id.toUpperCase()}  ->  ${owner.toUpperCase()}  ${driver}`;
}

function seatText(seat: LobbySeatView): string {
  const slots = seat.slots.length > 0 ? seat.slots.join(" ") : "unassigned";
  return `${seat.index}  ${seat.peer_id.toUpperCase()}${seat.is_local ? " (YOU)" : ""}  ${slots.toUpperCase()}  ${
    seat.ready ? "READY" : "-"
  }`;
}

function signalText(record: LobbySignalRecord | undefined, fallback: string): string {
  if (!record) {
    return fallback;
  }
  // Only the direction, size, and digest are ever rendered. The blob itself
  // is handed to the clipboard and dropped; it is never held for display.
  return `${record.direction.toUpperCase()} ${record.peer_id.toUpperCase()}  ${record.bytes} BYTES  #${record.fingerprint}`;
}

function identityText(view: LobbyView): string {
  return view.identity.map((row) => `${row.label}  ${row.value.toUpperCase()}`).join("\n");
}

interface LeftOptions {
  readonly kind?: string;
  readonly selected?: boolean;
  readonly x?: number;
  readonly w?: number;
  readonly tone?: "muted";
  readonly disabled?: boolean;
  readonly focusable?: boolean;
  readonly inline?: boolean;
}

export function layout(state: LobbyScreenState): Layout {
  const view = lobbyView(state.ports, state.model);
  const widgets: Widget[] = [
    {
      id: "title",
      kind: "title",
      text: "ONLINE LOBBY",
      rect: { x: 0, y: 18, w: state.viewport.w, h: 28 },
      data: { align: "center" },
    },
  ];

  let y = ROW_TOP;
  const left = (id: string, text: string, height: number, options?: LeftOptions): void => {
    widgets.push({
      id,
      kind: options?.kind ?? "button",
      text,
      ...(options?.selected !== undefined ? { selected: options.selected } : {}),
      focused: state.focus === id,
      rect: { x: options?.x ?? LEFT_X, y, w: options?.w ?? LEFT_W, h: height },
      data: {
        align: "left",
        ...(options?.tone !== undefined ? { tone: options.tone } : {}),
        ...(options?.disabled !== undefined ? { disabled: options.disabled } : {}),
        ...(options?.focusable !== undefined ? { focusable: options.focusable } : {}),
      },
    });
    if (!options?.inline) {
      y += height + 6;
    }
  };

  if (view.room_entry) {
    const entry: RoomCodeEntry = view.room_entry;
    const text = entry.chars
      .map((ch, index) => (index === entry.cursor ? `[${ch || "_"}]` : ch || "_"))
      .join(" ");
    left(ROOM_CODE_ENTRY_WIDGET, text, 40);
    left(
      "room_hint",
      "TYPE THE CODE, OR CYCLE A CHARACTER WITH UP/DOWN AND MOVE WITH LEFT/RIGHT. CONFIRM TO JOIN.",
      34,
      { kind: "label", tone: "muted", focusable: false },
    );
  } else if (!view.role) {
    // Disabled for the ENTIRE window a room-code attempt is in flight or
    // established but not yet a chosen role (`view.room_status ===
    // "connecting"` covers the first; `view.room_active` alone covers a
    // `"connected"` state that has not yet reached `chooseRole` -- both
    // collapse once `room_created`/`room_joined` lands, since this whole
    // branch stops rendering the moment `view.role` is set). Activating a
    // manual role or a second room-code attempt during this window used to
    // wedge the lobby: the late `room_created`/`room_joined` would no-op
    // against an already-chosen role but still leave `room_active` stuck
    // `true` forever, with both signaling paths dead (round-2 council
    // review, blocking finding 2). `chooseRole`'s and `roomPick`'s own
    // call-site guards are the belt to this layout's braces.
    const roomBusy = view.room_status === "connecting" || view.room_active;
    left("role_host", "HOST A SESSION", 40, { disabled: roomBusy });
    left("identity", `JOIN AS  ${view.peer_id.toUpperCase()}`, ROW_H);
    left("role_guest", "JOIN WITH AN OFFER", 40, { disabled: roomBusy });
    left("room_code_host", "HOST WITH A ROOM CODE", 40, { disabled: roomBusy });
    left("room_code_join", "JOIN WITH A ROOM CODE", 40, { disabled: roomBusy });
    left(
      "hint",
      "MANUAL SIGNALING: BLOBS ARE EXCHANGED BY HAND. A ROOM CODE CONNECTS AUTOMATICALLY.",
      34,
      { kind: "label", tone: "muted", focusable: false },
    );
  } else if (view.role === "host") {
    const modeW = 76;
    MODES.forEach((mode, index) => {
      left(`mode_${mode}`, mode.toUpperCase(), ROW_H, {
        x: LEFT_X + index * (modeW + 10),
        w: modeW,
        selected: view.mode === mode,
        disabled: view.mode_locked,
        inline: index < MODES.length - 1,
      });
    });
    left(
      "bot_fill",
      view.bot_fill ? "AI FILLS EMPTY SEATS: ON" : "AI FILLS EMPTY SEATS: OFF",
      ROW_H,
      {
        selected: view.bot_fill,
        disabled: view.mode_locked,
      },
    );
    left("invite", "INVITE A PEER", ROW_H, { disabled: !view.can_invite });
    left("lock", "LOCK CONFIGURATION", ROW_H, { disabled: !view.can_lock });
    if (view.room_code) {
      left("room_code_display", `ROOM CODE  ${view.room_code}`, ROW_H, {
        kind: "label",
        focusable: false,
      });
    }
  } else {
    left("mode_label", `MODE  ${view.mode_known ? view.mode.toUpperCase() : "PENDING"}`, 22, {
      kind: "label",
      tone: "muted",
      focusable: false,
    });
    if (view.room_active) {
      left("room_code_active", "CONNECTED VIA ROOM CODE", 20, {
        kind: "label",
        tone: "muted",
        focusable: false,
      });
    }
  }

  if (view.role && !view.room_active) {
    const half = (LEFT_W - 10) / 2;
    left("copy_signal", "COPY SIGNAL", ROW_H, {
      w: half,
      inline: true,
      disabled: !view.has_outgoing,
    });
    left("paste_signal", "PASTE SIGNAL", ROW_H, { x: LEFT_X + half + 10, w: half });
    left("signal_out", signalText(view.exported, "NO LOCAL SIGNAL"), 20, {
      kind: "label",
      tone: "muted",
      focusable: false,
    });
    left("signal_in", signalText(view.imported, "NO IMPORTED SIGNAL"), 20, {
      kind: "label",
      tone: "muted",
      focusable: false,
    });
  }
  if (view.role) {
    left(
      "peer_count",
      `PEERS  ${view.connected} / ${view.mode_known ? String(view.required) : "?"}`,
      20,
      {
        kind: "label",
        focusable: false,
      },
    );
    if (view.countdown !== undefined) {
      left("countdown", `COUNTDOWN  ${view.countdown} TICKS`, 20, {
        kind: "label",
        focusable: false,
      });
    }
    if (view.started) {
      left("started", "START BOUNDARY REACHED", 20, { kind: "label", focusable: false });
    }
  }

  // Roster: all eight canonical outfield slots, then both protected keepers.
  // A slot the local peer could ask the host for carries its own control. In
  // `1v1` and `4v4` no slot ever does, because there is no pair to choose.
  view.slots.forEach((slot, index) => {
    const rowY = ROW_TOP + index * ROW_STEP;
    widgets.push({
      id: `slot_${slot.slot}`,
      kind: "card",
      text: slotText(slot),
      selected: slot.local_owner,
      rect: { x: ROSTER_X, y: rowY, w: ROSTER_W - 66, h: ROW_H },
      data: { align: "left", focusable: false },
    });
    if (slot.can_prefer) {
      widgets.push({
        id: `prefer_${slot.slot}`,
        kind: "button",
        text: "TAKE",
        focused: state.focus === `prefer_${slot.slot}`,
        rect: { x: ROSTER_X + ROSTER_W - 62, y: rowY, w: 62, h: ROW_H },
        data: { align: "center" },
      });
    }
  });
  if (view.preference) {
    const preference: LobbyPreferenceView = view.preference;
    widgets.push({
      id: "preference",
      kind: "label",
      text: `PAIR ${preference.slots.join(" ").toUpperCase()}  ${preference.text.toUpperCase()}`,
      rect: { x: ROSTER_X, y: ROW_TOP + 8 * ROW_STEP + 2 * 22 + 6, w: ROSTER_W, h: 40 },
      data: { align: "left", tone: "muted", focusable: false },
    });
  }
  view.keepers.forEach((keeper, index) => {
    widgets.push({
      id: `keeper_${keeper.team}`,
      kind: "label",
      text: `KEEPER ${keeper.team.toUpperCase()}  ${keeper.player_id.toUpperCase()}  PROTECTED AI`,
      rect: { x: ROSTER_X, y: ROW_TOP + 8 * ROW_STEP + index * 22, w: ROSTER_W, h: 20 },
      data: { align: "left", tone: "muted", focusable: false },
    });
  });

  // Seats: one row per human, with the ownership swap that repartitions a
  // 2v2 pair (and moves a human between teams) before the freeze.
  view.seats.forEach((seat, index) => {
    const rowY = ROW_TOP + index * ROW_STEP;
    widgets.push({
      id: `seat_${index + 1}`,
      kind: "card",
      text: seatText(seat),
      selected: seat.is_local,
      rect: { x: RIGHT_X, y: rowY, w: RIGHT_W - 82, h: ROW_H },
      data: { align: "left", focusable: false },
    });
    if (view.role === "host" && index < view.seats.length - 1) {
      widgets.push({
        id: `swap_${index + 1}`,
        kind: "button",
        text: "SWAP",
        focused: state.focus === `swap_${index + 1}`,
        rect: { x: RIGHT_X + RIGHT_W - 74, y: rowY, w: 74, h: ROW_H },
        data: { align: "center", disabled: !view.can_configure },
      });
    }
  });

  widgets.push({
    id: "identity",
    kind: "label",
    text: identityText(view),
    rect: { x: RIGHT_X, y: ROW_TOP + 8 * ROW_STEP + 8, w: RIGHT_W, h: 116 },
    data: { align: "left", tone: "muted", focusable: false },
  });

  widgets.push({
    id: "status",
    kind: "label",
    text: view.status.toUpperCase(),
    rect: { x: LEFT_X, y: 428, w: 620, h: 20 },
    data: { align: "left", focusable: false },
  });
  // A terminated session outranks a dropped guest: once the lobby is over,
  // which seat emptied first is no longer the line to read. `room_error`
  // sits between `error` and `terminal_text`: it is what is left once
  // `error` itself has been cleared by a later `tick` command (see that
  // field's own doc on `LobbyModel`) -- a room-code failure this widget
  // must keep showing, not something a coordinator terminal ever outranks.
  let trouble = view.error ?? view.room_error ?? view.terminal_text;
  let detail = view.terminal?.detail;
  if (!trouble && view.departure) {
    trouble = view.departure_text;
    detail = view.departure.detail;
  }
  if (detail) {
    trouble = `${trouble ?? ""}  (${detail})`;
  }
  widgets.push({
    id: "trouble",
    kind: "label",
    text: trouble ? trouble.toUpperCase() : "",
    rect: { x: LEFT_X, y: 452, w: 620, h: 28 },
    data: { align: "left", tone: "muted", focusable: false },
  });

  widgets.push({
    id: "leave",
    kind: "button",
    text: "LEAVE LOBBY",
    focused: state.focus === "leave",
    rect: { x: LEFT_X, y: 488, w: 200, h: 36 },
  });
  if (view.role) {
    widgets.push({
      id: "ready",
      kind: "button",
      text: view.ready ? "NOT READY" : "READY",
      selected: view.ready,
      focused: state.focus === "ready",
      rect: { x: 380, y: 488, w: 200, h: 36 },
      data: { disabled: !view.can_ready },
    });
  }
  if (view.role === "host") {
    widgets.push({
      id: "start",
      kind: "button",
      text: "START COUNTDOWN",
      focused: state.focus === "start",
      rect: { x: 736, y: 488, w: 200, h: 36 },
      data: { disabled: !view.can_start },
    });
  }
  return widgets;
}

function commandFor(id: string, view: LobbyView): LobbyCommand | undefined {
  switch (id) {
    case "role_host":
      return { kind: "role", role: "host" };
    case "role_guest":
      return { kind: "role", role: "guest" };
    case "identity":
      return { kind: "identity" };
    case "bot_fill":
      return { kind: "bot_fill" };
    case "invite":
      return { kind: "invite" };
    case "copy_signal":
      return { kind: "copy" };
    case "paste_signal":
      return { kind: "paste_request" };
    case "lock":
      return { kind: "lock" };
    case "ready":
      return { kind: "ready", ready: !view.ready };
    case "start":
      return { kind: "start" };
    case "leave":
      return { kind: "leave" };
    case "room_code_host":
      return { kind: "room_pick", role: "host" };
    case "room_code_join":
      return { kind: "room_pick", role: "guest" };
    case ROOM_CODE_ENTRY_WIDGET:
      return { kind: "room_submit" };
    default:
      break;
  }
  const modeMatch = /^mode_(.+)$/.exec(id);
  if (modeMatch?.[1] !== undefined) {
    return { kind: "mode", mode: modeMatch[1] as SessionMatchMode };
  }
  const seatMatch = /^swap_(\d+)$/.exec(id);
  if (seatMatch?.[1] !== undefined) {
    return { kind: "swap", index: Number(seatMatch[1]) };
  }
  const slotMatch = /^prefer_(.+)$/.exec(id);
  if (slotMatch?.[1] !== undefined) {
    return { kind: "pair", slot: slotMatch[1] };
  }
  return undefined;
}

function actionFor(effects: readonly LobbyEffect[]): LobbyAction | undefined {
  for (const effect of effects) {
    if (effect.kind === "start_match") {
      return { go: "online_match", freeze: effect.freeze };
    }
  }
  for (const effect of effects) {
    if (effect.kind === "leave") {
      return { go: "main_menu" };
    }
  }
  return undefined;
}

function advance(
  state: LobbyScreenState,
  model: LobbyModel,
  nextFocus: string,
  effects: readonly LobbyEffect[],
): LobbyScreenState {
  return { viewport: state.viewport, ports: state.ports, model, focus: nextFocus, effects };
}

export type LobbyScreenEvent =
  FocusEvent | { readonly kind: "lobby"; readonly command: LobbyCommand };

function dispatchCommand(
  state: LobbyScreenState,
  cmd: LobbyCommand,
): readonly [LobbyScreenState, LobbyAction | undefined] {
  const [model, effects] = lobbyCommand(state.model, state.ports, cmd);
  return [advance(state, model, state.focus, effects), actionFor(effects)];
}

export function update(
  state: LobbyScreenState,
  event: LobbyScreenEvent,
): readonly [LobbyScreenState, LobbyAction | undefined] {
  if (event.kind === "lobby") {
    return dispatchCommand(state, event.command);
  }
  // The room-code composer is a single focused widget with its own
  // up/down/left/right meaning (cycle/move the character under the
  // cursor) instead of the general focus-navigation those actions
  // otherwise carry -- see `ROOM_CODE_ENTRY_WIDGET`'s own doc. "confirm"
  // and "back" fall through to the normal paths below (confirm submits
  // via `commandFor`'s `ROOM_CODE_ENTRY_WIDGET` case; back/leave is
  // universal).
  if (state.model.room_entry !== undefined && state.focus === ROOM_CODE_ENTRY_WIDGET) {
    if (event.kind === "key" && event.pressed !== false) {
      return dispatchCommand(state, { kind: "room_key", key: event.key });
    }
    if (event.kind === "action") {
      if (event.action === "up") {
        return dispatchCommand(state, { kind: "room_cycle", delta: 1 });
      }
      if (event.action === "down") {
        return dispatchCommand(state, { kind: "room_cycle", delta: -1 });
      }
      if (event.action === "left") {
        return dispatchCommand(state, { kind: "room_cursor", delta: -1 });
      }
      if (event.action === "right") {
        return dispatchCommand(state, { kind: "room_cursor", delta: 1 });
      }
    }
  }
  const currentLayout = layout(state);
  const nextFocus = focus.navigate(currentLayout, state.focus, event) ?? state.focus;
  if (event.kind === "action" && event.action === "back") {
    const [model, effects] = lobbyCommand(state.model, state.ports, { kind: "leave" });
    return [advance(state, model, nextFocus, effects), actionFor(effects)];
  }
  const id = focus.activated(currentLayout, nextFocus, event);
  if (id === null) {
    return [advance(state, state.model, nextFocus, []), undefined];
  }
  const cmd = commandFor(id, lobbyView(state.ports, state.model));
  if (!cmd) {
    return [advance(state, state.model, id, []), undefined];
  }
  const [model, effects] = lobbyCommand(state.model, state.ports, cmd);
  let nextState = advance(state, model, id, effects);
  // Focus survives a layout that no longer offers the activated control.
  nextState = { ...nextState, focus: focus.ensure(layout(nextState), id) ?? id };
  return [nextState, actionFor(effects)];
}

export const lobby = { newState, layout, update };
