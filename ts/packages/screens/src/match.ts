// MatchScreen: the playable 5v5 match screen.
//
// # Scope this milestone
//
// A fully interactive game loop needs real-time keyboard/gamepad input,
// stepping `sim.match` (Rust; no wasm bridge exists this milestone),
// and drawing through `@gc/render` (three.js), not a
// declared dependency of this package. None of that
// can run in this milestone, and `match_screen.spec.ts`/
// `match_gamepad.spec.ts`/`match_rollback_lab.spec.ts` all drive exactly
// that loop end to end (a real `sim.match` stepping real physics) --
// see each spec file's own header for what unblocks it. Those cases are
// `it.skip` in their own spec files, one unblocker per file header.
//
// What *is* in scope, and fully implemented, is the rollback-consumption
// seam this module is built around: `consume_rollback_event_diff`,
// `consume_confirmed_step`, and `consume_confirmed_lifecycle`. Those three
// methods are pure translations of an already-decided rollback batch into
// presentation state -- they read `RollbackEventDiff`/`RollbackEventStep`
// records and call into `effects`/`audio`/`replay`/`combat_feedback`, never
// into the sim. `combat_feedback` and `match_event_batch` are real modules
// (`@gc/presentation`, a declared dependency) and are used directly.
// Effects and audio have no implementation of their own in this package;
// replay exists in `@gc/render` but that package is not a declared
// dependency of this one. All three are therefore injected ports
// (`EffectsPort`/`AudioPort`/`ReplayPort`), following the same
// dependency-injection pattern `@gc/online`'s `match_presentation.ts` uses.
// `combat_feedback_rollback_spec.ts` exercises this module for real against
// small hand-written fakes for those three ports -- see its header.

import { combatFeedback, matchEventBatch } from "@gc/presentation";
import type {
  CombatEvent,
  CombatFeedbackState,
  MatchEvent,
  RollbackEventDiff,
  RollbackWrappedEvent,
} from "@gc/presentation";
import { Vec2 } from "@gc/core";
import {
  cameraFollow,
  correctionSmoothing,
  dispossessionFlinch,
  player3dPrewarmCharacters,
  player3dResetAnimation,
  releaseFollow,
  viewState,
} from "@gc/render";
import type { correctionSmoothingTypes, replayTypes } from "@gc/render";
import type { LifecyclePayload } from "./online_match_model.ts";
import type { GameSettings } from "./content.ts";
import type {
  RealMatchEvent,
  RealMatchInputEvent,
  RealMatchRosterEntry,
  RealMatchScreenPort,
  RealMatchState,
} from "./real_match.ts";
import type { OnlineMatchState } from "./online_match.ts";
import { bindings, captureFrame, controller, inputSample } from "@gc/input";
import type {
  ActionName,
  ControllerInputEvent,
  GamepadState,
  InputSource,
  KeyboardState,
  ViewportMapper,
  ViewportTransform,
  captureFrameTypes,
  inputSampleTypes,
} from "@gc/input";

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

/** Visual match effects (goal celebrations, trail resets, event-driven cues), injected -- see this module's header. */
export interface EffectsPort {
  reset(): void;
  resetVisuals(): void;
  resetTrail(): void;
  applyEventDiff(diff: RollbackEventDiff): void;
  discardEventDiff(diff: RollbackEventDiff): void;
  confirmEvent(
    event: RollbackWrappedMatchEvent | RollbackWrappedCombatEvent,
    replayOwnsScreen: boolean,
  ): void;
}

/** Match sound effects for confirmed events, injected -- see this module's header. */
export interface AudioPort {
  /** @returns whether this event id was newly consumed (dedup), regardless of whether it audibly played. */
  consumeConfirmed(
    event: RollbackWrappedMatchEvent | RollbackWrappedCombatEvent | RollbackWrappedLifecycleEvent,
    replayOwnsScreen?: boolean,
  ): boolean;
}

/**
 * Goal-replay capture and playback, injected -- see this module's header.
 *
 * `record`/`start`/`step`/`stop` are the BASE (non-rollback) goal-replay
 * seam -- `@gc/render`'s real `replay` module already implements all four
 * (`record`, `start`, `step`, `stop`, alongside `active`/`startAt`/`reset`
 * above), so a caller wiring the real module in satisfies this whole
 * interface for free; see this file's "THE GAME LOOP" section for how
 * `MatchScreen.update`/`event` use them. All four are optional so existing
 * rollback-only fakes (e.g. `combat_feedback_rollback.spec.ts`'s,
 * `match_rollback_lab.spec.ts`'s `FakeReplay`) need not grow them -- the
 * base game loop's goal replay simply stays unwired when they are absent,
 * matching this class's behavior before this capability existed.
 */
export interface ReplayPort {
  active(): boolean;
  startAt(team: "home" | "away", tick: number): boolean;
  resetVisuals?(): void;
  /** `replay.reset()` -- discards any buffered footage/active sequence. Optional so existing fakes (e.g. `combat_feedback_rollback.spec.ts`'s) need not grow it; only the rollback laboratory's restart path calls it. */
  reset?(): void;
  /** Record one live pre-step frame under this port's own private, contiguous boundary sequence -- `replay.record`. Called once per simulated tick, before `SimHostPort.step`, so a goal's flight remains in the buffer. */
  record?(state: replayTypes.MatchState): void;
  /** Begin a sequence at the newest recorded frame -- `replay.start`. Returns whether enough footage was buffered to start (matches `replay.start`'s own "refuses to start without enough footage" contract). */
  start?(team: "home" | "away"): boolean;
  /** Advance the active sequence by `dt` render seconds, returning the frame to display/animate from, or `undefined` once the sequence has finished -- `replay.step`. */
  step?(dt: number): replayTypes.ReplayFrame | undefined;
  /** Stop/skip the active sequence immediately -- `replay.stop`. */
  stop?(): void;
}

export interface MatchRollbackConsumerPorts {
  readonly effects: EffectsPort;
  readonly audio: AudioPort;
  readonly replay: ReplayPort;
}

/**
 * The screen-owned slice of `MatchScreen` this seam mutates: the confirmed
 * lifecycle ledger, the combat feedback state, and a handful of small
 * internal booleans. Everything else on the real screen (`state`,
 * `host`/`rollbackHost`/`onlineHost`, `renderSmoothing`, ...) is out of
 * scope this milestone -- see this module's header.
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
  event: RollbackWrappedLifecycleEvent,
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
  diff: RollbackEventDiff,
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
  step: RollbackEventStep,
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
  confirmedSteps: readonly RollbackEventStep[],
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

// =============================================================================
// THE GAME LOOP -- gather input, step the fixed simulation clock, hand frames
// to the renderer. This is the part of the match screen this package's
// original scope note (this module's header) said could not be built
// without a wasm bridge. That bridge (`crates/gc-wasm`, `@gc/wasm`) now
// exists, so this section builds it.
//
// ## `SimHostPort` -- not this module's free design
//
// `SimHostPort` below is a fixed contract, not something this file invented:
// `@gc/app` (`app/src/sim_host.ts`) implements the exact same shape.
// `@gc/screens` cannot depend on `@gc/app` (the dependency runs the other
// way -- `@gc/app` assembles screens, not the reverse), so the interface is
// declared here, independently, and the two files are expected to
// structurally agree.
//
// They used to NOT fully agree: `app/src/sim_host.ts`'s `RenderFrame` was
// `@gc/render`'s `pitchTypes.RenderFrame` (`packages/render/src/pitch.ts`)
// -- a slice sized for DRAWING (`field`/`roster`/`players`/`ball`/`control`)
// that omitted `hud`/`possession`. This module's job is different from
// drawing: it has to decide whether the match is over (`hud.finished`) and
// whether the controlled player has the ball (`hud.controlled_owns_ball`)
// before it can compute a single contextual input field, or expose
// `hud.home_score`/`away_score`/`time_left` to
// [`MatchScreenAsRealMatchScreen`]'s `state` (`real_match.ts`'s
// `RealMatchScreenPort`). `@gc/render`'s real `frame_buffer.ts` decoder
// always produced all of these -- the gap was that `sim_host.ts` used to
// carry its own trimmed, hand-duplicated decoder that only constructed the
// drawing slice. `@gc/render` now exports `frame_buffer` directly
// (`frameBuffer`), so `sim_host.ts` decodes through the real, untrimmed
// `decode`/`decodeRoster`/`toRenderFrame` and its `RenderFrame` is that
// module's complete wire type -- see that file's header. `RenderFrameHud`
// below only declares the fields THIS module's own game-loop logic reads
// (ARCHITECTURE.md §4 rule 6): the wire carries more (`possession_team`,
// `controlled_stamina`, ...), but nothing here inspects them.
//
// `field`/`players`/`ball`/`control`/`events`/`roster`'s per-player arrays
// are NOT declared here: this module never reads them, only forwards
// whatever `SimHostPort.frame()`/`.roster()` produced to [`RenderPort.draw`]
// untouched (the same "only the fields this module reads are declared
// locally" rule `pitch.ts` documents for its own slice, ARCHITECTURE.md §4 rule 6).
// =============================================================================

/** `gc_sim::input_frame::InputSample` (`@gc/input`'s TS mirror; see `input_sample.ts`'s header). Re-exported so a `SimHostPort` implementation need not also import `@gc/input` merely to name this type. */
export type InputSample = inputSampleTypes.InputSample;

/** `crates/gc-render/src/frame.rs`'s `RenderFrameHud` (mirrored via `@gc/render`'s `frame_buffer.ts`'s `RenderFrameHud`, field-for-field) -- only the fields this module's own game-loop logic reads. See this section's header. */
export interface RenderFrameHud {
  readonly finished: boolean;
  /** Whether the controlled roster slot currently owns the ball -- "carrying". */
  readonly controlled_owns_ball: boolean;
  /** Read by [`MatchScreenAsRealMatchScreen`]'s `state` (`real_match.ts`'s `RealMatchState.score.home`). */
  readonly home_score: number;
  /** Read by [`MatchScreenAsRealMatchScreen`]'s `state` (`real_match.ts`'s `RealMatchState.score.away`). */
  readonly away_score: number;
  /** Read by [`MatchScreenAsRealMatchScreen`]'s `state` (`real_match.ts`'s `RealMatchState.time_left`). */
  readonly time_left: number;
  /**
   * Roster slot, one-based -- present on every real wire frame
   * (`@gc/render`'s `frame_buffer.ts`'s `RenderFrameHud`), but only read now
   * that ONLINE mode needs a fallback for [`MatchScreen.onlineState`]'s
   * `controlled` when {@link OnlineHostPort.controlledPlayer} itself returns
   * `undefined` -- see that getter's doc. Base/rollback fakes that never
   * populate it (`FakeSimHost`, `FakeRollbackHost`) are unaffected: this
   * field is optional precisely so they stay valid `RenderFrameHud` values.
   */
  readonly controlled?: number;
}

/**
 * `crates/gc-render/src/frame.rs`'s `RenderFramePossession`. `owner` is now
 * read in BASE/ROLLBACK mode too -- see [`MatchScreen.matchObservationState`]
 * -- to populate [`real_match.ts`'s `RealMatchState.owner`] for
 * `@gc/app`'s `match_observer.ts`'s possession stat; carrying (whether the controlled
 * roster slot owns the ball) still comes off `hud.controlled_owns_ball`, a
 * separate concern from this getter.
 */
export interface RenderFramePossession {
  readonly owner?: number;
  readonly owner_team?: "home" | "away";
}

/**
 * The minimal slice of `crates/gc-render/src/frame.rs`'s `RenderFrameEvents`
 * this module's BASE/ROLLBACK game loop reads, to populate
 * [`MatchScreenAsRealMatchScreen.frameEvents`] for `@gc/app`'s `match_observer.ts`'s
 * shot/save/pass attribution. Structure-of-arrays, one entry per event this
 * tick, mirroring the real wire shape's own SoA layout (`@gc/render`'s
 * `frame_buffer.ts`'s `RenderFrameEvents`) narrowed to the two fields that
 * matter here. `slot` is the actor's one-based roster slot, absent if
 * unattributed -- `player` (a roster id) is dropped from the wire, same as
 * the real `RenderFrameEvents.slot` doc; recovered to an id via
 * [`SimHostPort.roster`]'s `ids`, the same pattern [`MatchScreen.onlineState`]
 * already uses for `players[]`.
 */
export interface RealMatchRenderFrameEvents {
  readonly count: number;
  /** Plain strings, not {@link RealMatchEventKind}: this is the HOST's wire
   * vocabulary (~25 kinds), of which that alias names only the subset
   * `@gc/app`'s `match_observer.ts` acts on -- see {@link RealMatchEvent}. */
  readonly kind: readonly string[];
  readonly slot: readonly (number | undefined)[];
}

/**
 * The minimal slice of `crates/gc-render/src/frame.rs`'s `RenderFramePlayers`
 * (structure-of-arrays, one entry per roster slot) this module's ONLINE mode
 * reads -- to build {@link OnlineMatchState.players} and to correct the
 * render highlight (`controlled`) once {@link OnlineHostPort.controlledPlayer}
 * has decided the real assignment. NOT part of the general opaque
 * `players`/`ball`/`control`/`events`/`roster` fields this file otherwise
 * never inspects (see [`RenderFrame`]'s own doc) -- a real `@gc/render`
 * `frame_buffer.ts` decode already produces a structurally wider object that
 * satisfies this narrower read.
 */
export interface OnlineRenderFramePlayers {
  readonly count: number;
  readonly x: readonly number[];
  readonly y: readonly number[];
  readonly facing_x: readonly number[];
  readonly facing_y: readonly number[];
  /** Per-slot render highlight -- see this interface's doc. */
  readonly controlled: readonly boolean[];
}

/**
 * The minimal slice of `crates/gc-render/src/frame.rs`'s `RenderFrameControl`
 * this module's ONLINE mode reads/overrides -- see
 * [`OnlineRenderFramePlayers`]'s doc for why this is narrower than, but
 * structurally compatible with, the real wire shape.
 */
export interface OnlineRenderFrameControl {
  readonly pass_target?: number;
  readonly charge_kind?: "shot" | "pass";
  readonly charge: number;
  readonly controlled: number;
}

/**
 * `crates/gc-render/src/frame.rs`'s `RenderFrame` -- the whole interface
 * between the simulation and a renderer. `hud`/`possession` are typed here
 * for the BASE/ROLLBACK modes (this module's own reads); `field`/`ball`/
 * `roster` are real fields on the wire object this module receives and
 * forwards to [`RenderPort.draw`], but this module never inspects them, so
 * they are not declared -- see this section's header. `players`/`control`
 * are declared narrowly (see [`OnlineRenderFramePlayers`]/
 * [`OnlineRenderFrameControl`]) -- ONLINE mode reads and overrides them
 * ([`MatchScreen.onlineState`]/`drawOnline`). `events` is declared narrowly
 * too (see [`RealMatchRenderFrameEvents`]) -- BASE/ROLLBACK mode reads it to
 * populate [`MatchScreenAsRealMatchScreen.frameEvents`]. All four are
 * optional so every existing fake (`FakeSimHost`, `FakeRollbackHost`, which
 * never populate them) remains a valid `RenderFrame`.
 */
export interface RenderFrame {
  readonly hud: RenderFrameHud;
  readonly possession: RenderFramePossession;
  readonly players?: OnlineRenderFramePlayers;
  readonly control?: OnlineRenderFrameControl;
  readonly events?: RealMatchRenderFrameEvents;
}

/**
 * `crates/gc-render/src/frame.rs`'s `RenderFrameRoster` -- match-constant
 * per-player identity. Mostly opaque here: this module only threads it from
 * [`SimHostPort.roster`] to [`RenderPort.draw`] for BASE/ROLLBACK modes, the
 * same way `OpaqueSnapshot` stays opaque in `@gc/online`'s
 * `match_presentation.ts`. `ids`/`teams` are read, narrowly, by ONLINE mode
 * ([`MatchScreen.onlineState`]) to build {@link OnlineMatchState.players};
 * `ids`/`teams`/`is_keeper` are now ALSO read, narrowly, by BASE/ROLLBACK
 * mode ([`MatchScreen.matchObservationState`]) to build
 * [`real_match.ts`'s `RealMatchState.roster`] for `@gc/app`'s `match_observer.ts`'s
 * attribution. Every field stays optional so a `Readonly<Record<string,
 * unknown>>` roster (every existing fake) remains a valid
 * `RenderFrameRoster`.
 */
export type RenderFrameRoster = Readonly<Record<string, unknown>> & {
  readonly ids?: readonly string[];
  readonly teams?: readonly ("home" | "away")[];
  readonly is_keeper?: readonly boolean[];
};

/**
 * The seam between this screen and whatever is actually simulating --
 * `@gc/wasm`'s compiled `gc-sim`, in the real build. A fixed contract shared
 * with `@gc/app`'s `sim_host.ts`; see this section's header.
 */
export interface SimHostPort {
  /**
   * Plan how many fixed ticks this render update should simulate, given
   * `dt` seconds elapsed since the last call -- delegates to
   * `gc_sim::fixed_clock`'s accumulator/catch-up/drop policy
   * (ARCHITECTURE.md §1.1) via the wasm session, so this decision has exactly
   * one implementation, in Rust: this screen used to hand-copy that
   * algorithm (see this class's own history/[`MatchScreen.update`]'s doc),
   * which is exactly the kind of drift ARCHITECTURE.md §1.1 warns changing
   * `MAX_TICKS_PER_UPDATE` in Rust alone would silently produce. Call once
   * per render update, then call `step` up to that many times, in order.
   */
  planTicks(dt: number): number;
  /** Advance the simulation by exactly one fixed tick with the given sample. */
  step(sample: InputSample): void;
  /**
   * The caller stopped running `step` before using every tick `planTicks`
   * authorized (e.g. the match finished mid-batch) -- must be called in
   * that case so the clock's carried-over accumulator resets, matching
   * what stopping early means to `gc_sim::fixed_clock::advance`'s own step
   * callback.
   */
  cancelPlannedTicks(): void;
  /** The current frame. Cheap to call; may return a reused object. */
  frame(): RenderFrame;
  /** Match-constant roster. */
  roster(): RenderFrameRoster;
  /** Ticks simulated so far. */
  tick(): number;
  /** Release the host's resources (e.g. a wasm handle). Safe to call twice. */
  dispose(): void;
}

/**
 * Constructs a fresh {@link SimHostPort} with a match's pre-match choices
 * (teams, formation, tactic, seed, ...) already baked in by the caller's
 * closure. `MatchScreen.restart` (the rematch path) calls this again rather
 * than owning any notion of "the same choices" itself -- see
 * `MatchScreenOptions`, which this factory pattern replaces: the options
 * live in whoever builds the factory, not in this screen.
 */
export type SimHostFactory = () => SimHostPort;

/**
 * The pitch renderer (`@gc/render`, three.js), injected -- `@gc/render` is
 * not currently a declared dependency of `@gc/screens` (a needed-dependency
 * gap, not this file's `package.json` to edit). `frame`/`roster` are exactly
 * `SimHostPort.frame()`/`.roster()`'s return values, forwarded untouched;
 * this module never reads them itself past `frame.hud`.
 */
export interface RenderPort {
  draw(frame: RenderFrame, roster: RenderFrameRoster): void;
}

// -----------------------------------------------------------------------
// THE CONTEXTUAL INPUT STATE MACHINE -- shoot vs jockey, pass vs switch, the
// lob latch, aerial strike/acrobatic. `@gc/input`'s `capture_frame.ts`
// deliberately leaves these out of `InputSampleCapture.sample` -- its own
// header names this file, `packages/screens`, as the one that computes them
// from live match state and passes them in via `ContextualInputFields`.
//
// Pure and independently testable: given this frame's raw control polls and
// whether the controlled player is carrying the ball, [`stepMatchControlLatches`]
// mutates the small set of frame-to-frame latches this screen keeps --
// whether SHOOT/PASS/ACTION were held last frame, and the lob latch -- and
// returns exactly the fields `InputSampleCapture.sample` needs,
// `switchPlayer` excepted (see below).
// -----------------------------------------------------------------------

type ContextualInputFields = captureFrameTypes.ContextualInputFields;

/** The frame-to-frame SHOOT/PASS/ACTION-held and lob-latch state this screen tracks across render frames. */
export interface MatchControlLatches {
  shootHeldPrev: boolean;
  passHeldPrev: boolean;
  /** ACTION held off the ball last frame -- the jockey stance's previous-frame flag. */
  actionHeldPrev: boolean;
  lobLatch: boolean;
}

/** A fresh, all-false set of latches. */
export function newMatchControlLatches(): MatchControlLatches {
  return { shootHeldPrev: false, passHeldPrev: false, actionHeldPrev: false, lobLatch: false };
}

/** This frame's raw, context-free control polls -- whether ACTION/PLAY/MODIFIER are currently held. */
export interface MatchControlPoll {
  readonly actionDown: boolean;
  readonly playDown: boolean;
  readonly modifierDown: boolean;
}

export interface MatchContextualStep {
  /** Everything `InputSampleCapture.sample` needs except `switchPlayer` -- see [`applySwitchEdge`]'s doc for why that field is computed separately, off the discrete event stream rather than a poll. */
  readonly contextual: ContextualInputFields;
  /**
   * The jockey-release dash edge. Not part of `ContextualInputFields` --
   * `capture_frame.ts`'s own header explains `dash` needs both a
   * previous- and current-frame contextual value, so it is not expressible
   * as a single neutral-by-default parameter the way the other contextual
   * fields are. The caller ORs this into the built `InputSample`'s `edges`
   * bitmask directly, via `inputSample.packEdges`.
   */
  readonly dashEdge: boolean;
}

/**
 * One render frame's worth of contextual-input computation. Mutates
 * `latches` in place and returns this frame's contextual fields plus the
 * `dash` edge `ContextualInputFields` cannot carry.
 */
export function stepMatchControlLatches(
  latches: MatchControlLatches,
  carrying: boolean,
  poll: MatchControlPoll,
): MatchContextualStep {
  // ACTION reads as "shoot" while carrying (hold to charge, release to
  // fire); off the ball it is "jockey" while held and fires the poke (the
  // `dash` edge) on release.
  const held = carrying && poll.actionDown;
  const actionDownOffball = !carrying && poll.actionDown;
  const playHeld = carrying && poll.playDown;

  // Jockey release: ACTION was held last frame off the ball and is not now.
  const dashEdge = latches.actionHeldPrev && !actionDownOffball && !carrying;

  // MODIFIER latches across the hold so a loft always registers even when
  // the modifier lifts a frame before the fire/release edge does.
  const firing = (latches.shootHeldPrev && !held) || (latches.passHeldPrev && !playHeld);
  const lob = poll.modifierDown || (firing && latches.lobLatch);
  latches.lobLatch = held || playHeld ? latches.lobLatch || poll.modifierDown : false;

  const shoot = latches.shootHeldPrev && !held; // fire on release
  const pass = latches.passHeldPrev && !playHeld; // pass fires on release too

  latches.shootHeldPrev = held;
  latches.passHeldPrev = playHeld;
  latches.actionHeldPrev = actionDownOffball;

  return {
    contextual: {
      shoot,
      shootHeld: held,
      pass,
      passHeld: playHeld,
      jockey: actionDownOffball, // hold ACTION off the ball: slow shadow stance
      lob,
      aerialStrike: actionDownOffball,
      aerialAcrobatic: actionDownOffball && poll.modifierDown,
    },
    dashEdge,
  };
}

// A trivial pair satisfying `controller.normalize`'s "click" branch
// signature, which this module's key/gamepad/action events never take.
// Verbatim copy of `capture_frame.ts`'s own private constants of the same
// name (not exported by that module, so this is the same unavoidable,
// disclosed duplication that file's header already accepts for itself).
const NOOP_TRANSFORM: ViewportTransform = {
  baseW: 1,
  baseH: 1,
  actualW: 1,
  actualH: 1,
  scale: 1,
  offsetX: 0,
  offsetY: 0,
};
const NOOP_VIEWPORT_MAPPER: ViewportMapper = { toVirtual: () => null };

// -----------------------------------------------------------------------
// THE FIXED SIMULATION CLOCK. This used to be a from-scratch, hand-copied
// mirror of `crates/gc-sim/src/fixed_clock.rs`'s `advance` -- `gc_sim::fixed_clock`
// had no TS binding, and `SimHostPort`
// exposed only a one-fixed-tick `step`, not a batched `advance`. That was a
// determinism-relevant algorithm (tick COUNT changes what the simulation
// computes) with two implementations and no way to keep them honest against
// each other: change `MAX_TICKS_PER_UPDATE` in Rust and this copy would
// silently keep the old behavior (ARCHITECTURE.md §1.1).
//
// `SimHostPort.planTicks` (`crates/gc-wasm/src/session.rs`'s `FixedClock`,
// bound through `@gc/wasm`) removes the second implementation instead of
// pinning it: this screen now only calls `planTicks`/`step`/
// `cancelPlannedTicks` in a loop, below. There is exactly one accumulator
// algorithm, and it lives in Rust.
// -----------------------------------------------------------------------

export type MatchScreenProfile = "product" | "playtest" | "online";

// =============================================================================
// THE ROLLBACK LABORATORY -- this screen's development-only slot-mode
// option (`MatchScreenOptions.rollback_lab`), a companion state machine
// over `sim.rollback_playable_lab` (`crates/gc-sim/src/rollback_playable_lab.rs`).
// See `match_rollback_lab.spec.ts`'s header for the two blockers this
// section clears (no construction option,
// no rollback surface on the host contract) and the one it does NOT (no
// `@gc/wasm` binding for `rollback_playable_lab` itself -- `MatchDriverBridge`
// is an OMP-3 two-peer online driver, not this single-process dev harness,
// and wiring a real one is a Rust/wasm change outside this file's ownership).
//
// `RollbackHostPort` is therefore this file's OWN design, unlike
// `SimHostPort` (a fixed contract shared with `@gc/app`) -- there is no
// existing production implementation to satisfy; `match_rollback_lab.spec.ts`
// drives it with a hand-written fake, the same "TS-glue-observable analog"
// pattern `match_screen.spec.ts`'s own header already establishes for
// `SimHostPort`/`FakeSimHost`. A real implementation (over two loopback
// `@gc/wasm` `MatchDriverBridge`s, or a new direct `rollback_playable_lab`
// binding) is future work not yet attempted.
// =============================================================================

/**
 * `sim.rollback_playable_lab`'s convergence/sync status, surfaced on
 * {@link RollbackLabDebug}'s `status`. `"diverged"`/`"late_input_unrecoverable"`/
 * `"comparison_history_missing"`/`"drain_incomplete"` are real
 * `gc_sim::rollback_playable_lab::advance` outcomes (see `@gc/wasm`'s
 * `RollbackPlayableLab.advance` doc) a real `RollbackHostPort`
 * implementation can report; `"paused"` is this screen's own synthesized
 * status (`tuning.open`/`MatchScreenPorts.tuningPause`), never produced by
 * the host itself.
 */
export type RollbackLabStatus =
  | "active"
  | "settling"
  | "converged"
  | "paused"
  | "diverged"
  | "late_input_unrecoverable"
  | "unconfirmed_window_exceeded"
  | "comparison_history_missing"
  | "drain_incomplete";

/**
 * This screen's rollback-mode "is the match over" check: `status !==
 * "active" && status !== "settling"`. Derived from the host's own reported
 * status -- `rollback_playable_lab.debug_model(lab).status` -- rather than
 * a separate `RollbackHostPort` method. This is NOT the same signal as
 * `hud.finished`: a rollback source can keep reporting `hud.finished` false
 * for a snapshot-restored `state.finished` field's own long settlement
 * window, or vice versa read `hud.finished` true well before the source
 * itself goes terminal. See `updateRollback`'s doc for where each of the two
 * checks actually matters.
 */
function rollbackTerminal(status: RollbackLabStatus): boolean {
  return status !== "active" && status !== "settling";
}

/** The network-condition profile applied to the rollback lab's simulated link (`fixed_profile` in `match_rollback_lab.spec.ts`). */
export interface RollbackLabNetworkProfile {
  readonly base_delay_ticks: number;
  readonly jitter_min_ticks?: number;
  readonly jitter_max_ticks?: number;
  readonly independent_loss_rate?: number;
  readonly duplication_rate?: number;
  readonly burst_start_rate?: number;
  readonly burst_length_ticks?: number;
}

/** This screen's `rollback_lab` construction option -- see {@link MatchScreenOptions.rollback_lab}. */
export interface RollbackLabOptions {
  /** One-based canonical input slot this screen's local player drives. */
  readonly local_slot: number;
  readonly profile_name: string;
  readonly network_profile?: RollbackLabNetworkProfile;
  readonly network_seed?: number;
  readonly bot_seed?: number;
  readonly duration?: number;
  readonly settlement_ticks?: number;
  readonly max_rollback_ticks?: number;
  /** An opaque, host-specific seed snapshot (e.g. a prior `MatchSnapshot`). Never inspected here -- forwarded to {@link RollbackHostFactory} untouched. */
  readonly initial_snapshot?: unknown;
}

/** `rollback_playable_lab.current_snapshot(...)`/`reference_snapshot(...)`'s combat-companion presence -- only what construction-time validation and the "combat companion" seam need. Never the raw `MatchSnapshot` itself (opaque, host-owned). */
export interface RollbackLabSnapshotSummary {
  readonly combat?: { readonly player_ids: readonly string[] };
}

/** {@link RollbackLabDebug}'s host-reported half. `active_smoothing_count`/`correction_magnitude` are NOT here -- those are computed screen-side from `correctionSmoothing.diagnostics` over this screen's own `renderSmoothing` field: the panel reads both a driver-reported status and a render-owned smoothing diagnostic. See {@link RollbackLabDebug}. */
export interface RollbackLabHostDebug {
  readonly profile: string;
  readonly local_slot: number;
  readonly transport_tick: number;
  readonly reference_tick: number;
  readonly current_tick: number;
  readonly rollback_count: number;
  readonly network_pending: number;
  readonly status: RollbackLabStatus;
  readonly event_status: string;
  /**
   * The remaining fields mirror `gc_sim::rollback_playable_lab::debug_model`
   * one-for-one (`@gc/wasm`'s `RollbackPlayableLab.debugModelJson` doc) but
   * are optional here: a real host reports all of them, while a minimal
   * test double (e.g. `match_rollback_lab.spec.ts`'s `FakeRollbackHost`)
   * need not fabricate them.
   */
  readonly resimulated_ticks?: number;
  readonly confirmed_input_tick?: number;
  readonly confirmed_output_tick?: number;
  readonly convergence?: {
    readonly status: string;
    readonly boundary: number;
    readonly expected_hash?: string;
    readonly actual_hash?: string;
    readonly first_difference?: number;
  };
}

/** As read by a caller of {@link MatchScreen.debugRollbackDebug} -- the host's own status plus this screen's render-owned smoothing diagnostics. */
export interface RollbackLabDebug extends RollbackLabHostDebug {
  readonly active_smoothing_count: number;
  readonly correction_magnitude: number;
}

/** The rollback-mode counterpart to the base game loop's fixed-clock tick count -- the same accumulator fields `SimHostPort.planTicks` hides behind a single tick count, exposed here because `match_rollback_lab.spec.ts` asserts on `dropped_ticks`/`overloads`/`accumulator` directly. */
export interface RollbackLabClockDebug {
  readonly tick: number;
  readonly dropped_ticks: number;
  readonly overloads: number;
  readonly accumulator: number;
}

/** One roster id's displayed (rollback-client) position -- the minimal slice `correctionSmoothing`/`viewState` read. */
export interface RollbackLabPlayerPosition {
  readonly id: string;
  readonly pos: { readonly x: number; readonly y: number };
}

/** The displayed rollback client's positions this render call -- source data for {@link correctionSmoothing} and {@link viewState}, never the raw `MatchState`. */
export interface RollbackLabDisplayedPositions {
  readonly players: readonly RollbackLabPlayerPosition[];
  readonly ball: { readonly x: number; readonly y: number };
}

/** One canonical input slot's wire sample, as recorded on a `_rollback_outputs` entry. */
export interface RollbackLabInputSlot {
  readonly sample: InputSample;
}

/** One simulated tick's raw output record -- `_rollback_outputs[i]`. Unconditional: one per tick actually stepped this render call, whether or not that tick's input has been confirmed yet (confirmation is `eventDiffs`/`confirmedSteps`' job, not this record's). */
export interface RollbackLabOutput {
  readonly tick: number;
  readonly input: { readonly slots: readonly RollbackLabInputSlot[] };
}

/** One confirmation's correction source -- fed to `correctionSmoothing.correct` when a resimulated tick lands. */
export interface RollbackLabCorrectionSample {
  readonly tick: number;
  readonly source: correctionSmoothingTypes.CorrectionSmoothingSource;
}

/** One `step` call's full result: this tick's raw output plus whatever `gc_sim::rollback_events`-shaped confirmation work resolved this tick (usually nothing, or one confirmation once the network delay window has passed). */
export interface RollbackLabStepResult {
  readonly output: RollbackLabOutput;
  readonly eventDiffs: readonly RollbackEventDiff[];
  readonly confirmedSteps: readonly RollbackEventStep[];
  readonly correction?: RollbackLabCorrectionSample;
  readonly debug: RollbackLabHostDebug;
}

/**
 * The rollback laboratory's host seam -- `SimHostPort`'s rollback-mode
 * counterpart. See this section's header for why this is this file's own
 * design rather than a fixed contract, and `match_rollback_lab.spec.ts` for
 * the hand-written fake that drives it in tests.
 */
export interface RollbackHostPort {
  /** Same accumulator/catch-up/drop decision as `SimHostPort.planTicks`, over this host's own fixed clock. */
  planTicks(dt: number): number;
  /** Advance exactly one tick with this render call's locally authored sample. */
  step(sample: InputSample): RollbackLabStepResult;
  /** Mirrors `SimHostPort.cancelPlannedTicks` -- called when this render call stops short of every tick `planTicks` authorized (paused, terminal, or the match finished mid-batch). */
  cancelPlannedTicks(): void;
  /** Cheap; callable before the first `step` (construction-time validation reads it immediately). */
  debug(): RollbackLabHostDebug;
  clockDebug(): RollbackLabClockDebug;
  frame(): RenderFrame;
  roster(): RenderFrameRoster;
  tick(): number;
  displayedPositions(): RollbackLabDisplayedPositions;
  currentSnapshot(): RollbackLabSnapshotSummary;
  referenceSnapshot(): RollbackLabSnapshotSummary;
  dispose(): void;
}

/** Constructs a fresh {@link RollbackHostPort} with a `RollbackLabOptions` already baked in by the caller's closure -- the rollback-mode counterpart of {@link SimHostFactory}. `MatchScreen.restart`'s rollback branch calls this again for "R" to rebuild a fresh laboratory with the same options. */
export type RollbackHostFactory = () => RollbackHostPort;

/** One canonical simulation tick, seconds -- matches `online_match.ts`'s own `TICK_SECONDS` and every fixed-clock fake in this package's spec files. */
const ONLINE_TICK_SECONDS = 1 / 60;
const ONLINE_CLOCK_EPSILON = ONLINE_TICK_SECONDS * 1e-9;
/** A runaway-loop guard only -- see [`MatchScreen.updateOnline`]'s doc for why this is not the determinism-relevant catch-up/drop decision `SimHostPort.planTicks` exists to keep singular.
 *
 * #612's network heartbeat (`@gc/app`'s `network_heartbeat.ts`) deliberately
 * does NOT fund this cap higher, and this cap is deliberately not raised or
 * removed to "keep up" with it -- the two caps guard different things. This
 * one bounds how much LOCAL RENDERED SIMULATION one render call attempts
 * (a real cost: sim step time, animation/camera state), so after a long
 * stall the match screen catches up over several calls at up to `1/60 * 8`
 * seconds of game time per call, rendering visibly "fast-forwarded" for a
 * few frames -- not a defect, an accepted, bounded consequence of catching
 * up at all. `online_match.ts`'s OWN `OnlineMatch.update` fixed-tick
 * accumulator, immediately alongside this screen's `update` in the same
 * caller, drives the COORDINATOR's tick clock instead and is intentionally
 * UNCAPPED (see that accumulator's own call site) -- capping the
 * coordinator's funding to match this constant would silently starve it of
 * tick-time again, reintroducing the exact class of bug #612 fixes. The
 * coordinator is allowed to "jump" a whole stall's worth of ticks in one
 * synchronous burst; only the rendered picture is deliberately smoothed out
 * over several calls instead. */
const MAX_ONLINE_TICKS_PER_UPDATE = 8;

// =============================================================================
// THE ONLINE-DRIVEN MATCH SCREEN -- a third construction mode, parallel to
// the rollback laboratory above, over `online_match.ts`'s OMP-3 match
// driver instead of `sim.rollback_playable_lab`.
//
// `OnlineHostPort` is the UNION of two shapes this file already handles
// separately, not a new design of its own:
//
//   - `online_match.ts`'s private `driverSource()` return shape
//     (`needsLocalSample`/`advance`/`currentSnapshot`/`snapshot`/`terminal`/
//     `failed`/`fullTime`/`debugModel`/`controlledPlayer`) -- already
//     exactly what `OnlineMatchOptions.newMatch`'s `rollbackSource`
//     parameter carries (see that file's own doc for `driverSource`).
//   - `RollbackHostPort`'s render-facing surface (`frame`/`roster`/`tick`/
//     `dispose`, the same `Pick<SimHostPort, ...>` shape
//     `MatchScreen.activeHost()` already normalizes `host`/`rollbackHost`
//     into).
//
// Unlike `rollback_lab`'s `createRollbackHost` factory, this file does NOT
// construct an `OnlineHostPort` itself -- `OnlineMatch` (online_match.ts)
// already constructs and owns the real instance via its own `driverSource()`
// closure, so `MatchScreenOptions.online` carries the object directly
// (already built), not options to build one from.
//
// `state.controlled` for this mode is NOT `hud.controlled` (that field is
// the RAW simulation's own notion -- whichever slot local switch input last
// picked, or the sim's construction-time default -- and does not know about
// OMP-3's frozen-owned-set/keeper-exclusion switch rule at all). It is
// `onlineSource.controlledPlayer(state) ?? hud.controlled` -- see
// [`MatchScreen.onlineState`]. The `RenderFrame` handed to `RenderPort.draw`
// is corrected the same way (`hud.controlled`, `control.controlled`, and
// the per-slot `players.controlled` highlight array) -- see `drawOnline` --
// or the renderer would highlight whichever player local switching would
// have picked, not the driver's real assignment: silently passing by
// drawing the wrong thing correctly.
// =============================================================================

/**
 * The union of `online_match.ts`'s private `driverSource()` return shape and
 * `RollbackHostPort`'s render-facing surface -- see this section's header.
 * `advance`'s `sample` is typed `InputSample` here (this file's own real
 * caller); `online_match.ts`'s `driverSource().advance` accepts `unknown`
 * (a strict superset), so it satisfies this narrower parameter type without
 * change.
 */
export interface OnlineHostPort extends Pick<SimHostPort, "frame" | "roster" | "tick" | "dispose"> {
  /** Whether this driver still wants a local sample this tick -- `status(driver) === "active"`. Gates [`MatchScreen`]'s online-mode advance loop; there is no `planTicks`-style tick-count decision here (see this section's header) because how many ticks a render call ATTEMPTS is not a determinism-relevant decision the way `gc_sim::fixed_clock`'s local-physics catch-up/drop policy is: the driver's own confirmation/rollback protocol (Rust-owned, `gc_netcode::match_driver`) is what actually decides whether an attempted tick is accepted, and `needsLocalSample()` is exactly that decision surfaced here. */
  needsLocalSample(): boolean;
  /** One fixed-tick driver step. */
  advance(tick: number, sample: InputSample): unknown;
  currentSnapshot(): unknown;
  snapshot(boundary: number): unknown;
  /** Whether the driver has left `"active"` status. */
  terminal(): boolean;
  /** Whether the driver's terminal status is a genuine failure (neither `"active"` nor `"completed"`). */
  failed(): boolean;
  /** The settled boundary, not merely "the driver stopped" -- see `online_match.ts`'s `driverSource().fullTime` doc. */
  fullTime(): boolean;
  debugModel(): unknown;
  /**
   * The real, driver-assigned controlled player index (0-based, into
   * {@link OnlineMatchState.players}), or `undefined` if this peer does not
   * currently own a live slot. See this section's header.
   */
  controlledPlayer(state: OnlineMatchState): number | undefined;
}

export interface MatchScreenOptions {
  /** Defaults to `"playtest"` when omitted. */
  readonly profile?: MatchScreenProfile;
  /**
   * For a rollback-lab construction, this gates the construction-time
   * validation against the supplied `initial_snapshot`'s `CombatMatchState`
   * companion (per `rollback_playable_lab`'s own invariant) -- see
   * `validateRollbackCombatCompanion`.
   *
   * For the BASE, non-rollback game loop, this is a construction-time-only
   * option, deliberately NOT part of `SimHostPort` -- see that interface's
   * own doc on why it is a fixed contract shared with `@gc/app`'s
   * `sim_host.ts`, not this file's to widen: `crates/gc-wasm/src/session.rs`'s
   * `Session::new` accepts a `combat_enabled` parameter and threads it
   * through every `Session::step` call -- but that choice is baked into the
   * `SimHostFactory` closure at construction time (the same way team ids,
   * seed, and formation already are; see `SimHostFactory`'s own doc), not
   * passed through this option. This option is therefore purely a record of
   * the CALLER's intent, exposed for tests/callers via
   * [`MatchScreen.debugCombatEnabled`] -- there is still no getter on
   * `Session`/`SimHostPort` a base-mode host could use to confirm the
   * closure actually honored it, nor any wasm-bound way to read per-tick
   * combat events/state once it has (see `match_screen.spec.ts`'s combat
   * describe block for exactly what that leaves provable).
   */
  readonly combat_enabled?: boolean;
  /** The rollback-lab construction option. See this section's header. */
  readonly rollback_lab?: RollbackLabOptions;
  /**
   * The already-constructed OMP-3 online host/source -- see "THE
   * ONLINE-DRIVEN MATCH SCREEN" section header above. Mutually exclusive
   * with {@link rollback_lab}; forces this screen's `profile` to `"online"`
   * regardless of what {@link profile} was passed (see the constructor),
   * so the rematch-key handling `playtest` mode relies on
   * (`handleRematchEvent`) can never misfire mid-online-match on an
   * un-set/default profile.
   */
  readonly online?: OnlineHostPort;
}

export interface MatchScreenPorts {
  readonly createHost: SimHostFactory;
  readonly renderer: RenderPort;
  readonly keyboard: KeyboardState;
  readonly gamepad?: GamepadState;
  /** Required when {@link MatchScreenOptions.rollback_lab} is set; unused otherwise. */
  readonly createRollbackHost?: RollbackHostFactory;
  /** The rollback-consumption seam's ports (see this file's header) -- required alongside {@link createRollbackHost}, since the rollback laboratory is the first caller that actually wires `consumeRollbackPresentation` into the game loop. */
  readonly effects?: EffectsPort;
  readonly audio?: AudioPort;
  readonly replay?: ReplayPort;
  /** `@gc/ui`'s `tuning_panel.ts`'s `.open` flag, injected -- pausing the panel pauses the rollback laboratory's clock. Omit to never pause. */
  readonly tuningPause?: { readonly open: boolean };
  /**
   * Raw `MatchState`, read live off whatever is actually simulating this
   * render call -- `SimSession.matchStateJson()`/
   * `RollbackPlayableLab.currentMatchStateJson()` in the real build
   * (`@gc/wasm`), parsed by the caller into `@gc/render`'s `replay.ts`
   * `MatchState` shape. Enables the BASE (non-rollback) game loop's
   * goal-replay recording/detection (`ReplayPort.record`/`start`, this
   * file's "THE GAME LOOP" section) and this screen's own base-mode
   * render-smoothing/view-state wiring. Optional: omit to leave both
   * unwired, matching this class's behavior before this capability
   * existed. Not read in rollback mode -- `RollbackHostPort.displayedPositions`
   * already covers that mode's correction/view-state needs, and a real
   * rollback host's own raw-state getters
   * (`currentMatchStateJson`/`referenceMatchStateJson`) are that mode's
   * analog of this port, not a reason to also read this one.
   */
  readonly matchState?: () => replayTypes.MatchState;
}

/**
 * The playable match screen: gathers input, drives the fixed simulation
 * clock through an injected {@link SimHostPort}, and hands frames to an
 * injected {@link RenderPort}. Scoped to what a
 * `step`/`frame`/`roster`/`tick`/`dispose` host can support this milestone
 * -- see this file's header for what is deliberately left out (combat,
 * goal-replay slow-mo, the rollback laboratory) and why.
 *
 * KNOWN SIMPLIFICATION -- input buffering across ticks. `@gc/app`'s
 * `match_input_adapter.ts` buffers one-shot edges across a zero-tick render
 * update and holds continuous state across a catch-up render update that
 * runs more than one tick. That adapter operates on the LEGACY `MatchInput`
 * shape, not the new wire-format `InputSample` `SimHostPort.step` takes,
 * and no equivalent exists for `InputSample` yet (`@gc/screens` cannot
 * depend on `@gc/app` to reach the legacy one regardless). This class
 * instead samples one `InputSample` per render call and feeds that SAME
 * sample to every tick a catch-up batch runs, and drops it entirely on a
 * zero-tick render call. At a healthy frame rate this is unobservable (one
 * tick per render call, always). Under sustained overload
 * (`MAX_TICKS_PER_UPDATE` catch-up, or a render call fast enough to produce
 * zero ticks) a one-shot edge can fire more than once, or be dropped,
 * instead of firing exactly once on its owning tick -- a real, disclosed
 * behavioral gap, not silently patched over.
 */
export class MatchScreen {
  private readonly ports: MatchScreenPorts;
  private readonly profile: MatchScreenProfile;
  private readonly combatEnabled: boolean;
  /** Set exactly when {@link MatchScreenOptions.rollback_lab} was supplied; mutually exclusive with `host`/`onlineHost`. */
  private host: SimHostPort | undefined;
  private rollbackHost: RollbackHostPort | undefined;
  /** Set exactly when {@link MatchScreenOptions.online} was supplied; mutually exclusive with `host`/`rollbackHost` -- see "THE ONLINE-DRIVEN MATCH SCREEN" section. */
  private onlineHost: OnlineHostPort | undefined;
  private onlineAccumulator = 0;
  private onlineTickCount = 0;
  private rollbackConsumerPorts: MatchRollbackConsumerPorts | undefined;
  private rollbackConsumer: MatchRollbackConsumerState = newMatchRollbackConsumerState();
  private rollbackOutputs: readonly RollbackLabOutput[] = [];
  private rollbackEventDiffs: readonly RollbackEventDiff[] = [];
  private rollbackConfirmedSteps: readonly RollbackEventStep[] = [];
  private rollbackCorrections: readonly RollbackLabCorrectionSample[] = [];
  private rollbackFrameEvents: MatchEvent[] = [];
  /**
   * This render call's BASE-mode discrete match events, translated from
   * `RenderFrame.events` and accumulated across every tick `update`
   * actually stepped -- source data for [`MatchScreenAsRealMatchScreen.
   * frameEvents`]. Cleared at the top of every BASE-mode `update()` call
   * (mirrors how `rollbackFrameEvents` is cleared per `updateRollback`
   * call); distinct from `rollbackFrameEvents` (a `@gc/presentation`
   * `MatchEvent[]`, fed by the rollback-consumption seam, unrelated to
   * observation stats). Not read outside BASE mode.
   */
  private observedFrameEvents: RealMatchEvent[] = [];
  private renderSmoothing: correctionSmoothingTypes.CorrectionSmoothingState | undefined;
  /** `_replay_state` -- the currently displayed goal-replay frame (base mode's legacy replay only; see `debugReplayState`'s doc). */
  private replayState: replayTypes.ReplayFrame | undefined;
  /** `_last_score`/`_last_home` -- base-mode goal-edge bookkeeping across render calls. Unused in rollback mode (that mode detects a goal off a CONFIRMED lifecycle record instead, see `consumeConfirmedLifecycle`). */
  private lastScore = 0;
  private lastHome = 0;
  private readonly latches: MatchControlLatches = newMatchControlLatches();
  private switchPending = false;
  private capture: captureFrame.InputSampleCapture;
  private pendingKeyEvents: ControllerInputEvent[] = [];
  private pendingGamepadEvents: ControllerInputEvent[] = [];
  private lastStepSample: InputSample | undefined;

  constructor(ports: MatchScreenPorts, options: MatchScreenOptions = {}) {
    this.ports = ports;
    // `online` forces the `"online"` profile regardless of what `profile`
    // was passed -- see `MatchScreenOptions.online`'s own doc.
    this.profile = options.online !== undefined ? "online" : (options.profile ?? "playtest");
    this.combatEnabled = options.combat_enabled ?? false;
    if (options.rollback_lab !== undefined && options.online !== undefined) {
      throw new Error("rollback_lab and online are mutually exclusive MatchScreenOptions");
    }
    if (options.online !== undefined) {
      this.onlineHost = options.online;
    } else if (options.rollback_lab !== undefined) {
      if (this.profile === "product") {
        throw new Error(
          "rollback_lab is an explicit development-only option; product-profile matches must not construct one",
        );
      }
      if (ports.createRollbackHost === undefined) {
        throw new Error("rollback_lab option requires MatchScreenPorts.createRollbackHost");
      }
      if (ports.effects === undefined || ports.audio === undefined || ports.replay === undefined) {
        throw new Error(
          "rollback_lab option requires effects/audio/replay ports for its rollback-consumption seam",
        );
      }
      this.rollbackHost = ports.createRollbackHost();
      this.rollbackConsumerPorts = {
        effects: ports.effects,
        audio: ports.audio,
        replay: ports.replay,
      };
      this.validateRollbackCombatCompanion();
    } else {
      this.host = ports.createHost();
    }
    this.capture = this.newCapture();
    // #447: build this match's character geometry HERE, not on frame 1.
    //
    // A roster crosses the wire ONCE PER MATCH and both sim hosts memoise the
    // decode, so the complete set of distinct `(theme, figure, loadout)`
    // variants is knowable the instant a host exists. Left to the renderer's
    // own laziness it is discovered inside `pitch.draw`'s depth-sorted loop
    // instead, which calls `characterMesh` -- and therefore `build()` --
    // synchronously for every rostered player on every frame: the first frame
    // of a match paid for the whole starting roster's geometry back to back,
    // measured at ~58 ms across the two fixture teams' nine variants, on the
    // very frame the pitch appears.
    //
    // Doing it at construction puts that cost in the screen transition, where
    // a pause is expected and invisible. Deliberately NOT wrapped in a
    // try/catch: unknown content should fail here, at match setup, rather
    // than mid-match on the frame that player first appears.
    this.prewarmCharacters();
  }

  /**
   * Build the character geometry this match's roster will ask for, before any
   * frame is drawn. See the call in the constructor for why this is not left
   * to the renderer's own laziness.
   *
   * A no-op for a host whose roster carries no presentation ids -- the fake
   * hosts throughout this package's own suites return `{}` -- so it costs
   * nothing outside a real match.
   */
  private prewarmCharacters(): void {
    // ALL THREE HOST KINDS, with no exclusion to justify. `SimHostPort`,
    // `OnlineHostPort` and `RollbackHostPort` each declare `roster()`, so
    // this needs no special case -- and an earlier revision of this method
    // asserted, wrongly, that `RollbackHostPort` had no such method and
    // skipped the laboratory on that basis. It declares `roster()` beside its
    // `frame()`, exactly as the other two ports do.
    //
    // The laboratory's roster IS degenerate today: every `RollbackHostPort`
    // implementation in the tree -- this package's own fakes and `@gc/app`'s
    // `RealRollbackHost` over the real wasm `RollbackPlayableLab` -- returns
    // `{}`, which `prewarmCharacters` treats as "nothing to warm". So this
    // call does nothing there right now. It is made anyway rather than
    // guarded: an empty roster is already a documented no-op, and the day the
    // laboratory grows a real one it gets warmed for free instead of needing
    // somebody to remember an exclusion.
    const host: Pick<SimHostPort, "roster"> | undefined =
      this.onlineHost ?? this.host ?? this.rollbackHost;
    if (host === undefined) {
      throw new Error(
        "match screen: constructed with no host to pre-warm (unreachable -- construction builds exactly one)",
      );
    }
    player3dPrewarmCharacters(host.roster());
  }

  // `rollback_playable_lab`'s own combat-companion invariant, enforced at
  // construction: a combat-bearing snapshot requires the explicit opt-in,
  // and the opt-in requires a combat-bearing snapshot -- see
  // `match_rollback_lab.spec.ts`'s "requires rollback snapshot combat
  // presence to match the explicit opt-in".
  private validateRollbackCombatCompanion(): void {
    const hasCombat = this.rollbackHost?.currentSnapshot().combat !== undefined;
    if (hasCombat && !this.combatEnabled) {
      throw new Error("combat-bearing rollback snapshots require combat_enabled = true");
    }
    if (this.combatEnabled && !hasCombat) {
      throw new Error("combat-enabled matches require a CombatMatchState companion");
    }
  }

  private newCapture(): captureFrame.InputSampleCapture {
    return new captureFrame.InputSampleCapture(
      this.ports.keyboard,
      this.ports.gamepad,
      () => {
        const drained = this.pendingKeyEvents;
        this.pendingKeyEvents = [];
        return drained;
      },
      () => {
        const drained = this.pendingGamepadEvents;
        this.pendingGamepadEvents = [];
        return drained;
      },
    );
  }

  /** Either active host, whichever this screen was constructed with -- mutually exclusive, see `host`/`rollbackHost`/`onlineHost`'s own docs. */
  private activeHost(): Pick<SimHostPort, "frame" | "roster" | "tick" | "dispose"> {
    const host = this.rollbackHost ?? this.onlineHost ?? this.host;
    if (host === undefined) {
      throw new Error(
        "match screen: no active host (unreachable -- constructor always builds one)",
      );
    }
    return host;
  }

  /** Read live off the host every call (never cached), so it always reflects current state rather than a stale snapshot. */
  get finished(): boolean {
    return this.activeHost().frame().hud.finished;
  }

  /** The current frame's HUD slice, read live off the host -- used by [`MatchScreenAsRealMatchScreen`]'s `state` (score/clock) and by this class's own `finished`/`carrying`. */
  get hud(): RenderFrameHud {
    return this.activeHost().frame().hud;
  }

  /**
   * `real_match.ts`'s `RealMatchState.roster`/`.owner`, read live off the
   * active host -- used by [`MatchScreenAsRealMatchScreen`]'s `state` for
   * `@gc/app`'s `match_observer.ts`'s shot/save/possession/pass-completion
   * attribution. Built from [`SimHostPort.roster`] (`ids`/`teams`/
   * `is_keeper`, match-constant) and `frame().possession.owner` (this
   * frame's one-based carrier slot), the same "roster + possession" pair
   * [`onlineState`] already reads for its own `players[]`/`owner`. `roster`
   * is `undefined` when the active host's `roster()` does not carry
   * `ids`/`teams`/`is_keeper` together (every field or none -- a partial
   * roster is not attributable) -- e.g. `match_screen.spec.ts`'s
   * `FakeSimHost.roster()`, which returns `{}`.
   */
  get matchObservationState(): {
    readonly roster?: readonly RealMatchRosterEntry[];
    readonly owner?: number;
  } {
    const host = this.activeHost();
    const roster = host.roster();
    const ids = roster.ids;
    const teams = roster.teams;
    const isKeeper = roster.is_keeper;
    const builtRoster =
      ids !== undefined && teams !== undefined && isKeeper !== undefined
        ? ids.map((id, index) => ({
            id,
            team: teams[index] ?? "home",
            is_keeper: isKeeper[index] ?? false,
          }))
        : undefined;
    const ownerSlot = host.frame().possession.owner;
    return {
      ...(builtRoster !== undefined ? { roster: builtRoster } : {}),
      ...(ownerSlot !== undefined ? { owner: ownerSlot - 1 } : {}),
    };
  }

  /**
   * This render call's BASE-mode discrete match events -- see
   * `observedFrameEvents`'s own doc. Used by
   * [`MatchScreenAsRealMatchScreen.frameEvents`]; always empty outside BASE
   * mode (rollback/online drive `@gc/app`'s `match_observer.ts` through a different
   * seam -- see this getter's callers).
   */
  get matchObservationEvents(): readonly RealMatchEvent[] {
    return this.observedFrameEvents;
  }

  /** This screen's tick count -- delegated straight to the host, which is this screen's only tick authority. */
  get tick(): number {
    return this.activeHost().tick();
  }

  /** Whether this screen was constructed with an {@link MatchScreenOptions.online} host. Exposed for tests, mirroring `debugRollbackActive`. */
  get debugOnlineActive(): boolean {
    return this.onlineHost !== undefined;
  }

  /**
   * The narrow `players`/`control`/`roster` read [`OnlineHostPort.frame`]/
   * `.roster()` must carry in ONLINE mode -- see [`OnlineRenderFramePlayers`]/
   * [`OnlineRenderFrameControl`]'s own docs for why this is a documented,
   * narrower read rather than a widening of the general opaque
   * `RenderFrame`/`RenderFrameRoster` contract. `assert`s (AGENTS.md §7):
   * an `OnlineHostPort` whose `frame()`/`roster()` do not carry them is a
   * construction bug, not a recoverable, caller-facing failure.
   */
  private onlineFrame(): {
    readonly frame: RenderFrame;
    readonly players: OnlineRenderFramePlayers;
    readonly control: OnlineRenderFrameControl;
    readonly roster: RenderFrameRoster;
  } {
    const onlineHost = this.onlineHost;
    if (onlineHost === undefined) {
      throw new Error("match screen: onlineFrame is only valid in online mode");
    }
    const frame = onlineHost.frame();
    const players = frame.players;
    const control = frame.control;
    if (players === undefined || control === undefined) {
      throw new Error(
        "match screen: an online host's frame() must carry players/control -- see OnlineHostPort's doc",
      );
    }
    return { frame, players, control, roster: onlineHost.roster() };
  }

  /**
   * ONLINE mode's `RealMatchScreenPort<OnlineMatchState, ...>.state` source
   * -- see "THE ONLINE-DRIVEN MATCH SCREEN" section's header for why
   * `controlled` is NOT `hud.controlled`. `players[]` is built from the
   * host's `roster().ids`/`.teams` (identity) and `frame().players`
   * (live `x`/`y`/`facing_x`/`facing_y`), the same wire fields
   * `@gc/render`'s `pitch.ts` itself reads to draw them.
   *
   * `facing` HERE IS THE DRAWN FACING, NOT THE SIMULATION'S AIM (#449).
   * `gc-render`'s `frame::drawn_facing` replaces `facing_x`/`facing_y` with
   * the goal-line normal for a keeper inside a dive, get-up or tip window, so
   * what this builds is post-override. That matters because `online_match.ts`
   * feeds this `players[]` into `@gc/presentation`'s `combat.model`, which
   * stores `player.facing` as `CombatPlayerPresentation.direction` -- a
   * telegraph direction.
   *
   * It is inert today, and asserted so rather than assumed: `gc-sim`'s
   * `combat_snapshot.rs` refuses a keeper a combat loadout ("keeper cannot
   * have a combat loadout") and its `validate_player` refuses a family
   * without a loadout ("family requires a loadout"), so a keeper's
   * `family_id` is always `None`, `telegraphKind` returns `undefined`, and
   * nothing is ever drawn from a keeper's `direction`. Give keepers a combat
   * loadout and that chain breaks: re-audit this line, `combat.model`'s
   * `direction`, and `frame::drawn_facing`'s consumer list together.
   */
  get onlineState(): OnlineMatchState {
    const onlineHost = this.onlineHost;
    if (onlineHost === undefined) {
      throw new Error("match screen: onlineState is only valid in online mode");
    }
    const { frame, players, roster } = this.onlineFrame();
    const ids = roster.ids ?? [];
    const teams = roster.teams ?? [];
    const builtPlayers = ids.map((id, index) => ({
      id,
      team: teams[index] ?? "home",
      pos: new Vec2(players.x[index] ?? 0, players.y[index] ?? 0),
      facing: new Vec2(players.facing_x[index] ?? 0, players.facing_y[index] ?? 0),
    }));
    // `hud.controlled` is one-based (ARCHITECTURE.md §3 rule 3's wire-identity
    // exception); `state.controlled` is a 0-based array index into
    // `players[]`, matching every other index this file uses.
    const fallbackControlled = (frame.hud.controlled ?? 1) - 1;
    const ownerSlot = frame.possession.owner;
    const withFallback: OnlineMatchState = {
      time_left: frame.hud.time_left,
      score: { home: frame.hud.home_score, away: frame.hud.away_score },
      controlled: fallbackControlled,
      ...(ownerSlot !== undefined ? { owner: ownerSlot - 1 } : {}),
      players: builtPlayers,
    };
    const controlled = onlineHost.controlledPlayer(withFallback) ?? fallbackControlled;
    return { ...withFallback, controlled };
  }

  /** The `InputSample` this screen last sent to `SimHostPort.step`, if any. Exposed for tests to inspect internal state directly. */
  get debugLastSample(): InputSample | undefined {
    return this.lastStepSample;
  }

  /** Whether a switch is currently buffered awaiting the next `update`. Exposed for tests -- see [`debugLastSample`]. */
  get debugSwitchPending(): boolean {
    return this.switchPending;
  }

  /** Whether this screen was constructed with a `rollback_lab` option. */
  get debugRollbackActive(): boolean {
    return this.rollbackHost !== undefined;
  }

  /**
   * This screen's own record of the caller's combat opt-in, fixed at
   * construction and never recomputed (see [`restart`], which always
   * rebuilds the rollback combat state fresh but consistent with this
   * option). Exposed for tests, and for [`MatchScreenAsRealMatchScreen`]'s
   * own `debugCombatEnabled`. See {@link MatchScreenOptions.combat_enabled}'s
   * doc for exactly what this option does and does not prove in the BASE
   * (non-rollback) game loop this milestone.
   */
  get debugCombatEnabled(): boolean {
    return this.combatEnabled;
  }

  /** `undefined` outside rollback mode. See {@link RollbackLabDebug}'s doc for the host/screen split. */
  debugRollbackDebug(): RollbackLabDebug | undefined {
    if (this.rollbackHost === undefined) {
      return undefined;
    }
    const raw = this.rollbackHost.debug();
    const diagnostics =
      this.renderSmoothing !== undefined
        ? correctionSmoothing.diagnostics(this.renderSmoothing)
        : { active_count: 0, maximum_magnitude: 0 };
    return {
      ...raw,
      active_smoothing_count: diagnostics.active_count,
      correction_magnitude: diagnostics.maximum_magnitude,
    };
  }

  /** This screen's rollback-mode fixed-clock debug info -- `undefined` outside rollback mode. */
  debugRollbackClock(): RollbackLabClockDebug | undefined {
    return this.rollbackHost?.clockDebug();
  }

  /** This render call's raw per-tick output records, one per tick actually stepped. Cleared at the top of every `update()` call in rollback mode. */
  get debugRollbackOutputs(): readonly RollbackLabOutput[] {
    return this.rollbackOutputs;
  }

  /** This render call's confirmed rollback event diffs. */
  get debugRollbackEventDiffs(): readonly RollbackEventDiff[] {
    return this.rollbackEventDiffs;
  }

  /** This render call's confirmed rollback steps. */
  get debugRollbackConfirmedSteps(): readonly RollbackEventStep[] {
    return this.rollbackConfirmedSteps;
  }

  /** This render call's correction sources, retained across every simulated tick (not just the last). */
  get debugRollbackCorrections(): readonly RollbackLabCorrectionSample[] {
    return this.rollbackCorrections;
  }

  /** Always empty in rollback mode (the legacy speculative frame-event consumer has nothing to read; see `updateRollback`'s doc). */
  get debugRollbackFrameEvents(): readonly MatchEvent[] {
    return this.rollbackFrameEvents;
  }

  /** This rollback host's current combat-companion snapshot presence -- `undefined` outside rollback mode. */
  debugRollbackCurrentSnapshot(): RollbackLabSnapshotSummary | undefined {
    return this.rollbackHost?.currentSnapshot();
  }

  /** This rollback host's reference combat-companion snapshot presence -- `undefined` outside rollback mode. */
  debugRollbackReferenceSnapshot(): RollbackLabSnapshotSummary | undefined {
    return this.rollbackHost?.referenceSnapshot();
  }

  /** The screen-owned rollback-consumption ledger (scoring-team/kickoff-banner tracking, ...) -- `undefined` outside rollback mode. Exposed for tests to inspect internal state directly. */
  debugRollbackConsumerState(): MatchRollbackConsumerState | undefined {
    return this.rollbackHost !== undefined ? this.rollbackConsumer : undefined;
  }

  /**
   * The frame currently displayed by the BASE (non-rollback) goal replay,
   * or `undefined` outside an active sequence. Exposed for tests to inspect
   * internal state directly. Rollback-mode confirmed replay shares the same
   * {@link MatchScreenPorts.replay} port but this screen does not itself
   * track a displayed frame for it this milestone -- see
   * `consumeConfirmedLifecycle`'s doc.
   */
  get debugReplayState(): replayTypes.ReplayFrame | undefined {
    return this.replayState;
  }

  /** Whether the injected {@link ReplayPort} currently reports an active goal-replay sequence. `false` whenever no `ReplayPort` was supplied. */
  get replayActive(): boolean {
    return this.ports.replay?.active() === true;
  }

  /** Render-only correction-smoothing diagnostics for whichever mode this screen is in (base or rollback). `undefined` until this screen has processed at least one correction/authoritative source. Exposed for tests. */
  get debugRenderSmoothingDiagnostics():
    correctionSmoothingTypes.CorrectionSmoothingDiagnostics | undefined {
    return this.renderSmoothing !== undefined
      ? correctionSmoothing.diagnostics(this.renderSmoothing)
      : undefined;
  }

  /**
   * Seeds a synthetic previous-frame render correction, for tests only, by
   * writing directly to this screen's internal render-smoothing state. A
   * no-op if this screen currently has no correction source (base mode
   * without {@link MatchScreenPorts.matchState}, or rollback mode before
   * the first `step`).
   */
  debugSeedRenderCorrection(previous: correctionSmoothingTypes.CorrectionSmoothingSource): void {
    const current = this.correctionSource();
    if (current === undefined) {
      return;
    }
    this.renderSmoothing = correctionSmoothing.correct(correctionSmoothing.new(previous), current);
  }

  /** The authoritative positions this screen currently corrects/smooths render state against -- the rollback client's displayed positions in rollback mode, or {@link MatchScreenPorts.matchState} in base mode. `undefined` when neither is available. */
  private correctionSource(): correctionSmoothingTypes.CorrectionSmoothingSource | undefined {
    if (this.rollbackHost !== undefined) {
      return this.rollbackHost.displayedPositions();
    }
    const state = this.ports.matchState?.();
    if (state === undefined) {
      return undefined;
    }
    return { players: state.players.map((p) => ({ id: p.id, pos: p.pos })), ball: state.ball };
  }

  /**
   * Whether the controlled player currently owns the ball. ONLINE mode
   * cannot read this off the raw host's `hud.controlled_owns_ball` the way
   * base/rollback do -- that flag is computed against the RAW sim's own
   * `controlled` slot, not the driver-corrected one this screen actually
   * reports (see [`onlineState`]) -- so it recomputes the same comparison
   * against the corrected `owner`/`controlled` pair instead.
   */
  private carrying(): boolean {
    if (this.onlineHost !== undefined) {
      const state = this.onlineState;
      return state.owner === state.controlled;
    }
    return this.activeHost().frame().hud.controlled_owns_ball;
  }

  /** Dispose the current host and build a fresh one from the same factory, resetting every frame-to-frame latch and (in rollback mode) every rollback/presentation-owned field. Never reached in ONLINE mode -- `handleRematchEvent` only fires for `profile === "playtest"`, and construction forces `profile = "online"` whenever {@link MatchScreenOptions.online} is set (see that option's doc). */
  private restart(): void {
    if (this.onlineHost !== undefined) {
      throw new Error(
        "match screen: restart() is not supported in online mode (unreachable -- see this method's doc)",
      );
    }
    if (this.rollbackHost !== undefined) {
      this.rollbackHost.dispose();
      this.rollbackHost = this.ports.createRollbackHost!();
      this.rollbackOutputs = [];
      this.rollbackEventDiffs = [];
      this.rollbackConfirmedSteps = [];
      this.rollbackCorrections = [];
      this.rollbackFrameEvents = [];
      this.rollbackConsumer = newMatchRollbackConsumerState();
      this.validateRollbackCombatCompanion();
    } else {
      this.host!.dispose();
      this.host = this.ports.createHost();
      this.lastScore = 0;
      this.lastHome = 0;
    }
    // A restart builds a NEW host and therefore a new roster decode, so the
    // pre-warm runs again (#447) -- outside the branch, so the rollback
    // laboratory is covered on the same terms as everything else (see
    // `prewarmCharacters`). Idempotent when the roster is unchanged: every
    // variant is already cached and this builds nothing. The case that
    // matters is when it is NOT unchanged -- a rematch with different teams
    // would otherwise rediscover its geometry inside the first drawn frame,
    // exactly as a cold start used to.
    this.prewarmCharacters();
    this.replayState = undefined;
    this.renderSmoothing = undefined;
    viewState.reset();
    cameraFollow.reset();
    releaseFollow.reset();
    dispossessionFlinch.reset();
    player3dResetAnimation();
    this.ports.replay?.reset?.();
    this.latches.shootHeldPrev = false;
    this.latches.passHeldPrev = false;
    this.latches.actionHeldPrev = false;
    this.latches.lobLatch = false;
    this.switchPending = false;
    // No accumulator to reset here -- the fixed clock (and its
    // carried-over render-time debt) lives entirely in the new host's own
    // `SimHostPort.planTicks`/`cancelPlannedTicks`, constructed fresh above.
    this.pendingKeyEvents = [];
    this.pendingGamepadEvents = [];
    this.lastStepSample = undefined;
    this.capture = this.newCapture();
  }

  /**
   * Discrete key/gamepad/action events -- the rematch keys
   * once the match is over, and (mid-match) the `pass_switch` edge plus
   * whatever `@gc/input`'s `InputSampleCapture` needs queued for its own
   * `dodge` detection.
   */
  event(evt: ControllerInputEvent): void {
    // A confirmed goal replay (base or rollback) remains skippable
    // regardless of match/finished state -- this check runs before
    // anything else in this handler, including the finished/rematch check
    // below.
    if (this.ports.replay?.active() === true) {
      if (this.isReplaySkipEvent(evt)) {
        this.finishReplay();
      }
      return;
    }
    if (this.finished) {
      this.handleRematchEvent(evt);
      return;
    }
    if (evt.kind === "gamepad" || (evt.kind === "action" && evt.source === "gamepad")) {
      this.pendingGamepadEvents.push(evt);
    } else if (evt.kind === "key" || evt.kind === "action") {
      this.pendingKeyEvents.push(evt);
    }
    this.applySwitchEdge(evt);
  }

  // The replay-skip event set: ACTION-kind allows
  // `confirm`/`pass_switch`; KEY-kind allows `space`/`return`/`k`.
  private isReplaySkipEvent(evt: ControllerInputEvent): boolean {
    if (evt.kind === "action") {
      return evt.action === "confirm" || evt.action === "pass_switch";
    }
    if (evt.kind === "key") {
      return evt.key === "space" || evt.key === "return" || evt.key === "k";
    }
    return false;
  }

  // Stop the sequence, clear the displayed replay
  // frame, and settle render-owned smoothing/view state back onto whatever
  // this screen is currently authoritative for.
  private finishReplay(): void {
    this.ports.replay?.stop?.();
    this.replayState = undefined;
    const source = this.correctionSource();
    // An unconditional `viewState.reset()` -- discards the replay's motion
    // before reseeding baselines below, so the render-live players'
    // gait/lean read back at rest rather than carrying over the replay's
    // last pose.
    viewState.reset();
    cameraFollow.reset();
    releaseFollow.reset();
    dispossessionFlinch.reset();
    player3dResetAnimation();
    if (source === undefined) {
      return;
    }
    this.renderSmoothing =
      this.renderSmoothing !== undefined
        ? correctionSmoothing.clear(this.renderSmoothing, source)
        : correctionSmoothing.new(source);
    viewState.update(source.players, 0);
  }

  // The rematch-confirmation handler, active only for `profile ===
  // "playtest"`. ACTION-kind: `evt.action === "confirm"` restarts with no
  // `pressed` check -- a deliberate quirk, not a bug to "fix". KEY-kind:
  // `evt.key === "r"` or `bindings.controlForKey(evt.key) === "confirm"`.
  // `controlForKey` resolves "space" to the `action` control (id
  // "action"), NOT "confirm" -- only literal `return`/`kpenter` (the
  // `confirm` control's own keys) satisfy `confirm` on this path, even
  // though `action`'s own ActionName is also "confirm".
  private handleRematchEvent(evt: ControllerInputEvent): void {
    if (this.profile !== "playtest") {
      return;
    }
    if (evt.kind === "action") {
      if (evt.action === "confirm") {
        this.restart();
      }
      return;
    }
    if (evt.kind !== "key") {
      return;
    }
    if (evt.key === "r" || bindings.controlForKey(evt.key) === "confirm") {
      this.restart();
    }
  }

  // The `pass_switch` rule: off the ball, PLAY's press buffers a switch.
  // Read off the discrete event stream via `controller.normalize` -- the
  // same primitive `capture_frame.ts`'s own `consumeDodgeEdge` uses for
  // `juke` -- rather than a per-frame poll, because a switch should latch
  // on the press EDGE, not on "was PLAY held this frame".
  private applySwitchEdge(evt: ControllerInputEvent): void {
    const normalized = controller.normalize(evt, NOOP_TRANSFORM, NOOP_VIEWPORT_MAPPER);
    if (
      normalized?.kind === "action" &&
      normalized.action === "pass_switch" &&
      normalized.pressed === true &&
      !this.carrying()
    ) {
      this.switchPending = true;
    }
  }

  /**
   * Samples this frame's input once and drives the
   * fixed clock (`SimHostPort.planTicks`/`step`/`cancelPlannedTicks` -- see
   * this section's header; the accumulator algorithm itself lives only in
   * Rust now). A no-op once the match is finished, minus the
   * rollback-replay/onboarding bookkeeping this milestone does not
   * implement.
   *
   * Drawing is NOT done here -- see [`MatchScreen.draw`]: this class follows
   * the model/view seam every other screen in this codebase uses
   * (AGENTS.md §9), `update(dt)` steps the simulation and `draw()` is the
   * only impure rendering call, called separately by whoever owns the
   * render loop.
   */
  update(dt: number): void {
    if (this.onlineHost !== undefined) {
      this.updateOnline(dt);
      return;
    }
    if (this.rollbackHost !== undefined) {
      this.updateRollback(dt);
      return;
    }
    // Slow-motion goal replay: the sim freezes while the buffer plays back.
    // Checked before `finished` -- a replay may still be finishing up on
    // the same render call the match itself reads as over.
    if (this.ports.replay?.step !== undefined && this.ports.replay.active()) {
      this.updateLegacyReplay(dt);
      return;
    }
    if (this.finished) {
      return;
    }
    const host = this.host!;
    const scoreBefore = host.frame().hud.home_score + host.frame().hud.away_score;
    // Cleared here, not at the top of `update()`, so a `finished`/legacy-
    // replay early return above leaves the previous batch's events intact
    // for a caller that has not yet read them -- matches how `scoreBefore`
    // itself is only computed once this render call is known to actually
    // simulate.
    this.observedFrameEvents = [];
    const rosterIds = host.roster().ids;
    // Plan BEFORE sampling, and touch neither `this.latches` nor
    // `this.switchPending`/`capture` on a zero-tick render call -- the same
    // rule `updateRollback`/`updateOnline` already follow, and the base path
    // used to break: above 60 Hz, roughly every other render call plans zero
    // ticks, and an edge computed on one of those calls (a charged pass or
    // shot release, a switch press, a dodge) was consumed by the latch
    // machine and then never stepped into the simulation. Leaving the
    // latches/queues alone means the still-current button LEVEL produces the
    // same edge on the next render call that actually ticks -- edges are
    // derived at sim cadence, not render cadence.
    const ticks = host.planTicks(dt);
    if (ticks > 0) {
      const carrying = this.carrying();
      const poll: MatchControlPoll = {
        actionDown: bindings.isDown("action", this.ports.keyboard, this.ports.gamepad),
        playDown: bindings.isDown("play", this.ports.keyboard, this.ports.gamepad),
        modifierDown: bindings.isDown("modifier", this.ports.keyboard, this.ports.gamepad),
      };
      const { contextual, dashEdge } = stepMatchControlLatches(this.latches, carrying, poll);
      const switchPlayer = this.switchPending;
      this.switchPending = false;

      let sample = this.capture.sample({ ...contextual, switchPlayer });
      if (dashEdge) {
        sample = { ...sample, edges: sample.edges | inputSample.packEdges(["dash"]) };
      }
      this.lastStepSample = sample;

      for (let step = 0; step < ticks; step += 1) {
        // KNOWN SIMPLIFICATION: every tick this render call produces is fed
        // the SAME sample -- see this class's doc comment.
        this.recordReplayFrame(); // pre-step, so a goal's flight remains in the buffer.
        host.step(sample);
        // One tick's worth of discrete match events is transient -- `frame()`
        // only ever reports the LAST stepped tick's events, so a catch-up
        // batch (`ticks > 1`) must accumulate per tick, immediately after
        // each `step`, or every tick but the last would be silently dropped.
        this.appendObservedFrameEvents(host.frame().events, rosterIds);
        // Breaking out of the tick loop here (`host.cancelPlannedTicks()`) is
        // the TS-side analog of what a `false` step_fn return does to the
        // accumulator on the Rust side (see `SimHostPort.cancelPlannedTicks`'s
        // doc): stop a catch-up batch as soon as the match ends. Falls
        // through to the shared post-batch bookkeeping below either way --
        // goal/lifecycle detection must run unconditionally after every
        // tick, not only when a full batch completes.
        if (host.frame().hud.finished) {
          host.cancelPlannedTicks();
          break;
        }
      }
    }
    this.finishBaseUpdate(dt, scoreBefore);
  }

  // Translate one tick's `RenderFrame.events` into `observedFrameEvents`
  // entries, recovering each event's actor id from its one-based roster
  // slot -- see `RealMatchRenderFrameEvents`'s doc. Every wire event kind is
  // forwarded, not just the twelve `@gc/app`'s `match_observer.ts` reacts
  // to: its `MatchObserver.observe` iterates `state.events` (ALL kinds
  // emitted that tick) and switches on `event.kind` itself, so filtering
  // here would diverge from what that function actually receives, not
  // merely from what it acts on.
  private appendObservedFrameEvents(
    events: RealMatchRenderFrameEvents | undefined,
    rosterIds: readonly string[] | undefined,
  ): void {
    if (events === undefined) {
      return;
    }
    for (let index = 0; index < events.count; index += 1) {
      const kind = events.kind[index];
      if (kind === undefined) {
        continue;
      }
      const slot = events.slot[index];
      const player = slot !== undefined ? rosterIds?.[slot - 1] : undefined;
      this.observedFrameEvents.push(player !== undefined ? { kind, player } : { kind });
    }
  }

  // Base-mode goal-replay recording: one pre-step capture per simulated
  // tick, under `ReplayPort`'s own private boundary sequence. A no-op
  // unless both `MatchScreenPorts.matchState` and `ReplayPort.record` are
  // wired -- see `ReplayPort`'s doc.
  private recordReplayFrame(): void {
    if (this.ports.matchState !== undefined && this.ports.replay?.record !== undefined) {
      this.ports.replay.record(this.ports.matchState());
    }
  }

  // This screen's post-tick bookkeeping for the BASE (non-rollback) game
  // loop: render-owned smoothing/view state, then the live goal edge
  // (tracked via `lastScore`/`lastHome`) that starts a fresh replay
  // sequence.
  private finishBaseUpdate(dt: number, scoreBefore: number): void {
    const host = this.host!;
    const frame = host.frame();
    const hud = frame.hud;
    const scoreAfter = hud.home_score + hud.away_score;
    // `release_follow.update(events, dt)` -- the renderer-owned kick
    // follow-through window, aged by this render call's dt and then latched
    // from THIS call's own accumulated event batch (`observedFrameEvents`,
    // filled once per simulated tick above, so a catch-up batch cannot drop
    // a release). Presentation only: it never enters a snapshot, a hash or a
    // resimulation -- see `release_follow.ts`'s header. Read back out at the
    // payload-build site (`sim_host.ts`'s `frame()`), which turns it into
    // the roster-slot mask `RenderFrameOptions.kick_follow` is built from.
    //
    // Sits before `updateBaseRenderSmoothing` so that call's own lifecycle
    // reset (a goal, full time) still gets the last word and clears a window
    // that must not survive the timeline that produced it.
    releaseFollow.update(this.observedFrameEvents, dt);
    // `dispossession_flinch.update(events, dt, ownerId)` (#591) -- the
    // renderer-owned victim reaction window for a standing-poke tackle, same
    // shape and same call-site rationale as `releaseFollow.update` above.
    // `ownerId` is THIS call's own carrier, read the same way
    // `matchObservationState`/`onlineState` already do (`frame.possession.owner`,
    // a one-based roster slot, resolved against `host.roster().ids`) --
    // `dispossession_flinch.ts`'s header explains why handing it the CURRENT
    // owner is what lets it resolve next call's tackle against THIS call's
    // owner ("the owner just before this batch").
    const ownerSlot = frame.possession.owner;
    const ownerId = ownerSlot !== undefined ? host.roster().ids?.[ownerSlot - 1] : undefined;
    dispossessionFlinch.update(this.observedFrameEvents, dt, ownerId);
    this.updateBaseRenderSmoothing(dt, scoreAfter !== scoreBefore || hud.finished);
    if (scoreAfter > this.lastScore && !hud.finished) {
      const scoringTeam: "home" | "away" = hud.home_score > this.lastHome ? "home" : "away";
      if (this.ports.replay?.start?.(scoringTeam) === true) {
        // The scene jumps back in time, but view state must not carry the
        // post-goal kickoff pose into it.
        viewState.reset();
        cameraFollow.reset();
        releaseFollow.reset();
        dispossessionFlinch.reset();
        player3dResetAnimation();
        this.replayState = undefined;
      }
    }
    this.lastHome = hud.home_score;
    this.lastScore = scoreAfter;
  }

  // `update_render_smoothing`'s base-mode counterpart: a scene discontinuity
  // (a score edge or full time) clears smoothing/view state outright;
  // otherwise smoothing decays toward the current authoritative source and
  // view state advances from it. A no-op without `MatchScreenPorts.matchState`
  // -- matches this class's behavior before this capability existed.
  private updateBaseRenderSmoothing(dt: number, lifecycleReset: boolean): void {
    const source = this.correctionSource();
    if (source === undefined) {
      return;
    }
    if (lifecycleReset) {
      this.renderSmoothing = correctionSmoothing.new(source);
      viewState.reset();
      cameraFollow.reset();
      releaseFollow.reset();
      dispossessionFlinch.reset();
      player3dResetAnimation();
    } else {
      this.renderSmoothing =
        this.renderSmoothing !== undefined
          ? correctionSmoothing.advance(this.renderSmoothing, source, dt)
          : correctionSmoothing.new(source);
    }
    // Unconditional relative to the branch above -- reseeds immediately
    // even on a lifecycle reset rather than waiting for the next render
    // call.
    viewState.update(source.players, dt);
    this.updateCameraFollow(dt);
  }

  /**
   * The driver for render/camera_follow.ts's broadcast-following camera.
   *
   * This call site used to be entirely missing: `camera_follow.ts` existed
   * in the workspace, but `cameraFollow.update` had no caller anywhere, so
   * the smoothed focus never left its initial `undefined`, `cameraFollow
   * .view` always returned `undefined`, and every projection silently fell
   * back to the whole-pitch view (`pitch.ts`'s `currentView`) no matter what
   * `pitch.follow_camera` said. The follow camera was inert, and the flag
   * that selects it could not do anything -- which is why the product shot
   * read as a fixed establishing view of the stadium rather than a camera
   * watching the match.
   *
   * Takes the FULL `MatchState` (`this.ports.matchState`), not
   * {@link correctionSource}'s slice: `cameraFollow.update` reads `field`
   * (to scale its deadzone/keep boxes and clamp the focus inside the
   * touchlines) and `controlled` (its `ball_weight` blend against the
   * steered player), neither of which that slice carries.
   *
   * No `correctionSmoothing.pose` is threaded through to this call, so it
   * tracks the authoritative ball rather than the smoothed displayed one;
   * the difference is a sub-frame correction offset, and wiring a pose
   * through both call sites together is its own change.
   */
  private updateCameraFollow(dt: number): void {
    const state = this.ports.matchState?.();
    if (state !== undefined) {
      cameraFollow.update(state, dt);
    }
  }

  // The legacy (BASE, non-rollback) replay branch: the sim is frozen (no
  // `host.step` this call) while the buffered footage plays back through
  // view state.
  private updateLegacyReplay(dt: number): void {
    const frame = this.ports.replay!.step!(dt);
    this.replayState = frame;
    if (frame !== undefined) {
      viewState.update(
        frame.players.map((p) => ({ id: p.id, pos: p.pos })),
        dt,
      );
      return;
    }
    this.finishReplay();
  }

  /**
   * `advance_rollback_replay(self, dt)` -- the rollback-mode counterpart of
   * {@link updateLegacyReplay}'s `ReplayPort.step` use. Previously never
   * called anywhere in rollback mode: a confirmed rollback goal could start
   * a replay sequence (`consumeConfirmedLifecycle`'s `ports.replay.startAt`)
   * but nothing ever advanced it afterward, so it played its first frame
   * forever and `ReplayPort.active()` never reported `false` again. Called
   * both from `updateRollback`'s early-return branch when the rollback
   * source is terminal, and unconditionally near the end of its normal path
   * -- a no-op either place when no replay is wired or none is active.
   *
   * @returns whether a replay was active and this call is standing in for
   * the caller's own render-only positional work.
   */
  private advanceRollbackReplay(dt: number): boolean {
    const replay = this.ports.replay;
    if (replay?.step === undefined || !replay.active()) {
      return false;
    }
    const frame = replay.step(dt);
    this.replayState = frame;
    if (frame !== undefined) {
      viewState.update(
        frame.players.map((p) => ({ id: p.id, pos: p.pos })),
        dt,
      );
      return true;
    }
    this.finishReplay();
    return true;
  }

  /**
   * Resets render smoothing to a fresh, offset-free state built from the
   * rollback client's current displayed positions, and resets view state.
   * Called from `updateRollback`'s early-return branch when the rollback
   * source is terminal. The caller only reaches this after
   * {@link advanceRollbackReplay} already reported no active replay to fall
   * back to, so unlike {@link finishReplay} this never touches
   * `this.replayState`/`ReplayPort`.
   */
  private clearRollbackRenderSmoothing(): void {
    viewState.reset();
    cameraFollow.reset();
    releaseFollow.reset();
    dispossessionFlinch.reset();
    player3dResetAnimation();
    const source = this.correctionSource();
    if (source === undefined) {
      return;
    }
    this.renderSmoothing =
      this.renderSmoothing !== undefined
        ? correctionSmoothing.clear(this.renderSmoothing, source)
        : correctionSmoothing.new(source);
  }

  /**
   * This screen's rollback-mode game-loop branch. Unlike the base branch
   * above, this one does NOT sample input unconditionally: a zero-tick
   * render call touches neither `this.latches` nor
   * `this.switchPending`/`capture`, so a one-shot edge queued just before a
   * zero-tick call survives to the next render call that actually steps --
   * see `match_rollback_lab.spec.ts`'s "retains a zero-tick edge and
   * consumes it exactly once" (the base branch's own KNOWN SIMPLIFICATION
   * deliberately does the opposite; the rollback branch preserves the real
   * input-adapter behavior instead).
   *
   * `rollbackOutputs`/`rollbackEventDiffs`/`rollbackConfirmedSteps`/
   * `rollbackCorrections`/`rollbackFrameEvents` are this render call's batch
   * ONLY -- cleared up front, never accumulated across calls.
   *
   * The early-return gate below is this screen's rollback-mode "is the
   * match over" check, {@link rollbackTerminal} over the host's own
   * reported status -- NOT `this.finished`/`hud.finished`. Those two ARE
   * different signals here: `hud.finished` is a snapshot-restored field
   * that can read `true` for a long settlement window before the rollback
   * source itself goes terminal (or, depending on the host, briefly the
   * other way around), and this screen keeps ticking/reconciling through
   * that window -- only once the source reports terminal does this method
   * stop calling it at all. `hud.finished` still matters, just further
   * down: it feeds this method's own lifecycle-reset fallthrough (see the
   * tail below), which clears render smoothing on THAT read, independently
   * of this gate.
   */
  private updateRollback(dt: number): void {
    const rollbackHost = this.rollbackHost!;
    this.rollbackOutputs = [];
    this.rollbackEventDiffs = [];
    this.rollbackConfirmedSteps = [];
    this.rollbackCorrections = [];
    this.rollbackFrameEvents = [];

    if (rollbackTerminal(rollbackHost.debug().status)) {
      if (!this.advanceRollbackReplay(dt)) {
        this.clearRollbackRenderSmoothing();
      }
      return;
    }
    const paused = this.ports.tuningPause?.open === true;
    if (paused || rollbackHost.debug().status === "unconfirmed_window_exceeded") {
      rollbackHost.cancelPlannedTicks();
      return;
    }

    const ticks = rollbackHost.planTicks(dt);

    const outputs: RollbackLabOutput[] = [];
    const diffs: RollbackEventDiff[] = [];
    const confirmed: RollbackEventStep[] = [];
    const corrections: RollbackLabCorrectionSample[] = [];
    // A zero-tick render call touches neither `this.latches` nor
    // `this.switchPending`/`capture` (see this method's doc), but the
    // render-only settling below (smoothing decay, view state, presentation
    // consumption) is NOT gated on ticks having run this call -- it must
    // run unconditionally, whether or not this render call produced any
    // ticks. Skipping them at `ticks === 0` used to mean a
    // sub-frame render call (e.g. the settling step right after a
    // correction lands) could never decay `correction_magnitude` at all.
    if (ticks > 0) {
      const carrying = this.carrying();
      const poll: MatchControlPoll = {
        actionDown: bindings.isDown("action", this.ports.keyboard, this.ports.gamepad),
        playDown: bindings.isDown("play", this.ports.keyboard, this.ports.gamepad),
        modifierDown: bindings.isDown("modifier", this.ports.keyboard, this.ports.gamepad),
      };
      const { contextual, dashEdge } = stepMatchControlLatches(this.latches, carrying, poll);
      const switchPlayer = this.switchPending;
      this.switchPending = false;
      let sample = this.capture.sample({ ...contextual, switchPlayer });
      if (dashEdge) {
        sample = { ...sample, edges: sample.edges | inputSample.packEdges(["dash"]) };
      }
      this.lastStepSample = sample;

      for (let step = 0; step < ticks; step += 1) {
        // Unlike the base branch's KNOWN SIMPLIFICATION (which reuses one
        // sample verbatim for a whole catch-up batch), the rollback branch
        // delivers this render call's one-shot edges (switch, dash, ...) to
        // only the FIRST tick of a multi-tick batch; a HELD state (sprint,
        // jockey, lob, equipment) still applies to every tick. Matches
        // `match_rollback_lab.spec.ts`'s "aggregates multi-tick edges,
        // holds, and corrections", which checks the edge fires on tick index
        // 1 of the batch only, while the held mask is constant across all of
        // them.
        const tickSample: InputSample = step === 0 ? sample : { ...sample, edges: 0 };
        const result = rollbackHost.step(tickSample);
        outputs.push(result.output);
        diffs.push(...result.eventDiffs);
        confirmed.push(...result.confirmedSteps);
        if (result.correction !== undefined) {
          corrections.push(result.correction);
        }
        if (result.debug.status === "unconfirmed_window_exceeded") {
          // Synchronization failure mid-batch: stop consuming further ticks
          // this call and discard any smoothing already applied -- the
          // "clears rollback handoff batches before ... terminal early
          // returns" behavior. What was already gathered THIS call still
          // committed (`outputs` stays non-empty); diffs/confirmed/
          // corrections stay at their cleared initial value.
          this.renderSmoothing = undefined;
          rollbackHost.cancelPlannedTicks();
          this.rollbackOutputs = outputs;
          return;
        }
        if (rollbackHost.frame().hud.finished) {
          rollbackHost.cancelPlannedTicks();
          break;
        }
      }
    }
    this.rollbackOutputs = outputs;
    this.rollbackEventDiffs = diffs;
    this.rollbackConfirmedSteps = confirmed;
    this.rollbackCorrections = corrections;

    // This method's rollback-mode render-smoothing fallthrough.
    // `hudFinished` is this mode's lifecycle-reset trigger -- score-edge/
    // kickoff have no rollback-mode presentation counterpart yet (this
    // file's own kickoff `it.skip`), and a mid-batch sync failure already
    // returned above (the `unconfirmed_window_exceeded` branch inside the
    // loop), so it can never reach here. THIS is the fallthrough the
    // top-of-method `rollbackTerminal` gate cannot substitute for -- see
    // this method's own doc: a rollback source can read `hud.finished` true
    // well before it goes terminal, and render smoothing must clear on
    // THIS read, not that one.
    const positions = rollbackHost.displayedPositions();
    const hudFinished = rollbackHost.frame().hud.finished;
    const drawingRollbackReplay = this.ports.replay?.active() === true;
    if (hudFinished) {
      this.renderSmoothing = correctionSmoothing.new(positions);
      if (!drawingRollbackReplay) {
        viewState.reset();
        cameraFollow.reset();
        releaseFollow.reset();
        dispossessionFlinch.reset();
        player3dResetAnimation();
      }
    } else {
      for (const correction of corrections) {
        this.renderSmoothing =
          this.renderSmoothing === undefined
            ? correctionSmoothing.correct(
                correctionSmoothing.new(correction.source),
                correction.source,
              )
            : correctionSmoothing.correct(this.renderSmoothing, correction.source);
      }
      if (this.renderSmoothing !== undefined && dt > 0) {
        this.renderSmoothing = correctionSmoothing.advance(this.renderSmoothing, positions, dt);
      }
    }

    // Live view state (gait/lean), from the displayed rollback client --
    // skipped while a confirmed rollback replay is drawing the screen
    // instead (`advanceRollbackReplay` below owns view state then).
    if (!drawingRollbackReplay) {
      viewState.update(positions.players, dt);
      // Same gate, same pairing as the base path's own
      // `viewState.update`/`updateCameraFollow` -- the follow camera has to
      // be driven alongside view state in both modes or it stalls in
      // whichever mode was missed. A no-op in rollback configurations that
      // wire no `matchState` port (see {@link updateCameraFollow}), which is
      // why it is safe to call unconditionally here.
      this.updateCameraFollow(dt);
    }

    if (this.rollbackConsumerPorts !== undefined) {
      consumeRollbackPresentation(
        this.rollbackConsumer,
        this.rollbackConsumerPorts,
        true,
        this.rollbackFrameEvents,
        diffs,
        confirmed,
      );
    }

    // `advance_rollback_replay(self, dt)` -- steps a confirmed rollback-goal
    // replay so it actually advances/finishes instead of freezing on its
    // first frame forever (see `advanceRollbackReplay`'s own doc for why
    // this call was missing entirely before this fix). A no-op when none is
    // active.
    this.advanceRollbackReplay(dt);
  }

  /**
   * This screen's ONLINE-mode game-loop branch -- see "THE ONLINE-DRIVEN
   * MATCH SCREEN" section's header. Samples this render call's input once (the
   * same latch/contextual pipeline base/rollback use) and drives
   * `onlineHost.advance` in a bounded loop, gated by `needsLocalSample()`
   * every iteration (not just once) so the driver's own readiness --
   * confirmation window, transport backpressure -- can stop this screen
   * mid-batch, matching how `updateRollback`'s `unconfirmed_window_exceeded`
   * check stops that mode mid-batch. `onlineAccumulator`'s bound is a
   * runaway-loop guard only, not a determinism-relevant catch-up/drop
   * decision -- see [`OnlineHostPort.needsLocalSample`]'s own doc for why
   * this does not reintroduce the TypeScript fixed-clock accumulator
   * `SimHostPort.planTicks` exists to keep singular, in Rust: the DRIVER
   * (`gc_netcode::match_driver`) is what decides whether an attempted tick
   * is actually accepted, not this loop's tick count.
   */
  private updateOnline(dt: number): void {
    const onlineHost = this.onlineHost!;
    if (this.finished) {
      return;
    }
    this.onlineAccumulator += dt;
    let ticks = 0;
    while (
      this.onlineAccumulator + ONLINE_CLOCK_EPSILON >= ONLINE_TICK_SECONDS &&
      ticks < MAX_ONLINE_TICKS_PER_UPDATE
    ) {
      this.onlineAccumulator -= ONLINE_TICK_SECONDS;
      ticks += 1;
    }
    if (ticks === 0) {
      return;
    }
    const carrying = this.carrying();
    const poll: MatchControlPoll = {
      actionDown: bindings.isDown("action", this.ports.keyboard, this.ports.gamepad),
      playDown: bindings.isDown("play", this.ports.keyboard, this.ports.gamepad),
      modifierDown: bindings.isDown("modifier", this.ports.keyboard, this.ports.gamepad),
    };
    const { contextual, dashEdge } = stepMatchControlLatches(this.latches, carrying, poll);
    const switchPlayer = this.switchPending;
    this.switchPending = false;
    let sample = this.capture.sample({ ...contextual, switchPlayer });
    if (dashEdge) {
      sample = { ...sample, edges: sample.edges | inputSample.packEdges(["dash"]) };
    }
    this.lastStepSample = sample;

    for (let step = 0; step < ticks; step += 1) {
      if (!onlineHost.needsLocalSample()) {
        break;
      }
      // Mirrors `updateRollback`'s own rule: this render call's one-shot
      // edges (switch, dash, ...) go to the FIRST tick of a multi-tick
      // batch only; a HELD state applies to every tick.
      const tickSample: InputSample = step === 0 ? sample : { ...sample, edges: 0 };
      this.onlineTickCount += 1;
      onlineHost.advance(this.onlineTickCount, tickSample);
      if (this.finished) {
        break;
      }
    }
  }

  /**
   * Draw the current frame. Separated from `update` (see that method's
   * doc) so callers control the render cadence independently of the
   * simulation cadence, and so this class satisfies
   * [`RealMatchScreenPort.draw`] (`real_match.ts`) via
   * [`MatchScreenAsRealMatchScreen`]. Reads only `frame()`/`roster()` off
   * the active host -- never `debug()`/`currentSnapshot()`/
   * `referenceSnapshot()` -- so drawing cannot mutate either rollback-lab
   * match (`match_rollback_lab.spec.ts`'s "draws only from the cached debug
   * model without mutating either match").
   */
  draw(): void {
    if (this.onlineHost !== undefined) {
      this.drawOnline();
      return;
    }
    const host = this.activeHost();
    const frame = host.frame();
    const roster = host.roster();
    this.ports.renderer.draw(frame, roster);
  }

  /**
   * ONLINE mode's `draw()` -- see "THE ONLINE-DRIVEN MATCH SCREEN" section's
   * header. Overrides `hud.controlled`, `control.controlled`, and the
   * per-slot `players.controlled` highlight array to the driver-corrected
   * assignment ([`onlineState`]) before forwarding to
   * [`RenderPort.draw`] -- otherwise the renderer draws the RAW host's own
   * (wrong) notion of who is controlled, silently passing by drawing the
   * wrong thing correctly.
   */
  private drawOnline(): void {
    const { frame, players, control, roster } = this.onlineFrame();
    const state = this.onlineState;
    const controlledSlot = state.controlled + 1; // back to one-based for the wire shape
    const overriddenPlayers: OnlineRenderFramePlayers = {
      ...players,
      controlled: players.controlled.map((_, index) => index === state.controlled),
    };
    const overriddenControl: OnlineRenderFrameControl = { ...control, controlled: controlledSlot };
    const overriddenFrame: RenderFrame = {
      ...frame,
      hud: { ...frame.hud, controlled: controlledSlot },
      players: overriddenPlayers,
      control: overriddenControl,
    };
    this.ports.renderer.draw(overriddenFrame, roster);
  }

  /** Release the underlying host's resources. Safe to call once; does not itself call `SimHostPort.dispose` twice, matching that contract's own "safe to call twice" note being unnecessary to rely on here. */
  dispose(): void {
    this.activeHost().dispose();
  }
}

// =============================================================================
// RECONCILING TWO MatchScreen CONTRACTS. `real_match.ts`'s
// `RealMatchScreenPort<TState, TStep>` (`state.score.home/away`,
// `state.time_left`, `rollbackLab`, `frameEvents`, a no-arg `draw()`,
// `teardown()`, `applySettings()`) was written first, against a `Match`
// class that did not exist yet -- it is generic, and already used that way
// by `online_match.ts`'s `OnlineMatchState`. `MatchScreen` above (this
// file's game loop, built afterward) satisfies none of it directly: no
// `state`/`rollbackLab`/`frameEvents`, `event()` takes `@gc/input`'s
// `ControllerInputEvent` rather than `real_match.ts`'s smaller
// `RealMatchInputEvent`, and `teardown`/`applySettings` do not exist.
//
// Rather than mutate `MatchScreen.event`'s signature to a union it mostly
// does not need (real_match.ts deliberately does not depend on `@gc/input`
// -- see that module's header -- so its `RealMatchInputEvent.action` is a
// plain `string`, not `@gc/input`'s closed `ActionName`), this section is
// the one adapter that bridges the two: [`MatchScreenAsRealMatchScreen`]
// composes a `MatchScreen` and implements `RealMatchScreenPort` around it,
// so `RealMatchScreen` (real_match.ts) can actually drive the real match
// screen wave 2 built. This is "one shape, not two" the way the codebase
// elsewhere handles a genuine vocabulary gap between two packages that must
// not depend on each other (`@gc/online`'s `match_presentation.ts`'s own
// translated types), not a redesign of either existing contract -- `real_match.ts` is
// unchanged, and `MatchScreen`'s own `event`/`update`/`draw`/`dispose` keep
// their existing, stricter types for every other caller.
// =============================================================================

/**
 * Bridges `@gc/input`'s `ControllerInputEvent` (`MatchScreen`'s own input
 * vocabulary) and `real_match.ts`'s smaller, package-agnostic
 * `RealMatchInputEvent` -- both describe the same closed key/action
 * vocabulary (`@gc/input`'s `ActionName`/`bindings`), just typed
 * differently because `real_match.ts` deliberately does not import
 * `@gc/input`. This is the one place that gap is bridged: widen to a shape
 * both variants satisfy, then narrow `action` back to `ActionName`. `key`/
 * `gamepad`/`click` events need no bridging -- `RealMatchInputEvent`'s key
 * variant (no `pressed`) is already assignable to `@gc/input`'s `KeyEvent`
 * (`pressed` is optional there), and `gamepad`/`click` are not part of
 * `RealMatchInputEvent`'s vocabulary at all.
 */
function toControllerInputEvent(
  evt: ControllerInputEvent | RealMatchInputEvent,
): ControllerInputEvent {
  if (evt.kind !== "action") {
    return evt;
  }
  const wide = evt as { kind: "action"; action: string; pressed?: boolean; source?: InputSource };
  return {
    kind: "action",
    action: wide.action as ActionName,
    ...(wide.pressed !== undefined ? { pressed: wide.pressed } : {}),
    ...(wide.source !== undefined ? { source: wide.source } : {}),
  };
}

/**
 * Adapts a {@link MatchScreen} to `real_match.ts`'s `RealMatchScreenPort`,
 * so {@link RealMatchScreen} (real_match.ts) can drive the real game-loop
 * screen this file builds -- see this section's header. `TStep` is `never`:
 * this milestone's `MatchScreen` has no rollback lab (`rollbackLab` is
 * always `false`), so `RealMatchScreen` never reads
 * `rollbackConfirmedSteps`, and an empty array satisfies `readonly TStep[]`
 * for any `TStep`.
 */
export class MatchScreenAsRealMatchScreen implements RealMatchScreenPort<RealMatchState, never> {
  private readonly screen: MatchScreen;

  constructor(screen: MatchScreen) {
    this.screen = screen;
  }

  /**
   * `real_match.ts`'s `RealMatchState` -- read live off the screen's
   * current frame, mirroring `MatchScreen.finished`'s own "never cached"
   * rule. `roster`/`owner` come from `MatchScreen.matchObservationState`
   * (`SimHostPort.roster()` plus `frame().possession.owner`) -- see that
   * getter's doc for when `roster` is `undefined`.
   */
  get state(): RealMatchState {
    const hud = this.screen.hud;
    return {
      time_left: hud.time_left,
      score: { home: hud.home_score, away: hud.away_score },
      ...this.screen.matchObservationState,
    };
  }

  /** This milestone's `MatchScreen` has no rollback lab -- see this class's doc. */
  readonly rollbackLab = false;

  /** Never read: `rollbackLab` is always `false` -- see this class's doc. */
  readonly rollbackConfirmedSteps: readonly never[] = [];

  /** `MatchScreen.debugCombatEnabled` -- see that getter's doc. */
  get debugCombatEnabled(): boolean {
    return this.screen.debugCombatEnabled;
  }

  /**
   * `MatchScreen.matchObservationEvents` -- this render call's discrete
   * match events, translated from `RenderFrame.events` and accumulated
   * across every tick `MatchScreen.update` actually stepped. Untyped
   * `unknown[]` here (this module's own `RealMatchScreenPort.frameEvents`
   * contract), but each entry is really a `RealMatchEvent`
   * (`{kind, player?}`) -- a `MatchObserverPort` implementation reads it as
   * such, the same way `@gc/app`'s `match_observer.ts` reads `state.events`.
   */
  get frameEvents(): readonly unknown[] {
    return this.screen.matchObservationEvents;
  }

  fullTimeConfirmed(): boolean {
    return this.screen.finished;
  }

  /** No replay is wired into `MatchScreen` this milestone (see this file's header), so completion is never blocked. */
  resultCompletionBlocked(): boolean {
    return false;
  }

  update(dt: number): void {
    this.screen.update(dt);
  }

  event(evt: RealMatchInputEvent): void {
    this.screen.event(toControllerInputEvent(evt));
  }

  draw(): void {
    this.screen.draw();
  }

  teardown(): void {
    this.screen.dispose();
  }

  /**
   * Settings (audio/graphics volume, ...) are not wired into `MatchScreen`
   * this milestone -- no module owns them yet (this file's header).
   * A no-op, not a lie: nothing in this milestone's `MatchScreen` reads a
   * setting, so there is nothing to apply.
   */
  applySettings(_settings: GameSettings): void {}
}

/**
 * Adapts a {@link MatchScreen} constructed with an
 * {@link MatchScreenOptions.online} host to `online_match.ts`'s
 * `OnlineMatchOptions.newMatch` return contract
 * (`RealMatchScreenPort<OnlineMatchState, unknown>`) -- the ONLINE-mode
 * counterpart of {@link MatchScreenAsRealMatchScreen}, over the same
 * `MatchScreen`/`RealMatchScreenPort` gap that section's header describes.
 * `TStep` is `unknown`, not `never`: `online_match.ts`'s own
 * `RealMatchScreenPort<OnlineMatchState, unknown>` bound requires it, and
 * (like `MatchScreenAsRealMatchScreen`) this milestone's `MatchScreen` never
 * populates `rollbackConfirmedSteps` regardless -- `rollbackLab` stays
 * `false` even for an online-mode screen, matching that this class drives
 * the OMP-3 driver directly, not `sim.rollback_playable_lab`'s local
 * laboratory.
 */
export class MatchScreenAsOnlineMatchScreen implements RealMatchScreenPort<
  OnlineMatchState,
  unknown
> {
  private readonly screen: MatchScreen;

  constructor(screen: MatchScreen) {
    this.screen = screen;
  }

  /** `MatchScreen.onlineState` -- see that getter's doc for the `controlled` override this section's header describes. */
  get state(): OnlineMatchState {
    return this.screen.onlineState;
  }

  /** This screen drives the OMP-3 online match driver directly, not `sim.rollback_playable_lab`'s local laboratory -- see this class's doc. */
  readonly rollbackLab = false;

  /** Never read: `rollbackLab` is always `false` -- see this class's doc. */
  readonly rollbackConfirmedSteps: readonly unknown[] = [];

  /** `MatchScreen.debugCombatEnabled` -- see that getter's doc. */
  get debugCombatEnabled(): boolean {
    return this.screen.debugCombatEnabled;
  }

  /** Not yet wired -- see `MatchScreenAsRealMatchScreen.frameEvents`'s identical doc; the same gap applies here. */
  readonly frameEvents: readonly unknown[] = [];

  fullTimeConfirmed(): boolean {
    return this.screen.finished;
  }

  /** No replay is wired into `MatchScreen` this milestone (see this file's header), so completion is never blocked. */
  resultCompletionBlocked(): boolean {
    return false;
  }

  update(dt: number): void {
    this.screen.update(dt);
  }

  event(evt: RealMatchInputEvent): void {
    this.screen.event(toControllerInputEvent(evt));
  }

  draw(): void {
    this.screen.draw();
  }

  teardown(): void {
    this.screen.dispose();
  }

  /** A no-op -- see `MatchScreenAsRealMatchScreen.applySettings`'s identical doc. */
  applySettings(_settings: GameSettings): void {}
}
