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
//
// # Layout is one dispatch over the model's phase (#566)
//
// `lobby_model.ts` publishes a nine-value `view.phase` (`role` | `handshake`
// | `manifest` | `assigned` | `ready` | `countdown` | `running` | `result` |
// `terminal`), plus a `view.room_entry` sub-view of `role` for the room-code
// composer. `layout()` below is one dispatch over that phase, and each arm
// builds only the widgets that phase's player can act on -- nothing a role
// screen can't seat, no eight-row roster before there is anything to seat,
// no ticks on a player's screen, no protocol dump unless it's asked for or
// load-bearing. `state.details` is the one piece of genuinely screen-local
// state this adds: whether the identity dump (build/content/tuning ids) is
// currently exposed outside the `terminal` phase, where it is load-bearing
// (a `build_mismatch`/`manifest_mismatch` reads there) and always shown.
//
// Every widget id below still maps to a `LobbyCommand` the model already
// accepts (`commandFor`) -- ids are presentation, not contract, but the
// commands they dispatch are the two-way contract with `lobby_model.ts`.
//
// # The two-click START collapse (#610)
//
// LOCK MATCH, the host's own READY toggle, and the guest's READY toggle are
// gone as separate widgets. A single START MATCH button (`footerWidgets`'s
// `showStart`) does all three, plus starting the countdown, from one host
// click; a guest becomes ready automatically the moment it is assigned a
// seat, with nothing to click at all. See `lobby_model.ts`'s own header for
// the collapse itself -- this file only ever renders what `view.can_start`/
// `view.can_configure` already say, same as it did for the individual
// commands before.

import { focus, type Layout, type Widget } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import {
  MODES,
  command as lobbyCommand,
  newLobbyModel,
  view as lobbyView,
  type InputTeam,
  type LobbyCommand,
  type LobbyEffect,
  type LobbyModel,
  type LobbyModelOptions,
  type LobbyModelPorts,
  type LobbySignalRecord,
  type LobbySlotView,
  type LobbyView,
  type RoomCodeEntry,
  type SessionMatchMode,
} from "./lobby_model.ts";
import {
  newRoomCodeEntry,
  roomCodeCursor,
  roomCodeCycle,
  roomCodeDisplay,
  roomCodeKey,
  roomCodeText,
  ROOM_CODE_ALPHABET,
} from "./room_code_entry.ts";

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

/** The host's own inline "have a code?" composer on the auto-hosted
 * handshake screen (#610 round-2 review, blocking finding 1c) --
 * deliberately a DIFFERENT widget id from `ROOM_CODE_ENTRY_WIDGET`: a
 * different screen state (host, already live), a different outcome
 * (`switchToGuest` restarts the whole lobby rather than dispatching a
 * model command), sharing only the character-editing primitives
 * (`room_code_entry.ts`). */
export const JOIN_ENTRY_WIDGET = "join_code_entry";

export interface LobbyScreenContext {
  readonly model?: LobbyModel;
  readonly options?: LobbyModelOptions;
}

export interface LobbyScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly ports: LobbyModelPorts;
  readonly model: LobbyModel;
  readonly focus: string;
  /** Whether the identity/build dump is currently exposed outside the
   * `terminal` phase (where it is always shown). Purely a screen concern --
   * `lobby_model.ts` never reads or sets it -- so it lives here rather than
   * on `LobbyModel`, per this file's own header. */
  readonly details: boolean;
  /** The host's own inline "have a code?" composer (#610) -- screen-local
   * exactly like `details`: `lobby_model.ts` never sees it, and it is
   * meaningless outside `view.role === "host" && view.phase ===
   * "handshake"` (`layoutHandshake`'s own guard), but always present so
   * `update()` never has to conjure one on first keystroke. */
  readonly join_entry: RoomCodeEntry;
  /** Produced by the last update; drained by the owner. */
  readonly effects: readonly LobbyEffect[];
}

export type LobbyAction =
  | { readonly go: "online_match"; readonly freeze: unknown }
  | { readonly go: "main_menu" }
  // #610 round-2 review, blocking finding 1: CANCEL/back from the
  // auto-hosted screen (no fields) and the inline "have a code?" composer
  // (`intent`/`code`) both restart the WHOLE lobby screen rather than
  // mutating this one's live star transport in place -- see
  // `cancelToRoleScreen`/`switchToGuest`'s own doc, and `app.ts`'s
  // `restartLobby`, which is the only place this is consumed.
  | { readonly go: "lobby_restart" }
  | { readonly go: "lobby_restart"; readonly intent: "guest"; readonly code: string };

// --- layout geometry, in virtual (960x540) pixels ---------------------------
//
// Page gutters are at least 62px and interactive targets at least 38px tall
// (docs/visual_style.md) -- both were violated before this pass (`LEFT_X`
// was 24; mode chips were 26px tall) and are fixed here along with the
// phase split itself.
const LEFT_X = 62;
const RIGHT_EDGE = 898;
/** The left content column: manual controls, the room-code hero, the roster. */
const LEFT_COL_W = 460;
/** The right column: the per-human players strip. */
const RIGHT_COL_X = 560;
const RIGHT_COL_W = 338;
const CONTENT_TOP = 84;
const FOOTER_TEXT_Y = 420;
const FOOTER_BUTTON_Y = 490;

export function newState(
  viewport: { readonly w: number; readonly h: number },
  ports: LobbyModelPorts,
  context?: LobbyScreenContext,
): LobbyScreenState {
  return {
    viewport,
    ports,
    model: context?.model ?? newLobbyModel(ports, context?.options),
    // Tab order is the statement of priority: a room code is the primary
    // path (it connects automatically), so it claims initial focus rather
    // than the manual "HOST A SESSION" button that used to sit first.
    focus: "room_code_host",
    details: false,
    join_entry: newRoomCodeEntry(),
    effects: [],
  };
}

// --- small widget-building primitives ---------------------------------------

type Align = "left" | "center" | "right";
interface Rect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

interface ControlOptions {
  readonly kind?: string;
  readonly selected?: boolean;
  readonly tone?: "muted";
  readonly disabled?: boolean;
  readonly align?: Align;
}

/** An interactive widget (button by default): focused from `state.focus`,
 * clickable unless `disabled`. */
function control(
  widgets: Widget[],
  state: LobbyScreenState,
  id: string,
  value: string,
  rect: Rect,
  options?: ControlOptions,
): void {
  widgets.push({
    id,
    kind: options?.kind ?? "button",
    text: value,
    ...(options?.selected !== undefined ? { selected: options.selected } : {}),
    focused: state.focus === id,
    rect,
    data: {
      align: options?.align ?? "left",
      ...(options?.tone !== undefined ? { tone: options.tone } : {}),
      ...(options?.disabled !== undefined ? { disabled: options.disabled } : {}),
    },
  });
}

interface TextOptions {
  readonly kind?: string;
  readonly align?: Align;
  readonly tone?: "muted";
}

/** A non-interactive label/eyebrow/title/hero_title widget. */
function text(
  widgets: Widget[],
  id: string,
  value: string,
  rect: Rect,
  options: TextOptions,
): void {
  widgets.push({
    id,
    kind: options.kind ?? "label",
    text: value,
    rect,
    data: {
      align: options.align ?? "left",
      ...(options.tone !== undefined ? { tone: options.tone } : {}),
      focusable: false,
    },
  });
}

interface CardOptions {
  readonly align?: Align;
  readonly tone?: "muted";
  readonly selected?: boolean;
}

/** A non-interactive display card -- `focus.ts` treats every "card" kind as
 * focusable unless told otherwise, so this always says otherwise. */
function displayCard(
  widgets: Widget[],
  id: string,
  value: string,
  rect: Rect,
  options?: CardOptions,
): void {
  widgets.push({
    id,
    kind: "card",
    text: value,
    ...(options?.selected !== undefined ? { selected: options.selected } : {}),
    rect,
    data: {
      align: options?.align ?? "left",
      ...(options?.tone !== undefined ? { tone: options.tone } : {}),
      focusable: false,
    },
  });
}

// --- copy helpers -------------------------------------------------------

/** A snake_case id ("zyro_vex", "guest_1") as a readable display name
 * ("Zyro Vex", "Guest 1"). Body copy is sentence case
 * (docs/visual_style.md); these ids have no separate display name, so this
 * is the whole of that rule applied to them. */
function displayName(id: string): string {
  return id
    .split("_")
    .filter((part) => part.length > 0)
    .map((part) => `${part[0]?.toUpperCase() ?? ""}${part.slice(1)}`)
    .join(" ");
}

/** A short, concise-uppercase status tag for a slot -- who drives it, not
 * who owns it (that's `slot.local_owner`/`selected`). */
function slotTag(slot: LobbySlotView): string {
  if (slot.driver === "human") {
    return "LIVE";
  }
  if (slot.owner_kind === "bot") {
    return "AI FILL";
  }
  if (slot.owner_kind === "peer") {
    return "AI (OWNED)";
  }
  return "OPEN";
}

/**
 * The invite line. Only the direction, size and digest ever leave the model —
 * the blob itself is handed to the clipboard and dropped, never held for
 * display — so the digest is what a player shares to confirm both sides
 * exchanged the same thing. The blob still travels by clipboard; this is the
 * human-readable handle on it, not a substitute for it.
 *
 * Used to render a `GC://JOIN/<fingerprint>` string here -- decorative, per
 * #598: nothing ever parsed it, so a player who pasted it anywhere got
 * nothing. Real join links exist now (`layoutHandshake`'s COPY LINK/SHARE),
 * so the fake protocol string is gone rather than left to mislead.
 */
function inviteText(record: LobbySignalRecord | undefined): string {
  if (!record) {
    return "No invite yet — invite a peer to create one.";
  }
  return `Invite ${record.direction} #${record.fingerprint.toUpperCase()} — ${record.bytes} bytes.`;
}

function answerText(record: LobbySignalRecord | undefined): string {
  if (!record) {
    return "No reply imported yet.";
  }
  return `Reply from ${displayName(record.peer_id)} — #${record.fingerprint.toUpperCase()}.`;
}

/** Peers, and what is still missing, as one line. */
function lineText(view: LobbyView): string {
  const required = view.mode_known ? String(view.required) : "?";
  const parts = [`${view.connected} / ${required} connected`];
  if (view.mode_known && view.connected < view.required) {
    const missing = view.required - view.connected;
    parts.push(`waiting on ${missing} ${missing === 1 ? "player" : "players"}`);
  }
  if (view.mode_known && view.connected >= view.required && !view.started) {
    parts.push(`${view.ready_count} / ${view.connected} ready`);
  }
  return parts.join("  •  ");
}

function identityLines(view: LobbyView): string {
  return view.identity.map((row) => `${row.label}  ${row.value}`).join("\n");
}

/**
 * A transient command error, a room-code failure (#602's own distinct copy
 * per reason), or a guest dropping while the lobby is still standing
 * (`departure_text`, with its raw `detail` folded in) -- everything short of
 * a terminated session, which gets its own headline (`layoutTerminal`)
 * instead. Shared by `footerWidgets` and `layoutCountdown`: a departure is
 * phase-independent and can land seconds before kickoff just as easily as
 * during handshake, so the countdown's otherwise-minimal screen still has
 * to make room for it.
 */
function troubleText(view: LobbyView): string {
  // `late_joiner_note` is the lowest-priority fallback deliberately (#610
  // round-2 review, blocking finding 3): it is the host's own quiet
  // record that a late joiner was turned away, not something urgent about
  // THIS session, so anything more pressing (a real error, a room
  // failure, a departure) still wins the line.
  const trouble =
    view.error ?? view.room_error ?? view.departure_text ?? view.late_joiner_note ?? "";
  if (!trouble) {
    return "";
  }
  const detail =
    view.error === undefined && view.room_error === undefined ? view.departure?.detail : undefined;
  return `${trouble}${detail ? ` (${detail})` : ""}`;
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

// --- shared header: title, role, and the mode (chips while open, a static
// label once locked -- `mode_locked` is permanent, and a permanently
// disabled control is a lie) --------------------------------------------

function headerWidgets(widgets: Widget[], state: LobbyScreenState, view: LobbyView): void {
  text(widgets, "title", "LOBBY", { x: LEFT_X, y: 20, w: 200, h: 26 }, { kind: "title" });
  if (view.role) {
    text(
      widgets,
      "role_label",
      view.role.toUpperCase(),
      { x: LEFT_X + 210, y: 24, w: 140, h: 20 },
      {},
    );
  }
  const modeAreaW = 272;
  const modeX = RIGHT_EDGE - modeAreaW;
  if (view.mode_locked) {
    text(
      widgets,
      "mode_label",
      `MODE ${view.mode.toUpperCase()}`,
      { x: modeX, y: 22, w: modeAreaW, h: 22 },
      { align: "right" },
    );
  } else if (view.role === "host") {
    const chipW = 84;
    const gap = 10;
    MODES.forEach((mode, index) => {
      control(
        widgets,
        state,
        `mode_${mode}`,
        mode.toUpperCase(),
        { x: modeX + index * (chipW + gap), y: 20, w: chipW, h: 38 },
        { selected: view.mode === mode, align: "center" },
      );
    });
  } else if (view.role === "guest") {
    text(
      widgets,
      "mode_label",
      view.mode_known ? `MODE ${view.mode.toUpperCase()}` : "MODE PENDING",
      { x: modeX, y: 22, w: modeAreaW, h: 22 },
      { align: "right", tone: "muted" },
    );
  }
}

// --- shared footer: status/trouble (or the identity dump, toggled),
// LEAVE, and the phase's own primary actions ------------------------------

interface FooterOptions {
  readonly showStart?: boolean;
  readonly showDetails?: boolean;
  readonly leaveText?: string;
}

function footerWidgets(
  widgets: Widget[],
  state: LobbyScreenState,
  view: LobbyView,
  opts: FooterOptions,
): void {
  if (opts.showDetails && state.details) {
    displayCard(
      widgets,
      "identity_card",
      identityLines(view),
      { x: LEFT_X, y: FOOTER_TEXT_Y, w: LEFT_COL_W, h: 64 },
      { tone: "muted" },
    );
  } else {
    text(widgets, "status", view.status, { x: LEFT_X, y: FOOTER_TEXT_Y, w: LEFT_COL_W, h: 20 }, {});
    text(
      widgets,
      "trouble",
      troubleText(view),
      { x: LEFT_X, y: FOOTER_TEXT_Y + 24, w: LEFT_COL_W, h: 36 },
      { tone: "muted" },
    );
  }

  const btnW = 170;
  const gap = 16;
  let footerX = LEFT_X;
  control(
    widgets,
    state,
    "leave",
    opts.leaveText ?? "LEAVE LOBBY",
    { x: footerX, y: FOOTER_BUTTON_Y, w: btnW, h: 38 },
    { align: "center" },
  );
  footerX += btnW + gap;
  if (opts.showStart) {
    // Two different reasons can disable this button, and they read
    // differently: "handshake" but not enough players yet is still
    // START MATCH, just disabled (`can_start`'s own gate); past
    // "handshake" the collapse is genuinely under way, so the button stays
    // visible (this footer keeps rendering it through manifest / assigned /
    // ready -- #610's own doc on `LobbyView.can_start`) but reads
    // "STARTING…" right up to the countdown screen taking over entirely.
    const starting = view.phase !== "handshake";
    control(
      widgets,
      state,
      "start",
      starting ? "STARTING…" : "START MATCH",
      { x: footerX, y: FOOTER_BUTTON_Y, w: btnW, h: 38 },
      { disabled: !view.can_start, align: "center" },
    );
  }
  if (opts.showDetails) {
    control(
      widgets,
      state,
      "details",
      state.details ? "HIDE DETAILS" : "DETAILS",
      { x: RIGHT_EDGE - btnW, y: FOOTER_BUTTON_Y, w: btnW, h: 38 },
      { selected: state.details, align: "center" },
    );
  }
}

// --- the players strip: the SEATS column's two real payloads (who is
// connected, who is ready), with the host's SWAP between adjacent chips --
// everything else SEATS restated (per-slot ownership) is now the roster's
// job, so it is gone. ------------------------------------------------------

function playersStripWidgets(
  widgets: Widget[],
  state: LobbyScreenState,
  view: LobbyView,
  x: number,
  y: number,
  w: number,
): void {
  text(widgets, "players_heading", "PEERS", { x, y, w, h: 18 }, { kind: "eyebrow" });
  text(widgets, "peer_count", lineText(view), { x, y: y + 20, w, h: 20 }, {});
  const rowStep = 42;
  const rowH = 38;
  const swapW = 70;
  const canSwap = view.role === "host" && view.can_configure;
  const cardW = canSwap ? w - swapW - 8 : w;
  view.seats.forEach((seat, index) => {
    const rowY = y + 48 + index * rowStep;
    displayCard(
      widgets,
      `player_${index + 1}`,
      `${displayName(seat.peer_id)}${seat.is_local ? " (you)" : ""}${seat.ready ? "  •  Ready" : ""}`,
      { x, y: rowY, w: cardW, h: rowH },
      { selected: seat.is_local },
    );
    if (canSwap && index < view.seats.length - 1) {
      control(
        widgets,
        state,
        `swap_${index + 1}`,
        "SWAP",
        { x: x + cardW + 8, y: rowY, w: swapW, h: rowH },
        { align: "center" },
      );
    }
  });
}

// --- the roster: mode-dependent, because `can_prefer` (a pair to trade
// for) is only ever true in 2v2 -- 1v1 owns a whole line, 4v4 owns a single
// slot, so per-slot ownership controls have nothing to offer in either. ---

function rosterTeamColumns(
  widgets: Widget[],
  state: LobbyScreenState,
  view: LobbyView,
  x: number,
  y: number,
  w: number,
): number {
  const teams = teamOrder(view);
  const colW = (w - 20) / 2;
  const rowStep = 42;
  const rowH = 38;
  let bottom = y;
  teams.forEach((team, teamIndex) => {
    const cx = x + teamIndex * (colW + 20);
    text(
      widgets,
      `team_${team}`,
      team.toUpperCase(),
      { x: cx, y, w: colW, h: 18 },
      { kind: "eyebrow" },
    );
    const slots = view.slots.filter((slot) => slot.team === team);
    slots.forEach((slot, slotIndex) => {
      const rowY = y + 24 + slotIndex * rowStep;
      const takeW = slot.can_prefer ? 66 : 0;
      displayCard(
        widgets,
        `slot_${slot.slot}`,
        `${displayName(slot.player_id)} — ${slotTag(slot)}`,
        { x: cx, y: rowY, w: colW - (takeW > 0 ? takeW + 8 : 0), h: rowH },
        { selected: slot.local_owner },
      );
      if (slot.can_prefer) {
        control(
          widgets,
          state,
          `prefer_${slot.slot}`,
          "TAKE",
          { x: cx + colW - takeW, y: rowY, w: takeW, h: rowH },
          { align: "center" },
        );
      }
    });
    const keeperY = y + 24 + slots.length * rowStep + 4;
    const keeper = view.keepers.find((entry) => entry.team === team);
    if (keeper) {
      text(
        widgets,
        `keeper_${team}`,
        `Keeper: ${displayName(keeper.player_id)} (AI)`,
        { x: cx, y: keeperY, w: colW, h: 20 },
        { tone: "muted" },
      );
    }
    bottom = Math.max(bottom, keeperY + 20);
  });
  return bottom;
}

function rosterSummaryCards(
  widgets: Widget[],
  view: LobbyView,
  x: number,
  y: number,
  w: number,
): number {
  const teams = teamOrder(view);
  const colW = (w - 20) / 2;
  const h = 150;
  teams.forEach((team, index) => {
    const cx = x + index * (colW + 20);
    const slots = view.slots.filter((slot) => slot.team === team);
    // A bot has no "live" slot at all (`previewLive` only ever names a
    // human's opening slot) -- a team the local peer does not own is either
    // a human's team (show who plays it) or entirely AI (say so, rather
    // than naming one interchangeable bot slot over another).
    const live = slots.find((slot) => slot.live);
    const mine = slots.some((slot) => slot.local_owner);
    const allBot = slots.length > 0 && slots.every((slot) => slot.owner_kind === "bot");
    const keeper = view.keepers.find((entry) => entry.team === team);
    const lines = [
      team === "home" ? "Home" : "Away",
      live ? displayName(live.player_id) : allBot ? "AI-controlled" : "Unassigned",
      mine ? "You" : "",
      keeper ? `Keeper: ${displayName(keeper.player_id)} (AI)` : "",
    ].filter((line) => line.length > 0);
    displayCard(
      widgets,
      `team_summary_${team}`,
      lines.join("\n"),
      { x: cx, y, w: colW, h },
      { selected: mine },
    );
  });
  return y + h;
}

function rosterPersonalCard(
  widgets: Widget[],
  view: LobbyView,
  x: number,
  y: number,
  w: number,
): number {
  const mine = view.slots.find((slot) => slot.local_owner);
  const cardH = 56;
  const headline = mine
    ? `You play ${displayName(mine.player_id)} — ${mine.team === "home" ? "Home" : "Away"}`
    : "Waiting for a seat.";
  displayCard(
    widgets,
    "you_card",
    headline,
    { x, y, w, h: cardH },
    { selected: mine !== undefined },
  );

  const teammates = view.slots
    .filter((slot) => slot.team === mine?.team && !slot.local_owner)
    .map((slot) => `${displayName(slot.player_id)}  ${slotTag(slot)}`);
  const keeper = view.keepers.find((entry) => entry.team === mine?.team);
  if (keeper) {
    teammates.push(`Keeper: ${displayName(keeper.player_id)} (AI)`);
  }
  const listH = 130;
  text(
    widgets,
    "teammates",
    teammates.length > 0 ? `Teammates:\n${teammates.join("\n")}` : "No teammates yet.",
    { x, y: y + cardH + 10, w, h: listH },
    { tone: "muted" },
  );
  return y + cardH + 10 + listH;
}

/** Dispatches to the mode-appropriate roster and returns the y just past it,
 * so the caller can place the pair-preference line without hand-tuned
 * per-mode constants drifting out of sync with the geometry above. */
function rosterWidgets(
  widgets: Widget[],
  state: LobbyScreenState,
  view: LobbyView,
  x: number,
  y: number,
  w: number,
): number {
  if (view.mode === "2v2") {
    return rosterTeamColumns(widgets, state, view, x, y, w);
  }
  if (view.mode === "1v1") {
    return rosterSummaryCards(widgets, view, x, y, w);
  }
  return rosterPersonalCard(widgets, view, x, y, w);
}

// --- role: the one phase with no session to seat -----------------------

function layoutRole(state: LobbyScreenState, view: LobbyView): Layout {
  const widgets: Widget[] = [];
  text(widgets, "title", "LOBBY", { x: LEFT_X, y: 20, w: 200, h: 26 }, { kind: "title" });
  text(
    widgets,
    "role_hint",
    "Host a session or join one with a code.",
    { x: 0, y: 58, w: 960, h: 20 },
    { align: "center", tone: "muted" },
  );

  // Disabled for the ENTIRE window a room-code attempt is in flight or
  // established but not yet a chosen role (`view.room_status ===
  // "connecting"` covers the first; `view.room_active` alone covers a
  // `"connected"` state that has not yet reached `chooseRole` -- both
  // collapse once `room_created`/`room_joined` lands, since this whole
  // phase stops rendering the moment `view.role` is set). Activating a
  // manual role or a second room-code attempt during this window used to
  // wedge the lobby -- `chooseRole`'s and `roomPick`'s own call-site guards
  // are the belt to this layout's braces.
  const roomBusy = view.room_status === "connecting" || view.room_active;
  const bigW = 360;
  const bigX = (960 - bigW) / 2;
  control(
    widgets,
    state,
    "room_code_host",
    "HOST WITH A ROOM CODE",
    { x: bigX, y: 120, w: bigW, h: 56 },
    { disabled: roomBusy, align: "center" },
  );
  control(
    widgets,
    state,
    "room_code_join",
    "JOIN WITH A ROOM CODE",
    { x: bigX, y: 186, w: bigW, h: 56 },
    { disabled: roomBusy, align: "center" },
  );
  text(
    widgets,
    "room_code_hint",
    "A room code connects you and a peer automatically. Nothing is stored.",
    { x: 180, y: 250, w: 600, h: 36 },
    { align: "center", tone: "muted" },
  );

  // The manual offer/answer path, demoted to a quiet strip: it still works
  // (and it is the only path once a room-code attempt has failed), but a
  // room code is the primary way in, and the tab order above says so.
  text(
    widgets,
    "manual_heading",
    "OR CONNECT MANUALLY",
    { x: LEFT_X, y: 330, w: 300, h: 16 },
    { tone: "muted" },
  );
  control(
    widgets,
    state,
    "role_host",
    "HOST A SESSION",
    { x: LEFT_X, y: 352, w: 180, h: 38 },
    {
      disabled: roomBusy,
    },
  );
  control(
    widgets,
    state,
    "role_guest",
    "JOIN WITH AN OFFER",
    { x: LEFT_X + 196, y: 352, w: 180, h: 38 },
    { disabled: roomBusy },
  );
  control(
    widgets,
    state,
    "identity",
    `Join as ${displayName(view.peer_id)}`,
    { x: LEFT_X + 392, y: 352, w: 260, h: 38 },
    { disabled: roomBusy },
  );

  footerWidgets(widgets, state, view, { leaveText: "BACK" });
  return widgets;
}

// --- role + room_entry: the composer alone, plus CANCEL -----------------

function layoutComposer(state: LobbyScreenState, view: LobbyView): Layout {
  const entry = view.room_entry as RoomCodeEntry;
  const widgets: Widget[] = [];
  text(widgets, "title", "LOBBY", { x: LEFT_X, y: 20, w: 200, h: 26 }, { kind: "title" });
  text(
    widgets,
    "composer_heading",
    "ENTER ROOM CODE",
    { x: 0, y: 96, w: 960, h: 20 },
    { align: "center", kind: "eyebrow" },
  );
  const codeText = entry.chars
    .map((ch, index) => (index === entry.cursor ? `[${ch || "_"}]` : ch || "_"))
    .join(" ");
  control(
    widgets,
    state,
    ROOM_CODE_ENTRY_WIDGET,
    codeText,
    { x: 230, y: 150, w: 500, h: 64 },
    { align: "center" },
  );
  text(
    widgets,
    "composer_hint",
    "Type the code, or cycle a character with up or down and move with left or right.",
    { x: 180, y: 230, w: 600, h: 40 },
    { align: "center", tone: "muted" },
  );
  const trouble = view.error ?? view.room_error ?? "";
  if (trouble) {
    text(
      widgets,
      "trouble",
      trouble,
      { x: 180, y: 274, w: 600, h: 36 },
      { align: "center", tone: "muted" },
    );
  }
  control(
    widgets,
    state,
    "room_cancel",
    "CANCEL",
    { x: 380, y: 330, w: 200, h: 48 },
    { align: "center" },
  );
  return widgets;
}

// --- handshake: the room code as the hero (or the manual signaling it
// replaces), mode chips, bot-fill, and the footer's own START MATCH button
// (#610). The eight slot rows are cut -- ownership is unpublished, so
// every one would read "unassigned". --

function layoutHandshake(state: LobbyScreenState, view: LobbyView): Layout {
  const widgets: Widget[] = [];
  headerWidgets(widgets, state, view);
  const LX = LEFT_X;
  const LW = LEFT_COL_W;
  let ly = CONTENT_TOP;

  if (view.role === "host" && view.room_code) {
    text(
      widgets,
      "room_code_heading",
      "ROOM CODE",
      { x: LX, y: ly, w: LW, h: 18 },
      { kind: "eyebrow" },
    );
    text(
      widgets,
      "room_code_display",
      view.room_code,
      { x: LX, y: ly + 20, w: LW, h: 44 },
      { kind: "hero_title" },
    );
    ly += 20 + 44 + 8;
    // The one-click join link (#598): COPY LINK is the primary share
    // action -- a friend clicking it lands straight in this room, no code
    // to transcribe -- with SHARE beside it only where the platform offers
    // a native share sheet (`view.can_share`, a capability flag the model
    // was handed, never sniffed here). The six-character code stays the
    // hero above for the voice-chat case.
    const linkBtnW = 200;
    control(
      widgets,
      state,
      "copy_link",
      "COPY LINK",
      { x: LX, y: ly, w: linkBtnW, h: 38 },
      { align: "center" },
    );
    if (view.can_share) {
      control(
        widgets,
        state,
        "share_link",
        "SHARE",
        { x: LX + linkBtnW + 10, y: ly, w: 140, h: 38 },
        { align: "center" },
      );
    }
    ly += 46;
  } else if (view.role === "guest" && view.room_active) {
    text(
      widgets,
      "room_code_active",
      "Connected via room code.",
      { x: LX, y: ly, w: LW, h: 24 },
      { tone: "muted" },
    );
    ly += 32;
  }

  // Manual signaling is the fallback path, so it disappears entirely once a
  // room code is carrying the connection -- and comes back if that drops
  // (independent of the room-code hero above: `room_code` outlives
  // `room_active`, so after a drop both render together).
  if (view.role && !view.room_active) {
    if (view.role === "host") {
      control(
        widgets,
        state,
        "invite",
        "INVITE A PEER",
        { x: LX, y: ly, w: 200, h: 38 },
        { disabled: !view.can_invite, align: "center" },
      );
      ly += 46;
    }
    control(
      widgets,
      state,
      "copy_signal",
      "COPY INVITE",
      { x: LX, y: ly, w: 200, h: 38 },
      { disabled: !view.has_outgoing, align: "center" },
    );
    control(
      widgets,
      state,
      "paste_signal",
      "PASTE REPLY",
      { x: LX + 210, y: ly, w: 200, h: 38 },
      { align: "center" },
    );
    ly += 46;
    displayCard(
      widgets,
      "signal_out",
      inviteText(view.exported),
      { x: LX, y: ly, w: 220, h: 44 },
      {
        tone: "muted",
      },
    );
    displayCard(
      widgets,
      "signal_in",
      answerText(view.imported),
      { x: LX + 230, y: ly, w: 230, h: 44 },
      { tone: "muted" },
    );
    ly += 50;
  }

  if (view.role === "host") {
    control(
      widgets,
      state,
      "bot_fill",
      view.bot_fill ? "AI FILLS EMPTY SEATS: ON" : "AI FILLS EMPTY SEATS: OFF",
      { x: LX, y: ly, w: 220, h: 38 },
      { selected: view.bot_fill, align: "center" },
    );
    // CANCEL: back to the role screen, manual signaling included (#610
    // round-2 review, blocking finding 1e) -- #597's own acceptance
    // criterion ("the manual fallback stays reachable") now has to survive
    // an auto-hosted room too, not only the pre-role composer window.
    control(
      widgets,
      state,
      "cancel_to_role",
      "CANCEL",
      { x: LX + 230, y: ly, w: 150, h: 38 },
      { align: "center" },
    );
    ly += 42;

    // The inline "have a code?" composer (#610 round-2 review, blocking
    // finding 1c): a friend meant to join, not host -- typing a code here
    // (or anywhere on this screen, `update()`'s own doc) restarts this
    // lobby as a guest of the typed room instead, no separate front-door
    // screen involved. The placeholder IS the hint (compact: this row can
    // land as far down as the manual-signaling fallback's own worst case,
    // `role && !room_active`'s block above, with the footer fixed right
    // below it -- a separate heading line does not fit).
    const joinEntryEmpty = state.join_entry.chars.every((ch) => ch === "");
    control(
      widgets,
      state,
      JOIN_ENTRY_WIDGET,
      joinEntryEmpty ? "HAVE A CODE? TYPE IT TO JOIN INSTEAD" : roomCodeDisplay(state.join_entry),
      { x: LX, y: ly, w: 280, h: 32 },
      { align: "center" },
    );
  }

  playersStripWidgets(widgets, state, view, RIGHT_COL_X, CONTENT_TOP, RIGHT_COL_W);
  // START MATCH -- the single prominent action (#610): it locks the mode,
  // publishes ownership, marks the host ready, and begins the countdown in
  // one command. `showStart` is `view.can_configure` (host, any phase short
  // of countdown/terminal), so the same button keeps rendering -- disabled,
  // reading as "starting..." -- through the collapse's own brief manifest/
  // assigned/ready window (`layoutRoster`, below).
  footerWidgets(widgets, state, view, {
    showDetails: true,
    showStart: view.can_configure,
  });
  return widgets;
}

// --- manifest / assigned / ready: the mode-dependent roster, the players
// strip, and the same START MATCH button from `layoutHandshake` -- shown
// disabled here (`can_start` is only true in "handshake") while the
// collapse's own round trips are in flight (#610). ------------------------

function layoutRoster(state: LobbyScreenState, view: LobbyView): Layout {
  const widgets: Widget[] = [];
  headerWidgets(widgets, state, view);
  const bottom = rosterWidgets(widgets, state, view, LEFT_X, CONTENT_TOP, LEFT_COL_W);
  if (view.preference) {
    text(
      widgets,
      "preference",
      view.preference.text,
      { x: LEFT_X, y: bottom + 4, w: LEFT_COL_W, h: 20 },
      { tone: "muted" },
    );
  }
  playersStripWidgets(widgets, state, view, RIGHT_COL_X, CONTENT_TOP, RIGHT_COL_W);
  footerWidgets(widgets, state, view, {
    showStart: view.can_configure,
    showDetails: true,
  });
  return widgets;
}

// --- countdown: hero numeral, one line, LEAVE. `ceil(ticks / 60)` seconds
// -- a tick count is a protocol unit, not something a player's screen
// should ever show. ---------------------------------------------------------

function layoutCountdown(state: LobbyScreenState, view: LobbyView): Layout {
  const widgets: Widget[] = [];
  text(
    widgets,
    "countdown_heading",
    "KICKOFF IN",
    { x: 0, y: 150, w: 960, h: 20 },
    { align: "center", kind: "eyebrow" },
  );
  const seconds = view.countdown !== undefined ? Math.ceil(view.countdown / 60) : undefined;
  text(
    widgets,
    "countdown",
    seconds !== undefined ? String(seconds) : "—",
    { x: 0, y: 176, w: 960, h: 90 },
    { align: "center", kind: "hero_title" },
  );
  text(
    widgets,
    "countdown_unit",
    view.started ? "Starting now." : "seconds",
    { x: 0, y: 270, w: 960, h: 24 },
    { align: "center", tone: "muted" },
  );
  // The one exception to "hero numeral, one line, LEAVE": a departure is
  // phase-independent (`departure_text`'s own doc) and can land seconds
  // before kickoff just as easily as during handshake -- pre-#566 it
  // rendered in every state, and this is the deliberate carve-out that
  // keeps that true rather than a state a peer's screen goes silent for.
  // Rendered only when there is something to say, so the common case stays
  // a clean hero moment.
  const trouble = troubleText(view);
  if (trouble) {
    text(
      widgets,
      "trouble",
      trouble,
      { x: 0, y: 300, w: 960, h: 36 },
      { align: "center", tone: "muted" },
    );
  }
  control(
    widgets,
    state,
    "leave",
    "LEAVE LOBBY",
    { x: 380, y: 420, w: 200, h: 44 },
    { align: "center" },
  );
  return widgets;
}

// --- terminal: a real end screen. `terminal_text` is the headline, not a
// footnote; the identity dump is always here (a build/manifest mismatch
// makes it load-bearing), never behind the DETAILS toggle that gates it
// everywhere else. -----------------------------------------------------------

function layoutTerminal(state: LobbyScreenState, view: LobbyView): Layout {
  const widgets: Widget[] = [];
  const headline = view.terminal_text ?? "The session ended.";
  text(
    widgets,
    "headline",
    headline,
    { x: 0, y: 130, w: 960, h: 50 },
    { align: "center", kind: "title" },
  );
  const detail = view.terminal?.detail;
  if (detail) {
    text(
      widgets,
      "detail",
      detail,
      { x: 0, y: 186, w: 960, h: 24 },
      { align: "center", tone: "muted" },
    );
  }
  displayCard(
    widgets,
    "identity_card",
    identityLines(view),
    { x: 280, y: 230, w: 400, h: 140 },
    {
      tone: "muted",
    },
  );
  control(
    widgets,
    state,
    "leave",
    "LEAVE LOBBY",
    { x: 380, y: 460, w: 200, h: 44 },
    { align: "center" },
  );
  return widgets;
}

// --- running / result: this screen normally never renders these -- a
// `start_match` effect returns `{ go: "online_match" }` and the owner
// unmounts it -- but a defensive, minimal fallback beats a blank frame if a
// headless caller ever drives the model past countdown without following.

function layoutGeneric(state: LobbyScreenState, view: LobbyView): Layout {
  const widgets: Widget[] = [];
  text(widgets, "status", view.status, { x: 0, y: 240, w: 960, h: 24 }, { align: "center" });
  control(
    widgets,
    state,
    "leave",
    "LEAVE LOBBY",
    { x: 380, y: 460, w: 200, h: 44 },
    { align: "center" },
  );
  return widgets;
}

export function layout(state: LobbyScreenState): Layout {
  const view = lobbyView(state.ports, state.model);
  if (view.room_entry) {
    return layoutComposer(state, view);
  }
  switch (view.phase) {
    case "role":
      // A room-code guest defers coordinator creation until it learns its
      // invitation slot (#601, `lobby_model.ts`'s own header): `view.role`
      // can already be "guest" here, with `phase` still "role", for the
      // whole window between `room_joined` and the first `room_peer_signal`
      // that names a slot. `layoutRole`'s own comment assumes "role" phase
      // means no role has been chosen yet ("this whole phase stops
      // rendering the moment `view.role` is set") -- once one has, showing
      // the role picker again is a regression a phase-only switch cannot
      // see (round-2 council review on #601's own PR). `layoutHandshake`
      // already renders exactly this waiting state once a room-code
      // guest's coordinator exists (`room_code_active`, "Connected via
      // room code."); every field it reads off `view` degrades the same
      // way with none, so it is the correct screen here too, not a new one.
      return view.role !== undefined ? layoutHandshake(state, view) : layoutRole(state, view);
    case "handshake":
      return layoutHandshake(state, view);
    case "manifest":
    case "assigned":
    case "ready":
      return layoutRoster(state, view);
    case "countdown":
      return layoutCountdown(state, view);
    case "terminal":
      return layoutTerminal(state, view);
    default:
      return layoutGeneric(state, view);
  }
}

function commandFor(id: string): LobbyCommand | undefined {
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
    case "copy_link":
      return { kind: "copy_link" };
    case "share_link":
      return { kind: "share_link" };
    case "paste_signal":
      return { kind: "paste_request" };
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
    case "room_cancel":
      return { kind: "room_cancel" };
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
  resetDetails = false,
): LobbyScreenState {
  return {
    viewport: state.viewport,
    ports: state.ports,
    model,
    focus: nextFocus,
    // A cancelled room attempt falls back to the role screen, which offers
    // no DETAILS toggle at all -- so a stale `true` is invisible right up
    // until the SAME instance reaches handshake/assigned/ready again (a
    // second room-code attempt, or a manual role pick), where it would pop
    // the identity card open unasked. `room_cancel` resets it along with
    // everything else the room-code path clears on the way back.
    details: resetDetails ? false : state.details,
    join_entry: state.join_entry,
    effects,
  };
}

// --- restarting the whole lobby screen (#610 round-2 review, blocking
// finding 1) -------------------------------------------------------------
//
// Both interactions below tear the CURRENT session down gracefully (the
// exact same `"leave"` command LEAVE LOBBY itself dispatches -- an abort
// broadcast if a guest already connected, the room-code socket closed) and
// hand a `LobbyAction` up to `app.ts`'s own `restartLobby`, which mounts a
// genuinely fresh `OnlineLobby` (`ScreenStack.replace`'s own `teardown()`
// contract closes this instance's star transport) rather than trying to
// reuse this one's live star mid-flight -- switching a host into a guest,
// or a live room back into "no role chosen", is not a state this screen's
// own model was ever built to hold, and inventing that state here would
// risk exactly the orphaned-connection defect round-2 council review
// caught earlier for room-code retries (blocking finding 3 on #601's PR).

function cancelToRoleScreen(state: LobbyScreenState): readonly [LobbyScreenState, LobbyAction] {
  const [model, effects] = lobbyCommand(state.model, state.ports, { kind: "leave" });
  return [advance(state, model, state.focus, effects, true), { go: "lobby_restart" }];
}

function switchToGuest(
  state: LobbyScreenState,
  code: string,
): readonly [LobbyScreenState, LobbyAction] {
  const [model, effects] = lobbyCommand(state.model, state.ports, { kind: "leave" });
  return [
    advance(state, model, state.focus, effects, true),
    { go: "lobby_restart", intent: "guest", code },
  ];
}

export type LobbyScreenEvent =
  FocusEvent | { readonly kind: "lobby"; readonly command: LobbyCommand };

function dispatchCommand(
  state: LobbyScreenState,
  cmd: LobbyCommand,
): readonly [LobbyScreenState, LobbyAction | undefined] {
  const [model, effects] = lobbyCommand(state.model, state.ports, cmd);
  let nextState = advance(state, model, state.focus, effects, cmd.kind === "room_cancel");
  // Mirrors the click path's own `focus.ensure` below: a command dispatched
  // directly -- every network-driven event (`room_created`, `signal`,
  // `control`, `tick`, ...) arrives this way, not through a click -- can
  // change phase just as a click can. Without this, a guest whose screen
  // flips handshake -> assigned the moment the host clicks START keeps
  // focus on a widget that no longer exists until their own next input.
  nextState = {
    ...nextState,
    focus: focus.ensure(layout(nextState), nextState.focus) ?? nextState.focus,
  };
  return [nextState, actionFor(effects)];
}

export function update(
  state: LobbyScreenState,
  event: LobbyScreenEvent,
): readonly [LobbyScreenState, LobbyAction | undefined] {
  if (event.kind === "lobby") {
    return dispatchCommand(state, event.command);
  }
  // The inline "have a code?" composer only exists on the auto-hosted
  // handshake screen (#610 round-2 review, blocking finding 1c) -- see
  // `JOIN_ENTRY_WIDGET`'s own doc and `layoutHandshake`'s matching guard.
  const view = lobbyView(state.ports, state.model);
  const joinEntryAvailable = view.role === "host" && view.phase === "handshake";
  // Typing a room-code character ANYWHERE on that screen -- not only while
  // the composer widget itself is focused -- redirects focus to it and
  // feeds the keystroke through, so "just start typing" needs no prior
  // click, mirroring the multiplayer front door's own inline entry before
  // it folded into this screen.
  if (
    joinEntryAvailable &&
    state.focus !== JOIN_ENTRY_WIDGET &&
    event.kind === "key" &&
    event.pressed !== false &&
    event.key.length === 1 &&
    ROOM_CODE_ALPHABET.includes(event.key.toUpperCase())
  ) {
    return [
      { ...state, focus: JOIN_ENTRY_WIDGET, join_entry: roomCodeKey(state.join_entry, event.key) },
      undefined,
    ];
  }
  // Either composer -- the guest's own post-role one, or the host's inline
  // "switch to guest" one -- is a single focused widget with its own
  // up/down/left/right meaning (cycle/move the character under the
  // cursor) instead of the general focus-navigation those actions
  // otherwise carry. "confirm"/"back"/click fall through to the normal
  // paths below (confirm submits via the `id === ...` branches further
  // down; back/leave is universal).
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
  if (joinEntryAvailable && state.focus === JOIN_ENTRY_WIDGET) {
    if (event.kind === "key" && event.pressed !== false) {
      return [{ ...state, join_entry: roomCodeKey(state.join_entry, event.key) }, undefined];
    }
    if (event.kind === "action") {
      if (event.action === "up") {
        return [{ ...state, join_entry: roomCodeCycle(state.join_entry, 1) }, undefined];
      }
      if (event.action === "down") {
        return [{ ...state, join_entry: roomCodeCycle(state.join_entry, -1) }, undefined];
      }
      if (event.action === "left") {
        return [{ ...state, join_entry: roomCodeCursor(state.join_entry, -1) }, undefined];
      }
      if (event.action === "right") {
        return [{ ...state, join_entry: roomCodeCursor(state.join_entry, 1) }, undefined];
      }
    }
  }
  const currentLayout = layout(state);
  const nextFocus = focus.navigate(currentLayout, state.focus, event) ?? state.focus;
  if (event.kind === "action" && event.action === "back") {
    // CANCEL/back from the auto-hosted screen must still reach the role
    // screen (#597's own acceptance criterion; #610 round-2 review,
    // blocking finding 1e) -- exactly what the CANCEL widget does, so
    // reuse it rather than a parallel back-specific path.
    if (joinEntryAvailable) {
      return cancelToRoleScreen(state);
    }
    // A room-code attempt with no role yet occupies the role screen's own
    // slot -- the guest composer (`room_entry`) or a host's still-connecting
    // request (`room_active`, `role` not resolved). Back/Escape used to
    // always dispatch "leave" here, ejecting the WHOLE lobby to the title;
    // for the composer -- now the mandatory first screen a room-joining
    // intent reaches (#597) -- that left no way back to the manual role
    // screen without abandoning the lobby entirely (#566's own "room_cancel
    // is unreachable" finding, promoted from a redesign nicety to a genuine
    // trap once #597 made the composer unavoidable -- round-2 council
    // review, blocking finding 2). `room_cancel` already exists and already
    // lands safely back on the role screen (`roomCancel`, lobby_model.ts);
    // this is the one call site that was never wired to it. Once a role IS
    // resolved -- including a room-code connection that has already reached
    // one, e.g. via `room_created`/`room_joined` -- this condition is false
    // and back/Escape leaves exactly as it always did: an established
    // session is left, not silently cancelled out from under the player.
    const roomAttemptPending =
      state.model.role === undefined &&
      (state.model.room_entry !== undefined || state.model.room_active);
    const [model, effects] = lobbyCommand(
      state.model,
      state.ports,
      roomAttemptPending ? { kind: "room_cancel" } : { kind: "leave" },
    );
    let nextState = advance(state, model, nextFocus, effects, roomAttemptPending);
    // A cancelled room attempt keeps the lobby mounted (unlike "leave",
    // which exits it and makes focus moot) -- land focus on something the
    // role screen it falls back to actually offers, mirroring the general
    // click path's own `focus.ensure` below.
    if (roomAttemptPending) {
      nextState = { ...nextState, focus: focus.ensure(layout(nextState), nextFocus) ?? nextFocus };
    }
    return [nextState, actionFor(effects)];
  }
  const id = focus.activated(currentLayout, nextFocus, event);
  if (id === null) {
    return [advance(state, state.model, nextFocus, []), undefined];
  }
  // The DETAILS toggle is screen-local state (this file's own header):
  // `lobby_model.ts` never sees it, so it is handled here instead of
  // through `commandFor`/`dispatchCommand`.
  if (id === "details") {
    return [{ ...state, focus: id, details: !state.details, effects: [] }, undefined];
  }
  if (id === "cancel_to_role") {
    return cancelToRoleScreen(state);
  }
  if (id === JOIN_ENTRY_WIDGET) {
    // Mirrors `ROOM_CODE_ENTRY_WIDGET`'s own click-or-confirm-submits
    // contract: activating it (click, or confirm while focused) attempts a
    // restart; an incomplete code just focuses it for more typing, exactly
    // as `roomSubmit`'s "enter all six characters" guard does for the
    // guest's own composer.
    const code = roomCodeText(state.join_entry);
    if (code === undefined) {
      return [advance(state, state.model, id, []), undefined];
    }
    return switchToGuest(state, code);
  }
  const cmd = commandFor(id);
  if (!cmd) {
    return [advance(state, state.model, id, []), undefined];
  }
  const [model, effects] = lobbyCommand(state.model, state.ports, cmd);
  let nextState = advance(state, model, id, effects, cmd.kind === "room_cancel");
  // Focus survives a layout that no longer offers the activated control.
  nextState = { ...nextState, focus: focus.ensure(layout(nextState), id) ?? id };
  return [nextState, actionFor(effects)];
}

export const lobby = { newState, layout, update };
