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
import { correctionSmoothing, viewState } from "@gc/render";
import type { correctionSmoothingTypes, replayTypes } from "@gc/render";
import type { LifecyclePayload } from "./online_match_model.ts";
import type { GameSettings } from "./content.ts";
import type { RealMatchInputEvent, RealMatchScreenPort, RealMatchState } from "./real_match.ts";
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

/**
 * `game/render/replay.lua`, injected -- see this module's header.
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
  /** Record one live pre-step frame under this port's own private, contiguous boundary sequence -- `replay.record`. Called once per simulated tick, before `SimHostPort.step`, so a goal's flight remains in the buffer (matches `game/screens/match.lua`'s own ordering). */
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

// =============================================================================
// THE GAME LOOP -- gather input, step the fixed simulation clock, hand frames
// to the renderer. This is the part of `game/screens/match.lua` this
// package's original scope note (this module's header) said could not be
// built without a wasm bridge. That bridge (`crates/gc-wasm`, `@gc/wasm`) now
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
// whether the controlled player has the ball (`hud.controlled_owns_ball`,
// the direct analog of `game/screens/match.lua`'s `self.state.owner ==
// self.state.controlled`) before it can compute a single contextual input
// field, or expose `hud.home_score`/`away_score`/`time_left` to
// [`MatchScreenAsRealMatchScreen`]'s `state` (`real_match.ts`'s
// `RealMatchScreenPort`). `@gc/render`'s real `frame_buffer.ts` decoder
// always produced all of these -- the gap was that `sim_host.ts` used to
// carry its own trimmed, hand-duplicated decoder that only constructed the
// drawing slice. `@gc/render` now exports `frame_buffer` directly
// (`frameBuffer`), so `sim_host.ts` decodes through the real, untrimmed
// `decode`/`decodeRoster`/`toRenderFrame` and its `RenderFrame` is that
// module's complete wire type -- see that file's header. `RenderFrameHud`
// below only declares the fields THIS module's own game-loop logic reads
// (README rule 6.7): the wire carries more (`possession_team`,
// `controlled_stamina`, ...), but nothing here inspects them.
//
// `field`/`players`/`ball`/`control`/`events`/`roster`'s per-player arrays
// are NOT declared here: this module never reads them, only forwards
// whatever `SimHostPort.frame()`/`.roster()` produced to [`RenderPort.draw`]
// untouched (the same "only the fields this module reads are declared
// locally" rule `pitch.ts` documents for its own slice, README rule 6.7).
// =============================================================================

/** `gc_sim::input_frame::InputSample` (`@gc/input`'s TS mirror; see `input_sample.ts`'s header). Re-exported so a `SimHostPort` implementation need not also import `@gc/input` merely to name this type. */
export type InputSample = inputSampleTypes.InputSample;

/** `crates/gc-render/src/frame.rs`'s `RenderFrameHud` (mirrored via `@gc/render`'s `frame_buffer.ts`'s `RenderFrameHud`, field-for-field) -- only the fields this module's own game-loop logic reads. See this section's header. */
export interface RenderFrameHud {
  readonly finished: boolean;
  /** `self.state.owner == self.state.controlled` in `game/screens/match.lua` -- "carrying". */
  readonly controlled_owns_ball: boolean;
  /** Read by [`MatchScreenAsRealMatchScreen`]'s `state` (`real_match.ts`'s `RealMatchState.score.home`). */
  readonly home_score: number;
  /** Read by [`MatchScreenAsRealMatchScreen`]'s `state` (`real_match.ts`'s `RealMatchState.score.away`). */
  readonly away_score: number;
  /** Read by [`MatchScreenAsRealMatchScreen`]'s `state` (`real_match.ts`'s `RealMatchState.time_left`). */
  readonly time_left: number;
}

/** `crates/gc-render/src/frame.rs`'s `RenderFramePossession` -- declared for parity with the real wire shape; this module does not currently read a field off it (carrying comes off `hud.controlled_owns_ball` instead, matching the Lua original's own comparison). */
export interface RenderFramePossession {
  readonly owner?: number;
  readonly owner_team?: "home" | "away";
}

/**
 * `crates/gc-render/src/frame.rs`'s `RenderFrame` -- the whole interface
 * between the simulation and a renderer. Only `hud`/`possession` are typed
 * here (this module's own reads); `field`/`players`/`ball`/`control`/
 * `events`/`roster` are real fields on the wire object this module receives
 * and forwards to [`RenderPort.draw`], but this module never inspects them,
 * so they are not declared -- see this section's header.
 */
export interface RenderFrame {
  readonly hud: RenderFrameHud;
  readonly possession: RenderFramePossession;
}

/**
 * `crates/gc-render/src/frame.rs`'s `RenderFrameRoster` -- match-constant
 * per-player identity. Opaque here: this module never inspects it, only
 * threads it from [`SimHostPort.roster`] to [`RenderPort.draw`], the same
 * way `OpaqueSnapshot` stays opaque in `@gc/online`'s `match_presentation.ts`.
 */
export type RenderFrameRoster = Readonly<Record<string, unknown>>;

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
   * (v2/README.md §2.1) via the wasm session, so this decision has exactly
   * one implementation, in Rust: this screen used to hand-copy that
   * algorithm (see this class's own history/[`MatchScreen.update`]'s doc),
   * which is exactly the kind of drift README §2.1 warns changing
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
 * closure. `MatchScreen.restart` (the `game/screens/match.lua` rematch path)
 * calls this again rather than owning any notion of "the same choices"
 * itself -- see `Match.new`'s `_opts` field, which this factory pattern
 * replaces: the options live in whoever builds the factory, not in this
 * screen.
 */
export type SimHostFactory = () => SimHostPort;

/**
 * `game/render/pitch.lua`'s successor (`@gc/render`, three.js), injected --
 * `@gc/render` is not currently a declared dependency of `@gc/screens` (see
 * this package's porting report: this is a needed-dependency gap to report,
 * not this file's package.json to edit). `frame`/`roster` are exactly
 * `SimHostPort.frame()`/`.roster()`'s return values, forwarded untouched;
 * this module never reads them itself past `frame.hud`.
 */
export interface RenderPort {
  draw(frame: RenderFrame, roster: RenderFrameRoster): void;
}

// -----------------------------------------------------------------------
// THE CONTEXTUAL INPUT STATE MACHINE -- shoot vs jockey, pass vs switch, the
// lob latch, aerial strike/acrobatic. Ported faithfully from
// `game/screens/match.lua`'s `Match:update` (the `frame_input` block,
// roughly lines 833-877) and `Match:event` (the `pass_switch`/`juke`
// branches, roughly lines 654-734). `@gc/input`'s `capture_frame.ts`
// deliberately leaves these out of `InputSampleCapture.sample` -- its own
// header names this file, `packages/screens`, as the one that computes them
// from live match state and passes them in via `ContextualInputFields`.
//
// Pure and independently testable: given this frame's raw control polls and
// whether the controlled player is carrying the ball, [`stepMatchControlLatches`]
// mutates the small set of frame-to-frame latches `Match:update` keeps as
// `self._shoot_held_prev`/`self._pass_held_prev`/`self._action_held_prev`/
// `self._lob_latch` and returns exactly the fields
// `InputSampleCapture.sample` needs, `switchPlayer` excepted (see below).
// -----------------------------------------------------------------------

type ContextualInputFields = captureFrameTypes.ContextualInputFields;

/** Mirrors `Match`'s `self._shoot_held_prev`/`self._pass_held_prev`/`self._action_held_prev`/`self._lob_latch` fields. */
export interface MatchControlLatches {
  shootHeldPrev: boolean;
  passHeldPrev: boolean;
  /** ACTION held off the ball last frame -- the jockey stance's previous-frame flag. */
  actionHeldPrev: boolean;
  lobLatch: boolean;
}

/** A fresh set of latches, matching `Match.new`'s initial `self._shoot_held_prev = false` etc. */
export function newMatchControlLatches(): MatchControlLatches {
  return { shootHeldPrev: false, passHeldPrev: false, actionHeldPrev: false, lobLatch: false };
}

/** This frame's raw, context-free control polls -- `control_down("action"/"play"/"modifier")` in `game/screens/match.lua`. */
export interface MatchControlPoll {
  readonly actionDown: boolean;
  readonly playDown: boolean;
  readonly modifierDown: boolean;
}

export interface MatchContextualStep {
  /** Everything `InputSampleCapture.sample` needs except `switchPlayer` -- see [`applySwitchEdge`]'s doc for why that field is computed separately, off the discrete event stream rather than a poll. */
  readonly contextual: ContextualInputFields;
  /**
   * `self._dash = true` (`Match:update`'s jockey-release branch). Not part
   * of `ContextualInputFields` -- `capture_frame.ts`'s own header explains
   * `dash` needs both a previous- and current-frame contextual value, so it
   * is not expressible as a single neutral-by-default parameter the way the
   * other contextual fields are. The caller ORs this into the built
   * `InputSample`'s `edges` bitmask directly, via `inputSample.packEdges`.
   */
  readonly dashEdge: boolean;
}

/**
 * One render frame's worth of `Match:update`'s `frame_input` block. Mutates
 * `latches` in place (mirroring the Lua original's `self._*` mutation) and
 * returns this frame's contextual fields plus the `dash` edge
 * `ContextualInputFields` cannot carry.
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
// mirror of `sim/fixed_clock.lua` / `crates/gc-sim/src/fixed_clock.rs`'s
// `advance` -- `gc_sim::fixed_clock` had no TS binding, and `SimHostPort`
// exposed only a one-fixed-tick `step`, not a batched `advance`. That was a
// determinism-relevant algorithm (tick COUNT changes what the simulation
// computes) with two implementations and no way to keep them honest against
// each other: change `MAX_TICKS_PER_UPDATE` in Rust and this copy would
// silently keep the old behavior (v2/README.md §2.1).
//
// `SimHostPort.planTicks` (`crates/gc-wasm/src/session.rs`'s `FixedClock`,
// bound through `@gc/wasm`) removes the second implementation instead of
// pinning it: this screen now only calls `planTicks`/`step`/
// `cancelPlannedTicks` in a loop, below. There is exactly one accumulator
// algorithm, and it lives in Rust.
// -----------------------------------------------------------------------

export type MatchScreenProfile = "product" | "playtest" | "online";

// =============================================================================
// THE ROLLBACK LABORATORY -- `Match.new({rollback_lab = {...}})`'s
// development-only slot-mode option, `sim.rollback_playable_lab`
// (`crates/gc-sim/src/rollback_playable_lab.rs`) ported to `game/screens/
// match.lua`'s companion state machine. See `match_rollback_lab.spec.ts`'s
// header for the two blockers this section clears (no construction option,
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
// binding) is future work this port does not attempt.
// =============================================================================

/**
 * `sim.rollback_playable_lab`'s convergence/sync status, surfaced on
 * `_rollback_debug.status`. `"diverged"`/`"late_input_unrecoverable"`/
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

/** Mirrors the Lua `NetworkProfile` shape (`fixed_profile` in the ported spec). */
export interface RollbackLabNetworkProfile {
  readonly base_delay_ticks: number;
  readonly jitter_min_ticks?: number;
  readonly jitter_max_ticks?: number;
  readonly independent_loss_rate?: number;
  readonly duplication_rate?: number;
  readonly burst_start_rate?: number;
  readonly burst_length_ticks?: number;
}

/** `Match.new`'s `opts.rollback_lab` table. */
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

/** `_rollback_debug`'s host-reported half. `active_smoothing_count`/`correction_magnitude` are NOT here -- those are computed screen-side from `correctionSmoothing.diagnostics` over this screen's own `_render_smoothing`, matching the Lua original's own split (the panel reads both a driver-reported status and a render-owned smoothing diagnostic). See {@link RollbackLabDebug}. */
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

/** `_rollback_debug`, as read by a caller of {@link MatchScreen.debugRollbackDebug} -- the host's own status plus this screen's render-owned smoothing diagnostics. */
export interface RollbackLabDebug extends RollbackLabHostDebug {
  readonly active_smoothing_count: number;
  readonly correction_magnitude: number;
}

/** `_clock`'s rollback-mode counterpart -- the same fixed-clock accumulator fields the base game loop's `SimHostPort.planTicks` hides behind a single tick count, exposed here because the ported spec asserts on `dropped_ticks`/`overloads`/`accumulator` directly. */
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

export interface MatchScreenOptions {
  /** Mirrors `Match.new`'s `opts.profile`; defaults to `"playtest"`, matching the Lua original's `self._opts.profile or "playtest"`. */
  readonly profile?: MatchScreenProfile;
  /**
   * Mirrors `Match.new`'s `opts.combat_enabled`. For a rollback-lab
   * construction, this still gates the construction-time validation against
   * the supplied `initial_snapshot`'s `CombatMatchState` companion (per
   * `rollback_playable_lab`'s own invariant) -- see `validateRollbackCombatCompanion`.
   *
   * For the BASE, non-rollback game loop, this is a "separate construction
   * option" in the sense `v2/README.md`'s porting rules use the phrase
   * (deliberately NOT part of `SimHostPort` -- see that interface's own doc
   * on why it is a fixed contract shared with `@gc/app`'s `sim_host.ts`, not
   * this file's to widen): `crates/gc-wasm/src/session.rs`'s `Session::new`
   * now accepts a `combat_enabled` parameter and threads it through every
   * `Session::step` call, mirroring `Match:restart`'s own
   * `combat_sim.new_state(initial)` -- but that choice is baked into the
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
  /** Mirrors `Match.new`'s `opts.rollback_lab`. See this section's header. */
  readonly rollback_lab?: RollbackLabOptions;
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
  /** `game.ui.tuning_panel`'s `.open` flag, injected -- pausing the panel pauses the rollback laboratory's clock. Omit to never pause. */
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
 * injected {@link RenderPort}. Ported from `game/screens/match.lua`'s
 * `Match` class, scoped to what a `step`/`frame`/`roster`/`tick`/`dispose`
 * host can support this milestone -- see this file's header and this
 * package's porting report for what is deliberately left out (combat,
 * goal-replay slow-mo, the rollback laboratory) and why.
 *
 * KNOWN SIMPLIFICATION -- input buffering across ticks. `Match:update`
 * samples `frame_input` once per render call and hands it to
 * `game/match_input_adapter.lua` (`@gc/app`'s `match_input_adapter.ts`,
 * README's `game/` root-file row), which buffers one-shot edges across a
 * zero-tick render update and holds continuous state across a catch-up
 * render update that runs more than one tick. That adapter operates on the
 * LEGACY `MatchInput` shape, not the new wire-format `InputSample`
 * `SimHostPort.step` takes, and no equivalent exists for `InputSample` yet
 * (`@gc/screens` cannot depend on `@gc/app` to reach the legacy one
 * regardless). This class instead samples one `InputSample` per render call
 * and feeds that SAME sample to every tick a catch-up batch runs, and drops
 * it entirely on a zero-tick render call. At a healthy frame rate this is
 * unobservable (one tick per render call, always). Under sustained overload
 * (`MAX_TICKS_PER_UPDATE` catch-up, or a render call fast enough to produce
 * zero ticks) a one-shot edge can fire more than once, or be dropped,
 * instead of firing exactly once on its owning tick. Flagged in this
 * package's porting report as a real behavioral gap, not silently patched
 * over.
 */
export class MatchScreen {
  private readonly ports: MatchScreenPorts;
  private readonly profile: MatchScreenProfile;
  private readonly combatEnabled: boolean;
  /** Set exactly when {@link MatchScreenOptions.rollback_lab} was supplied; mutually exclusive with `host`. */
  private host: SimHostPort | undefined;
  private rollbackHost: RollbackHostPort | undefined;
  private rollbackConsumerPorts: MatchRollbackConsumerPorts | undefined;
  private rollbackConsumer: MatchRollbackConsumerState = newMatchRollbackConsumerState();
  private rollbackOutputs: readonly RollbackLabOutput[] = [];
  private rollbackEventDiffs: readonly RollbackEventDiff[] = [];
  private rollbackConfirmedSteps: readonly RollbackEventStep[] = [];
  private rollbackCorrections: readonly RollbackLabCorrectionSample[] = [];
  private rollbackFrameEvents: MatchEvent[] = [];
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
    this.profile = options.profile ?? "playtest";
    this.combatEnabled = options.combat_enabled ?? false;
    if (options.rollback_lab !== undefined) {
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
      this.rollbackConsumerPorts = { effects: ports.effects, audio: ports.audio, replay: ports.replay };
      this.validateRollbackCombatCompanion();
    } else {
      this.host = ports.createHost();
    }
    this.capture = this.newCapture();
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

  /** Either active host, whichever this screen was constructed with -- mutually exclusive, see `host`/`rollbackHost`'s own docs. */
  private activeHost(): Pick<SimHostPort, "frame" | "roster" | "tick" | "dispose"> {
    const host = this.rollbackHost ?? this.host;
    if (host === undefined) {
      throw new Error("match screen: no active host (unreachable -- constructor always builds one)");
    }
    return host;
  }

  /** `self.state.finished`, read live off the host every call (never cached) -- matches the Lua original always reading current state, not a stale snapshot. */
  get finished(): boolean {
    return this.activeHost().frame().hud.finished;
  }

  /** The current frame's HUD slice, read live off the host -- used by [`MatchScreenAsRealMatchScreen`]'s `state` (score/clock) and by this class's own `finished`/`carrying`. */
  get hud(): RenderFrameHud {
    return this.activeHost().frame().hud;
  }

  /** `self._clock.tick` -- delegated straight to the host, which is this screen's only tick authority. */
  get tick(): number {
    return this.activeHost().tick();
  }

  /** The `InputSample` this screen last sent to `SimHostPort.step`, if any. Exposed for tests, mirroring how the ported spec reads `Match`'s own `self._switch`/`self._pass` fields directly. */
  get debugLastSample(): InputSample | undefined {
    return this.lastStepSample;
  }

  /** Whether a switch is currently buffered awaiting the next `update`. Exposed for tests -- see [`debugLastSample`]. */
  get debugSwitchPending(): boolean {
    return this.switchPending;
  }

  /** Whether this screen was constructed with a `rollback_lab` option -- `self._rollback_lab ~= nil` / `self.state.slot_mode`. */
  get debugRollbackActive(): boolean {
    return this.rollbackHost !== undefined;
  }

  /**
   * `self._opts.combat_enabled == true` -- this screen's own record of the
   * caller's combat opt-in, fixed at construction and never recomputed
   * (mirrors `Match:restart` rebuilding `_combat_state` fresh but always
   * consistent with `_opts.combat_enabled`; see [`restart`]). Exposed for
   * tests, and for [`MatchScreenAsRealMatchScreen`]'s own
   * `debugCombatEnabled`. See {@link MatchScreenOptions.combat_enabled}'s
   * doc for exactly what this option does and does not prove in the BASE
   * (non-rollback) game loop this milestone.
   */
  get debugCombatEnabled(): boolean {
    return this.combatEnabled;
  }

  /** `_rollback_debug` -- `undefined` outside rollback mode. See {@link RollbackLabDebug}'s doc for the host/screen split. */
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

  /** `_clock` -- `undefined` outside rollback mode. */
  debugRollbackClock(): RollbackLabClockDebug | undefined {
    return this.rollbackHost?.clockDebug();
  }

  /** `_rollback_outputs` -- this render call's raw per-tick output records, one per tick actually stepped. Cleared at the top of every `update()` call in rollback mode. */
  get debugRollbackOutputs(): readonly RollbackLabOutput[] {
    return this.rollbackOutputs;
  }

  /** `_rollback_event_diffs`. */
  get debugRollbackEventDiffs(): readonly RollbackEventDiff[] {
    return this.rollbackEventDiffs;
  }

  /** `_rollback_confirmed_steps`. */
  get debugRollbackConfirmedSteps(): readonly RollbackEventStep[] {
    return this.rollbackConfirmedSteps;
  }

  /** `_rollback_corrections` -- this render call's correction sources, retained across every simulated tick (not just the last). */
  get debugRollbackCorrections(): readonly RollbackLabCorrectionSample[] {
    return this.rollbackCorrections;
  }

  /** `_frame_events` -- always empty in rollback mode (the legacy speculative frame-event consumer has nothing to read; see `updateRollback`'s doc). */
  get debugRollbackFrameEvents(): readonly MatchEvent[] {
    return this.rollbackFrameEvents;
  }

  /** `rollback_playable_lab.current_snapshot(self._rollback_lab)`'s combat-companion presence -- `undefined` outside rollback mode. */
  debugRollbackCurrentSnapshot(): RollbackLabSnapshotSummary | undefined {
    return this.rollbackHost?.currentSnapshot();
  }

  /** `rollback_playable_lab.reference_snapshot(self._rollback_lab)`'s combat-companion presence -- `undefined` outside rollback mode. */
  debugRollbackReferenceSnapshot(): RollbackLabSnapshotSummary | undefined {
    return this.rollbackHost?.referenceSnapshot();
  }

  /** The screen-owned rollback-consumption ledger (`_last_scoring_team`/`_kickoff_banner`/...) -- `undefined` outside rollback mode. Exposed for tests; mirrors how the ported spec pokes `Match`'s own `_last_*` fields directly. */
  debugRollbackConsumerState(): MatchRollbackConsumerState | undefined {
    return this.rollbackHost !== undefined ? this.rollbackConsumer : undefined;
  }

  /**
   * `_replay_state` -- the frame currently displayed by the BASE (non-
   * rollback) legacy goal replay, or `undefined` outside an active
   * sequence. Exposed for tests, mirroring the ported spec's own
   * `screen._replay_state` field peek. Rollback-mode confirmed replay
   * shares the same {@link MatchScreenPorts.replay} port but this screen
   * does not itself track a displayed frame for it this milestone -- see
   * `consumeConfirmedLifecycle`'s doc.
   */
  get debugReplayState(): replayTypes.ReplayFrame | undefined {
    return this.replayState;
  }

  /** Whether the injected {@link ReplayPort} currently reports an active goal-replay sequence -- `replay.active()` in the ported original. `false` whenever no `ReplayPort` was supplied. */
  get replayActive(): boolean {
    return this.ports.replay?.active() === true;
  }

  /** Render-only correction-smoothing diagnostics for whichever mode this screen is in (base or rollback). `undefined` until this screen has processed at least one correction/authoritative source. Exposed for tests. */
  get debugRenderSmoothingDiagnostics(): correctionSmoothingTypes.CorrectionSmoothingDiagnostics | undefined {
    return this.renderSmoothing !== undefined ? correctionSmoothing.diagnostics(this.renderSmoothing) : undefined;
  }

  /**
   * Seeds a synthetic previous-frame render correction, for tests only --
   * mirrors the ported spec's `seed_render_correction` helper, which pokes
   * `Match`'s own `_render_smoothing` field directly. A no-op if this
   * screen currently has no correction source (base mode without
   * {@link MatchScreenPorts.matchState}, or rollback mode before the first
   * `step`).
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

  private carrying(): boolean {
    return this.activeHost().frame().hud.controlled_owns_ball;
  }

  /** `self:restart()` -- dispose the current host and build a fresh one from the same factory, resetting every frame-to-frame latch and (in rollback mode) every rollback/presentation-owned field. */
  private restart(): void {
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
    this.replayState = undefined;
    this.renderSmoothing = undefined;
    viewState.reset();
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
   * `Match:event`. Discrete key/gamepad/action events -- the rematch keys
   * once the match is over, and (mid-match) the `pass_switch` edge plus
   * whatever `@gc/input`'s `InputSampleCapture` needs queued for its own
   * `dodge` detection.
   */
  event(evt: ControllerInputEvent): void {
    // A confirmed goal replay (base or rollback) remains skippable
    // regardless of match/finished state -- mirrors `Match:event`'s own
    // `replay.active()` check, which runs before both the ACTION-kind and
    // KEY-kind bodies check anything else, including `match_is_over`.
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

  // `Match:event`'s `replay.active()` skip branches: ACTION-kind allows
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

  // `finish_replay(self)`: stop the sequence, clear the displayed replay
  // frame, and settle render-owned smoothing/view state back onto whatever
  // this screen is currently authoritative for.
  private finishReplay(): void {
    this.ports.replay?.stop?.();
    this.replayState = undefined;
    const source = this.correctionSource();
    // `clear_render_smoothing`'s own unconditional `view_state.reset()`
    // (`reset_view` defaults true) -- discards the replay's motion before
    // reseeding baselines below, so the render-live players' gait/lean read
    // back at rest rather than carrying over the replay's last pose.
    viewState.reset();
    if (source === undefined) {
      return;
    }
    this.renderSmoothing =
      this.renderSmoothing !== undefined
        ? correctionSmoothing.clear(this.renderSmoothing, source)
        : correctionSmoothing.new(source);
    viewState.update(source.players, 0);
  }

  // `Match:event`'s `match_is_over(self)` branch:
  //   if self._profile == "playtest" and evt.action == "confirm" then ... -- ACTION-kind, no `pressed` check (reproduced faithfully, not "fixed" -- see this package's porting rules).
  //   if self._profile == "playtest" and (evt.key == "r" or control_id == "confirm") then ... -- KEY-kind. `control_id` is `bindings.control_for_key`,
  //     which resolves "space" to the `action` control (id "action"), NOT "confirm" -- only literal `return`/`kpenter` (the `confirm` control's own
  //     keys) satisfy `control_id == "confirm"` on this path, even though `action`'s own ActionName is also "confirm".
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

  // `Match:event`'s `pass_switch` branches (both the ACTION-kind and
  // KEY-kind bodies reduce to the same rule): off the ball, PLAY's press
  // buffers a switch. Read off the discrete event stream via
  // `controller.normalize` -- the same primitive `capture_frame.ts`'s own
  // `consumeDodgeEdge` uses for `juke` -- rather than a per-frame poll,
  // because that is what the Lua original does (`Match:event`, not
  // `Match:update`'s `control_down` polling).
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
   * `Match:update(dt)`. Samples this frame's input once and drives the
   * fixed clock (`SimHostPort.planTicks`/`step`/`cancelPlannedTicks` -- see
   * this section's header; the accumulator algorithm itself lives only in
   * Rust now). A no-op once the match is finished -- `Match:update`'s own
   * `match_is_over(self)` early return, minus the rollback-replay/
   * onboarding bookkeeping this milestone does not implement.
   *
   * Drawing is NOT done here -- see [`MatchScreen.draw`]: this class follows
   * the model/view seam every other screen in this codebase uses
   * (AGENTS.md §9), `update(dt)` steps the simulation and `draw()` is the
   * only impure rendering call, called separately by whoever owns the
   * render loop.
   */
  update(dt: number): void {
    if (this.rollbackHost !== undefined) {
      this.updateRollback(dt);
      return;
    }
    // Slow-motion goal replay: the sim freezes while the buffer plays back.
    // Checked before `finished` -- matches `Match:update`'s own ordering
    // (`replay.active()` is checked first; a replay may still be finishing
    // up on the same render call the match itself reads as over).
    if (this.ports.replay?.step !== undefined && this.ports.replay.active()) {
      this.updateLegacyReplay(dt);
      return;
    }
    if (this.finished) {
      return;
    }
    const host = this.host!;
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

    const scoreBefore = host.frame().hud.home_score + host.frame().hud.away_score;
    const ticks = host.planTicks(dt);
    for (let step = 0; step < ticks; step += 1) {
      // KNOWN SIMPLIFICATION: every tick this render call produces is fed
      // the SAME sample -- see this class's doc comment.
      this.recordReplayFrame(); // pre-step, so a goal's flight remains in the buffer.
      host.step(sample);
      // `fixed_clock.advance`'s `step` callback returns `not
      // self.state.finished and not scored` to stop a catch-up batch as
      // soon as the match ends or a goal starts a replay. Telling the host
      // to cancel the remaining planned ticks is what makes that match
      // what a `false` step_fn return does to the accumulator on the Rust
      // side (see `SimHostPort.cancelPlannedTicks`'s doc). Falls through to
      // the shared post-batch bookkeeping below either way -- the Lua
      // original's own goal/lifecycle detection runs unconditionally after
      // its `fixed_clock.advance` call, not only on a full batch.
      if (host.frame().hud.finished) {
        host.cancelPlannedTicks();
        break;
      }
    }
    this.finishBaseUpdate(dt, scoreBefore);
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

  // The tail of `Match:update`'s base branch: render-owned smoothing/view
  // state, then the live goal edge that starts a fresh replay sequence
  // (`self._last_score`/`self._last_home` in the Lua original).
  private finishBaseUpdate(dt: number, scoreBefore: number): void {
    const host = this.host!;
    const hud = host.frame().hud;
    const scoreAfter = hud.home_score + hud.away_score;
    this.updateBaseRenderSmoothing(dt, scoreAfter !== scoreBefore || hud.finished);
    if (scoreAfter > this.lastScore && !hud.finished) {
      const scoringTeam: "home" | "away" = hud.home_score > this.lastHome ? "home" : "away";
      if (this.ports.replay?.start?.(scoringTeam) === true) {
        // The scene jumps back in time, but view state must not carry the
        // post-goal kickoff pose into it.
        viewState.reset();
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
    } else {
      this.renderSmoothing =
        this.renderSmoothing !== undefined
          ? correctionSmoothing.advance(this.renderSmoothing, source, dt)
          : correctionSmoothing.new(source);
    }
    // Unconditional relative to the branch above -- matches
    // `update_render_smoothing`'s own "if update_view then view_state.update(...)
    // end", which reseeds immediately even on a lifecycle reset rather than
    // waiting for the next render call.
    viewState.update(source.players, dt);
  }

  // `Match:update`'s legacy-replay branch: the sim is frozen (no `host.step`
  // this call) while the buffered footage plays back through view state.
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
   * `Match:update(dt)`'s rollback branch. Unlike the base branch above,
   * this one does NOT sample input unconditionally: a zero-tick render call
   * touches neither `this.latches` nor `this.switchPending`/`capture`, so a
   * one-shot edge queued just before a zero-tick call survives to the next
   * render call that actually steps -- see `match_rollback_lab.spec.ts`'s
   * "retains a zero-tick edge and consumes it exactly once" (the base
   * branch's own KNOWN SIMPLIFICATION deliberately does the opposite; the
   * rollback branch is held to the Lua original's real `_input_adapter`
   * behavior instead).
   *
   * `_rollback_outputs`/`_rollback_event_diffs`/`_rollback_confirmed_steps`/
   * `_rollback_corrections`/`_frame_events` are this render call's batch
   * ONLY -- cleared up front, never accumulated across calls.
   */
  private updateRollback(dt: number): void {
    const rollbackHost = this.rollbackHost!;
    this.rollbackOutputs = [];
    this.rollbackEventDiffs = [];
    this.rollbackConfirmedSteps = [];
    this.rollbackCorrections = [];
    this.rollbackFrameEvents = [];

    if (this.finished) {
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
    // consumption) is NOT gated on ticks having run this call -- matches
    // `Match:update`'s own `update_render_smoothing`/`consume_rollback_presentation`
    // calls, which sit OUTSIDE (after) `fixed_clock.advance`'s tick loop and
    // therefore always run, whether or not that loop produced any ticks
    // this render call. Skipping them at `ticks === 0` used to mean a
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

    for (const correction of corrections) {
      this.renderSmoothing =
        this.renderSmoothing === undefined
          ? correctionSmoothing.correct(correctionSmoothing.new(correction.source), correction.source)
          : correctionSmoothing.correct(this.renderSmoothing, correction.source);
    }
    const positions = rollbackHost.displayedPositions();
    if (this.renderSmoothing !== undefined && dt > 0) {
      this.renderSmoothing = correctionSmoothing.advance(this.renderSmoothing, positions, dt);
    }

    // Live view state (gait/lean), from the displayed rollback client.
    viewState.update(positions.players, dt);

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
    const host = this.activeHost();
    const frame = host.frame();
    const roster = host.roster();
    this.ports.renderer.draw(frame, roster);
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
// not depend on each other (`@gc/online`'s `match_presentation.ts`'s ported
// types), not a redesign of either existing contract -- `real_match.ts` is
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
function toControllerInputEvent(evt: ControllerInputEvent | RealMatchInputEvent): ControllerInputEvent {
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

  /** `real_match.ts`'s `RealMatchState` -- read live off the screen's current frame, mirroring `MatchScreen.finished`'s own "never cached" rule. */
  get state(): RealMatchState {
    const hud = this.screen.hud;
    return { time_left: hud.time_left, score: { home: hud.home_score, away: hud.away_score } };
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
   * Not yet wired: this milestone's `MatchScreen` does not itself call
   * `consumeRollbackPresentation` (the rollback-consumption seam this
   * file's header describes) to populate a per-frame match-event batch, so
   * there is nothing to report here yet. Honestly empty, not a lie -- see
   * this file's own scope note on what is/is not wired this milestone.
   */
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

  /**
   * Settings (audio/graphics volume, ...) are not wired into `MatchScreen`
   * this milestone -- no ported module owns them yet (this file's header).
   * A no-op, not a lie: nothing in this milestone's `MatchScreen` reads a
   * setting, so there is nothing to apply.
   */
  applySettings(_settings: GameSettings): void {}
}
