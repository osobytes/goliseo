// Team data (specifically the home team, `teams.nebula`) is Rust-owned
// (ARCHITECTURE.md §4 rule 6), so `newState`/`setStarters`/`buildRequest` take
// the home team's data as an explicit parameter instead of importing it.

import type { Result } from "@gc/core";
import {
  matchContract,
  type ProductMatchRequest,
  type ProductMatchResult,
} from "./match_contract.ts";
import type { MatchContractContent, TeamData } from "./content.ts";

export type ResultAction = "rematch" | "change_plan" | "main_menu";

export interface GameSession {
  starterIds: string[];
  formationId: string;
  tacticId: string;
  lastResult?: ProductMatchResult;
  firstMatch: boolean;
  matchNumber: number;
  combatEnabled: boolean;
}

function newState(homeTeam: TeamData): GameSession {
  return {
    starterIds: [...homeTeam.roster],
    formationId: homeTeam.formation,
    tacticId: "balanced",
    firstMatch: true,
    matchNumber: 0,
    // Combat ships on. It stopped being a hidden prototype behind a second
    // Play button and became a visible toggle on the team sheet.
    combatEnabled: true,
  };
}

function setStarters(
  content: Pick<MatchContractContent, "players">,
  state: GameSession,
  homeTeam: TeamData,
  ids: readonly string[],
): Result<true, string> {
  const validated = matchContract.validateStarters(content, ids, homeTeam);
  if (!validated.ok) {
    return validated;
  }
  state.starterIds = [...ids];
  return validated;
}

function setFormation(state: GameSession, formationId: string): void {
  state.formationId = formationId;
}

function setTactic(state: GameSession, tacticId: string): void {
  state.tacticId = tacticId;
}

function setCombatEnabled(state: GameSession, enabled: boolean): void {
  state.combatEnabled = enabled;
}

function buildRequest(
  content: MatchContractContent,
  state: GameSession,
  seed?: number,
): Result<ProductMatchRequest, string> {
  return matchContract.newRequest(content, {
    home_team_id: "nebula",
    away_team_id: "orion",
    home_starter_ids: state.starterIds,
    formation_id: state.formationId,
    tactic_id: state.tacticId,
    arena_id: "helios_crown",
    show_onboarding: state.firstMatch,
    combat_enabled: state.combatEnabled,
    ...(seed !== undefined ? { seed } : {}),
  });
}

function recordResult(state: GameSession, result: ProductMatchResult): void {
  state.lastResult = result;
  state.firstMatch = false;
  state.matchNumber += 1;
}

function routeForResult(action: ResultAction): "match" | "team_sheet" | "title" {
  if (action === "rematch") {
    return "match";
  } else if (action === "change_plan") {
    return "team_sheet";
  }
  return "title";
}

export const session = {
  new: newState,
  setStarters,
  setFormation,
  setTactic,
  setCombatEnabled,
  buildRequest,
  recordResult,
  routeForResult,
};
