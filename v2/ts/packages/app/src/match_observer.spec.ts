// Ported from spec/game/real_match_spec.lua's "real match observer" describe
// block -- the only part of that file about game/match_observer.lua (this
// package's). The rest of real_match_spec.lua ("real match adapter") drives
// `game.screens.real_match` and `game.screens.match`, neither of which has a
// TS port yet (see this package's porting report); left to @gc/screens.
//
// The Lua spec builds its fixture from `Match.new()` (`game/screens/match.lua`
// -> `sim/match.lua`, not yet ported/available here). `match_observer`'s
// `observe`/`observeConfirmed` only read a narrow slice of `MatchState`
// (players' id/team/is_keeper, events, owner, score) -- see match_observer.ts's
// `ObservedMatchState`. This fixture transcribes exactly that slice, using
// `sim/match.lua`'s own default construction rules: ten players (home
// indices 0-4, away indices 5-9, one keeper per side at index 0 of its
// team), and `place_kickoff`'s default kicking team ("home") for the owner
// `Match.new()` leaves in place when a test does not override it.

import { describe, expect, it } from "vitest";
import { matchObserver, type ObservedMatchState, type ObservedPlayer } from "./match_observer.ts";

function defaultPlayers(): ObservedPlayer[] {
  const players: ObservedPlayer[] = [];
  for (let i = 0; i < 5; i += 1) {
    players.push({ id: `home_${i + 1}`, team: "home", is_keeper: i === 0 });
  }
  for (let i = 0; i < 5; i += 1) {
    players.push({ id: `away_${i + 1}`, team: "away", is_keeper: i === 0 });
  }
  return players;
}

function defaultState(): ObservedMatchState {
  return {
    players: defaultPlayers(),
    events: [],
    owner: 1, // Match.new()'s default kickoff carrier: a home outfield player.
    score: { home: 0, away: 0 },
  };
}

describe("real match observer", () => {
  it("derives per-team stats and an evidence-backed MVP from match events", () => {
    const state = defaultState();
    const value = matchObserver.new(state);
    const homeId = state.players[1]?.id;
    const awayKeeperId = state.players[5]?.id;
    if (homeId === undefined || awayKeeperId === undefined) {
      throw new Error("unreachable");
    }

    state.events = [
      { kind: "pass", player: homeId },
      { kind: "shot", player: homeId },
      { kind: "parry", player: awayKeeperId },
    ];
    state.owner = 2; // a different home outfield player carries.
    matchObserver.observe(value, state, 1);
    state.score.home = 1;
    state.events = [];
    matchObserver.observe(value, state, 1);

    const summary = matchObserver.finish(value);
    expect(summary.home_stats.shots).toBe(1);
    expect(summary.home_stats.pass_completion).toBe(1);
    expect(summary.away_stats.saves).toBe(1);
    expect(summary.home_stats.possession).toBe(1);
    expect(summary.mvp_player_id).toBe(homeId);
  });

  it("can observe every event produced by a multi-tick render update", () => {
    const state = defaultState();
    const value = matchObserver.new(state);
    const homeId = state.players[1]?.id;
    if (homeId === undefined) {
      throw new Error("unreachable");
    }
    state.events = [];

    matchObserver.observe(value, state, 2 / 60, [
      { kind: "pass", player: homeId },
      { kind: "shot", player: homeId },
    ]);

    const summary = matchObserver.finish(value);
    expect(summary.home_stats.shots).toBe(1);
    expect(summary.home_stats.pass_completion).toBe(1);
  });
});
