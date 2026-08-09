// `THREE.AnimationMixer` playback for the rig, and the one thing that made it
// non-obvious: where the mixer's output goes.
//
// ---------------------------------------------------------------------------
// THE RECONCILIATION (read this before changing anything below)
// ---------------------------------------------------------------------------
//
// `player_renderer_3d.ts` deliberately BYPASSES three.js's transform pipeline
// for the character's bones. Every `THREE.Bone` is built with
// `matrixAutoUpdate = false` AND `matrixWorldAutoUpdate = false`, the mesh is
// bound with `bindMode = "detached"`, and the renderer writes `bone.matrixWorld`
// directly from `skeleton.apply(rig, pose)`. That is not an accident or a
// shortcut: each of those three flags is the fix for a specific defect that
// file's own comments document (a forced `updateMatrixWorld` cascade from the
// mesh's per-frame facing yaw silently replacing the pose with rest; "attached"
// bind mode recomputing `bindMatrixInverse` from the mesh's live world
// transform and exactly cancelling the screen placement pitch.ts applies).
//
// A `THREE.AnimationMixer` drives `.position`/`.quaternion`/`.scale` -- LOCAL
// transforms -- and needs three.js to compute world matrices from them. Those
// two designs cannot both own the same `THREE.Bone`.
//
// THE RESOLUTION: the mixer never touches a `THREE.Bone` at all. It drives a
// private graph of plain `THREE.Object3D` POSE CARRIERS -- one per bone name,
// flat children of a root that is in no scene, has no geometry and is never
// rendered. Their `.quaternion` and `.position` ARE the sparse pose
// `rig3d/clips.ts` has always produced, in the same authoring convention, and
// `evaluate()` reads them straight back out as a `MutablePose`. Everything
// downstream -- `rig3d/action_pose.ts`'s root overlay, `skeleton.apply`'s
// rest-rotation composition and `motion_scale`, the direct `bone.matrixWorld`
// write -- is untouched, because from its point of view nothing changed: it
// still receives a `{rot, move}` pose, just evaluated by three.js's mixer
// instead of by `clips.sample`/`clips.layer`.
//
// So this file replaces the SAMPLER, not the skinning path. The alternative --
// handing the bones to three.js and letting `updateMatrixWorld` do the
// skinning -- would mean re-litigating all three of those defects, and would
// additionally have to reproduce `skeleton.apply`'s `pose * rest` composition
// and its `motion_scale`-scaled additive translations inside baked clip data,
// making every clip rig-specific. Not worth it, and not what #425 asks for.
//
// ---------------------------------------------------------------------------
// WHY CLIPS ARE BAKED
// ---------------------------------------------------------------------------
//
// `clips.sample` eases each channel with smoothstep between adjacent keys.
// three.js's `QuaternionLinearInterpolant` does not (and three.js explicitly
// refuses cubic interpolation for quaternion tracks). Rather than reimplement
// the easing as a custom interpolant -- a second copy of the curve, free to
// drift from the authored one -- `bake` RESAMPLES each clip through
// `clips.sample` itself at a fixed rate and stores the result as ordinary
// linear keyframes. `clips.ts` stays the single authority on what a clip looks
// like, the baked tracks cannot disagree with it by construction, and the
// output is the shape an imported asset would arrive in anyway (#424).
//
// ---------------------------------------------------------------------------
// WHY MASKING IS STILL `rig3d/masks.ts`
// ---------------------------------------------------------------------------
//
// #425 proposed replacing `masks.ts` with three.js action weights. That does
// not work, for a reason worth recording so nobody tries it again:
// `THREE.PropertyMixer.accumulate` normalises by CUMULATIVE weight
// (`mix = weight / cumulativeWeight`). A full-weight overlay accumulated after
// a full-weight base therefore lands at `1 / (1 + 1)` -- a 50/50 blend, not an
// override. There is no per-action weight that yields an override while the
// base still drives the same bone; the only weight that does is zero.
//
// The alternative three.js offers is additive blending
// (`AnimationUtils.makeClipAdditive` + `AdditiveAnimationBlendMode`), which
// accumulates in its own slot and genuinely layers. But additive is a
// different LOOK: an additive guard stance ADDS its arm angles to the walk's
// arm swing instead of replacing them. That is the right mechanism for an
// imported, difference-authored asset set and the wrong one for these clips,
// which are authored as absolute poses.
//
// So layering stays `clips.layer` + a `masks.ts` bone set, and this file
// provides one mixer PER LAYER instead of one mixer with layered actions. Each
// layer's actions blend among themselves through the mixer (which is exactly
// where three's cumulative-weight normalisation is the behaviour you want --
// see `MixerLayer.evaluate`'s note on fading toward rest); the layers compose
// through the existing, tested `clips.layer`.

import * as THREE from "three";
import type { Quat } from "@gc/core";
import * as clips from "./clips.ts";
import type { Clip, EulerTriple } from "./clips.ts";
import type { MutablePose } from "./action_pose.ts";

/**
 * Keyframes per second `bake` resamples a clip at.
 *
 * Chosen against the shortest authored key spacing in `clips.ts` (`RUN`'s
 * 0.15s), which is where the linear reconstruction of a smoothstep ease is
 * worst: 60fps leaves ~9 baked keys per authored segment and drifts 3.2e-3 on
 * a quaternion component there (measured), 120fps leaves ~18 and drifts
 * 1.1e-3 -- about 0.13 degrees. `mixer.spec.ts` pins that tolerance across
 * every clip, so a future clip authored with tighter keys than `RUN` fails
 * the gate here rather than blending visibly wrong on screen.
 *
 * Raising it costs memory once, at build time, and nothing per frame:
 * `THREE.Interpolant` caches its last key index and binary-searches only on a
 * seek, so key count does not enter the per-evaluation cost.
 */
export const BAKE_FPS = 120;

const IDENTITY: Quat = [0, 0, 0, 1];
const ZERO: EulerTriple = [0, 0, 0];

/**
 * Resamples one `rig3d/clips.ts` clip into a `THREE.AnimationClip` whose track
 * names address the pose carriers `MixerLayer`'s constructor builds.
 *
 * Pure: same clip in, same tracks out. Track names are `"<bone>.quaternion"`
 * and `"<bone>.position"`. Bone names contain dots (`upper_arm.L`), which
 * three.js's `PropertyBinding.parseTrackName` handles -- its node pattern
 * admits dots and only the FINAL dotted segment is taken as the property, so
 * `"upper_arm.L.quaternion"` parses as node `upper_arm.L`, property
 * `quaternion`. `mixer.spec.ts` pins that, because it is the kind of thing an
 * upstream regex change could quietly break.
 */
export function bake(clip: Clip, fps = BAKE_FPS): THREE.AnimationClip {
  const frames = Math.max(1, Math.round(clip.duration * fps));
  const times = new Float32Array(frames + 1);
  const rotValues = new Map<string, Float32Array>();
  const moveValues = new Map<string, Float32Array>();
  for (const bone of clip.rotBones) {
    rotValues.set(bone, new Float32Array((frames + 1) * 4));
  }
  for (const bone of clip.moveBones) {
    moveValues.set(bone, new Float32Array((frames + 1) * 3));
  }

  for (let i = 0; i <= frames; i += 1) {
    const t = Math.min((i / frames) * clip.duration, clip.duration);
    times[i] = t;
    // `clips.sample` wraps with `time % duration`, so sampling exactly at the
    // duration returns the FIRST key. Every looping clip here has identical
    // first and last keys so that would be harmless, but `SWING` and
    // `KEEPER_SLING` are one-shots whose last key is a settle, and losing it
    // would silently truncate the follow-through. Nudging inside the last
    // segment costs nothing and is correct for both kinds.
    const pose = clips.sample(clip, Math.min(t, clip.duration * (1 - 1e-9)));
    for (const [bone, out] of rotValues) {
      const q = pose.rot[bone] ?? IDENTITY;
      out[i * 4] = q[0];
      out[i * 4 + 1] = q[1];
      out[i * 4 + 2] = q[2];
      out[i * 4 + 3] = q[3];
    }
    for (const [bone, out] of moveValues) {
      const m = pose.move[bone] ?? ZERO;
      out[i * 3] = m[0];
      out[i * 3 + 1] = m[1];
      out[i * 3 + 2] = m[2];
    }
  }

  const tracks: THREE.KeyframeTrack[] = [];
  for (const [bone, values] of rotValues) {
    tracks.push(new THREE.QuaternionKeyframeTrack(`${bone}.quaternion`, times, values));
  }
  for (const [bone, values] of moveValues) {
    tracks.push(new THREE.VectorKeyframeTrack(`${bone}.position`, times, values));
  }
  return new THREE.AnimationClip(clip.name, clip.duration, tracks);
}

/** One clip, with the id `MixerLayer` addresses it by. */
export interface LayerClip {
  readonly id: string;
  readonly clip: Clip;
  /** `clamp` holds the final key instead of wrapping. See `POSE_ACTIONS`. */
  readonly loop?: "repeat" | "clamp";
}

/**
 * One animation layer: a private pose graph, a `THREE.AnimationMixer` over it,
 * and one always-playing `THREE.AnimationAction` per clip.
 *
 * STATELESS BETWEEN CHARACTERS, which is what lets 22 players share one
 * instance. Every action's `.time` and `.weight` is written explicitly before
 * each `evaluate()`, and `evaluate()` advances the mixer by ZERO
 * (`mixer.update(0)`), so nothing carries over from the previous character but
 * values that are about to be overwritten. `THREE.AnimationAction._updateTime`
 * short-circuits on a zero delta and returns the pinned `.time` unchanged, so
 * this is evaluation, not playback -- see `pose_table.ts`'s
 * `locomotionTimeScale` for why playback phase is pinned to simulation
 * quantities rather than integrated from a clock.
 *
 * Actions are `play()`ed once at construction and never stopped: stopping one
 * drops its bindings' use count to zero, which makes three.js restore each
 * bound property to the value it recorded at bind time and re-record it on the
 * next `play()`. Holding them at `weight = 0` instead keeps the recorded
 * "original" pinned at the rest pose the carriers were built with, which is
 * what makes a partially-weighted layer fade toward REST -- exactly the
 * override semantics `clips.layer` gives a masked overlay.
 */
export class MixerLayer {
  private readonly root: THREE.Object3D;
  private readonly nodes: Map<string, THREE.Object3D>;
  private readonly mixer: THREE.AnimationMixer;
  private readonly actions: Map<string, THREE.AnimationAction>;
  private readonly baked: Map<string, THREE.AnimationClip>;
  /** Every bone any of this layer's clips rotates. */
  readonly rotBones: readonly string[];
  /** Every bone any of this layer's clips translates. */
  readonly moveBones: readonly string[];

  constructor(layerClips: readonly LayerClip[], fps = BAKE_FPS) {
    this.root = new THREE.Object3D();
    this.root.name = "pose_graph";
    this.nodes = new Map();
    this.actions = new Map();
    this.baked = new Map();

    const rot = new Set<string>();
    const move = new Set<string>();
    for (const entry of layerClips) {
      for (const bone of entry.clip.rotBones) {
        rot.add(bone);
      }
      for (const bone of entry.clip.moveBones) {
        move.add(bone);
      }
    }
    this.rotBones = [...rot];
    this.moveBones = [...move];

    for (const bone of new Set([...rot, ...move])) {
      const node = new THREE.Object3D();
      node.name = bone;
      // Nothing ever reads these carriers' matrices -- `evaluate` reads
      // `.quaternion`/`.position` directly -- so never spend the multiply.
      node.matrixAutoUpdate = false;
      node.matrixWorldAutoUpdate = false;
      this.nodes.set(bone, node);
      this.root.add(node);
    }

    this.mixer = new THREE.AnimationMixer(this.root);
    for (const entry of layerClips) {
      const baked = bake(entry.clip, fps);
      this.baked.set(entry.id, baked);
      const action = this.mixer.clipAction(baked);
      const once = entry.loop === "clamp";
      action.setLoop(once ? THREE.LoopOnce : THREE.LoopRepeat, once ? 1 : Infinity);
      action.clampWhenFinished = once;
      action.enabled = true;
      action.weight = 0;
      action.play();
      this.actions.set(entry.id, action);
    }
  }

  /** The baked clip for `id`, for callers that need its duration. */
  clipFor(id: string): THREE.AnimationClip | undefined {
    return this.baked.get(id);
  }

  /** Every action id this layer knows, in construction order. */
  ids(): readonly string[] {
    return [...this.actions.keys()];
  }

  /**
   * Pins one action's playback phase (seconds into the clip) and blend weight.
   * `weight` 0 leaves the action enabled but contributing nothing.
   */
  set(id: string, time: number, weight: number, timeScale = 1): void {
    const action = this.actions.get(id);
    if (action === undefined) {
      throw new Error(`rig3d/mixer.ts: no action "${id}" in this layer`);
    }
    action.time = time;
    action.weight = weight;
    action.timeScale = timeScale;
  }

  /** Sets every action's weight to 0, so a caller only has to state what IS playing. */
  silence(): void {
    for (const action of this.actions.values()) {
      action.weight = 0;
    }
  }

  /**
   * Evaluates every action at its pinned phase and reads the blended result
   * back as a sparse pose.
   *
   * A bone whose accumulated weight across this layer's actions is below 1 is
   * blended toward the value the carrier held when the bindings were first
   * bound -- rest. That is three.js's own behaviour
   * (`PropertyMixer.apply`'s `accuN := accuN + original * (1 - weight)`), and
   * it is precisely what `clips.layer` does for a masked bone the overlay does
   * not author. It is also why the locomotion base layer needs no explicit
   * `masks.FULL_BODY` pass: with idle/walk/run weights summing to 1, a bone
   * only some of them author (`spine`, `head`) fades toward rest by the same
   * arithmetic `poseFor`'s nested `clips.layer` calls used.
   */
  evaluate(): MutablePose {
    this.mixer.update(0);
    const rot: Record<string, Quat> = {};
    const move: Record<string, EulerTriple> = {};
    for (const bone of this.rotBones) {
      const node = this.nodes.get(bone);
      if (node !== undefined) {
        const q = node.quaternion;
        rot[bone] = [q.x, q.y, q.z, q.w];
      }
    }
    for (const bone of this.moveBones) {
      const node = this.nodes.get(bone);
      if (node !== undefined) {
        const p = node.position;
        move[bone] = [p.x, p.y, p.z];
      }
    }
    return { rot, move };
  }
}
