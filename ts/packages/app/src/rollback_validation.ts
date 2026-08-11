// This module cross-checks a reference (never-rolled-back) simulation
// against an impaired (rollback-corrected) one, replaying the same
// confirmed event/observer/replay/audio pipeline against both and diffing
// the results. Every one of its dependencies beyond this package's own
// `audio.ts`/`match_observer.ts`/`match_contract.ts` is either Rust-owned
// (`sim.match_snapshot`, `sim.rollback_events` -- `crates/gc-sim`;
// ARCHITECTURE.md §1) or not available to this package (`@gc/render`'s
// effects module; `@gc/presentation`'s `combat_feedback`, not a declared
// dependency here). All four are injected ports, following the same shape
// `match_presentation.ts` established for `RollbackEventsPort`.
// `@gc/render`'s `replay.ts` (a declared dependency) is injected here too
// rather than imported directly: this module's simulation state
// (`TState`) is otherwise fully opaque/generic, and `replay.ts`'s
// `MatchState` requires many concrete sim fields this module never itself
// reads. Injecting the four operations `replay.ts` provides keeps
// `TState` narrow (`ObservedMatchState` plus the handful of fields
// `recordReplaySample` mutates) instead of forcing the whole render
// `MatchState` shape onto every generic caller.

import { Vec2, type Result } from "@gc/core";
import {
  Audio,
  type CombatFeedbackPort,
  type RollbackWrappedEvent as AudioWrappedEvent,
} from "./audio.ts";
import { matchContract, type ProductMatchResult } from "./match_contract.ts";
import {
  matchObserver,
  type MatchObserver,
  type ObservedConfirmedStep,
  type ObservedMatchState,
  type ObservedMatchSummary,
} from "./match_observer.ts";

export type RollbackValidationScenario =
  "possession" | "tackle" | "shot" | "goal" | "kickoff" | "aerial" | "keeper" | "full_time";

export type MatchTeam = "home" | "away";

/** The extra fields `recordReplaySample` mutates on the module's scratch replay state -- see this file's header. */
export interface ReplaySampleState {
  input_tick: number;
  ball: Vec2;
  score: { home: number; away: number };
}

/** `sim.rollback_events`'s `RollbackWrappedEvent` (Rust-owned; opaque payload here). */
export interface RollbackWrappedEvent {
  readonly id: string;
  readonly tick: number;
  readonly domain: string;
  readonly ordinal: number;
  readonly payload: unknown;
}

export interface RollbackEventReplacement {
  readonly before: RollbackWrappedEvent;
  readonly after: RollbackWrappedEvent;
}

export interface RollbackEventDiff {
  readonly added: readonly RollbackWrappedEvent[];
  readonly revoked: readonly RollbackWrappedEvent[];
  readonly replaced: readonly RollbackEventReplacement[];
}

export interface RollbackConfirmedStateView {
  readonly owner_team?: MatchTeam;
  readonly score: { readonly home: number; readonly away: number };
}

export interface RollbackEventStep {
  readonly tick: number;
  readonly state: RollbackConfirmedStateView;
  readonly match_events: readonly RollbackWrappedEvent[];
  readonly combat_events?: readonly RollbackWrappedEvent[];
  readonly lifecycle_events: readonly RollbackWrappedEvent[];
}

export interface RollbackEventStepInput<TSnapshot> {
  readonly output: { readonly tick: number };
  readonly snapshot: TSnapshot;
}

/** `sim.match_snapshot`, injected -- see this file's header. */
export interface MatchSnapshotPort<TState, TCombatState, TSnapshot> {
  capture(state: TState, combatState: TCombatState | undefined): TSnapshot;
  restore(snapshot: TSnapshot): TState;
  /** A canonical, platform-stable byte encoding of a float -- used only for stable digests here.
   * A property, not a method: it is INJECTED and passed around by reference
   * (`stableValue(value, ports.matchSnapshot.numberBytes)`), so it carries no
   * `this` and must not be declared as though it did. Same spelling as
   * `ObserveImpairedStepPorts.numberBytes` below. */
  readonly numberBytes: (value: number) => string;
}

export interface RollbackApplyFailure {
  readonly message: string;
  readonly code: "unconfirmed_window_exceeded";
}

/** `sim.rollback_events`, injected -- see this file's header. */
export interface RollbackEventsPort<TTimeline, TSnapshot> {
  create(initialSnapshot: TSnapshot): TTimeline;
  apply(
    timeline: TTimeline,
    from: number,
    through: number,
    steps: readonly RollbackEventStepInput<TSnapshot>[],
  ): Result<RollbackEventDiff, RollbackApplyFailure>;
  confirm(timeline: TTimeline, tick: number): readonly RollbackEventStep[];
}

/** `game.render.effects`, injected -- see this file's header. */
export interface EffectsPort {
  reset(): void;
  applyEventDiff(diff: RollbackEventDiff): void;
  confirmEvent(event: RollbackWrappedEvent): void;
  diagnostics(): { readonly speculative_ids: readonly string[] };
}

export interface ReplayDiagnosticsSlice {
  readonly count: number;
  readonly boundaries: readonly number[];
}

export interface ReplayBoundarySample {
  readonly ball_x: number;
  readonly ball_y: number;
  readonly score_home: number;
  readonly score_away: number;
}

/** `game.render.replay` (`@gc/render`'s `replay.ts`), injected -- see this file's header. */
export interface ReplayPort<TState, TCombatState> {
  reset(): void;
  capacity(): number;
  truncateFrom(boundary: number): void;
  recordBoundary(boundary: number, state: TState, combatState?: TCombatState): void;
  diagnostics(): ReplayDiagnosticsSlice;
  boundarySample(boundary: number): ReplayBoundarySample | undefined;
}

export interface RollbackValidationReplaySample {
  readonly boundary: number;
  readonly ball_x: number;
  readonly ball_y: number;
  readonly score_home: number;
  readonly score_away: number;
}

export interface RollbackValidationEventMetrics {
  reference_unique: number;
  impaired_unique: number;
  duplicate_confirmed: number;
  confirmed_without_speculation: number;
  speculative_added: number;
  speculative_duplicate_added: number;
  speculative_revoked: number;
  speculative_unknown_revoked: number;
  speculative_replaced: number;
  speculative_invalid_replaced: number;
  missing_confirmed: number;
  unexpected_confirmed: number;
  mismatched_confirmed: number;
  terminal_speculative_residue: number;
  consumer_speculative_residue: number;
}

export interface RollbackValidationConsumerMetrics {
  readonly audio_cue_count: number;
  readonly expected_audio_cue_count: number;
  readonly audio_cues: Readonly<Record<string, number>>;
  readonly expected_audio_cues: Readonly<Record<string, number>>;
  readonly reference_observer_steps: number;
  readonly impaired_observer_steps: number;
  readonly replay_record_count: number;
  readonly replay_truncate_count: number;
  readonly replay_boundary_count: number;
  readonly result_count: number;
}

export interface RollbackValidationReport {
  readonly passed: boolean;
  readonly errors: readonly string[];
  readonly events: RollbackValidationEventMetrics;
  readonly consumers: RollbackValidationConsumerMetrics;
  readonly scenario_counts: Readonly<Record<RollbackValidationScenario, number>>;
  readonly observer_matched: boolean;
  readonly observer_reference_digest: string;
  readonly observer_impaired_digest: string;
  readonly result_matched: boolean;
  readonly result_reference_digest: string;
  readonly result_impaired_digest: string;
  readonly replay_matched: boolean;
  readonly replay_boundaries: readonly number[];
}

// `_TState` is a phantom parameter: unused in the body, kept because the
// exported type's arity is part of `@gc/app`'s public surface.
export interface RollbackValidationFinishOptions<_TState> {
  readonly home_team_id: string;
  readonly away_team_id: string;
  readonly reference_final_state?: {
    readonly score: { readonly home: number; readonly away: number };
  };
  readonly impaired_final_state?: {
    readonly score: { readonly home: number; readonly away: number };
  };
  readonly reference_final_score?: { readonly home: number; readonly away: number };
  readonly impaired_final_score?: { readonly home: number; readonly away: number };
  readonly seed?: number;
  readonly expected_replay_boundaries?: readonly number[];
  readonly expected_replay_samples?: readonly RollbackValidationReplaySample[];
  readonly expected_replay_truncate_count?: number;
  readonly required_scenarios?: readonly RollbackValidationScenario[];
}

// `_TSnapshot`: phantom, as above -- every caller still passes it positionally.
export interface RollbackValidationAudit<TState, TTimeline, _TSnapshot> {
  readonly referenceObserver: MatchObserver;
  readonly impairedObserver: MatchObserver;
  readonly referenceTimeline: TTimeline;
  readonly referenceIds: Map<string, string>;
  readonly referenceEvents: Map<string, RollbackWrappedEvent>;
  readonly impairedIds: Map<string, string>;
  readonly speculativeIds: Map<string, string>;
  readonly scenarioCounts: Record<RollbackValidationScenario, number>;
  referenceLastOwner?: MatchTeam | undefined;
  impairedLastOwner?: MatchTeam | undefined;
  readonly referenceScore: { home: number; away: number };
  readonly impairedScore: { home: number; away: number };
  readonly events: RollbackValidationEventMetrics;
  referenceObserverSteps: number;
  impairedObserverSteps: number;
  replayRecordCount: number;
  replayTruncateCount: number;
  readonly replayState: TState;
}

export interface RollbackValidationTraceRow<TState, TSnapshot> {
  readonly kind:
    | "reference_step"
    | "reference_confirmed"
    | "impaired_diff"
    | "confirmed"
    | "impaired_confirmed"
    | "replay_boundary"
    | "replay_truncate";
  readonly step?: RollbackEventStepInput<TSnapshot> | RollbackEventStep;
  readonly diff?: RollbackEventDiff;
  readonly boundary?: number;
  readonly state?: TState;
  readonly snapshot?: TSnapshot;
  readonly replaySample?: RollbackValidationReplaySample;
}

export interface RollbackValidationPorts<TState, TCombatState, TSnapshot, TTimeline> {
  readonly matchSnapshot: MatchSnapshotPort<TState, TCombatState, TSnapshot>;
  readonly rollbackEvents: RollbackEventsPort<TTimeline, TSnapshot>;
  readonly effects: EffectsPort;
  readonly replay: ReplayPort<TState, TCombatState>;
  readonly audio: Audio;
  readonly combatFeedback: CombatFeedbackPort;
}

const SCENARIOS: readonly RollbackValidationScenario[] = [
  "possession",
  "tackle",
  "shot",
  "goal",
  "kickoff",
  "aerial",
  "keeper",
  "full_time",
];

const AUDIO_KINDS: ReadonlySet<string> = new Set([
  "touch",
  "reception",
  "pass",
  "block",
  "catch",
  "claim",
  "parry",
  "tackle",
  "header",
  "shot",
  "volley",
  "bicycle",
]);

const AERIAL_KINDS: ReadonlySet<string> = new Set(["header", "volley", "bicycle"]);
const KEEPER_KINDS: ReadonlySet<string> = new Set(["catch", "parry", "tip", "claim"]);

function stableValue(value: unknown, numberBytes: (value: number) => string): string {
  if (value === undefined || value === null) {
    return "nil";
  }
  const kind = typeof value;
  if (kind === "number") {
    return `n:${numberBytes(value as number)}`;
  } else if (kind === "boolean") {
    return value ? "b:1" : "b:0";
  } else if (kind === "string") {
    const s = value as string;
    return `s:${s.length}:${s}`;
  }
  if (kind !== "object") {
    throw new Error("rollback validation values must contain only serializable data");
  }
  const record = value as Record<string, unknown>;
  const keys = Object.keys(record).sort();
  const parts = ["{"];
  for (const key of keys) {
    parts.push(stableValue(key, numberBytes));
    parts.push(stableValue(record[key], numberBytes));
  }
  parts.push("}");
  return parts.join("");
}

function eventSignature(
  event: RollbackWrappedEvent,
  numberBytes: (value: number) => string,
): string {
  return stableValue(
    {
      id: event.id,
      tick: event.tick,
      domain: event.domain,
      ordinal: event.ordinal,
      payload: event.payload,
    },
    numberBytes,
  );
}

function countScenario(
  counts: Record<RollbackValidationScenario, number>,
  scenario: RollbackValidationScenario,
): void {
  counts[scenario] += 1;
}

function eventKind(event: RollbackWrappedEvent): string | undefined {
  const payload = event.payload as { readonly kind?: string } | undefined;
  return payload?.kind;
}

function classifyEvent(
  counts: Record<RollbackValidationScenario, number>,
  event: RollbackWrappedEvent,
): void {
  if (event.domain.startsWith("match/")) {
    const kind = eventKind(event);
    if (kind === "tackle") {
      countScenario(counts, "tackle");
    } else if (kind === "shot") {
      countScenario(counts, "shot");
    }
    if (kind !== undefined && AERIAL_KINDS.has(kind)) {
      countScenario(counts, "aerial");
    }
    if (kind !== undefined && KEEPER_KINDS.has(kind)) {
      countScenario(counts, "keeper");
    }
  } else if (event.domain === "lifecycle/goal") {
    countScenario(counts, "goal");
  } else if (event.domain === "lifecycle/kickoff") {
    countScenario(counts, "kickoff");
  } else if (event.domain === "lifecycle/full_time") {
    countScenario(counts, "full_time");
  }
}

function classifyStep<TState, TTimeline, TSnapshot>(
  audit: RollbackValidationAudit<TState, TTimeline, TSnapshot>,
  step: RollbackEventStep,
  reference: boolean,
): void {
  const previous = reference ? audit.referenceLastOwner : audit.impairedLastOwner;
  if (step.state.owner_team !== previous) {
    countScenario(audit.scenarioCounts, "possession");
  }
  if (reference) {
    audit.referenceLastOwner = step.state.owner_team;
  } else {
    audit.impairedLastOwner = step.state.owner_team;
  }
}

function audioCue(
  event: RollbackWrappedEvent,
  combatFeedback: CombatFeedbackPort,
): string | undefined {
  if (event.domain.startsWith("match/")) {
    const kind = eventKind(event);
    return kind !== undefined && AUDIO_KINDS.has(kind) ? kind : undefined;
  } else if (event.domain.startsWith("combat/")) {
    return combatFeedback.link(event.payload, event.id).disposition.audio;
  } else if (event.domain === "lifecycle/goal") {
    return "goal";
  } else if (event.domain === "lifecycle/kickoff") {
    return "kickoff";
  } else if (event.domain === "lifecycle/full_time") {
    return "full_time";
  }
  return undefined;
}

function ownerTeam(state: ObservedMatchState): MatchTeam | undefined {
  const owner = state.owner !== undefined ? state.players[state.owner] : undefined;
  return owner?.team;
}

function newScenarioCounts(): Record<RollbackValidationScenario, number> {
  const counts = {} as Record<RollbackValidationScenario, number>;
  for (const name of SCENARIOS) {
    counts[name] = 0;
  }
  return counts;
}

export function newAudit<
  TState extends ObservedMatchState & ReplaySampleState,
  TCombatState,
  TSnapshot,
  TTimeline,
>(
  ports: RollbackValidationPorts<TState, TCombatState, TSnapshot, TTimeline>,
  initialState: TState,
  initialCombatState: TCombatState | undefined,
): RollbackValidationAudit<TState, TTimeline, TSnapshot> {
  ports.audio.reset();
  ports.effects.reset();
  ports.replay.reset();
  const initialOwner = ownerTeam(initialState);
  const snapshot = ports.matchSnapshot.capture(initialState, initialCombatState);
  return {
    referenceObserver: matchObserver.new(initialState),
    impairedObserver: matchObserver.new(initialState),
    referenceTimeline: ports.rollbackEvents.create(snapshot),
    referenceIds: new Map(),
    referenceEvents: new Map(),
    impairedIds: new Map(),
    speculativeIds: new Map(),
    scenarioCounts: newScenarioCounts(),
    ...(initialOwner !== undefined
      ? { referenceLastOwner: initialOwner, impairedLastOwner: initialOwner }
      : {}),
    referenceScore: { home: initialState.score.home, away: initialState.score.away },
    impairedScore: { home: initialState.score.home, away: initialState.score.away },
    events: {
      reference_unique: 0,
      impaired_unique: 0,
      duplicate_confirmed: 0,
      confirmed_without_speculation: 0,
      speculative_added: 0,
      speculative_duplicate_added: 0,
      speculative_revoked: 0,
      speculative_unknown_revoked: 0,
      speculative_replaced: 0,
      speculative_invalid_replaced: 0,
      missing_confirmed: 0,
      unexpected_confirmed: 0,
      mismatched_confirmed: 0,
      terminal_speculative_residue: 0,
      consumer_speculative_residue: 0,
    },
    referenceObserverSteps: 0,
    impairedObserverSteps: 0,
    replayRecordCount: 0,
    replayTruncateCount: 0,
    replayState: ports.matchSnapshot.restore(
      ports.matchSnapshot.capture(initialState, initialCombatState),
    ),
  };
}

const TICK_SECONDS = 1 / 60;

function stepEvents(step: RollbackEventStep): readonly (readonly RollbackWrappedEvent[])[] {
  return [step.match_events, step.combat_events ?? [], step.lifecycle_events];
}

// `RollbackEventStep.match_events` payloads are opaque here (`unknown`, this
// module never inspects them beyond passing them through) but
// `match_observer.ts`'s `ObservedConfirmedStep` needs the narrower
// `ObservedMatchEvent` shape to read `.kind`/`.player`. Both are Rust-owned
// (`sim.rollback_events`) shapes describing the exact same wire payload;
// this cast documents that rather than widening either module's own types.
function asObservedStep(step: RollbackEventStep): ObservedConfirmedStep {
  return step as unknown as ObservedConfirmedStep;
}

export function observeReferenceStep<
  TState extends ObservedMatchState & ReplaySampleState,
  TTimeline,
  TSnapshot,
>(
  numberBytes: (value: number) => string,
  audit: RollbackValidationAudit<TState, TTimeline, TSnapshot>,
  step: RollbackEventStep,
): void {
  const observed = matchObserver.observeConfirmed(
    audit.referenceObserver,
    asObservedStep(step),
    TICK_SECONDS,
  );
  if (observed) {
    audit.referenceObserverSteps += 1;
    classifyStep(audit, step, true);
    audit.referenceScore.home = step.state.score.home;
    audit.referenceScore.away = step.state.score.away;
  }
  for (const events of stepEvents(step)) {
    for (const event of events) {
      const signature = eventSignature(event, numberBytes);
      const previous = audit.referenceIds.get(event.id);
      if (previous === undefined) {
        audit.referenceIds.set(event.id, signature);
        audit.referenceEvents.set(event.id, event);
        audit.events.reference_unique += 1;
        classifyEvent(audit.scenarioCounts, event);
      } else if (previous !== signature) {
        audit.events.mismatched_confirmed += 1;
      }
    }
  }
}

export function applyImpairedDiff<TState, TTimeline, TSnapshot>(
  effects: EffectsPort,
  audit: RollbackValidationAudit<TState, TTimeline, TSnapshot>,
  diff: RollbackEventDiff,
  numberBytes: (value: number) => string,
): void {
  for (const event of diff.revoked) {
    audit.events.speculative_revoked += 1;
    if (!audit.speculativeIds.has(event.id)) {
      audit.events.speculative_unknown_revoked += 1;
    } else {
      audit.speculativeIds.delete(event.id);
    }
  }
  for (const replacement of diff.replaced) {
    audit.events.speculative_replaced += 1;
    const before = audit.speculativeIds.get(replacement.before.id);
    if (before === undefined || before !== eventSignature(replacement.before, numberBytes)) {
      audit.events.speculative_invalid_replaced += 1;
    }
    audit.speculativeIds.delete(replacement.before.id);
    audit.speculativeIds.set(replacement.after.id, eventSignature(replacement.after, numberBytes));
  }
  for (const event of diff.added) {
    audit.events.speculative_added += 1;
    const signature = eventSignature(event, numberBytes);
    const existing = audit.speculativeIds.get(event.id);
    if (existing !== undefined) {
      audit.events.speculative_duplicate_added += 1;
      if (existing !== signature) {
        audit.events.speculative_invalid_replaced += 1;
      }
    } else {
      audit.speculativeIds.set(event.id, signature);
    }
  }
  effects.applyEventDiff(diff);
}

export interface ObserveImpairedStepPorts {
  readonly numberBytes: (value: number) => string;
  readonly effects: EffectsPort;
  readonly audio: Audio;
}

export function observeImpairedStep<
  TState extends ObservedMatchState & ReplaySampleState,
  TTimeline,
  TSnapshot,
>(
  ports: ObserveImpairedStepPorts,
  audit: RollbackValidationAudit<TState, TTimeline, TSnapshot>,
  step: RollbackEventStep,
): void {
  const observed = matchObserver.observeConfirmed(
    audit.impairedObserver,
    asObservedStep(step),
    TICK_SECONDS,
  );
  if (observed) {
    audit.impairedObserverSteps += 1;
    classifyStep(audit, step, false);
    audit.impairedScore.home = step.state.score.home;
    audit.impairedScore.away = step.state.score.away;
  } else {
    audit.events.duplicate_confirmed += 1;
  }
  for (const events of stepEvents(step)) {
    for (const event of events) {
      const signature = eventSignature(event, ports.numberBytes);
      const previous = audit.impairedIds.get(event.id);
      if (previous === undefined) {
        audit.impairedIds.set(event.id, signature);
        audit.events.impaired_unique += 1;
      } else if (previous === signature) {
        audit.events.duplicate_confirmed += 1;
      } else {
        audit.events.mismatched_confirmed += 1;
      }
      if (!audit.speculativeIds.has(event.id) && previous === undefined) {
        audit.events.confirmed_without_speculation += 1;
      }
      audit.speculativeIds.delete(event.id);
      ports.effects.confirmEvent(event);
      ports.audio.consumeConfirmed(event as AudioWrappedEvent);
    }
  }
}

export function truncateReplay<TState, TTimeline, TSnapshot>(
  replay: Pick<ReplayPort<TState, unknown>, "truncateFrom">,
  audit: RollbackValidationAudit<TState, TTimeline, TSnapshot>,
  boundary: number,
): void {
  replay.truncateFrom(boundary);
  audit.replayTruncateCount += 1;
}

export function recordReplayBoundary<TState, TTimeline, TSnapshot>(
  replay: Pick<ReplayPort<TState, unknown>, "recordBoundary">,
  audit: RollbackValidationAudit<TState, TTimeline, TSnapshot>,
  boundary: number,
  state: TState,
): void {
  replay.recordBoundary(boundary, state);
  audit.replayRecordCount += 1;
}

export function recordReplaySample<TState extends ReplaySampleState, TTimeline, TSnapshot>(
  replay: Pick<ReplayPort<TState, unknown>, "recordBoundary">,
  audit: RollbackValidationAudit<TState, TTimeline, TSnapshot>,
  boundary: number,
  sample: RollbackValidationReplaySample,
): void {
  if (sample.boundary !== boundary) {
    throw new Error("replay sample boundary does not match its trace row");
  }
  const state = audit.replayState;
  state.input_tick = boundary;
  state.ball = new Vec2(sample.ball_x, sample.ball_y);
  state.score.home = sample.score_home;
  state.score.away = sample.score_away;
  recordReplayBoundary(replay, audit, boundary, state);
}

function equalBoundaries(left: readonly number[], right: readonly number[]): boolean {
  if (left.length !== right.length) {
    return false;
  }
  return left.every((value, index) => right[index] === value);
}

export function expectedReplayBoundaries(replayCapacity: number, finalBoundary: number): number[] {
  if (!(Number.isInteger(finalBoundary) && finalBoundary >= 0)) {
    throw new Error("expected replay final boundary must be a non-negative integer");
  }
  const first = Math.max(0, finalBoundary - replayCapacity + 1);
  const boundaries: number[] = [];
  for (let boundary = first; boundary <= finalBoundary; boundary += 1) {
    boundaries.push(boundary);
  }
  return boundaries;
}

function replaySamplesMatch(
  replay: Pick<ReplayPort<unknown, unknown>, "boundarySample">,
  expected: readonly RollbackValidationReplaySample[] | undefined,
): boolean {
  for (const sample of expected ?? []) {
    const actual = replay.boundarySample(sample.boundary);
    if (
      actual === undefined ||
      actual.ball_x !== sample.ball_x ||
      actual.ball_y !== sample.ball_y ||
      actual.score_home !== sample.score_home ||
      actual.score_away !== sample.score_away
    ) {
      return false;
    }
  }
  return true;
}

function sumCounts(values: Readonly<Record<string, number>>): number {
  return Object.values(values).reduce((total, value) => total + value, 0);
}

function observerDigest(
  summary: ObservedMatchSummary,
  numberBytes: (value: number) => string,
): string {
  return stableValue(summary, numberBytes);
}

function resultDigest(result: ProductMatchResult, numberBytes: (value: number) => string): string {
  return stableValue(result, numberBytes);
}

function finalScore(
  supplied: { readonly home: number; readonly away: number } | undefined,
  state: { readonly score: { readonly home: number; readonly away: number } } | undefined,
  fallback: { readonly home: number; readonly away: number },
): { readonly home: number; readonly away: number } {
  if (supplied) {
    return supplied;
  } else if (state) {
    return state.score;
  }
  return fallback;
}

function expectedAudioCues<TState, TTimeline, TSnapshot>(
  audit: RollbackValidationAudit<TState, TTimeline, TSnapshot>,
  combatFeedback: CombatFeedbackPort,
): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const id of audit.referenceIds.keys()) {
    const event = audit.referenceEvents.get(id);
    if (!event) {
      throw new Error("reference event missing for a recorded id");
    }
    const cue = audioCue(event, combatFeedback);
    if (cue !== undefined) {
      counts[cue] = (counts[cue] ?? 0) + 1;
    }
  }
  return counts;
}

function requireCondition(errors: string[], condition: boolean, message: string): void {
  if (!condition) {
    errors.push(message);
  }
}

export function finish<
  TState extends ObservedMatchState & ReplaySampleState,
  TCombatState,
  TSnapshot,
  TTimeline,
>(
  ports: RollbackValidationPorts<TState, TCombatState, TSnapshot, TTimeline>,
  audit: RollbackValidationAudit<TState, TTimeline, TSnapshot>,
  options: RollbackValidationFinishOptions<TState>,
): RollbackValidationReport {
  const errors: string[] = [];
  for (const [id, signature] of audit.referenceIds) {
    const impaired = audit.impairedIds.get(id);
    if (impaired === undefined) {
      audit.events.missing_confirmed += 1;
    } else if (impaired !== signature) {
      audit.events.mismatched_confirmed += 1;
    }
  }
  for (const id of audit.impairedIds.keys()) {
    if (!audit.referenceIds.has(id)) {
      audit.events.unexpected_confirmed += 1;
    }
  }
  audit.events.terminal_speculative_residue += audit.speculativeIds.size;
  const effectDiagnostics = ports.effects.diagnostics();
  audit.events.consumer_speculative_residue = effectDiagnostics.speculative_ids.length;

  const referenceSummary = matchObserver.finish(audit.referenceObserver);
  const impairedSummary = matchObserver.finish(audit.impairedObserver);
  const referenceObserverDigest = observerDigest(referenceSummary, ports.matchSnapshot.numberBytes);
  const impairedObserverDigest = observerDigest(impairedSummary, ports.matchSnapshot.numberBytes);
  const observersMatch = referenceObserverDigest === impairedObserverDigest;
  const referenceScore = finalScore(
    options.reference_final_score,
    options.reference_final_state,
    audit.referenceScore,
  );
  const impairedScore = finalScore(
    options.impaired_final_score,
    options.impaired_final_state,
    audit.impairedScore,
  );

  const referenceResult = matchContract.newResult(
    { teams: buildTeamsForDigest(options.home_team_id, options.away_team_id) },
    {
      home_team_id: options.home_team_id,
      away_team_id: options.away_team_id,
      home_score: referenceScore.home,
      away_score: referenceScore.away,
      ...(referenceSummary.mvp_player_id !== undefined
        ? { mvp_player_id: referenceSummary.mvp_player_id }
        : {}),
      ...(referenceSummary.mvp_summary !== undefined
        ? { mvp_summary: referenceSummary.mvp_summary }
        : {}),
      home_stats: referenceSummary.home_stats,
      away_stats: referenceSummary.away_stats,
      ...(options.seed !== undefined ? { seed: options.seed } : {}),
    },
  );
  const impairedResult = matchContract.newResult(
    { teams: buildTeamsForDigest(options.home_team_id, options.away_team_id) },
    {
      home_team_id: options.home_team_id,
      away_team_id: options.away_team_id,
      home_score: impairedScore.home,
      away_score: impairedScore.away,
      ...(impairedSummary.mvp_player_id !== undefined
        ? { mvp_player_id: impairedSummary.mvp_player_id }
        : {}),
      ...(impairedSummary.mvp_summary !== undefined
        ? { mvp_summary: impairedSummary.mvp_summary }
        : {}),
      home_stats: impairedSummary.home_stats,
      away_stats: impairedSummary.away_stats,
      ...(options.seed !== undefined ? { seed: options.seed } : {}),
    },
  );
  if (!referenceResult.ok || !impairedResult.ok) {
    throw new Error("rollback validation result construction failed");
  }
  const referenceResultDigest = resultDigest(
    referenceResult.value,
    ports.matchSnapshot.numberBytes,
  );
  const impairedResultDigest = resultDigest(impairedResult.value, ports.matchSnapshot.numberBytes);
  const resultsMatch = referenceResultDigest === impairedResultDigest;

  const replayDiagnostics = ports.replay.diagnostics();
  const replayMatched =
    (options.expected_replay_boundaries === undefined ||
      equalBoundaries(replayDiagnostics.boundaries, options.expected_replay_boundaries)) &&
    replaySamplesMatch(ports.replay, options.expected_replay_samples) &&
    (options.expected_replay_truncate_count === undefined ||
      audit.replayTruncateCount === options.expected_replay_truncate_count);
  const actualAudioCues = ports.audio.confirmedCueCounts();
  const expectedCues = expectedAudioCues(audit, ports.combatFeedback);

  requireCondition(errors, audit.events.missing_confirmed === 0, "confirmed events are missing");
  requireCondition(
    errors,
    audit.events.unexpected_confirmed === 0,
    "unexpected confirmed events were presented",
  );
  requireCondition(
    errors,
    audit.events.mismatched_confirmed === 0,
    "stable confirmed event IDs changed payload",
  );
  requireCondition(
    errors,
    audit.events.confirmed_without_speculation === 0,
    "confirmed events were missing from the speculative presentation timeline",
  );
  requireCondition(
    errors,
    audit.events.speculative_unknown_revoked === 0,
    "speculative revocation referenced an unknown event",
  );
  requireCondition(
    errors,
    audit.events.speculative_invalid_replaced === 0,
    "speculative replacement did not match the active event",
  );
  requireCondition(
    errors,
    audit.events.terminal_speculative_residue === 0,
    "speculative event ledger retained terminal residue",
  );
  requireCondition(
    errors,
    audit.events.consumer_speculative_residue === 0,
    "effects consumer retained terminal speculative residue",
  );
  requireCondition(
    errors,
    stableValue(actualAudioCues, ports.matchSnapshot.numberBytes) ===
      stableValue(expectedCues, ports.matchSnapshot.numberBytes),
    "confirmed audio cue counts differ from authority",
  );
  requireCondition(
    errors,
    audit.referenceObserverSteps === audit.impairedObserverSteps,
    "confirmed observer step count differs from authority",
  );
  requireCondition(errors, observersMatch, "observer statistics differ from authority");
  requireCondition(errors, resultsMatch, "product result differs from authority");
  requireCondition(errors, replayMatched, "replay boundaries differ from the expected timeline");
  for (const scenario of options.required_scenarios ?? []) {
    requireCondition(
      errors,
      audit.scenarioCounts[scenario] > 0,
      `required rollback scenario is missing: ${scenario}`,
    );
  }

  return {
    passed: errors.length === 0,
    errors,
    events: audit.events,
    consumers: {
      audio_cue_count: sumCounts(actualAudioCues),
      expected_audio_cue_count: sumCounts(expectedCues),
      audio_cues: actualAudioCues,
      expected_audio_cues: expectedCues,
      reference_observer_steps: audit.referenceObserverSteps,
      impaired_observer_steps: audit.impairedObserverSteps,
      replay_record_count: audit.replayRecordCount,
      replay_truncate_count: audit.replayTruncateCount,
      replay_boundary_count: replayDiagnostics.count,
      result_count: 2,
    },
    scenario_counts: audit.scenarioCounts,
    observer_matched: observersMatch,
    observer_reference_digest: referenceObserverDigest,
    observer_impaired_digest: impairedObserverDigest,
    result_matched: resultsMatch,
    result_reference_digest: referenceResultDigest,
    result_impaired_digest: impairedResultDigest,
    replay_matched: replayMatched,
    replay_boundaries: replayDiagnostics.boundaries,
  };
}

// `matchContract.newResult` only reads `teams[id].{id,name}`
// (match_contract.ts) to stamp the digest inputs; the caller's real team
// table is not needed for a stable digest comparison between the reference
// and impaired runs, only that both sides use the same one.
function buildTeamsForDigest(
  homeTeamId: string,
  awayTeamId: string,
): Readonly<Record<string, { readonly id: string; readonly name: string }>> {
  return {
    [homeTeamId]: { id: homeTeamId, name: homeTeamId },
    [awayTeamId]: { id: awayTeamId, name: awayTeamId },
  };
}

function observeRawReferenceStep<
  TState extends ObservedMatchState & ReplaySampleState,
  TTimeline,
  TSnapshot,
>(
  rollbackEvents: RollbackEventsPort<TTimeline, TSnapshot>,
  numberBytes: (value: number) => string,
  audit: RollbackValidationAudit<TState, TTimeline, TSnapshot>,
  supplied: RollbackEventStepInput<TSnapshot>,
): void {
  const tick = supplied.output.tick;
  const applied = rollbackEvents.apply(audit.referenceTimeline, tick, tick, [supplied]);
  if (!applied.ok) {
    throw new Error(applied.error.message);
  }
  const confirmed = rollbackEvents.confirm(audit.referenceTimeline, tick);
  if (confirmed.length !== 1) {
    throw new Error("reference rollback validation step did not confirm exactly once");
  }
  const first = confirmed[0];
  if (!first) {
    throw new Error("reference rollback validation step confirmed no steps");
  }
  observeReferenceStep(numberBytes, audit, first);
}

export function run<
  TState extends ObservedMatchState & ReplaySampleState,
  TCombatState,
  TSnapshot,
  TTimeline,
>(
  ports: RollbackValidationPorts<TState, TCombatState, TSnapshot, TTimeline>,
  initialState: TState,
  trace: readonly RollbackValidationTraceRow<TState, TSnapshot>[],
  finishOptions: RollbackValidationFinishOptions<TState> & {
    readonly initial_combat_state?: TCombatState;
  },
): RollbackValidationReport {
  const audit = newAudit(ports, initialState, finishOptions.initial_combat_state);
  for (const row of trace) {
    if (row.kind === "reference_step") {
      observeRawReferenceStep(
        ports.rollbackEvents,
        ports.matchSnapshot.numberBytes,
        audit,
        row.step as RollbackEventStepInput<TSnapshot>,
      );
    } else if (row.kind === "reference_confirmed") {
      observeReferenceStep(ports.matchSnapshot.numberBytes, audit, row.step as RollbackEventStep);
    } else if (row.kind === "impaired_diff") {
      if (!row.diff) {
        throw new Error("impaired_diff trace row is missing its diff");
      }
      applyImpairedDiff(ports.effects, audit, row.diff, ports.matchSnapshot.numberBytes);
    } else if (row.kind === "confirmed" || row.kind === "impaired_confirmed") {
      observeImpairedStep(
        {
          numberBytes: ports.matchSnapshot.numberBytes,
          effects: ports.effects,
          audio: ports.audio,
        },
        audit,
        row.step as RollbackEventStep,
      );
    } else if (row.kind === "replay_truncate") {
      if (row.boundary === undefined) {
        throw new Error("replay_truncate trace row is missing its boundary");
      }
      truncateReplay(ports.replay, audit, row.boundary);
    } else if (row.kind === "replay_boundary") {
      if (row.boundary === undefined) {
        throw new Error("replay_boundary trace row is missing its boundary");
      }
      if (row.replaySample) {
        recordReplaySample(ports.replay, audit, row.boundary, row.replaySample);
      } else {
        const state =
          row.state ??
          (row.snapshot !== undefined ? ports.matchSnapshot.restore(row.snapshot) : undefined);
        if (state === undefined) {
          throw new Error("replay validation boundary is missing its snapshot");
        }
        recordReplayBoundary(ports.replay, audit, row.boundary, state);
      }
    } else {
      throw new Error(
        `unknown rollback validation trace row: ${String((row as { kind: string }).kind)}`,
      );
    }
  }
  return finish(ports, audit, finishOptions);
}

export const rollbackValidation = {
  new: newAudit,
  observeReferenceStep,
  applyImpairedDiff,
  observeImpairedStep,
  truncateReplay,
  recordReplayBoundary,
  recordReplaySample,
  expectedReplayBoundaries,
  finish,
  run,
};
