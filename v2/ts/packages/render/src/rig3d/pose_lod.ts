// Pose level-of-detail policy, ported from `game/render/rig3d/pose_lod.lua` (#394/#400):
// which characters may reuse last frame's evaluated pose, and when.
//
// PORTED BUT DELIBERATELY NOT WIRED IN. Read this before using it.
//
// The Lua original was justified by a profile of the *wasm-hosted Lua 5.1
// interpreter*: ~88% of per-character draw cost sat in pose evaluation —
// `clips.sample`/`clips.layer`, `skeleton.apply`'s 28 bone world transforms, and
// the bone-row build — against ~9% in the whole uniform and draw submission.
// Holding the limb pose every other frame halved the dominant cost, worth
// ~0.8 ms of browser frame time.
//
// That justification does not transfer. In this port skinning is three.js's
// `Skeleton` and clip sampling is `AnimationMixer`, both optimised JS over typed
// arrays rather than an interpreter, so the bottleneck this targets may simply
// not exist. Holding a pose is not free — it introduces cached state and an
// invalidation rule — and paying that for an unmeasured win is exactly the
// premature optimisation the original PR was careful to avoid: it profiled
// first, and only then optimised.
//
// So the policy is ported, with its tests, because it encodes a real design
// decision worth keeping. Wire it into the render path only after re-measuring
// per-character cost on this stack.
//
// The contract, unchanged from the Lua:
//
//   * DETERMINISTIC IN ITS INPUTS. The schedule is a function of a per-character
//     draw counter and a stagger index, never the wall clock, so the same draw
//     sequence always refreshes on the same frames.
//   * PRESENTATION-ONLY. Nothing here reads or writes simulation state.
//   * CONSERVATIVE. Anything the eye tracks — the controlled player, dives,
//     aerials, throws, combat, any pose the simulation drives through its own
//     timers — refreshes at full rate. Only the plain gait/idle family is ever
//     held, and only while the character is small on screen.

import type { PlayerRenderOptions } from "../player_renderer.ts";

/**
 * Characters taller than this on screen re-evaluate every frame: at close-up
 * sizes a held gait frame starts to read as a hitch. The whole-pitch camera
 * draws characters at roughly 60 px, comfortably under it; a follow camera
 * zooming in pushes past it and turns the LOD off by itself.
 */
export const FULL_RATE_HEIGHT_PX = 120;

/**
 * Refresh interval for held characters: every other frame. Halves the dominant
 * per-character cost while keeping limb animation at 30 Hz on a ~60 px figure,
 * which is beneath notice. Deliberately NOT deeper — a third frame of hold
 * starts to stutter the gait cycle.
 */
export const REDUCED_INTERVAL = 2;

/**
 * The pose ids that may be held: the plain locomotion/idle family.
 *
 * Everything else — `action_pose`'s whole-body ids (dives, aerials, knockback,
 * stumble), the combat stances, anything new and unknown — refreshes at full
 * rate, so an unlisted id can never be degraded by accident. Exported so the
 * spec can pin the set exhaustively: a dropped or mistyped entry must fail.
 */
export const RELAXED_POSE: ReadonlySet<string> = new Set([
  "locomotion",
  "contain",
  "run_telegraph",
  "fatigue",
  "keeper_shuffle",
  "settle",
  "kick_follow",
]);

/**
 * How often one character's pose must be re-evaluated, in frames. 1 means every
 * frame (no hold). Pure: same options and size, same answer.
 */
export function interval(opts: PlayerRenderOptions, heightPx: number): number {
  if (heightPx > FULL_RATE_HEIGHT_PX) {
    return 1;
  }
  // The player being steered is the one pair of eyes always on a character.
  if (opts.controlled) {
    return 1;
  }
  // Possession-driven keeper arms and every simulation-timer-driven action
  // animate continuously; holding a frame of them visibly desynchronises the
  // body from the simulation's own ramp.
  if (
    (opts.dive ?? 0) > 0 ||
    (opts.aerial ?? 0) > 0 ||
    (opts.throw ?? 0) > 0 ||
    (opts.windup ?? 0) > 0 ||
    opts.holding === true
  ) {
    return 1;
  }
  const id = opts.pose?.id;
  if (id !== undefined && !RELAXED_POSE.has(id)) {
    return 1;
  }
  return REDUCED_INTERVAL;
}

/**
 * Whether this character's pose is due for re-evaluation on this draw.
 *
 * `tick` counts the character's own draws; `stagger` is a small per-character
 * constant. Offsetting the schedule by the stagger spreads refreshes across
 * frames, so ten held characters cost five re-evaluations every frame instead of
 * ten on alternate frames — the mean is the same, the p95 is not.
 *
 * `tick` and `stagger` are both non-negative by construction, so `%` is safe
 * here; Lua's floored modulo and JavaScript's truncated remainder agree on
 * non-negative operands.
 */
export function due(tick: number, stagger: number, poseInterval: number): boolean {
  if (poseInterval <= 1) {
    return true;
  }
  return (tick + stagger) % poseInterval === 0;
}

/** One cached character: held bone rows, draw counter, schedule offset, fill state. */
export interface PoseLodEntry {
  /** The character's held bone rows (renderer-owned content). */
  rows: number[][];
  /** This character's own draw counter. */
  tick: number;
  /** Schedule offset, assigned round-robin at creation. */
  readonly stagger: number;
  /** False until the rows have been written once. */
  fresh: boolean;
}

/**
 * Owns the per-character cache and the round-robin stagger counter.
 *
 * The Lua kept `next_stagger` as module-level mutable state. AGENTS.md §3
 * forbids stray globals, and this port has consistently turned Lua module state
 * into owned state (`sim/tuning.lua` became a value, `game/audio.lua` became a
 * class), so the counter lives on an instance here.
 */
export class PoseLodScheduler {
  private nextStagger = 0;
  private readonly cache = new WeakMap<object, PoseLodEntry>();

  /**
   * The whole cache-entry lifecycle for one character's draw: look up (or
   * create) its entry, advance its draw counter, and decide whether this draw
   * must re-evaluate the pose.
   *
   * `refresh` is true when the caller must evaluate and write `entry.rows`
   * before submitting them. The entry is marked fresh here because that write is
   * the caller's unconditional next step. A brand-new or never-filled entry
   * always refreshes regardless of schedule — cached rows that were never
   * written must never reach the GPU.
   */
  step(key: object, poseInterval: number): { entry: PoseLodEntry; refresh: boolean } {
    let entry = this.cache.get(key);
    if (entry === undefined) {
      entry = { rows: [], tick: -1, stagger: this.nextStagger, fresh: false };
      this.nextStagger = (this.nextStagger + 1) % 1024;
      this.cache.set(key, entry);
    }
    entry.tick += 1;
    const refresh = !entry.fresh || due(entry.tick, entry.stagger, poseInterval);
    if (refresh) {
      entry.fresh = true;
    }
    return { entry, refresh };
  }
}
