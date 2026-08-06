// Ported from game/screens/match.lua -- the playable 5v5 match screen.
//
// # Scope this milestone
//
// The full Lua screen is a `love`-driven game loop: it gathers real-time
// keyboard/gamepad input, steps `sim.match` (Rust; no wasm bridge exists
// this milestone -- v2/README.md §1), and draws through
// `game/render/**` (three.js's `@gc/render`, not a declared dependency of
// this package). None of that can run in this milestone regardless of how
// it is ported, and `match_screen_spec.lua`/`match_gamepad_spec.lua`/
// `match_rollback_lab_spec.lua` all drive exactly that loop end to end
// (stubbed `love.keyboard`/`love.joystick`, a real `sim.match` stepping
// real physics) -- see this package's porting report. They are ported as
// `it.skip` in their own spec files, one unblocker per file header.
//
// What *is* in scope, and genuinely portable, is the rollback-consumption
// seam this task was scoped to: `consume_rollback_event_diff`,
// `consume_confirmed_step`, and `consume_confirmed_lifecycle`. Those three
// methods are pure translations of an already-decided rollback batch into
// presentation state -- they read `RollbackEventDiff`/`RollbackEventStep`
// records and call into `effects`/`audio`/`replay`/`combat_feedback`, never
// into the sim. `combat_feedback` and `match_event_batch` are already
// real, ported modules (`@gc/presentation`, a declared dependency) and are
// used directly. `game/render/effects.lua` and `game/audio.lua` do not
// exist in TypeScript yet; `game/render/replay.lua` exists in `@gc/render`
// but that package is not a declared dependency of this one. All three are
// therefore injected ports (`EffectsPort`/`AudioPort`/`ReplayPort`),
// following the pattern set by `@gc/online`'s `match_presentation.ts`.
// `combat_feedback_rollback_spec.ts` exercises this module for real against
// small hand-written fakes for those three ports -- see its header.

import { combatFeedback, matchEventBatch } from "@gc/presentation";
import type { CombatEvent, CombatFeedbackState, MatchEvent, RollbackEventDiff, RollbackWrappedEvent } from "@gc/presentation";
import type { LifecyclePayload } from "./online_match_model.ts";

export type { LifecyclePayload } from "./online_match_model.ts";

export type RollbackWrappedMatchEvent = RollbackWrappedEvent<MatchEvent>;
export type RollbackWrappedCombatEvent = RollbackWrappedEvent<CombatEvent>;
export type RollbackWrappedLifecycleEvent = RollbackWrappedEvent<LifecyclePayload>;

/** `sim.rollback_events`'s `RollbackConfirmedStateView` -- only the shape this seam reads. */
export interface RollbackConfirmedStateView {
  readonly score: { readonly home: number; readonly away: number };
  readonly time_left: number;
  readonly finished: boolean;
  readonly owner_id?: string;
  readonly owner_team?: "home" | "away";
}

/** `sim.rollback_events`'s `RollbackEventStep`. */
export interface RollbackEventStep {
  readonly tick: number;
  readonly start_boundary: number;
  readonly end_boundary: number;
  readonly state: RollbackConfirmedStateView;
  readonly match_events: readonly RollbackWrappedMatchEvent[];
  readonly combat_events?: readonly RollbackWrappedCombatEvent[];
  readonly lifecycle_events: readonly RollbackWrappedLifecycleEvent[];
}

/** `game/render/effects.lua`, injected -- see this module's header. */
export interface EffectsPort {
  reset(): void;
  resetVisuals(): void;
  resetTrail(): void;
  applyEventDiff(diff: RollbackEventDiff): void;
  discardEventDiff(diff: RollbackEventDiff): void;
  confirmEvent(event: RollbackWrappedMatchEvent | RollbackWrappedCombatEvent, replayOwnsScreen: boolean): void;
}

/** `game/audio.lua`, injected -- see this module's header. */
export interface AudioPort {
  /** @returns whether this event id was newly consumed (dedup), regardless of whether it audibly played. */
  consumeConfirmed(
    event: RollbackWrappedMatchEvent | RollbackWrappedCombatEvent | RollbackWrappedLifecycleEvent,
    replayOwnsScreen?: boolean
  ): boolean;
}

/** `game/render/replay.lua`, injected -- see this module's header. */
export interface ReplayPort {
  active(): boolean;
  startAt(team: "home" | "away", tick: number): boolean;
  resetVisuals?(): void;
}

export interface MatchRollbackConsumerPorts {
  readonly effects: EffectsPort;
  readonly audio: AudioPort;
  readonly replay: ReplayPort;
}

/**
 * The screen-owned slice of `MatchScreen` this seam mutates: the confirmed
 * lifecycle ledger, the combat feedback state, and the small booleans the
 * Lua original keeps as `self._*` fields. Everything else on the real
 * screen (`self.state`, `self._source`, `self._render_smoothing`, ...) is
 * out of scope this milestone -- see this module's header.
 */
export interface MatchRollbackConsumerState {
  readonly combat_feedback: CombatFeedbackState;
  readonly confirmed_lifecycle_ids: Set<string>;
  last_scoring_team?: "home" | "away";
  pending_confirmed_kickoff: boolean;
  presentation_full_time: boolean;
  /** Seconds remaining on the kickoff banner; sourced by whatever draws it. */
  kickoff_banner: number;
  replay_state_cleared: boolean;
}

export function newMatchRollbackConsumerState(): MatchRollbackConsumerState {
  return {
    combat_feedback: combatFeedback.new(),
    confirmed_lifecycle_ids: new Set(),
    pending_confirmed_kickoff: false,
    presentation_full_time: false,
    kickoff_banner: 1.15,
    replay_state_cleared: false,
  };
}

// Consume one confirmed lifecycle record exactly once. The screen-owned
// ledger gates every associated side effect; audio deduplication is a
// second guard, not the authority that decides whether replay/HUD/result
// state may restart.
export function consumeConfirmedLifecycle(
  state: MatchRollbackConsumerState,
  ports: MatchRollbackConsumerPorts,
  event: RollbackWrappedLifecycleEvent
): boolean {
  if (state.confirmed_lifecycle_ids.has(event.id)) {
    return false;
  }
  state.confirmed_lifecycle_ids.add(event.id);
  ports.audio.consumeConfirmed(event);
  const payload = event.payload;
  if (payload.kind === "goal") {
    state.last_scoring_team = payload.team;
    if (ports.replay.startAt(payload.team, event.tick)) {
      ports.effects.resetVisuals();
      combatFeedback.resetVisuals(state.combat_feedback);
      state.replay_state_cleared = true;
    }
  } else if (payload.kind === "kickoff") {
    state.pending_confirmed_kickoff = true;
    if (!ports.replay.active() && !state.presentation_full_time) {
      state.kickoff_banner = 1.15;
      state.pending_confirmed_kickoff = false;
    }
  } else if (payload.kind === "full_time") {
    state.presentation_full_time = true;
    state.pending_confirmed_kickoff = false;
  }
  return true;
}

// Route rollback action deltas through the currently drawn timeline. Live
// effects are consumed reversibly but never spawned over confirmed past
// footage.
export function consumeRollbackEventDiff(
  state: MatchRollbackConsumerState,
  ports: MatchRollbackConsumerPorts,
  hasSource: boolean,
  diff: RollbackEventDiff
): boolean {
  if (hasSource && ports.replay.active()) {
    ports.effects.discardEventDiff(diff);
    return false;
  }
  ports.effects.applyEventDiff(diff);
  return true;
}

// Publish one stable rollback step to presentation consumers. Match and
// combat action records share the same confirmation ledger; lifecycle
// records retain their screen-owned transitions on top.
export function consumeConfirmedStep(
  state: MatchRollbackConsumerState,
  ports: MatchRollbackConsumerPorts,
  step: RollbackEventStep
): number {
  let consumedCount = 0;
  // Lifecycle owns the presentation boundary. A goal confirmed in the same
  // batch as action events must start replay before those actions are
  // published, so no live-timeline cue can flash over past footage.
  for (const event of step.lifecycle_events) {
    if (consumeConfirmedLifecycle(state, ports, event)) {
      consumedCount += 1;
    }
  }
  const replayOwnsScreen = ports.replay.active();
  for (const event of step.match_events) {
    ports.effects.confirmEvent(event, replayOwnsScreen);
    if (ports.audio.consumeConfirmed(event, replayOwnsScreen)) {
      consumedCount += 1;
    }
  }
  for (const event of step.combat_events ?? []) {
    ports.effects.confirmEvent(event, replayOwnsScreen);
    const link = combatFeedback.link(event.payload, event.id);
    combatFeedback.confirm(state.combat_feedback, link);
    if (replayOwnsScreen) {
      combatFeedback.resetVisuals(state.combat_feedback);
    }
    if (ports.audio.consumeConfirmed(event, replayOwnsScreen)) {
      consumedCount += 1;
    }
  }
  return consumedCount;
}

// Consume one driver/lab batch's worth of diffs and confirmed steps in
// order -- the render-update-time counterpart of the three functions above.
export function consumeRollbackPresentation(
  state: MatchRollbackConsumerState,
  ports: MatchRollbackConsumerPorts,
  hasSource: boolean,
  frameEvents: MatchEvent[],
  eventDiffs: readonly RollbackEventDiff[],
  confirmedSteps: readonly RollbackEventStep[]
): void {
  for (const event of matchEventBatch.surviving(eventDiffs)) {
    frameEvents.push(event);
  }
  for (const diff of eventDiffs) {
    consumeRollbackEventDiff(state, ports, hasSource, diff);
  }
  for (const step of confirmedSteps) {
    consumeConfirmedStep(state, ports, step);
  }
  if (state.pending_confirmed_kickoff && !ports.replay.active()) {
    state.kickoff_banner = 1.15;
    state.pending_confirmed_kickoff = false;
  }
}
