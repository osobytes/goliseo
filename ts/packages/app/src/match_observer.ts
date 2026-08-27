// `sim.fixed_clock.TICK_SECONDS` (Rust-owned, ARCHITECTURE.md §4 rule 6) is
// not read directly here. Rather than duplicate that constant in this
// module, `observeConfirmed` takes it as an explicit `tickSeconds`
// parameter — the caller (which does own a real fixed clock, once
// wasm-compiled `gc-sim` is wired up) supplies it.

import type { TeamResultStats } from "./match_contract.ts";

export type MatchTeam = "home" | "away";

/** The slice of the sim layer's `MatchPlayer` this module reads. */
export interface ObservedPlayer {
  readonly id: string;
  readonly team: MatchTeam;
  readonly is_keeper: boolean;
}

/**
 * The slice of the sim layer's `MatchState` `observer.observe` reads.
 * Mutable, mirroring the real `MatchState` table this stands in for --
 * the real-match screen mutates `events`/`owner`/`score` in place every
 * tick, and this module only reads them, never owns them.
 */
export interface ObservedMatchState {
  readonly players: readonly ObservedPlayer[];
  events: readonly ObservedMatchEvent[];
  owner?: number;
  score: { home: number; away: number };
}

export type ObservedEventKind =
  | "pass"
  | "shot"
  | "header"
  | "volley"
  | "bicycle"
  | "catch"
  | "parry"
  | "tackle"
  | "block"
  | "claim"
  | "juke"
  | "reception"
  | "first_touch_shot";

export interface ObservedMatchEvent {
  readonly kind: ObservedEventKind;
  readonly player?: string;
}

/** `sim.rollback_events`'s `RollbackWrappedEvent`, narrowed to a match-domain payload (Rust-owned; opaque beyond this). */
export interface RollbackWrappedMatchEvent {
  readonly id: string;
  readonly payload: ObservedMatchEvent;
}

/** The slice of `sim.rollback_events`'s `RollbackEventStep` this module reads (Rust-owned). */
export interface ObservedConfirmedStep {
  readonly tick: number;
  readonly match_events: readonly RollbackWrappedMatchEvent[];
  readonly state: {
    readonly owner_team?: MatchTeam;
    readonly score: { readonly home: number; readonly away: number };
  };
}

export interface ObservedMatchSummary {
  readonly home_stats: TeamResultStats;
  readonly away_stats: TeamResultStats;
  readonly mvp_player_id?: string;
  readonly mvp_summary?: string;
}

interface TeamTotals {
  home: number;
  away: number;
}

export interface MatchObserver {
  readonly teamOf: ReadonlyMap<string, MatchTeam>;
  readonly keeper: ReadonlyMap<string, boolean>;
  readonly playerOrder: readonly string[];
  readonly shots: TeamTotals;
  readonly saves: TeamTotals;
  readonly passes: TeamTotals;
  readonly completedPasses: TeamTotals;
  readonly possession: TeamTotals;
  readonly involvement: Map<string, number>;
  pendingPass?: MatchTeam | undefined;
  readonly lastShooter: { home?: string; away?: string };
  readonly previousScore: { home: number; away: number };
  readonly confirmedEventIds: Set<string>;
  lastConfirmedTick: number;
}

function award(values: Map<string, number>, id: string | undefined, amount: number): void {
  if (id !== undefined) {
    values.set(id, (values.get(id) ?? 0) + amount);
  }
}

function newObserver(state: ObservedMatchState): MatchObserver {
  const teamOf = new Map<string, MatchTeam>();
  const keeper = new Map<string, boolean>();
  const playerOrder: string[] = [];
  const involvement = new Map<string, number>();
  for (const player of state.players) {
    teamOf.set(player.id, player.team);
    keeper.set(player.id, player.is_keeper);
    playerOrder.push(player.id);
    involvement.set(player.id, 0);
  }
  return {
    teamOf,
    keeper,
    playerOrder,
    shots: { home: 0, away: 0 },
    saves: { home: 0, away: 0 },
    passes: { home: 0, away: 0 },
    completedPasses: { home: 0, away: 0 },
    possession: { home: 0, away: 0 },
    involvement,
    lastShooter: {},
    previousScore: { home: state.score.home, away: state.score.away },
    confirmedEventIds: new Set(),
    lastConfirmedTick: -1,
  };
}

function eventTeam(state: ObservedMatchState, event: ObservedMatchEvent): MatchTeam | undefined {
  if (event.player === undefined) {
    return undefined;
  }
  for (const player of state.players) {
    if (player.id === event.player) {
      return player.team;
    }
  }
  return undefined;
}

function observe(
  value: MatchObserver,
  state: ObservedMatchState,
  dt: number,
  events?: readonly ObservedMatchEvent[],
): void {
  for (const event of events ?? state.events) {
    const team = eventTeam(state, event);
    if (event.kind === "pass" && team !== undefined) {
      value.passes[team] += 1;
      value.pendingPass = team;
      award(value.involvement, event.player, 1);
    } else if (
      (event.kind === "shot" ||
        event.kind === "header" ||
        event.kind === "volley" ||
        event.kind === "bicycle" ||
        event.kind === "first_touch_shot") &&
      team !== undefined &&
      event.player !== undefined &&
      value.keeper.get(event.player) !== true
    ) {
      value.shots[team] += 1;
      value.lastShooter[team] = event.player;
      award(value.involvement, event.player, 2);
    } else if ((event.kind === "catch" || event.kind === "parry") && team !== undefined) {
      value.saves[team] += 1;
      award(value.involvement, event.player, 3);
    } else if (
      event.kind === "tackle" ||
      event.kind === "block" ||
      event.kind === "claim" ||
      event.kind === "juke" ||
      event.kind === "reception"
    ) {
      award(value.involvement, event.player, 1);
    }
  }

  const owner = state.owner !== undefined ? state.players[state.owner] : undefined;
  if (owner) {
    value.possession[owner.team] += dt;
    if (value.pendingPass !== undefined) {
      if (owner.team === value.pendingPass) {
        value.completedPasses[owner.team] += 1;
      }
      value.pendingPass = undefined;
    }
  }

  if (state.score.home > value.previousScore.home) {
    award(value.involvement, value.lastShooter.home, 5);
  }
  if (state.score.away > value.previousScore.away) {
    award(value.involvement, value.lastShooter.away, 5);
  }
  value.previousScore.home = state.score.home;
  value.previousScore.away = state.score.away;
}

function observeConfirmedEvent(value: MatchObserver, event: RollbackWrappedMatchEvent): void {
  if (value.confirmedEventIds.has(event.id)) {
    return;
  }
  value.confirmedEventIds.add(event.id);
  const payload = event.payload;
  const team = payload.player !== undefined ? value.teamOf.get(payload.player) : undefined;
  if (payload.kind === "pass" && team !== undefined) {
    value.passes[team] += 1;
    value.pendingPass = team;
    award(value.involvement, payload.player, 1);
  } else if (
    (payload.kind === "shot" ||
      payload.kind === "header" ||
      payload.kind === "volley" ||
      payload.kind === "bicycle" ||
      payload.kind === "first_touch_shot") &&
    team !== undefined &&
    payload.player !== undefined &&
    value.keeper.get(payload.player) !== true
  ) {
    value.shots[team] += 1;
    value.lastShooter[team] = payload.player;
    award(value.involvement, payload.player, 2);
  } else if ((payload.kind === "catch" || payload.kind === "parry") && team !== undefined) {
    value.saves[team] += 1;
    award(value.involvement, payload.player, 3);
  } else if (
    payload.kind === "tackle" ||
    payload.kind === "block" ||
    payload.kind === "claim" ||
    payload.kind === "juke" ||
    payload.kind === "reception"
  ) {
    award(value.involvement, payload.player, 1);
  }
}

// Rollback matches publish only newly confirmed, immutable steps. This path is
// boundary-addressed and idempotent; it never reads the speculative live state.
function observeConfirmed(
  value: MatchObserver,
  step: ObservedConfirmedStep,
  tickSeconds: number,
): boolean {
  if (step.tick <= value.lastConfirmedTick) {
    return false;
  }
  if (step.tick !== value.lastConfirmedTick + 1) {
    throw new Error("confirmed observer steps must be contiguous");
  }
  for (const event of step.match_events) {
    observeConfirmedEvent(value, event);
  }

  const ownerTeam = step.state.owner_team;
  if (ownerTeam !== undefined) {
    value.possession[ownerTeam] += tickSeconds;
    if (value.pendingPass !== undefined) {
      if (ownerTeam === value.pendingPass) {
        value.completedPasses[ownerTeam] += 1;
      }
      value.pendingPass = undefined;
    }
  }
  if (step.state.score.home > value.previousScore.home) {
    award(value.involvement, value.lastShooter.home, 5);
  }
  if (step.state.score.away > value.previousScore.away) {
    award(value.involvement, value.lastShooter.away, 5);
  }
  value.previousScore.home = step.state.score.home;
  value.previousScore.away = step.state.score.away;
  value.lastConfirmedTick = step.tick;
  return true;
}

function mvpId(value: MatchObserver): string | undefined {
  let bestId: string | undefined;
  let bestScore = 0;
  for (const id of value.playerOrder) {
    const score = value.involvement.get(id) ?? 0;
    if (score > bestScore) {
      bestId = id;
      bestScore = score;
    }
  }
  return bestId;
}

function share(value: number, total: number): number | undefined {
  return total > 0 ? value / total : undefined;
}

function completion(value: number, total: number): number | undefined {
  return total > 0 ? value / total : undefined;
}

function finish(value: MatchObserver): ObservedMatchSummary {
  const owned = value.possession.home + value.possession.away;
  const mvp = mvpId(value);
  const homePossession = share(value.possession.home, owned);
  const awayPossession = share(value.possession.away, owned);
  const homeCompletion = completion(value.completedPasses.home, value.passes.home);
  const awayCompletion = completion(value.completedPasses.away, value.passes.away);
  return {
    home_stats: {
      shots: value.shots.home,
      ...(homePossession !== undefined ? { possession: homePossession } : {}),
      saves: value.saves.home,
      ...(homeCompletion !== undefined ? { pass_completion: homeCompletion } : {}),
    },
    away_stats: {
      shots: value.shots.away,
      ...(awayPossession !== undefined ? { possession: awayPossession } : {}),
      saves: value.saves.away,
      ...(awayCompletion !== undefined ? { pass_completion: awayCompletion } : {}),
    },
    ...(mvp !== undefined ? { mvp_player_id: mvp } : {}),
    ...(mvp !== undefined
      ? { mvp_summary: "Recorded the fixture's strongest all-around contribution." }
      : {}),
  };
}

export const matchObserver = { new: newObserver, observe, observeConfirmed, finish };
