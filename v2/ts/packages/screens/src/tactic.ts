// Ported from game/screens/tactic.lua — choose the team's game plan before
// kickoff. See squad.ts/formation.ts's headers for the pure/impure seam and
// content-injection rationale.

import { focus, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import type { TacticData } from "./content.ts";

export interface TacticContentData {
  readonly tactics: Readonly<Record<string, TacticData>>;
}

export interface TacticScreenContext {
  readonly selected?: string;
  readonly formationId?: string;
}

export interface TacticScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly content: TacticContentData;
  readonly selected: string;
  readonly formationId: string;
  readonly focus: string;
}

// Both `tacticId` and `tactic` are carried on every transition, mirroring
// the Lua original (game/screens/tactic.lua); `tactic` is the field
// spec/screens/tactic_spec.lua reads (`action.tactic`).
export type TacticAction =
  | { readonly go: "formation"; readonly tacticId: string; readonly tactic: string }
  | { readonly go: "match"; readonly tacticId: string; readonly tactic: string };

function sortedTacticIds(tactics: Readonly<Record<string, TacticData>>): readonly string[] {
  return Object.keys(tactics).sort();
}

function newState(
  viewport: { readonly w: number; readonly h: number },
  content: TacticContentData,
  context?: TacticScreenContext,
): TacticScreenState {
  const selected = context?.selected ?? "balanced";
  return {
    viewport,
    content,
    selected,
    formationId: context?.formationId ?? "2-1-1",
    focus: `tactic_${selected}`,
  };
}

function layout(state: TacticScreenState): Layout {
  const widgets: Widget[] = [
    {
      id: "title",
      kind: "title",
      text: "PLAY THE PLAN",
      rect: { x: 0, y: 38, w: state.viewport.w, h: 34 },
      data: { align: "center" },
    },
    {
      id: "shape",
      kind: "eyebrow",
      text: `FORMATION  ${state.formationId}`,
      rect: { x: 0, y: 76, w: state.viewport.w, h: 24 },
      data: { align: "center" },
    },
  ];

  sortedTacticIds(state.content.tactics).forEach((id, i) => {
    const data = state.content.tactics[id];
    if (data === undefined) {
      return;
    }
    widgets.push({
      id: `tactic_${id}`,
      kind: "button",
      text: `${data.name.toUpperCase()}\n+ ${data.strength ?? "A clear team-wide intention."}\n− ${
        data.risk ?? "Creates a readable tradeoff."
      }`,
      selected: state.selected === id,
      focused: state.focus === `tactic_${id}`,
      rect: { x: 220, y: 124 + i * 104, w: 520, h: 86 },
      data: { align: "left" },
    });
  });

  widgets.push({
    id: "back",
    kind: "button",
    text: "BACK",
    focused: state.focus === "back",
    rect: { x: 254, y: 466, w: 190, h: 42 },
  });
  widgets.push({
    id: "kickoff",
    kind: "button",
    text: "KICK OFF",
    focused: state.focus === "kickoff",
    rect: { x: 516, y: 466, w: 190, h: 42 },
  });
  return widgets;
}

function update(state: TacticScreenState, event: FocusEvent): readonly [TacticScreenState, TacticAction | undefined] {
  const currentLayout = layout(state);
  let next: TacticScreenState = {
    ...state,
    focus: focus.navigate(currentLayout, state.focus, event) ?? state.focus,
  };
  if (event.kind === "action" && event.action === "back") {
    return [next, { go: "formation", tacticId: next.selected, tactic: next.selected }];
  }
  const id = focus.activated(currentLayout, next.focus, event);
  if (id !== null) {
    next = { ...next, focus: id };
    const tacticId = /^tactic_(.+)$/.exec(id)?.[1];
    if (tacticId !== undefined && state.content.tactics[tacticId] !== undefined) {
      next = { ...next, selected: tacticId };
    } else if (id === "back") {
      return [next, { go: "formation", tacticId: next.selected, tactic: next.selected }];
    } else if (id === "kickoff") {
      return [next, { go: "match", tacticId: next.selected, tactic: next.selected }];
    }
  }
  return [next, undefined];
}

export const tactic = { newState, layout, update };

type Widget = Layout[number];
