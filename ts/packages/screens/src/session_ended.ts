// Where an online session goes when it dies.
//
// `lobby_model.ts` has emitted typed terminal and departure reasons all along
// — `CoordinatorTerminalReason`, with `TERMINAL_TEXT`/`DEPARTURE_TEXT` giving
// each one plain language — and nothing rendered them. An online session that
// died dropped the player back at the title screen with an error string
// stored on the app object and shown nowhere.
//
// A dead session is a dead end, not a notification, which is why this is a
// screen and not a toast: the player needs somewhere to go, and both exits
// should be one keypress away.
//
// The typed reason stays visible in the detail strip. It is useful in a bug
// report and invisible to everyone else; the headline above it is the only
// thing a player has to read.

import { focus, theme, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import { TERMINAL_TEXT, type CoordinatorTerminalReason } from "./lobby_model.ts";

export interface SessionEndedScreenContext {
  /**
   * The coordinator's typed reason. Kept as a plain string rather than the
   * union so a reason this screen has never heard of still renders instead of
   * throwing — an unknown reason is a worse bug report than a generic one,
   * but a blank screen is worse than both.
   */
  readonly reason: string;
  /**
   * Plain language for that reason. `App` passes `LOBBY_TERMINAL_TEXT[reason]`
   * or the lobby's own departure text; when absent this screen falls back to
   * `TERMINAL_TEXT` and then to a generic line.
   */
  readonly text?: string;
  /** Whatever the transport or protocol attached, verbatim. */
  readonly detail?: string;
  /** Ticks elapsed when the session died, for the detail strip. */
  readonly tick?: number;
  /** Last measured round trip in milliseconds, for the detail strip. */
  readonly rttMs?: number;
}

export interface SessionEndedScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly reason: string;
  readonly headline: string;
  readonly consequence: string;
  readonly detail: string;
  readonly focus: string;
}

export type SessionEndedAction = { readonly go: "main_menu" } | { readonly go: "multiplayer" };

/**
 * Room for three lines of title type: two line advances plus one glyph box.
 * Derived from the theme rather than eyeballed, so a font-size change moves
 * the box with it.
 */
const HEADLINE_H = Math.ceil(2 * theme.fonts.title * 1.25 + theme.fonts.title * 1.2);

/**
 * Reasons that mean the match was abandoned rather than played out. Only these
 * get the "not recorded" sentence — saying it after a completed session would
 * be false.
 */
const ABANDONED: readonly string[] = [
  "local_abort",
  "peer_abort",
  "guest_left",
  "host_left",
  "removed",
  "transport_lost",
  "start_ack_timeout",
];

/** Reasons that mean the two peers disagreed, rather than one of them leaving. */
const DISAGREEMENT: readonly string[] = [
  "protocol_violation",
  "manifest_mismatch",
  "build_mismatch",
  "invalid_assignment",
  "late_input",
  "input_channel_failure",
];

function consequenceFor(reason: string): string {
  if (reason === "completed") {
    return "The session finished cleanly. You can open a new lobby whenever you like.";
  }
  if (ABANDONED.includes(reason)) {
    return "Results are not recorded when a session ends early.";
  }
  if (DISAGREEMENT.includes(reason)) {
    return "The two peers stopped agreeing on the match, so it could not continue. Nothing was recorded.";
  }
  return "The session cannot continue. Nothing was recorded.";
}

function headlineFor(context: SessionEndedScreenContext): string {
  if (context.text !== undefined && context.text.length > 0) {
    return context.text;
  }
  const known = TERMINAL_TEXT[context.reason as CoordinatorTerminalReason];
  return known ?? "The online session ended.";
}

/** The mono strip: typed reason first, then whatever measurements exist. */
function detailFor(context: SessionEndedScreenContext): string {
  const parts: string[] = [context.reason];
  if (context.detail !== undefined && context.detail.length > 0) {
    parts.push(context.detail);
  }
  if (context.tick !== undefined) {
    parts.push(`tick ${context.tick}`);
  }
  if (context.rttMs !== undefined) {
    parts.push(`last rtt ${context.rttMs} ms`);
  }
  return parts.join("   •   ");
}

function newState(
  viewport: { readonly w: number; readonly h: number },
  context: SessionEndedScreenContext,
): SessionEndedScreenState {
  return {
    viewport,
    reason: context.reason,
    headline: headlineFor(context),
    consequence: consequenceFor(context.reason),
    detail: detailFor(context),
    focus: "new_lobby",
  };
}

function layout(state: SessionEndedScreenState): Layout {
  return [
    {
      id: "status",
      kind: "eyebrow",
      text: "SESSION ENDED",
      rect: { x: 0, y: 118, w: state.viewport.w, h: 22 },
      data: { align: "center", focusable: false },
    },
    {
      id: "headline",
      kind: "title",
      text: state.headline,
      // Three lines of title type. `build_mismatch`'s headline ("The peers are
      // running different builds. Install the same build on both.") already
      // needs two at this width; a 40px box clipped it. Sized for three so the
      // next authored reason cannot re-break it silently, and `printCentred`
      // keeps a one-line headline centred rather than stranded at the top.
      rect: { x: 130, y: 146, w: 700, h: HEADLINE_H },
      data: { align: "center", focusable: false },
    },
    {
      id: "consequence",
      kind: "label",
      text: state.consequence,
      rect: { x: 230, y: 246, w: 500, h: 44 },
      data: { align: "center", tone: "muted", focusable: false },
    },
    {
      id: "detail",
      kind: "card",
      text: state.detail,
      rect: { x: 230, y: 302, w: 500, h: 48 },
      data: { align: "center", tone: "muted", focusable: false },
    },
    {
      id: "main_menu",
      kind: "button",
      text: "MAIN MENU",
      focused: state.focus === "main_menu",
      rect: { x: 268, y: 384, w: 200, h: 42 },
    },
    {
      id: "new_lobby",
      kind: "button",
      text: "NEW LOBBY",
      focused: state.focus === "new_lobby",
      rect: { x: 492, y: 384, w: 200, h: 42 },
    },
  ];
}

function update(
  state: SessionEndedScreenState,
  event: FocusEvent,
): readonly [SessionEndedScreenState, SessionEndedAction | undefined] {
  const currentLayout = layout(state);
  const next: SessionEndedScreenState = {
    ...state,
    focus: focus.navigate(currentLayout, state.focus, event) ?? state.focus,
  };
  if (event.kind === "action" && event.action === "back") {
    return [next, { go: "main_menu" }];
  }
  const id = focus.activated(currentLayout, next.focus, event);
  if (id === "main_menu") {
    return [{ ...next, focus: id }, { go: "main_menu" }];
  }
  if (id === "new_lobby") {
    return [{ ...next, focus: id }, { go: "multiplayer" }];
  }
  return [next, undefined];
}

export const sessionEnded = { newState, layout, update };
