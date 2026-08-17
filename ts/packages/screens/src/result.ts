// Full-time summary and the hub for rematch / change-lineup /
// change-plan / main-menu. See squad.ts's header for the pure/impure seam
// and content-injection rationale.
//
// `ProductMatchResult` is `@gc/app`'s `match_contract.ts`'s shape (not this
// package's to own); it is declared locally in content.ts, structurally
// compatible, the same way `@gc/ui`'s `types.ts` declares `FocusEvent`
// locally rather than importing it.

import { focus, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import type { ProductMatchResult, TeamResultStats } from "./content.ts";

export interface ResultContentData {
  /** The slice of player data this screen reads: just the display name. */
  readonly players: Readonly<Record<string, { readonly name: string }>>;
}

export interface ResultScreenContext {
  readonly result: ProductMatchResult;
  /**
   * True when this full time ended an online session. The screen was mounted
   * twice on two routes for exactly this — so the offline rematch, which
   * replays a local session that no longer exists, could not be reached from
   * an online one. One flag does the same job: it swaps the footer's middle
   * action for a route back to the lobby.
   */
  readonly online?: boolean;
}

export interface ResultScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly content: ResultContentData;
  readonly result: ProductMatchResult;
  readonly online: boolean;
  readonly focus: string;
}

export type ResultAction = { readonly go: string };

function percent(value: number | undefined): string {
  return value !== undefined ? `${Math.floor(value * 100 + 0.5)}%` : "—";
}

function count(value: number | undefined): string {
  return value !== undefined ? String(value) : "—";
}

function newState(
  viewport: { readonly w: number; readonly h: number },
  content: ResultContentData,
  context: ResultScreenContext,
): ResultScreenState {
  return {
    viewport,
    content,
    result: context.result,
    online: context.online ?? false,
    focus: "rematch",
  };
}

function layout(state: ResultScreenState): Layout {
  const result = state.result;
  const outcome =
    result.winner === "home"
      ? "NEBULA FC WIN"
      : result.winner === "away"
        ? "ORION MINERS WIN"
        : "HONORS EVEN";
  const mvp =
    result.mvp_player_id !== undefined ? state.content.players[result.mvp_player_id] : undefined;
  const mvpName = mvp?.name ?? "No MVP awarded";
  const home: TeamResultStats = result.home_stats;
  const away: TeamResultStats = result.away_stats;
  const stats = [
    `SHOTS          ${count(home.shots)}        ${count(away.shots)}`,
    `POSSESSION     ${percent(home.possession)}       ${percent(away.possession)}`,
    `SAVES          ${count(home.saves)}        ${count(away.saves)}`,
    `PASS COMPLETE  ${percent(home.pass_completion)}       ${percent(away.pass_completion)}`,
  ];

  const widgets: Widget[] = [
    {
      id: "status",
      kind: "eyebrow",
      text: "FULL TIME",
      rect: { x: 0, y: 34, w: state.viewport.w, h: 22 },
      data: { align: "center" },
    },
    {
      id: "outcome",
      kind: "title",
      text: outcome,
      rect: { x: 0, y: 70, w: state.viewport.w, h: 38 },
      data: { align: "center" },
    },
    {
      id: "score",
      kind: "hero_title",
      text: `${result.home_score}  —  ${result.away_score}`,
      rect: { x: 0, y: 116, w: state.viewport.w, h: 54 },
      data: { align: "center" },
    },
    {
      id: "names",
      kind: "label",
      text: `${result.home_name}                                  ${result.away_name}`,
      rect: { x: 180, y: 174, w: 600, h: 22 },
      data: { align: "center", tone: "muted" },
    },
    {
      id: "stats",
      kind: "card",
      text: stats.join("\n"),
      rect: { x: 180, y: 216, w: 360, h: 158 },
      data: { focusable: false },
    },
    {
      id: "mvp",
      kind: "card",
      text: `MATCH MVP\n${mvpName}\n\n${result.mvp_summary ?? "No summary available."}`,
      rect: { x: 560, y: 216, w: 220, h: 158 },
      data: { accent: [1, 0.66, 0.24], focusable: false },
    },
  ];

  // The middle action is the only thing context changes. Offline, LINEUP and
  // PLAN both led back into the same pre-match screen once squad, formation
  // and tactic became one; they are one button now.
  const buttons: readonly [string, string][] = state.online
    ? [
        ["main_menu", "MENU"],
        ["back_to_lobby", "BACK TO LOBBY"],
        ["rematch", "REMATCH"],
      ]
    : [
        ["main_menu", "MENU"],
        ["change_plan", "CHANGE PLAN"],
        ["rematch", "REMATCH"],
      ];
  const buttonW = 200;
  const gap = 24;
  const totalW = buttons.length * buttonW + (buttons.length - 1) * gap;
  const startX = Math.round((state.viewport.w - totalW) / 2);
  buttons.forEach(([id, text], i) => {
    widgets.push({
      id,
      kind: "button",
      text,
      focused: state.focus === id,
      rect: { x: startX + i * (buttonW + gap), y: 438, w: buttonW, h: 44 },
    });
  });
  return widgets;
}

function update(
  state: ResultScreenState,
  event: FocusEvent,
): readonly [ResultScreenState, ResultAction | undefined] {
  const currentLayout = layout(state);
  let next: ResultScreenState = {
    ...state,
    focus: focus.navigate(currentLayout, state.focus, event) ?? state.focus,
  };
  if (event.kind === "action" && event.action === "back") {
    return [next, { go: "main_menu" }];
  }
  const id = focus.activated(currentLayout, next.focus, event);
  if (id !== null) {
    next = { ...next, focus: id };
    return [next, { go: id }];
  }
  return [next, undefined];
}

export const result = { newState, layout, update };

type Widget = Layout[number];
