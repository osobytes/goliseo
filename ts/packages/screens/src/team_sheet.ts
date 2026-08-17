// The whole pre-match decision on one screen: the five, the shape, the plan.
//
// This replaces squad.ts + formation.ts + tactic.ts. Those were three views of
// a single decision, and they already referenced each other — formation drew
// the pitch preview, tactic showed the chosen formation as an eyebrow, squad
// owned the five. Splitting them across three routes hid the consequence and
// spent three titles, three BACK/NEXT footers and three transition wipes on a
// beat `docs/showcase_release.md` sells as *one* thirty-second setup.
//
// Kept from all three, deliberately:
//
//   - Focus and selection stay separate concepts (docs/visual_style.md).
//     Moving focus changes nothing until an activation.
//   - The keeper is locked and AI-controlled; it counts toward the five.
//   - Every formation and tactic keeps its one-line strength and risk, and
//     both lists are discovered from the injected content rather than named
//     here — authoring a new formation still needs no screen edit.
//
// See squad.ts's ancestor comment, preserved here: this screen needs player,
// team, formation and tactic data at `newState` time, and none of it is
// content this package may import (ARCHITECTURE.md §4 rule 6). It all arrives
// through `TeamSheetContentData` as an explicit parameter.

import { focus, invariant, type Layout } from "@gc/ui";
import type { FocusEvent, FormationMarkerData } from "@gc/ui";
import type {
  FormationData,
  PlayerData,
  PlayerPresentationIdentity,
  TacticData,
} from "./content.ts";

/** What `newState` needs: the squad, the shapes, the plans, and who is who. */
export interface TeamSheetContentData {
  readonly players: Readonly<Record<string, PlayerData>>;
  readonly identities: Readonly<Record<string, PlayerPresentationIdentity>>;
  /** `teams.nebula.squad` (falls back to `roster` when absent). */
  readonly squadIds: readonly string[];
  /** `teams.nebula.roster` — the default starting five. */
  readonly defaultStarterIds: readonly string[];
  readonly formations: Readonly<Record<string, FormationData>>;
  readonly tactics: Readonly<Record<string, TacticData>>;
  /** `teams.nebula.name`, shown so the sheet says whose it is. */
  readonly teamName: string;
}

export interface TeamSheetScreenContext {
  readonly starterIds?: readonly string[];
  readonly formationId?: string;
  readonly tacticId?: string;
  readonly combatEnabled?: boolean;
}

export interface TeamSheetScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly content: TeamSheetContentData;
  readonly roster: readonly PlayerData[];
  readonly selectedIds: readonly string[];
  readonly formationId: string;
  readonly tacticId: string;
  readonly combatEnabled: boolean;
  readonly focus: string;
  readonly message: string;
}

export type TeamSheetAction =
  | { readonly go: "title" }
  | {
      readonly go: "match";
      readonly starterIds: readonly string[];
      readonly formationId: string;
      readonly tacticId: string;
      readonly combatEnabled: boolean;
    };

/** The team sheet is complete at exactly five, keeper included. */
export const STARTER_COUNT = 5;

// --- layout geometry, in virtual (960x540) pixels ---------------------------
const MARGIN = 62;
const COL_ROSTER_X = MARGIN;
const COL_ROSTER_W = 300;
const COL_SHAPE_X = 374;
const COL_SHAPE_W = 216;
const COL_PLAN_X = 602;
const COL_PLAN_W = 296;
const HEADING_Y = 60;
const BODY_Y = 82;
// A roster row carries two lines of body copy (name/position, then the five
// stats) plus the card's own 10px top inset: 10 + 2 * 13 * 1.25 = 42.5. The
// row is 44 so the second line is not clipped.
const ROSTER_ROW_H = 44;
const ROSTER_ROW_STEP = 47;
const SHAPE_ROW_H = 30;
const SHAPE_ROW_STEP = 34;
const PREVIEW_H = 120;
const TACTIC_ROW_H = 62;
const TACTIC_ROW_STEP = 66;
const FOOTER_Y = 482;

function contains(ids: readonly string[], id: string): boolean {
  return ids.includes(id);
}

function without(ids: readonly string[], id: string): readonly string[] {
  return ids.filter((value) => value !== id);
}

function sortedIds(table: Readonly<Record<string, unknown>>): readonly string[] {
  return Object.keys(table).sort();
}

function newState(
  viewport: { readonly w: number; readonly h: number },
  content: TeamSheetContentData,
  context?: TeamSheetScreenContext,
): TeamSheetScreenState {
  const roster = content.squadIds.map((id) => {
    const player = content.players[id];
    invariant(player !== undefined, `unknown squad player: ${id}`);
    return player;
  });
  const firstPlayer = roster[0];
  invariant(firstPlayer !== undefined, "squad roster must not be empty");

  const formationId = context?.formationId ?? "2-1-1";
  invariant(content.formations[formationId] !== undefined, `missing formation: ${formationId}`);
  const tacticId = context?.tacticId ?? "balanced";
  invariant(content.tactics[tacticId] !== undefined, `missing tactic: ${tacticId}`);

  return {
    viewport,
    content,
    roster,
    selectedIds: [...(context?.starterIds ?? content.defaultStarterIds)],
    formationId,
    tacticId,
    // Combat ships on. It is a match option now, not a second Play button.
    combatEnabled: context?.combatEnabled ?? true,
    focus: `player_${firstPlayer.id}`,
    message: "Pick five. Your keeper is locked.",
  };
}

/** The one-line restatement of the whole decision, shown before it is committed. */
function summary(state: TeamSheetScreenState): string {
  const plan = state.content.tactics[state.tacticId];
  const planName = plan?.name.toUpperCase() ?? state.tacticId.toUpperCase();
  return `${planName}  •  ${state.formationId}  •  COMBAT ${state.combatEnabled ? "ON" : "OFF"}`;
}

/** Presentation marks for the chosen five, in selection order, for the pitch preview. */
function starterMarkers(state: TeamSheetScreenState): readonly FormationMarkerData[] {
  return state.selectedIds.map((id) => {
    const presentation = state.content.identities[id];
    const player = state.content.players[id];
    return {
      color: presentation?.palette ?? [0.8, 0.9, 1],
      shape: presentation?.shape ?? "round",
      name: player?.name ?? id,
    };
  });
}

function layout(state: TeamSheetScreenState): Layout {
  const widgets: Widget[] = [
    {
      id: "title",
      kind: "title",
      text: "TEAM SHEET",
      rect: { x: COL_ROSTER_X, y: 24, w: COL_ROSTER_W, h: 32 },
      data: { align: "left", focusable: false },
    },
    {
      id: "counter",
      kind: "eyebrow",
      text: `${state.selectedIds.length} / ${STARTER_COUNT} CHOSEN`,
      rect: { x: COL_SHAPE_X, y: 32, w: COL_SHAPE_W, h: 22 },
      data: { align: "left", focusable: false },
    },
    {
      id: "team",
      kind: "label",
      text: state.content.teamName,
      rect: { x: COL_PLAN_X, y: 32, w: COL_PLAN_W, h: 22 },
      data: { align: "right", tone: "muted", focusable: false },
    },
    {
      id: "roster_heading",
      kind: "eyebrow",
      text: "THE FIVE",
      rect: { x: COL_ROSTER_X, y: HEADING_Y, w: COL_ROSTER_W, h: 18 },
      data: { align: "left", focusable: false },
    },
    {
      id: "shape_heading",
      kind: "eyebrow",
      text: "THE SHAPE",
      rect: { x: COL_SHAPE_X, y: HEADING_Y, w: COL_SHAPE_W, h: 18 },
      data: { align: "left", focusable: false },
    },
    {
      id: "plan_heading",
      kind: "eyebrow",
      text: "THE PLAN",
      rect: { x: COL_PLAN_X, y: HEADING_Y, w: COL_PLAN_W, h: 18 },
      data: { align: "left", focusable: false },
    },
  ];

  // --- the five -------------------------------------------------------------
  state.roster.forEach((player, i) => {
    const presentation = state.content.identities[player.id];
    invariant(presentation !== undefined, `missing showcase identity: ${player.id}`);
    const id = `player_${player.id}`;
    const stats = player.stats;
    widgets.push({
      id,
      kind: "card",
      text: `${player.name}  •  ${player.position.toUpperCase()}\nPAC ${stats.pace}  STR ${stats.strength}  TEC ${stats.technique}  STA ${stats.stamina}  MEN ${stats.mental}`,
      selected: contains(state.selectedIds, player.id),
      focused: state.focus === id,
      rect: {
        x: COL_ROSTER_X,
        y: BODY_Y + i * ROSTER_ROW_STEP,
        w: COL_ROSTER_W,
        h: ROSTER_ROW_H,
      },
      data: {
        accent: presentation.palette,
        speciesShape: presentation.shape,
        locked: player.position === "keeper",
      },
    });
  });

  // --- the shape ------------------------------------------------------------
  const formationIds = sortedIds(state.content.formations);
  formationIds.forEach((id, i) => {
    const data = state.content.formations[id];
    invariant(data !== undefined, `missing formation: ${id}`);
    widgets.push({
      id: `formation_${id}`,
      kind: "button",
      text: `${id}  ${data.name}`,
      selected: state.formationId === id,
      focused: state.focus === `formation_${id}`,
      rect: {
        x: COL_SHAPE_X,
        y: BODY_Y + i * SHAPE_ROW_STEP,
        w: COL_SHAPE_W,
        h: SHAPE_ROW_H,
      },
      data: { align: "left" },
    });
  });

  // One permanent preview of the chosen shape, rather than one per row: this
  // is the panel that makes the formation choice legible, and it should not
  // compete with itself N times.
  const chosenShape = state.content.formations[state.formationId];
  invariant(chosenShape !== undefined, `missing formation: ${state.formationId}`);
  const previewY = BODY_Y + formationIds.length * SHAPE_ROW_STEP + 8;
  widgets.push({
    id: `preview_${state.formationId}`,
    kind: "formation_preview",
    selected: true,
    rect: { x: COL_SHAPE_X, y: previewY, w: COL_SHAPE_W, h: PREVIEW_H },
    data: {
      keeper: chosenShape.keeper,
      outfield: chosenShape.outfield,
      markers: starterMarkers(state),
    },
  });
  widgets.push({
    id: "shape_notes",
    kind: "label",
    text: `+ ${chosenShape.strength ?? "A flexible small-sided shape."}\n− ${
      chosenShape.risk ?? "Requires disciplined positioning."
    }`,
    rect: { x: COL_SHAPE_X, y: previewY + PREVIEW_H + 8, w: COL_SHAPE_W, h: 56 },
    data: { align: "left", tone: "muted", focusable: false },
  });

  // --- the plan -------------------------------------------------------------
  const tacticIds = sortedIds(state.content.tactics);
  tacticIds.forEach((id, i) => {
    const data = state.content.tactics[id];
    invariant(data !== undefined, `missing tactic: ${id}`);
    widgets.push({
      id: `tactic_${id}`,
      kind: "button",
      text: `${data.name.toUpperCase()}\n+ ${data.strength ?? "A clear team-wide intention."}\n− ${
        data.risk ?? "Creates a readable tradeoff."
      }`,
      selected: state.tacticId === id,
      focused: state.focus === `tactic_${id}`,
      rect: {
        x: COL_PLAN_X,
        y: BODY_Y + i * TACTIC_ROW_STEP,
        w: COL_PLAN_W,
        h: TACTIC_ROW_H,
      },
      data: { align: "left" },
    });
  });

  // Combat, demoted from a second Play button on the title screen to what it
  // always was: a match option, sitting with the other two.
  widgets.push({
    id: "combat",
    kind: "button",
    text: `COMBAT  ${state.combatEnabled ? "ON" : "OFF"}\nBlocks, staggers and dodges are ${
      state.combatEnabled ? "live" : "off"
    }.`,
    selected: state.combatEnabled,
    focused: state.focus === "combat",
    rect: {
      x: COL_PLAN_X,
      y: BODY_Y + tacticIds.length * TACTIC_ROW_STEP + 12,
      w: COL_PLAN_W,
      h: TACTIC_ROW_H,
    },
    data: { align: "left" },
  });

  // --- footer ---------------------------------------------------------------
  widgets.push({
    id: "message",
    kind: "label",
    text: state.message,
    // Below the longest column (eight roster rows end at 455), above the footer.
    rect: { x: COL_ROSTER_X, y: 462, w: 500, h: 18 },
    data: { align: "left", tone: "muted", focusable: false },
  });
  widgets.push({
    id: "summary",
    kind: "label",
    text: summary(state),
    rect: { x: 216, y: FOOTER_Y + 10, w: 480, h: 20 },
    data: { align: "center", tone: "muted", focusable: false },
  });
  widgets.push({
    id: "back",
    kind: "button",
    text: "BACK",
    focused: state.focus === "back",
    rect: { x: COL_ROSTER_X, y: FOOTER_Y, w: 140, h: 40 },
  });
  widgets.push({
    id: "kickoff",
    kind: "button",
    text: "KICK OFF",
    focused: state.focus === "kickoff",
    rect: { x: 718, y: FOOTER_Y, w: 180, h: 40 },
    data: { disabled: state.selectedIds.length !== STARTER_COUNT },
  });
  return widgets;
}

/** Toggle one squad member in or out of the five, with the message that explains it. */
function togglePlayer(state: TeamSheetScreenState, playerId: string): TeamSheetScreenState {
  const player = state.content.players[playerId];
  invariant(player !== undefined, `unknown squad player: ${playerId}`);
  if (player.position === "keeper") {
    return { ...state, message: "Your team sheet must keep exactly one keeper." };
  }
  if (contains(state.selectedIds, playerId)) {
    const selectedIds = without(state.selectedIds, playerId);
    return {
      ...state,
      selectedIds,
      message: `Choose ${STARTER_COUNT - selectedIds.length} more.`,
    };
  }
  if (state.selectedIds.length < STARTER_COUNT) {
    const selectedIds = [...state.selectedIds, playerId];
    return {
      ...state,
      selectedIds,
      message:
        selectedIds.length === STARTER_COUNT ? "Team sheet ready." : "Choose another starter.",
    };
  }
  return { ...state, message: `Drop someone before adding ${player.name}.` };
}

function kickOff(state: TeamSheetScreenState): TeamSheetAction {
  return {
    go: "match",
    starterIds: [...state.selectedIds],
    formationId: state.formationId,
    tacticId: state.tacticId,
    combatEnabled: state.combatEnabled,
  };
}

function update(
  state: TeamSheetScreenState,
  event: FocusEvent,
): readonly [TeamSheetScreenState, TeamSheetAction | undefined] {
  const currentLayout = layout(state);
  let next: TeamSheetScreenState = {
    ...state,
    selectedIds: [...state.selectedIds],
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
  if (id === "kickoff") {
    // `focus.clickable` already refuses the disabled button, so this guard is
    // defensive — it also covers a `confirm` action arriving while the count
    // is short.
    return next.selectedIds.length === STARTER_COUNT ? [next, kickOff(next)] : [next, undefined];
  }
  if (id === "combat") {
    return [{ ...next, combatEnabled: !next.combatEnabled }, undefined];
  }

  const formationId = (/^formation_(.+)$/.exec(id) ?? /^preview_(.+)$/.exec(id))?.[1];
  if (formationId !== undefined && state.content.formations[formationId] !== undefined) {
    return [{ ...next, formationId, focus: `formation_${formationId}` }, undefined];
  }

  const tacticId = /^tactic_(.+)$/.exec(id)?.[1];
  if (tacticId !== undefined && state.content.tactics[tacticId] !== undefined) {
    return [{ ...next, tacticId }, undefined];
  }

  const playerId = /^player_(.+)$/.exec(id)?.[1];
  if (playerId !== undefined) {
    return [togglePlayer(next, playerId), undefined];
  }
  return [next, undefined];
}

export const teamSheet = { newState, layout, update };

// Local alias so `layout`'s widget literals type-check against `@gc/ui`'s
// `Widget` without importing it twice under two names.
type Widget = Layout[number];
