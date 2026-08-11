// OnlineMatch: the impure half of the online match screen -- it owns the
// OMP-3 driver, borrows the lobby's star link, drives the shipped combat
// match renderer through the renderer's own drive seam, and draws one
// extra online overlay.
//
// Every decision it makes is delegated: the driver owns simulation and
// authority, `MatchPresentationPort` owns the stable event timeline, and
// `online_match_model.ts` owns session policy. This file translates input,
// executes effects, and reports facts.
//
// `MatchDriverPort`, `MatchSessionPort`, `MatchLobbyLinkPort`,
// `MatchContractPort`, and `MatchObserverPort` are all Rust-owned or
// `@gc/app`/`@gc/online`-owned (ARCHITECTURE.md §1.1; content.ts's header),
// none a declared dependency of this package, so all are injected ports,
// following the pattern used throughout this package (`real_match.ts`'s
// `MatchContractPort`/`MatchObserverPort`, `lobby_model.ts`'s
// `CoordinatorPort`). `match.ts` itself only implements the
// rollback-consumption seam this milestone (see its header) -- there is no
// working, self-constructing match class to build here either, so (like
// `real_match.ts`) this module takes the constructed match screen as a
// parameter. `combat` (`@gc/presentation`) and `theme` (`@gc/ui`) *are*
// declared dependencies and are used directly.

import { combat, type CombatPresentationData, type CombatMatchState } from "@gc/presentation";
import { theme } from "@gc/ui";
import type { Vec2 } from "@gc/core";
import type { GameSettings, ProductMatchResult, TeamData } from "./content.ts";
import type { MatchContractPort, ObservedMatchSummary, RealMatchInputEvent, RealMatchScreenPort, RealMatchState } from "./real_match.ts";
import type { OnlineHostPort, RenderFrame, RenderFrameRoster } from "./match.ts";

/**
 * The slice of `sim.match`'s `MatchState` this screen reads directly.
 * Broader than `@gc/presentation`'s `MatchState` (which is scoped to what
 * `combat.ts`'s projection reads) and than `real_match.ts`'s
 * `RealMatchState` (scoped to the full-time hold) -- this screen also
 * reads `controlled`, `owner`, and each player's `team`/`id`/`pos`/`facing`
 * (the last two so `state` is structurally a `@gc/presentation`
 * `MatchState` too, for `combat.model`'s sake) for its status overlay.
 */
export interface OnlineMatchState extends RealMatchState {
  readonly controlled: number;
  readonly owner?: number;
  readonly players: readonly { readonly id: string; readonly team: "home" | "away"; readonly pos: Vec2; readonly facing: Vec2 }[];
}

export interface OnlineMatchRequest {
  readonly role: "host" | "guest";
  readonly peer_id: string;
  readonly mode: string;
  readonly freeze: unknown;
  readonly manifest: unknown;
  readonly initial_snapshot: unknown;
  readonly first_input_tick: number;
  readonly home: TeamData;
  readonly away: TeamData;
  readonly arena: { readonly id: string };
  readonly seed?: number;
  readonly combat_enabled?: boolean;
  readonly live?: string;
  readonly owned: readonly string[];
}

/** `game.online.match_driver`, injected -- see this module's header. `TDriver`/`TBatch` opaque. */
export interface MatchDriverPort<TDriver, TBatch, TSnapshot, TCheckpoint> {
  /** Mirrors `match_driver.new`; renamed because `new` is a reserved word. */
  create(options: {
    readonly role: "host" | "guest";
    readonly peer_id: string;
    readonly freeze: unknown;
    readonly manifest: unknown;
    readonly transport: unknown;
    readonly initial_snapshot: unknown;
  }): TDriver;
  status(driver: TDriver): string;
  advance(driver: TDriver, sample: unknown): TBatch;
  currentSnapshot(driver: TDriver): TSnapshot;
  snapshot(driver: TDriver, boundary: number): unknown;
  terminal(driver: TDriver): { readonly status: string; readonly failure?: string; readonly detail?: string } | undefined;
  settled(driver: TDriver): boolean;
  diagnostics(driver: TDriver): {
    readonly transport_tick: number;
    readonly present_input_tick: number;
    readonly confirmed_output_tick: number;
    readonly rollback_count: number;
    readonly correction_count: number;
    readonly predicted_slot_samples: number;
    readonly status: string;
  };
  observeCheckpoint(driver: TDriver, tick: number, hash: string): void;
  batchControl(batch: TBatch): readonly unknown[];
  batchCheckpoints(batch: TBatch): readonly TCheckpoint[];
  batchLive(batch: TBatch): Readonly<Record<string, string>>;
  /**
   * The render-facing surface `driverSource()` now also exposes (via
   * `OnlineHostPort`, `match.ts`) so `MatchScreen`'s ONLINE construction
   * mode can draw a live online frame -- see that file's "THE
   * ONLINE-DRIVEN MATCH SCREEN" section header. Mirrors `SimHostPort`'s own
   * `frame`/`roster`/`tick`/`dispose` quartet, parametrized over `driver`
   * the same way every other `MatchDriverPort` method already is.
   */
  frame(driver: TDriver): RenderFrame;
  roster(driver: TDriver): RenderFrameRoster;
  tick(driver: TDriver): number;
  dispose(driver: TDriver): void;
}

/** `game.online.match_presentation`, injected -- see this module's header. */
export interface MatchPresentationPort<TPresentation, TDriver, TBatch, TPresented> {
  /** Mirrors `match_presentation.new`; renamed because `new` is a reserved word. */
  create(initialSnapshot: unknown, firstInputTick: number): TPresentation;
  consume(presentation: TPresentation, driver: TDriver, batch: TBatch): TPresented;
  presentedOutputs(presented: TPresented): readonly unknown[];
  presentedEventDiffs(presented: TPresented): readonly unknown[];
  presentedConfirmedSteps(presented: TPresented): readonly unknown[];
  presentedCorrections(presented: TPresented): readonly unknown[];
}

/** `game.online.match_session.player_index`, injected -- see this module's header. */
export interface MatchSessionPort<TState> {
  playerIndex(state: TState, live: string): number | undefined;
}

/** `game.online.lobby_link`'s `LobbyLink`, as used by the match screen (control + star polling). */
export interface MatchLobbyLinkPort {
  readonly star: {
    pollBatch(): readonly { readonly channel: string; readonly peer_id: string; readonly message: { readonly payload: string } }[];
    pollEvent(): { readonly kind: string; readonly peer_id?: string; readonly state?: string } | undefined;
  };
  send(linkId: string, wire: string): void;
  apply(effect: { readonly kind: string; readonly link_id?: string }): void;
}

export interface LobbyFrameBuffer {
  readonly _opaque?: never;
}

/** `game.online.lobby_link.new_buffer`/`.absorb`, injected. */
export interface LobbyFramingPort {
  newBuffer(): LobbyFrameBuffer;
  absorb(buffer: LobbyFrameBuffer, payload: string): readonly [string | undefined, string | undefined];
}

export interface OnlineMatchOptions<TDriver, TBatch, TSnapshot, TCheckpoint, TPresentation, TPresented, TCoordinator> {
  readonly request: OnlineMatchRequest;
  readonly coordinator: TCoordinator;
  readonly link: MatchLobbyLinkPort;
  readonly onAction?: (action: { readonly go: string; readonly [key: string]: unknown }) => void;
  readonly matchDriver: MatchDriverPort<TDriver, TBatch, TSnapshot, TCheckpoint>;
  readonly matchPresentation: MatchPresentationPort<TPresentation, TDriver, TBatch, TPresented>;
  readonly matchSession: MatchSessionPort<OnlineMatchState>;
  readonly lobbyFraming: LobbyFramingPort;
  readonly contract: MatchContractPort;
  readonly observer: {
    /** Mirrors `match_observer.new`; renamed because `new` is a reserved word. */
    create(state: OnlineMatchState): unknown;
    observeConfirmed(observer: unknown, step: unknown): boolean;
    finish(observer: unknown): ObservedMatchSummary;
  };
  readonly newMatch: (opts: {
    readonly home: TeamData;
    readonly away: TeamData;
    readonly arenaId: string;
    readonly seed: number | undefined;
    readonly combatEnabled: boolean | undefined;
    readonly initialSnapshot: unknown;
    /**
     * The seam `match.ts`'s ONLINE construction mode consumes
     * (`MatchScreenOptions.online`) -- see `OnlineHostPort`'s doc and this
     * file's own `driverSource()`. Was `() => unknown` (opaque) before
     * `match.ts` had an online-driven mode to consume it at all; widened
     * now that it does.
     */
    readonly rollbackSource: () => OnlineHostPort;
  }) => RealMatchScreenPort<OnlineMatchState, unknown>;
}

export interface OnlineMatchDispatchEvent {
  readonly kind: string;
  readonly [key: string]: unknown;
}

/** `online_match_model.ts`'s `command`, injected as a black box here -- this module only routes its effects. */
export interface OnlineMatchModelPort<TModel> {
  command(model: TModel, event: OnlineMatchDispatchEvent): readonly [TModel, readonly OnlineMatchEffect[]];
  ended(model: TModel): boolean;
  exitRoute(model: TModel): "result" | "terminal";
  readonly ABORT_PROMPT: string;
}

export type OnlineMatchEffect =
  | { readonly kind: "send"; readonly link_id: string; readonly wire: string }
  | { readonly kind: "close"; readonly link_id: string; readonly detail?: string }
  | { readonly kind: "observe_hash"; readonly tick: number; readonly hash: string };

const TICK_SECONDS = 1 / 60;

export class OnlineMatch<TDriver, TBatch, TSnapshot, TCheckpoint, TPresentation, TPresented, TCoordinator, TModel> {
  private readonly ports: OnlineMatchOptions<TDriver, TBatch, TSnapshot, TCheckpoint, TPresentation, TPresented, TCoordinator>;
  private readonly modelPort: OnlineMatchModelPort<TModel>;
  private readonly request: OnlineMatchRequest;
  private readonly link: MatchLobbyLinkPort;
  private readonly onAction: ((action: { readonly go: string; readonly [key: string]: unknown }) => void) | undefined;
  private driver!: TDriver;
  private presentation!: TPresentation;
  model: TModel;
  match: RealMatchScreenPort<OnlineMatchState, unknown>;
  private observerValue: unknown;
  private live: Record<string, string>;
  private buffers = new Map<string, LobbyFrameBuffer>();
  private control: { readonly channel: string; readonly peer_id: string; readonly message: { readonly payload: string } }[] = [];
  private checkpoints: TCheckpoint[] = [];
  // The most recent boundary hash this peer's own driver has observed, if
  // any -- `TCheckpoint` is opaque to this class (see `MatchDriverPort`'s
  // own doc), but every real implementation carries a `hash` field
  // (`online_match_model.ts`'s "checkpoints" command case already reads
  // `checkpoint.hash` structurally). Used by `reportDriver` as the final
  // hash a completed match reports -- the coordinator's own `matchFinish`
  // requires a real, canonical 16-character hash
  // (`gc_netcode::protocol::validate_hash`), so reporting an empty string
  // here was never going to reach an agreed result against the real
  // coordinator.
  private lastCheckpointHash: string | undefined;
  private confirmed: unknown[] = [];
  private accumulator = 0;
  private reportedTerminal = false;
  private routed = false;

  constructor(
    ports: OnlineMatchOptions<TDriver, TBatch, TSnapshot, TCheckpoint, TPresentation, TPresented, TCoordinator>,
    modelPort: OnlineMatchModelPort<TModel>,
    newModel: (request: OnlineMatchRequest, coordinator: TCoordinator) => TModel
  ) {
    this.ports = ports;
    this.modelPort = modelPort;
    this.request = ports.request;
    this.link = ports.link;
    this.onAction = ports.onAction;
    this.live = { [this.request.peer_id]: this.request.live ?? "" };
    this.driver = ports.matchDriver.create({
      role: this.request.role,
      peer_id: this.request.peer_id,
      freeze: this.request.freeze,
      manifest: this.request.manifest,
      transport: this.link.star,
      initial_snapshot: this.request.initial_snapshot,
    });
    this.presentation = ports.matchPresentation.create(this.request.initial_snapshot, this.request.first_input_tick);
    this.model = newModel(this.request, ports.coordinator);
    this.match = ports.newMatch({
      home: this.request.home,
      away: this.request.away,
      arenaId: this.request.arena.id,
      seed: this.request.seed,
      combatEnabled: this.request.combat_enabled,
      initialSnapshot: this.request.initial_snapshot,
      rollbackSource: () => this.driverSource(),
    });
    this.observerValue = ports.observer.create(this.match.state);
  }

  // The seam driving `match.ts`'s injected rollback source over the OMP-3
  // driver instead of the development laboratory. Returns `OnlineHostPort`
  // now that `match.ts` has an ONLINE construction mode to consume it (see
  // that file's "THE ONLINE-DRIVEN MATCH SCREEN" section) -- the
  // `frame`/`roster`/`tick`/`dispose` quartet below is new; every other
  // field was already exactly this shape (verified correct, untouched).
  private driverSource(): OnlineHostPort {
    return {
      needsLocalSample: () => this.ports.matchDriver.status(this.driver) === "active",
      advance: (_tick: number, sample: unknown) => {
        const batch = this.ports.matchDriver.advance(this.driver, sample);
        for (const entry of this.ports.matchDriver.batchControl(batch)) {
          this.control.push(entry as (typeof this.control)[number]);
        }
        for (const checkpoint of this.ports.matchDriver.batchCheckpoints(batch)) {
          this.checkpoints.push(checkpoint);
          const hash = (checkpoint as unknown as { readonly hash?: string }).hash;
          if (typeof hash === "string") {
            this.lastCheckpointHash = hash;
          }
        }
        this.live = { ...this.ports.matchDriver.batchLive(batch) };
        const presented = this.ports.matchPresentation.consume(this.presentation, this.driver, batch);
        for (const step of this.ports.matchPresentation.presentedConfirmedSteps(presented)) {
          this.confirmed.push(step);
        }
        return presented;
      },
      currentSnapshot: () => this.ports.matchDriver.currentSnapshot(this.driver),
      snapshot: (boundary: number) => this.ports.matchDriver.snapshot(this.driver, boundary + this.request.first_input_tick),
      terminal: () => this.ports.matchDriver.status(this.driver) !== "active",
      failed: () => {
        const status = this.ports.matchDriver.status(this.driver);
        return status !== "active" && status !== "completed";
      },
      // The settled boundary, not the tick the simulation stopped on. The
      // driver keeps draining the tail after full time.
      fullTime: () => this.ports.matchDriver.settled(this.driver),
      debugModel: () => undefined,
      controlledPlayer: (state: OnlineMatchState) => {
        const liveSlot = this.live[this.request.peer_id] ?? this.request.live;
        if (liveSlot === undefined) {
          return undefined;
        }
        return this.ports.matchSession.playerIndex(state, liveSlot);
      },
      frame: () => this.ports.matchDriver.frame(this.driver),
      roster: () => this.ports.matchDriver.roster(this.driver),
      tick: () => this.ports.matchDriver.tick(this.driver),
      dispose: () => this.ports.matchDriver.dispose(this.driver),
    };
  }

  dispatch(event: OnlineMatchDispatchEvent): void {
    const [model, effects] = this.modelPort.command(this.model, event);
    this.model = model;
    for (const effect of effects) {
      if (effect.kind === "send") {
        this.link.send(effect.link_id, effect.wire);
      } else if (effect.kind === "close") {
        this.link.apply({ kind: "close", link_id: effect.link_id });
      } else if (effect.kind === "observe_hash") {
        this.ports.matchDriver.observeCheckpoint(this.driver, effect.tick, effect.hash);
      }
    }
  }

  // Control traffic shares the transport with input. Once the driver is
  // terminal it stops polling entirely; the link is pumped directly from
  // that point on, which is also how a link lost after the final tick still
  // reaches the coordinator.
  private drainControl(): void {
    if (this.ports.matchDriver.status(this.driver) !== "active") {
      for (const entry of this.link.star.pollBatch()) {
        if (entry.channel === "control") {
          this.control.push(entry);
        }
      }
      let event = this.link.star.pollEvent();
      while (event) {
        if (event.kind === "peer_state" && event.peer_id !== undefined && (event.state === "closed" || event.state === "error" || event.state === "disconnected")) {
          this.dispatch({ kind: "link_lost", link_id: event.peer_id });
        }
        event = this.link.star.pollEvent();
      }
    }
    const entries = this.control;
    this.control = [];
    for (const entry of entries) {
      if (entry.channel === "control") {
        const buffer = this.buffers.get(entry.peer_id) ?? this.ports.lobbyFraming.newBuffer();
        this.buffers.set(entry.peer_id, buffer);
        const [wire, err] = this.ports.lobbyFraming.absorb(buffer, entry.message.payload);
        if (wire !== undefined) {
          this.dispatch({ kind: "control", link_id: entry.peer_id, wire });
        } else if (err !== undefined) {
          this.buffers.set(entry.peer_id, this.ports.lobbyFraming.newBuffer());
          this.model = { ...this.model, error: err };
        }
      }
    }
  }

  private drainSimulation(): void {
    const checkpoints = this.checkpoints;
    this.checkpoints = [];
    if (checkpoints.length > 0) {
      this.dispatch({ kind: "checkpoints", checkpoints });
    }
    const confirmedSteps = this.confirmed;
    this.confirmed = [];
    if (confirmedSteps.length > 0) {
      for (const step of confirmedSteps) {
        this.ports.observer.observeConfirmed(this.observerValue, step);
      }
      this.dispatch({ kind: "confirmed", steps: confirmedSteps });
    }
  }

  private reportDriver(): void {
    if (this.reportedTerminal) {
      return;
    }
    const terminal = this.ports.matchDriver.terminal(this.driver);
    if (terminal === undefined) {
      return;
    }
    this.reportedTerminal = true;
    if (terminal.status === "completed") {
      const state = this.match.state;
      this.dispatch({
        kind: "full_time",
        final_tick: state.time_left, // placeholder tick source -- see class header
        home_score: state.score.home,
        away_score: state.score.away,
        // The last boundary hash this peer's own driver observed -- see
        // `lastCheckpointHash`'s own doc. Empty only if the driver never
        // reported one, which the real coordinator refuses as malformed
        // (matching that it never had anything to agree on).
        final_hash: this.lastCheckpointHash ?? "",
      });
      return;
    }
    this.dispatch({ kind: "failure", status: terminal.status, failure: terminal.failure, detail: terminal.detail });
  }

  private result(): ProductMatchResult | undefined {
    const result = (this.model as { readonly result?: unknown }).result;
    if (result === undefined) {
      return undefined;
    }
    const observed = this.ports.observer.finish(this.observerValue);
    const typedResult = result as { readonly home_score: number; readonly away_score: number };
    // The score is the coordinator's acknowledged result, never this peer's
    // prediction. Statistics stay local: they are presentation, and OMP-3
    // does not put them on the wire.
    const outcome = this.ports.contract.newResult({
      home_team_id: this.request.home.id,
      away_team_id: this.request.away.id,
      home_score: typedResult.home_score,
      away_score: typedResult.away_score,
      home_stats: observed.home_stats,
      away_stats: observed.away_stats,
      ...(this.request.seed !== undefined ? { seed: this.request.seed } : {}),
      ...(observed.mvp_player_id !== undefined ? { mvp_player_id: observed.mvp_player_id } : {}),
      ...(observed.mvp_summary !== undefined ? { mvp_summary: observed.mvp_summary } : {}),
    });
    return outcome.ok ? outcome.value : undefined;
  }

  private route(): void {
    if (this.routed || !this.modelPort.ended(this.model)) {
      return;
    }
    // A confirmed goal replay owns the screen until it finishes; the session
    // has already ended underneath, so nothing is lost by letting it play
    // out.
    if (this.match.resultCompletionBlocked()) {
      return;
    }
    this.routed = true;
    if (!this.onAction) {
      return;
    }
    if (this.modelPort.exitRoute(this.model) === "result") {
      const result = this.result();
      if (result) {
        this.onAction({ go: "online_result", result });
        return;
      }
    }
    const model = this.model as { readonly terminal?: unknown; readonly error?: string };
    this.onAction({ go: "online_ended", terminal: model.terminal, detail: model.terminal ?? model.error });
  }

  update(dt: number): void {
    // The match screen owns the fixed clock and calls back into the driver
    // source. Local focus, pause requests, and a lost controller
    // deliberately do not reach this call: an online peer that stops
    // advancing stalls every peer.
    this.match.update(dt);
    this.drainControl();
    this.drainSimulation();
    this.reportDriver();
    this.accumulator += dt;
    while (this.accumulator >= TICK_SECONDS) {
      this.accumulator -= TICK_SECONDS;
      this.dispatch({ kind: "tick", dt: TICK_SECONDS });
    }
    this.route();
  }

  event(evt: RealMatchInputEvent & { readonly action?: string }): void {
    if (evt.kind === "action" && (evt.action === "pause" || evt.action === "back")) {
      this.dispatch({ kind: "pause_request" });
      return;
    }
    if ((this.model as { readonly abort_prompt?: boolean }).abort_prompt) {
      this.dispatch({ kind: "resume_request" });
    }
    this.match.event(evt);
  }

  focusLost(): void {
    this.dispatch({ kind: "focus_lost" });
  }

  controllerLost(): void {
    this.dispatch({ kind: "controller_lost" });
  }

  applySettings(settings: GameSettings): void {
    this.match.applySettings(settings);
  }

  teardown(): void {
    this.match.teardown();
  }

  // The online status panel. Left as raw lines -- see this module's header
  // for why the full renderer is out of scope this milestone.
  overlayLines(combatData: CombatPresentationData): string[] {
    const diagnostics = this.ports.matchDriver.diagnostics(this.driver);
    const state = this.match.state;
    const liveSlot = this.live[this.request.peer_id] ?? this.request.live;
    const combatModel = combat.model(state, (this.match as unknown as { _combat_state: CombatMatchState | null })._combat_state, combatData);
    const controlled = combatModel.players[state.controlled];
    const player = state.players[state.controlled];
    const lines = [
      `${this.request.mode.toUpperCase()}  ${this.request.role.toUpperCase()}  ${this.request.peer_id}`,
      `control ${String(liveSlot)}  owned ${this.request.owned.join(",")}`,
      `player ${player?.id ?? "-"}  family ${controlled ? (controlled.family_name ?? controlled.family_id ?? "none") : "none"}`,
      `equipment ${controlled ? (controlled.equipment_name ?? "none") : "none"}  ${controlled ? controlled.readiness : "unavailable"}`,
      `possession ${state.owner !== undefined ? state.players[state.owner]?.team : "loose"}`,
      `net tick ${diagnostics.transport_tick}  present ${diagnostics.present_input_tick}  confirmed ${diagnostics.confirmed_output_tick}`,
      `rollbacks ${diagnostics.rollback_count}  corrections ${diagnostics.correction_count}  predicted ${diagnostics.predicted_slot_samples}`,
      `session ${String((this.model as { readonly coordinator?: { readonly phase?: string } }).coordinator?.phase)}  driver ${diagnostics.status}`,
    ];
    const modelPhase = (this.model as { readonly phase?: string }).phase;
    if (modelPhase === "full_time") {
      lines.push("full time — waiting for every peer to acknowledge the result");
    }
    const terminal = (this.model as { readonly terminal?: { readonly reason: string } }).terminal;
    if (terminal) {
      lines.push(`session ended: ${terminal.reason}`);
    }
    const error = (this.model as { readonly error?: string }).error;
    if (error) {
      lines.push(error);
    }
    const abortPrompt = (this.model as { readonly abort_prompt?: boolean }).abort_prompt;
    const notice = (this.model as { readonly notice?: { readonly text: string } }).notice;
    if (abortPrompt) {
      lines.push(this.modelPort.ABORT_PROMPT);
    } else if (notice) {
      lines.push(notice.text);
    }
    return lines;
  }

  draw(): void {
    this.match.draw();
    void theme.colors.cyan; // the overlay panel's accent -- drawn by the (unbuilt) renderer this milestone.
  }
}
