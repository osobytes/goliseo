// Arena/formation/player/tactic/team data is Rust-owned (ARCHITECTURE.md §4
// rule 6), so every function here takes a `MatchContractContent` bundle as its
// first explicit parameter instead of importing those tables.

import { type Result, err, ok } from "@gc/core";
import type { MatchContractContent, TeamData } from "./content.ts";

export type MatchWinner = "home" | "away" | "draw";

export interface ProductMatchRequest {
  readonly home_team_id: string;
  readonly away_team_id: string;
  readonly home_starter_ids: readonly string[];
  readonly formation_id: string;
  readonly tactic_id: string;
  readonly arena_id: string;
  readonly show_onboarding: boolean;
  readonly combat_enabled: boolean;
  readonly seed?: number;
}

export interface TeamResultStats {
  readonly shots?: number;
  readonly possession?: number;
  readonly saves?: number;
  readonly pass_completion?: number;
}

export interface ProductMatchResult {
  readonly home_team_id: string;
  readonly away_team_id: string;
  readonly home_name: string;
  readonly away_name: string;
  readonly home_score: number;
  readonly away_score: number;
  readonly winner: MatchWinner;
  readonly mvp_player_id?: string;
  readonly mvp_summary?: string;
  readonly home_stats: TeamResultStats;
  readonly away_stats: TeamResultStats;
  readonly seed?: number;
}

export interface ProductMatchRequestOptions {
  readonly home_team_id: string;
  readonly away_team_id: string;
  readonly home_starter_ids: readonly string[];
  readonly formation_id: string;
  readonly tactic_id: string;
  readonly arena_id?: string;
  readonly show_onboarding?: boolean;
  readonly combat_enabled?: boolean;
  readonly seed?: number;
}

export interface ProductMatchResultOptions {
  readonly home_team_id: string;
  readonly away_team_id: string;
  readonly home_score: number;
  readonly away_score: number;
  readonly mvp_player_id?: string;
  readonly mvp_summary?: string;
  readonly home_stats?: TeamResultStats;
  readonly away_stats?: TeamResultStats;
  readonly seed?: number;
}

function eligibleIds(team: TeamData): ReadonlySet<string> {
  return new Set(team.squad ?? team.roster);
}

export function validateStarters(
  content: Pick<MatchContractContent, "players">,
  ids: readonly string[],
  team: TeamData,
): Result<true, string> {
  if (ids.length !== 5) {
    return err("a team sheet needs exactly five starters");
  }

  const eligible = eligibleIds(team);
  const seen = new Set<string>();
  let keepers = 0;
  for (const id of ids) {
    if (seen.has(id)) {
      return err("starter ids must be unique");
    }
    seen.add(id);
    if (!eligible.has(id)) {
      return err(`player is not eligible for ${team.name}: ${id}`);
    }
    const player = content.players[id];
    if (!player) {
      return err(`unknown player id: ${id}`);
    }
    if (player.position === "keeper") {
      keepers += 1;
    }
  }
  if (keepers !== 1) {
    return err("a team sheet needs exactly one keeper");
  }
  return ok(true);
}

export function newRequest(
  content: MatchContractContent,
  opts: ProductMatchRequestOptions,
): Result<ProductMatchRequest, string> {
  const home = content.teams[opts.home_team_id];
  if (!home) {
    return err(`unknown home team: ${String(opts.home_team_id)}`);
  }
  if (!content.teams[opts.away_team_id]) {
    return err(`unknown away team: ${String(opts.away_team_id)}`);
  }
  if (!content.formations[opts.formation_id]) {
    return err(`unknown formation: ${String(opts.formation_id)}`);
  }
  if (!content.tactics[opts.tactic_id]) {
    return err(`unknown tactic: ${String(opts.tactic_id)}`);
  }
  const arenaId = opts.arena_id ?? "helios_crown";
  if (!content.arenas[arenaId]) {
    return err(`unknown arena: ${String(arenaId)}`);
  }
  const validated = validateStarters(content, opts.home_starter_ids, home);
  if (!validated.ok) {
    return validated;
  }

  return ok({
    home_team_id: opts.home_team_id,
    away_team_id: opts.away_team_id,
    home_starter_ids: [...opts.home_starter_ids],
    formation_id: opts.formation_id,
    tactic_id: opts.tactic_id,
    arena_id: arenaId,
    show_onboarding: opts.show_onboarding === true,
    combat_enabled: opts.combat_enabled === true,
    ...(opts.seed !== undefined ? { seed: opts.seed } : {}),
  });
}

function validRatio(value: number | undefined): boolean {
  return value === undefined || (value >= 0 && value <= 1);
}

function validCount(value: number | undefined): boolean {
  return value === undefined || (value >= 0 && value === Math.floor(value));
}

function validateStats(stats: TeamResultStats): Result<true, string> {
  if (!validCount(stats.shots)) {
    return err("shots must be a non-negative integer");
  }
  if (!validCount(stats.saves)) {
    return err("saves must be a non-negative integer");
  }
  if (!validRatio(stats.possession)) {
    return err("possession must be between zero and one");
  }
  if (!validRatio(stats.pass_completion)) {
    return err("pass completion must be between zero and one");
  }
  return ok(true);
}

export function newResult(
  content: { readonly teams: Readonly<Record<string, Pick<TeamData, "id" | "name">>> },
  opts: ProductMatchResultOptions,
): Result<ProductMatchResult, string> {
  const home = content.teams[opts.home_team_id];
  const away = content.teams[opts.away_team_id];
  if (!home || !away) {
    return err("match result references an unknown team");
  }
  if (
    !validCount(opts.home_score) ||
    opts.home_score === undefined ||
    !validCount(opts.away_score) ||
    opts.away_score === undefined
  ) {
    return err("scores must be non-negative integers");
  }

  const homeStats = opts.home_stats ?? {};
  const awayStats = opts.away_stats ?? {};
  const homeValidated = validateStats(homeStats);
  if (!homeValidated.ok) {
    return homeValidated;
  }
  const awayValidated = validateStats(awayStats);
  if (!awayValidated.ok) {
    return awayValidated;
  }

  let winner: MatchWinner = "draw";
  if (opts.home_score > opts.away_score) {
    winner = "home";
  } else if (opts.away_score > opts.home_score) {
    winner = "away";
  }

  return ok({
    home_team_id: home.id,
    away_team_id: away.id,
    home_name: home.name,
    away_name: away.name,
    home_score: opts.home_score,
    away_score: opts.away_score,
    winner,
    ...(opts.mvp_player_id !== undefined ? { mvp_player_id: opts.mvp_player_id } : {}),
    ...(opts.mvp_summary !== undefined ? { mvp_summary: opts.mvp_summary } : {}),
    home_stats: homeStats,
    away_stats: awayStats,
    ...(opts.seed !== undefined ? { seed: opts.seed } : {}),
  });
}

export const matchContract = { validateStarters, newRequest, newResult };
