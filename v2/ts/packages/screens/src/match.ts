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

export interface MatchScreenOptions {
  /** Mirrors `Match.new`'s `opts.profile`; defaults to `"playtest"`, matching the Lua original's `self._opts.profile or "playtest"`. */
  readonly profile?: MatchScreenProfile;
}

export interface MatchScreenPorts {
  readonly createHost: SimHostFactory;
  readonly renderer: RenderPort;
  readonly keyboard: KeyboardState;
  readonly gamepad?: GamepadState;
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
  private host: SimHostPort;
  private readonly latches: MatchControlLatches = newMatchControlLatches();
  private switchPending = false;
  private capture: captureFrame.InputSampleCapture;
  private pendingKeyEvents: ControllerInputEvent[] = [];
  private pendingGamepadEvents: ControllerInputEvent[] = [];
  private lastStepSample: InputSample | undefined;

  constructor(ports: MatchScreenPorts, options: MatchScreenOptions = {}) {
    this.ports = ports;
    this.profile = options.profile ?? "playtest";
    this.host = ports.createHost();
    this.capture = this.newCapture();
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

  /** `self.state.finished`, read live off the host every call (never cached) -- matches the Lua original always reading current state, not a stale snapshot. */
  get finished(): boolean {
    return this.host.frame().hud.finished;
  }

  /** The current frame's HUD slice, read live off the host -- used by [`MatchScreenAsRealMatchScreen`]'s `state` (score/clock) and by this class's own `finished`/`carrying`. */
  get hud(): RenderFrameHud {
    return this.host.frame().hud;
  }

  /** `self._clock.tick` -- delegated straight to the host, which is this screen's only tick authority. */
  get tick(): number {
    return this.host.tick();
  }

  /** The `InputSample` this screen last sent to `SimHostPort.step`, if any. Exposed for tests, mirroring how the ported spec reads `Match`'s own `self._switch`/`self._pass` fields directly. */
  get debugLastSample(): InputSample | undefined {
    return this.lastStepSample;
  }

  /** Whether a switch is currently buffered awaiting the next `update`. Exposed for tests -- see [`debugLastSample`]. */
  get debugSwitchPending(): boolean {
    return this.switchPending;
  }

  private carrying(): boolean {
    return this.host.frame().hud.controlled_owns_ball;
  }

  /** `self:restart()` -- dispose the current host and build a fresh one from the same factory, resetting every frame-to-frame latch. */
  private restart(): void {
    this.host.dispose();
    this.host = this.ports.createHost();
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
    if (this.finished) {
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

    const ticks = this.host.planTicks(dt);
    for (let step = 0; step < ticks; step += 1) {
      // KNOWN SIMPLIFICATION: every tick this render call produces is fed
      // the SAME sample -- see this class's doc comment.
      this.host.step(sample);
      // `fixed_clock.advance`'s `step` callback returns `not
      // self.state.finished and not scored` to stop a catch-up batch as
      // soon as the match ends or a goal starts a replay. This milestone
      // has no replay, so only the "finished" half applies -- and telling
      // the host to cancel the remaining planned ticks is what makes that
      // match what a `false` step_fn return does to the accumulator on the
      // Rust side (see `SimHostPort.cancelPlannedTicks`'s doc).
      if (this.host.frame().hud.finished) {
        this.host.cancelPlannedTicks();
        return;
      }
    }
  }

  /**
   * Draw the current frame. Separated from `update` (see that method's
   * doc) so callers control the render cadence independently of the
   * simulation cadence, and so this class satisfies
   * [`RealMatchScreenPort.draw`] (`real_match.ts`) via
   * [`MatchScreenAsRealMatchScreen`].
   */
  draw(): void {
    const frame = this.host.frame();
    const roster = this.host.roster();
    this.ports.renderer.draw(frame, roster);
  }

  /** Release the underlying host's resources. Safe to call once; does not itself call `SimHostPort.dispose` twice, matching that contract's own "safe to call twice" note being unnecessary to rely on here. */
  dispose(): void {
    this.host.dispose();
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
