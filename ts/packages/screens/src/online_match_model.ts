// The pure flow policy between the OMP-3 driver and the session coordinator.
//
// It owns no simulation and no transport: the driver owns the match, the
// coordinator owns the session, and this module decides which control-plane
// events a confirmed simulation step justifies, what a terminal driver
// status reports, when a result may be committed, and how focus loss, a
// pause request, a lost controller, and an abort are answered.
//
// The session coordinator and the wire protocol are both Rust-owned
// (`crates/gc-netcode`; ARCHITECTURE.md §1.1). `@gc/wasm` now binds a real
// `Coordinator` (`crates/gc-wasm/src/coordinator_bridge.rs`), but
// `@gc/screens` has no dependency edge onto `@gc/wasm` (absent from
// `package.json`) -- and wouldn't take one even if it did, since wiring
// Rust-owned session state to this screen's pure model is `@gc/app`'s job,
// not this package's (ARCHITECTURE.md's directory table). So both stay
// injected ports -- `CoordinatorPort`/`ProtocolPort` -- following
// `@gc/online`'s `match_presentation.ts` precedent
// (`RollbackEventsPort`/`MatchDriverPort`). `CoordinatorState` stays a
// generic `TState` bounded by only the fields this module actually reads
// (`phase`, `role`, `host_link_id`); everything else about session state is
// opaque and passed straight through `coordinator.step`.
//
// Every function here is pure -- `command` returns a fresh model plus an
// ordered effect list -- so a whole host/guest session replays headlessly
// with no display, no clock, and no network.

export type OnlineMatchPhase = "playing" | "full_time" | "ended";

export type OnlineMatchEffect =
  | { readonly kind: "send"; readonly link_id: string; readonly wire: string }
  | { readonly kind: "close"; readonly link_id: string; readonly detail?: string }
  | { readonly kind: "observe_hash"; readonly tick: number; readonly hash: string };

export interface OnlineMatchNotice {
  readonly text: string;
  readonly remaining: number;
}

/** The session coordinator's own state shape -- the fields this module reads. Opaque otherwise. */
export interface CoordinatorStateCore {
  readonly phase: string;
  readonly role: "host" | "guest";
  readonly host_link_id?: string;
}

/** The session coordinator's terminal-reason shape. */
export interface CoordinatorTerminal {
  readonly reason: string;
  readonly detail?: string;
}

/** The session coordinator's action shape. */
export interface CoordinatorAction {
  readonly kind: "send" | "close" | "terminate";
  readonly message?: unknown;
  readonly targets?: readonly string[];
  readonly link_id?: string;
  readonly terminal?: CoordinatorTerminal;
}

/** The session coordinator's outcome shape. */
export interface CoordinatorOutcome {
  readonly accepted: boolean;
  readonly reason?: string;
  readonly actions: readonly CoordinatorAction[];
}

/** One event the session coordinator's `step` accepts. Opaque payload beyond `kind`. */
export interface CoordinatorEvent {
  readonly kind: string;
  readonly [key: string]: unknown;
}

/** The session coordinator, injected -- see this module's header. */
export interface CoordinatorPort<TState extends CoordinatorStateCore> {
  step(state: TState, event: CoordinatorEvent): readonly [TState, CoordinatorOutcome];
}

/** The wire protocol's decoded envelope -- only `kind`/`body` are read here. */
export interface ProtocolMessage {
  readonly kind: string;
  readonly body?: { readonly [key: string]: unknown };
}

/** The wire protocol, injected -- see this module's header. */
export interface ProtocolPort {
  encode(message: unknown): string | undefined;
  decode(wire: string): ProtocolMessage | undefined;
}

/** `online_match.ts`'s `OnlineMatchRequest` -- the fields this module reads. */
export interface OnlineMatchRequestCore {
  readonly role: "host" | "guest";
  readonly first_input_tick: number;
}

/** The rollback event system's wrapped event shape, generic over its opaque payload. */
export interface RollbackWrappedEvent<TPayload = unknown> {
  readonly id: string;
  readonly tick: number;
  readonly domain: string;
  readonly ordinal: number;
  readonly payload: TPayload;
}

export type LifecyclePayload =
  | { readonly kind: "goal"; readonly team: "home" | "away"; readonly score: { readonly home: number; readonly away: number } }
  | { readonly kind: "kickoff" }
  | { readonly kind: "full_time" };

/** The rollback event system's per-tick step shape -- only the fields this module reads. */
export interface RollbackEventStepCore {
  readonly tick: number;
  readonly lifecycle_events: readonly RollbackWrappedEvent<LifecyclePayload>[];
}

export interface OnlineMatchModel<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore> {
  readonly request: TRequest;
  readonly coordinator: TState;
  readonly phase: OnlineMatchPhase;
  readonly published?: string;
  readonly score: { readonly home: number; readonly away: number };
  readonly tick: number;
  readonly notice?: OnlineMatchNotice;
  readonly abort_prompt: boolean;
  readonly result?: unknown;
  readonly terminal?: CoordinatorTerminal;
  readonly error?: string;
}

export const NOTICE_SECONDS = 3.5;

// The longest legal walk between two match phases: goal_stoppage -> kickoff ->
// playing -> full_time, plus the opening step from nothing.
const MATCH_PHASE_CHAIN = ["kickoff", "playing", "goal_stoppage", "full_time"] as const;

export const PAUSE_NOTICE = "Online match: pausing would stall every peer. Press Escape again to abort.";
export const FOCUS_NOTICE = "Window lost focus — the online match kept running.";
export const CONTROLLER_NOTICE = "Controller disconnected — keyboard input still works.";
export const ABORT_PROMPT = "Abort the session for every peer? Escape confirms, any other key resumes.";

export type OnlineMatchCommand =
  | { readonly kind: "control"; readonly link_id: string; readonly wire: string }
  | { readonly kind: "tick"; readonly dt?: number }
  | { readonly kind: "confirmed"; readonly steps?: readonly RollbackEventStepCore[] }
  | { readonly kind: "checkpoints"; readonly checkpoints?: readonly { readonly tick: number; readonly hash: string }[] }
  | {
      readonly kind: "full_time";
      readonly home_score: number;
      readonly away_score: number;
      readonly final_tick: number;
      readonly final_hash: string;
    }
  | { readonly kind: "failure"; readonly failure?: string; readonly detail?: string }
  | { readonly kind: "link_lost"; readonly link_id: string; readonly code?: string }
  | { readonly kind: "pause_request" }
  | { readonly kind: "resume_request" }
  | { readonly kind: "focus_lost" }
  | { readonly kind: "controller_lost" }
  | { readonly kind: "abort"; readonly detail?: string };

/**
 * A command whose `kind` this module does not recognize -- `command`
 * ignores it, falling through its switch with no matching case and no
 * effect. Kept as a separate type, rather than folded into
 * `OnlineMatchCommand` as a catch-all member, because a catch-all with an
 * open `kind: string` would widen every other member's `kind` to `string`
 * too and defeat the switch's discrimination.
 */
export type UnknownOnlineMatchCommand = { readonly kind: string; readonly [key: string]: unknown };

function copy<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>
): OnlineMatchModel<TState, TRequest> {
  return {
    request: model.request,
    coordinator: model.coordinator,
    phase: model.phase,
    score: { home: model.score.home, away: model.score.away },
    tick: model.tick,
    abort_prompt: model.abort_prompt,
    ...(model.published !== undefined ? { published: model.published } : {}),
    ...(model.notice !== undefined ? { notice: { text: model.notice.text, remaining: model.notice.remaining } } : {}),
    ...(model.result !== undefined ? { result: model.result } : {}),
    ...(model.terminal !== undefined ? { terminal: model.terminal } : {}),
    ...(model.error !== undefined ? { error: model.error } : {}),
  };
}

export function newOnlineMatchModel<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  request: TRequest,
  state: TState
): OnlineMatchModel<TState, TRequest> {
  return {
    request,
    coordinator: state,
    phase: "playing",
    score: { home: 0, away: 0 },
    tick: 0,
    abort_prompt: false,
  };
}

function notify<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>,
  text: string
): OnlineMatchModel<TState, TRequest> {
  return { ...model, notice: { text, remaining: NOTICE_SECONDS } };
}

function absorb<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>,
  outcome: CoordinatorOutcome,
  protocol: ProtocolPort,
  effects: OnlineMatchEffect[]
): OnlineMatchModel<TState, TRequest> {
  let next = model;
  if (!outcome.accepted && outcome.reason) {
    next = { ...next, error: outcome.reason };
  }
  for (const action of outcome.actions) {
    if (action.kind === "send") {
      const wire = action.message !== undefined ? protocol.encode(action.message) : undefined;
      if (wire !== undefined) {
        for (const target of action.targets ?? []) {
          effects.push({ kind: "send", link_id: target, wire });
        }
      } else {
        next = { ...next, error: "a control message could not be encoded" };
      }
    } else if (action.kind === "close") {
      if (action.link_id === undefined) {
        throw new Error("a close action requires a link_id");
      }
      effects.push({ kind: "close", link_id: action.link_id });
    } else if (action.kind === "terminate") {
      if (action.terminal === undefined) {
        throw new Error("a terminate action requires a terminal");
      }
      next = { ...next, terminal: action.terminal };
    }
  }
  return next;
}

function step<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>,
  coordinator: CoordinatorPort<TState>,
  protocol: ProtocolPort,
  event: CoordinatorEvent,
  effects: OnlineMatchEffect[]
): OnlineMatchModel<TState, TRequest> {
  const state = model.coordinator;
  // A session that already ended keeps its reason. Late traffic after a
  // termination must not overwrite it with "the session already ended".
  if (state.phase === "terminal" && event.kind !== "tick") {
    return model;
  }
  const [nextState, outcome] = coordinator.step(state, event);
  return absorb({ ...model, coordinator: nextState }, outcome, protocol, effects);
}

function settle<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>,
  isCompleted: (terminal: CoordinatorTerminal) => boolean,
  localResult: (state: TState) => unknown
): OnlineMatchModel<TState, TRequest> {
  const state = model.coordinator;
  if (state.phase !== "terminal") {
    return model;
  }
  let next: OnlineMatchModel<TState, TRequest> = { ...model, phase: "ended" };
  // `terminal` is set by `absorb` off a coordinator `terminate` action; a
  // caller-supplied terminal reason living on the coordinator state itself
  // is not modeled here (opaque `TState`), so this only ever sees the half
  // of that fallback this module's own `model.terminal` carries.
  if (next.terminal !== undefined && isCompleted(next.terminal)) {
    next = { ...next, result: localResult(state) };
  }
  return next;
}

function publishPhase<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>,
  coordinator: CoordinatorPort<TState>,
  protocol: ProtocolPort,
  phase: string,
  effects: OnlineMatchEffect[]
): OnlineMatchModel<TState, TRequest> {
  const next = step(
    model,
    coordinator,
    protocol,
    { kind: "match_phase", phase, tick: model.tick, home_score: model.score.home, away_score: model.score.away },
    effects
  );
  return { ...next, published: phase };
}

// The coordinator's match-phase order is a chain: a running match opens with
// `kickoff`, `playing` follows it, `goal_stoppage` and `full_time` follow
// `playing`, and `kickoff` follows a stoppage. Walking that chain here means a
// caller names the phase it *means* and cannot accidentally publish an
// illegal transition, which would be refused, leaving the session stuck one
// step short of the result.
function advanceTo<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>,
  coordinator: CoordinatorPort<TState>,
  protocol: ProtocolPort,
  target: string,
  effects: OnlineMatchEffect[]
): OnlineMatchModel<TState, TRequest> {
  let next = model;
  for (let i = 0; i < MATCH_PHASE_CHAIN.length; i += 1) {
    if (next.published === target) {
      return next;
    }
    const current = next.published;
    let nextPhase: string;
    if (current === undefined) {
      nextPhase = "kickoff";
    } else if (current === "kickoff") {
      nextPhase = "playing";
    } else if (current === "goal_stoppage") {
      nextPhase = "kickoff";
    } else if (current === "playing") {
      nextPhase = target;
    } else {
      return next; // Full time has no successor; the result path takes over.
    }
    next = publishPhase(next, coordinator, protocol, nextPhase, effects);
  }
  return next;
}

// Publish the control-plane consequences of one confirmed simulation step.
// The ordering the coordinator enforces (kickoff, then playing, then a goal
// stoppage only after the score actually moved, then kickoff again) is
// derived here from the confirmed lifecycle records alone, so a predicted
// goal can never advance the session.
function publishSteps<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>,
  coordinator: CoordinatorPort<TState>,
  protocol: ProtocolPort,
  steps: readonly RollbackEventStepCore[],
  effects: OnlineMatchEffect[]
): OnlineMatchModel<TState, TRequest> {
  if (model.request.role !== "host") {
    return model;
  }
  let next = model;
  for (const confirmed of steps) {
    next = { ...next, tick: Math.max(next.tick, confirmed.tick + next.request.first_input_tick) };
    for (const event of confirmed.lifecycle_events) {
      const payload = event.payload;
      if (payload.kind === "goal") {
        // The score has to move *before* the stoppage is published: the
        // coordinator refuses a goal stoppage that no goal preceded.
        next = { ...next, score: { home: payload.score.home, away: payload.score.away } };
        next = advanceTo(next, coordinator, protocol, "goal_stoppage", effects);
      } else if (payload.kind === "kickoff") {
        next = advanceTo(next, coordinator, protocol, "kickoff", effects);
      }
    }
    next = advanceTo(next, coordinator, protocol, "playing", effects);
  }
  return next;
}

function reachFullTime<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>,
  coordinator: CoordinatorPort<TState>,
  protocol: ProtocolPort,
  event: { readonly home_score: number; readonly away_score: number; readonly final_tick: number; readonly final_hash: string },
  effects: OnlineMatchEffect[]
): OnlineMatchModel<TState, TRequest> {
  if (model.phase !== "playing") {
    return model;
  }
  let next: OnlineMatchModel<TState, TRequest> = {
    ...model,
    phase: "full_time",
    score: { home: event.home_score, away: event.away_score },
    tick: Math.max(model.tick, event.final_tick),
  };
  if (next.request.role === "host") {
    next = advanceTo(next, coordinator, protocol, "full_time", effects);
  }
  // Both roles report their own result. The host's becomes the session
  // result; a guest's is retained locally so the host's acknowledgement is
  // checked against a simulation this peer actually ran, rather than
  // accepted on faith.
  return step(
    next,
    coordinator,
    protocol,
    {
      kind: "finish",
      final_tick: next.tick,
      home_score: next.score.home,
      away_score: next.score.away,
      final_hash: event.final_hash,
    },
    effects
  );
}

function reportFailure<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>,
  coordinator: CoordinatorPort<TState>,
  protocol: ProtocolPort,
  event: { readonly failure?: string; readonly detail?: string },
  effects: OnlineMatchEffect[]
): OnlineMatchModel<TState, TRequest> {
  if (event.failure) {
    return step(
      model,
      coordinator,
      protocol,
      { kind: "netcode_failure", failure: event.failure, ...(event.detail !== undefined ? { detail: event.detail } : {}) },
      effects
    );
  }
  // A lost star has no failure class of its own. A guest knows exactly which
  // link died -- the only one it has -- so it reports the honest disconnect;
  // a host cannot tell which guest went, because the driver folds every link
  // event into one status, so it ends the session explicitly instead.
  const state = model.coordinator;
  if (state.role === "guest" && state.host_link_id !== undefined) {
    return step(
      model,
      coordinator,
      protocol,
      { kind: "link_lost", link_id: state.host_link_id, code: "transport_lost" },
      effects
    );
  }
  return step(
    model,
    coordinator,
    protocol,
    { kind: "abort", code: "peer_disconnect", detail: event.detail ?? "the star transport was lost" },
    effects
  );
}

function absorbControl<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>,
  coordinator: CoordinatorPort<TState>,
  protocol: ProtocolPort,
  linkId: string,
  wire: string,
  effects: OnlineMatchEffect[]
): OnlineMatchModel<TState, TRequest> {
  const message = protocol.decode(wire);
  // A peer's boundary hash is the driver's business as well as the session's:
  // the coordinator counts consecutive disagreements, the driver compares the
  // boundary it actually hashed. Both authorities see the same report.
  if (message?.kind === "hash_report" && message.body) {
    const body = message.body as { readonly tick: number; readonly boundary_hash: string };
    effects.push({ kind: "observe_hash", tick: body.tick, hash: body.boundary_hash });
  }
  return step(model, coordinator, protocol, { kind: "control", link_id: linkId, wire }, effects);
}

export interface OnlineMatchModelPorts<TState extends CoordinatorStateCore> {
  readonly coordinator: CoordinatorPort<TState>;
  readonly protocol: ProtocolPort;
  /** The session result if one was reached, else this peer's own local result -- see `settle`. */
  readonly localResult: (state: TState) => unknown;
  readonly isCompleted: (terminal: CoordinatorTerminal) => boolean;
}

// One flow event. `event` is abstracted the same way a screen event is: the
// caller translates transport traffic, driver batches, and raw input into
// these shapes and executes the returned effects.
export function command<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>,
  ports: OnlineMatchModelPorts<TState>,
  rawEvent: OnlineMatchCommand | UnknownOnlineMatchCommand
): readonly [OnlineMatchModel<TState, TRequest>, readonly OnlineMatchEffect[]] {
  const { coordinator, protocol } = ports;
  const effects: OnlineMatchEffect[] = [];
  let next = copy(model);
  // Unknown commands fall through to `default` unchanged -- an implicit
  // no-op. The cast reflects that every case below only runs once
  // `event.kind` has matched a known literal.
  const event = rawEvent as OnlineMatchCommand;
  switch (event.kind) {
    case "control":
      next = absorbControl(next, coordinator, protocol, event.link_id, event.wire, effects);
      break;
    case "tick": {
      next = step(next, coordinator, protocol, { kind: "tick" }, effects);
      if (next.notice) {
        const remaining = next.notice.remaining - (event.dt ?? 0);
        next = remaining > 0 ? { ...next, notice: { text: next.notice.text, remaining } } : withoutNotice(next);
      }
      break;
    }
    case "confirmed":
      next = publishSteps(next, coordinator, protocol, event.steps ?? [], effects);
      break;
    case "checkpoints":
      for (const checkpoint of event.checkpoints ?? []) {
        next = step(
          next,
          coordinator,
          protocol,
          { kind: "hash_report", tick: checkpoint.tick, boundary_hash: checkpoint.hash },
          effects
        );
      }
      break;
    case "full_time":
      next = reachFullTime(next, coordinator, protocol, event, effects);
      break;
    case "failure":
      next = reportFailure(next, coordinator, protocol, event, effects);
      break;
    case "link_lost":
      next = step(
        next,
        coordinator,
        protocol,
        { kind: "link_lost", link_id: event.link_id, code: event.code ?? "transport_lost" },
        effects
      );
      break;
    case "pause_request":
      // Deliberate: no local pause. The first press explains why, the second
      // offers the only honest alternative.
      if (next.abort_prompt) {
        next = step(
          next,
          coordinator,
          protocol,
          { kind: "abort", code: "host_abort", detail: "a peer aborted the online match" },
          effects
        );
      } else {
        next = notify({ ...next, abort_prompt: true }, PAUSE_NOTICE);
      }
      break;
    case "resume_request":
      next = { ...next, abort_prompt: false };
      break;
    case "focus_lost":
      next = notify(next, FOCUS_NOTICE);
      break;
    case "controller_lost":
      next = notify(next, CONTROLLER_NOTICE);
      break;
    case "abort":
      next = step(
        next,
        coordinator,
        protocol,
        { kind: "abort", code: "host_abort", detail: event.detail ?? "a peer aborted the online match" },
        effects
      );
      break;
    default:
      break;
  }
  next = settle(next, ports.isCompleted, ports.localResult);
  return [next, effects];
}

function withoutNotice<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>
): OnlineMatchModel<TState, TRequest> {
  const { notice: _notice, ...rest } = model;
  return rest;
}

export function ended<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>
): boolean {
  return model.phase === "ended";
}

// What the flow should do once the session has ended. A completed session has
// an acknowledged result to show; anything else leaves with its terminal
// reason and no score, because a session that failed has no half-agreed
// outcome to display.
export function exitRoute<TState extends CoordinatorStateCore, TRequest extends OnlineMatchRequestCore>(
  model: OnlineMatchModel<TState, TRequest>
): "result" | "terminal" {
  return model.result !== undefined ? "result" : "terminal";
}
