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
import { matchObserver, type MatchObserver, type ObservedMatchEvent, type ObservedMatchState, type ObservedPlayer } from "./match_observer.ts";
import { MatchScreen, MatchScreenAsRealMatchScreen, RealMatchScreen } from "@gc/screens";
import type {
  MatchContractPort,
  MatchObserverPort,
  ProductMatchResult,
  RealMatchEventKind,
  RealMatchRequest,
  RealMatchState,
  RenderFrame,
  RenderFrameRoster,
  RenderPort,
  SimHostFactory,
  SimHostPort,
} from "@gc/screens";
import type { KeyboardState } from "@gc/input";

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

// =============================================================================
// THE PORT-WIDENING GAP. `real_match.ts`'s `RealMatchScreenPort.state` used
// to carry only `time_left`/`score`, so `RealMatchScreen` (`@gc/screens`)
// could never hand `match_observer.ts`'s `observe` a real roster/owner/event
// stream -- every `ProductMatchResult.home_stats`/`away_stats` a real match
// produced came out zero, silently, because the CONTRACT could not carry
// the data, not because the observer's own arithmetic was wrong (the specs
// above already prove that arithmetic). `real_match.ts`'s `RealMatchState`
// now carries `roster`/`owner` too, and `@gc/screens`'s
// `MatchScreenAsRealMatchScreen` sources both (plus `frameEvents`) from a
// `SimHostPort`'s `roster()`/`frame().possession`/`frame().events` -- see
// that class's and `MatchScreen.matchObservationState`'s own docs.
//
// This block proves the WIDENED CONTRACT actually carries real data end to
// end: a hand-scripted `SimHostPort` (the same "TS-glue-observable analog"
// pattern `@gc/screens`'s own `match_screen.spec.ts` uses in place of a real
// wasm-compiled `gc-sim`) stands in for a real match, driven tick by tick
// through the REAL `MatchScreen`/`MatchScreenAsRealMatchScreen`/
// `RealMatchScreen` classes, into an `observerPort` that forwards straight
// to this package's own real `matchObserver` (no canned stats -- the
// observer computes them from the scripted roster/events exactly as it
// would from a real one). `real_match_factory.ts` (owned by another agent
// this wave, out of this batch's file ownership) still wires its own
// `observerPort` to a `{players: [], events: [], score}` stub -- closing
// that last seam is that file's job, not this one's; what this block proves
// is that the contract and `MatchScreenAsRealMatchScreen` no longer force
// zeros the way `real_match_factory.ts`'s header currently documents.
// =============================================================================

/** One simulated tick's scripted outcome. */
interface ScriptedTick {
  readonly events?: readonly ObservedMatchEvent[];
  /** One-based roster slot, matching the real wire's `RenderFramePossession.owner`. */
  readonly ownerSlot?: number;
  readonly homeScore?: number;
  readonly awayScore?: number;
  readonly finished?: boolean;
}

const SCRIPTED_TICK_SECONDS = 1 / 60;

/** `ids`/`teams`/`is_keeper`, ten players -- the same default roster shape `defaultPlayers()` above uses, restated as a `RenderFrameRoster` (the wire shape `SimHostPort.roster()` returns). */
function scriptedRoster(): RenderFrameRoster {
  const ids: string[] = [];
  const teams: ("home" | "away")[] = [];
  const isKeeper: boolean[] = [];
  for (let i = 0; i < 5; i += 1) {
    ids.push(`home_${i + 1}`);
    teams.push("home");
    isKeeper.push(i === 0);
  }
  for (let i = 0; i < 5; i += 1) {
    ids.push(`away_${i + 1}`);
    teams.push("away");
    isKeeper.push(i === 0);
  }
  return { ids, teams, is_keeper: isKeeper };
}

/**
 * A hand-scripted `SimHostPort`: each `step()` call consumes the next
 * `ScriptedTick`, applying it to `hud`/`possession`/`events` exactly as a
 * real `gc-sim` wasm session would report it on `frame()`. `planTicks`
 * ignores `dt` entirely (ticks are driven by the script, not by wall time)
 * -- the same simplification `match_screen.spec.ts`'s own `FakeSimHost`
 * documents for its purpose, just simpler because this suite does not need
 * to prove catch-up/drop behavior, only that scripted per-tick data reaches
 * `match_observer.ts` intact.
 */
class ScriptedSimHost implements SimHostPort {
  private readonly roster_: RenderFrameRoster;
  private readonly script: readonly ScriptedTick[];
  private cursor = 0;
  private tickCount = 0;
  private readonly hud: {
    finished: boolean;
    controlled_owns_ball: boolean;
    home_score: number;
    away_score: number;
    time_left: number;
  } = { finished: false, controlled_owns_ball: false, home_score: 0, away_score: 0, time_left: 300 };
  private possession: { owner?: number; owner_team?: "home" | "away" } = {};
  private eventsThisTick: {
    count: number;
    kind: readonly RealMatchEventKind[];
    slot: readonly (number | undefined)[];
  } = {
    count: 0,
    kind: [],
    slot: [],
  };

  constructor(roster: RenderFrameRoster, script: readonly ScriptedTick[]) {
    this.roster_ = roster;
    this.script = script;
  }

  planTicks(): number {
    return this.cursor < this.script.length ? 1 : 0;
  }

  step(): void {
    const tick = this.script[this.cursor];
    if (tick === undefined) {
      return;
    }
    this.cursor += 1;
    this.tickCount += 1;
    this.hud.time_left = Math.max(0, this.hud.time_left - SCRIPTED_TICK_SECONDS);
    if (tick.homeScore !== undefined) {
      this.hud.home_score = tick.homeScore;
    }
    if (tick.awayScore !== undefined) {
      this.hud.away_score = tick.awayScore;
    }
    if (tick.finished === true) {
      this.hud.finished = true;
    }
    this.possession = tick.ownerSlot !== undefined ? { owner: tick.ownerSlot } : {};
    const events = tick.events ?? [];
    this.eventsThisTick = {
      count: events.length,
      kind: events.map((event) => event.kind),
      // This suite's `ObservedMatchEvent` fixtures carry a roster ID
      // directly (`event.player`), not a slot -- recover the one-based slot
      // `RenderFrameEvents.slot` would carry on the real wire so
      // `MatchScreen.appendObservedFrameEvents` exercises the exact same
      // id-recovery path a real host's frame does.
      slot: events.map((event) => this.slotFor(event.player)),
    };
  }

  /** The one-based roster slot for a player id, or `undefined` if unset/unknown -- mirrors the real wire's `RenderFrameEvents.slot`. */
  private slotFor(playerId: string | undefined): number | undefined {
    if (playerId === undefined) {
      return undefined;
    }
    const index = this.roster_.ids?.indexOf(playerId);
    return index !== undefined && index >= 0 ? index + 1 : undefined;
  }

  cancelPlannedTicks(): void {}

  frame(): RenderFrame {
    return { hud: this.hud, possession: this.possession, events: this.eventsThisTick };
  }

  roster(): RenderFrameRoster {
    return this.roster_;
  }

  tick(): number {
    return this.tickCount;
  }

  dispose(): void {}
}

const noopRenderer: RenderPort = { draw: (): void => {} };
const noopKeyboard: KeyboardState = { isDown: (): boolean => false };

/**
 * The real fix `real_match_factory.ts` still owes (see this block's header):
 * forward `RealMatchState.roster`/`.owner`/the raw `frameEvents` straight
 * into this package's real `matchObserver`, instead of the
 * `{players: [], events: [], score}` stub that file currently hard-codes.
 * Written here, inline, so this suite proves the WIDENED CONTRACT actually
 * carries what this adapter needs -- not a fake asserting canned numbers.
 */
function realObserverPort(): MatchObserverPort<MatchObserver, RealMatchState, never> {
  const toObservedState = (state: RealMatchState): ObservedMatchState => ({
    players: (state.roster ?? []).map((entry) => ({ id: entry.id, team: entry.team, is_keeper: entry.is_keeper })),
    events: [],
    ...(state.owner !== undefined ? { owner: state.owner } : {}),
    score: { home: state.score.home, away: state.score.away },
  });
  return {
    create: (state) => matchObserver.new(toObservedState(state)),
    observe: (observer, state, dt, events) =>
      matchObserver.observe(observer, toObservedState(state), dt, (events ?? []) as readonly ObservedMatchEvent[]),
    observeConfirmed: () => false,
    finish: (observer) => matchObserver.finish(observer),
  };
}

const fakeContractPort: MatchContractPort = {
  newResult: (opts) => ({
    ok: true,
    value: {
      home_score: opts.home_score,
      away_score: opts.away_score,
      home_name: "Home",
      away_name: "Away",
      winner: opts.home_score === opts.away_score ? "draw" : opts.home_score > opts.away_score ? "home" : "away",
      ...(opts.mvp_player_id !== undefined ? { mvp_player_id: opts.mvp_player_id } : {}),
      ...(opts.mvp_summary !== undefined ? { mvp_summary: opts.mvp_summary } : {}),
      home_stats: opts.home_stats,
      away_stats: opts.away_stats,
    } satisfies ProductMatchResult,
  }),
};

const FIXTURE_REQUEST: RealMatchRequest = {
  home_team_id: "home_team",
  away_team_id: "away_team",
  home_starter_ids: [],
  formation_id: "1-2-1",
  tactic_id: "balanced",
  arena_id: "arena_1",
  show_onboarding: false,
  combat_enabled: false,
};

describe("real match screen port, widened", () => {
  it("carries a real roster/owner/event stream through to a non-zero ProductMatchResult", () => {
    const roster = scriptedRoster();
    // home_2 passes to home_3, home_3 shoots; away_1 (keeper) catches, then
    // passes to away_2, who shoots and scores -- the same shape of sequence
    // `match_observer.spec.ts`'s own hand-built-state tests above already
    // prove the arithmetic for, driven here through the real screen classes
    // instead of a direct `matchObserver.observe` call.
    const script: ScriptedTick[] = [
      { events: [{ kind: "pass", player: "home_2" }], ownerSlot: 3 },
      { events: [{ kind: "shot", player: "home_3" }] },
      { events: [{ kind: "catch", player: "away_1" }], ownerSlot: 6 },
      { events: [{ kind: "pass", player: "away_1" }], ownerSlot: 7 },
      { events: [{ kind: "shot", player: "away_2" }], awayScore: 1, finished: true },
    ];
    const host = new ScriptedSimHost(roster, script);
    const createHost: SimHostFactory = (): SimHostPort => host;

    const matchScreen = new MatchScreen({ createHost, renderer: noopRenderer, keyboard: noopKeyboard });
    const port = new MatchScreenAsRealMatchScreen(matchScreen);

    let finishedResult: ProductMatchResult | undefined;
    const realMatchScreen = new RealMatchScreen(
      port,
      FIXTURE_REQUEST,
      { onFinished: (result) => (finishedResult = result) },
      fakeContractPort,
      realObserverPort(),
    );

    for (let i = 0; i < script.length; i += 1) {
      realMatchScreen.update(SCRIPTED_TICK_SECONDS);
    }
    // Cross the full-time hold (`RealMatchScreen`'s `FULL_TIME_HOLD = 0.9`)
    // now that the scripted match is finished -- see this block's header.
    realMatchScreen.update(1.0);

    if (finishedResult === undefined) {
      throw new Error("expected the scripted match to reach a result");
    }
    expect(finishedResult.away_score).toBe(1);
    expect(finishedResult.home_stats.shots).toBe(1);
    expect(finishedResult.away_stats.shots).toBe(1);
    expect(finishedResult.away_stats.saves).toBe(1);
    expect(finishedResult.home_stats.pass_completion).toBe(1);
    expect(finishedResult.away_stats.pass_completion).toBe(1);
    expect(finishedResult.home_stats.possession ?? 0).toBeGreaterThan(0);
    expect(finishedResult.away_stats.possession ?? 0).toBeGreaterThan(0);
    // away_2 passed the ball on to no one, shot, and scored (2 + 5 = 7
    // involvement) -- strictly ahead of every other scripted contributor
    // (away_1's catch + pass is 3 + 1 = 4; home_2/home_3 each touched the
    // ball once, worth 1 and 2 respectively).
    expect(finishedResult.mvp_player_id).toBe("away_2");
  });

  it("reports MatchScreenAsRealMatchScreen.state.roster/.owner and .frameEvents live off the host, not just at completion", () => {
    const roster = scriptedRoster();
    const script: ScriptedTick[] = [{ events: [{ kind: "tackle", player: "home_4" }], ownerSlot: 4 }];
    const host = new ScriptedSimHost(roster, script);
    const createHost: SimHostFactory = (): SimHostPort => host;
    const matchScreen = new MatchScreen({ createHost, renderer: noopRenderer, keyboard: noopKeyboard });
    const port = new MatchScreenAsRealMatchScreen(matchScreen);

    // Before any tick has run, the host's own roster/possession are already
    // live-readable (matches `MatchScreen.hud`'s "never cached" rule).
    expect(port.state.roster?.length).toBe(10);
    expect(port.state.roster?.[0]).toEqual({ id: "home_1", team: "home", is_keeper: true });
    expect(port.frameEvents).toEqual([]);

    port.update(SCRIPTED_TICK_SECONDS);

    expect(port.state.owner).toBe(3); // one-based slot 4 -> 0-based index 3
    expect(port.frameEvents).toEqual([{ kind: "tackle", player: "home_4" }]);
  });
});
