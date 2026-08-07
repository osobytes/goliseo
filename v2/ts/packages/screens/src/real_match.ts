// Ported from game/screens/real_match.lua -- the product fixture wrapper
// around the match screen: run it to full time, then commit a result.
//
// The Lua original builds its own `MatchState` (`data.teams` lookups plus
// `sim.match`/`game.screens.match.new`) and calls straight into
// `game.match_contract.new_result` / `game.match_observer` -- three "game/"
// root modules (`match_contract.lua`, `match_observer.lua`; -> `@gc/app`,
// v2/README.md rule 6.7's content.ts header) plus this package's own
// `match.ts`. `match.ts` itself only ports the rollback-consumption seam
// this milestone (see its header): there is no working, self-constructing
// `Match` class to call `Match.new` on here. So this port goes one step
// further than the other injected-port screens and takes the constructed
// match screen itself as a parameter, alongside `MatchContractPort` and
// `MatchObserverPort` -- the same pattern `@gc/online`'s
// `match_presentation.ts` uses for `MatchDriverPort`, scaled to a third
// dependency. `RealMatch` becomes a thin, testable state machine over three
// injected collaborators rather than a constructor that reaches into
// Rust-owned or `@gc/app`-owned code directly.

import type { GameSettings, ProductMatchResult, TeamResultStats } from "./content.ts";
import type { Result } from "@gc/core";

const FULL_TIME_HOLD = 0.9;
const FULL_TIME_SKIP_DELAY = 0.25;

/**
 * `game.match_contract`'s full `ProductMatchRequest` (`contract.new_request`'s
 * return shape) -- not `content.ts`'s `ProductMatchRequest`, which is
 * deliberately narrowed to the two fields `fake_match.ts` renders. This
 * screen needs the whole request to build a result.
 */
export interface RealMatchRequest {
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

/** `game.match_contract.new_result`'s options (`ProductMatchResultOptions`). */
export interface MatchContractResultOptions {
  readonly home_team_id: string;
  readonly away_team_id: string;
  readonly home_score: number;
  readonly away_score: number;
  readonly mvp_player_id?: string;
  readonly mvp_summary?: string;
  readonly home_stats: TeamResultStats;
  readonly away_stats: TeamResultStats;
  readonly seed?: number;
}

/** `game.match_contract`, injected -- see this module's header. */
export interface MatchContractPort {
  newResult(opts: MatchContractResultOptions): Result<ProductMatchResult, string>;
}

/** `game.match_observer`'s `ObservedMatchSummary`. */
export interface ObservedMatchSummary {
  readonly home_stats: TeamResultStats;
  readonly away_stats: TeamResultStats;
  readonly mvp_player_id?: string;
  readonly mvp_summary?: string;
}

/** `game.match_observer`, injected -- see this module's header. `TState`/`TStep` stay opaque. */
export interface MatchObserverPort<TObserver, TState, TStep> {
  /** Mirrors `match_observer.new`; renamed because `new` is a reserved word. */
  create(state: TState): TObserver;
  observe(observer: TObserver, state: TState, dt: number, events?: readonly unknown[]): void;
  observeConfirmed(observer: TObserver, step: TStep): boolean;
  finish(observer: TObserver): ObservedMatchSummary;
}

/** The slice of `MatchState` this screen reads directly. */
export interface RealMatchState {
  readonly time_left: number;
  readonly score: { readonly home: number; readonly away: number };
}

/** The slice of `MatchScreen` (`match.ts`) `real_match.lua` drives -- see this module's header. */
export interface RealMatchScreenPort<TState extends RealMatchState, TStep> {
  readonly state: TState;
  /** Truthy exactly when this match is running a rollback lab (`self.match._rollback_lab`). */
  readonly rollbackLab: unknown;
  readonly rollbackConfirmedSteps: readonly TStep[];
  readonly frameEvents: readonly unknown[];
  /**
   * The caller's combat opt-in, as this match screen recorded it at
   * construction (`match.ts`'s `MatchScreen.debugCombatEnabled` /
   * `MatchScreenAsRealMatchScreen.debugCombatEnabled`). Optional -- not
   * every `RealMatchScreenPort` implementation carries a notion of combat
   * (e.g. `online_match.ts`'s `OnlineMatchState`), and this is a debug/test
   * seam, not something `RealMatchScreen`'s own control flow reads.
   */
  readonly debugCombatEnabled?: boolean;
  fullTimeConfirmed(): boolean;
  resultCompletionBlocked(): boolean;
  update(dt: number): void;
  event(evt: RealMatchInputEvent): void;
  draw(): void;
  teardown(): void;
  applySettings(settings: GameSettings): void;
}

export type RealMatchInputEvent =
  | { readonly kind: "action"; readonly action: string }
  | { readonly kind: "key"; readonly key: string };

export interface RealMatchCallbacks {
  onFinished(result: ProductMatchResult): void;
}

export class RealMatchScreen<TState extends RealMatchState, TStep, TObserver> {
  readonly match: RealMatchScreenPort<TState, TStep>;
  private readonly request: RealMatchRequest;
  private readonly callbacks: RealMatchCallbacks;
  private readonly contract: MatchContractPort;
  private readonly observerPort: MatchObserverPort<TObserver, TState, TStep>;
  private readonly observer: TObserver;
  private completed = false;
  private fullTimeElapsed = 0;

  constructor(
    match: RealMatchScreenPort<TState, TStep>,
    request: RealMatchRequest,
    callbacks: RealMatchCallbacks,
    contract: MatchContractPort,
    observerPort: MatchObserverPort<TObserver, TState, TStep>
  ) {
    this.match = match;
    this.request = request;
    this.callbacks = callbacks;
    this.contract = contract;
    this.observerPort = observerPort;
    this.observer = observerPort.create(match.state);
  }

  private complete(): void {
    if (this.completed) {
      return;
    }
    this.completed = true;
    const observed = this.observerPort.finish(this.observer);
    const state = this.match.state;
    const result = this.contract.newResult({
      home_team_id: this.request.home_team_id,
      away_team_id: this.request.away_team_id,
      home_score: state.score.home,
      away_score: state.score.away,
      home_stats: observed.home_stats,
      away_stats: observed.away_stats,
      ...(this.request.seed !== undefined ? { seed: this.request.seed } : {}),
      ...(observed.mvp_player_id !== undefined ? { mvp_player_id: observed.mvp_player_id } : {}),
      ...(observed.mvp_summary !== undefined ? { mvp_summary: observed.mvp_summary } : {}),
    });
    if (!result.ok) {
      throw new Error(result.error);
    }
    this.callbacks.onFinished(result.value);
  }

  update(dt: number): void {
    if (this.match.fullTimeConfirmed()) {
      const blocked = this.match.resultCompletionBlocked();
      this.match.update(dt);
      if (blocked || this.match.resultCompletionBlocked()) {
        return;
      }
      this.fullTimeElapsed += dt;
      if (this.fullTimeElapsed >= FULL_TIME_HOLD) {
        this.complete();
      }
      return;
    }
    const before = this.match.state.time_left;
    this.match.update(dt);
    if (this.match.rollbackLab) {
      for (const step of this.match.rollbackConfirmedSteps) {
        this.observerPort.observeConfirmed(this.observer, step);
      }
    } else {
      const elapsed = before - this.match.state.time_left;
      if (elapsed > 0) {
        this.observerPort.observe(this.observer, this.match.state, elapsed, this.match.frameEvents);
      }
    }
    if (this.match.fullTimeConfirmed()) {
      if (this.match.resultCompletionBlocked()) {
        return;
      }
      this.fullTimeElapsed += dt;
      if (this.fullTimeElapsed >= FULL_TIME_HOLD) {
        this.complete();
      }
    }
  }

  event(evt: RealMatchInputEvent): void {
    if (this.match.fullTimeConfirmed()) {
      if (this.match.resultCompletionBlocked()) {
        this.match.event(evt);
        return;
      }
      if (this.fullTimeElapsed >= FULL_TIME_SKIP_DELAY && evt.kind === "action" && evt.action === "confirm") {
        this.complete();
      }
      return;
    }
    this.match.event(evt);
  }

  draw(): void {
    this.match.draw();
  }

  teardown(): void {
    this.match.teardown();
  }

  applySettings(settings: GameSettings): void {
    this.match.applySettings(settings);
  }
}
