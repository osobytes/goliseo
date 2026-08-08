// Ported from game/screens/squad.lua — pick the five outfielders (plus the
// locked keeper) for the showcase team sheet. See AGENTS.md §9: layout/update
// stay pure, no love.graphics/three.js/DOM.
//
// The Lua original reads `data.players`, `data.teams`, and
// `render.identity` at module load time. All three are content this package
// must not import (v2/README.md rule 6.7 — `data/**` is Rust-owned;
// `render/identity.lua` is `@gc/render`, not a dependency of this package).
// `SquadContentData` threads them through as an explicit `newState`
// parameter instead; see content.ts's header for the full rationale.

import { focus, invariant, type Layout } from "@gc/ui";
import type { FocusEvent } from "@gc/ui";
import type { PlayerData, PlayerPresentationIdentity } from "./content.ts";

/** What `newState` needs from `data.players` / `data.teams` / `render.identity`. */
export interface SquadContentData {
  readonly players: Readonly<Record<string, PlayerData>>;
  readonly identities: Readonly<Record<string, PlayerPresentationIdentity>>;
  /** `teams.nebula.squad` (falls back to `roster` in the Lua original when absent). */
  readonly squadIds: readonly string[];
  /** `teams.nebula.roster` — the default starting five. */
  readonly defaultStarterIds: readonly string[];
}

export interface SquadScreenContext {
  readonly starterIds?: readonly string[];
}

export interface SquadScreenState {
  readonly viewport: { readonly w: number; readonly h: number };
  readonly content: SquadContentData;
  readonly roster: readonly PlayerData[];
  readonly selectedIds: readonly string[];
  readonly focus: string;
  readonly message: string;
}

export type SquadAction = { readonly go: "title" } | { readonly go: "formation"; readonly starterIds: readonly string[] };

function contains(ids: readonly string[], id: string): boolean {
  return ids.includes(id);
}

function without(ids: readonly string[], id: string): readonly string[] {
  return ids.filter((value) => value !== id);
}

function newState(
  viewport: { readonly w: number; readonly h: number },
  content: SquadContentData,
  context?: SquadScreenContext,
): SquadScreenState {
  const roster = content.squadIds.map((id) => {
    const player = content.players[id];
    invariant(player !== undefined, `unknown squad player: ${id}`);
    return player;
  });
  const firstPlayer = roster[0];
  invariant(firstPlayer !== undefined, "squad roster must not be empty");
  const selected = context?.starterIds ?? content.defaultStarterIds;
  return {
    viewport,
    content,
    roster,
    selectedIds: [...selected],
    focus: `player_${firstPlayer.id}`,
    message: "Select four outfielders. Ozzo is your locked keeper.",
  };
}

function layout(state: SquadScreenState): Layout {
  const widgets: Widget[] = [
    {
      id: "title",
      kind: "title",
      text: "PICK THE FIVE",
      rect: { x: 0, y: 22, w: state.viewport.w, h: 34 },
      data: { align: "center" },
    },
    {
      id: "counter",
      kind: "eyebrow",
      text: `${state.selectedIds.length} / 5 STARTERS`,
      rect: { x: 0, y: 60, w: state.viewport.w, h: 22 },
      data: { align: "center" },
    },
  ];

  state.roster.forEach((player, i) => {
    const presentation = state.content.identities[player.id];
    invariant(presentation !== undefined, `missing showcase identity: ${player.id}`);
    const col = i % 2;
    const row = Math.floor(i / 2);
    const id = `player_${player.id}`;
    widgets.push({
      id,
      kind: "card",
      text: `${player.name}  •  ${presentation.species_name} ${player.position.toUpperCase()}\nPAC ${player.stats.pace}  STR ${player.stats.strength}  TEC ${player.stats.technique}  STA ${player.stats.stamina}  MEN ${player.stats.mental}\n${presentation.tagline}`,
      selected: contains(state.selectedIds, player.id),
      focused: state.focus === id,
      rect: { x: 62 + col * 424, y: 96 + row * 86, w: 390, h: 74 },
      data: {
        accent: presentation.palette,
        speciesShape: presentation.shape,
        locked: player.position === "keeper",
      },
    });
  });

  widgets.push({
    id: "message",
    kind: "label",
    text: state.message,
    rect: { x: 120, y: 448, w: 720, h: 22 },
    data: { align: "center", tone: "muted" },
  });
  widgets.push({
    id: "back",
    kind: "button",
    text: "BACK",
    focused: state.focus === "back",
    rect: { x: 254, y: 482, w: 190, h: 40 },
  });
  widgets.push({
    id: "next",
    kind: "button",
    text: "SET THE SHAPE",
    focused: state.focus === "next",
    rect: { x: 516, y: 482, w: 190, h: 40 },
    data: { disabled: state.selectedIds.length !== 5 },
  });
  return widgets;
}

function update(state: SquadScreenState, event: FocusEvent): readonly [SquadScreenState, SquadAction | undefined] {
  const currentLayout = layout(state);
  let next: SquadScreenState = {
    ...state,
    selectedIds: [...state.selectedIds],
    focus: focus.navigate(currentLayout, state.focus, event) ?? state.focus,
  };
  if (event.kind === "action" && event.action === "back") {
    return [next, { go: "title" }];
  }

  const id = focus.activated(currentLayout, next.focus, event);
  if (id !== null) {
    next = { ...next, focus: id };
  }
  if (id === "back") {
    return [next, { go: "title" }];
  } else if (id === "next" && next.selectedIds.length === 5) {
    return [next, { go: "formation", starterIds: [...next.selectedIds] }];
  }

  const playerMatch = id !== null ? /^player_(.+)$/.exec(id) : null;
  const playerId = playerMatch?.[1];
  if (playerId !== undefined) {
    const player = state.content.players[playerId];
    invariant(player !== undefined, `unknown squad player: ${playerId}`);
    if (player.position === "keeper") {
      next = { ...next, message: "Your team sheet must keep exactly one keeper." };
    } else if (contains(next.selectedIds, playerId)) {
      const selectedIds = without(next.selectedIds, playerId);
      next = {
        ...next,
        selectedIds,
        message: `Choose ${5 - selectedIds.length} more starter.`,
      };
    } else if (next.selectedIds.length < 5) {
      const selectedIds = [...next.selectedIds, playerId];
      next = {
        ...next,
        selectedIds,
        message: selectedIds.length === 5 ? "Team sheet ready." : "Choose another starter.",
      };
    } else {
      next = { ...next, message: `Deselect an outfielder before adding ${player.name}.` };
    }
  }
  return [next, undefined];
}

export const squad = { newState, layout, update };

// Local alias so `layout`'s widget literals type-check against `@gc/ui`'s
// `Widget` without importing it twice under two names.
type Widget = Layout[number];
