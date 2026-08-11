// The character animator: `PlayerPoseId` in, one posed rig out.
//
// This is the playback layer #425 asks for. It composes three
// `rig3d/mixer.ts` layers and hands the result to the same
// `rig3d/action_pose.ts` root overlay `player_renderer_3d.ts` has always
// applied last:
//
//   1. LOCOMOTION BASE -- idle/walk/run, blended by speed
//      (`pose_table.locomotionBlend`), each pinned to `view_state.ts`'s
//      distance-derived `gait` rather than to a clock.
//   2. STANCE -- the one action `POSE_ACTIONS` maps the pose id to, crossfaded
//      in and out over that entry's own duration, masked to the bones the
//      entry says it owns.
//   3. POSSESSION -- the keeper's hands, driven off `holding`/`throw` rather
//      than the pose id, and outranking the stance layer exactly as
//      `poseFor` always had it: arms wrapped around a ball beat whatever else
//      the keeper is doing.
//
// Then a fourth layer that is NOT a mixer layer, and then `action_pose.apply`
// for the whole-body root overlay:
//
//   4. CROUCH -- `rig3d/crouch.ts`'s knee bend, sized by the drop
//      `action_pose.ATTITUDES` authors for this pose id (#445). It is composed
//      ADDITIVELY rather than layered through a mask, because a crouch is a
//      modifier on a body that is still running: an override would have to
//      restate the stride to keep it. See the block in `poseFor` for why it is
//      driven off the pose id rather than through `POSE_ACTIONS`.
//
// LEAN IS NOT HERE, deliberately. `view.lean` becomes a torso tilt in
// `player_renderer_3d.ts` (#428's `leanTilt`/`applyLean`), applied to the pose
// this function returns. That is the right home for it and not merely where
// it happened to land: the magnitude is derived from `ppmForRadius` and
// `HEIGHT_IN_RADII`, both of which are that file's, so putting the tilt here
// would mean rig3d reaching upward for a screen-space calibration. Keeping
// one implementation there also means the procedural and mixer paths lean
// identically by construction rather than by two tunings agreeing.
//
// HONEST ABOUT LAYER 3: it has exactly one action active at a time and never
// blends, so it gets none of the N-way accumulation that is the reason layers
// 1 and 2 are mixers at all. It is the mixer apparatus wrapped around what is
// effectively a single `clips.sample`, kept that way for API uniformity -- one
// `MixerLayer` type, one `set`/`evaluate` contract, one place the baked-clip
// and phase-precondition rules live. Worth stating rather than leaving a
// reader to infer that all three layers earn their machinery equally. If the
// possession rules stay a two-way `if` forever, collapsing this layer back to
// a direct sample would lose nothing but the uniformity.
//
// ONE SET OF LAYERS FOR EVERY CHARACTER. `MixerLayer` is stateless between
// evaluations (see its doc comment), so all 22 players share the three layers
// the same way `player_renderer_3d.ts`'s `characterMesh` already shares one
// pose-evaluation `rig` -- mutated and immediately consumed per player, safe
// because JS is single-threaded. The only genuinely per-character state is the
// crossfade, which is a handful of numbers in `states` below.
//
// LAYER-BOUNDARY NOTE (AGENTS.md §2). Everything here is pure derivation from
// (view state, render options, time): no `love`, no WebGL, no canvas, no
// simulation mutation. The `THREE.*` types it touches are `Object3D`
// property bags and the mixer's own scheduler, none of which need a GL
// context -- which is why `animator.spec.ts` runs the whole thing headless.
// Options are declared structurally here rather than imported from
// `../player_render_options.ts`, matching `action_pose.ts`'s own boundary note, so
// rig3d keeps pointing only upward.

// (no @gc/core import: the animator composes poses through `clips.layer` and
// `action_pose`, and no longer builds quaternions of its own -- see the header
// note on where the torso lean lives.)
import * as clips from "./clips.ts";
import * as crouch from "./crouch.ts";
import * as masks from "./masks.ts";
import * as actionPose from "./action_pose.ts";
import { MixerLayer, type LayerClip } from "./mixer.ts";
import * as poseTable from "./pose_table.ts";
import type { PhaseSource, StanceActionId } from "./pose_table.ts";

/** The `view_state.ts` fields the animator reads. */
export interface AnimatorView {
  /** Smoothed world-units/sec. */
  readonly speed: number;
  /** Normalised gait cycle position in [0, 1). */
  readonly gait: number;
  // `PlayerView.lean` is deliberately absent: the torso tilt is
  // `player_renderer_3d.ts`'s (see this file's header). A `PlayerView` still
  // satisfies this interface -- it just carries a field the animator ignores.
}

/** The `PlayerRenderOptions` fields the animator reads, on top of the root overlay's. */
export interface AnimatorOptions extends actionPose.ActionPoseOptions {
  /** Keeper is carrying the ball. */
  readonly holding?: boolean;
  /** Keeper throw timer, counting DOWN: 1 is the moment of commitment. */
  readonly throw?: number;
  /** Soccer/combat windup timer; > 0 means the action is still coiling. */
  readonly windup?: number;
}

// Playback rate for the idle clip, in clip-seconds per real second. Breathing
// and a weight shift have no ground-speed meaning, so idle is the one action
// here that runs off the wall clock rather than off distance travelled.
const IDLE_RATE = 0.35;

// How long a stance takes to fade OUT when the pose it belonged to is replaced
// by one with no action of its own (a dive, a stumble, plain locomotion).
// Those entries have no crossfade of their own -- there is nothing to fade IN
// -- so the outgoing action keeps the duration it was engaged with, and this
// is only the fallback for a character whose first observed frame is already
// a fade-out.
const DEFAULT_FADE_SECONDS = 0.16;

// Longest gap between two frames the crossfade will integrate. A tab that was
// backgrounded for ten seconds must not snap every stance in one step, and a
// negative delta (a caller passing a non-monotonic `now`) must not run the
// ramp backwards.
//
// 0.1s is a 10fps frame: below the clamp the crossfade is exact, above it the
// animation eases rather than cutting. It is deliberately shorter than the
// longest crossfade in `POSE_ACTIONS` (0.24s), or the clamp would let a single
// long frame complete any fade and would not be a clamp at all.
const MAX_FRAME_DT = 0.1;

// How long the DEEPEST authored crouch takes to settle in or out (#445).
//
// A crouch is a held posture, not a committed action, so it eases like a stance
// rather than snapping like a throw -- 0.16 s is `pose_table.ts`'s own EASE,
// the duration it gives a held stance. It is a rate rather than a duration for
// each pose, so a shallow crouch reaches its depth proportionally sooner
// instead of every pose taking the same time to travel a different distance;
// same linear-ramp argument `advanceCrossfade` makes for stance weights.
//
// NOT read from `POSE_ACTIONS[id].crossfade`, which would look tidier and would
// snap: three of the five crouching poses (`settle`, `contain`, `fatigue`) have
// no stance action at all, so their entry's crossfade is 0 by construction.
const CROUCH_FADE_SECONDS = 0.16;
const CROUCH_RATE = actionPose.DEEPEST_ATTITUDE_DROP / CROUCH_FADE_SECONDS;

const BASE_CLIPS: readonly LayerClip[] = [
  // Construction order is playback order is accumulation order: three.js
  // accumulates in `AnimationMixer._actions` order, which is the order actions
  // were first played. idle -> walk -> run is the order `poseFor`'s nested
  // `clips.layer` calls composed them in, and quaternion blending is not
  // commutative, so this list is load-bearing.
  { id: "idle", clip: clips.IDLE },
  { id: "walk", clip: clips.WALK },
  { id: "run", clip: clips.RUN },
];

const STANCE_CLIPS: readonly LayerClip[] = [
  { id: "guard_stance", clip: clips.GUARD_STANCE },
  { id: "charge", clip: clips.CHARGE },
  { id: "swing", clip: clips.SWING, loop: "clamp" },
  { id: "keeper_gather", clip: clips.KEEPER_GATHER },
  { id: "keeper_sling", clip: clips.KEEPER_SLING, loop: "clamp" },
];

const POSSESSION_CLIPS: readonly LayerClip[] = [
  { id: "keeper_gather", clip: clips.KEEPER_GATHER },
  { id: "keeper_sling", clip: clips.KEEPER_SLING, loop: "clamp" },
];

const CLIP_BY_ACTION: Readonly<Record<StanceActionId, clips.Clip>> = {
  guard_stance: clips.GUARD_STANCE,
  charge: clips.CHARGE,
  swing: clips.SWING,
  keeper_gather: clips.KEEPER_GATHER,
  keeper_sling: clips.KEEPER_SLING,
};

interface Layers {
  readonly base: MixerLayer;
  readonly stance: MixerLayer;
  readonly possession: MixerLayer;
}

let layers: Layers | undefined;

// Baking eight clips at `BAKE_FPS` is a few hundred microseconds of work and a
// few tens of kilobytes; doing it lazily keeps it off the import path of every
// consumer of `@gc/render` that never draws a character.
function ensureLayers(): Layers {
  if (layers === undefined) {
    layers = {
      base: new MixerLayer(BASE_CLIPS),
      stance: new MixerLayer(STANCE_CLIPS),
      possession: new MixerLayer(POSSESSION_CLIPS),
    };
  }
  return layers;
}

interface AnimatorState {
  crossfade: poseTable.CrossfadeState;
  /** The mask each in-flight action was engaged with. See `activeMask`. */
  masksByAction: Record<string, ReadonlySet<string>>;
  /** The crossfade duration of the last engaged action, used for its fade-out. */
  fadeSeconds: number;
  /** How deep this character is currently crouched, in `move` units (#445). */
  crouch: number;
  lastNow: number;
}

const states = new Map<string, AnimatorState>();

/**
 * Drops every character's crossfade state.
 *
 * Called wherever `viewState.reset()` is -- the scene-discontinuity points in
 * `screens/match.ts` and `benchmark.ts`'s constructor -- via
 * `player_renderer_3d.resetAnimation`, which carries the reasoning for why
 * those are exactly the right sites.
 */
export function reset(): void {
  states.clear();
}

function clamp(x: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, x));
}

// Steps `current` toward `goal` by at most `step`, never overshooting. The
// scalar twin of `pose_table.advanceCrossfade`'s own inner helper, which is not
// exported because a crossfade's ramp is that module's business; this one
// carries the crouch depth, which is metres rather than a weight.
function moveToward(current: number, goal: number, step: number): number {
  return current < goal ? Math.min(goal, current + step) : Math.max(goal, current - step);
}

// Playback phase, wrapped into the clip for a looping action and clamped into
// it for a one-shot. Necessary because `MixerLayer.evaluate` advances the
// mixer by zero and therefore never applies three.js's own loop handling: an
// unwrapped `now * IDLE_RATE` would walk off the end of the idle clip's
// keyframes after nine seconds and freeze on its last key.
//
// This is the caller's half of a contract `MixerLayer.set` ENFORCES -- it
// throws on a phase outside `[0, duration]` (see its doc comment for the
// three.js early-return that makes the check load-bearing). Only this side
// knows whether a given phase source loops, which is why the wrap lives here
// and the check lives there.
function wrapPhase(time: number, duration: number, loop: boolean): number {
  if (!loop) {
    return clamp(time, 0, duration);
  }
  const wrapped = time % duration;
  return wrapped < 0 ? wrapped + duration : wrapped;
}

function phaseFor(
  source: PhaseSource,
  clip: clips.Clip,
  view: AnimatorView | undefined,
  opts: AnimatorOptions,
  now: number,
): number {
  switch (source) {
    case "clock":
      return wrapPhase(now, clip.duration, true);
    case "gait":
      // The charge is a held pose, so it only needs a phase to breathe on;
      // tying it to the stride keeps the sway in step with the legs.
      return wrapPhase((view?.gait ?? 0) * clip.duration, clip.duration, true);
    case "throw_timer":
      // throw_timer counts DOWN, so 1 is the moment of commitment.
      return wrapPhase((1 - clamp(opts.throw ?? 0, 0, 1)) * clip.duration, clip.duration, false);
    case "windup":
      // The two phases of a strike worth holding: the coiled key while the
      // windup timer runs, the contact key once it has expired. Both fractions
      // are `poseFor`'s own, from the `"swing"` branch that no `POSE_CLIP`
      // entry ever reached and which this rewrite deletes.
      return wrapPhase(((opts.windup ?? 0) > 0 ? 0.3 : 0.55) * clip.duration, clip.duration, false);
  }
}

// The bones the stance layer is allowed to drive this frame: the union of the
// masks every in-flight action was engaged with. Almost always one set (every
// entry but `keeper_punt` masks to `UPPER_BODY`, and `ARMS` is a subset of it),
// so the common path returns the recorded set itself rather than a copy.
function activeMask(
  state: AnimatorState,
  weights: Readonly<Record<string, number>>,
): ReadonlySet<string> {
  const active: ReadonlySet<string>[] = [];
  for (const id of Object.keys(weights)) {
    const mask = state.masksByAction[id];
    if (mask !== undefined) {
      active.push(mask);
    }
  }
  const first = active[0];
  if (first === undefined) {
    return masks.UPPER_BODY;
  }
  if (active.length === 1) {
    return first;
  }
  const merged = new Set<string>(first);
  for (let i = 1; i < active.length; i += 1) {
    for (const bone of active[i] ?? []) {
      merged.add(bone);
    }
  }
  return merged;
}

/**
 * The locomotion base layer, evaluated on its own. Exported so
 * `player_renderer_3d.spec.ts` can pin it against the procedural sampler's own
 * base blend without the stance and possession layers in the way.
 */
export function basePose(view: AnimatorView | undefined, now: number): actionPose.MutablePose {
  const { base } = ensureLayers();
  const speed = view?.speed ?? 0;
  const gait = view?.gait ?? 0;
  const blend = poseTable.locomotionBlend(speed);
  base.silence();
  base.set("idle", wrapPhase(now * IDLE_RATE, clips.IDLE.duration, true), blend.idle, IDLE_RATE);
  base.set(
    "walk",
    wrapPhase(gait * clips.WALK.duration, clips.WALK.duration, true),
    blend.walk,
    poseTable.locomotionTimeScale(speed, clips.WALK.duration),
  );
  base.set(
    "run",
    wrapPhase(gait * clips.RUN.duration, clips.RUN.duration, true),
    blend.run,
    poseTable.locomotionTimeScale(speed, clips.RUN.duration),
  );
  return base.evaluate();
}

/**
 * Resolves the pose for one character.
 *
 * `playerId` keys the crossfade state; `now` is seconds (the same
 * `performance.now() / 1000` `pitch.draw` threads through), and the frame
 * delta the crossfade integrates is derived from it rather than taken as a
 * parameter, so this stays a drop-in for `poseFor`'s own signature.
 */
export function poseFor(
  playerId: string,
  view: AnimatorView | undefined,
  opts: AnimatorOptions,
  now: number,
): actionPose.MutablePose {
  const { stance, possession } = ensureLayers();
  let pose = basePose(view, now);

  // --- Stance layer -------------------------------------------------------
  const entry = poseTable.actionForPose(opts.pose?.id);
  const target = entry.action;
  // The crouch this pose id authors, which the crouch layer below eases toward.
  const crouchTarget = actionPose.attitudeDrop(opts);
  const previous = states.get(playerId);
  let state: AnimatorState;
  if (previous === undefined) {
    // A character's first observed frame commits to its stance outright.
    // Fading in from nothing would make every player on the pitch ease into
    // their stance at kickoff, and after a rollback re-seed, for no reason.
    // The crouch commits on the same frame and for the same reason.
    state = {
      crossfade: poseTable.snapCrossfade(target),
      masksByAction: {},
      fadeSeconds: target !== null ? entry.crossfade : DEFAULT_FADE_SECONDS,
      crouch: crouchTarget,
      lastNow: now,
    };
  } else {
    const dt = clamp(now - previous.lastNow, 0, MAX_FRAME_DT);
    const fadeSeconds = target !== null ? entry.crossfade : previous.fadeSeconds;
    state = {
      crossfade: poseTable.advanceCrossfade(previous.crossfade, target, dt, fadeSeconds),
      masksByAction: previous.masksByAction,
      fadeSeconds,
      crouch: moveToward(previous.crouch, crouchTarget, dt * CROUCH_RATE),
      lastNow: now,
    };
  }
  if (target !== null && entry.mask !== null) {
    state.masksByAction[target] = entry.mask;
  }
  // Forget masks for actions that have finished fading out, so the record
  // cannot outlive the crossfade it belongs to.
  for (const id of Object.keys(state.masksByAction)) {
    if (state.crossfade.weights[id] === undefined) {
      delete state.masksByAction[id];
    }
  }
  states.set(playerId, state);

  const total = poseTable.crossfadeTotal(state.crossfade);
  if (total > 0) {
    stance.silence();
    for (const [id, weight] of Object.entries(state.crossfade.weights)) {
      const actionId = id as StanceActionId;
      const clip = CLIP_BY_ACTION[actionId] as clips.Clip | undefined;
      if (clip === undefined) {
        continue;
      }
      // Normalised so the layer's own actions blend AMONG themselves at full
      // strength; `total` is what decides how much of the result reaches the
      // pose. Without the normalisation a half-faded stance would blend
      // halfway toward rest inside the mixer AND halfway toward the base
      // outside it, and land at a quarter strength.
      stance.set(
        actionId,
        phaseFor(poseTable.PHASE_BY_ACTION[actionId], clip, view, opts, now),
        weight / total,
      );
    }
    pose = clips.layer(pose, stance.evaluate(), activeMask(state, state.crossfade.weights), total);
  }

  // --- Possession layer ---------------------------------------------------
  // The keeper's hands follow possession, not the pose id: arms wrapped
  // around a ball outrank whatever else the keeper is doing.
  const throwAmount = opts.throw ?? 0;
  if (throwAmount > 0) {
    possession.silence();
    possession.set("keeper_sling", phaseFor("throw_timer", clips.KEEPER_SLING, view, opts, now), 1);
    pose = clips.layer(pose, possession.evaluate(), masks.UPPER_BODY, 1);
  } else if (opts.holding === true) {
    possession.silence();
    possession.set("keeper_gather", phaseFor("clock", clips.KEEPER_GATHER, view, opts, now), 1);
    pose = clips.layer(pose, possession.evaluate(), masks.UPPER_BODY, 1);
  }

  // --- Crouch layer (#445) ------------------------------------------------
  // The knee bend under a settling body. Driven off the POSE ID rather than
  // through `POSE_ACTIONS`, which is what lets it run alongside a stance
  // instead of competing for the one action slot: `keeper_set` and
  // `keeper_ready_low` have already spent theirs on `guard_stance`'s braced
  // arms, and those arms and this crouch are the two halves of one reading.
  //
  // COMPOSED, NOT LAYERED, and that is the reason it is not a fourth
  // `MixerLayer`. `clips.layer`'s override semantics would have to own every
  // leg bone to write any of them, which deletes the stride -- and the stride
  // is exactly what `pose_table.ts` says these poses keep ("a defender
  // containing mid-stride keeps the stride"). `clips.compose` adds the fold to
  // whatever the gait resolved instead. See its own note for why it takes no
  // weight, and `crouch.ts` for why the fold and the drop cancel exactly.
  const crouchPose = crouch.poseFor(state.crouch);
  if (crouchPose !== null) {
    pose = clips.compose(pose, crouchPose);
  }

  // Whole-body actions last: they move the root, so they ride on top of
  // whatever gait and stance resolved instead of competing with them.
  return actionPose.apply(pose, opts);
}
