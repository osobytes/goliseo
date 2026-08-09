// The pose -> animation-action table, and the pure blend maths that drives it.
//
// WHAT THIS REPLACES. `player_renderer_3d.ts`'s `POSE_CLIP` mapped 12 of the
// 32 `PlayerPoseId`s the wire format can carry (`frame_buffer.ts`'s own
// `PlayerPoseId` union, mirroring `gc-render/src/player_pose.rs`) onto a clip
// name, and returned `"idle"` for everything else. That fallback was silent
// and, worse, ambiguous: `"idle"` covered THREE different situations that
// happen to look the same from inside `poseFor` and are completely different
// facts about the game --
//
//   * poses another mechanism genuinely animates (a keeper's dive is a
//     `rig3d/action_pose.ts` root transform, not a limb clip),
//   * poses that are SUPPOSED to look like ordinary locomotion
//     (`keeper_shuffle` -- a keeper shuffling along the line is walking), and
//   * poses with a distinct intended reading that nothing renders, so they
//     show as a plain jog (`slide`, `contain`, `fatigue`).
//
// A reader could not tell those apart, so the third kind was invisible. Every
// one of the 32 ids now has an EXPLICIT entry here, and the three cases above
// are three distinct values of `fallback` rather than one shared `"idle"`.
// `POSE_ACTIONS` is typed `Record<PlayerPoseId, PoseActionEntry>`, so adding a
// pose id to the wire format without deciding how it animates is a type error
// rather than a silent idle.
//
// THIS TABLE IS NOW THE WHOLE COVERAGE STORY, not half of it. #418 deleted the
// procedural 2D billboard, which had its own pose-id vocabulary the rigged
// path never shared (see `PoseFallback`'s note). There is no second renderer
// left to pick up what this one drops.
//
// WHY THE TABLE IS DATA AND NOT CODE (AGENTS.md §8). Which clip plays for
// which pose, how long it crossfades, where its playback phase comes from --
// all of that is content. The mechanism that plays it is
// `rig3d/mixer.ts`/`rig3d/animator.ts`. Nothing in this file imports three.js
// or touches a renderer, so the whole table and every blend function below
// runs in the headless test runner (AGENTS.md §9 tier 1).
//
// PHASE, NOT `timeScale`, IS THE AUTHORITY -- see `locomotionTimeScale`'s doc
// comment for the full argument. Every action this table describes is pinned
// to a simulation-derived quantity (distance travelled, a throw timer, the
// windup timer) rather than to a free-running animation clock, for the same
// reason `rig3d/action_pose.ts` keeps dives parametric: a free-running clock
// does not move when rollback moves the simulation under it.

import * as masks from "./masks.ts";
import type { PlayerPoseId } from "../frame_buffer.ts";

/**
 * The stance/action clips `rig3d/clips.ts` exports today, by id. These are the
 * ids `rig3d/animator.ts` builds `THREE.AnimationAction`s from; the names match
 * the `Clip.name` fields so a reader can grep one and land on the other.
 *
 * Deliberately NOT including `idle`/`walk`/`run`: those three are the
 * locomotion base layer every pose sits on, blended by speed rather than
 * selected by pose id (see `locomotionBlend`).
 */
export type StanceActionId = "guard_stance" | "charge" | "swing" | "keeper_gather" | "keeper_sling";

/**
 * Where an action's playback phase comes from. Every one of these is a
 * simulation-derived quantity except `clock`, which is reserved for idling
 * motion that has no simulation meaning at all (breathing, a stance's sway).
 *
 * * `clock` -- wall-clock seconds (`now`), the only free-running source.
 * * `gait` -- `view_state.ts`'s distance-derived `gait` in [0, 1), scaled to
 *   the clip's duration. Locked to ground speed by construction.
 * * `throw_timer` -- `PlayerRenderOptions.throw`, which counts DOWN, so 1 is
 *   the moment of commitment and 0 is the end of the follow-through.
 * * `windup` -- `PlayerRenderOptions.windup`: held at the clip's coiled key
 *   while a windup timer runs, and at its strike key once it has expired.
 */
export type PhaseSource = "clock" | "gait" | "throw_timer" | "windup";

/**
 * Why a pose has no action of its own. The whole point of the rewrite: these
 * are three different facts, and `POSE_CLIP`'s `"idle"` conflated them.
 *
 * * `root_overlay` -- `rig3d/action_pose.ts` animates this pose as a
 *   parameterised transform of the rig root, driven by the simulation's own
 *   timer. Deliberately not a clip; see that file's header. The limbs keep
 *   whatever the locomotion blend resolved, which is correct -- a keeper
 *   diving mid-stride keeps the stride.
 * * `locomotion_base` -- this pose is SUPPOSED to look like ordinary
 *   locomotion, and has no reading of its own that is missing. A keeper
 *   shuffling along the line is walking; that is the whole pose.
 * * `unanimated` -- a real gap. This pose HAS a distinct intended reading and
 *   nothing currently renders it, so it shows as a plain jog. Needs authored
 *   (or imported) content: #423 sources the asset set, #424 builds the
 *   pipeline.
 *
 * THE `unanimated` LIST GOT LONGER WITH #418, not shorter. Until the
 * procedural 2D billboard was deleted, several of these poses DID read
 * distinctly -- that renderer carried a whole body-attitude vocabulary
 * (`actionLean`, a forward/back whole-figure lean, and `stanceDrop`, a crouch)
 * keyed off the pose id. The rigged path never had it. With the billboard gone
 * the 3D rig is the only renderer, so the coverage this table describes is the
 * WHOLE story rather than half of it. Each `unanimated` note below records the
 * deleted renderer's own constants (recoverable at
 * `git show 0333065^:v2/ts/packages/render/src/player_renderer.ts`), so the
 * follow-up is a transcription rather than a re-invention. Those are root
 * transforms, which makes `rig3d/action_pose.ts` -- not a clip -- their
 * natural home; deliberately not attempted here, since tuning a body attitude
 * without a visual pass is guesswork. #430 is that work, filed with these
 * constants.
 */
export type PoseFallback = "root_overlay" | "locomotion_base" | "unanimated";

/** How one `PlayerPoseId` animates. Exactly one of `action`/`fallback` is set. */
export interface PoseActionEntry {
  /** The stance action layered over the locomotion base, or `null`. */
  readonly action: StanceActionId | null;
  /** Which bones the action owns. `null` iff `action` is `null`. */
  readonly mask: ReadonlySet<string> | null;
  /** Seconds to blend this action in and out. `0` iff `action` is `null`. */
  readonly crossfade: number;
  /** Where the action's playback phase comes from. `null` iff `action` is `null`. */
  readonly phase: PhaseSource | null;
  /** Playback loop mode. `clamp` holds the final key rather than wrapping. */
  readonly loop: "repeat" | "clamp";
  /** Why there is no action. `null` iff `action` is non-null. */
  readonly fallback: PoseFallback | null;
  /** The reason this entry reads the way it does. Not decoration -- see file header. */
  readonly note: string;
}

// Crossfade durations, grouped so the relative ordering is visible rather than
// scattered as 32 magic numbers. A crossfade is how long a stance takes to
// take over from whatever was showing; it is a readability control, not a
// physical constant.
//
// The ordering that matters: a committed action (a throw, a strike windup) has
// to be on screen at the frame the simulation says it started, so it snaps;
// a held stance can afford to ease in, and a recovery should ease out slowly
// enough to read as "settling" rather than "cancelled".
const SNAP = 0.06;
const QUICK = 0.1;
const EASE = 0.16;
const SETTLE = 0.24;

function stance(
  action: StanceActionId,
  mask: ReadonlySet<string>,
  phase: PhaseSource,
  crossfade: number,
  loop: "repeat" | "clamp",
  note: string,
): PoseActionEntry {
  return { action, mask, crossfade, phase, loop, fallback: null, note };
}

function noAction(fallback: PoseFallback, note: string): PoseActionEntry {
  return { action: null, mask: null, crossfade: 0, phase: null, loop: "repeat", fallback, note };
}

/**
 * Every `PlayerPoseId`, and how it animates. The `Record` is exhaustive by
 * type: a new pose id on the wire is a compile error here, not a silent idle.
 */
export const POSE_ACTIONS: Readonly<Record<PlayerPoseId, PoseActionEntry>> = {
  // -------------------------------------------------------------------------
  // Keeper, possession. The keeper's hands ALSO follow `holding`/`throw`
  // independently of the pose id (see `animator.ts`'s possession layer, which
  // outranks this table exactly as `poseFor` always had it). These entries
  // make the pose id reach the same clips on its own, which is what stops a
  // grab or a throw rendering as a plain jog when the possession flags happen
  // not to be set on the frame the pose id is.
  // -------------------------------------------------------------------------
  keeper_grab: stance(
    "keeper_gather",
    masks.UPPER_BODY,
    "clock",
    QUICK,
    "repeat",
    "was: idle unless PlayerRenderOptions.holding happened to be set",
  ),
  keeper_throw: stance(
    "keeper_sling",
    masks.UPPER_BODY,
    "throw_timer",
    SNAP,
    "clamp",
    "was: idle unless PlayerRenderOptions.throw happened to be > 0",
  ),
  // A punt is a drop-and-kick: the arms release the ball exactly as a throw
  // does, the leg swing does not exist as a clip yet. Masked to ARMS rather
  // than UPPER_BODY so the torso keeps the locomotion lean instead of
  // adopting a thrower's unwind that no kick follows through on. The leg half
  // is #424's.
  keeper_punt: stance("keeper_sling", masks.ARMS, "throw_timer", SNAP, "clamp", "arms only; the kick itself needs #424"),

  // -------------------------------------------------------------------------
  // Keeper, saves and recoveries. All five save families plus the get-up are
  // `rig3d/action_pose.ts`'s, by design: they are continuous transforms driven
  // by the simulation's dive timer, and a baked clip would re-derive that ramp
  // and then disagree with the timer driving it.
  // -------------------------------------------------------------------------
  keeper_tip: noAction("root_overlay", "action_pose.SAVES.keeper_tip (fixed-amount lunge)"),
  keeper_spread: noAction("root_overlay", "action_pose.SAVES.keeper_spread"),
  keeper_central: noAction("root_overlay", "action_pose.SAVES.keeper_central"),
  keeper_stretch: noAction("root_overlay", "action_pose.SAVES.keeper_stretch"),
  keeper_dive: noAction("root_overlay", "action_pose.SAVES.keeper_dive"),
  keeper_get_up: noAction("root_overlay", "action_pose.TIPS.keeper_get_up"),

  // -------------------------------------------------------------------------
  // Keeper, ready stances. `keeper_set` and `keeper_ready_low` are the two
  // moments a keeper is braced rather than moving, and the guard stance is
  // exactly that shape -- weight forward, hands up, knees loaded. Both
  // rendered as a plain jog before this.
  // -------------------------------------------------------------------------
  keeper_set: stance(
    "guard_stance",
    masks.UPPER_BODY,
    "clock",
    EASE,
    "repeat",
    "was: plain gait. The deleted billboard also dropped the stance 0.18r; that crouch is #430's",
  ),
  keeper_ready_low: stance(
    "guard_stance",
    masks.UPPER_BODY,
    "clock",
    SETTLE,
    "repeat",
    "was: plain gait. The deleted billboard dropped the stance 0.32r -- #430 owns the LOW half; this entry is the braced arms",
  ),
  // Shuffling along the line IS locomotion -- that is the whole pose.
  keeper_shuffle: noAction("locomotion_base", "a shuffling keeper is walking; the base layer is the pose"),
  // The tall neutral: hands down, weight even. That is the idle end of the
  // locomotion blend already, so an overlay would only fight it.
  keeper_ready_tall: noAction("locomotion_base", "the idle end of the locomotion blend already reads as a tall ready"),

  // -------------------------------------------------------------------------
  // Aerials. `action_pose.aerial` lifts the root and, for a bicycle, rotates
  // it over backwards. The limbs stay on the locomotion blend deliberately:
  // the lift is what reads, and it is timer-driven.
  // -------------------------------------------------------------------------
  aerial_bicycle: noAction("root_overlay", "action_pose.aerial (lift + 78 degree backward rotation)"),
  aerial_action: noAction("root_overlay", "action_pose.aerial (lift only; header/volley/control)"),

  // -------------------------------------------------------------------------
  // Combat, forced states. Both are reactions to being hit, both are
  // continuous, both are `action_pose.TIPS`.
  // -------------------------------------------------------------------------
  combat_knockback: noAction("root_overlay", "action_pose.TIPS.combat_knockback"),
  combat_stagger: noAction("root_overlay", "action_pose.TIPS.combat_stagger"),

  // -------------------------------------------------------------------------
  // Combat, volitional phases. `guard`/`aim`/`recovery` all sit in the guard
  // stance and `active` holds the charge, exactly as `POSE_CLIP` had it --
  // parity, deliberately, so this rewrite is not also a re-tune.
  //
  // `combat_windup` is the one existing mapping that CHANGES: it used to
  // share the guard stance with `aim` and `recovery`, which made the three
  // phases indistinguishable. `clips.SWING`'s t=0.32 key is a literal windup
  // (torso coiled to the right, weapon arm overhead and back) and was
  // authored for exactly this -- see the dead `"swing"` branch this rewrite
  // deletes from `poseFor`, which no `POSE_CLIP` entry ever reached.
  // -------------------------------------------------------------------------
  combat_guard: stance("guard_stance", masks.UPPER_BODY, "clock", EASE, "repeat", "parity with POSE_CLIP"),
  combat_active: stance("charge", masks.UPPER_BODY, "gait", SNAP, "repeat", "parity with POSE_CLIP"),
  combat_windup: stance("swing", masks.UPPER_BODY, "windup", QUICK, "clamp", "was: guard, indistinguishable from aim/recovery"),
  combat_aim: stance("guard_stance", masks.UPPER_BODY, "clock", EASE, "repeat", "parity with POSE_CLIP"),
  combat_recovery: stance("guard_stance", masks.UPPER_BODY, "clock", SETTLE, "repeat", "parity with POSE_CLIP"),

  // -------------------------------------------------------------------------
  // Soccer actions.
  // -------------------------------------------------------------------------
  // Winding up a shot or a punt coils the torso and drops the shoulder over
  // the standing foot; `clips.SWING`'s windup key is that shape. Masked to the
  // upper body so the plant leg keeps the stride -- the leg half of a shot is
  // a `masks.KICK_R`/`KICK_L` clip that does not exist yet (#424).
  soccer_windup: stance("swing", masks.UPPER_BODY, "windup", QUICK, "clamp", "was: plain gait"),
  // A slide is a whole-body ground action: hips down, one leg extended, the
  // trailing leg folded, torso back. There is no clip for any of that and no
  // root transform approximates it (`action_pose.ts` covers reactions, not
  // committed actions), so this is a real gap and is named as one. The deleted
  // billboard had nothing for it either -- this one has never read distinctly.
  slide: noAction("unanimated", "no ground/slide clip exists; needs #423's asset set and #424's pipeline"),
  // A standing tackle is a committed forward lunge with the arms out for
  // balance, which is what `clips.CHARGE` is (a held attack pose that
  // deliberately says nothing about the legs, so the stride survives). The
  // deleted billboard threw the mass hardest of any pose at this one
  // (actionLean 0.8r forward, stanceDrop 0.2r), so a strong forward read is
  // the intent, not an approximation of one.
  tackle: stance("charge", masks.UPPER_BODY, "gait", SNAP, "repeat", "was: plain gait"),
  // A failed challenge: `action_pose.tip`'s own `stumble` branch tips the body
  // back off the trailing heel.
  stumble: noAction("root_overlay", "action_pose.tip's stumble branch"),
  // The follow-through after a kick is released. CONFIRMED INVISIBLE, not
  // merely unmapped: #428 wired the pose id end to end and a browser run of
  // `v2/tools/browser_match_harness` on hardware GL observed it on a real
  // roster slot at tick 600 -- so the wire works, and the pose still renders
  // bit-identically to plain running because neither this table nor
  // `action_pose.ts` claims it. `player_renderer_3d.spec.ts` pins that
  // equality as a stated gap.
  //
  // #426 is closed and did NOT close this: the torso lean it landed is a
  // velocity-derived balance cue, not a follow-through. #430 owns giving this
  // pose a root treatment of its own (billboard: 0.28r forward).
  kick_follow: noAction("unanimated", "wire confirmed live (#428), still renders as plain running; #430 owns the 0.28r follow-through"),
  // Deleted billboard: stanceDrop 0.3r -- a first touch is taken at a run, but
  // taken LOW, and the drop is what separated it from plain running.
  settle: noAction("unanimated", "billboard read it as stanceDrop 0.3r; #430 owns the crouch"),
  // Deleted billboard: actionLean 0.5r forward -- the telegraph is a body
  // committing before the feet do, not merely a change of speed.
  run_telegraph: noAction("unanimated", "billboard read it as actionLean 0.5r forward; #430 owns recovering it"),
  // Deleted billboard: actionLean -0.3r (weight BACK) plus stanceDrop 0.26r --
  // "contain holds its weight back: it shepherds, it does not commit". A guard
  // stance would be the opposite reading (weight forward), so this stays on the
  // base until the body attitude exists rather than borrowing a wrong one.
  contain: noAction("unanimated", "billboard read it as weight back 0.3r + stanceDrop 0.26r (#430); a forward stance would read as combat"),
  // `POSE_CLIP` mapped this to `"idle"`, which reached no branch in `poseFor`
  // at all -- so on the rigged path it has always rendered exactly as
  // `locomotion` does. The billboard did distinguish it (actionLean -0.12r,
  // stanceDrop 0.16r: a spent player sags), and that reading died with #418.
  fatigue: noAction("unanimated", "POSE_CLIP's `idle` entry was inert; billboard sagged it -0.12r + stanceDrop 0.16r (#430)"),
  locomotion: noAction("locomotion_base", "the fallback every player always has"),
};

/**
 * Which simulation quantity drives each stance action's playback phase.
 *
 * Keyed by ACTION rather than by pose id, deliberately: a crossfade has two
 * actions in flight, and the outgoing one still has to be sampled somewhere
 * sensible after the pose id that selected it is gone. Every `POSE_ACTIONS`
 * entry that names an action names a phase consistent with this table --
 * `pose_table.spec.ts` asserts that, so the two cannot drift.
 */
export const PHASE_BY_ACTION: Readonly<Record<StanceActionId, PhaseSource>> = {
  guard_stance: "clock",
  charge: "gait",
  swing: "windup",
  keeper_gather: "clock",
  keeper_sling: "throw_timer",
};

/**
 * How one pose id animates. Unknown/absent ids resolve to `locomotion`'s
 * entry, which is the documented default rather than a silent one.
 */
export function actionForPose(poseId?: string): PoseActionEntry {
  const entry = poseId !== undefined ? POSE_ACTIONS[poseId as PlayerPoseId] : undefined;
  return entry ?? POSE_ACTIONS.locomotion;
}

/** Every pose id that resolves to a stance action, for tests and diagnostics. */
export function animatedPoseIds(): readonly PlayerPoseId[] {
  return (Object.keys(POSE_ACTIONS) as PlayerPoseId[]).filter((id) => POSE_ACTIONS[id].action !== null);
}

// ---------------------------------------------------------------------------
// Locomotion blend
// ---------------------------------------------------------------------------

/** Blend weights for the three locomotion actions. Sums to exactly 1. */
export interface LocomotionBlend {
  readonly idle: number;
  readonly walk: number;
  readonly run: number;
}

// Mirrors `view_state.ts`'s own constants. Declared here rather than imported
// so this module stays free of `view_state.ts`'s mutable module-level `state`
// map; the two are pinned equal by `pose_table.spec.ts`.
const WALK_SPEED = 150;
const RUN_SPEED = 400;
const WALK_STRIDE = 130;
const RUN_STRIDE = 285;

function clamp01(x: number): number {
  return Math.max(0, Math.min(1, x));
}

/**
 * Speed-thresholded weights for the idle/walk/run actions, in world units per
 * second.
 *
 * These are exactly the weights that reproduce `poseFor`'s two nested
 * `clips.layer` calls once a `THREE.AnimationMixer` accumulates them, which is
 * not a coincidence and is worth stating because the arithmetic looks
 * arbitrary otherwise. `THREE.PropertyMixer.accumulate` slerps each incoming
 * action into the accumulator at `weight / cumulativeWeight`, so accumulating
 * idle, then walk, then run produces
 *
 *     slerp(slerp(idle, walk, walk / (idle + walk)), run, run / 1)
 *
 * and the products below are what make those two inner fractions come out as
 * `walkMix` and `runMix` -- the two numbers `poseFor` passed to `clips.layer`.
 * `player_renderer_3d.spec.ts` pins the two paths against each other.
 *
 * A run is not a fast walk, so the two stay separate clips blended by speed
 * rather than one clip played quicker.
 *
 * The identity above is EXACT rather than approximate only because the blend
 * is never genuinely three-way -- `runMix` leaves zero only once `walkMix` has
 * already saturated, so at most two weights are ever positive, and nested
 * slerps of two rotations are associative in a way three are not. That in turn
 * holds only while `RUN_SPEED > WALK_SPEED > 0`, which nothing here enforces
 * structurally; `pose_table.spec.ts` asserts the precondition before sweeping
 * the consequence.
 */
export function locomotionBlend(speed: number): LocomotionBlend {
  const walkMix = clamp01(speed / WALK_SPEED);
  const runMix = clamp01((speed - WALK_SPEED) / (RUN_SPEED - WALK_SPEED));
  return {
    idle: (1 - walkMix) * (1 - runMix),
    walk: walkMix * (1 - runMix),
    run: runMix,
  };
}

/** The stride length, in world units, one full two-step cycle covers at `speed`. */
export function strideFor(speed: number): number {
  const runMix = clamp01((speed - WALK_SPEED) / (RUN_SPEED - WALK_SPEED));
  return WALK_STRIDE + (RUN_STRIDE - WALK_STRIDE) * runMix;
}

/**
 * The ground-speed-matched playback rate for a locomotion clip of `duration`
 * seconds at `speed` world units per second: clip-seconds per real second.
 *
 * WHY THIS IS NOT THE THING THAT DRIVES PLAYBACK. `animator.ts` pins each
 * locomotion action's `.time` to `view.gait * duration` every frame and
 * evaluates with `mixer.update(0)`, so the mixer's own clock never advances on
 * its own. That is deliberate, and it is the same argument
 * `rig3d/action_pose.ts` makes for keeping dives parametric: `gait` is derived
 * from distance actually travelled, so a rollback correction that moves a
 * player also moves their feet, whereas a free-running animation clock would
 * carry on through the correction and the character would skate.
 *
 * This function is nonetheless the exact derivative of that pinning --
 * `d/dt (gait * duration) = duration * speed / stride(speed)` -- and
 * `animator.ts` assigns it to each action's `timeScale` so that the action's
 * own clock agrees with the anchor instead of fighting it, and so a future
 * caller that advances the mixer with a real `dt` (a baked asset set from
 * #424, played free-running) gets correct-speed playback without re-deriving
 * this. `pose_table.spec.ts` pins the derivative relationship itself.
 */
export function locomotionTimeScale(speed: number, duration: number): number {
  return (duration * speed) / strideFor(speed);
}

// ---------------------------------------------------------------------------
// Crossfade
// ---------------------------------------------------------------------------

/**
 * One character's stance-layer crossfade: how much of each stance action is
 * currently showing. Plain data, advanced by `advanceCrossfade` -- the state
 * lives in `animator.ts`'s per-player map, so this module stays pure.
 */
export interface CrossfadeState {
  /** Action id -> weight in [0, 1]. Absent means zero. */
  readonly weights: Readonly<Record<string, number>>;
}

/** An empty crossfade: nothing showing. */
export function newCrossfade(): CrossfadeState {
  return { weights: {} };
}

/** A crossfade already fully committed to `target`, for a character's first frame. */
export function snapCrossfade(target: StanceActionId | null): CrossfadeState {
  return target === null ? newCrossfade() : { weights: { [target]: 1 } };
}

function moveToward(current: number, goal: number, step: number): number {
  if (current < goal) {
    return Math.min(goal, current + step);
  }
  return Math.max(goal, current - step);
}

/**
 * Advances every stance weight toward its goal -- 1 for `target`, 0 for
 * everything else -- at `1 / crossfade` per second.
 *
 * Linear rather than exponential on purpose: a linear ramp reaches its goal at
 * a stated time, so "this stance takes 160ms to take over" is a fact a reader
 * can check against `POSE_ACTIONS` rather than a time constant they have to
 * integrate. It also means the weights of a two-action swap always sum to
 * exactly 1 (one rises by the same step the other falls), which is what keeps
 * the composite layer weight from overshooting mid-crossfade.
 *
 * `crossfade <= 0` (or a `dt` at least that long) snaps.
 */
export function advanceCrossfade(
  state: CrossfadeState,
  target: StanceActionId | null,
  dt: number,
  crossfade: number,
): CrossfadeState {
  const step = crossfade > 0 ? clamp01(Math.max(0, dt) / crossfade) : 1;
  const next: Record<string, number> = {};
  const ids = new Set(Object.keys(state.weights));
  if (target !== null) {
    ids.add(target);
  }
  for (const id of ids) {
    const goal = id === target ? 1 : 0;
    const weight = moveToward(state.weights[id] ?? 0, goal, step);
    // Dropping settled-to-zero entries keeps the map from accumulating one
    // key per stance a character has ever struck.
    if (weight > 0) {
      next[id] = weight;
    }
  }
  return { weights: next };
}

/** The composite weight the stance layer contributes, clamped to [0, 1]. */
export function crossfadeTotal(state: CrossfadeState): number {
  let total = 0;
  for (const weight of Object.values(state.weights)) {
    total += weight;
  }
  return Math.min(1, total);
}
