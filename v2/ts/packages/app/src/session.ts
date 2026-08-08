// Ported from game/session.lua.
//
// The Lua original reads `data.teams` (specifically `teams.nebula`) at
// module load time. That table is Rust-owned (v2/README.md rule 6.7), so
// `newState`/`setStarters`/`buildRequest` take the home team's data as an
// explicit parameter instead of a bare `require`.

import type { Result } from "@gc/core";
import { matchContract, type ProductMatchRequest, type ProductMatchResult } from "./match_contract.ts";
import type { MatchContractContent, TeamData } from "./content.ts";

export type SetupStep = "squad" | "formation" | "tactic";
export type ResultAction = "rematch" | "change_plan" | "change_lineup" | "main_menu";

export interface GameSession {
  starterIds: string[];
  formationId: string;
  tacticId: string;
  setupStep: SetupStep;
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
    setupStep: "squad",
    firstMatch: true,
    matchNumber: 0,
    combatEnabled: false,
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

function routeForResult(action: ResultAction): "match" | "formation" | "squad" | "title" {
  if (action === "rematch") {
    return "match";
  } else if (action === "change_plan") {
    return "formation";
  } else if (action === "change_lineup") {
    return "squad";
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
