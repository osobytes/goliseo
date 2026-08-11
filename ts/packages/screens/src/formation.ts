// Choose the team shape for the selected starting five. See AGENTS.md §9
// and squad.ts's header for the pure/impure seam and the content-injection
// rationale (ARCHITECTURE.md §4 rule 6): formation data, player data, and
// presentation identity come in as an explicit `FormationContentData`
// parameter instead of a module-level import.

import { focus, invariant, type Layout } from "@gc/ui";
import type { FocusEvent, FormationMarkerData } from "@gc/ui";
import type { FormationData, PlayerPresentationIdentity } from "./content.ts";

export interface FormationContentData {
  readonly formations: Readonly<Record<string, FormationData>>;
  /** The slice of player data this screen reads: just the display name. */
  readonly players: Readonly<Record<string, { readonly name: string }>>;
  readonly identities: Readonly<Record<string, PlayerPresentationIdentity>>;
}

export interface FormationScreenContext {
  readonly selected?: string;
  readonly starterIds?: readonly string[];
}

export interface FormationScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly content: FormationContentData;
  readonly selected: string;
  readonly starterIds: readonly string[];
  readonly focus: string;
}

// The selection is carried as both `formationId` and `formation` on every
// transition action; `formation` is the field formation.spec.ts actually
// reads (`action.formation`), so both are kept rather than collapsed into
// one name.
export type FormationAction =
  | { readonly go: "squad"; readonly formationId: string; readonly formation: string }
  | { readonly go: "tactic"; readonly formationId: string; readonly formation: string };

function sortedFormationIds(
  formations: Readonly<Record<string, FormationData>>,
): readonly string[] {
  return Object.keys(formations).sort();
}

function newState(
  viewport: { readonly w: number; readonly h: number },
  content: FormationContentData,
  context?: FormationScreenContext,
): FormationScreenState {
  const selected = context?.selected ?? "2-1-1";
  invariant(content.formations[selected] !== undefined, `missing formation: ${selected}`);
  return {
    viewport,
    content,
    selected,
    starterIds: [...(context?.starterIds ?? [])],
    focus: `formation_${selected}`,
  };
}

function layout(state: FormationScreenState): Layout {
  const widgets: Widget[] = [
    {
      id: "title",
      kind: "title",
      text: "SET THE SHAPE",
      rect: { x: 0, y: 34, w: state.viewport.w, h: 34 },
      data: { align: "center" },
    },
  ];

  const markers: FormationMarkerData[] = [];
  const lineupNames: string[] = [];
  for (const id of state.starterIds) {
    const player = state.content.players[id];
    const presentation = state.content.identities[id];
    if (player !== undefined) {
      lineupNames.push(player.name.toUpperCase());
    }
    markers.push({
      color: presentation?.palette ?? [0.8, 0.9, 1],
      shape: presentation?.shape ?? "round",
      name: player?.name ?? id,
    });
  }
  widgets.push({
    id: "lineup",
    kind: "label",
    text: lineupNames.join("  •  "),
    rect: { x: 80, y: 72, w: 800, h: 18 },
    data: { align: "center", tone: "muted" },
  });

  sortedFormationIds(state.content.formations).forEach((id, i) => {
    const data = state.content.formations[id];
    invariant(data !== undefined, `missing formation: ${id}`);
    const y = 96 + i * 112;
    widgets.push({
      id: `formation_${id}`,
      kind: "button",
      text: `${id}  ${data.name}\n+ ${data.strength ?? "A flexible small-sided shape."}\n− ${
        data.risk ?? "Requires disciplined positioning."
      }`,
      selected: state.selected === id,
      focused: state.focus === `formation_${id}`,
      rect: { x: 150, y, w: 410, h: 94 },
      data: { align: "left" },
    });
    widgets.push({
      id: `preview_${id}`,
      kind: "formation_preview",
      selected: state.selected === id,
      rect: { x: 580, y, w: 230, h: 94 },
      data: {
        keeper: data.keeper,
        outfield: data.outfield,
        markers,
      },
    });
  });

  widgets.push({
    id: "back",
    kind: "button",
    text: "BACK",
    focused: state.focus === "back",
    rect: { x: 254, y: 472, w: 190, h: 42 },
  });
  widgets.push({
    id: "next",
    kind: "button",
    text: "CHOOSE TACTIC",
    focused: state.focus === "next",
    rect: { x: 516, y: 472, w: 190, h: 42 },
  });
  return widgets;
}

function update(
  state: FormationScreenState,
  event: FocusEvent,
): readonly [FormationScreenState, FormationAction | undefined] {
  const currentLayout = layout(state);
  let next: FormationScreenState = {
    ...state,
    starterIds: [...state.starterIds],
    focus: focus.navigate(currentLayout, state.focus, event) ?? state.focus,
  };
  if (event.kind === "action" && event.action === "back") {
    return [next, { go: "squad", formationId: next.selected, formation: next.selected }];
  }
  const id = focus.activated(currentLayout, next.focus, event);
  if (id !== null) {
    next = { ...next, focus: id };
    const formationId = (/^formation_(.+)$/.exec(id) ?? /^preview_(.+)$/.exec(id))?.[1];
    if (formationId !== undefined && state.content.formations[formationId] !== undefined) {
      next = { ...next, selected: formationId, focus: `formation_${formationId}` };
    } else if (id === "back") {
      return [next, { go: "squad", formationId: next.selected, formation: next.selected }];
    } else if (id === "next") {
      return [next, { go: "tactic", formationId: next.selected, formation: next.selected }];
    }
  }
  return [next, undefined];
}

export const formation = { newState, layout, update };

type Widget = Layout[number];
