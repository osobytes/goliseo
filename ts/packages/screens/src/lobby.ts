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
  type InputTeam,
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
 * `update()`'s own special-casing right below `layout()`. Exported so
 * `online_lobby.ts` can focus it directly when a room-joining intent is
 * preset from the multiplayer front door (#597): dispatching `room_pick`
 * through `LobbyScreenEvent`'s `"lobby"` kind leaves `state.focus`
 * untouched (`dispatchCommand` below), unlike the normal click path that
 * would have focused this widget by activating "JOIN WITH A ROOM CODE"
 * itself. */
export const ROOM_CODE_ENTRY_WIDGET = "room_code_slots";

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

// --- layout geometry, in virtual (960x540) pixels ---------------------------
//
// The arrangement changed; the controls did not. Every id below still maps to
// a `LobbyCommand` the model already accepted (`commandFor`), so this is a
// presentation pass over an unchanged 1,450-line model.
//
// What moved, and why:
//
//   - Slots are grouped into two team columns instead of one flat list of
//     eight. Which team a seat is on is the thing a player is actually
//     deciding; a flat list made it something you had to read an id prefix to
//     work out.
//   - Signaling stops being the headline. The blob was never rendered (and
//     still is not — only a direction, a size and a digest ever leave the
//     model), but "OUT PEER 1234 BYTES #ab12" was the most prominent thing on
//     the screen. An invite code carries the same digest in a form a player
//     can read out loud, and copy/paste happens behind it.
//   - Peers, countdown and start boundary read as one line rather than three
//     scattered status labels.
const LEFT_X = 24;
const TEAM_W = 300;
const HOME_X = LEFT_X;
const AWAY_X = 336;
const SEATS_X = 648;
const SEATS_W = 288;
/** Everything left of the seats column: the two team blocks and the controls under them. */
const MAIN_W = AWAY_X + TEAM_W - LEFT_X;
const HEADING_Y = 62;
const ROW_TOP = 84;
const ROW_STEP = 34;
const ROW_H = 30;
/** Four canonical outfield slots per team, then that team's protected keeper. */
const SLOTS_PER_TEAM = 4;
const KEEPER_Y = ROW_TOP + SLOTS_PER_TEAM * ROW_STEP;
/** The control block under the team columns. */
const CONTROLS_Y = KEEPER_Y + 32;
const SIGNAL_Y = CONTROLS_Y + 76;
const LINE_Y = SIGNAL_Y + 48;

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

/**
 * The invite line. Only the direction, size and digest ever leave the model —
 * the blob itself is handed to the clipboard and dropped, never held for
 * display — so the digest is what a player shares to confirm both sides
 * exchanged the same thing. The blob still travels by clipboard; this is the
 * human-readable handle on it, not a substitute for it.
 */
function inviteText(record: LobbySignalRecord | undefined): string {
  if (!record) {
    return "NO INVITE YET  —  INVITE A PEER TO CREATE ONE";
  }
  return `INVITE  GC://JOIN/${record.fingerprint.toUpperCase()}  •  ${record.direction.toUpperCase()}  ${record.bytes} BYTES`;
}

function answerText(record: LobbySignalRecord | undefined): string {
  if (!record) {
    return "NO REPLY IMPORTED YET";
  }
  return `REPLY FROM ${record.peer_id.toUpperCase()}  #${record.fingerprint.toUpperCase()}`;
}

/**
 * Peers, countdown and start boundary as one sentence. Three separate status
 * labels made the player assemble the same fact themselves.
 */
function lineText(view: LobbyView): string {
  const required = view.mode_known ? String(view.required) : "?";
  const parts = [`PEERS  ${view.connected} / ${required}`];
  if (view.mode_known && view.connected < view.required) {
    const missing = view.required - view.connected;
    parts.push(`WAITING ON ${missing} ${missing === 1 ? "PLAYER" : "PLAYERS"}`);
  }
  if (view.mode_known && view.connected >= view.required && !view.started) {
    parts.push(`READY  ${view.ready_count} / ${view.connected}`);
  }
  return parts.join("  •  ");
}

function identityText(view: LobbyView): string {
  return view.identity.map((row) => `${row.label}  ${row.value.toUpperCase()}`).join("\n");
}

interface LeftOptions {
  readonly kind?: string;
  readonly selected?: boolean;
  readonly tone?: "muted";
  readonly disabled?: boolean;
  readonly focusable?: boolean;
  readonly align?: "left" | "center" | "right";
}

/** The team column order: whichever teams the model actually published, home first. */
function teamOrder(view: LobbyView): readonly InputTeam[] {
  const seen: InputTeam[] = [];
  for (const keeper of view.keepers) {
    if (!seen.includes(keeper.team)) {
      seen.push(keeper.team);
    }
  }
  for (const slot of view.slots) {
    if (!seen.includes(slot.team)) {
      seen.push(slot.team);
    }
  }
  return seen;
}

export function layout(state: LobbyScreenState): Layout {
  const view = lobbyView(state.ports, state.model);
  const widgets: Widget[] = [
    {
      id: "title",
      kind: "title",
      text: "LOBBY",
      rect: { x: LEFT_X, y: 18, w: 300, h: 28 },
      data: { align: "left", focusable: false },
    },
  ];

  const control = (
    id: string,
    text: string,
    rect: { x: number; y: number; w: number; h: number },
    options?: LeftOptions,
  ): void => {
    widgets.push({
      id,
      kind: options?.kind ?? "button",
      text,
      ...(options?.selected !== undefined ? { selected: options.selected } : {}),
      focused: state.focus === id,
      rect,
      data: {
        align: options?.align ?? "left",
        ...(options?.tone !== undefined ? { tone: options.tone } : {}),
        ...(options?.disabled !== undefined ? { disabled: options.disabled } : {}),
        ...(options?.focusable !== undefined ? { focusable: options.focusable } : {}),
      },
    });
  };

  // --- room code: its own sub-view, and the only one with no lobby behind it --
  if (view.room_entry) {
    const entry: RoomCodeEntry = view.room_entry;
    const text = entry.chars
      .map((ch, index) => (index === entry.cursor ? `[${ch || "_"}]` : ch || "_"))
      .join(" ");
    control(
      ROOM_CODE_ENTRY_WIDGET,
      text,
      { x: 280, y: 210, w: 400, h: 48 },
      {
        align: "center",
      },
    );
    control(
      "room_hint",
      "Type the code, or cycle a character with up/down and move with left/right. Confirm to join.",
      { x: 230, y: 272, w: 500, h: 44 },
      { kind: "label", tone: "muted", focusable: false, align: "center" },
    );
  } else if (!view.role) {
    // --- role: the one state with no seating to show -------------------------
    //
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
    control(
      "room_code_host",
      "HOST WITH A ROOM CODE",
      { x: 330, y: 150, w: 300, h: 44 },
      {
        disabled: roomBusy,
      },
    );
    control(
      "room_code_join",
      "JOIN WITH A ROOM CODE",
      { x: 330, y: 200, w: 300, h: 44 },
      {
        disabled: roomBusy,
      },
    );
    control(
      "role_host",
      "HOST A SESSION",
      { x: 330, y: 250, w: 300, h: 44 },
      {
        disabled: roomBusy,
      },
    );
    control(
      "role_guest",
      "JOIN WITH AN OFFER",
      { x: 330, y: 300, w: 300, h: 44 },
      {
        disabled: roomBusy,
      },
    );
    control("identity", `JOIN AS  ${view.peer_id.toUpperCase()}`, {
      x: 330,
      y: 350,
      w: 300,
      h: ROW_H,
    });
    control(
      "hint",
      "A room code connects automatically. The manual path trades an offer by clipboard instead. Nothing is stored either way.",
      { x: 170, y: 386, w: 620, h: 40 },
      { kind: "label", tone: "muted", focusable: false, align: "center" },
    );
  }

  // --- header: role, then the mode it implies --------------------------------
  if (view.role === "host") {
    const modeW = 76;
    MODES.forEach((mode, index) => {
      control(
        `mode_${mode}`,
        mode.toUpperCase(),
        {
          x: SEATS_X + index * (modeW + 10),
          y: 20,
          w: modeW,
          h: 26,
        },
        {
          selected: view.mode === mode,
          disabled: view.mode_locked,
          align: "center",
        },
      );
    });
  } else if (view.role === "guest") {
    control(
      "mode_label",
      `MODE  ${view.mode_known ? view.mode.toUpperCase() : "PENDING"}`,
      {
        x: SEATS_X,
        y: 22,
        w: SEATS_W,
        h: 22,
      },
      { kind: "label", tone: "muted", focusable: false, align: "right" },
    );
  }
  if (view.role) {
    control(
      "role_label",
      view.role.toUpperCase(),
      {
        x: 340,
        y: 24,
        w: 160,
        h: 22,
      },
      { kind: "label", focusable: false },
    );
  }

  // --- the two team columns --------------------------------------------------
  //
  // Grouped by team rather than listed as eight flat slots: which side a seat
  // is on is what the player is deciding. A slot the local peer could ask the
  // host for carries its own control; in `1v1` and `4v4` none ever does,
  // because there is no pair to choose.
  // Only once a role exists. Before that there is no session to seat anyone
  // in, and eight unassigned slots behind the role picker are noise the
  // player cannot act on.
  const teams = view.role ? teamOrder(view) : [];
  const columnX = (team: InputTeam): number => (teams.indexOf(team) === 0 ? HOME_X : AWAY_X);

  teams.forEach((team) => {
    const x = columnX(team);
    widgets.push({
      id: `team_${team}`,
      kind: "eyebrow",
      text: team.toUpperCase(),
      rect: { x, y: HEADING_Y, w: TEAM_W, h: 18 },
      data: { align: "left", focusable: false },
    });
  });

  const rowIndex = new Map<InputTeam, number>();
  (view.role ? view.slots : []).forEach((slot) => {
    const index = rowIndex.get(slot.team) ?? 0;
    rowIndex.set(slot.team, index + 1);
    const x = columnX(slot.team);
    const rowY = ROW_TOP + index * ROW_STEP;
    widgets.push({
      id: `slot_${slot.slot}`,
      kind: "card",
      text: slotText(slot),
      selected: slot.local_owner,
      rect: { x, y: rowY, w: TEAM_W - (slot.can_prefer ? 66 : 0), h: ROW_H },
      data: { align: "left", focusable: false },
    });
    if (slot.can_prefer) {
      widgets.push({
        id: `prefer_${slot.slot}`,
        kind: "button",
        text: "TAKE",
        focused: state.focus === `prefer_${slot.slot}`,
        rect: { x: x + TEAM_W - 62, y: rowY, w: 62, h: ROW_H },
        data: { align: "center" },
      });
    }
  });
  (view.role ? view.keepers : []).forEach((keeper) => {
    widgets.push({
      id: `keeper_${keeper.team}`,
      kind: "label",
      text: `KEEPER  ${keeper.player_id.toUpperCase()}  PROTECTED AI`,
      rect: { x: columnX(keeper.team), y: KEEPER_Y, w: TEAM_W, h: 20 },
      data: { align: "left", tone: "muted", focusable: false },
    });
  });
  if (view.role && view.preference) {
    const preference: LobbyPreferenceView = view.preference;
    widgets.push({
      id: "preference",
      kind: "label",
      text: `PAIR ${preference.slots.join(" ").toUpperCase()}  ${preference.text.toUpperCase()}`,
      rect: { x: LEFT_X, y: KEEPER_Y + 22, w: MAIN_W, h: 20 },
      data: { align: "left", tone: "muted", focusable: false },
    });
  }

  // --- host controls, and the invite that replaced the blob ------------------
  if (view.role === "host") {
    control(
      "invite",
      "INVITE A PEER",
      { x: LEFT_X, y: CONTROLS_Y, w: 180, h: ROW_H },
      {
        disabled: !view.can_invite,
        align: "center",
      },
    );
    control(
      "lock",
      "LOCK CONFIGURATION",
      { x: 214, y: CONTROLS_Y, w: 180, h: ROW_H },
      {
        disabled: !view.can_lock,
        align: "center",
      },
    );
    control(
      "bot_fill",
      view.bot_fill ? "AI FILLS EMPTY SEATS: ON" : "AI FILLS EMPTY SEATS: OFF",
      { x: 404, y: CONTROLS_Y, w: 232, h: ROW_H },
      { selected: view.bot_fill, disabled: view.mode_locked, align: "center" },
    );
  }

  // The room code, when there is one. It sits in the slot the copy/paste pair
  // vacates rather than replacing the invite line, because `room_code` outlives
  // `room_active`: after a `room_dropped` the manual controls come back while
  // the code is still on screen, so the two must not share a rect.
  if (view.role === "host" && view.room_code) {
    control(
      "room_code_display",
      `ROOM CODE  ${view.room_code}`,
      { x: 404, y: CONTROLS_Y + 38, w: 232, h: ROW_H },
      { kind: "label", focusable: false, align: "center" },
    );
  } else if (view.role === "guest" && view.room_active) {
    control(
      "room_code_active",
      "CONNECTED VIA ROOM CODE",
      { x: 404, y: CONTROLS_Y + 38, w: 232, h: ROW_H },
      { kind: "label", tone: "muted", focusable: false, align: "center" },
    );
  }

  // Manual signalling is the fallback path, so it disappears entirely once a
  // room code is carrying the connection — and comes back if that drops.
  if (view.role && !view.room_active) {
    control(
      "copy_signal",
      "COPY INVITE",
      { x: LEFT_X, y: CONTROLS_Y + 38, w: 180, h: ROW_H },
      {
        disabled: !view.has_outgoing,
        align: "center",
      },
    );
    control(
      "paste_signal",
      "PASTE REPLY",
      { x: 214, y: CONTROLS_Y + 38, w: 180, h: ROW_H },
      {
        align: "center",
      },
    );
    control(
      "signal_out",
      inviteText(view.exported),
      {
        x: LEFT_X,
        y: SIGNAL_Y,
        w: 370,
        h: 40,
      },
      { kind: "card", tone: "muted", focusable: false },
    );
    control(
      "signal_in",
      answerText(view.imported),
      {
        x: 404,
        y: SIGNAL_Y,
        w: 232,
        h: 40,
      },
      { kind: "card", tone: "muted", focusable: false },
    );
  }

  if (view.role) {
    // Peers, countdown and start boundary as one line.
    control(
      "peer_count",
      lineText(view),
      { x: LEFT_X, y: LINE_Y, w: 320, h: 20 },
      {
        kind: "label",
        focusable: false,
      },
    );
    if (view.countdown !== undefined) {
      control(
        "countdown",
        `COUNTDOWN  ${view.countdown} TICKS`,
        {
          x: 352,
          y: LINE_Y,
          w: 160,
          h: 20,
        },
        { kind: "label", focusable: false },
      );
    }
    if (view.started) {
      control(
        "started",
        "START BOUNDARY REACHED",
        { x: 520, y: LINE_Y, w: 216, h: 20 },
        {
          kind: "label",
          focusable: false,
        },
      );
    }
  }

  // --- seats: one row per human, with the host's ownership swap --------------
  if (view.role) {
    widgets.push({
      id: "seats_heading",
      kind: "eyebrow",
      text: "SEATS",
      rect: { x: SEATS_X, y: HEADING_Y, w: SEATS_W, h: 18 },
      data: { align: "left", focusable: false },
    });
  }
  (view.role ? view.seats : []).forEach((seat, index) => {
    const rowY = ROW_TOP + index * ROW_STEP;
    widgets.push({
      id: `seat_${index + 1}`,
      kind: "card",
      text: seatText(seat),
      selected: seat.is_local,
      rect: { x: SEATS_X, y: rowY, w: SEATS_W - 82, h: ROW_H },
      data: { align: "left", focusable: false },
    });
    if (view.role === "host" && index < view.seats.length - 1) {
      widgets.push({
        id: `swap_${index + 1}`,
        kind: "button",
        text: "SWAP",
        focused: state.focus === `swap_${index + 1}`,
        rect: { x: SEATS_X + SEATS_W - 74, y: rowY, w: 74, h: ROW_H },
        data: { align: "center", disabled: !view.can_configure },
      });
    }
  });

  if (view.role) {
    widgets.push({
      id: "identity",
      kind: "label",
      text: identityText(view),
      rect: { x: SEATS_X, y: 336, w: SEATS_W, h: 88 },
      data: { align: "left", tone: "muted", focusable: false },
    });
  }

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
